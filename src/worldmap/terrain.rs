//! Continents and base height: low-frequency noise picks land from sea, ridged noise raises
//! mountain ranges where the land is rugged.

use super::noise::{fbm, ridged};
use super::{par_rows, WorldConfig, WorldMap};
use crate::chunks::generate::{WATER_LEVEL, WORLD_HEIGHT};

// Each field gets its own block of seeds, as noise octaves seed seed, seed + 1, ...
const SEED_STRIDE: u32 = 101;

// Domain warp bends continents into peninsulas and bays instead of round blobs.
const WARP_WAVELENGTH: f64 = 9000.0;
const WARP_STRENGTH: f64 = 3000.0;
const CONTINENT_WAVELENGTH: f64 = 16000.0;
// Steepens continentalness, which a long wavelength leaves shallow, so coasts stay sharp.
const CONTINENT_CONTRAST: f64 = 1.7;
// Ranges follow the zero crossings of noise this many blocks across, as belts this wide.
const RANGE_WAVELENGTH: f64 = 11000.0;
const RANGE_BELT: (f64, f64) = (0.9, 0.985);
const RIDGE_WAVELENGTH: f64 = 1500.0;
// Finer octaves than a 32 m cell can hold are left to the chunks' detail noise.
const RIDGE_OCTAVES: u32 = 5;
const RIDGE_WARP: f64 = 0.18;
const UPLIFT_HEIGHT: f64 = 190.0;
const RIDGE_HEIGHT: f64 = 230.0;
const HILL_WAVELENGTH: f64 = 900.0;
const HILLINESS_WAVELENGTH: f64 = 2500.0;
const HILL_HEIGHT: f64 = 45.0;
// The ocean ring around the world starts and ends at this fraction of the half size.
const EDGE_FALLOFF: (f64, f64) = (0.8, 0.97);

const MAX_HEIGHT: f64 = WORLD_HEIGHT as f64 - 16.0;
const PEAK_EASE_START: f64 = MAX_HEIGHT - 96.0;

// Continentalness to height above sea level: abyssal plain, continental slope, the shelf
// around each coast, then the coast and inland.
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

pub(super) fn smoothstep(edge0: f64, edge1: f64, x: f64) -> f64 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

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

// Leaves heights below PEAK_EASE_START alone and approaches MAX_HEIGHT without reaching it.
fn ease_peaks(height: f64) -> f64 {
    if height <= PEAK_EASE_START {
        return height;
    }
    let room = MAX_HEIGHT - PEAK_EASE_START;
    MAX_HEIGHT - room * (-(height - PEAK_EASE_START) / room).exp()
}

// 0 inside the world, rising to 1 in the ocean ring at its edge.
fn edge_falloff(x: f64, z: f64, half: f64) -> f64 {
    smoothstep(EDGE_FALLOFF.0, EDGE_FALLOFF.1, x.abs() / half).max(smoothstep(
        EDGE_FALLOFF.0,
        EDGE_FALLOFF.1,
        z.abs() / half,
    ))
}

/// Ground height in blocks and continentalness at a point.
pub(super) fn column(seed: u32, x: f64, z: f64, half: f64) -> (f64, f64) {
    let field = |i: u32| seed.wrapping_add(i * SEED_STRIDE);
    let warp_x = fbm(field(0), x, z, WARP_WAVELENGTH, 3) * WARP_STRENGTH;
    let warp_z = fbm(field(1), x, z, WARP_WAVELENGTH, 3) * WARP_STRENGTH;
    let (wx, wz) = (x + warp_x, z + warp_z);

    let edge = edge_falloff(wx, wz, half);
    let continentalness =
        (fbm(field(2), wx, wz, CONTINENT_WAVELENGTH, 6) * CONTINENT_CONTRAST).clamp(-1.0, 1.0);
    let continentalness = lerp(continentalness, -1.0, edge);

    // ranges run along the curves where a slow noise crosses zero, so they are long and narrow
    let range = if continentalness > -0.25 {
        let belt = 1.0 - fbm(field(3), wx, wz, RANGE_WAVELENGTH, 3).abs();
        smoothstep(RANGE_BELT.0, RANGE_BELT.1, belt)
    } else {
        0.0
    };
    let base = spline(&CONTINENT_SPLINE, continentalness);
    let inland = smoothstep(0.02, 0.25, continentalness);
    let mountain_mask = inland * range;
    let mountains = if mountain_mask > 0.0 {
        let (rx, rz) = (x + warp_x * RIDGE_WARP, z + warp_z * RIDGE_WARP);
        let uplift = smoothstep(0.0, 1.0, mountain_mask) * UPLIFT_HEIGHT;
        let ridges = ridged(field(4), rx, rz, RIDGE_WAVELENGTH, RIDGE_OCTAVES).powf(1.3)
            * mountain_mask.powf(1.5)
            * RIDGE_HEIGHT;
        uplift + ridges
    } else {
        0.0
    };
    // kept above zero, so hills rise out of flat valley floors instead of dipping into lakes
    let hills = if continentalness > 0.0 {
        let hilliness = smoothstep(-0.25, 0.3, fbm(field(7), wx, wz, HILLINESS_WAVELENGTH, 2));
        let amplitude =
            lerp(0.15, 1.0, hilliness) * HILL_HEIGHT * smoothstep(0.0, 0.04, continentalness);
        (fbm(field(6), x, z, HILL_WAVELENGTH, 4) * 0.8 + 0.3).max(0.0)
            * amplitude
            * (1.0 - mountain_mask)
    } else {
        0.0
    };
    let height = ease_peaks(WATER_LEVEL as f64 + base + hills + mountains).max(0.0);
    (height, continentalness)
}

pub(super) fn base_terrain(seed: u32, config: WorldConfig) -> WorldMap {
    let dim = config.dim();
    let half = config.half_size() as f64;
    let cell = config.map_cell as f64;
    let mut cells = vec![(0.0f32, 0.0f32); dim * dim];
    par_rows(dim, &mut cells, |row, out| {
        let z = (row as f64 + 0.5) * cell - half;
        for (col, out) in out.iter_mut().enumerate() {
            let x = (col as f64 + 0.5) * cell - half;
            let (height, continentalness) = column(seed, x, z, half);
            *out = (height as f32, continentalness as f32);
        }
    });
    WorldMap {
        height: cells.iter().map(|c| c.0).collect(),
        continentalness: cells.iter().map(|c| c.1).collect(),
        ..WorldMap::empty(seed, config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_peaks_ease_under_max_height() {
        assert_eq!(300.0, ease_peaks(300.0));
        assert!(ease_peaks(450.0) > ease_peaks(420.0));
        assert!(ease_peaks(2000.0) < MAX_HEIGHT);
    }

    #[test]
    fn test_world_edge_is_deep_ocean() {
        let half = 32_768.0;
        for seed in [1, 7, 42] {
            for (x, z) in [(half, 0.0), (0.0, -half), (half, half)] {
                let (h, _) = column(seed, x, z, half);
                assert!(h < WATER_LEVEL as f64 - 30.0, "seed {seed} ({x}, {z}): {h}");
            }
        }
    }

    #[test]
    fn test_map_has_land_sea_and_ranges() {
        let map = super::super::generate(
            42,
            WorldConfig {
                size: 32_768,
                map_cell: 64,
            },
        );
        let sea = WATER_LEVEL as f32;
        let land = map.height.iter().filter(|&&h| h > sea).count() as f64 / map.height.len() as f64;
        assert!((0.15..0.7).contains(&land), "land share {land}");
        assert!(map.height.iter().any(|&h| h > sea + 250.0), "no mountains");
    }
}
