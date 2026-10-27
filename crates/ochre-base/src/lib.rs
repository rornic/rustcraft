//! The types every Ochre crate shares: blocks, biomes, what a terrain source answers, and the
//! world's sea level and supported vertical bounds.

pub mod biome;
pub mod block;
pub mod math;
pub mod noise;
pub mod relief;
pub mod terrain_source;

/// Default build ceiling in blocks; runtime generation and loading use `WorldBounds`.
pub const WORLD_HEIGHT: u64 = 2048;
/// Height of the sea's surface in blocks.
pub const WATER_LEVEL: i64 = 0;

/// Supported block elevations. `max_y` is exclusive; both ends are chunk aligned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(default)]
pub struct WorldBounds {
    pub min_y: i64,
    pub max_y: i64,
}

impl Default for WorldBounds {
    fn default() -> Self {
        Self {
            min_y: -1024,
            max_y: 2048,
        }
    }
}

impl WorldBounds {
    pub fn validate(self) -> Result<(), String> {
        if self.min_y > -64
            || self.max_y < 128
            || self.min_y >= self.max_y
            || self.min_y % 16 != 0
            || self.max_y % 16 != 0
            || self.min_y < -1_048_576
            || self.max_y > 1_048_576
        {
            return Err("world bounds need min_y <= -64, max_y >= 128, alignment to 16 blocks, and must lie within ±1048576".into());
        }
        Ok(())
    }

    pub fn layers(self) -> std::ops::Range<i64> {
        self.min_y / 16..self.max_y / 16
    }
}

/// Sky light under open sky. Each block it spreads sideways or up loses one level, and
/// blocks it passes through take their `light_opacity` too.
pub const MAX_LIGHT: u8 = 15;

#[cfg(test)]
mod bounds_tests {
    use super::WorldBounds;
    #[test]
    fn signed_bounds_are_chunk_aligned_and_exclusive() {
        let b = WorldBounds::default();
        assert!(b.validate().is_ok());
        assert_eq!(b.layers(), -64..128);
        assert!(!b.layers().contains(&128));
        assert!(WorldBounds {
            min_y: -1,
            max_y: 2048
        }
        .validate()
        .is_err());
        assert!(WorldBounds {
            min_y: 0,
            max_y: 2048
        }
        .validate()
        .is_err());
    }
}
