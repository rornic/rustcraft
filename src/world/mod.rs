use std::ops::Add;

use crate::render::mesh::MeshData;
use crate::vector3;

/// Represents a 3D position or direction in the world.
#[derive(Copy, Clone, PartialEq)]
pub struct Vector3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Add for Vector3 {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        vector3!(self.x + rhs.x, self.y + rhs.y, self.z + rhs.z)
    }
}

#[macro_export]
macro_rules! vector3 {
    ( $x:expr,$y:expr,$z:expr ) => {
        crate::world::Vector3 {
            x: $x,
            y: $y,
            z: $z,
        }
    };
}

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
    pub fn generate_chunk_mesh(&self) -> MeshData {
        let mut vertices: Vec<Vector3> = vec![];
        let mut normals: Vec<Vector3> = vec![];
        let mut indices: Vec<u32> = vec![];

        let mut block = 0;

        for x in 0..self.blocks.len() {
            for y in 0..self.blocks[x].len() {
                for z in 0..self.blocks[x][y].len() {
                    if self.blocks[x][y][z] {
                        let mut cube = super::render::mesh::primitives::cube();
                        vertices.append(
                            &mut cube
                                .vertices
                                .into_iter()
                                .map(|v| v + vector3!(x as f32, y as f32, z as f32))
                                .collect(),
                        );
                        normals.append(&mut cube.normals);
                        indices.append(
                            &mut cube.indices.into_iter().map(|i| i + (block * 24)).collect(),
                        );
                        block += 1;
                    }
                }
            }
        }

        MeshData::new(vertices, normals, indices)
    }
}

pub struct Transform {
    position: Vector3,
    scale: Vector3,
}

impl Transform {
    /// Calculates a model matrix for rendering
    pub fn matrix(&self) -> [[f32; 4]; 4] {
        [
            [self.scale.x, 0.0, 0.0, 0.0],
            [0.0, self.scale.y, 0.0, 0.0],
            [0.0, 0.0, self.scale.z, 0.0],
            [self.position.x, self.position.y, self.position.z, 1.0],
        ]
    }
}
