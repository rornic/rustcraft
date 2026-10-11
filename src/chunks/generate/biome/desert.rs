use noise::{NoiseFn, Perlin};

use super::{smoothstep, BaseColumn, Biome, Climate};
use crate::block::BlockType;
use crate::chunks::generate::WATER_LEVEL;

// Only the hottest, driest climates turn to desert.
const HOT: (f64, f64) = (0.55, 0.65);
const DRY: (f64, f64) = (0.45, 0.35);

// Dunes run in long crests: shorter across the wind than along it.
const DUNE_WAVELENGTH_ACROSS: f64 = 48.0;
const DUNE_WAVELENGTH_ALONG: f64 = 140.0;
const DUNE_HEIGHT: f64 = 9.0;
// Where dunes stand tall and where they lie low, so the sand isn't one even sea of crests.
const DUNE_FIELD_WAVELENGTH: f64 = 600.0;
// Dunes build up inland from the beach and give way to rock on high ground.
const DUNE_SHORE: (f64, f64) = (2.0, 14.0);
const DUNE_HIGH_GROUND: (f64, f64) = (70.0, 140.0);

// Seeds after the base terrain's, see `NoiseGenerator::new`.
const SEED_OFFSET: u32 = 2000;

// Sand over the lowlands, piled into dunes. Rock and snow on the heights are left as they are.
pub struct Desert {
    dunes: Perlin,
    dune_field: Perlin,
}

impl Desert {
    pub fn new(seed: u32) -> Self {
        Self {
            dunes: Perlin::new(seed.wrapping_add(SEED_OFFSET)),
            dune_field: Perlin::new(seed.wrapping_add(SEED_OFFSET + 1)),
        }
    }

    // 0..1, sharp crests and broad hollows
    fn dune(&self, x: f64, z: f64) -> f64 {
        let crest = 1.0 - self.dunes.get([x / DUNE_WAVELENGTH_ACROSS, z / DUNE_WAVELENGTH_ALONG]).abs();
        crest * crest
    }
}

impl Biome for Desert {
    fn strength(&self, climate: Climate) -> f64 {
        smoothstep(HOT.0, HOT.1, climate.temperature as f64) * smoothstep(DRY.0, DRY.1, climate.humidity as f64)
    }

    fn height(&self, column: &BaseColumn) -> f64 {
        let above_sea = column.height - WATER_LEVEL as f64;
        let room = smoothstep(DUNE_SHORE.0, DUNE_SHORE.1, above_sea)
            * smoothstep(DUNE_HIGH_GROUND.1, DUNE_HIGH_GROUND.0, above_sea);
        if room <= 0.0 {
            return column.height;
        }
        let field = 0.5 + 0.5 * self.dune_field.get([column.x / DUNE_FIELD_WAVELENGTH, column.z / DUNE_FIELD_WAVELENGTH]);
        column.height + self.dune(column.x, column.z) * DUNE_HEIGHT * (0.3 + 0.7 * field) * room
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

    fn climate(temperature: f32, humidity: f32) -> Climate {
        Climate { temperature, humidity }
    }

    #[test]
    fn test_only_hot_dry_climates_are_desert() {
        let desert = Desert::new(1);
        assert_eq!(1.0, desert.strength(climate(0.9, 0.1)));
        assert_eq!(0.0, desert.strength(climate(0.9, 0.6)));
        assert_eq!(0.0, desert.strength(climate(0.4, 0.1)));
        assert_eq!(0.0, desert.strength(Climate::default()));
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
        assert!(rise(30.0) > DUNE_HEIGHT * 0.5, "dunes only rose {}", rise(30.0));
        assert!(rise(30.0) <= DUNE_HEIGHT);
    }
}
