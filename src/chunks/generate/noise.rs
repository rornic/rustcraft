use std::collections::VecDeque;
use std::sync::RwLock;

use bevy::{math::I64Vec2, utils::HashMap};
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

    // our own keyed cache (below) supersedes the noise crate's single-last-value Cache
    // combinator, which also isn't Sync - it can't be shared across generation threads.
    Clamp::new(ScalePoint::new(combined).set_scale(scale))
        .set_lower_bound(0.0)
        .set_upper_bound(10.0)
}

// Shared across every chunk in a vertical stack at the same (world_x, world_z) - e.g.
// all the air chunks generated above a land column hit the same cache entries to learn
// there's nothing to generate there - so this is kept global rather than scoped to one
// chunk's generation. Bounded with FIFO eviction (not true LRU - no extra dependency for
// it) so it can't grow forever across a long session or a large explored area; the cap
// is sized as headroom over a realistic in-flight generation burst (bounded by core
// count and the loader's per-frame spawn cap, not by render distance), so normal play
// keeps full cache benefit while worst-case memory stays a few tens of MB.
const NOISE_CACHE_CAP: usize = 500_000;

struct NoiseCache {
    map: HashMap<I64Vec2, f64>,
    order: VecDeque<I64Vec2>,
}

impl NoiseCache {
    fn new() -> Self {
        Self {
            map: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    fn get(&self, pos: I64Vec2) -> Option<f64> {
        self.map.get(&pos).copied()
    }

    fn insert(&mut self, pos: I64Vec2, value: f64) {
        if self.map.insert(pos, value).is_some() {
            return; // already present - not a new entry, don't grow `order`
        }
        self.order.push_back(pos);
        if self.order.len() > NOISE_CACHE_CAP {
            if let Some(oldest) = self.order.pop_front() {
                self.map.remove(&oldest);
            }
        }
    }
}

pub struct NoiseGenerator {
    // RwLock, not RefCell: get() is called concurrently from many chunk-generation tasks
    // on the async task pool, so the cache needs real cross-thread synchronization, not
    // just interior mutability for a single-threaded borrower.
    cache: RwLock<NoiseCache>,
    source: Box<dyn NoiseFn<f64, 2> + Send + Sync>,
}

impl NoiseGenerator {
    pub fn new(seed: u32) -> Self {
        Self {
            cache: RwLock::new(NoiseCache::new()),
            source: Box::new(world_noise(seed)),
        }
    }
}

impl NoiseGenerator {
    pub fn get(&self, pos: I64Vec2) -> f64 {
        if let Some(value) = self.cache.read().unwrap().get(pos) {
            return value;
        }

        let value = self.source.get([pos.x as f64, pos.y as f64]);
        self.cache.write().unwrap().insert(pos, value);

        value
    }
}

#[cfg(test)]
mod tests {
    use super::{NoiseCache, NOISE_CACHE_CAP};
    use bevy::math::I64Vec2;

    #[test]
    fn test_eviction_removes_oldest_entry_not_arbitrary() {
        let mut cache = NoiseCache::new();
        for i in 0..NOISE_CACHE_CAP {
            cache.insert(I64Vec2::new(i as i64, 0), i as f64);
        }
        assert!(cache.get(I64Vec2::new(0, 0)).is_some());

        // one more insertion should evict exactly the oldest entry (key 0)
        cache.insert(I64Vec2::new(NOISE_CACHE_CAP as i64, 0), 0.0);

        assert!(cache.get(I64Vec2::new(0, 0)).is_none());
        assert!(cache.get(I64Vec2::new(1, 0)).is_some());
        assert!(cache.get(I64Vec2::new(NOISE_CACHE_CAP as i64, 0)).is_some());
    }

    #[test]
    fn test_cache_hit_does_not_grow_order() {
        let mut cache = NoiseCache::new();
        cache.insert(I64Vec2::new(0, 0), 1.0);
        cache.insert(I64Vec2::new(0, 0), 2.0); // overwrite, same key

        assert_eq!(1, cache.order.len());
        assert_eq!(Some(2.0), cache.get(I64Vec2::new(0, 0)));
    }
}
