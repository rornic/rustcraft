use noise::{NoiseFn, Perlin};

/// How much of a feature `wavelength` blocks across survives in cells `cell` blocks wide, 0..1:
/// what averaging a wave over a cell leaves of it, sin(πr)/(πr) for r = cell / wavelength, and
/// nothing once the cell is as wide as the wave. A cell standing for the mean of the ground it
/// covers agrees with the cells of the levels above and below it, where taking the noise at
/// its centre would alias fine features into bumps and gaps that differ from level to level.
/// Single blocks are full detail, and see everything.
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

/// Fractal Perlin noise, summed exactly as `noise::Fbm` sums it, that can also average each
/// octave over the cells it's sampled for, see `resolved`. Perlin noise averages to zero, so an
/// octave fades to nothing.
pub struct Fractal {
    sources: Vec<Perlin>,
    frequency: f64,
    lacunarity: f64,
    persistence: f64,
    scale: f64,
}

impl Fractal {
    pub fn new(
        seed: u32,
        wavelength: f64,
        octaves: usize,
        lacunarity: f64,
        persistence: f64,
    ) -> Self {
        // `Fbm` seeds its octaves seed, seed + 1, ... and scales by the sum of their amplitudes
        let denom = (1..=octaves).fold(0.0, |acc, octave| acc + persistence.powi(octave as i32));
        Self {
            sources: (0..octaves as u32)
                .map(|i| Perlin::new(seed.wrapping_add(i)))
                .collect(),
            frequency: 1.0 / wavelength,
            lacunarity,
            persistence,
            scale: 1.0 / denom,
        }
    }

    /// The noise at (`x`, `z`) as cells `cell` blocks wide see it; 1 for single blocks.
    pub fn get(&self, x: f64, z: f64, cell: f64) -> f64 {
        let mut point = [x * self.frequency, z * self.frequency];
        let mut wavelength = 1.0 / self.frequency;
        let mut attenuation = self.persistence;
        let mut result = 0.0;
        for source in &self.sources {
            let resolved = resolved(wavelength, cell);
            if resolved == 0.0 {
                // every octave after is finer still
                break;
            }
            let mut signal = source.get(point);
            signal *= attenuation;
            signal *= resolved;
            attenuation *= self.persistence;
            result += signal;
            point = [point[0] * self.lacunarity, point[1] * self.lacunarity];
            wavelength /= self.lacunarity;
        }
        result * self.scale
    }
}

#[cfg(test)]
mod tests {
    use noise::{Fbm, MultiFractal};

    use super::*;

    #[test]
    fn test_matches_fbm_exactly_for_single_blocks() {
        for (seed, wavelength, octaves, lacunarity) in [
            (7, 350.0, 5, 2.0),
            (3, 16000.0, 6, 2.0),
            (11, 32.0, 3, std::f64::consts::PI * 2.0 / 3.0),
        ] {
            let fractal = Fractal::new(seed, wavelength, octaves, lacunarity, 0.5);
            let fbm = Fbm::<Perlin>::new(seed)
                .set_frequency(1.0 / wavelength)
                .set_octaves(octaves)
                .set_persistence(0.5)
                .set_lacunarity(lacunarity);
            for i in 0..2000 {
                let (x, z) = (
                    (i * 37 % 1999) as f64 * 13.7 - 9000.0,
                    (i * 61 % 1993) as f64 * 11.3 - 7000.0,
                );
                assert_eq!(
                    fbm.get([x, z]).to_bits(),
                    fractal.get(x, z, 1.0).to_bits(),
                    "seed {seed} at ({x}, {z})"
                );
            }
        }
    }

    #[test]
    fn test_resolved_is_a_box_average() {
        assert_eq!(1.0, resolved(5.0, 1.0));
        assert_eq!(0.0, resolved(16.0, 16.0));
        assert_eq!(0.0, resolved(5.0, 16.0));
        assert!((resolved(32.0, 16.0) - 2.0 / std::f64::consts::PI).abs() < 1e-12);
        assert!(resolved(1000.0, 16.0) > 0.99);
    }

    #[test]
    fn test_octaves_no_wider_than_a_cell_are_left_out() {
        let fractal = Fractal::new(5, 1000.0, 6, 2.0, 0.5);
        let coarse = Fractal::new(5, 1000.0, 2, 2.0, 0.5);
        // cells 250 wide see the 1000 and 500 octaves, and none of 250 and finer
        let (x, z) = (1234.5, -987.25);
        let expected = coarse.get(x, z, 250.0) / coarse.scale * fractal.scale;
        assert!((fractal.get(x, z, 250.0) - expected).abs() < 1e-12);
    }

    #[test]
    fn test_wide_cells_vary_less_between_neighbours() {
        // the same terrain sampled once every 64 blocks: smoother where fine octaves fade out
        let fractal = Fractal::new(9, 350.0, 5, 2.0, 0.5);
        let roughness = |cell: f64| {
            (0..400)
                .map(|i| {
                    (fractal.get((i + 1) as f64 * 64.0, 0.0, cell)
                        - fractal.get(i as f64 * 64.0, 0.0, cell))
                    .abs()
                })
                .sum::<f64>()
        };
        assert!(roughness(64.0) < roughness(1.0) * 0.9);
    }
}
