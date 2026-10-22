use bevy::{
    asset::Handle,
    ecs::entity::Entity,
    math::I64Vec3,
    platform::collections::{HashMap, HashSet},
};
use ochre_base::terrain_source::TerrainSource as _;

use super::key::LodKey;
use super::layout::LodLayout;
use super::loader::LodLoader;
use super::select::{select_lods, select_lods_and_stand_ins, LodBand};
use super::state::{ChunkState, LodState};
use crate::chunk::ChunkCoordinate;

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
    assert_eq!(
        [16.0, 64.0, 256.0, 1024.0],
        [0, 1, 2, 3].map(|l| layout.reach(l))
    );
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

#[test]
fn test_full_detail_reaches_further_with_the_levels_where_they_were() {
    let base = LodLayout::new(16, 1024, 2);
    let further = base.with_full_detail_distance(24);
    assert_eq!(24.0, further.reach(0));
    for level in 1..=base.max_level {
        assert_eq!(base.reach(level), further.reach(level));
    }
    assert_eq!(base.max_level, further.max_level);
    let anchor = chunk(0, 5, 0);
    assert!(further.full_detail(chunk(21, 5, 0), anchor));
    assert!(!base.full_detail(chunk(21, 5, 0), anchor));
    assert!(further.full_detail_reach() > base.full_detail_reach());
}

#[test]
fn test_full_detail_always_leaves_level_one_a_ring() {
    // level 1 reaches 32, so full detail stops short of it however far it's asked to reach
    let layout = LodLayout::new(16, 1024, 2).with_full_detail_distance(40);
    assert_eq!(28.0, layout.reach(0));
    assert_eq!(32.0, layout.reach(1));
    let anchor = chunk(0, 5, 0);
    let selected = select_lods(anchor, layout, BAND);
    assert!(selected.iter().any(|key| key.level == 1));
}

// Full detail out past where the levels scale from, as the shipped settings draw.
fn further_full_detail() -> LodLayout {
    LodLayout::new(16, 256, 2).with_full_detail_distance(24)
}

fn assert_exact_cover(anchor: ChunkCoordinate, max_level: u8) {
    assert_exact_cover_with(anchor, layout(max_level));
    // further out per level, and a render distance short of the last level's reach
    assert_exact_cover_with(anchor, LodLayout::new(8, 100, 4));
    assert_exact_cover_with(anchor, further_full_detail());
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
                if LodKey::containing(c, 0).distance(anchor) < layout.render_distance() {
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
    let (_, stand_ins) = super::select::select_lods_and_stand_ins(anchor, layout, BAND);
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
    for layout in [layout(3), LodLayout::new(8, 128, 4), further_full_detail()] {
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
    let chunk_state = |_| {
        if chunks_ready {
            ChunkState::Drawn
        } else {
            ChunkState::Missing
        }
    };
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
        let mut queue = super::selection::spawn_order(s, [covered, gap].into_iter());
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
    assert!(near.distance(anchor) < far.distance(anchor) * super::selection::GAP_DISTANCE_WEIGHT);
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
    use crate::chunk::CHUNK_SIZE;
    use crate::generate::{generator::generate_lod_mesh, region_store::RegionStore};
    use bevy::math::I64Vec2;
    use ochre_base::{WATER_LEVEL, WORLD_HEIGHT};
    use std::sync::Arc;

    let noise = ochre_world::worldmap::test_terrain(42);
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
    assert!(nodes
        .iter()
        .all(|&key| !super::spawn::known_empty(key, &regions)));

    let mut empty = 0;
    for &key in &nodes {
        let meshed = generate_lod_mesh(regions.clone(), key).is_some();
        if super::spawn::known_empty(key, &regions) {
            assert!(!meshed, "{key:?} was taken as empty but has a mesh");
            empty += 1;
        }
    }
    // most of each column is air, rock or open water
    assert!(
        empty > nodes.len() * 3 / 4,
        "only {empty} of {} nodes known to be empty",
        nodes.len()
    );
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
        let mut queue = super::selection::spawn_order(s, selected.iter().copied());
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
    LodLoader::new(
        layout(4),
        1024,
        Handle::default(),
        Handle::default(),
        Handle::default(),
    )
}

fn key(level: u8, x: i64, y: i64, z: i64) -> LodKey {
    LodKey {
        level,
        coord: I64Vec3::new(x, y, z),
    }
}

#[test]
fn test_nodes_under_finds_only_existing_nodes_within_the_area() {
    let mut lods = loader();
    let inside = [key(1, 5, 1, 6), key(2, 2, 0, 3), key(3, 1, 0, 1)];
    // the same area's neighbours and ancestors
    let outside = [key(3, 2, 0, 1), key(1, 8, 0, 8), key(4, 0, 0, 0)];
    for (i, k) in inside.iter().chain(&outside).enumerate() {
        lods.add_node(*k, Some(Entity::from_raw_u32(i as u32).unwrap()));
    }
    let found: HashSet<LodKey> = lods
        .nodes_under(key(3, 1, 0, 1))
        .into_iter()
        .map(|(k, _)| k)
        .collect();
    // (1, 5, 1, 6) lies in (2, 2, 0, 3), which lies in (3, 1, 0, 1)
    assert_eq!(inside.into_iter().collect::<HashSet<_>>(), found);
}

#[test]
fn test_removing_nodes_empties_the_population() {
    let mut lods = loader();
    lods.add_node(key(1, 3, 2, 3), Some(Entity::from_raw_u32(0).unwrap()));
    lods.add_node(key(2, 1, 1, 1), Some(Entity::from_raw_u32(1).unwrap()));
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
    lods.add_node(key(2, 1, 0, 1), Some(Entity::from_raw_u32(7).unwrap()));
    assert!(lods.nodes.contains_key(&key(1, 2, 0, 2)));
    let found: Vec<LodKey> = lods
        .nodes_under(key(3, 0, 0, 0))
        .into_iter()
        .map(|(k, _)| k)
        .collect();
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
    lods.add_node(key(2, 0, 0, 0), Some(Entity::from_raw_u32(0).unwrap()));
    lods.add_node(key(2, 0, 0, 0), Some(Entity::from_raw_u32(1).unwrap()));
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
                    assert_eq!(
                        inside.contains(&group),
                        area.contains_group(group),
                        "{area:?} {group:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn test_full_detail_changes_only_where_the_stand_ins_change() {
    let layout = layout(3);
    for (from, to) in [
        ((100, 5, 100), (101, 5, 100)),
        ((100, 5, 100), (100, 6, 100)),
        ((-3, 8, 70), (-3, 8, 69)),
    ] {
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
