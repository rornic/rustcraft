#import bevy_pbr::{forward_io::VertexOutput, mesh_view_bindings as view_bindings}

@group(2) @binding(0) var<uniform> material_color: vec4<f32>;
@group(2) @binding(1) var material_color_texture: texture_2d<f32>;
@group(2) @binding(2) var material_color_sampler: sampler;

struct FragmentOutput {
  @location(0) color: vec4<f32>
}

fn getFog(start: f32, end: f32, dist: f32) -> f32 {
   if (dist>=end) { return 0.0; }
   if (dist<=start) { return 1.0; }

   return (end - dist) / (end - start);
}

@fragment
fn fragment(
    in: VertexOutput,
) ->  FragmentOutput {

    let world_position = in.world_position.xyz;
    let view_position = view_bindings::view.world_position.xyz;
    let view_to_world = world_position - view_position;

    let dist = length(view_to_world);
    let fog = getFog(496.0, 512.0, dist);
    if fog < 0.1 {
      discard;
    }

    let brightness = dot(normalize(in.world_normal), normalize(vec3(-0.2, 0.7, 0.2)));

    let color_lit = material_color * textureSample(material_color_texture, material_color_sampler, in.uv);

    let dark = color_lit * 0.7;
    let color = mix(dark, color_lit, brightness);

    var output: FragmentOutput;
    output.color = color;
    return output;
}
