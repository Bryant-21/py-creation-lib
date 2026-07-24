use std::collections::HashMap;

use crate::error::{HavokError, HavokResult};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct CompressedMeshSection {
    pub vertices: Vec<[f32; 3]>,
    pub triangles: Vec<[u32; 3]>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompressedMeshData {
    pub sections: Vec<CompressedMeshSection>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RawCompressedMeshDataRun {
    /// `hknpCompressedMeshShapeTreeDataRunData::data` (`hkUint16`).
    pub value: u16,
    /// `hkcdStaticMeshTreeBase::PrimitiveDataRunBase::index` (`hkUint8`).
    pub index: u8,
    pub count: u8,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RawCompressedMeshSparseMap {
    pub secondary_key_mask: u32,
    pub secondary_key_bits: u32,
    pub primary_key_to_index: Vec<u16>,
    pub value_and_secondary_keys: Vec<u16>,
}

impl Default for RawCompressedMeshSparseMap {
    fn default() -> Self {
        Self {
            secondary_key_mask: u32::MAX,
            secondary_key_bits: 0,
            primary_key_to_index: Vec::new(),
            value_and_secondary_keys: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RawCompressedMeshBitField {
    pub words: Vec<u32>,
    pub num_bits: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RawCompressedMeshSection {
    pub aabb_min: [f32; 3],
    pub aabb_max: [f32; 3],
    pub base: [f32; 3],
    pub scale: [f32; 3],
    pub packed_vertices: Vec<u32>,
    pub shared_vertices_index: Vec<u16>,
    pub primitive_bytes: Vec<u8>,
    pub section_tree_nodes: Vec<u8>,
    pub primitive_data_runs: Vec<RawCompressedMeshDataRun>,
    pub leaf_index: u16,
    pub page: u8,
    pub flags: u8,
    pub layer_data: u8,
    pub unused_data: u8,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RawCompressedMeshData {
    pub user_data: u64,
    pub edge_welding_map: RawCompressedMeshSparseMap,
    pub quad_is_flat: RawCompressedMeshBitField,
    pub triangle_is_interior: RawCompressedMeshBitField,
    pub materials: Vec<MaterialEntry>,
    pub object_aabb_min: [f32; 3],
    pub object_aabb_max: [f32; 3],
    pub num_primitive_keys: u32,
    pub bits_per_key: u32,
    pub max_key_value: u32,
    pub primitive_stores_is_flat_convex: u8,
    pub master_tree_nodes: Vec<u8>,
    pub sections: Vec<RawCompressedMeshSection>,
    pub shared_vertices: Vec<u64>,
}

const FLAT_QUAD_COPLANAR_TOLERANCE: f32 = 0.01;

/// Minimum triangle area-squared (cross.lengthSquared) for a compressed-mesh
/// triangle, matching the SDK builder's HKNP_DEFAULT_TRIANGLE_DEGENERACY_TOLERANCE
/// (hknpConfig.h: 1e-7f). Triangles below this are degenerate slivers the SDK
/// would drop; shipping them lets FO4's narrowphase compute a NaN face normal.
const MIN_TRIANGLE_AREA_SQUARED: f32 = 1.0e-7;

pub fn vertex_is_finite(vertex: &[f32; 3]) -> bool {
    vertex.iter().all(|value| value.is_finite())
}

pub fn triangle_area_squared(verts: &[[f32; 3]], tri: &[u32; 3]) -> Option<f32> {
    let a = *verts.get(tri[0] as usize)?;
    let b = *verts.get(tri[1] as usize)?;
    let c = *verts.get(tri[2] as usize)?;
    let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let cross = [
        ab[1] * ac[2] - ab[2] * ac[1],
        ab[2] * ac[0] - ab[0] * ac[2],
        ab[0] * ac[1] - ab[1] * ac[0],
    ];
    Some(cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2])
}

/// True if `tri_local` (indices into a section's decoded packed vertices) is
/// degenerate after quantization — decoded vertices fall below the SDK floor.
fn quantized_triangle_is_degenerate(
    decoded_section_verts: &[[f32; 3]],
    tri_local: &[u32; 3],
) -> bool {
    match triangle_area_squared(decoded_section_verts, tri_local) {
        Some(area_sq) => !area_sq.is_finite() || area_sq <= MIN_TRIANGLE_AREA_SQUARED,
        None => true,
    }
}

/// Quantize a section's vertices with the per-section min/scale the encoder
/// uses, then decode them back to positions. Mirrors `encode_compressed_mesh_section`'s
/// 11-11-10 path (the only format the per-section vertex grid uses) and the
/// 21-21-22 path for completeness. The decoded positions are what FO4 actually
/// sees, so they are what the post-quant degeneracy recheck operates on.
fn decode_quantized_section_vertices(section_vertices: &[[f32; 3]]) -> Vec<[f32; 3]> {
    if section_vertices.is_empty() {
        return Vec::new();
    }

    let mut mn = [f32::INFINITY; 3];
    let mut mx = [f32::NEG_INFINITY; 3];
    for v in section_vertices {
        for axis in 0..3 {
            mn[axis] = mn[axis].min(v[axis]);
            mx[axis] = mx[axis].max(v[axis]);
        }
    }

    // 11-11-10 grid maxima, matching `encode_compressed_mesh_section`.
    let max_q = [2047.0f32, 2047.0, 1023.0];
    let safe_scale = |a: usize| -> f32 {
        if mx[a] > mn[a] {
            (mx[a] - mn[a]) / max_q[a]
        } else {
            1.0
        }
    };
    let scale = [safe_scale(0), safe_scale(1), safe_scale(2)];

    section_vertices
        .iter()
        .map(|v| {
            let mut decoded = [0.0f32; 3];
            for axis in 0..3 {
                let q = if scale[axis] != 1.0 || mn[axis] != mx[axis] {
                    ((v[axis] - mn[axis]) / scale[axis])
                        .round()
                        .clamp(0.0, max_q[axis])
                } else {
                    0.0
                };
                decoded[axis] = mn[axis] + q * scale[axis];
            }
            decoded
        })
        .collect()
}

pub fn validate_vertices(verts: &[[f32; 3]]) -> HavokResult<()> {
    for (idx, vertex) in verts.iter().enumerate() {
        if !vertex_is_finite(vertex) {
            return Err(HavokError::InvalidInput(format!(
                "vertex {idx} is not finite: {vertex:?}"
            )));
        }
    }
    Ok(())
}

pub fn validate_compressed_triangle(
    verts: &[[f32; 3]],
    tri: &[u32; 3],
    tri_index: usize,
) -> HavokResult<()> {
    for &idx in tri {
        if idx as usize >= verts.len() {
            return Err(HavokError::InvalidInput(format!(
                "triangle {tri_index} references vertex {idx}, but only {} vertices were provided",
                verts.len()
            )));
        }
    }
    if tri[0] == tri[1] || tri[1] == tri[2] || tri[0] == tri[2] {
        return Err(HavokError::InvalidInput(format!(
            "triangle {tri_index} has repeated vertex indices: {tri:?}"
        )));
    }
    for &idx in tri {
        let vertex = &verts[idx as usize];
        if !vertex_is_finite(vertex) {
            return Err(HavokError::InvalidInput(format!(
                "triangle {tri_index} references non-finite vertex {idx}: {vertex:?}"
            )));
        }
    }
    let area_squared = triangle_area_squared(verts, tri).unwrap_or(0.0);
    if !area_squared.is_finite() || area_squared <= MIN_TRIANGLE_AREA_SQUARED {
        return Err(HavokError::InvalidInput(format!(
            "triangle {tri_index} has near-zero area: {tri:?}"
        )));
    }
    Ok(())
}

pub fn compressed_triangle_is_safe(verts: &[[f32; 3]], tri: &[u32; 3]) -> bool {
    validate_compressed_triangle(verts, tri, 0).is_ok()
}

/// One material entry for hknpBSMaterialProperties.
/// Provides the per-shape filter_info (layer/group/mask bitmask) and
/// material_crc (Bethesda material hash from the BSPhysicsMaterialTable).
/// The vanilla default single-material values are filter_info=0x064003D4,
/// material_crc=0xC0EB623D.
#[derive(Debug, Clone, PartialEq)]
pub struct MaterialEntry {
    pub filter_info: u32,
    pub material_crc: u32,
}

impl MaterialEntry {
    /// The vanilla FO4 single-material defaults sampled from reference NIFs.
    pub const VANILLA_DEFAULT: MaterialEntry = MaterialEntry {
        filter_info: 0x064003D4,
        material_crc: 0xC0EB623D,
    };
}

/// Options for build_compressed_mesh_collision.
#[derive(Debug, Clone)]
pub struct BuildOptions {
    pub friction: f32,
    pub restitution: f32,
    pub layer: u8,
    /// Body mass in kilograms.  `0.0` denotes a static body and produces a
    /// zero-filled `hknpShapeMassProperties` block (matching vanilla static
    /// content); positive values trigger AABB-approximation inertia.
    pub mass: f32,
    /// Opaque caller-defined tag written to `hknpShape::userData`.
    /// SDK contract: caller-defined, no runtime semantics.  Defaults to `0`.
    pub user_data: Option<u64>,
    /// Convex radius written to `hknpShape::convexRadius`.
    /// SDK default is 0.05; set to 0.0 to disable.
    pub convex_radius: f32,
    /// Per-shape materials for hknpBSMaterialProperties.
    /// If empty, defaults to [`MaterialEntry::VANILLA_DEFAULT`] (single-material
    /// backward compatibility).
    pub materials: Vec<MaterialEntry>,
    /// Verbatim 16-byte body_props material region (bytes `[0x10..0x20]` of
    /// the 0x50 hknpMaterial inline-struct payload). When `Some`, the writer
    /// emits these bytes after the friction/restitution patches — letting
    /// callers preserve damping / max-velocity / Bethesda flag bits captured
    /// from a vanilla NIF instead of reconstructing from defaults.
    ///
    /// Mirrors pynifly's `body_props_raw` (`refs/io_scene_nifly/nif/collision.py:1318`):
    /// `00ff003f003fcd3e01024c3deeff7f7f`.
    pub body_props_raw: Option<[u8; 16]>,
    /// The source body's true mass distribution (FO76 `hknpRefMassDistribution`),
    /// when present. When `Some`, the shape's `hknpShapeMassProperties` block is
    /// built from the real COM / volume / inertia instead of the AABB box
    /// approximation. Set per-body from [`BodyMeta::mass_distribution`].
    pub mass_distribution: Option<super::mass_properties::SourceMassDistribution>,
}

impl Default for BuildOptions {
    fn default() -> Self {
        BuildOptions {
            friction: 0.5,
            restitution: 0.4,
            layer: 5,
            mass: 0.0,
            user_data: None,
            convex_radius: 0.05,
            materials: Vec::new(),
            body_props_raw: None,
            mass_distribution: None,
        }
    }
}

/// Build an hknpBSMaterialProperties blob (hkReferencedObject + MaterialA
/// hkArray) for the given materials.
///
/// Binary layout (per vanilla FO4 reference NIFs):
///   +0x00: hkReferencedObject (16 bytes)
///   +0x10: hkArray<hknpBSMaterial> ptr (8 bytes), count (4), cap (4)
///   +0x20: hknpBSMaterial entries (hkReferencedObject + filter + material)
///
/// When `materials` is empty the vanilla single-entry template is emitted
/// unchanged for backward compatibility.
pub(crate) fn build_bs_material_properties(materials: &[MaterialEntry]) -> Vec<u8> {
    if materials.is_empty() {
        return PF_BS_MAT_PROPS.to_vec();
    }
    let n = materials.len() as u32;
    let cap_flag = n | 0x8000_0000;
    let mut buf = vec![0u8; 0x20]; // hkRefObj + MaterialA hkArray header
    // hkArray<hknpBSMaterial> at +0x10: ptr=0, count=n, cap=n|0x80000000
    buf[0x18..0x1C].copy_from_slice(&n.to_le_bytes());
    buf[0x1C..0x20].copy_from_slice(&cap_flag.to_le_bytes());

    for m in materials {
        buf.extend_from_slice(&[0u8; 16]); // hkReferencedObject base
        buf.extend_from_slice(&m.filter_info.to_le_bytes());
        buf.extend_from_slice(&m.material_crc.to_le_bytes());
    }
    while buf.len() % 16 != 0 {
        buf.push(0);
    }
    buf
}

// ---------------------------------------------------------------------------
// Pack helpers
// ---------------------------------------------------------------------------

pub fn pack_vertex_11_11_10(x: u32, y: u32, z: u32) -> u32 {
    (z << 22) | (y << 11) | x
}

pub fn pack_vertex_21_21_22(x: u64, y: u64, z: u64) -> u64 {
    (z << 42) | (y << 21) | x
}

#[derive(Debug, Clone, Copy)]
struct CmAabb {
    min: [f32; 3],
    max: [f32; 3],
}

impl CmAabb {
    fn from_triangle(verts: &[[f32; 3]], tri: &[u32; 3]) -> Self {
        let a = verts[tri[0] as usize];
        let b = verts[tri[1] as usize];
        let c = verts[tri[2] as usize];
        Self {
            min: [
                a[0].min(b[0]).min(c[0]),
                a[1].min(b[1]).min(c[1]),
                a[2].min(b[2]).min(c[2]),
            ],
            max: [
                a[0].max(b[0]).max(c[0]),
                a[1].max(b[1]).max(c[1]),
                a[2].max(b[2]).max(c[2]),
            ],
        }
    }

    fn merged(&self, other: &Self) -> Self {
        Self {
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

    fn centroid(&self, axis: usize) -> f32 {
        (self.min[axis] + self.max[axis]) * 0.5
    }
}

fn merge_aabbs(aabbs: &[CmAabb], indices: &[usize]) -> CmAabb {
    let mut combined = aabbs[indices[0]];
    for &idx in indices.iter().skip(1) {
        combined = combined.merged(&aabbs[idx]);
    }
    combined
}

fn merge_two_aabbs(a: CmAabb, b: CmAabb) -> CmAabb {
    a.merged(&b)
}

fn align16(v: usize) -> usize {
    (v + 15) & !15
}

fn ceil_log2_u32(v: u32) -> u32 {
    if v <= 1 {
        0
    } else {
        u32::BITS - (v - 1).leading_zeros()
    }
}

fn unpack_compressed_axis(parent_min: f32, parent_max: f32, packed: u8) -> (f32, f32) {
    let extent = (parent_max - parent_min) / 226.0;
    let min_nibble = (packed >> 4) as f32;
    let max_nibble = (packed & 0x0F) as f32;
    (
        parent_min + extent * min_nibble * min_nibble,
        parent_max - extent * max_nibble * max_nibble,
    )
}

fn pack_compressed_axis(parent_min: f32, parent_max: f32, child_min: f32, child_max: f32) -> u8 {
    if parent_max <= parent_min {
        return 0;
    }

    let mut packed = 0u8;
    while (packed >> 4) < 15 {
        let candidate = packed + 0x10;
        let (unpacked_min, _) = unpack_compressed_axis(parent_min, parent_max, candidate);
        if unpacked_min > child_min {
            break;
        }
        packed = candidate;
    }

    while (packed & 0x0F) < 15 {
        let candidate = packed + 0x01;
        let (_, unpacked_max) = unpack_compressed_axis(parent_min, parent_max, candidate);
        if unpacked_max < child_max {
            break;
        }
        packed = candidate;
    }

    packed
}

fn pack_aabb4_node(parent: &CmAabb, child: &CmAabb, data: u8) -> [u8; 4] {
    [
        pack_compressed_axis(parent.min[0], parent.max[0], child.min[0], child.max[0]),
        pack_compressed_axis(parent.min[1], parent.max[1], child.min[1], child.max[1]),
        pack_compressed_axis(parent.min[2], parent.max[2], child.min[2], child.max[2]),
        data,
    ]
}

// hkcdCompressedAabbCodecs::Aabb5BytesCodec node — the master tree over Sections.
// Memory layout is [x, y, z, m_hiData, m_loData]; the engine reads
//   isInternal = m_hiData & 0x80
//   leaf      -> getData()  = (m_hiData << 8) | m_loData          (high bit clear)
//   internal  -> getDelta() = (((m_hiData & 0x7F) << 8) | m_loData) << 1
// Note this is NOT the Aabb4 convention (data&1 / data>>1) used by the per-section
// sub-trees; using that for the master tree makes FO4 read the internal root as a
// leaf and deref m_sections out of bounds.
fn pack_aabb5_leaf(parent: &CmAabb, child: &CmAabb, section_index: u16) -> [u8; 5] {
    debug_assert!(section_index < 0x8000, "section index must fit in 15 bits");
    [
        pack_compressed_axis(parent.min[0], parent.max[0], child.min[0], child.max[0]),
        pack_compressed_axis(parent.min[1], parent.max[1], child.min[1], child.max[1]),
        pack_compressed_axis(parent.min[2], parent.max[2], child.min[2], child.max[2]),
        (section_index >> 8) as u8,   // m_hiData (bit7 = 0 => leaf)
        (section_index & 0xFF) as u8, // m_loData
    ]
}

fn pack_aabb5_internal(parent: &CmAabb, child: &CmAabb, delta: u16) -> [u8; 5] {
    debug_assert_eq!(delta & 1, 0, "right-child delta must be even");
    let odd = delta >> 1;
    [
        pack_compressed_axis(parent.min[0], parent.max[0], child.min[0], child.max[0]),
        pack_compressed_axis(parent.min[1], parent.max[1], child.min[1], child.max[1]),
        pack_compressed_axis(parent.min[2], parent.max[2], child.min[2], child.max[2]),
        0x80 | (odd >> 8) as u8, // m_hiData (bit7 = 1 => internal)
        (odd & 0xFF) as u8,      // m_loData
    ]
}

fn build_section_tree_node(
    primitive_aabbs: &[CmAabb],
    indices: &[usize],
    parent_aabb: &CmAabb,
    out: &mut Vec<[u8; 4]>,
) -> usize {
    let node_index = out.len();
    let node_aabb = merge_aabbs(primitive_aabbs, indices);

    if indices.len() == 1 {
        let primitive_index = indices[0];
        out.push(pack_aabb4_node(
            parent_aabb,
            &node_aabb,
            (primitive_index as u8) << 1,
        ));
        return node_index;
    }

    out.push([0u8; 4]);

    let dx = node_aabb.max[0] - node_aabb.min[0];
    let dy = node_aabb.max[1] - node_aabb.min[1];
    let dz = node_aabb.max[2] - node_aabb.min[2];
    let axis = if dx >= dy && dx >= dz {
        0
    } else if dy >= dz {
        1
    } else {
        2
    };

    let mut sorted = indices.to_vec();
    sorted.sort_by(|&a, &b| {
        primitive_aabbs[a]
            .centroid(axis)
            .partial_cmp(&primitive_aabbs[b].centroid(axis))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mid = sorted.len() / 2;
    let left = &sorted[..mid];
    let right = &sorted[mid..];

    build_section_tree_node(primitive_aabbs, left, &node_aabb, out);
    let right_index = build_section_tree_node(primitive_aabbs, right, &node_aabb, out);
    let delta = right_index - node_index;
    debug_assert_eq!(delta & 1, 0);
    debug_assert!(delta <= 0xFE);
    out[node_index] = pack_aabb4_node(parent_aabb, &node_aabb, (delta as u8) | 1);
    node_index
}

fn build_section_tree_nodes(primitive_aabbs: &[CmAabb], domain: &CmAabb) -> Vec<u8> {
    let mut nodes = Vec::with_capacity(primitive_aabbs.len() * 2 - 1);
    let indices: Vec<usize> = (0..primitive_aabbs.len()).collect();
    build_section_tree_node(primitive_aabbs, &indices, domain, &mut nodes);

    let mut out = Vec::with_capacity(nodes.len() * 4);
    for node in nodes {
        out.extend_from_slice(&node);
    }
    out
}

fn build_master_tree_node(
    section_aabbs: &[CmAabb],
    indices: &[usize],
    parent_aabb: &CmAabb,
    out: &mut Vec<[u8; 5]>,
    leaf_node_index: &mut [u16],
) -> usize {
    let node_index = out.len();
    let node_aabb = merge_aabbs(section_aabbs, indices);

    if indices.len() == 1 {
        let section_index = indices[0];
        out.push(pack_aabb5_leaf(
            parent_aabb,
            &node_aabb,
            section_index as u16,
        ));
        // Section::m_leafIndex (struct 0x5A) records the master-tree node index
        // where this section's leaf lives — matches vanilla FO4 (bookkeeping only;
        // the query path recovers the section via the node payload + pointer math).
        leaf_node_index[section_index] = node_index as u16;
        return node_index;
    }

    out.push([0u8; 5]);

    let dx = node_aabb.max[0] - node_aabb.min[0];
    let dy = node_aabb.max[1] - node_aabb.min[1];
    let dz = node_aabb.max[2] - node_aabb.min[2];
    let axis = if dx >= dy && dx >= dz {
        0
    } else if dy >= dz {
        1
    } else {
        2
    };

    let mut sorted = indices.to_vec();
    sorted.sort_by(|&a, &b| {
        section_aabbs[a]
            .centroid(axis)
            .partial_cmp(&section_aabbs[b].centroid(axis))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mid = sorted.len() / 2;
    let left = &sorted[..mid];
    let right = &sorted[mid..];

    build_master_tree_node(section_aabbs, left, &node_aabb, out, leaf_node_index);
    let right_index =
        build_master_tree_node(section_aabbs, right, &node_aabb, out, leaf_node_index);
    let delta = right_index - node_index;
    debug_assert_eq!(delta & 1, 0);
    debug_assert!(delta <= 0xFFFE);
    out[node_index] = pack_aabb5_internal(parent_aabb, &node_aabb, delta as u16);
    node_index
}

fn build_master_tree_nodes(section_aabbs: &[CmAabb], domain: &CmAabb) -> (Vec<u8>, Vec<u16>) {
    let mut nodes = Vec::with_capacity(section_aabbs.len() * 2 - 1);
    let mut leaf_node_index = vec![0u16; section_aabbs.len()];
    let indices: Vec<usize> = (0..section_aabbs.len()).collect();
    build_master_tree_node(
        section_aabbs,
        &indices,
        domain,
        &mut nodes,
        &mut leaf_node_index,
    );

    let mut out = Vec::with_capacity(nodes.len() * 5);
    for node in nodes {
        out.extend_from_slice(&node);
    }
    (out, leaf_node_index)
}

// ---------------------------------------------------------------------------
// Packfile constants (mirror of Python)
// ---------------------------------------------------------------------------

const HKX_MAGIC: &[u8; 8] = b"\x57\xE0\xE0\x57\x10\xC0\xC0\x10";
const SECTION_STRIDE: usize = 0x60;
const MAX_SECTION_VERTICES: usize = 255;
const MAX_SECTION_TRIANGLES: usize = 128;

// Section struct field offsets
const SEC_AABB_MIN: usize = 0x10;
const SEC_AABB_MAX: usize = 0x20;
const SEC_BASE: usize = 0x30;
const SEC_SCALE_X: usize = 0x3C;
const SEC_SCALE_Y: usize = 0x40;
const SEC_SCALE_Z: usize = 0x44;
const SEC_FIRST_VERTEX: usize = 0x48;
const SEC_VERT_PACKED: usize = 0x4C;
const SEC_QUAD_PACKED: usize = 0x50;
const SEC_DATA_RUN_PACKED: usize = 0x54;
const SEC_NUM_PACKED_VERTICES: usize = 0x58;
const SEC_NUM_SHARED_INDICES: usize = 0x59;

// ---------------------------------------------------------------------------
// Packfile reader helpers
// ---------------------------------------------------------------------------

fn u8_at(data: &[u8], off: usize) -> HavokResult<u8> {
    data.get(off)
        .copied()
        .ok_or_else(|| HavokError::InvalidInput(format!("u8 read at {off:#x} out of bounds")))
}

fn u16_le(data: &[u8], off: usize) -> HavokResult<u16> {
    if off + 2 > data.len() {
        return Err(HavokError::InvalidInput(format!(
            "u16 read at {off:#x} out of bounds"
        )));
    }
    Ok(u16::from_le_bytes(data[off..off + 2].try_into().unwrap()))
}

fn u32_le(data: &[u8], off: usize) -> HavokResult<u32> {
    if off + 4 > data.len() {
        return Err(HavokError::InvalidInput(format!(
            "u32 read at {off:#x} out of bounds"
        )));
    }
    Ok(u32::from_le_bytes(data[off..off + 4].try_into().unwrap()))
}

fn u64_le(data: &[u8], off: usize) -> HavokResult<u64> {
    if off + 8 > data.len() {
        return Err(HavokError::InvalidInput(format!(
            "u64 read at {off:#x} out of bounds"
        )));
    }
    Ok(u64::from_le_bytes(data[off..off + 8].try_into().unwrap()))
}

fn f32_le(data: &[u8], off: usize) -> HavokResult<f32> {
    if off + 4 > data.len() {
        return Err(HavokError::InvalidInput(format!(
            "f32 read at {off:#x} out of bounds"
        )));
    }
    Ok(f32::from_le_bytes(data[off..off + 4].try_into().unwrap()))
}

fn vec3_le(data: &[u8], off: usize) -> HavokResult<[f32; 3]> {
    Ok([
        f32_le(data, off)?,
        f32_le(data, off + 4)?,
        f32_le(data, off + 8)?,
    ])
}

// ---------------------------------------------------------------------------
// Packfile section header parsing
// ---------------------------------------------------------------------------

struct SectionHeader {
    abs_start: usize,
    local_fix: usize,
    global_fix: usize,
    virt_fix: usize,
    exports: usize,
}

fn parse_packfile_section_headers(
    data: &[u8],
) -> HavokResult<std::collections::HashMap<String, SectionHeader>> {
    let mut hdrs = std::collections::HashMap::new();
    for i in 0..3usize {
        let base = 0x40 + i * 0x40;
        if base + 0x30 > data.len() {
            break;
        }
        // Name is a null-terminated ASCII string in first 19 bytes
        let name_end = data[base..base + 19]
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(19);
        if name_end == 0 {
            continue;
        }
        let name = std::str::from_utf8(&data[base..base + name_end])
            .unwrap_or("")
            .to_string();
        if name.is_empty() {
            continue;
        }
        let s = u32_le(data, base + 0x14)? as usize;
        let local_fix_rel = u32_le(data, base + 0x18)? as usize;
        let global_fix_rel = u32_le(data, base + 0x1C)? as usize;
        let virt_fix_rel = u32_le(data, base + 0x20)? as usize;
        let exports_rel = u32_le(data, base + 0x24)? as usize;
        hdrs.insert(
            name,
            SectionHeader {
                abs_start: s,
                local_fix: s + local_fix_rel,
                global_fix: s + global_fix_rel,
                virt_fix: s + virt_fix_rel,
                exports: s + exports_rel,
            },
        );
    }
    Ok(hdrs)
}

fn parse_local_fixups(
    data: &[u8],
    hdr: &SectionHeader,
) -> HavokResult<std::collections::HashMap<usize, usize>> {
    let mut fix = std::collections::HashMap::new();
    let mut pos = hdr.local_fix;
    let end = hdr.global_fix.min(data.len());
    while pos + 8 <= end {
        let src = u32_le(data, pos)? as usize;
        let dst = u32_le(data, pos + 4)? as usize;
        if src == 0xFFFF_FFFF {
            break;
        }
        fix.insert(src, dst);
        pos += 8;
    }
    Ok(fix)
}

fn parse_virtual_fixups(
    data: &[u8],
    hdr: &SectionHeader,
    cn_start: usize,
) -> HavokResult<Vec<(usize, String)>> {
    let mut objs = Vec::new();
    let mut pos = hdr.virt_fix;
    let end = hdr.exports.min(data.len());
    while pos + 12 <= end {
        let src = u32_le(data, pos)? as usize;
        // _sec: u32 at pos+4, unused
        let name_off = u32_le(data, pos + 8)? as usize;
        if src == 0xFFFF_FFFF {
            break;
        }
        let abs_name = cn_start + name_off;
        let cls = if abs_name < data.len() {
            let max_end = (abs_name + 128).min(data.len());
            data[abs_name..max_end]
                .iter()
                .position(|&b| b == 0)
                .map(|ne| {
                    std::str::from_utf8(&data[abs_name..abs_name + ne])
                        .unwrap_or("")
                        .to_string()
                })
                .unwrap_or_else(|| format!("?{name_off:#x}"))
        } else {
            format!("?{name_off:#x}")
        };
        objs.push((src, cls));
        pos += 12;
    }
    Ok(objs)
}

fn hkarray_abs(
    fixups: &std::collections::HashMap<usize, usize>,
    data_start: usize,
    obj_rel: usize,
    field_off: usize,
) -> Option<usize> {
    fixups
        .get(&(obj_rel + field_off))
        .map(|&dst| data_start + dst)
}

fn hkarray_size(data: &[u8], obj_abs: usize, field_off: usize) -> HavokResult<usize> {
    Ok((u32_le(data, obj_abs + field_off + 8)? & 0x3FFF_FFFF) as usize)
}

// ---------------------------------------------------------------------------
// Parser: parse_fo4_compressed_mesh
// ---------------------------------------------------------------------------

pub fn parse_fo4_compressed_mesh(blob: &[u8]) -> HavokResult<CompressedMeshData> {
    if blob.len() < 0x100 || blob.get(0..8) != Some(HKX_MAGIC.as_slice()) {
        return Err(HavokError::InvalidInput(
            "missing Havok packfile magic for FO4 compressed mesh payload".to_string(),
        ));
    }

    let hdrs = parse_packfile_section_headers(blob)?;

    let data_hdr = hdrs.get("__data__").ok_or_else(|| {
        HavokError::InvalidInput("Missing __data__ section in packfile".to_string())
    })?;
    let data_start = data_hdr.abs_start;

    let cn_start = hdrs.get("__classnames__").map(|h| h.abs_start).unwrap_or(0);

    let fixups = parse_local_fixups(blob, data_hdr)?;
    let objects = parse_virtual_fixups(blob, data_hdr, cn_start)?;

    // Find hknpCompressedMeshShapeData
    let mesh_obj = objects
        .iter()
        .find(|(_, cls)| cls.contains("hknpCompressedMeshShapeData"))
        .ok_or_else(|| {
            HavokError::InvalidInput("No hknpCompressedMeshShapeData found in packfile".to_string())
        })?;

    let obj_rel = mesh_obj.0;
    let obj_abs = data_start + obj_rel;

    // Object-level AABB (vec4 fields: read x,y,z of each)
    let obj_bb_min = vec3_le(blob, obj_abs + 0x20)?;
    let obj_bb_max = vec3_le(blob, obj_abs + 0x30)?;

    // Array pointers and sizes
    let sections_abs = hkarray_abs(&fixups, data_start, obj_rel, 0x50);
    let sections_count = hkarray_size(blob, obj_abs, 0x50)?;
    let quads_abs = hkarray_abs(&fixups, data_start, obj_rel, 0x60);
    let total_quads = hkarray_size(blob, obj_abs, 0x60)?;
    let shidx_abs = hkarray_abs(&fixups, data_start, obj_rel, 0x70);
    let total_shidx = hkarray_size(blob, obj_abs, 0x70)?;
    let verts_abs = hkarray_abs(&fixups, data_start, obj_rel, 0x80);
    let total_verts = hkarray_size(blob, obj_abs, 0x80)?;
    let shared_abs = hkarray_abs(&fixups, data_start, obj_rel, 0x90);
    let total_shared = hkarray_size(blob, obj_abs, 0x90)?;

    if sections_abs.is_none() || sections_count == 0 {
        return Err(HavokError::InvalidInput(
            "No sections in compressed mesh data".to_string(),
        ));
    }
    let sections_abs = sections_abs.unwrap();

    // Decode shared (large) vertices using object-level AABB
    let mask_x = (1u64 << 21) - 1;
    let mask_y = (1u64 << 21) - 1;
    let mask_z = (1u64 << 22) - 1;
    let sx_shared = if obj_bb_max[0] > obj_bb_min[0] {
        (obj_bb_max[0] - obj_bb_min[0]) / mask_x as f32
    } else {
        0.0
    };
    let sy_shared = if obj_bb_max[1] > obj_bb_min[1] {
        (obj_bb_max[1] - obj_bb_min[1]) / mask_y as f32
    } else {
        0.0
    };
    let sz_shared = if obj_bb_max[2] > obj_bb_min[2] {
        (obj_bb_max[2] - obj_bb_min[2]) / mask_z as f32
    } else {
        0.0
    };

    let mut shared_verts: Vec<[f32; 3]> = Vec::new();
    if let Some(sh_abs) = shared_abs {
        for i in 0..total_shared {
            let off = sh_abs + i * 8;
            if off + 8 > blob.len() {
                break;
            }
            let v = u64_le(blob, off)?;
            let (qx, qy, qz) = unpack_vertex_21_21_22_local(v);
            shared_verts.push([
                obj_bb_min[0] + qx as f32 * sx_shared,
                obj_bb_min[1] + qy as f32 * sy_shared,
                obj_bb_min[2] + qz as f32 * sz_shared,
            ]);
        }
    }

    // Read section structs
    #[allow(dead_code)]
    struct RawSection {
        aabb_min: [f32; 3],
        aabb_max: [f32; 3],
        base: [f32; 3],
        scale: [f32; 3],
        first_vertex: usize,
        num_packed: usize,
        first_shidx: usize,
        num_quads: usize,
        first_quad: usize,
        num_vertices: usize,
        num_shared: usize,
        num_data_runs: usize,
    }

    let mut raw_sections: Vec<RawSection> = Vec::with_capacity(sections_count);
    for i in 0..sections_count {
        let o = sections_abs + i * SECTION_STRIDE;
        let vert_raw = u32_le(blob, o + SEC_VERT_PACKED)?;
        let num_packed = (vert_raw & 0xFF) as usize;
        let first_shidx = ((vert_raw >> 8) & 0xFF_FFFF) as usize;

        let quad_raw = u32_le(blob, o + SEC_QUAD_PACKED)?;
        let num_quads = (quad_raw & 0xFF) as usize;
        let first_quad = ((quad_raw >> 8) & 0xFF_FFFF) as usize;

        let data_run_raw = u32_le(blob, o + SEC_DATA_RUN_PACKED)?;
        let declared_num_packed = u8_at(blob, o + SEC_NUM_PACKED_VERTICES)? as usize;
        let declared_num_shared = u8_at(blob, o + SEC_NUM_SHARED_INDICES)? as usize;
        let first_vertex = u32_le(blob, o + SEC_FIRST_VERTEX)? as usize;

        raw_sections.push(RawSection {
            aabb_min: vec3_le(blob, o + SEC_AABB_MIN)?,
            aabb_max: vec3_le(blob, o + SEC_AABB_MAX)?,
            base: vec3_le(blob, o + SEC_BASE)?,
            scale: [
                f32_le(blob, o + SEC_SCALE_X)?,
                f32_le(blob, o + SEC_SCALE_Y)?,
                f32_le(blob, o + SEC_SCALE_Z)?,
            ],
            first_vertex,
            num_packed: declared_num_packed.max(num_packed),
            first_shidx,
            num_quads,
            first_quad,
            num_vertices: declared_num_packed.max(num_packed),
            num_shared: declared_num_shared,
            num_data_runs: (data_run_raw & 0xFF) as usize,
        });
    }

    // Compute num_vertices and num_shared from delta between sections
    for i in 0..raw_sections.len() {
        if i + 1 < raw_sections.len() {
            if raw_sections[i].num_vertices == 0 {
                raw_sections[i].num_vertices =
                    raw_sections[i + 1].first_vertex - raw_sections[i].first_vertex;
            }
            if raw_sections[i].num_shared == 0 {
                raw_sections[i].num_shared =
                    raw_sections[i + 1].first_shidx - raw_sections[i].first_shidx;
            }
        } else {
            if raw_sections[i].num_vertices == 0 {
                raw_sections[i].num_vertices =
                    total_verts.saturating_sub(raw_sections[i].first_vertex);
            }
            if raw_sections[i].num_shared == 0 {
                raw_sections[i].num_shared =
                    total_shidx.saturating_sub(raw_sections[i].first_shidx);
            }
        }
    }

    let _ = total_quads; // informational only

    // Parse each section
    let mut result_sections: Vec<CompressedMeshSection> = Vec::with_capacity(sections_count);
    for sec_info in &raw_sections {
        if sec_info.num_quads == 0 {
            result_sections.push(CompressedMeshSection {
                vertices: Vec::new(),
                triangles: Vec::new(),
            });
            continue;
        }

        // Decode packed vertices (11-11-10)
        let mut local_verts: Vec<[f32; 3]> = Vec::new();
        if let Some(va) = verts_abs {
            let bx = sec_info.base[0];
            let by = sec_info.base[1];
            let bz = sec_info.base[2];
            let sx = sec_info.scale[0];
            let sy = sec_info.scale[1];
            let sz = sec_info.scale[2];
            for i in 0..sec_info.num_vertices {
                let off = va + (sec_info.first_vertex + i) * 4;
                if off + 4 > blob.len() {
                    break;
                }
                let v = u32_le(blob, off)?;
                let (qx, qy, qz) = unpack_vertex_11_11_10_local(v);
                local_verts.push([
                    bx + qx as f32 * sx,
                    by + qy as f32 * sy,
                    bz + qz as f32 * sz,
                ]);
            }
        }

        // Resolve shared vertices for this section via shidx mapping
        let mut section_shared: Vec<[f32; 3]> = Vec::new();
        if let Some(si_abs) = shidx_abs {
            if sec_info.num_shared > 0 && !shared_verts.is_empty() {
                for k in 0..sec_info.num_shared {
                    let map_off = si_abs + (sec_info.first_shidx + k) * 2;
                    if map_off + 2 <= blob.len() {
                        let gi = u16_le(blob, map_off)? as usize;
                        if gi < shared_verts.len() {
                            section_shared.push(shared_verts[gi]);
                        } else {
                            section_shared.push([0.0, 0.0, 0.0]);
                        }
                    }
                }
            }
        }

        let mut all_verts = local_verts;
        all_verts.extend_from_slice(&section_shared);
        let idx_limit = all_verts.len();

        // Decode quads (4x u8 indices → triangles)
        let mut tris: Vec<[u32; 3]> = Vec::new();
        if let Some(qa) = quads_abs {
            for i in 0..sec_info.num_quads {
                let off = qa + (sec_info.first_quad + i) * 4;
                if off + 4 > blob.len() {
                    break;
                }
                let a = u8_at(blob, off)? as usize;
                let b = u8_at(blob, off + 1)? as usize;
                let c = u8_at(blob, off + 2)? as usize;
                let d = u8_at(blob, off + 3)? as usize;
                if a >= idx_limit || b >= idx_limit || c >= idx_limit || d >= idx_limit {
                    continue;
                }
                tris.push([a as u32, b as u32, c as u32]);
                if c != d {
                    tris.push([a as u32, c as u32, d as u32]);
                }
            }
        }

        result_sections.push(CompressedMeshSection {
            vertices: all_verts,
            triangles: tris,
        });
    }

    Ok(CompressedMeshData {
        sections: result_sections,
    })
}

pub(crate) fn fo4_compressed_mesh_flat_convex_markers(blob: &[u8]) -> HavokResult<Vec<u8>> {
    if blob.len() < 0x100 || blob.get(0..8) != Some(HKX_MAGIC.as_slice()) {
        return Err(HavokError::InvalidInput(
            "missing Havok packfile magic for FO4 compressed mesh payload".to_string(),
        ));
    }
    let headers = parse_packfile_section_headers(blob)?;
    let data_header = headers.get("__data__").ok_or_else(|| {
        HavokError::InvalidInput("Missing __data__ section in packfile".to_string())
    })?;
    let classnames_start = headers
        .get("__classnames__")
        .map(|header| header.abs_start)
        .unwrap_or(0);
    let objects = parse_virtual_fixups(blob, data_header, classnames_start)?;
    objects
        .into_iter()
        .filter(|(_, class_name)| class_name.contains("hknpCompressedMeshShapeData"))
        .map(|(object_rel, _)| {
            blob.get(data_header.abs_start + object_rel + 0x4C)
                .copied()
                .ok_or_else(|| {
                    HavokError::InvalidInput(
                        "compressed mesh flat-convex marker is out of bounds".to_string(),
                    )
                })
        })
        .collect()
}

pub(crate) fn patch_fo4_compressed_mesh_flat_convex_markers(
    blob: &mut [u8],
    markers: &[u8],
) -> HavokResult<()> {
    let headers = parse_packfile_section_headers(blob)?;
    let data_header = headers.get("__data__").ok_or_else(|| {
        HavokError::InvalidInput("Missing __data__ section in packfile".to_string())
    })?;
    let classnames_start = headers
        .get("__classnames__")
        .map(|header| header.abs_start)
        .unwrap_or(0);
    let objects = parse_virtual_fixups(blob, data_header, classnames_start)?;
    let data_objects = objects
        .into_iter()
        .filter(|(_, class_name)| class_name.contains("hknpCompressedMeshShapeData"))
        .collect::<Vec<_>>();
    if data_objects.len() != markers.len() {
        return Err(HavokError::InvalidInput(format!(
            "compressed mesh marker count {} does not match shape-data count {}",
            markers.len(),
            data_objects.len()
        )));
    }
    for ((object_rel, _), marker) in data_objects.into_iter().zip(markers) {
        let offset = data_header.abs_start + object_rel + 0x4C;
        let target = blob.get_mut(offset).ok_or_else(|| {
            HavokError::InvalidInput(
                "compressed mesh flat-convex marker is out of bounds".to_string(),
            )
        })?;
        *target = *marker;
    }
    Ok(())
}

// local versions to avoid cross-module dep cycle; mod.rs already has the public ones
fn unpack_vertex_11_11_10_local(packed: u32) -> (u32, u32, u32) {
    (
        packed & 0x7FF,
        (packed >> 11) & 0x7FF,
        (packed >> 22) & 0x3FF,
    )
}

fn unpack_vertex_21_21_22_local(packed: u64) -> (u64, u64, u64) {
    (
        packed & 0x1F_FFFF,
        (packed >> 21) & 0x1F_FFFF,
        (packed >> 42) & 0x3F_FFFF,
    )
}

// ---------------------------------------------------------------------------
// Builder constants (mirror of Python _PF_* constants)
// ---------------------------------------------------------------------------

pub(crate) const PF_MAGIC: &[u8; 8] = b"\x57\xE0\xE0\x57\x10\xC0\xC0\x10";
pub(crate) const PF_FILE_VERSION: i32 = 11;
pub(crate) const PF_LAYOUT_RULES: &[u8; 4] = b"\x08\x01\x00\x01";
pub(crate) const PF_CONTENTS_VER: &[u8; 16] = b"hk_2014.1.0-r1\x00\xff";
pub(crate) const PF_MAX_PREDICATE: i32 = 21;

// Class name hashes shared across all FO4 Havok 2014 packfile shapes.
#[allow(dead_code)]
pub(crate) const PF_HKCLASS_BASE: &[(u32, &str)] = &[
    (0x33D42383, "hkClass"),
    (0xB0EFA719, "hkClassMember"),
    (0x8A3609CF, "hkClassEnum"),
    (0xCE6F8A6C, "hkClassEnumItem"),
];

// Class name entries: compressed mesh chain.
const PF_CM_CLASS_ENTRIES: &[(u32, &str)] = &[
    (0x33D42383, "hkClass"),
    (0xB0EFA719, "hkClassMember"),
    (0x8A3609CF, "hkClassEnum"),
    (0xCE6F8A6C, "hkClassEnumItem"),
    (0xB857718B, "hknpPhysicsSystemData"),
    (0x5F60D536, "hknpCompressedMeshShape"),
    (0xA2BDFC59, "hknpCompressedMeshShapeData"),
    (0x7C574867, "hkRefCountedProperties"),
    (0xA3E47A9A, "hknpBSMaterialProperties"),
];

// Class name entries: single-polytope chain (covers convex_hull/box/sphere/
// capsule/cylinder — vanilla FO4 uses hknpConvexPolytopeShape for all convex
// primitives). Ordering matches vanilla 10mmCompensator dumps.
#[allow(dead_code)]
pub(crate) const PF_POLYTOPE_CLASS_ENTRIES: &[(u32, &str)] = &[
    (0x33D42383, "hkClass"),
    (0xB0EFA719, "hkClassMember"),
    (0x8A3609CF, "hkClassEnum"),
    (0xCE6F8A6C, "hkClassEnumItem"),
    (0xB857718B, "hknpPhysicsSystemData"),
    (0x3CE9B3E3, "hknpConvexPolytopeShape"),
    (0x7C574867, "hkRefCountedProperties"),
    (0xE9191728, "hknpShapeMassProperties"),
];

// Class name entries: hknpDynamicCompoundShape wrapping N polytope sub-shapes.
#[allow(dead_code)]
pub(crate) const PF_COMPOUND_POLY_CLASS_ENTRIES: &[(u32, &str)] = &[
    (0x33D42383, "hkClass"),
    (0xB0EFA719, "hkClassMember"),
    (0x8A3609CF, "hkClassEnum"),
    (0xCE6F8A6C, "hkClassEnumItem"),
    (0xB857718B, "hknpPhysicsSystemData"),
    (0x4620D11C, "hknpDynamicCompoundShape"),
    (0x3CE9B3E3, "hknpConvexPolytopeShape"),
    (0x7C574867, "hkRefCountedProperties"),
    (0xE9191728, "hknpShapeMassProperties"),
    (0xF33DC3CC, "hknpDynamicCompoundShapeData"),
];

// Class name entries: hknpDynamicCompoundShape wrapping N compressed-mesh
// sub-shapes (vanilla ACDuctConnector01 pattern).
#[allow(dead_code)]
pub(crate) const PF_COMPOUND_MESH_CLASS_ENTRIES: &[(u32, &str)] = &[
    (0x33D42383, "hkClass"),
    (0xB0EFA719, "hkClassMember"),
    (0x8A3609CF, "hkClassEnum"),
    (0xCE6F8A6C, "hkClassEnumItem"),
    (0xB857718B, "hknpPhysicsSystemData"),
    (0x4620D11C, "hknpDynamicCompoundShape"),
    (0x5F60D536, "hknpCompressedMeshShape"),
    (0x7C574867, "hkRefCountedProperties"),
    (0xA3E47A9A, "hknpBSMaterialProperties"),
    (0xA2BDFC59, "hknpCompressedMeshShapeData"),
    (0xF33DC3CC, "hknpDynamicCompoundShapeData"),
];

const PF_BODY_PROPS: &[u8; 0x50] = &[
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0xFF, 0x00, 0x3F, 0x00, 0x3F, 0xCD, 0x3E, 0x01, 0x02, 0x4C, 0x3D, 0xEE, 0xFF, 0x7F, 0x7F,
    0x00, 0x00, 0x80, 0x3F, 0x00, 0x00, 0x80, 0x3F, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xA0, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

const PF_BODY_CINFO: &[u8; 0x60] = &[
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0x7F, 0xFF, 0xFF, 0xFF, 0x7F,
    0xFF, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0x7F, 0x3F,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

const PF_CM_SHAPE_HDR: &[u8; 0xC0] = &[
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x04, 0x02, 0x07, 0x02, 0x00, 0x00, 0x00, 0x00, 0x15, 0x7d, 0x06, 0x26, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0xff, 0xff, 0xff, 0xff, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x44, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

const PF_REF_COUNTED_PROPS: &[u8; 0x20] = &[
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x80,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xF1, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

const PF_BS_MAT_PROPS: &[u8; 0x50] = &[
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x80,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x01, 0x00, 0x00, 0x00, 0xd4, 0x03, 0x40, 0x06, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x3d, 0x62, 0xeb, 0xc0,
];

fn bitfield_word_count(num_bits: u32) -> usize {
    num_bits.div_ceil(32) as usize
}

fn write_bitfield_header(buf: &mut Vec<u8>, off: usize, num_bits: u32) {
    let word_count = bitfield_word_count(num_bits) as u32;
    write_u32_le_into(buf, off + 0x08, word_count);
    write_u32_le_into(buf, off + 0x0C, word_count | 0x8000_0000);
    write_u32_le_into(buf, off + 0x10, num_bits);
}

fn quad_is_flat_bitfield_storage(
    encoded_sections: &[EncodedCompressedMeshSection],
    num_bits: u32,
    primitive_stores_is_flat_convex: u8,
) -> Vec<u8> {
    let mut words = vec![0u32; bitfield_word_count(num_bits)];
    for (section_index, section) in encoded_sections.iter().enumerate() {
        for (local_primitive_index, primitive) in
            section.primitive_bytes.chunks_exact(4).enumerate()
        {
            let is_quad = primitive[2] != primitive[3];
            let is_flat = primitive_stores_is_flat_convex != u8::MAX || primitive[1] > primitive[3];
            if !is_quad || !is_flat {
                continue;
            }

            let bit_index = (section_index << 7) | local_primitive_index;
            debug_assert!(bit_index < num_bits as usize);
            words[bit_index >> 5] |= 1u32 << (bit_index & 31);
        }
    }

    words.into_iter().flat_map(u32::to_le_bytes).collect()
}

fn build_compressed_mesh_shape_header(
    num_shape_key_bits: u8,
    user_data: u64,
    edge_welding_map: Option<&RawCompressedMeshSparseMap>,
    quad_is_flat_bits: u32,
    triangle_is_interior_bits: u32,
) -> Vec<u8> {
    let mut buf = PF_CM_SHAPE_HDR.to_vec();
    buf[0x12] = num_shape_key_bits;
    buf[0x18..0x20].copy_from_slice(&user_data.to_le_bytes());
    buf[0x30..0x98].fill(0);
    if let Some(map) = edge_welding_map {
        write_u32_le_into(&mut buf, 0x30, map.secondary_key_mask);
        write_u32_le_into(&mut buf, 0x34, map.secondary_key_bits);
        write_u32_le_into(&mut buf, 0x40, map.primary_key_to_index.len() as u32);
        write_u32_le_into(
            &mut buf,
            0x44,
            map.primary_key_to_index.len() as u32 | 0x8000_0000,
        );
        write_u32_le_into(&mut buf, 0x50, map.value_and_secondary_keys.len() as u32);
        write_u32_le_into(
            &mut buf,
            0x54,
            map.value_and_secondary_keys.len() as u32 | 0x8000_0000,
        );
    } else {
        write_u32_le_into(&mut buf, 0x30, u32::MAX);
        write_u32_le_into(&mut buf, 0x44, 0x8000_0000);
        write_u32_le_into(&mut buf, 0x54, 0x8000_0000);
    }
    write_u32_le_into(&mut buf, 0x58, u32::MAX);
    write_bitfield_header(&mut buf, 0x68, quad_is_flat_bits);
    write_bitfield_header(&mut buf, 0x80, triangle_is_interior_bits);
    buf
}

// ---------------------------------------------------------------------------
// Builder helpers
// ---------------------------------------------------------------------------

pub(crate) fn w_u32(v: u32) -> [u8; 4] {
    v.to_le_bytes()
}

pub(crate) fn w_u64(v: u64) -> [u8; 8] {
    v.to_le_bytes()
}

/// Build a 16-byte hkArray header (ptr=0, count, capFlags with user-alloc bit).
pub(crate) fn hkarray(count: usize) -> Vec<u8> {
    let cap_flags: u32 = if count > 0 {
        (count as u32) | 0x8000_0000
    } else {
        0x8000_0000
    };
    let mut v = Vec::with_capacity(16);
    v.extend_from_slice(&w_u64(0));
    v.extend_from_slice(&w_u32(count as u32));
    v.extend_from_slice(&w_u32(cap_flags));
    v
}

/// Pad to 16-byte boundary with 0xFF fill.
pub(crate) fn pad16_ff(data: &[u8]) -> Vec<u8> {
    let r = data.len() % 16;
    if r == 0 {
        data.to_vec()
    } else {
        let mut v = data.to_vec();
        v.extend(std::iter::repeat(0xFF).take(16 - r));
        v
    }
}

// ---------------------------------------------------------------------------
// Fixup builder
// ---------------------------------------------------------------------------

pub(crate) struct FixupBuilder {
    pub(crate) local: Vec<(usize, usize)>,
    pub(crate) global: Vec<(usize, usize, usize)>,
    pub(crate) virtual_: Vec<(usize, usize, usize)>,
}

impl FixupBuilder {
    pub(crate) fn new() -> Self {
        FixupBuilder {
            local: Vec::new(),
            global: Vec::new(),
            virtual_: Vec::new(),
        }
    }

    pub(crate) fn add_local(&mut self, src_rel: usize, dst_rel: usize) {
        self.local.push((src_rel, dst_rel));
    }

    pub(crate) fn add_global(&mut self, src_rel: usize, sec_idx: usize, dst_rel: usize) {
        self.global.push((src_rel, sec_idx, dst_rel));
    }

    pub(crate) fn add_virtual(&mut self, obj_rel: usize, sec_idx: usize, name_off: usize) {
        self.virtual_.push((obj_rel, sec_idx, name_off));
    }

    pub(crate) fn build_local_table(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for &(src, dst) in &self.local {
            out.extend_from_slice(&w_u32(src as u32));
            out.extend_from_slice(&w_u32(dst as u32));
        }
        out.extend_from_slice(&w_u32(0xFFFF_FFFF));
        out.extend_from_slice(&w_u32(0xFFFF_FFFF));
        out
    }

    pub(crate) fn build_global_table(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for &(src, sec, dst) in &self.global {
            out.extend_from_slice(&w_u32(src as u32));
            out.extend_from_slice(&w_u32(sec as u32));
            out.extend_from_slice(&w_u32(dst as u32));
        }
        out.extend_from_slice(&w_u32(0xFFFF_FFFF));
        out.extend_from_slice(&w_u32(0xFFFF_FFFF));
        out.extend_from_slice(&w_u32(0xFFFF_FFFF));
        out
    }

    pub(crate) fn build_virtual_table(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for &(obj, sec, noff) in &self.virtual_ {
            out.extend_from_slice(&w_u32(obj as u32));
            out.extend_from_slice(&w_u32(sec as u32));
            out.extend_from_slice(&w_u32(noff as u32));
        }
        out.extend_from_slice(&w_u32(0xFFFF_FFFF));
        out.extend_from_slice(&w_u32(0xFFFF_FFFF));
        out.extend_from_slice(&w_u32(0xFFFF_FFFF));
        out
    }
}

// ---------------------------------------------------------------------------
// Build classnames section
// ---------------------------------------------------------------------------

/// Build a packfile `__classnames__` section from `(hash, name)` entries.
///
/// Returns the section bytes (padded to 16-byte boundary with 0xFF) and a
/// `{name → name_offset}` map for use when emitting virtual fixups.
/// Shape-agnostic — used by the compressed-mesh, polytope, and compound builders.
pub(crate) fn build_classnames(
    entries: &[(u32, &str)],
) -> (Vec<u8>, std::collections::HashMap<String, usize>) {
    let mut data: Vec<u8> = Vec::new();
    let mut name_offs = std::collections::HashMap::new();
    for &(hash_val, name) in entries {
        let name_off = data.len() + 5; // 4-byte hash + 1-byte flag
        data.extend_from_slice(&hash_val.to_le_bytes());
        data.push(0x09);
        data.extend_from_slice(name.as_bytes());
        data.push(0x00);
        name_offs.insert(name.to_string(), name_off);
    }
    let data = pad16_ff(&data);
    (data, name_offs)
}

fn build_cm_classnames() -> (Vec<u8>, std::collections::HashMap<String, usize>) {
    build_classnames(PF_CM_CLASS_ENTRIES)
}

/// Build the 0x50 hknpMaterial body-props payload. Friction and restitution
/// are stored as truncated float16 (upper 16 bits of f32) at offsets 0x12,
/// 0x14, and 0x16. When `raw` is `Some`, the friction/restitution patches
/// are applied first and then the caller-supplied 16-byte material region
/// (`[0x10..0x20]`) overwrites them — so callers preserving pynifly-style
/// `body_props_raw` get a byte-exact round-trip even if those fields differ
/// from the friction/restitution values reconstructed from defaults.
pub(crate) fn build_body_props_with_raw(
    friction: f32,
    restitution: f32,
    raw: Option<&[u8; 16]>,
) -> Vec<u8> {
    let mut buf = PF_BODY_PROPS.to_vec();
    let fric_bytes = friction.to_le_bytes();
    let rest_bytes = restitution.to_le_bytes();
    buf[0x12] = fric_bytes[2];
    buf[0x13] = fric_bytes[3];
    buf[0x14] = fric_bytes[2];
    buf[0x15] = fric_bytes[3];
    buf[0x16] = rest_bytes[2];
    buf[0x17] = rest_bytes[3];
    if let Some(bytes) = raw {
        buf[0x10..0x20].copy_from_slice(bytes);
    }
    buf
}

pub(crate) fn build_body_cinfo(layer: u8) -> Vec<u8> {
    let mut buf = PF_BODY_CINFO.to_vec();
    // layer must land in the low byte of m_collisionFilterInfo at offset 0x14
    // (hknpBodyCinfo_2.xml), NOT in m_qualityId at offset 0x10. The template's
    // default qualityId byte (PF_BODY_CINFO[0x10] = 0xFF) is preserved.
    buf[0x14] = layer;
    buf
}

#[derive(Debug)]
struct EncodedCompressedMeshSection {
    aabb: CmAabb,
    base: [f32; 3],
    scale: [f32; 3],
    packed_vertices: Vec<u32>,
    shared_vertices_index: Vec<u16>,
    primitive_bytes: Vec<u8>,
    section_tree_nodes: Vec<u8>,
    primitive_data_runs: Vec<RawCompressedMeshDataRun>,
    leaf_index: u16,
    page: u8,
    flags: u8,
    layer_data: u8,
    unused_data: u8,
}

fn vec_sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn vec_cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn vec_dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn triangle_has_cyclic_order(triangle: [u32; 3], expected: [u32; 3]) -> bool {
    triangle == expected
        || triangle == [expected[1], expected[2], expected[0]]
        || triangle == [expected[2], expected[0], expected[1]]
}

fn primitive_is_flat_convex_quad(vertices: &[[f32; 3]], primitive: [u32; 4]) -> bool {
    let Some(&a) = vertices.get(primitive[0] as usize) else {
        return false;
    };
    let Some(&b) = vertices.get(primitive[1] as usize) else {
        return false;
    };
    let Some(&c) = vertices.get(primitive[2] as usize) else {
        return false;
    };
    let Some(&d) = vertices.get(primitive[3] as usize) else {
        return false;
    };

    let normal_a = vec_cross(vec_sub(b, a), vec_sub(c, a));
    let normal_b = vec_cross(vec_sub(c, a), vec_sub(d, a));
    let normal_a_len_sq = vec_dot(normal_a, normal_a);
    let normal_b_len_sq = vec_dot(normal_b, normal_b);
    if normal_a_len_sq <= MIN_TRIANGLE_AREA_SQUARED
        || normal_b_len_sq <= MIN_TRIANGLE_AREA_SQUARED
        || !normal_a_len_sq.is_finite()
        || !normal_b_len_sq.is_finite()
    {
        return false;
    }

    let normal_dot = vec_dot(normal_a, normal_b);
    if normal_dot <= 0.0 {
        return false;
    }

    let plane_distance = vec_dot(normal_a, vec_sub(d, a)).abs() / normal_a_len_sq.sqrt();
    if !plane_distance.is_finite() || plane_distance > FLAT_QUAD_COPLANAR_TOLERANCE {
        return false;
    }

    let points = [a, b, c, d];
    for idx in 0..4 {
        let p0 = points[idx];
        let p1 = points[(idx + 1) % 4];
        let p2 = points[(idx + 2) % 4];
        let turn = vec_cross(vec_sub(p1, p0), vec_sub(p2, p1));
        if vec_dot(turn, normal_a) < -1.0e-6 * normal_a_len_sq {
            return false;
        }
    }
    true
}

fn encode_triangle_pair_as_primitive(
    vertices: &[[f32; 3]],
    first: [u32; 3],
    second: [u32; 3],
) -> Option<[u8; 4]> {
    for tri in [[first, second], [second, first]] {
        let primary = tri[0];
        let secondary = tri[1];
        for [a, b, c] in [
            primary,
            [primary[1], primary[2], primary[0]],
            [primary[2], primary[0], primary[1]],
        ] {
            if secondary.contains(&a) && secondary.contains(&c) {
                let d = secondary.into_iter().find(|&idx| idx != a && idx != c)?;
                let primitive = [a, b, c, d];
                if primitive.iter().any(|&idx| idx > u8::MAX as u32)
                    || b <= d
                    || !triangle_has_cyclic_order(secondary, [a, c, d])
                    || !primitive_is_flat_convex_quad(vertices, primitive)
                {
                    continue;
                }
                return Some([a as u8, b as u8, c as u8, d as u8]);
            }
        }
    }
    None
}

pub(crate) fn encode_triangle_primitive_bytes(
    vertices: &[[f32; 3]],
    triangles: &[[u32; 3]],
) -> Vec<u8> {
    let mut primitive_bytes = Vec::with_capacity(triangles.len() * 4);
    let mut index = 0usize;
    while index < triangles.len() {
        let first = triangles[index];
        if let Some(second) = triangles.get(index + 1).copied() {
            if let Some(primitive) = encode_triangle_pair_as_primitive(vertices, first, second) {
                primitive_bytes.extend_from_slice(&primitive);
                index += 2;
                continue;
            }
        }

        primitive_bytes.extend_from_slice(&[
            first[0] as u8,
            first[1] as u8,
            first[2] as u8,
            first[2] as u8,
        ]);
        index += 1;
    }
    primitive_bytes
}

fn encode_triangle_primitives(
    vertices: &[[f32; 3]],
    triangles: &[[u32; 3]],
) -> (Vec<u8>, Vec<CmAabb>) {
    let primitive_bytes = encode_triangle_primitive_bytes(vertices, triangles);
    let mut primitive_aabbs = Vec::with_capacity(primitive_bytes.len() / 4);
    let mut tri_index = 0usize;
    for primitive in primitive_bytes.chunks_exact(4) {
        let first = triangles[tri_index];
        if primitive[2] != primitive[3] {
            let second = triangles[tri_index + 1];
            let tri_a = CmAabb::from_triangle(vertices, &first);
            let tri_b = CmAabb::from_triangle(vertices, &second);
            primitive_aabbs.push(merge_two_aabbs(tri_a, tri_b));
            tri_index += 2;
        } else {
            primitive_aabbs.push(CmAabb::from_triangle(vertices, &first));
            tri_index += 1;
        }
    }

    (primitive_bytes, primitive_aabbs)
}

fn split_compressed_mesh_sections(
    verts: &[[f32; 3]],
    tris: &[[u32; 3]],
) -> HavokResult<Vec<CompressedMeshSection>> {
    let mut sections = Vec::new();
    let mut section_vertices: Vec<[f32; 3]> = Vec::new();
    let mut section_triangles: Vec<[u32; 3]> = Vec::new();
    let mut local_indices: HashMap<u32, u32> = HashMap::new();

    for (tri_index, tri) in tris.iter().copied().enumerate() {
        validate_compressed_triangle(verts, &tri, tri_index)?;

        let new_vertices = tri
            .iter()
            .filter(|idx| !local_indices.contains_key(idx))
            .count();
        let over_triangle_limit = section_triangles.len() >= MAX_SECTION_TRIANGLES;
        let over_vertex_limit = local_indices.len() + new_vertices > MAX_SECTION_VERTICES;
        if !section_triangles.is_empty() && (over_triangle_limit || over_vertex_limit) {
            flush_compressed_mesh_section(
                &mut sections,
                &mut section_vertices,
                &mut section_triangles,
                &mut local_indices,
            );
        }

        let mut local_tri = [0u32; 3];
        for (slot, global_idx) in tri.into_iter().enumerate() {
            let local_idx = if let Some(local_idx) = local_indices.get(&global_idx) {
                *local_idx
            } else {
                let local_idx = section_vertices.len() as u32;
                section_vertices.push(verts[global_idx as usize]);
                local_indices.insert(global_idx, local_idx);
                local_idx
            };
            local_tri[slot] = local_idx;
        }
        section_triangles.push(local_tri);
    }

    flush_compressed_mesh_section(
        &mut sections,
        &mut section_vertices,
        &mut section_triangles,
        &mut local_indices,
    );

    Ok(sections)
}

fn flush_compressed_mesh_section(
    sections: &mut Vec<CompressedMeshSection>,
    section_vertices: &mut Vec<[f32; 3]>,
    section_triangles: &mut Vec<[u32; 3]>,
    local_indices: &mut HashMap<u32, u32>,
) {
    if section_triangles.is_empty() {
        return;
    }
    // Re-validate against the positions FO4 will actually decode: quantizing the
    // section's vertices into the per-section grid can collapse two float-distinct
    // vertices onto one cell, zeroing a triangle's area. Such triangles produce a
    // NaN narrowphase face normal -> CTD, so drop them post-quant. If every
    // triangle collapses the section is empty and must not be emitted (the writer
    // errors on empty sections); leaving it unflushed keeps the master-tree /
    // section numbering consistent.
    let decoded = decode_quantized_section_vertices(section_vertices);
    section_triangles.retain(|tri| !quantized_triangle_is_degenerate(&decoded, tri));
    if section_triangles.is_empty() {
        section_vertices.clear();
        local_indices.clear();
        return;
    }
    sections.push(CompressedMeshSection {
        vertices: std::mem::take(section_vertices),
        triangles: std::mem::take(section_triangles),
    });
    local_indices.clear();
}

fn encode_compressed_mesh_section(section: &CompressedMeshSection) -> EncodedCompressedMeshSection {
    let xs: Vec<f32> = section.vertices.iter().map(|v| v[0]).collect();
    let ys: Vec<f32> = section.vertices.iter().map(|v| v[1]).collect();
    let zs: Vec<f32> = section.vertices.iter().map(|v| v[2]).collect();

    let min_x = xs.iter().cloned().fold(f32::INFINITY, f32::min);
    let max_x = xs.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    let min_y = ys.iter().cloned().fold(f32::INFINITY, f32::min);
    let max_y = ys.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    let min_z = zs.iter().cloned().fold(f32::INFINITY, f32::min);
    let max_z = zs.iter().cloned().fold(f32::NEG_INFINITY, f32::max);

    let safe_scale =
        |mn: f32, mx: f32, max_q: f32| -> f32 { if mx > mn { (mx - mn) / max_q } else { 1.0 } };
    let sx = safe_scale(min_x, max_x, 2047.0);
    let sy = safe_scale(min_y, max_y, 2047.0);
    let sz = safe_scale(min_z, max_z, 1023.0);

    let mut packed_vertices = Vec::with_capacity(section.vertices.len());
    for v in &section.vertices {
        let qx = if sx != 1.0 || min_x != max_x {
            ((v[0] - min_x) / sx).round().clamp(0.0, 2047.0) as u32
        } else {
            0
        };
        let qy = if sy != 1.0 || min_y != max_y {
            ((v[1] - min_y) / sy).round().clamp(0.0, 2047.0) as u32
        } else {
            0
        };
        let qz = if sz != 1.0 || min_z != max_z {
            ((v[2] - min_z) / sz).round().clamp(0.0, 1023.0) as u32
        } else {
            0
        };
        packed_vertices.push(pack_vertex_11_11_10(qx, qy, qz));
    }

    let aabb = CmAabb {
        min: [min_x, min_y, min_z],
        max: [max_x, max_y, max_z],
    };
    let (primitive_bytes, primitive_aabbs) =
        encode_triangle_primitives(&section.vertices, &section.triangles);
    let section_tree_nodes = build_section_tree_nodes(&primitive_aabbs, &aabb);
    let primitive_count = primitive_bytes.len() / 4;
    EncodedCompressedMeshSection {
        aabb,
        base: [min_x, min_y, min_z],
        scale: [sx, sy, sz],
        packed_vertices,
        shared_vertices_index: Vec::new(),
        primitive_bytes,
        section_tree_nodes,
        primitive_data_runs: vec![RawCompressedMeshDataRun {
            value: 0,
            index: 0,
            count: primitive_count as u8,
        }],
        leaf_index: 0,
        page: 0,
        flags: 0,
        layer_data: 0,
        unused_data: 0,
    }
}

// ---------------------------------------------------------------------------
// Build data section
// ---------------------------------------------------------------------------

fn build_cm_data_section(
    encoded_sections: &[EncodedCompressedMeshSection],
    object_aabb: CmAabb,
    num_primitive_keys: u32,
    bits_per_key: u32,
    max_key_value: u32,
    primitive_stores_is_flat_convex: u8,
    master_tree_nodes: &[u8],
    shared_vertices: &[u64],
    source_shape: Option<&RawCompressedMeshData>,
    name_offs: &std::collections::HashMap<String, usize>,
    opts: &BuildOptions,
) -> (Vec<u8>, FixupBuilder) {
    let friction = opts.friction;
    let restitution = opts.restitution;
    let layer = opts.layer;
    let total_vertices = encoded_sections
        .iter()
        .map(|section| section.packed_vertices.len())
        .sum::<usize>();
    let total_shared_indices = encoded_sections
        .iter()
        .map(|section| section.shared_vertices_index.len())
        .sum::<usize>();
    let total_primitives = encoded_sections
        .iter()
        .map(|section| section.primitive_bytes.len() / 4)
        .sum::<usize>();
    let total_data_runs = encoded_sections
        .iter()
        .map(|section| section.primitive_data_runs.len())
        .sum::<usize>();
    let mut fx = FixupBuilder::new();
    let mut data: Vec<u8> = Vec::new();

    macro_rules! rel {
        () => {
            data.len()
        };
    }

    macro_rules! write_bytes {
        ($b:expr) => {{
            let off = data.len();
            data.extend_from_slice($b);
            off
        }};
    }

    // -- hknpPhysicsSystemData (0x80 bytes) --
    let psd_rel = rel!();
    fx.add_virtual(psd_rel, 0, *name_offs.get("hknpPhysicsSystemData").unwrap());

    write_bytes!(&hkarray(0)); // +0x00: unused
    let arr10_off = write_bytes!(&hkarray(1)); // +0x10: body_props
    write_bytes!(&hkarray(0)); // +0x20: dyn_motion (empty)
    write_bytes!(&hkarray(0)); // +0x30: dyn_inertia (empty)
    let arr40_off = write_bytes!(&hkarray(1)); // +0x40: BodyCInfo
    write_bytes!(&hkarray(0)); // +0x50: unused
    let arr60_off = write_bytes!(&hkarray(1)); // +0x60: ShapeEntry
    write_bytes!(&[0u8; 16]); // +0x70: pad
    debug_assert_eq!(rel!(), psd_rel + 0x80);

    // -- body_props (0x50 bytes) --
    let body_props_rel = rel!();
    write_bytes!(&build_body_props_with_raw(
        friction,
        restitution,
        opts.body_props_raw.as_ref()
    ));
    fx.add_local(arr10_off, body_props_rel);

    // -- BodyCInfo (0x60 bytes) --
    let body_cinfo_rel = rel!();
    write_bytes!(&build_body_cinfo(layer));
    fx.add_local(arr40_off, body_cinfo_rel);

    // -- ShapeEntry (0x10 bytes) --
    let shape_entry_rel = rel!();
    write_bytes!(&[0u8; 16]);
    fx.add_local(arr60_off, shape_entry_rel);

    // -- hknpCompressedMeshShape (0xC0 bytes) --
    let shape_rel = rel!();
    let generated_triangle_bits = max_key_value.saturating_add(1);
    let triangle_is_interior_bits = source_shape
        .map(|shape| shape.triangle_is_interior.num_bits)
        .filter(|bits| *bits > 0)
        .unwrap_or(generated_triangle_bits);
    let quad_is_flat_bits = source_shape
        .map(|shape| shape.quad_is_flat.num_bits)
        .filter(|bits| *bits > 0)
        .unwrap_or_else(|| generated_triangle_bits.saturating_add(1) / 2);
    let quad_is_flat_words = bitfield_word_count(quad_is_flat_bits);
    let triangle_is_interior_words = bitfield_word_count(triangle_is_interior_bits);
    let shape_hdr = build_compressed_mesh_shape_header(
        bits_per_key as u8,
        opts.user_data.unwrap_or(0),
        source_shape.map(|shape| &shape.edge_welding_map),
        quad_is_flat_bits,
        triangle_is_interior_bits,
    );
    write_bytes!(&shape_hdr);
    debug_assert_eq!(rel!(), shape_rel + 0xC0);

    fx.add_virtual(
        shape_rel,
        0,
        *name_offs.get("hknpCompressedMeshShape").unwrap(),
    );
    fx.add_global(body_cinfo_rel + 0x00, 2, shape_rel);
    fx.add_global(shape_entry_rel + 0x00, 2, shape_rel);
    let shape_refprop_ptr_rel = shape_rel + 0x20;
    let shape_data_ptr_rel = shape_rel + 0x60;

    // -- hknpCompressedMeshShape edge-welding arrays --
    if let Some(shape) = source_shape {
        if !shape.edge_welding_map.primary_key_to_index.is_empty() {
            let primary_key_to_index_rel = rel!();
            fx.add_local(shape_rel + 0x38, primary_key_to_index_rel);
            for value in &shape.edge_welding_map.primary_key_to_index {
                data.extend_from_slice(&value.to_le_bytes());
            }
            while data.len() % 16 != 0 {
                data.push(0);
            }
        }
        if !shape.edge_welding_map.value_and_secondary_keys.is_empty() {
            let value_and_secondary_keys_rel = rel!();
            fx.add_local(shape_rel + 0x48, value_and_secondary_keys_rel);
            for value in &shape.edge_welding_map.value_and_secondary_keys {
                data.extend_from_slice(&value.to_le_bytes());
            }
            while data.len() % 16 != 0 {
                data.push(0);
            }
        }
    }

    // -- hknpCompressedMeshShape bitfield storage --
    let quad_is_flat_words_rel = rel!();
    if quad_is_flat_words > 0 {
        fx.add_local(shape_rel + 0x68, quad_is_flat_words_rel);
        if let Some(words) = source_shape
            .map(|shape| &shape.quad_is_flat)
            .filter(|bitfield| bitfield.num_bits > 0)
            .map(|bitfield| &bitfield.words)
        {
            for word in words {
                data.extend_from_slice(&word.to_le_bytes());
            }
        } else {
            data.extend_from_slice(&quad_is_flat_bitfield_storage(
                encoded_sections,
                quad_is_flat_bits,
                primitive_stores_is_flat_convex,
            ));
        }
        while data.len() % 16 != 0 {
            data.push(0);
        }
    }
    let triangle_is_interior_words_rel = rel!();
    if triangle_is_interior_words > 0 {
        fx.add_local(shape_rel + 0x80, triangle_is_interior_words_rel);
        if let Some(words) = source_shape
            .map(|shape| &shape.triangle_is_interior)
            .filter(|bitfield| bitfield.num_bits > 0)
            .map(|bitfield| &bitfield.words)
        {
            for word in words {
                data.extend_from_slice(&word.to_le_bytes());
            }
        } else {
            data.extend(std::iter::repeat(0u8).take(triangle_is_interior_words * 4));
        }
        while data.len() % 16 != 0 {
            data.push(0);
        }
    }

    // -- hkRefCountedProperties (0x20 bytes) --
    let refprop_rel = rel!();
    write_bytes!(PF_REF_COUNTED_PROPS);
    fx.add_virtual(
        refprop_rel,
        0,
        *name_offs.get("hkRefCountedProperties").unwrap(),
    );
    fx.add_local(refprop_rel + 0x00, refprop_rel + 0x10);
    fx.add_global(shape_refprop_ptr_rel, 2, refprop_rel);
    while data.len() % 16 != 0 {
        data.push(0);
    }

    // -- hknpBSMaterialProperties --
    let bs_mat_rel = rel!();
    let mat_blob = build_bs_material_properties(&opts.materials);
    let n_mats = opts.materials.len();
    write_bytes!(&mat_blob);
    fx.add_global(refprop_rel + 0x10, 2, bs_mat_rel);
    fx.add_virtual(
        bs_mat_rel,
        0,
        *name_offs.get("hknpBSMaterialProperties").unwrap(),
    );
    if n_mats > 0 {
        fx.add_local(bs_mat_rel + 0x10, bs_mat_rel + 0x20);
    } else {
        fx.add_local(bs_mat_rel + 0x10, bs_mat_rel + 0x20);
    }
    while data.len() % 16 != 0 {
        data.push(0);
    }

    let total_section_tree_bytes = encoded_sections
        .iter()
        .map(|section| section.section_tree_nodes.len())
        .sum::<usize>();
    let total_primitive_bytes = encoded_sections
        .iter()
        .map(|section| section.primitive_bytes.len())
        .sum::<usize>();

    // Pre-compute layout positions. Object headers stay 16-byte aligned; POD
    // arrays may have odd element sizes, so each following array gets its own
    // aligned start and a local fixup points at the actual data.
    let sd_rel = rel!();
    let master_nodes_data_rel = sd_rel + 0xD0;
    let sections_data_rel = align16(master_nodes_data_rel + master_tree_nodes.len());
    let section_nodes_data_rel = sections_data_rel + SECTION_STRIDE * encoded_sections.len();
    let primitives_data_rel = align16(section_nodes_data_rel + total_section_tree_bytes);
    let shared_indices_data_rel = align16(primitives_data_rel + total_primitive_bytes);
    let verts_data_rel = align16(shared_indices_data_rel + total_shared_indices * 2);
    let shared_vertices_data_rel = align16(verts_data_rel + total_vertices * 4);
    let primitive_runs_data_rel = align16(shared_vertices_data_rel + shared_vertices.len() * 8);

    // -- hknpCompressedMeshShapeData header (0xD0 bytes) --
    fx.add_virtual(
        sd_rel,
        0,
        *name_offs.get("hknpCompressedMeshShapeData").unwrap(),
    );
    fx.add_global(shape_data_ptr_rel, 2, sd_rel);

    let mut sd_hdr = vec![0u8; 0xD0];
    // +0x00: hkReferencedObject
    // +0x10: meshTree.nodes
    write_u64_le_into(&mut sd_hdr, 0x10, 0);
    write_u32_le_into(&mut sd_hdr, 0x18, master_tree_nodes.len() as u32 / 5);
    write_u32_le_into(
        &mut sd_hdr,
        0x1C,
        (master_tree_nodes.len() as u32 / 5) | 0x8000_0000,
    );
    // +0x20: meshTree.domain.min
    write_f32_le_into(&mut sd_hdr, 0x20, object_aabb.min[0]);
    write_f32_le_into(&mut sd_hdr, 0x24, object_aabb.min[1]);
    write_f32_le_into(&mut sd_hdr, 0x28, object_aabb.min[2]);
    write_f32_le_into(&mut sd_hdr, 0x2C, 0.0);
    // +0x30: meshTree.domain.max
    write_f32_le_into(&mut sd_hdr, 0x30, object_aabb.max[0]);
    write_f32_le_into(&mut sd_hdr, 0x34, object_aabb.max[1]);
    write_f32_le_into(&mut sd_hdr, 0x38, object_aabb.max[2]);
    write_f32_le_into(&mut sd_hdr, 0x3C, 0.0);
    // +0x40: primitive key metadata
    write_u32_le_into(&mut sd_hdr, 0x40, num_primitive_keys);
    write_u32_le_into(&mut sd_hdr, 0x44, bits_per_key);
    write_u32_le_into(&mut sd_hdr, 0x48, max_key_value);
    sd_hdr[0x4C] = primitive_stores_is_flat_convex;
    // +0x50: sections
    write_u64_le_into(&mut sd_hdr, 0x50, 0);
    write_u32_le_into(&mut sd_hdr, 0x58, encoded_sections.len() as u32);
    write_u32_le_into(
        &mut sd_hdr,
        0x5C,
        (encoded_sections.len() as u32) | 0x8000_0000,
    );
    // +0x60: primitives
    write_u64_le_into(&mut sd_hdr, 0x60, 0);
    write_u32_le_into(&mut sd_hdr, 0x68, total_primitives as u32);
    write_u32_le_into(&mut sd_hdr, 0x6C, (total_primitives as u32) | 0x8000_0000);
    // +0x70: sharedVerticesIndex
    write_u64_le_into(&mut sd_hdr, 0x70, 0);
    write_u32_le_into(&mut sd_hdr, 0x78, total_shared_indices as u32);
    write_u32_le_into(
        &mut sd_hdr,
        0x7C,
        (total_shared_indices as u32) | 0x8000_0000,
    );
    // +0x80: packedVertices
    write_u64_le_into(&mut sd_hdr, 0x80, 0);
    write_u32_le_into(&mut sd_hdr, 0x88, total_vertices as u32);
    write_u32_le_into(&mut sd_hdr, 0x8C, (total_vertices as u32) | 0x8000_0000);
    // +0x90: sharedVerts
    write_u64_le_into(&mut sd_hdr, 0x90, 0);
    write_u32_le_into(&mut sd_hdr, 0x98, shared_vertices.len() as u32);
    write_u32_le_into(
        &mut sd_hdr,
        0x9C,
        (shared_vertices.len() as u32) | 0x8000_0000,
    );
    // +0xA0: primitiveDataRuns
    write_u64_le_into(&mut sd_hdr, 0xA0, 0);
    write_u32_le_into(&mut sd_hdr, 0xA8, total_data_runs as u32);
    write_u32_le_into(&mut sd_hdr, 0xAC, (total_data_runs as u32) | 0x8000_0000);
    // +0xB0: hkcdSimdTree.m_nodes — vanilla compressed-mesh shapes always
    // serialize 2 cleared nodes here (size=2, cap=0x80000002) even when the
    // tree is "logically empty". The Havok runtime's hkcdSimdTree::isEmpty()
    // implementation reads `m_nodes[1].isAllocated()` directly — with size=0
    // the data ptr is null, that read derefs low memory, and queries against
    // the mesh shape (e.g. workshop-placement sphere casts) crash in the
    // broadphase. Two zero-cleared nodes (224 bytes total, layout below)
    // satisfy isEmpty() == true while keeping bounds checks safe.
    write_u64_le_into(&mut sd_hdr, 0xB8, 0); // ptr (resolved by local fixup)
    write_u32_le_into(&mut sd_hdr, 0xC0, 2);
    write_u32_le_into(&mut sd_hdr, 0xC4, 0x8000_0002);

    write_bytes!(&sd_hdr);
    debug_assert_eq!(rel!(), sd_rel + 0xD0);

    fx.add_local(sd_rel + 0x10, master_nodes_data_rel);
    fx.add_local(sd_rel + 0x50, sections_data_rel);
    fx.add_local(sd_rel + 0x60, primitives_data_rel);
    if total_shared_indices > 0 {
        fx.add_local(sd_rel + 0x70, shared_indices_data_rel);
    }
    fx.add_local(sd_rel + 0x80, verts_data_rel);
    if !shared_vertices.is_empty() {
        fx.add_local(sd_rel + 0x90, shared_vertices_data_rel);
    }
    fx.add_local(sd_rel + 0xA0, primitive_runs_data_rel);

    // -- meshTree.nodes --
    write_bytes!(&master_tree_nodes);
    while rel!() < sections_data_rel {
        data.push(0);
    }

    // -- Section structs (0x60 bytes each) --
    let mut first_vertex = 0usize;
    let mut first_shared_index = 0usize;
    let mut first_primitive = 0usize;
    let mut first_data_run = 0usize;
    let mut section_tree_offset = 0usize;
    for section in encoded_sections {
        let section_rel = rel!();
        let mut sec = vec![0u8; 0x60];
        // +0x00: section tree nodes
        write_u64_le_into(&mut sec, 0x00, 0);
        write_u32_le_into(
            &mut sec,
            0x08,
            (section.section_tree_nodes.len() / 4) as u32,
        );
        write_u32_le_into(
            &mut sec,
            0x0C,
            ((section.section_tree_nodes.len() / 4) as u32) | 0x8000_0000,
        );
        // +0x10: aabb_min
        write_f32_le_into(&mut sec, 0x10, section.aabb.min[0]);
        write_f32_le_into(&mut sec, 0x14, section.aabb.min[1]);
        write_f32_le_into(&mut sec, 0x18, section.aabb.min[2]);
        write_f32_le_into(&mut sec, 0x1C, 0.0);
        // +0x20: aabb_max
        write_f32_le_into(&mut sec, 0x20, section.aabb.max[0]);
        write_f32_le_into(&mut sec, 0x24, section.aabb.max[1]);
        write_f32_le_into(&mut sec, 0x28, section.aabb.max[2]);
        write_f32_le_into(&mut sec, 0x2C, 0.0);
        // +0x30: base (codec origin)
        write_f32_le_into(&mut sec, 0x30, section.base[0]);
        write_f32_le_into(&mut sec, 0x34, section.base[1]);
        write_f32_le_into(&mut sec, 0x38, section.base[2]);
        // +0x3C: scale X/Y/Z
        write_f32_le_into(&mut sec, 0x3C, section.scale[0]);
        write_f32_le_into(&mut sec, 0x40, section.scale[1]);
        write_f32_le_into(&mut sec, 0x44, section.scale[2]);
        write_u32_le_into(&mut sec, 0x48, first_vertex as u32);
        write_u32_le_into(
            &mut sec,
            0x4C,
            ((first_shared_index as u32) << 8) | section.packed_vertices.len() as u32,
        );
        write_u32_le_into(
            &mut sec,
            0x50,
            ((first_primitive as u32) << 8) | (section.primitive_bytes.len() as u32 / 4),
        );
        write_u32_le_into(
            &mut sec,
            0x54,
            ((first_data_run as u32) << 8) | section.primitive_data_runs.len() as u32,
        );
        sec[0x58] = section.packed_vertices.len() as u8;
        sec[0x59] = section.shared_vertices_index.len() as u8;
        sec[0x5A..0x5C].copy_from_slice(&section.leaf_index.to_le_bytes());
        sec[0x5C] = section.page;
        sec[0x5D] = section.flags;
        sec[0x5E] = section.layer_data;
        sec[0x5F] = section.unused_data;
        write_bytes!(&sec);
        fx.add_local(
            section_rel + 0x00,
            section_nodes_data_rel + section_tree_offset,
        );

        first_vertex += section.packed_vertices.len();
        first_shared_index += section.shared_vertices_index.len();
        first_primitive += section.primitive_bytes.len() / 4;
        first_data_run += section.primitive_data_runs.len();
        section_tree_offset += section.section_tree_nodes.len();
    }
    debug_assert_eq!(
        rel!(),
        sections_data_rel + SECTION_STRIDE * encoded_sections.len()
    );

    // -- Section Aabb4 tree nodes --
    for section in encoded_sections {
        write_bytes!(&section.section_tree_nodes);
    }
    while rel!() < primitives_data_rel {
        data.push(0);
    }

    // -- Primitive data --
    for section in encoded_sections {
        write_bytes!(&section.primitive_bytes);
    }
    while rel!() < shared_indices_data_rel {
        data.push(0);
    }

    // -- Shared vertex index data --
    for section in encoded_sections {
        for index in &section.shared_vertices_index {
            data.extend_from_slice(&index.to_le_bytes());
        }
    }
    while rel!() < verts_data_rel {
        data.push(0);
    }

    // -- Packed vertex data --
    for section in encoded_sections {
        for pv in &section.packed_vertices {
            data.extend_from_slice(&w_u32(*pv));
        }
    }
    while rel!() < shared_vertices_data_rel {
        data.push(0);
    }

    // -- Shared vertex data --
    for shared_vertex in shared_vertices {
        data.extend_from_slice(&shared_vertex.to_le_bytes());
    }
    while rel!() < primitive_runs_data_rel {
        data.push(0);
    }

    // -- Primitive data runs --
    for section in encoded_sections {
        for run in &section.primitive_data_runs {
            data.extend_from_slice(&run.value.to_le_bytes());
            data.push(run.index);
            data.push(run.count);
        }
    }
    while data.len() % 16 != 0 {
        data.push(0);
    }

    // -- hkcdSimdTree::m_nodes: 2 zero-cleared nodes (224 bytes) --
    // Each Node = hkcdFourAabb (96 B: 6 hkVector4 = m_lx, m_hx, m_ly, m_hy,
    // m_lz, m_hz) followed by m_data[4] (16 B). Cleared layout uses Havok's
    // sentinel values: low bounds = +HK_REAL_HIGH (bytes 0xEE 0xFF 0x7F 0x7F
    // = +3.4027767e+38), high bounds = -HK_REAL_HIGH (bytes 0xEE 0xFF 0x7F
    // 0xFF). Matches the byte pattern in vanilla FO4 set-dressing meshes
    // (e.g. Safe01.nif). Required even when the tree is unused — see comment
    // at the m_nodes hkArray header above.
    let simd_nodes_rel = data.len();
    fx.add_local(sd_rel + 0xB8, simd_nodes_rel);
    let lo_lane: [u8; 4] = [0xEE, 0xFF, 0x7F, 0x7F];
    let hi_lane: [u8; 4] = [0xEE, 0xFF, 0x7F, 0xFF];
    for _node in 0..2 {
        // hkcdFourAabb: alternating low/high Vector4 (6 total: lx,hx,ly,hy,lz,hz)
        for vec_idx in 0..6 {
            let lane_pat = if vec_idx % 2 == 0 { &lo_lane } else { &hi_lane };
            for _ in 0..4 {
                data.extend_from_slice(lane_pat);
            }
        }
        // m_data[4]: zero-filled (16 bytes)
        data.extend_from_slice(&[0u8; 16]);
    }
    debug_assert_eq!(data.len() - simd_nodes_rel, 224);
    while data.len() % 16 != 0 {
        data.push(0);
    }
    (data, fx)
}

fn write_u32_le_into(buf: &mut Vec<u8>, off: usize, v: u32) {
    buf[off..off + 4].copy_from_slice(&v.to_le_bytes());
}

fn write_u64_le_into(buf: &mut Vec<u8>, off: usize, v: u64) {
    buf[off..off + 8].copy_from_slice(&v.to_le_bytes());
}

fn write_f32_le_into(buf: &mut Vec<u8>, off: usize, v: f32) {
    buf[off..off + 4].copy_from_slice(&v.to_le_bytes());
}

// ---------------------------------------------------------------------------
// Build file header and section header
// ---------------------------------------------------------------------------

pub(crate) fn build_file_header(cn_name_off: usize) -> Vec<u8> {
    let mut hdr = vec![0u8; 0x40];
    hdr[0x00..0x08].copy_from_slice(PF_MAGIC);
    write_i32_le_into(&mut hdr, 0x08, 0); // userTag
    write_i32_le_into(&mut hdr, 0x0C, PF_FILE_VERSION);
    hdr[0x10..0x14].copy_from_slice(PF_LAYOUT_RULES);
    write_i32_le_into(&mut hdr, 0x14, 3); // numSections
    write_i32_le_into(&mut hdr, 0x18, 2); // contentsSectionIndex (data=2)
    write_i32_le_into(&mut hdr, 0x1C, 0); // contentsSectionOffset
    write_i32_le_into(&mut hdr, 0x20, 0); // contentsClassNameSectionIndex
    write_i32_le_into(&mut hdr, 0x24, cn_name_off as i32);
    hdr[0x28..0x38].copy_from_slice(PF_CONTENTS_VER);
    write_i32_le_into(&mut hdr, 0x38, 0); // flags
    write_i32_le_into(&mut hdr, 0x3C, PF_MAX_PREDICATE);
    hdr
}

fn write_i32_le_into(buf: &mut Vec<u8>, off: usize, v: i32) {
    buf[off..off + 4].copy_from_slice(&v.to_le_bytes());
}

pub(crate) fn build_section_header(
    name: &str,
    abs_start: usize,
    local_fix: usize,
    global_fix: usize,
    virt_fix: usize,
    exports: usize,
) -> Vec<u8> {
    let mut hdr = vec![0xFFu8; 0x40];
    let name_b: Vec<u8> = {
        let mut b = name.as_bytes().to_vec();
        b.push(0);
        b
    };
    let n = name_b.len().min(0x14);
    hdr[..n].copy_from_slice(&name_b[..n]);
    // bytes name_b.len()..0x14 stay 0xFF as initialized
    write_u32_le_into_arr(&mut hdr, 0x14, abs_start as u32);
    write_u32_le_into_arr(&mut hdr, 0x18, (local_fix - abs_start) as u32);
    write_u32_le_into_arr(&mut hdr, 0x1C, (global_fix - abs_start) as u32);
    write_u32_le_into_arr(&mut hdr, 0x20, (virt_fix - abs_start) as u32);
    write_u32_le_into_arr(&mut hdr, 0x24, (exports - abs_start) as u32);
    write_u32_le_into_arr(&mut hdr, 0x28, (exports - abs_start) as u32); // imports
    write_u32_le_into_arr(&mut hdr, 0x2C, (exports - abs_start) as u32); // end
    // bytes 0x30..0x40 stay 0xFF as initialized
    hdr
}

fn write_u32_le_into_arr(buf: &mut Vec<u8>, off: usize, v: u32) {
    buf[off..off + 4].copy_from_slice(&v.to_le_bytes());
}

// ---------------------------------------------------------------------------
// Public builder
// ---------------------------------------------------------------------------

struct CompressedMeshPackfileInput<'a> {
    encoded_sections: &'a [EncodedCompressedMeshSection],
    object_aabb: CmAabb,
    num_primitive_keys: u32,
    bits_per_key: u32,
    max_key_value: u32,
    primitive_stores_is_flat_convex: u8,
    master_tree_nodes: &'a [u8],
    shared_vertices: &'a [u64],
    source_shape: Option<&'a RawCompressedMeshData>,
}

fn build_compressed_mesh_packfile_from_encoded(
    input: CompressedMeshPackfileInput<'_>,
    opts: BuildOptions,
) -> HavokResult<Vec<u8>> {
    validate_encoded_compressed_mesh(input.encoded_sections, input.master_tree_nodes)?;

    let (cn_data, name_offs) = build_cm_classnames();
    let cn_name_off = *name_offs
        .get("hknpPhysicsSystemData")
        .expect("hknpPhysicsSystemData classname must be present");

    let (obj_data, fx) = build_cm_data_section(
        input.encoded_sections,
        input.object_aabb,
        input.num_primitive_keys,
        input.bits_per_key,
        input.max_key_value,
        input.primitive_stores_is_flat_convex,
        input.master_tree_nodes,
        input.shared_vertices,
        input.source_shape,
        &name_offs,
        &opts,
    );

    let local_tbl = fx.build_local_table();
    let global_tbl = fx.build_global_table();
    let virt_tbl = fx.build_virtual_table();

    let mut data_section = obj_data;
    data_section.extend_from_slice(&local_tbl);
    data_section.extend_from_slice(&global_tbl);
    data_section.extend_from_slice(&virt_tbl);

    let cn_start = 0x100usize;
    let cn_end = cn_start + cn_data.len();
    let data_start = cn_end;

    let local_fix_abs =
        data_start + (data_section.len() - local_tbl.len() - global_tbl.len() - virt_tbl.len());
    let global_fix_abs = local_fix_abs + local_tbl.len();
    let virt_fix_abs = global_fix_abs + global_tbl.len();
    let data_end = virt_fix_abs + virt_tbl.len();

    let hdr = build_file_header(cn_name_off);

    let shdr0 = build_section_header(
        "__classnames__",
        cn_start,
        cn_start + cn_data.len(),
        cn_start + cn_data.len(),
        cn_start + cn_data.len(),
        cn_start + cn_data.len(),
    );
    let shdr1 = build_section_header("__types__", cn_end, cn_end, cn_end, cn_end, cn_end);
    let shdr2 = build_section_header(
        "__data__",
        data_start,
        local_fix_abs,
        global_fix_abs,
        virt_fix_abs,
        data_end,
    );

    let mut out = Vec::new();
    out.extend_from_slice(&hdr);
    out.extend_from_slice(&shdr0);
    out.extend_from_slice(&shdr1);
    out.extend_from_slice(&shdr2);
    debug_assert_eq!(out.len(), 0x100);
    out.extend_from_slice(&cn_data);
    out.extend_from_slice(&data_section);

    Ok(out)
}

fn validate_encoded_compressed_mesh(
    sections: &[EncodedCompressedMeshSection],
    master_tree_nodes: &[u8],
) -> HavokResult<()> {
    if sections.is_empty() {
        return Err(HavokError::InvalidInput(
            "compressed mesh must have at least one section".to_string(),
        ));
    }
    if master_tree_nodes.is_empty() || master_tree_nodes.len() % 5 != 0 {
        return Err(HavokError::InvalidInput(
            "compressed mesh master tree nodes must be non-empty Aabb5 records".to_string(),
        ));
    }
    let mut first_vertex = 0usize;
    let mut first_shared = 0usize;
    let mut first_primitive = 0usize;
    let mut first_data_run = 0usize;
    for (index, section) in sections.iter().enumerate() {
        if section.packed_vertices.len() > u8::MAX as usize {
            return Err(HavokError::InvalidInput(format!(
                "compressed mesh section {index} has {} packed vertices; max 255",
                section.packed_vertices.len()
            )));
        }
        if section.shared_vertices_index.len() > u8::MAX as usize {
            return Err(HavokError::InvalidInput(format!(
                "compressed mesh section {index} has {} shared indices; max 255",
                section.shared_vertices_index.len()
            )));
        }
        if section.primitive_bytes.is_empty() || section.primitive_bytes.len() % 4 != 0 {
            return Err(HavokError::InvalidInput(format!(
                "compressed mesh section {index} primitive data must be non-empty 4-byte records"
            )));
        }
        if section.primitive_bytes.len() / 4 > 128 {
            return Err(HavokError::InvalidInput(format!(
                "compressed mesh section {index} has {} primitives; max 128 (7-bit shape key)",
                section.primitive_bytes.len() / 4
            )));
        }
        if section.section_tree_nodes.is_empty() || section.section_tree_nodes.len() % 4 != 0 {
            return Err(HavokError::InvalidInput(format!(
                "compressed mesh section {index} tree nodes must be non-empty Aabb4 records"
            )));
        }
        if section.primitive_data_runs.is_empty()
            || section.primitive_data_runs.len() > u8::MAX as usize
        {
            return Err(HavokError::InvalidInput(format!(
                "compressed mesh section {index} must have 1..255 primitive data runs"
            )));
        }
        let primitive_count = section.primitive_bytes.len() / 4;
        let mut covered_primitives = 0usize;
        for (run_index, run) in section.primitive_data_runs.iter().enumerate() {
            if run.count == 0 {
                return Err(HavokError::InvalidInput(format!(
                    "compressed mesh section {index} primitive data run {run_index} has zero count"
                )));
            }
            if usize::from(run.index) != covered_primitives {
                return Err(HavokError::InvalidInput(format!(
                    "compressed mesh section {index} primitive data run {run_index} starts at {} but expected {covered_primitives}",
                    run.index
                )));
            }
            covered_primitives += usize::from(run.count);
            if covered_primitives > primitive_count {
                return Err(HavokError::InvalidInput(format!(
                    "compressed mesh section {index} primitive data runs cover {covered_primitives} primitives but the section has {primitive_count}"
                )));
            }
        }
        if covered_primitives != primitive_count {
            return Err(HavokError::InvalidInput(format!(
                "compressed mesh section {index} primitive data runs cover {covered_primitives} primitives but the section has {primitive_count}"
            )));
        }
        if first_vertex > 0x00FF_FFFF
            || first_shared > 0x00FF_FFFF
            || first_primitive > 0x00FF_FFFF
            || first_data_run > 0x00FF_FFFF
        {
            return Err(HavokError::InvalidInput(
                "compressed mesh section offset exceeds 24-bit Havok section field".to_string(),
            ));
        }
        first_vertex += section.packed_vertices.len();
        first_shared += section.shared_vertices_index.len();
        first_primitive += section.primitive_bytes.len() / 4;
        first_data_run += section.primitive_data_runs.len();
    }
    Ok(())
}

pub fn build_compressed_mesh_collision(
    verts: &[[f32; 3]],
    tris: &[[u32; 3]],
    opts: BuildOptions,
) -> HavokResult<Vec<u8>> {
    let nv = verts.len();
    let nt = tris.len();
    if nv == 0 || nt == 0 {
        return Err(HavokError::InvalidInput(
            "vertices and triangles must be non-empty".to_string(),
        ));
    }
    validate_vertices(verts)?;
    for (i, tri) in tris.iter().enumerate() {
        validate_compressed_triangle(verts, tri, i)?;
    }
    let sections = split_compressed_mesh_sections(verts, tris)?;
    if sections.is_empty() {
        return Err(HavokError::InvalidInput(
            "compressed mesh split produced no sections".to_string(),
        ));
    }
    let mut encoded_sections = sections
        .iter()
        .map(encode_compressed_mesh_section)
        .collect::<Vec<_>>();
    let section_aabbs = encoded_sections
        .iter()
        .map(|section| section.aabb)
        .collect::<Vec<_>>();
    let section_indices = (0..section_aabbs.len()).collect::<Vec<_>>();
    let object_aabb = merge_aabbs(&section_aabbs, &section_indices);
    let (master_tree_nodes, section_leaf_indices) =
        build_master_tree_nodes(&section_aabbs, &object_aabb);
    for (section, leaf_index) in encoded_sections.iter_mut().zip(section_leaf_indices) {
        section.leaf_index = leaf_index;
    }
    // hkcdStaticMeshTree composes a primitive shape key as
    //   (sectionIndex << 8) | (localPrimitiveIndex << 1) | triangleBit
    // so the max key lives in the LAST section, and m_numShapeKeyBits /
    // m_bitsPerKey = numBits(maxKey). A flat global-primitive index undershoots
    // for any multi-section mesh -> FO4 recovers a garbage section index and
    // walks off m_sections (the r10=0x2050204 narrowphase CTD).
    let last_section = encoded_sections.last().expect("at least one section");
    let last_section_primitives = (last_section.primitive_bytes.len() / 4) as u32;
    let last_primitive_is_two_triangles = last_section
        .primitive_bytes
        .chunks_exact(4)
        .last()
        .is_some_and(|quad| quad[2] != quad[3]);
    let last_section_index = (encoded_sections.len() - 1) as u32;
    let max_key_value = (last_section_index << 8)
        | (last_section_primitives.saturating_sub(1) << 1)
        | u32::from(last_primitive_is_two_triangles);
    let num_primitive_keys = max_key_value + 1;
    let bits_per_key = ceil_log2_u32(num_primitive_keys).max(1);

    build_compressed_mesh_packfile_from_encoded(
        CompressedMeshPackfileInput {
            encoded_sections: &encoded_sections,
            object_aabb,
            num_primitive_keys,
            bits_per_key,
            max_key_value,
            primitive_stores_is_flat_convex: 0,
            master_tree_nodes: &master_tree_nodes,
            shared_vertices: &[],
            source_shape: None,
        },
        opts,
    )
}

pub fn build_compressed_mesh_collision_from_raw(
    raw: &RawCompressedMeshData,
    mut opts: BuildOptions,
) -> HavokResult<Vec<u8>> {
    validate_raw_shape_metadata(raw)?;
    opts.user_data = Some(raw.user_data);
    if !raw.materials.is_empty() {
        opts.materials = raw
            .materials
            .iter()
            .map(|material| MaterialEntry {
                filter_info: (material.filter_info & !0xFF) | u32::from(opts.layer),
                material_crc: material.material_crc,
            })
            .collect();
    }
    let encoded_sections = raw
        .sections
        .iter()
        .map(|section| EncodedCompressedMeshSection {
            aabb: CmAabb {
                min: section.aabb_min,
                max: section.aabb_max,
            },
            base: section.base,
            scale: section.scale,
            packed_vertices: section.packed_vertices.clone(),
            shared_vertices_index: section.shared_vertices_index.clone(),
            primitive_bytes: section.primitive_bytes.clone(),
            section_tree_nodes: section.section_tree_nodes.clone(),
            primitive_data_runs: section.primitive_data_runs.clone(),
            leaf_index: section.leaf_index,
            page: section.page,
            flags: section.flags,
            layer_data: section.layer_data,
            unused_data: section.unused_data,
        })
        .collect::<Vec<_>>();

    build_compressed_mesh_packfile_from_encoded(
        CompressedMeshPackfileInput {
            encoded_sections: &encoded_sections,
            object_aabb: CmAabb {
                min: raw.object_aabb_min,
                max: raw.object_aabb_max,
            },
            num_primitive_keys: raw.num_primitive_keys,
            bits_per_key: raw.bits_per_key,
            max_key_value: raw.max_key_value,
            primitive_stores_is_flat_convex: raw.primitive_stores_is_flat_convex,
            master_tree_nodes: &raw.master_tree_nodes,
            shared_vertices: &raw.shared_vertices,
            source_shape: Some(raw),
        },
        opts,
    )
}

fn validate_raw_shape_metadata(raw: &RawCompressedMeshData) -> HavokResult<()> {
    for (name, bitfield) in [
        ("quadIsFlat", &raw.quad_is_flat),
        ("triangleIsInterior", &raw.triangle_is_interior),
    ] {
        if bitfield.num_bits > 0 && bitfield.words.len() != bitfield_word_count(bitfield.num_bits) {
            return Err(HavokError::InvalidInput(format!(
                "raw compressed mesh {name} has {} words for {} bits",
                bitfield.words.len(),
                bitfield.num_bits
            )));
        }
    }
    if raw.edge_welding_map.primary_key_to_index.is_empty()
        != raw.edge_welding_map.value_and_secondary_keys.is_empty()
    {
        return Err(HavokError::InvalidInput(
            "raw compressed mesh edge welding map has only one populated array".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_mesh() -> (Vec<[f32; 3]>, Vec<[u32; 3]>) {
        (
            vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            vec![[0, 1, 2], [0, 2, 3]],
        )
    }

    fn encoded_section_with_primitives(primitive_bytes: Vec<u8>) -> EncodedCompressedMeshSection {
        EncodedCompressedMeshSection {
            aabb: CmAabb {
                min: [0.0, 0.0, 0.0],
                max: [1.0, 1.0, 1.0],
            },
            base: [0.0, 0.0, 0.0],
            scale: [1.0, 1.0, 1.0],
            packed_vertices: vec![0; 4],
            shared_vertices_index: Vec::new(),
            primitive_data_runs: vec![RawCompressedMeshDataRun {
                value: 0,
                index: 0,
                count: (primitive_bytes.len() / 4) as u8,
            }],
            primitive_bytes,
            section_tree_nodes: vec![0; 4],
            leaf_index: 0,
            page: 0,
            flags: 0,
            layer_data: 0,
            unused_data: 0,
        }
    }

    fn first_bitfield_word(storage: &[u8]) -> u32 {
        u32::from_le_bytes(storage[..4].try_into().unwrap())
    }

    #[test]
    fn rebuilt_quad_bitfield_does_not_mark_triangles() {
        let section = encoded_section_with_primitives(vec![
            0, 3, 2, 1, // quad
            0, 1, 2, 2, // triangle
        ]);

        let storage = quad_is_flat_bitfield_storage(&[section], 2, 0);

        assert_eq!(first_bitfield_word(&storage) & 0b11, 0b01);
    }

    #[test]
    fn raw_quad_bitfield_translates_flat_convex_marker() {
        let section = encoded_section_with_primitives(vec![
            0, 1, 2, 3, // non-flat quad
            0, 3, 2, 1, // flat quad (b > d)
            0, 1, 2, 2, // triangle
        ]);

        let storage = quad_is_flat_bitfield_storage(&[section], 3, u8::MAX);

        assert_eq!(first_bitfield_word(&storage) & 0b111, 0b010);
    }

    #[test]
    fn quad_bitfield_uses_shape_key_section_stride() {
        let triangle = encoded_section_with_primitives(vec![0, 1, 2, 2]);
        let quad = encoded_section_with_primitives(vec![0, 3, 2, 1]);

        let storage = quad_is_flat_bitfield_storage(&[triangle, quad], 129, 0);
        let section_one_word = u32::from_le_bytes(storage[16..20].try_into().unwrap());

        assert_eq!(first_bitfield_word(&storage), 0);
        assert_eq!(section_one_word & 1, 1);
    }

    #[test]
    fn flat_triangle_pair_uses_sdk_flat_quad_order() {
        let (verts, tris) = test_mesh();
        let primitive_bytes = encode_triangle_primitive_bytes(&verts, &tris);

        assert_eq!(primitive_bytes.len(), 4);
        assert_ne!(primitive_bytes[2], primitive_bytes[3]);
        assert!(
            primitive_bytes[1] > primitive_bytes[3],
            "hkcdStaticMeshTree marks a primitive as flat-convex by b>d"
        );
    }

    #[test]
    fn non_coplanar_triangle_pair_stays_split() {
        let verts = vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 1.0],
        ];
        let tris = vec![[0, 1, 2], [0, 2, 3]];
        let primitive_bytes = encode_triangle_primitive_bytes(&verts, &tris);

        assert_eq!(primitive_bytes.len(), 8);
        for primitive in primitive_bytes.chunks_exact(4) {
            assert_eq!(primitive[2], primitive[3]);
        }
    }

    #[test]
    fn compressed_mesh_writer_emits_havok_section_metadata() {
        let (verts, tris) = test_mesh();
        let blob = build_compressed_mesh_collision(&verts, &tris, BuildOptions::default())
            .expect("compressed mesh should build");

        let hdrs = parse_packfile_section_headers(&blob).expect("section headers");
        let data_hdr = hdrs.get("__data__").expect("__data__ section");
        let data_start = data_hdr.abs_start;
        let classnames_start = hdrs.get("__classnames__").unwrap().abs_start;
        let fixups = parse_local_fixups(&blob, data_hdr).expect("local fixups");
        let objects =
            parse_virtual_fixups(&blob, data_hdr, classnames_start).expect("virtual fixups");

        let shape_rel = objects
            .iter()
            .find(|(_, class_name)| class_name == "hknpCompressedMeshShape")
            .map(|(rel, _)| *rel)
            .expect("compressed mesh shape");
        let shape_abs = data_start + shape_rel;
        assert_eq!(u8_at(&blob, shape_abs + 0x12).unwrap(), 1);
        assert_eq!(u32_le(&blob, shape_abs + 0x58).unwrap(), u32::MAX);
        // triangleIsInterior is sized to max_key_value + 1. This two-triangle
        // quad is one paired primitive, so the valid keys are 0 and 1.
        assert_eq!(u32_le(&blob, shape_abs + 0x90).unwrap(), 2);

        let mesh_rel = objects
            .iter()
            .find(|(_, class_name)| class_name == "hknpCompressedMeshShapeData")
            .map(|(rel, _)| *rel)
            .expect("compressed mesh data");
        let mesh_abs = data_start + mesh_rel;
        assert_eq!(hkarray_size(&blob, mesh_abs, 0x10).unwrap(), 1);
        assert!(fixups.contains_key(&(mesh_rel + 0x10)));
        assert_eq!(u32_le(&blob, mesh_abs + 0x40).unwrap(), 2);
        assert_eq!(u32_le(&blob, mesh_abs + 0x44).unwrap(), 1);
        assert_eq!(u32_le(&blob, mesh_abs + 0x48).unwrap(), 1);
        assert_eq!(hkarray_size(&blob, mesh_abs, 0x50).unwrap(), 1);
        assert_eq!(hkarray_size(&blob, mesh_abs, 0x60).unwrap(), 1);
        assert_eq!(hkarray_size(&blob, mesh_abs, 0x80).unwrap(), 4);
        assert_eq!(hkarray_size(&blob, mesh_abs, 0xA0).unwrap(), 1);

        let material_rel = objects
            .iter()
            .find(|(_, class_name)| class_name == "hknpBSMaterialProperties")
            .map(|(rel, _)| *rel)
            .expect("material properties");
        assert_eq!(
            fixups.get(&(material_rel + 0x10)).copied(),
            Some(material_rel + 0x20)
        );

        let section_rel = *fixups.get(&(mesh_rel + 0x50)).expect("section fixup");
        let section_abs = data_start + section_rel;
        assert_eq!(hkarray_size(&blob, section_abs, 0x00).unwrap(), 1);
        assert!(fixups.contains_key(&(section_rel + 0x00)));
        assert_eq!(u32_le(&blob, section_abs + SEC_VERT_PACKED).unwrap(), 4);
        assert_eq!(u32_le(&blob, section_abs + SEC_QUAD_PACKED).unwrap(), 1);
        assert_eq!(u32_le(&blob, section_abs + SEC_DATA_RUN_PACKED).unwrap(), 1);
        assert_eq!(
            u8_at(&blob, section_abs + SEC_NUM_PACKED_VERTICES).unwrap(),
            4
        );
        assert_eq!(
            u8_at(&blob, section_abs + SEC_NUM_SHARED_INDICES).unwrap(),
            0
        );

        let run_rel = *fixups
            .get(&(mesh_rel + 0xA0))
            .expect("primitive data run fixup");
        let run_abs = data_start + run_rel;
        assert_eq!(u16_le(&blob, run_abs).unwrap(), 0);
        assert_eq!(u8_at(&blob, run_abs + 2).unwrap(), 0);
        assert_eq!(u8_at(&blob, run_abs + 3).unwrap(), 1);

        let parsed = parse_fo4_compressed_mesh(&blob).expect("parse generated mesh");
        assert_eq!(parsed.sections.len(), 1);
        assert_eq!(parsed.sections[0].vertices.len(), 4);
        assert_eq!(parsed.sections[0].triangles.len(), 2);
    }

    #[test]
    fn raw_data_run_round_trips_sdk_u16_u8_u8_layout() {
        let verts = vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 1.0],
        ];
        let tris = vec![[0, 1, 2], [0, 2, 3]];
        let source_blob = build_compressed_mesh_collision(&verts, &tris, BuildOptions::default())
            .expect("source compressed mesh should build");
        let mut raw =
            crate::collision::preview::extract_raw_compressed_meshes_from_blob(&source_blob, None)
                .expect("source raw compressed mesh should parse")
                .into_iter()
                .next()
                .expect("source raw compressed mesh missing");

        assert_eq!(raw.sections[0].primitive_bytes.len() / 4, 2);
        let expected = vec![
            RawCompressedMeshDataRun {
                value: 0xBEEF,
                index: 0,
                count: 1,
            },
            RawCompressedMeshDataRun {
                value: u16::MAX,
                index: 1,
                count: 1,
            },
        ];
        raw.sections[0].primitive_data_runs = expected.clone();
        let blob = build_compressed_mesh_collision_from_raw(&raw, BuildOptions::default())
            .expect("raw compressed mesh should rebuild");

        let hdrs = parse_packfile_section_headers(&blob).expect("section headers");
        let data_hdr = hdrs.get("__data__").expect("__data__ section");
        let data_start = data_hdr.abs_start;
        let classnames_start = hdrs.get("__classnames__").unwrap().abs_start;
        let fixups = parse_local_fixups(&blob, data_hdr).expect("local fixups");
        let objects =
            parse_virtual_fixups(&blob, data_hdr, classnames_start).expect("virtual fixups");
        let mesh_rel = objects
            .iter()
            .find(|(_, class_name)| class_name == "hknpCompressedMeshShapeData")
            .map(|(rel, _)| *rel)
            .expect("compressed mesh shape data");
        let run_abs =
            hkarray_abs(&fixups, data_start, mesh_rel, 0xA0).expect("primitive data run pointer");

        assert_eq!(
            &blob[run_abs..run_abs + 8],
            &[0xEF, 0xBE, 0x00, 0x01, 0xFF, 0xFF, 0x01, 0x01]
        );

        let round_trip =
            crate::collision::preview::extract_raw_compressed_meshes_from_blob(&blob, None)
                .expect("rebuilt raw compressed mesh should parse")
                .into_iter()
                .next()
                .expect("rebuilt raw compressed mesh missing");
        assert_eq!(round_trip.sections[0].primitive_data_runs, expected);
    }

    #[test]
    fn compressed_mesh_writer_splits_oversized_mesh_into_sections() {
        // 260 independent right-triangles, each with its own 3 vertices (780 total),
        // guaranteed non-degenerate (area^2 = 0.25 >> 1e-7). Forces section splits
        // because 780 verts exceed MAX_SECTION_VERTICES (255) per section.
        let num_tris: u32 = 260;
        let mut verts = Vec::with_capacity((num_tris * 3) as usize);
        let mut tris = Vec::with_capacity(num_tris as usize);
        for i in 0..num_tris {
            let base = verts.len() as u32;
            let x = i as f32;
            verts.push([x, 0.0, 0.0]);
            verts.push([x + 1.0, 0.0, 0.0]);
            verts.push([x, 1.0, 0.0]);
            tris.push([base, base + 1, base + 2]);
        }

        let blob = build_compressed_mesh_collision(&verts, &tris, BuildOptions::default())
            .expect("oversized compressed mesh should build");
        let parsed = parse_fo4_compressed_mesh(&blob).expect("parse generated mesh");

        assert!(parsed.sections.len() > 1);
        assert_eq!(
            parsed
                .sections
                .iter()
                .map(|section| section.triangles.len())
                .sum::<usize>(),
            tris.len()
        );
        for section in &parsed.sections {
            assert!(section.vertices.len() <= MAX_SECTION_VERTICES);
            assert!(section.triangles.len() <= MAX_SECTION_TRIANGLES);
        }
    }

    #[test]
    fn compressed_mesh_multi_section_shape_key_bits_encode_section_index() {
        // 255 fully-independent triangles (no shared vertices) force the
        // 255-vertex section limit to split the mesh into 3 sections of 85
        // primitives each. hkcdStaticMeshTree composes a primitive shape key as
        //   (sectionIndex << 8) | (localPrimitiveIndex << 1) | triangleBit
        // so m_numShapeKeyBits must be 8 + numBits(numSections - 1) = 10 — NOT
        // the flat numBits(totalPrimitives * 2) = 9. With the flat value FO4
        // right-shifts each hit key by the wrong amount, recovers a garbage
        // section index, and dereferences m_sections off the end (the
        // r10=0x2050204 narrowphase CTD).
        let mut verts = Vec::new();
        let mut tris = Vec::new();
        for i in 0..255u32 {
            let base = verts.len() as u32;
            let x = i as f32;
            verts.push([x, 0.0, 0.0]);
            verts.push([x + 0.25, 0.0, 0.0]);
            verts.push([x, 0.25, 0.0]);
            tris.push([base, base + 1, base + 2]);
        }

        let blob = build_compressed_mesh_collision(&verts, &tris, BuildOptions::default())
            .expect("multi-section compressed mesh should build");

        let parsed = parse_fo4_compressed_mesh(&blob).expect("parse generated mesh");
        let num_sections = parsed.sections.len();
        assert!(
            num_sections >= 3,
            "expected >= 3 sections from 255 independent triangles, got {num_sections}"
        );

        let hdrs = parse_packfile_section_headers(&blob).expect("section headers");
        let data_hdr = hdrs.get("__data__").expect("__data__ section");
        let data_start = data_hdr.abs_start;
        let classnames_start = hdrs.get("__classnames__").unwrap().abs_start;
        let objects =
            parse_virtual_fixups(&blob, data_hdr, classnames_start).expect("virtual fixups");
        let shape_rel = objects
            .iter()
            .find(|(_, class_name)| class_name == "hknpCompressedMeshShape")
            .map(|(rel, _)| *rel)
            .expect("compressed mesh shape");
        let num_shape_key_bits = u8_at(&blob, data_start + shape_rel + 0x12).unwrap() as u32;

        let section_index_bits = u32::BITS - (num_sections as u32 - 1).leading_zeros();
        let expected = 8 + section_index_bits;
        assert_eq!(
            num_shape_key_bits, expected,
            "numShapeKeyBits must encode the section index in the high bits \
             (8 + numBits(numSections-1)); got {num_shape_key_bits}, expected {expected} \
             for {num_sections} sections"
        );
    }

    #[test]
    fn compressed_mesh_master_tree_uses_aabb5_codec_convention() {
        // The master tree over Sections is encoded with
        // hkcdCompressedAabbCodecs::Aabb5BytesCodec. Its 5-byte node is laid out
        // [x, y, z, m_hiData, m_loData] and the engine reads it as:
        //   isInternal = m_hiData & 0x80
        //   leaf      -> sectionIndex = (m_hiData << 8) | m_loData     (high bit clear)
        //   internal  -> rightChild  = node + ((((m_hiData & 0x7F) << 8) | m_loData) << 1)
        // The Aabb4 convention (data&1 == internal, data>>1 == payload) does NOT
        // apply to the master codec. Encoding it that way makes FO4 read the
        // internal root as a leaf pointing at a nonexistent section, deref
        // m_sections out of bounds, and CTD on the first physics query against a
        // multi-section mesh (single-section meshes survive only because their
        // lone leaf is section 0, where both conventions coincide).
        let mut verts = Vec::new();
        let mut tris = Vec::new();
        for i in 0..255u32 {
            let base = verts.len() as u32;
            let x = i as f32;
            verts.push([x, 0.0, 0.0]);
            verts.push([x + 0.25, 0.0, 0.0]);
            verts.push([x, 0.25, 0.0]);
            tris.push([base, base + 1, base + 2]);
        }

        let blob = build_compressed_mesh_collision(&verts, &tris, BuildOptions::default())
            .expect("multi-section compressed mesh should build");

        let hdrs = parse_packfile_section_headers(&blob).expect("section headers");
        let data_hdr = hdrs.get("__data__").expect("__data__ section");
        let data_start = data_hdr.abs_start;
        let classnames_start = hdrs.get("__classnames__").unwrap().abs_start;
        let fixups = parse_local_fixups(&blob, data_hdr).expect("local fixups");
        let objects =
            parse_virtual_fixups(&blob, data_hdr, classnames_start).expect("virtual fixups");

        let obj_rel = objects
            .iter()
            .find(|(_, class_name)| class_name == "hknpCompressedMeshShapeData")
            .map(|(rel, _)| *rel)
            .expect("compressed mesh shape data");
        let obj_abs = data_start + obj_rel;

        let num_sections = hkarray_size(&blob, obj_abs, 0x50).unwrap();
        assert!(num_sections >= 3, "test needs a multi-section mesh");

        let master_abs =
            hkarray_abs(&fixups, data_start, obj_rel, 0x10).expect("master tree m_nodes pointer");
        let master_count = hkarray_size(&blob, obj_abs, 0x10).unwrap();
        assert_eq!(
            master_count,
            2 * num_sections - 1,
            "a binary tree over N sections has exactly 2N-1 nodes"
        );

        // Walk the tree from the root; every leaf must map to a unique, in-range
        // section, and every section must be reachable.
        let mut leaf_node_of = vec![usize::MAX; num_sections];
        let mut stack = vec![0usize];
        let mut visited = 0usize;
        while let Some(n) = stack.pop() {
            visited += 1;
            assert!(visited <= master_count, "master tree traversal cycled");
            let o = master_abs + n * 5;
            let hi = u8_at(&blob, o + 3).unwrap();
            let lo = u8_at(&blob, o + 4).unwrap();
            if hi & 0x80 != 0 {
                let delta = ((((hi & 0x7F) as usize) << 8) | lo as usize) << 1;
                assert!(
                    delta >= 2 && n + delta < master_count,
                    "internal node {n} right-delta {delta} out of range (count {master_count})"
                );
                stack.push(n + 1);
                stack.push(n + delta);
            } else {
                let section = ((hi as usize) << 8) | lo as usize;
                assert!(
                    section < num_sections,
                    "leaf node {n} references section {section} >= {num_sections}: \
                     FO4 dereferences m_sections out of bounds here and CTDs"
                );
                assert!(
                    leaf_node_of[section] == usize::MAX,
                    "section {section} referenced by two leaves"
                );
                leaf_node_of[section] = n;
            }
        }
        assert!(
            leaf_node_of.iter().all(|&n| n != usize::MAX),
            "every section must be reachable as a master-tree leaf"
        );

        // Section::m_leafIndex (struct 0x5A) must equal the master-tree node index
        // of that section's leaf, as vanilla FO4 stores it.
        let sections_abs =
            hkarray_abs(&fixups, data_start, obj_rel, 0x50).expect("sections pointer");
        for (section, &node) in leaf_node_of.iter().enumerate() {
            let leaf_index = u16_le(&blob, sections_abs + section * SECTION_STRIDE + 0x5A).unwrap();
            assert_eq!(
                leaf_index as usize, node,
                "section {section} m_leafIndex={leaf_index} but its leaf is master node {node}"
            );
        }
    }

    #[test]
    fn compressed_mesh_rejects_indices_outside_vertex_array() {
        let verts = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let tris = vec![[0, 1, 3]];
        let err = build_compressed_mesh_collision(&verts, &tris, BuildOptions::default())
            .expect_err("invalid triangle index should fail");
        assert!(format!("{err}").contains("references vertex 3"));
    }

    #[test]
    fn rejects_sliver_triangle_below_sdk_tolerance() {
        // A near-collinear sliver whose area^2 (cross.lengthSquared) sits in the
        // band (1e-12, 1e-7) that the old 1e-12 floor accepted but the SDK rejects.
        let verts = vec![[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.5, 1.0e-4, 0.0]];
        let tri = [0u32, 1, 2];
        let area_sq = triangle_area_squared(&verts, &tri).unwrap();
        assert!(
            area_sq > 1.0e-12 && area_sq < 1.0e-7,
            "fixture must land in band, got {area_sq}"
        );
        assert!(
            validate_compressed_triangle(&verts, &tri, 0).is_err(),
            "sliver below SDK tolerance must be rejected"
        );
    }

    #[test]
    fn keeps_valid_small_triangle_above_sdk_tolerance() {
        let verts = vec![[0.0f32, 0.0, 0.0], [0.1, 0.0, 0.0], [0.0, 0.1, 0.0]];
        assert!(validate_compressed_triangle(&verts, &[0, 1, 2], 0).is_ok());
    }

    /// Returns the minimal valid `EncodedCompressedMeshSection` that passes all
    /// `validate_encoded_compressed_mesh` checks *except* the one under test.
    /// Fields: 1 packed vertex (≤255), 0 shared indices (≤255), 1 primitive
    /// (4 bytes, ≤128), 4-byte tree node (non-empty, multiple of 4), 1 data run.
    fn test_minimal_encoded_section() -> EncodedCompressedMeshSection {
        EncodedCompressedMeshSection {
            aabb: CmAabb {
                min: [0.0, 0.0, 0.0],
                max: [1.0, 1.0, 1.0],
            },
            base: [0.0, 0.0, 0.0],
            scale: [1.0, 1.0, 1.0],
            packed_vertices: vec![0u32],
            shared_vertices_index: Vec::new(),
            primitive_bytes: vec![0u8; 4],    // 1 primitive
            section_tree_nodes: vec![0u8; 4], // 1 Aabb4 node (4 bytes)
            primitive_data_runs: vec![RawCompressedMeshDataRun {
                value: 0,
                index: 0,
                count: 1,
            }],
            leaf_index: 0,
            page: 0,
            flags: 0,
            layer_data: 0,
            unused_data: 0,
        }
    }

    #[test]
    fn rejects_section_with_more_than_128_primitives() {
        // 129 primitives -> primitiveIndex overflows the 7-bit shape-key field.
        let section = EncodedCompressedMeshSection {
            primitive_bytes: vec![0u8; 129 * 4], // 129 primitives
            primitive_data_runs: vec![RawCompressedMeshDataRun {
                value: 0,
                index: 0,
                count: 129,
            }],
            ..test_minimal_encoded_section()
        };
        assert!(
            validate_encoded_compressed_mesh(&[section], &[0u8; 5]).is_err(),
            "129 primitives must be rejected (7-bit key max 128)"
        );
        // 128 primitives must be accepted.
        let section_ok = EncodedCompressedMeshSection {
            primitive_bytes: vec![0u8; 128 * 4], // 128 primitives
            primitive_data_runs: vec![RawCompressedMeshDataRun {
                value: 0,
                index: 0,
                count: 128,
            }],
            ..test_minimal_encoded_section()
        };
        assert!(
            validate_encoded_compressed_mesh(&[section_ok], &[0u8; 5]).is_ok(),
            "128 primitives must be accepted (7-bit key max 128)"
        );
    }

    #[test]
    fn rejects_primitive_data_run_gaps_and_zero_counts() {
        let gap = EncodedCompressedMeshSection {
            primitive_data_runs: vec![RawCompressedMeshDataRun {
                value: 0,
                index: 1,
                count: 1,
            }],
            ..test_minimal_encoded_section()
        };
        let gap_error = validate_encoded_compressed_mesh(&[gap], &[0u8; 5])
            .expect_err("a data-run gap must be rejected");
        assert!(format!("{gap_error}").contains("starts at 1 but expected 0"));

        let zero_count = EncodedCompressedMeshSection {
            primitive_data_runs: vec![RawCompressedMeshDataRun {
                value: 0,
                index: 0,
                count: 0,
            }],
            ..test_minimal_encoded_section()
        };
        let zero_error = validate_encoded_compressed_mesh(&[zero_count], &[0u8; 5])
            .expect_err("a zero-count data run must be rejected");
        assert!(format!("{zero_error}").contains("has zero count"));
    }

    fn decoded_triangle_count(decoded: &CompressedMeshData) -> usize {
        decoded
            .sections
            .iter()
            .map(|section| section.triangles.len())
            .sum()
    }

    #[test]
    fn drops_triangle_that_collapses_after_quantization() {
        // Wide section AABB makes the 11-bit grid step coarse (~2 units/step on X).
        // v0 and v3 are 1.0 apart -> they quantize to the same cell -> zero area.
        let span = 4096.0f32;
        let verts = vec![
            [0.0f32, 0.0, 0.0],
            [span, 0.0, 0.0],
            [span, span, 0.0],
            [1.0, 0.0, 0.0],
        ];
        let tris = vec![[0u32, 1, 2], [0, 3, 2]]; // 2nd triangle collapses post-quant
        let blob =
            build_compressed_mesh_collision(&verts, &tris, BuildOptions::default()).expect("build");
        let decoded = parse_fo4_compressed_mesh(&blob).expect("decode");
        // Fewer than the 2 input triangles survive (the collapsed one is dropped).
        let surviving = decoded_triangle_count(&decoded);
        assert!(
            surviving < 2,
            "post-quant-degenerate triangle must be dropped, got {surviving}"
        );
    }
}
