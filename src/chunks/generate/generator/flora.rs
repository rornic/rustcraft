use crate::block::BlockType;
use crate::chunks::generate::trees::hash;

// Salts for the flora's hashes, so they don't line up with the trees' or each other.
const PLANT_SALT: u64 = 0x5EED_F10A;
const DRIFT_SALT: u64 = 0xD81F_7F10;
const KIND_SALT: u64 = 0x6A55_0F3B;

// How wide the patches flowers grow in drifts across are, in blocks.
const DRIFT: i64 = 6;
// The share of patches with flowers in, and how thick they grow in the thickest.
const DRIFT_SHARE: f64 = 0.3;
const DRIFT_DENSITY: f64 = 0.3;
// How much of the open grass carries a tuft of tall grass, and of the forest floor.
const GRASS_DENSITY: f64 = 0.3;
const FOREST_GRASS_DENSITY: f64 = 0.12;
// How much of the forest floor, and the ground under a lone tree's crown, is ferns.
const FERN_DENSITY: f64 = 0.25;
// How much of the grass is short tufts, in the open and in the shade, and how much is wild
// grass gone to seed; the rest is tall grass.
const SHORT_SHARE: f64 = 0.45;
const SHADED_SHORT_SHARE: f64 = 0.65;
const SEED_SHARE: f64 = 0.15;
// The temperatures, see `NoiseGenerator::temperature`, the flora thins out between, to none
// at the first.
const COLD: (f32, f32) = (0.25, 0.4);

fn unit(h: u64) -> f64 {
    (h >> 11) as f64 / (1u64 << 53) as f64
}

/// The plant on the grass at (x, z), if any: grasses in the open, ferns in the shade, flowers
/// in drifts of one kind, all thinning out in the cold.
pub(super) fn flora_at(
    seed: u32,
    x: i64,
    z: i64,
    shaded: bool,
    temperature: f32,
) -> Option<BlockType> {
    let seed = seed as u64;
    let roll = unit(hash(seed ^ PLANT_SALT, x, z));
    let warm = ((temperature - COLD.0) / (COLD.1 - COLD.0)).clamp(0.0, 1.0) as f64;

    let drift = hash(seed ^ DRIFT_SALT, x.div_euclid(DRIFT), z.div_euclid(DRIFT));
    let in_drift = unit(drift);
    let flowers = if in_drift < DRIFT_SHARE {
        DRIFT_DENSITY * (1.0 - in_drift / DRIFT_SHARE)
    } else {
        0.0
    };
    let ferns = if shaded { FERN_DENSITY } else { 0.0 };
    let grass = if shaded {
        FOREST_GRASS_DENSITY
    } else {
        GRASS_DENSITY
    };

    let roll = roll / warm.max(f64::EPSILON);
    if roll < flowers {
        // one kind of flower to a drift
        Some(if drift & 1 == 0 {
            BlockType::Poppy
        } else {
            BlockType::Dandelion
        })
    } else if roll < flowers + ferns {
        Some(BlockType::Fern)
    } else if roll < flowers + ferns + grass {
        let kind = unit(hash(seed ^ KIND_SALT, x, z));
        let short = if shaded {
            SHADED_SHORT_SHARE
        } else {
            SHORT_SHARE
        };
        Some(if kind < short {
            BlockType::ShortGrass
        } else if kind > 1.0 - SEED_SHARE {
            BlockType::SeedGrass
        } else {
            BlockType::TallGrass
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // how many of each grass (all three kinds), fern, poppy and dandelion grow over a patch
    fn count(shaded: bool, temperature: f32) -> [usize; 4] {
        let mut counts = [0; 4];
        for x in 0..200 {
            for z in 0..200 {
                if let Some(plant) = flora_at(7, x, z, shaded, temperature) {
                    let kind = match plant {
                        BlockType::TallGrass | BlockType::ShortGrass | BlockType::SeedGrass => 0,
                        BlockType::Fern => 1,
                        BlockType::Poppy => 2,
                        BlockType::Dandelion => 3,
                        other => panic!("{other:?} isn't flora"),
                    };
                    counts[kind] += 1;
                }
            }
        }
        counts
    }

    #[test]
    fn test_grass_and_flowers_in_the_open_and_ferns_only_in_the_shade() {
        let open = count(false, 0.6);
        let total = 200.0 * 200.0;
        assert!((0.2..0.45).contains(&(open[0] as f64 / total)), "{open:?}");
        assert_eq!(0, open[1]);
        assert!(open[2] > 0 && open[3] > 0, "{open:?}");
        let forest = count(true, 0.6);
        assert!(forest[1] > forest[0], "{forest:?}");
    }

    #[test]
    fn test_flora_thins_out_in_the_cold() {
        let (mild, chilly, frozen) = (count(false, 0.6), count(false, 0.32), count(false, 0.2));
        let sum = |c: [usize; 4]| c.iter().sum::<usize>();
        assert!(sum(chilly) < sum(mild) && sum(chilly) > 0);
        assert_eq!(0, sum(frozen));
    }

    #[test]
    fn test_a_drift_is_one_kind_of_flower() {
        for (dx, dz) in [(0, 0), (6, 0), (12, 18), (-6, 30)] {
            let kinds: Vec<BlockType> = (0..DRIFT)
                .flat_map(|x| (0..DRIFT).map(move |z| (x, z)))
                .filter_map(|(x, z)| flora_at(3, dx + x, dz + z, false, 0.6))
                .filter(|p| matches!(p, BlockType::Poppy | BlockType::Dandelion))
                .collect();
            assert!(kinds.windows(2).all(|w| w[0] == w[1]), "{kinds:?}");
        }
    }
}
