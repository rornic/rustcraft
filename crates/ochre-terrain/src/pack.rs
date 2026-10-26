//! Packs a terrain mesh's vertices into fewer bytes before it goes to the GPU, so the same view
//! takes about half the memory.
//!
//! Positions become 16-bit fractions of the mesh's own bounds, normals 4-byte octahedral, both
//! sets of uvs 16-bit, colours half-floats and indices 16-bit where the mesh is small enough.
//! Bevy decodes all of them in its vertex shader, which every terrain shader already calls (see
//! `decompress_vertex`), apart from the colours, whose half-floats the GPU widens by itself.

use std::sync::atomic::{AtomicBool, Ordering};

use bevy::mesh::VertexAttributeValues;
use bevy::mesh::{AttributeQuantization, MeshAttributeCompressionFlags, MeshCompressionArgs};
use bevy::prelude::Mesh;

static PACKED: AtomicBool = AtomicBool::new(true);

/// Whether meshes are packed, which they are unless a setting turns it off, for comparing the two.
pub fn set_packed(on: bool) {
    PACKED.store(on, Ordering::Relaxed);
}

// Positions are packed as fractions of the mesh's bounds, which are divided by the bounds' size, so
// a mesh that is flat, as the top of a level stretch of ground is, has a size of nothing to divide
// by. Bevy guards against that, but a vertex that no triangle uses, a block off the mesh along each
// flat axis, gives the bounds some size at no cost and draws nothing. Returns false for a mesh whose
// attributes aren't the floats it expects.
fn give_bounds_size(mesh: &mut Mesh) -> bool {
    let Some(VertexAttributeValues::Float32x3(positions)) =
        mesh.attribute(Mesh::ATTRIBUTE_POSITION)
    else {
        return true;
    };
    let Some(first) = positions.first().copied() else {
        return true;
    };
    let (mut min, mut max) = (first, first);
    for p in positions {
        for axis in 0..3 {
            min[axis] = min[axis].min(p[axis]);
            max[axis] = max[axis].max(p[axis]);
        }
    }
    let flat: Vec<usize> = (0..3).filter(|&a| max[a] - min[a] < 1e-3).collect();
    if flat.is_empty() {
        return true;
    }
    let mut spare = first;
    for &axis in &flat {
        spare[axis] += 1.0;
    }
    let ids: Vec<_> = mesh
        .attributes()
        .map(|(attribute, _)| attribute.id)
        .collect();
    for id in ids {
        match mesh.attribute_mut(id) {
            Some(VertexAttributeValues::Float32x3(values)) => {
                let value = if id == Mesh::ATTRIBUTE_POSITION.id {
                    spare
                } else {
                    values[0]
                };
                values.push(value);
            }
            Some(VertexAttributeValues::Float32x2(values)) => values.push(values[0]),
            Some(VertexAttributeValues::Float32x4(values)) => values.push(values[0]),
            _ => return false,
        }
    }
    true
}

/// The mesh with its attributes packed, unless packing is off or it fails, in which case the mesh
/// is returned as it was, which draws the same. Call it as late as possible: once a mesh is
/// packed, code that reads its float attributes back no longer works.
pub fn pack(mut mesh: Mesh) -> Mesh {
    // Splitting foliage from ground can leave an empty solid mesh, with no bounds to quantize.
    if mesh.count_vertices() == 0 || !PACKED.load(Ordering::Relaxed) {
        return mesh;
    }
    if !give_bounds_size(&mut mesh) {
        return mesh;
    }
    let args = MeshCompressionArgs {
        compress_indices: true,
        quantize_attributes: [(Mesh::ATTRIBUTE_COLOR.id, AttributeQuantization::Float16)]
            .as_slice()
            .into(),
        compress_attributes: MeshAttributeCompressionFlags::COMPRESS_POSITION
            | MeshAttributeCompressionFlags::COMPRESS_NORMAL
            | MeshAttributeCompressionFlags::COMPRESS_UV0
            | MeshAttributeCompressionFlags::COMPRESS_UV1,
    };
    match mesh.compressed_mesh(&args) {
        Ok(packed) => packed,
        Err((mesh, errors)) => {
            tracing::warn!("a mesh could not be packed: {errors:?}");
            mesh
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::{ChunkCoordinate, ChunkData};
    use crate::generate::generator::{generate_chunk, generate_chunk_mesh};
    use crate::generate::region_store::RegionStore;
    use crate::group::merge_chunk_meshes;
    use bevy::math::{I64Vec3, Vec3};
    use std::collections::HashMap;
    use std::sync::Arc;

    // What the vertex shader makes of a packed position: the mesh's bounds' centre plus its half
    // size times the 16-bit fraction.
    fn decoded_positions(mesh: &Mesh) -> Vec<[f32; 3]> {
        let aabb = mesh.final_aabb.expect("bounds");
        let (center, half) = ((aabb.min + aabb.max) * 0.5, (aabb.max - aabb.min) * 0.5);
        match mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
            Some(VertexAttributeValues::Snorm16x4(values)) => values
                .iter()
                .map(|v| {
                    let at = |i: usize| (v[i] as f32 / 32767.0).max(-1.0);
                    [
                        center.x + half.x * at(0),
                        center.y + half.y * at(1),
                        center.z + half.z * at(2),
                    ]
                })
                .collect(),
            other => panic!("positions are {other:?}"),
        }
    }

    // Groups of 2x2x2 chunks over ground of every kind, merged as the game merges them, then packed.
    #[test]
    fn test_packed_full_detail_group_meshes_decode_to_what_they_were() {
        set_packed(true);
        let terrain = ochre_world::worldmap::test_terrain(5);
        let regions = Arc::new(RegionStore::new(terrain.clone()));
        let (mut checked, mut worst) = (0, 0.0f32);
        let mut worst_at = String::new();
        for (gx, gz) in (-6..6).flat_map(|x| (-6..6).map(move |z| (x * 2, z * 2))) {
            let ground = regions.get_height(gx * 16, gz * 16) as i64;
            let gy = (ground.div_euclid(16) - 1) & !1;
            let mut data: HashMap<ChunkCoordinate, Arc<ChunkData>> = HashMap::new();
            for dx in -1..3 {
                for dy in -1..3 {
                    for dz in -1..3 {
                        let c = ChunkCoordinate(I64Vec3::new(gx + dx, gy + dy, gz + dz));
                        data.insert(c, Arc::new(generate_chunk(regions.clone(), c)));
                    }
                }
            }
            let mut parts = Vec::new();
            for dx in 0..2 {
                for dy in 0..2 {
                    for dz in 0..2 {
                        let c = ChunkCoordinate(I64Vec3::new(gx + dx, gy + dy, gz + dz));
                        let neighbours = c
                            .neighbors_26()
                            .iter()
                            .map(|n| data.get(n).cloned())
                            .collect();
                        let meshes =
                            generate_chunk_mesh(data[&c].clone(), neighbours, c, &regions, false);
                        parts.push((Vec3::new(dx as f32, dy as f32, dz as f32) * 16.0, meshes));
                    }
                }
            }
            let merged = merge_chunk_meshes(parts);
            for original in [Some(merged.opaque), merged.water].into_iter().flatten() {
                if original.count_vertices() == 0 {
                    continue;
                }
                let before: Vec<[f32; 3]> = original
                    .attribute(Mesh::ATTRIBUTE_POSITION)
                    .and_then(VertexAttributeValues::as_float3)
                    .unwrap()
                    .to_vec();
                let packed = pack(original);
                let after = decoded_positions(&packed);
                for (i, (a, b)) in before.iter().zip(&after).enumerate() {
                    let error = (0..3).map(|k| (a[k] - b[k]).abs()).fold(0.0, f32::max);
                    if error.is_nan() || error > worst {
                        worst = error;
                        worst_at =
                            format!("group ({gx}, {gy}, {gz}) vertex {i}: {a:?} became {b:?}");
                    }
                }
                checked += 1;
            }
        }
        assert!(checked > 20, "only {checked} meshes");
        assert!(worst < 0.01, "worst error {worst}: {worst_at}");
    }

    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::{Indices, PrimitiveTopology};

    // A square of ground at one height, as the top of a level stretch of ground is.
    fn flat() -> Mesh {
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vec![
                [0.0, 5.0, 0.0],
                [16.0, 5.0, 0.0],
                [16.0, 5.0, 16.0],
                [0.0, 5.0, 16.0],
            ],
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 1.0, 0.0]; 4]);
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_UV_0,
            vec![[0.0, 0.0], [16.0, 0.0], [16.0, 16.0], [0.0, 16.0]],
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[1.0, 1.0, 1.0, 3.0]; 4]);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, vec![[1.0, 0.0]; 4]);
        mesh.insert_indices(Indices::U32(vec![0, 2, 1, 0, 3, 2]));
        mesh
    }

    #[test]
    fn test_a_flat_mesh_is_given_room_to_pack_into_and_still_draws_the_same_triangles() {
        set_packed(true);
        let packed = pack(flat());
        // every axis of its bounds has size, or positions would be divided by nothing
        let aabb = packed.final_aabb.expect("packing records the bounds");
        let size = aabb.max - aabb.min;
        assert!(size.x > 0.0 && size.y > 0.0 && size.z > 0.0, "{size:?}");
        // the spare vertex is extra and unused: the same triangles, now with 16-bit indices
        assert_eq!(5, packed.count_vertices());
        match packed.indices() {
            Some(Indices::U16(indices)) => assert_eq!(&[0, 2, 1, 0, 3, 2], indices.as_slice()),
            other => panic!("indices are {other:?}"),
        }
        // and a vertex is now far smaller than the 56 bytes it was
        assert!(
            packed.get_vertex_size() <= 32,
            "{} bytes",
            packed.get_vertex_size()
        );
    }

    #[test]
    fn test_bevy_packs_a_flat_mesh_to_bounds_with_no_height_which_is_why_it_is_padded() {
        let mut mesh = flat();
        mesh.compress_positions().expect("packs");
        let aabb = mesh.final_aabb.expect("bounds");
        assert_eq!(
            0.0,
            aabb.max.y - aabb.min.y,
            "positions are encoded over bounds with no height"
        );
    }

    #[test]
    fn test_a_mesh_with_size_on_every_axis_gets_no_spare_vertex() {
        set_packed(true);
        let mut mesh = flat();
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vec![
                [0.0, 0.0, 0.0],
                [16.0, 3.0, 0.0],
                [16.0, 5.0, 16.0],
                [0.0, 1.0, 16.0],
            ],
        );
        assert_eq!(4, pack(mesh).count_vertices());
    }
}
