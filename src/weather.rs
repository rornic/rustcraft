//! Storms now and then: darker skies and clouds, rain and its sound, a stronger wind, and a
//! rainbow once the rain clears in sunshine.
use std::{
    f32::consts::{PI, TAU},
    time::Duration,
};

use bevy::{
    asset::RenderAssetUsages,
    audio::{AddAudioSource, ChannelCount, Decodable, SampleRate, Source},
    camera::visibility::NoFrustumCulling,
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
    sky::{moon_illuminance, sun_direction, sun_illuminance, Moon, Sun, TimeOfDay},
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
        .init_resource::<Weather>()
        .add_systems(Startup, spawn_weather)
        .add_systems(
            Update,
            (run_weather, (darken_sky, fall_rain, show_rainbow)).chain(),
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

fn run_weather(time: Res<Time>, mut weather: ResMut<Weather>) {
    weather.advance(time.delta_secs());
}

// How much of the sun's and moon's light gets through the storm's clouds, and how dark the
// clouds themselves get, at the heaviest rain.
const STORM_LIGHT: f32 = 0.3;
const STORM_CLOUD: f32 = 0.42;
#[allow(clippy::type_complexity)]
fn darken_sky(
    weather: Res<Weather>,
    mut lights: Query<(&mut DirectionalLight, Has<Sun>), Or<(With<Sun>, With<Moon>)>>,
    mut clouds: ResMut<Assets<CloudMaterial>>,
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
    commands.insert_resource(WeatherHandles { rain, rainbow });
    commands.spawn((
        AudioPlayer(sounds.add(RainSound {
            level: weather.sound.clone(),
        })),
        PlaybackSettings::LOOP,
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
    ((sun_direction(clock.0).y + 0.1) / 0.4).clamp(0.05, 1.0)
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
    let away = -sun_direction(clock.0);
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
