use bevy::math::I64Vec2;

use super::erosion::{erode, ErosionParams, HeightGrid};
use super::noise::NoiseGenerator;

// Large enough that a droplet's full lifetime fits comfortably inside one region's
// interior away from the border most of the time; small enough that one region's
// full droplet pass is boundable compute rather than minutes.
pub const REGION_SIZE: i64 = 256;
// Extra margin sampled and eroded around a region before cropping back to
// REGION_SIZE - comfortably exceeds a droplet's expected travel distance, so a
// droplet that wanders out of the inner area still erodes against real terrain
// rather than a clamped edge. Erosion is path-dependent, so neighbouring regions
// still won't agree exactly at their shared boundary - this narrows that seam, it
// doesn't eliminate it.
pub const REGION_HALO: i64 = 64;

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

    fn sample_origin(&self) -> I64Vec2 {
        self.origin() - I64Vec2::splat(REGION_HALO)
    }
}

// The authoritative product of one region's compute: a cropped (halo removed)
// height array, REGION_SIZE x REGION_SIZE, row-major [z][x]. `discharge` is reserved
// for a future flow-accumulation/rivers pass - left all-zero for now so callers
// don't need to change once that lands.
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

// Synchronous and CPU-heavy (runs thousands of droplet steps over a halo-padded
// grid) - always called from inside an async task-pool task, never a Bevy system
// body on the main thread. Caching/coalescing concurrent calls is RegionStore's job.
pub fn compute_region(
    noise: &NoiseGenerator,
    seed: u32,
    coord: RegionCoord,
    world_height: u64,
    erosion_params: &ErosionParams,
) -> Region {
    let full_size = (REGION_SIZE + 2 * REGION_HALO) as u32;
    let mut grid = HeightGrid::from_noise(noise, coord.sample_origin(), full_size, full_size, world_height);
    erode(&mut grid, seed, coord.sample_origin(), erosion_params);
    let heights = crop_to_interior(&grid, REGION_HALO as u32, REGION_SIZE as u32);
    Region {
        heights,
        discharge: vec![0.0; (REGION_SIZE * REGION_SIZE) as usize],
    }
}

fn crop_to_interior(grid: &HeightGrid, halo: u32, size: u32) -> Vec<f32> {
    let mut out = Vec::with_capacity((size * size) as usize);
    for z in 0..size {
        for x in 0..size {
            out.push(grid.get((halo + x) as i32, (halo + z) as i32));
        }
    }
    out
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
        let region = compute_region(
            &noise,
            1,
            RegionCoord::from_world(0, 0),
            256,
            &ErosionParams { droplet_count: 100, ..ErosionParams::default() },
        );
        assert_eq!((REGION_SIZE * REGION_SIZE) as usize, region.heights.len());
    }
}
