use super::*;

#[derive(Debug, Copy, Clone)]
pub struct LodBand {
    pub min_y: i64,
    pub max_y: i64,
}

impl LodBand {
    pub(super) fn intersects(self, key: LodKey) -> bool {
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

pub(super) fn select_within(
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
