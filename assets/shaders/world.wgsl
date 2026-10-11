#import bevy_pbr::{
    forward_io::VertexOutput,
    mesh_view_bindings as view_bindings,
    mesh_view_types::DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT,
    pbr_functions::apply_fog,
    shadows::fetch_directional_shadow,
}
#import "shaders/block_texture.wgsl"::{block_texel, material_fade}

// Light falling on a face from the open sky, cool against the warm sun: faces up get
// the sky's blue, faces down only what the ground bounces back.
const SKY_AMBIENT: vec3<f32> = vec3<f32>(0.36, 0.46, 0.66);
const GROUND_AMBIENT: vec3<f32> = vec3<f32>(0.2, 0.18, 0.15);
// All that's left with no sky light at all, so caves aren't pitch black.
const CAVE_AMBIENT: vec3<f32> = vec3<f32>(0.02, 0.02, 0.025);
// Above 1, light falls away quickly at first under cover, then more slowly.
const SKY_LIGHT_CURVE: f32 = 2.0;
// Shadow maps only reach so far, so beyond them sunlight is kept out of caves by sky
// light instead: none gets in below this much of it.
const SUN_SKY_GATE: f32 = 0.3;
// Shadows fade out over this last part of their reach, instead of stopping at a line.
const SHADOW_FADE_START: f32 = 0.8;

// Light from the sky for sky light 0..1, see `light.rs`.
fn sky_ambient(sky: f32, normal: vec3<f32>) -> vec3<f32> {
    let hemisphere = mix(GROUND_AMBIENT, SKY_AMBIENT, normal.y * 0.5 + 0.5);
    return CAVE_AMBIENT + hemisphere * pow(clamp(sky, 0.0, 1.0), SKY_LIGHT_CURVE);
}

fn sun_shadow(world_position: vec4<f32>, normal: vec3<f32>) -> f32 {
    let sun = &view_bindings::lights.directional_lights[0];
    if ((*sun).flags & DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT) == 0u {
        return 1.0;
    }
    let view_from_world = view_bindings::view.view_from_world;
    let view_z = dot(vec4<f32>(
        view_from_world[0].z,
        view_from_world[1].z,
        view_from_world[2].z,
        view_from_world[3].z,
    ), world_position);
    let shadow = fetch_directional_shadow(0u, world_position, normal, view_z);
    let reach = (*sun).cascades[(*sun).num_cascades - 1u].far_bound;
    return mix(shadow, 1.0, smoothstep(reach * SHADOW_FADE_START, reach, -view_z));
}

// Sunlight reaching a face: warm, shadowed, and dimmer the more it glances off.
fn sunlight(world_position: vec4<f32>, normal: vec3<f32>, sky: f32) -> vec3<f32> {
    if view_bindings::lights.n_directional_lights == 0u {
        return vec3<f32>(0.0);
    }
    let sun = &view_bindings::lights.directional_lights[0];
    let n_dot_l = dot(normal, (*sun).direction_to_light);
    let reach = n_dot_l * smoothstep(0.0, SUN_SKY_GATE, sky);
    // faces turned away can't be lit, so skip their shadow lookups
    if (*sun).skip != 0u || reach <= 0.0 {
        return vec3<f32>(0.0);
    }
    let radiance = (*sun).color.rgb * view_bindings::view.exposure;
    return radiance * reach * sun_shadow(world_position, normal);
}

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
    let normal = normalize(in.world_normal);

    var vertex_alpha = 0.0;
#ifdef VERTEX_COLORS
    vertex_alpha = in.color.a;
#endif
    var color = block_texel(in.uv, vertex_alpha);

    // ambient occlusion and grass/leaf tint
#ifdef VERTEX_COLORS
    color = vec4<f32>(color.rgb * in.color.rgb, color.a);
#endif

    // block meshes carry smoothed sky light; far smooth terrain is all under open sky
    var sky = 1.0;
#ifdef VERTEX_UVS_B
    sky = in.uv_b.x;
#endif

    let light = sunlight(in.world_position, normal, sky) + sky_ambient(sky, normal);
    color = vec4<f32>(color.rgb * light, color.a);

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
