// Samples terrain round rivers and lakes at block scale and counts what looks wrong: water
// standing over lower dry ground, steps in a river's level, ridges built up beside water, and
// river cells dug deep under the map's ground. Run with:
//   cargo run --release --example water_audit -- <seed>
use std::sync::Arc;
use std::time::Instant;

use rustcraft::chunks::generate::terrain_source::{TerrainSource, WaterKind};
use rustcraft::worldmap::{WorldConfig, WorldTerrain};

fn main() {
    let seed: u64 = std::env::args().nth(1).expect("a seed").parse().unwrap();
    let map = Arc::new(rustcraft::worldmap::generate(
        seed as _,
        WorldConfig::default(),
    ));
    let start = Instant::now();
    let t = WorldTerrain::new(map.clone());
    println!("terrain setup {:.2?}", start.elapsed());
    let dim = map.dim();
    let (mut deep, mut deep50) = (0, 0);
    let mut worst: Vec<(f64, f64, f64)> = Vec::new();
    let mut marks: Vec<(&str, f64, f64, f64)> = Vec::new();
    for i in 0..dim * dim {
        if map.river_width[i] <= 0.0 {
            continue;
        }
        let (x, z) = map.cell_centre(i % dim, i / dim);
        // how far the river's bed is cut under the ground there would be with no river, at the
        // deepest wet column near the cell's centre
        let mut cut = 0.0f64;
        for dz in (-48..=48).step_by(12) {
            for dx in (-48..=48).step_by(12) {
                let p = t.sample(x + dx as f64, z + dz as f64);
                if p.water_kind == WaterKind::River {
                    cut = cut.max(map.height_at(x + dx as f64, z + dz as f64) as f64 - p.height);
                }
            }
        }
        if cut > 25.0 && worst.len() < 4 {
            worst.push((x, z, cut));
        }
        deep += (cut > 14.0) as usize;
        deep50 += (cut > 25.0) as usize;
    }
    let picks: Vec<usize> = (0..dim * dim)
        .filter(|&i| map.water_kind[i] == 3)
        .enumerate()
        .filter(|(n, _)| n % 300 == 299)
        .map(|(_, i)| i)
        .collect();
    let (mut wet, mut walls, mut tall_walls, mut steps, mut raised, mut dry_near) =
        (0, 0, 0, 0, 0, 0);
    for &i in &picks {
        let (x0, z0) = map.cell_centre(i % dim, i / dim);
        let (n, g) = (48i64, 4.0);
        let s: Vec<_> = (0..n * n)
            .map(|q| {
                t.sample(
                    x0 + ((q % n) - n / 2) as f64 * g,
                    z0 + ((q / n) - n / 2) as f64 * g,
                )
            })
            .collect();
        for z in 1..n - 1 {
            for x in 1..n - 1 {
                let p = &s[(z * n + x) as usize];
                if p.water_kind == WaterKind::None {
                    let near = [(1i64, 0i64), (-1, 0), (0, 1), (0, -1)]
                        .iter()
                        .any(|&(dx, dz)| {
                            s[((z + dz) * n + x + dx) as usize].water_kind != WaterKind::None
                        });
                    if near {
                        dry_near += 1;
                        let (wx, wz) =
                            (x0 + ((x - n / 2) as f64) * g, z0 + ((z - n / 2) as f64) * g);
                        raised += (p.height - map.height_at(wx, wz) as f64 > 2.0) as usize;
                    }
                    continue;
                }
                wet += 1;
                for (dx, dz) in [(1i64, 0i64), (0, 1)] {
                    let q = &s[((z + dz) * n + x + dx) as usize];
                    if q.water_kind == WaterKind::None {
                        let d = p.water_surface as f64 - q.height;
                        if d > 1.5 && marks.len() < 6 {
                            marks.push((
                                "wall",
                                x0 + ((x - n / 2) as f64) * g,
                                z0 + ((z - n / 2) as f64) * g,
                                d,
                            ));
                        }
                        walls += (d > 1.5) as usize;
                        tall_walls += (d > 6.0) as usize;
                    } else if p.water_kind == q.water_kind
                        && (p.water_surface - q.water_surface).abs() >= 1.5
                    {
                        steps += 1;
                        if marks.len() < 12 && steps % 8 == 0 {
                            marks.push((
                                "step",
                                x0 + ((x - n / 2) as f64) * g,
                                z0 + ((z - n / 2) as f64) * g,
                                (p.water_surface - q.water_surface) as f64,
                            ));
                        }
                    }
                }
            }
        }
    }
    for m in &marks {
        println!("  {} {:.1} at {:.0},{:.0}", m.0, m.3, m.1, m.2);
    }
    for w in &worst {
        println!("  deep cut {:.0} at {:.0},{:.0}", w.2, w.0, w.1);
    }
    println!(
        "river cell centres cut under the map's smooth ground: 14+ blocks {deep}, 25+ {deep50}\n{} windows, {wet} wet columns: water over lower dry ground 1.5+ {walls}, 6+ {tall_walls}; level steps 1.5+ {steps}\ndry columns beside water {dry_near}, standing over the map's smooth ground by more than 2 blocks {raised}",
        picks.len()
    );
}
