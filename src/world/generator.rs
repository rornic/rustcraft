use noise::{Cache, Fbm, MultiFractal, NoiseFn, Perlin, ScalePoint, Seedable, Select, Turbulence};

pub fn noise_generator(seed: u32) -> impl NoiseFn<f64, 2> {
    let scale: f64 = 1.0 / 2048.0;

    let freq = 0.2;
    let lacunarity = 2.2089;
    let base_continents = Fbm::<Perlin>::new(seed)
        .set_frequency(freq)
        .set_lacunarity(lacunarity)
        .set_octaves(7)
        .set_persistence(0.5);

    let base_continents_tu = Turbulence::<_, Perlin>::new(base_continents.clone())
        .set_seed(seed)
        .set_frequency(freq * 15.25)
        .set_power(1.0 / 40.75)
        .set_roughness(13);

    let mountains = base_continents
        .clone()
        .set_frequency(freq * 5.0)
        .set_octaves(32);

    let combined = Select::new(base_continents_tu.clone(), mountains, base_continents)
        .set_bounds(0.2, 1.0)
        .set_falloff(0.1);

    let generator = ScalePoint::new(combined).set_scale(scale);
    Cache::new(generator)
}
