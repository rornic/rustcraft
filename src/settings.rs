use std::error::Error;

use bevy::ecs::component::Component;
use serde::Deserialize;

use crate::chunks::lod::LodLayout;
use crate::worldmap::WorldConfig;

#[derive(Default, Deserialize, Clone, Copy, Component)]
pub struct Settings {
    pub renderer: RendererSettings,
    #[serde(default)]
    pub debug: DebugSettings,
    #[serde(default)]
    pub world: WorldConfig,
}

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

impl Settings {
    pub fn read(file: &str) -> Result<Self, Box<dyn Error>> {
        Ok(toml::from_str(&std::fs::read_to_string(file)?)?)
    }
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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Settings;
    use crate::worldmap::WorldConfig;

    #[test]
    fn test_missing_renderer_settings_use_defaults() {
        let settings: Settings = toml::from_str("[renderer]\nrender_distance = 64").unwrap();
        assert_eq!(64, settings.renderer.render_distance);
        assert_eq!(32, settings.renderer.full_detail_distance);
        assert!(!settings.debug.still_leaves && !settings.debug.solid_leaves);
        assert_eq!(WorldConfig::default(), settings.world);
    }

    #[test]
    fn test_world_size_is_config() {
        let settings: Settings =
            toml::from_str("[renderer]\n[world]\nsize = 16384\nmap_cell = 16").unwrap();
        assert_eq!(1024, settings.world.dim());
    }

    #[test]
    fn test_debug_settings_parse() {
        let settings: Settings =
            toml::from_str("[renderer]\n[debug]\nsolid_leaves = true").unwrap();
        assert!(settings.debug.solid_leaves);
        assert!(!settings.debug.still_leaves);
    }

    #[test]
    fn test_settings_file_parses() {
        let file = std::fs::read_to_string("assets/settings.toml").unwrap();
        let settings: Settings = toml::from_str(&file).unwrap();
        assert_eq!(24, settings.renderer.full_detail_distance);
        assert_eq!(Some(16), settings.renderer.lod_base_distance);
        assert_eq!(2, settings.renderer.lod_level_scale);
        assert_eq!(64, settings.renderer.mesh_slab_mb);
        assert!(settings.renderer.mesh_slab_growth > 1.0);
        // full detail reaches further than the levels scale from, with level 1 still between it and level 2
        let layout = settings.renderer.lod_layout();
        assert_eq!(24.0, layout.reach(0));
        assert_eq!([32.0, 64.0], [layout.reach(1), layout.reach(2)]);
        // the debug switches ship off
        assert!(!settings.debug.still_leaves && !settings.debug.solid_leaves);
    }
}
