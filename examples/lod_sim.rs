// Runs the chunk and LOD loading systems headless (no window or GPU) with the
// renderer settings from assets/settings.toml: lets the start area load, flies the
// player along +x, then stops. Every 2 seconds it reports, by distance in chunks,
// surface chunks with nothing visible drawing them, and full-detail chunks still
// drawn outside the full-detail area. Run with:
//   cargo run --release --example lod_sim -- [speed in blocks/s, default 80] [seconds moving, default 10]
use std::time::{Duration, Instant};

use bevy::math::I64Vec3;
use bevy::prelude::*;
use bevy::utils::HashMap;
use rustcraft::chunks::chunk::{ChunkCoordinate, CHUNK_SIZE};
use rustcraft::chunks::chunk_loader::Chunk;
use rustcraft::chunks::fade::FULLY_VISIBLE;
use rustcraft::chunks::generate::WATER_LEVEL;
use rustcraft::chunks::lod::{LodKey, LodLayout, LodNode};
use rustcraft::chunks::material::{ChunkMaterial, WaterMaterial};
use rustcraft::chunks::{insert_terrain_resources, task_pool_plugin, TerrainPlugin};
use rustcraft::settings::{RendererSettings, Settings};

#[derive(Component)]
struct SimCamera;

#[derive(Resource)]
struct Renderer(RendererSettings);

fn setup(
    mut commands: Commands,
    renderer: Res<Renderer>,
    mut chunk_materials: ResMut<Assets<ChunkMaterial>>,
    mut water_materials: ResMut<Assets<WaterMaterial>>,
) {
    let renderer = &renderer.0;
    let world = rustcraft::world::World::new(renderer.lod_layout().render_distance() as u32);
    let spawn = world.spawn();
    let height = world.height;
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
    let chunk = chunk_materials.add(ChunkMaterial {
        color: LinearRgba::WHITE,
        texture: None,
        fade: FULLY_VISIBLE,
    });
    let water = water_materials.add(WaterMaterial {
        color: LinearRgba::WHITE,
        texture: None,
        fade: FULLY_VISIBLE,
    });
    insert_terrain_resources(
        &mut commands,
        renderer,
        height,
        chunk,
        water,
        &mut chunk_materials,
        &mut water_materials,
    );
}

struct Report {
    holes: [usize; 4],
    stale: usize,
    lods: usize,
}

fn report(app: &mut App, layout: LodLayout, pos: Vec3) -> Report {
    let world = app.world_mut();
    let lod_vis: HashMap<LodKey, bool> = world
        .query::<(&LodNode, &Visibility)>()
        .iter(world)
        .map(|(n, v)| (n.key(), *v != Visibility::Hidden))
        .collect();
    let chunk_vis: HashMap<ChunkCoordinate, bool> = world
        .query_filtered::<(&Chunk, &Visibility), With<Mesh3d>>()
        .iter(world)
        .map(|(c, v)| (c.coord, *v != Visibility::Hidden))
        .collect();
    let game_world = world.resource::<rustcraft::world::World>();
    let regions = game_world.region_store.clone();
    let anchor = game_world.block_to_chunk_coordinate(pos.floor().as_i64vec3());
    let full = layout.full_detail_distance;
    let far = layout.render_distance().min(120.0);
    let mut holes = [0; 4];
    let mut stale = 0;
    let reach = far as i64;
    for dx in -reach..=reach {
        for dz in -reach..=reach {
            let d = ((dx * dx + dz * dz) as f32).sqrt();
            if d > far {
                continue;
            }
            let (cx, cz) = (anchor.0.x + dx, anchor.0.z + dz);
            let size = CHUNK_SIZE as i64;
            let h = regions
                .get_height(cx * size + size / 2, cz * size + size / 2)
                .max(WATER_LEVEL as f64);
            let c = ChunkCoordinate(I64Vec3::new(cx, (h as i64 - 1).div_euclid(size), cz));
            let drawn = chunk_vis.get(&c).copied().unwrap_or(false)
                || (1..=layout.max_level).any(|l| {
                    lod_vis
                        .get(&LodKey::containing(c, l))
                        .copied()
                        .unwrap_or(false)
                });
            if !drawn {
                let band = if d < full {
                    0
                } else if d < full * 1.5 {
                    1
                } else if d < full * 4.0 {
                    2
                } else {
                    3
                };
                holes[band] += 1;
            }
            if chunk_vis.get(&c).copied().unwrap_or(false) && !layout.full_detail(c, anchor) {
                stale += 1;
            }
        }
    }
    Report {
        holes,
        stale,
        lods: lod_vis.len(),
    }
}

fn main() {
    let renderer = Settings::read("assets/settings.toml")
        .expect("failed to read settings")
        .renderer;
    let layout = renderer.lod_layout();
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins.set(task_pool_plugin()),
        AssetPlugin::default(),
        TerrainPlugin,
    ))
    .insert_resource(Renderer(renderer))
    .init_asset::<Mesh>()
    .init_asset::<Image>()
    .init_asset::<ChunkMaterial>()
    .init_asset::<WaterMaterial>()
    .add_systems(Startup, setup);
    app.update();

    let speed: f32 = std::env::args().nth(1).map_or(80.0, |s| s.parse().unwrap());
    let move_secs: f32 = std::env::args().nth(2).map_or(10.0, |s| s.parse().unwrap());
    let total_secs: f32 = 60.0 + move_secs;
    let start = Instant::now();
    let mut last_report = 0.0;
    let mut moving_from: Option<f32> = None;
    let settle = 20.0;
    loop {
        let t = start.elapsed().as_secs_f32();
        if t > total_secs + settle {
            break;
        }
        // let the start area load first
        if t > settle && moving_from.is_none() {
            moving_from = Some(t);
        }
        let dt = 1.0 / 60.0;
        if let Some(from) = moving_from {
            if t - from < move_secs {
                let world = app.world_mut();
                let mut q = world.query_filtered::<&mut GlobalTransform, With<SimCamera>>();
                for mut g in q.iter_mut(world) {
                    *g = GlobalTransform::from_translation(g.translation() + Vec3::X * speed * dt);
                }
            }
        }
        app.update();
        std::thread::sleep(Duration::from_millis(5));
        if t - last_report >= 2.0 {
            last_report = t;
            let world = app.world_mut();
            let pos = world
                .query_filtered::<&GlobalTransform, With<SimCamera>>()
                .single(world)
                .translation();
            let r = report(&mut app, layout, pos);
            let phase = match moving_from {
                None => "loading",
                Some(f) if t - f < move_secs => "moving",
                _ => "stopped",
            };
            println!(
                "t={t:5.1} {phase:8} x={:7.0} holes [full detail, 1.5x, 4x, beyond]: {:?}  stale full-detail: {}  lod nodes: {}",
                pos.x, r.holes, r.stale, r.lods
            );
        }
    }
}
