use bevy::{
    asset::Assets,
    camera::{Camera3d, Exposure},
    color::Color,
    ecs::{bundle::Bundle, component::Component, query::{With, Without}, system::{Commands, Query, Res, ResMut}},
    input::{keyboard::KeyCode, ButtonInput},
    light::{
        atmosphere::ScatteringMedium, Atmosphere, AtmosphereEnvironmentMapLight,
        CascadeShadowConfigBuilder, DirectionalLight, SunDisk,
    },
    math::{UVec2, UVec3, Vec2, Vec3},
    time::Time,
    pbr::{AtmosphereSettings, DistanceFog, FogFalloff},
    transform::components::{GlobalTransform, Transform},
};

use crate::curvature::WORLD_CURVE;

/// How opaque fog gets at its far end.
pub const FOG_MAX_OPACITY: f32 = 0.8;

/// Which way the sun is from the ground: the only thing a day/night cycle needs to move,
/// since the sky, fog and sun disc all follow the light.
pub const SUN_DIRECTION: Vec3 = Vec3::new(-0.45, 0.8, 0.4);
// How fast `move_sun` moves the sun, in radians a second, and how low and high it goes.
const SUN_SPEED: f32 = 0.2;
const SUN_LOWEST: f32 = -0.15;
const SUN_HIGHEST: f32 = 1.5;
// The keys that move the sun down and up the sky.
const SUN_DOWN_KEY: KeyCode = KeyCode::BracketLeft;
const SUN_UP_KEY: KeyCode = KeyCode::BracketRight;
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
        SunDisk { angular_size: SUN_DISC_SIZE, intensity: SUN_DISC_INTENSITY },
        Transform::default().looking_to(-SUN_DIRECTION, Vec3::Y),
        CascadeShadowConfigBuilder {
            num_cascades: cascades.max(1),
            first_cascade_far_bound: FIRST_CASCADE_DISTANCE,
            maximum_distance: shadow_distance,
            ..Default::default()
        }
        .build(),
    )
}

/// On the sun, see `move_sun`.
#[derive(Component)]
pub struct Sun;

/// Toward the sun at `elevation` radians above the horizon, on `SUN_DIRECTION`'s side of the sky.
pub fn sun_direction(elevation: f32) -> Vec3 {
    let side = Vec2::new(SUN_DIRECTION.x, SUN_DIRECTION.z).normalize();
    Vec3::new(side.x * elevation.cos(), elevation.sin(), side.y * elevation.cos())
}

/// Moves the sun down and up the sky with `[` and `]`, to see the world at any time of day:
/// the sky, fog and sun disc all follow it.
pub fn move_sun(keys: Res<ButtonInput<KeyCode>>, time: Res<Time>, mut sun: Query<&mut Transform, With<Sun>>) {
    let way = keys.pressed(SUN_UP_KEY) as i32 - keys.pressed(SUN_DOWN_KEY) as i32;
    let Ok(mut transform) = sun.single_mut() else {
        return;
    };
    if way == 0 {
        return;
    }
    // the light shines along its forward, away from the sun
    let elevation = (-transform.forward().y).asin();
    let moved = (elevation + way as f32 * SUN_SPEED * time.delta_secs()).clamp(SUN_LOWEST, SUN_HIGHEST);
    *transform = Transform::default().looking_to(-sun_direction(moved), Vec3::Y);
}

pub fn camera_exposure() -> Exposure {
    Exposure { ev100: CAMERA_EV100 }
}

/// What a camera needs to see the atmosphere, and to light with it.
pub fn atmosphere_camera() -> impl Bundle {
    (
        camera_exposure(),
        atmosphere_settings(),
        AtmosphereEnvironmentMapLight { size: UVec2::splat(ENVIRONMENT_MAP_SIZE), ..Default::default() },
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
    *planet = GlobalTransform::from_translation(Vec3::new(eye.x, eye.y - altitude - atmosphere.inner_radius, eye.z));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sun_moves_up_and_down_its_side_of_the_sky() {
        for elevation in [SUN_LOWEST, 0.0, 0.3, SUN_HIGHEST] {
            let direction = sun_direction(elevation);
            assert!((direction.length() - 1.0).abs() < 1e-5);
            assert!((direction.y.asin() - elevation).abs() < 1e-5);
            let side = Vec2::new(direction.x, direction.z).normalize();
            assert!(side.dot(Vec2::new(SUN_DIRECTION.x, SUN_DIRECTION.z).normalize()) > 0.9999);
        }
    }
}
