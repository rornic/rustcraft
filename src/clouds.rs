use std::f64::consts::TAU;

use bevy::{
    asset::{Asset, RenderAssetUsages},
    image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor},
    pbr::{MaterialPipeline, MaterialPipelineKey},
    light::{NotShadowCaster, NotShadowReceiver},
    shader::ShaderRef,
    prelude::*,
    reflect::TypePath,
    render::{
        mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology},
        render_resource::{
            AsBindGroup, Extent3d, RenderPipelineDescriptor, SpecializedMeshPipelineError,
            TextureDimension, TextureFormat,
        },
    },
};
use noise::{Fbm, MultiFractal, NoiseFn, Perlin};

use crate::chunks::material::{push_shader_defs, water_level};

/// Height of the clouds' undersides: above most land, so only the tallest peaks reach them.
pub const CLOUD_BASE: f32 = 720.0;
// Clouds are blocks this wide, stacked in layers this tall.
const CELL: f32 = 48.0;
const LAYER: f32 = 16.0;
// The most layers a cloud is stacked, at its middle.
const MAX_LAYERS: u8 = 5;
// How many layers a cloud climbs for each cell in from its edge, so it domes up.
const LAYERS_PER_CELL: f64 = 0.6;
// How many layers lumps add or take away, and roughly how many cells across one is.
const LUMPS: f64 = 1.6;
const LUMP_CELLS: f64 = 3.0;
// Cells across the pattern, which repeats seamlessly.
const PATTERN: usize = 256;
// How much of the sky is cloud, before the small ones are dropped.
const COVER: f64 = 0.14;
// Clouds under this many cells are dropped, leaving only big ones.
const MIN_CLOUD_CELLS: usize = 80;
// Roughly how many cells across a cloud is.
const CLOUD_CELLS: f64 = 16.0;
// Copies of the pattern drawn out from the one over the camera, each way: enough to reach
// the fog from high up, where the render distance has grown to tens of thousands of blocks.
const COPIES: i32 = 3;
// The pattern's width in blocks.
const SPAN: f32 = PATTERN as f32 * CELL;
// Which way and how fast clouds drift, in blocks a second: a whole number of patterns an
// hour each way, so they carry on seamlessly when the shaders' clock, Bevy's `globals.time`,
// wraps back to zero on the hour.
const WIND: Vec2 = Vec2::new(SPAN * 2.0 / 3600.0, SPAN / 3600.0);
// Each level of detail's cells, in the finest's, and how far out it starts, in patterns
// from the camera to the nearest edge of a copy: far copies are coarser, as there are many
// and every one is drawn in full.
const LODS: [(usize, f32); 3] = [(1, 0.0), (2, 0.5), (4, 1.5)];
// Slightly warm white, as sunlight leaves them.
const CLOUD_COLOR: LinearRgba = LinearRgba::rgb(0.95, 0.95, 0.92);

pub struct CloudPlugin;

impl Plugin for CloudPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<CloudMaterial>::default())
        .add_systems(Startup, spawn_clouds)
        .add_systems(Update, drift_clouds);
    }
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct CloudMaterial {
    #[uniform(0)]
    color: LinearRgba,
    // the clouds' base and greatest height, for shading them darker lower down
    #[uniform(1)]
    shape: Vec4,
}

impl Material for CloudMaterial {
    // clouds are far above anything they could shade, see `CLOUD_BASE`
    fn enable_prepass() -> bool {
        false
    }

    fn enable_shadows() -> bool {
        false
    }

    fn fragment_shader() -> ShaderRef {
        "shaders/clouds.wesl".into()
    }

    // blended, stacked layers would show through one another in whatever order they were drawn
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Opaque
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        push_shader_defs(descriptor, &[water_level()]);
        Ok(())
    }
}

/// The cloud pattern, for the land's shader to cast clouds' shadows with, see
/// `cloud_shadow.wesl`. Spawned by `spawn_clouds`.
#[derive(Resource, Clone)]
pub struct CloudLayer {
    /// The pattern, one texel a cell, more the thicker the cloud there, 0 where clear.
    pub pattern: Handle<Image>,
    /// The middle of the layer's height, the pattern's width in blocks, and the wind.
    pub layer: Vec4,
}

// One copy of the pattern, `offset` patterns from the one over the camera.
#[derive(Component)]
struct CloudTile {
    offset: IVec2,
}

// The clouds' mesh at each of `LODS`, finest first.
#[derive(Resource)]
struct CloudMeshes(Vec<Handle<Mesh>>);

/// Which cells of the pattern are cloud, `PATTERN` x `PATTERN`, row-major [x][z]. Sampled
/// round a torus in 4D noise, so it wraps at its edges without a seam.
fn cloud_pattern(seed: u32) -> Vec<bool> {
    let noise = Fbm::<Perlin>::new(seed).set_octaves(2).set_persistence(0.35);
    let radius = PATTERN as f64 / (TAU * CLOUD_CELLS);
    let around = |cell: usize| {
        let angle = TAU * cell as f64 / PATTERN as f64;
        (angle.cos() * radius, angle.sin() * radius)
    };
    let mut values = Vec::with_capacity(PATTERN * PATTERN);
    for x in 0..PATTERN {
        for z in 0..PATTERN {
            let ((a, b), (c, d)) = (around(x), around(z));
            values.push(noise.get([a, b, c, d]));
        }
    }
    let mut sorted = values.clone();
    sorted.sort_by(f64::total_cmp);
    let threshold = sorted[((1.0 - COVER) * sorted.len() as f64) as usize];
    let mut pattern: Vec<bool> = values.into_iter().map(|v| v > threshold).collect();
    drop_small_clouds(&mut pattern);
    pattern
}

// Clears every cloud of fewer than `MIN_CLOUD_CELLS` cells, where cells touch edge to edge, and
// round the pattern's wrap like the clouds themselves do.
fn drop_small_clouds(pattern: &mut [bool]) {
    let n = PATTERN;
    let mut seen = vec![false; n * n];
    for start in 0..n * n {
        if !pattern[start] || seen[start] {
            continue;
        }
        let mut cloud = vec![start];
        seen[start] = true;
        let mut next = 0;
        while next < cloud.len() {
            let (x, z) = (cloud[next] / n, cloud[next] % n);
            next += 1;
            for (dx, dz) in [(1, 0), (n - 1, 0), (0, 1), (0, n - 1)] {
                let neighbour = ((x + dx) % n) * n + (z + dz) % n;
                if pattern[neighbour] && !seen[neighbour] {
                    seen[neighbour] = true;
                    cloud.push(neighbour);
                }
            }
        }
        if cloud.len() < MIN_CLOUD_CELLS {
            for cell in cloud {
                pattern[cell] = false;
            }
        }
    }
}

// One column of cloud: the layers it fills, from `bottom` up to but not including `top`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Column {
    bottom: u8,
    top: u8,
}

// Clouds as columns, `n` x `n` cells `cell` blocks wide, row-major [x][z], wrapping at the edges.
struct Columns {
    n: usize,
    cell: f32,
    cells: Vec<Option<Column>>,
}

impl Columns {
    fn get(&self, x: usize, z: usize) -> Option<Column> {
        self.cells[(x % self.n) * self.n + z % self.n]
    }
}

/// The pattern piled up into clouds: domed, higher the farther in from their edges, with
/// lumps on top, and the odd lip at the edge lifted off the base so they bulge out underneath.
fn cloud_columns(pattern: &[bool], seed: u32) -> Columns {
    let n = PATTERN;
    // cells in from the nearest clear sky, 1 at a cloud's edge, flooded out from the sky
    let mut inside = vec![0u32; n * n];
    let mut queue: Vec<usize> = (0..n * n).filter(|&i| !pattern[i]).collect();
    let mut next = 0;
    while next < queue.len() {
        let cell = queue[next];
        let (x, z) = (cell / n, cell % n);
        next += 1;
        for (dx, dz) in [(1, 0), (n - 1, 0), (0, 1), (0, n - 1)] {
            let neighbour = ((x + dx) % n) * n + (z + dz) % n;
            if pattern[neighbour] && inside[neighbour] == 0 {
                inside[neighbour] = inside[cell] + 1;
                queue.push(neighbour);
            }
        }
    }
    // round a torus, like the pattern, so lumps wrap without a seam too
    let noise = Perlin::new(seed.wrapping_add(1));
    let radius = PATTERN as f64 / (TAU * LUMP_CELLS);
    let around = |cell: usize| {
        let angle = TAU * cell as f64 / PATTERN as f64;
        (angle.cos() * radius, angle.sin() * radius)
    };
    let cells = (0..n * n)
        .map(|i| {
            if !pattern[i] {
                return None;
            }
            let ((a, b), (c, d)) = (around(i / n), around(i % n));
            let lump = noise.get([a, b, c, d]);
            let top = (inside[i] as f64 * LAYERS_PER_CELL + lump * LUMPS).round().clamp(1.0, MAX_LAYERS as f64) as u8;
            let lifted = inside[i] == 1 && top >= 2 && lump > 0.15;
            Some(Column { bottom: lifted as u8, top })
        })
        .collect();
    Columns { n, cell: CELL, cells }
}

// Coarser clouds, `factor` cells merged into one each way: cloud where at least half of
// them are, as high as they are on average.
fn coarsen(columns: &Columns, factor: usize) -> Columns {
    let n = columns.n / factor;
    let mut cells = Vec::with_capacity(n * n);
    for x in 0..n {
        for z in 0..n {
            let merged: Vec<Column> = (0..factor * factor)
                .filter_map(|i| columns.get(x * factor + i / factor, z * factor + i % factor))
                .collect();
            if merged.len() * 2 < factor * factor {
                cells.push(None);
                continue;
            }
            let mean = |f: fn(&Column) -> u8| {
                (merged.iter().map(|c| f(c) as f32).sum::<f32>() / merged.len() as f32).round() as u8
            };
            let bottom = mean(|c| c.bottom);
            cells.push(Some(Column { bottom, top: mean(|c| c.top).max(bottom + 1) }));
        }
    }
    Columns { n, cell: columns.cell * factor as f32, cells }
}

#[derive(Default)]
struct CloudMesh {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    indices: Vec<u32>,
}

impl CloudMesh {
    // A quad facing `normal`, wound so it's seen from that side.
    fn quad(&mut self, mut corners: [Vec3; 4], normal: Vec3) {
        if (corners[1] - corners[0]).cross(corners[2] - corners[0]).dot(normal) < 0.0 {
            corners.reverse();
        }
        let start = self.positions.len() as u32;
        self.positions.extend(corners.map(|c| c.to_array()));
        self.normals.extend([normal.to_array(); 4]);
        self.indices.extend([0, 1, 2, 0, 2, 3].map(|i| start + i));
    }
}

// The fewest rectangles covering the cells where `height` is the same, as
// (x0, z0, x1, z1, height), ends exclusive.
fn rectangles(n: usize, height: impl Fn(usize, usize) -> Option<u8>) -> Vec<(usize, usize, usize, usize, u8)> {
    let mut taken = vec![false; n * n];
    let mut found = vec![];
    for x0 in 0..n {
        for z0 in 0..n {
            let Some(h) = height(x0, z0).filter(|_| !taken[x0 * n + z0]) else {
                continue;
            };
            let same = |x: usize, z: usize| height(x, z) == Some(h) && !taken[x * n + z];
            let mut x1 = x0 + 1;
            while x1 < n && same(x1, z0) {
                x1 += 1;
            }
            let mut z1 = z0 + 1;
            while z1 < n && (x0..x1).all(|x| same(x, z1)) {
                z1 += 1;
            }
            for x in x0..x1 {
                for z in z0..z1 {
                    taken[x * n + z] = true;
                }
            }
            found.push((x0, z0, x1, z1, h));
        }
    }
    found
}

// The layers of `column`'s side left open by `neighbour`: below the neighbour, and above it.
fn open_side(column: Option<Column>, neighbour: Option<Column>) -> [Option<(u8, u8)>; 2] {
    let Some(c) = column else {
        return [None, None];
    };
    let Some(b) = neighbour else {
        return [Some((c.bottom, c.top)), None];
    };
    let below = (c.bottom < b.bottom).then(|| (c.bottom, c.top.min(b.bottom)));
    let above = (c.top > b.top).then(|| (c.bottom.max(b.top), c.top));
    [below, above]
}

/// The clouds as blocks: tops and bottoms merged into rectangles, and sides into runs.
fn cloud_mesh(columns: &Columns) -> Mesh {
    let (n, cell) = (columns.n, columns.cell);
    let mut mesh = CloudMesh::default();
    let corner = |x: usize, layer: u8, z: usize| Vec3::new(x as f32 * cell, layer as f32 * LAYER, z as f32 * cell);

    let tops = rectangles(n, |x, z| columns.get(x, z).map(|c| c.top));
    let bottoms = rectangles(n, |x, z| columns.get(x, z).map(|c| c.bottom));
    for ((x0, z0, x1, z1, y), normal) in tops.into_iter().map(|r| (r, Vec3::Y)).chain(bottoms.into_iter().map(|r| (r, Vec3::NEG_Y))) {
        mesh.quad([corner(x0, y, z0), corner(x1, y, z0), corner(x1, y, z1), corner(x0, y, z1)], normal);
    }

    // sides, where a column stands higher or lower than its neighbour, merged along each edge
    for along_x in [true, false] {
        for forward in [true, false] {
            let normal = match (along_x, forward) {
                (true, true) => Vec3::X,
                (true, false) => Vec3::NEG_X,
                (false, true) => Vec3::Z,
                (false, false) => Vec3::NEG_Z,
            };
            let step = if forward { 1 } else { n - 1 };
            // (across, along) to (x, z)
            let xz = |u: usize, v: usize| if along_x { (u, v) } else { (v, u) };
            let at = |u: usize, v: usize, layer: u8| {
                let (x, z) = xz(u, v);
                corner(x, layer, z)
            };
            for u in 0..n {
                let edge = if forward { u + 1 } else { u };
                for piece in 0..2 {
                    let open = |v: usize| {
                        let (x, z) = xz(u, v);
                        let (nx, nz) = xz(u + step, v);
                        open_side(columns.get(x, z), columns.get(nx, nz))[piece]
                    };
                    let mut v = 0;
                    while v < n {
                        let Some((lo, hi)) = open(v) else {
                            v += 1;
                            continue;
                        };
                        let start = v;
                        while v < n && open(v) == Some((lo, hi)) {
                            v += 1;
                        }
                        mesh.quad([at(edge, start, lo), at(edge, start, hi), at(edge, v, hi), at(edge, v, lo)], normal);
                    }
                }
            }
        }
    }

    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, mesh.positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, mesh.normals)
        .with_inserted_indices(Indices::U32(mesh.indices))
}

// The clouds as a texture that repeats, more where they're thicker, blended between cells
// so shadows' edges are soft.
fn pattern_image(columns: &Columns) -> Image {
    let n = columns.n;
    // u along x, v along z; even the thinnest cloud keeps off half as much sun as the thickest
    let shade = |c: Column| 0.5 + 0.5 * (c.top - c.bottom) as f32 / MAX_LAYERS as f32;
    let data = (0..n * n)
        .map(|i| columns.get(i % n, i / n).map_or(0, |c| (shade(c) * 255.0).round() as u8))
        .collect();
    let mut image = Image::new(
        Extent3d {
            width: n as u32,
            height: n as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::R8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        ..default()
    });
    image
}

pub fn spawn_clouds(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<CloudMaterial>>,
) {
    let seed = rand::random();
    let columns = cloud_columns(&cloud_pattern(seed), seed);
    let height = MAX_LAYERS as f32 * LAYER;
    commands.insert_resource(CloudLayer {
        pattern: images.add(pattern_image(&columns)),
        layer: Vec4::new(CLOUD_BASE + height / 2.0, SPAN, WIND.x, WIND.y),
    });
    let lods: Vec<Handle<Mesh>> = LODS
        .iter()
        .map(|&(factor, _)| meshes.add(cloud_mesh(&coarsen(&columns, factor))))
        .collect();
    let material = materials.add(CloudMaterial {
        color: CLOUD_COLOR,
        shape: Vec4::new(CLOUD_BASE, height, 0.0, 0.0),
    });
    for x in -COPIES..=COPIES {
        for z in -COPIES..=COPIES {
            commands.spawn((
                CloudTile { offset: IVec2::new(x, z) },
                Mesh3d(lods[LODS.len() - 1].clone()),
                MeshMaterial3d(material.clone()),
                Transform::default(),
                NotShadowCaster,
                NotShadowReceiver,
            ));
        }
    }
    commands.insert_resource(CloudMeshes(lods));
}

/// Where the pattern copy `offset` copies from the one over `camera` starts, `elapsed`
/// seconds into the wind.
fn tile_origin(camera: Vec2, elapsed: f32, offset: IVec2) -> Vec2 {
    let drift = (WIND * elapsed).rem_euclid(Vec2::splat(SPAN));
    ((camera - drift) / SPAN).floor() * SPAN + drift + offset.as_vec2() * SPAN
}

// Which of `LODS` a copy starting at `origin` is drawn at, seen from `camera`.
fn tile_lod(camera: Vec2, origin: Vec2) -> usize {
    let nearest = camera.clamp(origin, origin + Vec2::splat(SPAN));
    let away = camera.distance(nearest) / SPAN;
    LODS.iter().rposition(|&(_, start)| away >= start).unwrap_or(0)
}

fn drift_clouds(
    time: Res<Time>,
    lods: Res<CloudMeshes>,
    camera: Query<&GlobalTransform, With<Camera3d>>,
    mut tiles: Query<(&CloudTile, &mut Transform, &mut Mesh3d)>,
) {
    let Ok(camera) = camera.single() else {
        return;
    };
    let camera = camera.translation().xz();
    for (tile, mut transform, mut mesh) in &mut tiles {
        // the shaders' clock, so clouds and their shadows keep together
        let origin = tile_origin(camera, time.elapsed_secs_wrapped(), tile.offset);
        transform.translation = Vec3::new(origin.x, CLOUD_BASE, origin.y);
        let lod = &lods.0[tile_lod(camera, origin)];
        if mesh.0 != *lod {
            mesh.0 = lod.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pattern_covers_about_the_share_of_sky_asked() {
        let pattern = cloud_pattern(3);
        let cover = pattern.iter().filter(|&&c| c).count() as f64 / pattern.len() as f64;
        // dropping the small clouds takes some of it
        assert!(cover <= COVER + 0.005 && cover > COVER * 0.6, "{cover}");
    }

    #[test]
    fn test_only_big_clouds_are_left() {
        let pattern = cloud_pattern(3);
        let n = PATTERN;
        // every cloud has at least the minimum, found by flood fill as `drop_small_clouds` does
        let mut seen = vec![false; n * n];
        for start in 0..n * n {
            if !pattern[start] || seen[start] {
                continue;
            }
            let mut cloud = vec![start];
            seen[start] = true;
            let mut next = 0;
            while next < cloud.len() {
                let (x, z) = (cloud[next] / n, cloud[next] % n);
                next += 1;
                for (dx, dz) in [(1, 0), (n - 1, 0), (0, 1), (0, n - 1)] {
                    let neighbour = ((x + dx) % n) * n + (z + dz) % n;
                    if pattern[neighbour] && !seen[neighbour] {
                        seen[neighbour] = true;
                        cloud.push(neighbour);
                    }
                }
            }
            assert!(cloud.len() >= MIN_CLOUD_CELLS, "a cloud of {} cells", cloud.len());
        }
    }

    #[test]
    fn test_small_clouds_are_dropped_and_wrapped_ones_kept_whole() {
        let n = PATTERN;
        let mut pattern = vec![false; n * n];
        // a lone cell, and a cloud cut in two by the pattern's edge, each half under the minimum
        pattern[10 * n + 10] = true;
        for x in [n - 1, 0] {
            for z in 100..100 + MIN_CLOUD_CELLS {
                pattern[x * n + z] = true;
            }
        }
        drop_small_clouds(&mut pattern);
        assert!(!pattern[10 * n + 10]);
        assert!(pattern[(n - 1) * n + 100] && pattern[100]);
    }

    #[test]
    fn test_pattern_wraps_without_a_seam() {
        // across the wrap, neighbouring cells agree about as often as anywhere else
        let pattern = cloud_pattern(3);
        let n = PATTERN;
        let agree = |a: usize, b: usize| (0..n).filter(|&z| pattern[a * n + z] == pattern[b * n + z]).count();
        let inside = agree(n / 2, n / 2 + 1);
        let across = agree(n - 1, 0);
        assert!(across as f64 > inside as f64 * 0.8, "inside {inside}, across the wrap {across}");
    }

    fn columns(n: usize, cells: &[((usize, usize), (u8, u8))]) -> Columns {
        let mut columns = Columns { n, cell: CELL, cells: vec![None; n * n] };
        for &((x, z), (bottom, top)) in cells {
            columns.cells[x * n + z] = Some(Column { bottom, top });
        }
        columns
    }

    fn triangles(mesh: &Mesh) -> Vec<([Vec3; 3], Vec3)> {
        let Some(Indices::U32(indices)) = mesh.indices() else { panic!() };
        let positions = mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap().as_float3().unwrap();
        let normals = mesh.attribute(Mesh::ATTRIBUTE_NORMAL).unwrap().as_float3().unwrap();
        indices
            .chunks(3)
            .map(|tri| (tri.iter().map(|&i| Vec3::from(positions[i as usize])).collect::<Vec<_>>().try_into().unwrap(), Vec3::from(normals[tri[0] as usize])))
            .collect()
    }

    // The volume a closed mesh wraps, which comes out right only when no face is missing,
    // doubled up or facing the wrong way.
    fn volume(mesh: &Mesh) -> f32 {
        triangles(mesh).iter().map(|([a, b, c], _)| a.dot(b.cross(*c)) / 6.0).sum()
    }

    fn column_volume(columns: &Columns) -> f32 {
        columns.cells.iter().flatten().map(|c| (c.top - c.bottom) as f32 * LAYER * columns.cell * columns.cell).sum()
    }

    #[test]
    fn test_faces_point_outward() {
        let mesh = cloud_mesh(&columns(PATTERN, &[((5, 7), (0, 1))]));
        let triangles = triangles(&mesh);
        // a single cell is a box: six faces
        assert_eq!(12, triangles.len());
        for ([a, b, c], normal) in triangles {
            assert!((b - a).cross(c - a).dot(normal) > 0.0);
        }
    }

    #[test]
    fn test_stacked_columns_are_closed() {
        // a dome with a lip lifted off the base, a step down and a gap above a neighbour
        let columns = columns(
            8,
            &[((2, 2), (0, 4)), ((3, 2), (0, 2)), ((2, 3), (1, 3)), ((1, 2), (1, 2)), ((3, 3), (2, 5)), ((4, 3), (0, 1))],
        );
        let mesh = cloud_mesh(&columns);
        assert!((volume(&mesh) - column_volume(&columns)).abs() < 1.0, "{} vs {}", volume(&mesh), column_volume(&columns));
    }

    #[test]
    fn test_clouds_are_closed_and_domed() {
        let pattern = cloud_pattern(3);
        let mut columns = cloud_columns(&pattern, 3);
        // the pattern wraps, so its edges are only closed by the next copy: clear a border
        let n = columns.n;
        for x in 0..n {
            for z in 0..n {
                if x == 0 || z == 0 || x == n - 1 || z == n - 1 {
                    columns.cells[x * n + z] = None;
                }
            }
        }
        let mesh = cloud_mesh(&columns);
        let expected = column_volume(&columns);
        assert!((volume(&mesh) - expected).abs() < expected * 1e-4, "{} vs {expected}", volume(&mesh));
        let tops: Vec<u8> = columns.cells.iter().flatten().map(|c| c.top).collect();
        for layers in 1..=MAX_LAYERS {
            assert!(tops.contains(&layers), "no cloud {layers} layers high");
        }
    }

    #[test]
    fn test_coarser_lods_keep_the_clouds_with_far_fewer_vertices() {
        let columns = cloud_columns(&cloud_pattern(3), 3);
        let cover = |c: &Columns| c.cells.iter().flatten().count() as f32 * c.cell * c.cell;
        let mut vertices = vec![];
        for &(factor, _) in &LODS {
            let coarse = coarsen(&columns, factor);
            assert!((cover(&coarse) / cover(&columns) - 1.0).abs() < 0.15, "cover at {factor}");
            assert!((column_volume(&coarse) / column_volume(&columns) - 1.0).abs() < 0.2, "volume at {factor}");
            vertices.push(cloud_mesh(&coarse).count_vertices());
        }
        println!("vertices a copy at each LOD: {vertices:?}");
        assert!(vertices.windows(2).all(|w| w[1] * 2 < w[0]), "{vertices:?}");
    }

    #[test]
    fn test_wind_carries_on_seamlessly_past_the_hour() {
        let camera = Vec2::new(500.0, 900.0);
        assert!(tile_origin(camera, 3600.0, IVec2::ZERO).abs_diff_eq(tile_origin(camera, 0.0, IVec2::ZERO), 0.05));
    }

    #[test]
    fn test_image_puts_x_along_u_and_shades_thick_clouds_more() {
        let image = pattern_image(&columns(PATTERN, &[((3, 200), (0, MAX_LAYERS)), ((4, 200), (1, 2))]));
        let data = image.data.as_deref().unwrap();
        assert_eq!(255, data[200 * PATTERN + 3]);
        assert!(data[200 * PATTERN + 4] > 0 && data[200 * PATTERN + 4] < 255);
        assert_eq!(2, data.iter().filter(|&&v| v > 0).count());
    }

    #[test]
    fn test_nearby_copies_are_finest_and_far_ones_coarsest() {
        let camera = Vec2::new(10_000.0, -3_000.0);
        let origin = |offset| tile_origin(camera, 0.0, offset);
        assert_eq!(0, tile_lod(camera, origin(IVec2::ZERO)));
        assert_eq!(LODS.len() - 1, tile_lod(camera, origin(IVec2::new(COPIES, 0))));
        assert_eq!(LODS.len() - 1, tile_lod(camera, origin(IVec2::new(-COPIES, COPIES))));
    }

    #[test]
    fn test_tiles_cover_the_camera_and_drift_with_the_wind() {
        let span = SPAN;
        let camera = Vec2::new(10_000.0, -3_000.0);
        for elapsed in [0.0, 100.0, 5_000.0] {
            let origin = tile_origin(camera, elapsed, IVec2::ZERO);
            assert!((camera - origin).cmpge(Vec2::ZERO).all() && (camera - origin).cmplt(Vec2::splat(span)).all());
        }
        let moved = tile_origin(camera, 1.0, IVec2::ZERO) - tile_origin(camera, 0.0, IVec2::ZERO);
        assert!((moved - WIND).length() < 1e-3 || (moved - WIND).abs().max_element() > span * 0.5);
    }
}
