use uuid::Uuid;

pub struct Mesh {
    pub id: Uuid,
    pub vertices: Vec<Vertex>,
    pub triangles: Vec<u32>,
}

impl Mesh {
    pub fn new(vertices: Vec<Vertex>, triangles: Vec<u32>) -> Self {
        Self::new_with_id(Uuid::new_v4(), vertices, triangles)
    }

    pub fn new_with_id(id: Uuid, vertices: Vec<Vertex>, triangles: Vec<u32>) -> Self {
        Self {
            id,
            vertices,
            triangles,
        }
    }
}

#[derive(Copy, Clone)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
}
implement_vertex!(Vertex, position, normal, uv);
