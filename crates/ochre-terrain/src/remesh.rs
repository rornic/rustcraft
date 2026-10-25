//! Meshing chunks again once their blocks change, see `World::set_block_at`. A group is drawn
//! as one mesh, see `group::ChunkGroup`, so the whole group around a changed block is meshed
//! again and its mesh swapped for the new one where it stands, without the fade a group loading
//! in gets.

use std::sync::Arc;

use bevy::{
    asset::Assets,
    camera::visibility::Visibility,
    ecs::{
        component::Component,
        entity::Entity,
        hierarchy::Children,
        query::{Has, Or, With},
        system::{Commands, Query, ResMut},
    },
    math::I64Vec3,
    platform::collections::HashSet,
    prelude::Mesh3d,
    render::mesh::Mesh,
    tasks::{futures, AsyncComputeTaskPool, Task},
    transform::components::Transform,
};

use crate::chunk::{ChunkCoordinate, ChunkData, CHUNK_SIZE};
use crate::chunk_loader::{ChunkLoader, Unsettled};
use crate::generate::generator::{generate_chunk_mesh, ChunkMeshes, LIGHT_MARGIN};
use crate::generate::region_store::RegionStore;
use crate::group::{
    draw_group, group_aabb, merge_chunk_meshes, pack_group, BuildChunkGroup, BuiltGroup,
    ChunkGroup, GroupedChunk, MeshedChunk,
};
use crate::lod::LodKey;
use crate::world::World;

/// A group being meshed again, to be drawn in place of its mesh.
#[derive(Component)]
pub(crate) struct RemeshGroup {
    task: Task<RemeshedGroup>,
}

pub(crate) struct RemeshedGroup {
    meshes: ChunkMeshes,
    // each of its chunks, and whether it has anything to draw
    drawn: Vec<(ChunkCoordinate, bool)>,
}

/// What meshing a chunk reads: the chunk, and its 26 neighbours. None where there's nothing to
/// mesh, it being empty or buried.
type ChunkInputs = Option<(Arc<ChunkData>, Vec<Option<Arc<ChunkData>>>)>;

/// The chunks whose meshes read the block at `block`, as faces and their corners' shade, or as
/// the light reaching them, which `neighbourhood_light` takes from a box reaching `LIGHT_MARGIN`
/// out across, a block down, and up to the top of the chunk above.
pub(crate) fn chunks_reading(block: I64Vec3) -> impl Iterator<Item = ChunkCoordinate> {
    let size = CHUNK_SIZE as i64;
    let margin = LIGHT_MARGIN as i64;
    // the box's extent from a chunk's lowest corner, from its first block to past its last
    let (low, high) = (
        I64Vec3::new(-margin, -1, -margin),
        I64Vec3::new(size + margin, 2 * size, size + margin),
    );
    // the chunks c with `low <= block - c * size < high`
    let first = (block - high).div_euclid(I64Vec3::splat(size)) + 1;
    let last = (block - low).div_euclid(I64Vec3::splat(size));
    (first.x..=last.x).flat_map(move |x| {
        (first.y..=last.y).flat_map(move |y| {
            (first.z..=last.z).map(move |z| ChunkCoordinate(I64Vec3::new(x, y, z)))
        })
    })
}

/// The groups holding a chunk that reads the block at `block`.
pub(crate) fn groups_reading(block: I64Vec3) -> HashSet<LodKey> {
    chunks_reading(block)
        .map(|chunk| LodKey::containing(chunk, 1))
        .collect()
}

/// Meshes the group at `key` again, from each of its chunks' `ChunkInputs`, into one mesh.
pub(crate) fn mesh_group(
    key: LodKey,
    chunks: Vec<(ChunkCoordinate, ChunkInputs)>,
    regions: &RegionStore,
    solid_leaves: bool,
) -> RemeshedGroup {
    let origin = key.min_chunk();
    let mut parts = Vec::new();
    let mut drawn = Vec::new();
    for (coord, inputs) in chunks {
        let meshes = inputs
            .map(|(data, adjacent)| {
                generate_chunk_mesh(data, adjacent, coord, regions, solid_leaves)
            })
            .filter(ChunkMeshes::has_geometry);
        drawn.push((coord, meshes.is_some()));
        if let Some(meshes) = meshes {
            let offset = (coord.0 - origin).as_vec3() * CHUNK_SIZE as f32;
            parts.push((offset, meshes));
        }
    }
    RemeshedGroup {
        meshes: merge_chunk_meshes(parts),
        drawn,
    }
}

// What becomes of a group waiting to be meshed again.
enum Remesh {
    // its chunks are still loading, or it's still being built from what they held: what they
    // mesh to may be from before the change
    Wait,
    // some of it isn't loaded, so there's no group to draw: it meshes as it loads
    Drop,
    Ready(Vec<(ChunkCoordinate, ChunkInputs)>),
}

fn remesh_inputs(
    key: LodKey,
    world: &World,
    loader: &ChunkLoader,
    unsettled: &Query<(), Or<(Unsettled, With<MeshedChunk>)>>,
    building: &Query<(), With<BuildChunkGroup>>,
) -> Remesh {
    let mut entities = Vec::new();
    for chunk in key.chunks() {
        match loader.chunk_entity(chunk) {
            Some(entity) => entities.push(entity),
            None => return Remesh::Drop,
        }
    }
    let building = loader
        .group_entity(key)
        .is_some_and(|group| building.contains(group));
    if loader.waiting.contains(&key) || building || entities.iter().any(|&e| unsettled.contains(e))
    {
        return Remesh::Wait;
    }
    let mut chunks = Vec::new();
    for chunk in key.chunks() {
        // meshing reads the whole 3x3x3 around it, as when it was loaded
        if !chunk
            .neighbors_26()
            .into_iter()
            .all(|adj| world.is_chunk_generated(adj))
        {
            return Remesh::Wait;
        }
        let Some(data) = world.get_chunk_data(chunk) else {
            return Remesh::Drop;
        };
        let inputs = (!data.empty() && !world.is_chunk_buried(chunk))
            .then(|| (data, world.adjacent_chunk_data(chunk)));
        chunks.push((chunk, inputs));
    }
    Remesh::Ready(chunks)
}

/// Starts meshing again the groups that read blocks changed since last time, and any still
/// waiting from before.
pub(crate) fn remesh_edits(
    mut commands: Commands,
    mut world: ResMut<World>,
    mut loader: ResMut<ChunkLoader>,
    unsettled: Query<(), Or<(Unsettled, With<MeshedChunk>)>>,
    building: Query<(), With<BuildChunkGroup>>,
) {
    for block in world.take_edits() {
        loader.remesh.extend(groups_reading(block));
    }
    if loader.remesh.is_empty() {
        return;
    }
    let task_pool = AsyncComputeTaskPool::get();
    let pending: Vec<LodKey> = loader.remesh.iter().copied().collect();
    for key in pending {
        let chunks = match remesh_inputs(key, &world, &loader, &unsettled, &building) {
            Remesh::Wait => continue,
            Remesh::Drop => {
                loader.remesh.remove(&key);
                continue;
            }
            Remesh::Ready(chunks) => chunks,
        };
        loader.remesh.remove(&key);
        let regions = world.region_store.clone();
        let solid_leaves = loader.solid_leaves;
        // one meshing it already from before this change is dropped, and with it its task
        let remesh = RemeshGroup {
            task: task_pool.spawn(async move {
                let mut remeshed = mesh_group(key, chunks, &regions, solid_leaves);
                remeshed.meshes = pack_group(remeshed.meshes);
                remeshed
            }),
        };
        match loader.group_entity(key) {
            Some(group) => {
                commands.entity(group).insert(remesh);
            }
            // nothing in it was drawn before, as when it was all buried until now
            None => {
                let group = commands
                    .spawn((
                        ChunkGroup { key },
                        remesh,
                        Transform::from_translation(key.min_block().as_vec3()),
                        // shown by `refresh_lod_visibility` once it's built, as a group loading is
                        Visibility::Hidden,
                    ))
                    .id();
                loader.groups.insert(key, group);
            }
        }
    }
}

/// Draws the groups meshed again in place of their old meshes.
pub(crate) fn apply_remeshes(
    mut commands: Commands,
    loader: bevy::ecs::system::Res<ChunkLoader>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut remeshing: Query<(Entity, &mut RemeshGroup, Has<BuiltGroup>)>,
) {
    for (entity, mut remesh, built) in &mut remeshing {
        let Some(remeshed) = futures::check_ready(&mut remesh.task) else {
            continue;
        };
        let mut group = commands.entity(entity);
        group
            .remove::<(RemeshGroup, Mesh3d)>()
            .despawn_related::<Children>();
        if !built {
            group.insert((BuiltGroup, group_aabb()));
        }
        draw_group(&mut commands, entity, remeshed.meshes, &loader, &mut meshes);
        for (chunk, drawn) in remeshed.drawn {
            let Some(chunk) = loader.chunk_entity(chunk) else {
                continue;
            };
            if let Ok(mut chunk) = commands.get_entity(chunk) {
                if drawn {
                    chunk.try_insert(GroupedChunk);
                } else {
                    chunk.try_remove::<GroupedChunk>();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy::{
        app::{App, TaskPoolPlugin, Update},
        asset::{AssetApp, AssetPlugin, Assets, Handle},
        ecs::{entity::Entity, schedule::IntoScheduleConfigs},
        math::{I64Vec3, U16Vec3},
        mesh::Mesh3d,
        platform::collections::HashSet,
        render::mesh::{Mesh, VertexAttributeValues},
    };

    use super::{apply_remeshes, chunks_reading, groups_reading, mesh_group, remesh_edits};
    use crate::chunk::{ChunkCoordinate, ChunkData, CHUNK_SIZE};
    use crate::chunk_loader::{Chunk, ChunkLoader};
    use crate::generate::generator::{generate_chunk_mesh, ChunkMeshes, LIGHT_MARGIN};
    use crate::group::{BuiltGroup, ChunkGroup, GroupedChunk};
    use crate::lod::{LodKey, LodLayout};
    use crate::world::World;
    use ochre_base::block::BlockType;
    use ochre_base::WORLD_HEIGHT;

    fn world() -> World {
        World::with_terrain(1, 16, ochre_world::worldmap::test_terrain(1))
    }

    // Holes scattered through stone, so light gets in among the blocks and spreads.
    fn porous(coord: ChunkCoordinate) -> ChunkData {
        let size = CHUNK_SIZE as i64;
        let mut cells = Vec::new();
        for x in 0..size {
            for z in 0..size {
                for y in 0..size {
                    let p = coord.0 * size + I64Vec3::new(x, y, z);
                    let hash = (p.x * 73_856_093) ^ (p.y * 19_349_663) ^ (p.z * 83_492_791);
                    cells.push(if hash.rem_euclid(5) < 2 {
                        BlockType::Air
                    } else {
                        BlockType::Stone
                    });
                }
            }
        }
        ChunkData::from_cells(cells)
    }

    fn mesh_of(world: &World, chunk: ChunkCoordinate) -> ChunkMeshes {
        generate_chunk_mesh(
            world.get_chunk_data(chunk).unwrap(),
            world.adjacent_chunk_data(chunk),
            chunk,
            &world.region_store,
            false,
        )
    }

    fn attribute(mesh: &Mesh, attribute: bevy::render::mesh::MeshVertexAttribute) -> Vec<f32> {
        match mesh.attribute(attribute) {
            Some(VertexAttributeValues::Float32x2(v)) => v.iter().flatten().copied().collect(),
            Some(VertexAttributeValues::Float32x3(v)) => v.iter().flatten().copied().collect(),
            Some(VertexAttributeValues::Float32x4(v)) => v.iter().flatten().copied().collect(),
            _ => Vec::new(),
        }
    }

    // Everything a chunk's opaque faces carry: where they are, their shade and their light.
    fn faces(meshes: &ChunkMeshes) -> Vec<Vec<f32>> {
        [
            Mesh::ATTRIBUTE_POSITION,
            Mesh::ATTRIBUTE_COLOR,
            Mesh::ATTRIBUTE_UV_1,
        ]
        .map(|a| attribute(&meshes.opaque, a))
        .into()
    }

    #[test]
    fn test_chunks_reading_a_block_are_those_whose_light_box_holds_it() {
        let size = CHUNK_SIZE as i64;
        let margin = LIGHT_MARGIN as i64;
        for block in [
            I64Vec3::ZERO,
            I64Vec3::new(15, 15, 15),
            I64Vec3::new(-1, 40, 7),
            I64Vec3::new(-33, -17, 100),
        ] {
            let reading: HashSet<_> = chunks_reading(block).collect();
            for x in -4..4 {
                for y in -4..4 {
                    for z in -4..4 {
                        let chunk = ChunkCoordinate(
                            block.div_euclid(I64Vec3::splat(size)) + I64Vec3::new(x, y, z),
                        );
                        let local = block - chunk.0 * size;
                        let held = (-margin..size + margin).contains(&local.x)
                            && (-1..2 * size).contains(&local.y)
                            && (-margin..size + margin).contains(&local.z);
                        assert_eq!(held, reading.contains(&chunk), "{block:?} in {chunk:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn test_a_block_on_a_corner_reads_into_the_groups_around_it() {
        // the corner of eight groups, whose light reaches a chunk into each
        assert_eq!(
            8,
            groups_reading(I64Vec3::splat(2 * CHUNK_SIZE as i64)).len()
        );
    }

    // The chunks around one, meshed before and after a block in it is broken, change only where
    // `chunks_reading` says they read it.
    #[test]
    fn test_breaking_a_block_changes_only_the_meshes_reading_it() {
        let mut world = world();
        let centre = ChunkCoordinate(I64Vec3::new(0, 4, 0));
        for x in -3..=3 {
            for y in 1..=7 {
                for z in -3..=3 {
                    let coord = ChunkCoordinate(centre.0 + I64Vec3::new(x, y - 4, z));
                    world.insert_chunk(coord, porous(coord));
                }
            }
        }
        // stone on the chunk's +x side against stone in the next, whose face it uncovers
        let block = (1..15)
            .map(|y| centre.0 * CHUNK_SIZE as i64 + I64Vec3::new(15, y, 3))
            .find(|&b| {
                world.get_block_at(b) == BlockType::Stone
                    && world.get_block_at(b + I64Vec3::X) == BlockType::Stone
            })
            .unwrap();
        let near: Vec<ChunkCoordinate> = (-2..=2)
            .flat_map(|x| {
                (-2..=2).flat_map(move |y| {
                    (-2..=2).map(move |z| ChunkCoordinate(centre.0 + I64Vec3::new(x, y, z)))
                })
            })
            .collect();
        let before: Vec<_> = near.iter().map(|&c| faces(&mesh_of(&world, c))).collect();
        assert!(world.break_block(block).is_some());
        let reading: HashSet<_> = chunks_reading(block).collect();
        let mut changed = 0;
        for (&chunk, before) in near.iter().zip(before) {
            if faces(&mesh_of(&world, chunk)) != before {
                assert!(
                    reading.contains(&chunk),
                    "{chunk:?} changed but isn't meshed again"
                );
                changed += 1;
            }
        }
        // its own chunk's faces and the one beside it, and others only by light
        assert!(changed >= 2, "only {changed} chunks changed");
    }

    #[test]
    fn test_a_group_meshed_again_shows_the_hole() {
        let mut world = world();
        let key = LodKey::containing(ChunkCoordinate(I64Vec3::ZERO), 1);
        let mut floor = ChunkData::default();
        for x in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                floor.set_block_at(U16Vec3::new(x, 0, z), BlockType::Stone);
            }
        }
        for x in -1..=2 {
            for y in -1..=2 {
                for z in -1..=2 {
                    let coord = ChunkCoordinate(I64Vec3::new(x, y, z));
                    let data = if y == 0 {
                        floor.clone()
                    } else {
                        ChunkData::default()
                    };
                    world.insert_chunk(coord, data);
                }
            }
        }
        let inputs = |world: &World| {
            key.chunks()
                .map(|c| {
                    let data = world.get_chunk_data(c).unwrap();
                    (
                        c,
                        (!data.empty()).then(|| (data, world.adjacent_chunk_data(c))),
                    )
                })
                .collect::<Vec<_>>()
        };
        let regions = world.region_store.clone();
        let vertices = |world: &World| {
            mesh_group(key, inputs(world), &regions, false)
                .meshes
                .opaque
                .count_vertices()
        };
        let whole = vertices(&world);
        assert_eq!(
            Some(BlockType::Stone),
            world.break_block(I64Vec3::new(5, 0, 5))
        );
        // the floor's top is split around the hole, and the hole has walls
        assert!(vertices(&world) > whole);
        let remeshed = mesh_group(key, inputs(&world), &regions, false);
        assert_eq!(
            vec![true, true, true, true],
            remeshed
                .drawn
                .iter()
                .filter(|(c, _)| c.0.y == 0)
                .map(|&(_, drawn)| drawn)
                .collect::<Vec<_>>()
        );
    }

    // The systems, run in an app: a break meshes its group again and draws it.
    #[test]
    fn test_breaking_a_block_draws_its_group_again() {
        let mut world = world();
        let key = LodKey::containing(ChunkCoordinate(I64Vec3::ZERO), 1);
        for x in -1..=2 {
            for y in -1..=2 {
                for z in -1..=2 {
                    let coord = ChunkCoordinate(I64Vec3::new(x, y, z));
                    world.insert_chunk(coord, porous(coord));
                }
            }
        }
        let mut app = App::new();
        app.add_plugins((TaskPoolPlugin::default(), AssetPlugin::default()))
            .init_asset::<Mesh>()
            .add_systems(Update, (remesh_edits, apply_remeshes).chain());
        let mut loader = ChunkLoader::new(
            LodLayout::new(32, 128, 2),
            WORLD_HEIGHT,
            false,
            Handle::default(),
            Handle::default(),
            Handle::default(),
        );
        let chunks: Vec<(ChunkCoordinate, Entity)> = key
            .chunks()
            .map(|coord| (coord, app.world_mut().spawn(Chunk { coord }).id()))
            .collect();
        for &(coord, entity) in &chunks {
            loader.track_chunk(coord, entity);
        }
        app.insert_resource(loader).insert_resource(world);

        let draw = |app: &mut App| {
            for _ in 0..1000 {
                app.update();
                let world = app.world_mut();
                let remeshing = world.query::<&super::RemeshGroup>().iter(world).count();
                if remeshing == 0 {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            panic!("never meshed");
        };
        let opaque_vertices = |app: &mut App| {
            let group = app
                .world()
                .resource::<ChunkLoader>()
                .group_entity(key)
                .unwrap();
            let handle = app.world().get::<Mesh3d>(group).unwrap().0.clone();
            app.world()
                .resource::<Assets<Mesh>>()
                .get(&handle)
                .unwrap()
                .count_vertices()
        };

        // two stone blocks inside the group's first chunk
        let stones: Vec<I64Vec3> = (1..15)
            .map(|y| I64Vec3::new(5, y, 5))
            .filter(|&b| app.world().resource::<World>().get_block_at(b) == BlockType::Stone)
            .take(2)
            .collect();
        let broken = app
            .world_mut()
            .resource_mut::<World>()
            .break_block(stones[0]);
        assert_eq!(Some(BlockType::Stone), broken);
        draw(&mut app);
        let group = app
            .world()
            .resource::<ChunkLoader>()
            .group_entity(key)
            .expect("a group for what was never drawn");
        let entity = app.world().entity(group);
        assert!(entity.contains::<ChunkGroup>() && entity.contains::<BuiltGroup>());
        let first = opaque_vertices(&mut app);
        assert!(first > 0);
        for (_, chunk) in chunks {
            assert!(app.world().entity(chunk).contains::<GroupedChunk>());
        }
        assert!(app.world().resource::<ChunkLoader>().remesh.is_empty());

        // a second break swaps the same group's mesh for one with the new hole in
        let broken = app
            .world_mut()
            .resource_mut::<World>()
            .break_block(stones[1]);
        assert_eq!(Some(BlockType::Stone), broken);
        draw(&mut app);
        assert_eq!(
            Some(group),
            app.world().resource::<ChunkLoader>().group_entity(key)
        );
        assert_ne!(first, opaque_vertices(&mut app));
    }

    #[test]
    fn test_groups_not_loaded_are_dropped() {
        let mut world = world();
        let coord = ChunkCoordinate(I64Vec3::ZERO);
        world.insert_chunk(coord, porous(coord));
        let mut app = App::new();
        app.add_plugins((TaskPoolPlugin::default(), AssetPlugin::default()))
            .init_asset::<Mesh>()
            .add_systems(Update, (remesh_edits, apply_remeshes).chain())
            .insert_resource(ChunkLoader::new(
                LodLayout::new(32, 128, 2),
                WORLD_HEIGHT,
                false,
                Handle::default(),
                Handle::default(),
                Handle::default(),
            ))
            .insert_resource(world);
        let block = (0..CHUNK_SIZE as i64)
            .map(|y| I64Vec3::new(1, y, 1))
            .find(|&b| app.world().resource::<World>().get_block_at(b) == BlockType::Stone)
            .unwrap();
        app.world_mut().resource_mut::<World>().break_block(block);
        app.update();
        let loader = app.world().resource::<ChunkLoader>();
        assert!(loader.remesh.is_empty());
        assert!(loader.group_entity(LodKey::containing(coord, 1)).is_none());
    }
}
