use bevy::prelude::Resource;
use bevy::{
    asset::{Assets, Handle},
    camera::visibility::Visibility,
    ecs::hierarchy::Children,
    ecs::{
        component::Component,
        entity::Entity,
        system::{Commands, Query, Res},
    },
    math::Vec4,
    pbr::MeshMaterial3d,
    time::Time,
};

use super::material::{ChunkMaterial, TerrainMaterials, WaterMaterial};

// One step per level of the shaders' 8x8 dither pattern.
const FADE_STEPS: usize = 64;

/// The dither window that draws every pixel.
pub const FULLY_VISIBLE: Vec4 = Vec4::new(0.0, 1.0, 0.0, 0.0);

// A mesh fading out is drawn this fraction farther than it is, so the one replacing it wins where
// their surfaces coincide, as a LOD node's skirts and flat ground do with full detail.
const FADE_DEPTH_PUSH: f32 = 0.0005;

// A swap is staggered so one of the two meshes is always whole: the new one dithers in over the
// old for this share of the fade, then the old one dithers out from under it for the rest. So
// trees in the new mesh grow in over the old one's tint rather than popping up on top of it, and
// no pixel is ever given up by both.
const FADE_IN_SHARE: f32 = 0.5;

// The window drawn at `step` of a fade in: one more level of the dither is drawn each step.
fn fade_in_window(step: usize) -> Vec4 {
    Vec4::new(0.0, (step + 1) as f32 / FADE_STEPS as f32, 0.0, 0.0)
}

// The window drawn at `step` of a fade out, which hides `step` levels of the dither: whole at
// step 0, while what replaces it is still coming in, but pushed back already so that wins.
fn fade_out_window(step: usize) -> Vec4 {
    Vec4::new(step as f32 / FADE_STEPS as f32, 1.0, FADE_DEPTH_PUSH, 0.0)
}

// Which step of its fade, in or out, a mesh is at `progress` through it, or none once it's done.
fn fade_step(progress: f32, out: bool) -> Option<usize> {
    let progress = progress.max(0.0);
    let (start, end) = if out {
        (FADE_IN_SHARE, 1.0)
    } else {
        (0.0, FADE_IN_SHARE)
    };
    if progress >= end {
        return None;
    }
    if progress < start {
        // a fade out held whole while what replaces it comes in
        return Some(0);
    }
    let step = ((progress - start) / (end - start) * FADE_STEPS as f32) as usize;
    // once a fade out starts, one level is already gone, as one is already drawn as a fade in starts
    Some((step + out as usize).min(FADE_STEPS - 1))
}

#[derive(Resource)]
pub struct FadeMaterials {
    seconds: f32,
    chunk: Handle<ChunkMaterial>,
    water: Handle<WaterMaterial>,
    solid_water: Handle<WaterMaterial>,
    // a material for each step of a fade, indexed [out as usize][step]
    chunk_steps: [Vec<Handle<ChunkMaterial>>; 2],
    water_steps: [Vec<Handle<WaterMaterial>>; 2],
    solid_water_steps: [Vec<Handle<WaterMaterial>>; 2],
}

fn window(out: bool, step: usize) -> Vec4 {
    if out {
        fade_out_window(step)
    } else {
        fade_in_window(step)
    }
}

impl FadeMaterials {
    pub fn new(
        seconds: f32,
        materials: &TerrainMaterials,
        chunk_materials: &mut Assets<ChunkMaterial>,
        water_materials: &mut Assets<WaterMaterial>,
    ) -> Self {
        let TerrainMaterials {
            chunk,
            water,
            solid_water,
            ..
        } = materials.clone();
        let base_chunk = chunk_materials.get(&chunk).unwrap().clone();
        let base_water = water_materials.get(&water).unwrap().clone();
        let base_solid = water_materials.get(&solid_water).unwrap().clone();
        let chunk_steps = [false, true].map(|out| {
            (0..FADE_STEPS)
                .map(|step| {
                    chunk_materials.add(ChunkMaterial {
                        fade: window(out, step),
                        ..base_chunk.clone()
                    })
                })
                .collect()
        });
        let water_steps = [false, true].map(|out| {
            (0..FADE_STEPS)
                .map(|step| {
                    water_materials.add(WaterMaterial {
                        fade: window(out, step),
                        ..base_water.clone()
                    })
                })
                .collect()
        });
        let solid_water_steps = [false, true].map(|out| {
            (0..FADE_STEPS)
                .map(|step| {
                    water_materials.add(WaterMaterial {
                        fade: window(out, step),
                        ..base_solid.clone()
                    })
                })
                .collect()
        });
        Self {
            seconds,
            chunk,
            water,
            solid_water,
            chunk_steps,
            water_steps,
            solid_water_steps,
        }
    }
}

/// On a water mesh drawn with the solid material.
#[derive(Component)]
pub struct SolidWater;

#[derive(Component)]
pub struct Fade {
    start: f32,
    out: bool,
    despawn: bool,
}

/// Fades `entity` in or out unless it's already shown or hidden as asked. The two are staggered,
/// see `FADE_IN_SHARE`, so a mesh and the one replacing it, set going together, never both leave
/// a pixel undrawn.
pub fn set_shown(
    commands: &mut Commands,
    entity: Entity,
    visibility: &mut Visibility,
    fade: Option<&Fade>,
    shown: bool,
    now: f32,
) {
    // one fading out to be despawned is gone however the rules would show it, or it would linger unowned
    if fade.is_some_and(|f| f.despawn) {
        return;
    }
    let currently_shown = *visibility != Visibility::Hidden && !fade.is_some_and(|f| f.out);
    if currently_shown == shown {
        return;
    }
    if shown {
        // one fading out is brought back by finishing a fade in at once, which resets its material
        let start = if fade.is_some() { f32::MIN } else { now };
        *visibility = Visibility::Inherited;
        commands.entity(entity).insert(Fade {
            start,
            out: false,
            despawn: false,
        });
    } else {
        commands.entity(entity).insert(Fade {
            start: now,
            out: true,
            despawn: false,
        });
    }
}

pub fn fade_out_and_despawn(commands: &mut Commands, entity: Entity, visible: bool, now: f32) {
    if visible {
        commands.entity(entity).insert(Fade {
            start: now,
            out: true,
            despawn: true,
        });
    } else {
        commands.entity(entity).despawn();
    }
}

pub fn apply_fades(
    mut commands: Commands,
    time: Res<Time>,
    materials: Res<FadeMaterials>,
    mut fading: Query<(Entity, &Fade, &mut Visibility, Option<&Children>)>,
    mut chunk_materials: Query<&mut MeshMaterial3d<ChunkMaterial>>,
    mut water_materials: Query<(
        &mut MeshMaterial3d<WaterMaterial>,
        bevy::ecs::query::Has<SolidWater>,
    )>,
) {
    let now = time.elapsed_secs();

    for (entity, fade, mut visibility, children) in fading.iter_mut() {
        let progress = (now - fade.start) / materials.seconds.max(f32::EPSILON);
        let (chunk, water, solid_water) = match fade_step(progress, fade.out) {
            None => {
                if fade.out && fade.despawn {
                    commands.entity(entity).despawn();
                    continue;
                }
                if fade.out {
                    *visibility = Visibility::Hidden;
                }
                commands.entity(entity).remove::<Fade>();
                (&materials.chunk, &materials.water, &materials.solid_water)
            }
            Some(step) => {
                let out = fade.out as usize;
                (
                    &materials.chunk_steps[out][step],
                    &materials.water_steps[out][step],
                    &materials.solid_water_steps[out][step],
                )
            }
        };

        if let Ok(mut material) = chunk_materials.get_mut(entity) {
            if material.0 != *chunk {
                material.0 = chunk.clone();
            }
        }
        // water and the faces between leaves are drawn by child entities with their own materials
        for &child in children.into_iter().flatten() {
            if let Ok(mut material) = chunk_materials.get_mut(child) {
                if material.0 != *chunk {
                    material.0 = chunk.clone();
                }
            }
            if let Ok((mut material, solid)) = water_materials.get_mut(child) {
                let water = if solid { solid_water } else { water };
                if material.0 != *water {
                    material.0 = water.clone();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy::{
        camera::visibility::Visibility,
        ecs::{
            system::Commands,
            world::{CommandQueue, World},
        },
    };

    use super::{fade_in_window, fade_out_window, fade_step, set_shown, Fade, FADE_STEPS};

    // Every dither threshold the shader can produce, (level + 0.5) / 64.
    fn thresholds() -> impl Iterator<Item = f32> {
        (0..FADE_STEPS).map(|level| (level as f32 + 0.5) / FADE_STEPS as f32)
    }

    fn draws(window: bevy::math::Vec4, threshold: f32) -> bool {
        threshold >= window.x && threshold < window.y
    }

    #[test]
    fn test_fade_in_draws_one_more_level_per_step() {
        for step in 0..FADE_STEPS {
            let drawn = thresholds()
                .filter(|&t| draws(fade_in_window(step), t))
                .count();
            assert_eq!(step + 1, drawn);
        }
    }

    #[test]
    fn test_fade_out_hides_one_more_level_per_step() {
        for step in 0..FADE_STEPS {
            let hidden = thresholds()
                .filter(|&t| !draws(fade_out_window(step), t))
                .count();
            assert_eq!(step, hidden);
        }
    }

    #[test]
    fn test_one_of_a_swapped_pair_is_always_whole() {
        // a mesh and its replacement, set fading together, at every point through the fade
        for i in 0..=1000 {
            let progress = i as f32 / 1000.0;
            let coming = fade_step(progress, false).map_or(FADE_STEPS, |step| step + 1);
            let going = fade_step(progress, true).unwrap_or(FADE_STEPS);
            assert!(
                coming == FADE_STEPS || going == 0,
                "at {progress}: {coming} drawn and {going} hidden"
            );
        }
    }

    #[test]
    fn test_a_fade_in_finishes_as_the_fade_out_starts() {
        assert_eq!(None, fade_step(0.5, false));
        assert_eq!(Some(1), fade_step(0.5, true));
        assert_eq!(Some(0), fade_step(0.49, true));
        assert_eq!(None, fade_step(1.0, true));
    }

    // `set_shown` for an entity with this visibility and fade, then what's left of it.
    fn shown(
        visibility: Visibility,
        fade: Option<Fade>,
        shown: bool,
    ) -> (Visibility, Option<(f32, bool)>) {
        let mut world = World::new();
        let entity = match fade {
            Some(fade) => world.spawn((visibility, fade)).id(),
            None => world.spawn(visibility).id(),
        };
        let mut queue = CommandQueue::default();
        let mut v = visibility;
        let existing = world.get::<Fade>(entity).map(|f| Fade {
            start: f.start,
            out: f.out,
            despawn: f.despawn,
        });
        set_shown(
            &mut Commands::new(&mut queue, &world),
            entity,
            &mut v,
            existing.as_ref(),
            shown,
            5.0,
        );
        queue.apply(&mut world);
        (v, world.get::<Fade>(entity).map(|f| (f.start, f.out)))
    }

    #[test]
    fn test_showing_fades_in_from_now() {
        assert_eq!(
            (Visibility::Inherited, Some((5.0, false))),
            shown(Visibility::Hidden, None, true)
        );
    }

    #[test]
    fn test_hiding_fades_out_from_now() {
        let (_, fade) = shown(Visibility::Inherited, None, false);
        assert_eq!(Some((5.0, true)), fade);
    }

    #[test]
    fn test_bringing_back_one_fading_out_finishes_its_fade_at_once() {
        let fading = Fade {
            start: 3.0,
            out: true,
            despawn: false,
        };
        let (visibility, fade) = shown(Visibility::Inherited, Some(fading), true);
        assert_eq!(Visibility::Inherited, visibility);
        // a fade that's long over, which resets its material
        assert_eq!(Some((f32::MIN, false)), fade);
    }

    #[test]
    fn test_one_fading_out_to_despawn_is_never_brought_back() {
        let fading = Fade {
            start: 3.0,
            out: true,
            despawn: true,
        };
        let (_, fade) = shown(Visibility::Inherited, Some(fading), true);
        assert_eq!(Some((3.0, true)), fade);
    }
}
