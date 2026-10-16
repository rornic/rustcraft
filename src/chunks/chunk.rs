use bevy::math::{I64Vec3, U16Vec3, Vec3};

use crate::block::BlockType;
use crate::chunks::generate::trees::TreeTints;

#[derive(Debug, Copy, Clone, Hash, PartialEq, Eq)]
pub struct ChunkCoordinate(pub I64Vec3);

impl ChunkCoordinate {
    /// The chunk holding `block`, rounding negative coordinates down.
    pub fn containing(block: I64Vec3) -> Self {
        Self(block.div_euclid(I64Vec3::splat(CHUNK_SIZE as i64)))
    }

    /// The centre of the chunk, in blocks.
    pub fn centre(self) -> Vec3 {
        (self.0.as_vec3() + Vec3::splat(0.5)) * CHUNK_SIZE as f32
    }

    pub fn adjacent(self) -> [ChunkCoordinate; 6] {
        [
            I64Vec3::Z,
            I64Vec3::NEG_Z,
            I64Vec3::X,
            I64Vec3::NEG_X,
            I64Vec3::Y,
            I64Vec3::NEG_Y,
        ]
        .map(|offset| ChunkCoordinate(self.0 + offset))
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
    // Run-length encoded in `linear_index` order: most chunks are uniform or banded, so this is
    // far smaller than one entry per block, at the cost of `get`/`set` scanning the runs.
    runs: Vec<Run>,
    /// The trees reaching into the chunk, to tint their leaves, see `TreeTints`.
    pub trees: TreeTints,
}

pub const CHUNK_SIZE: u16 = 16;
pub const CELLS: usize = (CHUNK_SIZE as usize).pow(3);

// x outer, z middle, y inner - matches generate_chunk's loop order, so terrain bands form long runs.
fn linear_index(coord: U16Vec3) -> usize {
    assert!(
        coord.max_element() < CHUNK_SIZE,
        "block {coord:?} not in chunk"
    );
    let size = CHUNK_SIZE as usize;
    (coord.x as usize * size + coord.z as usize) * size + coord.y as usize
}

fn coord_from_index(index: usize) -> U16Vec3 {
    let size = CHUNK_SIZE as usize;
    U16Vec3::new(
        (index / (size * size)) as u16,
        (index % size) as u16,
        (index / size % size) as u16,
    )
}

impl Default for ChunkData {
    fn default() -> Self {
        Self {
            runs: vec![Run {
                block: BlockType::Air,
                length: CELLS as u16,
            }],
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
        assert_eq!(CELLS, runs.iter().map(|r| r.length as usize).sum::<usize>());
        Self {
            runs,
            trees: TreeTints::default(),
        }
    }

    // The run covering `index`, as its position in `runs` and the index it starts at. Chunks
    // hold few runs (a uniform one is 1, banded terrain tens), so a linear scan is fine.
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

    /// Every non-air block's coordinate and type.
    pub fn iter_non_air(&self) -> impl Iterator<Item = (U16Vec3, BlockType)> + '_ {
        let mut start = 0usize;
        self.runs.iter().flat_map(move |run| {
            let first = start;
            start += run.length as usize;
            let end = if run.block == BlockType::Air {
                first
            } else {
                start
            };
            (first..end).map(move |i| (coord_from_index(i), run.block))
        })
    }

    /// Writes every cell's block to `out`, in `linear_index` order.
    pub fn decode_into(&self, out: &mut [BlockType; CELLS]) {
        let mut start = 0;
        for run in &self.runs {
            let end = start + run.length as usize;
            out[start..end].fill(run.block);
            start = end;
        }
    }

    pub fn get_block_at(&self, coord: U16Vec3) -> BlockType {
        self.runs[self.locate(linear_index(coord)).0].block
    }

    pub fn set_block_at(&mut self, coord: U16Vec3, block: BlockType) {
        let index = linear_index(coord);
        let (at, start) = self.locate(index);
        let existing = self.runs[at];
        if existing.block == block {
            return;
        }
        let before = index - start;
        let after = existing.length as usize - before - 1;
        let run = |block, length: usize| Run {
            block,
            length: length as u16,
        };
        let pieces = [
            run(existing.block, before),
            run(block, 1),
            run(existing.block, after),
        ];
        let used = (before == 0) as usize..3 - (after == 0) as usize;
        self.runs.splice(at..=at, pieces[used].iter().copied());
        // merge the new run with a same-type neighbour either side, so edits don't fragment the runs
        let at = at + (before > 0) as usize;
        if at + 1 < self.runs.len() && self.runs[at + 1].block == block {
            self.runs[at].length += self.runs.remove(at + 1).length;
        }
        if at > 0 && self.runs[at - 1].block == block {
            self.runs[at - 1].length += self.runs.remove(at).length;
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy::math::{I64Vec3, U16Vec3, Vec3};

    use crate::block::BlockType;

    use super::{neighbor_26_index, ChunkCoordinate, ChunkData, CHUNK_SIZE};

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

        assert_eq!(
            BlockType::Grass,
            chunk_data.get_block_at(U16Vec3::new(4, 12, 5))
        );
        assert_eq!(
            BlockType::Air,
            chunk_data.get_block_at(U16Vec3::new(0, 0, 0))
        );
    }

    #[test]
    fn test_set_then_get_round_trip() {
        let mut chunk_data = ChunkData::default();
        chunk_data.set_block_at(U16Vec3::new(7, 8, 9), BlockType::Stone);

        assert_eq!(
            BlockType::Stone,
            chunk_data.get_block_at(U16Vec3::new(7, 8, 9))
        );
    }

    #[test]
    fn test_set_block_mid_run_splits_without_disturbing_neighbours() {
        let mut chunk_data = ChunkData::default();
        // a single Air run covers everything by default - set the middle cell of one
        // column and confirm the cells immediately above/below it are untouched.
        chunk_data.set_block_at(U16Vec3::new(0, 5, 0), BlockType::Stone);

        assert_eq!(
            BlockType::Air,
            chunk_data.get_block_at(U16Vec3::new(0, 4, 0))
        );
        assert_eq!(
            BlockType::Stone,
            chunk_data.get_block_at(U16Vec3::new(0, 5, 0))
        );
        assert_eq!(
            BlockType::Air,
            chunk_data.get_block_at(U16Vec3::new(0, 6, 0))
        );
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
        for x in 0..CHUNK_SIZE {
            for y in 0..CHUNK_SIZE {
                for z in 0..CHUNK_SIZE {
                    chunk_data.set_block_at(U16Vec3::new(x, y, z), BlockType::Stone);
                }
            }
        }

        assert_eq!(1, chunk_data.runs.len());
        assert_eq!(
            BlockType::Stone,
            chunk_data.get_block_at(U16Vec3::new(0, 0, 0))
        );
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
        for x in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                for y in 0..CHUNK_SIZE {
                    let c = U16Vec3::new(x, y, z);
                    set.set_block_at(c, block_at(c));
                    cells.push(block_at(c));
                }
            }
        }
        let built = ChunkData::from_cells(cells);
        assert_eq!(set.runs.len(), built.runs.len());
        for x in 0..CHUNK_SIZE {
            for y in 0..CHUNK_SIZE {
                for z in 0..CHUNK_SIZE {
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
    fn test_chunk_containing() {
        let chunk = |x, y, z| ChunkCoordinate::containing(I64Vec3::new(x, y, z)).0;
        assert_eq!(I64Vec3::new(0, 1, 2), chunk(5, 16, 47));
        // negative blocks belong to the chunk below, not chunk 0
        assert_eq!(I64Vec3::new(-1, -1, -2), chunk(-5, -16, -17));
    }

    #[test]
    fn test_chunk_centre() {
        assert_eq!(
            Vec3::new(8.0, 8.0, 8.0),
            ChunkCoordinate(I64Vec3::new(0, 0, 0)).centre()
        );
        assert_eq!(
            Vec3::new(-8.0, -8.0, 8.0),
            ChunkCoordinate(I64Vec3::new(-1, -1, 0)).centre()
        );
        assert_eq!(
            Vec3::new(-680.0, 360.0, -1592.0),
            ChunkCoordinate(I64Vec3::new(-43, 22, -100)).centre()
        );
    }
}
