// Slow patchy variation in coarse LOD terrain's colour, so wide areas of one block don't
// read as flat. It's tied to the world and wider than a pixel at any distance, so it can't shimmer.

// Patch width in blocks.
const PATCH: f32 = 14.0;
// How far a patch pushes the colour either way, as a fraction of it.
const PATCH_STRENGTH: f32 = 0.1;

fn hash21(p: vec2<f32>) -> f32 {
    var q = fract(p * vec2<f32>(123.34, 456.21));
    q += dot(q, q + 45.32);
    return fract(q.x * q.y);
}

// Smooth value noise, 0..1.
fn patch_noise(xz: vec2<f32>) -> f32 {
    let p = xz / PATCH;
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let top = mix(hash21(i), hash21(i + vec2<f32>(1.0, 0.0)), u.x);
    let bottom = mix(hash21(i + vec2<f32>(0.0, 1.0)), hash21(i + vec2<f32>(1.0, 1.0)), u.x);
    return mix(top, bottom, u.y);
}

// What to scale a coarse cell's colour by at `world_position`.
fn patch_tint(world_position: vec3<f32>) -> f32 {
    return 1.0 + PATCH_STRENGTH * (patch_noise(world_position.xz) * 2.0 - 1.0);
}
