//! Climate from geography: warmth falls with latitude and height, a prevailing wind carries
//! moisture off the sea and drops it on rising ground, leaving rain shadows behind ranges, and a
//! Whittaker-style table turns warmth and moisture into biomes.

use super::noise::fbm;
use super::terrain::smoothstep;
use super::{par_rows, WaterCell, WorldMap};
use crate::chunks::generate::biome::Biomes;
use crate::chunks::generate::WATER_LEVEL;

/// Warmth lost per block of height, the rate snow lines climb at, see `generator::terrain`.
pub const LAPSE_PER_BLOCK: f32 = 1.0 / 800.0;

// Moisture the wind picks up crossing a cell of warm sea, and how much of what it carries falls
// on a cell of flat land and on a cell rising a full OROGRAPHIC_RISE.
const EVAPORATION: f32 = 0.1;
const BASE_RAIN: f32 = 0.011;
const OROGRAPHIC_RAIN: f32 = 0.12;
const OROGRAPHIC_RISE: f32 = 18.0;
// The share of rain that evaporates again off the land and carries on inland.
const RECYCLED: f32 = 0.72;
// The rain this share of the way up the land's rainfall is as wet as it gets.
const WETTEST_PERCENTILE: f64 = 0.9;
// Moisture is blurred across the wind this many cells each way, so rows of sweeps don't streak.
const BLUR: usize = 10;
// Warmth and moisture are blurred this many cells each way before biomes are picked from them.
const FIELD_BLUR: usize = 4;
// Cells round a river or lake that it wets and cools, and by how much at the water's edge.
const RIPARIAN_REACH: u8 = 14;
// River widths, in blocks, that count for a little, a lot and everything of that reach, and the
// moisture between which streams dry up, and the width a river must have to survive the driest.
const RIPARIAN_WIDTHS: (f32, f32, f32) = (20.0, 38.0, 65.0);
const DRY_BELOW: (f64, f64) = (0.2, 0.45);
const DRY_RIVER_WIDTH: f32 = 34.0;
// A lake this many cells or fewer in dry ground dries up; only a great one survives a desert.
const DESERT_LAKE_CELLS: usize = 6_000;
const RIPARIAN_MOISTURE: f32 = 1.0;
const RIPARIAN_COOLING: f32 = 0.14;
// A cell takes the most common biome within this many cells, MODE_PASSES times, and a patch of
// fewer cells than MIN_BIOME_CELLS joins a neighbour.
const MODE_RADIUS: usize = 2;
const MODE_PASSES: usize = 2;
const MIN_BIOME_CELLS: usize = 400;
// Land this many blocks over the sea is mountain, or this many with ground at least this steep.
const MOUNTAIN_ABOVE: f32 = 150.0;
const MOUNTAIN_STEEP_ABOVE: f32 = 70.0;
const MOUNTAIN_STEEP: f32 = 0.4;
// Conifers on a mountain's lower slopes, this thick, thinning out between these heights over the
// sea to nothing: the treeline.
const ALPINE_FOREST: f64 = 0.3;
const TREELINE: (f64, f64) = (170.0, 290.0);

/// What grows on a stretch of land, stored in `WorldMap::biome`.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u8)]
pub enum BiomeKind {
    Grassland = 0,
    Glacier = 1,
    Desert = 2,
    Forest = 3,
    Tundra = 4,
    Taiga = 5,
    Steppe = 6,
    Savanna = 7,
    Jungle = 8,
    /// The sea floor, which has no land biome.
    Sea = 9,
    /// High, steep ground, whatever the climate.
    Mountain = 10,
}

impl BiomeKind {
    pub const ALL: [BiomeKind; 11] = [
        Self::Grassland,
        Self::Glacier,
        Self::Desert,
        Self::Forest,
        Self::Tundra,
        Self::Taiga,
        Self::Steppe,
        Self::Savanna,
        Self::Jungle,
        Self::Sea,
        Self::Mountain,
    ];

    pub fn from_u8(v: u8) -> Self {
        Self::ALL
            .get(v as usize)
            .copied()
            .unwrap_or(Self::Grassland)
    }

    /// The name in `Biomes`, or None for the base terrain.
    pub fn name(self) -> Option<&'static str> {
        match self {
            Self::Grassland | Self::Sea => None,
            Self::Glacier => Some("glacier"),
            Self::Desert => Some("desert"),
            Self::Forest => Some("forest"),
            Self::Tundra => Some("tundra"),
            Self::Taiga => Some("taiga"),
            Self::Steppe => Some("steppe"),
            Self::Savanna => Some("savanna"),
            Self::Jungle => Some("jungle"),
            Self::Mountain => Some("mountain"),
        }
    }
}

/// The biome where it is `temperature` warm and `moisture` wet, both 0..1, on land.
pub fn classify(temperature: f32, moisture: f32) -> BiomeKind {
    if temperature < 0.1 {
        BiomeKind::Glacier
    } else if temperature < 0.26 {
        if moisture < 0.35 {
            BiomeKind::Tundra
        } else {
            BiomeKind::Taiga
        }
    } else if temperature < 0.58 {
        match moisture {
            m if m < 0.3 => BiomeKind::Steppe,
            m if m < 0.58 => BiomeKind::Grassland,
            _ => BiomeKind::Forest,
        }
    } else {
        match moisture {
            m if m < 0.14 => BiomeKind::Desert,
            m if m < 0.5 => BiomeKind::Savanna,
            m if m < 0.7 => BiomeKind::Forest,
            _ => BiomeKind::Jungle,
        }
    }
}

// Tree density 0..1 for a biome, before the cold thins it out.
fn tree_density(kind: BiomeKind, biomes: &Biomes) -> f64 {
    kind.name().and_then(|name| biomes.id(name)).map_or(
        biomes.tree_density(crate::chunks::generate::biome::BiomeId::BASE),
        |id| biomes.tree_density(id),
    )
}

// Warmth at sea level: hot at the equator, cold at the poles, with a little drift.
fn sea_level_warmth(seed: u32, x: f64, z: f64, half: f64) -> f32 {
    let latitude = (z.abs() / half).min(1.0);
    let warmth = (latitude * std::f64::consts::FRAC_PI_2).cos();
    let drift = fbm(seed.wrapping_add(9 * 101), x, z, 9000.0, 3) * 0.1;
    (warmth + drift).clamp(0.0, 1.0) as f32
}

// Wet at the equator and in the mid latitudes, dry in the subtropics and at the poles.
fn rainy_belt(latitude: f32) -> f32 {
    0.78 + 0.22 * (3.0 * std::f32::consts::PI * latitude).cos()
}

pub fn run(map: &mut WorldMap) {
    let dim = map.dim();
    let half = map.config.half_size() as f64;
    let sea = WATER_LEVEL as f32;
    let seed = map.seed;

    let mut temperature = vec![0.0f32; dim * dim];
    par_rows(dim, &mut temperature, |row, out| {
        let z = map.cell_centre(0, row).1;
        for (col, t) in out.iter_mut().enumerate() {
            *t = sea_level_warmth(seed, map.cell_centre(col, 0).0, z, half);
        }
    });

    // the wind blows from the west: each row is swept east, picking moisture up off the sea
    // and dropping it where the ground rises
    let mut rain = vec![0.0f32; dim * dim];
    par_rows(dim, &mut rain, |row, out| {
        let latitude = ((row as f32 + 0.5) / dim as f32 * 2.0 - 1.0).abs();
        let belt = rainy_belt(latitude);
        let mut carried = 0.4 * belt;
        let mut last = map.height[row * dim];
        for (col, rain) in out.iter_mut().enumerate() {
            let i = row * dim + col;
            let h = map.height[i];
            if h < sea {
                carried = (carried + EVAPORATION * belt * (0.3 + 0.7 * temperature[i])).min(1.0);
            } else {
                let lift = ((h - last) / OROGRAPHIC_RISE).clamp(0.0, 1.0);
                let falls = carried * (BASE_RAIN + OROGRAPHIC_RAIN * lift);
                carried = (carried - falls * (1.0 - RECYCLED)).max(0.0);
                *rain = falls;
            }
            last = h;
        }
    });

    let rain = blur_across(&rain, dim);
    let mut land: Vec<f32> = map
        .height
        .iter()
        .zip(&rain)
        .filter(|(&h, _)| h >= sea)
        .map(|(_, &r)| r)
        .collect();
    land.sort_by(f32::total_cmp);
    let wettest = land
        .get(((land.len() as f64 * WETTEST_PERCENTILE) as usize).min(land.len().saturating_sub(1)))
        .copied()
        .unwrap_or(1.0)
        .max(1e-6);

    let biomes = Biomes::new(seed);
    let mut moisture = vec![1.0f32; dim * dim];
    for i in 0..dim * dim {
        if map.height[i] < sea {
            continue;
        }
        // part how much rain fell, part how it ranks among the land, so every world has its
        // deserts and its jungles however much rain its winds carried
        let rank = land.partition_point(|&r| r < rain[i]) as f32 / land.len() as f32;
        let wet = (rain[i] / wettest).clamp(0.0, 1.0);
        let (x, z) = map.cell_centre(i % dim, i / dim);
        let drift = fbm(seed.wrapping_add(10 * 101), x, z, 6000.0, 3) as f32 * 0.12;
        moisture[i] = (0.6 * rank + 0.4 * wet + drift).clamp(0.0, 1.0);
    }
    // smooth over a few cells each way, so a biome doesn't flip back and forth along the edge
    // where its warmth or moisture hovers at a threshold
    let mut moisture = blur_box(&moisture, dim, FIELD_BLUR);
    let mut temperature = blur_box(&temperature, dim, FIELD_BLUR);
    // streams dry up in dry country: the drier the ground, the wider a river has to be to carry
    // on across it, so a stream that wanders into a desert vanishes and only a river fed from
    // somewhere wetter crosses it. The channel stays, dry, where the water was.
    for i in 0..dim * dim {
        if map.water_kind[i] != WaterCell::River as u8 || map.height[i] < sea {
            continue;
        }
        let dryness = smoothstep(DRY_BELOW.1, DRY_BELOW.0, moisture[i] as f64) as f32;
        if map.river_width[i] < DRY_RIVER_WIDTH * dryness {
            map.water_kind[i] = WaterCell::None as u8;
            map.water_surface[i] = 0.0;
            map.river_width[i] = 0.0;
        }
    }
    // and so do lakes: one in dry ground that isn't a great one is gone, its basin left dry
    dry_lakes(map, &moisture);
    // only big water changes the ground round it: a wide river or a lake cools and wets its
    // banks, so a green belt follows one through a desert, and a stream does nothing of the kind
    let near_water = fresh_water_closeness(map);
    for i in 0..dim * dim {
        if map.height[i] >= sea {
            moisture[i] = (moisture[i] + RIPARIAN_MOISTURE * near_water[i]).min(1.0);
            temperature[i] = (temperature[i] - RIPARIAN_COOLING * near_water[i]).max(0.0);
        }
    }

    let slope = super::gameplay::slope_layer(map);
    let mut kinds = vec![BiomeKind::Sea as u8; dim * dim];
    for i in 0..dim * dim {
        let h = map.height[i];
        if h < sea {
            continue;
        }
        // what a cell grows depends on how warm it is at its height
        let warmth = (temperature[i] - (h - sea) * LAPSE_PER_BLOCK).max(0.0);
        kinds[i] = classify(warmth, moisture[i]) as u8;
        // high or steep ground is mountain whatever the climate, but for the ice on top of it
        let above = h - sea;
        if kinds[i] != BiomeKind::Glacier as u8
            && (above >= MOUNTAIN_ABOVE
                || (above >= MOUNTAIN_STEEP_ABOVE && slope[i] >= MOUNTAIN_STEEP))
        {
            kinds[i] = BiomeKind::Mountain as u8;
        }
    }
    let kinds = tidy_biomes(&kinds, dim);

    let mut trees = vec![0u8; dim * dim];
    for i in 0..dim * dim {
        if map.height[i] < sea {
            continue;
        }
        // lakes and rivers, and nothing grows in them
        let wet = map.water_kind[i] == WaterCell::Lake as u8
            || map.water_kind[i] == WaterCell::River as u8;
        let kind = BiomeKind::from_u8(kinds[i]);
        let above = (map.height[i] - sea) as f64;
        // a mountain's slopes are forested with conifers below the treeline, thinning out into
        // bare rock above it, instead of the few scattered trees the table gives the biome
        let density = if kind == BiomeKind::Mountain {
            ALPINE_FOREST
                * smoothstep(15.0, 60.0, above)
                * (1.0 - smoothstep(TREELINE.0, TREELINE.1, above))
                + 0.02
        } else {
            tree_density(kind, &biomes)
        } * if wet { 0.0 } else { 1.0 };
        trees[i] = (density.clamp(0.0, 1.0) * 255.0).round() as u8;
    }
    map.temperature = temperature;
    map.moisture = moisture;
    map.biome = kinds;
    map.trees = trees;
}

// Lakes in dry ground disappear, a whole lake at a time, unless there are more cells of it than
// DESERT_LAKE_CELLS.
fn dry_lakes(map: &mut WorldMap, moisture: &[f32]) {
    let dim = map.dim();
    let lake: Vec<bool> = map
        .water_kind
        .iter()
        .map(|&k| k == WaterCell::Lake as u8)
        .collect();
    let (comp, sizes) = super::gameplay::components(&lake, dim);
    let mut dryness = vec![0.0f32; sizes.len()];
    for (i, &c) in comp.iter().enumerate() {
        if c != u32::MAX {
            dryness[c as usize] += smoothstep(DRY_BELOW.1, DRY_BELOW.0, moisture[i] as f64) as f32;
        }
    }
    for (i, &c) in comp.iter().enumerate() {
        if c == u32::MAX {
            continue;
        }
        let mean = dryness[c as usize] / sizes[c as usize] as f32;
        if mean > 0.5 && sizes[c as usize] <= DESERT_LAKE_CELLS {
            map.water_kind[i] = WaterCell::None as u8;
            map.water_surface[i] = 0.0;
        }
    }
}

// How strongly water near each cell wets and cools it, 0..1: a lake or a very wide river counts
// in full, a wide river by less, a narrow stream not at all, falling to 0 at RIPARIAN_REACH cells
// away and squared so the green hugs the bank.
fn fresh_water_closeness(map: &WorldMap) -> Vec<f32> {
    let tiers: [(f32, f32); 3] = [
        (RIPARIAN_WIDTHS.0, 0.45),
        (RIPARIAN_WIDTHS.1, 0.8),
        (RIPARIAN_WIDTHS.2, 1.0),
    ];
    let mut out = vec![0.0f32; map.height.len()];
    for (index, (width, strength)) in tiers.into_iter().enumerate() {
        let near = closeness(map, |i| {
            let river = map.water_kind[i] == WaterCell::River as u8 && map.river_width[i] >= width;
            // lakes are the top tier
            river || (index == tiers.len() - 1 && map.water_kind[i] == WaterCell::Lake as u8)
        });
        for (o, n) in out.iter_mut().zip(near) {
            *o = o.max(n * strength);
        }
    }
    out
}

// 1 on the cells `source` picks, falling to 0 at RIPARIAN_REACH cells from them, squared.
fn closeness(map: &WorldMap, source: impl Fn(usize) -> bool) -> Vec<f32> {
    let dim = map.dim();
    let reach = RIPARIAN_REACH;
    let mut dist = vec![u8::MAX; dim * dim];
    let mut frontier: Vec<usize> = Vec::new();
    for i in 0..dim * dim {
        if source(i) {
            dist[i] = 0;
            frontier.push(i);
        }
    }
    let mut next = Vec::new();
    for d in 1..=reach {
        for &i in &frontier {
            let (x, z) = ((i % dim) as i64, (i / dim) as i64);
            for (dx, dz) in [(1i64, 0i64), (-1, 0), (0, 1), (0, -1)] {
                let (nx, nz) = (x + dx, z + dz);
                if nx < 0 || nz < 0 || nx >= dim as i64 || nz >= dim as i64 {
                    continue;
                }
                let n = nz as usize * dim + nx as usize;
                if dist[n] == u8::MAX {
                    dist[n] = d;
                    next.push(n);
                }
            }
        }
        std::mem::swap(&mut frontier, &mut next);
        next.clear();
    }
    dist.iter()
        .map(|&d| {
            if d == u8::MAX {
                0.0
            } else {
                let t = 1.0 - d as f32 / (reach as f32 + 1.0);
                t * t
            }
        })
        .collect()
}

// A box blur of `radius` cells each way along both axes.
pub(super) fn blur_box(layer: &[f32], dim: usize, radius: usize) -> Vec<f32> {
    let along_x = {
        let mut out = vec![0.0f32; layer.len()];
        par_rows(dim, &mut out, |row, cells| {
            for (col, cell) in cells.iter_mut().enumerate() {
                let (lo, hi) = (col.saturating_sub(radius), (col + radius).min(dim - 1));
                let sum: f32 = (lo..=hi).map(|x| layer[row * dim + x]).sum();
                *cell = sum / (hi - lo + 1) as f32;
            }
        });
        out
    };
    let mut out = vec![0.0f32; layer.len()];
    par_rows(dim, &mut out, |row, cells| {
        let (lo, hi) = (row.saturating_sub(radius), (row + radius).min(dim - 1));
        for (col, cell) in cells.iter_mut().enumerate() {
            let sum: f32 = (lo..=hi).map(|z| along_x[z * dim + col]).sum();
            *cell = sum / (hi - lo + 1) as f32;
        }
    });
    out
}

// Biomes with ragged edges and specks tidied: each land cell takes the most common biome in the
// cells round it, then any patch left smaller than MIN_BIOME_CELLS is given to the biome it
// borders most. The sea is left alone.
fn tidy_biomes(kinds: &[u8], dim: usize) -> Vec<u8> {
    let sea = BiomeKind::Sea as u8;
    let mut kinds = kinds.to_vec();
    for _ in 0..MODE_PASSES {
        let mut out = vec![0u8; kinds.len()];
        par_rows(dim, &mut out, |row, cells| {
            for (col, cell) in cells.iter_mut().enumerate() {
                let here = kinds[row * dim + col];
                if here == sea {
                    *cell = sea;
                    continue;
                }
                let mut counts = [0u16; BiomeKind::ALL.len()];
                for z in row.saturating_sub(MODE_RADIUS)..=(row + MODE_RADIUS).min(dim - 1) {
                    for x in col.saturating_sub(MODE_RADIUS)..=(col + MODE_RADIUS).min(dim - 1) {
                        counts[kinds[z * dim + x] as usize] += 1;
                    }
                }
                counts[sea as usize] = 0;
                // ties keep the cell's own biome
                let best = (0..counts.len())
                    .max_by_key(|&k| (counts[k], k as u8 == here))
                    .unwrap();
                *cell = best as u8;
            }
        });
        kinds = out;
    }
    // a patch of one biome too small to be worth its own: merged into the neighbour it shares
    // the longest edge with
    let mut small_passes = 0;
    loop {
        let mut changed = false;
        let mut comp = vec![u32::MAX; kinds.len()];
        let mut stack = Vec::new();
        for start in 0..kinds.len() {
            if comp[start] != u32::MAX || kinds[start] == sea {
                continue;
            }
            let id = start as u32;
            let mut cells = Vec::new();
            let mut border = [0u32; BiomeKind::ALL.len()];
            comp[start] = id;
            stack.push(start);
            while let Some(i) = stack.pop() {
                cells.push(i);
                let (x, z) = ((i % dim) as i64, (i / dim) as i64);
                for (dx, dz) in [(1i64, 0i64), (-1, 0), (0, 1), (0, -1)] {
                    let (nx, nz) = (x + dx, z + dz);
                    if nx < 0 || nz < 0 || nx >= dim as i64 || nz >= dim as i64 {
                        continue;
                    }
                    let n = nz as usize * dim + nx as usize;
                    if kinds[n] == kinds[start] {
                        if comp[n] == u32::MAX {
                            comp[n] = id;
                            stack.push(n);
                        }
                    } else if kinds[n] != sea {
                        border[kinds[n] as usize] += 1;
                    }
                }
            }
            if cells.len() < MIN_BIOME_CELLS {
                if let Some(best) = (0..border.len())
                    .filter(|&k| border[k] > 0)
                    .max_by_key(|&k| border[k])
                {
                    for &i in &cells {
                        kinds[i] = best as u8;
                    }
                    changed = true;
                }
            }
        }
        small_passes += 1;
        if !changed || small_passes >= 3 {
            break;
        }
    }
    kinds
}

// A box blur across the wind, along z.
fn blur_across(layer: &[f32], dim: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; layer.len()];
    par_rows(dim, &mut out, |row, cells| {
        let (lo, hi) = (row.saturating_sub(BLUR), (row + BLUR).min(dim - 1));
        for (col, cell) in cells.iter_mut().enumerate() {
            let sum: f32 = (lo..=hi).map(|z| layer[z * dim + col]).sum();
            *cell = sum / (hi - lo + 1) as f32;
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worldmap::WorldConfig;

    // Sea in the west, then land with a ridge across it north to south, then flat land beyond.
    fn ridge_world() -> WorldMap {
        let config = WorldConfig {
            size: 256 * 32,
            map_cell: 32,
        };
        let dim = config.dim();
        let mut map = WorldMap::empty(1, config);
        for z in 0..dim {
            for x in 0..dim {
                map.height[z * dim + x] = match x {
                    0..=39 => 380.0,
                    40..=79 => 452.0,
                    80..=119 => 452.0 + (x - 79) as f32 * 8.0,
                    120..=159 => 772.0 - (x - 119) as f32 * 8.0,
                    _ => 452.0,
                };
            }
        }
        map
    }

    #[test]
    fn test_rain_falls_on_the_windward_side_and_not_behind_the_range() {
        let mut map = ridge_world();
        run(&mut map);
        let dim = map.dim();
        let at = |x: usize| map.moisture[(dim / 2) * dim + x];
        let windward = (85..115).map(at).sum::<f32>() / 30.0;
        let shadow = (165..200).map(at).sum::<f32>() / 35.0;
        assert!(
            windward > shadow + 0.3,
            "windward {windward} against rain shadow {shadow}"
        );
    }

    #[test]
    fn test_warmth_falls_towards_the_poles_and_up_mountains() {
        let mut map = ridge_world();
        run(&mut map);
        let dim = map.dim();
        let t = |x: usize, z: usize| map.temperature[z * dim + x];
        assert!(
            t(60, dim / 2) > t(60, 4) + 0.5,
            "{} vs {}",
            t(60, dim / 2),
            t(60, 4)
        );
        // at 70% of the way to the pole the plain is mild and the 320 m ridge is ice
        let row = dim * 17 / 20;
        let kind = |x: usize| BiomeKind::from_u8(map.biome[row * dim + x]);
        assert_ne!(BiomeKind::Glacier, kind(60));
        assert_eq!(BiomeKind::Glacier, kind(119));
    }

    // A river along `row` over `x` cells, `width` blocks wide.
    fn with_river(width: f32, x: std::ops::Range<usize>) -> WorldMap {
        let mut map = ridge_world();
        let row = map.dim() / 2;
        for x in x {
            let i = map.index(x, row);
            map.water_kind[i] = WaterCell::River as u8;
            map.river_width[i] = width;
            map.water_surface[i] = 460.0;
        }
        map
    }

    #[test]
    fn test_a_big_river_wets_and_cools_the_dry_ground_round_it() {
        let mut map = with_river(70.0, 150..252);
        let (dim, row) = (map.dim(), map.dim() / 2);
        run(&mut map);
        let at = |x: usize, z: usize| map.moisture[z * dim + x];
        let t = |x: usize, z: usize| map.temperature[z * dim + x];
        assert_eq!(
            WaterCell::River as u8,
            map.water_kind[row * dim + 190],
            "a big river crosses the desert"
        );
        // beside the river, against the same dry plain well away from it
        assert!(
            at(190, row + 2) > at(190, row + 40) + 0.3,
            "{} vs {}",
            at(190, row + 2),
            at(190, row + 40)
        );
        // (warmth falls with latitude, so the dry ground to compare with is along the same row)
        assert!(t(190, row + 2) < t(125, row + 2) - 0.05);
        // and wet enough for trees where it would otherwise be steppe or desert
        let dry = BiomeKind::from_u8(map.biome[(row + 40) * dim + 190]);
        let bank = BiomeKind::from_u8(map.biome[(row + 2) * dim + 190]);
        assert_ne!(dry, bank);
        assert!(map.trees[(row + 2) * dim + 190] >= map.trees[(row + 40) * dim + 190]);
    }

    #[test]
    fn test_a_narrow_stream_dries_up_in_a_desert_but_not_where_it_is_wet() {
        let mut map = with_river(7.0, 90..215);
        let (dim, row) = (map.dim(), map.dim() / 2);
        run(&mut map);
        let wet_side = map.water_kind[row * dim + 100];
        let dry_side = map.water_kind[row * dim + 190];
        assert_eq!(
            WaterCell::River as u8,
            wet_side,
            "a stream on the windward slope keeps running"
        );
        assert_eq!(
            WaterCell::None as u8,
            dry_side,
            "the same stream is gone in the rain shadow"
        );
        assert_eq!(0.0, map.river_width[row * dim + 190]);
        // and it leaves the ground as dry as the desert round it, not a green belt
        let (at, far) = (
            map.moisture[(row + 2) * dim + 190],
            map.moisture[(row + 40) * dim + 190],
        );
        assert!((at - far).abs() < 0.25, "{at} vs {far}");
    }

    #[test]
    fn test_a_narrow_stream_leaves_the_biome_round_it_alone() {
        let (mut with, mut without) = (with_river(12.0, 90..120), ridge_world());
        let (dim, row) = (with.dim(), with.dim() / 2);
        run(&mut with);
        run(&mut without);
        for x in 95..115 {
            let i = (row + 3) * dim + x;
            assert_eq!(
                without.biome[i], with.biome[i],
                "biome changed by a stream at {x}"
            );
        }
    }

    #[test]
    fn test_mountain_slopes_have_a_conifer_belt_that_ends_at_the_treeline_and_the_air_cools_with_height(
    ) {
        use crate::chunks::generate::terrain_source::TerrainSource;
        let map = std::sync::Arc::new(crate::worldmap::generate(
            3,
            WorldConfig {
                size: 32_768,
                map_cell: 32,
            },
        ));
        let sea = WATER_LEVEL as f32;
        let (mut lower, mut thick, mut high, mut bare) = (0, 0, 0, 0);
        for i in 0..map.biome.len() {
            if BiomeKind::from_u8(map.biome[i]) != BiomeKind::Mountain || map.water_kind[i] != 0 {
                continue;
            }
            let above = map.height[i] - sea;
            if (60.0..150.0).contains(&above) {
                lower += 1;
                thick += (map.trees[i] >= 40) as usize;
            }
            if above > 330.0 {
                high += 1;
                bare += (map.trees[i] <= 10) as usize;
            }
        }
        assert!(
            lower > 100 && thick * 2 > lower,
            "{thick} of {lower} lower slope cells forested"
        );
        assert!(
            high > 50 && bare == high,
            "{bare} of {high} cells above the treeline are bare"
        );
        // warmth falls with height at the rate snow lines climb, at one place and then a higher one
        let terrain = crate::worldmap::WorldTerrain::new(map.clone());
        let low = (0..map.height.len())
            .find(|&i| {
                map.height[i] > sea + 5.0 && map.height[i] < sea + 20.0 && map.water_kind[i] == 0
            })
            .unwrap();
        let peak = (0..map.height.len())
            .find(|&i| map.height[i] > sea + 330.0)
            .unwrap();
        let warmth = |i: usize| {
            let (x, z) = map.cell_centre(i % map.dim(), i / map.dim());
            (terrain.climate(x, z).temperature, terrain.temperature(x, z))
        };
        let (high_air, high_sea_level) = warmth(peak);
        let (low_air, low_sea_level) = warmth(low);
        assert!(
            (low_sea_level - low_air) < 0.03,
            "lowland cooled by {}",
            low_sea_level - low_air
        );
        assert!(
            high_sea_level - high_air > 0.25,
            "a peak cooled by only {}",
            high_sea_level - high_air
        );
    }

    #[test]
    fn test_biomes_have_no_specks() {
        let map = crate::worldmap::generate(
            3,
            WorldConfig {
                size: 16_384,
                map_cell: 32,
            },
        );
        let dim = map.dim();
        let sea = BiomeKind::Sea as u8;
        let mut seen = vec![false; map.biome.len()];
        let mut small = 0;
        for start in 0..map.biome.len() {
            if seen[start] || map.biome[start] == sea {
                continue;
            }
            let (mut stack, mut cells, mut touches_other_land) = (vec![start], 0, false);
            seen[start] = true;
            while let Some(i) = stack.pop() {
                cells += 1;
                let (x, z) = (i % dim, i / dim);
                for (nx, nz) in [
                    (x.wrapping_sub(1), z),
                    (x + 1, z),
                    (x, z.wrapping_sub(1)),
                    (x, z + 1),
                ] {
                    if nx >= dim || nz >= dim {
                        continue;
                    }
                    let n = nz * dim + nx;
                    if map.biome[n] == map.biome[start] && !seen[n] {
                        seen[n] = true;
                        stack.push(n);
                    } else if map.biome[n] != map.biome[start] && map.biome[n] != sea {
                        touches_other_land = true;
                    }
                }
            }
            // a patch with land beside it that is small enough to be a speck
            if touches_other_land && cells < MIN_BIOME_CELLS / 2 {
                small += 1;
            }
        }
        assert_eq!(0, small, "{small} specks of biome");
    }

    #[test]
    fn test_whittaker_table_covers_the_extremes() {
        assert_eq!(BiomeKind::Glacier, classify(0.0, 0.5));
        assert_eq!(BiomeKind::Tundra, classify(0.2, 0.1));
        assert_eq!(BiomeKind::Taiga, classify(0.2, 0.9));
        assert_eq!(BiomeKind::Steppe, classify(0.45, 0.1));
        assert_eq!(BiomeKind::Forest, classify(0.45, 0.9));
        assert_eq!(BiomeKind::Desert, classify(0.9, 0.05));
        assert_eq!(BiomeKind::Savanna, classify(0.9, 0.4));
        assert_eq!(BiomeKind::Jungle, classify(0.9, 0.95));
    }

    #[test]
    fn test_every_kind_has_a_biome_in_the_table() {
        let biomes = Biomes::new(1);
        for kind in BiomeKind::ALL {
            if let Some(name) = kind.name() {
                assert!(biomes.id(name).is_some(), "{name} missing");
            }
        }
    }
}
