use std::thread;

use cgmath::Quaternion;
use specs::{Builder, WorldExt};

use crate::render::mesh::{Mesh, Vertex};
use crate::render::{RenderMesh, Renderer};
use crate::{vector3, vertex};

use self::components::Transform;

pub mod components;
pub mod systems;

/// Each chunk is a cube of blocks. `CHUNK_SIZE` determines the size of this cube in blocks.
const CHUNK_SIZE: usize = 64;

type ChunkBlocks = Vec<Vec<Vec<bool>>>;
#[derive(Clone)]
struct Chunk {
    blocks: ChunkBlocks,
}

impl Chunk {
    /// Generates a `Mesh` for this chunk.
    fn generate_mesh(&self) -> Mesh {
        let mut vertices: Vec<Vertex> = vec![];
        let mut indices: Vec<u32> = vec![];

        let mut triangle_start: u32 = 0;
        let mut add_vertices = |vs: &[Vertex], x: usize, y: usize, z: usize| {
            vertices.append(
                &mut vs
                    .iter()
                    .map(|v| {
                        vertex!(
                                    position: v.position + vector3!(x as f32, y as f32, z as f32),
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
                    if !self.blocks[x][y][z] {
                        continue;
                    }

                    // Front faces
                    if z == 0 || !self.blocks[x][y][z - 1] {
                        add_vertices(&mut face_vertices[0].clone(), x, y, z);
                    }
                    // Back faces
                    if z == CHUNK_SIZE - 1 || !self.blocks[x][y][z + 1] {
                        add_vertices(&mut face_vertices[3].clone(), x, y, z);
                    }

                    // Left faces
                    if x == 0 || !self.blocks[x - 1][y][z] {
                        add_vertices(&mut face_vertices[2].clone(), x, y, z);
                    }
                    // Right faces
                    if x == CHUNK_SIZE - 1 || !self.blocks[x + 1][y][z] {
                        add_vertices(&mut face_vertices[1].clone(), x, y, z);
                    }

                    // Bottom faces
                    if y == 0 || !self.blocks[x][y - 1][z] {
                        add_vertices(&mut face_vertices[5].clone(), x, y, z);
                    }
                    // Top faces
                    if y == CHUNK_SIZE - 1 || !self.blocks[x][y + 1][z] {
                        add_vertices(&mut face_vertices[4].clone(), x, y, z);
                    }
                }
            }
        }
        Mesh::new(vertices, indices)
    }
}

/// `WORLD_SIZE` determines the size of the world in chunks.
const WORLD_SIZE: usize = 4;
pub struct World {
    chunks: Vec<Vec<Vec<Chunk>>>,
}

impl World {
    pub fn new() -> World {
        let mut chunks = vec![];
        for chunk_x in 0..WORLD_SIZE {
            chunks.push(vec![]);
            for chunk_y in 0..WORLD_SIZE {
                chunks[chunk_x].push(vec![]);
                for chunk_z in 0..WORLD_SIZE {
                    let mut blocks: ChunkBlocks = Vec::new();

                    for x in 0..CHUNK_SIZE {
                        blocks.push(Vec::new());
                        for y in 0..CHUNK_SIZE {
                            blocks[x].push(Vec::new());
                            for _ in 0..CHUNK_SIZE {
                                blocks[x][y].push(chunk_y * CHUNK_SIZE + y <= 2);
                            }
                        }
                    }
                    chunks[chunk_x][chunk_y].push(Chunk { blocks });
                }
            }
        }

        World { chunks }
    }

    pub fn generate_chunks(&self, renderer: &mut Renderer, world: &mut specs::World) {
        let mut thread_handles = vec![];
        for chunk_x in 0..WORLD_SIZE {
            for chunk_y in 0..WORLD_SIZE {
                for chunk_z in 0..WORLD_SIZE {
                    let chunk = self.chunks[chunk_x][chunk_y][chunk_z].clone();
                    thread_handles.push((
                        thread::spawn(move || -> Mesh { chunk.generate_mesh() }),
                        (chunk_x, chunk_y, chunk_z),
                    ));
                }
            }
        }

        for (handle, (chunk_x, chunk_y, chunk_z)) in thread_handles {
            let chunk_mesh = handle.join().unwrap();
            renderer.register_mesh(&chunk_mesh).unwrap();

            world
                .create_entity()
                .with(Transform::new(
                    vector3!(
                        (chunk_x * CHUNK_SIZE) as f32,
                        (chunk_y * CHUNK_SIZE) as f32,
                        (chunk_z * CHUNK_SIZE) as f32
                    ),
                    vector3!(1.0, 1.0, 1.0),
                    Quaternion::new(1.0, 0.0, 0.0, 0.0),
                ))
                .with(RenderMesh::new(&chunk_mesh))
                .build();
        }
    }
}
