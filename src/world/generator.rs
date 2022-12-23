use noise::{Clamp, Fbm, MultiFractal, NoiseFn, Perlin, ScalePoint, Select};

pub fn noise_generator() -> impl NoiseFn<f64, 2> {
    let seed: u32 = 0;
    let scale: f64 = 1.0 / 2048.0;

    let base = Fbm::<Perlin>::new(seed)
        .set_frequency(0.15)
        .set_persistence(0.5)
        .set_lacunarity(2.0)
        .set_octaves(30);

    let mountains_def = Fbm::<Perlin>::new(seed + 10)
        .set_frequency(0.7)
        .set_lacunarity(1.21)
        .set_octaves(3);

    let mountains = base.clone().set_frequency(0.7).set_octaves(30);

    let base_with_mountains = Select::new(base, mountains, mountains_def).set_falloff(0.5);

    // Scale and clamp
    let generator = ScalePoint::new(
        Clamp::new(base_with_mountains)
            .set_lower_bound(0.0)
            .set_upper_bound(1.0),
    )
    .set_scale(scale);
    generator
}
