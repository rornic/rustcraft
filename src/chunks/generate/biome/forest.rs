use super::{smoothstep, BaseColumn, Biome};

// The wettest climates grow forest, whatever the temperature.
const WET: (f64, f64) = (0.58, 0.7);
const TREE_DENSITY: f64 = 0.55;

// Dense trees over the lush, wet lowlands.
pub struct Forest;

impl Biome for Forest {
    fn name(&self) -> &'static str {
        "forest"
    }

    fn strength(&self, column: &BaseColumn) -> f64 {
        smoothstep(WET.0, WET.1, column.climate.humidity as f64)
    }

    fn tree_density(&self) -> Option<f64> {
        Some(TREE_DENSITY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunks::generate::biome::{Biomes, Climate};

    fn column(humidity: f32) -> BaseColumn {
        BaseColumn {
            x: 0.0,
            z: 0.0,
            cell: 1.0,
            height: 500.0,
            continentalness: 0.3,
            ruggedness: -1.0,
            climate: Climate {
                temperature: 0.4,
                humidity,
            },
        }
    }

    #[test]
    fn test_forests_grow_where_wet() {
        assert_eq!(0.0, Forest.strength(&column(0.4)));
        assert_eq!(1.0, Forest.strength(&column(0.9)));
    }

    #[test]
    fn test_forests_are_denser_than_open_country() {
        let biomes = Biomes::new(1);
        let (_, _, open) = biomes.shape(&column(0.4));
        let (_, owner, forest) = biomes.shape(&column(0.9));
        assert_eq!(Some("forest"), biomes.name(owner));
        assert!(forest > open * 10.0, "forest {forest}, open country {open}");
    }
}
