pub mod primitives;

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
