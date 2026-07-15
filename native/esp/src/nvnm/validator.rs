//! Structural validator for NVNM payloads — the automated gate that
//! replaces "open CK and count PATHFINDING warnings" in the FO76→FO4
//! conversion harness.
//!
//! Three invariants are checked over a *set* of navmeshes that load
//! together (typically every NAVM in a plugin):
//!
//! 1. **No downfacing normals** — every triangle, viewed from above
//!    (worldspace XY plane), must be wound counter-clockwise. CK
//!    rejects clockwise triangles with "downfacing normal" and flips
//!    them at Finalize time; we want byte-stable output, so we flag
//!    instead of auto-fixing.
//!
//! 2. **Cross-mesh edge connectivity** — when two triangles in DIFFERENT
//!    meshes share a worldspace edge (after quantization using the same
//!    scheme as `target_write::quantized_edge_point`), at least one
//!    participating mesh must carry an `edge_links` row whose target
//!    form_id's low 24 bits (object_id) match the object_id of another
//!    participating mesh. The CK warning being avoided is "edge in common
//!    but not connected". We match on object_id only — ignoring the master
//!    index byte — so cross-master refs that may need rewriting still
//!    register as connected.
//!
//! 3. **Navmesh grid coverage** — every triangle index in the mesh's
//!    triangle array must appear in at least one cell of the mesh's
//!    `navmesh_grid`. The `-1` sentinel in cells means "no triangle"
//!    and is ignored; empty cells are fine; only the *union* of all
//!    cell triangle indices must cover `0..triangles.len()`.
//!
//! A potential fourth check, "Island marker reciprocity", is skipped:
//! `NvnmWaypoint` as we've modelled it carries no cross-cell reference
//! — see comment below.

use super::parser::parse_nvnm;
use super::types::{NvnmParent, NvnmPayload, NvnmVertex};
use crate::plugin_runtime::{ParsedItem, ParsedPlugin};
use std::collections::HashMap;

/// One worldspace cell is 4096 game units; vertices stored inside an
/// exterior NAVM are usually expressed in cell-local coordinates, so
/// we have to shift them by the cell origin before quantizing into a
/// worldspace key. Same rule as `target_write::quantized_edge_point`.
const WORLDSPACE_CELL_SIZE: f32 = 4096.0;
const EDGE_POINT_SCALE: f32 = 1024.0;

#[derive(Debug, Clone, PartialEq)]
pub struct ValidationError {
    pub mesh_form_key: String,
    pub kind: ValidationErrorKind,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationErrorKind {
    DownfacingNormal,
    UnconnectedSharedEdge,
    UncoveredTriangleInGrid,
    /// A grid cell does not list every triangle whose XY-AABB overlaps the
    /// cell's XY-AABB. FO4 CK's runtime expects each cell to enumerate every
    /// triangle whose AABB touches the cell so spatial queries don't miss
    /// triangles straddling cell borders. FO76's source navmesh_grid uses a
    /// single-cell bucket and was the source of the PATHFINDING "edge in
    /// common but not connected" warnings until `rebuild_nvnm_grid_fo4` in
    /// the conversion crate switched to the inclusive-AABB rule.
    GridCellMissingTriangle,
    /// Raised by `validate_plugin_navmeshes` when an NVNM payload fails to parse
    /// — not a structural finding, but surfaced as an error so callers don't
    /// need to thread parse Result types separately.
    NvnmParseError,
}

impl ValidationErrorKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ValidationErrorKind::DownfacingNormal => "downfacing_normal",
            ValidationErrorKind::UnconnectedSharedEdge => "unconnected_shared_edge",
            ValidationErrorKind::UncoveredTriangleInGrid => "uncovered_triangle_in_grid",
            ValidationErrorKind::GridCellMissingTriangle => "grid_cell_missing_triangle",
            ValidationErrorKind::NvnmParseError => "nvnm_parse_error",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ValidationReport {
    pub errors: Vec<ValidationError>,
}

impl ValidationReport {
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }
}

pub fn validate_navmesh_set(meshes: &[(String, NvnmPayload)]) -> ValidationReport {
    let mut report = ValidationReport::default();
    for (form_key, payload) in meshes {
        check_downfacing_triangles(form_key, payload, &mut report);
        check_grid_coverage(form_key, payload, &mut report);
    }
    check_cross_mesh_edge_connectivity(meshes, &mut report);
    report
}

/// Build a `OBJECTID:plugin_name` form key — same shape `nvnm_validator_form_key`
/// emits, kept GIL-free so the conversion crate can call it.
fn validator_form_key(raw: u32, masters: &[String], plugin_name: &str) -> String {
    if raw == 0 {
        return "00000000".to_string();
    }
    let index = ((raw >> 24) & 0xFF) as usize;
    let object_id = raw & 0x00FF_FFFF;
    if index == 0xFF {
        return format!("{raw:08X}");
    }
    if index < masters.len() {
        return format!("{object_id:06X}:{}", masters[index]);
    }
    if index == masters.len() {
        return format!("{object_id:06X}:{plugin_name}");
    }
    format!("{raw:08X}")
}

fn collect_payloads<'a>(
    items: &'a [ParsedItem],
    masters: &[String],
    plugin_name: &str,
    out: &mut Vec<(String, NvnmPayload)>,
    parse_failures: &mut Vec<(String, String)>,
) {
    for item in items {
        match item {
            ParsedItem::Record(record) if record.signature.as_str() == "NAVM" => {
                let Some(nvnm) = record
                    .subrecords
                    .iter()
                    .find(|sr| sr.signature.as_str() == "NVNM")
                else {
                    continue;
                };
                if nvnm.data.is_empty() {
                    continue;
                }
                let form_key = validator_form_key(record.form_id, masters, plugin_name);
                match parse_nvnm(nvnm.data.as_ref()) {
                    Ok(payload) => out.push((form_key, payload)),
                    Err(e) => parse_failures.push((form_key, e.to_string())),
                }
            }
            ParsedItem::Group(group) => {
                collect_payloads(&group.children, masters, plugin_name, out, parse_failures);
            }
            _ => {}
        }
    }
}

/// Walk every NAVM in `plugin` and run the structural validator across the
/// resulting set. Parse failures are surfaced as `ValidationError` entries
/// (kind: `NvnmParseError`) so a caller can report them alongside structural
/// findings without unwrapping a `Result`. GIL-free; safe to call from
/// conversion phase code.
pub fn validate_plugin_navmeshes(plugin: &ParsedPlugin) -> ValidationReport {
    let mut meshes: Vec<(String, NvnmPayload)> = Vec::new();
    let mut parse_failures: Vec<(String, String)> = Vec::new();
    collect_payloads(
        &plugin.root_items,
        &plugin.header.masters,
        &plugin.plugin_name,
        &mut meshes,
        &mut parse_failures,
    );
    let mut report = validate_navmesh_set(&meshes);
    for (form_key, err) in parse_failures {
        report.errors.push(ValidationError {
            mesh_form_key: form_key,
            kind: ValidationErrorKind::NvnmParseError,
            detail: err,
        });
    }
    report
}

fn check_downfacing_triangles(
    form_key: &str,
    payload: &NvnmPayload,
    report: &mut ValidationReport,
) {
    for (idx, triangle) in payload.triangles.iter().enumerate() {
        let (Some(a), Some(b), Some(c)) = (
            payload.vertices.get(triangle.vertices[0] as usize),
            payload.vertices.get(triangle.vertices[1] as usize),
            payload.vertices.get(triangle.vertices[2] as usize),
        ) else {
            continue;
        };
        let cross_z = projected_cross_z(*a, *b, *c);
        // Treat exactly-zero (degenerate or vertical triangle) as not
        // downfacing — CK's downfacing check is "normal.z < 0". A wall
        // navmesh with zero XY projection isn't a pathing surface and
        // shouldn't trip the validator.
        if cross_z < 0.0 {
            report.errors.push(ValidationError {
                mesh_form_key: form_key.to_string(),
                kind: ValidationErrorKind::DownfacingNormal,
                detail: format!(
                    "triangle {idx} cross_z={cross_z} (cw from above; flip vertex order to fix)"
                ),
            });
        }
    }
}

fn projected_cross_z(a: NvnmVertex, b: NvnmVertex, c: NvnmVertex) -> f64 {
    let abx = (b.x - a.x) as f64;
    let aby = (b.y - a.y) as f64;
    let acx = (c.x - a.x) as f64;
    let acy = (c.y - a.y) as f64;
    abx * acy - aby * acx
}

fn check_grid_coverage(form_key: &str, payload: &NvnmPayload, report: &mut ValidationReport) {
    if payload.grid.divisor == 0 {
        // No grid emitted — nothing to cover. CK will rebuild the grid
        // itself at Finalize time, so this is acceptable.
        return;
    }
    let mut covered = vec![false; payload.triangles.len()];
    for cell in &payload.grid.cells {
        for &idx in &cell.triangle_indices {
            if idx < 0 {
                continue; // -1 sentinel = "no triangle"
            }
            let i = idx as usize;
            if i < covered.len() {
                covered[i] = true;
            }
        }
    }
    let mut missing: Vec<usize> = Vec::new();
    for (i, c) in covered.iter().enumerate() {
        if !c {
            missing.push(i);
        }
    }
    if !missing.is_empty() {
        let preview = missing.iter().take(10).copied().collect::<Vec<_>>();
        report.errors.push(ValidationError {
            mesh_form_key: form_key.to_string(),
            kind: ValidationErrorKind::UncoveredTriangleInGrid,
            detail: format!(
                "{} triangle(s) absent from navmesh_grid cells (first 10: {:?})",
                missing.len(),
                preview
            ),
        });
    }

    check_grid_cell_completeness(form_key, payload, report);
}

/// For every grid cell `c`, verify that every triangle whose XY-AABB overlaps
/// `c`'s XY-AABB (inclusive on cell boundaries) is present in
/// `c.triangle_indices`. FO4 CK rejects sparse grids (FO76's single-cell
/// bucket rule) at Finalize and re-buckets — leaving the original NAVM with
/// the PATHFINDING "edge in common but not connected" warning. This check is
/// the validator-side gate that prevents `rebuild_nvnm_grid_fo4` from
/// silently regressing.
///
/// Mirrors the rebuild rule in `conversion::target_write::rebuild_nvnm_grid_fo4`:
/// f32-precision floor, inclusive `..=cmax`. Skips when divisor==0 or
/// grid_size_x/y are non-positive (degenerate grid — CK would rebuild itself).
fn check_grid_cell_completeness(
    form_key: &str,
    payload: &NvnmPayload,
    report: &mut ValidationReport,
) {
    let g = &payload.grid;
    if g.divisor == 0 || g.grid_size_x <= 0.0 || g.grid_size_y <= 0.0 {
        return;
    }
    let div = g.divisor as i32;
    let max_cell = div - 1;
    let cell_total = (g.divisor as usize).saturating_mul(g.divisor as usize);
    if g.cells.len() != cell_total {
        return; // structurally malformed — other checks fire
    }

    // Compute expected cell-set membership for each triangle.
    let mut expected: Vec<Vec<usize>> = vec![Vec::new(); g.cells.len()];
    for (tri_idx, tri) in payload.triangles.iter().enumerate() {
        let (Some(a), Some(b), Some(c)) = (
            payload.vertices.get(tri.vertices[0] as usize),
            payload.vertices.get(tri.vertices[1] as usize),
            payload.vertices.get(tri.vertices[2] as usize),
        ) else {
            continue;
        };
        let xmin = a.x.min(b.x).min(c.x);
        let xmax = a.x.max(b.x).max(c.x);
        let ymin = a.y.min(b.y).min(c.y);
        let ymax = a.y.max(b.y).max(c.y);
        let cx_min = (((xmin - g.bounds_min_x) / g.grid_size_x).floor() as i32).clamp(0, max_cell);
        let cx_max = (((xmax - g.bounds_min_x) / g.grid_size_x).floor() as i32).clamp(0, max_cell);
        let cy_min = (((ymin - g.bounds_min_y) / g.grid_size_y).floor() as i32).clamp(0, max_cell);
        let cy_max = (((ymax - g.bounds_min_y) / g.grid_size_y).floor() as i32).clamp(0, max_cell);
        for cy in cy_min..=cy_max {
            for cx in cx_min..=cx_max {
                let ci = (cy as usize) * (g.divisor as usize) + (cx as usize);
                expected[ci].push(tri_idx);
            }
        }
    }

    // Per cell, find triangles in `expected` that are missing from
    // `triangle_indices`. We compare against the i16 sentinel-filtered set;
    // -1 entries are skipped (CK pads short cells with -1 sometimes).
    let mut total_missing = 0usize;
    let mut sample: Vec<(usize, usize, usize)> = Vec::new(); // (cell, tri, missing count so far)
    for (ci, expect) in expected.iter().enumerate() {
        let cell = &g.cells[ci];
        let mut present = std::collections::HashSet::new();
        for &idx in &cell.triangle_indices {
            if idx >= 0 {
                present.insert(idx as usize);
            }
        }
        for &ti in expect {
            if !present.contains(&ti) {
                total_missing += 1;
                if sample.len() < 5 {
                    sample.push((ci, ti, total_missing));
                }
            }
        }
    }
    if total_missing > 0 {
        let preview: Vec<String> = sample
            .iter()
            .map(|(ci, ti, _)| format!("cell[{ci}] tri[{ti}]"))
            .collect();
        report.errors.push(ValidationError {
            mesh_form_key: form_key.to_string(),
            kind: ValidationErrorKind::GridCellMissingTriangle,
            detail: format!(
                "{total_missing} (cell, triangle) AABB-overlap entries missing from navmesh_grid \
                 (first {}: {})",
                preview.len(),
                preview.join(", ")
            ),
        });
    }
}

// --- Cross-mesh edge connectivity (coarse) ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct EdgePoint {
    x: i64,
    y: i64,
    z: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum ParentKey {
    Interior { cell: u32 },
    Exterior { world: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct GlobalEdgeKey {
    parent: ParentKey,
    a: EdgePoint,
    b: EdgePoint,
}

/// One edge-slot reference within a mesh; carries enough info to point back at
/// the offending mesh for error reporting.
#[derive(Debug, Clone)]
struct EdgeRef {
    mesh_idx: usize,
    triangle: usize,
    slot: u8,
}

fn parent_key(parent: NvnmParent) -> ParentKey {
    match parent {
        NvnmParent::Interior { cell } => ParentKey::Interior { cell },
        NvnmParent::Exterior { world, .. } => ParentKey::Exterior { world },
    }
}

fn exterior_cell_origin(parent: NvnmParent) -> Option<(i16, i16)> {
    match parent {
        NvnmParent::Exterior { grid_x, grid_y, .. } => Some((grid_x, grid_y)),
        NvnmParent::Interior { .. } => None,
    }
}

/// Quantize a vertex to a worldspace integer key, matching
/// `conversion::target_write::quantized_edge_point` so the writer's
/// edge-link keys agree with the validator's.
fn quantize(parent: NvnmParent, vertex: NvnmVertex) -> EdgePoint {
    let (x, y) = if let Some((cell_x, cell_y)) = exterior_cell_origin(parent) {
        if vertex_uses_worldspace_xy((cell_x, cell_y), vertex) {
            (vertex.x, vertex.y)
        } else {
            (
                vertex.x + cell_x as f32 * WORLDSPACE_CELL_SIZE,
                vertex.y + cell_y as f32 * WORLDSPACE_CELL_SIZE,
            )
        }
    } else {
        (vertex.x, vertex.y)
    };
    EdgePoint {
        x: (x * EDGE_POINT_SCALE).round() as i64,
        y: (y * EDGE_POINT_SCALE).round() as i64,
        z: (vertex.z * EDGE_POINT_SCALE).round() as i64,
    }
}

fn vertex_uses_worldspace_xy(cell: (i16, i16), vertex: NvnmVertex) -> bool {
    let (cell_x, cell_y) = cell;
    let origin_x = cell_x as f32 * WORLDSPACE_CELL_SIZE;
    let origin_y = cell_y as f32 * WORLDSPACE_CELL_SIZE;
    (cell_x != 0 && (vertex.x - origin_x).abs() < vertex.x.abs())
        || (cell_y != 0 && (vertex.y - origin_y).abs() < vertex.y.abs())
}

fn triangle_edge_slots(vertices: [u16; 3]) -> [[u16; 2]; 3] {
    [
        [vertices[0], vertices[1]],
        [vertices[1], vertices[2]],
        [vertices[2], vertices[0]],
    ]
}

/// Recover the 24-bit object_id from a form_key string of the form
/// `"OBJECTID:plugin_name"` (the canonical shape emitted by
/// `nvnm_validator_form_key`) or `"FORMID"` (8 hex chars for ESM-master 0xFF
/// or unresolvable refs). Returns the low 24 bits in both shapes.
fn parse_form_key_object_id(form_key: &str) -> Option<u32> {
    let hex = form_key.split(':').next().unwrap_or(form_key);
    let parsed = u32::from_str_radix(hex, 16).ok()?;
    Some(parsed & 0x00FF_FFFF)
}

fn normalized_edge(a: EdgePoint, b: EdgePoint) -> (EdgePoint, EdgePoint) {
    if a <= b { (a, b) } else { (b, a) }
}

fn check_cross_mesh_edge_connectivity(
    meshes: &[(String, NvnmPayload)],
    report: &mut ValidationReport,
) {
    // Map (parent, normalized-edge) -> list of (mesh_idx, triangle, slot).
    let mut edge_map: HashMap<GlobalEdgeKey, Vec<EdgeRef>> = HashMap::new();
    for (mesh_idx, (_form_key, payload)) in meshes.iter().enumerate() {
        let parent = payload.parent;
        let pkey = parent_key(parent);
        for (tri_idx, triangle) in payload.triangles.iter().enumerate() {
            let slots = triangle_edge_slots(triangle.vertices);
            for (slot, edge) in slots.iter().enumerate() {
                let (Some(va), Some(vb)) = (
                    payload.vertices.get(edge[0] as usize),
                    payload.vertices.get(edge[1] as usize),
                ) else {
                    continue;
                };
                let a = quantize(parent, *va);
                let b = quantize(parent, *vb);
                if a == b {
                    continue; // degenerate edge
                }
                let (a, b) = normalized_edge(a, b);
                let key = GlobalEdgeKey { parent: pkey, a, b };
                edge_map.entry(key).or_default().push(EdgeRef {
                    mesh_idx,
                    triangle: tri_idx,
                    slot: slot as u8,
                });
            }
        }
    }

    // Decode each mesh's form_key into a 24-bit object_id (the low bits of
    // the u32 form_id) for cross-referencing against edge_link rows. Form keys
    // are emitted by `nvnm_validator_form_key` as either "OBJECTID:plugin"
    // (object_id = first 6 hex chars, master-index resolved away) or as a
    // bare 8-hex form_id with master index 0xFF (object_id = low 24 bits).
    let mesh_object_ids: Vec<Option<u32>> = meshes
        .iter()
        .map(|(form_key, _)| parse_form_key_object_id(form_key))
        .collect();

    // For each edge that is shared across >=2 *different* meshes (same
    // worldspace), require that at least ONE row in some participating mesh's
    // edge_links array targets the object_id of one of the OTHER participating
    // meshes. The 11-byte edge_link row layout: bytes 0..4 = source-side
    // triangle/slot bookkeeping, bytes 4..8 = target navmesh form_id (LE u32),
    // bytes 8..10 = target triangle (LE i16), byte 10 = target edge slot.
    // We match on the low 24 bits (object_id) so cross-master refs that may
    // need rewriting still register as "connected".
    for (_key, refs) in &edge_map {
        if refs.len() < 2 {
            continue;
        }
        // Group by mesh; only fire if at least two DIFFERENT meshes touch
        // the same worldspace edge.
        let mut distinct_meshes: Vec<usize> = refs.iter().map(|r| r.mesh_idx).collect();
        distinct_meshes.sort_unstable();
        distinct_meshes.dedup();
        if distinct_meshes.len() < 2 {
            continue;
        }
        // Collect object_ids of OTHER participating meshes for each mesh's
        // edge_links scan: a row in mesh A targeting B's object_id satisfies
        // the A↔B shared edge.
        let participant_object_ids: Vec<u32> = distinct_meshes
            .iter()
            .filter_map(|&mi| mesh_object_ids[mi])
            .collect();
        let any_connected = distinct_meshes.iter().any(|&mi| {
            let (_, payload) = &meshes[mi];
            let self_object_id = mesh_object_ids[mi];
            payload.edge_links.iter().any(|link| {
                let target_form_id = u32::from_le_bytes(link.row[4..8].try_into().unwrap());
                let target_object_id = target_form_id & 0x00FF_FFFF;
                participant_object_ids
                    .iter()
                    .any(|&oid| oid == target_object_id && Some(oid) != self_object_id)
            })
        });
        if any_connected {
            continue;
        }
        // None of the meshes that meet at this edge have ANY edge_links
        // — flag every participating mesh once for this edge.
        for &mi in &distinct_meshes {
            let (form_key, _) = &meshes[mi];
            // Find one triangle this mesh contributed to the edge for the
            // detail message.
            let r = refs.iter().find(|r| r.mesh_idx == mi).unwrap();
            report.errors.push(ValidationError {
                mesh_form_key: form_key.clone(),
                kind: ValidationErrorKind::UnconnectedSharedEdge,
                detail: format!(
                    "triangle {} edge slot {} shared with {} other mesh(es), \
                     and no participating mesh has any edge_links rows",
                    r.triangle,
                    r.slot,
                    distinct_meshes.len() - 1
                ),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nvnm::types::{NvnmGrid, NvnmGridCell, NvnmTriangle};

    fn blank_payload(parent: NvnmParent) -> NvnmPayload {
        NvnmPayload {
            version: 15,
            flags: 0,
            parent,
            vertices: vec![],
            triangles: vec![],
            edge_links: vec![],
            door_refs: vec![],
            cover_array: vec![],
            cover_triangle_mappings: vec![],
            waypoints: vec![],
            grid: NvnmGrid::default(),
        }
    }

    fn triangle(v: [u16; 3]) -> NvnmTriangle {
        NvnmTriangle {
            vertices: v,
            links: [-1, -1, -1],
            cover_marker: [0; 9],
            flags: 0,
        }
    }

    #[test]
    fn validator_flags_downfacing_triangle() {
        let mut p = blank_payload(NvnmParent::Interior { cell: 0 });
        p.vertices = vec![
            NvnmVertex {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            NvnmVertex {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            NvnmVertex {
                x: 0.0,
                y: -1.0,
                z: 0.0,
            }, // clockwise from above
        ];
        p.triangles = vec![triangle([0, 1, 2])];
        let report = validate_navmesh_set(&[("test:1".into(), p)]);
        assert_eq!(report.errors.len(), 1);
        assert_eq!(report.errors[0].kind, ValidationErrorKind::DownfacingNormal);
    }

    #[test]
    fn validator_passes_clean_ccw_triangle() {
        let mut p = blank_payload(NvnmParent::Interior { cell: 0 });
        p.vertices = vec![
            NvnmVertex {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            NvnmVertex {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            NvnmVertex {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            }, // CCW from above
        ];
        p.triangles = vec![triangle([0, 1, 2])];
        // Grid covering both/all triangles.
        p.grid = NvnmGrid {
            divisor: 1,
            grid_size_x: 1.0,
            grid_size_y: 1.0,
            bounds_min_x: 0.0,
            bounds_min_y: 0.0,
            bounds_min_z: 0.0,
            bounds_max_x: 1.0,
            bounds_max_y: 1.0,
            bounds_max_z: 0.0,
            cells: vec![NvnmGridCell {
                triangle_indices: vec![0],
            }],
        };
        let report = validate_navmesh_set(&[("test:1".into(), p)]);
        assert!(
            report.is_ok(),
            "expected clean mesh to pass; got {:?}",
            report.errors
        );
    }

    #[test]
    fn validator_flags_uncovered_triangle_in_grid() {
        let mut p = blank_payload(NvnmParent::Interior { cell: 0 });
        p.vertices = vec![
            NvnmVertex {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            NvnmVertex {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            NvnmVertex {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
            NvnmVertex {
                x: 1.0,
                y: 1.0,
                z: 0.0,
            },
        ];
        p.triangles = vec![triangle([0, 1, 2]), triangle([1, 3, 2])];
        // Grid only references triangle 0 — triangle 1 is uncovered.
        p.grid = NvnmGrid {
            divisor: 1,
            grid_size_x: 1.0,
            grid_size_y: 1.0,
            bounds_min_x: 0.0,
            bounds_min_y: 0.0,
            bounds_min_z: 0.0,
            bounds_max_x: 1.0,
            bounds_max_y: 1.0,
            bounds_max_z: 0.0,
            cells: vec![NvnmGridCell {
                triangle_indices: vec![0, -1],
            }],
        };
        let report = validate_navmesh_set(&[("test:1".into(), p)]);
        // Triangle 1 absent from any cell triggers UncoveredTriangleInGrid
        // (Cuion-coarse coverage); it also triggers GridCellMissingTriangle
        // (per-cell completeness) because triangle 1's AABB (1,0)-(1,1)
        // overlaps cell 0 too. Filter to the kind this test targets.
        let uncovered: Vec<_> = report
            .errors
            .iter()
            .filter(|e| e.kind == ValidationErrorKind::UncoveredTriangleInGrid)
            .collect();
        assert_eq!(uncovered.len(), 1);
    }

    #[test]
    fn validator_flags_grid_cell_missing_triangle_under_aabb_rule() {
        // divisor=2, bounds (0,0)-(100,100), grid_size=(50,50).
        // Triangle AABB = (10,10)-(60,60) — overlaps all 4 cells.
        // Cell (0,0) lists triangle 0; the other 3 cells are empty.
        // The completeness check must fire 3 GridCellMissingTriangle entries
        // (one per missing cell).
        let mut p = blank_payload(NvnmParent::Interior { cell: 0 });
        p.vertices = vec![
            NvnmVertex {
                x: 10.0,
                y: 10.0,
                z: 0.0,
            },
            NvnmVertex {
                x: 60.0,
                y: 10.0,
                z: 0.0,
            },
            NvnmVertex {
                x: 10.0,
                y: 60.0,
                z: 0.0,
            },
        ];
        p.triangles = vec![triangle([0, 1, 2])];
        p.grid = NvnmGrid {
            divisor: 2,
            grid_size_x: 50.0,
            grid_size_y: 50.0,
            bounds_min_x: 0.0,
            bounds_min_y: 0.0,
            bounds_min_z: 0.0,
            bounds_max_x: 100.0,
            bounds_max_y: 100.0,
            bounds_max_z: 0.0,
            cells: vec![
                NvnmGridCell {
                    triangle_indices: vec![0],
                },
                NvnmGridCell {
                    triangle_indices: vec![],
                },
                NvnmGridCell {
                    triangle_indices: vec![],
                },
                NvnmGridCell {
                    triangle_indices: vec![],
                },
            ],
        };
        let report = validate_navmesh_set(&[("test:1".into(), p)]);
        let missing: Vec<_> = report
            .errors
            .iter()
            .filter(|e| e.kind == ValidationErrorKind::GridCellMissingTriangle)
            .collect();
        assert_eq!(
            missing.len(),
            1,
            "expected one aggregated GridCellMissingTriangle error per mesh, got {:?}",
            report.errors
        );
        // The detail string should report 3 missing (cell, tri) entries.
        assert!(
            missing[0].detail.starts_with("3 "),
            "expected 3 missing entries, got: {}",
            missing[0].detail
        );
        // The cell-0 listing should NOT be flagged (it has triangle 0).
        assert!(
            !missing[0].detail.contains("cell[0] tri[0]"),
            "cell[0] should not be in the missing list: {}",
            missing[0].detail
        );
    }

    #[test]
    fn validator_complete_grid_passes_cell_completeness() {
        // Same triangle as the previous test, but the grid lists tri 0 in all
        // 4 cells (the AABB-rule output). No GridCellMissingTriangle errors.
        let mut p = blank_payload(NvnmParent::Interior { cell: 0 });
        p.vertices = vec![
            NvnmVertex {
                x: 10.0,
                y: 10.0,
                z: 0.0,
            },
            NvnmVertex {
                x: 60.0,
                y: 10.0,
                z: 0.0,
            },
            NvnmVertex {
                x: 10.0,
                y: 60.0,
                z: 0.0,
            },
        ];
        p.triangles = vec![triangle([0, 1, 2])];
        p.grid = NvnmGrid {
            divisor: 2,
            grid_size_x: 50.0,
            grid_size_y: 50.0,
            bounds_min_x: 0.0,
            bounds_min_y: 0.0,
            bounds_min_z: 0.0,
            bounds_max_x: 100.0,
            bounds_max_y: 100.0,
            bounds_max_z: 0.0,
            cells: vec![
                NvnmGridCell {
                    triangle_indices: vec![0],
                },
                NvnmGridCell {
                    triangle_indices: vec![0],
                },
                NvnmGridCell {
                    triangle_indices: vec![0],
                },
                NvnmGridCell {
                    triangle_indices: vec![0],
                },
            ],
        };
        let report = validate_navmesh_set(&[("test:1".into(), p)]);
        let missing: Vec<_> = report
            .errors
            .iter()
            .filter(|e| e.kind == ValidationErrorKind::GridCellMissingTriangle)
            .collect();
        assert!(
            missing.is_empty(),
            "complete AABB grid should pass; got {:?}",
            report.errors
        );
    }

    #[test]
    fn validator_no_grid_skips_coverage_check() {
        let mut p = blank_payload(NvnmParent::Interior { cell: 0 });
        p.vertices = vec![
            NvnmVertex {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            NvnmVertex {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            NvnmVertex {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
        ];
        p.triangles = vec![triangle([0, 1, 2])];
        // grid.divisor == 0 = "CK will rebuild" — no coverage check.
        let report = validate_navmesh_set(&[("test:1".into(), p)]);
        assert!(report.is_ok(), "got errors {:?}", report.errors);
    }

    #[test]
    fn validator_flags_unconnected_shared_edge_across_meshes() {
        // Two interior NAVMs in DIFFERENT cells that share a worldspace
        // edge purely because their vertex coordinates agree (interior
        // navmeshes do share worldspace XY between cells in this test).
        // No edge_links on either side.
        let mut left = blank_payload(NvnmParent::Interior { cell: 1 });
        left.vertices = vec![
            NvnmVertex {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            NvnmVertex {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            NvnmVertex {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
        ];
        left.triangles = vec![triangle([0, 1, 2])];

        // Actually — interior parent_key includes cell, so two interior
        // cells DON'T share an edge in the validator's view. Use a
        // shared exterior worldspace so the parent keys match. Both
        // meshes live in world 0xABCD, in the same grid cell (so cell
        // origin shift is zero), sharing edge (0,0)-(1,0).
        let mut a = blank_payload(NvnmParent::Exterior {
            world: 0xABCD,
            grid_x: 0,
            grid_y: 0,
        });
        a.vertices = vec![
            NvnmVertex {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            NvnmVertex {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            NvnmVertex {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
        ];
        a.triangles = vec![triangle([0, 1, 2])];

        let mut b = blank_payload(NvnmParent::Exterior {
            world: 0xABCD,
            grid_x: 0,
            grid_y: 0,
        });
        b.vertices = vec![
            NvnmVertex {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            NvnmVertex {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            NvnmVertex {
                x: 0.5,
                y: -1.0,
                z: 0.0,
            }, // CCW on the OTHER side
        ];
        b.triangles = vec![triangle([0, 2, 1])]; // wound to share edge 0..1

        let _ = left; // silence unused
        let report = validate_navmesh_set(&[("test:A".into(), a), ("test:B".into(), b)]);
        let cross_edge_errors: Vec<_> = report
            .errors
            .iter()
            .filter(|e| e.kind == ValidationErrorKind::UnconnectedSharedEdge)
            .collect();
        assert!(
            !cross_edge_errors.is_empty(),
            "expected UnconnectedSharedEdge errors, got {:?}",
            report.errors
        );
    }

    /// Build an edge_link row whose target form_id resolves to `object_id`
    /// (master index 0x01, arbitrary). Triangle and slot are placeholders —
    /// the validator only inspects the form_id portion at bytes 4..8.
    fn edge_link_targeting(object_id: u32) -> crate::nvnm::types::NvnmEdgeLink {
        let form_id: u32 = 0x0100_0000 | (object_id & 0x00FF_FFFF);
        let mut row = [0u8; 11];
        row[4..8].copy_from_slice(&form_id.to_le_bytes());
        crate::nvnm::types::NvnmEdgeLink { row }
    }

    #[test]
    fn validator_shared_edge_with_edge_links_passes() {
        // A.edge_links has a row targeting B's object_id → A↔B edge is connected.
        let mut a = blank_payload(NvnmParent::Exterior {
            world: 0xABCD,
            grid_x: 0,
            grid_y: 0,
        });
        a.vertices = vec![
            NvnmVertex {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            NvnmVertex {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            NvnmVertex {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
        ];
        a.triangles = vec![triangle([0, 1, 2])];
        a.edge_links = vec![edge_link_targeting(0x000002)];

        let mut b = blank_payload(NvnmParent::Exterior {
            world: 0xABCD,
            grid_x: 0,
            grid_y: 0,
        });
        b.vertices = vec![
            NvnmVertex {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            NvnmVertex {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            NvnmVertex {
                x: 0.5,
                y: -1.0,
                z: 0.0,
            },
        ];
        b.triangles = vec![triangle([0, 2, 1])];

        let report =
            validate_navmesh_set(&[("000001:test.esp".into(), a), ("000002:test.esp".into(), b)]);
        let cross_edge_errors: Vec<_> = report
            .errors
            .iter()
            .filter(|e| e.kind == ValidationErrorKind::UnconnectedSharedEdge)
            .collect();
        assert!(
            cross_edge_errors.is_empty(),
            "shared edge with edge_links rows should be accepted; got {:?}",
            cross_edge_errors
        );
    }

    #[test]
    fn validator_flags_edge_link_targeting_wrong_mesh() {
        // A and B share a worldspace edge. A has one edge_link, but its row
        // targets mesh C (object_id 0x000099) — NOT B. B has no edge_links.
        // The validator must flag the A↔B edge as unconnected: under the
        // OLD coarse check (edge_links non-empty), A's link to C would
        // whitewash the A↔B requirement. The tighter check decodes the row.
        let mut a = blank_payload(NvnmParent::Exterior {
            world: 0xABCD,
            grid_x: 0,
            grid_y: 0,
        });
        a.vertices = vec![
            NvnmVertex {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            NvnmVertex {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            NvnmVertex {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
        ];
        a.triangles = vec![triangle([0, 1, 2])];
        a.edge_links = vec![edge_link_targeting(0x000099)]; // points at C, not B

        let mut b = blank_payload(NvnmParent::Exterior {
            world: 0xABCD,
            grid_x: 0,
            grid_y: 0,
        });
        b.vertices = vec![
            NvnmVertex {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            NvnmVertex {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            NvnmVertex {
                x: 0.5,
                y: -1.0,
                z: 0.0,
            },
        ];
        b.triangles = vec![triangle([0, 2, 1])];

        let report =
            validate_navmesh_set(&[("000001:test.esp".into(), a), ("000002:test.esp".into(), b)]);
        let cross_edge_errors: Vec<_> = report
            .errors
            .iter()
            .filter(|e| e.kind == ValidationErrorKind::UnconnectedSharedEdge)
            .collect();
        assert!(
            !cross_edge_errors.is_empty(),
            "expected UnconnectedSharedEdge: A's edge_link targets C (0x000099), \
             not B (0x000002); got errors {:?}",
            report.errors
        );
    }

    #[test]
    fn parse_form_key_object_id_handles_canonical_and_bare_forms() {
        assert_eq!(parse_form_key_object_id("000123:Fallout4.esm"), Some(0x123));
        assert_eq!(parse_form_key_object_id("FF000456"), Some(0x000456));
        assert_eq!(parse_form_key_object_id("01ABCDEF"), Some(0xABCDEF));
        assert_eq!(parse_form_key_object_id("not-hex"), None);
    }
}
