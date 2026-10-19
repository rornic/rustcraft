use bevy::{
    asset::{Assets, RenderAssetUsages},
    camera::{primitives::Aabb, visibility::Visibility, Camera},
    ecs::{
        change_detection::DetectChangesMut,
        component::Component,
        entity::Entity,
        hierarchy::ChildOf,
        query::With,
        system::{Commands, Query, Res, ResMut},
    },
    math::Vec3,
    pbr::MeshMaterial3d,
    prelude::Mesh3d,
    render::mesh::{Indices, Mesh, PrimitiveTopology, VertexAttributeValues},
    tasks::futures,
    tasks::{AsyncComputeTaskPool, Task},
    transform::components::{GlobalTransform, Transform},
};

use super::{
    chunk::ChunkCoordinate,
    chunk_loader::{ChunkLoader, Unsettled},
    generate::generator::{ChunkMeshes, INNER_LEAVES_REACH},
    lod::LodKey,
};
use crate::chunks::chunk::CHUNK_SIZE;

/// A level-1 node's worth of full-detail chunks drawn as one mesh. Level 1 is where terrain
/// swaps between full detail and LODs, so a group is always wholly shown or wholly hidden.
#[derive(Component)]
pub struct ChunkGroup {
    pub key: LodKey,
}

#[derive(Component)]
pub struct BuildChunkGroup {
    task: Task<ChunkMeshes>,
    chunks: Vec<Entity>,
}

/// A group whose mesh is merged and ready, whether or not it has any opaque faces.
#[derive(Component)]
pub struct BuiltGroup;

/// A chunk meshed with something to draw, whose mesh waits in `ChunkLoader::held` for the
/// rest of its group.
#[derive(Component)]
pub struct MeshedChunk;

/// A chunk drawn as part of its group.
#[derive(Component)]
pub struct GroupedChunk;

/// Draws the faces between its group's leaves, see `ChunkMeshes::inner_leaves`. Hidden unless
/// the group is in `INNER_LEAVES_REACH` of the camera.
#[derive(Component)]
pub struct InnerLeafFaces;

fn group_aabb() -> Aabb {
    let width = (2 * CHUNK_SIZE) as f32;
    Aabb::from_min_max(Vec3::ZERO, Vec3::splat(width))
}

/// Starts merging the meshes of each group whose chunks have all settled, one way or the other.
pub fn build_groups(
    mut commands: Commands,
    mut loader: ResMut<ChunkLoader>,
    unsettled: Query<(), Unsettled>,
) {
    let task_pool = AsyncComputeTaskPool::get();
    let ready: Vec<LodKey> = loader
        .waiting
        .iter()
        .copied()
        .filter(|key| {
            key.chunks().all(|chunk| {
                loader
                    .chunk_entity(chunk)
                    .is_some_and(|entity| !unsettled.contains(entity))
            })
        })
        .collect();

    for key in ready {
        loader.waiting.remove(&key);
        let chunks: Vec<ChunkCoordinate> = key.chunks().collect();
        let entities: Vec<Entity> = chunks
            .iter()
            .filter_map(|&c| loader.chunk_entity(c))
            .collect();
        let origin = key.min_chunk();
        let parts: Vec<(Vec3, ChunkMeshes)> = chunks
            .iter()
            .filter_map(|c| {
                let offset = (c.0 - origin).as_vec3() * CHUNK_SIZE as f32;
                loader.held.remove(c).map(|meshes| (offset, meshes))
            })
            .collect();
        let min = key.min_block().as_vec3();
        let entity = commands
            .spawn((
                ChunkGroup { key },
                BuildChunkGroup {
                    task: task_pool.spawn(async move { merge_chunk_meshes(parts) }),
                    chunks: entities,
                },
                Transform::from_translation(min),
                // hidden until refresh_lod_visibility fades it in, so it never flashes up
                // over the LOD node it's replacing
                Visibility::Hidden,
            ))
            .id();
        loader.groups.insert(key, entity);
    }
}

/// Shows the faces between leaves of the groups near the camera, and hides the rest, whose leaves
/// have their holes filled. A group is near if any of it is, so it's never missing faces that
/// would show through a hole.
pub fn show_inner_leaves(
    camera: Query<&GlobalTransform, (With<Camera>, With<ChildOf>)>,
    groups: Query<&ChunkGroup>,
    mut inner_leaves: Query<(&ChildOf, &mut Visibility), With<InnerLeafFaces>>,
) {
    let Ok(camera) = camera.single() else {
        return;
    };
    let eye = camera.translation();
    for (parent, mut visibility) in &mut inner_leaves {
        let Ok(group) = groups.get(parent.parent()) else {
            continue;
        };
        let near = inner_leaves_near(group.key, eye);
        visibility.set_if_neq(if near {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
    }
}

// Whether any of the group at `key` is within `INNER_LEAVES_REACH` of `eye`.
fn inner_leaves_near(key: LodKey, eye: Vec3) -> bool {
    let min = key.min_block().as_vec3();
    let max = min + Vec3::splat((2 * CHUNK_SIZE) as f32);
    eye.clamp(min, max).distance(eye) < INNER_LEAVES_REACH
}

/// Draws groups whose merged meshes are ready.
pub fn load_groups(
    mut commands: Commands,
    loader: Res<ChunkLoader>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut building: Query<(Entity, &mut BuildChunkGroup)>,
) {
    for (entity, mut build) in building.iter_mut() {
        let Some(merged) = futures::check_ready(&mut build.task) else {
            continue;
        };
        if merged.has_geometry() {
            commands.entity(entity).insert((BuiltGroup, group_aabb()));
            if merged.opaque.count_vertices() > 0 {
                commands.entity(entity).insert((
                    Mesh3d(meshes.add(merged.opaque)),
                    MeshMaterial3d(loader.material.clone()),
                ));
            }
            if let Some(water) = merged.water {
                let water_material = loader.water_material.clone();
                commands.entity(entity).with_children(|parent| {
                    parent.spawn((
                        Mesh3d(meshes.add(water)),
                        MeshMaterial3d(water_material),
                        Transform::default(),
                        group_aabb(),
                    ));
                });
            }
            if let Some(inner_leaves) = merged.inner_leaves {
                let material = loader.material.clone();
                commands.entity(entity).with_children(|parent| {
                    parent.spawn((
                        InnerLeafFaces,
                        Mesh3d(meshes.add(inner_leaves)),
                        MeshMaterial3d(material),
                        Transform::default(),
                        group_aabb(),
                        // shown by `show_inner_leaves` once it's near
                        Visibility::Hidden,
                    ));
                });
            }
        }
        for &chunk in &build.chunks {
            if let Ok(mut chunk) = commands.get_entity(chunk) {
                chunk.remove::<MeshedChunk>().try_insert(GroupedChunk);
            }
        }
        commands.entity(entity).remove::<BuildChunkGroup>();
    }
}

fn float3(mesh: &Mesh, attribute: bevy::render::mesh::MeshVertexAttribute) -> &[[f32; 3]] {
    mesh.attribute(attribute)
        .and_then(VertexAttributeValues::as_float3)
        .expect("float3 attribute")
}

fn float2(mesh: &Mesh, attribute: bevy::render::mesh::MeshVertexAttribute) -> Option<&[[f32; 2]]> {
    match mesh.attribute(attribute)? {
        VertexAttributeValues::Float32x2(values) => Some(values),
        _ => None,
    }
}

fn float4(mesh: &Mesh, attribute: bevy::render::mesh::MeshVertexAttribute) -> &[[f32; 4]] {
    match mesh.attribute(attribute) {
        Some(VertexAttributeValues::Float32x4(values)) => values,
        _ => panic!("float4 attribute"),
    }
}

// One mesh from `parts`, each moved by its offset. Parts with no vertices are skipped.
fn merge_meshes(parts: Vec<(Vec3, Mesh)>) -> Option<Mesh> {
    let parts: Vec<(Vec3, Mesh)> = parts
        .into_iter()
        .filter(|(_, mesh)| mesh.count_vertices() > 0)
        .collect();
    if parts.is_empty() {
        return None;
    }
    let vertices: usize = parts.iter().map(|(_, mesh)| mesh.count_vertices()).sum();
    let mut positions = Vec::with_capacity(vertices);
    let mut normals = Vec::with_capacity(vertices);
    let mut uvs = Vec::with_capacity(vertices);
    let mut colors = Vec::with_capacity(vertices);
    let mut lights = Vec::with_capacity(vertices);
    let mut indices = Vec::new();
    for (offset, mesh) in &parts {
        let start = positions.len() as u32;
        positions.extend(
            float3(mesh, Mesh::ATTRIBUTE_POSITION)
                .iter()
                .map(|&p| (Vec3::from(p) + *offset).to_array()),
        );
        normals.extend_from_slice(float3(mesh, Mesh::ATTRIBUTE_NORMAL));
        uvs.extend_from_slice(float2(mesh, Mesh::ATTRIBUTE_UV_0).expect("uvs"));
        colors.extend_from_slice(float4(mesh, Mesh::ATTRIBUTE_COLOR));
        // block faces carry light, water doesn't, and a mesh has either on all its vertices or none
        if let Some(light) = float2(mesh, Mesh::ATTRIBUTE_UV_1) {
            lights.extend_from_slice(light);
        }
        let Some(Indices::U32(part)) = mesh.indices() else {
            panic!("chunk meshes have u32 indices");
        };
        indices.extend(part.iter().map(|i| i + start));
    }
    let mut merged = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
    .with_inserted_indices(Indices::U32(indices));
    if !lights.is_empty() {
        assert_eq!(vertices, lights.len(), "only some vertices carry light");
        merged.insert_attribute(Mesh::ATTRIBUTE_UV_1, lights);
    }
    Some(merged)
}

/// The chunks' meshes as one, each moved by its offset in blocks.
pub fn merge_chunk_meshes(parts: Vec<(Vec3, ChunkMeshes)>) -> ChunkMeshes {
    let mut opaque = Vec::new();
    let mut water = Vec::new();
    let mut inner_leaves = Vec::new();
    for (offset, meshes) in parts {
        opaque.push((offset, meshes.opaque));
        water.extend(meshes.water.map(|mesh| (offset, mesh)));
        inner_leaves.extend(meshes.inner_leaves.map(|mesh| (offset, mesh)));
    }
    ChunkMeshes {
        // an empty mesh, so there's always something to hand back
        opaque: merge_meshes(opaque).unwrap_or_else(|| {
            Mesh::new(
                PrimitiveTopology::TriangleList,
                RenderAssetUsages::RENDER_WORLD,
            )
        }),
        water: merge_meshes(water),
        inner_leaves: merge_meshes(inner_leaves),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bevy::math::{I64Vec3, U16Vec3, Vec3};
    use bevy::render::mesh::{Indices, Mesh};

    use super::{inner_leaves_near, merge_chunk_meshes};
    use crate::block::BlockType;
    use crate::chunks::chunk::{ChunkCoordinate, ChunkData};
    use crate::chunks::generate::generator::INNER_LEAVES_REACH;
    use crate::chunks::generate::generator::{generate_chunk_mesh, ChunkMeshes};
    use crate::chunks::lod::LodKey;

    fn block_mesh(at: U16Vec3) -> ChunkMeshes {
        let mut chunk = ChunkData::default();
        chunk.set_block_at(at, BlockType::Stone);
        generate_chunk_mesh(
            Arc::new(chunk),
            vec![None; 26],
            ChunkCoordinate(I64Vec3::ZERO),
            &*crate::worldmap::test_terrain(1),
            false,
        )
    }

    fn positions(mesh: &Mesh) -> Vec<Vec3> {
        mesh.attribute(Mesh::ATTRIBUTE_POSITION)
            .unwrap()
            .as_float3()
            .unwrap()
            .iter()
            .map(|&p| p.into())
            .collect()
    }

    #[test]
    fn test_merged_meshes_keep_every_vertex_where_it_was() {
        let (a, b) = (
            block_mesh(U16Vec3::new(4, 4, 4)),
            block_mesh(U16Vec3::new(4, 4, 4)),
        );
        let alone = positions(&a.opaque);
        let offset = Vec3::new(16.0, 0.0, 32.0);
        let merged = merge_chunk_meshes(vec![(Vec3::ZERO, a), (offset, b)]);
        let merged_positions = positions(&merged.opaque);
        assert_eq!(2 * alone.len(), merged_positions.len());
        assert_eq!(alone, merged_positions[..alone.len()]);
        let moved: Vec<Vec3> = alone.iter().map(|p| *p + offset).collect();
        assert_eq!(moved, merged_positions[alone.len()..]);
    }

    #[test]
    fn test_merged_indices_point_into_their_own_part() {
        let (a, b) = (
            block_mesh(U16Vec3::new(1, 1, 1)),
            block_mesh(U16Vec3::new(2, 2, 2)),
        );
        let (a_vertices, a_indices) =
            (a.opaque.count_vertices(), a.opaque.indices().unwrap().len());
        let merged = merge_chunk_meshes(vec![(Vec3::ZERO, a), (Vec3::new(16.0, 0.0, 0.0), b)]);
        let Some(Indices::U32(indices)) = merged.opaque.indices() else {
            panic!()
        };
        assert!(indices[..a_indices]
            .iter()
            .all(|&i| (i as usize) < a_vertices));
        assert!(indices[a_indices..]
            .iter()
            .all(|&i| (i as usize) >= a_vertices));
        assert!(indices
            .iter()
            .all(|&i| (i as usize) < merged.opaque.count_vertices()));
    }

    #[test]
    fn test_faces_between_leaves_merge_apart() {
        let mut chunk = ChunkData::default();
        chunk.set_block_at(U16Vec3::new(4, 4, 4), BlockType::Leaves);
        chunk.set_block_at(U16Vec3::new(5, 4, 4), BlockType::Leaves);
        let meshes = generate_chunk_mesh(
            Arc::new(chunk),
            vec![None; 26],
            ChunkCoordinate(I64Vec3::ZERO),
            &*crate::worldmap::test_terrain(1),
            false,
        );
        let inner = meshes.inner_leaves.as_ref().unwrap().count_vertices();
        let merged = merge_chunk_meshes(vec![
            (Vec3::ZERO, meshes),
            (Vec3::ZERO, block_mesh(U16Vec3::ONE)),
        ]);
        assert_eq!(inner, merged.inner_leaves.unwrap().count_vertices());
    }

    #[test]
    fn test_inner_leaves_are_near_if_any_of_their_group_is() {
        let key = LodKey::containing(ChunkCoordinate(I64Vec3::ZERO), 1);
        // inside the group, and just off its far corner
        assert!(inner_leaves_near(key, Vec3::splat(10.0)));
        assert!(inner_leaves_near(
            key,
            Vec3::new(32.0 + INNER_LEAVES_REACH - 1.0, 5.0, 5.0)
        ));
        assert!(!inner_leaves_near(
            key,
            Vec3::new(32.0 + INNER_LEAVES_REACH + 1.0, 5.0, 5.0)
        ));
        assert!(!inner_leaves_near(
            key,
            Vec3::new(-INNER_LEAVES_REACH - 1.0, 5.0, 5.0)
        ));
    }

    #[test]
    fn test_chunks_with_nothing_to_draw_merge_to_nothing() {
        let merged = merge_chunk_meshes(Vec::new());
        assert!(!merged.has_geometry());
    }

    #[test]
    fn test_empty_parts_are_skipped() {
        let empty = merge_chunk_meshes(Vec::new());
        let block = block_mesh(U16Vec3::new(3, 3, 3));
        let vertices = block.opaque.count_vertices();
        let merged = merge_chunk_meshes(vec![(Vec3::ZERO, empty), (Vec3::ZERO, block)]);
        assert_eq!(vertices, merged.opaque.count_vertices());
    }
}
