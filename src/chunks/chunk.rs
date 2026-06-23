use std::sync::Arc;

use bevy::{
    math::{I64Vec3, U16Vec3, Vec3},
    utils::HashMap,
};

use crate::block::BlockType;
use crate::util::octree::Octree;

#[derive(Debug, Copy, Clone, Hash, PartialEq, Eq)]
pub struct ChunkCoordinate(pub I64Vec3);

impl From<ChunkCoordinate> for Vec3 {
    fn from(value: ChunkCoordinate) -> Self {
        Self::new(value.0.x as f32, value.0.y as f32, value.0.z as f32)
    }
}

impl From<I64Vec3> for ChunkCoordinate {
    fn from(value: I64Vec3) -> Self {
        Self(value)
    }
}

impl ChunkCoordinate {
    pub fn adjacent(&self) -> Vec<ChunkCoordinate> {
        vec![
            ChunkCoordinate(self.0 + I64Vec3::new(0, 0, 1)),
            ChunkCoordinate(self.0 + I64Vec3::new(0, 0, -1)),
            ChunkCoordinate(self.0 + I64Vec3::new(1, 0, 0)),
            ChunkCoordinate(self.0 + I64Vec3::new(-1, 0, 0)),
            ChunkCoordinate(self.0 + I64Vec3::new(0, 1, 0)),
            ChunkCoordinate(self.0 + I64Vec3::new(0, -1, 0)),
        ]
    }

    // All 26 chunks in the 3x3x3 neighborhood (face + edge + corner adjacent), needed so mesh
    // generation can sample diagonal-neighbor blocks for vertex AO at chunk boundaries. Ordered
    // by `neighbor_26_index` so callers can index straight into the result.
    pub fn neighbors_26(&self) -> Vec<ChunkCoordinate> {
        let mut result = vec![*self; 26];
        for dx in -1..=1i64 {
            for dy in -1..=1i64 {
                for dz in -1..=1i64 {
                    if dx == 0 && dy == 0 && dz == 0 {
                        continue;
                    }
                    let idx = neighbor_26_index(dx as i32, dy as i32, dz as i32);
                    result[idx] = ChunkCoordinate(self.0 + I64Vec3::new(dx, dy, dz));
                }
            }
        }
        result
    }
}

// Maps a unit offset (dx,dy,dz), each in {-1,0,1} and not all zero, to a stable 0..26 index.
pub fn neighbor_26_index(dx: i32, dy: i32, dz: i32) -> usize {
    let idx3 = ((dx + 1) * 9 + (dy + 1) * 3 + (dz + 1)) as usize;
    if idx3 < 13 {
        idx3
    } else {
        idx3 - 1
    }
}

// One block-type run in a chunk's linear cell ordering (see `ChunkData::linear_index`).
// `length` covers at most size^3 = 4096 cells, comfortably within u16.
#[derive(Debug, Clone, Copy)]
struct Run {
    block: BlockType,
    length: u16,
}

pub struct ChunkData {
    // Run-length encoded instead of a per-block map: most chunks are either fully
    // uniform (e.g. deep underground, see generate_chunk) or banded, so this is far
    // smaller in practice than one entry per block, at the cost of `get`/`set` becoming
    // an O(runs) scan instead of O(1) - see `locate`.
    runs: Vec<Run>,
    pub size: u16,
    pub dirty: bool,
}

pub const CHUNK_SIZE: u16 = 16;

impl Default for ChunkData {
    fn default() -> Self {
        let size = CHUNK_SIZE;
        Self {
            runs: vec![Run {
                block: BlockType::Air,
                length: size * size * size,
            }],
            size,
            dirty: false,
        }
    }
}

impl ChunkData {
    fn is_block_in_chunk(&self, block_coord: U16Vec3) -> bool {
        return block_coord.x < self.size && block_coord.y < self.size && block_coord.z < self.size;
    }

    // x outer, z middle, y inner - matches generate_chunk's nested loop order, so terrain
    // bands form long runs and sequential column-by-column generation tends to extend
    // the tail run rather than fragment the run list.
    fn linear_index(&self, coord: U16Vec3) -> usize {
        let size = self.size as usize;
        coord.x as usize * size * size + coord.z as usize * size + coord.y as usize
    }

    fn coord_from_index(&self, index: usize) -> U16Vec3 {
        let size = self.size as usize;
        let x = index / (size * size);
        let rem = index % (size * size);
        let z = rem / size;
        let y = rem % size;
        U16Vec3::new(x as u16, y as u16, z as u16)
    }

    // Finds the run covering `index`, returning its position in `runs` and the linear
    // index it starts at. Run counts stay small in practice (a uniform chunk is 1 run,
    // banded terrain is tens), so a linear scan is fine - never worse than the old
    // per-block map, and exact in the common case instead of needing a separate offset
    // index kept in sync.
    fn locate(&self, index: usize) -> (usize, usize) {
        let mut start = 0;
        for (i, run) in self.runs.iter().enumerate() {
            let end = start + run.length as usize;
            if index < end {
                return (i, start);
            }
            start = end;
        }
        unreachable!("index {index} out of range for a {start}-cell chunk")
    }

    // Merges the run at `index` with an immediate same-type neighbour on either side, if
    // present - keeps run count from growing unboundedly under repeated scattered edits.
    fn merge_at(&mut self, index: usize) {
        if index + 1 < self.runs.len() && self.runs[index + 1].block == self.runs[index].block {
            self.runs[index].length += self.runs[index + 1].length;
            self.runs.remove(index + 1);
        }
        if index > 0 && self.runs[index - 1].block == self.runs[index].block {
            self.runs[index - 1].length += self.runs[index].length;
            self.runs.remove(index);
        }
    }

    pub fn empty(&self) -> bool {
        self.runs.iter().all(|r| r.block == BlockType::Air)
    }

    // Replaces the old `blocks().iter()` contract (the underlying map never stored Air):
    // yields every non-air block's coordinate and type, in no particular order.
    pub fn iter_non_air(&self) -> impl Iterator<Item = (U16Vec3, BlockType)> {
        let mut result = Vec::new();
        let mut start = 0usize;
        for run in &self.runs {
            if run.block != BlockType::Air {
                for i in start..start + run.length as usize {
                    result.push((self.coord_from_index(i), run.block));
                }
            }
            start += run.length as usize;
        }
        result.into_iter()
    }

    pub fn get_block_at(&self, block_coord: U16Vec3) -> BlockType {
        if !self.is_block_in_chunk(block_coord) {
            panic!("get block {:?} not in chunk", block_coord);
        }

        let (run_index, _) = self.locate(self.linear_index(block_coord));
        self.runs[run_index].block
    }

    pub fn set_block_at(&mut self, block_coord: U16Vec3, block_type: BlockType) {
        if !self.is_block_in_chunk(block_coord) {
            panic!("set block {:?} not in chunk", block_coord);
        }
        self.dirty = true;

        let index = self.linear_index(block_coord);
        let (run_index, run_start) = self.locate(index);
        let existing = self.runs[run_index];
        if existing.block == block_type {
            return; // already correct - no structural change needed
        }

        let offset = index - run_start;
        let before_len = offset;
        let after_len = existing.length as usize - offset - 1;

        let mut replacement = Vec::with_capacity(3);
        if before_len > 0 {
            replacement.push(Run {
                block: existing.block,
                length: before_len as u16,
            });
        }
        let target_pos = replacement.len();
        replacement.push(Run {
            block: block_type,
            length: 1,
        });
        if after_len > 0 {
            replacement.push(Run {
                block: existing.block,
                length: after_len as u16,
            });
        }

        self.runs.splice(run_index..=run_index, replacement);
        self.merge_at(run_index + target_pos);
    }
}

pub struct ChunkOctree {
    octree: Octree<ChunkData>,
    cache: HashMap<ChunkCoordinate, usize>,
    pub chunk_size: u16,
}

impl Default for ChunkOctree {
    fn default() -> Self {
        let chunk_size = 16;
        Self {
            octree: Octree::new(1_048_576.0, 16),
            cache: HashMap::new(),
            chunk_size,
        }
    }
}

impl ChunkOctree {
    // cache-only: `set_chunk_data` is the only thing that ever subdivides the tree
    // or inserts a cache entry, so a cache miss always means "no data" - no need to
    // walk the tree on every read, and (more importantly) it means every arena node
    // either has data or is an ancestor of one that does, which is what makes
    // `clear_chunk`'s collapse safe (nothing else can be left pointing at a node it
    // removes).
    pub fn get_chunk_data(&mut self, coord: ChunkCoordinate) -> Option<Arc<ChunkData>> {
        let &id = self.cache.get(&coord)?;
        self.octree.get_node_by_id(id).read().unwrap().get_data()
    }

    pub fn set_chunk_data(
        &mut self,
        coord: ChunkCoordinate,
        chunk_data: ChunkData,
    ) -> Arc<ChunkData> {
        let chunk_octant = self.octree.query_octant(self.chunk_centre(coord));
        self.cache.insert(coord, chunk_octant.read().unwrap().id());

        let chunk_data = Arc::new(chunk_data);
        let mut write = chunk_octant.write().unwrap();
        write.set_data(chunk_data.clone());
        chunk_data
    }

    pub fn clear_chunk(&mut self, coord: ChunkCoordinate) {
        if let Some(id) = self.cache.remove(&coord) {
            self.octree.clear_data(id);
        }
    }

    pub fn chunk_centre(&self, chunk_coord: ChunkCoordinate) -> Vec3 {
        let chunk_size = self.chunk_size as f32;
        Vec3::new(
            chunk_coord.0.x as f32 * chunk_size + chunk_size / 2.0,
            chunk_coord.0.y as f32 * chunk_size + chunk_size / 2.0,
            chunk_coord.0.z as f32 * chunk_size + chunk_size / 2.0,
        )
    }
}

#[cfg(test)]
mod tests {
    use bevy::math::{I64Vec3, U16Vec3, Vec3};

    use crate::block::BlockType;

    use super::{neighbor_26_index, ChunkCoordinate, ChunkData, ChunkOctree};

    #[test]
    fn test_neighbors_26_returns_26_unique_coordinates() {
        let origin = ChunkCoordinate(I64Vec3::ZERO);
        let neighbors = origin.neighbors_26();

        assert_eq!(26, neighbors.len());
        let unique: std::collections::HashSet<_> = neighbors.iter().map(|c| c.0).collect();
        assert_eq!(26, unique.len());
        assert!(!neighbors.iter().any(|c| c.0 == I64Vec3::ZERO));
    }

    #[test]
    fn test_neighbors_26_indexed_by_neighbor_26_index() {
        let origin = ChunkCoordinate(I64Vec3::ZERO);
        let neighbors = origin.neighbors_26();

        let idx = neighbor_26_index(1, -1, 0);
        assert_eq!(I64Vec3::new(1, -1, 0), neighbors[idx].0);
    }

    #[test]
    #[should_panic]
    fn test_get_block_at_checks_limit() {
        let chunk_data = ChunkData::default();
        chunk_data.get_block_at(U16Vec3::new(16, 0, 16));
    }

    #[test]
    fn test_get_block_at_returns_air_when_empty() {
        let chunk_data = ChunkData::default();
        let block = chunk_data.get_block_at(U16Vec3::new(4, 12, 5));
        assert_eq!(BlockType::Air, block);
    }

    #[test]
    fn test_set_block_at_updates_correct_block() {
        let mut chunk_data = ChunkData::default();
        chunk_data.set_block_at(U16Vec3::new(4, 12, 5), BlockType::Grass);

        assert_eq!(BlockType::Grass, chunk_data.get_block_at(U16Vec3::new(4, 12, 5)));
        assert_eq!(BlockType::Air, chunk_data.get_block_at(U16Vec3::new(0, 0, 0)));
    }

    #[test]
    fn test_set_then_get_round_trip() {
        let mut chunk_data = ChunkData::default();
        chunk_data.set_block_at(U16Vec3::new(7, 8, 9), BlockType::Stone);

        assert_eq!(BlockType::Stone, chunk_data.get_block_at(U16Vec3::new(7, 8, 9)));
    }

    #[test]
    fn test_set_block_mid_run_splits_without_disturbing_neighbours() {
        let mut chunk_data = ChunkData::default();
        // a single Air run covers everything by default - set the middle cell of one
        // column and confirm the cells immediately above/below it are untouched.
        chunk_data.set_block_at(U16Vec3::new(0, 5, 0), BlockType::Stone);

        assert_eq!(BlockType::Air, chunk_data.get_block_at(U16Vec3::new(0, 4, 0)));
        assert_eq!(BlockType::Stone, chunk_data.get_block_at(U16Vec3::new(0, 5, 0)));
        assert_eq!(BlockType::Air, chunk_data.get_block_at(U16Vec3::new(0, 6, 0)));
    }

    #[test]
    fn test_set_block_back_to_neighbour_type_merges_runs() {
        let mut chunk_data = ChunkData::default();
        chunk_data.set_block_at(U16Vec3::new(0, 5, 0), BlockType::Stone);
        assert_eq!(3, chunk_data.runs.len()); // air, stone, air

        chunk_data.set_block_at(U16Vec3::new(0, 5, 0), BlockType::Air);
        assert_eq!(1, chunk_data.runs.len()); // back to a single uniform run
    }

    #[test]
    fn test_filling_entire_chunk_with_one_type_is_a_single_run() {
        let mut chunk_data = ChunkData::default();
        for x in 0..chunk_data.size {
            for y in 0..chunk_data.size {
                for z in 0..chunk_data.size {
                    chunk_data.set_block_at(U16Vec3::new(x, y, z), BlockType::Stone);
                }
            }
        }

        assert_eq!(1, chunk_data.runs.len());
        assert_eq!(BlockType::Stone, chunk_data.get_block_at(U16Vec3::new(0, 0, 0)));
    }

    #[test]
    fn test_empty_is_true_for_default_chunk_and_after_reset_to_air() {
        let mut chunk_data = ChunkData::default();
        assert!(chunk_data.empty());

        chunk_data.set_block_at(U16Vec3::new(1, 1, 1), BlockType::Sand);
        assert!(!chunk_data.empty());

        chunk_data.set_block_at(U16Vec3::new(1, 1, 1), BlockType::Air);
        assert!(chunk_data.empty());
    }

    #[test]
    fn test_iter_non_air_yields_exactly_the_set_blocks() {
        let mut chunk_data = ChunkData::default();
        chunk_data.set_block_at(U16Vec3::new(1, 1, 1), BlockType::Sand);
        chunk_data.set_block_at(U16Vec3::new(2, 2, 2), BlockType::Stone);

        let mut found: Vec<_> = chunk_data.iter_non_air().collect();
        found.sort_by_key(|(coord, _)| (coord.x, coord.y, coord.z));

        assert_eq!(
            vec![
                (U16Vec3::new(1, 1, 1), BlockType::Sand),
                (U16Vec3::new(2, 2, 2), BlockType::Stone),
            ],
            found
        );
    }

    #[test]
    fn test_set_block_at_makes_chunk_dirty() {
        let mut chunk_data = ChunkData::default();
        assert!(!chunk_data.dirty);

        chunk_data.set_block_at(U16Vec3::ZERO, BlockType::Snow);
        assert!(chunk_data.dirty);
    }

    #[test]
    fn test_set_get_chunk_data() {
        let mut octree = ChunkOctree::default();

        let mut chunk_data = ChunkData::default();
        chunk_data.set_block_at(U16Vec3::new(5, 4, 9), BlockType::Sand);
        octree.set_chunk_data(ChunkCoordinate(I64Vec3::new(3, 2, 1)), chunk_data);

        let queried_chunk_data = octree
            .get_chunk_data(ChunkCoordinate(I64Vec3::new(3, 2, 1)))
            .unwrap();

        assert_eq!(
            BlockType::Sand,
            queried_chunk_data.get_block_at(U16Vec3::new(5, 4, 9))
        );
        assert_eq!(
            BlockType::Air,
            queried_chunk_data.get_block_at(U16Vec3::new(0, 4, 9))
        );
    }

    #[test]
    fn test_set_get_chunk_data_far_from_origin() {
        let mut octree = ChunkOctree::default();

        let mut chunk_data = ChunkData::default();
        chunk_data.set_block_at(U16Vec3::new(2, 3, 4), BlockType::Sand);
        octree.set_chunk_data(ChunkCoordinate(I64Vec3::new(10_000, -5_000, 20_000)), chunk_data);

        let queried_chunk_data = octree
            .get_chunk_data(ChunkCoordinate(I64Vec3::new(10_000, -5_000, 20_000)))
            .unwrap();

        assert_eq!(
            BlockType::Sand,
            queried_chunk_data.get_block_at(U16Vec3::new(2, 3, 4))
        );
    }

    #[test]
    fn test_clear_chunk_then_get_returns_none() {
        let mut octree = ChunkOctree::default();
        let coord = ChunkCoordinate(I64Vec3::new(3, 2, 1));

        octree.set_chunk_data(coord, ChunkData::default());
        assert!(octree.get_chunk_data(coord).is_some());

        octree.clear_chunk(coord);

        assert!(octree.get_chunk_data(coord).is_none());
    }

    #[test]
    fn test_clear_chunk_leaves_no_cache_entry() {
        let mut octree = ChunkOctree::default();
        let coord = ChunkCoordinate(I64Vec3::new(3, 2, 1));

        octree.set_chunk_data(coord, ChunkData::default());
        octree.clear_chunk(coord);

        assert!(!octree.cache.contains_key(&coord));
    }

    #[test]
    fn test_chunk_centre() {
        let octree = ChunkOctree::default();

        assert_eq!(
            Vec3::new(8.0, 8.0, 8.0),
            octree.chunk_centre(ChunkCoordinate(I64Vec3::new(0, 0, 0)))
        );
        assert_eq!(
            Vec3::new(-8.0, -8.0, 8.0),
            octree.chunk_centre(ChunkCoordinate(I64Vec3::new(-1, -1, 0)))
        );
        assert_eq!(
            Vec3::new(-680.0, 360.0, -1592.0),
            octree.chunk_centre(ChunkCoordinate(I64Vec3::new(-43, 22, -100)))
        )
    }
}
