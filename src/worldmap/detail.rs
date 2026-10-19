//! The detail laid over the map's ground at block scale. Everything here must give the same
//! answer on every machine, as separate machines build the same chunks independently: integer
//! hash noise, and arithmetic with no `sin`, `exp` or `powf`, at full detail. Wider cells, which
//! only LOD meshes use, may call `resolved`, which does.

use super::climate::BiomeKind;
use super::noise::{fbm_filtered, perlin};
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
const DUNE_HEIGHT: f64 = 18.0;
// The share of each dune that's the gentle windward slope.
const WINDWARD: f64 = 0.75;
// How far, in dunes, the rows bend, and over what distance, so crests wander.
const CREST_BEND: f64 = 0.8;
const CREST_BEND_WAVELENGTH: f64 = 400.0;
// Where dunes stand tall and where they lie low.
const DUNE_FIELD_WAVELENGTH: f64 = 700.0;
// Dunes build up inland from the beach and give way to rock on high ground.
const DUNE_SHORE: (f64, f64) = (2.0, 14.0);
const DUNE_HIGH_GROUND: (f64, f64) = (70.0, 140.0);

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
            r *= match BiomeKind::from_u8(map.biome[row * dim + col]) {
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
        assert!(-down > up * 2.0, "steepest rise {up}, steepest fall {down}");
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
