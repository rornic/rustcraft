// Leaves swaying in the wind, shared by ChunkMaterial's main and shadow passes so leaves
// keep to their shadows.

// How far, in blocks, leaves sway: little enough that they never pull far from their twigs.
const SWAY: f32 = 0.08;

// Whether a block mesh's vertex alpha, its atlas cell, is leaves'. `LEAF_CELL` comes from
// `ChunkMaterial::specialize`.
fn is_leaf(vertex_alpha: f32) -> bool {
    return abs(vertex_alpha - f32(#{LEAF_CELL})) < 0.5;
}

// Where a vertex at `world` sways to by `time`. It depends on nothing but where the vertex
// is, so corners shared by neighbouring leaves move together and leave no cracks. Coarse LOD
// cells, whose faces span many blocks, stay still: their corners would drift apart and tear.
fn swayed(world: vec3<f32>, vertex_alpha: f32, coarse: bool, time: f32) -> vec3<f32> {
#ifdef STILL_LEAVES
    // `DebugSettings::still_leaves`
    return world;
#else
    if coarse || !is_leaf(vertex_alpha) {
        return world;
    }
    // slow gusts roll across the canopy, with a quicker flutter on top
    let gust = sin(time * 1.3 + dot(world.xz, vec2<f32>(0.11, 0.07)));
    let flutter = sin(time * 4.1 + dot(world, vec3<f32>(0.9, 0.7, 1.3)));
    let push = gust * 0.7 + flutter * 0.3;
    return world + vec3<f32>(push, flutter * 0.4, push * 0.6) * SWAY;
#endif
}
