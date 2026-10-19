use super::Biome;
use crate::block::BlockType;

// Ground steeper than this, or higher than ALPINE_ABOVE blocks over the sea, is bare rock.
const ROCK_GRADIENT: f64 = 1.0;
const ALPINE_ABOVE: i64 = 175;

/// High, steep ground: bare rock where it is steep or above the treeline, meadow in the gentle
/// pockets, scree for the sand and a few scattered trees.
pub struct Mountain;

impl Biome for Mountain {
    fn name(&self) -> &'static str {
        "mountain"
    }

    fn tree_density(&self) -> Option<f64> {
        Some(0.04)
    }

    fn block(&self, base: BlockType, above_sea: i64, gradient: f64) -> BlockType {
        match base {
            BlockType::Grass if gradient >= ROCK_GRADIENT || above_sea >= ALPINE_ABOVE => {
                BlockType::Stone
            }
            BlockType::Sand => BlockType::Gravel,
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_steep_and_high_ground_is_rock_and_gentle_low_pockets_stay_meadow() {
        assert_eq!(BlockType::Stone, Mountain.block(BlockType::Grass, 120, 1.6));
        assert_eq!(BlockType::Stone, Mountain.block(BlockType::Grass, 200, 0.2));
        assert_eq!(BlockType::Grass, Mountain.block(BlockType::Grass, 120, 0.3));
        assert_eq!(BlockType::Gravel, Mountain.block(BlockType::Sand, 120, 0.3));
        assert_eq!(BlockType::Snow, Mountain.block(BlockType::Snow, 220, 0.3));
        assert_eq!(BlockType::Water, Mountain.block(BlockType::Water, 5, 0.0));
    }
}
