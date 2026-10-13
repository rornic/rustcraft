// ChunkMaterial's depth-only passes, chiefly the sun's shadow maps: Bevy's own prepass
// shader can't see the atlas, so leaves would cast solid shadows without this.

import bevy_pbr::{
    prepass::{bindings as prepass_bindings, io as prepass_io},
    render::{mesh_functions, mesh_view_bindings::view, view_transformations::position_world_to_clip},
};
import bevy_render::globals::Globals;
import super::block_texture::{block_alpha, material_fade};
import super::dither::dither_threshold;
import super::foliage::{is_leaf, swayed};

// the prepass's view bindings are the view and then the frame's globals, see Bevy's `PrepassPipeline`
@group(0) @binding(1) var<uniform> globals: Globals;

// Bevy's prepass vertex shader, for meshes that are never skinned or morphed, with leaves
// swaying just as in the main pass (the sway isn't in the motion vectors). Block meshes always carry
// vertex colours, see `MeshBuffers`.
@vertex
fn vertex(vertex_in: prepass_io::Vertex) -> prepass_io::VertexOutput {
    let vertex = prepass_io::decompress_vertex(vertex_in, vertex_in.instance_index);
    var out: prepass_io::VertexOutput;
    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    out.world_position = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(vertex.position, 1.0));
    var coarse = false;
@if(VERTEX_UVS_B) {
    coarse = vertex.uv_b.y > 0.5;
}
    out.world_position = vec4<f32>(swayed(out.world_position.xyz, vertex.color.a, coarse, globals.time), 1.0);
    out.color = vertex.color;
    out.position = position_world_to_clip(out.world_position.xyz);
    // a mesh fading out sits a little farther than it is, as in the main pass, see `fade.rs`
@if(!UNCLIPPED_DEPTH_ORTHO_EMULATION) {
    out.position.z *= 1.0 - material_fade.z;
}
@if(UNCLIPPED_DEPTH_ORTHO_EMULATION) {
    out.unclipped_depth = out.position.z;
    out.position.z = min(out.position.z, 1.0);
}
@if(VERTEX_UVS_A) {
    out.uv = vertex.uv;
}
@if(VERTEX_UVS_B) {
    out.uv_b = vertex.uv_b;
}
@if(NORMAL_PREPASS_OR_DEFERRED_PREPASS) {
    out.world_normal = mesh_functions::mesh_normal_local_to_world(vertex.normal, vertex.instance_index);
}
@if(MOTION_VECTOR_PREPASS) {
    let previous_world_from_local = mesh_functions::get_previous_world_from_local(vertex.instance_index);
    let previous = mesh_functions::mesh_position_local_to_world(previous_world_from_local, vec4<f32>(vertex.position, 1.0));
    out.previous_world_position = vec4<f32>(swayed(previous.xyz, vertex.color.a, coarse, globals.time), 1.0);
}
@if(VERTEX_OUTPUT_INSTANCE_INDEX) {
    out.instance_index = vertex.instance_index;
}
    return out;
}

// Only leaves have holes, so only they pay for a texture sample. Untextured LOD cells
// draw leaves solid, so cast solid shadows too.
fn cut_leaf_holes(in: prepass_io::VertexOutput) {
// `DebugSettings::solid_leaves` casts solid shadows everywhere
@if(!SOLID_LEAVES) {
@if(VERTEX_UVS_B) {
    if in.uv_b.y > 0.5 {
        return;
    }
}
    if is_leaf(in.color.a) && block_alpha(in.uv, in.color.a) < 0.5 {
        discard;
    }
}
}

// The view's depth is dithered just as the main pass is, so a pixel a fading mesh gives up
// doesn't keep its depth, which would reject whatever is behind it. Shadows aren't: dithered
// in light space they would only flicker.
fn dither_fade(in: prepass_io::VertexOutput) {
@if(!UNCLIPPED_DEPTH_ORTHO_EMULATION) {
    let threshold = dither_threshold(in.position.xy);
    if threshold < material_fade.x || threshold >= material_fade.y {
        discard;
    }
}
}

@if(PREPASS_FRAGMENT)
@fragment
fn fragment(in: prepass_io::VertexOutput) -> prepass_io::FragmentOutput {
    dither_fade(in);
    cut_leaf_holes(in);

    var out: prepass_io::FragmentOutput;
@if(NORMAL_PREPASS) {
    out.normal = vec4<f32>(normalize(in.world_normal) * 0.5 + vec3<f32>(0.5), 1.0);
}
    // the sun's shadow maps clamp depth so casters behind their near plane still count
@if(UNCLIPPED_DEPTH_ORTHO_EMULATION) {
    out.frag_depth = in.unclipped_depth;
}
@if(MOTION_VECTOR_PREPASS) {
    let clip = view.unjittered_clip_from_world * in.world_position;
    let previous_clip = prepass_bindings::previous_view_uniforms.unjittered_clip_from_world * in.previous_world_position;
    out.motion_vector = (clip.xy / clip.w - previous_clip.xy / previous_clip.w) * vec2(0.5, -0.5);
}
    return out;
}
@else
@fragment
fn fragment(in: prepass_io::VertexOutput) {
    dither_fade(in);
    cut_leaf_holes(in);
}
