use std::{cell::LazyCell, collections::HashMap, ops::Deref, sync::Arc};

use bevy::{
    asset::RenderAssetUsages,
    math::{I64Vec2, I64Vec3, IVec3, U16Vec3, UVec2, Vec3},
    render::mesh::{Indices, Mesh, VertexAttributeValues},
};

use super::biome::{grass_tint, BiomeId, Biomes};
use super::columns::{ColumnGrid, COARSE_CELL};
use super::light::{SkyLight, MAX_LIGHT};
use super::noise::NoiseGenerator;
use super::region_store::{RegionStore, SLOPE_SPAN};
use super::trees::{
    grows_into, trees_near, Tree, TreeGround, TreeTints, THIN_TRUNK_CELL, TREE_HEIGHT, TREE_REACH,
};
use super::WATER_LEVEL;
use crate::block::{BlockType, Face, Tile, TILE_COUNT};
use crate::chunks::chunk::{neighbor_26_index, ChunkCoordinate, ChunkData, CHUNK_SIZE};
use crate::chunks::lod::LodKey;
use crate::util::primitives::Vertex;

mod buffers;
mod canopy;
mod chunk;
mod greedy;
mod lod;
mod mesh;
mod shading;
mod source;
mod terrain;
#[cfg(test)]
mod tests;

pub use buffers::ChunkMeshes;
pub use chunk::{generate_chunk, generate_chunk_mesh};
pub use lod::generate_lod_mesh;
pub(super) use shading::FACE_NORMAL_OFFSET;
pub(super) use terrain::column_block;
pub use terrain::FREEZE_TEMPERATURE;

#[allow(unused_imports)]
use self::{
    buffers::*, canopy::*, chunk::*, greedy::*, lod::*, mesh::*, shading::*, source::*, terrain::*,
};
