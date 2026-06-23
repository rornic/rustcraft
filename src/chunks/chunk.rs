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

type BlockPalette = HashMap<U16Vec3, BlockType>;

pub struct ChunkData {
    blocks: BlockPalette,
    pub size: u16,
    pub dirty: bool,
}

pub const CHUNK_SIZE: u16 = 16;

impl Default for ChunkData {
    fn default() -> Self {
        Self {
            blocks: HashMap::new(),
            size: CHUNK_SIZE,
            dirty: false,
        }
    }
}

impl ChunkData {
    fn is_block_in_chunk(&self, block_coord: U16Vec3) -> bool {
        return block_coord.x < self.size && block_coord.y < self.size && block_coord.z < self.size;
    }

    pub fn empty(&self) -> bool {
        self.blocks.is_empty()
    }

    pub fn blocks(&self) -> &BlockPalette {
        &self.blocks
    }

    pub fn get_block_at(&self, block_coord: U16Vec3) -> BlockType {
        if !self.is_block_in_chunk(block_coord) {
            panic!("get block {:?} not in chunk", block_coord);
        }

        return *self.blocks.get(&block_coord).unwrap_or(&BlockType::Air);
    }

    pub fn set_block_at(&mut self, block_coord: U16Vec3, block_type: BlockType) {
        if !self.is_block_in_chunk(block_coord) {
            panic!("set block {:?} not in chunk", block_coord);
        }

        self.blocks.insert(block_coord, block_type);
        self.dirty = true;
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
    pub fn get_chunk_data(&mut self, coord: ChunkCoordinate) -> Option<Arc<ChunkData>> {
        let octant = if let Some(&id) = self.cache.get(&coord) {
            self.octree.get_node_by_id(id)
        } else {
            let octant = self.octree.query_octant(self.chunk_centre(coord));
            self.cache.insert(coord, octant.read().unwrap().id());
            octant
        };

        let data = octant.read().unwrap().get_data();
        data
    }

    pub fn set_chunk_data(
        &mut self,
        coord: ChunkCoordinate,
        chunk_data: ChunkData,
    ) -> Arc<ChunkData> {
        let chunk_octant = self.octree.query_octant(self.chunk_centre(coord));

        let chunk_data = Arc::new(chunk_data);
        let mut write = chunk_octant.write().unwrap();
        write.set_data(chunk_data.clone());
        chunk_data
    }

    // only clears the leaf's data; the underlying octree node (and its ancestors)
    // is never freed/collapsed, so unloading chunks does not reclaim tree memory.
    // A real fix needs parent pointers plus a debounce window to avoid thrashing
    // as chunks repeatedly cross the render-distance boundary every frame.
    pub fn clear_chunk(&mut self, coord: ChunkCoordinate) {
        let chunk_octant = self.octree.query_octant(self.chunk_centre(coord));

        let mut write = chunk_octant.write().unwrap();
        write.clear_data();
        self.cache.remove(&coord);
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

        assert_eq!(1, chunk_data.blocks.len());
        assert_eq!(
            BlockType::Grass,
            *chunk_data.blocks.get(&U16Vec3::new(4, 12, 5)).unwrap()
        )
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
