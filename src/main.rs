use std::error::Error;

use rustcraft::settings::Settings;

use bevy::core::TaskPoolThreadAssignmentPolicy;
use bevy::core_pipeline::dof::{DepthOfField, DepthOfFieldMode};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::prelude::*;
use bevy::render::view::ColorGrading;
use rustcraft::chunks::{
    chunk_loader::{
        gather_chunks, generate_chunks, load_chunks, mark_chunks, unload_chunks, ChunkLoader,
    },
    fade::{apply_fades, FadeMaterials, FULLY_VISIBLE},
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

    // Linear fog from a quarter of the way out to the edge of render distance, so
    // distant terrain fades gradually into the sky like a horizon and is fully
    // fogged by the time it ends. At the default distances it starts about where
    // full-detail terrain gives way to LODs.
    let fog_distance = render_distance as f32 * 16.0;
    let camera = commands
        .spawn((
            Transform::from_xyz(0.0, 2.0, 0.0),
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
                    start: fog_distance * 0.25,
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
        fade: FULLY_VISIBLE,
    });
    let water_material_handle = water_materials.add(WaterMaterial {
        color: LinearRgba::rgb(0.0, 0.85, 0.85),
        texture: Some(asset_server.load::<Image>("textures/blocks.png")),
        fade: FULLY_VISIBLE,
    });
    commands.insert_resource(FadeMaterials::new(
        chunk_material_handle.clone(),
        water_material_handle.clone(),
        &mut chunk_materials,
        &mut water_materials,
    ));
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
                .set(TaskPoolPlugin {
                    task_pool_options: TaskPoolOptions {
                        // Bevy gives async compute 25% of cores (at most 4) by default,
                        // but all terrain generation and meshing runs there, so it gets
                        // 60% instead. Asset IO gets one thread and systems the rest.
                        io: TaskPoolThreadAssignmentPolicy {
                            min_threads: 1,
                            max_threads: 1,
                            percent: 0.0,
                        },
                        async_compute: TaskPoolThreadAssignmentPolicy {
                            min_threads: 1,
                            max_threads: usize::MAX,
                            percent: 0.6,
                        },
                        ..default()
                    },
                })
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
                    apply_fades,
                )
                    .chain()
                    .after(gather_chunks)
                    .before(unload_chunks),
                unload_chunks,
                (player_move, player_look, update_underwater_effects).chain(),
            ),
        )
        .run();
}
