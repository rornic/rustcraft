// The clouds lit from what `cloud_march.wesl` gathers, drawn by `cloud_dome.wesl` and reflected in water.

import bevy_pbr::render::mesh_view_bindings as view_bindings;
import super::lighting::{moon_radiance, open_sky, sun_radiance};
import super::sky::{MOON_LIGHT, SUN_LIGHT};
import super::horizon::horizon_haze;
import super::cloud_march::{BASE, FAR, SUN_SCATTER, TOP, march_clouds};

// Scattering phase toward `cos_theta`: mostly forward, for the bright rim before the sun.
fn phase(cos_theta: f32) -> f32 {
    return mix(henyey_greenstein(cos_theta, -0.2), henyey_greenstein(cos_theta, 0.8), 0.4);
}

fn henyey_greenstein(cos_theta: f32, g: f32) -> f32 {
    return (1.0 - g * g) / pow(1.0 + g * g - 2.0 * g * cos_theta, 1.5);
}

// The sun's warmth clouds keep, less than the land's so they're white by day, and their bases' cool cast.
const SUN_TINT: f32 = 0.5;
const SHADE_TINT: vec3<f32> = vec3<f32>(0.85, 0.92, 1.05);
const LUMINANCE: vec3<f32> = vec3<f32>(0.2126, 0.7152, 0.0722);

// `marched` lit and premultiplied, light in rgb and cover in a, dimmed by `shade` in storms.
fn lit_clouds(marched: vec4<f32>, origin: vec3<f32>, direction: vec3<f32>, shade: f32) -> vec4<f32> {
    let to_sun = view_bindings::lights.directional_lights[SUN_LIGHT].direction_to_light;
    let to_moon = view_bindings::lights.directional_lights[MOON_LIGHT].direction_to_light;
    let sunlight = sun_radiance(vec3<f32>(origin.x, 0.5 * (BASE + TOP), origin.z));
    let sun = mix(vec3<f32>(dot(sunlight, LUMINANCE)), sunlight, SUN_TINT) * phase(dot(direction, to_sun)) * SUN_SCATTER;
    let around = (moon_radiance() * phase(dot(direction, to_moon)) + open_sky(vec3<f32>(0.0, 1.0, 0.0))) * SHADE_TINT;
    let haze = horizon_haze(direction.xz + vec2<f32>(1e-5, 0.0));
    return vec4<f32>((sun * marched.x + around * marched.y) * shade + haze * marched.z, marched.w);
}

// A cheap flat march of the clouds for reflections, which the waves blur.
fn clouds(origin: vec3<f32>, direction: vec3<f32>, steps: i32, octaves: i32) -> vec4<f32> {
    let to_sun = view_bindings::lights.directional_lights[SUN_LIGHT].direction_to_light;
    let marched = march_clouds(origin, direction, 0.0, FAR, steps, octaves, 0.5, to_sun, view_bindings::globals.time);
    return lit_clouds(marched, origin, direction, 1.0);
}
