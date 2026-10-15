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

/// The night's stars, the Milky Way and the moon over the sky, see `night_sky.wesl`. A sphere
/// hung round the camera, drawn only where the sky shows.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone, Default)]
pub struct NightSkyMaterial {}

impl Material for NightSkyMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/night_sky.wesl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "shaders/night_sky.wesl".into()
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

pub struct NightSkyPlugin;

impl Plugin for NightSkyPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<NightSkyMaterial>::default())
            // once the camera's spawned
            .add_systems(PostStartup, spawn_night_sky);
    }
}

fn spawn_night_sky(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<NightSkyMaterial>>,
    camera: Query<Entity, With<Camera3d>>,
) {
    let Ok(camera) = camera.single() else {
        return;
    };
    commands.spawn((
        // any size: it's drawn as if infinitely far
        Mesh3d(meshes.add(Sphere::new(1.0).mesh().ico(5).unwrap())),
        MeshMaterial3d(materials.add(NightSkyMaterial::default())),
        Transform::default(),
        NotShadowCaster,
        NotShadowReceiver,
        // it's round the camera, so always in view
        NoFrustumCulling,
        ChildOf(camera),
    ));
}
