use noise::{NoiseFn, Perlin};

use super::{smoothstep, BaseColumn, Biome};
use crate::block::BlockType;
use crate::chunks::generate::WATER_LEVEL;

// Only the hottest, driest climates turn to desert.
const HOT: (f64, f64) = (0.55, 0.65);
const DRY: (f64, f64) = (0.45, 0.35);

// Rows of dunes driven by a wind along +x: a long gentle slope up to each crest, and a
// steep face falling away behind it.
const DUNE_SPACING: f64 = 90.0;
const DUNE_HEIGHT: f64 = 18.0;
// The share of each dune that's the gentle windward slope.
const WINDWARD: f64 = 0.75;
// How far, in dunes, the rows bend, and over what distance, so crests wander instead of running dead straight.
const CREST_BEND: f64 = 0.8;
const CREST_BEND_WAVELENGTH: f64 = 400.0;
// Where dunes stand tall and where they lie low, so the sand isn't one even sea of crests.
const DUNE_FIELD_WAVELENGTH: f64 = 700.0;
// Dunes build up inland from the beach and give way to rock on high ground.
const DUNE_SHORE: (f64, f64) = (2.0, 14.0);
const DUNE_HIGH_GROUND: (f64, f64) = (70.0, 140.0);

// Seeds after the base terrain's, see `NoiseGenerator::new`.
const SEED_OFFSET: u32 = 2000;

// Sand over the lowlands, piled into dunes. Rock and snow on the heights are left as they are.
pub struct Desert {
    crest_bend: Perlin,
    dune_field: Perlin,
}

impl Desert {
    pub fn new(seed: u32) -> Self {
        Self {
            crest_bend: Perlin::new(seed.wrapping_add(SEED_OFFSET)),
            dune_field: Perlin::new(seed.wrapping_add(SEED_OFFSET + 1)),
        }
    }

    // 0..1 along the rows of dunes, 1 on each crest
    fn dune(&self, x: f64, z: f64) -> f64 {
        let bend = self.crest_bend.get([x / CREST_BEND_WAVELENGTH, z / CREST_BEND_WAVELENGTH]) * CREST_BEND;
        let t = (x / DUNE_SPACING + bend).rem_euclid(1.0);
        if t < WINDWARD {
            smoothstep(0.0, WINDWARD, t)
        } else {
            smoothstep(1.0, WINDWARD, t)
        }
    }
}

impl Biome for Desert {
    fn name(&self) -> &'static str {
        "desert"
    }

    fn strength(&self, column: &BaseColumn) -> f64 {
        let climate = column.climate;
        smoothstep(HOT.0, HOT.1, climate.temperature as f64) * smoothstep(DRY.0, DRY.1, climate.humidity as f64)
    }

    fn height(&self, column: &BaseColumn) -> f64 {
        let above_sea = column.height - WATER_LEVEL as f64;
        let room = smoothstep(DUNE_SHORE.0, DUNE_SHORE.1, above_sea)
            * smoothstep(DUNE_HIGH_GROUND.1, DUNE_HIGH_GROUND.0, above_sea);
        if room <= 0.0 {
            return column.height;
        }
        let field = smoothstep(-0.5, 0.3, self.dune_field.get([column.x / DUNE_FIELD_WAVELENGTH, column.z / DUNE_FIELD_WAVELENGTH]));
        let dune = self.dune(column.x, column.z) * DUNE_HEIGHT * (0.4 + 0.6 * field);
        column.height + dune * room
    }

    fn tree_density(&self) -> Option<f64> {
        Some(0.0)
    }

    fn block(&self, base: BlockType, _above_sea: i64, _gradient: f64) -> BlockType {
        match base {
            BlockType::Grass => BlockType::Sand,
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunks::generate::biome::Climate;

    fn climate(temperature: f32, humidity: f32) -> Climate {
        Climate { temperature, humidity }
    }

    #[test]
    fn test_only_hot_dry_climates_are_desert() {
        let desert = Desert::new(1);
        let strength = |climate| desert.strength(&BaseColumn { climate, ..column(0.0, 30.0) });
        assert_eq!(1.0, strength(climate(0.9, 0.1)));
        assert_eq!(0.0, strength(climate(0.9, 0.6)));
        assert_eq!(0.0, strength(climate(0.4, 0.1)));
        assert_eq!(0.0, strength(Climate::default()));
    }

    #[test]
    fn test_grass_turns_to_sand_but_rock_and_snow_stay() {
        let desert = Desert::new(1);
        assert_eq!(BlockType::Sand, desert.block(BlockType::Grass, 10, 0.0));
        for block in [BlockType::Stone, BlockType::Snow, BlockType::Water, BlockType::Sand, BlockType::Air] {
            assert_eq!(block, desert.block(block, 10, 0.0));
        }
    }

    fn column(x: f64, above_sea: f64) -> BaseColumn {
        BaseColumn {
            x,
            z: 0.0,
            height: WATER_LEVEL as f64 + above_sea,
            continentalness: 0.3,
            ruggedness: -1.0,
            climate: climate(1.0, 0.0),
        }
    }

    #[test]
    fn test_dunes_rise_inland_but_not_on_beaches_or_mountains() {
        let desert = Desert::new(1);
        let rise = |above_sea: f64| {
            (0..200).map(|x| desert.height(&column(x as f64 * 3.0, above_sea)) - column(0.0, above_sea).height).fold(0.0, f64::max)
        };
        assert_eq!(0.0, rise(1.0));
        assert_eq!(0.0, rise(200.0));
        assert!(rise(30.0) > DUNE_HEIGHT * 0.8, "dunes only rose {}", rise(30.0));
        assert!(rise(30.0) <= DUNE_HEIGHT);
    }

    #[test]
    fn test_dunes_rise_gently_and_fall_steeply() {
        let desert = Desert::new(1);
        let (mut up, mut down) = (0.0f64, 0.0f64);
        for x in 0..2000 {
            let step = desert.dune(x as f64 + 1.0, 0.0) - desert.dune(x as f64, 0.0);
            (up, down) = (up.max(step), down.min(step));
        }
        assert!(-down > up * 2.0, "steepest rise {up}, steepest fall {down}");
    }
}
