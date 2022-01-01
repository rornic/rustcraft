use std::{collections::HashSet, sync::Arc};

use cgmath::{One, Quaternion, Vector2, Vector3, Zero};
use glium::Display;
use specs::prelude::*;

use crate::{
    render::{RenderMesh, Renderer},
    vector2, vector3,
};

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

        for (camera, transform) in (&cameras, &transforms).join() {
            let camera_chunk = game_world.world_to_chunk(transform.position);

            // Generate all surrounding chunks and then create entities for them.
            let range = [-3, -2, -1, 0, 1, 2, 3];
            'outer: for x in range {
                for z in range {
                    let chunk_position = camera_chunk + vector2!(x, z);

                    // Skip any chunks we've already loaded or haven't been generated yet
                    if self.loaded_chunks.contains(&chunk_position)
                        || !game_world.chunks.contains_key(&chunk_position)
                    {
                        continue;
                    }

                    // Create a mesh for the chunk
                    let mesh = game_world.generate_chunk_mesh(chunk_position);

                    let position = {
                        let p = chunk_position * crate::world::CHUNK_SIZE as i32;
                        vector3!(p.x as f32, 0.0, p.y as f32)
                    };

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
            let entity = entities
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

impl<'a> System<'a> for ChunkGeneratorSystem {
    type SystemData = (
        ReadStorage<'a, Camera>,
        ReadStorage<'a, Transform>,
        Write<'a, crate::world::World>,
    );

    fn run(&mut self, (cameras, transforms, mut game_world): Self::SystemData) {
        for (camera, transform) in (&cameras, &transforms).join() {
            let camera_chunk = game_world.world_to_chunk(transform.position);

            // Generate all surrounding chunks and then create entities for them.
            let range = [-7, -6, -5, -4, -3, -2, -1, 0, 1, 2, 3, 4, 5, 6, 7];
            for x in range {
                for z in range {
                    let chunk_position = camera_chunk + vector2!(x, z);
                    game_world.generate_chunk(chunk_position);
                }
            }
        }
    }
}
