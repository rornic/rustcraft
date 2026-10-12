pub mod chunk;
pub mod chunk_loader;
pub mod fade;
pub mod generate;
pub mod lod;
pub mod material;

use bevy::{
    app::{App, Plugin, Update},
    asset::{Assets, Handle},
    core::{TaskPoolOptions, TaskPoolPlugin, TaskPoolThreadAssignmentPolicy},
    ecs::{schedule::IntoSystemConfigs, system::Commands},
};

use crate::settings::{DebugSettings, RendererSettings};
use chunk_loader::{
    gather_chunks, generate_chunks, load_chunks, mark_chunks, unload_chunks, ChunkLoader,
};
use fade::{apply_fades, FadeMaterials};
use lod::{
    load_lods, refresh_lod_visibility, retire_lods, spawn_lods, update_lod_selection, LodLoader,
};
use material::{ChunkMaterial, WaterMaterial};

/// Loads, meshes and unloads full-detail chunks and LOD terrain around the camera.
pub struct TerrainPlugin;

impl Plugin for TerrainPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                (gather_chunks, generate_chunks, mark_chunks, load_chunks).before(unload_chunks),
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
            },
            async_compute: TaskPoolThreadAssignmentPolicy {
                min_threads: 1,
                max_threads: usize::MAX,
                percent: 0.6,
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
    chunk_material: Handle<ChunkMaterial>,
    water_material: Handle<WaterMaterial>,
    chunk_materials: &mut Assets<ChunkMaterial>,
    water_materials: &mut Assets<WaterMaterial>,
) {
    let layout = renderer.lod_layout();
    commands.insert_resource(FadeMaterials::new(
        renderer.lod_fade_seconds,
        chunk_material.clone(),
        water_material.clone(),
        chunk_materials,
        water_materials,
    ));
    commands.insert_resource(LodLoader::new(
        layout,
        world_height,
        chunk_material.clone(),
        water_material.clone(),
    ));
    commands.insert_resource(ChunkLoader::new(
        layout,
        debug.solid_leaves,
        chunk_material,
        water_material,
    ));
}
