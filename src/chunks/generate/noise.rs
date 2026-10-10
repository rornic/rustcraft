use bevy::math::I64Vec2;
use noise::{Fbm, MultiFractal, NoiseFn, Perlin};

use super::generator::WATER_LEVEL;

// The world is a finite square centred on the origin, ringed by deep ocean.
pub const WORLD_SIZE: i64 = 32_768;
pub const WORLD_HALF_SIZE: i64 = WORLD_SIZE / 2;

// Leaves headroom under the 512 block world height for erosion deposits.
const MAX_HEIGHT: f64 = 496.0;

// Each field gets its own block of seeds, since a fractal seeds its octaves seed, seed+1, ...
const SEED_STRIDE: u32 = 101;

// Domain warp bends continents into peninsulas and bays instead of round blobs.
const WARP_WAVELENGTH: f64 = 5000.0;
const WARP_STRENGTH: f64 = 1800.0;

// Continentalness: few octaves, so coastlines are long and smooth rather than archipelagos.
const CONTINENT_WAVELENGTH: f64 = 9000.0;
// Shifts continentalness down so most of the world is ocean.
const OCEAN_BIAS: f64 = 0.08;

// Ruggedness decides where inland is calm plains and where it's mountain ranges.
const RUGGEDNESS_WAVELENGTH: f64 = 4500.0;

const RIDGE_WAVELENGTH: f64 = 1100.0;
const RIDGE_OCTAVES: usize = 7;
const RIDGE_WARP: f64 = 0.3;
// A range is a broad uplifted massif with ridged peaks and valleys carved into it.
const UPLIFT_HEIGHT: f64 = 150.0;
const RIDGE_HEIGHT: f64 = 190.0;

// Hills between the coast and the mountains, strong in some areas and gentle in others.
const HILL_WAVELENGTH: f64 = 900.0;
const HILLINESS_WAVELENGTH: f64 = 2500.0;
const HILL_HEIGHT: f64 = 45.0;

const DETAIL_WAVELENGTH: f64 = 350.0;

// Where the ocean ring around the world starts and ends, as a fraction of the half size.
const EDGE_FALLOFF_START: f64 = 0.8;
const EDGE_FALLOFF_END: f64 = 0.97;

// Below this continentalness the sea floor only gets plain detail noise.
const OPEN_OCEAN: f64 = -0.25;
const SEA_FLOOR_DETAIL: f64 = 3.0;

// Continentalness to height above sea level: deep ocean, continental shelf, coast, inland.
const CONTINENT_SPLINE: [(f64, f64); 10] = [
    (-1.0, -46.0),
    (-0.4, -44.0),
    (-0.22, -30.0),
    (-0.12, -10.0),
    (-0.03, -3.0),
    (0.0, 0.5),
    (0.015, 3.0),
    (0.05, 8.0),
    (0.2, 18.0),
    (0.5, 36.0),
];

// The large scale fields behind a column's height, kept so later passes (biomes,
// rivers, a baked world map) can build on them.
#[derive(Debug, Copy, Clone)]
pub struct TerrainSample {
    pub continentalness: f64,
    pub ruggedness: f64,
    pub height: f64,
}

pub struct NoiseGenerator {
    warp_x: Fbm<Perlin>,
    warp_z: Fbm<Perlin>,
    continents: Fbm<Perlin>,
    ruggedness: Fbm<Perlin>,
    ridges: Vec<Perlin>,
    hills: Fbm<Perlin>,
    hilliness: Fbm<Perlin>,
    detail: Fbm<Perlin>,
}

impl NoiseGenerator {
    pub fn new(seed: u32) -> Self {
        let field_seed = |i: u32| seed.wrapping_add(i * SEED_STRIDE);
        let fbm = |i: u32, wavelength: f64, octaves: usize| {
            Fbm::<Perlin>::new(field_seed(i))
                .set_frequency(1.0 / wavelength)
                .set_octaves(octaves)
                .set_persistence(0.5)
                .set_lacunarity(2.0)
        };
        Self {
            warp_x: fbm(0, WARP_WAVELENGTH, 3),
            warp_z: fbm(1, WARP_WAVELENGTH, 3),
            continents: fbm(2, CONTINENT_WAVELENGTH, 5),
            ruggedness: fbm(3, RUGGEDNESS_WAVELENGTH, 3),
            ridges: (0..RIDGE_OCTAVES as u32)
                .map(|i| Perlin::new(field_seed(4).wrapping_add(i)))
                .collect(),
            detail: fbm(5, DETAIL_WAVELENGTH, 5),
            hills: fbm(6, HILL_WAVELENGTH, 4),
            hilliness: fbm(7, HILLINESS_WAVELENGTH, 2),
        }
    }

    // Column height in blocks.
    pub fn get(&self, pos: I64Vec2) -> f64 {
        self.sample(pos.x as f64, pos.y as f64).height
    }

    pub fn sample(&self, x: f64, z: f64) -> TerrainSample {
        let warp_x = self.warp_x.get([x, z]) * WARP_STRENGTH;
        let warp_z = self.warp_z.get([x, z]) * WARP_STRENGTH;
        let (wx, wz) = (x + warp_x, z + warp_z);

        let edge = edge_falloff(wx, wz);
        let continentalness = lerp(self.continents.get([wx, wz]) - OCEAN_BIAS, -1.0, edge);
        // Ruggedness and hilliness only shape land and the shallows, so open ocean skips them.
        let ruggedness = if continentalness > OPEN_OCEAN {
            self.ruggedness.get([wx, wz])
        } else {
            0.0
        };

        let base = spline(&CONTINENT_SPLINE, continentalness);

        let inland = smoothstep(0.02, 0.25, continentalness);
        let mountain_mask = inland * smoothstep(-0.3, 0.35, ruggedness);
        let mountains = if mountain_mask > 0.0 {
            // A full strength warp shears ridges into long streaks, so they get only a little.
            let ridge_x = x + warp_x * RIDGE_WARP;
            let ridge_z = z + warp_z * RIDGE_WARP;
            let uplift = smoothstep(0.0, 1.0, mountain_mask) * UPLIFT_HEIGHT;
            // Peaks grow towards the heart of a range, leaving foothills at its edges.
            let ridges = self.ridged(ridge_x, ridge_z).powf(1.3) * mountain_mask.powf(1.5) * RIDGE_HEIGHT;
            uplift + ridges
        } else {
            0.0
        };

        // Kept above zero, so hills rise out of flat valley floors instead of dipping into lakes.
        let hills = if continentalness > 0.0 {
            let hilliness = smoothstep(-0.25, 0.3, self.hilliness.get([wx, wz]));
            let hill_amplitude = lerp(0.15, 1.0, hilliness) * HILL_HEIGHT * smoothstep(0.0, 0.04, continentalness);
            (self.hills.get([x, z]) * 0.8 + 0.3).max(0.0) * hill_amplitude
        } else {
            0.0
        };

        // Calm the detail near the shoreline, or it breaks the coast up into specks of land.
        let shore_calm = lerp(0.2, 1.0, smoothstep(0.0, 0.1, continentalness.abs()));
        let land_detail = lerp(3.0, 12.0, smoothstep(-0.3, 0.4, ruggedness));
        let sea_floor_blend = smoothstep(OPEN_OCEAN, OPEN_OCEAN + 0.1, continentalness);
        let detail_amplitude = lerp(SEA_FLOOR_DETAIL, land_detail, sea_floor_blend) * shore_calm;
        let detail = self.detail.get([x, z]) * detail_amplitude;

        let height = (WATER_LEVEL as f64 + base + hills + mountains + detail).clamp(0.0, MAX_HEIGHT);
        TerrainSample {
            continentalness,
            ruggedness,
            height,
        }
    }

    // Ridged multifractal in 0..1: sharp crests where the noise crosses zero, with each
    // octave weighted by the last so fine detail gathers on the ridges, not the valleys.
    fn ridged(&self, x: f64, z: f64) -> f64 {
        let mut frequency = 1.0 / RIDGE_WAVELENGTH;
        let mut amplitude = 1.0;
        let mut weight = 1.0;
        let mut total = 0.0;
        let mut norm = 0.0;
        for octave in &self.ridges {
            let signal = 1.0 - octave.get([x * frequency, z * frequency]).abs();
            let signal = signal * signal * weight;
            weight = (signal * 1.5).clamp(0.0, 1.0);
            total += signal * amplitude;
            norm += amplitude;
            amplitude *= 0.5;
            frequency *= 2.0;
        }
        total / norm
    }

    // The nearest lowland column to the world centre, searched outwards in rings.
    pub fn find_spawn(&self) -> (i64, i64, f64) {
        const STEP: i64 = 64;
        for ring in 0..(WORLD_HALF_SIZE / STEP) {
            for i in -ring..=ring {
                for (x, z) in [(i, -ring), (i, ring), (-ring, i), (ring, i)] {
                    let (x, z) = (x * STEP, z * STEP);
                    let height = self.get(I64Vec2::new(x, z));
                    let above_sea = height - WATER_LEVEL as f64;
                    if (4.0..30.0).contains(&above_sea) {
                        return (x, z, height);
                    }
                }
            }
        }
        (0, 0, self.get(I64Vec2::ZERO))
    }
}

// 0 inside the world, rising to 1 in the ocean ring at its edge.
fn edge_falloff(x: f64, z: f64) -> f64 {
    let distance = x.abs().max(z.abs()) / WORLD_HALF_SIZE as f64;
    smoothstep(EDGE_FALLOFF_START, EDGE_FALLOFF_END, distance)
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

fn smoothstep(edge0: f64, edge1: f64, x: f64) -> f64 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

// Piecewise linear through `points`, which must be sorted by x; clamps beyond the ends.
fn spline(points: &[(f64, f64)], x: f64) -> f64 {
    let (first, last) = (points[0], points[points.len() - 1]);
    if x <= first.0 {
        return first.1;
    }
    if x >= last.0 {
        return last.1;
    }
    let i = points.iter().position(|p| p.0 > x).unwrap();
    let (x0, y0) = points[i - 1];
    let (x1, y1) = points[i];
    lerp(y0, y1, (x - x0) / (x1 - x0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spline_interpolates_and_clamps() {
        let points = [(0.0, 0.0), (1.0, 10.0), (2.0, 30.0)];
        assert_eq!(0.0, spline(&points, -5.0));
        assert_eq!(5.0, spline(&points, 0.5));
        assert_eq!(20.0, spline(&points, 1.5));
        assert_eq!(30.0, spline(&points, 9.0));
    }

    #[test]
    fn test_world_edge_is_deep_ocean() {
        let noise = NoiseGenerator::new(9);
        for (x, z) in [(WORLD_HALF_SIZE, 0), (0, -WORLD_HALF_SIZE - 2000), (40_000, 40_000)] {
            let height = noise.get(I64Vec2::new(x, z));
            assert!(height < WATER_LEVEL as f64 - 30.0, "({x}, {z}) should be deep ocean, was {height}");
        }
    }

    #[test]
    fn test_spawn_is_on_land() {
        let noise = NoiseGenerator::new(9);
        let (_, _, height) = noise.find_spawn();
        assert!(height > WATER_LEVEL as f64);
    }
}
