use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use bevy::{
    asset::{Assets, Handle},
    ecs::{
        component::Component,
        entity::Entity,
        query::{Added, Or, With},
        system::{Commands, Local, Query, Res, ResMut, Resource, SystemParam},
    },
    hierarchy::{BuildChildren, ChildBuild},
    math::{I64Vec2, I64Vec3, Vec3},
    pbr::MeshMaterial3d,
    prelude::Mesh3d,
    render::{mesh::Mesh, primitives::Aabb, view::Visibility},
    tasks::{AsyncComputeTaskPool, Task},
    time::Time,
    transform::components::Transform,
    utils::{futures, HashMap, HashSet},
};

use super::{
    chunk::{ChunkCoordinate, CHUNK_SIZE},
    chunk_loader::{ChunkLoader, DirtyChunk, GenerateChunkData, GenerateChunkMesh},
    fade::{fade_out_and_despawn, set_shown, Fade},
    group::{BuiltGroup, ChunkGroup, GroupedChunk, MeshedChunk},
    generate::{
        columns::COARSE_CELL,
        generator::{generate_lod_mesh, ChunkMeshes},
        region_store::RegionStore,
        trees::{TREE_HEIGHT, TREE_REACH},
        WATER_LEVEL,
    },
    material::{ChunkMaterial, WaterMaterial},
};
use crate::world::World;

/// How the world is split into detail levels, set from the renderer settings.
#[derive(Debug, Copy, Clone)]
pub struct LodLayout {
    /// Chunks within this many chunks of the camera are drawn at full detail.
    pub full_detail_distance: f32,
    /// Each LOD level up to the set render distance reaches this many times as far as the one before.
    pub level_scale: f32,
    // The levels that draw out to the set render distance. Any beyond, drawing further for a
    // view from high up, each reach only twice as far as the one before: nodes double in
    // width each level, so that adds about as many nodes a level, where `level_scale` times
    // as far would add more each time.
    scaled_levels: u8,
    pub max_level: u8,
    render_distance: f32,
}

impl LodLayout {
    /// Uses the fewest LOD levels that draw terrain out to `render_distance` chunks.
    pub fn new(full_detail_distance: u32, render_distance: u32, level_scale: u32) -> Self {
        let full_detail_distance = full_detail_distance.max(1) as f32;
        let layout = Self {
            full_detail_distance,
            // below 2, a level's nodes would be wider than the ring it draws
            level_scale: level_scale.max(2) as f32,
            scaled_levels: u8::MAX,
            max_level: 0,
            render_distance: 0.0,
        };
        let layout = layout.with_render_distance(render_distance as f32);
        Self {
            scaled_levels: layout.max_level,
            ..layout
        }
    }

    /// This layout drawing out to `render_distance` chunks instead, adding or dropping levels
    /// beyond the set render distance. Never draws less than full detail reaches.
    pub fn with_render_distance(self, render_distance: f32) -> Self {
        let mut layout = Self {
            max_level: 0,
            render_distance: render_distance.max(self.full_detail_distance),
            ..self
        };
        while layout.reach(layout.max_level) < layout.render_distance {
            layout.max_level += 1;
        }
        layout
    }

    /// How far, in chunks, terrain is drawn.
    pub fn render_distance(self) -> f32 {
        self.render_distance
    }

    /// How far, in chunks, `level` draws out to - level 0 being full detail.
    pub fn reach(self, level: u8) -> f32 {
        let scaled = level.min(self.scaled_levels);
        let doubled = level - scaled;
        self.full_detail_distance * self.level_scale.powi(scaled as i32) * 2f32.powi(doubled as i32)
    }

    /// How far, in chunks, terrain is built from full-detail regions: LOD nodes with coarser
    /// cells sample the noise themselves, see `columns::COARSE_CELL`.
    pub fn region_reach(self) -> f32 {
        let fine_levels = (0..=self.max_level).take_while(|&level| (1i64 << level) < COARSE_CELL);
        self.reach(fine_levels.last().unwrap_or(0))
    }

    /// Whether `key` is drawn as its 8 children: it's within reach of the next level down.
    pub fn splits(self, key: LodKey, anchor: ChunkCoordinate) -> bool {
        key.level > 0 && key.distance(anchor) < self.reach(key.level - 1)
    }

    /// Whether a chunk is close enough to be drawn at full detail.
    pub fn full_detail(self, chunk: ChunkCoordinate, anchor: ChunkCoordinate) -> bool {
        self.splits(LodKey::containing(chunk, 1), anchor)
    }

    /// How far out, in chunks, the farthest LOD level starts: halfway out with no LOD levels.
    pub fn last_level_start(self) -> f32 {
        match self.max_level {
            0 => self.render_distance / 2.0,
            level => self.reach(level - 1),
        }
    }

    /// How far full-detail chunks reach: a level-1 node in range, plus its diagonal.
    pub fn full_detail_reach(self) -> u32 {
        (self.full_detail_distance + 2.0 * 3f32.sqrt()).ceil() as u32
    }
}

// LOD meshes take well under a millisecond, so enough are kept in flight that the
// once-per-frame refill isn't what limits how fast they load.
const LOD_TASKS_PER_THREAD: usize = 32;
// The spawn queue is re-sorted this often, since nodes drift between covered and gap as terrain loads.
const LOD_RESORT_INTERVAL: f32 = 2.0;
// Nodes known to have nothing to draw are added straight away without a task or an entity, up to this many a frame.
const LOD_EMPTY_NODES_PER_FRAME: usize = 8192;
const LOD_APPLY_TIME_BUDGET: Duration = Duration::from_millis(2);

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

    /// This node and each one above it, up to `top`.
    pub fn self_and_ancestors(self, top: u8) -> impl Iterator<Item = LodKey> {
        std::iter::successors(Some(self), move |key| (key.level < top).then(|| key.parent()))
    }

    /// Whether the level-1 node `group` lies within this one.
    pub fn contains_group(self, group: LodKey) -> bool {
        match self.level {
            0 => group.coord == self.coord.div_euclid(I64Vec3::splat(2)),
            level => group.coord.div_euclid(I64Vec3::splat(1 << (level - 1))) == self.coord,
        }
    }

    /// The level-1 nodes within this one, or just it if it's level 1.
    pub fn groups_in(self) -> impl Iterator<Item = LodKey> {
        let width = 1i64 << self.level.saturating_sub(1);
        let min = if self.level == 0 { self.coord.div_euclid(I64Vec3::splat(2)) } else { self.coord * width };
        let count = if self.level == 0 { 1 } else { width };
        (0..count).flat_map(move |x| {
            (0..count).flat_map(move |y| {
                (0..count).map(move |z| LodKey { level: 1, coord: min + I64Vec3::new(x, y, z) })
            })
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
}

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
pub fn select_lods(anchor: ChunkCoordinate, layout: LodLayout, band: LodBand) -> HashSet<LodKey> {
    select_lods_and_stand_ins(anchor, layout, band).0
}

/// The LOD nodes to draw around `anchor`, and the level-1 nodes covering the full-detail
/// area, which stand in for its chunks until they're ready.
pub fn select_lods_and_stand_ins(
    anchor: ChunkCoordinate,
    layout: LodLayout,
    band: LodBand,
) -> (HashSet<LodKey>, HashSet<LodKey>) {
    let mut selected = HashSet::new();
    let mut stand_ins = HashSet::new();
    let max_level = layout.max_level;
    if max_level == 0 {
        return (selected, stand_ins);
    }

    let width = 1i64 << max_level;
    let range = layout.render_distance();
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
                    select_within(key, anchor, layout, band, &mut selected, &mut stand_ins);
                }
            }
        }
    }
    (selected, stand_ins)
}

fn select_within(
    key: LodKey,
    anchor: ChunkCoordinate,
    layout: LodLayout,
    band: LodBand,
    selected: &mut HashSet<LodKey>,
    stand_ins: &mut HashSet<LodKey>,
) {
    if !band.intersects(key) {
        return;
    }
    if !layout.splits(key, anchor) {
        selected.insert(key);
    } else if key.level > 1 {
        for child in key.children() {
            select_within(child, anchor, layout, band, selected, stand_ins);
        }
    } else {
        stand_ins.insert(key);
    }
}

/// What the retirement and visibility rules need to know about the current state.
pub struct LodState<'a> {
    pub anchor: ChunkCoordinate,
    pub layout: LodLayout,
    pub band: LodBand,
    pub selected: &'a HashSet<LodKey>,
    pub node_exists: &'a dyn Fn(LodKey) -> bool,
    pub node_ready: &'a dyn Fn(LodKey) -> bool,
    pub chunk_state: &'a dyn Fn(ChunkCoordinate) -> ChunkState,
}

impl LodState<'_> {
    fn selected_ancestor(&self, mut key: LodKey) -> Option<LodKey> {
        while key.level < self.layout.max_level {
            key = key.parent();
            if self.selected.contains(&key) {
                return Some(key);
            }
        }
        None
    }

    fn has_existing_ancestor(&self, mut key: LodKey) -> bool {
        while key.level < self.layout.max_level {
            key = key.parent();
            if (self.node_exists)(key) {
                return true;
            }
        }
        false
    }

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
        if !self.layout.splits(key, self.anchor) {
            return true;
        }
        if key.level == 1 {
            return key.chunks().all(|chunk| (self.chunk_state)(chunk).ready());
        }
        key.children()
            .into_iter()
            .all(|child| !self.band.intersects(child) || self.covered(child))
    }

    pub fn node_visible(&self, key: LodKey) -> bool {
        !self.selected.contains(&key) || !self.has_existing_ancestor(key)
    }

    pub fn chunk_visible(&self, chunk: ChunkCoordinate) -> bool {
        let key = LodKey::containing(chunk, 0);
        if self.layout.full_detail(chunk, self.anchor) {
            !self.has_existing_ancestor(key)
        } else {
            !self
                .selected_ancestor(key)
                .is_some_and(|ancestor| (self.node_ready)(ancestor))
        }
    }

    // Whether finer nodes, or for a level-1 node its chunks, are drawn where it goes.
    fn finer_drawn(&self, key: LodKey) -> bool {
        if key.level == 1 {
            key.chunks()
                .any(|chunk| (self.chunk_state)(chunk) == ChunkState::Drawn)
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

impl LodNode {
    pub fn key(&self) -> LodKey {
        self.key
    }
}

#[derive(Component)]
pub struct GenerateLodMesh {
    task: Task<Option<ChunkMeshes>>,
}

#[derive(Resource)]
pub struct LodLoader {
    // as set, drawing out to the set render distance
    set_layout: LodLayout,
    layout: LodLayout,
    band: LodBand,
    anchor: Option<ChunkCoordinate>,
    selected: Arc<HashSet<LodKey>>,
    // level-1 nodes over the full-detail area, drawn while its chunks aren't ready
    stand_ins: Arc<HashSet<LodKey>>,
    // the walk choosing nodes for a new anchor, which runs off the main thread
    selecting: Option<Task<LodSelection>>,
    // counts layout changes, which make a walk under the old one out of date
    generation: u32,
    // each node's entity, or none for one with nothing to draw, which needs no entity at all
    nodes: HashMap<LodKey, Option<Entity>>,
    // how many nodes exist at or below each key, so areas are walked only where nodes are
    populated: HashMap<LodKey, u32>,
    // farthest first, so popping yields the nearest
    to_spawn: Vec<LodKey>,
    retiring: Vec<LodKey>,
    // areas whose nodes and chunks need their visibility rechecked
    dirty_areas: Vec<LodKey>,
    refresh_all: bool,
    material: Handle<ChunkMaterial>,
    water_material: Handle<WaterMaterial>,
}

// Levels `LodLoader::populated` counts up through, however far the layout reaches.
const POPULATED_LEVELS: u8 = 16;

impl LodLoader {
    fn add_node(&mut self, key: LodKey, entity: Option<Entity>) {
        if self.nodes.insert(key, entity).is_none() {
            for ancestor in key.self_and_ancestors(POPULATED_LEVELS) {
                *self.populated.entry(ancestor).or_insert(0) += 1;
            }
        }
    }

    fn remove_node(&mut self, key: LodKey) -> Option<Option<Entity>> {
        let entity = self.nodes.remove(&key)?;
        for ancestor in key.self_and_ancestors(POPULATED_LEVELS) {
            if let Some(count) = self.populated.get_mut(&ancestor) {
                *count -= 1;
                if *count == 0 {
                    self.populated.remove(&ancestor);
                }
            }
        }
        Some(entity)
    }

    /// The nodes that exist at or below `area`, found without visiting keys that hold none.
    fn nodes_under(&self, area: LodKey) -> Vec<(LodKey, Entity)> {
        let mut found = Vec::new();
        let mut pending = vec![area];
        while let Some(key) = pending.pop() {
            if !self.populated.contains_key(&key) {
                continue;
            }
            if let Some(&Some(entity)) = self.nodes.get(&key) {
                found.push((key, entity));
            }
            if key.level > 1 {
                pending.extend(key.children());
            }
        }
        found
    }

    pub fn layout(&self) -> LodLayout {
        self.layout
    }

    /// Draws terrain out to `render_distance` chunks, or the set render distance if that's
    /// further, selecting nodes afresh for it.
    pub fn set_render_distance(&mut self, render_distance: f32) {
        let layout = self.set_layout.with_render_distance(render_distance.max(self.set_layout.render_distance()));
        if layout.render_distance() != self.layout.render_distance() {
            self.layout = layout;
            self.anchor = None;
            self.generation += 1;
        }
    }

    /// The render distance as set, which `set_render_distance` never goes below.
    pub fn set_render_distance_chunks(&self) -> f32 {
        self.set_layout.render_distance()
    }

    /// Whether a chunk outside full detail can be unloaded without leaving a hole.
    pub fn chunk_covered(&self, chunk: ChunkCoordinate, pending: &Query<(), With<GenerateLodMesh>>) -> bool {
        let Some(anchor) = self.anchor else {
            return false;
        };
        if self.layout.full_detail(chunk, anchor) {
            return false;
        }
        let mut key = LodKey::containing(chunk, 0);
        while key.level < self.layout.max_level {
            key = key.parent();
            if self.selected.contains(&key) {
                return node_ready(&self.nodes, pending, key);
            }
        }
        true
    }

    pub fn new(
        layout: LodLayout,
        world_height: u64,
        material: Handle<ChunkMaterial>,
        water_material: Handle<WaterMaterial>,
    ) -> Self {
        Self {
            set_layout: layout,
            layout,
            band: LodBand {
                min_y: 0,
                max_y: (world_height as i64 + CHUNK_SIZE as i64 - 1) / CHUNK_SIZE as i64,
            },
            anchor: None,
            selected: Arc::new(HashSet::new()),
            stand_ins: Arc::new(HashSet::new()),
            selecting: None,
            generation: 0,
            nodes: HashMap::new(),
            populated: HashMap::new(),
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
    With<MeshedChunk>,
)>;

/// How far along a full-detail chunk is, as far as LODs are concerned.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ChunkState {
    Missing,
    Loading,
    /// Loaded with nothing to draw.
    Empty,
    Drawn,
}

impl ChunkState {
    fn ready(self) -> bool {
        matches!(self, ChunkState::Empty | ChunkState::Drawn)
    }
}

#[derive(SystemParam)]
pub struct ChunkStates<'w, 's> {
    loader: Res<'w, ChunkLoader>,
    pending: Query<'w, 's, (), PendingChunk>,
    drawn: Query<'w, 's, (), With<GroupedChunk>>,
}

impl ChunkStates<'_, '_> {
    fn get(&self, chunk: ChunkCoordinate) -> ChunkState {
        match self.loader.chunk_entity(chunk) {
            None => ChunkState::Missing,
            Some(e) if self.drawn.contains(e) => ChunkState::Drawn,
            Some(e) if self.pending.contains(e) => ChunkState::Loading,
            Some(_) => ChunkState::Empty,
        }
    }
}

// Gaps with nothing drawn yet count as this fraction of their distance, so holes fill in
// quickly without holding up more detail near the player.
const GAP_DISTANCE_WEIGHT: f32 = 0.5;

// Nodes to spawn, popped from the end: chunk swaps first, then nearest first, with gaps
// counted as nearer than they are.
fn spawn_order(state: &LodState, unspawned: impl Iterator<Item = LodKey>) -> Vec<LodKey> {
    let mut queued: Vec<(bool, f32, LodKey)> = unspawned
        .map(|key| {
            let distance = key.distance(state.anchor);
            let finer_drawn = state.finer_drawn(key);
            let gap = !finer_drawn && !state.has_existing_ancestor(key);
            (
                // a level-1 node standing in for chunks that are still drawn
                key.level == 1 && finer_drawn,
                if gap { distance * GAP_DISTANCE_WEIGHT } else { distance },
                key,
            )
        })
        .collect();
    queued.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.total_cmp(&a.1)));
    queued.into_iter().map(|(.., key)| key).collect()
}

/// The nodes chosen for an anchor, and how they differ from those chosen before.
pub struct LodSelection {
    generation: u32,
    anchor: ChunkCoordinate,
    selected: HashSet<LodKey>,
    stand_ins: HashSet<LodKey>,
    // keys that entered or left either set, or none for a first selection
    changed: Vec<LodKey>,
}

pub fn update_lod_selection(
    mut lods: ResMut<LodLoader>,
    chunks: ChunkStates,
    pending_lods: Query<(), With<GenerateLodMesh>>,
) {
    if let Some(mut task) = lods.selecting.take() {
        match futures::check_ready(&mut task) {
            // one chosen under a layout since replaced is thrown away
            Some(done) if done.generation == lods.generation => apply_selection(&mut lods, done, &chunks, &pending_lods),
            Some(_) => {}
            None => lods.selecting = Some(task),
        }
    }

    // the walk is the slow part, so it runs off the main thread, a camera move behind
    let anchor = chunks.loader.anchor();
    if lods.selecting.is_some() || lods.anchor == Some(anchor) {
        return;
    }
    let (layout, band, generation) = (lods.layout, lods.band, lods.generation);
    let previous = lods.anchor.map(|_| (lods.selected.clone(), lods.stand_ins.clone()));
    lods.selecting = Some(AsyncComputeTaskPool::get().spawn(async move {
        let (selected, stand_ins) = select_lods_and_stand_ins(anchor, layout, band);
        // whether a node or chunk shows depends on the anchor only through these two sets, so
        // only where they changed needs rechecking
        let changed = previous
            .map(|(old_selected, old_stand_ins)| {
                selected
                    .symmetric_difference(&old_selected)
                    .chain(stand_ins.symmetric_difference(&old_stand_ins))
                    .copied()
                    .collect()
            })
            .unwrap_or_default();
        LodSelection { generation, anchor, selected, stand_ins, changed }
    }));
}

fn apply_selection(lods: &mut LodLoader, done: LodSelection, chunks: &ChunkStates, pending_lods: &Query<(), With<GenerateLodMesh>>) {
    let LodSelection { anchor, selected, stand_ins, changed, .. } = done;
    let first = lods.anchor.is_none();
    let nodes = &lods.nodes;
    let node_exists = |key: LodKey| nodes.contains_key(&key);
    let node_ready = |key: LodKey| node_ready(nodes, pending_lods, key);
    let state = LodState {
        anchor,
        layout: lods.layout,
        band: lods.band,
        selected: &selected,
        node_exists: &node_exists,
        node_ready: &node_ready,
        chunk_state: &|chunk| chunks.get(chunk),
    };

    // The first selection, and one for a new layout, start from scratch and check everything.
    // After that only the keys that changed can need spawning or retiring, so nothing walks
    // every node, of which there can be hundreds of thousands.
    let wanted = |key: &LodKey| selected.contains(key) || (stand_ins.contains(key) && !state.covered(*key));
    let (to_spawn, retiring) = if first {
        let to_spawn = spawn_order(
            &state,
            selected
                .iter()
                .chain(stand_ins.iter().filter(|&&key| !state.covered(key)))
                .filter(|&key| !nodes.contains_key(key))
                .copied(),
        );
        let retiring = nodes.keys().filter(|&key| !selected.contains(key)).copied().collect();
        (to_spawn, retiring)
    } else {
        // new arrivals go on the end, so are spawned first until the queue is next sorted
        let mut to_spawn = std::mem::take(&mut lods.to_spawn);
        to_spawn.extend(spawn_order(&state, changed.iter().copied().filter(|key| wanted(key) && !nodes.contains_key(key))));
        let mut retiring = std::mem::take(&mut lods.retiring);
        retiring.retain(|key| !selected.contains(key) && nodes.contains_key(key));
        let queued: HashSet<LodKey> = retiring.iter().copied().collect();
        retiring.extend(changed.iter().copied().filter(|key| !selected.contains(key) && nodes.contains_key(key) && !queued.contains(key)));
        (to_spawn, retiring)
    };

    if first {
        lods.refresh_all = true;
    } else {
        lods.dirty_areas.extend(changed);
    }
    lods.anchor = Some(anchor);
    lods.selected = Arc::new(selected);
    lods.stand_ins = Arc::new(stand_ins);
    lods.to_spawn = to_spawn;
    lods.retiring = retiring;
}

pub fn spawn_lods(
    mut commands: Commands,
    mut lods: ResMut<LodLoader>,
    world: Res<World>,
    time: Res<Time>,
    chunks: ChunkStates,
    pending: Query<(), With<GenerateLodMesh>>,
    mut since_resort: Local<f32>,
) {
    *since_resort += time.delta_secs();
    if *since_resort >= LOD_RESORT_INTERVAL {
        *since_resort = 0.0;
        if let Some(anchor) = lods.anchor {
            let remaining = std::mem::take(&mut lods.to_spawn);
            let nodes = &lods.nodes;
            let node_exists = |key: LodKey| nodes.contains_key(&key);
            let node_ready =
                |key: LodKey| node_ready(nodes, &pending, key);
            let state = LodState {
                anchor,
                layout: lods.layout,
                band: lods.band,
                selected: &lods.selected,
                node_exists: &node_exists,
                node_ready: &node_ready,
                chunk_state: &|chunk| chunks.get(chunk),
            };
            let resorted = spawn_order(&state, remaining.into_iter());
            lods.to_spawn = resorted;
        }
    }

    let task_pool = AsyncComputeTaskPool::get();
    let max_in_flight = LOD_TASKS_PER_THREAD * task_pool.thread_num();
    let mut in_flight = pending.iter().count();
    let mut empty_nodes = 0;

    while empty_nodes < LOD_EMPTY_NODES_PER_FRAME {
        let Some(key) = lods.to_spawn.pop() else {
            break;
        };
        if lods.nodes.contains_key(&key) {
            continue;
        }
        let stand_in = lods.stand_ins.contains(&key);
        // a stand-in is only needed until its chunks are ready
        let chunks_ready = || key.chunks().all(|chunk| chunks.get(chunk).ready());
        if !lods.selected.contains(&key) && !(stand_in && !chunks_ready()) {
            continue;
        }
        // nothing to mesh, so the node is ready as soon as it exists
        if known_empty(key, &world.region_store) {
            lods.add_node(key, None);
            if stand_in {
                lods.retiring.push(key);
            }
            empty_nodes += 1;
            continue;
        }
        if in_flight >= max_in_flight {
            lods.to_spawn.push(key);
            break;
        }

        let regions = world.region_store.clone();
        let entity = commands
            .spawn((
                LodNode { key },
                GenerateLodMesh {
                    task: task_pool
                        .spawn(async move { generate_lod_mesh(regions, key) }),
                },
                lod_transform(key),
                Visibility::Hidden,
            ))
            .id();
        lods.add_node(key, Some(entity));
        if stand_in {
            lods.retiring.push(key);
        }
        in_flight += 1;
    }
}

// Whether the node at `key` exists and has its mesh, or has nothing to draw.
fn node_ready(nodes: &HashMap<LodKey, Option<Entity>>, pending: &Query<(), With<GenerateLodMesh>>, key: LodKey) -> bool {
    nodes.get(&key).is_some_and(|slot| slot.map_or(true, |entity| !pending.contains(entity)))
}

// Whether `key` is known to have nothing to draw, going by regions or coarse columns
// already sampled - never samples one, so it can run on the main thread. That's when the terrain its mesh
// would sample is all below it with only air or only water above, or all above it so it's
// buried in rock. Only knows about generated terrain, since LOD meshes are built from it
// alone - if they ever show player edits or caves, those must stop counting as empty here.
fn known_empty(key: LodKey, regions: &RegionStore) -> bool {
    let min = key.min_block();
    let cell = key.cell_size();
    let width = key.width() * CHUNK_SIZE as i64;
    // meshes sample a cell past the node's edges, a block either side for slopes, and trees
    // standing just outside can reach in
    let margin = cell + 1 + TREE_REACH;
    let min_xz = I64Vec2::new(min.x, min.z) - margin;
    let max_xz = I64Vec2::new(min.x, min.z) + width - 1 + margin;
    let range = if cell >= COARSE_CELL {
        regions.column_height_range(key)
    } else {
        regions.loaded_height_range(min_xz, max_xz)
    };
    let Some((lowest, highest)) = range else {
        return false;
    };
    let (bottom, top) = (min.y - cell, min.y + width + cell);
    // coarse nodes' canopies stand at least a cell tall
    let above_terrain = bottom > highest.round() as i64 + 1 + TREE_HEIGHT.max(cell);
    let in_air = bottom > WATER_LEVEL;
    let under_water = top < WATER_LEVEL - 1;
    let buried = top < lowest.round() as i64 - 1;
    above_terrain && (in_air || under_water) || buried
}

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

        let has_mesh = result.is_some();
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
        if has_mesh {
            commands.entity(entity).remove::<GenerateLodMesh>();
        } else {
            // meshed to nothing, so it needs no entity
            commands.entity(entity).despawn();
            lods.add_node(node.key, None);
        }
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
    chunks: ChunkStates,
    pending_lods: Query<(), With<GenerateLodMesh>>,
    visibility: Query<&Visibility>,
) {
    let Some(anchor) = lods.anchor else {
        return;
    };

    let mut retired = Vec::new();
    {
        let nodes = &lods.nodes;
        let node_exists = |key: LodKey| nodes.contains_key(&key);
        let node_ready = |key: LodKey| node_ready(nodes, &pending_lods, key);
        let state = LodState {
            anchor,
            layout: lods.layout,
            band: lods.band,
            selected: &lods.selected,
            node_exists: &node_exists,
            node_ready: &node_ready,
            chunk_state: &|chunk| chunks.get(chunk),
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
    for key in retired {
        if let Some(Some(entity)) = lods.remove_node(key) {
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
    all_groups: Query<(Entity, &ChunkGroup), With<BuiltGroup>>,
    new_groups: Query<(Entity, &ChunkGroup), Added<BuiltGroup>>,
    built_groups: Query<(), With<BuiltGroup>>,
    // only nodes with something to draw - nodes still loading are shown when their mesh lands
    all_nodes: Query<(Entity, &LodNode), With<Mesh3d>>,
    mut visibility: Query<(&mut Visibility, Option<&Fade>)>,
) {
    let Some(anchor) = lods.anchor else {
        return;
    };

    let now = time.elapsed_secs();
    let lods = &mut *lods;
    let nodes = &lods.nodes;
    let node_exists = |key: LodKey| nodes.contains_key(&key);
    let node_ready = |key: LodKey| node_ready(nodes, &pending_lods, key);
    let state = LodState {
        anchor,
        layout: lods.layout,
        band: lods.band,
        selected: &lods.selected,
        node_exists: &node_exists,
        node_ready: &node_ready,
        // the visibility rules don't look at chunks' loading state
        chunk_state: &|_| ChunkState::Missing,
    };
    let mut set = |entity: Entity, visible: bool| {
        if let Ok((mut v, fade)) = visibility.get_mut(entity) {
            set_shown(&mut commands, entity, &mut v, fade, visible, now);
        }
    };
    // a node fades in once its mesh is ready, not when it's first spawned
    let node_shown = |key: LodKey| state.node_visible(key) && node_ready(key);

    if lods.refresh_all {
        for (entity, group) in all_groups.iter() {
            set(entity, state.chunk_visible(ChunkCoordinate(group.key.min_chunk())));
        }
        for (entity, node) in all_nodes.iter() {
            set(entity, node_shown(node.key));
        }
    } else {
        for &area in &lods.dirty_areas {
            // a group is drawn whole, so one chunk of it says whether it shows
            let in_area = |group: LodKey| {
                chunk_loader.group_entity(group).filter(|&e| built_groups.contains(e)).map(|e| (group, e))
            };
            let width = 1usize << (3 * area.level.saturating_sub(1) as usize).min(40);
            let groups: Vec<(LodKey, Entity)> = if width <= chunk_loader.groups.len() {
                LodKey::groups_in(area).filter_map(in_area).collect()
            } else {
                chunk_loader.groups.keys().filter(|&&group| area.contains_group(group)).filter_map(|&group| in_area(group)).collect()
            };
            for (group, entity) in groups {
                set(entity, state.chunk_visible(ChunkCoordinate(group.min_chunk())));
            }
            for (key, entity) in lods.nodes_under(area) {
                set(entity, node_shown(key));
            }
        }
    }

    // newly built groups arrive hidden, and fade in unless a node still covers them
    for (entity, group) in new_groups.iter() {
        set(entity, state.chunk_visible(ChunkCoordinate(group.key.min_chunk())));
    }

    lods.refresh_all = false;
    lods.dirty_areas.clear();
}

#[cfg(test)]
mod tests {
    use bevy::{
        asset::Handle,
        ecs::entity::Entity,
        math::I64Vec3,
        utils::{HashMap, HashSet},
    };

    use super::{select_lods, select_lods_and_stand_ins, ChunkState, LodBand, LodKey, LodLayout, LodLoader, LodState};
    use crate::chunks::chunk::ChunkCoordinate;

    const BAND: LodBand = LodBand {
        min_y: 0,
        max_y: 16,
    };

    // Each level reaching twice as far as the last, out to the edge of `max_level`.
    fn layout(max_level: u8) -> LodLayout {
        LodLayout::new(32, 32 << max_level, 2)
    }

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
    fn test_layout_uses_fewest_levels_reaching_render_distance() {
        assert_eq!(0, LodLayout::new(32, 32, 2).max_level);
        assert_eq!(1, LodLayout::new(32, 33, 2).max_level);
        assert_eq!(2, LodLayout::new(32, 128, 2).max_level);
        assert_eq!(3, LodLayout::new(16, 128, 2).max_level);
        assert_eq!(2, LodLayout::new(16, 128, 4).max_level);
        assert_eq!(3, LodLayout::new(16, 257, 4).max_level);
        assert_eq!(128.0, layout(2).render_distance());
    }

    #[test]
    fn test_levels_past_the_set_distance_reach_twice_as_far() {
        let set = LodLayout::new(16, 256, 4);
        let high = set.with_render_distance(2048.0);
        assert_eq!(set.max_level + 3, high.max_level);
        for level in 0..=set.max_level {
            assert_eq!(set.reach(level), high.reach(level));
        }
        assert_eq!(512.0, high.reach(3));
        assert_eq!(2048.0, high.reach(5));
        // and back down again
        assert_eq!(set.max_level, high.with_render_distance(256.0).max_level);
    }

    #[test]
    fn test_regions_reach_only_as_far_as_fine_levels() {
        assert_eq!(64.0, LodLayout::new(16, 256, 4).region_reach());
        assert_eq!(16.0, LodLayout::new(16, 16, 4).region_reach());
    }

    #[test]
    fn test_last_level_starts_where_the_one_before_ends() {
        let layout = LodLayout::new(16, 256, 4);
        assert_eq!(64.0, layout.last_level_start());
        assert_eq!(32.0, LodLayout::new(64, 64, 4).last_level_start());
    }

    #[test]
    fn test_render_distance_is_not_rounded_up_to_a_level() {
        let layout = LodLayout::new(16, 100, 4);
        assert_eq!(100.0, layout.render_distance());
        assert_eq!(256.0, layout.reach(layout.max_level));
    }

    #[test]
    fn test_each_level_reaches_level_scale_times_further() {
        let layout = LodLayout::new(16, 1024, 4);
        assert_eq!([16.0, 64.0, 256.0, 1024.0], [0, 1, 2, 3].map(|l| layout.reach(l)));
        let anchor = chunk(0, 4, 0);
        let selected = select_lods(anchor, layout, BAND);
        // a level-1 node 50 chunks out is drawn whole, a level-2 node that close is split
        let level_one = LodKey::containing(chunk(50, 4, 0), 1);
        assert!(selected.contains(&level_one));
        assert!(!selected.contains(&level_one.parent()));
    }

    #[test]
    fn test_full_detail_reaches_full_detail_distance() {
        let anchor = chunk(0, 5, 0);
        assert!(layout(2).full_detail(chunk(0, 5, 0), anchor));
        assert!(layout(2).full_detail(chunk(31, 5, 0), anchor));
        assert!(!layout(2).full_detail(chunk(36, 5, 0), anchor));
    }

    #[test]
    fn test_full_detail_distance_comes_from_layout() {
        let anchor = chunk(0, 5, 0);
        let near = LodLayout::new(16, 128, 2);
        assert!(near.full_detail(chunk(15, 5, 0), anchor));
        assert!(!near.full_detail(chunk(20, 5, 0), anchor));
        assert_eq!(20, near.full_detail_reach());
    }

    fn assert_exact_cover(anchor: ChunkCoordinate, max_level: u8) {
        assert_exact_cover_with(anchor, layout(max_level));
        // further out per level, and a render distance short of the last level's reach
        assert_exact_cover_with(anchor, LodLayout::new(8, 100, 4));
    }

    fn assert_exact_cover_with(anchor: ChunkCoordinate, layout: LodLayout) {
        let max_level = layout.max_level;
        let selected = select_lods(anchor, layout, BAND);
        let range = layout.render_distance() as i64;
        for x in -range..range {
            for z in -range..range {
                for y in BAND.min_y..BAND.max_y {
                    let c = chunk(anchor.0.x + x, y, anchor.0.z + z);
                    let covering = (1..=max_level)
                        .filter(|&level| selected.contains(&LodKey::containing(c, level)))
                        .count()
                        + layout.full_detail(c, anchor) as usize;
                    assert!(covering <= 1, "chunk {:?} drawn {covering} times", c.0);
                    if LodKey::containing(c, 0).distance(anchor)
                        < layout.render_distance()
                    {
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
    fn test_stand_ins_cover_exactly_the_full_detail_chunks() {
        let anchor = chunk(3, 4, -2);
        let layout = LodLayout::new(8, 128, 4);
        let (_, stand_ins) = super::select_lods_and_stand_ins(anchor, layout, BAND);
        for x in -12..12 {
            for z in -12..12 {
                for y in BAND.min_y..BAND.max_y {
                    let c = chunk(anchor.0.x + x, y, anchor.0.z + z);
                    let covered = stand_ins.contains(&LodKey::containing(c, 1));
                    assert_eq!(layout.full_detail(c, anchor), covered, "chunk {:?}", c.0);
                }
            }
        }
    }

    #[test]
    fn test_selection_neighbours_differ_by_at_most_one_level() {
        for layout in [layout(3), LodLayout::new(8, 128, 4)] {
            let anchor = chunk(3, 4, -2);
            let selected = select_lods(anchor, layout, BAND);
            let level_at = |c: ChunkCoordinate| -> u8 {
                (1..=layout.max_level)
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
    }

    // Whether something is drawn where `key` goes while it loads, so it isn't a gap.
    fn covered_while_loading(state: &LodState, key: LodKey) -> bool {
        state.finer_drawn(key) || state.has_existing_ancestor(key)
    }

    fn replaces_loaded_chunks(state: &LodState, key: LodKey) -> bool {
        key.level == 1 && state.finer_drawn(key)
    }

    fn with_state<R>(
        anchor: ChunkCoordinate,
        selected: &HashSet<LodKey>,
        existing: &dyn Fn(LodKey) -> bool,
        ready: &HashMap<LodKey, bool>,
        chunks_ready: bool,
        check: impl FnOnce(&LodState) -> R,
    ) -> R {
        let node_ready = |key| ready.get(&key).copied().unwrap_or(false);
        let chunk_state = |_| if chunks_ready { ChunkState::Drawn } else { ChunkState::Missing };
        check(&LodState {
            anchor,
            layout: layout(3),
            band: BAND,
            selected,
            node_exists: existing,
            node_ready: &node_ready,
            chunk_state: &chunk_state,
        })
    }

    #[test]
    fn test_merged_node_retires_once_parent_ready() {
        // far enough that the level-3 node is drawn whole
        let far = chunk(200, 4, 0);
        let selected = select_lods(far, layout(3), BAND);
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
        let selected = select_lods(anchor, layout(3), BAND);
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
        let selected = select_lods(anchor, layout(3), BAND);
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

    // A node that, once it exists, keeps `key` covered while `key` loads.
    fn cover_for(key: LodKey) -> LodKey {
        if key.level > 1 {
            key.children()[0]
        } else {
            key.parent()
        }
    }

    fn is_ancestor(ancestor: LodKey, mut key: LodKey) -> bool {
        while key.level < ancestor.level {
            key = key.parent();
        }
        key == ancestor
    }

    // Spawn order for a detail swap (`covered`, with its cover already drawn) and a `gap`.
    fn spawn_order_of(anchor: ChunkCoordinate, covered: LodKey, gap: LodKey) -> Vec<LodKey> {
        let selected: HashSet<_> = [covered, gap].into_iter().collect();
        let cover = cover_for(covered);
        assert!(!is_ancestor(cover, gap));
        let exists = move |k| k == cover;
        with_state(anchor, &selected, &exists, &HashMap::new(), false, |s| {
            assert!(covered_while_loading(s, covered));
            assert!(!covered_while_loading(s, gap));
            let mut queue = super::spawn_order(s, [covered, gap].into_iter());
            queue.reverse();
            queue
        })
    }

    fn by_distance(anchor: ChunkCoordinate) -> Vec<LodKey> {
        let mut keys: Vec<_> = select_lods(anchor, layout(3), BAND).into_iter().collect();
        keys.sort_by(|a, b| a.distance(anchor).total_cmp(&b.distance(anchor)));
        keys
    }

    #[test]
    fn test_near_detail_swaps_spawn_before_far_gaps() {
        let anchor = chunk(0, 4, 0);
        let keys = by_distance(anchor);
        let near = *keys.iter().find(|k| k.distance(anchor) > 0.0).unwrap();
        let far = *keys.last().unwrap();
        assert!(near.distance(anchor) < far.distance(anchor) * super::GAP_DISTANCE_WEIGHT);
        assert_eq!(vec![near, far], spawn_order_of(anchor, near, far));
    }

    #[test]
    fn test_gaps_spawn_before_detail_swaps_as_far_away() {
        let anchor = chunk(0, 4, 0);
        let keys = by_distance(anchor);
        let farthest = *keys.last().unwrap();
        let gap = *keys
            .iter()
            .rev()
            .find(|&&k| k != farthest && !is_ancestor(cover_for(farthest), k))
            .unwrap();
        assert_eq!(vec![gap, farthest], spawn_order_of(anchor, farthest, gap));
    }

    #[test]
    fn test_known_empty_nodes_have_nothing_to_mesh() {
        use crate::chunks::generate::{
            generator::generate_lod_mesh, noise::NoiseGenerator, region_store::RegionStore,
            WATER_LEVEL, WORLD_HEIGHT,
        };
        use crate::chunks::chunk::CHUNK_SIZE;
        use bevy::math::I64Vec2;
        use std::sync::Arc;

        let noise = Arc::new(NoiseGenerator::new(42));
        // a mountain range and the deep ocean, so terrain and water fill many of the nodes
        let find = |wanted: &dyn Fn(f64) -> bool| {
            (-20..20)
                .flat_map(|z| (-20..20).map(move |x| I64Vec2::new(x, z) * 512))
                .find(|&p| wanted(noise.get(p) - WATER_LEVEL as f64))
                .expect("no such terrain near the centre")
        };
        let patches = [find(&|h| h > 250.0), find(&|h| h < -300.0)];

        let regions = Arc::new(RegionStore::new(noise.clone()));
        let layers = (WORLD_HEIGHT / CHUNK_SIZE as u64) as i64;
        let nodes: Vec<LodKey> = patches
            .iter()
            .flat_map(|&p| {
                let corner = chunk(p.x / CHUNK_SIZE as i64, 0, p.y / CHUNK_SIZE as i64);
                (1..=2u8).flat_map(move |level| {
                    let across = 16 >> level;
                    let first = LodKey::containing(corner, level).coord;
                    (0..across).flat_map(move |x| {
                        (0..layers >> level).flat_map(move |y| {
                            (0..across).map(move |z| LodKey {
                                level,
                                coord: first + I64Vec3::new(x, y, z),
                            })
                        })
                    })
                })
            })
            .collect();
        // nothing is known until the regions under a node are computed
        assert!(nodes.iter().all(|&key| !super::known_empty(key, &regions)));

        let mut empty = 0;
        for &key in &nodes {
            let meshed = generate_lod_mesh(regions.clone(), key).is_some();
            if super::known_empty(key, &regions) {
                assert!(!meshed, "{key:?} was taken as empty but has a mesh");
                empty += 1;
            }
        }
        // most of each column is air, rock or open water
        assert!(empty > nodes.len() * 3 / 4, "only {empty} of {} nodes known to be empty", nodes.len());
    }

    #[test]
    fn test_empty_loaded_chunks_neither_cover_nor_get_replaced() {
        let anchor = chunk(0, 4, 0);
        let layout = layout(2);
        let selected = select_lods(anchor, layout, BAND);
        let node = *selected.iter().find(|k| k.level == 1).unwrap();
        // every chunk loaded, but none of them has anything to draw
        let state = LodState {
            anchor,
            layout,
            band: BAND,
            selected: &selected,
            node_exists: &|_| false,
            node_ready: &|_| false,
            chunk_state: &|_| ChunkState::Empty,
        };
        assert!(!covered_while_loading(&state, node));
        assert!(!replaces_loaded_chunks(&state, node));
    }

    #[test]
    fn test_nodes_replacing_loaded_chunks_spawn_first() {
        let anchor = chunk(6, 4, 0);
        let selected = select_lods(anchor, layout(2), BAND);
        let exists = |_| false;
        let ready = HashMap::new();
        with_state(anchor, &selected, &exists, &ready, true, |s| {
            let mut queue = super::spawn_order(s, selected.iter().copied());
            let first = queue.pop().unwrap();
            assert!(replaces_loaded_chunks(s, first));
        });
    }

    #[test]
    fn test_selected_node_hidden_while_coarser_node_remains() {
        let anchor = chunk(0, 4, 0);
        let selected = select_lods(anchor, layout(3), BAND);
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
        let selected = select_lods(anchor, layout(3), BAND);
        let c = chunk(48, 4, 0);
        assert!(!layout(3).full_detail(c, anchor));
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
    fn loader() -> LodLoader {
        LodLoader::new(layout(4), 1024, Handle::default(), Handle::default())
    }

    fn key(level: u8, x: i64, y: i64, z: i64) -> LodKey {
        LodKey { level, coord: I64Vec3::new(x, y, z) }
    }

    #[test]
    fn test_nodes_under_finds_only_existing_nodes_within_the_area() {
        let mut lods = loader();
        let inside = [key(1, 5, 1, 6), key(2, 2, 0, 3), key(3, 1, 0, 1)];
        // the same area's neighbours and ancestors
        let outside = [key(3, 2, 0, 1), key(1, 8, 0, 8), key(4, 0, 0, 0)];
        for (i, k) in inside.iter().chain(&outside).enumerate() {
            lods.add_node(*k, Some(Entity::from_raw(i as u32)));
        }
        let found: HashSet<LodKey> = lods.nodes_under(key(3, 1, 0, 1)).into_iter().map(|(k, _)| k).collect();
        // (1, 5, 1, 6) lies in (2, 2, 0, 3), which lies in (3, 1, 0, 1)
        assert_eq!(inside.into_iter().collect::<HashSet<_>>(), found);
    }

    #[test]
    fn test_removing_nodes_empties_the_population() {
        let mut lods = loader();
        lods.add_node(key(1, 3, 2, 3), Some(Entity::from_raw(0)));
        lods.add_node(key(2, 1, 1, 1), Some(Entity::from_raw(1)));
        assert!(lods.remove_node(key(1, 3, 2, 3)).is_some());
        assert!(lods.remove_node(key(1, 3, 2, 3)).is_none());
        assert!(lods.nodes_under(key(3, 0, 0, 0)).len() == 1);
        lods.remove_node(key(2, 1, 1, 1));
        assert!(lods.populated.is_empty());
        assert!(lods.nodes_under(key(4, 0, 0, 0)).is_empty());
    }

    #[test]
    fn test_nodes_without_entities_still_exist_but_are_not_found_to_refresh() {
        let mut lods = loader();
        lods.add_node(key(1, 2, 0, 2), None);
        lods.add_node(key(2, 1, 0, 1), Some(Entity::from_raw(7)));
        assert!(lods.nodes.contains_key(&key(1, 2, 0, 2)));
        let found: Vec<LodKey> = lods.nodes_under(key(3, 0, 0, 0)).into_iter().map(|(k, _)| k).collect();
        assert_eq!(vec![key(2, 1, 0, 1)], found);
        // an entity-less node later meshing to nothing, or the reverse, counts once
        lods.add_node(key(1, 2, 0, 2), None);
        lods.remove_node(key(1, 2, 0, 2));
        lods.remove_node(key(2, 1, 0, 1));
        assert!(lods.populated.is_empty());
    }

    #[test]
    fn test_adding_a_node_twice_counts_it_once() {
        let mut lods = loader();
        lods.add_node(key(2, 0, 0, 0), Some(Entity::from_raw(0)));
        lods.add_node(key(2, 0, 0, 0), Some(Entity::from_raw(1)));
        lods.remove_node(key(2, 0, 0, 0));
        assert!(lods.populated.is_empty());
    }

    #[test]
    fn test_contains_group_matches_groups_in() {
        for area in [key(1, -3, 2, 5), key(2, 1, -1, 0), key(4, -2, 1, 3)] {
            let inside: HashSet<LodKey> = LodKey::groups_in(area).collect();
            for x in -40..40 {
                for z in -40..40 {
                    for y in [-8, 0, 7] {
                        let group = key(1, x, y, z);
                        assert_eq!(inside.contains(&group), area.contains_group(group), "{area:?} {group:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn test_full_detail_changes_only_where_the_stand_ins_change() {
        let layout = layout(3);
        for (from, to) in [((100, 5, 100), (101, 5, 100)), ((100, 5, 100), (100, 6, 100)), ((-3, 8, 70), (-3, 8, 69))] {
            let (a, b) = (chunk(from.0, from.1, from.2), chunk(to.0, to.1, to.2));
            let (_, before) = select_lods_and_stand_ins(a, layout, BAND);
            let (_, after) = select_lods_and_stand_ins(b, layout, BAND);
            let changed: HashSet<LodKey> = before.symmetric_difference(&after).copied().collect();
            for x in -80..80 {
                for z in -80..80 {
                    for y in [0, 5, 8, 15] {
                        let c = chunk(from.0 + x, y, from.2 + z);
                        if layout.full_detail(c, a) != layout.full_detail(c, b) {
                            assert!(changed.contains(&LodKey::containing(c, 1)), "{c:?}");
                        }
                    }
                }
            }
        }
    }
}
