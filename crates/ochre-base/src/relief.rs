//! Ground shapes both the world map and the chunk generator need to agree on. Like
//! [`crate::noise`], everything here gives the same answer on every machine.

use crate::math::smoothstep_f64;
use crate::noise::{perlin, perlin3};

// Where the ground is steeper than this (see `terrain_block`), cliffs start to take shape: about
// half of a range's columns are over the first, and nine in ten are under the second. And
// by the second the surface can move this far either way, which is not far, or cliffs turn abrupt.
const CLIFF_GRADIENT: (f64, f64) = (1.0, 2.8);
const CLIFF_REACH: f64 = 9.0;

/// How many blocks the surface of ground this steep can be moved up or down by `cliff_density`,
/// 0 on ground too gentle for cliffs.
pub fn cliff_reach(gradient: f64) -> f64 {
    CLIFF_REACH * smoothstep_f64(CLIFF_GRADIENT.0, CLIFF_GRADIENT.1, gradient)
}

/// Noise in -1..1 that cliffs and overhangs are cut from: the surface in a column is wherever
/// `height - y + reach * cliff_density` crosses zero, which is a different height at each y, so
/// ledges overhang and caves open in the face.
pub fn cliff_density(seed: u32, x: i64, y: i64, z: i64) -> f64 {
    let (fx, fy, fz) = (x as f64 + 0.5, y as f64 + 0.5, z as f64 + 0.5);
    0.65 * perlin3(seed ^ 0xC11F, fx / 14.0, fy / 14.0, fz / 14.0)
        + 0.35 * perlin3(seed ^ 0xC120, fx / 6.0, fy / 6.0, fz / 6.0)
}

const MEADOW_WAVELENGTH: f64 = 55.0;
const MEADOW_OVER: f64 = 0.12;

/// Whether (`x`, `z`) is in a patch of meadow, which clings to a mountain's faces where the rest
/// is bare rock. About a third of the ground.
pub fn meadow_patch(seed: u32, x: f64, z: f64) -> bool {
    perlin(seed ^ 0x3EAD, x / MEADOW_WAVELENGTH, z / MEADOW_WAVELENGTH) > MEADOW_OVER
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cliffs_only_on_steep_ground_and_never_past_their_reach() {
        assert_eq!(0.0, cliff_reach(0.0));
        assert_eq!(0.0, cliff_reach(CLIFF_GRADIENT.0));
        assert_eq!(CLIFF_REACH, cliff_reach(CLIFF_GRADIENT.1 + 1.0));
        assert!(cliff_reach(2.0) > 0.0 && cliff_reach(2.0) < CLIFF_REACH);
        for i in 0..500 {
            let d = cliff_density(4, i * 3, i % 40, -i * 7);
            assert!((-1.0..=1.0).contains(&d), "{d}");
        }
    }

    #[test]
    fn test_meadow_patches_cover_a_third_in_patches_not_specks() {
        let patch: Vec<bool> = (0..4000)
            .map(|i| meadow_patch(2, i as f64 * 3.0, 11.0))
            .collect();
        let share = patch.iter().filter(|&&p| p).count() as f64 / patch.len() as f64;
        assert!((0.2..0.5).contains(&share), "{share}");
        // runs of meadow are tens of blocks long
        let flips = patch.windows(2).filter(|w| w[0] != w[1]).count();
        assert!(flips < 4000 / 8, "{flips} changes in 12 km: speckled");
    }
}
