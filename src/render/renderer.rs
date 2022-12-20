use std::{collections::HashMap, sync::Arc};

use cgmath::{InnerSpace, Vector3, Zero};
use glium::{
    index::{IndexBufferSlice, PrimitiveType},
    texture::SrgbTexture2d,
    uniforms::{
        MagnifySamplerFilter, MinifySamplerFilter, Sampler, SamplerBehavior, UniformBuffer,
    },
    vertex::VertexBufferSlice,
    Display, Frame, IndexBuffer, Program, Surface, VertexBuffer,
};
use specs::{Component, Join, ReadStorage, System, VecStorage, Write};
use uuid::Uuid;

use crate::{vector3, world::ecs::Transform, DrawCalls};

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
        ReadStorage<'a, Transform>,
        ReadStorage<'a, RenderMesh>,
        Write<'a, DrawCalls>,
    );

    /// Produce a `DrawCall` for every entity with both a `Transform` and `RenderMesh` component. TODO: Batch entities using the same mesh into a single `DrawCall`.
    fn run(&mut self, (transforms, render_meshes, mut draw_calls): Self::SystemData) {
        draw_calls.0.clear();
        for (transform, mesh_data) in (&transforms, &render_meshes).join() {
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
    mesh_buffer: MeshBuffer,
}

impl Renderer {
    pub fn new(display: Display) -> Self {
        let global_uniform_buffer: UniformBuffer<GlobalUniforms> =
            UniformBuffer::empty(&display).unwrap();

        // TODO: keep shaders and textures in a HashMap
        let shader = load_shader(&display, "default").unwrap();
        let texture = load_texture(&display, "textures/stone.png").unwrap();

        let mut mesh_buffer = MeshBuffer::new(&display);

        Self {
            display,
            global_uniform_buffer,
            shader,
            texture,
            mesh_buffer,
        }
    }

    pub fn render(&mut self, draw_calls: &Vec<DrawCall>, view_matrix: [[f32; 4]; 4]) {
        let mut target: Frame = self.display.draw();
        target.clear_color_and_depth((0.5, 0.5, 0.5, 1.0), 1.0);

        let draw_params = glium::DrawParameters {
            depth: glium::Depth {
                test: glium::draw_parameters::DepthTest::IfLess,
                write: true,
                ..Default::default()
            },
            backface_culling: glium::BackfaceCullingMode::CullClockwise,
            ..Default::default()
        };

        let (width, height) = target.get_dimensions();
        let projection_matrix = projection_matrix(width, height);

        let global_uniforms = GlobalUniforms {
            projection_matrix: projection_matrix,
            view_matrix: view_matrix,
            light: [-1.0, 0.4, 0.9f32],
        };
        self.global_uniform_buffer.write(&global_uniforms);

        for draw_call in draw_calls {
            if !self
                .mesh_buffer
                .mesh_locator
                .contains_key(&draw_call.mesh.id)
            {
                self.mesh_buffer.load_mesh(&self.display, &draw_call.mesh);
            }

            let (vbo, ibo) = self.mesh_buffer.mesh_buffer_slice(&draw_call.mesh).unwrap();

            target
                .draw(
                    vbo,
                    ibo,
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

fn projection_matrix(width: u32, height: u32) -> [[f32; 4]; 4] {
    let aspect_ratio = height as f32 / width as f32;

    let fov: f32 = 3.141592 / 3.0;
    let zfar = 1024.0;
    let znear = 0.1;

    let f = 1.0 / (fov / 2.0).tan();

    [
        [f * aspect_ratio, 0.0, 0.0, 0.0],
        [0.0, f, 0.0, 0.0],
        [0.0, 0.0, (zfar + znear) / (zfar - znear), 1.0],
        [0.0, 0.0, -(2.0 * zfar * znear) / (zfar - znear), 0.0],
    ]
}

pub struct ViewMatrix(pub [[f32; 4]; 4]);

impl Default for ViewMatrix {
    fn default() -> Self {
        view_matrix(
            Vector3::zero(),
            vector3!(0.0, 0.0, 1.0),
            vector3!(0.0, 1.0, 0.0),
        )
    }
}

pub fn view_matrix(
    position: Vector3<f32>,
    direction: Vector3<f32>,
    up: Vector3<f32>,
) -> ViewMatrix {
    let direction = direction.normalize();
    let s = vector3!(
        up.y * direction.z - up.z * direction.y,
        up.z * direction.x - up.x * direction.z,
        up.x * direction.y - up.y * direction.x
    )
    .normalize();
    let u = vector3!(
        direction.y * s.z - direction.z * s.y,
        direction.z * s.x - direction.x * s.z,
        direction.x * s.y - direction.y * s.x
    );

    let p = vector3!(
        -position.x * s.x - position.y * s.y - position.z * s.z,
        -position.x * u.x - position.y * u.y - position.z * u.z,
        -position.x * direction.x - position.y * direction.y - position.z * direction.z
    );

    ViewMatrix([
        [s.x, u.x, direction.x, 0.0],
        [s.y, u.y, direction.y, 0.0],
        [s.z, u.z, direction.z, 0.0],
        [p.x, p.y, p.z, 1.0],
    ])
}

#[derive(Clone, Copy)]
struct GlobalUniforms {
    projection_matrix: [[f32; 4]; 4],
    view_matrix: [[f32; 4]; 4],
    light: [f32; 3],
}
implement_uniform_block!(GlobalUniforms, projection_matrix, view_matrix, light);

struct MeshBuffer {
    vbos: Vec<VertexBuffer<Vertex>>,
    vbo_index: usize,
    ibos: Vec<IndexBuffer<u32>>,
    ibo_index: usize,
    mesh_locator: HashMap<Uuid, (usize, usize, usize)>,
}

impl MeshBuffer {
    pub fn new(display: &Display) -> MeshBuffer {
        let mut mesh_buffer = Self {
            vbos: vec![],
            vbo_index: 0,
            ibos: vec![],
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
        let vbo = VertexBuffer::empty_dynamic(display, 4096).unwrap();
        let ibo =
            IndexBuffer::empty_dynamic(display, PrimitiveType::TrianglesList, 4096 * 3).unwrap();
        self.vbos.push(vbo);
        self.ibos.push(ibo);
        self.vbo_index = 0;
        self.ibo_index = 0;
    }
}
