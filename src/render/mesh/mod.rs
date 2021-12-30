use cgmath::{Vector2, Vector3};
use glium::{
    vertex::{Attribute, AttributeType},
    Display, IndexBuffer, VertexBuffer,
};
use uuid::Uuid;

pub mod primitives;

/// A `Vertex` is represented by a 3D position, normal and 2D UV position.
#[derive(Copy, Clone)]
pub struct Vertex {
    pub position: Vector3<f32>,
    pub normal: Vector3<f32>,
    pub uv: Vector2<f32>,
}

/// Uses an internal `GpuVertex` struct to get the type bindings that a `Vertex` will use on the GPU.
impl glium::Vertex for Vertex {
    fn build_bindings() -> glium::VertexFormat {
        GpuVertex::build_bindings()
    }
}

/// Represents a `Vertex` as it should be laid out on the GPU.
#[derive(Clone, Copy)]
struct GpuVertex {
    position: [f32; 3],
    normal: [f32; 3],
    uv: [f32; 2],
}
implement_vertex!(GpuVertex, position, normal, uv);

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

    pub fn load(
        &self,
        display: &Display,
    ) -> Result<(VertexBuffer<Vertex>, IndexBuffer<u32>), MeshLoadError> {
        let mesh_data = (
            glium::VertexBuffer::new(display, &self.vertices)?,
            glium::IndexBuffer::new(
                display,
                glium::index::PrimitiveType::TrianglesList,
                &self.indices,
            )?,
        );

        Ok(mesh_data)
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
