use std::collections::HashMap;
use std::thread;

use cgmath::Vector3;
use noise::{Add, Multiply, NoiseFn, OpenSimplex, Perlin, Seedable};
use specs::{Builder, WorldExt};

use crate::render::mesh::{Mesh, Vertex};
use crate::render::{RenderMesh, Renderer};
use crate::{vector3, vertex};

use self::components::Transform;

pub mod components;
pub mod systems;

/// Each chunk is a cube of blocks. `CHUNK_SIZE` determines the size of this cube in blocks.
const CHUNK_SIZE: usize = 16;

type ChunkBlocks = Box<[[[bool; CHUNK_SIZE]; CHUNK_SIZE]; CHUNK_SIZE]>;
#[derive(Clone)]
struct Chunk {
    blocks: ChunkBlocks,
}

impl Chunk {}

/// `WORLD_SIZE` determines the size of the world in chunks.
const WORLD_SIZE: usize = 16;
pub struct World {
    chunks: HashMap<Vector3<i32>, Chunk>,
}

impl World {
    pub fn new() -> World {
        let world_generator = WorldGenerator {};

        let mut chunk_handles = vec![];
        for chunk_x in 0..WORLD_SIZE {
            for chunk_y in 0..WORLD_SIZE {
                for chunk_z in 0..WORLD_SIZE {
                    chunk_handles.push(thread::spawn(move || {
                        let chunk_pos = vector3!(chunk_x as i32, chunk_y as i32, chunk_z as i32);
                        let chunk = world_generator.generate_chunk(chunk_pos);
                        (chunk_pos, chunk)
                    }));
                }
            }
        }

        let mut world = World {
            chunks: HashMap::new(),
        };
        for handle in chunk_handles {
            let (pos, chunk) = handle.join().unwrap();
            world.chunks.insert(pos, chunk);
        }

        world
    }

    pub fn generate_chunk_meshes(&self, renderer: &mut Renderer, world: &mut specs::World) {
        let mut world_vertices: Vec<Vertex> = vec![];
        let mut world_indices: Vec<u32> = vec![];

        // Combines meshes for all chunks into a single mesh
        for (chunk_pos, _) in self.chunks.iter() {
            let chunk_mesh = self.generate_chunk_mesh(*chunk_pos);
            world_indices.append(
                &mut chunk_mesh
                    .indices
                    .iter()
                    .map(|i| *i + world_vertices.len() as u32)
                    .collect(),
            );
            world_vertices.append(&mut chunk_mesh.vertices.iter()
            .map(|v| vertex!(position: v.position + vector3!(chunk_pos.x as f32 * CHUNK_SIZE as f32, chunk_pos.y as f32 * CHUNK_SIZE as f32, chunk_pos.z as f32 * CHUNK_SIZE as f32), normal: v.normal, uv: v.uv)).collect());
        }

        let world_mesh = Mesh::new(world_vertices, world_indices);
        renderer.register_mesh(&world_mesh).unwrap();

        world
            .create_entity()
            .with(Transform::default())
            .with(RenderMesh::new(&world_mesh))
            .build();
    }

    /// Generates a `Mesh` for a chunk.
    fn generate_chunk_mesh(&self, chunk_pos: Vector3<i32>) -> Mesh {
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
            for y in 0..CHUNK_SIZE {
                for z in 0..CHUNK_SIZE {
                    let position = vector3!(x as f32, y as f32, z as f32);
                    let world_position = vector3!(
                        (chunk_pos.x * CHUNK_SIZE as i32 + x as i32) as f32,
                        (chunk_pos.y * CHUNK_SIZE as i32 + y as i32) as f32,
                        (chunk_pos.z * CHUNK_SIZE as i32 + z as i32) as f32
                    );

                    if !self.block_at(world_position) {
                        continue;
                    }

                    // Front faces
                    if !self.block_at(world_position + vector3!(0.0, 0.0, -1.0)) {
                        add_vertices(&mut face_vertices[0].clone(), position);
                    }
                    // Back faces
                    if !self.block_at(world_position + vector3!(0.0, 0.0, 1.0)) {
                        add_vertices(&mut face_vertices[3].clone(), position);
                    }

                    // Left faces
                    if !self.block_at(world_position + vector3!(-1.0, 0.0, 0.0)) {
                        add_vertices(&mut face_vertices[2].clone(), position);
                    }
                    // Right faces
                    if !self.block_at(world_position + vector3!(1.0, 0.0, 0.0)) {
                        add_vertices(&mut face_vertices[1].clone(), position);
                    }

                    // Bottom faces
                    if !self.block_at(world_position + vector3!(0.0, -1.0, 0.0)) {
                        add_vertices(&mut face_vertices[5].clone(), position);
                    }
                    // Top faces
                    if !self.block_at(world_position + vector3!(0.0, 1.0, 0.0)) {
                        add_vertices(&mut face_vertices[4].clone(), position);
                    }
                }
            }
        }
        Mesh::new(vertices, indices)
    }

    fn block_at(&self, position: Vector3<f32>) -> bool {
        let (chunk_pos, block_pos) = self.world_to_block(position);
        if let Some(chunk) = self.chunks.get(&chunk_pos) {
            chunk.blocks[block_pos.x][block_pos.y][block_pos.z]
        } else {
            false
        }
    }

    /// Takes a position in the world and returns the chunk that it's in.
    fn world_to_chunk(&self, world_position: Vector3<f32>) -> Vector3<i32> {
        vector3!(
            (world_position.x / CHUNK_SIZE as f32).floor() as i32,
            (world_position.y / CHUNK_SIZE as f32).floor() as i32,
            (world_position.z / CHUNK_SIZE as f32).floor() as i32
        )
    }

    /// Takes a position in the world and converts it to a position relative to the chunk it's in.
    fn world_to_block(&self, world_position: Vector3<f32>) -> (Vector3<i32>, Vector3<usize>) {
        let chunk = self.world_to_chunk(world_position);
        let relative_pos = vector3!(
            (world_position.x - (chunk.x * CHUNK_SIZE as i32) as f32).floor() as usize,
            (world_position.y - (chunk.y * CHUNK_SIZE as i32) as f32).floor() as usize,
            (world_position.z - (chunk.z * CHUNK_SIZE as i32) as f32).floor() as usize
        );
        (chunk, relative_pos)
    }
}

#[derive(Copy, Clone)]
struct WorldGenerator {}

impl WorldGenerator {
    fn generate_chunk(&self, chunk_pos: Vector3<i32>) -> Chunk {
        let mut blocks = [[[false; CHUNK_SIZE]; CHUNK_SIZE]; CHUNK_SIZE];

        let perlin = Perlin::new().set_seed(1);
        let perlin2 = Perlin::new().set_seed(2);
        let simplex = OpenSimplex::new().set_seed(3);
        let mul = Add::new(&perlin2, &simplex);
        let noise = Multiply::new(&mul, &perlin);

        for x in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                let (world_x, world_y, world_z) = (
                    chunk_pos.x * CHUNK_SIZE as i32 + x as i32,
                    chunk_pos.y * CHUNK_SIZE as i32,
                    chunk_pos.z * CHUNK_SIZE as i32 + z as i32,
                );
                let height = ((0.5 + noise.get([world_x as f64 / 128.0, world_z as f64 / 128.0]))
                    * 128.0)
                    .round() as i32;
                // println!("{},{},{} {}", world_x, world_y, world_z, height);
                if height > world_y {
                    let diff = (height - world_y).abs() as usize;
                    for y in 0..diff.min(CHUNK_SIZE) {
                        blocks[x][y][z] = true;
                    }
                }
            }
        }
        Chunk {
            blocks: Box::new(blocks),
        }
    }
}
