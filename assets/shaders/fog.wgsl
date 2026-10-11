// Fog for ChunkMaterial and WaterMaterial, which replace Bevy's PBR fragment stage and so
// must fog themselves.

#import bevy_pbr::{mesh_view_bindings as view_bindings, pbr_functions::apply_fog}

// `color` at `world_position` behind the camera's fog. Above water it thickens only with
// distance across the land, not up or down, so it hides the far edge of the world without
// greying out the ground below when flying high. Underwater, the murk closes in every way.
// `WATER_LEVEL` comes from the material's `specialize`.
fn fogged(color: vec4<f32>, world_position: vec3<f32>) -> vec4<f32> {
    let view_position = view_bindings::view.world_position.xyz;
    let underwater = view_position.y < f32(#{WATER_LEVEL});
    let level = vec3<f32>(world_position.x, view_position.y, world_position.z);
    return apply_fog(view_bindings::fog, color, select(level, world_position, underwater), view_position);
}
