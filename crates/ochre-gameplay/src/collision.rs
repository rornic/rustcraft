use bevy::math::{BVec3, I64Vec3, Vec3};

// Overlaps thinner than this don't count, so a box resting on a face, give or take rounding,
// isn't taken to be inside the block under it and fall through.
const EPSILON: f32 = 1e-3;

/// An axis-aligned box, as offsets from the position it's moved by.
#[derive(Debug, Clone, Copy)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

/// Moves `aabb` at `position` by `delta` an axis at a time, up first, stopping each axis
/// against the first block `solid` says it would move into. Returns where it ends up, and
/// which axes were stopped. Blocks it already overlaps are ignored, so it can always get out
/// of them.
pub fn move_and_collide(
    position: Vec3,
    delta: Vec3,
    aabb: Aabb,
    solid: impl Fn(I64Vec3) -> bool,
) -> (Vec3, BVec3) {
    let mut position = position;
    let mut hit = BVec3::FALSE;
    for axis in [1, 0, 2] {
        let (moved, stopped) = sweep_axis(position, aabb, axis, delta[axis], &solid);
        position[axis] += moved;
        hit.set(axis, stopped);
    }
    (position, hit)
}

// How far the box gets along `axis` towards `delta`, and whether a block stopped it.
fn sweep_axis(
    position: Vec3,
    aabb: Aabb,
    axis: usize,
    delta: f32,
    solid: &impl Fn(I64Vec3) -> bool,
) -> (f32, bool) {
    if delta == 0.0 {
        return (0.0, false);
    }
    let min = position + aabb.min;
    let max = position + aabb.max;
    // the cells the box covers across the other two axes
    let [a, b] = [(axis + 1) % 3, (axis + 2) % 3];
    let span = |i: usize| (min[i] + EPSILON).floor() as i64..=(max[i] - EPSILON).floor() as i64;
    let blocked = |cell: i64| {
        span(a).any(|x| {
            span(b).any(|y| {
                let mut coord = I64Vec3::ZERO;
                coord[axis] = cell;
                coord[a] = x;
                coord[b] = y;
                solid(coord)
            })
        })
    };

    if delta > 0.0 {
        // the cells past the leading face, up to where it ends up
        let edge = max[axis];
        let first = (edge - EPSILON).ceil() as i64;
        let last = (edge + delta - EPSILON).ceil() as i64 - 1;
        match (first..=last).find(|&cell| blocked(cell)) {
            Some(cell) => (cell as f32 - edge, true),
            None => (delta, false),
        }
    } else {
        let edge = min[axis];
        let first = (edge + EPSILON).floor() as i64 - 1;
        let last = (edge + delta + EPSILON).floor() as i64;
        match (last..=first).rev().find(|&cell| blocked(cell)) {
            Some(cell) => ((cell + 1) as f32 - edge, true),
            None => (delta, false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOX: Aabb = Aabb {
        min: Vec3::new(-0.3, 0.0, -0.3),
        max: Vec3::new(0.3, 1.8, 0.3),
    };

    // solid at and below y = 0
    fn floor(cell: I64Vec3) -> bool {
        cell.y < 0
    }

    #[test]
    fn test_falls_onto_the_floor_and_stops_on_it() {
        let (position, hit) =
            move_and_collide(Vec3::new(0.5, 3.0, 0.5), -Vec3::Y * 10.0, BOX, floor);
        assert_eq!(position, Vec3::new(0.5, 0.0, 0.5));
        assert!(hit.y && !hit.x && !hit.z);
    }

    #[test]
    fn test_falls_freely_where_nothing_is_solid() {
        let (position, hit) = move_and_collide(Vec3::new(0.5, 3.0, 0.5), -Vec3::Y, BOX, floor);
        assert_eq!(position, Vec3::new(0.5, 2.0, 0.5));
        assert_eq!(hit, BVec3::FALSE);
    }

    #[test]
    fn test_rests_on_the_floor_over_many_small_steps() {
        let mut position = Vec3::new(0.5, 0.0, 0.5);
        for _ in 0..1000 {
            position = move_and_collide(position, Vec3::new(0.001, -0.016, 0.0), BOX, floor).0;
        }
        assert_eq!(position.y, 0.0);
        assert!((position.x - 1.5).abs() < 1e-3);
    }

    #[test]
    fn test_slides_along_a_wall() {
        let wall = |cell: I64Vec3| floor(cell) || cell.x >= 2;
        let (position, hit) = move_and_collide(
            Vec3::new(0.5, 0.0, 0.5),
            Vec3::new(5.0, 0.0, 3.0),
            BOX,
            wall,
        );
        assert!((position.x - 1.7).abs() < 1e-5);
        assert_eq!(position.z, 3.5);
        assert!(hit.x && !hit.z);
    }

    #[test]
    fn test_stops_under_a_ceiling() {
        let ceiling = |cell: I64Vec3| cell.y >= 3;
        let (position, hit) =
            move_and_collide(Vec3::new(0.5, 0.0, 0.5), Vec3::Y * 5.0, BOX, ceiling);
        assert!((position.y - 1.2).abs() < 1e-5);
        assert!(hit.y);
    }

    #[test]
    fn test_moves_out_of_a_block_it_starts_inside() {
        let one_block = |cell: I64Vec3| cell == I64Vec3::new(0, 1, 0);
        let (position, hit) =
            move_and_collide(Vec3::new(0.5, 0.5, 0.5), Vec3::X * 2.0, BOX, one_block);
        assert_eq!(position, Vec3::new(2.5, 0.5, 0.5));
        assert_eq!(hit, BVec3::FALSE);
    }

    #[test]
    fn test_doesnt_tunnel_through_a_thin_wall_in_one_big_step() {
        let wall = |cell: I64Vec3| cell.x == 10;
        let (position, hit) =
            move_and_collide(Vec3::new(0.5, 0.0, 0.5), Vec3::X * 100.0, BOX, wall);
        assert!((position.x - 9.7).abs() < 1e-5);
        assert!(hit.x);
    }

    #[test]
    fn test_fits_through_a_gap_its_own_width() {
        // walls at x = -1 and x = 1, so the one-block gap at x = 0 is open
        let walls = |cell: I64Vec3| cell.x != 0;
        let (position, hit) = move_and_collide(Vec3::new(0.5, 0.0, 0.5), Vec3::Z * 4.0, BOX, walls);
        assert_eq!(position.z, 4.5);
        assert!(!hit.z);
    }

    #[test]
    fn test_box_as_tall_as_a_gap_walks_through_it() {
        let tall = Aabb {
            min: BOX.min,
            max: Vec3::new(0.3, 2.0, 0.3),
        };
        let tunnel = |cell: I64Vec3| cell.y < 0 || cell.y >= 2;
        let (position, hit) =
            move_and_collide(Vec3::new(0.5, 0.0, 0.5), Vec3::Z * 4.0, tall, tunnel);
        assert_eq!(position, Vec3::new(0.5, 0.0, 4.5));
        assert_eq!(hit, BVec3::FALSE);
    }
}
