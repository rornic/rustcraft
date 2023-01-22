use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use cgmath::{InnerSpace, One, Quaternion, Vector2, Vector3, Zero};
use specs::{prelude::*, rayon::prelude::IntoParallelRefIterator};

use crate::{
    render::{camera::Camera, mesh::Mesh, renderer::RenderMesh},
    vector2, vector3,
    world::{Chunk, CHUNK_SIZE, WORLD_HEIGHT},
};

use super::{bounds::Bounds, Transform};
pub struct ChunkGenerator {
    generate_distance: u32,
}

impl ChunkGenerator {
    pub fn new(generate_distance: u32) -> Self {
        Self { generate_distance }
    }
}

impl<'a> System<'a> for ChunkGenerator {
    type SystemData = (
        ReadStorage<'a, Camera>,
        WriteStorage<'a, Transform>,
        Write<'a, crate::world::World>,
    );

    fn run(&mut self, (cameras, transforms, mut game_world): Self::SystemData) {
        let (camera, transform) = (&cameras, &transforms).join().next().unwrap();
        let camera_chunk = game_world.world_to_chunk(transform.position);

        let mut chunks: Vec<Vector2<i32>> = all_chunks(camera_chunk, self.generate_distance)
            .filter(|chunk| !game_world.is_chunk_generated(*chunk))
            .collect();
        chunks.sort_by(|c1, c2| {
            chunk_distance(camera_chunk, *c1).total_cmp(&chunk_distance(camera_chunk, *c2))
        });

        let generated_chunks = chunks
            .iter()
            .take(32)
            .collect::<Vec<&Vector2<i32>>>()
            .par_iter()
            .map(|chunk| (**chunk, game_world.generate_chunk(**chunk)))
            .collect::<Vec<(Vector2<i32>, Chunk)>>();

        for chunk in generated_chunks {
            game_world.cache_chunk(chunk.0, chunk.1);
        }
    }
}

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

impl<'a> System<'a> for ChunkLoader {
    type SystemData = (
        ReadStorage<'a, Camera>,
        WriteStorage<'a, Transform>,
        WriteStorage<'a, RenderMesh>,
        WriteStorage<'a, Bounds>,
        Write<'a, crate::world::World>,
        Entities<'a>,
    );

    fn run(
        &mut self,
        (cameras, mut transforms, mut render_meshes, mut bounds, mut game_world, entities): Self::SystemData,
    ) {
        let (camera, transform) = (&cameras, &transforms).join().next().unwrap();
        let camera_chunk = game_world.world_to_chunk(transform.position);

        let all_chunks = all_chunks(camera_chunk, self.render_distance)
            .filter(|chunk| game_world.is_chunk_generated(*chunk))
            .filter(|chunk| game_world.are_neighbours_generated(*chunk))
            .collect::<HashSet<Vector2<i32>>>();

        // Unload old chunks
        for chunk in self
            .active_chunks
            .difference(&all_chunks)
            .cloned()
            .collect::<Vec<Vector2<i32>>>()
        {
            let e = self.chunk_entities.remove(&chunk).unwrap();
            entities.delete(e).unwrap();
            self.active_chunks.remove(&chunk);
            self.chunk_meshes.remove(&chunk);
        }

        // Re-mesh dirty chunks
        for chunk in self
            .active_chunks
            .iter()
            .cloned()
            .filter(|c| game_world.chunk(*c).unwrap().dirty)
            .collect::<Vec<Vector2<i32>>>()
        {
            self.chunk_meshes.remove(&chunk);
            let entity = self.chunk_entities.get(&chunk).unwrap();
            let new_mesh = game_world.generate_chunk_mesh(chunk);
            render_meshes.get_mut(*entity).unwrap().mesh = Arc::new(new_mesh);
            game_world.clear_chunk_dirty_bit(chunk);
        }

        // Load new chunks
        let mut to_load = all_chunks
            .difference(&self.active_chunks)
            .cloned()
            .collect::<Vec<Vector2<i32>>>();
        to_load.sort_by(|c1, c2| {
            chunk_camera_direction(camera_chunk, camera.look_direction(), *c1).total_cmp(
                &chunk_camera_direction(camera_chunk, camera.look_direction(), *c2),
            )
        });

        let new_meshes = to_load
            .iter()
            .cloned()
            .filter(|chunk| !self.chunk_meshes.contains_key(chunk))
            .collect::<Vec<Vector2<i32>>>()
            .par_iter()
            .map(|chunk| (*chunk, game_world.generate_chunk_mesh(*chunk)))
            .collect::<Vec<(Vector2<i32>, Mesh)>>();
        for (chunk, mesh) in new_meshes {
            self.chunk_meshes.insert(chunk, Arc::new(mesh));
        }

        for chunk in to_load {
            self.active_chunks.insert(chunk);

            if let Some(e) = self.chunk_entities.get(&chunk) {
                render_meshes.get_mut(*e).unwrap().visible = true;
                continue;
            }

            let mesh = self.chunk_meshes.get(&chunk).unwrap();

            let (t, r, b) = chunk_components(chunk, mesh.clone());
            let entity = entities
                .build_entity()
                .with(t, &mut transforms)
                .with(r, &mut render_meshes)
                .with(b, &mut bounds)
                .build();
            self.chunk_entities.insert(chunk, entity);
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

fn chunk_components(chunk: Vector2<i32>, mesh: Arc<Mesh>) -> (Transform, RenderMesh, Bounds) {
    let t = Transform::new(
        chunk_world_pos(chunk),
        vector3!(1.0, 1.0, 1.0),
        Quaternion::one(),
    );
    let r = RenderMesh::new(mesh, true);
    let b = Bounds::new(
        vector3!(
            CHUNK_SIZE as f32 / 2.0,
            WORLD_HEIGHT as f32 / 2.0,
            CHUNK_SIZE as f32 / 2.0
        ),
        vector3!(CHUNK_SIZE as f32, WORLD_HEIGHT as f32, CHUNK_SIZE as f32),
    );

    (t, r, b)
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
