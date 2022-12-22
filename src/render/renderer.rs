use std::{
    collections::{HashMap, VecDeque},
    option::{IntoIter, Iter},
    sync::Arc,
    time::Instant,
};

use cgmath::{InnerSpace, Vector3};
use glium::{
    buffer::BufferAnySlice,
    index::{
        DrawCommandIndices, DrawCommandsIndicesBuffer, IndexBufferSlice, IndicesSource,
        PrimitiveType,
    },
    texture::SrgbTexture2d,
    uniforms::{
        MagnifySamplerFilter, MinifySamplerFilter, Sampler, SamplerBehavior, UniformBuffer,
    },
    vertex::{MultiVerticesSource, VertexBufferAny, VertexBufferSlice, VerticesSource},
    Display, Frame, IndexBuffer, Program, Surface, VertexBuffer,
};
use specs::{Component, Join, ReadStorage, System, VecStorage, Write};
use uuid::Uuid;

use crate::{
    vector3,
    world::ecs::{bounds::Bounds, camera::Camera, Transform},
    DrawCalls,
};

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

pub struct RenderingSystem;

impl<'a> System<'a> for RenderingSystem {
    type SystemData = (
        ReadStorage<'a, Camera>,
        ReadStorage<'a, Transform>,
        ReadStorage<'a, RenderMesh>,
        ReadStorage<'a, Bounds>,
        Write<'a, DrawCalls>,
    );

    fn run(
        &mut self,
        (cameras, transforms, render_meshes, bounds, mut draw_calls): Self::SystemData,
    ) {
        let (camera, camera_transform) = (&cameras, &transforms).join().next().unwrap();

        draw_calls.0.clear();
        for (transform, mesh_data, bounds) in (&transforms, &render_meshes, &bounds).join() {
            if !camera.are_bounds_visible(camera_transform, transform.position, bounds) {
                continue;
            }

            draw_calls.0.push(DrawCall {
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
    mesh_buffer_manager: MeshBufferManager,
    command_buffer: DrawCommandsIndicesBuffer,
}

impl Renderer {
    pub fn new(display: Display) -> Self {
        let global_uniform_buffer: UniformBuffer<GlobalUniforms> =
            UniformBuffer::empty(&display).unwrap();

        // TODO: keep shaders and textures in a HashMap
        let shader = load_shader(&display, "default").unwrap();
        let texture = load_texture(&display, "textures/stone.png").unwrap();

        let mesh_buffer_manager = MeshBufferManager::new(&display);
        let command_buffer = DrawCommandsIndicesBuffer::empty_dynamic(&display, 8192).unwrap();

        Self {
            display,
            global_uniform_buffer,
            shader,
            texture,
            mesh_buffer_manager,
            command_buffer,
        }
    }

    pub fn render(
        &mut self,
        camera: &mut Camera,
        draw_calls: &Vec<DrawCall>,
        view_matrix: [[f32; 4]; 4],
    ) {
        let draw_params = glium::DrawParameters {
            depth: glium::Depth {
                test: glium::draw_parameters::DepthTest::IfLess,
                write: true,
                ..Default::default()
            },
            backface_culling: glium::BackfaceCullingMode::CullClockwise,
            ..Default::default()
        };

        let mut commands: Vec<DrawCommandIndices> = Vec::new();
        for (i, draw_call) in draw_calls.iter().enumerate() {
            if let None = self.mesh_buffer_manager.get_handle(&draw_call.mesh) {
                self.mesh_buffer_manager.load_mesh(&draw_call.mesh);
            }
            let handle = self
                .mesh_buffer_manager
                .get_handle(&draw_call.mesh)
                .unwrap();

            commands.push(DrawCommandIndices {
                count: handle.ibo_len as u32,
                instance_count: 1,
                first_index: handle.ibo_start as u32,
                base_vertex: handle.vbo_start as u32,
                base_instance: 0,
            });
        }

        if commands.len() == 0 {
            return;
        }

        self.command_buffer.invalidate();
        let cb_slice = self.command_buffer.slice_mut(0..commands.len()).unwrap();
        cb_slice.write(&commands);

        let mut target: Frame = self.display.draw();
        target.clear_color_and_depth((0.549, 0.745, 0.839, 1.0), 1.0);

        let (width, height) = target.get_dimensions();
        camera.aspect_ratio = width as f32 / height as f32;

        let global_uniforms = GlobalUniforms {
            projection_matrix: camera.projection_matrix(),
            view_matrix: view_matrix,
            light: [-1.0, 0.4, 0.9f32],
        };
        self.global_uniform_buffer.write(&global_uniforms);

        target
            .draw(
                self.mesh_buffer_manager
                    .allocator
                    .vbo
                    .slice(0..self.mesh_buffer_manager.allocator.vbo_pos)
                    .unwrap(),
                self.command_buffer
                    .with_index_buffer(&self.mesh_buffer_manager.allocator.ibo),
                &self.shader,
                &uniform! {
                    model_matrix: [
                                [ 1.0, 0.0, 0.0, 0.0],
                                [ 0.0, 1.0, 0.0, 0.0],
                                [ 0.0, 0.0, 1.0, 0.0],
                                [ 0.0, 0.0, 0.0, 1.0_f32],
                    ],
                    tex: Sampler(&self.texture, SamplerBehavior {
                        minify_filter: MinifySamplerFilter::NearestMipmapLinear,
                        magnify_filter: MagnifySamplerFilter::Nearest,
                        ..Default::default()
                    }),
                    global_render_uniforms: &self.global_uniform_buffer
                },
                &draw_params,
            )
            .unwrap();

        target.finish().unwrap();
    }
}

#[derive(Clone, Copy)]
struct GlobalUniforms {
    projection_matrix: [[f32; 4]; 4],
    view_matrix: [[f32; 4]; 4],
    light: [f32; 3],
}
implement_uniform_block!(GlobalUniforms, projection_matrix, view_matrix, light);

struct MeshBufferManager {
    allocator: MeshBufferAllocator,
    mesh_handles: HashMap<Uuid, MeshBufferHandle>,
}

impl MeshBufferManager {
    fn new(display: &Display) -> Self {
        let allocator = MeshBufferAllocator::new(display);
        Self {
            allocator,
            mesh_handles: HashMap::new(),
        }
    }

    fn load_mesh(&mut self, mesh: &Mesh) {
        let handle = self.allocator.allocate_mesh(mesh);

        let (vbo, ibo) = self.allocator.slice(&handle);
        vbo.write(&mesh.vertices);
        ibo.write(&mesh.triangles);

        self.mesh_handles.insert(mesh.id, handle);
    }

    fn get_handle<'a>(&'a self, mesh: &Mesh) -> Option<&'a MeshBufferHandle> {
        self.mesh_handles.get(&mesh.id)
    }
}

struct MeshBufferHandle {
    vbo_start: usize,
    ibo_start: usize,
    vbo_len: usize,
    ibo_len: usize,
}

struct MeshBufferAllocator {
    vbo: VertexBuffer<Vertex>,
    vbo_pos: usize,
    ibo: IndexBuffer<u32>,
    ibo_pos: usize,
}

impl MeshBufferAllocator {
    fn new(display: &Display) -> Self {
        let size = 65536 * 128;
        let vbo = VertexBuffer::empty_dynamic(display, size).unwrap();
        let ibo =
            IndexBuffer::empty_dynamic(display, PrimitiveType::TrianglesList, size * 3).unwrap();

        Self {
            vbo,
            vbo_pos: 0,
            ibo,
            ibo_pos: 0,
        }
    }

    fn allocate_mesh<'a>(&mut self, mesh: &Mesh) -> MeshBufferHandle {
        if self.vbo_pos + mesh.vertices.len() >= self.vbo.len()
            || self.ibo_pos + mesh.triangles.len() >= self.ibo.len()
        {
            panic!("out of mesh buffer memory");
        }

        let vbo_start = self.vbo_pos;
        self.vbo_pos += mesh.vertices.len();

        let ibo_start = self.ibo_pos;
        self.ibo_pos += 3 * mesh.vertices.len();

        MeshBufferHandle {
            vbo_start,
            ibo_start,
            vbo_len: mesh.vertices.len(),
            ibo_len: mesh.triangles.len(),
        }
    }

    fn slice<'a>(
        &'a self,
        handle: &MeshBufferHandle,
    ) -> (VertexBufferSlice<'a, Vertex>, IndexBufferSlice<'a, u32>) {
        (
            self.vbo
                .slice(handle.vbo_start..handle.vbo_start + handle.vbo_len)
                .unwrap(),
            self.ibo
                .slice(handle.ibo_start..handle.ibo_start + handle.ibo_len)
                .unwrap(),
        )
    }
}

const MAX_VBO_SIZE: usize = 65536;
const MAX_BUFFERS: usize = 128;

struct MeshBuffer {
    vbos: VecDeque<VertexBuffer<Vertex>>,
    vbo_index: usize,
    ibos: VecDeque<IndexBuffer<u32>>,
    ibo_index: usize,
    mesh_locator: HashMap<Uuid, (usize, usize, usize)>,
}

impl MeshBuffer {
    pub fn new(display: &Display) -> MeshBuffer {
        let mut mesh_buffer = Self {
            vbos: VecDeque::new(),
            vbo_index: 0,
            ibos: VecDeque::new(),
            ibo_index: 0,
            mesh_locator: HashMap::new(),
        };
        mesh_buffer.allocate_buffers(display);
        mesh_buffer
    }

    pub fn load_mesh(&mut self, display: &Display, mesh: &Mesh) {
        let (vbo, ibo) = self.last_buffers();
        let (vbo_start, vbo_end) = (self.vbo_index, self.vbo_index + mesh.vertices.len());
        let (ibo_start, ibo_end) = (self.ibo_index, self.ibo_index + mesh.triangles.len());

        // If the current buffer is out of space, we need to allocate a new one
        if vbo_end >= vbo.len() || ibo_end >= ibo.len() {
            self.allocate_buffers(display);
            self.load_mesh(display, mesh);
            return;
        }

        let (vbo, ibo) = self.last_buffers_mut();
        vbo.slice_mut(vbo_start..vbo_end)
            .unwrap()
            .write(&mesh.vertices);
        ibo.slice_mut(ibo_start..ibo_end)
            .unwrap()
            .write(&mesh.triangles);

        self.mesh_locator.insert(
            mesh.id,
            (self.vbos.len() - 1, self.vbo_index, self.ibo_index),
        );
        self.vbo_index += mesh.vertices.len();
        self.ibo_index += mesh.triangles.len();
    }

    pub fn mesh_buffer_slice<'a>(
        &'a self,
        mesh: &Mesh,
    ) -> Option<(VertexBufferSlice<'a, Vertex>, IndexBufferSlice<'a, u32>)> {
        let (buffer, vbo_start, ibo_start) = self.mesh_locator.get(&mesh.id)?.to_owned();
        Some((
            self.vbos[buffer]
                .slice(vbo_start..vbo_start + mesh.vertices.len())
                .unwrap(),
            self.ibos[buffer]
                .slice(ibo_start..ibo_start + mesh.triangles.len())
                .unwrap(),
        ))
    }

    fn last_buffers_mut<'a>(
        &'a mut self,
    ) -> (&'a mut VertexBuffer<Vertex>, &'a mut IndexBuffer<u32>) {
        let (vbo_pos, ibo_pos) = (self.vbos.len() - 1, self.ibos.len() - 1);
        (&mut self.vbos[vbo_pos], &mut self.ibos[ibo_pos])
    }

    fn last_buffers<'a>(&'a self) -> (&'a VertexBuffer<Vertex>, &'a IndexBuffer<u32>) {
        let (vbo_pos, ibo_pos) = (self.vbos.len() - 1, self.ibos.len() - 1);
        (&self.vbos[vbo_pos], &self.ibos[ibo_pos])
    }

    fn allocate_buffers(&mut self, display: &Display) {
        let (vbo, ibo) = if self.vbos.len() == MAX_BUFFERS {
            (
                self.vbos.pop_front().unwrap(),
                self.ibos.pop_front().unwrap(),
            )
        } else {
            (
                VertexBuffer::empty_dynamic(display, MAX_VBO_SIZE).unwrap(),
                IndexBuffer::empty_dynamic(display, PrimitiveType::TrianglesList, MAX_VBO_SIZE * 3)
                    .unwrap(),
            )
        };

        self.vbos.push_back(vbo);
        self.ibos.push_back(ibo);
        self.vbo_index = 0;
        self.ibo_index = 0;
    }
}
