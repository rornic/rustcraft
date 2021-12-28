#[macro_use]
extern crate glium;
use std::time::Instant;

use glium::glutin::event::Event;
use glium::glutin::event_loop::{ControlFlow, EventLoop};
use glium::{glutin::event::VirtualKeyCode, Surface};
use glium::{Display, DrawParameters, Program};
use input::Input;
use render::mesh::{Mesh, RenderMesh, Renderer, RenderingSystem, ViewMatrix};
use specs::WorldExt;

use specs::prelude::*;

use crate::render::mesh::GlobalRenderUniforms;

mod input;
mod render;
mod util;
mod world;

use world::{Transform, Vector3};

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

    let mut elapsed_time: f32 = 0.0;
    let mut delta_time: f32 = 0.0;

    let mut camera_pos = Vector3 {
        x: 0.0,
        y: 5.0,
        z: 0.0,
    };

    // Prepare keyboard for input
    let mut input = Input::new();

    let mut renderer = Renderer::new(display);

    let mut game_world = world::World::new();
    let world_mesh = game_world.generate_chunk_mesh();

    renderer.register_mesh(&world_mesh).unwrap();

    let mut world = specs::World::new();
    world.register::<Transform>();
    world.register::<RenderMesh>();
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
        .build();
    dispatcher.setup(&mut world);
    event_loop.run(move |ev, _, control_flow| match ev {
        glium::glutin::event::Event::MainEventsCleared => {
            let frame_start = Instant::now();

            dispatcher.run_now(&world);
            world.maintain();

            renderer.render(&mut world);
            update(delta_time, &mut camera_pos, &input);

            delta_time = (Instant::now() - frame_start).as_secs_f32();
            elapsed_time += delta_time;
        }
        ev => process_event(&mut input, ev, control_flow),
    });
}

fn update(delta_time: f32, camera_pos: &mut Vector3, input: &Input) {
    if input.keyboard.is_pressed(VirtualKeyCode::A) {
        camera_pos.x -= 3.0 * delta_time;
    } else if input.keyboard.is_pressed(VirtualKeyCode::D) {
        camera_pos.x += 3.0 * delta_time;
    }

    if input.keyboard.is_pressed(VirtualKeyCode::W) {
        camera_pos.z += 3.0 * delta_time;
    } else if input.keyboard.is_pressed(VirtualKeyCode::S) {
        camera_pos.z -= 3.0 * delta_time;
    }
}
