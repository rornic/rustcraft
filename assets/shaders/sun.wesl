// The sun's disc and glow, drawn over the sky towards the first directional light, so it
// sits wherever the light comes from. See `sky.rs`.

import bevy_pbr::render::mesh_view_bindings as view_bindings;

// Angular radii as tangents: the disc, and where the glow fades out.
const DISC_RADIUS: f32 = 0.035;
const GLOW_RADIUS: f32 = 0.45;
// HDR multiples of the sunlight's own colour, bright enough to bloom and still be white tonemapped.
const DISC_GAIN: f32 = 24.0;
const GLOW_GAIN: f32 = 1.2;
const GLOW_FALLOFF: f32 = 11.0;

struct SunVertex {
    @builtin(position) position: vec4<f32>,
}

// The lights aren't visible to the vertex stage, so this covers the screen at the far plane
// (anything in front wins the depth test) and the fragment works out where the sun is.
@vertex
fn vertex(@location(0) corner: vec3<f32>) -> SunVertex {
    var out: SunVertex;
    out.position = vec4<f32>(corner.xy, 1.0e-7, 1.0);
    return out;
}

@fragment
fn fragment(in: SunVertex) -> @location(0) vec4<f32> {
    if view_bindings::lights.n_directional_lights == 0u {
        discard;
    }
    let light = view_bindings::lights.directional_lights[0];
    let view = view_bindings::view;

    let uv = (in.position.xy - view.viewport.xy) / view.viewport.zw;
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let on_near = view.view_from_clip * vec4<f32>(ndc, 1.0, 1.0);
    let ray = normalize((view.world_from_view * vec4<f32>(on_near.xyz / on_near.w, 0.0)).xyz);

    let facing = dot(ray, light.direction_to_light);
    if facing <= 0.0 {
        discard;
    }
    // offset from the sun in the plane facing it, its length the tangent of the angle between
    let radius = length(ray / facing - light.direction_to_light) / GLOW_RADIUS;
    if radius >= 1.0 {
        discard;
    }

    // exposure is cancelled here, as in the lit shaders
    let radiance = light.color.rgb * view.exposure;
    let disc = 1.0 - smoothstep(0.85, 1.0, radius * GLOW_RADIUS / DISC_RADIUS);
    let glow = exp(-radius * GLOW_FALLOFF) * (1.0 - radius);
    // alpha 0 under premultiplied blending adds to the sky instead of replacing it
    return vec4<f32>(radiance * (disc * DISC_GAIN + glow * GLOW_GAIN), 0.0);
}
