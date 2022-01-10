use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use cgmath::{Vector2, Vector3};
use glium::{index::PrimitiveType, Display, IndexBuffer, VertexBuffer};
use uuid::Uuid;

use crate::{vector2, vector3};

pub mod primitives;

/// A `Vertex` is represented by a 3D position, normal and 2D UV position.
#[derive(Copy, Clone)]
pub struct Vertex {
    pub position: Vector3<f32>,
    pub normal: Vector3<f32>,
    pub uv: Vector2<f32>,
}

impl Default for Vertex {
    fn default() -> Self {
        Self {
            position: vector3!(0.0, 0.0, 0.0),
            normal: vector3!(0.0, 0.0, 0.0),
            uv: vector2!(0.0, 0.0),
        }
    }
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
    MeshNotPresent,
    WriteError,
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

const MESH_BUFFER_SIZE: usize = 4000000 / 32;
pub struct MeshBuffer {
    vbo: VertexBuffer<Vertex>,
    ibo: IndexBuffer<u32>,
    meshes: Vec<Arc<Mesh>>,
    mesh_positions: HashMap<Uuid, (usize, usize)>,
}

impl MeshBuffer {
    pub fn new(display: Display) -> Result<MeshBuffer, MeshBufferError> {
        let (vbo, ibo) = MeshBuffer::create_buffers(&display)?;
        Ok(MeshBuffer {
            vbo,
            ibo,
            meshes: Vec::new(),
            mesh_positions: HashMap::new(),
        })
    }

    /// Adds a mesh to this buffer
    pub fn add_mesh(&mut self, mesh: Arc<Mesh>) -> Result<(), MeshBufferError> {
        // Don't add meshes we already have
        if self.mesh_positions.contains_key(&mesh.mesh_id) {
            return Err(MeshBufferError::MeshAlreadyPresent);
        }

        // Either start at the end of the last mesh, or the start of the buffer
        let last_mesh = self.meshes.last();
        let (vbo_start, ibo_start) = if let Some(mesh) = last_mesh {
            let pos = self.mesh_positions.get(&mesh.mesh_id).unwrap();
            (pos.0 + mesh.vertices.len(), pos.1 + mesh.indices.len())
        } else {
            (0, 0)
        };

        // Not enough room for this mesh.
        if self.vbo.len() - vbo_start < mesh.vertices.len() {
            return Err(MeshBufferError::OutOfMemory);
        }

        self.write_data(&mesh.vertices, &mesh.indices, vbo_start, ibo_start)?;

        self.mesh_positions
            .insert(mesh.mesh_id, (vbo_start, ibo_start));
        self.meshes.push(mesh);

        Ok(())
    }

    /// Removes a mesh from this buffer
    /// This could do with some optimisations to avoid amount of data we have to upload to the GPU.
    pub fn remove_mesh(&mut self, mesh_id: &Uuid) -> Result<(), MeshBufferError> {
        // Find index of mesh we're going to remove, so we know which meshes come after it
        let remove_index = self
            .meshes
            .iter()
            .enumerate()
            .find(|(_, m)| m.mesh_id == *mesh_id)
            .ok_or(MeshBufferError::MeshNotPresent)?
            .0;

        // Find the vbo_start and ibo_start positions of the mesh we're removing. We will overwrite data from here with any subsequent meshes.
        let (mut vbo_start, mut ibo_start) = *self.mesh_positions.get(mesh_id).unwrap();

        // Overwrite old data
        if let Some(slice) = self.vbo.slice_mut(vbo_start..) {
            slice.map().fill(Vertex::default());
        }

        if let Some(slice) = self.ibo.slice_mut(ibo_start..) {
            slice.map().fill(0);
        }

        // Remove the mesh, shift other meshes back in the buffers.
        let mut new_meshes: Vec<Arc<Mesh>> = vec![];
        for (i, mesh) in self.meshes.clone().into_iter().enumerate() {
            if i == remove_index {
                continue;
            }

            // Any mesh after remove_index needs to be shifted back in the buffers.
            if i > remove_index {
                self.write_data(&mesh.vertices, &mesh.indices, vbo_start, ibo_start)?;
                self.mesh_positions
                    .insert(mesh.mesh_id, (vbo_start, ibo_start));
                vbo_start += mesh.vertices.len();
                ibo_start += mesh.indices.len();
            }
            new_meshes.push(mesh);
        }

        self.meshes = new_meshes;
        self.mesh_positions.remove(mesh_id);

        Ok(())
    }

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

    fn write_data(
        &mut self,
        vs: &[Vertex],
        is: &[u32],
        vbo_start: usize,
        ibo_start: usize,
    ) -> Result<(), MeshBufferError> {
        // Write vertices to vbo
        if let Some(slice) = self.vbo.slice_mut(vbo_start..vbo_start + vs.len()) {
            slice.write(vs);
        } else {
            return Err(MeshBufferError::WriteError);
        }

        // Write indices to ibo
        if let Some(slice) = self.ibo.slice_mut(ibo_start..ibo_start + is.len()) {
            slice.write(
                &is.iter()
                    .map(|i| *i + vbo_start as u32)
                    .collect::<Vec<u32>>(),
            );
        } else {
            return Err(MeshBufferError::WriteError);
        }

        Ok(())
    }

    pub fn vertex_buffer(&self) -> &VertexBuffer<Vertex> {
        &self.vbo
    }

    pub fn index_buffer(&self) -> &IndexBuffer<u32> {
        &self.ibo
    }
}
