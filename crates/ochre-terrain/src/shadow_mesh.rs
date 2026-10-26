//! Separates cutout terrain before packing, without changing triangle or vertex order within a mesh.

use bevy::{
    mesh::{Indices, VertexAttributeValues},
    prelude::Mesh,
};
use ochre_base::block::Tile;

pub(crate) fn split(mesh: Mesh) -> (Mesh, Option<Mesh>) {
    if mesh.count_vertices() == 0 {
        return (mesh, None);
    }
    let Some(VertexAttributeValues::Float32x4(colors)) = mesh.attribute(Mesh::ATTRIBUTE_COLOR)
    else {
        panic!("split terrain before packing");
    };
    let leaf = Tile::Leaves.atlas_cell();
    let flora = Tile::FIRST_FLORA.atlas_cell();
    let flags = match mesh.attribute(Mesh::ATTRIBUTE_UV_1) {
        Some(VertexAttributeValues::Float32x2(flags)) => Some(flags),
        _ => None,
    };
    let cutout: Vec<bool> = colors
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let solid_leaf = flags.is_some_and(|flags| flags[i][1] > 0.5);
            ((c[3] - leaf).abs() < 0.5 && !solid_leaf) || c[3] > flora - 0.5
        })
        .collect();
    if !cutout.iter().any(|c| *c) {
        return (mesh, None);
    }
    let mut vertices = [Vec::new(), Vec::new()];
    let mut remap = vec![0; colors.len()];
    for (i, &is_cutout) in cutout.iter().enumerate() {
        let part = &mut vertices[is_cutout as usize];
        remap[i] = part.len() as u32;
        part.push(i);
    }
    let mut indices = [Vec::new(), Vec::new()];
    let source: Vec<usize> = mesh.indices().expect("indexed terrain").iter().collect();
    let (triangles, remainder) = source.as_chunks::<3>();
    assert!(remainder.is_empty(), "terrain triangle list");
    for triangle in triangles {
        let class = cutout[triangle[0]];
        assert!(
            triangle.iter().all(|&v| cutout[v] == class),
            "mixed-material terrain triangle"
        );
        indices[class as usize].extend(triangle.iter().map(|&v| remap[v]));
    }
    let mut parts = vertices
        .into_iter()
        .zip(indices)
        .map(|(vertices, indices)| {
            let mut part = Mesh::new(mesh.primitive_topology(), mesh.asset_usage);
            for (attribute, values) in mesh.attributes() {
                let values = match values {
                    VertexAttributeValues::Float32x2(v) => {
                        VertexAttributeValues::Float32x2(vertices.iter().map(|&i| v[i]).collect())
                    }
                    VertexAttributeValues::Float32x3(v) => {
                        VertexAttributeValues::Float32x3(vertices.iter().map(|&i| v[i]).collect())
                    }
                    VertexAttributeValues::Float32x4(v) => {
                        VertexAttributeValues::Float32x4(vertices.iter().map(|&i| v[i]).collect())
                    }
                    _ => panic!("split terrain before packing"),
                };
                part.insert_attribute(*attribute, values);
            }
            part.insert_indices(Indices::U32(indices));
            part
        });
    (parts.next().unwrap(), parts.next())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::{asset::RenderAssetUsages, mesh::PrimitiveTopology};

    #[test]
    fn split_preserves_attributes_vertices_and_triangle_winding() {
        let cells = [Tile::Stone, Tile::Leaves, Tile::Dirt, Tile::FIRST_FLORA];
        let colors: Vec<[f32; 4]> = cells
            .iter()
            .flat_map(|tile| [[0.2, 0.3, 0.4, tile.atlas_cell()]; 4])
            .collect();
        let positions: Vec<[f32; 3]> = (0..16).map(|i| [i as f32, 1.0, 2.0]).collect();
        let indices: Vec<u32> = (0..4)
            .flat_map(|i| [0, 1, 2, 2, 1, 3].map(|v| v + 4 * i))
            .collect();
        let mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions.clone())
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_1, vec![[0.8, 0.0]; 16])
        .with_inserted_indices(Indices::U32(indices));
        let (solid, cutout) = split(mesh);
        let cutout = cutout.unwrap();
        assert_eq!(solid.count_vertices() + cutout.count_vertices(), 16);
        for (part, originals) in [
            (&solid, [0, 1, 2, 3, 8, 9, 10, 11]),
            (&cutout, [4, 5, 6, 7, 12, 13, 14, 15]),
        ] {
            assert_eq!(
                part.attribute(Mesh::ATTRIBUTE_POSITION)
                    .unwrap()
                    .as_float3()
                    .unwrap(),
                originals.map(|i| positions[i])
            );
            assert_eq!(
                part.indices().unwrap().iter().collect::<Vec<_>>(),
                [0, 1, 2, 2, 1, 3, 4, 5, 6, 6, 5, 7]
            );
            assert!(part.attribute(Mesh::ATTRIBUTE_UV_1).is_some());
        }
    }

    #[test]
    fn coarse_leaf_faces_stay_in_the_solid_mesh() {
        let mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0, 0.0, 0.0]; 3])
        .with_inserted_attribute(
            Mesh::ATTRIBUTE_COLOR,
            vec![[1.0, 1.0, 1.0, Tile::Leaves.atlas_cell()]; 3],
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_1, vec![[1.0, 0.75]; 3])
        .with_inserted_indices(Indices::U32(vec![0, 1, 2]));
        let (solid, cutout) = split(mesh);
        assert_eq!(solid.count_vertices(), 3);
        assert!(cutout.is_none());
    }
}
