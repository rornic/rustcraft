use bevy::{
    asset::{Asset, RenderAssetUsages},
    image::{
        CompressedImageFormats, ImageFilterMode, ImageSampler, ImageSamplerDescriptor, ImageType,
    },
    pbr::{MaterialPipeline, MaterialPipelineKey},
    prelude::*,
    reflect::TypePath,
    render::{
        mesh::MeshVertexBufferLayoutRef,
        render_resource::{
            AsBindGroup, Face, RenderPipelineDescriptor, SpecializedMeshPipelineError,
        },
        storage::ShaderBuffer,
    },
    shader::{ShaderDefVal, ShaderRef},
};

use crate::chunk::CHUNK_SIZE;
use crate::generate::generator::{INNER_LEAVES_BAND, INNER_LEAVES_REACH};
use crate::lod::{LodLayout, SOLID_WATER_LEVEL};
use crate::settings::DebugSettings;
use ochre_base::block::Tile;
use ochre_base::WATER_LEVEL;

// Sea level, where `fog.wesl` turns from fog across the land to murk all round.
pub fn water_level() -> ShaderDefVal {
    ShaderDefVal::Int("WATER_LEVEL".into(), WATER_LEVEL as i32)
}

// Every stage needs every def its file's imports use, since each is built from the whole file.
pub fn push_shader_defs(descriptor: &mut RenderPipelineDescriptor, defs: &[ShaderDefVal]) {
    descriptor.vertex.shader_defs.extend_from_slice(defs);
    if let Some(fragment) = &mut descriptor.fragment {
        fragment.shader_defs.extend_from_slice(defs);
    }
}

/// The block texture atlas, one square tile per block in a row, with mipmaps down to one texel
/// a tile: each mip level halves every tile, so none bleeds into its neighbour. Bevy builds no
/// mipmaps itself, and without them textures shimmer as the camera moves. Magnified texels
/// stay crisp.
pub fn block_atlas(path: &str) -> Image {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("could not read {path}: {e}"));
    let mut image = Image::from_buffer(
        &bytes,
        ImageType::Extension("png"),
        CompressedImageFormats::NONE,
        true,
        ImageSampler::Descriptor(ImageSamplerDescriptor {
            mag_filter: ImageFilterMode::Nearest,
            min_filter: ImageFilterMode::Linear,
            mipmap_filter: ImageFilterMode::Linear,
            ..default()
        }),
        RenderAssetUsages::RENDER_WORLD,
    )
    .unwrap_or_else(|e| panic!("could not decode {path}: {e}"));
    let tile = image.height();
    mip_tiles(&mut image, tile.ilog2() + 1);
    image
}

// Appends `levels` - 1 box-filtered mip levels to an RGBA8 sRGB image, averaging in linear
// light and weighting colour by alpha so cut-out texels don't darken what's left.
fn mip_tiles(image: &mut Image, levels: u32) {
    let (mut width, mut height) = (image.width() as usize, image.height() as usize);
    let data = image.data.get_or_insert_with(Vec::new);
    let mut level = data.clone();
    for _ in 1..levels {
        let (next_width, next_height) = (width / 2, height / 2);
        let mut next = Vec::with_capacity(next_width * next_height * 4);
        for y in 0..next_height {
            for x in 0..next_width {
                let (mut rgb, mut alpha) = ([0.0f32; 3], 0.0f32);
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let at = ((2 * y + dy) * width + 2 * x + dx) * 4;
                    let texel = &level[at..at + 4];
                    let linear = LinearRgba::from(Srgba::rgb_u8(texel[0], texel[1], texel[2]));
                    let a = texel[3] as f32 / 255.0;
                    rgb[0] += linear.red * a;
                    rgb[1] += linear.green * a;
                    rgb[2] += linear.blue * a;
                    alpha += a;
                }
                let weight = alpha.max(f32::EPSILON);
                let colour = Srgba::from(LinearRgba::rgb(
                    rgb[0] / weight,
                    rgb[1] / weight,
                    rgb[2] / weight,
                ))
                .to_u8_array();
                next.extend_from_slice(&[
                    colour[0],
                    colour[1],
                    colour[2],
                    (alpha / 4.0 * 255.0).round() as u8,
                ]);
            }
        }
        data.extend_from_slice(&next);
        level = next;
        (width, height) = (next_width, next_height);
    }
    image.texture_descriptor.mip_level_count = levels;
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
#[bind_group_data(DebugSettings)]
pub struct ChunkMaterial {
    #[uniform(0)]
    pub color: LinearRgba,
    #[texture(1)]
    #[sampler(2)]
    pub texture: Option<Handle<Image>>,
    // the dither window drawn, see `fade::FadeMaterials`
    #[uniform(3)]
    pub fade: Vec4,
    // the clouds, to shade the land under them, see `clouds::CloudLayer`
    #[texture(4)]
    #[sampler(5)]
    pub cloud_pattern: Handle<Image>,
    #[uniform(6)]
    pub cloud_layer: Vec4,
    // what the land bends around, see `curvature::Curvature`
    #[storage(8, read_only)]
    pub curvature: Handle<ShaderBuffer>,
    // compiled into the shaders, see `specialize`
    pub debug: DebugSettings,
}

impl From<&ChunkMaterial> for DebugSettings {
    fn from(material: &ChunkMaterial) -> Self {
        material.debug
    }
}

impl Material for ChunkMaterial {
    // sways leaves in the wind
    fn vertex_shader() -> ShaderRef {
        "shaders/world.wesl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "shaders/world.wesl".into()
    }

    // sways leaves just as the main pass does, so they keep to their shadows
    fn prepass_vertex_shader() -> ShaderRef {
        "shaders/world_prepass.wesl".into()
    }

    // cuts the holes in leaves out of their shadows too
    fn prepass_fragment_shader() -> ShaderRef {
        "shaders/world_prepass.wesl".into()
    }

    // solid leaves have no holes to cut, so are drawn as opaque
    fn alpha_mode(&self) -> AlphaMode {
        if self.debug.solid_leaves {
            AlphaMode::Opaque
        } else {
            AlphaMode::Mask(0.5)
        }
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = Some(Face::Front);
        // the main and shadow passes both sway leaves, see `foliage.wesl`
        let leaf_cell = ShaderDefVal::UInt("LEAF_CELL".into(), Tile::Leaves.atlas_cell() as u32);
        // grass sides draw over dirt, and turn to grass tops from afar, see `world.wesl`
        let grass_top_cell =
            ShaderDefVal::UInt("GRASS_TOP_CELL".into(), Tile::GrassTop.atlas_cell() as u32);
        let dirt_cell = ShaderDefVal::UInt("DIRT_CELL".into(), Tile::Dirt.atlas_cell() as u32);
        // flora's sprites are cut out and sway at their tips, see `foliage.wesl`
        let first_flora_cell = ShaderDefVal::UInt(
            "FIRST_FLORA_CELL".into(),
            Tile::FIRST_FLORA.atlas_cell() as u32,
        );
        // leaves' holes fill in past where the faces between leaves are drawn, see `InnerLeaves`
        let inner_leaves_reach =
            ShaderDefVal::UInt("INNER_LEAVES_REACH".into(), INNER_LEAVES_REACH as u32);
        let inner_leaves_band =
            ShaderDefVal::UInt("INNER_LEAVES_BAND".into(), INNER_LEAVES_BAND as u32);
        let mut defs = vec![
            leaf_cell,
            inner_leaves_reach,
            inner_leaves_band,
            grass_top_cell,
            dirt_cell,
            first_flora_cell,
            water_level(),
        ];
        // compiled in or out, so the switches leave no trace of the code they turn off
        if key.bind_group_data.still_leaves {
            defs.push("STILL_LEAVES".into());
        }
        if key.bind_group_data.solid_leaves {
            defs.push("SOLID_LEAVES".into());
        }
        push_shader_defs(descriptor, &defs);
        Ok(())
    }
}

/// The materials terrain draws with, before any fading.
#[derive(Clone)]
pub struct TerrainMaterials {
    pub chunk: Handle<ChunkMaterial>,
    pub water: Handle<WaterMaterial>,
    pub solid_water: Handle<WaterMaterial>,
    pub spray: Handle<SprayMaterial>,
}

/// White water thrown up where fresh water runs steeply or falls, as particles the shader moves.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct SprayMaterial {
    // what the land bends around, see `curvature::Curvature`
    #[storage(8, read_only)]
    pub curvature: Handle<ShaderBuffer>,
}

impl Material for SprayMaterial {
    fn enable_shadows() -> bool {
        false
    }

    fn enable_prepass() -> bool {
        false
    }

    fn vertex_shader() -> ShaderRef {
        "shaders/spray.wesl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "shaders/spray.wesl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // each particle is turned to face the camera in the shader, whichever way it was wound
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
#[bind_group_data(WaterKey)]
pub struct WaterMaterial {
    #[uniform(0)]
    pub color: LinearRgba,
    #[texture(1)]
    #[sampler(2)]
    pub texture: Option<Handle<Image>>,
    // the dither window drawn, see `fade::FadeMaterials`
    #[uniform(3)]
    pub fade: Vec4,
    // the clouds, for water to reflect and shade its glint by, see `clouds::CloudLayer`
    #[texture(4)]
    #[sampler(5)]
    pub cloud_pattern: Handle<Image>,
    #[uniform(6)]
    pub cloud_layer: Vec4,
    // between these distances in blocks, x to y, see-through water turns opaque, so it
    // matches the opaque water LODs draw farther out, see `see_through_reach`
    #[uniform(7)]
    pub see_through: Vec4,
    // what the land bends around, see `curvature::Curvature`
    #[storage(8, read_only)]
    pub curvature: Handle<ShaderBuffer>,
    // drawn with the opaque meshes, which Bevy batches, see `lod::SOLID_WATER_LEVEL`
    pub solid: bool,
}

/// Where see-through water turns opaque for `WaterMaterial::see_through`: by the nearest
/// LOD water drawn opaque can be, where a node of `lod::SOLID_WATER_LEVEL` is no longer
/// split, see `LodLayout::splits`. None at all if no level is that coarse.
pub fn see_through_reach(layout: LodLayout) -> Vec4 {
    if layout.max_level < SOLID_WATER_LEVEL {
        return Vec4::new(f32::MAX, f32::MAX, 0.0, 0.0);
    }
    let opaque = layout.reach(SOLID_WATER_LEVEL - 1) * CHUNK_SIZE as f32;
    Vec4::new(0.5 * opaque, 0.95 * opaque, 0.0, 0.0)
}

// Compiled into water's shaders, see `WaterMaterial::specialize`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WaterKey {
    solid: bool,
}

impl From<&WaterMaterial> for WaterKey {
    fn from(material: &WaterMaterial) -> Self {
        Self {
            solid: material.solid,
        }
    }
}

impl Material for WaterMaterial {
    // light passes through water, so it casts no shadow
    fn enable_shadows() -> bool {
        false
    }

    // its waves would put a prepass's depth off the main pass's
    fn enable_prepass() -> bool {
        false
    }

    fn vertex_shader() -> ShaderRef {
        "shaders/water.wesl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "shaders/water.wesl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        if self.solid {
            AlphaMode::Opaque
        } else {
            AlphaMode::Blend
        }
    }

    // fresh water shows the scene under it through ripples and reflects the world about it, so reads
    // a copy of the opaque scene, and is drawn after it
    fn reads_view_transmission_texture(&self) -> bool {
        !self.solid
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // unlike ChunkMaterial, water has no back-face culling: the player can swim
        // underneath the lowered surface and must still see it rendered from below.
        descriptor.primitive.cull_mode = None;
        let mut defs = vec![water_level()];
        // drawn opaque, so it shows the sea as it is, not blended over what's under it
        if key.bind_group_data.solid {
            defs.push("SOLID_WATER".into());
        }
        push_shader_defs(descriptor, &defs);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use bevy::color::{LinearRgba, Srgba};

    #[test]
    fn test_see_through_water_is_opaque_before_lods_draw_it_opaque() {
        use crate::lod::{LodLayout, SOLID_WATER_LEVEL};
        let layout = LodLayout::new(16, 512, 2);
        let reach = super::see_through_reach(layout);
        // the nearest a node drawing opaque water can be, in blocks
        let opaque = layout.reach(SOLID_WATER_LEVEL - 1) * 16.0;
        assert!(
            reach.x < reach.y && reach.y < opaque,
            "{reach} for opaque from {opaque}"
        );
        // with no level that coarse, water stays see-through however far out
        assert!(super::see_through_reach(LodLayout::new(16, 16, 2)).x > 1e30);
    }

    // the workspace's assets, wherever the tests run from
    fn asset(path: &str) -> String {
        format!("{}/../../assets/{path}", env!("CARGO_MANIFEST_DIR"))
    }

    #[test]
    fn test_atlas_mips_end_at_one_texel_a_tile() {
        let atlas = super::block_atlas(&asset("textures/blocks.png"));
        let tile = atlas.height() as usize;
        let levels = tile.ilog2() as usize + 1;
        assert_eq!(levels as u32, atlas.texture_descriptor.mip_level_count);
        let texels: usize = (0..levels)
            .map(|l| (atlas.width() as usize >> l) * (tile >> l))
            .sum();
        assert_eq!(texels * 4, atlas.data.as_ref().unwrap().len());
        // the last level has one texel for each of the atlas's tiles
        assert_eq!(
            atlas.width() as usize / tile,
            atlas.width() as usize >> (levels - 1)
        );
    }

    // `block_mean`'s colours in block_texture.wesl, in atlas order
    fn shader_means() -> Vec<[f32; 3]> {
        let shader = std::fs::read_to_string(asset("shaders/block_texture.wesl")).unwrap();
        shader
            .lines()
            .filter_map(|line| line.trim().strip_prefix("vec3<f32>(")?.split(')').next())
            .map(|args| {
                let v: Vec<f32> = args.split(',').map(|a| a.trim().parse().unwrap()).collect();
                [v[0], v[1], v[2]]
            })
            .collect()
    }

    // The mean linear colour of the texels each atlas cell draws, those the shaders don't cut out.
    fn atlas_means() -> Vec<[f32; 3]> {
        let atlas = image::open(asset("textures/blocks.png"))
            .unwrap()
            .to_rgba8();
        let tile = atlas.height();
        (0..atlas.width() / tile)
            .map(|cell| {
                let drawn: Vec<LinearRgba> = (0..tile)
                    .flat_map(|y| (0..tile).map(move |x| (x, y)))
                    .map(|(x, y)| atlas.get_pixel(cell * tile + x, y).0)
                    .filter(|p| p[3] >= 128)
                    .map(|p| Srgba::rgb_u8(p[0], p[1], p[2]).into())
                    .collect();
                let mean = |channel: fn(&LinearRgba) -> f32| {
                    drawn.iter().map(channel).sum::<f32>() / drawn.len() as f32
                };
                [mean(|c| c.red), mean(|c| c.green), mean(|c| c.blue)]
            })
            .collect()
    }

    #[test]
    fn test_block_means_match_the_atlas() {
        let (shader, atlas) = (shader_means(), atlas_means());
        assert_eq!(atlas.len(), shader.len(), "one mean per atlas cell");
        for (cell, (s, a)) in shader.iter().zip(&atlas).enumerate() {
            for i in 0..3 {
                assert!(
                    (s[i] - a[i]).abs() < 0.001,
                    "atlas cell {cell}: shader has {s:?}, blocks.png averages {a:?}"
                );
            }
        }
    }
}
