// Renders the world's large scale shape (before erosion) to a shaded PNG, so terrain
// changes can be judged across the whole world in seconds instead of by flying around.
// Run with:
//   cargo run --release --example world_map -- --seed 42
// Zoom in on an area with a smaller --scale (blocks per pixel):
//   cargo run --release --example world_map -- --seed 42 --center 3000,-2000 --scale 2

use std::env;

use rustcraft::chunks::generate::generator::WATER_LEVEL;
use rustcraft::chunks::generate::noise::{NoiseGenerator, TerrainSample, WORLD_SIZE};

struct Args {
    seed: u32,
    center: (f64, f64),
    scale: f64,
    pixels: u32,
    out: String,
}

fn parse_args() -> Args {
    let mut args = Args {
        seed: 42,
        center: (0.0, 0.0),
        scale: (WORLD_SIZE as f64 * 1.1 / 2048.0).ceil(),
        pixels: 2048,
        out: "world_map.png".to_string(),
    };

    let mut argv = env::args().skip(1);
    while let Some(flag) = argv.next() {
        let value = argv.next().unwrap_or_else(|| panic!("missing value for {flag}"));
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

fn sample_map(noise: &NoiseGenerator, args: &Args) -> Vec<TerrainSample> {
    let size = args.pixels as usize;
    let half = args.pixels as f64 / 2.0;
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let rows_per_thread = size.div_ceil(threads);

    let mut samples = vec![
        TerrainSample { continentalness: 0.0, ruggedness: 0.0, height: 0.0 };
        size * size
    ];
    std::thread::scope(|scope| {
        for (chunk_index, rows) in samples.chunks_mut(rows_per_thread * size).enumerate() {
            scope.spawn(move || {
                for (i, sample) in rows.iter_mut().enumerate() {
                    let row = chunk_index * rows_per_thread + i / size;
                    let col = i % size;
                    let x = args.center.0 + (col as f64 - half) * args.scale;
                    let z = args.center.1 + (row as f64 - half) * args.scale;
                    *sample = noise.sample(x, z);
                }
            });
        }
    });
    samples
}

fn lerp_rgb(a: [f64; 3], b: [f64; 3], t: f64) -> [f64; 3] {
    let t = t.clamp(0.0, 1.0);
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

// Height above (or below) sea level to a map colour.
fn height_colour(above_sea: f64) -> [f64; 3] {
    const STOPS: [(f64, [f64; 3]); 8] = [
        (-48.0, [12.0, 30.0, 80.0]),
        (-12.0, [30.0, 80.0, 150.0]),
        (0.0, [90.0, 160.0, 200.0]),
        (0.01, [210.0, 200.0, 140.0]),
        (4.0, [90.0, 150.0, 70.0]),
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

fn render(samples: &[TerrainSample], args: &Args) -> image::RgbImage {
    let size = args.pixels;
    let height_at = |x: u32, z: u32| samples[(z.min(size - 1) * size + x.min(size - 1)) as usize].height;

    let mut img = image::RgbImage::new(size, size);
    for z in 0..size {
        for x in 0..size {
            let height = height_at(x, z);
            let above_sea = height - WATER_LEVEL as f64;
            let mut colour = height_colour(above_sea);

            // Light from the north west, exaggerated so gentle slopes still read at map scale.
            if above_sea > 0.0 {
                let dx = (height_at(x + 1, z) - height_at(x.saturating_sub(1), z)) / (2.0 * args.scale);
                let dz = (height_at(x, z + 1) - height_at(x, z.saturating_sub(1))) / (2.0 * args.scale);
                let shade = (1.0 - (dx + dz) * 3.0).clamp(0.4, 1.4);
                colour = colour.map(|c| c * shade);
            }
            img.put_pixel(x, z, image::Rgb(colour.map(|c| c.clamp(0.0, 255.0) as u8)));
        }
    }
    img
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    sorted[((sorted.len() - 1) as f64 * p) as usize]
}

fn print_stats(samples: &[TerrainSample]) {
    let land = samples.iter().filter(|s| s.height >= WATER_LEVEL as f64).count();
    println!("land: {:.1}%", 100.0 * land as f64 / samples.len() as f64);

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
    println!("sampled {}x{} in {:.1?}", args.pixels, args.pixels, start.elapsed());

    print_stats(&samples);
    let spawn = noise.find_spawn();
    println!("spawn: x={} z={} height={:.1}", spawn.0, spawn.1, spawn.2);

    render(&samples, &args).save(&args.out).expect("failed to save image");
    println!("wrote {}", args.out);
}
