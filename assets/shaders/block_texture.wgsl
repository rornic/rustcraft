// ChunkMaterial's bindings and atlas lookup, shared by its main and shadow passes.

@group(2) @binding(0) var<uniform> material_color: vec4<f32>;
@group(2) @binding(1) var material_color_texture: texture_2d<f32>;
@group(2) @binding(2) var material_color_sampler: sampler;

@group(2) @binding(3) var<uniform> material_fade: vec4<f32>;

// textures in blocks.png, one per block type but air
const ATLAS_CELLS: f32 = 7.0;
const TILE_INSET: f32 = 0.002;

// Just the alpha of `block_texel`, at full detail: without screen-space derivatives, so
// it's fine where only some fragments sample, as the shadow pass does for leaves.
fn block_alpha(uv: vec2<f32>, vertex_alpha: f32) -> f32 {
    let atlas_cell = floor(vertex_alpha + 0.5);
    let tile_uv = clamp(fract(uv), vec2<f32>(TILE_INSET), vec2<f32>(1.0 - TILE_INSET));
    let atlas_uv = vec2<f32>((atlas_cell + tile_uv.x) / ATLAS_CELLS, tile_uv.y);
    return textureSampleLevel(material_color_texture, material_color_sampler, atlas_uv, 0.0).a * material_color.a;
}

// Block meshes put tile counts in uv and the atlas cell in vertex alpha, so merged faces
// repeat their texture.
fn block_texel(uv: vec2<f32>, vertex_alpha: f32) -> vec4<f32> {
    let atlas_cell = floor(vertex_alpha + 0.5);
    // kept just inside the cell: a fract near 1.0 would round into the next block's texture
    let tile_uv = clamp(fract(uv), vec2<f32>(TILE_INSET), vec2<f32>(1.0 - TILE_INSET));
    let atlas_uv = vec2<f32>((atlas_cell + tile_uv.x) / ATLAS_CELLS, tile_uv.y);
    let atlas_dx = vec2<f32>(dpdx(uv.x) / ATLAS_CELLS, dpdx(uv.y));
    let atlas_dy = vec2<f32>(dpdy(uv.x) / ATLAS_CELLS, dpdy(uv.y));
    let texel = textureSampleGrad(material_color_texture, material_color_sampler, atlas_uv, atlas_dx, atlas_dy);
    return material_color * texel;
}
