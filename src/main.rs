#[macro_use]
extern crate glium;
use std::error::Error;

use glium::glutin::dpi::LogicalSize;
use glium::glutin::event::Event;
use glium::glutin::event_loop::{ControlFlow, EventLoop};
use glium::Display;
use input::InputEvent;
use settings::Settings;

mod input;
mod math;
mod render;
mod settings;
mod util;
mod world;

use bevy::prelude::*;
use world::ecs::chunk_loader::{generate_chunks, load_chunks, ChunkLoader};
use world::World;

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

fn setup_scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn(DirectionalLightBundle {
        directional_light: DirectionalLight {
            shadows_enabled: true,
            ..default()
        },
        transform: Transform::from_xyz(4.0, 8.0, 4.0),
        ..default()
    });

    let game_world = World::default();
    let spawn = game_world.spawn();
    commands.spawn(game_world);

    info!("spawned at {:?}, {:?}, {:?}", spawn.x, spawn.y, spawn.z);

    commands.spawn(Camera3dBundle {
        transform: Transform::from_xyz(spawn.x, spawn.y, spawn.y)
            .looking_at(Vec3::new(spawn.x, spawn.y, spawn.z + 10.0), Vec3::Y),
        projection: Projection::Perspective(PerspectiveProjection {
            near: 0.1,
            far: 10000.0,
            ..default()
        }),
        ..default()
    });

    let chunk_loader = ChunkLoader::new(16);
    commands.spawn(chunk_loader);

    let settings = read_settings("resources/settings.toml").expect("Failed to read settings.toml");
    commands.spawn(settings);
}

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .insert_resource(ClearColor(Color::ALICE_BLUE))
        .add_systems(Startup, setup_scene)
        .add_systems(Update, (generate_chunks, load_chunks).chain())
        .run();
}

#[derive(Default)]
pub struct DeltaTime(f32);

#[derive(Default)]
pub struct ElapsedTime(f32);
