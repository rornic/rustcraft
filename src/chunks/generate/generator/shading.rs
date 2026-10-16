use super::source::BlockSource;
use super::terrain::is_solid;
use crate::chunks::generate::light::SkyLight;
use crate::chunks::generate::light::MAX_LIGHT;
use bevy::math::IVec3;

pub(in crate::chunks::generate) const FACE_NORMAL_OFFSET: [IVec3; 6] = [
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

    let side_a = source.block(block_coord + offset_a);
    let side_b = source.block(block_coord + offset_b);
    let corner = source.block(block_coord + offset_a + offset_b);

    let land_count = [side_a, side_b, corner]
        .iter()
        .filter(|&&b| is_solid(b))
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
