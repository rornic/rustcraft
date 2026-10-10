use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

use bevy::math::I64Vec2;

use super::noise::NoiseGenerator;
use super::region::{compute_region, Region, RegionCoord, REGION_SIZE};

// 64 resident regions x ~256KB/region (256x256 f32 heights) =~ 16MB
// worst case - generous headroom over the chunk-load front's working set at render
// distance 64 (1024 blocks =~ 4x4 regions visible at once) without being a memory
// concern. Retuned once real access patterns are observed.
const RESIDENT_REGION_CAP: usize = 64;

pub fn resident_cap_for(radius_blocks: i64) -> usize {
    let across = ((2 * radius_blocks + REGION_SIZE - 1) / REGION_SIZE) as usize + 2;
    (across * across).max(RESIDENT_REGION_CAP)
}

// Coalescing slot: many concurrent chunk-gen tasks may be the first to ask for the
// same not-yet-computed region. Exactly one of them runs `compute_region`; the rest
// await the same `OnceLock`, rather than each sampling the same noise again.
type RegionSlot = Arc<OnceLock<Arc<Region>>>;

// True LRU (eviction by *access* recency, not insertion order) - evicting a region
// means the next chunk near it pays to sample it again, so recency matters. `cap` is small (64) so an O(n) scan to
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
    noise: Arc<NoiseGenerator>,
    // Guards *which slots exist*, not each slot's contents - the OnceLock inside a
    // slot handles "many readers, exactly one writer" for that region's actual
    // compute, so this lock is only ever held briefly (get-or-insert + LRU touch),
    // never across the region's noise sampling itself.
    lru: RwLock<RegionLru>,
}

impl RegionStore {
    pub fn new(noise: Arc<NoiseGenerator>) -> Self {
        Self {
            noise,
            lru: RwLock::new(RegionLru::new(RESIDENT_REGION_CAP)),
        }
    }

    pub fn with_resident_cap(self, cap: usize) -> Self {
        Self {
            lru: RwLock::new(RegionLru::new(cap)),
            ..self
        }
    }

    // Blocks the calling thread until this region is ready (computing it, or
    // waiting on another in-flight compute for the same coordinate) - safe to call
    // only from inside an async-task-pool task, never a Bevy system body on the main
    // thread, exactly like `generate_chunk` itself.
    pub fn get_region(&self, coord: RegionCoord) -> Arc<Region> {
        let slot = self.lru.write().unwrap().get_or_insert(coord);
        slot.get_or_init(|| Arc::new(compute_region(&self.noise, coord)))
            .clone()
    }

    /// The highest terrain in the block range `min..=max` (x, z), rounded out to whole
    /// tiles, or None if a region it touches hasn't been computed yet. Never blocks.
    pub fn loaded_max_height(&self, min: I64Vec2, max: I64Vec2) -> Option<f32> {
        let lru = self.lru.read().unwrap();
        let (first, last) = (RegionCoord::from_world(min.x, min.y), RegionCoord::from_world(max.x, max.y));
        let mut highest = f32::MIN;
        for rz in first.0.y..=last.0.y {
            for rx in first.0.x..=last.0.x {
                let coord = RegionCoord(I64Vec2::new(rx, rz));
                let region = lru.entries.get(&coord)?.0.get()?;
                let origin = coord.origin();
                let local_min = (min - origin).max(I64Vec2::ZERO);
                let local_max = (max - origin).min(I64Vec2::splat(REGION_SIZE - 1));
                highest = highest.max(region.max_height(local_min, local_max));
            }
        }
        Some(highest)
    }

    pub fn sampler(&self) -> HeightSampler<'_> {
        HeightSampler {
            store: self,
            recent: Vec::with_capacity(SAMPLER_REGIONS),
        }
    }

    pub fn get_height(&self, world_x: i64, world_z: i64) -> f64 {
        self.sampler().height(world_x, world_z)
    }
}

// Enough for a lookup area that touches a corner where four regions meet.
const SAMPLER_REGIONS: usize = 4;

pub struct HeightSampler<'a> {
    store: &'a RegionStore,
    // most recently used last
    recent: Vec<(RegionCoord, Arc<Region>)>,
}

impl HeightSampler<'_> {
    pub fn height(&mut self, world_x: i64, world_z: i64) -> f64 {
        let coord = RegionCoord::from_world(world_x, world_z);
        let origin = coord.origin();
        self.region(coord)
            .height_at(world_x - origin.x, world_z - origin.y) as f64
    }

    // Central-difference gradient over the heightmap, in block-height units.
    pub fn gradient(&mut self, world_x: i64, world_z: i64) -> (f64, f64) {
        let gradient_x = self.height(world_x + 1, world_z) - self.height(world_x - 1, world_z);
        let gradient_z = self.height(world_x, world_z + 1) - self.height(world_x, world_z - 1);
        (gradient_x, gradient_z)
    }

    fn region(&mut self, coord: RegionCoord) -> &Region {
        match self.recent.iter().position(|(c, _)| *c == coord) {
            Some(i) => {
                let entry = self.recent.remove(i);
                self.recent.push(entry);
            }
            None => {
                if self.recent.len() == SAMPLER_REGIONS {
                    self.recent.remove(0);
                }
                self.recent.push((coord, self.store.get_region(coord)));
            }
        }
        &self.recent.last().unwrap().1
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Barrier;

    use super::*;

    #[test]
    fn test_resident_cap_covers_render_area() {
        assert_eq!(RESIDENT_REGION_CAP, resident_cap_for(256));
        // 2048 blocks either side spans 16 regions, plus one at each edge
        assert_eq!(18 * 18, resident_cap_for(2048));
    }

    #[test]
    fn test_sampler_matches_region_heights_across_region_corners() {
        let store = RegionStore::new(Arc::new(NoiseGenerator::new(1)));
        let mut sampler = store.sampler();
        // a block either side of the corner where regions (-1..0, -1..0) meet
        for (x, z) in [(-1, -1), (0, -1), (-1, 0), (0, 0), (255, 300), (-1, -1)] {
            let coord = RegionCoord::from_world(x, z);
            let origin = coord.origin();
            let expected = store
                .get_region(coord)
                .height_at(x - origin.x, z - origin.y) as f64;
            assert_eq!(expected, sampler.height(x, z));
        }
        assert!(sampler.recent.len() <= SAMPLER_REGIONS);
    }

    #[test]
    fn test_loaded_max_height_waits_for_every_region_it_touches() {
        let store = RegionStore::new(Arc::new(NoiseGenerator::new(1)));
        // spans the corner of regions (0, 0) to (1, 1)
        let (min, max) = (I64Vec2::splat(200), I64Vec2::splat(300));
        assert_eq!(None, store.loaded_max_height(min, max));
        store.get_region(RegionCoord(I64Vec2::new(0, 0)));
        assert_eq!(None, store.loaded_max_height(min, max));
        for (x, z) in [(1, 0), (0, 1), (1, 1)] {
            store.get_region(RegionCoord(I64Vec2::new(x, z)));
        }
        let mut sampler = store.sampler();
        let highest = (200..=300)
            .flat_map(|z| (200..=300).map(move |x| (x, z)))
            .map(|(x, z)| sampler.height(x, z) as f32)
            .fold(f32::MIN, f32::max);
        assert!(store.loaded_max_height(min, max).unwrap() >= highest);
    }

    #[test]
    fn test_get_region_caches_repeated_lookups() {
        let store = RegionStore::new(Arc::new(NoiseGenerator::new(1)));
        let coord = RegionCoord(I64Vec2::new(0, 0));

        let first = store.get_region(coord);
        let second = store.get_region(coord);
        assert!(Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn test_concurrent_get_region_computes_once() {
        // Exercises real `compute_region` work, not a mock - the property under
        // test (every caller observes the same computed Region under contention)
        // only means anything against the real coalescing path.
        let store = Arc::new(RegionStore::new(Arc::new(NoiseGenerator::new(1))));
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
