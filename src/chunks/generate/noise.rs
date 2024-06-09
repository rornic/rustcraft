use std::cell::RefCell;

use bevy::{math::I64Vec2, utils::HashMap};
use noise::{
    Cache, Clamp, Fbm, MultiFractal, NoiseFn, Perlin, ScalePoint, Seedable, Select, Turbulence,
};

pub fn world_noise(seed: u32) -> impl NoiseFn<f64, 2> {
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

    let generator = Clamp::new(ScalePoint::new(combined).set_scale(scale))
        .set_lower_bound(0.0)
        .set_upper_bound(10.0);

    Cache::new(generator)
}

pub struct NoiseGenerator {
    cache: RefCell<HashMap<I64Vec2, f64>>,
    source: Box<dyn NoiseFn<f64, 2>>,
}

unsafe impl Send for NoiseGenerator {}
unsafe impl Sync for NoiseGenerator {}

impl NoiseGenerator {
    pub fn new(seed: u32) -> Self {
        Self {
            cache: RefCell::new(HashMap::new()),
            source: Box::new(world_noise(seed)),
        }
    }
}

impl NoiseGenerator {
    pub fn get(&mut self, pos: I64Vec2) -> f64 {
        if self.cache.borrow().contains_key(&pos) {
            return *self.cache.borrow().get(&pos).unwrap();
        }

        let value = self.source.get([pos.x as f64, pos.y as f64]);
        self.cache.borrow_mut().insert(pos, value);

        value
    }
}
