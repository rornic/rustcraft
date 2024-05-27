use bevy::{math::I64Vec2, utils::HashMap};
use noise::{Cache, Fbm, MultiFractal, NoiseFn, Perlin, ScalePoint, Seedable, Turbulence};

pub fn world_noise(seed: u32) -> impl NoiseFn<f64, 2> {
    let scale: f64 = 1.0 / 4096.0;

    let freq = 0.2;
    let lacunarity = 2.2089;
    let base_continents = Fbm::<Perlin>::new(seed)
        .set_frequency(freq)
        .set_lacunarity(lacunarity)
        .set_octaves(7)
        .set_persistence(0.5);

    let _ = Turbulence::<_, Perlin>::new(base_continents.clone())
        .set_seed(seed)
        .set_frequency(freq * 15.25)
        .set_power(1.0 / 40.75)
        .set_roughness(13);

    let mountains = base_continents
        .clone()
        .set_frequency(freq * 5.0)
        .set_lacunarity(lacunarity)
        .set_octaves(32);

    let generator = ScalePoint::new(mountains).set_scale(scale);
    Cache::new(generator)
}

pub struct NoiseGenerator {
    noise_cache: HashMap<I64Vec2, f64>,
}

impl Default for NoiseGenerator {
    fn default() -> Self {
        Self {
            noise_cache: HashMap::new(),
        }
    }
}

impl NoiseGenerator {
    pub fn get(&mut self, pos: I64Vec2, noise_fn: &impl NoiseFn<f64, 2>) -> f64 {
        if self.noise_cache.contains_key(&pos) {
            return *self.noise_cache.get(&pos).unwrap();
        }

        let value = noise_fn.get([pos.x as f64, pos.y as f64]);
        self.noise_cache.insert(pos, value);
        value
    }
}
