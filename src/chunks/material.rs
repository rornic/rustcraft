use bevy::{
    asset::Asset,
    pbr::{MaterialPipeline, MaterialPipelineKey},
    prelude::*,
    reflect::TypePath,
    render::{
        mesh::MeshVertexBufferLayoutRef,
        render_resource::{
            AsBindGroup, Face, RenderPipelineDescriptor, ShaderDefVal, ShaderRef,
            SpecializedMeshPipelineError,
        },
    },
};

use crate::block::BlockType;
use crate::chunks::generate::WATER_LEVEL;

// Sea level, where `fog.wgsl` turns from fog across the land to murk all round.
pub(crate) fn water_level() -> ShaderDefVal {
    ShaderDefVal::Int("WATER_LEVEL".into(), WATER_LEVEL as i32)
}

// Every stage needs every def its file's imports use, since each is built from the whole file.
pub(crate) fn push_shader_defs(descriptor: &mut RenderPipelineDescriptor, defs: &[ShaderDefVal]) {
    descriptor.vertex.shader_defs.extend_from_slice(defs);
    if let Some(fragment) = &mut descriptor.fragment {
        fragment.shader_defs.extend_from_slice(defs);
    }
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
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
}

impl Material for ChunkMaterial {
    // sways leaves in the wind
    fn vertex_shader() -> ShaderRef {
        "shaders/world.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "shaders/world.wgsl".into()
    }

    // sways leaves just as the main pass does, so they keep to their shadows
    fn prepass_vertex_shader() -> ShaderRef {
        "shaders/world_prepass.wgsl".into()
    }

    // cuts the holes in leaves out of their shadows too
    fn prepass_fragment_shader() -> ShaderRef {
        "shaders/world_prepass.wgsl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Mask(0.5)
    }

    fn specialize(
        _pipeline: &MaterialPipeline<Self>,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = Some(Face::Front);
        // the main and shadow passes both sway leaves, see `foliage.wgsl`
        let leaf_cell = ShaderDefVal::UInt("LEAF_CELL".into(), BlockType::Leaves as u32 - 1);
        push_shader_defs(descriptor, &[leaf_cell, water_level()]);
        Ok(())
    }
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct WaterMaterial {
    #[uniform(0)]
    pub color: LinearRgba,
    #[texture(1)]
    #[sampler(2)]
    pub texture: Option<Handle<Image>>,
    // the dither window drawn, see `fade::FadeMaterials`
    #[uniform(3)]
    pub fade: Vec4,
}

impl Material for WaterMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/water.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "shaders/water.wgsl".into()
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
        // unlike ChunkMaterial, water has no back-face culling: the player can swim
        // underneath the lowered surface and must still see it rendered from below.
        descriptor.primitive.cull_mode = None;
        push_shader_defs(descriptor, &[water_level()]);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use bevy::color::{LinearRgba, Srgba};

    // `block_mean`'s colours in block_texture.wgsl, in atlas order
    fn shader_means() -> Vec<[f32; 3]> {
        let shader = std::fs::read_to_string("assets/shaders/block_texture.wgsl").unwrap();
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
        let atlas = image::open("assets/textures/blocks.png").unwrap().to_rgba8();
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
                assert!((s[i] - a[i]).abs() < 0.001, "atlas cell {cell}: shader has {s:?}, blocks.png averages {a:?}");
            }
        }
    }
}
