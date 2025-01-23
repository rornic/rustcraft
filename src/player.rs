use bevy::{
    ecs::{
        bundle::Bundle,
        component::Component,
        event::EventReader,
        query::{With, Without},
        system::{Query, Res},
    },
    hierarchy::Parent,
    input::{keyboard::KeyCode, mouse::MouseMotion, ButtonInput},
    math::{Dir3, Vec3},
    prelude::Transform,
    render::camera::Camera,
    time::Time,
};

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
