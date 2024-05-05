#[macro_use]
extern crate glium;
use std::error::Error;

use bevy::render::render_resource::Texture;
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

fn setup_scene(mut commands: Commands, asset_server: Res<AssetServer>) {
    commands.spawn(DirectionalLightBundle {
        directional_light: DirectionalLight::default(),
        transform: Transform::from_xyz(0.0, 800.0, 0.0).looking_at(Vec3::ZERO, Vec3::Y),
        ..default()
    });

    let game_world = World::default();
    let spawn = game_world.spawn();
    commands.spawn(game_world);

    info!("spawned at {:?}, {:?}, {:?}", spawn.x, spawn.y, spawn.z);

    commands.spawn(Camera3dBundle {
        transform: Transform::from_xyz(spawn.x, spawn.y, spawn.y)
            .looking_at(Vec3::new(spawn.x, spawn.y, spawn.z + 10.0), Vec3::Y),
        ..default()
    });

    let chunk_loader = ChunkLoader::new(32);
    commands.spawn(chunk_loader);

    let settings = read_settings("assets/settings.toml").expect("Failed to read settings.toml");
    commands.spawn(settings);

    asset_server.load::<Image>("textures/blocks.png");
}

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(ImagePlugin::default_nearest()))
        .insert_resource(ClearColor(Color::ALICE_BLUE))
        .insert_resource(Msaa::Off)
        .add_systems(Startup, setup_scene)
        .add_systems(Update, (generate_chunks, load_chunks).chain())
        .run();
}

#[derive(Default)]
pub struct DeltaTime(f32);

#[derive(Default)]
pub struct ElapsedTime(f32);
