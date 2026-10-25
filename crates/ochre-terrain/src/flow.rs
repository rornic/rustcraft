//! Water flowing out of the world's still water into the space edits open beside it, as
//! Minecraft's does. It falls as far as it can and spreads sideways from where it rests, a level
//! less each block, so it runs `SPREAD` blocks over flat ground and a channel carries it as far as
//! it keeps falling. It flows only while fed: cut off, it dries up back from the cut. Still water,
//! the water the world was generated with, never moves.

use bevy::{
    ecs::system::{Local, Res, ResMut},
    math::I64Vec3,
    time::Time,
};

use crate::chunk::ChunkCoordinate;
use crate::world::World;
use ochre_base::block::BlockType;

pub(crate) const SIDES: [I64Vec3; 4] = [I64Vec3::X, I64Vec3::NEG_X, I64Vec3::Z, I64Vec3::NEG_Z];

/// The level of water fed from above.
pub const FALLING: u8 = 8;
/// The level still water, and a fall where it lands, spread at, one less each block on.
pub const SPREAD: u8 = 7;

/// How far over the bottom of its block flowing water at `level` stands, in blocks, as still
/// water stands a whole block.
pub fn surface(level: u8) -> f32 {
    match level {
        FALLING => 1.0,
        level => (level + 1) as f32 / (SPREAD + 2) as f32,
    }
}

// Seconds between steps of the flow, each moving it a block.
const STEP: f32 = 0.25;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Cell {
    Still,
    Flowing(u8),
    // what water can flow into
    Open,
    Solid,
}

fn cell(world: &World, block: I64Vec3) -> Cell {
    // a chunk not generated yet holds water back until it is
    if !world.is_chunk_generated(ChunkCoordinate::containing(block)) {
        return Cell::Solid;
    }
    match world.get_block_at(block) {
        BlockType::Water => world.flow_at(block).map_or(Cell::Still, Cell::Flowing),
        BlockType::Air => Cell::Open,
        flora if flora.is_flora() => Cell::Open,
        _ => Cell::Solid,
    }
}

// The level water at `from` spreads to the blocks beside it at: it spreads where it rests, not as it falls past.
fn spreads(world: &World, from: I64Vec3) -> Option<u8> {
    let below = cell(world, from - I64Vec3::Y);
    match cell(world, from) {
        Cell::Still => (below != Cell::Open).then_some(SPREAD),
        Cell::Flowing(level) => {
            (below == Cell::Solid && level > 1).then(|| level.min(SPREAD + 1) - 1)
        }
        _ => None,
    }
}

// The level the water at `block` is fed to, None for none.
fn fed(world: &World, block: I64Vec3) -> Option<u8> {
    if matches!(
        cell(world, block + I64Vec3::Y),
        Cell::Still | Cell::Flowing(_)
    ) {
        return Some(FALLING);
    }
    // water meeting still water joins it rather than spreading over it, as over a river's steps down its course
    if cell(world, block - I64Vec3::Y) == Cell::Still {
        return None;
    }
    SIDES
        .into_iter()
        .filter_map(|side| spreads(world, block + side))
        .max()
}

/// Moves the water a block on wherever it was stirred, each block from how the water round it was
/// before the step. Returns how many blocks changed.
pub(crate) fn step(world: &mut World) -> usize {
    let changes: Vec<(I64Vec3, Option<u8>)> = world
        .take_stirred()
        .into_iter()
        .filter_map(|block| {
            let now = match cell(world, block) {
                Cell::Flowing(level) => Some(level),
                Cell::Open => None,
                Cell::Still | Cell::Solid => return None,
            };
            let fed = fed(world, block);
            (fed != now).then_some((block, fed))
        })
        .collect();
    for &(block, level) in &changes {
        match level {
            Some(level) => world.set_flow_at(block, Some(level)),
            None => world.set_block_at(block, BlockType::Air),
        };
    }
    changes.len()
}

pub(crate) fn flow_water(time: Res<Time>, mut world: ResMut<World>, mut since: Local<f32>) {
    *since += time.delta_secs();
    if *since < STEP {
        return;
    }
    // a slow frame doesn't run several steps to catch up
    *since = 0.0;
    step(&mut world);
}

#[cfg(test)]
mod tests {
    use bevy::math::{I64Vec3, U16Vec3};

    use super::{step, FALLING, SPREAD};
    use crate::chunk::{ChunkCoordinate, ChunkData, CHUNK_SIZE};
    use crate::world::World;
    use ochre_base::block::BlockType;

    const FLOOR: i64 = 5;
    const ROW: i64 = 8;
    const DAM: I64Vec3 = I64Vec3::new(1, FLOOR, ROW);

    // A stone chunk with a row of air along x at `FLOOR`, a still pool at its x = 0 end and a dam beside it.
    fn channel() -> World {
        let mut world = World::with_terrain(1, 16, ochre_world::worldmap::test_terrain(1));
        let mut data = ChunkData::from_cells(std::iter::repeat_n(
            BlockType::Stone,
            (CHUNK_SIZE as usize).pow(3),
        ));
        for x in 2..CHUNK_SIZE {
            data.set_block_at(U16Vec3::new(x, FLOOR as u16, ROW as u16), BlockType::Air);
        }
        data.set_flow_at(U16Vec3::new(0, FLOOR as u16, ROW as u16), None);
        world.insert_chunk(ChunkCoordinate(I64Vec3::ZERO), data);
        world
    }

    fn settle(world: &mut World) -> usize {
        (1..200)
            .find(|_| step(world) == 0)
            .expect("the water settles")
    }

    fn levels(world: &World, y: i64) -> Vec<Option<u8>> {
        (1..CHUNK_SIZE as i64)
            .map(|x| {
                let block = I64Vec3::new(x, y, ROW);
                (world.get_block_at(block) == BlockType::Water)
                    .then(|| world.flow_at(block).unwrap())
            })
            .collect()
    }

    #[test]
    fn test_water_spreads_a_level_less_each_block_until_it_runs_out() {
        let mut world = channel();
        assert_eq!(0, step(&mut world));
        world.set_block_at(DAM, BlockType::Air);
        settle(&mut world);
        let wet: Vec<_> = (1..=SPREAD).rev().map(Some).collect();
        assert_eq!(wet, levels(&world, FLOOR)[..SPREAD as usize]);
        assert!(levels(&world, FLOOR)[SPREAD as usize..]
            .iter()
            .all(Option::is_none));
        // the still water it came from stays as it was
        assert_eq!(None, world.flow_at(I64Vec3::new(0, FLOOR, ROW)));
    }

    #[test]
    fn test_water_falls_down_a_hole_rather_than_spreading_past_it() {
        let mut world = channel();
        for y in FLOOR - 3..FLOOR {
            world.set_block_at(I64Vec3::new(3, y, ROW), BlockType::Air);
        }
        world.set_block_at(DAM, BlockType::Air);
        settle(&mut world);
        assert_eq!(
            vec![Some(SPREAD), Some(SPREAD - 1), Some(SPREAD - 2), None],
            levels(&world, FLOOR)[..4]
        );
        for y in FLOOR - 3..FLOOR {
            assert_eq!(Some(FALLING), world.flow_at(I64Vec3::new(3, y, ROW)));
        }
    }

    #[test]
    fn test_water_doesnt_spread_over_still_water_a_step_down() {
        let mut world = channel();
        world.set_flow_at(I64Vec3::new(2, FLOOR - 1, ROW), None);
        world.set_block_at(DAM, BlockType::Air);
        settle(&mut world);
        assert_eq!(vec![Some(SPREAD), None, None], levels(&world, FLOOR)[..3]);
    }

    #[test]
    fn test_water_cut_off_from_its_source_dries_up() {
        let mut world = channel();
        world.set_block_at(DAM, BlockType::Air);
        settle(&mut world);
        world.set_block_at(DAM, BlockType::Stone);
        settle(&mut world);
        assert!(levels(&world, FLOOR).iter().skip(1).all(Option::is_none));
        assert_eq!(
            BlockType::Air,
            world.get_block_at(I64Vec3::new(2, FLOOR, ROW))
        );
    }
}
