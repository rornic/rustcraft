// Fog for ChunkMaterial and WaterMaterial, which replace Bevy's PBR fragment stage and so
// must fog themselves.

import bevy_pbr::render::{mesh_view_bindings as view_bindings, pbr_functions::apply_fog};
import super::sky::sky_colour;

// Slope above the horizon where the sky is read for fog, since far land meets the sky just under it.
const HORIZON_RISE: f32 = 0.02;

// `color` at `world_position` behind the camera's fog. Above water it thickens only with
// distance across the land, not up or down, so it hides the far edge of the world without
// greying out the ground below when flying high: easing in to half fogged at the linear
// fog's start, where the farthest LOD level begins, then on to fully fogged at its end.
// Underwater, Bevy's linear murk closes in every way. `WATER_LEVEL` comes from the
// material's `specialize`.
fn fogged(color: vec4<f32>, world_position: vec3<f32>, frag_coord: vec2<f32>) -> vec4<f32> {
    let fog = view_bindings::fog;
    let view_position = view_bindings::view.world_position.xyz;
    if view_position.y < f32(constants::WATER_LEVEL) {
        return apply_fog(fog, color, world_position, view_position, frag_coord);
    }
    let half = fog.be.x;
    let end = max(fog.be.y, half + 1.0);
    let across = world_position.xz - view_position.xz;
    let distance = length(across);
    let sky = sky_colour(normalize(vec3<f32>(across.x, HORIZON_RISE * distance, across.y)));
    let near = distance / half;
    let far = clamp((distance - half) / (end - half), 0.0, 1.0);
    let amount = select(mix(0.5, 1.0, far), 0.5 * near * near, distance < half);
    return vec4<f32>(mix(color.rgb, sky, amount * fog.base_color.a), color.a);
}
