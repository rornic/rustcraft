use std::ops::Add;

use cgmath::{prelude::*, Deg, Quaternion, Rad, Vector3};
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
                Quaternion::one(),
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

        view_matrix.0 = ViewMatrix::new(
            transform.position,
            transform.rotation * vector3!(0.0, 0.0, 1.0),
            transform.rotation * vector3!(0.0, 1.0, 0.0),
        )
        .0;

        // Rotate camera
        if input.keyboard.is_pressed(VirtualKeyCode::E) {
            transform.rotation =
                transform.rotation * Quaternion::from_angle_y(Deg(90.0 * delta_time));
        } else if input.keyboard.is_pressed(VirtualKeyCode::Q) {
            transform.rotation =
                transform.rotation * Quaternion::from_angle_y(Deg(-90.0 * delta_time));
        }

        let mut movement_vector = Vector3::new(0.0, 0.0, 0.0);
        if input.keyboard.is_pressed(VirtualKeyCode::A) {
            movement_vector.x = -3.0;
        } else if input.keyboard.is_pressed(VirtualKeyCode::D) {
            movement_vector.x = 3.0;
        }

        if input.keyboard.is_pressed(VirtualKeyCode::Space) {
            movement_vector.y = 3.0;
        } else if input.keyboard.is_pressed(VirtualKeyCode::LShift) {
            movement_vector.y = -3.0;
        }

        if input.keyboard.is_pressed(VirtualKeyCode::W) {
            movement_vector.z = 3.0;
        } else if input.keyboard.is_pressed(VirtualKeyCode::S) {
            movement_vector.z = -3.0;
        }

        transform.position += transform.rotation * movement_vector * delta_time;
    }
}
