use noise::{Fbm, MultiFractal, NoiseFn, Perlin};

use super::{smoothstep, BaseColumn, Biome};

// Foothills ring each mountain range: they start this far down the ruggedness scale, are at
// full strength where the range begins to rise (at 0, see `NoiseGenerator`), and give way to
// the range's own crags further in.
const FOOTHILLS: (f64, f64) = (-0.25, 0.0);
const RANGE: (f64, f64) = (0.15, 0.35);
// none right at the shore, or they raise specks of land out of the shallows
const SHORE: (f64, f64) = (0.0, 0.06);

// Bumps a few blocks across, so the ground breaks up as it climbs towards the mountains.
const BUMP_WAVELENGTH: f64 = 32.0;
const BUMP_HEIGHT: f64 = 9.0;

// Seeds after the base terrain's and the desert's, see `NoiseGenerator::new`.
const SEED_OFFSET: u32 = 2100;

// Broken, bumpy ground around the feet of the mountains.
pub struct Foothills {
    bumps: Fbm<Perlin>,
}

impl Foothills {
    pub fn new(seed: u32) -> Self {
        Self {
            bumps: Fbm::<Perlin>::new(seed.wrapping_add(SEED_OFFSET))
                .set_frequency(1.0 / BUMP_WAVELENGTH)
                .set_octaves(3),
        }
    }
}

impl Biome for Foothills {
    fn name(&self) -> &'static str {
        "foothills"
    }

    fn strength(&self, column: &BaseColumn) -> f64 {
        smoothstep(FOOTHILLS.0, FOOTHILLS.1, column.ruggedness)
            * smoothstep(RANGE.1, RANGE.0, column.ruggedness)
            * smoothstep(SHORE.0, SHORE.1, column.continentalness)
    }

    fn height(&self, column: &BaseColumn) -> f64 {
        column.height + self.bumps.get([column.x, column.z]) * BUMP_HEIGHT
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunks::generate::biome::Climate;

    fn column(continentalness: f64, ruggedness: f64) -> BaseColumn {
        BaseColumn {
            x: 0.0,
            z: 0.0,
            height: 500.0,
            continentalness,
            ruggedness,
            climate: Climate::default(),
        }
    }

    #[test]
    fn test_foothills_ring_mountains_but_not_plains_or_coast() {
        let foothills = Foothills::new(1);
        assert_eq!(0.0, foothills.strength(&column(0.3, -0.6)));
        assert!(foothills.strength(&column(0.3, -0.15)) > 0.0);
        assert_eq!(1.0, foothills.strength(&column(0.3, 0.0)));
        assert_eq!(0.0, foothills.strength(&column(0.3, 0.5)));
        assert_eq!(0.0, foothills.strength(&column(-0.1, 0.0)));
    }

    #[test]
    fn test_foothills_are_bumpy() {
        let foothills = Foothills::new(1);
        let heights: Vec<f64> = (0..100)
            .map(|x| foothills.height(&BaseColumn { x: x as f64 * 3.0, ..column(0.3, 0.0) }))
            .collect();
        let (lo, hi) = heights.iter().fold((f64::MAX, f64::MIN), |(l, h), &v| (l.min(v), h.max(v)));
        assert!(hi - lo > BUMP_HEIGHT * 0.5, "only {} blocks of bumps", hi - lo);
    }
}
