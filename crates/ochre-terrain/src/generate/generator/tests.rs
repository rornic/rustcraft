use ochre_base::terrain_source::TerrainSource as _;
use std::sync::Arc;

use bevy::math::Vec3;
use bevy::math::{I64Vec3, IVec3, U16Vec3};
use bevy::platform::collections::HashMap;
use bevy::render::mesh::{Indices, Mesh, VertexAttributeValues};

use ochre_base::block::{BlockType, Tile};

use super::buffers::{ChunkMeshes, GrassTint, InnerLeaves, MeshStyle};
use super::chunk::{generate_chunk, generate_chunk_mesh};
use super::lod::LodBlocks;
use super::shading::{
    spread_water_levels, vertex_ao, vertex_water_depth, water_corner, AO_STRENGTH,
    WATER_SPREAD_REACH,
};
use super::source::{water_depth, BlockSource, ChunkNeighbourhood};
use super::terrain::{freeze_cell, terrain_block, under_grass, FREEZE_TEMPERATURE};
use crate::chunk::{neighbor_26_index, ChunkCoordinate, ChunkData, CHUNK_SIZE};
use crate::generate::light::SkyLight;
use crate::generate::region_store::RegionStore;
use crate::lod::LodKey;
use ochre_base::WATER_LEVEL;

// leaves hiding the leaves behind them, as every LOD draws them
fn style(ambient_occlusion: bool, textured: bool) -> MeshStyle {
    MeshStyle {
        ambient_occlusion,
        textured,
        inner_leaves: InnerLeaves::Hidden,
    }
}

fn no_neighbors() -> [Option<Arc<ChunkData>>; 26] {
    std::array::from_fn(|_| None)
}

#[test]
fn test_neighbourhood_within_chunk() {
    let mut chunk = ChunkData::default();
    chunk.set_block_at(U16Vec3::new(4, 4, 4), BlockType::Stone);

    let block = ChunkNeighbourhood::new(&chunk, &no_neighbors()).block(IVec3::new(4, 4, 4));
    assert_eq!(BlockType::Stone, block);
}

#[test]
fn test_neighbourhood_crosses_single_axis_into_adjacent_chunk() {
    let chunk = ChunkData::default();
    let mut adjacent = ChunkData::default();
    adjacent.set_block_at(U16Vec3::new(0, 4, 4), BlockType::Stone);

    let mut neighbors = no_neighbors();
    neighbors[neighbor_26_index(1, 0, 0)] = Some(Arc::new(adjacent));
    let block =
        ChunkNeighbourhood::new(&chunk, &neighbors).block(IVec3::new(CHUNK_SIZE as i32, 4, 4));
    assert_eq!(BlockType::Stone, block);
}

#[test]
fn test_neighbourhood_crosses_two_axes_into_diagonal_chunk() {
    let chunk = ChunkData::default();
    let mut diagonal = ChunkData::default();
    diagonal.set_block_at(U16Vec3::new(0, 0, 4), BlockType::Stone);

    let mut neighbors = no_neighbors();
    neighbors[neighbor_26_index(1, 1, 0)] = Some(Arc::new(diagonal));
    let block = ChunkNeighbourhood::new(&chunk, &neighbors).block(IVec3::new(
        CHUNK_SIZE as i32,
        CHUNK_SIZE as i32,
        4,
    ));
    assert_eq!(BlockType::Stone, block);
}

#[test]
fn test_neighbourhood_missing_diagonal_chunk_is_air() {
    let chunk = ChunkData::default();
    let block = ChunkNeighbourhood::new(&chunk, &no_neighbors()).block(IVec3::new(
        CHUNK_SIZE as i32,
        CHUNK_SIZE as i32,
        4,
    ));
    assert_eq!(BlockType::Air, block);
}

#[test]
fn test_vertex_ao_no_occluders_is_unlit() {
    let chunk = ChunkData::default();
    let neighbors = no_neighbors();
    let source = ChunkNeighbourhood::new(&chunk, &neighbors);
    let ao = vertex_ao(&source, IVec3::new(4, 4, 4), 4, [0.5, 0.5, -0.5]);
    assert_eq!(1.0, ao);
}

#[test]
fn test_vertex_ao_fully_occluded_corner() {
    let mut chunk = ChunkData::default();
    // Top face (index 4) at (4,4,4): occluders are the y+1 layer's tangent/diagonal cells.
    chunk.set_block_at(U16Vec3::new(5, 5, 4), BlockType::Stone);
    chunk.set_block_at(U16Vec3::new(4, 5, 5), BlockType::Stone);
    chunk.set_block_at(U16Vec3::new(5, 5, 5), BlockType::Stone);

    let neighbors = no_neighbors();
    let source = ChunkNeighbourhood::new(&chunk, &neighbors);
    let ao = vertex_ao(&source, IVec3::new(4, 4, 4), 4, [0.5, 0.5, 0.5]);
    assert_eq!(1.0 - AO_STRENGTH, ao);
}

#[test]
fn test_neighbouring_water_columns_agree_on_shared_corner_depth() {
    let mut chunk = ChunkData::default();
    // two columns with their surface at y = 5: one 5 deep, its +x neighbour 2 deep
    chunk.set_block_at(U16Vec3::new(4, 0, 4), BlockType::Stone);
    for y in 1..=5 {
        chunk.set_block_at(U16Vec3::new(4, y, 4), BlockType::Water);
    }
    for y in 0..=3 {
        chunk.set_block_at(U16Vec3::new(5, y, 4), BlockType::Stone);
    }
    for y in 4..=5 {
        chunk.set_block_at(U16Vec3::new(5, y, 4), BlockType::Water);
    }
    let neighbors = no_neighbors();
    let source = ChunkNeighbourhood::new(&chunk, &neighbors);
    let mut depth_of = |coord: IVec3| water_depth(&source, coord) as f32;

    // the corner between them, on the +z side: seen from each column's top face
    let deep = vertex_water_depth(&mut depth_of, IVec3::new(4, 5, 4), 4, [0.5, 0.5, 0.5]);
    let shallow = vertex_water_depth(&mut depth_of, IVec3::new(5, 5, 4), 4, [-0.5, 0.5, 0.5]);
    assert_eq!(deep, shallow);
    // averaged with the two dry columns on the +z side
    assert_eq!((5.0 + 2.0) / 4.0, deep);
}

#[test]
fn test_a_rivers_surface_is_continuous_across_the_blocks_it_runs_down() {
    // a river falling 0.3 a column along x, over blocks whose tops step as it passes whole numbers
    let level = |x: i32, _z: i32| Some(10.4 - 0.3 * x as f32);
    let corner = |x: i32, sign: f32| {
        water_corner(
            &mut |x, z| level(x, z),
            (x, 0),
            level(x, 0).unwrap(),
            (if sign > 0.0 { 1 } else { -1 }, 1),
        )
        .height
    };
    // each corner is the same seen from the column either side of it
    for x in 0..8 {
        let (a, b) = (corner(x, 0.5), corner(x + 1, -0.5));
        assert!((a - b).abs() < 1e-5, "{a} {b}");
    }
    // falling all the way down, with no step at the corners where the blocks' tops change
    let heights: Vec<f32> = (0..8).map(|x| corner(x, 0.5)).collect();
    assert!(heights.windows(2).all(|w| w[0] > w[1]), "{heights:?}");
    // a curtain, two blocks apart, is not averaged across
    let mut stepped = |x: i32, _z: i32| Some(if x <= 0 { 10.0 } else { 7.0 });
    assert_eq!(
        10.0,
        water_corner(&mut stepped, (0, 0), 10.0, (1, 1)).height
    );
}

#[test]
fn test_a_step_in_the_water_ramps_over_the_columns_round_it() {
    let reach = WATER_SPREAD_REACH;
    let wide = 6 * reach;
    // water 5 high for x < 3 * reach and 3 lower beyond
    let raw: Vec<f32> = (0..wide * wide)
        .map(|i| if i / wide < 3 * reach { 5.0 } else { 2.0 })
        .collect();
    let spread = spread_water_levels(&raw, wide);
    let inner = wide - 2 * reach;
    let at = |x: i32| spread[((x - reach) * inner + inner / 2) as usize];
    let ramp: Vec<f32> = (reach..5 * reach).map(at).collect();
    // falling all the way, by under a block a column, so it's one slope with no curtain
    assert!(
        ramp.windows(2).all(|w| w[0] >= w[1] && w[0] - w[1] < 1.0),
        "{ramp:?}"
    );
    // and only near the step
    assert_eq!((5.0, 2.0), (at(reach), at(5 * reach - 1)));
}

#[test]
fn test_desert_ground_is_sand_and_other_lowland_grass() {
    use bevy::math::I64Vec2;

    let regions = Arc::new(RegionStore::new(ochre_world::worldmap::test_terrain(3)));
    let lowland = |desert: bool| {
        (-60..60)
            .flat_map(|z| (-60..60).map(move |x| I64Vec2::new(x, z) * 128))
            .find(|p| {
                let sample = regions.terrain().sample(p.x as f64, p.y as f64);
                let above_sea = sample.height - WATER_LEVEL as f64;
                (regions.terrain().biomes().name(sample.biome) == Some("desert")) == desert
                    && (20.0..60.0).contains(&above_sea)
                    && regions.sampler().gradient(p.x, p.y).0.abs() < 0.5
                    && regions.sampler().gradient(p.x, p.y).1.abs() < 0.5
            })
            .expect("no lowland of that kind near the centre")
    };
    let surface = |p: I64Vec2| {
        let top = regions.get_height(p.x, p.y).round() as i64 - 1;
        let coord = ChunkCoordinate(I64Vec3::new(
            p.x.div_euclid(16),
            top.div_euclid(16),
            p.y.div_euclid(16),
        ));
        let local = U16Vec3::new(
            p.x.rem_euclid(16) as u16,
            top.rem_euclid(16) as u16,
            p.y.rem_euclid(16) as u16,
        );
        generate_chunk(regions.clone(), coord).get_block_at(local)
    };
    assert_eq!(BlockType::Sand, surface(lowland(true)));
    assert_eq!(BlockType::Grass, surface(lowland(false)));
}

#[test]
fn test_trees_across_chunk_borders_are_whole() {
    use crate::generate::trees::trees_near;
    use bevy::math::I64Vec2;

    let regions = Arc::new(RegionStore::new(ochre_world::worldmap::test_terrain(5)));
    let mut heights = regions.sampler();
    let trees = trees_near(
        regions.terrain(),
        &mut heights,
        forest_at(5) - I64Vec2::splat(800),
        forest_at(5) + I64Vec2::splat(800),
    );
    let chunk_of = |p: I64Vec3| p.div_euclid(I64Vec3::splat(16));
    let tree = trees
        .iter()
        .find(|t| {
            let (low, high) = t.bounds();
            chunk_of(low).x != chunk_of(high).x && chunk_of(low).z != chunk_of(high).z
        })
        .expect("no tree straddling a chunk corner");

    let mut chunks = HashMap::new();
    let (low, high) = tree.bounds();
    let mut checked = 0;
    tree.plant(
        low,
        1,
        (IVec3::ZERO, (high - low).as_ivec3()),
        |cell, block| {
            let p = low + cell.as_i64vec3();
            let coord = chunk_of(p);
            let chunk = chunks
                .entry(coord)
                .or_insert_with(|| generate_chunk(regions.clone(), ChunkCoordinate(coord)));
            let local = (p - coord * 16).as_u16vec3();
            let found = chunk.get_block_at(local);
            // only another tree's log may stand in for this one's leaves
            assert!(
                found == block || (block == BlockType::Leaves && found == BlockType::Log),
                "{block:?} at {p} is {found:?}"
            );
            checked += 1;
        },
    );
    assert!(
        chunks.len() >= 2 && checked > 20,
        "{} blocks over {} chunks",
        checked,
        chunks.len()
    );
}

#[test]
fn test_cold_sea_freezes_from_the_top_and_thicker_when_colder() {
    let ice_layers = |temperature| {
        (WATER_LEVEL - 10..WATER_LEVEL)
            .filter(|&y| terrain_block(y, 0, 0.0, temperature, WATER_LEVEL) == BlockType::Ice)
            .count()
    };
    assert_eq!(0, ice_layers(0.5));
    assert_eq!(0, ice_layers(FREEZE_TEMPERATURE));
    assert_eq!(1, ice_layers(FREEZE_TEMPERATURE - 0.01));
    assert_eq!(3, ice_layers(0.0));
    assert_eq!(
        BlockType::Ice,
        terrain_block(WATER_LEVEL - 1, 0, 0.0, 0.0, WATER_LEVEL)
    );
    assert_eq!(
        BlockType::Water,
        terrain_block(WATER_LEVEL - 4, 0, 0.0, 0.0, WATER_LEVEL)
    );
    assert_eq!(
        BlockType::Air,
        terrain_block(WATER_LEVEL, 0, 0.0, 0.0, WATER_LEVEL)
    );
}

#[test]
fn test_wide_cells_freeze_when_they_reach_the_ice() {
    let top = WATER_LEVEL;
    assert_eq!(
        BlockType::Ice,
        freeze_cell(BlockType::Water, 0.0, top, (32, WATER_LEVEL))
    );
    assert_eq!(
        BlockType::Water,
        freeze_cell(BlockType::Water, 0.0, top - 32, (32, WATER_LEVEL))
    );
    assert_eq!(
        BlockType::Water,
        freeze_cell(BlockType::Water, 0.5, top, (32, WATER_LEVEL))
    );
    assert_eq!(
        BlockType::Stone,
        freeze_cell(BlockType::Stone, 0.0, top, (32, WATER_LEVEL))
    );
}

#[test]
fn test_polar_lowlands_are_snow_covered_and_deep_ice_gives_way_to_snow() {
    let ground = WATER_LEVEL + 20;
    assert_eq!(
        BlockType::Snow,
        terrain_block(ground - 1, ground, 0.0, 0.1, WATER_LEVEL)
    );
    assert_eq!(
        BlockType::Grass,
        terrain_block(ground - 1, ground, 0.0, 0.5, WATER_LEVEL)
    );
    assert_eq!(
        BlockType::Snow,
        under_grass(BlockType::Ice, ground - 3, ground, 1, false)
    );
    assert_eq!(
        BlockType::Ice,
        under_grass(BlockType::Ice, ground - 1, ground, 1, false)
    );
}

#[test]
fn test_terrain_block_floor_and_water() {
    // below the solid floor even a zero-height column is solid
    assert_eq!(BlockType::Sand, terrain_block(5, 0, 0.0, 0.5, WATER_LEVEL));
    assert_eq!(
        BlockType::Water,
        terrain_block(20, 0, 0.0, 0.5, WATER_LEVEL)
    );
    assert_eq!(
        BlockType::Air,
        terrain_block(WATER_LEVEL, 0, 0.0, 0.5, WATER_LEVEL)
    );
    assert_eq!(
        BlockType::Grass,
        terrain_block(WATER_LEVEL + 10, WATER_LEVEL + 20, 0.0, 0.5, WATER_LEVEL)
    );
}

#[test]
fn test_snow_settles_on_steeper_ground_higher_up() {
    let at = |above_sea: i64, slope: f64| {
        let y = WATER_LEVEL + above_sea;
        terrain_block(y, y + 1, slope, 0.5, WATER_LEVEL)
    };
    assert_ne!(BlockType::Snow, at(200, 0.0));
    assert_eq!(BlockType::Snow, at(230, 1.0));
    assert_ne!(BlockType::Snow, at(230, 3.0));
    assert_eq!(BlockType::Snow, at(360, 3.0));
    // cliffs stay bare however high they are
    assert_ne!(BlockType::Snow, at(400, 6.0));
}

#[test]
fn test_snow_line_drops_in_the_cold_and_hot_ground_is_bare() {
    let at = |above_sea: i64, temperature: f32| {
        let y = WATER_LEVEL + above_sea;
        terrain_block(y, y + 1, 0.0, temperature, WATER_LEVEL)
    };
    assert_ne!(BlockType::Snow, at(100, 0.5));
    assert_eq!(BlockType::Snow, at(100, 0.35));
    assert_eq!(BlockType::Snow, at(300, 0.5));
    // too hot for snow however high, so the peaks are rock
    assert_eq!(BlockType::Stone, at(300, 0.8));
    assert_eq!(BlockType::Grass, at(100, 0.8));
}

#[test]
fn test_only_the_top_of_frozen_grass_is_snow() {
    let at =
        |y: i64, temperature: f32| terrain_block(y, WATER_LEVEL + 7, 0.0, temperature, WATER_LEVEL);
    let top = WATER_LEVEL + 6;
    assert_eq!(BlockType::Snow, at(top, 0.05));
    assert_eq!(BlockType::Grass, at(top, 0.3));
    assert_eq!(BlockType::Grass, at(top - 1, 0.05));
}

fn test_regions() -> Arc<RegionStore> {
    Arc::new(RegionStore::new(ochre_world::worldmap::test_terrain(3)))
}

#[test]
fn test_lod_level_zero_matches_full_detail_chunk() {
    let regions = test_regions();
    for coord in [
        I64Vec3::new(1, 1, 2),
        I64Vec3::new(-2, 0, 3),
        I64Vec3::new(0, 2, -1),
    ] {
        let chunk = generate_chunk(regions.clone(), ChunkCoordinate(coord));
        let lod = LodBlocks::sample(&regions, LodKey { level: 0, coord });
        for x in 0..16 {
            for y in 0..16 {
                for z in 0..16 {
                    assert_eq!(
                        chunk.get_block_at(U16Vec3::new(x, y, z)),
                        lod.raw(IVec3::new(x as i32, y as i32, z as i32)),
                        "chunk {coord} block ({x},{y},{z})"
                    );
                }
            }
        }
    }
}

fn flat_lod(surface_y: i32) -> LodBlocks {
    let size = 16;
    let padded = (size + 2) as usize;
    let mut cells = vec![BlockType::Air; padded * padded * padded];
    for x in -1..=size {
        for z in -1..=size {
            for y in -1..surface_y {
                cells[LodBlocks::index(size, IVec3::new(x, y, z))] = BlockType::Stone;
            }
        }
    }
    let tops = vec![surface_y as i64; padded * padded];
    LodBlocks::new(size, cells, tops, 0, 1)
}

// Grass two cells of 4 blocks deep, its ground 6 blocks up the node for x < 8 and 7 blocks
// beyond: both inside the second cell, so its top is lowered. Above the sea, under open sky.
fn sloped_lod() -> LodBlocks {
    let base_y = WATER_LEVEL + 64;
    let size = 16;
    let padded = (size + 2) as usize;
    let mut cells = vec![BlockType::Air; padded * padded * padded];
    let mut tops = vec![0; padded * padded];
    for x in -1..=size {
        for z in -1..=size {
            tops[LodBlocks::column(size, x, z)] = base_y + if x < 8 { 6 } else { 7 };
            for y in -1..=1 {
                cells[LodBlocks::index(size, IVec3::new(x, y, z))] = BlockType::Grass;
            }
        }
    }
    LodBlocks::new(size, cells, tops, base_y, 4)
}

// `lod`'s opaque mesh, lit as `generate_lod_mesh` lights it
fn mesh_lod(lod: &LodBlocks) -> Mesh {
    let top = lod.base_y + (lod.size as i64 + 1) * lod.cell;
    let opacity = lod.cells.iter().map(BlockType::light_opacity).collect();
    let light = Box::new(SkyLight::new(
        IVec3::splat(-1),
        IVec3::splat(lod.size + 2),
        super::chunk::sky_above(top, lod.cell),
        opacity,
    ));
    let tint = GrassTint::new(
        &*ochre_world::worldmap::test_terrain(3),
        Vec3::ZERO,
        lod.cell as f32,
    );
    super::mesh::mesh_blocks(lod, lod.cell, style(false, false), &tint, &light).opaque
}

// each face's normal and its vertices' positions
fn faces(mesh: &Mesh) -> Vec<([f32; 3], Vec<[f32; 3]>)> {
    let Some(VertexAttributeValues::Float32x3(positions)) =
        mesh.attribute(Mesh::ATTRIBUTE_POSITION)
    else {
        panic!("missing positions");
    };
    let Some(VertexAttributeValues::Float32x3(normals)) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL)
    else {
        panic!("missing normals");
    };
    let mut faces: Vec<([f32; 3], Vec<[f32; 3]>)> = Vec::new();
    for (p, n) in positions.iter().zip(normals) {
        match faces
            .iter_mut()
            .find(|(normal, vs)| normal == n && vs.len() < 4)
        {
            Some((_, vs)) => vs.push(*p),
            None => faces.push((*n, vec![*p])),
        }
    }
    faces
}

#[test]
fn test_lod_surface_tops_out_at_the_grounds_height() {
    let lod = sloped_lod();
    // (6 - 0) / 4 - 0.5 and (7 - 0) / 4 - 0.5, in cell units centred on the cells
    assert_eq!(Some(1.0), lod.surface_top(IVec3::new(2, 1, 2)));
    assert_eq!(Some(1.25), lod.surface_top(IVec3::new(10, 1, 2)));
    assert_eq!(None, lod.surface_top(IVec3::new(2, 0, 2)));
}

#[test]
fn test_lod_surface_under_water_fills_its_cell() {
    let LodBlocks {
        size,
        mut cells,
        tops,
        base_y,
        cell,
        ..
    } = sloped_lod();
    cells[LodBlocks::index(16, IVec3::new(2, 2, 2))] = BlockType::Water;
    // nor does one beside water, which would show a gap under the water's side
    cells[LodBlocks::index(16, IVec3::new(6, 1, 2))] = BlockType::Water;
    let lod = LodBlocks::new(size, cells, tops, base_y, cell);
    assert_eq!(None, lod.surface_top(IVec3::new(2, 1, 2)));
    assert_eq!(None, lod.surface_top(IVec3::new(5, 1, 2)));
    assert_eq!(Some(1.0), lod.surface_top(IVec3::new(4, 1, 2)));
}

#[test]
fn test_lod_tops_are_drawn_at_the_grounds_height_with_a_step_between() {
    let faces = faces(&mesh_lod(&sloped_lod()));
    let tops: Vec<f32> = faces
        .iter()
        .filter(|(n, _)| *n == [0.0, 1.0, 0.0])
        .flat_map(|(_, vs)| vs.iter().map(|v| v[1]))
        .collect();
    assert!(tops.contains(&1.0) && tops.contains(&1.25));
    assert!(
        tops.iter().all(|&y| y == 1.0 || y == 1.25),
        "tops at {tops:?}"
    );
    // the higher ground's side, facing the lower, covers just the step between them
    let step: Vec<_> = faces
        .iter()
        .filter(|(n, vs)| *n == [-1.0, 0.0, 0.0] && vs.iter().all(|v| v[0] == 7.5))
        .collect();
    assert!(!step.is_empty());
    // one face along the whole contour, not one for each of its 16 cells
    let vertices: usize = step.iter().map(|(_, vs)| vs.len()).sum();
    assert!(vertices < 16 * 4, "{vertices} vertices along the step");
    for (_, vs) in step {
        assert!(
            vs.iter().all(|v| v[1] == 1.0 || v[1] == 1.25),
            "step at {vs:?}"
        );
    }
}

#[test]
fn test_lod_steps_are_lit_by_the_air_above_the_lower_ground() {
    let mesh = mesh_lod(&sloped_lod());
    let Some(VertexAttributeValues::Float32x3(positions)) =
        mesh.attribute(Mesh::ATTRIBUTE_POSITION)
    else {
        panic!("missing positions");
    };
    let Some(VertexAttributeValues::Float32x3(normals)) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL)
    else {
        panic!("missing normals");
    };
    let Some(VertexAttributeValues::Float32x2(lights)) = mesh.attribute(Mesh::ATTRIBUTE_UV_1)
    else {
        panic!("missing light");
    };
    let step: Vec<f32> = (0..positions.len())
        .filter(|&i| {
            normals[i] == [-1.0, 0.0, 0.0] && positions[i][0] == 7.5 && positions[i][1] >= 1.0
        })
        .map(|i| lights[i][0])
        .collect();
    assert!(!step.is_empty());
    assert!(
        step.iter().all(|&l| l == 1.0),
        "steps under open sky lit {step:?}"
    );
}

fn mesh_chunk(chunk: ChunkData) -> ChunkMeshes {
    // well above the sea, under open sky
    let coord = ChunkCoordinate(I64Vec3::new(0, 40, 0));
    generate_chunk_mesh(
        Arc::new(chunk),
        vec![None; 26],
        coord,
        &RegionStore::new(ochre_world::worldmap::test_terrain(3)),
        false,
    )
}

fn mesh_vertices(chunk: ChunkData) -> usize {
    mesh_chunk(chunk).opaque.count_vertices()
}

#[test]
fn test_single_block_has_six_quads() {
    let mut chunk = ChunkData::default();
    chunk.set_block_at(U16Vec3::new(4, 4, 4), BlockType::Stone);
    assert_eq!(24, mesh_vertices(chunk));
}

#[test]
fn test_flat_floor_merges_into_one_quad_per_face() {
    let mut chunk = ChunkData::default();
    for x in 0..16 {
        for z in 0..16 {
            chunk.set_block_at(U16Vec3::new(x, 0, z), BlockType::Stone);
        }
    }
    // each quad is a centre point plus a vertex per block step round its border: the top
    // is 16x16, the four sides 16x1. The underside shades as light creeps in under it.
    let mesh = mesh_chunk(chunk).opaque;
    let Some(VertexAttributeValues::Float32x3(normals)) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL)
    else {
        panic!("missing normals");
    };
    let facing = |n: [f32; 3]| normals.iter().filter(|&&v| v == n).count();
    assert_eq!(1 + 64, facing([0.0, 1.0, 0.0]));
    for side in [
        [1.0, 0.0, 0.0],
        [-1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0],
        [0.0, 0.0, -1.0],
    ] {
        assert_eq!(1 + 34, facing(side));
    }
}

#[test]
fn test_different_block_types_do_not_merge() {
    let mut chunk = ChunkData::default();
    chunk.set_block_at(U16Vec3::new(4, 4, 4), BlockType::Stone);
    chunk.set_block_at(U16Vec3::new(5, 4, 4), BlockType::Grass);
    // 5 faces each, the shared one hidden
    assert_eq!(10 * 4, mesh_vertices(chunk));
}

fn floor_chunk() -> ChunkData {
    let mut chunk = ChunkData::default();
    for x in 0..16 {
        for z in 0..16 {
            chunk.set_block_at(U16Vec3::new(x, 0, z), BlockType::Stone);
        }
    }
    chunk
}

#[test]
fn test_merged_quads_tile_their_uvs_and_carry_the_atlas_cell() {
    let mesh = mesh_chunk(floor_chunk()).opaque;
    let Some(VertexAttributeValues::Float32x2(uvs)) = mesh.attribute(Mesh::ATTRIBUTE_UV_0) else {
        panic!("missing uvs");
    };
    let Some(VertexAttributeValues::Float32x4(colors)) = mesh.attribute(Mesh::ATTRIBUTE_COLOR)
    else {
        panic!("missing colors");
    };
    let tiles = uvs.iter().flatten().fold(0.0f32, |m, &c| m.max(c));
    assert_eq!(16.0, tiles);
    assert!(colors.iter().all(|c| c[3] == Tile::Stone.atlas_cell()));
}

fn top_face_triangles(mesh: &Mesh) -> Vec<f32> {
    let Some(VertexAttributeValues::Float32x3(positions)) =
        mesh.attribute(Mesh::ATTRIBUTE_POSITION)
    else {
        panic!("missing positions");
    };
    let Some(VertexAttributeValues::Float32x3(normals)) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL)
    else {
        panic!("missing normals");
    };
    let Some(Indices::U32(indices)) = mesh.indices() else {
        panic!("missing indices");
    };
    indices
        .chunks(3)
        .filter(|t| t.iter().all(|&i| normals[i as usize] == [0.0, 1.0, 0.0]))
        .map(|t| {
            let p = |k: usize| Vec3::from(positions[t[k] as usize]);
            (p(1) - p(0)).cross(p(2) - p(0)).y
        })
        .filter(|area| *area != 0.0)
        .collect()
}

#[test]
fn test_merged_quads_keep_the_winding_of_single_faces() {
    let mut single = ChunkData::default();
    single.set_block_at(U16Vec3::new(4, 4, 4), BlockType::Stone);
    let single = mesh_chunk(single).opaque;
    let merged = mesh_chunk(floor_chunk()).opaque;
    let expected = top_face_triangles(&single)[0].signum();
    assert!(top_face_triangles(&merged)
        .iter()
        .all(|area| area.signum() == expected));
}

#[test]
fn test_merged_quad_borders_share_vertices_with_unit_faces() {
    let mesh = mesh_chunk(floor_chunk()).opaque;
    let Some(VertexAttributeValues::Float32x3(positions)) =
        mesh.attribute(Mesh::ATTRIBUTE_POSITION)
    else {
        panic!("missing positions");
    };
    for step in 0..=16 {
        let edge = [step as f32 - 0.5, 0.5, -0.5];
        assert!(positions.contains(&edge), "no top-face vertex at {edge:?}");
    }
}

fn tints(mesh: &Mesh) -> Vec<Vec3> {
    let Some(VertexAttributeValues::Float32x4(colors)) = mesh.attribute(Mesh::ATTRIBUTE_COLOR)
    else {
        panic!("missing colors");
    };
    colors.iter().map(|c| Vec3::new(c[0], c[1], c[2])).collect()
}

#[test]
fn test_only_grass_tops_and_sides_are_tinted() {
    let mut chunk = ChunkData::default();
    chunk.set_block_at(U16Vec3::new(4, 4, 4), BlockType::Stone);
    chunk.set_block_at(U16Vec3::new(10, 4, 10), BlockType::Grass);
    let colours = tints(&mesh_chunk(chunk).opaque);
    assert_eq!(28, colours.iter().filter(|c| **c == Vec3::ONE).count());
    assert_eq!(20, colours.iter().filter(|c| **c != Vec3::ONE).count());
}

#[test]
fn test_grass_sides_carry_their_untinted_shade() {
    let mut chunk = ChunkData::default();
    chunk.set_block_at(U16Vec3::new(10, 4, 10), BlockType::Grass);
    chunk.set_block_at(U16Vec3::new(11, 4, 10), BlockType::Grass);
    let mesh = mesh_chunk(chunk).opaque;
    let (
        Some(VertexAttributeValues::Float32x4(colors)),
        Some(VertexAttributeValues::Float32x2(modes)),
    ) = (
        mesh.attribute(Mesh::ATTRIBUTE_COLOR),
        mesh.attribute(Mesh::ATTRIBUTE_UV_1),
    )
    else {
        panic!("mesh lacks colours or uv_b");
    };
    let side = Tile::GrassSide.atlas_cell();
    let mut sides = 0;
    for (color, mode) in colors.iter().zip(modes) {
        if color[3] == side {
            sides += 1;
            // the shade is ambient occlusion here, so between dark and full
            assert!(mode[1] < 0.0 && mode[1] >= -1.0, "{mode:?}");
        } else {
            assert_eq!(super::buffers::TEXTURED, mode[1]);
        }
    }
    assert!(sides > 0);
}

#[test]
fn test_only_full_detail_grass_below_the_top_layer_is_dirt() {
    assert_eq!(
        BlockType::Dirt,
        under_grass(BlockType::Grass, 16, 20, 1, false)
    );
    assert_eq!(
        BlockType::Grass,
        under_grass(BlockType::Grass, 19, 20, 1, false)
    );
    assert_eq!(
        BlockType::Grass,
        under_grass(BlockType::Grass, 10, 20, 4, false)
    );
    assert_eq!(
        BlockType::Stone,
        under_grass(BlockType::Stone, 16, 20, 1, false)
    );
}

#[test]
fn test_faces_pick_their_tile_and_merge_within_a_tile() {
    let mut chunk = ChunkData::default();
    chunk.set_block_at(U16Vec3::new(4, 4, 4), BlockType::Log);
    let mesh = mesh_chunk(chunk).opaque;
    let Some(VertexAttributeValues::Float32x4(colors)) = mesh.attribute(Mesh::ATTRIBUTE_COLOR)
    else {
        panic!("missing colors");
    };
    let count = |tile: Tile| colors.iter().filter(|c| c[3] == tile.atlas_cell()).count();
    assert_eq!(8, count(Tile::LogEnd));
    assert_eq!(16, count(Tile::LogBark));
    let merged = mesh_chunk(floor_chunk()).opaque;
    let Some(VertexAttributeValues::Float32x4(colors)) = merged.attribute(Mesh::ATTRIBUTE_COLOR)
    else {
        panic!("missing colors");
    };
    assert!(colors.iter().all(|c| c[3] == Tile::Stone.atlas_cell()));
}

#[test]
fn test_grass_tint_matches_along_shared_edges() {
    let noise = ochre_world::worldmap::test_terrain(3);
    let chunk_tint =
        |x: i64, z: i64| GrassTint::new(&noise, Vec3::new(x as f32, 0.0, z as f32) * 16.0, 1.0);
    let (here, east, south) = (chunk_tint(5, 5), chunk_tint(6, 5), chunk_tint(5, 6));
    for step in 0..=16 {
        let along = step as f32 - 0.5;
        assert!(here
            .at(Vec3::new(15.5, 0.0, along))
            .abs_diff_eq(east.at(Vec3::new(-0.5, 0.0, along)), 1e-5));
        assert!(here
            .at(Vec3::new(along, 0.0, 15.5))
            .abs_diff_eq(south.at(Vec3::new(along, 0.0, -0.5)), 1e-5));
    }

    // a LOD node four chunks across, its local units 4 blocks wide, starting at chunk (4, 4)
    let node = GrassTint::new(&noise, Vec3::new(64.0, 0.0, 64.0) + Vec3::splat(1.5), 4.0);
    assert!(node
        .at(Vec3::splat(-0.5))
        .abs_diff_eq(chunk_tint(4, 4).at(Vec3::splat(-0.5)), 1e-5));
    assert!(node
        .at(Vec3::splat(15.5))
        .abs_diff_eq(chunk_tint(7, 7).at(Vec3::splat(15.5)), 1e-5));
}

// The middle of a forest on the test world with this seed.
fn forest_at(seed: u32) -> bevy::math::I64Vec2 {
    ochre_world::worldmap::forest_spot(&ochre_world::worldmap::test_terrain(seed))
}

// The middle of open country with a few lone trees on the test world with this seed.
fn sparse_at(seed: u32) -> bevy::math::I64Vec2 {
    ochre_world::worldmap::tree_spot(&ochre_world::worldmap::test_terrain(seed), 4, 10)
}

// top-face sky light at each vertex, by position
fn top_lights(mesh: &Mesh) -> Vec<([f32; 3], f32)> {
    let Some(VertexAttributeValues::Float32x3(positions)) =
        mesh.attribute(Mesh::ATTRIBUTE_POSITION)
    else {
        panic!("missing positions");
    };
    let Some(VertexAttributeValues::Float32x3(normals)) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL)
    else {
        panic!("missing normals");
    };
    let Some(VertexAttributeValues::Float32x2(lights)) = mesh.attribute(Mesh::ATTRIBUTE_UV_1)
    else {
        panic!("missing light");
    };
    (0..positions.len())
        .filter(|&i| normals[i] == [0.0, 1.0, 0.0])
        .map(|i| (positions[i], lights[i][0]))
        .collect()
}

#[test]
fn test_open_ground_is_fully_lit() {
    let lights = top_lights(&mesh_chunk(floor_chunk()).opaque);
    assert!(!lights.is_empty() && lights.iter().all(|(_, l)| *l == 1.0));
}

#[test]
fn test_ground_under_a_roof_is_shaded_smoothly() {
    let mut chunk = floor_chunk();
    // a stone roof over x 4..12, z 4..12, three blocks up
    for x in 4..12 {
        for z in 4..12 {
            chunk.set_block_at(U16Vec3::new(x, 3, z), BlockType::Stone);
        }
    }
    let lights = top_lights(&mesh_chunk(chunk).opaque);
    let floor: Vec<_> = lights.iter().filter(|(p, _)| p[1] == 0.5).collect();
    let (darkest, _) = floor.iter().min_by(|a, b| a.1.total_cmp(&b.1)).unwrap();
    assert!(
        floor.iter().any(|(_, l)| *l == 1.0),
        "nowhere on the floor is in the open"
    );
    assert!(
        (4.5..=11.5).contains(&darkest[0]) && (4.5..=11.5).contains(&darkest[2]),
        "darkest at {darkest:?}"
    );
    let mut levels: Vec<f32> = floor.iter().map(|(_, l)| *l).collect();
    levels.sort_by(f32::total_cmp);
    levels.dedup();
    assert!(levels.len() >= 4, "light falls off in steps {levels:?}");
    // the roof's top is under open sky
    assert!(lights.iter().any(|(p, l)| p[1] == 3.5 && *l == 1.0));
}

#[test]
fn test_lod_ground_under_open_sky_is_lit_to_its_edges() {
    use bevy::math::I64Vec2;

    // a mountainside, where LOD skirts stand in for the solid ground beside the node
    let noise = ochre_world::worldmap::test_terrain(42);
    let p = (-40..40)
        .flat_map(|z| (-40..40).map(move |x| I64Vec2::new(x, z) * 256))
        .find(|p| {
            let h = noise.get(*p);
            // gentle enough that no cliff or ridge beside it throws shade on the node
            (100.0..250.0).contains(&(h - WATER_LEVEL as f64))
                && (-6..=6).all(|i| {
                    (-6..=6).all(|j| (noise.get(*p + I64Vec2::new(i * 8, j * 8)) - h).abs() < 4.0)
                })
        })
        .expect("no mountainside near the centre");
    let height = noise.get(p) as i64;
    let chunk = ChunkCoordinate(I64Vec3::new(
        p.x.div_euclid(16),
        height.div_euclid(16),
        p.y.div_euclid(16),
    ));
    let regions = Arc::new(RegionStore::new(noise));
    let key = crate::lod::LodKey::containing(chunk, 1);
    let mesh = super::lod::generate_lod_mesh(regions, key)
        .expect("no mesh")
        .opaque;
    let lights = top_lights(&mesh);
    assert!(!lights.is_empty());
    let dark: Vec<_> = lights.iter().filter(|(_, l)| *l < 1.0).collect();
    assert!(dark.is_empty(), "shaded under open sky: {dark:?}");
}

// whether each vertex draws its block's mean colour instead of its texture, see `world.wesl`
fn untextured_flags(mesh: &Mesh) -> Vec<f32> {
    let Some(VertexAttributeValues::Float32x2(lights)) = mesh.attribute(Mesh::ATTRIBUTE_UV_1)
    else {
        panic!("missing light");
    };
    lights.iter().map(|l| l[1]).collect()
}

#[test]
fn test_chunks_are_textured() {
    let flags = untextured_flags(&mesh_chunk(floor_chunk()).opaque);
    assert!(!flags.is_empty() && flags.iter().all(|&f| f == 0.0));
}

#[test]
fn test_lods_wider_than_four_blocks_are_untextured() {
    use bevy::math::I64Vec2;

    let noise = ochre_world::worldmap::test_terrain(42);
    let height = noise.get(I64Vec2::ZERO) as i64;
    let chunk = ChunkCoordinate(I64Vec3::new(0, height.div_euclid(16), 0));
    let regions = Arc::new(RegionStore::new(noise));
    let key = crate::lod::LodKey::containing(chunk, 5);
    assert!(key.cell_size() > super::buffers::BOX_TEXTURE_MAX_CELL);
    let flags = untextured_flags(
        &super::lod::generate_lod_mesh(regions, key)
            .expect("no mesh")
            .opaque,
    );
    // but for tree boxes, textured with their leaves' holes filled, which come last
    let ground = flags
        .iter()
        .take_while(|&&f| f == super::buffers::UNTEXTURED)
        .count();
    assert!(
        ground > 0
            && flags[ground..]
                .iter()
                .all(|&f| f == super::buffers::TEXTURED_SOLID)
    );
}

#[test]
fn test_tree_boxes_stay_textured_up_to_eight_wide_cells_with_a_tile_a_block() {
    let regions = Arc::new(RegionStore::new(ochre_world::worldmap::test_terrain(5)));
    let mut heights = regions.sampler();
    let trees = crate::generate::trees::trees_near(
        regions.terrain(),
        &mut heights,
        forest_at(5) - bevy::math::I64Vec2::splat(300),
        forest_at(5) + bevy::math::I64Vec2::splat(300),
    );
    let tree = &trees[0];
    let crown = tree.base + I64Vec3::Y * (tree.trunk - 1);
    let key = LodKey::containing(ChunkCoordinate(crown.div_euclid(I64Vec3::splat(16))), 3);
    assert!(key.cell_size() <= super::buffers::BOX_TEXTURE_MAX_CELL);
    let opaque = super::lod::generate_lod_mesh(regions, key)
        .expect("no mesh")
        .opaque;
    let flags = untextured_flags(&opaque);
    let VertexAttributeValues::Float32x2(uvs) = opaque.attribute(Mesh::ATTRIBUTE_UV_0).unwrap()
    else {
        panic!()
    };
    // boxes come last: textured, and counting more than one tile across a crown
    let boxed = flags.len() - flags.iter().rev().take_while(|&&f| f == 0.0).count();
    assert!(boxed < flags.len(), "no textured tree boxes");
    assert!(uvs[boxed..].iter().any(|uv| uv[0] > 3.0 || uv[1] > 3.0));
    assert!(flags[..boxed].iter().all(|&f| f == 1.0));
}

#[test]
fn test_lods_up_to_four_blocks_are_textured_with_the_texture_repeating_over_each_cell() {
    use bevy::math::I64Vec2;

    let noise = ochre_world::worldmap::test_terrain(42);
    let height = noise.get(I64Vec2::ZERO) as i64;
    let chunk = ChunkCoordinate(I64Vec3::new(0, height.div_euclid(16), 0));
    let regions = Arc::new(RegionStore::new(noise));
    let key = crate::lod::LodKey::containing(chunk, 2);
    assert_eq!(4, key.cell_size());
    let mesh = super::lod::generate_lod_mesh(regions, key)
        .expect("no mesh")
        .opaque;
    let flags = untextured_flags(&mesh);
    // grass sides mark their shade instead, see `MeshBuffers::tint_grass_sides`
    assert!(!flags.is_empty() && flags.iter().all(|&f| f <= 0.0));
    // a face one cell wide spans four tiles, so none comes out a tile wide
    let uvs = mesh.attribute(Mesh::ATTRIBUTE_UV_0).unwrap();
    let bevy::render::mesh::VertexAttributeValues::Float32x2(uvs) = uvs else {
        panic!()
    };
    let tiles = |axis: usize| uvs.iter().map(|uv| uv[axis]).fold(0.0f32, f32::max);
    assert!(tiles(0) >= 4.0 || tiles(1) >= 4.0);
}

#[test]
fn test_ambient_occlusion_stops_faces_merging() {
    let mut chunk = floor_chunk();
    chunk.set_block_at(U16Vec3::new(8, 1, 8), BlockType::Stone);
    // one quad per face of the floor and of the pillar would be 44; the shaded ring around it splits the top
    assert!(mesh_vertices(chunk) > 44);
}

// A floor with a wall, a step up onto a ledge and a lone block on it, shaded every which way.
fn walled_chunk() -> ChunkData {
    let mut chunk = floor_chunk();
    for x in 2..14 {
        chunk.set_block_at(U16Vec3::new(x, 1, 4), BlockType::Stone);
        chunk.set_block_at(U16Vec3::new(x, 2, 4), BlockType::Stone);
    }
    for x in 0..16 {
        for z in 10..16 {
            chunk.set_block_at(U16Vec3::new(x, 1, z), BlockType::Stone);
        }
    }
    chunk.set_block_at(U16Vec3::new(7, 2, 12), BlockType::Stone);
    chunk
}

// Each vertex's position, normal, colour and light.
fn shaded_vertices(mesh: &Mesh) -> Vec<(Vec3, Vec3, Vec3, f32)> {
    let positions = mesh
        .attribute(Mesh::ATTRIBUTE_POSITION)
        .unwrap()
        .as_float3()
        .unwrap();
    let normals = mesh
        .attribute(Mesh::ATTRIBUTE_NORMAL)
        .unwrap()
        .as_float3()
        .unwrap();
    let Some(VertexAttributeValues::Float32x4(colors)) = mesh.attribute(Mesh::ATTRIBUTE_COLOR)
    else {
        panic!()
    };
    let Some(VertexAttributeValues::Float32x2(lights)) = mesh.attribute(Mesh::ATTRIBUTE_UV_1)
    else {
        panic!()
    };
    (0..positions.len())
        .map(|i| {
            (
                Vec3::from(positions[i]),
                Vec3::from(normals[i]),
                Vec3::new(colors[i][0], colors[i][1], colors[i][2]),
                lights[i][0],
            )
        })
        .collect()
}

#[test]
fn test_merging_shaded_faces_leaves_their_shading_as_it_was() {
    let unmerged = {
        super::greedy::NO_MERGING.set(true);
        let mesh = mesh_chunk(walled_chunk()).opaque;
        super::greedy::NO_MERGING.set(false);
        mesh
    };
    let merged = mesh_chunk(walled_chunk()).opaque;
    assert!(merged.count_vertices() < unmerged.count_vertices());
    let vertices = shaded_vertices(&merged);
    let Some(Indices::U32(indices)) = merged.indices() else {
        panic!()
    };
    // every corner of every face drawn alone is shaded the same by the merged quad over it
    for (p, normal, color, light) in shaded_vertices(&unmerged) {
        let found = indices.chunks(3).find_map(|tri| {
            let [a, b, c] = [0, 1, 2].map(|k| vertices[tri[k] as usize]);
            if a.1 != normal || (p - a.0).dot(normal).abs() > 1e-4 {
                return None;
            }
            // barycentric weights of p in the triangle
            let (v0, v1, v2) = (b.0 - a.0, c.0 - a.0, p - a.0);
            let (d00, d01, d11, d20, d21) =
                (v0.dot(v0), v0.dot(v1), v1.dot(v1), v2.dot(v0), v2.dot(v1));
            let denom = d00 * d11 - d01 * d01;
            let (wb, wc) = (
                (d11 * d20 - d01 * d21) / denom,
                (d00 * d21 - d01 * d20) / denom,
            );
            let wa = 1.0 - wb - wc;
            (wa.min(wb).min(wc) > -1e-4).then(|| {
                (
                    a.2 * wa + b.2 * wb + c.2 * wc,
                    a.3 * wa + b.3 * wb + c.3 * wc,
                )
            })
        });
        let (found_color, found_light) =
            found.unwrap_or_else(|| panic!("nothing drawn at {p} facing {normal}"));
        assert!(
            (found_color - color).abs().max_element() < 2e-3,
            "at {p} facing {normal}: {found_color} for {color}"
        );
        assert!(
            (found_light - light).abs() < 1e-4,
            "at {p} facing {normal}: light {found_light} for {light}"
        );
    }
}

// Open air over ground far below the node, with `blocks` set, in cells of 4 blocks.
fn lod_with(blocks: &[(IVec3, BlockType)]) -> LodBlocks {
    let size = 16;
    let padded = (size + 2) as usize;
    let mut cells = vec![BlockType::Air; padded * padded * padded];
    for &(coord, block) in blocks {
        cells[LodBlocks::index(size, coord)] = block;
    }
    LodBlocks::new(size, cells, vec![0; padded * padded], WATER_LEVEL + 64, 4)
}

// a 2x2x2 cube of leaves from `min` up
fn leaf_cube(min: i32) -> Vec<(IVec3, BlockType)> {
    let mut cube = Vec::new();
    for x in min..min + 2 {
        for y in min..min + 2 {
            for z in min..min + 2 {
                cube.push((IVec3::new(x, y, z), BlockType::Leaves));
            }
        }
    }
    cube
}

// how many vertices lie on faces in the plane at `plane` across their normal's axis
fn vertices_in_plane(mesh: &Mesh, plane: f32) -> usize {
    let Some(VertexAttributeValues::Float32x3(positions)) =
        mesh.attribute(Mesh::ATTRIBUTE_POSITION)
    else {
        panic!("missing positions");
    };
    let Some(VertexAttributeValues::Float32x3(normals)) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL)
    else {
        panic!("missing normals");
    };
    positions
        .iter()
        .zip(normals)
        .filter(|(p, n)| (0..3).any(|k| n[k] != 0.0 && p[k] == plane))
        .count()
}

#[test]
fn test_untextured_leaves_hide_the_leaves_behind_them() {
    let mesh = mesh_lod(&lod_with(&leaf_cube(7)));
    // cells 7 and 8 meet at 7.5; the shell is at 6.5 and 8.5
    assert_eq!(0, vertices_in_plane(&mesh, 7.5));
    assert!(vertices_in_plane(&mesh, 6.5) > 0 && vertices_in_plane(&mesh, 8.5) > 0);
    // each side of the shell merges into one quad, a centre point and 8 round its border
    assert_eq!(6 * 9, mesh.count_vertices());
}

#[test]
fn test_untextured_leaves_still_show_other_blocks() {
    let mut blocks = leaf_cube(7);
    blocks.push((IVec3::new(6, 7, 7), BlockType::Log));
    let faces = faces(&mesh_lod(&lod_with(&blocks)));
    // the log's face into the leaves stays: only leaves hide leaves
    assert!(faces
        .iter()
        .any(|(n, vs)| *n == [1.0, 0.0, 0.0] && vs.iter().all(|v| v[0] == 6.5)));
}

#[test]
fn test_untextured_leaves_on_the_nodes_edge_keep_their_faces() {
    // past the edge the leaves may be drawn at another detail, with none just there
    let lod = lod_with(&[
        (IVec3::new(15, 4, 4), BlockType::Leaves),
        (IVec3::new(16, 4, 4), BlockType::Leaves),
    ]);
    let faces = faces(&mesh_lod(&lod));
    assert!(faces
        .iter()
        .any(|(n, vs)| *n == [1.0, 0.0, 0.0] && vs.iter().all(|v| v[0] == 15.5)));
}

fn leaf_cube_chunk() -> ChunkData {
    let mut chunk = ChunkData::default();
    for (coord, block) in leaf_cube(4) {
        chunk.set_block_at(coord.as_u16vec3(), block);
    }
    chunk
}

#[test]
fn test_full_detail_leaves_mesh_the_leaves_behind_them_apart() {
    // blocks 4 and 5 meet at 4.5, where the faces between them show through the holes
    let meshes = mesh_chunk(leaf_cube_chunk());
    let inner = meshes.inner_leaves.expect("faces between leaves");
    assert!(vertices_in_plane(&inner, 4.5) > 0);
    // and only those: the shell is drawn with the rest of the chunk
    assert_eq!(
        0,
        vertices_in_plane(&inner, 3.5) + vertices_in_plane(&inner, 5.5)
    );
    assert_eq!(0, vertices_in_plane(&meshes.opaque, 4.5));
    assert!(
        vertices_in_plane(&meshes.opaque, 3.5) > 0 && vertices_in_plane(&meshes.opaque, 5.5) > 0
    );
}

#[test]
fn test_leaves_alone_have_no_inner_faces() {
    let mut chunk = ChunkData::default();
    chunk.set_block_at(U16Vec3::new(4, 4, 4), BlockType::Leaves);
    chunk.set_block_at(U16Vec3::new(5, 4, 4), BlockType::Stone);
    let meshes = mesh_chunk(chunk);
    assert!(meshes.inner_leaves.is_none());
}

#[test]
fn test_leaves_on_the_chunks_edge_keep_their_faces_with_the_rest() {
    // past the edge the leaves may be drawn by a LOD node, with none just there
    let mut chunk = ChunkData::default();
    chunk.set_block_at(U16Vec3::new(15, 4, 4), BlockType::Leaves);
    let mut east = ChunkData::default();
    east.set_block_at(U16Vec3::new(0, 4, 4), BlockType::Leaves);
    let mut neighbours = vec![None; 26];
    neighbours[neighbor_26_index(1, 0, 0)] = Some(Arc::new(east));
    let meshes = generate_chunk_mesh(
        Arc::new(chunk),
        neighbours,
        ChunkCoordinate(I64Vec3::new(0, 40, 0)),
        &RegionStore::new(ochre_world::worldmap::test_terrain(3)),
        false,
    );
    assert!(meshes.inner_leaves.is_none());
    assert!(vertices_in_plane(&meshes.opaque, 15.5) > 0);
}

#[test]
fn test_full_detail_solid_leaves_hide_the_leaves_behind_them() {
    let coord = ChunkCoordinate(I64Vec3::new(0, 40, 0));
    let mesh = generate_chunk_mesh(
        Arc::new(leaf_cube_chunk()),
        vec![None; 26],
        coord,
        &RegionStore::new(ochre_world::worldmap::test_terrain(3)),
        true,
    );
    assert!(mesh.inner_leaves.is_none());
    let mesh = mesh.opaque;
    assert_eq!(0, vertices_in_plane(&mesh, 4.5));
    assert!(vertices_in_plane(&mesh, 3.5) > 0 && vertices_in_plane(&mesh, 5.5) > 0);
}

#[test]
fn test_lod_border_exposes_surface_cells_as_skirt() {
    let lod = flat_lod(8);
    // the border's top two solid cells read as open, so the edge draws a wall
    assert_eq!(BlockType::Air, lod.block(IVec3::new(-1, 7, 4)));
    assert_eq!(BlockType::Air, lod.block(IVec3::new(-1, 6, 4)));
    assert_eq!(BlockType::Air, lod.block(IVec3::new(4, 7, 16)));
    // deeper border cells and every interior cell read as they are
    assert_eq!(BlockType::Stone, lod.block(IVec3::new(-1, 5, 4)));
    assert_eq!(BlockType::Stone, lod.block(IVec3::new(4, 7, 4)));
    // below the node isn't a horizontal border, so no skirt
    assert_eq!(BlockType::Stone, lod.block(IVec3::new(4, -1, 4)));
}

#[test]
fn test_skirt_walls_are_lit_like_the_open_ground_they_hang_from() {
    // flat ground under open sky, cells a block across, and the cells beyond it solid
    let lod = flat_lod(8);
    let noise = ochre_world::worldmap::test_terrain(1);
    let tint = super::buffers::GrassTint::new(&noise, Vec3::ZERO, 1.0);
    let opacity = lod.cells.iter().map(BlockType::light_opacity).collect();
    let light = std::cell::LazyCell::new(|| {
        crate::generate::light::SkyLight::new(
            IVec3::splat(-1),
            IVec3::splat(18),
            crate::generate::light::MAX_LIGHT,
            opacity,
        )
    });
    let mesh = super::mesh::mesh_blocks(&lod, 1, style(true, false), &tint, &light).opaque;
    let Some(bevy::render::mesh::VertexAttributeValues::Float32x2(lights)) =
        mesh.attribute(Mesh::ATTRIBUTE_UV_1)
    else {
        panic!("no light on the mesh");
    };
    // the top and the four walls, each from a corner to a corner of the node
    assert!(lights.len() >= 4 * 5);
    let darkest = lights.iter().map(|l| l[0]).fold(f32::MAX, f32::min);
    assert!(
        darkest > 0.6,
        "a vertex is lit {darkest}, so a skirt wall is dark"
    );
}

#[test]
fn test_lod_skirt_hangs_from_the_nodes_own_ground_not_its_neighbours() {
    // ground reaching the top of the node, where there's no air above to find
    let size = 16;
    let padded = (size + 2) as usize;
    let mut cells = vec![BlockType::Air; padded * padded * padded];
    for x in -1..=size {
        for z in -1..=size {
            for y in -1..=size {
                cells[LodBlocks::index(size, IVec3::new(x, y, z))] = BlockType::Stone;
            }
        }
    }
    // the ground's top is in the node above, 3 cells over the top row
    let lod = LodBlocks::new(
        size,
        cells.clone(),
        vec![size as i64 + 3; padded * padded],
        0,
        1,
    );
    assert_eq!(BlockType::Stone, lod.block(IVec3::new(-1, 8, 4)));
    // ground ending in the top row: the cells within two of it have walls, though the top row is solid
    let lod = LodBlocks::new(size, cells, vec![size as i64 + 1; padded * padded], 0, 1);
    assert_eq!(BlockType::Air, lod.block(IVec3::new(-1, 16, 4)));
    assert_eq!(BlockType::Air, lod.block(IVec3::new(-1, 15, 4)));
    assert_eq!(BlockType::Stone, lod.block(IVec3::new(-1, 14, 4)));
}

#[test]
fn test_lod_water_surface_cells_over_shallow_ground_are_water() {
    let regions = Arc::new(RegionStore::new(ochre_world::worldmap::test_terrain(102)));
    let mut checked = 0;
    for level in [3u8, 4, 5] {
        for x in -6..6i64 {
            for z in -6..6i64 {
                let width = 1i64 << level;
                let key = LodKey {
                    level,
                    coord: I64Vec3::new(x, (WATER_LEVEL / 16).div_euclid(width), z),
                };
                let lod = LodBlocks::sample(&regions, key);
                for cx in 0..lod.size {
                    for cz in 0..lod.size {
                        let column = LodBlocks::column(lod.size, cx, cz);
                        // a river held above the sea has its own surface to fill to
                        if lod.tops[column] >= WATER_LEVEL || lod.water_tops[column] != WATER_LEVEL
                        {
                            continue;
                        }
                        for cy in 0..lod.size {
                            let bottom = lod.base_y + cy as i64 * lod.cell;
                            if bottom < WATER_LEVEL && WATER_LEVEL <= bottom + lod.cell {
                                assert!(!super::terrain::is_solid(lod.raw(IVec3::new(cx, cy, cz))));
                                checked += 1;
                            }
                        }
                    }
                }
            }
        }
    }
    assert!(checked > 1000);
}

#[test]
fn test_water_depth_on_lod_border_ignores_skirts() {
    // ground up to y = 3 everywhere, water from 4 to 7
    let mut lod = flat_lod(4);
    for x in -1..=16 {
        for z in -1..=16 {
            for y in 4..8 {
                lod.cells[LodBlocks::index(16, IVec3::new(x, y, z))] = BlockType::Water;
            }
        }
    }
    // the border's top solid cells read as water for the skirt, but don't add depth
    assert_eq!(BlockType::Water, lod.block(IVec3::new(-1, 3, 4)));
    assert_eq!(4, water_depth(&lod, IVec3::new(4, 7, 4)));
    assert_eq!(4, water_depth(&lod, IVec3::new(-1, 7, 4)));
}

#[test]
fn test_shallow_water_in_a_coarse_lod_cell_is_drawn_to_its_real_surface() {
    // cells of 4 blocks, ground up to y = 4 and water to y = 6: its surface is the cell's middle
    let mut lod = flat_lod(1);
    lod.cell = 4;
    lod.base_y = 0;
    lod.water_tops = vec![6; (16 + 2) * (16 + 2)];
    lod.cells[LodBlocks::index(16, IVec3::new(4, 1, 4))] = BlockType::Water;
    assert_eq!(Some(0.0), lod.find_water_surface_top(IVec3::new(4, 1, 4)));
    // a cell the water fills to the top, or with water over it, has none
    lod.water_tops = vec![8; (16 + 2) * (16 + 2)];
    assert_eq!(None, lod.find_water_surface_top(IVec3::new(4, 1, 4)));
    lod.water_tops = vec![6; (16 + 2) * (16 + 2)];
    lod.cells[LodBlocks::index(16, IVec3::new(4, 2, 4))] = BlockType::Water;
    assert_eq!(None, lod.find_water_surface_top(IVec3::new(4, 1, 4)));
}

#[test]
fn test_lod_block_clamps_far_probes_to_border() {
    let lod = flat_lod(8);
    assert_eq!(
        lod.block(IVec3::new(-1, 3, 4)),
        lod.block(IVec3::new(-3, 3, 4))
    );
}

#[test]
fn test_buried_chunk_has_no_geometry() {
    let mut stone = ChunkData::default();
    for (x, y, z) in
        (0..16).flat_map(|x| (0..16).flat_map(move |y| (0..16).map(move |z| (x, y, z))))
    {
        stone.set_block_at(U16Vec3::new(x, y, z), BlockType::Stone);
    }
    let stone = Arc::new(stone);
    let meshes = generate_chunk_mesh(
        stone.clone(),
        vec![Some(stone); 26],
        ChunkCoordinate(I64Vec3::ZERO),
        &test_regions(),
        false,
    );
    assert!(!meshes.has_geometry());
}

#[test]
fn test_side_faces_texture_the_right_way_up_merged_or_not() {
    let mut chunk = ChunkData::default();
    for x in 4..7 {
        for y in 4..6 {
            chunk.set_block_at(U16Vec3::new(x, y, 4), BlockType::Grass);
        }
    }
    let mesh = mesh_chunk(chunk).opaque;
    let (
        Some(VertexAttributeValues::Float32x3(positions)),
        Some(VertexAttributeValues::Float32x3(normals)),
        Some(VertexAttributeValues::Float32x2(uvs)),
    ) = (
        mesh.attribute(Mesh::ATTRIBUTE_POSITION),
        mesh.attribute(Mesh::ATTRIBUTE_NORMAL),
        mesh.attribute(Mesh::ATTRIBUTE_UV_0),
    )
    else {
        panic!("mesh lacks positions, normals or uvs");
    };
    // v counts blocks down from the wall's top on every side, so the tile's top row is at the top
    let mut sides = 0;
    for ((p, n), uv) in positions.iter().zip(normals).zip(uvs) {
        if n[1] == 0.0 {
            sides += 1;
            assert!(
                (p[1] + uv[1] - 5.5).abs() < 1e-4,
                "normal {n:?}: v {} at y {}",
                uv[1],
                p[1]
            );
        }
    }
    assert!(sides > 0);
}

#[test]
fn test_canopy_shade_darkens_toward_the_ground() {
    let padded = 18 * 18;
    let lod = LodBlocks {
        canopy_tops: vec![40; padded],
        ..flat_lod(8)
    };
    let at = |y: f32| lod.canopy_shade(Vec3::new(4.0, y, 4.0));
    // a leaf at the canopy's top is unshaded, and the foot of a canopy is the darkest
    assert_eq!(1.0, at(39.5));
    assert!((at(19.5) - (1.0 - super::canopy::CANOPY_SHADE)).abs() < 1e-5);
    assert!(at(30.0) < at(35.0) && at(35.0) < at(39.5));
    // below the span it stops getting darker
    assert_eq!(at(19.5), at(9.5));
}

#[test]
fn test_canopy_shade_leaves_bare_columns_and_other_lods_alone() {
    let padded = 18 * 18;
    let bare = LodBlocks {
        canopy_tops: vec![i64::MIN; padded],
        ..flat_lod(8)
    };
    assert_eq!(1.0, bare.canopy_shade(Vec3::new(4.0, 2.0, 4.0)));
    assert_eq!(1.0, flat_lod(8).canopy_shade(Vec3::new(4.0, 2.0, 4.0)));
}

#[test]
fn test_a_lone_tree_is_still_there_in_every_cell_under_sixteen_wide() {
    use crate::generate::trees::trees_near;
    use bevy::math::I64Vec2;
    let regions = Arc::new(RegionStore::new(ochre_world::worldmap::test_terrain(5)));
    let mut heights = regions.sampler();
    let trees = trees_near(
        regions.terrain(),
        &mut heights,
        sparse_at(5) - I64Vec2::splat(700),
        sparse_at(5) + I64Vec2::splat(700),
    );
    let near = |a: &crate::generate::trees::Tree, b: &crate::generate::trees::Tree| {
        (a.base.x - b.base.x).abs() < 70 && (a.base.z - b.base.z).abs() < 70
    };
    let tree = trees
        .iter()
        .find(|t| trees.iter().filter(|o| near(t, o)).count() == 1)
        .expect("no lone tree");
    let crown = tree.base + I64Vec3::Y * (tree.trunk - 1);
    for level in [1u8, 2, 3] {
        let key = LodKey::containing(ChunkCoordinate(crown.div_euclid(I64Vec3::splat(16))), level);
        let blocks = LodBlocks::sample(&regions, key);
        let cell = key.cell_size();
        let leaf_cells = blocks
            .cells
            .iter()
            .filter(|&&c| c == BlockType::Leaves)
            .count();
        let crown_boxes = blocks
            .boxes
            .iter()
            .filter(|b| b.block == BlockType::Leaves)
            .count();
        if cell < super::canopy::BOX_CROWN_CELL {
            assert!(
                leaf_cells > 0,
                "no leaf cells at level {level}, cells {cell} blocks wide"
            );
        } else {
            // a crown is boxes of its real size, never a cell's worth of cube, and fewer the farther out
            assert_eq!(0, leaf_cells);
            let wanted = tree.crown_boxes(super::canopy::crown_layers(cell));
            assert!(
                !wanted.is_empty() && crown_boxes >= wanted.len(),
                "{crown_boxes} crown boxes of {} at level {level}",
                wanted.len()
            );
            for (min, max) in &wanted {
                let size = (*max - *min).as_vec3();
                let found = blocks
                    .boxes
                    .iter()
                    .filter(|b| b.block == BlockType::Leaves)
                    .any(|b| ((b.max - b.min) * cell as f32 - size).abs().max_element() < 1e-3);
                assert!(found, "no crown box of {size:?} at level {level}");
            }
            let wanted = wanted
                .iter()
                .fold(Vec3::ZERO, |w, (min, max)| w.max((*max - *min).as_vec3()));
            assert!(
                wanted.x < 20.0,
                "crown {wanted:?} wider than a crown should be"
            );
        }
    }
}

#[test]
fn test_a_lone_trees_trunk_is_a_thin_box_in_cells_four_to_eight_wide() {
    use crate::generate::trees::trees_near;
    use bevy::math::I64Vec2;
    let regions = Arc::new(RegionStore::new(ochre_world::worldmap::test_terrain(5)));
    let mut heights = regions.sampler();
    let trees = trees_near(
        regions.terrain(),
        &mut heights,
        sparse_at(5) - I64Vec2::splat(700),
        sparse_at(5) + I64Vec2::splat(700),
    );
    let near = |a: &crate::generate::trees::Tree, b: &crate::generate::trees::Tree| {
        (a.base.x - b.base.x).abs() < 70 && (a.base.z - b.base.z).abs() < 70
    };
    let tree = trees
        .iter()
        .find(|t| trees.iter().filter(|o| near(t, o)).count() == 1)
        .expect("no lone tree");
    for level in [2u8, 3] {
        let key = LodKey::containing(
            ChunkCoordinate(tree.base.div_euclid(I64Vec3::splat(16))),
            level,
        );
        let blocks = LodBlocks::sample(&regions, key);
        let cell = key.cell_size() as f32;
        let at = |block: i64, min: i64| (block - min) as f32 / cell - 0.5;
        let (min_x, min_z) = (key.min_block().x, key.min_block().z);
        let trunk = blocks
            .boxes
            .iter()
            .filter(|b| b.block == BlockType::Log)
            .find(|t| {
                (t.min.x - at(tree.base.x, min_x)).abs() < 1e-4
                    && (t.min.z - at(tree.base.z, min_z)).abs() < 1e-4
                    && (t.max.x - t.min.x - tree.width() as f32 / cell).abs() < 1e-4
            });
        assert!(
            trunk.is_some(),
            "no trunk box at level {level}: {:?}",
            blocks.boxes
        );
        let trunk = trunk.unwrap();
        assert!(((trunk.max.y - trunk.min.y) * cell - tree.trunk as f32).abs() < 1e-3);
        // and no log cells, which would be a pillar several times too thick
        assert_eq!(
            0,
            blocks
                .cells
                .iter()
                .filter(|&&c| c == BlockType::Log)
                .count()
        );
        // meshed, the box is a few quads standing out of the ground's
        let lone = LodBlocks {
            boxes: vec![*trunk],
            ..LodBlocks::new(
                16,
                vec![BlockType::Air; 18 * 18 * 18],
                vec![0; 18 * 18],
                0,
                key.cell_size(),
            )
        };
        let tint = super::buffers::GrassTint::new(regions.terrain(), Vec3::ZERO, 1.0);
        let light = std::cell::LazyCell::new(|| {
            crate::generate::light::SkyLight::new(
                IVec3::splat(-1),
                IVec3::splat(18),
                crate::generate::light::MAX_LIGHT,
                vec![0; 18 * 18 * 18],
            )
        });
        let mesh =
            super::mesh::mesh_blocks(&lone, key.cell_size(), style(false, false), &tint, &light)
                .opaque;
        // its foot is in the ground and its top in the crown, so only its sides show
        assert_eq!(4 * 4, mesh.count_vertices());
    }
}

// The densest forest near the origin, and the trees in it.
fn forest(regions: &RegionStore) -> (I64Vec3, Vec<crate::generate::trees::Tree>) {
    use crate::generate::trees::trees_near;
    use bevy::math::I64Vec2;
    // the search is slow, and every caller's world is seed 5's
    static FOREST: std::sync::OnceLock<(I64Vec3, Vec<crate::generate::trees::Tree>)> =
        std::sync::OnceLock::new();
    assert_eq!(5, regions.terrain().seed());
    FOREST
        .get_or_init(|| {
            let mut heights = regions.sampler();
            let corner = (-4..4)
                .flat_map(|x| (-4..4).map(move |z| I64Vec2::new(x, z) * 1024))
                .max_by_key(|&c| trees_near(regions.terrain(), &mut heights, c, c + 255).len())
                .unwrap();
            let trees = trees_near(regions.terrain(), &mut heights, corner, corner + 255);
            let ground = heights.height(corner.x + 128, corner.y + 128) as i64;
            (I64Vec3::new(corner.x + 128, ground, corner.y + 128), trees)
        })
        .clone()
}

#[test]
fn test_far_forests_are_a_few_canopy_boxes_not_a_box_a_tree() {
    let regions = Arc::new(RegionStore::new(ochre_world::worldmap::test_terrain(5)));
    let (middle, trees) = forest(&regions);
    assert!(
        trees.len() > 250,
        "only {} trees in the densest forest",
        trees.len()
    );
    for level in [4u8, 5] {
        let key = LodKey::containing(
            ChunkCoordinate(middle.div_euclid(I64Vec3::splat(16))),
            level,
        );
        let blocks = LodBlocks::sample(&regions, key);
        let cell = key.cell_size();
        let span = 16 * cell;
        let min = key.min_block();
        let inside = trees
            .iter()
            .filter(|t| {
                (min.x..min.x + span).contains(&t.base.x)
                    && (min.z..min.z + span).contains(&t.base.z)
            })
            .count();
        let leaf_boxes: Vec<_> = blocks
            .boxes
            .iter()
            .filter(|b| b.block == BlockType::Leaves)
            .collect();
        assert!(!leaf_boxes.is_empty(), "no canopy at level {level}");
        // each about as costly as one of the boxes a lone tree's crown is, where every tree was
        // three, and lone trees at the forest's edge keep theirs: a forest of 35% density merges
        // fewer crowns than the old 55% did, so the boxes only have to be two thirds of the trees
        // (at the widest cells a thin forest's crowns stop merging, so there only that no tree
        // costs more than two boxes is asked)
        let allowed = if level == 4 {
            inside * 2 / 3
        } else {
            inside * 2
        };
        assert!(
            leaf_boxes.len() < allowed,
            "{} leaf boxes for {inside} trees at level {level}",
            leaf_boxes.len()
        );
        // the forest's tree tops are under the canopy's, within a step of its rounding
        let step = (cell / super::canopy::CANOPY_STEPS) as f32;
        let highest = leaf_boxes
            .iter()
            .map(|b| min.y as f32 + (b.max.y + 0.5) * cell as f32)
            .fold(f32::MIN, f32::max);
        let corner = bevy::math::I64Vec2::new(min.x, min.z);
        let node_trees = crate::generate::trees::trees_near(
            regions.terrain(),
            &mut regions.sampler(),
            corner,
            corner + span,
        );
        let tallest = node_trees
            .iter()
            .map(|t| t.crown_top() as f32)
            .fold(f32::MIN, f32::max);
        assert!(
            highest <= tallest + step,
            "canopy at {highest}, trees up to {tallest} at level {level}"
        );
        // the ground under it is darkened by how much of it is under crowns
        assert!(blocks.forest.iter().any(|&f| f > 0.8));
    }
}

#[test]
fn test_canopy_is_no_bigger_than_the_crowns_under_it() {
    let regions = Arc::new(RegionStore::new(ochre_world::worldmap::test_terrain(5)));
    let (middle, _) = forest(&regions);
    for level in [4u8, 5] {
        let key = LodKey::containing(
            ChunkCoordinate(middle.div_euclid(I64Vec3::splat(16))),
            level,
        );
        let blocks = LodBlocks::sample(&regions, key);
        let canopy: Vec<_> = blocks
            .boxes
            .iter()
            .filter(|b| b.block == BlockType::Leaves)
            .collect();
        let area = |b: &&super::source::TreeBox| (b.max.x - b.min.x) * (b.max.z - b.min.z);
        let drawn: f32 = canopy.iter().map(area).sum();
        // what's under crowns over the ground this node draws
        let (min, span) = (key.min_block().y as f32, 16.0 * key.cell_size() as f32);
        let mut under = 0.0;
        for x in 0..16 {
            for z in 0..16 {
                let column = LodBlocks::column(16, x, z);
                if (min..min + span).contains(&blocks.ground_heights[column]) {
                    under += blocks.forest[column];
                }
            }
        }
        // its edges are drawn in to the crowns, so what's extra is the gaps between them inside
        // it, more of them the wider the columns
        assert!(
            drawn <= under * 1.4,
            "canopy {drawn} over {under} under crowns at level {level}"
        );
    }
}

#[test]
fn test_trees_vary_in_colour_the_same_at_every_level() {
    use crate::generate::trees::TreeTints;
    let regions = Arc::new(RegionStore::new(ochre_world::worldmap::test_terrain(5)));
    let (_, trees) = forest(&regions);
    let tints: Vec<Vec3> = trees.iter().map(|t| t.tint()).collect();
    let brightness = |t: &Vec3| t.x + t.y + t.z;
    let (dim, bright) = tints.iter().fold((f32::MAX, f32::MIN), |(lo, hi), t| {
        (lo.min(brightness(t)), hi.max(brightness(t)))
    });
    assert!(bright - dim > 0.4, "tints only span {dim}..{bright}");
    assert!(
        tints
            .iter()
            .all(|t| t.min_element() > 0.6 && t.max_element() < 1.5),
        "a tint strays too far"
    );
    // leaves take their own tree's tint, at full detail and in coarser cells
    let tree = &trees[0];
    let trunk_top = tree.base + I64Vec3::Y * tree.trunk;
    for cell in [1, 2] {
        let origin = trunk_top.div_euclid(I64Vec3::splat(16 * cell)) * 16 * cell;
        let at = (trunk_top - origin)
            .div_euclid(I64Vec3::splat(cell))
            .as_ivec3();
        assert_eq!(
            tree.tint(),
            TreeTints::new(&trees, origin, cell).at(at),
            "at cells {cell} wide"
        );
    }
}

#[test]
fn test_leaves_of_differently_tinted_trees_dont_merge() {
    let mut chunk = leaf_cube_chunk();
    let regions = Arc::new(RegionStore::new(ochre_world::worldmap::test_terrain(5)));
    let (_, trees) = forest(&regions);
    // two trees, one each side of the cube, so it's split down the middle
    let thin: Vec<_> = trees.iter().filter(|t| t.width() == 1).copied().collect();
    let mut pair = vec![thin[0], thin[1]];
    pair[0].base = I64Vec3::new(4, 0, 4);
    pair[1].base = I64Vec3::new(5, 0, 4);
    assert_ne!(pair[0].tint(), pair[1].tint());
    chunk.trees = crate::generate::trees::TreeTints::new(&pair, I64Vec3::ZERO, 1);
    let mesh = mesh_chunk(chunk).opaque;
    let Some(VertexAttributeValues::Float32x4(colors)) = mesh.attribute(Mesh::ATTRIBUTE_COLOR)
    else {
        panic!()
    };
    let leaf = Tile::Leaves.atlas_cell();
    let mut shades: Vec<[u32; 3]> = colors
        .iter()
        .filter(|c| c[3] == leaf)
        .map(|c| [c[0], c[1], c[2]].map(|v| (v * 1000.0) as u32))
        .collect();
    shades.sort();
    shades.dedup();
    assert!(shades.len() >= 2, "leaves all one shade: {shades:?}");
}

#[test]
fn test_forest_tints_the_ground_darker_the_more_of_it_trees_cover() {
    let padded = 18 * 18;
    let mut forest = vec![0.0; padded];
    forest[LodBlocks::column(16, 4, 4)] = 0.5;
    forest[LodBlocks::column(16, 8, 8)] = 1.0;
    let lod = LodBlocks {
        forest,
        ..flat_lod(8)
    };
    let at = |x: f32, z: f32| lod.forest_shade(Vec3::new(x, 7.5, z));
    assert_eq!(1.0, at(1.0, 1.0));
    assert!(at(4.0, 4.0) < 1.0 && at(8.0, 8.0) < at(4.0, 4.0));
    assert!((at(8.0, 8.0) - super::canopy::FOREST_DARKEN).abs() < 1e-5);
    // real trees and bare ground leave the ground as it was
    assert_eq!(1.0, flat_lod(8).forest_shade(Vec3::new(4.0, 7.5, 4.0)));
}

#[test]
fn test_cells_wider_than_real_trees_tint_forest_instead_of_drawing_leaves() {
    use crate::generate::trees::trees_near;
    use bevy::math::I64Vec2;
    let regions = Arc::new(RegionStore::new(ochre_world::worldmap::test_terrain(5)));
    let mut heights = regions.sampler();
    let trees = trees_near(
        regions.terrain(),
        &mut heights,
        forest_at(5) - I64Vec2::splat(300),
        forest_at(5) + I64Vec2::splat(300),
    );
    let tree = trees.first().expect("no trees");
    let key = LodKey::containing(ChunkCoordinate(tree.base.div_euclid(I64Vec3::splat(16))), 6);
    assert!(key.cell_size() > super::canopy::TREE_MAX_CELL);
    let blocks = LodBlocks::sample(&regions, key);
    assert_eq!(
        0,
        blocks
            .cells
            .iter()
            .filter(|&&c| c == BlockType::Leaves || c == BlockType::Log)
            .count()
    );
    assert!(blocks.boxes.is_empty());
    assert!(
        blocks.forest.iter().any(|&f| f > 0.0)
            && blocks.forest.iter().all(|&f| (0.0..=1.0).contains(&f))
    );
}

type Spots = Vec<(i64, i64)>;

// Where trees grow within a node's own area, as full detail decides, and as the node's grid does.
fn tree_spots(regions: &Arc<RegionStore>, key: LodKey) -> (Spots, Spots) {
    use crate::generate::trees::trees_near;
    use bevy::math::I64Vec2;
    let (cell, base, size) = (key.cell_size(), key.min_block(), CHUNK_SIZE as i32);
    let span = (size + 1) as i64 * cell;
    let min = I64Vec2::new(base.x, base.z) - cell;
    let own = |t: &crate::generate::trees::Tree| {
        (base.x..base.x + size as i64 * cell).contains(&t.base.x)
            && (base.z..base.z + size as i64 * cell).contains(&t.base.z)
    };
    let mut heights = regions.sampler();
    let mut exact: Vec<_> = trees_near(regions.terrain(), &mut heights, min, min + span + cell)
        .iter()
        .filter(|t| own(t))
        .map(|t| (t.base.x, t.base.z))
        .collect();
    let grid = regions.column_grid(key);
    let mut ground = super::lod::GridGround {
        grid: &grid,
        noise: regions.terrain(),
        base: I64Vec2::new(base.x, base.z),
        cell,
        size,
        last: None,
    };
    let mut coarse: Vec<_> = trees_near(regions.terrain(), &mut ground, min, min + span + cell)
        .iter()
        .filter(|t| own(t))
        .map(|t| (t.base.x, t.base.z))
        .collect();
    exact.sort();
    coarse.sort();
    (exact, coarse)
}

#[test]
fn test_every_level_grows_the_trees_full_detail_does() {
    use bevy::math::I64Vec2;
    let regions = Arc::new(RegionStore::new(ochre_world::worldmap::test_terrain(5)));
    for level in [2u8, 3, 4, 5] {
        // nodes over a forested stretch, found from the full-detail trees
        let mut heights = regions.sampler();
        let trees = crate::generate::trees::trees_near(
            regions.terrain(),
            &mut heights,
            forest_at(5) - I64Vec2::splat(500),
            forest_at(5) + I64Vec2::splat(500),
        );
        let mut checked = 0;
        for tree in trees.iter().step_by(trees.len() / 8 + 1) {
            let key = LodKey::containing(
                ChunkCoordinate(tree.base.div_euclid(I64Vec3::splat(16))),
                level,
            );
            let (exact, coarse) = tree_spots(&regions, key);
            assert_eq!(exact, coarse, "level {level} node {key:?}");
            checked += exact.len();
        }
        assert!(checked > 0, "no trees checked at level {level}");
    }
}

// Triangles of a node's opaque mesh in world space, placed as `lod_transform` does.
fn world_triangles(regions: &Arc<RegionStore>, key: LodKey) -> Vec<[Vec3; 3]> {
    let Some(meshes) = super::lod::generate_lod_mesh(regions.clone(), key) else {
        return Vec::new();
    };
    let cell = key.cell_size() as f32;
    let origin = key.min_block().as_vec3() + Vec3::splat(0.5 * cell - 0.5);
    let positions = meshes
        .opaque
        .attribute(Mesh::ATTRIBUTE_POSITION)
        .unwrap()
        .as_float3()
        .unwrap();
    let Some(Indices::U32(indices)) = meshes.opaque.indices() else {
        return Vec::new();
    };
    indices
        .chunks(3)
        .map(|t| [0, 1, 2].map(|k| origin + Vec3::from(positions[t[k] as usize]) * cell))
        .collect()
}

// Whether the ray from `from` along `direction` meets the triangle, either side.
fn ray_hits(from: Vec3, direction: Vec3, [a, b, c]: [Vec3; 3]) -> bool {
    let (e1, e2) = (b - a, c - a);
    let p = direction.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-9 {
        return false;
    }
    let t = from - a;
    let u = t.dot(p) / det;
    let q = t.cross(e1);
    let v = direction.dot(q) / det;
    u >= 0.0 && v >= 0.0 && u + v <= 1.0 && e2.dot(q) / det > 0.0
}

// How many oblique rays down across the seam between a node and its coarser neighbour pass
// through every triangle of both, seeing the sky through a crack.
fn seam_leaks(regions: &Arc<RegionStore>, fine: LodKey, coarse: LodKey) -> (usize, usize) {
    // the nodes above and below too, where the ground crosses into them
    let mut triangles = Vec::new();
    for dy in -1..=1 {
        for key in [fine, coarse] {
            triangles.extend(world_triangles(
                regions,
                LodKey {
                    coord: key.coord + I64Vec3::new(0, dy, 0),
                    ..key
                },
            ));
        }
    }
    let seam_x = coarse.min_block().x as f32;
    let (z0, z1) = (
        fine.min_block().z as f32,
        (fine.min_block().z + fine.width() * 16) as f32,
    );
    let mut sampler = regions.sampler();
    let (mut rays, mut leaks) = (0, 0);
    for direction in [
        Vec3::new(0.35, -1.0, 0.1),
        Vec3::new(-0.35, -1.0, 0.1),
        Vec3::new(0.0, -1.0, 0.45),
        Vec3::new(0.2, -1.0, -0.3),
    ] {
        let direction = direction.normalize();
        // clear of the node's z edges, past which the next node isn't loaded here
        let mut z = z0 + 8.713;
        while z < z1 - 8.0 {
            for dx in [-2.7f32, -1.77, -0.93, 0.21, 0.83, 1.61, 2.9] {
                let x = seam_x + dx;
                let y = sampler.height(x as i64, z as i64) as f32;
                // from well above any hill the ray might otherwise start inside of
                let from = Vec3::new(x, y, z) - direction * (300.0 / -direction.y);
                rays += 1;
                if !triangles.iter().any(|&t| ray_hits(from, direction, t)) {
                    leaks += 1;
                }
            }
            z += 2.07;
        }
    }
    (leaks, rays)
}

#[test]
fn test_nothing_is_seen_through_a_seam_between_lod_levels() {
    let regions = Arc::new(RegionStore::new(ochre_world::worldmap::test_terrain(5)));
    let mut sampler = regions.sampler();
    // stretches of land a few thousand blocks apart, with a fine node beside a coarser one
    let spots: Vec<(i64, i64)> = (0..8)
        .map(|i| {
            (
                -3000 + (i % 4) * 1900 + (i * 137) % 400,
                -3000 + (i / 4) * 1900 + (i * 211) % 400,
            )
        })
        // moved east, off the sea, to land a few blocks above it
        .filter_map(|(x, z)| {
            (0..400)
                .map(|step| x + step * 64)
                // not up in the high ranges (over 640 blocks), where this harness does not load everything below
                // the surface that the seams' rays pass through: a gap to come back to
                .find(|&x| (WATER_LEVEL as f64 + 3.0..640.0).contains(&sampler.height(x, z)))
                .map(|x| (x, z))
        })
        .collect();
    for fine_level in 1..=4u8 {
        let (mut leaks, mut rays) = (0, 0);
        for &(bx, bz) in &spots {
            let h = sampler.height(bx, bz) as i64;
            let coarse = LodKey::containing(
                ChunkCoordinate(I64Vec3::new(bx, h, bz).div_euclid(I64Vec3::splat(16))),
                fine_level + 1,
            );
            let fine_chunk = I64Vec3::new(coarse.min_block().x - 8, h, coarse.min_block().z + 8)
                .div_euclid(I64Vec3::splat(16));
            let fine = LodKey::containing(ChunkCoordinate(fine_chunk), fine_level);
            let (l, r) = seam_leaks(&regions, fine, coarse);
            leaks += l;
            rays += r;
        }
        assert_eq!(
            0,
            leaks,
            "{leaks} of {rays} rays slipped through seams between levels {fine_level} and {}",
            fine_level + 1
        );
    }
}

#[test]
fn test_lod_non_air_only_covers_interior() {
    let lod = flat_lod(8);
    let cells = lod.non_air();
    assert_eq!(16 * 16 * 8, cells.len());
    assert!(cells
        .iter()
        .all(|(c, _)| c.min_element() >= 0 && c.max_element() < 16));
}

// The block at world (x, y, z) from a freshly generated chunk.
fn block_at(regions: &Arc<RegionStore>, x: i64, y: i64, z: i64) -> BlockType {
    let coord = ChunkCoordinate(I64Vec3::new(x, y, z).div_euclid(I64Vec3::splat(16)));
    generate_chunk(regions.clone(), coord).get_block_at(U16Vec3::new(
        x.rem_euclid(16) as u16,
        y.rem_euclid(16) as u16,
        z.rem_euclid(16) as u16,
    ))
}

// Cell centres of the test world's map where `kind` water flows or sits, and the sample there.
fn wet_columns(kind: ochre_base::terrain_source::WaterKind) -> Vec<(i64, i64)> {
    use ochre_base::terrain_source::TerrainSource;
    let terrain = ochre_world::worldmap::test_terrain(5);
    let map = terrain.map();
    let mut found = Vec::new();
    'cells: for cz in 0..map.dim() {
        for cx in 0..map.dim() {
            let (x, z) = map.cell_centre(cx, cz);
            let (x, z) = (x.round(), z.round());
            let sample = terrain.sample(x, z);
            let depth = sample.water_surface - sample.height as f32;
            if sample.water_kind == kind
                && sample.water_surface > WATER_LEVEL as f32 + 3.0
                && depth > 2.0
            {
                found.push((x as i64, z as i64));
                if found.len() >= 6 {
                    break 'cells;
                }
            }
        }
    }
    found
}

#[test]
fn test_lakes_and_rivers_above_the_sea_are_filled_to_their_own_surface() {
    use ochre_base::terrain_source::{TerrainSource, WaterKind};
    let terrain = ochre_world::worldmap::test_terrain(5);
    let regions = Arc::new(RegionStore::new(terrain.clone()));
    for kind in [WaterKind::Lake, WaterKind::River] {
        let columns = wet_columns(kind);
        assert!(
            !columns.is_empty(),
            "no {kind:?} above the sea on the test world"
        );
        for (x, z) in columns {
            let sample = terrain.sample(x as f64, z as f64);
            // the column's own water, at whole blocks, as chunk generation takes it
            let top = regions.sampler().water_top(x, z);
            let ground = regions.get_height(x, z).round() as i64;
            assert!(
                top > ground,
                "{kind:?} at ({x}, {z}): water {top} over ground {ground}"
            );
            assert!(top > WATER_LEVEL, "{sample:?}");
            let water = |y| block_at(&regions, x, y, z);
            assert_eq!(BlockType::Air, water(top + 1), "{kind:?} at ({x}, {z})");
            // the top layer may be ice in the cold, but never air, and under it is water
            assert!(matches!(water(top - 1), BlockType::Water | BlockType::Ice));
            assert_eq!(BlockType::Water, water(top - 1 - 4.min(top - ground - 1)));
            // sand, or water where a gorge's cliffs were cut open under it and flooded
            assert!(
                matches!(water(ground - 1), BlockType::Sand | BlockType::Water),
                "the bed at ({x}, {z}) is {:?}",
                water(ground - 1)
            );
        }
    }
}

#[test]
fn test_ground_well_under_a_lake_is_under_its_water() {
    use ochre_base::terrain_source::TerrainSource;
    // seed 10 has mountain lakes whose terraces were cut dry pits into their floors
    for seed in [5, 10] {
        let terrain = ochre_world::worldmap::test_terrain(seed);
        let map = terrain.map();
        let mut dry = Vec::new();
        let lake = |cx: usize, cz: usize| map.water_kind[map.index(cx, cz)] == 2;
        for cz in 1..map.dim() - 1 {
            for cx in 1..map.dim() - 1 {
                // a lake cell with lake all round it
                if !(cz - 1..=cz + 1).all(|z| (cx - 1..=cx + 1).all(|x| lake(x, z))) {
                    continue;
                }
                let (x, z) = map.cell_centre(cx, cz);
                let level = map.water_surface[map.index(cx, cz)];
                let sample = terrain.sample(x, z);
                if (map.height_at(x, z) as f64) < level as f64
                    && sample.height < level.floor() as f64 - 1.0
                    && sample.water_surface == 0.0
                {
                    dry.push((x, z, sample.height, level));
                }
            }
        }
        assert!(dry.is_empty(), "seed {seed}: dry ground in lakes {dry:?}");
    }
}

#[test]
fn test_ground_is_soil_over_stone_over_compacted_stone_by_depth_below_the_surface() {
    use super::terrain::{COMPACTED_DEPTH, SOIL_DEPTH};
    let top = 600;
    let at = |block, depth| under_grass(block, top - 1 - depth, top, 1, false);
    assert_eq!(BlockType::Grass, at(BlockType::Grass, 0));
    for depth in 1..SOIL_DEPTH {
        assert_eq!(
            BlockType::Dirt,
            at(BlockType::Grass, depth),
            "depth {depth}"
        );
        assert_eq!(BlockType::Sand, at(BlockType::Sand, depth));
    }
    for depth in SOIL_DEPTH..COMPACTED_DEPTH {
        assert_eq!(
            BlockType::Stone,
            at(BlockType::Grass, depth),
            "depth {depth}"
        );
        assert_eq!(BlockType::Stone, at(BlockType::Sand, depth));
        assert_eq!(BlockType::Stone, at(BlockType::Stone, depth));
    }
    for depth in [COMPACTED_DEPTH, COMPACTED_DEPTH + 1, 40] {
        for block in [
            BlockType::Grass,
            BlockType::Sand,
            BlockType::Stone,
            BlockType::Snow,
        ] {
            assert_eq!(
                BlockType::CompactedStone,
                at(block, depth),
                "{block:?} {depth}"
            );
        }
    }
    // water is never packed, and wide cells keep to the surface's block
    assert_eq!(BlockType::Water, at(BlockType::Water, 9));
    // snow lies deep: full blocks of it, stone after, and packed stone only under that
    let snow_at = |block, depth| under_grass(block, top - 1 - depth, top, 1, true);
    for depth in 1..super::terrain::SNOW_DEPTH {
        assert_eq!(
            BlockType::Snow,
            snow_at(BlockType::Snow, depth),
            "depth {depth}"
        );
    }
    assert_eq!(
        BlockType::Stone,
        snow_at(BlockType::Snow, super::terrain::SNOW_DEPTH)
    );
    assert_eq!(
        BlockType::CompactedStone,
        snow_at(BlockType::Stone, super::terrain::SNOW_COMPACTED_DEPTH)
    );
    assert_eq!(
        BlockType::Grass,
        under_grass(BlockType::Grass, top - 20, top, 4, false)
    );
}

#[test]
fn test_compacted_stone_follows_the_local_surface_not_a_fixed_height() {
    use super::terrain::COMPACTED_DEPTH;
    let regions = Arc::new(RegionStore::new(ochre_world::worldmap::test_terrain(5)));
    let terrain = ochre_world::worldmap::test_terrain(5);
    let map = terrain.map();
    // dry cells a few blocks or a few hundred above the sea, spread over the world
    let columns: Vec<(i64, i64)> = (0..map.dim())
        .step_by(37)
        .flat_map(|cz| (0..map.dim()).step_by(41).map(move |cx| (cx, cz)))
        .filter(|&(cx, cz)| {
            let i = map.index(cx, cz);
            map.water_kind[i] == 0 && map.height[i] > WATER_LEVEL as f32 + 6.0
        })
        .map(|(cx, cz)| {
            let (x, z) = map.cell_centre(cx, cz);
            (x as i64, z as i64)
        })
        .collect();
    let mut seen = 0;
    for (x, z) in columns {
        // cliffs are cut into steep ground, which moves its surface from the height
        let (gx, gz) = regions.sampler().gradient(x, z);
        if ochre_base::relief::cliff_reach(gx.abs() + gz.abs()) > 0.0 {
            continue;
        }
        seen += 1;
        let ground = regions.get_height(x, z).round() as i64;
        let below = |depth: i64| block_at(&regions, x, ground - 1 - depth, z);
        // under snow the packed stone begins deeper, beneath the snow's own depth
        let packed = if below(0) == BlockType::Snow {
            super::terrain::SNOW_COMPACTED_DEPTH
        } else {
            COMPACTED_DEPTH
        };
        assert_eq!(
            BlockType::CompactedStone,
            below(packed),
            "({x}, {z}) at {ground}"
        );
        assert_ne!(BlockType::CompactedStone, below(packed - 1));
    }
    assert!(seen >= 2, "only {seen} dry columns found");
}

// Blocks in the chunk at (`cx`, `cy`, `cz`) with air under a solid block in the same column, and
// how many columns in it are steep enough for cliffs.
fn overhangs(regions: &Arc<RegionStore>, (cx, cy, cz): (i64, i64, i64)) -> (usize, usize) {
    use ochre_base::relief::cliff_reach;
    let chunk = generate_chunk(regions.clone(), ChunkCoordinate(I64Vec3::new(cx, cy, cz)));
    let mut heights = regions.sampler();
    let (mut caves, mut steep) = (0, 0);
    for x in 0..16u16 {
        for z in 0..16u16 {
            let (wx, wz) = (cx * 16 + x as i64, cz * 16 + z as i64);
            let (gx, gz) = heights.gradient(wx, wz);
            steep += (cliff_reach(gx.abs() + gz.abs()) > 0.0) as usize;
            for y in 0..15u16 {
                let (below, above) = (
                    chunk.get_block_at(U16Vec3::new(x, y, z)),
                    chunk.get_block_at(U16Vec3::new(x, y + 1, z)),
                );
                caves += (below == BlockType::Air && super::terrain::is_solid(above)) as usize;
            }
        }
    }
    (caves, steep)
}

#[test]
fn test_steep_ground_gets_overhangs_and_gentle_ground_gets_none() {
    let terrain = ochre_world::worldmap::test_terrain(5);
    let regions = Arc::new(RegionStore::new(terrain.clone()));
    let map = terrain.map();
    let (mut steep_caves, mut steep_columns, mut gentle_caves, mut gentle_chunks) = (0, 0, 0, 0);
    for cz in (0..map.dim()).step_by(3) {
        for cx in (0..map.dim()).step_by(3) {
            let i = map.index(cx, cz);
            if map.water_kind[i] != 0 || map.height[i] < WATER_LEVEL as f32 + 6.0 {
                continue;
            }
            let (x, z) = map.cell_centre(cx, cz);
            let (x, z) = (x as i64, z as i64);
            let ground = regions.get_height(x, z).round() as i64;
            let at = (
                x.div_euclid(16),
                (ground - 2).div_euclid(16),
                z.div_euclid(16),
            );
            let (caves, steep) = overhangs(&regions, at);
            if steep >= 8 {
                steep_caves += caves;
                steep_columns += steep;
            } else if steep == 0 && gentle_chunks < 30 {
                gentle_caves += caves;
                gentle_chunks += 1;
            }
        }
    }
    assert!(steep_columns > 0, "no steep ground on the test world");
    assert!(
        steep_caves > 0,
        "no overhangs on {steep_columns} steep columns"
    );
    assert_eq!(0, gentle_caves, "overhangs on gentle ground");
}

#[test]
fn test_cliffs_are_cut_before_the_water_fills_what_they_open() {
    use super::terrain::cut_cliff;
    let sea = WATER_LEVEL;
    // ground cut away under the water floods, and over it is open air
    assert_eq!(
        BlockType::Water,
        cut_cliff(BlockType::Stone, false, sea - 10, 0.5, sea)
    );
    assert_eq!(
        BlockType::Air,
        cut_cliff(BlockType::Stone, false, sea + 3, 0.5, sea)
    );
    // the top layer of a frozen sea freezes in the cut too
    assert_eq!(
        BlockType::Ice,
        cut_cliff(BlockType::Stone, false, sea - 1, 0.0, sea)
    );
    // rock builds up into air but never into water, and what was already right is left alone
    assert_eq!(
        BlockType::Stone,
        cut_cliff(BlockType::Air, true, sea + 2, 0.5, sea)
    );
    assert_eq!(
        BlockType::Water,
        cut_cliff(BlockType::Water, true, sea - 2, 0.5, sea)
    );
    assert_eq!(
        BlockType::Grass,
        cut_cliff(BlockType::Grass, true, sea + 2, 0.5, sea)
    );
    assert_eq!(
        BlockType::Water,
        cut_cliff(BlockType::Water, false, sea - 2, 0.5, sea)
    );
}

#[test]
fn test_rivers_leave_no_dry_crevice_beside_their_water() {
    use ochre_base::terrain_source::{TerrainSource, WaterKind};
    let terrain = ochre_world::worldmap::test_terrain(5);
    let columns = wet_columns(WaterKind::River);
    assert!(
        !columns.is_empty(),
        "no river above the sea on the test world"
    );
    let mut checked = 0;
    for (x, z) in columns {
        let here = terrain.sample(x as f64, z as f64);
        // across the river each way: dry ground beside the water is never lower than the water
        for (dx, dz) in [(1i64, 0i64), (0, 1)] {
            for step in 1..=24 {
                for sign in [-1i64, 1] {
                    let (px, pz) = (x + sign * dx * step, z + sign * dz * step);
                    let s = terrain.sample(px as f64, pz as f64);
                    if s.water_surface == 0.0 && s.height < here.water_surface as f64 - 1.5 {
                        // lower than this river's water: only fine if it has fallen away
                        // downhill, out of the river's banks, which is more than a bank wide
                        let near_water = (1..=3).any(|k| {
                            let q = terrain
                                .sample((px - sign * dx * k) as f64, (pz - sign * dz * k) as f64);
                            q.water_surface > 0.0 && s.height < q.water_surface as f64 - 1.5
                        });
                        assert!(
                            !near_water,
                            "dry ground {:.1} lower than water {:.1} right beside it at ({px}, {pz})",
                            s.height, here.water_surface
                        );
                    }
                    checked += 1;
                }
            }
        }
    }
    assert!(checked > 100);
}

#[test]
fn test_a_stream_joining_a_river_has_water_all_the_way_to_it() {
    use ochre_base::terrain_source::{TerrainSource, WaterKind};
    use ochre_world::worldmap::{WaterCell, NEIGHBOURS, NO_FLOW};
    // bigger worlds than the usual test one, which has too few rivers that join
    let (mut checked, mut broken) = (0, Vec::new());
    for seed in [7, 1, 2] {
        let terrain = ochre_world::worldmap::WorldTerrain::new(std::sync::Arc::new(
            ochre_world::worldmap::generate(
                seed,
                ochre_world::worldmap::WorldConfig {
                    size: 32_768,
                    map_cell: 32,
                },
            ),
        ));
        let map = terrain.map();
        let dim = map.dim();
        for i in 0..map.height.len() {
            if map.water_kind[i] != WaterCell::River as u8 || map.down[i] == NO_FLOW {
                continue;
            }
            let (dx, dz) = NEIGHBOURS[map.down[i] as usize];
            let j = ((i / dim) as i64 + dz) as usize * dim + ((i % dim) as i64 + dx) as usize;
            // a stream a few blocks wide running into a river at least twice as wide
            let joins = map.water_kind[j] == WaterCell::River as u8
                && (4.0..25.0).contains(&map.river_width[i])
                && map.river_width[j] >= 1.4 * map.river_width[i]
                && map.river_width[j] < 80.0;
            if !joins || map.water_surface[j] < WATER_LEVEL as f32 + 3.0 {
                continue;
            }
            // the water in a window round the junction, as one piece or two
            let (cx, cz) = map.cell_centre(j % dim, j / dim);
            let (cx, cz) = (cx as i64, cz as i64);
            let wet = |x: i64, z: i64| {
                let s = terrain.sample(x as f64, z as f64);
                s.water_kind == WaterKind::River
            };
            let span = 56;
            let mut seen = std::collections::HashSet::new();
            let mut pieces = 0;
            for z in -span..=span {
                for x in -span..=span {
                    if !wet(cx + x, cz + z) || seen.contains(&(x, z)) {
                        continue;
                    }
                    pieces += 1;
                    let mut stack = vec![(x, z)];
                    seen.insert((x, z));
                    while let Some((px, pz)) = stack.pop() {
                        for (nx, nz) in [(px + 1, pz), (px - 1, pz), (px, pz + 1), (px, pz - 1)] {
                            if nx.abs() <= span
                                && nz.abs() <= span
                                && !seen.contains(&(nx, nz))
                                && wet(cx + nx, cz + nz)
                            {
                                seen.insert((nx, nz));
                                stack.push((nx, nz));
                            }
                        }
                    }
                }
            }
            checked += 1;
            // the junction's cell must be wet, or there is no river there to join
            if pieces > 1 {
                broken.push((cx, cz, pieces));
            }
            if checked >= 12 {
                break;
            }
        }
    }
    assert!(checked >= 2, "only {checked} junctions on the test world");
    assert!(
        broken.len() * 4 <= checked,
        "water in pieces at {} of {checked} junctions: {broken:?}",
        broken.len()
    );
}

#[test]
fn test_meadow_clings_to_a_mountain_face_where_the_rest_is_rock() {
    use ochre_base::biome::Biomes;
    let biomes = Biomes::new(1);
    let mountain = biomes.id("mountain").unwrap();
    let above = WATER_LEVEL + 110;
    let steep = 2.2;
    let (mut meadow, mut rock) = (0, 0);
    for i in 0..3000 {
        let (x, z) = (i * 23, i * 7 - 9000);
        let gradient = super::terrain::block_gradient(&biomes, mountain, 1, x, z, steep);
        let top = super::terrain::column_block(
            &biomes,
            mountain,
            above - 1,
            above,
            gradient,
            0.55,
            WATER_LEVEL,
        );
        match top {
            BlockType::Grass => {
                meadow += 1;
                assert!(
                    gradient < steep,
                    "a meadow that is no gentler than the face"
                );
            }
            BlockType::Stone => rock += 1,
            other => panic!("{other:?} on a mountain face"),
        }
    }
    // a third or so is meadow, in patches, the rest rock
    assert!(meadow > 500 && rock > 1000, "{meadow} meadow, {rock} rock");
    // gentle ground and other biomes are left as they were
    let gentle = super::terrain::block_gradient(&biomes, mountain, 1, 5, 5, 0.3);
    assert_eq!(0.3, gentle);
    let plain = ochre_base::biome::BiomeId::BASE;
    assert_eq!(
        steep,
        super::terrain::block_gradient(&biomes, plain, 1, 5, 5, steep)
    );
}

#[test]
fn test_pines_grow_on_a_mountains_lower_slopes() {
    use crate::generate::trees::{trees_near, TreeKind};
    use ochre_world::worldmap::climate::BiomeKind;
    let terrain = ochre_world::worldmap::WorldTerrain::new(std::sync::Arc::new(
        ochre_world::worldmap::generate(
            3,
            ochre_world::worldmap::WorldConfig {
                size: 32_768,
                map_cell: 32,
            },
        ),
    ));
    let map = terrain.map().clone();
    let dim = map.dim();
    // the middle of a stretch of mountain in the upper conifer belt, whatever the climate
    let belt = (20..dim - 20)
        .step_by(5)
        .flat_map(|z| (20..dim - 20).step_by(5).map(move |x| (x, z)))
        .find(|&(x, z)| {
            (-4..=4).step_by(2).all(|dz| {
                (-4..=4).step_by(2).all(|dx| {
                    let i = map.index((x as i64 + dx) as usize, (z as i64 + dz) as usize);
                    BiomeKind::from_u8(map.biome[i]) == BiomeKind::Mountain
                        && (130.0..260.0).contains(&(map.height[i] - WATER_LEVEL as f32))
                        && map.water_kind[i] == 0
                })
            })
        });
    let (cx, cz) = belt.expect("no mountain slopes in the conifer belt");
    let (x, z) = map.cell_centre(cx, cz);
    let regions = RegionStore::new(std::sync::Arc::new(terrain));
    let mut heights = regions.sampler();
    let min = bevy::math::I64Vec2::new(x as i64 - 250, z as i64 - 250);
    let trees = trees_near(regions.terrain(), &mut heights, min, min + 500);
    let pines = trees.iter().filter(|t| t.kind == TreeKind::Pine).count();
    assert!(trees.len() > 20, "only {} trees on the slope", trees.len());
    assert!(
        pines * 2 > trees.len(),
        "{pines} of {} trees are pines",
        trees.len()
    );
}

#[test]
fn test_no_pines_stand_in_savanna_jungle_or_desert() {
    use crate::generate::trees::{trees_near, TreeKind};
    use ochre_world::worldmap::climate::BiomeKind;
    let terrain = ochre_world::worldmap::WorldTerrain::new(std::sync::Arc::new(
        ochre_world::worldmap::generate(1, ochre_world::worldmap::WorldConfig::default()),
    ));
    let map = terrain.map().clone();
    let dim = map.dim();
    let regions = RegionStore::new(std::sync::Arc::new(terrain));
    let mut heights = regions.sampler();
    let (mut hot_trees, mut high_hot_trees, mut places) = (0, 0, 0);
    for kind in [BiomeKind::Savanna, BiomeKind::Jungle] {
        // stretches of this biome on ground well over the sea, where the air is cooler, and the
        // windows round them take in the edges, where other biomes meet it
        let spots = (30..dim - 30)
            .step_by(7)
            .flat_map(|z| (30..dim - 30).step_by(7).map(move |x| (x, z)))
            .filter(|&(x, z)| {
                let i = map.index(x, z);
                BiomeKind::from_u8(map.biome[i]) == kind
                    && (110.0..260.0).contains(&(map.height[i] - WATER_LEVEL as f32))
                    && map.water_kind[i] == 0
            });
        for (cx, cz) in spots.step_by(5).take(12) {
            let (x, z) = map.cell_centre(cx, cz);
            let min = bevy::math::I64Vec2::new(x as i64 - 200, z as i64 - 200);
            let trees = trees_near(regions.terrain(), &mut heights, min, min + 400);
            places += 1;
            for tree in &trees {
                let name = regions
                    .terrain()
                    .biomes()
                    .name(heights.biome(tree.base.x, tree.base.z));
                if matches!(name, Some("savanna" | "jungle" | "desert")) {
                    hot_trees += 1;
                    high_hot_trees += (tree.base.y - WATER_LEVEL >= 110) as usize;
                    assert_ne!(
                        TreeKind::Pine,
                        tree.kind,
                        "a pine in {name:?} at {} ({} over the sea)",
                        tree.base,
                        tree.base.y - WATER_LEVEL
                    );
                }
            }
        }
    }
    assert!(
        places >= 6 && hot_trees > 50 && high_hot_trees > 10,
        "{places} places, {hot_trees} hot trees, {high_hot_trees} high"
    );
}

#[test]
fn test_the_foothills_round_a_range_are_much_rougher_than_the_lowland_far_from_one() {
    use ochre_world::worldmap::climate::BiomeKind;
    let terrain = ochre_world::worldmap::WorldTerrain::new(std::sync::Arc::new(
        ochre_world::worldmap::generate(1, ochre_world::worldmap::WorldConfig::default()),
    ));
    let map = terrain.map().clone();
    let regions = RegionStore::new(std::sync::Arc::new(terrain));
    let mut heights = regions.sampler();
    let dim = map.dim();
    let (mut near, mut far) = (Vec::new(), Vec::new());
    for cz in (0..dim).step_by(11) {
        for cx in (0..dim).step_by(11) {
            let i = map.index(cx, cz);
            let above = map.height[i] - WATER_LEVEL as f32;
            if !(40.0..200.0).contains(&above)
                || map.water_kind[i] != 0
                || BiomeKind::from_u8(map.biome[i]) == BiomeKind::Mountain
            {
                continue;
            }
            let range = map.range[i] as f32 / 255.0;
            let (x, z) = map.cell_centre(cx, cz);
            let (gx, gz) = heights.gradient(x as i64, z as i64);
            let gradient = gx.abs() + gz.abs();
            if range > 0.5 {
                near.push(gradient);
            } else if range < 0.05 {
                far.push(gradient);
            }
        }
    }
    let median = |v: &mut Vec<f64>| {
        v.sort_by(f64::total_cmp);
        v[v.len() / 2]
    };
    assert!(
        near.len() > 200 && far.len() > 200,
        "{} foothill and {} lowland columns",
        near.len(),
        far.len()
    );
    let (near, far) = (median(&mut near), median(&mut far));
    assert!(
        near > far * 3.0 && near > 0.9,
        "foothills {near} against lowland {far}"
    );
}
