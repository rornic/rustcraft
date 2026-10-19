// Micro-benchmarks of the terrain pipeline: one chunk at full detail in each kind of ground,
// the terrain samples LOD and chunk generation lean on, and LOD node builds by level. After
// criterion's own report it prints p50 and p99 over many distinct chunks of each kind.
//   cargo bench --bench terrain
use std::sync::Arc;
use std::time::Instant;

use bevy::math::I64Vec3;
use std::hint::black_box;
use criterion::{criterion_group, BatchSize, Criterion};
use rustcraft::chunks::chunk::{ChunkCoordinate, CHUNK_SIZE};
use rustcraft::chunks::generate::generator::{generate_chunk, generate_lod_mesh};
use rustcraft::chunks::generate::noise::{NoiseGenerator, WORLD_HALF_SIZE};
use rustcraft::chunks::generate::region_store::RegionStore;
use rustcraft::chunks::generate::WATER_LEVEL;
use rustcraft::chunks::lod::LodKey;

const SEED: u32 = 1;
// Distinct chunks of each kind measured for the percentiles.
const CHUNKS_PER_KIND: usize = 128;

#[derive(Copy, Clone, Debug)]
enum Kind {
    Plains,
    Mountains,
    Coast,
}

const KINDS: [Kind; 3] = [Kind::Plains, Kind::Mountains, Kind::Coast];

fn terrain() -> Arc<NoiseGenerator> {
    Arc::new(NoiseGenerator::new(SEED))
}

fn classify(terrain: &NoiseGenerator, x: i64, z: i64) -> Option<(Kind, f64)> {
    let height = |dx: i64, dz: i64| terrain.sample((x + dx) as f64, (z + dz) as f64).height;
    let h = height(0, 0);
    let above = h - WATER_LEVEL as f64;
    let slope = (height(8, 0) - height(-8, 0)).abs() + (height(0, 8) - height(0, -8)).abs();
    if above > 200.0 {
        Some((Kind::Mountains, h))
    } else if (6.0..30.0).contains(&above) && slope < 3.0 {
        Some((Kind::Plains, h))
    } else if (0.0..4.0).contains(&above) && height(24, 0) < WATER_LEVEL as f64 {
        Some((Kind::Coast, h))
    } else {
        None
    }
}

// Surface chunks of `kind`, found on a lattice over the world so they come from all over it.
fn chunks_of(terrain: &NoiseGenerator, kind: Kind, count: usize) -> Vec<ChunkCoordinate> {
    let size = CHUNK_SIZE as i64;
    let mut found = Vec::new();
    let step = 192;
    let reach = WORLD_HALF_SIZE * 4 / 5;
    let mut z = -reach;
    while z < reach && found.len() < count {
        let mut x = -reach;
        while x < reach && found.len() < count {
            // jittered so the lattice doesn't line up with regions
            let (jx, jz) = (x + (z / 7).rem_euclid(97), z + (x / 5).rem_euclid(89));
            if let Some((k, h)) = classify(terrain, jx, jz) {
                if std::mem::discriminant(&k) == std::mem::discriminant(&kind) {
                    found.push(ChunkCoordinate(I64Vec3::new(
                        jx.div_euclid(size),
                        (h as i64).div_euclid(size),
                        jz.div_euclid(size),
                    )));
                }
            }
            x += step;
        }
        z += step;
    }
    assert!(!found.is_empty(), "no {kind:?} chunks found");
    found
}

fn chunk_generation(c: &mut Criterion) {
    let terrain = terrain();
    let mut group = c.benchmark_group("chunk_gen");
    for kind in KINDS {
        let chunks = chunks_of(&terrain, kind, 32);
        let mut next = 0;
        // a store of its own each time: the cost of the first chunk in an area, regions and all
        group.bench_function(format!("{kind:?}/cold"), |b| {
            b.iter_batched(
                || {
                    next = (next + 1) % chunks.len();
                    (Arc::new(RegionStore::new(terrain.clone())), chunks[next])
                },
                |(store, chunk)| black_box(generate_chunk(store, chunk)),
                BatchSize::PerIteration,
            )
        });
    }
    group.finish();
}

fn terrain_samples(c: &mut Criterion) {
    let terrain = terrain();
    let mut group = c.benchmark_group("terrain_sample");
    // a cheap scramble of coordinates, so successive samples don't share cache
    let at = |i: u64| {
        let h = i.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let range = WORLD_HALF_SIZE as f64 * 1.6;
        (
            (h >> 40) as f64 / (1u64 << 24) as f64 * range - range / 2.0,
            (h & 0xff_ffff) as f64 / (1u64 << 24) as f64 * range - range / 2.0,
        )
    };
    let mut i = 0;
    group.bench_function("height", |b| {
        b.iter(|| {
            i += 1;
            let (x, z) = at(i);
            black_box(terrain.sample(x, z).height)
        })
    });
    group.bench_function("biome", |b| {
        b.iter(|| {
            i += 1;
            let (x, z) = at(i);
            black_box(terrain.sample(x, z).biome)
        })
    });
    group.bench_function("climate", |b| {
        b.iter(|| {
            i += 1;
            let (x, z) = at(i);
            black_box(terrain.climate(x, z))
        })
    });
    group.bench_function("coarse_cell_64", |b| {
        b.iter(|| {
            i += 1;
            let (x, z) = at(i);
            black_box(terrain.sample_for_cell(x, z, 64.0).height)
        })
    });
    group.finish();
}

fn lod_nodes(c: &mut Criterion) {
    let terrain = terrain();
    let (sx, sz, sh) = terrain.find_spawn();
    let spawn = ChunkCoordinate::containing(I64Vec3::new(sx, sh as i64, sz));
    let store = Arc::new(RegionStore::new(terrain));
    let mut group = c.benchmark_group("lod_node");
    for level in [1u8, 3, 5, 7] {
        let centre = LodKey::containing(spawn, level).coord;
        let keys: Vec<LodKey> = (-2..2)
            .flat_map(|x| (-2..2).map(move |z| (x, z)))
            .flat_map(|(x, z)| {
                (-1..=1).map(move |y| LodKey {
                    level,
                    coord: centre + I64Vec3::new(x, y, z),
                })
            })
            .collect();
        for &key in &keys {
            generate_lod_mesh(store.clone(), key);
        }
        let mut next = 0;
        group.bench_function(format!("level_{level}"), |b| {
            b.iter(|| {
                next = (next + 1) % keys.len();
                black_box(generate_lod_mesh(store.clone(), keys[next]))
            })
        });
    }
    group.finish();
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    sorted[((sorted.len() - 1) as f64 * p).round() as usize]
}

// Each distinct chunk once, with the regions under it already worked out: p50 and p99 of the
// chunk itself, then with a fresh store for each, regions and all.
fn report_percentiles() {
    let terrain = terrain();
    println!("\nchunk generation over {CHUNKS_PER_KIND} chunks of each kind, ms");
    for kind in KINDS {
        let chunks = chunks_of(&terrain, kind, CHUNKS_PER_KIND);
        let warm_store = Arc::new(RegionStore::new(terrain.clone()).with_resident_cap(100_000));
        for c in &chunks {
            let size = CHUNK_SIZE as i64;
            warm_store.sampler().height(c.0.x * size, c.0.z * size);
        }
        for (label, shared) in [("warm", true), ("cold", false)] {
            let mut times: Vec<f64> = chunks
                .iter()
                .map(|&c| {
                    let store = if shared {
                        warm_store.clone()
                    } else {
                        Arc::new(RegionStore::new(terrain.clone()))
                    };
                    let start = Instant::now();
                    black_box(generate_chunk(store, c));
                    start.elapsed().as_secs_f64() * 1000.0
                })
                .collect();
            times.sort_by(f64::total_cmp);
            println!(
                "{:>10} {label}: p50 {:8.3}  p99 {:8.3}  ({} chunks)",
                format!("{kind:?}"),
                percentile(&times, 0.5),
                percentile(&times, 0.99),
                times.len()
            );
        }
    }
}

criterion_group!(benches, chunk_generation, terrain_samples, lod_nodes);

fn main() {
    benches();
    Criterion::default().configure_from_args().final_summary();
    report_percentiles();
}
