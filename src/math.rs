use crate::vector3;
use std::ops::Add;

/// Represents a 3D position or direction in the world.
#[derive(Default, Copy, Clone, PartialEq)]
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
        crate::math::Vector3 {
            x: $x,
            y: $y,
            z: $z,
        }
    };
}

/// Represents a 2D position or direction in the world.
#[derive(Default, Copy, Clone, PartialEq)]
pub struct Vector2 {
    pub x: f32,
    pub y: f32,
}

#[macro_export]
macro_rules! vector2 {
    ( $x:expr,$y:expr ) => {
        crate::math::Vector2 { x: $x, y: $y }
    };
}
