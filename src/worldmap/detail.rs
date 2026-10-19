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

// Mountain ground is stepped into terraces in patches: plateaus joined by near-vertical risers,
// each step between these two heights, as the rock strata weather into big ledges and cliff faces.
const TERRACE_STEP: (f64, f64) = (55.0, 120.0);
// The share of each step that is the riser, the rest being flat.
const TERRACE_RISER: f64 = 0.3;
// Patches of terraces are noise this many blocks across, with this much of the ground in them
// (the noise is over `TERRACE_PATCH_START`), and the strata in each tilt by up to this many
// blocks a block, which is what points the ledges different ways.
const TERRACE_PATCH_WAVELENGTH: f64 = 2200.0;
const TERRACE_PATCH_START: f64 = -0.05;
const TERRACE_TILT: f64 = 0.45;

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
/// patch of them is, more the higher it is, in cells `cell` blocks wide: wide cells see the
/// ground without its steps, as the steps are finer than they are. The strata are tilted
/// differently from patch to patch, so their ledges run every which way, not along the contours.
pub fn terraced(
    seed: u32,
    x: f64,
    z: f64,
    cell: f64,
    mountain: bool,
    height: f64,
    above_sea: f64,
) -> f64 {
    if !mountain {
        return height;
    }
    let weight = terrace_patch(seed, x, z)
        * smoothstep(40.0, 130.0, above_sea)
        * (1.0 - smoothstep(2.0, 12.0, cell));
    if weight <= 0.0 {
        return height;
    }
    // each patch's step height and the lie of its strata wander from place to place
    let wander = 0.5 + 0.5 * perlin(seed ^ 0x7E22, x / 3100.0, z / 3100.0);
    let step = TERRACE_STEP.0 + (TERRACE_STEP.1 - TERRACE_STEP.0) * wander.clamp(0.0, 1.0);
    let tilt_x = perlin(seed ^ 0x7E23, x / 2700.0, z / 2700.0) * TERRACE_TILT;
    let tilt_z = perlin(seed ^ 0x7E24, x / 2700.0, z / 2700.0) * TERRACE_TILT;
    let plane = height + tilt_x * x + tilt_z * z;
    let t = plane / step;
    let floor = t.floor();
    let stepped =
        (floor + smoothstep(1.0 - TERRACE_RISER, 1.0, t - floor)) * step - tilt_x * x - tilt_z * z;
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
    fn test_terraces_are_few_and_big_and_only_on_mountains() {
        // a hillside climbing half a block a block, along a line through terraced and plain patches
        let steepest_in = |mountain: bool, above_sea: f64, range: std::ops::Range<i64>| {
            let h = |x: f64| {
                terraced(
                    1,
                    x,
                    0.0,
                    1.0,
                    mountain,
                    448.0 + above_sea + x * 0.5,
                    above_sea,
                )
            };
            range
                .map(|i| (h(i as f64 + 1.0) - h(i as f64)).abs())
                .fold(0.0, f64::max)
        };
        // somewhere along it steps turn the hillside into cliffs, and they are tall
        assert!(
            steepest_in(true, 200.0, 0..20_000) > 2.0,
            "{}",
            steepest_in(true, 200.0, 0..20_000)
        );
        // but not low mountain, nor ground that is not mountain
        assert!(steepest_in(true, 20.0, 0..20_000) < 0.55);
        assert!(steepest_in(false, 200.0, 0..20_000) < 0.55);
        // patches: some of the ground is left as it was, and some is terraced
        let patched: Vec<bool> = (0..400)
            .map(|i| terrace_patch(1, i as f64 * 97.0, i as f64 * 41.0) > 0.5)
            .collect();
        let share = patched.iter().filter(|&&p| p).count() as f64 / patched.len() as f64;
        assert!(
            (0.15..0.65).contains(&share),
            "{share} of the ground terraced"
        );
        // and cells too wide for the steps see the ground without them
        assert_eq!(540.0, terraced(1, 5.0, 5.0, 32.0, true, 540.0, 200.0));
    }

    #[test]
    fn test_terraces_run_different_ways() {
        // where a terraced patch is, the direction the ledges strike in (across which way the
        // ground steps) differs from patch to patch: sample the step's tilt over a wide area
        let tilts: Vec<(f64, f64)> = (0..60)
            .map(|i| {
                let (x, z) = (i as f64 * 1900.0, i as f64 * 1300.0);
                (
                    perlin(1 ^ 0x7E23, x / 2700.0, z / 2700.0),
                    perlin(1 ^ 0x7E24, x / 2700.0, z / 2700.0),
                )
            })
            .collect();
        let (mut north, mut south, mut east, mut west) = (0, 0, 0, 0);
        for (tx, tz) in tilts {
            if tz > 0.1 {
                north += 1;
            }
            if tz < -0.1 {
                south += 1;
            }
            if tx > 0.1 {
                east += 1;
            }
            if tx < -0.1 {
                west += 1;
            }
        }
        assert!(
            north > 3 && south > 3 && east > 3 && west > 3,
            "{north} {south} {east} {west}"
        );
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
