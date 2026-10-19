use super::biome::{BiomeId, Biomes, Climate};
use super::WATER_LEVEL;

/// What covers a column's ground with water.
#[derive(Debug, Default, Copy, Clone, PartialEq, Eq)]
pub enum WaterKind {
    #[default]
    None,
    Ocean,
    Lake,
    River,
}

/// The water over a column: its surface height in blocks and how deep it is there.
#[derive(Debug, Default, Copy, Clone, PartialEq)]
pub struct Water {
    pub kind: WaterKind,
    pub surface: f32,
    pub depth: f32,
}

// A column's height, with how far inland it is (negative out to sea), the biome owning it
// and how thickly trees grow there.
#[derive(Debug, Default, Copy, Clone)]
pub struct TerrainSample {
    pub continentalness: f64,
    pub height: f64,
    pub biome: BiomeId,
    pub trees: f64,
}

/// Where the ground, its biomes and its water come from. Chunk generation and LOD read terrain
/// only through this.
pub trait TerrainSource: Send + Sync {
    fn seed(&self) -> u32;

    fn biomes(&self) -> &Biomes;

    /// The terrain as a cell `cell` blocks wide, sampled at its centre, should see it: features
    /// too fine for cells that wide fade to their average. The same as `sample` for single blocks.
    fn sample_for_cell(&self, x: f64, z: f64, cell: f64) -> TerrainSample;

    /// How hot and wet the column at (x, z) is.
    fn climate(&self, x: f64, z: f64) -> Climate;

    /// How hot the column at (x, z) is, 0..1.
    fn temperature(&self, x: f64, z: f64) -> f32;

    /// The nearest lowland column to the world centre.
    fn find_spawn(&self) -> (i64, i64, f64);

    fn sample(&self, x: f64, z: f64) -> TerrainSample {
        self.sample_for_cell(x, z, 1.0)
    }

    fn height(&self, x: f64, z: f64) -> f64 {
        self.sample(x, z).height
    }

    fn biome(&self, x: f64, z: f64) -> BiomeId {
        self.sample(x, z).biome
    }

    /// The water over the column at (x, z), the sea where the ground is below it.
    fn water(&self, x: f64, z: f64) -> Water {
        let height = self.height(x, z);
        if height < WATER_LEVEL as f64 {
            Water {
                kind: WaterKind::Ocean,
                surface: WATER_LEVEL as f32,
                depth: (WATER_LEVEL as f64 - height) as f32,
            }
        } else {
            Water::default()
        }
    }
}
