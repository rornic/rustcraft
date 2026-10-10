#import bevy_pbr::{forward_io::VertexOutput, mesh_view_bindings as view_bindings}

@group(2) @binding(0) var<uniform> horizon: vec4<f32>;
@group(2) @binding(1) var<uniform> zenith: vec4<f32>;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let direction = normalize(in.world_position.xyz - view_bindings::view.world_position.xyz);
    // the square root keeps the paler band close to the horizon, like a real sky
    let height = sqrt(clamp(direction.y, 0.0, 1.0));
    return vec4<f32>(mix(horizon.rgb, zenith.rgb, height), 1.0);
}
