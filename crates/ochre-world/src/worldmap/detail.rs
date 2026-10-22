//! The detail laid over the map's ground at block scale. Everything here must give the same
//! answer on every machine, as separate machines build the same chunks independently: integer
//! hash noise, and arithmetic with no `sin`, `exp` or `powf`, at full detail. Wider cells, which
//! only LOD meshes use, may call `resolved`, which does.

use super::climate::BiomeKind;
use super::{par_rows, WorldMap};
use ochre_base::math::smoothstep_f64 as smoothstep;
use ochre_base::noise::{fbm_filtered, perlin, ridged, unit};
use ochre_base::WATER_LEVEL;

const DETAIL_WAVELENGTH: f64 = 120.0;
const DETAIL_OCTAVES: u32 = 6;
/// Peak height of the detail, in blocks, over the gentlest ground and the roughest.
pub(crate) const GENTLE_DETAIL: f64 = 3.0;
pub(crate) const ROUGH_DETAIL: f64 = 38.0;

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

// Mountain ground is stepped into terraces in patches: plateaus joined by steep risers, each step
// between these two heights, as the rock strata weather into big ledges and cliff faces. The
// steps stack: they follow the height alone, so ground only ever climbs through them.
const TERRACE_STEP: (f64, f64) = (55.0, 120.0);
// The share of each step that is the riser, the rest being flat, and how much of the ground's own
// slope a terraced patch keeps between its ledges.
const TERRACE_RISER: f64 = 0.35;
const TERRACE_KEEPS: f64 = 0.2;
// Patches of terraces are noise this many blocks across, with this much of the ground in them
// (the noise is over `TERRACE_PATCH_START`).
const TERRACE_PATCH_WAVELENGTH: f64 = 2200.0;
const TERRACE_PATCH_START: f64 = -0.05;
// Each square of this many blocks has its own step size and height of ledge.
const TERRACE_CELL: f64 = 2400.0;

// A staircase of `step` blocks a flight, `phase` blocks up, the ledge flat for the first
// `1 - riser` of each flight and rising steeply over the rest.
fn staircase(height: f64, step: f64, phase: f64, riser: f64) -> f64 {
    let t = (height + phase) / step;
    let floor = t.floor();
    (floor + smoothstep(1.0 - riser, 1.0, t - floor)) * step - phase
}

/// A staircase of ledges at (`x`, `z`) whose flights are between `steps.0` and `steps.1` blocks
/// and whose heights are a function of `height` alone within each square of `cell` blocks, each
/// square with its own flight size and phase and fixed throughout, so ledges stay where they are on
/// level ground. The four squares round a point are blended, so the ledges ease from one square's
/// into the next's rather than jumping: the blend only ever mixes staircases, so the ground
/// still climbs through them and never falls back.
fn blended_staircase(
    seed: u32,
    x: f64,
    z: f64,
    height: f64,
    steps: (f64, f64),
    riser: f64,
    cell: f64,
) -> f64 {
    let (u, v) = (x / cell, z / cell);
    let (cx, cz) = (u.floor(), v.floor());
    // mostly one square's staircase, easing into the next's only over the middle of the way
    let (fx, fz) = (
        smoothstep(0.35, 0.65, u - cx),
        smoothstep(0.35, 0.65, v - cz),
    );
    let corner = |dx: f64, dz: f64| {
        let (ix, iz) = ((cx + dx) as i32, (cz + dz) as i32);
        let step = steps.0 + (steps.1 - steps.0) * unit(seed ^ 0x57A1, ix, iz);
        let phase = unit(seed ^ 0x57A2, ix, iz) * step;
        staircase(height, step, phase, riser)
    };
    let top = corner(0.0, 0.0) * (1.0 - fx) + corner(1.0, 0.0) * fx;
    let bottom = corner(0.0, 1.0) * (1.0 - fx) + corner(1.0, 1.0) * fx;
    top * (1.0 - fz) + bottom * fz
}

/// How much of the way, 0..1, mountain ground at (`x`, `z`) is terraced, before its height.
pub(crate) fn terrace_patch(seed: u32, x: f64, z: f64) -> f64 {
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
    let ledge = blended_staircase(
        seed,
        x,
        z,
        height,
        TERRACE_STEP,
        TERRACE_RISER,
        TERRACE_CELL,
    );
    // some of the ground's own rise is kept between the ledges, so they don't sit on dead flats
    let stepped = ledge + (height - ledge) * TERRACE_KEEPS;
    height + (stepped - height) * weight
}

// Patches of meadow on a mountain: noise this many blocks across, with the ground where it is over
// the second taking grass however steep.
// Cells either side that a mountain biome's presence is spread over, twice, to find the foothills.
const RANGE_REACH: usize = 18;
// Foothills: the ground round a range, within a few kilometres of it, is rough, steps up in small
// scarps and is cut into gullies and sharp ridges, as weather works at the edge of high ground.
const SCARP_STEP: (f64, f64) = (12.0, 34.0);
const SCARP_RISER: f64 = 0.4;
const SCARP_PATCH_WAVELENGTH: f64 = 500.0;
const SCARP_CELL: f64 = 900.0;
const GULLY_WAVELENGTH: f64 = 85.0;
const GULLY_HEIGHT: f64 = 22.0;

/// How much of the way, 0..1, ground `above_sea` blocks up and `range` (0..1) near a mountain range
/// is foothills: none far from a range or on the lowlands, all of it on the higher ground against one.
pub fn foothill_weight(range: f64, above_sea: f64) -> f64 {
    smoothstep(0.1, 0.55, range) * smoothstep(15.0, 90.0, above_sea)
}

/// `height` stepped into small scarps where `weight` of foothills is, in patches, as the terraces on
/// a mountain are but smaller and closer: a function of height and slow drifts only, so rising
/// ground climbs through them and never falls back.
pub fn scarps(seed: u32, x: f64, z: f64, weight: f64, height: f64) -> f64 {
    let patch = smoothstep(
        -0.2,
        0.25,
        perlin(
            seed ^ 0x5CA2,
            x / SCARP_PATCH_WAVELENGTH,
            z / SCARP_PATCH_WAVELENGTH,
        ),
    );
    let weight = weight * patch;
    if weight <= 0.0 {
        return height;
    }
    let ledge = blended_staircase(seed, x, z, height, SCARP_STEP, SCARP_RISER, SCARP_CELL);
    height + (ledge - height) * weight * 0.85
}

/// Erosion's work on the ground: gullies and sharp ridges between them, in -1..1 where the ridges
/// are the highs, scaled by `GULLY_HEIGHT` and the weight by whoever calls it. Ridged noise
/// has sharp crests and rounded troughs, as slopes weather into ridge lines with gullies off them.
pub fn gullies(seed: u32, x: f64, z: f64) -> f64 {
    (ridged(seed ^ 0x6011, x, z, GULLY_WAVELENGTH, 3) - 0.45) * 2.0
}

/// How high gullies cut into ground `weight` of foothills or mountain.
pub fn gully_relief(seed: u32, x: f64, z: f64, weight: f64, cell: f64) -> f64 {
    // finer than wide cells can show, so they fade out in them, as the other detail does
    let seen = 1.0 - smoothstep(4.0, 16.0, cell);
    if weight <= 0.0 || seen <= 0.0 {
        return 0.0;
    }
    gullies(seed, x, z) * GULLY_HEIGHT * weight * seen
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
        ochre_base::noise::resolved(DUNE_SPACING, cell)
    };
    let shape = 0.5 + (dune(seed, x, z) - 0.5) * resolved;
    shape * DUNE_HEIGHT * (0.4 + 0.6 * field) * room
}

/// Fills the roughness layer: steep and high ground is rough, plains and flat biomes are not.
pub fn roughness_layer(map: &mut WorldMap) {
    let dim = map.dim();
    let sea = WATER_LEVEL as f32;
    let cell = map.config.map_cell as f32;
    // how near each cell is to a range: the share of mountain in a wide window round it, widened
    let mountain: Vec<f32> = map
        .biome
        .iter()
        .map(|&k| (BiomeKind::from_u8(k) == BiomeKind::Mountain) as u8 as f32)
        .collect();
    let near = super::climate::blur_box(
        &super::climate::blur_box(&mountain, dim, RANGE_REACH),
        dim,
        RANGE_REACH,
    );
    map.range = near
        .iter()
        .map(|&v| ((v * 2.5).clamp(0.0, 1.0) * 255.0).round() as u8)
        .collect();
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
            // the foothills are rough too, the more the nearer a range
            let range = map.range[row * dim + col] as f64 / 255.0;
            r = r.max(0.8 * foothill_weight(range, above_sea));
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
            terraced_somewhere |= steepest > 1.2;
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
    fn test_foothills_are_near_a_range_and_up_off_the_lowland() {
        assert_eq!(0.0, foothill_weight(0.0, 200.0));
        assert_eq!(0.0, foothill_weight(0.9, 5.0));
        assert!(foothill_weight(0.9, 120.0) > 0.99);
        assert!(foothill_weight(0.3, 60.0) > 0.0 && foothill_weight(0.3, 60.0) < 0.9);
    }

    #[test]
    fn test_scarps_step_up_a_foothill_slope_without_falling_back() {
        // ground rising a half block a block, through patches of scarps and not
        let mut climbed = false;
        for (x, z) in [
            (0.0, 0.0),
            (3000.0, 900.0),
            (-2500.0, 4000.0),
            (800.0, -5000.0),
        ] {
            let out: Vec<f64> = (0..6000)
                .map(|i| scarps(1, x + i as f64 * 2.0, z, 1.0, 500.0 + i as f64 * 0.5))
                .collect();
            let worst_fall = out.windows(2).map(|w| w[0] - w[1]).fold(0.0, f64::max);
            assert!(
                worst_fall < 1.0,
                "fell {worst_fall} on a rising slope at ({x}, {z})"
            );
            climbed |= out.windows(2).map(|w| w[1] - w[0]).fold(0.0, f64::max) > 1.0;
        }
        assert!(climbed, "no scarp anywhere");
        // none away from a range
        assert_eq!(500.0, scarps(1, 9.0, 9.0, 0.0, 500.0));
    }

    #[test]
    fn test_gullies_cut_sharp_ridges_and_fade_out_in_wide_cells_and_off_the_weight() {
        let relief: Vec<f64> = (0..4000)
            .map(|i| gully_relief(2, i as f64 * 2.0, 17.0, 1.0, 1.0))
            .collect();
        let (lo, hi) = relief
            .iter()
            .fold((f64::MAX, f64::MIN), |(l, h), &v| (l.min(v), h.max(v)));
        assert!(
            hi - lo > 15.0 && hi <= GULLY_HEIGHT * 1.1 && lo >= -GULLY_HEIGHT,
            "{lo}..{hi}"
        );
        assert_eq!(0.0, gully_relief(2, 5.0, 5.0, 0.0, 1.0));
        assert_eq!(0.0, gully_relief(2, 5.0, 5.0, 1.0, 32.0));
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
