use cgmath::{num_traits::Signed, InnerSpace, Vector3};

use crate::world::ecs::bounds::Bounds;

#[derive(Debug)]
pub struct ViewFrustum {
    planes: [Plane; 6],
}

impl ViewFrustum {
    pub fn new(
        pos: Vector3<f32>,
        dir: Vector3<f32>,
        up: Vector3<f32>,
        fov: f32,
        near: f32,
        far: f32,
        aspect_ratio: f32,
    ) -> Self {
        let h_near = (fov / 2.0).tan() * near;
        let w_near = h_near * aspect_ratio;

        let z = -dir;
        let x = (up.cross(z)).normalize();
        let y = z.cross(x);

        let (nc, fc) = (pos - z * near, pos - z * far);

        Self {
            planes: [
                Plane {
                    point: nc,
                    normal: -z,
                },
                Plane {
                    point: fc,
                    normal: z,
                },
                Plane {
                    point: nc + y * h_near,
                    normal: ((nc + y * h_near) - pos).normalize().cross(x),
                },
                Plane {
                    point: nc - y * h_near,
                    normal: x.cross(((nc - y * h_near) - pos).normalize()),
                },
                Plane {
                    point: nc - x * w_near,
                    normal: ((nc - x * w_near) - pos).normalize().cross(y),
                },
                Plane {
                    point: nc + x * w_near,
                    normal: y.cross(((nc + x * w_near) - pos).normalize()),
                },
            ],
        }
    }

    pub fn contains_box(&self, bounds: Bounds) -> bool {
        let contains = true;
        for p in self.planes.iter() {
            let (mut v_in, mut v_out) = (0_u32, 0_u32);

            let vs = bounds.vertices();
            for v in &vs {
                if p.distance(*v).is_negative() {
                    v_out += 1;
                } else {
                    v_in += 1;
                }

                if v_out > 0 && v_in > 0 {
                    break;
                }
            }

            if v_in == 0 {
                return false;
            }
        }

        contains
    }
}

#[derive(Debug)]
struct Plane {
    point: Vector3<f32>,
    normal: Vector3<f32>,
}

impl Plane {
    fn distance(&self, pos: Vector3<f32>) -> f32 {
        (pos - self.point).dot(self.normal)
    }
}

#[cfg(test)]
mod tests {
    use crate::vector3;

    use super::Plane;

    #[test]
    fn test_plane_distance() {
        let pos = vector3!(5.0, 5.0, 0.0);

        let plane = Plane {
            point: vector3!(0.0, 0.0, 0.0),
            normal: vector3!(1.0, 0.0, 0.0),
        };

        assert_eq!(plane.distance(pos), 5.0);
    }

    #[test]
    fn test_plane_negative_distance() {
        let pos = vector3!(-5.0, 5.0, 10.0);

        let plane = Plane {
            point: vector3!(0.0, 0.0, 0.0),
            normal: vector3!(1.0, 0.0, 0.0),
        };

        assert_eq!(plane.distance(pos), -5.0);
    }
}
