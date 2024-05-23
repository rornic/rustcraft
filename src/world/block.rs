#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub enum BlockType {
    Air,
    Stone,
    Grass,
    Sand,
    Water,
    Snow,
}

pub const BLOCK_COUNT: usize = 6;
