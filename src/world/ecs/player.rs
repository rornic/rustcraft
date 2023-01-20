use cgmath::{Deg, Euler, Quaternion, Vector3};
use glium::glutin::event::VirtualKeyCode;
use specs::{Component, Join, Read, ReadStorage, System, VecStorage, Write, WriteStorage};

use crate::{
    input::Input,
    render::camera::Camera,
    world::{BlockType, World},
};

use super::{
    physics::{self, Rigidbody},
    Transform,
};

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
        Read<'a, World>,
        Read<'a, Input>,
    );

    fn run(
        &mut self,
        (transforms, players, cameras, mut rigidbodies, world, input): Self::SystemData,
    ) {
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

#[derive(Default)]
pub struct PlayerBlockBreak {}

impl<'a> System<'a> for PlayerBlockBreak {
    type SystemData = (
        ReadStorage<'a, Transform>,
        ReadStorage<'a, Camera>,
        Write<'a, World>,
        Read<'a, Input>,
    );

    fn run(&mut self, (transforms, cameras, mut world, input): Self::SystemData) {
        for (transform, camera) in (&transforms, &cameras).join() {
            if input.mouse.is_left_pressed() {
                if let Some(hit) = physics::raycast::raycast(
                    &world,
                    transform.position,
                    camera.look_direction(),
                    5.0,
                ) {
                    world.set_block_at(hit.position, BlockType::Air);
                }
            }
        }
    }
}
