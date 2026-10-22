// The unlit clouds marched at a fraction of screen resolution with no view, told the camera by `clouds::CloudView`.

import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;
import super::cloud_march::{FAR, cloud_entry, march_clouds};
import super::curvature::curvature;

struct CloudView {
    right: vec4<f32>,
    up: vec4<f32>,
    forward: vec4<f32>,
    eye: vec4<f32>,
    sun: vec4<f32>,
    last_right: vec4<f32>,
    last_up: vec4<f32>,
    last_forward: vec4<f32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(7) var<uniform> cloud_view: CloudView;
// last frame's march, see `clouds::CloudMarch::history`
@group(constants::MATERIAL_BIND_GROUP) @binding(9) var cloud_history: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(10) var cloud_history_sampler: sampler;

// Moving only shifts clouds nearer than this against the sky, so the share of each frame kept for moving is for them: farther off the history stays good, and its averaging is what hides the march's noise.
const PARALLAX_REACH: f32 = 2500.0;
// The share of each frame kept when still, matching `HISTORY_BLEND` in `clouds.rs`.
const STILL_BLEND: f32 = 0.12;

// Few samples a pixel, offset per pixel and frame and blended into the history so they add up to many.
const STEPS: i32 = 12;

// Where last frame's view saw `direction`, in its uv, and in z whether it did at all.
fn last_seen(direction: vec3<f32>) -> vec3<f32> {
    let ahead = dot(direction, cloud_view.last_forward.xyz);
    let right = cloud_view.last_right.xyz;
    let up = cloud_view.last_up.xyz;
    let clip = vec2<f32>(dot(direction, right) / dot(right, right), dot(direction, up) / dot(up, up)) / max(ahead, 1e-4);
    let seen = ahead > 0.0 && all(abs(clip) < vec2<f32>(1.0));
    return vec3<f32>(clip.x * 0.5 + 0.5, 0.5 - clip.y * 0.5, select(0.0, 1.0, seen));
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let clip = vec2<f32>(in.uv.x * 2.0 - 1.0, 1.0 - in.uv.y * 2.0);
    let direction = normalize(cloud_view.forward.xyz + clip.x * cloud_view.right.xyz + clip.y * cloud_view.up.xyz);
    // interleaved gradient noise stepped by the golden ratio each frame, so starts spread evenly over the history
    let spatial = fract(52.9829189 * fract(dot(in.position.xy, vec2<f32>(0.06711056, 0.00583715))));
    let jitter = fract(spatial + 0.618034 * (cloud_view.sun.w % 4096.0));
    let marched = march_clouds(cloud_view.eye.xyz, direction, curvature.now.per_square_block, FAR, STEPS, 3, jitter, cloud_view.sun.xyz, cloud_view.eye.w);
    let last = last_seen(direction);
    if last.z == 0.0 {
        return marched;
    }
    let entry = cloud_entry(cloud_view.eye.xyz, direction, curvature.now.per_square_block);
    let blend = mix(STILL_BLEND, cloud_view.last_forward.w, saturate(PARALLAX_REACH / max(entry, 1.0)));
    return mix(textureSampleLevel(cloud_history, cloud_history_sampler, last.xy, 0.0), marched, blend);
}
