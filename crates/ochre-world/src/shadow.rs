//! Restricts terrain casters to the camera slice swept toward its shadow-casting light.

use bevy::{
    camera::{
        primitives::Aabb,
        visibility::{CascadesVisibleEntities, NoCpuCulling, NoFrustumCulling},
        Camera, Camera3d, Projection,
    },
    ecs::{
        query::{With, Without},
        system::Query,
    },
    light::{CascadeShadowConfig, Cascades, DirectionalLight},
    math::{Affine3A, Vec3A},
    pbr::MeshMaterial3d,
    transform::components::GlobalTransform,
};
use ochre_terrain::{curvature::WORLD_CURVE, material::ChunkMaterial};

/// Outward half-spaces of a view slice swept indefinitely toward the sun. Points are camera
/// relative so plane offsets keep their precision far from the world's origin.
pub struct CasterHull {
    planes: Vec<(Vec3A, f32)>,
}

impl CasterHull {
    pub fn new(corners: [Vec3A; 8], toward_sun: Vec3A) -> Self {
        let mut hull = Self {
            planes: Vec::with_capacity(12),
        };
        // Supporting faces which do not cut off the sweep.
        for i in 0..8 {
            for j in i + 1..8 {
                for k in j + 1..8 {
                    hull.support(
                        &corners,
                        (corners[j] - corners[i]).cross(corners[k] - corners[i]),
                        corners[i],
                        toward_sun,
                    );
                }
                // The silhouette edges extruded parallel to the sun form the remaining faces.
                hull.support(
                    &corners,
                    (corners[j] - corners[i]).cross(toward_sun),
                    corners[i],
                    toward_sun,
                );
            }
        }
        hull
    }

    fn support(&mut self, corners: &[Vec3A; 8], normal: Vec3A, at: Vec3A, sun: Vec3A) {
        if normal.length_squared() < 1e-10 {
            return;
        }
        let mut normal = normal.normalize();
        let mut d = -normal.dot(at);
        let (above, below) = corners.iter().fold((false, false), |(above, below), p| {
            let side = normal.dot(*p) + d;
            (above || side > 0.01, below || side < -0.01)
        });
        if above && below {
            return;
        }
        if above {
            normal = -normal;
            d = -d;
        }
        if normal.dot(sun) > 1e-5 {
            return;
        }
        if !self
            .planes
            .iter()
            .any(|(n, offset)| n.dot(normal) > 0.99999 && (*offset - d).abs() < 0.01)
        {
            self.planes.push((normal, d));
        }
    }

    /// Conservative OBB test, expanded for foliage sway, receiver bias and shadow filtering.
    pub fn intersects(&self, aabb: &Aabb, transform: &Affine3A, eye: Vec3A, margin: f32) -> bool {
        let centre = transform.transform_point3a(aabb.center) - eye;
        let half = aabb.half_extents;
        self.planes.iter().all(|(normal, d)| {
            let m = transform.matrix3;
            let radius = half.x * normal.dot(m.x_axis).abs()
                + half.y * normal.dot(m.y_axis).abs()
                + half.z * normal.dot(m.z_axis).abs();
            normal.dot(centre) + d <= radius + margin
        })
    }
}

#[allow(clippy::type_complexity)]
pub(crate) fn prune_shadow_casters(
    cameras: Query<(&Camera, &Projection, &GlobalTransform), With<Camera3d>>,
    mut lights: Query<(
        &DirectionalLight,
        &CascadeShadowConfig,
        &Cascades,
        &mut CascadesVisibleEntities,
    )>,
    terrain: Query<
        (&Aabb, &GlobalTransform),
        (
            With<MeshMaterial3d<ChunkMaterial>>,
            Without<NoFrustumCulling>,
            Without<NoCpuCulling>,
        ),
    >,
) {
    for (light, config, cascades, mut visible) in &mut lights {
        if !light.shadow_maps_enabled {
            continue;
        }
        for (view, lists) in &mut visible.entities {
            let Ok((camera, projection, transform)) = cameras.get(*view) else {
                continue;
            };
            let eye = transform.translation();
            // Flat CPU bounds no longer enclose the vertices once curvature is enabled.
            if !camera.is_active || !WORLD_CURVE.is_flat_at(eye.y) {
                continue;
            }
            let Some(built) = cascades.cascades.get(view) else {
                continue;
            };
            let rotation = transform.affine().matrix3;
            let mut near = config.minimum_distance;
            for ((list, cascade), &far) in lists.iter_mut().zip(built).zip(&config.bounds) {
                let corners = projection
                    .get_frustum_corners(-near, -far)
                    .map(|p| rotation * p);
                // Use the fitted cascade's direction, which can differ from the smooth visual sun.
                let sun = Vec3A::from(cascade.world_from_cascade.z_axis.truncate()).normalize();
                let hull = CasterHull::new(corners, sun);
                let margin = (4.0 + light.shadow_normal_bias.abs()) * cascade.texel_size
                    + light.shadow_depth_bias.abs()
                    + 0.3;
                list.entities.retain(|entity| {
                    terrain.get(*entity).map_or(true, |(aabb, world)| {
                        hull.intersects(aabb, &world.affine(), eye.into(), margin)
                    })
                });
                near = far * (1.0 - config.overlap_proportion);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::{
        app::App,
        camera::visibility::VisibleMeshEntities,
        camera::PerspectiveProjection,
        ecs::system::RunSystemOnce,
        light::{cascade::Cascade, CascadeShadowConfigBuilder},
        math::{Mat4, Quat, Vec3},
    };

    #[test]
    fn runtime_prunes_only_terrain_and_falls_back_when_curvature_starts() {
        for (height, should_prune) in [
            (ochre_base::WATER_LEVEL as f32, true),
            (ochre_base::WATER_LEVEL as f32 + 101.0, false),
        ] {
            let mut app = App::new();
            let eye = Vec3::new(10_000.0, height, -20_000.0);
            let camera = app
                .world_mut()
                .spawn((
                    Camera3d::default(),
                    Projection::Perspective(PerspectiveProjection::default()),
                    GlobalTransform::from_translation(eye),
                ))
                .id();
            let outside = app
                .world_mut()
                .spawn((
                    MeshMaterial3d::<ChunkMaterial>::default(),
                    Aabb::from_min_max(Vec3::splat(-0.5), Vec3::splat(0.5)),
                    GlobalTransform::from_translation(eye + Vec3::new(1_000.0, 0.0, -5.0)),
                ))
                .id();
            let unknown = app.world_mut().spawn_empty().id();
            let mut cascades = Cascades::default();
            cascades.cascades.insert(
                camera,
                vec![Cascade {
                    world_from_cascade: Mat4::from_quat(Quat::from_rotation_x(
                        -std::f32::consts::FRAC_PI_2,
                    )),
                    texel_size: 0.1,
                    ..Default::default()
                }],
            );
            let mut visible = CascadesVisibleEntities::default();
            visible.entities.insert(
                camera,
                vec![VisibleMeshEntities {
                    entities: vec![outside, unknown],
                }],
            );
            let light = app
                .world_mut()
                .spawn((
                    DirectionalLight {
                        shadow_maps_enabled: true,
                        ..Default::default()
                    },
                    CascadeShadowConfigBuilder {
                        num_cascades: 1,
                        maximum_distance: 10.0,
                        ..Default::default()
                    }
                    .build(),
                    cascades,
                    visible,
                ))
                .id();
            app.world_mut()
                .run_system_once(prune_shadow_casters)
                .unwrap();
            let kept = &app
                .world()
                .get::<CascadesVisibleEntities>(light)
                .unwrap()
                .entities[&camera][0]
                .entities;
            assert_eq!(kept.contains(&outside), !should_prune);
            assert!(kept.contains(&unknown));
        }
    }

    #[test]
    fn sweep_keeps_receivers_and_every_point_toward_the_sun() {
        let projection = Projection::Perspective(PerspectiveProjection {
            aspect_ratio: 1.7,
            ..Default::default()
        });
        let corners = projection.get_frustum_corners(-0.1, -100.0);
        for sun in [
            Vec3A::Y,
            Vec3A::new(1.0, 0.001, 0.0).normalize(),
            Vec3A::new(-1.0, 1.0, 1.0).normalize(),
        ] {
            let hull = CasterHull::new(corners, sun);
            assert!(!hull.planes.is_empty());
            for point in corners.into_iter().chain([Vec3A::new(0.0, 0.0, -50.0)]) {
                for distance in [0.0, 100.0, 10_000.0] {
                    let at = Affine3A::from_translation((point + sun * distance).into());
                    assert!(hull.intersects(
                        &Aabb::from_min_max(Vec3::splat(-0.1), Vec3::splat(0.1)),
                        &at,
                        Vec3A::ZERO,
                        0.01
                    ));
                }
            }
        }
    }

    #[test]
    fn sweep_rejects_sideways_casters_and_keeps_rotated_bounds_and_filter_margin() {
        let projection = Projection::Perspective(PerspectiveProjection::default());
        let hull = CasterHull::new(projection.get_frustum_corners(-1.0, -10.0), Vec3A::Y);
        let aabb = Aabb::from_min_max(Vec3::splat(-1.0), Vec3::splat(1.0));
        let outside = Affine3A::from_translation(Vec3::new(100.0, 0.0, -5.0));
        assert!(!hull.intersects(&aabb, &outside, Vec3A::ZERO, 0.0));
        let rotated = Affine3A::from_scale_rotation_translation(
            Vec3::new(2.0, 1.0, 1.0),
            Quat::from_rotation_y(0.7),
            Vec3::new(0.0, 0.0, -5.0),
        );
        assert!(hull.intersects(&aabb, &rotated, Vec3A::ZERO, 0.0));
        assert!(hull.intersects(&aabb, &outside, Vec3A::ZERO, 101.0));
    }
}
