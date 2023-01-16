use uuid::Uuid;

use crate::{vector3, world::ecs::bounds::Bounds};

pub struct Mesh {
    pub id: Uuid,
    pub vertices: Vec<Vertex>,
    pub triangles: Vec<u32>,
    mesh_bounds: Bounds,
}

impl Mesh {
    pub fn new(vertices: Vec<Vertex>, triangles: Vec<u32>) -> Self {
        Self::new_with_id(Uuid::new_v4(), vertices, triangles)
    }

    pub fn new_with_id(id: Uuid, vertices: Vec<Vertex>, triangles: Vec<u32>) -> Self {
        let mut mesh = Self {
            id,
            vertices,
            triangles,
            mesh_bounds: Bounds::new(vector3!(0.0, 0.0, 0.0), vector3!(0.0, 0.0, 0.0)),
        };
        mesh.recalculate_bounds();
        mesh
    }

    pub fn recalculate_bounds(&mut self) {
        let x_min = self
            .vertices
            .iter()
            .min_by(|a, b| a.position[0].total_cmp(&b.position[0]))
            .unwrap()
            .position[0];
        let x_max = self
            .vertices
            .iter()
            .max_by(|a, b| a.position[0].total_cmp(&b.position[0]))
            .unwrap()
            .position[0];
        let y_min = self
            .vertices
            .iter()
            .min_by(|a, b| a.position[1].total_cmp(&b.position[1]))
            .unwrap()
            .position[1];
        let y_max = self
            .vertices
            .iter()
            .max_by(|a, b| a.position[1].total_cmp(&b.position[1]))
            .unwrap()
            .position[1];
        let z_min = self
            .vertices
            .iter()
            .min_by(|a, b| a.position[2].total_cmp(&b.position[2]))
            .unwrap()
            .position[2];
        let z_max = self
            .vertices
            .iter()
            .max_by(|a, b| a.position[2].total_cmp(&b.position[2]))
            .unwrap()
            .position[2];

        let (x_diff, y_diff, z_diff) = (x_max - x_min, y_max - y_min, z_max - z_min);
        self.mesh_bounds = Bounds::new(
            vector3!(
                x_min + x_diff / 2.0,
                y_min + y_diff / 2.0,
                z_min + z_diff / 2.0
            ),
            vector3!(x_diff, y_diff, z_diff),
        );
    }

    pub fn bounds(&self) -> Bounds {
        self.mesh_bounds
    }
}

#[derive(Default, Copy, Clone)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
}
implement_vertex!(Vertex, position, normal, uv);
