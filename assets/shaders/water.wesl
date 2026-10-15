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
import super::lighting::{daylight, direct_sun, open_sky, sun_radiance};
import super::cloud_shadow::{cloud_shadow, cloud_pattern, cloud_sampler, cloud_layer};
import super::curvature::{curved, uncurved};

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

// 0..1 for the whole-number cell `p` is in. Integer mixing, since the usual fract(sin)
// hash loses its precision at world coordinates in the thousands and turns into stripes.
fn hash2(p: vec2<f32>) -> f32 {
    let cell = vec2<i32>(floor(p));
    var h = (bitcast<u32>(cell.x) * 0x8da6b343u) ^ (bitcast<u32>(cell.y) * 0xd8163841u);
    h = (h ^ (h >> 15u)) * 0x2c1b3c6du;
    h = (h ^ (h >> 12u)) * 0x297a2d39u;
    h = h ^ (h >> 15u);
    return f32(h) / 4294967295.0;
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
    let lit = direct_sun(facing, vec3<f32>(hit.x, layer, hit.y)) * CLOUD_SUN_SCATTER + open_sky(facing) * CLOUD_SKY_SCATTER;
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
    let radiance = sun_radiance(uncurved(world_position));
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
    // its colour's tuned for midday, so follows the light from there
    var scene = scene_colour(color.rgb) * daylight(uncurved(world_position));
    let to_eye = -view_to_world / dist;
    // blocks a pixel spans across the water here, worked out where every fragment does
    let footprint = max(length(dpdx(world_position.xz)), length(dpdy(world_position.xz)));
    let above = view_position.y > world_position.y && normal.y > 0.5;
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
    if above {
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
