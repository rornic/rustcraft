// ChunkMaterial's depth-only passes, chiefly the sun's shadow maps: Bevy's own prepass
// shader can't see the atlas, so leaves would cast solid shadows without this.

#import bevy_pbr::{mesh_functions, prepass_io, view_transformations::position_world_to_clip}
#import bevy_render::globals::Globals
#import "shaders/block_texture.wgsl"::block_alpha
#import "shaders/foliage.wgsl"::{is_leaf, swayed}

// the prepass's view bindings are the view and then the frame's globals, see Bevy's `PrepassPipeline`
@group(0) @binding(1) var<uniform> globals: Globals;

// Bevy's prepass vertex shader, for meshes that are never skinned or morphed and with no
// motion vectors, with leaves swaying just as in the main pass. Block meshes always carry
// vertex colours, see `MeshBuffers`.
@vertex
fn vertex(vertex: prepass_io::Vertex) -> prepass_io::VertexOutput {
    var out: prepass_io::VertexOutput;
    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    out.world_position = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(vertex.position, 1.0));
    var coarse = false;
#ifdef VERTEX_UVS_B
    coarse = vertex.uv_b.y > 0.5;
#endif
    out.world_position = vec4<f32>(swayed(out.world_position.xyz, vertex.color.a, coarse, globals.time), 1.0);
    out.color = vertex.color;
    out.position = position_world_to_clip(out.world_position.xyz);
#ifdef DEPTH_CLAMP_ORTHO
    out.clip_position_unclamped = out.position;
    out.position.z = min(out.position.z, 1.0);
#endif
#ifdef VERTEX_UVS_A
    out.uv = vertex.uv;
#endif
#ifdef VERTEX_UVS_B
    out.uv_b = vertex.uv_b;
#endif
#ifdef NORMAL_PREPASS_OR_DEFERRED_PREPASS
    out.world_normal = mesh_functions::mesh_normal_local_to_world(vertex.normal, vertex.instance_index);
#endif
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = vertex.instance_index;
#endif
    return out;
}

// Only leaves have holes, so only they pay for a texture sample. Untextured LOD cells
// draw leaves solid, so cast solid shadows too.
fn cut_leaf_holes(in: prepass_io::VertexOutput) {
// `DebugSettings::solid_leaves` casts solid shadows everywhere
#ifndef SOLID_LEAVES
#ifdef VERTEX_UVS_B
    if in.uv_b.y > 0.5 {
        return;
    }
#endif
    if is_leaf(in.color.a) && block_alpha(in.uv, in.color.a) < 0.5 {
        discard;
    }
#endif
}

// Unlike the main pass, no dither fade: a fading chunk and the one it's replacing are
// the same shape, and shadows dithered in light space would only flicker.
#ifdef PREPASS_FRAGMENT
@fragment
fn fragment(in: prepass_io::VertexOutput) -> prepass_io::FragmentOutput {
    cut_leaf_holes(in);

    var out: prepass_io::FragmentOutput;
#ifdef NORMAL_PREPASS
    out.normal = vec4<f32>(normalize(in.world_normal) * 0.5 + vec3<f32>(0.5), 1.0);
#endif
    // the sun's shadow maps clamp depth so casters behind their near plane still count
#ifdef DEPTH_CLAMP_ORTHO
    out.frag_depth = in.clip_position_unclamped.z;
#endif
    return out;
}
#else
@fragment
fn fragment(in: prepass_io::VertexOutput) {
    cut_leaf_holes(in);
}
#endif
