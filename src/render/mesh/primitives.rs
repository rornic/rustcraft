use super::{MeshData, Vertex};
use crate::{vector2, vector3, vertex};

pub fn square() -> MeshData {
    MeshData::new(
        vec![
            vertex!(
                position: vector3!(-0.5, 0.5, 0.0),
                normal: vector3!(0.0, 0.0, -1.0),
                uv: vector2!(-1.0, 1.0)
            ),
            vertex!(
                position: vector3!(0.5, 0.5, 0.0),
                normal: vector3!(0.0, 0.0, -1.0),
                uv: vector2!(1.0, 1.0)
            ),
            vertex!(
                position: vector3!(-0.5, -0.5, 0.0),
                normal: vector3!(0.0, 0.0, -1.0),
                uv: vector2!(-1.0, -1.0)
            ),
            vertex!(
                position: vector3!(0.5, -0.5, 0.0),
                normal: vector3!(0.0, 0.0, -1.0),
                uv: vector2!(1.0, -1.0)
            ),
        ],
        vec![0, 1, 2, 2, 1, 3],
    )
}

pub fn cube() -> MeshData {
    MeshData::new(
        vec![
            // Front face
            vertex!(
                position: vector3!(-0.5, 0.5, -0.5),
                normal: vector3!(0.0, 0.0, 1.0),
                uv: vector2!(0.0, 1.0)
            ),
            vertex!(
                position: vector3!(0.5, 0.5, -0.5),
                normal: vector3!(0.0, 0.0, 1.0),
                uv: vector2!(1.0, 1.0)
            ),
            vertex!(
                position: vector3!(-0.5, -0.5, -0.5),
                normal: vector3!(0.0, 0.0, 1.0),
                uv: vector2!(0.0, 0.0)
            ),
            vertex!(
                position: vector3!(0.5, -0.5, -0.5),
                normal: vector3!(0.0, 0.0, 1.0),
                uv: vector2!(1.0, 0.0)
            ),
            // Right face
            vertex!(
                position: vector3!(0.5, 0.5, -0.5),
                normal: vector3!(1.0, 0.0, 0.0),
                uv: vector2!(0.0, 1.0)
            ),
            vertex!(
                position: vector3!(0.5, -0.5, -0.5),
                normal: vector3!(1.0, 0.0, 0.0),
                uv: vector2!(0.0, 0.0)
            ),
            vertex!(
                position: vector3!(0.5, 0.5, 0.5),
                normal: vector3!(1.0, 0.0, 0.0),
                uv: vector2!(1.0, 1.0)
            ),
            vertex!(
                position: vector3!(0.5, -0.5, 0.5),
                normal: vector3!(1.0, 0.0, 0.0),
                uv: vector2!(1.0, 0.0)
            ),
            // Left face
            vertex!(
                position: vector3!(-0.5, 0.5, -0.5),
                normal: vector3!(-1.0, 0.0, 0.0),
                uv: vector2!(1.0, 0.0)
            ),
            vertex!(
                position: vector3!(-0.5, -0.5, -0.5),
                normal: vector3!(-1.0, 0.0, 0.0),
                uv: vector2!(1.0, 1.0)
            ),
            vertex!(
                position: vector3!(-0.5, 0.5, 0.5),
                normal: vector3!(-1.0, 0.0, 0.0),
                uv: vector2!(0.0, 0.0)
            ),
            vertex!(
                position: vector3!(-0.5, -0.5, 0.5),
                normal: vector3!(-1.0, 0.0, 0.0),
                uv: vector2!(0.0, 1.0)
            ),
            // Back face
            vertex!(
                position: vector3!(-0.5, 0.5, 0.5),
                normal: vector3!(0.0, 0.0, -1.0),
                uv: vector2!(0.0, 1.0)
            ),
            vertex!(
                position: vector3!(0.5, 0.5, 0.5),
                normal: vector3!(0.0, 0.0, -1.0),
                uv: vector2!(1.0, 1.0)
            ),
            vertex!(
                position: vector3!(-0.5, -0.5, 0.5),
                normal: vector3!(0.0, 0.0, -1.0),
                uv: vector2!(0.0, 0.0)
            ),
            vertex!(
                position: vector3!(0.5, -0.5, 0.5),
                normal: vector3!(0.0, 0.0, -1.0),
                uv: vector2!(1.0, 0.0)
            ),
            // Top face
            vertex!(
                position: vector3!(-0.5, 0.5, 0.5),
                normal: vector3!(0.0, 1.0, 0.0),
                uv: vector2!(0.0, 1.0)
            ),
            vertex!(
                position: vector3!(0.5, 0.5, 0.5),
                normal: vector3!(0.0, 1.0, 0.0),
                uv: vector2!(1.0, 1.0)
            ),
            vertex!(
                position: vector3!(-0.5, 0.5, -0.5),
                normal: vector3!(0.0, 1.0, 0.0),
                uv: vector2!(0.0, 0.0)
            ),
            vertex!(
                position: vector3!(0.5, 0.5, -0.5),
                normal: vector3!(0.0, 1.0, 0.0),
                uv: vector2!(1.0, 0.0)
            ),
            // Bottom face
            vertex!(
                position: vector3!(-0.5, -0.5, 0.5),
                normal: vector3!(0.0, -1.0, 0.0),
                uv: vector2!(0.0, 1.0)
            ),
            vertex!(
                position: vector3!(0.5, -0.5, 0.5),
                normal: vector3!(0.0, -1.0, 0.0),
                uv: vector2!(1.0, 1.0)
            ),
            vertex!(
                position: vector3!(-0.5, -0.5, -0.5),
                normal: vector3!(0.0, -1.0, 0.0),
                uv: vector2!(0.0, 0.0)
            ),
            vertex!(
                position: vector3!(0.5, -0.5, -0.5),
                normal: vector3!(0.0, -1.0, 0.0),
                uv: vector2!(1.0, 0.0)
            ),
        ],
        vec![
            0, 1, 2, 2, 1, 3, // Front
            4, 5, 6, 6, 5, 7, // Left
            8, 9, 10, 10, 9, 11, // Left
            12, 13, 14, 14, 13, 15, // Back
            16, 17, 18, 18, 17, 19, // Top
            20, 21, 22, 22, 21, 23, // Bottom
        ],
    )
}
