// Clouds: lit by the same sun and sky as the land, and fogged with it, see `clouds.rs`.

#import bevy_pbr::forward_io::VertexOutput
#import "shaders/fog.wgsl"::fogged
#import "shaders/lighting.wgsl"::{direct_sun, open_sky}

@group(2) @binding(0) var<uniform> cloud_color: vec4<f32>;

// Clouds scatter the light that falls on them, so their shaded sides are far brighter
// than solid ground's would be and the sunlit ones a little softer.
const SUN_SCATTER: f32 = 0.8;
const SKY_SCATTER: f32 = 1.4;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let normal = normalize(in.world_normal);
    let light = direct_sun(normal) * SUN_SCATTER + open_sky(normal) * SKY_SCATTER;
    return fogged(vec4<f32>(cloud_color.rgb * light, 1.0), in.world_position.xyz);
}
