use super::terrain::resolve_block;
use crate::block::BlockType;
use crate::chunks::chunk::ChunkData;
use bevy::math::IVec3;
use bevy::math::Vec3;
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
}

// Which of a tree box's faces can be seen.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(super) enum BoxFaces {
    All,
    // a box standing on the ground
    NoBottom,
    // a trunk, standing on the ground with its top in the crown
    Sides,
}

pub(super) struct ChunkNeighbourhood<'a> {
    pub(super) chunk: &'a ChunkData,
    pub(super) neighbors: &'a [Option<Arc<ChunkData>>],
}

impl BlockSource for ChunkNeighbourhood<'_> {
    fn block(&self, coord: IVec3) -> BlockType {
        resolve_block(self.chunk, self.neighbors, coord)
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
