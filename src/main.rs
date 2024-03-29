#[macro_use]
extern crate glium;
use std::error::Error;
use std::time::Instant;

use cgmath::{One, Quaternion};
use glium::glutin::dpi::LogicalSize;
use glium::glutin::event::Event;
use glium::glutin::event_loop::{ControlFlow, EventLoop};
use glium::Display;
use input::{Input, InputEvent};
use render::camera::{Camera, CameraSystem};
use render::renderer::{RenderMesh, RenderingSystem};
use settings::Settings;
use specs::WorldExt;

use specs::prelude::*;

mod input;
mod math;
mod render;
mod settings;
mod util;
mod world;

use world::ecs::bounds::Bounds;
use world::ecs::chunk_loader::{ChunkGenerator, ChunkLoader};
use world::ecs::physics::{Physics, Rigidbody};
use world::ecs::player::{Player, PlayerBlockBreak, PlayerMovement};
use world::ecs::Transform;
use world::World;

use crate::render::renderer::RenderJob;

/// Prepares a `Display` and `EventLoop` for rendering and updating.
fn init_display() -> (EventLoop<()>, Display) {
    use glium::glutin;

    // Set up window
    let event_loop = glutin::event_loop::EventLoop::new();
    let wb = glutin::window::WindowBuilder::new();
    let window = glutin::ContextBuilder::new()
        .with_depth_buffer(24)
        // .with_vsync(true)
        .build_windowed(wb, &event_loop)
        .unwrap();
    window
        .window()
        .set_cursor_grab(glutin::window::CursorGrabMode::Confined)
        .unwrap();
    window.window().set_cursor_visible(false);
    window.window().set_inner_size(LogicalSize::new(1600, 900));

    let display = glium::Display::from_gl_window(window).unwrap();
    (event_loop, display)
}

fn process_event(ev: Event<()>, control_flow: &mut ControlFlow) -> Option<InputEvent> {
    use glium::glutin;

    match ev {
        glutin::event::Event::WindowEvent { event, .. } => match event {
            glutin::event::WindowEvent::CloseRequested => {
                *control_flow = glutin::event_loop::ControlFlow::Exit;
                None
            }
            glutin::event::WindowEvent::KeyboardInput { input, .. } => {
                Some(InputEvent::Keyboard(input))
            }
            glutin::event::WindowEvent::MouseInput { state, button, .. } => {
                Some(InputEvent::MouseButton { button, state })
            }
            _ => None,
        },
        glutin::event::Event::DeviceEvent { event, .. } => match event {
            glutin::event::DeviceEvent::MouseMotion { delta: d } => {
                Some(InputEvent::MouseMotion { delta: d })
            }
            _ => None,
        },
        _ => None,
    }
}

fn read_settings(file: &str) -> Result<Settings, Box<dyn Error>> {
    let settings_str = std::fs::read_to_string(file)?;
    let settings = toml::from_str(&settings_str)?;
    Ok(settings)
}

fn main() {
    let (event_loop, display) = init_display();

    let settings = read_settings("resources/settings.toml").expect("Failed to read settings.toml");

    let mut renderer = crate::render::renderer::Renderer::new(display, &settings.renderer);

    let mut world = specs::World::new();
    world.register::<Transform>();
    world.register::<RenderMesh>();
    world.register::<Bounds>();
    world.register::<Camera>();
    world.register::<Rigidbody>();
    world.register::<Player>();

    let game_world = World::default();

    let camera = world
        .create_entity()
        .with(Player::default())
        .with(Transform::new(
            game_world.spawn(),
            vector3!(1.0, 1.0, 1.0),
            Quaternion::one(),
        ))
        .with(Camera::from(settings.renderer))
        .with(Rigidbody::default())
        .with(Bounds::new(
            vector3!(0.0, 0.0, 0.0),
            vector3!(0.5, 2.0, 0.5),
        ))
        .build();

    world.insert(game_world);
    let mut dispatcher = DispatcherBuilder::new()
        .with(CameraSystem::new(camera), "camera", &[])
        .with(Physics::new(), "physics", &[])
        .with(PlayerMovement::default(), "player_movement", &[])
        .with(PlayerBlockBreak::default(), "player_block_break", &[])
        .with(
            ChunkGenerator::new(settings.renderer.render_distance + 4),
            "chunk_generator",
            &[],
        )
        .with(
            ChunkLoader::new(settings.renderer.render_distance),
            "chunk_loader",
            &[],
        )
        .with(RenderingSystem, "rendering", &[])
        .build();

    world.insert(settings);
    dispatcher.setup(&mut world);

    let mut last_frame = Instant::now();
    event_loop.run(move |ev, _, control_flow| {
        // *control_flow = glium::glutin::event_loop::ControlFlow::WaitUntil(
        //     last_frame + std::time::Duration::from_nanos(16_666_667),
        // );

        match ev {
            glium::glutin::event::Event::MainEventsCleared => {
                let frame_start = Instant::now();
                let delta_time = (frame_start - last_frame).as_secs_f32();
                last_frame = frame_start;

                world.write_resource::<DeltaTime>().0 = delta_time;

                dispatcher.dispatch(&mut world);
                world.maintain();

                world.write_resource::<Input>().update();

                renderer.render(
                    world.write_storage::<Camera>().get_mut(camera).unwrap(),
                    world
                        .read_storage::<Transform>()
                        .get(camera)
                        .unwrap()
                        .position,
                    &world.read_resource::<RenderJob>(),
                );
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
