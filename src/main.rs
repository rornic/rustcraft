#[macro_use]
extern crate glium;
use std::time::Instant;

use glium::glutin::event::Event;
use glium::glutin::event_loop::{ControlFlow, EventLoop};
use glium::Display;
use input::Input;
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

use world::components::Transform;
use world::systems::CameraSystem;

/// Prepares a `Display` and `EventLoop` for rendering and updating.
fn init_display() -> (EventLoop<()>, Display) {
    use glium::glutin;

    // Set up window
    let event_loop = glutin::event_loop::EventLoop::new();
    let wb = glutin::window::WindowBuilder::new();
    let cb = glutin::ContextBuilder::new()
        .with_depth_buffer(24)
        .with_vsync(true);
    let display = glium::Display::new(wb, cb, &event_loop).unwrap();
    (event_loop, display)
}

fn process_event(input: &mut Input, ev: Event<()>, control_flow: &mut ControlFlow) {
    use glium::glutin;

    // Handle window events
    match ev {
        glutin::event::Event::WindowEvent { event, .. } => match event {
            glutin::event::WindowEvent::CloseRequested => {
                *control_flow = glutin::event_loop::ControlFlow::Exit;
                return;
            }
            _ => (),
        },
        glutin::event::Event::DeviceEvent { event, .. } => match event {
            glutin::event::DeviceEvent::Key(ki) => {
                input.keyboard.process_event(ki);
            }
            _ => (),
        },
        _ => (),
    };
}

fn main() {
    let (event_loop, display) = init_display();

    let game_world = world::World::new();
    let world_mesh = game_world.generate_chunk_mesh();

    let mut renderer = Renderer::new(display);
    renderer.register_mesh(&world_mesh).unwrap();

    let mut world = specs::World::new();
    world.register::<Transform>();
    world.register::<RenderMesh>();

    world.insert(ViewMatrix::default());
    world.insert(DeltaTime(0.0));
    world.insert(ElapsedTime(0.0));

    world
        .create_entity()
        .with(Transform::new(
            vector3!(0.0, 0.0, 25.0),
            vector3!(1.0, 1.0, 1.0),
        ))
        .with(RenderMesh::new(&world_mesh))
        .build();

    let mut dispatcher: Dispatcher = DispatcherBuilder::new()
        .with_thread_local(RenderingSystem)
        .with(CameraSystem::new(&mut world), "camera", &[])
        .build();
    dispatcher.setup(&mut world);

    event_loop.run(move |ev, _, control_flow| match ev {
        glium::glutin::event::Event::MainEventsCleared => {
            let frame_start = Instant::now();

            dispatcher.run_now(&world);
            world.maintain();

            renderer.render(&mut world);

            let delta_time = (Instant::now() - frame_start).as_secs_f32();
            world.write_resource::<DeltaTime>().0 = delta_time;
            world.write_resource::<ElapsedTime>().0 += delta_time;
        }
        ev => process_event(&mut world.write_resource::<Input>(), ev, control_flow),
    });
}

#[derive(Default)]
pub struct DeltaTime(f32);

#[derive(Default)]
pub struct ElapsedTime(f32);
