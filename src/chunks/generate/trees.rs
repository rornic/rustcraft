use bevy::math::{DVec3, I64Vec2, I64Vec3, IVec3};

use super::generator::column_block;
use super::noise::NoiseGenerator;
use super::region_store::HeightSampler;
use crate::block::BlockType;

// Trees stand on a grid of cells this many blocks across, at most one somewhere in each cell.
const CELL: i64 = 5;
/// How far out from its trunk any tree reaches.
pub const TREE_REACH: i64 = 3;
/// How far above its ground any tree reaches.
pub const TREE_HEIGHT: i64 = 12;
// Colder than this, trees are pines.
const PINE_TEMPERATURE: f32 = 0.38;
// Ground steeper than this (see `terrain_block`) is too steep to take root on.
const MAX_GRADIENT: f64 = 1.5;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum TreeKind {
    // a round crown of leaves on a short trunk
    Broadleaf,
    // tiers of leaves narrowing up a tall trunk
    Pine,
}

#[derive(Debug, Copy, Clone)]
pub struct Tree {
    pub kind: TreeKind,
    // the trunk's lowest log
    pub base: I64Vec3,
    // logs in the trunk
    pub trunk: i64,
    // roughens the edge of the leaves
    seed: u64,
}

impl Tree {
    // The lowest and highest block the tree can fill.
    pub fn bounds(&self) -> (I64Vec3, I64Vec3) {
        let reach = I64Vec3::new(TREE_REACH, 0, TREE_REACH);
        (self.base - reach, self.base + reach + I64Vec3::Y * (self.trunk + 1))
    }

    // The tree's block in the cell `size` blocks across with `min` its lowest block, if any:
    // leaves if its centre is among them, a log if the trunk runs through it. Cells one block
    // across are just blocks, where the trunk runs up through the leaves; in coarser cells the
    // leaves win, or a whole cell's width of trunk would stand in for the crown.
    pub fn block_in_cell(&self, min: I64Vec3, size: i64) -> Option<BlockType> {
        let spans = |lo: i64, v: i64| (lo..lo + size).contains(&v);
        let trunk = spans(min.x, self.base.x)
            && spans(min.z, self.base.z)
            && min.y < self.base.y + self.trunk
            && min.y + size > self.base.y;
        let centre = min.as_dvec3() + (size - 1) as f64 / 2.0;
        let leaves = self.in_leaves(centre - self.base.as_dvec3());
        match (trunk, leaves) {
            (true, true) if size > 1 => Some(BlockType::Leaves),
            (true, _) => Some(BlockType::Log),
            (false, true) => Some(BlockType::Leaves),
            (false, false) => None,
        }
    }

    // Whether the point `offset` from the trunk's base is among the leaves.
    fn in_leaves(&self, offset: DVec3) -> bool {
        let across = offset.x * offset.x + offset.z * offset.z;
        match self.kind {
            TreeKind::Broadleaf => {
                // a crown around the top of the trunk, nibbled at its edge
                let rounded = offset.round().as_i64vec3();
                let nibble = if hash(self.seed, rounded.x, rounded.y * 31 + rounded.z) % 4 == 0 { 0.5 } else { 0.0 };
                let (radius, half_height) = (2.7 - nibble, 2.2);
                let up = offset.y - (self.trunk - 1) as f64;
                across / (radius * radius) + up * up / (half_height * half_height) <= 1.0
            }
            TreeKind::Pine => {
                let (bottom, top) = (2.0, (self.trunk + 1) as f64);
                if !(bottom..=top).contains(&offset.y) {
                    return false;
                }
                let t = (offset.y - bottom) / (top - bottom);
                // every other layer drawn in, so the leaves hang in tiers
                let tier = if (offset.y.round() as i64) % 2 == 1 { 0.7 } else { 1.0 };
                let radius = (0.5 + 2.4 * (1.0 - t)) * tier;
                across <= radius * radius
            }
        }
    }

    // Puts the tree into a grid of `size`-block cells whose cell 0 has `origin` as its lowest
    // block, calling `place` with each cell index in `first..=last` that the tree fills.
    pub fn plant(
        &self,
        origin: I64Vec3,
        size: i64,
        (first, last): (IVec3, IVec3),
        mut place: impl FnMut(IVec3, BlockType),
    ) {
        let (low, high) = self.bounds();
        let cell_of = |block: I64Vec3| (block - origin).div_euclid(I64Vec3::splat(size)).as_ivec3();
        let (low, high) = (cell_of(low).max(first), cell_of(high).min(last));
        for x in low.x..=high.x {
            for z in low.z..=high.z {
                for y in low.y..=high.y {
                    let cell = IVec3::new(x, y, z);
                    if let Some(block) = self.block_in_cell(origin + cell.as_i64vec3() * size, size) {
                        place(cell, block);
                    }
                }
            }
        }
    }
}

// Whether a tree's `block` may go where `existing` is: into air, and logs through leaves.
pub fn grows_into(existing: BlockType, block: BlockType) -> bool {
    existing == BlockType::Air || (existing == BlockType::Leaves && block == BlockType::Log)
}

// Every tree that could reach into the columns `min..=max` (x, z).
pub fn trees_near(noise: &NoiseGenerator, heights: &mut HeightSampler, min: I64Vec2, max: I64Vec2) -> Vec<Tree> {
    let first = (min - TREE_REACH).div_euclid(I64Vec2::splat(CELL));
    let last = (max + TREE_REACH).div_euclid(I64Vec2::splat(CELL));
    let mut trees = Vec::new();
    for cz in first.y..=last.y {
        for cx in first.x..=last.x {
            if let Some(tree) = tree_in_cell(noise, heights, cx, cz) {
                trees.push(tree);
            }
        }
    }
    trees
}

fn tree_in_cell(noise: &NoiseGenerator, heights: &mut HeightSampler, cx: i64, cz: i64) -> Option<Tree> {
    let seed = hash(noise.seed() as u64, cx, cz);
    let x = cx * CELL + ((seed >> 16) % CELL as u64) as i64;
    let z = cz * CELL + ((seed >> 24) % CELL as u64) as i64;
    let chance = (seed & 0xffff) as f64 / 65536.0;
    if chance >= heights.trees(x, z) {
        return None;
    }

    let height = heights.height(x, z).round() as i64;
    let (gradient_x, gradient_z) = heights.gradient(x, z);
    let gradient = gradient_x.abs() + gradient_z.abs();
    if gradient > MAX_GRADIENT {
        return None;
    }
    let ground = column_block(noise.biomes(), heights.biome(x, z), height - 1, height, gradient);
    if ground != BlockType::Grass {
        return None;
    }

    let size = (seed >> 32) % 4;
    let (kind, trunk) = if noise.climate(x as f64, z as f64).temperature < PINE_TEMPERATURE {
        (TreeKind::Pine, 7 + size as i64)
    } else {
        (TreeKind::Broadleaf, 5 + (size % 3) as i64)
    };
    Some(Tree {
        kind,
        base: I64Vec3::new(x, height, z),
        trunk,
        seed,
    })
}

// splitmix64 over a seed and two coordinates
fn hash(seed: u64, a: i64, b: i64) -> u64 {
    let mut h = seed ^ (a as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (b as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h = (h ^ (h >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    h ^ (h >> 31)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::chunks::generate::region_store::RegionStore;

    fn tree(kind: TreeKind, trunk: i64) -> Tree {
        Tree {
            kind,
            base: I64Vec3::new(100, 500, -40),
            trunk,
            seed: 7,
        }
    }

    fn blocks(tree: &Tree) -> Vec<(I64Vec3, BlockType)> {
        let (low, high) = tree.bounds();
        let mut blocks = Vec::new();
        tree.plant(low, 1, (IVec3::ZERO, (high - low).as_ivec3()), |cell, block| {
            blocks.push((low + cell.as_i64vec3(), block));
        });
        blocks
    }

    #[test]
    fn test_trees_are_a_trunk_under_leaves_within_bounds() {
        for t in [tree(TreeKind::Broadleaf, 5), tree(TreeKind::Broadleaf, 7), tree(TreeKind::Pine, 10)] {
            let blocks = blocks(&t);
            let logs: Vec<_> = blocks.iter().filter(|(_, b)| *b == BlockType::Log).collect();
            assert_eq!(t.trunk as usize, logs.len());
            assert!(logs.iter().all(|(p, _)| p.x == t.base.x && p.z == t.base.z));
            let leaves = blocks.iter().filter(|(_, b)| *b == BlockType::Leaves).count();
            assert!(leaves > 20, "{:?} has only {leaves} leaves", t.kind);
            // the trunk's top is hidden in the leaves
            let above_trunk = t.base + I64Vec3::Y * t.trunk;
            assert_eq!(Some(BlockType::Leaves), t.block_in_cell(above_trunk, 1));
            assert!(t.base.y + t.trunk + 1 - t.base.y <= TREE_HEIGHT);
        }
    }

    #[test]
    fn test_coarse_cells_keep_the_trunk_and_crown() {
        let t = tree(TreeKind::Broadleaf, 6);
        // 4-block cells aligned so the trunk's column isn't at a cell's centre
        let origin = I64Vec3::new(96, 496, -44);
        let mut seen = Vec::new();
        t.plant(origin, 4, (IVec3::splat(-2), IVec3::splat(6)), |_, block| seen.push(block));
        assert!(seen.contains(&BlockType::Log));
        assert!(seen.contains(&BlockType::Leaves));
    }

    #[test]
    fn test_logs_grow_through_leaves_but_nothing_into_ground() {
        assert!(grows_into(BlockType::Air, BlockType::Leaves));
        assert!(grows_into(BlockType::Leaves, BlockType::Log));
        assert!(!grows_into(BlockType::Log, BlockType::Leaves));
        assert!(!grows_into(BlockType::Grass, BlockType::Log));
        assert!(!grows_into(BlockType::Water, BlockType::Leaves));
    }

    #[test]
    fn test_trees_stand_on_grass_and_agree_between_lookups() {
        let noise = Arc::new(NoiseGenerator::new(5));
        let regions = RegionStore::new(noise.clone());
        let mut heights = regions.sampler();
        let (min, max) = (I64Vec2::new(-600, -600), I64Vec2::new(600, 600));
        let trees = trees_near(&noise, &mut heights, min, max);
        assert!(!trees.is_empty(), "no trees in {min}..{max}");
        for t in &trees {
            let ground = heights.height(t.base.x, t.base.z).round() as i64;
            assert_eq!(ground, t.base.y);
        }
        // a smaller window finds the same trees where they overlap
        let some = trees_near(&noise, &mut heights, I64Vec2::new(0, 0), I64Vec2::new(40, 40));
        for t in &some {
            assert!(trees.iter().any(|o| o.base == t.base && o.trunk == t.trunk && o.kind == t.kind));
        }
    }
}
