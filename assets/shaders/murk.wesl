// The murk that hides the sky underwater, see `player::MurkMaterial`: the fog's colour, or
// nothing at all above water.

import bevy_pbr::render::forward_io::VertexOutput;

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> murk: vec4<f32>;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    if murk.a < 0.5 {
        discard;
    }
    return vec4<f32>(murk.rgb, 1.0);
}
