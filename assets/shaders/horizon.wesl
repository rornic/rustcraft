// The haze along the horizon, which far land fades into: the sky just over the land's edge,
// which dips below level as the land curves away, see `curvature.rs`. The sky's own horizon
// dips with it, see `sky::match_sky_horizon`.

import bevy_pbr::render::mesh_view_bindings as view_bindings;
import super::curvature::curvature;
import super::sky::sky_colour;

// Slope above the land's edge where the sky is read for the haze, since far land meets the
// sky just under it.
const HORIZON_RISE: f32 = 0.02;

// How far below level the camera sees the land's edge, as a slope: the sea's horizon, or
// nearer, where the fog ends, if the land's drawn short of it. The same as `Curve::edge_dip`.
// `WATER_LEVEL` comes from the material's `specialize`.
fn horizon_dip() -> f32 {
    let height = max(curvature.now.eye.y - f32(constants::WATER_LEVEL), 0.0);
    let bend = curvature.now.per_square_block;
    var edge = max(view_bindings::fog.be.y, 1.0);
    if bend > 0.0 {
        edge = min(edge, sqrt(height / bend));
    }
    return height / edge + bend * edge;
}

// The haze toward `across`, a direction along the ground.
fn horizon_haze(across: vec2<f32>) -> vec3<f32> {
    let level = across / max(length(across), 1e-6);
    return sky_colour(normalize(vec3<f32>(level.x, HORIZON_RISE - horizon_dip(), level.y)));
}
