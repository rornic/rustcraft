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
import super::lighting::{daylight, moon_radiance, sun_radiance};
import super::sky::{MOON_LIGHT, SUN_LIGHT};
import super::stars::reflected_celestial;
import super::cloud_shadow::cloud_shadow;
import super::clouds::lit_clouds;
import super::cloud_march::{BASE, LAND_TOP, OVERHEAD_REACH, TOP, overhead, sky_disc};
import super::cloud_cover::{cloud_reflection, cloud_reflection_sampler, disc_showing};
import super::curvature::{curved, uncurved};
import super::hash::hash2;

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> material_color: vec4<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var material_color_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var material_color_sampler: sampler;

@group(constants::MATERIAL_BIND_GROUP) @binding(3) var<uniform> material_fade: vec4<f32>;

// The open ocean's colour, which the sea's water darkens to between these depths in blocks.
const DEEP_OCEAN_COLOR: vec3<f32> = vec3<f32>(0.004, 0.03, 0.16);
const DEEP_OCEAN_START: f32 = 3.0;
const DEEP_OCEAN_END: f32 = 14.0;
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

// A reflection pointing down would be of the water itself, so it's kept to the sky.
fn skyward(direction: vec3<f32>) -> vec3<f32> {
    return normalize(vec3<f32>(direction.x, max(direction.y, 0.02), direction.z));
}

// The sky and stars reflected along `direction`.
fn reflected_sky(direction: vec3<f32>) -> vec3<f32> {
    let up = skyward(direction);
    let night = reflected_celestial(up);
    return sky_colour(up) * (1.0 - night.a) + night.rgb;
}

// `sky` with clouds over it, from the sky map rather than the screen's cloud target, which left a seam in the water where the reflected way left the screen.
fn under_clouds(sky: vec3<f32>, origin: vec3<f32>, direction: vec3<f32>, keep: f32) -> vec3<f32> {
    if keep <= 0.0 {
        return sky;
    }
    let up = skyward(direction);
    // the map is marched from over the eye, so the way there to where this ray meets the clouds is looked up instead
    let eye = view_bindings::view.world_position;
    let meets = origin + up * max(0.5 * (BASE + TOP) - origin.y, 0.0) / up.y;
    let seen = normalize(meets - vec3<f32>(eye.x, min(eye.y, LAND_TOP), eye.z));
    var marched = textureSampleLevel(cloud_reflection, cloud_reflection_sampler, sky_disc(seen), 0.0);
    // but far off that way slants through more of the clouds than this ray does, smearing them, so there the clouds straight over are read
    let slant = smoothstep(1e-4, 2e-3, 1.0 - dot(up, seen));
    if slant > 0.0 {
        let off = abs(meets.xz - eye.xz) / OVERHEAD_REACH;
        let over = textureSampleLevel(cloud_reflection, cloud_reflection_sampler, overhead(meets.xz, eye.xz), 0.0) * (1.0 - smoothstep(0.8, 1.0, max(off.x, off.y)));
        marched = mix(marched, over, slant);
    }
    let cloud = lit_clouds(marched, origin, up, 1.0) * keep;
    return sky * (1.0 - cloud.a) + cloud.rgb;
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
        moonlit = moon_radiance() * highlight(normal, to_eye, variance, view_bindings::lights.directional_lights[MOON_LIGHT].direction_to_light) * disc_showing(true);
    }
    let shine = highlight(normal, to_eye, variance, (*sun).direction_to_light);
    // most water is nowhere near the glint, so needn't look up the land's shadow
    if shine < 1e-5 {
        return moonlit;
    }
    // a glint is the disc mirrored, so goes where a cloud hides it, as well as in the clouds' shadow
    let sunlit = sun_radiance(uncurved(world_position)) * shine * disc_showing(false);
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

// Water is clear: it takes a little of each colour out of the light that
// reaches the bed and back, red first and fastest, per block of depth it passes through, and adds a
// faint scatter of its own, so a shallow bed shows true and a deep pool goes slowly green and blue.
const FRESH_ABSORB: vec3<f32> = vec3<f32>(0.22, 0.07, 0.04);
const FRESH_SCATTER: vec3<f32> = vec3<f32>(0.03, 0.10, 0.14);
// The most of what's seen that the surface reflects, however slant, for a lake and for the sea, which
// is rougher, and how much of the sun's glint the sea keeps.
const LAKE_REFLECT_MAX: f32 = 0.5;
const SEA_REFLECT_MAX: f32 = 0.18;
const SEA_GLINT: f32 = 0.25;
// How much of the clouds' reflection the sea keeps: none, as a rough sea smears them out, and drawn they're a mirror's copy of the sky.
const SEA_CLOUDS: f32 = 0.0;
// The water must lie this far over the sea for its surface to be fresh, which has no surf and none of the ocean's depth tint.
const FRESH_ABOVE_SEA: f32 = 0.6;
// How much of the water's own colour shows over the bed at any depth.
const FRESH_TINT: f32 = 0.1;
// How far, in screen heights, ripples bend what's seen through the surface, per block of depth.
const REFRACTION: f32 = 0.035;
// How strong the ripples are.
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

// Water seen from above: what's under the surface, through ripples that bend it and a depth of clear
// water that tints it, and the sky, the sun and the world about it reflected off the top. `sea` is 1 for
// the sea and 0 for fresh water, `ocean` how far the sea's depth has darkened it to open ocean blue,
// and `waves` the swell on the surface.
fn water(world_position: vec3<f32>, normal: vec3<f32>, to_eye: vec3<f32>, frag_coord: vec4<f32>, depth: f32, footprint: f32, dist: f32, tint: vec3<f32>, sea: f32, ocean: f32, waves: Swell) -> vec3<f32> {
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
    let slope = ripple.xy * FRESH_RIPPLES + waves.slope;
    let surface = normalize(vec3<f32>(-slope.x, 1.0, -slope.y));
    let variance = CALM_SLOPE_VARIANCE + ripple.z * FRESH_RIPPLES * FRESH_RIPPLES + waves.lost;

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
    var body = mix(behind * transmitted + scatter * (1.0 - transmitted), tint, FRESH_TINT);
    // the sea goes on darkening with depth, past where any bed shows
    body = mix(body, scene_colour(DEEP_OCEAN_COLOR) * daylight(uncurved(world_position)), ocean);

    let reflect_dir = reflect(-to_eye, surface);
    let share = min(fresnel(dot(surface, to_eye)) + 0.03, mix(LAKE_REFLECT_MAX, SEA_REFLECT_MAX, sea));
    var world = vec4<f32>(0.0);
    if dist < REFLECTION_NEAR && share > 0.06 {
        world = screen_reflection(world_position, reflect_dir, frag_coord);
    }
    // the sky and clouds only where the land's reflection leaves them showing
    var reflection = world.rgb;
    if world.a < 1.0 {
        let sky = under_clouds(reflected_sky(reflect_dir), uncurved(world_position), reflect_dir, mix(1.0, SEA_CLOUDS, sea));
        reflection = mix(sky, world.rgb, world.a);
    }
    let sun = glint(surface, to_eye, variance, world_position, frag_coord.xy) * cloud_shadow(uncurved(world_position)) * mix(1.0, SEA_GLINT, sea);
    return body * (1.0 - share) + reflection * share + sun;
}

// Water's surface seen from under it: what's above, bent by the ripples, clearest through Snell's window straight
// up and tinted more past it, where real water would only mirror what's under it.
fn water_below(world_position: vec3<f32>, to_eye: vec3<f32>, frag_coord: vec4<f32>, footprint: f32) -> vec3<f32> {
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
    let time = view_bindings::globals.time;
    // LOD water has no depth to go by and a coarse bed under it shows in squares, so is deep enough to hide it
    var depth = 10.0;
    var ocean = 1.0;
    var waves = Swell(0.0, vec2<f32>(0.0), 0.0);
    // where the surface would lie without the swell lifting it, which tells fresh water from the sea
    var flat_y = uncurved(world_position).y;
@if(VERTEX_COLORS) {
    depth = max(in.color.g * CHUNK_SIZE, 0.25);
    ocean = smoothstep(DEEP_OCEAN_START, DEEP_OCEAN_END, depth);
    waves = swell(world_position.xz, time, dist, swell_size(in.color.r, depth, world_position.xz, time), deep_water(depth));
    flat_y -= waves.height * in.color.b;
}
    let fresh_surface = flat_y > f32(constants::WATER_LEVEL) + FRESH_ABOVE_SEA;
    let sea = f32(!fresh_surface);
    ocean *= sea;
@if(!VERTEX_COLORS) {
    // LOD water has no depth or shore to calm the swell by, and is far enough out to be deep, so is the sea at full size: left flat it's a mirror
    if !fresh_surface {
        waves = swell(world_position.xz, time, dist, 1.0, 1.0);
    }
}

    // its colour's tuned for midday, so follows the light from there
    let tint = scene_colour((material_color * textureSample(material_color_texture, material_color_sampler, in.uv)).rgb) * daylight(uncurved(world_position));

    // Foam's noise sampling (up to 6 hash lookups per pixel between the 3 octaves
    // and the frame crossfade) is the most expensive part of this shader, for
    // detail that's imperceptible at distance anyway - branching around it entirely
    // beyond FOAM_FADE_END is a real cost saving, not just a visual fade.
    var foam = 0.0;
@if(VERTEX_COLORS) {
    // fresh water is calm, with no surf
    if dist < FOAM_FADE_END && !fresh_surface {
        // Phase is driven by distance-to-shore (baked per vertex - see
        // vertex_shore_distance, anchored at the shared grid corner so it can't
        // disagree between neighbouring blocks) rather than world xz, so the
        // travelling envelope always advances toward the coast along its local
        // normal instead of sliding sideways along it like a fixed-direction wave.
        let shore_distance = in.color.a;
        let phase = shore_distance * FOAM_PHASE_SCALE + time * FOAM_SPEED;
        let lap = smoothstep(1.0 - FOAM_BAND, 1.0, sin(phase)) * in.color.r;
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
}

    let to_eye = -view_to_world / dist;
    // blocks a pixel spans across the water here, worked out where every fragment does
    let footprint = max(length(dpdx(world_position.xz)), length(dpdy(world_position.xz)));
    // which side of the surface's plane the eye is on, as a slope rising away from it is seen from above though over the eye
    let outside = dot(view_position - world_position, normal) > 0.0;

    // water is its own sight of what's under it, so is drawn over it, not blended, and keeps
    // its look at any distance, LODs included
    var scene = tint;
    if outside {
        scene = water(world_position, normal, to_eye, in.position, depth, footprint, dist, tint, sea, ocean, waves);
        scene = mix(scene, scene_colour(FOAM_COLOR) * daylight(uncurved(world_position)), foam);
    } else if normal.y > 0.5 {
        // seen from under it, clear, not tinted
        scene = water_below(world_position, to_eye, in.position, footprint);
    }

    // so distant water fades into the sky instead of popping in and out
    let shown = fogged(vec4<f32>(scene, 1.0), world_position, in.position.xy);

    // after the texture samples above, which need uniform control flow
    let threshold = dither_threshold(in.position.xy);
    if threshold < material_fade.x || threshold >= material_fade.y {
        discard;
    }

    var output: FragmentOutput;
    output.color = shown;
    return output;
}
