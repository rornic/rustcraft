#[derive(Debug, PartialEq, Eq, Copy, Clone)]
#[repr(u8)]
#[derive(Default)]
pub enum BlockType {
    #[default]
    Air,
    Stone,
    Grass,
    Sand,
    Water,
    Snow,
    Log,
    Leaves,
    Dirt,
    Gravel,
    Ice,
    Sandstone,
}

pub const BLOCK_COUNT: usize = 12;

impl BlockType {
    pub fn occlusion_weight(&self) -> f32 {
        match self {
            BlockType::Air => 0.0,
            BlockType::Water => 0.4,
            BlockType::Leaves => 0.3,
            BlockType::Stone
            | BlockType::Grass
            | BlockType::Sand
            | BlockType::Snow
            | BlockType::Log
            | BlockType::Dirt
            | BlockType::Gravel
            | BlockType::Ice
            | BlockType::Sandstone => 1.0,
        }
    }

    // How many levels of sky light this block takes from light passing through it: all of
    // it for opaque blocks, see `light.rs`.
    pub fn light_opacity(&self) -> u8 {
        match self {
            BlockType::Air => 0,
            BlockType::Leaves => 1,
            BlockType::Water => 2,
            BlockType::Ice => 3,
            _ => crate::chunks::generate::light::MAX_LIGHT,
        }
    }

    // Whether the faces of blocks behind this one are hidden by it. Leaves have holes.
    pub fn is_opaque(&self) -> bool {
        self.light_opacity() == crate::chunks::generate::light::MAX_LIGHT
    }
}

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub enum Face {
    Top,
    Side,
    Bottom,
}

impl Face {
    pub fn of_normal_y(y: f32) -> Self {
        if y > 0.5 {
            Face::Top
        } else if y < -0.5 {
            Face::Bottom
        } else {
            Face::Side
        }
    }
}

// The textures in blocks.png, left to right; `gen_atlas` makes them in this order.
#[derive(Debug, PartialEq, Eq, Copy, Clone)]
#[repr(u8)]
pub enum Tile {
    Stone,
    GrassTop,
    Sand,
    Water,
    Snow,
    LogBark,
    Leaves,
    GrassSide,
    Dirt,
    LogEnd,
    Gravel,
    Ice,
    SandstoneSide,
    SandstoneTop,
    SnowSide,
}

pub const TILE_COUNT: usize = 15;

impl Tile {
    // What meshes carry in vertex alpha for `block_texture.wesl`.
    pub fn atlas_cell(self) -> f32 {
        self as u8 as f32
    }

    // Greyscale tiles that vertex colours tint to the climate's grass.
    pub fn tinted(self) -> bool {
        matches!(self, Tile::GrassTop | Tile::Leaves)
    }
}

impl BlockType {
    // The tile a face of this block draws. A flora class would slot in here, one tile for every
    // face, with the mesher branching on its shape before it ever asks for a face.
    pub fn tile(&self, face: Face) -> Tile {
        match (self, face) {
            (BlockType::Air, _) => Tile::Stone, // never drawn
            (BlockType::Stone, _) => Tile::Stone,
            (BlockType::Grass, Face::Top) => Tile::GrassTop,
            (BlockType::Grass, Face::Side) => Tile::GrassSide,
            (BlockType::Grass, Face::Bottom) => Tile::Dirt,
            (BlockType::Sand, _) => Tile::Sand,
            (BlockType::Water, _) => Tile::Water,
            (BlockType::Snow, Face::Side) => Tile::SnowSide,
            (BlockType::Snow, _) => Tile::Snow,
            (BlockType::Log, Face::Side) => Tile::LogBark,
            (BlockType::Log, _) => Tile::LogEnd,
            (BlockType::Leaves, _) => Tile::Leaves,
            (BlockType::Dirt, _) => Tile::Dirt,
            (BlockType::Gravel, _) => Tile::Gravel,
            (BlockType::Ice, _) => Tile::Ice,
            (BlockType::Sandstone, Face::Side) => Tile::SandstoneSide,
            (BlockType::Sandstone, _) => Tile::SandstoneTop,
        }
    }

    // The one tile coarse LODs draw all of a block's faces with: grass stays green on hillsides.
    pub fn lod_tile(&self) -> Tile {
        match self {
            BlockType::Grass => Tile::GrassTop,
            _ => self.tile(Face::Side),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{BlockType, Face, Tile, TILE_COUNT};

    #[test]
    fn test_tile_count_matches_the_last_tile() {
        assert_eq!(TILE_COUNT, Tile::SnowSide as usize + 1);
    }

    #[test]
    fn test_blocks_pick_their_tile_by_face() {
        assert_eq!(Tile::GrassTop, BlockType::Grass.tile(Face::Top));
        assert_eq!(Tile::GrassSide, BlockType::Grass.tile(Face::Side));
        assert_eq!(Tile::Dirt, BlockType::Grass.tile(Face::Bottom));
        assert_eq!(Tile::LogEnd, BlockType::Log.tile(Face::Bottom));
        assert_eq!(Tile::LogBark, BlockType::Log.tile(Face::Side));
        assert_eq!(Tile::GrassTop, BlockType::Grass.lod_tile());
        assert_eq!(Tile::LogBark, BlockType::Log.lod_tile());
    }

    #[test]
    fn test_ice_lets_light_through_but_is_solid() {
        assert!(!BlockType::Ice.is_opaque());
        assert!(BlockType::Ice.light_opacity() > BlockType::Water.light_opacity());
        assert_eq!(1.0, BlockType::Ice.occlusion_weight());
    }

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
