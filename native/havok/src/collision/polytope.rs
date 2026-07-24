// FO4 Havok 2014.1.0 packfile builder — hknpConvexPolytopeShape.
//
// Mirrors py_creation_lib/python/creation_lib/havok/_fo4_polytope.py exactly.  The convex hull is computed by
// hull::compute_hull_topology (a pure-Rust Quickhull implementation).
//
// Layout of generated packfile matches the Python builder; see
// _fo4_polytope.py for the full byte-layout comment.

use super::compressed_mesh::{
    BuildOptions, FixupBuilder, PF_POLYTOPE_CLASS_ENTRIES, build_body_cinfo,
    build_body_props_with_raw, build_classnames, build_file_header, build_section_header, hkarray,
};
use super::hull::compute_hull_topology_robust;
use super::mass_properties::{polytope_mass_properties, serialize_mass_properties_block};
use crate::error::{HavokError, HavokResult};

// hknpConvexPolytopeShape constants
const SHAPE_FLAGS: u16 = 0x0143;
const SHAPE_DISPATCH_TYPE: u8 = 1;
const FO4_POLYTOPE_FACE_MIN_HALF_ANGLE: u8 = 128;
const FO4_POLYTOPE_SENTINEL_PLANES: [[f32; 4]; 2] = [[0.0, 0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0]];

// hknpShapeMassProperties is 0x30 bytes (hkReferencedObject 16 + hkCompressedMassProperties 32).

use super::constants::REFCOUNTED_PROPS_KEY_MASS_PROPS;

#[derive(Debug, Clone, PartialEq)]
pub struct SourcePolytopeShape {
    pub vertices: Vec<[f32; 3]>,
    pub planes: Vec<[f32; 4]>,
    pub faces: Vec<(u16, u8, u8)>,
    pub indices: Vec<u8>,
    pub convex_radius: f32,
    /// Source `hknpShapeMassProperties` carried verbatim; vanilla FO4 compound
    /// children always ship real compressed mass properties, never zeros.
    pub mass_properties: Option<super::mass_properties::CompressedMassProperties>,
}

impl SourcePolytopeShape {
    pub fn validate(&self) -> HavokResult<()> {
        if self.vertices.is_empty() {
            return Err(HavokError::InvalidInput(
                "source polytope has no vertices".to_string(),
            ));
        }
        if self.planes.is_empty() {
            return Err(HavokError::InvalidInput(
                "source polytope has no planes".to_string(),
            ));
        }
        if self.faces.is_empty() {
            return Err(HavokError::InvalidInput(
                "source polytope has no faces".to_string(),
            ));
        }
        if self.indices.is_empty() {
            return Err(HavokError::InvalidInput(
                "source polytope has no indices".to_string(),
            ));
        }
        if !self.convex_radius.is_finite() || self.convex_radius < 0.0 {
            return Err(HavokError::InvalidInput(format!(
                "source polytope has invalid convex radius {}",
                self.convex_radius
            )));
        }
        super::compressed_mesh::validate_vertices(&self.vertices)?;
        for (index, plane) in self.planes.iter().enumerate() {
            if !plane.iter().all(|value| value.is_finite()) {
                return Err(HavokError::InvalidInput(format!(
                    "source polytope plane {index} is not finite: {plane:?}"
                )));
            }
        }
        for (face_index, &(first, count, _)) in self.faces.iter().enumerate() {
            if count < 3 {
                return Err(HavokError::InvalidInput(format!(
                    "source polytope face {face_index} has fewer than 3 indices"
                )));
            }
            let first = usize::from(first);
            let count = usize::from(count);
            let end = first.checked_add(count).ok_or_else(|| {
                HavokError::InvalidInput(format!(
                    "source polytope face {face_index} index span overflows"
                ))
            })?;
            if end > self.indices.len() {
                return Err(HavokError::InvalidInput(format!(
                    "source polytope face {face_index} index span {first}..{end} exceeds {} indices",
                    self.indices.len()
                )));
            }
            for &vertex_index in &self.indices[first..end] {
                if usize::from(vertex_index) >= self.vertices.len() {
                    return Err(HavokError::InvalidInput(format!(
                        "source polytope face {face_index} references vertex {vertex_index}, but only {} vertices exist",
                        self.vertices.len()
                    )));
                }
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// hkRelArray helper (4 bytes: u16 size + u16 rel_offset_from_field)
// ---------------------------------------------------------------------------

fn rel_array(size: usize, rel_off: usize) -> [u8; 4] {
    let mut b = [0u8; 4];
    b[0..2].copy_from_slice(&(size as u16).to_le_bytes());
    b[2..4].copy_from_slice(&(rel_off as u16).to_le_bytes());
    b
}

fn normalize_fo4_polytope_arrays(
    planes: &mut Vec<[f32; 4]>,
    faces: &mut Vec<(u16, u8, u8)>,
    preserve_min_half_angles: bool,
) {
    while planes.len() < 4 && !planes.is_empty() {
        planes.push(planes[0]);
    }
    while faces.len() < 4 && !faces.is_empty() {
        faces.push(faces[0]);
    }

    if !preserve_min_half_angles {
        for face in faces.iter_mut() {
            face.2 = FO4_POLYTOPE_FACE_MIN_HALF_ANGLE;
        }
    }

    if !planes.is_empty() && planes.len() == faces.len() {
        planes.extend_from_slice(&FO4_POLYTOPE_SENTINEL_PLANES);
    }
}

// ---------------------------------------------------------------------------
// Shared polytope shape-object writer
// ---------------------------------------------------------------------------

/// Write a single hknpConvexPolytopeShape object block (shape header + arrays +
/// hkRefCountedProperties + hknpShapeMassProperties) into `data`, appending fixups
/// to `fx`.
///
/// Returns `(shape_rel, refprop_rel)` — the offsets of the shape and refprop objects
/// within `data`.  Used by both the standalone polytope builder and the compound
/// sub-shape writer to avoid layout duplication.
pub(crate) fn write_polytope_shape_object(
    hull_verts: &[[f32; 3]],
    planes: &[[f32; 4]],
    faces: &[(u16, u8, u8)],
    indices: &[u8],
    mass: f32,
    mass_dist: Option<&super::mass_properties::SourceMassDistribution>,
    source_mass_props: Option<&super::mass_properties::CompressedMassProperties>,
    preserve_min_half_angles: bool,
    convex_radius: f32,
    user_data: u64,
    name_offs: &std::collections::HashMap<String, usize>,
    fx: &mut FixupBuilder,
    data: &mut Vec<u8>,
) -> (usize, usize) {
    let mut planes = planes.to_vec();
    let mut faces = faces.to_vec();
    normalize_fo4_polytope_arrays(&mut planes, &mut faces, preserve_min_half_angles);

    let real_n = hull_verts.len();
    let n_verts = (real_n + 3) & !3; // pad up to a multiple of 4
    let n_planes = planes.len();
    let n_faces = faces.len();
    let n_indices = indices.len();

    let shape_rel = data.len();
    if let Some(off) = name_offs.get("hknpConvexPolytopeShape") {
        fx.add_virtual(shape_rel, 0, *off);
    }

    // Compute relarray positions relative to shape_rel
    let verts_data_off = shape_rel + 0x50;
    let planes_data_off = verts_data_off + n_verts * 16;
    let faces_data_off = planes_data_off + n_planes * 16;
    let faces_padded_size = ((n_faces * 4 + 15) / 16) * 16;
    let indices_data_off = faces_data_off + faces_padded_size;

    let verts_field = shape_rel + 0x30;
    let planes_field = shape_rel + 0x40;
    let faces_field = shape_rel + 0x44;
    let indices_field = shape_rel + 0x48;

    let mut shape_hdr = vec![0u8; 0x50];
    // +0x00: hkReferencedObject (16 bytes, all zeros)
    // +0x10: flags u16
    shape_hdr[0x10..0x12].copy_from_slice(&SHAPE_FLAGS.to_le_bytes());
    // +0x13: dispatchType u8
    shape_hdr[0x13] = SHAPE_DISPATCH_TYPE;
    // +0x14: convexRadius f32
    shape_hdr[0x14..0x18].copy_from_slice(&convex_radius.to_le_bytes());
    // +0x18: userData u64
    shape_hdr[0x18..0x20].copy_from_slice(&user_data.to_le_bytes());
    // +0x30: vertices hkRelArray
    let vr = rel_array(n_verts, verts_data_off - verts_field);
    shape_hdr[0x30..0x34].copy_from_slice(&vr);
    // +0x40: planes hkRelArray
    let pr = rel_array(n_planes, planes_data_off - planes_field);
    shape_hdr[0x40..0x44].copy_from_slice(&pr);
    // +0x44: faces hkRelArray
    let fr = rel_array(n_faces, faces_data_off - faces_field);
    shape_hdr[0x44..0x48].copy_from_slice(&fr);
    // +0x48: indices hkRelArray
    let ir = rel_array(n_indices, indices_data_off - indices_field);
    shape_hdr[0x48..0x4C].copy_from_slice(&ir);
    data.extend_from_slice(&shape_hdr);

    let shape_refprop_ptr_rel = shape_rel + 0x20;

    // vertices (n_verts × 16 bytes with vanilla index tag in W)
    // Pad to a multiple of 4 by duplicating the last real vertex; the W-id for
    // padding entries clamps to the last real index (SDK hkcdSupportingVertex
    // duplicate-id break invariant).
    for i in 0..n_verts {
        let src = i.min(real_n.saturating_sub(1));
        let v = &hull_verts[src];
        data.extend_from_slice(&v[0].to_le_bytes());
        data.extend_from_slice(&v[1].to_le_bytes());
        data.extend_from_slice(&v[2].to_le_bytes());
        let w_bits: u32 = 0x3F00_0000u32.wrapping_add(src as u32);
        data.extend_from_slice(&w_bits.to_le_bytes());
    }
    // planes (n_planes × 16 bytes)
    for plane in &planes {
        data.extend_from_slice(&plane[0].to_le_bytes());
        data.extend_from_slice(&plane[1].to_le_bytes());
        data.extend_from_slice(&plane[2].to_le_bytes());
        data.extend_from_slice(&plane[3].to_le_bytes());
    }
    while data.len() % 16 != 0 {
        data.push(0);
    }
    // faces (n_faces × 4 bytes)
    for &(first_idx, num_idx, min_half) in &faces {
        data.extend_from_slice(&first_idx.to_le_bytes());
        data.push(num_idx);
        data.push(min_half);
    }
    while data.len() % 16 != 0 {
        data.push(0);
    }
    // indices
    data.extend_from_slice(indices);
    while data.len() % 16 != 0 {
        data.push(0);
    }

    // hkRefCountedProperties (0x20 bytes)
    let refprop_rel = data.len();
    if let Some(off) = name_offs.get("hkRefCountedProperties") {
        fx.add_virtual(refprop_rel, 0, *off);
    }
    fx.add_global(shape_refprop_ptr_rel, 2, refprop_rel);
    let entries_ptr_field = refprop_rel;
    let refprop_entry_rel = refprop_rel + 0x10;
    data.extend_from_slice(&hkarray(1)); // entries array header (0x10 bytes)
    let entry_start = data.len();
    data.extend_from_slice(&[0u8; 16]); // entry[0]
    let key_off = entry_start + 8;
    data[key_off..key_off + 2].copy_from_slice(&REFCOUNTED_PROPS_KEY_MASS_PROPS.to_le_bytes());
    fx.add_local(entries_ptr_field, refprop_entry_rel);

    // hknpShapeMassProperties (0x30 bytes)
    let mass_props_rel = data.len();
    if let Some(off) = name_offs.get("hknpShapeMassProperties") {
        fx.add_virtual(mass_props_rel, 0, *off);
    }
    fx.add_global(refprop_entry_rel, 2, mass_props_rel);
    // Prefer the source body's real mass distribution (COM / volume / inertia)
    // over the AABB box approximation when it was decoded and this is a dynamic
    // (non-zero mass) body. Static bodies carry the source's compressed block
    // verbatim when one was decoded (vanilla FO4 statics ship real values);
    // otherwise they keep the zeroed block.
    let mp_bytes = match (mass_dist, source_mass_props) {
        (Some(dist), _) if mass > 0.0 => serialize_mass_properties_block(
            &super::mass_properties::mass_properties_from_source(dist),
        ),
        (_, Some(props)) => {
            super::mass_properties::serialize_compressed_mass_properties_block(props)
        }
        _ => serialize_mass_properties_block(&polytope_mass_properties(hull_verts, mass)),
    };
    data.extend_from_slice(&mp_bytes);
    while data.len() % 16 != 0 {
        data.push(0);
    }

    (shape_rel, refprop_rel)
}

// ---------------------------------------------------------------------------
// Data section builder
// ---------------------------------------------------------------------------

fn build_polytope_data_section(
    hull_verts: &[[f32; 3]],
    planes_in: &[[f32; 4]],
    faces_in: &[(u16, u8, u8)],
    indices: &[u8],
    name_offs: &std::collections::HashMap<String, usize>,
    opts: &BuildOptions,
    preserve_min_half_angles: bool,
    source_mass_props: Option<&super::mass_properties::CompressedMassProperties>,
) -> (Vec<u8>, FixupBuilder) {
    // hknpConvexPolytopeShape.h:85,196 mandates that m_planes and m_faces are
    // padded up to a minimum of 4 entries each (planes pad with m_planes[0];
    // faces duplicate the first face).
    let planes_owned: Vec<[f32; 4]>;
    let planes: &[[f32; 4]] = if planes_in.len() >= 4 || planes_in.is_empty() {
        planes_in
    } else {
        let mut v = planes_in.to_vec();
        let pad = v[0];
        while v.len() < 4 {
            v.push(pad);
        }
        planes_owned = v;
        &planes_owned
    };
    let faces_owned: Vec<(u16, u8, u8)>;
    let faces: &[(u16, u8, u8)] = if faces_in.len() >= 4 || faces_in.is_empty() {
        faces_in
    } else {
        let mut v = faces_in.to_vec();
        let pad = v[0];
        while v.len() < 4 {
            v.push(pad);
        }
        faces_owned = v;
        &faces_owned
    };

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

    write_bytes!(&hkarray(0));
    let arr10_off = write_bytes!(&hkarray(1));
    write_bytes!(&hkarray(0));
    write_bytes!(&hkarray(0));
    let arr40_off = write_bytes!(&hkarray(1));
    write_bytes!(&hkarray(0));
    let arr60_off = write_bytes!(&hkarray(1));
    write_bytes!(&[0u8; 16]);
    debug_assert_eq!(rel!(), psd_rel + 0x80);

    // -- body_props (0x50 bytes) --
    let body_props_rel = rel!();
    write_bytes!(&build_body_props_with_raw(
        opts.friction,
        opts.restitution,
        opts.body_props_raw.as_ref()
    ));
    fx.add_local(arr10_off, body_props_rel);

    // -- body_cinfo (0x60 bytes) --
    let body_cinfo_rel = rel!();
    write_bytes!(&build_body_cinfo(opts.layer));
    fx.add_local(arr40_off, body_cinfo_rel);

    // -- shape_entry (0x10 bytes) --
    let shape_entry_rel = rel!();
    write_bytes!(&[0u8; 16]);
    fx.add_local(arr60_off, shape_entry_rel);

    // -- hknpConvexPolytopeShape + refprop + mass props (via shared inner writer) --
    let (shape_rel, _refprop_rel) = write_polytope_shape_object(
        hull_verts,
        planes,
        faces,
        indices,
        opts.mass,
        opts.mass_distribution.as_ref(),
        source_mass_props,
        preserve_min_half_angles,
        opts.convex_radius,
        opts.user_data.unwrap_or(0),
        name_offs,
        &mut fx,
        &mut data,
    );
    fx.add_global(body_cinfo_rel, 2, shape_rel);
    fx.add_global(shape_entry_rel, 2, shape_rel);

    (data, fx)
}

// ---------------------------------------------------------------------------
// Public builder
// ---------------------------------------------------------------------------

/// Build an FO4 Havok 2014.1.0 packfile blob for a single convex polytope.
///
/// Constructs a complete hk_2014.1.0-r1 packfile containing:
/// - hknpPhysicsSystemData (one static body)
/// - hknpConvexPolytopeShape (convex hull computed via Quickhull)
/// - hkRefCountedProperties (one entry)
/// - hknpShapeMassProperties (zeroed, static body)
///
/// Mirrors `build_fo4_polytope_collision` in `py_creation_lib/python/creation_lib/havok/_fo4_polytope.py`.
pub fn build_fo4_polytope_collision(
    vertices: &[[f32; 3]],
    opts: &BuildOptions,
) -> HavokResult<Vec<u8>> {
    if vertices.len() < 4 {
        return Err(HavokError::InvalidInput(
            "at least 4 non-coplanar vertices required".to_string(),
        ));
    }
    super::compressed_mesh::validate_vertices(vertices)?;

    let hull = compute_hull_topology_robust(vertices)?;

    let (cn_data, name_offs) = build_classnames(PF_POLYTOPE_CLASS_ENTRIES);
    let cn_name_off = *name_offs
        .get("hknpPhysicsSystemData")
        .expect("hknpPhysicsSystemData must be in classnames");

    let (obj_data, fx) = build_polytope_data_section(
        &hull.vertices,
        &hull.planes,
        &hull.faces,
        &hull.indices,
        &name_offs,
        opts,
        false,
        None,
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

    let obj_len = data_section.len() - local_tbl.len() - global_tbl.len() - virt_tbl.len();
    let local_fix_abs = data_start + obj_len;
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

/// Build a FO4 Havok 2014.1.0 packfile blob for a single convex polytope using
/// already-decoded source topology. Unlike [`build_fo4_polytope_collision`],
/// this does not recompute a hull from preview vertices.
pub fn build_fo4_source_polytope_collision(
    shape: &SourcePolytopeShape,
    opts: &BuildOptions,
) -> HavokResult<Vec<u8>> {
    shape.validate()?;

    let mut opts = opts.clone();
    opts.convex_radius = shape.convex_radius;

    let (cn_data, name_offs) = build_classnames(PF_POLYTOPE_CLASS_ENTRIES);
    let cn_name_off = *name_offs
        .get("hknpPhysicsSystemData")
        .expect("hknpPhysicsSystemData must be in classnames");

    let (obj_data, fx) = build_polytope_data_section(
        &shape.vertices,
        &shape.planes,
        &shape.faces,
        &shape.indices,
        &name_offs,
        &opts,
        true,
        shape.mass_properties.as_ref(),
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

    let obj_len = data_section.len() - local_tbl.len() - global_tbl.len() - virt_tbl.len();
    let local_fix_abs = data_start + obj_len;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn cube_verts() -> Vec<[f32; 3]> {
        vec![
            [-1.0, -1.0, -1.0],
            [1.0, -1.0, -1.0],
            [1.0, 1.0, -1.0],
            [-1.0, 1.0, -1.0],
            [-1.0, -1.0, 1.0],
            [1.0, -1.0, 1.0],
            [1.0, 1.0, 1.0],
            [-1.0, 1.0, 1.0],
        ]
    }

    fn read_relarray(blob: &[u8], field_off: usize) -> (u16, u16) {
        let cnt = u16::from_le_bytes(blob[field_off..field_off + 2].try_into().unwrap());
        let rel = u16::from_le_bytes(blob[field_off + 2..field_off + 4].try_into().unwrap());
        (cnt, rel)
    }

    fn read_face_min_half_angles(blob: &[u8], shape_abs: usize) -> Vec<u8> {
        let (n_faces, rel) = read_relarray(blob, shape_abs + 0x44);
        let faces_abs = shape_abs + 0x44 + rel as usize;
        (0..n_faces as usize)
            .map(|i| blob[faces_abs + i * 4 + 3])
            .collect()
    }

    /// Locate the hknpConvexPolytopeShape virtual fixup → return abs offset.
    fn find_polytope_shape(blob: &[u8]) -> Option<usize> {
        let data_start =
            u32::from_le_bytes(blob[0xC0 + 0x14..0xC0 + 0x18].try_into().unwrap()) as usize;
        let virt_fix_rel =
            u32::from_le_bytes(blob[0xC0 + 0x20..0xC0 + 0x24].try_into().unwrap()) as usize;
        let exports_rel =
            u32::from_le_bytes(blob[0xC0 + 0x24..0xC0 + 0x28].try_into().unwrap()) as usize;
        let cn_section_start = 0x100usize;
        let mut pos = data_start + virt_fix_rel;
        let exports_abs = data_start + exports_rel;
        while pos + 12 <= exports_abs {
            let obj_rel = u32::from_le_bytes(blob[pos..pos + 4].try_into().unwrap());
            let sec_idx = u32::from_le_bytes(blob[pos + 4..pos + 8].try_into().unwrap());
            let name_off = u32::from_le_bytes(blob[pos + 8..pos + 12].try_into().unwrap());
            if obj_rel == 0xFFFF_FFFF {
                break;
            }
            if sec_idx == 0 {
                let name_abs = cn_section_start + name_off as usize;
                let name_bytes: Vec<u8> = blob[name_abs..]
                    .iter()
                    .take_while(|&&b| b != 0)
                    .copied()
                    .collect();
                if name_bytes == b"hknpConvexPolytopeShape" {
                    return Some(data_start + obj_rel as usize);
                }
            }
            pos += 12;
        }
        None
    }

    #[test]
    fn cube_polytope_planes_and_faces_each_at_least_four() {
        // Sanity: standard quickhull on a cube produces 12 planes/faces.
        let opts = BuildOptions {
            friction: 0.5,
            restitution: 0.4,
            layer: 5,
            mass: 0.0,
            ..BuildOptions::default()
        };
        let blob = build_fo4_polytope_collision(&cube_verts(), &opts).expect("build");
        let shape_abs = find_polytope_shape(&blob).expect("shape located");
        let (n_planes, _) = read_relarray(&blob, shape_abs + 0x40);
        let (n_faces, _) = read_relarray(&blob, shape_abs + 0x44);
        assert!(
            n_planes >= 4,
            "cube hull must emit ≥4 planes (got {n_planes})"
        );
        assert!(n_faces >= 4, "cube hull must emit ≥4 faces (got {n_faces})");
    }

    #[test]
    fn cube_polytope_matches_fo4_box_shape_layout() {
        let opts = BuildOptions {
            friction: 0.5,
            restitution: 0.4,
            layer: 5,
            mass: 0.0,
            ..BuildOptions::default()
        };
        let blob = build_fo4_polytope_collision(&cube_verts(), &opts).expect("build");
        let shape_abs = find_polytope_shape(&blob).expect("shape located");
        let (n_planes, _) = read_relarray(&blob, shape_abs + 0x40);
        let (n_faces, _) = read_relarray(&blob, shape_abs + 0x44);

        assert_eq!(n_faces, 6, "cube must emit six polygonal faces");
        assert_eq!(
            n_planes, 8,
            "FO4 native box-like polytopes carry six face planes plus two sentinel planes"
        );
        assert!(
            read_face_min_half_angles(&blob, shape_abs)
                .into_iter()
                .all(|angle| angle == FO4_POLYTOPE_FACE_MIN_HALF_ANGLE),
            "FO4 convex polytope faces must use the native minHalfAngle"
        );
    }

    #[test]
    fn source_polytope_preserves_face_min_half_angles() {
        let shape = SourcePolytopeShape {
            vertices: cube_verts(),
            planes: vec![
                [1.0, 0.0, 0.0, -1.0],
                [-1.0, 0.0, 0.0, -1.0],
                [0.0, 1.0, 0.0, -1.0],
                [0.0, -1.0, 0.0, -1.0],
                [0.0, 0.0, 1.0, -1.0],
                [0.0, 0.0, -1.0, -1.0],
            ],
            faces: vec![
                (0, 4, 1),
                (4, 4, 127),
                (8, 4, 59),
                (12, 4, 67),
                (16, 4, 31),
                (20, 4, 94),
            ],
            indices: vec![
                1, 2, 6, 5, 0, 4, 7, 3, 2, 3, 7, 6, 0, 1, 5, 4, 4, 5, 6, 7, 0, 3, 2, 1,
            ],
            convex_radius: 0.01,
            mass_properties: None,
        };
        let expected = shape.faces.iter().map(|face| face.2).collect::<Vec<_>>();
        let blob = build_fo4_source_polytope_collision(&shape, &BuildOptions::default())
            .expect("build source polytope");
        let shape_abs = find_polytope_shape(&blob).expect("shape located");

        assert_eq!(read_face_min_half_angles(&blob, shape_abs), expected);
    }

    #[test]
    fn polytope_vertices_padded_to_multiple_of_four() {
        // 5 vertices → must be padded to 8 (next multiple of 4).
        let verts = vec![
            [0.0f32, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 1.0, 1.0],
        ];
        let opts = BuildOptions {
            friction: 0.5,
            restitution: 0.4,
            layer: 5,
            mass: 0.0,
            ..BuildOptions::default()
        };
        let blob = build_fo4_polytope_collision(&verts, &opts).expect("build");
        let shape_abs = find_polytope_shape(&blob).expect("shape located");
        let (n_verts, _) = read_relarray(&blob, shape_abs + 0x30);
        assert_eq!(
            n_verts % 4,
            0,
            "vertex count must be a multiple of 4 (got {n_verts})"
        );
        assert!(n_verts >= 8, "5 real verts must pad to ≥8 (got {n_verts})");
    }

    #[test]
    fn build_polytope_data_section_pads_three_planes_to_four() {
        // Even if a degenerate input bypasses Quickhull and hands us 3 planes /
        // 3 faces, the writer must pad to 4 before emitting hkRelArrays —
        // hknpConvexPolytopeShape.h:85,196 invariant.
        let mut name_offs = std::collections::HashMap::new();
        name_offs.insert("hknpPhysicsSystemData".to_string(), 0);
        name_offs.insert("hknpConvexPolytopeShape".to_string(), 0);
        name_offs.insert("hkRefCountedProperties".to_string(), 0);
        name_offs.insert("hknpShapeMassProperties".to_string(), 0);
        let verts: Vec<[f32; 3]> = vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
        ];
        let planes: Vec<[f32; 4]> = vec![
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [1.0, 0.0, 0.0, 0.0],
        ];
        let faces: Vec<(u16, u8, u8)> = vec![(0, 3, 0), (3, 3, 0), (6, 3, 0)];
        let indices: Vec<u8> = vec![0, 1, 2, 0, 1, 3, 1, 2, 3];
        let opts = BuildOptions {
            friction: 0.5,
            restitution: 0.4,
            layer: 5,
            mass: 0.0,
            ..BuildOptions::default()
        };
        let (data, _fx) = build_polytope_data_section(
            &verts, &planes, &faces, &indices, &name_offs, &opts, false, None,
        );
        // Find the polytope shape header in the synthetic data section.
        // It starts after psd (0x80) + body_props (0x50) + body_cinfo (0x60)
        // + shape_entry (0x10) = 0x150.
        let shape_off = 0x80 + 0x50 + 0x60 + 0x10;
        // planes hkRelArray at +0x40; padded count should now be 4.
        let n_planes =
            u16::from_le_bytes(data[shape_off + 0x40..shape_off + 0x42].try_into().unwrap());
        let n_faces =
            u16::from_le_bytes(data[shape_off + 0x44..shape_off + 0x46].try_into().unwrap());
        assert!(n_planes >= 4, "planes must pad to ≥4 (got {n_planes})");
        assert!(n_faces >= 4, "faces must pad to ≥4 (got {n_faces})");
    }
}
