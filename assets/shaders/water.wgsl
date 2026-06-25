#import bevy_pbr::{
    forward_io::{Vertex, VertexOutput},
    mesh_view_bindings as view_bindings,
    mesh_functions,
    view_transformations::position_world_to_clip,
    pbr_functions::apply_fog,
}

@group(2) @binding(0) var<uniform> material_color: vec4<f32>;
@group(2) @binding(1) var material_color_texture: texture_2d<f32>;
@group(2) @binding(2) var material_color_sampler: sampler;

const TROPICAL_TINT: vec3<f32> = vec3<f32>(0.0, 0.85, 0.9);
const TROPICAL_PUSH: f32 = 0.4;
const DEEP_TINT: vec3<f32> = vec3<f32>(0.03, 0.18, 0.3);
// Beer-Lambert-style extinction: how many blocks of water it takes for the seafloor
// to fade to invisible, rather than a flat translucency that stays see-through no
// matter how deep the water gets.
const EXTINCTION_DISTANCE: f32 = 3.0;
const CHUNK_SIZE: f32 = 16.0;

const FOAM_COLOR: vec3<f32> = vec3<f32>(0.95, 0.98, 0.97);
const FOAM_SPEED: f32 = 0.45;
const FOAM_BAND: f32 = 0.35;
// How many radians the shore-distance band (0..1, see vertex_shore_distance) spans -
// roughly how many foam bands are visible across the ~3-block search radius.
const FOAM_PHASE_SCALE: f32 = 5.0;
// Scales how much of the lapping envelope gets revealed as foam cells - pushes
// coverage up for a denser look without changing where/how fast the envelope moves.
const FOAM_DENSITY: f32 = 1.2;
// Foam is a blocky, flickering cutout (Minecraft's animated-texture-frame look)
// rather than smooth organic noise - a coarse grid hashed per discrete time step.
const FOAM_CELL_SIZE: f32 = 0.125;
const FOAM_FRAME_RATE: f32 = 1.2;

const WAVE_AMPLITUDE: f32 = 0.15;
const WAVE_SPEED: f32 = 1.0;
const WAVE_FREQ: f32 = 0.35;
// Waves fade out (mostly to avoid distant shimmer/aliasing) further out than foam
// does - foam's noise sampling is the expensive part per-pixel, so it gets cut
// earlier purely for performance, well before its fine detail would matter anyway.
const WAVE_FADE_END: f32 = 256.0;
const WAVE_FADE_START: f32 = 192.0;
const FOAM_FADE_END: f32 = 192.0;
const FOAM_FADE_START: f32 = 144.0;

struct FragmentOutput {
  @location(0) color: vec4<f32>
}

// Two travelling sine waves, summed - a cheap stand-in for a proper Gerstner wave
// that's enough to read as gentle open-water ripples at this scale. Deliberately a
// pure function of world position + time (no per-block depth scaling) so that two
// neighbouring water quads always agree on displacement at their shared edge.
fn wave_height(xz: vec2<f32>, time: f32) -> f32 {
    return sin(xz.x * WAVE_FREQ + time * WAVE_SPEED)
        + sin(xz.y * WAVE_FREQ * 1.3 - time * WAVE_SPEED * 0.8);
}

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;

    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    var world_position =
        mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(vertex.position, 1.0));

    out.world_normal = mesh_functions::mesh_normal_local_to_world(vertex.normal, vertex.instance_index);

#ifdef VERTEX_COLORS
    // is_top (B channel) is a flat 0/1 flag, not depth-scaled - see generator.rs for
    // why uniform amplitude across every top face is required to avoid seams.
    //
    // shore_factor (R channel) instead damps amplitude toward 0 right at the shore:
    // land's wall geometry is static, so an undamped wave can lift water's edge
    // above the adjacent land block and tear the seam there. This is safe to vary
    // per-vertex (unlike depth) because shore_factor is provably identical at any
    // shared edge between two water quads - see vertex_shore_factor in generator.rs.
    let is_top = vertex.color.b;
    let shore_factor = vertex.color.r;
    let time = view_bindings::globals.time;
    let cam_dist = distance(world_position.xyz, view_bindings::view.world_position.xyz);
    let dist_fade = 1.0 - smoothstep(WAVE_FADE_START, WAVE_FADE_END, cam_dist);
    let amplitude = WAVE_AMPLITUDE * (1.0 - shore_factor) * dist_fade;
    world_position.y += wave_height(world_position.xz, time) * amplitude * is_top;
    out.color = vertex.color;
#endif

    out.world_position = world_position;
    out.position = position_world_to_clip(world_position.xyz);
    out.uv = vertex.uv;

    return out;
}

fn hash2(p: vec2<f32>) -> f32 {
    let h = dot(p, vec2<f32>(127.1, 311.7));
    return fract(sin(h) * 43758.5453123);
}

// Reveals a fraction (`coverage`) of a coarse, block-aligned grid - a blocky,
// hard-edged cutout (like Minecraft's animated water/foam texture frames) instead
// of a smooth noise gradient. `frame` steps discretely so the revealed cells
// flicker between frames rather than sliding continuously.
fn foam_cells(xz: vec2<f32>, frame: f32, coverage: f32) -> f32 {
    let cell = floor(xz / FOAM_CELL_SIZE) + vec2<f32>(frame * 13.7, frame * 7.3);
    return step(1.0 - coverage, hash2(cell));
}

@fragment
fn fragment(
    in: VertexOutput,
) -> FragmentOutput {

    let world_position = in.world_position.xyz;
    let view_position = view_bindings::view.world_position.xyz;
    let view_to_world = world_position - view_position;

    let dist = length(view_to_world);
    if dist > 2000.0 {
      discard;
    }

    let normal = normalize(in.world_normal);
    let brightness = dot(normal, normalize(vec3(-0.2, 0.7, 0.2)));

    let color_lit = material_color * textureSample(material_color_texture, material_color_sampler, in.uv);

    let dark = color_lit * 0.7;
    var color = mix(dark, color_lit, brightness);
    color = vec4<f32>(mix(color.rgb, TROPICAL_TINT, TROPICAL_PUSH), color.a);
    var alpha = 0.6;

#ifdef VERTEX_COLORS
    let shore_factor = in.color.r;
    let raw_depth = in.color.g * CHUNK_SIZE;

    // How much of the seafloor's light has been "absorbed" by this depth of water -
    // 0 at the surface, approaching 1 within a few blocks, never quite reaching it.
    let extinction = 1.0 - exp(-raw_depth / EXTINCTION_DISTANCE);

    let shallow_color = color.rgb * 1.15;
    let deep_color = mix(color.rgb, DEEP_TINT, 0.6);
    color = vec4<f32>(mix(shallow_color, deep_color, extinction), color.a);
    alpha = mix(0.3, 0.97, extinction);

    // Foam's noise sampling (up to 6 hash lookups per pixel between the 3 octaves
    // and the frame crossfade) is the most expensive part of this shader, for
    // detail that's imperceptible at distance anyway - branching around it entirely
    // beyond FOAM_FADE_END is a real cost saving, not just a visual fade.
    var foam = 0.0;
    if dist < FOAM_FADE_END {
        // Phase is driven by distance-to-shore (baked per vertex - see
        // vertex_shore_distance, anchored at the shared grid corner so it can't
        // disagree between neighbouring blocks) rather than world xz, so the
        // travelling envelope always advances toward the coast along its local
        // normal instead of sliding sideways along it like a fixed-direction wave.
        let shore_distance = in.color.a;
        let time = view_bindings::globals.time;
        let phase = shore_distance * FOAM_PHASE_SCALE + time * FOAM_SPEED;
        let lap = smoothstep(1.0 - FOAM_BAND, 1.0, sin(phase)) * shore_factor;
        let coverage = clamp(lap * FOAM_DENSITY, 0.0, 1.0);

        // Crossfade between this frame's cell pattern and the next, instead of
        // hard-cutting straight to a whole new random pattern every frame - that cut
        // read as a jarring jump/pop, especially once FOAM_FRAME_RATE was slowed.
        let frame_t = time * FOAM_FRAME_RATE;
        let frame = floor(frame_t);
        let foam_a = foam_cells(world_position.xz, frame, coverage);
        let foam_b = foam_cells(world_position.xz, frame + 1.0, coverage);
        let fade = 1.0 - smoothstep(FOAM_FADE_START, FOAM_FADE_END, dist);
        foam = mix(foam_a, foam_b, fract(frame_t)) * fade;
    }

    color = vec4<f32>(mix(color.rgb, FOAM_COLOR, foam), color.a);
    alpha = mix(alpha, 1.0, foam);
#endif

    // This shader fully replaces Bevy's standard PBR fragment stage, so fog isn't
    // applied automatically the way it would be for StandardMaterial - apply it
    // explicitly so distant water fades into the sky instead of popping in/out.
    let fogged = apply_fog(view_bindings::fog, vec4<f32>(color.rgb, alpha), world_position, view_position);

    var output: FragmentOutput;
    output.color = fogged;
    return output;
}
