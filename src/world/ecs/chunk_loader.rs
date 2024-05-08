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
    math::Vec3,
    pbr::{PbrBundle, StandardMaterial},
    prelude::default,
    render::{mesh::Mesh, primitives::Aabb, render_resource::Face, texture::Image},
    tasks::{AsyncComputeTaskPool, ParallelSlice},
    transform::components::Transform,
};

use crate::world::{ChunkCoordinate, ChunkData, World, CHUNK_SIZE, WORLD_HEIGHT};

use super::player::Player;

const GENERATE_DISTANCE: u32 = 32;

pub fn generate_chunks(
    mut world_query: Query<&mut World>,
    player_query: Query<&Transform, With<Player>>,
) {
    let world = &mut world_query
        .get_single_mut()
        .expect("could not find single world");

    let player = player_query.get_single().expect("could not find player");

    let camera_chunk = world.world_to_chunk(Vec3::new(
        player.translation.x,
        player.translation.y,
        player.translation.z,
    ));

    let mut chunks: Vec<ChunkCoordinate> = all_chunks(camera_chunk, GENERATE_DISTANCE)
        .filter(|chunk| !world.is_chunk_generated(*chunk))
        .collect();
    chunks.sort_by(|c1, c2| {
        chunk_distance(camera_chunk, *c1).total_cmp(&chunk_distance(camera_chunk, *c2))
    });

    let chunks_to_generate: Vec<ChunkCoordinate> = chunks.into_iter().take(8).collect();
    let thread_pool = AsyncComputeTaskPool::get();
    let generated_chunks = chunks_to_generate.par_chunk_map(thread_pool, 2, |_index, chunks| {
        chunks
            .iter()
            .map(|chunk| (*chunk, world.generate_chunk(*chunk)))
            .collect::<Vec<(ChunkCoordinate, ChunkData)>>()
    });

    for (pos, chunk_data) in generated_chunks.into_iter().flatten() {
        world.cache_chunk(pos, chunk_data);
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
    mut world_query: Query<&mut World>,
    mut chunk_loader_query: Query<&mut ChunkLoader>,
    player_query: Query<&Transform, With<Player>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    loaded_chunks: Query<(Entity, &Chunk)>,
    asset_server: Res<AssetServer>,
) {
    let world = &mut world_query
        .get_single_mut()
        .expect("could not find single world");
    let chunk_loader = &mut chunk_loader_query
        .get_single_mut()
        .expect("could not find single chunk loader");

    let player = player_query.get_single().expect("could not find player");
    let camera_chunk = world.world_to_chunk(Vec3::new(
        player.translation.x,
        player.translation.y,
        player.translation.z,
    ));

    let mut chunks_to_load = all_chunks(camera_chunk, chunk_loader.render_distance)
        .filter(|chunk| world.is_chunk_generated(*chunk))
        .filter(|chunk| world.are_neighbours_generated(*chunk))
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
    //     .filter(|c| world.chunk(*c).unwrap().dirty)
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
    let thread_pool = AsyncComputeTaskPool::get();
    let generated_meshes = chunks_to_load.par_chunk_map(thread_pool, 2, |_index, chunks| {
        chunks
            .iter()
            .map(|chunk| (*chunk, world.generate_chunk_mesh(*chunk)))
            .collect::<Vec<(ChunkCoordinate, Mesh)>>()
    });

    for (chunk, mesh) in generated_meshes.into_iter().flatten() {
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
    let (x_min, x_max) = (centre.x - distance as i64, centre.x + distance as i64);
    let (z_min, z_max) = (centre.y - distance as i64, centre.y + distance as i64);
    (x_min..x_max).flat_map(move |a| (z_min..z_max).map(move |b| ChunkCoordinate::new(a, b)))
}

fn chunk_distance(chunk1: ChunkCoordinate, chunk2: ChunkCoordinate) -> f32 {
    (((chunk2.x - chunk1.x).abs().pow(2) + (chunk2.y - chunk1.y).abs().pow(2)) as f32).sqrt()
}

fn chunk_camera_direction(
    camera_chunk: ChunkCoordinate,
    camera_forward: Vec3,
    chunk: ChunkCoordinate,
) -> f32 {
    let camera_dir = (chunk_world_pos(camera_chunk) - chunk_world_pos(chunk)).normalize();
    let dot = camera_forward.dot(camera_dir);
    let dist = chunk_distance(camera_chunk, chunk) as f32;
    if dist == 0.0 {
        return -f32::INFINITY;
    }
    dot / dist
}

fn chunk_world_pos(chunk: ChunkCoordinate) -> Vec3 {
    Vec3::new(
        (chunk.x * CHUNK_SIZE as i64) as f32,
        0.0,
        (chunk.y * CHUNK_SIZE as i64) as f32,
    )
}

fn chunk_components(chunk: ChunkCoordinate) -> (Transform, Aabb) {
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
    use bevy::math::Vec3;

    use crate::world::ChunkCoordinate;

    use super::chunk_camera_direction;

    #[test]
    fn test_chunk_sorting() {
        let mut chunks = vec![
            ChunkCoordinate::new(-5, 5),
            ChunkCoordinate::new(-1, 0),
            ChunkCoordinate::new(0, 0),
            ChunkCoordinate::new(1, 0),
            ChunkCoordinate::new(1, 1),
            ChunkCoordinate::new(5, 0),
        ];
        let camera_chunk = ChunkCoordinate::new(0, 0);
        let camera_dir = Vec3::new(1.0, 0.0, 0.0);
        chunks.sort_by(|c1, c2| {
            chunk_camera_direction(camera_chunk, camera_dir, *c1)
                .total_cmp(&chunk_camera_direction(camera_chunk, camera_dir, *c2))
        });

        assert_eq!(chunks[0], ChunkCoordinate::new(0, 0));
        assert_eq!(chunks[1], ChunkCoordinate::new(1, 0));
    }
}
