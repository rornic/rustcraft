pub struct Mesh {
    pub vertices: Vec<Vertex>,
    pub triangles: Vec<u32>,
}

impl Mesh {
    pub fn new(vertices: Vec<Vertex>, triangles: Vec<u32>) -> Self {
        Self {
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
