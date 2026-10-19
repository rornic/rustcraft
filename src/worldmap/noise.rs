//! Gradient noise from an integer hash and nothing but `+ - * /` and `floor`, so every machine
//! computes the same value: no libm call whose last bit differs between x86 and ARM.

/// A well-mixed 32-bit hash of a seed and a lattice point.
pub fn hash(seed: u32, x: i32, z: i32) -> u32 {
    let mut h = seed ^ (x as u32).wrapping_mul(0x85EB_CA6B) ^ (z as u32).wrapping_mul(0xC2B2_AE35);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7FEB_352D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846C_A68B);
    h ^ (h >> 16)
}

/// `hash` as a number in 0..1.
pub fn unit(seed: u32, x: i32, z: i32) -> f64 {
    hash(seed, x, z) as f64 / 4_294_967_296.0
}

const R: f64 = std::f64::consts::FRAC_1_SQRT_2;
const GRADIENTS: [(f64, f64); 8] = [
    (1.0, 0.0),
    (-1.0, 0.0),
    (0.0, 1.0),
    (0.0, -1.0),
    (R, R),
    (-R, R),
    (R, -R),
    (-R, -R),
];

fn fade(t: f64) -> f64 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

/// Perlin noise, about -1..1.
pub fn perlin(seed: u32, x: f64, z: f64) -> f64 {
    let (fx, fz) = (x.floor(), z.floor());
    let (ix, iz) = (fx as i32, fz as i32);
    let (dx, dz) = (x - fx, z - fz);
    let corner = |cx: i32, cz: i32| {
        let (gx, gz) = GRADIENTS[(hash(seed, ix + cx, iz + cz) & 7) as usize];
        gx * (dx - cx as f64) + gz * (dz - cz as f64)
    };
    let (u, v) = (fade(dx), fade(dz));
    let top = corner(0, 0) + u * (corner(1, 0) - corner(0, 0));
    let bottom = corner(0, 1) + u * (corner(1, 1) - corner(0, 1));
    // unit gradients top out at sqrt(2) / 2
    (top + v * (bottom - top)) * std::f64::consts::SQRT_2
}

/// Fractal Perlin noise of `octaves`, about -1..1, with features `wavelength` blocks across.
pub fn fbm(seed: u32, x: f64, z: f64, wavelength: f64, octaves: u32) -> f64 {
    let (mut frequency, mut amplitude, mut total, mut norm) = (1.0 / wavelength, 1.0, 0.0, 0.0);
    for octave in 0..octaves {
        // a lattice offset per octave keeps their origins from lining up
        let offset = octave as f64 * 17.31;
        total += perlin(
            seed.wrapping_add(octave.wrapping_mul(0x9E37_79B9)),
            x * frequency + offset,
            z * frequency - offset,
        ) * amplitude;
        norm += amplitude;
        amplitude *= 0.5;
        frequency *= 2.0;
    }
    total / norm
}

/// How much of a feature `wavelength` blocks across survives in cells `cell` blocks wide, 0..1:
/// what averaging a wave over a cell leaves of it, and nothing once the cell is as wide as the
/// wave. Single blocks see everything, with no `sin` call, so full detail stays portable.
pub fn resolved(wavelength: f64, cell: f64) -> f64 {
    if cell <= 1.0 {
        return 1.0;
    }
    let ratio = cell / wavelength;
    if ratio >= 1.0 {
        return 0.0;
    }
    let angle = std::f64::consts::PI * ratio;
    angle.sin() / angle
}

/// `fbm`, with each octave faded by `resolved` for cells `cell` blocks wide, so wide cells see
/// the mean of what they cover rather than aliasing fine features into bumps that differ from
/// one LOD level to the next. The same as `fbm` for single blocks.
pub fn fbm_filtered(seed: u32, x: f64, z: f64, wavelength: f64, octaves: u32, cell: f64) -> f64 {
    let (mut frequency, mut amplitude, mut total, mut norm) = (1.0 / wavelength, 1.0, 0.0, 0.0);
    let mut octave_wavelength = wavelength;
    for octave in 0..octaves {
        let offset = octave as f64 * 17.31;
        let seen = resolved(octave_wavelength, cell);
        if seen > 0.0 {
            total += perlin(
                seed.wrapping_add(octave.wrapping_mul(0x9E37_79B9)),
                x * frequency + offset,
                z * frequency - offset,
            ) * amplitude
                * seen;
        }
        norm += amplitude;
        amplitude *= 0.5;
        frequency *= 2.0;
        octave_wavelength *= 0.5;
    }
    total / norm
}

/// Ridged multifractal, 0..1: sharp crests where the noise crosses zero, each octave weighted by
/// the last so fine detail gathers on the ridges.
pub fn ridged(seed: u32, x: f64, z: f64, wavelength: f64, octaves: u32) -> f64 {
    let (mut frequency, mut amplitude, mut weight) = (1.0 / wavelength, 1.0, 1.0);
    let (mut total, mut norm) = (0.0, 0.0);
    for octave in 0..octaves {
        let offset = octave as f64 * 31.7;
        let n = perlin(
            seed.wrapping_add(octave.wrapping_mul(0x9E37_79B9)),
            x * frequency + offset,
            z * frequency - offset,
        );
        let signal = 1.0 - n.abs().min(1.0);
        let signal = signal * signal * weight;
        weight = (signal * 1.5).clamp(0.0, 1.0);
        total += signal * amplitude;
        norm += amplitude;
        amplitude *= 0.5;
        frequency *= 2.0;
    }
    total / norm
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_perlin_is_continuous_and_bounded() {
        let mut last = perlin(1, 0.0, 3.3);
        for i in 1..2000 {
            let n = perlin(1, i as f64 * 0.01, 3.3);
            assert!(n.abs() <= 1.0001);
            assert!((n - last).abs() < 0.1, "jumped at {i}");
            last = n;
        }
    }

    #[test]
    fn test_noise_is_fixed_for_a_seed() {
        // pinned, as a change to the hash or the maths would change every world
        assert_eq!(1_640_221_175, hash(1, 2, 3));
        assert_eq!(-0.2166502339539473, perlin(7, 1.3, 2.7));
    }

    #[test]
    fn test_filtered_fbm_matches_fbm_at_full_detail_and_calms_in_wide_cells() {
        for i in 0..50 {
            let (x, z) = (i as f64 * 91.7, i as f64 * -33.1);
            assert_eq!(
                fbm(5, x, z, 120.0, 6).to_bits(),
                fbm_filtered(5, x, z, 120.0, 6, 1.0).to_bits()
            );
        }
        let spread = |cell: f64| {
            (0..400)
                .map(|i| fbm_filtered(5, i as f64 * 13.0, i as f64 * 7.0, 120.0, 6, cell).abs())
                .sum::<f64>()
        };
        assert!(spread(64.0) < spread(1.0) * 0.6);
    }

    #[test]
    fn test_fbm_spans_the_range() {
        let values: Vec<f64> = (0..5000)
            .map(|i| fbm(3, i as f64 * 37.0, i as f64 * 11.0, 1000.0, 5))
            .collect();
        let (lo, hi) = values
            .iter()
            .fold((f64::MAX, f64::MIN), |(l, h), &v| (l.min(v), h.max(v)));
        assert!(lo < -0.3 && hi > 0.3, "{lo}..{hi}");
        assert!(lo >= -1.0 && hi <= 1.0);
    }
}
