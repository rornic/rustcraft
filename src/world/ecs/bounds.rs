use cgmath::Vector3;
use specs::{Component, VecStorage};

use crate::vector3;

#[derive(Clone)]
pub struct Bounds {
    origin: Vector3<f32>,
    dimensions: Vector3<f32>,
}

impl Bounds {
    pub fn new(origin: Vector3<f32>, dimensions: Vector3<f32>) -> Bounds {
        Bounds { origin, dimensions }
    }

    pub fn vertices(&self) -> Vec<Vector3<f32>> {
        let mut vertices = vec![];
        for i in [-1.0, 1.0] {
            for j in [-1.0, 1.0] {
                for k in [-1.0, 1.0] {
                    vertices.push(
                        self.origin
                            + vector3!(
                                i * (self.dimensions.x / 2.0),
                                j * (self.dimensions.y / 2.0),
                                k * (self.dimensions.z / 2.0)
                            ),
                    )
                }
            }
        }
        vertices
    }

    pub fn to_world(&self, position: Vector3<f32>) -> Bounds {
        Bounds {
            origin: self.origin + position,
            dimensions: self.dimensions,
        }
    }
}

impl Component for Bounds {
    type Storage = VecStorage<Self>;
}
