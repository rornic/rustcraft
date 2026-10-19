//! Rivers and lakes at block scale. The map stores them per cell: a river is a chain of cells,
//! each with a width and a water surface, drawn here as segments between cell centres so a
//! river 6 blocks wide stays 6 blocks wide on a map of 32-block cells.

use super::{WaterCell, WorldMap, NEIGHBOURS, NO_FLOW};

// Cells searched round a point for river segments: a segment runs one cell, and a wide river
// reaches further than a cell either side of its line.
const SEARCH: i64 = 2;
// Straight pieces a river's curve between two cells is measured as.
const CURVE_PIECES: usize = 8;

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

    /// The way each river cell's water leaves it: the direction it runs in, a blend of the way it
    /// came in and the way it goes on, so rivers can be drawn as smooth curves through the cells
    /// instead of kinked segments between their centres. Zero for cells with no river.
    pub fn river_tangents(&self) -> Vec<[f32; 2]> {
        let dim = self.dim();
        let direction = |i: usize| -> Option<[f32; 2]> {
            let d = self.down[i];
            if d == NO_FLOW {
                return None;
            }
            let (dx, dz) = NEIGHBOURS[d as usize];
            let len = ((dx * dx + dz * dz) as f32).sqrt();
            Some([dx as f32 / len, dz as f32 / len])
        };
        // the biggest river cell running into each cell
        let mut main_up = vec![usize::MAX; self.height.len()];
        for i in 0..self.height.len() {
            if self.river_width[i] <= 0.0 {
                continue;
            }
            if let Some(d) = self.down[i].ne(&NO_FLOW).then(|| {
                let (dx, dz) = NEIGHBOURS[self.down[i] as usize];
                ((i / dim) as i64 + dz) as usize * dim + ((i % dim) as i64 + dx) as usize
            }) {
                if main_up[d] == usize::MAX || self.flow[i] > self.flow[main_up[d]] {
                    main_up[d] = i;
                }
            }
        }
        (0..self.height.len())
            .map(|i| {
                if self.river_width[i] <= 0.0 {
                    return [0.0, 0.0];
                }
                let out = direction(i).unwrap_or([1.0, 0.0]);
                let into = (main_up[i] != usize::MAX)
                    .then(|| direction(main_up[i]))
                    .flatten()
                    .unwrap_or(out);
                let sum = [out[0] + into[0], out[1] + into[1]];
                let len = (sum[0] * sum[0] + sum[1] * sum[1]).sqrt();
                if len < 1e-3 {
                    out
                } else {
                    [sum[0] / len, sum[1] / len]
                }
            })
            .collect()
    }

    /// The river whose water or banks are nearest (`x`, `z`), if one is within a few cells.
    pub fn river_at(&self, tangents: &[[f32; 2]], x: f64, z: f64) -> Option<RiverHit> {
        self.river_hits(tangents, x, z)[0]
    }

    /// The two nearest rivers' water or banks: the nearest, and the nearest that isn't another
    /// stretch of the same river. Where a stream joins a river the river's bank is the nearer, and
    /// the stream's water would be dropped a few blocks short of it if only the nearest counted.
    /// Each stretch is a smooth curve through its cell and the next, see `river_tangents`.
    pub fn river_hits(&self, tangents: &[[f32; 2]], x: f64, z: f64) -> [Option<RiverHit>; 2] {
        let (u, v) = self.to_cell(x, z);
        let (px, pz) = (u.round() as i64, v.round() as i64);
        let max = self.dim() as i64 - 1;
        // each stretch near the point, as (margin, hit, the cell it starts in, the cell it runs to)
        let mut stretches: Vec<(f32, RiverHit, usize, usize)> = Vec::new();
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
                let into_river = self.water_kind[j] == WaterCell::River as u8;
                let (end_width, end_surface) = if into_river {
                    (self.river_width[j], self.water_surface[j])
                } else {
                    (width, self.water_surface[i])
                };
                let (ax, az) = self.cell_centre(cx as usize, cz as usize);
                let (bx, bz) = self.cell_centre(dx, dz);
                let (sx, sz) = (bx - ax, bz - az);
                let length = (sx * sx + sz * sz).sqrt();
                let along = [(sx / length) as f32, (sz / length) as f32];
                let t0 = tangents[i];
                let t1 = if into_river { tangents[j] } else { along };
                let (m0, m1) = (
                    [t0[0] as f64 * length, t0[1] as f64 * length],
                    [t1[0] as f64 * length, t1[1] as f64 * length],
                );
                // the curve as short straight pieces, and the nearest point on them
                let point = |t: f64| {
                    let (t2, t3) = (t * t, t * t * t);
                    let (h00, h10, h01, h11) = (
                        2.0 * t3 - 3.0 * t2 + 1.0,
                        t3 - 2.0 * t2 + t,
                        -2.0 * t3 + 3.0 * t2,
                        t3 - t2,
                    );
                    (
                        h00 * ax + h10 * m0[0] + h01 * bx + h11 * m1[0],
                        h00 * az + h10 * m0[1] + h01 * bz + h11 * m1[1],
                    )
                };
                let (mut best_d2, mut best_t) = (f64::MAX, 0.0);
                let mut last = point(0.0);
                for k in 1..=CURVE_PIECES {
                    let t_end = k as f64 / CURVE_PIECES as f64;
                    let next = point(t_end);
                    let (ex, ez) = (next.0 - last.0, next.1 - last.1);
                    let len2 = (ex * ex + ez * ez).max(1e-9);
                    let f = (((x - last.0) * ex + (z - last.1) * ez) / len2).clamp(0.0, 1.0);
                    let (qx, qz) = (last.0 + ex * f - x, last.1 + ez * f - z);
                    let d2 = qx * qx + qz * qz;
                    if d2 < best_d2 {
                        best_d2 = d2;
                        best_t = (k as f64 - 1.0 + f) / CURVE_PIECES as f64;
                    }
                    last = next;
                }
                let distance = best_d2.sqrt() as f32;
                let t = best_t as f32;
                let width = width + (end_width - width) * t;
                let surface = self.water_surface[i] + (end_surface - self.water_surface[i]) * t;
                // nearest the water first, not the centre line, so a wide river wins over a narrow one
                let margin = distance - width / 2.0;
                let hit = RiverHit {
                    distance,
                    width,
                    surface,
                    // a ford is shallow however wide the river is there
                    bed: surface
                        - if self.ford[i] == 1 {
                            0.9
                        } else {
                            depth_for_width(width)
                        },
                };
                stretches.push((margin, hit, i, j));
            }
        }
        let nearest = stretches.iter().min_by(|a, b| a.0.total_cmp(&b.0)).copied();
        let Some((_, first, from, to)) = nearest else {
            return [None, None];
        };
        // not the stretch before or after the nearest one, which is the same river
        let other = stretches
            .iter()
            .filter(|s| s.2 != from && s.2 != to && s.3 != from)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|s| s.1);
        [Some(first), other]
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
    fn test_a_right_angle_bend_turns_smoothly_not_in_a_kink() {
        let config = WorldConfig {
            size: 32 * 32,
            map_cell: 32,
        };
        let mut map = WorldMap::empty(1, config);
        // east along row 10 to the corner at cell 15, then south down column 15
        let mut path: Vec<(usize, usize)> = (5..=15).map(|cx| (cx, 10)).collect();
        path.extend((11..=20).map(|cz| (15, cz)));
        for (n, &(cx, cz)) in path.iter().enumerate() {
            let i = map.index(cx, cz);
            map.river_width[i] = 6.0;
            map.water_kind[i] = WaterCell::River as u8;
            map.water_surface[i] = 460.0;
            map.flow[i] = 1.0 + n as f32;
            map.down[i] = match path.get(n + 1) {
                Some(&(nx, nz)) => NEIGHBOURS
                    .iter()
                    .position(|&d| d == (nx as i64 - cx as i64, nz as i64 - cz as i64))
                    .unwrap() as u8,
                None => NO_FLOW,
            };
        }
        let tangents = map.river_tangents();
        // at the corner the water runs south east, halfway between the two legs
        let corner = tangents[map.index(15, 10)];
        assert!(
            (corner[0] - 0.707).abs() < 0.01 && (corner[1] - 0.707).abs() < 0.01,
            "{corner:?}"
        );
        // and the way before and after it is nearly straight
        let before = tangents[map.index(12, 10)];
        assert!(before[0] > 0.99);
        // the curve still passes through the corner's centre, so the river keeps its course
        let (cx, cz) = map.cell_centre(15, 10);
        assert!(map.river_at(&tangents, cx, cz).unwrap().distance < 0.01);
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
        let tangents = map.river_tangents();
        let on = map.river_at(&tangents, x + 7.0, z + 3.0).unwrap();
        assert!((on.distance - 3.0).abs() < 1e-3 && on.width == 8.0);
        assert!(map.river_at(&tangents, x, z + 200.0).is_none());
    }
}
