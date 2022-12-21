use cgmath::{prelude::*, Deg, Euler, Quaternion, Vector3};
use glium::glutin::event::VirtualKeyCode;
use specs::{Component, Entity, Read, System, VecStorage, Write, WriteStorage};

use crate::{input::Input, vector3, DeltaTime, ViewMatrix};

use super::Transform;

/// Runs on a single `Entity` designated as the camera. This entity must have a `Transform` component otherwise the system will fail.
pub struct CameraSystem {
    camera: Entity,
}

impl CameraSystem {
    /// Creates a new `CameraSystem`, inserting a default `Camera` entity into the world.
    pub fn new(camera: Entity) -> CameraSystem {
        CameraSystem { camera }
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
            .get_mut(self.camera)
            .expect("No transform found on camera entity");

        let camera = cameras
            .get_mut(self.camera)
            .expect("No camera found on camera entity");

        // Apply yaw first, then pitch (YXZ)
        transform.rotation = Quaternion::from(Euler::new(Deg(0.0), camera.yaw, Deg(0.0)))
            * Quaternion::from(Euler::new(camera.pitch, Deg(0.0), Deg(0.0)));

        view_matrix.0 = camera
            .view_matrix(
                transform.position,
                transform.rotation * vector3!(0.0, 0.0, 1.0),
                vector3!(0.0, 1.0, 0.0),
            )
            .0;

        // Apply changes to camera's pitch and yaw based on mouse movement
        camera.pitch.0 = (camera.pitch.0 + input.mouse.vertical_motion() * 30.0 * delta_time)
            .clamp(-camera.max_pitch.0, camera.max_pitch.0);
        camera.yaw.0 += input.mouse.horizontal_motion() * 30.0 * delta_time;

        let move_speed = 10.0;

        // Move camera in XYZ space
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

        if input.keyboard.is_pressed(VirtualKeyCode::Space) {
            movement_vector.y = move_speed;
        } else if input.keyboard.is_pressed(VirtualKeyCode::LShift) {
            movement_vector.y = -move_speed;
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
    max_pitch: Deg<f32>,
    pub aspect_ratio: f32,
    pub near_dist: f32,
    pub far_dist: f32,
    pub fov: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            yaw: Deg(0.0),
            pitch: Deg(0.0),
            max_pitch: Deg(65.0),
            aspect_ratio: 9.0 / 16.0,
            near_dist: 0.1,
            far_dist: 1024.0,
            fov: 3.141592 / 2.0,
        }
    }
}

impl Component for Camera {
    type Storage = VecStorage<Self>;
}

impl Camera {
    pub fn projection_matrix(&self) -> [[f32; 4]; 4] {
        let f = 1.0 / (self.fov / 2.0).tan();

        [
            [f * (1.0 / self.aspect_ratio), 0.0, 0.0, 0.0],
            [0.0, f, 0.0, 0.0],
            [
                0.0,
                0.0,
                (self.far_dist + self.near_dist) / (self.far_dist - self.near_dist),
                1.0,
            ],
            [
                0.0,
                0.0,
                -(2.0 * self.far_dist * self.near_dist) / (self.far_dist - self.near_dist),
                0.0,
            ],
        ]
    }

    pub fn view_matrix(
        &self,
        position: Vector3<f32>,
        direction: Vector3<f32>,
        up: Vector3<f32>,
    ) -> ViewMatrix {
        let direction = direction.normalize();
        let s = vector3!(
            up.y * direction.z - up.z * direction.y,
            up.z * direction.x - up.x * direction.z,
            up.x * direction.y - up.y * direction.x
        )
        .normalize();
        let u = vector3!(
            direction.y * s.z - direction.z * s.y,
            direction.z * s.x - direction.x * s.z,
            direction.x * s.y - direction.y * s.x
        );

        let p = vector3!(
            -position.x * s.x - position.y * s.y - position.z * s.z,
            -position.x * u.x - position.y * u.y - position.z * u.z,
            -position.x * direction.x - position.y * direction.y - position.z * direction.z
        );

        ViewMatrix([
            [s.x, u.x, direction.x, 0.0],
            [s.y, u.y, direction.y, 0.0],
            [s.z, u.z, direction.z, 0.0],
            [p.x, p.y, p.z, 1.0],
        ])
    }
}
