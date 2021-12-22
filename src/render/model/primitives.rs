use super::{ModelData, Normal, Vertex};
use crate::{normal, vertex};

pub fn square() -> ModelData {
    ModelData::new(
        vec![
            vertex!(-0.5, 0.5, 0.0),
            vertex!(0.5, 0.5, 0.0),
            vertex!(-0.5, -0.5, 0.0),
            vertex!(0.5, -0.5, 0.0),
        ],
        vec![
            normal!(0.0, 0.0, -1.0),
            normal!(0.0, 0.0, -1.0),
            normal!(0.0, 0.0, -1.0),
            normal!(0.0, 0.0, -1.0),
        ],
        vec![0, 1, 2, 2, 1, 3],
    )
}

pub fn cube() -> ModelData {
    ModelData::new(
        vec![
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
            // Top face
            vertex!(-0.5, 0.5, 0.5),
            vertex!(0.5, 0.5, 0.5),
            vertex!(-0.5, 0.5, -0.5),
            vertex!(0.5, 0.5, -0.5),
            // Bottom face
            vertex!(-0.5, -0.5, 0.5),
            vertex!(0.5, -0.5, 0.5),
            vertex!(-0.5, -0.5, -0.5),
            vertex!(0.5, -0.5, -0.5),
        ],
        vec![
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
            // Top face
            normal!(0.0, 1.0, 0.0),
            normal!(0.0, 1.0, 0.0),
            normal!(0.0, 1.0, 0.0),
            normal!(0.0, 1.0, 0.0),
            // Bottom face
            normal!(0.0, -1.0, 0.0),
            normal!(0.0, -1.0, 0.0),
            normal!(0.0, -1.0, 0.0),
            normal!(0.0, -1.0, 0.0),
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