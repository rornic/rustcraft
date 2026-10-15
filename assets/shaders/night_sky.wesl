// The night's stars, Milky Way and moon, drawn over the sky, see `night_sky.rs`.

import bevy_pbr::render::{
    forward_io::{Vertex, VertexOutput},
    mesh_functions,
    mesh_view_bindings as view_bindings,
    view_transformations::position_world_to_clip,
};
import super::stars::celestial;

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
    let direction = normalize(in.world_position.xyz - view_bindings::view.world_position.xyz);
    // radians a pixel spans, worked out where every fragment does
    let footprint = length(fwidth(direction));
    // premultiplied: the moon covers the sky, the rest only adds light
    return celestial(direction, footprint, true);
}
