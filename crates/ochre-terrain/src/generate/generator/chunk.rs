use super::buffers::{ChunkMeshes, GrassTint, InnerLeaves, MeshStyle};
use super::flora::flora_at;
use super::mesh::mesh_blocks;
use super::source::{BlockSource, ChunkNeighbourhood};
use super::terrain::{block_gradient, column_block, cut_cliff, ground_top, under_grass};
use crate::chunk::ChunkCoordinate;
use crate::chunk::ChunkData;
use crate::chunk::CHUNK_SIZE;
use crate::generate::light::SkyLight;
use crate::generate::light::MAX_LIGHT;
use crate::generate::region_store::{HeightSampler, RegionStore};
use crate::generate::trees::grows_into;
use crate::generate::trees::trees_near;
use crate::generate::trees::TreeTints;
use crate::generate::trees::TREE_HEIGHT;
use bevy::math::I64Vec2;
use bevy::math::IVec3;
use ochre_base::block::BlockType;
use ochre_base::relief::{cliff_density, cliff_reach};
use ochre_base::terrain_source::SpringPart;
use ochre_base::WATER_LEVEL;
use std::cell::LazyCell;
use std::sync::Arc;

// Blocks above the sea desert ground must be before it has cliffs: below it the steep ground is
// dunes.
const DESERT_CLIFFS_ABOVE: i64 = 90;

// Which of the passes laid over the terrain's blocks run, so a tool can tell which one made a block.
#[derive(Clone, Copy, Debug)]
pub struct ChunkPasses {
    pub cliffs: bool,
    pub trees: bool,
    pub flora: bool,
}

impl ChunkPasses {
    pub const ALL: Self = Self {
        cliffs: true,
        trees: true,
        flora: true,
    };
}

pub fn generate_chunk(regions: Arc<RegionStore>, chunk_pos: ChunkCoordinate) -> ChunkData {
    generate_chunk_with(regions, chunk_pos, ChunkPasses::ALL)
}

pub fn generate_chunk_with(
    regions: Arc<RegionStore>,
    chunk_pos: ChunkCoordinate,
    passes: ChunkPasses,
) -> ChunkData {
    let mut heights = regions.sampler();
    let biomes = regions.terrain().biomes();
    // the same for every chunk stacked over these columns, so worked out once for them all
    let columns = regions.chunk_columns(&mut heights, chunk_pos.0.x, chunk_pos.0.z);
    let (lowest, highest) = (columns.lowest, columns.highest);
    let size = CHUNK_SIZE as i64;
    let seed = regions.terrain().seed();
    let desert = biomes.id("desert");
    let world_y = chunk_pos.0.y * size;
    // cells in `ChunkData::from_cells` order, which is the order they're worked out in here
    let mut cells = Vec::with_capacity((size * size * size) as usize);

    for x in 0..size {
        for z in 0..size {
            let column_height = columns.height(x, z);
            let water_top = columns.water_top(x, z);
            let gradient = columns.gradient(x, z);
            // how far the surface here is cut or built up into cliffs, on steep ground
            // dunes' steep faces are not cliffs: desert has them only up in its mountains
            let dunes = Some(columns.biome(x, z)) == desert
                && column_height - WATER_LEVEL < DESERT_CLIFFS_ABOVE;
            // a river's or lake's bed isn't cut into cliffs either: rock would stand up through its water
            let fresh_water = water_top > WATER_LEVEL && water_top > column_height;
            let cliff = if !passes.cliffs || dunes || fresh_water {
                0.0
            } else {
                cliff_reach(gradient)
            };
            // a bank's cliffs are only cut, not built up: built up they leave a dry ledge between the water and the ground
            let water_by = if cliff > 0.0 {
                fresh_water_near(
                    &mut heights,
                    chunk_pos.0.x * size + x,
                    chunk_pos.0.z * size + z,
                )
            } else {
                None
            };
            // and ground at the waterline isn't cut at all, or a shore's edge is lowered into ragged water
            let cliff = if water_by.is_some_and(|top| column_height <= top + SHORE_HEIGHT) {
                0.0
            } else {
                cliff
            };
            let bank = water_by.is_some();
            // ground cut open beside fresh water floods to its level, not left as air under it
            let flood_top = water_by.map_or(water_top, |top| top.max(water_top));
            let bulge = cliff.ceil() as i64;
            // above the ground and the water, the column is air here whatever its climate
            if world_y >= (ground_top(column_height) + bulge).max(water_top) {
                cells.extend(std::iter::repeat_n(BlockType::Air, size as usize));
                continue;
            }
            let (biome, temperature) = (columns.biome(x, z), columns.temperature(x, z));
            // the gradient the blocks are chosen by: meadow on a mountain takes grass up its faces
            let slope = block_gradient(
                biomes,
                biome,
                seed,
                chunk_pos.0.x * size + x,
                chunk_pos.0.z * size + z,
                gradient,
            );
            // snow lies deep, in full blocks, with the packed stone only beneath it
            let snowy = column_block(
                biomes,
                biome,
                column_height - 1,
                column_height,
                slope,
                temperature,
                water_top,
            ) == BlockType::Snow;
            let (world_x, world_z) = (chunk_pos.0.x * size + x, chunk_pos.0.z * size + z);
            // a spring's rim and the bed of its pool are the biome's own block
            let spring =
                (columns.spring(x, z) != SpringPart::None).then(|| biomes.spring_block(biome));
            cells.extend((world_y..world_y + size).map(|y| {
                if let Some(block) = spring.filter(|_| y == column_height - 1) {
                    return block;
                }
                let block = under_grass(
                    column_block(
                        biomes,
                        biome,
                        y,
                        column_height,
                        gradient,
                        temperature,
                        water_top,
                    ),
                    y,
                    column_height,
                    1,
                    snowy,
                );
                // cliffs are cut from the ground before the water fills what they open
                if cliff == 0.0 || (y - column_height).abs() > bulge || bank && y >= column_height {
                    return block;
                }
                let density = (column_height - y) as f64 - 0.5
                    + cliff * cliff_density(seed, world_x, y, world_z);
                cut_cliff(block, density > 0.0, y, temperature, flood_top)
            }));
        }
    }
    let mut chunk_data = ChunkData::from_cells(cells);

    let origin = chunk_pos.0 * size;
    // any ground in the chunk is within the trees' reach, so flora sees every tree over it
    let mut trees = Vec::new();
    if passes.trees && trees_may_reach(origin.y, origin.y + size, lowest, highest) {
        let min = I64Vec2::new(origin.x, origin.z);
        trees = trees_near(regions.terrain(), &mut heights, min, min + size - 1);
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

    // plants on the grass, in the air just over it, after the trees, so trunks keep their place
    if !passes.flora {
        return chunk_data;
    }
    for x in 0..size {
        for z in 0..size {
            let column_height = columns.height(x, z);
            let local_y = column_height - world_y;
            if !(0..size).contains(&local_y) || cliff_reach(columns.gradient(x, z)) > 0.0 {
                continue;
            }
            let (gradient, biome, temperature) = (
                columns.gradient(x, z),
                columns.biome(x, z),
                columns.temperature(x, z),
            );
            let gradient = block_gradient(
                biomes,
                biome,
                seed,
                chunk_pos.0.x * size + x,
                chunk_pos.0.z * size + z,
                gradient,
            );
            let ground = column_block(
                biomes,
                biome,
                column_height - 1,
                column_height,
                gradient,
                temperature,
                columns.water_top(x, z),
            );
            let coord = bevy::math::U16Vec3::new(x as u16, local_y as u16, z as u16);
            if ground != BlockType::Grass || chunk_data.get_block_at(coord) != BlockType::Air {
                continue;
            }
            let (world_x, world_z) = (origin.x + x, origin.z + z);
            // ferns grow on the forest floor, and in the shade of a lone tree's crown
            let shaded = biomes.name(biome) == Some("forest")
                || trees
                    .iter()
                    .any(|tree| tree.shades(world_x as f64 + 0.5, world_z as f64 + 0.5));
            if let Some(plant) = flora_at(seed, world_x, world_z, shaded, temperature) {
                chunk_data.set_block_at(coord, plant);
            }
        }
    }

    chunk_data
}

// Blocks from fresh water within which ground's cliffs are cut but not built up.
const BANK_REACH: i64 = 4;

// Ground this many blocks or fewer over fresh water by it isn't cut into cliffs at all.
const SHORE_HEIGHT: i64 = 2;

// The highest river or lake water within `BANK_REACH` of the column, if any.
fn fresh_water_near(heights: &mut HeightSampler, x: i64, z: i64) -> Option<i64> {
    [
        (1, 0),
        (-1, 0),
        (0, 1),
        (0, -1),
        (1, 1),
        (-1, -1),
        (1, -1),
        (-1, 1),
    ]
    .into_iter()
    .map(|(dx, dz)| heights.water_top(x + dx * BANK_REACH, z + dz * BANK_REACH))
    .filter(|&top| top > WATER_LEVEL)
    .max()
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
    regions: &RegionStore,
    solid_leaves: bool,
) -> ChunkMeshes {
    let noise = regions.terrain();
    let source = ChunkNeighbourhood::new(&chunk, &adjacent_chunks)
        .with_water(regions, (coord.0 * CHUNK_SIZE as i64).as_ivec3());
    let tint = GrassTint::new(noise, (coord.0 * CHUNK_SIZE as i64).as_vec3(), 1.0);
    // worked out only once there's a face to light: buried and empty chunks have none
    let light = LazyCell::new(|| neighbourhood_light(&source, coord));
    // leaves drawn solid hide the leaves behind them, see `DebugSettings::solid_leaves`
    mesh_blocks(
        &source,
        1,
        MeshStyle {
            ambient_occlusion: true,
            textured: true,
            inner_leaves: if solid_leaves {
                InnerLeaves::Hidden
            } else {
                InnerLeaves::Apart
            },
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
fn neighbourhood_light(source: &ChunkNeighbourhood, coord: ChunkCoordinate) -> SkyLight {
    let size = CHUNK_SIZE as i32;
    let min = IVec3::new(-LIGHT_MARGIN, -1, -LIGHT_MARGIN);
    let span = IVec3::new(size + LIGHT_MARGIN, 2 * size, size + LIGHT_MARGIN) - min;
    let mut opacity = Vec::with_capacity((span.x * span.y * span.z) as usize);
    // in `SkyLight::index` order
    for x in 0..span.x {
        for z in 0..span.z {
            for y in 0..span.y {
                opacity.push(source.block(min + IVec3::new(x, y, z)).light_opacity());
            }
        }
    }
    let top = coord.0.y * size as i64 + (min.y + span.y) as i64 - 1;
    SkyLight::new(min, span, sky_above(top, 1), opacity)
}
