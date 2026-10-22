use super::Biome;

const TREE_DENSITY: f64 = 0.35;

pub struct Forest;

impl Biome for Forest {
    fn name(&self) -> &'static str {
        "forest"
    }

    fn tree_density(&self) -> Option<f64> {
        Some(TREE_DENSITY)
    }
}
