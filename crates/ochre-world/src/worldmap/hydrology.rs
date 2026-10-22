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
// A basin shallower than this is no lake, just a flat.
const MIN_LAKE_DEPTH: f32 = 4.0;
// Fewer cells than this is a pond, not worth a lake.
const MIN_LAKE_CELLS: usize = 300;
// A deeper basin is cut down to this depth, and one wider than MAX_LAKE_CELLS is drained, as a
// range's rings would otherwise hold seas. Deep enough that the flood, not this, sets a lake's depth.
const MAX_LAKE_DEPTH: f32 = 150.0;
const MAX_LAKE_CELLS: usize = 60_000;
// Rounds of cutting small basins' rims, each of which can open the next basin along.
const BREACH_ROUNDS: usize = 6;
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
        // positive floats order as their bits do, then ties go by `ties`
        heap.push((
            Reverse(((key.to_bits() as u64) << 32) | ties[i] as u64),
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

// A basin the flood filled: its cells, how deep it is filled and where its floor is.
struct Basin {
    cells: Vec<usize>,
    depth: f32,
    floor: usize,
}

impl Basin {
    // Big and deep enough to stay a lake; a smaller one is drained by cutting its rim.
    fn is_lake(&self) -> bool {
        (MIN_LAKE_DEPTH..=MAX_LAKE_DEPTH).contains(&self.depth)
            && (MIN_LAKE_CELLS..=MAX_LAKE_CELLS).contains(&self.cells.len())
    }

    // How low to cut the rim: to a lake of the deepest allowed, or down to the floor to drain it.
    fn rim_target(&self, floor: f32) -> f32 {
        if self.depth > MAX_LAKE_DEPTH && self.cells.len() <= MAX_LAKE_CELLS {
            floor + MAX_LAKE_DEPTH
        } else {
            floor
        }
    }
}

// The connected runs of cells the flood raised, each flooded from one of its cells.
fn basins(height: &[f32], flood: &Flood, dim: usize) -> Vec<Basin> {
    let wet = |i: usize| flood.filled[i] - height[i] > 0.25;
    let mut seen = vec![false; height.len()];
    let mut stack = Vec::new();
    let mut basins = Vec::new();
    for start in 0..height.len() {
        if seen[start] || !wet(start) {
            continue;
        }
        let mut basin = Basin {
            cells: Vec::new(),
            depth: 0.0,
            floor: start,
        };
        seen[start] = true;
        stack.push(start);
        while let Some(i) = stack.pop() {
            basin.cells.push(i);
            basin.depth = basin.depth.max(flood.filled[i] - height[i]);
            if height[i] < height[basin.floor] {
                basin.floor = i;
            }
            let (x, z) = (i % dim, i / dim);
            let around = [
                (x > 0).then(|| i - 1),
                (x + 1 < dim).then(|| i + 1),
                (z > 0).then(|| i - dim),
                (z + 1 < dim).then(|| i + dim),
            ];
            for n in around.into_iter().flatten() {
                if !seen[n] && wet(n) {
                    seen[n] = true;
                    stack.push(n);
                }
            }
        }
        basins.push(basin);
    }
    basins
}

// Lowers the rim a basin spills over, see `Basin::rim_target`.
// Returns whether it changed anything.
fn breach(height: &mut [f32], flood: &Flood, basin: &Basin) -> bool {
    let mut i = basin.floor;
    for _ in 0..basin.cells.len() + 2 {
        let d = flood.down[i];
        if d == NONE {
            return false;
        }
        let wet = flood.filled[i] - height[i] > 0.25;
        if !wet {
            // the rim: no lower than the ground it drains on to, nor the basin's floor
            let floor = basin
                .rim_target(height[basin.floor])
                .max(flood.filled[d as usize]);
            if height[i] > floor {
                height[i] = floor;
                return true;
            }
            return false;
        }
        i = d as usize;
    }
    false
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

/// The same, with each cell's own rain `supply` and the share `keep` of a cell's flow that gets
/// through it to the next, less an `evaporated` amount: dry ground sheds little and loses what crosses it, so a river that
/// leaves wet country thins out in a desert and may dry up, with every cell downstream of it.
pub(super) fn accumulate_runoff(
    flood: &Flood,
    supply: &[f32],
    keep: &[f32],
    evaporated: &[f32],
) -> Vec<f32> {
    let mut flow = supply.to_vec();
    for &i in flood.order.iter().rev() {
        let d = flood.down[i as usize];
        if d != NONE {
            flow[d as usize] +=
                (flow[i as usize] * keep[i as usize] - evaporated[i as usize]).max(0.0);
        }
    }
    flow
}

// How far over the sea's level a walled-in basin of ground under it is filled: a lake's, which looks like one.
const INLAND_SEA_ABOVE_SEA: f32 = 1.0;

/// Works out lakes, drainage and rivers from the height layer, cutting open basins that are too
/// small or big to be lakes and, with `carve`, the rivers' beds into the ground.
pub fn run(map: &mut WorldMap, carve: bool) {
    let dim = map.dim();
    let sea = WATER_LEVEL as f32;
    let mut original = map.height.clone();
    // Scatter of up to a block or so, only for working out where water goes: on gentle ground the
    // flood's order would send it in straight lines, and this makes its way wind. Taken out of the
    // heights again below.
    let scatter: Vec<f32> = (0..original.len())
        .map(|i| {
            let (x, z) = map.cell_centre(i % dim, i / dim);
            (ochre_base::noise::fbm(map.seed ^ 0x7E1D, x, z, 260.0, 3) * 0.9) as f32
                + ochre_base::noise::unit(map.seed ^ 0x7E1E, i as i32, (i >> 16) as i32) as f32
                    * 0.05
        })
        .collect();
    for (h, s) in original.iter_mut().zip(&scatter) {
        *h += s;
    }
    let ties = flood_ties(map);
    let mut flood = priority_flood(&original, dim, &ties);
    let mut lakes = basins(&original, &flood, dim);
    for _ in 0..BREACH_ROUNDS {
        let mut cut = false;
        for basin in lakes.iter().filter(|b| !b.is_lake()) {
            cut |= breach(&mut original, &flood, basin);
        }
        if !cut {
            break;
        }
        flood = priority_flood(&original, dim, &ties);
        lakes = basins(&original, &flood, dim);
    }
    // the cuts stay in the ground
    for ((m, o), s) in map.height.iter_mut().zip(&original).zip(&scatter) {
        *m = o - s;
    }

    let mut kind = vec![WaterCell::None as u8; original.len()];
    for (i, &h) in original.iter().enumerate() {
        if h < sea {
            kind[i] = WaterCell::Ocean as u8;
        }
    }
    for cell in lakes.iter().filter(|b| b.is_lake()).flat_map(|b| &b.cells) {
        kind[*cell] = WaterCell::Lake as u8;
    }
    // ground under the sea's level that the sea doesn't reach, a basin walled in by higher land, is a
    // lake, not an inland sea: it stands just over the sea's level, which is a lake's look
    let mut walled = Vec::new();
    let mut seen = vec![false; kind.len()];
    for start in 0..kind.len() {
        if kind[start] != WaterCell::Ocean as u8 || seen[start] {
            continue;
        }
        let (mut stack, mut cells, mut edge) = (vec![start], Vec::new(), false);
        seen[start] = true;
        while let Some(i) = stack.pop() {
            cells.push(i);
            let (x, z) = ((i % dim) as i64, (i / dim) as i64);
            edge |= x == 0 || z == 0 || x == dim as i64 - 1 || z == dim as i64 - 1;
            for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let (nx, nz) = (x + dx, z + dz);
                if !(0..dim as i64).contains(&nx) || !(0..dim as i64).contains(&nz) {
                    continue;
                }
                let j = nz as usize * dim + nx as usize;
                if kind[j] == WaterCell::Ocean as u8 && !seen[j] {
                    seen[j] = true;
                    stack.push(j);
                }
            }
        }
        if !edge {
            for &i in &cells {
                kind[i] = WaterCell::Lake as u8;
            }
            walled.extend(cells);
        }
    }

    // rain runs off wet ground and soaks into dry ground, which also loses water crossing it
    let (_, moisture) = super::climate::moisture_field(map);
    let moisture = super::climate::blur_box(&moisture, dim, super::climate::FIELD_BLUR);
    let dry: Vec<f32> = moisture
        .iter()
        .map(|&m| super::climate::dryness(m))
        .collect();
    // bare rock sheds rain where soil soaks it up: high and steep ground, thin soil on stone, runs
    // off many times what the lowlands' deep soil does
    let cell_len = map.config.map_cell as f32;
    let rock: Vec<f32> = (0..original.len())
        .map(|i| {
            let (x, z) = (i % dim, i / dim);
            let at = |x: usize, z: usize| original[z * dim + x];
            let (xl, xr) = (x.saturating_sub(1), (x + 1).min(dim - 1));
            let (zl, zr) = (z.saturating_sub(1), (z + 1).min(dim - 1));
            let dx = (at(xr, z) - at(xl, z)) / ((xr - xl).max(1) as f32 * cell_len);
            let dz = (at(x, zr) - at(x, zl)) / ((zr - zl).max(1) as f32 * cell_len);
            let slope = (dx * dx + dz * dz).sqrt() as f64;
            let high = ochre_base::math::smoothstep_f64(
                ROCK_ABOVE.0,
                ROCK_ABOVE.1,
                (original[i] - sea) as f64,
            );
            let steep = ochre_base::math::smoothstep_f64(ROCK_SLOPE.0, ROCK_SLOPE.1, slope);
            high.max(steep) as f32
        })
        .collect();
    let supply: Vec<f32> = dry
        .iter()
        .zip(&rock)
        .map(|(&d, &r)| {
            (1.0 - DRY_RUNOFF_LOSS * d) * (SOIL_RUNOFF + (ROCK_RUNOFF - SOIL_RUNOFF) * r)
        })
        .collect();
    let keep: Vec<f32> = dry.iter().map(|&d| 1.0 - DRY_FLOW_LOSS * d).collect();
    let evaporated: Vec<f32> = dry.iter().map(|&d| DRY_EVAPORATION * d).collect();
    let flow = accumulate_runoff(&flood, &supply, &keep, &evaporated);
    // A river starts only where the ground is steep enough to hold a channel for the water it
    // drains: flat lowlands need many times the catchment, so rivers rise in hills and mountains
    // and not out of the plain, and once one has started it carries on across flat ground.
    let cell_area = cell_len * cell_len;
    let mut is_river = vec![false; original.len()];
    let mut fed = vec![false; original.len()];
    for &i in flood.order.iter().rev() {
        let (i, d) = (i as usize, flood.down[i as usize]);
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
        let slope = (original[i] - original[d]).max(0.0) / run;
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
    let mut end = vec![usize::MAX; original.len()];
    let mut size = vec![0usize; original.len()];
    for &i in &flood.order {
        let i = i as usize;
        if !is_river[i] {
            continue;
        }
        let d = flood.down[i];
        end[i] = if d != NONE && is_river[d as usize] {
            end[d as usize]
        } else {
            i
        };
        size[end[i]] += 1;
    }
    for i in 0..original.len() {
        if !is_river[i] || size[end[i]] >= MIN_INLAND_RIVER_CELLS {
            continue;
        }
        let e = end[i];
        let d = flood.down[e];
        let into_water = d != NONE
            && matches!(
                WaterCell::from_u8(kind[d as usize]),
                WaterCell::Ocean | WaterCell::Lake
            );
        if !into_water {
            is_river[i] = false;
        }
    }

    map.down = flood
        .down
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

    let cell_area = (map.config.map_cell * map.config.map_cell) as f32;
    let mut surface = vec![0.0f32; original.len()];
    let mut width = vec![0.0f32; original.len()];
    // downstream first, so each river cell knows the water it flows into
    for &i in &flood.order {
        let i = i as usize;
        match WaterCell::from_u8(kind[i]) {
            WaterCell::Ocean => surface[i] = sea,
            WaterCell::Lake => surface[i] = flood.filled[i],
            _ if is_river[i] => {
                let depth = river_depth(flow[i] * cell_area);
                let (x, z) = ((i % dim) as i64, (i / dim) as i64);
                // the lowest ground beside the river, so the banks stand above the water
                let bank = NEIGHBOURS
                    .iter()
                    .filter(|&&(dx, dz)| {
                        (0..dim as i64).contains(&(x + dx)) && (0..dim as i64).contains(&(z + dz))
                    })
                    .map(|&(dx, dz)| original[((z + dz) as usize) * dim + (x + dx) as usize])
                    .fold(flood.filled[i], f32::min);
                let into = surface[flood.down[i] as usize];
                let bed = (bank - depth).max(into - depth);
                kind[i] = WaterCell::River as u8;
                surface[i] = bed + depth;
                width[i] = river_width(flow[i] * cell_area);
                if carve {
                    // the cell dips only as far as the river fills it: a stream a few blocks
                    // wide is cut at block scale, see `worldmap::source`, and digging all of its
                    // 32 m cell down to the bed would leave a dry crevice beside the water
                    let filled = (width[i] / map.config.map_cell as f32).min(1.0);
                    // only ever cut: a bed held up by the water downstream must not raise the land
                    map.height[i] = (bed + depth * (1.0 - filled)).min(map.height[i]);
                }
            }
            _ => {}
        }
    }
    for &i in &walled {
        surface[i] = surface[i].max(sea + INLAND_SEA_ABOVE_SEA);
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

    // A bowl with a rim, a notch in the rim, and a slope to the sea beyond it.
    fn valley() -> WorldMap {
        let config = WorldConfig {
            size: 64 * 32,
            map_cell: 32,
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
                map.height[z * dim + x] = h.max(400.0);
            }
        }
        map
    }

    #[test]
    fn test_rivers_are_cut_into_the_land_and_never_raise_it() {
        let config = WorldConfig {
            size: 32_768,
            map_cell: 32,
        };
        let mut map = crate::worldmap::terrain::base_terrain(7, config);
        run(&mut map, false);
        let before = map.height.clone();
        run(&mut map, true);
        let mut rivers = 0;
        for (i, (&was, &now)) in before.iter().zip(&map.height).enumerate() {
            if map.water_kind[i] == WaterCell::River as u8 {
                rivers += 1;
                assert!(now <= was + 1e-3, "river cell {i} raised from {was} to {now}");
            }
        }
        assert!(rivers > 50);
    }

    #[test]
    fn test_basins_fill_to_lakes() {
        let mut map = valley();
        let dim = map.dim();
        // an enclosed basin: floor 464, rim 482
        run(&mut map, true);
        let middle = 32 * dim + 32;
        assert_eq!(WaterCell::Lake as u8, map.water_kind[middle]);
        assert!(map.water_surface[middle] > map.height[middle] + 5.0);
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
        };
        let mut map = crate::worldmap::terrain::base_terrain(7, config);
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
                            || crate::worldmap::climate::dryness(moisture[i] - 0.05) > 0.0,
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
                wet_end || crate::worldmap::climate::dryness(moisture[i] - 0.05) > 0.0,
                "river ends on wet land at {i}"
            );
        }
        assert!(rivers > 50, "only {rivers} river cells");
    }
}
