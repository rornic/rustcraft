use bevy::{
    math::{I64Vec2, U16Vec3, Vec3},
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

use super::noise::noise_generator;

pub struct WorldGenerator {
    seed: u32,
    world_height: u64,
}

impl Default for WorldGenerator {
    fn default() -> Self {
        Self {
            seed: rand::random(),
            world_height: 256,
        }
    }
}

impl WorldGenerator {
    pub fn generate_chunk_mesh(&self, chunk: &ChunkData, chunk_coord: ChunkCoordinate) -> Mesh {
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

        // let adjacent_chunks = [
        //     self.chunk(chunk_coord + I64Vec2::new(0, 1)),
        //     self.chunk(chunk_coord + I64Vec2::new(0, -1)),
        //     self.chunk(chunk_coord + I64Vec2::new(1, 0)),
        //     self.chunk(chunk_coord + I64Vec2::new(-1, 0)),
        // ];

        // let chunk = world.get_chunk_data(chunk_coord).unwrap();
        // TODO: get chunk boundaries working properly, remove -1
        for x in 0..chunk.size - 1 {
            for z in 0..chunk.size - 1 {
                for y in 0..chunk.size - 1 {
                    let block = chunk.get_block_at(U16Vec3::new(x, y, z));
                    if block == BlockType::Air {
                        continue;
                    }

                    let world_position = Vec3::new(x as f32, y as f32, z as f32);

                    let front = if let Some(z) = z.checked_sub(1) {
                        chunk.get_block_at(U16Vec3::new(x, y, z))
                    } else {
                        BlockType::Air
                    };
                    let back = if let Some(z) = z.checked_add(1) {
                        chunk.get_block_at(U16Vec3::new(x, y, z))
                    } else {
                        BlockType::Air
                    };
                    let left = if let Some(x) = x.checked_sub(1) {
                        chunk.get_block_at(U16Vec3::new(x, y, z))
                    } else {
                        BlockType::Air
                    };
                    let right = if let Some(x) = x.checked_add(1) {
                        chunk.get_block_at(U16Vec3::new(x, y, z))
                    } else {
                        BlockType::Air
                    };

                    let top = if y < chunk.size - 1 {
                        if let Some(y) = y.checked_add(1) {
                            chunk.get_block_at(U16Vec3::new(x, y, z))
                        } else {
                            BlockType::Air
                        }
                    } else {
                        BlockType::Air
                    };
                    let bottom = if let Some(y) = y.checked_sub(1) {
                        chunk.get_block_at(U16Vec3::new(x, y, z))
                    } else {
                        BlockType::Air
                    };
                    // let front = z
                    //     .checked_sub(1)
                    //     .and_then(|z| chunk.get_block_at(U16Vec3::new(x, y, z)).unwrap());
                    // .or(adjacent_chunks[1]
                    //     .and_then(|c| self.chunk_block(c, U16Vec3::new(x, y, CHUNK_SIZE - 1))));

                    // let back = self
                    //     .chunk_block(chunk, U16Vec3::new(x, y, z + 1))
                    //     .or(adjacent_chunks[0]
                    //         .and_then(|c| self.chunk_block(c, U16Vec3::new(x, y, 0))));
                    // let left = x
                    //     .checked_sub(1)
                    //     .and_then(|x| self.chunk_block(chunk, U16Vec3::new(x, y, z)))
                    //     .or(adjacent_chunks[3]
                    //         .and_then(|c| self.chunk_block(c, U16Vec3::new(CHUNK_SIZE - 1, y, z))));
                    // let right = self
                    //     .chunk_block(chunk, U16Vec3::new(x + 1, y, z))
                    //     .or(adjacent_chunks[2]
                    //         .and_then(|c| self.chunk_block(c, U16Vec3::new(0, y, z))));
                    // let top = self.chunk_block(chunk, U16Vec3::new(x, y + 1, z));

                    // let bottom = if y == 0 {
                    //     Some(BlockType::Stone)
                    // } else {
                    //     y.checked_sub(1)
                    //         .and_then(|y| self.chunk_block(chunk, U16Vec3::new(x, y, z)))
                    // };

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

    pub fn generate_chunk(&self, chunk_pos: ChunkCoordinate) -> ChunkData {
        let mut chunk_data = ChunkData::default();
        let noise = noise_generator(self.seed);

        for x in 0..chunk_data.size {
            for z in 0..chunk_data.size {
                let (world_x, world_y, world_z) = (
                    chunk_pos.0.x * chunk_data.size as i64 + x as i64,
                    chunk_pos.0.y as u64 * chunk_data.size as u64,
                    chunk_pos.0.z * chunk_data.size as i64 + z as i64,
                );
                let noise_val = noise.get([world_x as f64, world_z as f64]);

                let world_height = (noise_val * self.world_height as f64).round() as u64;
                let mut chunk_height = (world_height - world_y).min(chunk_data.size as u64 - 1);
                if chunk_height == 0 && world_y == 0 {
                    chunk_height = 1;
                }

                // let gradient_x = (WORLD_HEIGHT as f64
                //     * (noise.get([(world_x + 1) as f64, world_z as f64])
                //         - noise.get([(world_x - 1) as f64, world_z as f64])))
                // .abs();
                // let gradient_z = (WORLD_HEIGHT as f64
                //     * (noise.get([world_x as f64, (world_z + 1) as f64])
                //         - noise.get([world_x as f64, (world_z - 1) as f64])))
                // .abs();

                for y in 0..chunk_height {
                    // if height >= 180 && ((gradient_x + gradient_z) <= 2.0) {
                    //     blocks[x][y][z] = BlockType::Snow;
                    // } else if y >= 10 && ((gradient_x + gradient_z) >= 2.0) {
                    //     blocks[x][y][z] = BlockType::Stone;
                    // } else if y < 10 {
                    //     blocks[x][y][z] = BlockType::Sand;
                    // } else {
                    //     blocks[x][y][z] = BlockType::Grass;
                    // }
                    chunk_data.set_block_at(U16Vec3::new(x, y as u16, z), BlockType::Grass);
                }

                // for y in height..5 {
                //     blocks[x][y][z] = BlockType::Water;
                // }
            }
        }

        chunk_data
    }
}
