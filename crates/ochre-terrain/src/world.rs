use std::{fmt::Debug, sync::Arc};

use bevy::prelude::Resource;
use bevy::{
    math::{I64Vec3, Vec3},
    platform::collections::{HashMap, HashSet},
};

use crate::generate::region_store::{resident_cap_for, RegionStore};
use ochre_base::block::BlockType;
use ochre_base::terrain_source::TerrainSource;
use ochre_base::WORLD_HEIGHT;

use crate::chunk::{ChunkCoordinate, ChunkData, CHUNK_SIZE};

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
    // blocks changed since `take_edits` was last asked, whose chunks need meshing again
    edits: Vec<I64Vec3>,
    // blocks whose water may flow differently since `take_stirred`: changed ones and those beside them
    stirred: HashSet<I64Vec3>,
}

impl World {
    /// A world over `terrain`, spawning on its nearest lowland.
    pub fn with_terrain(seed: u32, region_distance: u32, terrain: Arc<dyn TerrainSource>) -> Self {
        let height = WORLD_HEIGHT;
        let (mut spawn_x, mut spawn_z, mut spawn_height) = terrain.find_spawn();
        // OCHRE_SPAWN=x,z[,lift] starts the player there, lift blocks over the ground (20 by default)
        let mut lift = 20.0;
        if let Some((x, z, up)) = std::env::var("OCHRE_SPAWN").ok().and_then(|s| {
            let mut parts = s.split(',').map(|p| p.trim().parse::<f64>().ok());
            Some((parts.next()??, parts.next()??, parts.next().flatten()))
        }) {
            (spawn_x, spawn_z) = (x as i64, z as i64);
            spawn_height = terrain.sample(x, z).height;
            lift = up.unwrap_or(lift);
        }
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
                RegionStore::new(terrain)
                    .with_resident_cap(resident_cap_for(region_distance_blocks)),
            ),
            spawn: Vec3::new(
                spawn_x as f32,
                spawn_height as f32 + lift as f32,
                spawn_z as f32,
            ),
            edits: Vec::new(),
            stirred: HashSet::new(),
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

    /// Sets the block at `block`, in a chunk generated already, and remembers it was changed, see
    /// `take_edits`. Returns whether it changed.
    pub fn set_block_at(&mut self, block: I64Vec3, to: BlockType) -> bool {
        let size = CHUNK_SIZE as i64;
        // only the world's own chunks: what lies beyond it isn't for changing
        let Some(data) = self.chunks.get_mut(&ChunkCoordinate::containing(block)) else {
            return false;
        };
        let local = block.rem_euclid(I64Vec3::splat(size)).as_u16vec3();
        if data.get_block_at(local) == to {
            return false;
        }
        // a copy, if meshing still holds this one
        Arc::make_mut(data).set_block_at(local, to);
        self.changed(block);
        true
    }

    /// The level of the water flowing at `block`, None for still water or any other block.
    pub fn flow_at(&self, block: I64Vec3) -> Option<u8> {
        let size = CHUNK_SIZE as i64;
        self.chunk(ChunkCoordinate::containing(block))
            .and_then(|data| data.flow_at(block.rem_euclid(I64Vec3::splat(size)).as_u16vec3()))
    }

    /// Makes the block at `block` water flowing at `level`, or still if None, in a chunk generated
    /// already. Returns whether it changed.
    pub(crate) fn set_flow_at(&mut self, block: I64Vec3, level: Option<u8>) -> bool {
        let size = CHUNK_SIZE as i64;
        let Some(data) = self.chunks.get_mut(&ChunkCoordinate::containing(block)) else {
            return false;
        };
        let local = block.rem_euclid(I64Vec3::splat(size)).as_u16vec3();
        if data.get_block_at(local) == BlockType::Water && data.flow_at(local) == level {
            return false;
        }
        Arc::make_mut(data).set_flow_at(local, level);
        self.changed(block);
        true
    }

    fn changed(&mut self, block: I64Vec3) {
        self.edits.push(block);
        self.stirred.insert(block);
        self.stirred
            .extend(crate::flow::SIDES.map(|side| block + side));
        self.stirred
            .extend([block + I64Vec3::Y, block - I64Vec3::Y]);
    }

    /// The blocks stirred since this was last asked, in a fixed order.
    pub(crate) fn take_stirred(&mut self) -> Vec<I64Vec3> {
        let mut stirred: Vec<_> = self.stirred.drain().collect();
        stirred.sort_unstable_by_key(|block| block.to_array());
        stirred
    }

    /// Breaks the block at `block`, leaving air, unless it's one that can't be broken. Returns
    /// what was broken.
    pub fn break_block(&mut self, block: I64Vec3) -> Option<BlockType> {
        let broken = self.get_block_at(block);
        (broken.is_breakable() && self.set_block_at(block, BlockType::Air)).then_some(broken)
    }

    /// The blocks changed since this was last asked.
    pub fn take_edits(&mut self) -> Vec<I64Vec3> {
        std::mem::take(&mut self.edits)
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
    use crate::chunk::{ChunkCoordinate, ChunkData, CHUNK_SIZE};
    use ochre_base::block::BlockType;

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
        let mut world = World::with_terrain(1, 16, ochre_world::worldmap::test_terrain(1));
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
    fn test_breaking_leaves_air_and_remembers_the_edit() {
        let mut world = World::with_terrain(1, 16, ochre_world::worldmap::test_terrain(1));
        let mut data = ChunkData::default();
        data.set_block_at(bevy::math::U16Vec3::new(1, 2, 3), BlockType::Dirt);
        let held = world.insert_chunk(ChunkCoordinate(I64Vec3::new(-1, 0, 0)), data);
        let block = I64Vec3::new(-15, 2, 3);

        assert_eq!(Some(BlockType::Dirt), world.break_block(block));
        assert_eq!(BlockType::Air, world.get_block_at(block));
        assert_eq!(vec![block], world.take_edits());
        assert!(world.take_edits().is_empty());
        // what meshing was handed before is left as it was
        assert_eq!(
            BlockType::Dirt,
            held.get_block_at(bevy::math::U16Vec3::new(1, 2, 3))
        );
        // and there's nothing left to break
        assert_eq!(None, world.break_block(block));
        assert!(world.take_edits().is_empty());
    }

    #[test]
    fn test_compacted_stone_air_and_ungenerated_chunks_are_not_broken() {
        let mut world = World::with_terrain(1, 16, ochre_world::worldmap::test_terrain(1));
        let mut data = ChunkData::default();
        data.set_block_at(bevy::math::U16Vec3::ZERO, BlockType::CompactedStone);
        world.insert_chunk(ChunkCoordinate(I64Vec3::ZERO), data);

        assert_eq!(None, world.break_block(I64Vec3::ZERO));
        assert_eq!(BlockType::CompactedStone, world.get_block_at(I64Vec3::ZERO));
        assert_eq!(None, world.break_block(I64Vec3::new(1, 0, 0)));
        // not generated, and under the world
        assert_eq!(None, world.break_block(I64Vec3::new(100, 0, 0)));
        assert_eq!(None, world.break_block(I64Vec3::new(0, -1, 0)));
        assert!(world.take_edits().is_empty());
    }

    #[test]
    fn test_beyond_the_world_is_solid_below_and_open_above() {
        let world = World::with_terrain(1, 16, ochre_world::worldmap::test_terrain(1));
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
