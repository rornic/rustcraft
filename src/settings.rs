use bevy::ecs::component::Component;
use serde::Deserialize;

#[derive(Default, Deserialize, Clone, Copy, Component)]
pub struct Settings {
    pub renderer: RendererSettings,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(default)]
pub struct RendererSettings {
    /// How far terrain is drawn, in chunks. Rounded up to the edge of the next LOD level.
    pub render_distance: u32,
    /// How far terrain is drawn at full detail, in chunks. Each LOD reaches twice as far.
    pub full_detail_distance: u32,
    /// The first LOD level drawn as a smooth surface instead of blocks.
    pub smooth_lod_level: u8,
    /// How long swapping between LOD levels takes to cross-fade, in seconds.
    pub lod_fade_seconds: f32,
}

impl Default for RendererSettings {
    fn default() -> Self {
        Self {
            render_distance: 8,
            full_detail_distance: 32,
            smooth_lod_level: 2,
            lod_fade_seconds: 0.6,
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
        assert_eq!(2, settings.renderer.smooth_lod_level);
    }

    #[test]
    fn test_settings_file_parses() {
        let file = std::fs::read_to_string("assets/settings.toml").unwrap();
        let settings: Settings = toml::from_str(&file).unwrap();
        assert_eq!(32, settings.renderer.full_detail_distance);
    }
}
