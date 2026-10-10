use bevy::{
    asset::{Assets, RenderAssetUsages},
    color::{Color, ColorToPacked, LinearRgba, Mix},
    core_pipeline::Skybox,
    image::Image,
    math::{Quat, Vec3},
    render::{
        camera::Exposure,
        render_resource::{
            Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension,
        },
    },
};

/// The sky at the horizon, also used for fog so distant terrain fades into it.
pub const HORIZON_COLOR: Color = Color::srgb(165.0 / 255.0, 200.0 / 255.0, 235.0 / 255.0);
/// The sky straight overhead.
pub const ZENITH_COLOR: Color = Color::srgb(60.0 / 255.0, 110.0 / 255.0, 200.0 / 255.0);

// Texels per cubemap face side. The gradient is smooth, so this can be small.
const FACE_SIZE: u32 = 128;

/// A skybox shading from `HORIZON_COLOR` at the horizon to `ZENITH_COLOR` overhead.
pub fn gradient_skybox(images: &mut Assets<Image>) -> Skybox {
    let mut data = Vec::with_capacity((FACE_SIZE * FACE_SIZE * 6 * 4) as usize);
    for face in 0..6 {
        for y in 0..FACE_SIZE {
            for x in 0..FACE_SIZE {
                let colour = sky_colour(face_direction(face, x, y, FACE_SIZE));
                data.extend_from_slice(&Color::from(colour).to_srgba().to_u8_array());
            }
        }
    }

    let mut image = Image::new(
        Extent3d {
            width: FACE_SIZE,
            height: FACE_SIZE * 6,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.reinterpret_stacked_2d_as_array(6);
    image.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::Cube),
        ..Default::default()
    });

    Skybox {
        image: images.add(image),
        // cancels out camera exposure, which the terrain shaders don't use
        brightness: 1.0 / Exposure::default().exposure(),
        rotation: Quat::IDENTITY,
    }
}

// The square root keeps the paler band close to the horizon, like a real sky.
fn sky_colour(direction: Vec3) -> LinearRgba {
    let height = direction.normalize().y.clamp(0.0, 1.0).sqrt();
    LinearRgba::from(HORIZON_COLOR).mix(&LinearRgba::from(ZENITH_COLOR), height)
}

// The direction through texel (`x`, `y`) of a cubemap face, in the order +X, -X, +Y, -Y, +Z, -Z.
fn face_direction(face: u32, x: u32, y: u32, size: u32) -> Vec3 {
    let s = 2.0 * (x as f32 + 0.5) / size as f32 - 1.0;
    let t = 2.0 * (y as f32 + 0.5) / size as f32 - 1.0;
    match face {
        0 => Vec3::new(1.0, -t, -s),
        1 => Vec3::new(-1.0, -t, s),
        2 => Vec3::new(s, 1.0, t),
        3 => Vec3::new(s, -1.0, -t),
        4 => Vec3::new(s, -t, 1.0),
        _ => Vec3::new(-s, -t, -1.0),
    }
}

#[cfg(test)]
mod tests {
    use bevy::color::LinearRgba;
    use bevy::math::Vec3;

    use super::{face_direction, sky_colour, HORIZON_COLOR, ZENITH_COLOR};

    #[test]
    fn test_face_directions_point_out_of_their_faces() {
        let axes = [Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y, Vec3::Z, -Vec3::Z];
        for (face, axis) in axes.iter().enumerate() {
            let centre = face_direction(face as u32, 63, 63, 128).normalize();
            assert!(centre.dot(*axis) > 0.99, "face {face}");
        }
    }

    #[test]
    fn test_side_faces_are_lighter_at_the_bottom_than_the_top() {
        // image rows run top to bottom, so the top row of a side face looks up
        let top = face_direction(0, 64, 0, 128);
        let bottom = face_direction(0, 64, 127, 128);
        assert!(top.y > 0.0 && bottom.y < 0.0);
    }

    #[test]
    fn test_sky_colour_runs_from_horizon_to_zenith() {
        let horizon = LinearRgba::from(HORIZON_COLOR);
        let zenith = LinearRgba::from(ZENITH_COLOR);
        assert_eq!(horizon, sky_colour(Vec3::X));
        assert_eq!(horizon, sky_colour(-Vec3::Y));
        assert_eq!(zenith, sky_colour(Vec3::Y));
        let halfway_up = sky_colour(Vec3::new(1.0, 1.0, 0.0));
        assert!(halfway_up.red < horizon.red && halfway_up.red > zenith.red);
    }
}
