import bevy_pbr::render::{
    forward_io::{Vertex, VertexOutput, decompress_vertex},
    mesh_view_bindings as view_bindings,
    mesh_view_types::DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT,
    mesh_functions,
    shadows::fetch_directional_shadow,
    view_transformations::{position_world_to_clip, position_world_to_view},
};
import super::fog::fogged;
import super::dither::dither_threshold;
import super::sky::sky_colour;
import super::lighting::{SKY_GAIN, direct_sun, open_sky};
import super::cloud_shadow::{cloud_shadow, cloud_pattern, cloud_sampler, cloud_layer};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> material_color: vec4<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var material_color_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var material_color_sampler: sampler;

@group(constants::MATERIAL_BIND_GROUP) @binding(3) var<uniform> material_fade: vec4<f32>;


const TROPICAL_TINT: vec3<f32> = vec3<f32>(0.0, 0.85, 0.9);
const TROPICAL_PUSH: f32 = 0.4;
const DEEP_TINT: vec3<f32> = vec3<f32>(0.03, 0.18, 0.3);
// Open ocean colour, shaded towards from tropical between these depths in blocks.
const DEEP_OCEAN_COLOR: vec3<f32> = vec3<f32>(0.004, 0.03, 0.16);
const DEEP_OCEAN_START: f32 = 3.0;
const DEEP_OCEAN_END: f32 = 14.0;
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

// Ripples, as directional waves summed: each one's direction across the water, wavelength in
// blocks, and steepness (its height over its wavelength, times 2 pi). Only their slopes are
// drawn, for the surface's normal: the mesh rises and falls with `wave_height`'s longer swell.
const RIPPLES: i32 = 8;
const RIPPLE_DIRECTIONS: array<vec2<f32>, 8> = array<vec2<f32>, 8>(
    vec2<f32>(0.8, 0.6), vec2<f32>(0.21, 0.978), vec2<f32>(0.954, -0.3), vec2<f32>(-0.39, 0.921),
    vec2<f32>(0.62, -0.785), vec2<f32>(0.995, 0.1), vec2<f32>(-0.71, 0.704), vec2<f32>(0.44, 0.898),
);
const RIPPLE_LENGTHS: array<f32, 8> = array<f32, 8>(13.1, 7.7, 5.3, 3.4, 2.3, 1.7, 1.21, 0.87);
const RIPPLE_STEEPNESS: array<f32, 8> = array<f32, 8>(0.11, 0.1, 0.09, 0.09, 0.08, 0.07, 0.07, 0.06);
// The wind gusts over patches of water roughly this many blocks across, drifting this fast,
// roughening some and calming others, between these shares of the ripples' steepness.
const GUST_SIZE: f32 = 56.0;
const GUST_DRIFT: vec2<f32> = vec2<f32>(1.2, 0.7);
const GUST_RANGE: vec2<f32> = vec2<f32>(0.35, 1.35);
// Ripples fade out once they're this many pixels long or shorter, which would only shimmer.
// What they'd have tilted the surface by roughens it instead, so the sun's glint spreads
// into a glittering path over far water rather than vanishing.
const RIPPLE_MIN_PIXELS: f32 = 4.0;
// How smooth still water is, as the slope's spread: a sharp glint off it.
const CALM_SLOPE_VARIANCE: f32 = 0.002;
// How much light water reflects looking straight down at it, rising to all at a glance.
const REFLECT_STRAIGHT_DOWN: f32 = 0.02;
// How bright the sun's glint is, to how bright it would be off a perfect mirror.
const GLINT: f32 = 0.6;

// Clouds as reflected: their colour, lit as `clouds.wesl` lights them, and how far out they
// fade, to the fog's end, as the clouds themselves do.
const CLOUD_COLOR: vec3<f32> = vec3<f32>(0.95, 0.95, 0.92);
const CLOUD_SUN_SCATTER: f32 = 0.8;
const CLOUD_SKY_SCATTER: f32 = 1.4;

// Water's colours are picked as they should look on screen, so they're turned back into
// the scene colours the camera's exposure and tonemapping take to them. See `tonemap.rs`.
const UNTONEMAP_STEPS: i32 = 4;

fn exposure_scale() -> f32 {
    return exp2(view_bindings::view.color_grading.exposure);
}

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
fn vertex(vertex_in: Vertex) -> VertexOutput {
    let vertex = decompress_vertex(vertex_in, vertex_in.instance_index);
    var out: VertexOutput;

    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    var world_position =
        mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(vertex.position, 1.0));

    out.world_normal = mesh_functions::mesh_normal_local_to_world(vertex.normal, vertex.instance_index);

@if(VERTEX_COLORS) {
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
}

    out.world_position = world_position;
    out.position = position_world_to_clip(world_position.xyz);
    out.uv = vertex.uv;

    return out;
}

fn hash2(p: vec2<f32>) -> f32 {
    let h = dot(p, vec2<f32>(127.1, 311.7));
    return fract(sin(h) * 43758.5453123);
}

// Value noise, 0..1, smooth across whole-number cells.
fn smooth_noise(p: vec2<f32>) -> f32 {
    let cell = floor(p);
    let t = fract(p);
    let u = t * t * (3.0 - 2.0 * t);
    let a = mix(hash2(cell), hash2(cell + vec2<f32>(1.0, 0.0)), u.x);
    let b = mix(hash2(cell + vec2<f32>(0.0, 1.0)), hash2(cell + vec2<f32>(1.0, 1.0)), u.x);
    return mix(a, b, u.y);
}

// Reveals a fraction (`coverage`) of a coarse, block-aligned grid - a blocky,
// hard-edged cutout (like Minecraft's animated water/foam texture frames) instead
// of a smooth noise gradient. `frame` steps discretely so the revealed cells
// flicker between frames rather than sliding continuously.
fn foam_cells(xz: vec2<f32>, frame: f32, coverage: f32) -> f32 {
    let cell = floor(xz / FOAM_CELL_SIZE) + vec2<f32>(frame * 13.7, frame * 7.3);
    return step(1.0 - coverage, hash2(cell));
}

fn tonemap_curve(v: vec3<f32>) -> vec3<f32> {
    return 1.0 - exp(-v);
}

// Bevy's somewhat boring display transform, after exposure.
fn displayed(scene: vec3<f32>) -> vec3<f32> {
    let colour = scene * exposure_scale();
    let luma = dot(colour, vec3<f32>(0.2126, 0.7152, 0.0722));
    let chroma = vec2<f32>(
        dot(colour, vec3<f32>(-0.1146, -0.3854, 0.5)),
        dot(colour, vec3<f32>(0.5, -0.4542, -0.0458)),
    );
    let bt = 1.0 - exp(-length(chroma) * 2.4);
    let desat_amount = max((bt - 0.7) * 0.8, 0.0);
    let desat = mix(colour, vec3<f32>(luma), desat_amount * desat_amount);
    let tm0 = colour * max(0.0, (1.0 - exp(-luma)) / max(1e-5, luma));
    return mix(tm0, tonemap_curve(desat), bt * bt) * 0.97;
}

// Close to the identity for all but bright colours, so a few nudges towards the target
// by however far off it still is land near enough.
fn scene_colour(display: vec3<f32>) -> vec3<f32> {
    let scale = exposure_scale();
    var scene = display / scale;
    for (var i = 0; i < UNTONEMAP_STEPS; i++) {
        scene = max(scene + (display - displayed(scene)) * 1.5 / scale, vec3<f32>(0.0));
    }
    return scene;
}

// The water's slope at `xz` from the ripples, d(height)/dx and d(height)/dz, and the
// spread of slope left out where ripples are too fine for `footprint`, the blocks a
// pixel spans there.
fn ripples(xz: vec2<f32>, time: f32, footprint: f32) -> vec3<f32> {
    var slope = vec2<f32>(0.0);
    var lost = 0.0;
    let gust = mix(GUST_RANGE.x, GUST_RANGE.y, smooth_noise((xz - GUST_DRIFT * time) / GUST_SIZE));
    for (var i = 0; i < RIPPLES; i++) {
        let k = 6.2831853 / RIPPLE_LENGTHS[i];
        // deep water waves travel at sqrt(g / k), here in blocks a second with a game's gravity
        let speed = sqrt(9.8 / k);
        let direction = RIPPLE_DIRECTIONS[i];
        let phase = k * (dot(direction, xz) - speed * time);
        let shown = 1.0 - smoothstep(RIPPLE_MIN_PIXELS * 0.5, RIPPLE_MIN_PIXELS, RIPPLE_LENGTHS[i] / footprint);
        let fine = 1.0 - shown;
        let steepness = RIPPLE_STEEPNESS[i] * gust;
        slope += direction * steepness * cos(phase) * fine;
        lost += 0.5 * steepness * steepness * shown;
    }
    return vec3<f32>(slope, lost);
}

// The sky reflected along `direction` from `origin`, with the clouds in it where it rises to them.
fn reflected_sky(origin: vec3<f32>, direction: vec3<f32>) -> vec3<f32> {
    // a reflection pointing down would be of the water itself, so it's kept to the sky
    let sky = sky_colour(normalize(vec3<f32>(direction.x, max(direction.y, 0.02), direction.z)));
    let layer = cloud_layer.x;
    if direction.y <= 0.0 || origin.y >= layer || cloud_layer.y <= 0.0 {
        return sky;
    }
    let hit = origin.xz + direction.xz * ((layer - origin.y) / direction.y);
    // `globals.time` wraps when the wind has carried the clouds a whole number of patterns
    let drift = cloud_layer.zw * view_bindings::globals.time;
    let thickness = textureSampleLevel(cloud_pattern, cloud_sampler, (hit - drift) / cloud_layer.y, 0.0).r;
    let away = distance(hit, view_bindings::view.world_position.xz);
    let end = max(view_bindings::fog.be.y, 1.0);
    let cover = smoothstep(0.1, 0.5, thickness) * (1.0 - smoothstep(0.4 * end, end, away));
    // seen from below, steeply, or from the side, low down
    let side = normalize(vec3<f32>(-direction.x, 0.0, -direction.z) + vec3<f32>(0.0, 0.0, 1e-4));
    let facing = normalize(mix(side, vec3<f32>(0.0, -1.0, 0.0), direction.y));
    let lit = direct_sun(facing) * CLOUD_SUN_SCATTER + open_sky(facing) * CLOUD_SKY_SCATTER;
    return mix(sky, CLOUD_COLOR * lit, cover);
}

// How much of the light arriving at an angle with cosine `cos_theta` off the normal water
// reflects, by Schlick's approximation.
fn fresnel(cos_theta: f32) -> f32 {
    return REFLECT_STRAIGHT_DOWN + (1.0 - REFLECT_STRAIGHT_DOWN) * pow(1.0 - clamp(cos_theta, 0.0, 1.0), 5.0);
}

// The sun's glint towards `to_eye` off water turned to `normal`, whose slopes spread by
// `variance`, in scene colour: a Beckmann highlight, widening as the water roughens.
fn glint(normal: vec3<f32>, to_eye: vec3<f32>, variance: f32, world_position: vec3<f32>, frag_coord: vec2<f32>) -> vec3<f32> {
    if view_bindings::lights.n_directional_lights == 0u {
        return vec3<f32>(0.0);
    }
    let sun = &view_bindings::lights.directional_lights[0];
    let to_sun = (*sun).direction_to_light;
    let n_dot_l = dot(normal, to_sun);
    let n_dot_v = dot(normal, to_eye);
    if n_dot_l <= 0.0 || n_dot_v <= 0.0 {
        return vec3<f32>(0.0);
    }
    let half_way = normalize(to_sun + to_eye);
    let n_dot_h = max(dot(normal, half_way), 1e-4);
    // Beckmann's spread of facets, with its roughness squared twice the slopes' variance
    let alpha2 = 2.0 * variance;
    let cos2 = n_dot_h * n_dot_h;
    let facets = exp((cos2 - 1.0) / (alpha2 * cos2)) / (3.14159265 * alpha2 * cos2 * cos2);
    let reflected = facets * fresnel(dot(half_way, to_eye)) / (4.0 * n_dot_v);
    let radiance = (*sun).color.rgb * view_bindings::view.exposure / SKY_GAIN;
    // in the land's shadow, no glint
    if ((*sun).flags & DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT) == 0u {
        return radiance * reflected * n_dot_l * GLINT;
    }
    let view_z = position_world_to_view(world_position).z;
    let shadow = fetch_directional_shadow(0u, vec4<f32>(world_position, 1.0), normal, view_z, frag_coord);
    let reach = (*sun).cascades[(*sun).num_cascades - 1u].far_bound;
    let shaded = mix(shadow, 1.0, smoothstep(reach * 0.8, reach, -view_z));
    return radiance * reflected * n_dot_l * GLINT * shaded;
}

@fragment
fn fragment(
    in: VertexOutput,
) -> FragmentOutput {

    let world_position = in.world_position.xyz;
    let view_position = view_bindings::view.world_position.xyz;
    let view_to_world = world_position - view_position;

    let dist = length(view_to_world);

    let normal = normalize(in.world_normal);
    let brightness = dot(normal, normalize(vec3(-0.2, 0.7, 0.2)));

    let color_lit = material_color * textureSample(material_color_texture, material_color_sampler, in.uv);

    let dark = color_lit * 0.7;
    var color = mix(dark, color_lit, brightness);
    color = vec4<f32>(mix(color.rgb, TROPICAL_TINT, TROPICAL_PUSH), color.a);
    var alpha = 0.6;

@if(VERTEX_COLORS) {
    let shore_factor = in.color.r;
    let raw_depth = in.color.g * CHUNK_SIZE;

    // How much of the seafloor's light has been "absorbed" by this depth of water -
    // 0 at the surface, approaching 1 within a few blocks, never quite reaching it.
    let extinction = 1.0 - exp(-raw_depth / EXTINCTION_DISTANCE);

    let shallow_color = color.rgb * 1.15;
    let deep_color = mix(color.rgb, DEEP_TINT, 0.6);
    color = vec4<f32>(mix(shallow_color, deep_color, extinction), color.a);
    alpha = mix(0.3, 0.97, extinction);

    let ocean = smoothstep(DEEP_OCEAN_START, DEEP_OCEAN_END, raw_depth);
    let ocean_lit = DEEP_OCEAN_COLOR * mix(0.7, 1.0, brightness);
    color = vec4<f32>(mix(color.rgb, ocean_lit, ocean), color.a);
    // open ocean is solid, or the seabed's terraces show through its dark colour
    alpha = mix(alpha, 1.0, ocean);

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
}

    // What the water shows of itself, from below the surface, and the sky and sun it
    // reflects. Looking up at it from underwater, only itself.
    var scene = scene_colour(color.rgb);
    let to_eye = -view_to_world / dist;
    // blocks a pixel spans across the water here, worked out where every fragment does
    let footprint = max(length(dpdx(world_position.xz)), length(dpdy(world_position.xz)));
    let above = view_position.y > world_position.y && normal.y > 0.5;
@if(SOLID_WATER) {
    // nothing shows through, so it's all the water's own colour that isn't reflected
    alpha = 1.0;
}
    if above {
        let time = view_bindings::globals.time;
        let ripple = ripples(world_position.xz, time, footprint);
        let surface = normalize(vec3<f32>(-ripple.x, 1.0, -ripple.y));
        let variance = CALM_SLOPE_VARIANCE + ripple.z;
        let sky = reflected_sky(world_position, reflect(-to_eye, surface));
        let reflected = fresnel(dot(surface, to_eye));
        let sun = glint(surface, to_eye, variance, world_position, in.position.xy) * cloud_shadow(world_position);
        // blended over what's behind: the reflection and glint stay as bright however clear the water
        let body = scene * alpha * (1.0 - reflected);
        let covered = alpha * (1.0 - reflected) + reflected;
        scene = (body + sky * reflected + sun) / max(covered, 1e-4);
        alpha = covered;
    }

    // so distant water fades into the sky instead of popping in and out
    let shown = fogged(vec4<f32>(scene, alpha), world_position, in.position.xy);

    // after the texture samples above, which need uniform control flow
    let threshold = dither_threshold(in.position.xy);
    if threshold < material_fade.x || threshold >= material_fade.y {
        discard;
    }

    var output: FragmentOutput;
    output.color = shown;
    return output;
}
