//! Ochre, a voxel engine for Bevy.
//!
//! Each part is its own crate, re-exported here:
//! - [`ochre_base`]: blocks, biomes, terrain sources and the world's fixed heights
//! - [`ochre_terrain`]: chunk storage, generation, meshing and octree LODs
//! - [`ochre_world`]: world maps with erosion, rivers and climate, and the sky, clouds and weather
//! - [`ochre_gameplay`]: the player, collision, wildlife and the debug overlay

pub mod settings;

pub use ochre_base;
pub use ochre_gameplay;
pub use ochre_terrain;
pub use ochre_world;
