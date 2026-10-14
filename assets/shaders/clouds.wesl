// Clouds: lit by the same sun and sky as the land, and fogged with it, see `clouds.rs`.

import bevy_pbr::render::{forward_io::VertexOutput, mesh_view_bindings as view_bindings};
import super::fog::fogged;
import super::lighting::{direct_sun, open_sky};
import super::dither::dither_threshold;

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> cloud_color: vec4<f32>;
// the clouds' base and greatest height
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var<uniform> cloud_shape: vec4<f32>;

// Clouds scatter the light that falls on them, so their shaded sides are far brighter
// than solid ground's would be and the sunlit ones a little softer.
const SUN_SCATTER: f32 = 0.8;
const SKY_SCATTER: f32 = 1.4;
// How much light reaches a cloud's base, through the cloud above it, to its top's.
const BASE_LIGHT: f32 = 0.8;
// Clouds fade away this close to the camera, in blocks, so flying through one doesn't
// leave the view a wall of white.
const NEAR_FADE: vec2<f32> = vec2<f32>(8.0, 40.0);

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let distance = length(in.world_position.xyz - view_bindings::view.world_position.xyz);
    if dither_threshold(in.position.xy) >= smoothstep(NEAR_FADE.x, NEAR_FADE.y, distance) {
        discard;
    }
    let normal = normalize(in.world_normal);
    let up = clamp((in.world_position.y - cloud_shape.x) / cloud_shape.y, 0.0, 1.0);
    let light = (direct_sun(normal) * SUN_SCATTER + open_sky(normal) * SKY_SCATTER) * mix(BASE_LIGHT, 1.0, up);
    return fogged(vec4<f32>(cloud_color.rgb * light, 1.0), in.world_position.xyz, in.position.xy);
}
