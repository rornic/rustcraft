use std::collections::HashMap;
use std::sync::Arc;

use cgmath::{Vector2, Vector3};
use noise::{Add, Multiply, NoiseFn, OpenSimplex, Perlin, RidgedMulti, Seedable};

use crate::render::mesh::{Mesh, Vertex};
use crate::{vector2, vector3};

pub mod ecs;
mod generator;

/// Each chunk is a cube of blocks. `CHUNK_SIZE` determines the size of this cube in blocks.
pub const CHUNK_SIZE: usize = 16;
type Chunk = Box<[[[bool; CHUNK_SIZE]; WORLD_HEIGHT]; CHUNK_SIZE]>;

const WORLD_HEIGHT: usize = 128;

#[derive(Default)]
pub struct World {
    generator: WorldGenerator,
    chunks: HashMap<Vector2<i32>, Chunk>,
    chunk_meshes: HashMap<Vector2<i32>, Arc<Mesh>>,
}

impl World {
    /// Generates a chunk at a given chunk coordinate and returns a reference to it.
    ///
    /// Returns `None` if this chunk has already been generated.
    pub fn generate_chunk(&mut self, chunk_position: Vector2<i32>) -> Option<&Chunk> {
        if !self.chunks.contains_key(&chunk_position) {
            let chunk = self.generator.generate_chunk(chunk_position);
            self.chunks.insert(chunk_position, chunk);
            self.chunks.get(&chunk_position)
        } else {
            None
        }
    }

    pub fn is_chunk_generated(&self, chunk_position: Vector2<i32>) -> bool {
        self.chunks.get(&chunk_position).is_some()
    }

    pub fn chunk_mesh(&mut self, chunk_position: Vector2<i32>) -> Arc<Mesh> {
        if !self.chunk_meshes.contains_key(&chunk_position) {
            let mesh = self.generate_chunk_mesh(chunk_position);
            self.chunk_meshes.insert(chunk_position, Arc::new(mesh));
        }
        self.chunk_meshes.get(&chunk_position).unwrap().clone()
    }

    /// Generates a `Mesh` for a chunk.
    fn generate_chunk_mesh(&self, chunk_pos: Vector2<i32>) -> Mesh {
        let mut vertices: Vec<Vertex> = vec![];
        let mut indices: Vec<u32> = vec![];

        let mut add_vertices = |vs: &[Vertex], position: Vector3<f32>| {
            let triangle_start: u32 = vertices.len() as u32;
            vertices.extend(&mut vs.iter().map(|v| Vertex {
                position: (Vector3::from(v.position) + position).into(),
                normal: v.normal,
                uv: v.uv,
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
                    if !chunk[x][y][z] {
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
                    let bottom = y
                        .checked_sub(1)
                        .and_then(|y| self.chunk_block(chunk, vector3!(x, y, z)));

                    // Front faces
                    if let Some(false) = front {
                        add_vertices(&face_vertices[0], world_position);
                    }

                    // Back faces
                    if let Some(false) = back {
                        add_vertices(face_vertices[3], world_position);
                    }

                    // Left faces
                    if let Some(false) = left {
                        add_vertices(face_vertices[2], world_position);
                    }
                    // Right faces
                    if let Some(false) = right {
                        add_vertices(face_vertices[1], world_position);
                    }

                    // Bottom faces
                    if let Some(false) = bottom {
                        add_vertices(face_vertices[5], world_position);
                    }
                    // Top faces
                    if let Some(false) = top {
                        add_vertices(face_vertices[4], world_position);
                    }
                }
            }
        }
        Mesh::new(vertices, indices)
    }

    fn block_at(&self, position: Vector3<f32>) -> bool {
        let (chunk_pos, block_pos) = self.world_to_block(position);
        if let Some(chunk) = self.chunks.get(&chunk_pos) {
            chunk[block_pos.x][block_pos.y][block_pos.z]
        } else {
            false
        }
    }

    fn chunk(&self, chunk_position: Vector2<i32>) -> Option<&Chunk> {
        self.chunks.get(&chunk_position)
    }

    fn chunk_block<'a>(&self, chunk: &'a Chunk, block: Vector3<usize>) -> Option<&'a bool> {
        chunk
            .get(block.x)
            .and_then(|c| c.get(block.y).and_then(|c| c.get(block.z)))
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
}

#[derive(Default, Copy, Clone)]
struct WorldGenerator {}

impl WorldGenerator {
    fn generate_chunk(&self, chunk_pos: Vector2<i32>) -> Chunk {
        let mut blocks = [[[false; CHUNK_SIZE]; WORLD_HEIGHT]; CHUNK_SIZE];

        let noise = generator::noise_generator();

        for x in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                let (world_x, _, world_z) = (
                    chunk_pos.x * CHUNK_SIZE as i32 + x as i32,
                    0,
                    chunk_pos.y * CHUNK_SIZE as i32 + z as i32,
                );
                let noise_val = noise.get([world_x as f64, world_z as f64]);
                let height = (noise_val * WORLD_HEIGHT as f64).round() as usize;

                // Height must be at least 1!
                let height = height.min(WORLD_HEIGHT - 1).max(1);
                for y in 0..height {
                    blocks[x][y][z] = true;
                }
            }
        }
        Box::new(blocks)
    }
}
