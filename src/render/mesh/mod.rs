use glium::vertex::{Attribute, AttributeType};
use uuid::Uuid;

use crate::math::{Vector2, Vector3};

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
