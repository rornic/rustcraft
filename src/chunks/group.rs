use bevy::{
    asset::{Assets, RenderAssetUsages},
    camera::{primitives::Aabb, visibility::Visibility},
    ecs::{
        component::Component,
        entity::Entity,
        system::{Commands, Query, Res, ResMut},
    },
    math::Vec3,
    pbr::MeshMaterial3d,
    prelude::Mesh3d,
    render::mesh::{Indices, Mesh, PrimitiveTopology, VertexAttributeValues},
    tasks::futures,
    tasks::{AsyncComputeTaskPool, Task},
    transform::components::Transform,
};

use super::{
    chunk::ChunkCoordinate,
    chunk_loader::{ChunkLoader, Unsettled},
    generate::generator::ChunkMeshes,
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
    for (offset, meshes) in parts {
        opaque.push((offset, meshes.opaque));
        water.extend(meshes.water.map(|mesh| (offset, mesh)));
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
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bevy::math::{I64Vec3, U16Vec3, Vec3};
    use bevy::render::mesh::{Indices, Mesh};

    use super::merge_chunk_meshes;
    use crate::block::BlockType;
    use crate::chunks::chunk::{ChunkCoordinate, ChunkData};
    use crate::chunks::generate::generator::{generate_chunk_mesh, ChunkMeshes};
    use crate::chunks::generate::noise::NoiseGenerator;

    fn block_mesh(at: U16Vec3) -> ChunkMeshes {
        let mut chunk = ChunkData::default();
        chunk.set_block_at(at, BlockType::Stone);
        generate_chunk_mesh(
            Arc::new(chunk),
            vec![None; 26],
            ChunkCoordinate(I64Vec3::ZERO),
            &NoiseGenerator::new(1),
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
