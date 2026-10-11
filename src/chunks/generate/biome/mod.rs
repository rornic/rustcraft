mod desert;
mod foothills;

use bevy::color::{ColorToComponents, LinearRgba, Srgba};
use bevy::math::Vec3;

use crate::block::BlockType;
pub use desert::Desert;
pub use foothills::Foothills;

// How hot and how wet a column is, each 0..1. See `NoiseGenerator::climate`.
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

// A column of the base terrain, before any biome has changed it.
#[derive(Debug, Copy, Clone)]
pub struct BaseColumn {
    pub x: f64,
    pub z: f64,
    pub height: f64,
    // how far inland, negative out to sea
    pub continentalness: f64,
    // where mountain ranges rise: they start at 0 and grow with it
    pub ruggedness: f64,
    pub climate: Climate,
}

// A biome takes over from the base terrain where the climate or lie of the land suits it:
// it can reshape the ground and swap the blocks the base terrain would have used.
pub trait Biome: Send + Sync {
    fn name(&self) -> &'static str;

    // How strongly this biome holds on a column, 0..1. The ground it shapes blends in
    // with this strength, and it owns the column's blocks where it's over 0.5.
    fn strength(&self, column: &BaseColumn) -> f64;

    // The column's height, given the base terrain's.
    fn height(&self, column: &BaseColumn) -> f64 {
        column.height
    }

    // The block to use where the base terrain would put `base`, `above_sea` blocks up, on
    // ground with the given gradient (see `terrain_block`).
    fn block(&self, base: BlockType, _above_sea: i64, _gradient: f64) -> BlockType {
        base
    }
}

// Which biome owns a column, as an index into `Biomes`, or the base terrain.
#[derive(Debug, Default, Copy, Clone, PartialEq, Eq, Hash)]
pub struct BiomeId(u8);

impl BiomeId {
    pub const BASE: BiomeId = BiomeId(0);
}

// Every biome that can take over from the base terrain.
pub struct Biomes {
    biomes: Vec<Box<dyn Biome>>,
}

impl Biomes {
    pub fn new(seed: u32) -> Self {
        // earlier biomes win ties for a column
        Self::with(vec![Box::new(Desert::new(seed)), Box::new(Foothills::new(seed))])
    }

    pub fn with(biomes: Vec<Box<dyn Biome>>) -> Self {
        assert!(biomes.len() < u8::MAX as usize);
        Self { biomes }
    }

    // The column's height and the biome owning it, starting from the base terrain.
    pub fn shape(&self, column: &BaseColumn) -> (f64, BiomeId) {
        let mut height = column.height;
        let (mut owner, mut strongest) = (BiomeId::BASE, 0.5);
        for (i, biome) in self.biomes.iter().enumerate() {
            let strength = biome.strength(column);
            if strength <= 0.0 {
                continue;
            }
            height += (biome.height(column) - column.height) * strength;
            if strength > strongest {
                (owner, strongest) = (BiomeId(i as u8 + 1), strength);
            }
        }
        (height, owner)
    }

    // The biome's name, or None for the base terrain.
    pub fn name(&self, biome: BiomeId) -> Option<&'static str> {
        biome.0.checked_sub(1).map(|i| self.biomes[i as usize].name())
    }

    // The block `biome` uses in place of the base terrain's `base`.
    pub fn block(&self, biome: BiomeId, base: BlockType, above_sea: i64, gradient: f64) -> BlockType {
        match biome.0 {
            0 => base,
            i => self.biomes[i as usize - 1].block(base, above_sea, gradient),
        }
    }
}

// 0 at `edge0`, rising smoothly to 1 at `edge1`; `edge1` may be below `edge0` to fall instead.
fn smoothstep(edge0: f64, edge1: f64, x: f64) -> f64 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

// Grass at the four corners of the climate square, in sRGB: hot and dry is parched
// yellow, hot and wet lush green, and the cold side duller versions of both.
const GRASS_HOT_DRY: [f32; 3] = [200.0, 170.0, 60.0];
const GRASS_HOT_WET: [f32; 3] = [40.0, 130.0, 12.0];
const GRASS_COLD_DRY: [f32; 3] = [140.0, 140.0, 60.0];
const GRASS_COLD_WET: [f32; 3] = [30.0, 96.0, 24.0];

// The mean linear value of the greyscale grass tile in blocks.png, which vertex colours tint.
const GRASS_TEXTURE_MEAN: f32 = 0.5;

// Linear colour of grass in this climate.
pub fn grass_colour(climate: Climate) -> Vec3 {
    let srgb = |c: [f32; 3]| Vec3::from(c) / 255.0;
    let (t, h) = (climate.temperature.clamp(0.0, 1.0), climate.humidity.clamp(0.0, 1.0));
    let dry = srgb(GRASS_COLD_DRY).lerp(srgb(GRASS_HOT_DRY), t);
    let wet = srgb(GRASS_COLD_WET).lerp(srgb(GRASS_HOT_WET), t);
    let colour = dry.lerp(wet, h);
    LinearRgba::from(Srgba::rgb(colour.x, colour.y, colour.z)).to_vec3()
}

// What to multiply the grass texture by to draw `grass_colour`.
pub fn grass_tint(climate: Climate) -> Vec3 {
    grass_colour(climate) / GRASS_TEXTURE_MEAN
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Raise(f64);

    impl Biome for Raise {
        fn name(&self) -> &'static str {
            "raise"
        }

        fn strength(&self, column: &BaseColumn) -> f64 {
            column.climate.temperature as f64
        }

        fn height(&self, column: &BaseColumn) -> f64 {
            column.height + self.0
        }

        fn block(&self, _base: BlockType, _above_sea: i64, _gradient: f64) -> BlockType {
            BlockType::Snow
        }
    }

    fn column(temperature: f32) -> BaseColumn {
        BaseColumn {
            x: 0.0,
            z: 0.0,
            height: 500.0,
            continentalness: 0.3,
            ruggedness: -1.0,
            climate: Climate { temperature, humidity: 0.5 },
        }
    }

    #[test]
    fn test_biome_height_blends_in_with_strength() {
        let biomes = Biomes::with(vec![Box::new(Raise(10.0))]);
        assert_eq!((500.0, BiomeId::BASE), biomes.shape(&column(0.0)));
        assert_eq!(502.5, biomes.shape(&column(0.25)).0);
        assert_eq!(510.0, biomes.shape(&column(1.0)).0);
    }

    #[test]
    fn test_biome_owns_blocks_where_it_holds_over_half() {
        let biomes = Biomes::with(vec![Box::new(Raise(10.0))]);
        let owner = |t| biomes.shape(&column(t)).1;
        assert_eq!(BlockType::Grass, biomes.block(owner(0.4), BlockType::Grass, 10, 0.0));
        assert_eq!(BlockType::Snow, biomes.block(owner(0.6), BlockType::Grass, 10, 0.0));
    }

    #[test]
    fn test_strongest_biome_owns_the_column() {
        let biomes = Biomes::with(vec![Box::new(Raise(10.0)), Box::new(Desert::new(1))]);
        let hot_dry = BaseColumn { climate: Climate { temperature: 1.0, humidity: 0.0 }, ..column(1.0) };
        let (_, owner) = biomes.shape(&hot_dry);
        // both hold fully, so the first listed keeps it
        assert_eq!(BiomeId(1), owner);
        assert_eq!(Some("raise"), biomes.name(owner));
        assert_eq!(None, biomes.name(BiomeId::BASE));
    }

    fn climate(temperature: f32, humidity: f32) -> Climate {
        Climate { temperature, humidity }
    }

    // How much yellower than green, as red over green.
    fn yellowness(colour: Vec3) -> f32 {
        colour.x / colour.y
    }

    #[test]
    fn test_hot_dry_grass_is_yellower_than_wet() {
        let dry = grass_colour(climate(1.0, 0.0));
        let wet = grass_colour(climate(1.0, 1.0));
        assert!(yellowness(dry) > 0.8, "dry grass should be yellow, was {dry}");
        assert!(yellowness(wet) < 0.2, "wet grass should be green, was {wet}");
    }

    #[test]
    fn test_drier_and_hotter_is_always_yellower() {
        for step in 0..10 {
            let x = step as f32 / 10.0;
            let y = x + 0.1;
            assert!(yellowness(grass_colour(climate(0.5, x))) > yellowness(grass_colour(climate(0.5, y))));
            assert!(yellowness(grass_colour(climate(y, 0.3))) > yellowness(grass_colour(climate(x, 0.3))));
        }
    }

    #[test]
    fn test_tint_draws_grass_colour_from_texture() {
        let c = climate(0.3, 0.8);
        assert!((grass_tint(c) * GRASS_TEXTURE_MEAN - grass_colour(c)).length() < 1e-6);
    }
}
