use std::{
    fmt::Debug,
    sync::{Arc, RwLock},
};

use bevy::{
    ecs::system::Resource,
    math::{I64Vec3, Vec3},
};

use crate::chunks::generate::noise::NoiseGenerator;

use super::chunks::chunk::{ChunkCoordinate, ChunkData, ChunkOctree};

#[derive(Resource)]
pub struct World {
    seed: u32,
    pub height: u64,
    chunks: ChunkOctree,
    pub noise_generator: Arc<RwLock<NoiseGenerator>>,
}

impl World {
    pub fn new() -> Self {
        let seed = rand::random();
        Self {
            seed,
            height: 256,
            chunks: ChunkOctree::default(),
            noise_generator: Arc::new(RwLock::new(NoiseGenerator::new(seed))),
        }
    }

    pub fn seed(&self) -> u32 {
        self.seed
    }

    pub fn insert_chunk(
        &mut self,
        chunk_coord: ChunkCoordinate,
        chunk_data: ChunkData,
    ) -> Arc<ChunkData> {
        self.chunks.set_chunk_data(chunk_coord, chunk_data)
    }

    pub fn get_chunk_data(&mut self, chunk_coord: ChunkCoordinate) -> Option<Arc<ChunkData>> {
        self.chunks.get_chunk_data(chunk_coord)
    }

    pub fn clear_chunk(&mut self, chunk_coord: ChunkCoordinate) {
        self.chunks.clear_chunk(chunk_coord)
    }

    pub fn adjacent_chunk_data(
        &mut self,
        chunk_coord: ChunkCoordinate,
    ) -> Vec<Option<Arc<ChunkData>>> {
        chunk_coord
            .adjacent()
            .iter()
            .map(|coord| self.get_chunk_data(*coord))
            .collect()
    }

    pub fn is_chunk_generated(&mut self, chunk_coord: ChunkCoordinate) -> bool {
        self.chunks.get_chunk_data(chunk_coord).is_some()
    }

    pub fn is_chunk_empty(&mut self, chunk_coord: ChunkCoordinate) -> bool {
        self.chunks
            .get_chunk_data(chunk_coord)
            .map(|chunk_data| chunk_data.empty())
            .unwrap_or(false)
    }

    pub fn chunk_to_world(&self, chunk_coord: ChunkCoordinate) -> Vec3 {
        self.chunks.chunk_centre(chunk_coord)
    }

    pub fn block_to_chunk_coordinate(&self, block_coord: I64Vec3) -> ChunkCoordinate {
        (block_coord / self.chunks.chunk_size as i64).into()
    }
}

impl Debug for World {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("World").field("seed", &self.seed).finish()
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn test_block_to_chunk_coordinate() {}

    #[test]
    fn test_is_chunk_generated() {}

    #[test]
    fn test_generate_chunk_updates_chunk_data() {}

    #[test]
    fn test_generate_chunk_mesh_none_for_ungenerated_chunk() {}

    #[test]
    fn test_generate_chunk_mesh_some_for_generated_chunk() {}
}
