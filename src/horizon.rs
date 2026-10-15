use bevy::{
    camera::{Camera3d, Projection},
    ecs::{
        query::With,
        system::{Query, Res, ResMut},
    },
    math::FloatExt,
    pbr::{DistanceFog, FogFalloff},
    time::Time,
    transform::components::GlobalTransform,
};

use crate::chunks::chunk::CHUNK_SIZE;
use crate::chunks::generate::{WATER_LEVEL, WORLD_HEIGHT};
use crate::chunks::lod::LodLoader;
use crate::curvature::WORLD_CURVE;

/// The farthest terrain is drawn, in chunks, however high up.
pub const MAX_RENDER_DISTANCE: f32 = 2048.0;
// How far above the sea the top of the world is.
const ABOVE_SEA_AT_TOP: f32 = (WORLD_HEIGHT as i64 - WATER_LEVEL) as f32;
// Smaller changes than this, as a share of the render distance, are let be, so bobbing up
// and down doesn't keep selecting LOD nodes afresh.
const RESELECT_CHANGE: f32 = 0.1;
// Roughly how long fog takes to follow a new render distance, so it eases out as the
// further terrain loads in rather than jumping ahead of it.
const FOG_FOLLOW_SECONDS: f32 = 2.0;

/// How far terrain is drawn, in chunks, from `height` blocks up with `set_distance` set: the
/// set distance from the ground, growing to twice it at the top of the world, but never past
/// the horizon, where the curve hides the rest, see `WORLD_CURVE`.
pub fn render_distance_at(set_distance: f32, height: f32) -> f32 {
    let above_sea = (height - WATER_LEVEL as f32).max(0.0);
    let climbed = set_distance * (1.0 + (above_sea / ABOVE_SEA_AT_TOP).sqrt());
    let horizon = WORLD_CURVE.horizon_distance(height) / CHUNK_SIZE as f32;
    climbed.min(horizon).min(MAX_RENDER_DISTANCE).max(set_distance)
}

/// Draws terrain out to the horizon, further the higher the camera climbs, and moves the fog
/// with it: half fogged where the farthest LOD level starts, fully fogged at the edge, see
/// `fog.wesl`.
pub fn update_horizon(
    time: Res<Time>,
    mut lods: ResMut<LodLoader>,
    mut camera: Query<(&GlobalTransform, &mut DistanceFog, &mut Projection), With<Camera3d>>,
) {
    let Ok((transform, mut fog, mut projection)) = camera.single_mut() else {
        return;
    };
    let wanted = render_distance_at(lods.set_render_distance_chunks(), transform.translation().y);
    let current = lods.layout().render_distance();
    if (wanted - current).abs() > current * RESELECT_CHANGE {
        lods.set_render_distance(wanted);
    }

    let layout = lods.layout();
    let (half, end) = (
        layout.last_level_start() * CHUNK_SIZE as f32,
        layout.render_distance() * CHUNK_SIZE as f32,
    );
    if let Projection::Perspective(perspective) = &mut *projection {
        perspective.far = end + WORLD_HEIGHT as f32;
    }
    // underwater, `update_underwater_effects` overrides the fog after this
    if let FogFalloff::Linear { start, end: fog_end } = &mut fog.falloff {
        let follow = 1.0 - (-time.delta_secs() / FOG_FOLLOW_SECONDS).exp();
        *start = start.lerp(half, follow);
        *fog_end = fog_end.lerp(end, follow);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEA: f32 = WATER_LEVEL as f32;
    const TOP: f32 = WORLD_HEIGHT as f32;

    #[test]
    fn test_the_ground_draws_the_set_distance() {
        for set in [256.0, 512.0, 1024.0, 1536.0] {
            assert_eq!(set, render_distance_at(set, SEA));
            assert_eq!(set, render_distance_at(set, 0.0));
        }
    }

    #[test]
    fn test_climbing_to_the_top_draws_twice_as_far() {
        for set in [256.0, 512.0, 1024.0] {
            let hill = render_distance_at(set, SEA + 100.0);
            let mountain = render_distance_at(set, SEA + 300.0);
            assert!(set < hill && hill < mountain);
            assert!((render_distance_at(set, TOP) - 2.0 * set).abs() < 0.1);
        }
    }

    #[test]
    fn test_never_past_the_horizon_or_the_cap_or_short_of_the_set_distance() {
        // set further than the world shows from the ground, it's drawn out to the horizon once that's further
        let horizon = |height: f32| WORLD_CURVE.horizon_distance(height) / CHUNK_SIZE as f32;
        assert_eq!(1536.0, render_distance_at(1536.0, SEA + 100.0));
        assert!(horizon(SEA + 200.0) > 1536.0);
        assert_eq!(horizon(SEA + 200.0), render_distance_at(1536.0, SEA + 200.0));
        assert_eq!(MAX_RENDER_DISTANCE, render_distance_at(1024.0, SEA + 1.0e6));
        assert_eq!(4096.0, render_distance_at(4096.0, SEA + 1.0e6));
    }
}
