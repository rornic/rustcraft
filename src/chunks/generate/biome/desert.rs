use super::Biome;
use crate::block::BlockType;

// Sand over the lowlands. Rock and snow on the heights are left as they are, and the dunes are
// the ground's, see `worldmap::detail::dunes`.
pub struct Desert;

impl Biome for Desert {
    fn name(&self) -> &'static str {
        "desert"
    }

    fn spring_block(&self) -> BlockType {
        BlockType::Sandstone
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

    #[test]
    fn test_grass_turns_to_sand_but_rock_and_snow_stay() {
        assert_eq!(BlockType::Sand, Desert.block(BlockType::Grass, 10, 0.0));
        for block in [
            BlockType::Stone,
            BlockType::Snow,
            BlockType::Water,
            BlockType::Sand,
            BlockType::Air,
        ] {
            assert_eq!(block, Desert.block(block, 10, 0.0));
        }
    }
}
