use std::sync::LazyLock;

use std::f32::consts::TAU;

use bevy::{
    asset::{Asset, Assets, Handle},
    camera::{visibility::NoFrustumCulling, Camera},
    ecs::hierarchy::ChildOf,
    ecs::{
        bundle::Bundle,
        component::Component,
        message::MessageReader,
        query::{With, Without},
        resource::Resource,
        system::{Local, Query, Res, ResMut},
    },
    input::{keyboard::KeyCode, mouse::MouseMotion, ButtonInput},
    light::{NotShadowCaster, NotShadowReceiver},
    log::info,
    math::{Dir3, EulerRot, I64Vec3, Quat, Vec3, Vec3Swizzles},
    mesh::{Mesh, Mesh3d, MeshVertexBufferLayoutRef},
    pbr::{
        DistanceFog, FogFalloff, Material, MaterialPipeline, MaterialPipelineKey, MeshMaterial3d,
    },
    post_process::dof::{DepthOfField, DepthOfFieldMode},
    prelude::{default, Alpha, ClearColor, Color, LinearRgba, Meshable, Sphere, Transform},
    reflect::TypePath,
    render::{
        render_resource::{AsBindGroup, RenderPipelineDescriptor, SpecializedMeshPipelineError},
        view::ColorGrading,
    },
    shader::ShaderRef,
    time::Time,
};

use crate::collision::{move_and_collide, Aabb};
use bevy::{
    anti_alias::taa::TemporalAntiAliasing,
    app::{App, Plugin, Update},
    camera::{Camera3d, Hdr, PerspectiveProjection, Projection},
    core_pipeline::prepass::{DepthPrepass, MotionVectorPrepass},
    ecs::{
        schedule::IntoScheduleConfigs,
        system::{Commands, SystemParam},
    },
    pbr::{MaterialPlugin, ScreenSpaceTransmission},
    post_process::{bloom::Bloom, effect_stack::Vignette},
    render::view::Msaa,
};
use ochre_base::{block::BlockType, WATER_LEVEL, WORLD_HEIGHT};
use ochre_terrain::{
    chunk::{ChunkCoordinate, CHUNK_SIZE},
    horizon::update_horizon,
    settings::RendererSettings,
    world::World,
};
use ochre_world::{
    sky::{atmosphere_camera, FOG_MAX_OPACITY},
    tonemap::{dry_color_grading, scene_colour, TONEMAPPING},
};

/// The player's body, at its feet, with the camera at its eyes.
#[derive(Component, Default)]
pub struct Player {
    pub movement: Movement,
    velocity: Vec3,
    grounded: bool,
    // whether it walked into a wall last frame, so swimming against a bank climbs out
    against_wall: bool,
    // how far through its stride the head's bob is, in radians a pair of steps, and how big the bob is now
    bob_phase: f32,
    bob: f32,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Movement {
    /// Flying anywhere, through anything, the way the camera is facing.
    #[default]
    Fly,
    /// Walking under gravity, kept out of solid blocks.
    Walk,
}

const FLY_TOGGLE_KEY: KeyCode = KeyCode::KeyF;

// Blocks a second, and how much faster while Shift is held.
const MOVE_SPEED: f32 = 20.0;
const SPRINT_MULTIPLIER: f32 = 8.0;
const LOOK_SENSITIVITY: f32 = 0.1;
// Just short of straight up or down, where the view would flip over.
const MAX_PITCH: f32 = 89.0_f32.to_radians();

/// How high the camera sits above the player's feet, far enough under the top of its head
/// that the camera's near plane stays out of the ceiling.
pub const EYE_HEIGHT: f32 = 1.9;
// The player's size: two blocks tall, so it just fits through a two-block gap.
const BODY: Aabb = Aabb {
    min: Vec3::new(-0.3, 0.0, -0.3),
    max: Vec3::new(0.3, 2.0, 0.3),
};
// Walking, blocks a second, and how much faster while Shift is held.
const WALK_SPEED: f32 = 5.0;
const WALK_SPRINT_MULTIPLIER: f32 = 1.5;
// Blocks a second squared, and the speed falling tops out at.
const GRAVITY: f32 = 32.0;
const TERMINAL_SPEED: f32 = 60.0;
// Enough to clear one block, not two.
const JUMP_SPEED: f32 = 9.0;
// In water: slower across, sinking slowly, Space swimming up.
const SWIM_MULTIPLIER: f32 = 0.6;
const WATER_GRAVITY: f32 = 8.0;
const SINK_SPEED: f32 = 2.0;
const SWIM_UP_SPEED: f32 = 4.0;
// Head bob walking and sprinting in blocks, blocks walked a pair of steps, and how fast it eases in and out a second.
const BOB_WALK: f32 = 0.05;
const BOB_SPRINT: f32 = 0.065;
const BOB_STRIDE: f32 = 2.6;
// Sprinting takes longer strides, so its bob is only a little quicker than walking's.
const BOB_SPRINT_STRIDE: f32 = 3.4;
const BOB_EASE: f32 = 6.0;
// How fast, a share a second, the player's speed across turns toward what's wanted in the air.
const AIR_CONTROL: f32 = 3.0;
// How far up the body the water must reach for the player to be swimming.
const SWIM_CHEST: f32 = 0.6;
// Blocks are drawn centred on their coordinates, and collision takes a block to start at its coordinate.
const BLOCK_CORNER: Vec3 = Vec3::splat(0.5);
// The longest frame moved in one step, so a hitch doesn't fling the player.
const MAX_STEP: f32 = 0.1;

type PlayerCamera<'a> = (&'a ChildOf, &'a mut Transform);

#[allow(clippy::type_complexity)]
fn player_move(
    time: Res<Time>,
    world: Res<World>,
    mut player_query: Query<(&mut Transform, &mut Player)>,
    mut camera_query: Query<(&ChildOf, &mut Transform), (With<Camera>, Without<Player>)>,
    keys: Res<ButtonInput<KeyCode>>,
) {
    let (parent, mut camera) = camera_query.single_mut().expect("camera does not exist");
    let (mut transform, mut player) = player_query
        .get_mut(parent.parent())
        .expect("player does not exist");
    if keys.just_pressed(FLY_TOGGLE_KEY) {
        player.movement = match player.movement {
            Movement::Walk => Movement::Fly,
            Movement::Fly => Movement::Walk,
        };
        player.velocity = Vec3::ZERO;
        info!("{:?}ing", player.movement);
    }
    let dt = time.delta_secs().min(MAX_STEP);
    let axis = |negative, positive| {
        if keys.pressed(negative) {
            -1.0
        } else if keys.pressed(positive) {
            1.0
        } else {
            0.0
        }
    };
    let across = Vec3::new(
        axis(KeyCode::KeyA, KeyCode::KeyD),
        0.0,
        axis(KeyCode::KeyW, KeyCode::KeyS),
    );
    let sprinting = keys.pressed(KeyCode::ShiftLeft);

    match player.movement {
        Movement::Fly => {
            let speed = MOVE_SPEED * dt * if sprinting { SPRINT_MULTIPLIER } else { 1.0 };
            let up = -axis(KeyCode::Space, KeyCode::ControlLeft);
            let step = transform.rotation * camera.rotation * across + Vec3::Y * up;
            transform.translation += step * speed;
        }
        Movement::Walk => walk(
            &world,
            &mut transform,
            &mut player,
            across,
            sprinting,
            &keys,
            dt,
        ),
    }

    // the head bobs a little walking and more sprinting, as far through a stride as the player has walked
    let across_speed = player.velocity.xz().length();
    let striding = player.movement == Movement::Walk && player.grounded && across_speed > 0.1;
    let target = match (striding, sprinting) {
        (false, _) => 0.0,
        (true, false) => BOB_WALK,
        (true, true) => BOB_SPRINT,
    };
    player.bob += (target - player.bob) * (BOB_EASE * dt).min(1.0);
    let stride = if sprinting {
        BOB_SPRINT_STRIDE
    } else {
        BOB_STRIDE
    };
    player.bob_phase = (player.bob_phase + across_speed * dt * TAU / stride) % TAU;
    camera.translation = Vec3::new(
        player.bob * 0.5 * player.bob_phase.sin(),
        EYE_HEIGHT - player.bob * (1.0 - (2.0 * player.bob_phase).cos()) / 2.0,
        0.0,
    );
}

fn walk(
    world: &World,
    transform: &mut Transform,
    player: &mut Player,
    across: Vec3,
    sprinting: bool,
    keys: &ButtonInput<KeyCode>,
    dt: f32,
) {
    // swimming until the chest is out, so the head bobs well clear of the water
    let chest = transform.translation + Vec3::Y * SWIM_CHEST;
    let swimming = world.get_block_at(chest.round().as_i64vec3()) == BlockType::Water;
    // just over the feet, so wading through a shallow river slows the player as swimming does
    let feet = transform.translation + Vec3::Y * 0.1;
    let wading = world.get_block_at(feet.round().as_i64vec3()) == BlockType::Water;
    let jumping = keys.pressed(KeyCode::Space);

    let mut speed = WALK_SPEED;
    if sprinting {
        speed *= WALK_SPRINT_MULTIPLIER;
    }
    if swimming || wading {
        speed *= SWIM_MULTIPLIER;
    }
    // where the body faces, not the camera, so looking down doesn't slow it
    let wanted = transform.rotation * across.normalize_or_zero() * speed;
    // in the air the player keeps its way and only steers toward where it's heading, as the air holds it back
    let grip = if player.grounded || swimming || wading {
        1.0
    } else {
        (AIR_CONTROL * dt).min(1.0)
    };
    player.velocity.x += (wanted.x - player.velocity.x) * grip;
    player.velocity.z += (wanted.z - player.velocity.z) * grip;

    if swimming {
        player.velocity.y = if jumping && player.against_wall {
            JUMP_SPEED
        } else if jumping {
            SWIM_UP_SPEED
        } else {
            (player.velocity.y - WATER_GRAVITY * dt).max(-SINK_SPEED)
        };
    } else {
        if jumping && player.grounded {
            player.velocity.y = JUMP_SPEED;
        }
        // falling with the feet still in water is gentle, so the head stays up a while, but a jump out of shallows isn't
        let gravity = if wading && player.velocity.y <= 0.0 {
            WATER_GRAVITY
        } else {
            GRAVITY
        };
        player.velocity.y = (player.velocity.y - gravity * dt).max(-TERMINAL_SPEED);
    }

    // Chunks not generated yet are solid, so the player waits where it is for the ground to
    // load rather than falling through it.
    let solid = |block: I64Vec3| {
        !world.is_chunk_generated(ChunkCoordinate::containing(block))
            || world.get_block_at(block).is_solid()
    };
    let (position, hit) = move_and_collide(
        transform.translation + BLOCK_CORNER,
        player.velocity * dt,
        BODY,
        solid,
    );
    transform.translation = position - BLOCK_CORNER;
    player.grounded = hit.y && player.velocity.y < 0.0;
    if hit.y {
        player.velocity.y = 0.0;
    }
    if hit.x {
        player.velocity.x = 0.0;
    }
    if hit.z {
        player.velocity.z = 0.0;
    }
    player.against_wall = (hit.x || hit.z) && across != Vec3::ZERO;
}

fn player_look(
    time: Res<Time>,
    mut player_query: Query<&mut Transform, With<Player>>,
    mut camera_query: Query<PlayerCamera, (With<Camera>, Without<Player>)>,
    mut motion_evr: MessageReader<MouseMotion>,
) {
    let (parent, mut camera) = camera_query.single_mut().expect("camera does not exist");
    let mut player = player_query
        .get_mut(parent.parent())
        .expect("player does not exist");
    let turn = LOOK_SENSITIVITY * time.delta_secs();
    // the camera only ever turns about x, under the body's turn about y
    let (mut pitch, _, _) = camera.rotation.to_euler(EulerRot::XYZ);
    for ev in motion_evr.read() {
        player.rotate_axis(Dir3::Y, -ev.delta.x * turn);
        pitch = (pitch - ev.delta.y * turn).clamp(-MAX_PITCH, MAX_PITCH);
    }
    camera.rotation = Quat::from_rotation_x(pitch);
}

pub struct DryDefaults {
    fog: DistanceFog,
    dof: DepthOfField,
    color_grading: ColorGrading,
    clear_color: Color,
}

// The fog's colour underwater, picked as it should look on screen.
static UNDERWATER_COLOR: LazyLock<Color> =
    LazyLock::new(|| scene_colour(Color::srgb_u8(20, 90, 130)).into());
// How far round the camera the murk hangs, in blocks: past the underwater fog's end, where
// everything is already its colour, so it only hides the sky.
const MURK_DISTANCE: f32 = 48.0;
// Where the fog underwater in a river or lake is thick enough to hide everything, in blocks.
const FRESH_FOG_END: f32 = 160.0;

/// The sky isn't fogged, so underwater it's hidden behind a sphere of the fog's colour hung
/// round the camera. It's always drawn, and only shows when its alpha is 1, so going under
/// changes nothing but a uniform: taking the atmosphere off the camera instead changed what
/// every material's shaders were compiled with, which stalled the first dive recompiling
/// them all.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct MurkMaterial {
    #[uniform(0)]
    color: LinearRgba,
}

impl Material for MurkMaterial {
    fn fragment_shader() -> ShaderRef {
        "shaders/murk.wesl".into()
    }

    fn enable_prepass() -> bool {
        false
    }

    fn enable_shadows() -> bool {
        false
    }

    // seen from inside
    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

#[derive(Resource)]
pub struct Murk(Handle<MurkMaterial>);

/// The murk, hidden, to hang from the camera, and the handle `update_underwater_effects`
/// shows it by.
fn murk(meshes: &mut Assets<Mesh>, materials: &mut Assets<MurkMaterial>) -> (impl Bundle, Murk) {
    let material = materials.add(MurkMaterial {
        color: LinearRgba::from(*UNDERWATER_COLOR).with_alpha(0.0),
    });
    let bundle = (
        Mesh3d(meshes.add(Sphere::new(MURK_DISTANCE).mesh().ico(3).unwrap())),
        MeshMaterial3d(material.clone()),
        Transform::default(),
        NotShadowCaster,
        NotShadowReceiver,
        // it's round the camera, so always in view
        NoFrustumCulling,
    );
    (bundle, Murk(material))
}

// Snapshots whichever DistanceFog/DepthOfField/ClearColor were active right before the camera
// entered water, and restores that exact snapshot on exit, instead of hardcoding "dry" defaults
// that could drift out of sync with setup_scene's render-distance-derived fog settings.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn update_underwater_effects(
    world: Res<World>,
    murk: Res<Murk>,
    mut murk_materials: ResMut<Assets<MurkMaterial>>,
    mut clear_color: ResMut<ClearColor>,
    player_query: Query<&Transform, With<Player>>,
    mut camera_query: Query<
        (
            &ChildOf,
            &Transform,
            &mut DistanceFog,
            &mut DepthOfField,
            &mut ColorGrading,
        ),
        With<Camera>,
    >,
    mut dry_defaults: Local<Option<DryDefaults>>,
    mut murk_shown: Local<Option<bool>>,
) {
    let (parent, camera_transform, mut fog, mut dof, mut color_grading) =
        camera_query.single_mut().expect("camera does not exist");
    let player_transform = player_query
        .get(parent.parent())
        .expect("player does not exist");

    let eye = player_transform.translation + camera_transform.translation;
    if world.get_block_at(eye.round().as_i64vec3()) == BlockType::Water {
        if dry_defaults.is_none() {
            *dry_defaults = Some(DryDefaults {
                fog: fog.clone(),
                dof: *dof,
                color_grading: color_grading.clone(),
                clear_color: clear_color.0,
            });
        }
        // rivers and lakes are clear: the sky's seen out of them, and things farther off sharp
        let fresh = eye.y > WATER_LEVEL as f32 + 1.0;
        if *murk_shown != Some(!fresh) {
            show_murk(&murk, &mut murk_materials, !fresh);
            *murk_shown = Some(!fresh);
        }
        // Matches ClearColor to the fog color, same trick setup_scene uses above water (its
        // ClearColor equals the dry fog's color) - otherwise empty/unloaded space shows the old
        // background color straight through, breaking the underwater illusion.
        let underwater_color = *UNDERWATER_COLOR;
        *fog = DistanceFog {
            color: underwater_color,
            falloff: FogFalloff::Linear {
                start: 0.0,
                end: if fresh { FRESH_FOG_END } else { 32.0 },
            },
            ..default()
        };
        *dof = DepthOfField {
            mode: DepthOfFieldMode::Gaussian,
            focal_distance: if fresh { 4.0 } else { 1.5 },
            aperture_f_stops: if fresh { 8.0 } else { 0.5 },
            ..default()
        };
        // The fog's colour is already the water's: a white balance shift on top, like the
        // -0.4 this once had, moves the white point off the chromaticity diagram, where Bevy's
        // balance matrix turns blue into olive.
        color_grading.global.post_saturation = 0.85;
        clear_color.0 = underwater_color;
    } else if let Some(dry) = dry_defaults.take() {
        *fog = dry.fog;
        *dof = dry.dof;
        *color_grading = dry.color_grading;
        clear_color.0 = dry.clear_color;
        show_murk(&murk, &mut murk_materials, false);
        *murk_shown = None;
    }
}

fn show_murk(murk: &Murk, materials: &mut Assets<MurkMaterial>, shown: bool) {
    if let Some(mut material) = materials.get_mut(&murk.0) {
        material.color.alpha = if shown { 1.0 } else { 0.0 };
    }
}
/// Walking and flying, looking round, and the view underwater.
pub struct PlayerPlugin;

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<MurkMaterial>::default())
            .add_systems(
                Update,
                (
                    player_move,
                    player_look,
                    update_horizon,
                    update_underwater_effects,
                )
                    .chain(),
            );
    }
}

#[derive(SystemParam)]
pub struct PlayerAssets<'w> {
    meshes: ResMut<'w, Assets<Mesh>>,
    murk: ResMut<'w, Assets<MurkMaterial>>,
}

/// Spawns the player at `position` facing `look`, with a camera that sees out to the renderer's
/// fog distance, and the murk the camera sees through underwater.
pub fn spawn_player(
    commands: &mut Commands,
    assets: &mut PlayerAssets,
    renderer: &RendererSettings,
    position: Vec3,
    look: Vec3,
) {
    let fog_distance = renderer.fog_distance();
    let player = commands
        .spawn((
            Player::default(),
            Transform::from_translation(position).looking_to(look, Vec3::Y),
        ))
        .id();
    let camera = commands
        .spawn((
            Transform::from_xyz(0.0, EYE_HEIGHT, 0.0),
            Camera3d::default(),
            atmosphere_camera(),
            Projection::Perspective(PerspectiveProjection {
                // vertical, as Minecraft's field of view setting is
                fov: 90f32.to_radians(),
                far: fog_distance + WORLD_HEIGHT as f32,
                ..default()
            }),
            // Depth of field, bloom and sunlight brighter than white all need HDR.
            Camera::default(),
            Hdr,
            TONEMAPPING,
            // Just a soft glow around the brightest sunlit faces and sky.
            Bloom {
                intensity: 0.05,
                ..Bloom::NATURAL
            },
            // TAA for the thin faces and one-block steps that shimmer at a distance
            Msaa::Off,
            // the transmission copy of the scene is what fresh water refracts and reflects
            (
                DepthPrepass,
                MotionVectorPrepass,
                ScreenSpaceTransmission::default(),
            ),
            TemporalAntiAliasing::default(),
            DistanceFog {
                color: Color::NONE.with_alpha(FOG_MAX_OPACITY),
                falloff: FogFalloff::Linear {
                    // half fogged where the farthest LOD level starts, hiding the swap to it,
                    // see `fog.wesl`
                    start: renderer.lod_layout().last_level_start() * CHUNK_SIZE as f32,
                    end: fog_distance,
                },
                ..default()
            },
        ))
        .insert((
            // In focus 8 blocks out, blurred ~1px at 16 blocks and ~2px from 150 on, so LOD seams soften. Gaussian is cheaper than Bokeh.
            DepthOfField {
                mode: DepthOfFieldMode::Gaussian,
                focal_distance: 8.0,
                aperture_f_stops: 2.2,
                ..default()
            },
            dry_color_grading(),
            // a light darkening toward the corners, which draws the eye in
            Vignette {
                intensity: 0.35,
                radius: 0.9,
                smoothness: 3.0,
                ..default()
            },
            ChildOf(player),
        ))
        .id();
    let (murk, handle) = murk(&mut assets.meshes, &mut assets.murk);
    commands.spawn((murk, ChildOf(camera)));
    commands.insert_resource(handle);
}
