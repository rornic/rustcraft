//! Climate from geography: warmth falls with latitude and height, a prevailing wind carries
//! moisture off the sea and drops it on rising ground, leaving rain shadows behind ranges, and a
//! Whittaker-style table turns warmth and moisture into biomes.

use super::noise::fbm;
use super::{par_rows, WaterCell, WorldMap};
use crate::chunks::generate::biome::Biomes;
use crate::chunks::generate::WATER_LEVEL;

/// Warmth lost per block of height, the rate snow lines climb at, see `generator::terrain`.
pub const LAPSE_PER_BLOCK: f32 = 1.0 / 800.0;

// Moisture the wind picks up crossing a cell of warm sea, and how much of what it carries falls
// on a cell of flat land and on a cell rising a full OROGRAPHIC_RISE.
const EVAPORATION: f32 = 0.1;
const BASE_RAIN: f32 = 0.008;
const OROGRAPHIC_RAIN: f32 = 0.12;
const OROGRAPHIC_RISE: f32 = 18.0;
// The share of rain that evaporates again off the land and carries on inland.
const RECYCLED: f32 = 0.6;
// The rain this share of the way up the land's rainfall is as wet as it gets.
const WETTEST_PERCENTILE: f64 = 0.9;
// Moisture is blurred across the wind this many cells each way, so rows of sweeps don't streak.
const BLUR: usize = 10;

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
}

impl BiomeKind {
    pub const ALL: [BiomeKind; 10] = [
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
            m if m < 0.28 => BiomeKind::Desert,
            m if m < 0.58 => BiomeKind::Savanna,
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
    0.65 + 0.35 * (3.0 * std::f32::consts::PI * latitude).cos()
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
    let mut kinds = vec![BiomeKind::Sea as u8; dim * dim];
    let mut trees = vec![0u8; dim * dim];
    for i in 0..dim * dim {
        let h = map.height[i];
        if h < sea {
            continue;
        }
        // part how much rain fell, part how it ranks among the land, so every world has its
        // deserts and its jungles however much rain its winds carried
        let rank = land.partition_point(|&r| r < rain[i]) as f32 / land.len() as f32;
        let wet = (rain[i] / wettest).clamp(0.0, 1.0);
        let (x, z) = map.cell_centre(i % dim, i / dim);
        let drift = fbm(seed.wrapping_add(10 * 101), x, z, 6000.0, 3) as f32 * 0.12;
        moisture[i] = (0.6 * rank + 0.4 * wet + drift).clamp(0.0, 1.0);
        // what a cell grows depends on how warm it is at its height
        let warmth = (temperature[i] - (h - sea) * LAPSE_PER_BLOCK).max(0.0);
        let kind = classify(warmth, moisture[i]);
        kinds[i] = kind as u8;
        // lakes and rivers wet their banks, and nothing grows in them
        let wet = map.water_kind[i] == WaterCell::Lake as u8
            || map.water_kind[i] == WaterCell::River as u8;
        let density = tree_density(kind, &biomes) * if wet { 0.0 } else { 1.0 };
        trees[i] = (density.clamp(0.0, 1.0) * 255.0).round() as u8;
    }
    map.temperature = temperature;
    map.moisture = moisture;
    map.biome = kinds;
    map.trees = trees;
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
