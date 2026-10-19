//! The world's geography on a coarse grid, made once per seed: continents, mountain ranges,
//! rivers and climate. Chunk generation and LOD sample it, adding local detail on top.
//!
//! A `WorldMap` is a plain value that saves to disk and loads back, so one machine can make it
//! and share it with the rest: nothing downstream depends on the float maths that made it.

pub mod io;
pub mod noise;
mod source;
mod terrain;

use std::sync::Arc;

pub use source::WorldTerrain;

/// Stamped on every saved map and world. Bump it whenever generation changes what it makes: edits
/// players make are stored against the generated terrain, so a world from another version is refused.
pub const GENERATOR_VERSION: u32 = 1;

/// How big the world is and how finely its map is drawn. Both are config, not constants.
#[derive(Debug, Copy, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(default)]
pub struct WorldConfig {
    /// Blocks across the square world, a multiple of `map_cell`.
    pub size: i64,
    /// Blocks across a map cell.
    pub map_cell: i64,
}

impl Default for WorldConfig {
    fn default() -> Self {
        Self {
            size: 65_536,
            map_cell: 32,
        }
    }
}

impl WorldConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.map_cell < 1 || self.size < 8 * self.map_cell || self.size % self.map_cell != 0 {
            return Err(format!(
                "world size {} must be a multiple of the map cell {} and at least 8 cells across",
                self.size, self.map_cell
            ));
        }
        Ok(())
    }

    /// Cells across the map.
    pub fn dim(&self) -> usize {
        (self.size / self.map_cell) as usize
    }

    pub fn half_size(&self) -> i64 {
        self.size / 2
    }
}

/// What sits on a stretch of water: none on dry land.
#[derive(Debug, Default, Copy, Clone, PartialEq, Eq)]
#[repr(u8)]
pub enum WaterCell {
    #[default]
    None = 0,
    Ocean = 1,
    Lake = 2,
    River = 3,
}

impl WaterCell {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Ocean,
            2 => Self::Lake,
            3 => Self::River,
            _ => Self::None,
        }
    }
}

/// The map: one value per cell of each layer, row-major `[z][x]`, with x and z running from the
/// world's north-west corner. Layers a later generation stage hasn't filled are zero.
#[derive(Debug, Clone, PartialEq)]
pub struct WorldMap {
    pub version: u32,
    pub seed: u32,
    pub config: WorldConfig,
    /// Ground height in blocks at each cell's centre.
    pub height: Vec<f32>,
    /// How far inland, -1 deep ocean to 1.
    pub continentalness: Vec<f32>,
}

impl WorldMap {
    pub fn dim(&self) -> usize {
        self.config.dim()
    }

    pub fn index(&self, cx: usize, cz: usize) -> usize {
        cz * self.dim() + cx
    }

    /// World coordinates to continuous cell coordinates, where whole numbers are cell centres.
    pub fn to_cell(&self, x: f64, z: f64) -> (f64, f64) {
        let half = self.config.half_size() as f64;
        let cell = self.config.map_cell as f64;
        ((x + half) / cell - 0.5, (z + half) / cell - 0.5)
    }

    /// The layer value at cell (`cx`, `cz`), clamped to the map's edge.
    fn at(&self, layer: &[f32], cx: i64, cz: i64) -> f32 {
        let max = self.dim() as i64 - 1;
        layer[self.index(cx.clamp(0, max) as usize, cz.clamp(0, max) as usize)]
    }

    pub fn bilinear(&self, layer: &[f32], x: f64, z: f64) -> f32 {
        let (u, v) = self.to_cell(x, z);
        let (fu, fv) = (u.floor(), v.floor());
        let (cx, cz) = (fu as i64, fv as i64);
        let (tx, tz) = ((u - fu) as f32, (v - fv) as f32);
        let top = self.at(layer, cx, cz) * (1.0 - tx) + self.at(layer, cx + 1, cz) * tx;
        let bottom = self.at(layer, cx, cz + 1) * (1.0 - tx) + self.at(layer, cx + 1, cz + 1) * tx;
        top * (1.0 - tz) + bottom * tz
    }

    /// Catmull-Rom interpolation of the layer: smooth through every cell, no creases at cell edges.
    pub fn bicubic(&self, layer: &[f32], x: f64, z: f64) -> f32 {
        let (u, v) = self.to_cell(x, z);
        let (fu, fv) = (u.floor(), v.floor());
        let (cx, cz) = (fu as i64, fv as i64);
        let (tx, tz) = ((u - fu) as f32, (v - fv) as f32);
        let mut rows = [0.0; 4];
        for (j, row) in rows.iter_mut().enumerate() {
            let z = cz + j as i64 - 1;
            *row = catmull_rom(
                [
                    self.at(layer, cx - 1, z),
                    self.at(layer, cx, z),
                    self.at(layer, cx + 1, z),
                    self.at(layer, cx + 2, z),
                ],
                tx,
            );
        }
        catmull_rom(rows, tz)
    }

    pub fn height_at(&self, x: f64, z: f64) -> f32 {
        self.bicubic(&self.height, x, z)
    }

    /// An `Arc`ed map, as every chunk task shares it.
    pub fn shared(self) -> Arc<Self> {
        Arc::new(self)
    }
}

fn catmull_rom(p: [f32; 4], t: f32) -> f32 {
    0.5 * (2.0 * p[1]
        + (p[2] - p[0]) * t
        + (2.0 * p[0] - 5.0 * p[1] + 4.0 * p[2] - p[3]) * t * t
        + (3.0 * (p[1] - p[2]) + p[3] - p[0]) * t * t * t)
}

/// Runs `f(row, cells)` over each row of a `dim`-wide layer on all cores.
pub(crate) fn par_rows<T: Send>(dim: usize, layer: &mut [T], f: impl Fn(usize, &mut [T]) + Sync) {
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let rows_per_thread = dim.div_ceil(threads);
    std::thread::scope(|scope| {
        for (n, rows) in layer.chunks_mut(rows_per_thread * dim).enumerate() {
            let f = &f;
            scope.spawn(move || {
                for (i, row) in rows.chunks_mut(dim).enumerate() {
                    f(n * rows_per_thread + i, row);
                }
            });
        }
    });
}

/// Makes the whole map for `seed`.
pub fn generate(seed: u32, config: WorldConfig) -> WorldMap {
    config.validate().expect("invalid world config");
    let mut map = terrain::base_terrain(seed, config);
    map.version = GENERATOR_VERSION;
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny() -> WorldMap {
        let config = WorldConfig {
            size: 1024,
            map_cell: 32,
        };
        let dim = config.dim();
        WorldMap {
            version: GENERATOR_VERSION,
            seed: 1,
            config,
            height: (0..dim * dim).map(|i| (i % dim) as f32 * 10.0).collect(),
            continentalness: vec![0.0; dim * dim],
        }
    }

    #[test]
    fn test_bicubic_passes_through_cell_centres() {
        let map = tiny();
        let half = map.config.half_size() as f64;
        // the centre of cell (5, 7) in world coordinates
        let (x, z) = (5.5 * 32.0 - half, 7.5 * 32.0 - half);
        assert!((map.height_at(x, z) - 50.0).abs() < 1e-3);
        assert!(
            (map.height_at(x + 16.0, z) - 55.0).abs() < 1e-3,
            "a ramp stays a ramp"
        );
    }

    #[test]
    fn test_sampling_clamps_at_the_edge() {
        let map = tiny();
        let half = map.config.half_size() as f64;
        assert!(map.height_at(-half - 500.0, 0.0).is_finite());
        assert!(map.height_at(half + 500.0, 0.0).is_finite());
    }

    #[test]
    fn test_config_validates() {
        assert!(WorldConfig::default().validate().is_ok());
        assert!(WorldConfig {
            size: 1000,
            map_cell: 32
        }
        .validate()
        .is_err());
        assert!(WorldConfig {
            size: 64,
            map_cell: 32
        }
        .validate()
        .is_err());
    }
}
