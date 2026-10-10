use std::{fmt::Debug, sync::Arc};

use bevy::{
    ecs::system::Resource,
    math::{I64Vec3, U16Vec3, Vec3},
};

use crate::block::BlockType;
use crate::chunks::generate::noise::NoiseGenerator;
use crate::chunks::generate::region_store::{resident_cap_for, RegionStore};
use crate::chunks::generate::WORLD_HEIGHT;

use super::chunks::chunk::{ChunkCoordinate, ChunkData, ChunkOctree};

#[derive(Resource)]
pub struct World {
    seed: u32,
    pub height: u64,
    chunks: ChunkOctree,
    pub region_store: Arc<RegionStore>,
    spawn: Vec3,
}

impl World {
    /// `render_distance` is how far, in chunks, terrain will be drawn.
    pub fn new(render_distance: u32) -> Self {
        let seed = rand::random();
        let height = WORLD_HEIGHT;
        let noise_generator = Arc::new(NoiseGenerator::new(seed));
        let (spawn_x, spawn_z, spawn_height) = noise_generator.find_spawn();
        let chunks = ChunkOctree::default();
        let render_distance_blocks = render_distance as i64 * chunks.chunk_size as i64;
        Self {
            seed,
            height,
            chunks,
            region_store: Arc::new(
                RegionStore::new(noise_generator)
                    .with_resident_cap(resident_cap_for(render_distance_blocks)),
            ),
            spawn: Vec3::new(spawn_x as f32, spawn_height as f32 + 20.0, spawn_z as f32),
        }
    }

    pub fn seed(&self) -> u32 {
        self.seed
    }

    pub fn spawn(&self) -> Vec3 {
        self.spawn
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
            .neighbors_26()
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
        block_coord
            .div_euclid(I64Vec3::splat(self.chunks.chunk_size as i64))
            .into()
    }

    pub fn get_block_at(&mut self, world_block_coord: I64Vec3) -> BlockType {
        let chunk_size = self.chunks.chunk_size as i64;
        // div_euclid/rem_euclid, not `/` and `%` - those truncate toward zero, which puts
        // negative coordinates in the wrong chunk and yields a negative (then
        // wrapping-to-huge-u16) local coordinate.
        let chunk_coord = ChunkCoordinate(I64Vec3::new(
            world_block_coord.x.div_euclid(chunk_size),
            world_block_coord.y.div_euclid(chunk_size),
            world_block_coord.z.div_euclid(chunk_size),
        ));
        let Some(chunk_data) = self.get_chunk_data(chunk_coord) else {
            return BlockType::Air; // ungenerated chunk under the camera isn't water
        };
        let local = U16Vec3::new(
            world_block_coord.x.rem_euclid(chunk_size) as u16,
            world_block_coord.y.rem_euclid(chunk_size) as u16,
            world_block_coord.z.rem_euclid(chunk_size) as u16,
        );
        chunk_data.get_block_at(local)
    }
}

impl Debug for World {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("World").field("seed", &self.seed).finish()
    }
}

#[cfg(test)]
mod tests {
    use bevy::math::I64Vec3;

    use super::World;
    use crate::chunks::chunk::ChunkCoordinate;

    #[test]
    fn test_block_to_chunk_coordinate() {
        let world = World::new(16);
        let chunk = |x, y, z| world.block_to_chunk_coordinate(I64Vec3::new(x, y, z));
        assert_eq!(ChunkCoordinate(I64Vec3::new(0, 1, 2)), chunk(5, 16, 47));
        // negative blocks belong to the chunk below, not chunk 0
        assert_eq!(ChunkCoordinate(I64Vec3::new(-1, -1, -2)), chunk(-5, -16, -17));
    }

    #[test]
    fn test_is_chunk_generated() {}

    #[test]
    fn test_generate_chunk_updates_chunk_data() {}

    #[test]
    fn test_generate_chunk_mesh_none_for_ungenerated_chunk() {}

    #[test]
    fn test_generate_chunk_mesh_some_for_generated_chunk() {}
}
