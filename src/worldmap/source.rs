use std::sync::Arc;

use super::climate::{BiomeKind, LAPSE_PER_BLOCK};
use super::detail::{detail, dunes, terraced};
use super::noise::perlin;
use super::terrain::smoothstep;
use super::water::RiverHit;
use super::FeatureKind;
use super::WorldMap;
use crate::chunks::generate::biome::{BiomeId, Biomes, Climate};
use crate::chunks::generate::terrain_source::{TerrainSample, TerrainSource, WaterKind};
use crate::chunks::generate::WATER_LEVEL;

// How far under the land's ground, in blocks, a river's water lies at most.
const FREEBOARD: f32 = 0.4;

// Cells at least this wide leave a river's channel to the map's own carving.
const RIVER_DETAIL_CELL: f64 = 16.0;

// The ground over a river's channel: a parabolic bed up to the water's edge, then the bank
// easing out into the ground beside it.
fn channel(ground: f32, river: RiverHit) -> f32 {
    let half = river.width / 2.0;
    if river.distance < half {
        return ground.min(bed_height(river));
    }
    // ground above the water is cut down to it at the water's edge and eases back up to the
    // land's own height; ground already lower is left, never built up
    if ground <= river.surface {
        return ground;
    }
    let t = ((river.distance - half) / bank_width(river)).clamp(0.0, 1.0);
    river.surface + (ground - river.surface) * t * t * (3.0 - 2.0 * t)
}

// The river with its water no higher than `FREEBOARD` under the land's ground `base`, and the
// bed the same depth under that as before.
fn sunk_into(river: RiverHit, base: f64) -> RiverHit {
    let depth = river.surface - river.bed;
    let surface = river.surface.min(base as f32 - FREEBOARD);
    RiverHit {
        surface,
        bed: surface - depth,
        ..river
    }
}

// Where the bed of the river under it is, a parabola across it.
fn bed_height(river: RiverHit) -> f32 {
    let t = (river.distance / (river.width / 2.0)).min(1.0);
    river.bed + (river.surface - river.bed) * t * t
}

fn bank_width(river: RiverHit) -> f32 {
    2.0 + river.width * 0.25
}

// How far a river's line is moved off its straight run between cell centres at a point: slow
// bends a few hundred blocks long, with smaller wiggles on them.
fn meander(seed: u32, x: f64, z: f64) -> (f64, f64) {
    let bend = |salt: u32| {
        perlin(seed ^ salt, x / 260.0, z / 260.0) * 34.0
            + perlin(seed ^ salt ^ 0x55, x / 80.0, z / 80.0) * 9.0
    };
    (bend(0x3EA1), bend(0x3EA2))
}

// Which cells have a river or lake within `NEAR` cells, which is as far as a river's banks, a
// meander and a lake's shore reach.
fn near_water(map: &WorldMap) -> Vec<bool> {
    const NEAR: i64 = 4;
    let dim = map.dim() as i64;
    let mut near = vec![false; map.height.len()];
    for cz in 0..dim {
        for cx in 0..dim {
            let i = (cz * dim + cx) as usize;
            if map.river_width[i] <= 0.0 && map.water_kind[i] != 2 {
                continue;
            }
            for z in (cz - NEAR).max(0)..=(cz + NEAR).min(dim - 1) {
                for x in (cx - NEAR).max(0)..=(cx + NEAR).min(dim - 1) {
                    near[(z * dim + x) as usize] = true;
                }
            }
        }
    }
    near
}

/// Terrain read from a `WorldMap`, with detail laid over its ground.
pub struct WorldTerrain {
    map: Arc<WorldMap>,
    biomes: Biomes,
    // the biome each `BiomeKind` is, indexed by kind
    kinds: Vec<BiomeId>,
    desert: BiomeId,
    mountain: BiomeId,
    // cells with a river or lake within a few cells of them, so the rest skip looking for water
    near_water: Vec<bool>,
    // the way each river cell's water leaves it, see `WorldMap::river_tangents`
    tangents: Vec<[f32; 2]>,
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
        let mountain = biomes.id("mountain").expect("mountain in table");
        let near_water = near_water(&map);
        let tangents = map.river_tangents();
        Self {
            map,
            biomes,
            kinds,
            desert,
            mountain,
            near_water,
            tangents,
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
        let near_water = {
            let (u, v) = map.to_cell(x, z);
            let max = (map.dim() - 1) as f64;
            self.near_water[map.index(
                u.round().clamp(0.0, max) as usize,
                v.round().clamp(0.0, max) as usize,
            )]
        };
        // the nearest river and, where one joins another, the other too
        let rivers = if near_water && cell <= RIVER_DETAIL_CELL {
            let (mx, mz) = meander(seed, x, z);
            map.river_hits(&self.tangents, x + mx, z + mz)
        } else {
            [None, None]
        };
        let roughness = map.bilinear_bytes(&map.roughness, x, z) as f64 / 255.0;
        let mut detail = detail(seed, x, z, cell, roughness, base - sea);
        if biome == self.desert {
            detail += dunes(seed ^ 0xD05E, x, z, cell, base - sea);
        }
        // lake floors and shores stay as the map has them, or the detail would stripe the water's edge
        if near_water {
            if let Some(surface) = map.lake_at(x, z, f32::NEG_INFINITY) {
                detail *= smoothstep(0.0, 6.0, base - surface as f64);
            }
        }
        // the channel of a river is smooth, banks and all
        for river in rivers.iter().flatten() {
            let half = river.width as f64 / 2.0;
            detail *= smoothstep(
                half,
                half + bank_width(*river) as f64,
                river.distance as f64,
            );
        }
        // the water lies just under the land's smooth ground where it is, and the bed is cut down
        // from there, so a river is carved into the terrain, and never stands above or beside
        // ground lower than itself
        let rivers = rivers.map(|r| r.map(|r| sunk_into(r, base)));
        let mut ground = terraced(
            seed,
            x,
            z,
            biome == self.mountain,
            base + detail,
            base - sea,
        );
        if let Some(river) = rivers[0] {
            ground = channel(ground as f32, river) as f64;
        }
        // a river the nearest one meets still has its bed cut where its water is, so a stream is
        // not cut off short of the bank of the river it runs into
        if let Some(joining) = rivers[1].filter(|r| r.distance < r.width / 2.0) {
            ground = ground.min(bed_height(joining) as f64);
        }
        let ground = ground.max(0.0);

        let (water_surface, water_kind) = if ground < sea {
            (WATER_LEVEL as f32, WaterKind::Ocean)
        } else if let Some(river) = rivers
            .iter()
            .flatten()
            .filter(|r| r.distance < r.width / 2.0 && ground < r.surface as f64)
            // where two meet, the wider river's level is the water's
            .max_by(|a, b| a.width.total_cmp(&b.width))
        {
            (river.surface, WaterKind::River)
        } else if let Some(surface) = near_water
            .then(|| map.lake_at(x, z, ground as f32))
            .flatten()
        {
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

    // How warm it is at the ground here, which falls with height at the rate snow lines climb at,
    // so trees and grass higher up are cooler, and pines grow on the slopes of a range.
    fn climate(&self, x: f64, z: f64) -> Climate {
        let above_sea = (self.map.height_at(x, z) - WATER_LEVEL as f32).max(0.0);
        Climate {
            temperature: (self.temperature(x, z) - above_sea * LAPSE_PER_BLOCK).clamp(0.0, 1.0),
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

    // The spawn region nearest the world's middle.
    // The spawn region nearest the world's middle.
    fn find_spawn(&self) -> (i64, i64, f64) {
        let nearest = self
            .map
            .features
            .iter()
            .filter(|f| f.kind == FeatureKind::Spawn)
            .min_by_key(|f| (f.x as i64).abs() + (f.z as i64).abs());
        let (x, z) = nearest.map_or((0, 0), |f| (f.x as i64, f.z as i64));
        (x, z, self.height(x as f64, z as f64))
    }
}
