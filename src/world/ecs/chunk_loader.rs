use std::collections::{HashMap, HashSet};

use cgmath::{One, Quaternion, Vector2, Vector3, Zero};
use specs::prelude::*;

use crate::{render::RenderMesh, vector2, vector3};

use super::{camera::Camera, Transform};

/// A system that continously generates and loads chunks around the camera.
pub struct ChunkLoaderSystem {
    loaded_chunks: HashMap<Vector2<i32>, Entity>,
}

impl ChunkLoaderSystem {
    pub fn new() -> ChunkLoaderSystem {
        ChunkLoaderSystem {
            loaded_chunks: HashMap::new(),
        }
    }
}

impl<'a> System<'a> for ChunkLoaderSystem {
    type SystemData = (
        ReadStorage<'a, Camera>,
        WriteStorage<'a, Transform>,
        WriteStorage<'a, RenderMesh>,
        Write<'a, crate::world::World>,
        Entities<'a>,
    );

    fn run(
        &mut self,
        (cameras, mut transforms, mut render_meshes, mut game_world, entities): Self::SystemData,
    ) {
        let mut new_chunks = vec![];

        for (_, transform) in (&cameras, &transforms).join() {
            let camera_chunk = game_world.world_to_chunk(transform.position);

            // Get a list of all chunk positions in a circle with radius r around the camera
            let mut chunks_to_load: HashSet<Vector2<i32>> = HashSet::new();
            let r = 16;
            for x in camera_chunk.x - r..camera_chunk.x + r {
                for z in camera_chunk.y - r..camera_chunk.y + r {
                    if (x - camera_chunk.x).pow(2) + (z - camera_chunk.y).pow(2) >= r.pow(2) {
                        continue;
                    }
                    chunks_to_load.insert(vector2!(x, z));
                }
            }

            // Delete any chunks we've loaded that are no longer in the circle
            let keys = self
                .loaded_chunks
                .keys()
                .cloned()
                .collect::<Vec<Vector2<i32>>>();

            for chunk_position in keys {
                if !chunks_to_load.contains(&chunk_position) {
                    let e = self.loaded_chunks.remove(&chunk_position).unwrap();
                    entities.delete(e).unwrap();
                }
            }

            // Load any chunks in the circle we've not already loaded
            for chunk_position in chunks_to_load {
                // Skip any chunks we've already loaded or haven't been generated yet
                if self.loaded_chunks.contains_key(&chunk_position) {
                    continue;
                }

                // 1. Ensure this chunk and all its surrounding chunks have been generated.
                for [x, z] in [[0, 0], [0, 1], [0, -1], [1, 0], [-1, 0]] {
                    let chunk = chunk_position + vector2!(x, z);
                    if !game_world.is_chunk_generated(chunk) {
                        game_world.generate_chunk(chunk_position + vector2!(x, z));
                    }
                }

                // 2. Compute the mesh for this chunk.
                let mesh = game_world.chunk_mesh(chunk_position);

                // 3. Create a new entity for this chunk.
                new_chunks.push((
                    chunk_position,
                    Transform::new(Vector3::zero(), vector3!(1.0, 1.0, 1.0), Quaternion::one()),
                    RenderMesh::new(mesh),
                ));

                // 4. Exit early so we produce all the chunks over multiple frames rather than all in one.
                break;
            }
        }

        for (pos, t, r) in new_chunks.into_iter() {
            let entity = entities
                .build_entity()
                .with(t, &mut transforms)
                .with(r, &mut render_meshes)
                .build();
            self.loaded_chunks.insert(pos, entity);
        }
    }
}
