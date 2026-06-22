use bevy::{
    asset::{Assets, Handle},
    ecs::{
        component::Component,
        entity::Entity,
        query::{With, Without},
        system::{Commands, Query, Res, ResMut, Resource},
    },
    hierarchy::Parent,
    math::{Dir3, I64Vec3, Vec3},
    pbr::MeshMaterial3d,
    prelude::Mesh3d,
    render::{camera::Camera, mesh::Mesh, primitives::Aabb},
    tasks::{AsyncComputeTaskPool, Task},
    transform::components::{GlobalTransform, Transform},
    utils::{futures, HashMap},
};

use super::{
    chunk::{ChunkCoordinate, ChunkData},
    generate::generator::{generate_chunk, generate_chunk_mesh},
    material::ChunkMaterial,
};
use crate::{player::PlayerLook, world::World};

#[derive(Component)]
pub struct Chunk {
    coord: ChunkCoordinate,
}

#[derive(Component)]
pub struct DirtyChunk {}

#[derive(Component)]
pub struct GenerateChunkData {
    task: Task<ChunkData>,
}

#[derive(Component)]
pub struct GenerateChunkMesh {
    coord: ChunkCoordinate,
    task: Option<Task<Mesh>>,
}

#[derive(Resource)]
pub struct ChunkLoader {
    render_distance: u32,
    chunk_to_entity: HashMap<ChunkCoordinate, Entity>,
    spawn_queue: ChunkSpawnQueue,
    material: Handle<ChunkMaterial>,
}

// spawning a generation task is cheap (just hands work to the async pool)
const MAX_CHUNKS_SPAWNED_PER_FRAME: usize = 64;
// safety cap on how many offsets gather_chunks examines in one frame - bounds worst
// case frame time of a full rescan (triggered every chunk crossing) regardless of
// render distance, rather than relying on the hash lookup staying cheap forever
const MAX_SCAN_PER_FRAME: usize = 50_000;
// GPU buffer upload + ECS component insert on the main thread - the most expensive of
// these per item, and the most likely source of a visible stutter, so smallest budget
const MAX_MESHES_APPLIED_PER_FRAME: usize = 16;

impl ChunkLoader {
    pub fn new(render_distance: u32, material: Handle<ChunkMaterial>) -> Self {
        Self {
            render_distance,
            chunk_to_entity: HashMap::new(),
            // generate one shell of chunk data beyond render_distance so the chunks
            // actually meshed (up to render_distance) always have complete neighbour
            // data - without this, the outermost shell would never have all 6
            // neighbours generated, since one of them is always one step further out
            spawn_queue: ChunkSpawnQueue::new(render_distance + 1),
            material,
        }
    }
}

pub fn gather_chunks(
    mut commands: Commands,
    mut chunk_loader: ResMut<ChunkLoader>,
    mut world: ResMut<World>,
    camera_query: Query<(&Parent, &GlobalTransform), (With<Camera>, Without<PlayerLook>)>,
    generating_chunks_query: Query<&Chunk, With<GenerateChunkData>>,
) {
    if generating_chunks_query.iter().count() > 1024 {
        return;
    }

    let (_, camera) = camera_query.get_single().expect("could not find camera");

    let camera_pos = camera.translation();
    let camera_chunk = world.block_to_chunk_coordinate(I64Vec3::new(
        camera_pos.x as i64,
        camera_pos.y as i64,
        camera_pos.z as i64,
    ));
    let camera_forward = camera.forward();

    let ChunkLoader {
        spawn_queue,
        chunk_to_entity,
        ..
    } = &mut *chunk_loader;
    let mut next_chunks = spawn_queue.next_batch(
        camera_chunk,
        chunk_to_entity,
        MAX_CHUNKS_SPAWNED_PER_FRAME,
        MAX_SCAN_PER_FRAME,
    );
    sort_by_viewport_bias(&mut next_chunks, camera_chunk, camera_forward);

    let task_pool = AsyncComputeTaskPool::get();
    for chunk in next_chunks {
        generate_single_chunk(
            &mut commands,
            &mut world,
            chunk,
            task_pool,
            &mut chunk_loader,
        );
    }
}

fn generate_single_chunk(
    commands: &mut Commands,
    world: &mut ResMut<World>,
    coord: ChunkCoordinate,
    task_pool: &AsyncComputeTaskPool,
    chunk_loader: &mut ResMut<ChunkLoader>,
) {
    let noise_generator = world.noise_generator.clone();
    let height = world.height;
    let entity = commands
        .spawn((
            Chunk { coord },
            GenerateChunkData {
                task: task_pool
                    .spawn(async move { generate_chunk(noise_generator, coord, height) }),
            },
        ))
        .id();
    chunk_loader.chunk_to_entity.insert(coord, entity);
}

pub fn generate_chunks(
    mut commands: Commands,
    mut world: ResMut<World>,
    mut chunks_query: Query<(Entity, &mut Chunk, &mut GenerateChunkData)>,
) {
    for (entity, chunk, mut gen_chunk) in chunks_query.iter_mut() {
        if let Some(chunk_data) = futures::check_ready(&mut gen_chunk.task) {
            let data = world.insert_chunk(chunk.coord, chunk_data);
            if !data.empty() {
                commands.entity(entity).insert(DirtyChunk {});
            }
            commands.entity(entity).remove::<GenerateChunkData>();
        }
    }
}

pub fn mark_chunks(
    mut commands: Commands,
    mut world: ResMut<World>,
    chunk_loader: Res<ChunkLoader>,
    mut chunks_query: Query<
        (Entity, &mut Chunk),
        (
            With<DirtyChunk>,
            Without<GenerateChunkData>,
            Without<GenerateChunkMesh>,
        ),
    >,
) {
    chunks_query.iter_mut().for_each(|(entity, chunk)| {
        // chunks beyond render_distance are generated only to supply neighbour data
        // for the true boundary's meshing - they're never meshed themselves
        if chunk_distance(chunk.coord, chunk_loader.spawn_queue.anchor)
            > chunk_loader.render_distance as f32
        {
            return;
        }

        if chunk
            .coord
            .adjacent()
            .into_iter()
            .all(|adj| world.is_chunk_generated(adj))
        {
            commands.entity(entity).insert(GenerateChunkMesh {
                coord: chunk.coord,
                task: None,
            });
            commands.entity(entity).remove::<DirtyChunk>();
        }
    });
}

pub fn load_chunks(
    mut commands: Commands,
    mut world: ResMut<World>,
    mut chunks_query: Query<(Entity, &Chunk, &mut GenerateChunkMesh)>,
    mut meshes: ResMut<Assets<Mesh>>,
    chunk_loader: ResMut<ChunkLoader>,
) {
    let mut ready = vec![];
    let task_pool = AsyncComputeTaskPool::get();

    for (entity, chunk, mut gen_chunk_mesh) in chunks_query.iter_mut() {
        match &mut gen_chunk_mesh.task {
            Some(task) => {
                if let Some(mesh) = futures::check_ready(task) {
                    ready.push((entity, chunk, mesh));
                }
            }
            None => {
                // re-check neighbour completeness here, not just in mark_chunks: a neighbour
                // confirmed generated when GenerateChunkMesh was added can still be unloaded
                // by the time this system runs in a later frame, if the player has moved far
                // enough since. Without this check, adjacent_chunk_data below would silently
                // return None for that side, baking a permanent border into the mesh.
                let neighbours_ready = chunk
                    .coord
                    .adjacent()
                    .into_iter()
                    .all(|adj| world.is_chunk_generated(adj));
                if neighbours_ready {
                    if let Some(data) = world.get_chunk_data(gen_chunk_mesh.coord) {
                        let adjacent = world.adjacent_chunk_data(chunk.coord);
                        gen_chunk_mesh.task = Some(
                            task_pool.spawn(async move { generate_chunk_mesh(data, adjacent) }),
                        );
                    }
                }
            }
        }

        if ready.len() >= MAX_MESHES_APPLIED_PER_FRAME {
            break;
        }
    }

    for (entity, chunk, mesh) in ready {
        let (t, aabb) = chunk_components(chunk.coord);

        commands.entity(entity).insert((
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(chunk_loader.material.clone_weak()),
            t,
            aabb,
        ));
        commands.entity(entity).remove::<GenerateChunkMesh>();
    }
}

pub fn unload_chunks(
    mut commands: Commands,
    mut world: ResMut<World>,
    mut chunk_loader: ResMut<ChunkLoader>,
    chunks_query: Query<(Entity, &Chunk), (Without<GenerateChunkData>, Without<GenerateChunkMesh>)>,
) {
    for (entity, chunk) in chunks_query.iter() {
        // + 1 to match the neighbour-data padding shell generated beyond render_distance
        if chunk_distance(chunk.coord, chunk_loader.spawn_queue.anchor)
            > (chunk_loader.render_distance + 1) as f32
        {
            commands.entity(entity).despawn();
            chunk_loader.chunk_to_entity.remove(&chunk.coord);
            world.clear_chunk(chunk.coord);
        }
    }
}

fn chunk_world_pos(chunk: ChunkCoordinate) -> Vec3 {
    Vec3::new(
        (chunk.0.x * 16) as f32,
        (chunk.0.y * 16) as f32,
        (chunk.0.z * 16) as f32,
    )
}

fn chunk_world_centre(chunk: ChunkCoordinate) -> Vec3 {
    chunk_world_pos(chunk) + Vec3::splat(8.0)
}

// Euclidean, not Chebyshev - render distance is a sphere, not a cube, so "distance"
// means the same thing everywhere it's used (offset building/sorting/rewinding, the
// mesh/unload cutoffs below, and viewport-bias spawn ordering).
fn chunk_distance(chunk: ChunkCoordinate, other: ChunkCoordinate) -> f32 {
    (chunk.0 - other.0).as_vec3().length()
}

fn offset_distance(offset: I64Vec3) -> f32 {
    offset.as_vec3().length()
}

fn chunk_components(chunk: ChunkCoordinate) -> (Transform, Aabb) {
    let pos = chunk_world_pos(chunk);
    let t = Transform::from_translation(Vec3::new(pos.x, pos.y, pos.z));
    let aabb = Aabb::from_min_max(Vec3::new(0.0, 0.0, 0.0), Vec3::new(16.0, 16.0, 16.0));
    (t, aabb)
}

// pure data: depends only on render_distance (which never changes at runtime), so
// it's computed exactly once at ChunkLoader::new time. Filters the bounding cube down
// to a sphere (Euclidean distance <= render_distance), then sorts nearest-first.
fn build_offsets(render_distance: u32) -> Vec<I64Vec3> {
    let r = render_distance as i64;
    let r_f = render_distance as f32;
    let mut offsets = Vec::new();
    for x in -r..=r {
        for y in -r..=r {
            for z in -r..=r {
                let o = I64Vec3::new(x, y, z);
                if offset_distance(o) <= r_f {
                    offsets.push(o);
                }
            }
        }
    }

    offsets.sort_by(|a, b| offset_distance(*a).total_cmp(&offset_distance(*b)));
    offsets
}

/// Decides which chunks to load next via a precomputed, distance-sorted offset table
/// plus a scan cursor - no priority queue, no seen/boundary sets. "Already loaded" is
/// answered fresh from `chunk_to_entity` every call, so there is no separate tracking
/// state that can drift out of sync with it, and no permanently-stuck state is possible:
/// any time the camera moves to a new chunk, the cursor rewinds just far enough to
/// re-cover the chunks that might newly be in range (see `rewind_for_shift`), so
/// newly-exposed volume is always found without re-scanning everything already done.
#[derive(Debug)]
struct ChunkSpawnQueue {
    offsets: Vec<I64Vec3>,
    anchor: ChunkCoordinate,
    cursor: usize,
}

impl ChunkSpawnQueue {
    fn new(render_distance: u32) -> Self {
        Self {
            offsets: build_offsets(render_distance),
            anchor: ChunkCoordinate(I64Vec3::ZERO),
            cursor: 0,
        }
    }

    // by the triangle inequality (holds for Euclidean distance same as Chebyshev), a
    // chunk confirmed (loaded or not) up to distance D from the old anchor is
    // guaranteed already confirmed for the new anchor too, as long as its distance
    // from the new anchor is <= D - shift (shift = distance the anchor moved). So only
    // the outermost `shift`-wide band of what was already scanned needs re-checking -
    // not the whole list. A teleport (shift > everything scanned so far) naturally
    // rewinds all the way to 0.
    fn rewind_for_shift(&mut self, shift: f32) {
        let processed_distance = if self.cursor == 0 {
            0.0
        } else {
            offset_distance(self.offsets[self.cursor - 1])
        };
        let target_distance = (processed_distance - shift).max(0.0);
        self.cursor = self
            .offsets
            .partition_point(|o| offset_distance(*o) < target_distance);
    }

    // pure function of its inputs (no ECS/World access), directly unit-testable.
    fn next_batch(
        &mut self,
        camera_chunk: ChunkCoordinate,
        loaded: &HashMap<ChunkCoordinate, Entity>,
        max_candidates: usize,
        max_scanned: usize,
    ) -> Vec<ChunkCoordinate> {
        if camera_chunk != self.anchor {
            let shift = chunk_distance(camera_chunk, self.anchor);
            self.rewind_for_shift(shift);
            self.anchor = camera_chunk;
        }

        let mut candidates = Vec::with_capacity(max_candidates);
        let mut scanned = 0;
        while self.cursor < self.offsets.len()
            && candidates.len() < max_candidates
            && scanned < max_scanned
        {
            let coord = ChunkCoordinate(self.anchor.0 + self.offsets[self.cursor]);
            self.cursor += 1;
            scanned += 1;

            if !loaded.contains_key(&coord) {
                candidates.push(coord);
            }
        }

        candidates
    }
}

// distance is the primary key (closer chunks must always spawn first - never let
// viewport bias schedule a farther chunk ahead of a closer one); viewport bias only
// breaks ties among chunks at the same/similar distance. Recomputed fresh from the
// live camera_forward every call - nothing is cached or carried between frames, so
// there is no staleness to go wrong as the camera turns.
fn sort_by_viewport_bias(
    candidates: &mut [ChunkCoordinate],
    camera_chunk: ChunkCoordinate,
    camera_forward: Dir3,
) {
    let camera_pos = chunk_world_centre(camera_chunk);
    candidates.sort_by(|a, b| {
        let dist_a = chunk_distance(*a, camera_chunk);
        let dist_b = chunk_distance(*b, camera_chunk);
        dist_a.total_cmp(&dist_b).then_with(|| {
            let score_a = viewport_score(*a, camera_pos, camera_forward);
            let score_b = viewport_score(*b, camera_pos, camera_forward);
            score_b.total_cmp(&score_a)
        })
    });
}

fn viewport_score(chunk: ChunkCoordinate, camera_pos: Vec3, camera_forward: Dir3) -> f32 {
    let to_chunk = chunk_world_centre(chunk) - camera_pos;
    if to_chunk == Vec3::ZERO {
        return 1.0; // camera's own chunk; also avoids normalize()-of-zero NaN
    }
    camera_forward.dot(to_chunk.normalize())
}

#[cfg(test)]
mod tests {
    use bevy::{
        ecs::entity::Entity,
        math::{Dir3, I64Vec3, Vec3},
    };

    use super::{
        build_offsets, offset_distance, sort_by_viewport_bias, ChunkCoordinate, ChunkSpawnQueue,
        HashMap,
    };

    #[test]
    fn test_build_offsets_count_and_bounds() {
        let offsets = build_offsets(2);
        assert_eq!(33, offsets.len());
        for o in &offsets {
            assert!(offset_distance(*o) <= 2.0);
        }
    }

    #[test]
    fn test_build_offsets_starts_at_origin() {
        let offsets = build_offsets(2);
        assert_eq!(I64Vec3::ZERO, offsets[0]);
    }

    #[test]
    fn test_build_offsets_sorted_ascending_by_distance() {
        let offsets = build_offsets(3);
        let mut last = 0.0;
        for o in &offsets {
            let d = offset_distance(*o);
            assert!(d >= last);
            last = d;
        }
    }

    #[test]
    fn test_next_batch_rewind_is_proportional_not_full_reset() {
        let mut queue = ChunkSpawnQueue::new(10);
        let loaded = HashMap::new();
        let anchor = ChunkCoordinate(I64Vec3::ZERO);

        queue.next_batch(anchor, &loaded, 1000, 100_000);
        let cursor_before = queue.cursor;
        assert!(cursor_before > 0);

        // a normal 1-chunk step should only rewind the outermost shell, not reset to 0
        let new_anchor = ChunkCoordinate(I64Vec3::new(1, 0, 0));
        queue.next_batch(new_anchor, &loaded, 0, 0);

        assert!(queue.cursor > 0);
        assert!(queue.cursor <= cursor_before);
    }

    #[test]
    fn test_next_batch_rewinds_fully_on_large_jump() {
        let mut queue = ChunkSpawnQueue::new(10);
        let loaded = HashMap::new();
        let anchor = ChunkCoordinate(I64Vec3::ZERO);

        queue.next_batch(anchor, &loaded, 1000, 100_000);
        assert!(queue.cursor > 0);

        // a jump far larger than anything scanned so far invalidates all of it
        let far_anchor = ChunkCoordinate(I64Vec3::new(1000, 0, 0));
        queue.next_batch(far_anchor, &loaded, 0, 0);

        assert_eq!(0, queue.cursor);
    }

    #[test]
    fn test_next_batch_skips_already_loaded() {
        let mut queue = ChunkSpawnQueue::new(2);
        let anchor = ChunkCoordinate(I64Vec3::ZERO);
        let mut loaded = HashMap::new();
        loaded.insert(anchor, Entity::PLACEHOLDER);

        let batch = queue.next_batch(anchor, &loaded, 5, 1000);
        assert!(!batch.contains(&anchor));
    }

    #[test]
    fn test_next_batch_respects_max_candidates() {
        let mut queue = ChunkSpawnQueue::new(5);
        let anchor = ChunkCoordinate(I64Vec3::ZERO);
        let loaded = HashMap::new();

        let batch = queue.next_batch(anchor, &loaded, 7, 100_000);
        assert_eq!(7, batch.len());
    }

    #[test]
    fn test_next_batch_respects_max_scanned_even_if_under_candidate_cap() {
        let mut queue = ChunkSpawnQueue::new(5);
        let anchor = ChunkCoordinate(I64Vec3::ZERO);
        let mut loaded = HashMap::new();
        for o in queue.offsets.clone() {
            loaded.insert(ChunkCoordinate(anchor.0 + o), Entity::PLACEHOLDER);
        }

        let batch = queue.next_batch(anchor, &loaded, 64, 10);
        assert_eq!(0, batch.len());
        assert_eq!(10, queue.cursor);
    }

    #[test]
    fn test_next_batch_terminal_state_is_idempotent() {
        let mut queue = ChunkSpawnQueue::new(1); // origin + 6 face neighbours = 7 offsets
        let anchor = ChunkCoordinate(I64Vec3::ZERO);
        let loaded = HashMap::new();

        let batch1 = queue.next_batch(anchor, &loaded, 100, 1000);
        assert_eq!(7, batch1.len());
        assert_eq!(7, queue.cursor);

        let batch2 = queue.next_batch(anchor, &loaded, 100, 1000);
        assert_eq!(0, batch2.len());
        assert_eq!(7, queue.cursor);
    }

    #[test]
    fn test_sort_by_viewport_bias_prefers_forward_chunks() {
        let camera_chunk = ChunkCoordinate(I64Vec3::ZERO);
        let forward = Dir3::new(Vec3::new(1.0, 0.0, 0.0)).unwrap();
        let mut candidates = vec![
            ChunkCoordinate(I64Vec3::new(-3, 0, 0)),
            ChunkCoordinate(I64Vec3::new(3, 0, 0)),
        ];

        sort_by_viewport_bias(&mut candidates, camera_chunk, forward);

        assert_eq!(ChunkCoordinate(I64Vec3::new(3, 0, 0)), candidates[0]);
    }

    #[test]
    fn test_sort_by_viewport_bias_never_puts_farther_chunk_before_closer_one() {
        let camera_chunk = ChunkCoordinate(I64Vec3::ZERO);
        // a chunk directly ahead but far away, vs a chunk behind but close - distance
        // must win, even though the far chunk has a much better viewport score
        let forward = Dir3::new(Vec3::new(1.0, 0.0, 0.0)).unwrap();
        let far_ahead = ChunkCoordinate(I64Vec3::new(10, 0, 0));
        let close_behind = ChunkCoordinate(I64Vec3::new(-1, 0, 0));
        let mut candidates = vec![far_ahead, close_behind];

        sort_by_viewport_bias(&mut candidates, camera_chunk, forward);

        assert_eq!(close_behind, candidates[0]);
    }
}
