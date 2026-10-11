use bevy::color::{ColorToComponents, LinearRgba, Srgba};
use bevy::math::Vec3;

// How hot and how wet a column is, each 0..1. See `NoiseGenerator::climate`.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct Climate {
    pub temperature: f32,
    pub humidity: f32,
}

impl Default for Climate {
    fn default() -> Self {
        Self {
            temperature: 0.5,
            humidity: 0.5,
        }
    }
}

// Grass at the four corners of the climate square, in sRGB: hot and dry is parched
// yellow, hot and wet lush green, and the cold side duller versions of both.
const GRASS_HOT_DRY: [f32; 3] = [200.0, 170.0, 60.0];
const GRASS_HOT_WET: [f32; 3] = [40.0, 130.0, 12.0];
const GRASS_COLD_DRY: [f32; 3] = [140.0, 140.0, 60.0];
const GRASS_COLD_WET: [f32; 3] = [30.0, 96.0, 24.0];

// The mean linear value of the greyscale grass tile in blocks.png, which vertex colours tint.
const GRASS_TEXTURE_MEAN: f32 = 0.5;

// Linear colour of grass in this climate.
pub fn grass_colour(climate: Climate) -> Vec3 {
    let srgb = |c: [f32; 3]| Vec3::from(c) / 255.0;
    let (t, h) = (climate.temperature.clamp(0.0, 1.0), climate.humidity.clamp(0.0, 1.0));
    let dry = srgb(GRASS_COLD_DRY).lerp(srgb(GRASS_HOT_DRY), t);
    let wet = srgb(GRASS_COLD_WET).lerp(srgb(GRASS_HOT_WET), t);
    let colour = dry.lerp(wet, h);
    LinearRgba::from(Srgba::rgb(colour.x, colour.y, colour.z)).to_vec3()
}

// What to multiply the grass texture by to draw `grass_colour`.
pub fn grass_tint(climate: Climate) -> Vec3 {
    grass_colour(climate) / GRASS_TEXTURE_MEAN
}

#[cfg(test)]
mod tests {
    use super::*;

    fn climate(temperature: f32, humidity: f32) -> Climate {
        Climate { temperature, humidity }
    }

    // How much yellower than green, as red over green.
    fn yellowness(colour: Vec3) -> f32 {
        colour.x / colour.y
    }

    #[test]
    fn test_hot_dry_grass_is_yellower_than_wet() {
        let dry = grass_colour(climate(1.0, 0.0));
        let wet = grass_colour(climate(1.0, 1.0));
        assert!(yellowness(dry) > 0.8, "dry grass should be yellow, was {dry}");
        assert!(yellowness(wet) < 0.2, "wet grass should be green, was {wet}");
    }

    #[test]
    fn test_drier_and_hotter_is_always_yellower() {
        for step in 0..10 {
            let x = step as f32 / 10.0;
            let y = x + 0.1;
            assert!(yellowness(grass_colour(climate(0.5, x))) > yellowness(grass_colour(climate(0.5, y))));
            assert!(yellowness(grass_colour(climate(y, 0.3))) > yellowness(grass_colour(climate(x, 0.3))));
        }
    }

    #[test]
    fn test_tint_draws_grass_colour_from_texture() {
        let c = climate(0.3, 0.8);
        assert!((grass_tint(c) * GRASS_TEXTURE_MEAN - grass_colour(c)).length() < 1e-6);
    }
}
