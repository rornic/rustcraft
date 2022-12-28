use std::{collections::HashMap, sync::Arc, sync::Weak};

use glium::{
    index::{
        DrawCommandIndices, DrawCommandsIndicesBuffer, IndexBufferSlice, IndicesSource,
        PrimitiveType,
    },
    texture::SrgbTexture2d,
    uniforms::{
        MagnifySamplerFilter, MinifySamplerFilter, Sampler, SamplerBehavior, SamplerWrapFunction,
        UniformBuffer,
    },
    vertex::VertexBufferSlice,
    Blend, Display, Frame, IndexBuffer, Program, Surface, VertexBuffer,
};
use specs::{Component, Join, ReadStorage, System, VecStorage, Write};
use uuid::Uuid;

use crate::world::ecs::{bounds::Bounds, camera::Camera, Transform};

use super::{
    material::{load_shader, load_texture, Material},
    mesh::{Mesh, Vertex},
};

#[derive(Clone)]
pub struct DrawCall {
    material: Material,
    mesh: Arc<Mesh>,
    transform: Transform,
}

pub struct RenderMesh {
    mesh: Arc<Mesh>,
}

impl RenderMesh {
    pub fn new(mesh: Arc<Mesh>) -> RenderMesh {
        RenderMesh { mesh: mesh }
    }
}

impl Component for RenderMesh {
    type Storage = VecStorage<Self>;
}

#[derive(Default)]
pub struct RenderJob {
    draw_calls: Vec<DrawCall>,
    camera: (Camera, Transform),
}

pub struct RenderingSystem;

impl<'a> System<'a> for RenderingSystem {
    type SystemData = (
        ReadStorage<'a, Camera>,
        ReadStorage<'a, Transform>,
        ReadStorage<'a, RenderMesh>,
        ReadStorage<'a, Bounds>,
        Write<'a, RenderJob>,
    );

    fn run(
        &mut self,
        (cameras, transforms, render_meshes, bounds, mut render_job): Self::SystemData,
    ) {
        let (camera, camera_transform) = (&cameras, &transforms).join().next().unwrap();
        render_job.camera = (camera.clone(), camera_transform.clone());

        render_job.draw_calls.clear();

        for (transform, mesh_data, bounds) in (&transforms, &render_meshes, &bounds).join() {
            if !camera.are_bounds_visible(camera_transform, transform.position, bounds) {
                continue;
            }

            render_job.draw_calls.push(DrawCall {
                material: Material {
                    name: "default".to_string(),
                },
                mesh: mesh_data.mesh.clone(),
                transform: transform.clone(),
            });
        }
    }
}

pub struct Renderer {
    display: Display,
    global_uniform_buffer: UniformBuffer<GlobalUniforms>,
    shader: Program,
    texture: SrgbTexture2d,
    world_mesh: WorldMesh,
    command_buffer: DrawCommandsIndicesBuffer,
}

impl Renderer {
    pub fn new(display: Display) -> Self {
        let global_uniform_buffer: UniformBuffer<GlobalUniforms> =
            UniformBuffer::empty(&display).unwrap();

        // TODO: keep shaders and textures in a HashMap
        let shader = load_shader(&display, "default").unwrap();
        let texture = load_texture(&display, "textures/stone.png").unwrap();

        let world_mesh = WorldMesh::new(&display);
        let command_buffer = DrawCommandsIndicesBuffer::empty_dynamic(&display, 1024).unwrap();

        Self {
            display,
            global_uniform_buffer,
            shader,
            texture,
            world_mesh,
            command_buffer,
        }
    }

    pub fn render(&mut self, camera: &mut Camera, render_job: &RenderJob) {
        let draw_params = glium::DrawParameters {
            depth: glium::Depth {
                test: glium::draw_parameters::DepthTest::IfLess,
                write: true,
                ..Default::default()
            },
            backface_culling: glium::BackfaceCullingMode::CullClockwise,
            blend: Blend::alpha_blending(),
            ..Default::default()
        };

        for draw_call in render_job.draw_calls.iter() {
            if !self
                .world_mesh
                .loaded_meshes
                .contains_key(&draw_call.mesh.id)
            {
                self.world_mesh.load_mesh(draw_call.mesh.clone());
            }
        }

        if render_job.draw_calls.len() > 0 {
            let mut target: Frame = self.display.draw();
            target.clear_color_and_depth((0.549, 0.745, 0.839, 1.0), 1.0);

            let (width, height) = target.get_dimensions();
            camera.aspect_ratio = width as f32 / height as f32;

            let global_uniforms = GlobalUniforms {
                projection_matrix: camera.projection_matrix(),
                view_matrix: camera.view_matrix(),
                light: [-0.2, 0.7, 0.2f32],
                model_matrix: [
                    [1.0, 0.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0, 0.0],
                    [0.0, 0.0, 0.0, 1.0_f32],
                ],
            };
            self.global_uniform_buffer.write(&global_uniforms);

            let commands = render_job
                .draw_calls
                .iter()
                .map(|draw_call| {
                    let (_, vbo, ibo) = self
                        .world_mesh
                        .loaded_meshes
                        .get(&draw_call.mesh.id)
                        .unwrap();
                    DrawCommandIndices {
                        count: (ibo.count * DynamicIndexBuffer::BLOCK_SIZE) as u32,
                        instance_count: 1,
                        first_index: ibo.start as u32,
                        base_vertex: vbo.start as u32,
                        base_instance: 0,
                    }
                })
                .collect::<Vec<DrawCommandIndices>>();
            self.command_buffer
                .slice(0..commands.len())
                .unwrap()
                .write(&commands);

            target
                .draw(
                    &self.world_mesh.vbo.buffer.vbo,
                    IndicesSource::MultidrawElement {
                        commands: self
                            .command_buffer
                            .slice(0..commands.len())
                            .unwrap()
                            .as_slice_any(),
                        indices: self.world_mesh.ibo.buffer.ibo.as_slice_any(),
                        data_type: self.world_mesh.ibo.buffer.ibo.get_indices_type(),
                        primitives: self.world_mesh.ibo.buffer.ibo.get_primitives_type(),
                    },
                    &self.shader,
                    &uniform! {
                                GlobalUniforms: &self.global_uniform_buffer,
                                tex: Sampler(
                        &self.texture,
                        SamplerBehavior {
                            wrap_function: (
                                SamplerWrapFunction::Clamp,
                                SamplerWrapFunction::Clamp,
                                SamplerWrapFunction::Clamp
                            ),
                            minify_filter: MinifySamplerFilter::NearestMipmapLinear,
                            magnify_filter: MagnifySamplerFilter::Nearest,
                            ..Default::default()
                        },
                    )
                            },
                    &draw_params,
                )
                .unwrap();
            target.finish().unwrap();
            self.world_mesh.garbage_collect();
        }
    }
}

#[derive(Clone, Copy)]
struct GlobalUniforms {
    model_matrix: [[f32; 4]; 4],
    projection_matrix: [[f32; 4]; 4],
    view_matrix: [[f32; 4]; 4],
    light: [f32; 3],
}
implement_uniform_block!(
    GlobalUniforms,
    model_matrix,
    projection_matrix,
    view_matrix,
    light
);

#[derive(Clone, Copy)]
struct MemoryAllocation {
    start: usize,
    count: usize,
}

pub const RENDER_DISTANCE: usize = 8;

trait Buffer<'a> {
    const BLOCK_SIZE: usize;
    const BLOCK_COUNT: usize;
    type Slice;

    fn new(display: &Display) -> Self;
    fn slice(&'a self, block: MemoryAllocation) -> Self::Slice;
    fn clear(&self, block: MemoryAllocation);
    fn block_size(&self) -> usize {
        Self::BLOCK_SIZE
    }
}

struct DynamicVertexBuffer {
    vbo: VertexBuffer<Vertex>,
}

impl<'a> Buffer<'a> for DynamicVertexBuffer {
    const BLOCK_SIZE: usize = 4096;
    const BLOCK_COUNT: usize = 4096;
    type Slice = VertexBufferSlice<'a, Vertex>;

    fn slice(&'a self, block: MemoryAllocation) -> Self::Slice {
        self.vbo
            .slice(block.start..block.start + block.count * Self::BLOCK_SIZE)
            .unwrap()
    }

    fn new(display: &Display) -> Self {
        let vbo =
            VertexBuffer::empty_dynamic(display, Self::BLOCK_SIZE * Self::BLOCK_COUNT).unwrap();
        Self { vbo }
    }

    fn clear(&self, block: MemoryAllocation) {
        self.slice(block)
            .write(&vec![Vertex::default()].repeat(block.count * Self::BLOCK_SIZE));
    }
}

struct DynamicIndexBuffer {
    ibo: IndexBuffer<u32>,
}

impl<'a> Buffer<'a> for DynamicIndexBuffer {
    const BLOCK_SIZE: usize = DynamicVertexBuffer::BLOCK_SIZE * 3;
    const BLOCK_COUNT: usize = DynamicVertexBuffer::BLOCK_COUNT;
    type Slice = IndexBufferSlice<'a, u32>;

    fn slice(&'a self, block: MemoryAllocation) -> Self::Slice {
        self.ibo
            .slice(block.start..block.start + block.count * Self::BLOCK_SIZE)
            .unwrap()
    }

    fn new(display: &Display) -> Self {
        let ibo = IndexBuffer::empty_dynamic(
            display,
            PrimitiveType::TrianglesList,
            Self::BLOCK_SIZE * Self::BLOCK_COUNT,
        )
        .unwrap();
        Self { ibo }
    }

    fn clear(&self, block: MemoryAllocation) {
        self.slice(block)
            .write(&vec![0].repeat(block.count * Self::BLOCK_SIZE));
    }
}

// TODO: store free blocks in a linkedlist so consecutive 'small' blocks can be combined for bigger chunks instead of relying on the block size always
// being big enough. especially important with high frequency terrain
struct AllocatedBuffer<T> {
    buffer: T,
    free_blocks: Vec<MemoryAllocation>,
}

impl<'a, T: Buffer<'a>> AllocatedBuffer<T> {
    fn new(buffer: T) -> Self {
        let initial_block = MemoryAllocation {
            start: 0,
            count: buffer.block_size(),
        };
        Self {
            buffer,
            free_blocks: vec![initial_block],
        }
    }
}

impl<'a, T: Buffer<'a>> AllocatedBuffer<T> {
    fn allocate(&'a mut self, size: usize) -> Option<MemoryAllocation> {
        let block_size = self.buffer.block_size();
        let free_blocks = self.free_blocks_mut();
        if free_blocks.is_empty() {
            return None;
        }

        let desired_blocks = 1 + (size / block_size);
        let (i, block) = free_blocks
            .iter()
            .cloned()
            .enumerate()
            .find(|(_, block)| block.count >= desired_blocks)
            .expect("could not find a fitting free block");
        free_blocks.remove(i);

        if block.count > desired_blocks {
            let split = MemoryAllocation {
                start: block.start + desired_blocks * block_size,
                count: block.count - desired_blocks,
            };
            free_blocks.push(split);
            return Some(MemoryAllocation {
                start: block.start,
                count: desired_blocks,
            });
        }

        Some(block)
    }

    fn free_blocks_mut(&'a mut self) -> &'a mut Vec<MemoryAllocation> {
        &mut self.free_blocks
    }

    fn free(&mut self, block: MemoryAllocation) {
        self.buffer.clear(block);
        self.free_blocks.push(block);
    }
}

type AllocatedMesh = (Weak<Mesh>, MemoryAllocation, MemoryAllocation);
struct WorldMesh {
    vbo: AllocatedBuffer<DynamicVertexBuffer>,
    ibo: AllocatedBuffer<DynamicIndexBuffer>,
    loaded_meshes: HashMap<Uuid, AllocatedMesh>,
}

impl WorldMesh {
    fn new(display: &Display) -> Self {
        let (vbo, ibo) = (
            DynamicVertexBuffer::new(display),
            DynamicIndexBuffer::new(display),
        );
        let (vbo, ibo) = (AllocatedBuffer::new(vbo), AllocatedBuffer::new(ibo));
        Self {
            vbo,
            ibo,
            loaded_meshes: HashMap::new(),
        }
    }

    fn load_mesh(&mut self, mesh: Arc<Mesh>) {
        let vbo_alloc = self
            .vbo
            .allocate(mesh.vertices.len())
            .expect("could not allocate vbo memory");
        let ibo_alloc = self
            .ibo
            .allocate(mesh.triangles.len())
            .expect("could not allocate ibo memory");

        self.vbo
            .buffer
            .slice(vbo_alloc)
            .slice(0..mesh.vertices.len())
            .unwrap()
            .write(&mesh.vertices);

        self.ibo
            .buffer
            .slice(ibo_alloc)
            .slice(0..mesh.triangles.len())
            .unwrap()
            .write(&mesh.triangles);

        self.loaded_meshes
            .insert(mesh.id, (Arc::downgrade(&mesh), vbo_alloc, ibo_alloc));
    }

    fn unload_mesh(&mut self, id: Uuid) {
        let (_, vbo_alloc, ibo_alloc) = self.loaded_meshes.remove(&id).unwrap();
        self.vbo.free(vbo_alloc);
        self.ibo.free(ibo_alloc);
    }

    fn garbage_collect(&mut self) {
        let to_unload: Vec<Uuid> = self
            .loaded_meshes
            .iter()
            .filter(|(_, (mesh_ref, _, _))| mesh_ref.upgrade().is_none())
            .take(16)
            .map(|(id, _)| *id)
            .collect();

        for id in to_unload {
            self.unload_mesh(id);
        }
    }
}
