use super::key::LodKey;
use super::loader::{lod_transform, GenerateLodMesh, LodLoader, LodNode};
use super::selection::{spawn_order, LOD_RESORT_INTERVAL};
use super::state::{lod_state, ChunkStates, LodState};
use crate::chunk::CHUNK_SIZE;
use crate::generate::columns::COARSE_CELL;
use crate::generate::generator::generate_lod_mesh;
use crate::generate::region_store::RegionStore;
use crate::generate::trees::TREE_HEIGHT;
use crate::generate::trees::TREE_REACH;
use crate::world::World;
use bevy::camera::visibility::Visibility;
use bevy::ecs::query::With;
use bevy::ecs::system::Commands;
use bevy::ecs::system::Local;
use bevy::ecs::system::Query;
use bevy::ecs::system::Res;
use bevy::ecs::system::ResMut;
use bevy::math::I64Vec2;
use bevy::tasks::AsyncComputeTaskPool;
use bevy::time::Time;
use ochre_base::WATER_LEVEL;

// LOD meshes take well under a millisecond, so enough are kept in flight that the
// once-per-frame refill isn't what limits how fast they load.
pub(super) const LOD_TASKS_PER_THREAD: usize = 32;

// Nodes known to have nothing to draw are added straight away without a task or an entity, up to this many a frame.
pub(super) const LOD_EMPTY_NODES_PER_FRAME: usize = 8192;

pub(crate) fn spawn_lods(
    mut commands: Commands,
    mut lods: ResMut<LodLoader>,
    world: Res<World>,
    time: Res<Time>,
    chunks: ChunkStates,
    pending: Query<(), With<GenerateLodMesh>>,
    mut since_resort: Local<f32>,
) {
    *since_resort += time.delta_secs();
    if *since_resort >= LOD_RESORT_INTERVAL {
        *since_resort = 0.0;
        let lods = &mut *lods;
        if let Some(anchor) = lods.anchor {
            let remaining = std::mem::take(&mut lods.to_spawn);
            lod_state!(state, lods, anchor, &lods.selected, &pending, |chunk| {
                chunks.get(chunk)
            });
            lods.to_spawn = spawn_order(&state, remaining.into_iter());
        }
    }

    let task_pool = AsyncComputeTaskPool::get();
    let max_in_flight = LOD_TASKS_PER_THREAD * task_pool.thread_num();
    let mut in_flight = pending.iter().count();
    let mut empty_nodes = 0;

    while empty_nodes < LOD_EMPTY_NODES_PER_FRAME {
        let Some(key) = lods.to_spawn.pop() else {
            break;
        };
        if lods.nodes.contains_key(&key) {
            continue;
        }
        let stand_in = lods.stand_ins.contains(&key);
        // a stand-in is only needed until its chunks are ready
        let chunks_ready = || key.chunks().all(|chunk| chunks.get(chunk).ready());
        if !lods.selected.contains(&key) && !(stand_in && !chunks_ready()) {
            continue;
        }
        // nothing to mesh, so the node is ready as soon as it exists
        if known_empty(key, &world.region_store) {
            lods.add_node(key, None);
            if stand_in {
                lods.retiring.push(key);
            }
            empty_nodes += 1;
            continue;
        }
        if in_flight >= max_in_flight {
            lods.to_spawn.push(key);
            break;
        }

        let regions = world.region_store.clone();
        let entity = commands
            .spawn((
                LodNode { key },
                GenerateLodMesh {
                    task: task_pool.spawn(async move {
                        generate_lod_mesh(regions, key).map(crate::group::pack_group)
                    }),
                },
                lod_transform(key),
                Visibility::Hidden,
            ))
            .id();
        lods.add_node(key, Some(entity));
        if stand_in {
            lods.retiring.push(key);
        }
        in_flight += 1;
    }
}

// Whether `key` is known to have nothing to draw, going by regions or coarse columns
// already sampled - never samples one, so it can run on the main thread. That's when the terrain its mesh
// would sample is all below it with only air or only water above, or all above it so it's
// buried in rock. Only knows about generated terrain, since LOD meshes are built from it
// alone - if they ever show player edits or caves, those must stop counting as empty here.
pub(super) fn known_empty(key: LodKey, regions: &RegionStore) -> bool {
    let min = key.min_block();
    let cell = key.cell_size();
    let width = key.width() * CHUNK_SIZE as i64;
    // meshes sample a cell past the node's edges, a block either side for slopes, and trees
    // standing just outside can reach in
    let margin = cell + 1 + TREE_REACH;
    let min_xz = I64Vec2::new(min.x, min.z) - margin;
    let max_xz = I64Vec2::new(min.x, min.z) + width - 1 + margin;
    let range = if cell >= COARSE_CELL {
        regions.column_height_range(key)
    } else {
        regions.loaded_height_range(min_xz, max_xz)
    };
    let Some((lowest, highest)) = range else {
        return false;
    };
    let (bottom, top) = (
        min.y - cell,
        min.y + width + cell * crate::generate::generator::SKIRT_CELLS as i64,
    );
    // coarse nodes' canopies stand at least a cell tall
    let above_terrain = bottom > highest.round() as i64 + 1 + TREE_HEIGHT.max(cell);
    let in_air = bottom > WATER_LEVEL;
    let under_water = top < WATER_LEVEL - 1;
    let buried = top < lowest.round() as i64 - 1;
    above_terrain && (in_air || under_water) || buried
}
