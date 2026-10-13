// ChunkMaterial's bindings and atlas lookup, shared by its main and shadow passes.

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> material_color: vec4<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var material_color_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var material_color_sampler: sampler;

@group(constants::MATERIAL_BIND_GROUP) @binding(3) var<uniform> material_fade: vec4<f32>;

// textures in blocks.png, see `Tile` in block.rs
const ATLAS_CELLS: f32 = 15.0;
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
    let atlas_dx = vec2<f32>(dpdx(uv.x) / ATLAS_CELLS, dpdx(uv.y));
    let atlas_dy = vec2<f32>(dpdy(uv.x) / ATLAS_CELLS, dpdy(uv.y));
    // Blurry mip levels' texels are wider, and bilinear filtering would blend the edge ones
    // with the next tile's, so the lookup stays half a texel of the coarsest level in use inside.
    let tile_texels = f32(textureDimensions(material_color_texture).y);
    let footprint = max(length(vec2<f32>(dpdx(uv.x), dpdx(uv.y))), length(vec2<f32>(dpdy(uv.x), dpdy(uv.y)))) * tile_texels;
    let coarsest = exp2(clamp(ceil(log2(max(footprint, 1.0))), 0.0, log2(tile_texels)));
    let inset = max(TILE_INSET, 0.5 * coarsest / tile_texels);
    let tile_uv = clamp(fract(uv), vec2<f32>(inset), vec2<f32>(1.0 - inset));
    let atlas_uv = vec2<f32>((atlas_cell + tile_uv.x) / ATLAS_CELLS, tile_uv.y);
    let texel = textureSampleGrad(material_color_texture, material_color_sampler, atlas_uv, atlas_dx, atlas_dy);
    return material_color * texel;
}

// What a tile averages to: the mean linear colour of each atlas cell's opaque texels, in atlas
// order. `gen_atlas` prints these, and `material.rs`'s tests check them against blocks.png.
fn block_mean(vertex_alpha: f32) -> vec4<f32> {
    var means = array<vec3<f32>, 15>(
        vec3<f32>(0.1250, 0.1303, 0.1441), // stone
        vec3<f32>(0.5016, 0.5016, 0.5016), // grass top
        vec3<f32>(0.4721, 0.3520, 0.1795), // sand
        vec3<f32>(0.0643, 0.0980, 0.8945), // water
        vec3<f32>(0.9499, 0.9789, 0.9951), // snow
        vec3<f32>(0.0586, 0.0327, 0.0156), // log bark
        vec3<f32>(0.2500, 0.2500, 0.2500), // leaves
        vec3<f32>(0.1431, 0.1228, 0.0351), // grass side
        vec3<f32>(0.1616, 0.0849, 0.0413), // dirt
        vec3<f32>(0.3259, 0.1941, 0.0795), // log end
        vec3<f32>(0.1665, 0.1502, 0.1318), // gravel
        vec3<f32>(0.3799, 0.6297, 0.8646), // ice
        vec3<f32>(0.5894, 0.4148, 0.1934), // sandstone side
        vec3<f32>(0.6515, 0.4739, 0.2276), // sandstone top
        vec3<f32>(0.8942, 0.9485, 0.9850), // snow side
    );
    let atlas_cell = u32(floor(vertex_alpha + 0.5));
    return material_color * vec4<f32>(means[atlas_cell], 1.0);
}
