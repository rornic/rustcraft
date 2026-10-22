use super::shading::{spread_water_levels, WATER_SPREAD_REACH};
use crate::chunk::{neighbor_26_index, ChunkData, CELLS, CHUNK_SIZE};
use crate::generate::region_store::RegionStore;
use bevy::math::IVec3;
use bevy::math::Vec3;
use ochre_base::block::BlockType;
use ochre_base::terrain_source::SpringPart;
use ochre_base::WATER_LEVEL;
use std::cell::OnceCell;
use std::sync::Arc;

pub(super) trait BlockSource {
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

    /// Where the surface of the water really is in a LOD cell it reaches only part of the way up,
    /// in mesh units from the cell's centre, so shallow water isn't drawn up to the cell's top.
    fn water_surface_top(&self, _coord: IVec3) -> Option<f32> {
        None
    }

    /// The surface of the fresh water in the column at `x`, `z`, in blocks from the lowest block of the
    /// chunk, where it isn't a whole block: a river's runs down its course.
    fn water_level(&self, _x: i32, _z: i32) -> Option<f32> {
        None
    }

    /// The fresh water's real surface, which its blocks are filled to, where `water_level` is spread out.
    fn water_block_level(&self, _x: i32, _z: i32) -> Option<f32> {
        None
    }

    /// Whether the column is in the pool of a spring a river rises in.
    fn spring_pool(&self, _x: i32, _z: i32) -> bool {
        false
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

    /// The tint of the tree whose leaves are at `coord`, see `Tree::tint`.
    fn leaf_tint(&self, _coord: IVec3) -> Vec3 {
        Vec3::ONE
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
pub(super) struct TreeBox {
    pub(super) min: Vec3,
    pub(super) max: Vec3,
    pub(super) block: BlockType,
    // over the grass's tint, see `Tree::tint`
    pub(super) tint: Vec3,
    pub(super) faces: BoxFaces,
    // how much of its width the top is drawn in by, 0 for a box, so a crown isn't a cube
    pub(super) taper: f32,
}

// Which of a tree box's faces can be seen.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(super) enum BoxFaces {
    All,
    // a trunk, standing on the ground with its top in the crown
    Sides,
}

// Blocks across a chunk's 3x3x3 neighbourhood, each way.
const SPAN: i32 = 3 * CHUNK_SIZE as i32;

/// A chunk with its 26 neighbours decoded into one dense grid, so meshing reads any block near it
/// without scanning a chunk's runs. Missing neighbours are air.
// A column's fresh water, in blocks from the chunk's lowest: its real surface the blocks fill to, its spread one, and a spring's pool.
#[derive(Clone, Copy)]
struct FreshColumn {
    block_level: f32,
    level: f32,
    pool: bool,
}

pub(super) struct ChunkNeighbourhood<'a> {
    pub(super) chunk: &'a ChunkData,
    blocks: Vec<BlockType>,
    water: Option<(&'a RegionStore, IVec3)>,
    levels: OnceCell<Vec<Option<FreshColumn>>>,
}

impl<'a> ChunkNeighbourhood<'a> {
    /// `neighbours` in `neighbor_26_index` order.
    pub(super) fn new(chunk: &'a ChunkData, neighbours: &[Option<Arc<ChunkData>>]) -> Self {
        let size = CHUNK_SIZE as i32;
        let mut blocks = vec![BlockType::Air; (SPAN * SPAN * SPAN) as usize];
        let mut cells = [BlockType::Air; CELLS];
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    let data = match (dx, dy, dz) {
                        (0, 0, 0) => chunk,
                        _ => match &neighbours[neighbor_26_index(dx, dy, dz)] {
                            Some(data) if !data.empty() => data,
                            _ => continue,
                        },
                    };
                    data.decode_into(&mut cells);
                    // both grids run y innermost, so each column copies across whole
                    for x in 0..size {
                        for z in 0..size {
                            let from = ((x * size + z) * size) as usize;
                            let to =
                                Self::index(IVec3::new(dx * size + x, dy * size, dz * size + z));
                            blocks[to..to + size as usize]
                                .copy_from_slice(&cells[from..from + size as usize]);
                        }
                    }
                }
            }
        }
        Self {
            chunk,
            blocks,
            water: None,
            levels: OnceCell::new(),
        }
    }

    /// Reads fresh water's surface from `regions` for the chunk whose lowest block is at `origin`.
    pub(super) fn with_water(mut self, regions: &'a RegionStore, origin: IVec3) -> Self {
        self.water = Some((regions, origin));
        self
    }

    // `coord` from the chunk's lowest block, -CHUNK_SIZE..2 * CHUNK_SIZE each way.
    fn index(coord: IVec3) -> usize {
        let c = coord + CHUNK_SIZE as i32;
        ((c.x * SPAN + c.z) * SPAN + c.y) as usize
    }

    fn contains(coord: IVec3) -> bool {
        let size = CHUNK_SIZE as i32;
        coord.cmpge(IVec3::splat(-size)).all() && coord.cmplt(IVec3::splat(2 * size)).all()
    }
    // The fresh water in the column at `x`, `z`, if it has any.
    fn fresh_at(&self, x: i32, z: i32) -> Option<FreshColumn> {
        let (regions, origin) = self.water?;
        let size = CHUNK_SIZE as i32;
        let span = size + 2;
        if !(-1..=size).contains(&x) || !(-1..=size).contains(&z) {
            return None;
        }
        // the sea is flat and over the chunks that can reach fresh water only
        if (origin.y + size) as i64 <= WATER_LEVEL + 1 {
            return None;
        }
        let levels = self.levels.get_or_init(|| {
            let pad = 1 + WATER_SPREAD_REACH;
            let wide = size + 2 * pad;
            let mut sampler = regions.sampler();
            let raw: Vec<f32> = (0..wide * wide)
                .map(|i| {
                    let (wx, wz) = (origin.x + i / wide - pad, origin.z + i % wide - pad);
                    sampler
                        .fresh_water(wx as i64, wz as i64)
                        .map_or(f32::NAN, |surface| surface - origin.y as f32)
                })
                .collect();
            let spread = spread_water_levels(&raw, wide);
            (0..span * span)
                .map(|i| {
                    let (x, z) = (i / span, i % span);
                    let at = ((x + WATER_SPREAD_REACH) * wide + z + WATER_SPREAD_REACH) as usize;
                    (!raw[at].is_nan()).then(|| FreshColumn {
                        block_level: raw[at],
                        level: spread[i as usize],
                        pool: sampler.spring((origin.x + x - 1) as i64, (origin.z + z - 1) as i64)
                            == SpringPart::Pool,
                    })
                })
                .collect()
        });
        levels[((x + 1) * span + z + 1) as usize]
    }
}

impl BlockSource for ChunkNeighbourhood<'_> {
    fn water_level(&self, x: i32, z: i32) -> Option<f32> {
        self.fresh_at(x, z).map(|fresh| fresh.level)
    }

    fn water_block_level(&self, x: i32, z: i32) -> Option<f32> {
        self.fresh_at(x, z).map(|fresh| fresh.block_level)
    }

    fn spring_pool(&self, x: i32, z: i32) -> bool {
        self.fresh_at(x, z).is_some_and(|fresh| fresh.pool)
    }

    fn block(&self, coord: IVec3) -> BlockType {
        if Self::contains(coord) {
            self.blocks[Self::index(coord)]
        } else {
            BlockType::Air
        }
    }

    fn leaf_tint(&self, coord: IVec3) -> Vec3 {
        self.chunk.trees.at(coord)
    }

    fn non_air(&self) -> Vec<(IVec3, BlockType)> {
        self.chunk
            .iter_non_air()
            .map(|(c, block)| (c.as_ivec3(), block))
            .collect()
    }
}

// Counts contiguous Water blocks straight down from `block_coord`, within this
// chunk only (so it's a cheap, purely-local lookup the async mesh task can do
// without `World` access - see the plan's note on why depth doesn't cross chunk
// boundaries). Used to scale wave amplitude: open water ripples, a shallow puddle
// at a chunk's edge stays calm.
pub(super) fn water_depth(source: &impl BlockSource, block_coord: IVec3) -> u32 {
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

pub(super) fn grid_index(size: i32, coord: IVec3) -> usize {
    ((coord.x * size + coord.y) * size + coord.z) as usize
}

// Reads blocks inside the chunk from a dense grid, which is much cheaper than `source`, and the rest from `source`.
pub(super) struct GridView<'a, S: BlockSource> {
    pub(super) size: i32,
    pub(super) blocks: &'a [BlockType],
    pub(super) source: &'a S,
}

impl<S: BlockSource> GridView<'_, S> {
    pub(super) fn contains(&self, coord: IVec3) -> bool {
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

    fn leaf_tint(&self, coord: IVec3) -> Vec3 {
        self.source.leaf_tint(coord)
    }

    fn tree_boxes(&self) -> &[TreeBox] {
        self.source.tree_boxes()
    }

    fn has_skirts(&self) -> bool {
        self.source.has_skirts()
    }
}
