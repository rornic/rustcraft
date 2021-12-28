use glium::glutin::event::VirtualKeyCode;
use specs::{Builder, Entity, Read, System, World, WorldExt, Write, WriteStorage};

use crate::{input::Input, render::ViewMatrix, vector3, DeltaTime};

use super::components::Transform;

/// Runs on a single `Entity` designated as the camera. This entity must have a `Transform` component otherwise the system will fail.
pub struct CameraSystem {
    camera_entity: Entity,
}

impl CameraSystem {
    pub fn new(world: &mut World) -> CameraSystem {
        let camera_entity = world
            .create_entity()
            .with(Transform::new(
                vector3!(0.0, 0.0, 25.0),
                vector3!(1.0, 1.0, 1.0),
            ))
            .build();

        CameraSystem { camera_entity }
    }
}

impl<'a> System<'a> for CameraSystem {
    type SystemData = (
        WriteStorage<'a, Transform>,
        Read<'a, Input>,
        Read<'a, DeltaTime>,
        Write<'a, ViewMatrix>,
    );

    fn run(&mut self, (mut transforms, input, delta_time, mut view_matrix): Self::SystemData) {
        let delta_time = delta_time.0;

        let transform = transforms
            .get_mut(self.camera_entity)
            .expect("No transform found on camera entity");

        view_matrix.0 = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [
                -transform.position.x,
                -transform.position.y,
                -transform.position.z,
                1.0,
            ],
        ];
        if input.keyboard.is_pressed(VirtualKeyCode::A) {
            transform.position.x -= 3.0 * delta_time;
        } else if input.keyboard.is_pressed(VirtualKeyCode::D) {
            transform.position.x += 3.0 * delta_time;
        }

        if input.keyboard.is_pressed(VirtualKeyCode::Space) {
            transform.position.y += 3.0 * delta_time;
        } else if input.keyboard.is_pressed(VirtualKeyCode::LShift) {
            transform.position.y -= 3.0 * delta_time;
        }

        if input.keyboard.is_pressed(VirtualKeyCode::W) {
            transform.position.z += 3.0 * delta_time;
        } else if input.keyboard.is_pressed(VirtualKeyCode::S) {
            transform.position.z -= 3.0 * delta_time;
        }
    }
}
