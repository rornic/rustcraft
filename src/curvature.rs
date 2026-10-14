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

use crate::chunks::generate::{WATER_LEVEL, WORLD_HEIGHT};

/// The radius of the world the land is drawn bent around, in blocks: small enough that
/// from sea level the horizon is about the default render distance, see `horizon_distance`.
const RADIUS: f32 = 60_000.0;
// The tallest land can rise above the sea.
const TALLEST: f32 = (WORLD_HEIGHT as i64 - WATER_LEVEL) as f32;
/// Between these heights above sea level the bend eases in, so near the ground the world is
/// drawn flat as ever and distant mountains don't sink, and from high up the horizon curves away.
const FADE_START: f32 = 100.0;
const FADE_END: f32 = 450.0;

/// How far below the camera's flat horizon land `distance` blocks away is drawn, seen from
/// `height` blocks up: the drop of a sphere of `RADIUS` while it's still eased in.
pub fn drop_at(distance: f32, height: f32) -> f32 {
    distance * distance * bend_at(height)
}

/// How far out land can be seen from `height` blocks up over a world of `RADIUS`: out to the
/// sea's horizon, and on past it for as long as the tallest peaks still rise above it.
/// While the bend is still easing in, land drops less and so could be seen farther, but
/// it's fogged out by then, as on flat ground.
pub fn horizon_distance(height: f32) -> f32 {
    let above_sea = (height - WATER_LEVEL as f32).max(0.0);
    (2.0 * RADIUS).sqrt() * (above_sea.sqrt() + TALLEST.sqrt())
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
        let sphere = 10_000.0f32.powi(2) / (2.0 * RADIUS);
        assert!((drop_at(10_000.0, height) - sphere).abs() < 1e-3);
        assert_eq!(drop_at(10_000.0, height), drop_at(10_000.0, height + 1000.0));
    }

    #[test]
    fn test_bends_more_the_higher_up() {
        let drops: Vec<f32> = (0..=10)
            .map(|i| drop_at(10_000.0, SEA + FADE_START + (FADE_END - FADE_START) * i as f32 / 10.0))
            .collect();
        assert!(drops.windows(2).all(|w| w[0] < w[1]), "{drops:?}");
    }

    #[test]
    fn test_horizon_is_where_the_land_drops_out_of_sight() {
        // fully bent, the sea's horizon is where a line from the eye just grazes it
        let height = SEA + 500.0;
        let tallest_beyond = (2.0 * RADIUS * TALLEST).sqrt();
        let sea_horizon = horizon_distance(height) - tallest_beyond;
        let eye = height - SEA;
        // the line's slope to the sea there is the curve's own slope there
        let to_sea = (eye + drop_at(sea_horizon, height)) / sea_horizon;
        let curve = 2.0 * sea_horizon * drop_at(1.0, height);
        assert!((to_sea - curve).abs() < 1e-4, "{to_sea} {curve}");
        // and the tallest peaks just show over it as far again as they'd see it themselves
        assert!((drop_at(tallest_beyond, height) - TALLEST).abs() < 1e-2);
    }

    #[test]
    fn test_horizon_grows_with_height() {
        let distances: Vec<f32> = (0..10).map(|i| horizon_distance(SEA + i as f32 * 100.0)).collect();
        assert!(distances.windows(2).all(|w| w[0] < w[1]), "{distances:?}");
        assert_eq!(horizon_distance(SEA), horizon_distance(0.0));
    }
}
