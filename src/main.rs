use rustcraft::settings::Settings;

use bevy::core_pipeline::bloom::Bloom;
use bevy::core_pipeline::dof::{DepthOfField, DepthOfFieldMode};
use bevy::prelude::*;
use rustcraft::chunks::{
    fade::FULLY_VISIBLE,
    insert_terrain_resources,
    material::{block_atlas, ChunkMaterial, WaterMaterial},
    task_pool_plugin, TerrainPlugin,
};
use rustcraft::clouds::{spawn_clouds, CloudLayer, CloudPlugin};
use rustcraft::horizon::update_horizon;
use rustcraft::player::{player_look, player_move, update_underwater_effects, PlayerBundle};
use rustcraft::sky::{gradient_skybox, sun, FOG_MAX_OPACITY, HORIZON_COLOR};
use rustcraft::tonemap::{dry_color_grading, scene_colour, TONEMAPPING};

fn setup_scene(
    mut commands: Commands,
    mut chunk_materials: ResMut<Assets<ChunkMaterial>>,
    mut water_materials: ResMut<Assets<WaterMaterial>>,
    mut images: ResMut<Assets<Image>>,
    clouds: Res<CloudLayer>,
) {
    let settings = Settings::read("assets/settings.toml").expect("Failed to read settings.toml");
    let renderer = settings.renderer;
    let layout = renderer.lod_layout();
    let render_distance = layout.render_distance() as u32;
    info!(
        "drawing terrain {render_distance} chunks out with {} LOD levels",
        layout.max_level
    );

    let game_world = rustcraft::world::World::new(layout.region_reach() as u32);
    let world_height = game_world.height;
    info!("world seed is {}", game_world.seed());
    let spawn = game_world.spawn();
    commands.insert_resource(game_world);

    info!("spawned at {:?}, {:?}, {:?}", spawn.x, spawn.y, spawn.z);

    let player = commands
        .spawn(PlayerBundle {
            transform: Transform::from_xyz(spawn.x, spawn.y, spawn.z)
                .looking_to(Vec3::new(0.0, 0.0, 1.0), Vec3::new(0.0, 1.0, 0.0)),
            ..default()
        })
        .id();

    let fog_distance = render_distance as f32 * 16.0;
    let camera = commands
        .spawn((
            Transform::from_xyz(0.0, 2.0, 0.0),
            Camera3d { ..default() },
            gradient_skybox(&mut images),
            Projection::Perspective(PerspectiveProjection {
                far: fog_distance + world_height as f32,
                ..default()
            }),
            // Depth of field, bloom and sunlight brighter than white all need HDR.
            Camera { hdr: true, ..default() },
            TONEMAPPING,
            // Just a soft glow around the brightest sunlit faces and sky.
            Bloom {
                intensity: 0.05,
                ..Bloom::NATURAL
            },
            Msaa::Off,
            DistanceFog {
                color: scene_colour(HORIZON_COLOR).with_alpha(FOG_MAX_OPACITY).into(),
                falloff: FogFalloff::Linear {
                    // half fogged where the farthest LOD level starts, hiding the swap to it,
                    // see `fog.wgsl`
                    start: layout.last_level_start() * 16.0,
                    end: fog_distance,
                },
                ..default()
            },
            // In focus 8 blocks out, blurred ~1px at 16 blocks and ~2px from 150 on, so LOD seams soften. Gaussian is cheaper than Bokeh.
            DepthOfField {
                mode: DepthOfFieldMode::Gaussian,
                focal_distance: 8.0,
                aperture_f_stops: 2.2,
                ..default()
            },
            dry_color_grading(),
        ))
        .id();
    commands.entity(player).add_children(&[camera]);
    let shadow_distance = (renderer.shadow_distance as f32 * 16.0).min(fog_distance);
    commands.spawn(sun(shadow_distance, renderer.shadow_cascades));

    let atlas = images.add(block_atlas("assets/textures/blocks.png"));
    let chunk_material_handle = chunk_materials.add(ChunkMaterial {
        color: LinearRgba::WHITE,
        texture: Some(atlas.clone()),
        fade: FULLY_VISIBLE,
        cloud_pattern: clouds.pattern.clone(),
        cloud_layer: clouds.layer,
        debug: settings.debug,
    });
    let water_material_handle = water_materials.add(WaterMaterial {
        color: LinearRgba::rgb(0.0, 0.85, 0.85),
        texture: Some(atlas),
        fade: FULLY_VISIBLE,
    });
    insert_terrain_resources(
        &mut commands,
        &renderer,
        &settings.debug,
        world_height,
        chunk_material_handle,
        water_material_handle,
        &mut chunk_materials,
        &mut water_materials,
    );
    commands.spawn(settings);
}

fn main() {
    App::new()
        .add_plugins((
            DefaultPlugins
                .set(ImagePlugin::default_nearest())
                .set(task_pool_plugin())
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        present_mode: bevy::window::PresentMode::AutoNoVsync,
                        ..default()
                    }),
                    ..default()
                }),
            MaterialPlugin::<ChunkMaterial>::default(),
            // light passes through water, so it casts no shadow
            MaterialPlugin::<WaterMaterial> {
                shadows_enabled: false,
                ..default()
            },
            TerrainPlugin,
            CloudPlugin,
        ))
        .insert_resource(ClearColor(scene_colour(HORIZON_COLOR).into()))
        .add_systems(Startup, setup_scene.after(spawn_clouds))
        .add_systems(
            Update,
            (player_move, player_look, update_horizon, update_underwater_effects).chain(),
        )
        .run();
}
