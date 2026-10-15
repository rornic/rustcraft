use bevy::{
    app::{App, Plugin, PostUpdate},
    asset::{Assets, Handle, RenderAssetUsages},
    camera::{
        primitives::Frustum, visibility::VisibilitySystems, Camera3d, Projection,
    },
    ecs::{
        query::With,
        resource::Resource,
        schedule::IntoScheduleConfigs,
        system::{Query, ResMut},
        world::{FromWorld, World},
    },
    math::{FloatExt, Vec3, Vec4},
    render::storage::ShaderBuffer,
    shape::HalfSpace,
    transform::{components::GlobalTransform, TransformSystems},
};

use crate::chunks::chunk::CHUNK_SIZE;
use crate::chunks::generate::{WATER_LEVEL, WORLD_HEIGHT};
use crate::horizon::MAX_RENDER_DISTANCE;

// The tallest land can rise above the sea.
const TALLEST: f32 = (WORLD_HEIGHT as i64 - WATER_LEVEL) as f32;
/// Between these heights above sea level the bend eases in, so near the ground the world is
/// drawn flat as ever and distant mountains don't sink, and from high up the horizon curves away.
const FADE_START: f32 = 100.0;
const FADE_END: f32 = 450.0;

/// The round world the land is drawn bent around: always the same size, so a shorter render
/// distance shows less of its curve.
pub const WORLD_CURVE: Curve = Curve::seen_from_the_top_to(MAX_RENDER_DISTANCE);

/// A round world to bend the land around.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Curve {
    /// In blocks.
    pub radius: f32,
}

impl Curve {
    /// The world whose horizon from the top of the world is `chunks` away. From sea level
    /// its horizon is half as far, see `horizon_distance`.
    pub const fn seen_from_the_top_to(chunks: f32) -> Self {
        let blocks = chunks * CHUNK_SIZE as f32;
        // solves `horizon_distance` at the top of the world for the radius
        Self { radius: blocks * blocks / (8.0 * TALLEST) }
    }

    /// How far out land can be seen from `height` blocks up: out to the sea's horizon, and
    /// on past it for as long as the tallest peaks still rise above it. While the bend is
    /// still easing in, land drops less and so could be seen farther, but it's fogged out by
    /// then, as on flat ground.
    pub fn horizon_distance(self, height: f32) -> f32 {
        let above_sea = (height - WATER_LEVEL as f32).max(0.0);
        (2.0 * self.radius).sqrt() * (above_sea.sqrt() + TALLEST.sqrt())
    }

    /// How far below the camera's flat horizon land `distance` blocks away is drawn, seen
    /// from `height` blocks up: a sphere's drop, while it's still eased in.
    pub fn drop_at(self, distance: f32, height: f32) -> f32 {
        distance * distance * self.bend_at(height)
    }

    // The drop per square block of distance from `height` blocks up.
    fn bend_at(self, height: f32) -> f32 {
        let above_sea = height - WATER_LEVEL as f32;
        let t = ((above_sea - FADE_START) / (FADE_END - FADE_START)).clamp(0.0, 1.0);
        0.0.lerp(0.5 / self.radius, t * t * (3.0 - 2.0 * t))
    }
}

/// What the shaders bend the land around, see `curvature.wesl`: the camera's position and
/// `Curve::bend_at` its height, this frame's then last frame's for motion vectors. The camera's
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

fn frame(eye: Vec3, curve: Curve) -> [f32; 4] {
    [eye.x, eye.y, eye.z, curve.bend_at(eye.y)]
}

fn update_curvature(
    mut curvature: ResMut<Curvature>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
    camera: Query<&GlobalTransform, With<Camera3d>>,
) {
    let Ok(transform) = camera.single() else {
        return;
    };
    let now = frame(transform.translation(), WORLD_CURVE);
    if let Some(mut buffer) = buffers.get_mut(&curvature.buffer) {
        buffer.clear();
        buffer.extend_from_slice(&now);
        buffer.extend_from_slice(&curvature.last);
    }
    curvature.last = now;
}

/// Bevy culls what's out of view where it lies flat, but the land's drawn dropped by the
/// curve, so from high up some of it drops into view under the top of the screen while it
/// still lies flat above it. Each side of the camera's frustum that faces down is loosened by
/// as far as anything drawn can drop, letting through everything the curve could bring into
/// view. Rebuilt from the camera every frame, as Bevy only rebuilds it when the camera moves.
fn loosen_frustum(mut cameras: Query<(&GlobalTransform, &Projection, &mut Frustum), With<Camera3d>>) {
    for (transform, projection, mut frustum) in &mut cameras {
        let Projection::Perspective(perspective) = projection else {
            continue;
        };
        // nothing's drawn past the far plane
        let drop = WORLD_CURVE.drop_at(perspective.far, transform.translation().y);
        *frustum = loosened(projection.compute_frustum(transform), drop);
    }
}

// `frustum` letting through all that's in it once dropped by up to `drop`.
fn loosened(mut frustum: Frustum, drop: f32) -> Frustum {
    for half_space in &mut frustum.0.half_spaces {
        // inside where normal · p + d > 0, and dropping p by `drop` adds -normal.y * drop
        let normal_d = half_space.normal_d();
        *half_space = HalfSpace::new(normal_d + Vec4::W * (-normal_d.y).max(0.0) * drop);
    }
    frustum
}

pub struct CurvaturePlugin;

impl Plugin for CurvaturePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Curvature>()
            // after the camera has moved this frame, so the land bends around where it is now
            .add_systems(
                PostUpdate,
                (
                    update_curvature.after(TransformSystems::Propagate),
                    loosen_frustum
                        .after(VisibilitySystems::UpdateFrusta)
                        .before(VisibilitySystems::CheckVisibility),
                ),
            );
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    const SEA: f32 = WATER_LEVEL as f32;
    const CURVE: Curve = Curve { radius: 200_000.0 };

    #[test]
    fn test_flat_near_the_ground() {
        assert_eq!(0.0, CURVE.drop_at(10_000.0, SEA));
        assert_eq!(0.0, CURVE.drop_at(10_000.0, SEA + FADE_START));
        assert_eq!(0.0, CURVE.drop_at(10_000.0, 0.0));
    }

    #[test]
    fn test_a_sphere_from_high_up() {
        // a sphere's drop d²/2R, nearly exact this close
        let height = SEA + FADE_END;
        let sphere = 10_000.0f32.powi(2) / (2.0 * CURVE.radius);
        assert!((CURVE.drop_at(10_000.0, height) - sphere).abs() < 1e-3);
        assert_eq!(CURVE.drop_at(10_000.0, height), CURVE.drop_at(10_000.0, height + 1000.0));
    }

    #[test]
    fn test_bends_more_the_higher_up() {
        let drops: Vec<f32> = (0..=10)
            .map(|i| CURVE.drop_at(10_000.0, SEA + FADE_START + (FADE_END - FADE_START) * i as f32 / 10.0))
            .collect();
        assert!(drops.windows(2).all(|w| w[0] < w[1]), "{drops:?}");
    }

    #[test]
    fn test_horizon_is_where_the_land_drops_out_of_sight() {
        // fully bent, the sea's horizon is where a line from the eye just grazes it
        let height = SEA + 500.0;
        let tallest_beyond = (2.0 * CURVE.radius * TALLEST).sqrt();
        let sea_horizon = CURVE.horizon_distance(height) - tallest_beyond;
        let eye = height - SEA;
        // the line's slope to the sea there is the curve's own slope there
        let to_sea = (eye + CURVE.drop_at(sea_horizon, height)) / sea_horizon;
        let curve = 2.0 * sea_horizon * CURVE.drop_at(1.0, height);
        assert!((to_sea - curve).abs() < 1e-4, "{to_sea} {curve}");
        // and the tallest peaks just show over it as far again as they'd see it themselves
        assert!((CURVE.drop_at(tallest_beyond, height) - TALLEST).abs() < 1e-1);
    }

    #[test]
    fn test_horizon_grows_with_height() {
        let distances: Vec<f32> = (0..10).map(|i| CURVE.horizon_distance(SEA + i as f32 * 100.0)).collect();
        assert!(distances.windows(2).all(|w| w[0] < w[1]), "{distances:?}");
        assert_eq!(CURVE.horizon_distance(SEA), CURVE.horizon_distance(0.0));
    }

    #[test]
    fn test_the_top_of_the_world_sees_to_the_cap() {
        let blocks = MAX_RENDER_DISTANCE * CHUNK_SIZE as f32;
        assert!((WORLD_CURVE.horizon_distance(WORLD_HEIGHT as f32) - blocks).abs() < 1.0);
        assert!((WORLD_CURVE.horizon_distance(SEA) - blocks / 2.0).abs() < 0.5);
    }

    #[test]
    fn test_loosened_frustum_keeps_what_drops_into_view() {
        use bevy::{camera::{primitives::Sphere, PerspectiveProjection}, math::{Quat, Vec3A}};
        // looking down from high up
        let eye = GlobalTransform::from_translation(Vec3::new(0.0, 1000.0, 0.0))
            .mul_transform(bevy::transform::components::Transform::from_rotation(Quat::from_rotation_x(-0.48)));
        let projection = Projection::Perspective(PerspectiveProjection { far: 30_000.0, ..Default::default() });
        let frustum = projection.compute_frustum(&eye);
        // the view's top edge is about 5° below level, the point 3° below flat and 7° dropped
        let drop = 1500.0;
        let wide = loosened(frustum, drop);
        let seen = |frustum: &Frustum, p: Vec3| frustum.intersects_sphere(&Sphere { center: Vec3A::from(p), radius: 0.0 }, false);
        // far ahead, above the top of the view where it lies, but in it once dropped
        let flat = Vec3::new(0.0, 0.0, -20_000.0);
        let dropped = flat - Vec3::Y * drop;
        assert!(!seen(&frustum, flat) && seen(&frustum, dropped));
        assert!(seen(&wide, flat));
        // still culled off to the side and behind
        assert!(!seen(&wide, Vec3::new(30_000.0, 0.0, -1000.0)));
        assert!(!seen(&wide, Vec3::new(0.0, 0.0, 5000.0)));
    }
}
