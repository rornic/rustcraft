use std::sync::Arc;

use super::WorldMap;
use crate::chunks::generate::biome::{BiomeId, Biomes, Climate};
use crate::chunks::generate::terrain_source::{TerrainSample, TerrainSource};
use crate::chunks::generate::WATER_LEVEL;

/// Terrain read from a `WorldMap`.
pub struct WorldTerrain {
    map: Arc<WorldMap>,
    biomes: Biomes,
}

impl WorldTerrain {
    pub fn new(map: Arc<WorldMap>) -> Self {
        let biomes = Biomes::new(map.seed);
        Self { map, biomes }
    }

    pub fn map(&self) -> &Arc<WorldMap> {
        &self.map
    }
}

impl TerrainSource for WorldTerrain {
    fn seed(&self) -> u32 {
        self.map.seed
    }

    fn biomes(&self) -> &Biomes {
        &self.biomes
    }

    fn sample_for_cell(&self, x: f64, z: f64, _cell: f64) -> TerrainSample {
        TerrainSample {
            continentalness: self.map.bilinear(&self.map.continentalness, x, z) as f64,
            height: self.map.height_at(x, z) as f64,
            biome: BiomeId::BASE,
            trees: 0.02,
        }
    }

    fn climate(&self, _x: f64, _z: f64) -> Climate {
        Climate::default()
    }

    fn temperature(&self, _x: f64, _z: f64) -> f32 {
        0.5
    }

    fn find_spawn(&self) -> (i64, i64, f64) {
        let map = &self.map;
        let (half, cell) = (map.config.half_size(), map.config.map_cell);
        let mid = map.dim() as i64 / 2;
        for ring in 0..mid {
            for i in -ring..=ring {
                for (dx, dz) in [(i, -ring), (i, ring), (-ring, i), (ring, i)] {
                    let (cx, cz) = ((mid + dx) as usize, (mid + dz) as usize);
                    let above = map.height[map.index(cx, cz)] as f64 - WATER_LEVEL as f64;
                    if (4.0..30.0).contains(&above) {
                        let x = (cx as i64 * cell + cell / 2) - half;
                        let z = (cz as i64 * cell + cell / 2) - half;
                        return (x, z, self.height(x as f64, z as f64));
                    }
                }
            }
        }
        (0, 0, self.height(0.0, 0.0))
    }
}
