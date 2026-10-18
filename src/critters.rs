//! Butterflies flitting over the flowers by day, and fireflies blinking over the grass at
//! dusk and through the night, round wherever the camera is near the ground.
use std::f32::consts::TAU;

use bevy::{
    asset::RenderAssetUsages,
    image::Image,
    math::{ops, I64Vec3},
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};

use crate::{
    block::BlockType,
    math::ease,
    sky::{daylight, TimeOfDay},
    wings::{spawn_winged, wing_mesh, Flap, WingPlugin},
    world::World,
};

pub struct CritterPlugin;

impl Plugin for CritterPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<WingPlugin>() {
            app.add_plugins(WingPlugin);
        }
        app.add_systems(Startup, make_assets).add_systems(
            Update,
            (spawn_critters, fly_butterflies, fly_fireflies).chain(),
        );
    }
}

// How many of each are about at most, and how far from the camera they're found and let go.
const BUTTERFLIES: usize = 14;
const FIREFLIES: usize = 60;
const NEAR: f32 = 26.0;
const FAR: f32 = 44.0;
// How many spots round the camera are looked at for a new one each frame.
const SEARCHES: usize = 3;
// How long one takes to appear, or to fade away once it's the wrong time of day, in seconds.
const FADE_SECONDS: f32 = 1.5;
// How much of the day it is, see `daylight`, when butterflies come out and fireflies leave.
const BUTTERFLY_DAY: f32 = 0.55;
const FIREFLY_NIGHT: f32 = 0.35;

#[derive(Resource)]
struct CritterAssets {
    wing: Handle<Mesh>,
    body: Handle<Mesh>,
    body_material: Handle<StandardMaterial>,
    // one for each kind of butterfly
    wings: Vec<Handle<StandardMaterial>>,
    firefly: Handle<Mesh>,
    glow: Handle<StandardMaterial>,
}

/// One of them: where it keeps to, wandering round it, and how far it's faded in.
#[derive(Component)]
struct Critter {
    home: Vec3,
    // where it's drifting its home to, see `fly_butterflies`
    heading_for: Vec3,
    phase: f32,
    life: f32,
    leaving: bool,
}

#[derive(Component)]
struct Butterfly {
    // how fast it circles and flaps, so no two keep time
    pace: f32,
    // how long until it picks another flower to go to
    restless: f32,
}

#[derive(Component)]
struct Firefly;

// Butterflies' colours: the wing texture's white takes them, its black edges stay black.
const BUTTERFLY_COLOURS: [Color; 5] = [
    Color::srgb(1.0, 0.55, 0.1),
    Color::srgb(0.98, 0.97, 0.92),
    Color::srgb(0.35, 0.6, 1.0),
    Color::srgb(1.0, 0.88, 0.25),
    Color::srgb(0.85, 0.45, 0.9),
];

fn make_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let texture = images.add(wing_texture());
    let wings = BUTTERFLY_COLOURS
        .iter()
        .map(|&colour| {
            materials.add(StandardMaterial {
                base_color: colour,
                base_color_texture: Some(texture.clone()),
                alpha_mode: AlphaMode::Mask(0.5),
                double_sided: true,
                cull_mode: None,
                perceptual_roughness: 0.9,
                ..default()
            })
        })
        .collect();
    commands.insert_resource(CritterAssets {
        // from the hinge on the body out to the tip, its leading edge forward at -z
        wing: meshes.add(wing_mesh([
            [0.0, 0.0, -0.3],
            [0.45, 0.0, -0.3],
            [0.0, 0.0, 0.26],
            [0.45, 0.0, 0.26],
        ])),
        body: meshes.add(Cuboid::new(0.045, 0.045, 0.3)),
        body_material: materials.add(StandardMaterial {
            base_color: Color::srgb(0.08, 0.06, 0.05),
            perceptual_roughness: 0.9,
            ..default()
        }),
        wings,
        firefly: meshes.add(Sphere::new(0.06).mesh().ico(1).unwrap()),
        // bright past white, so the bloom haloes it, and as bright whatever the exposure
        glow: materials.add(StandardMaterial {
            base_color: Color::BLACK,
            emissive: LinearRgba::rgb(1.8, 3.2, 0.25),
            emissive_exposure_weight: 0.0,
            ..default()
        }),
    });
}

// A 16px wing, fore and hind lobes, white to take the butterfly's colour, with dark edges and
// a pale spot. u runs from the body out, v from front to back.
fn wing_texture() -> Image {
    const SIZE: usize = 16;
    let mut data = vec![0u8; SIZE * SIZE * 4];
    // centre and radii, in texels, of the two lobes
    let lobes = [((8.5, 5.0), (7.5, 5.0)), ((6.0, 11.5), (5.5, 4.0))];
    let inside = |x: f32, y: f32, grow: f32| {
        lobes.iter().any(|&((cx, cy), (rx, ry))| {
            let (dx, dy) = ((x - cx) / (rx + grow), (y - cy) / (ry + grow));
            dx * dx + dy * dy <= 1.0
        })
    };
    for y in 0..SIZE {
        for x in 0..SIZE {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            if !inside(fx, fy, 0.0) {
                continue;
            }
            let edge = !inside(fx, fy, -1.3);
            let spot = (fx - 12.0).powi(2) + (fy - 4.0).powi(2) < 2.2;
            // veins from the body darken the wing a little toward it
            let vein = 0.82 + 0.18 * (fx / SIZE as f32);
            let shade = if spot {
                1.0
            } else if edge {
                0.08
            } else {
                vein
            };
            let v = (shade * 255.0) as u8;
            let at = (y * SIZE + x) * 4;
            data[at..at + 4].copy_from_slice(&[v, v, v, 255]);
        }
    }
    Image::new(
        Extent3d {
            width: SIZE as u32,
            height: SIZE as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

// A spot on the ground near `eye` whose top block passes `wanted`, anywhere round it or mostly
// ahead along `facing`, so new ones turn up where they'll be seen.
fn find_spot(
    world: &World,
    eye: Vec3,
    facing: Option<Vec3>,
    wanted: impl Fn(BlockType) -> bool,
) -> Option<Vec3> {
    let angle = match facing {
        Some(facing) => ops::atan2(facing.z, facing.x) + rand::random_range(-1.1..1.1),
        None => rand::random_range(0.0..TAU),
    };
    let reach = rand::random_range(3.0..NEAR);
    let (x, z) = (
        (eye.x + ops::cos(angle) * reach).floor() as i64,
        (eye.z + ops::sin(angle) * reach).floor() as i64,
    );
    let top = eye.y.floor() as i64 + 12;
    for y in (top - 52..=top).rev() {
        let block = world.get_block_at(I64Vec3::new(x, y, z));
        if block == BlockType::Air {
            continue;
        }
        return wanted(block).then(|| Vec3::new(x as f32 + 0.5, y as f32, z as f32 + 0.5));
    }
    None
}

fn is_flower(block: BlockType) -> bool {
    matches!(block, BlockType::Poppy | BlockType::Dandelion)
}

fn is_meadow(block: BlockType) -> bool {
    block == BlockType::Grass || block.is_flora()
}

fn spawn_critters(
    mut commands: Commands,
    assets: Option<Res<CritterAssets>>,
    world: Res<World>,
    clock: Res<TimeOfDay>,
    camera: Query<&GlobalTransform, With<Camera3d>>,
    mut critters: Query<(Entity, &mut Critter, &Transform, Has<Butterfly>)>,
) {
    let (Some(assets), Ok(camera)) = (assets, camera.single()) else {
        return;
    };
    let eye = camera.translation();
    let ahead = Some(camera.forward().with_y(0.0));
    let day = daylight(&clock);
    let underwater = world.get_block_at(eye.floor().as_i64vec3()) == BlockType::Water;
    let (mut butterflies, mut fireflies) = (0, 0);
    for (entity, mut critter, transform, is_butterfly) in &mut critters {
        let wrong_time = if is_butterfly {
            day < BUTTERFLY_DAY
        } else {
            day > FIREFLY_NIGHT
        };
        if wrong_time || underwater {
            critter.leaving = true;
        }
        if transform.translation.distance(eye) > FAR || (critter.leaving && critter.life <= 0.0) {
            commands.entity(entity).despawn();
            continue;
        }
        if is_butterfly {
            butterflies += 1;
        } else {
            fireflies += 1;
        }
    }
    if underwater {
        return;
    }
    for _ in 0..SEARCHES {
        if day >= BUTTERFLY_DAY && butterflies < BUTTERFLIES {
            if let Some(flower) = find_spot(&world, eye, ahead, is_flower) {
                spawn_butterfly(&mut commands, &assets, flower + Vec3::Y * 1.2);
                butterflies += 1;
            }
        }
        if day <= FIREFLY_NIGHT && fireflies < FIREFLIES {
            if let Some(grass) = find_spot(&world, eye, ahead, is_meadow) {
                let home = grass + Vec3::Y * rand::random_range(1.0..2.6);
                commands.spawn((
                    critter(home),
                    Firefly,
                    Mesh3d(assets.firefly.clone()),
                    MeshMaterial3d(assets.glow.clone()),
                    Transform::from_translation(home).with_scale(Vec3::ZERO),
                    bevy::light::NotShadowCaster,
                ));
                fireflies += 1;
            }
        }
    }
}

fn critter(home: Vec3) -> Critter {
    Critter {
        home,
        heading_for: home,
        phase: rand::random_range(0.0..TAU),
        life: 0.0,
        leaving: false,
    }
}

fn spawn_butterfly(commands: &mut Commands, assets: &CritterAssets, home: Vec3) {
    let kind = rand::random_range(0..assets.wings.len());
    let pace = rand::random_range(0.8..1.25);
    spawn_winged(
        commands,
        (
            critter(home),
            Butterfly {
                pace,
                restless: rand::random_range(4.0..12.0),
            },
        ),
        home,
        (&assets.body, &assets.body_material),
        (&assets.wing, &assets.wings[kind]),
        // quick, deep beats, wings held up a little between them
        Flap {
            beat: 19.0 * pace,
            sweep: 0.95,
            lift: 0.25,
            glides: false,
        },
    );
}

// Fades a critter in, or out once it's leaving, and gives how big it is for that, 0..1.
fn fade(critter: &mut Critter, dt: f32) -> f32 {
    let step = dt / FADE_SECONDS;
    critter.life = if critter.leaving {
        (critter.life - step).max(0.0)
    } else {
        (critter.life + step).min(1.0)
    };
    ease(critter.life)
}

// Where a butterfly is at `t` seconds: looping round its home, bobbing and dipping to land.
fn butterfly_at(home: Vec3, phase: f32, pace: f32, t: f32) -> Vec3 {
    let t = t * pace;
    let angle = phase + t * 0.9 + 0.6 * ops::sin(t * 0.37 + phase);
    let radius = 1.2 + 0.8 * ops::sin(t * 0.53 + phase * 2.0);
    let bob = 0.35 * ops::sin(t * 2.6 + phase) + 0.12 * ops::sin(t * 7.3)
        - 0.5 * ops::sin(t * 0.21 + phase).max(0.0).powi(4);
    home + Vec3::new(
        ops::cos(angle) * radius,
        bob,
        ops::sin(angle * 1.3) * radius * 0.8,
    )
}

fn fly_butterflies(
    time: Res<Time>,
    world: Res<World>,
    camera: Query<&GlobalTransform, With<Camera3d>>,
    mut butterflies: Query<(&mut Critter, &mut Butterfly, &mut Transform)>,
) {
    let (t, dt) = (time.elapsed_secs(), time.delta_secs());
    let eye = camera.single().map(|c| c.translation()).unwrap_or_default();
    for (mut critter, mut butterfly, mut transform) in &mut butterflies {
        // now and then it's off to another flower nearby
        butterfly.restless -= dt;
        if butterfly.restless <= 0.0 {
            butterfly.restless = rand::random_range(6.0..16.0);
            if let Some(flower) = find_spot(&world, eye, None, is_flower) {
                if flower.distance(critter.home) < 12.0 {
                    critter.heading_for = flower + Vec3::Y * 1.2;
                }
            }
        }
        let drift = 1.0 - ops::exp(-dt * 0.4);
        critter.home = critter.home.lerp(critter.heading_for, drift);

        let size = fade(&mut critter, dt);
        let (here, ahead) = (
            butterfly_at(critter.home, critter.phase, butterfly.pace, t),
            butterfly_at(critter.home, critter.phase, butterfly.pace, t + 0.1),
        );
        transform.translation = here;
        let course = (ahead - here).with_y(0.0);
        if course.length_squared() > 1e-6 {
            transform.look_to(course, Vec3::Y);
        }
        transform.scale = Vec3::splat(size);
    }
}

fn fly_fireflies(
    time: Res<Time>,
    mut fireflies: Query<(&mut Critter, &mut Transform), With<Firefly>>,
) {
    let (t, dt) = (time.elapsed_secs(), time.delta_secs());
    for (mut critter, mut transform) in &mut fireflies {
        let size = fade(&mut critter, dt);
        let p = critter.phase;
        // a slow, lazy drift, rising and sinking
        transform.translation = critter.home
            + Vec3::new(
                1.6 * ops::sin(t * 0.23 + p) + 0.4 * ops::sin(t * 0.9 + p * 3.0),
                0.5 * ops::sin(t * 0.31 + p * 2.0),
                1.6 * ops::cos(t * 0.19 + p * 1.7) + 0.4 * ops::cos(t * 0.8 + p),
            );
        // each glows up and fades in its own time, dark more than lit
        let blink = ops::sin(t * (0.9 + 0.3 * ops::sin(p)) + p * 5.0)
            .max(0.0)
            .powi(3);
        transform.scale = Vec3::splat(size * (0.15 + 0.85 * blink));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_butterflies_keep_near_their_flower() {
        let home = Vec3::new(10.0, 20.0, -4.0);
        for i in 0..2000 {
            let at = butterfly_at(home, 1.3, 1.1, i as f32 * 0.05);
            assert!(at.with_y(0.0).distance(home.with_y(0.0)) < 2.6, "{at}");
            assert!((at.y - home.y).abs() < 1.0, "{at}");
        }
    }
}
