#[macro_use]
extern crate glium;
use std::collections::VecDeque;
use std::time::Instant;

use glium::glutin::event::Event;
use glium::glutin::event_loop::{ControlFlow, EventLoop};
use glium::Display;
use input::{Input, InputEvent};
use render::RenderMesh;
use render::Renderer;
use render::RenderingSystem;
use render::ViewMatrix;
use specs::WorldExt;

use specs::prelude::*;

mod input;
mod math;
mod render;
mod util;
mod world;

use world::ecs::camera::{Camera, CameraSystem};
use world::ecs::chunk_loader::{ChunkGeneratorSystem, ChunkLoaderSystem};
use world::ecs::Transform;

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

    let mut renderer = Renderer::new(display);

    let mut world = specs::World::new();
    world.register::<Transform>();
    world.register::<RenderMesh>();
    world.register::<Camera>();

    world.insert(ViewMatrix::default());
    world.insert(DeltaTime(0.0));
    world.insert(ElapsedTime(0.0));

    let mut dispatcher = DispatcherBuilder::new()
        .with(RenderingSystem, "rendering", &[])
        .with(CameraSystem::new(&mut world), "camera", &[])
        .with(ChunkGeneratorSystem::new(), "chunk_generator", &[])
        .with(
            ChunkLoaderSystem::new(),
            "chunk_loader",
            &["chunk_generator"],
        )
        .build_async(world);
    dispatcher.setup();

    let mut event_buffer = VecDeque::new();

    event_loop.run(move |ev, _, control_flow| match ev {
        glium::glutin::event::Event::MainEventsCleared => {
            let frame_start = Instant::now();

            dispatcher.dispatch();

            let world = dispatcher.world_mut();
            world.write_resource::<Input>().update();
            while let Some(ie) = event_buffer.pop_front() {
                world.write_resource::<Input>().process_event(ie);
            }

            renderer.render(world);

            let delta_time = (Instant::now() - frame_start).as_secs_f32();
            world.write_resource::<DeltaTime>().0 = delta_time;
            world.write_resource::<ElapsedTime>().0 += delta_time;

            dispatcher.wait();
        }
        ev => {
            if let Some(e) = process_event(ev, control_flow) {
                event_buffer.push_back(e);
            }
        }
    });
}

#[derive(Default)]
pub struct DeltaTime(f32);

#[derive(Default)]
pub struct ElapsedTime(f32);
