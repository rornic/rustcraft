use cgmath::{Deg, Euler, Quaternion, Vector3};
use glium::glutin::event::VirtualKeyCode;
use specs::{Component, Join, Read, ReadStorage, System, VecStorage, WriteStorage};

use crate::{input::Input, render::camera::Camera};

use super::{physics::Rigidbody, Transform};

#[derive(Default)]
pub struct Player {}

impl Component for Player {
    type Storage = VecStorage<Player>;
}

#[derive(Default)]
pub struct PlayerMovement {}

impl<'a> System<'a> for PlayerMovement {
    type SystemData = (
        ReadStorage<'a, Transform>,
        ReadStorage<'a, Player>,
        ReadStorage<'a, Camera>,
        WriteStorage<'a, Rigidbody>,
        Read<'a, Input>,
    );

    fn run(&mut self, (transforms, players, cameras, mut rigidbodies, input): Self::SystemData) {
        for (transform, player, camera, rigidbody) in
            (&transforms, &players, &cameras, &mut rigidbodies).join()
        {
            let move_speed = 5.0;

            let mut movement_vector = Vector3::new(0.0, 0.0, 0.0);
            if input.keyboard.is_pressed(VirtualKeyCode::A) {
                movement_vector.x = -move_speed;
            } else if input.keyboard.is_pressed(VirtualKeyCode::D) {
                movement_vector.x = move_speed;
            }

            if input.keyboard.is_pressed(VirtualKeyCode::W) {
                movement_vector.z = move_speed;
            } else if input.keyboard.is_pressed(VirtualKeyCode::S) {
                movement_vector.z = -move_speed;
            }

            movement_vector = Quaternion::from(Euler {
                x: Deg(0.0),
                y: camera.yaw(),
                z: Deg(0.0),
            }) * movement_vector;
            rigidbody.velocity.x = movement_vector.x;
            rigidbody.velocity.z = movement_vector.z;

            if input.keyboard.is_pressed(VirtualKeyCode::Space) && rigidbody.is_grounded() {
                rigidbody.velocity.y = 4.0;
            }
        }
    }
}
