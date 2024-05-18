use std::collections::HashMap;

use bevy::ecs::component::Component;
use bevy::math::{I64Vec2, U16Vec3, Vec3};
use bevy::render::mesh::{Indices, Mesh, VertexAttributeValues};
use bevy::render::render_asset::RenderAssetUsages;
use noise::NoiseFn;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use crate::util::primitives::Vertex;

pub mod ecs;
mod generator;

/// Each chunk is a cube of blocks. `CHUNK_SIZE` determines the size of this cube in blocks.
pub const CHUNK_SIZE: u16 = 16;

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub enum BlockType {
    Air,
    Stone,
    Grass,
    Sand,
    Water,
    Snow,
}

impl BlockType {
    pub fn is_solid(&self) -> bool {
        match self {
            BlockType::Water | BlockType::Air => false,
            _ => true,
        }
    }
}

const BLOCK_COUNT: usize = 6;

pub type ChunkCoordinate = I64Vec2;

pub struct ChunkData {
    blocks: Box<[[[BlockType; CHUNK_SIZE as usize]; WORLD_HEIGHT as usize]; CHUNK_SIZE as usize]>,
    dirty: bool,
}

const WORLD_HEIGHT: u16 = 256;
const MIN_SPAWN_HEIGHT: u16 = WORLD_HEIGHT / 3;
const MAX_SPAWN_HEIGHT: u16 = WORLD_HEIGHT / 2;

#[derive(Component)]
pub struct World {
    generator: WorldGenerator,
    chunks: HashMap<ChunkCoordinate, ChunkData>,
    spawn: Vec3,
}

impl Default for World {
    fn default() -> Self {
        let mut world = Self {
            generator: Default::default(),
            chunks: Default::default(),
            spawn: Vec3::new(0.0, 0.0, 0.0),
        };

        let mut rng = StdRng::seed_from_u64(world.generator.seed as u64);
        let mut spawn: Option<Vec3> = None;
        while let None = spawn {
            let chunk_pos =
                ChunkCoordinate::new(rng.gen_range(-256..256), rng.gen_range(-256..256));
            let chunk = world.generate_chunk(chunk_pos);

            let y = (0..WORLD_HEIGHT)
                .rev()
                .find(|y| chunk.blocks[0][*y as usize][0] != BlockType::Air)
                .unwrap_or(0);
            if y > MIN_SPAWN_HEIGHT && y < MAX_SPAWN_HEIGHT {
                spawn = Some(Vec3::new(
                    chunk_pos.x as f32 * CHUNK_SIZE as f32,
                    y as f32 + 2.0,
                    chunk_pos.y as f32 * CHUNK_SIZE as f32,
                ));
            }
        }
        world.spawn = spawn.unwrap();

        world
    }
}

impl World {
    pub fn cache_chunk(&mut self, chunk_coord: ChunkCoordinate, chunk: ChunkData) {
        self.chunks.insert(chunk_coord, chunk);
    }

    pub fn generate_chunk(&self, chunk_coord: ChunkCoordinate) -> ChunkData {
        self.generator.generate_chunk(chunk_coord)
    }

    pub fn is_chunk_generated(&self, chunk_coord: ChunkCoordinate) -> bool {
        self.chunks.contains_key(&chunk_coord)
    }

    pub fn are_neighbours_generated(&self, chunk_coord: ChunkCoordinate) -> bool {
        [[0, 1], [0, -1], [1, 0], [-1, 0]]
            .iter()
            .all(|p| self.is_chunk_generated(chunk_coord + I64Vec2::new(p[0], p[1])))
    }

    pub fn spawn(&self) -> Vec3 {
        self.spawn
    }

    pub fn set_block_at(&mut self, pos: Vec3, block: BlockType) {
        let (chunk_pos, pos) = self.world_to_block_relative(pos);
        let chunk = self
            .chunks
            .get_mut(&chunk_pos)
            .expect("attempting to set block in chunk that is not generated");
        chunk.blocks[pos.x as usize][pos.y as usize][pos.z as usize] = block;
        chunk.dirty = true;

        // If changing a block on the edge of a chunk, we also need to set the dirty bit
        // on the neighbouring chunks.
        let mut regenerate_neighbours = vec![];
        if pos.x == 0 {
            regenerate_neighbours.push(chunk_pos + I64Vec2::new(-1, 0));
        } else if pos.x == CHUNK_SIZE - 1 {
            regenerate_neighbours.push(chunk_pos + I64Vec2::new(1, 0));
        }

        if pos.z == 0 {
            regenerate_neighbours.push(chunk_pos + I64Vec2::new(0, -1));
        } else if pos.z == CHUNK_SIZE - 1 {
            regenerate_neighbours.push(chunk_pos + I64Vec2::new(0, 1));
        }

        for neighbour in regenerate_neighbours {
            self.chunks
                .get_mut(&neighbour)
                .expect("neighbouring chunk is not generated")
                .dirty = true;
        }
    }

    fn generate_chunk_mesh(&self, chunk_coord: ChunkCoordinate) -> Mesh {
        let mut vertices: Vec<Vertex> = vec![];
        let mut indices: Vec<u32> = vec![];

        let mut add_vertices = |vs: &[Vertex], position: Vec3, block_type: BlockType| {
            let uv_scale = 1.0 / (BLOCK_COUNT - 1) as f32;

            let triangle_start: u32 = vertices.len() as u32;
            vertices.extend(&mut vs.iter().map(|v| Vertex {
                position: (Vec3::from(v.position) + position).into(),
                normal: v.normal,
                uv: [
                    uv_scale * (v.uv[0] + (block_type as usize - 1) as f32),
                    v.uv[1],
                ],
            }));
            indices.extend(vec![
                triangle_start,
                triangle_start + 1,
                triangle_start + 2,
                triangle_start + 2,
                triangle_start + 1,
                triangle_start + 3,
            ]);
        };

        let cube_vertices = super::util::primitives::cube();
        let face_vertices = [
            &cube_vertices[0..4],   // front
            &cube_vertices[4..8],   // right
            &cube_vertices[8..12],  // left
            &cube_vertices[12..16], // back
            &cube_vertices[16..20], // top
            &cube_vertices[20..24], // bottom
        ];

        let chunk = self.chunk(chunk_coord).unwrap();
        let adjacent_chunks = [
            self.chunk(chunk_coord + I64Vec2::new(0, 1)),
            self.chunk(chunk_coord + I64Vec2::new(0, -1)),
            self.chunk(chunk_coord + I64Vec2::new(1, 0)),
            self.chunk(chunk_coord + I64Vec2::new(-1, 0)),
        ];

        for x in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                for y in 0..WORLD_HEIGHT {
                    if chunk.blocks[x as usize][y as usize][z as usize] == BlockType::Air {
                        continue;
                    }

                    let world_position = Vec3::new(x as f32, y as f32, z as f32);

                    let front = z
                        .checked_sub(1)
                        .and_then(|z| self.chunk_block(chunk, U16Vec3::new(x, y, z)))
                        .or(adjacent_chunks[1]
                            .and_then(|c| self.chunk_block(c, U16Vec3::new(x, y, CHUNK_SIZE - 1))));
                    let back = self
                        .chunk_block(chunk, U16Vec3::new(x, y, z + 1))
                        .or(adjacent_chunks[0]
                            .and_then(|c| self.chunk_block(c, U16Vec3::new(x, y, 0))));
                    let left = x
                        .checked_sub(1)
                        .and_then(|x| self.chunk_block(chunk, U16Vec3::new(x, y, z)))
                        .or(adjacent_chunks[3]
                            .and_then(|c| self.chunk_block(c, U16Vec3::new(CHUNK_SIZE - 1, y, z))));
                    let right = self
                        .chunk_block(chunk, U16Vec3::new(x + 1, y, z))
                        .or(adjacent_chunks[2]
                            .and_then(|c| self.chunk_block(c, U16Vec3::new(0, y, z))));
                    let top = self.chunk_block(chunk, U16Vec3::new(x, y + 1, z));

                    let bottom = if y == 0 {
                        Some(BlockType::Stone)
                    } else {
                        y.checked_sub(1)
                            .and_then(|y| self.chunk_block(chunk, U16Vec3::new(x, y, z)))
                    };

                    let sides = [front, right, left, back, top, bottom];
                    for (i, side) in sides.iter().enumerate() {
                        match side {
                            Some(BlockType::Water) => {
                                if chunk.blocks[x as usize][y as usize][z as usize]
                                    != BlockType::Water
                                {
                                    add_vertices(
                                        &face_vertices[i],
                                        world_position,
                                        chunk.blocks[x as usize][y as usize][z as usize],
                                    )
                                }
                            }
                            None | Some(BlockType::Air) => add_vertices(
                                &face_vertices[i],
                                world_position,
                                chunk.blocks[x as usize][y as usize][z as usize],
                            ),
                            _ => (),
                        };
                    }
                }
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
        mesh
    }

    fn block_at(&self, position: Vec3) -> BlockType {
        let (chunk_pos, block_pos) = self.world_to_block_relative(position);
        if block_pos.y >= WORLD_HEIGHT {
            return BlockType::Air;
        }

        if let Some(chunk) = self.chunks.get(&chunk_pos) {
            chunk.blocks[block_pos.x as usize][block_pos.y as usize][block_pos.z as usize]
        } else {
            BlockType::Air
        }
    }

    fn chunk(&self, chunk_coord: ChunkCoordinate) -> Option<&ChunkData> {
        self.chunks.get(&chunk_coord)
    }

    fn clear_chunk_dirty_bit(&mut self, chunk_coord: ChunkCoordinate) {
        self.chunks.get_mut(&chunk_coord).unwrap().dirty = false;
    }

    fn chunk_block<'a>(&self, chunk: &'a ChunkData, block: U16Vec3) -> Option<BlockType> {
        chunk
            .blocks
            .get(block.x as usize)
            .and_then(|c| {
                c.get(block.y as usize)
                    .and_then(|c| c.get(block.z as usize))
            })
            .copied()
    }

    /// Takes a position in the world and returns the chunk that it's in.
    fn world_to_chunk(&self, world_position: Vec3) -> ChunkCoordinate {
        ChunkCoordinate::new(
            (world_position.x / CHUNK_SIZE as f32).floor() as i64,
            (world_position.z / CHUNK_SIZE as f32).floor() as i64,
        )
    }

    /// Takes a position in the world and converts it to a position relative to the chunk it's in.
    fn world_to_block_relative(&self, world_position: Vec3) -> (ChunkCoordinate, U16Vec3) {
        let chunk = self.world_to_chunk(world_position);
        let relative_pos = U16Vec3::new(
            (world_position.x - (chunk.x * CHUNK_SIZE as i64) as f32).floor() as u16,
            world_position.y.floor() as u16,
            (world_position.z - (chunk.y * CHUNK_SIZE as i64) as f32).floor() as u16,
        );
        (chunk, relative_pos)
    }

    fn world_to_block(&self, world_position: Vec3) -> Vec3 {
        Vec3::new(
            world_position.x.floor(),
            world_position.y.floor(),
            world_position.z.floor(),
        )
    }

    fn block_centre(&self, world_position: Vec3) -> Vec3 {
        Vec3::new(
            world_position.x.round(),
            world_position.y.round(),
            world_position.z.round(),
        )
    }
}

#[derive(Copy, Clone)]
struct WorldGenerator {
    seed: u32,
}

impl Default for WorldGenerator {
    fn default() -> Self {
        Self {
            seed: rand::random(),
        }
    }
}

impl WorldGenerator {
    fn generate_chunk(&self, chunk_pos: ChunkCoordinate) -> ChunkData {
        let mut blocks =
            [[[BlockType::Air; CHUNK_SIZE as usize]; WORLD_HEIGHT as usize]; CHUNK_SIZE as usize];

        let noise = generator::noise_generator(self.seed);

        for x in 0..CHUNK_SIZE.into() {
            for z in 0..CHUNK_SIZE.into() {
                let (world_x, _, world_z) = (
                    chunk_pos.x * CHUNK_SIZE as i64 + x as i64,
                    0,
                    chunk_pos.y * CHUNK_SIZE as i64 + z as i64,
                );
                let noise_val = noise.get([world_x as f64, world_z as f64]);

                let height = (noise_val * WORLD_HEIGHT as f64).round() as usize;
                let gradient_x = (WORLD_HEIGHT as f64
                    * (noise.get([(world_x + 1) as f64, world_z as f64])
                        - noise.get([(world_x - 1) as f64, world_z as f64])))
                .abs();
                let gradient_z = (WORLD_HEIGHT as f64
                    * (noise.get([world_x as f64, (world_z + 1) as f64])
                        - noise.get([world_x as f64, (world_z - 1) as f64])))
                .abs();

                // Height must be at least 1!
                let height = height.min(WORLD_HEIGHT as usize - 1).max(1);
                for y in 0..height {
                    if height >= 180 && ((gradient_x + gradient_z) <= 2.0) {
                        blocks[x][y][z] = BlockType::Snow;
                    } else if y >= 10 && ((gradient_x + gradient_z) >= 2.0) {
                        blocks[x][y][z] = BlockType::Stone;
                    } else if y < 10 {
                        blocks[x][y][z] = BlockType::Sand;
                    } else {
                        blocks[x][y][z] = BlockType::Grass;
                    }
                }

                for y in height..5 {
                    blocks[x][y][z] = BlockType::Water;
                }
            }
        }

        ChunkData {
            blocks: Box::new(blocks),
            dirty: false,
        }
    }
}
