use std::collections::VecDeque;

use super::generator::FACE_NORMAL_OFFSET;

use bevy::math::IVec3;

pub use ochre_base::MAX_LIGHT;

// Sky light over a box of cells, worked out from how much light each lets through.
pub struct SkyLight {
    min: IVec3,
    size: IVec3,
    levels: Vec<u8>,
}

impl SkyLight {
    /// Lights the box of `size` cells from `min`, whose `light_opacity`s are in `opacity`
    /// in `SkyLight::index` order, with `sky` falling into the top of every column.
    pub fn new(min: IVec3, size: IVec3, sky: u8, opacity: Vec<u8>) -> Self {
        assert_eq!((size.x * size.y * size.z) as usize, opacity.len());
        let mut levels = vec![0; opacity.len()];
        // whether anything light can pass through is shaded, so light has somewhere to spread
        let mut shade = false;
        // each column is a contiguous run of cells, lowest first
        for (column, opacity) in levels
            .chunks_mut(size.y as usize)
            .zip(opacity.chunks(size.y as usize))
        {
            // straight down, it keeps its full strength through open air
            let mut level = sky;
            for (cell, &opacity) in column.iter_mut().zip(opacity).rev() {
                level = level.saturating_sub(opacity);
                *cell = level;
                shade |= opacity < MAX_LIGHT && level < MAX_LIGHT;
            }
        }
        let mut light = Self { min, size, levels };
        if shade {
            light.spread(&opacity);
        }
        light
    }

    pub fn index(size: IVec3, local: IVec3) -> usize {
        ((local.x * size.z + local.z) * size.y + local.y) as usize
    }

    // Floods light from every lit cell into any neighbour it would brighten.
    fn spread(&mut self, opacity: &[u8]) {
        let mut queue: VecDeque<IVec3> = VecDeque::new();
        for x in 0..self.size.x {
            for z in 0..self.size.z {
                for y in 0..self.size.y {
                    let local = IVec3::new(x, y, z);
                    let level = self.levels[Self::index(self.size, local)];
                    // only light beside something darker has anywhere to go
                    let darker = |offset: IVec3| {
                        let next = local + offset;
                        !(next.cmplt(IVec3::ZERO).any() || next.cmpge(self.size).any())
                            && self.levels[Self::index(self.size, next)] + 1 < level
                    };
                    if level > 1 && FACE_NORMAL_OFFSET.into_iter().any(darker) {
                        queue.push_back(local);
                    }
                }
            }
        }
        while let Some(local) = queue.pop_front() {
            let level = self.levels[Self::index(self.size, local)];
            for offset in FACE_NORMAL_OFFSET {
                let next = local + offset;
                if next.cmplt(IVec3::ZERO).any() || next.cmpge(self.size).any() {
                    continue;
                }
                let cell = Self::index(self.size, next);
                let lit = level.saturating_sub(1 + opacity[cell]);
                if lit > self.levels[cell] {
                    self.levels[cell] = lit;
                    queue.push_back(next);
                }
            }
        }
    }

    /// The light at `coord`, or full light outside the box.
    pub fn level(&self, coord: IVec3) -> u8 {
        let local = coord - self.min;
        if local.cmplt(IVec3::ZERO).any() || local.cmpge(self.size).any() {
            return MAX_LIGHT;
        }
        self.levels[Self::index(self.size, local)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ochre_base::block::BlockType;

    fn light(size: IVec3, block: impl Fn(IVec3) -> BlockType) -> SkyLight {
        let mut opacity = vec![0; (size.x * size.y * size.z) as usize];
        for x in 0..size.x {
            for z in 0..size.z {
                for y in 0..size.y {
                    let c = IVec3::new(x, y, z);
                    opacity[SkyLight::index(size, c)] = block(c).light_opacity();
                }
            }
        }
        SkyLight::new(IVec3::ZERO, size, MAX_LIGHT, opacity)
    }

    // a 9x12x9 box with a flat roof of `roof` blocks at y 8, from x and z 2 to 6
    fn under_roof(roof: BlockType) -> SkyLight {
        light(IVec3::new(9, 12, 9), |c| {
            let roofed = (2..=6).contains(&c.x) && (2..=6).contains(&c.z) && c.y == 8;
            if c.y == 0 {
                BlockType::Stone
            } else if roofed {
                roof
            } else {
                BlockType::Air
            }
        })
    }

    #[test]
    fn test_open_sky_is_fully_lit_down_to_the_ground() {
        let light = under_roof(BlockType::Air);
        assert_eq!(MAX_LIGHT, light.level(IVec3::new(4, 1, 4)));
        assert_eq!(0, light.level(IVec3::new(4, 0, 4)));
    }

    #[test]
    fn test_light_creeps_under_a_roof_and_fades_inwards() {
        let light = under_roof(BlockType::Stone);
        let edge = light.level(IVec3::new(2, 4, 4));
        let middle = light.level(IVec3::new(4, 4, 4));
        // the cell under the roof's edge is one step in from open sky
        assert_eq!(MAX_LIGHT - 1, edge);
        assert_eq!(MAX_LIGHT - 3, middle);
        assert_eq!(MAX_LIGHT, light.level(IVec3::new(1, 4, 4)));
    }

    #[test]
    fn test_leaves_shade_less_than_stone() {
        let leaves = under_roof(BlockType::Leaves).level(IVec3::new(4, 4, 4));
        let stone = under_roof(BlockType::Stone).level(IVec3::new(4, 4, 4));
        assert_eq!(MAX_LIGHT - 1, leaves);
        assert!(leaves > stone);
    }

    #[test]
    fn test_deep_water_darkens() {
        let light = light(IVec3::new(1, 20, 1), |c| {
            if c.y < 19 {
                BlockType::Water
            } else {
                BlockType::Air
            }
        });
        let depths: Vec<u8> = (0..20)
            .rev()
            .map(|y| light.level(IVec3::new(0, y, 0)))
            .collect();
        assert!(depths.windows(2).all(|w| w[0] >= w[1]), "{depths:?}");
        assert_eq!(0, light.level(IVec3::ZERO));
    }

    #[test]
    fn test_outside_the_box_is_lit() {
        let light = under_roof(BlockType::Stone);
        assert_eq!(MAX_LIGHT, light.level(IVec3::new(-1, 4, 4)));
    }
}
