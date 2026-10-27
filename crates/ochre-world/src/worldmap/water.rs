//! Rivers and lakes at block scale. The map stores them per cell: a river is a chain of cells,
//! each with a width and a water surface, drawn here as segments between cell centres so a
//! river 6 blocks wide stays 6 blocks wide on a map of 32-block cells.

use super::{WaterCell, WorldMap, NEIGHBOURS, NO_FLOW};

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

// Points a river's water level is worked out at along each stretch between two cells, which is
// every few blocks: the ground it must lie under is looked at there, not once a cell.
pub(super) const PROFILE_POINTS: usize = 8;
// How far a river's level may sit over the lake or sea it runs into, a block of height for each
// 20 blocks it runs back from them, and how far above that it may be and still be brought down.
const MOUTH_SLOPE: f32 = 0.05;
const MOUTH_DROP: f32 = 8.0;
// The most, in blocks, a river is dug under the ground at its middle to stay under its banks.
const MAX_CUT: f64 = 6.0;
// Steps along a stretch a river into a lake looks for the shore in.
const SHORE_STEPS: usize = 60;
// How far, in cells, a river runs on past the centre of the water it drains into.
const MOUTH_REACH: f64 = 0.75;
// How many cells back from its mouth a river is brought down to the water it runs into.
const MOUTH_CELLS: usize = 6;
// How steeply, in blocks of height a block, a river comes down from the lake it leaves.
const OUTLET_SLOPE: f32 = 0.25;

/// How far under the ground beside it a river's water lies, and its banks stand over it at its
/// edge: up to a block, less for the narrowest streams, which would be cut far too deep.
pub(super) fn freeboard(width: f32) -> f32 {
    (width * 0.25).clamp(0.2, 1.0)
}

/// `WorldMap::still_beside`, from layers of water kinds and surfaces `dim` cells across.
pub(super) fn still_beside(dim: usize, kind: &[u8], surface: &[f32], i: usize) -> f32 {
    let dim = dim as i64;
    let (x, z) = (i as i64 % dim, i as i64 / dim);
    NEIGHBOURS
        .iter()
        .map(|&(dx, dz)| (x + dx, z + dz))
        .filter(|&(nx, nz)| (0..dim).contains(&nx) && (0..dim).contains(&nz))
        .map(|(nx, nz)| (nz * dim + nx) as usize)
        .filter(|&n| {
            matches!(
                WaterCell::from_u8(kind[n]),
                WaterCell::Lake | WaterCell::Ocean
            )
        })
        .map(|n| surface[n])
        .fold(f32::MIN, f32::max)
}

/// How deep, in blocks, a river `width` blocks across is.
pub(crate) fn depth_for_width(width: f32) -> f32 {
    // depth grows with width the way `river_depth` does with `river_width`, near enough
    (0.6 + (width - 1.2).max(0.0) / 3.0).min(12.0)
}

/// A river's course between two cells: a smooth curve through their centres, see `river_tangents`.
#[derive(Debug, Copy, Clone)]
pub(super) struct Stretch {
    from: usize,
    to: usize,
    a: (f64, f64),
    b: (f64, f64),
    m0: [f64; 2],
    m1: [f64; 2],
    width: f32,
    end_width: f32,
    into_river: bool,
}

impl Stretch {
    /// The curve `t` of the way along, 0..1.
    pub(super) fn point(&self, t: f64) -> (f64, f64) {
        let (t2, t3) = (t * t, t * t * t);
        let (h00, h10, h01, h11) = (
            2.0 * t3 - 3.0 * t2 + 1.0,
            t3 - 2.0 * t2 + t,
            -2.0 * t3 + 3.0 * t2,
            t3 - t2,
        );
        (
            h00 * self.a.0 + h10 * self.m0[0] + h01 * self.b.0 + h11 * self.m1[0],
            h00 * self.a.1 + h10 * self.m0[1] + h01 * self.b.1 + h11 * self.m1[1],
        )
    }

    pub(super) fn width_at(&self, t: f64) -> f32 {
        self.width + (self.end_width - self.width) * t as f32
    }
}

/// Each river stretch's water level every few blocks along it, see `PROFILE_POINTS`.
#[derive(Default)]
pub struct RiverProfiles {
    // the stretch's profile in `data` by the cell it starts in
    index: Vec<u32>,
    data: Vec<[f32; PROFILE_POINTS + 1]>,
    // how far along the stretch, 0..1, a river that runs into a lake reaches: its curve goes on
    // past the shore, over land, which would leave a stub of channel there
    ends: Vec<f32>,
    // each profile's course
    stretches: Vec<Stretch>,
    // by map cell, the stretches whose water, banks or valley can reach into it, see `index`: the
    // cell's in `near[near_start[cell]..near_start[cell + 1]]`
    near_start: Vec<u32>,
    near: Vec<u32>,
    // each stretch's river: the stretch at the head of the run of widest feeders it's part of, so one wide
    // river's stretches are one river, however many of them reach a point, and a tributary is its own
    stem: Vec<u32>,
}

impl RiverProfiles {
    const NONE: u32 = u32::MAX;

    /// The level of the water `t` of the way along the stretch from `cell`, 0..1.
    fn end(&self, cell: usize) -> f32 {
        self.ends[self.index[cell] as usize]
    }

    fn level(&self, cell: usize, t: f32) -> f32 {
        let profile = &self.data[self.index[cell] as usize];
        let at = t.clamp(0.0, 1.0) * PROFILE_POINTS as f32;
        let k = (at as usize).min(PROFILE_POINTS - 1);
        profile[k] + (profile[k + 1] - profile[k]) * (at - k as f32)
    }

    /// Lists each stretch in every cell of `map` its water, banks and valley reach into, a cell off its
    /// line for its curve's bend and `margin` blocks more, so a point need only look at its own cell's.
    pub fn index(&mut self, map: &WorldMap, margin: f64) {
        self.stem = self.stems();
        let (dim, cell) = (map.dim() as i64, map.config.map_cell as f64);
        let cells_of = |s: &Stretch| {
            let reach = super::source::river_reach(s.width.max(s.end_width)) as f64 + cell + margin;
            let (lo, hi) = (
                map.to_cell(s.a.0.min(s.b.0) - reach, s.a.1.min(s.b.1) - reach),
                map.to_cell(s.a.0.max(s.b.0) + reach, s.a.1.max(s.b.1) + reach),
            );
            let span = |a: f64, b: f64| (a.round() as i64).max(0)..=(b.round() as i64).min(dim - 1);
            (span(lo.0, hi.0), span(lo.1, hi.1))
        };
        let mut count = vec![0u32; (dim * dim) as usize + 1];
        for s in &self.stretches {
            let (xs, zs) = cells_of(s);
            for z in zs {
                for x in xs.clone() {
                    count[(z * dim + x) as usize + 1] += 1;
                }
            }
        }
        for i in 1..count.len() {
            count[i] += count[i - 1];
        }
        let mut next = count.clone();
        self.near = vec![0; *count.last().unwrap() as usize];
        for (n, s) in self.stretches.iter().enumerate() {
            let (xs, zs) = cells_of(s);
            for z in zs {
                for x in xs.clone() {
                    let slot = &mut next[(z * dim + x) as usize];
                    self.near[*slot as usize] = n as u32;
                    *slot += 1;
                }
            }
        }
        self.near_start = count;
    }

    // Each stretch's river, see `stem`: a stretch carries on the river of the one it runs into if it's the
    // widest that runs into it, and is the head of its own if not.
    fn stems(&self) -> Vec<u32> {
        let next = |n: usize| {
            let to = self.index[self.stretches[n].to];
            (to != Self::NONE).then_some(to as usize)
        };
        let mut widest = vec![u32::MAX; self.stretches.len()];
        for n in 0..self.stretches.len() {
            if let Some(m) = next(n) {
                let width = |k: u32| self.stretches[k as usize].width;
                if widest[m] == u32::MAX || width(n as u32) > width(widest[m]) {
                    widest[m] = n as u32;
                }
            }
        }
        let mut stem = vec![u32::MAX; self.stretches.len()];
        for start in 0..self.stretches.len() {
            // down the river until a stretch whose river is known, or that heads its own
            let mut path = vec![start];
            let found = loop {
                let n = *path.last().unwrap();
                if stem[n] != u32::MAX {
                    break stem[n];
                }
                match next(n) {
                    Some(m) if widest[m] == n as u32 && path.len() <= self.stretches.len() => {
                        path.push(m)
                    }
                    _ => break n as u32,
                }
            };
            for n in path {
                stem[n] = found;
            }
        }
        stem
    }

    /// The stretches that can reach into map cell `cell`, see `index`.
    pub fn near(&self, cell: usize) -> &[u32] {
        match self.near_start.get(cell..cell + 2) {
            Some(&[from, to]) => &self.near[from as usize..to as usize],
            _ => &[],
        }
    }
}

impl WorldMap {
    /// The highest level of the lake or sea of any cell round cell `i`, or far under any ground if none:
    /// a cell beside still water stays at least that high, or the water would spill over it, see
    /// `WorldTerrain::still_round`.
    pub(super) fn still_beside(&self, i: usize) -> f32 {
        still_beside(self.dim(), &self.water_kind, &self.water_surface, i)
    }

    // The cell a river cell drains into, if it has one.
    pub(super) fn downstream(&self, cx: usize, cz: usize) -> Option<(usize, usize)> {
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
    pub fn river_at(&self, profiles: &RiverProfiles, x: f64, z: f64) -> Option<RiverHit> {
        self.river_hits(profiles, x, z)[0]
    }

    /// The lake cell that drains into cell (`cx`, `cz`), if one does, or into a cell of dry land that
    /// drains into it: the lake spills over a rim of ground, and the river starts below.
    pub(super) fn outlet_lake(&self, cx: usize, cz: usize) -> Option<(usize, usize)> {
        let kind = |(x, z): (usize, usize)| self.water_kind[self.index(x, z)];
        self.feeders(cx, cz).find_map(|u| {
            if kind(u) == WaterCell::Lake as u8 {
                Some(u)
            } else if kind(u) == WaterCell::None as u8 {
                self.feeders(u.0, u.1)
                    .find(|&w| kind(w) == WaterCell::Lake as u8)
            } else {
                None
            }
        })
    }

    /// The cells round (`x`, `z`) that drain into it.
    pub(super) fn feeders(&self, x: usize, z: usize) -> impl Iterator<Item = (usize, usize)> + '_ {
        let dim = self.dim() as i64;
        NEIGHBOURS
            .iter()
            .map(move |&(nx, nz)| (x as i64 + nx, z as i64 + nz))
            .filter(move |&(ux, uz)| (0..dim).contains(&ux) && (0..dim).contains(&uz))
            .map(|(ux, uz)| (ux as usize, uz as usize))
            .filter(move |&(ux, uz)| self.downstream(ux, uz) == Some((x, z)))
    }

    /// The course from river cell `i` to the one it drains into, if it has one.
    pub(super) fn stretch(&self, tangents: &[[f32; 2]], i: usize) -> Option<Stretch> {
        let dim = self.dim();
        let (cx, cz) = (i % dim, i / dim);
        let width = self.river_width[i];
        if width <= 0.0 {
            return None;
        }
        let (dx, dz) = self.downstream(cx, cz)?;
        let j = self.index(dx, dz);
        // a river runs on into the lake or sea at its mouth, at its own width
        let into_river = self.water_kind[j] == WaterCell::River as u8;
        let end_width = if into_river {
            self.river_width[j]
        } else {
            width
        };
        let (mut a, mut b) = (self.cell_centre(cx, cz), self.cell_centre(dx, dz));
        // whether a lake drains into this cell: the river is its outlet, and starts at the lake
        let outlet = self.outlet_lake(cx, cz);
        let (sx, sz) = (b.0 - a.0, b.1 - a.1);
        let mut length = (sx * sx + sz * sz).sqrt();
        let along = [(sx / length) as f32, (sz / length) as f32];
        // a river runs on past the centre of the lake or sea cell it drains into, which may be on
        // the land at the water's edge, so it reaches the water the land really lets in
        let extra = self.config.map_cell as f64 * MOUTH_REACH;
        if !into_river {
            b = (b.0 + along[0] as f64 * extra, b.1 + along[1] as f64 * extra);
            length += extra;
        }
        if let Some((ux, uz)) = outlet {
            // from the middle of the lake cell that drains into it, so the river starts in the lake's
            // water and not short of it, on the rim's ground
            let from = self.cell_centre(ux, uz);
            length += ((a.0 - from.0).powi(2) + (a.1 - from.1).powi(2)).sqrt();
            a = from;
        }
        let t0 = tangents[i];
        let t1 = if into_river { tangents[j] } else { along };
        Some(Stretch {
            from: i,
            to: j,
            a,
            b,
            m0: [t0[0] as f64 * length, t0[1] as f64 * length],
            m1: [t1[0] as f64 * length, t1[1] as f64 * length],
            width,
            end_width,
            into_river,
        })
    }

    /// Water levels as the map has them, a straight run between each stretch's cells: what rivers
    /// are drawn at before `build_river_profiles` sets them to the ground.
    pub fn flat_river_profiles(&self, tangents: &[[f32; 2]]) -> RiverProfiles {
        let mut profiles = RiverProfiles {
            index: vec![RiverProfiles::NONE; self.height.len()],
            ..RiverProfiles::default()
        };
        for i in 0..self.height.len() {
            let Some(stretch) = self.stretch(tangents, i) else {
                continue;
            };
            profiles.stretches.push(stretch);
            let end = if stretch.into_river {
                self.water_surface[stretch.to]
            } else {
                self.water_surface[i]
            };
            let start = self.water_surface[i];
            let mut data = [0.0; PROFILE_POINTS + 1];
            for (k, level) in data.iter_mut().enumerate() {
                *level = start + (end - start) * k as f32 / PROFILE_POINTS as f32;
            }
            profiles.index[i] = profiles.data.len() as u32;
            profiles.data.push(data);
            profiles.ends.push(1.0);
        }
        profiles.index(self, 0.0);
        profiles
    }

    /// Each river stretch's water level every few blocks along it, held under the lowest of the
    /// `ground` round that point (in the river's own bed and banks, and where its banks end) and
    /// never rising downstream, so water is dug into the ground it runs over and no bank has to be built
    /// up to hold it. `centre` moves a point on a stretch's curve to where the river is really drawn.
    pub fn build_river_profiles(
        &self,
        tangents: &[[f32; 2]],
        ground: &(dyn Fn(f64, f64) -> f64 + Sync),
        centre: &(dyn Fn(f64, f64) -> (f64, f64) + Sync),
        in_still_water: &(dyn Fn(f64, f64) -> bool + Sync),
    ) -> RiverProfiles {
        let dim = self.dim();
        let stretches: Vec<Stretch> = (0..self.height.len())
            .filter_map(|i| self.stretch(tangents, i))
            .collect();
        // how high the ground allows the water at each point along each stretch
        let mut caps: Vec<[f32; PROFILE_POINTS + 1]> =
            vec![[0.0; PROFILE_POINTS + 1]; stretches.len()];
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
        let per_thread = stretches.len().div_ceil(threads).max(1);
        std::thread::scope(|scope| {
            for (chunk, out) in stretches
                .chunks(per_thread)
                .zip(caps.chunks_mut(per_thread))
            {
                scope.spawn(move || {
                    for (stretch, cap) in chunk.iter().zip(out.iter_mut()) {
                        for (k, level) in cap.iter_mut().enumerate() {
                            let t = k as f64 / PROFILE_POINTS as f64;
                            let width = stretch.width_at(t);
                            let (x, z) = centre(stretch.point(t).0, stretch.point(t).1);
                            let reach = width as f64 / 2.0 + 1.0;
                            let middle = ground(x, z);
                            let mut lowest = middle;
                            for (dx, dz) in
                                [(reach, 0.0), (-reach, 0.0), (0.0, reach), (0.0, -reach)]
                            {
                                lowest = lowest.min(ground(x + dx, z + dz));
                            }
                            // a bank much lower than the middle doesn't dig the river in after it: the
                            // water stands that far over it, and the bank is built up a little
                            *level = lowest.max(middle - MAX_CUT) as f32 - freeboard(width);
                        }
                    }
                });
            }
        });
        // a river that leaves a lake starts at the lake's level and comes down from it, through the
        // rim, so the two waters are joined and not parted by a bank of the rim's ground
        let cell_blocks = self.config.map_cell as f32;
        let mut from_lake = vec![false; stretches.len()];
        for (n, stretch) in stretches.iter().enumerate() {
            let (cx, cz) = (stretch.from % dim, stretch.from / dim);
            // the lake that drains into this river's first cell, if one does
            let Some((fx, fz)) = self.outlet_lake(cx, cz) else {
                continue;
            };
            let feeder = self.index(fx, fz);
            // the lake's water is a whole block, floored: the river's never lies in a block over it
            let top = self.water_surface[feeder].floor();
            from_lake[n] = true;
            for (k, level) in caps[n].iter_mut().enumerate() {
                let back = k as f32 / PROFILE_POINTS as f32 * cell_blocks;
                *level = level.max(top - back * OUTLET_SLOPE).min(top);
            }
        }
        // a river runs down to the lake or sea it drains into over its last few cells, if it comes
        // down to it: one far above it ends in a cliff or a fall
        let at: std::collections::HashMap<usize, usize> = stretches
            .iter()
            .enumerate()
            .map(|(n, s)| (s.from, n))
            .collect();
        for n in 0..stretches.len() {
            let mut hops = 0;
            let mut end = stretches[n];
            let (still, surface) = loop {
                let to = end.to;
                if self.water_kind[to] == WaterCell::Lake as u8
                    || self.water_kind[to] == WaterCell::Ocean as u8
                {
                    break (true, self.water_surface[to]);
                }
                hops += 1;
                match at.get(&to) {
                    Some(&next) if hops < MOUTH_CELLS => end = stretches[next],
                    _ => break (false, 0.0),
                }
            };
            if !still {
                continue;
            }
            let cell = self.config.map_cell as f32;
            for (k, level) in caps[n].iter_mut().enumerate() {
                // blocks back from the mouth: the cells between, and what's left of this stretch
                let back = (hops as f32 + 1.0 - k as f32 / PROFILE_POINTS as f32) * cell - cell;
                let ramp = surface + back.max(0.0) * MOUTH_SLOPE;
                if *level - ramp <= MOUTH_DROP {
                    *level = level.min(ramp);
                }
            }
        }
        // downstream of every stretch that runs into it, a river never rises: work from the
        // sources down
        let mut inflow = vec![0u32; stretches.len()];
        for s in &stretches {
            if let Some(&next) = at.get(&s.to) {
                inflow[next] += 1;
            }
        }
        let mut ready: Vec<usize> = (0..stretches.len()).filter(|&n| inflow[n] == 0).collect();
        let mut envelope = vec![f32::MAX; stretches.len()];
        let mut profiles = RiverProfiles {
            index: vec![RiverProfiles::NONE; self.height.len()],
            data: vec![[0.0; PROFILE_POINTS + 1]; stretches.len()],
            ends: vec![1.0; stretches.len()],
            stretches: stretches.clone(),
            ..RiverProfiles::default()
        };
        // a river into a lake or the sea ends where the land last gives way to the water, with a
        // rounded end, and goes no further over the land beside it: a pocket of low ground inland of
        // the shore doesn't count
        for (n, stretch) in stretches.iter().enumerate() {
            let into = self.water_kind[stretch.to];
            if into != WaterCell::Lake as u8 && into != WaterCell::Ocean as u8 {
                continue;
            }
            let wet = |step: usize| {
                let t = step as f64 / SHORE_STEPS as f64;
                let (x, z) = centre(stretch.point(t).0, stretch.point(t).1);
                in_still_water(x, z)
            };
            if wet(SHORE_STEPS) {
                let shore = (0..=SHORE_STEPS)
                    .rev()
                    .find(|&step| !wet(step))
                    .unwrap_or(0);
                profiles.ends[n] = ((shore + 3) as f32 / SHORE_STEPS as f32).min(1.0);
            }
        }
        let _ = dim;
        // sources first, every stretch after all that run into it
        let mut order = Vec::with_capacity(stretches.len());
        while let Some(n) = ready.pop() {
            order.push(n);
            let stretch = stretches[n];
            let mut level = envelope[n];
            for (slot, cap) in profiles.data[n].iter_mut().zip(&caps[n]) {
                level = level.min(*cap);
                *slot = level;
            }
            profiles.index[stretch.from] = n as u32;
            if let Some(&next) = at.get(&stretch.to) {
                envelope[next] = envelope[next].min(level);
                inflow[next] -= 1;
                if inflow[next] == 0 {
                    ready.push(next);
                }
            }
        }
        // a stream that runs into a river comes down to the river's level over its last cells, so
        // its water isn't left standing at its own over the river's, as a step beside it
        let mut feeders: Vec<Vec<usize>> = vec![Vec::new(); stretches.len()];
        for (n, stretch) in stretches.iter().enumerate() {
            if let Some(&next) = at.get(&stretch.to) {
                feeders[next].push(n);
            }
        }
        let cell = self.config.map_cell as f32;
        for next in 0..stretches.len() {
            let target = profiles.data[next][0];
            // the widest feeder is the river itself running on, which already comes down to its own next stretch
            let main = feeders[next]
                .iter()
                .copied()
                .max_by(|&a, &b| stretches[a].width.total_cmp(&stretches[b].width));
            let mut level: Vec<(usize, f32)> = feeders[next]
                .iter()
                .filter(|&&feeder| Some(feeder) != main)
                .map(|&feeder| (feeder, 0.0))
                .collect();
            let mut hops = 0;
            while !level.is_empty() && hops < MOUTH_CELLS {
                let mut up = Vec::new();
                for (n, run) in level {
                    // one that leaves a lake keeps the lake's level at its start
                    if from_lake[n] {
                        continue;
                    }
                    let mut lowered = false;
                    for k in 0..=PROFILE_POINTS {
                        let back = run + (1.0 - k as f32 / PROFILE_POINTS as f32) * cell;
                        let ramp = target + back * MOUTH_SLOPE;
                        let here = profiles.data[n][k];
                        if here > ramp && here - ramp <= MOUTH_DROP {
                            profiles.data[n][k] = ramp;
                            lowered = true;
                        }
                    }
                    if lowered {
                        // lowered a point at a time, a stretch never rises again further down it
                        for k in 1..=PROFILE_POINTS {
                            profiles.data[n][k] = profiles.data[n][k].min(profiles.data[n][k - 1]);
                        }
                        up.extend(feeders[n].iter().map(|&f| (f, run + cell)));
                    }
                }
                level = up;
                hops += 1;
            }
        }
        // streams lowered toward a river further on may now end under the one they run into: it comes down with them
        for &n in &order {
            if let Some(&next) = at.get(&stretches[n].to) {
                let end = profiles.data[n][PROFILE_POINTS];
                for level in profiles.data[next].iter_mut() {
                    *level = level.min(end);
                }
            }
        }
        profiles
    }

    /// The two nearest rivers' water or banks: the nearest, and the nearest that isn't another
    /// stretch of the same river. Where a stream joins a river the river's bank is the nearer, and
    /// the stream's water would be dropped a few blocks short of it if only the nearest counted.
    pub fn river_hits(&self, profiles: &RiverProfiles, x: f64, z: f64) -> [Option<RiverHit>; 2] {
        let (u, v) = self.to_cell(x, z);
        let max = self.dim() as f64 - 1.0;
        let cell = self.index(
            u.round().clamp(0.0, max) as usize,
            v.round().clamp(0.0, max) as usize,
        );
        // each stretch near the point, as (margin, hit, its river)
        let mut found: Vec<(f32, RiverHit, u32)> = Vec::new();
        {
            for &n in profiles.near(cell) {
                let stretch = profiles.stretches[n as usize];
                let i = stretch.from;
                // too far from the line between its cells for any of its curve, a cell off that at most, to reach the point
                let reach = super::source::river_reach(stretch.width.max(stretch.end_width)) as f64
                    + self.config.map_cell as f64;
                let (ex, ez) = (stretch.b.0 - stretch.a.0, stretch.b.1 - stretch.a.1);
                let f = (((x - stretch.a.0) * ex + (z - stretch.a.1) * ez)
                    / (ex * ex + ez * ez).max(1e-9))
                .clamp(0.0, 1.0);
                let (qx, qz) = (stretch.a.0 + ex * f - x, stretch.a.1 + ez * f - z);
                if qx * qx + qz * qz > reach * reach {
                    continue;
                }
                // the curve as short straight pieces, and the nearest point on them
                let (mut best_d2, mut best_t) = (f64::MAX, 0.0);
                let mut last = stretch.point(0.0);
                for k in 1..=CURVE_PIECES {
                    let next = stretch.point(k as f64 / CURVE_PIECES as f64);
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
                // a river into a lake stops at its shore, with a rounded end, not on over the land past it
                let end = profiles.end(i) as f64;
                if best_t > end {
                    let (ex, ez) = stretch.point(end);
                    best_d2 = (x - ex) * (x - ex) + (z - ez) * (z - ez);
                    best_t = end;
                    // and no banks round that end: they would be a bar across the mouth
                    let half = stretch.width_at(end) as f64 / 2.0;
                    if best_d2 > half * half {
                        continue;
                    }
                }
                let distance = best_d2.sqrt() as f32;
                let width = stretch.width_at(best_t);
                let surface = profiles.level(i, best_t as f32);
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
                found.push((margin, hit, profiles.stem[n as usize]));
            }
        }
        let nearest = found.iter().min_by(|a, b| a.0.total_cmp(&b.0)).copied();
        let Some((_, first, river)) = nearest else {
            return [None, None];
        };
        // not another stretch of the same river: a wide one's reach over several of them
        let other = found
            .iter()
            .filter(|s| s.2 != river)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|s| s.1);
        [Some(first), other]
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
            ..Default::default()
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
        let profiles = map.flat_river_profiles(&tangents);
        assert!(map.river_at(&profiles, cx, cz).unwrap().distance < 0.01);
    }

    #[test]
    fn test_river_is_found_along_its_line_and_not_far_from_it() {
        let config = WorldConfig {
            size: 32 * 32,
            map_cell: 32,
            ..Default::default()
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
        let profiles = map.flat_river_profiles(&tangents);
        let on = map.river_at(&profiles, x + 7.0, z + 3.0).unwrap();
        assert!((on.distance - 3.0).abs() < 1e-3 && on.width == 8.0);
        assert!(map.river_at(&profiles, x, z + 200.0).is_none());
    }

    #[test]
    fn test_a_river_never_rises_along_its_course() {
        for seed in [3, 5, 10] {
            let terrain = crate::worldmap::test_terrain(seed);
            let (map, (tangents, profiles)) = (terrain.map(), terrain.rivers());
            for i in 0..map.height.len() {
                let (Some(stretch), n) = (map.stretch(tangents, i), profiles.index[i]) else {
                    continue;
                };
                if n == RiverProfiles::NONE {
                    continue;
                }
                let levels = profiles.data[n as usize];
                assert!(
                    levels.windows(2).all(|w| w[1] <= w[0]),
                    "seed {seed} cell {i}: {levels:?}"
                );
                let next = profiles.index[stretch.to];
                if next != RiverProfiles::NONE {
                    let on = profiles.data[next as usize][0];
                    assert!(
                        on <= levels[PROFILE_POINTS] + 1e-3,
                        "seed {seed} cell {i} rises into {on}: {levels:?}"
                    );
                }
            }
        }
    }
}
