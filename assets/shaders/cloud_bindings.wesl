// The cloud pattern every material that draws clouds or their shadows binds, see `clouds::CloudLayer`.

@group(constants::MATERIAL_BIND_GROUP) @binding(4) var cloud_pattern: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(5) var cloud_sampler: sampler;
// the middle of the layer's height, the pattern's width in blocks, and the wind
@group(constants::MATERIAL_BIND_GROUP) @binding(6) var<uniform> cloud_layer: vec4<f32>;
