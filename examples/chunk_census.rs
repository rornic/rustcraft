// Sorts the full-detail chunks loaded round spawn by what they hold, and times meshing each kind.
//   cargo run --release --example chunk_census -- [seed, default 1] [full detail distance] [chunks below spawn]
use std::sync::Arc;
use std::time::Instant;

use bevy::math::I64Vec3;
use rustcraft::chunks::chunk::{ChunkCoordinate, CHUNK_SIZE};
use rustcraft::chunks::generate::generator::{generate_chunk, generate_chunk_mesh};
use rustcraft::chunks::generate::noise::NoiseGenerator;
use rustcraft::chunks::generate::region_store::RegionStore;
use rustcraft::settings::Settings;

fn main() {
    let seed = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    let noise = Arc::new(NoiseGenerator::new(seed));
    let regions = Arc::new(RegionStore::new(noise.clone()));
    let (sx, sz, sh) = noise.find_spawn();
    let depth: i64 = std::env::args()
        .nth(3)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let anchor = I64Vec3::new(sx, sh as i64 + 20, sz).div_euclid(I64Vec3::splat(CHUNK_SIZE as i64))
        - I64Vec3::Y * depth;
    let mut renderer = Settings::read("assets/settings.toml").unwrap().renderer;
    if let Some(distance) = std::env::args().nth(2).and_then(|s| s.parse().ok()) {
        renderer.full_detail_distance = distance;
    }
    let layout = renderer.lod_layout();
    let r = layout.full_detail_reach() as i64 + 2;
    println!("anchor {anchor:?}, radius {r}");

    let mut data = std::collections::HashMap::new();
    let mut gen_time = [0.0f64; 3]; // air, all opaque, the rest
    let t = Instant::now();
    for x in -r..=r {
        for y in -r..=r {
            for z in -r..=r {
                let o = I64Vec3::new(x, y, z);
                if o.as_vec3().length() <= r as f32 {
                    let c = ChunkCoordinate(anchor + o);
                    let t = Instant::now();
                    let d = generate_chunk(regions.clone(), c);
                    gen_time[if d.empty() {
                        0
                    } else if d.opaque() {
                        1
                    } else {
                        2
                    }] += t.elapsed().as_secs_f64();
                    data.insert(c, Arc::new(d));
                }
            }
        }
    }
    let layers = rustcraft::world::World::layers(rustcraft::chunks::generate::WORLD_HEIGHT);
    println!(
        "{} of them in the world's layers {layers:?}, the rest never loaded",
        data.keys().filter(|c| layers.contains(&c.0.y)).count()
    );
    println!(
        "generated {} chunks in {:?}: air {:.2}s, all opaque {:.2}s, the rest {:.2}s",
        data.len(),
        t.elapsed(),
        gen_time[0],
        gen_time[1],
        gen_time[2]
    );
    let mut sum = 0u64;
    for (c, d) in &data {
        for (local, block) in d.iter_non_air() {
            let h = (c.0.x * 73_856_093) ^ (c.0.y * 19_349_663) ^ (c.0.z * 83_492_791);
            sum = sum.wrapping_add(
                (h as u64
                    ^ ((local.x as u64) << 8 | (local.y as u64) << 16 | (local.z as u64) << 24))
                    .wrapping_mul(block as u64 + 1),
            );
        }
    }
    println!("block checksum {sum:x}");
    let t = Instant::now();
    let buried = data
        .keys()
        .filter(|c| {
            data[c].opaque()
                && c.adjacent()
                    .iter()
                    .all(|n| data.get(n).is_some_and(|n| n.opaque()))
        })
        .count();
    println!(
        "buried check over every chunk: {:?} ({buried} buried)",
        t.elapsed()
    );

    let heap: usize = data.values().map(|d| d.heap_size()).sum();
    println!("chunk data on the heap: {:.1} MB", heap as f64 / 1e6);

    let (mut air, mut opaque, mut buried, mut surface, mut skipped, mut missed) =
        (0, 0, 0, 0, 0, 0);
    let (mut buried_time, mut surface_time) = (0.0, 0.0);
    for (&c, d) in &data {
        if d.empty() {
            air += 1;
            continue;
        }
        if d.opaque() {
            opaque += 1;
        }
        if (c.0 - anchor).as_vec3().length() > (r - 2) as f32 {
            continue; // padding shell, never meshed
        }
        let boxed_in = d.opaque()
            && c.adjacent()
                .iter()
                .all(|n| data.get(n).is_some_and(|n| n.opaque()));
        let adjacent = c
            .neighbors_26()
            .iter()
            .map(|n| data.get(n).cloned())
            .collect();
        let t = Instant::now();
        let meshes = generate_chunk_mesh(d.clone(), adjacent, c, &noise, false);
        let dt = t.elapsed().as_secs_f64();
        if meshes.has_geometry() {
            surface += 1;
            surface_time += dt;
            assert!(!boxed_in, "{c:?} was skipped but has faces");
        } else {
            if boxed_in {
                skipped += 1
            } else {
                missed += 1
            }
            buried += 1;
            buried_time += dt;
        }
    }
    println!("air {air}, all opaque {opaque}");
    println!("meshed with geometry {surface} ({surface_time:.2}s), meshed to nothing {buried} ({buried_time:.2}s), of which boxed in by opaque chunks {skipped}, not {missed}");
}
