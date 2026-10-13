// Runs the chunk and LOD loading systems headless (no window or GPU) with the
// renderer settings from assets/settings.toml: lets the start area load, flies the
// player along +x, then stops. Every 2 seconds it reports, by distance in chunks,
// surface chunks with nothing visible drawing them, and full-detail chunks still
// drawn outside the full-detail area, and logs frames slower than 100ms. Run with:
//   cargo run --release --example lod_sim -- [speed in blocks/s, default 80] [seconds moving, default 10]
use std::time::{Duration, Instant};

use bevy::math::I64Vec3;
use bevy::prelude::*;
use bevy::utils::HashMap;
use rustcraft::chunks::chunk::{ChunkCoordinate, CHUNK_SIZE};
use rustcraft::chunks::chunk_loader::{ChunkLoader, DirtyChunk, GenerateChunkData, GenerateChunkMesh};
use rustcraft::chunks::group::{BuiltGroup, ChunkGroup, GroupedChunk, MeshedChunk};
use rustcraft::horizon::render_distance_at;
use rustcraft::chunks::fade::FULLY_VISIBLE;
use rustcraft::chunks::generate::noise::NoiseGenerator;
use rustcraft::chunks::generate::WATER_LEVEL;
use rustcraft::chunks::lod::{LodKey, LodLayout, LodLoader, LodNode};
use rustcraft::chunks::material::{ChunkMaterial, WaterMaterial};
use rustcraft::chunks::{insert_terrain_resources, task_pool_plugin, TerrainPlugin};
use rustcraft::settings::{DebugSettings, RendererSettings, Settings};

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
    let world = rustcraft::world::World::new(renderer.lod_layout().region_reach() as u32);
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
        // nothing's drawn, so no clouds
        cloud_pattern: Default::default(),
        cloud_layer: Default::default(),
        debug: DebugSettings::default(),
    });
    let water = water_materials.add(WaterMaterial {
        color: LinearRgba::WHITE,
        texture: None,
        fade: FULLY_VISIBLE,
    });
    insert_terrain_resources(
        &mut commands,
        renderer,
        &DebugSettings::default(),
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
    let group_vis: HashMap<LodKey, bool> = world
        .query::<(&ChunkGroup, &Visibility)>()
        .iter(world)
        .map(|(g, v)| (g.key, *v != Visibility::Hidden))
        .collect();
    let chunk_vis = |c: &ChunkCoordinate| group_vis.get(&LodKey::containing(*c, 1)).copied();
    let game_world = world.resource::<rustcraft::world::World>();
    // sampled straight from the noise, so reporting never computes regions for the game
    let noise = NoiseGenerator::new(game_world.seed());
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
            // from the same sample the regions full detail reads are made from
            let h = noise
                .sample((cx * size + size / 2) as f64, (cz * size + size / 2) as f64)
                .height
                .max(WATER_LEVEL as f64);
            let c = ChunkCoordinate(I64Vec3::new(cx, (h as i64 - 1).div_euclid(size), cz));
            let drawn = chunk_vis(&c).unwrap_or(false)
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
            if chunk_vis(&c).unwrap_or(false) && !layout.full_detail(c, anchor) {
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

// The surface chunks within `radius` chunks of `pos`, found once so checking them is cheap.
fn surface_chunks(app: &App, pos: Vec3, radius: i64) -> Vec<ChunkCoordinate> {
    let game_world = app.world().resource::<rustcraft::world::World>();
    let noise = NoiseGenerator::new(game_world.seed());
    let anchor = game_world.block_to_chunk_coordinate(pos.floor().as_i64vec3());
    let size = CHUNK_SIZE as i64;
    let mut chunks = Vec::new();
    for dx in -radius..=radius {
        for dz in -radius..=radius {
            if dx * dx + dz * dz > radius * radius {
                continue;
            }
            let (cx, cz) = (anchor.0.x + dx, anchor.0.z + dz);
            // from the same sample the regions full detail reads are made from
            let h = noise
                .sample((cx * size + size / 2) as f64, (cz * size + size / 2) as f64)
                .height
                .max(WATER_LEVEL as f64);
            chunks.push(ChunkCoordinate(I64Vec3::new(cx, (h as i64 - 1).div_euclid(size), cz)));
        }
    }
    chunks
}

// How many of `surface` have nothing visible drawing them, and the first few.
fn holes_in(app: &mut App, surface: &[ChunkCoordinate], max_level: u8) -> (usize, Vec<ChunkCoordinate>) {
    let world = app.world_mut();
    let lod_vis: HashMap<LodKey, bool> = world
        .query::<(&LodNode, &Visibility)>()
        .iter(world)
        .map(|(n, v)| (n.key(), *v != Visibility::Hidden))
        .collect();
    let group_vis: HashMap<LodKey, bool> = world
        .query::<(&ChunkGroup, &Visibility)>()
        .iter(world)
        .map(|(g, v)| (g.key, *v != Visibility::Hidden))
        .collect();
    let lods = world.resource::<LodLoader>();
    let mut count = 0;
    let mut first = Vec::new();
    for &c in surface {
        let drawn = group_vis.get(&LodKey::containing(c, 1)).copied().unwrap_or(false)
            || (1..=max_level).any(|l| lod_vis.get(&LodKey::containing(c, l)).copied().unwrap_or(false))
            // an empty node draws nothing because there's nothing to draw
            || (1..=max_level).any(|l| lods.node_is_empty(LodKey::containing(c, l)));
        if !drawn {
            count += 1;
            if first.len() < 3 {
                first.push(c);
            }
        }
    }
    (count, first)
}

// What's holding up the group over a chunk with nothing drawing it.
fn explain(app: &mut App, c: ChunkCoordinate) {
    let key = LodKey::containing(c, 1);
    let world = app.world_mut();
    let loader = world.resource::<ChunkLoader>();
    let group = loader.group_entity(key);
    let entities: Vec<_> = key.chunks().map(|chunk| (chunk, loader.chunk_entity(chunk))).collect();
    let lods = world.resource::<LodLoader>();
    let anchor = lods.anchor_chunk();
    println!("  chunk {:?}: level-1 node {key:?}, group entity {group:?}, anchor {anchor:?}, full detail {}", c.0, anchor.is_some_and(|a| lods.layout().full_detail(c, a)));
    for (chunk, entity) in entities {
        let Some(entity) = entity else {
            println!("    {:?}: no entity", chunk.0);
            continue;
        };
        let e = world.entity(entity);
        println!(
            "    {:?}: data {} dirty {} meshing {} meshed {} grouped {}",
            chunk.0,
            e.contains::<GenerateChunkData>(),
            e.contains::<DirtyChunk>(),
            e.contains::<GenerateChunkMesh>(),
            e.contains::<MeshedChunk>(),
            e.contains::<GroupedChunk>()
        );
    }
    if let Some(g) = group {
        let e = world.entity(g);
        println!("    group built {} visibility {:?}", e.contains::<BuiltGroup>(), e.get::<Visibility>());
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
    // `up` flies straight up instead, driving the render distance as the game's horizon does
    let climb = std::env::args().nth(3).is_some_and(|d| d == "up");
    let total_secs: f32 = 60.0 + move_secs;
    let start = Instant::now();
    let mut last_report = 0.0;
    let mut moving_from: Option<f32> = None;
    let mut surface: Vec<ChunkCoordinate> = Vec::new();
    let mut explained = false;
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
                    let step = if climb { Vec3::Y } else { Vec3::X };
                    *g = GlobalTransform::from_translation(g.translation() + step * speed * dt);
                }
            }
        }
        if climb {
            let world = app.world_mut();
            let y = world
                .query_filtered::<&GlobalTransform, With<SimCamera>>()
                .single(world)
                .translation()
                .y;
            let mut lods = world.resource_mut::<LodLoader>();
            let wanted = render_distance_at(lods.set_render_distance_chunks(), y);
            let current = lods.layout().render_distance();
            if (wanted - current).abs() > current * 0.1 {
                lods.set_render_distance(wanted);
            }
        }
        let frame = Instant::now();
        app.update();
        // a flash of sky lasts a few frames, so while climbing every frame is checked
        if climb && moving_from.is_some() {
            if surface.is_empty() {
                let world = app.world_mut();
                let pos = world
                    .query_filtered::<&GlobalTransform, With<SimCamera>>()
                    .single(world)
                    .translation();
                surface = surface_chunks(&app, pos, 60);
            }
            let max_level = app.world().resource::<LodLoader>().layout().max_level;
            let (count, first) = holes_in(&mut app, &surface, max_level);
            let world = app.world_mut();
            let y = world.query_filtered::<&GlobalTransform, With<SimCamera>>().single(world).translation().y;
            let levels = world.resource::<LodLoader>().layout().max_level;
            if count > 0 && !explained {
                explained = true;
                for c in first.clone() {
                    explain(&mut app, c);
                }
            }
            if count > 0 {
                println!("t={t:5.1} HOLES {count:4} of {} surface chunks under the camera at y={y:6.0}, {levels} levels, first {first:?}", surface.len());
            }
        }
        if frame.elapsed() > Duration::from_millis(100) {
            println!("t={t:5.1} slow frame: {:?}", frame.elapsed());
        }
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
