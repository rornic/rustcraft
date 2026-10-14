import bevy_pbr::render::{
    forward_io::{Vertex, VertexOutput, decompress_vertex},
    mesh_functions,
    mesh_view_bindings as view_bindings,
    mesh_view_types::DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT,
    shadows::fetch_directional_shadow,
    view_transformations::{position_world_to_clip, position_world_to_view},
};
import super::block_texture::{block_mean, block_texel, material_fade};
import super::foliage::{is_leaf, swayed};
import super::fog::fogged;
import super::lighting::{direct_sun, open_sky};
import super::cloud_shadow::cloud_shadow;
import super::lod_patch::patch_tint;
import super::dither::dither_threshold;
// How bright the leaves seen through a solid tree box's holes are, to its surface's.
const HOLE_SHADE: f32 = 0.55;
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

fn sun_shadow(world_position: vec4<f32>, normal: vec3<f32>, frag_coord: vec2<f32>) -> f32 {
    let sun = &view_bindings::lights.directional_lights[0];
    if ((*sun).flags & DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT) == 0u {
        return 1.0;
    }
    let view_z = position_world_to_view(world_position.xyz).z;
    let shadow = fetch_directional_shadow(0u, world_position, normal, view_z, frag_coord);
    let reach = (*sun).cascades[(*sun).num_cascades - 1u].far_bound;
    return mix(shadow, 1.0, smoothstep(reach * SHADOW_FADE_START, reach, -view_z));
}

// Sunlight reaching a face, shadowed by the land and the clouds.
fn sunlight(world_position: vec4<f32>, normal: vec3<f32>, shaded: vec3<f32>, sky: f32, frag_coord: vec2<f32>) -> vec3<f32> {
    let light = direct_sun(shaded) * smoothstep(0.0, SUN_SKY_GATE, sky);
    // faces it can't reach need no shadow lookups
    if all(light == vec3<f32>(0.0)) {
        return light;
    }
    return light * sun_shadow(world_position, normal, frag_coord) * cloud_shadow(world_position.xyz);
}

// Faces turn toward up between these distances in blocks, to this much of the way: past
// where a step is under a pixel, lit tops and dark sides shimmer into each other. LOD cells are
// a few blocks wide well before then, so it starts where shadows end, the default
// `RendererSettings::shadow_distance`, and the land is lit block by block as far as it's shadowed.
const FLATTEN_START: f32 = 1024.0;
const FLATTEN_END: f32 = 4096.0;
const FLATTEN_MAX: f32 = 0.85;

// `Tile::GrassTop`'s atlas cell
const GRASS_TOP_CELL: f32 = 1.0;
// Blocks per pixel at which grass sides start turning to the grass on top, and are wholly it:
// four pixels tall, as the tile's green fringe is about a quarter of it, to under two.
const GRASS_SIDE_FADE_START: f32 = 0.25;
const GRASS_SIDE_FADE_END: f32 = 0.6;

// The normal to light by: turned toward up with distance, which evens out the sun's and sky's contrast between faces.
fn lit_normal(normal: vec3<f32>, world_position: vec3<f32>) -> vec3<f32> {
    let flatten = FLATTEN_MAX * smoothstep(FLATTEN_START, FLATTEN_END, distance(world_position, view_bindings::view.world_position));
    return normalize(mix(normal, vec3<f32>(0.0, 1.0, 0.0), flatten));
}

// Bevy's mesh vertex shader, for meshes that are never skinned or morphed, with leaves
// swaying in the wind. Block meshes always carry vertex colours, see `MeshBuffers`.
@vertex
fn vertex(vertex_in: Vertex) -> VertexOutput {
    let vertex = decompress_vertex(vertex_in, vertex_in.instance_index);
    var out: VertexOutput;
    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    out.world_position = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(vertex.position, 1.0));
    var coarse = false;
@if(VERTEX_UVS_B) {
    coarse = vertex.uv_b.y > 0.5;
}
    out.world_position = vec4<f32>(swayed(out.world_position.xyz, vertex.color.a, coarse, view_bindings::globals.time), 1.0);
    out.color = vertex.color;
    out.position = position_world_to_clip(out.world_position.xyz);
    // a mesh fading out sits a little farther than it is, so what replaces it wins where they coincide
    out.position.z *= 1.0 - material_fade.z;
    out.world_normal = mesh_functions::mesh_normal_local_to_world(vertex.normal, vertex.instance_index);
    out.uv = vertex.uv;
@if(VERTEX_UVS_B) {
    out.uv_b = vertex.uv_b;
}
@if(VERTEX_OUTPUT_INSTANCE_INDEX) {
    out.instance_index = vertex.instance_index;
}
@if(VISIBILITY_RANGE_DITHER) {
    out.visibility_range_dither = mesh_functions::get_visibility_range_dither_level(vertex.instance_index, world_from_local[3]);
}
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
    // blocks per pixel up the face, worked out here as derivatives need uniform control flow
    let block_rate = length(vec2<f32>(dpdx(in.uv.y), dpdy(in.uv.y)));
    // coarse LOD cells, where a stretched texture would read as one giant block, and wide
    // tree boxes, which keep it: leaves come out solid, and their shadows match, see
    // `world_prepass.wesl`. How a face draws is `generator::TEXTURED` and so on.
@if(VERTEX_UVS_B) {
    if in.uv_b.y > 0.875 {
        let mean = block_mean(in.color.a);
        color = vec4<f32>(mean.rgb * patch_tint(world_position), mean.a);
    } else if in.uv_b.y > 0.5 {
        // the holes show deeper leaves, in their shade
        let deeper = block_mean(in.color.a).rgb * HOLE_SHADE;
        color = vec4<f32>(mix(deeper, color.rgb, smoothstep(0.3, 0.7, color.a)), 1.0);
    }
}
    // `DebugSettings::solid_leaves`: leaves without their holes, as coarse LODs draw them
@if(SOLID_LEAVES) {
    if is_leaf(in.color.a) {
        color = block_mean(in.color.a);
    }
}
    // ambient occlusion and grass/leaf tint
    var tinted = color.rgb * in.color.rgb;
@if(VERTEX_UVS_B) {
    // grass sides carry the grass's tint for their top, and their shade alone for their tile,
    // see `MeshBuffers::tint_grass_sides`: once blocks are a few pixels tall their tile averages
    // out to mostly dirt, while the grass on top of them would fill the pixel, so they turn to it
    if in.uv_b.y < 0.0 {
        let grass_top = block_mean(GRASS_TOP_CELL).rgb * in.color.rgb;
        tinted = mix(color.rgb * -in.uv_b.y, grass_top, smoothstep(GRASS_SIDE_FADE_START, GRASS_SIDE_FADE_END, block_rate));
    }
}
    color = vec4<f32>(tinted, color.a);

    // block meshes carry smoothed sky light, all but those with no faces at all
    var sky = 1.0;
@if(VERTEX_UVS_B) {
    sky = in.uv_b.x;
}

    let shaded = lit_normal(normal, world_position);
    let light = sunlight(in.world_position, normal, shaded, sky, in.position.xy) + sky_ambient(sky, shaded);
    color = vec4<f32>(color.rgb * light, color.a);

    // so distant chunks fade into the sky instead of popping in and out
    color = fogged(color, world_position, in.position.xy);

    // after the texture samples above, which need uniform control flow
    let threshold = dither_threshold(in.position.xy);
    if threshold < material_fade.x || threshold >= material_fade.y {
        discard;
    }
    // the holes in leaves: this shader replaces Bevy's, so must cut out alpha-masked texels itself
@if(!SOLID_LEAVES) {
    if color.a < 0.5 {
        discard;
    }
}

    var output: FragmentOutput;
    output.color = color;
    return output;
}
