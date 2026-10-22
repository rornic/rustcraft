use super::source::BlockSource;
use super::terrain::is_solid;
use crate::generate::light::SkyLight;
use crate::generate::light::MAX_LIGHT;
use bevy::math::IVec3;

pub(in crate::generate) const FACE_NORMAL_OFFSET: [IVec3; 6] = [
    IVec3::new(0, 0, -1), // front
    IVec3::new(1, 0, 0),  // right
    IVec3::new(-1, 0, 0), // left
    IVec3::new(0, 0, 1),  // back
    IVec3::new(0, 1, 0),  // top
    IVec3::new(0, -1, 0), // bottom
];

pub(super) const FACE_TANGENTS: [(IVec3, IVec3); 6] = [
    (IVec3::new(1, 0, 0), IVec3::new(0, 1, 0)), // front
    (IVec3::new(0, 0, 1), IVec3::new(0, 1, 0)), // right
    (IVec3::new(0, 0, 1), IVec3::new(0, 1, 0)), // left
    (IVec3::new(1, 0, 0), IVec3::new(0, 1, 0)), // back
    (IVec3::new(1, 0, 0), IVec3::new(0, 0, 1)), // top
    (IVec3::new(1, 0, 0), IVec3::new(0, 0, 1)), // bottom
];

// How dark a fully occluded corner gets, 0..1; two solid sides already count as fully occluded, see `vertex_ao`.
pub(super) const AO_STRENGTH: f32 = 0.6;
// AO reads as the brightness it should show, so it's raised to this to multiply in linear light: a 0.7 straight in linear shows as 0.85.
pub(super) const AO_GAMMA: f32 = 2.2;

// Height of the water surface within a full-detail block, whose cube spans -0.5..0.5.
pub(super) const WATER_TOP: f32 = 0.4;
// How far under a block's top water's surface sits.
pub(super) const WATER_DROP: f32 = 0.5 - WATER_TOP;

// The axis a unit offset runs along.
pub(super) fn axis_of(v: IVec3) -> usize {
    if v.x != 0 {
        0
    } else if v.y != 0 {
        1
    } else {
        2
    }
}

// A face's two tangents, each pointing toward the corner `vertex_local_pos` is on.
pub(super) fn tangent_offsets(face_index: usize, vertex_local_pos: [f32; 3]) -> (IVec3, IVec3) {
    let (ta, tb) = FACE_TANGENTS[face_index];
    let sign = |t: IVec3| vertex_local_pos[axis_of(t)].signum() as i32;
    (ta * sign(ta), tb * sign(tb))
}

pub(super) fn vertex_ao(
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
    1.0 - (sum / 2.0).clamp(0.0, 1.0) * AO_STRENGTH
}

// Sky light at a face's corner, 0..1: the average over the cells around the corner on the
// face's open side, leaving out solid ones, and the diagonal when both sides are solid
// since light can't reach round to it.
pub(super) fn vertex_light(
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
    let levels: Vec<f32> = cells
        .into_iter()
        .flatten()
        .map(|c| light.level(c) as f32)
        .collect();
    levels.iter().sum::<f32>() / levels.len() as f32 / MAX_LIGHT as f32
}

// One light level for a whole face, 0..1, in a few coarse steps so neighbouring faces
// match and merge.
pub(super) fn face_light(level: u8) -> f32 {
    const STEP: u8 = 3;
    let stepped = (level + STEP / 2) / STEP * STEP;
    stepped.min(MAX_LIGHT) as f32 / MAX_LIGHT as f32
}

// The share of the cells round a water top-face corner, at its height, that are land, where the
// shader laps foam.
pub(super) fn vertex_shore_factor(
    source: &impl BlockSource,
    block_coord: IVec3,
    face_index: usize,
    vertex_local_pos: [f32; 3],
) -> f32 {
    let (offset_a, offset_b) = tangent_offsets(face_index, vertex_local_pos);
    // a sloped top's columns are land where they have no fresh water, so blocks either side of a step agree
    let sloped = source.water_level(block_coord.x, block_coord.z).is_some();
    let is_land = |c: IVec3| match sloped {
        true => source.water_level(c.x, c.z).is_none(),
        false => is_solid(source.block(c)),
    };

    let land_count = [offset_a, offset_b, offset_a + offset_b]
        .into_iter()
        .filter(|&offset| is_land(block_coord + offset))
        .count();
    land_count as f32 / 3.0
}

pub(super) const SHORE_DISTANCE_RADIUS: i32 = 3;

// Distance from this top-face corner to the nearest land, 0..1 over `SHORE_DISTANCE_RADIUS`, 1 for
// none in range. Measured from the corner itself, so every block sharing it agrees, which keeps
// the foam's travelling phase from tearing at shared edges.
pub(super) fn vertex_shore_distance(
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
            if is_solid(source.block(probe)) {
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

// Water depth at a top-face corner, averaged over the four columns sharing it (land counts as 0).
pub(super) fn vertex_water_depth(
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

// Columns whose surfaces differ by more than this are a curtain between them, not one slope.
pub(super) const WATER_SLOPE_MAX_STEP: f32 = 1.0;

// The x and z signs, -1 or 1, of the top-face corner a vertex at `vertex_local_pos` is over.
pub(super) fn corner_signs(vertex_local_pos: [f32; 3]) -> (i32, i32) {
    let (a, b) = tangent_offsets(4, vertex_local_pos);
    ((a + b).x, (a + b).z)
}

// A top-face corner of a water column: the surface's height there, its slope in x and z, and the columns joining it.
#[derive(Clone, Copy)]
pub(super) struct WaterCorner {
    pub(super) height: f32,
    pub(super) slope: [f32; 2],
    joined: [(i32, i32); 4],
    count: usize,
}

impl WaterCorner {
    pub(super) fn joins(&self, column: (i32, i32)) -> bool {
        self.joined[..self.count].contains(&column)
    }
}

// The corner `signs` of `column`, joined by the columns round it until sorted levels gap a block, so a group agrees on it.
pub(super) fn water_corner(
    level_at: &mut impl FnMut(i32, i32) -> Option<f32>,
    column: (i32, i32),
    own: f32,
    (dx, dz): (i32, i32),
) -> WaterCorner {
    let mut levels = [(own, column); 4];
    let mut count = 1;
    for (ox, oz) in [(dx, 0), (0, dz), (dx, dz)] {
        let c = (column.0 + ox, column.1 + oz);
        if let Some(level) = level_at(c.0, c.1) {
            levels[count] = (level, c);
            count += 1;
        }
    }
    let levels = &mut levels[..count];
    levels.sort_by(|a, b| a.0.total_cmp(&b.0));
    let at = levels.iter().position(|&(_, c)| c == column).unwrap();
    let start = (0..at)
        .rev()
        .find(|&i| levels[i + 1].0 - levels[i].0 >= WATER_SLOPE_MAX_STEP)
        .map_or(0, |i| i + 1);
    let end = (at..count - 1)
        .find(|&i| levels[i + 1].0 - levels[i].0 >= WATER_SLOPE_MAX_STEP)
        .map_or(count, |i| i + 1);
    let group = &levels[start..end];
    let height = group.iter().map(|&(level, _)| level).sum::<f32>() / group.len() as f32;
    // the far side's average less the near side's, along each axis, where the group has both
    let rise = |far: &dyn Fn((i32, i32)) -> bool| {
        let (mut sums, mut counts) = ([0.0; 2], [0.0; 2]);
        for &(level, c) in group {
            let side = far(c) as usize;
            sums[side] += level;
            counts[side] += 1.0;
        }
        if counts[0] > 0.0 && counts[1] > 0.0 {
            sums[1] / counts[1] - sums[0] / counts[0]
        } else {
            0.0
        }
    };
    let slope = [
        rise(&|c| c.0 == column.0.max(column.0 + dx)),
        rise(&|c| c.1 == column.1.max(column.1 + dz)),
    ];
    let mut joined = [(0, 0); 4];
    for (slot, &(_, c)) in joined.iter_mut().zip(group) {
        *slot = c;
    }
    WaterCorner {
        height,
        slope,
        joined,
        count: group.len(),
    }
}

// Columns a step in the water's surface is spread out over, each side of it.
pub(super) const WATER_SPREAD_REACH: i32 = 5;
// The most the spread moves a surface: a drop past twice it keeps a curtain for the rest.
const WATER_SPREAD_MAX_MOVE: f32 = 1.5;
// How far under a bank's ground a water corner beside it is held, so raised water never stands over it.
pub(super) const WATER_BANK_MARGIN: f32 = 0.05;

// `raw` (NaN where dry) averaged over the water within reach of each column of the grid `WATER_SPREAD_REACH` inside it
// shortcut: the spread is only drawn, the blocks under it aren't moved; fill them at generation if swimming in it matters
pub(super) fn spread_water_levels(raw: &[f32], wide: i32) -> Vec<f32> {
    let reach = WATER_SPREAD_REACH;
    let inner = wide - 2 * reach;
    (0..inner * inner)
        .map(|i| {
            let (x, z) = (i / inner + reach, i % inner + reach);
            let own = raw[(x * wide + z) as usize];
            if own.is_nan() {
                return own;
            }
            let (mut total, mut count) = (0.0, 0.0);
            for dx in -reach..=reach {
                for dz in -reach..=reach {
                    let level = raw[((x + dx) * wide + z + dz) as usize];
                    if dx * dx + dz * dz <= reach * reach && !level.is_nan() {
                        total += level;
                        count += 1.0;
                    }
                }
            }
            own + (total / count - own).clamp(-WATER_SPREAD_MAX_MOVE, WATER_SPREAD_MAX_MOVE)
        })
        .collect()
}
