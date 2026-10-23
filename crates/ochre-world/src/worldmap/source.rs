use std::sync::Arc;

use super::climate::{BiomeKind, LAPSE_PER_BLOCK};
use super::detail::{detail, dunes, foothill_weight, gully_relief, scarps, terraced};
use super::water::{freeboard, RiverHit, RiverProfiles};
use super::FeatureKind;
use super::WorldMap;
use ochre_base::biome::{BiomeId, Biomes, Climate};
use ochre_base::math::smoothstep_f64 as smoothstep;
use ochre_base::noise::perlin;
use ochre_base::terrain_source::{SpringPart, TerrainSample, TerrainSource, WaterKind};
use ochre_base::WATER_LEVEL;

// How much of a point lake cells must cover for any river there to be the lake's, whatever its level.
const DROWNED_COVER: f64 = 0.75;

// How far a biome's cover is wobbled, so its edge wanders off the lines between the cells.
const BIOME_WOBBLE: f64 = 0.15;

// How far a lake's cover is wobbled, so its shore wanders off the lines between its cells.
const LAKE_WOBBLE: f64 = 0.2;

// Land the map puts under the sea but isn't the sea's is this much over it.
const LAND_ABOVE_SEA: f64 = 0.3;

// Over the sea by up to this much the ground's detail is eased out, so a flat shore has no hollows
// the sea would fill as ponds cut off from it.
const SHORE_SMOOTH: f64 = 5.0;

// A river at the sea's level runs on, with banks, through ground this much under it, not further.
const MARSH_DEPTH: f64 = 2.0;

// How far under the map's smooth ground, in blocks, a pit cut in the ground can be and still hold the lake's water.
const LAKE_PIT: f64 = 4.0;

// Blocks either side of a lake's level the ground's detail is calmed to nothing at, easing back from it.
const LAKE_CALM_BAND: f64 = 8.0;

// How far over a lake's level the map's smooth ground may be where its water still stands.
const LAKE_BASE_SLACK: f64 = 0.0;

// How much of a point lake cells cover for a river's valley to give way to the lake's ground there.
const LAKE_VALLEY_COVER: f64 = 0.4;

// A river's valley floor is never lower than this over the sea or a lake: the map's carving of a river's cells
// can sink it under the sea well up the river, and the sea would flood the valley in a wider inlet than
// the river.
const VALLEY_ABOVE_SEA: f64 = 0.6;

// How far past a river's banks, in blocks, its valley floor eases back out into the land's own ground.
const VALLEY_WIDTH: f64 = 40.0;

// A river whose water is within this much under the sea's is the river's all the way to the sea, not
// the sea's wherever its bed dips under the sea's level.
const SEA_MARGIN: f32 = 0.75;

// Within a lake this much covered, ground that rises less than this many blocks over its water is
// under it.
const SPECK_COVER: f64 = 0.8;
const SPECK_HEIGHT: f64 = 1.5;

// A lake's water must stand this far over the sea's level to be a lake's, however low its bed.
const INLAND_LAKE_ABOVE_SEA: f64 = 0.6;

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
    // eased out, so the bank climbs from the water at once instead of lying as a dry shelf beside it
    rim + (ground - rim) * t * (2.0 - t)
}

// Where the bed of the river under it is, a parabola across it.
fn bed_height(river: RiverHit) -> f32 {
    let t = (river.distance / (river.width / 2.0)).min(1.0);
    river.bed + (river.surface - river.bed) * t * t
}

fn bank_width(river: RiverHit) -> f32 {
    2.0 + river.width * 0.25
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

// Which cells have a river or lake within `NEAR` cells, which is as far as a river's banks, a
// meander and a lake's shore reach.
fn near_water(map: &WorldMap) -> Vec<bool> {
    const NEAR: i64 = 4;
    let dim = map.dim() as i64;
    let mut near = vec![false; map.height.len()];
    for cz in 0..dim {
        for cx in 0..dim {
            let i = (cz * dim + cx) as usize;
            if map.river_width[i] <= 0.0 && map.water_kind[i] != 2 {
                continue;
            }
            // a wide river's banks and valley reach further than `NEAR`, or they'd end at the cells' edges
            let reach =
                (river_reach(map.river_width[i]) + MEANDER_REACH) / map.config.map_cell as f32;
            let radius = NEAR.max(reach.ceil() as i64 + 1);
            for z in (cz - radius).max(0)..=(cz + radius).min(dim - 1) {
                for x in (cx - radius).max(0)..=(cx + radius).min(dim - 1) {
                    near[(z * dim + x) as usize] = true;
                }
            }
        }
    }
    near
}

/// Terrain read from a `WorldMap`, with detail laid over its ground.
pub struct WorldTerrain {
    map: Arc<WorldMap>,
    biomes: Biomes,
    // the biome each `BiomeKind` is, indexed by kind
    kinds: Vec<BiomeId>,
    // cells with a river or lake within a few cells of them, so the rest skip looking for water
    near_water: Vec<bool>,
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
        let near_water = near_water(&map);
        let tangents = map.river_tangents();
        let mut terrain = Self {
            map,
            biomes,
            kinds,
            near_water,
            tangents,
            profiles: RiverProfiles::default(),
            springs: Vec::new(),
        };
        terrain.profiles = terrain.river_profiles();
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
                let surface = map.river_at(&self.tangents, &self.profiles, x, z)?.surface;
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
            &|x, z| self.valley_floor(x, z, self.land_base(x, z)),
            &|x, z| {
                let (mx, mz) = meander(seed, x, z);
                let (mx, mz) = meander(seed, x - mx, z - mz);
                (x - mx, z - mz)
            },
            &|x, z| {
                // the sea where the land is under it, a lake where its cover and the ground say
                self.dry_ground(x, z, 1.0) < WATER_LEVEL as f64
                    || self.map.lake_level(x, z).is_some_and(|surface| {
                        self.dry_ground(x, z, 1.0) < surface as f64
                            && self.land_base(x, z) < surface as f64 + LAKE_BASE_SLACK
                    })
            },
        )
    }

    // The map's smooth height at a point, but inland of the sea never under it: the map's carving of a
    // river's cells can sink them under the sea well up the coast, which would leave ponds cut off from
    // it. Where sea cells cover the point the height is the map's own, and it eases to the floor inland.
    fn land_base(&self, x: f64, z: f64) -> f64 {
        let base = self.map.height_at(x, z) as f64;
        let floor = WATER_LEVEL as f64 + LAND_ABOVE_SEA;
        if base >= floor {
            return base;
        }
        // a lake's bed is its own, however low: lifted, a lake under the sea's level would be left shallow
        let lake = self.map.lake_cover(x, z).map_or(0.0, |(_, cover)| cover);
        let sea = smoothstep(0.15, 0.6, self.map.ocean_cover(x, z)).max(lake);
        floor + (base - floor) * sea
    }

    // The ground at a point with no river or lake water on it: the map's smooth height with the
    // detail laid over it, calmed in a lake's shallows, and terraced.
    fn ground_dry(&self, x: f64, z: f64, cell: f64, base: f64, near_water: bool) -> f64 {
        let map = &self.map;
        let sea = WATER_LEVEL as f64;
        let detail = self.detail_at(x, z, cell, base) * smoothstep(0.0, SHORE_SMOOTH, base - sea);
        // lake floors and shores stay as the map has them, or the detail would stripe the water's edge
        let mut kept = 1.0;
        if near_water {
            if let Some((surface, cover)) = map.lake_cover(x, z) {
                // eased in with the lake's cover, which is smooth across cells: all at once at the edge
                // of the cells that hold it would step the ground at their squares' sides
                let calm = smoothstep(0.0, 6.0, base - surface as f64);
                kept *= 1.0 - smoothstep(0.0, 0.5, cover) * (1.0 - calm);
                // and near the lake's level either side of it, so a hollow in the ground there doesn't leave
                // water standing in a part of it, with walls, where the smooth ground crosses the level
                let near = 1.0 - smoothstep(0.0, LAKE_CALM_BAND, (base - surface as f64).abs());
                kept *= 1.0 - smoothstep(0.0, 0.25, cover) * near;
            }
        }
        let smooth = base + detail * kept;
        // terraces are calmed with the detail: one cut more than `LAKE_PIT` into a lake's floor stands dry
        let mountain = self.kind_cover(BiomeKind::Mountain, x, z) * kept;
        let ground = if mountain > 0.0 {
            smooth + (terraced(map.seed, x, z, true, smooth, base - sea) - smooth) * mountain
        } else {
            smooth
        };
        // inland of the sea the detail's hollows never sink under it: a hollow there would flood as a
        // pond of sea cut off from it, and only the ground the sea's cells cover is the sea's
        let lake = if near_water {
            map.lake_cover(x, z).map_or(0.0, |(_, cover)| cover)
        } else {
            0.0
        };
        let inland = (1.0 - smoothstep(0.15, 0.6, map.ocean_cover(x, z))).min(1.0 - lake);
        ground + (sea + LAND_ABOVE_SEA - ground).max(0.0) * inland
    }

    /// The ground at a point with no river or lake water on it.
    pub fn dry_ground(&self, x: f64, z: f64, cell: f64) -> f64 {
        let map = &self.map;
        let (u, v) = map.to_cell(x, z);
        let max = (map.dim() - 1) as f64;
        let near_water = self.near_water[map.index(
            u.round().clamp(0.0, max) as usize,
            v.round().clamp(0.0, max) as usize,
        )];
        self.ground_dry(x, z, cell, self.land_base(x, z), near_water)
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
        let lake = [
            (0.0, 0.0),
            (40.0, 0.0),
            (-40.0, 0.0),
            (0.0, 40.0),
            (0.0, -40.0),
        ]
        .into_iter()
        .filter_map(|(dx, dz)| self.map.lake_cover(x + dx, z + dz))
        .map(|(surface, _)| surface as f64)
        .fold(f64::MIN, f64::max);
        if lake > f64::MIN {
            floor.max(lake + VALLEY_ABOVE_SEA)
        } else {
            floor
        }
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
        let sea = WATER_LEVEL as f64;
        let biome = self.biome_at(x, z);
        let base = self.land_base(x, z);
        let near_water = {
            let (u, v) = map.to_cell(x, z);
            let max = (map.dim() - 1) as f64;
            self.near_water[map.index(
                u.round().clamp(0.0, max) as usize,
                v.round().clamp(0.0, max) as usize,
            )]
        };
        // where rivers are laid, drawn off the map's lines by the meander
        let meandered = (near_water && cell <= RIVER_DETAIL_CELL).then(|| {
            let (mx, mz) = meander(seed, x, z);
            (x + mx, z + mz)
        });
        // the nearest river and, where one joins another, the other too
        let rivers = meandered.map_or([None, None], |(mx, mz)| {
            map.river_hits(&self.tangents, &self.profiles, mx, mz)
        });
        let spring = meandered
            .and_then(|(mx, mz)| self.spring_near(mx, mz))
            .filter(|&(distance, _)| distance < SPRING_POOL + SPRING_RIM + SPRING_MOUND);
        let spring_part = match spring {
            Some((distance, _)) if distance < SPRING_POOL => SpringPart::Pool,
            Some((distance, _)) if distance < SPRING_POOL + SPRING_RIM => SpringPart::Rim,
            _ => SpringPart::None,
        };
        // how much lake cells cover a point, wobbled so a shore is not a straight line of the cells' squares
        // the level of the lake whose cells are by a point, which its water stands at wherever the ground is
        // under it: the shore is where the ground meets the water, so no wobbled mask holds it back
        let lake_core = near_water.then(|| map.lake_level(x, z)).flatten();
        let lake_level = lake_core;
        // water stands over ground under the level by the lake's cells
        // and only where the map's smooth ground is under it too, or a gorge cut deep beside the lake
        // would stand full of its water, a wall at the edge of the lake's cells
        let lake_wet = |g: f64| {
            lake_core.is_some_and(|s| {
                g < s as f64 && base < s as f64 + LAKE_BASE_SLACK && g > base - LAKE_PIT
            })
        };
        let lake = near_water
            .then(|| map.lake_cover(x, z))
            .flatten()
            .map(|(surface, cover)| {
                let wobble = perlin(seed ^ 0x1A4E, x / 45.0, z / 45.0) * LAKE_WOBBLE;
                (surface, (cover + wobble).clamp(0.0, 1.0))
            });
        // a lake's shore is a bank of ground held at its level, except where a river runs in: the water
        // of the two meets there, with no strip of bank between them
        let by_mouth = rivers
            .iter()
            .flatten()
            .any(|r| r.distance < r.width / 2.0 + bank_width(*r));
        // a river running through a lake at its level is part of the lake: its channel isn't
        // cut into the lake's bed
        let rivers = match lake {
            Some((surface, cover))
                if cover >= DROWNED_COVER
                    || cover >= 0.5 && rivers[0].is_some_and(|r| r.surface >= surface - 1.0) =>
            {
                [None, None]
            }
            _ => rivers,
        };
        // the ground as it would be with no river, which a river's water is already under: its
        // levels are worked out from this, see `river_profiles`
        let mut ground = self.ground_dry(x, z, cell, base, near_water);
        // a river runs in a valley with a graded floor, the map's smooth ground, which it eases out
        // of into the land's detail and terraces across the valley's sides: so its water, held under
        // that floor, never needs a bank built up, whatever the ground's steps and hollows
        let valley = rivers
            .iter()
            .flatten()
            .map(|r| {
                let edge = r.width as f64 / 2.0 + bank_width(*r) as f64;
                1.0 - smoothstep(edge, edge + VALLEY_WIDTH, r.distance as f64)
            })
            .fold(0.0, f64::max);
        // the sea is the sea's wherever the land is really under it, but a river at the sea's level runs
        // on through shallow ground by it in a channel with banks, not into a wider inlet of the sea
        // a lake's bed may lie far under the sea's level while its water stands well over it: that
        // water is the lake's, not the sea's, wherever its bed is
        let inland_lake =
            lake_level.is_some_and(|surface| surface as f64 > sea + INLAND_LAKE_ABOVE_SEA);
        let under_sea = ground < sea
            && !inland_lake
            && !(ground >= sea - MARSH_DEPTH
                && rivers.iter().flatten().any(|r| {
                    r.distance < r.width / 2.0 + bank_width(*r)
                        && r.surface >= WATER_LEVEL as f32 - SEA_MARGIN
                }));
        // a lake covers ground under its level, so a river's valley is not built up there: raised to the
        // lake's level and over, it would stand between the river and the lake's water
        let lake_cover = lake.map_or(0.0, |(_, cover)| cover);
        let valley = valley * (1.0 - smoothstep(0.0, LAKE_VALLEY_COVER, lake_cover));
        if !under_sea && valley > 0.0 {
            ground += (self.valley_floor(x, z, base) - ground) * valley;
        }
        // ground the sea or a lake covers is theirs, and the river enters their water cleanly: its
        // banks aren't built up inside it
        // a river's channel and banks are its own: where one runs out of a lake, or by one, lower than
        // it, the lake's water stays out of the channel the river is cut through its rim, and the
        // river's level holds
        let by_river = lake_level.is_some_and(|surface| {
            rivers
                .iter()
                .flatten()
                .any(|r| r.distance < r.width / 2.0 + bank_width(*r) && r.surface < surface - 1.0)
        });
        let flooded = under_sea || (!by_river && lake_wet(ground));
        let rivers = if flooded { [None, None] } else { rivers };
        if let Some(river) = rivers[0] {
            ground = channel(ground as f32, river) as f64;
        }
        // a river the nearest one meets still has its bed cut where its water is, so a stream is
        // not cut off short of the bank of the river it runs into
        if let Some(joining) = rivers[1].filter(|r| r.distance < r.width / 2.0) {
            ground = ground.min(bed_height(joining) as f64);
        }
        // a lake's shore is where the ground meets its water, wherever that falls: the ground is not
        // built up to the water, or cut down to it
        if let Some((surface, cover)) = lake.filter(|_| !by_river && !by_mouth) {
            // a bump that only just clears the water out in the lake would be a lone block standing
            // in it: it goes under
            let surface = surface as f64;
            if cover >= SPECK_COVER && (surface..surface + SPECK_HEIGHT).contains(&ground) {
                ground = surface - 0.6;
            }
        }
        // dry ground at a lake's edge but under its level, a dip its water doesn't flood, is built up flush with the
        // water, or the lake's edge stands as a wall of water over it
        let shore = near_water && !flooded && !by_river && !by_mouth;
        if let Some((surface, out)) = shore.then(|| map.lake_level_near(x, z)).flatten() {
            // and past where its water can reach, eased down a block a block, so the ground doesn't end in a line
            ground = ground.max(surface.floor() as f64 + 0.1 - out);
        }
        // the lake's water is a whole block, floored: ground under the lake's real level but over that
        // block is dug just under it, or it would stand as a strip of dry shore between the water and a
        // river running into it
        if let Some(surface) = lake_level.filter(|_| !by_river && lake_wet(ground)) {
            ground = ground.min(surface.floor() as f64 - 0.01);
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

        // a river's water by a lake never stands over the lake's whole block by more than half a block, or
        // a river just over the lake's level floods its shore at a level over the lake's
        let by_lake = |surface: f32| match lake_level {
            Some(level) if surface > level.floor() && surface < level + 2.0 => level.floor(),
            _ => surface,
        };
        // a river held above the sea in a trench cut below it is still the river's water there
        let river_water = rivers
            .iter()
            .flatten()
            .filter(|r| r.distance < r.width / 2.0 && ground < r.surface as f64)
            // where two meet, the wider river's level is the water's
            .max_by(|a, b| a.width.total_cmp(&b.width));
        let (water_surface, water_kind) =
            if let Some((_, surface)) = spring.filter(|_| spring_part == SpringPart::Pool) {
                (surface, WaterKind::River)
            } else if let Some(river) =
                river_water.filter(|r| r.surface >= WATER_LEVEL as f32 - SEA_MARGIN)
            {
                (by_lake(river.surface), WaterKind::River)
            } else if ground < sea && !inland_lake {
                (WATER_LEVEL as f32, WaterKind::Ocean)
            } else if let Some(river) = river_water {
                (by_lake(river.surface), WaterKind::River)
            } else if let Some(surface) = lake_level.filter(|_| !by_river && lake_wet(ground)) {
                // a whole block, never over the surface: rounded up, the water stands a block over a shore at its level
                (surface.floor(), WaterKind::Lake)
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
