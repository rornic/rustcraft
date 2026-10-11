// Clouds' shadows on the land: where the sun's rays cross the cloud layer, looked up in
// its pattern, so they drift with the clouds however far out. See `clouds.rs`.

#import bevy_pbr::mesh_view_bindings as view_bindings

@group(2) @binding(4) var cloud_pattern: texture_2d<f32>;
@group(2) @binding(5) var cloud_sampler: sampler;
// the middle of the layer's height, the pattern's width in blocks, and the wind
@group(2) @binding(6) var<uniform> cloud_layer: vec4<f32>;

// How much of the sun a cloud keeps off the ground.
const CLOUD_SHADE: f32 = 0.7;

// How much sunlight reaches `world_position` past the clouds, 0..1.
fn cloud_shadow(world_position: vec3<f32>) -> f32 {
    let to_sun = view_bindings::lights.directional_lights[0].direction_to_light;
    let layer = cloud_layer.x;
    if to_sun.y <= 0.0 || world_position.y >= layer {
        return 1.0;
    }
    let under = world_position.xz + to_sun.xz * ((layer - world_position.y) / to_sun.y);
    // `globals.time` wraps on the hour, when the wind has carried the clouds a whole
    // number of patterns, see `clouds::WIND`
    let drift = cloud_layer.zw * view_bindings::globals.time;
    let cover = textureSampleLevel(cloud_pattern, cloud_sampler, (under - drift) / cloud_layer.y, 0.0).r;
    return 1.0 - CLOUD_SHADE * cover;
}
