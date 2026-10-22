// Clouds' shadows on the land: where the sun's rays cross the cloud layer, looked up in
// its pattern, so they drift with the clouds however far out. See `clouds.rs`.

import bevy_pbr::render::mesh_view_bindings as view_bindings;
import super::sky::SUN_LIGHT;
import super::cloud_bindings::{cloud_pattern, cloud_sampler, cloud_layer};

// How much sun a cloud blocks, and the cover its shadow darkens over, matching `cloud_march.wesl`.
const CLOUD_SHADE: f32 = 0.7;
const SHADOW_COVER: vec2<f32> = vec2<f32>(0.35, 0.8);

// How much sunlight reaches `world_position` past the clouds, 0..1.
fn cloud_shadow(world_position: vec3<f32>) -> f32 {
    let to_sun = view_bindings::lights.directional_lights[SUN_LIGHT].direction_to_light;
    let layer = cloud_layer.x;
    if to_sun.y <= 0.0 || world_position.y >= layer {
        return 1.0;
    }
    let under = world_position.xz + to_sun.xz * ((layer - world_position.y) / to_sun.y);
    // `globals.time` wraps on the hour, when the wind has carried the clouds a whole
    // number of patterns, see `clouds::WIND`
    let drift = cloud_layer.zw * view_bindings::globals.time;
    let cover = textureSampleLevel(cloud_pattern, cloud_sampler, (under - drift) / cloud_layer.y, 0.0).r;
    return 1.0 - CLOUD_SHADE * smoothstep(SHADOW_COVER.x, SHADOW_COVER.y, cover);
}
