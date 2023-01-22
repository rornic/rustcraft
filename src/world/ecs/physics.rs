use cgmath::{Vector3, Zero};
use specs::{Component, Join, Read, ReadStorage, System, VecStorage, WriteStorage};

use crate::{vector3, world::World, DeltaTime};

use super::{bounds::Bounds, Transform};

const GRAVITY: f32 = -9.8;

pub struct Rigidbody {
    pub velocity: Vector3<f32>,
    apply_gravity: bool,
    grounded: bool,
}

impl Component for Rigidbody {
    type Storage = VecStorage<Self>;
}

impl Default for Rigidbody {
    fn default() -> Self {
        Self {
            velocity: Vector3::zero(),
            apply_gravity: true,
            grounded: false,
        }
    }
}

impl Rigidbody {
    pub fn is_grounded(&self) -> bool {
        self.grounded
    }
}

pub struct Physics {}

impl Physics {
    pub fn new() -> Self {
        Self {}
    }
}

impl<'a> System<'a> for Physics {
    type SystemData = (
        WriteStorage<'a, Transform>,
        WriteStorage<'a, Rigidbody>,
        ReadStorage<'a, Bounds>,
        Read<'a, DeltaTime>,
        Read<'a, World>,
    );

    fn run(
        &mut self,
        (mut transforms, mut rigidbodies, bounds, delta_time, world): Self::SystemData,
    ) {
        for (transform, rigidbody, bounds) in (&mut transforms, &mut rigidbodies, &bounds).join() {
            let bottom_block = world.block_centre(bounds.to_world(transform.position).bottom());

            let new_position = transform.position
                + vector3!(rigidbody.velocity.x, 0.0, rigidbody.velocity.z) * delta_time.0;
            if collides_with_block(
                bottom_block + vector3!(rigidbody.velocity.x.signum(), 0.0, 0.0),
                bounds.to_world(new_position),
                &world,
            ) {
                rigidbody.velocity.x = 0.0;
            }

            if collides_with_block(
                bottom_block + vector3!(0.0, 0.0, rigidbody.velocity.z.signum()),
                bounds.to_world(new_position),
                &world,
            ) {
                rigidbody.velocity.z = 0.0;
            }

            if !rigidbody.grounded && rigidbody.apply_gravity {
                rigidbody.velocity += vector3!(0.0, GRAVITY, 0.0f32) * delta_time.0;
            }

            rigidbody.grounded = collides_with_block(
                bottom_block + vector3!(0.0, -0.1, 0.0),
                bounds.to_world(new_position),
                &world,
            );
            if rigidbody.velocity.y < 0.0 && rigidbody.grounded {
                rigidbody.velocity.y = 0.0;
            }

            transform.position += rigidbody.velocity * delta_time.0;
        }
    }
}

fn collides_with_block(block: Vector3<f32>, bounds: Bounds, world: &World) -> bool {
    world.block_at(block).is_solid()
        && bounds.intersects(Bounds::new(block, vector3!(1.0, 1.0, 1.0)))
}

pub mod raycast {
    use cgmath::{InnerSpace, Vector3};

    use crate::{
        vector3,
        world::{BlockType, World},
    };

    const RAYCAST_STEP: f32 = 0.5;

    pub struct RaycastHit {
        pub block: BlockType,
        pub position: Vector3<f32>,
    }

    pub fn raycast(
        world: &World,
        origin: Vector3<f32>,
        dir: Vector3<f32>,
        max_dist: f32,
    ) -> Option<RaycastHit> {
        let dir = dir.normalize();
        for i in 0..(max_dist / RAYCAST_STEP).ceil() as usize {
            let pos = origin + i as f32 * RAYCAST_STEP * dir;

            let block = world.block_at(pos);
            if block.is_solid() {
                return Some(RaycastHit {
                    block: block,
                    position: pos,
                });
            }
        }
        None
    }

    pub fn block_aligned_raycast(
        world: &World,
        origin: Vector3<f32>,
        dir: Vector3<f32>,
        max_blocks: f32,
    ) -> Option<RaycastHit> {
        let dir = dir.normalize();
        let (step_x, step_y, step_z) = (dir.x.signum(), dir.y.signum(), dir.z.signum());

        let (t_delta_x, t_delta_y, t_delta_z) =
            (1.0 / dir.x.abs(), 1.0 / dir.y.abs(), 1.0 / dir.z.abs());

        let start_block = world.block_centre(origin);
        let (x_out, y_out, z_out) = (
            start_block.x + step_x * max_blocks,
            start_block.y + step_y * max_blocks,
            start_block.z + step_z * max_blocks,
        );
        let (mut t_max_x, mut t_max_y, mut t_max_z) = (
            (start_block.x - origin.x) / dir.x,
            (start_block.y - origin.y) / dir.y,
            (start_block.z - origin.z) / dir.z,
        );
        let (mut x, mut y, mut z) = (start_block.x, start_block.y, start_block.z);
        loop {
            if t_max_x < t_max_y && t_max_x < t_max_z {
                x += step_x;
                if x == x_out {
                    return None;
                }
                t_max_x += t_delta_x;
            } else if t_max_y < t_max_z {
                y += step_y;
                if y == y_out {
                    return None;
                }
                t_max_y += t_delta_y;
            } else {
                z += step_z;
                if z == z_out {
                    return None;
                }
                t_max_z += t_delta_z;
            }

            let pos = vector3!(x, y, z);
            let block = world.block_at(pos);
            if block.is_solid() {
                return Some(RaycastHit {
                    block: block,
                    position: pos,
                });
            }
        }
    }
}
