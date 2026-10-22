// Looks for steps in the ground beside rivers and lakes, where one block's change in height is far
// more than its neighbours', and where they fall on the map's cells.
use ochre_base::terrain_source::{TerrainSource, WaterKind};
use ochre_world::worldmap::{WorldConfig, WorldTerrain};
use std::sync::Arc;

fn main() {
    let seed: u64 = std::env::args().nth(1).unwrap().parse().unwrap();
    let map = Arc::new(ochre_world::worldmap::generate(
        seed as _,
        WorldConfig::default(),
    ));
    let t = WorldTerrain::new(map.clone());
    let dim = map.dim();
    let half = map.config.half_size() as f64;
    let picks: Vec<usize> = (0..dim * dim)
        .filter(|&i| map.water_kind[i] == 3 || map.water_kind[i] == 2)
        .enumerate()
        .filter(|(n, _)| n % 250 == 249)
        .map(|(_, i)| i)
        .collect();
    let (mut total, mut across) = (0, 0);
    let mut hist_x = [0usize; 8];
    let mut examples = Vec::new();
    for &i in &picks {
        let (x0, z0) = map.cell_centre(i % dim, i / dim);
        let (x0, z0) = (x0.floor() - 100.0, z0.floor() - 100.0);
        let n = 200usize;
        let s: Vec<_> = (0..n * n)
            .map(|q| t.sample(x0 + (q % n) as f64 + 0.5, z0 + (q / n) as f64 + 0.5))
            .collect();
        let at = |x: usize, z: usize| &s[z * n + x];
        for z in 3..n - 3 {
            for x in 3..n - 3 {
                if (0..5).any(|k| at(x + k - 2, z).water_kind != WaterKind::None) {
                    continue;
                }
                let d = |k: usize| (at(x + k + 1 - 2, z).height - at(x + k - 2, z).height).abs();
                // d(2) is the change from x to x+1
                let jump = d(2);
                let around = d(1).max(d(3)).max(d(0));
                if jump > 0.2 && jump < 3.0 && jump > 4.0 * around.max(0.04) {
                    total += 1;
                    let wx = x0 + x as f64 + 1.0;
                    let u = ((wx + half) / 32.0).rem_euclid(1.0);
                    hist_x[((u * 8.0) as usize).min(7)] += 1;
                    if examples.len() < 10 {
                        examples.push((wx, z0 + z as f64, jump, u));
                    }
                }
                let _ = &mut across;
            }
        }
    }
    println!("{} windows: {total} ground steps along x beside water; where they fall across a cell (eighths): {hist_x:?}", picks.len());
    for e in examples {
        println!(
            "  step {:.2} at {:.0},{:.0} ({:.2} across its cell)",
            e.2, e.0, e.1, e.3
        );
    }
}
