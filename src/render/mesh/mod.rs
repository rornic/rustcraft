use std::collections::{HashMap, VecDeque};

use glium::texture::SrgbTexture2d;
use glium::uniforms::{
    MagnifySamplerFilter, MinifySamplerFilter, Sampler, SamplerBehavior, UniformBuffer,
};
use glium::{
    uniforms::UniformsStorage,
    vertex::{Attribute, AttributeType},
    Display, Frame, IndexBuffer, Program, Surface, VertexBuffer,
};
use specs::WorldExt;
use specs::Write;
use specs::{
    shred::ResourceId, Component, Join, Read, ReadExpect, ReadStorage, System, SystemData,
    VecStorage, World,
};
use uuid::Uuid;

use crate::world::{Transform, Vector2, Vector3};

pub mod primitives;

/// A `Vertex` is represented by a 3D position.
#[derive(Copy, Clone)]
pub struct Vertex {
    pub position: Vector3,
    pub normal: Vector3,
    pub uv: Vector2,
}
implement_vertex!(Vertex, position, normal, uv);

/// Implement `Attribute` for `Vector3` so that we can use it as a `Vertex` attribute on the GPU. Maps it to an `F32F32F32` or `vec3` type.
unsafe impl Attribute for Vector3 {
    fn get_type() -> glium::vertex::AttributeType {
        AttributeType::F32F32F32
    }
}

/// Implement `Attribute` for `Vector2` so that we can use it as a `Vertex` attribute on the GPU. Maps it to an `F32F32` or `vec2` type.
unsafe impl Attribute for Vector2 {
    fn get_type() -> glium::vertex::AttributeType {
        AttributeType::F32F32
    }
}

#[macro_export]
macro_rules! vertex {
    ( position: $position:expr, normal: $normal:expr, uv: $uv:expr) => {
        Vertex {
            position: $position,
            normal: $normal,
            uv: $uv,
        }
    };
}

/// An abstract representation of a model by its vertices, normals and indices.
///
/// Simply a store of model data that must be loaded onto the GPU for rendering.
pub struct Mesh {
    pub mesh_id: Uuid,
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

impl Mesh {
    /// Creates new `Mesh` from a list of vertices and indices.
    pub fn new(vertices: Vec<Vertex>, indices: Vec<u32>) -> Mesh {
        Mesh {
            mesh_id: uuid::Uuid::new_v4(),
            vertices,
            indices,
        }
    }
}

/// Represents uniforms that are global across all shaders and should be present for every render.
///
/// This includes information required to project from model space to screen space as well as calculating lighting.
#[derive(Copy, Clone)]
pub struct GlobalRenderUniforms {
    pub projection_matrix: [[f32; 4]; 4],
    pub view_matrix: [[f32; 4]; 4],
    pub light: [f32; 3],
}
implement_uniform_block!(GlobalRenderUniforms, projection_matrix, view_matrix, light);

/// A `UniformStorage` type that contains a `VertexBuffer<GlobalRenderUniforms>` alongside any extra uniforms for this render.
type RenderUniforms<'a, T, R> =
    UniformsStorage<'a, &'a UniformBuffer<GlobalRenderUniforms>, UniformsStorage<'a, T, R>>;

/// Component for entities that a `Mesh` should be rendered for.
///
/// Does not store the actual `Mesh` data, but just a reference to a `Mesh` that has been loaded into the `RenderingSystem`.
pub struct RenderMesh {
    mesh_id: Uuid,
}

impl RenderMesh {
    pub fn new(mesh: &Mesh) -> RenderMesh {
        RenderMesh {
            mesh_id: mesh.mesh_id,
        }
    }
}

impl Component for RenderMesh {
    type Storage = VecStorage<Self>;
}

pub struct ViewMatrix([[f32; 4]; 4]);

impl Default for ViewMatrix {
    fn default() -> Self {
        ViewMatrix([
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0_f32],
        ])
    }
}

/// The `RenderingSystem` produces `DrawCall` structs in parallel. These are handled by the `Renderer` which runs on the main thread.
pub struct RenderingSystem;

impl<'a> System<'a> for RenderingSystem {
    type SystemData = (
        ReadStorage<'a, Transform>,
        ReadStorage<'a, RenderMesh>,
        Write<'a, VecDeque<DrawCall>>,
    );

    /// Produce a `DrawCall` for every entity with both a `Transform` and `RenderMesh` component. TODO: Batch entities using the same mesh into a single `DrawCall`.
    fn run(&mut self, (transforms, render_meshes, mut draw_calls): Self::SystemData) {
        for (transform, mesh_data) in (&transforms, &render_meshes).join() {
            let model_matrix = transform.matrix();

            draw_calls.push_back(DrawCall {
                model_matrix: model_matrix,
                mesh_id: mesh_data.mesh_id,
            });
        }
    }
}

pub struct DrawCall {
    model_matrix: [[f32; 4]; 4],
    mesh_id: Uuid,
}

/// The `Renderer` receives `DrawCall` structs and processes each of them into a draw call on the GPU.
///
/// Takes a `Display` to draw to. The `Renderer` keeps track of resources loaded onto the GPU.
pub struct Renderer {
    display: Display,
    global_uniform_buffer: UniformBuffer<GlobalRenderUniforms>,
    shader_program: Program,
    texture: SrgbTexture2d,
    mesh_register: HashMap<Uuid, (VertexBuffer<Vertex>, IndexBuffer<u32>)>,
}

impl Renderer {
    pub fn new(display: Display) -> Self {
        // Create a buffer for global uniforms
        let global_uniform_buffer: UniformBuffer<GlobalRenderUniforms> =
            UniformBuffer::empty(&display).unwrap();

        // Create the shader program
        // TODO: Store shader id in `RenderMesh` component and keep track of shaders in `RenderingSystem`.
        let shader_program = super::shader::load_shader(&display, "default").unwrap();

        // Create the texture
        // TODO: Store texture id in `RenderMesh` component and keep track of textures in `RenderingSystem`.
        let texture = super::texture::load_texture(&display, "textures/stone.png").unwrap();

        Self {
            display,
            global_uniform_buffer,
            shader_program,
            texture,
            mesh_register: HashMap::new(),
        }
    }

    /// Loads a mesh onto the GPU, centrally mapping its UUID to its `VertexBuffer` and `IndexBuffer`.
    pub fn register_mesh(&mut self, mesh: &Mesh) -> Result<(), MeshLoadError> {
        let mesh_data = (
            glium::VertexBuffer::new(&self.display, &mesh.vertices)?,
            glium::IndexBuffer::new(
                &self.display,
                glium::index::PrimitiveType::TrianglesList,
                &mesh.indices,
            )?,
        );

        self.mesh_register.insert(mesh.mesh_id, mesh_data);

        Ok(())
    }

    pub fn render(&mut self, world: &mut World) {
        // Start drawing on window
        let mut target: Frame = self.display.draw();
        target.clear_color_and_depth((0.01, 0.01, 0.01, 1.0), 1.0);

        // Set up draw parameters
        let params = glium::DrawParameters {
            depth: glium::Depth {
                test: glium::draw_parameters::DepthTest::IfLess,
                write: true,
                ..Default::default()
            },
            ..Default::default()
        };

        let projection_matrix = {
            let (width, height) = target.get_dimensions();
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
        };

        let view_matrix = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0_f32],
        ];

        // Update global uniforms
        let global_render_uniforms = GlobalRenderUniforms {
            projection_matrix: projection_matrix,
            view_matrix: view_matrix,
            light: [-1.0, 0.4, 0.9f32],
        };
        self.global_uniform_buffer.write(&global_render_uniforms);

        // Empty the draw call queue
        while let Some(draw_call) = world.write_resource::<VecDeque<DrawCall>>().pop_front() {
            if let Some((vertex_buffer, index_buffer)) = self.mesh_register.get(&draw_call.mesh_id)
            {
                target
                    .draw(
                        vertex_buffer,
                        index_buffer,
                        &self.shader_program,
                        &uniform! {
                            model_matrix: draw_call.model_matrix,
                            tex: Sampler(&self.texture, SamplerBehavior {
                                minify_filter: MinifySamplerFilter::Nearest,
                                magnify_filter: MagnifySamplerFilter::Nearest,
                                ..Default::default()
                            }),
                            global_render_uniforms: &self.global_uniform_buffer
                        },
                        &params,
                    )
                    .unwrap();
            }
        }

        target.finish().unwrap();
    }
}

/// Represents the errors that can occur when loading `MeshData` onto the GPU.
#[derive(Debug)]
pub enum MeshLoadError {
    VertexBufferCreationError(glium::vertex::BufferCreationError),
    IndexBufferCreationError(glium::index::BufferCreationError),
}

/// Conversion traits from `BufferCreationError` types to `MeshLoadError`
impl From<glium::vertex::BufferCreationError> for MeshLoadError {
    fn from(err: glium::vertex::BufferCreationError) -> Self {
        MeshLoadError::VertexBufferCreationError(err)
    }
}

impl From<glium::index::BufferCreationError> for MeshLoadError {
    fn from(err: glium::index::BufferCreationError) -> Self {
        MeshLoadError::IndexBufferCreationError(err)
    }
}
