use bevy::math::Vec3;

use super::generator::{
    terrain_block, ChunkMeshes, MeshBuffers, SOLID_FLOOR, WATER_LEVEL, WATER_TOP,
};
use super::region_store::RegionStore;
use crate::block::BlockType;
use crate::chunks::chunk::CHUNK_SIZE;
use crate::chunks::lod::LodKey;
use crate::util::primitives::Vertex;

const CELLS: i32 = CHUNK_SIZE as i32;
// Corners sampled per side: every cell corner plus one extra ring for normals.
const CORNERS: i32 = CELLS + 3;
// How far below a node's edge its skirt hangs, in cells. Covers the gap where a
// neighbour at another LOD puts the surface at a slightly different height.
const SKIRT_CELLS: f32 = 2.0;

// The world height of the water surface, matching full-detail water.
fn water_surface_y() -> f32 {
    (WATER_LEVEL - 1) as f32 + WATER_TOP
}

// Blocks are centred on whole coordinates, so a column `height` blocks tall has its
// top face half a block below `height`.
fn surface_y(height: f32) -> f32 {
    height.max(SOLID_FLOOR as f32) - 0.5
}

/// The terrain surface across one LOD node's footprint: its height at every cell
/// corner, and the block on top of every cell the node draws.
pub(super) struct Surface {
    corners: Vec<f32>,
    blocks: Vec<BlockType>,
}

impl Surface {
    pub(super) fn sample(regions: &RegionStore, key: LodKey) -> Self {
        let cell = key.cell_size();
        let base = key.min_block();

        let mut corners = Vec::with_capacity((CORNERS * CORNERS) as usize);
        for i in -1..=CELLS + 1 {
            for j in -1..=CELLS + 1 {
                let height = regions.get_height(base.x + i as i64 * cell, base.z + j as i64 * cell);
                corners.push(surface_y(height as f32));
            }
        }
        let mut surface = Self {
            corners,
            blocks: vec![BlockType::Air; (CELLS * CELLS) as usize],
        };

        let range = NodeRange::new(key);
        for i in 0..CELLS {
            for j in 0..CELLS {
                if !range.contains(surface.cell_centre(i, j)) {
                    continue;
                }
                let world_x = base.x + i as i64 * cell + cell / 2;
                let world_z = base.z + j as i64 * cell + cell / 2;
                let height = regions.get_height(world_x, world_z).round() as i64;
                let (gradient_x, gradient_z) = regions.get_gradient(world_x, world_z);
                surface.blocks[(i * CELLS + j) as usize] = terrain_block(
                    height.max(SOLID_FLOOR) - 1,
                    height,
                    gradient_x.abs() + gradient_z.abs(),
                );
            }
        }
        surface
    }

    // `i` and `j` run from -1 to CELLS + 1
    fn corner(&self, i: i32, j: i32) -> f32 {
        self.corners[((i + 1) * CORNERS + j + 1) as usize]
    }

    fn cell_centre(&self, i: i32, j: i32) -> f32 {
        (self.corner(i, j)
            + self.corner(i + 1, j)
            + self.corner(i, j + 1)
            + self.corner(i + 1, j + 1))
            / 4.0
    }

    fn block(&self, i: i32, j: i32) -> BlockType {
        self.blocks[(i * CELLS + j) as usize]
    }
}

// The world heights a node spans. Nodes stacked on top of each other split the surface
// between them by which one each cell's centre falls in.
struct NodeRange {
    min: f32,
    max: f32,
}

impl NodeRange {
    fn new(key: LodKey) -> Self {
        let min = key.min_block().y as f32 - 0.5;
        Self {
            min,
            max: min + (CELLS as i64 * key.cell_size()) as f32,
        }
    }

    fn contains(&self, y: f32) -> bool {
        y >= self.min && y < self.max
    }
}

/// Meshes the part of `surface` that falls inside `key`'s node as a smooth heightfield,
/// in the node's cell coordinates like a blocky LOD mesh.
pub(super) fn mesh_surface(surface: &Surface, key: LodKey) -> ChunkMeshes {
    let cell = key.cell_size() as f32;
    let base = key.min_block().as_vec3();
    let range = NodeRange::new(key);
    // inverse of the node's transform, see `lod_transform`
    let origin = base + Vec3::splat(0.5 * cell - 0.5);
    let to_local = |world: Vec3| (world - origin) / cell;

    let corner = |i: i32, j: i32| {
        Vec3::new(
            base.x + i as f32 * cell - 0.5,
            surface.corner(i, j),
            base.z + j as f32 * cell - 0.5,
        )
    };
    let normal = |i: i32, j: i32| {
        let slope_x = (surface.corner(i + 1, j) - surface.corner(i - 1, j)) / (2.0 * cell);
        let slope_z = (surface.corner(i, j + 1) - surface.corner(i, j - 1)) / (2.0 * cell);
        Vec3::new(-slope_x, 1.0, -slope_z).normalize()
    };
    let drawn = |i: i32, j: i32| {
        (0..CELLS).contains(&i)
            && (0..CELLS).contains(&j)
            && range.contains(surface.cell_centre(i, j))
    };
    let water_y = water_surface_y();

    let mut opaque = MeshBuffers::default();
    let mut water = MeshBuffers::default();
    for i in 0..CELLS {
        for j in 0..CELLS {
            if drawn(i, j) {
                let uv_offset = surface.block(i, j) as usize as f32 - 1.0;
                let cells = [(i, j + 1), (i, j), (i + 1, j + 1), (i + 1, j)];
                push_quad(
                    &mut opaque,
                    cells.map(|(a, b)| to_local(corner(a, b))),
                    cells.map(|(a, b)| normal(a, b)),
                    Vec3::Y,
                    uv_offset,
                    [1.0; 4],
                );

                // a skirt down every edge whose neighbouring cell this node doesn't draw
                for (di, dj) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                    if drawn(i + di, j + dj) {
                        continue;
                    }
                    let edge = if di != 0 {
                        let x = if di < 0 { i } else { i + 1 };
                        [(x, j), (x, j + 1)]
                    } else {
                        let z = if dj < 0 { j } else { j + 1 };
                        [(i, z), (i + 1, z)]
                    };
                    let (top_a, top_b) =
                        (corner(edge[0].0, edge[0].1), corner(edge[1].0, edge[1].1));
                    let drop = Vec3::new(0.0, SKIRT_CELLS * cell, 0.0);
                    let outward = Vec3::new(di as f32, 0.0, dj as f32);
                    push_quad(
                        &mut opaque,
                        [top_a, top_a - drop, top_b, top_b - drop].map(to_local),
                        [outward; 4],
                        outward,
                        uv_offset,
                        [1.0; 4],
                    );
                }
            }

            let lowest = [(i, j), (i + 1, j), (i, j + 1), (i + 1, j + 1)]
                .map(|(a, b)| surface.corner(a, b))
                .into_iter()
                .fold(f32::INFINITY, f32::min);
            if range.contains(water_y) && lowest < water_y {
                let depth = (water_y - surface.cell_centre(i, j)).max(0.0);
                // R/G/B/A: shore, depth / CHUNK_SIZE, is-top, shore distance - see the
                // full-detail water mesh
                let color = [0.0, depth / CHUNK_SIZE as f32, 1.0, 1.0];
                let cells = [(i, j + 1), (i, j), (i + 1, j + 1), (i + 1, j)];
                push_quad(
                    &mut water,
                    cells.map(|(a, b)| to_local(corner(a, b).with_y(water_y))),
                    [Vec3::Y; 4],
                    Vec3::Y,
                    BlockType::Water as usize as f32 - 1.0,
                    color,
                );
            }
        }
    }

    ChunkMeshes {
        opaque: opaque.build_mesh(),
        water: (!water.is_empty()).then(|| water.build_mesh()),
    }
}

// Pushes a quad laid out like the cube's faces: triangles (0, 1, 2) and (2, 1, 3).
// Terrain is drawn with front faces culled, so a face points away from the side its
// corners wind anticlockwise on - the corners are swapped if needed to face `outward`.
fn push_quad(
    buffers: &mut MeshBuffers,
    mut corners: [Vec3; 4],
    mut normals: [Vec3; 4],
    outward: Vec3,
    uv_offset: f32,
    color: [f32; 4],
) {
    if (corners[1] - corners[0])
        .cross(corners[2] - corners[0])
        .dot(outward)
        > 0.0
    {
        corners.swap(1, 2);
        normals.swap(1, 2);
    }
    let vertices: Vec<Vertex> = corners
        .iter()
        .zip(normals)
        .map(|(position, normal)| Vertex {
            position: position.to_array(),
            normal: normal.to_array(),
            // the middle of the block's atlas tile: from this far away a block reads
            // as its overall colour, not its texture
            uv: [0.5, 0.5],
        })
        .collect();
    buffers.push_face(&vertices, Vec3::ZERO, uv_offset, &[color; 4]);
}

#[cfg(test)]
mod tests {
    use bevy::math::{I64Vec3, Vec3};
    use bevy::render::mesh::{Mesh, VertexAttributeValues};

    use super::{mesh_surface, surface_y, Surface, CELLS, CORNERS};
    use crate::block::BlockType;
    use crate::chunks::lod::LodKey;
    use crate::util::primitives::cube;

    fn flat(height: f32) -> Surface {
        Surface {
            corners: vec![surface_y(height); (CORNERS * CORNERS) as usize],
            blocks: vec![BlockType::Grass; (CELLS * CELLS) as usize],
        }
    }

    // a level-2 node, 64 blocks across and covering y 0..64
    fn node(y: i64) -> LodKey {
        LodKey {
            level: 2,
            coord: I64Vec3::new(3, y, -2),
        }
    }

    fn positions(mesh: &Mesh) -> Vec<Vec3> {
        match mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap() {
            VertexAttributeValues::Float32x3(v) => v.iter().map(|p| Vec3::from(*p)).collect(),
            _ => unreachable!(),
        }
    }

    // each triangle's winding normal, alongside the normal its first vertex was given
    fn triangle_normals(mesh: &Mesh) -> Vec<(Vec3, Vec3)> {
        let p = positions(mesh);
        let normals = match mesh.attribute(Mesh::ATTRIBUTE_NORMAL).unwrap() {
            VertexAttributeValues::Float32x3(v) => {
                v.iter().map(|n| Vec3::from(*n)).collect::<Vec<_>>()
            }
            _ => unreachable!(),
        };
        let indices: Vec<usize> = mesh.indices().unwrap().iter().collect();
        indices
            .chunks(3)
            .map(|t| ((p[t[1]] - p[t[0]]).cross(p[t[2]] - p[t[0]]), normals[t[0]]))
            .collect()
    }

    #[test]
    fn test_flat_surface_drawn_by_the_node_it_falls_in() {
        let surface = flat(50.0);
        let inside = mesh_surface(&surface, node(0));
        // a quad per cell, plus a skirt quad per cell edge around the border
        assert_eq!(
            (CELLS * CELLS + 4 * CELLS) as usize * 4,
            inside.opaque.count_vertices()
        );
        assert!(inside.water.is_none());

        let above = mesh_surface(&surface, node(1));
        assert_eq!(0, above.opaque.count_vertices());
    }

    #[test]
    fn test_surface_sits_on_top_of_full_detail_blocks() {
        let mesh = mesh_surface(&flat(50.0), node(0)).opaque;
        let key = node(0);
        let cell = key.cell_size() as f32;
        // back to world space through the node's transform
        let origin = key.min_block().as_vec3() + Vec3::splat(0.5 * cell - 0.5);
        let top = positions(&mesh)
            .iter()
            .map(|p| p.y * cell + origin.y)
            .fold(f32::MIN, f32::max);
        // a 50-tall column's top block is y = 49, whose top face is at 49.5
        assert_eq!(49.5, top);
    }

    #[test]
    fn test_quads_wind_like_cube_faces() {
        // the cube's top face, wound the way the terrain material expects
        let cube = cube();
        let (a, b, c) = (
            Vec3::from(cube[16].position),
            Vec3::from(cube[17].position),
            Vec3::from(cube[18].position),
        );
        let convention = (b - a).cross(c - a).dot(Vec3::Y).signum();

        // tops and skirts alike wind the same way relative to the way they face
        let mesh = mesh_surface(&flat(50.0), node(0)).opaque;
        for (winding, facing) in triangle_normals(&mesh) {
            assert_eq!(convention, winding.dot(facing).signum());
        }
    }

    #[test]
    fn test_water_drawn_over_low_ground() {
        let meshes = mesh_surface(&flat(20.0), node(0));
        let water = meshes.water.expect("water over ground below sea level");
        assert_eq!((CELLS * CELLS) as usize * 4, water.count_vertices());
    }
}
