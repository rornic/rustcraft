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
        storage::ShaderBuffer,
    },
    shader::ShaderRef,
};

use crate::chunks::material::{push_shader_defs, water_level};
use crate::curvature::Curvature;

/// The glow along the horizon where the land curves away below it, drawn over the sky, see
/// `horizon.wesl`. A sphere hung round the camera, drawn only where the sky shows.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct HorizonGlowMaterial {
    // where the horizon is, see `curvature::Curvature`
    #[storage(8, read_only)]
    curvature: Handle<ShaderBuffer>,
}

impl Material for HorizonGlowMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/horizon_glow.wesl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "shaders/horizon_glow.wesl".into()
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
        push_shader_defs(descriptor, &[water_level()]);
        Ok(())
    }
}

pub struct HorizonGlowPlugin;

impl Plugin for HorizonGlowPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<HorizonGlowMaterial>::default())
            // once the camera's spawned
            .add_systems(PostStartup, spawn_horizon_glow);
    }
}

fn spawn_horizon_glow(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<HorizonGlowMaterial>>,
    curvature: Res<Curvature>,
    camera: Query<Entity, With<Camera3d>>,
) {
    let Ok(camera) = camera.single() else {
        return;
    };
    let material = materials.add(HorizonGlowMaterial { curvature: curvature.buffer.clone() });
    commands.spawn((
        // any size: it's drawn as if infinitely far
        Mesh3d(meshes.add(Sphere::new(1.0).mesh().ico(4).unwrap())),
        MeshMaterial3d(material),
        Transform::default(),
        NotShadowCaster,
        NotShadowReceiver,
        // it's round the camera, so always in view
        NoFrustumCulling,
        ChildOf(camera),
    ));
}
