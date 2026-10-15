// Counts the vertices trees cost at each level of detail, and all the land's, over the densest forest found near
// the origin, and what that comes to over each level's ring of the view with the renderer
// settings from assets/settings.toml, were it all forest like that. Run with:
//   cargo run --release --example tree_cost -- [seed, default 1] [x z, default the densest forest]
use std::sync::Arc;

use bevy::math::{I64Vec2, I64Vec3};
use bevy::mesh::{Mesh, VertexAttributeValues};
use rustcraft::block::Tile;
use rustcraft::chunks::chunk::{ChunkCoordinate, CHUNK_SIZE};
use rustcraft::chunks::generate::generator::{
    generate_chunk, generate_chunk_mesh, generate_lod_mesh,
};
use rustcraft::chunks::generate::noise::NoiseGenerator;
use rustcraft::chunks::generate::region_store::RegionStore;
use rustcraft::chunks::generate::trees::trees_near;
use rustcraft::chunks::lod::LodKey;
use rustcraft::settings::Settings;

// Blocks across the patch of forest measured.
const PATCH: i64 = 1024;

// The vertices in `mesh` that draw leaves or logs.
fn tree_vertices(mesh: &Mesh) -> usize {
    let Some(VertexAttributeValues::Float32x4(colors)) = mesh.attribute(Mesh::ATTRIBUTE_COLOR)
    else {
        return 0;
    };
    let tree = [Tile::Leaves, Tile::LogBark, Tile::LogEnd].map(|t| t.atlas_cell());
    colors.iter().filter(|c| tree.contains(&c[3])).count()
}

fn main() {
    let seed = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    let noise = Arc::new(NoiseGenerator::new(seed));
    // the densest forest on a coarse grid, by trees counted in a window round each point
    let regions = Arc::new(RegionStore::new(noise.clone()));
    let mut best = (0, 0, 0);
    {
        let mut sampler = regions.sampler();
        for x in (-12_000..12_000).step_by(1536) {
            for z in (-12_000..12_000).step_by(1536) {
                let corner = I64Vec2::new(x, z);
                let count = trees_near(&noise, &mut sampler, corner, corner + 200).len();
                if count > best.0 {
                    best = (count, x + 100, z + 100);
                }
            }
        }
    }
    let (count, mut cx, mut cz) = best;
    let args: Vec<i64> = std::env::args()
        .skip(2)
        .filter_map(|a| a.parse().ok())
        .collect();
    if let [x, z] = args[..] {
        (cx, cz) = (x, z);
    }
    println!(
        "measuring round {cx}, {cz} (the densest forest is {count} trees in 200 x 200 blocks)"
    );
    let layout = Settings::read("assets/settings.toml")
        .unwrap()
        .renderer
        .lod_layout();
    let size = CHUNK_SIZE as i64;
    let min = I64Vec3::new(cx - PATCH / 2, 0, cz - PATCH / 2);
    let height_range = || {
        let mut sampler = regions.sampler();
        let mut range = (i64::MAX, i64::MIN);
        for x in (min.x..min.x + PATCH).step_by(16) {
            for z in (min.z..min.z + PATCH).step_by(16) {
                let h = sampler.height(x, z).round() as i64;
                range = (range.0.min(h), range.1.max(h + 30));
            }
        }
        range
    };
    let (low, high) = height_range();

    let (mut total, mut total_all) = (0.0, 0.0);
    let mut previous_reach = 0.0;
    for level in 0..=layout.max_level {
        let width = size << level;
        let (mut vertices, mut all) = (0, 0);

        let measured = if level == 0 {
            // a quarter of the patch, as full detail is slow to make
            let span = PATCH / 4 / size;
            for x in 0..span {
                for z in 0..span {
                    for y in low.div_euclid(size)..=high.div_euclid(size) {
                        let coord =
                            ChunkCoordinate(I64Vec3::new(min.x / size + x, y, min.z / size + z));
                        let data = generate_chunk(regions.clone(), coord);
                        let mesh = generate_chunk_mesh(
                            Arc::new(data),
                            vec![None; 26],
                            coord,
                            &noise,
                            false,
                        )
                        .opaque;
                        vertices += tree_vertices(&mesh);
                        all += mesh.count_vertices();
                    }
                }
            }
            (span * size).pow(2)
        } else {
            let span = (PATCH / width).max(1);
            for x in 0..span {
                for z in 0..span {
                    for y in low.div_euclid(width)..=high.div_euclid(width) {
                        let chunk = ChunkCoordinate(
                            I64Vec3::new(min.x + x * width, y * width, min.z + z * width) / size,
                        );
                        if let Some(meshes) =
                            generate_lod_mesh(regions.clone(), LodKey::containing(chunk, level))
                        {
                            vertices += tree_vertices(&meshes.opaque);
                            all += meshes.opaque.count_vertices();
                        }
                    }
                }
            }
            (span * width).pow(2)
        };
        let per_km2 = vertices as f64 / measured as f64 * 1e6;
        let all_per_km2 = all as f64 / measured as f64 * 1e6;
        let reach = layout.reach(level).min(layout.render_distance()) as f64 * size as f64;
        let ring = std::f64::consts::PI * (reach * reach - previous_reach * previous_reach);
        previous_reach = reach;
        let in_view = per_km2 * ring / 1e6;
        total += in_view;
        let all_in_view = all_per_km2 * ring / 1e6;
        total_all += all_in_view;
        println!(
            "level {level} (cells {:>2}): {per_km2:>10.0} tree vertices/km², ring out to {reach:>6.0} blocks: {:>6.2}M, {:>6.2}M in all",
            1 << level,
            in_view / 1e6,
            all_in_view / 1e6
        );
    }
    println!(
        "all levels: {:.2}M tree vertices, {:.2}M in all, in a view all like this",
        total / 1e6,
        total_all / 1e6
    );
}
