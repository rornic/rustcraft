// The clouds marched for water to reflect, and how much of the sun's and moon's discs they leave showing, see `CloudLayer::reflection`.

@group(constants::MATERIAL_BIND_GROUP) @binding(9) var cloud_reflection: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(10) var cloud_reflection_sampler: sampler;

// A disc's light passes the clouds as their transmittance to this power: the sun's and moon's discs are thousands of times brighter than the sky a cloud thins, so would glare out of any but the thickest.
const DISC_HIDING: f32 = 4.0;

// How much of the sun's disc, or the moon's, shows past the clouds, 0..1, from the march toward each in the map's first two texels, see `reflection` in `cloud_target.wesl`.
fn disc_showing(moon: bool) -> f32 {
    let cover = textureLoad(cloud_reflection, vec2<i32>(select(0, 1, moon), 0), 0).a;
    return pow(1.0 - cover, DISC_HIDING);
}
