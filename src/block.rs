#[derive(Debug, Default, PartialEq, Eq, Copy, Clone)]
#[repr(u8)]
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
    // ground flora, drawn as two crossed quads, see `BlockType::is_flora`
    TallGrass,
    Fern,
    Poppy,
    Dandelion,
    ShortGrass,
    SeedGrass,
    // stone packed hard under the ground, which only explosives break
    CompactedStone,
}

impl BlockType {
    pub fn occlusion_weight(&self) -> f32 {
        match self {
            BlockType::Air => 0.0,
            _ if self.is_flora() => 0.0,
            BlockType::Water => 0.4,
            BlockType::Leaves => 0.3,
            _ => 1.0,
        }
    }

    // How many levels of sky light this block takes from light passing through it: all of
    // it for opaque blocks, see `light.rs`.
    pub fn light_opacity(&self) -> u8 {
        match self {
            BlockType::Air => 0,
            _ if self.is_flora() => 0,
            BlockType::Leaves => 1,
            BlockType::Water => 2,
            BlockType::Ice => 3,
            _ => crate::chunks::generate::light::MAX_LIGHT,
        }
    }

    // Whether the player is kept out of it: grass and flowers are walked through.
    pub fn is_solid(&self) -> bool {
        !matches!(self, BlockType::Air | BlockType::Water) && !self.is_flora()
    }

    // Whether the faces of blocks behind this one are hidden by it. Leaves have holes.
    pub fn is_opaque(&self) -> bool {
        self.light_opacity() == crate::chunks::generate::light::MAX_LIGHT
    }

    // Plants on the ground: drawn as two quads crossed through the block, not as a cube, and
    // see-through, hiding nothing and shading nothing.
    pub fn is_flora(&self) -> bool {
        matches!(
            self,
            BlockType::TallGrass
                | BlockType::Fern
                | BlockType::Poppy
                | BlockType::Dandelion
                | BlockType::ShortGrass
                | BlockType::SeedGrass
        )
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

// The textures in blocks.png, left to right, in `generate.py`'s `TILES` order.
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
    CompactedStone,
    TallGrass,
    Fern,
    Poppy,
    Dandelion,
    ShortGrass,
    SeedGrass,
}

pub const TILE_COUNT: usize = 22;

impl Tile {
    // What meshes carry in vertex alpha for `block_texture.wesl`.
    pub fn atlas_cell(self) -> f32 {
        self as u8 as f32
    }

    // Greyscale tiles that vertex colours tint to the climate's grass.
    pub fn tinted(self) -> bool {
        matches!(
            self,
            Tile::GrassTop
                | Tile::Leaves
                | Tile::TallGrass
                | Tile::Fern
                | Tile::ShortGrass
                | Tile::SeedGrass
        )
    }

    // The first of the flora sprites, which are the atlas's last tiles, see `BlockType::is_flora`.
    pub const FIRST_FLORA: Tile = Tile::TallGrass;
}

impl BlockType {
    // The tile a face of this block draws. Flora draws its one sprite on both its quads.
    pub fn tile(&self, face: Face) -> Tile {
        match (self, face) {
            (BlockType::Air, _) => Tile::Stone, // never drawn
            (BlockType::Stone, _) => Tile::Stone,
            (BlockType::CompactedStone, _) => Tile::CompactedStone,
            (BlockType::Grass, Face::Top) => Tile::GrassTop,
            (BlockType::Grass, Face::Side) => Tile::GrassSide,
            (BlockType::Grass, Face::Bottom) => Tile::Dirt,
            (BlockType::Sand, _) => Tile::Sand,
            (BlockType::Water, _) => Tile::Water,
            (BlockType::Snow, _) => Tile::Snow,
            (BlockType::Log, Face::Side) => Tile::LogBark,
            (BlockType::Log, _) => Tile::LogEnd,
            (BlockType::Leaves, _) => Tile::Leaves,
            (BlockType::Dirt, _) => Tile::Dirt,
            (BlockType::Gravel, _) => Tile::Gravel,
            (BlockType::Ice, _) => Tile::Ice,
            (BlockType::Sandstone, Face::Side) => Tile::SandstoneSide,
            (BlockType::Sandstone, _) => Tile::SandstoneTop,
            (BlockType::TallGrass, _) => Tile::TallGrass,
            (BlockType::Fern, _) => Tile::Fern,
            (BlockType::Poppy, _) => Tile::Poppy,
            (BlockType::Dandelion, _) => Tile::Dandelion,
            (BlockType::ShortGrass, _) => Tile::ShortGrass,
            (BlockType::SeedGrass, _) => Tile::SeedGrass,
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
        assert_eq!(TILE_COUNT, Tile::SeedGrass as usize + 1);
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
    fn test_snow_is_full_snow_on_every_face_not_snow_over_dirt() {
        for face in [Face::Top, Face::Side, Face::Bottom] {
            assert_eq!(Tile::Snow, BlockType::Snow.tile(face));
        }
        assert_eq!(Tile::Snow, BlockType::Snow.lod_tile());
    }

    #[test]
    fn test_ice_lets_light_through_but_is_solid() {
        assert!(!BlockType::Ice.is_opaque());
        assert!(BlockType::Ice.light_opacity() > BlockType::Water.light_opacity());
        assert_eq!(1.0, BlockType::Ice.occlusion_weight());
    }

    #[test]
    fn test_flora_hides_and_shades_nothing() {
        for flora in [
            BlockType::TallGrass,
            BlockType::Fern,
            BlockType::Poppy,
            BlockType::Dandelion,
            BlockType::ShortGrass,
            BlockType::SeedGrass,
        ] {
            assert!(flora.is_flora() && !flora.is_opaque());
            assert_eq!(0, flora.light_opacity());
            assert_eq!(0.0, flora.occlusion_weight());
            assert!(flora.tile(Face::Side) as u8 >= Tile::FIRST_FLORA as u8);
        }
        assert!(!BlockType::Grass.is_flora() && !BlockType::Leaves.is_flora());
        // the shaders take every tile from the first flora's on as a plant's, see `foliage.wesl`
        assert_eq!(6, TILE_COUNT - Tile::FIRST_FLORA as usize);
    }

    #[test]
    fn test_occlusion_weights() {
        assert_eq!(0.0, BlockType::Air.occlusion_weight());
        assert_eq!(1.0, BlockType::Stone.occlusion_weight());
        let w = BlockType::Water.occlusion_weight();
        assert!(w > 0.0 && w < 1.0);
    }

    #[test]
    fn test_only_solid_blocks_are_opaque() {
        assert!(BlockType::Stone.is_opaque());
        assert!(BlockType::Log.is_opaque());
        assert!(!BlockType::Leaves.is_opaque());
        assert!(!BlockType::Water.is_opaque());
        assert!(!BlockType::Air.is_opaque());
    }
}
