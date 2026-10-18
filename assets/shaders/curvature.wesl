// Bends the land away from the camera as if the world were round, see `curvature.rs`. Only
// how it's drawn: the world itself stays flat.

struct Bend {
    // the camera's position, not the view's: the sun's shadow passes see the light's
    eye: vec3<f32>,
    // how far land drops per square block of distance across from the camera
    per_square_block: f32,
}

struct Bends {
    now: Bend,
    // last frame's, for motion vectors
    last: Bend,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(8) var<storage, read> curvature: Bends;

fn bent(world_position: vec3<f32>, bend: Bend) -> vec3<f32> {
    let across = world_position.xz - bend.eye.xz;
    return world_position - vec3<f32>(0.0, dot(across, across) * bend.per_square_block, 0.0);
}

// Where `world_position` is drawn this frame.
fn curved(world_position: vec3<f32>) -> vec3<f32> {
    return bent(world_position, curvature.now);
}

// Where `world_position` was drawn last frame.
fn curved_last(world_position: vec3<f32>) -> vec3<f32> {
    return bent(world_position, curvature.last);
}

// Where land drawn at `world_position` this frame really is.
fn uncurved(world_position: vec3<f32>) -> vec3<f32> {
    let across = world_position.xz - curvature.now.eye.xz;
    return world_position + vec3<f32>(0.0, dot(across, across) * curvature.now.per_square_block, 0.0);
}

// Where the camera is, in every pass: the sun's shadow passes' view is the light's.
fn eye() -> vec3<f32> {
    return curvature.now.eye;
}
