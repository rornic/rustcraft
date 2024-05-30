use std::collections::{HashMap, HashSet, VecDeque};

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
    pbr::{wireframe::Wireframe, MaterialMeshBundle},
    prelude::default,
    render::{
        camera::{self, Camera},
        color::Color,
        mesh::Mesh,
        primitives::Aabb,
        render_resource::Face,
        texture::Image,
    },
    transform::components::{GlobalTransform, Transform},
};
use priority_queue::PriorityQueue;

use super::{chunk::ChunkCoordinate, material::ChunkMaterial};
use crate::{player::PlayerLook, world::World};

#[derive(Component)]
pub struct Chunk {
    coord: ChunkCoordinate,
    dirty: bool,
}

#[derive(Resource)]
pub struct ChunkLoader {
    render_distance: u32,
    generate_queue: VecDeque<ChunkCoordinate>,
    load_queue: VecDeque<ChunkCoordinate>,
    unload_queue: VecDeque<ChunkCoordinate>,
    loaded: HashMap<ChunkCoordinate, Entity>,
    chunk_iterator: ChunkIterator,
}

impl ChunkLoader {
    pub fn new(render_distance: u32) -> Self {
        Self {
            render_distance,
            generate_queue: VecDeque::new(),
            load_queue: VecDeque::new(),
            unload_queue: VecDeque::new(),
            loaded: HashMap::new(),
            chunk_iterator: ChunkIterator::new(),
        }
    }
}

pub fn gather_chunks(
    mut chunk_loader: ResMut<ChunkLoader>,
    mut world: ResMut<World>,
    camera_query: Query<(&Parent, &GlobalTransform), (With<Camera>, Without<PlayerLook>)>,
) {
    let (_, camera) = camera_query.get_single().expect("could not find camera");

    let queued_for_generation = chunk_loader
        .generate_queue
        .iter()
        .cloned()
        .collect::<HashSet<ChunkCoordinate>>();

    let queued_for_loading = chunk_loader
        .load_queue
        .iter()
        .cloned()
        .collect::<HashSet<ChunkCoordinate>>();

    let queued_for_unload = chunk_loader
        .unload_queue
        .iter()
        .cloned()
        .collect::<HashSet<ChunkCoordinate>>();

    let camera_pos = camera.translation();
    let camera_chunk = world.block_to_chunk_coordinate(I64Vec3::new(
        camera_pos.x as i64,
        camera_pos.y as i64,
        camera_pos.z as i64,
    ));

    let camera_forward = camera.forward();
    chunk_loader
        .chunk_iterator
        .update(camera_chunk, camera_forward, &world);

    let distance = chunk_loader.render_distance;
    let next_chunks: Vec<ChunkCoordinate> = chunk_loader
        .chunk_iterator
        .next_chunks(16, distance, &mut world)
        .collect();

    let loaded = chunk_loader
        .loaded
        .keys()
        .cloned()
        .collect::<HashSet<ChunkCoordinate>>();

    let to_unload = loaded
        .iter()
        .filter(|chunk| !queued_for_unload.contains(chunk))
        .filter(|chunk| !queued_for_generation.contains(chunk))
        .filter(|chunk| !queued_for_loading.contains(chunk))
        .filter(|chunk| chunk_distance(**chunk, camera_chunk) > distance);

    for chunk in to_unload {
        chunk_loader.unload_queue.push_front(*chunk);
    }

    let to_generate = next_chunks
        .iter()
        .filter(|chunk| !queued_for_generation.contains(chunk))
        .filter(|chunk| !queued_for_loading.contains(chunk))
        .filter(|chunk| !loaded.contains(*chunk))
        .filter(|chunk| !world.is_chunk_empty(**chunk));

    for chunk in to_generate {
        chunk_loader.generate_queue.push_front(*chunk);
    }
}

pub fn generate_chunks(mut world: ResMut<World>, mut chunk_loader: ResMut<ChunkLoader>) {
    while let Some(chunk) = chunk_loader.generate_queue.pop_front() {
        let mut chunks = vec![chunk];
        chunks.extend(chunk.adjacent());

        world.generate_chunks(chunks);

        chunk_loader.load_queue.push_front(chunk);
    }
}

pub fn load_chunks(
    mut commands: Commands,
    mut chunk_loader: ResMut<ChunkLoader>,
    mut world: ResMut<World>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ChunkMaterial>>,
    asset_server: Res<AssetServer>,
) {
    let mut generated_meshes = vec![];
    while let Some(chunk) = chunk_loader.load_queue.pop_front() {
        if world.is_chunk_empty(chunk) {
            continue;
        }

        generated_meshes.push((chunk, world.generate_chunk_mesh(chunk)));
    }

    for (chunk, mesh) in generated_meshes.into_iter() {
        let (t, aabb) = chunk_components(chunk);
        let entity = commands
            .spawn((
                MaterialMeshBundle {
                    mesh: meshes.add(mesh),
                    material: materials.add(ChunkMaterial {
                        color: Color::WHITE,
                        texture: Some(asset_server.load::<Image>("textures/blocks.png")),
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
        chunk_loader.loaded.insert(chunk, entity);
    }
}

pub fn unload_chunks(mut commands: Commands, mut chunk_loader: ResMut<ChunkLoader>) {
    while let Some(chunk) = chunk_loader.unload_queue.pop_front() {
        if let Some(entity) = chunk_loader.loaded.get(&chunk) {
            commands.entity(*entity).despawn();
            chunk_loader.loaded.remove(&chunk);
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
        if dot < 0.5 {
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

        // deprioritise empty chunks
        if let Some(chunk_data) = world.get_chunk_data(chunk) {
            if chunk_data.empty() {
                score = score * 0.0;
            }
        }

        (score * 100.0).round() as u32
    }

    fn update(&mut self, camera_chunk: ChunkCoordinate, camera_forward: Vec3, world: &World) {
        // reset if camera turns too far from original direction
        if camera_forward.dot(self.camera_forward) < 0.75 {
            self.reset(camera_chunk, camera_forward, world);
            return;
        }

        if self.camera_chunk != camera_chunk {
            self.reset(camera_chunk, camera_forward, world);
            return;
        }
        self.camera_chunk = camera_chunk;
    }

    fn reset(&mut self, camera_chunk: ChunkCoordinate, camera_forward: Vec3, world: &World) {
        self.seen.clear();

        self.camera_chunk = camera_chunk;
        self.camera_forward = camera_forward;

        self.queue.push(camera_chunk, 99999);
    }
}
