/// Represents a 3D position or direction in the world.
pub struct Vector3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
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
