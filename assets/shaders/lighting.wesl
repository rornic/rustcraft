// The sun and sky's light, shared by everything lit outdoors.

import bevy_pbr::render::mesh_view_bindings as view_bindings;
import super::sky::sky_irradiance;

// Matches `SKY_GAIN` in `sky.rs`.
const SKY_GAIN: f32 = 4.0;

// Light falling on a face from the open sky, cool against the warm sun: faces up get
// the sky's blue, faces down only what the ground bounces back.
// How much of the sky's irradiance lights a face, for the look rather than physics.
const SKY_AMBIENT_GAIN: f32 = 1.0;
const GROUND_AMBIENT: vec3<f32> = vec3<f32>(0.2, 0.18, 0.15);

// Light from the whole sky on a face turned to `normal`, out in the open.
fn open_sky(normal: vec3<f32>) -> vec3<f32> {
    return mix(GROUND_AMBIENT, sky_irradiance(normal) * SKY_AMBIENT_GAIN, normal.y * 0.5 + 0.5);
}

// Unshadowed sunlight on a face turned to `normal`: warm, and dimmer the more it glances
// off. Nothing when it faces away, or there's no sun.
fn direct_sun(normal: vec3<f32>) -> vec3<f32> {
    if view_bindings::lights.n_directional_lights == 0u {
        return vec3<f32>(0.0);
    }
    let sun = &view_bindings::lights.directional_lights[0];
    let radiance = (*sun).color.rgb * view_bindings::view.exposure / SKY_GAIN;
    return radiance * max(dot(normal, (*sun).direction_to_light), 0.0);
}
