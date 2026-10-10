use bevy::math::I64Vec2;

use super::noise::NoiseGenerator;

pub const REGION_SIZE: i64 = 256;

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

// One region's column heights, REGION_SIZE x REGION_SIZE, row-major [z][x].
// `discharge` is reserved for a future flow-accumulation/rivers pass - left all-zero
// for now so callers don't need to change once that lands.
pub struct Region {
    pub heights: Vec<f32>,
    pub discharge: Vec<f32>,
}

impl Region {
    fn index(local_x: i64, local_z: i64) -> usize {
        (local_z * REGION_SIZE + local_x) as usize
    }

    pub fn height_at(&self, local_x: i64, local_z: i64) -> f32 {
        self.heights[Self::index(local_x, local_z)]
    }

    pub fn discharge_at(&self, local_x: i64, local_z: i64) -> f32 {
        self.discharge[Self::index(local_x, local_z)]
    }
}

// Samples REGION_SIZE^2 columns of noise - run from an async task-pool task, never
// a Bevy system body on the main thread. Caching/coalescing concurrent calls is
// RegionStore's job.
pub fn compute_region(noise: &NoiseGenerator, coord: RegionCoord, world_height: u64) -> Region {
    let origin = coord.origin();
    let mut heights = Vec::with_capacity((REGION_SIZE * REGION_SIZE) as usize);
    for z in 0..REGION_SIZE {
        for x in 0..REGION_SIZE {
            let height = noise.get(I64Vec2::new(origin.x + x, origin.y + z));
            heights.push(height.min(world_height as f64) as f32);
        }
    }
    Region {
        heights,
        discharge: vec![0.0; (REGION_SIZE * REGION_SIZE) as usize],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_region_coord_from_world_handles_negative_coordinates() {
        assert_eq!(RegionCoord(I64Vec2::new(-1, -1)), RegionCoord::from_world(-1, -1));
        assert_eq!(RegionCoord(I64Vec2::new(-1, 0)), RegionCoord::from_world(-REGION_SIZE, 0));
        assert_eq!(RegionCoord(I64Vec2::new(0, 0)), RegionCoord::from_world(0, REGION_SIZE - 1));
    }

    #[test]
    fn test_compute_region_produces_region_sized_height_array() {
        let noise = NoiseGenerator::new(1);
        let region = compute_region(&noise, RegionCoord::from_world(0, 0), 256);
        assert_eq!((REGION_SIZE * REGION_SIZE) as usize, region.heights.len());
    }
}
