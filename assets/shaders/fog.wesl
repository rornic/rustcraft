// Fog for ChunkMaterial, WaterMaterial and CloudMaterial, which replace Bevy's PBR fragment
// stage and so must fog themselves. Each carries the curvature buffer, see `horizon.wesl`.

import bevy_pbr::render::{mesh_view_bindings as view_bindings, pbr_functions::apply_fog};
import super::horizon::horizon_haze;

// Over this last share of the way to the fog's end, the land fades out altogether into the
// haze just over its edge, past the fog's opacity: the square edge of the farthest LOD nodes
// never shows against the sky, and nodes coming into reach fade in as the fog eases out.
const EDGE_FADE_START: f32 = 0.9;

// `color` at `world_position` behind the camera's fog. Above water it thickens only with
// distance across the land, not up or down, so it hides the far edge of the world in the
// haze along the horizon without greying out the ground below when flying high: easing in
// to half fogged at the linear fog's start, where the farthest LOD level begins, then on to
// fully fogged at its end.
// Underwater, Bevy's linear murk closes in every way. `WATER_LEVEL` comes from the
// material's `specialize`.
fn fogged(color: vec4<f32>, world_position: vec3<f32>, frag_coord: vec2<f32>) -> vec4<f32> {
    let fog = view_bindings::fog;
    let view_position = view_bindings::view.world_position.xyz;
    // under a river or lake too, over the sea's level: only the underwater fog starts at the camera
    if view_position.y < f32(constants::WATER_LEVEL) || fog.be.x <= 0.0 {
        return apply_fog(fog, color, world_position, view_position, frag_coord);
    }
    let half = fog.be.x;
    let end = max(fog.be.y, half + 1.0);
    let across = world_position.xz - view_position.xz;
    let distance = length(across);
    let sky = horizon_haze(across);
    let near = distance / half;
    let far = clamp((distance - half) / (end - half), 0.0, 1.0);
    let amount = select(mix(0.5, 1.0, far), 0.5 * near * near, distance < half);
    let opacity = mix(amount * fog.base_color.a, 1.0, smoothstep(EDGE_FADE_START, 1.0, distance / end));
    return vec4<f32>(mix(color.rgb, sky, opacity), color.a);
}
