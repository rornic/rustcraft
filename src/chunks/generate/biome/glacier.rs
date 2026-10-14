use super::{smoothstep, BaseColumn, Biome};
use crate::block::BlockType;
use crate::chunks::generate::WATER_LEVEL;

// Strength rises as the temperature falls from the first to the second.
const COLD: (f64, f64) = (0.2, 0.12);
// none right at the shore, or the ground is raised out of the shallows
const SHORE: (f64, f64) = (0.0, 0.06);

// Ground flatter than this is bare ice, up to the next is snow, and steeper than that is gravel.
const ICE_GRADIENT: f64 = 0.4;
const SNOW_GRADIENT: f64 = 1.6;

// Relief above the sea is scaled by this, up to FLATTENED_UNDER blocks and fading out above it so mountains keep their height.
const LOWLAND_RELIEF: f64 = 0.45;
const FLATTENED_UNDER: (f64, f64) = (60.0, 200.0);

// Flat ice fields and snow over the coldest land, near the poles.
pub struct Glacier;

impl Biome for Glacier {
    fn name(&self) -> &'static str {
        "glacier"
    }

    fn strength(&self, column: &BaseColumn) -> f64 {
        smoothstep(COLD.0, COLD.1, column.climate.temperature as f64)
            * smoothstep(SHORE.0, SHORE.1, column.continentalness)
    }

    fn height(&self, column: &BaseColumn) -> f64 {
        let above_sea = column.height - WATER_LEVEL as f64;
        if above_sea <= 0.0 {
            return column.height;
        }
        let relief = LOWLAND_RELIEF + (1.0 - LOWLAND_RELIEF) * smoothstep(FLATTENED_UNDER.0, FLATTENED_UNDER.1, above_sea);
        WATER_LEVEL as f64 + above_sea * relief
    }

    fn tree_density(&self) -> Option<f64> {
        Some(0.0)
    }

    fn block(&self, base: BlockType, _above_sea: i64, gradient: f64) -> BlockType {
        match base {
            BlockType::Grass | BlockType::Snow if gradient < ICE_GRADIENT => BlockType::Ice,
            BlockType::Grass if gradient < SNOW_GRADIENT => BlockType::Snow,
            BlockType::Grass | BlockType::Sand => BlockType::Gravel,
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunks::generate::biome::Climate;

    fn column(temperature: f32, above_sea: f64, continentalness: f64) -> BaseColumn {
        BaseColumn {
            x: 0.0,
            z: 0.0,
            cell: 1.0,
            height: WATER_LEVEL as f64 + above_sea,
            continentalness,
            ruggedness: -1.0,
            climate: Climate { temperature, humidity: 0.5 },
        }
    }

    #[test]
    fn test_only_cold_land_is_glacier() {
        assert_eq!(1.0, Glacier.strength(&column(0.05, 20.0, 0.3)));
        assert_eq!(0.0, Glacier.strength(&column(0.3, 20.0, 0.3)));
        assert_eq!(0.0, Glacier.strength(&column(0.05, 20.0, -0.1)));
    }

    #[test]
    fn test_glacier_ground_follows_the_slope() {
        assert_eq!(BlockType::Ice, Glacier.block(BlockType::Grass, 20, 0.1));
        assert_eq!(BlockType::Snow, Glacier.block(BlockType::Grass, 20, 1.0));
        assert_eq!(BlockType::Gravel, Glacier.block(BlockType::Grass, 20, 2.5));
        assert_eq!(BlockType::Stone, Glacier.block(BlockType::Stone, 20, 2.5));
        assert_eq!(BlockType::Water, Glacier.block(BlockType::Water, -5, 0.0));
    }

    #[test]
    fn test_glacier_flattens_lowlands_but_not_peaks() {
        let flat = Glacier.height(&column(0.0, 40.0, 0.3)) - WATER_LEVEL as f64;
        assert!(flat < 40.0 * 0.6, "lowland was {flat}");
        assert_eq!(WATER_LEVEL as f64 + 300.0, Glacier.height(&column(0.0, 300.0, 0.3)));
        assert_eq!(WATER_LEVEL as f64 - 20.0, Glacier.height(&column(0.0, -20.0, -0.3)));
    }
}
