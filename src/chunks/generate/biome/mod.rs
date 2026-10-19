mod desert;
mod forest;
mod glacier;
mod plain;

use bevy::color::{ColorToComponents, LinearRgba, Srgba};
use bevy::math::Vec3;

use crate::block::BlockType;
pub use desert::Desert;
pub use forest::Forest;
pub use glacier::Glacier;
pub use plain::Plain;

// How hot and how wet a column is, each 0..1. See `TerrainSource::climate`.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct Climate {
    pub temperature: f32,
    pub humidity: f32,
}

impl Default for Climate {
    fn default() -> Self {
        Self {
            temperature: 0.5,
            humidity: 0.5,
        }
    }
}

// What a biome changes about the ground and what grows on it: the blocks it uses in place of
// the base terrain's, and how thickly trees grow.
pub trait Biome: Send + Sync {
    fn name(&self) -> &'static str;

    // The chance, 0..1, of a tree on each spot one could grow, or None to leave the base
    // terrain's. Trees only grow on grass.
    fn tree_density(&self) -> Option<f64> {
        None
    }

    // The block to use where the base terrain would put `base`, `above_sea` blocks up, on
    // ground with the given gradient (see `terrain_block`).
    fn block(&self, base: BlockType, _above_sea: i64, _gradient: f64) -> BlockType {
        base
    }
}

// Which biome a column is, as an index into `Biomes`, or the base terrain.
#[derive(Debug, Default, Copy, Clone, PartialEq, Eq, Hash)]
pub struct BiomeId(u8);

impl BiomeId {
    pub const BASE: BiomeId = BiomeId(0);
}

// The base terrain's scattering of lone trees, see `Biome::tree_density`.
pub(super) const BASE_TREE_DENSITY: f64 = 0.02;

// Every biome a world map can name.
pub struct Biomes {
    biomes: Vec<Box<dyn Biome>>,
}

impl Biomes {
    pub fn new(_seed: u32) -> Self {
        Self::with(vec![
            Box::new(Glacier),
            Box::new(Desert),
            Box::new(Forest),
            // placeholder kept so ids stay put, see `worldmap::climate::BiomeKind`
            Box::new(Plain::new("foothills", BASE_TREE_DENSITY)),
            Box::new(Plain::new("tundra", 0.01)),
            Box::new(Plain::new("taiga", 0.3)),
            Box::new(Plain::new("steppe", 0.004)),
            Box::new(Plain::new("savanna", 0.03)),
            Box::new(Plain::new("jungle", 0.4)),
        ])
    }

    pub fn with(biomes: Vec<Box<dyn Biome>>) -> Self {
        assert!(biomes.len() < u8::MAX as usize);
        Self { biomes }
    }

    /// The biome called `name`.
    pub fn id(&self, name: &str) -> Option<BiomeId> {
        self.biomes
            .iter()
            .position(|b| b.name() == name)
            .map(|i| BiomeId(i as u8 + 1))
    }

    /// How thickly trees grow in `biome`, 0..1.
    pub fn tree_density(&self, biome: BiomeId) -> f64 {
        match biome.0.checked_sub(1) {
            None => BASE_TREE_DENSITY,
            Some(i) => self.biomes[i as usize]
                .tree_density()
                .unwrap_or(BASE_TREE_DENSITY),
        }
    }

    // The biome's name, or None for the base terrain.
    pub fn name(&self, biome: BiomeId) -> Option<&'static str> {
        biome
            .0
            .checked_sub(1)
            .map(|i| self.biomes[i as usize].name())
    }

    // The block `biome` uses in place of the base terrain's `base`.
    pub fn block(
        &self,
        biome: BiomeId,
        base: BlockType,
        above_sea: i64,
        gradient: f64,
    ) -> BlockType {
        match biome.0 {
            0 => base,
            i => self.biomes[i as usize - 1].block(base, above_sea, gradient),
        }
    }
}

// Grass at the four corners of the climate square, in sRGB: hot and dry is parched
// yellow, hot and wet lush green, and the cold side duller versions of both.
const GRASS_HOT_DRY: [f32; 3] = [200.0, 170.0, 60.0];
const GRASS_HOT_WET: [f32; 3] = [40.0, 130.0, 12.0];
const GRASS_COLD_DRY: [f32; 3] = [140.0, 140.0, 60.0];
const GRASS_COLD_WET: [f32; 3] = [30.0, 96.0, 24.0];

// The mean linear value of the greyscale grass and leaves tiles in blocks.png, which vertex
// colours tint: leaves share grass's tint, and their darker tile makes them a deeper green.
const GRASS_TEXTURE_MEAN: f32 = 0.5;
const LEAF_TEXTURE_MEAN: f32 = 0.25;

// Linear colour of grass in this climate.
pub fn grass_colour(climate: Climate) -> Vec3 {
    let srgb = |c: [f32; 3]| Vec3::from(c) / 255.0;
    let (t, h) = (
        climate.temperature.clamp(0.0, 1.0),
        climate.humidity.clamp(0.0, 1.0),
    );
    let dry = srgb(GRASS_COLD_DRY).lerp(srgb(GRASS_HOT_DRY), t);
    let wet = srgb(GRASS_COLD_WET).lerp(srgb(GRASS_HOT_WET), t);
    let colour = dry.lerp(wet, h);
    LinearRgba::from(Srgba::rgb(colour.x, colour.y, colour.z)).to_vec3()
}

// Linear colour of leaves in this climate.
pub fn leaf_colour(climate: Climate) -> Vec3 {
    grass_tint(climate) * LEAF_TEXTURE_MEAN
}

// What to multiply the grass texture by to draw `grass_colour`.
pub fn grass_tint(climate: Climate) -> Vec3 {
    grass_colour(climate) / GRASS_TEXTURE_MEAN
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_biomes_are_found_by_name_and_know_their_trees() {
        let biomes = Biomes::new(1);
        let forest = biomes.id("forest").unwrap();
        assert_eq!(Some("forest"), biomes.name(forest));
        assert_eq!(None, biomes.name(BiomeId::BASE));
        assert!(biomes.tree_density(forest) > biomes.tree_density(BiomeId::BASE) * 10.0);
        assert_eq!(0.0, biomes.tree_density(biomes.id("desert").unwrap()));
        assert!(biomes.id("atlantis").is_none());
    }

    #[test]
    fn test_a_biome_owns_the_blocks_it_swaps() {
        let biomes = Biomes::new(1);
        let desert = biomes.id("desert").unwrap();
        assert_eq!(
            BlockType::Sand,
            biomes.block(desert, BlockType::Grass, 10, 0.0)
        );
        assert_eq!(
            BlockType::Grass,
            biomes.block(BiomeId::BASE, BlockType::Grass, 10, 0.0)
        );
    }

    fn climate(temperature: f32, humidity: f32) -> Climate {
        Climate {
            temperature,
            humidity,
        }
    }

    // How much yellower than green, as red over green.
    fn yellowness(colour: Vec3) -> f32 {
        colour.x / colour.y
    }

    #[test]
    fn test_hot_dry_grass_is_yellower_than_wet() {
        let dry = grass_colour(climate(1.0, 0.0));
        let wet = grass_colour(climate(1.0, 1.0));
        assert!(
            yellowness(dry) > 0.8,
            "dry grass should be yellow, was {dry}"
        );
        assert!(
            yellowness(wet) < 0.2,
            "wet grass should be green, was {wet}"
        );
    }

    #[test]
    fn test_drier_and_hotter_is_always_yellower() {
        for step in 0..10 {
            let x = step as f32 / 10.0;
            let y = x + 0.1;
            assert!(
                yellowness(grass_colour(climate(0.5, x)))
                    > yellowness(grass_colour(climate(0.5, y)))
            );
            assert!(
                yellowness(grass_colour(climate(y, 0.3)))
                    > yellowness(grass_colour(climate(x, 0.3)))
            );
        }
    }

    #[test]
    fn test_tint_draws_grass_colour_from_texture() {
        let c = climate(0.3, 0.8);
        assert!((grass_tint(c) * GRASS_TEXTURE_MEAN - grass_colour(c)).length() < 1e-6);
    }
}
