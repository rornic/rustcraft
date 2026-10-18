//! Storms now and then: darker skies and clouds, rain and its sound, a stronger wind,
//! lightning and thunder, and a rainbow once the rain clears in sunshine.
use std::{
    f32::consts::{PI, TAU},
    time::Duration,
};

use bevy::{
    asset::RenderAssetUsages,
    audio::{AddAudioSource, ChannelCount, Decodable, SampleRate, Source},
    camera::{visibility::NoFrustumCulling, Exposure},
    light::{NotShadowCaster, NotShadowReceiver},
    math::ops,
    mesh::{Indices, PrimitiveTopology},
    pbr::{MaterialPipeline, MaterialPipelineKey},
    prelude::*,
    reflect::TypePath,
    render::{
        mesh::MeshVertexBufferLayoutRef,
        render_resource::{AsBindGroup, RenderPipelineDescriptor, SpecializedMeshPipelineError},
    },
    shader::ShaderRef,
};

use crate::{
    ambience::{Dial, SAMPLE_RATE},
    block::BlockType,
    clouds::CloudMaterial,
    math::smoothstep,
    sky::{
        camera_exposure, moon_illuminance, sun_direction, sun_illuminance, Moon, Sun, TimeOfDay,
        TICKS_PER_SECOND,
    },
    world::World,
};

pub struct WeatherPlugin;

impl Plugin for WeatherPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            MaterialPlugin::<RainMaterial>::default(),
            MaterialPlugin::<RainbowMaterial>::default(),
        ))
        .add_audio_source::<RainSound>()
        .add_audio_source::<Thunder>()
        .init_resource::<Weather>()
        .add_systems(Startup, spawn_weather)
        .add_systems(
            Update,
            (
                run_weather,
                (darken_sky, fall_rain, strike_lightning, show_rainbow),
            )
                .chain(),
        );
    }
}

/// How the weather is now.
#[derive(Resource)]
pub struct Weather {
    // how hard it's raining, 0..1, and how hard it's set to, which it eases toward
    pub rain: f32,
    target: f32,
    // seconds until the weather next turns, whether a storm's on, and how many there've been
    turn_in: f32,
    storming: bool,
    storms: u32,
    // how strong the rainbow after a storm is, 0..1, and how long it has left
    rainbow: f32,
    rainbow_left: f32,
    // seconds until the next lightning strike
    strike_in: f32,
    // the rain's level, shared with its sound on the audio thread
    sound: Dial,
}

impl Default for Weather {
    fn default() -> Self {
        Self {
            rain: 0.0,
            target: 0.0,
            turn_in: FIRST_STORM_IN,
            storming: false,
            storms: 0,
            rainbow: 0.0,
            rainbow_left: 0.0,
            strike_in: 6.0,
            sound: Dial::default(),
        }
    }
}

// The first storm comes soon after the world opens, while the sun's still up for a rainbow
// after it; later ones come and go at random.
const FIRST_STORM_IN: f32 = 25.0;
const FIRST_STORM_SECONDS: f32 = 95.0;
const CLEAR_SECONDS: (f32, f32) = (240.0, 480.0);
const STORM_SECONDS: (f32, f32) = (90.0, 180.0);
// How long rain takes to come on or clear, in seconds.
const RAIN_EASE_SECONDS: f32 = 12.0;
// How long a rainbow stays after a storm, in seconds, and how long it takes to come and go.
const RAINBOW_SECONDS: f32 = 70.0;
const RAINBOW_FADE_SECONDS: f32 = 10.0;

impl Weather {
    // Moves the weather on by `dt` seconds.
    fn advance(&mut self, dt: f32) {
        self.turn_in -= dt;
        if self.turn_in <= 0.0 {
            self.storming = !self.storming;
            if self.storming {
                self.target = rand::random_range(0.75..1.0);
                self.turn_in = if self.storms == 0 {
                    FIRST_STORM_SECONDS
                } else {
                    rand::random_range(STORM_SECONDS.0..STORM_SECONDS.1)
                };
                self.storms += 1;
            } else {
                self.target = 0.0;
                self.turn_in = rand::random_range(CLEAR_SECONDS.0..CLEAR_SECONDS.1);
                self.rainbow_left = RAINBOW_SECONDS;
            }
        }
        let ease = 1.0 - ops::exp(-dt / RAIN_EASE_SECONDS * 3.0);
        self.rain += (self.target - self.rain) * ease;
        self.rainbow_left = (self.rainbow_left - dt).max(0.0);
        // it shows once the rain's nearly cleared, and fades before its time's up
        let wanted = if self.rainbow_left > RAINBOW_FADE_SECONDS && self.rain < 0.15 {
            1.0
        } else {
            0.0
        };
        let step = dt / RAINBOW_FADE_SECONDS;
        self.rainbow = (self.rainbow + (wanted - self.rainbow).clamp(-step, step)).clamp(0.0, 1.0);
        self.sound.set(self.rain);
    }

    /// Whether it's raining hard enough that what's out and about takes shelter.
    pub fn sheltering(&self) -> bool {
        self.rain > SHELTER_RAIN
    }
}

// How hard it rains, see `Weather::rain`, before birds and insects take shelter.
const SHELTER_RAIN: f32 = 0.2;

// The weather keeps game time, so winding the clock on winds it on too, and back holds it.
// Lightning keeps real time, or a wound-on storm would strobe.
fn run_weather(clock: Res<TimeOfDay>, mut weather: ResMut<Weather>, mut last: Local<Option<u64>>) {
    let now = clock.0;
    let then = last.replace(now).unwrap_or(now);
    weather.advance(weather_seconds(then, now));
}

/// How many seconds of weather pass as the clock goes from `then` to `now`, in ticks.
pub fn weather_seconds(then: u64, now: u64) -> f32 {
    now.saturating_sub(then) as f32 / TICKS_PER_SECOND as f32
}

// How much of the sun's and moon's light gets through the storm's clouds, and how dark the
// clouds themselves get, at the heaviest rain.
const STORM_LIGHT: f32 = 0.3;
const STORM_CLOUD: f32 = 0.42;
// Lightning's flash, in stops brighter: held, then dying away, in seconds, with a fainter
// flicker after.
const FLASH_STOPS: f32 = 3.2;
const FLASH_HOLD_SECONDS: f32 = 0.08;
const FLASH_SECONDS: f32 = 0.5;
const FLICKER_STOPS: f32 = 2.2;

#[derive(Component)]
struct Flash {
    // seconds since the strike, and when the second, fainter flicker comes
    age: f32,
    flicker: f32,
}

impl Flash {
    // How many stops brighter it lights everything now.
    fn stops(&self) -> f32 {
        let dying = |since: f32| {
            let t = ((since - FLASH_HOLD_SECONDS) / FLASH_SECONDS).clamp(0.0, 1.0);
            if since < 0.0 {
                0.0
            } else {
                (1.0 - t).powf(2.5)
            }
        };
        (FLASH_STOPS * dying(self.age)).max(FLICKER_STOPS * dying(self.age - self.flicker))
    }
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn darken_sky(
    weather: Res<Weather>,
    time: Res<Time>,
    mut lights: Query<(&mut DirectionalLight, Has<Sun>), Or<(With<Sun>, With<Moon>)>>,
    mut clouds: ResMut<Assets<CloudMaterial>>,
    mut cameras: Query<&mut Exposure, With<Camera3d>>,
    mut flashes: Query<(Entity, &mut Flash)>,
    mut commands: Commands,
    mut last_shade: Local<f32>,
) {
    let shade = 1.0 - (1.0 - STORM_LIGHT) * weather.rain;
    for (mut light, is_sun) in &mut lights {
        light.illuminance = shade
            * if is_sun {
                sun_illuminance()
            } else {
                moon_illuminance()
            };
    }
    let cloud_shade = 1.0 - (1.0 - STORM_CLOUD) * weather.rain;
    if (cloud_shade - *last_shade).abs() > 0.002 {
        *last_shade = cloud_shade;
        for (_, cloud) in clouds.iter_mut() {
            cloud.set_shade(cloud_shade);
        }
    }

    // lightning lights everything the moment it strikes, holds, dies away and flickers once
    let mut stops = 0.0f32;
    for (entity, mut flash) in &mut flashes {
        flash.age += time.delta_secs();
        stops = stops.max(flash.stops());
        if flash.age > flash.flicker + FLASH_HOLD_SECONDS + FLASH_SECONDS {
            commands.entity(entity).despawn();
        }
    }
    for mut exposure in &mut cameras {
        exposure.ev100 = camera_exposure().ev100 - stops;
    }
}

// A see-through effect drawn from both sides by its own shaders, which casts nothing and
// takes one uniform: what it needs to know this frame.
macro_rules! effect_material {
    ($(#[$doc:meta])* $material:ident, vertex: $vertex:expr, fragment: $fragment:literal) => {
        $(#[$doc])*
        #[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
        pub struct $material {
            #[uniform(0)]
            params: Vec4,
        }

        impl Material for $material {
            fn vertex_shader() -> ShaderRef {
                $vertex
            }

            fn fragment_shader() -> ShaderRef {
                $fragment.into()
            }

            fn alpha_mode(&self) -> AlphaMode {
                AlphaMode::Premultiplied
            }

            fn enable_prepass() -> bool {
                false
            }

            fn enable_shadows() -> bool {
                false
            }

            fn specialize(
                _pipeline: &MaterialPipeline,
                descriptor: &mut RenderPipelineDescriptor,
                _layout: &MeshVertexBufferLayoutRef,
                _key: MaterialPipelineKey<Self>,
            ) -> Result<(), SpecializedMeshPipelineError> {
                descriptor.primitive.cull_mode = None;
                Ok(())
            }
        }
    };
}

effect_material!(
    /// The streaks of rain, falling in `rain.wesl`.
    RainMaterial,
    vertex: "shaders/rain.wesl".into(),
    fragment: "shaders/rain.wesl"
);
effect_material!(
    /// The rainbow's band, coloured in `rainbow.wesl`.
    RainbowMaterial,
    vertex: ShaderRef::Default,
    fragment: "shaders/rainbow.wesl"
);

// The box of rain round the camera, in blocks, matching `BOX` in `rain.wesl`, and how many
// streaks fill it in the heaviest rain.
const RAIN_BOX: Vec3 = Vec3::new(48.0, 32.0, 48.0);
const STREAKS: usize = 14_000;
// The wind the rain slants in, in blocks a second.
const RAIN_WIND: Vec2 = Vec2::new(3.0, 1.5);

#[derive(Resource)]
struct WeatherHandles {
    rain: Handle<RainMaterial>,
    rainbow: Handle<RainbowMaterial>,
    bolt: Handle<StandardMaterial>,
}

#[derive(Component)]
struct Rainbow;

/// The streaks of rain round the camera, hidden while it's dry.
#[derive(Component)]
struct RainCurtain;

// Shows or hides an effect, leaving it be when it's already so, as change detection would
// otherwise re-extract it every frame.
fn show_if(visibility: &mut Mut<Visibility>, shown: bool) {
    visibility.set_if_neq(if shown {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    });
}

fn spawn_weather(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut rains: ResMut<Assets<RainMaterial>>,
    mut rainbows: ResMut<Assets<RainbowMaterial>>,
    mut standard: ResMut<Assets<StandardMaterial>>,
    mut sounds: ResMut<Assets<RainSound>>,
    weather: Res<Weather>,
) {
    let rain = rains.add(RainMaterial { params: Vec4::ZERO });
    let rainbow = rainbows.add(RainbowMaterial { params: Vec4::ZERO });
    commands.spawn((
        RainCurtain,
        Mesh3d(meshes.add(rain_mesh())),
        MeshMaterial3d(rain.clone()),
        Transform::default(),
        NoFrustumCulling,
        NotShadowCaster,
        NotShadowReceiver,
    ));
    commands.spawn((
        Rainbow,
        Mesh3d(meshes.add(rainbow_mesh())),
        MeshMaterial3d(rainbow.clone()),
        Transform::default(),
        NoFrustumCulling,
        NotShadowCaster,
        NotShadowReceiver,
    ));
    let bolt = standard.add(StandardMaterial {
        base_color: Color::BLACK,
        emissive: LinearRgba::rgb(4.0, 4.4, 6.0),
        emissive_exposure_weight: 0.0,
        fog_enabled: false,
        cull_mode: None,
        ..default()
    });
    commands.insert_resource(WeatherHandles {
        rain,
        rainbow,
        bolt,
    });
    commands.spawn((
        AudioPlayer(sounds.add(RainSound {
            level: weather.sound.clone(),
        })),
        PlaybackSettings::ONCE,
    ));
}

// Streaks scattered through the box, each four corners at one spot, told apart by their uv.
fn rain_mesh() -> Mesh {
    let mut positions = Vec::with_capacity(STREAKS * 4);
    let mut uvs = Vec::with_capacity(STREAKS * 4);
    let mut indices = Vec::with_capacity(STREAKS * 6);
    for i in 0..STREAKS {
        let spot = Vec3::new(
            rand::random_range(-0.5..0.5),
            rand::random_range(-0.5..0.5),
            rand::random_range(-0.5..0.5),
        ) * RAIN_BOX;
        let start = (i * 4) as u32;
        for uv in [[-1.0, 0.0], [1.0, 0.0], [-1.0, 1.0], [1.0, 1.0]] {
            positions.push(spot.to_array());
            uvs.push(uv);
        }
        indices.extend([0, 2, 1, 1, 2, 3].map(|k| start + k));
    }
    let count = positions.len();
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 1.0, 0.0]; count])
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    // `rain.wesl` passes its own values along in the colour
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, vec![[1.0f32; 4]; count])
    .with_inserted_indices(Indices::U32(indices))
}

// A unit ring in its xy plane, its band from 0.95 to 1 of the way out, uv.x across the band.
fn rainbow_mesh() -> Mesh {
    const SEGMENTS: usize = 128;
    const INNER: f32 = 0.95;
    let mut positions = Vec::new();
    let mut uvs = Vec::new();
    let mut indices = Vec::new();
    for i in 0..=SEGMENTS {
        let angle = i as f32 / SEGMENTS as f32 * TAU;
        let (sin, cos) = ops::sin_cos(angle);
        positions.push([cos * INNER, sin * INNER, 0.0]);
        positions.push([cos, sin, 0.0]);
        uvs.push([0.0, 0.0]);
        uvs.push([1.0, 0.0]);
        if i < SEGMENTS {
            let k = (i * 2) as u32;
            indices.extend([k, k + 1, k + 2, k + 2, k + 1, k + 3]);
        }
    }
    let count = positions.len();
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 0.0, 1.0]; count])
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_indices(Indices::U32(indices))
}

/// How bright the daylight is, 0..1, for things that aren't lit by the world's shaders.
fn unlit_light(clock: &TimeOfDay) -> f32 {
    ((sun_direction(clock.days()).y + 0.1) / 0.4).clamp(0.05, 1.0)
}

fn fall_rain(
    weather: Res<Weather>,
    clock: Res<TimeOfDay>,
    world: Res<World>,
    handles: Option<Res<WeatherHandles>>,
    camera: Query<&GlobalTransform, With<Camera3d>>,
    mut rains: ResMut<Assets<RainMaterial>>,
    mut curtains: Query<&mut Visibility, With<RainCurtain>>,
) {
    let (Some(handles), Ok(camera)) = (handles, camera.single()) else {
        return;
    };
    let eye = camera.translation();
    // none underwater, nor once high above the clouds' base
    let dry = world.get_block_at(eye.floor().as_i64vec3()) == BlockType::Water
        || eye.y > crate::clouds::CLOUD_BASE;
    let rain = if dry { 0.0 } else { weather.rain };
    let light = unlit_light(&clock) * (1.0 - 0.5 * weather.rain);
    for mut visibility in &mut curtains {
        show_if(&mut visibility, rain > 0.0);
    }
    let params = Vec4::new(rain, light, RAIN_WIND.x, RAIN_WIND.y);
    // written only when it changes, as writing marks it for the GPU again
    if rains
        .get(&handles.rain)
        .is_some_and(|material| material.params != params)
    {
        if let Some(mut material) = rains.get_mut(&handles.rain) {
            material.params = params;
        }
    }
}

// Lightning strikes this often in the heaviest rain, at the least and most, in seconds, from
// this far off, in blocks, and its thunder rolls in at the speed of sound in blocks a second,
// sped up so it isn't a long wait.
const STRIKE_SECONDS: (f32, f32) = (7.0, 22.0);
const STRIKE_DISTANCE: (f32, f32) = (30.0, 520.0);
const THUNDER_SPEED: f32 = 240.0;
// Now and then a strike lands close, within this many blocks; the rest are farther off.
const CLOSE_STRIKE_CHANCE: f64 = 0.25;
const CLOSE_STRIKE_WITHIN: f32 = 110.0;
// How long a bolt shows, in seconds.
const BOLT_SECONDS: f32 = 0.3;

#[derive(Component)]
struct Bolt {
    age: f32,
}

#[derive(Component)]
struct ThunderDue {
    in_seconds: f32,
    distance: f32,
}

#[allow(clippy::too_many_arguments)]
fn strike_lightning(
    mut commands: Commands,
    time: Res<Time>,
    mut weather: ResMut<Weather>,
    world: Res<World>,
    handles: Option<Res<WeatherHandles>>,
    camera: Query<&GlobalTransform, With<Camera3d>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut thunders: ResMut<Assets<Thunder>>,
    mut bolts: Query<(Entity, &mut Bolt, &mut Visibility)>,
    mut due: Query<(Entity, &mut ThunderDue)>,
) {
    let dt = time.delta_secs();
    for (entity, mut bolt, mut visibility) in &mut bolts {
        bolt.age += dt;
        // it flickers as it goes
        *visibility = if bolt.age < BOLT_SECONDS && (bolt.age * 30.0) as u32 % 3 != 1 {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        if bolt.age > BOLT_SECONDS {
            commands.entity(entity).despawn();
        }
    }
    for (entity, mut thunder) in &mut due {
        thunder.in_seconds -= dt;
        if thunder.in_seconds <= 0.0 {
            commands.entity(entity).despawn();
            commands.spawn((
                AudioPlayer(thunders.add(Thunder {
                    distance: thunder.distance,
                    seed: rand::random(),
                })),
                PlaybackSettings::DESPAWN,
            ));
        }
    }

    let (Some(handles), Ok(camera)) = (handles, camera.single()) else {
        return;
    };
    if weather.rain < 0.6 {
        return;
    }
    weather.strike_in -= dt;
    if weather.strike_in > 0.0 {
        return;
    }
    weather.strike_in = rand::random_range(STRIKE_SECONDS.0..STRIKE_SECONDS.1);

    let eye = camera.translation();
    // mostly somewhere ahead, where it's seen
    let facing = camera.forward().with_y(0.0).normalize_or(Vec3::Z);
    let angle = ops::atan2(facing.z, facing.x) + rand::random_range(-0.55..0.55);
    let distance = if rand::random_bool(CLOSE_STRIKE_CHANCE) {
        rand::random_range(STRIKE_DISTANCE.0..CLOSE_STRIKE_WITHIN)
    } else {
        rand::random_range(CLOSE_STRIKE_WITHIN..STRIKE_DISTANCE.1)
    };
    let foot = eye + Vec3::new(ops::cos(angle), 0.0, ops::sin(angle)) * distance;
    // from the noise, as the region there may not be worked out, and working it out here
    // would stall the frame the bolt strikes on
    let ground = world
        .region_store
        .noise()
        .sample(foot.x.floor() as f64, foot.z.floor() as f64)
        .height
        .max(crate::chunks::generate::WATER_LEVEL as f64) as f32;
    let foot = foot.with_y(ground);
    let top = foot.with_y(crate::clouds::CLOUD_BASE);
    commands.spawn((
        Bolt { age: 0.0 },
        Mesh3d(meshes.add(bolt_mesh(top, foot, eye, distance))),
        MeshMaterial3d(handles.bolt.clone()),
        Transform::default(),
        Visibility::Visible,
        NoFrustumCulling,
        NotShadowCaster,
        NotShadowReceiver,
    ));
    commands.spawn(Flash {
        age: 0.0,
        flicker: rand::random_range(0.12..0.22),
    });
    commands.spawn(ThunderDue {
        in_seconds: distance / THUNDER_SPEED,
        distance,
    });
}

// A forked bolt from `top` down to `foot`, as quads turned to face `eye`: a jagged trunk and a
// branch or two off it. Close up it's drawn thinner and wanders less, so it isn't a fat zigzag
// filling the view.
fn bolt_mesh(top: Vec3, foot: Vec3, eye: Vec3, distance: f32) -> Mesh {
    let scale = (distance / 160.0).clamp(0.4, 1.0);
    let mut positions = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut segment = |a: Vec3, b: Vec3, width: f32| {
        let along = (b - a).normalize_or(Vec3::Y);
        let across = along
            .cross((eye - a).normalize_or(Vec3::Z))
            .normalize_or(Vec3::X)
            * width;
        let k = positions.len() as u32;
        positions.extend([a - across, a + across, b - across, b + across].map(|p| p.to_array()));
        indices.extend([0, 2, 1, 1, 2, 3].map(|i| k + i));
    };
    let jagged = |from: Vec3, to: Vec3, steps: usize, wander: f32| {
        let mut points = vec![from];
        for i in 1..steps {
            let t = i as f32 / steps as f32;
            let jitter = Vec3::new(
                rand::random_range(-1.0..1.0),
                0.0,
                rand::random_range(-1.0..1.0),
            ) * wander;
            points.push(from.lerp(to, t) + jitter);
        }
        points.push(to);
        points
    };
    let trunk = jagged(top, foot, 18, 14.0 * scale);
    for pair in trunk.windows(2) {
        segment(pair[0], pair[1], 1.4 * scale);
    }
    for _ in 0..rand::random_range(1..4) {
        let from = trunk[rand::random_range(3..trunk.len() - 4)];
        let reach = Vec3::new(
            rand::random_range(-60.0..60.0),
            -rand::random_range(40.0..110.0),
            rand::random_range(-60.0..60.0),
        );
        let branch = jagged(from, from + reach, 7, 8.0 * scale);
        for pair in branch.windows(2) {
            segment(pair[0], pair[1], 0.7 * scale);
        }
    }
    let count = positions.len();
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 1.0, 0.0]; count])
    .with_inserted_indices(Indices::U32(indices))
}

// How far off the rainbow hangs, in blocks: near enough to be in front of the far hills'
// fog, and how wide it is from the point opposite the sun, as real ones are.
const RAINBOW_DISTANCE: f32 = 700.0;
const RAINBOW_ANGLE: f32 = 42.0 * PI / 180.0;

fn show_rainbow(
    weather: Res<Weather>,
    clock: Res<TimeOfDay>,
    handles: Option<Res<WeatherHandles>>,
    camera: Query<&GlobalTransform, With<Camera3d>>,
    mut bows: Query<(&mut Transform, &mut Visibility), With<Rainbow>>,
    mut materials: ResMut<Assets<RainbowMaterial>>,
) {
    let (Some(handles), Ok(camera)) = (handles, camera.single()) else {
        return;
    };
    let eye = camera.translation();
    let away = -sun_direction(clock.days());
    // only by day, with the sun low enough that some of the bow clears the horizon: once
    // it's higher than the bow is wide, all of it's below
    let sun_height = -away.y;
    let strength = weather.rainbow
        * smoothstep(-0.02, 0.08, sun_height)
        * (1.0 - smoothstep(0.6, 0.68, sun_height));
    if let Ok((mut transform, mut visibility)) = bows.single_mut() {
        show_if(&mut visibility, strength > 0.0);
        if strength > 0.0 {
            *transform = Transform::from_translation(eye + away * RAINBOW_DISTANCE)
                .looking_to(away, Vec3::Y)
                .with_scale(Vec3::splat(RAINBOW_DISTANCE * ops::tan(RAINBOW_ANGLE)));
        }
    }
    let params = Vec4::new(strength, unlit_light(&clock), 0.0, 0.0);
    if materials
        .get(&handles.rainbow)
        .is_some_and(|material| material.params != params)
    {
        if let Some(mut material) = materials.get_mut(&handles.rainbow) {
            material.params = params;
        }
    }
}

// xorshift: plenty for noise
fn next_random(state: &mut u64) -> f32 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    (*state >> 40) as f32 / (1u64 << 24) as f32 * 2.0 - 1.0
}

/// Rain's hiss, made as it plays: noise with the rumble and the sharpest highs cut, swelling
/// gently, as loud as the rain.
#[derive(Asset, TypePath)]
pub struct RainSound {
    level: Dial,
}

impl Decodable for RainSound {
    type Decoder = RainDecoder;

    fn decoder(&self) -> RainDecoder {
        RainDecoder {
            level: self.level.clone(),
            heard: 0.0,
            rng: rand::random::<u64>() | 1,
            channel: 0,
            low: [0.0; 2],
            high: [0.0; 2],
            body: [0.0; 2],
            swell: 0.0,
            t: 0,
        }
    }
}

// The rain's loudness at its heaviest, see `RainDecoder::next`.
const RAIN_GAIN: f32 = 0.11;

pub struct RainDecoder {
    level: Dial,
    heard: f32,
    rng: u64,
    channel: usize,
    // per ear: a low-pass taking the harshest highs off, one finding the rumble to take away,
    // and a soft low body of drumming
    low: [f32; 2],
    high: [f32; 2],
    body: [f32; 2],
    swell: f32,
    t: u64,
}

impl Iterator for RainDecoder {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.channel == 0 {
            self.t += 1;
            if self.t.is_multiple_of(64) {
                let wanted = self.level.get();
                self.heard += (wanted - self.heard) * 0.004;
                let t = self.t as f32 / SAMPLE_RATE as f32;
                self.swell = 0.85 + 0.15 * ops::sin(t * 0.7) * ops::sin(t * 0.23 + 1.0);
            }
        }
        let c = self.channel;
        self.channel = 1 - self.channel;
        let white = next_random(&mut self.rng);
        // low-passed near 5 kHz, then the part under ~500 Hz taken out
        self.low[c] += (white - self.low[c]) * 0.5;
        self.high[c] += (self.low[c] - self.high[c]) * 0.07;
        let hiss = self.low[c] - self.high[c];
        self.body[c] += (white - self.body[c]) * 0.02;
        let sample = (hiss * 0.8 + self.body[c] * 1.2) * self.heard.sqrt() * self.swell * RAIN_GAIN;
        Some(sample.clamp(-1.0, 1.0))
    }
}

impl Source for RainDecoder {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> ChannelCount {
        ChannelCount::new(2).unwrap()
    }

    fn sample_rate(&self) -> SampleRate {
        SampleRate::new(SAMPLE_RATE).unwrap()
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

/// One clap of thunder from `distance` blocks off: close by, a rapid run of sharp cracks over a
/// deep boom; farther off, only a softer rumble. Then a long roll that swells again as the far
/// parts of the bolt are heard, echoes, and dies away.
#[derive(Asset, TypePath)]
pub struct Thunder {
    distance: f32,
    seed: u64,
}

// How long a clap lasts, in seconds, and its loudness from the nearest strikes.
const THUNDER_SECONDS: f32 = 7.0;
const THUNDER_GAIN: f32 = 0.46;
// The roll's gain from the farthest strikes and the nearest.
const ROLL_GAIN: (f32, f32) = (0.30, 1.16);
// How far back, in frames, the two echoes come from, ~120 ms and ~270 ms; the first heard in
// the other ear.
const ECHO_TAPS: (usize, usize) = (5_200, 11_800);
const ECHO_FRAMES: usize = ECHO_TAPS.1 + 1;

impl Decodable for Thunder {
    type Decoder = ThunderDecoder;

    fn decoder(&self) -> ThunderDecoder {
        // by the log of the distance, as loudness falls off
        let near = 1.0
            - (ops::ln(self.distance / STRIKE_DISTANCE.0)
                / ops::ln(STRIKE_DISTANCE.1 / STRIKE_DISTANCE.0))
            .clamp(0.0, 1.0);
        let mut rng = (self.seed << 1) | 1;
        let mut uniform = || next_random(&mut rng) * 0.5 + 0.5;

        // the roll swells again round 0.7, 1.5, 2.5 and 3.7 seconds in, each later one broader,
        // quieter and wider apart in the ears
        let swells: [Swell; 4] = [
            Swell {
                at: 0.72 + uniform() * 0.20,
                rise: 0.15 + uniform() * 0.06,
                decay: 0.75 + uniform() * 0.25,
                size: 0.65 + uniform() * 0.28,
                pan: (uniform() * 2.0 - 1.0) * 0.55,
            },
            Swell {
                at: 1.48 + uniform() * 0.32,
                rise: 0.18 + uniform() * 0.08,
                decay: 0.90 + uniform() * 0.30,
                size: 0.70 + uniform() * 0.32,
                pan: (uniform() * 2.0 - 1.0) * 0.75,
            },
            Swell {
                at: 2.45 + uniform() * 0.42,
                rise: 0.22 + uniform() * 0.10,
                decay: 1.05 + uniform() * 0.35,
                size: 0.50 + uniform() * 0.28,
                pan: (uniform() * 2.0 - 1.0) * 0.85,
            },
            Swell {
                at: 3.65 + uniform() * 0.50,
                rise: 0.28 + uniform() * 0.12,
                decay: 1.20 + uniform() * 0.40,
                size: 0.35 + uniform() * 0.22,
                pan: (uniform() * 2.0 - 1.0) * 0.85,
            },
        ];

        // the crack: five shocks in the first third of a second, each longer and lower than the
        // last, leaning left and right in turn
        let shocks: [Shock; 5] = [
            Shock {
                at: 0.0,
                duration: 0.0028,
                size: 4.6,
                pan: -0.15,
            },
            Shock {
                at: 0.026 + uniform() * 0.004,
                duration: 0.0042,
                size: 5.2,
                pan: 0.25,
            },
            Shock {
                at: 0.058 + uniform() * 0.008,
                duration: 0.0060,
                size: 4.8,
                pan: -0.20,
            },
            Shock {
                at: 0.105 + uniform() * 0.012,
                duration: 0.0095,
                size: 6.2,
                pan: 0.30,
            },
            Shock {
                at: 0.265 + uniform() * 0.020,
                duration: 0.0140,
                size: 5.4,
                pan: -0.35,
            },
        ];

        ThunderDecoder {
            rng,
            frame: 0,
            channel: 0,
            near,
            swells,
            shocks,
            low: [[0.0; 4]; 2],
            heave: [[0.0; 4]; 2],
            echo: Box::new([[0.0; 2]; ECHO_FRAMES]),
            echo_low: [0.0; 2],
            dc: [0.0; 2],
            air: [[0.0; 2]; 2],
        }
    }
}

// One swell in the roll: when it peaks and how long it takes to, how fast it dies, all in
// seconds, how loud, and which way it leans, -1 left to 1 right.
#[derive(Clone, Copy)]
struct Swell {
    at: f32,
    rise: f32,
    decay: f32,
    size: f32,
    pan: f32,
}

// One shock in the crack: a single cycle of a sine that dies away as it goes, sharp up then
// down. When it comes and how long it lasts, in seconds, how loud, and which way it leans.
#[derive(Clone, Copy)]
struct Shock {
    at: f32,
    duration: f32,
    size: f32,
    pan: f32,
}

pub struct ThunderDecoder {
    rng: u64,
    frame: u32,
    channel: usize,
    // 0 for the farthest strikes, 1 for the nearest
    near: f32,
    swells: [Swell; 4],
    shocks: [Shock; 5],
    // per ear: the roll's three low-pass stages, then one finding the rumble below ~25 Hz to
    // take away
    low: [[f32; 4]; 2],
    // per ear: two slow wanderings that make the roll heave and tumble, each two low-passes,
    // round 2 Hz and 7 Hz
    heave: [[f32; 4]; 2],
    // what's been heard, for the echoes, and a low-pass dulling them
    echo: Box<[[f32; 2]; ECHO_FRAMES]>,
    echo_low: [f32; 2],
    // per ear: what drifts below ~25 Hz, to take away
    dc: [f32; 2],
    // per ear: two low-passes, the air dulling the sound as it travels
    air: [[f32; 2]; 2],
}

impl Iterator for ThunderDecoder {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        let frame = self.frame as usize;
        let t = self.frame as f32 / SAMPLE_RATE as f32;
        if t >= THUNDER_SECONDS {
            return None;
        }
        let c = self.channel;
        if c == 1 {
            self.frame += 1;
        }
        self.channel = 1 - self.channel;
        let near = self.near;
        let ear = if c == 0 { -1.0 } else { 1.0 };
        let white = next_random(&mut self.rng);

        // the crack, only from nearer strikes
        let crack_amount = near * near;
        let mut crack = 0.0f32;
        if crack_amount > 0.0 && t < 0.6 {
            for shock in &self.shocks {
                let since = t - shock.at;
                if (0.0..shock.duration).contains(&since) {
                    let phase = since / shock.duration;
                    crack += ops::sin(phase * TAU)
                        * (1.0 - phase)
                        * shock.size
                        * (1.0 + 0.25 * shock.pan * ear);
                }
            }
        }
        let crack = crack * crack_amount;

        // how much the roll swells here, and how much of that is a fresh swell cresting
        let (mut swelling, mut cresting) = (0.0f32, 0.0f32);
        for swell in &self.swells {
            let since = t - swell.at + swell.rise;
            if since > 0.0 {
                let u = (since / swell.rise).min(1.0);
                let onset = u * u * (3.0 - 2.0 * u);
                let after = (since - swell.rise).max(0.0);
                let size = swell.size * onset * ops::exp(-after / swell.decay);
                swelling += size * (1.0 + 0.35 * swell.pan * ear);
                cresting += size * ops::exp(-after / 0.35);
            }
        }

        // the roll heaving and tumbling
        let [h1, h2, g1, g2] = &mut self.heave[c];
        *h1 += (white - *h1) * 0.00032;
        *h2 += (*h1 - *h2) * 0.00032;
        *g1 += (white - *g1) * 0.00095;
        *g2 += (*g1 - *g2) * 0.00095;
        let heave = (*h2 * 120.0).clamp(-0.55, 1.45);
        let tumble = (*g2 * 70.0).clamp(-0.50, 1.40);
        let ragged = (0.85 + 0.48 * heave + 0.38 * tumble.max(-0.20)).clamp(0.55, 2.20);

        // the roll: spiky noise through three low-passes, round 150-500 Hz, brighter near, early
        // on and as it heaves or swells, with the lowest rumble taken out
        let crest = (0.35 * heave.max(0.0) + 0.25 * tumble.max(0.0) + 0.50 * cresting).min(1.4);
        let cut = (0.021 + (0.015 + 0.022 * near) * ops::exp(-t / 3.2) + 0.017 * crest)
            .clamp(0.019, 0.072);
        let mut x = (white * 0.60 + white * white * white * 0.75) * ragged;
        let [l1, l2, l3, rumble] = &mut self.low[c];
        for stage in [l1, l2, l3] {
            *stage += (x - *stage) * cut;
            x = *stage;
        }
        *rumble += (x - *rumble) * 0.0035;
        let x = (x - *rumble) * ops::sqrt(0.026 / cut);
        let roll_envelope = {
            // near, it's on top of you at once; far, it fades in, slower at first
            let attack = (t / (0.42 * (1.0 - near).max(0.002))).min(1.0);
            let attack = if near < 0.3 { attack * attack } else { attack };
            let decay = ops::exp(-t / (2.4 + 0.7 * near));
            attack * decay * (0.60 + 0.65 * swelling)
        };
        let roll_gain = ROLL_GAIN.0 * ops::powf(ROLL_GAIN.1 / ROLL_GAIN.0, near);
        let mut sample = x * 17.0 * roll_envelope * roll_gain + crack;

        // two echoes off the land and clouds, dulled, the nearer the louder, each feeding the
        // next
        let tap = |delay: usize| (frame + ECHO_FRAMES - delay) % ECHO_FRAMES;
        let heard =
            self.echo[tap(ECHO_TAPS.0)][1 - c] * 0.58 + self.echo[tap(ECHO_TAPS.1)][c] * 0.42;
        self.echo_low[c] += (heard - self.echo_low[c]) * 0.038;
        let echo = self.echo_low[c];
        self.echo[frame % ECHO_FRAMES][c] = crack * 0.40 + sample * 0.20 + echo * 0.12;
        sample += echo * (0.35 + 0.30 * near);

        self.dc[c] += (sample - self.dc[c]) * 0.0035;
        let sample = (sample - self.dc[c]) * THUNDER_GAIN;
        // fades out at the very end so it doesn't click, and saturates rather than clipping
        let tail = ((THUNDER_SECONDS - t) / 0.5).min(1.0);
        let sample = (sample * tail).tanh();

        // the air dulls it: round 2.5 kHz from the nearest strikes at first, closing to round
        // 270 Hz within half a second
        let cut = (0.038 + (0.020 + 0.240 * near) * ops::exp(-t / 0.14)).clamp(0.036, 0.300);
        let [a1, a2] = &mut self.air[c];
        *a1 += (sample - *a1) * cut;
        *a2 += (*a1 - *a2) * cut;
        Some((*a2 * 1.05).clamp(-1.0, 1.0))
    }
}

impl Source for ThunderDecoder {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> ChannelCount {
        ChannelCount::new(2).unwrap()
    }

    fn sample_rate(&self) -> SampleRate {
        SampleRate::new(SAMPLE_RATE).unwrap()
    }

    fn total_duration(&self) -> Option<Duration> {
        Some(Duration::from_secs_f32(THUNDER_SECONDS))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rms(samples: &[f32]) -> f32 {
        (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
    }

    #[test]
    fn test_thunder_ends_and_near_claps_are_louder() {
        let clap = |distance| -> Vec<f32> { Thunder { distance, seed: 9 }.decoder().collect() };
        let (near, far) = (clap(STRIKE_DISTANCE.0), clap(STRIKE_DISTANCE.1));
        assert_eq!((THUNDER_SECONDS * 44_100.0) as usize * 2, near.len());
        assert!(near.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
        assert!(rms(&near) > rms(&far) * 1.5, "{} {}", rms(&near), rms(&far));
    }

    #[test]
    fn test_near_thunder_opens_with_a_bright_crack() {
        let opening = |distance| -> Vec<f32> {
            Thunder { distance, seed: 9 }
                .decoder()
                .take(44_100 / 20 * 2)
                .collect()
        };
        // how much it changes from one frame to the next, in the left ear: the brighter, the more
        let brightness = |s: &[f32]| {
            rms(&s
                .windows(3)
                .map(|w| w[2] - w[0])
                .step_by(2)
                .collect::<Vec<_>>())
        };
        let (near, far) = (opening(STRIKE_DISTANCE.0), opening(STRIKE_DISTANCE.1));
        assert!(
            rms(&near) > rms(&far) * 10.0,
            "{} {}",
            rms(&near),
            rms(&far)
        );
        assert!(
            brightness(&near) > brightness(&far) * 100.0,
            "{} {}",
            brightness(&near),
            brightness(&far)
        );
        // and it's loud from the very start
        assert!(rms(&near[..88]) > 0.1, "{}", rms(&near[..88]));
    }

    #[test]
    fn test_weather_keeps_the_games_time() {
        // a second of play at the normal pace is a second of weather
        assert_eq!(1.0, weather_seconds(100, 100 + TICKS_PER_SECOND));
        // a tenth of a day wound on is a tenth of a day's weather
        assert_eq!(DAY_SECONDS * 0.1, weather_seconds(0, DAY_TICKS / 10));
        // and winding back holds it
        assert_eq!(0.0, weather_seconds(DAY_TICKS / 2, DAY_TICKS / 5));
    }

    #[test]
    fn test_the_first_storm_comes_clears_and_leaves_a_rainbow() {
        let mut weather = Weather::default();
        let (mut peak, mut bow) = (0.0f32, 0.0f32);
        let dt = 0.1;
        for _ in 0..((FIRST_STORM_IN + FIRST_STORM_SECONDS + 60.0) / dt) as usize {
            weather.advance(dt);
            peak = peak.max(weather.rain);
            bow = bow.max(weather.rainbow);
        }
        assert_eq!(1, weather.storms);
        assert!(peak > 0.7, "{peak}");
        assert!(weather.rain < 0.05, "{}", weather.rain);
        assert!(bow > 0.9, "{bow}");
        // and the rainbow goes again
        for _ in 0..(RAINBOW_SECONDS / dt) as usize {
            weather.advance(dt);
        }
        assert_eq!(0.0, weather.rainbow);
    }
    use crate::sky::{DAY_SECONDS, DAY_TICKS};
}
