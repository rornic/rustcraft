use bevy::{
    asset::Asset,
    pbr::{MaterialPipeline, MaterialPipelineKey},
    prelude::*,
    reflect::TypePath,
    render::{
        mesh::MeshVertexBufferLayoutRef,
        render_resource::{
            AsBindGroup, Face, RenderPipelineDescriptor, ShaderRef, SpecializedMeshPipelineError,
        },
    },
};

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
        Ok(())
    }
}
