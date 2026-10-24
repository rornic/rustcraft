use std::sync::Arc;

use super::climate::{BiomeKind, LAPSE_PER_BLOCK};
use super::detail::{detail, dunes, foothill_weight, gully_relief, scarps, terraced};
use super::water::{freeboard, RiverHit, RiverProfiles};
use super::FeatureKind;
use super::{WaterCell, WorldMap};
use ochre_base::biome::{BiomeId, Biomes, Climate};
use ochre_base::math::smoothstep_f64 as smoothstep;
use ochre_base::noise::perlin;
use ochre_base::terrain_source::{SpringPart, TerrainSample, TerrainSource, WaterKind};
use ochre_base::WATER_LEVEL;

// How far a biome's cover is wobbled, so its edge wanders off the lines between the cells.
const BIOME_WOBBLE: f64 = 0.15;

// Land inland of the sea and out of any lake is at least this much over the sea, or it would flood.
const LAND_ABOVE_SEA: f64 = 0.3;

// Over the sea by up to this much the ground's detail is eased out, so a flat shore stays flat.
const SHORE_SMOOTH: f64 = 5.0;

// How far either side of still water's level, in blocks, the ground is held to the side of it its
// smooth ground is on, see `contain`, and over how far the ground's detail is calmed to nothing at
// the water's edge, so a shore and the bed out from it are smooth.
const HOLD: f64 = 1.5;
const CALM: f64 = 8.0;

// How far over still water's level, in blocks, ground surrounded by its cells is still under it: the water
// is drawn a whole block, under the level its cells were found by, so ground just over that would stand
// out of it as strips, parting it into pools.
const DROWN: f64 = 2.0;

// A river within this many blocks of still water's level is joined to it, as one running into or out of
// it is: their waters meet and the still water's is the one inside its shore.
const JOIN: f64 = 1.5;

// A river's valley floor is never lower than this over the sea or a lake near it, which would flood it in
// a wider inlet than the river.
const VALLEY_ABOVE_SEA: f64 = 0.6;

// How far past a river's banks, in blocks, its valley floor eases back out into the land's own ground.
const VALLEY_WIDTH: f64 = 40.0;

// Cells at least this wide leave a river's channel to the map's own carving.
const RIVER_DETAIL_CELL: f64 = 16.0;
// A river rises in a round pool this many blocks across its middle, deepening from the first to the second
// under its level, in a rim of ground this wide held over it.
const SPRING_POOL: f32 = 2.6;
const SPRING_DEPTH: (f64, f64) = (0.8, 2.2);
const SPRING_RIM: f32 = 1.6;
// Past the rim the ground falls away from it a block for each block out, over this far, so it stands in a mound.
const SPRING_MOUND: f32 = 4.0;

// The ground over a river's channel: a parabolic bed up to the water's edge, then the bank
// easing out into the ground beside it.
fn channel(ground: f32, river: RiverHit) -> f32 {
    let half = river.width / 2.0;
    if river.distance < half {
        return ground.min(bed_height(river));
    }
    // the ground at the water's edge is a block over it, whatever the land is there, and eases to
    // the land's own height across the banks: cut down where it's higher, and built up a little
    // where a dip would leave the water standing over it, so the water is always held in by ground
    // and over the block the water's surface lies in by at least half of it, or the water stands over the bank
    let rim = (river.surface + freeboard(river.width)).max(river.surface.ceil() - 0.4);
    let t = ((river.distance - half) / bank_width(river)).clamp(0.0, 1.0);
    // eased out, so the bank climbs from the water at once instead of lying as a dry shelf beside it,
    // but held over the water within `HOLD` of its edge, as still water's shore is: on a slope across
    // the river the bank would otherwise fall away under the water at once
    let held = river.surface + (river.distance - half).min(HOLD as f32);
    (rim + (ground - rim) * t * (2.0 - t)).max(held)
}

// Where the bed of the river under it is, a parabola across it.
fn bed_height(river: RiverHit) -> f32 {
    let t = (river.distance / (river.width / 2.0)).min(1.0);
    river.bed + (river.surface - river.bed) * t * t
}

fn bank_width(river: RiverHit) -> f32 {
    2.0 + river.width * 0.25
}

// The sea or a lake, standing level in its basin.
#[derive(Debug, Copy, Clone)]
struct Still {
    level: f64,
    kind: WaterKind,
    // how much a point lies within a cell of this water's cells, 0..1, smooth across cells: 1 wherever
    // the water can stand, falling to nothing a cell past that, so what it does to the ground fades out
    near: f64,
    // how much of the four cells round a point are this water's, 0..1, smooth across cells
    cover: f64,
}

impl Still {
    // Whether a point whose smooth ground is `base` is under this water: under its level, or up to
    // `DROWN` over it where all four cells round it are the water's, which no rim can be among.
    fn under(&self, base: f64) -> bool {
        base < self.level + DROWN * smoothstep(0.5, 1.0, self.cover)
    }

    fn is(&self, other: &Still) -> bool {
        self.level == other.level && self.kind == other.kind
    }
}

// `ground` held over still water `w` where the map's smooth ground `base` is over its level, by as much
// as `near` the water the point is, see `contain`.
fn hold_over(ground: f64, base: f64, w: Still) -> f64 {
    if base < w.level {
        return ground;
    }
    let held = contain(ground, base, w.level, false);
    ground + (held - ground) * w.near
}

// `ground` held to still water at `level`, by which side of it the map's smooth ground `base` is on:
// `under` it, the ground stays under the water by as much as the smooth ground is, up to `HOLD`, and over
// it, over the water by as much. So the detail laid over the ground never crosses the water's edge, which
// is where the smooth ground meets the level: no island pokes through and no dry pit sinks under it.
fn contain(ground: f64, base: f64, level: f64, under: bool) -> f64 {
    let t = (base - level).abs().min(HOLD);
    if under {
        // at least half a block under, so it rounds to the block under the water's
        ground.min(level - t.max(0.55))
    } else {
        ground.max(level + t)
    }
}

/// How far from its line a river `width` blocks across shapes the ground: its water, banks and valley.
pub(super) fn river_reach(width: f32) -> f32 {
    width * 0.75 + 2.0 + VALLEY_WIDTH as f32
}

// The most `meander` moves a point, in blocks.
const MEANDER_REACH: f32 = 34.0 + 9.0;

// How far a river's line is moved off its straight run between cell centres at a point: slow
// bends a few hundred blocks long, with smaller wiggles on them.
fn meander(seed: u32, x: f64, z: f64) -> (f64, f64) {
    let bend = |salt: u32| {
        perlin(seed ^ salt, x / 260.0, z / 260.0) * 34.0
            + perlin(seed ^ salt ^ 0x55, x / 80.0, z / 80.0) * 9.0
    };
    (bend(0x3EA1), bend(0x3EA2))
}

/// Terrain read from a `WorldMap`, with detail laid over its ground.
pub struct WorldTerrain {
    map: Arc<WorldMap>,
    biomes: Biomes,
    // the biome each `BiomeKind` is, indexed by kind
    kinds: Vec<BiomeId>,
    // the way each river cell's water leaves it, see `WorldMap::river_tangents`
    tangents: Vec<[f32; 2]>,
    // each river stretch's water level every few blocks along it, under the ground it runs over
    profiles: RiverProfiles,
    // by map cell, where a river rises in it: the spring's middle and level
    springs: Vec<Option<(f64, f64, f32)>>,
}

impl WorldTerrain {
    pub fn new(map: Arc<WorldMap>) -> Self {
        let biomes = Biomes::new(map.seed);
        let kinds = BiomeKind::ALL
            .iter()
            .map(|k| {
                k.name().map_or(BiomeId::BASE, |name| {
                    biomes.id(name).expect("biome in table")
                })
            })
            .collect();
        let tangents = map.river_tangents();
        let mut terrain = Self {
            map,
            biomes,
            kinds,
            tangents,
            profiles: RiverProfiles::default(),
            springs: Vec::new(),
        };
        terrain.profiles = terrain.river_profiles();
        // looked up from the point before it's meandered, as well as after
        terrain.profiles.index(&terrain.map, MEANDER_REACH as f64);
        terrain.springs = terrain.springs();
        terrain
    }

    // Where each river rises: a river cell no river drains into and no lake spills into.
    fn springs(&self) -> Vec<Option<(f64, f64, f32)>> {
        let map = &self.map;
        let dim = map.dim();
        (0..dim * dim)
            .map(|i| {
                if map.river_width[i] <= 0.0 {
                    return None;
                }
                let (cx, cz) = (i % dim, i / dim);
                let fed = map
                    .feeders(cx, cz)
                    .any(|(ux, uz)| map.river_width[map.index(ux, uz)] > 0.0);
                if fed || map.outlet_lake(cx, cz).is_some() {
                    return None;
                }
                let (x, z) = map.cell_centre(cx, cz);
                let surface = map.river_at(&self.profiles, x, z)?.surface;
                Some((x, z, surface))
            })
            .collect()
    }

    // How far the nearest spring is from (`x`, `z`), in the meandered space rivers are laid in, and its level.
    fn spring_near(&self, x: f64, z: f64) -> Option<(f32, f32)> {
        let map = &self.map;
        let (u, v) = map.to_cell(x, z);
        let max = map.dim() as i64 - 1;
        let (u, v) = (u.round() as i64, v.round() as i64);
        (-1..=1)
            .flat_map(|dz| (-1..=1).map(move |dx| (u + dx, v + dz)))
            .filter(|&(cx, cz)| (0..=max).contains(&cx) && (0..=max).contains(&cz))
            .filter_map(|(cx, cz)| self.springs[map.index(cx as usize, cz as usize)])
            .map(|(sx, sz, surface)| {
                (
                    (((x - sx).powi(2) + (z - sz).powi(2)).sqrt()) as f32,
                    surface,
                )
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
    }

    // Each river's water level every few blocks along it, under the map's smooth ground, which is
    // the floor of its valley and graded as rivers are: with the river drawn off its map line by the
    // meander.
    fn river_profiles(&self) -> RiverProfiles {
        let seed = self.map.seed;
        self.map.build_river_profiles(
            &self.tangents,
            &|x, z| self.valley_floor(x, z, self.map.height_at(x, z) as f64),
            &|x, z| {
                let (mx, mz) = meander(seed, x, z);
                let (mx, mz) = meander(seed, x - mx, z - mz);
                (x - mx, z - mz)
            },
            &|x, z| {
                let base = self.map.height_at(x, z) as f64;
                self.still_round(x, z)
                    .into_iter()
                    .flatten()
                    .any(|w| w.under(base))
            },
        )
    }

    // The still water with cells within `reach` cells of a point, `reach` 1 being the four round it: the
    // sea first, then up to three lakes, each at its level, a whole block.
    fn still_near(&self, x: f64, z: f64, reach: i64) -> [Option<Still>; 4] {
        let map = &self.map;
        let (u, v) = map.to_cell(x, z);
        let (cx, cz) = (u.floor() as i64, v.floor() as i64);
        let (fx, fz) = (u - u.floor(), v - v.floor());
        let max = map.dim() as i64 - 1;
        let mut still: [Option<Still>; 4] = [None; 4];
        // which of the four cells round the point are within a cell of each water's cells
        let mut corners = [[false; 4]; 4];
        // and which are each water's own cells
        let mut owns = [[false; 4]; 4];
        for dz in 1 - reach..=reach {
            for dx in 1 - reach..=reach {
                let i = map.index(
                    (cx + dx).clamp(0, max) as usize,
                    (cz + dz).clamp(0, max) as usize,
                );
                let (slot, level, kind) = match WaterCell::from_u8(map.water_kind[i]) {
                    WaterCell::Ocean => (0, WATER_LEVEL as f64, WaterKind::Ocean),
                    WaterCell::Lake => {
                        let level = map.water_surface[i].floor() as f64;
                        let Some(slot) =
                            (1..4).find(|&k| still[k].is_none_or(|w: Still| w.level == level))
                        else {
                            continue;
                        };
                        (slot, level, WaterKind::Lake)
                    }
                    _ => continue,
                };
                still[slot] = Some(Still {
                    level,
                    kind,
                    near: 0.0,
                    cover: 0.0,
                });
                for (c, (a, b)) in [(0, 0), (1, 0), (0, 1), (1, 1)].into_iter().enumerate() {
                    corners[slot][c] |= (dx - a).abs() <= 1 && (dz - b).abs() <= 1;
                    owns[slot][c] |= dx == a && dz == b;
                }
            }
        }
        let blend = |bits: [bool; 4]| {
            let at = |c: usize| if bits[c] { 1.0 } else { 0.0 };
            (at(0) * (1.0 - fx) + at(1) * fx) * (1.0 - fz) + (at(2) * (1.0 - fx) + at(3) * fx) * fz
        };
        for ((w, near), own) in still.iter_mut().zip(corners).zip(owns) {
            if let Some(w) = w {
                w.near = blend(near);
                w.cover = blend(own);
            }
        }
        still
    }

    // The still water a point can be in: that of the four cells round it. Two lakes' cells are never side
    // by side, as one basin's are joined, nor a lake's and the sea's. Every other cell beside a lake's or the
    // sea's is at least as high as its brim, or the water would have spilled there, so the water's edge
    // always falls between the four: where the smooth ground meets its level, never at a cell's side, and
    // never past a rim on ground falling away beyond it.
    fn still_round(&self, x: f64, z: f64) -> [Option<Still>; 4] {
        self.still_near(x, z, 1)
    }

    // The still water whose shore a point may be near: within a cell more each way than `still_round`, so the
    // ground is held and calmed on past the farthest the water reaches.
    fn still_by(&self, x: f64, z: f64) -> [Option<Still>; 4] {
        self.still_near(x, z, 2)
    }

    // The ground at a point with no river or lake water on it: the map's smooth height with the
    // detail laid over it, and terraced, calmed by `calm`, 0..1.
    fn ground_dry(&self, x: f64, z: f64, cell: f64, base: f64, calm: f64) -> f64 {
        let map = &self.map;
        let sea = WATER_LEVEL as f64;
        let smooth = base
            + self.detail_at(x, z, cell, base) * smoothstep(0.0, SHORE_SMOOTH, base - sea) * calm;
        let mountain = self.kind_cover(BiomeKind::Mountain, x, z) * calm;
        if mountain > 0.0 {
            smooth + (terraced(map.seed, x, z, true, smooth, base - sea) - smooth) * mountain
        } else {
            smooth
        }
    }

    // `ground` held to the still water round a point, `base` being the map's smooth ground there, and
    // whether the point is in still water, with the water. `round` and `by` are `still_round` and `still_by` there.
    fn held(
        &self,
        base: f64,
        mut ground: f64,
        round: &[Option<Still>; 4],
        by: &[Option<Still>; 4],
    ) -> (f64, Option<Still>) {
        let mut wet = None;
        for &w in round.iter().flatten() {
            if w.under(base) {
                ground = contain(ground, base.min(w.level), w.level, true);
                wet = Some(w);
            }
        }
        for &w in by.iter().flatten() {
            if !wet.is_some_and(|v: Still| v.is(&w)) {
                ground = hold_over(ground, base, w);
            }
        }
        // inland of the sea the ground never sinks under it, or it would flood as a pond cut off from it
        if wet.is_none() {
            ground = ground.max(WATER_LEVEL as f64 + LAND_ABOVE_SEA);
        }
        (ground, wet)
    }

    // How much of the ground's detail is kept at a point, 0..1: none at still water's edge, `CALM`
    // blocks of smooth ground either side of its level.
    fn calm(base: f64, by: &[Option<Still>; 4]) -> f64 {
        by.iter()
            .flatten()
            .map(|w| 1.0 - w.near * (1.0 - smoothstep(0.0, CALM, (base - w.level).abs())))
            .fold(1.0, f64::min)
    }

    /// The ground at a point with no river or lake water on it.
    pub fn dry_ground(&self, x: f64, z: f64, cell: f64) -> f64 {
        let base = self.map.height_at(x, z) as f64;
        let (round, by) = (self.still_round(x, z), self.still_by(x, z));
        let ground = self.ground_dry(x, z, cell, base, Self::calm(base, &by));
        self.held(base, ground, &round, &by).0
    }

    // How much of the ground round a point is `kind`, 0..1: smooth across the map's cells, and
    // wobbled so a biome's edge wanders off the lines between them.
    fn kind_cover(&self, kind: BiomeKind, x: f64, z: f64) -> f64 {
        let map = &self.map;
        let (u, v) = map.to_cell(x, z);
        let (cx, cz) = (u.floor() as i64, v.floor() as i64);
        let (fx, fz) = (u - u.floor(), v - v.floor());
        let max = map.dim() as i64 - 1;
        let mut cover = 0.0;
        for (dx, dz) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            let i = map.index(
                (cx + dx).clamp(0, max) as usize,
                (cz + dz).clamp(0, max) as usize,
            );
            if map.biome[i] == kind as u8 {
                cover +=
                    (if dx == 0 { 1.0 - fx } else { fx }) * (if dz == 0 { 1.0 - fz } else { fz });
            }
        }
        if cover <= 0.0 || cover >= 1.0 {
            return cover;
        }
        let wobble = perlin(map.seed ^ 0xB107 ^ kind as u32, x / 60.0, z / 60.0) * BIOME_WOBBLE;
        smoothstep(0.25, 0.75, cover + wobble)
    }

    // How far the ground's own detail lifts or sinks it from the map's smooth `base` at a point, before
    // rivers and lakes calm it.
    fn detail_at(&self, x: f64, z: f64, cell: f64, base: f64) -> f64 {
        let map = &self.map;
        let seed = map.seed;
        let sea = WATER_LEVEL as f64;
        let roughness = map.bilinear_bytes(&map.roughness, x, z) as f64 / 255.0;
        let mut detail = detail(seed, x, z, cell, roughness, base - sea);
        // a biome's ground is blended in by how much of it there is, which is smooth across the
        // map's cells: switched on at the biome's edge it would step the ground there
        let desert = self.kind_cover(BiomeKind::Desert, x, z);
        if desert > 0.0 {
            detail += dunes(seed ^ 0xD05E, x, z, cell, base - sea) * desert;
        }
        // the ground round a range is weathered: small scarps stepping up it and gullies and sharp
        // ridges cut into it, as the range's own faces are
        let mountain = self.kind_cover(BiomeKind::Mountain, x, z);
        let near_range = map.bilinear_bytes(&map.range, x, z) as f64 / 255.0;
        let foothills = (1.0 - mountain) * foothill_weight(near_range, base - sea);
        let rough = mountain + foothills;
        detail += scarps(seed, x, z, foothills, base + detail) - (base + detail);
        detail += gully_relief(
            seed,
            x,
            z,
            rough * smoothstep(40.0, 110.0, base - sea),
            cell,
        );
        detail
    }

    // The floor of a river's valley at a point, the map's smooth ground `base` but never under the sea
    // or a lake near it, which would otherwise flood the valley in a wider inlet than the river and
    // leave the river running in under its level.
    fn valley_floor(&self, x: f64, z: f64, base: f64) -> f64 {
        let floor = base.max(WATER_LEVEL as f64 + VALLEY_ABOVE_SEA);
        // a lake within a few dozen blocks counts, as a river's last cells run in toward it
        [
            (0.0, 0.0),
            (40.0, 0.0),
            (-40.0, 0.0),
            (0.0, 40.0),
            (0.0, -40.0),
        ]
        .into_iter()
        .filter_map(|(dx, dz)| self.still_round(x + dx, z + dz)[1])
        .fold(floor, |floor, lake| {
            floor.max(lake.level + VALLEY_ABOVE_SEA)
        })
    }

    // The biome of the cell nearest a point, found a little to one side by a wandering offset so
    // the edges between biomes wander too, rather than following the cells' squares.
    fn biome_at(&self, x: f64, z: f64) -> BiomeId {
        let seed = self.map.seed;
        let wander = self.map.config.map_cell as f64 * 0.6;
        let jx = perlin(seed ^ 0x51A7, x / 40.0, z / 40.0) * wander;
        let jz = perlin(seed ^ 0xB10E, x / 40.0, z / 40.0) * wander;
        let (u, v) = self.map.to_cell(x + jx, z + jz);
        let max = (self.map.dim() - 1) as f64;
        let i = self.map.index(
            u.round().clamp(0.0, max) as usize,
            v.round().clamp(0.0, max) as usize,
        );
        self.kinds[self.map.biome[i] as usize]
    }

    #[cfg(test)]
    pub(super) fn rivers(&self) -> (&[[f32; 2]], &RiverProfiles) {
        (&self.tangents, &self.profiles)
    }

    pub fn map(&self) -> &Arc<WorldMap> {
        &self.map
    }
}

impl TerrainSource for WorldTerrain {
    fn seed(&self) -> u32 {
        self.map.seed
    }

    fn biomes(&self) -> &Biomes {
        &self.biomes
    }

    fn sample_for_cell(&self, x: f64, z: f64, cell: f64) -> TerrainSample {
        let map = &self.map;
        let seed = map.seed;
        let biome = self.biome_at(x, z);
        let base = map.height_at(x, z) as f64;
        // where rivers are laid, drawn off the map's lines by the meander, if any reach this cell
        let (u, v) = map.to_cell(x, z);
        let max = (map.dim() - 1) as f64;
        let here = map.index(
            u.round().clamp(0.0, max) as usize,
            v.round().clamp(0.0, max) as usize,
        );
        let meandered =
            (cell <= RIVER_DETAIL_CELL && !self.profiles.near(here).is_empty()).then(|| {
                let (mx, mz) = meander(seed, x, z);
                (x + mx, z + mz)
            });
        // the nearest river and, where one joins another, the other too
        let rivers = meandered.map_or([None, None], |(mx, mz)| {
            map.river_hits(&self.profiles, mx, mz)
        });
        let spring = meandered
            .and_then(|(mx, mz)| self.spring_near(mx, mz))
            .filter(|&(distance, _)| distance < SPRING_POOL + SPRING_RIM + SPRING_MOUND);
        let spring_part = match spring {
            Some((distance, _)) if distance < SPRING_POOL => SpringPart::Pool,
            Some((distance, _)) if distance < SPRING_POOL + SPRING_RIM => SpringPart::Rim,
            _ => SpringPart::None,
        };
        let (still, by) = (self.still_round(x, z), self.still_by(x, z));
        let mut ground = self.ground_dry(x, z, cell, base, Self::calm(base, &by));
        // a river runs in a valley with a graded floor, the map's smooth ground, which it eases out
        // of into the land's detail and terraces across the valley's sides: so its water, held under
        // that floor, never needs a bank built up, whatever the ground's steps and hollows. Not under
        // still water, whose bed it would raise.
        let valley = rivers
            .iter()
            .flatten()
            .map(|r| {
                let edge = r.width as f64 / 2.0 + bank_width(*r) as f64;
                1.0 - smoothstep(edge, edge + VALLEY_WIDTH, r.distance as f64)
            })
            .fold(0.0, f64::max);
        let under_still = still.into_iter().flatten().any(|w| w.under(base));
        if !under_still && valley > 0.0 {
            ground += (self.valley_floor(x, z, base) - ground) * valley;
        }
        let (held, wet) = self.held(base, ground, &still, &by);
        ground = held;
        // a river is left out where it's in still water it runs into or out of, or that stands over it:
        // the still water's shore is the water's edge there. One over the still water it passes is cut
        // and banked as ever, its banks keeping the two apart.
        let shaped = |r: &RiverHit| wet.is_none_or(|w| r.surface as f64 > w.level + JOIN);
        // near still water it's joined to, a river builds no banks: the two meet at one level, and the
        // still water's shore holds both, where banks would stand between them as a strip of land
        let joined_by = |r: &RiverHit| {
            by.iter()
                .flatten()
                .any(|w| (r.surface as f64 - w.level).abs() <= JOIN)
        };
        if let Some(river) = rivers[0].filter(shaped) {
            ground = if joined_by(&river) {
                if river.distance < river.width / 2.0 {
                    ground.min(bed_height(river) as f64)
                } else {
                    ground
                }
            } else {
                channel(ground as f32, river) as f64
            };
        }
        // a river the nearest one meets still has its bed cut where its water is, so a stream is
        // not cut off short of the bank of the river it runs into, but not where it lies well under the
        // nearest's water, which its banks hold there, as another stretch of a river curving back by it does
        if let Some(joining) = rivers[1]
            .filter(|r| r.distance < r.width / 2.0)
            .filter(shaped)
        {
            let held_off = rivers[0].filter(shaped).is_some_and(|main| {
                (joining.surface + JOIN as f32) < main.surface
                    && main.distance - main.width / 2.0 < HOLD as f32
            });
            if !held_off {
                ground = ground.min(bed_height(joining) as f64);
            }
        }
        // and the other river's bank holds too, unless the two are joined: where one stretch of a river
        // bends past another, the nearer's banks would cut the farther's from under its water
        if let Some(other) = rivers[1].filter(shaped) {
            let out = other.distance - other.width / 2.0;
            // and only on dry ground: inside the nearest's water, as where a stream runs into a river,
            // the other's banks would stand up through it as a strip of land
            let joined_or_under = rivers[0].is_some_and(|main| {
                (main.surface - other.surface).abs() <= JOIN as f32
                    || main.distance < main.width / 2.0
            });
            if !joined_or_under && (0.0..HOLD as f32).contains(&out) {
                ground = ground.max((other.surface + out) as f64);
            }
        }
        // and no river's banks sink still water's shore under its level beside it, but in the river's own water
        if !rivers
            .iter()
            .flatten()
            .filter(|r| shaped(r))
            .any(|r| r.distance < r.width / 2.0)
        {
            for &w in by.iter().flatten() {
                if !wet.is_some_and(|v: Still| v.is(&w)) {
                    ground = hold_over(ground, base, w);
                }
            }
        }
        // a river rises in a bowl, held in a rim of ground over its level, but where the river runs out
        if let Some((distance, surface)) = spring {
            if spring_part == SpringPart::Pool {
                let t = (distance / SPRING_POOL) as f64;
                ground = ground.min(
                    surface as f64
                        - (SPRING_DEPTH.0 + (SPRING_DEPTH.1 - SPRING_DEPTH.0) * (1.0 - t * t)),
                );
            } else if !rivers[0].is_some_and(|r| r.distance < r.width / 2.0 + 1.0) {
                let out = (distance - SPRING_POOL - SPRING_RIM).max(0.0) as f64;
                ground = ground.max(surface.ceil() as f64 + 0.2 - out);
            }
        }
        let ground = ground.max(0.0);

        // where two meet, the wider river's level is the water's
        let river_water = rivers
            .iter()
            .flatten()
            .filter(|r| shaped(r))
            .filter(|r| r.distance < r.width / 2.0 && ground < r.surface as f64)
            .max_by(|a, b| a.width.total_cmp(&b.width));
        // a river by still water it runs into never stands over the still water's whole block by much,
        // or it floods the shore beside it over that level
        let by_still = |surface: f32| {
            by.iter()
                .flatten()
                .find(|w| (w.level..w.level + 2.0).contains(&(surface as f64)))
                .map_or(surface, |w| w.level as f32)
        };
        let (water_surface, water_kind) =
            if let Some((_, surface)) = spring.filter(|_| spring_part == SpringPart::Pool) {
                (surface, WaterKind::River)
            } else if let Some(river) = river_water {
                (by_still(river.surface), WaterKind::River)
            } else if let Some(w) = wet.filter(|w| ground < w.level) {
                (w.level as f32, w.kind)
            } else {
                (0.0, WaterKind::None)
            };
        let wet = water_surface > 0.0;
        TerrainSample {
            continentalness: map.bilinear(&map.continentalness, x, z) as f64,
            height: ground,
            biome,
            // nothing grows in water
            trees: if wet || spring_part != SpringPart::None {
                0.0
            } else {
                map.bilinear_bytes(&map.trees, x, z) as f64 / 255.0
            },
            water_surface,
            water_kind,
            spring: spring_part,
        }
    }

    // How warm it is at the ground here, which falls with height at the rate snow lines climb at,
    // so trees and grass higher up are cooler, and pines grow on the slopes of a range.
    fn climate(&self, x: f64, z: f64) -> Climate {
        let above_sea = (self.map.height_at(x, z) - WATER_LEVEL as f32).max(0.0);
        Climate {
            temperature: (self.temperature(x, z) - above_sea * LAPSE_PER_BLOCK).clamp(0.0, 1.0),
            humidity: self.map.bilinear(&self.map.moisture, x, z).clamp(0.0, 1.0),
        }
    }

    fn temperature(&self, x: f64, z: f64) -> f32 {
        self.map
            .bilinear(&self.map.temperature, x, z)
            .clamp(0.0, 1.0)
    }

    fn tree_bound(&self, x: f64, z: f64) -> f64 {
        self.map.bilinear_bytes(&self.map.trees, x, z) as f64 / 255.0
    }

    // The spawn region nearest the world's middle.
    // The spawn region nearest the world's middle.
    fn find_spawn(&self) -> (i64, i64, f64) {
        let nearest = self
            .map
            .features
            .iter()
            .filter(|f| f.kind == FeatureKind::Spawn)
            .min_by_key(|f| (f.x as i64).abs() + (f.z as i64).abs());
        let (x, z) = nearest.map_or((0, 0), |f| (f.x as i64, f.z as i64));
        (x, z, self.height(x as f64, z as f64))
    }
}
