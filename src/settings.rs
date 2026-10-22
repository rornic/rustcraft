use std::error::Error;

use serde::Deserialize;

pub use ochre_terrain::settings::{DebugSettings, RendererSettings};
use ochre_world::worldmap::WorldConfig;

/// Everything `settings.toml` holds, each part read by the crate it configures.
#[derive(Default, Deserialize, Clone, Copy)]
pub struct Settings {
    pub renderer: RendererSettings,
    #[serde(default)]
    pub debug: DebugSettings,
    #[serde(default)]
    pub world: WorldConfig,
}

impl Settings {
    pub fn read(file: &str) -> Result<Self, Box<dyn Error>> {
        Ok(toml::from_str(&std::fs::read_to_string(file)?)?)
    }
}

#[cfg(test)]
mod tests {
    use super::Settings;
    use ochre_world::worldmap::WorldConfig;

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
