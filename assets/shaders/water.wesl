import bevy_pbr::render::{
    forward_io::{Vertex, VertexOutput, decompress_vertex},
    mesh_view_bindings as view_bindings,
    mesh_view_types::DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT,
    mesh_functions,
    shadows::fetch_directional_shadow,
    view_transformations::{
        depth_ndc_to_view_z, ndc_to_uv, position_world_to_clip, position_world_to_ndc,
        position_world_to_view,
    },
};
import bevy_pbr::prepass::utils as prepass_utils;
import super::fog::fogged;
import super::dither::dither_threshold;
import super::sky::sky_colour;
import super::lighting::{cloud_light, daylight, moon_radiance, sun_radiance};
import super::sky::{MOON_LIGHT, SUN_LIGHT};
import super::stars::reflected_celestial;
import super::cloud_shadow::{cloud_shadow, cloud_pattern, cloud_sampler, cloud_layer};
import super::curvature::{curved, uncurved};
import super::hash::hash2;

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> material_color: vec4<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var material_color_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var material_color_sampler: sampler;

@group(constants::MATERIAL_BIND_GROUP) @binding(3) var<uniform> material_fade: vec4<f32>;
// between these distances, x to y, see-through water turns opaque, see `WaterMaterial`
@group(constants::MATERIAL_BIND_GROUP) @binding(7) var<uniform> see_through: vec4<f32>;


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

// The swell the mesh rises and falls with: waves from around the wind's direction, each
// at an angle off it in radians, a wavelength and a height in blocks, crest to trough
// about twice that. Their crests are sharp and troughs broad, see `swell_shape`.
const SWELLS: i32 = 5;
const WIND: vec2<f32> = vec2<f32>(0.8, 0.6);
const SWELL_ANGLES: array<f32, 5> = array<f32, 5>(0.0, 0.8, -1.05, 1.9, -0.45);
const SWELL_LENGTHS: array<f32, 5> = array<f32, 5>(71.0, 43.0, 29.0, 17.0, 11.0);
const SWELL_HEIGHTS: array<f32, 5> = array<f32, 5>(0.55, 0.38, 0.25, 0.15, 0.08);
// How peaked crests are: 0 is a sine wave.
const SWELL_SHARPNESS: f32 = 1.2;
// Swell runs slower than deep water waves of its length would, for a calmer sea.
const SWELL_PACE: f32 = 0.55;
// Swell builds up from this depth in blocks to its full height at this one.
const SWELL_DEPTH: vec2<f32> = vec2<f32>(0.5, 6.0);
// Out over deep water, between these depths, the longest waves grow up to this many times
// taller, by these shares for each, into big rolling crests, while the short ones stay
// as they are. Full-detail chunks only count depth to the bottom of their own 16 blocks, so
// it's all grown by then, or the swell would change height where LOD water takes over.
const DEEP_SWELL_DEPTH: vec2<f32> = vec2<f32>(8.0, 15.0);
const DEEP_SWELL: f32 = 3.5;
const DEEP_SWELL_SHARES: array<f32, 5> = array<f32, 5>(1.0, 0.8, 0.45, 0.15, 0.0);
// Each wave fades out between these many of its wavelengths from the camera, where LOD
// cells grow too wide to carry it. By distance alone, so meshes side by side agree.
const SWELL_FADE: vec2<f32> = vec2<f32>(14.0, 28.0);
// All waves are laid on the water swirled at two scales, each bending it by up to so many
// blocks over patches so many across, drifting so fast, so their crests curve and wander
// instead of running straight across the world. Kept gentle enough that the water's never
// stretched by more than about half: past that it folds over and crests double back.
const SWIRLS: i32 = 2;
const SWIRL_REACH: array<f32, 2> = array<f32, 2>(90.0, 24.0);
const SWIRL_SIZE: array<f32, 2> = array<f32, 2>(760.0, 200.0);
const SWIRL_DRIFT: array<vec2<f32>, 2> = array<vec2<f32>, 2>(vec2<f32>(0.25, -0.15), vec2<f32>(0.6, -0.4));
// And each wave slips in and out of step with the others from place to place, by up to this
// many of its cycles over patches this many of its wavelengths across, so where they meet
// they never line up into the same rows twice.
const PHASE_DRIFT: f32 = 1.5;
const PHASE_SCALE: f32 = 20.0;
const FOAM_FADE_END: f32 = 192.0;
const FOAM_FADE_START: f32 = 144.0;

// Ripples, as directional waves summed: each one's direction across the water, wavelength in
// blocks, and steepness (its height over its wavelength, times 2 pi). Only their slopes are
// drawn, for the surface's normal: the mesh rises and falls with the longer `swell`.
const RIPPLES: i32 = 8;
const RIPPLE_DIRECTIONS: array<vec2<f32>, 8> = array<vec2<f32>, 8>(
    vec2<f32>(0.800, 0.600), vec2<f32>(-0.221, 0.975), vec2<f32>(0.792, -0.610), vec2<f32>(-0.901, 0.433),
    vec2<f32>(-0.086, -0.996), vec2<f32>(0.414, 0.910), vec2<f32>(-0.955, -0.297), vec2<f32>(1.000, -0.006),
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

// Clouds as reflected: their colour, lit by `cloud_light` as the clouds are, and how far out they
// fade, to the fog's end, as the clouds themselves do.
const CLOUD_COLOR: vec3<f32> = vec3<f32>(0.95, 0.95, 0.92);

// Water's colours are picked as they should look on screen, so they're turned back into
// the scene colours the camera's exposure and tonemapping take to them. See `tonemap.rs`.
const UNTONEMAP_STEPS: i32 = 4;

fn exposure_scale() -> f32 {
    return exp2(view_bindings::view.color_grading.exposure);
}

struct FragmentOutput {
  @location(0) color: vec4<f32>
}

// Where waves are laid on the water at `xz`, bent by drifting noise. A pure function of
// position and time, as everything the mesh moves by is, so neighbouring quads agree.
fn warped(xz: vec2<f32>, time: f32) -> vec2<f32> {
    var at = xz;
    for (var i = 0; i < SWIRLS; i++) {
        let p = (xz + SWIRL_DRIFT[i] * time) / SWIRL_SIZE[i] + vec2<f32>(f32(i) * 53.7, 0.0);
        let bend = vec2<f32>(smooth_noise(p), smooth_noise(p + vec2<f32>(17.3, 31.1))) * 2.0 - 1.0;
        at += bend * SWIRL_REACH[i];
    }
    return at;
}

// How far out of step wave `index`, `length` blocks long, has slipped at `xz`, in radians.
fn phase_drift(xz: vec2<f32>, index: i32, length: f32) -> f32 {
    let p = xz / (PHASE_SCALE * length) + vec2<f32>(f32(index) * 37.1, f32(index) * 11.7);
    return (smooth_noise(p) * 2.0 - 1.0) * PHASE_DRIFT * 6.2831853;
}

// How rough the wind leaves the water at `xz`, a share of the waves' full size.
fn gust(xz: vec2<f32>, time: f32) -> f32 {
    return mix(GUST_RANGE.x, GUST_RANGE.y, smooth_noise((xz - GUST_DRIFT * time) / GUST_SIZE));
}

// A peaked wave's height at `phase`, about -0.6..1, and its rate of change with the phase.
fn swell_shape(phase: f32) -> vec2<f32> {
    let peak = exp(SWELL_SHARPNESS * (sin(phase) - 1.0));
    // less its mean, so the water's level stays put
    let mean = 0.42;
    return vec2<f32>((peak - mean) / (1.0 - mean), peak * SWELL_SHARPNESS * cos(phase) / (1.0 - mean));
}

// The swell at `xz`, `distance` blocks from the camera: its height, and its slope,
// d(height)/dx and d(height)/dz. The slope left out by waves faded with distance comes back
// as `lost`, to roughen the surface by instead.
struct Swell {
    height: f32,
    slope: vec2<f32>,
    lost: f32,
}

fn swell(xz: vec2<f32>, time: f32, distance: f32, size: f32, deep: f32) -> Swell {
    var out = Swell(0.0, vec2<f32>(0.0), 0.0);
    let at = warped(xz, time);
    let wind_angle = atan2(WIND.y, WIND.x);
    for (var i = 0; i < SWELLS; i++) {
        let angle = wind_angle + SWELL_ANGLES[i];
        let direction = vec2<f32>(cos(angle), sin(angle));
        let k = 6.2831853 / SWELL_LENGTHS[i];
        let speed = sqrt(9.8 / k) * SWELL_PACE;
        let drift = phase_drift(xz, i, SWELL_LENGTHS[i]);
        let shape = swell_shape(k * (dot(direction, at) - speed * time) + drift);
        let fade = 1.0 - smoothstep(SWELL_FADE.x * SWELL_LENGTHS[i], SWELL_FADE.y * SWELL_LENGTHS[i], distance);
        let height = SWELL_HEIGHTS[i] * size * (1.0 + (DEEP_SWELL - 1.0) * DEEP_SWELL_SHARES[i] * deep);
        out.height += height * shape.x * fade;
        let steepness = height * k * shape.y;
        out.slope += direction * steepness * fade;
        out.lost += 0.5 * height * height * k * k * (1.0 - fade);
    }
    return out;
}

// How deep the water is, 0..1, for the swell to grow over, see `DEEP_SWELL`.
fn deep_water(depth: f32) -> f32 {
    return smoothstep(DEEP_SWELL_DEPTH.x, DEEP_SWELL_DEPTH.y, depth);
}

// How much of its full size the swell has at a vertex or fragment: calmer over shallows
// and gone at the shore, see the vertex shader.
fn swell_size(shore_factor: f32, depth: f32, xz: vec2<f32>, time: f32) -> f32 {
    return (1.0 - shore_factor) * smoothstep(SWELL_DEPTH.x, SWELL_DEPTH.y, depth) * gust(xz, time);
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
    // is_top (B channel) is a flat 0/1 flag: only the surface moves.
    //
    // shore_factor (R channel) damps the swell toward 0 right at the shore: land's wall
    // geometry is static, so an undamped wave can lift water's edge above the adjacent
    // land block and tear the seam there. It and the depth (G channel), averaged over
    // the columns round each corner, are the same at any corner two water quads share
    // - see vertex_shore_factor and vertex_water_depth in generator.rs - so the swell
    // can vary with them without tearing quads apart.
    let is_top = vertex.color.b;
    let shore_factor = vertex.color.r;
    let depth = vertex.color.g * CHUNK_SIZE;
    let time = view_bindings::globals.time;
    let cam_dist = distance(world_position.xyz, view_bindings::view.world_position.xyz);
    let size = swell_size(shore_factor, depth, world_position.xz, time);
    world_position.y += swell(world_position.xz, time, cam_dist, size, deep_water(depth)).height * is_top;
    out.color = vertex.color;
}

    out.world_position = vec4<f32>(curved(world_position.xyz), 1.0);
    out.position = position_world_to_clip(out.world_position.xyz);
    out.uv = vertex.uv;

    return out;
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
    let gusty = gust(xz, time);
    let at = warped(xz, time);
    for (var i = 0; i < RIPPLES; i++) {
        let k = 6.2831853 / RIPPLE_LENGTHS[i];
        // deep water waves travel at sqrt(g / k), here in blocks a second with a game's gravity
        let speed = sqrt(9.8 / k);
        let direction = RIPPLE_DIRECTIONS[i];
        let phase = k * (dot(direction, at) - speed * time) + phase_drift(xz, i + SWELLS, RIPPLE_LENGTHS[i]);
        let shown = 1.0 - smoothstep(RIPPLE_MIN_PIXELS * 0.5, RIPPLE_MIN_PIXELS, RIPPLE_LENGTHS[i] / footprint);
        let fine = 1.0 - shown;
        let steepness = RIPPLE_STEEPNESS[i] * gusty;
        slope += direction * steepness * cos(phase) * fine;
        lost += 0.5 * steepness * steepness * shown;
    }
    return vec3<f32>(slope, lost);
}

// The sky reflected along `direction` from `origin`, with the clouds in it where it rises to them.
fn reflected_sky(origin: vec3<f32>, direction: vec3<f32>) -> vec3<f32> {
    // a reflection pointing down would be of the water itself, so it's kept to the sky
    let up = normalize(vec3<f32>(direction.x, max(direction.y, 0.02), direction.z));
    let night = reflected_celestial(up);
    let sky = sky_colour(up) * (1.0 - night.a) + night.rgb;
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
    let lit = cloud_light(facing, vec3<f32>(hit.x, layer, hit.y));
    return mix(sky, CLOUD_COLOR * lit, cover);
}

// Where reflections are looked up across the spread of slopes waves too fine to draw would
// have had, as offsets in standard deviations: a spiral through a disc.
const REFLECTION_TAPS: i32 = 8;
const REFLECTION_SPREAD: array<vec2<f32>, 8> = array<vec2<f32>, 8>(
    vec2<f32>(0.35, 0.0), vec2<f32>(-0.42, 0.55), vec2<f32>(-0.38, -0.78), vec2<f32>(0.98, -0.52),
    vec2<f32>(0.21, 1.25), vec2<f32>(-1.37, 0.18), vec2<f32>(-0.06, -1.5), vec2<f32>(1.38, 0.9),
);

// The sky and clouds reflected towards `to_eye` off water at `origin` sloping by `slope`,
// in rgb, and how much it reflects, in a. Where waves are too fine to draw, so `spread`
// over their slopes is wide, it's the mean over that spread: clouds blur and smear
// towards the eye, as they do on a real sea, instead of reflecting as off a mirror.
fn rough_reflection(origin: vec3<f32>, slope: vec2<f32>, spread: f32, to_eye: vec3<f32>) -> vec4<f32> {
    var light = vec3<f32>(0.0);
    var reflected = 0.0;
    for (var i = 0; i < REFLECTION_TAPS; i++) {
        let tilted = slope + REFLECTION_SPREAD[i] * spread;
        let normal = normalize(vec3<f32>(-tilted.x, 1.0, -tilted.y));
        // facets turned away from the eye aren't seen
        let facing = dot(normal, to_eye);
        if facing <= 0.0 {
            continue;
        }
        let share = fresnel(facing);
        light += reflected_sky(origin, reflect(-to_eye, normal)) * share;
        reflected += share;
    }
    let mean = reflected / f32(REFLECTION_TAPS);
    return vec4<f32>(light / max(reflected, 1e-4), mean);
}

// How much of the light arriving at an angle with cosine `cos_theta` off the normal water
// reflects, by Schlick's approximation.
fn fresnel(cos_theta: f32) -> f32 {
    return REFLECT_STRAIGHT_DOWN + (1.0 - REFLECT_STRAIGHT_DOWN) * pow(1.0 - clamp(cos_theta, 0.0, 1.0), 5.0);
}

// How much of the light from `to_light` water turned to `normal` glints towards `to_eye`, whose
// slopes spread by `variance`: a Beckmann highlight, widening as the water roughens.
fn highlight(normal: vec3<f32>, to_eye: vec3<f32>, variance: f32, to_light: vec3<f32>) -> f32 {
    let n_dot_l = dot(normal, to_light);
    let n_dot_v = dot(normal, to_eye);
    if n_dot_l <= 0.0 || n_dot_v <= 0.0 {
        return 0.0;
    }
    let half_way = normalize(to_light + to_eye);
    let n_dot_h = max(dot(normal, half_way), 1e-4);
    // Beckmann's spread of facets, with its roughness squared twice the slopes' variance
    let alpha2 = 2.0 * variance;
    let cos2 = n_dot_h * n_dot_h;
    let facets = exp((cos2 - 1.0) / (alpha2 * cos2)) / (3.14159265 * alpha2 * cos2 * cos2);
    let reflected = facets * fresnel(dot(half_way, to_eye)) / (4.0 * n_dot_v);
    return reflected * n_dot_l * GLINT;
}

// The sun's glint, and the moon's, towards `to_eye` off water turned to `normal`, in scene colour.
fn glint(normal: vec3<f32>, to_eye: vec3<f32>, variance: f32, world_position: vec3<f32>, frag_coord: vec2<f32>) -> vec3<f32> {
    if view_bindings::lights.n_directional_lights == 0u {
        return vec3<f32>(0.0);
    }
    let sun = &view_bindings::lights.directional_lights[SUN_LIGHT];
    var moonlit = vec3<f32>(0.0);
    if view_bindings::lights.n_directional_lights > 1u {
        moonlit = moon_radiance() * highlight(normal, to_eye, variance, view_bindings::lights.directional_lights[MOON_LIGHT].direction_to_light);
    }
    let sunlit = sun_radiance(uncurved(world_position)) * highlight(normal, to_eye, variance, (*sun).direction_to_light);
    // in the land's shadow, no glint
    if ((*sun).flags & DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT) == 0u {
        return sunlit + moonlit;
    }
    let view_z = position_world_to_view(world_position).z;
    let shadow = fetch_directional_shadow(SUN_LIGHT, vec4<f32>(world_position, 1.0), normal, view_z, frag_coord);
    let reach = (*sun).cascades[(*sun).num_cascades - 1u].far_bound;
    let shaded = mix(shadow, 1.0, smoothstep(reach * 0.8, reach, -view_z));
    return sunlit * shaded + moonlit;
}

// Fresh water, a river's or a lake's, is clear: it takes a little of each colour out of the light that
// reaches the bed and back, red first and fastest, per block of depth it passes through, and adds a
// faint scatter of its own, so a shallow bed shows true and a deep pool goes slowly green and blue.
const FRESH_ABSORB: vec3<f32> = vec3<f32>(0.22, 0.07, 0.04);
const FRESH_SCATTER: vec3<f32> = vec3<f32>(0.03, 0.10, 0.14);
// The water must lie this far over the sea for its surface to be fresh: the sea's own is the ocean's.
const FRESH_ABOVE_SEA: f32 = 0.6;
// How far, in screen heights, ripples bend what's seen through the surface, per block of depth.
// How much of the water's own colour shows over the bed at any depth.
const FRESH_TINT: f32 = 0.1;
const REFRACTION: f32 = 0.035;
// How strong fresh water's ripples are next to the ocean's: it's calmer.
const FRESH_RIPPLES: f32 = 0.15;
// Fresh water's speed downhill per unit of slope, in blocks a second, its most, and how much more rippled it is at that.
const FLOW_SPEED: f32 = 12.0;
const FLOW_MAX: f32 = 4.0;
const FLOW_ROUGHEN: f32 = 1.5;
// Seconds ripples are carried before fading to a fresh copy, so they never stretch.
const FLOW_PERIOD: f32 = 1.2;
// The cosine off straight up of the edge of what's seen out of the water from under it, by its refraction, and how
// much of what's above shows past that edge and within it.
const SNELL_WINDOW: f32 = 0.661;
const BELOW_CLEAR: vec2<f32> = vec2<f32>(0.7, 0.95);
// The reflection ray marches this many steps over up to this many blocks, and counts as having
// reached a surface it has gone this far behind.
const REFLECTION_STEPS: i32 = 20;
// Farther than this the bed isn't read through the water, and farther than the second the world isn't
// reflected in it: only the sky is, which costs far less.
const BED_REACH: f32 = 96.0;
const FAR_BED: vec3<f32> = vec3<f32>(0.42, 0.44, 0.42);
const REFLECTION_NEAR: f32 = 160.0;
const REFLECTION_REACH: f32 = 64.0;
const REFLECTION_THICKNESS: f32 = 1.2;

// 0..1 noise from a pixel's position, for jittering the march so its steps don't band.
fn interleaved_noise(pixel: vec2<f32>) -> f32 {
    return fract(52.9829189 * fract(dot(pixel, vec2<f32>(0.06711056, 0.00583715))));
}

// What the screen shows where a ray from `origin` going `direction` first passes behind it, and how
// sure that is, 0..1: the opaque scene's colour there in rgb. Nothing where it leaves the screen or
// never meets anything, which the sky reflection fills in.
fn screen_reflection(origin: vec3<f32>, direction: vec3<f32>, frag_coord: vec4<f32>) -> vec4<f32> {
@if(DEPTH_PREPASS && VIEW_TRANSMISSION_TEXTURE) {
    let viewport = view_bindings::view.viewport.zw;
    let jitter = interleaved_noise(frag_coord.xy + vec2<f32>(f32(view_bindings::globals.frame_count % 16u) * 5.3, 0.0));
    let unit = REFLECTION_REACH / f32(REFLECTION_STEPS * REFLECTION_STEPS);
    var travelled = 0.0;
    var last = 0.0;
    for (var i = 1; i <= REFLECTION_STEPS; i++) {
        // steps lengthen with the distance, so nearer things are found more exactly
        last = travelled;
        travelled = unit * f32(i * i) + unit * jitter * f32(2 * i - 1);
        let point = origin + direction * travelled;
        let ndc = position_world_to_ndc(point);
        if ndc.z <= 0.0 || abs(ndc.x) >= 1.0 || abs(ndc.y) >= 1.0 {
            break;
        }
        let uv = ndc_to_uv(ndc.xy);
        let scene_depth = prepass_utils::prepass_depth(vec4<f32>(uv * viewport, 0.0, 0.0), 0u);
        // behind the surface seen there, within its thickness
        let gap = depth_ndc_to_view_z(scene_depth) - position_world_to_view(point).z;
        if gap > 0.0 && gap < REFLECTION_THICKNESS + 0.04 * travelled {
            // the hit is between the last step and this one: halve the interval a few times
            var low = last;
            var high = travelled;
            for (var k = 0; k < 4; k++) {
                let mid = 0.5 * (low + high);
                let p = origin + direction * mid;
                let n = position_world_to_ndc(p);
                let d = prepass_utils::prepass_depth(vec4<f32>(ndc_to_uv(n.xy) * viewport, 0.0, 0.0), 0u);
                if depth_ndc_to_view_z(d) - position_world_to_view(p).z > 0.0 {
                    high = mid;
                } else {
                    low = mid;
                }
            }
            let hit = position_world_to_ndc(origin + direction * high);
            let hit_uv = ndc_to_uv(hit.xy);
            let edge = min(hit_uv, vec2<f32>(1.0) - hit_uv);
            let fade = smoothstep(0.0, 0.08, min(edge.x, edge.y)) * (1.0 - smoothstep(0.6, 1.0, high / REFLECTION_REACH));
            let colour = textureSampleLevel(view_bindings::view_transmission_texture, view_bindings::view_transmission_sampler, hit_uv, 0.0).rgb;
            return vec4<f32>(colour, fade);
        }
    }
}
    return vec4<f32>(0.0);
}

// Fresh water seen from above: what's under the surface, through ripples that bend it and a depth
// of clear water that tints it, and the sky, the sun and the world about it reflected off the top.
fn fresh_water(world_position: vec3<f32>, normal: vec3<f32>, to_eye: vec3<f32>, frag_coord: vec4<f32>, depth: f32, footprint: f32, dist: f32, tint: vec3<f32>) -> vec3<f32> {
    let time = view_bindings::globals.time;
    // ripples carried down the slope the normal leans to, as two copies half a period apart faded across each other
    var flow = normal.xz / max(normal.y, 0.2) * FLOW_SPEED * f32(normal.y > 0.5);
    flow *= min(1.0, FLOW_MAX / max(length(flow), 1e-4));
    let speed = length(flow) / FLOW_MAX;
    // still ripples only show where it barely runs
    var ripple = vec3<f32>(0.0);
    if speed < 0.1 {
        ripple = ripples(world_position.xz, time, footprint);
    }
    if speed > 0.01 {
        let t = fract(time / FLOW_PERIOD);
        let a = ripples(world_position.xz - flow * FLOW_PERIOD * t, time, footprint);
        let b = ripples(world_position.xz - flow * FLOW_PERIOD * fract(t + 0.5) + vec2<f32>(7.3, 3.1), time, footprint);
        let wa = 1.0 - abs(1.0 - 2.0 * t);
        let wb = 1.0 - wa;
        let carried = vec3<f32>((a.xy * wa + b.xy * wb) / sqrt(wa * wa + wb * wb), a.z * wa + b.z * wb);
        ripple = mix(ripple, carried, smoothstep(0.01, 0.1, speed));
        ripple *= 1.0 + FLOW_ROUGHEN * speed;
    }
    let slope = ripple.xy * FRESH_RIPPLES;
    let surface = normalize(vec3<f32>(-slope.x, 1.0, -slope.y));
    let variance = CALM_SLOPE_VARIANCE + ripple.z * FRESH_RIPPLES * FRESH_RIPPLES;

    // the bed shows only near, easing to a typical bed's grey past that, or where nothing can be read of it
    let far_bed = FAR_BED * daylight(uncurved(world_position));
    var behind = far_bed;
@if(VIEW_TRANSMISSION_TEXTURE) {
    if dist < BED_REACH {
    let viewport = view_bindings::view.viewport.zw;
    let uv = frag_coord.xy / viewport;
    // the bed bends toward where the ripples lean, more the deeper it is, and less far off
    var bent = uv + slope * REFRACTION * clamp(depth, 0.3, 3.0) / (1.0 + 0.03 * dist);
@if(DEPTH_PREPASS) {
    // something nearer than the water, a bank or a tree, isn't seen through it
    let water_z = position_world_to_view(world_position).z;
    let seen_z = depth_ndc_to_view_z(prepass_utils::prepass_depth(vec4<f32>(bent * viewport, 0.0, 0.0), 0u));
    if seen_z > water_z {
        bent = uv;
    }
}
    let bed = textureSampleLevel(view_bindings::view_transmission_texture, view_bindings::view_transmission_sampler, clamp(bent, vec2<f32>(0.001), vec2<f32>(0.999)), 0.0).rgb;
    behind = mix(bed, far_bed, smoothstep(0.5 * BED_REACH, BED_REACH, dist));
    }
}
    // the light's path through the water there and back, longer seen at a slant
    let path = max(depth, 1.5) / max(to_eye.y, 0.25);
    let transmitted = exp(-FRESH_ABSORB * path);
    let scatter = scene_colour(FRESH_SCATTER) * daylight(uncurved(world_position));
    // however shallow, a lake's water is plainly blue, not just the bed's colour
    let body = mix(behind * transmitted + scatter * (1.0 - transmitted), tint, FRESH_TINT);

    let reflect_dir = reflect(-to_eye, surface);
    // a calm lake mirrors more than the plain fresnel says, enough to show the trees from above
    let share = min(fresnel(dot(surface, to_eye)) * 1.6 + 0.03, 0.75);
    let sky = reflected_sky(uncurved(world_position), reflect_dir);
    var world = vec4<f32>(0.0);
    if dist < REFLECTION_NEAR && share > 0.06 {
        world = screen_reflection(world_position, reflect_dir, frag_coord);
    }
    let reflection = mix(sky, world.rgb, world.a);
    let sun = glint(surface, to_eye, variance, world_position, frag_coord.xy) * cloud_shadow(uncurved(world_position));
    return body * (1.0 - share) + reflection * share + sun;
}

// Fresh water's surface seen from under it: what's above, bent by the ripples, clearest through Snell's window straight
// up and tinted more past it, where real water would only mirror what's under it.
fn fresh_water_below(world_position: vec3<f32>, to_eye: vec3<f32>, frag_coord: vec4<f32>, footprint: f32) -> vec3<f32> {
    let time = view_bindings::globals.time;
    let slope = ripples(world_position.xz, time, footprint).xy * FRESH_RIPPLES;
    let surface = normalize(vec3<f32>(-slope.x, 1.0, -slope.y));
    let scatter = scene_colour(FRESH_SCATTER) * daylight(uncurved(world_position));
    // light from above leaves the water only within this cosine of straight up, 1 / 1.33 its sine
    let window = smoothstep(SNELL_WINDOW - 0.04, SNELL_WINDOW + 0.04, dot(-to_eye, surface));
    var above = scatter;
@if(VIEW_TRANSMISSION_TEXTURE) {
    let uv = frag_coord.xy / view_bindings::view.viewport.zw;
    let bent = clamp(uv + slope * REFRACTION * 3.0, vec2<f32>(0.001), vec2<f32>(0.999));
    above = textureSampleLevel(view_bindings::view_transmission_texture, view_bindings::view_transmission_sampler, bent, 0.0).rgb;
}
    return mix(scatter, above, mix(BELOW_CLEAR.x, BELOW_CLEAR.y, window));
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
    // where the surface would lie without the swell lifting it, which tells fresh water from the sea
    var flat_y = uncurved(world_position).y;
@if(VERTEX_COLORS) {
    let swell_depth = in.color.g * CHUNK_SIZE;
    let swell_time = view_bindings::globals.time;
    let lift = swell(world_position.xz, swell_time, dist, swell_size(in.color.r, swell_depth, world_position.xz, swell_time), deep_water(swell_depth)).height;
    flat_y -= lift * in.color.b;
}
    let fresh_surface = flat_y > f32(constants::WATER_LEVEL) + FRESH_ABOVE_SEA;
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
    // fresh water is calm, with no surf
    if dist < FOAM_FADE_END && !fresh_surface {
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
    // its colour's tuned for midday, so follows the light from there
    var scene = scene_colour(color.rgb) * daylight(uncurved(world_position));
    let to_eye = -view_to_world / dist;
    // blocks a pixel spans across the water here, worked out where every fragment does
    let footprint = max(length(dpdx(world_position.xz)), length(dpdy(world_position.xz)));
    // which side of the surface's plane the eye is on, as a slope rising away from it is seen from above though over the eye
    let outside = dot(view_position - world_position, normal) > 0.0;
    let above = outside && normal.y > 0.5;
@if(SOLID_WATER) {
    // nothing shows through, so it's all the water's own colour that isn't reflected
    alpha = 1.0;
}
@if(!SOLID_WATER) {
    // and see-through water closes up before LODs draw it opaque, or where they take over
    // the seabed would stop showing all at once, along the square edge of their nodes
    if see_through.y > see_through.x {
        alpha = mix(alpha, 1.0, smoothstep(see_through.x, see_through.y, dist));
    }
}
    // fresh water has its own look, below, so is spared this one's cost
    if above && !fresh_surface {
        let time = view_bindings::globals.time;
        let ripple = ripples(world_position.xz, time, footprint);
        var size = 1.0;
        var deep = 1.0;
@if(VERTEX_COLORS) {
        size = swell_size(in.color.r, in.color.g * CHUNK_SIZE, world_position.xz, time);
        deep = deep_water(in.color.g * CHUNK_SIZE);
}
        let waves = swell(world_position.xz, time, dist, size, deep);
        let slope = ripple.xy + waves.slope;
        let surface = normalize(vec3<f32>(-slope.x, 1.0, -slope.y));
        let variance = CALM_SLOPE_VARIANCE + ripple.z + waves.lost;
        // the clouds are flat, so they're reflected and cast shadows from where the water really is
        let rough = rough_reflection(uncurved(world_position), slope, sqrt(ripple.z + waves.lost), to_eye);
        let sky = rough.rgb;
        let reflected = rough.a;
        let sun = glint(surface, to_eye, variance, world_position, in.position.xy) * cloud_shadow(uncurved(world_position));
        // blended over what's behind: the reflection and glint stay as bright however clear the water
        let body = scene * alpha * (1.0 - reflected);
        let covered = alpha * (1.0 - reflected) + reflected;
        scene = (body + sky * reflected + sun) / max(covered, 1e-4);
        alpha = covered;
    }

    // fresh water is its own sight of what's under it, so is drawn over it, not blended, and keeps
    // its look at any distance, LODs included
    // a fall's side seen from out of the water is fresh water's outside too
    if outside && fresh_surface {
        // LOD water has no depth to go by and a coarse bed under it shows in squares, so is deep enough to hide it
        var depth = 10.0;
@if(VERTEX_COLORS) {
        depth = max(in.color.g * CHUNK_SIZE, 0.25);
}
        scene = fresh_water(world_position, normal, to_eye, in.position, depth, footprint, dist, scene_colour(color.rgb) * daylight(uncurved(world_position)));
        alpha = 1.0;
    } else if fresh_surface && normal.y > 0.5 {
        // seen from under it, clear, not tinted
        scene = fresh_water_below(world_position, to_eye, in.position, footprint);
        alpha = 1.0;
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
