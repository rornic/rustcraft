use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use bevy::prelude::Resource;
use bevy::{
    asset::{Assets, Handle},
    camera::{primitives::Aabb, visibility::Visibility},
    ecs::{
        component::Component,
        entity::Entity,
        query::{Added, Or, With, Without},
        system::{Commands, Local, Query, Res, ResMut, SystemParam},
    },
    math::{I64Vec2, I64Vec3, Vec3},
    pbr::MeshMaterial3d,
    platform::collections::{HashMap, HashSet},
    prelude::Mesh3d,
    render::mesh::Mesh,
    tasks::futures,
    tasks::{AsyncComputeTaskPool, Task},
    time::Time,
    transform::components::Transform,
};

use super::{
    chunk::{ChunkCoordinate, CHUNK_SIZE},
    chunk_loader::{ChunkLoader, DirtyChunk, GenerateChunkData, GenerateChunkMesh},
    fade::{fade_out_and_despawn, set_shown, Fade, SolidWater},
    generate::{
        columns::COARSE_CELL,
        generator::{generate_lod_mesh, ChunkMeshes},
        region_store::RegionStore,
        trees::{TREE_HEIGHT, TREE_REACH},
        WATER_LEVEL,
    },
    group::{BuiltGroup, ChunkGroup, GroupedChunk, MeshedChunk},
    material::{ChunkMaterial, WaterMaterial},
};
use crate::world::World;

mod apply;
mod key;
mod layout;
mod loader;
mod select;
mod selection;
mod spawn;
mod state;
#[cfg(test)]
mod tests;

pub use apply::{load_lods, refresh_lod_visibility, retire_lods, SOLID_WATER_LEVEL};
pub use key::LodKey;
pub use layout::LodLayout;
pub use loader::{GenerateLodMesh, LodLoader, LodNode};
pub use select::{select_lods, select_lods_and_stand_ins, LodBand};
pub use selection::{update_lod_selection, LodSelection};
pub use spawn::spawn_lods;
pub use state::{ChunkState, ChunkStates, LodState};

#[allow(unused_imports)]
use self::{apply::*, key::*, layout::*, loader::*, select::*, selection::*, spawn::*, state::*};
