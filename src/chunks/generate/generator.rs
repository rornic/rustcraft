use std::{cell::LazyCell, collections::HashMap, ops::Deref, sync::Arc};

use bevy::{
    asset::RenderAssetUsages,
    math::{I64Vec2, I64Vec3, IVec3, U16Vec3, Vec3},
    render::{
        mesh::{Indices, Mesh, VertexAttributeValues},
    },
};

use super::biome::{grass_tint, BiomeId, Biomes};
use super::light::{SkyLight, MAX_LIGHT};
use super::noise::NoiseGenerator;
use super::region_store::{RegionStore, SLOPE_SPAN};
use super::WATER_LEVEL;
use super::columns::{ColumnGrid, COARSE_CELL};
use super::trees::{grows_into, trees_near, TreeGround, THIN_TRUNK_CELL, TREE_HEIGHT};
use crate::block::{BlockType, BLOCK_COUNT};
use crate::chunks::chunk::{neighbor_26_index, ChunkCoordinate, ChunkData, CHUNK_SIZE};
use crate::chunks::lod::LodKey;
use crate::util::primitives::Vertex;

// Face order matches `face_vertices`/`sides` below: [front, right, left, back, top, bottom].
pub(super) const FACE_NORMAL_OFFSET: [IVec3; 6] = [
    IVec3::new(0, 0, -1), // front
    IVec3::new(1, 0, 0),  // right
    IVec3::new(-1, 0, 0), // left
    IVec3::new(0, 0, 1),  // back
    IVec3::new(0, 1, 0),  // top
    IVec3::new(0, -1, 0), // bottom
];
const FACE_TANGENTS: [(IVec3, IVec3); 6] = [
    (IVec3::new(1, 0, 0), IVec3::new(0, 1, 0)), // front
    (IVec3::new(0, 0, 1), IVec3::new(0, 1, 0)), // right
    (IVec3::new(0, 0, 1), IVec3::new(0, 1, 0)), // left
    (IVec3::new(1, 0, 0), IVec3::new(0, 1, 0)), // back
    (IVec3::new(1, 0, 0), IVec3::new(0, 0, 1)), // top
    (IVec3::new(1, 0, 0), IVec3::new(0, 0, 1)), // bottom
];
const AO_STRENGTH: f32 = 0.5;

const SOLID_FLOOR: i64 = 16;
// Height of the water surface within a full-detail block, whose cube spans -0.5..0.5.
const WATER_TOP: f32 = 0.4;

trait BlockSource {
    fn block(&self, coord: IVec3) -> BlockType;
    fn non_air(&self) -> Vec<(IVec3, BlockType)>;

    /// The terrain itself, without the skirts `block` adds around a LOD node's edges.
    fn terrain(&self, coord: IVec3) -> BlockType {
        self.block(coord)
    }

    /// Where the ground really tops out in a LOD surface cell, in mesh units, when that's
    /// below the top of the cell. See `LodBlocks::surface_top`.
    fn surface_top(&self, _coord: IVec3) -> Option<f32> {
        None
    }

    /// Whether any cell has a `surface_top`, so meshing can skip asking when none do.
    fn has_surface_tops(&self) -> bool {
        false
    }

    /// How much light a leaf at a mesh position gets under its canopy, 0..1.
    fn canopy_shade(&self, _position: Vec3) -> f32 {
        1.0
    }

    /// How bright the ground at a mesh position is under the forest over it, 0..1.
    fn forest_shade(&self, _position: Vec3) -> f32 {
        1.0
    }

    /// Trunks and crowns too small for the cells they stand in, drawn as boxes of their own.
    fn tree_boxes(&self) -> &[TreeBox] {
        &[]
    }

    /// Whether faces on its edge are skirt walls hanging from its own ground, over what's beyond.
    fn has_skirts(&self) -> bool {
        false
    }

    /// The water over the column at `coord`, in blocks, 0 for land.
    fn water_depth_blocks(&self, coord: IVec3) -> f32
    where
        Self: Sized,
    {
        water_depth(self, coord) as f32
    }
}

/// Part of a tree as a box of leaves or logs, in mesh units, for cells too wide to hold it.
#[derive(Debug, Copy, Clone)]
struct TreeBox {
    min: Vec3,
    max: Vec3,
    block: BlockType,
}

struct ChunkNeighbourhood<'a> {
    chunk: &'a ChunkData,
    neighbors: &'a [Option<Arc<ChunkData>>],
}

impl BlockSource for ChunkNeighbourhood<'_> {
    fn block(&self, coord: IVec3) -> BlockType {
        resolve_block(self.chunk, self.neighbors, coord)
    }

    fn non_air(&self) -> Vec<(IVec3, BlockType)> {
        self.chunk
            .iter_non_air()
            .map(|(c, block)| (c.as_ivec3(), block))
            .collect()
    }
}

const SKIRT_CELLS: i32 = 2;

struct LodBlocks {
    size: i32,
    cells: Vec<BlockType>,
    // the ground's height in blocks under each padded column, ground being below it
    tops: Vec<i64>,
    // the node's lowest block and cell size, to place `tops` among the cells
    base_y: i64,
    cell: i64,
    // `surface_top` for each padded cell, worked out once since meshing asks for every face
    surface_tops: Vec<Option<f32>>,
    any_surface_tops: bool,
    // the top of each padded column's canopy in blocks, `i64::MIN` where it has none, or empty with no canopies at all
    canopy_tops: Vec<i64>,
    // each column's ground height before rounding, empty when not known
    ground_heights: Vec<f32>,
    // how much of each padded column's ground forest covers, 0..1, or empty where trees are real
    forest: Vec<f32>,
    boxes: Vec<TreeBox>,
}

// Coarse cells up to this wide hold real trees. Wider ones, where a crown would be a cube many
// times its size, tint the ground by how much forest covers it instead, so forests never cut off.
const TREE_MAX_CELL: i64 = 32;
// Trunks are boxes only up to cells this wide: past them one is far under a pixel.
const TRUNK_MAX_CELL: i64 = 16;
// From cells this wide a crown is a box too, of its real size: as cells it would round up to a
// cube a cell wide, several times its size.
const BOX_CROWN_CELL: i64 = 4;
// Each tree a cell grows covers this much of its ground, see `Biome::tree_density`, and a cell
// that's all covered is drawn this fraction as bright.
const FOREST_COVER: f64 = 1.6;
const FOREST_DARKEN: f32 = 0.55;
// Leaves at the foot of a canopy are this much darker than at its top, over this many blocks.
const CANOPY_SHADE: f32 = 0.45;
const CANOPY_SHADE_SPAN: f32 = 20.0;

// The ground a coarse node's trees stand on, read from its grid of columns.
struct GridGround<'a> {
    grid: &'a ColumnGrid,
    noise: &'a NoiseGenerator,
    base: I64Vec2,
    cell: i64,
    size: i32,
}

impl GridGround<'_> {
    fn column(&self, x: i64, z: i64) -> (i32, i32) {
        let index = |world: i64, base: i64| ((world - base).div_euclid(self.cell) as i32).clamp(-1, self.size);
        (index(x, self.base.x), index(z, self.base.y))
    }
}

// Whether a tree grows, and where, is decided from the noise at its exact position, as the
// regions full detail reads are made from it, or levels would grow different trees. Only its
// base stands on the ground this node draws.
impl TreeGround for GridGround<'_> {
    fn trees(&mut self, x: i64, z: i64) -> f64 {
        // stored as a byte
        (self.noise.sample(x as f64, z as f64).trees * 255.0).round() / 255.0
    }

    fn height(&mut self, x: i64, z: i64) -> f64 {
        // stored as a float
        self.noise.sample(x as f64, z as f64).height as f32 as f64
    }

    fn gradient(&mut self, x: i64, z: i64) -> (f64, f64) {
        let span = SLOPE_SPAN;
        let scale = 1.0 / span as f64;
        let gradient_x = self.height(x + span, z) - self.height(x - span, z);
        let gradient_z = self.height(x, z + span) - self.height(x, z - span);
        (gradient_x * scale, gradient_z * scale)
    }

    fn biome(&mut self, x: i64, z: i64) -> BiomeId {
        self.noise.sample(x as f64, z as f64).biome
    }

    fn base_height(&mut self, x: i64, z: i64) -> i64 {
        let (cx, cz) = self.column(x, z);
        self.grid.height(cx, cz).round() as i64
    }
}

impl LodBlocks {
    // The padded column under a mesh position. A vertex on a cell's edge reads the column on its
    // far side, close enough for a gradient.
    fn column_at(&self, position: Vec3) -> usize {
        let column = |c: f32| ((c + 0.5).floor() as i32).clamp(-1, self.size);
        Self::column(self.size, column(position.x), column(position.z))
    }

    fn sample(regions: &RegionStore, key: LodKey) -> Self {
        if key.cell_size() >= COARSE_CELL {
            Self::sample_coarse(regions, key)
        } else {
            Self::sample_regions(regions, key)
        }
    }

    // From the noise at each cell's centre, with the real trees where the cells are narrow
    // enough, and forests as a tint on the ground where they're not.
    fn sample_coarse(regions: &RegionStore, key: LodKey) -> Self {
        let size = CHUNK_SIZE as i32;
        let padded = (size + 2) as usize;
        let cell = key.cell_size();
        let base = key.min_block();
        let mut cells = vec![BlockType::Air; padded * padded * padded];
        let mut tops = vec![0; padded * padded];
        let mut ground_heights = vec![0.0; padded * padded];
        let mut canopy_tops = vec![i64::MIN; padded * padded];
        let mut boxes = Vec::new();
        let grid = regions.column_grid(key);
        let noise = regions.noise();
        let real_trees = cell <= TREE_MAX_CELL;
        let mut forest = if real_trees { Vec::new() } else { vec![0.0; padded * padded] };
        let (mut lowest, mut highest) = (i64::MAX, i64::MIN);

        for cx in -1..=size {
            for cz in -1..=size {
                let exact_height = grid.height(cx, cz);
                let height = exact_height.round() as i64;
                ground_heights[Self::column(size, cx, cz)] = exact_height as f32;
                (lowest, highest) = (lowest.min(height), highest.max(height));
                let gradient = grid.gradient(cx, cz);
                let biome = grid.biome(cx, cz);
                let world_x = base.x + cx as i64 * cell + cell / 2;
                let world_z = base.z + cz as i64 * cell + cell / 2;
                let temperature = noise.temperature(world_x as f64, world_z as f64);
                let ground = column_block(noise.biomes(), biome, height - 1, height, gradient, temperature);
                tops[Self::column(size, cx, cz)] = ground_top(height);
                if !real_trees && ground == BlockType::Grass {
                    forest[Self::column(size, cx, cz)] = (grid.trees(cx, cz) * FOREST_COVER).min(1.0) as f32;
                }
                for cy in -1..=size {
                    let world_y = base.y + cy as i64 * cell + cell / 2;
                    let mut block = column_block(noise.biomes(), biome, world_y, height, gradient, temperature);
                    if block == BlockType::Air && base.y + cy as i64 * cell < ground_top(height) {
                        block = column_block(noise.biomes(), biome, ground_top(height) - 1, height, gradient, temperature);
                    }
                    cells[Self::index(size, IVec3::new(cx, cy, cz))] = block;
                }
            }
        }
        if real_trees {
            let span = (size + 1) as i64 * cell;
            if trees_may_reach(base.y - cell, base.y + span, lowest, highest) {
                let min = I64Vec2::new(base.x, base.z) - cell;
                let mut ground = GridGround { grid: &grid, noise, base: I64Vec2::new(base.x, base.z), cell, size };
                for tree in trees_near(noise, &mut ground, min, min + span + cell) {
                    // each tree's boxes are drawn by the one node its base is in
                    let at = (tree.base - base).div_euclid(I64Vec3::splat(cell));
                    if at.cmpge(I64Vec3::ZERO).all() && at.cmplt(I64Vec3::splat(size as i64)).all() {
                        let mesh_at = |block: Vec3| (block - base.as_vec3()) / cell as f32 - 0.5;
                        if (THIN_TRUNK_CELL..=TRUNK_MAX_CELL).contains(&cell) {
                            let width = tree.width();
                            let (min, max) = (tree.base.as_vec3(), (tree.base + I64Vec3::new(width, tree.trunk, width)).as_vec3());
                            boxes.push(TreeBox { min: mesh_at(min), max: mesh_at(max), block: BlockType::Log });
                        }
                        if cell >= BOX_CROWN_CELL {
                            for (min, max) in tree.crown_boxes() {
                                boxes.push(TreeBox { min: mesh_at(min.as_vec3()), max: mesh_at(max.as_vec3()), block: BlockType::Leaves });
                            }
                        }
                    }
                    if cell < BOX_CROWN_CELL {
                        tree.plant(base, cell, (IVec3::splat(-1), IVec3::splat(size)), |coord, block| {
                            let cell = &mut cells[Self::index(size, coord)];
                            if grows_into(*cell, block) {
                                *cell = block;
                            }
                        });
                    }
                }
            }
            // leaves darken down from the top of each column's highest, see `canopy_shade`
            for cx in -1..=size {
                for cz in -1..=size {
                    let top = (-1..=size).rev().find(|&cy| cells[Self::index(size, IVec3::new(cx, cy, cz))] == BlockType::Leaves);
                    if let Some(cy) = top {
                        canopy_tops[Self::column(size, cx, cz)] = base.y + (cy as i64 + 1) * cell;
                    }
                }
            }
        }
        Self::new(size, cells, tops, base.y, cell).with_ground_heights(ground_heights).with_canopy_tops(canopy_tops).with_forest(forest).with_boxes(boxes)
    }

    // From full-detail regions, with real trees.
    fn sample_regions(regions: &RegionStore, key: LodKey) -> Self {
        let size = CHUNK_SIZE as i32;
        let padded = (size + 2) as usize;
        let cell = key.cell_size();
        let base = key.min_block();
        let mut cells = vec![BlockType::Air; padded * padded * padded];
        let mut tops = vec![0; padded * padded];
        let mut ground_heights = vec![0.0; padded * padded];
        let mut heights = regions.sampler();
        let noise = regions.noise();
        let biomes = noise.biomes();
        let (mut lowest, mut highest) = (i64::MAX, i64::MIN);

        for cx in -1..=size {
            for cz in -1..=size {
                let world_x = base.x + cx as i64 * cell + cell / 2;
                let world_z = base.z + cz as i64 * cell + cell / 2;
                let exact_height = heights.height(world_x, world_z);
                let column_height = exact_height.round() as i64;
                ground_heights[Self::column(size, cx, cz)] = exact_height as f32;
                (lowest, highest) = (lowest.min(column_height), highest.max(column_height));
                let (gradient_x, gradient_z) = heights.gradient(world_x, world_z);
                let combined_gradient = gradient_x.abs() + gradient_z.abs();
                let biome = heights.biome(world_x, world_z);
                let temperature = noise.temperature(world_x as f64, world_z as f64);

                let top = ground_top(column_height);
                tops[Self::column(size, cx, cz)] = top;
                for cy in -1..=size {
                    let world_y = base.y + cy as i64 * cell + cell / 2;
                    let mut block = column_block(biomes, biome, world_y, column_height, combined_gradient, temperature);
                    if block == BlockType::Air && base.y + cy as i64 * cell < top {
                        block = column_block(biomes, biome, top - 1, column_height, combined_gradient, temperature);
                    }
                    cells[Self::index(size, IVec3::new(cx, cy, cz))] = block;
                }
            }
        }

        let span = (size + 1) as i64 * cell;
        if trees_may_reach(base.y - cell, base.y + span, lowest, highest) {
            let min = I64Vec2::new(base.x, base.z) - cell;
            for tree in trees_near(regions.noise(), &mut heights, min, min + span + cell) {
                tree.plant(base, cell, (IVec3::splat(-1), IVec3::splat(size)), |coord, block| {
                    let cell = &mut cells[Self::index(size, coord)];
                    if grows_into(*cell, block) {
                        *cell = block;
                    }
                });
            }
        }

        Self::new(size, cells, tops, base.y, cell).with_ground_heights(ground_heights)
    }

    fn with_ground_heights(mut self, ground_heights: Vec<f32>) -> Self {
        self.ground_heights = ground_heights;
        self
    }

    fn with_canopy_tops(mut self, canopy_tops: Vec<i64>) -> Self {
        self.canopy_tops = canopy_tops;
        self
    }

    fn with_forest(mut self, forest: Vec<f32>) -> Self {
        self.forest = forest;
        self
    }

    fn with_boxes(mut self, boxes: Vec<TreeBox>) -> Self {
        self.boxes = boxes;
        self
    }

    fn new(size: i32, cells: Vec<BlockType>, tops: Vec<i64>, base_y: i64, cell: i64) -> Self {
        let mut blocks = Self { size, cells, tops, base_y, cell, surface_tops: Vec::new(), any_surface_tops: false, canopy_tops: Vec::new(), ground_heights: Vec::new(), forest: Vec::new(), boxes: Vec::new() };
        blocks.surface_tops = vec![None; blocks.cells.len()];
        for x in -1..=size {
            for z in -1..=size {
                for y in -1..=size {
                    let coord = IVec3::new(x, y, z);
                    blocks.surface_tops[Self::index(size, coord)] = blocks.find_surface_top(coord);
                }
            }
        }
        blocks.any_surface_tops = blocks.surface_tops.iter().any(Option::is_some);
        blocks
    }

    // Whether the ground's top lands inside the cell at `coord`, which is solid.
    fn ground_tops_out_in(&self, coord: IVec3) -> bool {
        let top = self.tops[Self::column(self.size, coord.x, coord.z)];
        let bottom = self.base_y + coord.y as i64 * self.cell;
        bottom < top && top < bottom + self.cell && is_solid(self.raw(coord))
    }

    fn surface_top(&self, coord: IVec3) -> Option<f32> {
        let coord = coord.clamp(IVec3::splat(-1), IVec3::splat(self.size));
        self.surface_tops[Self::index(self.size, coord)]
    }

    fn column(size: i32, cx: i32, cz: i32) -> usize {
        let padded = size + 2;
        ((cx + 1) * padded + cz + 1) as usize
    }

    // The cell holding the ground's surface is solid however little of it the ground fills,
    // and its top drawn at the ground's real height instead, so slopes step a block at a
    // time however coarse the cells. Only under open air: under water or leaves, and beside
    // water, the whole cell is drawn, since they fill the cell above from its bottom.
    fn find_surface_top(&self, coord: IVec3) -> Option<f32> {
        let top = self.tops[Self::column(self.size, coord.x, coord.z)];
        let bottom = self.base_y + coord.y as i64 * self.cell;
        if !(bottom < top && top < bottom + self.cell) {
            return None;
        }
        let ground = matches!(
            self.raw(coord),
            BlockType::Stone | BlockType::Grass | BlockType::Sand | BlockType::Snow | BlockType::Leaves
        );
        let beside_water = [IVec3::X, IVec3::NEG_X, IVec3::Z, IVec3::NEG_Z]
            .iter()
            .any(|&side| self.raw(coord + side) == BlockType::Water);
        if !ground || beside_water || self.raw(coord + IVec3::Y) != BlockType::Air {
            return None;
        }
        Some((top - self.base_y) as f32 / self.cell as f32 - 0.5)
    }

    fn index(size: i32, coord: IVec3) -> usize {
        let padded = (size + 2) as usize;
        let c = (coord + IVec3::ONE).as_uvec3();
        (c.x as usize * padded + c.z as usize) * padded + c.y as usize
    }

    fn raw(&self, coord: IVec3) -> BlockType {
        let coord = coord.clamp(IVec3::splat(-1), IVec3::splat(self.size));
        self.cells[Self::index(self.size, coord)]
    }
}

impl BlockSource for LodBlocks {
    fn terrain(&self, coord: IVec3) -> BlockType {
        self.raw(coord)
    }

    fn surface_top(&self, coord: IVec3) -> Option<f32> {
        LodBlocks::surface_top(self, coord)
    }

    fn has_surface_tops(&self) -> bool {
        self.any_surface_tops
    }

    fn tree_boxes(&self) -> &[TreeBox] {
        &self.boxes
    }

    fn has_skirts(&self) -> bool {
        true
    }

    // the seabed's real depth, not the cells' count, so deep colour shades smoothly however coarse they are
    fn water_depth_blocks(&self, coord: IVec3) -> f32 {
        if water_depth(self, coord) == 0 {
            return 0.0;
        }
        let column = Self::column(self.size, coord.x.clamp(-1, self.size), coord.z.clamp(-1, self.size));
        let ground = self.ground_heights.get(column).copied().unwrap_or(self.tops[column] as f32);
        (WATER_LEVEL as f32 - ground).max(1.0)
    }

    fn forest_shade(&self, position: Vec3) -> f32 {
        if self.forest.is_empty() {
            return 1.0;
        }
        1.0 - (1.0 - FOREST_DARKEN) * self.forest[self.column_at(position)]
    }

    fn canopy_shade(&self, position: Vec3) -> f32 {
        if self.canopy_tops.is_empty() {
            return 1.0;
        }
        let top = self.canopy_tops[self.column_at(position)];
        if top == i64::MIN {
            return 1.0;
        }
        let y = self.base_y as f32 + (position.y + 0.5) * self.cell as f32;
        1.0 - CANOPY_SHADE * ((top as f32 - y) / CANOPY_SHADE_SPAN).clamp(0.0, 1.0)
    }

    fn block(&self, coord: IVec3) -> BlockType {
        let coord = coord.clamp(IVec3::splat(-1), IVec3::splat(self.size));
        let block = self.raw(coord);
        let outside = |c: i32| c < 0 || c >= self.size;
        if outside(coord.x) || outside(coord.z) {
            // The wall hangs from this node's own surface edge, so it's there whatever the cell
            // beyond holds: that is sampled at this node's resolution, and a coarser neighbour
            // that draws its ground lower would be left with a gap.
            let inner = IVec3::new(coord.x.clamp(0, self.size - 1), coord.y, coord.z.clamp(0, self.size - 1));
            for cell in [inner, coord] {
                if is_solid(self.raw(cell)) {
                    for dy in 1..=SKIRT_CELLS {
                        let above = self.raw(cell + IVec3::new(0, dy, 0));
                        if !is_solid(above) {
                            return above;
                        }
                    }
                }
            }
            // The ground may run on up into the node above, where there's no air to find, so
            // cells within the skirt's depth of the column's ground top count too. A buried
            // node's ground is far above it, so it grows no walls.
            let ground = self.tops[Self::column(self.size, inner.x, inner.z)];
            let bottom = self.base_y + inner.y as i64 * self.cell;
            if is_solid(self.raw(inner)) && bottom + self.cell > ground - SKIRT_CELLS as i64 * self.cell {
                return BlockType::Air;
            }
        }
        // full detail can have the ground a block below a surface cell in the node above, so lid it
        if coord.y == self.size && !outside(coord.x) && !outside(coord.z) && self.ground_tops_out_in(coord) {
            return BlockType::Air;
        }
        block
    }

    fn non_air(&self) -> Vec<(IVec3, BlockType)> {
        let mut result = Vec::new();
        for x in 0..self.size {
            for z in 0..self.size {
                for y in 0..self.size {
                    let coord = IVec3::new(x, y, z);
                    let block = self.raw(coord);
                    if block != BlockType::Air {
                        result.push((coord, block));
                    }
                }
            }
        }
        result
    }
}

fn is_solid(block: BlockType) -> bool {
    block.occlusion_weight() == 1.0
}

// Height above the sea where snow starts to settle in a temperate climate (temperature 0.5).
const SNOW_LINE: i64 = 220;
// How many blocks the snow line climbs per unit of temperature.
const SNOW_LINE_PER_DEGREE: f64 = 800.0;
// At or above this temperature it's too hot for snow, and the heights are bare rock.
const SNOW_MAX_TEMPERATURE: f32 = 0.62;
// Below this temperature even the grass is under a layer of snow.
const SNOWY_GRASS_TEMPERATURE: f32 = 0.15;

// The lowest the snow line gets, so even the coldest lowlands are grass under their snow.
const COLDEST_SNOW_LINE: i64 = 60;

// Height above the sea where snow starts to settle, lower in the cold.
fn snow_line(temperature: f32) -> i64 {
    (SNOW_LINE as f64 + (temperature as f64 - 0.5) * SNOW_LINE_PER_DEGREE).max(COLDEST_SNOW_LINE as f64) as i64
}

// The steepest ground snow settles on, `above_sea` blocks up: only gentle ground at the snow
// line, steeper the higher it gets, until only cliffs are left bare.
fn snow_holding_slope(above_sea: i64, snow_line: i64) -> f64 {
    let t = ((above_sea - snow_line) as f64 / 120.0).clamp(0.0, 1.0);
    1.5 + 3.0 * t
}

// The height, in blocks, the ground of a column `column_height` high is solid up to.
fn ground_top(column_height: i64) -> i64 {
    column_height.max(SOLID_FLOOR)
}

fn terrain_block(world_y: i64, column_height: i64, combined_gradient: f64, temperature: f32) -> BlockType {
    if world_y < ground_top(column_height) {
        let above_sea = world_y - WATER_LEVEL;
        let line = snow_line(temperature);
        let snows = temperature < SNOW_MAX_TEMPERATURE;
        if snows && above_sea >= line && combined_gradient <= snow_holding_slope(above_sea, line) {
            BlockType::Snow
        } else if !snows && above_sea >= SNOW_LINE
            || above_sea >= 120 && combined_gradient >= 2.0
            || (above_sea >= 2 && combined_gradient >= 3.5)
        {
            BlockType::Stone
        } else if above_sea >= 2 {
            if temperature < SNOWY_GRASS_TEMPERATURE && world_y == column_height - 1 {
                BlockType::Snow
            } else {
                BlockType::Grass
            }
        } else {
            BlockType::Sand
        }
    } else if world_y < WATER_LEVEL {
        BlockType::Water
    } else {
        BlockType::Air
    }
}

// `terrain_block`, as the biome owning the column has it.
pub(super) fn column_block(
    biomes: &Biomes,
    biome: BiomeId,
    world_y: i64,
    column_height: i64,
    combined_gradient: f64,
    temperature: f32,
) -> BlockType {
    let base = terrain_block(world_y, column_height, combined_gradient, temperature);
    biomes.block(biome, base, world_y - WATER_LEVEL, combined_gradient)
}

// `neighbor_chunks` holds all 26 chunks in the 3x3x3 neighborhood (see `ChunkCoordinate::neighbors_26`),
// since AO sampling near a chunk corner can need a diagonal-adjacent chunk, not just a face-adjacent one.
fn resolve_block(chunk: &ChunkData, neighbor_chunks: &[Option<Arc<ChunkData>>], coord: IVec3) -> BlockType {
    let size = chunk.size as i32;
    let out = |c: i32| if c < 0 { -1 } else if c >= size { 1 } else { 0 };
    let wrap = |c: i32, out: i32| match out {
        -1 => size - 1,
        1 => 0,
        _ => c,
    };
    let (out_x, out_y, out_z) = (out(coord.x), out(coord.y), out(coord.z));

    if out_x == 0 && out_y == 0 && out_z == 0 {
        return chunk.get_block_at(U16Vec3::new(coord.x as u16, coord.y as u16, coord.z as u16));
    }

    let wrapped = IVec3::new(
        wrap(coord.x, out_x),
        wrap(coord.y, out_y),
        wrap(coord.z, out_z),
    );
    let idx = neighbor_26_index(out_x, out_y, out_z);
    neighbor_chunks[idx]
        .as_ref()
        .map(|c| c.get_block_at(U16Vec3::new(wrapped.x as u16, wrapped.y as u16, wrapped.z as u16)))
        .unwrap_or_default()
}

// The two tangent-direction offsets (toward whichever corner `vertex_local_pos` is
// on) for a face - shared by `vertex_ao` (which samples one layer along the face
// normal, for overhang darkening) and `vertex_shore_factor` (which samples at the
// block's own height, since shoreline land sits flush with the water surface, not
// above it).
fn tangent_offsets(face_index: usize, vertex_local_pos: [f32; 3]) -> (IVec3, IVec3) {
    let (ta, tb) = FACE_TANGENTS[face_index];
    let axis_index = |v: IVec3| if v.x != 0 { 0 } else if v.y != 0 { 1 } else { 2 };
    let sign_a = vertex_local_pos[axis_index(ta)].signum() as i32;
    let sign_b = vertex_local_pos[axis_index(tb)].signum() as i32;
    (ta * sign_a, tb * sign_b)
}

fn vertex_ao(
    source: &impl BlockSource,
    block_coord: IVec3,
    face_index: usize,
    vertex_local_pos: [f32; 3],
) -> f32 {
    let (offset_a, offset_b) = tangent_offsets(face_index, vertex_local_pos);
    let normal_off = FACE_NORMAL_OFFSET[face_index];

    let side_a = source.block(block_coord + normal_off + offset_a);
    let side_b = source.block(block_coord + normal_off + offset_b);
    let corner = source.block(block_coord + normal_off + offset_a + offset_b);

    let sum = side_a.occlusion_weight() + side_b.occlusion_weight() + corner.occlusion_weight();
    1.0 - (sum / 3.0).clamp(0.0, 1.0) * AO_STRENGTH
}

// Sky light at a face's corner, 0..1: the average over the cells around the corner on the
// face's open side, leaving out solid ones, and the diagonal when both sides are solid
// since light can't reach round to it.
fn vertex_light(
    source: &impl BlockSource,
    light: &SkyLight,
    block_coord: IVec3,
    face_index: usize,
    vertex_local_pos: [f32; 3],
) -> f32 {
    let (offset_a, offset_b) = tangent_offsets(face_index, vertex_local_pos);
    let front = block_coord + FACE_NORMAL_OFFSET[face_index];
    // the terrain the light was worked out from, not the skirts LODs add at their edges
    let open = |coord: IVec3| !source.terrain(coord).is_opaque();
    let (side_a, side_b) = (open(front + offset_a), open(front + offset_b));
    let corner = (side_a || side_b) && open(front + offset_a + offset_b);
    let cells = [
        Some(front),
        side_a.then_some(front + offset_a),
        side_b.then_some(front + offset_b),
        corner.then_some(front + offset_a + offset_b),
    ];
    let levels: Vec<f32> = cells.into_iter().flatten().map(|c| light.level(c) as f32).collect();
    levels.iter().sum::<f32>() / levels.len() as f32 / MAX_LIGHT as f32
}

// One light level for a whole face, 0..1, in a few coarse steps so neighbouring faces
// match and merge.
fn face_light(level: u8) -> f32 {
    const STEP: u8 = 3;
    let stepped = (level + STEP / 2) / STEP * STEP;
    stepped.min(MAX_LIGHT) as f32 / MAX_LIGHT as f32
}

// Fraction of a water top-face vertex's same-height tangent corners that are solid
// land - used to bake a shoreline mask into the water mesh so the shader can lap
// foam there.
fn vertex_shore_factor(
    source: &impl BlockSource,
    block_coord: IVec3,
    face_index: usize,
    vertex_local_pos: [f32; 3],
) -> f32 {
    let (offset_a, offset_b) = tangent_offsets(face_index, vertex_local_pos);

    let side_a = source.block(block_coord + offset_a);
    let side_b = source.block(block_coord + offset_b);
    let corner = source.block(block_coord + offset_a + offset_b);

    let land_count = [side_a, side_b, corner]
        .iter()
        .filter(|b| b.occlusion_weight() == 1.0)
        .count();
    land_count as f32 / 3.0
}

const SHORE_DISTANCE_RADIUS: i32 = 3;

// Distance (in blocks, normalized to 0..1 over `SHORE_DISTANCE_RADIUS`, 1.0 meaning
// "no land within range") from this top-face vertex to the nearest land block -
// anchored at the *shared grid corner* (block_coord + tangent offsets) rather than
// the block itself, so up to four blocks touching that corner all compute the exact
// same distance there. That's what keeps it safe to drive the foam's travelling
// phase: unlike per-block data, it can't disagree at a shared edge.
fn vertex_shore_distance(
    source: &impl BlockSource,
    block_coord: IVec3,
    face_index: usize,
    vertex_local_pos: [f32; 3],
) -> f32 {
    let (offset_a, offset_b) = tangent_offsets(face_index, vertex_local_pos);
    let anchor = block_coord + offset_a + offset_b;

    let mut min_dist_sq = i32::MAX;
    for dz in -SHORE_DISTANCE_RADIUS..=SHORE_DISTANCE_RADIUS {
        for dx in -SHORE_DISTANCE_RADIUS..=SHORE_DISTANCE_RADIUS {
            let dist_sq = dx * dx + dz * dz;
            if dist_sq > SHORE_DISTANCE_RADIUS * SHORE_DISTANCE_RADIUS || dist_sq >= min_dist_sq {
                continue;
            }
            let probe = anchor + IVec3::new(dx, 0, dz);
            if source.block(probe).occlusion_weight() == 1.0 {
                min_dist_sq = dist_sq;
            }
        }
    }

    if min_dist_sq == i32::MAX {
        1.0
    } else {
        (min_dist_sq as f32).sqrt() / SHORE_DISTANCE_RADIUS as f32
    }
}

pub fn generate_chunk(
    regions: Arc<RegionStore>,
    chunk_pos: ChunkCoordinate,
) -> ChunkData {
    let mut chunk_data = ChunkData::default();
    let mut heights = regions.sampler();
    let noise = regions.noise();
        let biomes = noise.biomes();
    let (mut lowest, mut highest) = (i64::MAX, i64::MIN);

    for x in 0..chunk_data.size {
        for z in 0..chunk_data.size {
            let (world_x, world_y, world_z) = (
                chunk_pos.0.x * chunk_data.size as i64 + x as i64,
                chunk_pos.0.y * chunk_data.size as i64,
                chunk_pos.0.z * chunk_data.size as i64 + z as i64,
            );

            let column_height = heights.height(world_x, world_z).round() as i64;
            (lowest, highest) = (lowest.min(column_height), highest.max(column_height));
            let (gradient_x, gradient_z) = heights.gradient(world_x, world_z);
            let combined_gradient = gradient_x.abs() + gradient_z.abs();
            let biome = heights.biome(world_x, world_z);
            let temperature = noise.temperature(world_x as f64, world_z as f64);

            for y in 0..chunk_data.size {
                let block = column_block(biomes, biome, world_y + y as i64, column_height, combined_gradient, temperature);
                if block != BlockType::Air {
                    chunk_data.set_block_at(U16Vec3::new(x, y, z), block);
                }
            }
        }
    }

    let size = chunk_data.size as i64;
    let origin = chunk_pos.0 * size;
    if trees_may_reach(origin.y, origin.y + size, lowest, highest) {
        let min = I64Vec2::new(origin.x, origin.z);
        for tree in trees_near(regions.noise(), &mut heights, min, min + size - 1) {
            tree.plant(origin, 1, (IVec3::ZERO, IVec3::splat(size as i32 - 1)), |coord, block| {
                let coord = coord.as_u16vec3();
                if grows_into(chunk_data.get_block_at(coord), block) {
                    chunk_data.set_block_at(coord, block);
                }
            });
        }
    }

    chunk_data
}

// Whether trees on ground between `lowest` and `highest` could reach into blocks `bottom..top`.
// Trees stand on gentle ground, so neighbouring ground just outside is close in height.
fn trees_may_reach(bottom: i64, top: i64, lowest: i64, highest: i64) -> bool {
    const NEARBY_GROUND: i64 = 4;
    top > lowest - NEARBY_GROUND && bottom < highest + NEARBY_GROUND + TREE_HEIGHT
}

// Accumulates one mesh's worth of vertex/index/color data - one instance for the
// opaque blocks, a second for water, so they can become separate `Mesh`es with
// separate materials (water needs alpha blending, opaque blocks don't).
#[derive(Default)]
struct MeshBuffers {
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
    colors: Vec<[f32; 4]>,
    // sky light per vertex, for block faces only, see `world.wesl`
    lights: Vec<f32>,
    // whether faces draw their block's mean colour instead of its texture, see `world.wesl`
    untextured: bool,
    // where tree boxes start and whether they're untextured, which they are on their own terms
    boxes: Option<(usize, bool)>,
}

impl MeshBuffers {
    fn push_face(&mut self, vs: &[Vertex], position: Vec3, uv_offset: f32, colors: &[[f32; 4]]) {
        let uv_scale = 1.0 / (BLOCK_COUNT - 1) as f32;

        let triangle_start: u32 = self.vertices.len() as u32;
        for (v, color) in vs.iter().zip(colors.iter()) {
            self.vertices.push(Vertex {
                position: (Vec3::from(v.position) + position).into(),
                normal: v.normal,
                uv: [uv_scale * (v.uv[0] + uv_offset), v.uv[1]],
            });
            self.colors.push(*color);
        }
        self.indices.extend(vec![
            triangle_start,
            triangle_start + 1,
            triangle_start + 2,
            triangle_start + 2,
            triangle_start + 1,
            triangle_start + 3,
        ]);
    }

    // `uvs` count tiles across the quad, and the colour alpha carries the atlas cell, see `world.wesl`.
    fn push_tiled_quad(
        &mut self,
        positions: [[f32; 3]; 4],
        normal: [f32; 3],
        uvs: [[f32; 2]; 4],
        colors: [[f32; 4]; 4],
        lights: [f32; 4],
    ) {
        let triangle_start: u32 = self.vertices.len() as u32;
        for j in 0..4 {
            self.vertices.push(Vertex {
                position: positions[j],
                normal,
                uv: uvs[j],
            });
            self.colors.push(colors[j]);
            self.lights.push(lights[j]);
        }
        self.indices.extend([
            triangle_start,
            triangle_start + 1,
            triangle_start + 2,
            triangle_start + 2,
            triangle_start + 1,
            triangle_start + 3,
        ]);
    }

    // Vertices at every block step round the border, so neighbours share exact edge points and leave no cracks.
    fn push_fan_quad(&mut self, quad: FanQuad) {
        let (w, h) = quad.size;
        let mut ring = Vec::with_capacity(2 * (w + h));
        ring.extend((0..w).map(|i| (i, 0)));
        ring.extend((0..h).map(|j| (w, j)));
        ring.extend((1..=w).rev().map(|i| (i, h)));
        ring.extend((1..=h).rev().map(|j| (0, j)));

        let mut push = |i: f32, j: f32, uv: [f32; 2]| {
            let position = quad.corner + quad.ta * i + quad.tb * j;
            self.vertices.push(Vertex {
                position: position.into(),
                normal: quad.normal,
                uv,
            });
            self.colors.push((quad.color)(position));
            self.lights.push(quad.light);
            self.vertices.len() as u32 - 1
        };
        let (uv_min, uv_max) = ((quad.uv_at)(0, 0), (quad.uv_at)(w, h));
        let centre = push(w as f32 / 2.0, h as f32 / 2.0, [0, 1].map(|k| (uv_min[k] + uv_max[k]) / 2.0));
        let first = centre + 1;
        for &(i, j) in &ring {
            push(i as f32, j as f32, (quad.uv_at)(i, j));
        }
        for k in 0..ring.len() as u32 {
            let (from, to) = (first + k, first + (k + 1) % ring.len() as u32);
            if quad.reversed {
                self.indices.extend([centre, to, from]);
            } else {
                self.indices.extend([centre, from, to]);
            }
        }
    }

    fn build_mesh(self) -> Mesh {
        let mut mesh = Mesh::new(
            bevy::render::mesh::PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
        );
        mesh.insert_indices(Indices::U32(self.indices));
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_POSITION,
            VertexAttributeValues::Float32x3(self.vertices.iter().map(|v| v.position).collect()),
        );
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_NORMAL,
            VertexAttributeValues::Float32x3(self.vertices.iter().map(|v| v.normal).collect()),
        );
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_UV_0,
            VertexAttributeValues::Float32x2(self.vertices.iter().map(|v| v.uv).collect()),
        );
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_COLOR,
            VertexAttributeValues::Float32x4(self.colors),
        );
        if !self.lights.is_empty() {
            assert_eq!(self.lights.len(), self.vertices.len(), "only some vertices carry light");
            let flag = |untextured: bool| if untextured { 1.0 } else { 0.0 };
            let lights = self
                .lights
                .into_iter()
                .enumerate()
                .map(|(i, l)| match self.boxes {
                    Some((start, untextured)) if i >= start => [l, flag(untextured)],
                    _ => [l, flag(self.untextured)],
                })
                .collect();
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, VertexAttributeValues::Float32x2(lights));
        }
        mesh
    }
}

struct FanQuad<'a> {
    corner: Vec3,
    ta: Vec3,
    tb: Vec3,
    size: (usize, usize),
    normal: [f32; 3],
    color: &'a dyn Fn(Vec3) -> [f32; 4],
    light: f32,
    uv_at: &'a dyn Fn(usize, usize) -> [f32; 2],
    reversed: bool,
}

pub struct ChunkMeshes {
    pub opaque: Mesh,
    pub water: Option<Mesh>,
}

impl ChunkMeshes {
    /// Whether there's anything to draw: buried chunks mesh to nothing.
    pub fn has_geometry(&self) -> bool {
        self.opaque.count_vertices() > 0 || self.water.is_some()
    }
}

// Counts contiguous Water blocks straight down from `block_coord`, within this
// chunk only (so it's a cheap, purely-local lookup the async mesh task can do
// without `World` access - see the plan's note on why depth doesn't cross chunk
// boundaries). Used to scale wave amplitude: open water ripples, a shallow puddle
// at a chunk's edge stays calm.
fn water_depth(source: &impl BlockSource, block_coord: IVec3) -> u32 {
    let mut depth = 0u32;
    let mut y = block_coord.y;
    while y >= 0 {
        if source.terrain(IVec3::new(block_coord.x, y, block_coord.z)) != BlockType::Water {
            break;
        }
        depth += 1;
        y -= 1;
    }
    depth
}

// Water depth at a top-face corner, averaged over the four columns sharing it (land counts as 0).
fn vertex_water_depth(
    depth_of: &mut impl FnMut(IVec3) -> f32,
    block_coord: IVec3,
    face_index: usize,
    vertex_local_pos: [f32; 3],
) -> f32 {
    let (offset_a, offset_b) = tangent_offsets(face_index, vertex_local_pos);
    let total: f32 = [IVec3::ZERO, offset_a, offset_b, offset_a + offset_b]
        .into_iter()
        .map(|offset| depth_of(block_coord + offset))
        .sum();
    total / 4.0
}

// Tints grass by the climate it grows in. Climate drifts over thousands of blocks, so it's
// sampled at the mesh's four corners and blended across it, rather than at every vertex.
struct GrassTint {
    // at the mesh's edges, so neighbouring meshes share their tints along each edge: (-x, -z), (+x, -z), (-x, +z), (+x, +z)
    corners: [Vec3; 4],
}

impl GrassTint {
    // `origin` is where the mesh's local origin is in the world, and `scale` how many blocks one local unit is.
    fn new(noise: &NoiseGenerator, origin: Vec3, scale: f32) -> Self {
        let size = CHUNK_SIZE as f32;
        let corners = [(0.0, 0.0), (size, 0.0), (0.0, size), (size, size)].map(|(x, z)| {
            let world = origin + Vec3::new(x - 0.5, 0.0, z - 0.5) * scale;
            grass_tint(noise.climate(world.x as f64, world.z as f64))
        });
        Self { corners }
    }

    fn at(&self, local: Vec3) -> Vec3 {
        let size = CHUNK_SIZE as f32;
        let (u, v) = ((local.x + 0.5) / size, (local.z + 0.5) / size);
        let [a, b, c, d] = self.corners;
        a.lerp(b, u).lerp(c.lerp(d, u), v)
    }
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
    mesh_blocks(&source, 1, true, true, solid_leaves, &tint, &light)
}

// Light falling into the top of a box whose highest block is at `top`, in boxes of `cell`
// blocks: full sky, less any sea above it.
fn sky_above(top: i64, cell: i64) -> u8 {
    let water_above = ((WATER_LEVEL - 1 - top) / cell).clamp(0, MAX_LIGHT as i64) as u8;
    MAX_LIGHT.saturating_sub(water_above.saturating_mul(BlockType::Water.light_opacity()))
}

// How far to each side of a chunk its light takes blocks into account: light that could
// only arrive from further out would already have faded to deep shade.
const LIGHT_MARGIN: i32 = 8;

// Sky light for the chunk and a block around it, from the chunk and its 26 neighbours, as
// high as the top of the chunk above, with open sky above that.
fn neighbourhood_light(chunk: &ChunkData, neighbours: &[Option<Arc<ChunkData>>], coord: ChunkCoordinate) -> SkyLight {
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

// Cells wider than this draw their block's mean colour instead of its texture, which tiles a
// cell's width of times over a face, since one stretched over a coarse cell would read as a
// giant block. Past a few blocks a tile is under a pixel and mipmaps down to the mean anyway.
const TEXTURE_MAX_CELL_SIZE: i64 = 4;
// Tree boxes keep their leaf texture, holes and all, up to cells this wide: it has to stay
// readable as leaves, and past it a crown is a few pixels.
const BOX_TEXTURE_MAX_CELL: i64 = 8;

// A quad's `uvs` scaled to count blocks across it, so a texture isn't stretched over a whole box.
fn tiled_uvs(positions: &[[f32; 3]; 4], uvs: &[[f32; 2]; 4], cell: f32) -> [[f32; 2]; 4] {
    let length = |axis: usize| {
        let other = |j: usize, k: usize| uvs[j][1 - axis] == uvs[k][1 - axis] && uvs[j][axis] != uvs[k][axis];
        let (j, k) = (0..4).flat_map(|j| (0..4).map(move |k| (j, k))).find(|&(j, k)| other(j, k)).unwrap_or((0, 0));
        (Vec3::from(positions[j]) - Vec3::from(positions[k])).length() * cell
    };
    let blocks = [length(0), length(1)];
    std::array::from_fn(|j| [uvs[j][0] * blocks[0], uvs[j][1] * blocks[1]])
}

pub fn generate_lod_mesh(regions: Arc<RegionStore>, key: LodKey) -> Option<ChunkMeshes> {
    let cell_size = key.cell_size();
    // matches `lod_transform`
    let origin = key.min_block().as_vec3() + Vec3::splat(0.5 * cell_size as f32 - 0.5);
    let tint = GrassTint::new(regions.noise(), origin, cell_size as f32);
    let blocks = LodBlocks::sample(&regions, key);
    let top = key.min_block().y + (blocks.size as i64 + 1) * cell_size;
    // the padded cells are already in `SkyLight::index` order
    let light = LazyCell::new(|| {
        let opacity = blocks.cells.iter().map(BlockType::light_opacity).collect();
        SkyLight::new(IVec3::splat(-1), IVec3::splat(blocks.size + 2), sky_above(top, cell_size), opacity)
    });
    let textured = cell_size <= TEXTURE_MAX_CELL_SIZE;
    // untextured leaves are drawn solid, so hide the leaves behind them
    let meshes = mesh_blocks(&blocks, cell_size, true, textured, !textured, &tint, &light);
    if meshes.opaque.count_vertices() == 0 && meshes.water.is_none() {
        return None;
    }
    Some(meshes)
}

fn grid_index(size: i32, coord: IVec3) -> usize {
    ((coord.x * size + coord.y) * size + coord.z) as usize
}

fn axis_of(v: IVec3) -> usize {
    if v.x != 0 {
        0
    } else if v.y != 0 {
        1
    } else {
        2
    }
}

// Which tangent (0 or 1) UV component `k` runs along on this face, so a merged quad can repeat it.
fn uv_tangent(face: &[Vertex], k: usize, a_axis: usize) -> usize {
    let high = |v: &Vertex| v.position[a_axis] > 0.0;
    let follows_a = face.iter().all(|v| (high(v) == high(&face[0])) == (v.uv[k] == face[0].uv[k]));
    if follows_a {
        0
    } else {
        1
    }
}

// Reads blocks inside the chunk from a dense grid, which is much cheaper than `source`, and the rest from `source`.
struct GridView<'a, S: BlockSource> {
    size: i32,
    blocks: &'a [BlockType],
    source: &'a S,
}

impl<S: BlockSource> GridView<'_, S> {
    fn contains(&self, coord: IVec3) -> bool {
        coord.min_element() >= 0 && coord.max_element() < self.size
    }
}

impl<S: BlockSource> BlockSource for GridView<'_, S> {
    fn block(&self, coord: IVec3) -> BlockType {
        if self.contains(coord) {
            self.blocks[grid_index(self.size, coord)]
        } else {
            self.source.block(coord)
        }
    }

    fn terrain(&self, coord: IVec3) -> BlockType {
        if self.contains(coord) {
            self.blocks[grid_index(self.size, coord)]
        } else {
            self.source.terrain(coord)
        }
    }

    fn non_air(&self) -> Vec<(IVec3, BlockType)> {
        self.source.non_air()
    }

    fn surface_top(&self, coord: IVec3) -> Option<f32> {
        self.source.surface_top(coord)
    }

    fn has_surface_tops(&self) -> bool {
        self.source.has_surface_tops()
    }

    fn canopy_shade(&self, position: Vec3) -> f32 {
        self.source.canopy_shade(position)
    }

    fn forest_shade(&self, position: Vec3) -> f32 {
        self.source.forest_shade(position)
    }

    fn tree_boxes(&self) -> &[TreeBox] {
        self.source.tree_boxes()
    }

    fn has_skirts(&self) -> bool {
        self.source.has_skirts()
    }
}

#[derive(Copy, Clone)]
struct FaceCell {
    block: BlockType,
    ao: [f32; 4],
    // sky light at each corner, 0..1
    light: [f32; 4],
    // where the face starts and ends up the mesh, if not the whole cell, see `face_span`
    span: Option<[f32; 2]>,
}

impl FaceCell {
    fn flat(&self) -> bool {
        self.ao.iter().all(|&ao| ao == self.ao[0]) && self.light.iter().all(|&l| l == self.light[0])
    }

    // Only flat-shaded faces merge: stretching a gradient over several blocks would change the shading.
    fn merges_with(&self, other: &FaceCell) -> bool {
        self.block == other.block
            && self.flat()
            && other.flat()
            && self.ao[0] == other.ao[0]
            && self.light[0] == other.light[0]
            && self.span == other.span
    }
}

// Whether a face of `block` at `coord` shows, and if so how much of it up the mesh's y axis,
// with LOD surface cells topped out at the ground's real height (see `LodBlocks::surface_top`):
// `Some(None)` for the whole face. Faces show through leaves' holes, other leaves' too, unless
// `leaves_cover_leaves`.
fn face_span(
    source: &impl BlockSource,
    coord: IVec3,
    normal: IVec3,
    block: BlockType,
    leaves_cover_leaves: bool,
) -> Option<Option<[f32; 2]>> {
    if matches!(block, BlockType::Air | BlockType::Water) {
        return None;
    }
    let neighbour = coord + normal;
    let neighbour_block = source.block(neighbour);
    let covered = neighbour_block.is_opaque()
        || (leaves_cover_leaves && block == BlockType::Leaves && neighbour_block == BlockType::Leaves);
    // every face is asked about, and most are hidden, so those leave before any lookups
    if !source.has_surface_tops() || normal.y < 0 {
        return (!covered).then_some(None);
    }
    if normal.y > 0 {
        // a lowered top still faces the air
        return (!covered).then(|| source.surface_top(coord).map(|t| [t, t]));
    }
    let bottom = coord.y as f32 - 0.5;
    // a lowered neighbour leaves this side bare above the ground's real height
    let floor = if covered { source.surface_top(neighbour)? } else { bottom };
    let top = source.surface_top(coord);
    let ceiling = top.unwrap_or(bottom + 1.0);
    if ceiling <= floor {
        return None;
    }
    Some((top.is_some() || covered).then_some([floor, ceiling]))
}

// Merges runs of identical, flat-shaded opaque faces into single quads, one face direction slice at a time.
// With `solid_leaves`, leaves hide the faces of the leaves behind them.
fn mesh_opaque_greedy(
    source: &GridView<impl BlockSource>,
    face_vertices: &[&[Vertex]; 6],
    buffers: &mut MeshBuffers,
    ambient_occlusion: bool,
    solid_leaves: bool,
    // how many times the texture repeats across a cell
    uv_tiles: f32,
    grass_tint: &GrassTint,
    light: &impl Deref<Target = SkyLight>,
) {
    let size = CHUNK_SIZE as i32;
    let n = size as usize;
    let mut mask: Vec<Option<FaceCell>> = vec![None; n * n];

    for (i, template) in face_vertices.iter().enumerate() {
        let normal = FACE_NORMAL_OFFSET[i];
        let (ta, tb) = FACE_TANGENTS[i];
        let (a_axis, b_axis) = (axis_of(ta), axis_of(tb));
        let uv_axes = [0, 1].map(|k| if uv_tangent(template, k, a_axis) == 0 { a_axis } else { b_axis });
        let uv_ascending = [0, 1].map(|k| {
            template
                .iter()
                .find(|v| v.position[uv_axes[k]] > 0.0)
                .is_some_and(|v| v.uv[k] > 0.5)
        });
        let normal_axis = axis_of(normal);
        let normal_unit = normal.abs();
        let (p0, p1, p2) = (Vec3::from(template[0].position), Vec3::from(template[1].position), Vec3::from(template[2].position));
        let ccw_in_ab = (p1 - p0).cross(p2 - p0).dot(ta.as_vec3().cross(tb.as_vec3())) > 0.0;

        for slice in 0..size {
            for b in 0..size {
                for a in 0..size {
                    let coord = normal_unit * slice + ta * a + tb * b;
                    let block = source.block(coord);
                    // past the mesh's edge the terrain may be drawn at another detail, with no
                    // leaves just there, so leaves on the edge keep their faces
                    let inside = source.contains(coord + normal);
                    let span = face_span(source, coord, normal, block, solid_leaves && inside);
                    // unshaded leaves merge into big quads, which keeps the faces between them cheap,
                    // so they and coarse LODs take one light level a face instead of smooth light
                    let smooth = ambient_occlusion && block != BlockType::Leaves;
                    // a step above lowered ground faces into that ground's cell, which is dark: it's
                    // lit by the air over it, and is too thin to shade smoothly
                    let step = span.is_some() && normal.y == 0 && source.block(coord + normal).is_opaque();
                    // A skirt wall hangs from the node's own ground over solid cells beyond, whose
                    // occlusion and light are none, so it is lit as that ground is, by the open air above.
                    let skirt = !inside && normal.y == 0 && source.has_skirts();
                    let smooth = smooth && !step && !skirt;
                    let lit_from = if step { coord + normal + IVec3::Y } else { coord + normal };
                    let skirt_light = skirt.then(|| {
                        let air = (0..=SKIRT_CELLS + 1).find(|&dy| !is_solid(source.terrain(coord + IVec3::Y * dy)));
                        face_light(air.map_or(MAX_LIGHT, |dy| light.level(coord + IVec3::Y * dy)))
                    });
                    mask[(b * size + a) as usize] = span.map(|span| FaceCell {
                        span,
                        block,
                        ao: if smooth {
                            std::array::from_fn(|j| vertex_ao(source, coord, i, template[j].position))
                        } else {
                            [1.0; 4]
                        },
                        light: if smooth {
                            std::array::from_fn(|j| vertex_light(source, light, coord, i, template[j].position))
                        } else {
                            [skirt_light.unwrap_or_else(|| face_light(light.level(lit_from))); 4]
                        },
                    });
                }
            }

            for b0 in 0..n {
                for a0 in 0..n {
                    let Some(cell) = mask[b0 * n + a0] else {
                        continue;
                    };

                    let (mut w, mut h) = (1, 1);
                    // a trimmed side's top and bottom sit off the cell grid, so it merges only
                    // along the ground's contour, where its neighbours share those heights
                    let trimmed_side = cell.span.is_some() && normal.y == 0;
                    let (grow_a, grow_b) = (!trimmed_side || a_axis != 1, !trimmed_side || b_axis != 1);
                    if cell.flat() {
                        while grow_a && a0 + w < n && mask[b0 * n + a0 + w].is_some_and(|o| cell.merges_with(&o)) {
                            w += 1;
                        }
                        while grow_b
                            && b0 + h < n
                            && (a0..a0 + w).all(|a| mask[(b0 + h) * n + a].is_some_and(|o| cell.merges_with(&o)))
                        {
                            h += 1;
                        }
                    }
                    for b in b0..b0 + h {
                        for a in a0..a0 + w {
                            mask[b * n + a] = None;
                        }
                    }

                    let origin = (normal_unit * slice + ta * a0 as i32 + tb * b0 as i32).as_vec3();
                    let atlas_cell = (cell.block as usize - 1) as f32;
                    let color = |ao: f32, position: Vec3| {
                        // leaves share grass's tint, over a darker texture
                        let tinted = matches!(cell.block, BlockType::Grass | BlockType::Leaves);
                        let tint = if tinted { grass_tint.at(position) } else { Vec3::ONE };
                        let shade = match cell.block {
                            BlockType::Leaves => source.canopy_shade(position),
                            BlockType::Grass => source.forest_shade(position),
                            _ => 1.0,
                        };
                        (tint * ao * shade).extend(atlas_cell).to_array()
                    };
                    if w == 1 && h == 1 {
                        let positions: [Vec3; 4] = std::array::from_fn(|j| {
                            let mut position = Vec3::from(template[j].position) + origin;
                            if let Some([floor, ceiling]) = cell.span {
                                position.y = if template[j].position[1] > 0.0 { ceiling } else { floor };
                            }
                            position
                        });
                        let uvs = std::array::from_fn(|j| template[j].uv.map(|c| c * uv_tiles));
                        let colors = std::array::from_fn(|j| color(cell.ao[j], positions[j]));
                        let positions = positions.map(|p| p.to_array());
                        buffers.push_tiled_quad(positions, template[0].normal, uvs, colors, cell.light);
                        continue;
                    }

                    let mut corner = origin;
                    corner[a_axis] -= 0.5;
                    corner[b_axis] -= 0.5;
                    corner[normal_axis] += template[0].position[normal_axis];
                    let (mut ta_step, mut tb_step) = (ta.as_vec3(), tb.as_vec3());
                    match cell.span {
                        Some([_, ceiling]) if normal.y > 0 => corner.y = ceiling,
                        Some([floor, ceiling]) => {
                            corner.y = floor;
                            // the side's one step up spans just its trimmed height
                            let up = if a_axis == 1 { &mut ta_step } else { &mut tb_step };
                            up.y *= ceiling - floor;
                        }
                        None => {}
                    }
                    let extent = [w as f32, h as f32];
                    let uv_at = |i: usize, j: usize| {
                        let along = [i as f32, j as f32];
                        [0, 1].map(|k| {
                            let t = (uv_axes[k] == b_axis) as usize;
                            (if uv_ascending[k] { along[t] } else { extent[t] - along[t] }) * uv_tiles
                        })
                    };
                    buffers.push_fan_quad(FanQuad {
                        corner,
                        ta: ta_step,
                        tb: tb_step,
                        size: (w, h),
                        normal: template[0].normal,
                        color: &|position| color(cell.ao[0], position),
                        light: cell.light[0],
                        uv_at: &uv_at,
                        reversed: !ccw_in_ab,
                    });
                }
            }
        }
    }
}

// `cell_size` is how many blocks wide each cell of `source` is. `solid_leaves` is whether
// leaves are drawn without their holes, so hide the leaves behind them.
fn mesh_blocks(
    source: &impl BlockSource,
    cell_size: i64,
    ambient_occlusion: bool,
    textured: bool,
    solid_leaves: bool,
    grass_tint: &GrassTint,
    light: &impl Deref<Target = SkyLight>,
) -> ChunkMeshes {
    let mut opaque = MeshBuffers {
        untextured: !textured,
        ..Default::default()
    };
    let mut water = MeshBuffers::default();

    let cube_vertices = crate::util::primitives::cube();
    let face_vertices = [
        &cube_vertices[0..4],   // front
        &cube_vertices[4..8],   // right
        &cube_vertices[8..12],  // left
        &cube_vertices[12..16], // back
        &cube_vertices[16..20], // top
        &cube_vertices[20..24], // bottom
    ];

    // Water's surface sits below a full block's height (see `cube_with_top`), so its
    // top/side geometry is shorter than every other block's.
    let water_cube_vertices =
        crate::util::primitives::cube_with_top(0.5 - (0.5 - WATER_TOP) / cell_size as f32);
    let water_face_vertices = [
        &water_cube_vertices[0..4],
        &water_cube_vertices[4..8],
        &water_cube_vertices[8..12],
        &water_cube_vertices[12..16],
        &water_cube_vertices[16..20],
        &water_cube_vertices[20..24],
    ];

    // columns' water depths, shared between the corners that sample them
    let mut depths = HashMap::new();
    let mut depth_of =
        |coord: IVec3| *depths.entry(coord).or_insert_with(|| source.water_depth_blocks(coord));

    let size = CHUNK_SIZE as i32;
    let mut blocks = vec![BlockType::Air; (size * size * size) as usize];
    let non_air = source.non_air();
    for &(coord, block) in &non_air {
        blocks[grid_index(size, coord)] = block;
    }

    for (block_coord, block) in non_air {
        if block != BlockType::Water {
            continue;
        }

        let world_position = block_coord.as_vec3();

        // Order matches `water_face_vertices` and `FACE_NORMAL_OFFSET`.
        let sides = FACE_NORMAL_OFFSET.map(|offset| source.block(block_coord + offset));
        for (i, side) in sides.iter().enumerate() {
            if !matches!(side, BlockType::Air | BlockType::Leaves) {
                continue;
            }

            // Water's color channels carry baked shoreline/depth/top-face/distance
            // data (R/G/B/A, see `vertex_shore_factor`/`water_depth`/
            // `vertex_shore_distance`), and only the top face (i == 4) gets it, since
            // foam/waves are surface-only.
            //
            // B (is_top) is deliberately *not* depth-scaled: wave displacement in the
            // shader is gated by this flag alone, with a uniform amplitude for every
            // top face. If amplitude varied with each block's own depth instead, two
            // neighbouring water columns with slightly different depths would animate
            // their shared edge by different amounts and visibly tear apart - depth
            // only ever affects per-pixel color/alpha (G), which can't crack.
            let is_top = i == 4;
            let colors: Vec<[f32; 4]> = water_face_vertices[i]
                .iter()
                .map(|v| {
                    let depth_factor = if is_top {
                        vertex_water_depth(&mut depth_of, block_coord, i, v.position) / CHUNK_SIZE as f32
                    } else {
                        0.0
                    };
                    let (shore, shore_distance) = if is_top {
                        (
                            vertex_shore_factor(source, block_coord, i, v.position),
                            vertex_shore_distance(source, block_coord, i, v.position),
                        )
                    } else {
                        (0.0, 1.0)
                    };
                    [shore, depth_factor, if is_top { 1.0 } else { 0.0 }, shore_distance]
                })
                .collect();
            water.push_face(water_face_vertices[i], world_position, (BlockType::Water as usize - 1) as f32, &colors);
        }
    }

    mesh_opaque_greedy(
        &GridView { size, blocks: &blocks, source },
        &face_vertices,
        &mut opaque,
        ambient_occlusion,
        solid_leaves,
        if textured { cell_size as f32 } else { 1.0 },
        grass_tint,
        light,
    );
    // sky light there is rarely bright under a canopy, and these boxes are too small to shade smoothly
    const BOX_LIGHT: f32 = 0.8;
    opaque.boxes = Some((opaque.vertices.len(), cell_size > BOX_TEXTURE_MAX_CELL));
    for tree_box in source.tree_boxes() {
        let (centre, extent) = ((tree_box.min + tree_box.max) / 2.0, tree_box.max - tree_box.min);
        let atlas_cell = (tree_box.block as usize - 1) as f32;
        let tint = if tree_box.block == BlockType::Leaves { grass_tint.at(centre) } else { Vec3::ONE };
        // a trunk has no bottom face, it stands on the ground
        let faces = if tree_box.block == BlockType::Log { &face_vertices[..5] } else { &face_vertices[..] };
        for face in faces {
            let positions = std::array::from_fn(|j| (centre + Vec3::from(face[j].position) * extent).to_array());
            let uvs = tiled_uvs(&positions, &std::array::from_fn(|j| face[j].uv), cell_size as f32);
            opaque.push_tiled_quad(positions, face[0].normal, uvs, [tint.extend(atlas_cell).to_array(); 4], [BOX_LIGHT; 4]);
        }
    }

    let water_mesh = if water.vertices.is_empty() {
        None
    } else {
        Some(water.build_mesh())
    };

    ChunkMeshes {
        opaque: opaque.build_mesh(),
        water: water_mesh,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bevy::math::{I64Vec3, IVec3, U16Vec3};
    use bevy::math::Vec3;
    use bevy::render::mesh::{Indices, Mesh, VertexAttributeValues};
    use bevy::platform::collections::HashMap;

    use crate::block::BlockType;

    use super::{
        generate_chunk, generate_chunk_mesh, resolve_block, terrain_block, vertex_ao,
        vertex_water_depth, water_depth, BlockSource, ChunkMeshes, ChunkNeighbourhood, GrassTint, LodBlocks, AO_STRENGTH,
        WATER_LEVEL,
    };
    use crate::chunks::chunk::{neighbor_26_index, ChunkCoordinate, ChunkData};
    use crate::chunks::generate::noise::NoiseGenerator;
    use crate::chunks::generate::region_store::RegionStore;
    use crate::chunks::lod::LodKey;

    fn no_neighbors() -> [Option<Arc<ChunkData>>; 26] {
        std::array::from_fn(|_| None)
    }

    #[test]
    fn test_resolve_block_within_chunk() {
        let mut chunk = ChunkData::default();
        chunk.set_block_at(U16Vec3::new(4, 4, 4), BlockType::Stone);

        let block = resolve_block(&chunk, &no_neighbors(), IVec3::new(4, 4, 4));
        assert_eq!(BlockType::Stone, block);
    }

    #[test]
    fn test_resolve_block_crosses_single_axis_into_adjacent_chunk() {
        let chunk = ChunkData::default();
        let mut adjacent = ChunkData::default();
        adjacent.set_block_at(U16Vec3::new(0, 4, 4), BlockType::Stone);

        let mut neighbors = no_neighbors();
        neighbors[neighbor_26_index(1, 0, 0)] = Some(Arc::new(adjacent));
        let block = resolve_block(&chunk, &neighbors, IVec3::new(chunk.size as i32, 4, 4));
        assert_eq!(BlockType::Stone, block);
    }

    #[test]
    fn test_resolve_block_crosses_two_axes_into_diagonal_chunk() {
        let chunk = ChunkData::default();
        let mut diagonal = ChunkData::default();
        diagonal.set_block_at(U16Vec3::new(0, 0, 4), BlockType::Stone);

        let mut neighbors = no_neighbors();
        neighbors[neighbor_26_index(1, 1, 0)] = Some(Arc::new(diagonal));
        let block = resolve_block(
            &chunk,
            &neighbors,
            IVec3::new(chunk.size as i32, chunk.size as i32, 4),
        );
        assert_eq!(BlockType::Stone, block);
    }

    #[test]
    fn test_resolve_block_missing_diagonal_chunk_is_air() {
        let chunk = ChunkData::default();
        let block = resolve_block(
            &chunk,
            &no_neighbors(),
            IVec3::new(chunk.size as i32, chunk.size as i32, 4),
        );
        assert_eq!(BlockType::Air, block);
    }

    #[test]
    fn test_vertex_ao_no_occluders_is_unlit() {
        let chunk = ChunkData::default();
        let neighbors = no_neighbors();
        let source = ChunkNeighbourhood {
            chunk: &chunk,
            neighbors: &neighbors,
        };
        let ao = vertex_ao(&source, IVec3::new(4, 4, 4), 4, [0.5, 0.5, -0.5]);
        assert_eq!(1.0, ao);
    }

    #[test]
    fn test_vertex_ao_fully_occluded_corner() {
        let mut chunk = ChunkData::default();
        // Top face (index 4) at (4,4,4): occluders are the y+1 layer's tangent/diagonal cells.
        chunk.set_block_at(U16Vec3::new(5, 5, 4), BlockType::Stone);
        chunk.set_block_at(U16Vec3::new(4, 5, 5), BlockType::Stone);
        chunk.set_block_at(U16Vec3::new(5, 5, 5), BlockType::Stone);

        let neighbors = no_neighbors();
        let source = ChunkNeighbourhood {
            chunk: &chunk,
            neighbors: &neighbors,
        };
        let ao = vertex_ao(&source, IVec3::new(4, 4, 4), 4, [0.5, 0.5, 0.5]);
        assert_eq!(1.0 - AO_STRENGTH, ao);
    }

    #[test]
    fn test_neighbouring_water_columns_agree_on_shared_corner_depth() {
        let mut chunk = ChunkData::default();
        // two columns with their surface at y = 5: one 5 deep, its +x neighbour 2 deep
        chunk.set_block_at(U16Vec3::new(4, 0, 4), BlockType::Stone);
        for y in 1..=5 {
            chunk.set_block_at(U16Vec3::new(4, y, 4), BlockType::Water);
        }
        for y in 0..=3 {
            chunk.set_block_at(U16Vec3::new(5, y, 4), BlockType::Stone);
        }
        for y in 4..=5 {
            chunk.set_block_at(U16Vec3::new(5, y, 4), BlockType::Water);
        }
        let neighbors = no_neighbors();
        let source = ChunkNeighbourhood {
            chunk: &chunk,
            neighbors: &neighbors,
        };
        let mut depth_of = |coord: IVec3| water_depth(&source, coord) as f32;

        // the corner between them, on the +z side: seen from each column's top face
        let deep = vertex_water_depth(&mut depth_of, IVec3::new(4, 5, 4), 4, [0.5, 0.5, 0.5]);
        let shallow =
            vertex_water_depth(&mut depth_of, IVec3::new(5, 5, 4), 4, [-0.5, 0.5, 0.5]);
        assert_eq!(deep, shallow);
        // averaged with the two dry columns on the +z side
        assert_eq!((5.0 + 2.0) / 4.0, deep);
    }

    #[test]
    fn test_desert_ground_is_sand_and_other_lowland_grass() {
        use bevy::math::I64Vec2;

        let regions = Arc::new(RegionStore::new(Arc::new(NoiseGenerator::new(3))));
        let lowland = |desert: bool| {
            (-60..60)
                .flat_map(|z| (-60..60).map(move |x| I64Vec2::new(x, z) * 128))
                .find(|p| {
                    let sample = regions.noise().sample(p.x as f64, p.y as f64);
                    let above_sea = sample.height - WATER_LEVEL as f64;
                    (regions.noise().biomes().name(sample.biome) == Some("desert")) == desert
                        && (20.0..60.0).contains(&above_sea)
                        && regions.sampler().gradient(p.x, p.y).0.abs() < 0.5
                        && regions.sampler().gradient(p.x, p.y).1.abs() < 0.5
                })
                .expect("no lowland of that kind near the centre")
        };
        let surface = |p: I64Vec2| {
            let top = regions.get_height(p.x, p.y).round() as i64 - 1;
            let coord = ChunkCoordinate(I64Vec3::new(p.x.div_euclid(16), top.div_euclid(16), p.y.div_euclid(16)));
            let local = U16Vec3::new(p.x.rem_euclid(16) as u16, top.rem_euclid(16) as u16, p.y.rem_euclid(16) as u16);
            generate_chunk(regions.clone(), coord).get_block_at(local)
        };
        assert_eq!(BlockType::Sand, surface(lowland(true)));
        assert_eq!(BlockType::Grass, surface(lowland(false)));
    }

    #[test]
    fn test_trees_across_chunk_borders_are_whole() {
        use crate::chunks::generate::trees::trees_near;
        use bevy::math::I64Vec2;

        let regions = Arc::new(RegionStore::new(Arc::new(NoiseGenerator::new(5))));
        let mut heights = regions.sampler();
        let trees = trees_near(regions.noise(), &mut heights, I64Vec2::splat(-800), I64Vec2::splat(800));
        let chunk_of = |p: I64Vec3| p.div_euclid(I64Vec3::splat(16));
        let tree = trees
            .iter()
            .find(|t| {
                let (low, high) = t.bounds();
                chunk_of(low).x != chunk_of(high).x && chunk_of(low).z != chunk_of(high).z
            })
            .expect("no tree straddling a chunk corner");

        let mut chunks = HashMap::new();
        let (low, high) = tree.bounds();
        let mut checked = 0;
        tree.plant(low, 1, (IVec3::ZERO, (high - low).as_ivec3()), |cell, block| {
            let p = low + cell.as_i64vec3();
            let coord = chunk_of(p);
            let chunk = chunks
                .entry(coord)
                .or_insert_with(|| generate_chunk(regions.clone(), ChunkCoordinate(coord)));
            let local = (p - coord * 16).as_u16vec3();
            let found = chunk.get_block_at(local);
            // only another tree's log may stand in for this one's leaves
            assert!(found == block || (block == BlockType::Leaves && found == BlockType::Log), "{block:?} at {p} is {found:?}");
            checked += 1;
        });
        assert!(chunks.len() >= 2 && checked > 20, "{} blocks over {} chunks", checked, chunks.len());
    }

    #[test]
    fn test_terrain_block_floor_and_water() {
        // below the solid floor even a zero-height column is solid
        assert_eq!(BlockType::Sand, terrain_block(5, 0, 0.0, 0.5));
        assert_eq!(BlockType::Water, terrain_block(20, 0, 0.0, 0.5));
        assert_eq!(BlockType::Air, terrain_block(WATER_LEVEL, 0, 0.0, 0.5));
        assert_eq!(BlockType::Grass, terrain_block(WATER_LEVEL + 10, WATER_LEVEL + 20, 0.0, 0.5));
    }

    #[test]
    fn test_snow_settles_on_steeper_ground_higher_up() {
        let at = |above_sea: i64, slope: f64| {
            let y = WATER_LEVEL + above_sea;
            terrain_block(y, y + 1, slope, 0.5)
        };
        assert_ne!(BlockType::Snow, at(200, 0.0));
        assert_eq!(BlockType::Snow, at(230, 1.0));
        assert_ne!(BlockType::Snow, at(230, 3.0));
        assert_eq!(BlockType::Snow, at(360, 3.0));
        // cliffs stay bare however high they are
        assert_ne!(BlockType::Snow, at(400, 6.0));
    }

    #[test]
    fn test_snow_line_drops_in_the_cold_and_hot_ground_is_bare() {
        let at = |above_sea: i64, temperature: f32| {
            let y = WATER_LEVEL + above_sea;
            terrain_block(y, y + 1, 0.0, temperature)
        };
        assert_ne!(BlockType::Snow, at(100, 0.5));
        assert_eq!(BlockType::Snow, at(100, 0.35));
        assert_eq!(BlockType::Snow, at(300, 0.5));
        // too hot for snow however high, so the peaks are rock
        assert_eq!(BlockType::Stone, at(300, 0.8));
        assert_eq!(BlockType::Grass, at(100, 0.8));
    }

    #[test]
    fn test_only_the_top_of_frozen_grass_is_snow() {
        let at = |y: i64, temperature: f32| terrain_block(y, WATER_LEVEL + 20, 0.0, temperature);
        let top = WATER_LEVEL + 19;
        assert_eq!(BlockType::Snow, at(top, 0.05));
        assert_eq!(BlockType::Grass, at(top, 0.3));
        assert_eq!(BlockType::Grass, at(top - 1, 0.05));
    }

    fn test_regions() -> Arc<RegionStore> {
        Arc::new(RegionStore::new(Arc::new(NoiseGenerator::new(3))))
    }

    #[test]
    fn test_lod_level_zero_matches_full_detail_chunk() {
        let regions = test_regions();
        for coord in [
            I64Vec3::new(1, 1, 2),
            I64Vec3::new(-2, 0, 3),
            I64Vec3::new(0, 2, -1),
        ] {
            let chunk = generate_chunk(regions.clone(), ChunkCoordinate(coord));
            let lod = LodBlocks::sample(&regions, LodKey { level: 0, coord });
            for x in 0..16 {
                for y in 0..16 {
                    for z in 0..16 {
                        assert_eq!(
                            chunk.get_block_at(U16Vec3::new(x, y, z)),
                            lod.raw(IVec3::new(x as i32, y as i32, z as i32)),
                            "chunk {coord} block ({x},{y},{z})"
                        );
                    }
                }
            }
        }
    }

    fn flat_lod(surface_y: i32) -> LodBlocks {
        let size = 16;
        let padded = (size + 2) as usize;
        let mut cells = vec![BlockType::Air; padded * padded * padded];
        for x in -1..=size {
            for z in -1..=size {
                for y in -1..surface_y {
                    cells[LodBlocks::index(size, IVec3::new(x, y, z))] = BlockType::Stone;
                }
            }
        }
        let tops = vec![surface_y as i64; padded * padded];
        LodBlocks::new(size, cells, tops, 0, 1)
    }

    // Grass two cells of 4 blocks deep, its ground 6 blocks up the node for x < 8 and 7 blocks
    // beyond: both inside the second cell, so its top is lowered. Above the sea, under open sky.
    fn sloped_lod() -> LodBlocks {
        let base_y = WATER_LEVEL + 64;
        let size = 16;
        let padded = (size + 2) as usize;
        let mut cells = vec![BlockType::Air; padded * padded * padded];
        let mut tops = vec![0; padded * padded];
        for x in -1..=size {
            for z in -1..=size {
                tops[LodBlocks::column(size, x, z)] = base_y + if x < 8 { 6 } else { 7 };
                for y in -1..=1 {
                    cells[LodBlocks::index(size, IVec3::new(x, y, z))] = BlockType::Grass;
                }
            }
        }
        LodBlocks::new(size, cells, tops, base_y, 4)
    }

    // `lod`'s opaque mesh, lit as `generate_lod_mesh` lights it
    fn mesh_lod(lod: &LodBlocks) -> Mesh {
        let top = lod.base_y + (lod.size as i64 + 1) * lod.cell;
        let opacity = lod.cells.iter().map(BlockType::light_opacity).collect();
        let light = Box::new(super::SkyLight::new(IVec3::splat(-1), IVec3::splat(lod.size + 2), super::sky_above(top, lod.cell), opacity));
        let tint = GrassTint::new(&NoiseGenerator::new(3), Vec3::ZERO, lod.cell as f32);
        super::mesh_blocks(lod, lod.cell, false, false, true, &tint, &light).opaque
    }

    // each face's normal and its vertices' positions
    fn faces(mesh: &Mesh) -> Vec<([f32; 3], Vec<[f32; 3]>)> {
        let Some(VertexAttributeValues::Float32x3(positions)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else {
            panic!("missing positions");
        };
        let Some(VertexAttributeValues::Float32x3(normals)) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL) else {
            panic!("missing normals");
        };
        let mut faces: Vec<([f32; 3], Vec<[f32; 3]>)> = Vec::new();
        for (p, n) in positions.iter().zip(normals) {
            match faces.iter_mut().find(|(normal, vs)| normal == n && vs.len() < 4) {
                Some((_, vs)) => vs.push(*p),
                None => faces.push((*n, vec![*p])),
            }
        }
        faces
    }

    #[test]
    fn test_lod_surface_tops_out_at_the_grounds_height() {
        let lod = sloped_lod();
        // (6 - 0) / 4 - 0.5 and (7 - 0) / 4 - 0.5, in cell units centred on the cells
        assert_eq!(Some(1.0), lod.surface_top(IVec3::new(2, 1, 2)));
        assert_eq!(Some(1.25), lod.surface_top(IVec3::new(10, 1, 2)));
        assert_eq!(None, lod.surface_top(IVec3::new(2, 0, 2)));
    }

    #[test]
    fn test_lod_surface_under_water_fills_its_cell() {
        let LodBlocks { size, mut cells, tops, base_y, cell, .. } = sloped_lod();
        cells[LodBlocks::index(16, IVec3::new(2, 2, 2))] = BlockType::Water;
        // nor does one beside water, which would show a gap under the water's side
        cells[LodBlocks::index(16, IVec3::new(6, 1, 2))] = BlockType::Water;
        let lod = LodBlocks::new(size, cells, tops, base_y, cell);
        assert_eq!(None, lod.surface_top(IVec3::new(2, 1, 2)));
        assert_eq!(None, lod.surface_top(IVec3::new(5, 1, 2)));
        assert_eq!(Some(1.0), lod.surface_top(IVec3::new(4, 1, 2)));
    }

    #[test]
    fn test_lod_tops_are_drawn_at_the_grounds_height_with_a_step_between() {
        let faces = faces(&mesh_lod(&sloped_lod()));
        let tops: Vec<f32> = faces
            .iter()
            .filter(|(n, _)| *n == [0.0, 1.0, 0.0])
            .flat_map(|(_, vs)| vs.iter().map(|v| v[1]))
            .collect();
        assert!(tops.contains(&1.0) && tops.contains(&1.25));
        assert!(tops.iter().all(|&y| y == 1.0 || y == 1.25), "tops at {tops:?}");
        // the higher ground's side, facing the lower, covers just the step between them
        let step: Vec<_> = faces
            .iter()
            .filter(|(n, vs)| *n == [-1.0, 0.0, 0.0] && vs.iter().all(|v| v[0] == 7.5))
            .collect();
        assert!(!step.is_empty());
        // one face along the whole contour, not one for each of its 16 cells
        let vertices: usize = step.iter().map(|(_, vs)| vs.len()).sum();
        assert!(vertices < 16 * 4, "{vertices} vertices along the step");
        for (_, vs) in step {
            assert!(vs.iter().all(|v| v[1] == 1.0 || v[1] == 1.25), "step at {vs:?}");
        }
    }

    #[test]
    fn test_lod_steps_are_lit_by_the_air_above_the_lower_ground() {
        let mesh = mesh_lod(&sloped_lod());
        let Some(VertexAttributeValues::Float32x3(positions)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else {
            panic!("missing positions");
        };
        let Some(VertexAttributeValues::Float32x3(normals)) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL) else {
            panic!("missing normals");
        };
        let Some(VertexAttributeValues::Float32x2(lights)) = mesh.attribute(Mesh::ATTRIBUTE_UV_1) else {
            panic!("missing light");
        };
        let step: Vec<f32> = (0..positions.len())
            .filter(|&i| normals[i] == [-1.0, 0.0, 0.0] && positions[i][0] == 7.5 && positions[i][1] >= 1.0)
            .map(|i| lights[i][0])
            .collect();
        assert!(!step.is_empty());
        assert!(step.iter().all(|&l| l == 1.0), "steps under open sky lit {step:?}");
    }

    fn mesh_chunk(chunk: ChunkData) -> ChunkMeshes {
        // well above the sea, under open sky
        let coord = ChunkCoordinate(I64Vec3::new(0, 40, 0));
        generate_chunk_mesh(Arc::new(chunk), vec![None; 26], coord, &NoiseGenerator::new(3), false)
    }

    fn mesh_vertices(chunk: ChunkData) -> usize {
        mesh_chunk(chunk).opaque.count_vertices()
    }

    #[test]
    fn test_single_block_has_six_quads() {
        let mut chunk = ChunkData::default();
        chunk.set_block_at(U16Vec3::new(4, 4, 4), BlockType::Stone);
        assert_eq!(24, mesh_vertices(chunk));
    }

    #[test]
    fn test_flat_floor_merges_into_one_quad_per_face() {
        let mut chunk = ChunkData::default();
        for x in 0..16 {
            for z in 0..16 {
                chunk.set_block_at(U16Vec3::new(x, 0, z), BlockType::Stone);
            }
        }
        // each quad is a centre point plus a vertex per block step round its border: the top
        // is 16x16, the four sides 16x1. The underside shades as light creeps in under it.
        let mesh = mesh_chunk(chunk).opaque;
        let Some(VertexAttributeValues::Float32x3(normals)) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL) else {
            panic!("missing normals");
        };
        let facing = |n: [f32; 3]| normals.iter().filter(|&&v| v == n).count();
        assert_eq!(1 + 64, facing([0.0, 1.0, 0.0]));
        for side in [[1.0, 0.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 0.0, -1.0]] {
            assert_eq!(1 + 34, facing(side));
        }
    }

    #[test]
    fn test_different_block_types_do_not_merge() {
        let mut chunk = ChunkData::default();
        chunk.set_block_at(U16Vec3::new(4, 4, 4), BlockType::Stone);
        chunk.set_block_at(U16Vec3::new(5, 4, 4), BlockType::Grass);
        // 5 faces each, the shared one hidden
        assert_eq!(10 * 4, mesh_vertices(chunk));
    }

    fn floor_chunk() -> ChunkData {
        let mut chunk = ChunkData::default();
        for x in 0..16 {
            for z in 0..16 {
                chunk.set_block_at(U16Vec3::new(x, 0, z), BlockType::Stone);
            }
        }
        chunk
    }

    #[test]
    fn test_merged_quads_tile_their_uvs_and_carry_the_atlas_cell() {
        let mesh = mesh_chunk(floor_chunk()).opaque;
        let Some(VertexAttributeValues::Float32x2(uvs)) = mesh.attribute(Mesh::ATTRIBUTE_UV_0) else {
            panic!("missing uvs");
        };
        let Some(VertexAttributeValues::Float32x4(colors)) = mesh.attribute(Mesh::ATTRIBUTE_COLOR) else {
            panic!("missing colors");
        };
        let tiles = uvs.iter().flatten().fold(0.0f32, |m, &c| m.max(c));
        assert_eq!(16.0, tiles);
        assert!(colors.iter().all(|c| c[3] == (BlockType::Stone as usize - 1) as f32));
    }

    fn top_face_triangles(mesh: &Mesh) -> Vec<f32> {
        let Some(VertexAttributeValues::Float32x3(positions)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else {
            panic!("missing positions");
        };
        let Some(VertexAttributeValues::Float32x3(normals)) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL) else {
            panic!("missing normals");
        };
        let Some(Indices::U32(indices)) = mesh.indices() else {
            panic!("missing indices");
        };
        indices
            .chunks(3)
            .filter(|t| t.iter().all(|&i| normals[i as usize] == [0.0, 1.0, 0.0]))
            .map(|t| {
                let p = |k: usize| Vec3::from(positions[t[k] as usize]);
                (p(1) - p(0)).cross(p(2) - p(0)).y
            })
            .filter(|area| *area != 0.0)
            .collect()
    }

    #[test]
    fn test_merged_quads_keep_the_winding_of_single_faces() {
        let mut single = ChunkData::default();
        single.set_block_at(U16Vec3::new(4, 4, 4), BlockType::Stone);
        let single = mesh_chunk(single).opaque;
        let merged = mesh_chunk(floor_chunk()).opaque;
        let expected = top_face_triangles(&single)[0].signum();
        assert!(top_face_triangles(&merged).iter().all(|area| area.signum() == expected));
    }

    #[test]
    fn test_merged_quad_borders_share_vertices_with_unit_faces() {
        let mesh = mesh_chunk(floor_chunk()).opaque;
        let Some(VertexAttributeValues::Float32x3(positions)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else {
            panic!("missing positions");
        };
        for step in 0..=16 {
            let edge = [step as f32 - 0.5, 0.5, -0.5];
            assert!(positions.contains(&edge), "no top-face vertex at {edge:?}");
        }
    }

    fn tints(mesh: &Mesh) -> Vec<Vec3> {
        let Some(VertexAttributeValues::Float32x4(colors)) = mesh.attribute(Mesh::ATTRIBUTE_COLOR) else {
            panic!("missing colors");
        };
        colors.iter().map(|c| Vec3::new(c[0], c[1], c[2])).collect()
    }

    #[test]
    fn test_only_grass_is_tinted() {
        let mut chunk = ChunkData::default();
        chunk.set_block_at(U16Vec3::new(4, 4, 4), BlockType::Stone);
        chunk.set_block_at(U16Vec3::new(10, 4, 10), BlockType::Grass);
        let colours = tints(&mesh_chunk(chunk).opaque);
        assert_eq!(24, colours.iter().filter(|c| **c == Vec3::ONE).count());
        assert_eq!(24, colours.iter().filter(|c| **c != Vec3::ONE).count());
    }

    #[test]
    fn test_grass_tint_matches_along_shared_edges() {
        let noise = NoiseGenerator::new(3);
        let chunk_tint = |x: i64, z: i64| GrassTint::new(&noise, Vec3::new(x as f32, 0.0, z as f32) * 16.0, 1.0);
        let (here, east, south) = (chunk_tint(5, 5), chunk_tint(6, 5), chunk_tint(5, 6));
        for step in 0..=16 {
            let along = step as f32 - 0.5;
            assert!(here.at(Vec3::new(15.5, 0.0, along)).abs_diff_eq(east.at(Vec3::new(-0.5, 0.0, along)), 1e-5));
            assert!(here.at(Vec3::new(along, 0.0, 15.5)).abs_diff_eq(south.at(Vec3::new(along, 0.0, -0.5)), 1e-5));
        }

        // a LOD node four chunks across, its local units 4 blocks wide, starting at chunk (4, 4)
        let node = GrassTint::new(&noise, Vec3::new(64.0, 0.0, 64.0) + Vec3::splat(1.5), 4.0);
        assert!(node.at(Vec3::splat(-0.5)).abs_diff_eq(chunk_tint(4, 4).at(Vec3::splat(-0.5)), 1e-5));
        assert!(node.at(Vec3::splat(15.5)).abs_diff_eq(chunk_tint(7, 7).at(Vec3::splat(15.5)), 1e-5));
    }

    // top-face sky light at each vertex, by position
    fn top_lights(mesh: &Mesh) -> Vec<([f32; 3], f32)> {
        let Some(VertexAttributeValues::Float32x3(positions)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else {
            panic!("missing positions");
        };
        let Some(VertexAttributeValues::Float32x3(normals)) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL) else {
            panic!("missing normals");
        };
        let Some(VertexAttributeValues::Float32x2(lights)) = mesh.attribute(Mesh::ATTRIBUTE_UV_1) else {
            panic!("missing light");
        };
        (0..positions.len())
            .filter(|&i| normals[i] == [0.0, 1.0, 0.0])
            .map(|i| (positions[i], lights[i][0]))
            .collect()
    }

    #[test]
    fn test_open_ground_is_fully_lit() {
        let lights = top_lights(&mesh_chunk(floor_chunk()).opaque);
        assert!(!lights.is_empty() && lights.iter().all(|(_, l)| *l == 1.0));
    }

    #[test]
    fn test_ground_under_a_roof_is_shaded_smoothly() {
        let mut chunk = floor_chunk();
        // a stone roof over x 4..12, z 4..12, three blocks up
        for x in 4..12 {
            for z in 4..12 {
                chunk.set_block_at(U16Vec3::new(x, 3, z), BlockType::Stone);
            }
        }
        let lights = top_lights(&mesh_chunk(chunk).opaque);
        let floor: Vec<_> = lights.iter().filter(|(p, _)| p[1] == 0.5).collect();
        let (darkest, _) = floor.iter().min_by(|a, b| a.1.total_cmp(&b.1)).unwrap();
        assert!(floor.iter().any(|(_, l)| *l == 1.0), "nowhere on the floor is in the open");
        assert!((4.5..=11.5).contains(&darkest[0]) && (4.5..=11.5).contains(&darkest[2]), "darkest at {darkest:?}");
        let mut levels: Vec<f32> = floor.iter().map(|(_, l)| *l).collect();
        levels.sort_by(f32::total_cmp);
        levels.dedup();
        assert!(levels.len() >= 4, "light falls off in steps {levels:?}");
        // the roof's top is under open sky
        assert!(lights.iter().any(|(p, l)| p[1] == 3.5 && *l == 1.0));
    }

    #[test]
    fn test_lod_ground_under_open_sky_is_lit_to_its_edges() {
        use bevy::math::I64Vec2;

        // a mountainside, where LOD skirts stand in for the solid ground beside the node
        let noise = Arc::new(NoiseGenerator::new(42));
        let p = (-40..40)
            .flat_map(|z| (-40..40).map(move |x| I64Vec2::new(x, z) * 256))
            .find(|p| (150.0..250.0).contains(&(noise.get(*p) - WATER_LEVEL as f64)))
            .expect("no mountainside near the centre");
        let height = noise.get(p) as i64;
        let chunk = ChunkCoordinate(I64Vec3::new(p.x.div_euclid(16), height.div_euclid(16), p.y.div_euclid(16)));
        let regions = Arc::new(RegionStore::new(noise));
        let key = crate::chunks::lod::LodKey::containing(chunk, 1);
        let mesh = super::generate_lod_mesh(regions, key).expect("no mesh").opaque;
        let lights = top_lights(&mesh);
        assert!(!lights.is_empty());
        let dark: Vec<_> = lights.iter().filter(|(_, l)| *l < 1.0).collect();
        assert!(dark.is_empty(), "shaded under open sky: {dark:?}");
    }

    // whether each vertex draws its block's mean colour instead of its texture, see `world.wesl`
    fn untextured_flags(mesh: &Mesh) -> Vec<f32> {
        let Some(VertexAttributeValues::Float32x2(lights)) = mesh.attribute(Mesh::ATTRIBUTE_UV_1) else {
            panic!("missing light");
        };
        lights.iter().map(|l| l[1]).collect()
    }

    #[test]
    fn test_chunks_are_textured() {
        let flags = untextured_flags(&mesh_chunk(floor_chunk()).opaque);
        assert!(!flags.is_empty() && flags.iter().all(|&f| f == 0.0));
    }

    #[test]
    fn test_lods_wider_than_four_blocks_are_untextured() {
        use bevy::math::I64Vec2;

        let noise = Arc::new(NoiseGenerator::new(42));
        let height = noise.get(I64Vec2::ZERO) as i64;
        let chunk = ChunkCoordinate(I64Vec3::new(0, height.div_euclid(16), 0));
        let regions = Arc::new(RegionStore::new(noise));
        let key = crate::chunks::lod::LodKey::containing(chunk, 5);
        assert!(key.cell_size() > super::BOX_TEXTURE_MAX_CELL);
        let flags = untextured_flags(&super::generate_lod_mesh(regions, key).expect("no mesh").opaque);
        assert!(!flags.is_empty() && flags.iter().all(|&f| f == 1.0));
    }

    #[test]
    fn test_tree_boxes_stay_textured_up_to_eight_wide_cells_with_a_tile_a_block() {
        let regions = Arc::new(RegionStore::new(Arc::new(NoiseGenerator::new(5))));
        let mut heights = regions.sampler();
        let trees = crate::chunks::generate::trees::trees_near(regions.noise(), &mut heights, bevy::math::I64Vec2::splat(-300), bevy::math::I64Vec2::splat(300));
        let tree = &trees[0];
        let crown = tree.base + I64Vec3::Y * (tree.trunk - 1);
        let key = LodKey::containing(ChunkCoordinate(crown.div_euclid(I64Vec3::splat(16))), 3);
        assert!(key.cell_size() <= super::BOX_TEXTURE_MAX_CELL);
        let opaque = super::generate_lod_mesh(regions, key).expect("no mesh").opaque;
        let flags = untextured_flags(&opaque);
        let VertexAttributeValues::Float32x2(uvs) = opaque.attribute(Mesh::ATTRIBUTE_UV_0).unwrap() else { panic!() };
        // boxes come last: textured, and counting more than one tile across a crown
        let boxed = flags.len() - flags.iter().rev().take_while(|&&f| f == 0.0).count();
        assert!(boxed < flags.len(), "no textured tree boxes");
        assert!(uvs[boxed..].iter().any(|uv| uv[0] > 3.0 || uv[1] > 3.0));
        assert!(flags[..boxed].iter().all(|&f| f == 1.0));
    }

    #[test]
    fn test_lods_up_to_four_blocks_are_textured_with_the_texture_repeating_over_each_cell() {
        use bevy::math::I64Vec2;

        let noise = Arc::new(NoiseGenerator::new(42));
        let height = noise.get(I64Vec2::ZERO) as i64;
        let chunk = ChunkCoordinate(I64Vec3::new(0, height.div_euclid(16), 0));
        let regions = Arc::new(RegionStore::new(noise));
        let key = crate::chunks::lod::LodKey::containing(chunk, 2);
        assert_eq!(4, key.cell_size());
        let mesh = super::generate_lod_mesh(regions, key).expect("no mesh").opaque;
        let flags = untextured_flags(&mesh);
        assert!(!flags.is_empty() && flags.iter().all(|&f| f == 0.0));
        // a face one cell wide spans four tiles, so none comes out a tile wide
        let uvs = mesh.attribute(Mesh::ATTRIBUTE_UV_0).unwrap();
        let bevy::render::mesh::VertexAttributeValues::Float32x2(uvs) = uvs else { panic!() };
        let tiles = |axis: usize| uvs.iter().map(|uv| uv[axis]).fold(0.0f32, f32::max);
        assert!(tiles(0) >= 4.0 || tiles(1) >= 4.0);
    }

    #[test]
    fn test_ambient_occlusion_stops_faces_merging() {
        let mut chunk = floor_chunk();
        chunk.set_block_at(U16Vec3::new(8, 1, 8), BlockType::Stone);
        // one quad per face of the floor and of the pillar would be 44; the shaded ring around it splits the top
        assert!(mesh_vertices(chunk) > 44);
    }

    // Open air over ground far below the node, with `blocks` set, in cells of 4 blocks.
    fn lod_with(blocks: &[(IVec3, BlockType)]) -> LodBlocks {
        let size = 16;
        let padded = (size + 2) as usize;
        let mut cells = vec![BlockType::Air; padded * padded * padded];
        for &(coord, block) in blocks {
            cells[LodBlocks::index(size, coord)] = block;
        }
        LodBlocks::new(size, cells, vec![0; padded * padded], WATER_LEVEL + 64, 4)
    }

    // a 2x2x2 cube of leaves from `min` up
    fn leaf_cube(min: i32) -> Vec<(IVec3, BlockType)> {
        let mut cube = Vec::new();
        for x in min..min + 2 {
            for y in min..min + 2 {
                for z in min..min + 2 {
                    cube.push((IVec3::new(x, y, z), BlockType::Leaves));
                }
            }
        }
        cube
    }

    // how many vertices lie on faces in the plane at `plane` across their normal's axis
    fn vertices_in_plane(mesh: &Mesh, plane: f32) -> usize {
        let Some(VertexAttributeValues::Float32x3(positions)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else {
            panic!("missing positions");
        };
        let Some(VertexAttributeValues::Float32x3(normals)) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL) else {
            panic!("missing normals");
        };
        positions
            .iter()
            .zip(normals)
            .filter(|(p, n)| (0..3).any(|k| n[k] != 0.0 && p[k] == plane))
            .count()
    }

    #[test]
    fn test_untextured_leaves_hide_the_leaves_behind_them() {
        let mesh = mesh_lod(&lod_with(&leaf_cube(7)));
        // cells 7 and 8 meet at 7.5; the shell is at 6.5 and 8.5
        assert_eq!(0, vertices_in_plane(&mesh, 7.5));
        assert!(vertices_in_plane(&mesh, 6.5) > 0 && vertices_in_plane(&mesh, 8.5) > 0);
        // each side of the shell merges into one quad, a centre point and 8 round its border
        assert_eq!(6 * 9, mesh.count_vertices());
    }

    #[test]
    fn test_untextured_leaves_still_show_other_blocks() {
        let mut blocks = leaf_cube(7);
        blocks.push((IVec3::new(6, 7, 7), BlockType::Log));
        let faces = faces(&mesh_lod(&lod_with(&blocks)));
        // the log's face into the leaves stays: only leaves hide leaves
        assert!(faces.iter().any(|(n, vs)| *n == [1.0, 0.0, 0.0] && vs.iter().all(|v| v[0] == 6.5)));
    }

    #[test]
    fn test_untextured_leaves_on_the_nodes_edge_keep_their_faces() {
        // past the edge the leaves may be drawn at another detail, with none just there
        let lod = lod_with(&[(IVec3::new(15, 4, 4), BlockType::Leaves), (IVec3::new(16, 4, 4), BlockType::Leaves)]);
        let faces = faces(&mesh_lod(&lod));
        assert!(faces.iter().any(|(n, vs)| *n == [1.0, 0.0, 0.0] && vs.iter().all(|v| v[0] == 15.5)));
    }

    fn leaf_cube_chunk() -> ChunkData {
        let mut chunk = ChunkData::default();
        for (coord, block) in leaf_cube(4) {
            chunk.set_block_at(coord.as_u16vec3(), block);
        }
        chunk
    }

    #[test]
    fn test_full_detail_leaves_show_the_leaves_behind_them() {
        // blocks 4 and 5 meet at 4.5, where the faces between them show through the holes
        let mesh = mesh_chunk(leaf_cube_chunk()).opaque;
        assert!(vertices_in_plane(&mesh, 4.5) > 0);
    }

    #[test]
    fn test_full_detail_solid_leaves_hide_the_leaves_behind_them() {
        let coord = ChunkCoordinate(I64Vec3::new(0, 40, 0));
        let mesh = generate_chunk_mesh(Arc::new(leaf_cube_chunk()), vec![None; 26], coord, &NoiseGenerator::new(3), true).opaque;
        assert_eq!(0, vertices_in_plane(&mesh, 4.5));
        assert!(vertices_in_plane(&mesh, 3.5) > 0 && vertices_in_plane(&mesh, 5.5) > 0);
    }

    #[test]
    fn test_lod_border_exposes_surface_cells_as_skirt() {
        let lod = flat_lod(8);
        // the border's top two solid cells read as open, so the edge draws a wall
        assert_eq!(BlockType::Air, lod.block(IVec3::new(-1, 7, 4)));
        assert_eq!(BlockType::Air, lod.block(IVec3::new(-1, 6, 4)));
        assert_eq!(BlockType::Air, lod.block(IVec3::new(4, 7, 16)));
        // deeper border cells and every interior cell read as they are
        assert_eq!(BlockType::Stone, lod.block(IVec3::new(-1, 5, 4)));
        assert_eq!(BlockType::Stone, lod.block(IVec3::new(4, 7, 4)));
        // below the node isn't a horizontal border, so no skirt
        assert_eq!(BlockType::Stone, lod.block(IVec3::new(4, -1, 4)));
    }

    #[test]
    fn test_skirt_walls_are_lit_like_the_open_ground_they_hang_from() {
        // flat ground under open sky, cells a block across, and the cells beyond it solid
        let lod = flat_lod(8);
        let noise = NoiseGenerator::new(1);
        let tint = super::GrassTint::new(&noise, Vec3::ZERO, 1.0);
        let opacity = lod.cells.iter().map(BlockType::light_opacity).collect();
        let light = std::cell::LazyCell::new(|| {
            crate::chunks::generate::light::SkyLight::new(IVec3::splat(-1), IVec3::splat(18), crate::chunks::generate::light::MAX_LIGHT, opacity)
        });
        let mesh = super::mesh_blocks(&lod, 1, true, false, true, &tint, &light).opaque;
        let Some(bevy::render::mesh::VertexAttributeValues::Float32x2(lights)) = mesh.attribute(Mesh::ATTRIBUTE_UV_1) else {
            panic!("no light on the mesh");
        };
        // the top and the four walls, each from a corner to a corner of the node
        assert!(lights.len() >= 4 * 5);
        let darkest = lights.iter().map(|l| l[0]).fold(f32::MAX, f32::min);
        assert!(darkest > 0.6, "a vertex is lit {darkest}, so a skirt wall is dark");
    }

    #[test]
    fn test_lod_skirt_hangs_from_the_nodes_own_ground_not_its_neighbours() {
        // ground reaching the top of the node, where there's no air above to find
        let size = 16;
        let padded = (size + 2) as usize;
        let mut cells = vec![BlockType::Air; padded * padded * padded];
        for x in -1..=size {
            for z in -1..=size {
                for y in -1..=size {
                    cells[LodBlocks::index(size, IVec3::new(x, y, z))] = BlockType::Stone;
                }
            }
        }
        // the ground's top is in the node above, 3 cells over the top row
        let lod = LodBlocks::new(size, cells.clone(), vec![size as i64 + 3; padded * padded], 0, 1);
        assert_eq!(BlockType::Stone, lod.block(IVec3::new(-1, 8, 4)));
        // ground ending in the top row: the cells within two of it have walls, though the top row is solid
        let lod = LodBlocks::new(size, cells, vec![size as i64 + 1; padded * padded], 0, 1);
        assert_eq!(BlockType::Air, lod.block(IVec3::new(-1, 16, 4)));
        assert_eq!(BlockType::Air, lod.block(IVec3::new(-1, 15, 4)));
        assert_eq!(BlockType::Stone, lod.block(IVec3::new(-1, 14, 4)));
    }

    #[test]
    fn test_water_depth_on_lod_border_ignores_skirts() {
        // ground up to y = 3 everywhere, water from 4 to 7
        let mut lod = flat_lod(4);
        for x in -1..=16 {
            for z in -1..=16 {
                for y in 4..8 {
                    lod.cells[LodBlocks::index(16, IVec3::new(x, y, z))] = BlockType::Water;
                }
            }
        }
        // the border's top solid cells read as water for the skirt, but don't add depth
        assert_eq!(BlockType::Water, lod.block(IVec3::new(-1, 3, 4)));
        assert_eq!(4, water_depth(&lod, IVec3::new(4, 7, 4)));
        assert_eq!(4, water_depth(&lod, IVec3::new(-1, 7, 4)));
    }

    #[test]
    fn test_lod_block_clamps_far_probes_to_border() {
        let lod = flat_lod(8);
        assert_eq!(
            lod.block(IVec3::new(-1, 3, 4)),
            lod.block(IVec3::new(-3, 3, 4))
        );
    }

    #[test]
    fn test_buried_chunk_has_no_geometry() {
        let mut stone = ChunkData::default();
        for (x, y, z) in (0..16).flat_map(|x| (0..16).flat_map(move |y| (0..16).map(move |z| (x, y, z)))) {
            stone.set_block_at(U16Vec3::new(x, y, z), BlockType::Stone);
        }
        let stone = Arc::new(stone);
        let meshes = generate_chunk_mesh(
            stone.clone(),
            vec![Some(stone); 26],
            ChunkCoordinate(I64Vec3::ZERO),
            test_regions().noise(),
            false,
        );
        assert!(!meshes.has_geometry());
    }

    #[test]
    fn test_chunk_with_a_block_has_geometry() {
        let mut chunk = ChunkData::default();
        chunk.set_block_at(U16Vec3::new(4, 4, 4), BlockType::Stone);
        assert!(mesh_chunk(chunk).has_geometry());
    }

    #[test]
    fn test_canopy_shade_darkens_toward_the_ground() {
        let padded = 18 * 18;
        let lod = flat_lod(8).with_canopy_tops(vec![40; padded]);
        let at = |y: f32| lod.canopy_shade(Vec3::new(4.0, y, 4.0));
        // a leaf at the canopy's top is unshaded, and the foot of a canopy is the darkest
        assert_eq!(1.0, at(39.5));
        assert!((at(19.5) - (1.0 - super::CANOPY_SHADE)).abs() < 1e-5);
        assert!(at(30.0) < at(35.0) && at(35.0) < at(39.5));
        // below the span it stops getting darker
        assert_eq!(at(19.5), at(9.5));
    }

    #[test]
    fn test_canopy_shade_leaves_bare_columns_and_other_lods_alone() {
        let padded = 18 * 18;
        let bare = flat_lod(8).with_canopy_tops(vec![i64::MIN; padded]);
        assert_eq!(1.0, bare.canopy_shade(Vec3::new(4.0, 2.0, 4.0)));
        assert_eq!(1.0, flat_lod(8).canopy_shade(Vec3::new(4.0, 2.0, 4.0)));
    }

    #[test]
    fn test_a_lone_tree_is_still_there_in_every_cell_up_to_thirty_two_wide() {
        use crate::chunks::generate::trees::trees_near;
        use bevy::math::I64Vec2;
        let regions = Arc::new(RegionStore::new(Arc::new(NoiseGenerator::new(5))));
        let mut heights = regions.sampler();
        let trees = trees_near(regions.noise(), &mut heights, I64Vec2::splat(-300), I64Vec2::splat(300));
        let near = |a: &crate::chunks::generate::trees::Tree, b: &crate::chunks::generate::trees::Tree| {
            (a.base.x - b.base.x).abs() < 40 && (a.base.z - b.base.z).abs() < 40
        };
        let tree = trees.iter().find(|t| trees.iter().filter(|o| near(t, o)).count() == 1).expect("no lone tree");
        let crown = tree.base + I64Vec3::Y * (tree.trunk - 1);
        for level in [1u8, 2, 3, 4, 5] {
            let key = LodKey::containing(ChunkCoordinate(crown.div_euclid(I64Vec3::splat(16))), level);
            let blocks = LodBlocks::sample(&regions, key);
            let cell = key.cell_size();
            let leaf_cells = blocks.cells.iter().filter(|&&c| c == BlockType::Leaves).count();
            let crown_boxes = blocks.boxes.iter().filter(|b| b.block == BlockType::Leaves).count();
            if cell < super::BOX_CROWN_CELL {
                assert!(leaf_cells > 0, "no leaf cells at level {level}, cells {cell} blocks wide");
            } else {
                // a crown is boxes of its real size, never a cell's worth of cube
                assert_eq!(0, leaf_cells);
                let wanted = tree.crown_boxes();
                assert!(!wanted.is_empty() && crown_boxes >= wanted.len(), "{crown_boxes} crown boxes of {} at level {level}", wanted.len());
                for (min, max) in &wanted {
                    let size = (*max - *min).as_vec3();
                    let found = blocks.boxes.iter().filter(|b| b.block == BlockType::Leaves).any(|b| ((b.max - b.min) * cell as f32 - size).abs().max_element() < 1e-3);
                    assert!(found, "no crown box of {size:?} at level {level}");
                }
                let wanted = wanted.iter().fold(Vec3::ZERO, |w, (min, max)| w.max((*max - *min).as_vec3()));
                assert!(wanted.x < 20.0, "crown {wanted:?} wider than a crown should be");
            }
        }
    }

    #[test]
    fn test_a_lone_trees_trunk_is_a_thin_box_in_cells_four_to_sixteen_wide() {
        use crate::chunks::generate::trees::trees_near;
        use bevy::math::I64Vec2;
        let regions = Arc::new(RegionStore::new(Arc::new(NoiseGenerator::new(5))));
        let mut heights = regions.sampler();
        let trees = trees_near(regions.noise(), &mut heights, I64Vec2::splat(-300), I64Vec2::splat(300));
        let near = |a: &crate::chunks::generate::trees::Tree, b: &crate::chunks::generate::trees::Tree| {
            (a.base.x - b.base.x).abs() < 40 && (a.base.z - b.base.z).abs() < 40
        };
        let tree = trees.iter().find(|t| trees.iter().filter(|o| near(t, o)).count() == 1).expect("no lone tree");
        for level in [2u8, 3, 4] {
            let key = LodKey::containing(ChunkCoordinate(tree.base.div_euclid(I64Vec3::splat(16))), level);
            let blocks = LodBlocks::sample(&regions, key);
            let cell = key.cell_size() as f32;
            let at = |block: i64, min: i64| (block - min) as f32 / cell - 0.5;
            let (min_x, min_z) = (key.min_block().x, key.min_block().z);
            let trunk = blocks.boxes.iter().filter(|b| b.block == BlockType::Log).find(|t| {
                (t.min.x - at(tree.base.x, min_x)).abs() < 1e-4
                    && (t.min.z - at(tree.base.z, min_z)).abs() < 1e-4
                    && (t.max.x - t.min.x - tree.width() as f32 / cell).abs() < 1e-4
            });
            assert!(trunk.is_some(), "no trunk box at level {level}: {:?}", blocks.boxes);
            let trunk = trunk.unwrap();
            assert!(((trunk.max.y - trunk.min.y) * cell - tree.trunk as f32).abs() < 1e-3);
            // and no log cells, which would be a pillar several times too thick
            assert_eq!(0, blocks.cells.iter().filter(|&&c| c == BlockType::Log).count());
            // meshed, the box is a few quads standing out of the ground's
            let lone = LodBlocks { boxes: vec![*trunk], ..LodBlocks::new(16, vec![BlockType::Air; 18 * 18 * 18], vec![0; 18 * 18], 0, key.cell_size()) };
            let tint = super::GrassTint::new(regions.noise(), Vec3::ZERO, 1.0);
            let light = std::cell::LazyCell::new(|| {
                crate::chunks::generate::light::SkyLight::new(IVec3::splat(-1), IVec3::splat(18), crate::chunks::generate::light::MAX_LIGHT, vec![0; 18 * 18 * 18])
            });
            let mesh = super::mesh_blocks(&lone, key.cell_size(), false, false, true, &tint, &light).opaque;
            assert_eq!(5 * 4, mesh.count_vertices());
        }
    }

    #[test]
    fn test_forest_tints_the_ground_darker_the_more_of_it_trees_cover() {
        let padded = 18 * 18;
        let mut forest = vec![0.0; padded];
        forest[LodBlocks::column(16, 4, 4)] = 0.5;
        forest[LodBlocks::column(16, 8, 8)] = 1.0;
        let lod = flat_lod(8).with_forest(forest);
        let at = |x: f32, z: f32| lod.forest_shade(Vec3::new(x, 7.5, z));
        assert_eq!(1.0, at(1.0, 1.0));
        assert!(at(4.0, 4.0) < 1.0 && at(8.0, 8.0) < at(4.0, 4.0));
        assert!((at(8.0, 8.0) - super::FOREST_DARKEN).abs() < 1e-5);
        // real trees and bare ground leave the ground as it was
        assert_eq!(1.0, flat_lod(8).forest_shade(Vec3::new(4.0, 7.5, 4.0)));
    }

    #[test]
    fn test_cells_wider_than_real_trees_tint_forest_instead_of_drawing_leaves() {
        use crate::chunks::generate::trees::trees_near;
        use bevy::math::I64Vec2;
        let regions = Arc::new(RegionStore::new(Arc::new(NoiseGenerator::new(5))));
        let mut heights = regions.sampler();
        let trees = trees_near(regions.noise(), &mut heights, I64Vec2::splat(-300), I64Vec2::splat(300));
        let tree = trees.first().expect("no trees");
        let key = LodKey::containing(ChunkCoordinate(tree.base.div_euclid(I64Vec3::splat(16))), 6);
        assert!(key.cell_size() > super::TREE_MAX_CELL);
        let blocks = LodBlocks::sample(&regions, key);
        assert_eq!(0, blocks.cells.iter().filter(|&&c| c == BlockType::Leaves || c == BlockType::Log).count());
        assert!(blocks.boxes.is_empty());
        assert!(blocks.forest.iter().any(|&f| f > 0.0) && blocks.forest.iter().all(|&f| (0.0..=1.0).contains(&f)));
    }

    // Where trees grow within a node's own area, as full detail decides, and as the node's grid does.
    fn tree_spots(regions: &Arc<RegionStore>, key: LodKey) -> (Vec<(i64, i64)>, Vec<(i64, i64)>) {
        use crate::chunks::generate::trees::trees_near;
        use bevy::math::I64Vec2;
        let (cell, base, size) = (key.cell_size(), key.min_block(), crate::chunks::chunk::CHUNK_SIZE as i32);
        let span = (size + 1) as i64 * cell;
        let min = I64Vec2::new(base.x, base.z) - cell;
        let own = |t: &crate::chunks::generate::trees::Tree| {
            (base.x..base.x + size as i64 * cell).contains(&t.base.x) && (base.z..base.z + size as i64 * cell).contains(&t.base.z)
        };
        let mut heights = regions.sampler();
        let mut exact: Vec<_> = trees_near(regions.noise(), &mut heights, min, min + span + cell)
            .iter()
            .filter(|t| own(t))
            .map(|t| (t.base.x, t.base.z))
            .collect();
        let grid = regions.column_grid(key);
        let mut ground = super::GridGround { grid: &grid, noise: regions.noise(), base: I64Vec2::new(base.x, base.z), cell, size };
        let mut coarse: Vec<_> = trees_near(regions.noise(), &mut ground, min, min + span + cell)
            .iter()
            .filter(|t| own(t))
            .map(|t| (t.base.x, t.base.z))
            .collect();
        exact.sort();
        coarse.sort();
        (exact, coarse)
    }

    #[test]
    fn test_every_level_grows_the_trees_full_detail_does() {
        use bevy::math::I64Vec2;
        let regions = Arc::new(RegionStore::new(Arc::new(NoiseGenerator::new(5))));
        for level in [2u8, 3, 4, 5] {
            // nodes over a forested stretch, found from the full-detail trees
            let mut heights = regions.sampler();
            let trees = crate::chunks::generate::trees::trees_near(regions.noise(), &mut heights, I64Vec2::splat(-500), I64Vec2::splat(500));
            let mut checked = 0;
            for tree in trees.iter().step_by(trees.len() / 8 + 1) {
                let key = LodKey::containing(ChunkCoordinate(tree.base.div_euclid(I64Vec3::splat(16))), level);
                let (exact, coarse) = tree_spots(&regions, key);
                assert_eq!(exact, coarse, "level {level} node {key:?}");
                checked += exact.len();
            }
            assert!(checked > 0, "no trees checked at level {level}");
        }
    }

    // Triangles of a node's opaque mesh in world space, placed as `lod_transform` does.
    fn world_triangles(regions: &Arc<RegionStore>, key: LodKey) -> Vec<[Vec3; 3]> {
        let Some(meshes) = super::generate_lod_mesh(regions.clone(), key) else {
            return Vec::new();
        };
        let cell = key.cell_size() as f32;
        let origin = key.min_block().as_vec3() + Vec3::splat(0.5 * cell - 0.5);
        let positions = meshes.opaque.attribute(Mesh::ATTRIBUTE_POSITION).unwrap().as_float3().unwrap();
        let Some(Indices::U32(indices)) = meshes.opaque.indices() else {
            return Vec::new();
        };
        indices
            .chunks(3)
            .map(|t| [0, 1, 2].map(|k| origin + Vec3::from(positions[t[k] as usize]) * cell))
            .collect()
    }

    // Whether the ray from `from` along `direction` meets the triangle, either side.
    fn ray_hits(from: Vec3, direction: Vec3, [a, b, c]: [Vec3; 3]) -> bool {
        let (e1, e2) = (b - a, c - a);
        let p = direction.cross(e2);
        let det = e1.dot(p);
        if det.abs() < 1e-9 {
            return false;
        }
        let t = from - a;
        let u = t.dot(p) / det;
        let q = t.cross(e1);
        let v = direction.dot(q) / det;
        u >= 0.0 && v >= 0.0 && u + v <= 1.0 && e2.dot(q) / det > 0.0
    }

    // How many oblique rays down across the seam between a node and its coarser neighbour pass
    // through every triangle of both, seeing the sky through a crack.
    fn seam_leaks(regions: &Arc<RegionStore>, fine: LodKey, coarse: LodKey) -> (usize, usize) {
        // the nodes above and below too, where the ground crosses into them
        let mut triangles = Vec::new();
        for dy in -1..=1 {
            for key in [fine, coarse] {
                triangles.extend(world_triangles(regions, LodKey { coord: key.coord + I64Vec3::new(0, dy, 0), ..key }));
            }
        }
        let seam_x = coarse.min_block().x as f32;
        let (z0, z1) = (fine.min_block().z as f32, (fine.min_block().z + fine.width() * 16) as f32);
        let mut sampler = regions.sampler();
        let (mut rays, mut leaks) = (0, 0);
        for direction in [Vec3::new(0.35, -1.0, 0.1), Vec3::new(-0.35, -1.0, 0.1), Vec3::new(0.0, -1.0, 0.45), Vec3::new(0.2, -1.0, -0.3)] {
            let direction = direction.normalize();
            // clear of the node's z edges, past which the next node isn't loaded here
            let mut z = z0 + 8.713;
            while z < z1 - 8.0 {
                for dx in [-2.7f32, -1.77, -0.93, 0.21, 0.83, 1.61, 2.9] {
                    let x = seam_x + dx;
                    let y = sampler.height(x as i64, z as i64) as f32;
                    // from well above any hill the ray might otherwise start inside of
                    let from = Vec3::new(x, y, z) - direction * (300.0 / -direction.y);
                    rays += 1;
                    if !triangles.iter().any(|&t| ray_hits(from, direction, t)) {
                        leaks += 1;
                    }
                }
                z += 2.07;
            }
        }
        (leaks, rays)
    }

    #[test]
    fn test_nothing_is_seen_through_a_seam_between_lod_levels() {
        let regions = Arc::new(RegionStore::new(Arc::new(NoiseGenerator::new(5))));
        let mut sampler = regions.sampler();
        // stretches of land a few thousand blocks apart, with a fine node beside a coarser one
        let spots: Vec<(i64, i64)> = (0..8).map(|i| (-3000 + (i % 4) * 1900 + (i * 137) % 400, -3000 + (i / 4) * 1900 + (i * 211) % 400)).collect();
        for fine_level in 1..=4u8 {
            let (mut leaks, mut rays) = (0, 0);
            for &(bx, bz) in &spots {
                let h = sampler.height(bx, bz) as i64;
                let coarse = LodKey::containing(ChunkCoordinate(I64Vec3::new(bx, h, bz).div_euclid(I64Vec3::splat(16))), fine_level + 1);
                let fine_chunk = I64Vec3::new(coarse.min_block().x - 8, h, coarse.min_block().z + 8).div_euclid(I64Vec3::splat(16));
                let fine = LodKey::containing(ChunkCoordinate(fine_chunk), fine_level);
                let (l, r) = seam_leaks(&regions, fine, coarse);
                leaks += l;
                rays += r;
            }
            assert_eq!(0, leaks, "{leaks} of {rays} rays slipped through seams between levels {fine_level} and {}", fine_level + 1);
        }
    }

    #[test]
    #[ignore]
    fn scratch_one_leak() {
        // a leak the real layout showed, from (-224.7, 504.4, -276.3) along (0.329, -0.940, 0.094), with only level 1 nodes near it
        let regions = Arc::new(RegionStore::new(Arc::new(NoiseGenerator::new(5))));
        let _ = regions;
    }

    #[test]
    fn test_ground_a_block_lower_than_a_surface_cell_above_is_still_lidded() {
        // here full detail has the ground a block under the level 1 cell the node above draws it in
        let regions = Arc::new(RegionStore::new(Arc::new(NoiseGenerator::new(2775841323))));
        let key = LodKey { level: 1, coord: I64Vec3::new(6, 13, 4) };
        let meshes = super::generate_lod_mesh(regions, key).unwrap();
        let origin = key.min_block().as_vec3() + Vec3::splat(0.5 * 2.0 - 0.5);
        let Some(bevy::render::mesh::VertexAttributeValues::Float32x3(pos)) = meshes.opaque.attribute(Mesh::ATTRIBUTE_POSITION) else { panic!() };
        let Some(Indices::U32(idx)) = meshes.opaque.indices() else { panic!() };
        let (px, pz) = (198.7f32, 127.7f32);
        let lidded = idx.chunks(3).any(|t| {
            let q = [0, 1, 2].map(|j| origin + Vec3::from(pos[t[j] as usize]) * 2.0);
            let e = |a: Vec3, b: Vec3| (px - b.x) * (a.z - b.z) - (a.x - b.x) * (pz - b.z);
            let d = [e(q[0], q[1]), e(q[1], q[2]), e(q[2], q[0])];
            q.iter().all(|v| v.y == 447.5) && !(d.iter().any(|&x| x < 0.0) && d.iter().any(|&x| x > 0.0))
        });
        assert!(lidded);
    }

    #[test]
    fn test_lod_non_air_only_covers_interior() {
        let lod = flat_lod(8);
        let cells = lod.non_air();
        assert_eq!(16 * 16 * 8, cells.len());
        assert!(cells
            .iter()
            .all(|(c, _)| c.min_element() >= 0 && c.max_element() < 16));
    }

    // Not run as part of normal `cargo test` - this is a before/after timing comparison
    // for the ChunkData storage representation, not a correctness check. Run with:
    //   cargo test --release -- --ignored --nocapture bench_generate_and_mesh_chunks
    // Capture a baseline before changing ChunkData's storage, then re-run after with the
    // same seed/coordinates and compare - `get_block_at`/`set_block_at` are on the hot
    // path here (every block, every face, every AO sample), so this isolates exactly the
    // cost a storage change would add, rather than inferring it from `cargo test` passing.
    #[test]
    #[ignore]
    fn bench_generate_and_mesh_chunks() {
        let noise = Arc::new(NoiseGenerator::new(42));

        // Meshed chunks are the inner ones, so every one has real neighbours on all sides.
        let offset: i64 = std::env::var("BENCH_OFFSET").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
        let mut coords = Vec::new();
        let mut meshed = Vec::new();
        for x in 0..24 {
            for y in -3..10 {
                for z in 0..24 {
                    let coord = ChunkCoordinate(I64Vec3::new(x + offset, y, z + offset));
                    coords.push(coord);
                    if (2..22).contains(&x) && (-1..8).contains(&y) && (2..22).contains(&z) {
                        meshed.push(coord);
                    }
                }
            }
        }

        let regions = Arc::new(RegionStore::new(noise));
        let start = std::time::Instant::now();
        let data: HashMap<ChunkCoordinate, Arc<ChunkData>> = coords
            .iter()
            .map(|&c| (c, Arc::new(generate_chunk(regions.clone(), c))))
            .collect();
        let generate_elapsed = start.elapsed();

        let mut best = std::time::Duration::MAX;
        let mut vertices = 0;
        for _ in 0..3 {
            let start = std::time::Instant::now();
            vertices = 0;
            for &c in &meshed {
                let neighbours = c.neighbors_26().iter().map(|n| data.get(n).cloned()).collect();
                let meshes = generate_chunk_mesh(data[&c].clone(), neighbours, c, regions.noise(), false);
                vertices += meshes.opaque.count_vertices();
            }
            best = best.min(start.elapsed());
        }

        println!(
            "bench_generate_and_mesh_chunks: {} chunks meshed, generate={:?}, mesh={:?} (best of 3), opaque vertices={}",
            meshed.len(),
            generate_elapsed,
            best,
            vertices
        );
    }
}

