use bevy::{
    color::{Color, LinearRgba},
    core_pipeline::tonemapping::Tonemapping,
    math::{Mat3, Vec3},
    render::view::{ColorGrading, ColorGradingGlobal},
};

/// Rolls sunlit highlights off instead of clipping them, while keeping shade's colour:
/// ACES turned dark greens neon, and AgX and Tony McMapface wash out bright ones.
pub const TONEMAPPING: Tonemapping = Tonemapping::SomewhatBoringDisplayTransform;

/// Stops of exposure before tonemapping, so a mid grey stays about mid grey once the
/// tonemapping curve has bent it. `water.wgsl` has a copy.
pub const EXPOSURE: f32 = 0.5;

/// The camera's colour grading out of water.
pub fn dry_color_grading() -> ColorGrading {
    ColorGrading {
        global: ColorGradingGlobal {
            exposure: EXPOSURE,
            ..Default::default()
        },
        ..Default::default()
    }
}

const LUMA: Vec3 = Vec3::new(0.2126, 0.7152, 0.0722);
// rows: luma, then the two chroma channels
const RGB_TO_YCBCR: Mat3 = Mat3::from_cols(
    Vec3::new(0.2126, -0.1146, 0.5),
    Vec3::new(0.7152, -0.3854, -0.4542),
    Vec3::new(0.0722, 0.5, -0.0458),
);
// Enough for `scene_colour` to land within a hundredth of the colours asked for.
const INVERSE_STEPS: usize = 32;

fn tonemap_curve(v: f32) -> f32 {
    1.0 - (-v).exp()
}

// Bevy's somewhat boring display transform, on an exposed colour.
fn somewhat_boring_display_transform(colour: Vec3) -> Vec3 {
    let ycbcr = RGB_TO_YCBCR * colour;
    let bt = tonemap_curve(Vec3::new(0.0, ycbcr.y, ycbcr.z).length() * 2.4);
    let desat = ((bt - 0.7) * 0.8).max(0.0).powi(2);
    let desat_colour = colour.lerp(Vec3::splat(ycbcr.x), desat);
    let luma_scale = tonemap_curve(ycbcr.x) / colour.dot(LUMA).max(1e-5);
    let tm0 = colour * luma_scale.max(0.0);
    let tm1 = desat_colour.to_array().map(tonemap_curve);
    tm0.lerp(Vec3::from_array(tm1), bt * bt) * 0.97
}

fn exposure_scale() -> f32 {
    2f32.powf(EXPOSURE)
}

fn rgb(colour: LinearRgba) -> Vec3 {
    Vec3::new(colour.red, colour.green, colour.blue)
}

/// What a scene colour looks like on screen once exposed and tonemapped.
pub fn displayed(scene: LinearRgba) -> LinearRgba {
    let out = somewhat_boring_display_transform(rgb(scene).max(Vec3::ZERO) * exposure_scale());
    LinearRgba::new(out.x, out.y, out.z, scene.alpha)
}

/// The scene colour to draw for `display` to show on screen after tonemapping: for things
/// like the sky and fog that were picked to look a certain way rather than be lit.
pub fn scene_colour(display: Color) -> LinearRgba {
    let display = display.to_linear();
    let target = rgb(display);
    // tonemapping is near enough the identity up close that nudging towards the target
    // by however far off it still is converges
    let mut scene = target;
    for _ in 0..INVERSE_STEPS {
        let shown = rgb(displayed(LinearRgba::rgb(scene.x, scene.y, scene.z)));
        scene = (scene + target - shown).max(Vec3::ZERO);
    }
    LinearRgba::new(scene.x, scene.y, scene.z, display.alpha)
}

#[cfg(test)]
mod tests {
    use bevy::color::{Color, ColorToComponents, LinearRgba};

    use super::{displayed, scene_colour, EXPOSURE};

    fn assert_close(expected: LinearRgba, actual: LinearRgba) {
        for (e, a) in expected.to_f32_array().iter().zip(actual.to_f32_array()) {
            assert!(
                (e - a).abs() < 0.01,
                "expected {expected:?}, got {actual:?}"
            );
        }
    }

    #[test]
    fn test_scene_colours_display_as_asked() {
        for colour in [
            Color::srgb_u8(165, 200, 235),
            Color::srgb_u8(60, 110, 200),
            Color::srgb_u8(20, 90, 130),
            Color::srgb_u8(0, 217, 217),
            Color::srgb(0.5, 0.5, 0.5),
        ] {
            assert_close(colour.to_linear(), displayed(scene_colour(colour)));
        }
    }

    #[test]
    fn test_mid_grey_stays_about_mid_grey() {
        let grey = displayed(LinearRgba::rgb(0.18, 0.18, 0.18));
        assert!((0.16..0.25).contains(&grey.green), "{grey:?}");
    }

    #[test]
    fn test_bright_light_rolls_off_instead_of_clipping() {
        let bright = displayed(LinearRgba::rgb(2.0, 2.0, 2.0));
        let brighter = displayed(LinearRgba::rgb(4.0, 4.0, 4.0));
        assert!(bright.green < brighter.green && brighter.green < 1.0);
    }

    #[test]
    fn test_water_shader_has_the_same_exposure() {
        let shader = std::fs::read_to_string("assets/shaders/water.wgsl").unwrap();
        let line = shader
            .lines()
            .find(|line| line.starts_with("const EXPOSURE_SCALE"))
            .unwrap();
        let value = line.split(['=', ';']).nth(1).unwrap().trim();
        let scale: f32 = value.parse().unwrap();
        assert!((scale - 2f32.powf(EXPOSURE)).abs() < 1e-4, "{scale}");
    }
}
