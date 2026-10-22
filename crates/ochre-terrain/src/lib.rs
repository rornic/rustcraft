//! Voxel terrain: chunk storage and generation, meshing, octree LODs, and the world they make.

pub mod chunk;
pub mod chunk_loader;
pub mod curvature;
pub mod fade;
pub mod generate;
pub mod group;
pub mod horizon;
pub mod lod;
pub mod material;
pub mod pack;
pub mod settings;
pub mod world;

use bevy::{
    app::{App, Plugin, Update},
    app::{TaskPoolOptions, TaskPoolPlugin, TaskPoolThreadAssignmentPolicy},
    asset::Assets,
    ecs::{schedule::IntoScheduleConfigs, system::Commands},
};

use crate::settings::{DebugSettings, RendererSettings};
use chunk_loader::{
    gather_chunks, generate_chunks, load_chunks, mark_chunks, unload_chunks, ChunkLoader,
};
use fade::{apply_fades, FadeMaterials};
use group::{build_groups, load_groups, show_inner_leaves};
use lod::{
    load_lods, refresh_lod_visibility, retire_lods, spawn_lods, update_lod_selection, LodLoader,
};
use material::{ChunkMaterial, TerrainMaterials, WaterMaterial};

/// Loads, meshes and unloads full-detail chunks and LOD terrain around the camera.
pub struct TerrainPlugin;

impl Plugin for TerrainPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                (
                    gather_chunks,
                    generate_chunks,
                    mark_chunks,
                    load_chunks,
                    build_groups,
                    load_groups,
                )
                    .before(unload_chunks),
                (
                    update_lod_selection,
                    spawn_lods,
                    load_lods,
                    retire_lods,
                    refresh_lod_visibility,
                    apply_fades,
                )
                    .chain()
                    .after(gather_chunks)
                    .before(unload_chunks),
                unload_chunks,
                show_inner_leaves.after(load_groups),
            ),
        );
    }
}

/// Terrain generation runs on the async compute pool, so it gets most of the cores.
pub fn task_pool_plugin() -> TaskPoolPlugin {
    TaskPoolPlugin {
        task_pool_options: TaskPoolOptions {
            io: TaskPoolThreadAssignmentPolicy {
                min_threads: 1,
                max_threads: 1,
                percent: 0.0,
                on_thread_spawn: None,
                on_thread_destroy: None,
            },
            async_compute: TaskPoolThreadAssignmentPolicy {
                min_threads: 1,
                max_threads: usize::MAX,
                percent: 0.6,
                on_thread_spawn: None,
                on_thread_destroy: None,
            },
            ..Default::default()
        },
    }
}

/// Inserts the resources `TerrainPlugin`'s systems run on, drawing with the given materials.
pub fn insert_terrain_resources(
    commands: &mut Commands,
    renderer: &RendererSettings,
    debug: &DebugSettings,
    world_height: u64,
    materials: TerrainMaterials,
    chunk_materials: &mut Assets<ChunkMaterial>,
    water_materials: &mut Assets<WaterMaterial>,
) {
    let layout = renderer.lod_layout();
    commands.insert_resource(FadeMaterials::new(
        renderer.lod_fade_seconds,
        &materials,
        chunk_materials,
        water_materials,
    ));
    commands.insert_resource(LodLoader::new(
        layout,
        world_height,
        materials.chunk.clone(),
        materials.water.clone(),
        materials.solid_water,
    ));
    commands.insert_resource(ChunkLoader::new(
        layout,
        world_height,
        debug.solid_leaves,
        materials.chunk,
        materials.water,
        materials.spray,
    ));
}
