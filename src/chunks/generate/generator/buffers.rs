use crate::block::Tile;
use crate::block::TILE_COUNT;
use crate::chunks::chunk::CHUNK_SIZE;
use crate::chunks::generate::biome::grass_tint;
use crate::chunks::generate::noise::NoiseGenerator;
use crate::util::primitives::Vertex;
use bevy::asset::RenderAssetUsages;
use bevy::math::Vec3;
use bevy::render::mesh::Indices;
use bevy::render::mesh::Mesh;
use bevy::render::mesh::VertexAttributeValues;

// Accumulates one mesh's worth of vertex/index/color data - one instance for the
// opaque blocks, a second for water, so they can become separate `Mesh`es with
// separate materials (water needs alpha blending, opaque blocks don't).
#[derive(Default)]
pub(super) struct MeshBuffers {
    pub(super) vertices: Vec<Vertex>,
    pub(super) indices: Vec<u32>,
    pub(super) colors: Vec<[f32; 4]>,
    // sky light per vertex, for block faces only, see `world.wesl`
    pub(super) lights: Vec<f32>,
    // whether faces draw their block's mean colour instead of its texture, see `world.wesl`
    pub(super) untextured: bool,
    // grass sides' vertices and their shade without the grass's tint, see `tint_grass_sides`
    pub(super) grass_sides: Vec<(usize, f32)>,
    // where tree boxes start and how they draw, which is on their own terms
    pub(super) boxes: Option<(usize, f32)>,
}

// How a face draws, carried in each vertex's second uv, see `world.wesl`: its own tile, its
// tile with leaves' holes filled, or its block's mean colour.
pub(super) const TEXTURED: f32 = 0.0;

pub(super) const TEXTURED_SOLID: f32 = 0.75;

pub(super) const UNTEXTURED: f32 = 1.0;

impl MeshBuffers {
    pub(super) fn push_face(
        &mut self,
        vs: &[Vertex],
        position: Vec3,
        uv_offset: f32,
        colors: &[[f32; 4]],
    ) {
        let uv_scale = 1.0 / TILE_COUNT as f32;

        let triangle_start: u32 = self.vertices.len() as u32;
        for (v, color) in vs.iter().zip(colors.iter()) {
            self.vertices.push(Vertex {
                position: (Vec3::from(v.position) + position).into(),
                normal: v.normal,
                uv: [uv_scale * (v.uv[0] + uv_offset), v.uv[1]],
            });
            self.colors.push(*color);
        }
        self.indices.extend(vec![
            triangle_start,
            triangle_start + 1,
            triangle_start + 2,
            triangle_start + 2,
            triangle_start + 1,
            triangle_start + 3,
        ]);
    }

    // `uvs` count tiles across the quad, and the colour alpha carries the atlas cell, see `world.wesl`.
    pub(super) fn push_tiled_quad(
        &mut self,
        positions: [[f32; 3]; 4],
        normal: [f32; 3],
        uvs: [[f32; 2]; 4],
        colors: [[f32; 4]; 4],
        lights: [f32; 4],
    ) {
        let triangle_start: u32 = self.vertices.len() as u32;
        for j in 0..4 {
            self.vertices.push(Vertex {
                position: positions[j],
                normal,
                uv: uvs[j],
            });
            self.colors.push(colors[j]);
            self.lights.push(lights[j]);
        }
        self.indices.extend([
            triangle_start,
            triangle_start + 1,
            triangle_start + 2,
            triangle_start + 2,
            triangle_start + 1,
            triangle_start + 3,
        ]);
    }

    // Vertices at every block step round the border, so neighbours share exact edge points and leave no cracks.
    pub(super) fn push_fan_quad(&mut self, quad: FanQuad) {
        let (w, h) = quad.size;
        let mut ring = Vec::with_capacity(2 * (w + h));
        ring.extend((0..w).map(|i| (i, 0)));
        ring.extend((0..h).map(|j| (w, j)));
        ring.extend((1..=w).rev().map(|i| (i, h)));
        ring.extend((1..=h).rev().map(|j| (0, j)));

        let blend = |corners: [[f32; 2]; 2], u: f32, v: f32| {
            let along = |c: [f32; 2]| c[0] + (c[1] - c[0]) * v;
            along(corners[0]) + (along(corners[1]) - along(corners[0])) * u
        };
        let mut push = |i: f32, j: f32, uv: [f32; 2]| {
            let position = quad.corner + quad.ta * i + quad.tb * j;
            self.vertices.push(Vertex {
                position: position.into(),
                normal: quad.normal,
                uv,
            });
            let (u, v) = (i / w as f32, j / h as f32);
            self.colors
                .push((quad.color)(blend(quad.ao, u, v), position));
            self.lights.push(blend(quad.light, u, v));
            self.vertices.len() as u32 - 1
        };
        let (uv_min, uv_max) = ((quad.uv_at)(0, 0), (quad.uv_at)(w, h));
        let centre = push(
            w as f32 / 2.0,
            h as f32 / 2.0,
            [0, 1].map(|k| (uv_min[k] + uv_max[k]) / 2.0),
        );
        let first = centre + 1;
        for &(i, j) in &ring {
            push(i as f32, j as f32, (quad.uv_at)(i, j));
        }
        for k in 0..ring.len() as u32 {
            let (from, to) = (first + k, first + (k + 1) % ring.len() as u32);
            if quad.reversed {
                self.indices.extend([centre, to, from]);
            } else {
                self.indices.extend([centre, from, to]);
            }
        }
    }

    // Grass sides carry the grass's tint, which the dirt in their tile mustn't take, so their shade
    // without it goes in place of how they draw. A block a pixel or so tall shows its side's tile
    // averaged, mostly dirt, so far off they turn to the tinted grass top, see `world.wesl`.
    pub(super) fn tint_grass_sides(&mut self, grass_tint: &GrassTint) {
        let side = Tile::GrassSide.atlas_cell();
        for (i, (vertex, color)) in self.vertices.iter().zip(&mut self.colors).enumerate() {
            if color[3] == side {
                let shade = color[0];
                let tinted = grass_tint.at(Vec3::from(vertex.position)) * shade;
                color[..3].copy_from_slice(&tinted.to_array());
                self.grass_sides.push((i, shade));
            }
        }
    }

    pub(super) fn build_mesh(self) -> Mesh {
        let mut mesh = Mesh::new(
            bevy::render::mesh::PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
        );
        mesh.insert_indices(Indices::U32(self.indices));
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_POSITION,
            VertexAttributeValues::Float32x3(self.vertices.iter().map(|v| v.position).collect()),
        );
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_NORMAL,
            VertexAttributeValues::Float32x3(self.vertices.iter().map(|v| v.normal).collect()),
        );
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_UV_0,
            VertexAttributeValues::Float32x2(self.vertices.iter().map(|v| v.uv).collect()),
        );
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_COLOR,
            VertexAttributeValues::Float32x4(self.colors),
        );
        if !self.lights.is_empty() {
            assert_eq!(
                self.lights.len(),
                self.vertices.len(),
                "only some vertices carry light"
            );
            let mut lights: Vec<[f32; 2]> = self
                .lights
                .into_iter()
                .enumerate()
                .map(|(i, l)| match self.boxes {
                    Some((start, flag)) if i >= start => [l, flag],
                    _ => [
                        l,
                        if self.untextured {
                            UNTEXTURED
                        } else {
                            TEXTURED
                        },
                    ],
                })
                .collect();
            for (i, shade) in self.grass_sides {
                lights[i][1] = -shade;
            }
            mesh.insert_attribute(
                Mesh::ATTRIBUTE_UV_1,
                VertexAttributeValues::Float32x2(lights),
            );
        }
        mesh
    }
}

pub(super) struct FanQuad<'a> {
    pub(super) corner: Vec3,
    pub(super) ta: Vec3,
    pub(super) tb: Vec3,
    pub(super) size: (usize, usize),
    pub(super) normal: [f32; 3],
    // the colour at a position, given its ambient occlusion
    pub(super) color: &'a dyn Fn(f32, Vec3) -> [f32; 4],
    // ambient occlusion and sky light at the quad's corners, [along ta][along tb], blended across it
    pub(super) ao: [[f32; 2]; 2],
    pub(super) light: [[f32; 2]; 2],
    pub(super) uv_at: &'a dyn Fn(usize, usize) -> [f32; 2],
    pub(super) reversed: bool,
}

// `solid_leaves` draws leaves without their holes, so they hide the leaves behind them; `textured`
// is whether faces draw their own tile, not the one their block's colour is read from.
#[derive(Clone, Copy)]
pub(super) struct MeshStyle {
    pub(super) ambient_occlusion: bool,
    pub(super) textured: bool,
    pub(super) solid_leaves: bool,
}

pub struct ChunkMeshes {
    pub opaque: Mesh,
    pub water: Option<Mesh>,
}

impl ChunkMeshes {
    /// Whether there's anything to draw: buried chunks mesh to nothing.
    pub fn has_geometry(&self) -> bool {
        self.opaque.count_vertices() > 0 || self.water.is_some()
    }
}

// Tints grass by the climate it grows in. Climate drifts over thousands of blocks, so it's
// sampled at the mesh's four corners and blended across it, rather than at every vertex.
pub(super) struct GrassTint {
    // at the mesh's edges, so neighbouring meshes share their tints along each edge: (-x, -z), (+x, -z), (-x, +z), (+x, +z)
    pub(super) corners: [Vec3; 4],
}

impl GrassTint {
    // `origin` is where the mesh's local origin is in the world, and `scale` how many blocks one local unit is.
    pub(super) fn new(noise: &NoiseGenerator, origin: Vec3, scale: f32) -> Self {
        let size = CHUNK_SIZE as f32;
        let corners = [(0.0, 0.0), (size, 0.0), (0.0, size), (size, size)].map(|(x, z)| {
            let world = origin + Vec3::new(x - 0.5, 0.0, z - 0.5) * scale;
            grass_tint(noise.climate(world.x as f64, world.z as f64))
        });
        Self { corners }
    }

    pub(super) fn at(&self, local: Vec3) -> Vec3 {
        let size = CHUNK_SIZE as f32;
        let (u, v) = ((local.x + 0.5) / size, (local.z + 0.5) / size);
        let [a, b, c, d] = self.corners;
        a.lerp(b, u).lerp(c.lerp(d, u), v)
    }
}

// Cells wider than this draw their block's mean colour instead of its texture, which tiles a
// cell's width of times over a face, since one stretched over a coarse cell would read as a
// giant block. Past a few blocks a tile is under a pixel and mipmaps down to the mean anyway.
pub(super) const TEXTURE_MAX_CELL_SIZE: i64 = 4;

// Tree boxes keep their leaf texture's holes up to cells this wide. Past it boxes are mostly
// canopy, too wide to see through, and keep the texture with its holes filled.
pub(super) const BOX_TEXTURE_MAX_CELL: i64 = 8;

// A quad's `uvs` scaled to count blocks across it, so a texture isn't stretched over a whole box.
pub(super) fn tiled_uvs(
    positions: &[[f32; 3]; 4],
    uvs: &[[f32; 2]; 4],
    cell: f32,
) -> [[f32; 2]; 4] {
    let length = |axis: usize| {
        let other = |j: usize, k: usize| {
            uvs[j][1 - axis] == uvs[k][1 - axis] && uvs[j][axis] != uvs[k][axis]
        };
        let (j, k) = (0..4)
            .flat_map(|j| (0..4).map(move |k| (j, k)))
            .find(|&(j, k)| other(j, k))
            .unwrap_or((0, 0));
        (Vec3::from(positions[j]) - Vec3::from(positions[k])).length() * cell
    };
    let blocks = [length(0), length(1)];
    std::array::from_fn(|j| [uvs[j][0] * blocks[0], uvs[j][1] * blocks[1]])
}
