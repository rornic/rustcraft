// Leaves and ground flora swaying in the wind, shared by ChunkMaterial's main and shadow
// passes so they keep to their shadows.

// How far, in blocks, leaves sway: little enough that they never pull far from their twigs.
const SWAY: f32 = 0.08;
// How far, in blocks, a plant's tip sways. Its roots stay put.
const FLORA_SWAY: f32 = 0.16;
// How far a plant leans with the wind on average, as a share of `FLORA_SWAY`, so a meadow
// bends one way in a gust rather than wobbling about upright.
const FLORA_LEAN: f32 = 0.35;

// Whether a block mesh's vertex alpha, its atlas cell, is leaves'. `LEAF_CELL` comes from
// `ChunkMaterial::specialize`.
fn is_leaf(vertex_alpha: f32) -> bool {
    return abs(vertex_alpha - f32(constants::LEAF_CELL)) < 0.5;
}

// How much of the way leaves at `world_position` are to having their holes filled with the shade
// of the leaves behind them, 0..1: past `INNER_LEAVES_REACH` blocks of the camera at `eye` those
// leaves aren't drawn, see `InnerLeaves`, and the holes fill in over the band before it.
fn leaf_holes_filled(world_position: vec3<f32>, eye: vec3<f32>) -> f32 {
    let reach = f32(constants::INNER_LEAVES_REACH);
    let band = f32(constants::INNER_LEAVES_BAND);
    return smoothstep(reach - band, reach, distance(world_position, eye));
}

// Whether it's ground flora's, whose sprites are the atlas's last tiles. `FIRST_FLORA_CELL`
// comes from `ChunkMaterial::specialize`.
fn is_flora(vertex_alpha: f32) -> bool {
    return vertex_alpha > f32(constants::FIRST_FLORA_CELL) - 0.5;
}

// Where a vertex at `world` sways to by `time`. It depends on nothing but where the vertex
// is, so corners shared by neighbouring leaves move together and leave no cracks. Coarse LOD
// cells, whose faces span many blocks, stay still: their corners would drift apart and tear.
// Flora's tops sway, by `uv`'s v, which is 0 at a plant's tip and 1 at its roots.
fn swayed(world: vec3<f32>, vertex_alpha: f32, uv: vec2<f32>, coarse: bool, time: f32) -> vec3<f32> {
    // `DebugSettings::still_leaves`
@if(STILL_LEAVES) {
    return world;
}
    if coarse {
        return world;
    }
    // slow gusts roll across the land, the same for leaves and plants, so one crosses a
    // meadow and the wood behind it alike
    if is_flora(vertex_alpha) {
        let gust = sin(time * 1.3 + dot(world.xz, vec2<f32>(0.11, 0.07)));
        // bending most at the tip, as a stem does, and quicker than the boughs
        let bend = 1.0 - clamp(uv.y, 0.0, 1.0);
        let flutter = sin(time * 3.3 + dot(world.xz, vec2<f32>(1.7, 1.1)));
        let push = (FLORA_LEAN + gust * 0.45 + flutter * 0.25) * bend * bend * FLORA_SWAY;
        // a leaning tip dips a little, as its stem doesn't stretch
        return world + vec3<f32>(push, -abs(push) * 0.3, push * 0.6);
    }
    if !is_leaf(vertex_alpha) {
        return world;
    }
    let gust = sin(time * 1.3 + dot(world.xz, vec2<f32>(0.11, 0.07)));
    // with a quicker flutter on top
    let flutter = sin(time * 4.1 + dot(world, vec3<f32>(0.9, 0.7, 1.3)));
    let push = gust * 0.7 + flutter * 0.3;
    return world + vec3<f32>(push, flutter * 0.4, push * 0.6) * SWAY;
}
