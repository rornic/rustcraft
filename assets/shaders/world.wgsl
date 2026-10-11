#import bevy_pbr::{forward_io::VertexOutput, mesh_view_bindings as view_bindings, pbr_functions::apply_fog}

@group(2) @binding(0) var<uniform> material_color: vec4<f32>;
@group(2) @binding(1) var material_color_texture: texture_2d<f32>;
@group(2) @binding(2) var material_color_sampler: sampler;

@group(2) @binding(3) var<uniform> material_fade: vec4<f32>;

// textures in blocks.png, one per block type but air
const ATLAS_CELLS: f32 = 7.0;
const TILE_INSET: f32 = 0.002;

fn dither_threshold(frag_coord: vec2<f32>) -> f32 {
    var bayer = array<f32, 16>(0.0, 8.0, 2.0, 10.0, 12.0, 4.0, 14.0, 6.0, 3.0, 11.0, 1.0, 9.0, 15.0, 7.0, 13.0, 5.0);
    let p = vec2<u32>(frag_coord) % 4u;
    return (bayer[p.y * 4u + p.x] + 0.5) / 16.0;
}

struct FragmentOutput {
  @location(0) color: vec4<f32>
}

@fragment
fn fragment(
    in: VertexOutput,
) ->  FragmentOutput {

    let world_position = in.world_position.xyz;
    let view_position = view_bindings::view.world_position.xyz;

    let brightness = dot(normalize(in.world_normal), normalize(vec3(-0.2, 0.7, 0.2)));

    // block meshes put tile counts in uv and the atlas cell in vertex alpha, so merged faces repeat their texture
    var atlas_cell = 0.0;
#ifdef VERTEX_COLORS
    atlas_cell = floor(in.color.a + 0.5);
#endif
    // kept just inside the cell: a fract near 1.0 would round into the next block's texture
    let tile_uv = clamp(fract(in.uv), vec2<f32>(TILE_INSET), vec2<f32>(1.0 - TILE_INSET));
    let atlas_uv = vec2<f32>((atlas_cell + tile_uv.x) / ATLAS_CELLS, tile_uv.y);
    let atlas_dx = vec2<f32>(dpdx(in.uv.x) / ATLAS_CELLS, dpdx(in.uv.y));
    let atlas_dy = vec2<f32>(dpdy(in.uv.x) / ATLAS_CELLS, dpdy(in.uv.y));
    let texel = textureSampleGrad(material_color_texture, material_color_sampler, atlas_uv, atlas_dx, atlas_dy);
    let color_lit = material_color * select(texel, vec4<f32>(1.0), in.uv.x < 0.0);

    let dark = color_lit * 0.7;
    var color = mix(dark, color_lit, brightness);

#ifdef VERTEX_COLORS
    color = vec4<f32>(color.rgb * in.color.rgb, color.a);
#endif

    // This shader fully replaces Bevy's standard PBR fragment stage, so fog isn't
    // applied automatically the way it would be for StandardMaterial - apply it
    // explicitly so distant chunks fade into the sky instead of popping in/out.
    color = apply_fog(view_bindings::fog, color, world_position, view_position);

    // after the texture samples above, which need uniform control flow
    let threshold = dither_threshold(in.position.xy);
    if threshold < material_fade.x || threshold >= material_fade.y {
        discard;
    }
    // the holes in leaves: this shader replaces Bevy's, so must cut out alpha-masked texels itself
    if color.a < 0.5 {
        discard;
    }

    var output: FragmentOutput;
    output.color = color;
    return output;
}
