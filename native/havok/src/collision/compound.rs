use super::aabb_tree::{
    Aabb, CODEC32_MAX_LEAVES, build_aabb_tree_nodes, codec32_counts_fit, leaf_node_indices,
};
use super::compressed_mesh::{
    BuildOptions, FixupBuilder, PF_COMPOUND_MESH_CLASS_ENTRIES, PF_COMPOUND_POLY_CLASS_ENTRIES,
    build_body_cinfo, build_body_props_with_raw, build_classnames, build_file_header,
    build_section_header, hkarray,
};
use super::polytope::SourcePolytopeShape;
/// FO4 Havok 2014.1.0 hknpDynamicCompoundShape packfile builder.
///
/// Produces the binary blob that goes into bhkPhysicsSystem.Binary Data for FO4
/// NIFs using a compound shape (multiple sub-shapes under one rigid body).
///
/// Uses shared scaffolding from `compressed_mesh.rs` (file header, section
/// headers, classnames builder, fixup tables, body_props / body_cinfo).
use crate::error::HavokResult;

// ---------------------------------------------------------------------------
// Constants duplicated from compressed_mesh (private there; defined locally)
// ---------------------------------------------------------------------------

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

fn bitfield_word_count(num_bits: u32) -> usize {
    num_bits.div_ceil(32) as usize
}

fn write_bitfield_header(buf: &mut Vec<u8>, off: usize, num_bits: u32) {
    let word_count = bitfield_word_count(num_bits) as u32;
    write_u32_le(buf, off + 0x08, word_count);
    write_u32_le(buf, off + 0x0C, word_count | 0x8000_0000);
    write_u32_le(buf, off + 0x10, num_bits);
}

fn filled_bitfield_storage(num_bits: u32) -> Vec<u8> {
    let word_count = bitfield_word_count(num_bits);
    let mut data = Vec::with_capacity(word_count * 4);
    for word_index in 0..word_count {
        let used_bits = (word_index as u32) * 32;
        let remaining = num_bits.saturating_sub(used_bits);
        let word = if remaining >= 32 {
            u32::MAX
        } else if remaining == 0 {
            0
        } else {
            (1u32 << remaining) - 1
        };
        data.extend_from_slice(&word.to_le_bytes());
    }
    data
}

fn max_primitive_key_value(primitive_bytes: &[u8]) -> u32 {
    let primitive_count = (primitive_bytes.len() / 4) as u32;
    if primitive_count == 0 {
        return 0;
    }
    let last = &primitive_bytes[primitive_bytes.len() - 4..];
    ((primitive_count - 1) << 1) | u32::from(last[2] != last[3])
}

// ---------------------------------------------------------------------------
// Public compound child type
// ---------------------------------------------------------------------------

/// Kind of sub-shape inside a compound.
#[derive(Debug, Clone, PartialEq)]
pub enum CompoundChildKind {
    /// Convex polytope — hull computed from vertices.
    Polytope { vertices: Vec<[f32; 3]> },
    /// Convex polytope with source-provided topology.
    SourcePolytope { shape: SourcePolytopeShape },
    /// Compressed triangle mesh — vertices + pre-supplied triangles.
    CompressedMesh {
        vertices: Vec<[f32; 3]>,
        triangles: Vec<[u32; 3]>,
    },
}

/// One sub-shape entry in an hknpDynamicCompoundShape.
#[derive(Debug, Clone, PartialEq)]
pub struct CompoundChild {
    /// Row-major 4×4 local-to-compound transform.  Use identity if no offset.
    pub transform: [[f32; 4]; 4],
    pub kind: CompoundChildKind,
}

impl CompoundChild {
    /// Identity transform convenience.
    pub fn identity_transform() -> [[f32; 4]; 4] {
        [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ]
    }
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

// hknpDynamicCompoundShape flags / dispatch observed in vanilla
const COMPOUND_FLAGS: u8 = 0x04;
const COMPOUND_DISPATCH: u8 = 0x02;

// hknpConvexPolytopeShape flags / dispatch observed in vanilla
const POLYTOPE_FLAGS: u8 = 0x43;
const POLYTOPE_DISPATCH: u16 = 0x0100;

// hknpShapeInstance flag bits (hknpShapeInstance.h::FlagsEnum).
// Stored in `hkVector4::setInt24W(flags)` → float bits = 0x3F000000 | flags.
// See hknpShapeInstance.inl::setFlags / getFlags.
pub const SHAPE_INST_HAS_TRANSLATION: u32 = 1 << 1; // 0x02
pub const SHAPE_INST_HAS_ROTATION: u32 = 1 << 2; // 0x04
pub const SHAPE_INST_HAS_SCALE: u32 = 1 << 3; // 0x08
pub const SHAPE_INST_DEPRECATED: u32 = 1 << 4; // 0x10
pub const SHAPE_INST_SCALE_SURFACE: u32 = 1 << 5; // 0x20
pub const SHAPE_INST_IS_ENABLED: u32 = 1 << 6; // 0x40

/// Pack shape-instance flags into the hkVector4 W-component encoding used by
/// `hkVector4::setInt24W` — high byte is always 0x3F, bottom 24 bits hold flags.
pub fn pack_inst_row_w(flags: u32) -> u32 {
    0x3F00_0000 | (flags & 0x00FF_FFFF)
}

const COMPRESSED_MESH_INSTANCE_SIZE: usize = 0x90;

fn align16(v: usize) -> usize {
    (v + 15) & !15
}

// Instance stride: 122-byte struct padded to 128 bytes
const INST_STRIDE: usize = 0x80;
const SHAPE_INSTANCE_MAX_COUNT: usize = 32_767;

// Compound shape fixed header size (before instances)
const COMPOUND_HDR_SIZE: usize = 0xd0;

// ---------------------------------------------------------------------------
// Shape instance builder
// ---------------------------------------------------------------------------

fn build_shape_instance(
    transform: &[[f32; 4]; 4],
    tree_node_idx: usize,
    child_shape_size: usize,
) -> Vec<u8> {
    let mut buf = vec![0u8; INST_STRIDE];

    // hkTransform: 3 basis columns followed by a translation column.
    // column0.w = flags, column1.w = 0, column2.w = child shape size,
    // column3.w = tree node.
    //
    // tree_node_idx uses the full 24-bit field (hknpShapeInstance::setLeafIndex),
    // not 8 bits, so compounds with >255 sub-shapes are not corrupted; the high
    // byte carries the 0x3F flag pattern.
    let row3_w: u32 = 0x3F00_0000 | (tree_node_idx as u32 & 0x00FF_FFFF);
    let row2_w: u32 = 0x3F00_0000 | (child_shape_size as u32 & 0x00FF_FFFF);
    let has_translation = (0..3).any(|row| transform[row][3].abs() > 1.0e-6);
    let has_rotation = (0..3).any(|row| {
        (0..3).any(|column| {
            let identity = if row == column { 1.0 } else { 0.0 };
            (transform[row][column] - identity).abs() > 1.0e-6
        })
    });
    let mut flags = SHAPE_INST_IS_ENABLED;
    if has_translation {
        flags |= SHAPE_INST_HAS_TRANSLATION;
    }
    if has_rotation {
        flags |= SHAPE_INST_HAS_ROTATION;
    }
    let row0_w = pack_inst_row_w(flags);

    for column in 0..4 {
        let base = column * 16;
        let xyz = if column < 3 {
            [
                transform[0][column],
                transform[1][column],
                transform[2][column],
            ]
        } else {
            [transform[0][3], transform[1][3], transform[2][3]]
        };
        buf[base..base + 4].copy_from_slice(&xyz[0].to_le_bytes());
        buf[base + 4..base + 8].copy_from_slice(&xyz[1].to_le_bytes());
        buf[base + 8..base + 12].copy_from_slice(&xyz[2].to_le_bytes());
        let w: u32 = match column {
            0 => row0_w,
            1 => 0,
            2 => row2_w,
            3 => row3_w,
            _ => 0,
        };
        buf[base + 12..base + 16].copy_from_slice(&w.to_le_bytes());
    }

    // Scale = (1,1,1,1) at +0x40
    buf[0x40..0x44].copy_from_slice(&1.0f32.to_le_bytes());
    buf[0x44..0x48].copy_from_slice(&1.0f32.to_le_bytes());
    buf[0x48..0x4c].copy_from_slice(&1.0f32.to_le_bytes());
    buf[0x4c..0x50].copy_from_slice(&1.0f32.to_le_bytes());

    // shape* ptr = 0 at +0x50 (global fixup fills it)
    // shapeTag = 0xffff at +0x58, destructionTag = 0xffff at +0x5a
    buf[0x58] = 0xff;
    buf[0x59] = 0xff;
    buf[0x5a] = 0xff;
    buf[0x5b] = 0xff;

    // hknpShapeInstance constructor initializes allocated slots with
    // m_isEmpty = 0 and m_nextEmptyElement = 0. Only setEmpty() writes a
    // next-free id.
    buf[0x5C] = 0; // m_isEmpty
    buf[0x60..0x64].copy_from_slice(&0u32.to_le_bytes()); // m_nextEmptyElement

    buf
}

fn transformed_vertices(vertices: &[[f32; 3]], transform: &[[f32; 4]; 4]) -> Vec<[f32; 3]> {
    vertices
        .iter()
        .map(|vertex| {
            [
                transform[0][0] * vertex[0]
                    + transform[0][1] * vertex[1]
                    + transform[0][2] * vertex[2]
                    + transform[0][3],
                transform[1][0] * vertex[0]
                    + transform[1][1] * vertex[1]
                    + transform[1][2] * vertex[2]
                    + transform[1][3],
                transform[2][0] * vertex[0]
                    + transform[2][1] * vertex[1]
                    + transform[2][2] * vertex[2]
                    + transform[2][3],
            ]
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Compound shape header builder
// ---------------------------------------------------------------------------

fn build_compound_shape_header(n_instances: usize, aabb: &Aabb, user_data: u64) -> Vec<u8> {
    let mut buf = vec![0u8; COMPOUND_HDR_SIZE];

    // +0x00: hkReferencedObject (16 bytes, all zeros)

    // +0x10: hknpShape base
    buf[0x10] = COMPOUND_FLAGS;
    buf[0x12] = compound_shape_key_bits(n_instances);
    buf[0x13] = COMPOUND_DISPATCH;
    buf[0x14..0x18].copy_from_slice(&0.0f32.to_le_bytes()); // convexRadius
    buf[0x18..0x20].copy_from_slice(&user_data.to_le_bytes());
    // properties* ptr = 0

    // +0x30: edgeWeldingMap (hknpSparseCompactMap<unsigned short>, empty)
    buf[0x30..0x34].copy_from_slice(&u32::MAX.to_le_bytes());
    buf[0x44..0x48].copy_from_slice(&0x8000_0000u32.to_le_bytes());
    buf[0x54..0x58].copy_from_slice(&0x8000_0000u32.to_le_bytes());

    // +0x58: hknpCompositeShape.shapeTagCodecInfo. Vanilla FO4 compounds use
    // UINT_MAX here; zero is not a valid "no codec" sentinel for these shapes.
    buf[0x58..0x5c].copy_from_slice(&u32::MAX.to_le_bytes());

    // +0x60: instances hkFreeListArray header (ptr=0, count=n, cap=n|0x80000000)
    let count = n_instances as u32;
    let cap = count | 0x8000_0000;
    buf[0x68..0x6c].copy_from_slice(&count.to_le_bytes());
    buf[0x6c..0x70].copy_from_slice(&cap.to_le_bytes());

    // +0x70: 0xffffffff sentinel
    buf[0x70..0x74].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes());

    // +0x80: compound AABB
    buf[0x80..0x84].copy_from_slice(&aabb.min[0].to_le_bytes());
    buf[0x84..0x88].copy_from_slice(&aabb.min[1].to_le_bytes());
    buf[0x88..0x8c].copy_from_slice(&aabb.min[2].to_le_bytes());
    // w = 0.0 at 0x8c
    buf[0x90..0x94].copy_from_slice(&aabb.max[0].to_le_bytes());
    buf[0x94..0x98].copy_from_slice(&aabb.max[1].to_le_bytes());
    buf[0x98..0x9c].copy_from_slice(&aabb.max[2].to_le_bytes());

    // +0xa0: isMutable = 1
    buf[0xa0] = 1;

    // +0xc0: boundingVolumeData* ptr = 0 (global fixup added by caller)

    buf
}

fn compound_shape_key_bits(n_instances: usize) -> u8 {
    let count = n_instances.max(1) as u32;
    (u32::BITS - count.leading_zeros()) as u8
}

fn validate_compound_tree_counts(n_leaves: usize) -> HavokResult<()> {
    if n_leaves > SHAPE_INSTANCE_MAX_COUNT {
        return Err(crate::error::HavokError::InvalidInput(format!(
            "compound child count {n_leaves} exceeds hknpShapeInstanceId handle capacity {SHAPE_INSTANCE_MAX_COUNT}"
        )));
    }
    if !codec32_counts_fit(n_leaves) {
        return Err(crate::error::HavokError::InvalidInput(format!(
            "compound AABB tree for {n_leaves} leaves exceeds Codec32/DynamicStorage16 capacity of {CODEC32_MAX_LEAVES} leaves"
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Sub-shape data section helpers
// ---------------------------------------------------------------------------

fn write_polytope_objects(
    vertices: &[[f32; 3]],
    mass: f32,
    user_data: u64,
    convex_radius: f32,
    name_offs: &std::collections::HashMap<String, usize>,
    fx: &mut FixupBuilder,
    data: &mut Vec<u8>,
) -> HavokResult<(usize, usize)> {
    // Compute hull via Quickhull (same as polytope.rs). On degenerate input
    // (coplanar or too-few vertices), return an error. The NIF conversion layer
    // will use its visible/AABB fallback instead of serializing a broken hull.
    super::compressed_mesh::validate_vertices(vertices)?;
    let topo = crate::collision::hull::compute_hull_topology_robust(vertices)?;
    let (hull_verts, mut planes, mut faces, indices) =
        (topo.vertices, topo.planes, topo.faces, topo.indices);

    // Pad to SDK minimum of 4 planes / 4 faces (hknpConvexPolytopeShape.h B1).
    while planes.len() < 4 && !planes.is_empty() {
        planes.push(planes[0]);
    }
    while faces.len() < 4 && !faces.is_empty() {
        faces.push(faces[0]);
    }

    // Delegate to the shared inner writer that polytope.rs also calls. Compound
    // children keep the AABB per-child mass solve; the source mass distribution
    // describes the whole body, so it is applied to single-shape bodies only.
    Ok(super::polytope::write_polytope_shape_object(
        &hull_verts,
        &planes,
        &faces,
        &indices,
        mass,
        None,
        None,
        false,
        convex_radius,
        user_data,
        name_offs,
        fx,
        data,
    ))
}

fn write_source_polytope_objects(
    shape: &SourcePolytopeShape,
    mass: f32,
    user_data: u64,
    name_offs: &std::collections::HashMap<String, usize>,
    fx: &mut FixupBuilder,
    data: &mut Vec<u8>,
) -> HavokResult<(usize, usize)> {
    shape.validate()?;
    Ok(super::polytope::write_polytope_shape_object(
        &shape.vertices,
        &shape.planes,
        &shape.faces,
        &shape.indices,
        mass,
        None,
        shape.mass_properties.as_ref(),
        true,
        shape.convex_radius,
        user_data,
        name_offs,
        fx,
        data,
    ))
}

fn write_cm_sub_shape_objects(
    vertices: &[[f32; 3]],
    triangles: &[[u32; 3]],
    mass: f32,
    name_offs: &std::collections::HashMap<String, usize>,
    fx: &mut FixupBuilder,
    data: &mut Vec<u8>,
) -> HavokResult<(usize, usize)> {
    use super::compressed_mesh::pack_vertex_11_11_10;

    // Compressed-mesh sub-shapes are static-only in vanilla FO4 — they don't
    // emit an attached hknpShapeMassProperties block (the property bag stops
    // at hknpBSMaterialProperties).  We accept `mass` for API uniformity with
    // the polytope path and ignore it here.
    let _ = mass;
    let nv = vertices.len();
    let nt = triangles.len();
    if nv == 0 || nt == 0 {
        return Err(crate::error::HavokError::InvalidInput(
            "compressed mesh compound child must have vertices and triangles".to_string(),
        ));
    }
    if nv > 255 || nt > 128 {
        return Err(crate::error::HavokError::InvalidInput(format!(
            "compressed mesh compound child exceeds one-section limits: {nv} vertices, {nt} triangles"
        )));
    }
    super::compressed_mesh::validate_vertices(vertices)?;
    for (i, tri) in triangles.iter().enumerate() {
        super::compressed_mesh::validate_compressed_triangle(vertices, tri, i)?;
    }

    let quad_bytes = super::compressed_mesh::encode_triangle_primitive_bytes(vertices, triangles);
    let primitive_count = (quad_bytes.len() / 4) as u32;
    let max_key_value = max_primitive_key_value(&quad_bytes);
    let triangle_bits = max_key_value.saturating_add(1);
    let quad_bits = triangle_bits.saturating_add(1) / 2;
    let shape_key_bits = if triangle_bits <= 1 {
        1
    } else {
        u32::BITS - (triangle_bits - 1).leading_zeros()
    } as u8;
    let quad_words = bitfield_word_count(quad_bits);
    let triangle_words = bitfield_word_count(triangle_bits);

    // hknpCompressedMeshShape header (0xC0 bytes)
    let shape_rel = data.len();
    let mut shape_hdr = PF_CM_SHAPE_HDR.to_vec();
    shape_hdr[0x12] = shape_key_bits;
    write_u32_le(&mut shape_hdr, 0x58, u32::MAX);
    write_bitfield_header(&mut shape_hdr, 0x68, quad_bits);
    write_bitfield_header(&mut shape_hdr, 0x80, triangle_bits);
    data.extend_from_slice(&shape_hdr);
    if let Some(off) = name_offs.get("hknpCompressedMeshShape") {
        fx.add_virtual(shape_rel, 0, *off);
    }
    let shape_refprop_ptr_rel = shape_rel + 0x20;
    let shape_data_ptr_rel = shape_rel + 0x60;

    let quad_words_rel = data.len();
    if quad_words > 0 {
        fx.add_local(shape_rel + 0x68, quad_words_rel);
        data.extend_from_slice(&filled_bitfield_storage(quad_bits));
        while data.len() % 16 != 0 {
            data.push(0);
        }
    }
    let triangle_words_rel = data.len();
    if triangle_words > 0 {
        fx.add_local(shape_rel + 0x80, triangle_words_rel);
        data.extend(std::iter::repeat(0u8).take(triangle_words * 4));
        while data.len() % 16 != 0 {
            data.push(0);
        }
    }

    // hkRefCountedProperties (0x20 bytes)
    let refprop_rel = data.len();
    data.extend_from_slice(PF_REF_COUNTED_PROPS);
    if let Some(off) = name_offs.get("hkRefCountedProperties") {
        fx.add_virtual(refprop_rel, 0, *off);
    }
    fx.add_local(refprop_rel, refprop_rel + 0x10);
    fx.add_global(shape_refprop_ptr_rel, 2, refprop_rel);
    while data.len() % 16 != 0 {
        data.push(0);
    }

    // hknpBSMaterialProperties (0x50 bytes)
    let bs_mat_rel = data.len();
    data.extend_from_slice(PF_BS_MAT_PROPS);
    fx.add_global(refprop_rel + 0x10, 2, bs_mat_rel);
    if let Some(off) = name_offs.get("hknpBSMaterialProperties") {
        fx.add_virtual(bs_mat_rel, 0, *off);
    }
    fx.add_local(bs_mat_rel + 0x10, bs_mat_rel + 0x20);
    while data.len() % 16 != 0 {
        data.push(0);
    }

    // Quantize vertices
    let xs: Vec<f32> = vertices.iter().map(|v| v[0]).collect();
    let ys: Vec<f32> = vertices.iter().map(|v| v[1]).collect();
    let zs: Vec<f32> = vertices.iter().map(|v| v[2]).collect();

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

    let mut packed_verts: Vec<u32> = Vec::with_capacity(nv);
    for v in vertices {
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
        packed_verts.push(pack_vertex_11_11_10(qx, qy, qz));
    }

    // Build a single-leaf AABB tree for the section so the Havok runtime can use
    // O(log n) BVH queries instead of brute-force O(n).
    let mesh_aabb = Aabb {
        min: [min_x, min_y, min_z],
        max: [max_x, max_y, max_z],
    };
    let tree_nodes_bytes = build_aabb_tree_nodes(&[mesh_aabb]);
    let n_tree_nodes = tree_nodes_bytes.len() / 32; // null + 1 leaf = 2

    let primitive_data_run = [0u8, 0u8, 0u8, primitive_count as u8];

    // CompressedMeshShapeData header (0xD0 bytes)
    let sd_rel = data.len();
    let master_nodes_data_rel = sd_rel + 0xD0;
    let sections_data_rel = align16(master_nodes_data_rel + tree_nodes_bytes.len());
    let quads_data_rel = align16(sections_data_rel + 0x60);
    let verts_data_rel = align16(quads_data_rel + quad_bytes.len());
    let primitive_runs_data_rel = align16(verts_data_rel + packed_verts.len() * 4);

    if let Some(off) = name_offs.get("hknpCompressedMeshShapeData") {
        fx.add_virtual(sd_rel, 0, *off);
    }
    fx.add_global(shape_data_ptr_rel, 2, sd_rel);

    let mut sd_hdr = vec![0u8; 0xD0];
    // +0x00: hkReferencedObject
    // +0x10: meshTree.nodes
    write_u64_le(&mut sd_hdr, 0x10, 0);
    write_u32_le(&mut sd_hdr, 0x18, n_tree_nodes as u32);
    write_u32_le(&mut sd_hdr, 0x1C, (n_tree_nodes as u32) | 0x8000_0000);
    write_f32_le(&mut sd_hdr, 0x20, min_x);
    write_f32_le(&mut sd_hdr, 0x24, min_y);
    write_f32_le(&mut sd_hdr, 0x28, min_z);
    write_f32_le(&mut sd_hdr, 0x2C, 0.0);
    write_f32_le(&mut sd_hdr, 0x30, max_x);
    write_f32_le(&mut sd_hdr, 0x34, max_y);
    write_f32_le(&mut sd_hdr, 0x38, max_z);
    write_f32_le(&mut sd_hdr, 0x3C, 0.0);
    write_u32_le(&mut sd_hdr, 0x40, triangle_bits);
    write_u32_le(&mut sd_hdr, 0x44, shape_key_bits as u32);
    write_u32_le(&mut sd_hdr, 0x48, max_key_value);
    sd_hdr[0x4C] = 0;
    write_u64_le(&mut sd_hdr, 0x50, 0);
    write_u32_le(&mut sd_hdr, 0x58, 1);
    write_u32_le(&mut sd_hdr, 0x5C, 1 | 0x8000_0000);
    write_u64_le(&mut sd_hdr, 0x60, 0);
    write_u32_le(&mut sd_hdr, 0x68, primitive_count);
    write_u32_le(&mut sd_hdr, 0x6C, primitive_count | 0x8000_0000);
    write_u64_le(&mut sd_hdr, 0x70, 0);
    write_u32_le(&mut sd_hdr, 0x78, 0);
    write_u32_le(&mut sd_hdr, 0x7C, 0x8000_0000);
    write_u64_le(&mut sd_hdr, 0x80, 0);
    write_u32_le(&mut sd_hdr, 0x88, nv as u32);
    write_u32_le(&mut sd_hdr, 0x8C, (nv as u32) | 0x8000_0000);
    write_u64_le(&mut sd_hdr, 0x90, 0);
    write_u32_le(&mut sd_hdr, 0x98, 0);
    write_u32_le(&mut sd_hdr, 0x9C, 0x8000_0000);
    write_u64_le(&mut sd_hdr, 0xA0, 0);
    write_u32_le(&mut sd_hdr, 0xA8, 1);
    write_u32_le(&mut sd_hdr, 0xAC, 1 | 0x8000_0000);
    write_u64_le(&mut sd_hdr, 0xB8, 0);
    write_u32_le(&mut sd_hdr, 0xC0, 2);
    write_u32_le(&mut sd_hdr, 0xC4, 0x8000_0002);
    data.extend_from_slice(&sd_hdr);

    fx.add_local(sd_rel + 0x10, master_nodes_data_rel);
    fx.add_local(sd_rel + 0x50, sections_data_rel);
    fx.add_local(sd_rel + 0x60, quads_data_rel);
    fx.add_local(sd_rel + 0x80, verts_data_rel);
    fx.add_local(sd_rel + 0xA0, primitive_runs_data_rel);

    // Section struct (0x60 bytes)
    while data.len() < master_nodes_data_rel {
        data.push(0);
    }
    data.extend_from_slice(&tree_nodes_bytes);
    while data.len() < sections_data_rel {
        data.push(0);
    }

    let mut sec = vec![0u8; 0x60];
    write_u64_le(&mut sec, 0x00, 0);
    write_u32_le(&mut sec, 0x08, 0);
    write_u32_le(&mut sec, 0x0C, 0x8000_0000);
    write_f32_le(&mut sec, 0x10, min_x);
    write_f32_le(&mut sec, 0x14, min_y);
    write_f32_le(&mut sec, 0x18, min_z);
    write_f32_le(&mut sec, 0x1C, 0.0);
    write_f32_le(&mut sec, 0x20, max_x);
    write_f32_le(&mut sec, 0x24, max_y);
    write_f32_le(&mut sec, 0x28, max_z);
    write_f32_le(&mut sec, 0x2C, 0.0);
    write_f32_le(&mut sec, 0x30, min_x);
    write_f32_le(&mut sec, 0x34, min_y);
    write_f32_le(&mut sec, 0x38, min_z);
    write_f32_le(&mut sec, 0x3C, sx);
    write_f32_le(&mut sec, 0x40, sy);
    write_f32_le(&mut sec, 0x44, sz);
    write_u32_le(&mut sec, 0x48, 0);
    write_u32_le(&mut sec, 0x4C, nv as u32);
    write_u32_le(&mut sec, 0x50, primitive_count);
    write_u32_le(&mut sec, 0x54, 1);
    sec[0x58] = nv as u8;
    sec[0x59] = 0;
    data.extend_from_slice(&sec);

    while data.len() < quads_data_rel {
        data.push(0);
    }

    data.extend_from_slice(&quad_bytes);
    while data.len() < verts_data_rel {
        data.push(0);
    }
    for pv in &packed_verts {
        data.extend_from_slice(&pv.to_le_bytes());
    }
    while data.len() < primitive_runs_data_rel {
        data.push(0);
    }
    data.extend_from_slice(&primitive_data_run);
    while data.len() % 16 != 0 {
        data.push(0);
    }

    let simd_nodes_rel = data.len();
    fx.add_local(sd_rel + 0xB8, simd_nodes_rel);
    let lo_lane: [u8; 4] = [0xEE, 0xFF, 0x7F, 0x7F];
    let hi_lane: [u8; 4] = [0xEE, 0xFF, 0x7F, 0xFF];
    for _node in 0..2 {
        for vec_idx in 0..6 {
            let lane_pat = if vec_idx % 2 == 0 { &lo_lane } else { &hi_lane };
            for _ in 0..4 {
                data.extend_from_slice(lane_pat);
            }
        }
        data.extend_from_slice(&[0u8; 16]);
    }
    while data.len() % 16 != 0 {
        data.push(0);
    }

    Ok((shape_rel, refprop_rel))
}

// ---------------------------------------------------------------------------
// Write helpers (local to this module)
// ---------------------------------------------------------------------------

fn write_u32_le(buf: &mut Vec<u8>, off: usize, v: u32) {
    buf[off..off + 4].copy_from_slice(&v.to_le_bytes());
}

fn write_u64_le(buf: &mut Vec<u8>, off: usize, v: u64) {
    buf[off..off + 8].copy_from_slice(&v.to_le_bytes());
}

fn write_f32_le(buf: &mut Vec<u8>, off: usize, v: f32) {
    buf[off..off + 4].copy_from_slice(&v.to_le_bytes());
}

// ---------------------------------------------------------------------------
// Main data section builder
// ---------------------------------------------------------------------------

fn build_compound_data_section(
    sub_shapes: &[CompoundChild],
    name_offs: &std::collections::HashMap<String, usize>,
    opts: &BuildOptions,
) -> HavokResult<(Vec<u8>, FixupBuilder)> {
    let n = sub_shapes.len();
    validate_compound_tree_counts(n)?;
    let all_polytope = sub_shapes.iter().all(|s| {
        matches!(
            s.kind,
            CompoundChildKind::Polytope { .. } | CompoundChildKind::SourcePolytope { .. }
        )
    });
    let all_mesh = sub_shapes
        .iter()
        .all(|s| matches!(s.kind, CompoundChildKind::CompressedMesh { .. }));

    if !all_polytope && !all_mesh {
        return Err(crate::error::HavokError::InvalidInput(
            "Mixed-kind compound shapes not supported".to_string(),
        ));
    }

    let mut fx = FixupBuilder::new();
    let mut data: Vec<u8> = Vec::new();

    // Compute per-child AABBs and compound AABB. Havok's child AABB includes
    // the convex radius; the compound tree must cover the same surface used by
    // narrowphase queries.
    let mut leaf_aabbs: Vec<Aabb> = Vec::with_capacity(sub_shapes.len());
    for (idx, s) in sub_shapes.iter().enumerate() {
        let (verts, convex_radius) = match &s.kind {
            CompoundChildKind::Polytope { vertices } => (vertices.as_slice(), opts.convex_radius),
            CompoundChildKind::SourcePolytope { shape } => {
                (shape.vertices.as_slice(), shape.convex_radius)
            }
            CompoundChildKind::CompressedMesh { vertices, .. } => (vertices.as_slice(), 0.0),
        };
        let world_vertices = transformed_vertices(verts, &s.transform);
        match Aabb::from_vertices(&world_vertices) {
            Some(aabb) => leaf_aabbs.push(aabb.expanded(convex_radius)),
            None => {
                return Err(crate::error::HavokError::InvalidInput(format!(
                    "sub-shape {idx} has no vertices (EmptySubShape)"
                )));
            }
        }
    }

    let compound_aabb = leaf_aabbs
        .iter()
        .skip(1)
        .fold(leaf_aabbs[0], |acc, a| acc.merged(a));

    // -- hknpPhysicsSystemData (0x80 bytes, static body variant) --
    let psd_rel = data.len();
    if let Some(off) = name_offs.get("hknpPhysicsSystemData") {
        fx.add_virtual(psd_rel, 0, *off);
    }

    let mut arr_off = |count: usize| -> usize {
        let off = data.len();
        data.extend_from_slice(&hkarray(count));
        off
    };

    let _arr00 = arr_off(0); // materials (empty)
    let arr10 = arr_off(1); // body_props
    let _arr20 = arr_off(0); // motionProperties (empty)
    let _arr30 = arr_off(0); // motionCinfos (empty)
    let arr40 = arr_off(1); // bodyCinfos
    let _arr50 = arr_off(0); // constraintCinfos (empty)
    let arr60 = arr_off(1); // shapeEntries
    data.extend_from_slice(&[0u8; 16]); // +0x70 pad
    debug_assert_eq!(data.len(), psd_rel + 0x80);

    // body_props (0x50 bytes)
    let body_props_rel = data.len();
    data.extend_from_slice(&build_body_props_with_raw(
        opts.friction,
        opts.restitution,
        opts.body_props_raw.as_ref(),
    ));
    fx.add_local(arr10, body_props_rel);

    // bodyCinfo (0x60 bytes)
    let body_cinfo_rel = data.len();
    data.extend_from_slice(&build_body_cinfo(opts.layer));
    fx.add_local(arr40, body_cinfo_rel);

    // shapeEntry (0x10 bytes)
    let shape_entry_rel = data.len();
    data.extend_from_slice(&[0u8; 16]);
    fx.add_local(arr60, shape_entry_rel);

    // -- hknpDynamicCompoundShape header --
    let compound_shape_rel = data.len();
    let compound_user_data = opts.user_data.unwrap_or(0);
    let compound_hdr = build_compound_shape_header(n, &compound_aabb, compound_user_data);
    data.extend_from_slice(&compound_hdr);
    debug_assert_eq!(data.len(), compound_shape_rel + COMPOUND_HDR_SIZE);

    if let Some(off) = name_offs.get("hknpDynamicCompoundShape") {
        fx.add_virtual(compound_shape_rel, 0, *off);
    }
    fx.add_global(body_cinfo_rel, 2, compound_shape_rel);
    fx.add_global(shape_entry_rel, 2, compound_shape_rel);

    // Compute leaf→tree-node mapping for instance encoding
    let node_indices = leaf_node_indices(&leaf_aabbs);

    // Write shape instances (shape* ptr = 0, fixups added after sub-shapes)
    let mut inst_ptr_rels: Vec<usize> = Vec::new();
    let mut inst_shape_size_rels: Vec<usize> = Vec::new();
    let instances_rel = data.len();
    fx.add_local(compound_shape_rel + 0x60, instances_rel);
    for (i, child) in sub_shapes.iter().enumerate() {
        let inst_rel = data.len();
        let inst_bytes = build_shape_instance(&child.transform, node_indices[i], 0);
        data.extend_from_slice(&inst_bytes);
        inst_ptr_rels.push(inst_rel + 0x50); // shape* offset within instance
        inst_shape_size_rels.push(inst_rel + 0x2c);
    }

    // Pointer to boundingVolumeData (DynCompShapeData)
    let bvd_ptr_rel = compound_shape_rel + 0xc0;

    // Write sub-shape objects. Each child gets an equal share of the body mass,
    // which keeps the parent's m_inverse_mass = 1/M. No parallel-axis aggregation:
    // FO4 vanilla compounds are almost all static.
    let n_children = sub_shapes.len().max(1) as f32;
    let per_child_mass = if opts.mass > 0.0 {
        opts.mass / n_children
    } else {
        0.0
    };
    let child_user_data = opts.user_data.unwrap_or(0);
    let child_convex_radius = opts.convex_radius;
    let mut shape_rels: Vec<usize> = Vec::new();
    let mut shape_instance_sizes: Vec<usize> = Vec::new();
    for child in sub_shapes.iter() {
        let (shape_rel, shape_instance_size) = match &child.kind {
            CompoundChildKind::Polytope { vertices } => {
                let (sr, refprop_rel) = write_polytope_objects(
                    vertices,
                    per_child_mass,
                    child_user_data,
                    child_convex_radius,
                    name_offs,
                    &mut fx,
                    &mut data,
                )?;
                (sr, refprop_rel - sr)
            }
            CompoundChildKind::SourcePolytope { shape } => {
                let (sr, refprop_rel) = write_source_polytope_objects(
                    shape,
                    per_child_mass,
                    child_user_data,
                    name_offs,
                    &mut fx,
                    &mut data,
                )?;
                (sr, refprop_rel - sr)
            }
            CompoundChildKind::CompressedMesh {
                vertices,
                triangles,
            } => {
                let (sr, _) = write_cm_sub_shape_objects(
                    vertices,
                    triangles,
                    per_child_mass,
                    name_offs,
                    &mut fx,
                    &mut data,
                )?;
                (sr, COMPRESSED_MESH_INSTANCE_SIZE)
            }
        };
        shape_rels.push(shape_rel);
        shape_instance_sizes.push(shape_instance_size);
    }

    for (&size_rel, &shape_size) in inst_shape_size_rels.iter().zip(shape_instance_sizes.iter()) {
        if shape_size > 0x00ff_ffff {
            return Err(crate::error::HavokError::InvalidInput(format!(
                "compound child shape size {shape_size} exceeds hknpShapeInstance int24 capacity"
            )));
        }
        let packed = 0x3f00_0000 | shape_size as u32;
        data[size_rel..size_rel + 4].copy_from_slice(&packed.to_le_bytes());
    }

    // Wire instance shape* pointers → sub-shapes
    for (inst_ptr, &shape_rel) in inst_ptr_rels.iter().zip(shape_rels.iter()) {
        fx.add_global(*inst_ptr, 2, shape_rel);
    }

    // -- hknpDynamicCompoundShapeData --
    while data.len() % 16 != 0 {
        data.push(0);
    }
    let bvd_rel = data.len();
    if let Some(off) = name_offs.get("hknpDynamicCompoundShapeData") {
        fx.add_virtual(bvd_rel, 0, *off);
    }
    fx.add_global(bvd_ptr_rel, 2, bvd_rel);

    // hkReferencedObject (16 bytes)
    data.extend_from_slice(&[0u8; 16]);

    let nodes_bytes = build_aabb_tree_nodes(&leaf_aabbs);
    let num_nodes = nodes_bytes.len() / 32;
    let first_free = if n == 0 { 0 } else { num_nodes - 1 };

    // aabbTree hkArray header
    let arr_tree_off = data.len();
    data.extend_from_slice(&hkarray(num_nodes));

    // hknpDynamicCompoundShapeTree inherits:
    // hkcdDynamicTreeDefaultTree32Storage
    //   -> hkcdDynamicTreeTreehkcdDynamicTreeDynamicStorage16
    //   -> hkcdDynamicTreeDynamicStorage16
    //   -> hkcdDynamicTreeDefaultDynamicStoragehkcdDynamicTreeCodec32.
    // Classxml offsets: nodes +0x00, firstFree u16 +0x10,
    // numLeaves u32 +0x18, path u32 +0x1c, root u16 +0x20.
    data.extend_from_slice(&(first_free as u16).to_le_bytes());
    data.extend_from_slice(&[0u8; 6]);
    data.extend_from_slice(&(n as u32).to_le_bytes());
    data.extend_from_slice(&0u32.to_le_bytes()); // path
    let root = if n == 0 { 0 } else { 1 };
    data.extend_from_slice(&(root as u16).to_le_bytes());
    data.extend_from_slice(&[0u8; 14]); // pad to 0x30

    // Node array bytes
    let nodes_data_rel = data.len();
    data.extend_from_slice(&nodes_bytes);

    fx.add_local(arr_tree_off, nodes_data_rel);

    while data.len() % 16 != 0 {
        data.push(0);
    }

    Ok((data, fx))
}

// ---------------------------------------------------------------------------
// Public builder
// ---------------------------------------------------------------------------

/// Build a FO4 Havok 2014.1.0 packfile with hknpDynamicCompoundShape.
///
/// All sub-shapes must have the same kind (all polytope or all compressed_mesh).
pub fn build_fo4_compound_collision(
    sub_shapes: &[CompoundChild],
    opts: &BuildOptions,
) -> HavokResult<Vec<u8>> {
    if sub_shapes.is_empty() {
        return Err(crate::error::HavokError::InvalidInput(
            "at least one sub-shape required".to_string(),
        ));
    }
    validate_compound_tree_counts(sub_shapes.len())?;

    let all_polytope = sub_shapes.iter().all(|s| {
        matches!(
            s.kind,
            CompoundChildKind::Polytope { .. } | CompoundChildKind::SourcePolytope { .. }
        )
    });
    let all_mesh = sub_shapes
        .iter()
        .all(|s| matches!(s.kind, CompoundChildKind::CompressedMesh { .. }));

    if !all_polytope && !all_mesh {
        return Err(crate::error::HavokError::InvalidInput(
            "Mixed-kind compound shapes not supported".to_string(),
        ));
    }

    let class_entries = if all_polytope {
        PF_COMPOUND_POLY_CLASS_ENTRIES
    } else {
        PF_COMPOUND_MESH_CLASS_ENTRIES
    };

    let (cn_data, name_offs) = build_classnames(class_entries);
    let cn_name_off = *name_offs
        .get("hknpPhysicsSystemData")
        .expect("hknpPhysicsSystemData must be present");

    let (obj_data, fx) = build_compound_data_section(sub_shapes, &name_offs, opts)?;

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
    let obj_data_len = data_section.len() - local_tbl.len() - global_tbl.len() - virt_tbl.len();
    let local_fix_abs = data_start + obj_data_len;
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

    let mut out = hdr;
    out.extend_from_slice(&shdr0);
    out.extend_from_slice(&shdr1);
    out.extend_from_slice(&shdr2);
    out.extend_from_slice(&cn_data);
    out.extend_from_slice(&data_section);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> [[f32; 4]; 4] {
        CompoundChild::identity_transform()
    }

    fn read_row3_w(inst_bytes: &[u8]) -> u32 {
        u32::from_le_bytes(inst_bytes[0x3C..0x40].try_into().unwrap())
    }

    fn read_row2_w(inst_bytes: &[u8]) -> u32 {
        u32::from_le_bytes(inst_bytes[0x2C..0x30].try_into().unwrap())
    }

    fn tetra_child(x: f32) -> CompoundChild {
        CompoundChild {
            transform: identity(),
            kind: CompoundChildKind::Polytope {
                vertices: vec![
                    [x, 0.0, 0.0],
                    [x + 1.0, 0.0, 0.0],
                    [x, 1.0, 0.0],
                    [x, 0.0, 1.0],
                ],
            },
        }
    }

    fn source_tetrahedron() -> SourcePolytopeShape {
        SourcePolytopeShape {
            vertices: vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 1.0],
            ],
            planes: vec![
                [0.0, 0.0, -1.0, 0.0],
                [0.0, -1.0, 0.0, 0.0],
                [-1.0, 0.0, 0.0, 0.0],
                [0.57735026, 0.57735026, 0.57735026, -0.57735026],
            ],
            faces: vec![(0, 3, 1), (3, 3, 31), (6, 3, 59), (9, 3, 127)],
            indices: vec![0, 2, 1, 0, 1, 3, 0, 3, 2, 1, 2, 3],
            convex_radius: 0.01,
            mass_properties: None,
        }
    }

    #[test]
    fn shape_instance_row3_encodes_low_24_bits_of_tree_node_idx() {
        // The high byte of row3.w carries Havok flag bits (0x3F constant);
        // the remaining 24 bits hold the leaf index.
        let bytes = build_shape_instance(&identity(), 0x000123_AB, 0);
        let row3_w = read_row3_w(&bytes);
        // Top byte preserves the 0x3F flag pattern.
        assert_eq!(row3_w >> 24, 0x3F, "row3.w top byte must remain 0x3F");
        // Low 24 bits must hold the full leaf index.
        assert_eq!(
            row3_w & 0x00FF_FFFF,
            0x000123_AB,
            "row3.w low 24 bits must hold tree_node_idx (got 0x{:06X})",
            row3_w & 0xFFFFFF
        );
    }

    #[test]
    fn shape_instance_row2_encodes_child_shape_size() {
        let bytes = build_shape_instance(&identity(), 1, 0x190);
        assert_eq!(read_row2_w(&bytes), 0x3f00_0190);
    }

    #[test]
    fn shape_instance_serializes_column_major_transform_and_flags() {
        let transform = [
            [0.0, -1.0, 0.0, 5.0],
            [1.0, 0.0, 0.0, 6.0],
            [0.0, 0.0, 1.0, 7.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        let bytes = build_shape_instance(&transform, 1, 0x190);
        let read_f32 = |offset| f32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
        let read_u32 = |offset| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());

        assert_eq!(
            [read_f32(0x00), read_f32(0x04), read_f32(0x08)],
            [0.0, 1.0, 0.0]
        );
        assert_eq!(
            [read_f32(0x10), read_f32(0x14), read_f32(0x18)],
            [-1.0, 0.0, 0.0]
        );
        assert_eq!(
            [read_f32(0x20), read_f32(0x24), read_f32(0x28)],
            [0.0, 0.0, 1.0]
        );
        assert_eq!(
            [read_f32(0x30), read_f32(0x34), read_f32(0x38)],
            [5.0, 6.0, 7.0]
        );
        assert_eq!(
            read_u32(0x0c) & 0x00ff_ffff,
            SHAPE_INST_IS_ENABLED | SHAPE_INST_HAS_TRANSLATION | SHAPE_INST_HAS_ROTATION
        );
    }

    #[test]
    fn shape_instance_supports_indices_above_255() {
        // The tightest regression: index 299 must round-trip without being
        // truncated to (299 & 0xFF) = 43.
        let bytes = build_shape_instance(&identity(), 299, 0);
        let row3_w = read_row3_w(&bytes);
        assert_eq!(row3_w & 0x00FF_FFFF, 299);
    }

    #[test]
    fn shape_instance_writes_freelist_metadata() {
        let bytes = build_shape_instance(&identity(), 0, 0);
        assert_eq!(
            bytes[0x5C], 0,
            "m_isEmpty must be 0 (this slot is allocated)"
        );
        let next_empty = u32::from_le_bytes(bytes[0x60..0x64].try_into().unwrap());
        assert_eq!(
            next_empty, 0,
            "m_nextEmptyElement must be 0 for allocated slots"
        );
    }

    #[test]
    fn source_compound_child_preserves_face_min_half_angles() {
        let shape = source_tetrahedron();
        let expected = shape.faces.iter().map(|face| face.2).collect::<Vec<_>>();
        let mut data = Vec::new();
        let mut fixups = FixupBuilder::new();
        let (shape_rel, _) = write_source_polytope_objects(
            &shape,
            0.0,
            0,
            &std::collections::HashMap::new(),
            &mut fixups,
            &mut data,
        )
        .expect("write source compound child");
        let face_count =
            u16::from_le_bytes(data[shape_rel + 0x44..shape_rel + 0x46].try_into().unwrap());
        let face_rel =
            u16::from_le_bytes(data[shape_rel + 0x46..shape_rel + 0x48].try_into().unwrap());
        let faces_abs = shape_rel + 0x44 + usize::from(face_rel);
        let actual = (0..usize::from(face_count))
            .map(|index| data[faces_abs + index * 4 + 3])
            .collect::<Vec<_>>();

        assert_eq!(actual, expected);
    }

    #[test]
    fn source_compound_child_carries_source_mass_properties_verbatim() {
        let mut shape = source_tetrahedron();
        shape.mass_properties = Some(
            crate::collision::mass_properties::CompressedMassProperties {
                center_of_mass: [23241, 0, 0, 8832],
                inertia: [29172, 11849, 30893, 11136],
                major_axis_space: [-32768, -32768, -32768, -2768],
                mass: 0.038964,
                volume: 0.038964,
            },
        );
        let mut data = Vec::new();
        let mut fixups = FixupBuilder::new();
        let (_, refprop_rel) = write_source_polytope_objects(
            &shape,
            0.0,
            0,
            &std::collections::HashMap::new(),
            &mut fixups,
            &mut data,
        )
        .expect("write source compound child");
        // hknpShapeMassProperties block starts after the 0x20-byte
        // hkRefCountedProperties object.
        let mp = refprop_rel + 0x20;
        let read_i16x4 = |offset: usize| {
            let mut out = [0i16; 4];
            for (i, slot) in out.iter_mut().enumerate() {
                *slot = i16::from_le_bytes(
                    data[offset + i * 2..offset + i * 2 + 2].try_into().unwrap(),
                );
            }
            out
        };
        assert_eq!(read_i16x4(mp + 0x10), [23241, 0, 0, 8832]);
        assert_eq!(read_i16x4(mp + 0x18), [29172, 11849, 30893, 11136]);
        assert_eq!(read_i16x4(mp + 0x20), [-32768, -32768, -32768, -2768]);
        let mass = f32::from_le_bytes(data[mp + 0x28..mp + 0x2C].try_into().unwrap());
        let volume = f32::from_le_bytes(data[mp + 0x2C..mp + 0x30].try_into().unwrap());
        assert_eq!(mass, 0.038964);
        assert_eq!(volume, 0.038964);
    }

    #[test]
    fn source_compound_instance_uses_serialized_polytope_size() {
        let shape = source_tetrahedron();
        let mut shape_data = Vec::new();
        let mut shape_fixups = FixupBuilder::new();
        let (shape_rel, refprop_rel) = write_source_polytope_objects(
            &shape,
            0.0,
            0,
            &std::collections::HashMap::new(),
            &mut shape_fixups,
            &mut shape_data,
        )
        .expect("write source polytope");
        let expected_size = refprop_rel - shape_rel;
        let children = vec![CompoundChild {
            transform: identity(),
            kind: CompoundChildKind::SourcePolytope { shape },
        }];
        let (data, _) = build_compound_data_section(
            &children,
            &std::collections::HashMap::new(),
            &BuildOptions::default(),
        )
        .expect("build source compound");
        let compound_shape_rel = 0x80 + 0x50 + 0x60 + 0x10;
        let instance_rel = compound_shape_rel + COMPOUND_HDR_SIZE;
        let packed_size = u32::from_le_bytes(
            data[instance_rel + 0x2c..instance_rel + 0x30]
                .try_into()
                .unwrap(),
        );

        assert_eq!(packed_size & 0x00ff_ffff, expected_size as u32);
    }

    #[test]
    fn dynamic_compound_tree_metadata_uses_dynamic_storage16_layout() {
        let children = vec![tetra_child(0.0), tetra_child(2.0)];
        let (data, _) = build_compound_data_section(
            &children,
            &std::collections::HashMap::new(),
            &BuildOptions::default(),
        )
        .unwrap();
        let leaf_aabbs: Vec<Aabb> = children
            .iter()
            .map(|child| match &child.kind {
                CompoundChildKind::Polytope { vertices } => Aabb::from_vertices(vertices)
                    .unwrap()
                    .expanded(BuildOptions::default().convex_radius),
                _ => unreachable!(),
            })
            .collect();
        let nodes_bytes = build_aabb_tree_nodes(&leaf_aabbs);
        let nodes_rel = data
            .windows(nodes_bytes.len())
            .rposition(|window| window == nodes_bytes)
            .expect("serialized tree nodes should be present");
        let tree_rel = nodes_rel - 0x30;

        let first_free =
            u16::from_le_bytes(data[tree_rel + 0x10..tree_rel + 0x12].try_into().unwrap());
        let first_free_padding = &data[tree_rel + 0x12..tree_rel + 0x18];
        let num_leaves =
            u32::from_le_bytes(data[tree_rel + 0x18..tree_rel + 0x1c].try_into().unwrap());
        let path = u32::from_le_bytes(data[tree_rel + 0x1c..tree_rel + 0x20].try_into().unwrap());
        let root = u16::from_le_bytes(data[tree_rel + 0x20..tree_rel + 0x22].try_into().unwrap());
        let root_padding = &data[tree_rel + 0x22..tree_rel + 0x30];

        assert_eq!(first_free, 4, "firstFree is the spare free-list node");
        assert!(first_free_padding.iter().all(|&byte| byte == 0));
        assert_eq!(num_leaves, 2);
        assert_eq!(path, 0);
        assert_eq!(root, 1);
        assert!(root_padding.iter().all(|&byte| byte == 0));
    }

    #[test]
    fn compound_rejects_shape_instance_handle_overflow() {
        let too_many = (0..=SHAPE_INSTANCE_MAX_COUNT)
            .map(|_| CompoundChild {
                transform: identity(),
                kind: CompoundChildKind::Polytope {
                    vertices: Vec::new(),
                },
            })
            .collect::<Vec<_>>();
        let err = build_fo4_compound_collision(&too_many, &BuildOptions::default()).unwrap_err();
        assert!(
            matches!(err, crate::error::HavokError::InvalidInput(ref message) if message.contains("hknpShapeInstanceId handle capacity")),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn compound_shape_header_writes_first_free_sentinel() {
        // Per hkFreeListArrayhknpShapeInstance...xml: m_firstFree is hkInt32
        // at offset 16 of the array (relative to the array's hkArray header).
        // In the compound shape header, the instances array starts at +0x60
        // so m_firstFree lives at +0x70.  Empty list → -1 = 0xFFFFFFFF.
        let aabb = Aabb {
            min: [-1.0; 3],
            max: [1.0; 3],
        };
        let buf = build_compound_shape_header(2, &aabb, 0);
        let first_free = u32::from_le_bytes(buf[0x70..0x74].try_into().unwrap());
        assert_eq!(
            first_free, 0xFFFF_FFFF,
            "compound instances m_firstFree must be -1 (empty free list)"
        );
    }

    #[test]
    fn source_compound_bounds_include_child_convex_radius() {
        let shape = source_tetrahedron();
        let radius = shape.convex_radius;
        let children = vec![CompoundChild {
            transform: identity(),
            kind: CompoundChildKind::SourcePolytope { shape },
        }];
        let (data, _) = build_compound_data_section(
            &children,
            &std::collections::HashMap::new(),
            &BuildOptions::default(),
        )
        .expect("build source compound");
        let compound_shape_rel = 0x80 + 0x50 + 0x60 + 0x10;
        let read_f32 = |offset| f32::from_le_bytes(data[offset..offset + 4].try_into().unwrap());

        assert!((read_f32(compound_shape_rel + 0x80) + radius).abs() < 1e-6);
        assert!((read_f32(compound_shape_rel + 0x84) + radius).abs() < 1e-6);
        assert!((read_f32(compound_shape_rel + 0x88) + radius).abs() < 1e-6);
        assert!((read_f32(compound_shape_rel + 0x90) - (1.0 + radius)).abs() < 1e-6);
        assert!((read_f32(compound_shape_rel + 0x94) - (1.0 + radius)).abs() < 1e-6);
        assert!((read_f32(compound_shape_rel + 0x98) - (1.0 + radius)).abs() < 1e-6);
    }

    #[test]
    fn source_compound_bounds_apply_child_transform() {
        let shape = source_tetrahedron();
        let radius = shape.convex_radius;
        let mut transform = identity();
        transform[0][3] = 5.0;
        transform[1][3] = 6.0;
        transform[2][3] = 7.0;
        let children = vec![CompoundChild {
            transform,
            kind: CompoundChildKind::SourcePolytope { shape },
        }];
        let (data, _) = build_compound_data_section(
            &children,
            &std::collections::HashMap::new(),
            &BuildOptions::default(),
        )
        .expect("build transformed source compound");
        let compound_shape_rel = 0x80 + 0x50 + 0x60 + 0x10;
        let read_f32 = |offset| f32::from_le_bytes(data[offset..offset + 4].try_into().unwrap());

        assert!((read_f32(compound_shape_rel + 0x80) - (5.0 - radius)).abs() < 1e-6);
        assert!((read_f32(compound_shape_rel + 0x84) - (6.0 - radius)).abs() < 1e-6);
        assert!((read_f32(compound_shape_rel + 0x88) - (7.0 - radius)).abs() < 1e-6);
        assert!((read_f32(compound_shape_rel + 0x90) - (6.0 + radius)).abs() < 1e-6);
        assert!((read_f32(compound_shape_rel + 0x94) - (7.0 + radius)).abs() < 1e-6);
        assert!((read_f32(compound_shape_rel + 0x98) - (8.0 + radius)).abs() < 1e-6);
    }

    #[test]
    fn compound_shape_header_writes_empty_edge_welding_map() {
        let aabb = Aabb {
            min: [-1.0; 3],
            max: [1.0; 3],
        };
        let buf = build_compound_shape_header(2, &aabb, 0);
        let read_u32 = |offset| u32::from_le_bytes(buf[offset..offset + 4].try_into().unwrap());

        assert_eq!(read_u32(0x30), u32::MAX, "secondaryKeyMask");
        assert_eq!(read_u32(0x34), 0, "sencondaryKeyBits");
        assert!(buf[0x38..0x40].iter().all(|&byte| byte == 0));
        assert_eq!(read_u32(0x40), 0, "primaryKeyToIndex size");
        assert_eq!(read_u32(0x44), 0x8000_0000, "primaryKeyToIndex capacity");
        assert!(buf[0x48..0x50].iter().all(|&byte| byte == 0));
        assert_eq!(read_u32(0x50), 0, "valueAndSecondaryKeys size");
        assert_eq!(
            read_u32(0x54),
            0x8000_0000,
            "valueAndSecondaryKeys capacity"
        );
    }

    #[test]
    fn compound_shape_header_matches_vanilla_shape_key_bits() {
        let aabb = Aabb {
            min: [-1.0; 3],
            max: [1.0; 3],
        };

        for (count, expected_bits) in [(2, 2), (3, 2), (4, 3), (6, 3), (8, 4), (11, 4)] {
            let buf = build_compound_shape_header(count, &aabb, 0);
            assert_eq!(
                buf[0x12], expected_bits,
                "numShapeKeyBits for {count} compound child instance(s)"
            );
            assert_eq!(buf[0x13], COMPOUND_DISPATCH, "dispatchType");
        }
    }

    #[test]
    fn compound_shape_header_writes_vanilla_shape_tag_codec_info() {
        let aabb = Aabb {
            min: [-1.0; 3],
            max: [1.0; 3],
        };
        let buf = build_compound_shape_header(6, &aabb, 0);
        let codec = u32::from_le_bytes(buf[0x58..0x5c].try_into().unwrap());
        assert_eq!(
            codec,
            u32::MAX,
            "hknpCompositeShape.shapeTagCodecInfo must match vanilla FO4 compound shapes"
        );
    }
}
