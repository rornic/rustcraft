use super::key::LodKey;
use super::loader::{GenerateLodMesh, LodLoader};
use super::select::select_lods_and_stand_ins;
use super::state::{lod_state, ChunkStates, LodState};
use crate::chunk::ChunkCoordinate;
use bevy::ecs::query::With;
use bevy::ecs::system::Query;
use bevy::ecs::system::ResMut;
use bevy::platform::collections::HashSet;
use bevy::tasks::futures;
use bevy::tasks::AsyncComputeTaskPool;
use std::sync::Arc;

// The spawn queue is re-sorted this often, since nodes drift between covered and gap as terrain loads.
pub(super) const LOD_RESORT_INTERVAL: f32 = 2.0;

// Gaps with nothing drawn yet count as this fraction of their distance, so holes fill in
// quickly without holding up more detail near the player.
pub(super) const GAP_DISTANCE_WEIGHT: f32 = 0.5;

// Nodes to spawn, popped from the end: chunk swaps first, then nearest first, with gaps
// counted as nearer than they are.
pub(super) fn spawn_order(
    state: &LodState,
    unspawned: impl Iterator<Item = LodKey>,
) -> Vec<LodKey> {
    let mut queued: Vec<(bool, f32, LodKey)> = unspawned
        .map(|key| {
            let distance = key.distance(state.anchor);
            let finer_drawn = state.finer_drawn(key);
            let gap = !finer_drawn && !state.has_existing_ancestor(key);
            (
                // a level-1 node standing in for chunks that are still drawn
                key.level == 1 && finer_drawn,
                if gap {
                    distance * GAP_DISTANCE_WEIGHT
                } else {
                    distance
                },
                key,
            )
        })
        .collect();
    queued.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.total_cmp(&a.1)));
    queued.into_iter().map(|(.., key)| key).collect()
}

/// The nodes chosen for an anchor, and how they differ from those chosen before.
pub struct LodSelection {
    generation: u32,
    anchor: ChunkCoordinate,
    selected: HashSet<LodKey>,
    stand_ins: HashSet<LodKey>,
    // keys that entered or left either set, or none for a first selection
    changed: Vec<LodKey>,
}

pub(crate) fn update_lod_selection(
    mut lods: ResMut<LodLoader>,
    chunks: ChunkStates,
    pending_lods: Query<(), With<GenerateLodMesh>>,
) {
    if let Some(mut task) = lods.selecting.take() {
        match futures::check_ready(&mut task) {
            // one chosen under a layout since replaced is thrown away
            Some(done) if done.generation == lods.generation => {
                apply_selection(&mut lods, done, &chunks, &pending_lods)
            }
            Some(_) => {}
            None => lods.selecting = Some(task),
        }
    }

    // the walk is the slow part, so it runs off the main thread, a camera move behind
    let anchor = chunks.loader.anchor();
    if lods.selecting.is_some() || lods.anchor == Some(anchor) {
        return;
    }
    let (layout, band, generation) = (lods.layout, lods.band, lods.generation);
    let previous = lods
        .anchor
        .map(|_| (lods.selected.clone(), lods.stand_ins.clone()));
    lods.selecting = Some(AsyncComputeTaskPool::get().spawn(async move {
        let (selected, stand_ins) = select_lods_and_stand_ins(anchor, layout, band);
        // whether a node or chunk shows depends on the anchor only through these two sets, so
        // only where they changed needs rechecking
        let changed = previous
            .map(|(old_selected, old_stand_ins)| {
                selected
                    .symmetric_difference(&old_selected)
                    .chain(stand_ins.symmetric_difference(&old_stand_ins))
                    .copied()
                    .collect()
            })
            .unwrap_or_default();
        LodSelection {
            generation,
            anchor,
            selected,
            stand_ins,
            changed,
        }
    }));
}

pub(super) fn apply_selection(
    lods: &mut LodLoader,
    done: LodSelection,
    chunks: &ChunkStates,
    pending_lods: &Query<(), With<GenerateLodMesh>>,
) {
    let LodSelection {
        anchor,
        selected,
        stand_ins,
        changed,
        ..
    } = done;
    let first = lods.anchor.is_none();
    lod_state!(state, lods, anchor, &selected, pending_lods, |chunk| chunks
        .get(chunk));
    let nodes = &lods.nodes;

    // The first selection, and one for a new layout, start from scratch and check everything.
    // After that only the keys that changed can need spawning or retiring, so nothing walks
    // every node, of which there can be hundreds of thousands.
    let wanted =
        |key: &LodKey| selected.contains(key) || (stand_ins.contains(key) && !state.covered(*key));
    let (to_spawn, retiring) = if first {
        let to_spawn = spawn_order(
            &state,
            selected
                .iter()
                .chain(stand_ins.iter().filter(|&&key| !state.covered(key)))
                .filter(|&key| !nodes.contains_key(key))
                .copied(),
        );
        let retiring = nodes
            .keys()
            .filter(|&key| !selected.contains(key))
            .copied()
            .collect();
        (to_spawn, retiring)
    } else {
        // new arrivals go on the end, so are spawned first until the queue is next sorted
        let mut to_spawn = std::mem::take(&mut lods.to_spawn);
        to_spawn.extend(spawn_order(
            &state,
            changed
                .iter()
                .copied()
                .filter(|key| wanted(key) && !nodes.contains_key(key)),
        ));
        let mut retiring = std::mem::take(&mut lods.retiring);
        retiring.retain(|key| !selected.contains(key) && nodes.contains_key(key));
        let queued: HashSet<LodKey> = retiring.iter().copied().collect();
        retiring.extend(changed.iter().copied().filter(|key| {
            !selected.contains(key) && nodes.contains_key(key) && !queued.contains(key)
        }));
        (to_spawn, retiring)
    };

    if first {
        lods.refresh_all = true;
    } else {
        lods.dirty_areas.extend(changed);
    }
    lods.anchor = Some(anchor);
    lods.selected = Arc::new(selected);
    lods.stand_ins = Arc::new(stand_ins);
    lods.to_spawn = to_spawn;
    lods.retiring = retiring;
}
