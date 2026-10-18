//! What the world sounds like where the camera is: wind, and loops of forest, crickets, waves
//! and so on, each as loud as the camera's surroundings call for, and the odd owl.
mod wind;

use std::f32::consts::TAU;

use bevy::{
    app::{App, Plugin, Startup, Update},
    asset::{AssetServer, Handle},
    audio::{
        AddAudioSource, AudioPlayer, AudioSink, AudioSinkPlayback, AudioSource, PlaybackSettings,
        SpatialListener, SpatialScale, Volume,
    },
    camera::Camera3d,
    ecs::{
        component::Component,
        entity::Entity,
        query::{With, Without},
        resource::Resource,
        schedule::IntoScheduleConfigs,
        system::{Commands, Local, Query, Res, ResMut},
    },
    math::{ops, Vec3},
    time::Time,
    transform::components::{GlobalTransform, Transform},
};

pub(crate) use crate::math::smoothstep;
use crate::{
    block::BlockType,
    chunks::generate::WATER_LEVEL,
    sky::{sun_direction, TimeOfDay},
    world::World,
};
use wind::Gust;
pub use wind::{wind_strength, Wind, WindDials};
pub(crate) use wind::{Dial, SAMPLE_RATE};

pub struct AmbiencePlugin;

impl Plugin for AmbiencePlugin {
    fn build(&self, app: &mut App) {
        app.add_audio_source::<Wind>()
            .init_resource::<WindDials>()
            .init_resource::<Surroundings>()
            .add_systems(Startup, (wind::play_wind, play_loops))
            .add_systems(
                Update,
                (
                    listen_from_camera,
                    sense_surroundings,
                    (wind::update_wind, mix_loops, hoot),
                )
                    .chain(),
            );
    }
}

/// What's round the camera, as far as what it hears goes. The fractions are 0..1.
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct Surroundings {
    // how high the camera is over the ground or sea, in blocks
    pub altitude: f32,
    // how fast it's moving, in blocks a second
    pub speed: f32,
    pub underwater: bool,
    // how much of what's round it is sea; the rest is land
    pub water: f32,
    // how thick the forest round it is, and right round it, under the trees' crowns
    pub trees: f32,
    pub trees_overhead: f32,
    // 0 where it's cold enough to freeze, up to 1 where it's mild
    pub warmth: f32,
    // 1 over desert
    pub desert: f32,
    // 1 by day, 0 by night, in between at dawn and dusk
    pub day: f32,
    // how much of a breeze is blowing, see `WindDials`
    pub breeze: f32,
}

impl Default for Surroundings {
    fn default() -> Self {
        Self {
            altitude: 0.0,
            speed: 0.0,
            underwater: false,
            water: 0.0,
            trees: 0.0,
            trees_overhead: 0.0,
            warmth: 1.0,
            desert: 0.0,
            day: 1.0,
            breeze: 0.0,
        }
    }
}

// How often the terrain round the camera is sampled again, in seconds. The loops glide to
// their new volumes over seconds anyway.
const SENSE_SECONDS: f32 = 0.25;
// The rings of columns sampled round the camera: their radius in blocks, and how much each
// column on them counts. The camera's own column counts as much as the inner ring's.
const RINGS: [(f32, f32); 2] = [(24.0, 2.0), (64.0, 1.0)];
const RING_COLUMNS: usize = 8;
const CENTRE_WEIGHT: f32 = 2.0;
// The tree density, see `Biome::tree_density`, that sounds like the depths of a forest.
const DENSE_FOREST: f64 = 0.4;
// The temperatures, see `NoiseGenerator::temperature`, between which the cold sets in.
const FREEZING: (f32, f32) = (0.15, 0.35);

#[allow(clippy::too_many_arguments)]
fn sense_surroundings(
    time: Res<Time>,
    world: Res<World>,
    clock: Res<TimeOfDay>,
    dials: Res<WindDials>,
    camera: Query<&GlobalTransform, With<Camera3d>>,
    mut around: ResMut<Surroundings>,
    mut since: Local<f32>,
    mut last: Local<Option<Vec3>>,
) {
    let Ok(camera) = camera.single() else {
        return;
    };
    *since += time.delta_secs();
    if last.is_some() && *since < SENSE_SECONDS {
        return;
    }
    let eye = camera.translation();
    let speed = match last.replace(eye) {
        Some(last) if *since > 0.0 => last.distance(eye) / *since,
        _ => 0.0,
    };
    *since = 0.0;

    let store = &world.region_store;
    let noise = store.noise();
    let ground = store
        .get_height(eye.x.floor() as i64, eye.z.floor() as i64)
        .max(WATER_LEVEL as f64) as f32;
    let (mut water, mut trees, mut total) = (0.0, 0.0, 0.0);
    let (mut overhead, mut overhead_total) = (0.0, 0.0);
    // the camera's own column and the inner ring count as overhead
    let mut sample = |x: f32, z: f32, weight: f32, is_overhead: bool| {
        let column = noise.sample(x as f64, z as f64);
        let mut cover = 0.0;
        if column.height < WATER_LEVEL as f64 {
            water += weight;
        } else {
            cover = (column.trees / DENSE_FOREST).min(1.0) as f32;
            trees += weight * cover;
        }
        total += weight;
        if is_overhead {
            overhead += weight * cover;
            overhead_total += weight;
        }
    };
    sample(eye.x, eye.z, CENTRE_WEIGHT, true);
    for (ring, (radius, weight)) in RINGS.into_iter().enumerate() {
        for i in 0..RING_COLUMNS {
            let (sin, cos) = ops::sin_cos(i as f32 / RING_COLUMNS as f32 * TAU);
            sample(
                eye.x + cos * radius,
                eye.z + sin * radius,
                weight,
                ring == 0,
            );
        }
    }
    let here = noise.sample(eye.x as f64, eye.z as f64);
    let temperature = noise.temperature(eye.x as f64, eye.z as f64);

    *around = Surroundings {
        altitude: eye.y - ground,
        speed,
        underwater: world.get_block_at(eye.floor().as_i64vec3()) == BlockType::Water,
        water: water / total,
        trees: trees / total,
        trees_overhead: overhead / overhead_total,
        warmth: smoothstep(FREEZING.0, FREEZING.1, temperature),
        desert: (noise.biomes().name(here.biome) == Some("desert")) as u8 as f32,
        day: smoothstep(-0.05, 0.2, sun_direction(clock.days()).y),
        breeze: dials.breeze(),
    };
}

/// A recorded loop, and how loud it plays at most, as the files are all levelled alike.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Loop {
    Forest,
    Crickets,
    Waves,
    Desert,
    Underwater,
    Leaves,
}

impl Loop {
    const ALL: [Loop; 6] = [
        Loop::Forest,
        Loop::Crickets,
        Loop::Waves,
        Loop::Desert,
        Loop::Underwater,
        Loop::Leaves,
    ];

    fn file(self) -> &'static str {
        match self {
            Loop::Forest => "sounds/forest.ogg",
            Loop::Crickets => "sounds/crickets.ogg",
            Loop::Waves => "sounds/waves.ogg",
            Loop::Desert => "sounds/desert.ogg",
            Loop::Underwater => "sounds/underwater.ogg",
            Loop::Leaves => "sounds/leaves.ogg",
        }
    }

    // Each file's measured loudness and the loudness it plays at in full, in LUFS: the sea and
    // birds up front, crickets and the desert behind, the ground wind quieter still.
    fn loudness(self) -> (f32, f32) {
        match self {
            Loop::Forest => (-19.2, -24.0),
            Loop::Crickets => (-14.0, -29.0),
            Loop::Waves => (-19.7, -24.0),
            Loop::Desert => (-24.5, -31.0),
            Loop::Underwater => (-18.4, -22.0),
            Loop::Leaves => (-20.2, -31.0),
        }
    }

    fn gain(self) -> f32 {
        let (measured, wanted) = self.loudness();
        10f32.powf((wanted - measured) / 20.0)
    }

    /// Whether this loop comes and goes on its own, as birds and crickets do, rather than
    /// playing on as long as its surroundings call for it.
    fn coming_and_going(self) -> Option<Bursts> {
        match self {
            // birds strike up now and then, and fall quiet more than they sing
            Loop::Forest => Some(Bursts {
                seconds: (3.0, 12.0),
                quiet: (0.5, 0.85),
            }),
            // a cricket or two starts up, chirps a while and stops
            Loop::Crickets => Some(Bursts {
                seconds: (2.0, 8.0),
                quiet: (0.6, 0.9),
            }),
            Loop::Desert => Some(Bursts {
                seconds: (12.0, 40.0),
                quiet: (0.35, 0.75),
            }),
            // the leaves come and go with the wind's breezes instead, see `Loop::level`
            Loop::Waves | Loop::Underwater | Loop::Leaves => None,
        }
    }

    /// How loud this loop should be in these surroundings, 0..1, before its gain.
    pub fn level(self, around: &Surroundings) -> f32 {
        if around.underwater {
            return (self == Loop::Underwater) as u8 as f32;
        }
        // what's on the ground fades as the camera climbs away from it
        let near = |from, to| 1.0 - smoothstep(from, to, around.altitude);
        let night = 1.0 - around.day;
        match self {
            Loop::Forest => around.trees * around.day * around.warmth * near(15.0, 90.0),
            Loop::Crickets => {
                (1.0 - around.water)
                    * (1.0 - around.desert)
                    * night
                    * around.warmth
                    * near(15.0, 90.0)
            }
            // frozen sea doesn't break on the shore
            Loop::Waves => around.water * around.warmth * near(10.0, 120.0),
            Loop::Desert => around.desert * near(30.0, 150.0),
            Loop::Underwater => 0.0,
            // only under the trees' crowns, and only while a breeze stirs them
            Loop::Leaves => around.trees_overhead * around.breeze * near(8.0, 40.0),
        }
    }
}

/// How a loop comes and goes: it follows a value wandering 0..1, see `Gust`.
struct Bursts {
    // how long the value takes to wander to each new target, in seconds, at the least and most
    seconds: (f32, f32),
    // where the value goes from silence (under the first) to full (over the second): the
    // higher they are, the more of the time it's silent
    quiet: (f32, f32),
}

/// Where a loop that comes and goes is in its comings and goings, see `Loop::coming_and_going`.
#[derive(Component, Default)]
struct Activity(Gust);

// How long the loops take to follow the surroundings, in seconds.
const LOOP_FOLLOW_SECONDS: f32 = 1.5;

fn play_loops(mut commands: Commands, assets: Res<AssetServer>) {
    commands.insert_resource(Owl(assets.load("sounds/owl.ogg")));
    for kind in Loop::ALL {
        commands.spawn((
            kind,
            Activity::default(),
            AudioPlayer::<AudioSource>(assets.load(kind.file())),
            PlaybackSettings::LOOP.with_volume(Volume::SILENT),
        ));
    }
}

fn mix_loops(
    time: Res<Time>,
    around: Res<Surroundings>,
    weather: Option<Res<crate::weather::Weather>>,
    mut loops: Query<(&Loop, &mut Activity, &mut AudioSink)>,
) {
    // birds and crickets fall silent in the rain
    let rain = weather.map_or(0.0, |weather| weather.rain);
    let follow = 1.0 - (-time.delta_secs() / LOOP_FOLLOW_SECONDS).exp();
    for (kind, mut activity, mut sink) in &mut loops {
        let active = kind.coming_and_going().map_or(1.0, |bursts| {
            let (a, b) = (rand::random(), rand::random());
            let wander = activity.0.advance(time.delta_secs(), bursts.seconds, a, b);
            smoothstep(bursts.quiet.0, bursts.quiet.1, wander)
        });
        let current = sink.volume().to_linear();
        // square-rooted, as loudness goes by power: a wood a third as thick as the deepest
        // forest sounds about 5 dB quieter, not 10
        let sheltering = match kind {
            Loop::Forest | Loop::Crickets => 1.0 - rain,
            _ => 1.0,
        };
        let wanted = kind.level(&around).sqrt() * kind.gain() * active * sheltering;
        let volume = current + (wanted - current) * follow;
        sink.set_volume(Volume::Linear(volume));
        // a loop no one can hear is paused, so it isn't decoded for nothing
        let silent = volume < SILENT && wanted < SILENT;
        if silent && !sink.is_paused() {
            sink.pause();
        } else if !silent && sink.is_paused() {
            sink.play();
        }
    }
}

// How quiet a loop is, as a linear volume, before it's paused, see `mix_loops`.
const SILENT: f32 = 1e-3;

/// Hears from the camera, for sounds placed in the world, see `hoot`.
fn listen_from_camera(
    mut commands: Commands,
    cameras: Query<Entity, (With<Camera3d>, Without<SpatialListener>)>,
) {
    for camera in &cameras {
        commands.entity(camera).insert(SpatialListener::default());
    }
}

// How long between owls' hoots in a forest at night, in seconds, at the least and most.
const HOOT_SECONDS: (f32, f32) = (12.0, 40.0);
// How far off an owl hoots, in blocks, at the nearest and farthest.
const OWL_DISTANCE: (f32, f32) = (20.0, 45.0);
// Shrinks blocks for the stereo falloff, which goes by the inverse of distance: a block is
// this far, so owls a few dozen blocks off are faint but clear.
const OWL_SCALE: f32 = 0.06;
// Brings a hoot a few dozen blocks off to about the crickets' loudness.
const OWL_VOLUME: f32 = 0.2;

#[derive(Resource)]
struct Owl(Handle<AudioSource>);

fn hoot(
    mut commands: Commands,
    time: Res<Time>,
    around: Res<Surroundings>,
    camera: Query<&GlobalTransform, With<Camera3d>>,
    owl: Res<Owl>,
    mut wait: Local<Option<f32>>,
) {
    let chance = Loop::Forest.level(&Surroundings {
        day: 1.0 - around.day,
        ..*around
    });
    let left = wait.get_or_insert_with(|| rand::random_range(HOOT_SECONDS.0..HOOT_SECONDS.1));
    // owls only call where the forest's loud enough at night, and wait longer in thin woods
    *left -= time.delta_secs() * chance;
    if *left > 0.0 || chance < 0.25 {
        return;
    }
    *wait = None;
    let Ok(camera) = camera.single() else {
        return;
    };
    let angle = rand::random_range(0.0..TAU);
    let distance = rand::random_range(OWL_DISTANCE.0..OWL_DISTANCE.1);
    let offset = Vec3::new(ops::cos(angle), 0.0, ops::sin(angle)) * distance
        + Vec3::Y * rand::random_range(4.0..12.0);
    commands.spawn((
        AudioPlayer(owl.0.clone()),
        PlaybackSettings {
            spatial: true,
            spatial_scale: Some(SpatialScale::new(OWL_SCALE)),
            volume: Volume::Linear(OWL_VOLUME),
            speed: rand::random_range(0.95..1.05),
            ..PlaybackSettings::DESPAWN
        },
        Transform::from_translation(camera.translation() + offset),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn levels(around: Surroundings) -> Vec<(Loop, f32)> {
        Loop::ALL
            .into_iter()
            .map(|kind| (kind, kind.level(&around)))
            .filter(|(_, level)| *level > 0.01)
            .collect()
    }

    #[test]
    fn test_forest_by_day_and_crickets_by_night() {
        let forest = Surroundings {
            trees: 0.9,
            ..Default::default()
        };
        assert_eq!(
            vec![Loop::Forest],
            levels(forest).into_iter().map(|l| l.0).collect::<Vec<_>>()
        );
        let night = Surroundings { day: 0.0, ..forest };
        assert_eq!(
            vec![Loop::Crickets],
            levels(night).into_iter().map(|l| l.0).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_leaves_rustle_only_under_trees_in_a_breeze() {
        let still = Surroundings {
            trees: 0.9,
            trees_overhead: 0.9,
            ..Default::default()
        };
        assert_eq!(0.0, Loop::Leaves.level(&still));
        let breeze = Surroundings {
            breeze: 1.0,
            ..still
        };
        assert!(Loop::Leaves.level(&breeze) > 0.8);
        // forest down the slope doesn't rustle over a bare peak
        let peak = Surroundings {
            trees_overhead: 0.0,
            ..breeze
        };
        assert_eq!(0.0, Loop::Leaves.level(&peak));
    }

    #[test]
    fn test_underwater_hears_only_the_water() {
        let around = Surroundings {
            underwater: true,
            water: 1.0,
            trees: 0.5,
            ..Default::default()
        };
        assert_eq!(vec![(Loop::Underwater, 1.0)], levels(around));
    }

    #[test]
    fn test_the_ground_falls_quiet_high_up_and_the_sea_freezes_silent() {
        let coast = Surroundings {
            water: 0.5,
            trees: 0.5,
            ..Default::default()
        };
        assert!(Loop::Waves.level(&coast) > 0.4);
        let high = Surroundings {
            altitude: 200.0,
            ..coast
        };
        assert!(levels(high).is_empty());
        let frozen = Surroundings {
            warmth: 0.0,
            ..coast
        };
        assert_eq!(0.0, Loop::Waves.level(&frozen));
    }
}
