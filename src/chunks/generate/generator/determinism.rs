//! Separate machines will build the same chunks from the same map and seed, and any difference
//! shows as ghost blocks where their chunks meet. This pins the blocks of a fixed set of chunks.
//!
//! The map is made here from integer-hash noise and arithmetic alone, not by `worldmap::generate`:
//! generation may use `exp` and `powf`, which can differ in the last bit between x86 and ARM, and
//! it is the map, not its generation, that machines share. So this test gives the same answer
//! on every machine, and fails on any where chunk generation from a map is not exact.

use std::sync::Arc;

use super::generate_chunk;
use crate::block::BlockType;
use crate::chunks::chunk::{ChunkCoordinate, CELLS};
use crate::chunks::generate::region_store::RegionStore;
use crate::chunks::generate::WATER_LEVEL;
use crate::worldmap::climate::BiomeKind;
use crate::worldmap::noise::{fbm, hash, ridged};
use crate::worldmap::{WaterCell, WorldConfig, WorldMap, WorldTerrain};
use bevy::math::I64Vec3;

const SEED: u32 = 20_231;
const DIM: usize = 256;

// A world 8 km across with hills and a range, a lake, a river that widens to the sea, and a
// biome in each quarter, every number made from `noise` and sums and products.
fn synthetic_map() -> WorldMap {
    let config = WorldConfig {
        size: DIM as i64 * 32,
        map_cell: 32,
    };
    let mut map = WorldMap::empty(SEED, config);
    for cz in 0..DIM {
        for cx in 0..DIM {
            let i = map.index(cx, cz);
            let (x, z) = map.cell_centre(cx, cz);
            let range = ridged(SEED, x, z, 1800.0, 4);
            let hills = fbm(SEED + 1, x, z, 900.0, 4);
            // falls away to sea at the world's north edge
            let coast = (cz as f64 / 40.0).min(1.0);
            map.height[i] =
                (WATER_LEVEL as f64 - 30.0 + coast * (62.0 + hills * 35.0 + range * 220.0)) as f32;
            map.continentalness[i] = coast as f32;
            let quarter = (cx >= DIM / 2) as u8 + 2 * (cz >= DIM / 2) as u8;
            map.biome[i] = [
                BiomeKind::Forest,
                BiomeKind::Desert,
                BiomeKind::Taiga,
                BiomeKind::Grassland,
            ][quarter as usize] as u8;
            map.trees[i] = [150, 0, 110, 5][quarter as usize];
            map.temperature[i] = 0.55 + 0.1 * quarter as f32;
            map.moisture[i] = 0.3 + 0.15 * quarter as f32;
            map.roughness[i] = (hash(SEED, cx as i32, cz as i32) % 200) as u8 + 30;
            if map.height[i] < WATER_LEVEL as f32 {
                map.water_kind[i] = WaterCell::Ocean as u8;
                map.biome[i] = BiomeKind::Sea as u8;
                map.trees[i] = 0;
            }
        }
    }
    // a lake in a hollow
    for cz in 100..112 {
        for cx in 60..72 {
            let i = map.index(cx, cz);
            map.height[i] =
                505.0 - 0.1 * ((cx as f32 - 66.0).powi(2) + (cz as f32 - 106.0).powi(2));
            map.water_kind[i] = WaterCell::Lake as u8;
            map.water_surface[i] = 504.0;
        }
    }
    // a river from the lake east along row 106, wider as it goes, then north down to the sea
    let mut path = Vec::new();
    path.extend((72..200).map(|cx| (cx, 106)));
    path.extend((0..106).rev().map(|cz| (200, cz)));
    for (n, &(cx, cz)) in path.iter().enumerate() {
        let i = map.index(cx, cz);
        let bed = 500.0 - n as f32 * 0.35;
        if bed < WATER_LEVEL as f32 - 5.0 {
            break;
        }
        map.height[i] = bed;
        map.water_kind[i] = WaterCell::River as u8;
        map.water_surface[i] = bed + 3.0;
        map.river_width[i] = 4.0 + n as f32 * 0.12;
        map.down[i] = match path.get(n + 1) {
            Some(&(nx, nz)) => crate::worldmap::NEIGHBOURS
                .iter()
                .position(|&d| d == (nx as i64 - cx as i64, nz as i64 - cz as i64))
                .unwrap() as u8,
            None => crate::worldmap::NO_FLOW,
        };
    }
    map
}

// FNV-1a over a chunk's blocks in cell order.
fn hash_chunks(regions: &Arc<RegionStore>, chunks: &[(i64, i64, i64)]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    let mut cells = [BlockType::Air; CELLS];
    for &(x, y, z) in chunks {
        generate_chunk(regions.clone(), ChunkCoordinate(I64Vec3::new(x, y, z)))
            .decode_into(&mut cells);
        for block in cells {
            h = (h ^ block as u8 as u64).wrapping_mul(0x0100_0000_01b3);
        }
    }
    h
}

// Chunks over each kind of ground: the lake, the river's reaches, a range, each biome and the coast,
// each as the surface chunk and the one under it.
fn chunk_set(map: &WorldMap) -> Vec<(i64, i64, i64)> {
    let cells = [
        (66, 106),
        (100, 106),
        (150, 106),
        (200, 60),
        (200, 5),
        (30, 30),
        (200, 30),
        (30, 200),
        (200, 200),
        (128, 128),
        (20, 8),
        (230, 12),
    ];
    cells
        .into_iter()
        .flat_map(|(cx, cz)| {
            let (x, z) = map.cell_centre(cx, cz);
            let top = (map.height[map.index(cx, cz)] as i64)
                .max(WATER_LEVEL + 6)
                .div_euclid(16);
            [top, top - 1].map(|y| (x as i64 >> 4, y, z as i64 >> 4))
        })
        .collect()
}

#[test]
fn test_chunks_from_a_fixed_map_and_seed_hash_the_same_on_every_machine() {
    let map = Arc::new(synthetic_map());
    let chunks = chunk_set(&map);
    let make = || Arc::new(RegionStore::new(Arc::new(WorldTerrain::new(map.clone()))));
    let first = hash_chunks(&make(), &chunks);
    assert_eq!(
        first,
        hash_chunks(&make(), &chunks),
        "not even repeatable on one machine"
    );
    // the set really does cover the kinds of ground it claims to
    let mut seen = std::collections::HashSet::new();
    let mut cells = [BlockType::Air; CELLS];
    for &(x, y, z) in &chunks {
        generate_chunk(make(), ChunkCoordinate(I64Vec3::new(x, y, z))).decode_into(&mut cells);
        seen.extend(cells.iter().map(|b| *b as u8));
    }
    for block in [
        BlockType::Grass,
        BlockType::Dirt,
        BlockType::Stone,
        BlockType::CompactedStone,
        BlockType::Sand,
        BlockType::Water,
        BlockType::Leaves,
        BlockType::Log,
    ] {
        assert!(seen.contains(&(block as u8)), "no {block:?} in the chunks");
    }
    // pinned: a change here means every machine's chunks changed, and with them any edits stored
    // as deltas against generated terrain, so bump `worldmap::GENERATOR_VERSION` with it
    assert_eq!(2_114_178_784_140_027_164, first, "chunk hash changed");
}
