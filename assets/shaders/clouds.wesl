// The clouds lit from what `cloud_march.wesl` gathers, drawn by `cloud_dome.wesl` and reflected in water.

import bevy_pbr::render::mesh_view_bindings as view_bindings;
import super::lighting::{moon_radiance, open_sky, sun_radiance};
import super::sky::{MOON_LIGHT, SUN_LIGHT};
import super::horizon::horizon_haze;
import super::cloud_march::{BASE, SUN_SCATTER, TOP};

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
// Light through a thick cloud has scattered too often to keep `phase`'s forward peak, so it evens out over this cover, or the sun would glow through as a bright spot.
const DIFFUSED: vec2<f32> = vec2<f32>(0.6, 1.0);

// `marched` lit and premultiplied, light in rgb and cover in a, dimmed by `shade` in storms.
fn lit_clouds(marched: vec4<f32>, origin: vec3<f32>, direction: vec3<f32>, shade: f32) -> vec4<f32> {
    let to_sun = view_bindings::lights.directional_lights[SUN_LIGHT].direction_to_light;
    let to_moon = view_bindings::lights.directional_lights[MOON_LIGHT].direction_to_light;
    let sunlight = sun_radiance(vec3<f32>(origin.x, 0.5 * (BASE + TOP), origin.z));
    let diffused = smoothstep(DIFFUSED.x, DIFFUSED.y, marched.w);
    let sun = mix(vec3<f32>(dot(sunlight, LUMINANCE)), sunlight, SUN_TINT) * mix(phase(dot(direction, to_sun)), 1.0, diffused) * SUN_SCATTER;
    let around = (moon_radiance() * mix(phase(dot(direction, to_moon)), 1.0, diffused) + open_sky(vec3<f32>(0.0, 1.0, 0.0))) * SHADE_TINT;
    let haze = horizon_haze(direction.xz + vec2<f32>(1e-5, 0.0));
    return vec4<f32>((sun * marched.x + around * marched.y) * shade + haze * marched.z, marched.w);
}
