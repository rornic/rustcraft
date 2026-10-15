// An ordered 8x8 dither, for fading things in and out without blending: a fragment shows
// when its threshold is under how visible the thing is. 64 levels, so a fade steps finely.

fn dither_threshold(frag_coord: vec2<f32>) -> f32 {
    // the Bayer matrix's entry, built from the bits of x ^ y and y, interleaved and reversed
    let p = vec2<u32>(frag_coord) % 8u;
    let z = p.x ^ p.y;
    let level = ((z & 1u) << 5u) | ((p.y & 1u) << 4u) | ((z & 2u) << 2u) | ((p.y & 2u) << 1u) | ((z & 4u) >> 1u) | ((p.y & 4u) >> 2u);
    return (f32(level) + 0.5) / 64.0;
}
