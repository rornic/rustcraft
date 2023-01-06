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
            let bottom = bounds.to_world(transform.position).bottom() + vector3!(0.0, -0.01, 0.0);
            rigidbody.grounded = world.block_at(bottom).is_solid();
            if rigidbody.velocity.y < 0.0 && rigidbody.grounded {
                rigidbody.velocity.y = 0.0;
            }

            if !rigidbody.grounded && rigidbody.apply_gravity {
                rigidbody.velocity += vector3!(0.0, GRAVITY, 0.0f32) * delta_time.0;
            }

            let new_position = transform.position + rigidbody.velocity * delta_time.0;
            let x_block = world.block_centre(transform.position)
                + vector3!(rigidbody.velocity.x.signum(), 0.0, 0.0);
            if collides_with_block(x_block, bounds.to_world(new_position), &world) {
                rigidbody.velocity.x = 0.0;
            }

            let z_block = world.block_centre(transform.position)
                + vector3!(0.0, 0.0, rigidbody.velocity.z.signum());
            if collides_with_block(z_block, bounds.to_world(new_position), &world) {
                rigidbody.velocity.z = 0.0;
            }

            transform.position += rigidbody.velocity * delta_time.0;
        }
    }
}

fn collides_with_block(block: Vector3<f32>, bounds: Bounds, world: &World) -> bool {
    world.block_at(block).is_solid()
        && bounds.intersects(Bounds::new(block, vector3!(1.0, 1.0, 1.0)))
}
