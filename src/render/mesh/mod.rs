use glium::{
    uniforms::{AsUniformValue, UniformBuffer, Uniforms, UniformsStorage},
    Display, DrawError, DrawParameters, Frame, IndexBuffer, Program, Surface, VertexBuffer,
};

use crate::world::Vector3;

pub mod primitives;

/// A `Vertex` is represented by a 3D position.
#[derive(Copy, Clone)]
pub struct Vertex {
    position: [f32; 3],
}
implement_vertex!(Vertex, position);

impl From<Vector3> for Vertex {
    fn from(v: Vector3) -> Self {
        Vertex {
            position: [v.x, v.y, v.z],
        }
    }
}

/// A normal is represented by a 3D direction vector.
#[derive(Copy, Clone)]
pub struct Normal {
    normal: [f32; 3],
}
implement_vertex!(Normal, normal);

impl From<Vector3> for Normal {
    fn from(v: Vector3) -> Self {
        Normal {
            normal: [v.x, v.y, v.z],
        }
    }
}

/// An abstract representation of a model by its vertices, normals and indices.
///
/// Simply a store of model data that must be loaded onto the GPU for rendering.
pub struct MeshData {
    pub vertices: Vec<Vector3>,
    pub normals: Vec<Vector3>,
    pub indices: Vec<u32>,
}

impl MeshData {
    /// Creates new `MeshData` from a list of vertices, normals and indices.
    pub fn new(vertices: Vec<Vector3>, normals: Vec<Vector3>, indices: Vec<u32>) -> MeshData {
        MeshData {
            vertices,
            normals,
            indices,
        }
    }

    /// Loads this `MeshData` onto the GPU and returns a `Mesh` that can be rendered to the screen.
    ///
    /// Returns a `MeshLoadError` if any part of the model failed to load.
    pub fn load(&self, display: &Display) -> Result<Mesh, MeshLoadError> {
        let vertices: Vec<Vertex> = self.vertices.iter().map(|v| Vertex::from(*v)).collect();
        let normals: Vec<Normal> = self.normals.iter().map(|v| Normal::from(*v)).collect();
        let (vertex_buffer, normal_buffer, index_buffer) = (
            glium::VertexBuffer::new(display, &vertices)?,
            glium::VertexBuffer::new(display, &normals)?,
            glium::IndexBuffer::new(
                display,
                glium::index::PrimitiveType::TrianglesList,
                &self.indices,
            )?,
        );

        Ok(Mesh {
            vertex_buffer,
            normal_buffer,
            index_buffer,
        })
    }
}

/// A representation of a model that has been loaded onto the GPU.
/// This model can be rendered.
pub struct Mesh {
    vertex_buffer: VertexBuffer<Vertex>,
    normal_buffer: VertexBuffer<Normal>,
    index_buffer: IndexBuffer<u32>,
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

impl<T, R> Renderable<T, R> for Mesh {
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
