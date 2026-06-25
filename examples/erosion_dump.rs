// Visual debug tool for Milestone 1 of hydraulic erosion: dumps before/after/diff
// heightmap PNGs so erosion bugs are spotted in an image, not by flying through
// voxel chunks in-game. Run with:
//   cargo run --example erosion_dump -- --seed 42 --origin 0,0 --size 384 --out-dir erosion_debug

use std::env;

use bevy::math::I64Vec2;
use rustcraft::chunks::generate::erosion::{erode, ErosionParams, HeightGrid};
use rustcraft::chunks::generate::noise::NoiseGenerator;

// Matches the `world_height` value passed to `generate_chunk` elsewhere (see
// `World::height` / `bench_generate_and_mesh_chunks`).
const WORLD_HEIGHT: u64 = 256;

struct Args {
    seed: u32,
    origin: I64Vec2,
    size: u32,
    out_dir: String,
}

fn parse_args() -> Args {
    let mut seed = 42u32;
    let mut origin = I64Vec2::new(0, 0);
    let mut size = 384u32;
    let mut out_dir = "erosion_debug".to_string();

    let mut args = env::args().skip(1);
    while let Some(flag) = args.next() {
        let value = args
            .next()
            .unwrap_or_else(|| panic!("missing value for {flag}"));
        match flag.as_str() {
            "--seed" => seed = value.parse().expect("--seed must be a u32"),
            "--origin" => {
                let (x, z) = value.split_once(',').expect("--origin must be x,z");
                origin = I64Vec2::new(
                    x.parse().expect("invalid origin x"),
                    z.parse().expect("invalid origin z"),
                );
            }
            "--size" => size = value.parse().expect("--size must be a u32"),
            "--out-dir" => out_dir = value,
            other => panic!("unknown flag {other}"),
        }
    }

    Args { seed, origin, size, out_dir }
}

fn save_grayscale(path: &str, width: u32, height: u32, data: &[f32]) {
    let min = data.iter().cloned().fold(f32::INFINITY, f32::min);
    let max = data.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    let range = (max - min).max(f32::EPSILON);

    let mut img = image::GrayImage::new(width, height);
    for (i, value) in data.iter().enumerate() {
        let normalized = (((value - min) / range) * 255.0) as u8;
        img.put_pixel(i as u32 % width, i as u32 / width, image::Luma([normalized]));
    }
    img.save(path).expect("failed to save image");
}

// Blue = deposited, red = eroded - the single most useful image for spotting bugs:
// erosion concentrated in straight lines indicates a gradient sign error, uniform
// haze instead of dendritic channels indicates inertia/sediment_capacity_factor is off.
fn save_diff(path: &str, width: u32, height: u32, before: &[f32], after: &[f32]) {
    let diffs: Vec<f32> = before.iter().zip(after.iter()).map(|(b, a)| a - b).collect();
    let max_abs = diffs
        .iter()
        .cloned()
        .fold(0.0f32, |m, v| m.max(v.abs()))
        .max(f32::EPSILON);

    let mut img = image::RgbImage::new(width, height);
    for (i, diff) in diffs.iter().enumerate() {
        let normalized = (diff / max_abs).clamp(-1.0, 1.0);
        let pixel = if normalized >= 0.0 {
            [0, 0, (normalized * 255.0) as u8]
        } else {
            [(-normalized * 255.0) as u8, 0, 0]
        };
        img.put_pixel(i as u32 % width, i as u32 / width, image::Rgb(pixel));
    }
    img.save(path).expect("failed to save image");
}

fn stats(label: &str, data: &[f32]) {
    let min = data.iter().cloned().fold(f32::INFINITY, f32::min);
    let max = data.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    let mean = data.iter().sum::<f32>() / data.len() as f32;
    println!("{label}: min={min:.2} max={max:.2} mean={mean:.2}");
}

fn main() {
    let args = parse_args();
    std::fs::create_dir_all(&args.out_dir).expect("failed to create output directory");

    let noise = NoiseGenerator::new(args.seed);
    let mut grid = HeightGrid::from_noise(&noise, args.origin, args.size, args.size, WORLD_HEIGHT);
    let before = grid.data().to_vec();
    stats("before", &before);

    erode(&mut grid, args.seed, args.origin, &ErosionParams::default());

    let after = grid.data().to_vec();
    stats("after", &after);

    // Should nearly balance - a large imbalance indicates a mass-conservation bug.
    let eroded: f32 = before.iter().zip(after.iter()).map(|(b, a)| (b - a).max(0.0)).sum();
    let deposited: f32 = before.iter().zip(after.iter()).map(|(b, a)| (a - b).max(0.0)).sum();
    println!("total eroded={eroded:.2} total deposited={deposited:.2}");

    save_grayscale(&format!("{}/before.png", args.out_dir), args.size, args.size, &before);
    save_grayscale(&format!("{}/after.png", args.out_dir), args.size, args.size, &after);
    save_diff(&format!("{}/diff.png", args.out_dir), args.size, args.size, &before, &after);

    println!("wrote before.png, after.png, diff.png to {}", args.out_dir);
}
