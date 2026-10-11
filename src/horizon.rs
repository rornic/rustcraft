use bevy::{
    core_pipeline::core_3d::Camera3d,
    ecs::{
        query::With,
        system::{Query, Res, ResMut},
    },
    math::FloatExt,
    pbr::{DistanceFog, FogFalloff},
    render::camera::Projection,
    time::Time,
    transform::components::GlobalTransform,
};

use crate::chunks::chunk::CHUNK_SIZE;
use crate::chunks::generate::noise::WORLD_SIZE;
use crate::chunks::generate::{WATER_LEVEL, WORLD_HEIGHT};
use crate::chunks::lod::LodLoader;

/// How far below the horizon the edge of the world lies, as a slope: the higher up the
/// camera, the further out terrain is drawn to keep it there, so climbing shows more of the
/// world. The set render distance is how far it's drawn at sea level.
const HORIZON_DIP: f32 = 0.05;
/// Far enough to see the whole world from its middle.
const MAX_RENDER_DISTANCE: f32 = (WORLD_SIZE / CHUNK_SIZE as i64) as f32;
// Smaller changes than this, as a share of the render distance, are let be, so bobbing up
// and down doesn't keep selecting LOD nodes afresh.
const RESELECT_CHANGE: f32 = 0.1;
// Roughly how long fog takes to follow a new render distance, so it eases out as the
// further terrain loads in rather than jumping ahead of it.
const FOG_FOLLOW_SECONDS: f32 = 2.0;

/// How far terrain is drawn, in chunks, from `height` blocks up with `set_distance` set.
pub fn render_distance_at(set_distance: f32, height: f32) -> f32 {
    let above_sea = (height - WATER_LEVEL as f32).max(0.0);
    let further = above_sea / HORIZON_DIP / CHUNK_SIZE as f32;
    (set_distance + further).min(MAX_RENDER_DISTANCE.max(set_distance))
}

/// Draws terrain further out the higher the camera climbs, and moves the fog with it: half
/// fogged where the farthest LOD level starts, fully fogged at the edge, see `fog.wgsl`.
pub fn update_horizon(
    time: Res<Time>,
    mut lods: ResMut<LodLoader>,
    mut camera: Query<(&GlobalTransform, &mut DistanceFog, &mut Projection), With<Camera3d>>,
) {
    let Ok((transform, mut fog, mut projection)) = camera.get_single_mut() else {
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

    #[test]
    fn test_sea_level_draws_the_set_distance() {
        assert_eq!(256.0, render_distance_at(256.0, WATER_LEVEL as f32));
        assert_eq!(256.0, render_distance_at(256.0, 0.0));
    }

    #[test]
    fn test_climbing_draws_further_out_up_to_the_whole_world() {
        let sea = WATER_LEVEL as f32;
        let hill = render_distance_at(256.0, sea + 100.0);
        let mountain = render_distance_at(256.0, sea + 300.0);
        assert!(256.0 < hill && hill < mountain);
        // the edge stays the same way below the horizon
        let dip = |height: f32, distance: f32| height / ((distance - 256.0) * CHUNK_SIZE as f32);
        assert!((dip(100.0, hill) - dip(300.0, mountain)).abs() < 1e-6);
        assert_eq!(MAX_RENDER_DISTANCE, render_distance_at(256.0, sea + 5000.0));
    }
}
