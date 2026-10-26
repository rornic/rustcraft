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
use worldmap::{gameplay::validate, io::load_or_generate, WorldConfig, WorldTerrain};

// How many random seeds are tried for a playable world before taking the last.
const MAX_SEED_ATTEMPTS: u32 = 8;

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
    // a random seed whose map can't be made playable is dropped for another; a chosen one is
    // kept, with what's wrong with it logged
    let mut seed = fixed.unwrap_or_else(rand::random);
    let mut map = None;
    for attempt in 0..MAX_SEED_ATTEMPTS {
        let candidate = load_or_generate(std::path::Path::new(&dir), seed, config);
        let problems = validate(&candidate);
        if problems.is_empty() {
            map = Some(candidate);
            break;
        }
        tracing::warn!("seed {seed} is not playable: {}", problems.join("; "));
        if fixed.is_some() || attempt + 1 == MAX_SEED_ATTEMPTS {
            map = Some(candidate);
            break;
        }
        seed = rand::random();
    }
    let map = map.expect("a map was made");
    tracing::info!("world map for seed {seed} ready in {:.1?}", start.elapsed());
    World::with_terrain(
        seed,
        region_distance,
        Arc::new(WorldTerrain::new(Arc::new(map))),
    )
}
