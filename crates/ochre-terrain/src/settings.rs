use serde::Deserialize;

use crate::lod::LodLayout;

#[derive(Deserialize, Clone, Copy)]
#[serde(default)]
pub struct RendererSettings {
    /// How far terrain is drawn, in chunks.
    pub render_distance: u32,
    /// How far terrain is drawn at full detail, in chunks.
    pub full_detail_distance: u32,
    /// Each LOD level reaches this many times as far as the one before it, starting
    /// from `lod_base_distance`: level n reaches it times this to the n.
    pub lod_level_scale: u32,
    /// The distance, in chunks, LOD levels scale out from, the full-detail distance if unset.
    /// Full detail reaching further than this only narrows the first LOD level's ring.
    pub lod_base_distance: Option<u32>,
    /// How long swapping between LOD levels takes to cross-fade, in seconds.
    pub lod_fade_seconds: f32,
    /// How far the sun casts shadows, in chunks.
    pub shadow_distance: u32,
    /// Shadow map cascades, each drawing the terrain in its slice again.
    pub shadow_cascades: usize,
    /// The most megabytes in one buffer the terrain's meshes are packed into. A buffer that fills
    /// is copied into a bigger one, with both alive for a moment, so smaller buffers mean a lower
    /// peak and less empty room in the last one.
    pub mesh_slab_mb: u64,
    /// How much bigger a mesh buffer is made each time it fills, over 1.
    pub mesh_slab_growth: f64,
    /// Pack terrain meshes' vertices into about half the bytes, see `crate::pack`. On unless a
    /// test of the two needs it off.
    pub pack_meshes: bool,
}

/// Switches for testing what the renderer spends its time on, all off by default.
#[derive(Default, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(default)]
pub struct DebugSettings {
    /// Leaves stay still instead of swaying in the wind.
    pub still_leaves: bool,
    /// Leaves are drawn in their mean colour without their holes, in shadows too, and hide
    /// the faces of the leaves behind them, as coarse LODs draw them everywhere.
    pub solid_leaves: bool,
}

impl RendererSettings {
    /// The renderer's mesh allocator, with these slab limits in place of Bevy's 512 MB and 1.5x.
    pub fn mesh_allocator(&self) -> bevy::render::mesh::allocator::MeshAllocatorSettings {
        let mut settings = bevy::render::mesh::allocator::MeshAllocatorSettings::default();
        settings.max_slab_size = self.mesh_slab_mb.max(1) * 1024 * 1024;
        // a buffer is never too small to hold the biggest thing that goes in a shared one
        settings.large_threshold = settings.large_threshold.min(settings.max_slab_size / 2);
        settings.growth_factor = self.mesh_slab_growth.max(1.05);
        settings
    }

    pub fn lod_layout(&self) -> LodLayout {
        LodLayout::new(
            self.lod_base_distance.unwrap_or(self.full_detail_distance),
            self.render_distance,
            self.lod_level_scale,
        )
        .with_full_detail_distance(self.full_detail_distance)
    }
}

impl Default for RendererSettings {
    fn default() -> Self {
        Self {
            render_distance: 8,
            full_detail_distance: 32,
            lod_level_scale: 4,
            lod_base_distance: None,
            lod_fade_seconds: 1.5,
            shadow_distance: 64,
            shadow_cascades: 3,
            mesh_slab_mb: 64,
            mesh_slab_growth: 1.25,
            pack_meshes: true,
        }
    }
}
