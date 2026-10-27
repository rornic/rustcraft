// Samples terrain round rivers, lake shores and sea coasts at block scale and counts what looks
// wrong: water standing over lower dry ground, steps in a water's level, ridges built up beside
// water, river cells dug deep under the map's ground, and on the map, dry cells under the level of
// the lake or sea beside them. Each kind gets an `AUDIT` line, for diffing a sweep before and after
// a change. Run with:
//   cargo run --release --example water_audit -- <seed> [--fine]
use std::sync::Arc;
use std::time::Instant;

use ochre_base::terrain_source::{TerrainSource, WaterKind};
use ochre_terrain::generate::region_store::{bed_top, water_block_top};
use ochre_world::worldmap::{WaterCell, WorldConfig, WorldMap, WorldTerrain, NEIGHBOURS};

// Every this many edge cells of each kind is looked at closely.
const PICK_EVERY: usize = 100;

#[derive(Default)]
struct Counts {
    windows: usize,
    wet: usize,
    walls: usize,
    tall_walls: usize,
    steps: usize,
    dry_near: usize,
    raised: usize,
    fine_walls: usize,
    fine_tall_walls: usize,
    fine_open_edges: usize,
}

// A window of columns round each of `picks`, counting walls of water, steps and raised banks.
fn audit(
    t: &WorldTerrain,
    map: &WorldMap,
    picks: &[usize],
    kind: WaterKind,
    fine: bool,
    marks: &mut Vec<(&'static str, f64, f64, f64)>,
) -> Counts {
    let dim = map.dim();
    let mut c = Counts {
        windows: picks.len(),
        ..Counts::default()
    };
    for &i in picks {
        let (x0, z0) = map.cell_centre(i % dim, i / dim);
        let (n, g) = (48i64, 4.0);
        let at = |x: i64, z: i64| (x0 + ((x - n / 2) as f64) * g, z0 + ((z - n / 2) as f64) * g);
        let s: Vec<_> = (0..n * n)
            .map(|q| {
                let (x, z) = at(q % n, q / n);
                t.sample(x, z)
            })
            .collect();
        for z in 1..n - 1 {
            for x in 1..n - 1 {
                let p = &s[(z * n + x) as usize];
                if p.water_kind == WaterKind::None {
                    let near = [(1i64, 0i64), (-1, 0), (0, 1), (0, -1)]
                        .iter()
                        .any(|&(dx, dz)| s[((z + dz) * n + x + dx) as usize].water_kind == kind);
                    if near {
                        c.dry_near += 1;
                        let (wx, wz) = at(x, z);
                        c.raised += (p.height - map.height_at(wx, wz) as f64 > 2.0) as usize;
                    }
                    continue;
                }
                if p.water_kind != kind {
                    continue;
                }
                c.wet += 1;
                if fine && kind == WaterKind::River {
                    let (wx, wz) = at(x, z);
                    for (dx, dz) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
                        let q = t.sample(wx + dx, wz + dz);
                        if q.water_kind == WaterKind::None {
                            let drop = p.water_surface as f64 - q.height;
                            if bed_top(q.height as f32, q.water_surface)
                                < water_block_top(p.water_surface)
                            {
                                c.fine_open_edges += 1;
                                if c.fine_open_edges <= 8 {
                                    marks.push(("open block edge", wx, wz, drop));
                                }
                            }
                            if drop > 1.5 {
                                c.fine_walls += 1;
                                c.fine_tall_walls += (drop > 6.0) as usize;
                                if c.fine_walls <= 8 {
                                    marks.push(("one-block wall", wx, wz, drop));
                                }
                            }
                        }
                    }
                }
                for (dx, dz) in [(1i64, 0i64), (0, 1)] {
                    let q = &s[((z + dz) * n + x + dx) as usize];
                    if q.water_kind == WaterKind::None {
                        let d = p.water_surface as f64 - q.height;
                        if (d > 1.5 && marks.len() < 8) || (d > 6.0 && c.tall_walls < 3) {
                            let (wx, wz) = at(x, z);
                            marks.push((if d > 6.0 { "tall wall" } else { "wall" }, wx, wz, d));
                        }
                        c.walls += (d > 1.5) as usize;
                        c.tall_walls += (d > 6.0) as usize;
                    } else if q.water_kind == kind
                        && (p.water_surface - q.water_surface).abs() >= 1.5
                    {
                        c.steps += 1;
                        if marks.len() < 16 && c.steps.is_multiple_of(8) {
                            let (wx, wz) = at(x, z);
                            marks.push((
                                "step",
                                wx,
                                wz,
                                (p.water_surface - q.water_surface) as f64,
                            ));
                        }
                    }
                }
            }
        }
    }
    c
}

// Every `PICK_EVERY`th cell of `kind` with a neighbour of another kind: its edge.
fn edge_picks(map: &WorldMap, kind: WaterCell) -> Vec<usize> {
    let dim = map.dim() as i64;
    (0..map.height.len())
        .filter(|&i| {
            map.water_kind[i] == kind as u8 && {
                let (x, z) = ((i as i64) % dim, (i as i64) / dim);
                NEIGHBOURS.iter().any(|&(dx, dz)| {
                    let (nx, nz) = (x + dx, z + dz);
                    (0..dim).contains(&nx)
                        && (0..dim).contains(&nz)
                        && map.water_kind[(nz * dim + nx) as usize] != kind as u8
                })
            }
        })
        .enumerate()
        .filter(|(n, _)| n % PICK_EVERY == PICK_EVERY - 1)
        .map(|(_, i)| i)
        .collect()
}

// Dry cells under the level of a lake or the sea beside them by more than 2 blocks: water there
// would stand over them, held only by the cells' edges.
fn uncontained(map: &WorldMap) -> usize {
    let dim = map.dim() as i64;
    let mut count = 0;
    for i in 0..map.height.len() {
        let kind = map.water_kind[i];
        if kind != WaterCell::Lake as u8 && kind != WaterCell::Ocean as u8 {
            continue;
        }
        let (x, z) = ((i as i64) % dim, (i as i64) / dim);
        for (dx, dz) in NEIGHBOURS {
            let (nx, nz) = (x + dx, z + dz);
            if !(0..dim).contains(&nx) || !(0..dim).contains(&nz) {
                continue;
            }
            let n = (nz * dim + nx) as usize;
            count += (map.water_kind[n] == WaterCell::None as u8
                && map.height[n] < map.water_surface[i] - 2.0) as usize;
        }
    }
    count
}

fn main() {
    let seed: u64 = std::env::args().nth(1).expect("a seed").parse().unwrap();
    let fine = std::env::args().any(|arg| arg == "--fine");
    let start = Instant::now();
    let map = Arc::new(ochre_world::worldmap::generate(
        seed as _,
        WorldConfig::default(),
    ));
    println!("map {:.2?}", start.elapsed());
    let start = Instant::now();
    let t = WorldTerrain::new(map.clone());
    println!("terrain setup {:.2?}", start.elapsed());
    let dim = map.dim();
    let (mut deep, mut deep50) = (0, 0);
    let mut worst: Vec<(f64, f64, f64)> = Vec::new();
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
    let rivers: Vec<usize> = (0..dim * dim)
        .filter(|&i| map.water_kind[i] == WaterCell::River as u8)
        .enumerate()
        .filter(|(n, _)| n % PICK_EVERY == PICK_EVERY - 1)
        .map(|(_, i)| i)
        .collect();
    let mut kinds = Vec::new();
    for (name, kind, picks) in [
        ("rivers", WaterKind::River, rivers),
        ("lakes", WaterKind::Lake, edge_picks(&map, WaterCell::Lake)),
        (
            "coasts",
            WaterKind::Ocean,
            edge_picks(&map, WaterCell::Ocean),
        ),
    ] {
        let mut marks = Vec::new();
        let counts = audit(&t, &map, &picks, kind, fine, &mut marks);
        kinds.push((name, counts, marks));
    }
    for (name, _, marks) in &kinds {
        for m in marks {
            println!("  {name} {} {:.1} at {:.0},{:.0}", m.0, m.3, m.1, m.2);
        }
    }
    for w in &worst {
        println!("  deep cut {:.0} at {:.0},{:.0}", w.2, w.0, w.1);
    }
    println!(
        "AUDIT seed={seed} map uncontained={} deep14={deep} deep25={deep50}",
        uncontained(&map)
    );
    for (name, c, _) in kinds {
        println!(
            "AUDIT seed={seed} {name} windows={} wet={} walls={} tall_walls={} steps={} dry_near={} raised={}",
            c.windows, c.wet, c.walls, c.tall_walls, c.steps, c.dry_near, c.raised
        );
        if fine && name == "rivers" {
            println!(
                "AUDIT seed={seed} {name} one_block_walls={} one_block_tall_walls={} open_block_edges={}",
                c.fine_walls, c.fine_tall_walls, c.fine_open_edges
            );
        }
    }
}
