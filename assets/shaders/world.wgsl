#import bevy_pbr::{
    forward_io::{Vertex, VertexOutput},
    mesh_functions,
    mesh_view_bindings as view_bindings,
    mesh_view_types::DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT,
    shadows::fetch_directional_shadow,
    view_transformations::{position_world_to_clip, position_world_to_view},
}
#import "shaders/block_texture.wgsl"::{block_mean, block_texel, material_fade}
#import "shaders/foliage.wgsl"::{is_leaf, swayed}
#import "shaders/fog.wgsl"::fogged
#import "shaders/lighting.wgsl"::{direct_sun, open_sky}
#import "shaders/cloud_shadow.wgsl"::cloud_shadow
#import "shaders/dither.wgsl"::dither_threshold
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
    return CAVE_AMBIENT + open_sky(normal) * pow(clamp(sky, 0.0, 1.0), SKY_LIGHT_CURVE);
}

fn sun_shadow(world_position: vec4<f32>, normal: vec3<f32>) -> f32 {
    let sun = &view_bindings::lights.directional_lights[0];
    if ((*sun).flags & DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT) == 0u {
        return 1.0;
    }
    let view_z = position_world_to_view(world_position.xyz).z;
    let shadow = fetch_directional_shadow(0u, world_position, normal, view_z);
    let reach = (*sun).cascades[(*sun).num_cascades - 1u].far_bound;
    return mix(shadow, 1.0, smoothstep(reach * SHADOW_FADE_START, reach, -view_z));
}

// Sunlight reaching a face, shadowed by the land and the clouds.
fn sunlight(world_position: vec4<f32>, normal: vec3<f32>, sky: f32) -> vec3<f32> {
    let light = direct_sun(normal) * smoothstep(0.0, SUN_SKY_GATE, sky);
    // faces it can't reach need no shadow lookups
    if all(light == vec3<f32>(0.0)) {
        return light;
    }
    return light * sun_shadow(world_position, normal) * cloud_shadow(world_position.xyz);
}

// Bevy's mesh vertex shader, for meshes that are never skinned or morphed, with leaves
// swaying in the wind. Block meshes always carry vertex colours, see `MeshBuffers`.
@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    out.world_position = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(vertex.position, 1.0));
    out.world_position = vec4<f32>(swayed(out.world_position.xyz, vertex.color.a, view_bindings::globals.time), 1.0);
    out.color = vertex.color;
    out.position = position_world_to_clip(out.world_position.xyz);
    out.world_normal = mesh_functions::mesh_normal_local_to_world(vertex.normal, vertex.instance_index);
    out.uv = vertex.uv;
#ifdef VERTEX_UVS_B
    out.uv_b = vertex.uv_b;
#endif
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = vertex.instance_index;
#endif
#ifdef VISIBILITY_RANGE_DITHER
    out.visibility_range_dither = mesh_functions::get_visibility_range_dither_level(vertex.instance_index, world_from_local[3]);
#endif
    return out;
}

struct FragmentOutput {
  @location(0) color: vec4<f32>
}

@fragment
fn fragment(
    in: VertexOutput,
) ->  FragmentOutput {

    let world_position = in.world_position.xyz;
    let normal = normalize(in.world_normal);

    var color = block_texel(in.uv, in.color.a);
#ifdef VERTEX_UVS_B
    // coarse LOD cells, where a stretched texture would read as one giant block. Leaves
    // come out solid, and their shadows match, see `world_prepass.wgsl`.
    if in.uv_b.y > 0.5 {
        color = block_mean(in.color.a);
    }
#endif
#ifdef SOLID_LEAVES
    // `DebugSettings::solid_leaves`: leaves without their holes, as coarse LODs draw them
    if is_leaf(in.color.a) {
        color = block_mean(in.color.a);
    }
#endif
    // ambient occlusion and grass/leaf tint
    color = vec4<f32>(color.rgb * in.color.rgb, color.a);

    // block meshes carry smoothed sky light, all but those with no faces at all
    var sky = 1.0;
#ifdef VERTEX_UVS_B
    sky = in.uv_b.x;
#endif

    let light = sunlight(in.world_position, normal, sky) + sky_ambient(sky, normal);
    color = vec4<f32>(color.rgb * light, color.a);

    // so distant chunks fade into the sky instead of popping in and out
    color = fogged(color, world_position);

    // after the texture samples above, which need uniform control flow
    let threshold = dither_threshold(in.position.xy);
    if threshold < material_fade.x || threshold >= material_fade.y {
        discard;
    }
    // the holes in leaves: this shader replaces Bevy's, so must cut out alpha-masked texels itself
#ifndef SOLID_LEAVES
    if color.a < 0.5 {
        discard;
    }
#endif

    var output: FragmentOutput;
    output.color = color;
    return output;
}
