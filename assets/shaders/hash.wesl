// 0..1 for the whole-number cell `p` is in. Integer mixing, since the usual fract(sin)
// hash loses its precision at world coordinates in the thousands and turns into stripes.
fn hash2(p: vec2<f32>) -> f32 {
    let cell = vec2<i32>(floor(p));
    var h = (bitcast<u32>(cell.x) * 0x8da6b343u) ^ (bitcast<u32>(cell.y) * 0xd8163841u);
    h = (h ^ (h >> 15u)) * 0x2c1b3c6du;
    h = (h ^ (h >> 12u)) * 0x297a2d39u;
    h = h ^ (h >> 15u);
    return f32(h) / 4294967295.0;
}

// 0..1 for the whole-number cell `cell`, as `hash2` is.
fn hash3(cell: vec3<i32>) -> f32 {
    var h = (bitcast<u32>(cell.x) * 0x8da6b343u) ^ (bitcast<u32>(cell.y) * 0xd8163841u) ^ (bitcast<u32>(cell.z) * 0xcb1ab31fu);
    h = (h ^ (h >> 15u)) * 0x2c1b3c6du;
    h = (h ^ (h >> 12u)) * 0x297a2d39u;
    h = h ^ (h >> 15u);
    return f32(h) / 4294967295.0;
}
