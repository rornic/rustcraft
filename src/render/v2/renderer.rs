use cgmath::{InnerSpace, Vector3};
use glium::{
    index::{Index, PrimitiveType},
    texture::SrgbTexture2d,
    uniforms::{
        MagnifySamplerFilter, MinifySamplerFilter, Sampler, SamplerBehavior, UniformBuffer,
    },
    Display, DrawParameters, Frame, IndexBuffer, Program, Surface, VertexBuffer,
};

use crate::{vector3, world::ecs::Transform};

use super::{
    material::Material,
    mesh::{Mesh, Vertex},
    primitives,
};

pub struct DrawCall {
    material: Material,
    mesh: Mesh,
    transform: Transform,
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
        let shader = crate::render::shader::load_shader(&display, "default").unwrap();
        let texture = crate::render::texture::load_texture(&display, "textures/stone.png").unwrap();

        let mut mesh_buffer = MeshBuffer::new(&display);
        mesh_buffer.load_mesh(&primitives::cube());

        Self {
            display,
            global_uniform_buffer,
            shader,
            texture,
            mesh_buffer,
        }
    }

    pub fn render(&self, draw_calls: Vec<DrawCall>, view_matrix: [[f32; 4]; 4]) {
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

        target
            .draw(
                &self.mesh_buffer.vbo,
                &self.mesh_buffer.ibo,
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

pub fn view_matrix(
    position: Vector3<f32>,
    direction: Vector3<f32>,
    up: Vector3<f32>,
) -> [[f32; 4]; 4] {
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

    [
        [s.x, u.x, direction.x, 0.0],
        [s.y, u.y, direction.y, 0.0],
        [s.z, u.z, direction.z, 0.0],
        [p.x, p.y, p.z, 1.0],
    ]
}

#[derive(Clone, Copy)]
struct GlobalUniforms {
    projection_matrix: [[f32; 4]; 4],
    view_matrix: [[f32; 4]; 4],
    light: [f32; 3],
}
implement_uniform_block!(GlobalUniforms, projection_matrix, view_matrix, light);

struct MeshBuffer {
    vbo: VertexBuffer<Vertex>,
    ibo: IndexBuffer<u32>,
}

impl MeshBuffer {
    pub fn new(display: &Display) -> MeshBuffer {
        let vbo = VertexBuffer::empty_dynamic(display, 1024).unwrap();
        let ibo =
            IndexBuffer::empty_dynamic(display, PrimitiveType::TrianglesList, 1024 * 3).unwrap();
        Self { vbo, ibo }
    }

    pub fn load_mesh(&mut self, mesh: &Mesh) {
        self.vbo
            .slice_mut(0..mesh.vertices.len())
            .unwrap()
            .write(&mesh.vertices);

        self.ibo
            .slice_mut(0..mesh.triangles.len())
            .unwrap()
            .write(&mesh.triangles);
    }
}
