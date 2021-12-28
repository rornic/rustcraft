use specs::{Component, VecStorage};

use crate::math::Vector3;

#[derive(Default)]
pub struct Transform {
    pub position: Vector3,
    pub scale: Vector3,
}

impl Transform {
    pub fn new(position: Vector3, scale: Vector3) -> Transform {
        Transform {
            position: position,
            scale: scale,
        }
    }
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

impl Component for Transform {
    type Storage = VecStorage<Self>;
}
