use glium::{framebuffer::RenderBuffer, Frame};

#[derive(Copy, Clone)]
pub struct Vertex {
    position: [f32; 3],
}
implement_vertex!(Vertex, position);

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

pub struct Shape {
    pub vertices: Vec<Vertex>,
    pub normals: Vec<Normal>,
    pub indices: Vec<u32>,
}

pub fn square() -> Shape {
    Shape {
        vertices: vec![
            vertex!(-0.5, 0.5, 0.0),
            vertex!(0.5, 0.5, 0.0),
            vertex!(-0.5, -0.5, 0.0),
            vertex!(0.5, -0.5, 0.0),
        ],
        normals: vec![
            normal!(0.0, 0.0, -1.0),
            normal!(0.0, 0.0, -1.0),
            normal!(0.0, 0.0, -1.0),
            normal!(0.0, 0.0, -1.0),
        ],
        indices: vec![0, 1, 2, 2, 1, 3],
    }
}

pub fn cube() -> Shape {
    Shape {
        vertices: vec![
            // Front face
            vertex!(-0.5, 0.5, -0.5),
            vertex!(0.5, 0.5, -0.5),
            vertex!(-0.5, -0.5, -0.5),
            vertex!(0.5, -0.5, -0.5),
            // Right face
            vertex!(0.5, 0.5, -0.5),
            vertex!(0.5, -0.5, -0.5),
            vertex!(0.5, 0.5, 0.5),
            vertex!(0.5, -0.5, 0.5),
            // Left face
            vertex!(-0.5, 0.5, -0.5),
            vertex!(-0.5, -0.5, -0.5),
            vertex!(-0.5, 0.5, 0.5),
            vertex!(-0.5, -0.5, 0.5),
            // Back face
            vertex!(-0.5, 0.5, 0.5),
            vertex!(0.5, 0.5, 0.5),
            vertex!(-0.5, -0.5, 0.5),
            vertex!(0.5, -0.5, 0.5),
        ],
        normals: vec![
            // Front face
            normal!(0.0, 0.0, 1.0),
            normal!(0.0, 0.0, 1.0),
            normal!(0.0, 0.0, 1.0),
            normal!(0.0, 0.0, 1.0),
            // Right face
            normal!(1.0, 0.0, 0.0),
            normal!(1.0, 0.0, 0.0),
            normal!(1.0, 0.0, 0.0),
            normal!(1.0, 0.0, 0.0),
            // Left face
            normal!(-1.0, 0.0, 0.0),
            normal!(-1.0, 0.0, 0.0),
            normal!(-1.0, 0.0, 0.0),
            normal!(-1.0, 0.0, 0.0),
            // Back face
            normal!(0.0, 0.0, -1.0),
            normal!(0.0, 0.0, -1.0),
            normal!(0.0, 0.0, -1.0),
            normal!(0.0, 0.0, -1.0),
        ],
        indices: vec![
            // Front
            0, 1, 2, 2, 1, 3, // Right
            4, 5, 6, 6, 5, 7, // Left
            8, 9, 10, 10, 9, 11, // Back
            12, 13, 14, 14, 13, 15,
        ],
    }
}
