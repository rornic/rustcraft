//! Rivers and lakes at block scale. The map stores them per cell: a river is a chain of cells,
//! each with a width and a water surface, drawn here as segments between cell centres so a
//! river 6 blocks wide stays 6 blocks wide on a map of 32-block cells.

use super::{WaterCell, WorldMap, NEIGHBOURS, NO_FLOW};

// Cells searched round a point for river segments: a segment runs one cell, and a wide river
// reaches further than a cell either side of its line.
const SEARCH: i64 = 2;

/// The nearest river to a point.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct RiverHit {
    /// Blocks from the river's centre line.
    pub distance: f32,
    pub width: f32,
    /// Height of the water's surface there.
    pub surface: f32,
    /// Height of the riverbed at the centre line.
    pub bed: f32,
}

/// How deep, in blocks, a river `width` blocks across is.
pub fn depth_for_width(width: f32) -> f32 {
    // depth grows with width the way `river_depth` does with `river_width`, near enough
    (0.6 + (width - 1.2).max(0.0) / 3.0).min(12.0)
}

impl WorldMap {
    // The cell a river cell drains into, if it has one.
    fn downstream(&self, cx: usize, cz: usize) -> Option<(usize, usize)> {
        match self.down[self.index(cx, cz)] {
            NO_FLOW => None,
            d => {
                let (dx, dz) = NEIGHBOURS[d as usize];
                Some(((cx as i64 + dx) as usize, (cz as i64 + dz) as usize))
            }
        }
    }

    /// The river whose water or banks are nearest (`x`, `z`), if one is within a few cells.
    pub fn river_at(&self, x: f64, z: f64) -> Option<RiverHit> {
        let (u, v) = self.to_cell(x, z);
        let (px, pz) = (u.round() as i64, v.round() as i64);
        let max = self.dim() as i64 - 1;
        let mut best: Option<(f32, RiverHit)> = None;
        for cz in (pz - SEARCH).max(0)..=(pz + SEARCH).min(max) {
            for cx in (px - SEARCH).max(0)..=(px + SEARCH).min(max) {
                let i = self.index(cx as usize, cz as usize);
                let width = self.river_width[i];
                if width <= 0.0 {
                    continue;
                }
                let Some((dx, dz)) = self.downstream(cx as usize, cz as usize) else {
                    continue;
                };
                let j = self.index(dx, dz);
                // a river runs on into the lake or sea at its mouth, at its own width
                let (end_width, end_surface) = if self.water_kind[j] == WaterCell::River as u8 {
                    (self.river_width[j], self.water_surface[j])
                } else {
                    (width, self.water_surface[i])
                };
                let (ax, az) = self.cell_centre(cx as usize, cz as usize);
                let (bx, bz) = self.cell_centre(dx, dz);
                let (sx, sz) = (bx - ax, bz - az);
                let t = (((x - ax) * sx + (z - az) * sz) / (sx * sx + sz * sz)).clamp(0.0, 1.0);
                let (qx, qz) = (ax + sx * t - x, az + sz * t - z);
                let distance = (qx * qx + qz * qz).sqrt() as f32;
                let t = t as f32;
                let width = width + (end_width - width) * t;
                let surface = self.water_surface[i] + (end_surface - self.water_surface[i]) * t;
                // nearest the water first, not the centre line, so a wide river wins over a narrow one
                let margin = distance - width / 2.0;
                if best.is_none_or(|(m, _)| margin < m) {
                    let hit = RiverHit {
                        distance,
                        width,
                        surface,
                        bed: surface - depth_for_width(width),
                    };
                    best = Some((margin, hit));
                }
            }
        }
        best.map(|(_, hit)| hit)
    }

    /// The surface of the lake over (`x`, `z`) if the ground, `ground` high, is under it.
    pub fn lake_at(&self, x: f64, z: f64, ground: f32) -> Option<f32> {
        let (u, v) = self.to_cell(x, z);
        let (cx, cz) = (u.floor() as i64, v.floor() as i64);
        let max = self.dim() as i64 - 1;
        let mut surface = f32::MIN;
        for (dx, dz) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            let i = self.index(
                (cx + dx).clamp(0, max) as usize,
                (cz + dz).clamp(0, max) as usize,
            );
            if self.water_kind[i] == WaterCell::Lake as u8 {
                surface = surface.max(self.water_surface[i]);
            }
        }
        (surface > f32::MIN && ground < surface).then_some(surface)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worldmap::WorldConfig;

    #[test]
    fn test_depth_follows_width() {
        assert!(depth_for_width(5.0) < depth_for_width(40.0));
        assert!(depth_for_width(500.0) <= 12.0);
    }

    #[test]
    fn test_river_is_found_along_its_line_and_not_far_from_it() {
        let config = WorldConfig {
            size: 32 * 32,
            map_cell: 32,
        };
        let mut map = WorldMap::empty(1, config);
        // a river running east along row 10, from cell 5 to cell 15
        for cx in 5..15 {
            let i = map.index(cx, 10);
            map.river_width[i] = 8.0;
            map.water_kind[i] = WaterCell::River as u8;
            map.water_surface[i] = 460.0;
            map.down[i] = 0;
        }
        let i = map.index(15, 10);
        map.water_kind[i] = WaterCell::Ocean as u8;
        let (x, z) = map.cell_centre(8, 10);
        let on = map.river_at(x + 7.0, z + 3.0).unwrap();
        assert!((on.distance - 3.0).abs() < 1e-3 && on.width == 8.0);
        assert!(map.river_at(x, z + 200.0).is_none());
    }
}
