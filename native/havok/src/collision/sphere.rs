// FO4 Havok 2014.1.0 packfile builder — hknpSphereShape.
//
// Mirrors pynifly's `pack_sphere` (refs/io_scene_nifly/pyn/bhk_autopack.py:1587).
// Unlike polytope/CM, sphere packfiles have NO hkRefCountedProperties and NO
// hknpShapeMassProperties — the shape stores its radius in the inherited
// hknpShape::convexRadius field. The shape object's bytes are hand-coded after
// `Poolball_Cue.nif`: the descriptor doesn't cover the full 0x50-byte trailer
// (offsets 0x34-0x4F carry the 0x00100004 marker and the 0.5 float that the
// FO4 runtime requires for sphere shapes).
//
// Because those trailer bytes aren't in `hknpSphereShape_0.xml`, a round-trip
// through `HkxFile::read → save` would silently strip them. The single-body
// sphere path in `build_fo4_multi_body_collision` short-circuits the merge
// for `[Sphere]` and returns the raw blob directly to preserve the trailer.

use super::compressed_mesh::{
    BuildOptions, FixupBuilder, build_body_cinfo, build_body_props_with_raw, build_classnames,
    build_file_header, build_section_header, hkarray,
};
use crate::error::HavokResult;

const PF_SPHERE_CLASS_ENTRIES: &[(u32, &str)] = &[
    (0x33D42383, "hkClass"),
    (0xB0EFA719, "hkClassMember"),
    (0x8A3609CF, "hkClassEnum"),
    (0xCE6F8A6C, "hkClassEnumItem"),
    (0xB857718B, "hknpPhysicsSystemData"),
    (0x741E9012, "hknpSphereShape"),
];

/// Magic byte at `+0x10` (u32 little-endian).
/// Decodes as flags=0x0111 (IS_CONVEX_SHAPE | USE_SINGLE_POINT_MANIFOLD |
/// NO_GET_SHAPE_KEYS_ON_SPU), numShapeKeyBits=0, dispatchType=1.
const SPHERE_FLAGS_NSK_DISPATCH: u32 = 0x01000111;

/// Marker u32 at `+0x30` (vertices hkRelArray field on hknpConvexShape).
/// Matches vanilla Poolball_Cue.nif. The descriptor only declares the 4-byte
/// header here, but the FO4 runtime expects this exact value.
const SPHERE_TRAILER_MARKER: u32 = 0x00100004;

/// Sphere shape blob (0x50 bytes). `radius` lives in `hknpShape::convexRadius`
/// at offset 0x14. `_user_data` is accepted for API parity but pynifly leaves
/// hknpShape::userData at zero — write zero so vanilla NIF byte-diffs match.
fn build_sphere_shape_blob(radius: f32, _user_data: u64) -> [u8; 0x50] {
    let mut buf = [0u8; 0x50];
    buf[0x10..0x14].copy_from_slice(&SPHERE_FLAGS_NSK_DISPATCH.to_le_bytes());
    buf[0x14..0x18].copy_from_slice(&radius.to_le_bytes());
    buf[0x30..0x34].copy_from_slice(&SPHERE_TRAILER_MARKER.to_le_bytes());
    buf[0x4C..0x50].copy_from_slice(&0.5f32.to_le_bytes());
    buf
}

/// Patch the BodyCInfo's position field with the sphere center (Havok space).
/// `body_cinfo` layout: hknpBodyCinfo_2.xml — position is the first 12 bytes
/// of the `position` Vector4 at field offset 0x30.
fn patch_body_cinfo_position(body_cinfo: &mut [u8], position: [f32; 3]) {
    body_cinfo[0x30..0x34].copy_from_slice(&position[0].to_le_bytes());
    body_cinfo[0x34..0x38].copy_from_slice(&position[1].to_le_bytes());
    body_cinfo[0x38..0x3C].copy_from_slice(&position[2].to_le_bytes());
}

fn build_sphere_data_section(
    radius: f32,
    position: [f32; 3],
    name_offs: &std::collections::HashMap<String, usize>,
    opts: &BuildOptions,
) -> (Vec<u8>, FixupBuilder) {
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

    write_bytes!(&hkarray(0)); // +0x00: hkReferencedObject (unused payload)
    let materials_off = write_bytes!(&hkarray(1)); // +0x10: materials
    write_bytes!(&hkarray(0)); // +0x20: motionProperties (static body)
    write_bytes!(&hkarray(0)); // +0x30: motionCinfos (static body)
    let body_cinfos_off = write_bytes!(&hkarray(1)); // +0x40: bodyCinfos
    write_bytes!(&hkarray(0)); // +0x50: constraintCinfos
    let referenced_off = write_bytes!(&hkarray(1)); // +0x60: referencedObjects
    write_bytes!(&[0u8; 16]); // +0x70: name (StringPtr) + padding
    debug_assert_eq!(rel!(), psd_rel + 0x80);

    // -- hknpMaterial[0] inline struct (0x50 bytes; reuse polytope/CM helper) --
    let body_props_rel = rel!();
    write_bytes!(&build_body_props_with_raw(
        opts.friction,
        opts.restitution,
        opts.body_props_raw.as_ref()
    ));
    fx.add_local(materials_off, body_props_rel);

    // -- hknpBodyCinfo[0] (0x60 bytes) with sphere center patched in --
    let body_cinfo_rel = rel!();
    let mut body_cinfo = build_body_cinfo(opts.layer);
    patch_body_cinfo_position(&mut body_cinfo, position);
    write_bytes!(&body_cinfo);
    fx.add_local(body_cinfos_off, body_cinfo_rel);

    // -- referencedObjects[0] pointer slot (0x10 bytes) --
    let shape_entry_rel = rel!();
    write_bytes!(&[0u8; 16]);
    fx.add_local(referenced_off, shape_entry_rel);

    // -- hknpSphereShape (0x50 bytes) --
    let shape_rel = rel!();
    fx.add_virtual(shape_rel, 0, *name_offs.get("hknpSphereShape").unwrap());
    let shape_blob = build_sphere_shape_blob(radius, opts.user_data.unwrap_or(0));
    write_bytes!(&shape_blob);

    // -- Wire bodyCinfo and shape entry to the sphere shape --
    fx.add_global(body_cinfo_rel, 2, shape_rel); // bodyCinfo.shape pointer
    fx.add_global(shape_entry_rel, 2, shape_rel); // referencedObjects[0]

    (data, fx)
}

/// Build a FO4 Havok 2014.1.0 packfile blob for a single sphere body.
///
/// Mirrors pynifly's `pack_sphere`. Returns a single-body packfile containing:
/// - hknpPhysicsSystemData (one static body, one material)
/// - hknpBodyCinfo with position pointing to the sphere center
/// - hknpSphereShape (radius stored in `hknpShape::convexRadius` at offset 0x14)
///
/// `position` is the sphere center in Havok space (NIF / havok_scale).
pub fn build_fo4_sphere_collision(
    radius: f32,
    position: [f32; 3],
    opts: &BuildOptions,
) -> HavokResult<Vec<u8>> {
    let (cn_data, name_offs) = build_classnames(PF_SPHERE_CLASS_ENTRIES);
    let cn_name_off = *name_offs
        .get("hknpPhysicsSystemData")
        .expect("hknpPhysicsSystemData must be in classnames");

    let (obj_data, fx) = build_sphere_data_section(radius, position, &name_offs, opts);

    let local_tbl = fx.build_local_table();
    let global_tbl = fx.build_global_table();
    let virt_tbl = fx.build_virtual_table();

    let mut data_section = obj_data;
    let obj_len = data_section.len();
    data_section.extend_from_slice(&local_tbl);
    data_section.extend_from_slice(&global_tbl);
    data_section.extend_from_slice(&virt_tbl);

    let cn_start = 0x100usize;
    let cn_end = cn_start + cn_data.len();
    let data_start = cn_end;

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
    use crate::hkx::HkxFile;

    #[test]
    fn sphere_blob_parses_as_packfile() {
        let blob = build_fo4_sphere_collision(0.5, [1.0, 2.0, 3.0], &BuildOptions::default())
            .expect("build sphere");
        let file = HkxFile::read(&blob).expect("parse sphere packfile");
        let class_names: Vec<&str> = file
            .objects()
            .iter()
            .map(|o| o.class_name.as_str())
            .collect();
        assert!(class_names.contains(&"hknpPhysicsSystemData"));
        assert!(class_names.contains(&"hknpSphereShape"));
    }

    #[test]
    fn sphere_radius_lands_in_convex_radius_field() {
        let blob = build_fo4_sphere_collision(0.75, [0.0, 0.0, 0.0], &BuildOptions::default())
            .expect("build sphere");
        // Locate the hknpSphereShape virtual fixup → its convexRadius lives
        // at +0x14 in the shape body.
        let data_start =
            u32::from_le_bytes(blob[0xC0 + 0x14..0xC0 + 0x18].try_into().unwrap()) as usize;
        let virt_fix_rel =
            u32::from_le_bytes(blob[0xC0 + 0x20..0xC0 + 0x24].try_into().unwrap()) as usize;
        let exports_rel =
            u32::from_le_bytes(blob[0xC0 + 0x24..0xC0 + 0x28].try_into().unwrap()) as usize;
        let cn_section_start = 0x100usize;
        let mut pos = data_start + virt_fix_rel;
        let exports_abs = data_start + exports_rel;
        let mut shape_abs = None;
        while pos + 12 <= exports_abs {
            let obj_rel = u32::from_le_bytes(blob[pos..pos + 4].try_into().unwrap());
            if obj_rel == 0xFFFF_FFFF {
                break;
            }
            let sec_idx = u32::from_le_bytes(blob[pos + 4..pos + 8].try_into().unwrap());
            let name_off = u32::from_le_bytes(blob[pos + 8..pos + 12].try_into().unwrap());
            if sec_idx == 0 {
                let name_abs = cn_section_start + name_off as usize;
                let name_bytes: Vec<u8> = blob[name_abs..]
                    .iter()
                    .take_while(|&&b| b != 0)
                    .copied()
                    .collect();
                if name_bytes == b"hknpSphereShape" {
                    shape_abs = Some(data_start + obj_rel as usize);
                    break;
                }
            }
            pos += 12;
        }
        let shape_abs = shape_abs.expect("hknpSphereShape virtual fixup not found");
        let radius_bytes: [u8; 4] = blob[shape_abs + 0x14..shape_abs + 0x18].try_into().unwrap();
        let radius = f32::from_le_bytes(radius_bytes);
        assert!(
            (radius - 0.75).abs() < 1e-6,
            "expected radius 0.75 in convexRadius, got {radius}"
        );

        // Trailer marker at +0x30 and 0.5 magic at +0x4C must be present
        // — game requires them for sphere stability.
        let marker =
            u32::from_le_bytes(blob[shape_abs + 0x30..shape_abs + 0x34].try_into().unwrap());
        assert_eq!(marker, SPHERE_TRAILER_MARKER);
        let trailer =
            f32::from_le_bytes(blob[shape_abs + 0x4C..shape_abs + 0x50].try_into().unwrap());
        assert!((trailer - 0.5).abs() < 1e-6);
    }
}
