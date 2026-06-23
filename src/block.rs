#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub enum BlockType {
    Air,
    Stone,
    Grass,
    Sand,
    Water,
    Snow,
}

impl Default for BlockType {
    fn default() -> Self {
        Self::Air
    }
}

pub const BLOCK_COUNT: usize = 6;

impl BlockType {
    pub fn occlusion_weight(&self) -> f32 {
        match self {
            BlockType::Air => 0.0,
            BlockType::Water => 0.4,
            BlockType::Stone | BlockType::Grass | BlockType::Sand | BlockType::Snow => 1.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::BlockType;

    #[test]
    fn test_occlusion_weight_air_is_zero() {
        assert_eq!(0.0, BlockType::Air.occlusion_weight());
    }

    #[test]
    fn test_occlusion_weight_solid_blocks_are_one() {
        assert_eq!(1.0, BlockType::Stone.occlusion_weight());
        assert_eq!(1.0, BlockType::Grass.occlusion_weight());
        assert_eq!(1.0, BlockType::Sand.occlusion_weight());
        assert_eq!(1.0, BlockType::Snow.occlusion_weight());
    }

    #[test]
    fn test_occlusion_weight_water_is_partial() {
        let w = BlockType::Water.occlusion_weight();
        assert!(w > 0.0 && w < 1.0);
    }
}
