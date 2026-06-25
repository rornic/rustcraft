use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

use super::erosion::ErosionParams;
use super::noise::NoiseGenerator;
use super::region::{compute_region, Region, RegionCoord};

// 64 resident regions x ~256KB/region (heights+discharge, 256x256 f32 x2) =~ 32MB
// worst case - generous headroom over the chunk-load front's working set at render
// distance 64 (1024 blocks =~ 4x4 regions visible at once) without being a memory
// concern. Retuned once real access patterns are observed.
const RESIDENT_REGION_CAP: usize = 64;

// Coalescing slot: many concurrent chunk-gen tasks may be the first to ask for the
// same not-yet-computed region. Exactly one of them runs `compute_region`; the rest
// await the same `OnceLock`, rather than each kicking off their own (expensive)
// duplicate erosion pass.
type RegionSlot = Arc<OnceLock<Arc<Region>>>;

// True LRU (eviction by *access* recency, not insertion order) - unlike the FIFO
// point cache in `noise.rs`, evicting a region here means the next chunk near it
// pays a full re-erosion, so recency matters. `cap` is small (64) so an O(n) scan to
// find the least-recently-used entry on eviction is cheap; a real LRU data
// structure would be overkill at this size.
struct RegionLru {
    cap: usize,
    tick: u64,
    entries: HashMap<RegionCoord, (RegionSlot, u64)>,
}

impl RegionLru {
    fn new(cap: usize) -> Self {
        Self { cap, tick: 0, entries: HashMap::new() }
    }

    fn get_or_insert(&mut self, coord: RegionCoord) -> RegionSlot {
        self.tick += 1;
        let tick = self.tick;
        if let Some((slot, last_access)) = self.entries.get_mut(&coord) {
            *last_access = tick;
            return slot.clone();
        }

        let slot: RegionSlot = Arc::new(OnceLock::new());
        self.entries.insert(coord, (slot.clone(), tick));
        if self.entries.len() > self.cap {
            if let Some(evict) = self
                .entries
                .iter()
                .min_by_key(|(_, (_, last_access))| *last_access)
                .map(|(coord, _)| *coord)
            {
                self.entries.remove(&evict);
            }
        }
        slot
    }
}

pub struct RegionStore {
    seed: u32,
    world_height: u64,
    erosion_params: ErosionParams,
    noise: Arc<NoiseGenerator>,
    // Guards *which slots exist*, not each slot's contents - the OnceLock inside a
    // slot handles "many readers, exactly one writer" for that region's actual
    // compute, so this lock is only ever held briefly (get-or-insert + LRU touch),
    // never across the expensive erosion call itself.
    lru: RwLock<RegionLru>,
}

impl RegionStore {
    pub fn new(seed: u32, world_height: u64, noise: Arc<NoiseGenerator>, erosion_params: ErosionParams) -> Self {
        Self {
            seed,
            world_height,
            erosion_params,
            noise,
            lru: RwLock::new(RegionLru::new(RESIDENT_REGION_CAP)),
        }
    }

    // Blocks the calling thread until this region is ready (computing it, or
    // waiting on another in-flight compute for the same coordinate) - safe to call
    // only from inside an async-task-pool task, never a Bevy system body on the main
    // thread, exactly like `generate_chunk` itself.
    pub fn get_region(&self, coord: RegionCoord) -> Arc<Region> {
        let slot = self.lru.write().unwrap().get_or_insert(coord);
        slot.get_or_init(|| {
            Arc::new(compute_region(&self.noise, self.seed, coord, self.world_height, &self.erosion_params))
        })
        .clone()
    }

    pub fn get_height(&self, world_x: i64, world_z: i64) -> f64 {
        let coord = RegionCoord::from_world(world_x, world_z);
        let region = self.get_region(coord);
        let origin = coord.origin();
        region.height_at(world_x - origin.x, world_z - origin.y) as f64
    }

    pub fn get_discharge(&self, world_x: i64, world_z: i64) -> f64 {
        let coord = RegionCoord::from_world(world_x, world_z);
        let region = self.get_region(coord);
        let origin = coord.origin();
        region.discharge_at(world_x - origin.x, world_z - origin.y) as f64
    }

    // Central-difference gradient over the eroded heightmap - same shape as
    // `generate_chunk`'s previous inline gradient computation, but reading
    // `get_height` (already in absolute block-height units) instead of raw noise.
    pub fn get_gradient(&self, world_x: i64, world_z: i64) -> (f64, f64) {
        let gradient_x = self.get_height(world_x + 1, world_z) - self.get_height(world_x - 1, world_z);
        let gradient_z = self.get_height(world_x, world_z + 1) - self.get_height(world_x, world_z - 1);
        (gradient_x, gradient_z)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Barrier;

    use super::*;
    use bevy::math::I64Vec2;

    #[test]
    fn test_get_region_caches_repeated_lookups() {
        let store = RegionStore::new(
            1,
            256,
            Arc::new(NoiseGenerator::new(1)),
            ErosionParams { droplet_count: 50, ..ErosionParams::default() },
        );
        let coord = RegionCoord(I64Vec2::new(0, 0));

        let first = store.get_region(coord);
        let second = store.get_region(coord);
        assert!(Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn test_concurrent_get_region_computes_once() {
        // Uses a tiny droplet count so this test stays fast even though it's
        // exercising real `compute_region` work, not a mock - the property under
        // test (every caller observes the same computed Region under contention)
        // only means anything against the real coalescing path.
        let store = Arc::new(RegionStore::new(
            1,
            256,
            Arc::new(NoiseGenerator::new(1)),
            ErosionParams { droplet_count: 50, ..ErosionParams::default() },
        ));
        let coord = RegionCoord(I64Vec2::new(5, 5));
        let thread_count = 8;
        let barrier = Arc::new(Barrier::new(thread_count));

        let handles: Vec<_> = (0..thread_count)
            .map(|_| {
                let store = store.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    store.get_region(coord)
                })
            })
            .collect();

        let regions: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        for region in &regions[1..] {
            assert!(Arc::ptr_eq(&regions[0], region), "all callers should observe the same computed Region");
        }
    }
}
