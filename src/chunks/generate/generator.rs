use std::{collections::HashMap, sync::Arc};

use bevy::{
    math::{IVec3, U16Vec3, Vec3},
    render::{
        mesh::{Indices, Mesh, VertexAttributeValues},
        render_asset::RenderAssetUsages,
    },
};

use super::region_store::RegionStore;
use super::smooth::{mesh_surface, Surface};
use crate::block::{BlockType, BLOCK_COUNT};
use crate::chunks::chunk::{neighbor_26_index, ChunkCoordinate, ChunkData, CHUNK_SIZE};
use crate::chunks::lod::LodKey;
use crate::util::primitives::Vertex;

// Face order matches `face_vertices`/`sides` below: [front, right, left, back, top, bottom].
const FACE_NORMAL_OFFSET: [IVec3; 6] = [
    IVec3::new(0, 0, -1), // front
    IVec3::new(1, 0, 0),  // right
    IVec3::new(-1, 0, 0), // left
    IVec3::new(0, 0, 1),  // back
    IVec3::new(0, 1, 0),  // top
    IVec3::new(0, -1, 0), // bottom
];
const FACE_TANGENTS: [(IVec3, IVec3); 6] = [
    (IVec3::new(1, 0, 0), IVec3::new(0, 1, 0)), // front
    (IVec3::new(0, 0, 1), IVec3::new(0, 1, 0)), // right
    (IVec3::new(0, 0, 1), IVec3::new(0, 1, 0)), // left
    (IVec3::new(1, 0, 0), IVec3::new(0, 1, 0)), // back
    (IVec3::new(1, 0, 0), IVec3::new(0, 0, 1)), // top
    (IVec3::new(1, 0, 0), IVec3::new(0, 0, 1)), // bottom
];
const AO_STRENGTH: f32 = 0.5;

pub(super) const SOLID_FLOOR: i64 = 16;
pub(super) const WATER_LEVEL: i64 = 32;
// Height of the water surface within a full-detail block, whose cube spans -0.5..0.5.
pub(super) const WATER_TOP: f32 = 0.4;

trait BlockSource {
    fn block(&self, coord: IVec3) -> BlockType;
    fn non_air(&self) -> Vec<(IVec3, BlockType)>;
}

struct ChunkNeighbourhood<'a> {
    chunk: &'a ChunkData,
    neighbors: &'a [Option<Arc<ChunkData>>],
}

impl BlockSource for ChunkNeighbourhood<'_> {
    fn block(&self, coord: IVec3) -> BlockType {
        resolve_block(self.chunk, self.neighbors, coord)
    }

    fn non_air(&self) -> Vec<(IVec3, BlockType)> {
        self.chunk
            .iter_non_air()
            .map(|(c, block)| (c.as_ivec3(), block))
            .collect()
    }
}

const SKIRT_CELLS: i32 = 2;

struct LodBlocks {
    size: i32,
    cells: Vec<BlockType>,
}

impl LodBlocks {
    fn sample(regions: &RegionStore, key: LodKey) -> Self {
        let size = CHUNK_SIZE as i32;
        let padded = (size + 2) as usize;
        let cell = key.cell_size();
        let base = key.min_block();
        let mut cells = vec![BlockType::Air; padded * padded * padded];
        let mut heights = regions.sampler();

        for cx in -1..=size {
            for cz in -1..=size {
                let world_x = base.x + cx as i64 * cell + cell / 2;
                let world_z = base.z + cz as i64 * cell + cell / 2;
                let column_height = heights.height(world_x, world_z).round() as i64;
                let (gradient_x, gradient_z) = heights.gradient(world_x, world_z);
                let combined_gradient = gradient_x.abs() + gradient_z.abs();

                for cy in -1..=size {
                    let world_y = base.y + cy as i64 * cell + cell / 2;
                    cells[Self::index(size, IVec3::new(cx, cy, cz))] =
                        terrain_block(world_y, column_height, combined_gradient);
                }
            }
        }

        Self { size, cells }
    }

    fn index(size: i32, coord: IVec3) -> usize {
        let padded = (size + 2) as usize;
        let c = (coord + IVec3::ONE).as_uvec3();
        (c.x as usize * padded + c.z as usize) * padded + c.y as usize
    }

    fn raw(&self, coord: IVec3) -> BlockType {
        let coord = coord.clamp(IVec3::splat(-1), IVec3::splat(self.size));
        self.cells[Self::index(self.size, coord)]
    }
}

impl BlockSource for LodBlocks {
    fn block(&self, coord: IVec3) -> BlockType {
        let coord = coord.clamp(IVec3::splat(-1), IVec3::splat(self.size));
        let block = self.raw(coord);
        let outside = |c: i32| c < 0 || c >= self.size;
        if (outside(coord.x) || outside(coord.z)) && is_solid(block) {
            for dy in 1..=SKIRT_CELLS {
                let above = self.raw(coord + IVec3::new(0, dy, 0));
                if !is_solid(above) {
                    return above;
                }
            }
        }
        block
    }

    fn non_air(&self) -> Vec<(IVec3, BlockType)> {
        let mut result = Vec::new();
        for x in 0..self.size {
            for z in 0..self.size {
                for y in 0..self.size {
                    let coord = IVec3::new(x, y, z);
                    let block = self.raw(coord);
                    if block != BlockType::Air {
                        result.push((coord, block));
                    }
                }
            }
        }
        result
    }
}

fn is_solid(block: BlockType) -> bool {
    block.occlusion_weight() == 1.0
}

pub(super) fn terrain_block(world_y: i64, column_height: i64, combined_gradient: f64) -> BlockType {
    if world_y < column_height.max(SOLID_FLOOR) {
        if world_y >= 90 && combined_gradient <= 2.0 {
            BlockType::Snow
        } else if world_y >= 70 && combined_gradient >= 2.0
            || (world_y >= 36 && combined_gradient >= 3.5)
        {
            BlockType::Stone
        } else if world_y >= 36 {
            BlockType::Grass
        } else {
            BlockType::Sand
        }
    } else if world_y < WATER_LEVEL {
        BlockType::Water
    } else {
        BlockType::Air
    }
}

// `neighbor_chunks` holds all 26 chunks in the 3x3x3 neighborhood (see `ChunkCoordinate::neighbors_26`),
// since AO sampling near a chunk corner can need a diagonal-adjacent chunk, not just a face-adjacent one.
fn resolve_block(chunk: &ChunkData, neighbor_chunks: &[Option<Arc<ChunkData>>], coord: IVec3) -> BlockType {
    let size = chunk.size as i32;
    let out = |c: i32| if c < 0 { -1 } else if c >= size { 1 } else { 0 };
    let wrap = |c: i32, out: i32| match out {
        -1 => size - 1,
        1 => 0,
        _ => c,
    };
    let (out_x, out_y, out_z) = (out(coord.x), out(coord.y), out(coord.z));

    if out_x == 0 && out_y == 0 && out_z == 0 {
        return chunk.get_block_at(U16Vec3::new(coord.x as u16, coord.y as u16, coord.z as u16));
    }

    let wrapped = IVec3::new(
        wrap(coord.x, out_x),
        wrap(coord.y, out_y),
        wrap(coord.z, out_z),
    );
    let idx = neighbor_26_index(out_x, out_y, out_z);
    neighbor_chunks[idx]
        .as_ref()
        .map(|c| c.get_block_at(U16Vec3::new(wrapped.x as u16, wrapped.y as u16, wrapped.z as u16)))
        .unwrap_or_default()
}

// The two tangent-direction offsets (toward whichever corner `vertex_local_pos` is
// on) for a face - shared by `vertex_ao` (which samples one layer along the face
// normal, for overhang darkening) and `vertex_shore_factor` (which samples at the
// block's own height, since shoreline land sits flush with the water surface, not
// above it).
fn tangent_offsets(face_index: usize, vertex_local_pos: [f32; 3]) -> (IVec3, IVec3) {
    let (ta, tb) = FACE_TANGENTS[face_index];
    let axis_index = |v: IVec3| if v.x != 0 { 0 } else if v.y != 0 { 1 } else { 2 };
    let sign_a = vertex_local_pos[axis_index(ta)].signum() as i32;
    let sign_b = vertex_local_pos[axis_index(tb)].signum() as i32;
    (ta * sign_a, tb * sign_b)
}

fn vertex_ao(
    source: &impl BlockSource,
    block_coord: IVec3,
    face_index: usize,
    vertex_local_pos: [f32; 3],
) -> f32 {
    let (offset_a, offset_b) = tangent_offsets(face_index, vertex_local_pos);
    let normal_off = FACE_NORMAL_OFFSET[face_index];

    let side_a = source.block(block_coord + normal_off + offset_a);
    let side_b = source.block(block_coord + normal_off + offset_b);
    let corner = source.block(block_coord + normal_off + offset_a + offset_b);

    let sum = side_a.occlusion_weight() + side_b.occlusion_weight() + corner.occlusion_weight();
    1.0 - (sum / 3.0).clamp(0.0, 1.0) * AO_STRENGTH
}

// Fraction of a water top-face vertex's same-height tangent corners that are solid
// land - used to bake a shoreline mask into the water mesh so the shader can lap
// foam there.
fn vertex_shore_factor(
    source: &impl BlockSource,
    block_coord: IVec3,
    face_index: usize,
    vertex_local_pos: [f32; 3],
) -> f32 {
    let (offset_a, offset_b) = tangent_offsets(face_index, vertex_local_pos);

    let side_a = source.block(block_coord + offset_a);
    let side_b = source.block(block_coord + offset_b);
    let corner = source.block(block_coord + offset_a + offset_b);

    let land_count = [side_a, side_b, corner]
        .iter()
        .filter(|b| b.occlusion_weight() == 1.0)
        .count();
    land_count as f32 / 3.0
}

const SHORE_DISTANCE_RADIUS: i32 = 3;

// Distance (in blocks, normalized to 0..1 over `SHORE_DISTANCE_RADIUS`, 1.0 meaning
// "no land within range") from this top-face vertex to the nearest land block -
// anchored at the *shared grid corner* (block_coord + tangent offsets) rather than
// the block itself, so up to four blocks touching that corner all compute the exact
// same distance there. That's what keeps it safe to drive the foam's travelling
// phase: unlike per-block data, it can't disagree at a shared edge.
fn vertex_shore_distance(
    source: &impl BlockSource,
    block_coord: IVec3,
    face_index: usize,
    vertex_local_pos: [f32; 3],
) -> f32 {
    let (offset_a, offset_b) = tangent_offsets(face_index, vertex_local_pos);
    let anchor = block_coord + offset_a + offset_b;

    let mut min_dist_sq = i32::MAX;
    for dz in -SHORE_DISTANCE_RADIUS..=SHORE_DISTANCE_RADIUS {
        for dx in -SHORE_DISTANCE_RADIUS..=SHORE_DISTANCE_RADIUS {
            let dist_sq = dx * dx + dz * dz;
            if dist_sq > SHORE_DISTANCE_RADIUS * SHORE_DISTANCE_RADIUS || dist_sq >= min_dist_sq {
                continue;
            }
            let probe = anchor + IVec3::new(dx, 0, dz);
            if source.block(probe).occlusion_weight() == 1.0 {
                min_dist_sq = dist_sq;
            }
        }
    }

    if min_dist_sq == i32::MAX {
        1.0
    } else {
        (min_dist_sq as f32).sqrt() / SHORE_DISTANCE_RADIUS as f32
    }
}

pub fn generate_chunk(
    regions: Arc<RegionStore>,
    chunk_pos: ChunkCoordinate,
) -> ChunkData {
    let mut chunk_data = ChunkData::default();
    let mut heights = regions.sampler();

    for x in 0..chunk_data.size {
        for z in 0..chunk_data.size {
            let (world_x, world_y, world_z) = (
                chunk_pos.0.x * chunk_data.size as i64 + x as i64,
                chunk_pos.0.y * chunk_data.size as i64,
                chunk_pos.0.z * chunk_data.size as i64 + z as i64,
            );

            let column_height = heights.height(world_x, world_z).round() as i64;
            let (gradient_x, gradient_z) = heights.gradient(world_x, world_z);
            let combined_gradient = gradient_x.abs() + gradient_z.abs();

            for y in 0..chunk_data.size {
                let block = terrain_block(world_y + y as i64, column_height, combined_gradient);
                if block != BlockType::Air {
                    chunk_data.set_block_at(U16Vec3::new(x, y, z), block);
                }
            }
        }
    }

    chunk_data
}

// Accumulates one mesh's worth of vertex/index/color data - one instance for the
// opaque blocks, a second for water, so they can become separate `Mesh`es with
// separate materials (water needs alpha blending, opaque blocks don't).
#[derive(Default)]
pub(super) struct MeshBuffers {
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
    colors: Vec<[f32; 4]>,
}

impl MeshBuffers {
    pub(super) fn push_face(&mut self, vs: &[Vertex], position: Vec3, uv_offset: f32, colors: &[[f32; 4]]) {
        let uv_scale = 1.0 / (BLOCK_COUNT - 1) as f32;

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

    pub(super) fn is_empty(&self) -> bool {
        self.vertices.is_empty()
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
        mesh
    }
}

pub struct ChunkMeshes {
    pub opaque: Mesh,
    pub water: Option<Mesh>,
}

// Counts contiguous Water blocks straight down from `block_coord`, within this
// chunk only (so it's a cheap, purely-local lookup the async mesh task can do
// without `World` access - see the plan's note on why depth doesn't cross chunk
// boundaries). Used to scale wave amplitude: open water ripples, a shallow puddle
// at a chunk's edge stays calm.
fn water_depth(source: &impl BlockSource, block_coord: IVec3) -> u32 {
    let mut depth = 0u32;
    let mut y = block_coord.y;
    while y >= 0 {
        if source.block(IVec3::new(block_coord.x, y, block_coord.z)) != BlockType::Water {
            break;
        }
        depth += 1;
        y -= 1;
    }
    depth
}

// Water depth at a top-face corner, averaged over the four columns sharing it (land counts as 0).
fn vertex_water_depth(
    depth_of: &mut impl FnMut(IVec3) -> u32,
    block_coord: IVec3,
    face_index: usize,
    vertex_local_pos: [f32; 3],
) -> f32 {
    let (offset_a, offset_b) = tangent_offsets(face_index, vertex_local_pos);
    let total: u32 = [IVec3::ZERO, offset_a, offset_b, offset_a + offset_b]
        .into_iter()
        .map(|offset| depth_of(block_coord + offset))
        .sum();
    total as f32 / 4.0
}

pub fn generate_chunk_mesh(
    chunk: Arc<ChunkData>,
    adjacent_chunks: Vec<Option<Arc<ChunkData>>>,
) -> ChunkMeshes {
    let source = ChunkNeighbourhood {
        chunk: &chunk,
        neighbors: &adjacent_chunks,
    };
    mesh_blocks(&source, 1)
}

pub fn generate_lod_mesh(
    regions: Arc<RegionStore>,
    key: LodKey,
    smooth_level: u8,
) -> Option<ChunkMeshes> {
    let meshes = if key.level >= smooth_level {
        mesh_surface(&Surface::sample(&regions, key), key)
    } else {
        mesh_blocks(&LodBlocks::sample(&regions, key), key.cell_size())
    };
    if meshes.opaque.count_vertices() == 0 && meshes.water.is_none() {
        return None;
    }
    Some(meshes)
}

// `cell_size` is how many blocks wide each cell of `source` is.
fn mesh_blocks(source: &impl BlockSource, cell_size: i64) -> ChunkMeshes {
    let mut opaque = MeshBuffers::default();
    let mut water = MeshBuffers::default();

    let cube_vertices = crate::util::primitives::cube();
    let face_vertices = [
        &cube_vertices[0..4],   // front
        &cube_vertices[4..8],   // right
        &cube_vertices[8..12],  // left
        &cube_vertices[12..16], // back
        &cube_vertices[16..20], // top
        &cube_vertices[20..24], // bottom
    ];

    // Water's surface sits below a full block's height (see `cube_with_top`), so its
    // top/side geometry is shorter than every other block's.
    let water_cube_vertices =
        crate::util::primitives::cube_with_top(0.5 - (0.5 - WATER_TOP) / cell_size as f32);
    let water_face_vertices = [
        &water_cube_vertices[0..4],
        &water_cube_vertices[4..8],
        &water_cube_vertices[8..12],
        &water_cube_vertices[12..16],
        &water_cube_vertices[16..20],
        &water_cube_vertices[20..24],
    ];

    // columns' water depths, shared between the corners that sample them
    let mut depths = HashMap::new();
    let mut depth_of =
        |coord: IVec3| *depths.entry(coord).or_insert_with(|| water_depth(source, coord));

    for (block_coord, block) in source.non_air() {
        let world_position = block_coord.as_vec3();

        // Order matches `face_vertices`/`sides` below and `FACE_NORMAL_OFFSET`.
        let front = source.block(block_coord + FACE_NORMAL_OFFSET[0]);
        let right = source.block(block_coord + FACE_NORMAL_OFFSET[1]);
        let left = source.block(block_coord + FACE_NORMAL_OFFSET[2]);
        let back = source.block(block_coord + FACE_NORMAL_OFFSET[3]);
        let top = source.block(block_coord + FACE_NORMAL_OFFSET[4]);
        let bottom = source.block(block_coord + FACE_NORMAL_OFFSET[5]);

        let is_water = block == BlockType::Water;
        let geometry = if is_water { &water_face_vertices } else { &face_vertices };
        let buffers = if is_water { &mut water } else { &mut opaque };
        let uv_offset = (block as usize - 1) as f32;

        let sides = [front, right, left, back, top, bottom];
        for (i, side) in sides.iter().enumerate() {
            let draw = match side {
                BlockType::Water => !is_water,
                BlockType::Air => true,
                _ => false,
            };
            if !draw {
                continue;
            }

            // Water's color channels carry baked shoreline/depth/top-face/distance
            // data (R/G/B/A, see `vertex_shore_factor`/`water_depth`/
            // `vertex_shore_distance`) instead of grayscale AO - and only the top face
            // (i == 4) gets it, since foam/waves are surface-only.
            //
            // B (is_top) is deliberately *not* depth-scaled: wave displacement in the
            // shader is gated by this flag alone, with a uniform amplitude for every
            // top face. If amplitude varied with each block's own depth instead, two
            // neighbouring water columns with slightly different depths would animate
            // their shared edge by different amounts and visibly tear apart - depth
            // only ever affects per-pixel color/alpha (G), which can't crack.
            let is_top = is_water && i == 4;
            let colors: Vec<[f32; 4]> = geometry[i]
                .iter()
                .map(|v| {
                    if is_water {
                        let depth_factor = if is_top {
                            vertex_water_depth(&mut depth_of, block_coord, i, v.position)
                                * cell_size as f32
                                / CHUNK_SIZE as f32
                        } else {
                            0.0
                        };
                        let (shore, shore_distance) = if is_top {
                            (
                                vertex_shore_factor(source, block_coord, i, v.position),
                                vertex_shore_distance(source, block_coord, i, v.position),
                            )
                        } else {
                            (0.0, 1.0)
                        };
                        [shore, depth_factor, if is_top { 1.0 } else { 0.0 }, shore_distance]
                    } else {
                        let ao = vertex_ao(source, block_coord, i, v.position);
                        [ao, ao, ao, 1.0]
                    }
                })
                .collect();
            buffers.push_face(geometry[i], world_position, uv_offset, &colors);
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bevy::math::{I64Vec3, IVec3, U16Vec3};
    use bevy::utils::HashMap;

    use crate::block::BlockType;

    use super::{
        generate_chunk, generate_chunk_mesh, resolve_block, terrain_block, vertex_ao,
        vertex_water_depth, water_depth, BlockSource, ChunkNeighbourhood, LodBlocks, AO_STRENGTH,
    };
    use crate::chunks::chunk::{neighbor_26_index, ChunkCoordinate, ChunkData};
    use crate::chunks::generate::noise::NoiseGenerator;
    use crate::chunks::generate::region_store::RegionStore;
    use crate::chunks::generate::erosion::ErosionParams;
    use crate::chunks::lod::LodKey;

    fn no_neighbors() -> [Option<Arc<ChunkData>>; 26] {
        std::array::from_fn(|_| None)
    }

    #[test]
    fn test_resolve_block_within_chunk() {
        let mut chunk = ChunkData::default();
        chunk.set_block_at(U16Vec3::new(4, 4, 4), BlockType::Stone);

        let block = resolve_block(&chunk, &no_neighbors(), IVec3::new(4, 4, 4));
        assert_eq!(BlockType::Stone, block);
    }

    #[test]
    fn test_resolve_block_crosses_single_axis_into_adjacent_chunk() {
        let chunk = ChunkData::default();
        let mut adjacent = ChunkData::default();
        adjacent.set_block_at(U16Vec3::new(0, 4, 4), BlockType::Stone);

        let mut neighbors = no_neighbors();
        neighbors[neighbor_26_index(1, 0, 0)] = Some(Arc::new(adjacent));
        let block = resolve_block(&chunk, &neighbors, IVec3::new(chunk.size as i32, 4, 4));
        assert_eq!(BlockType::Stone, block);
    }

    #[test]
    fn test_resolve_block_crosses_two_axes_into_diagonal_chunk() {
        let chunk = ChunkData::default();
        let mut diagonal = ChunkData::default();
        diagonal.set_block_at(U16Vec3::new(0, 0, 4), BlockType::Stone);

        let mut neighbors = no_neighbors();
        neighbors[neighbor_26_index(1, 1, 0)] = Some(Arc::new(diagonal));
        let block = resolve_block(
            &chunk,
            &neighbors,
            IVec3::new(chunk.size as i32, chunk.size as i32, 4),
        );
        assert_eq!(BlockType::Stone, block);
    }

    #[test]
    fn test_resolve_block_missing_diagonal_chunk_is_air() {
        let chunk = ChunkData::default();
        let block = resolve_block(
            &chunk,
            &no_neighbors(),
            IVec3::new(chunk.size as i32, chunk.size as i32, 4),
        );
        assert_eq!(BlockType::Air, block);
    }

    #[test]
    fn test_vertex_ao_no_occluders_is_unlit() {
        let chunk = ChunkData::default();
        let neighbors = no_neighbors();
        let source = ChunkNeighbourhood {
            chunk: &chunk,
            neighbors: &neighbors,
        };
        let ao = vertex_ao(&source, IVec3::new(4, 4, 4), 4, [0.5, 0.5, -0.5]);
        assert_eq!(1.0, ao);
    }

    #[test]
    fn test_vertex_ao_fully_occluded_corner() {
        let mut chunk = ChunkData::default();
        // Top face (index 4) at (4,4,4): occluders are the y+1 layer's tangent/diagonal cells.
        chunk.set_block_at(U16Vec3::new(5, 5, 4), BlockType::Stone);
        chunk.set_block_at(U16Vec3::new(4, 5, 5), BlockType::Stone);
        chunk.set_block_at(U16Vec3::new(5, 5, 5), BlockType::Stone);

        let neighbors = no_neighbors();
        let source = ChunkNeighbourhood {
            chunk: &chunk,
            neighbors: &neighbors,
        };
        let ao = vertex_ao(&source, IVec3::new(4, 4, 4), 4, [0.5, 0.5, 0.5]);
        assert_eq!(1.0 - AO_STRENGTH, ao);
    }

    #[test]
    fn test_neighbouring_water_columns_agree_on_shared_corner_depth() {
        let mut chunk = ChunkData::default();
        // two columns with their surface at y = 5: one 5 deep, its +x neighbour 2 deep
        chunk.set_block_at(U16Vec3::new(4, 0, 4), BlockType::Stone);
        for y in 1..=5 {
            chunk.set_block_at(U16Vec3::new(4, y, 4), BlockType::Water);
        }
        for y in 0..=3 {
            chunk.set_block_at(U16Vec3::new(5, y, 4), BlockType::Stone);
        }
        for y in 4..=5 {
            chunk.set_block_at(U16Vec3::new(5, y, 4), BlockType::Water);
        }
        let neighbors = no_neighbors();
        let source = ChunkNeighbourhood {
            chunk: &chunk,
            neighbors: &neighbors,
        };
        let mut depth_of = |coord: IVec3| water_depth(&source, coord);

        // the corner between them, on the +z side: seen from each column's top face
        let deep = vertex_water_depth(&mut depth_of, IVec3::new(4, 5, 4), 4, [0.5, 0.5, 0.5]);
        let shallow =
            vertex_water_depth(&mut depth_of, IVec3::new(5, 5, 4), 4, [-0.5, 0.5, 0.5]);
        assert_eq!(deep, shallow);
        // averaged with the two dry columns on the +z side
        assert_eq!((5.0 + 2.0) / 4.0, deep);
    }

    #[test]
    fn test_terrain_block_floor_and_water() {
        // below the solid floor even a zero-height column is solid
        assert_eq!(BlockType::Sand, terrain_block(5, 0, 0.0));
        assert_eq!(BlockType::Water, terrain_block(20, 0, 0.0));
        assert_eq!(BlockType::Air, terrain_block(40, 0, 0.0));
        assert_eq!(BlockType::Grass, terrain_block(40, 50, 0.0));
    }

    fn test_regions() -> Arc<RegionStore> {
        Arc::new(RegionStore::new(
            3,
            256,
            Arc::new(NoiseGenerator::new(3)),
            ErosionParams {
                droplet_count: 50,
                ..ErosionParams::default()
            },
        ))
    }

    #[test]
    fn test_lod_level_zero_matches_full_detail_chunk() {
        let regions = test_regions();
        for coord in [
            I64Vec3::new(1, 1, 2),
            I64Vec3::new(-2, 0, 3),
            I64Vec3::new(0, 2, -1),
        ] {
            let chunk = generate_chunk(regions.clone(), ChunkCoordinate(coord));
            let lod = LodBlocks::sample(&regions, LodKey { level: 0, coord });
            for x in 0..16 {
                for y in 0..16 {
                    for z in 0..16 {
                        assert_eq!(
                            chunk.get_block_at(U16Vec3::new(x, y, z)),
                            lod.raw(IVec3::new(x as i32, y as i32, z as i32)),
                            "chunk {coord} block ({x},{y},{z})"
                        );
                    }
                }
            }
        }
    }

    fn flat_lod(surface_y: i32) -> LodBlocks {
        let size = 16;
        let padded = (size + 2) as usize;
        let mut cells = vec![BlockType::Air; padded * padded * padded];
        for x in -1..=size {
            for z in -1..=size {
                for y in -1..surface_y {
                    cells[LodBlocks::index(size, IVec3::new(x, y, z))] = BlockType::Stone;
                }
            }
        }
        LodBlocks { size, cells }
    }

    #[test]
    fn test_lod_border_exposes_surface_cells_as_skirt() {
        let lod = flat_lod(8);
        // the border's top two solid cells read as open, so the edge draws a wall
        assert_eq!(BlockType::Air, lod.block(IVec3::new(-1, 7, 4)));
        assert_eq!(BlockType::Air, lod.block(IVec3::new(4, 6, 16)));
        // deeper border cells and every interior cell read as they are
        assert_eq!(BlockType::Stone, lod.block(IVec3::new(-1, 5, 4)));
        assert_eq!(BlockType::Stone, lod.block(IVec3::new(4, 7, 4)));
        // below the node isn't a horizontal border, so no skirt
        assert_eq!(BlockType::Stone, lod.block(IVec3::new(4, -1, 4)));
    }

    #[test]
    fn test_lod_block_clamps_far_probes_to_border() {
        let lod = flat_lod(8);
        assert_eq!(
            lod.block(IVec3::new(-1, 3, 4)),
            lod.block(IVec3::new(-3, 3, 4))
        );
    }

    #[test]
    fn test_lod_non_air_only_covers_interior() {
        let lod = flat_lod(8);
        let cells = lod.non_air();
        assert_eq!(16 * 16 * 8, cells.len());
        assert!(cells
            .iter()
            .all(|(c, _)| c.min_element() >= 0 && c.max_element() < 16));
    }

    // Not run as part of normal `cargo test` - this is a before/after timing comparison
    // for the ChunkData storage representation, not a correctness check. Run with:
    //   cargo test --release -- --ignored --nocapture bench_generate_and_mesh_chunks
    // Capture a baseline before changing ChunkData's storage, then re-run after with the
    // same seed/coordinates and compare - `get_block_at`/`set_block_at` are on the hot
    // path here (every block, every face, every AO sample), so this isolates exactly the
    // cost a storage change would add, rather than inferring it from `cargo test` passing.
    #[test]
    #[ignore]
    fn bench_generate_and_mesh_chunks() {
        let noise = Arc::new(NoiseGenerator::new(42));

        // x,z: 20 chunks across; y: from deep underground (-10) to above the surface (9),
        // so the batch mixes uniform deep chunks with real varied surface terrain.
        let mut coords = Vec::new();
        for x in 0..20 {
            for y in -10..10 {
                for z in 0..20 {
                    coords.push(ChunkCoordinate(I64Vec3::new(x, y, z)));
                }
            }
        }

        let regions = Arc::new(RegionStore::new(42, 256, noise, ErosionParams::default()));
        let start = std::time::Instant::now();
        let data: HashMap<ChunkCoordinate, Arc<ChunkData>> = coords
            .iter()
            .map(|&c| (c, Arc::new(generate_chunk(regions.clone(), c))))
            .collect();
        let generate_elapsed = start.elapsed();

        let start = std::time::Instant::now();
        for &c in &coords {
            let neighbours = c.neighbors_26().iter().map(|n| data.get(n).cloned()).collect();
            generate_chunk_mesh(data[&c].clone(), neighbours);
        }
        let mesh_elapsed = start.elapsed();

        println!(
            "bench_generate_and_mesh_chunks: {} chunks, generate={:?}, mesh={:?}",
            coords.len(),
            generate_elapsed,
            mesh_elapsed
        );
    }
}
