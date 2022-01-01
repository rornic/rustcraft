use std::{
    collections::{HashMap, VecDeque},
    rc::Rc,
    sync::Arc,
};

use glium::{
    index::PrimitiveType,
    texture::SrgbTexture2d,
    uniforms::{
        MagnifySamplerFilter, MinifySamplerFilter, Sampler, SamplerBehavior, UniformBuffer,
        Uniforms,
    },
    Display, DrawParameters, Frame, IndexBuffer, Program, Surface, VertexBuffer,
};
use specs::{Component, Join, ReadStorage, System, VecStorage, World, WorldExt, Write};
use uuid::Uuid;

use crate::{vector3, world::ecs::Transform};
use cgmath::{prelude::*, Vector3};

use self::mesh::{Mesh, MeshLoadError, Vertex};

pub mod mesh;
pub mod shader;
pub mod texture;

/// Represents uniforms that are global across all shaders and should be present for every render.
///
/// This includes information required to project from model space to screen space as well as calculating lighting.
#[derive(Copy, Clone)]
pub struct GlobalRenderUniforms {
    pub projection_matrix: [[f32; 4]; 4],
    pub view_matrix: [[f32; 4]; 4],
    pub light: [f32; 3],
}
implement_uniform_block!(GlobalRenderUniforms, projection_matrix, view_matrix, light);

/// Component for entities that a `Mesh` should be rendered for.
///
/// Does not store the actual `Mesh` data, but just a reference to a `Mesh` that has been loaded into the `RenderingSystem`.
pub struct RenderMesh {
    mesh: Arc<Mesh>,
}

impl RenderMesh {
    pub fn new(mesh: Arc<Mesh>) -> RenderMesh {
        RenderMesh { mesh: mesh }
    }
}

impl Component for RenderMesh {
    type Storage = VecStorage<Self>;
}

pub struct ViewMatrix(pub [[f32; 4]; 4]);

impl ViewMatrix {
    pub fn new(position: Vector3<f32>, direction: Vector3<f32>, up: Vector3<f32>) -> ViewMatrix {
        let direction = direction.normalize();
        let s = vector3!(
            up.y * direction.z - up.z * direction.y,
            up.z * direction.x - up.x * direction.z,
            up.x * direction.y - up.y * direction.x
        )
        .normalize();
        let u = vector3!(
            direction.y * s.z - direction.z * s.y,
            direction.z * s.x - direction.x * s.z,
            direction.x * s.y - direction.y * s.x
        );

        let p = vector3!(
            -position.x * s.x - position.y * s.y - position.z * s.z,
            -position.x * u.x - position.y * u.y - position.z * u.z,
            -position.x * direction.x - position.y * direction.y - position.z * direction.z
        );

        ViewMatrix([
            [s.x, u.x, direction.x, 0.0],
            [s.y, u.y, direction.y, 0.0],
            [s.z, u.z, direction.z, 0.0],
            [p.x, p.y, p.z, 1.0],
        ])
    }
}

impl Default for ViewMatrix {
    fn default() -> Self {
        ViewMatrix([
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0_f32],
        ])
    }
}

/// The `RenderingSystem` produces `DrawCall` structs in parallel. These are handled by the `Renderer` which runs on the main thread.
pub struct RenderingSystem;

impl<'a> System<'a> for RenderingSystem {
    type SystemData = (
        ReadStorage<'a, Transform>,
        ReadStorage<'a, RenderMesh>,
        Write<'a, VecDeque<DrawCall>>,
    );

    /// Produce a `DrawCall` for every entity with both a `Transform` and `RenderMesh` component. TODO: Batch entities using the same mesh into a single `DrawCall`.
    fn run(&mut self, (transforms, render_meshes, mut draw_calls): Self::SystemData) {
        for (transform, mesh_data) in (&transforms, &render_meshes).join() {
            let model_matrix = transform.matrix();

            draw_calls.push_back(DrawCall {
                material: Material {
                    name: "default".to_string(),
                },
                mesh: mesh_data.mesh.clone(),
            });
        }
    }
}

#[derive(Hash, PartialEq, Eq, Clone)]
pub struct Material {
    name: String,
}

pub struct DrawCall {
    material: Material,
    mesh: Arc<Mesh>,
}

/// Represents a batch of meshes that can be rendered in a single draw call.
pub struct Batch {
    display: Display,
    meshes: HashMap<Uuid, Arc<Mesh>>,
    vbo: VertexBuffer<Vertex>,
    ibo: IndexBuffer<u32>,
    vbo_index: usize,
    ibo_index: usize,
}

impl Batch {
    pub fn new(display: Display) -> Result<Batch, MeshLoadError> {
        let vbo = VertexBuffer::empty_dynamic(&display, 1000000)?;
        let ibo = IndexBuffer::empty_dynamic(&display, PrimitiveType::TrianglesList, 1000000)?;
        Ok(Batch {
            display: display,
            meshes: HashMap::new(),
            vbo,
            ibo,
            vbo_index: 0,
            ibo_index: 0,
        })
    }

    /// Adds a `Mesh` into this batch. Assumes that the mesh vertices are relative to the world origin (0,0,0)
    pub fn add_mesh(&mut self, mesh: Arc<Mesh>) {
        // Only add the mesh if we haven't already seen it
        if !self.meshes.contains_key(&mesh.mesh_id) {
            let is = mesh
                .indices
                .iter()
                .map(|i| *i + self.vbo_index as u32)
                .collect::<Vec<u32>>();
            self.ibo
                .slice_mut(self.ibo_index..self.ibo_index + mesh.indices.len())
                .unwrap()
                .write(&is);
            self.ibo_index += is.len();

            self.vbo
                .slice_mut(self.vbo_index..self.vbo_index + mesh.vertices.len())
                .unwrap()
                .write(&mesh.vertices);
            self.vbo_index += mesh.vertices.len();

            println!("{},{}", self.ibo_index, self.vbo_index);

            self.meshes.insert(mesh.mesh_id, mesh);
        }
    }
}

/// The `Renderer` receives `DrawCall` structs and processes each of them into a draw call on the GPU.
///
/// Takes a `Display` to draw to. The `Renderer` keeps track of resources loaded onto the GPU.
pub struct Renderer {
    pub display: Display,
    global_uniform_buffer: UniformBuffer<GlobalRenderUniforms>,
    shader_program: Program,
    texture: SrgbTexture2d,
    batches: HashMap<Material, Batch>,
}

impl Renderer {
    pub fn new(display: Display) -> Self {
        // Create a buffer for global uniforms
        let global_uniform_buffer: UniformBuffer<GlobalRenderUniforms> =
            UniformBuffer::empty(&display).unwrap();

        // Create the shader program
        // TODO: Store shader id in `RenderMesh` component and keep track of shaders in `RenderingSystem`.
        let shader_program = shader::load_shader(&display, "default").unwrap();

        // Create the texture
        // TODO: Store texture id in `RenderMesh` component and keep track of textures in `RenderingSystem`.
        let texture = texture::load_texture(&display, "textures/stone.png").unwrap();

        Self {
            display,
            global_uniform_buffer,
            shader_program,
            texture,
            batches: HashMap::new(),
        }
    }

    pub fn render(&mut self, world: &mut World) {
        // Start drawing on window
        let mut target: Frame = self.display.draw();
        target.clear_color_and_depth((0.5, 0.5, 0.5, 1.0), 1.0);

        // Set up draw parameters
        let params = glium::DrawParameters {
            depth: glium::Depth {
                test: glium::draw_parameters::DepthTest::IfLess,
                write: true,
                ..Default::default()
            },
            backface_culling: glium::BackfaceCullingMode::CullClockwise,
            ..Default::default()
        };

        // Set up projection matrix
        let projection_matrix = {
            let (width, height) = target.get_dimensions();
            let aspect_ratio = height as f32 / width as f32;

            let fov: f32 = 3.141592 / 3.0;
            let zfar = 1024.0;
            let znear = 0.1;

            let f = 1.0 / (fov / 2.0).tan();

            [
                [f * aspect_ratio, 0.0, 0.0, 0.0],
                [0.0, f, 0.0, 0.0],
                [0.0, 0.0, (zfar + znear) / (zfar - znear), 1.0],
                [0.0, 0.0, -(2.0 * zfar * znear) / (zfar - znear), 0.0],
            ]
        };

        let view_matrix = world.read_resource::<ViewMatrix>().0;

        // Update global uniforms
        let global_render_uniforms = GlobalRenderUniforms {
            projection_matrix: projection_matrix,
            view_matrix: view_matrix,
            light: [-1.0, 0.4, 0.9f32],
        };
        self.global_uniform_buffer.write(&global_render_uniforms);

        // 1. Group draw calls into batches based on their material.
        let mut batches: HashMap<Material, Vec<Arc<Mesh>>> = HashMap::new();
        while let Some(draw_call) = world.write_resource::<VecDeque<DrawCall>>().pop_front() {
            batches
                .entry(draw_call.material)
                .or_insert(vec![])
                .push(draw_call.mesh);
        }

        // 2. Add any unseen meshes to the batch for that material.
        for (mat, meshes) in batches {
            // Get existing batch or create a new one
            let batch = self
                .batches
                .entry(mat)
                .or_insert(Batch::new(self.display.clone()).unwrap());

            // Add meshes to the batch
            for mesh in meshes {
                batch.add_mesh(mesh);
            }
        }

        // 3. Draw batches -- one draw call per batch.
        for (material, batch) in &self.batches {
            target
                .draw(
                    &batch.vbo,
                    &batch.ibo,
                    &self.shader_program,
                    &uniform! {
                        model_matrix: [
                            [ 1.0, 0.0, 0.0, 0.0],
                            [ 0.0, 1.0, 0.0, 0.0],
                            [ 0.0, 0.0, 1.0, 0.0],
                            [ 0.0, 0.0, 0.0, 1.0_f32],
                        ],
                        tex: Sampler(&self.texture, SamplerBehavior {
                            minify_filter: MinifySamplerFilter::NearestMipmapLinear,
                            magnify_filter: MagnifySamplerFilter::Nearest,
                            ..Default::default()
                        }),
                        global_render_uniforms: &self.global_uniform_buffer
                    },
                    &params,
                )
                .unwrap();
        }

        target.finish().unwrap();
    }
}
