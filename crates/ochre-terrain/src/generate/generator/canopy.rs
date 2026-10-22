use super::lod::LodBlocks;
use super::source::{BoxFaces, TreeBox};
use crate::generate::trees::Tree;
use crate::generate::trees::TREE_REACH;
use bevy::math::I64Vec3;
use bevy::math::UVec2;
use bevy::math::Vec3;
use ochre_base::block::BlockType;

// How many stacked boxes a crown is drawn as in cells `cell` blocks wide, too wide to hold it:
// fewer the farther out, where there are more of them and each is smaller.
pub(super) fn crown_layers(cell: i64) -> i64 {
    match cell {
        ..=4 => 3,
        8 => 2,
        _ => 1,
    }
}

// From cells this wide, trees are drawn together as the canopy they make, a few boxes a node,
// instead of each one as boxes of its own: there are so many in view this far out that their
// boxes would cost far more than the land under them, and each is only a few pixels across.
pub(super) const CANOPY_CELL: i64 = 16;

// How many points across each column, each way, its canopy's cover is judged from.
pub(super) const CANOPY_SAMPLES: usize = 4;

// Columns at least this much under crowns are canopy. The rest only darken the ground, by how
// much of them is, see `LodBlocks::forest_shade`. Higher, and the trees left out of the canopy
// each need a box of their own; lower, and it fills more of the gaps between crowns.
pub(super) const CANOPY_COVER: f32 = 0.5;

// Canopy tops are rounded to this many steps a cell, so neighbouring columns merge into one box.
pub(super) const CANOPY_STEPS: i64 = 4;

// The trees around a coarse node seen from above, column by padded column.
pub(super) struct Canopy {
    // which of each column's `CANOPY_SAMPLES` x `CANOPY_SAMPLES` points are under crowns, x fastest
    pub(super) hits: Vec<u32>,
    // how much of each column crowns cover, 0..1
    pub(super) cover: Vec<f32>,
    // the crowns' tops over each column, in blocks, weighed by how much of it each covers
    pub(super) tops: Vec<f64>,
    // the same for the crowns' bottoms
    pub(super) bottoms: Vec<f64>,
    // the mean tint of the crowns over each column
    pub(super) tints: Vec<Vec3>,
}

impl Canopy {
    pub(super) fn new(trees: &[Tree], base: I64Vec3, cell: i64, size: i32) -> Self {
        let padded = ((size + 2) * (size + 2)) as usize;
        let (mut hits, mut tops, mut bottoms, mut tints) = (
            vec![0u32; padded],
            vec![0.0; padded],
            vec![0.0; padded],
            vec![Vec3::ZERO; padded],
        );
        let mut weights = vec![0u32; padded];
        let step = cell as f64 / CANOPY_SAMPLES as f64;
        for tree in trees {
            let column_of =
                |x: i64, base: i64| ((x - base).div_euclid(cell) as i32).clamp(-1, size);
            let reach = |x: i64, base: i64| {
                column_of(x - TREE_REACH, base)..=column_of(x + TREE_REACH, base)
            };
            let (top, bottom, tint) = (tree.crown_top(), tree.crown_bottom(), tree.tint());
            for cx in reach(tree.base.x, base.x) {
                for cz in reach(tree.base.z, base.z) {
                    let column = LodBlocks::column(size, cx, cz);
                    let corner = (base.x + cx as i64 * cell, base.z + cz as i64 * cell);
                    let mut under = 0;
                    for i in 0..CANOPY_SAMPLES * CANOPY_SAMPLES {
                        let x = corner.0 as f64 + ((i % CANOPY_SAMPLES) as f64 + 0.5) * step;
                        let z = corner.1 as f64 + ((i / CANOPY_SAMPLES) as f64 + 0.5) * step;
                        if tree.shades(x, z) {
                            under |= 1 << i;
                        }
                    }
                    if under != 0 {
                        hits[column] |= under;
                        tops[column] += top * under.count_ones() as f64;
                        bottoms[column] += bottom * under.count_ones() as f64;
                        tints[column] += tint * under.count_ones() as f32;
                        weights[column] += under.count_ones();
                    }
                }
            }
        }
        let samples = (CANOPY_SAMPLES * CANOPY_SAMPLES) as f32;
        let cover = hits
            .iter()
            .map(|h| h.count_ones() as f32 / samples)
            .collect();
        let tints = tints
            .iter()
            .zip(&weights)
            .map(|(&t, &w)| if w > 0 { t / w as f32 } else { Vec3::ONE })
            .collect();
        let tops = tops
            .iter()
            .zip(&weights)
            .map(|(&t, &w)| if w > 0 { t / w as f64 } else { 0.0 })
            .collect();
        let bottoms = bottoms
            .iter()
            .zip(&weights)
            .map(|(&b, &w)| if w > 0 { b / w as f64 } else { 0.0 })
            .collect();
        Self {
            hits,
            cover,
            tops,
            bottoms,
            tints,
        }
    }

    // Whether `tree` is part of the canopy, which is drawn in its place: the column it stands in is.
    pub(super) fn covers(&self, tree: &Tree, base: I64Vec3, cell: i64, size: i32) -> bool {
        let at = |x: i64, base: i64| ((x - base).div_euclid(cell) as i32).clamp(-1, size);
        self.cover[LodBlocks::column(size, at(tree.base.x, base.x), at(tree.base.z, base.z))]
            >= CANOPY_COVER
    }

    // The canopy over this node's columns whose ground is in it, as boxes from the ground up to
    // the crowns' tops, merged where columns side by side round to the same top, and drawn in at
    // their edges to the points under crowns, so they're no wider than the trees.
    pub(super) fn boxes(
        &self,
        ground_heights: &[f32],
        base: I64Vec3,
        cell: i64,
        size: i32,
    ) -> Vec<TreeBox> {
        let n = size as usize;
        let step = (cell / CANOPY_STEPS) as f64;
        let ceiling = base.y + size as i64 * cell;
        let column_of = |x: usize, z: usize| LodBlocks::column(size, x as i32, z as i32);
        let top = |x: usize, z: usize| {
            let column = column_of(x, z);
            let ground = ground_heights[column] as i64;
            let canopy = self.cover[column] >= CANOPY_COVER && (base.y..ceiling).contains(&ground);
            canopy.then(|| (self.tops[column] / step).round() as i64)
        };
        let mut taken = vec![false; n * n];
        let mut boxes = Vec::new();
        let mesh_y = |y: f64| ((y - base.y as f64) / cell as f64 - 0.5) as f32;
        for x0 in 0..n {
            for z0 in 0..n {
                let Some(steps) = top(x0, z0).filter(|_| !taken[x0 * n + z0]) else {
                    continue;
                };
                let same = |x: usize, z: usize| !taken[x * n + z] && top(x, z) == Some(steps);
                let mut x1 = x0 + 1;
                while x1 < n && same(x1, z0) {
                    x1 += 1;
                }
                let mut z1 = z0 + 1;
                while z1 < n && (x0..x1).all(|x| same(x, z1)) {
                    z1 += 1;
                }
                // one column alone is a cube that stands out from afar, and the ground is tinted for it
                let alone = x1 - x0 == 1 && z1 - z0 == 1;
                let (mut lowest, mut tint, mut count) = (f64::MAX, Vec3::ZERO, 0.0);
                // the points under crowns' bounds, in points across the node
                let (mut near, mut far) = (UVec2::MAX, UVec2::ZERO);
                for x in x0..x1 {
                    for z in z0..z1 {
                        taken[x * n + z] = true;
                        let column = column_of(x, z);
                        lowest = lowest.min(self.bottoms[column]);
                        tint += self.tints[column];
                        count += 1.0;
                        for i in (0..CANOPY_SAMPLES * CANOPY_SAMPLES)
                            .filter(|i| self.hits[column] & (1 << i) != 0)
                        {
                            let point = UVec2::new(
                                (x * CANOPY_SAMPLES + i % CANOPY_SAMPLES) as u32,
                                (z * CANOPY_SAMPLES + i / CANOPY_SAMPLES) as u32,
                            );
                            (near, far) = (near.min(point), far.max(point));
                        }
                    }
                }
                // each point stands for the square round it
                let samples = CANOPY_SAMPLES as f32;
                let (near, far) = (
                    near.as_vec2() / samples - 0.5,
                    (far + 1).as_vec2() / samples - 0.5,
                );
                // from the crowns' bottoms, so no pillar stands where the trunks are too thin to draw
                let bottom = mesh_y(lowest);
                if alone {
                    continue;
                }
                boxes.push(TreeBox {
                    min: Vec3::new(near.x, bottom, near.y),
                    max: Vec3::new(far.x, mesh_y(steps as f64 * step).max(bottom + 0.1), far.y),
                    block: BlockType::Leaves,
                    tint: tint / count,
                    faces: BoxFaces::All,
                    taper: 0.0,
                });
            }
        }
        boxes
    }
}

// Coarse cells up to this wide hold real trees. Wider ones, where a crown would be a cube many
// times its size, tint the ground by how much forest covers it instead, so forests never cut off.
pub(super) const TREE_MAX_CELL: i64 = 32;

// Trunks are boxes only up to cells this wide: past them one is far under a pixel.
pub(super) const TRUNK_MAX_CELL: i64 = 8;

// From cells this wide a crown is a box too, of its real size: as cells it would round up to a
// cube a cell wide, several times its size.
pub(super) const BOX_CROWN_CELL: i64 = 8;

// Each tree a cell grows covers this much of its ground, see `Biome::tree_density`, and a cell
// that's all covered is drawn this fraction as bright.
pub(super) const FOREST_COVER: f64 = 1.6;

pub(super) const FOREST_DARKEN: f32 = 0.55;

// Leaves at the foot of a canopy are this much darker than at its top, over this many blocks.
pub(super) const CANOPY_SHADE: f32 = 0.45;

pub(super) const CANOPY_SHADE_SPAN: f32 = 20.0;
