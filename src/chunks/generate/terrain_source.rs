use super::biome::{BiomeId, Biomes, Climate};

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
    /// Where the water's surface is over the column, or 0 if it's dry: sea, lake or river.
    pub water_surface: f32,
    pub water_kind: WaterKind,
    pub spring: SpringPart,
}

/// The part of a spring, where a river rises, a column is in.
#[derive(Debug, Default, Copy, Clone, PartialEq, Eq)]
pub enum SpringPart {
    #[default]
    None,
    /// The ring of ground round the pool.
    Rim,
    /// The pool the river rises in.
    Pool,
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

    /// The most tree density the column at (x, z) can have, without sampling its terrain, so
    /// callers can rule out trees cheaply. 1 if it can't say.
    fn tree_bound(&self, _x: f64, _z: f64) -> f64 {
        1.0
    }

    fn sample(&self, x: f64, z: f64) -> TerrainSample {
        self.sample_for_cell(x, z, 1.0)
    }

    /// Column height in blocks at a block position.
    fn get(&self, pos: bevy::math::I64Vec2) -> f64 {
        self.height(pos.x as f64, pos.y as f64)
    }

    fn height(&self, x: f64, z: f64) -> f64 {
        self.sample(x, z).height
    }

    fn biome(&self, x: f64, z: f64) -> BiomeId {
        self.sample(x, z).biome
    }

    /// The water over the column at (x, z).
    fn water(&self, x: f64, z: f64) -> Water {
        let sample = self.sample(x, z);
        if sample.water_surface > 0.0 {
            Water {
                kind: sample.water_kind,
                surface: sample.water_surface,
                depth: sample.water_surface - sample.height as f32,
            }
        } else {
            Water::default()
        }
    }
}

impl<T: TerrainSource + ?Sized> TerrainSource for std::sync::Arc<T> {
    fn seed(&self) -> u32 {
        (**self).seed()
    }

    fn biomes(&self) -> &Biomes {
        (**self).biomes()
    }

    fn sample_for_cell(&self, x: f64, z: f64, cell: f64) -> TerrainSample {
        (**self).sample_for_cell(x, z, cell)
    }

    fn climate(&self, x: f64, z: f64) -> Climate {
        (**self).climate(x, z)
    }

    fn temperature(&self, x: f64, z: f64) -> f32 {
        (**self).temperature(x, z)
    }

    fn find_spawn(&self) -> (i64, i64, f64) {
        (**self).find_spawn()
    }

    fn tree_bound(&self, x: f64, z: f64) -> f64 {
        (**self).tree_bound(x, z)
    }
}
