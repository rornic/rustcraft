use super::buffers::{ChunkMeshes, GrassTint, InnerLeaves, MeshStyle, TEXTURE_MAX_CELL_SIZE};
use super::canopy::{
    crown_layers, Canopy, BOX_CROWN_CELL, CANOPY_CELL, CANOPY_SHADE, CANOPY_SHADE_SPAN,
    FOREST_COVER, FOREST_DARKEN, TREE_MAX_CELL, TRUNK_MAX_CELL,
};
use super::chunk::{sky_above, trees_may_reach};
use super::mesh::mesh_blocks;
use super::source::{water_depth, BlockSource, BoxFaces, TreeBox};
use super::terrain::{column_block, freeze_cell, ground_top, is_solid, under_grass, underwater};
use crate::block::BlockType;
use crate::chunks::chunk::CHUNK_SIZE;
use crate::chunks::generate::biome::{BiomeId, Biomes};
use crate::chunks::generate::columns::ColumnGrid;
use crate::chunks::generate::columns::COARSE_CELL;
use crate::chunks::generate::light::SkyLight;
use crate::chunks::generate::noise::{NoiseGenerator, TerrainSample};
use crate::chunks::generate::region_store::RegionStore;
use crate::chunks::generate::trees::grows_into;
use crate::chunks::generate::trees::trees_near;
use crate::chunks::generate::trees::Tree;
use crate::chunks::generate::trees::TreeGround;
use crate::chunks::generate::trees::TreeTints;
use crate::chunks::generate::trees::THIN_TRUNK_CELL;
use crate::chunks::generate::WATER_LEVEL;
use crate::chunks::lod::LodKey;
use bevy::math::I64Vec2;
use bevy::math::I64Vec3;
use bevy::math::IVec3;
use bevy::math::Vec3;
use std::cell::LazyCell;
use std::sync::Arc;

pub(super) const SKIRT_CELLS: i32 = 2;

pub(super) struct LodBlocks {
    pub(super) size: i32,
    pub(super) cells: Vec<BlockType>,
    // the ground's height in blocks under each padded column, ground being below it
    pub(super) tops: Vec<i64>,
    // the node's lowest block and cell size, to place `tops` among the cells
    pub(super) base_y: i64,
    pub(super) cell: i64,
    // `surface_top` for each padded cell, worked out once since meshing asks for every face
    pub(super) surface_tops: Vec<Option<f32>>,
    pub(super) any_surface_tops: bool,
    // the top of each padded column's canopy in blocks, `i64::MIN` where it has none, or empty with no canopies at all
    pub(super) canopy_tops: Vec<i64>,
    // each column's ground height before rounding, empty when not known
    pub(super) ground_heights: Vec<f32>,
    // how much of each padded column's ground forest covers, 0..1, or empty where trees are real
    pub(super) forest: Vec<f32>,
    pub(super) boxes: Vec<TreeBox>,
    // the trees whose leaves are among the cells
    pub(super) trees: TreeTints,
}

// The ground a coarse node's trees stand on, read from its grid of columns.
pub(super) struct GridGround<'a> {
    pub(super) grid: &'a ColumnGrid,
    pub(super) noise: &'a NoiseGenerator,
    pub(super) base: I64Vec2,
    pub(super) cell: i64,
    pub(super) size: i32,
    // the last point sampled, which trees ask about several times over
    pub(super) last: Option<(i64, i64, TerrainSample)>,
}

impl GridGround<'_> {
    fn sample(&mut self, x: i64, z: i64) -> TerrainSample {
        match self.last {
            Some((lx, lz, sample)) if (lx, lz) == (x, z) => sample,
            _ => {
                let sample = self.noise.sample(x as f64, z as f64);
                self.last = Some((x, z, sample));
                sample
            }
        }
    }

    pub(super) fn column(&self, x: i64, z: i64) -> (i32, i32) {
        let index = |world: i64, base: i64| {
            ((world - base).div_euclid(self.cell) as i32).clamp(-1, self.size)
        };
        (index(x, self.base.x), index(z, self.base.y))
    }
}

// Whether a tree grows, and where, is decided from the noise at its exact position, as the
// regions full detail reads are made from it, or levels would grow different trees. Only its
// base stands on the ground this node draws.
impl TreeGround for GridGround<'_> {
    fn trees(&mut self, x: i64, z: i64) -> f64 {
        // stored as a byte
        (self.sample(x, z).trees * 255.0).round() / 255.0
    }

    fn height(&mut self, x: i64, z: i64) -> f64 {
        // stored as a float
        self.sample(x, z).height as f32 as f64
    }

    fn biome(&mut self, x: i64, z: i64) -> BiomeId {
        self.sample(x, z).biome
    }

    // From the climate alone, far cheaper than the whole terrain `trees` takes. The density is
    // stored as a byte, which can round it up by half a step.
    fn rules_out(&mut self, x: i64, z: i64, chance: f64) -> bool {
        let climate = self.noise.climate(x as f64, z as f64);
        chance >= self.noise.biomes().tree_bound(climate) + 1.0 / 255.0
    }

    fn base_height(&mut self, x: i64, z: i64) -> i64 {
        let (cx, cz) = self.column(x, z);
        self.grid.height(cx, cz).round() as i64
    }
}

impl LodBlocks {
    // The padded column under a mesh position. A vertex on a cell's edge reads the column on its
    // far side, close enough for a gradient.
    pub(super) fn column_at(&self, position: Vec3) -> usize {
        let column = |c: f32| ((c + 0.5).floor() as i32).clamp(-1, self.size);
        Self::column(self.size, column(position.x), column(position.z))
    }

    pub(super) fn sample(regions: &RegionStore, key: LodKey) -> Self {
        if key.cell_size() >= COARSE_CELL {
            Self::sample_coarse(regions, key)
        } else {
            Self::sample_regions(regions, key)
        }
    }

    // From the noise at each cell's centre, with the real trees where the cells are narrow
    // enough, and forests as a tint on the ground where they're not.
    pub(super) fn sample_coarse(regions: &RegionStore, key: LodKey) -> Self {
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
        let mut forest = if real_trees {
            Vec::new()
        } else {
            vec![0.0; padded * padded]
        };
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
                let ground = column_block(
                    noise.biomes(),
                    biome,
                    height - 1,
                    height,
                    gradient,
                    temperature,
                );
                tops[Self::column(size, cx, cz)] = ground_top(height);
                if !real_trees && ground == BlockType::Grass {
                    forest[Self::column(size, cx, cz)] =
                        (grid.trees(cx, cz) * FOREST_COVER).min(1.0) as f32;
                }
                fill_column(
                    &mut cells,
                    noise.biomes(),
                    (base, cell),
                    (cx, cz),
                    biome,
                    (height, gradient, temperature),
                );
            }
        }
        if real_trees {
            let span = (size + 1) as i64 * cell;
            if trees_may_reach(base.y - cell, base.y + span, lowest, highest) {
                let min = I64Vec2::new(base.x, base.z) - cell;
                let mut ground = GridGround {
                    grid: &grid,
                    noise,
                    base: I64Vec2::new(base.x, base.z),
                    cell,
                    size,
                    last: None,
                };
                let trees = trees_near(noise, &mut ground, min, min + span + cell);
                let canopy = (cell >= CANOPY_CELL).then(|| Canopy::new(&trees, base, cell, size));
                if let Some(canopy) = &canopy {
                    forest = canopy.cover.clone();
                    boxes = canopy.boxes(&ground_heights, base, cell, size);
                }
                // trees in a canopy are drawn as part of it
                let in_canopy = |tree: &Tree| {
                    canopy
                        .as_ref()
                        .is_some_and(|c| c.covers(tree, base, cell, size))
                };
                for tree in trees.iter().filter(|t| !in_canopy(t)) {
                    // each tree's boxes are drawn by the one node its base is in
                    let at = (tree.base - base).div_euclid(I64Vec3::splat(cell));
                    if at.cmpge(I64Vec3::ZERO).all() && at.cmplt(I64Vec3::splat(size as i64)).all()
                    {
                        let mesh_at = |block: Vec3| (block - base.as_vec3()) / cell as f32 - 0.5;
                        if (THIN_TRUNK_CELL..=TRUNK_MAX_CELL).contains(&cell) {
                            let width = tree.width();
                            let (min, max) = (
                                tree.base.as_vec3(),
                                (tree.base + I64Vec3::new(width, tree.trunk, width)).as_vec3(),
                            );
                            boxes.push(TreeBox {
                                min: mesh_at(min),
                                max: mesh_at(max),
                                block: BlockType::Log,
                                tint: Vec3::ONE,
                                faces: BoxFaces::Sides,
                            });
                        }
                        if cell >= BOX_CROWN_CELL {
                            for (min, max) in tree.crown_boxes(crown_layers(cell)) {
                                boxes.push(TreeBox {
                                    min: mesh_at(min.as_vec3()),
                                    max: mesh_at(max.as_vec3()),
                                    block: BlockType::Leaves,
                                    tint: tree.tint(),
                                    faces: BoxFaces::All,
                                });
                            }
                        }
                    }
                    if cell < BOX_CROWN_CELL {
                        tree.plant(
                            base,
                            cell,
                            (IVec3::splat(-1), IVec3::splat(size)),
                            |coord, block| {
                                let cell = &mut cells[Self::index(size, coord)];
                                if grows_into(*cell, block) {
                                    *cell = block;
                                }
                            },
                        );
                    }
                }
            }
            // leaves darken down from the top of each column's highest, see `canopy_shade`
            for cx in -1..=size {
                for cz in -1..=size {
                    let top = (-1..=size).rev().find(|&cy| {
                        cells[Self::index(size, IVec3::new(cx, cy, cz))] == BlockType::Leaves
                    });
                    if let Some(cy) = top {
                        canopy_tops[Self::column(size, cx, cz)] = base.y + (cy as i64 + 1) * cell;
                    }
                }
            }
        }
        Self {
            ground_heights,
            canopy_tops,
            forest,
            boxes,
            ..Self::new(size, cells, tops, base.y, cell)
        }
    }

    // From full-detail regions, with real trees.
    pub(super) fn sample_regions(regions: &RegionStore, key: LodKey) -> Self {
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

                tops[Self::column(size, cx, cz)] = ground_top(column_height);
                fill_column(
                    &mut cells,
                    biomes,
                    (base, cell),
                    (cx, cz),
                    biome,
                    (column_height, combined_gradient, temperature),
                );
            }
        }

        let span = (size + 1) as i64 * cell;
        let mut trees = Vec::new();
        if trees_may_reach(base.y - cell, base.y + span, lowest, highest) {
            let min = I64Vec2::new(base.x, base.z) - cell;
            trees = trees_near(regions.noise(), &mut heights, min, min + span + cell);
            for tree in &trees {
                tree.plant(
                    base,
                    cell,
                    (IVec3::splat(-1), IVec3::splat(size)),
                    |coord, block| {
                        let cell = &mut cells[Self::index(size, coord)];
                        if grows_into(*cell, block) {
                            *cell = block;
                        }
                    },
                );
            }
        }

        Self {
            ground_heights,
            trees: TreeTints::new(&trees, base, cell),
            ..Self::new(size, cells, tops, base.y, cell)
        }
    }

    pub(super) fn new(
        size: i32,
        cells: Vec<BlockType>,
        tops: Vec<i64>,
        base_y: i64,
        cell: i64,
    ) -> Self {
        let mut blocks = Self {
            size,
            cells,
            tops,
            base_y,
            cell,
            surface_tops: Vec::new(),
            any_surface_tops: false,
            canopy_tops: Vec::new(),
            ground_heights: Vec::new(),
            forest: Vec::new(),
            boxes: Vec::new(),
            trees: TreeTints::default(),
        };
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
    pub(super) fn ground_tops_out_in(&self, coord: IVec3) -> bool {
        let top = self.tops[Self::column(self.size, coord.x, coord.z)];
        let bottom = self.base_y + coord.y as i64 * self.cell;
        bottom < top && top < bottom + self.cell && is_solid(self.raw(coord))
    }

    pub(super) fn surface_top(&self, coord: IVec3) -> Option<f32> {
        let coord = coord.clamp(IVec3::splat(-1), IVec3::splat(self.size));
        self.surface_tops[Self::index(self.size, coord)]
    }

    pub(super) fn column(size: i32, cx: i32, cz: i32) -> usize {
        let padded = size + 2;
        ((cx + 1) * padded + cz + 1) as usize
    }

    // The cell holding the ground's surface is solid however little of it the ground fills,
    // and its top drawn at the ground's real height instead, so slopes step a block at a
    // time however coarse the cells. Only under open air: under water or leaves, and beside
    // water, the whole cell is drawn, since they fill the cell above from its bottom.
    pub(super) fn find_surface_top(&self, coord: IVec3) -> Option<f32> {
        let top = self.tops[Self::column(self.size, coord.x, coord.z)];
        let bottom = self.base_y + coord.y as i64 * self.cell;
        if !(bottom < top && top < bottom + self.cell) {
            return None;
        }
        let ground = !matches!(
            self.raw(coord),
            BlockType::Air | BlockType::Water | BlockType::Log
        );
        let beside_water = [IVec3::X, IVec3::NEG_X, IVec3::Z, IVec3::NEG_Z]
            .iter()
            .any(|&side| self.raw(coord + side) == BlockType::Water);
        if !ground || beside_water || self.raw(coord + IVec3::Y) != BlockType::Air {
            return None;
        }
        Some((top - self.base_y) as f32 / self.cell as f32 - 0.5)
    }

    pub(super) fn index(size: i32, coord: IVec3) -> usize {
        let padded = (size + 2) as usize;
        let c = (coord + IVec3::ONE).as_uvec3();
        (c.x as usize * padded + c.z as usize) * padded + c.y as usize
    }

    pub(super) fn raw(&self, coord: IVec3) -> BlockType {
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

    fn leaf_tint(&self, coord: IVec3) -> Vec3 {
        self.trees.at(coord)
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
        let column = Self::column(
            self.size,
            coord.x.clamp(-1, self.size),
            coord.z.clamp(-1, self.size),
        );
        let ground = self
            .ground_heights
            .get(column)
            .copied()
            .unwrap_or(self.tops[column] as f32);
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
            let inner = IVec3::new(
                coord.x.clamp(0, self.size - 1),
                coord.y,
                coord.z.clamp(0, self.size - 1),
            );
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
            if is_solid(self.raw(inner))
                && bottom + self.cell > ground - SKIRT_CELLS as i64 * self.cell
            {
                return BlockType::Air;
            }
        }
        // full detail can have the ground a block below a surface cell in the node above, so lid it
        if coord.y == self.size
            && !outside(coord.x)
            && !outside(coord.z)
            && self.ground_tops_out_in(coord)
        {
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
        SkyLight::new(
            IVec3::splat(-1),
            IVec3::splat(blocks.size + 2),
            sky_above(top, cell_size),
            opacity,
        )
    });
    let textured = cell_size <= TEXTURE_MAX_CELL_SIZE;
    // untextured leaves are drawn solid, and textured ones are all past `INNER_LEAVES_REACH`, so
    // have their holes filled: both hide the leaves behind them
    let meshes = mesh_blocks(
        &blocks,
        cell_size,
        MeshStyle {
            ambient_occlusion: true,
            textured,
            inner_leaves: InnerLeaves::Hidden,
        },
        &tint,
        &light,
    );
    meshes.has_geometry().then_some(meshes)
}

// Fills the cells up column (`cx`, `cz`) of the node with its lowest block and cell size at
// `base`, each sampled at its middle, with ground `height` high on its `gradient` at `temperature`.
// The cell the ground tops out in is solid however little of it the ground fills.
fn fill_column(
    cells: &mut [BlockType],
    biomes: &Biomes,
    (base, cell): (I64Vec3, i64),
    (cx, cz): (i32, i32),
    biome: BiomeId,
    (height, gradient, temperature): (i64, f64, f32),
) {
    let size = CHUNK_SIZE as i32;
    let block_at = |y| column_block(biomes, biome, y, height, gradient, temperature);
    let top = ground_top(height);
    for cy in -1..=size {
        let bottom = base.y + cy as i64 * cell;
        let middle = bottom + cell / 2;
        let mut block = under_grass(block_at(middle), middle, height, cell);
        block = freeze_cell(block, temperature, middle + cell / 2, cell);
        if block == BlockType::Air && bottom < top {
            block = block_at(top - 1);
        }
        cells[LodBlocks::index(size, IVec3::new(cx, cy, cz))] =
            underwater(block, bottom, cell, height);
    }
}
