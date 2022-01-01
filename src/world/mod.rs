use std::collections::HashMap;
use std::thread;

use cgmath::{Vector2, Vector3};
use noise::{Add, Multiply, NoiseFn, OpenSimplex, Perlin, Seedable};

use crate::render::mesh::{Mesh, Vertex};
use crate::{vector2, vector3, vertex};

pub mod ecs;

/// Each chunk is a cube of blocks. `CHUNK_SIZE` determines the size of this cube in blocks.
pub const CHUNK_SIZE: usize = 16;
type Chunk = Box<[[[bool; CHUNK_SIZE]; WORLD_HEIGHT]; CHUNK_SIZE]>;

const WORLD_HEIGHT: usize = 255;

#[derive(Default)]
pub struct World {
    generator: WorldGenerator,
    chunks: HashMap<Vector2<i32>, Chunk>,
}

impl World {
    pub fn new() -> World {
        let world_generator = WorldGenerator {};

        let mut world = World {
            generator: world_generator,
            chunks: HashMap::new(),
        };

        for x in 0..10 {
            for z in 0..10 {
                world.generate_chunk(vector2!(x, z));
            }
        }

        world
    }

    /// Generates a chunk at a given chunk coordinate.
    ///
    /// Does nothing if this chunk has already been generated.
    pub fn generate_chunk(&mut self, chunk_position: Vector2<i32>) -> &Chunk {
        if !self.chunks.contains_key(&chunk_position) {
            let chunk = self.generator.generate_chunk(chunk_position);
            self.chunks.insert(chunk_position, chunk);
        }
        self.chunks.get(&chunk_position).unwrap()
    }

    /// Generates a `Mesh` for a chunk.
    fn generate_chunk_mesh(&self, chunk_pos: Vector2<i32>) -> Mesh {
        let mut vertices: Vec<Vertex> = vec![];
        let mut indices: Vec<u32> = vec![];

        let mut triangle_start: u32 = 0;
        let mut add_vertices = |vs: &[Vertex], position: Vector3<f32>| {
            vertices.append(
                &mut vs
                    .iter()
                    .map(|v| {
                        vertex!(
                                    position: v.position + position,
                                    normal: v.normal,
                                    uv: v.uv)
                    })
                    .collect(),
            );
            indices.append(&mut vec![
                triangle_start,
                triangle_start + 1,
                triangle_start + 2,
                triangle_start + 2,
                triangle_start + 1,
                triangle_start + 3,
            ]);
            triangle_start += 4;
        };

        let cube = super::render::mesh::primitives::cube();
        let face_vertices = [
            &cube.vertices[0..4],   // front
            &cube.vertices[4..8],   // right
            &cube.vertices[8..12],  // left
            &cube.vertices[12..16], // back
            &cube.vertices[16..20], // top
            &cube.vertices[20..24], // bottom
        ];

        for x in 0..CHUNK_SIZE {
            for y in 0..WORLD_HEIGHT {
                for z in 0..CHUNK_SIZE {
                    let position = vector3!(x as f32, y as f32, z as f32);
                    let world_position = vector3!(
                        (chunk_pos.x * CHUNK_SIZE as i32 + x as i32) as f32,
                        y as f32,
                        (chunk_pos.y * CHUNK_SIZE as i32 + z as i32) as f32
                    );

                    if !self.block_at(world_position) {
                        continue;
                    }

                    // Front faces
                    if !self.block_at(world_position + vector3!(0.0, 0.0, -1.0)) {
                        add_vertices(&mut face_vertices[0].clone(), world_position);
                    }
                    // Back faces
                    if !self.block_at(world_position + vector3!(0.0, 0.0, 1.0)) {
                        add_vertices(&mut face_vertices[3].clone(), world_position);
                    }

                    // Left faces
                    if !self.block_at(world_position + vector3!(-1.0, 0.0, 0.0)) {
                        add_vertices(&mut face_vertices[2].clone(), world_position);
                    }
                    // Right faces
                    if !self.block_at(world_position + vector3!(1.0, 0.0, 0.0)) {
                        add_vertices(&mut face_vertices[1].clone(), world_position);
                    }

                    // Bottom faces
                    if !self.block_at(world_position + vector3!(0.0, -1.0, 0.0)) {
                        add_vertices(&mut face_vertices[5].clone(), world_position);
                    }
                    // Top faces
                    if !self.block_at(world_position + vector3!(0.0, 1.0, 0.0)) {
                        add_vertices(&mut face_vertices[4].clone(), world_position);
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

        let perlin = Perlin::new().set_seed(1);
        let perlin2 = Perlin::new().set_seed(2);
        let simplex = OpenSimplex::new().set_seed(3);
        let mul = Add::new(&perlin2, &simplex);
        let noise = Multiply::new(&mul, &perlin);

        for x in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                let (world_x, world_y, world_z) = (
                    chunk_pos.x * CHUNK_SIZE as i32 + x as i32,
                    0,
                    chunk_pos.y * CHUNK_SIZE as i32 + z as i32,
                );
                let height = ((0.5 + noise.get([world_x as f64 / 128.0, world_z as f64 / 128.0]))
                    * 128.0)
                    .round() as usize;

                for y in 0..height {
                    blocks[x][y][z] = true;
                }
            }
        }
        Box::new(blocks)
    }
}
