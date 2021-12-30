use crate::vector3;
use std::ops::{Add, Sub};

/// Represents a 3D position or direction in the world.
// #[derive(Default, Copy, Clone, PartialEq)]
// pub struct Vector3 {
//     pub x: f32,
//     pub y: f32,
//     pub z: f32,
// }

// impl Vector3 {
//     pub fn normalize(&self) -> Vector3 {
//         let len = self.len();
//         vector3!(self.x / len, self.y / len, self.z / len)
//     }

//     pub fn len(&self) -> f32 {
//         (self.x * self.x + self.y * self.y + self.z * self.z).sqrt()
//     }
// }

// impl Add for Vector3 {
//     type Output = Self;

//     fn add(self, rhs: Self) -> Self::Output {
//         vector3!(self.x + rhs.x, self.y + rhs.y, self.z + rhs.z)
//     }
// }

// impl Sub for Vector3 {
//     type Output = Self;

//     fn sub(self, rhs: Self) -> Self::Output {
//         vector3!(self.x - rhs.x, self.y - rhs.y, self.z - rhs.z)
//     }
// }

#[macro_export]
macro_rules! vector3 {
    ( $x:expr,$y:expr,$z:expr ) => {
        cgmath::Vector3 {
            x: $x,
            y: $y,
            z: $z,
        }
    };
}

// /// Represents a 2D position or direction in the world.
// #[derive(Default, Copy, Clone, PartialEq)]
// pub struct Vector2 {
//     pub x: f32,
//     pub y: f32,
// }

#[macro_export]
macro_rules! vector2 {
    ( $x:expr,$y:expr ) => {
        cgmath::Vector2 { x: $x, y: $y }
    };
}
