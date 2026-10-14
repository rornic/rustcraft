use std::sync::LazyLock;

use bevy::{
    ecs::{
        bundle::Bundle,
        component::Component,
        entity::Entity,
        message::MessageReader,
        query::{With, Without},
        system::{Commands, Local, Query, Res, ResMut},
    },
    ecs::hierarchy::ChildOf,
    input::{keyboard::KeyCode, mouse::MouseMotion, ButtonInput},
    math::{Dir3, I64Vec3, Vec3},
    light::AtmosphereEnvironmentMapLight,
    pbr::{AtmosphereSettings, DistanceFog, FogFalloff},
    prelude::{default, ClearColor, Color, Transform},
    camera::Camera,
    post_process::dof::{DepthOfField, DepthOfFieldMode},
    render::view::ColorGrading,
    time::Time,
};

use crate::{block::BlockType, tonemap::scene_colour, world::World};

#[derive(Bundle, Default)]
pub struct PlayerBundle {
    pub marker: Player,
    pub movement: PlayerMovement,
    pub look: PlayerLook,
    pub transform: Transform,
}

#[derive(Component, Default)]
pub struct Player {}

// How much faster the camera moves while Shift is held.
const SPRINT_MULTIPLIER: f32 = 4.0;

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
    camera_query: Query<(&ChildOf, &Transform), (With<Camera>, Without<PlayerMovement>)>,
    keys: Res<ButtonInput<KeyCode>>,
) {
    let (parent, camera_transform) = camera_query.single().expect("camera does not exist");
    let (player_movement, player_transform) = &mut player_query
        .get_mut(parent.parent())
        .expect("player does not exist");

    let move_speed = if keys.pressed(KeyCode::ShiftLeft) {
        player_movement.move_speed * SPRINT_MULTIPLIER
    } else {
        player_movement.move_speed
    };

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
    } else if keys.pressed(KeyCode::ControlLeft) {
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
    mut camera_query: Query<(&ChildOf, &mut Transform), (With<Camera>, Without<PlayerLook>)>,
    mut motion_evr: MessageReader<MouseMotion>,
) {
    let (parent, camera_transform) = &mut camera_query
        .single_mut()
        .expect("camera does not exist");
    let (player_look, player_transform) = &mut player_query
        .get_mut(parent.parent())
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

pub struct DryDefaults {
    fog: DistanceFog,
    dof: DepthOfField,
    color_grading: ColorGrading,
    clear_color: Color,
    atmosphere: Option<AtmosphereSettings>,
    environment: Option<AtmosphereEnvironmentMapLight>,
}

// Snapshots whichever DistanceFog/DepthOfField/ClearColor/atmosphere were active right before the camera
// entered water, and restores that exact snapshot on exit, instead of hardcoding "dry" defaults
// that could drift out of sync with setup_scene's render-distance-derived fog settings.
#[allow(clippy::type_complexity)]
pub fn update_underwater_effects(
    mut commands: Commands,
    mut world: ResMut<World>,
    mut clear_color: ResMut<ClearColor>,
    player_query: Query<&Transform, With<Player>>,
    mut camera_query: Query<
        (
            Entity,
            &ChildOf,
            &Transform,
            &mut DistanceFog,
            &mut DepthOfField,
            &mut ColorGrading,
            Option<&AtmosphereSettings>,
            Option<&AtmosphereEnvironmentMapLight>,
        ),
        With<Camera>,
    >,
    mut dry_defaults: Local<Option<DryDefaults>>,
) {
    let (camera, parent, camera_transform, mut fog, mut dof, mut color_grading, atmosphere, environment) =
        camera_query.single_mut().expect("camera does not exist");
    let player_transform = player_query.get(parent.parent()).expect("player does not exist");

    let world_pos = player_transform.translation + camera_transform.translation;
    let block_coord = I64Vec3::new(
        world_pos.x.floor() as i64,
        world_pos.y.floor() as i64,
        world_pos.z.floor() as i64,
    );

    if world.get_block_at(block_coord) == BlockType::Water {
        if dry_defaults.is_none() {
            *dry_defaults = Some(DryDefaults {
                fog: fog.clone(),
                dof: *dof,
                color_grading: color_grading.clone(),
                clear_color: clear_color.0,
                atmosphere: atmosphere.cloned(),
                environment: environment.cloned(),
            });
            // the sky isn't fogged, so it's taken off to show the underwater colour
            commands.entity(camera).remove::<(AtmosphereSettings, AtmosphereEnvironmentMapLight)>();
        }
        // Matches ClearColor to the fog color, same trick setup_scene uses above water (its
        // ClearColor equals the dry fog's color) - otherwise empty/unloaded space shows the old
        // background color straight through, breaking the underwater illusion.
        static UNDERWATER_COLOR: LazyLock<Color> = LazyLock::new(|| scene_colour(Color::srgb_u8(20, 90, 130)).into());
        let underwater_color = *UNDERWATER_COLOR;
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
        // The fog's colour is already the water's: a white balance shift on top, like the
        // -0.4 this once had, moves the white point off the chromaticity diagram, where Bevy's
        // balance matrix turns blue into olive.
        color_grading.global.post_saturation = 0.85;
        clear_color.0 = underwater_color;
    } else if let Some(dry) = dry_defaults.take() {
        *fog = dry.fog;
        *dof = dry.dof;
        *color_grading = dry.color_grading;
        clear_color.0 = dry.clear_color;
        if let Some(atmosphere) = dry.atmosphere {
            commands.entity(camera).insert(atmosphere);
        }
        if let Some(environment) = dry.environment {
            commands.entity(camera).insert(environment);
        }
    }
}
