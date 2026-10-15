use super::buffers::{ChunkMeshes, GrassTint, MeshStyle};
use super::mesh::mesh_blocks;
use super::source::ChunkNeighbourhood;
use super::terrain::{column_block, ground_top, under_grass};
use crate::block::BlockType;
use crate::chunks::chunk::neighbor_26_index;
use crate::chunks::chunk::ChunkCoordinate;
use crate::chunks::chunk::ChunkData;
use crate::chunks::chunk::CHUNK_SIZE;
use crate::chunks::generate::light::SkyLight;
use crate::chunks::generate::light::MAX_LIGHT;
use crate::chunks::generate::noise::NoiseGenerator;
use crate::chunks::generate::region_store::RegionStore;
use crate::chunks::generate::trees::grows_into;
use crate::chunks::generate::trees::trees_near;
use crate::chunks::generate::trees::TreeTints;
use crate::chunks::generate::trees::TREE_HEIGHT;
use crate::chunks::generate::WATER_LEVEL;
use bevy::math::I64Vec2;
use bevy::math::IVec3;
use std::cell::LazyCell;
use std::sync::Arc;

pub fn generate_chunk(regions: Arc<RegionStore>, chunk_pos: ChunkCoordinate) -> ChunkData {
    let mut heights = regions.sampler();
    let biomes = regions.noise().biomes();
    // the same for every chunk stacked over these columns, so worked out once for them all
    let columns = regions.chunk_columns(&mut heights, chunk_pos.0.x, chunk_pos.0.z);
    let (lowest, highest) = (columns.lowest, columns.highest);
    let size = CHUNK_SIZE as i64;
    let world_y = chunk_pos.0.y * size;
    // cells in `ChunkData::from_cells` order, which is the order they're worked out in here
    let mut cells = Vec::with_capacity((size * size * size) as usize);

    for x in 0..size {
        for z in 0..size {
            let column_height = columns.height(x, z);
            // above the ground and the sea, the column is air here whatever its climate
            if world_y >= ground_top(column_height).max(WATER_LEVEL) {
                cells.extend(std::iter::repeat_n(BlockType::Air, size as usize));
                continue;
            }
            let (gradient, biome, temperature) = (
                columns.gradient(x, z),
                columns.biome(x, z),
                columns.temperature(x, z),
            );
            cells.extend((world_y..world_y + size).map(|y| {
                under_grass(
                    column_block(biomes, biome, y, column_height, gradient, temperature),
                    y,
                    column_height,
                    1,
                )
            }));
        }
    }
    let mut chunk_data = ChunkData::from_cells(cells);

    let origin = chunk_pos.0 * size;
    if trees_may_reach(origin.y, origin.y + size, lowest, highest) {
        let min = I64Vec2::new(origin.x, origin.z);
        let trees = trees_near(regions.noise(), &mut heights, min, min + size - 1);
        for tree in &trees {
            tree.plant(
                origin,
                1,
                (IVec3::ZERO, IVec3::splat(size as i32 - 1)),
                |coord, block| {
                    let coord = coord.as_u16vec3();
                    if grows_into(chunk_data.get_block_at(coord), block) {
                        chunk_data.set_block_at(coord, block);
                    }
                },
            );
        }
        chunk_data.trees = TreeTints::new(&trees, origin, 1);
    }

    chunk_data
}

// Whether trees on ground between `lowest` and `highest` could reach into blocks `bottom..top`.
// Trees stand on gentle ground, so neighbouring ground just outside is close in height.
pub(super) fn trees_may_reach(bottom: i64, top: i64, lowest: i64, highest: i64) -> bool {
    const NEARBY_GROUND: i64 = 4;
    top > lowest - NEARBY_GROUND && bottom < highest + NEARBY_GROUND + TREE_HEIGHT
}

pub fn generate_chunk_mesh(
    chunk: Arc<ChunkData>,
    adjacent_chunks: Vec<Option<Arc<ChunkData>>>,
    coord: ChunkCoordinate,
    noise: &NoiseGenerator,
    solid_leaves: bool,
) -> ChunkMeshes {
    let source = ChunkNeighbourhood {
        chunk: &chunk,
        neighbors: &adjacent_chunks,
    };
    let tint = GrassTint::new(noise, (coord.0 * CHUNK_SIZE as i64).as_vec3(), 1.0);
    // worked out only once there's a face to light: buried and empty chunks have none
    let light = LazyCell::new(|| neighbourhood_light(&chunk, &adjacent_chunks, coord));
    // leaves drawn solid hide the leaves behind them, see `DebugSettings::solid_leaves`
    mesh_blocks(
        &source,
        1,
        MeshStyle {
            ambient_occlusion: true,
            textured: true,
            solid_leaves,
        },
        &tint,
        &light,
    )
}

// Light falling into the top of a box whose highest block is at `top`, in boxes of `cell`
// blocks: full sky, less any sea above it.
pub(super) fn sky_above(top: i64, cell: i64) -> u8 {
    let water_above = ((WATER_LEVEL - 1 - top) / cell).clamp(0, MAX_LIGHT as i64) as u8;
    MAX_LIGHT.saturating_sub(water_above.saturating_mul(BlockType::Water.light_opacity()))
}

// How far to each side of a chunk its light takes blocks into account: light that could
// only arrive from further out would already have faded to deep shade.
pub(super) const LIGHT_MARGIN: i32 = 8;

// Sky light for the chunk and a block around it, from the chunk and its 26 neighbours, as
// high as the top of the chunk above, with open sky above that.
pub(super) fn neighbourhood_light(
    chunk: &ChunkData,
    neighbours: &[Option<Arc<ChunkData>>],
    coord: ChunkCoordinate,
) -> SkyLight {
    let size = CHUNK_SIZE as i32;
    let min = IVec3::new(-LIGHT_MARGIN, -1, -LIGHT_MARGIN);
    let span = IVec3::new(size + LIGHT_MARGIN, 2 * size, size + LIGHT_MARGIN) - min;
    let mut opacity = vec![0; (span.x * span.y * span.z) as usize];
    for dx in -1..=1 {
        for dy in -1..=1 {
            for dz in -1..=1 {
                let data = match (dx, dy, dz) {
                    (0, 0, 0) => Some(chunk),
                    _ => neighbours[neighbor_26_index(dx, dy, dz)].as_deref(),
                };
                let Some(data) = data else {
                    continue;
                };
                for (local, block) in data.iter_non_air() {
                    let cell = IVec3::new(dx, dy, dz) * size + local.as_ivec3() - min;
                    if cell.cmpge(IVec3::ZERO).all() && cell.cmplt(span).all() {
                        opacity[SkyLight::index(span, cell)] = block.light_opacity();
                    }
                }
            }
        }
    }
    let top = coord.0.y * size as i64 + (min.y + span.y) as i64 - 1;
    SkyLight::new(min, span, sky_above(top, 1), opacity)
}
