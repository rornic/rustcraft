// A scripted, repeatable camera flight through the chunk and LOD loading systems, headless (no
// window or GPU), for comparing frame time and loading throughput between builds. The camera
// settles at spawn, then walks, sprints and flies high over the same line of ground, and the
// report gives each leg's frame time, chunks generated a second and generation queue depth.
//   cargo run --release --example flight -- [--seed 1] [--out target/flight.txt] [--secs 30]
// The report is `key: value` lines, so two runs diff cleanly.
use std::collections::HashSet;
use std::fmt::Write as _;
use std::time::{Duration, Instant};

use bevy::prelude::*;
use ochre_engine::settings::{DebugSettings, RendererSettings, Settings};
use ochre_terrain::chunk_loader::{Chunk, GenerateChunkData};
use ochre_terrain::group::ChunkGroup;
use ochre_terrain::horizon::render_distance_at;
use ochre_terrain::lod::{GenerateLodMesh, LodLoader, LodNode};
use ochre_terrain::material::{
    ChunkMaterial, ShaderInputs, SprayMaterial, TerrainAssets, WaterMaterial,
};
use ochre_terrain::{insert_terrain_resources, task_pool_plugin, TerrainPlugin};
use ochre_world::worldmap::WorldConfig;

const FRAME: f32 = 1.0 / 60.0;
const WALK_SPEED: f32 = 20.0;
const SPRINT_SPEED: f32 = 80.0;
const FLY_SPEED: f32 = 250.0;
const FLY_HEIGHT: f32 = 900.0;

#[derive(Component)]
struct FlightCamera;

#[derive(Resource)]
struct Renderer(RendererSettings, WorldConfig);

fn setup(mut commands: Commands, renderer: Res<Renderer>, mut assets: TerrainAssets) {
    let world_config = renderer.1;
    let renderer = &renderer.0;
    let world = ochre_world::load_world(renderer.lod_layout().region_reach() as u32, world_config);
    let spawn = world.spawn();
    let height = world.height;
    commands.insert_resource(world);
    let player = commands.spawn(Transform::from_translation(spawn)).id();
    let camera = commands
        .spawn((
            FlightCamera,
            Camera::default(),
            GlobalTransform::from_translation(spawn),
        ))
        .id();
    commands.entity(player).add_children(&[camera]);
    // nothing's drawn, so the shaders get no clouds, curvature or atlas
    insert_terrain_resources(
        &mut commands,
        renderer,
        DebugSettings::default(),
        height,
        &ShaderInputs::default(),
        &mut assets,
    );
}

#[derive(Default)]
struct Leg {
    frames_ms: Vec<f64>,
    chunk_queue: Vec<usize>,
    lod_queue: Vec<usize>,
    chunks_done: u64,
    lods_done: u64,
    wall: Duration,
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    sorted[((sorted.len() - 1) as f64 * p).round() as usize]
}

impl Leg {
    fn report(&mut self, name: &str, out: &mut String) {
        self.frames_ms.sort_by(f64::total_cmp);
        let f = &self.frames_ms;
        let secs = self.wall.as_secs_f64().max(1e-9);
        let mean = |q: &[usize]| q.iter().sum::<usize>() as f64 / q.len().max(1) as f64;
        let _ = writeln!(out, "{name} frames: {}", f.len());
        let _ = writeln!(out, "{name} frame_ms p50: {:.3}", percentile(f, 0.5));
        let _ = writeln!(out, "{name} frame_ms p99: {:.3}", percentile(f, 0.99));
        let _ = writeln!(out, "{name} frame_ms worst: {:.3}", percentile(f, 1.0));
        let _ = writeln!(
            out,
            "{name} chunks_generated_per_s: {:.1}",
            self.chunks_done as f64 / secs
        );
        let _ = writeln!(
            out,
            "{name} lod_meshes_per_s: {:.1}",
            self.lods_done as f64 / secs
        );
        let _ = writeln!(
            out,
            "{name} chunk_queue mean/max: {:.1}/{}",
            mean(&self.chunk_queue),
            self.chunk_queue.iter().max().copied().unwrap_or(0)
        );
        let _ = writeln!(
            out,
            "{name} lod_queue mean/max: {:.1}/{}",
            mean(&self.lod_queue),
            self.lod_queue.iter().max().copied().unwrap_or(0)
        );
    }
}

struct Flight {
    app: App,
    in_flight_chunks: HashSet<Entity>,
    in_flight_lods: HashSet<Entity>,
    climb: bool,
}

impl Flight {
    fn step(&mut self, leg: &mut Leg) {
        let start = Instant::now();
        self.app.update();
        let elapsed = start.elapsed();
        leg.frames_ms.push(elapsed.as_secs_f64() * 1000.0);
        // paced like a 60 fps game, so tasks get the wall time they would in play
        let paced = elapsed.max(Duration::from_secs_f32(FRAME));
        std::thread::sleep(paced - elapsed);
        leg.wall += paced;

        let world = self.app.world_mut();
        let chunks: HashSet<Entity> = world
            .query_filtered::<Entity, With<GenerateChunkData>>()
            .iter(world)
            .collect();
        let lods: HashSet<Entity> = world
            .query_filtered::<Entity, With<GenerateLodMesh>>()
            .iter(world)
            .collect();
        // finished, not unloaded before they were
        leg.chunks_done += self
            .in_flight_chunks
            .difference(&chunks)
            .filter(|&&e| world.get::<Chunk>(e).is_some())
            .count() as u64;
        leg.lods_done += self
            .in_flight_lods
            .difference(&lods)
            .filter(|&&e| world.get_entity(e).is_ok())
            .count() as u64;
        leg.chunk_queue.push(chunks.len());
        leg.lod_queue.push(lods.len());
        self.in_flight_chunks = chunks;
        self.in_flight_lods = lods;
    }

    // What the meshes hold, in megabytes: every vertex and index buffer, split by what draws it.
    fn mesh_memory(&mut self) -> String {
        let world = self.app.world_mut();
        let size = |mesh: &Mesh| {
            (mesh.get_vertex_buffer_size() + mesh.indices().map_or(0, |i| i.len() * 4)) as f64 / 1e6
        };
        let vertices = |mesh: &Mesh| mesh.count_vertices();
        let handles: Vec<(Option<u8>, bool, bevy::asset::AssetId<Mesh>)> = {
            let mut out = Vec::new();
            let mut lods = world.query::<(&LodNode, &Mesh3d)>();
            for (node, mesh) in lods.iter(world) {
                out.push((Some(node.key().level), false, mesh.0.id()));
            }
            let mut groups = world.query::<(&ChunkGroup, &Mesh3d)>();
            for (_, mesh) in groups.iter(world) {
                out.push((None, true, mesh.0.id()));
            }
            out
        };
        let meshes = world.resource::<Assets<Mesh>>();
        let total: f64 = meshes.iter().map(|(_, m)| size(m)).sum();
        let total_vertices: usize = meshes.iter().map(|(_, m)| vertices(m)).sum();
        let mut by_level = [(0.0f64, 0usize, 0usize); 12];
        let (mut chunks, mut chunk_vertices, mut chunk_meshes) = (0.0, 0, 0);
        for (level, is_chunk, id) in handles {
            let Some(mesh) = meshes.get(id) else { continue };
            if is_chunk {
                chunks += size(mesh);
                chunk_vertices += vertices(mesh);
                chunk_meshes += 1;
            } else if let Some(level) = level {
                let slot = &mut by_level[level as usize % 12];
                slot.0 += size(mesh);
                slot.1 += vertices(mesh);
                slot.2 += 1;
            }
        }
        let mut out = format!(
            "all meshes: {total:.0} MB, {total_vertices} vertices, {:.1} bytes a vertex\nfull-detail chunk groups: {chunks:.0} MB, {chunk_vertices} vertices, {chunk_meshes} meshes\n",
            total * 1e6 / total_vertices.max(1) as f64
        );
        for (level, (mb, v, n)) in by_level.iter().enumerate().filter(|(_, s)| s.2 > 0) {
            out += &format!("lod level {level}: {mb:.0} MB, {v} vertices, {n} nodes\n");
        }
        out
    }

    fn camera(&mut self) -> Vec3 {
        let world = self.app.world_mut();
        world
            .query_filtered::<&GlobalTransform, With<FlightCamera>>()
            .single(world)
            .unwrap()
            .translation()
    }

    fn place(&mut self, position: Vec3) {
        let world = self.app.world_mut();
        let mut query = world.query_filtered::<&mut GlobalTransform, With<FlightCamera>>();
        for mut transform in query.iter_mut(world) {
            *transform = GlobalTransform::from_translation(position);
        }
        if self.climb {
            let mut lods = world.resource_mut::<LodLoader>();
            let wanted = render_distance_at(lods.set_render_distance_chunks(), position.y);
            let current = lods.layout().render_distance();
            if (wanted - current).abs() > current * 0.1 {
                lods.set_render_distance(wanted);
            }
        }
    }

    fn ground(&mut self, x: f32, z: f32) -> f32 {
        let world = self.app.world();
        let world = world.resource::<ochre_terrain::world::World>();
        let height = world
            .region_store
            .terrain()
            .sample(x as f64, z as f64)
            .height;
        height.max(ochre_base::WATER_LEVEL as f64) as f32
    }

    // Along +x at `speed` for `secs` of game time, `above` the ground, or at a fixed height.
    fn fly(&mut self, secs: f32, speed: f32, height: Height) -> Leg {
        let mut leg = Leg::default();
        let from = self.camera();
        for frame in 0..(secs / FRAME) as usize {
            let x = from.x + speed * FRAME * frame as f32;
            let y = match height {
                Height::Above(blocks) => self.ground(x, from.z) + blocks,
                Height::At(y) => y,
            };
            self.place(Vec3::new(x, y, from.z));
            self.step(&mut leg);
        }
        leg
    }
}

#[derive(Copy, Clone)]
enum Height {
    Above(f32),
    At(f32),
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let seed = arg("--seed").unwrap_or_else(|| "1".into());
    let secs: f32 = arg("--secs").map_or(30.0, |s| s.parse().expect("--secs is a number"));
    let out = arg("--out");
    std::env::set_var("OCHRE_SEED", &seed);

    let settings = Settings::read("assets/settings.toml").expect("failed to read settings");
    let (renderer, world) = (settings.renderer, settings.world);
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins.set(task_pool_plugin()),
        AssetPlugin::default(),
        TerrainPlugin,
    ))
    .insert_resource(Renderer(renderer, world))
    .init_asset::<Mesh>()
    .init_asset::<Image>()
    .init_asset::<ChunkMaterial>()
    .init_asset::<WaterMaterial>()
    .init_asset::<SprayMaterial>()
    .add_systems(Startup, setup);
    app.update();

    let mut flight = Flight {
        app,
        in_flight_chunks: HashSet::new(),
        in_flight_lods: HashSet::new(),
        climb: false,
    };
    let spawn = flight.camera();
    let mut report = String::new();
    let _ = writeln!(report, "seed: {seed}");
    let _ = writeln!(report, "render_distance: {}", renderer.render_distance);

    // the start area loading in, standing still
    let mut load = Leg::default();
    for _ in 0..(secs / FRAME) as usize {
        flight.step(&mut load);
    }
    load.report("load", &mut report);
    let _ = writeln!(report, "after load:\n{}", flight.mesh_memory());

    let mut walk = flight.fly(secs, WALK_SPEED, Height::Above(2.0));
    walk.report("walk", &mut report);
    let _ = writeln!(report, "after walk:\n{}", flight.mesh_memory());
    let mut sprint = flight.fly(secs, SPRINT_SPEED, Height::Above(2.0));
    sprint.report("sprint", &mut report);

    // the horizon pushes the render distance out with the height
    flight.climb = true;
    let x = flight.camera().x;
    flight.place(Vec3::new(x, FLY_HEIGHT, spawn.z));
    let mut high = flight.fly(secs, FLY_SPEED, Height::At(FLY_HEIGHT));
    high.report("high", &mut report);

    print!("{report}");
    if let Some(path) = out {
        std::fs::write(path, &report).expect("failed to write the report");
    }
}
