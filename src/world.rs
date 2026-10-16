use std::{fmt::Debug, sync::Arc};

use bevy::prelude::Resource;
use bevy::{
    math::{I64Vec3, Vec3},
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

    pub fn insert_chunk(&mut self, coord: ChunkCoordinate, data: ChunkData) -> Arc<ChunkData> {
        let data = Arc::new(data);
        self.chunks.insert(coord, data.clone());
        data
    }

    /// The chunk layers the world is generated in, from the bottom up.
    pub fn layers(height: u64) -> std::ops::Range<i64> {
        0..height.div_ceil(CHUNK_SIZE as u64) as i64
    }

    // Below the world is solid and above it open sky, standing in for chunks that are never
    // generated, so the chunks at its bottom and top still have neighbours to mesh against.
    fn chunk(&self, coord: ChunkCoordinate) -> Option<&Arc<ChunkData>> {
        let layers = Self::layers(self.height);
        if coord.0.y < layers.start {
            Some(&self.below)
        } else if coord.0.y >= layers.end {
            Some(&self.above)
        } else {
            self.chunks.get(&coord)
        }
    }

    pub fn get_chunk_data(&self, coord: ChunkCoordinate) -> Option<Arc<ChunkData>> {
        self.chunk(coord).cloned()
    }

    pub fn clear_chunk(&mut self, coord: ChunkCoordinate) {
        self.chunks.remove(&coord);
    }

    pub fn adjacent_chunk_data(&self, coord: ChunkCoordinate) -> Vec<Option<Arc<ChunkData>>> {
        coord
            .neighbors_26()
            .into_iter()
            .map(|c| self.get_chunk_data(c))
            .collect()
    }

    pub fn is_chunk_generated(&self, coord: ChunkCoordinate) -> bool {
        self.chunk(coord).is_some()
    }

    /// Whether the chunk and the chunks against its faces are all opaque, so it has no faces to draw.
    pub fn is_chunk_buried(&self, coord: ChunkCoordinate) -> bool {
        let opaque = |c| self.chunk(c).is_some_and(|data| data.opaque());
        opaque(coord) && coord.adjacent().into_iter().all(opaque)
    }

    /// The block at `block`, or air in a chunk not generated yet.
    pub fn get_block_at(&self, block: I64Vec3) -> BlockType {
        let size = CHUNK_SIZE as i64;
        self.chunk(ChunkCoordinate::containing(block))
            .map_or(BlockType::Air, |data| {
                data.get_block_at(block.rem_euclid(I64Vec3::splat(size)).as_u16vec3())
            })
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
    use crate::chunks::chunk::{ChunkCoordinate, ChunkData, CHUNK_SIZE};

    #[test]
    fn test_chunk_is_buried_only_when_it_and_its_face_neighbours_are_opaque() {
        let filled = |block| {
            let mut data = ChunkData::default();
            for x in 0..CHUNK_SIZE {
                for y in 0..CHUNK_SIZE {
                    for z in 0..CHUNK_SIZE {
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
        let world = World::new(16);
        let top = World::layers(world.height).end;
        let (below, inside, above) = (
            ChunkCoordinate(I64Vec3::new(3, -1, 2)),
            ChunkCoordinate(I64Vec3::new(3, 0, 2)),
            ChunkCoordinate(I64Vec3::new(3, top, 2)),
        );
        assert!(world.is_chunk_generated(below) && world.is_chunk_generated(above));
        assert!(!world.is_chunk_generated(inside));
        assert!(world.get_chunk_data(below).unwrap().opaque());
        assert!(world.get_chunk_data(above).unwrap().empty());
        assert_eq!(BlockType::Stone, world.get_block_at(I64Vec3::new(0, -1, 0)));
    }
}
