use std::time::{Duration, Instant};

use bevy::{
    asset::{Assets, Handle},
    ecs::{
        change_detection::DetectChangesMut,
        component::Component,
        entity::Entity,
        query::{Added, Or, With},
        system::{Commands, Local, Query, Res, ResMut, Resource},
    },
    hierarchy::{BuildChildren, ChildBuild},
    math::{I64Vec3, Vec3},
    pbr::{DistanceFog, FogFalloff, MeshMaterial3d},
    prelude::Mesh3d,
    render::{camera::Camera, mesh::Mesh, primitives::Aabb, view::Visibility},
    tasks::{AsyncComputeTaskPool, Task},
    time::Time,
    transform::components::Transform,
    utils::{futures, HashMap, HashSet},
};

use super::{
    chunk::{ChunkCoordinate, CHUNK_SIZE},
    chunk_loader::{Chunk, ChunkLoader, DirtyChunk, GenerateChunkData, GenerateChunkMesh},
    fade::{fade_out_and_despawn, set_shown, Fade},
    generate::generator::{generate_lod_mesh, ChunkMeshes},
    material::{ChunkMaterial, WaterMaterial},
};
use crate::world::World;

/// Chunks within this many chunks of the camera are drawn at full detail.
pub const LOD0_DISTANCE: f32 = 32.0;
// A node splits into its 8 children when the camera is closer than this many of its
// own widths, so each LOD reaches twice as far as the one before it.
const SPLIT_FACTOR: f32 = LOD0_DISTANCE / 2.0;

// LOD tasks queued per generation thread. Kept small so full-detail chunk tasks, which
// share the task pool, aren't left queued behind a long backlog of distant nodes.
const LOD_TASKS_PER_THREAD: usize = 4;
const LOD_APPLY_TIME_BUDGET: Duration = Duration::from_millis(2);

/// An octree node at `level`, covering `2^level` chunks per side. Level 0 is a single
/// chunk; `coord` is in units of the node's own width.
#[derive(Debug, Copy, Clone, Hash, PartialEq, Eq)]
pub struct LodKey {
    pub level: u8,
    pub coord: I64Vec3,
}

impl LodKey {
    pub fn containing(chunk: ChunkCoordinate, level: u8) -> Self {
        Self {
            level,
            coord: chunk.0.div_euclid(I64Vec3::splat(1 << level)),
        }
    }

    /// Width in chunks.
    pub fn width(self) -> i64 {
        1 << self.level
    }

    /// Every node is `CHUNK_SIZE` cells across, so each cell is this many blocks wide.
    pub fn cell_size(self) -> i64 {
        self.width()
    }

    pub fn min_chunk(self) -> I64Vec3 {
        self.coord * self.width()
    }

    pub fn min_block(self) -> I64Vec3 {
        self.min_chunk() * CHUNK_SIZE as i64
    }

    pub fn parent(self) -> Self {
        Self {
            level: self.level + 1,
            coord: self.coord.div_euclid(I64Vec3::splat(2)),
        }
    }

    pub fn children(self) -> [Self; 8] {
        std::array::from_fn(|i| Self {
            level: self.level - 1,
            coord: self.coord * 2
                + I64Vec3::new((i & 1) as i64, ((i >> 1) & 1) as i64, ((i >> 2) & 1) as i64),
        })
    }

    pub fn chunks(self) -> impl Iterator<Item = ChunkCoordinate> {
        let min = self.min_chunk();
        let width = self.width();
        (0..width).flat_map(move |x| {
            (0..width).flat_map(move |y| {
                (0..width).map(move |z| ChunkCoordinate(min + I64Vec3::new(x, y, z)))
            })
        })
    }

    /// Distance in chunks from the centre of `anchor` to the nearest point of this node.
    pub fn distance(self, anchor: ChunkCoordinate) -> f32 {
        let point = anchor.0.as_vec3() + Vec3::splat(0.5);
        let min = self.min_chunk().as_vec3();
        let max = min + Vec3::splat(self.width() as f32);
        (min - point).max(point - max).max(Vec3::ZERO).length()
    }

    pub fn splits(self, anchor: ChunkCoordinate) -> bool {
        self.level > 0 && self.distance(anchor) < SPLIT_FACTOR * self.width() as f32
    }
}

/// How far, in chunks, terrain is drawn with LOD levels up to `max_level`.
pub fn lod_render_distance(max_level: u8) -> f32 {
    LOD0_DISTANCE * (1u32 << max_level) as f32
}

/// The fewest LOD levels needed to draw terrain out to `render_distance` chunks.
pub fn max_level_for(render_distance: u32) -> u8 {
    let mut level = 0;
    while lod_render_distance(level) < render_distance as f32 {
        level += 1;
    }
    level
}

/// Whether a chunk is close enough to be drawn at full detail.
pub fn lod0_wanted(chunk: ChunkCoordinate, anchor: ChunkCoordinate) -> bool {
    LodKey::containing(chunk, 1).splits(anchor)
}

/// The chunk rows, as `min..max` chunk y, that LOD nodes are generated for. Terrain
/// never reaches outside them, so nodes above or below would always be empty.
#[derive(Debug, Copy, Clone)]
pub struct LodBand {
    pub min_y: i64,
    pub max_y: i64,
}

impl LodBand {
    fn intersects(self, key: LodKey) -> bool {
        let bottom = key.min_chunk().y;
        bottom < self.max_y && bottom + key.width() > self.min_y
    }
}

/// The LOD nodes to draw around `anchor`, leaving out the area drawn at full detail.
pub fn select_lods(anchor: ChunkCoordinate, max_level: u8, band: LodBand) -> HashSet<LodKey> {
    let mut selected = HashSet::new();
    if max_level == 0 {
        return selected;
    }

    let width = 1i64 << max_level;
    let range = lod_render_distance(max_level);
    let reach = (range / width as f32).ceil() as i64 + 1;
    let centre = anchor.0.div_euclid(I64Vec3::splat(width));
    for x in -reach..=reach {
        for z in -reach..=reach {
            for y in band.min_y.div_euclid(width)..=(band.max_y - 1).div_euclid(width) {
                let key = LodKey {
                    level: max_level,
                    coord: I64Vec3::new(centre.x + x, y, centre.z + z),
                };
                if key.distance(anchor) < range {
                    select_within(key, anchor, band, &mut selected);
                }
            }
        }
    }
    selected
}

fn select_within(
    key: LodKey,
    anchor: ChunkCoordinate,
    band: LodBand,
    selected: &mut HashSet<LodKey>,
) {
    if !band.intersects(key) {
        return;
    }
    if !key.splits(anchor) {
        selected.insert(key);
    } else if key.level > 1 {
        for child in key.children() {
            select_within(child, anchor, band, selected);
        }
    }
}

/// What the retirement and visibility rules need to know about the current state.
pub struct LodState<'a> {
    pub anchor: ChunkCoordinate,
    pub max_level: u8,
    pub band: LodBand,
    pub selected: &'a HashSet<LodKey>,
    pub node_exists: &'a dyn Fn(LodKey) -> bool,
    pub node_ready: &'a dyn Fn(LodKey) -> bool,
    pub chunk_ready: &'a dyn Fn(ChunkCoordinate) -> bool,
}

impl LodState<'_> {
    fn selected_ancestor(&self, mut key: LodKey) -> Option<LodKey> {
        while key.level < self.max_level {
            key = key.parent();
            if self.selected.contains(&key) {
                return Some(key);
            }
        }
        None
    }

    fn has_existing_ancestor(&self, mut key: LodKey) -> bool {
        while key.level < self.max_level {
            key = key.parent();
            if (self.node_exists)(key) {
                return true;
            }
        }
        false
    }

    /// Whether a node that's no longer selected can go without leaving a hole: the
    /// node now covering its area is ready, or every node and chunk inside it is.
    pub fn can_retire(&self, key: LodKey) -> bool {
        match self.selected_ancestor(key) {
            Some(ancestor) => (self.node_ready)(ancestor),
            None => self.covered(key),
        }
    }

    fn covered(&self, key: LodKey) -> bool {
        if self.selected.contains(&key) {
            return (self.node_ready)(key);
        }
        if !key.splits(self.anchor) {
            return true;
        }
        if key.level == 1 {
            return key.chunks().all(|chunk| (self.chunk_ready)(chunk));
        }
        key.children()
            .into_iter()
            .all(|child| !self.band.intersects(child) || self.covered(child))
    }

    /// Selected nodes stay hidden while a coarser node they're replacing is still up,
    /// so the swap happens all at once. Nodes on their way out stay visible.
    pub fn node_visible(&self, key: LodKey) -> bool {
        !self.selected.contains(&key) || !self.has_existing_ancestor(key)
    }

    /// Full-detail chunks follow the same rule as nodes. A chunk that's no longer
    /// wanted stays loaded as neighbour data, hidden once the node covering it is ready.
    pub fn chunk_visible(&self, chunk: ChunkCoordinate) -> bool {
        let key = LodKey::containing(chunk, 0);
        if lod0_wanted(chunk, self.anchor) {
            !self.has_existing_ancestor(key)
        } else {
            !self
                .selected_ancestor(key)
                .is_some_and(|ancestor| (self.node_ready)(ancestor))
        }
    }

    /// How far out, in chunks, terrain is drawn without gaps: the distance to the
    /// nearest selected node that isn't ready and has nothing drawn in its place yet, or
    /// the full render distance once there are none. A node still covered by the
    /// coarser or finer terrain it's replacing doesn't count, so moving around doesn't
    /// pull this in - only areas that haven't loaded at all do.
    pub fn loaded_distance(&self) -> f32 {
        self.selected
            .iter()
            .filter(|&&key| !(self.node_ready)(key) && !self.covered_while_loading(key))
            .map(|key| key.distance(self.anchor))
            .fold(lod_render_distance(self.max_level), f32::min)
    }

    fn covered_while_loading(&self, key: LodKey) -> bool {
        self.has_existing_ancestor(key)
            || if key.level == 1 {
                key.chunks().any(|chunk| (self.chunk_ready)(chunk))
            } else {
                key.children()
                    .into_iter()
                    .any(|child| (self.node_exists)(child))
            }
    }
}

#[derive(Component)]
pub struct LodNode {
    key: LodKey,
}

#[derive(Component)]
pub struct GenerateLodMesh {
    task: Task<Option<ChunkMeshes>>,
}

#[derive(Resource)]
pub struct LodLoader {
    max_level: u8,
    band: LodBand,
    anchor: Option<ChunkCoordinate>,
    selected: HashSet<LodKey>,
    nodes: HashMap<LodKey, Entity>,
    // farthest first, so popping yields the nearest
    to_spawn: Vec<LodKey>,
    retiring: Vec<LodKey>,
    // areas whose nodes and chunks need their visibility rechecked
    dirty_areas: Vec<LodKey>,
    refresh_all: bool,
    material: Handle<ChunkMaterial>,
    water_material: Handle<WaterMaterial>,
}

impl LodLoader {
    pub fn new(
        max_level: u8,
        world_height: u64,
        material: Handle<ChunkMaterial>,
        water_material: Handle<WaterMaterial>,
    ) -> Self {
        Self {
            max_level,
            band: LodBand {
                min_y: 0,
                max_y: (world_height as i64 + CHUNK_SIZE as i64 - 1) / CHUNK_SIZE as i64,
            },
            anchor: None,
            selected: HashSet::new(),
            nodes: HashMap::new(),
            to_spawn: Vec::new(),
            retiring: Vec::new(),
            dirty_areas: Vec::new(),
            refresh_all: false,
            material,
            water_material,
        }
    }
}

type PendingChunk = Or<(
    With<GenerateChunkData>,
    With<DirtyChunk>,
    With<GenerateChunkMesh>,
)>;

pub fn update_lod_selection(mut lods: ResMut<LodLoader>, chunk_loader: Res<ChunkLoader>) {
    let anchor = chunk_loader.anchor();
    if lods.anchor == Some(anchor) {
        return;
    }

    let selected = select_lods(anchor, lods.max_level, lods.band);
    let mut to_spawn: Vec<LodKey> = selected
        .iter()
        .filter(|&key| !lods.nodes.contains_key(key))
        .copied()
        .collect();
    to_spawn.sort_by(|a, b| b.distance(anchor).total_cmp(&a.distance(anchor)));
    let retiring = lods
        .nodes
        .keys()
        .filter(|&key| !selected.contains(key))
        .copied()
        .collect();

    lods.anchor = Some(anchor);
    lods.selected = selected;
    lods.to_spawn = to_spawn;
    lods.retiring = retiring;
    lods.refresh_all = true;
}

pub fn spawn_lods(
    mut commands: Commands,
    mut lods: ResMut<LodLoader>,
    world: Res<World>,
    pending: Query<(), With<GenerateLodMesh>>,
) {
    let task_pool = AsyncComputeTaskPool::get();
    let max_in_flight = LOD_TASKS_PER_THREAD * task_pool.thread_num();
    let mut in_flight = pending.iter().count();

    while in_flight < max_in_flight {
        let Some(key) = lods.to_spawn.pop() else {
            break;
        };
        if !lods.selected.contains(&key) || lods.nodes.contains_key(&key) {
            continue;
        }

        let regions = world.region_store.clone();
        let entity = commands
            .spawn((
                LodNode { key },
                GenerateLodMesh {
                    task: task_pool.spawn(async move { generate_lod_mesh(regions, key) }),
                },
                lod_transform(key),
                Visibility::Hidden,
            ))
            .id();
        lods.nodes.insert(key, entity);
        in_flight += 1;
    }
}

// Mesh cells are centred on whole coordinates, like full-detail blocks, so the node is
// shifted to line its cells up with the blocks they cover.
fn lod_transform(key: LodKey) -> Transform {
    let cell = key.cell_size() as f32;
    Transform::from_translation(key.min_block().as_vec3() + Vec3::splat(0.5 * cell - 0.5))
        .with_scale(Vec3::splat(cell))
}

fn lod_aabb() -> Aabb {
    let size = CHUNK_SIZE as f32;
    Aabb::from_min_max(Vec3::splat(-0.5), Vec3::splat(size - 0.5))
}

pub fn load_lods(
    mut commands: Commands,
    mut lods: ResMut<LodLoader>,
    mut nodes_query: Query<(Entity, &LodNode, &mut GenerateLodMesh)>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let start = Instant::now();

    for (entity, node, mut generate) in nodes_query.iter_mut() {
        let Some(result) = futures::check_ready(&mut generate.task) else {
            continue;
        };

        if let Some(chunk_meshes) = result {
            commands.entity(entity).insert((
                Mesh3d(meshes.add(chunk_meshes.opaque)),
                MeshMaterial3d(lods.material.clone_weak()),
                lod_aabb(),
            ));
            if let Some(water_mesh) = chunk_meshes.water {
                let water_material = lods.water_material.clone_weak();
                commands.entity(entity).with_children(|parent| {
                    parent.spawn((
                        Mesh3d(meshes.add(water_mesh)),
                        MeshMaterial3d(water_material),
                        Transform::default(),
                        lod_aabb(),
                    ));
                });
            }
        }
        commands.entity(entity).remove::<GenerateLodMesh>();
        lods.dirty_areas.push(node.key);

        if start.elapsed() >= LOD_APPLY_TIME_BUDGET {
            break;
        }
    }
}

pub fn retire_lods(
    mut commands: Commands,
    mut lods: ResMut<LodLoader>,
    time: Res<Time>,
    chunk_loader: Res<ChunkLoader>,
    pending_lods: Query<(), With<GenerateLodMesh>>,
    pending_chunks: Query<(), PendingChunk>,
    visibility: Query<&Visibility>,
) {
    let Some(anchor) = lods.anchor else {
        return;
    };

    let mut retired = Vec::new();
    {
        let nodes = &lods.nodes;
        let node_exists = |key: LodKey| nodes.contains_key(&key);
        let node_ready = |key: LodKey| nodes.get(&key).is_some_and(|e| !pending_lods.contains(*e));
        let chunk_ready = |chunk: ChunkCoordinate| {
            chunk_loader
                .chunk_entity(chunk)
                .is_some_and(|e| !pending_chunks.contains(e))
        };
        let state = LodState {
            anchor,
            max_level: lods.max_level,
            band: lods.band,
            selected: &lods.selected,
            node_exists: &node_exists,
            node_ready: &node_ready,
            chunk_ready: &chunk_ready,
        };
        for &key in &lods.retiring {
            if !lods.selected.contains(&key) && state.can_retire(key) {
                retired.push(key);
            }
        }
    }

    if retired.is_empty() {
        return;
    }
    let lods = &mut *lods;
    lods.retiring
        .retain(|key| !retired.contains(key) && !lods.selected.contains(key));
    // Leaving `nodes` straight away lets whatever replaces a node start fading in this
    // frame, while the node itself fades out.
    for key in retired {
        if let Some(entity) = lods.nodes.remove(&key) {
            let visible = visibility
                .get(entity)
                .is_ok_and(|v| *v != Visibility::Hidden);
            fade_out_and_despawn(&mut commands, entity, visible, time.elapsed_secs());
        }
        lods.dirty_areas.push(key);
    }
}

#[allow(clippy::too_many_arguments)]
pub fn refresh_lod_visibility(
    mut commands: Commands,
    mut lods: ResMut<LodLoader>,
    time: Res<Time>,
    chunk_loader: Res<ChunkLoader>,
    pending_lods: Query<(), With<GenerateLodMesh>>,
    all_chunks: Query<(Entity, &Chunk), With<Mesh3d>>,
    new_chunks: Query<(Entity, &Chunk), Added<Mesh3d>>,
    all_nodes: Query<(Entity, &LodNode)>,
    mut visibility: Query<(&mut Visibility, Option<&Fade>)>,
) {
    let Some(anchor) = lods.anchor else {
        return;
    };

    let now = time.elapsed_secs();
    let lods = &mut *lods;
    let nodes = &lods.nodes;
    let node_exists = |key: LodKey| nodes.contains_key(&key);
    let node_ready = |key: LodKey| nodes.get(&key).is_some_and(|e| !pending_lods.contains(*e));
    let state = LodState {
        anchor,
        max_level: lods.max_level,
        band: lods.band,
        selected: &lods.selected,
        node_exists: &node_exists,
        node_ready: &node_ready,
        chunk_ready: &|_| false,
    };
    let mut set = |entity: Entity, visible: bool| {
        if let Ok((mut v, fade)) = visibility.get_mut(entity) {
            set_shown(&mut commands, entity, &mut v, fade, visible, now);
        }
    };
    // a node fades in once its mesh is ready, not when it's first spawned
    let node_shown = |key: LodKey| state.node_visible(key) && node_ready(key);

    if lods.refresh_all {
        for (entity, chunk) in all_chunks.iter() {
            set(entity, state.chunk_visible(chunk.coord));
        }
        for (entity, node) in all_nodes.iter() {
            set(entity, node_shown(node.key));
        }
    } else {
        for &area in &lods.dirty_areas {
            for chunk in area.chunks() {
                if let Some(entity) = chunk_loader.chunk_entity(chunk) {
                    set(entity, state.chunk_visible(chunk));
                }
            }
            let mut level_keys = vec![area];
            loop {
                for &key in &level_keys {
                    if let Some(&entity) = nodes.get(&key) {
                        set(entity, node_shown(key));
                    }
                }
                if level_keys[0].level <= 1 {
                    break;
                }
                level_keys = level_keys.iter().flat_map(|key| key.children()).collect();
            }
        }
    }

    // newly meshed chunks just appear, hidden straight away if a node still covers them
    for (entity, chunk) in new_chunks.iter() {
        if let Ok((mut v, _)) = visibility.get_mut(entity) {
            v.set_if_neq(if state.chunk_visible(chunk.coord) {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            });
        }
    }

    lods.refresh_all = false;
    lods.dirty_areas.clear();
}

// Fog is 95% opaque at this fraction of the distance terrain has loaded to, so the
// loaded edge is all but hidden.
pub const FOG_VISIBILITY: f32 = 0.85;
const FOG_UPDATE_INTERVAL: f32 = 0.25;
// How quickly fog eases towards its target, in seconds: in quickly when an unloaded gap
// appears closer, out gently as terrain loads.
const FOG_PULL_IN_SECONDS: f32 = 0.3;
const FOG_EASE_OUT_SECONDS: f32 = 2.0;

/// Keeps fog just short of how far terrain has actually loaded, so distant terrain
/// always fades into the horizon instead of ending at a hard edge, however far render
/// distance reaches or however much of it has loaded. Runs before the underwater
/// effects, which replace the fog while the camera is in water.
#[allow(clippy::too_many_arguments)]
pub fn update_horizon_fog(
    lods: Res<LodLoader>,
    chunk_loader: Res<ChunkLoader>,
    time: Res<Time>,
    pending_lods: Query<(), With<GenerateLodMesh>>,
    pending_chunks: Query<(), PendingChunk>,
    mut fog: Query<&mut DistanceFog, With<Camera>>,
    mut target: Local<Option<f32>>,
    mut visibility: Local<Option<f32>>,
    mut since_update: Local<f32>,
) {
    let Some(anchor) = lods.anchor else {
        return;
    };

    *since_update += time.delta_secs();
    if target.is_none() || *since_update >= FOG_UPDATE_INTERVAL {
        *since_update = 0.0;
        let nodes = &lods.nodes;
        let node_exists = |key: LodKey| nodes.contains_key(&key);
        let node_ready = |key: LodKey| nodes.get(&key).is_some_and(|e| !pending_lods.contains(*e));
        let chunk_ready = |chunk: ChunkCoordinate| {
            chunk_loader
                .chunk_entity(chunk)
                .is_some_and(|e| !pending_chunks.contains(e))
        };
        let state = LodState {
            anchor,
            max_level: lods.max_level,
            band: lods.band,
            selected: &lods.selected,
            node_exists: &node_exists,
            node_ready: &node_ready,
            chunk_ready: &chunk_ready,
        };
        *target = Some(state.loaded_distance() * CHUNK_SIZE as f32 * FOG_VISIBILITY);
    }

    let target = target.unwrap();
    let current = visibility.unwrap_or(target);
    let seconds = if target < current {
        FOG_PULL_IN_SECONDS
    } else {
        FOG_EASE_OUT_SECONDS
    };
    let eased = current + (target - current) * (1.0 - (-time.delta_secs() / seconds).exp());
    *visibility = Some(eased);

    if let Ok(mut fog) = fog.get_single_mut() {
        fog.falloff = FogFalloff::from_visibility_squared(eased);
    }
}

#[cfg(test)]
mod tests {
    use bevy::{
        math::I64Vec3,
        utils::{HashMap, HashSet},
    };

    use super::{
        lod0_wanted, lod_render_distance, max_level_for, select_lods, LodBand, LodKey, LodState,
    };
    use crate::chunks::chunk::ChunkCoordinate;

    const BAND: LodBand = LodBand {
        min_y: 0,
        max_y: 16,
    };

    fn chunk(x: i64, y: i64, z: i64) -> ChunkCoordinate {
        ChunkCoordinate(I64Vec3::new(x, y, z))
    }

    #[test]
    fn test_containing_handles_negative_chunks() {
        assert_eq!(
            I64Vec3::new(-1, 0, -1),
            LodKey::containing(chunk(-1, 3, -4), 2).coord
        );
        assert_eq!(
            I64Vec3::new(-2, 0, 0),
            LodKey::containing(chunk(-5, 3, 3), 2).coord
        );
    }

    #[test]
    fn test_parent_and_children_round_trip() {
        let key = LodKey {
            level: 2,
            coord: I64Vec3::new(-3, 1, 4),
        };
        for child in key.children() {
            assert_eq!(key, child.parent());
        }
        let unique: HashSet<_> = key.children().into_iter().collect();
        assert_eq!(8, unique.len());
    }

    #[test]
    fn test_chunks_covers_node() {
        let key = LodKey {
            level: 2,
            coord: I64Vec3::new(-1, 0, 2),
        };
        let chunks: Vec<_> = key.chunks().collect();
        assert_eq!(64, chunks.len());
        assert!(chunks.iter().all(|&c| LodKey::containing(c, 2) == key));
    }

    #[test]
    fn test_distance_is_zero_inside_node() {
        let key = LodKey {
            level: 3,
            coord: I64Vec3::ZERO,
        };
        assert_eq!(0.0, key.distance(chunk(4, 4, 4)));
        assert_eq!(2.5, key.distance(chunk(10, 4, 4)));
    }

    #[test]
    fn test_max_level_for() {
        assert_eq!(0, max_level_for(32));
        assert_eq!(1, max_level_for(33));
        assert_eq!(2, max_level_for(128));
        assert_eq!(128.0, lod_render_distance(2));
    }

    #[test]
    fn test_lod0_wanted_reaches_lod0_distance() {
        let anchor = chunk(0, 5, 0);
        assert!(lod0_wanted(chunk(0, 5, 0), anchor));
        assert!(lod0_wanted(chunk(31, 5, 0), anchor));
        assert!(!lod0_wanted(chunk(36, 5, 0), anchor));
    }

    // Every chunk in the band within range is drawn exactly once: at full detail, or
    // by exactly one selected node.
    fn assert_exact_cover(anchor: ChunkCoordinate, max_level: u8) {
        let selected = select_lods(anchor, max_level, BAND);
        let range = lod_render_distance(max_level) as i64;
        for x in -range..range {
            for z in -range..range {
                for y in BAND.min_y..BAND.max_y {
                    let c = chunk(anchor.0.x + x, y, anchor.0.z + z);
                    let covering = (1..=max_level)
                        .filter(|&level| selected.contains(&LodKey::containing(c, level)))
                        .count()
                        + lod0_wanted(c, anchor) as usize;
                    assert!(covering <= 1, "chunk {:?} drawn {covering} times", c.0);
                    if LodKey::containing(c, 0).distance(anchor) < lod_render_distance(max_level) {
                        assert_eq!(1, covering, "chunk {:?} not drawn", c.0);
                    }
                }
            }
        }
    }

    #[test]
    fn test_selection_covers_each_chunk_once() {
        assert_exact_cover(chunk(0, 3, 0), 2);
        assert_exact_cover(chunk(-7, 9, 13), 2);
        assert_exact_cover(chunk(5, -4, -3), 3);
    }

    #[test]
    fn test_selection_neighbours_differ_by_at_most_one_level() {
        let anchor = chunk(3, 4, -2);
        let selected = select_lods(anchor, 3, BAND);
        let level_at = |c: ChunkCoordinate| -> u8 {
            (1..=3)
                .find(|&level| selected.contains(&LodKey::containing(c, level)))
                .unwrap_or(0)
        };
        for x in -100..100 {
            for z in -100..100 {
                let c = chunk(anchor.0.x + x, 4, anchor.0.z + z);
                let n = chunk(c.0.x + 1, 4, c.0.z);
                if LodKey::containing(n, 0).distance(anchor) < 110.0 {
                    assert!(level_at(c).abs_diff(level_at(n)) <= 1);
                }
            }
        }
    }

    // Runs `check` against a state where `existing` nodes exist, nodes in `ready` are
    // ready and every chunk's readiness is `chunks_ready`.
    fn with_state<R>(
        anchor: ChunkCoordinate,
        selected: &HashSet<LodKey>,
        existing: &dyn Fn(LodKey) -> bool,
        ready: &HashMap<LodKey, bool>,
        chunks_ready: bool,
        check: impl FnOnce(&LodState) -> R,
    ) -> R {
        let node_ready = |key| ready.get(&key).copied().unwrap_or(false);
        let chunk_ready = |_| chunks_ready;
        check(&LodState {
            anchor,
            max_level: 3,
            band: BAND,
            selected,
            node_exists: existing,
            node_ready: &node_ready,
            chunk_ready: &chunk_ready,
        })
    }

    #[test]
    fn test_merged_node_retires_once_parent_ready() {
        // far enough that the level-3 node is drawn whole
        let far = chunk(200, 4, 0);
        let selected = select_lods(far, 3, BAND);
        let child = LodKey {
            level: 1,
            coord: I64Vec3::new(0, 2, 0),
        };
        let parent = LodKey::containing(ChunkCoordinate(child.min_chunk()), 3);
        assert!(selected.contains(&parent));

        let none = |_| false;
        let mut ready = HashMap::new();
        assert!(!with_state(far, &selected, &none, &ready, true, |s| s.can_retire(child)));
        ready.insert(parent, true);
        assert!(with_state(far, &selected, &none, &ready, true, |s| s.can_retire(child)));
    }

    #[test]
    fn test_split_node_retires_once_children_ready() {
        let anchor = chunk(0, 4, 0);
        let selected = select_lods(anchor, 3, BAND);
        // close enough to split, far enough that part of it is drawn by level-1 nodes
        let split = LodKey {
            level: 3,
            coord: I64Vec3::new(4, 0, 0),
        };
        assert!(!selected.contains(&split));
        assert!(selected
            .iter()
            .any(|k| k.level == 1 && k.parent().parent() == split));

        let none = |_| false;
        let not_ready = HashMap::new();
        assert!(!with_state(
            anchor,
            &selected,
            &none,
            &not_ready,
            true,
            |s| s.can_retire(split)
        ));
        let all_ready: HashMap<_, _> = selected.iter().map(|&k| (k, true)).collect();
        assert!(!with_state(
            anchor,
            &selected,
            &none,
            &all_ready,
            false,
            |s| s.can_retire(split)
        ));
        assert!(with_state(
            anchor,
            &selected,
            &none,
            &all_ready,
            true,
            |s| s.can_retire(split)
        ));
    }

    #[test]
    fn test_out_of_range_node_retires_immediately() {
        let anchor = chunk(0, 4, 0);
        let selected = select_lods(anchor, 3, BAND);
        let far = LodKey {
            level: 3,
            coord: I64Vec3::new(100, 0, 0),
        };
        let none = |_| false;
        assert!(with_state(
            anchor,
            &selected,
            &none,
            &HashMap::new(),
            false,
            |s| s.can_retire(far)
        ));
    }

    #[test]
    fn test_loaded_distance_is_full_range_once_everything_is_ready() {
        let anchor = chunk(0, 4, 0);
        let selected = select_lods(anchor, 3, BAND);
        let none = |_| false;
        let all_ready: HashMap<_, _> = selected.iter().map(|&k| (k, true)).collect();
        assert_eq!(
            lod_render_distance(3),
            with_state(anchor, &selected, &none, &all_ready, true, |s| s
                .loaded_distance())
        );
    }

    #[test]
    fn test_loaded_distance_stops_at_nearest_unloaded_gap() {
        let anchor = chunk(0, 4, 0);
        let selected = select_lods(anchor, 3, BAND);
        let gap = *selected.iter().find(|k| k.level == 3).unwrap();
        let none = |_| false;
        let mut ready: HashMap<_, _> = selected.iter().map(|&k| (k, true)).collect();
        ready.insert(gap, false);
        assert_eq!(
            gap.distance(anchor),
            with_state(anchor, &selected, &none, &ready, true, |s| s
                .loaded_distance())
        );
    }

    #[test]
    fn test_loaded_distance_ignores_nodes_still_covered_by_what_they_replace() {
        let anchor = chunk(0, 4, 0);
        let selected = select_lods(anchor, 3, BAND);
        let node = *selected.iter().find(|k| k.level == 2).unwrap();
        let mut ready: HashMap<_, _> = selected.iter().map(|&k| (k, true)).collect();
        ready.insert(node, false);

        // the coarser node it's replacing is still up
        let parent = node.parent();
        let parent_exists = move |k| k == parent;
        assert_eq!(
            lod_render_distance(3),
            with_state(anchor, &selected, &parent_exists, &ready, true, |s| s
                .loaded_distance())
        );
        // the finer nodes it's replacing are still up
        let children = node.children();
        let children_exist = move |k| children.contains(&k);
        assert_eq!(
            lod_render_distance(3),
            with_state(anchor, &selected, &children_exist, &ready, true, |s| s
                .loaded_distance())
        );
    }

    #[test]
    fn test_selected_node_hidden_while_coarser_node_remains() {
        let anchor = chunk(0, 4, 0);
        let selected = select_lods(anchor, 3, BAND);
        let node = *selected.iter().find(|k| k.level == 2).unwrap();
        let parent = node.parent();

        let none = |_| false;
        let parent_only = move |k| k == parent;
        let ready = HashMap::new();
        assert!(with_state(anchor, &selected, &none, &ready, true, |s| s
            .node_visible(node)));
        assert!(!with_state(
            anchor,
            &selected,
            &parent_only,
            &ready,
            true,
            |s| s.node_visible(node)
        ));
        // the node on its way out stays visible until it's removed
        assert!(with_state(
            anchor,
            &selected,
            &parent_only,
            &ready,
            true,
            |s| s.node_visible(parent)
        ));
    }

    #[test]
    fn test_unwanted_chunk_hidden_once_covering_node_ready() {
        let anchor = chunk(0, 4, 0);
        let selected = select_lods(anchor, 3, BAND);
        let c = chunk(48, 4, 0);
        assert!(!lod0_wanted(c, anchor));
        let cover = (1..=3)
            .map(|level| LodKey::containing(c, level))
            .find(|k| selected.contains(k))
            .unwrap();

        let none = |_| false;
        let mut ready = HashMap::new();
        assert!(with_state(anchor, &selected, &none, &ready, true, |s| s
            .chunk_visible(c)));
        ready.insert(cover, true);
        assert!(!with_state(anchor, &selected, &none, &ready, true, |s| s
            .chunk_visible(c)));
    }
}
