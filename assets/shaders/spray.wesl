import bevy_pbr::render::{
    forward_io::{Vertex, VertexOutput, decompress_vertex},
    mesh_view_bindings as view_bindings,
    mesh_functions,
    view_transformations::position_world_to_clip,
};
import super::lighting::daylight;
import super::curvature::{curved, uncurved};
import super::hash::hash2;

// Seconds a plume and a droplet live before they're thrown up again, how fast they fall back in blocks a second
// squared, and how far off its flow a droplet scatters, in blocks a second.
const PLUME_LIFE: f32 = 1.4;
const DROPLET_LIFE: f32 = 0.8;
const PLUME_GRAVITY: f32 = 0.6;
const DROPLET_GRAVITY: f32 = 9.0;
const DROPLET_SCATTER: f32 = 1.6;
// Seconds between an emitter's bursts, varied this much either way from one emitter to the next, and over how many
// seconds of each its particles are thrown.
const BURST_EVERY: f32 = 1.8;
const BURST_VARY: f32 = 0.35;
const BURST_SPAN: f32 = 0.25;
// Spray thins out to nothing between these distances, where it'd only be a few pixels.
const FADE: vec2<f32> = vec2<f32>(40.0, 72.0);
const SPRAY_COLOR: vec3<f32> = vec3<f32>(0.85, 0.9, 0.92);

@vertex
fn vertex(vertex_in: Vertex) -> VertexOutput {
    let vertex = decompress_vertex(vertex_in, vertex_in.instance_index);
    var out: VertexOutput;
    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    let base = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(vertex.position, 1.0)).xyz;
    // the flow is in the normal, unnormalised, and the seed and size in the colour
    let flow = vertex.normal;
    let seed = vertex.color.r;
    let size = vertex.color.g;
    let droplet = vertex.color.b;
    let lifetime = mix(PLUME_LIFE, DROPLET_LIFE, droplet);
    let burst = vertex.color.a;
    let time = view_bindings::globals.time;
    let every = BURST_EVERY * (1.0 + BURST_VARY * (2.0 * fract(burst * 13.73) - 1.0));
    let cycle = time / every + burst;
    let age = fract(cycle) * every - seed * BURST_SPAN;
    // out of its moment in the burst, it's drawn as nothing
    let alive = f32(age >= 0.0 && age < lifetime);
    let t = clamp(age / lifetime, 0.0, 1.0);
    // each burst throws from somewhere else round the emitter and a droplet another way, so the spray doesn't repeat
    let life = floor(cycle);
    let cell = vec2<f32>(life, floor(seed * 65535.0));
    let jitter = vec3<f32>(hash2(cell), hash2(cell + vec2<f32>(0.0, 70001.0)), hash2(cell + vec2<f32>(0.0, 140002.0))) - 0.5;
    let velocity = flow + jitter * 2.0 * DROPLET_SCATTER * droplet;
    var at = base + vec3<f32>(jitter.x, 0.0, jitter.z) * 0.3 + velocity * age;
    at.y -= 0.5 * mix(PLUME_GRAVITY, DROPLET_GRAVITY, droplet) * age * age;
    let right = view_bindings::view.world_from_view[0].xyz;
    let up = view_bindings::view.world_from_view[1].xyz;
    // a solid square like a voxel's, shrinking away rather than fading
    let grown = size * (1.0 - 0.7 * t * t) * alive;
    at += (right * vertex.uv.x + up * vertex.uv.y) * grown * 0.5;
    out.world_position = vec4<f32>(curved(at), 1.0);
    out.position = position_world_to_clip(out.world_position.xyz);
    out.uv = vertex.uv;
    out.color = vec4<f32>(t, 0.0, 0.0, 1.0);
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let world = uncurved(in.world_position.xyz);
    let dist = distance(world, view_bindings::view.world_position.xyz);
    let alpha = 0.9 * (1.0 - smoothstep(FADE.x, FADE.y, dist));
    return vec4<f32>(SPRAY_COLOR * daylight(world), alpha);
}
