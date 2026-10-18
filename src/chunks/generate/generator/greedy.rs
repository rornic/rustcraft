use super::buffers::Vertex;
use super::buffers::{FanQuad, GrassTint, InnerLeaves, MeshBuffers, MeshStyle};
use super::lod::SKIRT_CELLS;
use super::shading::{
    axis_of, face_light, vertex_ao, vertex_light, AO_GAMMA, FACE_NORMAL_OFFSET, FACE_TANGENTS,
};
use super::source::{BlockSource, GridView};
use super::terrain::is_solid;
use crate::block::BlockType;
use crate::block::Face;
use crate::chunks::chunk::CHUNK_SIZE;
use crate::chunks::generate::light::SkyLight;
use crate::chunks::generate::light::MAX_LIGHT;
use bevy::math::IVec3;
use bevy::math::Vec3;
use std::ops::Deref;

// Which tangent (0 or 1) UV component `k` runs along on this face, so a merged quad can repeat it.
pub(super) fn uv_tangent(face: &[Vertex], k: usize, a_axis: usize) -> usize {
    let high = |v: &Vertex| v.position[a_axis] > 0.0;
    let follows_a = face
        .iter()
        .all(|v| (high(v) == high(&face[0])) == (v.uv[k] == face[0].uv[k]));
    if follows_a {
        0
    } else {
        1
    }
}

#[derive(Copy, Clone)]
pub(super) struct FaceCell {
    pub(super) block: BlockType,
    // the tree's own tint over the grass's, for leaves, see `Tree::tint`
    pub(super) tint: Vec3,
    pub(super) ao: [f32; 4],
    // sky light at each corner, 0..1
    pub(super) light: [f32; 4],
    // where the face starts and ends up the mesh, if not the whole cell, see `face_span`
    pub(super) span: Option<[f32; 2]>,
    // a face between leaves, see `InnerLeaves`
    pub(super) inner: bool,
}

impl FaceCell {
    // Whether its shading is the same at both its ends along `axis` of the face, 0 for its
    // first tangent and 1 for its second, with `corners` where each corner is along them.
    pub(super) fn even_along(&self, corners: &[[bool; 2]; 4], axis: usize) -> bool {
        (0..4).all(|j| {
            (0..4).all(|k| {
                corners[j][1 - axis] != corners[k][1 - axis]
                    || (self.ao[j] == self.ao[k] && self.light[j] == self.light[k])
            })
        })
    }

    // Whether `other`, next to it along `axis`, merges into one quad with it: shaded just the
    // same, and evenly along `axis`, so a gradient across the run is stretched only along
    // where it doesn't change, which leaves the shading as it was.
    pub(super) fn merges_along(
        &self,
        other: &FaceCell,
        corners: &[[bool; 2]; 4],
        axis: usize,
    ) -> bool {
        self.block == other.block
            && self.tint == other.tint
            && self.ao == other.ao
            && self.light == other.light
            && self.span == other.span
            && self.inner == other.inner
            && self.even_along(corners, axis)
    }
}

// Whether a face of `block` at `coord` shows, and if so how much of it up the mesh's y axis,
// with LOD surface cells topped out at the ground's real height (see `LodBlocks::surface_top`):
// `Some(None)` for the whole face. Faces show through leaves' holes, other leaves' too, see
// `InnerLeaves`.
pub(super) fn face_span(
    source: &impl BlockSource,
    coord: IVec3,
    normal: IVec3,
    block: BlockType,
) -> Option<Option<[f32; 2]>> {
    // flora draws as crossed quads of its own, see `mesh_flora`
    if matches!(block, BlockType::Air | BlockType::Water) || block.is_flora() {
        return None;
    }
    let neighbour = coord + normal;
    let neighbour_block = source.block(neighbour);
    let covered = neighbour_block.is_opaque()
        || (block == BlockType::Ice && neighbour_block == BlockType::Ice);
    // every face is asked about, and most are hidden, so those leave before any lookups
    if !source.has_surface_tops() || normal.y < 0 {
        return (!covered).then_some(None);
    }
    if normal.y > 0 {
        // a lowered top still faces the air
        return (!covered).then(|| source.surface_top(coord).map(|t| [t, t]));
    }
    let bottom = coord.y as f32 - 0.5;
    // a lowered neighbour leaves this side bare above the ground's real height
    let floor = if covered {
        source.surface_top(neighbour)?
    } else {
        bottom
    };
    let top = source.surface_top(coord);
    let ceiling = top.unwrap_or(bottom + 1.0);
    if ceiling <= floor {
        return None;
    }
    Some((top.is_some() || covered).then_some([floor, ceiling]))
}

#[cfg(test)]
thread_local! {
    // lets tests mesh face by face, to check merging leaves the shading as it was
    pub(super) static NO_MERGING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

// Merges runs of identically shaded opaque faces into single quads, one face direction slice at a
// time, where the shading doesn't change along the run, see `FaceCell::merges_along`.
// Faces between leaves go to `inner`, or nowhere, see `InnerLeaves`.
#[allow(clippy::too_many_arguments)]
pub(super) fn mesh_opaque_greedy(
    source: &GridView<impl BlockSource>,
    faces: &[[Vertex; 4]; 6],
    buffers: &mut MeshBuffers,
    inner: &mut MeshBuffers,
    style: MeshStyle,
    // how many times the texture repeats across a cell
    uv_tiles: f32,
    grass_tint: &GrassTint,
    light: &impl Deref<Target = SkyLight>,
) {
    let MeshStyle {
        ambient_occlusion,
        textured,
        inner_leaves,
    } = style;
    let size = CHUNK_SIZE as i32;
    let n = size as usize;
    let mut mask: Vec<Option<FaceCell>> = vec![None; n * n];

    for (i, template) in faces.iter().enumerate() {
        let normal = FACE_NORMAL_OFFSET[i];
        let (ta, tb) = FACE_TANGENTS[i];
        let (a_axis, b_axis) = (axis_of(ta), axis_of(tb));
        let uv_axes = [0, 1].map(|k| {
            if uv_tangent(template, k, a_axis) == 0 {
                a_axis
            } else {
                b_axis
            }
        });
        let uv_ascending = [0, 1].map(|k| {
            template
                .iter()
                .find(|v| v.position[uv_axes[k]] > 0.0)
                .is_some_and(|v| v.uv[k] > 0.5)
        });
        let normal_axis = axis_of(normal);
        let normal_unit = normal.abs();
        let (p0, p1, p2) = (
            Vec3::from(template[0].position),
            Vec3::from(template[1].position),
            Vec3::from(template[2].position),
        );
        let ccw_in_ab = (p1 - p0)
            .cross(p2 - p0)
            .dot(ta.as_vec3().cross(tb.as_vec3()))
            > 0.0;
        // whether each corner is at the far end along each tangent
        let corners: [[bool; 2]; 4] = std::array::from_fn(|j| {
            [
                template[j].position[a_axis] > 0.0,
                template[j].position[b_axis] > 0.0,
            ]
        });
        let by_corner = |values: [f32; 4]| {
            let mut out = [[0.0; 2]; 2];
            for (j, [far_a, far_b]) in corners.iter().enumerate() {
                out[*far_a as usize][*far_b as usize] = values[j];
            }
            out
        };

        for slice in 0..size {
            // merging takes every face it finds out of the mask, so it's left empty for the next slice
            let mut faces = 0;
            for b in 0..size {
                for a in 0..size {
                    let coord = normal_unit * slice + ta * a + tb * b;
                    let block = source.block(coord);
                    if matches!(block, BlockType::Air | BlockType::Water) {
                        continue;
                    }
                    // past the mesh's edge the terrain may be drawn at another detail, with no
                    // leaves just there, so leaves on the edge keep their faces
                    let inside = source.contains(coord + normal);
                    let between_leaves = inside
                        && block == BlockType::Leaves
                        && source.block(coord + normal) == BlockType::Leaves;
                    if between_leaves && inner_leaves == InnerLeaves::Hidden {
                        continue;
                    }
                    let Some(span) = face_span(source, coord, normal, block) else {
                        continue;
                    };
                    faces += 1;
                    // unshaded leaves merge into big quads, which keeps the faces between them cheap,
                    // so they and coarse LODs take one light level a face instead of smooth light
                    let smooth = ambient_occlusion && block != BlockType::Leaves;
                    // a step above lowered ground faces into that ground's cell, which is dark: it's
                    // lit by the air over it, and is too thin to shade smoothly
                    let step = normal.y == 0 && source.block(coord + normal).is_opaque();
                    // A skirt wall hangs from the node's own ground over solid cells beyond, whose
                    // occlusion and light are none, so it is lit as that ground is, by the open air above.
                    let skirt = !inside && normal.y == 0 && source.has_skirts();
                    let smooth = smooth && !step && !skirt;
                    let lit_from = if step {
                        coord + normal + IVec3::Y
                    } else {
                        coord + normal
                    };
                    let skirt_light = skirt.then(|| {
                        let air = (0..=SKIRT_CELLS + 1)
                            .find(|&dy| !is_solid(source.terrain(coord + IVec3::Y * dy)));
                        face_light(air.map_or(MAX_LIGHT, |dy| light.level(coord + IVec3::Y * dy)))
                    });
                    mask[(b * size + a) as usize] = Some(FaceCell {
                        span,
                        inner: between_leaves,
                        block,
                        tint: if block == BlockType::Leaves {
                            source.leaf_tint(coord)
                        } else {
                            Vec3::ONE
                        },
                        ao: if smooth {
                            std::array::from_fn(|j| {
                                vertex_ao(source, coord, i, template[j].position)
                            })
                        } else {
                            [1.0; 4]
                        },
                        light: if smooth {
                            std::array::from_fn(|j| {
                                vertex_light(source, light, coord, i, template[j].position)
                            })
                        } else {
                            [skirt_light.unwrap_or_else(|| face_light(light.level(lit_from))); 4]
                        },
                    });
                }
            }
            if faces == 0 {
                continue;
            }

            for b0 in 0..n {
                for a0 in 0..n {
                    let Some(cell) = mask[b0 * n + a0] else {
                        continue;
                    };

                    let (mut w, mut h) = (1, 1);
                    // a trimmed side's top and bottom sit off the cell grid, so it merges only
                    // along the ground's contour, where its neighbours share those heights
                    let trimmed_side = cell.span.is_some() && normal.y == 0;
                    let (grow_a, grow_b) =
                        (!trimmed_side || a_axis != 1, !trimmed_side || b_axis != 1);
                    #[cfg(test)]
                    let (grow_a, grow_b) = if NO_MERGING.get() {
                        (false, false)
                    } else {
                        (grow_a, grow_b)
                    };
                    while grow_a
                        && a0 + w < n
                        && mask[b0 * n + a0 + w].is_some_and(|o| cell.merges_along(&o, &corners, 0))
                    {
                        w += 1;
                    }
                    while grow_b
                        && b0 + h < n
                        && (a0..a0 + w).all(|a| {
                            mask[(b0 + h) * n + a]
                                .is_some_and(|o| cell.merges_along(&o, &corners, 1))
                        })
                    {
                        h += 1;
                    }
                    for b in b0..b0 + h {
                        for a in a0..a0 + w {
                            mask[b * n + a] = None;
                        }
                    }

                    let origin = (normal_unit * slice + ta * a0 as i32 + tb * b0 as i32).as_vec3();
                    let buffers = if cell.inner {
                        &mut *inner
                    } else {
                        &mut *buffers
                    };
                    let tile = if textured {
                        cell.block.tile(Face::of_normal_y(normal.y as f32))
                    } else {
                        cell.block.lod_tile()
                    };
                    let atlas_cell = tile.atlas_cell();
                    let color = |ao: f32, position: Vec3| {
                        // leaves share grass's tint, over a darker texture
                        let tint = if tile.tinted() {
                            grass_tint.at(position) * cell.tint
                        } else {
                            Vec3::ONE
                        };
                        let shade = match cell.block {
                            BlockType::Leaves => source.canopy_shade(position),
                            BlockType::Grass => source.forest_shade(position),
                            _ => 1.0,
                        };
                        (tint * ao.powf(AO_GAMMA) * shade)
                            .extend(atlas_cell)
                            .to_array()
                    };
                    if w == 1 && h == 1 {
                        let positions: [Vec3; 4] = std::array::from_fn(|j| {
                            let mut position = Vec3::from(template[j].position) + origin;
                            if let Some([floor, ceiling]) = cell.span {
                                position.y = if template[j].position[1] > 0.0 {
                                    ceiling
                                } else {
                                    floor
                                };
                            }
                            position
                        });
                        let uvs = std::array::from_fn(|j| template[j].uv.map(|c| c * uv_tiles));
                        let colors = std::array::from_fn(|j| color(cell.ao[j], positions[j]));
                        let positions = positions.map(|p| p.to_array());
                        buffers.push_tiled_quad(
                            positions,
                            template[0].normal,
                            uvs,
                            colors,
                            cell.light,
                        );
                        continue;
                    }

                    let mut corner = origin;
                    corner[a_axis] -= 0.5;
                    corner[b_axis] -= 0.5;
                    corner[normal_axis] += template[0].position[normal_axis];
                    let (mut ta_step, mut tb_step) = (ta.as_vec3(), tb.as_vec3());
                    match cell.span {
                        Some([_, ceiling]) if normal.y > 0 => corner.y = ceiling,
                        Some([floor, ceiling]) => {
                            corner.y = floor;
                            // the side's one step up spans just its trimmed height
                            let up = if a_axis == 1 {
                                &mut ta_step
                            } else {
                                &mut tb_step
                            };
                            up.y *= ceiling - floor;
                        }
                        None => {}
                    }
                    let extent = [w as f32, h as f32];
                    let uv_at = |i: usize, j: usize| {
                        let along = [i as f32, j as f32];
                        [0, 1].map(|k| {
                            let t = (uv_axes[k] == b_axis) as usize;
                            (if uv_ascending[k] {
                                along[t]
                            } else {
                                extent[t] - along[t]
                            }) * uv_tiles
                        })
                    };
                    buffers.push_fan_quad(FanQuad {
                        corner,
                        ta: ta_step,
                        tb: tb_step,
                        size: (w, h),
                        normal: template[0].normal,
                        color: &color,
                        ao: by_corner(cell.ao),
                        light: by_corner(cell.light),
                        uv_at: &uv_at,
                        reversed: !ccw_in_ab,
                    });
                }
            }
        }
    }
}
