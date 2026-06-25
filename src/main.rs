use std::error::Error;

use rustcraft::settings::Settings;

use bevy::core_pipeline::dof::{DepthOfField, DepthOfFieldMode};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::prelude::*;
use bevy::render::view::ColorGrading;
use rustcraft::chunks::{
    chunk_loader::{
        gather_chunks, generate_chunks, load_chunks, mark_chunks, unload_chunks, ChunkLoader,
    },
    material::{ChunkMaterial, WaterMaterial},
};
use rustcraft::player::{player_look, player_move, update_underwater_effects, PlayerBundle};

fn read_settings(file: &str) -> Result<Settings, Box<dyn Error>> {
    let settings_str = std::fs::read_to_string(file)?;
    let settings = toml::from_str(&settings_str)?;
    Ok(settings)
}

fn setup_scene(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut chunk_materials: ResMut<Assets<ChunkMaterial>>,
    mut water_materials: ResMut<Assets<WaterMaterial>>,
) {
    let game_world = rustcraft::world::World::new();
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
    // Both Exponential and ExponentialSquared crept in well before the edge of
    // render distance, hazing most of the view. Linear keeps fog at exactly zero
    // until `start`, then ramps to fully opaque by `end` (the unload boundary).
    let fog_distance = render_distance as f32 * 16.0;
    let camera = commands
        .spawn((
            Transform::from_xyz(0.0, 2.0, 0.0),
            Camera3d { ..default() },
            // Depth of field needs HDR enabled on the camera.
            Camera { hdr: true, ..default() },
            // Camera3d requires Tonemapping, defaulting to TonyMcMapface - it
            // deliberately desaturates brights, which dulled this game's flatter,
            // saturated palette. None bypasses tonemapping, matching how colors
            // looked before HDR was enabled for depth of field.
            Tonemapping::None,
            Msaa::Off,
            DistanceFog {
                color: Color::srgb_u8(120, 224, 232),
                falloff: FogFalloff::Linear {
                    start: fog_distance - 128.0,
                    end: fog_distance,
                },
                ..default()
            },
            // Keeps nearby gameplay (within ~1.5 chunks) sharp and softens
            // everything farther out - Gaussian over Bokeh since render distance is
            // already large and Bokeh is the pricier of the two modes.
            DepthOfField {
                mode: DepthOfFieldMode::Gaussian,
                focal_distance: 24.0,
                aperture_f_stops: 1.4,
                ..default()
            },
            ColorGrading::default(),
        ))
        .id();
    commands.entity(player).add_children(&[camera]);

    let chunk_material_handle = chunk_materials.add(ChunkMaterial {
        color: LinearRgba::WHITE,
        texture: Some(asset_server.load::<Image>("textures/blocks.png")),
    });
    let water_material_handle = water_materials.add(WaterMaterial {
        color: LinearRgba::rgb(0.0, 0.85, 0.85),
        texture: Some(asset_server.load::<Image>("textures/blocks.png")),
    });
    let chunk_loader =
        ChunkLoader::new(render_distance as u32, chunk_material_handle, water_material_handle);
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
            MaterialPlugin::<WaterMaterial>::default(),
        ))
        .insert_resource(ClearColor(Color::srgb_u8(120, 224, 232)))
        .add_systems(Startup, setup_scene)
        .add_systems(
            Update,
            (
                (gather_chunks, generate_chunks, mark_chunks, load_chunks).before(unload_chunks),
                unload_chunks,
                (player_move, player_look, update_underwater_effects).chain(),
            ),
        )
        .run();
}
