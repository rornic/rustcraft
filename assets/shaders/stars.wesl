// The night sky's stars, Milky Way and moon, for `night_sky.wesl` and the water that reflects
// them. Stars are placed by hashing a grid of cells, and turn with the sun round the same axis
// it circles.

import bevy_pbr::render::mesh_view_bindings as view_bindings;
import super::sky::{MOON_LIGHT, SUN_LIGHT, night_factor, night_glow};
import super::cloud_cover::disc_showing;
import super::lighting::moon_phase;

// The sun's path is the circle through these, which are square to each other, and the pole it
// turns about. Matches `path_basis` in `sky.rs`.
const EAST: vec3<f32> = vec3<f32>(0.66436, 0.0, 0.74738);
const PEAK: vec3<f32> = vec3<f32>(-0.44944, 0.79900, 0.39950);
const POLE: vec3<f32> = vec3<f32>(-0.5972, -0.6013, 0.5308);

// The stars run ahead of the moon, which keeps to the sun, by twice this many radians over a night, and ease back by day
// while they're hidden, so they're the same each night, as the real moon falls behind them through its month.
const NIGHT_DRIFT: f32 = 0.25;
// The moon's size in radians, about four times the real moon's, and how bright its lit face is.
const MOON_RADIUS: f32 = 0.035;
const MOON_SCENE: f32 = 2.2;
const MOON_HALO: vec3<f32> = vec3<f32>(0.5, 0.65, 1.0);
// How blurred the stars are as reflected in water, in radians.
const REFLECTED_BLUR: f32 = 0.012;

// The star layers: cells round the sky, the share with a star, how far its glow reaches in
// cells, and its brightest, in scene colour.
const LAYERS: i32 = 3;
const LAYER_GRID: array<f32, 3> = array<f32, 3>(60.0, 140.0, 320.0);
const LAYER_DENSITY: array<f32, 3> = array<f32, 3>(0.22, 0.16, 0.1);
const LAYER_REACH: array<f32, 3> = array<f32, 3>(0.14, 0.16, 0.2);
const LAYER_GAIN: array<f32, 3> = array<f32, 3>(4.0, 1.6, 0.8);
// Fainter layers are thicker along the Milky Way, by up to this many times.
const BAND_CROWDING: f32 = 3.0;
// The Milky Way's pole, in the stars' frame, how narrow its band is, and how bright.
const GALAXY_POLE: vec3<f32> = vec3<f32>(0.30, 0.80, -0.52);
const BAND_WIDTH: f32 = 0.05;
const MILKY_WAY: f32 = 0.09;

fn pcg3d(p: vec3<u32>) -> vec3<u32> {
    var v = p * 1664525u + 1013904223u;
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    v ^= v >> vec3<u32>(16u);
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    return v;
}

// Three random numbers, 0..1, for a cell.
fn hash3(cell: vec3<i32>) -> vec3<f32> {
    return vec3<f32>(pcg3d(bitcast<vec3<u32>>(cell))) * (1.0 / 4294967295.0);
}

fn value_noise(p: vec3<f32>) -> f32 {
    let cell = vec3<i32>(floor(p));
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    var corners: array<f32, 8>;
    for (var i = 0; i < 8; i++) {
        corners[i] = hash3(cell + vec3<i32>(i & 1, (i >> 1) & 1, (i >> 2) & 1)).x;
    }
    let x0 = mix(mix(corners[0], corners[1], u.x), mix(corners[2], corners[3], u.x), u.y);
    let x1 = mix(mix(corners[4], corners[5], u.x), mix(corners[6], corners[7], u.x), u.y);
    return mix(x0, x1, u.z);
}

fn fbm(p: vec3<f32>) -> f32 {
    return 0.5 * value_noise(p) + 0.25 * value_noise(p * 2.03 + 7.1) + 0.125 * value_noise(p * 4.1 + 13.7);
}

fn rotate_about(axis: vec3<f32>, angle: f32, v: vec3<f32>) -> vec3<f32> {
    return v * cos(angle) + cross(axis, v) * sin(angle) + axis * dot(axis, v) * (1.0 - cos(angle));
}

// `direction` in the frame the stars are fixed in: the sky turned back by how far round its
// path the sun has come, and on by `NIGHT_DRIFT`.
fn star_frame(direction: vec3<f32>) -> vec3<f32> {
    let sun = view_bindings::lights.directional_lights[SUN_LIGHT].direction_to_light;
    let angle = atan2(dot(sun, PEAK), dot(sun, EAST));
    return rotate_about(POLE, -(angle + NIGHT_DRIFT * cos(angle)), direction);
}

// The light of one layer's star, if there's one in this direction's cell, for `footprint`
// radians a pixel: stars smaller than a pixel are drawn a pixel wide and dimmed to match.
fn star_layer(stars: vec3<f32>, layer: i32, crowding: f32, footprint: f32) -> vec3<f32> {
    let grid = LAYER_GRID[layer];
    let at = stars * grid;
    let cell = floor(at);
    let id = vec3<i32>(cell);
    let place = hash3(id);
    let kind = hash3(id + vec3<i32>(71, 13, 5));
    if kind.x > LAYER_DENSITY[layer] * crowding {
        return vec3<f32>(0.0);
    }
    let reach = LAYER_REACH[layer] * mix(0.4, 1.0, kind.y);
    let drawn = max(reach, footprint * grid * 0.8);
    let falloff = pow(clamp(1.0 - distance(at - cell, 0.2 + 0.6 * place) / drawn, 0.0, 1.0), 2.0);
    let brightness = LAYER_GAIN[layer] * (0.25 + 0.75 * pow(kind.y, 5.0));
    let colour = mix(vec3<f32>(1.0, 0.78, 0.6), vec3<f32>(0.72, 0.82, 1.0), kind.z);
    let twinkle = 1.0 + 0.2 * sin(view_bindings::globals.time * 3.0 + place.x * 50.0);
    return colour * brightness * falloff * twinkle * (reach * reach) / (drawn * drawn);
}

// The Milky Way's glow in the stars' frame, and in w how far into its band that is, 0..1.
fn milky_way(stars: vec3<f32>) -> vec4<f32> {
    let latitude = dot(stars, GALAXY_POLE);
    let band = exp(-latitude * latitude / BAND_WIDTH);
    if band < 0.01 {
        return vec4<f32>(0.0);
    }
    // a gentle unevenness along the band, no darker than it is brighter
    let uneven = 0.75 + 0.5 * fbm(stars * 5.0 + 11.0);
    let warmth = smoothstep(0.4, 0.8, fbm(stars * 2.0 + 5.0));
    let colour = mix(vec3<f32>(0.45, 0.5, 0.8), vec3<f32>(0.9, 0.75, 0.55), warmth);
    return vec4<f32>(colour * MILKY_WAY * band * uneven, band);
}

// The moon over `direction`, premultiplied: its face, lit by the sun, with dark seas.
fn moon_disc(direction: vec3<f32>, footprint: f32) -> vec4<f32> {
    if view_bindings::lights.n_directional_lights < 2u {
        return vec4<f32>(0.0);
    }
    let to_moon = view_bindings::lights.directional_lights[MOON_LIGHT].direction_to_light;
    let to_sun = view_bindings::lights.directional_lights[SUN_LIGHT].direction_to_light;
    let angle = acos(clamp(dot(direction, to_moon), -1.0, 1.0));
    if angle > MOON_RADIUS + footprint {
        return vec4<f32>(0.0);
    }
    let right = normalize(cross(vec3<f32>(0.0, 1.0, 0.0), to_moon));
    let up = cross(to_moon, right);
    let off = direction - to_moon * dot(direction, to_moon);
    let x = dot(off, right) / MOON_RADIUS;
    let y = dot(off, up) / MOON_RADIUS;
    let z = sqrt(max(1.0 - x * x - y * y, 0.0));
    let normal = x * right + y * up - z * to_moon;
    // the face the moon turns to us
    let face = vec3<f32>(x, y, z);
    let seas = smoothstep(0.52, 0.6, fbm(face * 1.8 + 4.0));
    let albedo = mix(0.95, 0.5, seas * 0.85) * (0.85 + 0.3 * fbm(face * 9.0));
    let lit = mix(0.04, 1.0, smoothstep(-0.03, 0.06, dot(normal, to_sun)));
    let cover = 1.0 - smoothstep(MOON_RADIUS - footprint, MOON_RADIUS + footprint, angle);
    // behind cloud its face goes, but it still covers the stars behind it
    return vec4<f32>(vec3<f32>(1.0, 0.97, 0.9) * albedo * lit * MOON_SCENE * cover * disc_showing(true), cover);
}

// The glow of moonlight in the air round the moon.
fn moon_halo(direction: vec3<f32>) -> vec3<f32> {
    if view_bindings::lights.n_directional_lights < 2u {
        return vec3<f32>(0.0);
    }
    let angle = acos(clamp(dot(direction, view_bindings::lights.directional_lights[MOON_LIGHT].direction_to_light), -1.0, 1.0));
    return MOON_HALO * moon_phase() * (0.03 * exp(-angle / 0.12) + 0.008 * exp(-angle / 0.6)) * disc_showing(true);
}

// Everything the night adds to the sky towards `direction`, premultiplied, for `footprint`
// radians a pixel: its glow, stars, Milky Way and the moon. Only the brightest stars for `rich` false.
fn celestial(direction: vec3<f32>, footprint: f32, rich: bool) -> vec4<f32> {
    let night = night_factor();
    var light = night_glow(direction);
    if night > 0.0 {
        let stars = star_frame(direction);
        var crowding = 1.0;
        var milky = vec3<f32>(0.0);
        if rich {
            let way = milky_way(stars);
            milky = way.rgb;
            crowding = 1.0 + BAND_CROWDING * way.a;
        }
        var field = vec3<f32>(0.0);
        for (var i = 0; i < LAYERS; i++) {
            if rich || i == 0 {
                field += star_layer(stars, i, select(1.0, crowding, i > 0), footprint);
            }
        }
        // stars dim and vanish into the haze near the horizon
        let rise = smoothstep(0.0, 0.12, direction.y);
        light += night * (milky + field * rise + moon_halo(direction));
    }
    let moon = moon_disc(direction, footprint);
    return vec4<f32>(light * (1.0 - moon.a) + moon.rgb, moon.a);
}

// The night's stars and moon as reflected in water towards `direction`, premultiplied, blurred
// for the waves that stir them and without the sky's glow, which the reflected sky has.
fn reflected_celestial(direction: vec3<f32>) -> vec4<f32> {
    let night = night_factor();
    var light = vec3<f32>(0.0);
    if night > 0.0 {
        let rise = smoothstep(0.0, 0.12, direction.y);
        light = night * (star_layer(star_frame(direction), 0, 1.0, REFLECTED_BLUR) * rise + moon_halo(direction));
    }
    let moon = moon_disc(direction, REFLECTED_BLUR);
    return vec4<f32>(light * (1.0 - moon.a) + moon.rgb, moon.a);
}
