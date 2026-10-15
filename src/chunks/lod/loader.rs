use super::*;

#[derive(Component)]
pub struct LodNode {
    pub(super) key: LodKey,
}

impl LodNode {
    pub fn key(&self) -> LodKey {
        self.key
    }
}

#[derive(Component)]
pub struct GenerateLodMesh {
    pub(super) task: Task<Option<ChunkMeshes>>,
}

#[derive(Resource)]
pub struct LodLoader {
    // as set, drawing out to the set render distance
    set_layout: LodLayout,
    pub(super) layout: LodLayout,
    pub(super) band: LodBand,
    pub(super) anchor: Option<ChunkCoordinate>,
    pub(super) selected: Arc<HashSet<LodKey>>,
    // level-1 nodes over the full-detail area, drawn while its chunks aren't ready
    pub(super) stand_ins: Arc<HashSet<LodKey>>,
    // the walk choosing nodes for a new anchor, which runs off the main thread
    pub(super) selecting: Option<Task<LodSelection>>,
    // counts layout changes, which make a walk under the old one out of date
    pub(super) generation: u32,
    // each node's entity, or none for one with nothing to draw, which needs no entity at all
    pub(super) nodes: HashMap<LodKey, Option<Entity>>,
    // how many nodes exist at or below each key, so areas are walked only where nodes are
    pub(super) populated: HashMap<LodKey, u32>,
    // farthest first, so popping yields the nearest
    pub(super) to_spawn: Vec<LodKey>,
    pub(super) retiring: Vec<LodKey>,
    // areas whose nodes and chunks need their visibility rechecked
    pub(super) dirty_areas: Vec<LodKey>,
    pub(super) refresh_all: bool,
    pub(super) material: Handle<ChunkMaterial>,
    pub(super) water_material: Handle<WaterMaterial>,
    pub(super) solid_water_material: Handle<WaterMaterial>,
}

// Levels `LodLoader::populated` counts up through, however far the layout reaches.
pub(super) const POPULATED_LEVELS: u8 = 16;

impl LodLoader {
    pub(super) fn add_node(&mut self, key: LodKey, entity: Option<Entity>) {
        if self.nodes.insert(key, entity).is_none() {
            for ancestor in key.self_and_ancestors(POPULATED_LEVELS) {
                *self.populated.entry(ancestor).or_insert(0) += 1;
            }
        }
    }

    pub(super) fn remove_node(&mut self, key: LodKey) -> Option<Option<Entity>> {
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
    pub(super) fn nodes_under(&self, area: LodKey) -> Vec<(LodKey, Entity)> {
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

    /// The camera chunk the selection was last applied for.
    pub fn anchor_chunk(&self) -> Option<ChunkCoordinate> {
        self.anchor
    }

    /// How the node at `key` is held, for diagnostics.
    pub fn node_kind(&self, key: LodKey) -> &'static str {
        match self.nodes.get(&key) {
            None => "none",
            Some(None) => "empty slot",
            Some(Some(_)) => "entity",
        }
    }

    /// Whether `key` is among the nodes chosen to be drawn.
    pub fn is_retiring(&self, key: LodKey) -> bool {
        self.retiring.contains(&key)
    }

    pub fn is_selected(&self, key: LodKey) -> bool {
        self.selected.contains(&key)
    }

    /// Whether `key` is a stand-in for the full-detail chunks under it.
    pub fn is_stand_in(&self, key: LodKey) -> bool {
        self.stand_ins.contains(&key)
    }

    /// Whether the node at `key` exists with nothing to draw.
    pub fn node_is_empty(&self, key: LodKey) -> bool {
        self.nodes.get(&key).is_some_and(Option::is_none)
    }

    /// Draws terrain out to `render_distance` chunks, or the set render distance if that's
    /// further, selecting nodes afresh for it.
    pub fn set_render_distance(&mut self, render_distance: f32) {
        let layout = self
            .set_layout
            .with_render_distance(render_distance.max(self.set_layout.render_distance()));
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
    pub fn chunk_covered(
        &self,
        chunk: ChunkCoordinate,
        pending: &Query<(), With<GenerateLodMesh>>,
    ) -> bool {
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
        solid_water_material: Handle<WaterMaterial>,
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
            solid_water_material,
        }
    }
}

pub(super) fn lod_transform(key: LodKey) -> Transform {
    let cell = key.cell_size() as f32;
    Transform::from_translation(key.min_block().as_vec3() + Vec3::splat(0.5 * cell - 0.5))
        .with_scale(Vec3::splat(cell))
}

pub(super) fn lod_aabb() -> Aabb {
    let size = CHUNK_SIZE as f32;
    Aabb::from_min_max(Vec3::splat(-0.5), Vec3::splat(size - 0.5))
}
