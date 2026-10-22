// The clouds' low-res target, lit and blended over the scene wherever nothing nearer hides it, see `clouds.rs`.

import bevy_pbr::render::{
    forward_io::{Vertex, VertexOutput},
    mesh_functions,
    mesh_view_bindings as view_bindings,
    view_transformations::{depth_ndc_to_view_z, position_world_to_clip},
};
import super::clouds::lit_clouds;
import super::cloud_march::{FAR, cloud_entry};
import super::curvature::curvature;

// the share of light they scatter, in x, see `CloudMaterial::set_shade`
@group(constants::MATERIAL_BIND_GROUP) @binding(7) var<uniform> cloud_shade: vec4<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(9) var cloud_target: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(10) var cloud_target_sampler: sampler;

// Over every pixel: it's drawn without a depth test, see `CloudMaterial::specialize`.
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

// How far along `direction` the scene drawn at `frag_coord` is, or `FAR` where there's only sky.
fn scene_distance(frag_coord: vec2<f32>, direction: vec3<f32>) -> f32 {
@if(DEPTH_PREPASS && !MULTISAMPLED) {
    let depth = textureLoad(view_bindings::depth_prepass_texture, vec2<i32>(frag_coord), 0);
    if depth > 0.0 {
        let forward = -view_bindings::view.world_from_view[2].xyz;
        return -depth_ndc_to_view_z(depth) / max(dot(direction, forward), 1e-4);
    }
}
    return FAR;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let viewport = view_bindings::view.viewport;
    let marched = textureSampleLevel(cloud_target, cloud_target_sampler, (in.position.xy - viewport.xy) / viewport.zw, 0.0);
    if marched.w < 0.002 {
        return vec4<f32>(0.0);
    }
    let eye = view_bindings::view.world_position.xyz;
    let direction = normalize(in.world_position.xyz - eye);
    if scene_distance(in.position.xy, direction) < cloud_entry(eye, direction, curvature.now.per_square_block) {
        return vec4<f32>(0.0);
    }
    return lit_clouds(marched, eye, direction, cloud_shade.x);
}
