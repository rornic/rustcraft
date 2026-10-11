use bevy::math::Vec3;

use super::biome::{grass_colour, leaf_colour, Climate};
use super::generator::{column_block, ChunkMeshes, MeshBuffers, SOLID_FLOOR, WATER_TOP};
use super::region_store::RegionStore;
use super::WATER_LEVEL;
use crate::block::BlockType;
use crate::chunks::chunk::CHUNK_SIZE;
use crate::chunks::lod::LodKey;
use crate::util::primitives::Vertex;

const CELLS: i32 = CHUNK_SIZE as i32;
// Corners sampled per side: every cell corner plus one extra ring for normals.
const CORNERS: i32 = CELLS + 3;
const COLOUR_CELLS: i32 = CELLS + 2;
const SKIRT_CELLS: f32 = 1.0;
const SLOPE_SHADE: f32 = 0.7;
const UNTEXTURED_UV: [f32; 2] = [-1.0, 0.0];
// How much of the ground a given tree density hides under leaves, seen from afar.
const CANOPY_COVER: f64 = 1.6;

// Matches the mean of each block's texture, so far terrain is the colour of near terrain.
fn block_colour(block: BlockType, climate: Climate) -> Vec3 {
    match block {
        BlockType::Stone => Vec3::new(0.118, 0.118, 0.118),
        BlockType::Grass => grass_colour(climate),
        BlockType::Sand => Vec3::new(0.478, 0.363, 0.183),
        BlockType::Water => Vec3::new(0.066, 0.101, 0.915),
        BlockType::Snow => Vec3::new(0.945, 0.992, 0.992),
        BlockType::Log => Vec3::new(0.08, 0.045, 0.019),
        BlockType::Leaves => leaf_colour(climate),
        BlockType::Air => Vec3::ZERO,
    }
}

// The world height of the water surface, matching full-detail water.
fn water_surface_y() -> f32 {
    (WATER_LEVEL - 1) as f32 + WATER_TOP
}

fn surface_y(height: f32) -> f32 {
    height.max(SOLID_FLOOR as f32) - 0.5
}

pub(super) struct Surface {
    corners: Vec<f32>,
    // the colour of the surface block in each cell
    colours: Vec<Vec3>,
}

impl Surface {
    pub(super) fn sample(regions: &RegionStore, key: LodKey) -> Self {
        let cell = key.cell_size();
        let base = key.min_block();
        let mut heights = regions.sampler();

        let mut corners = Vec::with_capacity((CORNERS * CORNERS) as usize);
        for i in -1..=CELLS + 1 {
            for j in -1..=CELLS + 1 {
                let height = heights.height(base.x + i as i64 * cell, base.z + j as i64 * cell);
                corners.push(surface_y(height as f32));
            }
        }

        let noise = regions.noise();
        let mut colours = Vec::with_capacity((COLOUR_CELLS * COLOUR_CELLS) as usize);
        for i in -1..=CELLS {
            for j in -1..=CELLS {
                let world_x = base.x + i as i64 * cell + cell / 2;
                let world_z = base.z + j as i64 * cell + cell / 2;
                let height = heights.height(world_x, world_z).round() as i64;
                let (gradient_x, gradient_z) = heights.gradient(world_x, world_z);
                let block = column_block(
                    noise.biomes(),
                    heights.biome(world_x, world_z),
                    height.max(SOLID_FLOOR) - 1,
                    height,
                    gradient_x.abs() + gradient_z.abs(),
                );
                let climate = noise.climate(world_x as f64, world_z as f64);
                let mut colour = block_colour(block, climate);
                if block == BlockType::Grass {
                    // trees are too small to draw this far out, so forests show as their leaves
                    let canopy = (heights.trees(world_x, world_z) * CANOPY_COVER).min(1.0) as f32;
                    colour = colour.lerp(block_colour(BlockType::Leaves, climate), canopy);
                }
                colours.push(colour);
            }
        }

        Self { corners, colours }
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

    // `i` and `j` run from -1 to CELLS
    fn cell_colour(&self, i: i32, j: i32) -> Vec3 {
        self.colours[((i + 1) * COLOUR_CELLS + j + 1) as usize]
    }

    fn corner_colour(&self, i: i32, j: i32) -> Vec3 {
        (self.cell_colour(i - 1, j - 1)
            + self.cell_colour(i, j - 1)
            + self.cell_colour(i - 1, j)
            + self.cell_colour(i, j))
            / 4.0
    }
}

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
    let colour = |i: i32, j: i32| {
        let shade = 1.0 - SLOPE_SHADE * (1.0 - normal(i, j).y);
        (surface.corner_colour(i, j) * shade).extend(1.0).to_array()
    };
    let in_footprint = |i: i32, j: i32| (0..CELLS).contains(&i) && (0..CELLS).contains(&j);
    let water_y = water_surface_y();

    let mut opaque = MeshBuffers::default();
    let mut water = MeshBuffers::default();
    for i in 0..CELLS {
        for j in 0..CELLS {
            if range.contains(surface.cell_centre(i, j)) {
                let cells = [(i, j + 1), (i, j), (i + 1, j + 1), (i + 1, j)];
                push_quad(
                    &mut opaque,
                    cells.map(|(a, b)| to_local(corner(a, b))),
                    cells.map(|(a, b)| normal(a, b)),
                    Vec3::Y,
                    (UNTEXTURED_UV, 0.0),
                    cells.map(|(a, b)| colour(a, b)),
                );

                for (di, dj) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                    if in_footprint(i + di, j + dj) {
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
                    let (colour_a, colour_b) =
                        (colour(edge[0].0, edge[0].1), colour(edge[1].0, edge[1].1));
                    let drop = Vec3::new(0.0, SKIRT_CELLS * cell, 0.0);
                    let outward = Vec3::new(di as f32, 0.0, dj as f32);
                    push_quad(
                        &mut opaque,
                        [top_a, top_a - drop, top_b, top_b - drop].map(to_local),
                        [outward; 4],
                        outward,
                        (UNTEXTURED_UV, 0.0),
                        [colour_a, colour_a, colour_b, colour_b],
                    );
                }
            }

            let lowest = [(i, j), (i + 1, j), (i, j + 1), (i + 1, j + 1)]
                .map(|(a, b)| surface.corner(a, b))
                .into_iter()
                .fold(f32::INFINITY, f32::min);
            if range.contains(water_y) && lowest < water_y {
                let cells = [(i, j + 1), (i, j), (i + 1, j + 1), (i + 1, j)];
                // depth per corner, so it blends smoothly across the water
                let colours = cells.map(|(a, b)| {
                    let depth = (water_y - surface.corner(a, b)).max(0.0);
                    [0.0, depth / CHUNK_SIZE as f32, 1.0, 1.0]
                });
                push_quad(
                    &mut water,
                    cells.map(|(a, b)| to_local(corner(a, b).with_y(water_y))),
                    [Vec3::Y; 4],
                    Vec3::Y,
                    // the middle of the water tile
                    ([0.5, 0.5], BlockType::Water as usize as f32 - 1.0),
                    colours,
                );
            }
        }
    }

    ChunkMeshes {
        opaque: opaque.build_mesh(),
        water: (!water.is_empty()).then(|| water.build_mesh()),
    }
}

fn push_quad(
    buffers: &mut MeshBuffers,
    mut corners: [Vec3; 4],
    mut normals: [Vec3; 4],
    outward: Vec3,
    (uv, uv_offset): ([f32; 2], f32),
    mut colours: [[f32; 4]; 4],
) {
    if (corners[1] - corners[0])
        .cross(corners[2] - corners[0])
        .dot(outward)
        > 0.0
    {
        corners.swap(1, 2);
        normals.swap(1, 2);
        colours.swap(1, 2);
    }
    let vertices: Vec<Vertex> = corners
        .iter()
        .zip(normals)
        .map(|(position, normal)| Vertex {
            position: position.to_array(),
            normal: normal.to_array(),
            uv,
        })
        .collect();
    buffers.push_face(&vertices, Vec3::ZERO, uv_offset, &colours);
}

#[cfg(test)]
mod tests {
    use bevy::math::{I64Vec3, Vec3};
    use bevy::render::mesh::{Mesh, VertexAttributeValues};

    use super::{
        block_colour, mesh_surface, surface_y, Surface, CELLS, CHUNK_SIZE, COLOUR_CELLS, CORNERS,
        WATER_LEVEL,
    };
    use crate::block::BlockType;
    use crate::chunks::generate::biome::Climate;
    use crate::chunks::lod::LodKey;
    use crate::util::primitives::cube;

    fn grass() -> Vec3 {
        block_colour(BlockType::Grass, Climate::default())
    }

    fn flat(height: f32) -> Surface {
        Surface {
            corners: vec![surface_y(height); (CORNERS * CORNERS) as usize],
            colours: vec![grass(); (COLOUR_CELLS * COLOUR_CELLS) as usize],
        }
    }

    // Ground at y 40 for x cells below 8, and a cliff up to y 90 above it.
    fn cliff() -> Surface {
        let mut corners = Vec::new();
        for i in -1..=CELLS + 1 {
            for _ in -1..=CELLS + 1 {
                corners.push(if i <= 8 { 40.0 } else { 90.0 });
            }
        }
        Surface {
            corners,
            colours: vec![grass(); (COLOUR_CELLS * COLOUR_CELLS) as usize],
        }
    }

    fn colours(mesh: &Mesh) -> Vec<[f32; 4]> {
        match mesh.attribute(Mesh::ATTRIBUTE_COLOR).unwrap() {
            VertexAttributeValues::Float32x4(v) => v.clone(),
            _ => unreachable!(),
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
        // above sea level, so the node draws no water
        let height = WATER_LEVEL + 16;
        let surface = flat(height as f32);
        let level_node = |y| height.div_euclid(node(0).width() * CHUNK_SIZE as i64) + y;
        let inside = mesh_surface(&surface, node(level_node(0)));
        // a quad per cell, plus a skirt quad per cell edge around the border
        assert_eq!(
            (CELLS * CELLS + 4 * CELLS) as usize * 4,
            inside.opaque.count_vertices()
        );
        assert!(inside.water.is_none());

        let above = mesh_surface(&surface, node(level_node(1)));
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
    fn test_no_skirt_where_a_stacked_node_takes_over() {
        // node(0) draws the 8 columns of low ground, node(1) the cliff top above it
        let low = mesh_surface(&cliff(), node(0)).opaque;
        let owned = 8 * CELLS;
        let skirts = CELLS + 2 * 8;
        assert_eq!((owned + skirts) as usize * 4, low.count_vertices());
    }

    #[test]
    fn test_colours_blend_between_cells() {
        let mut surface = flat(50.0);
        let snow = block_colour(BlockType::Snow, Climate::default());
        // snow on every cell with i >= 8
        for i in -1..=CELLS {
            for j in -1..=CELLS {
                if i >= 8 {
                    surface.colours[((i + 1) * COLOUR_CELLS + j + 1) as usize] = snow;
                }
            }
        }
        let expected = (grass() + snow) / 2.0;
        // corners on the boundary between the two halves are half grass, half snow
        assert_eq!(expected, surface.corner_colour(8, 5));
        assert_eq!(grass(), surface.corner_colour(7, 5));
    }

    #[test]
    fn test_steeper_ground_is_darker() {
        let flat_colour = colours(&mesh_surface(&flat(50.0), node(0)).opaque)[0];
        let mesh = mesh_surface(&cliff(), node(0)).opaque;
        let darkest = colours(&mesh).iter().map(|c| c[1]).fold(f32::MAX, f32::min);
        assert!(darkest < flat_colour[1]);
    }

    #[test]
    fn test_terrain_is_untextured() {
        let mesh = mesh_surface(&flat(50.0), node(0)).opaque;
        match mesh.attribute(Mesh::ATTRIBUTE_UV_0).unwrap() {
            VertexAttributeValues::Float32x2(uvs) => assert!(uvs.iter().all(|uv| uv[0] < 0.0)),
            _ => unreachable!(),
        }
    }

    #[test]
    fn test_water_drawn_over_low_ground() {
        // the node the sea's surface falls in
        let y = (WATER_LEVEL - 1).div_euclid(node(0).width() * CHUNK_SIZE as i64);
        let meshes = mesh_surface(&flat((WATER_LEVEL - 20) as f32), node(y));
        let water = meshes.water.expect("water over ground below sea level");
        assert_eq!((CELLS * CELLS) as usize * 4, water.count_vertices());
    }
}
