// What the sun's shadow cascades would draw, headless (no window or GPU): loads the start area
// with the renderer settings from assets/settings.toml, then, for a range of sun heights and
// directions relative to the camera, builds the cascades with Bevy's own `build_directional_light_cascades`
// and counts the terrain meshes and triangles each cascade's frustum lets through, as
// `check_dir_light_mesh_visibility` does, against what the camera's own view draws. Run with:
//   cargo run --release --example shadow_cull -- [shadow distance in blocks] [cascades] [camera pitch in degrees]
// Counts are CPU-side submissions, so they say how much geometry the shadow pass is given, not
// how long the GPU takes over it. It also counts what would be left if each cascade kept only the
// casters that can shadow something in its slice of the view: those in the hull of the slice and
// the slice swept toward the sun, up to the top of the world.
use std::time::{Duration, Instant};

use bevy::camera::primitives::{Aabb, Frustum};
use bevy::camera::{PerspectiveProjection, Projection};
use bevy::ecs::system::RunSystemOnce;
use bevy::light::cascade::build_directional_light_cascades;
use bevy::light::{
    CascadeShadowConfig, CascadeShadowConfigBuilder, Cascades, DirectionalLight,
    DirectionalLightShadowMap,
};
use bevy::math::Vec3A;
use bevy::mesh::{Indices, VertexAttributeValues};
use bevy::pbr::MeshMaterial3d;
use bevy::prelude::*;
use bevy::shape::ViewFrustum;
use ochre_base::block::Tile;
use ochre_base::WORLD_HEIGHT;
use ochre_engine::settings::{DebugSettings, RendererSettings, Settings};
use ochre_terrain::chunk_loader::{GenerateChunkData, GenerateChunkMesh};
use ochre_terrain::group::ChunkGroup;
use ochre_terrain::lod::{GenerateLodMesh, LodNode};
use ochre_terrain::material::{
    ChunkMaterial, ShaderInputs, SprayMaterial, TerrainAssets, WaterMaterial,
};
use ochre_terrain::{insert_terrain_resources, task_pool_plugin, TerrainPlugin};
use ochre_world::worldmap::WorldConfig;

// The game's camera, see `spawn_player`.
const FOV_DEGREES: f32 = 90.0;
const ASPECT: f32 = 16.0 / 9.0;
// The sun's first cascade ends here, see `FIRST_CASCADE_DISTANCE` in `sky.rs`.
const FIRST_CASCADE: f32 = 20.0;
const EYE_HEIGHT: f32 = 1.7;
// The most levels counted separately, the rest are lumped together.
const LEVELS: usize = 8;

#[derive(Component)]
struct SimCamera;

#[derive(Resource)]
struct Renderer(RendererSettings, WorldConfig);

fn setup(mut commands: Commands, renderer: Res<Renderer>, mut assets: TerrainAssets) {
    let world_config = renderer.1;
    let renderer = &renderer.0;
    let world = ochre_world::load_world(renderer.lod_layout().region_reach() as u32, world_config);
    let spawn = world.spawn();
    let height = world.height;
    commands.insert_resource(world);
    // the loaders follow a camera under the player
    let player = commands.spawn(Transform::from_translation(spawn)).id();
    let camera = commands
        .spawn((
            SimCamera,
            Camera::default(),
            Transform::from_translation(Vec3::Y * EYE_HEIGHT).looking_to(Vec3::X, Vec3::Y),
        ))
        .id();
    commands.entity(player).add_children(&[camera]);
    insert_terrain_resources(
        &mut commands,
        renderer,
        DebugSettings::default(),
        height,
        &ShaderInputs::default(),
        &mut assets,
    );
}

// A terrain mesh that could be drawn: where it is and what it costs.
struct Caster {
    aabb: Aabb,
    transform: GlobalTransform,
    triangles: usize,
    // of those, drawn with holes cut out by the shadow pass's fragment shader
    leaves: usize,
    flora: usize,
    // 0 for full-detail groups, then the LOD level
    class: usize,
}

// How many of `mesh`'s triangles are leaves and ground flora, by the atlas cell in vertex alpha.
// Needs the mesh unpacked, see `main`.
fn cutouts(mesh: &Mesh) -> (usize, usize) {
    let Some(VertexAttributeValues::Float32x4(colors)) = mesh.attribute(Mesh::ATTRIBUTE_COLOR)
    else {
        return (0, 0);
    };
    let leaf = Tile::Leaves.atlas_cell();
    let flora = Tile::FIRST_FLORA.atlas_cell();
    let first: Vec<u32> = match mesh.indices() {
        Some(Indices::U16(i)) => i.chunks(3).map(|t| t[0] as u32).collect(),
        Some(Indices::U32(i)) => i.chunks(3).map(|t| t[0]).collect(),
        None => (0..colors.len() as u32).step_by(3).collect(),
    };
    let mut counts = (0, 0);
    for v in first {
        let alpha = colors[v as usize][3];
        if (alpha - leaf).abs() < 0.5 {
            counts.0 += 1;
        } else if alpha > flora - 0.5 {
            counts.1 += 1;
        }
    }
    counts
}

fn casters(app: &mut App) -> Vec<Caster> {
    let world = app.world_mut();
    let mut found = Vec::new();
    let mut query = world.query::<(
        &Mesh3d,
        &Aabb,
        &GlobalTransform,
        &Visibility,
        Option<&ChunkGroup>,
        Option<&LodNode>,
        // only land, as water and spray cast no shadows
        &MeshMaterial3d<ChunkMaterial>,
    )>();
    let meshes = world.resource::<Assets<Mesh>>();
    for (mesh, aabb, transform, visibility, group, node, _) in query.iter(world) {
        if *visibility == Visibility::Hidden {
            continue;
        }
        let Some(mesh) = meshes.get(&mesh.0) else {
            continue;
        };
        let triangles = mesh
            .indices()
            .map_or(mesh.count_vertices() / 3, |i| i.len() / 3);
        let (leaves, flora) = cutouts(mesh);
        let class = match (group, node) {
            (Some(_), _) => 0,
            (_, Some(node)) => node.key().level as usize,
            _ => continue,
        };
        found.push(Caster {
            aabb: *aabb,
            transform: *transform,
            triangles,
            leaves,
            flora,
            class,
        });
    }
    found
}

#[derive(Default, Clone)]
struct Tally {
    meshes: usize,
    triangles: usize,
    leaves: usize,
    flora: usize,
    by_class: [usize; LEVELS + 1],
}

fn tally(
    casters: &[Caster],
    frustum: &Frustum,
    near: bool,
    hull: Option<&[(Vec3A, f32)]>,
) -> Tally {
    let mut tally = Tally::default();
    for caster in casters {
        if frustum.intersects_obb(&caster.aabb, &caster.transform.affine(), near, true)
            && hull.is_none_or(|planes| inside(planes, caster))
        {
            tally.meshes += 1;
            tally.triangles += caster.triangles;
            tally.leaves += caster.leaves;
            tally.flora += caster.flora;
            tally.by_class[caster.class.min(LEVELS)] += caster.triangles;
        }
    }
    tally
}

// The planes, as outward normals and offsets, of the convex hull of `points`: every plane through
// three of them with all the rest on one side. Slow, but there are 16 points.
fn hull_planes(points: &[Vec3A]) -> Vec<(Vec3A, f32)> {
    let mut planes: Vec<(Vec3A, f32)> = Vec::new();
    for i in 0..points.len() {
        for j in i + 1..points.len() {
            for k in j + 1..points.len() {
                let normal = (points[j] - points[i]).cross(points[k] - points[i]);
                if normal.length() < 1e-3 {
                    continue;
                }
                let normal = normal.normalize();
                let d = -normal.dot(points[i]);
                let sides = points.iter().map(|p| normal.dot(*p) + d);
                let (above, below) =
                    sides.fold((false, false), |(a, b), s| (a || s > 1e-2, b || s < -1e-2));
                let plane = match (above, below) {
                    (false, _) => (normal, d),
                    (true, false) => (-normal, -d),
                    _ => continue,
                };
                if !planes
                    .iter()
                    .any(|(n, e)| n.dot(plane.0) > 0.9999 && (e - plane.1).abs() < 0.05)
                {
                    planes.push(plane);
                }
            }
        }
    }
    planes
}

// Whether any of `caster` lies inside the hull of `planes`.
fn inside(planes: &[(Vec3A, f32)], caster: &Caster) -> bool {
    let affine = caster.transform.affine();
    let centre = affine.transform_point3a(caster.aabb.center);
    let half = caster.aabb.half_extents;
    planes.iter().all(|(normal, d)| {
        let m = affine.matrix3;
        let radius = half.x * normal.dot(m.x_axis).abs()
            + half.y * normal.dot(m.y_axis).abs()
            + half.z * normal.dot(m.z_axis).abs();
        normal.dot(centre) + d - radius <= 0.0
    })
}

fn classes(tally: &Tally) -> String {
    tally
        .by_class
        .iter()
        .enumerate()
        .filter(|(_, &t)| t > 0)
        .map(|(class, t)| {
            let name = if class == 0 {
                "full".to_string()
            } else {
                format!("L{class}")
            };
            format!("{name} {:.0}k", *t as f32 / 1000.0)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn main() {
    let settings = Settings::read("assets/settings.toml").expect("failed to read settings");
    let (renderer, world) = (settings.renderer, settings.world);
    let args: Vec<String> = std::env::args().collect();
    let shadow_distance: f32 = args.get(1).map_or(
        (renderer.shadow_distance as f32 * 16.0).min(renderer.fog_distance()),
        |s| s.parse().unwrap(),
    );
    let cascades: usize = args
        .get(2)
        .map_or(renderer.shadow_cascades, |s| s.parse().unwrap());
    let pitch: f32 = args.get(3).map_or(0.0, |s| s.parse().unwrap());
    let far = renderer.fog_distance();

    // the leaves are found by vertex colours, which packing turns to half-floats
    ochre_terrain::pack::set_packed(false);
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins.set(task_pool_plugin()),
        AssetPlugin::default(),
        TransformPlugin,
        TerrainPlugin,
    ))
    .insert_resource(Renderer(renderer, world))
    .init_resource::<DirectionalLightShadowMap>()
    .init_asset::<Mesh>()
    .init_asset::<Image>()
    .init_asset::<ChunkMaterial>()
    .init_asset::<WaterMaterial>()
    .init_asset::<SprayMaterial>()
    .add_systems(Startup, setup);
    app.update();

    // let the start area load: nothing left to build for a few seconds running
    let start = Instant::now();
    let mut quiet_since = None;
    loop {
        app.update();
        std::thread::sleep(Duration::from_millis(10));
        let world = app.world_mut();
        let pending = world.query::<&GenerateChunkData>().iter(world).count()
            + world.query::<&GenerateChunkMesh>().iter(world).count()
            + world.query::<&GenerateLodMesh>().iter(world).count();
        if pending == 0 {
            let since = *quiet_since.get_or_insert_with(Instant::now);
            if since.elapsed() > Duration::from_secs(4) {
                break;
            }
        } else {
            quiet_since = None;
        }
        if start.elapsed() > Duration::from_secs(300) {
            println!("gave up waiting for the start area after 300 s");
            break;
        }
    }
    println!("loaded in {:.0} s", start.elapsed().as_secs_f32());

    let casters = casters(&mut app);
    let total: usize = casters.iter().map(|c| c.triangles).sum();
    println!(
        "{} terrain meshes shown, {:.2}M triangles in all; shadows reach {shadow_distance} blocks in {cascades} cascades, view reaches {far}",
        casters.len(),
        total as f32 / 1e6
    );

    // the camera, as the game's
    let (camera, eye) = {
        let world = app.world_mut();
        let (entity, transform) = world
            .query_filtered::<(Entity, &GlobalTransform), With<SimCamera>>()
            .single(world)
            .unwrap();
        (entity, *transform)
    };
    let projection = Projection::Perspective(PerspectiveProjection {
        fov: FOV_DEGREES.to_radians(),
        aspect_ratio: ASPECT,
        near: 0.1,
        far,
        ..default()
    });
    let (pitch_sin, pitch_cos) = pitch.to_radians().sin_cos();
    let view = GlobalTransform::from(
        Transform::from_translation(eye.translation())
            .looking_to(Vec3::new(pitch_cos, pitch_sin, 0.0), Vec3::Y),
    );
    app.world_mut()
        .entity_mut(camera)
        .insert((view, projection.clone()));
    let clip_from_world = projection.get_clip_from_view() * view.to_matrix().inverse();
    let main = tally(
        &casters,
        &Frustum(ViewFrustum::from_clip_from_world(&clip_from_world)),
        true,
        None,
    );
    println!(
        "\ncamera view: {} meshes, {:.0}k triangles ({})",
        main.meshes,
        main.triangles as f32 / 1000.0,
        classes(&main)
    );

    let light = app
        .world_mut()
        .spawn((
            DirectionalLight {
                shadow_maps_enabled: true,
                ..default()
            },
            CascadeShadowConfigBuilder {
                num_cascades: cascades,
                first_cascade_far_bound: FIRST_CASCADE,
                maximum_distance: shadow_distance,
                ..default()
            }
            .build(),
            Cascades::default(),
            GlobalTransform::default(),
        ))
        .id();

    let config: CascadeShadowConfig = app
        .world()
        .get::<CascadeShadowConfig>(light)
        .unwrap()
        .clone();
    // each cascade's slice of the view, as `build_directional_light_cascades` takes them
    let overlap = 1.0 - config.overlap_proportion;
    let nears =
        std::iter::once(config.minimum_distance).chain(config.bounds.iter().map(|b| overlap * b));
    let slices: Vec<[Vec3A; 8]> = nears
        .zip(config.bounds.iter())
        .map(|(near, &far)| {
            let corners = projection.get_frustum_corners(-near, -far);
            corners.map(|c| view.affine().transform_point3a(c))
        })
        .collect();

    // the camera looks along +x: the sun ahead, to the side, and behind
    let mut sums = vec![(0usize, 0usize, 0usize, 0usize); cascades];
    let mut scenarios = 0;
    for (name, azimuth) in [("ahead", 0.0f32), ("side", 90.0), ("behind", 180.0)] {
        for elevation in [5.0f32, 15.0, 30.0, 60.0, 85.0] {
            let (az, el) = (azimuth.to_radians(), elevation.to_radians());
            let toward_sun = Vec3::new(el.cos() * az.cos(), el.sin(), el.cos() * az.sin());
            let rotation = Transform::default().looking_to(-toward_sun, Vec3::Y);
            app.world_mut()
                .entity_mut(light)
                .insert(GlobalTransform::from(rotation));
            app.world_mut()
                .run_system_once(build_directional_light_cascades)
                .unwrap();
            let world = app.world();
            let built = &world.get::<Cascades>(light).unwrap().cascades[&camera];
            println!("\nsun {name}, {elevation:.0} degrees up");
            scenarios += 1;
            let (mut all, mut kept_all) = (0, 0);
            for (i, cascade) in built.iter().enumerate() {
                let frustum = Frustum(ViewFrustum::from_clip_from_world(&cascade.clip_from_world));
                // a caster can lie before the near plane, as Bevy's visibility check allows
                let t = tally(&casters, &frustum, false, None);
                // the slice and the slice swept toward the sun up to the top of the world
                let sun = Vec3A::from(toward_sun);
                let mut points: Vec<Vec3A> = slices[i].to_vec();
                for p in slices[i] {
                    let climb = (WORLD_HEIGHT as f32 - p.y).max(0.0);
                    points.push(p + sun * climb / el.sin());
                }
                let planes = hull_planes(&points);
                let kept = tally(&casters, &frustum, false, Some(&planes));
                all += t.triangles;
                kept_all += kept.triangles;
                sums[i].0 += t.triangles;
                sums[i].1 += kept.triangles;
                sums[i].2 += t.leaves + t.flora;
                sums[i].3 += t.meshes;
                println!(
                    "  cascade {i}: texel {:5.2} blocks, {:5} meshes, {:7.0}k triangles ({}), swept hull keeps {:.0}k ({:.0}%)",
                    cascade.texel_size,
                    t.meshes,
                    t.triangles as f32 / 1000.0,
                    classes(&t),
                    kept.triangles as f32 / 1000.0,
                    100.0 * kept.triangles as f32 / t.triangles.max(1) as f32,
                );
            }
            println!(
                "  shadows {:.0}k triangles in all, {:.2}x the camera's view; {:.0}k with the swept hull",
                all as f32 / 1000.0,
                all as f32 / main.triangles.max(1) as f32,
                kept_all as f32 / 1000.0,
            );
        }
    }
    let n = scenarios as f32;
    println!("\nmean over {scenarios} suns, shadow distance {shadow_distance}, {cascades} cascades (camera view: {:.0}k triangles):", main.triangles as f32 / 1000.0);
    let (mut total, mut total_kept) = (0.0, 0.0);
    for (i, (t, kept, cutout, meshes)) in sums.iter().enumerate() {
        println!(
            "  cascade {i}: {:7.0}k triangles ({:.0}% leaves or flora), {:5.0} meshes; swept hull {:.0}k",
            *t as f32 / n / 1000.0,
            100.0 * *cutout as f32 / (*t).max(1) as f32,
            *meshes as f32 / n,
            *kept as f32 / n / 1000.0
        );
        total += *t as f32 / n;
        total_kept += *kept as f32 / n;
    }
    println!(
        "  all: {:.0}k triangles ({:.2}x the view), {:.0}k with the swept hull ({:.2}x)",
        total / 1000.0,
        total / main.triangles as f32,
        total_kept / 1000.0,
        total_kept / main.triangles as f32
    );
}
