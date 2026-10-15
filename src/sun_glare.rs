use bevy::{
    asset::Asset,
    camera::visibility::NoFrustumCulling,
    light::{NotShadowCaster, NotShadowReceiver},
    pbr::{MaterialPipeline, MaterialPipelineKey},
    prelude::*,
    reflect::TypePath,
    render::{
        mesh::MeshVertexBufferLayoutRef,
        render_resource::{AsBindGroup, RenderPipelineDescriptor, SpecializedMeshPipelineError},
    },
    shader::ShaderRef,
};

/// The sun's glare over the sky, see `sun_glare.wesl`. A sphere hung round the camera, drawn
/// only where the sky shows.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone, Default)]
pub struct SunGlareMaterial {}

impl Material for SunGlareMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/sun_glare.wesl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "shaders/sun_glare.wesl".into()
    }

    // over the sky, which is drawn after the opaque meshes
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Premultiplied
    }

    fn enable_prepass() -> bool {
        false
    }

    fn enable_shadows() -> bool {
        false
    }

    // seen from inside
    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

pub struct SunGlarePlugin;

impl Plugin for SunGlarePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<SunGlareMaterial>::default())
            // once the camera's spawned
            .add_systems(PostStartup, spawn_sun_glare);
    }
}

fn spawn_sun_glare(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<SunGlareMaterial>>,
    camera: Query<Entity, With<Camera3d>>,
) {
    let Ok(camera) = camera.single() else {
        return;
    };
    commands.spawn((
        // any size: it's drawn as if infinitely far
        Mesh3d(meshes.add(Sphere::new(1.0).mesh().ico(5).unwrap())),
        MeshMaterial3d(materials.add(SunGlareMaterial::default())),
        Transform::default(),
        NotShadowCaster,
        NotShadowReceiver,
        // it's round the camera, so always in view
        NoFrustumCulling,
        ChildOf(camera),
    ));
}
