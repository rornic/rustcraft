use bevy::{
    asset::{Asset, Assets},
    color::{Color, LinearRgba},
    ecs::{
        component::Component,
        query::{With, Without},
        system::{Commands, Query},
    },
    math::{primitives::Sphere, Vec3},
    pbr::{Material, MaterialPipeline, MaterialPipelineKey, MeshMaterial3d},
    prelude::Mesh3d,
    reflect::TypePath,
    render::{
        camera::Camera,
        mesh::{Mesh, MeshVertexBufferLayoutRef},
        render_resource::{
            AsBindGroup, RenderPipelineDescriptor, ShaderRef, SpecializedMeshPipelineError,
        },
        view::NoFrustumCulling,
    },
    transform::components::{GlobalTransform, Transform},
};

/// The sky at the horizon. Fog and the background use it too, so distant terrain
/// fades into the bottom of the sky.
pub const HORIZON_COLOR: Color = Color::srgb(165.0 / 255.0, 200.0 / 255.0, 235.0 / 255.0);
/// The sky straight overhead.
pub const ZENITH_COLOR: Color = Color::srgb(60.0 / 255.0, 110.0 / 255.0, 200.0 / 255.0);

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct SkyMaterial {
    #[uniform(0)]
    pub horizon: LinearRgba,
    #[uniform(1)]
    pub zenith: LinearRgba,
}

impl Material for SkyMaterial {
    fn fragment_shader() -> ShaderRef {
        "shaders/sky.wgsl".into()
    }

    // the camera is inside the sphere, looking at its back faces
    fn specialize(
        _pipeline: &MaterialPipeline<Self>,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

/// The sphere the sky is drawn on, kept centred on the camera.
#[derive(Component)]
pub struct Sky;

/// Spawns the sky as a sphere of `radius` blocks, which must sit between the furthest
/// terrain and the camera's far plane.
pub fn spawn_sky(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<SkyMaterial>,
    radius: f32,
) {
    commands.spawn((
        Sky,
        Mesh3d(meshes.add(Sphere::new(1.0))),
        MeshMaterial3d(materials.add(SkyMaterial {
            horizon: HORIZON_COLOR.into(),
            zenith: ZENITH_COLOR.into(),
        })),
        Transform::from_scale(Vec3::splat(radius)),
        NoFrustumCulling,
    ));
}

// Uses last frame's camera position, which is close enough at the sky's distance.
pub fn follow_camera(
    camera: Query<&GlobalTransform, (With<Camera>, Without<Sky>)>,
    mut sky: Query<&mut Transform, With<Sky>>,
) {
    let (Ok(camera), Ok(mut sky)) = (camera.get_single(), sky.get_single_mut()) else {
        return;
    };
    sky.translation = camera.translation();
}
