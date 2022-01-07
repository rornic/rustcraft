use std::{collections::HashMap, sync::Arc};

use cgmath::{Vector2, Vector3};
use glium::{index::PrimitiveType, Display, IndexBuffer, VertexBuffer};
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
}

/// Represents the errors that can occur when loading `MeshData` onto the GPU.
#[derive(Debug)]
pub enum MeshBufferError {
    OutOfMemory,
    MeshAlreadyPresent,
    VertexBufferCreationError(glium::vertex::BufferCreationError),
    IndexBufferCreationError(glium::index::BufferCreationError),
}

/// Conversion traits from `BufferCreationError` types to `MeshLoadError`
impl From<glium::vertex::BufferCreationError> for MeshBufferError {
    fn from(err: glium::vertex::BufferCreationError) -> Self {
        MeshBufferError::VertexBufferCreationError(err)
    }
}

impl From<glium::index::BufferCreationError> for MeshBufferError {
    fn from(err: glium::index::BufferCreationError) -> Self {
        MeshBufferError::IndexBufferCreationError(err)
    }
}

const MESH_BUFFER_SIZE: usize = 65535;
pub struct MeshBuffer {
    vbo: VertexBuffer<Vertex>,
    ibo: IndexBuffer<u32>,
    vbo_start: usize,
    ibo_start: usize,
    meshes: HashMap<Uuid, (Arc<Mesh>, usize, usize)>,
}

impl MeshBuffer {
    pub fn new(display: Display) -> Result<MeshBuffer, MeshBufferError> {
        let (vbo, ibo) = MeshBuffer::create_buffers(&display)?;
        Ok(MeshBuffer {
            vbo,
            ibo,
            vbo_start: 0,
            ibo_start: 0,
            meshes: HashMap::new(),
        })
    }

    /// Adds a mesh to this buffer
    pub fn add_mesh(&mut self, mesh: Arc<Mesh>) -> Result<(), MeshBufferError> {
        // Don't add meshes we already have
        if self.meshes.contains_key(&mesh.mesh_id) {
            return Err(MeshBufferError::MeshAlreadyPresent);
        }

        // Not enough room for this mesh. TODO: resize or error.
        if self.vbo.len() - self.vbo_start < mesh.vertices.len() {
            return Err(MeshBufferError::OutOfMemory);
        }

        let (vbo_start, ibo_start) = (self.vbo_start, self.ibo_start);

        // Write vertices to vbo
        if let Some(slice) = self
            .vbo
            .slice_mut(vbo_start..vbo_start + mesh.vertices.len())
        {
            slice.write(&mesh.vertices);
            self.vbo_start += mesh.vertices.len();
        }

        // Write indices to ibo
        if let Some(slice) = self
            .ibo
            .slice_mut(ibo_start..ibo_start + mesh.indices.len())
        {
            slice.write(
                &mesh
                    .indices
                    .iter()
                    .map(|i| *i + vbo_start as u32)
                    .collect::<Vec<u32>>(),
            );
            self.ibo_start += mesh.indices.len();
        }

        self.meshes
            .insert(mesh.mesh_id, (mesh, vbo_start, ibo_start));

        Ok(())
    }

    /// Removes a mesh from this buffer
    pub fn remove_mesh(&mut self) {}

    /// Creates a new pair of buffers, inserting them to the `buffers` vec and returning a reference.
    fn create_buffers(
        display: &Display,
    ) -> Result<(VertexBuffer<Vertex>, IndexBuffer<u32>), MeshBufferError> {
        let new_buffers = (
            VertexBuffer::empty_dynamic(display, MESH_BUFFER_SIZE)?,
            IndexBuffer::empty_dynamic(
                display,
                PrimitiveType::TrianglesList,
                MESH_BUFFER_SIZE * 3,
            )?,
        );

        Ok(new_buffers)
    }

    pub fn vertex_buffer(&self) -> &VertexBuffer<Vertex> {
        &self.vbo
    }

    pub fn index_buffer(&self) -> &IndexBuffer<u32> {
        &self.ibo
    }
}
