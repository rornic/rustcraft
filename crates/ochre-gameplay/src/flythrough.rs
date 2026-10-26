//! OCHRE_FLYTHROUGH=<dir> flies a scripted path, laid out on seed 2665537132, on a fixed 60 fps clock and saves every frame there.
use std::time::{Duration, Instant};

use bevy::{
    app::AppExit,
    prelude::*,
    render::view::screenshot::{save_to_disk, Screenshot},
    time::TimeUpdateStrategy,
    transform::TransformSystems,
};

use crate::player::{Player, EYE_HEIGHT};
use ochre_terrain::world::World;

const FPS: u32 = 60;
// Real seconds, from the player spawning, the camera waits at the start for the terrain round it to load.
const WARMUP: f32 = 5.0;
// Blocks the camera keeps over the ground and water, enough to clear trees, and over any block right round it.
const CLEARANCE: f32 = 24.0;
const BLOCK_CLEARANCE: f32 = 4.0;
// Seconds either side the lift over the ground looks for the highest it needs, and eases over, so it rises and settles in long curves.
const LIFT_REACH: f32 = 2.5;

// (seconds, camera position, where it looks)
const KEYS: [(f32, [f32; 3], [f32; 3]); 22] = [
    // high over the world, looking out to the horizon away from the sun, then down and round from the east by the south to the west
    (0.0, [7520.0, 6000.0, -2800.0], [7520.0, 5600.0, -6800.0]),
    (3.0, [7520.0, 6000.0, -2800.0], [7520.0, 5600.0, -6800.0]),
    (6.0, [7520.0, 6000.0, -2800.0], [11520.0, 500.0, -2800.0]),
    (9.0, [7520.0, 6000.0, -2800.0], [7520.0, 500.0, 1200.0]),
    (12.0, [7520.0, 6000.0, -2800.0], [3520.0, 500.0, -2800.0]),
    // down through the clouds to the summit, always looking north past it, then round by the east and south to the lake in the west
    (17.0, [7520.0, 2600.0, -3600.0], [7520.0, 830.0, -5280.0]),
    (22.0, [7520.0, 925.0, -4680.0], [7520.0, 830.0, -5280.0]),
    (25.0, [7520.0, 925.0, -4680.0], [8120.0, 830.0, -4680.0]),
    (28.0, [7520.0, 925.0, -4680.0], [7520.0, 830.0, -4080.0]),
    (31.0, [7520.0, 925.0, -4680.0], [6900.0, 650.0, -4900.0]),
    // down the ridge's west flank and out over the lake, north along it to where the stream leaves
    (36.0, [6700.0, 700.0, -5100.0], [5800.0, 540.0, -5500.0]),
    (41.0, [6100.0, 560.0, -5300.0], [5400.0, 520.0, -5700.0]),
    (47.0, [5550.0, 545.0, -6000.0], [5400.0, 520.0, -6600.0]),
    (53.0, [5380.0, 545.0, -6900.0], [5330.0, 520.0, -7500.0]),
    // down the stream to the sea
    (57.0, [5312.0, 540.0, -7376.0], [5360.0, 510.0, -7520.0]),
    (60.5, [5400.0, 525.0, -7560.0], [5560.0, 500.0, -7690.0]),
    (64.0, [5600.0, 515.0, -7720.0], [5700.0, 490.0, -7830.0]),
    (67.5, [5800.0, 505.0, -7830.0], [5950.0, 485.0, -7870.0]),
    (71.0, [6000.0, 495.0, -7900.0], [6080.0, 470.0, -8040.0]),
    (74.5, [6080.0, 490.0, -8030.0], [6150.0, 455.0, -8200.0]),
    (78.0, [6130.0, 480.0, -8140.0], [6170.0, 450.0, -8450.0]),
    // out over the sea
    (83.0, [6170.0, 500.0, -8500.0], [6200.0, 450.0, -9300.0]),
];

pub struct FlythroughPlugin;

impl Plugin for FlythroughPlugin {
    fn build(&self, app: &mut App) {
        if let Ok(dir) = std::env::var("OCHRE_FLYTHROUGH") {
            std::fs::create_dir_all(&dir).expect("flythrough frame directory");
            app.insert_resource(Flythrough {
                dir,
                started: None,
                frame: None,
                lift: Vec::new(),
            })
            .add_systems(PostUpdate, fly.before(TransformSystems::Propagate));
        }
    }
}

#[derive(Resource)]
struct Flythrough {
    dir: String,
    // when the player first existed, which the warmup counts from
    started: Option<Instant>,
    // the frame being recorded, once warmed up
    frame: Option<u32>,
    // blocks over the path the camera rises each frame, worked out for the whole flight up front
    lift: Vec<f32>,
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
            if a * b <= 0.0 {
                0.0
            } else {
                2.0 * a * b / (a + b)
            }
        }))
    };
    let (position, look) = (|i: usize| at(i).1, |i: usize| at(i).2);
    let ((t0, p0, q0), (t1, p1, q1)) = (at(i), at(i + 1));
    let (m0, m1) = (tangent(i, &position), tangent(i + 1, &position));
    let (n0, n1) = (tangent(i, &look), tangent(i + 1, &look));
    let span = t1 - t0;
    let s = ((t - t0) / span).clamp(0.0, 1.0);
    let (s2, s3) = (s * s, s * s * s);
    let (h00, h10, h01, h11) = (
        2.0 * s3 - 3.0 * s2 + 1.0,
        s3 - 2.0 * s2 + s,
        -2.0 * s3 + 3.0 * s2,
        s3 - s2,
    );
    let hermite = |a: Vec3, ma: Vec3, b: Vec3, mb: Vec3| {
        a * h00 + ma * span * h10 + b * h01 + mb * span * h11
    };
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
    let started = *state.started.get_or_insert_with(Instant::now);
    let frame = match state.frame {
        Some(frame) => frame,
        None if started.elapsed().as_secs_f32() < WARMUP => 0,
        None => {
            commands.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
                1.0 / FPS as f64,
            )));
            state.frame = Some(0);
            0
        }
    };
    if state.lift.is_empty() {
        let frames = (KEYS[KEYS.len() - 1].0 * FPS as f32) as usize + 1;
        let need: Vec<f32> = (0..frames)
            .map(|frame| {
                let p = path(frame as f32 / FPS as f32).0;
                ground_height(&world, p) + CLEARANCE - p.y
            })
            .collect();
        state.lift = smooth_lift(&need, (LIFT_REACH * FPS as f32) as usize);
    }
    let t = frame as f32 / FPS as f32;
    if t > KEYS[KEYS.len() - 1].0 {
        exit.write(AppExit::Success);
        return;
    }
    let (mut at, look) = path(t);
    at.y += state.lift[(frame as usize).min(state.lift.len() - 1)];
    // and never in a block the ground's height missed, a tree or an overhang
    at.y = at.y.max(block_height(&world, at));
    let facing = (look - at).normalize_or_zero();
    body.translation = at - Vec3::Y * EYE_HEIGHT;
    body.rotation = Quat::from_rotation_y(f32::atan2(-facing.x, -facing.z));
    eye.translation = Vec3::Y * EYE_HEIGHT;
    eye.rotation = Quat::from_rotation_x(facing.y.clamp(-1.0, 1.0).asin());
    if let Some(frame) = state.frame {
        let file = format!("{}/frame_{frame:05}.png", state.dir);
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(file));
        state.frame = Some(frame + 1);
    }
}

// The highest ground or water round `p`'s column, from the terrain itself, as the blocks far along the path aren't loaded yet.
fn ground_height(world: &World, p: Vec3) -> f32 {
    let terrain = world.region_store.terrain();
    let mut top = f32::MIN;
    for dx in [-8.0, 0.0, 8.0] {
        for dz in [-8.0, 0.0, 8.0] {
            let sample = terrain.sample((p.x + dx) as f64, (p.z + dz) as f64);
            top = top.max((sample.height as f32).max(sample.water_surface));
        }
    }
    top
}

// How high the camera must be over any solid block right round `p`, trees included.
fn block_height(world: &World, p: Vec3) -> f32 {
    let mut floor = f32::MIN;
    for dx in -1..=1 {
        for dz in -1..=1 {
            let (x, z) = (p.x.round() as i64 + dx, p.z.round() as i64 + dz);
            let top = (p.y.round() as i64 - 4..=p.y.round() as i64 + 16)
                .rev()
                .find(|&y| {
                    world
                        .get_block_at(bevy::math::I64Vec3::new(x, y, z))
                        .is_solid()
                });
            if let Some(y) = top {
                floor = floor.max(y as f32 + 0.5 + BLOCK_CLEARANCE);
            }
        }
    }
    floor
}

// The lift over each frame: never under what it `need`s, the highest need within `reach` frames either side, eased twice over that span so it curves in and out.
fn smooth_lift(need: &[f32], reach: usize) -> Vec<f32> {
    let window = |i: usize| i.saturating_sub(reach)..(i + reach + 1).min(need.len());
    let mut lift: Vec<f32> = (0..need.len())
        .map(|i| need[window(i)].iter().copied().fold(0.0, f32::max))
        .collect();
    // each frame of two means over half the span still only draws on frames whose highest covers its own need
    let half = reach / 2;
    for _ in 0..2 {
        lift = (0..lift.len())
            .map(|i| {
                let span = i.saturating_sub(half)..(i + half + 1).min(lift.len());
                lift[span.clone()].iter().sum::<f32>() / span.len() as f32
            })
            .collect();
    }
    lift
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_path_passes_through_each_key_and_never_overshoots_between() {
        for (i, &(t, at, look)) in KEYS.iter().enumerate() {
            let (p, q) = path(t);
            assert!(
                p.distance(Vec3::from(at)) < 1e-2 && q.distance(Vec3::from(look)) < 1e-2,
                "key {i}"
            );
        }
        for pair in KEYS.windows(2) {
            let ((t0, a0, _), (t1, a1, _)) = (pair[0], pair[1]);
            for step in 0..=50 {
                let p = path(t0 + (t1 - t0) * step as f32 / 50.0).0;
                for k in 0..3 {
                    let (lo, hi) = (a0[k].min(a1[k]), a0[k].max(a1[k]));
                    assert!(
                        (lo - 1e-2..=hi + 1e-2).contains(&p[k]),
                        "overshoot between {t0} and {t1}"
                    );
                }
            }
        }
    }

    #[test]
    fn test_lift_clears_every_need_and_changes_gently() {
        // flat, then a sharp spike and a jagged ridge, as rough ground under a straight path
        let need: Vec<f32> = (0..600)
            .map(|i| match i {
                100 => 80.0,
                300..=400 => 40.0 + 30.0 * ((i % 7) as f32 / 7.0),
                _ => -20.0,
            })
            .collect();
        let lift = smooth_lift(&need, 150);
        assert!(need.iter().zip(&lift).all(|(n, l)| l >= n && *l >= 0.0));
        let steepest = lift
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f32::max);
        assert!(steepest < 1.5, "{steepest} blocks in a frame");
    }
}
