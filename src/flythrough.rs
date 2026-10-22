//! RC_FLYTHROUGH=<dir> flies a scripted path, laid out on seed 2665537132, on a fixed 60 fps clock and saves every frame there.
use std::time::{Duration, Instant};

use bevy::{
    app::AppExit,
    prelude::*,
    render::view::screenshot::{save_to_disk, Screenshot},
    time::TimeUpdateStrategy,
    transform::TransformSystems,
};

use crate::player::{Player, EYE_HEIGHT};
use crate::world::World;

const FPS: u32 = 60;
// Real seconds the camera waits at the start for the terrain round it to load.
const WARMUP: f32 = 5.0;
// Blocks the camera keeps over the ground, water and trees, how fast it may rise and sink back, and how far ahead it looks.
const CLEARANCE: f32 = 4.0;
const RISE: f32 = 12.0;
const SINK: f32 = 6.0;
const AHEAD: [f32; 7] = [0.0, 0.25, 0.5, 0.75, 1.0, 1.25, 1.5];

// (seconds, camera position, where it looks)
const KEYS: [(f32, [f32; 3], [f32; 3]); 14] = [
    (0.0, [9330.0, 512.0, -2000.0], [9420.0, 506.0, -2110.0]),
    (4.0, [9420.0, 510.0, -2120.0], [9550.0, 503.0, -2350.0]),
    // low across the lake to the spring on the far shore, then slow over its meadow
    (16.0, [10000.0, 514.0, -3240.0], [10061.0, 506.0, -3304.0]),
    (25.0, [10050.0, 513.0, -3290.0], [10085.0, 505.0, -3335.0]),
    // up and away west to the mountain over the lake
    (29.0, [9700.0, 640.0, -3150.0], [7225.0, 880.0, -2900.0]),
    (39.0, [7420.0, 935.0, -2920.0], [6900.0, 880.0, -2880.0]),
    // on the summit, looking round from the west by the north to the lake in the east
    (42.0, [7235.0, 912.0, -2900.0], [6625.0, 860.0, -2900.0]),
    (45.0, [7230.0, 914.0, -2900.0], [7225.0, 860.0, -3500.0]),
    (48.0, [7225.0, 916.0, -2900.0], [7825.0, 820.0, -2900.0]),
    // straight up, tilting down over the whole world
    (53.0, [7300.0, 2400.0, -2920.0], [8600.0, 500.0, -2950.0]),
    (58.0, [7400.0, 5000.0, -2950.0], [8200.0, 450.0, -2960.0]),
    // and a look round from up there, east by the north to the west
    (61.0, [7400.0, 5000.0, -2950.0], [11400.0, 500.0, -2950.0]),
    (63.5, [7400.0, 5000.0, -2950.0], [7400.0, 500.0, -6950.0]),
    (66.0, [7400.0, 5000.0, -2950.0], [3400.0, 500.0, -2950.0]),
];

pub struct FlythroughPlugin;

impl Plugin for FlythroughPlugin {
    fn build(&self, app: &mut App) {
        if let Ok(dir) = std::env::var("RC_FLYTHROUGH") {
            std::fs::create_dir_all(&dir).expect("flythrough frame directory");
            app.insert_resource(Flythrough { dir, started: Instant::now(), frame: None, lift: 0.0 })
                .add_systems(PostUpdate, fly.before(TransformSystems::Propagate));
        }
    }
}

#[derive(Resource)]
struct Flythrough {
    dir: String,
    started: Instant,
    // the frame being recorded, once warmed up
    frame: Option<u32>,
    lift: f32,
}

// The path at `t` seconds: a Hermite curve through the keys, with monotone tangents so it never overshoots a key and swings back.
fn path(t: f32) -> (Vec3, Vec3) {
    let at = |i: usize| (KEYS[i].0, Vec3::from(KEYS[i].1), Vec3::from(KEYS[i].2));
    let last = KEYS.len() - 1;
    let i = (0..last).find(|&i| t < KEYS[i + 1].0).unwrap_or(last - 1);
    // per axis, the harmonic mean of the speeds either side, or still where the path turns back or ends
    let tangent = |i: usize, value: &dyn Fn(usize) -> Vec3| {
        if i == 0 || i == last {
            return Vec3::ZERO;
        }
        let before = (value(i) - value(i - 1)) / (KEYS[i].0 - KEYS[i - 1].0);
        let after = (value(i + 1) - value(i)) / (KEYS[i + 1].0 - KEYS[i].0);
        Vec3::from_array(std::array::from_fn(|k| {
            let (a, b) = (before[k], after[k]);
            if a * b <= 0.0 { 0.0 } else { 2.0 * a * b / (a + b) }
        }))
    };
    let (position, look) = (|i: usize| at(i).1, |i: usize| at(i).2);
    let ((t0, p0, q0), (t1, p1, q1)) = (at(i), at(i + 1));
    let (m0, m1) = (tangent(i, &position), tangent(i + 1, &position));
    let (n0, n1) = (tangent(i, &look), tangent(i + 1, &look));
    let span = t1 - t0;
    let s = ((t - t0) / span).clamp(0.0, 1.0);
    let (s2, s3) = (s * s, s * s * s);
    let (h00, h10, h01, h11) = (2.0 * s3 - 3.0 * s2 + 1.0, s3 - 2.0 * s2 + s, -2.0 * s3 + 3.0 * s2, s3 - s2);
    let hermite = |a: Vec3, ma: Vec3, b: Vec3, mb: Vec3| a * h00 + ma * span * h10 + b * h01 + mb * span * h11;
    (hermite(p0, m0, p1, m1), hermite(q0, n0, q1, n1))
}

fn fly(
    mut commands: Commands,
    mut state: ResMut<Flythrough>,
    world: Res<World>,
    mut exit: MessageWriter<AppExit>,
    mut player: Query<&mut Transform, With<Player>>,
    mut camera: Query<&mut Transform, (With<Camera3d>, Without<Player>)>,
) {
    let (Ok(mut body), Ok(mut eye)) = (player.single_mut(), camera.single_mut()) else {
        return;
    };
    let frame = match state.frame {
        Some(frame) => frame,
        None if state.started.elapsed().as_secs_f32() < WARMUP => 0,
        None => {
            commands.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(1.0 / FPS as f64)));
            state.frame = Some(0);
            0
        }
    };
    let t = frame as f32 / FPS as f32;
    if t > KEYS[KEYS.len() - 1].0 {
        exit.write(AppExit::Success);
        return;
    }
    let (mut at, look) = path(t);
    // kept clear of what's coming up along the path, rising early and smoothly over it and easing back down after
    let wanted = AHEAD
        .iter()
        .map(|&ahead| {
            let p = path(t + ahead).0;
            clear_height(&world, p) - p.y
        })
        .fold(0.0, f32::max);
    let step = (wanted - state.lift).clamp(-SINK / FPS as f32, RISE / FPS as f32);
    state.lift += step;
    // and never under the ground or a tree, even for a frame the smooth lift hasn't caught up
    at.y = (at.y + state.lift).max(clear_height(&world, at) - CLEARANCE + 1.5);
    let facing = (look - at).normalize_or_zero();
    body.translation = at - Vec3::Y * EYE_HEIGHT;
    body.rotation = Quat::from_rotation_y(f32::atan2(-facing.x, -facing.z));
    eye.translation = Vec3::Y * EYE_HEIGHT;
    eye.rotation = Quat::from_rotation_x(facing.y.clamp(-1.0, 1.0).asin());
    if let Some(frame) = state.frame {
        let file = format!("{}/frame_{frame:05}.png", state.dir);
        commands.spawn(Screenshot::primary_window()).observe(save_to_disk(file));
        state.frame = Some(frame + 1);
    }
}

// How high the camera must be over `p`'s column: over the ground, water, and any solid block round it, trees included.
fn clear_height(world: &World, p: Vec3) -> f32 {
    let sample = world.region_store.terrain().sample(p.x as f64, p.z as f64);
    let mut floor = (sample.height as f32).max(sample.water_surface);
    for dx in -1..=1 {
        for dz in -1..=1 {
            let (x, z) = (p.x.round() as i64 + dx, p.z.round() as i64 + dz);
            let top = (p.y.round() as i64 - 4..=p.y.round() as i64 + 16)
                .rev()
                .find(|&y| world.get_block_at(bevy::math::I64Vec3::new(x, y, z)).is_solid());
            if let Some(y) = top {
                floor = floor.max(y as f32 + 0.5);
            }
        }
    }
    floor + CLEARANCE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_path_passes_through_each_key_and_never_overshoots_between() {
        for (i, &(t, at, look)) in KEYS.iter().enumerate() {
            let (p, q) = path(t);
            assert!(p.distance(Vec3::from(at)) < 1e-2 && q.distance(Vec3::from(look)) < 1e-2, "key {i}");
        }
        for pair in KEYS.windows(2) {
            let ((t0, a0, _), (t1, a1, _)) = (pair[0], pair[1]);
            for step in 0..=50 {
                let p = path(t0 + (t1 - t0) * step as f32 / 50.0).0;
                for k in 0..3 {
                    let (lo, hi) = (a0[k].min(a1[k]), a0[k].max(a1[k]));
                    assert!((lo - 1e-2..=hi + 1e-2).contains(&p[k]), "overshoot between {t0} and {t1}");
                }
            }
        }
    }
}
