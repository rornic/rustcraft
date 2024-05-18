use std::sync::Arc;

use bevy::{
    ecs::{component::Component, system::Resource},
    math::{I64Vec3, Vec3},
    render::mesh::Mesh,
};

use super::{
    chunk::{ChunkCoordinate, ChunkData, ChunkOctree},
    generate::generator::WorldGenerator,
};

#[derive(Resource)]
pub struct World {
    chunks: ChunkOctree,
    generator: WorldGenerator,
}

impl World {
    pub fn new() -> Self {
        Self {
            chunks: ChunkOctree::default(),
            generator: WorldGenerator::default(),
        }
    }

    // pub fn get_block_at(&mut self, block_coord: I64Vec3) -> BlockType {
    //     let chunk = self.block_to_chunk_coordinate(block_coord);

    //     if let Some(chunk_data) = self.chunks.get_chunk_data(chunk) {
    //         return chunk_data.get_block_at(self.block_to_chunk_local(block_coord));
    //     }

    //     BlockType::Air
    // }
    //

    pub fn generate_chunk(&mut self, chunk_coord: ChunkCoordinate) {
        let chunk_data = self.generator.generate_chunk(chunk_coord);
        self.chunks.set_chunk_data(chunk_coord, chunk_data);
    }

    pub fn generate_chunk_mesh(&mut self, chunk_coord: ChunkCoordinate) -> Option<Mesh> {
        self.chunks
            .get_chunk_data(chunk_coord)
            .and_then(|chunk_data| {
                Some(self.generator.generate_chunk_mesh(&chunk_data, chunk_coord))
            })
    }

    pub fn is_chunk_generated(&mut self, chunk_coord: ChunkCoordinate) -> bool {
        self.chunks.get_chunk_data(chunk_coord).is_some()
    }

    pub fn is_chunk_dirty(&mut self, chunk_coord: ChunkCoordinate) -> bool {
        let chunk_data = self.chunks.get_chunk_data(chunk_coord);
        if let Some(chunk_data) = chunk_data {
            return chunk_data.dirty;
        }
        return false;
    }

    pub fn block_to_chunk_coordinate(&self, block_coord: I64Vec3) -> ChunkCoordinate {
        (block_coord / self.chunks.chunk_size as i64).into()
    }

    fn block_to_chunk_local(&self, block_coord: I64Vec3) -> ChunkCoordinate {
        (block_coord / self.chunks.chunk_size as i64).into()
    }
}
