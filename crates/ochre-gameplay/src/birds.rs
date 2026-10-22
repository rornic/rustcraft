//! Birds by day: a murmuration of starlings wheeling round the camera, and now and then a
//! skein of geese in a V. They roost at night and shelter from the rain.
use std::f32::consts::TAU;

use bevy::{math::ops, prelude::*};

use crate::wings::{spawn_winged, wing_mesh, Flap, WingPlugin};
use ochre_base::{math::ease, WATER_LEVEL};
use ochre_terrain::world::World;
use ochre_world::{
    sky::{daylight, TimeOfDay},
    weather::Weather,
};

pub struct BirdPlugin;

impl Plugin for BirdPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<WingPlugin>() {
            app.add_plugins(WingPlugin);
        }
        app.init_resource::<Murmuration>()
            .add_systems(Startup, make_bird_assets)
            .add_systems(Update, (gather_flock, flock, fly_geese).chain());
    }
}

// How many starlings are in the flock, how far round the camera it wheels, in blocks, and how
// high over the ground or sea.
const STARLINGS: usize = 160;
const FLOCK_RANGE: f32 = 55.0;
const FLOCK_HEIGHT: f32 = 34.0;
// The starlings' speeds, in blocks a second, and how near they keep with and clear of others.
const SPEED: (f32, f32) = (9.0, 15.0);
const NEIGHBOURS: f32 = 7.0;
const PERSONAL_SPACE: f32 = 3.2;
// How hard each starling steers to keep with its neighbours' heading, toward their middle,
// away from those too near, and toward where the flock's heading.
const ALIGN: f32 = 1.4;
const COHERE: f32 = 0.25;
const SEPARATE: f32 = 9.0;
const HOME: f32 = 0.55;
// How long the flock takes to gather or leave, in seconds.
const FADE_SECONDS: f32 = 3.0;
// How much of the day it is, see `daylight`, that birds are about in.
const BIRD_DAY: f32 = 0.5;

#[derive(Resource)]
struct BirdAssets {
    starling_wing: Handle<Mesh>,
    starling_body: Handle<Mesh>,
    starling: Handle<StandardMaterial>,
    goose_wing: Handle<Mesh>,
    goose_body: Handle<Mesh>,
    goose: Handle<StandardMaterial>,
    goose_wings: Handle<StandardMaterial>,
}

/// The flock, and how far it's gathered, 0..1.
#[derive(Resource, Default)]
struct Murmuration {
    gathered: f32,
    // whether it's the time and weather for birds, or they're leaving
    about: bool,
    // where it's heading round the camera, which wanders on so the flock never settles
    clock: f32,
}

#[derive(Component)]
struct Starling {
    velocity: Vec3,
}

#[derive(Component)]
struct Goose {
    velocity: Vec3,
    travelled: f32,
    phase: f32,
}

fn make_bird_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let matte = |materials: &mut Assets<StandardMaterial>, colour: Color| {
        materials.add(StandardMaterial {
            base_color: colour,
            perceptual_roughness: 0.95,
            double_sided: true,
            cull_mode: None,
            ..default()
        })
    };
    commands.insert_resource(BirdAssets {
        starling_wing: meshes.add(swept_wing(0.42, 0.16)),
        starling_body: meshes.add(Cuboid::new(0.1, 0.08, 0.32)),
        starling: matte(&mut materials, Color::srgb(0.05, 0.05, 0.06)),
        goose_wing: meshes.add(swept_wing(0.95, 0.32)),
        goose_body: meshes.add(Cuboid::new(0.22, 0.2, 0.9)),
        goose: matte(&mut materials, Color::srgb(0.32, 0.28, 0.24)),
        goose_wings: matte(&mut materials, Color::srgb(0.42, 0.38, 0.33)),
    });
}

// A swept-back wing `span` long and `chord` deep at the body.
fn swept_wing(span: f32, chord: f32) -> Mesh {
    wing_mesh([
        [0.0, 0.0, -chord * 0.5],
        [span * 0.55, 0.0, -chord * 0.15],
        [0.0, 0.0, chord * 0.5],
        [span, 0.0, chord * 0.55],
    ])
}

fn birds_about(clock: &TimeOfDay, weather: Option<&Weather>) -> bool {
    daylight(clock) >= BIRD_DAY && weather.is_none_or(|weather| !weather.sheltering())
}

// The flock gathers round the camera by day and leaves at dusk or in the rain.
#[allow(clippy::too_many_arguments)]
fn gather_flock(
    mut commands: Commands,
    time: Res<Time>,
    clock: Res<TimeOfDay>,
    weather: Option<Res<Weather>>,
    assets: Option<Res<BirdAssets>>,
    mut murmuration: ResMut<Murmuration>,
    camera: Query<&GlobalTransform, With<Camera3d>>,
    starlings: Query<Entity, With<Starling>>,
) {
    let (Some(assets), Ok(camera)) = (assets, camera.single()) else {
        return;
    };
    let about = birds_about(&clock, weather.as_deref());
    murmuration.about = about;
    let step = time.delta_secs() / FADE_SECONDS;
    murmuration.gathered =
        (murmuration.gathered + if about { step } else { -step }).clamp(0.0, 1.0);
    let count = starlings.iter().count();
    if about && count == 0 {
        let eye = camera.translation();
        // they arrive together, from somewhere off to one side
        let angle = rand::random_range(0.0..TAU);
        let centre = eye
            + Vec3::new(ops::cos(angle), 0.0, ops::sin(angle)) * FLOCK_RANGE
            + Vec3::Y * FLOCK_HEIGHT;
        let heading = Vec3::new(-ops::sin(angle), 0.0, ops::cos(angle));
        for _ in 0..STARLINGS {
            let at = centre
                + Vec3::new(
                    rand::random_range(-4.0..4.0),
                    rand::random_range(-2.0..2.0),
                    rand::random_range(-4.0..4.0),
                );
            spawn_winged(
                &mut commands,
                Starling {
                    velocity: heading * SPEED.0,
                },
                at,
                (&assets.starling_body, &assets.starling),
                (&assets.starling_wing, &assets.starling),
                // quick beats, now and then gliding with their wings out
                Flap {
                    beat: rand::random_range(16.0..20.0),
                    sweep: 0.8,
                    lift: 0.1,
                    glides: true,
                },
            );
        }
    } else if !about && murmuration.gathered <= 0.0 {
        for starling in &starlings {
            commands.entity(starling).despawn();
        }
    }
}

/// Where the flock heads at `t` seconds, wandering round `eye` far enough off to be seen whole.
pub fn flock_heading(eye: Vec3, t: f32) -> Vec3 {
    let a = t * 0.11 + 0.8 * ops::sin(t * 0.05);
    let radius = FLOCK_RANGE * (0.75 + 0.25 * ops::sin(t * 0.17));
    eye + Vec3::new(ops::cos(a) * radius, 0.0, ops::sin(a) * radius)
}

// Each starling heads with its neighbours, toward their middle and out of their way, while
// all of them steer after the flock's heading. Nothing leads, so it wheels and folds.
fn flock(
    time: Res<Time>,
    world: Res<World>,
    mut murmuration: ResMut<Murmuration>,
    camera: Query<&GlobalTransform, With<Camera3d>>,
    mut starlings: Query<(&mut Starling, &mut Transform)>,
) {
    let Ok(camera) = camera.single() else {
        return;
    };
    let dt = time.delta_secs().min(0.05);
    murmuration.clock += dt;
    let eye = camera.translation();
    let heading = flock_heading(eye, murmuration.clock);
    let ground = world
        .region_store
        .get_height(heading.x.floor() as i64, heading.z.floor() as i64)
        .max(WATER_LEVEL as f64) as f32;
    let mut target = heading
        .with_y(ground.max(eye.y - 20.0) + FLOCK_HEIGHT + 6.0 * ops::sin(murmuration.clock * 0.3));
    // leaving, they make off together, up and away, fading as they go
    if !murmuration.about {
        target = eye + (heading - eye).with_y(0.0).normalize_or(Vec3::X) * 400.0 + Vec3::Y * 120.0;
    }

    let birds: Vec<(Vec3, Vec3)> = starlings
        .iter()
        .map(|(starling, transform)| (transform.translation, starling.velocity))
        .collect();
    let size = ease(murmuration.gathered);
    for (i, (mut starling, mut transform)) in starlings.iter_mut().enumerate() {
        let (here, velocity) = birds[i];
        let steer = steering(i, &birds, target);
        let mut velocity = velocity + steer * dt;
        let speed = velocity.length().clamp(SPEED.0, SPEED.1);
        velocity = velocity.normalize_or(Vec3::X) * speed;
        // never down into the land
        if here.y < ground + 8.0 {
            velocity.y = velocity.y.max(3.0);
        }
        starling.velocity = velocity;
        transform.translation = here + velocity * dt;
        transform.look_to(velocity, Vec3::Y);
        // banking into its turns
        let bank = steer.dot(transform.right().as_vec3()) * 0.04;
        transform.rotate_local_z(-bank.clamp(-0.9, 0.9));
        transform.scale = Vec3::splat(size);
    }
}

/// How hard bird `i` steers, in blocks a second squared, given every bird's place and velocity.
pub fn steering(i: usize, birds: &[(Vec3, Vec3)], target: Vec3) -> Vec3 {
    let (here, velocity) = birds[i];
    let (mut heading, mut middle, mut away, mut count) = (Vec3::ZERO, Vec3::ZERO, Vec3::ZERO, 0.0);
    for (j, &(there, their_velocity)) in birds.iter().enumerate() {
        if j == i {
            continue;
        }
        let between = there - here;
        let distance_squared = between.length_squared();
        if distance_squared < NEIGHBOURS * NEIGHBOURS {
            heading += their_velocity;
            middle += there;
            count += 1.0;
            if distance_squared < PERSONAL_SPACE * PERSONAL_SPACE {
                away -= between / distance_squared.max(0.01);
            }
        }
    }
    let mut steer = (target - here).normalize_or_zero() * SPEED.1 * HOME;
    if count > 0.0 {
        steer += (heading / count - velocity) * ALIGN;
        steer += (middle / count - here) * COHERE;
    }
    steer + away * SEPARATE
}

// How often a skein of geese flies over, in seconds, and how high, fast and far, in blocks.
const SKEIN_SECONDS: (f32, f32) = (45.0, 110.0);
const FIRST_SKEIN_IN: f32 = 12.0;
const SKEIN_HEIGHT: f32 = 48.0;
const GOOSE_SPEED: f32 = 11.0;
const SKEIN_REACH: f32 = 230.0;

#[allow(clippy::too_many_arguments)]
fn fly_geese(
    mut commands: Commands,
    time: Res<Time>,
    clock: Res<TimeOfDay>,
    weather: Option<Res<Weather>>,
    assets: Option<Res<BirdAssets>>,
    camera: Query<&GlobalTransform, With<Camera3d>>,
    mut geese: Query<(Entity, &mut Goose, &mut Transform)>,
    mut next: Local<Option<f32>>,
) {
    let (Some(assets), Ok(camera)) = (assets, camera.single()) else {
        return;
    };
    let dt = time.delta_secs();
    let t = time.elapsed_secs();
    for (entity, mut goose, mut transform) in &mut geese {
        goose.travelled += GOOSE_SPEED * dt;
        transform.translation += goose.velocity * dt;
        // a gentle rise and fall, each in its own time
        transform.translation.y += ops::sin(t * 1.1 + goose.phase) * 0.15 * dt;
        let grown = (goose.travelled / 20.0).min(1.0);
        let gone = ((2.0 * SKEIN_REACH - goose.travelled) / 20.0).clamp(0.0, 1.0);
        transform.scale = Vec3::splat(grown.min(gone));
        transform.look_to(goose.velocity, Vec3::Y);
        if goose.travelled > 2.0 * SKEIN_REACH {
            commands.entity(entity).despawn();
        }
    }

    let due = next.get_or_insert(t + FIRST_SKEIN_IN);
    if t < *due {
        return;
    }
    *due = t + rand::random_range(SKEIN_SECONDS.0..SKEIN_SECONDS.1);
    if !birds_about(&clock, weather.as_deref()) {
        return;
    }
    // across the camera's view, passing a little to one side of it
    let eye = camera.translation();
    let ahead = camera.forward().with_y(0.0).normalize_or(Vec3::Z);
    let across =
        Vec3::new(-ahead.z, 0.0, ahead.x) * if rand::random::<bool>() { 1.0 } else { -1.0 };
    let course = (across + ahead * rand::random_range(-0.4..0.4)).normalize();
    let passing = eye + ahead * rand::random_range(40.0..90.0) + Vec3::Y * SKEIN_HEIGHT;
    let start = passing - course * SKEIN_REACH;
    let side = Vec3::new(-course.z, 0.0, course.x);
    let count: u32 = rand::random_range(7..12);
    for k in 0..count {
        // the leader at the point, the rest down either arm of the V
        let rank = k.div_ceil(2) as f32;
        let arm = if k % 2 == 0 { 1.0 } else { -1.0 };
        let at = start - course * rank * 2.4 + side * arm * rank * 2.0 + Vec3::Y * rank * 0.3;
        spawn_winged(
            &mut commands,
            Goose {
                velocity: course * GOOSE_SPEED,
                travelled: 0.0,
                phase: rand::random_range(0.0..TAU),
            },
            at,
            (&assets.goose_body, &assets.goose),
            (&assets.goose_wing, &assets.goose_wings),
            // slow, deep strokes
            Flap {
                beat: rand::random_range(5.5..6.5),
                sweep: 0.55,
                lift: 0.1,
                glides: true,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_crowded_birds_push_apart_and_lone_ones_head_home() {
        let target = Vec3::new(100.0, 0.0, 0.0);
        let lone = [(Vec3::ZERO, Vec3::X * 10.0)];
        assert!(steering(0, &lone, target).x > 0.0);
        let crowded = [
            (Vec3::ZERO, Vec3::Z * 10.0),
            (Vec3::new(0.0, 0.0, 0.5), Vec3::Z * 10.0),
        ];
        // the one behind is pushed back, away from the one just ahead
        assert!(steering(0, &crowded, Vec3::ZERO).z < 0.0);
    }

    #[test]
    fn test_neighbours_turn_a_bird_to_their_heading() {
        let mut birds = vec![(Vec3::ZERO, Vec3::X * 10.0)];
        for k in 0..6 {
            birds.push((Vec3::new(k as f32 * 0.4 - 1.0, 2.0, 0.0), Vec3::Z * 10.0));
        }
        let steer = steering(0, &birds, Vec3::ZERO);
        assert!(steer.z > 0.0 && steer.x < 0.0, "{steer}");
    }
}
