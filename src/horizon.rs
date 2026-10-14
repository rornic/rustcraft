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
use crate::chunks::generate::noise::WORLD_SIZE;
use crate::chunks::generate::WORLD_HEIGHT;
use crate::chunks::lod::LodLoader;
use crate::curvature::horizon_distance;

/// Far enough to see the whole world from its middle.
const MAX_RENDER_DISTANCE: f32 = (WORLD_SIZE / CHUNK_SIZE as i64) as f32;
// Smaller changes than this, as a share of the render distance, are let be, so bobbing up
// and down doesn't keep selecting LOD nodes afresh.
const RESELECT_CHANGE: f32 = 0.1;
// Roughly how long fog takes to follow a new render distance, so it eases out as the
// further terrain loads in rather than jumping ahead of it.
const FOG_FOLLOW_SECONDS: f32 = 2.0;

/// How far terrain is drawn, in chunks, from `height` blocks up with `set_distance` set: out
/// to the horizon, so climbing shows more of the world, but never less than the set distance.
pub fn render_distance_at(set_distance: f32, height: f32) -> f32 {
    let horizon = horizon_distance(height) / CHUNK_SIZE as f32;
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
    fn test_never_draws_less_than_the_set_distance() {
        let sea = WATER_LEVEL as f32;
        let far = 4.0 * horizon_distance(sea + 1000.0) / CHUNK_SIZE as f32;
        assert_eq!(far, render_distance_at(far, sea));
        assert_eq!(far, render_distance_at(far, 0.0));
    }

    #[test]
    fn test_climbing_draws_out_to_the_horizon_up_to_the_whole_world() {
        let sea = WATER_LEVEL as f32;
        let hill = render_distance_at(16.0, sea + 100.0);
        let mountain = render_distance_at(16.0, sea + 300.0);
        assert!(16.0 < hill && hill < mountain);
        assert_eq!(horizon_distance(sea + 300.0) / CHUNK_SIZE as f32, mountain);
        assert_eq!(MAX_RENDER_DISTANCE, render_distance_at(16.0, sea + 1.0e9));
    }
}
