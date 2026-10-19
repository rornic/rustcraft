use std::sync::Arc;

use super::climate::BiomeKind;
use super::detail::{detail, dunes};
use super::noise::perlin;
use super::terrain::smoothstep;
use super::water::RiverHit;
use super::WorldMap;
use crate::chunks::generate::biome::{BiomeId, Biomes, Climate};
use crate::chunks::generate::terrain_source::{TerrainSample, TerrainSource, WaterKind};
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
    if ground <= river.surface {
        return ground;
    }
    let t = ((river.distance - half) / bank_width(river)).clamp(0.0, 1.0);
    river.surface + (ground - river.surface) * t * t * (3.0 - 2.0 * t)
}

fn bank_width(river: RiverHit) -> f32 {
    2.0 + river.width * 0.25
}

/// Terrain read from a `WorldMap`, with detail laid over its ground.
pub struct WorldTerrain {
    map: Arc<WorldMap>,
    biomes: Biomes,
    // the biome each `BiomeKind` is, indexed by kind
    kinds: Vec<BiomeId>,
    desert: BiomeId,
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
        let desert = biomes.id("desert").expect("desert in table");
        Self {
            map,
            biomes,
            kinds,
            desert,
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
        let max = (self.map.dim() - 1) as f64;
        let i = self.map.index(
            u.round().clamp(0.0, max) as usize,
            v.round().clamp(0.0, max) as usize,
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
        let map = &self.map;
        let seed = map.seed;
        let sea = WATER_LEVEL as f64;
        let biome = self.biome_at(x, z);
        let base = map.height_at(x, z) as f64;
        let river = if cell <= RIVER_DETAIL_CELL {
            map.river_at(x, z)
        } else {
            None
        };
        let roughness = map.bilinear_bytes(&map.roughness, x, z) as f64 / 255.0;
        let mut detail = detail(seed, x, z, cell, roughness, base - sea);
        if biome == self.desert {
            detail += dunes(seed ^ 0xD05E, x, z, cell, base - sea);
        }
        // the channel of a river is smooth, banks and all
        if let Some(river) = river {
            let half = river.width as f64 / 2.0;
            detail *= smoothstep(half, half + bank_width(river) as f64, river.distance as f64);
        }
        let mut ground = base + detail;
        if let Some(river) = river {
            ground = channel(ground as f32, river) as f64;
        }
        let ground = ground.max(0.0);

        let (water_surface, water_kind) = if ground < sea {
            (WATER_LEVEL as f32, WaterKind::Ocean)
        } else if let Some(river) =
            river.filter(|r| r.distance < r.width / 2.0 && ground < r.surface as f64)
        {
            (river.surface, WaterKind::River)
        } else if let Some(surface) = map.lake_at(x, z, ground as f32) {
            (surface, WaterKind::Lake)
        } else {
            (0.0, WaterKind::None)
        };
        let wet = water_surface > 0.0;
        TerrainSample {
            continentalness: map.bilinear(&map.continentalness, x, z) as f64,
            height: ground,
            biome,
            // nothing grows in water
            trees: if wet {
                0.0
            } else {
                map.bilinear_bytes(&map.trees, x, z) as f64 / 255.0
            },
            water_surface,
            water_kind,
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
                    if (4.0..30.0).contains(&above) && map.water_kind[map.index(cx, cz)] == 0 {
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
