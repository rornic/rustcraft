// An ordered 4x4 dither, for fading things in and out without blending: a fragment shows
// when its threshold is under how visible the thing is.

fn dither_threshold(frag_coord: vec2<f32>) -> f32 {
    var bayer = array<f32, 16>(0.0, 8.0, 2.0, 10.0, 12.0, 4.0, 14.0, 6.0, 3.0, 11.0, 1.0, 9.0, 15.0, 7.0, 13.0, 5.0);
    let p = vec2<u32>(frag_coord) % 4u;
    return (bayer[p.y * 4u + p.x] + 0.5) / 16.0;
}
