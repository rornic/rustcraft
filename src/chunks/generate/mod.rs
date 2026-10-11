pub mod biome;
pub mod generator;
pub mod noise;
pub mod region;
pub mod region_store;
mod smooth;
pub mod trees;

/// Height of the world in blocks.
pub const WORLD_HEIGHT: u64 = 1024;
/// Height of the sea's surface in blocks.
pub const WATER_LEVEL: i64 = 448;
