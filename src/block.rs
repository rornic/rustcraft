#[derive(Debug, PartialEq, Eq, Copy, Clone)]
#[repr(u8)]
pub enum BlockType {
    Air,
    Stone,
    Grass,
    Sand,
    Water,
    Snow,
    Log,
    Leaves,
}

impl Default for BlockType {
    fn default() -> Self {
        Self::Air
    }
}

pub const BLOCK_COUNT: usize = 8;

impl BlockType {
    pub fn occlusion_weight(&self) -> f32 {
        match self {
            BlockType::Air => 0.0,
            BlockType::Water => 0.4,
            BlockType::Leaves => 0.3,
            BlockType::Stone | BlockType::Grass | BlockType::Sand | BlockType::Snow | BlockType::Log => 1.0,
        }
    }

    // How many levels of sky light this block takes from light passing through it: all of
    // it for opaque blocks, see `light.rs`.
    pub fn light_opacity(&self) -> u8 {
        match self {
            BlockType::Air => 0,
            BlockType::Leaves => 1,
            BlockType::Water => 2,
            _ => crate::chunks::generate::light::MAX_LIGHT,
        }
    }

    // Whether the faces of blocks behind this one are hidden by it. Leaves have holes.
    pub fn is_opaque(&self) -> bool {
        self.light_opacity() == crate::chunks::generate::light::MAX_LIGHT
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
        assert_eq!(1.0, BlockType::Log.occlusion_weight());
    }

    #[test]
    fn test_only_solid_blocks_are_opaque() {
        assert!(BlockType::Stone.is_opaque());
        assert!(BlockType::Log.is_opaque());
        assert!(!BlockType::Leaves.is_opaque());
        assert!(!BlockType::Water.is_opaque());
        assert!(!BlockType::Air.is_opaque());
    }

    #[test]
    fn test_occlusion_weight_water_is_partial() {
        let w = BlockType::Water.occlusion_weight();
        assert!(w > 0.0 && w < 1.0);
    }
}
