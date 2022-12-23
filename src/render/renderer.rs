use std::{collections::HashMap, sync::Arc};

use glium::{
    index::{DrawCommandsIndicesBuffer, IndexBufferSlice, PrimitiveType},
    texture::SrgbTexture2d,
    uniforms::{
        MagnifySamplerFilter, MinifySamplerFilter, Sampler, SamplerBehavior, UniformBuffer,
    },
    vertex::VertexBufferSlice,
    Display, Frame, IndexBuffer, Program, Surface, VertexBuffer,
};
use specs::{Component, Join, ReadStorage, System, VecStorage, Write};
use uuid::Uuid;

use crate::{
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
    mesh_heap: MeshHeap,
}

impl Renderer {
    pub fn new(display: Display) -> Self {
        let global_uniform_buffer: UniformBuffer<GlobalUniforms> =
            UniformBuffer::empty(&display).unwrap();

        // TODO: keep shaders and textures in a HashMap
        let shader = load_shader(&display, "default").unwrap();
        let texture = load_texture(&display, "textures/stone.png").unwrap();

        let mesh_heap = MeshHeap::new();

        Self {
            display,
            global_uniform_buffer,
            shader,
            texture,
            mesh_heap,
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

        for draw_call in draw_calls.iter() {
            if !self
                .mesh_heap
                .loaded_meshes
                .contains_key(&draw_call.mesh.id)
            {
                self.mesh_heap.load_mesh(&self.display, &draw_call.mesh);
            }
        }

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

        for mesh_buffer in &self.mesh_heap.mesh_buffers {
            target
                .draw(
                    mesh_buffer.vbo.slice(0..mesh_buffer.vbo_pos).unwrap(),
                    mesh_buffer.ibo.slice(0..mesh_buffer.ibo_pos).unwrap(),
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
        }

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

const MAX_VBO_SIZE: usize = 65536 * 4;
struct MeshBuffer {
    vbo: VertexBuffer<Vertex>,
    vbo_pos: usize,
    vbo_free_space: Vec<MemoryBlock>,
    ibo: IndexBuffer<u32>,
    ibo_pos: usize,
    ibo_free_space: Vec<MemoryBlock>,
}

impl MeshBuffer {
    fn new(display: &Display) -> Self {
        let vbo = VertexBuffer::empty_dynamic(display, MAX_VBO_SIZE).unwrap();
        let ibo =
            IndexBuffer::empty_dynamic(display, PrimitiveType::TrianglesList, MAX_VBO_SIZE * 3)
                .unwrap();

        Self {
            vbo,
            vbo_pos: 0,
            vbo_free_space: Vec::new(),
            ibo,
            ibo_pos: 0,
            ibo_free_space: Vec::new(),
        }
    }

    fn allocate(&mut self, mesh: &Mesh) -> Option<MeshLocator> {
        if self.vbo_pos + mesh.vertices.len() >= MAX_VBO_SIZE
            || self.ibo_pos + mesh.triangles.len() >= MAX_VBO_SIZE * 3
        {
            return None;
        }

        let mesh_locator = MeshLocator {
            vertices: MemoryBlock {
                start: self.vbo_pos,
                size: mesh.vertices.len(),
            },
            triangles: MemoryBlock {
                start: self.ibo_pos,
                size: mesh.triangles.len(),
            },
        };

        self.vbo_pos += mesh.vertices.len();
        self.ibo_pos += mesh.triangles.len();

        Some(mesh_locator)
    }

    fn free(&mut self, locator: &MeshLocator) {
        self.free_vbo(locator.vertices);
        self.free_ibo(locator.triangles);
    }

    fn free_vbo(&mut self, mem: MemoryBlock) {
        self.slice_vbo(mem).invalidate();
        self.vbo_free_space.push(mem);
    }

    fn free_ibo(&mut self, mem: MemoryBlock) {
        self.slice_ibo(mem).invalidate();
        self.ibo_free_space.push(mem);
    }

    fn slice_vbo<'a>(&'a self, mem: MemoryBlock) -> VertexBufferSlice<'a, Vertex> {
        self.vbo.slice(mem.start..mem.start + mem.size).unwrap()
    }

    fn slice_ibo<'a>(&'a self, mem: MemoryBlock) -> IndexBufferSlice<'a, u32> {
        self.ibo.slice(mem.start..mem.start + mem.size).unwrap()
    }
}

struct MeshLocator {
    vertices: MemoryBlock,
    triangles: MemoryBlock,
}

#[derive(Clone, Copy)]
struct MemoryBlock {
    start: usize,
    size: usize,
}

struct MeshHeap {
    mesh_buffers: Vec<MeshBuffer>,
    loaded_meshes: HashMap<Uuid, (usize, MeshLocator)>,
}

impl MeshHeap {
    fn new() -> MeshHeap {
        MeshHeap {
            mesh_buffers: vec![],
            loaded_meshes: HashMap::new(),
        }
    }

    fn load_mesh(&mut self, display: &Display, mesh: &Mesh) {
        let (buf, locator) = self.allocate(display, mesh);

        self.mesh_buffers[buf]
            .slice_vbo(locator.vertices)
            .write(&mesh.vertices);

        let shifted_tris: Vec<u32> = mesh
            .triangles
            .iter()
            .map(|i| *i + locator.vertices.start as u32)
            .collect();
        self.mesh_buffers[buf]
            .slice_ibo(locator.triangles)
            .write(&shifted_tris);

        self.loaded_meshes.insert(mesh.id, (buf, locator));
    }

    fn free_mesh(&mut self, mesh: &Mesh) {}

    fn allocate(&mut self, display: &Display, mesh: &Mesh) -> (usize, MeshLocator) {
        for (i, buffer) in self.mesh_buffers.iter_mut().enumerate().rev() {
            if let Some(locator) = buffer.allocate(mesh) {
                return (i, locator);
            }
        }

        let locator = self.new_buffer(display).allocate(mesh).unwrap();
        (self.mesh_buffers.len() - 1, locator)
    }

    fn new_buffer(&mut self, display: &Display) -> &mut MeshBuffer {
        let buffer = MeshBuffer::new(display);
        self.mesh_buffers.push(buffer);
        self.mesh_buffers.last_mut().unwrap()
    }
}
