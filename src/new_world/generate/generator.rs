use std::sync::Arc;

use bevy::{
    math::{I64Vec2, I64Vec3, U16Vec3, Vec3},
    render::{
        mesh::{Indices, Mesh, VertexAttributeValues},
        render_asset::RenderAssetUsages,
    },
};
use noise::NoiseFn;

use crate::new_world::{
    block::{BlockType, BLOCK_COUNT},
    chunk::ChunkData,
    world::World,
};
use crate::{new_world::chunk::ChunkCoordinate, util::primitives::Vertex};

use super::noise::NoiseGenerator;

pub struct WorldGenerator {
    world_height: u64,
    noise_generator: NoiseGenerator,
}

impl Default for WorldGenerator {
    fn default() -> Self {
        Self {
            world_height: 256,
            noise_generator: Default::default(),
        }
    }
}

impl WorldGenerator {
    pub fn generate_chunk_mesh(
        &self,
        chunk: &ChunkData,
        adjacent_chunks: Vec<Option<Arc<ChunkData>>>,
        chunk_coord: ChunkCoordinate,
    ) -> Mesh {
        let mut vertices: Vec<Vertex> = vec![];
        let mut indices: Vec<u32> = vec![];

        let mut add_vertices = |vs: &[Vertex], position: Vec3, block_type: BlockType| {
            let uv_scale = 1.0 / (BLOCK_COUNT - 1) as f32;

            let triangle_start: u32 = vertices.len() as u32;
            vertices.extend(&mut vs.iter().map(|v| Vertex {
                position: (Vec3::from(v.position) + position).into(),
                normal: v.normal,
                uv: [
                    uv_scale * (v.uv[0] + (block_type as usize - 1) as f32),
                    v.uv[1],
                ],
            }));
            indices.extend(vec![
                triangle_start,
                triangle_start + 1,
                triangle_start + 2,
                triangle_start + 2,
                triangle_start + 1,
                triangle_start + 3,
            ]);
        };

        let cube_vertices = crate::util::primitives::cube();
        let face_vertices = [
            &cube_vertices[0..4],   // front
            &cube_vertices[4..8],   // right
            &cube_vertices[8..12],  // left
            &cube_vertices[12..16], // back
            &cube_vertices[16..20], // top
            &cube_vertices[20..24], // bottom
        ];

        // let chunk = world.get_chunk_data(chunk_coord).unwrap();
        // TODO: get chunk boundaries working properly, remove -1
        for x in 0..chunk.size {
            for z in 0..chunk.size {
                for y in 0..chunk.size {
                    let block = chunk.get_block_at(U16Vec3::new(x, y, z));
                    if block == BlockType::Air {
                        continue;
                    }

                    let world_position = Vec3::new(x as f32, y as f32, z as f32);

                    let front = if z > 0 {
                        chunk.get_block_at(U16Vec3::new(x, y, z - 1))
                    } else {
                        let adjacent = &adjacent_chunks[1].as_ref().unwrap();
                        adjacent.get_block_at(U16Vec3::new(x, y, adjacent.size - 1))
                    };

                    let back = if z < chunk.size - 1 {
                        chunk.get_block_at(U16Vec3::new(x, y, z + 1))
                    } else {
                        let adjacent = &adjacent_chunks[0].as_ref().unwrap();
                        adjacent.get_block_at(U16Vec3::new(x, y, 0))
                    };

                    let left = if x > 0 {
                        chunk.get_block_at(U16Vec3::new(x - 1, y, z))
                    } else {
                        let adjacent = &adjacent_chunks[3].as_ref().unwrap();
                        adjacent.get_block_at(U16Vec3::new(adjacent.size - 1, y, z))
                    };

                    let right = if x < chunk.size - 1 {
                        chunk.get_block_at(U16Vec3::new(x + 1, y, z))
                    } else {
                        let adjacent = &adjacent_chunks[2].as_ref().unwrap();
                        adjacent.get_block_at(U16Vec3::new(0, y, z))
                    };

                    let top = if y < chunk.size - 1 {
                        chunk.get_block_at(U16Vec3::new(x, y + 1, z))
                    } else {
                        let adjacent = &adjacent_chunks[4].as_ref().unwrap();
                        adjacent.get_block_at(U16Vec3::new(x, 0, z))
                    };

                    let bottom = if y > 0 {
                        chunk.get_block_at(U16Vec3::new(x, y - 1, z))
                    } else {
                        let adjacent = &adjacent_chunks[5].as_ref().unwrap();
                        adjacent.get_block_at(U16Vec3::new(x, adjacent.size - 1, z))
                    };

                    let sides = [front, right, left, back, top, bottom];
                    for (i, side) in sides.iter().enumerate() {
                        match side {
                            BlockType::Water => {
                                if block != BlockType::Water {
                                    add_vertices(&face_vertices[i], world_position, block)
                                }
                            }
                            BlockType::Air => {
                                add_vertices(&face_vertices[i], world_position, block)
                            }
                            _ => (),
                        };
                    }
                }
            }
        }

        let mut mesh = Mesh::new(
            bevy::render::mesh::PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
        );
        mesh.insert_indices(Indices::U32(indices));
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_POSITION,
            VertexAttributeValues::Float32x3(vertices.iter().map(|v| v.position).collect()),
        );
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_NORMAL,
            VertexAttributeValues::Float32x3(vertices.iter().map(|v| v.normal).collect()),
        );
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_UV_0,
            VertexAttributeValues::Float32x2(vertices.iter().map(|v| v.uv).collect()),
        );
        mesh
    }

    pub fn generate_chunk(
        &mut self,
        chunk_pos: ChunkCoordinate,
        noise_fn: &impl NoiseFn<f64, 2>,
    ) -> ChunkData {
        let mut chunk_data = ChunkData::default();

        for x in 0..chunk_data.size {
            for z in 0..chunk_data.size {
                let (world_x, world_y, world_z) = (
                    chunk_pos.0.x * chunk_data.size as i64 + x as i64,
                    chunk_pos.0.y * chunk_data.size as i64,
                    chunk_pos.0.z * chunk_data.size as i64 + z as i64,
                );
                let noise_val = self
                    .noise_generator
                    .get(I64Vec2::new(world_x, world_z), noise_fn);

                let world_height = (noise_val * self.world_height as f64).round() as u64;
                let chunk_height = if world_y > 0 {
                    let positive_y = world_y as u64;
                    (world_height - positive_y.min(world_height)).min(chunk_data.size as u64)
                } else {
                    chunk_data.size as u64
                };

                let gradient_x = (self.world_height as f64
                    * (self
                        .noise_generator
                        .get(I64Vec2::new(world_x + 1, world_z), noise_fn)
                        - self
                            .noise_generator
                            .get(I64Vec2::new(world_x - 1, world_z), noise_fn)))
                .abs();
                let gradient_z = (self.world_height as f64
                    * (self
                        .noise_generator
                        .get(I64Vec2::new(world_x, world_z + 1), noise_fn)
                        - self
                            .noise_generator
                            .get(I64Vec2::new(world_x, world_z - 1), noise_fn)))
                .abs();

                for y in 0..chunk_height {
                    let world_y = world_y + y as i64;
                    let block = if world_y >= 90 && ((gradient_x + gradient_z) <= 2.0) {
                        BlockType::Snow
                    } else if world_y >= 70 && ((gradient_x + gradient_z) >= 2.0) {
                        BlockType::Stone
                    } else if world_y as i64 == 0 {
                        BlockType::Water
                    } else if world_y <= 10 {
                        BlockType::Sand
                    } else {
                        BlockType::Grass
                    };
                    chunk_data.set_block_at(U16Vec3::new(x, y as u16, z), block);
                }
            }
        }

        chunk_data
    }
}
