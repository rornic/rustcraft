use std::sync::Arc;

use bevy::{
    math::{I64Vec2, IVec3, U16Vec3, Vec3},
    render::{
        mesh::{Indices, Mesh, VertexAttributeValues},
        render_asset::RenderAssetUsages,
    },
};

use super::noise::NoiseGenerator;
use crate::block::{BlockType, BLOCK_COUNT};
use crate::chunks::chunk::{neighbor_26_index, ChunkCoordinate, ChunkData};
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
    chunk: &ChunkData,
    adjacent_chunks: &[Option<Arc<ChunkData>>],
    block_coord: IVec3,
    face_index: usize,
    vertex_local_pos: [f32; 3],
) -> f32 {
    let (offset_a, offset_b) = tangent_offsets(face_index, vertex_local_pos);
    let normal_off = FACE_NORMAL_OFFSET[face_index];

    let side_a = resolve_block(chunk, adjacent_chunks, block_coord + normal_off + offset_a);
    let side_b = resolve_block(chunk, adjacent_chunks, block_coord + normal_off + offset_b);
    let corner = resolve_block(
        chunk,
        adjacent_chunks,
        block_coord + normal_off + offset_a + offset_b,
    );

    let sum = side_a.occlusion_weight() + side_b.occlusion_weight() + corner.occlusion_weight();
    1.0 - (sum / 3.0).clamp(0.0, 1.0) * AO_STRENGTH
}

// Fraction of a water top-face vertex's same-height tangent corners that are solid
// land - used to bake a shoreline mask into the water mesh so the shader can lap
// foam there.
fn vertex_shore_factor(
    chunk: &ChunkData,
    adjacent_chunks: &[Option<Arc<ChunkData>>],
    block_coord: IVec3,
    face_index: usize,
    vertex_local_pos: [f32; 3],
) -> f32 {
    let (offset_a, offset_b) = tangent_offsets(face_index, vertex_local_pos);

    let side_a = resolve_block(chunk, adjacent_chunks, block_coord + offset_a);
    let side_b = resolve_block(chunk, adjacent_chunks, block_coord + offset_b);
    let corner = resolve_block(chunk, adjacent_chunks, block_coord + offset_a + offset_b);

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
    chunk: &ChunkData,
    adjacent_chunks: &[Option<Arc<ChunkData>>],
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
            if resolve_block(chunk, adjacent_chunks, probe).occlusion_weight() == 1.0 {
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
    noise: Arc<NoiseGenerator>,
    chunk_pos: ChunkCoordinate,
    world_height: u64,
) -> ChunkData {
    let mut chunk_data = ChunkData::default();

    for x in 0..chunk_data.size {
        for z in 0..chunk_data.size {
            let (world_x, world_y, world_z) = (
                chunk_pos.0.x * chunk_data.size as i64 + x as i64,
                chunk_pos.0.y * chunk_data.size as i64,
                chunk_pos.0.z * chunk_data.size as i64 + z as i64,
            );
            let noise_val = noise.get(I64Vec2::new(world_x, world_z));

            let world_height = (noise_val * world_height as f64).round() as u64;
            let chunk_height = if world_y > 0 {
                let positive_y = world_y as u64;
                (world_height - positive_y.min(world_height)).min(chunk_data.size as u64)
            } else {
                chunk_data.size as u64
            };

            let gradient_x = (world_height as f64
                * (noise.get(I64Vec2::new(world_x + 1, world_z))
                    - noise.get(I64Vec2::new(world_x - 1, world_z))))
            .abs();
            let gradient_z = (world_height as f64
                * (noise.get(I64Vec2::new(world_x, world_z + 1))
                    - noise.get(I64Vec2::new(world_x, world_z - 1))))
            .abs();

            let combined_gradient = gradient_x + gradient_z;

            for y in 0..chunk_height {
                let world_y = world_y + y as i64;

                let block = if world_y >= 90 && combined_gradient <= 2.0 {
                    BlockType::Snow
                } else if world_y >= 70 && combined_gradient >= 2.0
                    || (world_y >= 36 && combined_gradient >= 3.5)
                {
                    BlockType::Stone
                } else if world_y >= 36 {
                    BlockType::Grass
                } else {
                    BlockType::Sand
                };
                chunk_data.set_block_at(U16Vec3::new(x, y as u16, z), block);
            }

            if world_y <= 16 {
                for y in chunk_height..chunk_data.size as u64 {
                    chunk_data.set_block_at(U16Vec3::new(x, y as u16, z), BlockType::Water);
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
struct MeshBuffers {
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
    colors: Vec<[f32; 4]>,
}

impl MeshBuffers {
    fn push_face(&mut self, vs: &[Vertex], position: Vec3, uv_offset: f32, colors: &[[f32; 4]]) {
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

    fn build_mesh(self) -> Mesh {
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
fn water_depth(chunk: &ChunkData, block_coord: IVec3) -> u32 {
    let mut depth = 0u32;
    let mut y = block_coord.y;
    while y >= 0 {
        let pos = U16Vec3::new(block_coord.x as u16, y as u16, block_coord.z as u16);
        if chunk.get_block_at(pos) != BlockType::Water {
            break;
        }
        depth += 1;
        y -= 1;
    }
    depth
}

pub fn generate_chunk_mesh(
    chunk: Arc<ChunkData>,
    adjacent_chunks: Vec<Option<Arc<ChunkData>>>,
) -> ChunkMeshes {
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
    let water_cube_vertices = crate::util::primitives::cube_with_top(0.4);
    let water_face_vertices = [
        &water_cube_vertices[0..4],
        &water_cube_vertices[4..8],
        &water_cube_vertices[8..12],
        &water_cube_vertices[12..16],
        &water_cube_vertices[16..20],
        &water_cube_vertices[20..24],
    ];

    for (coord, block) in chunk.iter_non_air() {
        let (x, y, z) = (coord.x, coord.y, coord.z);
        let world_position = Vec3::new(x as f32, y as f32, z as f32);
        let block_coord = IVec3::new(x as i32, y as i32, z as i32);

        // Order matches `face_vertices`/`sides` below and `FACE_NORMAL_OFFSET`.
        let front = resolve_block(&chunk, &adjacent_chunks, block_coord + FACE_NORMAL_OFFSET[0]);
        let right = resolve_block(&chunk, &adjacent_chunks, block_coord + FACE_NORMAL_OFFSET[1]);
        let left = resolve_block(&chunk, &adjacent_chunks, block_coord + FACE_NORMAL_OFFSET[2]);
        let back = resolve_block(&chunk, &adjacent_chunks, block_coord + FACE_NORMAL_OFFSET[3]);
        let top = resolve_block(&chunk, &adjacent_chunks, block_coord + FACE_NORMAL_OFFSET[4]);
        let bottom = resolve_block(&chunk, &adjacent_chunks, block_coord + FACE_NORMAL_OFFSET[5]);

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
            let depth_factor = if is_top {
                water_depth(&chunk, block_coord) as f32 / chunk.size as f32
            } else {
                0.0
            };
            let colors: Vec<[f32; 4]> = geometry[i]
                .iter()
                .map(|v| {
                    if is_water {
                        let (shore, shore_distance) = if is_top {
                            (
                                vertex_shore_factor(&chunk, &adjacent_chunks, block_coord, i, v.position),
                                vertex_shore_distance(&chunk, &adjacent_chunks, block_coord, i, v.position),
                            )
                        } else {
                            (0.0, 1.0)
                        };
                        [shore, depth_factor, if is_top { 1.0 } else { 0.0 }, shore_distance]
                    } else {
                        let ao = vertex_ao(&chunk, &adjacent_chunks, block_coord, i, v.position);
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

    use super::{generate_chunk, generate_chunk_mesh, resolve_block, vertex_ao, AO_STRENGTH};
    use crate::chunks::chunk::{neighbor_26_index, ChunkCoordinate, ChunkData};
    use crate::chunks::generate::noise::NoiseGenerator;

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
        let ao = vertex_ao(&chunk, &no_neighbors(), IVec3::new(4, 4, 4), 4, [0.5, 0.5, -0.5]);
        assert_eq!(1.0, ao);
    }

    #[test]
    fn test_vertex_ao_fully_occluded_corner() {
        let mut chunk = ChunkData::default();
        // Top face (index 4) at (4,4,4): occluders are the y+1 layer's tangent/diagonal cells.
        chunk.set_block_at(U16Vec3::new(5, 5, 4), BlockType::Stone);
        chunk.set_block_at(U16Vec3::new(4, 5, 5), BlockType::Stone);
        chunk.set_block_at(U16Vec3::new(5, 5, 5), BlockType::Stone);

        let ao = vertex_ao(&chunk, &no_neighbors(), IVec3::new(4, 4, 4), 4, [0.5, 0.5, 0.5]);
        assert_eq!(1.0 - AO_STRENGTH, ao);
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

        let start = std::time::Instant::now();
        let data: HashMap<ChunkCoordinate, Arc<ChunkData>> = coords
            .iter()
            .map(|&c| (c, Arc::new(generate_chunk(noise.clone(), c, 256))))
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
