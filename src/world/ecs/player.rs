use bevy::{
    ecs::{
        bundle::Bundle,
        component::Component,
        query::{With, Without},
        system::{Query, Res},
    },
    hierarchy::Parent,
    input::{keyboard::KeyCode, ButtonInput},
    math::Vec3,
    render::camera::{self, Camera},
    time::Time,
    transform::{components::Transform, TransformBundle},
};

#[derive(Bundle, Default)]
pub struct PlayerBundle {
    pub movement: PlayerMovement,
    pub transform_bundle: TransformBundle,
}

#[derive(Component)]
pub struct PlayerMovement {
    move_speed: f32,
}

impl Default for PlayerMovement {
    fn default() -> Self {
        Self { move_speed: 5.0 }
    }
}

pub fn move_player(
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

    let mut movement_vector = Vec3::new(0.0, 0.0, 0.0);
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

    // movement_vector = Quaternion::from(Euler {
    //     x: Deg(0.0),
    //     y: 0.0,
    //     z: Deg(0.0),
    // }) * movement_vector;

    let final_movement = player_transform.rotation * movement_vector * time.delta_seconds();
    player_transform.translation += final_movement;

    // rigidbody.velocity.x = movement_vector.x;
    // rigidbody.velocity.z = movement_vector.z;

    // if input.keyboard.is_pressed(VirtualKeyCode::Space) && rigidbody.is_grounded() {
    //     rigidbody.velocity.y = 4.0;
    // }
}

// #[derive(Default)]
// pub struct PlayerBlockBreak {}

// impl<'a> System<'a> for PlayerBlockBreak {
//     type SystemData = (
//         ReadStorage<'a, Transform>,
//         ReadStorage<'a, Camera>,
//         Write<'a, World>,
//         Read<'a, Input>,
//     );

//     fn run(&mut self, (transforms, cameras, mut world, input): Self::SystemData) {
//         for (transform, camera) in (&transforms, &cameras).join() {
//             if input.mouse.is_left_pressed() {
//                 if let Some(hit) = physics::raycast::block_aligned_raycast(
//                     &world,
//                     transform.position,
//                     camera.look_direction(),
//                     5.0,
//                 ) {
//                     world.set_block_at(hit.position, BlockType::Air);
//                 }
//             }
//         }
//     }
// }
