#[macro_use]
extern crate glium;
use std::time::Instant;

use cgmath::{One, Quaternion};
use glium::glutin::event::Event;
use glium::glutin::event_loop::{ControlFlow, EventLoop};
use glium::Display;
use input::{Input, InputEvent};
use render::renderer::{RenderMesh, RenderingSystem};
use specs::WorldExt;

use specs::prelude::*;

mod input;
mod math;
mod render;
mod util;
mod world;

use world::ecs::bounds::Bounds;
use world::ecs::camera::{Camera, CameraSystem};
use world::ecs::chunk_loader::ChunkLoaderSystem;
use world::ecs::Transform;

use crate::render::renderer::RenderJob;

/// Prepares a `Display` and `EventLoop` for rendering and updating.
fn init_display() -> (EventLoop<()>, Display) {
    use glium::glutin;

    // Set up window
    let event_loop = glutin::event_loop::EventLoop::new();
    let wb = glutin::window::WindowBuilder::new();
    let cb = glutin::ContextBuilder::new()
        .with_depth_buffer(24)
        .with_vsync(true)
        .with_multisampling(8);
    let display = glium::Display::new(wb, cb, &event_loop).unwrap();
    (event_loop, display)
}

fn process_event(ev: Event<()>, control_flow: &mut ControlFlow) -> Option<InputEvent> {
    use glium::glutin;

    // Handle window events
    match ev {
        glutin::event::Event::WindowEvent { event, .. } => match event {
            glutin::event::WindowEvent::CloseRequested => {
                *control_flow = glutin::event_loop::ControlFlow::Exit;
                None
            }
            _ => None,
        },
        glutin::event::Event::DeviceEvent { event, .. } => match event {
            glutin::event::DeviceEvent::Key(ki) => Some(InputEvent::Keyboard(ki)),
            glutin::event::DeviceEvent::MouseMotion { delta: d } => {
                Some(InputEvent::MouseMotion { delta: d })
            }
            _ => None,
        },
        _ => None,
    }
}

fn main() {
    let (event_loop, display) = init_display();

    let mut renderer = crate::render::renderer::Renderer::new(display);

    let mut world = specs::World::new();
    world.register::<Transform>();
    world.register::<RenderMesh>();
    world.register::<Bounds>();
    world.register::<Camera>();

    world.insert(ViewMatrix::default());
    world.insert(DeltaTime(0.0));
    world.insert(ElapsedTime(0.0));
    world.insert(RenderJob::default());

    let camera = world
        .create_entity()
        .with(Transform::new(
            vector3!(0.0, 32.0, 25.0),
            vector3!(1.0, 1.0, 1.0),
            Quaternion::one(),
        ))
        .with(Camera::default())
        .build();

    let mut dispatcher = DispatcherBuilder::new()
        .with(CameraSystem::new(camera), "camera", &[])
        .with(ChunkLoaderSystem::new(), "chunk_loader", &[])
        .with(RenderingSystem, "rendering", &["chunk_loader"])
        .build();
    dispatcher.setup(&mut world);

    let mut last_frame = Instant::now();
    event_loop.run(move |ev, _, control_flow| {
        *control_flow = glium::glutin::event_loop::ControlFlow::WaitUntil(
            last_frame + std::time::Duration::from_nanos(16_666_667),
        );

        match ev {
            glium::glutin::event::Event::MainEventsCleared => {
                let frame_start = Instant::now();
                let delta_time = (frame_start - last_frame).as_secs_f32();
                last_frame = frame_start;

                world.write_resource::<DeltaTime>().0 = delta_time;
                world.write_resource::<ElapsedTime>().0 += delta_time;

                dispatcher.dispatch(&mut world);
                world.maintain();

                world.write_resource::<Input>().update();

                let now = Instant::now();
                renderer.render(
                    world.write_storage::<Camera>().get_mut(camera).unwrap(),
                    &world.read_resource::<RenderJob>(),
                    world.read_resource::<ViewMatrix>().0,
                );
                println!("render {}ms", (Instant::now() - now).as_millis());
            }
            ev => {
                if let Some(e) = process_event(ev, control_flow) {
                    world.write_resource::<Input>().process_event(&e);
                }
            }
        };
    });
}

#[derive(Default)]
pub struct DeltaTime(f32);

#[derive(Default)]
pub struct ElapsedTime(f32);

#[derive(Default)]
pub struct ViewMatrix(pub [[f32; 4]; 4]);
