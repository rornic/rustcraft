use cgmath::Vector3;
use specs::{Component, VecStorage};

use crate::vector3;

#[derive(Clone)]
pub struct Bounds {
    pub origin: Vector3<f32>,
    pub dimensions: Vector3<f32>,
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

    pub fn top(&self) -> Vector3<f32> {
        self.origin + vector3!(0.0, self.dimensions.y / 2.0, 0.0)
    }

    pub fn bottom(&self) -> Vector3<f32> {
        self.origin - vector3!(0.0, self.dimensions.y / 2.0, 0.0)
    }

    pub fn left(&self) -> Vector3<f32> {
        self.origin - vector3!(self.dimensions.x / 2.0, 0.0, 0.0)
    }

    pub fn right(&self) -> Vector3<f32> {
        self.origin + vector3!(self.dimensions.x / 2.0, 0.0, 0.0)
    }

    pub fn front(&self) -> Vector3<f32> {
        self.origin + vector3!(0.0, 0.0, self.dimensions.z / 2.0)
    }

    pub fn back(&self) -> Vector3<f32> {
        self.origin - vector3!(0.0, 0.0, self.dimensions.z / 2.0)
    }

    fn corners(&self) -> [Vector3<f32>; 8] {
        [
            self.origin
                + vector3!(
                    self.dimensions.x / 2.0,
                    self.dimensions.y / 2.0,
                    self.dimensions.z / 2.0
                ),
            self.origin
                + vector3!(
                    -self.dimensions.x / 2.0,
                    self.dimensions.y / 2.0,
                    self.dimensions.z / 2.0
                ),
            self.origin
                + vector3!(
                    self.dimensions.x / 2.0,
                    self.dimensions.y / 2.0,
                    -self.dimensions.z / 2.0
                ),
            self.origin
                + vector3!(
                    -self.dimensions.x / 2.0,
                    self.dimensions.y / 2.0,
                    -self.dimensions.z / 2.0
                ),
            self.origin
                + vector3!(
                    self.dimensions.x / 2.0,
                    -self.dimensions.y / 2.0,
                    self.dimensions.z / 2.0
                ),
            self.origin
                + vector3!(
                    -self.dimensions.x / 2.0,
                    -self.dimensions.y / 2.0,
                    self.dimensions.z / 2.0
                ),
            self.origin
                + vector3!(
                    self.dimensions.x / 2.0,
                    -self.dimensions.y / 2.0,
                    -self.dimensions.z / 2.0
                ),
            self.origin
                + vector3!(
                    -self.dimensions.x / 2.0,
                    -self.dimensions.y / 2.0,
                    -self.dimensions.z / 2.0
                ),
        ]
    }

    pub fn intersects(&self, other: Bounds) -> bool {
        self.corners().iter().any(|s| other.contains(*s))
    }

    pub fn contains(&self, other: Vector3<f32>) -> bool {
        (other.x >= self.left().x && other.x <= self.right().x)
            && (other.y >= self.bottom().y && other.y <= self.top().y)
            && (other.z >= self.back().z && other.z <= self.front().z)
    }
}

impl Component for Bounds {
    type Storage = VecStorage<Self>;
}

#[cfg(test)]
mod tests {
    use crate::vector3;

    use super::Bounds;

    #[test]
    fn test_bounds_intersects() {
        let bounds = Bounds {
            origin: vector3!(30.854647, 4.8238087, 31.315569),
            dimensions: vector3!(0.5, 2.0, 0.5),
        };

        let other = Bounds {
            origin: vector3!(30.5, 4.5, 31.5),
            dimensions: vector3!(1.0, 1.0, 1.0),
        };
        assert!(bounds.intersects(other));
    }

    #[test]
    fn test_bounds_contains() {
        let bounds = Bounds {
            origin: vector3!(0.0, 0.0, 0.0),
            dimensions: vector3!(1.0, 1.0, 1.0),
        };

        assert!(bounds.contains(vector3!(0.2, 0.2, 0.2)));
        assert!(bounds.contains(vector3!(0.1, 0.2, 0.3)));
        assert!(bounds.contains(vector3!(0.3, 0.2, 0.1)));
        assert!(bounds.contains(vector3!(-0.4, -0.3, 0.45)));
    }

    #[test]
    fn test_bounds_contains_non_zero_origin() {
        let bounds = Bounds {
            origin: vector3!(5.0, 4.0, 3.0),
            dimensions: vector3!(1.0, 1.0, 1.0),
        };

        assert!(bounds.contains(vector3!(5.4, 3.6, 3.0)));
        assert!(!bounds.contains(vector3!(0.0, -0.3, 8.9)));
    }
}
