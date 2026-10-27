//! Weathering: droplets of rain wear gullies and valleys into the slopes and lay the silt in
//! flats, then thermal erosion slumps whatever is left steeper than rock holds.
//!
//! Droplets run in tiles, four rounds each tile apart from the next of its round, so tiles run on
//! all cores and a seed still gives the same map: nothing races on a shared grid.

use super::hydrology::{accumulate, priority_flood, NONE};
use super::{par_rows, WorldMap};
use ochre_base::WATER_LEVEL;

// Heights are scaled down by this for the droplets, which are tuned for ground that rises
// about as much as a cell is wide over a range's flank.
const HEIGHT_SCALE: f32 = 1.0 / 120.0;
const DROPLETS_PER_CELL: f32 = 0.12;
const ROUNDS: u32 = 2;
const TILE: usize = 128;
// Cells a droplet can wander past its tile before it is dropped.
const MARGIN: usize = 40;
const LIFETIME: usize = 60;
const INERTIA: f32 = 0.3;
const CAPACITY: f32 = 5.0;
const MIN_SLOPE: f32 = 0.01;
const ERODE: f32 = 0.15;
const DEPOSIT: f32 = 0.3;
const EVAPORATE: f32 = 0.012;
const GRAVITY: f32 = 4.0;
const BRUSH_RADIUS: i32 = 4;

// Thermal erosion: ground steeper than this many blocks of drop per block across slumps.
const TALUS: f32 = 0.7;
const THERMAL_PASSES: usize = 24;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // splitmix64
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
}

// A window of the height grid a tile's droplets run on.
struct Window {
    w: usize,
    h: usize,
    cells: Vec<f32>,
    // cells at or under this are the sea, which droplets don't wear
    sea: f32,
}

impl Window {
    fn at(&self, x: usize, z: usize) -> f32 {
        self.cells[z * self.w + x]
    }

    // Height and gradient at a point, from the four cells round it.
    fn height_and_gradient(&self, x: f32, z: f32) -> (f32, f32, f32) {
        let (ix, iz) = (x as usize, z as usize);
        let (fx, fz) = (x - ix as f32, z - iz as f32);
        let (a, b) = (self.at(ix, iz), self.at(ix + 1, iz));
        let (c, d) = (self.at(ix, iz + 1), self.at(ix + 1, iz + 1));
        (
            a * (1.0 - fx) * (1.0 - fz) + b * fx * (1.0 - fz) + c * (1.0 - fx) * fz + d * fx * fz,
            (b - a) * (1.0 - fz) + (d - c) * fz,
            (c - a) * (1.0 - fx) + (d - b) * fx,
        )
    }

    fn droplet(&mut self, rng: &mut Rng, start: (f32, f32), brush: &[(i32, i32, f32)]) {
        let (mut x, mut z) = start;
        let (mut dx, mut dz) = (0.0f32, 0.0f32);
        let (mut speed, mut water, mut sediment) = (1.0f32, 1.0f32, 0.0f32);
        for _ in 0..LIFETIME {
            let (ix, iz) = (x as usize, z as usize);
            let (fx, fz) = (x - ix as f32, z - iz as f32);
            let (height, gx, gz) = self.height_and_gradient(x, z);
            dx = dx * INERTIA - gx * (1.0 - INERTIA);
            dz = dz * INERTIA - gz * (1.0 - INERTIA);
            let len = (dx * dx + dz * dz).sqrt();
            if len < 1e-6 {
                // on a flat: pick a way to go
                let angle = rng.unit() * std::f32::consts::TAU;
                dx = angle.cos();
                dz = angle.sin();
            } else {
                dx /= len;
                dz /= len;
            }
            x += dx;
            z += dz;
            if x < 1.0 || z < 1.0 || x >= (self.w - 2) as f32 || z >= (self.h - 2) as f32 {
                return;
            }
            let (new_height, _, _) = self.height_and_gradient(x, z);
            if new_height <= self.sea {
                return;
            }
            let drop = new_height - height;
            let capacity = (-drop).max(MIN_SLOPE) * speed * water * CAPACITY;
            if sediment > capacity || drop > 0.0 {
                // uphill it can only drop what fills the step, otherwise a share of the excess
                let amount = if drop > 0.0 {
                    drop.min(sediment)
                } else {
                    (sediment - capacity) * DEPOSIT
                };
                sediment -= amount;
                let w = self.w;
                self.cells[iz * w + ix] += amount * (1.0 - fx) * (1.0 - fz);
                self.cells[iz * w + ix + 1] += amount * fx * (1.0 - fz);
                self.cells[(iz + 1) * w + ix] += amount * (1.0 - fx) * fz;
                self.cells[(iz + 1) * w + ix + 1] += amount * fx * fz;
            } else {
                let amount = ((capacity - sediment) * ERODE).min(-drop);
                for &(bx, bz, weight) in brush {
                    let (cx, cz) = (ix as i32 + bx, iz as i32 + bz);
                    if cx < 0 || cz < 0 || cx >= self.w as i32 || cz >= self.h as i32 {
                        continue;
                    }
                    let cell = &mut self.cells[cz as usize * self.w + cx as usize];
                    // never wear into the sea
                    let taken = (amount * weight).min((*cell - self.sea).max(0.0));
                    *cell -= taken;
                    sediment += taken;
                }
            }
            speed = (speed * speed - drop * GRAVITY).max(0.0).sqrt();
            water *= 1.0 - EVAPORATE;
        }
    }
}

fn brush() -> Vec<(i32, i32, f32)> {
    let mut cells = Vec::new();
    let mut total = 0.0;
    for z in -BRUSH_RADIUS..=BRUSH_RADIUS {
        for x in -BRUSH_RADIUS..=BRUSH_RADIUS {
            let d = ((x * x + z * z) as f32).sqrt();
            if d < BRUSH_RADIUS as f32 {
                let weight = BRUSH_RADIUS as f32 - d;
                cells.push((x, z, weight));
                total += weight;
            }
        }
    }
    for c in &mut cells {
        c.2 /= total;
    }
    cells
}

// One round of droplets over the whole grid, in tiles shifted by `shift` cells.
fn hydraulic_round(grid: &mut [f32], dim: usize, seed: u32, round: u32, shift: usize) {
    let sea = WATER_LEVEL as f32 * HEIGHT_SCALE;
    let brush = brush();
    let tiles = (dim + shift).div_ceil(TILE);
    for phase in 0..4usize {
        let jobs: Vec<(usize, usize)> = (0..tiles)
            .flat_map(|tz| (0..tiles).map(move |tx| (tx, tz)))
            .filter(|&(tx, tz)| (tx % 2) + 2 * (tz % 2) == phase)
            .collect();
        let results: Vec<_> = std::thread::scope(|scope| {
            let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
            let per = jobs.len().div_ceil(threads).max(1);
            let handles: Vec<_> = jobs
                .chunks(per)
                .map(|batch| {
                    let (grid, brush) = (&*grid, &brush);
                    scope.spawn(move || {
                        batch
                            .iter()
                            .filter_map(|&(tx, tz)| {
                                tile(grid, dim, (tx, tz), shift, (seed, round), sea, brush)
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            handles
                .into_iter()
                .flat_map(|h| h.join().unwrap())
                .collect()
        });
        for (x0, z0, window) in results {
            for row in 0..window.h {
                let from = row * window.w;
                let to = (z0 + row) * dim + x0;
                grid[to..to + window.w].copy_from_slice(&window.cells[from..from + window.w]);
            }
        }
    }
}

// Runs a tile's droplets on a window of the grid and returns the window and where it goes back.
fn tile(
    grid: &[f32],
    dim: usize,
    (tx, tz): (usize, usize),
    shift: usize,
    (seed, round): (u32, u32),
    sea: f32,
    brush: &[(i32, i32, f32)],
) -> Option<(usize, usize, Window)> {
    let lo = |t: usize| (t * TILE).saturating_sub(shift);
    let hi = |t: usize| ((t + 1) * TILE).saturating_sub(shift).min(dim);
    let (ix0, ix1, iz0, iz1) = (lo(tx), hi(tx), lo(tz), hi(tz));
    if ix0 >= ix1 || iz0 >= iz1 {
        return None;
    }
    let (x0, z0) = (ix0.saturating_sub(MARGIN), iz0.saturating_sub(MARGIN));
    let (x1, z1) = ((ix1 + MARGIN).min(dim), (iz1 + MARGIN).min(dim));
    let (w, h) = (x1 - x0, z1 - z0);
    let mut window = Window {
        w,
        h,
        cells: Vec::with_capacity(w * h),
        sea,
    };
    for z in z0..z1 {
        window
            .cells
            .extend_from_slice(&grid[z * dim + x0..z * dim + x1]);
    }
    let mut rng = Rng((seed as u64) << 32
        ^ (round as u64) << 24
        ^ (tz as u64) << 12
        ^ tx as u64
        ^ 0xA5A5_0000);
    let droplets = ((ix1 - ix0) * (iz1 - iz0)) as f32 * DROPLETS_PER_CELL / ROUNDS as f32;
    for _ in 0..droplets as usize {
        let x = (ix0 - x0) as f32 + rng.unit() * (ix1 - ix0) as f32;
        let z = (iz0 - z0) as f32 + rng.unit() * (iz1 - iz0) as f32;
        if x >= 1.0 && z >= 1.0 && x < (w - 2) as f32 && z < (h - 2) as f32 {
            window.droplet(&mut rng, (x, z), brush);
        }
    }
    Some((x0, z0, window))
}

/// Rain wears the land: gullies and valleys on the slopes, silt in the flats.
pub(crate) fn hydraulic(map: &mut WorldMap) {
    let dim = map.dim();
    let mut grid: Vec<f32> = map.height.iter().map(|h| h * HEIGHT_SCALE).collect();
    for round in 0..ROUNDS {
        // each round's tiles sit across the last's seams
        let shift = if round % 2 == 0 { 0 } else { TILE / 2 };
        hydraulic_round(&mut grid, dim, map.seed, round, shift);
    }
    for (h, g) in map.height.iter_mut().zip(grid) {
        *h = g / HEIGHT_SCALE;
    }
}

/// Slumps ground steeper than rock holds, leaving scree slopes at the angle of rest.
pub(crate) fn thermal(map: &mut WorldMap) {
    let dim = map.dim();
    let sea = WATER_LEVEL as f32;
    let cell = map.config.map_cell as f32;
    let offsets: [(i64, i64, f32); 8] = [
        (1, 0, 1.0),
        (-1, 0, 1.0),
        (0, 1, 1.0),
        (0, -1, 1.0),
        (1, 1, std::f32::consts::SQRT_2),
        (1, -1, std::f32::consts::SQRT_2),
        (-1, 1, std::f32::consts::SQRT_2),
        (-1, -1, std::f32::consts::SQRT_2),
    ];
    let mut out = vec![(0.0f32, 0.0f32); dim * dim];
    let mut delta = vec![0.0f32; dim * dim];
    for _ in 0..THERMAL_PASSES {
        let h = &map.height;
        let get = |x: i64, z: i64| h[z as usize * dim + x as usize];
        let inside = |x: i64, z: i64| x >= 0 && z >= 0 && x < dim as i64 && z < dim as i64;
        // per cell: how much it sheds, and the total drop it sheds over
        par_rows(dim, &mut out, |row, cells| {
            for (col, cell_out) in cells.iter_mut().enumerate() {
                let (x, z) = (col as i64, row as i64);
                let here = get(x, z);
                if here <= sea {
                    continue;
                }
                let (mut max_drop, mut total) = (0.0f32, 0.0f32);
                for &(dx, dz, dist) in &offsets {
                    if inside(x + dx, z + dz) {
                        let rest = TALUS * cell * dist;
                        let drop = here - get(x + dx, z + dz) - rest;
                        if drop > 0.0 {
                            max_drop = max_drop.max(drop);
                            total += drop;
                        }
                    }
                }
                *cell_out = (max_drop * 0.4, total);
            }
        });
        let shed = &out;
        par_rows(dim, &mut delta, |row, cells| {
            for (col, d) in cells.iter_mut().enumerate() {
                let (x, z) = (col as i64, row as i64);
                let here = get(x, z);
                let mut change = -shed[row * dim + col].0;
                for &(dx, dz, dist) in &offsets {
                    if !inside(x + dx, z + dz) {
                        continue;
                    }
                    let n = (z + dz) as usize * dim + (x + dx) as usize;
                    let (give, total) = shed[n];
                    let drop = get(x + dx, z + dz) - here - TALUS * cell * dist;
                    if total > 0.0 && drop > 0.0 {
                        change += give * drop / total;
                    }
                }
                *d = if here <= sea && change > 0.0 {
                    0.0
                } else {
                    change
                };
            }
        });
        for (h, d) in map.height.iter_mut().zip(&delta) {
            *h += d;
        }
    }
}

// Stream power: ground wears at a rate that grows with the water through it and the slope, so
// the lines water gathers along cut into valleys and the ridges between them stand.
const STREAM_PASSES: usize = 6;
// Per pass, with the water as the square root of the area drained in cells.
const STREAM_K: f32 = 0.03;

/// Cuts valleys along the lines water gathers, solving each pass for the new heights outward
/// from the sea in one sweep, which holds at any rate of wear.
pub(crate) fn stream_power(map: &mut WorldMap) {
    let dim = map.dim();
    let sea = WATER_LEVEL as f32;
    let cell = map.config.map_cell as f32;
    let ties = super::hydrology::flood_ties(map);
    for _ in 0..STREAM_PASSES {
        let flood = priority_flood(&map.height, dim, &ties);
        let flow = accumulate(&flood);
        let before = map.height.clone();
        for &i in &flood.order {
            let (i, d) = (i as usize, flood.down[i as usize]);
            if d == NONE || before[i] <= sea {
                continue;
            }
            let d = d as usize;
            let dist = cell
                * if (i % dim != d % dim) && (i / dim != d / dim) {
                    std::f32::consts::SQRT_2
                } else {
                    1.0
                };
            let f = STREAM_K * flow[i].sqrt() * cell / dist;
            // the new height can't pass the downstream cell's: that would run water uphill
            map.height[i] = ((before[i] + f * map.height[d]) / (1.0 + f)).min(before[i]);
        }
    }
}

/// Valleys, then gullies and silt, then slumping.
pub fn run(map: &mut WorldMap) {
    stream_power(map);
    hydraulic(map);
    thermal(map);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worldmap::WorldConfig;

    fn mountain() -> WorldMap {
        let config = WorldConfig {
            size: 256 * 32,
            map_cell: 32,
            ..Default::default()
        };
        let dim = config.dim();
        let mut map = WorldMap::empty(3, config);
        for z in 0..dim {
            for x in 0..dim {
                let (fx, fz) = (x as f32 - 128.0, z as f32 - 128.0);
                let r = (fx * fx + fz * fz).sqrt();
                // a cone with ridges, on a plain just over the sea
                let ridges = ((fz.atan2(fx) * 7.0).sin() * 0.5 + 0.5) * (1.0 - r / 120.0).max(0.0);
                map.height[z * dim + x] = 12.0 + (1.0 - r / 120.0).max(0.0) * 220.0 + ridges * 80.0;
            }
        }
        map
    }

    #[test]
    fn test_erosion_is_repeatable_and_changes_the_ground() {
        let (mut a, mut b) = (mountain(), mountain());
        let before = a.height.clone();
        run(&mut a);
        run(&mut b);
        assert_eq!(a.height, b.height, "the same seed must erode the same way");
        assert_ne!(before, a.height);
        assert!(a.height.iter().all(|h| h.is_finite()));
    }

    #[test]
    fn test_erosion_keeps_mass_roughly_and_the_sea_alone() {
        let mut map = mountain();
        let dim = map.dim();
        // some sea around the edge
        for i in 0..dim {
            map.height[i] = -48.0;
        }
        let sea_row: Vec<f32> = map.height[..dim].to_vec();
        let before: f64 = map.height.iter().map(|&h| h as f64).sum();
        run(&mut map);
        let after: f64 = map.height.iter().map(|&h| h as f64).sum();
        assert!(
            (after - before).abs() < 3.0 * map.height.len() as f64,
            "{before} -> {after}"
        );
        assert_eq!(sea_row, map.height[..dim], "the sea floor is not eroded");
    }

    #[test]
    fn test_thermal_erosion_slumps_a_spike_and_keeps_its_mass() {
        let config = WorldConfig {
            size: 32 * 32,
            map_cell: 32,
            ..Default::default()
        };
        let mut map = WorldMap::empty(1, config);
        map.height.fill(500.0);
        let spike = 16 * 32 + 16;
        map.height[spike] = 900.0;
        thermal(&mut map);
        assert!(map.height[spike] < 700.0, "spike is {}", map.height[spike]);
        assert!(map.height[spike + 1] > 500.0);
        let mass: f32 = map.height.iter().sum();
        assert!((mass - (500.0 * 1024.0 + 400.0)).abs() < 1.0, "mass {mass}");
    }
}
