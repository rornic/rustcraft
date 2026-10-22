// Renders the world's terrain heights to a shaded PNG, so terrain
// changes can be judged across the whole world in seconds instead of by flying around.
// Run with:
//   cargo run --release --example world_map -- --seed 42
// Zoom in on an area with a smaller --scale (blocks per pixel):
//   cargo run --release --example world_map -- --seed 42 --center 3000,-2000 --scale 2
// Colour lowlands by biome, with grass tinted by climate as it's drawn in game:
//   cargo run --release --example world_map -- --seed 42 --biomes
// A smaller or finer world (--size and --cell in blocks, defaults from WorldConfig):
//   cargo run --release --example world_map -- --seed 42 --size 16384 --cell 16
// Every run also writes <out>-water.png and <out>-biome.png beside the height map.

use std::env;

use bevy::color::{ColorToComponents, LinearRgba, Srgba};
use bevy::math::DVec3;

use ochre_base::biome::{grass_colour, leaf_colour, Biomes, Climate};
use ochre_base::terrain_source::{TerrainSample, TerrainSource, WaterKind};
use ochre_base::WATER_LEVEL;
use ochre_terrain::generate::generator::FREEZE_TEMPERATURE;
use ochre_world::worldmap::{WorldConfig, WorldTerrain};

struct Args {
    seed: u32,
    center: (f64, f64),
    scale: f64,
    pixels: u32,
    out: String,
    biomes: bool,
    size: i64,
    cell: i64,
}

fn parse_args() -> Args {
    let mut args = Args {
        seed: 42,
        center: (0.0, 0.0),
        scale: (WorldConfig::default().size as f64 * 1.1 / 2048.0).ceil(),
        pixels: 2048,
        out: "world_map.png".to_string(),
        biomes: false,
        size: WorldConfig::default().size,
        cell: WorldConfig::default().map_cell,
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
            "--size" => args.size = value.parse().expect("--size must be a number"),
            "--cell" => args.cell = value.parse().expect("--cell must be a number"),
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

// Lakes and rivers over the height map, rivers drawn a pixel wide at least so they show at any scale.
fn overlay_water(image: &mut image::RgbImage, map: &ochre_world::worldmap::WorldMap, args: &Args) {
    use ochre_world::worldmap::WaterCell;
    let half = args.pixels as f64 / 2.0;
    for cz in 0..map.dim() {
        for cx in 0..map.dim() {
            let i = map.index(cx, cz);
            let colour = match WaterCell::from_u8(map.water_kind[i]) {
                WaterCell::Lake => [50, 130, 230],
                WaterCell::River => [90, 190, 255],
                _ => continue,
            };
            let to_pixel = |cx: usize, cz: usize| {
                let (wx, wz) = map.cell_centre(cx, cz);
                (
                    (wx - args.center.0) / args.scale + half,
                    (wz - args.center.1) / args.scale + half,
                )
            };
            // rivers run to the cell they drain into, lakes and sea cells are single patches
            let (from, to) = match map.down[i] {
                255 => (to_pixel(cx, cz), to_pixel(cx, cz)),
                d => {
                    let (dx, dz) = ochre_world::worldmap::NEIGHBOURS[d as usize];
                    (
                        to_pixel(cx, cz),
                        to_pixel((cx as i64 + dx) as usize, (cz as i64 + dz) as usize),
                    )
                }
            };
            let (from, to) = if colour == [90, 190, 255] {
                (from, to)
            } else {
                (from, from)
            };
            let wide = if map.water_kind[i] == WaterCell::Lake as u8 {
                map.config.map_cell as f32
            } else {
                map.river_width[i]
            };
            let radius = (wide as f64 / args.scale / 2.0).max(0.0);
            let steps = ((to.0 - from.0).abs().max((to.1 - from.1).abs()) * 2.0)
                .ceil()
                .max(1.0) as i64;
            let reach = radius.ceil() as i64;
            for step in 0..=steps {
                let t = step as f64 / steps as f64;
                let (px, pz) = (from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t);
                for dz in -reach..=reach {
                    for dx in -reach..=reach {
                        let (x, z) = (px.round() as i64 + dx, pz.round() as i64 + dz);
                        if (dx * dx + dz * dz) as f64 <= radius * radius + 0.5
                            && (0..args.pixels as i64).contains(&x)
                            && (0..args.pixels as i64).contains(&z)
                        {
                            image.put_pixel(x as u32, z as u32, image::Rgb(colour));
                        }
                    }
                }
            }
        }
    }
}

// Marks each feature: passes white, fords cyan, bridge sites orange, spawn regions green rings,
// iron brown, coal black, clay terracotta, salt pink, caves blue, hotspots red.
fn draw_labels(image: &mut image::RgbImage, map: &ochre_world::worldmap::WorldMap, args: &Args) {
    use ochre_world::worldmap::FeatureKind::*;
    let half = args.pixels as f64 / 2.0;
    // marks stay a readable size however far the map is zoomed out
    let size = (3.0f64).max(120.0 / args.scale);
    for f in &map.features {
        let (colour, ring) = match f.kind {
            Pass => ([255, 255, 255], false),
            Ford => ([0, 230, 255], false),
            BridgeSite => ([255, 140, 0], false),
            Spawn => ([40, 255, 60], true),
            Iron => ([140, 80, 40], false),
            Coal => ([10, 10, 10], false),
            Clay => ([210, 110, 80], false),
            Salt => ([255, 150, 200], false),
            Cave => ([60, 80, 255], false),
            Hotspot => ([255, 0, 0], false),
        };
        let cx = (f.x as f64 - args.center.0) / args.scale + half;
        let cz = (f.z as f64 - args.center.1) / args.scale + half;
        let radius = if ring { size * 2.0 } else { size };
        let r = radius.ceil() as i64;
        for dz in -r..=r {
            for dx in -r..=r {
                let d = ((dx * dx + dz * dz) as f64).sqrt();
                let inside = if ring {
                    d <= radius && d >= radius - 2.0
                } else {
                    d <= radius
                };
                let (x, z) = (cx.round() as i64 + dx, cz.round() as i64 + dz);
                if inside
                    && (0..args.pixels as i64).contains(&x)
                    && (0..args.pixels as i64).contains(&z)
                {
                    image.put_pixel(x as u32, z as u32, image::Rgb(colour));
                }
            }
        }
    }
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
            let rgb = match terrain.biomes().name(sample.biome) {
                None => [140, 190, 90],
                Some("glacier") => [235, 242, 250],
                Some("desert") => [232, 200, 130],
                Some("forest") => [40, 110, 50],
                Some("tundra") => [150, 160, 135],
                Some("taiga") => [50, 100, 80],
                Some("steppe") => [190, 180, 100],
                Some("savanna") => [205, 165, 65],
                Some("jungle") => [10, 85, 30],
                Some("mountain") => [135, 125, 115],
                Some(_) => [255, 0, 255],
            };
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
    for name in [
        "desert",
        "forest",
        "foothills",
        "glacier",
        "tundra",
        "taiga",
        "steppe",
        "savanna",
        "jungle",
    ] {
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
    let config = WorldConfig {
        size: args.size,
        map_cell: args.cell,
    };
    let start = std::time::Instant::now();
    let map = std::sync::Arc::new(ochre_world::worldmap::generate(args.seed, config));
    println!("generated the world map in {:.1?}", start.elapsed());
    let world_map = Some(map.clone());
    let noise: Box<dyn TerrainSource> = Box::new(WorldTerrain::new(map));

    let start = std::time::Instant::now();
    let samples = sample_map(&*noise, &args);
    println!(
        "sampled {}x{} in {:.1?}",
        args.pixels,
        args.pixels,
        start.elapsed()
    );

    print_stats(&samples, noise.biomes());
    let spawn = noise.find_spawn();
    println!("spawn: x={} z={} height={:.1}", spawn.0, spawn.1, spawn.2);

    let mut image = render(&samples, noise.biomes(), &args);
    if let Some(map) = &world_map {
        overlay_water(&mut image, map, &args);
    }
    image.save(&args.out).expect("failed to save image");
    println!("wrote {}", args.out);
    let stem = args.out.strip_suffix(".png").unwrap_or(&args.out);
    if let Some(map) = &world_map {
        let mut labels = image.clone();
        draw_labels(&mut labels, map, &args);
        let path = format!("{stem}-labels.png");
        labels.save(&path).expect("failed to save image");
        println!("wrote {path}");
        for kind in ochre_world::worldmap::FeatureKind::ALL {
            let count = map.features.iter().filter(|f| f.kind == kind).count();
            println!("{:>12}: {count}", kind.name());
        }
        let problems = ochre_world::worldmap::gameplay::validate(map);
        println!(
            "validation: {}",
            if problems.is_empty() {
                "playable".to_string()
            } else {
                problems.join("; ")
            }
        );
    }
    for (layer, image) in layers(&*noise, &args) {
        let path = format!("{stem}-{layer}.png");
        image.save(&path).expect("failed to save image");
        println!("wrote {path}");
    }
}
