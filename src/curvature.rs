use bevy::{
    app::{App, Plugin, PostUpdate},
    asset::{Assets, Handle, RenderAssetUsages},
    camera::Camera3d,
    ecs::{
        query::With,
        resource::Resource,
        schedule::IntoScheduleConfigs,
        system::{Query, ResMut},
        world::{FromWorld, World},
    },
    math::{FloatExt, Vec3},
    render::storage::ShaderBuffer,
    transform::{components::GlobalTransform, TransformSystems},
};

use crate::chunks::generate::WATER_LEVEL;

/// The radius of the world the land is drawn bent around, in blocks.
const RADIUS: f32 = 100_000.0;
/// Between these heights above sea level the bend eases in, so near the ground the world is
/// drawn flat as ever and distant mountains don't sink, and from high up the horizon curves away.
const FADE_START: f32 = 100.0;
const FADE_END: f32 = 450.0;

/// How far below the camera's flat horizon land `distance` blocks away is drawn, seen from
/// `height` blocks up: the drop of a sphere of `RADIUS` while it's still eased in.
pub fn drop_at(distance: f32, height: f32) -> f32 {
    distance * distance * bend_at(height)
}

// The drop per square block of distance from `height` blocks up.
fn bend_at(height: f32) -> f32 {
    let above_sea = height - WATER_LEVEL as f32;
    let t = ((above_sea - FADE_START) / (FADE_END - FADE_START)).clamp(0.0, 1.0);
    0.0.lerp(0.5 / RADIUS, t * t * (3.0 - 2.0 * t))
}

/// What the shaders bend the land around, see `curvature.wesl`: the camera's position and
/// `bend_at` its height, this frame's then last frame's for motion vectors. The camera's
/// own, as the sun's shadow passes see the light's view in place of the camera's.
///
/// Every material that draws the land carries this one buffer, at binding 8.
#[derive(Resource)]
pub struct Curvature {
    pub buffer: Handle<ShaderBuffer>,
    last: [f32; 4],
}

impl FromWorld for Curvature {
    fn from_world(world: &mut World) -> Self {
        let buffer = ShaderBuffer::new(vec![0.0f32; 8], RenderAssetUsages::default());
        Self {
            buffer: world.resource_mut::<Assets<ShaderBuffer>>().add(buffer),
            last: [0.0; 4],
        }
    }
}

fn frame(eye: Vec3) -> [f32; 4] {
    [eye.x, eye.y, eye.z, bend_at(eye.y)]
}

fn update_curvature(
    mut curvature: ResMut<Curvature>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
    camera: Query<&GlobalTransform, With<Camera3d>>,
) {
    let Ok(transform) = camera.single() else {
        return;
    };
    let now = frame(transform.translation());
    if let Some(mut buffer) = buffers.get_mut(&curvature.buffer) {
        buffer.clear();
        buffer.extend_from_slice(&now);
        buffer.extend_from_slice(&curvature.last);
    }
    curvature.last = now;
}

pub struct CurvaturePlugin;

impl Plugin for CurvaturePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Curvature>()
            // after the camera has moved this frame, so the land bends around where it is now
            .add_systems(PostUpdate, update_curvature.after(TransformSystems::Propagate));
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    const SEA: f32 = WATER_LEVEL as f32;

    #[test]
    fn test_flat_near_the_ground() {
        assert_eq!(0.0, drop_at(10_000.0, SEA));
        assert_eq!(0.0, drop_at(10_000.0, SEA + FADE_START));
        assert_eq!(0.0, drop_at(10_000.0, 0.0));
    }

    #[test]
    fn test_a_sphere_from_high_up() {
        // a sphere's drop d²/2R, nearly exact this close
        let height = SEA + FADE_END;
        assert!((drop_at(10_000.0, height) - 500.0).abs() < 1e-3);
        assert_eq!(drop_at(10_000.0, height), drop_at(10_000.0, height + 1000.0));
    }

    #[test]
    fn test_bends_more_the_higher_up() {
        let drops: Vec<f32> = (0..=10)
            .map(|i| drop_at(10_000.0, SEA + FADE_START + (FADE_END - FADE_START) * i as f32 / 10.0))
            .collect();
        assert!(drops.windows(2).all(|w| w[0] < w[1]), "{drops:?}");
    }
}
