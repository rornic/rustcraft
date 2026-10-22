//! Small maths shared by the effects, sounds and creatures.

/// Eases from 0 at `from` to 1 at `to`, smoothly at both ends, as the shaders' `smoothstep`.
pub fn smoothstep(from: f32, to: f32, x: f32) -> f32 {
    ease(((x - from) / (to - from)).clamp(0.0, 1.0))
}

/// `smoothstep` in f64, for the world map and terrain, which must give the same answer on every
/// machine.
pub fn smoothstep_f64(edge0: f64, edge1: f64, x: f64) -> f64 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Eases `t`, 0..1, smoothly in and out.
pub fn ease(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}
