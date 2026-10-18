// A rainbow after the rain, see `weather::RainbowMaterial`: a ring round the point opposite
// the sun, its uv.x running from the inside of the band to the outside, fading where it
// would dip below the camera, as a real one ends at the ground.

import bevy_pbr::render::{forward_io::VertexOutput, mesh_view_bindings as view_bindings};

// x: how strong it is, 0..1; y: how bright the daylight is.
@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> bow: vec4<f32>;

// The colours from the inside of the bow out, violet to red.
fn band(t: f32) -> vec3<f32> {
    let colours = array<vec3<f32>, 6>(
        vec3<f32>(0.45, 0.2, 0.75),
        vec3<f32>(0.2, 0.3, 1.0),
        vec3<f32>(0.2, 0.8, 0.4),
        vec3<f32>(1.0, 0.95, 0.25),
        vec3<f32>(1.0, 0.55, 0.15),
        vec3<f32>(0.95, 0.15, 0.12),
    );
    let x = clamp(t, 0.0, 1.0) * 5.0;
    let i = u32(floor(x));
    let j = min(i + 1u, 5u);
    return mix(colours[i], colours[j], fract(x));
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let t = in.uv.x;
    // soft at both edges of the band
    let edges = smoothstep(0.0, 0.18, t) * (1.0 - smoothstep(0.82, 1.0, t));
    let eye = view_bindings::view.world_position;
    let above = smoothstep(eye.y - 10.0, eye.y + 60.0, in.world_position.y);
    let alpha = edges * above * bow.x * 0.3;
    if alpha < 0.002 {
        discard;
    }
    return vec4<f32>(band(t) * bow.y * alpha, alpha);
}
