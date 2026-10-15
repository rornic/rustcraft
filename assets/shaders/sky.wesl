// The atmosphere's sky as the camera sees it, read from the environment map Bevy fills
// from it each frame, so everything that has to match the sky follows the sun.

import bevy_pbr::render::mesh_view_bindings as view_bindings;
import bevy_render::maths::quat_rotate;

// Mip of the specular cubemap read for the sky: its blur hides the sun disc and smooths the colour.
const SKY_MIP: f32 = 3.0;
// Shown where there's no atmosphere to read.
const NO_SKY: vec3<f32> = vec3<f32>(0.0);

fn read_cube(direction: vec3<f32>, mip: f32, diffuse: bool) -> vec3<f32> {
@if(ENVIRONMENT_MAP) {
    let probes = view_bindings::light_probes;
    if probes.view_cubemap_index < 0 {
        return NO_SKY;
    }
    var dir = quat_rotate(probes.view_rotation, direction);
    dir.z = -dir.z;
    let scale = probes.intensity_for_view * view_bindings::view.exposure;
@if(MULTIPLE_LIGHT_PROBES_IN_ARRAY) {
    if diffuse {
        return textureSampleLevel(view_bindings::diffuse_environment_maps[probes.view_cubemap_index], view_bindings::environment_map_sampler, dir, mip).rgb * scale;
    }
    return textureSampleLevel(view_bindings::specular_environment_maps[probes.view_cubemap_index], view_bindings::environment_map_sampler, dir, mip).rgb * scale;
}
@if(!MULTIPLE_LIGHT_PROBES_IN_ARRAY) {
    if diffuse {
        return textureSampleLevel(view_bindings::diffuse_environment_map, view_bindings::environment_map_sampler, dir, mip).rgb * scale;
    }
    return textureSampleLevel(view_bindings::specular_environment_map, view_bindings::environment_map_sampler, dir, mip).rgb * scale;
}
}
@if(!ENVIRONMENT_MAP) {
    return NO_SKY;
}
}

// Where the sun and moon are among the view's directional lights: Bevy puts lights without shadows,
// the moon's, before those with them. See `moon` in `sky.rs`.
const SUN_LIGHT: u32 = 1u;
const MOON_LIGHT: u32 = 0u;

// How dark the sky is for the sun's height: 0 by day, 1 once the sun is far enough below
// the horizon for the stars to be out.
fn night_factor() -> f32 {
    if view_bindings::lights.n_directional_lights == 0u {
        return 0.0;
    }
    let sun_height = view_bindings::lights.directional_lights[SUN_LIGHT].direction_to_light.y;
    return 1.0 - smoothstep(-0.2, 0.02, sun_height);
}

// The faint glow of the night sky itself, brightest along the horizon, which the atmosphere
// leaves black once the sun is far down.
const GLOW_ZENITH: vec3<f32> = vec3<f32>(0.0008, 0.0016, 0.005);
const GLOW_HORIZON: vec3<f32> = vec3<f32>(0.004, 0.007, 0.016);

fn night_glow(direction: vec3<f32>) -> vec3<f32> {
    let low = pow(1.0 - clamp(direction.y, 0.0, 1.0), 4.0);
    return night_factor() * mix(GLOW_ZENITH, GLOW_HORIZON, low);
}

// Scene colour of the sky towards `direction`.
fn sky_colour(direction: vec3<f32>) -> vec3<f32> {
    return read_cube(direction, SKY_MIP, false) + night_glow(direction);
}

// Light the whole sky casts on a face turned to `normal`.
fn sky_irradiance(normal: vec3<f32>) -> vec3<f32> {
    return read_cube(normal, 0.0, true);
}
