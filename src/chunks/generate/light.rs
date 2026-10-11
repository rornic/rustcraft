use std::collections::VecDeque;

use bevy::math::IVec3;

use crate::block::BlockType;

/// Sky light under open sky. Each block it spreads sideways or up loses one level, and
/// blocks it passes through take their `light_opacity` too.
pub const MAX_LIGHT: u8 = 15;

const NEIGHBOURS: [IVec3; 6] = [IVec3::X, IVec3::NEG_X, IVec3::Y, IVec3::NEG_Y, IVec3::Z, IVec3::NEG_Z];

// Sky light over a box of cells, worked out from the blocks in it.
pub struct SkyLight {
    min: IVec3,
    size: IVec3,
    levels: Vec<u8>,
    opacity: Vec<u8>,
}

impl SkyLight {
    /// Lights the box of `size` cells from `min`, where `block` gives each cell's block,
    /// with `sky` falling into the top of every column.
    pub fn new(min: IVec3, size: IVec3, sky: u8, block: impl Fn(IVec3) -> BlockType) -> Self {
        let count = (size.x * size.y * size.z) as usize;
        let mut light = Self {
            min,
            size,
            levels: vec![0; count],
            opacity: vec![0; count],
        };
        // whether anything light can pass through is shaded, so light has somewhere to spread
        let mut shade = false;
        for x in 0..size.x {
            for z in 0..size.z {
                // straight down, it keeps its full strength through open air
                let mut level = sky;
                for y in (0..size.y).rev() {
                    let local = IVec3::new(x, y, z);
                    let cell = light.index(local);
                    let opacity = block(min + local).light_opacity();
                    level = level.saturating_sub(opacity);
                    light.opacity[cell] = opacity;
                    light.levels[cell] = level;
                    shade |= opacity < MAX_LIGHT && level < MAX_LIGHT;
                }
            }
        }
        if shade {
            light.spread();
        }
        light
    }

    fn index(&self, local: IVec3) -> usize {
        ((local.x * self.size.z + local.z) * self.size.y + local.y) as usize
    }

    fn local(&self, coord: IVec3) -> Option<IVec3> {
        let local = coord - self.min;
        (local.cmpge(IVec3::ZERO).all() && local.cmplt(self.size).all()).then_some(local)
    }

    // Floods light out from wherever it's brighter than next door, into shade.
    fn spread(&mut self) {
        let opacity = std::mem::take(&mut self.opacity);
        let gain = |from: u8, into: usize| from.saturating_sub(1 + opacity[into]);
        let mut queue = VecDeque::new();
        for x in 0..self.size.x {
            for z in 0..self.size.z {
                for y in 0..self.size.y {
                    let local = IVec3::new(x, y, z);
                    let cell = self.index(local);
                    if opacity[cell] >= MAX_LIGHT || self.levels[cell] >= MAX_LIGHT - 1 {
                        continue;
                    }
                    let brightest = NEIGHBOURS
                        .iter()
                        .filter_map(|&d| self.local(self.min + local + d))
                        .map(|n| gain(self.levels[self.index(n)], cell))
                        .max()
                        .unwrap_or(0);
                    if brightest > self.levels[cell] {
                        self.levels[cell] = brightest;
                        queue.push_back(local);
                    }
                }
            }
        }
        while let Some(local) = queue.pop_front() {
            let level = self.levels[self.index(local)];
            for d in NEIGHBOURS {
                let Some(n) = self.local(self.min + local + d) else {
                    continue;
                };
                let cell = self.index(n);
                let lit = gain(level, cell);
                if lit > self.levels[cell] {
                    self.levels[cell] = lit;
                    queue.push_back(n);
                }
            }
        }
        self.opacity = opacity;
    }

    /// Whether `coord` hides the blocks behind it from the light, see `BlockType::is_opaque`.
    /// Outside the box is open.
    pub fn opaque(&self, coord: IVec3) -> bool {
        self.local(coord).is_some_and(|local| self.opacity[self.index(local)] >= MAX_LIGHT)
    }

    /// The light at `coord`, or full light outside the box.
    pub fn level(&self, coord: IVec3) -> u8 {
        self.local(coord).map_or(MAX_LIGHT, |local| self.levels[self.index(local)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // a 9x12x9 box with a flat roof of `roof` blocks at y 8, from x and z 2 to 6
    fn under_roof(roof: BlockType) -> SkyLight {
        SkyLight::new(IVec3::ZERO, IVec3::new(9, 12, 9), MAX_LIGHT, |c| {
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
        let light = SkyLight::new(IVec3::ZERO, IVec3::new(1, 20, 1), MAX_LIGHT, |c| {
            if c.y < 19 { BlockType::Water } else { BlockType::Air }
        });
        let depths: Vec<u8> = (0..20).rev().map(|y| light.level(IVec3::new(0, y, 0))).collect();
        assert!(depths.windows(2).all(|w| w[0] >= w[1]), "{depths:?}");
        assert_eq!(0, light.level(IVec3::ZERO));
    }

    #[test]
    fn test_outside_the_box_is_lit() {
        let light = under_roof(BlockType::Stone);
        assert_eq!(MAX_LIGHT, light.level(IVec3::new(-1, 4, 4)));
    }
}
