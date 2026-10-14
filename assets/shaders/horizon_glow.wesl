// The glow along the curved horizon drawn over the sky, see `horizon_glow.rs`. The land
// takes on the same glow through its fog, see `fog.wesl`, so the two meet without a seam.

import bevy_pbr::render::{
    forward_io::{Vertex, VertexOutput},
    mesh_functions,
    mesh_view_bindings as view_bindings,
    view_transformations::position_world_to_clip,
};
import super::horizon::{glow_strength, horizon_glow, sky_bend_top, sky_over_curve};

// Pinned to the far plane, so it only draws where there's nothing but sky.
@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    out.world_position = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(vertex.position, 1.0));
    out.position = position_world_to_clip(out.world_position.xyz);
    // reversed depth: 0 is infinitely far
    out.position.z = 0.0;
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let strength = glow_strength();
    if strength <= 0.0 {
        discard;
    }
    let toward = in.world_position.xyz - view_bindings::view.world_position.xyz;
    let across = toward.xz;
    let slope = toward.y / max(length(across), 1e-6);
    // the sky brought down to the land's edge, handing back to the sky itself up high
    let top = sky_bend_top();
    let cover = strength * (1.0 - smoothstep(0.5 * top, top, slope));
    // premultiplied: the glow adds to the sky, the lowered sky covers it
    return vec4<f32>(sky_over_curve(across, slope) * cover + horizon_glow(across, slope), cover);
}
