// The unlit clouds marched at a fraction of screen resolution with no view, told the camera by `clouds::CloudView`.

import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;
import super::cloud_march::{FADE, FAR, HAZE, LAND_TOP, OVERHEAD_REACH, cloud_entry, disc_sky, march_clouds, march_ray};
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
    last_eye: vec4<f32>,
    // toward the moon
    moon: vec4<f32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(7) var<uniform> cloud_view: CloudView;
// last frame's march, see `clouds::CloudMarch::history`
@group(constants::MATERIAL_BIND_GROUP) @binding(9) var cloud_history: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(10) var cloud_history_sampler: sampler;

// The share of each frame kept over the history, and more for each target pixel moving shifted the clouds by, as their depth that reprojects the history is only an average.
const BLEND: f32 = 0.12;
const BLEND_PER_PIXEL: f32 = 0.03;
// A ray that met no cloud at all is sure of it, so takes far more, or moving trails the history's clouds over what they uncover.
const CLEAR_PER_PIXEL: f32 = 0.5;
// A clear ray reprojects the history from no nearer than this. Where the camera is in the slab the slab is entered at the
// camera itself, which reprojects nowhere, so every clear ray lost its history and the clouds' edges flickered.
const CLEAR_DEPTH: f32 = 2000.0;

// Few samples a pixel, offset per pixel and frame and blended into the history so they add up to many. Fewer leave
// the clouds' edges noisy even with the history, and inside the slab, where it's mostly discarded, every frame shows them.
const STEPS: i32 = 24;
// Steps and octaves for the reflections' sky map: few, as waves blur them.
const REFLECTED_STEPS: i32 = 4;
const REFLECTED_OCTAVES: i32 = 2;
// Steps for the marches toward the sun and moon, which hide them: only two rays, so many.
const DISC_STEPS: i32 = 32;

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
    let eye = cloud_view.eye.xyz;
    let bend = curvature.now.per_square_block;
    let ray = march_ray(eye, direction, bend, FAR, STEPS, 3, jitter, cloud_view.sun.xyz, cloud_view.eye.w);
    let cover = 1.0 - ray.through;
    let marched = vec4<f32>(ray.gathered, cover);
    // last frame saw what was gathered where it lies, or the slab where the ray met nothing; moving shifts it against the sky
    let depth = select(max(cloud_entry(eye, direction, bend), CLEAR_DEPTH), ray.depth / cover, cover > 0.01);
    let sky = last_seen(direction);
    let last = select(sky, last_seen(direction * depth + eye - cloud_view.last_eye.xyz), depth < FAR);
    if last.z == 0.0 {
        return marched;
    }
    let parallax = length((last.xy - sky.xy) * vec2<f32>(textureDimensions(cloud_history)));
    let blend = saturate(BLEND + select(BLEND_PER_PIXEL, CLEAR_PER_PIXEL, cover < 0.01) * parallax);
    return mix(textureSampleLevel(cloud_history, cloud_history_sampler, last.xy, 0.0), marched, blend);
}

// The clouds water reflects, laid out as `sky_disc` and `overhead` read them, seen from no higher than water lies so a camera above them still sees their undersides.
@fragment
fn reflection(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let eye = cloud_view.eye.xyz;
    // the first two texels lie outside the disc, so hold the clouds from the eye itself toward the sun and moon, see `disc_showing`
    let texel = vec2<i32>(in.position.xy);
    if texel.y == 0 && texel.x < 2 {
        let toward = select(cloud_view.sun.xyz, cloud_view.moon.xyz, texel.x == 1);
        return march_clouds(eye, toward, curvature.now.per_square_block, FAR, DISC_STEPS, 3, 0.5, cloud_view.sun.xyz, cloud_view.eye.w);
    }
    let uv = vec2<f32>(fract(in.uv.x * 2.0), in.uv.y);
    if in.uv.x < 0.5 {
        let direction = disc_sky(uv);
        if direction.y < 0.0 {
            return vec4<f32>(0.0);
        }
        return march_clouds(vec3<f32>(eye.x, min(eye.y, LAND_TOP), eye.z), direction, 0.0, FAR, REFLECTED_STEPS, REFLECTED_OCTAVES, 0.5, cloud_view.sun.xyz, cloud_view.eye.w);
    }
    // straight up there, then hazed and faded by how far off that is, which a march straight up never counts
    let xz = eye.xz + (uv * 2.0 - 1.0) * OVERHEAD_REACH;
    let ray = march_ray(vec3<f32>(xz.x, LAND_TOP, xz.y), vec3<f32>(0.0, 1.0, 0.0), 0.0, FAR, REFLECTED_STEPS, REFLECTED_OCTAVES, 0.5, cloud_view.sun.xyz, cloud_view.eye.w);
    let away = distance(xz, eye.xz);
    let hazed = 1.0 - exp2(-away / HAZE);
    let cover = (1.0 - ray.through) * (1.0 - smoothstep(FADE.x, FADE.y, away));
    let lit = ray.gathered.xy / max(1.0 - ray.through, 1e-4);
    return vec4<f32>(vec3<f32>(lit * (1.0 - hazed), hazed) * cover, cover);
}
