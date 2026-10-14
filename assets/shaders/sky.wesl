// The atmosphere's sky as the camera sees it, read from the environment map Bevy fills
// from it each frame, so everything that has to match the sky follows the sun.

import bevy_pbr::render::mesh_view_bindings as view_bindings;
import bevy_render::maths::quat_rotate;

// Mip of the specular cubemap read for the sky: its blur hides the sun disc and smooths the colour.
const SKY_MIP: f32 = 3.0;
// A sharper mip, for drawing the sky itself, where `SKY_MIP` would blur the dark ground
// below the horizon up into it.
const CLEAR_SKY_MIP: f32 = 1.0;
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

// Scene colour of the sky towards `direction`.
fn sky_colour(direction: vec3<f32>) -> vec3<f32> {
    return read_cube(direction, SKY_MIP, false);
}

// Scene colour of the sky towards `direction` as it's drawn, see `CLEAR_SKY_MIP`.
fn clear_sky_colour(direction: vec3<f32>) -> vec3<f32> {
    return read_cube(direction, CLEAR_SKY_MIP, false);
}

// Light the whole sky casts on a face turned to `normal`.
fn sky_irradiance(normal: vec3<f32>) -> vec3<f32> {
    return read_cube(normal, 0.0, true);
}
