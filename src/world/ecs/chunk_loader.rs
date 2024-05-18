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
        query::With,
        system::{Commands, Query, Res, ResMut},
    },
    math::{I64Vec3, Vec3},
    pbr::{PbrBundle, StandardMaterial},
    prelude::default,
    render::{mesh::Mesh, primitives::Aabb, render_resource::Face, texture::Image},
    transform::components::Transform,
};

use crate::new_world::{chunk::ChunkCoordinate, world::World};

use super::player::Player;

const GENERATE_DISTANCE: u32 = 8;

pub fn generate_chunks(mut world: ResMut<World>, player_query: Query<&Transform, With<Player>>) {
    let player = player_query.get_single().expect("could not find player");

    let camera_chunk = world.block_to_chunk_coordinate(I64Vec3::new(
        player.translation.x as i64,
        player.translation.y as i64,
        player.translation.z as i64,
    ));

    let mut chunks: Vec<ChunkCoordinate> = all_chunks(camera_chunk, GENERATE_DISTANCE)
        .filter(|chunk| !world.is_chunk_generated(*chunk))
        .collect();
    chunks.sort_by(|c1, c2| {
        chunk_distance(camera_chunk, *c1).total_cmp(&chunk_distance(camera_chunk, *c2))
    });

    let chunks_to_generate: Vec<ChunkCoordinate> = chunks.into_iter().take(8).collect();
    for chunk in chunks_to_generate {
        world.generate_chunk(chunk);
    }
}

#[derive(Component)]
pub struct Chunk {
    coord: ChunkCoordinate,
    dirty: bool,
}

#[derive(Component)]
pub struct ChunkLoader {
    render_distance: u32,
    chunk_meshes: HashMap<ChunkCoordinate, Arc<Mesh>>,
    chunk_entities: HashMap<ChunkCoordinate, Entity>,
}

impl ChunkLoader {
    pub fn new(render_distance: u32) -> Self {
        Self {
            render_distance,
            chunk_meshes: HashMap::new(),
            chunk_entities: HashMap::new(),
        }
    }
}

pub fn load_chunks(
    mut commands: Commands,
    mut world: ResMut<World>,
    mut chunk_loader_query: Query<&mut ChunkLoader>,
    player_query: Query<&Transform, With<Player>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    loaded_chunks: Query<(Entity, &Chunk)>,
    asset_server: Res<AssetServer>,
) {
    let chunk_loader = &mut chunk_loader_query
        .get_single_mut()
        .expect("could not find single chunk loader");

    let player = player_query.get_single().expect("could not find player");
    let camera_chunk = world.block_to_chunk_coordinate(I64Vec3::new(
        player.translation.x as i64,
        player.translation.y as i64,
        player.translation.z as i64,
    ));

    let mut chunks_to_load = all_chunks(camera_chunk, chunk_loader.render_distance)
        .filter(|chunk| world.is_chunk_generated(*chunk))
        // .filter(|chunk| world.are_neighbours_generated(*chunk))
        .collect::<HashSet<ChunkCoordinate>>();

    // Unload old chunks
    for (entity, chunk) in loaded_chunks.iter() {
        let chunk_coords = chunk.coord;
        if chunks_to_load.contains(&chunk_coords) {
            chunks_to_load.remove(&chunk_coords);
        } else {
            commands.entity(entity).despawn();
        }
        chunk_loader.chunk_meshes.remove(&chunk_coords);
    }

    // Re-mesh dirty chunks
    // for chunk in chunk_loader
    //     .active_chunks
    //     .iter()
    //     .cloned()
    //     .filter(|c| world.is_chunk_dirty())
    //     .collect::<Vec<Vector2<i32>>>()
    // {
    //     chunk_loader.chunk_meshes.remove(&chunk);
    //     let entity = chunk_loader.chunk_entities.get(&chunk).unwrap();
    //     let new_mesh = world.generate_chunk_mesh(chunk);
    //     render_meshes.get_mut(*entity).unwrap().mesh = Arc::new(new_mesh);
    //     game_world.clear_chunk_dirty_bit(chunk);
    // }

    let forward = Vec3::new(player.forward().x, player.forward().y, player.forward().z);
    let mut chunks_to_load = chunks_to_load
        .into_iter()
        .filter(|chunk| !chunk_loader.chunk_meshes.contains_key(chunk))
        .collect::<Vec<ChunkCoordinate>>();
    chunks_to_load.sort_by(|c1, c2| {
        chunk_camera_direction(camera_chunk, forward, *c1).total_cmp(&chunk_camera_direction(
            camera_chunk,
            forward,
            *c2,
        ))
    });

    let chunks_to_load: Vec<ChunkCoordinate> = chunks_to_load.into_iter().take(8).collect();
    // let thread_pool = AsyncComputeTaskPool::get();
    // let generated_meshes = chunks_to_load.par_chunk_map(thread_pool, 2, |_index, chunks| {
    //     chunks
    //         .iter()
    //         .map(|chunk| (*chunk, world.generate_chunk_mesh(*chunk)))
    //         .collect::<Vec<(ChunkCoordinate, Mesh)>>()
    // });

    let mut generated_meshes = vec![];
    for chunk in chunks_to_load {
        generated_meshes.push((chunk, world.generate_chunk_mesh(chunk)));
    }

    for (chunk, mesh) in generated_meshes
        .into_iter()
        .filter(|(_, mesh)| mesh.is_some())
        .map(|(coord, mesh)| (coord, mesh.unwrap()))
    {
        let (t, aabb) = chunk_components(chunk);
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
                Chunk {
                    coord: chunk,
                    dirty: false,
                },
            ))
            .id();
        chunk_loader.chunk_entities.insert(chunk, entity);
    }
}

fn all_chunks(centre: ChunkCoordinate, distance: u32) -> impl Iterator<Item = ChunkCoordinate> {
    let (x_min, x_max) = (centre.0.x - distance as i64, centre.0.x + distance as i64);
    let (y_min, y_max) = (
        (centre.0.y - distance as i64).max(0),
        centre.0.y + distance as i64,
    );
    let (z_min, z_max) = (centre.0.z - distance as i64, centre.0.z + distance as i64);

    let mut chunks = vec![];
    for x in x_min..x_max {
        for y in 0..4 {
            for z in z_min..z_max {
                chunks.push(ChunkCoordinate(I64Vec3::new(x, y, z)));
            }
        }
    }
    chunks.into_iter()
}

fn chunk_distance(chunk1: ChunkCoordinate, chunk2: ChunkCoordinate) -> f32 {
    (((chunk2.0.x - chunk1.0.x).abs().pow(2) + (chunk2.0.z - chunk1.0.z).abs().pow(2)) as f32)
        .sqrt()
}

fn chunk_camera_direction(
    camera_chunk: ChunkCoordinate,
    camera_forward: Vec3,
    chunk: ChunkCoordinate,
) -> f32 {
    let camera_dir = (camera_chunk.0 - chunk.0).as_vec3().normalize();
    let dot = camera_forward.dot(camera_dir);
    let dist = chunk_distance(camera_chunk, chunk) as f32;
    if dist == 0.0 {
        return -f32::INFINITY;
    }
    dot / dist
}

fn chunk_world_pos(chunk: ChunkCoordinate) -> Vec3 {
    Vec3::new(
        (chunk.0.x * 16 as i64) as f32,
        (chunk.0.y * 16) as f32,
        (chunk.0.z * 16 as i64) as f32,
    )
}

fn chunk_components(chunk: ChunkCoordinate) -> (Transform, Aabb) {
    let pos = chunk_world_pos(chunk);
    let t = Transform::from_translation(Vec3::new(pos.x, pos.y, pos.z));
    let aabb = Aabb::from_min_max(Vec3::new(0.0, 0.0, 0.0), Vec3::new(16.0, 16.0, 16.0));
    (t, aabb)
}

#[cfg(test)]
mod tests {
    use bevy::math::Vec3;

    use crate::world::ChunkCoordinate;

    use super::chunk_camera_direction;

    // #[test]
    // fn test_chunk_sorting() {
    //     let mut chunks = vec![
    //         ChunkCoordinate::new(-5, 5),
    //         ChunkCoordinate::new(-1, 0),
    //         ChunkCoordinate::new(0, 0),
    //         ChunkCoordinate::new(1, 0),
    //         ChunkCoordinate::new(1, 1),
    //         ChunkCoordinate::new(5, 0),
    //     ];
    //     let camera_chunk = ChunkCoordinate::new(0, 0);
    //     let camera_dir = Vec3::new(1.0, 0.0, 0.0);
    //     chunks.sort_by(|c1, c2| {
    //         chunk_camera_direction(camera_chunk, camera_dir, *c1)
    //             .total_cmp(&chunk_camera_direction(camera_chunk, camera_dir, *c2))
    //     });

    //     assert_eq!(chunks[0], ChunkCoordinate::new(0, 0));
    //     assert_eq!(chunks[1], ChunkCoordinate::new(1, 0));
    // }
}
