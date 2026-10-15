// The sun's glare, drawn over the sky round it, see `sun_glare.rs`: a halo and a burst of
// rays, as bright light scatters in an eye or a lens. Bevy's bloom leaves out a spot as small
// as the sun's disc, to keep single pixels from flickering, so it's drawn here instead.

import bevy_pbr::render::{
    forward_io::{Vertex, VertexOutput},
    mesh_functions,
    mesh_view_bindings as view_bindings,
    view_transformations::{position_world_to_clip, ndc_to_uv},
};
import super::lighting::sun_radiance;

// The halo, bright close in and fainter far out: how bright each part is, as a share of
// sunlight on a face turned to it, and how far it reaches, in radians.
const CORE: f32 = 6.0;
const CORE_REACH: f32 = 0.012;
const HALO: f32 = 0.6;
const HALO_REACH: f32 = 0.12;
// The rays: how many, how thin, how bright and how far they reach, in radians.
const RAYS: f32 = 6.0;
const RAY_SHARPNESS: f32 = 60.0;
const RAY_GAIN: f32 = 0.7;
const RAY_REACH: f32 = 0.05;
// Past this angle off the sun, in radians, there's no glare to draw.
const REACH: f32 = 0.6;
// How far round the sun's disc its cover is looked at, in radians, see `sun_showing`.
const DISC: f32 = 0.01;

// Pinned to the far plane, so it only draws where there's nothing but sky.
@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    out.world_position = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(vertex.position, 1.0));
    out.position = position_world_to_clip(out.world_position.xyz);
    // reversed depth: 0 is infinitely far
    out.position.z = 0.0;
    return out;
}

// How much of the sun's disc the land leaves showing, 0..1, from what the prepass drew where
// it is on screen: the glare goes when it sets behind a mountain. All of it without a prepass.
fn sun_showing(to_sun: vec3<f32>) -> f32 {
@if(DEPTH_PREPASS && !MULTISAMPLED) {
    let eye = view_bindings::view.world_position.xyz;
    let size = vec2<f32>(textureDimensions(view_bindings::depth_prepass_texture));
    // two directions square to the sun, to look round its disc
    let side = normalize(cross(to_sun, select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(1.0, 0.0, 0.0), abs(to_sun.y) > 0.99)));
    let other = cross(to_sun, side);
    let offsets = array<vec2<f32>, 5>(vec2(0.0), vec2(1.0, 0.0), vec2(-1.0, 0.0), vec2(0.0, 1.0), vec2(0.0, -1.0));
    var showing = 0.0;
    for (var i = 0; i < 5; i++) {
        let toward = normalize(to_sun + (side * offsets[i].x + other * offsets[i].y) * DISC);
        let clip = position_world_to_clip(eye + toward * 1.0e4);
        let uv = ndc_to_uv(clip.xy / clip.w);
        if clip.w > 0.0 && all(uv >= vec2(0.0)) && all(uv < vec2(1.0)) {
            let depth = textureLoad(view_bindings::depth_prepass_texture, vec2<i32>(uv * size), 0);
            showing += select(0.0, 1.0, depth == 0.0);
        } else {
            // off screen, so nothing drawn over it to see
            showing += 1.0;
        }
    }
    return showing / 5.0;
}
@if(!(DEPTH_PREPASS && !MULTISAMPLED)) {
    return 1.0;
}
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    if view_bindings::lights.n_directional_lights == 0u {
        discard;
    }
    let eye = view_bindings::view.world_position.xyz;
    let toward = normalize(in.world_position.xyz - eye);
    let to_sun = view_bindings::lights.directional_lights[0].direction_to_light;
    let off = acos(clamp(dot(toward, to_sun), -1.0, 1.0));
    if off > REACH {
        discard;
    }
    // round the sun as the screen sees it, so the rays stay put as a lens's would
    let right = normalize(view_bindings::view.world_from_view[0].xyz - to_sun * dot(view_bindings::view.world_from_view[0].xyz, to_sun));
    let up = cross(to_sun, right);
    let around = atan2(dot(toward, up), dot(toward, right));
    let rays = pow(abs(cos(around * RAYS * 0.5)), RAY_SHARPNESS) * RAY_GAIN * exp(-off / RAY_REACH);
    let halo = CORE * exp(-off / CORE_REACH) + HALO * exp(-off / HALO_REACH);
    let glare = sun_radiance(eye) * (halo + rays) * sun_showing(to_sun);
    // premultiplied, with nothing covered: it only adds light
    return vec4<f32>(glare, 0.0);
}
