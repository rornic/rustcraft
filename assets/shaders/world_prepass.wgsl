// ChunkMaterial's depth-only passes, chiefly the sun's shadow maps: Bevy's own prepass
// shader can't see the atlas, so leaves would cast solid shadows without this.

#import bevy_pbr::prepass_io
#import "shaders/block_texture.wgsl"::block_texel

fn cut_leaf_holes(in: prepass_io::VertexOutput) {
    var vertex_alpha = 0.0;
#ifdef VERTEX_COLORS
    vertex_alpha = in.color.a;
#endif
    if block_texel(in.uv, vertex_alpha).a < 0.5 {
        discard;
    }
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
