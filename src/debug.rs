use std::fmt::Write;

use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;

use crate::chunks::chunk_loader::Chunk;
use crate::chunks::lod::LodNode;
use crate::player::Player;
use crate::world::World;

const TOGGLE_KEY: KeyCode = KeyCode::F1;
const REFRESH_SECONDS: f32 = 0.25;

/// F1 toggles a text readout of the player's climate, frame cost and loaded terrain.
pub struct DebugOverlayPlugin;

impl Plugin for DebugOverlayPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<FrameTimeDiagnosticsPlugin>() {
            app.add_plugins(FrameTimeDiagnosticsPlugin::default());
        }
        app.add_systems(Startup, spawn_overlay)
            .add_systems(Update, (toggle_overlay, update_overlay).chain());
    }
}

#[derive(Component)]
struct DebugOverlay;

fn spawn_overlay(mut commands: Commands) {
    commands.spawn((
        DebugOverlay,
        Text::new(""),
        TextFont {
            font_size: 16.0,
            ..default()
        },
        TextColor(Color::WHITE),
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.5)),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(6.0),
            left: Val::Px(6.0),
            padding: UiRect::all(Val::Px(6.0)),
            ..default()
        },
        Visibility::Hidden,
    ));
}

fn toggle_overlay(keys: Res<ButtonInput<KeyCode>>, mut overlay: Query<&mut Visibility, With<DebugOverlay>>) {
    if !keys.just_pressed(TOGGLE_KEY) {
        return;
    }
    for mut visibility in &mut overlay {
        visibility.toggle_inherited_hidden();
    }
}

#[allow(clippy::too_many_arguments)]
fn update_overlay(
    time: Res<Time>,
    mut since_refresh: Local<f32>,
    diagnostics: Res<DiagnosticsStore>,
    world: Option<Res<World>>,
    player: Query<&GlobalTransform, With<Player>>,
    chunks: Query<(), With<Chunk>>,
    lod_nodes: Query<&LodNode>,
    drawn: Query<(&Mesh3d, &ViewVisibility)>,
    meshes: Res<Assets<Mesh>>,
    mut overlay: Query<(&mut Text, &Visibility), With<DebugOverlay>>,
) {
    let Ok((mut text, visibility)) = overlay.get_single_mut() else {
        return;
    };
    *since_refresh += time.delta_secs();
    if *visibility == Visibility::Hidden || *since_refresh < REFRESH_SECONDS {
        return;
    }
    *since_refresh = 0.0;

    let mut out = String::new();

    let fps = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FPS)
        .and_then(|d| d.smoothed())
        .unwrap_or(0.0);
    let _ = writeln!(out, "fps: {fps:.0}");

    if let (Some(world), Ok(player)) = (&world, player.get_single()) {
        let pos = player.translation();
        let noise = world.region_store.noise();
        let (x, z) = (pos.x as f64, pos.z as f64);
        let biome = noise.biomes().name(noise.sample(x, z).biome).unwrap_or("base");
        let climate = noise.climate(x, z);
        let _ = writeln!(out, "pos: {:.0} {:.0} {:.0}", pos.x, pos.y, pos.z);
        let _ = writeln!(
            out,
            "biome: {biome}  temp: {:.2}  humidity: {:.2}",
            climate.temperature, climate.humidity
        );
    }

    let (mut draw_calls, mut vertices) = (0usize, 0usize);
    for (mesh, visible) in &drawn {
        if visible.get() {
            draw_calls += 1;
            vertices += meshes.get(&mesh.0).map_or(0, Mesh::count_vertices);
        }
    }
    let _ = writeln!(out, "draw calls: ~{draw_calls} (visible meshes, excl. shadows)");
    let _ = writeln!(out, "vertices: {vertices}");

    let mut per_level = vec![chunks.iter().count()];
    for node in &lod_nodes {
        let level = node.key().level as usize;
        if per_level.len() <= level {
            per_level.resize(level + 1, 0);
        }
        per_level[level] += 1;
    }
    let _ = write!(out, "chunks:");
    for (level, count) in per_level.iter().enumerate() {
        let _ = write!(out, " L{level}={count}");
    }

    text.0 = out;
}
