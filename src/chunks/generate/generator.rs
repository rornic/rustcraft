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

fn vertex_ao(
    chunk: &ChunkData,
    adjacent_chunks: &[Option<Arc<ChunkData>>],
    block_coord: IVec3,
    face_index: usize,
    vertex_local_pos: [f32; 3],
) -> f32 {
    let (ta, tb) = FACE_TANGENTS[face_index];
    let axis_index = |v: IVec3| if v.x != 0 { 0 } else if v.y != 0 { 1 } else { 2 };
    let sign_a = vertex_local_pos[axis_index(ta)].signum() as i32;
    let sign_b = vertex_local_pos[axis_index(tb)].signum() as i32;
    let normal_off = FACE_NORMAL_OFFSET[face_index];
    let offset_a = ta * sign_a;
    let offset_b = tb * sign_b;

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

pub fn generate_chunk_mesh(
    chunk: Arc<ChunkData>,
    adjacent_chunks: Vec<Option<Arc<ChunkData>>>,
) -> Mesh {
    let mut vertices: Vec<Vertex> = vec![];
    let mut indices: Vec<u32> = vec![];
    let mut colors: Vec<[f32; 4]> = vec![];

    let mut add_vertices = |vs: &[Vertex],
                             position: Vec3,
                             block_type: BlockType,
                             block_coord: IVec3,
                             face_index: usize| {
        let uv_scale = 1.0 / (BLOCK_COUNT - 1) as f32;

        let triangle_start: u32 = vertices.len() as u32;
        for v in vs.iter() {
            vertices.push(Vertex {
                position: (Vec3::from(v.position) + position).into(),
                normal: v.normal,
                uv: [
                    uv_scale * (v.uv[0] + (block_type as usize - 1) as f32),
                    v.uv[1],
                ],
            });
            let ao = vertex_ao(&chunk, &adjacent_chunks, block_coord, face_index, v.position);
            colors.push([ao, ao, ao, 1.0]);
        }
        indices.extend(vec![
            triangle_start,
            triangle_start + 1,
            triangle_start + 2,
            triangle_start + 2,
            triangle_start + 1,
            triangle_start + 3,
        ]);
    };

    let cube_vertices = crate::util::primitives::cube();
    let face_vertices = [
        &cube_vertices[0..4],   // front
        &cube_vertices[4..8],   // right
        &cube_vertices[8..12],  // left
        &cube_vertices[12..16], // back
        &cube_vertices[16..20], // top
        &cube_vertices[20..24], // bottom
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

        let sides = [front, right, left, back, top, bottom];
        for (i, side) in sides.iter().enumerate() {
            match side {
                BlockType::Water => {
                    if block != BlockType::Water {
                        add_vertices(&face_vertices[i], world_position, block, block_coord, i)
                    }
                }
                BlockType::Air => {
                    add_vertices(&face_vertices[i], world_position, block, block_coord, i)
                }
                _ => (),
            };
        }
    }

    let mut mesh = Mesh::new(
        bevy::render::mesh::PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    );
    mesh.insert_indices(Indices::U32(indices));
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        VertexAttributeValues::Float32x3(vertices.iter().map(|v| v.position).collect()),
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_NORMAL,
        VertexAttributeValues::Float32x3(vertices.iter().map(|v| v.normal).collect()),
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_UV_0,
        VertexAttributeValues::Float32x2(vertices.iter().map(|v| v.uv).collect()),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, VertexAttributeValues::Float32x4(colors));
    mesh
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
