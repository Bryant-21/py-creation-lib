//! Havok 2019 tagged binary (TAG0) collision data — parser and builder.
//!
//! Direct port of the TAG0 half of `py_creation_lib/python/creation_lib/havok/collision_payload.py` (lines 1–768).
use super::tagged_writer::{PatchEntry, TaggedBlobBuilder, TaggedItem};
use crate::error::{HavokError, HavokResult};

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

/// Parsed ITEM record from the INDX section.
#[derive(Debug, Clone, PartialEq)]
pub struct ItemRecord {
    pub index: usize,
    /// 0x10 = single object, 0x20 = array
    pub kind: u32,
    pub type_idx: u32,
    pub data_offset: usize,
    pub count: usize,
}

/// Parsed PTCH record from the INDX section.
#[derive(Debug, Clone, PartialEq)]
pub struct PatchRecord {
    pub src: u32,
    pub flag: u32,
    pub target: u32,
}

/// All collision data extracted from a Havok tagged binary blob.
#[derive(Debug, Clone)]
pub struct ParsedCollision {
    pub vertices: Vec<[f32; 3]>,
    pub planes: Vec<[f32; 4]>,
    pub faces: Vec<(u16, u8, u8)>,
    pub edges: Vec<(u16, u8, u8)>,
    pub indices: Vec<u8>,
    pub vertex_edges: Vec<u32>,
    pub friction: f32,
    pub restitution: f32,
    pub layer: u32,
    pub mass: f32,
    pub motion_type: u32,
    /// Fixed (single-object) items: ITEM index → raw DATA bytes
    pub fixed_objects: Vec<(usize, Vec<u8>)>,
    pub items: Vec<ItemRecord>,
    pub patches: Vec<PatchRecord>,
    /// Full TYPE container bytes (verbatim from blob)
    pub type_section: Vec<u8>,
    /// Raw trailing bytes after PTCH records (format artifacts)
    pub ptch_trailing: Vec<u8>,
}

// ---------------------------------------------------------------------------
// Alignment helper
// ---------------------------------------------------------------------------

fn align16(size: usize) -> usize {
    (size + 15) & !15
}

// ---------------------------------------------------------------------------
// Section walker
// ---------------------------------------------------------------------------

/// Walk TAG0 children, return map of tag_name → (content_start, content_end).
///
/// Each section has an 8-byte header:
///   4 bytes: (type_byte << 24 | size) big-endian
///   4 bytes: tag name (ASCII)
///
/// type_byte 0x40 = leaf (content follows header)
/// type_byte 0x00 = container (children follow header)
fn walk_sections(
    data: &[u8],
    offset: usize,
    end: usize,
    out: &mut std::collections::HashMap<String, (usize, usize)>,
) -> HavokResult<()> {
    let mut pos = offset;
    while pos < end {
        if pos + 8 > end {
            break;
        }
        let raw = read_u32_be(data, pos)?;
        let type_byte = ((raw >> 24) & 0xFF) as u8;
        let size = (raw & 0x00FF_FFFF) as usize;
        if size < 8 || pos + size > end {
            return Err(HavokError::InvalidInput(format!(
                "invalid TAG0 section at offset {pos}: size={size}"
            )));
        }
        let tag = std::str::from_utf8(&data[pos + 4..pos + 8])
            .map_err(|_| {
                HavokError::InvalidInput("TAG0 section tag is not valid UTF-8".to_string())
            })?
            .to_string();
        let is_leaf = (type_byte & 0x40) != 0;
        if is_leaf {
            let content_start = pos + 8;
            let content_end = pos + size;
            out.insert(tag, (content_start, content_end));
        } else {
            // container — recurse into children
            walk_sections(data, pos + 8, pos + size, out)?;
        }
        pos += size;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Parser internals
// ---------------------------------------------------------------------------

fn parse_items_raw(data: &[u8], item_start: usize, item_end: usize) -> Vec<ItemRecord> {
    let mut items = Vec::new();
    let mut i = 0usize;
    let mut record_idx = 0usize;
    while item_start + i + 12 <= item_end {
        let packed =
            u32::from_le_bytes(data[item_start + i..item_start + i + 4].try_into().unwrap());
        let data_off = u32::from_le_bytes(
            data[item_start + i + 4..item_start + i + 8]
                .try_into()
                .unwrap(),
        ) as usize;
        let count = u32::from_le_bytes(
            data[item_start + i + 8..item_start + i + 12]
                .try_into()
                .unwrap(),
        ) as usize;
        let kind = (packed >> 24) & 0xFF;
        let type_idx = packed & 0x00FF_FFFF;
        if record_idx != 0 {
            // skip the null entry at index 0
            items.push(ItemRecord {
                index: record_idx,
                kind,
                type_idx,
                data_offset: data_off,
                count,
            });
        }
        i += 12;
        record_idx += 1;
    }
    items
}

fn parse_patches_raw(data: &[u8], ptch_start: usize, ptch_end: usize) -> Vec<PatchRecord> {
    let num_full = (ptch_end - ptch_start) / 12;
    (0..num_full)
        .map(|i| {
            let off = ptch_start + i * 12;
            PatchRecord {
                src: u32::from_le_bytes(data[off..off + 4].try_into().unwrap()),
                flag: u32::from_le_bytes(data[off + 4..off + 8].try_into().unwrap()),
                target: u32::from_le_bytes(data[off + 8..off + 12].try_into().unwrap()),
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Public parser
// ---------------------------------------------------------------------------

/// Parse a Havok 2019 TAG0 blob and extract collision geometry.
///
/// Port of `parse_tagged_collision` in `py_creation_lib/python/creation_lib/havok/collision_payload.py`.
pub fn parse_tagged_collision(blob: &[u8]) -> HavokResult<ParsedCollision> {
    use std::collections::HashMap;

    // Walk sections from the start of the blob
    let mut sections: HashMap<String, (usize, usize)> = HashMap::new();
    walk_sections(blob, 0, blob.len(), &mut sections)?;

    let (data_start, data_end) = sections.get("DATA").copied().ok_or_else(|| {
        HavokError::InvalidInput("TAG0 collision payload missing DATA section".to_string())
    })?;
    let data_size = data_end - data_start;

    // Capture TYPE section container verbatim.
    // Python logic: find TPTR first child, then span to last TYPE child's end.
    let type_section = extract_type_section(blob, &sections);

    let (item_start, item_end) = sections.get("ITEM").copied().ok_or_else(|| {
        HavokError::InvalidInput("TAG0 collision payload missing ITEM section".to_string())
    })?;
    let (ptch_start, ptch_end) = sections.get("PTCH").copied().ok_or_else(|| {
        HavokError::InvalidInput("TAG0 collision payload missing PTCH section".to_string())
    })?;

    let items = parse_items_raw(blob, item_start, item_end);
    let patches = parse_patches_raw(blob, ptch_start, ptch_end);

    // Trailing bytes after PTCH records
    let num_patch_bytes = patches.len() * 12;
    let ptch_trailing = blob[ptch_start + num_patch_bytes..ptch_end].to_vec();

    // Sort items by data_offset for size computation
    let mut sorted_items: Vec<&ItemRecord> = items.iter().collect();
    sorted_items.sort_by_key(|it| it.data_offset);

    // Compute data range for each item
    let mut item_ranges: std::collections::HashMap<usize, (usize, usize)> = HashMap::new();
    for (i, item) in sorted_items.iter().enumerate() {
        let start = item.data_offset;
        let end = sorted_items
            .get(i + 1)
            .map(|next| next.data_offset)
            .unwrap_or(data_size);
        item_ranges.insert(item.index, (start, end));
    }

    // Classify and extract data
    let mut vertices: Vec<[f32; 3]> = Vec::new();
    let mut planes: Vec<[f32; 4]> = Vec::new();
    let mut vec4_arrays: Vec<(&ItemRecord, usize, usize)> = Vec::new();
    let mut four_byte_arrays: Vec<(&ItemRecord, usize, usize)> = Vec::new();
    let mut indices: Vec<u8> = Vec::new();
    let mut fixed_objects: Vec<(usize, Vec<u8>)> = Vec::new();

    for item in &items {
        let (start, end) = item_ranges[&item.index];
        let total_bytes = end - start;
        let abs_start = data_start + start;
        let abs_end = data_start + end;

        if item.kind == 0x10 {
            fixed_objects.push((item.index, blob[abs_start..abs_end].to_vec()));
            continue;
        }

        // Array items (kind == 0x20)
        if item.count == 0 {
            continue;
        }

        let elem_size = total_bytes as f64 / item.count as f64;

        if item.count > 1 && (elem_size - 12.0).abs() < 0.5 {
            vertices = (0..item.count)
                .map(|j| {
                    let off = abs_start + j * 12;
                    [
                        read_f32_le(blob, off),
                        read_f32_le(blob, off + 4),
                        read_f32_le(blob, off + 8),
                    ]
                })
                .collect();
        } else if item.count > 1 && (elem_size - 16.0).abs() < 0.5 {
            vec4_arrays.push((item, abs_start, abs_end));
        } else if item.count > 1 && (elem_size - 4.0).abs() < 0.5 {
            four_byte_arrays.push((item, abs_start, abs_end));
        } else if item.count > 1 && total_bytes >= item.count && elem_size < 2.0 {
            indices = blob[abs_start..abs_start + item.count].to_vec();
        } else if item.count == 1 {
            fixed_objects.push((item.index, blob[abs_start..abs_end].to_vec()));
        }
    }

    if vertices.is_empty() {
        if let Some((item, abs_start, _)) = vec4_arrays.first().copied() {
            vertices = (0..item.count)
                .map(|j| {
                    let off = abs_start + j * 16;
                    [
                        read_f32_le(blob, off),
                        read_f32_le(blob, off + 4),
                        read_f32_le(blob, off + 8),
                    ]
                })
                .collect();
        }
    }
    if planes.is_empty() {
        let plane_source = vec4_arrays.get(1).or_else(|| vec4_arrays.first()).copied();
        if let Some((item, abs_start, _)) = plane_source {
            planes = (0..item.count)
                .map(|j| {
                    let off = abs_start + j * 16;
                    [
                        read_f32_le(blob, off),
                        read_f32_le(blob, off + 4),
                        read_f32_le(blob, off + 8),
                        read_f32_le(blob, off + 12),
                    ]
                })
                .collect();
        }
    }

    // Disambiguate 4-byte arrays using vertex/plane counts
    let n_verts = vertices.len();
    let n_planes = planes.len();

    let mut faces: Vec<(u16, u8, u8)> = Vec::new();
    let mut edges: Vec<(u16, u8, u8)> = Vec::new();
    let mut vertex_edges: Vec<u32> = Vec::new();

    for (item, abs_start, _abs_end) in &four_byte_arrays {
        if item.count == n_planes {
            faces = (0..item.count)
                .map(|j| {
                    let off = abs_start + j * 4;
                    let first_idx = u16::from_le_bytes(blob[off..off + 2].try_into().unwrap());
                    let num_idx = blob[off + 2];
                    let min_half = blob[off + 3];
                    (first_idx, num_idx, min_half)
                })
                .collect();
        } else if item.count == n_verts {
            vertex_edges = (0..item.count)
                .map(|j| {
                    let off = abs_start + j * 4;
                    let face_idx =
                        u16::from_le_bytes(blob[off..off + 2].try_into().unwrap()) as u32;
                    let edge_idx = blob[off + 2] as u32;
                    face_idx | (edge_idx << 16)
                })
                .collect();
        } else {
            edges = (0..item.count)
                .map(|j| {
                    let off = abs_start + j * 4;
                    let face_idx = u16::from_le_bytes(blob[off..off + 2].try_into().unwrap());
                    let edge_idx = blob[off + 2];
                    let pad = blob[off + 3];
                    (face_idx, edge_idx, pad)
                })
                .collect();
        }
    }

    // Sort fixed_objects by index for deterministic order
    fixed_objects.sort_by_key(|(idx, _)| *idx);

    Ok(ParsedCollision {
        vertices,
        planes,
        faces,
        edges,
        indices,
        vertex_edges,
        friction: 0.0,
        restitution: 0.0,
        layer: 0,
        mass: 0.0,
        motion_type: 0,
        fixed_objects,
        items,
        patches,
        type_section,
        ptch_trailing,
    })
}

// ---------------------------------------------------------------------------
// TYPE section extraction
// ---------------------------------------------------------------------------

/// Extract the full TYPE container bytes from the blob.
///
/// Python logic: TPTR first child header starts 8 bytes before TPTR content_start,
/// and the TYPE container header starts 8 bytes before that (the TPTR header is
/// within a TYPE container). We capture blob[type_container_start..last_child_end].
fn extract_type_section(
    blob: &[u8],
    sections: &std::collections::HashMap<String, (usize, usize)>,
) -> Vec<u8> {
    // Python: the TYPE container wraps TPTR, TST1, TNA1, FST1, TBDY, THSH, TPAD
    // The content offsets point inside the blob. The TYPE container header sits
    // 8 bytes before the first child header, which sits 8 bytes before TPTR content_start.
    let type_children = ["TPTR", "TST1", "TNA1", "FST1", "TBDY", "THSH", "TPAD"];

    let tptr = match sections.get("TPTR") {
        Some(&v) => v,
        None => return Vec::new(),
    };

    // TPTR leaf header starts at tptr.0 - 8 (content_start - header_size)
    let tptr_leaf_header = tptr.0.saturating_sub(8);
    // TYPE container header starts 8 bytes before TPTR leaf header
    let type_container_start = tptr_leaf_header.saturating_sub(8);

    // Find the end of all TYPE children
    let last_end = type_children
        .iter()
        .filter_map(|name| sections.get(*name))
        .map(|&(_, end)| end)
        .max()
        .unwrap_or(tptr.1);

    if last_end > blob.len() || type_container_start >= blob.len() {
        return Vec::new();
    }

    blob[type_container_start..last_end].to_vec()
}

// ---------------------------------------------------------------------------
// Variable array serialization helpers
// ---------------------------------------------------------------------------

fn serialize_vertices(verts: &[[f32; 3]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(verts.len() * 12);
    for v in verts {
        out.extend_from_slice(&v[0].to_le_bytes());
        out.extend_from_slice(&v[1].to_le_bytes());
        out.extend_from_slice(&v[2].to_le_bytes());
    }
    out
}

fn serialize_planes(planes: &[[f32; 4]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(planes.len() * 16);
    for p in planes {
        out.extend_from_slice(&p[0].to_le_bytes());
        out.extend_from_slice(&p[1].to_le_bytes());
        out.extend_from_slice(&p[2].to_le_bytes());
        out.extend_from_slice(&p[3].to_le_bytes());
    }
    out
}

fn serialize_faces(faces: &[(u16, u8, u8)]) -> Vec<u8> {
    let mut out = Vec::with_capacity(faces.len() * 4);
    for &(first_idx, num_idx, min_half) in faces {
        out.extend_from_slice(&first_idx.to_le_bytes());
        out.push(num_idx);
        out.push(min_half);
    }
    out
}

fn serialize_edges(edges: &[(u16, u8, u8)]) -> Vec<u8> {
    let mut out = Vec::with_capacity(edges.len() * 4);
    for &(face_idx, edge_idx, pad) in edges {
        out.extend_from_slice(&face_idx.to_le_bytes());
        out.push(edge_idx);
        out.push(pad);
    }
    out
}

fn serialize_vertex_edges(vertex_edges: &[u32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(vertex_edges.len() * 4);
    for &ve in vertex_edges {
        let face_idx = (ve & 0xFFFF) as u16;
        let edge_idx = ((ve >> 16) & 0xFF) as u8;
        out.extend_from_slice(&face_idx.to_le_bytes());
        out.push(edge_idx);
        out.push(0u8);
    }
    out
}

/// Identify which variable array type an ITEM represents.
///
/// Port of `_identify_var_array` in `py_creation_lib/python/creation_lib/havok/collision_payload.py`.
fn identify_var_array(
    item: &ItemRecord,
    total_bytes: usize,
    n_verts: usize,
    n_planes: usize,
) -> Option<&'static str> {
    if item.count == 0 {
        return None;
    }
    let elem_size = total_bytes as f64 / item.count as f64;

    if item.count > 1 && (elem_size - 12.0).abs() < 0.5 {
        Some("vertices")
    } else if item.count > 1 && (elem_size - 16.0).abs() < 0.5 {
        Some("planes")
    } else if item.count > 1 && (elem_size - 4.0).abs() < 0.5 {
        if item.count == n_planes {
            Some("faces")
        } else if item.count == n_verts {
            Some("vertex_edges")
        } else {
            Some("edges")
        }
    } else if item.count > 1 && elem_size < 2.0 {
        Some("indices")
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Builder — round-trip from ParsedCollision
// ---------------------------------------------------------------------------

/// Reconstruct exact blob from ParsedCollision data.
///
/// Port of `_rebuild_from_parsed` in `py_creation_lib/python/creation_lib/havok/collision_payload.py`.
pub fn rebuild_tag0_collision(parsed: &ParsedCollision) -> HavokResult<Vec<u8>> {
    // Sort items by offset to compute sizes
    let mut sorted_items: Vec<&ItemRecord> = parsed.items.iter().collect();
    sorted_items.sort_by_key(|it| it.data_offset);

    if sorted_items.is_empty() {
        return Err(HavokError::InvalidInput(
            "no ITEM records to rebuild".to_string(),
        ));
    }

    let n_verts = parsed.vertices.len();
    let n_planes = parsed.planes.len();

    // Build a lookup for fixed_objects
    let fixed_map: std::collections::HashMap<usize, &Vec<u8>> = parsed
        .fixed_objects
        .iter()
        .map(|(idx, bytes)| (*idx, bytes))
        .collect();

    // Compute allocated size for each item
    let mut item_alloc: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    for i in 0..sorted_items.len() {
        let item = sorted_items[i];
        let alloc = if i + 1 < sorted_items.len() {
            sorted_items[i + 1].data_offset - item.data_offset
        } else {
            // Last item: compute from its content
            if let Some(raw) = fixed_map.get(&item.index) {
                raw.len()
            } else {
                // Determine from variable array type
                let var_sizes = var_array_sizes(parsed);
                let arr_type = identify_var_array(item, 0, n_verts, n_planes);
                *var_sizes.get(arr_type.unwrap_or("")).unwrap_or(&0)
            }
        };
        item_alloc.insert(item.index, alloc);
    }

    // Compute total DATA size
    let total_data_size = sorted_items
        .iter()
        .map(|item| item.data_offset + item_alloc[&item.index])
        .max()
        .unwrap_or(0);

    // Build DATA byte array
    let mut data = vec![0u8; total_data_size];

    for item in &sorted_items {
        let off = item.data_offset;
        let alloc = item_alloc[&item.index];

        if let Some(raw) = fixed_map.get(&item.index) {
            let copy_len = raw.len().min(alloc);
            data[off..off + copy_len].copy_from_slice(&raw[..copy_len]);
            continue;
        }

        // Variable array — identify and serialize
        let arr_type = identify_var_array(item, alloc, n_verts, n_planes);
        let serialized = match arr_type {
            Some("vertices") => serialize_vertices(&parsed.vertices),
            Some("planes") => serialize_planes(&parsed.planes),
            Some("faces") => serialize_faces(&parsed.faces),
            Some("indices") => parsed.indices.clone(),
            Some("edges") => serialize_edges(&parsed.edges),
            Some("vertex_edges") => serialize_vertex_edges(&parsed.vertex_edges),
            _ => Vec::new(),
        };

        let copy_len = serialized.len().min(alloc);
        if copy_len > 0 {
            data[off..off + copy_len].copy_from_slice(&serialized[..copy_len]);
        }
        // Rest of allocated region stays zero (padding)
    }

    // Build ITEM entries (include null entry at index 0)
    let mut tagged_items: Vec<TaggedItem> = vec![TaggedItem {
        kind: 0,
        type_idx: 0,
        data_offset: 0,
        count: 0,
    }];
    for item in &parsed.items {
        tagged_items.push(TaggedItem {
            kind: item.kind,
            type_idx: item.type_idx,
            data_offset: item.data_offset as u32,
            count: item.count as u32,
        });
    }

    // Build PTCH entries
    let tagged_patches: Vec<PatchEntry> = parsed
        .patches
        .iter()
        .map(|p| PatchEntry {
            src: p.src,
            flag: p.flag,
            target: p.target,
        })
        .collect();

    // Assemble
    let mut builder = TaggedBlobBuilder::new("20190200");
    builder.set_type_section(parsed.type_section.clone());
    builder.set_data(data);
    builder.set_items(tagged_items);
    builder.set_patches(tagged_patches);
    if !parsed.ptch_trailing.is_empty() {
        builder.set_ptch_trailer(parsed.ptch_trailing.clone());
    }
    Ok(builder.build())
}

fn var_array_sizes(parsed: &ParsedCollision) -> std::collections::HashMap<&'static str, usize> {
    let n_indices_padded = align16(parsed.indices.len());
    let mut m = std::collections::HashMap::new();
    m.insert("vertices", parsed.vertices.len() * 12);
    m.insert("planes", parsed.planes.len() * 16);
    m.insert("faces", parsed.faces.len() * 4);
    m.insert("indices", n_indices_padded);
    m.insert("edges", parsed.edges.len() * 4);
    m.insert("vertex_edges", parsed.vertex_edges.len() * 4);
    m
}

// ---------------------------------------------------------------------------
// Builder — novel geometry from vertices
// ---------------------------------------------------------------------------

// Reference binary files embedded at compile time.
// The novablast reference supplies fixed structural objects (items 1-7, 14-15).
// The TYPE section template is game-format-specific (Starfield hknpPhysicsSystemData).
static NOVABLAST_REF: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../python/creation_lib/havok/tests/novablast_reference.bin"
));
static CONVEX_TYPE_SECTION: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../python/creation_lib/havok/templates/convex_type_section.bin"
));

/// Build a Havok 2019 tagged binary blob for Starfield convex collision.
///
/// Port of `_build_novel` in `py_creation_lib/python/creation_lib/havok/collision_payload.py`.
///
/// The function uses the embedded novablast reference blob to supply the
/// structural fixed objects (items 1-7, 14-15) and replaces the variable
/// geometry arrays (items 8-13: vertices, planes, faces, indices, edges,
/// vertex_edges) with the hull computed from `verts`.
///
/// Hull algorithm: pure-Rust Quickhull in `py_creation_lib/native/havok/src/collision/hull.rs`.
/// Output contract: byte-comparable to Python for the same inputs within
/// f32 floating-point ordering differences (Qhull vs Quickhull facet
/// ordering may differ).
pub fn build_convex_collision(verts: &[[f32; 3]]) -> HavokResult<Vec<u8>> {
    use super::hull::compute_hull_topology_robust;

    // Parse the embedded reference blob once to get fixed structural objects.
    let ref_parsed = parse_tagged_collision(NOVABLAST_REF)?;

    // Compute convex hull topology.
    let hull = compute_hull_topology_robust(verts)?;

    // Serialize variable arrays.
    let vert_bytes = serialize_vertices(&hull.vertices);
    let plane_bytes = serialize_planes(&hull.planes);
    let face_bytes = serialize_faces(&hull.faces);
    let idx_bytes = hull.indices.clone();
    let edge_bytes = serialize_edges(&hull.edges);
    let ve_bytes = serialize_vertex_edges(&hull.vertex_edges);

    let n_verts = hull.vertices.len();
    let n_planes = hull.planes.len();
    let n_faces = hull.faces.len();
    let n_indices = hull.indices.len();
    let n_edges = hull.edges.len();
    let n_vertex_edges = hull.vertex_edges.len();

    // Build a lookup for fixed objects from the reference.
    let fixed_map: std::collections::HashMap<usize, &Vec<u8>> = ref_parsed
        .fixed_objects
        .iter()
        .map(|(idx, b)| (*idx, b))
        .collect();
    let ref_items_by_idx: std::collections::HashMap<usize, &ItemRecord> =
        ref_parsed.items.iter().map(|it| (it.index, it)).collect();

    // Layout DATA section:
    //   fixed prefix: items 1-5 at their original reference offsets
    //   variable arrays: items 8-13 at new computed offsets
    //   fixed suffix: items 6, 7, 14, 15 at new computed offsets
    let fixed_prefix_items = [1usize, 2, 3, 4, 5];
    let fixed_suffix_items = [6usize, 7, 14, 15];

    let mut new_offsets: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    let mut new_counts: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();

    // Place fixed prefix at their original reference offsets (structure is unchanged).
    for &idx in &fixed_prefix_items {
        let ref_item = ref_items_by_idx[&idx];
        new_offsets.insert(idx, ref_item.data_offset);
        new_counts.insert(idx, ref_item.count);
    }

    // Place variable arrays starting immediately after fixed prefix item 5.
    // Reference item 5 ends at offset 496 + 112 = 608; start variable arrays at 608.
    let var_start = {
        let item5 = ref_items_by_idx[&5];
        let raw5 = fixed_map[&5];
        align16(item5.data_offset + raw5.len())
    };

    let var_arrays: &[(&[u8], usize, usize)] = &[
        (vert_bytes.as_slice(), n_verts, 8),
        (plane_bytes.as_slice(), n_planes, 9),
        (face_bytes.as_slice(), n_faces, 10),
        (idx_bytes.as_slice(), n_indices, 11),
        (edge_bytes.as_slice(), n_edges, 12),
        (ve_bytes.as_slice(), n_vertex_edges, 13),
    ];

    let mut offset = var_start;
    for &(arr, count, item_idx) in var_arrays {
        new_offsets.insert(item_idx, offset);
        new_counts.insert(item_idx, count);
        offset = align16(offset + arr.len());
    }

    // Place fixed suffix items.
    for &idx in &fixed_suffix_items {
        let raw = fixed_map[&idx];
        new_offsets.insert(idx, offset);
        new_counts.insert(idx, ref_items_by_idx[&idx].count);
        offset = align16(offset + raw.len());
    }

    let total_data_size = offset;
    let mut data = vec![0u8; total_data_size];

    // Write fixed prefix.
    for &idx in &fixed_prefix_items {
        let raw = fixed_map[&idx];
        let off = new_offsets[&idx];
        data[off..off + raw.len()].copy_from_slice(raw);
    }

    // Write variable arrays.
    for &(arr, _, item_idx) in var_arrays {
        let off = new_offsets[&item_idx];
        data[off..off + arr.len()].copy_from_slice(arr);
    }

    // Write fixed suffix.
    for &idx in &fixed_suffix_items {
        let raw = fixed_map[&idx];
        let off = new_offsets[&idx];
        data[off..off + raw.len()].copy_from_slice(raw);
    }

    // Build ITEM entries (null at index 0, then items 1-15 in order).
    let mut tagged_items: Vec<TaggedItem> = vec![TaggedItem {
        kind: 0,
        type_idx: 0,
        data_offset: 0,
        count: 0,
    }];
    for idx in 1..=15usize {
        let ref_item = ref_items_by_idx[&idx];
        tagged_items.push(TaggedItem {
            kind: ref_item.kind,
            type_idx: ref_item.type_idx,
            data_offset: new_offsets[&idx] as u32,
            count: new_counts[&idx] as u32,
        });
    }

    // Adjust PTCH entries: any src within item 7's reference data range
    // must be relocated to item 7's new offset.
    let ref_item7_offset = ref_items_by_idx[&7].data_offset;
    let raw7_len = fixed_map[&7].len();
    let new_item7_offset = new_offsets[&7];

    let tagged_patches: Vec<PatchEntry> = ref_parsed
        .patches
        .iter()
        .map(|p| {
            let src = if (p.src as usize) >= ref_item7_offset
                && (p.src as usize) < ref_item7_offset + raw7_len
            {
                (new_item7_offset + (p.src as usize - ref_item7_offset)) as u32
            } else {
                p.src
            };
            PatchEntry {
                src,
                flag: p.flag,
                target: p.target,
            }
        })
        .collect();

    let mut builder = TaggedBlobBuilder::new("20190200");
    builder.set_type_section(CONVEX_TYPE_SECTION.to_vec());
    builder.set_data(data);
    builder.set_items(tagged_items);
    builder.set_patches(tagged_patches);
    Ok(builder.build())
}

// ---------------------------------------------------------------------------
// Primitive readers
// ---------------------------------------------------------------------------

fn read_f32_le(data: &[u8], offset: usize) -> f32 {
    f32::from_le_bytes(data[offset..offset + 4].try_into().unwrap())
}

fn read_u32_be(data: &[u8], offset: usize) -> HavokResult<u32> {
    if data.len() < offset + 4 {
        return Err(HavokError::InvalidInput(format!(
            "need 4 bytes at offset {offset}, only {} available",
            data.len().saturating_sub(offset)
        )));
    }
    Ok(u32::from_be_bytes(
        data[offset..offset + 4].try_into().unwrap(),
    ))
}
