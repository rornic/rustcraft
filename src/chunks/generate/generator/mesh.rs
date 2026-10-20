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
use crate::chunks::generate::light::MAX_LIGHT;
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
    let mut inner_leaves = MeshBuffers::default();

    // water's surface sits below a full block's height, see `WATER_TOP`
    let lowering = (0.5 - WATER_TOP) / cell_size as f32;
    let water_face = |i: usize, top: f32| {
        CUBE[i].map(|mut v| {
            if v.position[1] == 0.5 {
                v.position[1] = top;
            }
            v
        })
    };

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

    for &(block_coord, block) in &non_air {
        if block != BlockType::Water {
            continue;
        }

        // shallow water in a coarse cell is drawn to its real surface, not the cell's top
        let top = source
            .water_surface_top(block_coord)
            .map_or(0.5 - lowering, |top| (top - lowering).max(-0.45));
        let water_faces: [_; 6] = std::array::from_fn(|i| water_face(i, top));
        for (i, offset) in FACE_NORMAL_OFFSET.into_iter().enumerate() {
            let neighbour = source.block(block_coord + offset);
            if !matches!(neighbour, BlockType::Air | BlockType::Leaves) && !neighbour.is_flora() {
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
        &mut inner_leaves,
        style,
        if textured { cell_size as f32 } else { 1.0 },
        grass_tint,
        light,
    );
    opaque.tint_grass_sides(grass_tint);
    // flora only grows at full detail, so coarser cells have none to look for
    if cell_size == 1 {
        mesh_flora(&non_air, &mut opaque, grass_tint, source, light);
    }
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
            BoxFaces::Sides => &CUBE[..4],
        };
        for face in faces {
            let tile = tree_box.block.tile(Face::of_normal_y(face[0].normal[1]));
            let positions = std::array::from_fn(|j| {
                let mut corner = Vec3::from(face[j].position);
                if corner.y > 0.0 {
                    corner *= Vec3::new(1.0 - tree_box.taper, 1.0, 1.0 - tree_box.taper);
                }
                (centre + corner * extent).to_array()
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
        inner_leaves: (!inner_leaves.vertices.is_empty()).then(|| inner_leaves.build_mesh()),
    }
}

// How far in from the block's corners flora's quads reach, so neighbouring plants don't touch.
const FLORA_INSET: f32 = 0.05;
// How far a plant may stand off its block's centre, and how much shorter than a block grass
// and other plants may be: grass varies far more, so a meadow isn't cut level.
const FLORA_JITTER: f32 = 0.18;
const GRASS_SHORTEN: f32 = 0.45;
const FLORA_SHORTEN: f32 = 0.2;
// How dark a plant is at its roots, to its tips, as the grass round it shades them.
const FLORA_ROOT_SHADE: f32 = 0.65;

// Flora as two quads crossed corner to corner, both sides drawn as block meshes cull one,
// v = 0 at the top so `foliage.wesl` sways the tips, and facing up to light like the grass.
fn mesh_flora(
    non_air: &[(IVec3, BlockType)],
    buffers: &mut MeshBuffers,
    grass_tint: &GrassTint,
    source: &impl BlockSource,
    light: &impl Deref<Target = SkyLight>,
) {
    for &(coord, block) in non_air {
        if !block.is_flora() {
            continue;
        }
        // where in the chunk it is picks how it stands, so it's the same every time it's meshed
        let jitter = |salt: u32| {
            let h = (coord.x as u32)
                .wrapping_mul(0x9E37_79B1)
                .wrapping_add((coord.z as u32).wrapping_mul(0x85EB_CA77))
                .wrapping_add(salt.wrapping_mul(0xC2B2_AE3D));
            let h = (h ^ (h >> 15)).wrapping_mul(0x2C1B_3C6D);
            (h ^ (h >> 13)) as f32 / u32::MAX as f32
        };
        let centre = coord.as_vec3()
            + Vec3::new(
                (jitter(1) - 0.5) * 2.0 * FLORA_JITTER,
                0.0,
                (jitter(2) - 0.5) * 2.0 * FLORA_JITTER,
            );
        let shorten = match block {
            BlockType::TallGrass | BlockType::ShortGrass | BlockType::SeedGrass => GRASS_SHORTEN,
            _ => FLORA_SHORTEN,
        };
        let (bottom, top) = (-0.5, 0.5 - jitter(3) * shorten);
        let reach = 0.5 - FLORA_INSET;
        let tile = block.tile(Face::Side);
        // tinted just as the grass it grows from, see `mesh_opaque_greedy`
        let ground = coord.as_vec3() - Vec3::Y * 0.5;
        let tint = if tile.tinted() {
            grass_tint.at(ground) * source.forest_shade(ground)
        } else {
            Vec3::ONE
        };
        let lit = light.level(coord) as f32 / MAX_LIGHT as f32;
        let (root, tip) = (
            (tint * FLORA_ROOT_SHADE)
                .extend(tile.atlas_cell())
                .to_array(),
            tint.extend(tile.atlas_cell()).to_array(),
        );
        for (dx, dz) in [(reach, reach), (reach, -reach)] {
            let corner =
                |side: f32, y: f32| (centre + Vec3::new(side * dx, y, side * dz)).to_array();
            let (top_a, bottom_a, top_b, bottom_b) = (
                corner(-1.0, top),
                corner(-1.0, bottom),
                corner(1.0, top),
                corner(1.0, bottom),
            );
            // the second winding faces the other way, as the material culls one side
            for (positions, uvs) in [
                (
                    [top_a, bottom_a, top_b, bottom_b],
                    [[0.0, 0.0], [0.0, 1.0], [1.0, 0.0], [1.0, 1.0]],
                ),
                (
                    [top_b, bottom_b, top_a, bottom_a],
                    [[1.0, 0.0], [1.0, 1.0], [0.0, 0.0], [0.0, 1.0]],
                ),
            ] {
                buffers.push_tiled_quad(
                    positions,
                    [0.0, 1.0, 0.0],
                    uvs,
                    [tip, root, tip, root],
                    [lit; 4],
                );
            }
        }
    }
}
