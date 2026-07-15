/// Find-closest-position and vertex-weight transfer.
///
/// Builds a simple KD-tree over a source mesh to enable O(log n) nearest-
/// neighbor queries.  Used to transfer skinning weights from a high-resolution
/// bind-pose mesh to a decimated or re-meshed target.
use crate::geometry::skinning::SkinWeight;

// ---------------------------------------------------------------------------
// KD-tree
// ---------------------------------------------------------------------------

struct KdNode {
    point: [f32; 3],
    index: usize,
    left: Option<Box<KdNode>>,
    right: Option<Box<KdNode>>,
}

impl KdNode {
    fn build(mut points: Vec<(usize, [f32; 3])>, depth: usize) -> Option<Box<KdNode>> {
        if points.is_empty() {
            return None;
        }
        let axis = depth % 3;
        points.sort_unstable_by(|a, b| {
            a.1[axis]
                .partial_cmp(&b.1[axis])
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let mid = points.len() / 2;
        let (idx, pt) = points[mid];
        let right_pts = points.split_off(mid + 1);
        let left_pts: Vec<_> = points.into_iter().take(mid).collect();
        Some(Box::new(KdNode {
            point: pt,
            index: idx,
            left: KdNode::build(left_pts, depth + 1),
            right: KdNode::build(right_pts, depth + 1),
        }))
    }

    fn nearest(&self, query: [f32; 3], depth: usize, best: &mut (f32, usize)) {
        let d = dist_sq(self.point, query);
        if d < best.0 {
            *best = (d, self.index);
        }
        let axis = depth % 3;
        let diff = query[axis] - self.point[axis];
        let (near, far) = if diff <= 0.0 {
            (self.left.as_deref(), self.right.as_deref())
        } else {
            (self.right.as_deref(), self.left.as_deref())
        };
        if let Some(n) = near {
            n.nearest(query, depth + 1, best);
        }
        if diff * diff < best.0 {
            if let Some(f) = far {
                f.nearest(query, depth + 1, best);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A KD-tree over a set of 3D positions for nearest-neighbor queries.
pub struct KdTree {
    root: Option<Box<KdNode>>,
}

impl KdTree {
    /// Build from a slice of positions.
    pub fn build(points: &[[f32; 3]]) -> Self {
        let indexed: Vec<(usize, [f32; 3])> = points.iter().copied().enumerate().collect();
        KdTree {
            root: KdNode::build(indexed, 0),
        }
    }

    /// Return the index of the closest point to `query`, or `None` if empty.
    pub fn nearest(&self, query: [f32; 3]) -> Option<usize> {
        self.root.as_ref().map(|root| {
            let mut best = (f32::INFINITY, 0);
            root.nearest(query, 0, &mut best);
            best.1
        })
    }

    /// Return the `k` nearest indices (unsorted).
    pub fn k_nearest(&self, query: [f32; 3], k: usize) -> Vec<usize> {
        if k == 0 {
            return vec![];
        }
        // Simple approach: collect all candidates within a growing radius
        // using a max-heap of (dist, idx) bounded to k entries.
        let mut heap: Vec<(ordered_float::OrderedFloat, usize)> = Vec::with_capacity(k + 1);
        if let Some(ref root) = self.root {
            k_nearest_impl(root, query, k, 0, &mut heap);
        }
        heap.into_iter().map(|(_, i)| i).collect()
    }
}

fn k_nearest_impl(
    node: &KdNode,
    query: [f32; 3],
    k: usize,
    depth: usize,
    heap: &mut Vec<(ordered_float::OrderedFloat, usize)>,
) {
    let d = dist_sq(node.point, query);
    let of = ordered_float::OrderedFloat(d);

    let worst = heap
        .iter()
        .map(|(d, _)| *d)
        .max()
        .unwrap_or(ordered_float::OrderedFloat(0.0));
    if heap.len() < k || of < worst {
        heap.push((of, node.index));
        if heap.len() > k {
            // Remove the worst
            let worst_pos = heap
                .iter()
                .enumerate()
                .max_by_key(|(_, (d, _))| *d)
                .map(|(i, _)| i)
                .unwrap();
            heap.swap_remove(worst_pos);
        }
    }

    let axis = depth % 3;
    let diff = query[axis] - node.point[axis];
    let (near, far) = if diff <= 0.0 {
        (node.left.as_deref(), node.right.as_deref())
    } else {
        (node.right.as_deref(), node.left.as_deref())
    };
    if let Some(n) = near {
        k_nearest_impl(n, query, k, depth + 1, heap);
    }
    let current_worst = heap.iter().map(|(d, _)| d.0).fold(0.0f32, f32::max);
    if heap.len() < k || diff * diff < current_worst {
        if let Some(f) = far {
            k_nearest_impl(f, query, k, depth + 1, heap);
        }
    }
}

// ---------------------------------------------------------------------------
// Weight transfer
// ---------------------------------------------------------------------------

/// Transfer skinning weights from a source mesh to a target mesh.
///
/// For each target vertex, finds the nearest source vertex and copies its
/// weights.  Weights are copied as-is (already normalized on the source).
///
/// Returns one `Vec<SkinWeight>` per target vertex.
pub fn transfer_weights(
    source_verts: &[[f32; 3]],
    source_weights: &[Vec<SkinWeight>],
    target_verts: &[[f32; 3]],
) -> Vec<Vec<SkinWeight>> {
    assert_eq!(
        source_verts.len(),
        source_weights.len(),
        "source_verts and source_weights must have equal length"
    );
    let tree = KdTree::build(source_verts);
    target_verts
        .iter()
        .map(|&tv| {
            tree.nearest(tv)
                .map(|si| source_weights[si].clone())
                .unwrap_or_default()
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

#[inline]
fn dist_sq(a: [f32; 3], b: [f32; 3]) -> f32 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    let dz = a[2] - b[2];
    dx * dx + dy * dy + dz * dz
}

// ---------------------------------------------------------------------------
// ordered_float shim (avoid external dep — implement just enough for k_nearest)
// ---------------------------------------------------------------------------

mod ordered_float {
    #[derive(Clone, Copy, PartialEq)]
    pub struct OrderedFloat(pub f32);
    impl Eq for OrderedFloat {}
    impl PartialOrd for OrderedFloat {
        fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
            Some(self.cmp(other))
        }
    }
    impl Ord for OrderedFloat {
        fn cmp(&self, other: &Self) -> std::cmp::Ordering {
            self.0
                .partial_cmp(&other.0)
                .unwrap_or(std::cmp::Ordering::Equal)
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_finds_correct_point() {
        let pts = vec![[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [5.0, 0.0, 0.0]];
        let tree = KdTree::build(&pts);
        assert_eq!(tree.nearest([0.1, 0.0, 0.0]), Some(0));
        assert_eq!(tree.nearest([0.9, 0.0, 0.0]), Some(1));
        assert_eq!(tree.nearest([4.8, 0.0, 0.0]), Some(2));
    }

    #[test]
    fn transfer_weights_copies_nearest() {
        let src = vec![[0.0f32, 0.0, 0.0], [10.0, 0.0, 0.0]];
        let src_weights = vec![
            vec![SkinWeight {
                bone: 0,
                weight: 1.0,
            }],
            vec![SkinWeight {
                bone: 1,
                weight: 1.0,
            }],
        ];
        let tgt = vec![[0.1f32, 0.0, 0.0], [9.9, 0.0, 0.0]];
        let tw = transfer_weights(&src, &src_weights, &tgt);
        assert_eq!(tw[0][0].bone, 0);
        assert_eq!(tw[1][0].bone, 1);
    }

    #[test]
    fn empty_tree_returns_none() {
        let tree = KdTree::build(&[]);
        assert!(tree.nearest([0.0, 0.0, 0.0]).is_none());
    }
}
