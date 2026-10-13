// The sun's disc and glow, a quad that always faces the camera towards the first
// directional light, so it sits wherever the light comes from. See `sky.rs`.

import bevy_pbr::render::mesh_view_bindings as view_bindings;

// Angular radii as tangents: the disc, and the quad the glow fades out across.
const DISC_RADIUS: f32 = 0.035;
const GLOW_RADIUS: f32 = 0.45;
// HDR multiples of the sunlight's own colour, bright enough to bloom and still be white tonemapped.
const DISC_GAIN: f32 = 24.0;
const GLOW_GAIN: f32 = 1.2;
const GLOW_FALLOFF: f32 = 11.0;

struct SunVertex {
    @builtin(position) position: vec4<f32>,
    @location(0) offset: vec2<f32>,
}

@vertex
fn vertex(@location(0) corner: vec3<f32>) -> SunVertex {
    var out: SunVertex;
    out.offset = corner.xy;
    if view_bindings::lights.n_directional_lights == 0u {
        out.position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
        return out;
    }
    let to_sun = view_bindings::lights.directional_lights[0].direction_to_light;
    var clip = view_bindings::view.clip_from_world * vec4<f32>(to_sun, 0.0);
    if clip.w <= 0.0 {
        out.position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
        return out;
    }
    let projection = view_bindings::view.clip_from_view;
    clip.x += corner.x * GLOW_RADIUS * projection[0][0] * clip.w;
    clip.y += corner.y * GLOW_RADIUS * projection[1][1] * clip.w;
    // the far plane, so any terrain or cloud in front wins the depth test
    clip.z = clip.w * 1.0e-7;
    out.position = clip;
    return out;
}

@fragment
fn fragment(in: SunVertex) -> @location(0) vec4<f32> {
    let radius = length(in.offset);
    let light = view_bindings::lights.directional_lights[0];
    // exposure is cancelled here, as in the lit shaders
    let radiance = light.color.rgb * view_bindings::view.exposure;
    let disc = 1.0 - smoothstep(0.85, 1.0, radius * GLOW_RADIUS / DISC_RADIUS);
    let glow = exp(-radius * GLOW_FALLOFF) * (1.0 - radius);
    let colour = radiance * (disc * DISC_GAIN + glow * GLOW_GAIN);
    return vec4<f32>(colour, 1.0);
}
