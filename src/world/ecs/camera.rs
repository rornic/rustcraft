use cgmath::{prelude::*, Deg, Euler, Quaternion, Vector3};
use glium::glutin::event::VirtualKeyCode;
use specs::{Component, Entity, Read, System, VecStorage, WriteStorage};

use crate::{
    input::Input, render::renderer::RENDER_DISTANCE, vector3, world::CHUNK_SIZE, DeltaTime,
};

use super::{bounds::Bounds, Transform};

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
    );

    fn run(&mut self, (mut transforms, mut cameras, input, delta_time): Self::SystemData) {
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

        camera.calculate_view_matrix(
            transform.position,
            transform.rotation * vector3!(0.0, 0.0, 1.0),
            vector3!(0.0, 1.0, 0.0),
        );
        camera.calculate_projection_matrix();

        let sensitivity = 90.0;

        camera.pitch.0 = (camera.pitch.0
            + input.mouse.vertical_motion() * sensitivity * delta_time)
            .clamp(-camera.max_pitch.0, camera.max_pitch.0);
        camera.yaw.0 += input.mouse.horizontal_motion() * sensitivity * delta_time;
    }
}

#[derive(Clone)]
pub struct Camera {
    yaw: Deg<f32>,
    pitch: Deg<f32>,
    max_pitch: Deg<f32>,
    pub aspect_ratio: f32,
    pub near_dist: f32,
    pub far_dist: f32,
    pub fov: f32,
    view_matrix: [[f32; 4]; 4],
    projection_matrix: [[f32; 4]; 4],
}

impl Default for Camera {
    fn default() -> Self {
        let mut cam = Self {
            yaw: Deg(0.0),
            pitch: Deg(0.0),
            max_pitch: Deg(65.0),
            aspect_ratio: 16.0 / 9.0,
            near_dist: 0.1,
            far_dist: RENDER_DISTANCE as f32 * CHUNK_SIZE as f32,
            fov: 3.141592 / 3.0,
            view_matrix: [[0.0; 4]; 4],
            projection_matrix: [[0.0; 4]; 4],
        };
        cam.calculate_projection_matrix();
        cam
    }
}

impl Component for Camera {
    type Storage = VecStorage<Self>;
}

impl Camera {
    pub fn projection_matrix(&self) -> [[f32; 4]; 4] {
        self.projection_matrix
    }

    pub fn view_matrix(&self) -> [[f32; 4]; 4] {
        self.view_matrix
    }

    fn calculate_projection_matrix(&mut self) {
        let f = 1.0 / (self.fov / 2.0).tan();
        self.projection_matrix = [
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

    pub fn calculate_view_matrix(
        &mut self,
        position: Vector3<f32>,
        direction: Vector3<f32>,
        up: Vector3<f32>,
    ) {
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

        self.view_matrix = [
            [s.x, u.x, direction.x, 0.0],
            [s.y, u.y, direction.y, 0.0],
            [s.z, u.z, direction.z, 0.0],
            [p.x, p.y, p.z, 1.0],
        ];
    }

    pub fn is_point_visible(&self, transform: &Transform, point: Vector3<f32>) -> bool {
        let v = point - transform.position;

        let pcz = v.dot(transform.rotation * vector3!(0.0, 0.0, 1.0));
        if pcz < self.near_dist || pcz > self.far_dist {
            return false;
        }

        let h = pcz * 2.0 * f32::tan(175.0_f32.to_radians() / 2.0);
        let pcy = v.dot(transform.rotation * vector3!(0.0, 1.0, 0.0));
        if -h / 2.0 > pcy || pcy > h / 2.0 {
            return false;
        }

        let pcx = v.dot(transform.rotation * vector3!(1.0, 0.0, 0.0));
        let w = h * self.aspect_ratio;
        if -w / 2.0 > pcx || pcx > w / 2.0 {
            return false;
        }

        true
    }

    pub fn are_bounds_visible(
        &self,
        transform: &Transform,
        position: Vector3<f32>,
        bounds: &Bounds,
    ) -> bool {
        bounds
            .to_world(position)
            .vertices()
            .iter()
            .any(|v| self.is_point_visible(transform, *v))
    }
}
