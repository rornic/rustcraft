use std::error::Error;

use bevy::ecs::component::Component;
use serde::Deserialize;

use crate::chunks::lod::LodLayout;

#[derive(Default, Deserialize, Clone, Copy, Component)]
pub struct Settings {
    pub renderer: RendererSettings,
    #[serde(default)]
    pub debug: DebugSettings,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(default)]
pub struct RendererSettings {
    /// How far terrain is drawn, in chunks.
    pub render_distance: u32,
    /// How far terrain is drawn at full detail, in chunks.
    pub full_detail_distance: u32,
    /// Each LOD level reaches this many times as far as the one before it, starting
    /// from the full-detail distance.
    pub lod_level_scale: u32,
    /// How long swapping between LOD levels takes to cross-fade, in seconds.
    pub lod_fade_seconds: f32,
    /// How far the sun casts shadows, in chunks.
    pub shadow_distance: u32,
    /// Shadow map cascades, each drawing the terrain in its slice again.
    pub shadow_cascades: usize,
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
    pub fn lod_layout(&self) -> LodLayout {
        LodLayout::new(
            self.full_detail_distance,
            self.render_distance,
            self.lod_level_scale,
        )
    }
}

impl Default for RendererSettings {
    fn default() -> Self {
        Self {
            render_distance: 8,
            full_detail_distance: 32,
            lod_level_scale: 4,
            lod_fade_seconds: 1.5,
            shadow_distance: 64,
            shadow_cascades: 3,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Settings;

    #[test]
    fn test_missing_renderer_settings_use_defaults() {
        let settings: Settings = toml::from_str("[renderer]\nrender_distance = 64").unwrap();
        assert_eq!(64, settings.renderer.render_distance);
        assert_eq!(32, settings.renderer.full_detail_distance);
        assert!(!settings.debug.still_leaves && !settings.debug.solid_leaves);
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
        assert_eq!(16, settings.renderer.full_detail_distance);
        assert_eq!(2, settings.renderer.lod_level_scale);
        // the debug switches ship off
        assert!(!settings.debug.still_leaves && !settings.debug.solid_leaves);
    }
}
