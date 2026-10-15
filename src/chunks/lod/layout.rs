use super::key::LodKey;
use crate::chunks::chunk::ChunkCoordinate;
use crate::chunks::generate::columns::COARSE_CELL;

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
