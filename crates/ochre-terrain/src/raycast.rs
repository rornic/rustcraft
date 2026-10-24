use bevy::math::{I64Vec3, Vec3};

/// A block a ray stopped at.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RayHit {
    pub block: I64Vec3,
    /// The outward normal of the face the ray came in through, zero if it started inside.
    pub face: I64Vec3,
    /// How far along the ray it came in, in blocks.
    pub distance: f32,
}

/// The first block along the ray from `origin` towards `direction`, no farther than `reach`,
/// that `hits` stops at, stepping block by block (Amanatides & Woo). Blocks are centred on their
/// coordinates, so block `c` spans `c - 0.5` to `c + 0.5`.
pub fn raycast(
    origin: Vec3,
    direction: Vec3,
    reach: f32,
    hits: impl Fn(I64Vec3) -> bool,
) -> Option<RayHit> {
    let direction = direction.normalize_or_zero();
    if direction == Vec3::ZERO {
        return None;
    }
    // in the space where each block starts at its coordinate
    let start = origin + Vec3::splat(0.5);
    let mut block = start.floor().as_i64vec3();
    let mut step = I64Vec3::ZERO;
    // how far along the ray it next crosses into the block beyond on each axis, and how far
    // apart those crossings are
    let mut next = Vec3::INFINITY;
    let mut apart = Vec3::INFINITY;
    for axis in 0..3 {
        let d = direction[axis];
        let into = start[axis] - start[axis].floor();
        if d > 0.0 {
            step[axis] = 1;
            next[axis] = (1.0 - into) / d;
            apart[axis] = 1.0 / d;
        } else if d < 0.0 {
            step[axis] = -1;
            next[axis] = into / -d;
            apart[axis] = 1.0 / -d;
        }
    }

    let mut face = I64Vec3::ZERO;
    let mut distance = 0.0;
    loop {
        if hits(block) {
            return Some(RayHit {
                block,
                face,
                distance,
            });
        }
        let axis = if next.x <= next.y && next.x <= next.z {
            0
        } else if next.y <= next.z {
            1
        } else {
            2
        };
        distance = next[axis];
        if distance > reach {
            return None;
        }
        block[axis] += step[axis];
        face = I64Vec3::ZERO;
        face[axis] = -step[axis];
        next[axis] += apart[axis];
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use bevy::math::{I64Vec3, Vec3};

    use super::raycast;

    #[test]
    fn test_hits_the_block_ahead_through_its_near_face() {
        let target = I64Vec3::new(3, 0, 0);
        let hit = raycast(Vec3::ZERO, Vec3::X, 7.0, |b| b == target).unwrap();
        assert_eq!(target, hit.block);
        assert_eq!(I64Vec3::NEG_X, hit.face);
        // its face is half a block short of its centre
        assert!((hit.distance - 2.5).abs() < 1e-5);
    }

    #[test]
    fn test_misses_blocks_past_its_reach() {
        let target = I64Vec3::new(0, -8, 0);
        assert_eq!(None, raycast(Vec3::ZERO, Vec3::NEG_Y, 7.0, |b| b == target));
        // the face is 7.5 away, so it takes a reach of 7.5 to touch it
        assert!(raycast(Vec3::ZERO, Vec3::NEG_Y, 7.5, |b| b == target).is_some());
    }

    #[test]
    fn test_starting_inside_a_block_hits_it_with_no_face() {
        let hit = raycast(Vec3::new(0.4, 0.0, -0.4), Vec3::Z, 7.0, |_| true).unwrap();
        assert_eq!(I64Vec3::ZERO, hit.block);
        assert_eq!(I64Vec3::ZERO, hit.face);
        assert_eq!(0.0, hit.distance);
    }

    #[test]
    fn test_steps_through_every_block_a_diagonal_passes_through() {
        let visited = RefCell::new(Vec::new());
        let origin = Vec3::new(0.2, 0.1, 0.0);
        let direction = Vec3::new(1.0, 0.6, 0.0);
        raycast(origin, direction, 5.0, |b| {
            visited.borrow_mut().push(b);
            false
        });
        let visited = visited.into_inner();
        // one axis at a time, so consecutive blocks always share a face
        for pair in visited.windows(2) {
            let d = pair[1] - pair[0];
            assert_eq!(1, d.x.abs() + d.y.abs() + d.z.abs(), "{pair:?}");
        }
        // and every block it visits is one the line really goes through
        for block in &visited {
            let (min, max) = (block.as_vec3() - 0.5, block.as_vec3() + 0.5);
            let dir = direction.normalize();
            let t_near = ((min - origin) / dir).min((max - origin) / dir);
            let t_far = ((min - origin) / dir).max((max - origin) / dir);
            let (enter, exit) = (t_near.x.max(t_near.y), t_far.x.min(t_far.y));
            assert!(enter <= exit + 1e-5, "{block:?} not on the line");
        }
        assert!(visited.contains(&I64Vec3::new(4, 3, 0)));
    }

    #[test]
    fn test_negative_directions_and_coordinates() {
        let target = I64Vec3::new(-5, 2, -3);
        let origin = Vec3::new(-1.0, 2.0, -1.0);
        let hit = raycast(origin, target.as_vec3() - origin, 7.0, |b| b == target).unwrap();
        assert_eq!(target, hit.block);
        // in through its +x face, the one facing back towards the origin
        assert_eq!(I64Vec3::X, hit.face);
    }

    #[test]
    fn test_no_direction_hits_nothing() {
        assert_eq!(None, raycast(Vec3::ZERO, Vec3::ZERO, 7.0, |_| true));
    }
}
