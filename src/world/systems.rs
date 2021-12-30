use std::ops::Add;

use cgmath::{prelude::*, Deg, Euler, Quaternion, Rad, Vector3};
use glium::glutin::event::VirtualKeyCode;
use specs::{
    Builder, Component, Entity, Read, System, VecStorage, World, WorldExt, Write, WriteStorage,
};

use crate::{input::Input, render::ViewMatrix, vector3, DeltaTime};

use super::components::Transform;

/// Runs on a single `Entity` designated as the camera. This entity must have a `Transform` component otherwise the system will fail.
pub struct CameraSystem {
    camera_entity: Entity,
}

impl CameraSystem {
    /// Creates a new `CameraSystem`, inserting a default `Camera` entity into the world.
    pub fn new(world: &mut World) -> CameraSystem {
        let camera_entity = world
            .create_entity()
            .with(Transform::new(
                vector3!(0.0, 0.0, 25.0),
                vector3!(1.0, 1.0, 1.0),
                Quaternion::one(),
            ))
            .with(Camera::default())
            .build();

        CameraSystem { camera_entity }
    }
}

impl<'a> System<'a> for CameraSystem {
    type SystemData = (
        WriteStorage<'a, Transform>,
        WriteStorage<'a, Camera>,
        Read<'a, Input>,
        Read<'a, DeltaTime>,
        Write<'a, ViewMatrix>,
    );

    fn run(
        &mut self,
        (mut transforms, mut cameras, input, delta_time, mut view_matrix): Self::SystemData,
    ) {
        let delta_time = delta_time.0;

        let transform = transforms
            .get_mut(self.camera_entity)
            .expect("No transform found on camera entity");

        let camera = cameras
            .get_mut(self.camera_entity)
            .expect("No camera found on camera entity");

        transform.rotation = Quaternion::from(Euler::new(camera.pitch, camera.yaw, Deg(0.0)));

        view_matrix.0 = ViewMatrix::new(
            transform.position,
            transform.rotation * vector3!(0.0, 0.0, 1.0),
            vector3!(0.0, 1.0, 0.0),
        )
        .0;

        // Apply changes to camera's pitch and yaw based on mouse movement
        camera.pitch.0 += input.mouse.vertical_motion() * 30.0 * delta_time;
        camera.yaw.0 += input.mouse.horizontal_motion() * 30.0 * delta_time;

        // Move camera in XYZ space
        let mut movement_vector = Vector3::new(0.0, 0.0, 0.0);
        if input.keyboard.is_pressed(VirtualKeyCode::A) {
            movement_vector.x = -3.0;
        } else if input.keyboard.is_pressed(VirtualKeyCode::D) {
            movement_vector.x = 3.0;
        }

        if input.keyboard.is_pressed(VirtualKeyCode::W) {
            movement_vector.z = 3.0;
        } else if input.keyboard.is_pressed(VirtualKeyCode::S) {
            movement_vector.z = -3.0;
        }

        if input.keyboard.is_pressed(VirtualKeyCode::Space) {
            movement_vector.y = 3.0;
        } else if input.keyboard.is_pressed(VirtualKeyCode::LShift) {
            movement_vector.y = -3.0;
        }

        // Apply Y axis movement separately so we don't rotate the movement to follow the camera
        transform.position += (transform.rotation
            * vector3!(movement_vector.x, 0.0, movement_vector.z)
            + vector3!(0.0, movement_vector.y, 0.0))
            * delta_time;
    }
}

pub struct Camera {
    yaw: Deg<f32>,
    pitch: Deg<f32>,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            yaw: Deg(0.0),
            pitch: Deg(0.0),
        }
    }
}

impl Component for Camera {
    type Storage = VecStorage<Self>;
}
