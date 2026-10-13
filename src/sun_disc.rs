use bevy::{
    camera::visibility::NoFrustumCulling,
    light::{NotShadowCaster, NotShadowReceiver},
    prelude::*,
    reflect::TypePath,
    render::render_resource::AsBindGroup,
    shader::ShaderRef,
};

/// Draws the sun in the sky wherever the first directional light is, see `sun.wesl`.
pub struct SunDiscPlugin;

impl Plugin for SunDiscPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<SunDiscMaterial>::default())
            .add_systems(Startup, spawn_sun_disc);
    }
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct SunDiscMaterial {}

impl Material for SunDiscMaterial {
    fn enable_prepass() -> bool {
        false
    }

    fn enable_shadows() -> bool {
        false
    }

    fn vertex_shader() -> ShaderRef {
        "shaders/sun.wesl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "shaders/sun.wesl".into()
    }

    // drawn before the clouds (1e6) so they blend over it
    fn depth_bias(&self) -> f32 {
        1.0e7
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Add
    }
}

fn spawn_sun_disc(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<SunDiscMaterial>>,
) {
    commands.spawn((
        Mesh3d(meshes.add(Rectangle::new(2.0, 2.0))),
        MeshMaterial3d(materials.add(SunDiscMaterial {})),
        Transform::default(),
        // the vertex shader places it, so its mesh bounds mean nothing
        NoFrustumCulling,
        NotShadowCaster,
        NotShadowReceiver,
    ));
}
