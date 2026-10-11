use std::error::Error;

use bevy::ecs::component::Component;
use serde::Deserialize;

use crate::chunks::lod::LodLayout;

#[derive(Default, Deserialize, Clone, Copy, Component)]
pub struct Settings {
    pub renderer: RendererSettings,
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
    }

    #[test]
    fn test_settings_file_parses() {
        let file = std::fs::read_to_string("assets/settings.toml").unwrap();
        let settings: Settings = toml::from_str(&file).unwrap();
        assert_eq!(16, settings.renderer.full_detail_distance);
        assert_eq!(4, settings.renderer.lod_level_scale);
    }
}
