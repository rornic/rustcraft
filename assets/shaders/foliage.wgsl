// Leaves swaying in the wind, shared by ChunkMaterial's main and shadow passes so leaves
// keep to their shadows.

// leaves' cell in blocks.png, see `BlockType`: block meshes carry the cell in vertex alpha
const LEAF_CELL: f32 = 6.0;
// How far, in blocks, leaves sway: little enough that they never pull far from their twigs.
const SWAY: f32 = 0.08;

// Where a vertex at `world` sways to by `time`. It depends on nothing but where the vertex
// is, so corners shared by neighbouring leaves move together and leave no cracks.
fn swayed(world: vec3<f32>, vertex_alpha: f32, time: f32) -> vec3<f32> {
    if abs(vertex_alpha - LEAF_CELL) > 0.5 {
        return world;
    }
    // slow gusts roll across the canopy, with a quicker flutter on top
    let gust = sin(time * 1.3 + dot(world.xz, vec2<f32>(0.11, 0.07)));
    let flutter = sin(time * 4.1 + dot(world, vec3<f32>(0.9, 0.7, 1.3)));
    let push = gust * 0.7 + flutter * 0.3;
    return world + vec3<f32>(push, flutter * 0.4, push * 0.6) * SWAY;
}
