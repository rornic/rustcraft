use std::error::Error;

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
    mut chunk_materials: ResMut<Assets<ChunkMaterial>>,
) {
    let game_world = crate::world::World::new();
    info!("world seed is {}", game_world.seed());
    let spawn = Vec3::new(0.0, 20.0, 0.0);
    commands.insert_resource(game_world);

    info!("spawned at {:?}, {:?}, {:?}", spawn.x, spawn.y, spawn.z);

    let player = commands
        .spawn(PlayerBundle {
            transform: Transform::from_xyz(spawn.x, spawn.y, spawn.z)
                .looking_to(Vec3::new(0.0, 0.0, 1.0), Vec3::new(0.0, 1.0, 0.0)),
            ..default()
        })
        .id();

    let render_distance = 64;
    let camera = commands
        .spawn((
            Transform::from_xyz(0.0, 2.0, 0.0),
            Camera3d { ..default() },
            Msaa::Off,
        ))
        .id();
    commands.entity(player).add_children(&[camera]);

    let chunk_material_handle = chunk_materials.add(ChunkMaterial {
        color: LinearRgba::WHITE,
        texture: Some(asset_server.load::<Image>("textures/blocks.png")),
    });
    let chunk_loader = ChunkLoader::new(render_distance as u32, chunk_material_handle);
    commands.insert_resource(chunk_loader);

    let settings = read_settings("assets/settings.toml").expect("Failed to read settings.toml");
    commands.spawn(settings);
}

fn main() {
    App::new()
        .add_plugins((
            DefaultPlugins
                .set(ImagePlugin::default_nearest())
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        present_mode: bevy::window::PresentMode::AutoNoVsync,
                        ..default()
                    }),
                    ..default()
                }),
            MaterialPlugin::<ChunkMaterial>::default(),
        ))
        .insert_resource(ClearColor(Color::srgb_u8(135, 206, 235)))
        .add_systems(Startup, setup_scene)
        .add_systems(
            Update,
            (
                (gather_chunks, generate_chunks, mark_chunks, load_chunks).before(unload_chunks),
                unload_chunks,
                player_move,
                player_look,
            ),
        )
        .run();
}
