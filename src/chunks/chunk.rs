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
            octree: Octree::new(1024.0, 7),
            cache: HashMap::new(),
            chunk_size,
        }
    }
}

impl ChunkOctree {
    pub fn get_chunk_data(&mut self, coord: ChunkCoordinate) -> Option<Arc<ChunkData>> {
        let octant = if self.cache.contains_key(&coord) {
            self.octree.get_node_by_id(*self.cache.get(&coord).unwrap())
        } else {
            self.octree.query_octant(self.chunk_centre(coord))
        };

        let read = octant.read().unwrap();
        self.cache.insert(coord, read.id());
        read.get_data()
    }

    pub fn set_chunk_data(&mut self, coord: ChunkCoordinate, chunk_data: ChunkData) {
        let chunk_octant = self.octree.query_octant(self.chunk_centre(coord));

        let mut write = chunk_octant.write().unwrap();
        write.set_data(Arc::new(chunk_data));
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
    use bevy::math::{I64Vec3, U16Vec3};

    use crate::world::chunk::BlockType;

    use super::{ChunkCoordinate, ChunkData, ChunkOctree};

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
}
