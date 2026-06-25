use bevy::{
    core_pipeline::dof::{DepthOfField, DepthOfFieldMode},
    ecs::{
        bundle::Bundle,
        component::Component,
        event::EventReader,
        query::{With, Without},
        system::{Local, Query, Res, ResMut},
    },
    hierarchy::Parent,
    input::{keyboard::KeyCode, mouse::MouseMotion, ButtonInput},
    math::{Dir3, I64Vec3, Vec3},
    pbr::{DistanceFog, FogFalloff},
    prelude::{default, ClearColor, Color, Transform},
    render::{camera::Camera, view::ColorGrading},
    time::Time,
};

use crate::{block::BlockType, world::World};

#[derive(Bundle, Default)]
pub struct PlayerBundle {
    pub marker: Player,
    pub movement: PlayerMovement,
    pub look: PlayerLook,
    pub transform: Transform,
}

#[derive(Component, Default)]
pub struct Player {}

#[derive(Component)]
pub struct PlayerMovement {
    move_speed: f32,
}

impl Default for PlayerMovement {
    fn default() -> Self {
        Self { move_speed: 20.0 }
    }
}

pub fn player_move(
    time: Res<Time>,
    mut player_query: Query<(&PlayerMovement, &mut Transform)>,
    camera_query: Query<(&Parent, &Transform), (With<Camera>, Without<PlayerMovement>)>,
    keys: Res<ButtonInput<KeyCode>>,
) {
    let (parent, camera_transform) = camera_query.get_single().expect("camera does not exist");
    let (player_movement, player_transform) = &mut player_query
        .get_mut(parent.get())
        .expect("player does not exist");

    let move_speed = player_movement.move_speed;

    let mut movement_vector = Vec3::ZERO;
    if keys.pressed(KeyCode::KeyA) {
        movement_vector.x = -move_speed;
    } else if keys.pressed(KeyCode::KeyD) {
        movement_vector.x = move_speed;
    }

    if keys.pressed(KeyCode::KeyW) {
        movement_vector.z = -move_speed;
    } else if keys.pressed(KeyCode::KeyS) {
        movement_vector.z = move_speed;
    }

    let mut vertical_movement = Vec3::ZERO;
    if keys.pressed(KeyCode::Space) {
        vertical_movement.y = move_speed;
    } else if keys.pressed(KeyCode::ShiftLeft) {
        vertical_movement.y = -move_speed;
    }

    let final_movement =
        player_transform.rotation * camera_transform.rotation * movement_vector * time.delta_secs()
            + (vertical_movement * time.delta_secs());
    player_transform.translation += final_movement;
}

#[derive(Component)]
pub struct PlayerLook {
    sensitivity: f32,
}

impl Default for PlayerLook {
    fn default() -> Self {
        Self { sensitivity: 0.1 }
    }
}

pub fn player_look(
    time: Res<Time>,
    mut player_query: Query<(&PlayerLook, &mut Transform)>,
    mut camera_query: Query<(&Parent, &mut Transform), (With<Camera>, Without<PlayerLook>)>,
    mut motion_evr: EventReader<MouseMotion>,
) {
    let (parent, camera_transform) = &mut camera_query
        .get_single_mut()
        .expect("camera does not exist");
    let (player_look, player_transform) = &mut player_query
        .get_mut(parent.get())
        .expect("player does not exist");

    for ev in motion_evr.read() {
        player_transform.rotate_axis(
            Dir3::Y,
            -ev.delta.x * player_look.sensitivity * time.delta_secs(),
        );
        camera_transform.rotate_axis(
            Dir3::X,
            -ev.delta.y * player_look.sensitivity * time.delta_secs(),
        );
    }
}

// Snapshots whichever DistanceFog/DepthOfField/ClearColor were active right before the camera
// entered water, and restores that exact snapshot on exit, instead of hardcoding "dry" defaults
// that could drift out of sync with setup_scene's render-distance-derived fog settings.
pub fn update_underwater_effects(
    mut world: ResMut<World>,
    mut clear_color: ResMut<ClearColor>,
    player_query: Query<&Transform, With<Player>>,
    mut camera_query: Query<
        (
            &Parent,
            &Transform,
            &mut DistanceFog,
            &mut DepthOfField,
            &mut ColorGrading,
        ),
        With<Camera>,
    >,
    mut dry_defaults: Local<Option<(DistanceFog, DepthOfField, Color)>>,
) {
    let (parent, camera_transform, mut fog, mut dof, mut color_grading) =
        camera_query.get_single_mut().expect("camera does not exist");
    let player_transform = player_query.get(parent.get()).expect("player does not exist");

    let world_pos = player_transform.translation + camera_transform.translation;
    let block_coord = I64Vec3::new(
        world_pos.x.floor() as i64,
        world_pos.y.floor() as i64,
        world_pos.z.floor() as i64,
    );

    if world.get_block_at(block_coord) == BlockType::Water {
        if dry_defaults.is_none() {
            *dry_defaults = Some((fog.clone(), *dof, clear_color.0));
        }
        // Matches ClearColor to the fog color, same trick setup_scene uses above water (its
        // ClearColor equals the dry fog's color) - otherwise empty/unloaded space shows the old
        // background color straight through, breaking the underwater illusion.
        let underwater_color = Color::srgb_u8(20, 90, 130);
        *fog = DistanceFog {
            color: underwater_color,
            falloff: FogFalloff::Linear {
                start: 0.0,
                end: 32.0,
            },
            ..default()
        };
        *dof = DepthOfField {
            mode: DepthOfFieldMode::Gaussian,
            focal_distance: 1.5,
            aperture_f_stops: 0.5,
            ..default()
        };
        color_grading.global.temperature = -0.4;
        color_grading.global.post_saturation = 0.85;
        clear_color.0 = underwater_color;
    } else if let Some((dry_fog, dry_dof, dry_clear_color)) = dry_defaults.take() {
        *fog = dry_fog;
        *dof = dry_dof;
        *color_grading = ColorGrading::default();
        clear_color.0 = dry_clear_color;
    }
}
