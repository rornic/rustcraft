// Runs the chunk and LOD loading systems headless (no window or GPU) with the
// renderer settings from assets/settings.toml: lets the start area load, flies the
// player along +x, then stops. Every 2 seconds it reports, by distance in chunks,
// surface chunks with nothing visible drawing them, and full-detail chunks still
// drawn outside the full-detail area, and logs frames slower than 100ms. Run with:
//   cargo run --release --example lod_sim -- [speed in blocks/s, default 80] [seconds moving, default 10]
use std::time::{Duration, Instant};

use bevy::math::{I64Vec3, Vec2};
use bevy::render::mesh::{Indices, VertexAttributeValues};
use rustcraft::chunks::generate::generator::generate_lod_mesh;
use bevy::prelude::*;
use bevy::platform::collections::HashMap;
use rustcraft::chunks::chunk::{ChunkCoordinate, CHUNK_SIZE};
use rustcraft::chunks::chunk_loader::{ChunkLoader, DirtyChunk, GenerateChunkData, GenerateChunkMesh};
use rustcraft::chunks::group::{BuiltGroup, ChunkGroup, GroupedChunk, MeshedChunk};
use std::sync::Arc;
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

fn inside(p: Vec2, [a, b, c]: [Vec2; 3]) -> bool {
    let sign = |p1: Vec2, p2: Vec2, p3: Vec2| (p1.x - p3.x) * (p2.y - p3.y) - (p2.x - p3.x) * (p1.y - p3.y);
    let (d1, d2, d3) = (sign(p, a, b), sign(p, b, c), sign(p, c, a));
    let (neg, pos) = (d1 < -1e-4 || d2 < -1e-4 || d3 < -1e-4, d1 > 1e-4 || d2 > 1e-4 || d3 > 1e-4);
    !(neg && pos)
}

// Shoots straight down at points around `centre`, against the geometry of every node and group
// that is drawn, and reports points where nothing is there: holes seen from above.
fn probe_holes(app: &mut App, centre: Vec3, radius: f32, samples: usize) {
    let world = app.world_mut();
    let regions = world.resource::<rustcraft::world::World>().region_store.clone();
    let shown = |v: &Visibility| *v != Visibility::Hidden;
    let nodes: Vec<LodKey> = world
        .query::<(&LodNode, &Visibility, &Mesh3d)>()
        .iter(world)
        .filter(|(_, v, _)| shown(v))
        .map(|(n, _, _)| n.key())
        .collect();
    let groups: Vec<LodKey> = world
        .query_filtered::<(&ChunkGroup, &Visibility), With<BuiltGroup>>()
        .iter(world)
        .filter(|(_, v)| shown(v))
        .map(|(g, _)| g.key)
        .collect();
    println!("probing {} visible nodes and {} visible groups, world seed {}", nodes.len(), groups.len(), world.resource::<rustcraft::world::World>().seed());
    let mut bins: HashMap<(i32, i32), Vec<[Vec2; 3]>> = HashMap::new();
    let mut bins3: HashMap<(i32, i32), Vec<([Vec3; 3], LodKey)>> = HashMap::new();
    let mut add = |mesh: &Mesh, key: LodKey| {
        let cell = key.cell_size() as f32;
        let origin = key.min_block().as_vec3() + Vec3::splat(0.5 * cell - 0.5);
        let Some(VertexAttributeValues::Float32x3(positions)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else { return };
        let Some(Indices::U32(indices)) = mesh.indices() else { return };
        for t in indices.chunks(3) {
            let tri3 = [0, 1, 2].map(|k| origin + Vec3::from(positions[t[k] as usize]) * cell);
            let tri = tri3.map(|p| Vec2::new(p.x, p.z));
            let (lo, hi) = (tri[0].min(tri[1]).min(tri[2]), tri[0].max(tri[1]).max(tri[2]));
            for bx in (lo.x / 16.0).floor() as i32..=(hi.x / 16.0).floor() as i32 {
                for bz in (lo.y / 16.0).floor() as i32..=(hi.y / 16.0).floor() as i32 {
                    bins.entry((bx, bz)).or_default().push(tri);
                    bins3.entry((bx, bz)).or_default().push((tri3, key));
                }
            }
        }
    };
    for &key in &nodes {
        if let Some(meshes) = generate_lod_mesh(Arc::clone(&regions), key) {
            add(&meshes.opaque, key);
            if let Some(water) = &meshes.water {
                add(water, key);
            }
        }
    }
    let lods = world.resource::<LodLoader>();
    // every visible group's chunks, meshed as the game does, with real neighbours
    {
        use rustcraft::chunks::chunk::{neighbor_26_index, ChunkCoordinate, ChunkData};
        use rustcraft::chunks::generate::generator::{generate_chunk, generate_chunk_mesh};
        let mut data: HashMap<ChunkCoordinate, Arc<ChunkData>> = HashMap::new();
        let mut wanted: Vec<ChunkCoordinate> = Vec::new();
        for g in &groups {
            for c in g.chunks() {
                wanted.push(c);
                for dx in -1..=1i64 {
                    for dy in -1..=1i64 {
                        for dz in -1..=1i64 {
                            let n = ChunkCoordinate(c.0 + I64Vec3::new(dx, dy, dz));
                            data.entry(n).or_insert_with(|| Arc::new(generate_chunk(Arc::clone(&regions), n)));
                        }
                    }
                }
            }
        }
        println!("meshing {} chunks of the visible groups", wanted.len());
        for c in wanted {
            if data[&c].empty() {
                continue;
            }
            let mut adjacent: Vec<Option<Arc<ChunkData>>> = vec![None; 26];
            for dx in -1i64..=1 {
                for dy in -1i64..=1 {
                    for dz in -1i64..=1 {
                        if (dx, dy, dz) != (0, 0, 0) {
                            adjacent[neighbor_26_index(dx as i32, dy as i32, dz as i32)] = data.get(&ChunkCoordinate(c.0 + I64Vec3::new(dx, dy, dz))).cloned();
                        }
                    }
                }
            }
            let meshes = generate_chunk_mesh(Arc::clone(&data[&c]), adjacent, c, regions.noise(), false);
            let origin = (c.0 * 16).as_vec3();
            let key = LodKey::containing(c, 1);
            for mesh in std::iter::once(&meshes.opaque).chain(meshes.water.iter()) {
                let Some(VertexAttributeValues::Float32x3(positions)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else { continue };
                let Some(Indices::U32(indices)) = mesh.indices() else { continue };
                for t in indices.chunks(3) {
                    let tri3 = [0, 1, 2].map(|k| origin + Vec3::from(positions[t[k] as usize]));
                    let tri = tri3.map(|p| Vec2::new(p.x, p.z));
                    let (lo, hi) = (tri[0].min(tri[1]).min(tri[2]), tri[0].max(tri[1]).max(tri[2]));
                    for bx in (lo.x / 16.0).floor() as i32..=(hi.x / 16.0).floor() as i32 {
                        for bz in (lo.y / 16.0).floor() as i32..=(hi.y / 16.0).floor() as i32 {
                            bins.entry((bx, bz)).or_default().push(tri);
                            bins3.entry((bx, bz)).or_default().push((tri3, key));
                        }
                    }
                }
            }
        }
    }
    let group_covers = |_p: Vec2| false;
    let _unused_group_footprint = |p: Vec2| {
        groups.iter().any(|g| {
            let min = g.min_block();
            p.x >= min.x as f32 - 0.6 && p.x <= min.x as f32 + 31.6 && p.y >= min.z as f32 - 0.6 && p.y <= min.z as f32 + 31.6
        })
    };
    let mut rng = 0x9E3779B97F4A7C15u64;
    let mut next = || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        (rng >> 11) as f32 / (1u64 << 53) as f32
    };
    let (mut misses, mut shown_examples) = (0, 0);
    for _ in 0..samples {
        let (angle, distance) = (next() * std::f32::consts::TAU, radius * next().sqrt());
        let p = Vec2::new(centre.x + angle.cos() * distance, centre.z + angle.sin() * distance);
        let covered = bins
            .get(&((p.x / 16.0).floor() as i32, (p.y / 16.0).floor() as i32))
            .is_some_and(|tris| tris.iter().any(|&t| inside(p, t)))
            || group_covers(p);
        if !covered {
            misses += 1;
            if shown_examples < 12 {
                shown_examples += 1;
                let near: Vec<String> = nodes
                    .iter()
                    .filter(|k| {
                        let (min, w) = (k.min_block(), (k.width() * 16) as f32);
                        p.x > min.x as f32 - 40.0 && p.x < min.x as f32 + w + 40.0 && p.y > min.z as f32 - 40.0 && p.y < min.z as f32 + w + 40.0
                    })
                    .map(|k| format!("L{} y{}", k.level, k.coord.y))
                    .collect();
                println!("  hole at ({:.1}, {:.1}), {:.0} blocks from the camera; nodes within 40 blocks: {near:?}", p.x, p.y, distance);
                let ground = regions.noise().sample(p.x as f64, p.y as f64).height;
                let chunk = rustcraft::chunks::chunk::ChunkCoordinate(I64Vec3::new(p.x.round() as i64, ground as i64, p.y.round() as i64).div_euclid(I64Vec3::splat(16)));
                println!("    ground there {ground:.2}; the nodes in its column:");
                for level in 1..=3u8 {
                    for dy in -1..=1i64 {
                        let centre = LodKey::containing(chunk, level);
                        let k = LodKey { coord: centre.coord + I64Vec3::new(0, dy, 0), ..centre };
                        println!("      L{level} y{} (blocks {}..{}): shown {}, held as {}, selected {}, stand-in {}", k.coord.y, k.min_block().y, k.min_block().y + k.width() * 16, nodes.contains(&k), lods.node_kind(k), lods.is_selected(k), lods.is_stand_in(k));
                    }
                }
            }
        }
    }
    println!("{misses} of {samples} sample points within {radius} blocks are covered by nothing, seen from above");

    // and at an angle, down onto the ground from each of four sides, where a missing wall shows
    let noise = regions.noise();
    let (mut leaks, mut shown_leaks, mut rays) = (0, 0, 0);
    for k in 0..samples {
        let (angle, distance) = (next() * std::f32::consts::TAU, radius * next().sqrt());
        let (x, z) = (centre.x + angle.cos() * distance, centre.z + angle.sin() * distance);
        let y = noise.sample(x as f64, z as f64).height as f32;
        let direction = [Vec3::new(0.35, -1.0, 0.1), Vec3::new(-0.35, -1.0, 0.1), Vec3::new(0.0, -1.0, 0.45), Vec3::new(0.2, -1.0, -0.3)][k % 4].normalize();
        let from = Vec3::new(x, y, z) - direction * (300.0 / -direction.y);
        rays += 1;
        // the bins the ray's footprint crosses, from where it starts to a little past the ground
        let steps = 40;
        let mut hit = false;
        'bins: for step in 0..=steps {
            let at = from + (Vec3::new(x, y, z) + direction * 6.0 - from) * (step as f32 / steps as f32);
            let bin = ((at.x / 16.0).floor() as i32, (at.z / 16.0).floor() as i32);
            if let Some(tris) = bins3.get(&bin) {
                for &([a, b, c], _) in tris {
                    let (e1, e2) = (b - a, c - a);
                    let pv = direction.cross(e2);
                    let det = e1.dot(pv);
                    if det.abs() < 1e-9 {
                        continue;
                    }
                    let t = from - a;
                    let u = t.dot(pv) / det;
                    let q = t.cross(e1);
                    let v = direction.dot(q) / det;
                    if u >= 0.0 && v >= 0.0 && u + v <= 1.0 && e2.dot(q) / det > 0.0 {
                        hit = true;
                        break 'bins;
                    }
                }
            }
        }
        // checked against every triangle, in case the bins it samples along the ray missed one
        let hit = hit
            || group_covers(Vec2::new(x, z))
            || bins3.values().flatten().any(|&([a, b, c], _)| {
                let (e1, e2) = (b - a, c - a);
                let pv = direction.cross(e2);
                let det = e1.dot(pv);
                if det.abs() < 1e-9 {
                    return false;
                }
                let t = from - a;
                let u = t.dot(pv) / det;
                let q = t.cross(e1);
                let v = direction.dot(q) / det;
                u >= 0.0 && v >= 0.0 && u + v <= 1.0 && e2.dot(q) / det > 0.0
            });
        let p2 = Vec2::new(x, z);
        if !hit && !group_covers(p2) {
            leaks += 1;
            if shown_leaks < 4 {
                // what is around where the ray goes through the ground: triangles near its footprint at the ground's height
                let ground = Vec3::new(x, y, z);
                let mut around: Vec<(f32, LodKey, [Vec3; 3])> = Vec::new();
                for step in -60..=60 {
                    let at = ground + direction * (step as f32 * 0.1);
                    if let Some(tris) = bins3.get(&((at.x / 16.0).floor() as i32, (at.z / 16.0).floor() as i32)) {
                        for &(t, key) in tris {
                            let near = t.iter().any(|v| (*v - at).length() < 0.9);
                            if near && !around.iter().any(|(_, k, u)| *k == key && *u == t) {
                                around.push((step as f32 * 0.1, key, t));
                            }
                        }
                    }
                }
                // the mesh's top surface under each point of the ray's path, against the ray's own height
                // every node that covers the first point with nothing under it, shown or not, and how it's held
                if let Some(gap) = (-30..=30).map(|st| ground + direction * (st as f32 * 0.25)).find(|at| {
                    bins3.get(&((at.x / 16.0).floor() as i32, (at.z / 16.0).floor() as i32)).is_none_or(|tris| !tris.iter().any(|(t, _)| inside(Vec2::new(at.x, at.z), t.map(|v| Vec2::new(v.x, v.z))) && (t[1] - t[0]).cross(t[2] - t[0]).y.abs() > 1e-6))
                        && at.y < y + 1.0
                }) {
                    let sampled = regions.noise().sample(gap.x as f64, gap.z as f64).height;
                    let (low, high) = regions.column_height_range(LodKey::containing(rustcraft::chunks::chunk::ChunkCoordinate(I64Vec3::new(gap.x.floor() as i64, y as i64, gap.z.floor() as i64).div_euclid(I64Vec3::splat(16))), 2)).unwrap_or((0.0, 0.0));
                    if shown_leaks == 0 {
                        // every triangle whose footprint touches the few blocks just past the first gap point
                        println!("  every triangle over x {:.1}..{:.1}, z {:.1}..{:.1}:", gap.x - 1.5, gap.x + 1.5, gap.z - 0.6, gap.z + 4.0);
                        let mut seen = Vec::new();
                        for tris in bins3.values() {
                            for &(t, key) in tris {
                                let (lo, hi) = (t[0].min(t[1]).min(t[2]), t[0].max(t[1]).max(t[2]));
                                if hi.x >= gap.x - 1.5 && lo.x <= gap.x + 1.5 && hi.z >= gap.z - 0.6 && lo.z <= gap.z + 4.0 && !seen.contains(&(key, t)) {
                                    seen.push((key, t));
                                }
                            }
                        }
                        // from the seam on, which is the neighbour's side
                        seen.retain(|(_, t)| t.iter().any(|v| v.z > gap.z - 0.1));
                        seen.sort_by(|a, b| a.1[0].z.total_cmp(&b.1[0].z));
                        println!("  {} triangles past the seam", seen.len());
                        for (key, t) in seen.iter().take(30) {
                            let n = (t[1] - t[0]).cross(t[2] - t[0]).normalize_or_zero();
                            println!("    L{} node ({},{},{}) normal ({:.0},{:.0},{:.0}) verts {:?}", key.level, key.coord.x, key.coord.y, key.coord.z, n.x, n.y, n.z, t.map(|v| (v.x, v.y, v.z)));
                        }
                    }
                    println!("  first point with no surface under it: ({:.2}, {:.2}), where the ground is {sampled:.2} (a level 2 node's columns run {low:.1} to {high:.1}); nodes whose columns contain it:", gap.x, gap.z);
                    for level in 1..=6u8 {
                        let key = LodKey::containing(rustcraft::chunks::chunk::ChunkCoordinate(I64Vec3::new(gap.x.round() as i64, y as i64, gap.z.round() as i64).div_euclid(I64Vec3::splat(16))), level);
                        for dy in -2..=2i64 {
                            let k = LodKey { coord: key.coord + I64Vec3::new(0, dy, 0), ..key };
                            let is_shown = nodes.contains(&k);
                            let (mn, w) = (k.min_block(), k.width() * 16);
                            println!("    L{level} y{} (blocks {}..{}): shown {is_shown}, held as {}, selected {}, stand-in {}", k.coord.y, mn.y, mn.y + w, lods.node_kind(k), lods.is_selected(k), lods.is_stand_in(k));
                        }
                    }
                    // the group over it and its chunks
                    let key = LodKey::containing(rustcraft::chunks::chunk::ChunkCoordinate(I64Vec3::new(gap.x.round() as i64, y as i64, gap.z.round() as i64).div_euclid(I64Vec3::splat(16))), 1);
                    let loader = world.resource::<ChunkLoader>();
                    // the chunk at the gap meshed here on its own with its real neighbours, to see whether its own mesh has the surface
                    {
                        use rustcraft::chunks::chunk::{neighbor_26_index, ChunkCoordinate, ChunkData};
                        use rustcraft::chunks::generate::generator::{generate_chunk, generate_chunk_mesh};
                        let centre = ChunkCoordinate(I64Vec3::new(gap.x.round() as i64, y as i64, gap.z.round() as i64).div_euclid(I64Vec3::splat(16)));
                        let mut data: HashMap<ChunkCoordinate, Arc<ChunkData>> = HashMap::new();
                        for dx in -2..=2 {
                            for dy in -2..=2 {
                                for dz in -2..=2 {
                                    let c = ChunkCoordinate(centre.0 + I64Vec3::new(dx, dy, dz));
                                    data.insert(c, Arc::new(generate_chunk(Arc::clone(&regions), c)));
                                }
                            }
                        }
                        let mut adjacent: Vec<Option<Arc<ChunkData>>> = vec![None; 26];
                        for dx in -1i64..=1 {
                            for dy in -1i64..=1 {
                                for dz in -1i64..=1 {
                                    if (dx, dy, dz) != (0, 0, 0) {
                                        adjacent[neighbor_26_index(dx as i32, dy as i32, dz as i32)] = data.get(&ChunkCoordinate(centre.0 + I64Vec3::new(dx, dy, dz))).cloned();
                                    }
                                }
                            }
                        }
                        let meshes = generate_chunk_mesh(Arc::clone(&data[&centre]), adjacent, centre, regions.noise(), false);
                        let origin = (centre.0 * 16).as_vec3();
                        let Some(VertexAttributeValues::Float32x3(positions)) = meshes.opaque.attribute(Mesh::ATTRIBUTE_POSITION) else { panic!("no positions") };
                        let Some(Indices::U32(indices)) = meshes.opaque.indices() else { panic!("no indices") };
                        let mut tops = 0;
                        let mut covering = 0;
                        for t in indices.chunks(3) {
                            let tri = [0, 1, 2].map(|k| origin + Vec3::from(positions[t[k] as usize]));
                            let n = (tri[1] - tri[0]).cross(tri[2] - tri[0]);
                            if n.y.abs() > 1e-6 {
                                tops += 1;
                                if inside(Vec2::new(gap.x, gap.z), tri.map(|v| Vec2::new(v.x, v.z))) {
                                    covering += 1;
                                }
                            }
                        }
                        println!("  chunk {:?} meshed alone: {} vertices, {tops} horizontal triangles, {covering} cover the gap point; data empty {}", centre.0, meshes.opaque.count_vertices(), data[&centre].empty());
                    }
                    println!("  groups over it, level-1 key {:?}:", key.coord);
                    for dy in -1..=1i64 {
                        let k = LodKey { coord: key.coord + I64Vec3::new(0, dy, 0), ..key };
                        let state = loader.group_entity(k).map(|e| (world.entity(e).contains::<BuiltGroup>(), world.entity(e).get::<Visibility>().copied()));
                        println!("    group y{}: (built, visibility) {:?}", k.coord.y, state);
                    }
                    for chunk in key.chunks().filter(|c| c.0.y == key.min_chunk().y + 1 || c.0.y == key.min_chunk().y) {
                        let state = loader.chunk_entity(chunk).map(|e| {
                            let ent = world.entity(e);
                            (ent.contains::<GenerateChunkData>(), ent.contains::<DirtyChunk>(), ent.contains::<GenerateChunkMesh>(), ent.contains::<MeshedChunk>(), ent.contains::<GroupedChunk>())
                        });
                        println!("    chunk {:?}: (data, dirty, meshing, meshed, grouped) {:?}", chunk.0, state);
                    }
                }
                println!("  profile along the ray (ray y vs top surface under it, node):");
                for step in (-30..=30).step_by(3) {
                    let at = ground + direction * (step as f32 * 0.25);
                    let mut top: Option<(f32, LodKey)> = None;
                    if let Some(tris) = bins3.get(&((at.x / 16.0).floor() as i32, (at.z / 16.0).floor() as i32)) {
                        for &(t, key) in tris {
                            let tri2 = t.map(|v| Vec2::new(v.x, v.z));
                            if inside(Vec2::new(at.x, at.z), tri2) {
                                let n = (t[1] - t[0]).cross(t[2] - t[0]);
                                if n.y.abs() > 1e-6 {
                                    let h = t[0].y - (n.x * (at.x - t[0].x) + n.z * (at.z - t[0].z)) / n.y;
                                    if top.is_none_or(|(b, _)| h > b) {
                                        top = Some((h, key));
                                    }
                                }
                            }
                        }
                    }
                    match top {
                        Some((h, key)) => println!("    ({:.2}, {:.2}) ray y {:.2}, top {h:.2} (L{} y{}){}", at.x, at.z, at.y, key.level, key.coord.y, if at.y < h { "  <- ray is under the surface" } else { "" }),
                        None => println!("    ({:.2}, {:.2}) ray y {:.2}, nothing under it", at.x, at.z, at.y),
                    }
                }
                println!("  leak {shown_leaks} at {ground:?}, {} triangles within 0.9 of the ray near the ground:", around.len());
                for (along, key, t) in around.iter().take(14) {
                    let n = (t[1] - t[0]).cross(t[2] - t[0]).normalize_or_zero();
                    println!("    at {along:+.1} along: L{} y{} normal ({:.0},{:.0},{:.0}) verts {:?}", key.level, key.coord.y, n.x, n.y, n.z, t.map(|v| (v.x, v.y, v.z)));
                }
            }
            if shown_leaks < 12 {
                shown_leaks += 1;
                let near: Vec<String> = nodes
                    .iter()
                    .filter(|key| {
                        let (min, w) = (key.min_block(), (key.width() * 16) as f32);
                        x > min.x as f32 - 40.0 && x < min.x as f32 + w + 40.0 && z > min.z as f32 - 40.0 && z < min.z as f32 + w + 40.0
                    })
                    .map(|key| format!("L{} y{}", key.level, key.coord.y))
                    .collect();
                println!("  leak at ({x:.1}, {y:.1}, {z:.1}) dir {direction:?}, {distance:.0} blocks out; nodes within 40: {near:?}");
            }
        }
    }
    println!("{leaks} of {rays} oblique rays within {radius} blocks pass through everything");
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
    // `holes` waits for everything to load, then probes for holes seen from above, and stops
    let probe = std::env::args().nth(3).is_some_and(|d| d == "holes");
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
        if probe && t > settle + 40.0 {
            let world = app.world_mut();
            let pos = world.query_filtered::<&GlobalTransform, With<SimCamera>>().single(world).translation();
            for radius in [300.0, 700.0, 1400.0, 2800.0] {
                println!("radius {radius}:");
                probe_holes(&mut app, pos, radius, 6000);
            }
            return;
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
