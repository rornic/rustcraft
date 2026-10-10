use bevy::math::I64Vec2;
use noise::{
    Clamp, Fbm, MultiFractal, NoiseFn, Perlin, ScalePoint, Seedable, Select, Turbulence,
};

pub fn world_noise(seed: u32) -> impl NoiseFn<f64, 2> + Send + Sync {
    let scale: f64 = 1.0 / 1024.0;

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

    let combined = Select::new(base_continents, mountains, base_continents_tu)
        .set_bounds(0.2, 1.0)
        .set_falloff(0.1);

    // not wrapped in the noise crate's Cache combinator, which isn't Sync - it can't be
    // shared across generation threads.
    Clamp::new(ScalePoint::new(combined).set_scale(scale))
        .set_lower_bound(0.0)
        .set_upper_bound(10.0)
}

// Sampled directly with no cache: region erosion is the only caller, and a shared cache
// measured slower - its locking between generation threads cost more than the overlap
// between neighbouring regions' samples saved.
pub struct NoiseGenerator {
    source: Box<dyn NoiseFn<f64, 2> + Send + Sync>,
}

impl NoiseGenerator {
    pub fn new(seed: u32) -> Self {
        Self {
            source: Box::new(world_noise(seed)),
        }
    }

    pub fn get(&self, pos: I64Vec2) -> f64 {
        self.source.get([pos.x as f64, pos.y as f64])
    }
}
