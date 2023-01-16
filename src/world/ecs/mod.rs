use specs::{Component, VecStorage};

use crate::vector3;
use cgmath::{One, Quaternion, Vector3};

pub mod bounds;
pub mod chunk_loader;
pub mod physics;
pub mod player;

/// Contains data about an entity's `Transform`. This includes its position, scale and rotation in the world.
#[derive(Clone)]
pub struct Transform {
    pub position: Vector3<f32>,
    pub scale: Vector3<f32>,
    pub rotation: Quaternion<f32>,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            position: vector3!(0.0, 0.0, 0.0),
            scale: vector3!(1.0, 1.0, 1.0),
            rotation: Quaternion::one(),
        }
    }
}

impl Transform {
    pub fn new(
        position: Vector3<f32>,
        scale: Vector3<f32>,
        rotation: Quaternion<f32>,
    ) -> Transform {
        Transform {
            position: position,
            scale: scale,
            rotation: rotation,
        }
    }

    pub fn matrix(&self) -> [[f32; 4]; 4] {
        [
            [self.scale.x, 0.0, 0.0, 0.0],
            [0.0, self.scale.y, 0.0, 0.0],
            [0.0, 0.0, self.scale.z, 0.0],
            [self.position.x, self.position.y, self.position.z, 1.0],
        ]
    }

    pub fn forward(&self) -> Vector3<f32> {
        self.rotation * vector3!(0.0, 0.0, 1.0)
    }
}

impl Component for Transform {
    type Storage = VecStorage<Self>;
}
