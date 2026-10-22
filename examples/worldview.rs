// A browser viewer for a seed's whole world, for debugging world generation: pan and zoom, stack
// layers (height, hillshade, blocks, biomes, trees, water, climate, roughness, features...), click
// a spot for every number there, and export the view as a PNG to share.
//   cargo run --release --example worldview -- [seed] [port]
// then open http://127.0.0.1:8777 (default port). The map takes a few seconds per seed.
// Zoomed out (above 2 blocks a pixel) tiles come from `WorldTerrain::sample_for_cell`; at 2 or
// fewer they come from `RegionStore` ground, water and slope, which is what chunks are built from.
// Export writes target/worldview/<seed>-<x>_<z>-<scale>.png, prints the path and downloads it too:
//   curl 'http://127.0.0.1:8777/export?seed=42&layers=height,hillshade,water:0.6&x=0&z=0&w=1024&h=1024&scale=4' -o view.png
// Rough: one world in memory at a time, tree density only (not individual trees), localhost only.
use std::collections::HashMap;
use std::io::{Cursor, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use bevy::color::{ColorToComponents, LinearRgba, Srgba};
use ochre_base::biome::grass_colour;
use ochre_base::block::BlockType;
use ochre_base::terrain_source::{TerrainSample, TerrainSource, WaterKind};
use ochre_base::WATER_LEVEL;
use ochre_terrain::generate::generator::surface_block;
use ochre_terrain::generate::region_store::RegionStore;
use ochre_world::worldmap::{
    FeatureKind, WaterCell, WorldConfig, WorldMap, WorldTerrain, NEIGHBOURS,
};

struct World {
    map: Arc<WorldMap>,
    terrain: Arc<WorldTerrain>,
    regions: RegionStore,
}

#[derive(Default)]
struct Server {
    world: Mutex<Option<(u32, Arc<World>)>>,
}

impl Server {
    fn world(&self, seed: u32) -> Arc<World> {
        let mut slot = self.world.lock().unwrap();
        if let Some((s, w)) = &*slot {
            if *s == seed {
                return w.clone();
            }
        }
        let start = Instant::now();
        let map = Arc::new(ochre_world::worldmap::generate(
            seed,
            WorldConfig::default(),
        ));
        let terrain = Arc::new(WorldTerrain::new(map.clone()));
        let regions = RegionStore::new(terrain.clone()).with_resident_cap(512);
        println!("seed {seed} generated in {:.1?}", start.elapsed());
        let world = Arc::new(World {
            map,
            terrain,
            regions,
        });
        *slot = Some((seed, world.clone()));
        world
    }
}

#[derive(Default, Clone)]
struct Col {
    s: TerrainSample,
    block: BlockType,
    temp: f32,
    x: f64,
    z: f64,
}

struct View {
    x: f64,
    z: f64,
    w: usize,
    h: usize,
    scale: f64,
    layers: Vec<(String, f32)>,
}

impl View {
    fn has(&self, name: &str) -> bool {
        self.layers.iter().any(|l| l.0 == name)
    }
}

type Rgb = [f32; 3];

fn lerp(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    [0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * t)
}

fn ramp(stops: &[(f32, Rgb)], v: f32) -> Rgb {
    if v <= stops[0].0 {
        return stops[0].1;
    }
    for p in stops.windows(2) {
        if v < p[1].0 {
            return lerp(p[0].1, p[1].1, (v - p[0].0) / (p[1].0 - p[0].0));
        }
    }
    stops[stops.len() - 1].1
}

fn hsv(h: f32, s: f32, v: f32) -> Rgb {
    let f = |n: f32| {
        let k = (n + h * 6.0) % 6.0;
        v - v * s * k.min(4.0 - k).clamp(0.0, 1.0)
    };
    [f(5.0) * 255.0, f(3.0) * 255.0, f(1.0) * 255.0]
}

fn height_colour(above_sea: f32) -> Rgb {
    ramp(
        &[
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
        ],
        above_sea,
    )
}

fn block_colour(b: BlockType) -> Option<Rgb> {
    use BlockType::*;
    Some(match b {
        Air => return None,
        Stone => [128.0, 128.0, 128.0],
        CompactedStone => [90.0, 90.0, 95.0],
        Grass | TallGrass | Fern | ShortGrass | SeedGrass | Poppy | Dandelion => {
            [90.0, 160.0, 60.0]
        }
        Sand => [222.0, 204.0, 140.0],
        Sandstone => [200.0, 170.0, 110.0],
        Water => [40.0, 90.0, 190.0],
        Snow => [250.0, 250.0, 255.0],
        Ice => [160.0, 205.0, 240.0],
        Dirt => [120.0, 85.0, 55.0],
        Gravel => [150.0, 145.0, 140.0],
        Log => [100.0, 70.0, 40.0],
        Leaves => [30.0, 110.0, 40.0],
    })
}

fn biome_colour(name: Option<&str>) -> Rgb {
    match name {
        None => [140.0, 190.0, 90.0],
        Some("glacier") => [235.0, 242.0, 250.0],
        Some("desert") => [232.0, 200.0, 130.0],
        Some("forest") => [40.0, 110.0, 50.0],
        Some("tundra") => [150.0, 160.0, 135.0],
        Some("taiga") => [50.0, 100.0, 80.0],
        Some("steppe") => [190.0, 180.0, 100.0],
        Some("savanna") => [205.0, 165.0, 65.0],
        Some("jungle") => [10.0, 85.0, 30.0],
        Some("mountain") => [135.0, 125.0, 115.0],
        Some(_) => [255.0, 0.0, 255.0],
    }
}

fn feature_colour(k: FeatureKind) -> [u8; 3] {
    use FeatureKind::*;
    match k {
        Pass => [255, 255, 255],
        Ford => [0, 230, 255],
        BridgeSite => [255, 140, 0],
        Spawn => [40, 255, 60],
        Iron => [140, 80, 40],
        Coal => [10, 10, 10],
        Clay => [210, 110, 80],
        Salt => [255, 150, 200],
        Cave => [60, 80, 255],
        Hotspot => [255, 0, 0],
    }
}

fn fill(w: &World, v: &View, grid: &mut [Col], gw: usize) {
    let (need_block, need_temp) = (v.has("blocks"), v.has("temperature"));
    let exact = v.scale <= 2.0;
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let rows = grid.len() / gw;
    std::thread::scope(|scope| {
        for (n, chunk) in grid.chunks_mut(rows.div_ceil(threads) * gw).enumerate() {
            let first = n * rows.div_ceil(threads);
            scope.spawn(move || {
                let mut hs = w.regions.sampler();
                for (i, col) in chunk.iter_mut().enumerate() {
                    let (px, pz) = ((i % gw) as f64 - 1.0, (first + i / gw) as f64 - 1.0);
                    let mut x = v.x + (px + 0.5) * v.scale;
                    let mut z = v.z + (pz + 0.5) * v.scale;
                    if exact {
                        (x, z) = (x.floor(), z.floor());
                    }
                    col.x = x;
                    col.z = z;
                    col.s = w
                        .terrain
                        .sample_for_cell(x, z, if exact { 1.0 } else { v.scale });
                    if need_temp || need_block {
                        col.temp = w.terrain.temperature(x, z);
                    }
                    if need_block {
                        let (bx, bz) = (x as i64, z as i64);
                        let (ground, water_top, gradient) = if exact {
                            let (gx, gz) = hs.gradient(bx, bz);
                            (
                                hs.ground_top(bx, bz),
                                hs.water_top(bx, bz),
                                gx.abs() + gz.abs(),
                            )
                        } else {
                            let h = |dx: i64, dz: i64| {
                                w.terrain.sample((bx + dx) as f64, (bz + dz) as f64).height
                            };
                            let g =
                                (h(6, 0) - h(-6, 0)).abs() / 6.0 + (h(0, 6) - h(0, -6)).abs() / 6.0;
                            let s = col.s;
                            let top = s.height.round() as i64;
                            let water = if s.water_surface > 0.0 {
                                s.water_surface.round() as i64
                            } else {
                                WATER_LEVEL
                            };
                            (top, water, g)
                        };
                        col.block = surface_block(
                            w.terrain.biomes(),
                            col.s.biome,
                            w.map.seed,
                            (bx, bz),
                            (ground, water_top),
                            gradient,
                            col.temp,
                        );
                    }
                }
            });
        }
    });
}

// A layer's colour at one pixel and how opaque it is there, or None for no change.
fn layer_pixel(w: &World, v: &View, name: &str, c: &Col) -> Option<(Rgb, f32)> {
    let map = &w.map;
    let b = |l: &[u8]| map.bilinear_bytes(l, c.x, c.z) / 255.0;
    let above = (c.s.height - WATER_LEVEL as f64) as f32;
    let gray = |t: f32| [t.clamp(0.0, 1.0) * 255.0; 3];
    Some(match name {
        "height" => (height_colour(above), 1.0),
        "blocks" => {
            let mut colour = block_colour(c.block)?;
            if c.block == BlockType::Grass {
                let l = grass_colour(w.terrain.climate(c.x, c.z));
                let srgb = Srgba::from(LinearRgba::rgb(l.x, l.y, l.z)).to_vec3() * 255.0;
                colour = [srgb.x, srgb.y, srgb.z];
            }
            (colour, 1.0)
        }
        "biomes" => {
            let colour = if above < 0.0 {
                [20.0, 40.0, 90.0]
            } else {
                biome_colour(w.terrain.biomes().name(c.s.biome))
            };
            (colour, 1.0)
        }
        "trees" => (
            lerp(
                [30.0, 30.0, 30.0],
                [20.0, 200.0, 40.0],
                (c.s.trees * 1.6) as f32,
            ),
            1.0,
        ),
        "water" => {
            let depth = (c.s.water_surface - c.s.height as f32).clamp(0.0, 60.0) / 60.0;
            let colour = match c.s.water_kind {
                WaterKind::None => return None,
                WaterKind::Ocean => [30.0 - 25.0 * depth, 90.0 - 70.0 * depth, 200.0],
                WaterKind::Lake => [60.0, 200.0, 230.0],
                WaterKind::River => [120.0, 230.0, 255.0],
            };
            (colour, 1.0)
        }
        "waterlevel" => {
            if c.s.water_kind == WaterKind::None {
                return None;
            }
            let s = c.s.water_surface;
            (
                hsv(
                    ((s - WATER_LEVEL as f32) / 300.0).clamp(0.0, 1.0) * 0.75,
                    1.0,
                    0.55 + 0.45 * s.fract().abs(),
                ),
                1.0,
            )
        }
        "flowdir" => {
            if c.s.water_kind != WaterKind::River {
                return None;
            }
            let (u, vv) = map.to_cell(c.x, c.z);
            let max = (map.dim() - 1) as f64;
            let i = map.index(
                u.round().clamp(0.0, max) as usize,
                vv.round().clamp(0.0, max) as usize,
            );
            match map.down[i] {
                255 => ([255.0, 255.0, 255.0], 1.0),
                d => (hsv(d as f32 / 8.0, 1.0, 1.0), 1.0),
            }
        }
        "flow" => {
            let f = map.bilinear(&map.flow, c.x, c.z).max(1.0).log2() / 14.0;
            (lerp([20.0, 20.0, 30.0], [80.0, 200.0, 255.0], f), 1.0)
        }
        "temperature" => (
            ramp(
                &[
                    (0.0, [40.0, 60.0, 220.0]),
                    (0.5, [240.0, 240.0, 120.0]),
                    (1.0, [230.0, 40.0, 30.0]),
                ],
                c.temp,
            ),
            1.0,
        ),
        "moisture" => (
            lerp(
                [200.0, 170.0, 100.0],
                [20.0, 80.0, 200.0],
                map.bilinear(&map.moisture, c.x, c.z),
            ),
            1.0,
        ),
        "continentalness" => (
            ramp(
                &[
                    (-1.0, [10.0, 20.0, 80.0]),
                    (0.0, [200.0, 200.0, 140.0]),
                    (1.0, [60.0, 140.0, 60.0]),
                ],
                c.s.continentalness as f32,
            ),
            1.0,
        ),
        "roughness" => (gray(b(&map.roughness)), 1.0),
        "range" => (gray(b(&map.range)), 1.0),
        "contours" => {
            let h = c.s.height as f32;
            if (h / 10.0).fract().abs() * 10.0 < v.scale as f32 {
                ([0.0, 0.0, 0.0], 0.6)
            } else {
                return None;
            }
        }
        "cells" => {
            let (cell, half) = (map.config.map_cell as f64, map.config.half_size() as f64);
            if ((c.x + half) % cell) < v.scale || ((c.z + half) % cell) < v.scale {
                ([255.0, 255.0, 255.0], 0.35)
            } else {
                return None;
            }
        }
        _ => return None,
    })
}

fn render(w: &World, v: &View) -> Vec<u8> {
    let gw = v.w + 2;
    let mut grid = vec![Col::default(); gw * (v.h + 2)];
    fill(w, v, &mut grid, gw);
    let mut out = vec![0u8; v.w * v.h * 3];
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let per = v.h.div_ceil(threads);
    let grid = &grid;
    std::thread::scope(|scope| {
        for (n, rows) in out.chunks_mut(per * v.w * 3).enumerate() {
            scope.spawn(move || {
                for (i, px) in rows.chunks_mut(3).enumerate() {
                    let (x, z) = (i % v.w + 1, n * per + i / v.w + 1);
                    let c = &grid[z * gw + x];
                    let hgt = |x: usize, z: usize| grid[z * gw + x].s.height;
                    let mut colour: Rgb = [24.0; 3];
                    for (name, alpha) in &v.layers {
                        if name == "hillshade" {
                            let dx = (hgt(x + 1, z) - hgt(x - 1, z)) / (2.0 * v.scale);
                            let dz = (hgt(x, z + 1) - hgt(x, z - 1)) / (2.0 * v.scale);
                            let dry = c.s.height > WATER_LEVEL as f64;
                            let shade = if dry {
                                (1.0 - (dx + dz) * 3.0).clamp(0.4, 1.4) as f32
                            } else {
                                1.0
                            };
                            colour = colour.map(|ch| ch * (1.0 + (shade - 1.0) * alpha));
                        } else if let Some((layer, a)) = layer_pixel(w, v, name, c) {
                            colour = lerp(colour, layer, a * alpha);
                        }
                    }
                    for k in 0..3 {
                        px[k] = colour[k].clamp(0.0, 255.0) as u8;
                    }
                }
            });
        }
    });
    if v.has("features") {
        let radius = 4.0f64.max(100.0 / v.scale).min(12.0);
        for f in &w.map.features {
            let (cx, cz) = ((f.x as f64 - v.x) / v.scale, (f.z as f64 - v.z) / v.scale);
            let r = radius.ceil() as i64;
            for dz in -r..=r {
                for dx in -r..=r {
                    let (x, z) = (cx as i64 + dx, cz as i64 + dz);
                    if (dx * dx + dz * dz) as f64 <= radius * radius
                        && x >= 0
                        && z >= 0
                        && (x as usize) < v.w
                        && (z as usize) < v.h
                    {
                        let i = (z as usize * v.w + x as usize) * 3;
                        out[i..i + 3].copy_from_slice(&feature_colour(f.kind));
                    }
                }
            }
        }
    }
    out
}

fn png(rgb: Vec<u8>, w: usize, h: usize) -> Vec<u8> {
    let img = image::RgbImage::from_raw(w as u32, h as u32, rgb).unwrap();
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

fn name_of(b: BlockType) -> String {
    format!("{b:?}")
}

fn info(w: &World, x: f64, z: f64) -> String {
    let map = &w.map;
    let (bx, bz) = (x.floor() as i64, z.floor() as i64);
    let s = w.terrain.sample(bx as f64, bz as f64);
    let mut hs = w.regions.sampler();
    let (ground, water_top) = (hs.ground_top(bx, bz), hs.water_top(bx, bz));
    let (gx, gz) = hs.gradient(bx, bz);
    let temp = w.terrain.temperature(bx as f64, bz as f64);
    let block = surface_block(
        w.terrain.biomes(),
        s.biome,
        map.seed,
        (bx, bz),
        (ground, water_top),
        gx.abs() + gz.abs(),
        temp,
    );
    let (u, v) = map.to_cell(bx as f64, bz as f64);
    let max = (map.dim() - 1) as f64;
    let (cx, cz) = (
        u.round().clamp(0.0, max) as usize,
        v.round().clamp(0.0, max) as usize,
    );
    let i = map.index(cx, cz);
    let near: Vec<String> = map
        .features
        .iter()
        .filter(|f| (f.x as f64 - x).abs() < 300.0 && (f.z as f64 - z).abs() < 300.0)
        .map(|f| format!("{} {:.0} at {},{}", f.kind.name(), f.value, f.x, f.z))
        .collect();
    let down = match map.down[i] {
        255 => "none".to_string(),
        d => format!("{:?}", NEIGHBOURS[d as usize]),
    };
    let lines = [
        format!("block x={bx} z={bz}  map cell {cx},{cz}"),
        format!(
            "ground height {:.2} (map bicubic {:.2}), top block y={ground}",
            s.height,
            map.height_at(x, z)
        ),
        format!(
            "surface block {}  slope {:.2}",
            name_of(block),
            gx.abs() + gz.abs()
        ),
        format!(
            "water {:?} surface {:.2} depth {:.2} (water_top {water_top})",
            s.water_kind,
            s.water_surface,
            s.water_surface - s.height as f32
        ),
        format!(
            "map cell water {:?} surface {:.2} river width {:.1} ford {}",
            WaterCell::from_u8(map.water_kind[i]),
            map.water_surface[i],
            map.river_width[i],
            map.ford[i]
        ),
        format!("flow {:.0} cells, drains {down}", map.flow[i]),
        format!(
            "biome {:?} (map {:?})  trees {:.2}",
            w.terrain.biomes().name(s.biome),
            ochre_world::worldmap::climate::BiomeKind::from_u8(map.biome[i]),
            s.trees
        ),
        format!(
            "temperature {:.3} moisture {:.3} (map cell {:.3}/{:.3})",
            temp,
            map.bilinear(&map.moisture, x, z),
            map.temperature[i],
            map.moisture[i]
        ),
        format!(
            "continentalness {:.3} roughness {:.2} range {:.2}",
            s.continentalness,
            map.roughness[i] as f32 / 255.0,
            map.range[i] as f32 / 255.0
        ),
        format!(
            "features within 300: {}",
            if near.is_empty() {
                "none".into()
            } else {
                near.join("; ")
            }
        ),
    ];
    lines.join("\n")
}

fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let (mut out, mut i) = (Vec::new(), 0);
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => {
                out.push(u8::from_str_radix(&s[i + 1..i + 3], 16).unwrap_or(b'?'));
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn reply(mut s: TcpStream, status: &str, ctype: &str, extra: &str, body: &[u8]) {
    let head = format!("HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\nAccess-Control-Expose-Headers: X-Path\r\n{extra}\r\n", body.len());
    let _ = s.write_all(head.as_bytes()).and_then(|_| s.write_all(body));
}

fn parse_view(q: &HashMap<String, String>, w: usize, h: usize) -> View {
    let num = |k: &str, d: f64| q.get(k).and_then(|v| v.parse().ok()).unwrap_or(d);
    let layers = q
        .get("layers")
        .map(|l| {
            l.split(',')
                .filter(|s| !s.is_empty())
                .map(|s| match s.split_once(':') {
                    Some((n, a)) => (n.to_string(), a.parse().unwrap_or(1.0)),
                    None => (s.to_string(), 1.0),
                })
                .collect()
        })
        .unwrap_or_default();
    View {
        x: num("x", 0.0),
        z: num("z", 0.0),
        w,
        h,
        scale: num("scale", 1.0).max(0.25),
        layers,
    }
}

fn handle(server: &Server, mut stream: TcpStream) {
    let mut buf = [0u8; 4096];
    let n = stream.read(&mut buf).unwrap_or(0);
    let req = String::from_utf8_lossy(&buf[..n]);
    let target = req.split_whitespace().nth(1).unwrap_or("/");
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let q: HashMap<String, String> = query
        .split('&')
        .filter_map(|kv| kv.split_once('='))
        .map(|(k, v)| (k.to_string(), decode(v)))
        .collect();
    let seed: u32 = q.get("seed").and_then(|s| s.parse().ok()).unwrap_or(42);
    let num = |k: &str, d: f64| q.get(k).and_then(|v| v.parse().ok()).unwrap_or(d);
    match path {
        "/" => reply(
            stream,
            "200 OK",
            "text/html",
            "",
            include_bytes!("worldview.html"),
        ),
        "/seed" => {
            server.world(seed);
            reply(stream, "200 OK", "text/plain", "", b"ok");
        }
        "/tile" => {
            let world = server.world(seed);
            let size = (num("size", 256.0) as usize).clamp(1, 1024);
            let v = parse_view(&q, size, size);
            reply(
                stream,
                "200 OK",
                "image/png",
                "Cache-Control: max-age=600\r\n",
                &png(render(&world, &v), size, size),
            );
        }
        "/info" => {
            let world = server.world(seed);
            reply(
                stream,
                "200 OK",
                "text/plain",
                "",
                info(&world, num("x", 0.0), num("z", 0.0)).as_bytes(),
            );
        }
        "/export" => {
            let world = server.world(seed);
            let (w, h) = (
                (num("w", 1024.0) as usize).clamp(1, 4096),
                (num("h", 1024.0) as usize).clamp(1, 4096),
            );
            let v = parse_view(&q, w, h);
            let data = png(render(&world, &v), w, h);
            let dir = std::path::Path::new("target/worldview");
            std::fs::create_dir_all(dir).unwrap();
            let name = format!(
                "{seed}-{:.0}_{:.0}-{}x{}-s{}-{}.png",
                v.x,
                v.z,
                w,
                h,
                v.scale,
                v.layers
                    .iter()
                    .map(|l| l.0.as_str())
                    .collect::<Vec<_>>()
                    .join("+")
            );
            let path = dir.join(name);
            std::fs::write(&path, &data).unwrap();
            let abs = std::fs::canonicalize(&path).unwrap();
            println!("exported {}", abs.display());
            let extra = format!("X-Path: {}\r\n", abs.display());
            reply(stream, "200 OK", "image/png", &extra, &data);
        }
        _ => reply(stream, "404 Not Found", "text/plain", "", b"not found"),
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let seed: u32 = args
        .next()
        .map_or(42, |s| s.parse().expect("seed is a u32"));
    let port: u16 = args.next().map_or(8777, |s| s.parse().expect("port"));
    let server = Arc::new(Server::default());
    let listener = TcpListener::bind(("127.0.0.1", port)).expect("port in use");
    println!("http://127.0.0.1:{port}/?seed={seed}");
    for stream in listener.incoming().flatten() {
        let server = server.clone();
        std::thread::spawn(move || handle(&server, stream));
    }
}
