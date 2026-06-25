#[derive(Default, Copy, Clone)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
}

// Same geometry as `cube()`, but every vertex at the top of the cube (y == 0.5) is
// pulled down to `top_y` instead - used to give water a surface that sits below a
// full block's height while keeping its bottom/footprint identical to other blocks.
pub fn cube_with_top(top_y: f32) -> Vec<Vertex> {
    cube()
        .into_iter()
        .map(|mut v| {
            if v.position[1] == 0.5 {
                v.position[1] = top_y;
            }
            v
        })
        .collect()
}

pub fn cube() -> Vec<Vertex> {
    vec![
        // Front face
        Vertex {
            position: [-0.5, 0.5, -0.5],
            normal: [0.0, 0.0, 1.0],
            uv: [0.0, 1.0],
        },
        Vertex {
            position: [-0.5, -0.5, -0.5],
            normal: [0.0, 0.0, 1.0],
            uv: [0.0, 0.0],
        },
        Vertex {
            position: [0.5, 0.5, -0.5],
            normal: [0.0, 0.0, 1.0],
            uv: [1.0, 1.0],
        },
        Vertex {
            position: [0.5, -0.5, -0.5],
            normal: [0.0, 0.0, 1.0],
            uv: [1.0, 0.0],
        },
        // Right face
        Vertex {
            position: [0.5, 0.5, -0.5],
            normal: [1.0, 0.0, 0.0],
            uv: [0.0, 1.0],
        },
        Vertex {
            position: [0.5, -0.5, -0.5],
            normal: [1.0, 0.0, 0.0],
            uv: [0.0, 0.0],
        },
        Vertex {
            position: [0.5, 0.5, 0.5],
            normal: [1.0, 0.0, 0.0],
            uv: [1.0, 1.0],
        },
        Vertex {
            position: [0.5, -0.5, 0.5],
            normal: [1.0, 0.0, 0.0],
            uv: [1.0, 0.0],
        },
        // Left face
        Vertex {
            position: [-0.5, 0.5, 0.5],
            normal: [-1.0, 0.0, 0.0],
            uv: [1.0, 1.0],
        },
        Vertex {
            position: [-0.5, -0.5, 0.5],
            normal: [-1.0, 0.0, 0.0],
            uv: [1.0, 0.0],
        },
        Vertex {
            position: [-0.5, 0.5, -0.5],
            normal: [-1.0, 0.0, 0.0],
            uv: [0.0, 1.0],
        },
        Vertex {
            position: [-0.5, -0.5, -0.5],
            normal: [-1.0, 0.0, 0.0],
            uv: [0.0, 0.0],
        },
        // Back face
        Vertex {
            position: [0.5, 0.5, 0.5],
            normal: [0.0, 0.0, -1.0],
            uv: [1.0, 0.0],
        },
        Vertex {
            position: [0.5, -0.5, 0.5],
            normal: [0.0, 0.0, -1.0],
            uv: [1.0, 1.0],
        },
        Vertex {
            position: [-0.5, 0.5, 0.5],
            normal: [0.0, 0.0, -1.0],
            uv: [0.0, 0.0],
        },
        Vertex {
            position: [-0.5, -0.5, 0.5],
            normal: [0.0, 0.0, -1.0],
            uv: [0.0, 1.0],
        },
        // Top face
        Vertex {
            position: [-0.5, 0.5, 0.5],
            normal: [0.0, 1.0, 0.0],
            uv: [0.0, 1.0],
        },
        Vertex {
            position: [-0.5, 0.5, -0.5],
            normal: [0.0, 1.0, 0.0],
            uv: [1.0, 1.0],
        },
        Vertex {
            position: [0.5, 0.5, 0.5],
            normal: [0.0, 1.0, 0.0],
            uv: [0.0, 0.0],
        },
        Vertex {
            position: [0.5, 0.5, -0.5],
            normal: [0.0, 1.0, 0.0],
            uv: [1.0, 0.0],
        },
        // Bottom face
        Vertex {
            position: [-0.5, -0.5, -0.5],
            normal: [0.0, -1.0, 0.0],
            uv: [0.0, 1.0],
        },
        Vertex {
            position: [-0.5, -0.5, 0.5],
            normal: [0.0, -1.0, 0.0],
            uv: [1.0, 1.0],
        },
        Vertex {
            position: [0.5, -0.5, -0.5],
            normal: [0.0, -1.0, 0.0],
            uv: [0.0, 0.0],
        },
        Vertex {
            position: [0.5, -0.5, 0.5],
            normal: [0.0, -1.0, 0.0],
            uv: [1.0, 0.0],
        },
    ]
}
