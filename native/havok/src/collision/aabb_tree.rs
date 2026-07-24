/// Balanced binary AABB tree builder for hknpDynamicCompoundShapeTree.
///
/// Produces the raw node-array bytes that are embedded as the `nodes` array
/// inside `hknpDynamicCompoundShapeData`.  Internal node equivalence with
/// vanilla is NOT required — the Havok runtime rebuilds the tree on load.
/// Only per-leaf AABBs need to be correct.
///
/// Tree layout:
///   - Node 0: null sentinel (all zeros)
///   - Node 1: root
///   - Nodes 2..N: sub-tree children in pre-order
///
/// Each node is 32 bytes:
///   min hkVector4 (16 bytes): [min_x, min_y, min_z, min_w]
///   max hkVector4 (16 bytes): [max_x, max_y, max_z, max_w]
///
/// Encoding of max_w (as u32 LE):
///   Internal: lower16 = left_child_idx, upper16 = right_child_idx
///   Leaf:     lower16 = 0,              upper16 = leaf_instance_idx
///
/// Encoding of min_w (as u32 LE):
///   Active: high byte = 0x3F, low24 = parent node index.
///   Free:   first u16 of the zeroed node stores the next free node index.

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Axis-aligned bounding box for one compound sub-shape.
#[derive(Debug, Clone, Copy)]
pub struct Aabb {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

impl Aabb {
    pub fn from_vertices(verts: &[[f32; 3]]) -> Option<Self> {
        if verts.is_empty() {
            return None;
        }
        let mut mn = verts[0];
        let mut mx = verts[0];
        for v in verts.iter().skip(1) {
            mn[0] = mn[0].min(v[0]);
            mn[1] = mn[1].min(v[1]);
            mn[2] = mn[2].min(v[2]);
            mx[0] = mx[0].max(v[0]);
            mx[1] = mx[1].max(v[1]);
            mx[2] = mx[2].max(v[2]);
        }
        Some(Self { min: mn, max: mx })
    }

    fn centroid(&self, axis: usize) -> f32 {
        (self.min[axis] + self.max[axis]) * 0.5
    }

    pub(crate) fn merged(&self, other: &Aabb) -> Aabb {
        Aabb {
            min: [
                self.min[0].min(other.min[0]),
                self.min[1].min(other.min[1]),
                self.min[2].min(other.min[2]),
            ],
            max: [
                self.max[0].max(other.max[0]),
                self.max[1].max(other.max[1]),
                self.max[2].max(other.max[2]),
            ],
        }
    }

    pub(crate) fn expanded(&self, radius: f32) -> Aabb {
        Aabb {
            min: [
                self.min[0] - radius,
                self.min[1] - radius,
                self.min[2] - radius,
            ],
            max: [
                self.max[0] + radius,
                self.max[1] + radius,
                self.max[2] + radius,
            ],
        }
    }
}

// ---------------------------------------------------------------------------
// Internal tree builder
// ---------------------------------------------------------------------------

struct NodeDesc {
    aabb: Option<Aabb>,
    parent_idx: usize,
    max_w: u32,
}

const INT24_W_PREFIX: u32 = 0x3F00_0000;
pub const CODEC32_MAX_LEAVES: usize = (u16::MAX as usize - 1) / 2;

fn build_recursive(
    leaf_aabbs: &[Aabb],
    indices: &mut Vec<usize>,
    nodes: &mut Vec<NodeDesc>,
    parent_idx: usize,
) -> usize {
    let node_idx = nodes.len();

    if indices.len() == 1 {
        let leaf_idx = indices[0];
        let max_w = ((leaf_idx as u32) << 16) & 0xFFFF_FFFF;
        nodes.push(NodeDesc {
            aabb: Some(leaf_aabbs[leaf_idx]),
            parent_idx,
            max_w,
        });
        return node_idx;
    }

    // Compute combined AABB over these indices
    let mut combined = leaf_aabbs[indices[0]];
    for &i in indices.iter().skip(1) {
        combined = combined.merged(&leaf_aabbs[i]);
    }

    // Choose split axis (longest extent)
    let dx = combined.max[0] - combined.min[0];
    let dy = combined.max[1] - combined.min[1];
    let dz = combined.max[2] - combined.min[2];
    let axis = if dx >= dy && dx >= dz {
        0
    } else if dy >= dz {
        1
    } else {
        2
    };

    // Spatial median split on centroids
    let mut centroids: Vec<(f32, usize)> = indices
        .iter()
        .map(|&i| (leaf_aabbs[i].centroid(axis), i))
        .collect();
    centroids.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

    let mid = centroids.len() / 2;
    let mut left_indices: Vec<usize> = centroids[..mid].iter().map(|&(_, i)| i).collect();
    let mut right_indices: Vec<usize> = centroids[mid..].iter().map(|&(_, i)| i).collect();

    // Guard against degenerate splits
    if left_indices.is_empty() || right_indices.is_empty() {
        let all: Vec<usize> = indices.iter().copied().collect();
        let mid2 = all.len() / 2;
        left_indices = all[..mid2].to_vec();
        right_indices = all[mid2..].to_vec();
    }

    // Reserve this node slot before recursing so children get higher indices
    nodes.push(NodeDesc {
        aabb: Some(combined),
        parent_idx,
        max_w: 0, // filled in after children
    });

    let left_child = build_recursive(leaf_aabbs, &mut left_indices, nodes, node_idx);
    let right_child = build_recursive(leaf_aabbs, &mut right_indices, nodes, node_idx);

    let max_w = (left_child as u32 & 0xFFFF) | ((right_child as u32 & 0xFFFF) << 16);
    nodes[node_idx].max_w = max_w;

    node_idx
}

// ---------------------------------------------------------------------------
// Public functions
// ---------------------------------------------------------------------------

/// Build the raw node-array bytes for `hknpDynamicCompoundShapeTree`.
///
/// Returns serialized nodes (N × 32 bytes) where N includes node 0 (null),
/// active tree nodes, and one free-list node.
/// The array starts with node 0 (null sentinel), followed by node 1 (root) etc.
pub fn build_aabb_tree_nodes(leaf_aabbs: &[Aabb]) -> Vec<u8> {
    // Node 0 = null sentinel
    let mut nodes: Vec<NodeDesc> = vec![NodeDesc {
        aabb: None,
        parent_idx: 0,
        max_w: 0,
    }];

    if !leaf_aabbs.is_empty() {
        let mut indices: Vec<usize> = (0..leaf_aabbs.len()).collect();
        build_recursive(leaf_aabbs, &mut indices, &mut nodes, 0);
    }

    // Vanilla FO4 dynamic compound trees keep one spare free-list node at
    // m_firstFree. Havok's dynamic tree storage treats index 0 as invalid, so a
    // non-zero firstFree must point at a serialized node, not just past the end.
    if !leaf_aabbs.is_empty() {
        nodes.push(NodeDesc {
            aabb: None,
            parent_idx: 0,
            max_w: 0,
        });
    }

    let mut out = Vec::with_capacity(nodes.len() * 32);
    for node in &nodes {
        match &node.aabb {
            None => out.extend_from_slice(&[0u8; 32]),
            Some(aabb) => {
                let min_w = INT24_W_PREFIX | (node.parent_idx as u32);
                out.extend_from_slice(&aabb.min[0].to_le_bytes());
                out.extend_from_slice(&aabb.min[1].to_le_bytes());
                out.extend_from_slice(&aabb.min[2].to_le_bytes());
                out.extend_from_slice(&min_w.to_le_bytes());
                out.extend_from_slice(&aabb.max[0].to_le_bytes());
                out.extend_from_slice(&aabb.max[1].to_le_bytes());
                out.extend_from_slice(&aabb.max[2].to_le_bytes());
                out.extend_from_slice(&node.max_w.to_le_bytes());
            }
        }
    }
    out
}

/// Returns an upper bound on the node count (including null sentinel at index 0).
///
/// Actual count is `build_aabb_tree_nodes(...).len() / 32`. This returns
/// `2*n + 1`: null node, `2*n - 1` active tree nodes, and one free-list node.
pub fn max_node_count(n_leaves: usize) -> usize {
    if n_leaves == 0 { 1 } else { 2 * n_leaves + 1 }
}

pub fn codec32_counts_fit(n_leaves: usize) -> bool {
    n_leaves <= CODEC32_MAX_LEAVES
}

/// Map each leaf index to its assigned tree node index.
/// Used to encode the tree-node reference inside hknpShapeInstance.transform.row3.w.
pub fn leaf_node_indices(leaf_aabbs: &[Aabb]) -> Vec<usize> {
    let mut nodes: Vec<NodeDesc> = vec![NodeDesc {
        aabb: None,
        parent_idx: 0,
        max_w: 0,
    }];
    let mut leaf_to_node: Vec<usize> = vec![0; leaf_aabbs.len()];

    if leaf_aabbs.is_empty() {
        return leaf_to_node;
    }

    let mut indices: Vec<usize> = (0..leaf_aabbs.len()).collect();
    _build_with_tracking(leaf_aabbs, &mut indices, &mut nodes, &mut leaf_to_node, 0);
    leaf_to_node
}

fn _build_with_tracking(
    leaf_aabbs: &[Aabb],
    indices: &mut Vec<usize>,
    nodes: &mut Vec<NodeDesc>,
    leaf_to_node: &mut Vec<usize>,
    parent_idx: usize,
) -> usize {
    let node_idx = nodes.len();

    if indices.len() == 1 {
        let leaf_idx = indices[0];
        let max_w = ((leaf_idx as u32) << 16) & 0xFFFF_FFFF;
        nodes.push(NodeDesc {
            aabb: Some(leaf_aabbs[leaf_idx]),
            parent_idx,
            max_w,
        });
        leaf_to_node[leaf_idx] = node_idx;
        return node_idx;
    }

    let mut combined = leaf_aabbs[indices[0]];
    for &i in indices.iter().skip(1) {
        combined = combined.merged(&leaf_aabbs[i]);
    }

    let dx = combined.max[0] - combined.min[0];
    let dy = combined.max[1] - combined.min[1];
    let dz = combined.max[2] - combined.min[2];
    let axis = if dx >= dy && dx >= dz {
        0
    } else if dy >= dz {
        1
    } else {
        2
    };

    let mut centroids: Vec<(f32, usize)> = indices
        .iter()
        .map(|&i| (leaf_aabbs[i].centroid(axis), i))
        .collect();
    centroids.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

    let mid = centroids.len() / 2;
    let mut left_indices: Vec<usize> = centroids[..mid].iter().map(|&(_, i)| i).collect();
    let mut right_indices: Vec<usize> = centroids[mid..].iter().map(|&(_, i)| i).collect();

    if left_indices.is_empty() || right_indices.is_empty() {
        let all: Vec<usize> = indices.iter().copied().collect();
        let mid2 = all.len() / 2;
        left_indices = all[..mid2].to_vec();
        right_indices = all[mid2..].to_vec();
    }

    nodes.push(NodeDesc {
        aabb: Some(combined),
        parent_idx,
        max_w: 0,
    });

    let left_child =
        _build_with_tracking(leaf_aabbs, &mut left_indices, nodes, leaf_to_node, node_idx);
    let right_child = _build_with_tracking(
        leaf_aabbs,
        &mut right_indices,
        nodes,
        leaf_to_node,
        node_idx,
    );

    let max_w = (left_child as u32 & 0xFFFF) | ((right_child as u32 & 0xFFFF) << 16);
    nodes[node_idx].max_w = max_w;

    node_idx
}

#[cfg(test)]
mod tests {
    use super::*;

    fn min_w(bytes: &[u8], node_idx: usize) -> u32 {
        let off = node_idx * 32 + 12;
        u32::from_le_bytes(bytes[off..off + 4].try_into().unwrap())
    }

    fn max_w(bytes: &[u8], node_idx: usize) -> u32 {
        let off = node_idx * 32 + 28;
        u32::from_le_bytes(bytes[off..off + 4].try_into().unwrap())
    }

    fn free_next(bytes: &[u8], node_idx: usize) -> u16 {
        let off = node_idx * 32;
        u16::from_le_bytes(bytes[off..off + 2].try_into().unwrap())
    }

    #[test]
    fn single_leaf_produces_null_leaf_and_free_node() {
        let aabbs = vec![Aabb {
            min: [0.0, 0.0, 0.0],
            max: [1.0, 1.0, 1.0],
        }];
        let bytes = build_aabb_tree_nodes(&aabbs);
        // node 0 (null) + node 1 (leaf) + node 2 (free) = 3 nodes × 32 bytes
        assert_eq!(bytes.len(), 96, "single leaf: 3 nodes × 32 bytes");
        // Node 0 is all zeros
        assert!(bytes[..32].iter().all(|&b| b == 0));
        // Node 1 min.xyz = (0,0,0), max.xyz = (1,1,1)
        let min_x = f32::from_le_bytes(bytes[32..36].try_into().unwrap());
        let max_x = f32::from_le_bytes(bytes[48..52].try_into().unwrap());
        assert!((min_x - 0.0).abs() < 1e-6);
        assert!((max_x - 1.0).abs() < 1e-6);
    }

    #[test]
    fn two_leaves_encode_structure_and_free_list() {
        let aabbs = vec![
            Aabb {
                min: [0.0, 0.0, 0.0],
                max: [1.0, 1.0, 1.0],
            },
            Aabb {
                min: [2.0, 0.0, 0.0],
                max: [3.0, 1.0, 1.0],
            },
        ];
        let bytes = build_aabb_tree_nodes(&aabbs);
        // null + root(internal) + leaf0 + leaf1 + free = 5 nodes
        assert_eq!(bytes.len(), 5 * 32, "two leaves: 5 nodes × 32 bytes");

        // Node 1 is internal: max_w lower16 = 2 (left), upper16 = 3 (right)
        let max_w_1 = max_w(&bytes, 1);
        let left = max_w_1 & 0xFFFF;
        let right = (max_w_1 >> 16) & 0xFFFF;
        assert_eq!(left, 2, "left child should be node 2");
        assert_eq!(right, 3, "right child should be node 3");

        assert_eq!(min_w(&bytes, 1), 0x3F00_0000, "root parent is 0");
        assert_eq!(min_w(&bytes, 2), 0x3F00_0001, "left leaf parent is root");
        assert_eq!(min_w(&bytes, 3), 0x3F00_0001, "right leaf parent is root");

        assert_eq!(max_w(&bytes, 2) & 0xFFFF, 0, "leaf child slot is 0");
        assert_eq!(max_w(&bytes, 2) >> 16, 0, "leaf data is instance 0");
        assert_eq!(max_w(&bytes, 3) & 0xFFFF, 0, "leaf child slot is 0");
        assert_eq!(max_w(&bytes, 3) >> 16, 1, "leaf data is instance 1");

        assert_eq!(free_next(&bytes, 4), 0, "free node next-free starts at 0");
    }

    #[test]
    fn leaf_to_node_mapping_correct() {
        let aabbs = vec![
            Aabb {
                min: [0.0, 0.0, 0.0],
                max: [1.0, 1.0, 1.0],
            },
            Aabb {
                min: [2.0, 0.0, 0.0],
                max: [3.0, 1.0, 1.0],
            },
        ];
        let mapping = leaf_node_indices(&aabbs);
        assert_eq!(mapping.len(), 2);
        // Both leaves should map to nodes 2 and 3
        let set: std::collections::HashSet<usize> = mapping.iter().copied().collect();
        assert!(set.contains(&2) && set.contains(&3));
    }

    #[test]
    fn empty_leaf_list_produces_null_only() {
        let bytes = build_aabb_tree_nodes(&[]);
        assert_eq!(bytes.len(), 32, "empty: just null node (32 bytes)");
        assert!(bytes.iter().all(|&b| b == 0));
    }

    #[test]
    fn max_node_count_matches_serialized_tree_capacity() {
        for n in [0, 1, 2, 6, 9] {
            let aabbs = vec![
                Aabb {
                    min: [0.0, 0.0, 0.0],
                    max: [1.0, 1.0, 1.0],
                };
                n
            ];
            assert_eq!(build_aabb_tree_nodes(&aabbs).len() / 32, max_node_count(n));
        }
    }

    #[test]
    fn codec32_count_guard_rejects_unrepresentable_tree() {
        assert!(codec32_counts_fit(CODEC32_MAX_LEAVES));
        assert!(!codec32_counts_fit(CODEC32_MAX_LEAVES + 1));
    }
}
