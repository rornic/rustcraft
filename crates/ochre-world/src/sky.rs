use bevy::{
    app::{App, Plugin, PostUpdate, Startup, Update},
    asset::Assets,
    camera::{Camera3d, Exposure},
    color::Color,
    ecs::{
        bundle::Bundle,
        component::Component,
        query::{With, Without},
        resource::Resource,
        schedule::IntoScheduleConfigs,
        system::{Commands, Local, Query, Res, ResMut},
    },
    input::{keyboard::KeyCode, ButtonInput},
    light::{
        atmosphere::ScatteringMedium, Atmosphere, AtmosphereEnvironmentMapLight,
        CascadeShadowConfigBuilder, DirectionalLight, SimulationLightSystems, SunDisk,
    },
    math::{Quat, UVec2, UVec3, Vec3},
    pbr::{AtmosphereSettings, DistanceFog, FogFalloff},
    time::Time,
    transform::{
        components::{GlobalTransform, Transform},
        TransformSystems,
    },
};

use std::f32::consts::{PI, TAU};

use crate::clouds::CloudPlugin;
use crate::sky_dome::{NightSkyPlugin, SunGlarePlugin};
use crate::weather::WeatherPlugin;
use ochre_base::math::smoothstep;
use ochre_terrain::chunk::CHUNK_SIZE;
use ochre_terrain::curvature::WORLD_CURVE;
use ochre_terrain::settings::{CloudResolution, RendererSettings};

/// How opaque fog gets at its far end.
pub const FOG_MAX_OPACITY: f32 = 0.8;

/// Where the sun is at noon: its path across the sky is the great circle through this and the horizon, see `path_basis`.
pub(crate) const SUN_DIRECTION: Vec3 = Vec3::new(-0.45, 0.8, 0.4);
/// Seconds in a full day/night cycle: six hours, giving three hours of daylight.
pub const DAY_SECONDS: f32 = 21_600.0;
/// The clock counts whole ticks, so it stays exact however long the game runs.
pub const TICKS_PER_SECOND: u64 = 20;
pub const DAY_TICKS: u64 = DAY_SECONDS as u64 * TICKS_PER_SECOND;
/// Days of the clock the world starts at: 0 is midnight, 0.5 is noon.
pub const START_TIME: f32 = 0.4;
// How fast `[` and `]` wind the clock, in days a second.
const SCRUB_SPEED: f32 = 0.1;
const SCRUB_BACK_KEY: KeyCode = KeyCode::BracketLeft;
const SCRUB_FORWARD_KEY: KeyCode = KeyCode::BracketRight;
// How far past opposite the sun the moon rides, in radians, which keeps it nearly full.
const MOON_LAG: f32 = 0.5;
/// Warm, to stand out against the cool light from the sky.
pub(crate) const SUN_COLOR: Color = Color::srgb(1.0, 0.9, 0.75);
/// How bright sunlight is in the world shaders, where the camera's exposure cancels out.
const SUN_BRIGHTNESS: f32 = 1.3;
/// Fixed, so sun and sky keep the same brightness in the scene whatever the illuminance.
/// How many times brighter the sun is made than the world's shading needs, to lift the sky; `lighting.wesl` divides it back out.
const SKY_GAIN: f32 = 4.0;
const CAMERA_EV100: f32 = Exposure::EV100_SUNLIGHT;
const FIRST_CASCADE_DISTANCE: f32 = 20.0;
const ENVIRONMENT_MAP_SIZE: u32 = 64;
// About twice the real sun's size; matches `DISC_SIZE` in `sun_glare.wesl`, which draws it so clouds can hide it.
const SUN_DISC_SIZE: f32 = 0.02;
/// Colour of moonlight, cool against the sun's.
const MOON_COLOR: Color = Color::srgb(0.65, 0.78, 1.0);
/// How many times more light the shaders make of the moon's than its light gives, so the moon's
/// scattering stays too faint to light the sky while its light on the land is visible; matches `MOON_BOOST` in `lighting.wesl`.
const MOON_BOOST: f32 = 400.0;
/// How bright moonlight is in the world shaders at full moon, as `SUN_BRIGHTNESS` is for sunlight.
const MOON_BRIGHTNESS: f32 = 0.1;

/// The sun's illuminance in clear air, which a storm dims, see `weather.rs`.
pub fn sun_illuminance() -> f32 {
    SUN_BRIGHTNESS * SKY_GAIN / camera_exposure().exposure()
}

/// The moon's illuminance in clear air, see `sun_illuminance`.
pub fn moon_illuminance() -> f32 {
    MOON_BRIGHTNESS * SKY_GAIN / MOON_BOOST / camera_exposure().exposure()
}

/// How much it's day, 0 at night to 1 once the sun's well up, for what's about by day.
pub fn daylight(clock: &TimeOfDay) -> f32 {
    ((sun_direction(clock.days()).y + 0.05) / 0.25).clamp(0.0, 1.0)
}

/// The sun, with shadows out to `shadow_distance` blocks in `cascades` slices.
pub fn sun(shadow_distance: f32, cascades: usize) -> impl Bundle {
    (
        DirectionalLight {
            color: SUN_COLOR,
            illuminance: sun_illuminance(),
            shadow_maps_enabled: true,
            ..Default::default()
        },
        Sun,
        SunShadows(shining_from(sun_direction(START_TIME)).rotation),
        // its size still dims the light as it sets; the disc itself is drawn by `sun_glare.wesl`
        SunDisk {
            angular_size: SUN_DISC_SIZE,
            intensity: 0.0,
        },
        shining_from(sun_direction(START_TIME)),
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

/// Which way the sun casts its shadows: turned in steps, see `aim_sun_shadows`.
#[derive(Component)]
struct SunShadows(Quat);

// The shadow map turns 20 times a second, holding before it slides to its next angle.
const SHADOW_STEP_TICKS: u64 = TICKS_PER_SECOND / 20;
const SHADOW_SLIDE: f32 = 0.5;

/// On the moon, see `move_sky`. Without shadows, so Bevy puts it before the sun among the
/// lights, matching `MOON_LIGHT` and `SUN_LIGHT` in `sky.wesl`.
#[derive(Component)]
pub struct Moon;

/// The moon, too faint to light the sky with the atmosphere but not the land, see `MOON_BOOST`.
pub fn moon() -> impl Bundle {
    (
        DirectionalLight {
            color: MOON_COLOR,
            illuminance: moon_illuminance(),
            shadow_maps_enabled: false,
            ..Default::default()
        },
        Moon,
        shining_from(moon_direction(START_TIME)),
    )
}

/// The time in ticks since midnight of day 0, see `DAY_TICKS` and `START_TIME`.
#[derive(Resource)]
pub struct TimeOfDay(pub u64);

impl TimeOfDay {
    pub fn from_days(days: f32) -> Self {
        Self((days.max(0.0) * DAY_TICKS as f32).round() as u64)
    }

    /// How far through the day it is, 0..1: 0 is midnight, 0.5 is noon.
    pub fn days(&self) -> f32 {
        (self.0 % DAY_TICKS) as f32 / DAY_TICKS as f32
    }
}

/// Where the sun is on its path at `days` through the day, in radians: 0 as it rises, a quarter
/// turn at noon.
fn sun_angle(days: f32) -> f32 {
    TAU * (days.fract() - 0.25)
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

/// Toward the sun `time` of the way through the day.
pub fn sun_direction(time: f32) -> Vec3 {
    on_path(sun_angle(time))
}

/// Toward the moon `time` of the way through the day: opposite the sun, so it rises as the sun sets.
pub(crate) fn moon_direction(time: f32) -> Vec3 {
    on_path(sun_angle(time) + PI + MOON_LAG)
}

/// Runs the clock, wound with `[` and `]`, and moves the sun and moon to it: the sky, fog and
/// light all follow them.
fn move_sky(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut clock: ResMut<TimeOfDay>,
    mut sun: Query<(&mut Transform, &mut SunShadows), Without<Moon>>,
    mut moon: Query<&mut Transform, (With<Moon>, Without<Sun>)>,
    // the part of a tick not yet counted
    mut carry: Local<f32>,
) {
    let wound = keys.pressed(SCRUB_FORWARD_KEY) as i32 - keys.pressed(SCRUB_BACK_KEY) as i32;
    *carry += time.delta_secs()
        * (TICKS_PER_SECOND as f32 + wound as f32 * SCRUB_SPEED * DAY_TICKS as f32);
    let whole = carry.floor();
    *carry -= whole;
    clock.0 = clock.0.saturating_add_signed(whole as i64);
    // on between ticks too: stepped 20 times a second, the sun's shadow-map texels jump and shadows crawl
    let days = clock.days() + *carry / DAY_TICKS as f32;
    if let Ok((mut transform, mut shadows)) = sun.single_mut() {
        *transform = shining_from(sun_direction(days));
        shadows.0 = shining_from(sun_direction(shadow_days(clock.0, *carry))).rotation;
    }
    if let Ok(mut transform) = moon.single_mut() {
        *transform = shining_from(moon_direction(days));
    }
}

/// The day fraction the sun's shadows follow: held at the last step, then slid to the next at its end.
fn shadow_days(ticks: u64, carry: f32) -> f32 {
    let step = ticks - ticks % SHADOW_STEP_TICKS;
    let into = ((ticks - step) as f32 + carry) / SHADOW_STEP_TICKS as f32;
    let slid = smoothstep(1.0 - SHADOW_SLIDE, 1.0, into);
    ((step % DAY_TICKS) as f32 + slid * SHADOW_STEP_TICKS as f32) / DAY_TICKS as f32
}

/// Turns the sun to `SunShadows` while its shadow maps fit, as turning them every frame makes the texels crawl.
fn aim_sun_shadows(mut sun: Query<(&mut GlobalTransform, &SunShadows)>) {
    for (mut transform, shadows) in &mut sun {
        *transform = GlobalTransform::from_rotation(shadows.0);
    }
}

/// Turns the sun back once its shadow maps are fitted, so its light and disc still move smoothly.
fn aim_sun_light(mut sun: Query<(&mut GlobalTransform, &Transform), With<SunShadows>>) {
    for (mut global, transform) in &mut sun {
        *global = GlobalTransform::from(*transform);
    }
}

// A light shines along its forward, away from where it is in the sky.
fn shining_from(direction: Vec3) -> Transform {
    Transform::default().looking_to(-direction, Vec3::Y)
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
fn spawn_atmosphere(mut commands: Commands, mut media: ResMut<Assets<ScatteringMedium>>) {
    let medium = media.add(ScatteringMedium::earth(256, 256));
    commands.spawn(Atmosphere::earth(medium));
}

/// Sinks the atmosphere's planet under the camera so the sky's horizon dips as far below
/// level as the land's edge, see `WORLD_CURVE`. Bevy draws the sky round a planet the size of
/// the Earth, whose horizon barely dips from a mountain top, so from high up its glow would
/// hang level, far above the land's edge. Instead the sky's seen from as high above the
/// planet as it takes for its horizon to dip that far, darkening as it would from a plane.
/// Low down, where the land's edge dips no further than the Earth's would, nothing moves.
fn match_sky_horizon(
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

/// Everything overhead: the clock, sun, moon and atmosphere, the night sky and sun's glare, clouds
/// and weather. `OCHRE_TIME` starts the clock at that time of day, in days, for looking at the sky
/// at any hour.
pub struct SkyPlugin {
    /// How far the sun casts shadows, in blocks, and in how many slices.
    pub shadow_distance: f32,
    pub shadow_cascades: usize,
    pub cloud_resolution: CloudResolution,
}

impl SkyPlugin {
    pub fn new(renderer: &RendererSettings) -> Self {
        Self {
            shadow_distance: (renderer.shadow_distance as f32 * CHUNK_SIZE as f32)
                .min(renderer.fog_distance()),
            shadow_cascades: renderer.shadow_cascades,
            cloud_resolution: renderer.cloud_resolution,
        }
    }
}

impl Plugin for SkyPlugin {
    fn build(&self, app: &mut App) {
        let start = std::env::var("OCHRE_TIME")
            .ok()
            .and_then(|time| time.parse().ok())
            .unwrap_or(START_TIME);
        let (distance, cascades) = (self.shadow_distance, self.shadow_cascades);
        app.add_plugins((
            NightSkyPlugin,
            SunGlarePlugin,
            CloudPlugin {
                resolution: self.cloud_resolution,
            },
            WeatherPlugin,
        ))
        .insert_resource(TimeOfDay::from_days(start))
        .add_systems(
            Startup,
            (spawn_atmosphere, move |mut commands: Commands| {
                commands.spawn(sun(distance, cascades));
                commands.spawn(moon());
            }),
        )
        .add_systems(Update, move_sky)
        // once the camera's moved, see `TransformSystems::Propagate`
        .add_systems(
            PostUpdate,
            (
                match_sky_horizon.after(TransformSystems::Propagate),
                aim_sun_shadows
                    .after(TransformSystems::Propagate)
                    .before(SimulationLightSystems::UpdateDirectionalLightCascades),
                aim_sun_light.after(SimulationLightSystems::UpdateDirectionalLightCascades),
                crate::shadow::prune_shadow_casters
                    .after(SimulationLightSystems::CheckLightVisibility),
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_shadows_hold_then_slide_on_to_the_next_step() {
        let step = super::SHADOW_STEP_TICKS;
        let day = super::DAY_TICKS as f32;
        assert_eq!(step, 1);
        let start = super::shadow_days(3 * step, 0.0);
        assert_eq!(start, super::shadow_days(3 * step, 0.3));
        let middle = super::shadow_days(3 * step, 0.75);
        let end = super::shadow_days(4 * step - 1, 0.999);
        assert!(start < middle && middle < end);
        assert!((end - 4.0 * step as f32 / day).abs() < 1e-8, "{end}");
        assert!((super::shadow_days(4 * step, 0.0) - end).abs() < 1e-8);
    }

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
