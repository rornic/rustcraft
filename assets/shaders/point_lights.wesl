// Light from the point lights near a face, as Bevy clusters them, for the world shader's own
// lighting: Bevy's would only light its own materials. Fireflies glow on the grass by it.

import bevy_pbr::render::{clustered_forward as clustering, mesh_view_bindings as view_bindings};

// The point lights' light on a face at `world_position` turned to `normal`, in scene colour.
// `frag_coord` finds the face's cluster, so only the lights that reach it are looked at.
fn point_light_glow(world_position: vec3<f32>, normal: vec3<f32>, frag_coord: vec2<f32>) -> vec3<f32> {
    let view_from_world = view_bindings::view.view_from_world;
    let view_z = dot(vec4<f32>(view_from_world[0].z, view_from_world[1].z, view_from_world[2].z, view_from_world[3].z), vec4<f32>(world_position, 1.0));
    let cluster = clustering::view_fragment_cluster_index(frag_coord, view_z, false);
    let ranges = clustering::unpack_clusterable_object_index_ranges(cluster);
    var glow = vec3<f32>(0.0);
    for (var i: u32 = ranges.first_point_light_index_offset; i < ranges.first_spot_light_index_offset; i = i + 1u) {
        let light = &view_bindings::clustered_lights.data[clustering::get_clusterable_object_id(i)];
        let to_light = (*light).position_radius.xyz - world_position;
        let distance_squared = dot(to_light, to_light);
        // Bevy's own falloff: the inverse square, eased to nothing at the light's range
        let reach = distance_squared * (*light).color_inverse_square_range.w;
        let fade = saturate(1.0 - reach * reach);
        let attenuation = fade * fade / max(distance_squared, 0.0001);
        let facing = max(dot(normal, to_light * inverseSqrt(max(distance_squared, 0.0001))), 0.0);
        glow += (*light).color_inverse_square_range.rgb * attenuation * facing;
    }
    return glow * view_bindings::view.exposure;
}
