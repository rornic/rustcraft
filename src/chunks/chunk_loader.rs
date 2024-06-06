use std::collections::{HashMap, HashSet};

use bevy::{
    asset::{AssetServer, Assets},
    ecs::{
        component::Component,
        entity::Entity,
        query::{With, Without},
        system::{Commands, Query, Res, ResMut, Resource},
    },
    hierarchy::Parent,
    math::{I64Vec3, Vec3},
    pbr::MaterialMeshBundle,
    render::{camera::Camera, color::Color, mesh::Mesh, primitives::Aabb, texture::Image},
    tasks::{AsyncComputeTaskPool, Task},
    transform::components::{GlobalTransform, Transform},
    utils::futures,
};
use priority_queue::PriorityQueue;
use tracing::info;

use super::{
    chunk::{ChunkCoordinate, ChunkData},
    generate::generator::{generate_chunk, generate_chunk_mesh},
    material::ChunkMaterial,
};
use crate::{player::PlayerLook, world::World};

#[derive(Component)]
pub struct Chunk {
    coord: ChunkCoordinate,
    dirty: bool,
}

#[derive(Component)]
pub struct GenerateChunkData {
    task: Task<ChunkData>,
}

#[derive(Component)]
pub struct GenerateChunkMesh {
    coord: ChunkCoordinate,
    task: Option<Task<Mesh>>,
}

#[derive(Resource)]
pub struct ChunkLoader {
    render_distance: u32,
    chunk_to_entity: HashMap<ChunkCoordinate, Entity>,
    chunk_iterator: ChunkIterator,
}

impl ChunkLoader {
    pub fn new(render_distance: u32) -> Self {
        Self {
            render_distance,
            chunk_to_entity: HashMap::new(),
            chunk_iterator: ChunkIterator::new(),
        }
    }
}

pub fn gather_chunks(
    mut commands: Commands,
    mut chunk_loader: ResMut<ChunkLoader>,
    mut world: ResMut<World>,
    camera_query: Query<(&Parent, &GlobalTransform), (With<Camera>, Without<PlayerLook>)>,
) {
    let (_, camera) = camera_query.get_single().expect("could not find camera");

    let camera_pos = camera.translation();
    let camera_chunk = world.block_to_chunk_coordinate(I64Vec3::new(
        camera_pos.x as i64,
        camera_pos.y as i64,
        camera_pos.z as i64,
    ));

    let camera_forward = camera.forward();
    chunk_loader
        .chunk_iterator
        .update(camera_chunk, camera_forward);

    let distance = chunk_loader.render_distance;
    let next_chunks: Vec<ChunkCoordinate> = chunk_loader
        .chunk_iterator
        .next_chunks(8, distance, &mut world)
        .collect();

    let loaded = chunk_loader
        .chunk_to_entity
        .keys()
        .cloned()
        .collect::<HashSet<ChunkCoordinate>>();

    let to_generate: HashSet<ChunkCoordinate> = next_chunks
        .into_iter()
        .filter(|chunk| !loaded.contains(chunk))
        .collect();

    let task_pool = AsyncComputeTaskPool::get();
    for chunk in to_generate.iter() {
        generate_single_chunk(
            &mut commands,
            &mut world,
            *chunk,
            task_pool,
            &mut chunk_loader,
        );
    }
}

fn generate_single_chunk(
    commands: &mut Commands,
    world: &mut ResMut<World>,
    coord: ChunkCoordinate,
    task_pool: &AsyncComputeTaskPool,
    chunk_loader: &mut ResMut<ChunkLoader>,
) {
    let noise_generator = world.noise_generator.clone();
    let height = world.height;
    let entity = commands
        .spawn((
            Chunk {
                coord,
                dirty: false,
            },
            GenerateChunkData {
                task: task_pool
                    .spawn(async move { generate_chunk(noise_generator, coord, height) }),
            },
        ))
        .id();
    chunk_loader.chunk_to_entity.insert(coord, entity);
}

pub fn generate_chunks(
    mut commands: Commands,
    mut world: ResMut<World>,
    mut chunks_query: Query<(Entity, &mut Chunk, &mut GenerateChunkData)>,
) {
    for (entity, mut chunk, mut gen_chunk) in chunks_query.iter_mut() {
        if let Some(chunk_data) = futures::check_ready(&mut gen_chunk.task) {
            let data = world.insert_chunk(chunk.coord, chunk_data);
            commands.entity(entity).remove::<GenerateChunkData>();
            chunk.dirty = !data.empty();
        }
    }
}

pub fn mark_chunks(
    mut commands: Commands,
    mut world: ResMut<World>,
    mut chunks_query: Query<
        (Entity, &mut Chunk),
        (Without<GenerateChunkData>, Without<GenerateChunkMesh>),
    >,
) {
    // let mut regen_adjacent = HashSet::new();
    chunks_query.iter_mut().for_each(|(entity, mut chunk)| {
        if chunk.dirty {
            if chunk
                .coord
                .adjacent()
                .into_iter()
                .all(|adj| world.is_chunk_generated(adj))
            {
                commands.entity(entity).insert(GenerateChunkMesh {
                    coord: chunk.coord,
                    task: None,
                });
                chunk.dirty = false;
            }
        }
    });
}

pub fn load_chunks(
    mut commands: Commands,
    mut world: ResMut<World>,
    mut chunks_query: Query<(Entity, &Chunk, &mut GenerateChunkMesh)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ChunkMaterial>>,
    asset_server: Res<AssetServer>,
) {
    let mut ready = vec![];
    let task_pool = AsyncComputeTaskPool::get();

    for (entity, chunk, mut gen_chunk_mesh) in chunks_query.iter_mut() {
        match &mut gen_chunk_mesh.task {
            Some(task) => {
                if let Some(mesh) = futures::check_ready(task) {
                    ready.push((entity, chunk, mesh));
                }
            }
            None => {
                if let Some(data) = world.get_chunk_data(gen_chunk_mesh.coord) {
                    let adjacent = world.adjacent_chunk_data(chunk.coord);
                    gen_chunk_mesh.task =
                        Some(task_pool.spawn(async move { generate_chunk_mesh(data, adjacent) }));
                }
            }
        }
    }

    for (entity, chunk, mesh) in ready {
        let (t, aabb) = chunk_components(chunk.coord);

        commands.entity(entity).insert((
            MaterialMeshBundle {
                mesh: meshes.add(mesh),
                material: materials.add(ChunkMaterial {
                    color: Color::WHITE,
                    texture: Some(asset_server.load::<Image>("textures/blocks.png")),
                }),
                transform: t,
                ..Default::default()
            },
            aabb,
        ));
        commands.entity(entity).remove::<GenerateChunkMesh>();
    }
}

pub fn unload_chunks(
    mut commands: Commands,
    mut chunk_loader: ResMut<ChunkLoader>,
    chunks_query: Query<(Entity, &Chunk), (Without<GenerateChunkData>, Without<GenerateChunkMesh>)>,
) {
    for (entity, chunk) in chunks_query.iter() {
        if chunk_distance(chunk.coord, chunk_loader.chunk_iterator.camera_chunk)
            > chunk_loader.render_distance
        {
            commands.entity(entity).despawn();
            chunk_loader.chunk_to_entity.remove(&chunk.coord);
        }
    }
}

fn chunk_world_pos(chunk: ChunkCoordinate) -> Vec3 {
    Vec3::new(
        (chunk.0.x * 16) as f32,
        (chunk.0.y * 16) as f32,
        (chunk.0.z * 16) as f32,
    )
}

fn chunk_distance(chunk: ChunkCoordinate, other: ChunkCoordinate) -> u32 {
    (chunk.0 - other.0).abs().max_element() as u32
}

fn chunk_components(chunk: ChunkCoordinate) -> (Transform, Aabb) {
    let pos = chunk_world_pos(chunk);
    let t = Transform::from_translation(Vec3::new(pos.x, pos.y, pos.z));
    let aabb = Aabb::from_min_max(Vec3::new(0.0, 0.0, 0.0), Vec3::new(16.0, 16.0, 16.0));
    (t, aabb)
}

/// `ChunkIterator` enables iteration of nearby chunks over multiple frames
/// by storing BFS state in memory and dynamically recalculating when the camera chunk or direction changes
#[derive(Debug)]
struct ChunkIterator {
    seen: HashSet<ChunkCoordinate>,
    camera_chunk: ChunkCoordinate,
    camera_forward: Vec3,
    queue: PriorityQueue<ChunkCoordinate, u32>,
}

impl ChunkIterator {
    fn new() -> Self {
        Self {
            seen: HashSet::new(),
            camera_chunk: ChunkCoordinate(I64Vec3::ZERO),
            camera_forward: Vec3::ZERO,
            queue: PriorityQueue::new(),
        }
    }

    #[tracing::instrument]
    fn next_chunks(
        &mut self,
        count: usize,
        max_distance: u32,
        world: &mut World,
    ) -> impl Iterator<Item = ChunkCoordinate> {
        let mut next_chunks = Vec::new();
        while !self.queue.is_empty() && next_chunks.len() < count {
            let (next, _) = self.queue.pop().unwrap();
            next_chunks.push(next);
            self.seen.insert(next);

            if chunk_distance(next, self.camera_chunk) >= max_distance {
                continue;
            }

            for neighbour in next.adjacent().into_iter() {
                self.queue_chunk(neighbour, world);
            }
        }

        next_chunks.into_iter()
    }

    fn queue_chunk(&mut self, chunk: ChunkCoordinate, world: &mut World) {
        if self.seen.contains(&chunk) {
            return;
        }

        let dot = self.dot(chunk, world);
        if dot < 0.0 {
            return;
        }

        let score = self.calculate_priority(chunk, world);
        self.queue.push(chunk, score);
        self.seen.insert(chunk);
    }

    fn dot(&self, chunk: ChunkCoordinate, world: &World) -> f32 {
        let direction: Vec3 =
            (world.chunk_to_world(chunk) - world.chunk_to_world(self.camera_chunk)).normalize();
        self.camera_forward.dot(direction)
    }

    fn calculate_priority(&self, chunk: ChunkCoordinate, world: &mut World) -> u32 {
        let score = self.dot(chunk, world) / chunk_distance(chunk, self.camera_chunk) as f32;
        (score * 100.0).round() as u32
    }

    fn update(&mut self, camera_chunk: ChunkCoordinate, camera_forward: Vec3) {
        // reset if camera turns too far from original direction
        if camera_forward.dot(self.camera_forward) < 0.75 {
            self.reset(camera_chunk, camera_forward);
            return;
        }
        self.camera_chunk = camera_chunk;
    }

    fn reset(&mut self, camera_chunk: ChunkCoordinate, camera_forward: Vec3) {
        self.seen.clear();

        self.camera_chunk = camera_chunk;
        self.camera_forward = camera_forward;

        self.queue.push(camera_chunk, 99999);
    }
}
