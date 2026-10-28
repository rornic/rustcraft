//! A regional tectonic snapshot. Plates carry motion, not a pre-drawn mountain mask.
//! Noise bends the boundaries and breaks up ridges; compression decides where uplift occurs.

use super::{par_rows, WorldConfig, WorldMap};
use ochre_base::math::smoothstep_f64 as smooth;
use ochre_base::noise::{fbm, ridged, unit};
use serde::Deserialize;

pub fn generate(seed: u32, config: WorldConfig) -> WorldMap {
    let model = Tectonics::new(seed, config.size, config.tectonics);
    let mut samples = vec![Sample::default(); config.dim() * config.dim()];
    par_rows(config.dim(), &mut samples, |row, out| {
        for (col, s) in out.iter_mut().enumerate() {
            let half = config.half_size() as f64;
            let x = (col as f64 + 0.5) * config.map_cell as f64 - half;
            let z = (row as f64 + 0.5) * config.map_cell as f64 - half;
            *s = model.sample(x, z);
        }
    });
    let mut map = WorldMap::empty(seed, config);
    for (i, s) in samples.into_iter().enumerate() {
        let ceiling = (config.bounds.max_y - 64) as f64;
        let room = (ceiling * 0.15).clamp(1.0, 300.0);
        let start = ceiling - room;
        let height = if s.height > start {
            ceiling - room * (-(s.height - start) / room).exp()
        } else {
            s.height
        };
        map.height[i] = height.max((config.bounds.min_y + 16) as f64) as f32;
        map.continentalness[i] = s.continentalness as f32;
        map.plate[i] = s.plate;
        map.boundary[i] = s.boundary;
        map.uplift[i] = s.uplift;
        map.geological_age[i] = (s.age * 255.0).round() as u8;
    }
    map
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(default)]
pub struct TectonicConfig {
    pub plates: u8,
    pub relief: f32,
}

impl Default for TectonicConfig {
    fn default() -> Self {
        Self {
            plates: 12,
            relief: 1.0,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Plate {
    pub x: f64,
    pub z: f64,
    pub vx: f64,
    pub vz: f64,
    crust: f64,
    age: f64,
}

/// Diagnostic codes: quiet, collision, subduction, extension, shear.
pub const COLLISION: u8 = 1;
pub const SUBDUCTION: u8 = 2;
pub const EXTENSION: u8 = 3;
pub const SHEAR: u8 = 4;

#[derive(Clone)]
pub struct Tectonics {
    seed: u32,
    half: f64,
    config: TectonicConfig,
    pub plates: Vec<Plate>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(ocean: bool) -> Tectonics {
        let mut model = Tectonics::new(42, 65536, TectonicConfig::default());
        model.plates = vec![
            Plate {
                x: -8000.0,
                z: 0.0,
                vx: 0.7,
                vz: 0.0,
                crust: if ocean { -1.5 } else { 1.5 },
                age: 0.8,
            },
            Plate {
                x: 8000.0,
                z: 0.0,
                vx: -0.7,
                vz: 0.0,
                crust: 1.5,
                age: 0.8,
            },
            Plate {
                x: 0.0,
                z: 60000.0,
                vx: 0.0,
                vz: 0.0,
                crust: 0.0,
                age: 0.8,
            },
        ];
        model
    }

    #[test]
    fn collision_uplift_requires_convergent_motion() {
        let active = fixture(false);
        let mut quiet = active.clone();
        for p in &mut quiet.plates {
            p.vx = 0.0;
            p.vz = 0.0;
        }
        let mut raised = 0;
        for x in (-6000..6000).step_by(100) {
            let a = active.sample(x as f64, 0.0);
            let b = quiet.sample(x as f64, 0.0);
            assert_eq!(b.uplift, 0.0);
            raised += (a.height > b.height + 100.0) as usize;
        }
        assert!(
            raised > 20,
            "collision should raise a broad belt, not a thin line"
        );
    }

    #[test]
    fn subduction_places_trench_on_oceanic_side() {
        let active = fixture(true);
        let mut quiet = active.clone();
        for p in &mut quiet.plates {
            p.vx = 0.0;
            p.vz = 0.0;
        }
        let mut trench = false;
        let mut range = false;
        for x in (-6000..6000).step_by(100) {
            let a = active.sample(x as f64, 0.0);
            let b = quiet.sample(x as f64, 0.0);
            trench |= x < 0 && a.height < b.height - 20.0;
            range |= x > 0 && a.uplift > 100.0;
        }
        assert!(
            trench && range,
            "need an ocean trench and a continental range"
        );
    }

    #[test]
    fn pair_orientation_does_not_flip_the_ridge_at_a_plate_edge() {
        let model = fixture(false);
        for x in -4000..4000 {
            let a = model.sample(x as f64, 0.0);
            let b = model.sample(x as f64 + 1.0, 0.0);
            assert!(
                (a.height - b.height).abs() < 30.0,
                "ridge discontinuity at {x}"
            );
        }
    }

    #[test]
    fn extension_and_shear_cut_continental_valleys() {
        let mut extension = fixture(false);
        extension.plates[0].vx = -0.7;
        extension.plates[1].vx = 0.7;
        let mut shear = extension.clone();
        shear.plates[0].vx = 0.0;
        shear.plates[1].vx = 0.0;
        shear.plates[0].vz = 0.7;
        shear.plates[1].vz = -0.7;
        let mut quiet = shear.clone();
        quiet.plates[0].vz = 0.0;
        quiet.plates[1].vz = 0.0;
        let mut rift_depth: f64 = 0.0;
        let mut shear_depth: f64 = 0.0;
        let mut shoulder: f64 = 0.0;
        let mut youngest: f64 = 1.0;
        for x in (-6000..6000).step_by(100) {
            let base = quiet.sample(x as f64, 0.0).height;
            let sample = extension.sample(x as f64, 0.0);
            let rift = sample.height - base;
            youngest = youngest.min(sample.age);
            let fault = shear.sample(x as f64, 0.0).height - base;
            rift_depth = rift_depth.max(-rift);
            shear_depth = shear_depth.max(-fault);
            shoulder = shoulder.max(rift);
        }
        assert!(
            rift_depth > 100.0 && shoulder > 80.0,
            "rift {rift_depth}, shoulder {shoulder}"
        );
        assert!(shear_depth > 70.0, "shear valley {shear_depth}");
        assert!(
            youngest < 0.4,
            "extension should expose young crust: {youngest}"
        );
    }

    #[test]
    fn oceanic_boundaries_build_ridges_and_island_arcs() {
        let mut ocean = fixture(true);
        ocean.plates[1].crust = -1.5;
        let mut quiet = ocean.clone();
        quiet.plates[0].vx = 0.0;
        quiet.plates[1].vx = 0.0;
        let mut divergent = ocean.clone();
        divergent.plates[0].vx = -0.7;
        divergent.plates[1].vx = 0.7;
        let mut arc: f64 = 0.0;
        let mut ridge: f64 = 0.0;
        for z in (-8000..8000).step_by(400) {
            for x in (-5000..5000).step_by(200) {
                let base = quiet.sample(x as f64, z as f64).height;
                arc = arc.max(ocean.sample(x as f64, z as f64).height - base);
                ridge = ridge.max(divergent.sample(x as f64, z as f64).height - base);
            }
        }
        assert!(arc > 400.0, "island arc {arc}");
        assert!(ridge > 200.0, "spreading ridge {ridge}");
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Sample {
    pub height: f64,
    pub continentalness: f64,
    pub plate: u8,
    pub boundary: u8,
    pub uplift: f32,
    /// Relative crust age, from newly formed (0) to old (1).
    pub age: f64,
}

impl Tectonics {
    pub fn new(seed: u32, size: i64, config: TectonicConfig) -> Self {
        let half = size as f64 * 0.5;
        // Best-candidate sites keep plates reasonably spaced without imposing a grid.
        let mut plates: Vec<Plate> = Vec::new();
        for i in 0..config.plates as i32 {
            let mut best = (f64::NEG_INFINITY, 0.0, 0.0);
            for candidate in 0..32 {
                let x = (unit(seed ^ 0x7101, i, candidate) * 2.0 - 1.0) * half;
                let z = (unit(seed ^ 0x7102, i, candidate) * 2.0 - 1.0) * half;
                let separation = plates
                    .iter()
                    .map(|p| (x - p.x).hypot(z - p.z))
                    .fold(f64::INFINITY, f64::min);
                if separation > best.0 {
                    best = (separation, x, z);
                }
            }
            let u = |salt| unit(seed ^ salt, i, 0);
            plates.push(Plate {
                x: best.1,
                z: best.2,
                vx: 2.0 * u(0x73) - 1.0,
                vz: 2.0 * u(0x74) - 1.0,
                crust: (u(0x75) - 0.5) * 0.45,
                age: 0.3 + u(0x76) * 0.7,
            });
        }
        Self {
            seed,
            half,
            config,
            plates,
        }
    }

    fn crust(&self, x: f64, z: f64) -> f64 {
        // Continental crust crosses plate boundaries: coastlines are not Voronoi edges.
        fbm(self.seed ^ 0xC017, x, z, self.half * 0.85, 3) * 1.8 + 0.12
    }

    pub fn sample(&self, x: f64, z: f64) -> Sample {
        let wx = x
            + 5500.0 * fbm(self.seed ^ 0xA11, x, z, 10000.0, 3)
            + 1000.0 * fbm(self.seed ^ 0xA13, x, z, 3500.0, 2);
        let wz = z
            + 5500.0 * fbm(self.seed ^ 0xA12, x, z, 10000.0, 3)
            + 1000.0 * fbm(self.seed ^ 0xA14, x, z, 3500.0, 2);
        let mut distances = [f64::INFINITY; 64];
        let mut nearest = (f64::INFINITY, 0usize);
        for (i, p) in self.plates.iter().enumerate() {
            let d = (wx - p.x).hypot(wz - p.z);
            distances[i] = d;
            if d < nearest.0 {
                nearest = (d, i);
            }
        }
        // Compact, continuous support includes every nearby plate. Selecting only N nearest
        // sites would introduce a height seam whenever the Nth and (N+1)th swap order.
        let reach = self.half * 2.0 / (self.plates.len() as f64).sqrt() * 0.35;
        let mut weights = [0.0; 64];
        for (i, _) in self.plates.iter().enumerate() {
            weights[i] = 1.0 - smooth(0.0, reach, distances[i] - nearest.0);
        }
        let sum: f64 = weights.iter().sum();
        let bias: f64 = self
            .plates
            .iter()
            .enumerate()
            .map(|(i, p)| weights[i] * p.crust / sum)
            .sum();
        let inherited_age: f64 = self
            .plates
            .iter()
            .enumerate()
            .map(|(i, p)| weights[i] * p.age / sum)
            .sum();
        let crust = self.crust(wx, wz) + bias;
        let continentalness = crust.clamp(-1.0, 1.0);
        let land = smooth(-0.10, 0.16, crust);
        let base = -650.0 * (1.0 - land)
            + 90.0 * land
            + 70.0 * fbm(self.seed ^ 0xB01, x, z, 7000.0, 3) * land;
        let mut uplift = 0.0;
        let mut trench = 0.0;
        let mut spreading = 0.0;
        let mut valley = 0.0;
        let mut resurfacing: f64 = 0.0;
        let mut dominant = (0.0, 0);
        for a in 0..self.plates.len() {
            if weights[a] == 0.0 {
                continue;
            }
            for b in a + 1..self.plates.len() {
                if weights[b] == 0.0 {
                    continue;
                }
                // Stable index order preserves the orientation of each pair.
                let pa = self.plates[a];
                let pb = self.plates[b];
                let dx = pb.x - pa.x;
                let dz = pb.z - pa.z;
                let len = dx.hypot(dz).max(1.0);
                let (nx, nz) = (dx / len, dz / len);
                let signed = ((wx - pa.x).powi(2) + (wz - pa.z).powi(2)
                    - (wx - pb.x).powi(2)
                    - (wz - pb.z).powi(2))
                    / (2.0 * len);
                let compression = (pa.vx - pb.vx) * nx + (pa.vz - pb.vz) * nz;
                let shear = ((pa.vx - pb.vx) * nz - (pa.vz - pb.vz) * nx).abs();
                let pair_weight = (weights[a] * weights[b]).sqrt();
                let active = smooth(0.08, 0.9, compression);
                let divergent = smooth(0.08, 0.9, -compression);
                let lateral =
                    smooth(0.15, 0.9, shear) * (1.0 - smooth(0.08, 0.4, compression.abs()));
                let boundary_x = wx - signed * nx;
                let boundary_z = wz - signed * nz;
                let ca = self.crust(boundary_x - nx * 3500.0, boundary_z - nz * 3500.0) + pa.crust;
                let cb = self.crust(boundary_x + nx * 3500.0, boundary_z + nz * 3500.0) + pb.crust;
                let collision = ca > 0.0 && cb > 0.0;
                let la = smooth(-0.14, 0.14, ca);
                let lb = smooth(-0.14, 0.14, cb);
                let along = -wx * nz + wz * nx;
                let ridges = ridged(self.seed ^ 0xB02, along, signed * 2.8, 1800.0, 4);
                let belt = (-(signed / 3600.0).powi(2)).exp();
                let collision_rise = la * lb * belt;
                let mut subduction_rise = 0.0;
                for (continental_side, strength) in
                    [(-signed, la * (1.0 - lb)), (signed, lb * (1.0 - la))]
                {
                    let range = (-((continental_side - 1900.0) / 2200.0).powi(2)).exp();
                    let plateau = 0.25
                        * smooth(2000.0, 4500.0, continental_side)
                        * (1.0 - smooth(7000.0, 12000.0, continental_side));
                    subduction_rise += strength * (range + plateau);
                    trench += active
                        * pair_weight
                        * strength
                        * 350.0
                        * (-((continental_side + 1500.0) / 1000.0).powi(2)).exp();
                }
                // Oceanic convergence builds a chain on one side of the trench. The
                // stable plate order keeps the overriding side fixed across the boundary.
                let ocean_pair = (1.0 - la) * (1.0 - lb);
                let arc = (-((signed - 2200.0) / 1400.0).powi(2)).exp();
                let islands = smooth(0.25, 0.72, ridged(self.seed ^ 0xB03, along, 0.0, 3500.0, 3));
                resurfacing = resurfacing.max(
                    pair_weight * (divergent * belt + active * ocean_pair * arc * islands).min(1.0),
                );
                uplift += active * pair_weight * ocean_pair * arc * islands * 1500.0;
                trench += active
                    * pair_weight
                    * ocean_pair
                    * 240.0
                    * (-((signed + 1200.0) / 900.0).powi(2)).exp();

                // Continental extension opens a trough between raised shoulders;
                // in the ocean it creates a broad submarine spreading ridge.
                let rift = (-(signed / 1200.0).powi(2)).exp();
                let shoulders = (-((signed.abs() - 2600.0) / 1500.0).powi(2)).exp();
                valley += divergent * pair_weight * land * 240.0 * rift;
                uplift += divergent * pair_weight * land * 220.0 * shoulders;
                spreading += divergent * pair_weight * (1.0 - land) * 420.0 * belt;
                // Strike-slip motion makes a narrower linear depression without
                // raising an artificial wall across the drainage network.
                valley += lateral * pair_weight * land * 150.0 * (-(signed / 850.0).powi(2)).exp();
                let rise =
                    (collision_rise + subduction_rise) * (650.0 + 1050.0 * ridges.powi(2)) * land;
                uplift += active * pair_weight * rise;
                let strength = belt * pair_weight;
                let kind = if compression > 0.08 {
                    if collision {
                        COLLISION
                    } else {
                        SUBDUCTION
                    }
                } else if compression < -0.08 {
                    EXTENSION
                } else if shear > 0.15 {
                    SHEAR
                } else {
                    0
                };
                if strength > dominant.0 {
                    dominant = (strength, kind);
                }
            }
        }
        // The region is an island/continental fragment, retaining the existing open ocean edge.
        let rounded = (x.abs().powf(2.4) + z.abs().powf(2.4)).powf(1.0 / 2.4) / self.half;
        let wander = fbm(self.seed ^ 0xED6E, x, z, 4500.0, 3) * 0.10;
        let edge = smooth(0.70, 0.94, rounded + wander);
        let relief = self.config.relief as f64;
        Sample {
            height: (base + (uplift + spreading - valley) * relief - trench) * (1.0 - edge)
                - 700.0 * edge,
            continentalness: continentalness * (1.0 - edge) - edge,
            plate: nearest.1 as u8,
            boundary: if dominant.0 > 0.10 { dominant.1 } else { 0 },
            uplift: (uplift * relief * (1.0 - edge)) as f32,
            age: (inherited_age * (1.0 - resurfacing)).clamp(0.0, 1.0),
        }
    }
}
