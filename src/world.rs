use std::{fmt::Debug, sync::Arc};

use bevy::prelude::Resource;
use bevy::{
    math::{I64Vec3, U16Vec3, Vec3},
    platform::collections::HashMap,
};

use crate::block::BlockType;
use crate::chunks::generate::noise::NoiseGenerator;
use crate::chunks::generate::region_store::{resident_cap_for, RegionStore};
use crate::chunks::generate::WORLD_HEIGHT;

use super::chunks::chunk::{ChunkCoordinate, ChunkData, CHUNK_SIZE};

#[derive(Resource)]
pub struct World {
    seed: u32,
    pub height: u64,
    chunks: HashMap<ChunkCoordinate, Arc<ChunkData>>,
    // what lies below and above the world, which isn't generated, see `beyond_world`
    below: Arc<ChunkData>,
    above: Arc<ChunkData>,
    pub region_store: Arc<RegionStore>,
    spawn: Vec3,
}

impl World {
    /// `region_distance` is how far, in chunks, terrain is built from full-detail regions,
    /// see `LodLayout::region_reach`.
    pub fn new(region_distance: u32) -> Self {
        // RC_SEED repeats a world, for chasing bugs that depend on its terrain
        let seed = std::env::var("RC_SEED")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or_else(rand::random);
        let height = WORLD_HEIGHT;
        let noise_generator = Arc::new(NoiseGenerator::new(seed));
        let (spawn_x, spawn_z, spawn_height) = noise_generator.find_spawn();
        let region_distance_blocks = region_distance as i64 * CHUNK_SIZE as i64;
        Self {
            seed,
            height,
            chunks: HashMap::new(),
            below: Arc::new(ChunkData::from_cells(std::iter::repeat_n(
                BlockType::Stone,
                (CHUNK_SIZE as usize).pow(3),
            ))),
            above: Arc::new(ChunkData::default()),
            region_store: Arc::new(
                RegionStore::new(noise_generator)
                    .with_resident_cap(resident_cap_for(region_distance_blocks)),
            ),
            spawn: Vec3::new(spawn_x as f32, spawn_height as f32 + 20.0, spawn_z as f32),
        }
    }

    pub fn seed(&self) -> u32 {
        self.seed
    }

    pub fn spawn(&self) -> Vec3 {
        self.spawn
    }

    pub fn insert_chunk(
        &mut self,
        chunk_coord: ChunkCoordinate,
        chunk_data: ChunkData,
    ) -> Arc<ChunkData> {
        let chunk_data = Arc::new(chunk_data);
        self.chunks.insert(chunk_coord, chunk_data.clone());
        chunk_data
    }

    /// The chunk layers the world is generated in, from the bottom up.
    pub fn layers(height: u64) -> std::ops::Range<i64> {
        0..height.div_ceil(CHUNK_SIZE as u64) as i64
    }

    // Below the world is solid and above it open sky, standing in for chunks that are never
    // generated, so the chunks at its bottom and top still have neighbours to mesh against.
    fn beyond_world(&self, chunk_coord: ChunkCoordinate) -> Option<&Arc<ChunkData>> {
        let layers = Self::layers(self.height);
        if chunk_coord.0.y < layers.start {
            Some(&self.below)
        } else if chunk_coord.0.y >= layers.end {
            Some(&self.above)
        } else {
            None
        }
    }

    fn chunk(&self, chunk_coord: ChunkCoordinate) -> Option<&Arc<ChunkData>> {
        self.beyond_world(chunk_coord)
            .or_else(|| self.chunks.get(&chunk_coord))
    }

    pub fn get_chunk_data(&mut self, chunk_coord: ChunkCoordinate) -> Option<Arc<ChunkData>> {
        self.chunk(chunk_coord).cloned()
    }

    pub fn clear_chunk(&mut self, chunk_coord: ChunkCoordinate) {
        self.chunks.remove(&chunk_coord);
    }

    pub fn adjacent_chunk_data(
        &mut self,
        chunk_coord: ChunkCoordinate,
    ) -> Vec<Option<Arc<ChunkData>>> {
        chunk_coord
            .neighbors_26()
            .iter()
            .map(|coord| self.get_chunk_data(*coord))
            .collect()
    }

    pub fn is_chunk_generated(&mut self, chunk_coord: ChunkCoordinate) -> bool {
        self.chunk(chunk_coord).is_some()
    }

    pub fn is_chunk_empty(&mut self, chunk_coord: ChunkCoordinate) -> bool {
        self.chunk(chunk_coord)
            .is_some_and(|chunk_data| chunk_data.empty())
    }

    /// Whether the chunk and the chunks against its faces are all opaque, so it has no faces to draw.
    pub fn is_chunk_buried(&self, chunk_coord: ChunkCoordinate) -> bool {
        let opaque = |coord| self.chunk(coord).is_some_and(|data| data.opaque());
        opaque(chunk_coord) && chunk_coord.adjacent().into_iter().all(opaque)
    }

    pub fn chunk_to_world(&self, chunk_coord: ChunkCoordinate) -> Vec3 {
        chunk_coord.centre()
    }

    pub fn block_to_chunk_coordinate(&self, block_coord: I64Vec3) -> ChunkCoordinate {
        block_coord
            .div_euclid(I64Vec3::splat(CHUNK_SIZE as i64))
            .into()
    }

    pub fn get_block_at(&mut self, world_block_coord: I64Vec3) -> BlockType {
        let chunk_size = CHUNK_SIZE as i64;
        // div_euclid/rem_euclid, not `/` and `%` - those truncate toward zero, which puts
        // negative coordinates in the wrong chunk and yields a negative (then
        // wrapping-to-huge-u16) local coordinate.
        let chunk_coord = ChunkCoordinate(I64Vec3::new(
            world_block_coord.x.div_euclid(chunk_size),
            world_block_coord.y.div_euclid(chunk_size),
            world_block_coord.z.div_euclid(chunk_size),
        ));
        let Some(chunk_data) = self.get_chunk_data(chunk_coord) else {
            return BlockType::Air; // ungenerated chunk under the camera isn't water
        };
        let local = U16Vec3::new(
            world_block_coord.x.rem_euclid(chunk_size) as u16,
            world_block_coord.y.rem_euclid(chunk_size) as u16,
            world_block_coord.z.rem_euclid(chunk_size) as u16,
        );
        chunk_data.get_block_at(local)
    }
}

impl Debug for World {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("World").field("seed", &self.seed).finish()
    }
}

#[cfg(test)]
mod tests {
    use bevy::math::I64Vec3;

    use super::World;
    use crate::block::BlockType;
    use crate::chunks::chunk::{ChunkCoordinate, ChunkData};

    #[test]
    fn test_block_to_chunk_coordinate() {
        let world = World::new(16);
        let chunk = |x, y, z| world.block_to_chunk_coordinate(I64Vec3::new(x, y, z));
        assert_eq!(ChunkCoordinate(I64Vec3::new(0, 1, 2)), chunk(5, 16, 47));
        // negative blocks belong to the chunk below, not chunk 0
        assert_eq!(
            ChunkCoordinate(I64Vec3::new(-1, -1, -2)),
            chunk(-5, -16, -17)
        );
    }

    #[test]
    fn test_chunk_is_buried_only_when_it_and_its_face_neighbours_are_opaque() {
        let filled = |block| {
            let mut data = ChunkData::default();
            for x in 0..data.size {
                for y in 0..data.size {
                    for z in 0..data.size {
                        data.set_block_at(bevy::math::U16Vec3::new(x, y, z), block);
                    }
                }
            }
            data
        };
        let mut world = World::new(16);
        let centre = ChunkCoordinate(I64Vec3::ZERO);
        world.insert_chunk(centre, filled(BlockType::Stone));
        // its diagonal neighbours don't matter, as faces only meet the chunks against them
        for side in centre.adjacent() {
            world.insert_chunk(side, filled(BlockType::Dirt));
        }
        assert!(world.is_chunk_buried(centre));

        // ice and leaves show what's behind them
        world.insert_chunk(ChunkCoordinate(I64Vec3::Y), filled(BlockType::Ice));
        assert!(!world.is_chunk_buried(centre));
        world.insert_chunk(ChunkCoordinate(I64Vec3::Y), ChunkData::default());
        assert!(!world.is_chunk_buried(centre));
        // nor is a chunk buried by one not generated yet
        world.clear_chunk(ChunkCoordinate(I64Vec3::Y));
        assert!(!world.is_chunk_buried(centre));
    }

    #[test]
    fn test_beyond_the_world_is_solid_below_and_open_above() {
        let mut world = World::new(16);
        let top = World::layers(world.height).end;
        let (below, inside, above) = (
            ChunkCoordinate(I64Vec3::new(3, -1, 2)),
            ChunkCoordinate(I64Vec3::new(3, 0, 2)),
            ChunkCoordinate(I64Vec3::new(3, top, 2)),
        );
        assert!(world.is_chunk_generated(below) && world.is_chunk_generated(above));
        assert!(!world.is_chunk_generated(inside));
        assert!(world.get_chunk_data(below).unwrap().opaque());
        assert!(world.is_chunk_empty(above));
        assert_eq!(BlockType::Stone, world.get_block_at(I64Vec3::new(0, -1, 0)));
    }

    #[test]
    fn test_is_chunk_generated() {}

    #[test]
    fn test_generate_chunk_updates_chunk_data() {}

    #[test]
    fn test_generate_chunk_mesh_none_for_ungenerated_chunk() {}

    #[test]
    fn test_generate_chunk_mesh_some_for_generated_chunk() {}
}
