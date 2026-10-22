//! The types every Ochre crate shares: blocks, biomes, what a terrain source answers, and the
//! world's fixed heights.

pub mod biome;
pub mod block;
pub mod math;
pub mod noise;
pub mod relief;
pub mod terrain_source;

/// Height of the world in blocks.
pub const WORLD_HEIGHT: u64 = 1024;
/// Height of the sea's surface in blocks.
pub const WATER_LEVEL: i64 = 448;

/// Sky light under open sky. Each block it spreads sideways or up loses one level, and
/// blocks it passes through take their `light_opacity` too.
pub const MAX_LIGHT: u8 = 15;
