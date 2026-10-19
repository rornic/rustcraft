// Renders the world's terrain heights to a shaded PNG, so terrain
// changes can be judged across the whole world in seconds instead of by flying around.
// Run with:
//   cargo run --release --example world_map -- --seed 42
// Zoom in on an area with a smaller --scale (blocks per pixel):
//   cargo run --release --example world_map -- --seed 42 --center 3000,-2000 --scale 2
// Colour lowlands by biome, with grass tinted by climate as it's drawn in game:
//   cargo run --release --example world_map -- --seed 42 --biomes
// Every run also writes <out>-water.png and <out>-biome.png beside the height map.

use std::env;

use bevy::color::{ColorToComponents, LinearRgba, Srgba};
use bevy::math::DVec3;

use rustcraft::chunks::generate::biome::{grass_colour, leaf_colour, Biomes, Climate};
use rustcraft::chunks::generate::generator::FREEZE_TEMPERATURE;
use rustcraft::chunks::generate::noise::{NoiseGenerator, WORLD_SIZE};
use rustcraft::chunks::generate::terrain_source::{TerrainSample, TerrainSource, WaterKind};
use rustcraft::chunks::generate::WATER_LEVEL;

struct Args {
    seed: u32,
    center: (f64, f64),
    scale: f64,
    pixels: u32,
    out: String,
    biomes: bool,
}

fn parse_args() -> Args {
    let mut args = Args {
        seed: 42,
        center: (0.0, 0.0),
        scale: (WORLD_SIZE as f64 * 1.1 / 2048.0).ceil(),
        pixels: 2048,
        out: "world_map.png".to_string(),
        biomes: false,
    };

    let mut argv = env::args().skip(1);
    while let Some(flag) = argv.next() {
        if flag == "--biomes" {
            args.biomes = true;
            continue;
        }
        let value = argv
            .next()
            .unwrap_or_else(|| panic!("missing value for {flag}"));
        match flag.as_str() {
            "--seed" => args.seed = value.parse().expect("--seed must be a u32"),
            "--center" => {
                let (x, z) = value.split_once(',').expect("--center must be x,z");
                args.center = (x.parse().expect("invalid x"), z.parse().expect("invalid z"));
            }
            "--scale" => args.scale = value.parse().expect("--scale must be a number"),
            "--pixels" => args.pixels = value.parse().expect("--pixels must be a u32"),
            "--out" => args.out = value,
            other => panic!("unknown flag {other}"),
        }
    }
    args
}

fn sample_map(noise: &dyn TerrainSource, args: &Args) -> Vec<(TerrainSample, Climate)> {
    let size = args.pixels as usize;
    let half = args.pixels as f64 / 2.0;
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let rows_per_thread = size.div_ceil(threads);

    let mut samples = vec![(TerrainSample::default(), Climate::default()); size * size];
    std::thread::scope(|scope| {
        for (chunk_index, rows) in samples.chunks_mut(rows_per_thread * size).enumerate() {
            scope.spawn(move || {
                for (i, sample) in rows.iter_mut().enumerate() {
                    let row = chunk_index * rows_per_thread + i / size;
                    let col = i % size;
                    let x = args.center.0 + (col as f64 - half) * args.scale;
                    let z = args.center.1 + (row as f64 - half) * args.scale;
                    *sample = (noise.sample(x, z), noise.climate(x, z));
                }
            });
        }
    });
    samples
}

fn lerp_rgb(a: [f64; 3], b: [f64; 3], t: f64) -> [f64; 3] {
    DVec3::from(a)
        .lerp(DVec3::from(b), t.clamp(0.0, 1.0))
        .into()
}

// Height above (or below) sea level to a map colour.
fn height_colour(above_sea: f64) -> [f64; 3] {
    const STOPS: [(f64, [f64; 3]); 10] = [
        (-380.0, [8.0, 18.0, 55.0]),
        (-200.0, [14.0, 34.0, 90.0]),
        (-70.0, [25.0, 70.0, 140.0]),
        (-30.0, [55.0, 120.0, 180.0]),
        (0.0, [100.0, 170.0, 205.0]),
        (0.01, [210.0, 200.0, 140.0]),
        (2.0, [90.0, 150.0, 70.0]),
        (40.0, [60.0, 115.0, 50.0]),
        (140.0, [125.0, 110.0, 95.0]),
        (250.0, [245.0, 245.0, 250.0]),
    ];
    if above_sea <= STOPS[0].0 {
        return STOPS[0].1;
    }
    for pair in STOPS.windows(2) {
        let ((h0, c0), (h1, c1)) = (pair[0], pair[1]);
        if above_sea < h1 {
            return lerp_rgb(c0, c1, (above_sea - h0) / (h1 - h0));
        }
    }
    STOPS[STOPS.len() - 1].1
}

const SAND: [f64; 3] = [222.0, 196.0, 132.0];
const SEA_ICE: [f64; 3] = [190.0, 222.0, 240.0];
const GLACIER: [f64; 3] = [226.0, 238.0, 248.0];

// Grass in this climate under trees this dense, as an sRGB map colour.
fn grass_map_colour(climate: Climate, trees: f64) -> [f64; 3] {
    let canopy = (trees * 1.6).min(1.0) as f32;
    let linear = grass_colour(climate).lerp(leaf_colour(climate), canopy);
    let srgb = Srgba::from(LinearRgba::rgb(linear.x, linear.y, linear.z)).to_vec3();
    (srgb.as_dvec3() * 255.0).into()
}

fn render(samples: &[(TerrainSample, Climate)], biomes: &Biomes, args: &Args) -> image::RgbImage {
    let size = args.pixels;
    let at = |x: u32, z: u32| samples[(z.min(size - 1) * size + x.min(size - 1)) as usize];
    let height_at = |x: u32, z: u32| at(x, z).0.height;

    let mut img = image::RgbImage::new(size, size);
    for z in 0..size {
        for x in 0..size {
            let height = height_at(x, z);
            let above_sea = height - WATER_LEVEL as f64;
            let mut colour = height_colour(above_sea);
            // up to where the height colours turn to rock
            if args.biomes && (2.0..140.0).contains(&above_sea) {
                let (sample, climate) = at(x, z);
                let ground = match biomes.name(sample.biome) {
                    Some("desert") => SAND,
                    Some("glacier") => GLACIER,
                    _ => grass_map_colour(climate, sample.trees),
                };
                let rock = ((above_sea - 40.0) / 100.0).clamp(0.0, 1.0);
                colour = lerp_rgb(ground, colour, rock);
            }

            if args.biomes && above_sea <= 0.0 && above_sea > -60.0 {
                let ice = (1.0 - at(x, z).1.temperature as f64 / FREEZE_TEMPERATURE as f64)
                    .clamp(0.0, 1.0);
                if ice > 0.0 {
                    colour = lerp_rgb(colour, SEA_ICE, (ice * 4.0).min(1.0));
                }
            }

            // Light from the north west, exaggerated so gentle slopes still read at map scale.
            if above_sea > 0.0 {
                let dx =
                    (height_at(x + 1, z) - height_at(x.saturating_sub(1), z)) / (2.0 * args.scale);
                let dz =
                    (height_at(x, z + 1) - height_at(x, z.saturating_sub(1))) / (2.0 * args.scale);
                let shade = (1.0 - (dx + dz) * 3.0).clamp(0.4, 1.4);
                colour = colour.map(|c| c * shade);
            }
            img.put_pixel(x, z, image::Rgb(colour.map(|c| c.clamp(0.0, 255.0) as u8)));
        }
    }
    img
}

// The water and biome layers, one pixel per sample of the height map.
fn layers(terrain: &dyn TerrainSource, args: &Args) -> [(&'static str, image::RgbImage); 2] {
    let size = args.pixels;
    let half = size as f64 / 2.0;
    let mut water = image::RgbImage::new(size, size);
    let mut biome = image::RgbImage::new(size, size);
    for z in 0..size {
        for x in 0..size {
            let wx = args.center.0 + (x as f64 - half) * args.scale;
            let wz = args.center.1 + (z as f64 - half) * args.scale;
            let sample = terrain.sample(wx, wz);
            let w = terrain.water(wx, wz);
            let depth = (w.depth as f64 / 60.0).clamp(0.0, 1.0);
            let colour = match w.kind {
                WaterKind::None => [70, 70, 70],
                WaterKind::Ocean => [
                    (30.0 - 25.0 * depth) as u8,
                    (90.0 - 70.0 * depth) as u8,
                    200,
                ],
                WaterKind::Lake => [60, 200, 230],
                WaterKind::River => [120, 230, 255],
            };
            water.put_pixel(x, z, image::Rgb(colour));
            // a stable colour per biome id
            let id = format!("{:?}", sample.biome)
                .bytes()
                .fold(7u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
            let rgb = [
                (id >> 3) as u8 | 64,
                (id >> 11) as u8 | 64,
                (id >> 19) as u8 | 64,
            ];
            biome.put_pixel(
                x,
                z,
                image::Rgb(if sample.height < WATER_LEVEL as f64 {
                    [20, 40, 90]
                } else {
                    rgb
                }),
            );
        }
    }
    [("water", water), ("biome", biome)]
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    sorted[((sorted.len() - 1) as f64 * p) as usize]
}

fn print_stats(samples: &[(TerrainSample, Climate)], biomes: &Biomes) {
    let samples: Vec<TerrainSample> = samples.iter().map(|s| s.0).collect();
    let land = samples
        .iter()
        .filter(|s| s.height >= WATER_LEVEL as f64)
        .count();
    println!("land: {:.1}%", 100.0 * land as f64 / samples.len() as f64);
    for name in ["desert", "forest", "foothills", "glacier"] {
        let count = samples
            .iter()
            .filter(|s| s.height >= WATER_LEVEL as f64 && biomes.name(s.biome) == Some(name))
            .count();
        println!(
            "{name}: {:.1}% of land",
            100.0 * count as f64 / land.max(1) as f64
        );
    }

    let mut heights: Vec<f64> = samples.iter().map(|s| s.height).collect();
    heights.sort_by(f64::total_cmp);
    let mut continents: Vec<f64> = samples.iter().map(|s| s.continentalness).collect();
    continents.sort_by(f64::total_cmp);
    for p in [0.0, 0.05, 0.25, 0.5, 0.75, 0.95, 0.99, 1.0] {
        println!(
            "p{:<3} height={:6.1} continentalness={:6.3}",
            (p * 100.0) as u32,
            percentile(&heights, p),
            percentile(&continents, p)
        );
    }
}

fn main() {
    let args = parse_args();
    let noise = NoiseGenerator::new(args.seed);

    let start = std::time::Instant::now();
    let samples = sample_map(&noise, &args);
    println!(
        "sampled {}x{} in {:.1?}",
        args.pixels,
        args.pixels,
        start.elapsed()
    );

    print_stats(&samples, noise.biomes());
    let spawn = noise.find_spawn();
    println!("spawn: x={} z={} height={:.1}", spawn.0, spawn.1, spawn.2);

    render(&samples, noise.biomes(), &args)
        .save(&args.out)
        .expect("failed to save image");
    println!("wrote {}", args.out);
    let stem = args.out.strip_suffix(".png").unwrap_or(&args.out);
    for (layer, image) in layers(&noise, &args) {
        let path = format!("{stem}-{layer}.png");
        image.save(&path).expect("failed to save image");
        println!("wrote {path}");
    }
}
