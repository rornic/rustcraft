// The sun and sky's light, shared by everything lit outdoors.

import bevy_pbr::render::mesh_view_bindings as view_bindings;
import bevy_pbr::atmosphere::{
    bruneton_functions::transmittance_lut_r_mu_to_uv, functions::calculate_visible_sun_ratio,
};
import super::sky::sky_irradiance;

// Matches `SKY_GAIN` in `sky.rs`.
const SKY_GAIN: f32 = 4.0;
// Matches `SUN_DIRECTION.y` in `sky.rs`: how high the sun is at the time of day the world's
// unlit colours were tuned for, see `daylight`.
const TUNED_SUN_HEIGHT: f32 = 0.8;

// Light falling on a face from the open sky, cool against the warm sun: faces up get
// the sky's blue, faces down only what the ground bounces back.
// How much of the sky's irradiance lights a face, for the look rather than physics.
const SKY_AMBIENT_GAIN: f32 = 1.0;
const GROUND_AMBIENT: vec3<f32> = vec3<f32>(0.2, 0.18, 0.15);

// Light from the whole sky on a face turned to `normal`, out in the open.
fn open_sky(normal: vec3<f32>) -> vec3<f32> {
    return mix(GROUND_AMBIENT, sky_irradiance(normal) * SKY_AMBIENT_GAIN, normal.y * 0.5 + 0.5);
}

// How much of the sun's light gets through the air to `world_position`, by colour, as a
// share of what gets through with the sun overhead: so midday is as the sun's tuned, and it
// dims and reddens as the sun sinks, and goes once it's set, as Bevy's own materials light.
// Read at the point's real height, the planet's ground at y = 0, not from the raised view
// `sky::match_sky_horizon` gives the sky.
fn sun_transmittance(world_position: vec3<f32>) -> vec3<f32> {
@if(ATMOSPHERE) {
    let atmosphere = view_bindings::atmosphere;
    let sun = &view_bindings::lights.directional_lights[0];
    let r = atmosphere.inner_radius + max(world_position.y, 0.0);
    let mu = (*sun).direction_to_light.y;
    let through = transmittance(atmosphere, r, mu) * calculate_visible_sun_ratio(atmosphere, r, mu, (*sun).sun_disk_angular_size);
    return through / max(transmittance(atmosphere, r, 1.0), vec3<f32>(1e-4));
}
@if(!ATMOSPHERE) {
    return vec3<f32>(1.0);
}
}

@if(ATMOSPHERE)
fn transmittance(atmosphere: bevy_pbr::atmosphere::types::Atmosphere, r: f32, mu: f32) -> vec3<f32> {
    let uv = transmittance_lut_r_mu_to_uv(atmosphere, r, mu);
    return textureSampleLevel(view_bindings::atmosphere_transmittance_texture, view_bindings::atmosphere_transmittance_sampler, uv, 0.0).rgb;
}

// The sun's light reaching `world_position`, in scene colour. Nothing when there's no sun.
fn sun_radiance(world_position: vec3<f32>) -> vec3<f32> {
    if view_bindings::lights.n_directional_lights == 0u {
        return vec3<f32>(0.0);
    }
    let sun = &view_bindings::lights.directional_lights[0];
    return (*sun).color.rgb * view_bindings::view.exposure / SKY_GAIN * sun_transmittance(world_position);
}

// Unshadowed sunlight on a face at `world_position` turned to `normal`: dimmer the more it
// glances off, and nothing when it faces away.
fn direct_sun(normal: vec3<f32>, world_position: vec3<f32>) -> vec3<f32> {
    if view_bindings::lights.n_directional_lights == 0u {
        return vec3<f32>(0.0);
    }
    let to_sun = view_bindings::lights.directional_lights[0].direction_to_light;
    return sun_radiance(world_position) * max(dot(normal, to_sun), 0.0);
}

// How lit a face turned up at `world_position` is now, by colour, as a share of how lit it
// was with the sun where the world's unlit colours, like water's own, were tuned: 1 then,
// dimming and reddening toward sunset, so those colours follow the time of day too.
fn daylight(world_position: vec3<f32>) -> vec3<f32> {
    let up = vec3<f32>(0.0, 1.0, 0.0);
    let sky = open_sky(up);
    var tuned = sky;
    if view_bindings::lights.n_directional_lights > 0u {
        let sun = &view_bindings::lights.directional_lights[0];
        tuned += (*sun).color.rgb * view_bindings::view.exposure / SKY_GAIN * TUNED_SUN_HEIGHT;
    }
    return (direct_sun(up, world_position) + sky) / max(tuned, vec3<f32>(1e-4));
}

// Clouds scatter the light that falls on them about inside, so most of what lights them,
// shaded sides and all, is sunlight come through them, and their colour follows the sun's:
// white at midday, glowing orange and pink as it sets. On top, sunlight straight on, a
// little softened, and some of the sky's light. Weighted so midday is much as before
// sunlight came through them.
const CLOUD_SUN_DIRECT: f32 = 0.8;
const CLOUD_SUN_THROUGH: f32 = 0.5;
const CLOUD_SKY: f32 = 0.6;

// Light on a cloud's face at `world_position` turned to `normal`, for the clouds and their
// reflections in the water.
fn cloud_light(normal: vec3<f32>, world_position: vec3<f32>) -> vec3<f32> {
    let sun = direct_sun(normal, world_position) * CLOUD_SUN_DIRECT + sun_radiance(world_position) * CLOUD_SUN_THROUGH;
    return sun + open_sky(normal) * CLOUD_SKY;
}
