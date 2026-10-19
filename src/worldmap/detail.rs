//! The detail laid over the map's ground at block scale. Everything here must give the same
//! answer on every machine, as separate machines build the same chunks independently: integer
//! hash noise, and arithmetic with no `sin`, `exp` or `powf`, at full detail. Wider cells, which
//! only LOD meshes use, may call `resolved`, which does.

use super::climate::BiomeKind;
use super::noise::{fbm_filtered, perlin, perlin3};
use super::terrain::smoothstep;
use super::{par_rows, WorldMap};
use crate::chunks::generate::WATER_LEVEL;

const DETAIL_WAVELENGTH: f64 = 120.0;
const DETAIL_OCTAVES: u32 = 6;
/// Peak height of the detail, in blocks, over the gentlest ground and the roughest.
pub const GENTLE_DETAIL: f64 = 3.0;
pub const ROUGH_DETAIL: f64 = 38.0;

// Rows of dunes driven by a wind along +x: a long gentle slope up to each crest, and a steep
// face falling away behind it.
const DUNE_SPACING: f64 = 90.0;
const DUNE_HEIGHT: f64 = 16.0;
// The share of each dune that's the gentle windward slope.
const WINDWARD: f64 = 0.6;
// How far, in dunes, the rows bend, and over what distance, so crests wander.
const CREST_BEND: f64 = 0.8;
const CREST_BEND_WAVELENGTH: f64 = 400.0;
// Where dunes stand tall and where they lie low.
const DUNE_FIELD_WAVELENGTH: f64 = 700.0;
// Dunes build up inland from the beach and give way to rock on high ground.
const DUNE_SHORE: (f64, f64) = (2.0, 14.0);
const DUNE_HIGH_GROUND: (f64, f64) = (70.0, 140.0);

// Where the ground is steeper than this (see `terrain_block`), cliffs start to take shape: about
// half of a range's columns are over the first, and nine in ten are under the second. And
// by the second the surface can move this far either way, which is not far, or cliffs turn abrupt.
const CLIFF_GRADIENT: (f64, f64) = (1.0, 2.8);
const CLIFF_REACH: f64 = 9.0;

/// How many blocks the surface of ground this steep can be moved up or down by `cliff_density`,
/// 0 on ground too gentle for cliffs.
pub fn cliff_reach(gradient: f64) -> f64 {
    CLIFF_REACH * smoothstep(CLIFF_GRADIENT.0, CLIFF_GRADIENT.1, gradient)
}

/// Noise in -1..1 that cliffs and overhangs are cut from: the surface in a column is wherever
/// `height - y + reach * cliff_density` crosses zero, which is a different height at each y, so
/// ledges overhang and caves open in the face.
pub fn cliff_density(seed: u32, x: i64, y: i64, z: i64) -> f64 {
    let (fx, fy, fz) = (x as f64 + 0.5, y as f64 + 0.5, z as f64 + 0.5);
    0.65 * perlin3(seed ^ 0xC11F, fx / 14.0, fy / 14.0, fz / 14.0)
        + 0.35 * perlin3(seed ^ 0xC120, fx / 6.0, fy / 6.0, fz / 6.0)
}

// Mountain ground is stepped into terraces in patches: plateaus joined by steep risers, each step
// between these two heights, as the rock strata weather into big ledges and cliff faces. The
// steps stack: they follow the height alone, so ground only ever climbs through them.
const TERRACE_STEP: (f64, f64) = (55.0, 120.0);
// The share of each step that is the riser, the rest being flat, and how much of the ground's own
// slope a terraced patch keeps between its ledges.
const TERRACE_RISER: f64 = 0.45;
const TERRACE_KEEPS: f64 = 0.2;
// Patches of terraces are noise this many blocks across, with this much of the ground in them
// (the noise is over `TERRACE_PATCH_START`).
const TERRACE_PATCH_WAVELENGTH: f64 = 2200.0;
const TERRACE_PATCH_START: f64 = -0.05;

/// How much of the way, 0..1, mountain ground at (`x`, `z`) is terraced, before its height.
pub fn terrace_patch(seed: u32, x: f64, z: f64) -> f64 {
    smoothstep(
        TERRACE_PATCH_START,
        TERRACE_PATCH_START + 0.3,
        perlin(
            seed ^ 0x7E21,
            x / TERRACE_PATCH_WAVELENGTH,
            z / TERRACE_PATCH_WAVELENGTH,
        ),
    )
}

/// `height`, blocks above the sea `above_sea`, stepped into terraces on mountain ground where a
/// patch of them is, more the higher it is. The same at every cell size.
///
/// Each patch's steps are a different size and sit at a different height, drifting slowly from
/// place to place, so ledges don't line up from one patch to the next. The result is a function
/// of height and these slow drifts only, so rising ground climbs through the steps in turn and
/// never falls back between them.
pub fn terraced(seed: u32, x: f64, z: f64, mountain: bool, height: f64, above_sea: f64) -> f64 {
    if !mountain {
        return height;
    }
    // the same at every cell size, so LOD levels beside each other differ only by what a slope
    // changes between their sample points, which their skirts cover
    let weight = terrace_patch(seed, x, z) * smoothstep(40.0, 130.0, above_sea);
    if weight <= 0.0 {
        return height;
    }
    let wander = 0.5 + 0.5 * perlin(seed ^ 0x7E22, x / 3100.0, z / 3100.0);
    let step = TERRACE_STEP.0 + (TERRACE_STEP.1 - TERRACE_STEP.0) * wander.clamp(0.0, 1.0);
    // where in its step the sea level falls, which moves ledges up and down between patches
    let phase = (0.5 + 0.5 * perlin(seed ^ 0x7E25, x / 2600.0, z / 2600.0)) * step;
    let t = (height + phase) / step;
    let floor = t.floor();
    let ledge = (floor + smoothstep(1.0 - TERRACE_RISER, 1.0, t - floor)) * step - phase;
    // some of the ground's own rise is kept between the ledges, so they don't sit on dead flats
    let stepped = ledge + (height - ledge) * TERRACE_KEEPS;
    height + (stepped - height) * weight
}

/// How high the detail rises over, or falls under, the map's ground at (`x`, `z`) on ground
/// `roughness` rough (0..1) and `above_sea` blocks up, in cells `cell` blocks wide.
pub fn detail(seed: u32, x: f64, z: f64, cell: f64, roughness: f64, above_sea: f64) -> f64 {
    let amplitude = GENTLE_DETAIL + (ROUGH_DETAIL - GENTLE_DETAIL) * roughness;
    // calm near the shoreline, where noise would break the coast up into specks of land
    let shore = 0.2 + 0.8 * smoothstep(0.0, 8.0, above_sea.abs());
    fbm_filtered(seed, x, z, DETAIL_WAVELENGTH, DETAIL_OCTAVES, cell) * amplitude * shore
}

// 0..1 along the rows of dunes, 1 on each crest.
fn dune(seed: u32, x: f64, z: f64) -> f64 {
    let bend = perlin(seed, x / CREST_BEND_WAVELENGTH, z / CREST_BEND_WAVELENGTH) * CREST_BEND;
    let t = (x / DUNE_SPACING + bend).rem_euclid(1.0);
    if t < WINDWARD {
        smoothstep(0.0, WINDWARD, t)
    } else {
        smoothstep(1.0, WINDWARD, t)
    }
}

/// Height the dunes add on desert `above_sea` blocks up. Cells too wide to see the rows of dunes
/// see their average height, half a crest.
pub fn dunes(seed: u32, x: f64, z: f64, cell: f64, above_sea: f64) -> f64 {
    let room = smoothstep(DUNE_SHORE.0, DUNE_SHORE.1, above_sea)
        * smoothstep(DUNE_HIGH_GROUND.1, DUNE_HIGH_GROUND.0, above_sea);
    if room <= 0.0 {
        return 0.0;
    }
    let field = smoothstep(
        -0.5,
        0.3,
        perlin(
            seed.wrapping_add(1),
            x / DUNE_FIELD_WAVELENGTH,
            z / DUNE_FIELD_WAVELENGTH,
        ),
    );
    let resolved = if cell <= 1.0 {
        1.0
    } else {
        super::noise::resolved(DUNE_SPACING, cell)
    };
    let shape = 0.5 + (dune(seed, x, z) - 0.5) * resolved;
    shape * DUNE_HEIGHT * (0.4 + 0.6 * field) * room
}

/// Fills the roughness layer: steep and high ground is rough, plains and flat biomes are not.
pub fn roughness_layer(map: &mut WorldMap) {
    let dim = map.dim();
    let sea = WATER_LEVEL as f32;
    let cell = map.config.map_cell as f32;
    let mut out = vec![0u8; dim * dim];
    par_rows(dim, &mut out, |row, cells| {
        let h = |x: usize, z: usize| map.height[z * dim + x];
        for (col, out) in cells.iter_mut().enumerate() {
            let (xl, xr) = (col.saturating_sub(1), (col + 1).min(dim - 1));
            let (zl, zr) = (row.saturating_sub(1), (row + 1).min(dim - 1));
            let dx = (h(xr, row) - h(xl, row)) / ((xr - xl).max(1) as f32 * cell);
            let dz = (h(col, zr) - h(col, zl)) / ((zr - zl).max(1) as f32 * cell);
            let slope = (dx * dx + dz * dz).sqrt() as f64;
            let above_sea = (h(col, row) - sea) as f64;
            let mut r = (0.75 * smoothstep(0.1, 0.6, slope)
                + 0.35 * smoothstep(60.0, 300.0, above_sea))
            .clamp(0.08, 1.0);
            let kind = BiomeKind::from_u8(map.biome[row * dim + col]);
            if kind == BiomeKind::Mountain {
                r = r.max(0.75);
            }
            r *= match kind {
                BiomeKind::Desert => 0.6,
                BiomeKind::Glacier => 0.5,
                BiomeKind::Sea => 0.15,
                _ => 1.0,
            };
            *out = (r * 255.0).round() as u8;
        }
    });
    map.roughness = out;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dunes_rise_inland_but_not_on_beaches_or_mountains() {
        let rise = |above_sea: f64| {
            (0..200)
                .map(|x| dunes(1, x as f64 * 3.0, 0.0, 1.0, above_sea))
                .fold(0.0, f64::max)
        };
        assert_eq!(0.0, rise(1.0));
        assert_eq!(0.0, rise(200.0));
        assert!(
            rise(30.0) > DUNE_HEIGHT * 0.5,
            "dunes only rose {}",
            rise(30.0)
        );
        assert!(rise(30.0) <= DUNE_HEIGHT);
    }

    #[test]
    fn test_dunes_rise_gently_and_fall_steeply() {
        let (mut up, mut down) = (0.0f64, 0.0f64);
        for x in 0..2000 {
            let step = dune(1, x as f64 + 1.0, 0.0) - dune(1, x as f64, 0.0);
            (up, down) = (up.max(step), down.min(step));
        }
        assert!(-down > up * 1.3, "steepest rise {up}, steepest fall {down}");
    }

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
    fn test_terraces_stack_climbing_never_falling_back_between_ledges() {
        // ground rising steadily half a block a block, along a line through terraced patches and not
        let profile = |x: f64, y: f64, above: f64| {
            (0..6000)
                .map(|i| {
                    let h = 448.0 + above + i as f64 * 0.5;
                    terraced(1, x + i as f64 * 3.0, y, true, h, above + i as f64 * 0.5)
                })
                .collect::<Vec<f64>>()
        };
        let mut terraced_somewhere = false;
        for (x, y) in [
            (0.0, 0.0),
            (5000.0, -3000.0),
            (-8000.0, 4000.0),
            (2000.0, 9000.0),
        ] {
            let out = profile(x, y, 100.0);
            // never down: going up the slope never takes the ground back down, bar the little a
            // drifting patch edge gives, which is under a block over a metre
            let worst_fall = out.windows(2).map(|w| w[0] - w[1]).fold(0.0, f64::max);
            assert!(
                worst_fall < 1.0,
                "ground fell {worst_fall} on a rising slope at ({x}, {y})"
            );
            // and a terraced patch is steeper than the slope somewhere: that is its cliff
            let steepest = out.windows(2).map(|w| w[1] - w[0]).fold(0.0, f64::max);
            terraced_somewhere |= steepest > 2.0;
        }
        assert!(terraced_somewhere, "no terraces on any of the slopes");
        // flat ground stays flat, and ground that is not mountain, or low, is left alone
        assert_eq!(600.0, terraced(1, 9.0, 9.0, false, 600.0, 200.0));
        assert_eq!(500.0, terraced(1, 9.0, 9.0, true, 500.0, 10.0));
    }

    #[test]
    fn test_terraces_are_in_patches_with_ledges_at_different_heights() {
        let patched: Vec<bool> = (0..400)
            .map(|i| terrace_patch(1, i as f64 * 97.0, i as f64 * 41.0) > 0.5)
            .collect();
        let share = patched.iter().filter(|&&p| p).count() as f64 / patched.len() as f64;
        assert!(
            (0.15..0.65).contains(&share),
            "{share} of the ground terraced"
        );
        // the height the first riser, where the ground is steeper than twice its slope, starts at
        // differs from one terraced place to another
        let first_ledge = |x: f64, z: f64| {
            let at = |h: f64| terraced(1, x, z, true, 600.0 + h, 200.0 + h);
            (0..30_000)
                .map(|i| i as f64 * 0.1)
                .find(|&h| at(h + 0.1) - at(h) > 0.2)
        };
        let places: Vec<(f64, f64)> = (0..400)
            .map(|i| (i as f64 * 710.0, i as f64 * 330.0 - 40_000.0))
            .filter(|&(x, z)| terrace_patch(1, x, z) > 0.95)
            .take(8)
            .collect();
        let ledges: Vec<f64> = places
            .iter()
            .filter_map(|&(x, z)| first_ledge(x, z))
            .collect();
        let spread = ledges.iter().cloned().fold(f64::MIN, f64::max)
            - ledges.iter().cloned().fold(f64::MAX, f64::min);
        assert!(ledges.len() >= 2 && spread > 8.0, "ledges {ledges:?}");
    }

    #[test]
    fn test_rougher_ground_has_taller_detail() {
        let spread = |roughness: f64| {
            (0..400)
                .map(|i| detail(3, i as f64 * 5.3, i as f64 * 2.1, 1.0, roughness, 100.0).abs())
                .fold(0.0, f64::max)
        };
        assert!(spread(1.0) > spread(0.0) * 4.0);
    }
}
