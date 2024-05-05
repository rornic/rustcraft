use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use bevy::{
    asset::{AssetServer, Assets},
    color::Color,
    ecs::{
        component::Component,
        entity::Entity,
        system::{Commands, Query, Res, ResMut},
    },
    log::info,
    math::{Quat, Vec3},
    pbr::{PbrBundle, StandardMaterial},
    prelude::default,
    render::{
        mesh::Mesh, primitives::Aabb, render_resource::Face, texture::Image, view::NoFrustumCulling,
    },
    transform::components::{GlobalTransform, Transform},
};
use cgmath::{InnerSpace, One, Quaternion, Vector2, Vector3, Zero};
use specs::{prelude::*, rayon::prelude::IntoParallelRefIterator};

use crate::{
    vector2, vector3,
    world::{Chunk, World, CHUNK_SIZE, WORLD_HEIGHT},
};

const GENERATE_DISTANCE: u32 = 8;

pub fn generate_chunks(mut query: Query<&mut World>) {
    let world = &mut query.get_single_mut().expect("could not find single world");

    let spawn = world.spawn();
    let camera_chunk = world.world_to_chunk(spawn);

    let mut chunks: Vec<Vector2<i32>> = all_chunks(camera_chunk, GENERATE_DISTANCE)
        .filter(|chunk| !world.is_chunk_generated(*chunk))
        .collect();
    chunks.sort_by(|c1, c2| {
        chunk_distance(camera_chunk, *c1).total_cmp(&chunk_distance(camera_chunk, *c2))
    });

    let generated_chunks = chunks
        .iter()
        .take(32)
        .collect::<Vec<&Vector2<i32>>>()
        .par_iter()
        .map(|chunk| (**chunk, world.generate_chunk(**chunk)))
        .collect::<Vec<(Vector2<i32>, Chunk)>>();

    for chunk in generated_chunks {
        world.cache_chunk(chunk.0, chunk.1);
    }
}

pub fn load_chunks(
    mut commands: Commands,
    mut world_query: Query<&mut World>,
    mut chunk_loader_query: Query<&mut ChunkLoader>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    asset_server: Res<AssetServer>,
) {
    let world = &mut world_query
        .get_single_mut()
        .expect("could not find single world");
    let chunk_loader = &mut chunk_loader_query
        .get_single_mut()
        .expect("could not find single chunk loader");

    let spawn = world.spawn();
    let camera_chunk = world.world_to_chunk(spawn);

    let all_chunks = all_chunks(camera_chunk, chunk_loader.render_distance)
        .filter(|chunk| world.is_chunk_generated(*chunk))
        .filter(|chunk| world.are_neighbours_generated(*chunk))
        .collect::<HashSet<Vector2<i32>>>();

    // Unload old chunks
    // for chunk in chunk_loader
    //     .active_chunks
    //     .difference(&all_chunks)
    //     .cloned()
    //     .collect::<Vec<Vector2<i32>>>()
    // {
    //     let e = chunk_loader.chunk_entities.remove(&chunk).unwrap();
    //     commands.entity(e).despawn();
    //     chunk_loader.active_chunks.remove(&chunk);
    //     chunk_loader.chunk_meshes.remove(&chunk);
    // }

    // Re-mesh dirty chunks
    // for chunk in chunk_loader
    //     .active_chunks
    //     .iter()
    //     .cloned()
    //     .filter(|c| world.chunk(*c).unwrap().dirty)
    //     .collect::<Vec<Vector2<i32>>>()
    // {
    //     chunk_loader.chunk_meshes.remove(&chunk);
    //     let entity = chunk_loader.chunk_entities.get(&chunk).unwrap();
    //     let new_mesh = world.generate_chunk_mesh(chunk);
    //     render_meshes.get_mut(*entity).unwrap().mesh = Arc::new(new_mesh);
    //     game_world.clear_chunk_dirty_bit(chunk);
    // }

    // Load new chunks
    let mut to_load = all_chunks
        .difference(&chunk_loader.active_chunks)
        .cloned()
        .collect::<Vec<Vector2<i32>>>();
    // to_load.sort_by(|c1, c2| {
    //     chunk_camera_direction(camera_chunk, camera.look_direction(), *c1).total_cmp(
    //         &chunk_camera_direction(camera_chunk, camera.look_direction(), *c2),
    //     )
    // });

    let new_meshes = to_load
        .iter()
        .cloned()
        .filter(|chunk| !chunk_loader.chunk_meshes.contains_key(chunk))
        .collect::<Vec<Vector2<i32>>>()
        .par_iter()
        .map(|chunk| (*chunk, world.generate_chunk_mesh(*chunk)))
        .collect::<Vec<(Vector2<i32>, Mesh)>>();
    for (chunk, mesh) in new_meshes {
        let (t, aabb) = chunk_components(chunk);
        chunk_loader.active_chunks.insert(chunk);
        let entity = commands
            .spawn((
                PbrBundle {
                    mesh: meshes.add(mesh),
                    material: materials.add(StandardMaterial {
                        base_color: Color::WHITE,
                        base_color_texture: Some(asset_server.load::<Image>("textures/blocks.png")),
                        reflectance: 0.0,
                        cull_mode: Some(Face::Front),
                        ..default()
                    }),
                    transform: t,
                    ..default()
                },
                aabb,
            ))
            .id();
        chunk_loader.chunk_entities.insert(chunk, entity);
    }

    // for chunk in to_load {
    //     chunk_loader.active_chunks.insert(chunk);

    //     if let Some(e) = chunk_loader.chunk_entities.get(&chunk) {
    //         // render_meshes.get_mut(*e).unwrap().visible = true;
    //         continue;
    //     }

    //     let mesh = chunk_loader.chunk_meshes.get(&chunk).unwrap();
    // }
}

#[derive(Component)]
pub struct ChunkLoader {
    render_distance: u32,
    active_chunks: HashSet<Vector2<i32>>,
    chunk_meshes: HashMap<Vector2<i32>, Arc<Mesh>>,
    chunk_entities: HashMap<Vector2<i32>, Entity>,
}

impl ChunkLoader {
    pub fn new(render_distance: u32) -> Self {
        Self {
            render_distance,
            active_chunks: HashSet::new(),
            chunk_meshes: HashMap::new(),
            chunk_entities: HashMap::new(),
        }
    }
}

fn all_chunks(centre: Vector2<i32>, distance: u32) -> impl Iterator<Item = Vector2<i32>> {
    let (x_min, x_max) = (centre.x - distance as i32, centre.x + distance as i32);
    let (z_min, z_max) = (centre.y - distance as i32, centre.y + distance as i32);
    (x_min..x_max).flat_map(move |a| (z_min..z_max).map(move |b| vector2!(a, b)))
}

fn chunk_distance(chunk1: Vector2<i32>, chunk2: Vector2<i32>) -> f32 {
    (((chunk2.x - chunk1.x).abs().pow(2) + (chunk2.y - chunk1.y).abs().pow(2)) as f32).sqrt()
}

fn chunk_camera_direction(
    camera_chunk: Vector2<i32>,
    camera_forward: Vector3<f32>,
    chunk: Vector2<i32>,
) -> f32 {
    let camera_dir = (chunk_world_pos(camera_chunk) - chunk_world_pos(chunk)).normalize();
    let dot = camera_forward.dot(camera_dir);
    let dist = chunk_distance(camera_chunk, chunk) as f32;
    if dist.is_zero() {
        return -f32::INFINITY;
    }
    dot / dist
}

fn chunk_world_pos(chunk: Vector2<i32>) -> Vector3<f32> {
    vector3!(
        (chunk.x * CHUNK_SIZE as i32) as f32,
        0.0,
        (chunk.y * CHUNK_SIZE as i32) as f32
    )
}

fn chunk_components(chunk: Vector2<i32>) -> (Transform, Aabb) {
    let pos = chunk_world_pos(chunk);
    let t = Transform::from_translation(Vec3::new(pos.x, pos.y, pos.z));
    let aabb = Aabb::from_min_max(
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(CHUNK_SIZE as f32, WORLD_HEIGHT as f32, CHUNK_SIZE as f32),
    );
    (t, aabb)
}

#[cfg(test)]
mod tests {
    use crate::{vector2, vector3};

    use super::chunk_camera_direction;

    #[test]
    fn test_chunk_sorting() {
        let mut chunks = vec![
            vector2!(-5, 5),
            vector2!(-1, 0),
            vector2!(0, 0),
            vector2!(1, 0),
            vector2!(1, 1),
            vector2!(5, 0),
        ];
        let camera_chunk = vector2!(0, 0);
        let camera_dir = vector3!(1.0, 0.0, 0.0);
        chunks.sort_by(|c1, c2| {
            chunk_camera_direction(camera_chunk, camera_dir, *c1)
                .total_cmp(&chunk_camera_direction(camera_chunk, camera_dir, *c2))
        });

        assert_eq!(chunks[0], vector2!(0, 0));
        assert_eq!(chunks[1], vector2!(1, 0));
    }
}
