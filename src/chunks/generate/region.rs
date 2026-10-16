use bevy::math::I64Vec2;

use super::biome::BiomeId;
use super::noise::NoiseGenerator;

pub const REGION_SIZE: i64 = 256;
// Regions keep the lowest and highest column in each tile this many blocks across.
const TILE_SIZE: i64 = 16;
const TILES: i64 = REGION_SIZE / TILE_SIZE;

#[derive(Debug, Copy, Clone, Hash, PartialEq, Eq)]
pub struct RegionCoord(pub I64Vec2);

impl RegionCoord {
    pub fn from_world(world_x: i64, world_z: i64) -> Self {
        Self(I64Vec2::new(
            world_x.div_euclid(REGION_SIZE),
            world_z.div_euclid(REGION_SIZE),
        ))
    }

    pub(crate) fn origin(&self) -> I64Vec2 {
        I64Vec2::new(self.0.x * REGION_SIZE, self.0.y * REGION_SIZE)
    }
}

// One region's column heights and biomes, REGION_SIZE x REGION_SIZE, row-major [z][x].
pub struct Region {
    heights: Vec<f32>,
    biomes: Vec<BiomeId>,
    // tree density, 0..1 as 0..=255
    trees: Vec<u8>,
    // the lowest and highest column in each TILE_SIZE tile, TILES x TILES, row-major [z][x]
    tile_range: Vec<(f32, f32)>,
}

impl Region {
    fn index(local_x: i64, local_z: i64) -> usize {
        (local_z * REGION_SIZE + local_x) as usize
    }

    pub fn height_at(&self, local_x: i64, local_z: i64) -> f32 {
        self.heights[Self::index(local_x, local_z)]
    }

    pub fn biome_at(&self, local_x: i64, local_z: i64) -> BiomeId {
        self.biomes[Self::index(local_x, local_z)]
    }

    pub fn trees_at(&self, local_x: i64, local_z: i64) -> f64 {
        self.trees[Self::index(local_x, local_z)] as f64 / 255.0
    }

    /// The lowest and highest columns in `min..=max` (local x, z), rounded out to whole tiles.
    pub fn height_range(&self, min: I64Vec2, max: I64Vec2) -> (f32, f32) {
        let (min, max) = (min / TILE_SIZE, max / TILE_SIZE);
        (min.y..=max.y)
            .flat_map(|z| (min.x..=max.x).map(move |x| (z * TILES + x) as usize))
            .map(|i| self.tile_range[i])
            .fold((f32::MAX, f32::MIN), |(lo, hi), (a, b)| {
                (lo.min(a), hi.max(b))
            })
    }
}

// Samples REGION_SIZE^2 columns of noise, so only call from a task.
pub fn compute_region(noise: &NoiseGenerator, coord: RegionCoord) -> Region {
    let origin = coord.origin();
    let mut heights = Vec::with_capacity((REGION_SIZE * REGION_SIZE) as usize);
    let mut biomes = Vec::with_capacity((REGION_SIZE * REGION_SIZE) as usize);
    let mut trees = Vec::with_capacity((REGION_SIZE * REGION_SIZE) as usize);
    let mut tile_range = vec![(f32::MAX, f32::MIN); (TILES * TILES) as usize];
    for z in 0..REGION_SIZE {
        for x in 0..REGION_SIZE {
            let sample = noise.sample((origin.x + x) as f64, (origin.y + z) as f64);
            let height = sample.height as f32;
            heights.push(height);
            biomes.push(sample.biome);
            trees.push((sample.trees * 255.0).round() as u8);
            let tile = &mut tile_range[((z / TILE_SIZE) * TILES + x / TILE_SIZE) as usize];
            *tile = (tile.0.min(height), tile.1.max(height));
        }
    }
    Region {
        heights,
        biomes,
        trees,
        tile_range,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_region_coord_from_world_handles_negative_coordinates() {
        assert_eq!(
            RegionCoord(I64Vec2::new(-1, -1)),
            RegionCoord::from_world(-1, -1)
        );
        assert_eq!(
            RegionCoord(I64Vec2::new(-1, 0)),
            RegionCoord::from_world(-REGION_SIZE, 0)
        );
        assert_eq!(
            RegionCoord(I64Vec2::new(0, 0)),
            RegionCoord::from_world(0, REGION_SIZE - 1)
        );
    }

    #[test]
    fn test_height_range_covers_every_column_in_range() {
        let region = &compute_region(&NoiseGenerator::new(1), RegionCoord::from_world(0, 0));
        for (min, max) in [(0, 0), (5, 40), (17, 31), (0, REGION_SIZE - 1)] {
            let (min, max) = (I64Vec2::splat(min), I64Vec2::splat(max));
            let (lowest, highest) = region.height_range(min, max);
            for z in min.y..=max.y {
                for x in min.x..=max.x {
                    assert!((lowest..=highest).contains(&region.height_at(x, z)));
                }
            }
        }
    }
}
