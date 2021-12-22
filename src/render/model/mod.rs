use std::io::Empty;

use glium::{
    buffer::Content,
    program::Uniform,
    uniforms::{
        AsUniformValue, EmptyUniforms, UniformBlock, UniformBuffer, UniformValue, Uniforms,
        UniformsStorage,
    },
    Display, DrawError, DrawParameters, Frame, IndexBuffer, Program, Surface, VertexBuffer,
};

pub mod primitives;

/// A `Vertex` is represented by a 3D position.
#[derive(Copy, Clone)]
pub struct Vertex {
    position: [f32; 3],
}
implement_vertex!(Vertex, position);

/// A normal is represented by a 3D direction vector.
#[derive(Copy, Clone)]
pub struct Normal {
    normal: [f32; 3],
}
implement_vertex!(Normal, normal);

#[macro_export]
macro_rules! vertex {
    ( $x:expr,$y:expr,$z:expr ) => {
        Vertex {
            position: [$x, $y, $z],
        }
    };
}

#[macro_export]
macro_rules! normal {
    ( $x:expr,$y:expr,$z:expr ) => {
        Normal {
            normal: [$x, $y, $z],
        }
    };
}

/// An abstract representation of a model by its vertices, normals and indices.
///
/// Simply a store of model data that must be loaded onto the GPU for rendering.
pub struct ModelData {
    pub vertices: Vec<Vertex>,
    pub normals: Vec<Normal>,
    pub indices: Vec<u32>,
}

impl ModelData {
    /// Creates new `ModelData` from a list of vertices, normals and indices.
    pub fn new(vertices: Vec<Vertex>, normals: Vec<Normal>, indices: Vec<u32>) -> ModelData {
        ModelData {
            vertices,
            normals,
            indices,
        }
    }

    /// Loads this `ModelData` onto the GPU and returns a `Model` that can be rendered to the screen.
    ///
    /// Returns a `ModelLoadError` if any part of the model failed to load.
    pub fn load(&self, display: &Display) -> Result<Model, ModelLoadError> {
        let (vertex_buffer, normal_buffer, index_buffer) = (
            glium::VertexBuffer::new(display, &self.vertices)?,
            glium::VertexBuffer::new(display, &self.normals)?,
            glium::IndexBuffer::new(
                display,
                glium::index::PrimitiveType::TrianglesList,
                &self.indices,
            )?,
        );

        Ok(Model {
            vertex_buffer,
            normal_buffer,
            index_buffer,
        })
    }
}

/// A representation of a model that has been loaded onto the GPU.
/// This model can be rendered.
pub struct Model {
    vertex_buffer: VertexBuffer<Vertex>,
    normal_buffer: VertexBuffer<Normal>,
    index_buffer: IndexBuffer<u32>,
}

/// Represents the errors that can occur when loading `ModelData` onto the GPU.
#[derive(Debug)]
pub enum ModelLoadError {
    VertexBufferCreationError(glium::vertex::BufferCreationError),
    IndexBufferCreationError(glium::index::BufferCreationError),
}

/// Conversion traits from `BufferCreationError` types to `ModelLoadError`
impl From<glium::vertex::BufferCreationError> for ModelLoadError {
    fn from(err: glium::vertex::BufferCreationError) -> Self {
        ModelLoadError::VertexBufferCreationError(err)
    }
}

impl From<glium::index::BufferCreationError> for ModelLoadError {
    fn from(err: glium::index::BufferCreationError) -> Self {
        ModelLoadError::IndexBufferCreationError(err)
    }
}

#[derive(Copy, Clone)]
pub struct GlobalRenderUniforms {
    pub projection_matrix: [[f32; 4]; 4],
    pub view_matrix: [[f32; 4]; 4],
    pub light: [f32; 3],
}
implement_uniform_block!(GlobalRenderUniforms, projection_matrix, view_matrix, light);

type RenderUniforms<'a, T, R> =
    UniformsStorage<'a, &'a UniformBuffer<GlobalRenderUniforms>, UniformsStorage<'a, T, R>>;

/// A trait for anything that can be rendered to the screen.
///
/// Takes in the frame to render to, the shader program to render with, a set of uniforms to run the shader with, and additional draw parameters.
pub trait Renderable<T, R> {
    fn render(
        &self,
        target: &mut Frame,
        program: &Program,
        uniforms: &RenderUniforms<T, R>,
        params: &DrawParameters,
    ) -> Result<(), DrawError>
    where
        T: AsUniformValue,
        R: Uniforms;
}

/// Implement the `Renderable` trait for `Model`.
///
/// This provides an implementation for how any `Model` should be rendered to the screen.
impl<T, R> Renderable<T, R> for Model {
    fn render(
        &self,
        target: &mut Frame,
        program: &Program,
        uniforms: &RenderUniforms<T, R>,
        params: &DrawParameters,
    ) -> Result<(), DrawError>
    where
        T: AsUniformValue,
        R: Uniforms,
    {
        target.draw(
            (&self.vertex_buffer, &self.normal_buffer),
            &self.index_buffer,
            program,
            uniforms,
            params,
        )
    }
}
