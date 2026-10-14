use rustcraft::settings::Settings;

use bevy::post_process::bloom::Bloom;
use bevy::post_process::dof::{DepthOfField, DepthOfFieldMode};
use bevy::anti_alias::taa::TemporalAntiAliasing;
use bevy::core_pipeline::prepass::{DepthPrepass, MotionVectorPrepass};
use bevy::prelude::*;
use rustcraft::chunks::{
    fade::FULLY_VISIBLE,
    insert_terrain_resources,
    material::{block_atlas, see_through_reach, ChunkMaterial, WaterMaterial},
    task_pool_plugin, TerrainPlugin,
};
use rustcraft::clouds::{spawn_clouds, CloudLayer, CloudPlugin};
use rustcraft::curvature::{Curvature, CurvaturePlugin};
use rustcraft::debug::DebugOverlayPlugin;
use rustcraft::horizon::update_horizon;
use rustcraft::horizon_glow::HorizonGlowPlugin;
use rustcraft::player::{murk, player_look, player_move, update_underwater_effects, MurkMaterial, PlayerBundle};
use rustcraft::sky::{atmosphere_camera, spawn_atmosphere, sun, FOG_MAX_OPACITY};
use rustcraft::tonemap::{dry_color_grading, TONEMAPPING};

fn setup_scene(
    mut commands: Commands,
    mut chunk_materials: ResMut<Assets<ChunkMaterial>>,
    mut water_materials: ResMut<Assets<WaterMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut murk_materials: ResMut<Assets<MurkMaterial>>,
    clouds: Res<CloudLayer>,
    curvature: Res<Curvature>,
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
            atmosphere_camera(),
            Projection::Perspective(PerspectiveProjection {
                far: fog_distance + world_height as f32,
                ..default()
            }),
            // Depth of field, bloom and sunlight brighter than white all need HDR.
            Camera::default(),
            bevy::camera::Hdr,
            TONEMAPPING,
            // Just a soft glow around the brightest sunlit faces and sky.
            Bloom {
                intensity: 0.05,
                ..Bloom::NATURAL
            },
            // TAA for the thin faces and one-block steps that shimmer at a distance
            Msaa::Off,
            DepthPrepass,
            MotionVectorPrepass,
            TemporalAntiAliasing::default(),
            DistanceFog {
                color: Color::NONE.with_alpha(FOG_MAX_OPACITY),
                falloff: FogFalloff::Linear {
                    // half fogged where the farthest LOD level starts, hiding the swap to it,
                    // see `fog.wesl`
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
    let (murk, murk_handle) = murk(&mut meshes, &mut murk_materials);
    commands.spawn((murk, ChildOf(camera)));
    commands.insert_resource(murk_handle);
    let shadow_distance = (renderer.shadow_distance as f32 * 16.0).min(fog_distance);
    commands.spawn(sun(shadow_distance, renderer.shadow_cascades));

    let atlas = images.add(block_atlas("assets/textures/blocks.png"));
    let chunk_material_handle = chunk_materials.add(ChunkMaterial {
        color: LinearRgba::WHITE,
        texture: Some(atlas.clone()),
        fade: FULLY_VISIBLE,
        cloud_pattern: clouds.pattern.clone(),
        cloud_layer: clouds.layer,
        curvature: curvature.buffer.clone(),
        debug: settings.debug,
    });
    let water_material_handle = water_materials.add(WaterMaterial {
        color: LinearRgba::rgb(0.0, 0.85, 0.85),
        texture: Some(atlas),
        fade: FULLY_VISIBLE,
        cloud_pattern: clouds.pattern.clone(),
        cloud_layer: clouds.layer,
        see_through: see_through_reach(layout),
        curvature: curvature.buffer.clone(),
        solid: false,
    });
    let solid_water = WaterMaterial {
        solid: true,
        ..water_materials.get(&water_material_handle).unwrap().clone()
    };
    let solid_water_material_handle = water_materials.add(solid_water);
    insert_terrain_resources(
        &mut commands,
        &renderer,
        &settings.debug,
        world_height,
        chunk_material_handle,
        water_material_handle,
        solid_water_material_handle,
        &mut chunk_materials,
        &mut water_materials,
    );
    commands.spawn(settings);
}

// the stats runs are unfocused, which Bevy otherwise throttles to 60 fps
fn winit_settings() -> bevy::winit::WinitSettings {
    use bevy::winit::{UpdateMode, WinitSettings};
    if std::env::var("RC_STATS").is_ok() {
        WinitSettings { focused_mode: UpdateMode::Continuous, unfocused_mode: UpdateMode::Continuous }
    } else {
        WinitSettings::game()
    }
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
            MaterialPlugin::<WaterMaterial>::default(),
            MaterialPlugin::<MurkMaterial>::default(),
            TerrainPlugin,
            CloudPlugin,
            CurvaturePlugin,
            HorizonGlowPlugin,
            DebugOverlayPlugin,
        ))
        .insert_resource(winit_settings())
        .insert_resource(ClearColor(Color::BLACK))
        .add_systems(Startup, (spawn_atmosphere, setup_scene.after(spawn_clouds)))
        .add_systems(
            Update,
            (player_move, player_look, update_horizon, update_underwater_effects).chain(),
        )
        .run();
}
