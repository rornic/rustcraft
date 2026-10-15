use super::key::LodKey;
use super::layout::LodLayout;
use super::loader::GenerateLodMesh;
use super::select::LodBand;
use crate::chunks::chunk::ChunkCoordinate;
use crate::chunks::chunk_loader::ChunkLoader;
use crate::chunks::chunk_loader::DirtyChunk;
use crate::chunks::chunk_loader::GenerateChunkData;
use crate::chunks::chunk_loader::GenerateChunkMesh;
use crate::chunks::group::GroupedChunk;
use crate::chunks::group::MeshedChunk;
use bevy::ecs::entity::Entity;
use bevy::ecs::query::Or;
use bevy::ecs::query::With;
use bevy::ecs::system::Query;
use bevy::ecs::system::Res;
use bevy::ecs::system::SystemParam;
use bevy::platform::collections::HashMap;
use bevy::platform::collections::HashSet;

/// What the retirement and visibility rules need to know about the current state.
pub struct LodState<'a> {
    pub anchor: ChunkCoordinate,
    pub layout: LodLayout,
    pub band: LodBand,
    pub selected: &'a HashSet<LodKey>,
    pub node_exists: &'a dyn Fn(LodKey) -> bool,
    pub node_ready: &'a dyn Fn(LodKey) -> bool,
    pub chunk_state: &'a dyn Fn(ChunkCoordinate) -> ChunkState,
}

impl LodState<'_> {
    fn selected_ancestor(&self, mut key: LodKey) -> Option<LodKey> {
        while key.level < self.layout.max_level {
            key = key.parent();
            if self.selected.contains(&key) {
                return Some(key);
            }
        }
        None
    }

    pub(super) fn has_existing_ancestor(&self, mut key: LodKey) -> bool {
        while key.level < self.layout.max_level {
            key = key.parent();
            if (self.node_exists)(key) {
                return true;
            }
        }
        false
    }

    pub fn can_retire(&self, key: LodKey) -> bool {
        match self.selected_ancestor(key) {
            Some(ancestor) => (self.node_ready)(ancestor),
            None => self.covered(key),
        }
    }

    pub(super) fn covered(&self, key: LodKey) -> bool {
        if self.selected.contains(&key) {
            return (self.node_ready)(key);
        }
        if !self.layout.splits(key, self.anchor) {
            return true;
        }
        if key.level == 1 {
            return key.chunks().all(|chunk| (self.chunk_state)(chunk).ready());
        }
        key.children()
            .into_iter()
            .all(|child| !self.band.intersects(child) || self.covered(child))
    }

    pub fn node_visible(&self, key: LodKey) -> bool {
        !self.selected.contains(&key) || !self.has_existing_ancestor(key)
    }

    pub fn chunk_visible(&self, chunk: ChunkCoordinate) -> bool {
        let key = LodKey::containing(chunk, 0);
        if self.layout.full_detail(chunk, self.anchor) {
            !self.has_existing_ancestor(key)
        } else {
            !self
                .selected_ancestor(key)
                .is_some_and(|ancestor| (self.node_ready)(ancestor))
        }
    }

    // Whether finer nodes, or for a level-1 node its chunks, are drawn where it goes.
    pub(super) fn finer_drawn(&self, key: LodKey) -> bool {
        if key.level == 1 {
            key.chunks()
                .any(|chunk| (self.chunk_state)(chunk) == ChunkState::Drawn)
        } else {
            key.children()
                .into_iter()
                .any(|child| (self.node_exists)(child))
        }
    }
}

pub(super) type PendingChunk = Or<(
    With<GenerateChunkData>,
    With<DirtyChunk>,
    With<GenerateChunkMesh>,
    With<MeshedChunk>,
)>;

/// How far along a full-detail chunk is, as far as LODs are concerned.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ChunkState {
    Missing,
    Loading,
    /// Loaded with nothing to draw.
    Empty,
    Drawn,
}

impl ChunkState {
    pub(super) fn ready(self) -> bool {
        matches!(self, ChunkState::Empty | ChunkState::Drawn)
    }
}

#[derive(SystemParam)]
pub struct ChunkStates<'w, 's> {
    pub(super) loader: Res<'w, ChunkLoader>,
    pending: Query<'w, 's, (), PendingChunk>,
    drawn: Query<'w, 's, (), With<GroupedChunk>>,
}

impl ChunkStates<'_, '_> {
    pub(super) fn get(&self, chunk: ChunkCoordinate) -> ChunkState {
        match self.loader.chunk_entity(chunk) {
            None => ChunkState::Missing,
            Some(e) if self.drawn.contains(e) => ChunkState::Drawn,
            Some(e) if self.pending.contains(e) => ChunkState::Loading,
            Some(_) => ChunkState::Empty,
        }
    }
}

// Whether the node at `key` exists and has its mesh, or has nothing to draw.
pub(super) fn node_ready(
    nodes: &HashMap<LodKey, Option<Entity>>,
    pending: &Query<(), With<GenerateLodMesh>>,
    key: LodKey,
) -> bool {
    nodes
        .get(&key)
        .is_some_and(|slot| slot.is_none_or(|entity| !pending.contains(entity)))
}
