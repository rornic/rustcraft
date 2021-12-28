use crate::render::mesh::{Mesh, Vertex};
use crate::{vector3, vertex};

pub mod components;
pub mod systems;

// Simple representation of the world.
// TODO: just a placeholder, will need replacing.
pub struct World {
    pub blocks: [[[bool; 16]; 16]; 16],
}

impl World {
    pub fn new() -> World {
        let mut blocks: [[[bool; 16]; 16]; 16] = [[[false; 16]; 16]; 16];

        for x in 0..blocks.len() {
            for y in 0..blocks[x].len() {
                for z in 0..blocks[x][y].len() {
                    blocks[x][y][z] = true;
                }
            }
        }

        World { blocks }
    }

    /// Generates a single chunk mesh from the whole world
    pub fn generate_chunk_mesh(&self) -> Mesh {
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
