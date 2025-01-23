use std::{
    collections::{HashMap, HashSet},
    vec::IntoIter,
};

use bevy::{
    asset::{Assets, Handle},
    ecs::{
        component::Component,
        entity::Entity,
        query::{With, Without},
        system::{Commands, Query, ResMut, Resource},
    },
    hierarchy::Parent,
    math::{Dir3, I64Vec3, Vec3},
    pbr::MeshMaterial3d,
    prelude::Mesh3d,
    render::{camera::Camera, mesh::Mesh, primitives::Aabb},
    tasks::{AsyncComputeTaskPool, Task},
    transform::components::{GlobalTransform, Transform},
    utils::futures,
};
use priority_queue::PriorityQueue;

use super::{
    chunk::{ChunkCoordinate, ChunkData},
    generate::generator::{generate_chunk, generate_chunk_mesh},
    material::ChunkMaterial,
};
use crate::{player::PlayerLook, world::World};

#[derive(Component)]
pub struct Chunk {
    coord: ChunkCoordinate,
}

#[derive(Component)]
pub struct DirtyChunk {}

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
    material: Handle<ChunkMaterial>,
}

const MAX_CHUNKS_PER_FRAME: usize = 32;

impl ChunkLoader {
    pub fn new(render_distance: u32, material: Handle<ChunkMaterial>) -> Self {
        Self {
            render_distance,
            chunk_to_entity: HashMap::new(),
            chunk_iterator: ChunkIterator::new(),
            material,
        }
    }
}

pub fn gather_chunks(
    mut commands: Commands,
    mut chunk_loader: ResMut<ChunkLoader>,
    mut world: ResMut<World>,
    camera_query: Query<(&Parent, &GlobalTransform), (With<Camera>, Without<PlayerLook>)>,
    generating_chunks_query: Query<&Chunk, With<GenerateChunkData>>,
) {
    if generating_chunks_query.iter().count() > 1024 {
        return;
    }

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

    let mut next_chunks: Vec<ChunkCoordinate> = vec![];
    while next_chunks.len() < MAX_CHUNKS_PER_FRAME {
        if let Some(next) =
            chunk_loader
                .chunk_iterator
                .next_chunks(MAX_CHUNKS_PER_FRAME, distance, &mut world)
        {
            next_chunks
                .extend(next.filter(|chunk| !chunk_loader.chunk_to_entity.contains_key(chunk)));
        } else {
            break;
        }
    }

    let task_pool = AsyncComputeTaskPool::get();
    for chunk in next_chunks {
        generate_single_chunk(
            &mut commands,
            &mut world,
            chunk,
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
            Chunk { coord },
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
    for (entity, chunk, mut gen_chunk) in chunks_query.iter_mut() {
        if let Some(chunk_data) = futures::check_ready(&mut gen_chunk.task) {
            let data = world.insert_chunk(chunk.coord, chunk_data);
            if !data.empty() {
                commands.entity(entity).insert(DirtyChunk {});
            }
            commands.entity(entity).remove::<GenerateChunkData>();
        }
    }
}

pub fn mark_chunks(
    mut commands: Commands,
    mut world: ResMut<World>,
    mut chunks_query: Query<
        (Entity, &mut Chunk),
        (
            With<DirtyChunk>,
            Without<GenerateChunkData>,
            Without<GenerateChunkMesh>,
        ),
    >,
) {
    chunks_query.iter_mut().for_each(|(entity, chunk)| {
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
            commands.entity(entity).remove::<DirtyChunk>();
        }
    });
}

pub fn load_chunks(
    mut commands: Commands,
    mut world: ResMut<World>,
    mut chunks_query: Query<(Entity, &Chunk, &mut GenerateChunkMesh)>,
    mut meshes: ResMut<Assets<Mesh>>,
    chunk_loader: ResMut<ChunkLoader>,
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

        if ready.len() > MAX_CHUNKS_PER_FRAME {
            break;
        }
    }

    for (entity, chunk, mesh) in ready {
        let (t, aabb) = chunk_components(chunk.coord);

        commands.entity(entity).insert((
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(chunk_loader.material.clone_weak()),
            t,
            aabb,
        ));
        commands.entity(entity).remove::<GenerateChunkMesh>();
    }
}

pub fn unload_chunks(
    mut commands: Commands,
    mut world: ResMut<World>,
    mut chunk_loader: ResMut<ChunkLoader>,
    chunks_query: Query<(Entity, &Chunk), (Without<GenerateChunkData>, Without<GenerateChunkMesh>)>,
) {
    for (entity, chunk) in chunks_query.iter() {
        if chunk_distance(chunk.coord, chunk_loader.chunk_iterator.camera_chunk)
            > chunk_loader.render_distance
        {
            commands.entity(entity).despawn();
            chunk_loader.chunk_to_entity.remove(&chunk.coord);
            world.clear_chunk(chunk.coord);
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
    camera_forward: Dir3,
    queue: PriorityQueue<ChunkCoordinate, u32>,
}

impl ChunkIterator {
    fn new() -> Self {
        Self {
            seen: HashSet::new(),
            camera_chunk: ChunkCoordinate(I64Vec3::ZERO),
            camera_forward: Dir3::X,
            queue: PriorityQueue::new(),
        }
    }

    fn next_chunks(
        &mut self,
        count: usize,
        max_distance: u32,
        world: &mut World,
    ) -> Option<IntoIter<ChunkCoordinate>> {
        if self.queue.is_empty() {
            return None;
        }

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

        Some(next_chunks.into_iter())
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
        let mut score = self.dot(chunk, world) / chunk_distance(chunk, self.camera_chunk) as f32;

        if let Some(true) = world.get_chunk_data(chunk).map(|data| data.empty()) {
            score = 0.0;
        }

        (score * 100.0).round() as u32
    }

    fn update(&mut self, camera_chunk: ChunkCoordinate, camera_forward: Dir3) {
        // reset if camera turns too far from original direction
        if camera_forward.dot(self.camera_forward.as_vec3()) < 0.9 {
            self.reset(camera_chunk, camera_forward);
            return;
        }
    }

    fn reset(&mut self, camera_chunk: ChunkCoordinate, camera_forward: Dir3) {
        self.seen.clear();

        self.camera_chunk = camera_chunk;
        self.camera_forward = camera_forward;

        self.queue.push(camera_chunk, 99999);
    }
}
