use bevy::math::{FloatExt, I64Vec2};
use noise::{Fbm, MultiFractal, NoiseFn, Perlin};

use super::fractal::{resolved, Fractal};
pub use super::terrain_source::TerrainSample;
use super::terrain_source::TerrainSource;

use super::biome::{smoothstep, BaseColumn, Biomes, Climate};
use super::{WATER_LEVEL, WORLD_HEIGHT};

// The world is a finite square centred on the origin, ringed by deep ocean.
pub const WORLD_SIZE: i64 = 32_768;
pub const WORLD_HALF_SIZE: i64 = WORLD_SIZE / 2;

// A little headroom under the top of the world.
const MAX_HEIGHT: f64 = WORLD_HEIGHT as f64 - 16.0;
// Peaks above this are eased towards MAX_HEIGHT instead of being cut off flat.
const PEAK_EASE_START: f64 = MAX_HEIGHT - 96.0;

// Each field gets its own block of seeds, since a fractal seeds its octaves seed, seed+1, ...
const SEED_STRIDE: u32 = 101;

// Domain warp bends continents into peninsulas and bays instead of round blobs.
const WARP_WAVELENGTH: f64 = 9000.0;
const WARP_STRENGTH: f64 = 3000.0;

// Continentalness: few octaves, so coastlines are long and smooth rather than archipelagos.
const CONTINENT_WAVELENGTH: f64 = 16000.0;
// Steepens continentalness, which a long wavelength leaves shallow, so coasts stay as sharp.
const CONTINENT_CONTRAST: f64 = 1.7;

// Ruggedness decides where inland is calm plains and where it's mountain ranges.
const RUGGEDNESS_WAVELENGTH: f64 = 7000.0;

const RIDGE_WAVELENGTH: f64 = 1500.0;
const RIDGE_OCTAVES: usize = 9;
const RIDGE_WARP: f64 = 0.18;
// The mean of a ridged octave's signal, (1 - |perlin|)^2, measured; it stands in for octaves
// too fine for the cells sampled, so leaving them out doesn't sink the peaks.
const RIDGE_SIGNAL_MEAN: f64 = 0.49;
// A range is a broad uplifted massif with ridged peaks and valleys carved into it.
const UPLIFT_HEIGHT: f64 = 190.0;
const RIDGE_HEIGHT: f64 = 230.0;

// Hills between the coast and the mountains, strong in some areas and gentle in others.
const HILL_WAVELENGTH: f64 = 900.0;
const HILLINESS_WAVELENGTH: f64 = 2500.0;
const HILL_HEIGHT: f64 = 45.0;

const DETAIL_WAVELENGTH: f64 = 350.0;
// How much detail lowlands, rugged country and mountain ranges get.
const PLAINS_DETAIL: f64 = 6.0;
const RUGGED_DETAIL: f64 = 18.0;
const MOUNTAIN_DETAIL: f64 = 30.0;

// Crags a few blocks across on mountains. Foothills bring their own, see `biome::Foothills`.
const TEXTURE_WAVELENGTH: f64 = 32.0;
const MOUNTAIN_TEXTURE: f64 = 16.0;

// Where the ocean ring around the world starts and ends, as a fraction of the half size, east and west.
const EDGE_FALLOFF_START: f64 = 0.8;
const EDGE_FALLOFF_END: f64 = 0.97;
// The same to the north and south, where land runs on to the poles.
const POLE_FALLOFF_START: f64 = 0.86;
const POLE_FALLOFF_END: f64 = 0.99;
// How much of the domain warp bends the north and south coast, little enough that it never pulls land over the world's edge.
const POLE_COAST_WARP: f64 = 0.3;
// Land is lifted towards each pole over this stretch of latitude, so an ice cap always stands there.
const POLAR_UPLIFT_LATITUDE: (f64, f64) = (0.7, 0.88);
const POLAR_UPLIFT: f64 = 1.0;

// Below this continentalness the sea floor only gets plain detail noise.
const OPEN_OCEAN: f64 = -0.25;
const SEA_FLOOR_DETAIL: f64 = 10.0;

// Temperature and humidity drift over many thousands of blocks, so a biome can fill much of a continent.
const CLIMATE_WAVELENGTH: f64 = 12000.0;
// Spreads the climate noise, which clusters around 0, out over the whole 0..1 range.
const CLIMATE_CONTRAST: f64 = 1.6;
// How much of the temperature is latitude (hot equator, cold poles) rather than noise.
const LATITUDE_WEIGHT: f64 = 0.6;
// Over this stretch of latitude the climate's noise is squeezed towards freezing, by up to POLAR_CHILL.
const POLAR_CHILL_LATITUDE: (f64, f64) = (0.7, 0.95);
const POLAR_CHILL: f64 = 0.9;

// Continentalness to height above sea level: abyssal plain, continental slope, the
// shelf around each coast, then the coast and inland.
const CONTINENT_SPLINE: [(f64, f64); 14] = [
    (-1.0, -390.0),
    (-0.6, -375.0),
    (-0.4, -340.0),
    (-0.3, -220.0),
    (-0.22, -75.0),
    (-0.18, -55.0),
    (-0.1, -35.0),
    (-0.04, -14.0),
    (-0.01, -3.0),
    (0.0, 0.5),
    (0.015, 3.0),
    (0.05, 8.0),
    (0.2, 18.0),
    (0.5, 36.0),
];

pub struct NoiseGenerator {
    seed: u32,
    warp_x: Fbm<Perlin>,
    warp_z: Fbm<Perlin>,
    continents: Fractal,
    ruggedness: Fractal,
    ridges: Vec<Perlin>,
    hills: Fractal,
    hilliness: Fractal,
    detail: Fractal,
    texture: Fractal,
    temperature: Fbm<Perlin>,
    humidity: Fbm<Perlin>,
    biomes: Biomes,
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
        let fractal = |i: u32, wavelength: f64, octaves: usize| {
            Fractal::new(field_seed(i), wavelength, octaves, 2.0, 0.5)
        };
        Self {
            seed,
            warp_x: fbm(0, WARP_WAVELENGTH, 3),
            warp_z: fbm(1, WARP_WAVELENGTH, 3),
            continents: fractal(2, CONTINENT_WAVELENGTH, 6),
            ruggedness: fractal(3, RUGGEDNESS_WAVELENGTH, 3),
            ridges: (0..RIDGE_OCTAVES as u32)
                .map(|i| Perlin::new(field_seed(4).wrapping_add(i)))
                .collect(),
            detail: fractal(5, DETAIL_WAVELENGTH, 5),
            texture: fractal(8, TEXTURE_WAVELENGTH, 3),
            hills: fractal(6, HILL_WAVELENGTH, 4),
            hilliness: fractal(7, HILLINESS_WAVELENGTH, 2),
            temperature: fbm(9, CLIMATE_WAVELENGTH, 4),
            humidity: fbm(10, CLIMATE_WAVELENGTH, 4),
            biomes: Biomes::new(seed),
        }
    }

    // Column height in blocks.
    pub fn get(&self, pos: I64Vec2) -> f64 {
        self.sample(pos.x as f64, pos.y as f64).height
    }

    pub fn seed(&self) -> u32 {
        self.seed
    }

    pub fn biomes(&self) -> &Biomes {
        &self.biomes
    }

    // The base terrain, then whichever biomes the climate here calls for.
    pub fn sample(&self, x: f64, z: f64) -> TerrainSample {
        self.sample_for_cell(x, z, 1.0)
    }

    /// The terrain as a cell `cell` blocks wide, sampled at its centre, should see it: features
    /// too fine for cells that wide fade to their average, instead of aliasing into bumps and
    /// gaps that change from one LOD level to the next. The same as `sample` for single blocks.
    pub fn sample_for_cell(&self, x: f64, z: f64, cell: f64) -> TerrainSample {
        let (continentalness, ruggedness, height) = self.base_terrain(x, z, cell);
        let (height, biome, trees) = self.biomes.shape(&BaseColumn {
            x,
            z,
            cell,
            height,
            continentalness,
            ruggedness,
            climate: self.climate(x, z),
        });
        TerrainSample {
            continentalness,
            height: height.max(0.0),
            biome,
            trees,
        }
    }

    // Continentalness, ruggedness and height of the terrain every biome starts from.
    fn base_terrain(&self, x: f64, z: f64, cell: f64) -> (f64, f64, f64) {
        let warp_x = self.warp_x.get([x, z]) * WARP_STRENGTH;
        let warp_z = self.warp_z.get([x, z]) * WARP_STRENGTH;
        let (wx, wz) = (x + warp_x, z + warp_z);

        let edge = edge_falloff(wx, z + warp_z * POLE_COAST_WARP);
        let continentalness =
            (self.continents.get(wx, wz, cell) * CONTINENT_CONTRAST).clamp(-1.0, 1.0);
        let continentalness = (continentalness
            + POLAR_UPLIFT
                * smoothstep(
                    POLAR_UPLIFT_LATITUDE.0,
                    POLAR_UPLIFT_LATITUDE.1,
                    wz.abs() / WORLD_HALF_SIZE as f64,
                ))
        .lerp(-1.0, edge);
        // Ruggedness and hilliness only shape land and the shallows, so open ocean skips them.
        let ruggedness = if continentalness > OPEN_OCEAN {
            self.ruggedness.get(wx, wz, cell)
        } else {
            0.0
        };

        let base = spline(&CONTINENT_SPLINE, continentalness);

        let inland = smoothstep(0.02, 0.25, continentalness);
        let mountain_mask = inland * smoothstep(0.0, 0.4, ruggedness);
        let mountains = if mountain_mask > 0.0 {
            // A full strength warp shears ridges into long streaks, so they get only a little.
            let ridge_x = x + warp_x * RIDGE_WARP;
            let ridge_z = z + warp_z * RIDGE_WARP;
            let uplift = smoothstep(0.0, 1.0, mountain_mask) * UPLIFT_HEIGHT;
            // Peaks grow towards the heart of a range, leaving foothills at its edges.
            let ridges = self.ridged(ridge_x, ridge_z, cell).powf(1.3)
                * mountain_mask.powf(1.5)
                * RIDGE_HEIGHT;
            uplift + ridges
        } else {
            0.0
        };

        // Kept above zero, so hills rise out of flat valley floors instead of dipping into lakes.
        let hills = if continentalness > 0.0 {
            let hilliness = smoothstep(-0.25, 0.3, self.hilliness.get(wx, wz, cell));
            let hill_amplitude = 0.15_f64.lerp(1.0, hilliness)
                * HILL_HEIGHT
                * smoothstep(0.0, 0.04, continentalness);
            // Ranges carry their own relief, so hills fade out into them.
            (self.hills.get(x, z, cell) * 0.8 + 0.3).max(0.0)
                * hill_amplitude
                * (1.0 - mountain_mask)
        } else {
            0.0
        };

        // Calm the detail near the shoreline, or it breaks the coast up into specks of land.
        let shore_calm = 0.2_f64.lerp(1.0, smoothstep(0.0, 0.1, continentalness.abs()));
        let land_detail = PLAINS_DETAIL.lerp(RUGGED_DETAIL, smoothstep(-0.3, 0.4, ruggedness))
            + mountain_mask * MOUNTAIN_DETAIL;
        let sea_floor_blend = smoothstep(OPEN_OCEAN, OPEN_OCEAN + 0.1, continentalness);
        let detail_amplitude = SEA_FLOOR_DETAIL.lerp(land_detail, sea_floor_blend) * shore_calm;
        let detail = self.detail.get(x, z, cell) * detail_amplitude;

        let land_texture = mountain_mask * MOUNTAIN_TEXTURE;
        // none right at the shore, or it raises specks of land out of the shallows
        let shore_fade = smoothstep(0.0, 0.06, continentalness.abs());
        let texture = self.texture.get(x, z, cell) * land_texture * sea_floor_blend * shore_fade;

        let height =
            ease_peaks(WATER_LEVEL as f64 + base + hills + mountains + detail + texture).max(0.0);
        (continentalness, ruggedness, height)
    }

    // How hot and wet the column at (x, z) is.
    pub fn climate(&self, x: f64, z: f64) -> Climate {
        let spread = |n: f64| (n * CLIMATE_CONTRAST * 0.5 + 0.5).clamp(0.0, 1.0) as f32;
        Climate {
            temperature: self.temperature(x, z),
            humidity: spread(self.humidity.get([x, z])),
        }
    }

    // How hot the column at (x, z) is: noise, pulled cold towards the poles at the north and south edges of the land.
    pub fn temperature(&self, x: f64, z: f64) -> f32 {
        let noise = (self.temperature.get([x, z]) * CLIMATE_CONTRAST * 0.5 + 0.5).clamp(0.0, 1.0);
        let latitude = (z.abs() / WORLD_HALF_SIZE as f64).min(1.0);
        let warmth = (latitude * std::f64::consts::FRAC_PI_2).cos();
        let chill =
            POLAR_CHILL * smoothstep(POLAR_CHILL_LATITUDE.0, POLAR_CHILL_LATITUDE.1, latitude);
        ((noise * (1.0 - LATITUDE_WEIGHT) + warmth * LATITUDE_WEIGHT) * (1.0 - chill)) as f32
    }

    // Ridged multifractal in 0..1: sharp crests where the noise crosses zero, with each
    // octave weighted by the last so fine detail gathers on the ridges, not the valleys.
    // Octaves too fine for `cell` give way to their mean signal.
    fn ridged(&self, x: f64, z: f64, cell: f64) -> f64 {
        let mut frequency = 1.0 / RIDGE_WAVELENGTH;
        let mut wavelength = RIDGE_WAVELENGTH;
        let mut amplitude = 1.0;
        let mut weight = 1.0;
        let mut total = 0.0;
        let mut norm = 0.0;
        for octave in &self.ridges {
            let resolved = resolved(wavelength, cell);
            let signal = if resolved > 0.0 {
                let signal = 1.0 - octave.get([x * frequency, z * frequency]).abs();
                signal * signal
            } else {
                0.0
            };
            let signal = (signal * resolved + RIDGE_SIGNAL_MEAN * (1.0 - resolved)) * weight;
            weight = (signal * 1.5).clamp(0.0, 1.0);
            total += signal * amplitude;
            norm += amplitude;
            amplitude *= 0.5;
            frequency *= 2.0;
            wavelength /= 2.0;
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

// Leaves heights below PEAK_EASE_START alone and approaches MAX_HEIGHT without reaching it.
fn ease_peaks(height: f64) -> f64 {
    if height <= PEAK_EASE_START {
        return height;
    }
    let room = MAX_HEIGHT - PEAK_EASE_START;
    MAX_HEIGHT - room * (-(height - PEAK_EASE_START) / room).exp()
}

// 0 inside the world, rising to 1 in the ocean ring at its edge.
fn edge_falloff(x: f64, z: f64) -> f64 {
    let half = WORLD_HALF_SIZE as f64;
    let sides = smoothstep(EDGE_FALLOFF_START, EDGE_FALLOFF_END, x.abs() / half);
    sides.max(smoothstep(
        POLE_FALLOFF_START,
        POLE_FALLOFF_END,
        z.abs() / half,
    ))
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
    y0.lerp(y1, (x - x0) / (x1 - x0))
}

impl TerrainSource for NoiseGenerator {
    fn seed(&self) -> u32 {
        NoiseGenerator::seed(self)
    }

    fn biomes(&self) -> &Biomes {
        NoiseGenerator::biomes(self)
    }

    fn sample_for_cell(&self, x: f64, z: f64, cell: f64) -> TerrainSample {
        NoiseGenerator::sample_for_cell(self, x, z, cell)
    }

    fn climate(&self, x: f64, z: f64) -> Climate {
        NoiseGenerator::climate(self, x, z)
    }

    fn temperature(&self, x: f64, z: f64) -> f32 {
        NoiseGenerator::temperature(self, x, z)
    }

    fn find_spawn(&self) -> (i64, i64, f64) {
        NoiseGenerator::find_spawn(self)
    }

    fn tree_bound(&self, x: f64, z: f64) -> f64 {
        self.biomes.tree_bound(self.climate(x, z))
    }
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
    fn test_peaks_ease_under_max_height() {
        assert_eq!(300.0, ease_peaks(300.0));
        assert!(ease_peaks(450.0) > ease_peaks(420.0));
        assert!(ease_peaks(1000.0) < MAX_HEIGHT);
    }

    #[test]
    fn test_world_edge_is_deep_ocean() {
        let noise = NoiseGenerator::new(9);
        for (x, z) in [
            (WORLD_HALF_SIZE, 0),
            (0, -WORLD_HALF_SIZE - 2000),
            (40_000, 40_000),
        ] {
            let height = noise.get(I64Vec2::new(x, z));
            assert!(
                height < WATER_LEVEL as f64 - 30.0,
                "({x}, {z}) should be deep ocean, was {height}"
            );
        }
    }

    #[test]
    fn test_both_poles_have_land_and_the_edge_is_ocean() {
        let half = WORLD_HALF_SIZE as f64;
        for seed in [1, 7, 9, 42, 123] {
            let noise = NoiseGenerator::new(seed);
            for sign in [-1.0, 1.0] {
                let height = |z: f64| {
                    (-60..60)
                        .map(|i| noise.get(I64Vec2::new(i * 250, (sign * z) as i64)))
                        .collect::<Vec<_>>()
                };
                let cap = height(half * 0.8);
                let land = cap
                    .iter()
                    .filter(|&&h| h > WATER_LEVEL as f64 + 2.0)
                    .count();
                assert!(
                    land > cap.len() / 2,
                    "seed {seed}: only {land} of {} columns at the pole are land",
                    cap.len()
                );
                let edge = height(half);
                assert!(
                    edge.iter().all(|&h| h < WATER_LEVEL as f64 - 30.0),
                    "seed {seed}: land at the world's edge"
                );
            }
        }
    }

    #[test]
    fn test_climate_varies_across_the_whole_range() {
        let noise = NoiseGenerator::new(9);
        let samples: Vec<Climate> = (-40..40)
            .flat_map(|z| (-40..40).map(move |x| (x as f64 * 400.0, z as f64 * 400.0)))
            .map(|(x, z)| noise.climate(x, z))
            .collect();
        for value in [|c: &Climate| c.temperature, |c: &Climate| c.humidity] {
            let (low, high) = samples
                .iter()
                .map(value)
                .fold((1.0f32, 0.0f32), |(l, h), v| (l.min(v), h.max(v)));
            assert!(
                low < 0.15 && high > 0.85,
                "climate only spans {low}..{high}"
            );
        }
    }

    #[test]
    fn test_poles_are_colder_than_the_equator() {
        let noise = NoiseGenerator::new(9);
        let mean = |z: f64| {
            (-40..40)
                .map(|i| noise.temperature(i as f64 * 400.0, z))
                .sum::<f32>()
                / 80.0
        };
        let pole = WORLD_HALF_SIZE as f64 * 0.95;
        assert!(mean(0.0) > 0.7, "equator was {}", mean(0.0));
        assert!(
            mean(pole) < 0.1 && mean(-pole) < 0.1,
            "poles were {} and {}",
            mean(pole),
            mean(-pole)
        );
    }

    #[test]
    fn test_climate_changes_gradually() {
        let noise = NoiseGenerator::new(9);
        let (a, b) = (noise.climate(1000.0, 1000.0), noise.climate(1016.0, 1000.0));
        assert!((a.temperature - b.temperature).abs() < 0.05);
        assert!((a.humidity - b.humidity).abs() < 0.05);
    }

    #[test]
    fn test_ridge_signal_mean_matches_perlin() {
        let noise = NoiseGenerator::new(9);
        let samples =
            (0..300).flat_map(|i| (0..300).map(move |j| (i as f64 * 0.0731, j as f64 * 0.0689)));
        let mean = samples
            .map(|(x, z)| (1.0 - noise.ridges[0].get([x, z]).abs()).powi(2))
            .sum::<f64>()
            / 90_000.0;
        assert!((mean - RIDGE_SIGNAL_MEAN).abs() < 0.02, "measured {mean}");
    }

    #[test]
    fn test_full_detail_is_unfiltered() {
        let noise = NoiseGenerator::new(9);
        for i in 0..200 {
            let (x, z) = (i as f64 * 97.3 - 9000.0, i as f64 * -61.7 + 4000.0);
            assert_eq!(
                noise.sample(x, z).height.to_bits(),
                noise.sample_for_cell(x, z, 1.0).height.to_bits()
            );
        }
    }

    #[test]
    fn test_wide_cells_keep_the_lie_of_the_land() {
        // a wide cell's height is closer to the mean of the full-detail ground it covers than the
        // height at its centre is
        let noise = NoiseGenerator::new(9);
        for cell in [64.0, 256.0] {
            let (mut filtered_error, mut point_error) = (0.0, 0.0);
            for i in 0..60 {
                let (x, z) = (i as f64 * 517.0 - 15000.0, i as f64 * 311.0 - 9000.0);
                let mean = (0..16)
                    .flat_map(|a| (0..16).map(move |b| (a, b)))
                    .map(|(a, b)| {
                        noise
                            .sample(
                                x + (a as f64 - 7.5) * cell / 16.0,
                                z + (b as f64 - 7.5) * cell / 16.0,
                            )
                            .height
                    })
                    .sum::<f64>()
                    / 256.0;
                filtered_error += (noise.sample_for_cell(x, z, cell).height - mean).abs();
                point_error += (noise.sample(x, z).height - mean).abs();
            }
            assert!(
                filtered_error < point_error,
                "cells {cell} wide: filtered {filtered_error} vs point sampled {point_error}"
            );
        }
    }

    #[test]
    fn test_spawn_is_on_land() {
        let noise = NoiseGenerator::new(9);
        let (_, _, height) = noise.find_spawn();
        assert!(height > WATER_LEVEL as f64);
    }
}
