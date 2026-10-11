use bevy::math::{DVec3, I64Vec2, I64Vec3, IVec3};

use super::generator::column_block;
use super::noise::NoiseGenerator;
use super::region_store::HeightSampler;
use crate::block::BlockType;

// Trees stand on a grid of cells this many blocks across, at most one somewhere in each cell.
const CELL: i64 = 10;
/// How far out from its trunk any tree reaches.
pub const TREE_REACH: i64 = 10;
/// How far above its ground any tree reaches.
pub const TREE_HEIGHT: i64 = 27;
// Colder than this, trees are pines.
const PINE_TEMPERATURE: f32 = 0.38;
// Ground steeper than this (see `terrain_block`) is too steep to take root on.
const MAX_GRADIENT: f64 = 1.5;
// Broadleaf crowns are a big round crown with smaller lobes hanging off it on branches.
const LOBES: usize = 3;
// Trunks this tall are two blocks thick.
const THICK_TRUNK: i64 = 12;
// How far in leaves are nibbled from the edge of a crown.
const NIBBLE: f64 = 0.9;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum TreeKind {
    // a broad, lumpy crown on a stout trunk
    Broadleaf,
    // tiers of leaves narrowing up a tall trunk
    Pine,
}

#[derive(Debug, Copy, Clone)]
pub struct Tree {
    pub kind: TreeKind,
    // the trunk's lowest log, at its -x, -z corner when it's thick
    pub base: I64Vec3,
    // logs up the trunk
    pub trunk: i64,
    // how far the leaves spread from the trunk
    spread: f64,
    // roughens the edge of the leaves
    seed: u64,
    // the broadleaf crown, and smaller lobes around it each held up by a branch
    crown: Blob,
    lobes: [Blob; LOBES],
}

// A ball of leaves, squashed by `squash` vertically.
#[derive(Debug, Copy, Clone)]
struct Blob {
    centre: DVec3,
    radius: f64,
    squash: f64,
}

impl Blob {
    // Whether `point` is inside the blob with its edge drawn in by `nibble` there.
    fn contains(&self, point: DVec3, nibble: impl Fn() -> f64) -> bool {
        let d = (point - self.centre) / DVec3::new(1.0, self.squash, 1.0);
        let distance = d.length_squared();
        if distance > self.radius * self.radius {
            return false;
        }
        let inner = (self.radius - NIBBLE).max(0.0);
        // only the rim gets nibbled, so most points skip working it out
        distance <= inner * inner || distance <= (self.radius - nibble()).powi(2)
    }
}

impl Tree {
    fn new(kind: TreeKind, base: I64Vec3, trunk: i64, spread: f64, seed: u64) -> Self {
        let width = if trunk >= THICK_TRUNK { 2 } else { 1 };
        let middle = (width - 1) as f64 / 2.0;
        let crown = Blob {
            centre: DVec3::new(middle, (trunk - 1) as f64, middle),
            radius: spread,
            squash: 0.7,
        };
        let lobes = std::array::from_fn(|i| {
            let h = hash(seed, i as i64, 17);
            let angle = (i as f64 + (h % 1000) as f64 / 1000.0 * 0.6) * std::f64::consts::TAU / LOBES as f64;
            let out = spread * 0.75;
            let drop = spread * (0.25 + ((h >> 10) % 100) as f64 / 400.0);
            Blob {
                centre: crown.centre + DVec3::new(angle.cos() * out, -drop, angle.sin() * out),
                radius: spread * 0.6,
                squash: 0.75,
            }
        });
        Self { kind, base, trunk, spread, seed, crown, lobes }
    }

    fn width(&self) -> i64 {
        if self.trunk >= THICK_TRUNK { 2 } else { 1 }
    }

    // the middle of the trunk, as an offset from `base`
    fn axis(&self) -> DVec3 {
        let middle = (self.width() - 1) as f64 / 2.0;
        DVec3::new(middle, 0.0, middle)
    }

    // The lowest and highest block the tree can fill.
    pub fn bounds(&self) -> (I64Vec3, I64Vec3) {
        let reach = I64Vec3::new(TREE_REACH, 0, TREE_REACH);
        let top = I64Vec3::Y * TREE_HEIGHT;
        (self.base - reach, self.base + reach + top + I64Vec3::new(1, 0, 1))
    }

    // The tree's block in the cell `size` blocks across with `min` its lowest block, if any:
    // leaves if its centre is among them, a log if the trunk or a branch runs through it. Cells
    // one block across are just blocks, where wood runs up through the leaves; in coarser cells
    // the leaves win, or a whole cell's width of trunk would stand in for the crown.
    pub fn block_in_cell(&self, min: I64Vec3, size: i64) -> Option<BlockType> {
        let overlaps = |lo: i64, from: i64, to: i64| lo < to && lo + size > from;
        let width = self.width();
        let trunk = overlaps(min.x, self.base.x, self.base.x + width)
            && overlaps(min.z, self.base.z, self.base.z + width)
            && overlaps(min.y, self.base.y, self.base.y + self.trunk);
        let offset = min.as_dvec3() + (size - 1) as f64 / 2.0 - self.base.as_dvec3();
        let leaves = self.in_leaves(offset);
        match (trunk, leaves) {
            (true, true) if size > 1 => Some(BlockType::Leaves),
            (true, _) => Some(BlockType::Log),
            (false, _) if size == 1 && self.in_branch(offset) => Some(BlockType::Log),
            (false, true) => Some(BlockType::Leaves),
            (false, false) => None,
        }
    }

    // Whether the point `offset` from the trunk's base is among the leaves.
    fn in_leaves(&self, offset: DVec3) -> bool {
        // nibbled at the edges, so crowns aren't perfect balls
        let nibble = || {
            let rounded = offset.round().as_i64vec3();
            if hash(self.seed, rounded.x * 7919 + rounded.y, rounded.z) % 3 == 0 { NIBBLE } else { 0.0 }
        };
        match self.kind {
            TreeKind::Broadleaf => {
                self.crown.contains(offset, nibble) || self.lobes.iter().any(|lobe| lobe.contains(offset, nibble))
            }
            TreeKind::Pine => {
                let (bottom, top) = ((self.trunk / 4) as f64, (self.trunk + 1) as f64);
                if !(bottom..=top).contains(&offset.y) {
                    return false;
                }
                let t = (offset.y - bottom) / (top - bottom);
                // each tier is widest at its foot and drawn in towards the next
                let tier = 1.0 - 0.3 * ((offset.y.round() as i64).rem_euclid(3) as f64 / 2.0);
                // never too narrow to wrap the trunk, which runs up to the tip
                let wrap = self.width() as f64 / 2.0 + 1.0;
                let radius = ((0.6 + self.spread * (1.0 - t)) * tier - nibble() * 0.5).max(wrap);
                let across = offset - self.axis();
                across.x * across.x + across.z * across.z <= radius * radius
            }
        }
    }

    // Whether the point `offset` from the trunk's base is on a branch out to a lobe.
    fn in_branch(&self, offset: DVec3) -> bool {
        if self.kind != TreeKind::Broadleaf {
            return false;
        }
        let fork = self.axis() + DVec3::Y * (self.trunk as f64 * 0.6);
        self.lobes.iter().any(|lobe| {
            let along = lobe.centre - fork;
            let t = ((offset - fork).dot(along) / along.length_squared()).clamp(0.0, 1.0);
            (offset - (fork + along * t)).length_squared() <= 0.36
        })
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

    // 0..1, how big this one grew
    let growth = ((seed >> 32) % 1000) as f64 / 1000.0;
    let (kind, trunk, spread) = if noise.climate(x as f64, z as f64).temperature < PINE_TEMPERATURE {
        (TreeKind::Pine, 16 + (growth * 9.0) as i64, 4.5 + growth * 1.5)
    } else {
        (TreeKind::Broadleaf, 10 + (growth * 6.0) as i64, 5.0 + growth * 2.0)
    };
    Some(Tree::new(kind, I64Vec3::new(x, height, z), trunk, spread, seed))
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

    fn tree(kind: TreeKind, trunk: i64, spread: f64) -> Tree {
        Tree::new(kind, I64Vec3::new(100, 500, -40), trunk, spread, 7)
    }

    // the smallest and largest each kind grows
    fn extremes() -> [Tree; 4] {
        [
            tree(TreeKind::Broadleaf, 10, 5.0),
            tree(TreeKind::Broadleaf, 15, 7.0),
            tree(TreeKind::Pine, 16, 4.5),
            tree(TreeKind::Pine, 24, 6.0),
        ]
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
    fn test_trees_are_a_trunk_under_big_crowns() {
        for t in extremes() {
            let blocks = blocks(&t);
            let in_trunk = |p: I64Vec3| (0..t.width()).contains(&(p.x - t.base.x)) && (0..t.width()).contains(&(p.z - t.base.z));
            let trunk_logs = blocks.iter().filter(|(p, b)| *b == BlockType::Log && in_trunk(*p)).count();
            assert_eq!((t.trunk * t.width() * t.width()) as usize, trunk_logs);
            let branches = blocks.iter().filter(|(p, b)| *b == BlockType::Log && !in_trunk(*p)).count();
            assert_eq!(t.kind == TreeKind::Broadleaf, branches > 0, "{:?} has {branches} branch logs", t.kind);
            let leaves = blocks.iter().filter(|(_, b)| *b == BlockType::Leaves).count();
            assert!(leaves > 200, "{:?} has only {leaves} leaves", t.kind);
            // the trunk's top is hidden in the leaves, all the way round
            let top = t.base + I64Vec3::Y * (t.trunk - 1);
            assert_eq!(Some(BlockType::Leaves), t.block_in_cell(top + I64Vec3::Y, 1));
            for side in [I64Vec3::NEG_X, I64Vec3::NEG_Z, I64Vec3::X * t.width(), I64Vec3::Z * t.width()] {
                assert_eq!(Some(BlockType::Leaves), t.block_in_cell(top + side, 1), "{:?} trunk bare at {side}", t.kind);
            }
        }
    }

    #[test]
    fn test_trees_stay_within_their_reach() {
        for t in extremes() {
            let (low, high) = (t.base - I64Vec3::new(TREE_REACH, 0, TREE_REACH), t.base + I64Vec3::new(TREE_REACH, TREE_HEIGHT, TREE_REACH));
            // look well past the bounds, so anything poking out is caught
            for x in low.x - 4..=high.x + 4 {
                for z in low.z - 4..=high.z + 4 {
                    for y in low.y - 4..=high.y + 4 {
                        let p = I64Vec3::new(x, y, z);
                        if t.block_in_cell(p, 1).is_some() {
                            assert!(p.cmpge(low).all() && p.cmple(high).all(), "{:?} reaches {p}", t.kind);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn test_coarse_cells_keep_the_trunk_and_crown() {
        let t = tree(TreeKind::Broadleaf, 12, 6.0);
        // 4-block cells aligned so the trunk's column isn't at a cell's centre
        let origin = I64Vec3::new(96, 496, -44);
        let mut seen = Vec::new();
        t.plant(origin, 4, (IVec3::splat(-4), IVec3::splat(10)), |_, block| seen.push(block));
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

