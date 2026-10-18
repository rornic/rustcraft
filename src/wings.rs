//! Flapping wings for the butterflies: a flat wing, a winged body to spawn, and its beat.
use std::f32::consts::TAU;

use bevy::{
    asset::RenderAssetUsages,
    light::NotShadowCaster,
    math::ops,
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
};

/// Beats every wing, added by whichever of the creatures' plugins comes first.
pub struct WingPlugin;

impl Plugin for WingPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, flap_wings);
    }
}

/// How a wing beats: how fast, in radians a second, how far either way, held how far up, and
/// whether it now and then glides with its wings out, as birds do.
#[derive(Clone, Copy)]
pub struct Flap {
    pub beat: f32,
    pub sweep: f32,
    pub lift: f32,
    pub glides: bool,
}

#[derive(Component)]
struct Wing {
    side: f32,
    flap: Flap,
    phase: f32,
}

/// A flat wing from its corners: at the body and at the tip, along the leading edge and then
/// the trailing one. It's mirrored for the other side, and carries uvs across it for a texture.
pub fn wing_mesh(corners: [[f32; 3]; 4]) -> Mesh {
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, corners.to_vec())
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 1.0, 0.0]; 4])
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_UV_0,
        vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]],
    )
    .with_inserted_indices(Indices::U32(vec![0, 2, 1, 1, 2, 3]))
}

/// Spawns `creature` at `at`, unseen until it's grown, with a body and a pair of wings.
pub fn spawn_winged(
    commands: &mut Commands,
    creature: impl Bundle,
    at: Vec3,
    body: (&Handle<Mesh>, &Handle<StandardMaterial>),
    wing: (&Handle<Mesh>, &Handle<StandardMaterial>),
    flap: Flap,
) {
    let phase = rand::random_range(0.0..TAU);
    commands
        .spawn((
            creature,
            Transform::from_translation(at).with_scale(Vec3::ZERO),
            Visibility::default(),
        ))
        .with_children(|parent| {
            parent.spawn((
                Mesh3d(body.0.clone()),
                MeshMaterial3d(body.1.clone()),
                Transform::default(),
                NotShadowCaster,
            ));
            for side in [1.0, -1.0] {
                parent.spawn((
                    Wing { side, flap, phase },
                    Mesh3d(wing.0.clone()),
                    MeshMaterial3d(wing.1.clone()),
                    Transform::from_scale(Vec3::new(side, 1.0, 1.0)),
                    NotShadowCaster,
                ));
            }
        });
}

fn flap_wings(time: Res<Time>, mut wings: Query<(&Wing, &mut Transform)>) {
    let t = time.elapsed_secs();
    for (wing, mut transform) in &mut wings {
        let Flap {
            beat,
            sweep,
            lift,
            glides,
        } = wing.flap;
        let glide = if glides {
            ops::sin(t * 0.4 + wing.phase * 3.0).max(0.0).powi(6)
        } else {
            0.0
        };
        let stroke = ops::sin(t * beat + wing.phase) * sweep * (1.0 - glide);
        transform.rotation = Quat::from_rotation_z(wing.side * (lift + stroke));
    }
}
