use super::buffers::{
    tiled_uvs, ChunkMeshes, GrassTint, MeshBuffers, MeshStyle, BOX_TEXTURE_MAX_CELL, CUBE,
    TEXTURED, TEXTURED_SOLID,
};
use super::greedy::mesh_opaque_greedy;
use super::shading::{
    corner_signs, vertex_shore_distance, vertex_shore_factor, vertex_water_depth, water_corner,
    WaterCorner, FACE_NORMAL_OFFSET, WATER_BANK_MARGIN, WATER_BED_MARGIN, WATER_DROP,
};
use super::source::{grid_index, BlockSource, BoxFaces, GridView};
use super::terrain::is_solid;
use crate::chunk::CHUNK_SIZE;
use crate::generate::light::SkyLight;
use crate::generate::light::MAX_LIGHT;
use crate::generate::region_store::water_block_top;
use bevy::math::IVec3;
use bevy::math::Vec3;
use bevy::math::{Vec2, Vec3Swizzles};
use ochre_base::block::BlockType;
use ochre_base::block::Face;
use ochre_base::block::Tile;
use ochre_base::noise::unit;
use std::cell::RefCell;
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
    let mut spray = MeshBuffers::default();
    let mut inner_leaves = MeshBuffers::default();

    // water's surface sits below a full block's height, see `WATER_TOP`
    // how far the sides of a sloped top block reach down into the block under, to meet a lower surface
    const WATER_SKIRT: f32 = 0.25;
    let lowering = WATER_DROP / cell_size as f32;
    // `top_at` is a sloped surface's height at a corner and its slope there, which the top's normals follow
    let water_face =
        |i: usize,
         top: f32,
         skirt: bool,
         top_at: &mut dyn FnMut([f32; 3]) -> Option<(f32, [f32; 2])>| {
            CUBE[i].map(|mut v| {
                if v.position[1] == 0.5 {
                    let at = top_at(v.position);
                    v.position[1] = at.map_or(top, |(height, _)| height);
                    if let Some((_, [x, z])) = at {
                        v.normal = Vec3::new(-x, 1.0, -z).normalize().to_array();
                    }
                } else if skirt && i != 4 && i != 5 {
                    v.position[1] = (v.position[1] - WATER_SKIRT)
                        .min(top_at(v.position).map_or(top, |(height, _)| height) - WATER_SKIRT);
                }
                v
            })
        };

    // columns' water depths, shared between the corners that sample them
    let mut depths = HashMap::new();
    let mut depth_of = |coord: IVec3| {
        // a sloped column is measured from its own top block, so blocks either side of a step agree on it,
        // and to at most a chunk down, all any chunk the top could be drawn by sees (the swell's full by then)
        let level = source.water_block_level(coord.x, coord.z);
        let key = if level.is_some() {
            coord.with_y(i32::MIN)
        } else {
            coord
        };
        *depths.entry(key).or_insert_with(|| match level {
            Some(level) => {
                let top = coord.with_y(water_block_top(level) as i32 - 1);
                (0..CHUNK_SIZE as i32)
                    .take_while(|&d| source.terrain(top - IVec3::Y * d) == BlockType::Water)
                    .count() as f32
            }
            None => source.water_depth_blocks(coord),
        })
    };

    let size = CHUNK_SIZE as i32;
    let mut blocks = vec![BlockType::Air; (size * size * size) as usize];
    let non_air = source.non_air();
    for &(coord, block) in &non_air {
        blocks[grid_index(size, coord)] = block;
    }

    // each column's surface at each of its corners and the columns joining it there, shared by its blocks
    let corners = RefCell::new(HashMap::new());
    let corner_of = |block_coord: IVec3, level: f32, position: [f32; 3]| {
        let (dx, dz) = corner_signs(position);
        let key = (block_coord.x, block_coord.z, dx, dz);
        if let Some(&found) = corners.borrow().get(&key) {
            return found;
        }
        let mut corner = water_corner(
            &mut |x, z| source.water_level(x, z),
            (block_coord.x, block_coord.z),
            level,
            (dx, dz),
        );
        // held over the bed of every column joining it, or a ramp down a steep run dips under the beds and they show through
        let bed = corner
            .columns()
            .iter()
            .filter_map(|&(x, z)| {
                let top = water_block_top(source.water_block_level(x, z)?) as i32 - 1;
                let water = (0..CHUNK_SIZE as i32)
                    .take_while(|&d| source.terrain(IVec3::new(x, top - d, z)) == BlockType::Water)
                    .count() as i32;
                (water > 0).then_some((top - water + 1) as f32)
            })
            .fold(f32::NEG_INFINITY, f32::max);
        corner.height = corner.height.max(bed + WATER_DROP + WATER_BED_MARGIN);
        // held under the ground of any dry column solid just under the corner, which its blocks all see alike
        let under = (corner.height - WATER_DROP).floor() as i32 - 1;
        let bank = [(dx, 0), (0, dz), (dx, dz)]
            .into_iter()
            .map(|(ox, oz)| IVec3::new(block_coord.x + ox, under, block_coord.z + oz))
            .filter(|c| source.water_level(c.x, c.z).is_none() && is_solid(source.block(*c)))
            .map(|c| {
                (1..)
                    .find(|&up| !is_solid(source.block(c + IVec3::Y * up)))
                    .unwrap() as f32
                    + c.y as f32
            })
            .fold(f32::INFINITY, f32::min);
        if !source.flowing(block_coord.x, block_coord.z) {
            corner.height = corner.height.min(bank + WATER_DROP - WATER_BANK_MARGIN);
        }
        corners.borrow_mut().insert(key, corner);
        corner
    };

    for &(block_coord, block) in &non_air {
        if block != BlockType::Water {
            continue;
        }

        // shallow water in a coarse cell is drawn to its real surface, not the cell's top
        let surface_top = source.water_surface_top(block_coord);
        let top = surface_top.map_or(0.5 - lowering, |top| (top - lowering).max(-0.45));
        // fresh water at full detail is drawn to its spread surface, a slope down the river, not to whole blocks
        let level = if cell_size == 1 && surface_top.is_none() {
            source.water_level(block_coord.x, block_coord.z)
        } else {
            None
        };
        // the block the real surface lies in, as `water_block_top` fills to: the surface may leave it, the blocks under it are cut to it
        let top_block = source
            .water_block_level(block_coord.x, block_coord.z)
            .is_some_and(|level| water_block_top(level) as i32 - 1 == block_coord.y);
        let open = |block: BlockType| {
            matches!(block, BlockType::Air | BlockType::Leaves) || block.is_flora()
        };
        let under = block_coord - IVec3::Y;
        let water_face_at = |i: usize| {
            // no skirt where flowing water under has a side of its own there, as down a fall, or the two overlap
            let skirt = level.is_some()
                && top_block
                && !(source.flowing(block_coord.x, block_coord.z)
                    && source.block(under) == BlockType::Water
                    && open(source.block(under + FACE_NORMAL_OFFSET[i])));
            water_face(i, top, skirt, &mut |position| {
                let corner = corner_of(block_coord, level?, position);
                // unclamped to the block, so blocks either side of a step meet at their shared corner, even outside it
                let height = corner.height - WATER_DROP - block_coord.y as f32 - 0.5;
                Some((
                    if top_block { height } else { height.min(0.5) },
                    corner.slope,
                ))
            })
        };
        for (i, offset) in FACE_NORMAL_OFFSET.into_iter().enumerate() {
            if !open(source.block(block_coord + offset)) {
                continue;
            }
            let face = water_face_at(i);
            // a step down to a column whose surface meets this one's is closed by the sloped tops
            let beside = (block_coord.x + offset.x, block_coord.z + offset.z);
            if i < 4
                && level.is_some_and(|own| {
                    CUBE[i]
                        .iter()
                        .filter(|v| v.position[1] == 0.5)
                        .all(|v| corner_of(block_coord, own, v.position).joins(beside))
                })
            {
                continue;
            }
            // a side of a block the spread surface is lowered under
            if i < 4 && !top_block && face.iter().all(|v| v.position[1] <= -0.5) {
                continue;
            }
            // Only the top face carries the shore, depth, top flag and shore distance (R, G, B,
            // A), as foam and waves are surface-only. Waves are gated by the flag alone, so
            // neighbouring columns of different depths move their shared edge alike.
            let colors = face.map(|v| {
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
                &face,
                block_coord.as_vec3(),
                Tile::Water.atlas_cell(),
                &colors,
            );
            if let Some(own) = level.filter(|_| top_block) {
                let height = face.iter().map(|v| v.position[1]).sum::<f32>() / 4.0;
                if i == 4 && source.spring_pool(block_coord.x, block_coord.z) {
                    push_bubbles(&mut spray, block_coord, height);
                }
                push_spray(&mut spray, block_coord, i, own, height, source, &corner_of);
            }
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
        spray: (!spray.vertices.is_empty()).then(|| spray.build_mesh()),
        inner_leaves: (!inner_leaves.vertices.is_empty()).then(|| inner_leaves.build_mesh()),
    }
}

// Fresh water throws up spray where a run steeper than this, as a drop of about 3 blocks spreads to, eases off, and where it falls.
const SPRAY_SLOPE: f32 = 0.3;
// Rising plumes and scattering droplets thrown up at each.
const SPRAY_PLUMES: u32 = 2;
const SPRAY_DROPLETS: u32 = 4;
// Bubbles rising through each block of a spring's pool.
const SPRING_BUBBLES: u32 = 3;
// The share of them left out at random.
const SPRAY_GAPS: f32 = 0.45;

// Three values 0..1 for the `k`th particle off face `i` of the block at `block_coord`.
fn spray_random(block_coord: IVec3, i: usize, k: u32) -> [f32; 3] {
    let seed = (block_coord.y as u32).wrapping_mul(97) ^ (i as u32 * 64 + k * 3);
    [0, 1, 2].map(|j| unit(seed + j, block_coord.x, block_coord.z) as f32)
}

// A spring's pool bubbles up from its bed through the surface `height` up the block at `block_coord`.
fn push_bubbles(spray: &mut MeshBuffers, block_coord: IVec3, height: f32) {
    for k in 0..SPRING_BUBBLES {
        let [u, v, seed] = spray_random(block_coord, 4, k + 16);
        let at = block_coord.as_vec3() + Vec3::new(u - 0.5, height - 0.6, v - 0.5);
        spray.push_particle(
            at,
            Vec3::Y * 0.9,
            seed,
            0.06,
            false,
            spray_random(block_coord, 4, k + 20)[0],
        );
    }
}

// Spray off a sloped top block's face, its surface `height` up it: at the foot of a steep run, and of a fall down a side.
fn push_spray(
    spray: &mut MeshBuffers,
    block_coord: IVec3,
    i: usize,
    own: f32,
    height: f32,
    source: &impl BlockSource,
    corner_of: &impl Fn(IVec3, f32, [f32; 3]) -> WaterCorner,
) {
    let random = |k: u32| spray_random(block_coord, i, k);
    let slope_of = |column: IVec3, level: f32| {
        CUBE[4].iter().fold(Vec2::ZERO, |sum, v| {
            sum + Vec2::from(corner_of(column, level, v.position).slope) / 4.0
        })
    };
    // where on the face, which way it's thrown and how hard
    let (base, down, strength) = if i == 4 {
        let slope = slope_of(block_coord, own);
        let steep = slope.length();
        if steep <= SPRAY_SLOPE {
            return;
        }
        let down = -slope / steep;
        // only at the foot, a band where one of the next two columns downhill runs gentler, or there are none
        let step = if down.x.abs() > down.y.abs() {
            IVec3::new(down.x.signum() as i32, 0, 0)
        } else {
            IVec3::new(0, 0, down.y.signum() as i32)
        };
        let easing = [1, 2].into_iter().any(|n| {
            let next = block_coord + step * n;
            source
                .water_level(next.x, next.z)
                .is_none_or(|level| slope_of(next, level).length() < 0.75 * steep)
        });
        if !easing {
            return;
        }
        (
            block_coord.as_vec3() + Vec3::Y * height + down.extend(0.0).xzy() * 0.3,
            down,
            steep,
        )
    } else if i < 4 {
        // a fall, onto the lower water beside
        let offset = FACE_NORMAL_OFFSET[i];
        let Some(below) = source.water_level(block_coord.x + offset.x, block_coord.z + offset.z)
        else {
            return;
        };
        let foot = below - WATER_DROP - 0.5;
        let base = block_coord.with_y(0).as_vec3() + offset.as_vec3() * 0.6 + Vec3::Y * foot;
        (base, offset.as_vec3().xz(), (own - below).max(1.0))
    } else {
        return;
    };
    let along = Vec3::new(-down.y, 0.0, down.x);
    let burst = random(15)[0];
    for k in 0..SPRAY_PLUMES + SPRAY_DROPLETS {
        let [u, v, seed] = random(k);
        // some left out at random, so a foot's spray is scattered, not a row along it
        if random(k + 8)[0] < SPRAY_GAPS {
            continue;
        }
        let at = base + along * (u - 0.5) + down.extend(0.0).xzy() * (v - 0.5);
        if k < SPRAY_PLUMES {
            // a plume rises off the foot, leaning a little downstream
            let flow = down.extend(0.0).xzy() * 0.4 + Vec3::Y * (0.8 + 0.6 * strength.min(2.0));
            spray.push_particle(at, flow, seed, 0.16, false, burst);
        } else {
            // droplets scatter, mostly downstream, the shader spreading each life another way
            let flow = down.extend(0.0).xzy() * (1.2 + strength.min(2.0))
                + Vec3::Y * (1.5 + strength.min(2.0));
            spray.push_particle(at, flow, seed, 0.06, true, burst);
        }
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
