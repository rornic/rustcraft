use std::f64::consts::TAU;

use bevy::{
    asset::{Asset, RenderAssetUsages},
    image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor},
    pbr::{MaterialPipeline, MaterialPipelineKey, NotShadowCaster, NotShadowReceiver},
    prelude::*,
    reflect::TypePath,
    render::{
        mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology},
        render_resource::{
            AsBindGroup, Extent3d, RenderPipelineDescriptor, ShaderRef, SpecializedMeshPipelineError,
            TextureDimension, TextureFormat,
        },
    },
};
use noise::{Fbm, MultiFractal, NoiseFn, Perlin};

use crate::chunks::material::{push_shader_defs, water_level};

/// Height of the clouds' undersides: above most land, so only the tallest peaks reach them.
pub const CLOUD_BASE: f32 = 720.0;
// Clouds are blocks this wide and tall.
const CELL: f32 = 20.0;
const THICKNESS: f32 = 8.0;
// Cells across the pattern, which repeats seamlessly.
const PATTERN: usize = 256;
// How much of the sky is cloud.
const COVER: f64 = 0.2;
// Roughly how many cells across a cloud is.
const CLOUD_CELLS: f64 = 10.0;
// Copies of the pattern drawn out from the one over the camera, each way: enough to reach
// the fog at the sea-level render distance.
const COPIES: i32 = 2;
// The pattern's width in blocks.
const SPAN: f32 = PATTERN as f32 * CELL;
// Which way and how fast clouds drift, in blocks a second: a whole number of patterns an
// hour each way, so they carry on seamlessly when the shaders' clock, Bevy's `globals.time`,
// wraps back to zero on the hour.
const WIND: Vec2 = Vec2::new(SPAN * 5.0 / 3600.0, SPAN * 2.0 / 3600.0);
// Slightly warm white, as sunlight leaves them.
const CLOUD_COLOR: LinearRgba = LinearRgba::rgb(0.95, 0.95, 0.92);

pub struct CloudPlugin;

impl Plugin for CloudPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<CloudMaterial> {
            // clouds are far above anything they could shade, see `CLOUD_BASE`
            prepass_enabled: false,
            shadows_enabled: false,
            ..default()
        })
        .add_systems(Startup, spawn_clouds)
        .add_systems(Update, drift_clouds);
    }
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct CloudMaterial {
    #[uniform(0)]
    color: LinearRgba,
}

impl Material for CloudMaterial {
    fn fragment_shader() -> ShaderRef {
        "shaders/clouds.wgsl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }

    fn specialize(
        _pipeline: &MaterialPipeline<Self>,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        push_shader_defs(descriptor, &[water_level()]);
        Ok(())
    }
}

/// The cloud pattern, for the land's shader to cast clouds' shadows with, see
/// `cloud_shadow.wgsl`. Spawned by `spawn_clouds`.
#[derive(Resource, Clone)]
pub struct CloudLayer {
    /// The pattern, one texel a cell, cloud where it's 1.
    pub pattern: Handle<Image>,
    /// The middle of the layer's height, the pattern's width in blocks, and the wind.
    pub layer: Vec4,
}

// One copy of the pattern, `offset` patterns from the one over the camera.
#[derive(Component)]
struct CloudTile {
    offset: IVec2,
}

/// Which cells of the pattern are cloud, `PATTERN` x `PATTERN`, row-major [x][z]. Sampled
/// round a torus in 4D noise, so it wraps at its edges without a seam.
fn cloud_pattern(seed: u32) -> Vec<bool> {
    let noise = Fbm::<Perlin>::new(seed).set_octaves(3);
    let radius = PATTERN as f64 / (TAU * CLOUD_CELLS);
    let around = |cell: usize| {
        let angle = TAU * cell as f64 / PATTERN as f64;
        (angle.cos() * radius, angle.sin() * radius)
    };
    let mut values = Vec::with_capacity(PATTERN * PATTERN);
    for x in 0..PATTERN {
        for z in 0..PATTERN {
            let ((a, b), (c, d)) = (around(x), around(z));
            values.push(noise.get([a, b, c, d]));
        }
    }
    let mut sorted = values.clone();
    sorted.sort_by(f64::total_cmp);
    let threshold = sorted[((1.0 - COVER) * sorted.len() as f64) as usize];
    values.into_iter().map(|v| v > threshold).collect()
}

#[derive(Default)]
struct CloudMesh {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    indices: Vec<u32>,
}

impl CloudMesh {
    // A quad facing `normal`, wound so it's seen from that side.
    fn quad(&mut self, mut corners: [Vec3; 4], normal: Vec3) {
        if (corners[1] - corners[0]).cross(corners[2] - corners[0]).dot(normal) < 0.0 {
            corners.reverse();
        }
        let start = self.positions.len() as u32;
        self.positions.extend(corners.map(|c| c.to_array()));
        self.normals.extend([normal.to_array(); 4]);
        self.indices.extend([0, 1, 2, 0, 2, 3].map(|i| start + i));
    }
}

/// The pattern as blocks: tops and bottoms merged into rectangles, and sides into runs.
fn cloud_mesh(pattern: &[bool]) -> Mesh {
    let n = PATTERN;
    let cloud = |x: usize, z: usize| pattern[(x % n) * n + z % n];
    let mut mesh = CloudMesh::default();
    let corner = |x: usize, y: f32, z: usize| Vec3::new(x as f32 * CELL, y, z as f32 * CELL);

    // tops and bottoms, as the fewest rectangles the cells fill
    let mut taken = vec![false; n * n];
    for x0 in 0..n {
        for z0 in 0..n {
            if !cloud(x0, z0) || taken[x0 * n + z0] {
                continue;
            }
            let mut x1 = x0 + 1;
            while x1 < n && cloud(x1, z0) && !taken[x1 * n + z0] {
                x1 += 1;
            }
            let mut z1 = z0 + 1;
            while z1 < n && (x0..x1).all(|x| cloud(x, z1) && !taken[x * n + z1]) {
                z1 += 1;
            }
            for x in x0..x1 {
                for z in z0..z1 {
                    taken[x * n + z] = true;
                }
            }
            for (y, normal) in [(THICKNESS, Vec3::Y), (0.0, Vec3::NEG_Y)] {
                mesh.quad([corner(x0, y, z0), corner(x1, y, z0), corner(x1, y, z1), corner(x0, y, z1)], normal);
            }
        }
    }

    // sides, where a cloud cell meets clear sky, merged along each edge
    for x in 0..n {
        for (normal, step) in [(Vec3::X, 1), (Vec3::NEG_X, n - 1)] {
            let edge = if normal.x > 0.0 { x + 1 } else { x };
            let mut z = 0;
            while z < n {
                let open = |z: usize| cloud(x, z) && !cloud(x + step, z);
                if !open(z) {
                    z += 1;
                    continue;
                }
                let start = z;
                while z < n && open(z) {
                    z += 1;
                }
                mesh.quad(
                    [corner(edge, 0.0, start), corner(edge, THICKNESS, start), corner(edge, THICKNESS, z), corner(edge, 0.0, z)],
                    normal,
                );
            }
        }
    }
    for z in 0..n {
        for (normal, step) in [(Vec3::Z, 1), (Vec3::NEG_Z, n - 1)] {
            let edge = if normal.z > 0.0 { z + 1 } else { z };
            let mut x = 0;
            while x < n {
                let open = |x: usize| cloud(x, z) && !cloud(x, z + step);
                if !open(x) {
                    x += 1;
                    continue;
                }
                let start = x;
                while x < n && open(x) {
                    x += 1;
                }
                mesh.quad(
                    [corner(start, 0.0, edge), corner(start, THICKNESS, edge), corner(x, THICKNESS, edge), corner(x, 0.0, edge)],
                    normal,
                );
            }
        }
    }

    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, mesh.positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, mesh.normals)
        .with_inserted_indices(Indices::U32(mesh.indices))
}

// The pattern as a texture that repeats, blended between cells so shadows' edges are soft.
fn pattern_image(pattern: &[bool]) -> Image {
    let n = PATTERN;
    // u along x, v along z
    let data = (0..n * n).map(|i| if pattern[(i % n) * n + i / n] { 255 } else { 0 }).collect();
    let mut image = Image::new(
        Extent3d {
            width: n as u32,
            height: n as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::R8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        ..default()
    });
    image
}

pub fn spawn_clouds(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<CloudMaterial>>,
) {
    let pattern = cloud_pattern(rand::random());
    commands.insert_resource(CloudLayer {
        pattern: images.add(pattern_image(&pattern)),
        layer: Vec4::new(CLOUD_BASE + THICKNESS / 2.0, SPAN, WIND.x, WIND.y),
    });
    let mesh = meshes.add(cloud_mesh(&pattern));
    let material = materials.add(CloudMaterial { color: CLOUD_COLOR });
    for x in -COPIES..=COPIES {
        for z in -COPIES..=COPIES {
            commands.spawn((
                CloudTile { offset: IVec2::new(x, z) },
                Mesh3d(mesh.clone()),
                MeshMaterial3d(material.clone()),
                Transform::default(),
                NotShadowCaster,
                NotShadowReceiver,
            ));
        }
    }
}

/// Where the pattern copy `offset` copies from the one over `camera` starts, `elapsed`
/// seconds into the wind.
fn tile_origin(camera: Vec2, elapsed: f32, offset: IVec2) -> Vec2 {
    let drift = (WIND * elapsed).rem_euclid(Vec2::splat(SPAN));
    ((camera - drift) / SPAN).floor() * SPAN + drift + offset.as_vec2() * SPAN
}

fn drift_clouds(
    time: Res<Time>,
    camera: Query<&GlobalTransform, With<Camera3d>>,
    mut tiles: Query<(&CloudTile, &mut Transform)>,
) {
    let Ok(camera) = camera.get_single() else {
        return;
    };
    let camera = camera.translation().xz();
    for (tile, mut transform) in &mut tiles {
        // the shaders' clock, so clouds and their shadows keep together
        let origin = tile_origin(camera, time.elapsed_secs_wrapped(), tile.offset);
        transform.translation = Vec3::new(origin.x, CLOUD_BASE, origin.y);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pattern_covers_about_the_share_of_sky_asked() {
        let pattern = cloud_pattern(3);
        let cover = pattern.iter().filter(|&&c| c).count() as f64 / pattern.len() as f64;
        assert!((cover - COVER).abs() < 0.01, "{cover}");
    }

    #[test]
    fn test_pattern_wraps_without_a_seam() {
        // across the wrap, neighbouring cells agree about as often as anywhere else
        let pattern = cloud_pattern(3);
        let n = PATTERN;
        let agree = |a: usize, b: usize| (0..n).filter(|&z| pattern[a * n + z] == pattern[b * n + z]).count();
        let inside = agree(n / 2, n / 2 + 1);
        let across = agree(n - 1, 0);
        assert!(across as f64 > inside as f64 * 0.8, "inside {inside}, across the wrap {across}");
    }

    #[test]
    fn test_faces_point_outward() {
        let mut pattern = vec![false; PATTERN * PATTERN];
        pattern[5 * PATTERN + 7] = true;
        let mesh = cloud_mesh(&pattern);
        let Some(Indices::U32(indices)) = mesh.indices() else { panic!() };
        let positions = mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap().as_float3().unwrap();
        let normals = mesh.attribute(Mesh::ATTRIBUTE_NORMAL).unwrap().as_float3().unwrap();
        // a single cell is a box: six faces
        assert_eq!(36, indices.len());
        for tri in indices.chunks(3) {
            let p = |i: u32| Vec3::from(positions[i as usize]);
            let winding = (p(tri[1]) - p(tri[0])).cross(p(tri[2]) - p(tri[0]));
            assert!(winding.dot(Vec3::from(normals[tri[0] as usize])) > 0.0);
        }
    }

    #[test]
    fn test_wind_carries_on_seamlessly_past_the_hour() {
        let camera = Vec2::new(500.0, 900.0);
        assert!(tile_origin(camera, 3600.0, IVec2::ZERO).abs_diff_eq(tile_origin(camera, 0.0, IVec2::ZERO), 0.05));
    }

    #[test]
    fn test_image_puts_x_along_u() {
        let mut pattern = vec![false; PATTERN * PATTERN];
        pattern[3 * PATTERN + 200] = true;
        let image = pattern_image(&pattern);
        let data = image.data.as_slice();
        assert_eq!(255, data[200 * PATTERN + 3]);
        assert_eq!(1, data.iter().filter(|&&v| v > 0).count());
    }

    #[test]
    fn test_tiles_cover_the_camera_and_drift_with_the_wind() {
        let span = SPAN;
        let camera = Vec2::new(10_000.0, -3_000.0);
        for elapsed in [0.0, 100.0, 5_000.0] {
            let origin = tile_origin(camera, elapsed, IVec2::ZERO);
            assert!((camera - origin).cmpge(Vec2::ZERO).all() && (camera - origin).cmplt(Vec2::splat(span)).all());
        }
        let moved = tile_origin(camera, 1.0, IVec2::ZERO) - tile_origin(camera, 0.0, IVec2::ZERO);
        assert!((moved - WIND).length() < 1e-3 || (moved - WIND).abs().max_element() > span * 0.5);
    }
}

