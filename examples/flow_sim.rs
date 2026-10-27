// Runs the terrain systems headless (no window or GPU), as `lod_sim` does, to time frames while water
// flows. It lets the area round spawn load, times frames with the water still, then breaks the banks
// nearest the camera, lets the water flow out and times frames again, with how much is flowing. Fix the
// place with OCHRE_SEED and OCHRE_SPAWN (by a lake or river), and run with:
//   cargo run --release --example flow_sim -- [banks to break, default 8] [seconds each phase, default 30]
use std::time::{Duration, Instant};

use bevy::math::I64Vec3;
use bevy::prelude::*;
use ochre_base::block::BlockType;
use ochre_engine::settings::{DebugSettings, Settings};
use ochre_terrain::chunk::{ChunkCoordinate, CHUNK_SIZE};
use ochre_terrain::material::{
    ChunkMaterial, ShaderInputs, SprayMaterial, TerrainAssets, WaterMaterial,
};
use ochre_terrain::world::World;
use ochre_terrain::{insert_terrain_resources, task_pool_plugin, TerrainPlugin};
use ochre_world::worldmap::WorldConfig;

#[derive(Component)]
struct SimCamera;

#[derive(Resource)]
struct Config(ochre_terrain::settings::RendererSettings, WorldConfig);

fn setup(mut commands: Commands, config: Res<Config>, mut assets: TerrainAssets) {
    let renderer = &config.0;
    let world = ochre_world::load_world(renderer.lod_layout().region_reach() as u32, config.1);
    let (spawn, height) = (world.spawn(), world.bounds);
    commands.insert_resource(world);
    let player = commands.spawn(Transform::from_translation(spawn)).id();
    let camera = commands
        .spawn((
            SimCamera,
            Camera::default(),
            GlobalTransform::from_translation(spawn),
        ))
        .id();
    commands.entity(player).add_children(&[camera]);
    insert_terrain_resources(
        &mut commands,
        renderer,
        DebugSettings::default(),
        height,
        &ShaderInputs::default(),
        &mut assets,
    );
}

const SIDES: [I64Vec3; 4] = [I64Vec3::X, I64Vec3::NEG_X, I64Vec3::Z, I64Vec3::NEG_Z];

// Up to `count` banks a block thick round `centre`, nearest first and at least 8 blocks apart: a solid
// block with still water on one side and air on the other.
fn banks(world: &World, centre: I64Vec3, count: usize) -> Vec<I64Vec3> {
    let reach = 4;
    let middle = ChunkCoordinate::containing(centre).0;
    let mut found = Vec::new();
    for dx in -reach..=reach {
        for dy in -2..=2 {
            for dz in -reach..=reach {
                let coord = middle + I64Vec3::new(dx, dy, dz);
                let Some(data) = world.get_chunk_data(ChunkCoordinate(coord)) else {
                    continue;
                };
                for (local, block) in data.iter_non_air() {
                    let water = coord * CHUNK_SIZE as i64 + local.as_i64vec3();
                    if block != BlockType::Water || world.flow_at(water).is_some() {
                        continue;
                    }
                    for side in SIDES {
                        let bank = water + side;
                        let solid =
                            !matches!(world.get_block_at(bank), BlockType::Air | BlockType::Water);
                        if solid && world.get_block_at(bank + side) == BlockType::Air {
                            found.push(bank);
                        }
                    }
                }
            }
        }
    }
    found.sort_by_key(|b| (*b - centre).length_squared());
    let mut picked: Vec<I64Vec3> = Vec::new();
    for bank in found {
        if picked.iter().all(|p| (*p - bank).length_squared() >= 64) {
            picked.push(bank);
            if picked.len() == count {
                break;
            }
        }
    }
    picked
}

fn flowing(world: &World, centre: I64Vec3) -> usize {
    let middle = ChunkCoordinate::containing(centre).0;
    let mut count = 0;
    for dx in -6..=6 {
        for dy in -4..=2 {
            for dz in -6..=6 {
                if let Some(data) =
                    world.get_chunk_data(ChunkCoordinate(middle + I64Vec3::new(dx, dy, dz)))
                {
                    count += data.iter_flowing().count();
                }
            }
        }
    }
    count
}

// Frame times in ms, sorted: p50, p99 and the slowest.
fn summary(frames: &mut [f32]) -> String {
    frames.sort_by(f32::total_cmp);
    let at = |p: f32| frames[((frames.len() - 1) as f32 * p) as usize];
    format!(
        "{} frames, p50 {:.2} ms, p99 {:.2} ms, max {:.2} ms",
        frames.len(),
        at(0.5),
        at(0.99),
        frames[frames.len() - 1]
    )
}

fn main() {
    let settings = Settings::read("assets/settings.toml").expect("failed to read settings");
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins.set(task_pool_plugin()),
        AssetPlugin::default(),
        TerrainPlugin,
    ))
    .insert_resource(Config(settings.renderer, settings.world))
    .init_asset::<Mesh>()
    .init_asset::<Image>()
    .init_asset::<ChunkMaterial>()
    .init_asset::<WaterMaterial>()
    .init_asset::<SprayMaterial>()
    .add_systems(Startup, setup);
    app.update();

    let count: usize = std::env::args().nth(1).map_or(8, |s| s.parse().unwrap());
    let phase: f32 = std::env::args().nth(2).map_or(30.0, |s| s.parse().unwrap());
    let settle = 25.0;
    let start = Instant::now();
    let (mut still, mut flowing_frames) = (Vec::new(), Vec::new());
    let mut breached = false;
    let mut last_report = 0.0;
    let centre = app.world().resource::<World>().spawn().floor().as_i64vec3();
    loop {
        let t = start.elapsed().as_secs_f32();
        if t > settle + 2.0 * phase {
            break;
        }
        if t > settle + phase && !breached {
            breached = true;
            let mut world = app.world_mut().resource_mut::<World>();
            let picked = banks(&world, centre, count);
            for &bank in &picked {
                world.break_block(bank);
            }
            println!("t={t:5.1} broke {} banks: {picked:?}", picked.len());
        }
        let frame = Instant::now();
        app.update();
        let ms = frame.elapsed().as_secs_f32() * 1000.0;
        if t > settle + phase {
            flowing_frames.push(ms);
        } else if t > settle {
            still.push(ms);
        }
        if breached && t - last_report >= 2.0 {
            last_report = t;
            println!(
                "t={t:5.1} flowing blocks: {}",
                flowing(app.world().resource::<World>(), centre)
            );
        }
        // about 60 frames a second, as the game is capped by vsync
        std::thread::sleep(Duration::from_millis(16).saturating_sub(frame.elapsed()));
    }
    println!("still:   {}", summary(&mut still));
    println!("flowing: {}", summary(&mut flowing_frames));
}
