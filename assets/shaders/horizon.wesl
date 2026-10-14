// The haze along the horizon, which far land fades into, and the glow it gathers where the
// land curves away below it from high up, as the air does round the real Earth seen from
// a mountain or a plane, see `curvature.rs`.

import super::curvature::curvature;
import super::sky::{clear_sky_colour, sky_colour};

// Slope above level where the sky is read for the haze, since far land meets the sky just under it.
const HORIZON_RISE: f32 = 0.02;
// How much brighter than the haze the glow is at the land's edge.
const GLOW_GAIN: f32 = 0.7;
// How far the glow spreads from the land's edge, as a share of how far below level the edge is.
const GLOW_WIDTH: f32 = 0.4;
// The glow comes in fully once the land's edge is this far below level, as a slope.
const GLOW_FULL_DIP: f32 = 0.08;
// The lowered sky is averaged over this much slope either way, in this many samples: the
// environment map it's read from is coarse enough to show steps once stretched.
const SKY_SMOOTHING: f32 = 0.06;
const SKY_SAMPLES: i32 = 8;
// How far above level the sky is drawn down to meet the land's edge, as a multiple of how
// far below level the edge is.
const SKY_BEND_TOP: f32 = 2.0;

// How far below level the camera sees the sea's horizon, as a slope: where a line from the
// eye grazes the bent sea. 0 where the land isn't bent. `WATER_LEVEL` comes from the
// material's `specialize`.
fn horizon_dip() -> f32 {
    let height = max(curvature.now.eye.y - f32(constants::WATER_LEVEL), 0.0);
    return 2.0 * sqrt(curvature.now.per_square_block * height);
}

// How strongly the glow shows, 0..1: none while the land's flat, coming in as it bends.
fn glow_strength() -> f32 {
    return smoothstep(0.0, GLOW_FULL_DIP, horizon_dip());
}

// Toward `across`, a direction along the ground, at `slope` above level.
fn direction_at(across: vec2<f32>, slope: f32) -> vec3<f32> {
    let level = across / max(length(across), 1e-6);
    return normalize(vec3<f32>(level.x, slope, level.y));
}

// The haze toward `across`, which far land fades into.
fn horizon_haze(across: vec2<f32>) -> vec3<f32> {
    return sky_colour(direction_at(across, HORIZON_RISE));
}

// Up to this slope above level the sky is drawn down to meet the land's edge.
fn sky_bend_top() -> f32 {
    return SKY_BEND_TOP * horizon_dip();
}

// The sky toward `across` at `slope` above level as it's seen over the bent land: its
// horizon, the haze, brought down to the land's edge, easing back to where it really is
// by `sky_bend_top`. Without it the sky's horizon would hang level, high above the edge.
fn sky_over_curve(across: vec2<f32>, slope: f32) -> vec3<f32> {
    let dip = horizon_dip();
    let lowered = 1.0 - smoothstep(-dip, sky_bend_top(), slope);
    let shown = slope + (dip + HORIZON_RISE) * lowered;
    var sum = vec3<f32>(0.0);
    for (var i = 0; i < SKY_SAMPLES; i++) {
        let offset = SKY_SMOOTHING * ((f32(i) + 0.5) / f32(SKY_SAMPLES) * 2.0 - 1.0);
        sum += clear_sky_colour(direction_at(across, shown + offset));
    }
    return sum / f32(SKY_SAMPLES);
}

// The glow seen toward `across` at `slope` above level: brightest along the land's edge and
// fading away either side of it.
fn horizon_glow(across: vec2<f32>, slope: f32) -> vec3<f32> {
    let dip = horizon_dip();
    if dip <= 0.0 {
        return vec3<f32>(0.0);
    }
    let from_edge = abs(slope + dip) / (dip * GLOW_WIDTH);
    return horizon_haze(across) * GLOW_GAIN * glow_strength() * exp(-from_edge);
}
