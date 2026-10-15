use std::collections::HashMap;
use std::hash::Hash;
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use super::biome::BiomeId;
use super::noise::NoiseGenerator;
use super::region_store::HeightSampler;
use crate::chunks::chunk::CHUNK_SIZE;
use crate::chunks::lod::LodKey;

/// LOD nodes with cells at least this many blocks across sample the noise at their cells'
/// spacing, instead of from full-detail regions: a node a kilometre wide needs a few hundred
/// columns, not the dozens of 256-block regions it covers.
pub const COARSE_CELL: i64 = 4;

// Grids kept for the nodes stacked over a column to share: they're spawned together, so a
// short memory is enough.
const GRIDS_KEPT: usize = 4096;
// Full-detail chunk columns kept, about 6KB each: chunks load outward in shells, so a column's
// chunks come over the whole time the view takes to fill, and all those in view are kept. That's
// about 3,000 at a full detail distance of 24.
const CHUNK_COLUMNS_KEPT: usize = 8192;
// Height ranges kept for skipping empty nodes: a few bytes each, so plenty.
const RANGES_KEPT: usize = 500_000;

// Cells sampled past a node's edge each way: one for the cells meshes look past their edges,
// and one more for slopes there.
const RING: i32 = 2;

// One column of LOD nodes, stacked up the world, all sharing a grid.
#[derive(Debug, Copy, Clone, Hash, PartialEq, Eq)]
struct ColumnKey {
    level: u8,
    x: i64,
    z: i64,
}

impl From<LodKey> for ColumnKey {
    fn from(key: LodKey) -> Self {
        Self {
            level: key.level,
            x: key.coord.x,
            z: key.coord.z,
        }
    }
}

/// The ground at the centre of each cell of a column of LOD nodes, from one noise sample apiece,
/// leaving out what's too fine for the cells to show.
pub struct ColumnGrid {
    cell: i64,
    heights: Vec<f32>,
    biomes: Vec<BiomeId>,
    trees: Vec<f32>,
    /// The lowest and highest ground under the node's cells, and the cell beyond each edge.
    pub lowest: f32,
    pub highest: f32,
}

impl ColumnGrid {
    fn sample(noise: &NoiseGenerator, key: LodKey) -> Self {
        let size = CHUNK_SIZE as i32;
        let cell = key.cell_size();
        let base = key.min_block();
        let span = (size + 2 * RING) as usize;
        let mut grid = Self {
            cell,
            heights: Vec::with_capacity(span * span),
            biomes: Vec::with_capacity(span * span),
            trees: Vec::with_capacity(span * span),
            lowest: f32::MAX,
            highest: f32::MIN,
        };
        for cx in -RING..size + RING {
            for cz in -RING..size + RING {
                let world_x = base.x + cx as i64 * cell + cell / 2;
                let world_z = base.z + cz as i64 * cell + cell / 2;
                let sample = noise.sample_for_cell(world_x as f64, world_z as f64, cell as f64);
                grid.heights.push(sample.height as f32);
                grid.biomes.push(sample.biome);
                grid.trees.push(sample.trees as f32);
                if (-1..=size).contains(&cx) && (-1..=size).contains(&cz) {
                    grid.lowest = grid.lowest.min(sample.height as f32);
                    grid.highest = grid.highest.max(sample.height as f32);
                }
            }
        }
        grid
    }

    // `cx` and `cz` run from -RING to CHUNK_SIZE + RING - 1
    fn index(cx: i32, cz: i32) -> usize {
        let span = CHUNK_SIZE as i32 + 2 * RING;
        ((cx + RING) * span + cz + RING) as usize
    }

    /// The ground's height at the centre of cell (`cx`, `cz`), from -1 to CHUNK_SIZE each way.
    pub fn height(&self, cx: i32, cz: i32) -> f64 {
        self.heights[Self::index(cx, cz)] as f64
    }

    /// The slope there, as `terrain_block` takes it: the height change across two blocks
    /// each way, summed, measured between the cells either side.
    pub fn gradient(&self, cx: i32, cz: i32) -> f64 {
        let across = |a: f64, b: f64| (a - b).abs() / self.cell as f64;
        across(self.height(cx + 1, cz), self.height(cx - 1, cz))
            + across(self.height(cx, cz + 1), self.height(cx, cz - 1))
    }

    pub fn biome(&self, cx: i32, cz: i32) -> BiomeId {
        self.biomes[Self::index(cx, cz)]
    }

    pub fn trees(&self, cx: i32, cz: i32) -> f64 {
        self.trees[Self::index(cx, cz)] as f64
    }
}

/// What generating a full-detail chunk takes from each of its columns, the same for every chunk
/// stacked over them, see `generator::generate_chunk`.
pub struct ChunkColumns {
    heights: Vec<i64>,
    gradients: Vec<f64>,
    biomes: Vec<BiomeId>,
    temperatures: Vec<f32>,
    /// The lowest and highest ground under the chunks.
    pub lowest: i64,
    pub highest: i64,
}

impl ChunkColumns {
    fn sample(noise: &NoiseGenerator, heights: &mut HeightSampler, x: i64, z: i64) -> Self {
        let size = CHUNK_SIZE as i64;
        let cells = (size * size) as usize;
        let mut columns = Self {
            heights: Vec::with_capacity(cells),
            gradients: Vec::with_capacity(cells),
            biomes: Vec::with_capacity(cells),
            temperatures: Vec::with_capacity(cells),
            lowest: i64::MAX,
            highest: i64::MIN,
        };
        for cx in 0..size {
            for cz in 0..size {
                let (world_x, world_z) = (x * size + cx, z * size + cz);
                let height = heights.height(world_x, world_z).round() as i64;
                let (gradient_x, gradient_z) = heights.gradient(world_x, world_z);
                columns.heights.push(height);
                columns.gradients.push(gradient_x.abs() + gradient_z.abs());
                columns.biomes.push(heights.biome(world_x, world_z));
                columns
                    .temperatures
                    .push(noise.temperature(world_x as f64, world_z as f64));
                (columns.lowest, columns.highest) =
                    (columns.lowest.min(height), columns.highest.max(height));
            }
        }
        columns
    }

    fn index(cx: i64, cz: i64) -> usize {
        (cx * CHUNK_SIZE as i64 + cz) as usize
    }

    /// The ground's height in column (`cx`, `cz`) of the chunk, rounded to whole blocks.
    pub fn height(&self, cx: i64, cz: i64) -> i64 {
        self.heights[Self::index(cx, cz)]
    }

    /// The slope there, as `terrain_block` takes it.
    pub fn gradient(&self, cx: i64, cz: i64) -> f64 {
        self.gradients[Self::index(cx, cz)]
    }

    pub fn biome(&self, cx: i64, cz: i64) -> BiomeId {
        self.biomes[Self::index(cx, cz)]
    }

    pub fn temperature(&self, cx: i64, cz: i64) -> f32 {
        self.temperatures[Self::index(cx, cz)]
    }
}

type Slot<V> = Arc<OnceLock<Arc<V>>>;

// Each slot with the tick it was last used at.
type Slots<K, V> = HashMap<K, (Slot<V>, u64)>;

// The most recently used values by key, each worked out once however many threads ask for it.
struct Recent<K, V> {
    kept: usize,
    slots: Mutex<(u64, Slots<K, V>)>,
}

impl<K: Hash + Eq, V> Recent<K, V> {
    fn new(kept: usize) -> Self {
        Self {
            kept,
            slots: Mutex::new((0, HashMap::new())),
        }
    }

    // Blocks while another thread works it out, so only call from a task, never a system.
    fn get(&self, key: K, make: impl FnOnce() -> V) -> Arc<V> {
        let slot = {
            let mut guard = self.slots.lock().unwrap();
            let (tick, slots) = &mut *guard;
            *tick += 1;
            let slot = slots
                .entry(key)
                .or_insert_with(|| (Arc::new(OnceLock::new()), 0));
            slot.1 = *tick;
            let slot = slot.0.clone();
            if slots.len() > self.kept {
                // forget the least recently used half at once, so this stays cheap
                let mut ages: Vec<u64> = slots.values().map(|(_, used)| *used).collect();
                let (_, &mut cutoff, _) = ages.select_nth_unstable(slots.len() / 2);
                slots.retain(|_, (_, used)| *used > cutoff);
            }
            slot
        };
        slot.get_or_init(|| Arc::new(make())).clone()
    }
}

/// Coarse LOD nodes' grids, shared by each column's stacked nodes, and their height ranges, and
/// full-detail chunks' columns, shared by each column's stacked chunks.
pub struct ColumnCache {
    grids: Recent<ColumnKey, ColumnGrid>,
    ranges: RwLock<HashMap<ColumnKey, (f32, f32)>>,
    chunk_columns: Recent<(i64, i64), ChunkColumns>,
}

impl Default for ColumnCache {
    fn default() -> Self {
        Self {
            grids: Recent::new(GRIDS_KEPT),
            ranges: RwLock::default(),
            chunk_columns: Recent::new(CHUNK_COLUMNS_KEPT),
        }
    }
}

impl ColumnCache {
    /// The columns under full-detail chunks at chunk `x`, `z`, sampling them if no chunk over
    /// them has yet. Blocks while another thread samples them, so only call from a task.
    pub fn chunk_columns(
        &self,
        noise: &NoiseGenerator,
        heights: &mut HeightSampler,
        x: i64,
        z: i64,
    ) -> Arc<ChunkColumns> {
        self.chunk_columns
            .get((x, z), || ChunkColumns::sample(noise, heights, x, z))
    }

    /// The grid under `key`, sampling it if no node over the same column has yet. Blocks
    /// while another thread samples it, so only call from a task, never a system.
    pub fn grid(&self, noise: &NoiseGenerator, key: LodKey) -> Arc<ColumnGrid> {
        let column = ColumnKey::from(key);
        self.grids.get(column, || {
            let grid = ColumnGrid::sample(noise, key);
            let mut ranges = self.ranges.write().unwrap();
            if ranges.len() >= RANGES_KEPT {
                ranges.clear();
            }
            ranges.insert(column, (grid.lowest, grid.highest));
            grid
        })
    }

    /// The lowest and highest ground under `key`'s column, if a node over it has been
    /// sampled. Never blocks.
    pub fn height_range(&self, key: LodKey) -> Option<(f32, f32)> {
        self.ranges
            .read()
            .unwrap()
            .get(&ColumnKey::from(key))
            .copied()
    }
}

#[cfg(test)]
mod tests {
    use bevy::math::I64Vec3;

    use super::*;

    fn key(level: u8, x: i64, y: i64, z: i64) -> LodKey {
        LodKey {
            level,
            coord: I64Vec3::new(x, y, z),
        }
    }

    #[test]
    fn test_grid_matches_the_noise_at_cell_centres() {
        let noise = NoiseGenerator::new(4);
        let key = key(3, 5, 2, -7);
        let grid = ColumnGrid::sample(&noise, key);
        let base = key.min_block();
        let cell = key.cell_size();
        for (cx, cz) in [(-1, -1), (0, 0), (7, 12), (16, 16)] {
            let x = base.x + cx as i64 * cell + cell / 2;
            let z = base.z + cz as i64 * cell + cell / 2;
            assert_eq!(
                noise
                    .sample_for_cell(x as f64, z as f64, cell as f64)
                    .height as f32 as f64,
                grid.height(cx, cz)
            );
        }
        assert!(grid.lowest <= grid.highest);
    }

    #[test]
    fn test_chunk_columns_match_the_regions_and_are_shared_up_the_stack() {
        let regions =
            super::super::region_store::RegionStore::new(Arc::new(NoiseGenerator::new(4)));
        let (x, z) = (-3, 5);
        let mut heights = regions.sampler();
        let columns = regions.chunk_columns(&mut heights, x, z);
        let size = CHUNK_SIZE as i64;
        for (cx, cz) in [(0, 0), (3, 11), (15, 15)] {
            let (world_x, world_z) = (x * size + cx, z * size + cz);
            let (gx, gz) = heights.gradient(world_x, world_z);
            assert_eq!(
                heights.height(world_x, world_z).round() as i64,
                columns.height(cx, cz)
            );
            assert_eq!(gx.abs() + gz.abs(), columns.gradient(cx, cz));
            assert_eq!(heights.biome(world_x, world_z), columns.biome(cx, cz));
            assert_eq!(
                regions.noise().temperature(world_x as f64, world_z as f64),
                columns.temperature(cx, cz)
            );
            assert!((columns.lowest..=columns.highest).contains(&columns.height(cx, cz)));
        }
        assert!(Arc::ptr_eq(
            &columns,
            &regions.chunk_columns(&mut heights, x, z)
        ));
        assert!(!Arc::ptr_eq(
            &columns,
            &regions.chunk_columns(&mut heights, x + 1, z)
        ));
    }

    #[test]
    fn test_recent_forgets_the_least_recently_used() {
        let recent = Recent::new(4);
        let first = recent.get(0, || 0);
        for key in 1..=4 {
            recent.get(key, || key);
        }
        // the first was the least recently used, so it's worked out again
        assert!(!Arc::ptr_eq(&first, &recent.get(0, || 0)));
        assert!(Arc::ptr_eq(&recent.get(4, || 99), &recent.get(4, || 99)));
    }

    #[test]
    fn test_stacked_nodes_share_a_grid_and_record_its_range() {
        let noise = NoiseGenerator::new(4);
        let cache = ColumnCache::default();
        assert_eq!(None, cache.height_range(key(2, 1, 0, 1)));
        let low = cache.grid(&noise, key(2, 1, 0, 1));
        let high = cache.grid(&noise, key(2, 1, 9, 1));
        assert!(Arc::ptr_eq(&low, &high));
        assert_eq!(
            Some((low.lowest, low.highest)),
            cache.height_range(key(2, 1, 5, 1))
        );
        assert_eq!(None, cache.height_range(key(2, 2, 0, 1)));
    }
}
