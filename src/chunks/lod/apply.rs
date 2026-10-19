use super::key::LodKey;
use super::loader::{lod_aabb, GenerateLodMesh, LodLoader, LodNode};
use super::state::{lod_state, ChunkState, ChunkStates, LodState};
use crate::chunks::chunk::ChunkCoordinate;
use crate::chunks::chunk_loader::ChunkLoader;
use crate::chunks::fade::fade_out_and_despawn;
use crate::chunks::fade::set_shown;
use crate::chunks::fade::Fade;
use crate::chunks::fade::SolidWater;
use crate::chunks::group::BuiltGroup;
use crate::chunks::group::ChunkGroup;
use crate::chunks::pack::pack;
use bevy::asset::Assets;
use bevy::camera::visibility::Visibility;
use bevy::ecs::entity::Entity;
use bevy::ecs::query::Added;
use bevy::ecs::query::With;
use bevy::ecs::query::Without;
use bevy::ecs::system::Commands;
use bevy::ecs::system::Query;
use bevy::ecs::system::Res;
use bevy::ecs::system::ResMut;
use bevy::pbr::MeshMaterial3d;
use bevy::prelude::Mesh3d;
use bevy::render::mesh::Mesh;
use bevy::tasks::futures;
use bevy::time::Time;
use bevy::transform::components::Transform;
use std::time::Duration;
use std::time::Instant;

pub(super) const LOD_APPLY_TIME_BUDGET: Duration = Duration::from_millis(2);

// From this level up, water draws with the opaque meshes. Blended meshes are drawn one at a time,
// where Bevy batches opaque ones into a few draws, and water this far out is nearly opaque anyway.
pub const SOLID_WATER_LEVEL: u8 = 2;

pub fn load_lods(
    mut commands: Commands,
    mut lods: ResMut<LodLoader>,
    mut nodes_query: Query<(Entity, &LodNode, &mut GenerateLodMesh)>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let start = Instant::now();

    for (entity, node, mut generate) in nodes_query.iter_mut() {
        let Some(result) = futures::check_ready(&mut generate.task) else {
            continue;
        };

        let has_mesh = result.is_some();
        if let Some(chunk_meshes) = result {
            commands.entity(entity).insert(lod_aabb());
            // open sea is all water, with no opaque faces to upload
            if chunk_meshes.opaque.count_vertices() > 0 {
                commands.entity(entity).insert((
                    Mesh3d(meshes.add(pack(chunk_meshes.opaque))),
                    MeshMaterial3d(lods.material.clone()),
                ));
            }
            if let Some(water_mesh) = chunk_meshes.water {
                let solid = node.key.level >= SOLID_WATER_LEVEL;
                let water_material = if solid {
                    &lods.solid_water_material
                } else {
                    &lods.water_material
                }
                .clone();
                commands.entity(entity).with_children(|parent| {
                    let mut water = parent.spawn((
                        Mesh3d(meshes.add(pack(water_mesh))),
                        MeshMaterial3d(water_material),
                        Transform::default(),
                        lod_aabb(),
                    ));
                    if solid {
                        water.insert(SolidWater);
                    }
                });
            }
        }
        if has_mesh {
            commands.entity(entity).remove::<GenerateLodMesh>();
        } else {
            // meshed to nothing, so it needs no entity
            commands.entity(entity).despawn();
            lods.add_node(node.key, None);
        }
        lods.dirty_areas.push(node.key);

        if start.elapsed() >= LOD_APPLY_TIME_BUDGET {
            break;
        }
    }
}

pub fn retire_lods(
    mut commands: Commands,
    mut lods: ResMut<LodLoader>,
    time: Res<Time>,
    chunks: ChunkStates,
    pending_lods: Query<(), With<GenerateLodMesh>>,
    visibility: Query<&Visibility>,
) {
    let Some(anchor) = lods.anchor else {
        return;
    };

    let retired: Vec<LodKey> = {
        lod_state!(
            state,
            lods,
            anchor,
            &lods.selected,
            &pending_lods,
            |chunk| chunks.get(chunk)
        );
        lods.retiring
            .iter()
            .copied()
            .filter(|key| !lods.selected.contains(key) && state.can_retire(*key))
            .collect()
    };

    if retired.is_empty() {
        return;
    }
    let lods = &mut *lods;
    lods.retiring
        .retain(|key| !retired.contains(key) && !lods.selected.contains(key));
    for key in retired {
        if let Some(Some(entity)) = lods.remove_node(key) {
            let visible = visibility
                .get(entity)
                .is_ok_and(|v| *v != Visibility::Hidden);
            fade_out_and_despawn(&mut commands, entity, visible, time.elapsed_secs());
        }
        lods.dirty_areas.push(key);
    }
}

#[allow(clippy::too_many_arguments)]
pub fn refresh_lod_visibility(
    mut commands: Commands,
    mut lods: ResMut<LodLoader>,
    time: Res<Time>,
    chunk_loader: Res<ChunkLoader>,
    pending_lods: Query<(), With<GenerateLodMesh>>,
    all_groups: Query<(Entity, &ChunkGroup), With<BuiltGroup>>,
    new_groups: Query<(Entity, &ChunkGroup), Added<BuiltGroup>>,
    built_groups: Query<(), With<BuiltGroup>>,
    // only nodes with something to draw, as those meshed to nothing are despawned - nodes still
    // loading are shown when their mesh lands
    all_nodes: Query<(Entity, &LodNode), Without<GenerateLodMesh>>,
    mut visibility: Query<(&mut Visibility, Option<&Fade>)>,
) {
    let Some(anchor) = lods.anchor else {
        return;
    };

    let now = time.elapsed_secs();
    let lods = &mut *lods;
    // the visibility rules don't look at chunks' loading state
    lod_state!(state, lods, anchor, &lods.selected, &pending_lods, |_| {
        ChunkState::Missing
    });
    let mut set = |entity: Entity, visible: bool| {
        if let Ok((mut v, fade)) = visibility.get_mut(entity) {
            set_shown(&mut commands, entity, &mut v, fade, visible, now);
        }
    };
    // a node fades in once its mesh is ready, not when it's first spawned
    let node_shown = |key: LodKey| state.node_visible(key) && (state.node_ready)(key);

    if lods.refresh_all {
        for (entity, group) in all_groups.iter() {
            set(
                entity,
                state.chunk_visible(ChunkCoordinate(group.key.min_chunk())),
            );
        }
        for (entity, node) in all_nodes.iter() {
            set(entity, node_shown(node.key));
        }
    } else {
        for &area in &lods.dirty_areas {
            // a group is drawn whole, so one chunk of it says whether it shows
            let in_area = |group: LodKey| {
                chunk_loader
                    .group_entity(group)
                    .filter(|&e| built_groups.contains(e))
                    .map(|e| (group, e))
            };
            let width = 1usize << (3 * area.level.saturating_sub(1) as usize).min(40);
            let groups: Vec<(LodKey, Entity)> = if width <= chunk_loader.groups.len() {
                LodKey::groups_in(area).filter_map(in_area).collect()
            } else {
                chunk_loader
                    .groups
                    .keys()
                    .filter(|&&group| area.contains_group(group))
                    .filter_map(|&group| in_area(group))
                    .collect()
            };
            for (group, entity) in groups {
                set(
                    entity,
                    state.chunk_visible(ChunkCoordinate(group.min_chunk())),
                );
            }
            for (key, entity) in lods.nodes_under(area) {
                set(entity, node_shown(key));
            }
        }
    }

    // newly built groups arrive hidden, and fade in unless a node still covers them
    for (entity, group) in new_groups.iter() {
        set(
            entity,
            state.chunk_visible(ChunkCoordinate(group.key.min_chunk())),
        );
    }

    lods.refresh_all = false;
    lods.dirty_areas.clear();
}
