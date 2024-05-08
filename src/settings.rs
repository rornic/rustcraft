use bevy::ecs::component::Component;
use serde::Deserialize;

#[derive(Default, Deserialize, Clone, Copy, Component)]
pub struct Settings {
    pub renderer: RendererSettings,
}

#[derive(Deserialize, Clone, Copy)]
pub struct RendererSettings {
    pub render_distance: u32,
}

impl Default for RendererSettings {
    fn default() -> Self {
        Self { render_distance: 8 }
    }
}
