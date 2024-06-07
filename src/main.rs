use std::error::Error;

use bevy::pbr::light_consts::lux;
use settings::Settings;

mod block;
mod chunks;
mod player;
mod settings;
mod util;
mod world;

use bevy::prelude::*;
use chunks::{
    chunk_loader::{
        gather_chunks, generate_chunks, load_chunks, mark_chunks, unload_chunks, ChunkLoader,
    },
    material::ChunkMaterial,
};
use player::{player_look, player_move, PlayerBundle};

fn read_settings(file: &str) -> Result<Settings, Box<dyn Error>> {
    let settings_str = std::fs::read_to_string(file)?;
    let settings = toml::from_str(&settings_str)?;
    Ok(settings)
}

fn setup_scene(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut ambient_light: ResMut<AmbientLight>,
) {
    commands.spawn(DirectionalLightBundle {
        directional_light: DirectionalLight {
            illuminance: lux::AMBIENT_DAYLIGHT,
            shadows_enabled: true,
            ..default()
        },
        ..default()
    });
    ambient_light.brightness = 5000.0;

    let game_world = crate::world::World::new();
    info!("world seed is {}", game_world.seed());
    let spawn = Vec3::new(0.0, 20.0, 0.0);
    commands.insert_resource(game_world);

    info!("spawned at {:?}, {:?}, {:?}", spawn.x, spawn.y, spawn.z);

    let player = commands
        .spawn(PlayerBundle {
            transform_bundle: TransformBundle {
                local: Transform::from_xyz(spawn.x, spawn.y, spawn.z)
                    .looking_to(Vec3::new(0.0, 0.0, 1.0), Vec3::new(0.0, 1.0, 0.0)),
                ..default()
            },
            ..default()
        })
        .id();

    let render_distance = 32;
    let camera = commands
        .spawn((Camera3dBundle {
            transform: Transform::from_xyz(0.0, 2.0, 0.0),
            ..default()
        },))
        .id();
    commands.entity(player).push_children(&[camera]);

    let chunk_loader = ChunkLoader::new(render_distance as u32);
    commands.insert_resource(chunk_loader);

    let settings = read_settings("assets/settings.toml").expect("Failed to read settings.toml");
    commands.spawn(settings);

    let _ = asset_server.load::<Image>("textures/blocks.png");
}

fn main() {
    App::new()
        .add_plugins((
            DefaultPlugins.set(ImagePlugin::default_nearest()),
            MaterialPlugin::<ChunkMaterial>::default(),
        ))
        .insert_resource(ClearColor(Color::rgb_u8(135, 206, 235)))
        .insert_resource(Msaa::Off)
        .add_systems(Startup, setup_scene)
        .add_systems(
            Update,
            (
                (gather_chunks, generate_chunks, mark_chunks, load_chunks)
                    .chain()
                    .before(unload_chunks),
                unload_chunks,
                player_move,
                player_look,
            ),
        )
        .run();
}
