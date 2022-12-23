use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    sync::Weak,
    time::Instant,
};

use glium::{
    index::{DrawCommandsIndicesBuffer, IndexBufferSlice, PrimitiveType},
    texture::SrgbTexture2d,
    uniforms::{
        MagnifySamplerFilter, MinifySamplerFilter, Sampler, SamplerBehavior, UniformBuffer,
    },
    vertex::VertexBufferSlice,
    Display, Frame, IndexBuffer, Program, Surface, VertexBuffer,
};
use specs::{
    world::EntitiesRes, Component, Entities, Join, Read, ReadStorage, System, VecStorage, Write,
};
use uuid::Uuid;

use crate::world::ecs::{bounds::Bounds, camera::Camera, Transform};

use super::{
    material::{load_shader, load_texture, Material},
    mesh::{Mesh, Vertex},
};

#[derive(Clone)]
pub struct DrawCall {
    material: Material,
    mesh: Arc<Mesh>,
    transform: Transform,
}

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

#[derive(Default)]
pub struct RenderJob {
    draw_calls: Vec<DrawCall>,
}

pub struct RenderingSystem;

impl<'a> System<'a> for RenderingSystem {
    type SystemData = (
        ReadStorage<'a, Camera>,
        ReadStorage<'a, Transform>,
        ReadStorage<'a, RenderMesh>,
        ReadStorage<'a, Bounds>,
        Write<'a, RenderJob>,
    );

    fn run(
        &mut self,
        (cameras, transforms, render_meshes, bounds, mut render_job): Self::SystemData,
    ) {
        let (camera, camera_transform) = (&cameras, &transforms).join().next().unwrap();

        render_job.draw_calls.clear();

        for (transform, mesh_data, bounds) in (&transforms, &render_meshes, &bounds).join() {
            if !camera.are_bounds_visible(camera_transform, transform.position, bounds) {
                continue;
            }

            render_job.draw_calls.push(DrawCall {
                material: Material {
                    name: "default".to_string(),
                },
                mesh: mesh_data.mesh.clone(),
                transform: transform.clone(),
            });
        }
    }
}

pub struct Renderer {
    display: Display,
    global_uniform_buffer: UniformBuffer<GlobalUniforms>,
    shader: Program,
    texture: SrgbTexture2d,
    mesh_heap: MeshHeap,
}

impl Renderer {
    pub fn new(display: Display) -> Self {
        let global_uniform_buffer: UniformBuffer<GlobalUniforms> =
            UniformBuffer::empty(&display).unwrap();

        // TODO: keep shaders and textures in a HashMap
        let shader = load_shader(&display, "default").unwrap();
        let texture = load_texture(&display, "textures/stone.png").unwrap();

        let mesh_heap = MeshHeap::new();

        Self {
            display,
            global_uniform_buffer,
            shader,
            texture,
            mesh_heap,
        }
    }

    pub fn render(
        &mut self,
        camera: &mut Camera,
        render_job: &RenderJob,
        view_matrix: [[f32; 4]; 4],
    ) {
        let draw_params = glium::DrawParameters {
            depth: glium::Depth {
                test: glium::draw_parameters::DepthTest::IfLess,
                write: true,
                ..Default::default()
            },
            backface_culling: glium::BackfaceCullingMode::CullClockwise,
            ..Default::default()
        };

        for draw_call in render_job.draw_calls.iter() {
            if !self
                .mesh_heap
                .loaded_meshes
                .contains_key(&draw_call.mesh.id)
            {
                self.mesh_heap
                    .load_mesh(&self.display, draw_call.mesh.clone());
            }
        }

        let mut target: Frame = self.display.draw();
        target.clear_color_and_depth((0.549, 0.745, 0.839, 1.0), 1.0);

        let (width, height) = target.get_dimensions();
        camera.aspect_ratio = width as f32 / height as f32;

        let global_uniforms = GlobalUniforms {
            projection_matrix: camera.projection_matrix(),
            view_matrix: view_matrix,
            light: [-1.0, 0.4, 0.9f32],
            model_matrix: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0_f32],
            ],
        };
        self.global_uniform_buffer.write(&global_uniforms);

        for mesh_buffer in &self.mesh_heap.mesh_buffers {
            target
                .draw(
                    mesh_buffer.vbo.slice(0..mesh_buffer.vbo_pos).unwrap(),
                    mesh_buffer.ibo.slice(0..mesh_buffer.ibo_pos).unwrap(),
                    &self.shader,
                    &uniform! {
                                GlobalUniforms: &self.global_uniform_buffer,
                                tex: Sampler(
                        &self.texture,
                        SamplerBehavior {
                            minify_filter: MinifySamplerFilter::NearestMipmapLinear,
                            magnify_filter: MagnifySamplerFilter::Nearest,
                            ..Default::default()
                        },
                    )
                            },
                    &draw_params,
                )
                .unwrap();
        }
        target.finish().unwrap();

        self.mesh_heap.unload_dropped_meshes();

        for buf in self.mesh_heap.mesh_buffers.iter_mut() {
            let unusable_space = buf.vbo_free_space.iter().map(|m| m.size).sum::<usize>();
            if unusable_space >= MAX_VBO_SIZE / 8 {
                buf.compact(&self.display);
            }
        }
    }
}

#[derive(Clone, Copy)]
struct GlobalUniforms {
    model_matrix: [[f32; 4]; 4],
    projection_matrix: [[f32; 4]; 4],
    view_matrix: [[f32; 4]; 4],
    light: [f32; 3],
}
implement_uniform_block!(
    GlobalUniforms,
    model_matrix,
    projection_matrix,
    view_matrix,
    light
);

const MAX_VBO_SIZE: usize = 65536 * 8;
struct MeshBuffer {
    vbo: VertexBuffer<Vertex>,
    vbo_pos: usize,
    vbo_free_space: Vec<MemoryBlock>,
    vbo_allocated: Vec<MemoryBlock>,
    ibo: IndexBuffer<u32>,
    ibo_pos: usize,
    ibo_free_space: Vec<MemoryBlock>,
    ibo_allocated: Vec<MemoryBlock>,
}

impl MeshBuffer {
    fn new(display: &Display, size: usize) -> Self {
        let vbo = VertexBuffer::empty_dynamic(display, size).unwrap();
        let ibo =
            IndexBuffer::empty_dynamic(display, PrimitiveType::TrianglesList, size * 3)
                .unwrap();

        Self {
            vbo,
            vbo_pos: 0,
            vbo_free_space: Vec::new(),
            vbo_allocated: Vec::new(),
            ibo,
            ibo_pos: 0,
            ibo_free_space: Vec::new(),
            ibo_allocated: Vec::new(),
        }
    }

    fn allocate(&mut self, size: usize) -> Option<MeshLocator> {
        if let Some(mesh_locator) = self.allocate_from_free_space(size) {
            return Some(mesh_locator);
        }

        if self.vbo_pos + size >= MAX_VBO_SIZE
            || self.ibo_pos + 3*size >= MAX_VBO_SIZE * 3
        {
            return None;
        }

        let mesh_locator = MeshLocator {
            vertices: MemoryBlock {
                start: self.vbo_pos,
                size: size,
            },
            triangles: MemoryBlock {
                start: self.ibo_pos,
                size: 3*size,
            },
        };
        self.vbo_allocated.push(mesh_locator.vertices);
        self.ibo_allocated.push(mesh_locator.triangles);

        self.vbo_pos += size;
        self.ibo_pos += 3*size;

        Some(mesh_locator)
    }

    fn allocate_from_free_space(&mut self, size: usize) -> Option<MeshLocator> {
        let vbo_block = self.closest_block_fit(size, &self.vbo_free_space);
        let ibo_block = self.closest_block_fit(3*size, &self.ibo_free_space);
        if vbo_block.is_some() && ibo_block.is_some() {
            let old_block = self.vbo_free_space.remove(vbo_block.unwrap());
            let vbo = MemoryBlock {
                start: old_block.start,
                size: size,
            };
            self.vbo_free_space.push(MemoryBlock {
                start: vbo.start + vbo.size,
                size: old_block.size - vbo.size,
            });

            let old_block = self.ibo_free_space.remove(ibo_block.unwrap());
            let ibo = MemoryBlock {
                start: old_block.start,
                size: 3*size,
            };

            self.ibo_free_space.push(MemoryBlock {
                start: ibo.start + ibo.size,
                size: old_block.size - ibo.size,
            });

            self.vbo_allocated.push(vbo);
            self.ibo_allocated.push(ibo);
            return Some(MeshLocator {
                vertices: vbo,
                triangles: ibo,
            });
        }

        None
    }

    fn free(&mut self, locator: &MeshLocator) {
        self.free_vbo(locator.vertices);
        self.free_ibo(locator.triangles);
    }

    fn free_vbo(&mut self, mem: MemoryBlock) {
        self.slice_vbo(mem).invalidate();
        self.slice_vbo(mem).write(
            &(0..mem.size as u32)
                .map(|_| Vertex {
                    position: [0.0, 0.0, 0.0],
                    normal: [0.0, 0.0, 0.0],
                    uv: [0.0, 0.0],
                })
                .collect::<Vec<Vertex>>(),
        );
        self.vbo_free_space.push(mem);
    }

    fn free_ibo(&mut self, mem: MemoryBlock) {
        self.slice_ibo(mem).invalidate();
        self.slice_ibo(mem)
            .write(&(0..mem.size as u32).collect::<Vec<u32>>());
        self.ibo_free_space.push(mem);
    }

    fn slice_vbo<'a>(&'a self, mem: MemoryBlock) -> VertexBufferSlice<'a, Vertex> {
        self.vbo.slice(mem.start..mem.start + mem.size).unwrap()
    }

    fn slice_ibo<'a>(&'a self, mem: MemoryBlock) -> IndexBufferSlice<'a, u32> {
        self.ibo.slice(mem.start..mem.start + mem.size).unwrap()
    }

    fn closest_block_fit(&self, size: usize, blocks: &[MemoryBlock]) -> Option<usize> {
        let mut valid_blocks: Vec<(usize, usize)> = blocks
            .iter()
            .enumerate()
            .filter(|(_, b)| b.size >= size)
            .map(|(i, b)| (b.size - size, i))
            .collect();
        valid_blocks.sort();
        valid_blocks.get(0).map(|(_, i)| *i)
    }

    fn compact(&mut self, display: &Display) {
        let vbo: VertexBuffer<Vertex> = VertexBuffer::empty_dynamic(display, MAX_VBO_SIZE).unwrap();
        let mut new_blocks = vec![];
        let mut pos = 0;
        for block in self.vbo_allocated.iter() {
            self.slice_vbo(*block)
                .copy_to(vbo.slice(pos..pos + block.size).unwrap())
                .unwrap();
            new_blocks.push(MemoryBlock {
                start: pos,
                size: block.size,
            });
            pos += block.size;
        }
        println!("compacted vbo: old {}, new {}", self.vbo_pos, pos);
        vbo.copy_to(&self.vbo).unwrap();
        self.vbo_allocated = new_blocks;
        self.vbo_pos = pos;
        self.vbo_free_space.clear();

        let ibo: IndexBuffer<u32> =
            IndexBuffer::empty_dynamic(display, PrimitiveType::TrianglesList, MAX_VBO_SIZE * 3)
                .unwrap();
        let mut new_blocks = vec![];
        let mut pos = 0;
        for block in self.ibo_allocated.iter() {
            let triangles = self.slice_ibo(*block).read().unwrap().iter().map(f)

                ibo.slice(pos..pos + block.size).unwrap();
            new_blocks.push(MemoryBlock {
                start: pos,
                size: block.size,
            });
            pos += block.size;
        }
        ibo.copy_to(&self.ibo).unwrap();
        self.ibo_allocated = new_blocks;
        self.ibo_pos = pos;
        self.ibo_free_space.clear();
    }

    fn allocated(&self) -> usize {
        self.vbo_allocated.iter().map(|b|b.size).sum()
    }
}

#[derive(Clone, Copy)]
struct MeshLocator {
    vertices: MemoryBlock,
    triangles: MemoryBlock,
}

#[derive(Clone, Copy)]
struct MemoryBlock {
    start: usize,
    size: usize,
}

struct MeshHeap {
    mesh_buffers: Vec<MeshBuffer>,
    loaded_meshes: HashMap<Uuid, (usize, MeshLocator)>,
    mesh_refs: HashMap<Uuid, Weak<Mesh>>,
}

impl MeshHeap {
    fn new() -> MeshHeap {
        MeshHeap {
            mesh_buffers: vec![],
            loaded_meshes: HashMap::new(),
            mesh_refs: HashMap::new(),
        }
    }

    fn load_mesh(&mut self, display: &Display, mesh: Arc<Mesh>) {
        let (buf, locator) = self.allocate(display, &mesh);
        self.write_mesh(&mesh, buf, locator);

        self.loaded_meshes.insert(mesh.id, (buf, locator));
        self.mesh_refs.insert(mesh.id, Arc::downgrade(&mesh));
    }

    fn write_mesh(&self, mesh: &Mesh, buf: usize, locator: MeshLocator) {
        let (vbo, ibo) = self.slice_mesh(buf, locator);

        vbo.write(&mesh.vertices);

        let shifted_tris: Vec<u32> = mesh
            .triangles
            .iter()
            .map(|i| *i + locator.vertices.start as u32)
            .collect();
        ibo.write(&shifted_tris);
    }

    fn unload_mesh(&mut self, id: Uuid) {
        if !self.loaded_meshes.contains_key(&id) {
            return;
        }
        let (buf, locator) = self.loaded_meshes.get(&id).unwrap();
        self.mesh_buffers[*buf].free(locator);
        self.loaded_meshes.remove(&id);
        self.mesh_refs.remove(&id);
    }

    fn unload_dropped_meshes(&mut self) {
        let to_unload: Vec<Uuid> = self
            .mesh_refs
            .iter()
            .filter(|(_, mesh_ref)| mesh_ref.upgrade().is_none())
            .map(|(id, _)| *id)
            .collect();

        for id in to_unload {
            self.unload_mesh(id);
        }
    }

    fn compact(&mut self, display: &Display) {
        let total: usize = self.mesh_buffers.iter().map(|buf| buf.allocated()).sum();

        let new_buf = MeshBuffer::new(display, total);
        for (id, (buf, loc)) in self.loaded_meshes.iter_mut() {
            let new_locator = new_buf.allocate(loc.vertices.size).unwrap();
            let (old_vbo, old_ibo) = self.slice_mesh(*buf, *loc);
            old_vbo.copy_to(new_buf.slice_vbo(new_locator.vertices));
            old_ibo.copy_to(new_buf.slice_ibo(new_locator.triangles));
            *buf = 0;
            *loc = new_locator;
        }

        for buf in self.mesh_buffers {
            buf.vbo.invalidate();
            buf.ibo.invalidate();
        }

        self.mesh_buffers = vec![new_buf];

    }

    fn allocate(&mut self, display: &Display, mesh: &Mesh) -> (usize, MeshLocator) {
        for (i, buffer) in self.mesh_buffers.iter_mut().enumerate().rev() {
            if let Some(locator) = buffer.allocate(mesh) {
                return (i, locator);
            }
        }

        let locator = self.new_buffer(display).allocate(mesh).unwrap();
        (self.mesh_buffers.len() - 1, locator)
    }

    fn slice_mesh<'a>(
        &'a self,
        buf: usize,
        locator: MeshLocator,
    ) -> (VertexBufferSlice<'a, Vertex>, IndexBufferSlice<'a, u32>) {
        (
            self.mesh_buffers[buf].slice_vbo(locator.vertices),
            self.mesh_buffers[buf].slice_ibo(locator.triangles),
        )
    }

    fn new_buffer(&mut self, display: &Display) -> &mut MeshBuffer {
        let buffer = MeshBuffer::new(display, MAX_VBO_SIZE);
        self.mesh_buffers.push(buffer);
        self.mesh_buffers.last_mut().unwrap()
    }
}
