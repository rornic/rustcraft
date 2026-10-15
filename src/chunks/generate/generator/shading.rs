use super::source::BlockSource;
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

pub(super) const AO_STRENGTH: f32 = 0.5;

// Height of the water surface within a full-detail block, whose cube spans -0.5..0.5.
pub(super) const WATER_TOP: f32 = 0.4;

// The two tangent-direction offsets (toward whichever corner `vertex_local_pos` is
// on) for a face - shared by `vertex_ao` (which samples one layer along the face
// normal, for overhang darkening) and `vertex_shore_factor` (which samples at the
// block's own height, since shoreline land sits flush with the water surface, not
// above it).
pub(super) fn tangent_offsets(face_index: usize, vertex_local_pos: [f32; 3]) -> (IVec3, IVec3) {
    let (ta, tb) = FACE_TANGENTS[face_index];
    let axis_index = |v: IVec3| {
        if v.x != 0 {
            0
        } else if v.y != 0 {
            1
        } else {
            2
        }
    };
    let sign_a = vertex_local_pos[axis_index(ta)].signum() as i32;
    let sign_b = vertex_local_pos[axis_index(tb)].signum() as i32;
    (ta * sign_a, tb * sign_b)
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
    1.0 - (sum / 3.0).clamp(0.0, 1.0) * AO_STRENGTH
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

// Fraction of a water top-face vertex's same-height tangent corners that are solid
// land - used to bake a shoreline mask into the water mesh so the shader can lap
// foam there.
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
        .filter(|b| b.occlusion_weight() == 1.0)
        .count();
    land_count as f32 / 3.0
}

pub(super) const SHORE_DISTANCE_RADIUS: i32 = 3;

// Distance (in blocks, normalized to 0..1 over `SHORE_DISTANCE_RADIUS`, 1.0 meaning
// "no land within range") from this top-face vertex to the nearest land block -
// anchored at the *shared grid corner* (block_coord + tangent offsets) rather than
// the block itself, so up to four blocks touching that corner all compute the exact
// same distance there. That's what keeps it safe to drive the foam's travelling
// phase: unlike per-block data, it can't disagree at a shared edge.
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
