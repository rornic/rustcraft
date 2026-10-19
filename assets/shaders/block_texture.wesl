// ChunkMaterial's bindings and atlas lookup, shared by its main and shadow passes.

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> material_color: vec4<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var material_color_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var material_color_sampler: sampler;

@group(constants::MATERIAL_BIND_GROUP) @binding(3) var<uniform> material_fade: vec4<f32>;

// textures in blocks.png, see `Tile` in block.rs
const ATLAS_CELLS: f32 = 22.0;
const TILE_INSET: f32 = 0.002;

// Just the alpha of `block_sample`, at full detail: without screen-space derivatives, so
// it's fine where only some fragments sample, as the shadow pass does for leaves.
fn block_alpha(uv: vec2<f32>, vertex_alpha: f32) -> f32 {
    return block_alpha_at(uv, vertex_alpha, 0.0);
}

// `block_alpha` from mip `level`, which averages away holes smaller than the lookup's footprint.
fn block_alpha_at(uv: vec2<f32>, vertex_alpha: f32, level: f32) -> f32 {
    let atlas_cell = floor(vertex_alpha + 0.5);
    // a coarse level's texels are wider, so the lookup stays half of one inside the tile
    let inset = max(TILE_INSET, 0.5 * exp2(level) / f32(textureDimensions(material_color_texture).y));
    let tile_uv = clamp(fract(uv), vec2<f32>(inset), vec2<f32>(1.0 - inset));
    let atlas_uv = vec2<f32>((atlas_cell + tile_uv.x) / ATLAS_CELLS, tile_uv.y);
    return textureSampleLevel(material_color_texture, material_color_sampler, atlas_uv, level).a * material_color.a;
}

// How a block face's uv covers the screen, for `block_sample`. It takes derivatives, so needs
// uniform control flow, while `block_sample` doesn't: work it out once, then sample any tiles.
struct BlockFootprint {
    dx: vec2<f32>,
    dy: vec2<f32>,
    inset: f32,
}

fn block_footprint(uv: vec2<f32>) -> BlockFootprint {
    // Blurry mip levels' texels are wider, and bilinear filtering would blend the edge ones
    // with the next tile's, so the lookup stays half a texel of the coarsest level in use inside.
    let tile_texels = f32(textureDimensions(material_color_texture).y);
    let footprint = max(length(vec2<f32>(dpdx(uv.x), dpdx(uv.y))), length(vec2<f32>(dpdy(uv.x), dpdy(uv.y)))) * tile_texels;
    let coarsest = exp2(clamp(ceil(log2(max(footprint, 1.0))), 0.0, log2(tile_texels)));
    return BlockFootprint(
        vec2<f32>(dpdx(uv.x) / ATLAS_CELLS, dpdx(uv.y)),
        vec2<f32>(dpdy(uv.x) / ATLAS_CELLS, dpdy(uv.y)),
        max(TILE_INSET, 0.5 * coarsest / tile_texels),
    );
}

// Block meshes put tile counts in uv and the atlas cell in vertex alpha, so merged faces
// repeat their texture.
fn block_sample(uv: vec2<f32>, vertex_alpha: f32, footprint: BlockFootprint) -> vec4<f32> {
    let atlas_cell = floor(vertex_alpha + 0.5);
    let tile_uv = clamp(fract(uv), vec2<f32>(footprint.inset), vec2<f32>(1.0 - footprint.inset));
    let atlas_uv = vec2<f32>((atlas_cell + tile_uv.x) / ATLAS_CELLS, tile_uv.y);
    let texel = textureSampleGrad(material_color_texture, material_color_sampler, atlas_uv, footprint.dx, footprint.dy);
    return material_color * texel;
}

// What a tile averages to: the mean linear colour of each atlas cell's opaque texels, in atlas
// order. `gen_atlas` prints these and the block-textures skill's `pack.py`
// rewrites them, and `material.rs`'s tests check them against blocks.png.
fn block_mean(vertex_alpha: f32) -> vec4<f32> {
    var means = array<vec3<f32>, 22>(
        vec3<f32>(0.1182, 0.1186, 0.1186), // stone
        vec3<f32>(0.5003, 0.5003, 0.5003), // grass top
        vec3<f32>(0.4776, 0.3611, 0.1822), // sand
        vec3<f32>(0.0665, 0.1023, 0.9149), // water
        vec3<f32>(0.9308, 0.9726, 0.9921), // snow
        vec3<f32>(0.0776, 0.0449, 0.0211), // log bark
        vec3<f32>(0.2433, 0.2433, 0.2433), // leaves
        vec3<f32>(0.5106, 0.5106, 0.5106), // grass side
        vec3<f32>(0.2611, 0.1228, 0.0581), // dirt
        vec3<f32>(0.4083, 0.2484, 0.0983), // log end
        vec3<f32>(0.2623, 0.2423, 0.2180), // gravel
        vec3<f32>(0.3090, 0.6154, 0.8315), // ice
        vec3<f32>(0.6178, 0.4855, 0.2738), // sandstone side
        vec3<f32>(0.6696, 0.5261, 0.2962), // sandstone top
        vec3<f32>(0.4256, 0.3341, 0.2906), // snow side
        vec3<f32>(0.0414, 0.0436, 0.0504), // compacted stone
        vec3<f32>(0.5186, 0.5186, 0.5186), // tall grass
        vec3<f32>(0.3112, 0.3112, 0.3112), // fern
        vec3<f32>(0.2675, 0.0986, 0.0166), // poppy
        vec3<f32>(0.3420, 0.3242, 0.0266), // dandelion
        vec3<f32>(0.5175, 0.5175, 0.5175), // short grass
        vec3<f32>(0.5246, 0.5246, 0.5246), // seed grass
    );
    let atlas_cell = u32(floor(vertex_alpha + 0.5));
    return material_color * vec4<f32>(means[atlas_cell], 1.0);
}
