use bevy::{
    light::{NotShadowCaster, NotShadowReceiver},
    log::info,
    prelude::*,
};

use crate::player::{cursor_grabbed, grab_cursor};
use ochre_base::block::BlockType;
use ochre_terrain::{
    raycast::{raycast, RayHit},
    world::World,
};

/// How far from the eye the player can reach a block, in blocks.
pub const REACH: f32 = 7.0;
const BREAK_BUTTON: MouseButton = MouseButton::Left;
// How far out from the block the outline stands, so it isn't lost in the block's faces, and how
// thick its edges are, in blocks.
const OUTLINE_GAP: f32 = 0.004;
const OUTLINE_WIDTH: f32 = 0.012;

/// The player looks at blocks, outlined, and breaks them with a click.
pub struct BreakingPlugin;

impl Plugin for BreakingPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LookedAt>()
            .add_systems(Startup, spawn_outline)
            // what's broken is looked past at once, and outlined the same frame
            .add_systems(
                Update,
                (
                    // the click that grabs the cursor doesn't break a block too
                    break_looked_at.run_if(cursor_grabbed).before(grab_cursor),
                    look_at_block,
                    show_outline,
                )
                    .chain(),
            );
    }
}

/// The block the player's looking at in reach, and what it is.
#[derive(Resource, Default, Debug)]
pub struct LookedAt(pub Option<(RayHit, BlockType)>);

/// The first block from `eye` along `forward`, past air and water, within `REACH`.
pub fn block_looked_at(world: &World, eye: Vec3, forward: Vec3) -> Option<(RayHit, BlockType)> {
    raycast(eye, forward, REACH, |block| {
        world.get_block_at(block).is_targetable()
    })
    .map(|hit| (hit, world.get_block_at(hit.block)))
}

fn look_at_block(
    world: Option<Res<World>>,
    camera: Query<&GlobalTransform, (With<Camera3d>, With<ChildOf>)>,
    mut looked_at: ResMut<LookedAt>,
) {
    let (Some(world), Ok(camera)) = (world, camera.single()) else {
        looked_at.0 = None;
        return;
    };
    looked_at.0 = block_looked_at(&world, camera.translation(), *camera.forward());
}

fn break_looked_at(
    buttons: Res<ButtonInput<MouseButton>>,
    looked_at: Res<LookedAt>,
    world: Option<ResMut<World>>,
) {
    let (Some(mut world), Some((hit, _))) = (world, looked_at.0) else {
        return;
    };
    if !buttons.just_pressed(BREAK_BUTTON) {
        return;
    }
    if let Some(broken) = world.break_block(hit.block) {
        info!("broke {} at {}", broken.name(), hit.block);
    }
}

#[derive(Component)]
struct BlockOutline;

// The edges of a block, as thin bars, centred on the origin.
fn outline_mesh() -> Mesh {
    let half = 0.5 + OUTLINE_GAP + OUTLINE_WIDTH / 2.0;
    let mut outline: Option<Mesh> = None;
    for axis in 0..3 {
        let mut size = Vec3::splat(OUTLINE_WIDTH);
        size[axis] = 2.0 * half + OUTLINE_WIDTH;
        for corner in [(-1.0, -1.0), (-1.0, 1.0), (1.0, -1.0), (1.0, 1.0)] {
            let mut centre = Vec3::ZERO;
            centre[(axis + 1) % 3] = corner.0 * half;
            centre[(axis + 2) % 3] = corner.1 * half;
            let bar = Cuboid::from_size(size).mesh().build().translated_by(centre);
            match &mut outline {
                Some(mesh) => mesh.merge(&bar).expect("cuboids share attributes"),
                None => outline = Some(bar),
            }
        }
    }
    outline.expect("twelve edges")
}

fn spawn_outline(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        BlockOutline,
        Mesh3d(meshes.add(outline_mesh())),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.05, 0.05, 0.05),
            unlit: true,
            fog_enabled: false,
            ..default()
        })),
        Transform::default(),
        NotShadowCaster,
        NotShadowReceiver,
        Visibility::Hidden,
    ));
}

fn show_outline(
    looked_at: Res<LookedAt>,
    mut outline: Query<(&mut Transform, &mut Visibility), With<BlockOutline>>,
) {
    let Ok((mut transform, mut visibility)) = outline.single_mut() else {
        return;
    };
    match looked_at.0 {
        Some((hit, _)) => {
            // blocks are drawn centred on their coordinates
            transform.translation = hit.block.as_vec3();
            visibility.set_if_neq(Visibility::Inherited);
        }
        None => {
            visibility.set_if_neq(Visibility::Hidden);
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy::math::{I64Vec3, U16Vec3, Vec3};

    use super::{block_looked_at, outline_mesh, REACH};
    use ochre_base::block::BlockType;
    use ochre_terrain::chunk::{ChunkCoordinate, ChunkData};
    use ochre_terrain::world::World;

    // A chunk at the origin with a column of blocks, from the bottom up, at x = z = 4.
    fn world_with_column(column: &[BlockType]) -> World {
        let mut world = World::with_terrain(1, 16, ochre_world::worldmap::test_terrain(1));
        let mut data = ChunkData::default();
        for (y, &block) in column.iter().enumerate() {
            data.set_block_at(U16Vec3::new(4, y as u16, 4), block);
        }
        world.insert_chunk(ChunkCoordinate(I64Vec3::ZERO), data);
        world
    }

    #[test]
    fn test_looks_through_water_to_the_block_under_it() {
        use BlockType::*;
        let world = world_with_column(&[Stone, Sand, Water, Water]);
        let eye = Vec3::new(4.0, 6.0, 4.0);
        let (hit, block) = block_looked_at(&world, eye, Vec3::NEG_Y).unwrap();
        assert_eq!((I64Vec3::new(4, 1, 4), Sand), (hit.block, block));
        assert_eq!(I64Vec3::Y, hit.face);
    }

    #[test]
    fn test_nothing_is_looked_at_out_of_reach() {
        let world = world_with_column(&[BlockType::Stone]);
        let eye = Vec3::new(4.0, REACH + 1.0, 4.0);
        assert!(block_looked_at(&world, eye, Vec3::NEG_Y).is_none());
        let eye = Vec3::new(4.0, REACH, 4.0);
        assert!(block_looked_at(&world, eye, Vec3::NEG_Y).is_some());
    }

    #[test]
    fn test_compacted_stone_is_looked_at_but_not_broken() {
        use BlockType::*;
        let mut world = world_with_column(&[CompactedStone, Grass, TallGrass]);
        let eye = Vec3::new(4.0, 5.0, 4.0);
        let mut broken = Vec::new();
        while let Some((hit, _)) = block_looked_at(&world, eye, Vec3::NEG_Y) {
            match world.break_block(hit.block) {
                Some(block) => broken.push(block),
                None => break,
            }
        }
        assert_eq!(vec![TallGrass, Grass], broken);
        let (hit, block) = block_looked_at(&world, eye, Vec3::NEG_Y).unwrap();
        assert_eq!((I64Vec3::new(4, 0, 4), CompactedStone), (hit.block, block));
    }

    #[test]
    fn test_outline_wraps_the_block() {
        let mesh = outline_mesh();
        let positions = mesh
            .attribute(bevy::mesh::Mesh::ATTRIBUTE_POSITION)
            .unwrap()
            .as_float3()
            .unwrap();
        // twelve bars of 24 vertices
        assert_eq!(12 * 24, positions.len());
        let reach = positions
            .iter()
            .flatten()
            .fold(0.0f32, |most, &p| most.max(p.abs()));
        assert!(reach > 0.5 && reach < 0.55, "{reach}");
    }
}
