use crate::block::BlockType;
use crate::chunks::generate::biome::BiomeId;
use crate::chunks::generate::biome::Biomes;
use crate::chunks::generate::WATER_LEVEL;

pub(super) const SOLID_FLOOR: i64 = 16;

// The cell at the water's surface over ground below it is water, not ground: filled by its
// ground it would draw as a beach at sea level, where the water over it is mostly of the cell.
pub(super) fn underwater(
    block: BlockType,
    bottom: i64,
    cell: i64,
    (column_height, water_top): (i64, i64),
) -> BlockType {
    if is_solid(block)
        && bottom < water_top
        && water_top <= bottom + cell
        && ground_top(column_height) < water_top
    {
        BlockType::Water
    } else {
        block
    }
}

pub(super) fn is_solid(block: BlockType) -> bool {
    block.occlusion_weight() == 1.0
}

// Height above the sea where snow starts to settle in a temperate climate (temperature 0.5).
pub(super) const SNOW_LINE: i64 = 220;

// How many blocks the snow line climbs per unit of temperature.
pub(super) const SNOW_LINE_PER_DEGREE: f64 = 800.0;

// At or above this temperature it's too hot for snow, and the heights are bare rock.
pub(super) const SNOW_MAX_TEMPERATURE: f32 = 0.62;

// Above this height over the sea the ground is snow-capped in any climate, in the tropics too.
pub(super) const SNOWCAP_ABOVE: i64 = 360;

// Below this temperature even the grass is under a layer of snow.
pub(super) const SNOWY_GRASS_TEMPERATURE: f32 = 0.25;

// The lowest the snow line gets, so even the coldest lowlands are snow-covered.
pub(super) const COLDEST_SNOW_LINE: i64 = 10;

// Below this temperature the top of the sea freezes, thicker the colder it is.
pub const FREEZE_TEMPERATURE: f32 = 0.12;

pub(super) const ICE_MAX_DEPTH: i64 = 3;

pub(super) const ICE_TEMPERATURE_PER_LAYER: f32 = 0.04;

// How many blocks of the sea's surface are ice at this temperature.
pub(super) fn ice_depth(temperature: f32) -> i64 {
    if temperature >= FREEZE_TEMPERATURE {
        return 0;
    }
    (1 + ((FREEZE_TEMPERATURE - temperature) / ICE_TEMPERATURE_PER_LAYER) as i64).min(ICE_MAX_DEPTH)
}

// A wider LOD cell's water, as ice when it overlaps the frozen top of the sea; its middle is sampled, which is usually below it.
pub(super) fn freeze_cell(
    block: BlockType,
    temperature: f32,
    cell_top: i64,
    (cell, water_top): (i64, i64),
) -> BlockType {
    if cell > 1 && block == BlockType::Water && cell_top > water_top - ice_depth(temperature) {
        BlockType::Ice
    } else {
        block
    }
}

// Height above the sea where snow starts to settle, lower in the cold.
pub(super) fn snow_line(temperature: f32) -> i64 {
    (SNOW_LINE as f64 + (temperature as f64 - 0.5) * SNOW_LINE_PER_DEGREE)
        .max(COLDEST_SNOW_LINE as f64) as i64
}

// The steepest ground snow settles on, `above_sea` blocks up: only gentle ground at the snow
// line, steeper the higher it gets, until only cliffs are left bare.
pub(super) fn snow_holding_slope(above_sea: i64, snow_line: i64) -> f64 {
    let t = ((above_sea - snow_line) as f64 / 120.0).clamp(0.0, 1.0);
    1.5 + 3.0 * t
}

// The height, in blocks, the ground of a column `column_height` high is solid up to.
pub(super) fn ground_top(column_height: i64) -> i64 {
    column_height.max(SOLID_FLOOR)
}

pub(super) fn terrain_block(
    world_y: i64,
    column_height: i64,
    combined_gradient: f64,
    temperature: f32,
    water_top: i64,
) -> BlockType {
    if world_y < ground_top(column_height) {
        // under water the ground is sand, whatever it would be dry
        if ground_top(column_height) < water_top {
            return BlockType::Sand;
        }
        let above_sea = world_y - WATER_LEVEL;
        let above_water = world_y - water_top;
        // the tallest ground wears snow whatever the climate, the line lowering to meet the cap
        let capped = above_sea >= SNOWCAP_ABOVE;
        let line = if capped {
            snow_line(temperature).min(SNOWCAP_ABOVE)
        } else {
            snow_line(temperature)
        };
        let snows = temperature < SNOW_MAX_TEMPERATURE || capped;
        if snows && above_sea >= line && combined_gradient <= snow_holding_slope(above_sea, line) {
            BlockType::Snow
        } else if !snows && above_sea >= SNOW_LINE
            || above_sea >= 120 && combined_gradient >= 2.0
            || (above_water >= 2 && combined_gradient >= 3.5)
        {
            BlockType::Stone
        } else if above_water >= 2 {
            if temperature < SNOWY_GRASS_TEMPERATURE && world_y == column_height - 1 {
                BlockType::Snow
            } else {
                BlockType::Grass
            }
        } else {
            BlockType::Sand
        }
    } else if world_y < water_top {
        if world_y >= water_top - ice_depth(temperature) {
            BlockType::Ice
        } else {
            BlockType::Water
        }
    } else {
        BlockType::Air
    }
}

// Blocks of soil, sand or snow under the surface block before stone, and how far under the
// ground's surface the stone is packed hard, which only explosives break.
pub(super) const SOIL_DEPTH: i64 = 4;
pub(super) const COMPACTED_DEPTH: i64 = 6;
// Under snow the cover is thick, full blocks of it, and the packed stone begins only beneath.
pub(super) const SNOW_DEPTH: i64 = 14;
pub(super) const SNOW_COMPACTED_DEPTH: i64 = 16;

// The ground in layers: grass is only the top block, over dirt, glacier ice over snow, then the
// soil gives way to stone and the stone to compacted stone a fixed depth under this column's own
// surface. Wider LOD cells are sampled mid-cell, which would turn their tops into dirt, and they
// don't show the layers. `snowy` is whether the column's surface is snow, which lies deep.
pub(super) fn under_grass(
    block: BlockType,
    world_y: i64,
    column_height: i64,
    cell: i64,
    snowy: bool,
) -> BlockType {
    let depth = column_height - 1 - world_y;
    if cell != 1 || depth <= 0 {
        return block;
    }
    let block = match block {
        BlockType::Grass => BlockType::Dirt,
        BlockType::Ice => BlockType::Snow,
        other => other,
    };
    let soil = matches!(
        block,
        BlockType::Dirt | BlockType::Sand | BlockType::Gravel | BlockType::Snow
    );
    let (soil_depth, compacted_depth) = if snowy {
        (SNOW_DEPTH, SNOW_COMPACTED_DEPTH)
    } else {
        (SOIL_DEPTH, COMPACTED_DEPTH)
    };
    match block {
        _ if depth >= compacted_depth && is_solid(block) => BlockType::CompactedStone,
        _ if depth >= soil_depth && soil => BlockType::Stone,
        other => other,
    }
}

// A block after cliffs are cut, where the noise says the ground is `solid` here: rock builds up
// where it wasn't, and ground cut away is open air above the water but floods below it, ice
// and all, so caves in a sea cliff fill with the sea.
pub(super) fn cut_cliff(
    block: BlockType,
    solid: bool,
    y: i64,
    temperature: f32,
    water_top: i64,
) -> BlockType {
    match (solid, is_solid(block)) {
        (true, false) => BlockType::Stone,
        (false, true) => {
            if y < water_top {
                // well above the world's floor, so `terrain_block` sees only the water here
                terrain_block(y, 0, 0.0, temperature, water_top)
            } else {
                BlockType::Air
            }
        }
        _ => block,
    }
}

// The steepest ground a patch of meadow still takes grass on: past it even meadow is rock.
const MEADOW_GRADIENT: f64 = 0.9;

// The gradient to choose a column's blocks by: on a mountain, in a patch of meadow, as gentle as
// MEADOW_GRADIENT at most, so grass and the trees and flora on it cling to the face where the
// rest of the slope is bare rock.
pub(in crate::chunks::generate) fn block_gradient(
    biomes: &Biomes,
    biome: BiomeId,
    seed: u32,
    x: i64,
    z: i64,
    gradient: f64,
) -> f64 {
    if gradient > MEADOW_GRADIENT
        && biomes.name(biome) == Some("mountain")
        && crate::worldmap::detail::meadow_patch(seed, x as f64, z as f64)
    {
        MEADOW_GRADIENT
    } else {
        gradient
    }
}

// `terrain_block`, as the biome owning the column has it.
pub(in crate::chunks::generate) fn column_block(
    biomes: &Biomes,
    biome: BiomeId,
    world_y: i64,
    column_height: i64,
    combined_gradient: f64,
    temperature: f32,
    water_top: i64,
) -> BlockType {
    let base = terrain_block(
        world_y,
        column_height,
        combined_gradient,
        temperature,
        water_top,
    );
    biomes.block(biome, base, world_y - WATER_LEVEL, combined_gradient)
}
