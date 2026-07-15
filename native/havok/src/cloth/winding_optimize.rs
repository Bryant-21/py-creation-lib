// Cloth pre-bake validation utilities.
//
// Two utilities meant to surface authoring bugs that otherwise manifest as
// runtime cloth weirdness:
//
// 1. `check_winding` — walk triangles, detect winding inconsistencies via
//    shared edges. Two triangles sharing an edge (a,b) should orient it in
//    opposite directions (one a→b, the other b→a). Same direction = one
//    triangle is flipped relative to its neighbor.
//
// 2. `find_redundant_links` — detect overlapping link constraints (same
//    unordered particle pair). Returned as a per-pair group so the caller
//    can decide to merge or warn.
//
// The Lint-style return shape is compatible with the existing
// `cloth/validate.rs` `ValidationResult`; callers can fold these issues in.

use std::collections::{BTreeMap, HashMap};

use crate::cloth::setup::mesh::{SetupMesh, Tri};
use crate::cloth::validate::{LintIssue, Severity};

/// Walk every triangle pair sharing an edge; emit a warning for each pair
/// whose orientations agree (i.e., the shared edge is traversed in the
/// same direction by both).
pub fn check_winding(mesh: &SetupMesh) -> Vec<LintIssue> {
    let mut issues = Vec::new();
    if mesh.triangles.is_empty() {
        return issues;
    }

    // Map each *directed* edge → list of triangle indices that produced it.
    // The same directed edge appearing in two different triangles means
    // those triangles wind the same direction → flip mismatch.
    let mut directed: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    let mut undirected: HashMap<(u32, u32), Vec<usize>> = HashMap::new();

    for (ti, tri) in mesh.triangles.iter().enumerate() {
        let edges = directed_edges(tri);
        for &(a, b) in &edges {
            directed.entry((a, b)).or_default().push(ti);
            let key = if a < b { (a, b) } else { (b, a) };
            undirected.entry(key).or_default().push(ti);
        }
    }

    // For each undirected edge that is shared by exactly two triangles,
    // verify they direct the edge oppositely.
    for ((a, b), tris) in undirected.iter() {
        if tris.len() != 2 {
            continue;
        }
        let same_dir_count = directed.get(&(*a, *b)).map(|v| v.len()).unwrap_or(0)
            + directed.get(&(*b, *a)).map(|v| v.len()).unwrap_or(0);
        // Shared edge with two triangles → expect 2 directed entries
        // *split* across (a,b) and (b,a). If either side has 2, both
        // triangles wind the same way.
        let ab = directed.get(&(*a, *b)).map(|v| v.len()).unwrap_or(0);
        let ba = directed.get(&(*b, *a)).map(|v| v.len()).unwrap_or(0);
        let _ = same_dir_count;
        if ab == 2 || ba == 2 {
            issues.push(LintIssue {
                severity: Severity::Warning,
                code: "WINDING_FLIP".to_string(),
                message: format!(
                    "Triangles {} and {} share edge ({a}, {b}) with the same orientation \
                     — one is wound backwards",
                    tris[0], tris[1],
                ),
            });
        }
    }

    issues
}

/// A simple link constraint with rest length, suitable for redundancy
/// detection. The detector groups by unordered particle pair (`a`, `b`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinkLite {
    pub particle_a: u16,
    pub particle_b: u16,
    pub rest_length: f32,
}

/// A group of redundant links — two or more links sharing the same
/// unordered (a, b) pair.
#[derive(Debug, Clone, PartialEq)]
pub struct RedundantLinkGroup {
    pub particle_a: u16,
    pub particle_b: u16,
    /// Indices of duplicate links into the original `links` slice.
    pub link_indices: Vec<usize>,
    /// Whether the rest lengths overlap within `tolerance`.
    pub rest_lengths_overlap: bool,
    /// Range of rest lengths observed (min, max).
    pub rest_length_range: (f32, f32),
}

/// Find redundant links. `tolerance` controls the rest-length-overlap
/// flag — two links count as overlapping if `|len_a - len_b| <= tolerance`.
pub fn find_redundant_links(links: &[LinkLite], tolerance: f32) -> Vec<RedundantLinkGroup> {
    let mut by_pair: BTreeMap<(u16, u16), Vec<usize>> = BTreeMap::new();
    for (i, l) in links.iter().enumerate() {
        let key = if l.particle_a < l.particle_b {
            (l.particle_a, l.particle_b)
        } else {
            (l.particle_b, l.particle_a)
        };
        by_pair.entry(key).or_default().push(i);
    }

    let mut out = Vec::new();
    for ((a, b), indices) in by_pair {
        if indices.len() < 2 {
            continue;
        }
        let lens: Vec<f32> = indices.iter().map(|&i| links[i].rest_length).collect();
        let mut min_l = lens[0];
        let mut max_l = lens[0];
        for &v in &lens[1..] {
            if v < min_l {
                min_l = v;
            }
            if v > max_l {
                max_l = v;
            }
        }
        out.push(RedundantLinkGroup {
            particle_a: a,
            particle_b: b,
            link_indices: indices,
            rest_lengths_overlap: (max_l - min_l) <= tolerance,
            rest_length_range: (min_l, max_l),
        });
    }
    out
}

/// Convert a `RedundantLinkGroup` list into LintIssues for inclusion in a
/// `ValidationResult`. Overlapping groups are warnings (likely safe to
/// merge); non-overlapping ones are info (rest lengths disagree, may be
/// intentional layered constraints).
pub fn redundant_links_to_issues(groups: &[RedundantLinkGroup]) -> Vec<LintIssue> {
    let mut out = Vec::new();
    for g in groups {
        let (lo, hi) = g.rest_length_range;
        if g.rest_lengths_overlap {
            out.push(LintIssue {
                severity: Severity::Warning,
                code: "REDUNDANT_LINK_MERGEABLE".to_string(),
                message: format!(
                    "{} duplicate link(s) on pair ({}, {}) with overlapping rest length \
                     [{lo:.4}, {hi:.4}] — consider merging",
                    g.link_indices.len(),
                    g.particle_a,
                    g.particle_b,
                ),
            });
        } else {
            out.push(LintIssue {
                severity: Severity::Info,
                code: "REDUNDANT_LINK_DIVERGENT".to_string(),
                message: format!(
                    "{} duplicate link(s) on pair ({}, {}) with divergent rest length \
                     [{lo:.4}, {hi:.4}]",
                    g.link_indices.len(),
                    g.particle_a,
                    g.particle_b,
                ),
            });
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn directed_edges(tri: &Tri) -> [(u32, u32); 3] {
    [(tri[0], tri[1]), (tri[1], tri[2]), (tri[2], tri[0])]
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn two_tri_quad_consistent() -> SetupMesh {
        // Two CCW triangles sharing edge (1, 2) — one directs 1→2, other 2→1.
        // Verts: 0=(0,0), 1=(1,0), 2=(1,1), 3=(0,1)
        let mut m = SetupMesh::default();
        m.positions = vec![
            [0.0, 0.0, 0.0, 1.0],
            [1.0, 0.0, 0.0, 1.0],
            [1.0, 1.0, 0.0, 1.0],
            [0.0, 1.0, 0.0, 1.0],
        ];
        m.triangles = vec![[0, 1, 2], [0, 2, 3]];
        m
    }

    fn two_tri_quad_flipped() -> SetupMesh {
        // Same geometry but second triangle is wound backwards. Both
        // triangles emit the directed edge 2→0 on shared edge (0,2) → mismatch.
        let mut m = SetupMesh::default();
        m.positions = vec![
            [0.0, 0.0, 0.0, 1.0],
            [1.0, 0.0, 0.0, 1.0],
            [1.0, 1.0, 0.0, 1.0],
            [0.0, 1.0, 0.0, 1.0],
        ];
        m.triangles = vec![[0, 1, 2], [3, 2, 0]];
        m
    }

    #[test]
    fn check_winding_clean_quad_has_no_issues() {
        let mesh = two_tri_quad_consistent();
        let issues = check_winding(&mesh);
        assert!(issues.is_empty(), "unexpected issues: {issues:?}");
    }

    #[test]
    fn check_winding_flagged_when_neighbor_is_flipped() {
        let mesh = two_tri_quad_flipped();
        let issues = check_winding(&mesh);
        assert!(
            issues.iter().any(|i| i.code == "WINDING_FLIP"),
            "expected WINDING_FLIP, got {issues:?}"
        );
    }

    #[test]
    fn find_redundant_links_groups_duplicates() {
        let links = vec![
            LinkLite {
                particle_a: 0,
                particle_b: 1,
                rest_length: 1.0,
            },
            LinkLite {
                particle_a: 1,
                particle_b: 0,
                rest_length: 1.001,
            },
            LinkLite {
                particle_a: 2,
                particle_b: 3,
                rest_length: 1.0,
            },
        ];
        let groups = find_redundant_links(&links, 1e-2);
        assert_eq!(groups.len(), 1);
        let g = &groups[0];
        assert_eq!(g.particle_a, 0);
        assert_eq!(g.particle_b, 1);
        assert_eq!(g.link_indices.len(), 2);
        assert!(g.rest_lengths_overlap);
    }

    #[test]
    fn find_redundant_links_flags_divergent_rest_length() {
        let links = vec![
            LinkLite {
                particle_a: 0,
                particle_b: 1,
                rest_length: 1.0,
            },
            LinkLite {
                particle_a: 0,
                particle_b: 1,
                rest_length: 5.0,
            },
        ];
        let groups = find_redundant_links(&links, 1e-3);
        assert_eq!(groups.len(), 1);
        assert!(!groups[0].rest_lengths_overlap);
        assert_eq!(groups[0].rest_length_range, (1.0, 5.0));
    }

    #[test]
    fn redundant_links_to_issues_severity_split() {
        let groups = vec![
            RedundantLinkGroup {
                particle_a: 0,
                particle_b: 1,
                link_indices: vec![0, 1],
                rest_lengths_overlap: true,
                rest_length_range: (1.0, 1.001),
            },
            RedundantLinkGroup {
                particle_a: 2,
                particle_b: 3,
                link_indices: vec![2, 3],
                rest_lengths_overlap: false,
                rest_length_range: (1.0, 5.0),
            },
        ];
        let issues = redundant_links_to_issues(&groups);
        assert_eq!(issues.len(), 2);
        assert_eq!(issues[0].severity, Severity::Warning);
        assert_eq!(issues[0].code, "REDUNDANT_LINK_MERGEABLE");
        assert_eq!(issues[1].severity, Severity::Info);
        assert_eq!(issues[1].code, "REDUNDANT_LINK_DIVERGENT");
    }

    #[test]
    fn empty_inputs_produce_empty_outputs() {
        let mesh = SetupMesh::default();
        assert!(check_winding(&mesh).is_empty());
        assert!(find_redundant_links(&[], 1e-3).is_empty());
    }
}
