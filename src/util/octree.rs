use std::sync::Arc;

use bevy::math::UVec3;

pub type NodeId = u32;

const ROOT: NodeId = 0;

struct Node<Data> {
    parent: Option<NodeId>,
    first_child: Option<NodeId>,
    data: Option<Arc<Data>>,
}

impl<Data> Node<Data> {
    fn empty(parent: Option<NodeId>) -> Self {
        Self {
            parent,
            first_child: None,
            data: None,
        }
    }

    fn is_empty_leaf(&self) -> bool {
        self.first_child.is_none() && self.data.is_none()
    }
}

pub struct Octree<Data> {
    nodes: Vec<Node<Data>>,
    // first slot of each freed 8-node sibling block, reused before growing the arena
    free_blocks: Vec<NodeId>,
    max_depth: u8,
}

impl<Data> Octree<Data> {
    pub fn new(max_depth: u8) -> Self {
        assert!(
            max_depth < 32,
            "octree max_depth {max_depth} doesn't fit u32 cells"
        );
        Self {
            nodes: vec![Node::empty(None)],
            free_blocks: Vec::new(),
            max_depth,
        }
    }

    /// True when the tree holds no data and has collapsed back to a bare root.
    pub fn is_empty(&self) -> bool {
        self.node(ROOT).is_empty_leaf()
    }

    pub fn get(&self, id: NodeId) -> Option<&Arc<Data>> {
        self.node(id).data.as_ref()
    }

    pub fn insert(&mut self, pos: UVec3, data: Arc<Data>) -> NodeId {
        let side = 1u32 << self.max_depth;
        assert!(
            pos.max_element() < side,
            "octree cell {pos:?} is outside the tree's bounds (side {side})"
        );

        let mut id = ROOT;
        for depth in 0..self.max_depth {
            let first_child = match self.node(id).first_child {
                Some(first_child) => first_child,
                None => self.subdivide(id),
            };
            id = first_child + self.child_index(pos, depth);
        }
        self.node_mut(id).data = Some(data);
        id
    }

    pub fn remove(&mut self, id: NodeId) -> Option<Arc<Data>> {
        let data = self.node_mut(id).data.take();
        self.collapse_upward(id);
        data
    }

    fn child_index(&self, pos: UVec3, depth: u8) -> NodeId {
        let bit = self.max_depth - 1 - depth;
        ((pos.x >> bit) & 1) | ((pos.y >> bit) & 1) << 1 | ((pos.z >> bit) & 1) << 2
    }

    fn subdivide(&mut self, id: NodeId) -> NodeId {
        let first_child = match self.free_blocks.pop() {
            Some(first_child) => {
                for child in first_child..first_child + 8 {
                    *self.node_mut(child) = Node::empty(Some(id));
                }
                first_child
            }
            None => {
                let first_child = self.nodes.len() as NodeId;
                self.nodes.extend((0..8).map(|_| Node::empty(Some(id))));
                first_child
            }
        };
        self.node_mut(id).first_child = Some(first_child);
        first_child
    }

    fn collapse_upward(&mut self, mut id: NodeId) {
        while let Some(parent) = self.node(id).parent {
            let first_child = self.node(parent).first_child.unwrap();
            let siblings = &self.nodes[first_child as usize..first_child as usize + 8];
            if !siblings.iter().all(Node::is_empty_leaf) {
                return;
            }

            self.node_mut(parent).first_child = None;
            self.free_blocks.push(first_child);
            id = parent;
        }
    }

    fn node(&self, id: NodeId) -> &Node<Data> {
        &self.nodes[id as usize]
    }

    fn node_mut(&mut self, id: NodeId) -> &mut Node<Data> {
        &mut self.nodes[id as usize]
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bevy::math::UVec3;

    use super::{Octree, ROOT};

    fn live_nodes<Data>(octree: &Octree<Data>) -> usize {
        octree.nodes.len() - octree.free_blocks.len() * 8
    }

    #[test]
    fn test_insert_get() {
        let mut octree = Octree::new(4);
        let id = octree.insert(UVec3::new(3, 9, 15), Arc::new(7));
        assert_eq!(Some(&Arc::new(7)), octree.get(id));
    }

    #[test]
    fn test_insert_subdivides_down_to_leaf() {
        let mut octree = Octree::new(4);
        octree.insert(UVec3::new(3, 9, 15), Arc::new(7));
        // root + one full block of 8 children per level
        assert_eq!(1 + 4 * 8, octree.nodes.len());
    }

    #[test]
    fn test_insert_distinct_cells_get_distinct_leaves() {
        let mut octree = Octree::new(2);
        let mut ids = Vec::new();
        for x in 0..4 {
            for y in 0..4 {
                for z in 0..4 {
                    ids.push(octree.insert(UVec3::new(x, y, z), Arc::new((x, y, z))));
                }
            }
        }
        ids.sort();
        ids.dedup();
        assert_eq!(64, ids.len());
    }

    #[test]
    fn test_insert_same_cell_replaces_data() {
        let mut octree = Octree::new(3);
        let a = octree.insert(UVec3::new(1, 2, 3), Arc::new(1));
        let b = octree.insert(UVec3::new(1, 2, 3), Arc::new(2));
        assert_eq!(a, b);
        assert_eq!(Some(&Arc::new(2)), octree.get(a));
    }

    #[test]
    fn test_child_index_splits_on_each_axis() {
        let octree = Octree::<u32>::new(1);
        assert_eq!(0, octree.child_index(UVec3::new(0, 0, 0), 0));
        assert_eq!(1, octree.child_index(UVec3::new(1, 0, 0), 0));
        assert_eq!(2, octree.child_index(UVec3::new(0, 1, 0), 0));
        assert_eq!(4, octree.child_index(UVec3::new(0, 0, 1), 0));
        assert_eq!(7, octree.child_index(UVec3::new(1, 1, 1), 0));
    }

    #[test]
    fn test_insert_max_depth_zero_uses_root() {
        let mut octree = Octree::new(0);
        assert_eq!(ROOT, octree.insert(UVec3::ZERO, Arc::new(1)));
        assert_eq!(1, octree.nodes.len());
    }

    #[test]
    #[should_panic]
    fn test_insert_out_of_bounds_panics() {
        let mut octree = Octree::new(2);
        octree.insert(UVec3::new(4, 0, 0), Arc::new(1));
    }

    #[test]
    fn test_subdivide_sets_parent_pointer() {
        let mut octree = Octree::<u32>::new(1);
        let first_child = octree.subdivide(ROOT);
        for child in first_child..first_child + 8 {
            assert_eq!(Some(ROOT), octree.node(child).parent);
        }
    }

    #[test]
    fn test_remove_collapses_empty_parent() {
        let mut octree = Octree::new(1);
        let leaf = octree.insert(UVec3::new(1, 1, 1), Arc::new(1));

        assert_eq!(Some(Arc::new(1)), octree.remove(leaf));

        assert!(octree.is_empty());
        assert_eq!(1, live_nodes(&octree));
    }

    #[test]
    fn test_remove_does_not_collapse_if_sibling_has_data() {
        let mut octree = Octree::new(1);
        let a = octree.insert(UVec3::new(1, 1, 1), Arc::new(1));
        let b = octree.insert(UVec3::new(0, 1, 1), Arc::new(2));

        octree.remove(a);

        assert!(!octree.is_empty());
        assert_eq!(Some(&Arc::new(2)), octree.get(b));
    }

    #[test]
    fn test_remove_collapses_multiple_levels_upward() {
        let mut octree = Octree::new(3);
        let leaf = octree.insert(UVec3::new(5, 2, 7), Arc::new(1));

        octree.remove(leaf);

        // only the root should remain - every level of subdivision collapses away
        assert!(octree.is_empty());
        assert_eq!(1, live_nodes(&octree));
    }

    #[test]
    fn test_freed_blocks_are_reused() {
        let mut octree = Octree::new(3);
        let leaf = octree.insert(UVec3::new(5, 2, 7), Arc::new(1));
        octree.remove(leaf);
        let arena_len = octree.nodes.len();

        let leaf = octree.insert(UVec3::new(0, 6, 1), Arc::new(2));

        assert_eq!(arena_len, octree.nodes.len());
        assert!(octree.free_blocks.is_empty());
        assert_eq!(Some(&Arc::new(2)), octree.get(leaf));
    }

    #[test]
    fn test_remove_keeps_other_ids_valid() {
        let mut octree = Octree::new(4);
        let a = octree.insert(UVec3::new(0, 0, 0), Arc::new(1));
        let b = octree.insert(UVec3::new(15, 15, 15), Arc::new(2));

        octree.remove(a);
        // reuses a's freed blocks, must not disturb b
        let c = octree.insert(UVec3::new(1, 0, 0), Arc::new(3));

        assert_eq!(Some(&Arc::new(2)), octree.get(b));
        assert_eq!(Some(&Arc::new(3)), octree.get(c));
    }
}
