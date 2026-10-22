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

/// Sorts the sky domes before every other blended thing, each pinned to the far plane so it draws only where nothing else is.
pub const SKY_DEPTH_BIAS: f32 = -2.0e9;

// A material drawn over the sky by a sphere hung round the camera, and the plugin spawning it.
macro_rules! sky_dome {
    ($(#[$doc:meta])* $material:ident, $plugin:ident, $shader:literal) => {
        $(#[$doc])*
        #[derive(Asset, TypePath, AsBindGroup, Debug, Clone, Default)]
        pub struct $material {}

        impl Material for $material {
            fn vertex_shader() -> ShaderRef {
                $shader.into()
            }

            fn fragment_shader() -> ShaderRef {
                $shader.into()
            }

            // over the sky, which is drawn after the opaque meshes
            fn alpha_mode(&self) -> AlphaMode {
                AlphaMode::Premultiplied
            }

            fn depth_bias(&self) -> f32 {
                SKY_DEPTH_BIAS
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

        pub struct $plugin;

        impl Plugin for $plugin {
            fn build(&self, app: &mut App) {
                app.add_plugins(MaterialPlugin::<$material>::default())
                    // once the camera's spawned
                    .add_systems(PostStartup, spawn_dome::<$material>);
            }
        }
    };
}

sky_dome!(
    /// The sun's glare over the sky, see `sun_glare.wesl`.
    SunGlareMaterial,
    SunGlarePlugin,
    "shaders/sun_glare.wesl"
);

sky_dome!(
    /// The night's stars, the Milky Way and the moon over the sky, see `night_sky.wesl`.
    NightSkyMaterial,
    NightSkyPlugin,
    "shaders/night_sky.wesl"
);

fn spawn_dome<M: Material + Default>(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<M>>,
    camera: Query<Entity, With<Camera3d>>,
) {
    let Ok(camera) = camera.single() else {
        return;
    };
    commands.spawn(dome(&mut meshes, materials.add(M::default()), camera));
}

/// A sphere hung round `camera`, drawing `material` over the sky.
pub fn dome<M: Material>(
    meshes: &mut Assets<Mesh>,
    material: Handle<M>,
    camera: Entity,
) -> impl Bundle {
    (
        // any size: it's drawn as if infinitely far
        Mesh3d(meshes.add(Sphere::new(1.0).mesh().ico(5).unwrap())),
        MeshMaterial3d(material),
        Transform::default(),
        NotShadowCaster,
        NotShadowReceiver,
        // it's round the camera, so always in view
        NoFrustumCulling,
        ChildOf(camera),
    )
}
