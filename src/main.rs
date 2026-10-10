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
    lod::{
        load_lods, lod_render_distance, max_level_for, refresh_lod_visibility, retire_lods,
        spawn_lods, update_lod_selection, LodLoader,
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
    let settings = read_settings("assets/settings.toml").expect("Failed to read settings.toml");
    let max_lod_level = max_level_for(settings.renderer.render_distance);
    let render_distance = lod_render_distance(max_lod_level) as u32;
    info!("drawing terrain {render_distance} chunks out with {max_lod_level} LOD levels");

    let game_world = rustcraft::world::World::new(render_distance);
    let world_height = game_world.height;
    info!("world seed is {}", game_world.seed());
    let spawn = Vec3::new(0.0, 170.0, 0.0);
    commands.insert_resource(game_world);

    info!("spawned at {:?}, {:?}, {:?}", spawn.x, spawn.y, spawn.z);

    let player = commands
        .spawn(PlayerBundle {
            transform: Transform::from_xyz(spawn.x, spawn.y, spawn.z)
                .looking_to(Vec3::new(1.0, 0.0, 0.3), Vec3::new(0.0, 1.0, 0.0)),
            ..default()
        })
        .id();

    // Both Exponential and ExponentialSquared crept in well before the edge of
    // render distance, hazing most of the view. Linear keeps fog at exactly zero
    // until `start`, then ramps to fully opaque by `end` (the unload boundary).
    let fog_distance = render_distance as f32 * 16.0;
    let camera = commands
        .spawn((
            Transform::from_xyz(0.0, 2.0, 0.0).with_rotation(Quat::from_rotation_x(-0.35)),
            Camera3d { ..default() },
            Projection::Perspective(PerspectiveProjection {
                far: fog_distance + 256.0,
                ..default()
            }),
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
    commands.insert_resource(LodLoader::new(
        max_lod_level,
        world_height,
        chunk_material_handle.clone(),
        water_material_handle.clone(),
    ));
    commands.insert_resource(ChunkLoader::new(chunk_material_handle, water_material_handle));
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
                (
                    update_lod_selection,
                    spawn_lods,
                    load_lods,
                    retire_lods,
                    refresh_lod_visibility,
                )
                    .chain()
                    .after(gather_chunks)
                    .before(unload_chunks),
                unload_chunks,
                (player_move, player_look, update_underwater_effects).chain(),
                debug_probe,
            ),
        )
        .run();
}

fn debug_probe(
    mut commands: Commands,
    time: Res<Time>,
    mut last: Local<f32>,
    mut shots: Local<u32>,
    nodes: Query<&rustcraft::chunks::lod::LodNode>,
    pending: Query<(), With<rustcraft::chunks::lod::GenerateLodMesh>>,
    meshed_nodes: Query<(), (With<rustcraft::chunks::lod::LodNode>, With<Mesh3d>)>,
    chunks: Query<(), With<rustcraft::chunks::chunk_loader::Chunk>>,
    mut player: Query<&mut Transform, With<rustcraft::player::Player>>,
    mut exit: EventWriter<AppExit>,
) {
    use bevy::render::view::screenshot::{save_to_disk, Screenshot};
    let t = time.elapsed_secs();
    let schedule = [(150.0, "a_overview"), (190.0, "b_moving"), (230.0, "c_settled")];
    if t > 150.0 && t < 190.0 {
        player.single_mut().translation.x += 80.0 * time.delta_secs();
    }
    if t - *last >= 10.0 {
        *last = t;
        println!(
            "PROBE t={t:.0} fps={:.1} lod_nodes={} lod_pending={} lod_meshed={} chunks={}",
            1.0 / time.delta_secs(),
            nodes.iter().count(),
            pending.iter().count(),
            meshed_nodes.iter().count(),
            chunks.iter().count()
        );
    }
    if let Some((at, name)) = schedule.get(*shots as usize) {
        if t >= *at {
            let path = format!("/tmp/claude-0/-home-user-rustcraft/fcfd7dd2-90b8-5f73-9fc1-4522f7e380f0/scratchpad/{name}.png");
            commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
            *shots += 1;
        }
    } else if t >= 240.0 {
        exit.send(AppExit::Success);
    }
}
