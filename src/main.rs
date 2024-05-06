#[macro_use]
extern crate glium;
use std::error::Error;
use std::f32::consts::PI;

use bevy::pbr::light_consts::lux;
use bevy::pbr::ScreenSpaceAmbientOcclusionBundle;
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
use world::ecs::player::{player_look, player_move};
use world::World;

use crate::world::ecs::player::{PlayerBundle, PlayerMovement};

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
    asset_server: Res<AssetServer>,
    mut ambient_light: ResMut<AmbientLight>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    commands.spawn(DirectionalLightBundle {
        directional_light: DirectionalLight {
            illuminance: lux::AMBIENT_DAYLIGHT,
            shadows_enabled: false,
            ..default()
        },
        ..default()
    });
    ambient_light.brightness = 5000.0;

    let game_world = World::default();
    let spawn = game_world.spawn();
    commands.spawn(game_world);

    info!("spawned at {:?}, {:?}, {:?}", spawn.x, spawn.y, spawn.z);

    let player = commands
        .spawn(PlayerBundle {
            transform_bundle: TransformBundle {
                local: Transform::from_xyz(spawn.x, spawn.y, spawn.z).looking_to(Dir3::Z, Dir3::Y),
                ..default()
            },
            ..default()
        })
        .id();

    let camera = commands
        .spawn(Camera3dBundle {
            transform: Transform::from_xyz(0.0, 2.0, 0.0),
            ..default()
        })
        .insert(ScreenSpaceAmbientOcclusionBundle::default())
        .id();
    commands.entity(player).push_children(&[camera]);

    let chunk_loader = ChunkLoader::new(32);
    commands.spawn(chunk_loader);

    let settings = read_settings("assets/settings.toml").expect("Failed to read settings.toml");
    commands.spawn(settings);

    let _ = asset_server.load::<Image>("textures/blocks.png");
}

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(ImagePlugin::default_nearest()))
        .insert_resource(ClearColor(Color::srgb_u8(135, 206, 235)))
        .insert_resource(Msaa::Off)
        .add_systems(Startup, setup_scene)
        .add_systems(Update, (generate_chunks, load_chunks).chain())
        .add_systems(Update, (player_move, player_look))
        .run();
}

#[derive(Default)]
pub struct DeltaTime(f32);

#[derive(Default)]
pub struct ElapsedTime(f32);
