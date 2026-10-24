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
pub mod raycast;
pub mod remesh;
pub mod settings;
pub mod world;

use bevy::{
    app::{App, Plugin, Update},
    app::{TaskPoolOptions, TaskPoolPlugin, TaskPoolThreadAssignmentPolicy},
    ecs::{schedule::IntoScheduleConfigs, system::Commands},
    pbr::MaterialPlugin,
    render::RenderApp,
};

use crate::settings::{DebugSettings, RendererSettings};
use chunk_loader::{
    gather_chunks, generate_chunks, load_chunks, mark_chunks, unload_chunks, ChunkLoader,
};
use curvature::CurvaturePlugin;
use fade::{apply_fades, FadeMaterials};
use group::{build_groups, load_groups, show_inner_leaves};
use lod::{
    load_lods, refresh_lod_visibility, retire_lods, spawn_lods, update_lod_selection, LodLoader,
};
use material::{
    ChunkMaterial, ShaderInputs, SprayMaterial, TerrainAssets, TerrainMaterials, WaterMaterial,
};
use remesh::{apply_remeshes, remesh_edits};

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
                // after the groups loading this frame, so one built from meshes made before a
                // change is meshed again
                (remesh_edits, apply_remeshes)
                    .chain()
                    .after(load_groups)
                    .before(unload_chunks),
                unload_chunks,
                // sees the inner leaves `apply_remeshes` spawns hidden this frame, or they flash
                show_inner_leaves.after(apply_remeshes),
            ),
        );
    }
}

/// Draws the terrain: its materials, the bend of the land away to the horizon, and the renderer's
/// mesh buffers. Needs a window and GPU, so a headless run goes without.
pub struct TerrainRenderPlugin {
    pub renderer: RendererSettings,
}

impl Plugin for TerrainRenderPlugin {
    fn build(&self, app: &mut App) {
        pack::set_packed(self.renderer.pack_meshes);
        app.add_plugins((
            MaterialPlugin::<ChunkMaterial>::default(),
            MaterialPlugin::<WaterMaterial>::default(),
            MaterialPlugin::<SprayMaterial>::default(),
            CurvaturePlugin,
        ));
        // the terrain's meshes share a few big buffers, see `RendererSettings::mesh_slab_mb`
        if let Some(render) = app.get_sub_app_mut(RenderApp) {
            render.insert_resource(self.renderer.mesh_allocator());
        }
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

/// Inserts the resources `TerrainPlugin`'s systems run on, drawing with materials built from `inputs`.
pub fn insert_terrain_resources(
    commands: &mut Commands,
    renderer: &RendererSettings,
    debug: DebugSettings,
    world_height: u64,
    inputs: &ShaderInputs,
    assets: &mut TerrainAssets,
) {
    let layout = renderer.lod_layout();
    let materials = TerrainMaterials::new(debug, inputs, assets);
    commands.insert_resource(FadeMaterials::new(
        renderer.lod_fade_seconds,
        &materials,
        &mut assets.chunk,
        &mut assets.water,
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
