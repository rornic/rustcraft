use bevy::{
    asset::Assets,
    camera::{Camera3d, Exposure},
    color::Color,
    ecs::{
        bundle::Bundle,
        component::Component,
        query::{With, Without},
        resource::Resource,
        system::{Commands, Query, Res, ResMut},
    },
    input::{keyboard::KeyCode, ButtonInput},
    light::{
        atmosphere::ScatteringMedium, Atmosphere, AtmosphereEnvironmentMapLight,
        CascadeShadowConfigBuilder, DirectionalLight, SunDisk,
    },
    math::{UVec2, UVec3, Vec3},
    pbr::{AtmosphereSettings, DistanceFog, FogFalloff},
    time::Time,
    transform::components::{GlobalTransform, Transform},
};

use std::f32::consts::{PI, TAU};

use crate::curvature::WORLD_CURVE;

/// How opaque fog gets at its far end.
pub const FOG_MAX_OPACITY: f32 = 0.8;

/// Where the sun is at noon: its path across the sky is the great circle through this and the horizon, see `path_basis`.
pub const SUN_DIRECTION: Vec3 = Vec3::new(-0.45, 0.8, 0.4);
/// Seconds in a game day.
const DAY_SECONDS: f32 = 600.0;
/// Days of the clock the world starts at: 0 is midnight, 0.5 is noon.
pub const START_TIME: f32 = 0.4;
// How fast `[` and `]` wind the clock, in days a second.
const SCRUB_SPEED: f32 = 0.1;
const SCRUB_BACK_KEY: KeyCode = KeyCode::BracketLeft;
const SCRUB_FORWARD_KEY: KeyCode = KeyCode::BracketRight;
// How far past opposite the sun the moon rides, in radians, which keeps it nearly full.
const MOON_LAG: f32 = 0.5;
/// Warm, to stand out against the cool light from the sky.
pub const SUN_COLOR: Color = Color::srgb(1.0, 0.9, 0.75);
/// How bright sunlight is in the world shaders, where the camera's exposure cancels out.
const SUN_BRIGHTNESS: f32 = 1.3;
/// Fixed, so sun and sky keep the same brightness in the scene whatever the illuminance.
/// How many times brighter the sun is made than the world's shading needs, to lift the sky; `lighting.wesl` divides it back out.
const SKY_GAIN: f32 = 4.0;
const CAMERA_EV100: f32 = Exposure::EV100_SUNLIGHT;
const FIRST_CASCADE_DISTANCE: f32 = 20.0;
const ENVIRONMENT_MAP_SIZE: u32 = 64;
// About twice the real sun's size, and bright enough past white that bloom haloes it.
const SUN_DISC_SIZE: f32 = 0.02;
const SUN_DISC_INTENSITY: f32 = 1.0;
/// Colour of moonlight, cool against the sun's.
const MOON_COLOR: Color = Color::srgb(0.65, 0.78, 1.0);
/// How many times more light the shaders make of the moon's than its light gives, so the moon's
/// scattering stays too faint to light the sky while its light on the land is visible; matches `MOON_BOOST` in `lighting.wesl`.
const MOON_BOOST: f32 = 400.0;
/// How bright moonlight is in the world shaders at full moon, as `SUN_BRIGHTNESS` is for sunlight.
const MOON_BRIGHTNESS: f32 = 0.1;

/// The sun, with shadows out to `shadow_distance` blocks in `cascades` slices.
pub fn sun(shadow_distance: f32, cascades: usize) -> impl Bundle {
    (
        DirectionalLight {
            color: SUN_COLOR,
            illuminance: SUN_BRIGHTNESS * SKY_GAIN / camera_exposure().exposure(),
            shadow_maps_enabled: true,
            ..Default::default()
        },
        Sun,
        SunDisk {
            angular_size: SUN_DISC_SIZE,
            intensity: SUN_DISC_INTENSITY,
        },
        Transform::default().looking_to(-sun_direction(START_TIME), Vec3::Y),
        CascadeShadowConfigBuilder {
            num_cascades: cascades.max(1),
            first_cascade_far_bound: FIRST_CASCADE_DISTANCE,
            maximum_distance: shadow_distance,
            ..Default::default()
        }
        .build(),
    )
}

/// On the sun, see `move_sky`.
#[derive(Component)]
pub struct Sun;

/// On the moon, see `move_sky`. Without shadows, so Bevy puts it before the sun among the
/// lights, matching `MOON_LIGHT` and `SUN_LIGHT` in `sky.wesl`.
#[derive(Component)]
pub struct Moon;

/// The moon, too faint to light the sky with the atmosphere but not the land, see `MOON_BOOST`.
pub fn moon() -> impl Bundle {
    (
        DirectionalLight {
            color: MOON_COLOR,
            illuminance: MOON_BRIGHTNESS * SKY_GAIN / MOON_BOOST / camera_exposure().exposure(),
            shadow_maps_enabled: false,
            ..Default::default()
        },
        Moon,
        Transform::default().looking_to(-moon_direction(START_TIME), Vec3::Y),
    )
}

/// The time of day, in days: the whole part counts days, the rest is the clock, see `START_TIME`.
#[derive(Resource)]
pub struct TimeOfDay(pub f32);

impl TimeOfDay {
    /// Where the sun is on its path, in radians: 0 as it rises, a quarter turn at noon.
    fn sun_angle(&self) -> f32 {
        TAU * (self.0.fract() - 0.25)
    }
}

// The sun's path across the sky as a circle through `east` and `peak`, which are square to each other.
// Matches `EAST` and `PEAK` in `stars.wesl`.
fn path_basis() -> (Vec3, Vec3) {
    let peak = SUN_DIRECTION.normalize();
    (Vec3::new(peak.z, 0.0, -peak.x).normalize(), peak)
}

fn on_path(angle: f32) -> Vec3 {
    let (east, peak) = path_basis();
    east * angle.cos() + peak * angle.sin()
}

/// Toward the sun at `time` days.
pub fn sun_direction(time: f32) -> Vec3 {
    on_path(TimeOfDay(time).sun_angle())
}

/// Toward the moon at `time` days: opposite the sun, so it rises as the sun sets.
pub fn moon_direction(time: f32) -> Vec3 {
    on_path(TimeOfDay(time).sun_angle() + PI + MOON_LAG)
}

/// Runs the clock, wound with `[` and `]`, and moves the sun and moon to it: the sky, fog and
/// light all follow them.
pub fn move_sky(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut clock: ResMut<TimeOfDay>,
    mut sun: Query<&mut Transform, (With<Sun>, Without<Moon>)>,
    mut moon: Query<&mut Transform, (With<Moon>, Without<Sun>)>,
) {
    let wound = keys.pressed(SCRUB_FORWARD_KEY) as i32 - keys.pressed(SCRUB_BACK_KEY) as i32;
    clock.0 += time.delta_secs() * (1.0 / DAY_SECONDS + wound as f32 * SCRUB_SPEED);
    // the light shines along its forward, away from the luminary
    if let Ok(mut transform) = sun.single_mut() {
        *transform = Transform::default().looking_to(-sun_direction(clock.0), Vec3::Y);
    }
    if let Ok(mut transform) = moon.single_mut() {
        *transform = Transform::default().looking_to(-moon_direction(clock.0), Vec3::Y);
    }
}

pub fn camera_exposure() -> Exposure {
    Exposure {
        ev100: CAMERA_EV100,
    }
}

/// What a camera needs to see the atmosphere, and to light with it.
pub fn atmosphere_camera() -> impl Bundle {
    (
        camera_exposure(),
        atmosphere_settings(),
        AtmosphereEnvironmentMapLight {
            size: UVec2::splat(ENVIRONMENT_MAP_SIZE),
            ..Default::default()
        },
    )
}

// Smaller than Bevy's defaults: the sky is smooth, and only its colour is read for fog and ambient light.
fn atmosphere_settings() -> AtmosphereSettings {
    AtmosphereSettings {
        sky_view_lut_size: UVec2::new(192, 96),
        aerial_view_lut_size: UVec3::new(16, 16, 32),
        ..Default::default()
    }
}

/// The planet, with its ground at y = 0.
pub fn spawn_atmosphere(mut commands: Commands, mut media: ResMut<Assets<ScatteringMedium>>) {
    let medium = media.add(ScatteringMedium::earth(256, 256));
    commands.spawn(Atmosphere::earth(medium));
}

/// Sinks the atmosphere's planet under the camera so the sky's horizon dips as far below
/// level as the land's edge, see `WORLD_CURVE`. Bevy draws the sky round a planet the size of
/// the Earth, whose horizon barely dips from a mountain top, so from high up its glow would
/// hang level, far above the land's edge. Instead the sky's seen from as high above the
/// planet as it takes for its horizon to dip that far, darkening as it would from a plane.
/// Low down, where the land's edge dips no further than the Earth's would, nothing moves.
pub fn match_sky_horizon(
    camera: Query<(&GlobalTransform, &DistanceFog), With<Camera3d>>,
    mut planets: Query<(&Atmosphere, &mut GlobalTransform), Without<Camera3d>>,
) {
    let Ok((camera, fog)) = camera.single() else {
        return;
    };
    let Ok((atmosphere, mut planet)) = planets.single_mut() else {
        return;
    };
    let eye = camera.translation();
    // the land's edge is where the fog ends, as in `horizon.wesl`; underwater it's all murk
    let FogFalloff::Linear { end, .. } = fog.falloff else {
        return;
    };
    let dip = WORLD_CURVE.edge_dip(eye.y, end);
    // a sphere's horizon dips by the slope s from (R + h) / R = sqrt(1 + s²)
    let matched = atmosphere.inner_radius * ((1.0 + dip * dip).sqrt() - 1.0);
    // the planet's ground is at y = 0, as Bevy puts it, until the land's edge dips further
    let altitude = eye.y.max(matched);
    *planet = GlobalTransform::from_translation(Vec3::new(
        eye.x,
        eye.y - altitude - atmosphere.inner_radius,
        eye.z,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sun_rises_peaks_and_sets_on_its_path() {
        assert!(sun_direction(0.25).y.abs() < 1e-5);
        assert!((sun_direction(0.5) - SUN_DIRECTION.normalize()).length() < 1e-5);
        assert!(sun_direction(0.75).y.abs() < 1e-5);
        assert!(sun_direction(0.0).y < -0.7);
        for time in [0.0, 0.1, 0.3, 0.6, 0.9] {
            assert!((sun_direction(time).length() - 1.0).abs() < 1e-5);
            assert!((moon_direction(time).length() - 1.0).abs() < 1e-5);
        }
    }

    #[test]
    fn test_path_basis_matches_the_shaders() {
        let (east, peak) = path_basis();
        assert!((east - Vec3::new(0.66436, 0.0, 0.74738)).length() < 1e-4);
        assert!((peak - Vec3::new(-0.44944, 0.79900, 0.39950)).length() < 1e-4);
        assert!(east.dot(peak).abs() < 1e-5);
    }

    #[test]
    fn test_moon_stays_opposite_the_sun_and_rises_as_it_sets() {
        for time in [0.0, 0.5, 2.3, 4.5] {
            let opposite = -sun_direction(time).dot(moon_direction(time));
            assert!((opposite - MOON_LAG.cos()).abs() < 1e-4);
        }
        // at sunset the moon's already up, and at noon it's well down
        assert!(moon_direction(0.75).y > 0.0);
        assert!(moon_direction(0.5).y < -0.5);
    }
}
