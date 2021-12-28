use specs::{Builder, WorldExt};

use crate::render::mesh::{Mesh, Vertex};
use crate::render::{RenderMesh, Renderer};
use crate::{vector3, vertex};

use self::components::Transform;

pub mod components;
pub mod systems;

/// Each chunk is a cube of blocks. `CHUNK_SIZE` determines the size of this cube in blocks.
const CHUNK_SIZE: usize = 16;

type ChunkBlocks = Vec<Vec<Vec<bool>>>;
#[derive(Clone)]
struct Chunk {
    blocks: ChunkBlocks,
}

impl Chunk {
    fn new(blocks: ChunkBlocks) -> Chunk {
        Chunk { blocks }
    }

    /// Generates a single mesh for this chunk
    fn generate_mesh(&self) -> Mesh {
        let mut vertices: Vec<Vertex> = vec![];
        let mut indices: Vec<u32> = vec![];

        let mut block = 0;

        for x in 0..self.blocks.len() {
            for y in 0..self.blocks[x].len() {
                for z in 0..self.blocks[x][y].len() {
                    if self.blocks[x][y][z] {
                        let cube = super::render::mesh::primitives::cube();
                        vertices.append(
                            &mut cube
                                .vertices
                                .into_iter()
                                .map(|v| {
                                    vertex!(
                                    position: v.position + vector3!(x as f32, y as f32, z as f32),
                                    normal: v.normal,
                                    uv: v.uv)
                                })
                                .collect(),
                        );
                        indices.append(
                            &mut cube.indices.into_iter().map(|i| i + (block * 24)).collect(),
                        );
                        block += 1;
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
        for chunk_x in 0..WORLD_SIZE {
            for chunk_y in 0..WORLD_SIZE {
                for chunk_z in 0..WORLD_SIZE {
                    let chunk_mesh = self.chunks[chunk_x][chunk_y][chunk_z].generate_mesh();
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
                        ))
                        .with(RenderMesh::new(&chunk_mesh))
                        .build();
                }
            }
        }
    }
}
