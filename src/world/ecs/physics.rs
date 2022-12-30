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
            let grounded = world.block_at(bottom).is_solid();
            if grounded != rigidbody.grounded {
                rigidbody.velocity.y = 0.0;
            }
            rigidbody.grounded = grounded;

            if !rigidbody.grounded && rigidbody.apply_gravity {
                rigidbody.velocity += vector3!(0.0, GRAVITY, 0.0f32) * delta_time.0;
            }

            transform.position += rigidbody.velocity * delta_time.0;
        }
    }
}
