use bevy::{
    asset::{Assets, Handle},
    ecs::{
        component::Component,
        entity::Entity,
        system::{Commands, Query, Res, Resource},
    },
    hierarchy::{Children, DespawnRecursiveExt},
    math::Vec4,
    pbr::MeshMaterial3d,
    render::view::Visibility,
    time::Time,
};

use super::material::{ChunkMaterial, WaterMaterial};

// One step per level of the shaders' 4x4 dither pattern.
const FADE_STEPS: usize = 16;

/// The dither window that draws every pixel.
pub const FULLY_VISIBLE: Vec4 = Vec4::new(0.0, 1.0, 0.0, 0.0);

fn fade_window(step: usize, out: bool) -> Vec4 {
    let shown = (step + 1) as f32 / FADE_STEPS as f32;
    if out {
        Vec4::new(shown, 1.0, 0.0, 0.0)
    } else {
        Vec4::new(0.0, shown, 0.0, 0.0)
    }
}

#[derive(Resource)]
pub struct FadeMaterials {
    seconds: f32,
    chunk: Handle<ChunkMaterial>,
    water: Handle<WaterMaterial>,
    // indexed [fading out as usize][step]
    chunk_steps: [Vec<Handle<ChunkMaterial>>; 2],
    water_steps: [Vec<Handle<WaterMaterial>>; 2],
}

impl FadeMaterials {
    pub fn new(
        seconds: f32,
        chunk: Handle<ChunkMaterial>,
        water: Handle<WaterMaterial>,
        chunk_materials: &mut Assets<ChunkMaterial>,
        water_materials: &mut Assets<WaterMaterial>,
    ) -> Self {
        let base_chunk = chunk_materials.get(&chunk).unwrap().clone();
        let base_water = water_materials.get(&water).unwrap().clone();
        let chunk_steps = [false, true].map(|out| {
            (0..FADE_STEPS)
                .map(|step| {
                    chunk_materials.add(ChunkMaterial {
                        fade: fade_window(step, out),
                        ..base_chunk.clone()
                    })
                })
                .collect()
        });
        let water_steps = [false, true].map(|out| {
            (0..FADE_STEPS)
                .map(|step| {
                    water_materials.add(WaterMaterial {
                        fade: fade_window(step, out),
                        ..base_water.clone()
                    })
                })
                .collect()
        });
        Self {
            seconds,
            chunk,
            water,
            chunk_steps,
            water_steps,
        }
    }
}

#[derive(Component)]
pub struct Fade {
    start: f32,
    out: bool,
    despawn: bool,
}

/// Fades `entity` in or out unless it's already shown or hidden as asked.
pub fn set_shown(
    commands: &mut Commands,
    entity: Entity,
    visibility: &mut Visibility,
    fade: Option<&Fade>,
    shown: bool,
    now: f32,
) {
    let currently_shown = *visibility != Visibility::Hidden && !fade.is_some_and(|f| f.out);
    if currently_shown == shown {
        return;
    }
    if shown {
        *visibility = Visibility::Inherited;
    }
    commands.entity(entity).insert(Fade {
        start: now,
        out: !shown,
        despawn: false,
    });
}

pub fn fade_out_and_despawn(commands: &mut Commands, entity: Entity, visible: bool, now: f32) {
    if visible {
        commands.entity(entity).insert(Fade {
            start: now,
            out: true,
            despawn: true,
        });
    } else {
        commands.entity(entity).despawn_recursive();
    }
}

pub fn apply_fades(
    mut commands: Commands,
    time: Res<Time>,
    materials: Res<FadeMaterials>,
    mut fading: Query<(Entity, &Fade, &mut Visibility, Option<&Children>)>,
    mut chunk_materials: Query<&mut MeshMaterial3d<ChunkMaterial>>,
    mut water_materials: Query<&mut MeshMaterial3d<WaterMaterial>>,
) {
    let now = time.elapsed_secs();

    for (entity, fade, mut visibility, children) in fading.iter_mut() {
        let progress = (now - fade.start) / materials.seconds.max(f32::EPSILON);
        let (chunk, water) = if progress >= 1.0 {
            if fade.out && fade.despawn {
                commands.entity(entity).despawn_recursive();
                continue;
            }
            if fade.out {
                *visibility = Visibility::Hidden;
            }
            commands.entity(entity).remove::<Fade>();
            (&materials.chunk, &materials.water)
        } else {
            let step = ((progress.max(0.0) * FADE_STEPS as f32) as usize).min(FADE_STEPS - 1);
            let direction = fade.out as usize;
            (
                &materials.chunk_steps[direction][step],
                &materials.water_steps[direction][step],
            )
        };

        if let Ok(mut material) = chunk_materials.get_mut(entity) {
            if material.0 != *chunk {
                material.0 = chunk.clone();
            }
        }
        // water is drawn by a child entity with its own material
        for &child in children.into_iter().flatten() {
            if let Ok(mut material) = water_materials.get_mut(child) {
                if material.0 != *water {
                    material.0 = water.clone();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{fade_window, FADE_STEPS};

    // Every dither threshold the shader can produce, (level + 0.5) / 16.
    fn thresholds() -> impl Iterator<Item = f32> {
        (0..16).map(|level| (level as f32 + 0.5) / 16.0)
    }

    fn draws(window: bevy::math::Vec4, threshold: f32) -> bool {
        threshold >= window.x && threshold < window.y
    }

    #[test]
    fn test_fade_in_and_out_draw_complementary_pixels() {
        for step in 0..FADE_STEPS {
            let (fade_in, fade_out) = (fade_window(step, false), fade_window(step, true));
            for t in thresholds() {
                assert_ne!(
                    draws(fade_in, t),
                    draws(fade_out, t),
                    "step {step} threshold {t}"
                );
            }
        }
    }

    #[test]
    fn test_fade_in_reveals_one_more_level_per_step() {
        for step in 0..FADE_STEPS {
            let drawn = thresholds()
                .filter(|&t| draws(fade_window(step, false), t))
                .count();
            assert_eq!(step + 1, drawn);
        }
    }
}
