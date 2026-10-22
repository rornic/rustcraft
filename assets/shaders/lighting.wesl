// The sun and sky's light, shared by everything lit outdoors.

import bevy_pbr::render::mesh_view_bindings as view_bindings;
import bevy_pbr::atmosphere::{
    bruneton_functions::transmittance_lut_r_mu_to_uv, functions::calculate_visible_sun_ratio,
};
import super::sky::{MOON_LIGHT, SUN_LIGHT, night_factor, sky_irradiance};

// Matches `SKY_GAIN` in `sky.rs`.
const SKY_GAIN: f32 = 4.0;
// Matches `SUN_DIRECTION.y` in `sky.rs`: how high the sun is at the time of day the world's
// unlit colours were tuned for, see `daylight`.
const TUNED_SUN_HEIGHT: f32 = 0.8;

// Moonlight in the shaders is the moon's light boosted by this, see `MOON_BOOST` in `sky.rs`.
const MOON_BOOST: f32 = 400.0;
// Light from the stars on a night with no moon, and the share of moonlight that falls
// everywhere, as the sky scatters it.
const STARLIGHT: vec3<f32> = vec3<f32>(0.004, 0.007, 0.016);
const MOONLIGHT_FILL: f32 = 0.25;

// How much of the moon is lit, 0..1: full when it's opposite the sun.
fn moon_phase() -> f32 {
    if view_bindings::lights.n_directional_lights < 2u {
        return 0.0;
    }
    return 0.5 - 0.5 * dot(view_bindings::lights.directional_lights[SUN_LIGHT].direction_to_light, view_bindings::lights.directional_lights[MOON_LIGHT].direction_to_light);
}

// The moon's light, by phase, fading as it nears the horizon. Nothing when there's no moon.
fn moon_radiance() -> vec3<f32> {
    if view_bindings::lights.n_directional_lights < 2u {
        return vec3<f32>(0.0);
    }
    let moon = &view_bindings::lights.directional_lights[MOON_LIGHT];
    let risen = smoothstep(-0.05, 0.2, (*moon).direction_to_light.y);
    return (*moon).color.rgb * view_bindings::view.exposure / SKY_GAIN * MOON_BOOST * moon_phase() * risen;
}

// Unshadowed moonlight on a face turned to `normal`.
fn direct_moon(normal: vec3<f32>) -> vec3<f32> {
    if view_bindings::lights.n_directional_lights < 2u {
        return vec3<f32>(0.0);
    }
    return moon_radiance() * max(dot(normal, view_bindings::lights.directional_lights[MOON_LIGHT].direction_to_light), 0.0);
}

// Light falling on a face from the open sky, cool against the warm sun: faces up get
// the sky's blue, faces down only what the ground bounces back, which goes with the sun.
// How much of the sky's irradiance lights a face, for the look rather than physics.
const SKY_AMBIENT_GAIN: f32 = 1.0;
const GROUND_AMBIENT: vec3<f32> = vec3<f32>(0.2, 0.18, 0.15);

// Light from the whole sky on a face turned to `normal`, out in the open.
fn open_sky(normal: vec3<f32>) -> vec3<f32> {
    let night = night_factor();
    let ground = GROUND_AMBIENT * (1.0 - night);
    let dark = night * (STARLIGHT + moon_radiance() * MOONLIGHT_FILL);
    return mix(ground, sky_irradiance(normal) * SKY_AMBIENT_GAIN, normal.y * 0.5 + 0.5) + dark;
}

// How much of the sun's light gets through the air to `world_position`, by colour, as a
// share of what gets through with the sun overhead: so midday is as the sun's tuned, and it
// dims and reddens as the sun sinks, and goes once it's set, as Bevy's own materials light.
// Read at the point's real height, the planet's ground at y = 0, not from the raised view
// `sky::match_sky_horizon` gives the sky.
fn sun_transmittance(world_position: vec3<f32>) -> vec3<f32> {
@if(ATMOSPHERE) {
    let atmosphere = view_bindings::atmosphere;
    let sun = &view_bindings::lights.directional_lights[SUN_LIGHT];
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
    let sun = &view_bindings::lights.directional_lights[SUN_LIGHT];
    return (*sun).color.rgb * view_bindings::view.exposure / SKY_GAIN * sun_transmittance(world_position);
}

// Unshadowed sunlight on a face at `world_position` turned to `normal`: dimmer the more it
// glances off, and nothing when it faces away.
fn direct_sun(normal: vec3<f32>, world_position: vec3<f32>) -> vec3<f32> {
    if view_bindings::lights.n_directional_lights == 0u {
        return vec3<f32>(0.0);
    }
    let to_sun = view_bindings::lights.directional_lights[SUN_LIGHT].direction_to_light;
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
        let sun = &view_bindings::lights.directional_lights[SUN_LIGHT];
        tuned += (*sun).color.rgb * view_bindings::view.exposure / SKY_GAIN * TUNED_SUN_HEIGHT;
    }
    return (direct_sun(up, world_position) + direct_moon(up) + sky) / max(tuned, vec3<f32>(1e-4));
}
