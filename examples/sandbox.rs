use bevy::prelude::*;
use ochre_engine::settings::Settings;
use ochre_gameplay::birds::BirdPlugin;
use ochre_gameplay::breaking::BreakingPlugin;
use ochre_gameplay::critters::CritterPlugin;
use ochre_gameplay::debug::DebugOverlayPlugin;
use ochre_gameplay::flythrough::FlythroughPlugin;
use ochre_gameplay::player::{spawn_player, PlayerAssets, PlayerPlugin};
use ochre_terrain::curvature::Curvature;
use ochre_terrain::material::{block_atlas, ShaderInputs, TerrainAssets};
use ochre_terrain::{
    insert_terrain_resources, task_pool_plugin, TerrainPlugin, TerrainRenderPlugin,
};
use ochre_world::ambience::AmbiencePlugin;
use ochre_world::clouds::CloudLayer;
use ochre_world::sky::SkyPlugin;

#[derive(Resource)]
struct SandboxSettings(Settings);

fn setup_scene(
    mut commands: Commands,
    settings: Res<SandboxSettings>,
    mut images: ResMut<Assets<Image>>,
    mut terrain: TerrainAssets,
    mut player_assets: PlayerAssets,
    clouds: Res<CloudLayer>,
    curvature: Res<Curvature>,
) {
    let settings = settings.0;
    let renderer = settings.renderer;
    let layout = renderer.lod_layout();
    info!(
        "drawing terrain {} chunks out with {} LOD levels",
        layout.render_distance() as u32,
        layout.max_level
    );

    let game_world = ochre_world::load_world(layout.region_reach() as u32, settings.world);
    let world_bounds = game_world.bounds;
    info!("world seed is {}", game_world.seed());
    let spawn = game_world.spawn();
    commands.insert_resource(game_world);
    info!("spawned at {:?}, {:?}, {:?}", spawn.x, spawn.y, spawn.z);

    spawn_player(
        &mut commands,
        &mut player_assets,
        &renderer,
        spawn,
        look_direction(),
    );
    insert_terrain_resources(
        &mut commands,
        &renderer,
        settings.debug,
        world_bounds,
        &ShaderInputs {
            atlas: Some(images.add(block_atlas("assets/textures/blocks.png"))),
            clouds: clouds.clone(),
            curvature: curvature.buffer.clone(),
        },
        &mut terrain,
    );
}

fn main() {
    let settings = Settings::read("assets/settings.toml").expect("Failed to read settings.toml");
    let renderer = settings.renderer;
    let flythrough = std::env::var_os("OCHRE_FLYTHROUGH").is_some();
    let mut app = App::new();
    app.add_plugins((
        DefaultPlugins
            .set(ImagePlugin::default_nearest())
            .set(task_pool_plugin())
            .set(WindowPlugin {
                primary_window: Some(Window {
                    // uncapped, the GPU runs flat out and can starve the desktop's compositor until the PC stalls
                    // a recording runs unsynced, as the desktop throttles a hidden vsynced window to a crawl
                    present_mode: if flythrough {
                        bevy::window::PresentMode::AutoNoVsync
                    } else {
                        bevy::window::PresentMode::AutoVsync
                    },
                    // a flythrough is recorded at video size
                    resolution: if flythrough {
                        bevy::window::WindowResolution::new(1920, 1080)
                    } else {
                        default()
                    },
                    ..default()
                }),
                ..default()
            }),
        TerrainPlugin,
        TerrainRenderPlugin { renderer },
        SkyPlugin::new(&renderer),
        PlayerPlugin,
        BreakingPlugin,
        FlythroughPlugin,
        DebugOverlayPlugin,
        AmbiencePlugin,
        CritterPlugin,
        BirdPlugin,
    ))
    .insert_resource(SandboxSettings(settings))
    .insert_resource(winit_settings())
    .insert_resource(ClearColor(Color::BLACK))
    .add_systems(Startup, setup_scene)
    .run();
}

// the stats runs are unfocused, which Bevy otherwise throttles to 60 fps
fn winit_settings() -> bevy::winit::WinitSettings {
    use bevy::winit::{UpdateMode, WinitSettings};
    if std::env::var("OCHRE_STATS").is_ok() {
        WinitSettings {
            focused_mode: UpdateMode::Continuous,
            unfocused_mode: UpdateMode::Continuous,
        }
    } else {
        WinitSettings::game()
    }
}

// OCHRE_LOOK=x,y,z points the camera that way at the start, for framing a place by its coordinates
fn look_direction() -> Vec3 {
    std::env::var("OCHRE_LOOK")
        .ok()
        .and_then(|s| {
            let mut parts = s.split(',').map(|p| p.trim().parse::<f32>().ok());
            Some(Vec3::new(parts.next()??, parts.next()??, parts.next()??))
        })
        .filter(|v| v.length() > 0.0)
        .unwrap_or(Vec3::Z)
}
