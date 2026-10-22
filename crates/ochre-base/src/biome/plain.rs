use super::Biome;

/// A biome that only sets how thickly trees grow: a world map picks it, and the ground keeps
/// the base terrain's blocks.
pub struct Plain {
    name: &'static str,
    trees: f64,
}

impl Plain {
    pub fn new(name: &'static str, trees: f64) -> Self {
        Self { name, trees }
    }
}

impl Biome for Plain {
    fn name(&self) -> &'static str {
        self.name
    }

    fn tree_density(&self) -> Option<f64> {
        Some(self.trees)
    }
}
