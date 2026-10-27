//! The world around the terrain: the map it's carved from (erosion, rivers, lakes and climate),
//! and the sky, clouds, weather and ambient sound over it.

pub mod ambience;
pub mod clouds;
pub mod shadow;
pub mod sky;
pub mod sky_dome;
pub mod tonemap;
pub mod weather;
pub mod worldmap;

use std::sync::Arc;

use ochre_terrain::world::World;
use worldmap::{io::load_or_generate, WorldConfig, WorldTerrain};

/// A world over a generated map. `region_distance` is how far, in chunks, terrain is built from
/// full-detail regions, see `LodLayout::region_reach`. The world's map is read from
/// `OCHRE_WORLD_DIR` (default `worlds`), where it's saved the first time a seed is played.
pub fn load_world(region_distance: u32, config: WorldConfig) -> World {
    // OCHRE_SEED repeats a world, for chasing bugs that depend on its terrain
    let fixed: Option<u32> = std::env::var("OCHRE_SEED")
        .ok()
        .and_then(|s| s.parse().ok());
    let dir = std::env::var("OCHRE_WORLD_DIR").unwrap_or_else(|_| "worlds".into());
    let start = std::time::Instant::now();
    let seed = fixed.unwrap_or_else(rand::random);
    let map = load_or_generate(std::path::Path::new(&dir), seed, config);
    tracing::info!("world map for seed {seed} ready in {:.1?}", start.elapsed());
    World::with_terrain(
        seed,
        region_distance,
        Arc::new(WorldTerrain::new(Arc::new(map))),
    )
}
