// Sorts the full-detail chunks loaded round spawn by what they hold, and times meshing each kind.
//   cargo run --release --example chunk_census -- [seed, default 1]
use std::sync::Arc;
use std::time::Instant;

use bevy::math::I64Vec3;
use rustcraft::chunks::chunk::{ChunkCoordinate, CHUNK_SIZE};
use rustcraft::chunks::generate::generator::{generate_chunk, generate_chunk_mesh};
use rustcraft::chunks::generate::noise::NoiseGenerator;
use rustcraft::chunks::generate::region_store::RegionStore;
use rustcraft::settings::Settings;

fn main() {
    let seed = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(1);
    let noise = Arc::new(NoiseGenerator::new(seed));
    let regions = Arc::new(RegionStore::new(noise.clone()));
    let (sx, sz, sh) = noise.find_spawn();
    let anchor = I64Vec3::new(sx, sh as i64 + 20, sz).div_euclid(I64Vec3::splat(CHUNK_SIZE as i64));
    let layout = Settings::read("assets/settings.toml").unwrap().renderer.lod_layout();
    let r = layout.full_detail_reach() as i64 + 2;
    println!("anchor {anchor:?}, radius {r}");

    let mut data = std::collections::HashMap::new();
    let t = Instant::now();
    for x in -r..=r {
        for y in -r..=r {
            for z in -r..=r {
                let o = I64Vec3::new(x, y, z);
                if o.as_vec3().length() <= r as f32 {
                    let c = ChunkCoordinate(anchor + o);
                    data.insert(c, Arc::new(generate_chunk(regions.clone(), c)));
                }
            }
        }
    }
    println!("generated {} chunks in {:?}", data.len(), t.elapsed());

    let heap: usize = data.values().map(|d| d.heap_size()).sum();
    println!("chunk data on the heap: {:.1} MB", heap as f64 / 1e6);

    let (mut air, mut opaque, mut buried, mut surface, mut skipped, mut missed) = (0, 0, 0, 0, 0, 0);
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
        let boxed_in = d.opaque() && c.adjacent().iter().all(|n| data.get(n).is_some_and(|n| n.opaque()));
        let adjacent = c.neighbors_26().iter().map(|n| data.get(n).cloned()).collect();
        let t = Instant::now();
        let meshes = generate_chunk_mesh(d.clone(), adjacent, c, &noise, false);
        let dt = t.elapsed().as_secs_f64();
        if meshes.has_geometry() {
            surface += 1;
            surface_time += dt;
            assert!(!boxed_in, "{c:?} was skipped but has faces");
        } else {
            if boxed_in { skipped += 1 } else { missed += 1 }
            buried += 1;
            buried_time += dt;
        }
    }
    println!("air {air}, all opaque {opaque}");
    println!("meshed with geometry {surface} ({surface_time:.2}s), meshed to nothing {buried} ({buried_time:.2}s), of which boxed in by opaque chunks {skipped}, not {missed}");
}
