//! The gameplay pass: reads the finished geography once and makes it playable. Ranges get passes
//! so lowlands are not walled off from each other, rivers get crossings at reasonable intervals,
//! and spawn regions, resources, caves and encounters are scattered across the land. What it
//! finds is stored as `Feature`s on the map, and `validate` checks it all, so a seed that can't be
//! made playable is refused rather than shipped.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use super::climate::BiomeKind;
use super::noise::hash;
use super::{Feature, FeatureKind, WaterCell, WorldMap, NEIGHBOURS, NO_FLOW};
use crate::chunks::generate::WATER_LEVEL;

// Ground steeper than this, as height gained per block walked, is too steep to walk across.
const WALK_SLOPE: f32 = 0.7;
// A stretch of walkable ground smaller than this, in cells, is not a region worth connecting.
const MIN_REGION: usize = 5_000;
// Most passes carved to join a world's regions, a bound on a seed that is mostly mountains.
const MAX_PASSES: usize = 40;
// Cells either side of a pass's line that are graded into a valley.
const PASS_RADIUS: i64 = 3;

// Rivers narrower than this, in blocks, can be waded anywhere.
const WADE_WIDTH: f32 = 8.0;
// The widest river that gets a ford; wider ones get a bridge site.
const FORD_MAX_WIDTH: f32 = 36.0;
// A river that can't be waded gets a crossing once this many blocks of it have gone by, at the
// first good spot, and in any case by `CROSSING_MAX`.
const CROSSING_SPACING: f32 = 1_800.0;
const CROSSING_MAX: f32 = 3_000.0;

// A landmass with this many land cells or more must have somewhere to spawn on it.
const BIG_LANDMASS: usize = 100_000;
const SPAWN_SPACING: f32 = 12_000.0;
const MAX_SPAWNS: usize = 12;
// Hotspots keep at least this far from any spawn region.
const HOTSPOT_CLEARANCE: f32 = 3_500.0;
const SPAWN_RADIUS: f32 = 400.0;

fn sea() -> f32 {
    WATER_LEVEL as f32
}

// Rise per run to the steepest of a cell's four neighbours.
fn slope_layer(map: &WorldMap) -> Vec<f32> {
    let dim = map.dim();
    let cell = map.config.map_cell as f32;
    let mut slope = vec![0.0f32; dim * dim];
    super::par_rows(dim, &mut slope, |row, out| {
        for (col, s) in out.iter_mut().enumerate() {
            let h = map.height[row * dim + col];
            let mut steepest = 0.0f32;
            for (dx, dz) in [(1i64, 0i64), (-1, 0), (0, 1), (0, -1)] {
                let (x, z) = (col as i64 + dx, row as i64 + dz);
                if x >= 0 && z >= 0 && x < dim as i64 && z < dim as i64 {
                    steepest = steepest.max((h - map.height[z as usize * dim + x as usize]).abs());
                }
            }
            *s = steepest / cell;
        }
    });
    slope
}

const NONE: u32 = u32::MAX;

// Connected runs of cells where `mask` holds, 8-connected: each cell's run, or NONE, and the
// runs' sizes.
fn components(mask: &[bool], dim: usize) -> (Vec<u32>, Vec<usize>) {
    let mut comp = vec![NONE; mask.len()];
    let mut sizes = Vec::new();
    let mut stack = Vec::new();
    for start in 0..mask.len() {
        if !mask[start] || comp[start] != NONE {
            continue;
        }
        let id = sizes.len() as u32;
        let mut size = 0;
        comp[start] = id;
        stack.push(start);
        while let Some(i) = stack.pop() {
            size += 1;
            let (x, z) = ((i % dim) as i64, (i / dim) as i64);
            for (dx, dz) in NEIGHBOURS {
                let (nx, nz) = (x + dx, z + dz);
                if nx < 0 || nz < 0 || nx >= dim as i64 || nz >= dim as i64 {
                    continue;
                }
                let n = nz as usize * dim + nx as usize;
                if mask[n] && comp[n] == NONE {
                    comp[n] = id;
                    stack.push(n);
                }
            }
        }
        sizes.push(size);
    }
    (comp, sizes)
}

fn land_mask(map: &WorldMap) -> Vec<bool> {
    map.height.iter().map(|&h| h >= sea() + 2.0).collect()
}

fn walk_mask(map: &WorldMap, slope: &[f32]) -> Vec<bool> {
    map.height
        .iter()
        .zip(slope)
        .map(|(&h, &s)| h >= sea() + 2.0 && s <= WALK_SLOPE)
        .collect()
}

// Which big regions of walkable ground share a landmass without a walkable way between them:
// the first landmass's, as (landmass, the regions' ids, the biggest first), or None if there's
// none.
struct Split {
    walk: Vec<u32>,
    land: Vec<u32>,
    regions: Vec<u32>,
}

fn find_split(map: &WorldMap) -> Option<Split> {
    let dim = map.dim();
    let slope = slope_layer(map);
    let (walk, walk_sizes) = components(&walk_mask(map, &slope), dim);
    let (land, _) = components(&land_mask(map), dim);
    // each big region's landmass, from any one of its cells
    let mut landmass_of = vec![NONE; walk_sizes.len()];
    for (i, &c) in walk.iter().enumerate() {
        if c != NONE && landmass_of[c as usize] == NONE {
            landmass_of[c as usize] = land[i];
        }
    }
    let mut by_land: std::collections::BTreeMap<u32, Vec<u32>> = Default::default();
    for (region, &size) in walk_sizes.iter().enumerate() {
        if size >= MIN_REGION {
            by_land
                .entry(landmass_of[region])
                .or_default()
                .push(region as u32);
        }
    }
    by_land
        .into_values()
        .find(|regions| regions.len() >= 2)
        .map(|mut regions| {
            regions.sort_by_key(|&r| Reverse(walk_sizes[r as usize]));
            Split {
                walk,
                land,
                regions,
            }
        })
}

// The cheapest way over the land from the biggest region to any other of the split's: steep
// ground costs, high ground costs a little, so the way found goes through the lowest saddle.
fn cheapest_way(map: &WorldMap, split: &Split) -> Option<Vec<usize>> {
    let dim = map.dim();
    let slope = slope_layer(map);
    let from = split.regions[0];
    let landmass = split.land[split
        .walk
        .iter()
        .position(|&c| c == from)
        .expect("region has cells")];
    let targets = &split.regions[1..];
    let mut dist = vec![f32::INFINITY; map.height.len()];
    let mut prev = vec![NONE; map.height.len()];
    let mut heap = BinaryHeap::new();
    for (i, &c) in split.walk.iter().enumerate() {
        if c == from {
            dist[i] = 0.0;
            heap.push((Reverse(0.0f32.to_bits()), i as u32));
        }
    }
    while let Some((Reverse(bits), i)) = heap.pop() {
        let i = i as usize;
        if f32::from_bits(bits) > dist[i] {
            continue;
        }
        if split.walk[i] != NONE && targets.contains(&split.walk[i]) {
            let mut path = vec![i];
            let mut at = i;
            while prev[at] != NONE {
                at = prev[at] as usize;
                path.push(at);
            }
            path.reverse();
            return Some(path);
        }
        let (x, z) = ((i % dim) as i64, (i / dim) as i64);
        for (dx, dz) in NEIGHBOURS {
            let (nx, nz) = (x + dx, z + dz);
            if nx < 0 || nz < 0 || nx >= dim as i64 || nz >= dim as i64 {
                continue;
            }
            let n = nz as usize * dim + nx as usize;
            if split.land[n] != landmass {
                continue;
            }
            let step = if dx != 0 && dz != 0 {
                std::f32::consts::SQRT_2
            } else {
                1.0
            };
            let cost = step
                * (1.0 + 30.0 * (slope[n] - WALK_SLOPE).max(0.0) + (map.height[n] - sea()) / 120.0);
            let d = dist[i] + cost;
            if d < dist[n] {
                dist[n] = d;
                prev[n] = i as u32;
                heap.push((Reverse(d.to_bits()), n as u32));
            }
        }
    }
    None
}

// Grades the ground along `path` so it never climbs steeper than WALK_SLOPE, and opens a valley
// round it, then labels the highest point left as the pass.
fn carve(map: &mut WorldMap, path: &[usize]) {
    let dim = map.dim();
    let cell = map.config.map_cell as f32;
    let reach = WALK_SLOPE * 0.85 * cell;
    let step = |a: usize, b: usize| {
        let diagonal = (a % dim != b % dim) && (a / dim != b / dim);
        reach
            * if diagonal {
                std::f32::consts::SQRT_2
            } else {
                1.0
            }
    };
    let mut graded: Vec<f32> = path.iter().map(|&i| map.height[i]).collect();
    for k in 1..path.len() {
        graded[k] = graded[k].min(graded[k - 1] + step(path[k - 1], path[k]));
    }
    for k in (0..path.len() - 1).rev() {
        graded[k] = graded[k].min(graded[k + 1] + step(path[k], path[k + 1]));
    }
    for (k, &i) in path.iter().enumerate() {
        let (x, z) = ((i % dim) as i64, (i / dim) as i64);
        for dz in -PASS_RADIUS..=PASS_RADIUS {
            for dx in -PASS_RADIUS..=PASS_RADIUS {
                let d = ((dx * dx + dz * dz) as f32).sqrt();
                let (nx, nz) = (x + dx, z + dz);
                if d > PASS_RADIUS as f32
                    || nx < 0
                    || nz < 0
                    || nx >= dim as i64
                    || nz >= dim as i64
                {
                    continue;
                }
                let n = nz as usize * dim + nx as usize;
                let floor = graded[k] + d * reach;
                // never so low it floods
                map.height[n] = map.height[n].min(floor).max(sea() + 1.5);
            }
        }
    }
    let (top, &height) = graded
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .expect("path has cells");
    let (x, z) = map.cell_centre(path[top] % dim, path[top] / dim);
    map.features.push(Feature {
        kind: FeatureKind::Pass,
        x: x as i32,
        z: z as i32,
        value: height,
    });
}

/// Joins the lowlands a range walls off from each other by carving a pass at the lowest saddle
/// between them, one pass at a time until every big region of walkable ground on a landmass can
/// be walked to from every other.
pub fn carve_passes(map: &mut WorldMap) {
    map.features.retain(|f| f.kind != FeatureKind::Pass);
    for _ in 0..MAX_PASSES {
        let Some(split) = find_split(map) else {
            return;
        };
        let Some(path) = cheapest_way(map, &split) else {
            return;
        };
        carve(map, &path);
    }
}

fn world_of(map: &WorldMap, i: usize) -> (i32, i32) {
    let dim = map.dim();
    let (x, z) = map.cell_centre(i % dim, i / dim);
    (x as i32, z as i32)
}

fn push(map: &mut WorldMap, kind: FeatureKind, i: usize, value: f32) {
    let (x, z) = world_of(map, i);
    map.features.push(Feature { kind, x, z, value });
}

// Which river cells are crossings, given those already marked: walks every river from its
// source down, adding up the blocks of a river too wide to wade since the last crossing.
// `place` is called with each cell that needs one and the blocks since the last, or the longest
// stretch left without is returned.
fn river_gaps(
    map: &WorldMap,
    slope: &[f32],
    crossing: &mut Vec<bool>,
    mut place: Option<&mut Vec<usize>>,
) -> f32 {
    let dim = map.dim();
    let cell = map.config.map_cell as f32;
    let mut rivers: Vec<usize> = (0..map.height.len())
        .filter(|&i| map.water_kind[i] == WaterCell::River as u8)
        .collect();
    // upstream first: a river's flow only grows downstream
    rivers.sort_by(|&a, &b| map.flow[a].total_cmp(&map.flow[b]));
    let mut since = vec![0.0f32; map.height.len()];
    let mut longest = 0.0f32;
    for i in rivers {
        let width = map.river_width[i];
        let here = since[i];
        longest = longest.max(here);
        let mut onward = 0.0;
        if width >= WADE_WIDTH {
            let good = slope[i] <= 0.35;
            if !crossing[i] && (here >= CROSSING_SPACING && good || here >= CROSSING_MAX) {
                if let Some(place) = place.as_deref_mut() {
                    place.push(i);
                    crossing[i] = true;
                }
            }
            if !crossing[i] {
                let d = map.down[i];
                let diagonal =
                    d != NO_FLOW && NEIGHBOURS[d as usize].0 != 0 && NEIGHBOURS[d as usize].1 != 0;
                onward = here
                    + cell
                        * if diagonal {
                            std::f32::consts::SQRT_2
                        } else {
                            1.0
                        };
            }
        }
        if map.down[i] != NO_FLOW {
            let (dx, dz) = NEIGHBOURS[map.down[i] as usize];
            let n = ((i / dim) as i64 + dz) as usize * dim + ((i % dim) as i64 + dx) as usize;
            since[n] = since[n].max(onward);
        }
    }
    longest
}

fn crossings(map: &mut WorldMap, slope: &[f32]) {
    let mut crossing = vec![false; map.height.len()];
    let mut placed = Vec::new();
    river_gaps(map, slope, &mut crossing, Some(&mut placed));
    let dim = map.dim();
    for i in placed {
        let width = map.river_width[i];
        if width <= FORD_MAX_WIDTH {
            map.ford[i] = 1;
            push(map, FeatureKind::Ford, i, width);
        } else {
            // flat banks to build across: the ground either side is brought down to near the water
            let surface = map.water_surface[i];
            let (x, z) = ((i % dim) as i64, (i / dim) as i64);
            for dz in -3..=3i64 {
                for dx in -3..=3i64 {
                    let (nx, nz) = (x + dx, z + dz);
                    if nx < 0 || nz < 0 || nx >= dim as i64 || nz >= dim as i64 {
                        continue;
                    }
                    let n = nz as usize * dim + nx as usize;
                    if map.water_kind[n] == WaterCell::None as u8 {
                        let d = ((dx * dx + dz * dz) as f32).sqrt();
                        map.height[n] = map.height[n].min(surface + 2.0 + 0.8 * d);
                    }
                }
            }
            push(map, FeatureKind::BridgeSite, i, width);
        }
    }
}

// Cells to the nearest water of any kind, up to a bound, by a sweep out from the water.
fn water_distance(map: &WorldMap, bound: u8) -> Vec<u8> {
    let dim = map.dim();
    let mut dist = vec![bound; map.height.len()];
    let mut frontier = Vec::new();
    for (i, &k) in map.water_kind.iter().enumerate() {
        if k != 0 {
            dist[i] = 0;
            frontier.push(i);
        }
    }
    let mut next = Vec::new();
    for d in 1..bound {
        for &i in &frontier {
            let (x, z) = ((i % dim) as i64, (i / dim) as i64);
            for (dx, dz) in [(1i64, 0i64), (-1, 0), (0, 1), (0, -1)] {
                let (nx, nz) = (x + dx, z + dz);
                if nx < 0 || nz < 0 || nx >= dim as i64 || nz >= dim as i64 {
                    continue;
                }
                let n = nz as usize * dim + nx as usize;
                if dist[n] > d {
                    dist[n] = d;
                    next.push(n);
                }
            }
        }
        std::mem::swap(&mut frontier, &mut next);
        next.clear();
    }
    dist
}

// Takes candidates in an order fixed by the seed and keeps each that is `spacing` blocks from
// every one kept before it, which spreads them evenly with no pattern.
fn scatter(
    map: &WorldMap,
    mut candidates: Vec<usize>,
    spacing: f32,
    salt: u32,
    limit: usize,
    apart_from: &[(f32, f32)],
) -> Vec<usize> {
    let dim = map.dim();
    candidates.sort_by_key(|&i| hash(map.seed ^ salt, i as i32, (i >> 16) as i32));
    let bucket = |x: f32, z: f32| ((x / spacing).floor() as i64, (z / spacing).floor() as i64);
    let mut grid: std::collections::HashMap<(i64, i64), Vec<(f32, f32)>> = Default::default();
    for &(x, z) in apart_from {
        grid.entry(bucket(x, z)).or_default().push((x, z));
    }
    let mut kept = Vec::new();
    for i in candidates {
        let (x, z) = map.cell_centre(i % dim, i / dim);
        let (x, z) = (x as f32, z as f32);
        let (bx, bz) = bucket(x, z);
        let crowded = (bx - 1..=bx + 1).any(|gx| {
            (bz - 1..=bz + 1).any(|gz| {
                grid.get(&(gx, gz)).is_some_and(|near| {
                    near.iter()
                        .any(|&(nx, nz)| (nx - x).powi(2) + (nz - z).powi(2) < spacing * spacing)
                })
            })
        });
        if !crowded {
            grid.entry((bx, bz)).or_default().push((x, z));
            kept.push(i);
            if kept.len() >= limit {
                break;
            }
        }
    }
    kept
}

fn kind_at(map: &WorldMap, i: usize) -> BiomeKind {
    BiomeKind::from_u8(map.biome[i])
}

// How good a place to start the cell is, or None if it is no place to. `relaxed` takes any
// open, gentle land, for a landmass with nowhere better.
fn spawn_score(
    map: &WorldMap,
    i: usize,
    slope: f32,
    water: u8,
    shelter: f32,
    relaxed: bool,
) -> Option<f32> {
    let above = map.height[i] - sea();
    if map.water_kind[i] != 0 || !(3.0..=if relaxed { 90.0 } else { 45.0 }).contains(&above) {
        return None;
    }
    if slope > if relaxed { 0.35 } else { 0.15 } || map.trees[i] > if relaxed { 150 } else { 110 } {
        return None;
    }
    let mild = match kind_at(map, i) {
        BiomeKind::Grassland => 1.0,
        BiomeKind::Forest | BiomeKind::Savanna => 0.7,
        BiomeKind::Steppe | BiomeKind::Taiga => 0.4,
        _ if relaxed => 0.1,
        _ => return None,
    };
    let warm = (map.temperature[i] - above * super::climate::LAPSE_PER_BLOCK).clamp(0.0, 1.0);
    let temperate = 1.0 - ((warm - 0.55).abs() / 0.35).min(1.0);
    // water close by, but not so close it floods
    let wet = if (4..=24).contains(&water) { 1.0 } else { 0.0 };
    Some(2.0 * mild + temperate + 0.6 * shelter + 0.8 * wet)
}

fn spawns(map: &mut WorldMap, slope: &[f32]) {
    let dim = map.dim();
    let water = water_distance(map, 40);
    let (land, land_sizes) = components(&land_mask(map), dim);
    // sheltered: the ground round it, a kilometre or so out, stands higher
    let shelter = |i: usize| -> f32 {
        let (x, z) = ((i % dim) as i64, (i / dim) as i64);
        let ring = 30;
        let mut higher = 0.0;
        for (dx, dz) in [
            (1i64, 0i64),
            (-1, 0),
            (0, 1),
            (0, -1),
            (1, 1),
            (-1, 1),
            (1, -1),
            (-1, -1),
        ] {
            let (nx, nz) = (
                (x + dx * ring).clamp(0, dim as i64 - 1),
                (z + dz * ring).clamp(0, dim as i64 - 1),
            );
            let h = map.height[nz as usize * dim + nx as usize];
            higher += ((h - map.height[i]) / 25.0).clamp(0.0, 1.0);
        }
        higher / 8.0
    };
    let candidates: Vec<usize> = (0..dim * dim)
        .step_by(1)
        .filter(|i| (i % dim) % 6 == 0 && (i / dim) % 6 == 0)
        .collect();
    let mut kept: Vec<usize> = Vec::new();
    for relaxed in [false, true] {
        let mut scored: Vec<(f32, usize)> = candidates
            .iter()
            .filter_map(|&i| {
                spawn_score(map, i, slope[i], water[i], shelter(i), relaxed).map(|s| (s, i))
            })
            .collect();
        scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        let have_landmass: std::collections::HashSet<u32> = kept.iter().map(|&i| land[i]).collect();
        let mut taken: Vec<(f32, f32)> = kept
            .iter()
            .map(|&i| {
                let (x, z) = map.cell_centre(i % dim, i / dim);
                (x as f32, z as f32)
            })
            .collect();
        for (_, i) in scored {
            // the relaxed round only fills big landmasses the first left without
            if relaxed
                && (land_sizes[land[i] as usize] < BIG_LANDMASS
                    || have_landmass.contains(&land[i])
                    || kept.iter().any(|&k| land[k] == land[i]))
            {
                continue;
            }
            if kept.len() >= MAX_SPAWNS && !relaxed {
                break;
            }
            let (x, z) = map.cell_centre(i % dim, i / dim);
            let (x, z) = (x as f32, z as f32);
            let spacing = if relaxed {
                SPAWN_SPACING / 2.0
            } else {
                SPAWN_SPACING
            };
            if taken
                .iter()
                .all(|&(tx, tz)| (tx - x).powi(2) + (tz - z).powi(2) >= spacing * spacing)
            {
                taken.push((x, z));
                kept.push(i);
            }
        }
    }
    for i in kept {
        push(map, FeatureKind::Spawn, i, SPAWN_RADIUS);
    }
}

fn sites(map: &mut WorldMap, slope: &[f32]) {
    let dim = map.dim();
    let water = water_distance(map, 40);
    let spawn_points: Vec<(f32, f32)> = map
        .features
        .iter()
        .filter(|f| f.kind == FeatureKind::Spawn)
        .map(|f| (f.x as f32, f.z as f32))
        .collect();
    let every = |keep: &dyn Fn(usize) -> bool| -> Vec<usize> {
        (0..dim * dim)
            .filter(|i| (i % dim) % 3 == 0 && (i / dim) % 3 == 0)
            .filter(|&i| map.water_kind[i] == 0 && map.height[i] >= sea() + 2.0 && keep(i))
            .collect()
    };
    let above = |i: usize| map.height[i] - sea();
    let plans: [(FeatureKind, Vec<usize>, f32, u32); 6] = [
        (
            FeatureKind::Iron,
            every(&|i| (60.0..=320.0).contains(&above(i)) && (0.2..=0.8).contains(&slope[i])),
            4_000.0,
            0x1801,
        ),
        (
            FeatureKind::Coal,
            every(&|i| {
                (10.0..=200.0).contains(&above(i))
                    && slope[i] <= 0.5
                    && kind_at(map, i) != BiomeKind::Desert
            }),
            3_500.0,
            0xC0A1,
        ),
        (
            FeatureKind::Clay,
            every(&|i| water[i] > 0 && water[i] <= 3 && slope[i] <= 0.2 && above(i) >= 3.0),
            3_000.0,
            0xC1A7,
        ),
        (
            FeatureKind::Salt,
            every(&|i| {
                matches!(kind_at(map, i), BiomeKind::Desert | BiomeKind::Steppe) && slope[i] <= 0.15
            }),
            6_000.0,
            0x5A17,
        ),
        (
            FeatureKind::Cave,
            every(&|i| slope[i] >= 0.8 && above(i) >= 30.0),
            2_500.0,
            0xCA7E,
        ),
        (
            FeatureKind::Hotspot,
            every(&|i| above(i) >= 5.0 && kind_at(map, i) != BiomeKind::Glacier),
            6_000.0,
            0x4075,
        ),
    ];
    for (kind, candidates, spacing, salt) in plans {
        // encounters keep well away from where players start, see HOTSPOT_CLEARANCE
        let apart: Vec<(f32, f32)> = if kind == FeatureKind::Hotspot {
            let mut near = Vec::new();
            for &(x, z) in &spawn_points {
                // a ring of points stands in for "5 km from a spawn" in the spacing test
                near.push((x, z));
            }
            near
        } else {
            Vec::new()
        };
        let candidates = if kind == FeatureKind::Hotspot {
            candidates
                .into_iter()
                .filter(|&i| {
                    let (x, z) = map.cell_centre(i % dim, i / dim);
                    spawn_points
                        .iter()
                        .all(|&(sx, sz)| (sx - x as f32).hypot(sz - z as f32) >= HOTSPOT_CLEARANCE)
                })
                .collect()
        } else {
            candidates
        };
        for i in scatter(map, candidates, spacing, salt, usize::MAX, &apart) {
            push(map, kind, i, 0.0);
        }
    }
}

/// Finds the map's crossings, spawn regions, resources, caves and hotspots, and stores them.
pub fn label(map: &mut WorldMap) {
    map.features.retain(|f| f.kind == FeatureKind::Pass);
    map.ford.fill(0);
    let slope = slope_layer(map);
    crossings(map, &slope);
    let slope = slope_layer(map);
    spawns(map, &slope);
    sites(map, &slope);
}

/// What is still wrong with the map after the pass has done what it can: empty if it is playable.
pub fn validate(map: &WorldMap) -> Vec<String> {
    let mut problems = Vec::new();
    let dim = map.dim();
    if find_split(map).is_some() {
        problems.push("a range walls off lowlands with no pass through it".to_string());
    }
    let (land, sizes) = components(&land_mask(map), dim);
    let mut spawned = vec![false; sizes.len()];
    for f in map.features.iter().filter(|f| f.kind == FeatureKind::Spawn) {
        let (u, v) = map.to_cell(f.x as f64, f.z as f64);
        let (cx, cz) = (
            u.round().clamp(0.0, dim as f64 - 1.0) as usize,
            v.round().clamp(0.0, dim as f64 - 1.0) as usize,
        );
        let l = land[map.index(cx, cz)];
        if l != NONE {
            spawned[l as usize] = true;
        }
    }
    for (l, &size) in sizes.iter().enumerate() {
        if size >= BIG_LANDMASS && !spawned[l] {
            problems.push(format!("a landmass of {size} cells has nowhere to spawn"));
        }
    }
    let slope = slope_layer(map);
    let mut crossing = vec![false; map.height.len()];
    for f in map
        .features
        .iter()
        .filter(|f| matches!(f.kind, FeatureKind::Ford | FeatureKind::BridgeSite))
    {
        let (u, v) = map.to_cell(f.x as f64, f.z as f64);
        crossing[map.index(u.round() as usize, v.round() as usize)] = true;
    }
    let longest = river_gaps(map, &slope, &mut crossing, None);
    if longest > CROSSING_MAX * 1.5 {
        problems.push(format!(
            "a river runs {longest:.0} blocks without a crossing"
        ));
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worldmap::WorldConfig;

    // A sea round two plains and a ridge between them, as steep as a range: the plains are
    // walkable and the ridge is not, so one pass is needed.
    fn walled(ridge: f32) -> WorldMap {
        let config = WorldConfig {
            size: 160 * 32,
            map_cell: 32,
        };
        let dim = config.dim();
        let mut map = WorldMap::empty(1, config);
        for z in 0..dim {
            for x in 0..dim {
                let on_land = (10..150).contains(&x) && (10..150).contains(&z);
                let ridge_here = (x as f32 - 80.0).abs() < 8.0;
                // a notch in the ridge at z = 40, still too steep to walk
                let notch = if (35..45).contains(&z) { 0.6 } else { 1.0 };
                map.height[z * dim + x] = if !on_land {
                    400.0
                } else if ridge_here {
                    460.0 + (1.0 - (x as f32 - 80.0).abs() / 8.0) * ridge * notch
                } else {
                    460.0
                };
            }
        }
        map
    }

    #[test]
    fn test_a_wall_gets_a_pass_that_can_be_walked() {
        let mut map = walled(400.0);
        assert!(
            find_split(&map).is_some(),
            "the ridge should wall the plains off"
        );
        carve_passes(&mut map);
        assert!(find_split(&map).is_none(), "no way across after carving");
        let passes: Vec<_> = map
            .features
            .iter()
            .filter(|f| f.kind == FeatureKind::Pass)
            .collect();
        assert_eq!(1, passes.len());
        // through the notch, the lowest saddle, not across the highest ridge
        let (_, z) = map.to_cell(passes[0].x as f64, passes[0].z as f64);
        assert!((30.0..50.0).contains(&z), "pass at row {z}");
    }

    #[test]
    fn test_a_world_without_walls_needs_no_pass() {
        let mut map = walled(0.0);
        carve_passes(&mut map);
        assert!(map.features.is_empty());
    }

    #[test]
    fn test_scatter_keeps_its_distance_and_repeats() {
        let config = WorldConfig {
            size: 160 * 32,
            map_cell: 32,
        };
        let map = WorldMap::empty(3, config);
        let all: Vec<usize> = (0..160 * 160).collect();
        let a = scatter(&map, all.clone(), 800.0, 9, usize::MAX, &[]);
        assert_eq!(a, scatter(&map, all, 800.0, 9, usize::MAX, &[]));
        assert!(a.len() > 10);
        for (n, &i) in a.iter().enumerate() {
            for &j in &a[n + 1..] {
                let (pi, pj) = (
                    map.cell_centre(i % 160, i / 160),
                    map.cell_centre(j % 160, j / 160),
                );
                assert!((pi.0 - pj.0).hypot(pi.1 - pj.1) >= 800.0 - 1e-6);
            }
        }
    }

    #[test]
    fn test_generated_worlds_pass_validation_and_have_every_kind_of_place() {
        for seed in [1, 7] {
            let map = crate::worldmap::generate(
                seed,
                WorldConfig {
                    size: 32_768,
                    map_cell: 32,
                },
            );
            let problems = validate(&map);
            assert!(problems.is_empty(), "seed {seed}: {problems:?}");
            for kind in [
                FeatureKind::Spawn,
                FeatureKind::Iron,
                FeatureKind::Coal,
                FeatureKind::Cave,
                FeatureKind::Hotspot,
            ] {
                assert!(
                    map.features.iter().any(|f| f.kind == kind),
                    "seed {seed} has no {kind:?}"
                );
            }
        }
    }

    #[test]
    fn test_hotspots_keep_away_from_spawns() {
        let map = crate::worldmap::generate(
            7,
            WorldConfig {
                size: 32_768,
                map_cell: 32,
            },
        );
        for h in map
            .features
            .iter()
            .filter(|f| f.kind == FeatureKind::Hotspot)
        {
            for s in map.features.iter().filter(|f| f.kind == FeatureKind::Spawn) {
                assert!(((h.x - s.x) as f32).hypot((h.z - s.z) as f32) >= HOTSPOT_CLEARANCE);
            }
        }
    }
}
