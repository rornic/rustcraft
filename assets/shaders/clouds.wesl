// Clouds: lit by the same sun and sky as the land, and fogged with it, see `clouds.rs`.

import bevy_pbr::render::{
    forward_io::{Vertex, VertexOutput, decompress_vertex},
    mesh_functions,
    mesh_view_bindings as view_bindings,
    view_transformations::position_world_to_clip,
};
import super::curvature::{curved, uncurved};
import super::fog::fogged;
import super::lighting::cloud_light;
import super::dither::dither_threshold;

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> cloud_color: vec4<f32>;
// the clouds' base and greatest height
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var<uniform> cloud_shape: vec4<f32>;

// How much light reaches a cloud's base, through the cloud above it, to its top's.
const BASE_LIGHT: f32 = 0.8;
// Clouds fade away this close to the camera, in blocks, so flying through one doesn't
// leave the view a wall of white.
const NEAR_FADE: vec2<f32> = vec2<f32>(8.0, 40.0);

// Bevy's mesh vertex shader, for meshes that are never skinned or morphed, with the clouds
// curving away with the land below them.
@vertex
fn vertex(vertex_in: Vertex) -> VertexOutput {
    let vertex = decompress_vertex(vertex_in, vertex_in.instance_index);
    var out: VertexOutput;
    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    let world_position = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(vertex.position, 1.0));
    out.world_position = vec4<f32>(curved(world_position.xyz), 1.0);
    out.position = position_world_to_clip(out.world_position.xyz);
    out.world_normal = mesh_functions::mesh_normal_local_to_world(vertex.normal, vertex.instance_index);
@if(VERTEX_OUTPUT_INSTANCE_INDEX) {
    out.instance_index = vertex.instance_index;
}
@if(VISIBILITY_RANGE_DITHER) {
    out.visibility_range_dither = mesh_functions::get_visibility_range_dither_level(vertex.instance_index, world_from_local[3]);
}
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let distance = length(in.world_position.xyz - view_bindings::view.world_position.xyz);
    if dither_threshold(in.position.xy) >= smoothstep(NEAR_FADE.x, NEAR_FADE.y, distance) {
        discard;
    }
    let normal = normalize(in.world_normal);
    // where the cloud really is
    let position = uncurved(in.world_position.xyz);
    let up = clamp((position.y - cloud_shape.x) / cloud_shape.y, 0.0, 1.0);
    let light = cloud_light(normal, position) * mix(BASE_LIGHT, 1.0, up);
    return fogged(vec4<f32>(cloud_color.rgb * light, 1.0), in.world_position.xyz, in.position.xy);
}
