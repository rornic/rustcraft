// Rain falling round the camera, see `weather::RainMaterial`. The mesh is a box of streaks
// that each carry a fixed spot in their position and which corner they are in their uv;
// they fall here, wrapped round the camera so the box goes wherever it does.

import bevy_pbr::render::{
    forward_io::{Vertex, VertexOutput},
    mesh_view_bindings as view_bindings,
    view_transformations::position_world_to_clip,
};

// x: how hard it's raining, 0..1; y: how bright the light on it is; zw: the wind, in blocks
// a second across the ground.
@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> rain: vec4<f32>;

// Matches `RAIN_BOX` in `weather.rs`.
const BOX: vec3<f32> = vec3<f32>(48.0, 32.0, 48.0);
// How fast drops fall, in blocks a second, and how long and wide a streak is.
const FALL: f32 = 22.0;
const LENGTH: f32 = 0.9;
const WIDTH: f32 = 0.012;

// A number 0..1 that's the same for every corner of a streak, from where it starts.
fn hash(p: vec3<f32>) -> f32 {
    return fract(sin(dot(p, vec3<f32>(12.9898, 78.233, 37.719))) * 43758.5453);
}

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let eye = view_bindings::view.world_position;
    let time = view_bindings::globals.time;
    let seed = hash(vertex.position);
    // heavier drops fall faster
    let velocity = vec3<f32>(rain.z, -FALL * (0.8 + 0.4 * seed), rain.w);
    let fallen = vertex.position + velocity * time - eye;
    let offset = (fract(fallen / BOX + 0.5) - 0.5) * BOX;
    let centre = eye + offset;
    // a streak along its fall, turned to face the camera across it
    let along = normalize(velocity);
    let across = normalize(cross(along, normalize(eye - centre)));
    let world = centre + across * vertex.uv.x * WIDTH * (1.0 + length(offset) * 0.05) + along * (vertex.uv.y - 0.5) * LENGTH;
    out.world_position = vec4<f32>(world, 1.0);
    out.position = position_world_to_clip(world);
    out.world_normal = vec3<f32>(0.0, 1.0, 0.0);
    // how far through the box it is, to fade it out before it wraps round, and only some
    // of the streaks show in lighter rain
    let through = abs(offset) / (BOX * 0.5);
    let edge = max(through.x, max(through.y, through.z));
    let shown = step(seed, rain.x);
    out.uv = vertex.uv;
    // and before it's so near it smears across the view
    let near = smoothstep(1.5, 5.0, length(offset));
    out.color = vec4<f32>((1.0 - smoothstep(0.7, 1.0, edge)) * near, shown, 0.0, 0.0);
    // a streak that won't show is put outside the view, so it's never rasterised
    if shown * near == 0.0 {
        out.position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
    }
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let across = 1.0 - abs(in.uv.x);
    let along = sin(in.uv.y * 3.14159);
    let alpha = across * along * in.color.x * in.color.y * 0.35;
    if alpha < 0.002 {
        discard;
    }
    let tint = vec3<f32>(0.72, 0.78, 0.86) * rain.y;
    return vec4<f32>(tint * alpha, alpha);
}
