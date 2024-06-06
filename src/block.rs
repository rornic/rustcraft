#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub enum BlockType {
    Air,
    Stone,
    Grass,
    Sand,
    Water,
    Snow,
}

impl Default for BlockType {
    fn default() -> Self {
        Self::Air
    }
}

pub const BLOCK_COUNT: usize = 6;
