// Times each stage of building terrain around spawn, one thread, for comparing before and after a
// change and for profiling. Run with:
//   cargo run --release --example bench -- [seed, default 1] [lod, to time only LOD meshes]
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bevy::math::I64Vec3;
use rustcraft::chunks::chunk::{ChunkCoordinate, ChunkData, CHUNK_SIZE};
use rustcraft::chunks::generate::generator::{
    generate_chunk, generate_chunk_mesh, generate_lod_mesh,
};
use rustcraft::chunks::generate::region_store::RegionStore;
use rustcraft::chunks::generate::terrain_source::TerrainSource;
use rustcraft::chunks::lod::LodKey;
use rustcraft::worldmap::{WorldConfig, WorldTerrain};

// Chunks across the area generated, and how many layers of them up from below the surface.
const ACROSS: i64 = 20;
const LAYERS: i64 = 8;
// LOD nodes meshed at each level, in a square round spawn.
const NODES_ACROSS: i64 = 4;
const LEVELS: std::ops::RangeInclusive<u8> = 1..=8;

fn timed<T>(label: &str, f: impl FnOnce() -> T) -> T {
    let start = Instant::now();
    let result = f();
    println!("{label:>16}: {:>8.1?}", start.elapsed());
    result
}

fn main() {
    let seed = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    let map = rustcraft::worldmap::generate(seed, WorldConfig::default());
    let noise: Arc<dyn TerrainSource> = Arc::new(WorldTerrain::new(Arc::new(map)));
    let (sx, sz, sh) = noise.find_spawn();
    let spawn = ChunkCoordinate::containing(I64Vec3::new(sx, sh as i64, sz)).0;
    let regions = Arc::new(RegionStore::new(noise.clone()));
    if std::env::args().nth(2).as_deref() != Some("lod") {
        chunks(&regions, spawn);
    }
    lods(&regions, spawn);
}

fn chunks(regions: &Arc<RegionStore>, spawn: I64Vec3) {
    let coords: Vec<ChunkCoordinate> = (-ACROSS / 2..ACROSS / 2)
        .flat_map(|x| {
            (-LAYERS / 2..LAYERS / 2).flat_map(move |y| {
                (-ACROSS / 2..ACROSS / 2)
                    .map(move |z| ChunkCoordinate(spawn + I64Vec3::new(x, y, z)))
            })
        })
        .collect();
    timed("regions", || {
        for c in &coords {
            let size = CHUNK_SIZE as i64;
            regions.sampler().height(c.0.x * size, c.0.z * size);
        }
    });
    let data: HashMap<ChunkCoordinate, Arc<ChunkData>> = timed("generate chunks", || {
        coords
            .iter()
            .map(|&c| (c, Arc::new(generate_chunk(regions.clone(), c))))
            .collect()
    });
    // the inner chunks, which have all their neighbours
    let meshed: Vec<ChunkCoordinate> = coords
        .iter()
        .copied()
        .filter(|c| c.neighbors_26().iter().all(|n| data.contains_key(n)))
        .collect();
    let mut best = Duration::MAX;
    let mut vertices = 0;
    for _ in 0..3 {
        let start = Instant::now();
        vertices = 0;
        for &c in &meshed {
            let neighbours = c
                .neighbors_26()
                .iter()
                .map(|n| data.get(n).cloned())
                .collect();
            let meshes =
                generate_chunk_mesh(data[&c].clone(), neighbours, c, regions.terrain(), false);
            vertices += meshes.opaque.count_vertices();
        }
        best = best.min(start.elapsed());
    }
    println!(
        "{:>16}: {best:>8.1?} best of 3, {} chunks, {vertices} vertices",
        "mesh chunks",
        meshed.len()
    );
}

// Best of 3 at each level, after a first pass that fills the regions and column grids, which
// nodes share with those near them.
fn lods(regions: &Arc<RegionStore>, spawn: I64Vec3) {
    let keys = |level: u8| {
        let centre = LodKey::containing(ChunkCoordinate(spawn), level).coord;
        (-NODES_ACROSS / 2..NODES_ACROSS / 2).flat_map(move |x| {
            (-NODES_ACROSS / 2..NODES_ACROSS / 2).flat_map(move |z| {
                (-1..=1).map(move |y| LodKey {
                    level,
                    coord: centre + I64Vec3::new(x, y, z),
                })
            })
        })
    };
    let mesh = |level: u8| {
        keys(level)
            .filter_map(|key| generate_lod_mesh(regions.clone(), key))
            .map(|meshes| meshes.opaque.count_vertices())
            .sum::<usize>()
    };
    for level in LEVELS {
        mesh(level);
    }
    let (mut total, mut vertices) = (Duration::ZERO, 0);
    for level in LEVELS {
        let mut best = Duration::MAX;
        for _ in 0..3 {
            let start = Instant::now();
            vertices += mesh(level);
            best = best.min(start.elapsed());
        }
        total += best;
        println!("{:>16}: {best:>8.1?}", format!("lod level {level}"));
    }
    println!(
        "{:>16}: {total:>8.1?}, {} vertices",
        "lod total",
        vertices / 3
    );
}
