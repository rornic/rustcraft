//! Where water goes: lakes fill the basins, every other cell drains downhill to a lake or the
//! sea, and cells that a lot of land drains through become rivers, carved into the ground.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use super::{WaterCell, WorldMap, NEIGHBOURS, NO_FLOW};
use ochre_base::WATER_LEVEL;

/// Square blocks of land that must drain through a cell for it to be a stream, which starts a
/// block or two wide and widens from there.
pub(crate) const RIVER_MIN_AREA: f32 = 400_000.0;
// No river is narrower, in blocks: one block wide doesn't fill with water reliably.
const MIN_RIVER_WIDTH: f32 = 2.0;

// A sortable IEEE key, including negative elevations. Raw float bits sort only positive values.
fn height_key(value: f32) -> u32 {
    let bits = value.to_bits();
    if bits & 0x8000_0000 != 0 {
        !bits
    } else {
        bits ^ 0x8000_0000
    }
}

fn key_height(key: u32) -> f32 {
    f32::from_bits(if key & 0x8000_0000 != 0 {
        key ^ 0x8000_0000
    } else {
        !key
    })
}
// Fewer cells than this in a river that never reaches water, and it is no river.
const MIN_INLAND_RIVER_CELLS: usize = 30;

// How far apart, in blocks, the hills and hollows of the noise that breaks ties in the flood are.
const TIE_WAVELENGTH: f64 = 900.0;

// Slopes (rise over run) between which ground goes from flat to steep enough for a river to start on, and how many times more catchment flat ground needs.
const FLAT_SLOPE: (f64, f64) = (0.005, 0.08);
const FLAT_START_FACTOR: f32 = 8.0;
// Height over the sea and slope between which ground is bare rock, and how much rain a cell of soil and of rock sheds, 1 being the old even share.
const ROCK_ABOVE: (f64, f64) = (60.0, 300.0);
const ROCK_SLOPE: (f64, f64) = (0.1, 0.5);
const SOIL_RUNOFF: f32 = 0.7;
const ROCK_RUNOFF: f32 = 2.5;
// How much of its rain the driest ground sheds no runoff of, and how much of the flow crossing a cell of it is lost.
const DRY_RUNOFF_LOSS: f32 = 0.9;
const DRY_FLOW_LOSS: f32 = 0.01;
// And how many cells' worth of flow the driest ground evaporates from a river crossing it, so a
// big river carries on across a desert that a stream can't.
const DRY_EVAPORATION: f32 = 8.0;
// A basin shallower than this, or with fewer cells, is drained: a flat or a pond, not worth a lake.
const MIN_LAKE_DEPTH: f32 = 4.0;
const MIN_LAKE_CELLS: usize = 300;
// Fewer cells under a lake's level than this and its basin is left a dry flat.
const MIN_POND_CELLS: usize = 4;
// How many cells' worth of flow a cell of lake loses to the air, in wet country and in the driest: a
// wet country's lakes fill to their brims and spill on, a dry one's stand lower in closed basins.
const LAKE_EVAPORATION: (f32, f32) = (0.6, 4.0);
// Rounds of draining small basins, each of which can open the next one along.
const DRAIN_ROUNDS: usize = 4;
// How much a drained basin's channel falls each cell, so it runs out of it.
const DRAIN_FALL: f32 = 0.01;
// Land under the sea's level but cut off from it is this much over it once drained or filled.
const LAND_ABOVE_SEA: f32 = 0.3;
// The shallowest a lake's cells are under its water, a whole block.
const SHALLOWEST: f32 = 1.5;
// How far over the sea's level a basin of ground under it, walled off from the sea, holds water at least.
const INLAND_SEA_ABOVE_SEA: f32 = 1.0;
const MAX_RIVER_DEPTH: f32 = 12.0;

/// Blocks across a river draining `area` square blocks of land: a stream a block or two wide where
/// it starts, ten at a few square kilometres, close to a hundred where a whole range drains.
pub fn river_width(area: f32) -> f32 {
    (1.5 * (area / RIVER_MIN_AREA).max(1.0).powf(0.9)).max(MIN_RIVER_WIDTH)
}

/// Blocks deep, at the middle of a river draining `area` square blocks of land.
pub fn river_depth(area: f32) -> f32 {
    (1.2 + 0.00125 * area.sqrt()).min(MAX_RIVER_DEPTH)
}

pub(super) const NONE: u32 = u32::MAX;

// What the priority flood works out: every cell's height once its basin is filled to its
// spill point, the cell it drains into, and the cells in the order they were reached, which is
// downstream before upstream.
pub(super) struct Flood {
    pub(super) filled: Vec<f32>,
    pub(super) down: Vec<u32>,
    pub(super) order: Vec<u32>,
}

// Barnes' priority flood: start from the sea and the map's edge and always take the lowest cell
// next, so each cell is reached from its lowest neighbour and a basin's floor is raised to its
// spill point. A cell drains into the one it was reached from, so every path ends at the sea.
/// What breaks ties in the flood, so water over flat ground, where every cell is as low as the next,
/// runs along smooth noise's valleys: in first-in first-out order it runs in dead straight lines, and
/// with each cell's tie of its own, in a jagged comb. The low bits are a hash, so no two tie.
pub(super) fn flood_ties(map: &WorldMap) -> Vec<u32> {
    let dim = map.dim();
    (0..dim * dim)
        .map(|i| {
            let (x, z) = map.cell_centre(i % dim, i / dim);
            let smooth = (ochre_base::noise::fbm(map.seed ^ 0x71E5, x, z, TIE_WAVELENGTH, 3) * 0.5
                + 0.5)
                .clamp(0.0, 1.0);
            let hash = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 48;
            (((smooth * 65_535.0) as u32) << 16) | hash as u32
        })
        .collect()
}

// The cells under the sea's level that join the map's edge, the open sea: a basin of them walled in by
// higher land is not the sea, and is filled like any other.
fn open_sea(height: &[f32], dim: usize) -> Vec<bool> {
    let sea = WATER_LEVEL as f32;
    let mut open = vec![false; height.len()];
    let mut stack = Vec::new();
    for z in 0..dim {
        for x in 0..dim {
            let i = z * dim + x;
            if (x == 0 || z == 0 || x == dim - 1 || z == dim - 1) && height[i] < sea {
                open[i] = true;
                stack.push(i);
            }
        }
    }
    while let Some(i) = stack.pop() {
        let (x, z) = ((i % dim) as i64, (i / dim) as i64);
        for (dx, dz) in NEIGHBOURS {
            let (nx, nz) = (x + dx, z + dz);
            if nx < 0 || nz < 0 || nx >= dim as i64 || nz >= dim as i64 {
                continue;
            }
            let n = nz as usize * dim + nx as usize;
            if !open[n] && height[n] < sea {
                open[n] = true;
                stack.push(n);
            }
        }
    }
    open
}

pub(super) fn priority_flood(height: &[f32], dim: usize, ties: &[u32]) -> Flood {
    let cells = height.len();
    let sea = WATER_LEVEL as f32;
    let open = open_sea(height, dim);
    let mut flood = Flood {
        filled: height.to_vec(),
        down: vec![NONE; cells],
        order: Vec::with_capacity(cells),
    };
    let mut seen = vec![false; cells];
    let mut heap = BinaryHeap::new();
    let push = |heap: &mut BinaryHeap<_>, key: f32, i: usize| {
        // Elevation first, including below sea level, then deterministic flood ties.
        heap.push((
            Reverse(((height_key(key) as u64) << 32) | ties[i] as u64),
            i as u32,
        ));
    };
    for z in 0..dim {
        for x in 0..dim {
            let i = z * dim + x;
            let edge = x == 0 || z == 0 || x == dim - 1 || z == dim - 1;
            if open[i] || edge {
                seen[i] = true;
                // open sea is never reached from the land, so only its shore needs the queue
                let shore = edge
                    || NEIGHBOURS.iter().any(|&(dx, dz)| {
                        height[(z as i64 + dz) as usize * dim + (x as i64 + dx) as usize] >= sea
                    });
                if shore {
                    push(&mut heap, height[i], i);
                }
            }
        }
    }
    while let Some((_, i)) = heap.pop() {
        let i = i as usize;
        flood.order.push(i as u32);
        let (x, z) = ((i % dim) as i64, (i / dim) as i64);
        for (dx, dz) in NEIGHBOURS {
            let (nx, nz) = (x + dx, z + dz);
            if nx < 0 || nz < 0 || nx >= dim as i64 || nz >= dim as i64 {
                continue;
            }
            let n = nz as usize * dim + nx as usize;
            if seen[n] {
                continue;
            }
            seen[n] = true;
            flood.filled[n] = height[n].max(flood.filled[i]);
            flood.down[n] = i as u32;
            push(&mut heap, flood.filled[n], n);
        }
    }
    flood
}

// A basin the flood filled: its cells, how deep it is filled, the level it spills at and its lowest cell.
pub(super) struct Basin {
    pub(super) cells: Vec<usize>,
    depth: f32,
    pub(super) spill: f32,
    floor: usize,
}

// The connected runs of cells the flood raised, each one water it filled to its spill point, and
// the basin each cell is in, or `NONE`.
fn basins(height: &[f32], flood: &Flood, dim: usize) -> (Vec<Basin>, Vec<u32>) {
    let wet = |i: usize| flood.filled[i] - height[i] > 0.25;
    let mut of = vec![NONE; height.len()];
    let mut stack = Vec::new();
    let mut basins = Vec::new();
    for start in 0..height.len() {
        if of[start] != NONE || !wet(start) {
            continue;
        }
        let id = basins.len() as u32;
        let mut basin = Basin {
            cells: Vec::new(),
            depth: 0.0,
            spill: flood.filled[start],
            floor: start,
        };
        of[start] = id;
        stack.push(start);
        while let Some(i) = stack.pop() {
            basin.cells.push(i);
            basin.depth = basin.depth.max(flood.filled[i] - height[i]);
            basin.spill = basin.spill.max(flood.filled[i]);
            if height[i] < height[basin.floor] {
                basin.floor = i;
            }
            // all eight ways round, as the flood goes, so a basin is what it filled as one
            let (x, z) = ((i % dim) as i64, (i / dim) as i64);
            for (dx, dz) in NEIGHBOURS {
                let (nx, nz) = (x + dx, z + dz);
                if nx < 0 || nz < 0 || nx >= dim as i64 || nz >= dim as i64 {
                    continue;
                }
                let n = nz as usize * dim + nx as usize;
                if of[n] == NONE && wet(n) {
                    of[n] = id;
                    stack.push(n);
                }
            }
        }
        basins.push(basin);
    }
    (basins, of)
}

// Drains basin `id`, too small to be a lake, in one go: a channel from its floor out the way its
// water would leave, each cell of it under the one before, until it reaches lower ground. It's cut no
// lower than water it runs into, which would drain too, nor under the sea, which would come in up it.
// One whose floor is under the sea already is filled to just over it instead. Returns whether it changed anything.
fn drain(height: &mut [f32], flood: &Flood, basin_of: &[u32], id: u32, basin: &Basin) -> bool {
    let sea = WATER_LEVEL as f32;
    if height[basin.floor] < sea + LAND_ABOVE_SEA {
        for &i in &basin.cells {
            height[i] = height[i].max(sea + LAND_ABOVE_SEA);
        }
        return true;
    }
    let mut level = height[basin.floor];
    let mut i = basin.floor;
    let mut cut = false;
    while flood.down[i] != NONE {
        let d = flood.down[i] as usize;
        let inside = basin_of[d] == id;
        let joins = !inside && flood.filled[d] - height[d] > 0.25;
        let mut target = (level - DRAIN_FALL).max(sea + LAND_ABOVE_SEA);
        if joins {
            target = target.max(flood.filled[d]);
        }
        if height[d] > target {
            height[d] = target;
            cut = true;
        } else if !inside {
            break;
        }
        if joins {
            break;
        }
        level = height[d];
        i = d;
    }
    cut
}

/// Cells of land draining through each cell, itself included.
pub(super) fn accumulate(flood: &Flood) -> Vec<f32> {
    let mut flow = vec![1.0f32; flood.down.len()];
    for &i in flood.order.iter().rev() {
        let d = flood.down[i as usize];
        if d != NONE {
            flow[d as usize] += flow[i as usize];
        }
    }
    flow
}

// Every cell, each before the one it drains into.
fn upstream_first(down: &[u32]) -> Vec<u32> {
    let mut inflow = vec![0u32; down.len()];
    for &d in down {
        if d != NONE {
            inflow[d as usize] += 1;
        }
    }
    let mut order: Vec<u32> = (0..down.len() as u32)
        .filter(|&i| inflow[i as usize] == 0)
        .collect();
    let mut next = 0;
    while next < order.len() {
        let d = down[order[next] as usize];
        next += 1;
        if d != NONE {
            inflow[d as usize] -= 1;
            if inflow[d as usize] == 0 {
                order.push(d);
            }
        }
    }
    order
}

// What rain does on each cell: the share of it shed as runoff, the share of the flow crossing it kept,
// what it evaporates from a river over it, and how dry it is, 0..1.
struct Rain {
    supply: Vec<f32>,
    keep: Vec<f32>,
    evaporated: Vec<f32>,
    dry: Vec<f32>,
}

// Rain runs off wet ground and soaks into dry ground, which also loses water crossing it. Bare rock
// sheds rain where soil soaks it up: high and steep ground, thin soil on stone, runs off many times
// what the lowlands' deep soil does.
fn rain(map: &WorldMap) -> Rain {
    let dim = map.dim();
    let sea = WATER_LEVEL as f32;
    let (_, moisture) = super::climate::moisture_field(map);
    let moisture = super::climate::blur_box(&moisture, dim, super::climate::FIELD_BLUR);
    let dry: Vec<f32> = moisture
        .iter()
        .map(|&m| super::climate::dryness(m))
        .collect();
    let cell_len = map.config.map_cell as f32;
    let rock: Vec<f32> = (0..map.height.len())
        .map(|i| {
            let (x, z) = (i % dim, i / dim);
            let at = |x: usize, z: usize| map.height[z * dim + x];
            let (xl, xr) = (x.saturating_sub(1), (x + 1).min(dim - 1));
            let (zl, zr) = (z.saturating_sub(1), (z + 1).min(dim - 1));
            let dx = (at(xr, z) - at(xl, z)) / ((xr - xl).max(1) as f32 * cell_len);
            let dz = (at(x, zr) - at(x, zl)) / ((zr - zl).max(1) as f32 * cell_len);
            let slope = (dx * dx + dz * dz).sqrt() as f64;
            let high = ochre_base::math::smoothstep_f64(
                ROCK_ABOVE.0,
                ROCK_ABOVE.1,
                (map.height[i] - sea) as f64,
            );
            let steep = ochre_base::math::smoothstep_f64(ROCK_SLOPE.0, ROCK_SLOPE.1, slope);
            high.max(steep) as f32
        })
        .collect();
    Rain {
        supply: dry
            .iter()
            .zip(&rock)
            .map(|(&d, &r)| {
                (1.0 - DRY_RUNOFF_LOSS * d) * (SOIL_RUNOFF + (ROCK_RUNOFF - SOIL_RUNOFF) * r)
            })
            .collect(),
        keep: dry.iter().map(|&d| 1.0 - DRY_FLOW_LOSS * d).collect(),
        evaporated: dry.iter().map(|&d| DRY_EVAPORATION * d).collect(),
        dry,
    }
}

// The level the lake in `basin` stands at, its cells `sorted` from lowest up, with `inflow` running in:
// it spreads cell by cell from the floor until the water it loses to the air from its surface is all
// that runs in, or reaches the basin's brim and spills the rest over it. Returns the level, what
// spills over the brim, and whether the lake is held under it, with no outlet of its own.
fn balance(
    basin: &Basin,
    sorted: &[usize],
    height: &[f32],
    dry: &[f32],
    inflow: f32,
) -> (f32, f32, bool) {
    let sea = WATER_LEVEL as f32;
    let floor = height[sorted[0]];
    let evaporation =
        |i: usize| LAKE_EVAPORATION.0 + (LAKE_EVAPORATION.1 - LAKE_EVAPORATION.0) * dry[i];
    let mut lost = 0.0;
    let mut level = basin.spill;
    for (k, &i) in sorted.iter().enumerate() {
        if lost + evaporation(i) > inflow {
            // between this cell's height and the last one's, as far as what's left of the water goes
            let under = if k == 0 {
                height[i]
            } else {
                height[sorted[k - 1]]
            };
            let reach = ((inflow - lost) / evaporation(i)).clamp(0.0, 1.0);
            level = under + (height[i] - under) * reach;
            break;
        }
        lost += evaporation(i);
    }
    if floor < sea {
        // ground under the sea's level walled off from it holds water at least just over the sea, or it
        // would stand dry under the sea's level, which the land never does
        level = level.max(sea + INLAND_SEA_ABOVE_SEA).min(basin.spill);
    } else if sorted.partition_point(|&i| height[i] < level) < MIN_POND_CELLS {
        level = floor;
    }
    let closed = level < basin.spill - 1e-3;
    let lost: f32 = sorted[..sorted.partition_point(|&i| height[i] < level)]
        .iter()
        .map(|&i| evaporation(i))
        .sum();
    (level, (inflow - lost).max(0.0), closed)
}

// Each basin's lake: its level, and whether it's held under its basin's brim, with no outlet.
struct Lakes {
    level: Vec<f32>,
    held: Vec<bool>,
}

// Flow through every cell, with each basin's lake taking what it loses to the air from what passes through
// it: what drains into a basin is gathered at the cells it leaves by, and once all of it's in, the lake's
// level is worked out and what spills over its brim goes on.
fn flow_through(
    down: &[u32],
    basins: &[Basin],
    sorted: &[Vec<usize>],
    basin_of: &[u32],
    height: &[f32],
    rain: &Rain,
) -> (Vec<f32>, Lakes) {
    let mut flow = rain.supply.clone();
    let mut exits = vec![0usize; basins.len()];
    for (i, &d) in down.iter().enumerate() {
        let b = basin_of[i];
        if b != NONE && d != NONE && basin_of[d as usize] != b {
            exits[b as usize] += 1;
        }
    }
    let mut gathered = vec![0.0f32; basins.len()];
    let mut lakes = Lakes {
        level: basins.iter().map(|b| height[b.floor]).collect(),
        held: vec![true; basins.len()],
    };
    for i in upstream_first(down) {
        let i = i as usize;
        let d = down[i];
        if d == NONE {
            continue;
        }
        let d = d as usize;
        let b = basin_of[i];
        if b == NONE {
            flow[d] += (flow[i] * rain.keep[i] - rain.evaporated[i]).max(0.0);
        } else if basin_of[d] == b {
            // the lake's balance counts what it loses
            flow[d] += flow[i];
        } else {
            let b = b as usize;
            gathered[b] += flow[i];
            exits[b] -= 1;
            if exits[b] == 0 {
                let (level, out, held) =
                    balance(&basins[b], &sorted[b], height, &rain.dry, gathered[b]);
                (lakes.level[b], lakes.held[b]) = (level, held);
                if !held {
                    flow[d] += out;
                }
            }
        }
    }
    (flow, lakes)
}

// Inside a basin whose lake doesn't fill it, water runs down to the lake, not out over the brim the
// flood found: a flood from the lake, or the floor of a basin with none, out over the basin's cells,
// each draining into the one it was reached from, and the lake's own cells into nothing.
fn reroute(
    down: &mut [u32],
    height: &[f32],
    basin: &Basin,
    sorted: &[usize],
    level: f32,
    basin_of: &[u32],
    dim: usize,
) {
    let id = basin_of[basin.floor];
    let mut seen = std::collections::HashSet::new();
    let mut heap = BinaryHeap::new();
    let lake = sorted.partition_point(|&i| height[i] < level).max(1);
    for &i in &sorted[..lake] {
        down[i] = NONE;
        seen.insert(i);
        heap.push((Reverse(height_key(height[i].max(level))), i));
    }
    while let Some((Reverse(bits), i)) = heap.pop() {
        let filled = key_height(bits);
        let (x, z) = ((i % dim) as i64, (i / dim) as i64);
        for (dx, dz) in NEIGHBOURS {
            let (nx, nz) = (x + dx, z + dz);
            if nx < 0 || nz < 0 || nx >= dim as i64 || nz >= dim as i64 {
                continue;
            }
            let n = nz as usize * dim + nx as usize;
            if basin_of[n] != id || !seen.insert(n) {
                continue;
            }
            down[n] = i as u32;
            heap.push((Reverse(height_key(height[n].max(filled))), n));
        }
    }
}

/// Scatters the ground by up to a block or so before the last `run`, so water on gentle ground winds
/// instead of running in the flood's straight lines. It stays in the ground, so the lakes the flood
/// finds and the ground they lie in agree.
pub fn scatter(map: &mut WorldMap) {
    let dim = map.dim();
    for i in 0..map.height.len() {
        let (x, z) = map.cell_centre(i % dim, i / dim);
        map.height[i] += (ochre_base::noise::fbm(map.seed ^ 0x7E1D, x, z, 260.0, 3) * 0.9) as f32
            + ochre_base::noise::unit(map.seed ^ 0x7E1E, i as i32, (i >> 16) as i32) as f32 * 0.05;
    }
}

/// Works out lakes, drainage and rivers from the height layer: small basins are drained, the rest
/// hold lakes as high as the water running into them keeps them against what they lose to the air,
/// and with `carve`, the rivers' beds are cut into the ground.
pub fn run(map: &mut WorldMap, carve: bool) {
    let dim = map.dim();
    let sea = WATER_LEVEL as f32;
    let ties = flood_ties(map);
    let mut flood = priority_flood(&map.height, dim, &ties);
    let (mut lakes, mut basin_of) = basins(&map.height, &flood, dim);
    for _ in 0..DRAIN_ROUNDS {
        let mut cut = false;
        for (id, basin) in lakes.iter().enumerate() {
            if basin.cells.len() < MIN_LAKE_CELLS || basin.depth < MIN_LAKE_DEPTH {
                cut |= drain(&mut map.height, &flood, &basin_of, id as u32, basin);
            }
        }
        if !cut {
            break;
        }
        flood = priority_flood(&map.height, dim, &ties);
        (lakes, basin_of) = basins(&map.height, &flood, dim);
    }
    let height = map.height.clone();
    let sorted: Vec<Vec<usize>> = lakes
        .iter()
        .map(|b| {
            let mut cells = b.cells.clone();
            cells.sort_by(|&a, &b| height[a].total_cmp(&height[b]));
            cells
        })
        .collect();
    let rain = rain(map);
    let (_, held) = flow_through(&flood.down, &lakes, &sorted, &basin_of, &height, &rain);
    let level = held.level;
    let mut down = flood.down.clone();
    for (b, basin) in lakes.iter().enumerate() {
        if held.held[b] {
            reroute(
                &mut down, &height, basin, &sorted[b], level[b], &basin_of, dim,
            );
        }
    }
    let (flow, _) = flow_through(&down, &lakes, &sorted, &basin_of, &height, &rain);
    let order = upstream_first(&down);

    let open = open_sea(&height, dim);
    let mut kind = vec![WaterCell::None as u8; height.len()];
    let mut surface = vec![0.0f32; height.len()];
    for (i, &o) in open.iter().enumerate() {
        if o {
            kind[i] = WaterCell::Ocean as u8;
            surface[i] = sea;
        }
    }
    for (b, basin) in lakes.iter().enumerate() {
        for &i in &basin.cells {
            if height[i] < level[b] {
                kind[i] = WaterCell::Lake as u8;
                surface[i] = level[b];
                // a lake's water is drawn a whole block, under its level: a bed between the two would
                // stand out of it, parting the lake into pools where its cells only just hold water
                if carve {
                    map.height[i] = map.height[i].min(level[b].floor() - SHALLOWEST);
                }
            }
        }
    }

    // A river starts only where the ground is steep enough to hold a channel for the water it
    // drains: flat lowlands need many times the catchment, so rivers rise in hills and mountains
    // and not out of the plain, and once one has started it carries on across flat ground.
    let cell_len = map.config.map_cell as f32;
    let cell_area = cell_len * cell_len;
    let mut is_river = vec![false; height.len()];
    let mut fed = vec![false; height.len()];
    for &i in &order {
        let (i, d) = (i as usize, down[i as usize]);
        if d == NONE {
            continue;
        }
        let d = d as usize;
        let diagonal = i % dim != d % dim && i / dim != d / dim;
        let run = cell_len
            * if diagonal {
                std::f32::consts::SQRT_2
            } else {
                1.0
            };
        let slope = (height[i] - height[d]).max(0.0) / run;
        let flat =
            1.0 - ochre_base::math::smoothstep_f64(FLAT_SLOPE.0, FLAT_SLOPE.1, slope as f64) as f32;
        let area = flow[i] * cell_area;
        if area >= RIVER_MIN_AREA * (1.0 + FLAT_START_FACTOR * flat)
            || (fed[i] && area >= RIVER_MIN_AREA)
        {
            is_river[i] = true;
            fed[d] = true;
        }
    }
    // a short river that dries up before reaching water, with nothing feeding it, is a scrap of
    // stream out of nowhere: it goes
    let mut end = vec![usize::MAX; height.len()];
    let mut size = vec![0usize; height.len()];
    for &i in order.iter().rev() {
        let i = i as usize;
        if !is_river[i] {
            continue;
        }
        let d = down[i];
        end[i] = if d != NONE && is_river[d as usize] {
            end[d as usize]
        } else {
            i
        };
        size[end[i]] += 1;
    }
    for i in 0..height.len() {
        if !is_river[i] || size[end[i]] >= MIN_INLAND_RIVER_CELLS {
            continue;
        }
        let d = down[end[i]];
        let into_water = d != NONE
            && matches!(
                WaterCell::from_u8(kind[d as usize]),
                WaterCell::Ocean | WaterCell::Lake
            );
        if !into_water {
            is_river[i] = false;
        }
    }

    map.down = down
        .iter()
        .enumerate()
        .map(|(i, &d)| {
            if d == NONE {
                return NO_FLOW;
            }
            let (dx, dz) = (
                (d as usize % dim) as i64 - (i % dim) as i64,
                (d as usize / dim) as i64 - (i / dim) as i64,
            );
            NEIGHBOURS.iter().position(|&n| n == (dx, dz)).unwrap() as u8
        })
        .collect();

    let mut width = vec![0.0f32; height.len()];
    // downstream first, so each river cell knows the water it flows into
    for &i in order.iter().rev() {
        let i = i as usize;
        if kind[i] != WaterCell::None as u8 || !is_river[i] || down[i] == NONE {
            continue;
        }
        let depth = river_depth(flow[i] * cell_area);
        let (x, z) = ((i % dim) as i64, (i / dim) as i64);
        // the lowest ground beside the river, so the banks stand above the water
        let bank = NEIGHBOURS
            .iter()
            .filter(|&&(dx, dz)| {
                (0..dim as i64).contains(&(x + dx)) && (0..dim as i64).contains(&(z + dz))
            })
            .map(|&(dx, dz)| height[((z + dz) as usize) * dim + (x + dx) as usize])
            .fold(flood.filled[i].max(height[i]), f32::min);
        let into = surface[down[i] as usize];
        let bed = (bank - depth).max(into - depth);
        kind[i] = WaterCell::River as u8;
        surface[i] = bed + depth;
        width[i] = river_width(flow[i] * cell_area);
        if carve {
            // the cell dips only as far as the river fills it: a stream a few blocks
            // wide is cut at block scale, see `worldmap::source`, and digging all of its
            // 32 m cell down to the bed would leave a dry crevice beside the water
            let filled = (width[i] / map.config.map_cell as f32).min(1.0);
            // only ever cut: a bed held up by the water downstream must not raise the land; and not
            // under the lake or sea beside it, which would spill into the cell's whole square
            let held = super::water::still_beside(dim, &kind, &surface, i);
            map.height[i] = (bed + depth * (1.0 - filled)).max(held).min(map.height[i]);
        }
    }
    map.flow = flow;
    map.water_kind = kind;
    map.water_surface = surface;
    map.river_width = width;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worldmap::WorldConfig;

    #[test]
    fn signed_height_keys_sort_and_round_trip() {
        let heights = [-1008.0, -20.0, -1.0, -0.0, 0.0, 0.5, 1000.0];
        let keys = heights.map(height_key);
        assert!(keys.windows(2).all(|p| p[0] < p[1]));
        for h in heights {
            assert_eq!(key_height(height_key(h)).to_bits(), h.to_bits());
        }
    }

    #[test]
    fn negative_ocean_shores_do_not_flood_lower_land_from_a_higher_shore() {
        let dim = 5;
        let mut heights = vec![100.0; dim * dim];
        for z in 0..dim {
            for x in 0..dim {
                if x == 0 || z == 0 || x + 1 == dim || z + 1 == dim {
                    heights[z * dim + x] = -20.0;
                }
            }
        }
        heights[0] = -1.0;
        heights[6] = 80.0;
        heights[12] = 5.0;
        heights[13] = 1.0;
        let ties = vec![0; heights.len()];
        let flood = priority_flood(&heights, dim, &ties);
        assert_eq!(flood.filled[12], 5.0);
        assert_eq!(flood.filled[13], 1.0);
        let shifted: Vec<_> = heights.iter().map(|h| h + 100.0).collect();
        let other = priority_flood(&shifted, dim, &ties);
        for i in 0..heights.len() {
            assert_eq!(flood.filled[i] + 100.0, other.filled[i]);
        }
    }

    // A bowl with a rim, a notch in the rim, and a slope to the sea beyond it.
    fn valley() -> WorldMap {
        let config = WorldConfig {
            size: 64 * 32,
            map_cell: 32,
            ..Default::default()
        };
        let dim = config.dim();
        let mut map = WorldMap::empty(1, config);
        for z in 0..dim {
            for x in 0..dim {
                let (fx, fz) = (x as f32 - 32.0, z as f32 - 32.0);
                let r = (fx * fx + fz * fz).sqrt();
                // a basin in the middle, the rim at radius 12, sloping down to the sea outside
                let h = if r < 12.0 {
                    470.0 + (12.0 - r) * -1.5 + 12.0
                } else {
                    482.0 - (r - 12.0) * 2.0
                };
                map.height[z * dim + x] = h.max(400.0) - 448.0;
            }
        }
        map
    }

    #[test]
    fn test_rivers_are_cut_into_the_land_and_never_raise_it() {
        let config = WorldConfig {
            size: 32_768,
            map_cell: 32,
            ..Default::default()
        };
        let mut map = crate::worldmap::tectonics::generate(7, config);
        run(&mut map, false);
        scatter(&mut map);
        let before = map.height.clone();
        run(&mut map, true);
        let mut rivers = 0;
        for (i, (&was, &now)) in before.iter().zip(&map.height).enumerate() {
            if map.water_kind[i] == WaterCell::River as u8 {
                rivers += 1;
                assert!(
                    now <= was + 1e-3,
                    "river cell {i} raised from {was} to {now}"
                );
            }
        }
        assert!(rivers > 50);
    }

    #[test]
    fn test_basins_fill_to_lakes() {
        let mut map = valley();
        let dim = map.dim();
        // an enclosed basin: floor 16, rim 34
        run(&mut map, true);
        let middle = 32 * dim + 32;
        assert_eq!(WaterCell::Lake as u8, map.water_kind[middle]);
        assert!(map.water_surface[middle] > map.height[middle] + 5.0);
    }

    #[test]
    fn test_ground_under_the_sea_walled_in_holds_a_lake_over_it() {
        // a bowl too big and deep to be a lake, its rim too thick to breach, dipping under the sea in its middle
        let config = WorldConfig {
            size: 340 * 32,
            map_cell: 32,
            ..Default::default()
        };
        let dim = config.dim();
        let mut map = WorldMap::empty(1, config);
        for z in 0..dim {
            for x in 0..dim {
                let r = ((x as f32 - 170.0).powi(2) + (z as f32 - 170.0).powi(2)).sqrt();
                map.height[z * dim + x] = match r {
                    r if r < 145.0 => -18.0 + r * 0.4,
                    r if r < 160.0 => 252.0,
                    r => (252.0 - (r - 160.0) * 30.0).max(-48.0),
                };
            }
        }
        run(&mut map, true);
        let middle = 170 * dim + 170;
        assert_eq!(WaterCell::Lake as u8, map.water_kind[middle]);
        // at least just over the sea, and no higher than its brim
        assert!(map.water_surface[middle] >= WATER_LEVEL as f32 + INLAND_SEA_ABOVE_SEA);
        assert!(map.water_surface[middle] < 252.0);
        // and no water stands over the dry ground beside it, which would end at the cells' edges, past the scatter the flood sees
        for i in 0..dim * dim {
            if map.water_kind[i] != WaterCell::Lake as u8 {
                continue;
            }
            let (x, z) = (i % dim, i / dim);
            for n in [i - 1, i + 1, i - dim, i + dim] {
                if x > 0
                    && x + 1 < dim
                    && z > 0
                    && z + 1 < dim
                    && map.water_kind[n] == WaterCell::None as u8
                {
                    assert!(
                        map.height[n] > map.water_surface[i] - 2.0,
                        "lake at {i} over dry {n}"
                    );
                }
            }
        }
    }

    #[test]
    fn test_water_all_drains_to_the_sea() {
        let mut map = valley();
        run(&mut map, true);
        let dim = map.dim();
        for start in 0..map.height.len() {
            let mut i = start;
            for _ in 0..4 * dim {
                match map.down[i] {
                    NO_FLOW => break,
                    d => {
                        let (dx, dz) = NEIGHBOURS[d as usize];
                        i = ((i / dim) as i64 + dz) as usize * dim
                            + ((i % dim) as i64 + dx) as usize;
                    }
                }
            }
            assert_eq!(NO_FLOW, map.down[i], "cell {start} never reaches an outlet");
        }
    }

    #[test]
    fn test_rivers_reach_water_or_dry_up_in_dry_ground() {
        let config = WorldConfig {
            size: 32_768,
            map_cell: 32,
            ..Default::default()
        };
        let mut map = crate::worldmap::tectonics::generate(7, config);
        run(&mut map, false);
        run(&mut map, true);
        let dim = map.dim();
        let (_, moisture) = crate::worldmap::climate::moisture_field(&map);
        let moisture = crate::worldmap::climate::blur_box(
            &moisture,
            dim,
            crate::worldmap::climate::FIELD_BLUR,
        );
        let mut rivers = 0;
        for start in 0..map.height.len() {
            if map.water_kind[start] != WaterCell::River as u8 {
                continue;
            }
            rivers += 1;
            let mut i = start;
            let mut steps = 0;
            while map.water_kind[i] == WaterCell::River as u8 {
                let d = map.down[i];
                assert_ne!(NO_FLOW, d, "river cell {i} drains nowhere");
                let (dx, dz) = NEIGHBOURS[d as usize];
                let next =
                    ((i / dim) as i64 + dz) as usize * dim + ((i % dim) as i64 + dx) as usize;
                if map.water_kind[next] == WaterCell::River as u8 {
                    // a river only narrows by losing water crossing dry ground
                    assert!(
                        map.river_width[next] >= map.river_width[i]
                            // (the moisture here is of the final ground, a little off the one rivers ran on)
                            || crate::worldmap::climate::dryness(moisture[i] - 0.1) > 0.0,
                        "river narrows in wet ground at {i}: {} to {}, moisture {}",
                        map.river_width[i],
                        map.river_width[next],
                        moisture[i]
                    );
                    assert!(
                        map.water_surface[next] <= map.water_surface[i] + 1e-3,
                        "river runs uphill at {i}"
                    );
                }
                i = next;
                steps += 1;
                assert!(steps < 4 * dim, "river loops");
            }
            // it runs into water, or dries up in dry ground, where its last cell is
            let wet_end = matches!(
                WaterCell::from_u8(map.water_kind[i]),
                WaterCell::Ocean | WaterCell::Lake
            );
            assert!(
                wet_end || crate::worldmap::climate::dryness(moisture[i] - 0.1) > 0.0,
                "river ends on wet land at {i}"
            );
        }
        assert!(rivers > 50, "only {rivers} river cells");
    }
}
