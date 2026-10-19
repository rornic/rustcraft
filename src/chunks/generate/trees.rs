use bevy::math::{DVec3, I64Vec2, I64Vec3, IVec3, Vec2, Vec3};

use super::biome::BiomeId;
use super::generator::column_block;
use super::region_store::{HeightSampler, SLOPE_SPAN};
use super::terrain_source::TerrainSource;
use super::WATER_LEVEL;
use crate::block::BlockType;

/// What trees need to know about the ground to take root, from the full-detail heightmap or
/// from the coarser grid a distant LOD node samples.
pub trait TreeGround {
    fn trees(&mut self, x: i64, z: i64) -> f64;
    fn height(&mut self, x: i64, z: i64) -> f64;
    fn biome(&mut self, x: i64, z: i64) -> BiomeId;

    /// The slope there, as `HeightSampler::gradient` measures it.
    fn gradient(&mut self, x: i64, z: i64) -> (f64, f64) {
        let span = SLOPE_SPAN;
        let across = |a: f64, b: f64| (a - b) * (1.0 / span as f64);
        (
            across(self.height(x + span, z), self.height(x - span, z)),
            across(self.height(x, z + span), self.height(x, z - span)),
        )
    }

    /// Whether no tree with this `chance` can grow at `x`, `z`, known without `trees` there.
    fn rules_out(&mut self, _x: i64, _z: i64, _chance: f64) -> bool {
        false
    }

    /// The height a tree's base stands at, which a coarse node sets to the ground it draws, so
    /// trees sit on it, while the rest above stay exact, so the same trees grow at every level.
    fn base_height(&mut self, x: i64, z: i64) -> i64 {
        self.height(x, z).round() as i64
    }
}

impl TreeGround for HeightSampler<'_> {
    fn trees(&mut self, x: i64, z: i64) -> f64 {
        HeightSampler::trees(self, x, z)
    }

    fn height(&mut self, x: i64, z: i64) -> f64 {
        HeightSampler::height(self, x, z)
    }

    fn biome(&mut self, x: i64, z: i64) -> BiomeId {
        HeightSampler::biome(self, x, z)
    }
}

// Trees stand on a grid of cells this many blocks across, at most one somewhere in each cell.
const CELL: i64 = 10;
/// How far out from its trunk any tree reaches.
pub const TREE_REACH: i64 = 10;
/// How far above its ground any tree reaches.
pub const TREE_HEIGHT: i64 = 27;
// Colder than this, trees are pines.
// Trees growing where it is cooler than this, at their height, are pines: cold lowlands, and
// the slopes of any range higher up, where the air has cooled.
const PINE_TEMPERATURE: f32 = 0.46;
// Higher than this above the sea, trees are pines whatever the climate below.
const ALPINE_PINES_ABOVE: i64 = 120;
// Ground steeper than this (see `terrain_block`) is too steep to take root on.
const MAX_GRADIENT: f64 = 1.5;
// Broadleaf crowns are a big round crown with smaller lobes hanging off it on branches.
const LOBES: usize = 3;
// Trunks this tall are two blocks thick.
const THICK_TRUNK: i64 = 12;
// Cells this wide or wider judge leaves by coverage: a quarter of a 3 x 3 x 3 spread of points
// inside, or small crowns would fall between cell centres.
const COVERAGE_CELL: i64 = 4;
const COVERAGE_POINTS: usize = 7;
/// Cells this wide or wider leave out the trunk, which would stand a pillar far thicker than it
/// is. Meshing draws it as a thin box instead, see `generator::TreeBox`.
pub const THIN_TRUNK_CELL: i64 = 4;
// How far in leaves are nibbled from the edge of a crown.
const NIBBLE: f64 = 0.9;
// How far each tree's leaves stray from the grass's tint, so a forest isn't one flat green:
// brighter or darker by up to this much, and warmer or cooler by up to this much.
const TINT_BRIGHTNESS: f32 = 0.22;
const TINT_WARMTH: f32 = 0.18;

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
            let angle = (i as f64 + (h % 1000) as f64 / 1000.0 * 0.6) * std::f64::consts::TAU
                / LOBES as f64;
            let out = spread * 0.75;
            let drop = spread * (0.25 + ((h >> 10) % 100) as f64 / 400.0);
            Blob {
                centre: crown.centre + DVec3::new(angle.cos() * out, -drop, angle.sin() * out),
                radius: spread * 0.6,
                squash: 0.75,
            }
        });
        Self {
            kind,
            base,
            trunk,
            spread,
            seed,
            crown,
            lobes,
        }
    }

    /// The tree's own tint over the grass's, the same at every level of detail.
    pub fn tint(&self) -> Vec3 {
        let h = hash(self.seed, 31, 59);
        let unit = |shift: u32| ((h >> shift) & 0xffff) as f32 / 65535.0 * 2.0 - 1.0;
        let warm = Vec3::new(1.0, 0.35, -1.0) * TINT_WARMTH * unit(16);
        (1.0 + TINT_BRIGHTNESS * unit(0)) * (Vec3::ONE + warm)
    }

    /// Whether the crown is over the point `x`, `z` across the world, in blocks, seen from above.
    pub fn shades(&self, x: f64, z: f64) -> bool {
        // from the trunk's base, as `in_leaves` takes it, where a block's index is its middle
        let offset = DVec3::new(
            x - 0.5 - self.base.x as f64,
            0.0,
            z - 0.5 - self.base.z as f64,
        );
        let under = |blob: &Blob| {
            (offset - blob.centre.with_y(0.0)).length_squared() <= blob.radius * blob.radius
        };
        match self.kind {
            TreeKind::Broadleaf => under(&self.crown) || self.lobes.iter().any(under),
            // the lowest tier is the widest
            TreeKind::Pine => {
                (offset - self.axis()).length_squared() <= (0.6 + self.spread).powi(2)
            }
        }
    }

    /// The top of the crown, in blocks.
    pub fn crown_top(&self) -> f64 {
        let top = match self.kind {
            TreeKind::Broadleaf => self.crown.centre.y + self.crown.radius * self.crown.squash,
            TreeKind::Pine => (self.trunk + 1) as f64,
        };
        self.base.y as f64 + top + 1.0
    }

    pub fn width(&self) -> i64 {
        if self.trunk >= THICK_TRUNK {
            2
        } else {
            1
        }
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
        (
            self.base - reach,
            self.base + reach + top + I64Vec3::new(1, 0, 1),
        )
    }

    // The tree's block in the cell `size` blocks across with `min` its lowest block, if any:
    // leaves if its centre is among them, a log if the trunk or a branch runs through it. Cells
    // one block across are just blocks, where wood runs up through the leaves; in coarser cells
    // the leaves win, or a whole cell's width of trunk would stand in for the crown. Cells this
    // wide or wider hold leaves if a quarter of them would be, or small crowns would fall between
    // cell centres, and from `THIN_TRUNK_CELL` they leave out the trunk.
    pub fn block_in_cell(&self, min: I64Vec3, size: i64) -> Option<BlockType> {
        let overlaps = |lo: i64, from: i64, to: i64| lo < to && lo + size > from;
        let width = self.width();
        let trunk = overlaps(min.x, self.base.x, self.base.x + width)
            && overlaps(min.z, self.base.z, self.base.z + width)
            && overlaps(min.y, self.base.y, self.base.y + self.trunk);
        let offset = min.as_dvec3() + (size - 1) as f64 / 2.0 - self.base.as_dvec3();
        let leaves = if size >= COVERAGE_CELL {
            self.leaf_coverage(offset, size)
        } else {
            self.in_leaves(offset)
        };
        match (trunk, leaves) {
            (true, true) if size > 1 => Some(BlockType::Leaves),
            (true, _) if size >= THIN_TRUNK_CELL => None,
            (true, _) => Some(BlockType::Log),
            (false, _) if size == 1 && self.in_branch(offset) => Some(BlockType::Log),
            (false, true) => Some(BlockType::Leaves),
            (false, false) => None,
        }
    }

    // Whether enough of the cell centred `offset` from the trunk's base is among the leaves, judged
    // from a 3 x 3 x 3 spread of points through it.
    fn leaf_coverage(&self, offset: DVec3, size: i64) -> bool {
        let step = size as f64 / 3.0;
        let mut inside = 0;
        for x in -1..=1 {
            for y in -1..=1 {
                for z in -1..=1 {
                    let point = DVec3::new(x as f64, y as f64, z as f64) * step;
                    if self.in_leaves(offset + point) {
                        inside += 1;
                    }
                }
            }
        }
        inside >= COVERAGE_POINTS
    }

    /// The crown as `layers` stacked boxes, in blocks from `base`'s lower corner, for cells too
    /// wide to hold it: each layer is as wide as the leaves reach there, so a crown keeps its dome
    /// or tiers instead of being one cuboid.
    pub fn crown_boxes(&self, layers: i64) -> Vec<(DVec3, DVec3)> {
        let (low, high) = self.leaf_bounds();
        // only each row's first and last leaf, found from its ends, bear on the layers' bounds
        let mut leaves = Vec::new();
        let xs = low.x.max(-TREE_REACH)..=high.x.min(TREE_REACH);
        for y in low.y.max(0)..=high.y.min(TREE_HEIGHT) {
            for z in low.z.max(-TREE_REACH)..=high.z.min(TREE_REACH) {
                let leaf = |x: i64| self.in_leaves(DVec3::new(x as f64, y as f64, z as f64));
                if let Some(first) = xs.clone().find(|&x| leaf(x)) {
                    let last = xs.clone().rev().find(|&x| leaf(x)).unwrap_or(first);
                    leaves.extend([I64Vec3::new(first, y, z), I64Vec3::new(last, y, z)]);
                }
            }
        }
        let (Some(low), Some(high)) = (
            leaves.iter().map(|l| l.y).min(),
            leaves.iter().map(|l| l.y).max(),
        ) else {
            return Vec::new();
        };
        let height = high - low + 1;
        let layer = |y: i64| ((y - low) * layers / height) as usize;
        let mut bounds: Vec<Option<(I64Vec3, I64Vec3)>> = vec![None; layers as usize];
        for leaf in &leaves {
            let (min, max) = bounds[layer(leaf.y)].get_or_insert((*leaf, *leaf));
            *min = min.min(*leaf);
            *max = max.max(*leaf);
        }
        bounds
            .into_iter()
            .flatten()
            .map(|(min, max)| {
                (
                    (self.base + min).as_dvec3(),
                    (self.base + max + I64Vec3::ONE).as_dvec3(),
                )
            })
            .collect()
    }

    // Whole-block offsets from the trunk's base that every point `in_leaves` lies within.
    fn leaf_bounds(&self) -> (I64Vec3, I64Vec3) {
        let (low, high) = match self.kind {
            TreeKind::Broadleaf => std::iter::once(&self.crown)
                .chain(&self.lobes)
                .map(|blob| {
                    let reach = DVec3::new(blob.radius, blob.radius * blob.squash, blob.radius);
                    (blob.centre - reach, blob.centre + reach)
                })
                .fold((DVec3::MAX, DVec3::MIN), |(low, high), (min, max)| {
                    (low.min(min), high.max(max))
                }),
            TreeKind::Pine => {
                let wrap = self.width() as f64 / 2.0 + 1.0;
                let reach = (0.6 + self.spread).max(wrap);
                let axis = self.axis();
                (
                    DVec3::new(axis.x - reach, (self.trunk / 4) as f64, axis.z - reach),
                    DVec3::new(axis.x + reach, (self.trunk + 1) as f64, axis.z + reach),
                )
            }
        };
        (low.floor().as_i64vec3(), high.ceil().as_i64vec3())
    }

    // Whether the point `offset` from the trunk's base is among the leaves.
    fn in_leaves(&self, offset: DVec3) -> bool {
        // nibbled at the edges, so crowns aren't perfect balls
        let nibble = || {
            let rounded = offset.round().as_i64vec3();
            if hash(self.seed, rounded.x * 7919 + rounded.y, rounded.z).is_multiple_of(3) {
                NIBBLE
            } else {
                0.0
            }
        };
        match self.kind {
            TreeKind::Broadleaf => {
                self.crown.contains(offset, nibble)
                    || self.lobes.iter().any(|lobe| lobe.contains(offset, nibble))
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
        // the leaves, the trunk, and the branches running between them, within a block of each
        let (leaves_low, leaves_high) = self.leaf_bounds();
        let width = self.width();
        let low = self.base + leaves_low.min(I64Vec3::ZERO) - 1;
        let high = self.base + leaves_high.max(I64Vec3::new(width, self.trunk, width) - 1) + 1;
        let cell_of = |block: I64Vec3| (block - origin).div_euclid(I64Vec3::splat(size)).as_ivec3();
        let (low, high) = (cell_of(low).max(first), cell_of(high).min(last));
        for x in low.x..=high.x {
            for z in low.z..=high.z {
                for y in low.y..=high.y {
                    let cell = IVec3::new(x, y, z);
                    if let Some(block) = self.block_in_cell(origin + cell.as_i64vec3() * size, size)
                    {
                        place(cell, block);
                    }
                }
            }
        }
    }
}

/// Which tree's tint each leaf takes, for leaves placed block by block, where which tree put
/// them is lost: the one whose trunk is nearest across, which is the one they belong to
/// but where crowns meet.
#[derive(Default, Debug, Clone)]
pub struct TreeTints(Vec<(Vec2, Vec3)>);

impl TreeTints {
    /// Every tree in `trees`, placed by its trunk's axis less `origin`, in cells `cell` blocks
    /// wide whose centres are whole numbers.
    pub fn new<'a>(trees: impl IntoIterator<Item = &'a Tree>, origin: I64Vec3, cell: i64) -> Self {
        Self(
            trees
                .into_iter()
                .map(|t| {
                    let axis = (t.base - origin).as_dvec3() + t.axis();
                    (
                        Vec2::new(axis.x as f32, axis.z as f32) / cell as f32
                            - (cell - 1) as f32 / (2.0 * cell as f32),
                        t.tint(),
                    )
                })
                .collect(),
        )
    }

    /// The tint of the leaf at `coord`, in cells.
    pub fn at(&self, coord: IVec3) -> Vec3 {
        let p = Vec2::new(coord.x as f32, coord.z as f32);
        self.0
            .iter()
            .min_by(|a, b| a.0.distance_squared(p).total_cmp(&b.0.distance_squared(p)))
            .map_or(Vec3::ONE, |&(_, tint)| tint)
    }

    pub fn heap_size(&self) -> usize {
        self.0.capacity() * size_of::<(Vec2, Vec3)>()
    }
}

// Whether a tree's `block` may go where `existing` is: into air, and logs through leaves.
pub fn grows_into(existing: BlockType, block: BlockType) -> bool {
    existing == BlockType::Air || (existing == BlockType::Leaves && block == BlockType::Log)
}

// Every tree that could reach into the columns `min..=max` (x, z).
pub fn trees_near(
    noise: &dyn TerrainSource,
    heights: &mut impl TreeGround,
    min: I64Vec2,
    max: I64Vec2,
) -> Vec<Tree> {
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

fn tree_in_cell(
    noise: &dyn TerrainSource,
    heights: &mut impl TreeGround,
    cx: i64,
    cz: i64,
) -> Option<Tree> {
    let seed = hash(noise.seed() as u64, cx, cz);
    let x = cx * CELL + ((seed >> 16) % CELL as u64) as i64;
    let z = cz * CELL + ((seed >> 24) % CELL as u64) as i64;
    let chance = (seed & 0xffff) as f64 / 65536.0;
    if heights.rules_out(x, z, chance) || chance >= heights.trees(x, z) {
        return None;
    }

    // the biome before the slope, which samples elsewhere, while the ground here is to hand
    let height = heights.height(x, z).round() as i64;
    // a trunk is more than a column wide, and stands on ground level with its foot: not across a
    // step or a cliff's edge, where its far side would be buried or hang in the air
    for (dx, dz) in [(1, 0), (0, 1), (1, 1)] {
        let beside = heights.height(x + dx, z + dz).round() as i64;
        if !(height - 1..=height).contains(&beside) {
            return None;
        }
    }
    let biome = heights.biome(x, z);
    let (gradient_x, gradient_z) = heights.gradient(x, z);
    // meadow clings to a mountain's faces, and trees stand on it
    let gradient = super::generator::block_gradient(
        noise.biomes(),
        biome,
        noise.seed(),
        x,
        z,
        gradient_x.abs() + gradient_z.abs(),
    );
    if gradient > MAX_GRADIENT {
        return None;
    }
    let temperature = noise.temperature(x as f64, z as f64);
    let ground = column_block(
        noise.biomes(),
        biome,
        height - 1,
        height,
        gradient,
        temperature,
        WATER_LEVEL,
    );
    if ground != BlockType::Grass {
        return None;
    }

    // 0..1, how big this one grew
    let growth = ((seed >> 32) % 1000) as f64 / 1000.0;
    // how warm it is up where the tree stands, which falls with height
    let warmth = noise.climate(x as f64, z as f64).temperature;
    // pines in the cold, and on any range's upper slopes, even in the tropics
    let alpine = height - WATER_LEVEL >= ALPINE_PINES_ABOVE;
    let (kind, trunk, spread) = if warmth < PINE_TEMPERATURE || alpine {
        (
            TreeKind::Pine,
            16 + (growth * 9.0) as i64,
            4.5 + growth * 1.5,
        )
    } else {
        (
            TreeKind::Broadleaf,
            10 + (growth * 6.0) as i64,
            5.0 + growth * 2.0,
        )
    };
    Some(Tree::new(
        kind,
        I64Vec3::new(x, heights.base_height(x, z), z),
        trunk,
        spread,
        seed,
    ))
}

// splitmix64 over a seed and two coordinates
pub(super) fn hash(seed: u64, a: i64, b: i64) -> u64 {
    let mut h = seed
        ^ (a as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (b as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h = (h ^ (h >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    h ^ (h >> 31)
}

#[cfg(test)]
mod tests {

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
        tree.plant(
            low,
            1,
            (IVec3::ZERO, (high - low).as_ivec3()),
            |cell, block| {
                blocks.push((low + cell.as_i64vec3(), block));
            },
        );
        blocks
    }

    #[test]
    fn test_leaves_lie_within_their_bounds() {
        for tree in extremes() {
            let (low, high) = tree.leaf_bounds();
            for y in -2..=TREE_HEIGHT + 2 {
                for x in -TREE_REACH - 2..=TREE_REACH + 2 {
                    for z in -TREE_REACH - 2..=TREE_REACH + 2 {
                        let at = I64Vec3::new(x, y, z);
                        if tree.in_leaves(at.as_dvec3()) {
                            assert!(at.cmpge(low).all() && at.cmple(high).all(), "{at}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn test_trees_are_a_trunk_under_big_crowns() {
        for t in extremes() {
            let blocks = blocks(&t);
            let in_trunk = |p: I64Vec3| {
                (0..t.width()).contains(&(p.x - t.base.x))
                    && (0..t.width()).contains(&(p.z - t.base.z))
            };
            let trunk_logs = blocks
                .iter()
                .filter(|(p, b)| *b == BlockType::Log && in_trunk(*p))
                .count();
            assert_eq!((t.trunk * t.width() * t.width()) as usize, trunk_logs);
            let branches = blocks
                .iter()
                .filter(|(p, b)| *b == BlockType::Log && !in_trunk(*p))
                .count();
            assert_eq!(
                t.kind == TreeKind::Broadleaf,
                branches > 0,
                "{:?} has {branches} branch logs",
                t.kind
            );
            let leaves = blocks
                .iter()
                .filter(|(_, b)| *b == BlockType::Leaves)
                .count();
            assert!(leaves > 200, "{:?} has only {leaves} leaves", t.kind);
            // the trunk's top is hidden in the leaves, all the way round
            let top = t.base + I64Vec3::Y * (t.trunk - 1);
            assert_eq!(
                Some(BlockType::Leaves),
                t.block_in_cell(top + I64Vec3::Y, 1)
            );
            for side in [
                I64Vec3::NEG_X,
                I64Vec3::NEG_Z,
                I64Vec3::X * t.width(),
                I64Vec3::Z * t.width(),
            ] {
                assert_eq!(
                    Some(BlockType::Leaves),
                    t.block_in_cell(top + side, 1),
                    "{:?} trunk bare at {side}",
                    t.kind
                );
            }
        }
    }

    #[test]
    fn test_trees_stay_within_their_reach() {
        for t in extremes() {
            let (low, high) = (
                t.base - I64Vec3::new(TREE_REACH, 0, TREE_REACH),
                t.base + I64Vec3::new(TREE_REACH, TREE_HEIGHT, TREE_REACH),
            );
            // look well past the bounds, so anything poking out is caught
            for x in low.x - 4..=high.x + 4 {
                for z in low.z - 4..=high.z + 4 {
                    for y in low.y - 4..=high.y + 4 {
                        let p = I64Vec3::new(x, y, z);
                        if t.block_in_cell(p, 1).is_some() {
                            assert!(
                                p.cmpge(low).all() && p.cmple(high).all(),
                                "{:?} reaches {p}",
                                t.kind
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn test_coarse_cells_keep_the_trunk_and_crown() {
        let t = tree(TreeKind::Broadleaf, 12, 6.0);
        // 2-block cells aligned so the trunk's column isn't at a cell's centre
        let origin = I64Vec3::new(96, 496, -44);
        let mut seen = Vec::new();
        t.plant(
            origin,
            2,
            (IVec3::splat(-8), IVec3::splat(20)),
            |_, block| seen.push(block),
        );
        assert!(seen.contains(&BlockType::Log));
        assert!(seen.contains(&BlockType::Leaves));
        // from `THIN_TRUNK_CELL` the trunk is drawn as a box of its own, so the cells are crown alone
        let mut wide = Vec::new();
        t.plant(
            origin,
            THIN_TRUNK_CELL,
            (IVec3::splat(-4), IVec3::splat(10)),
            |_, block| wide.push(block),
        );
        assert!(wide.contains(&BlockType::Leaves) && !wide.contains(&BlockType::Log));
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
        let noise = crate::worldmap::test_terrain(5);
        let regions = RegionStore::new(noise.clone());
        let mut heights = regions.sampler();
        let centre = crate::worldmap::forest_spot(&noise);
        let (min, max) = (centre - I64Vec2::splat(600), centre + I64Vec2::splat(600));
        let trees = trees_near(&*noise, &mut heights, min, max);
        assert!(!trees.is_empty(), "no trees in {min}..{max}");
        for t in &trees {
            let ground = heights.height(t.base.x, t.base.z).round() as i64;
            assert_eq!(ground, t.base.y);
        }
        // a smaller window finds the same trees where they overlap
        let some = trees_near(&*noise, &mut heights, centre, centre + I64Vec2::splat(40));
        for t in &some {
            assert!(trees
                .iter()
                .any(|o| o.base == t.base && o.trunk == t.trunk && o.kind == t.kind));
        }
    }
    // Every cell `size` blocks across the tree fills, from its bounds.
    fn cells(tree: &Tree, size: i64) -> Vec<(I64Vec3, BlockType)> {
        let (low, high) = tree.bounds();
        let origin = I64Vec3::new(
            low.x.div_euclid(size),
            low.y.div_euclid(size),
            low.z.div_euclid(size),
        ) * size;
        let mut cells = Vec::new();
        tree.plant(
            origin,
            size,
            (IVec3::ZERO, ((high - origin) / size).as_ivec3()),
            |cell, block| {
                cells.push((origin + cell.as_i64vec3() * size, block));
            },
        );
        cells
    }

    #[test]
    fn test_every_tree_leaves_a_crown_in_cells_four_and_eight_wide() {
        for t in extremes() {
            // the thinnest pines are a spire a cell or two across at eight
            for (size, least) in [(4, 2), (8, 1)] {
                let leaves = cells(&t, size)
                    .into_iter()
                    .filter(|(_, b)| *b == BlockType::Leaves)
                    .count();
                assert!(
                    leaves >= least,
                    "{:?} {} trunk at {size}: {leaves} leaf cells",
                    t.kind,
                    t.trunk
                );
            }
        }
    }

    #[test]
    fn test_trunks_stand_in_cells_up_to_two_wide_and_not_beyond() {
        let t = tree(TreeKind::Pine, 24, 6.0);
        let logs = |size| {
            cells(&t, size)
                .into_iter()
                .filter(|(_, b)| *b == BlockType::Log)
                .count()
        };
        assert!(logs(1) > 0 && logs(2) > 0);
        assert_eq!(0, logs(THIN_TRUNK_CELL));
        assert_eq!(0, logs(8));
    }

    #[test]
    fn test_coverage_widens_a_crown_by_less_than_a_cell() {
        let t = tree(TreeKind::Broadleaf, 15, 7.0);
        for size in [4, 8] {
            let reach = cells(&t, size)
                .into_iter()
                .filter(|(_, b)| *b == BlockType::Leaves)
                .map(|(p, _)| {
                    (p.x + size / 2 - t.base.x)
                        .abs()
                        .max((p.z + size / 2 - t.base.z).abs())
                })
                .max()
                .unwrap();
            // a crown's lobes hang out to about 1.35 times its spread, and a cell edge can add up to a cell
            assert!(
                reach as f64 <= t.spread * 1.35 + size as f64,
                "reach {reach} at {size}"
            );
        }
    }
}
