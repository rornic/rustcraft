use std::f64::consts::TAU;

use bevy::{
    asset::{Asset, RenderAssetUsages},
    core_pipeline::{schedule::Core3d, Core3dSystems, FullscreenShader},
    diagnostic::FrameCount,
    image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor},
    pbr::{MaterialPipeline, MaterialPipelineKey},
    prelude::*,
    reflect::TypePath,
    render::{
        diagnostic::RecordDiagnostics,
        extract_resource::{ExtractResource, ExtractResourcePlugin},
        mesh::MeshVertexBufferLayoutRef,
        render_asset::RenderAssets,
        render_resource::{
            binding_types::{sampler, storage_buffer_read_only_sized, texture_2d, uniform_buffer},
            encase::UniformBuffer,
            AsBindGroup, BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries,
            BufferInitDescriptor, BufferUsages, CachedRenderPipelineId, ColorTargetState,
            ColorWrites, CompareFunction, Extent3d, FragmentState, Operations, PipelineCache,
            RenderPassColorAttachment, RenderPassDescriptor, RenderPipelineDescriptor,
            SamplerBindingType, ShaderStages, ShaderType, SpecializedMeshPipelineError,
            TextureDimension, TextureFormat, TextureSampleType, TextureUsages,
        },
        renderer::{RenderContext, RenderDevice},
        storage::{GpuShaderBuffer, ShaderBuffer},
        texture::GpuImage,
        RenderApp, RenderStartup,
    },
    shader::{ShaderDefVal, ShaderRef},
    transform::TransformSystems,
    window::PrimaryWindow,
};
use noise::{Fbm, MultiFractal, NoiseFn, Perlin};

use crate::sky::{Moon, Sun};
use crate::sky_dome::dome;
use ochre_terrain::curvature::Curvature;
pub use ochre_terrain::material::CloudLayer;
use ochre_terrain::material::{push_shader_defs, water_level};
use ochre_terrain::settings::CloudResolution;

/// Height of the clouds' undersides, far above the highest land; matches `BASE` in `cloud_march.wesl`.
pub const CLOUD_BASE: f32 = 2400.0;
// Height the tallest clouds' tops reach; matches `TOP` in `cloud_march.wesl`.
const CLOUD_TOP: f32 = 3000.0;
// The pattern's cells, in blocks, and how many it has each way: it repeats seamlessly.
const CELL: f32 = 48.0;
const PATTERN: usize = 256;
// The pattern's width in blocks.
const SPAN: f32 = PATTERN as f32 * CELL;
// Share of the sky with any cloud over it, from wisps at the edges to full cover in the middle.
const CLOUDY: f64 = 0.45;
// Cells across a cloud and a cluster of them, and how much clustering counts against single clouds.
const CLOUD_CELLS: f64 = 24.0;
const CLUSTER_CELLS: f64 = 80.0;
const CLUSTERING: f64 = 0.8;
// Which way and how fast clouds drift, in blocks a second: a whole number of patterns an
// hour each way, so they carry on seamlessly when the shaders' clock, Bevy's `globals.time`,
// wraps back to zero on the hour.
const WIND: Vec2 = Vec2::new(SPAN * 2.0 / 3600.0, SPAN / 3600.0);
// The clouds are marched at this fraction of the screen's width and height, and blended up to it.
const TARGET_SCALE: u32 = 2;
// Texels across each half of the reflections' map, matching `REFLECTION_TEXELS` in `cloud_march.wesl`.
const REFLECTION_SIZE: u32 = 512;

#[derive(Default)]
pub struct CloudPlugin {
    pub resolution: CloudResolution,
}

#[derive(Resource)]
struct CloudTargetResolution(CloudResolution);

impl Plugin for CloudPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(CloudTargetResolution(self.resolution))
            .add_plugins((
                MaterialPlugin::<CloudMaterial>::default(),
                ExtractResourcePlugin::<CloudMarch>::default(),
            ))
            // before the Startup systems that build materials from the cloud layer
            .add_systems(PreStartup, spawn_clouds)
            // once the camera's spawned
            .add_systems(PostStartup, spawn_cloud_dome)
            .add_systems(Update, fit_cloud_target)
            .add_systems(
                PostUpdate,
                aim_cloud_march.after(TransformSystems::Propagate),
            );
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app
                .add_systems(RenderStartup, init_cloud_pipeline)
                // nothing it draws depends on the scene, so before any of it
                .add_systems(
                    Core3d,
                    march_clouds
                        .after(Core3dSystems::Prepass)
                        .before(Core3dSystems::MainPass),
                );
        }
    }
}

/// The clouds drawn into the target, lit and blended up to the screen, see `cloud_dome.wesl`.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct CloudMaterial {
    #[texture(4)]
    #[sampler(5)]
    pattern: Handle<Image>,
    #[uniform(6)]
    layer: Vec4,
    // the share of light they scatter, in x, less in a storm
    #[uniform(7)]
    shade: Vec4,
    // what the land bends around, which the clouds bend with, see `curvature::Curvature`
    #[storage(8, read_only)]
    curvature: Handle<ShaderBuffer>,
    #[texture(9)]
    #[sampler(10)]
    target: Handle<Image>,
}

impl CloudMaterial {
    /// Darkens the clouds to `shade` of their colour, as storm clouds, see `weather.rs`.
    pub fn set_shade(&mut self, shade: f32) {
        self.shade.x = shade;
    }
}

impl Material for CloudMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/cloud_dome.wesl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "shaders/cloud_dome.wesl".into()
    }

    // blended over the scene drawn before it
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Premultiplied
    }

    // over the stars and sun glare, under rain and every other blended thing, see `sky_dome::SKY_DEPTH_BIAS`
    fn depth_bias(&self) -> f32 {
        -1.0e9
    }

    fn enable_prepass() -> bool {
        false
    }

    fn enable_shadows() -> bool {
        false
    }

    // seen from inside, over every pixel: the shader stops each ray at the scene itself
    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = None;
        if let Some(depth) = descriptor.depth_stencil.as_mut() {
            depth.depth_compare = Some(CompareFunction::Always);
        }
        push_shader_defs(descriptor, &[water_level()]);
        Ok(())
    }
}

/// Draws the unlit clouds into the low-res target before the main passes, see `cloud_target.wesl`.
#[derive(Resource, Clone, ExtractResource)]
#[extract_app(RenderApp)]
struct CloudMarch {
    target: Handle<Image>,
    reflection: Handle<Image>,
    // last frame's target, which the march blends into, so few steps a frame add up to many
    history: Handle<Image>,
    pattern: Handle<Image>,
    layer: Vec4,
    curvature: Handle<ShaderBuffer>,
    view: CloudView,
}

/// How the main camera looks and where the sun is, for the march, which has no view of its own.
#[derive(ShaderType, Clone, Copy, Debug, Default)]
struct CloudView {
    // the view's right and up, as far across as the screen's edges are a block ahead
    right: Vec4,
    up: Vec4,
    forward: Vec4,
    // the camera's position in xyz, and the shaders' clock, Bevy's `globals.time`, in w
    eye: Vec4,
    // toward the sun in xyz, and the frame count in w
    sun: Vec4,
    // last frame's axes and position, for reprojecting the history
    last_right: Vec4,
    last_up: Vec4,
    last_forward: Vec4,
    last_eye: Vec4,
    // toward the moon, whose disc the clouds hide as they do the sun's
    moon: Vec4,
}

// Where cell (`x`, `z`) lies round a torus in 4D, for noise with features about `cells` across
// that wraps at the pattern's edges without a seam.
fn torus(x: usize, z: usize, cells: f64) -> [f64; 4] {
    let radius = PATTERN as f64 / (TAU * cells);
    let angle = |c: usize| TAU * c as f64 / PATTERN as f64;
    let (x, z) = (angle(x), angle(z));
    [x.cos(), x.sin(), z.cos(), z.sin()].map(|v| v * radius)
}

/// Cloud cover per cell, row-major [z][x]: clear over `1 - CLOUDY` of the sky, rising evenly to 1 over the rest.
fn cloud_pattern(seed: u32) -> Vec<f32> {
    let noise = Fbm::<Perlin>::new(seed)
        .set_octaves(3)
        .set_persistence(0.45);
    let clusters = Perlin::new(seed.wrapping_add(1));
    let values: Vec<f64> = (0..PATTERN * PATTERN)
        .map(|i| {
            let (x, z) = (i % PATTERN, i / PATTERN);
            noise.get(torus(x, z, CLOUD_CELLS))
                + CLUSTERING * clusters.get(torus(x, z, CLUSTER_CELLS))
        })
        .collect();
    let mut order: Vec<usize> = (0..values.len()).collect();
    order.sort_by(|&a, &b| values[a].total_cmp(&values[b]));
    let mut cover = vec![0.0; values.len()];
    for (rank, i) in order.into_iter().enumerate() {
        let share = rank as f64 / values.len() as f64;
        cover[i] = ((share - (1.0 - CLOUDY)) / CLOUDY).max(0.0) as f32;
    }
    cover
}

// The pattern as a texture that repeats, u along x and v along z.
fn pattern_image(cover: &[f32]) -> Image {
    let mut image = Image::new(
        Extent3d {
            width: PATTERN as u32,
            height: PATTERN as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        cover.iter().map(|c| (c * 255.0).round() as u8).collect(),
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

fn spawn_clouds(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    curvature: Res<Curvature>,
    window: Query<&Window, With<PrimaryWindow>>,
    resolution: Res<CloudTargetResolution>,
) {
    let size = target_size(
        window.single().map_or(UVec2::ONE, Window::physical_size),
        resolution.0,
    );
    let mut target = Image::new_target_texture(size.x, size.y, TextureFormat::Rgba16Float, None);
    target.sampler = ImageSampler::linear();
    let history = images.add(target.clone());
    // copied on to the history each frame
    target.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    let mut reflection = Image::new_target_texture(
        2 * REFLECTION_SIZE,
        REFLECTION_SIZE,
        TextureFormat::Rgba16Float,
        None,
    );
    reflection.sampler = ImageSampler::linear();
    let clouds = CloudLayer {
        // OCHRE_SEED repeats the clouds too, for comparing frames across runs
        pattern: images.add(pattern_image(&cloud_pattern(
            std::env::var("OCHRE_SEED")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or_else(rand::random),
        ))),
        layer: Vec4::new((CLOUD_BASE + CLOUD_TOP) / 2.0, SPAN, WIND.x, WIND.y),
        target: images.add(target),
        reflection: images.add(reflection),
    };
    commands.insert_resource(CloudMarch {
        target: clouds.target.clone(),
        reflection: clouds.reflection.clone(),
        history,
        pattern: clouds.pattern.clone(),
        layer: clouds.layer,
        curvature: curvature.buffer.clone(),
        view: CloudView::default(),
    });
    commands.insert_resource(clouds);
}

// The size of the clouds' target for a window `size` pixels across.
fn target_size(size: UVec2, resolution: CloudResolution) -> UVec2 {
    let target = (size / TARGET_SCALE).max(UVec2::ONE);
    let width = target.x.min(resolution.width());
    let height = (u64::from(target.y) * u64::from(width) / u64::from(target.x)).max(1) as u32;
    UVec2::new(width, height)
}

/// Spawns the dome over the main camera blending the clouds' target up over the scene.
fn spawn_cloud_dome(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<CloudMaterial>>,
    clouds: Res<CloudLayer>,
    curvature: Res<Curvature>,
    camera: Query<Entity, With<Camera3d>>,
) {
    let Ok(camera) = camera.single() else {
        return;
    };
    let material = materials.add(CloudMaterial {
        pattern: clouds.pattern.clone(),
        layer: clouds.layer,
        shade: Vec4::ONE,
        curvature: curvature.buffer.clone(),
        target: clouds.target.clone(),
    });
    commands.spawn(dome(&mut meshes, material, camera));
}

// Refits the cloud target on resize and rebinds its readers, whose bind groups would otherwise freeze the old texture on screen.
fn fit_cloud_target(
    window: Query<&Window, With<PrimaryWindow>>,
    march: Option<Res<CloudMarch>>,
    mut images: ResMut<Assets<Image>>,
    mut clouds: ResMut<Assets<CloudMaterial>>,
    resolution: Res<CloudTargetResolution>,
) {
    let (Ok(window), Some(march)) = (window.single(), march) else {
        return;
    };
    let size = target_size(window.physical_size(), resolution.0);
    if images
        .get(&march.target)
        .is_none_or(|image| image.size() == size)
    {
        return;
    }
    for handle in [&march.target, &march.history] {
        if let Some(mut image) = images.get_mut(handle) {
            image.resize(Extent3d {
                width: size.x,
                height: size.y,
                depth_or_array_layers: 1,
            });
        }
    }
    let ids: Vec<_> = clouds.ids().collect();
    for id in ids {
        if let Some(material) = clouds.get_mut(id) {
            material.into_inner();
        }
    }
}

// Tells the march how the main camera looks and where the sun is, once both have moved.
fn aim_cloud_march(
    time: Res<Time>,
    frames: Res<FrameCount>,
    camera: Query<(&GlobalTransform, &Projection), With<Camera3d>>,
    sun: Query<&Transform, With<Sun>>,
    moon: Query<&Transform, With<Moon>>,
    march: Option<ResMut<CloudMarch>>,
) {
    let (Ok((camera, Projection::Perspective(projection))), Ok(sun), Some(mut march)) =
        (camera.single(), sun.single(), march)
    else {
        return;
    };
    let up = (projection.fov / 2.0).tan();
    let last = march.view;
    let eye = camera.translation();
    march.view = CloudView {
        right: (camera.right() * up * projection.aspect_ratio).extend(0.0),
        up: (camera.up() * up).extend(0.0),
        forward: camera.forward().extend(0.0),
        eye: eye.extend(time.elapsed_secs_wrapped()),
        sun: sun.back().extend(frames.0 as f32),
        last_right: last.right,
        last_up: last.up,
        last_forward: last.forward,
        last_eye: last.eye,
        moon: moon
            .single()
            .map_or(Vec4::ZERO, |moon| moon.back().extend(0.0)),
    };
}

#[derive(Resource)]
struct CloudPipeline {
    layout: BindGroupLayoutDescriptor,
    pipeline: CachedRenderPipelineId,
    reflection: CachedRenderPipelineId,
}

fn init_cloud_pipeline(
    mut commands: Commands,
    fullscreen_shader: Res<FullscreenShader>,
    asset_server: Res<AssetServer>,
    pipeline_cache: Res<PipelineCache>,
) {
    // as `cloud_bindings.wesl` and `curvature.wesl` lay them out for materials
    let layout = BindGroupLayoutDescriptor::new(
        "cloud_march_layout",
        &BindGroupLayoutEntries::with_indices(
            ShaderStages::FRAGMENT,
            (
                (4, texture_2d(TextureSampleType::Float { filterable: true })),
                (5, sampler(SamplerBindingType::Filtering)),
                (6, uniform_buffer::<Vec4>(false)),
                (7, uniform_buffer::<CloudView>(false)),
                (8, storage_buffer_read_only_sized(false, None)),
                (9, texture_2d(TextureSampleType::Float { filterable: true })),
                (10, sampler(SamplerBindingType::Filtering)),
            ),
        ),
    );
    let shader = asset_server.load("shaders/cloud_target.wesl");
    let queue = |label: &'static str, entry_point: &'static str| {
        pipeline_cache.queue_render_pipeline(RenderPipelineDescriptor {
            label: Some(label.into()),
            layout: vec![layout.clone()],
            vertex: fullscreen_shader.to_vertex_state(),
            fragment: Some(FragmentState {
                shader: shader.clone(),
                shader_defs: vec![ShaderDefVal::UInt("MATERIAL_BIND_GROUP".into(), 0)],
                entry_point: Some(entry_point.into()),
                targets: vec![Some(ColorTargetState {
                    format: TextureFormat::Rgba16Float,
                    blend: None,
                    write_mask: ColorWrites::ALL,
                })],
                ..default()
            }),
            ..default()
        })
    };
    let pipeline = queue("cloud_march_pipeline", "fragment");
    let reflection = queue("cloud_reflection_pipeline", "reflection");
    commands.insert_resource(CloudPipeline {
        layout,
        pipeline,
        reflection,
    });
}

// A uniform buffer holding `value`.
fn uniform<T: ShaderType + bevy::render::render_resource::encase::internal::WriteInto>(
    device: &RenderDevice,
    value: &T,
) -> Option<bevy::render::render_resource::Buffer> {
    let mut bytes = UniformBuffer::new(Vec::new());
    bytes.write(value).ok()?;
    Some(device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("cloud_march_uniform"),
        contents: &bytes.into_inner(),
        usage: BufferUsages::UNIFORM,
    }))
}

/// Draws the clouds, unlit, into their target and the reflections' sky map, see `CloudMarch`.
fn march_clouds(
    march: Option<Res<CloudMarch>>,
    pipeline: Option<Res<CloudPipeline>>,
    pipeline_cache: Res<PipelineCache>,
    images: Res<RenderAssets<GpuImage>>,
    buffers: Res<RenderAssets<GpuShaderBuffer>>,
    mut ctx: RenderContext,
) {
    let (Some(march), Some(pipeline)) = (march, pipeline) else {
        return;
    };
    let (
        Some(render_pipeline),
        Some(reflection_pipeline),
        Some(target),
        Some(reflection),
        Some(history),
        Some(pattern),
        Some(curvature),
    ) = (
        pipeline_cache.get_render_pipeline(pipeline.pipeline),
        pipeline_cache.get_render_pipeline(pipeline.reflection),
        images.get(&march.target),
        images.get(&march.reflection),
        images.get(&march.history),
        images.get(&march.pattern),
        buffers.get(&march.curvature),
    )
    else {
        return;
    };
    let device = ctx.render_device().clone();
    let (Some(layer), Some(view)) = (
        uniform(&device, &march.layer),
        uniform(&device, &march.view),
    ) else {
        return;
    };
    let bind_group = device.create_bind_group(
        "cloud_march_bind_group",
        &pipeline_cache.get_bind_group_layout(&pipeline.layout),
        &BindGroupEntries::with_indices((
            (4, &pattern.texture_view),
            (5, &pattern.sampler),
            (6, layer.as_entire_binding()),
            (7, view.as_entire_binding()),
            (8, curvature.buffer.as_entire_binding()),
            (9, &history.texture_view),
            (10, &history.sampler),
        )),
    );
    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    for (label, render_pipeline, image) in [
        ("cloud_march", render_pipeline, target),
        ("cloud_reflection", reflection_pipeline, reflection),
    ] {
        let mut pass = ctx
            .command_encoder()
            .begin_render_pass(&RenderPassDescriptor {
                label: Some(label),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &image.texture_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations::default(),
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        let span = diagnostics.pass_span(&mut pass, label);
        pass.set_pipeline(render_pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..3, 0..1);
        span.end(&mut pass);
    }
    if target.texture.size() == history.texture.size() {
        ctx.command_encoder().copy_texture_to_texture(
            target.texture.as_image_copy(),
            history.texture.as_image_copy(),
            target.texture.size(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloud_target_caps_high_resolutions_and_keeps_smaller_views_half_size() {
        for (window, expected) in [
            (UVec2::new(1280, 720), UVec2::new(640, 360)),
            (UVec2::new(1920, 1080), UVec2::new(640, 360)),
            (UVec2::new(2560, 1440), UVec2::new(640, 360)),
            (UVec2::new(3840, 2160), UVec2::new(640, 360)),
            (UVec2::ZERO, UVec2::ONE),
        ] {
            assert_eq!(target_size(window, CloudResolution::Medium), expected);
        }
    }

    #[test]
    fn cloud_target_preserves_aspect_ratio_within_one_pixel_and_caps_width() {
        for window in [
            UVec2::new(3440, 1440),
            UVec2::new(2160, 3840),
            UVec2::new(5120, 1440),
        ] {
            let target = target_size(window, CloudResolution::Medium);
            assert!(target.x <= CloudResolution::Medium.width());
            let aspect = window.x as f64 / window.y as f64;
            assert!((f64::from(target.x) - f64::from(target.y) * aspect).abs() <= 1.0 + aspect);
        }
        for window in [UVec2::new(u32::MAX, 1), UVec2::new(1, u32::MAX)] {
            let target = target_size(window, CloudResolution::Medium);
            assert!(target.min_element() >= 1);
            assert!(target.x <= CloudResolution::Medium.width());
        }
    }

    #[test]
    fn cloud_resolution_presets_set_width_and_derive_height() {
        for (resolution, width) in [
            (CloudResolution::Low, 320),
            (CloudResolution::Medium, 640),
            (CloudResolution::High, 960),
            (CloudResolution::Ultra, 1280),
        ] {
            assert_eq!(
                target_size(UVec2::new(3840, 2160), resolution),
                UVec2::new(width, width * 9 / 16)
            );
        }
        assert_eq!(
            target_size(UVec2::new(800, 600), CloudResolution::Ultra),
            UVec2::new(400, 300)
        );
    }

    #[test]
    fn test_pattern_covers_the_share_of_sky_asked() {
        let cover = cloud_pattern(3);
        let cloudy = cover.iter().filter(|&&c| c > 0.0).count() as f64 / cover.len() as f64;
        assert!((cloudy - CLOUDY).abs() < 0.01, "{cloudy}");
        assert!(cover.iter().all(|c| (0.0..=1.0).contains(c)));
    }

    #[test]
    fn test_pattern_wraps_without_a_seam() {
        // across the wrap, neighbouring cells differ about as much as anywhere else
        let cover = cloud_pattern(3);
        let n = PATTERN;
        let step = |a: usize, b: usize| -> f32 {
            (0..n)
                .map(|z| (cover[z * n + a] - cover[z * n + b]).abs())
                .sum()
        };
        let inside = step(n / 2, n / 2 + 1);
        let across = step(n - 1, 0);
        assert!(
            across < inside * 1.5 + 1.0,
            "inside {inside}, across the wrap {across}"
        );
    }

    #[test]
    fn test_wind_carries_whole_patterns_an_hour() {
        let patterns = WIND * 3600.0 / SPAN;
        assert!(patterns.abs_diff_eq(patterns.round(), 1e-4), "{patterns}");
    }
}
