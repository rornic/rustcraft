use super::MeshData;
use crate::vector3;

pub fn square() -> MeshData {
    MeshData::new(
        vec![
            vector3!(-0.5, 0.5, 0.0),
            vector3!(0.5, 0.5, 0.0),
            vector3!(-0.5, -0.5, 0.0),
            vector3!(0.5, -0.5, 0.0),
        ],
        vec![
            vector3!(0.0, 0.0, -1.0),
            vector3!(0.0, 0.0, -1.0),
            vector3!(0.0, 0.0, -1.0),
            vector3!(0.0, 0.0, -1.0),
        ],
        vec![0, 1, 2, 2, 1, 3],
    )
}

pub fn cube() -> MeshData {
    MeshData::new(
        vec![
            // Front face
            vector3!(-0.5, 0.5, -0.5),
            vector3!(0.5, 0.5, -0.5),
            vector3!(-0.5, -0.5, -0.5),
            vector3!(0.5, -0.5, -0.5),
            // Right face
            vector3!(0.5, 0.5, -0.5),
            vector3!(0.5, -0.5, -0.5),
            vector3!(0.5, 0.5, 0.5),
            vector3!(0.5, -0.5, 0.5),
            // Left face
            vector3!(-0.5, 0.5, -0.5),
            vector3!(-0.5, -0.5, -0.5),
            vector3!(-0.5, 0.5, 0.5),
            vector3!(-0.5, -0.5, 0.5),
            // Back face
            vector3!(-0.5, 0.5, 0.5),
            vector3!(0.5, 0.5, 0.5),
            vector3!(-0.5, -0.5, 0.5),
            vector3!(0.5, -0.5, 0.5),
            // Top face
            vector3!(-0.5, 0.5, 0.5),
            vector3!(0.5, 0.5, 0.5),
            vector3!(-0.5, 0.5, -0.5),
            vector3!(0.5, 0.5, -0.5),
            // Bottom face
            vector3!(-0.5, -0.5, 0.5),
            vector3!(0.5, -0.5, 0.5),
            vector3!(-0.5, -0.5, -0.5),
            vector3!(0.5, -0.5, -0.5),
        ],
        vec![
            // Front face
            vector3!(0.0, 0.0, 1.0),
            vector3!(0.0, 0.0, 1.0),
            vector3!(0.0, 0.0, 1.0),
            vector3!(0.0, 0.0, 1.0),
            // Right face
            vector3!(1.0, 0.0, 0.0),
            vector3!(1.0, 0.0, 0.0),
            vector3!(1.0, 0.0, 0.0),
            vector3!(1.0, 0.0, 0.0),
            // Left face
            vector3!(-1.0, 0.0, 0.0),
            vector3!(-1.0, 0.0, 0.0),
            vector3!(-1.0, 0.0, 0.0),
            vector3!(-1.0, 0.0, 0.0),
            // Back face
            vector3!(0.0, 0.0, -1.0),
            vector3!(0.0, 0.0, -1.0),
            vector3!(0.0, 0.0, -1.0),
            vector3!(0.0, 0.0, -1.0),
            // Top face
            vector3!(0.0, 1.0, 0.0),
            vector3!(0.0, 1.0, 0.0),
            vector3!(0.0, 1.0, 0.0),
            vector3!(0.0, 1.0, 0.0),
            // Bottom face
            vector3!(0.0, -1.0, 0.0),
            vector3!(0.0, -1.0, 0.0),
            vector3!(0.0, -1.0, 0.0),
            vector3!(0.0, -1.0, 0.0),
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
