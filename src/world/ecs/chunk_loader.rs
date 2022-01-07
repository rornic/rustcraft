use std::collections::HashSet;

use cgmath::{One, Quaternion, Vector2, Vector3, Zero};
use specs::prelude::*;

use crate::{render::RenderMesh, vector2, vector3};

use super::{camera::Camera, Transform};

/// A system that continously generates and loads chunks around the camera.
pub struct ChunkLoaderSystem {
    loaded_chunks: HashSet<Vector2<i32>>,
}

impl ChunkLoaderSystem {
    pub fn new() -> ChunkLoaderSystem {
        ChunkLoaderSystem {
            loaded_chunks: HashSet::new(),
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

            // Generate all surrounding chunks and then create entities for them.

            let r = 16;
            'outer: for x in camera_chunk.x - r..camera_chunk.x + r {
                'inner: for z in camera_chunk.y - r..camera_chunk.y + r {
                    if (x - camera_chunk.x).pow(2) + (z - camera_chunk.y).pow(2) >= r.pow(2) {
                        continue;
                    }

                    let chunk_position = vector2!(x, z);

                    // Skip any chunks we've already loaded or haven't been generated yet
                    if self.loaded_chunks.contains(&chunk_position) {
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
                        Transform::new(Vector3::zero(), vector3!(1.0, 1.0, 1.0), Quaternion::one()),
                        RenderMesh::new(mesh),
                    ));

                    self.loaded_chunks.insert(chunk_position);
                    break 'inner;
                }
            }
        }

        for (t, r) in new_chunks.into_iter() {
            entities
                .build_entity()
                .with(t, &mut transforms)
                .with(r, &mut render_meshes)
                .build();
        }
    }
}
