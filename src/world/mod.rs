use std::collections::HashMap;

use cgmath::{Vector2, Vector3};
use noise::NoiseFn;

use crate::render::mesh::{Mesh, Vertex};
use crate::{vector2, vector3};

pub mod ecs;
mod generator;

/// Each chunk is a cube of blocks. `CHUNK_SIZE` determines the size of this cube in blocks.
pub const CHUNK_SIZE: usize = 16;

#[derive(PartialEq, Eq, Copy, Clone)]
pub enum BlockType {
    Air,
    Stone,
    Grass,
    Sand,
    Water,
    Snow,
}

impl BlockType {
    pub fn is_solid(&self) -> bool {
        match self {
            BlockType::Water | BlockType::Air => false,
            _ => true,
        }
    }
}

const BLOCK_COUNT: usize = 6;

type Chunk = Box<[[[BlockType; CHUNK_SIZE]; WORLD_HEIGHT]; CHUNK_SIZE]>;

const WORLD_HEIGHT: usize = 256;

#[derive(Default)]
pub struct World {
    generator: WorldGenerator,
    chunks: HashMap<Vector2<i32>, Chunk>,
}

impl World {
    pub fn cache_chunk(&mut self, chunk_position: Vector2<i32>, chunk: Chunk) {
        self.chunks.insert(chunk_position, chunk);
    }

    pub fn generate_chunk(&self, chunk_position: Vector2<i32>) -> Chunk {
        self.generator.generate_chunk(chunk_position)
    }

    pub fn is_chunk_generated(&self, chunk_position: Vector2<i32>) -> bool {
        self.chunks.contains_key(&chunk_position)
    }

    pub fn are_neighbours_generated(&self, chunk: Vector2<i32>) -> bool {
        [[0, 1], [0, -1], [1, 0], [-1, 0]]
            .iter()
            .all(|p| self.is_chunk_generated(chunk + vector2!(p[0], p[1])))
    }

    fn generate_chunk_mesh(&self, chunk_pos: Vector2<i32>) -> Mesh {
        let mut vertices: Vec<Vertex> = vec![];
        let mut indices: Vec<u32> = vec![];

        let mut add_vertices = |vs: &[Vertex], position: Vector3<f32>, block_type: BlockType| {
            let uv_scale = 1.0 / (BLOCK_COUNT - 1) as f32;

            let triangle_start: u32 = vertices.len() as u32;
            vertices.extend(&mut vs.iter().map(|v| Vertex {
                position: (Vector3::from(v.position) + position).into(),
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

        let cube = super::render::primitives::cube();
        let face_vertices = [
            &cube.vertices[0..4],   // front
            &cube.vertices[4..8],   // right
            &cube.vertices[8..12],  // left
            &cube.vertices[12..16], // back
            &cube.vertices[16..20], // top
            &cube.vertices[20..24], // bottom
        ];

        let chunk = self.chunk(chunk_pos).unwrap();
        let adjacent_chunks = [
            self.chunk(chunk_pos + vector2!(0, 1)),
            self.chunk(chunk_pos + vector2!(0, -1)),
            self.chunk(chunk_pos + vector2!(1, 0)),
            self.chunk(chunk_pos + vector2!(-1, 0)),
        ];

        for x in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                for y in 0..WORLD_HEIGHT {
                    if chunk[x][y][z] == BlockType::Air {
                        continue;
                    }

                    let world_position = vector3!(
                        chunk_pos.x as f32 * CHUNK_SIZE as f32 + x as f32,
                        y as f32,
                        chunk_pos.y as f32 * CHUNK_SIZE as f32 + z as f32
                    );

                    let front = z
                        .checked_sub(1)
                        .and_then(|z| self.chunk_block(chunk, vector3!(x, y, z)))
                        .or(adjacent_chunks[1]
                            .and_then(|c| self.chunk_block(c, vector3!(x, y, CHUNK_SIZE - 1))));
                    let back =
                        self.chunk_block(chunk, vector3!(x, y, z + 1))
                            .or(adjacent_chunks[0]
                                .and_then(|c| self.chunk_block(c, vector3!(x, y, 0))));
                    let left = x
                        .checked_sub(1)
                        .and_then(|x| self.chunk_block(chunk, vector3!(x, y, z)))
                        .or(adjacent_chunks[3]
                            .and_then(|c| self.chunk_block(c, vector3!(CHUNK_SIZE - 1, y, z))));
                    let right =
                        self.chunk_block(chunk, vector3!(x + 1, y, z))
                            .or(adjacent_chunks[2]
                                .and_then(|c| self.chunk_block(c, vector3!(0, y, z))));
                    let top = self.chunk_block(chunk, vector3!(x, y + 1, z));

                    let bottom = if y == 0 {
                        Some(BlockType::Stone)
                    } else {
                        y.checked_sub(1)
                            .and_then(|y| self.chunk_block(chunk, vector3!(x, y, z)))
                    };

                    let sides = [front, right, left, back, top, bottom];
                    for (i, side) in sides.iter().enumerate() {
                        match side {
                            Some(BlockType::Water) => {
                                if chunk[x][y][z] != BlockType::Water {
                                    add_vertices(&face_vertices[i], world_position, chunk[x][y][z])
                                }
                            }
                            None | Some(BlockType::Air) => {
                                add_vertices(&face_vertices[i], world_position, chunk[x][y][z])
                            }
                            _ => (),
                        };
                    }
                }
            }
        }
        Mesh::new(vertices, indices)
    }

    fn block_at(&self, position: Vector3<f32>) -> BlockType {
        let (chunk_pos, block_pos) = self.world_to_block(position);
        if let Some(chunk) = self.chunks.get(&chunk_pos) {
            chunk[block_pos.x][block_pos.y][block_pos.z]
        } else {
            BlockType::Air
        }
    }

    fn chunk(&self, chunk_position: Vector2<i32>) -> Option<&Chunk> {
        self.chunks.get(&chunk_position)
    }

    fn chunk_block<'a>(&self, chunk: &'a Chunk, block: Vector3<usize>) -> Option<BlockType> {
        chunk
            .get(block.x)
            .and_then(|c| c.get(block.y).and_then(|c| c.get(block.z)))
            .copied()
    }

    /// Takes a position in the world and returns the chunk that it's in.
    fn world_to_chunk(&self, world_position: Vector3<f32>) -> Vector2<i32> {
        vector2!(
            (world_position.x / CHUNK_SIZE as f32).floor() as i32,
            (world_position.z / CHUNK_SIZE as f32).floor() as i32
        )
    }

    /// Takes a position in the world and converts it to a position relative to the chunk it's in.
    fn world_to_block(&self, world_position: Vector3<f32>) -> (Vector2<i32>, Vector3<usize>) {
        let chunk = self.world_to_chunk(world_position);
        let relative_pos = vector3!(
            (world_position.x - (chunk.x * CHUNK_SIZE as i32) as f32).floor() as usize,
            world_position.y.floor() as usize,
            (world_position.z - (chunk.y * CHUNK_SIZE as i32) as f32).floor() as usize
        );
        (chunk, relative_pos)
    }

    fn block_centre(&self, world_position: Vector3<f32>) -> Vector3<f32> {
        vector3!(
            world_position.x.round(),
            world_position.y.round(),
            world_position.z.round()
        )
    }
}

#[derive(Copy, Clone)]
struct WorldGenerator {
    seed: u32,
}

impl Default for WorldGenerator {
    fn default() -> Self {
        Self {
            seed: rand::random(),
        }
    }
}

impl WorldGenerator {
    fn generate_chunk(&self, chunk_pos: Vector2<i32>) -> Chunk {
        let mut blocks = [[[BlockType::Air; CHUNK_SIZE]; WORLD_HEIGHT]; CHUNK_SIZE];

        let noise = generator::noise_generator(self.seed);

        for x in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                let (world_x, _, world_z) = (
                    chunk_pos.x * CHUNK_SIZE as i32 + x as i32,
                    0,
                    chunk_pos.y * CHUNK_SIZE as i32 + z as i32,
                );
                let noise_val = noise.get([world_x as f64, world_z as f64]);

                let height = (noise_val * WORLD_HEIGHT as f64).round() as usize;
                let gradient_x = (WORLD_HEIGHT as f64
                    * (noise.get([(world_x + 1) as f64, world_z as f64])
                        - noise.get([(world_x - 1) as f64, world_z as f64])))
                .abs();
                let gradient_z = (WORLD_HEIGHT as f64
                    * (noise.get([world_x as f64, (world_z + 1) as f64])
                        - noise.get([world_x as f64, (world_z - 1) as f64])))
                .abs();

                // Height must be at least 1!
                let height = height.min(WORLD_HEIGHT - 1).max(1);
                for y in 0..height {
                    if height >= 180 && ((gradient_x + gradient_z) <= 3.0) {
                        blocks[x][y][z] = BlockType::Snow;
                    } else if y >= 30 && ((gradient_x + gradient_z) >= 2.0) {
                        blocks[x][y][z] = BlockType::Stone;
                    } else if y < 10 {
                        blocks[x][y][z] = BlockType::Sand;
                    } else {
                        blocks[x][y][z] = BlockType::Grass;
                    }
                }

                for y in height..5 {
                    blocks[x][y][z] = BlockType::Water;
                }
            }
        }

        Box::new(blocks)
    }
}
