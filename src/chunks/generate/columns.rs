use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use super::biome::BiomeId;
use super::noise::NoiseGenerator;
use crate::chunks::chunk::CHUNK_SIZE;
use crate::chunks::lod::LodKey;

/// LOD nodes with cells at least this many blocks across sample the noise at their cells'
/// spacing, instead of from full-detail regions: a node a kilometre wide needs a few hundred
/// columns, not the dozens of 256-block regions it covers.
pub const COARSE_CELL: i64 = 4;

// Grids kept for the nodes stacked over a column to share: they're spawned together, so a
// short memory is enough.
const GRIDS_KEPT: usize = 4096;
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

/// The ground at the centre of each cell of a column of LOD nodes, from one noise sample apiece.
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
                let sample = noise.sample(world_x as f64, world_z as f64);
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
        across(self.height(cx + 1, cz), self.height(cx - 1, cz)) + across(self.height(cx, cz + 1), self.height(cx, cz - 1))
    }

    pub fn biome(&self, cx: i32, cz: i32) -> BiomeId {
        self.biomes[Self::index(cx, cz)]
    }

    pub fn trees(&self, cx: i32, cz: i32) -> f64 {
        self.trees[Self::index(cx, cz)] as f64
    }
}

type GridSlot = Arc<OnceLock<Arc<ColumnGrid>>>;

/// Coarse LOD nodes' grids, shared by each column's stacked nodes, and their height ranges.
#[derive(Default)]
pub struct ColumnCache {
    grids: Mutex<(u64, HashMap<ColumnKey, (GridSlot, u64)>)>,
    ranges: RwLock<HashMap<ColumnKey, (f32, f32)>>,
}

impl ColumnCache {
    /// The grid under `key`, sampling it if no node over the same column has yet. Blocks
    /// while another thread samples it, so only call from a task, never a system.
    pub fn grid(&self, noise: &NoiseGenerator, key: LodKey) -> Arc<ColumnGrid> {
        let column = ColumnKey::from(key);
        let slot = {
            let mut guard = self.grids.lock().unwrap();
            let (tick, grids) = &mut *guard;
            *tick += 1;
            let slot = grids.entry(column).or_insert_with(|| (Arc::new(OnceLock::new()), 0));
            slot.1 = *tick;
            let slot = slot.0.clone();
            if grids.len() > GRIDS_KEPT {
                // forget the least recently used half at once, so this stays cheap
                let mut ages: Vec<u64> = grids.values().map(|(_, used)| *used).collect();
                let (_, &mut cutoff, _) = ages.select_nth_unstable(grids.len() / 2);
                grids.retain(|_, (_, used)| *used > cutoff);
            }
            slot
        };
        slot.get_or_init(|| {
            let grid = Arc::new(ColumnGrid::sample(noise, key));
            let mut ranges = self.ranges.write().unwrap();
            if ranges.len() >= RANGES_KEPT {
                ranges.clear();
            }
            ranges.insert(column, (grid.lowest, grid.highest));
            grid
        })
        .clone()
    }

    /// The lowest and highest ground under `key`'s column, if a node over it has been
    /// sampled. Never blocks.
    pub fn height_range(&self, key: LodKey) -> Option<(f32, f32)> {
        self.ranges.read().unwrap().get(&ColumnKey::from(key)).copied()
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
            assert_eq!(noise.get(bevy::math::I64Vec2::new(x, z)) as f32 as f64, grid.height(cx, cz));
        }
        assert!(grid.lowest <= grid.highest);
    }

    #[test]
    fn test_stacked_nodes_share_a_grid_and_record_its_range() {
        let noise = NoiseGenerator::new(4);
        let cache = ColumnCache::default();
        assert_eq!(None, cache.height_range(key(2, 1, 0, 1)));
        let low = cache.grid(&noise, key(2, 1, 0, 1));
        let high = cache.grid(&noise, key(2, 1, 9, 1));
        assert!(Arc::ptr_eq(&low, &high));
        assert_eq!(Some((low.lowest, low.highest)), cache.height_range(key(2, 1, 5, 1)));
        assert_eq!(None, cache.height_range(key(2, 2, 0, 1)));
    }
}
