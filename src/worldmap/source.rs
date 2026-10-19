use std::sync::Arc;

use super::climate::BiomeKind;
use super::noise::perlin;
use super::water::RiverHit;
use super::WorldMap;
use crate::chunks::generate::biome::{BiomeId, Biomes, Climate};
use crate::chunks::generate::terrain_source::{TerrainSample, TerrainSource, Water, WaterKind};
use crate::chunks::generate::WATER_LEVEL;

// Cells at least this wide leave a river's channel to the map's own carving.
const RIVER_DETAIL_CELL: f64 = 16.0;

// The ground over a river's channel: a parabolic bed up to the water's edge, then the bank
// easing out into the ground beside it.
fn channel(ground: f32, river: RiverHit) -> f32 {
    let half = river.width / 2.0;
    let depth = river.surface - river.bed;
    if river.distance < half {
        let t = river.distance / half;
        return ground.min(river.bed + depth * t * t);
    }
    let bank = 2.0 + half * 0.5;
    let t = ((river.distance - half) / bank).clamp(0.0, 1.0);
    if ground <= river.surface {
        return ground;
    }
    river.surface + (ground - river.surface) * t * t * (3.0 - 2.0 * t)
}

/// Terrain read from a `WorldMap`.
pub struct WorldTerrain {
    map: Arc<WorldMap>,
    biomes: Biomes,
    // the biome each `BiomeKind` is, indexed by kind
    kinds: Vec<BiomeId>,
}

impl WorldTerrain {
    pub fn new(map: Arc<WorldMap>) -> Self {
        let biomes = Biomes::new(map.seed);
        let kinds = BiomeKind::ALL
            .iter()
            .map(|k| {
                k.name().map_or(BiomeId::BASE, |name| {
                    biomes.id(name).expect("biome in table")
                })
            })
            .collect();
        Self { map, biomes, kinds }
    }

    // The ground at a point, with the channel of any river through it cut in. Cells wider than a
    // river's banks see only the carving the map itself has.
    fn ground(&self, x: f64, z: f64, cell: f64) -> f64 {
        let ground = self.map.height_at(x, z);
        if cell > RIVER_DETAIL_CELL {
            return ground as f64;
        }
        match self.map.river_at(x, z) {
            Some(river) => channel(ground, river) as f64,
            None => ground as f64,
        }
    }

    // The biome of the cell nearest a point, found a little to one side by a wandering offset so
    // the edges between biomes wander too, rather than following the cells' squares.
    fn biome_at(&self, x: f64, z: f64) -> BiomeId {
        let seed = self.map.seed;
        let wander = self.map.config.map_cell as f64 * 0.6;
        let jx = perlin(seed ^ 0x51A7, x / 40.0, z / 40.0) * wander;
        let jz = perlin(seed ^ 0xB10E, x / 40.0, z / 40.0) * wander;
        let (u, v) = self.map.to_cell(x + jx, z + jz);
        let max = self.map.dim() as i64 - 1;
        let i = self.map.index(
            u.round().clamp(0.0, max as f64) as usize,
            v.round().clamp(0.0, max as f64) as usize,
        );
        self.kinds[self.map.biome[i] as usize]
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

    fn sample_for_cell(&self, x: f64, z: f64, cell: f64) -> TerrainSample {
        TerrainSample {
            continentalness: self.map.bilinear(&self.map.continentalness, x, z) as f64,
            height: self.ground(x, z, cell),
            biome: self.biome_at(x, z),
            trees: self.map.bilinear_bytes(&self.map.trees, x, z) as f64 / 255.0,
        }
    }

    fn water(&self, x: f64, z: f64) -> Water {
        let ground = self.ground(x, z, 1.0);
        if ground < WATER_LEVEL as f64 {
            return Water {
                kind: WaterKind::Ocean,
                surface: WATER_LEVEL as f32,
                depth: (WATER_LEVEL as f64 - ground) as f32,
            };
        }
        let ground = ground as f32;
        if let Some(river) = self.map.river_at(x, z) {
            if river.distance < river.width / 2.0 && ground < river.surface {
                return Water {
                    kind: WaterKind::River,
                    surface: river.surface,
                    depth: river.surface - ground,
                };
            }
        }
        match self.map.lake_at(x, z, ground) {
            Some(surface) => Water {
                kind: WaterKind::Lake,
                surface,
                depth: surface - ground,
            },
            None => Water::default(),
        }
    }

    fn climate(&self, x: f64, z: f64) -> Climate {
        Climate {
            temperature: self.temperature(x, z),
            humidity: self.map.bilinear(&self.map.moisture, x, z).clamp(0.0, 1.0),
        }
    }

    fn temperature(&self, x: f64, z: f64) -> f32 {
        self.map
            .bilinear(&self.map.temperature, x, z)
            .clamp(0.0, 1.0)
    }

    fn tree_bound(&self, x: f64, z: f64) -> f64 {
        self.map.bilinear_bytes(&self.map.trees, x, z) as f64 / 255.0
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
