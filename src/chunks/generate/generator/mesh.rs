use super::buffers::{
    tiled_uvs, ChunkMeshes, GrassTint, MeshBuffers, MeshStyle, BOX_TEXTURE_MAX_CELL, CUBE,
    TEXTURED, TEXTURED_SOLID,
};
use super::greedy::mesh_opaque_greedy;
use super::shading::{
    vertex_shore_distance, vertex_shore_factor, vertex_water_depth, FACE_NORMAL_OFFSET, WATER_TOP,
};
use super::source::{grid_index, BlockSource, BoxFaces, GridView};
use crate::block::BlockType;
use crate::block::Face;
use crate::block::Tile;
use crate::chunks::chunk::CHUNK_SIZE;
use crate::chunks::generate::light::SkyLight;
use bevy::math::IVec3;
use bevy::math::Vec3;
use std::collections::HashMap;
use std::ops::Deref;

// `cell_size` is how many blocks wide each cell of `source` is.
pub(super) fn mesh_blocks(
    source: &impl BlockSource,
    cell_size: i64,
    style: MeshStyle,
    grass_tint: &GrassTint,
    light: &impl Deref<Target = SkyLight>,
) -> ChunkMeshes {
    let MeshStyle { textured, .. } = style;
    let mut opaque = MeshBuffers {
        untextured: !textured,
        ..Default::default()
    };
    let mut water = MeshBuffers::default();

    // water's surface sits below a full block's height, see `WATER_TOP`
    let water_top = 0.5 - (0.5 - WATER_TOP) / cell_size as f32;
    let water_faces = CUBE.map(|face| {
        face.map(|mut v| {
            if v.position[1] == 0.5 {
                v.position[1] = water_top;
            }
            v
        })
    });

    // columns' water depths, shared between the corners that sample them
    let mut depths = HashMap::new();
    let mut depth_of = |coord: IVec3| {
        *depths
            .entry(coord)
            .or_insert_with(|| source.water_depth_blocks(coord))
    };

    let size = CHUNK_SIZE as i32;
    let mut blocks = vec![BlockType::Air; (size * size * size) as usize];
    let non_air = source.non_air();
    for &(coord, block) in &non_air {
        blocks[grid_index(size, coord)] = block;
    }

    for (block_coord, block) in non_air {
        if block != BlockType::Water {
            continue;
        }

        for (i, offset) in FACE_NORMAL_OFFSET.into_iter().enumerate() {
            if !matches!(
                source.block(block_coord + offset),
                BlockType::Air | BlockType::Leaves
            ) {
                continue;
            }
            // Only the top face carries the shore, depth, top flag and shore distance (R, G, B,
            // A), as foam and waves are surface-only. Waves are gated by the flag alone, so
            // neighbouring columns of different depths move their shared edge alike.
            let colors = water_faces[i].map(|v| {
                if i != 4 {
                    return [0.0, 0.0, 0.0, 1.0];
                }
                [
                    vertex_shore_factor(source, block_coord, i, v.position),
                    vertex_water_depth(&mut depth_of, block_coord, i, v.position)
                        / CHUNK_SIZE as f32,
                    1.0,
                    vertex_shore_distance(source, block_coord, i, v.position),
                ]
            });
            water.push_face(
                &water_faces[i],
                block_coord.as_vec3(),
                Tile::Water.atlas_cell(),
                &colors,
            );
        }
    }

    mesh_opaque_greedy(
        &GridView {
            size,
            blocks: &blocks,
            source,
        },
        &CUBE,
        &mut opaque,
        style,
        if textured { cell_size as f32 } else { 1.0 },
        grass_tint,
        light,
    );
    opaque.tint_grass_sides(grass_tint);
    // sky light there is rarely bright under a canopy, and these boxes are too small to shade smoothly
    const BOX_LIGHT: f32 = 0.8;
    // past them, so wide a box would show through to its back where leaves' holes line up, so
    // they're filled
    let box_holes = cell_size <= BOX_TEXTURE_MAX_CELL;
    opaque.boxes = Some((
        opaque.vertices.len(),
        if box_holes { TEXTURED } else { TEXTURED_SOLID },
    ));
    for tree_box in source.tree_boxes() {
        let (centre, extent) = (
            (tree_box.min + tree_box.max) / 2.0,
            tree_box.max - tree_box.min,
        );
        let tint = if tree_box.block.lod_tile().tinted() {
            grass_tint.at(centre) * tree_box.tint
        } else {
            Vec3::ONE
        };
        let faces = match tree_box.faces {
            BoxFaces::All => &CUBE[..],
            BoxFaces::NoBottom => &CUBE[..5],
            BoxFaces::Sides => &CUBE[..4],
        };
        for face in faces {
            let tile = tree_box.block.tile(Face::of_normal_y(face[0].normal[1]));
            let positions = std::array::from_fn(|j| {
                (centre + Vec3::from(face[j].position) * extent).to_array()
            });
            let uvs = tiled_uvs(
                &positions,
                &std::array::from_fn(|j| face[j].uv),
                cell_size as f32,
            );
            opaque.push_tiled_quad(
                positions,
                face[0].normal,
                uvs,
                [tint.extend(tile.atlas_cell()).to_array(); 4],
                [BOX_LIGHT; 4],
            );
        }
    }

    let water_mesh = if water.vertices.is_empty() {
        None
    } else {
        Some(water.build_mesh())
    };

    ChunkMeshes {
        opaque: opaque.build_mesh(),
        water: water_mesh,
    }
}
