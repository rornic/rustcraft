use bevy::math::{I64Vec3, U16Vec3, Vec3};

use crate::block::BlockType;
use crate::chunks::generate::trees::TreeTints;

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
    /// The centre of the chunk, in blocks.
    pub fn centre(self) -> Vec3 {
        (self.0.as_vec3() + Vec3::splat(0.5)) * CHUNK_SIZE as f32
    }

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
    /// The trees reaching into the chunk, to tint their leaves, see `TreeTints`.
    pub trees: TreeTints,
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
            trees: TreeTints::default(),
        }
    }
}

impl ChunkData {
    /// A chunk of every cell's block in turn, x outermost and y innermost (see `linear_index`).
    pub fn from_cells(cells: impl IntoIterator<Item = BlockType>) -> Self {
        let mut runs: Vec<Run> = Vec::new();
        for block in cells {
            match runs.last_mut() {
                Some(run) if run.block == block => run.length += 1,
                _ => runs.push(Run { block, length: 1 }),
            }
        }
        let chunk = Self { runs, ..Self::default() };
        assert_eq!(chunk.runs.iter().map(|r| r.length as usize).sum::<usize>(), chunk.linear_index(U16Vec3::splat(chunk.size - 1)) + 1);
        chunk
    }

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

    /// Whether every block hides what's behind it, so the chunk shows nothing past its own faces.
    pub fn opaque(&self) -> bool {
        self.runs.iter().all(|r| r.block.is_opaque())
    }

    /// Bytes held on the heap for the chunk's blocks and trees.
    pub fn heap_size(&self) -> usize {
        self.runs.capacity() * size_of::<Run>() + self.trees.heap_size()
    }


    // Replaces the old `blocks().iter()` contract (the underlying map never stored Air):
    // yields every non-air block's coordinate and type, in no particular order.
    pub fn iter_non_air(&self) -> impl Iterator<Item = (U16Vec3, BlockType)> + '_ {
        let mut start = 0usize;
        self.runs.iter().flat_map(move |run| {
            let first = start;
            start += run.length as usize;
            let cells = if run.block == BlockType::Air { first..first } else { first..start };
            cells.map(move |i| (self.coord_from_index(i), run.block))
        })
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

#[cfg(test)]
mod tests {
    use bevy::math::{I64Vec3, U16Vec3, Vec3};

    use crate::block::BlockType;

    use super::{neighbor_26_index, ChunkCoordinate, ChunkData};

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
    fn test_from_cells_matches_setting_each_block() {
        let block_at = |c: U16Vec3| match (c.x + 3 * c.y + c.z) % 7 {
            0 => BlockType::Stone,
            1 | 2 => BlockType::Dirt,
            _ => BlockType::Air,
        };
        let mut set = ChunkData::default();
        let mut cells = Vec::new();
        for x in 0..set.size {
            for z in 0..set.size {
                for y in 0..set.size {
                    let c = U16Vec3::new(x, y, z);
                    set.set_block_at(c, block_at(c));
                    cells.push(block_at(c));
                }
            }
        }
        let built = ChunkData::from_cells(cells);
        assert_eq!(set.runs.len(), built.runs.len());
        for x in 0..set.size {
            for y in 0..set.size {
                for z in 0..set.size {
                    let c = U16Vec3::new(x, y, z);
                    assert_eq!(set.get_block_at(c), built.get_block_at(c));
                }
            }
        }
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
    fn test_chunk_centre() {
        assert_eq!(Vec3::new(8.0, 8.0, 8.0), ChunkCoordinate(I64Vec3::new(0, 0, 0)).centre());
        assert_eq!(Vec3::new(-8.0, -8.0, 8.0), ChunkCoordinate(I64Vec3::new(-1, -1, 0)).centre());
        assert_eq!(Vec3::new(-680.0, 360.0, -1592.0), ChunkCoordinate(I64Vec3::new(-43, 22, -100)).centre());
    }
}
