use std::fmt::Write;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use bevy::core_pipeline::core_3d::{AlphaMask3d, Opaque3d, Transparent3d};
use bevy::core_pipeline::prepass::{AlphaMask3dPrepass, Opaque3dPrepass};
use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::pbr::Shadow;
use bevy::prelude::*;
use bevy::render::render_phase::{
    BinnedPhaseItem, BinnedRenderPhase, ViewBinnedRenderPhases, ViewSortedRenderPhases,
};
use bevy::render::{Render, RenderApp, RenderSystems};

use crate::player::Player;
use ochre_terrain::chunk_loader::Chunk;
use ochre_terrain::lod::LodNode;
use ochre_terrain::world::World;

const TOGGLE_KEY: KeyCode = KeyCode::F1;
const REFRESH_SECONDS: f32 = 0.25;

/// F1 toggles a text readout of the player's climate, frame cost and loaded terrain.
pub struct DebugOverlayPlugin;

impl Plugin for DebugOverlayPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<FrameTimeDiagnosticsPlugin>() {
            app.add_plugins(FrameTimeDiagnosticsPlugin::default());
        }
        let counts = DrawCounts::default();
        app.insert_resource(counts.clone())
            .add_systems(Startup, spawn_overlay)
            .add_systems(Update, (toggle_overlay, update_overlay).chain());
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app
                .insert_resource(counts)
                .add_systems(Render, count_draws.in_set(RenderSystems::Prepare));
        }
    }
}

/// Draws the render world queued last frame, which Bevy only issues one per mesh here.
#[derive(Resource, Clone, Default)]
struct DrawCounts {
    main: Arc<AtomicU32>,
    prepass: Arc<AtomicU32>,
    blended: Arc<AtomicU32>,
    shadow: Arc<AtomicU32>,
}

// a multi-draw batch set is one draw call however many meshes it holds
fn binned_draws<T: BinnedPhaseItem>(phases: &ViewBinnedRenderPhases<T>) -> u32 {
    let draws = |phase: &BinnedRenderPhase<T>| {
        phase.multidrawable_meshes.len()
            + phase.batchable_meshes.len()
            + phase
                .unbatchable_meshes
                .values()
                .map(|v| v.entities.len())
                .sum::<usize>()
    };
    phases.values().map(draws).sum::<usize>() as u32
}

#[allow(clippy::too_many_arguments)]
fn count_draws(
    counts: Res<DrawCounts>,
    opaque: Res<ViewBinnedRenderPhases<Opaque3d>>,
    masked: Res<ViewBinnedRenderPhases<AlphaMask3d>>,
    opaque_prepass: Res<ViewBinnedRenderPhases<Opaque3dPrepass>>,
    masked_prepass: Res<ViewBinnedRenderPhases<AlphaMask3dPrepass>>,
    shadow: Res<ViewBinnedRenderPhases<Shadow>>,
    blended: Res<ViewSortedRenderPhases<Transparent3d>>,
) {
    counts.main.store(
        binned_draws(&opaque) + binned_draws(&masked),
        Ordering::Relaxed,
    );
    counts.prepass.store(
        binned_draws(&opaque_prepass) + binned_draws(&masked_prepass),
        Ordering::Relaxed,
    );
    counts
        .shadow
        .store(binned_draws(&shadow), Ordering::Relaxed);
    counts.blended.store(
        blended
            .values()
            .map(|phase| phase.items.len())
            .sum::<usize>() as u32,
        Ordering::Relaxed,
    );
}

#[derive(Component)]
struct DebugOverlay;

fn spawn_overlay(mut commands: Commands) {
    commands.spawn((
        DebugOverlay,
        Text::new(""),
        TextFont {
            font_size: bevy::text::FontSize::Px(16.0),
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
        if std::env::var("OCHRE_STATS").is_ok() {
            Visibility::Visible
        } else {
            Visibility::Hidden
        },
    ));
}

fn toggle_overlay(
    keys: Res<ButtonInput<KeyCode>>,
    mut overlay: Query<&mut Visibility, With<DebugOverlay>>,
) {
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
    counts: Res<DrawCounts>,
    mut overlay: Query<(&mut Text, &Visibility), With<DebugOverlay>>,
) {
    let Ok((mut text, visibility)) = overlay.single_mut() else {
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

    if let (Some(world), Ok(player)) = (&world, player.single()) {
        let pos = player.translation();
        let noise = world.region_store.terrain();
        let (x, z) = (pos.x as f64, pos.z as f64);
        let biome = noise
            .biomes()
            .name(noise.sample(x, z).biome)
            .unwrap_or("base");
        let climate = noise.climate(x, z);
        let _ = writeln!(out, "pos: {:.0} {:.0} {:.0}", pos.x, pos.y, pos.z);
        let _ = writeln!(
            out,
            "biome: {biome}  temp: {:.2}  humidity: {:.2}",
            climate.temperature, climate.humidity
        );
    }

    let load = |count: &AtomicU32| count.load(Ordering::Relaxed);
    let (main, prepass, blended, shadow) = (
        load(&counts.main),
        load(&counts.prepass),
        load(&counts.blended),
        load(&counts.shadow),
    );
    let _ = writeln!(
        out,
        "draw calls: {} (main {main}, depth {prepass}, water {blended}, shadows {shadow})",
        main + prepass + blended + shadow
    );

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

    if std::env::var("OCHRE_STATS").is_ok() {
        println!("STATS {}", out.replace('\n', " | "));
    }
    text.0 = out;
}
