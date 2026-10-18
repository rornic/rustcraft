//! Small maths shared by the effects, sounds and creatures.

/// Eases from 0 at `from` to 1 at `to`, smoothly at both ends, as the shaders' `smoothstep`.
pub fn smoothstep(from: f32, to: f32, x: f32) -> f32 {
    ease(((x - from) / (to - from)).clamp(0.0, 1.0))
}

/// Eases `t`, 0..1, smoothly in and out.
pub fn ease(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}
