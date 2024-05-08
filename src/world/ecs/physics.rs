use crate::world::World;

const GRAVITY: f32 = -9.8;

// pub struct Rigidbody {
//     pub velocity: Vector3<f32>,
//     apply_gravity: bool,
//     grounded: bool,
// }

// impl Component for Rigidbody {
//     type Storage = VecStorage<Self>;
// }

// impl Default for Rigidbody {
//     fn default() -> Self {
//         Self {
//             velocity: Vector3::zero(),
//             apply_gravity: true,
//             grounded: false,
//         }
//     }
// }

// impl Rigidbody {
//     pub fn is_grounded(&self) -> bool {
//         self.grounded
//     }
// }

// pub struct Physics {}

// impl Physics {
//     pub fn new() -> Self {
//         Self {}
//     }
// }

// impl<'a> System<'a> for Physics {
//     type SystemData = (
//         WriteStorage<'a, Transform>,
//         WriteStorage<'a, Rigidbody>,
//         ReadStorage<'a, Bounds>,
//         Read<'a, DeltaTime>,
//         Read<'a, World>,
//     );

//     fn run(
//         &mut self,
//         (mut transforms, mut rigidbodies, bounds, delta_time, world): Self::SystemData,
//     ) {
//         for (transform, rigidbody, bounds) in (&mut transforms, &mut rigidbodies, &bounds).join() {
//             let bottom_block = world.block_centre(bounds.to_world(transform.position).bottom());

//             let new_position = transform.position
//                 + vector3!(rigidbody.velocity.x, 0.0, rigidbody.velocity.z) * delta_time.0;
//             if collides_with_block(
//                 bottom_block + vector3!(rigidbody.velocity.x.signum(), 0.0, 0.0),
//                 bounds.to_world(new_position),
//                 &world,
//             ) {
//                 rigidbody.velocity.x = 0.0;
//             }

//             if collides_with_block(
//                 bottom_block + vector3!(0.0, 0.0, rigidbody.velocity.z.signum()),
//                 bounds.to_world(new_position),
//                 &world,
//             ) {
//                 rigidbody.velocity.z = 0.0;
//             }

//             if !rigidbody.grounded && rigidbody.apply_gravity {
//                 rigidbody.velocity += vector3!(0.0, GRAVITY, 0.0f32) * delta_time.0;
//             }

//             rigidbody.grounded = collides_with_block(
//                 bottom_block + vector3!(0.0, -0.1, 0.0),
//                 bounds.to_world(new_position),
//                 &world,
//             );
//             if rigidbody.velocity.y < 0.0 && rigidbody.grounded {
//                 rigidbody.velocity.y = 0.0;
//             }

//             transform.position += rigidbody.velocity * delta_time.0;
//         }
//     }
// }

// fn collides_with_block(block: Vector3<f32>, bounds: Bounds, world: &World) -> bool {
//     world.block_at(block).is_solid()
//         && bounds.intersects(Bounds::new(block, vector3!(1.0, 1.0, 1.0)))
// }

// pub mod raycast {
//     use cgmath::{InnerSpace, Vector3};

//     use crate::{
//         vector3,
//         world::{BlockType, World},
//     };

//     pub struct RaycastHit {
//         pub block: BlockType,
//         pub position: Vector3<f32>,
//     }

//     // Implemented according to "A Fast Voxel Traversal Algorithm for Ray Tracing" (Amanatides, Woo)
//     pub fn block_aligned_raycast(
//         world: &World,
//         origin: Vector3<f32>,
//         dir: Vector3<f32>,
//         max_blocks: f32,
//     ) -> Option<RaycastHit> {
//         let dir = dir.normalize();
//         let (t_delta_x, t_delta_y, t_delta_z) =
//             (1.0 / dir.x.abs(), 1.0 / dir.y.abs(), 1.0 / dir.z.abs());

//         let end = origin + dir * max_blocks;
//         let (x_out, y_out, z_out) = (end.x.ceil(), end.y.ceil(), end.z.ceil());

//         let (mut x, step_x, mut t_max_x) = calculate_raycast_params(origin.x, dir.x, max_blocks);
//         let (mut y, step_y, mut t_max_y) = calculate_raycast_params(origin.y, dir.y, max_blocks);
//         let (mut z, step_z, mut t_max_z) = calculate_raycast_params(origin.z, dir.z, max_blocks);

//         while x != x_out || y != y_out || z != z_out {
//             if t_max_x < t_max_y && t_max_x < t_max_z {
//                 x += step_x;
//                 t_max_x += t_delta_x;
//             } else if t_max_y < t_max_z {
//                 y += step_y;
//                 t_max_y += t_delta_y;
//             } else {
//                 z += step_z;
//                 t_max_z += t_delta_z;
//             }

//             let pos = vector3!(x, y, z);
//             let block = world.block_at(pos);
//             if block.is_solid() {
//                 return Some(RaycastHit {
//                     block: block,
//                     position: pos,
//                 });
//             }
//         }
//         None
//     }

//     fn calculate_raycast_params(origin: f32, dir: f32, max: f32) -> (f32, f32, f32) {
//         let p = origin.ceil();
//         let (step, t_max) = if dir > 0.0 {
//             (1.0, (p - origin) / dir)
//         } else if dir < 0.0 {
//             (-1.0, (p - 1.0 - origin) / dir)
//         } else {
//             (0.0, max)
//         };
//         (p, step, t_max)
//     }
// }
