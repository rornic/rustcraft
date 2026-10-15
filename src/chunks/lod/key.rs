use crate::chunks::chunk::ChunkCoordinate;
use crate::chunks::chunk::CHUNK_SIZE;
use bevy::math::I64Vec3;
use bevy::math::Vec3;

#[derive(Debug, Copy, Clone, Hash, PartialEq, Eq)]
pub struct LodKey {
    pub level: u8,
    pub coord: I64Vec3,
}

impl LodKey {
    pub fn containing(chunk: ChunkCoordinate, level: u8) -> Self {
        Self {
            level,
            coord: chunk.0.div_euclid(I64Vec3::splat(1 << level)),
        }
    }

    /// Width in chunks.
    pub fn width(self) -> i64 {
        1 << self.level
    }

    /// Every node is `CHUNK_SIZE` cells across, so each cell is this many blocks wide.
    pub fn cell_size(self) -> i64 {
        self.width()
    }

    pub fn min_chunk(self) -> I64Vec3 {
        self.coord * self.width()
    }

    pub fn min_block(self) -> I64Vec3 {
        self.min_chunk() * CHUNK_SIZE as i64
    }

    pub fn parent(self) -> Self {
        Self {
            level: self.level + 1,
            coord: self.coord.div_euclid(I64Vec3::splat(2)),
        }
    }

    pub fn children(self) -> [Self; 8] {
        std::array::from_fn(|i| Self {
            level: self.level - 1,
            coord: self.coord * 2
                + I64Vec3::new((i & 1) as i64, ((i >> 1) & 1) as i64, ((i >> 2) & 1) as i64),
        })
    }

    /// This node and each one above it, up to `top`.
    pub fn self_and_ancestors(self, top: u8) -> impl Iterator<Item = LodKey> {
        std::iter::successors(Some(self), move |key| {
            (key.level < top).then(|| key.parent())
        })
    }

    /// Whether the level-1 node `group` lies within this one.
    pub fn contains_group(self, group: LodKey) -> bool {
        match self.level {
            0 => group.coord == self.coord.div_euclid(I64Vec3::splat(2)),
            level => group.coord.div_euclid(I64Vec3::splat(1 << (level - 1))) == self.coord,
        }
    }

    /// The level-1 nodes within this one, or just it if it's level 1.
    pub fn groups_in(self) -> impl Iterator<Item = LodKey> {
        let width = 1i64 << self.level.saturating_sub(1);
        let min = if self.level == 0 {
            self.coord.div_euclid(I64Vec3::splat(2))
        } else {
            self.coord * width
        };
        let count = if self.level == 0 { 1 } else { width };
        (0..count).flat_map(move |x| {
            (0..count).flat_map(move |y| {
                (0..count).map(move |z| LodKey {
                    level: 1,
                    coord: min + I64Vec3::new(x, y, z),
                })
            })
        })
    }

    pub fn chunks(self) -> impl Iterator<Item = ChunkCoordinate> {
        let min = self.min_chunk();
        let width = self.width();
        (0..width).flat_map(move |x| {
            (0..width).flat_map(move |y| {
                (0..width).map(move |z| ChunkCoordinate(min + I64Vec3::new(x, y, z)))
            })
        })
    }

    /// Distance in chunks from the centre of `anchor` to the nearest point of this node.
    pub fn distance(self, anchor: ChunkCoordinate) -> f32 {
        let point = anchor.0.as_vec3() + Vec3::splat(0.5);
        let min = self.min_chunk().as_vec3();
        let max = min + Vec3::splat(self.width() as f32);
        (min - point).max(point - max).max(Vec3::ZERO).length()
    }
}
