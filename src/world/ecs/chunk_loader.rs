use std::{collections::HashSet, sync::Arc};

use cgmath::{num_traits::Pow, One, Quaternion, Vector2, Vector3, Zero};
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

            let r = 8;
            'outer: for x in camera_chunk.x - r..camera_chunk.x + r {
                for z in camera_chunk.y - r..camera_chunk.y + r {
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
                        game_world.generate_chunk(chunk_position + vector2!(x, z));
                    }

                    // 2. Create a mesh for this chunk.
                    let mesh = game_world.generate_chunk_mesh(chunk_position);

                    // 3. Create a new entity for this chunk.
                    new_chunks.push((
                        Transform::new(Vector3::zero(), vector3!(1.0, 1.0, 1.0), Quaternion::one()),
                        RenderMesh::new(Arc::new(mesh)),
                    ));

                    self.loaded_chunks.insert(chunk_position);
                    break 'outer;
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

/// Generates chunks on the fly around the camera
pub struct ChunkGeneratorSystem;

impl ChunkGeneratorSystem {
    pub fn new() -> Self {
        Self {}
    }
}

const CHUNKS_PER_FRAME: u32 = 25;
impl<'a> System<'a> for ChunkGeneratorSystem {
    type SystemData = (
        ReadStorage<'a, Camera>,
        ReadStorage<'a, Transform>,
        Write<'a, crate::world::World>,
    );

    fn run(&mut self, (cameras, transforms, mut game_world): Self::SystemData) {
        for (_, transform) in (&cameras, &transforms).join() {
            let camera_chunk = game_world.world_to_chunk(transform.position);

            // let mut chunk_count = 0;
            // // Generate all surrounding chunks and then create entities for them.
            // for x in -50..50 {
            //     for z in -50..50 {
            //         let chunk_position = camera_chunk + vector2!(x, z);
            //         if let Some(_) = game_world.generate_chunk(chunk_position) {
            //             chunk_count += 1;
            //             if chunk_count >= CHUNKS_PER_FRAME {
            //                 return;
            //             }
            //         }
            //     }
            // }
        }
    }
}
