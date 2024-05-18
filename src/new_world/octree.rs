use std::sync::{Arc, RwLock};

use bevy::{math::Vec3, utils::HashMap};

pub struct OctreeNode<Data> {
    id: usize,
    centre: Vec3,
    pub size: f32,
    depth: u8,
    children: Option<[usize; 8]>,
    data: Option<Arc<Data>>,
}

impl<Data> OctreeNode<Data> {
    fn new(id: usize, centre: Vec3, size: f32, depth: u8, data: Option<Arc<Data>>) -> Self {
        Self {
            id,
            children: None,
            centre,
            size,
            depth,
            data,
        }
    }

    pub fn depth(&self) -> u8 {
        self.depth
    }

    pub fn id(&self) -> usize {
        self.id
    }

    fn is_subdivided(&self) -> bool {
        self.children.is_some()
    }

    pub fn get_data(&self) -> Option<Arc<Data>> {
        self.data.clone()
    }

    pub fn set_data(&mut self, data: Arc<Data>) {
        self.data = Some(data);
    }
}

pub struct Octree<Data> {
    arena: HashMap<usize, Arc<RwLock<OctreeNode<Data>>>>,
    current_id: usize,
    root_id: usize,
    max_depth: u8,
}

impl<Data> Octree<Data> {
    pub fn new(size: f32, max_depth: u8) -> Self {
        let root_id = 0;
        let root = Arc::new(RwLock::new(OctreeNode::new(
            root_id,
            Vec3::ZERO,
            size,
            0,
            None,
        )));

        let mut arena = HashMap::new();
        arena.insert(0, root);

        Self {
            arena,
            max_depth,
            current_id: root_id + 1,
            root_id,
        }
    }

    fn get_node(&self, id: usize) -> Arc<RwLock<OctreeNode<Data>>> {
        self.arena.get(&id).unwrap().clone()
    }

    fn get_node_centre(&self, id: usize) -> Vec3 {
        let node_ref = self.get_node(id);
        let read = node_ref.read().unwrap();
        read.centre
    }

    fn insert_node(&mut self, centre: Vec3, size: f32, depth: u8) -> usize {
        let new_id = self.current_id;
        let new_node = OctreeNode::new(new_id, centre, size, depth, None);
        self.arena
            .insert(new_node.id, Arc::new(RwLock::new(new_node)));
        self.current_id += 1;

        new_id
    }

    fn closest_child(&self, point: Vec3, parent: usize) -> usize {
        let parent_ref = self.get_node(parent);

        let parent_read = parent_ref.read().unwrap();
        if !parent_read.is_subdivided() {
            return parent;
        }

        let mut lowest_distance = f32::INFINITY;
        let mut closest_child = 0;
        for child in parent_read.children.unwrap() {
            let child_centre = self.get_node_centre(child);

            let distance = point.distance(child_centre);
            if distance < lowest_distance {
                lowest_distance = distance;
                closest_child = child;
            }
        }
        return closest_child;
    }

    pub fn subdivide(&mut self, octant: usize) {
        let node_ref = self.get_node(octant);

        let mut octree_node = node_ref.write().unwrap();
        if octree_node.is_subdivided() || octree_node.depth >= self.max_depth {
            return;
        }

        let child_size = octree_node.size / 2.0;

        // top layer
        // 0 1
        // 2 3
        // bottom layer
        // 4 5
        // 6 7
        let child_centres = [
            octree_node.centre + Vec3::new(-child_size, child_size, child_size),
            octree_node.centre + Vec3::new(child_size, child_size, child_size),
            octree_node.centre + Vec3::new(-child_size, child_size, -child_size),
            octree_node.centre + Vec3::new(child_size, child_size, -child_size),
            octree_node.centre + Vec3::new(-child_size, -child_size, child_size),
            octree_node.centre + Vec3::new(child_size, -child_size, child_size),
            octree_node.centre + Vec3::new(-child_size, -child_size, -child_size),
            octree_node.centre + Vec3::new(child_size, -child_size, -child_size),
        ];

        let mut ids = [0; 8];
        for (i, id) in ids.iter_mut().enumerate() {
            let centre = child_centres[i];
            *id = self.insert_node(centre, child_size, octree_node.depth + 1);
        }
        octree_node.children = Some(ids);
    }

    pub fn query_octant(&mut self, point: Vec3) -> Arc<RwLock<OctreeNode<Data>>> {
        let mut i = 0;

        let mut current_id = 0;
        while i < self.max_depth {
            let octree_node_ref = self.get_node(current_id);

            let subdivided = octree_node_ref.read().unwrap().is_subdivided();
            if !subdivided {
                self.subdivide(current_id)
            }

            current_id = self.closest_child(point, current_id);
            i += 1;
        }

        return self.get_node(current_id);
    }
}

#[cfg(test)]
mod tests {

    use bevy::{math::Vec3, utils::HashSet};

    use super::Octree;

    #[test]
    fn test_subdivide() {
        let mut octree = Octree::<u32>::new(16.0, 2);
        octree.subdivide(0);

        let root_ref = octree.get_node(0);
        let root = root_ref.read().unwrap();
        assert!(root.children.is_some());

        // pythagorean theorem
        let expected_distance = (3.0 * 8.0_f32.powi(2)).sqrt();

        let mut child_node_set = HashSet::new();
        for child in root.children.unwrap().into_iter() {
            let child_ref = octree.get_node(child);
            let child_node = child_ref.read().unwrap();
            assert_eq!(8.0, child_node.size);
            assert_eq!(expected_distance, root.centre.distance(child_node.centre));

            child_node_set.insert(child_node.centre.to_string());
        }

        assert_eq!(8, child_node_set.len());
    }

    #[test]
    fn test_closest_child_no_children() {
        let octree = Octree::<u32>::new(16.0, 2);
        assert_eq!(0, octree.closest_child(Vec3::ZERO, 0));
    }

    #[test]
    fn test_closest_child_subdivided_once() {
        let mut octree = Octree::<u32>::new(16.0, 2);
        octree.subdivide(octree.root_id);
        assert_eq!(
            2,
            octree.closest_child(Vec3::new(3.0, 1.0, 5.0), octree.root_id)
        );
        assert_eq!(
            7,
            octree.closest_child(Vec3::new(-3.0, -1.0, -5.0), octree.root_id)
        );
    }

    #[test]
    fn test_closest_child_subdivided_twice() {
        let mut octree = Octree::<u32>::new(16.0, 2);
        octree.subdivide(octree.root_id);
        octree.subdivide(1);
        assert_eq!(9, octree.closest_child(Vec3::new(-15.0, 15.0, 15.0), 1));
        assert_eq!(14, octree.closest_child(Vec3::new(-3.0, 1.0, 8.0), 1));
        assert_eq!(16, octree.closest_child(Vec3::new(-3.0, 1.0, 3.0), 1));
    }

    #[test]
    fn test_max_depth_not_exceeded() {
        let mut octree = Octree::<u32>::new(16.0, 0);
        octree.subdivide(octree.root_id);
        assert_eq!(1, octree.arena.len());
    }

    #[test]
    fn test_query_octant_max_depth_zero() {
        let mut octree = Octree::<u32>::new(16.0, 0);

        let octant = octree.query_octant(Vec3::new(4.0, 4.0, 4.0));
        let octant = octant.read().unwrap();
        assert_eq!(16.0, octant.size);
        assert_eq!(Vec3::new(0.0, 0.0, 0.0), octant.centre);
    }

    #[test]
    fn test_query_octant_first_subdivision() {
        let mut octree = Octree::<u32>::new(16.0, 1);
        octree.subdivide(0);

        let octant = octree.query_octant(Vec3::new(4.0, 4.0, 4.0));
        let octant = octant.read().unwrap();
        assert_eq!(8.0, octant.size);
        assert_eq!(Vec3::new(8.0, 8.0, 8.0), octant.centre);
    }
}
