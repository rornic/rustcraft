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
use crate::chunks::generate::WORLD_HEIGHT;
use crate::chunks::lod::LodLoader;
use crate::curvature::Curve;

/// The farthest terrain is drawn, in chunks, however high up.
const MAX_RENDER_DISTANCE: f32 = 2048.0;
// Smaller changes than this, as a share of the render distance, are let be, so bobbing up
// and down doesn't keep selecting LOD nodes afresh.
const RESELECT_CHANGE: f32 = 0.1;
// Roughly how long fog takes to follow a new render distance, so it eases out as the
// further terrain loads in rather than jumping ahead of it.
const FOG_FOLLOW_SECONDS: f32 = 2.0;

/// How far terrain is drawn, in chunks, from `height` blocks up with `set_distance` set: out
/// to the horizon of the world bent to it, so the set distance from the ground, and further
/// the higher up, see `Curve::for_render_distance`.
pub fn render_distance_at(set_distance: f32, height: f32) -> f32 {
    let horizon = Curve::for_render_distance(set_distance).horizon_distance(height) / CHUNK_SIZE as f32;
    horizon.max(set_distance).min(MAX_RENDER_DISTANCE.max(set_distance))
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
    use crate::chunks::generate::WATER_LEVEL;

    #[test]
    fn test_the_ground_draws_the_set_distance() {
        let sea = WATER_LEVEL as f32;
        assert!((render_distance_at(1024.0, sea) - 1024.0).abs() < 0.1);
        assert_eq!(1024.0, render_distance_at(1024.0, 0.0));
    }

    #[test]
    fn test_climbing_draws_further_out_up_to_the_cap() {
        let sea = WATER_LEVEL as f32;
        let hill = render_distance_at(1024.0, sea + 100.0);
        let mountain = render_distance_at(1024.0, sea + 300.0);
        assert!(1024.0 < hill && hill < mountain);
        // the top of the world sees twice as far as the ground
        assert!((render_distance_at(1024.0, WORLD_HEIGHT as f32) - 2048.0).abs() < 0.1);
        assert_eq!(MAX_RENDER_DISTANCE, render_distance_at(1024.0, sea + 1.0e6));
        // but never draws less than it's set to
        assert_eq!(4096.0, render_distance_at(4096.0, sea + 1.0e6));
    }
}
