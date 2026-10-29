//! Coarse rock provinces and tilted sedimentary beds. These are geological inputs, not textures.

use super::{par_rows, tectonics, WorldMap};
use ochre_base::noise::fbm;

/// The rock at a point. Clay is included as a soft sedimentary material.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rock {
    Stone,
    Basalt,
    Marble,
    Limestone,
    Clay,
    Sandstone,
}

impl Rock {
    /// Fraction of a droplet's erosion demand taken from this material.
    pub fn hydraulic_erodibility(self) -> f32 {
        match self {
            Self::Stone | Self::Limestone => 0.8,
            Self::Basalt => 0.55,
            Self::Marble => 0.7,
            Self::Clay => 1.0,
            Self::Sandstone => 0.85,
        }
    }

    /// Relative rate at which streams cut this material. These are artistic tuning values.
    pub fn stream_erodibility(self) -> f32 {
        match self {
            Self::Stone => 1.0,
            Self::Basalt => 0.65,
            Self::Marble => 0.8,
            Self::Limestone => 1.0,
            Self::Clay => 1.5,
            Self::Sandstone => 1.15,
        }
    }
}

// The province describes a vertical column. Sedimentary provinces contain alternating beds;
// volcanic and metamorphic provinces are treated as massive rock for this first iteration.
const GENERIC: u8 = 0;
const SEDIMENTARY: u8 = 1;
const VOLCANIC: u8 = 2;
const METAMORPHIC: u8 = 3;
const BED_HEIGHT: f32 = 160.0;

pub(super) fn generate(map: &mut WorldMap) {
    let config = map.config;
    let mut geology = vec![(GENERIC, 0.0f32); map.height.len()];
    par_rows(map.dim(), &mut geology, |row, cells| {
        for (col, output) in cells.iter_mut().enumerate() {
            let i = row * config.dim() + col;
            let (x, z) = map.cell_centre(col, row);
            let region = fbm(map.seed ^ 0xB314_20A7, x, z, 12_000.0, 3);
            let province = if map.continentalness[i] < -0.2
                || ((map.boundary[i] == tectonics::SUBDUCTION
                    || map.boundary[i] == tectonics::EXTENSION)
                    && region > -0.05)
            {
                VOLCANIC
            } else if map.boundary[i] == tectonics::COLLISION
                && map.uplift[i] > 100.0
                && region > -0.12
            {
                METAMORPHIC
            } else {
                SEDIMENTARY
            };
            let phase = fbm(map.seed ^ 0x73A9_8C4D, x, z, 18_000.0, 2) as f32 * 400.0;
            *output = (province, phase);
        }
    });
    for (i, (province, phase)) in geology.into_iter().enumerate() {
        map.geology[i] = province;
        map.strata_offset[i] = phase;
    }
}

impl WorldMap {
    /// Rock at a map cell and elevation, including beds below the surface.
    pub fn rock_at_cell(&self, i: usize, y: f32) -> Rock {
        match self.geology[i] {
            VOLCANIC => Rock::Basalt,
            METAMORPHIC => Rock::Marble,
            SEDIMENTARY => {
                let bed = ((y - self.strata_offset[i]) / BED_HEIGHT).floor() as i32;
                match bed.rem_euclid(4) {
                    0 | 1 => Rock::Limestone,
                    2 => Rock::Clay,
                    _ => Rock::Sandstone,
                }
            }
            _ => Rock::Stone,
        }
    }

    /// Rock at world coordinates, sampled from the nearest geological map cell.
    pub fn rock_at(&self, x: f64, y: f32, z: f64) -> Rock {
        let (u, v) = self.to_cell(x, z);
        let max = (self.dim() - 1) as f64;
        let i = self.index(
            u.round().clamp(0.0, max) as usize,
            v.round().clamp(0.0, max) as usize,
        );
        self.rock_at_cell(i, y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worldmap::WorldConfig;

    #[test]
    fn sedimentary_beds_continue_below_zero_and_follow_the_offset() {
        let mut map = WorldMap::empty(
            1,
            WorldConfig {
                size: 256,
                ..Default::default()
            },
        );
        map.geology[0] = SEDIMENTARY;
        assert_eq!(map.rock_at_cell(0, -1.0), Rock::Sandstone);
        assert_eq!(map.rock_at_cell(0, 0.0), Rock::Limestone);
        assert_eq!(map.rock_at_cell(0, 320.0), Rock::Clay);
        map.strata_offset[0] = 80.0;
        assert_eq!(map.rock_at_cell(0, 79.0), Rock::Sandstone);
        assert_eq!(map.rock_at_cell(0, 80.0), Rock::Limestone);
    }

    #[test]
    fn generated_geology_repeats_for_the_same_seed() {
        let config = WorldConfig {
            size: 8192,
            ..Default::default()
        };
        let mut a = super::super::tectonics::generate(42, config);
        let mut b = a.clone();
        generate(&mut a);
        generate(&mut b);
        assert_eq!(a.geology, b.geology);
        assert_eq!(a.strata_offset, b.strata_offset);
        assert!(a
            .geology
            .iter()
            .all(|&p| (SEDIMENTARY..=METAMORPHIC).contains(&p)));
        assert!(a.strata_offset.iter().all(|p| p.is_finite()));
        for i in [0, a.height.len() / 2, a.height.len() - 1] {
            let (x, z) = a.cell_centre(i % a.dim(), i / a.dim());
            assert_eq!(a.rock_at(x, 123.0, z), a.rock_at_cell(i, 123.0));
        }
    }

    #[test]
    fn softer_rock_erodes_faster_under_the_same_stream() {
        let config = WorldConfig {
            size: 1024,
            ..Default::default()
        };
        let mut hard = WorldMap::empty(1, config);
        for z in 0..hard.dim() {
            for x in 0..hard.dim() {
                let i = hard.index(x, z);
                hard.height[i] = if z == 0 { -1.0 } else { 320.0 + z as f32 * 4.0 };
                hard.geology[i] = VOLCANIC;
            }
        }
        let mut soft = hard.clone();
        soft.geology.fill(SEDIMENTARY);
        super::super::erosion::stream_power(&mut hard);
        super::super::erosion::stream_power(&mut soft);
        let hard_sum: f64 = hard.height.iter().map(|&h| h as f64).sum();
        let soft_sum: f64 = soft.height.iter().map(|&h| h as f64).sum();
        assert!(soft_sum < hard_sum, "soft {soft_sum}, hard {hard_sum}");
    }

    #[test]
    fn softer_rock_slumps_further_without_losing_mass() {
        let config = WorldConfig {
            size: 1024,
            ..Default::default()
        };
        let mut hard = WorldMap::empty(1, config);
        hard.height.fill(500.0);
        hard.geology.fill(VOLCANIC);
        let spike = hard.index(16, 16);
        hard.height[spike] = 900.0;
        let mut soft = hard.clone();
        soft.geology.fill(SEDIMENTARY);
        // The spike begins in a clay bed and exposes lower beds as it slumps.
        soft.strata_offset.fill(500.0);
        let before: f64 = hard.height.iter().map(|&h| h as f64).sum();
        super::super::erosion::thermal(&mut hard);
        super::super::erosion::thermal(&mut soft);
        assert!(
            soft.height[spike] < hard.height[spike],
            "soft {}, hard {}",
            soft.height[spike],
            hard.height[spike]
        );
        for map in [&hard, &soft] {
            let after: f64 = map.height.iter().map(|&h| h as f64).sum();
            assert!((after - before).abs() < 1.0, "mass {before} -> {after}");
        }
    }

    #[test]
    fn droplets_cut_soft_rock_more_than_hard_rock() {
        let config = WorldConfig {
            size: 4096,
            ..Default::default()
        };
        let mut hard = WorldMap::empty(7, config);
        let dim = hard.dim();
        for z in 0..dim {
            for x in 0..dim {
                let i = hard.index(x, z);
                hard.height[i] = 340.0 + z as f32;
                hard.geology[i] = VOLCANIC;
            }
        }
        let mut soft = hard.clone();
        soft.geology.fill(SEDIMENTARY);
        let before = hard.height.clone();
        super::super::erosion::hydraulic(&mut hard);
        super::super::erosion::hydraulic(&mut soft);
        let cut = |map: &WorldMap| -> f64 {
            before
                .iter()
                .zip(&map.height)
                .map(|(a, b)| (a - b).max(0.0) as f64)
                .sum()
        };
        assert!(
            cut(&soft) > cut(&hard),
            "soft {}, hard {}",
            cut(&soft),
            cut(&hard)
        );
    }
}
