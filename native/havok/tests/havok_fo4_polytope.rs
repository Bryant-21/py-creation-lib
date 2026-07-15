// Integration tests for the FO4 hknpConvexPolytopeShape builder.
//
// These tests verify:
//   1. Tetrahedron input produces a valid packfile with correct magic bytes.
//   2. The packfile sections (__classnames__, __types__, __data__) are present
//      and correctly structured (a lightweight section-header walker).
//   3. Cube input: hull correctly preserves 8 vertices (parsed by walking
//      the data section).
//
// Run with: cd py_creation_lib/native/havok && cargo test --test havok_fo4_polytope

use havok_native::collision::compressed_mesh::BuildOptions;
use havok_native::collision::polytope::build_fo4_polytope_collision;

const PF_MAGIC: &[u8; 8] = b"\x57\xE0\xE0\x57\x10\xC0\xC0\x10";

fn tetrahedron_verts() -> Vec<[f32; 3]> {
    vec![
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 1.0],
    ]
}

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

fn default_opts() -> BuildOptions {
    BuildOptions {
        friction: 0.5,
        restitution: 0.4,
        layer: 5,
        mass: 0.0,
        ..BuildOptions::default()
    }
}

// ---------------------------------------------------------------------------
// Section-header walker (lightweight packfile parser)
// ---------------------------------------------------------------------------

struct SectionInfo {
    abs_start: u32,
    local_fix_rel: u32,
    global_fix_rel: u32,
    virt_fix_rel: u32,
    exports_rel: u32,
}

fn read_u32_le(data: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(data[off..off + 4].try_into().unwrap())
}

fn read_section_header(data: &[u8], off: usize) -> (String, SectionInfo) {
    // Section header: 0x40 bytes
    // +0x00: name (null-terminated, padded to 0x14 bytes with 0xFF)
    // +0x14: abs_start u32
    // +0x18: local_fix u32 (relative to abs_start)
    // +0x1C: global_fix u32
    // +0x20: virt_fix u32
    // +0x24: exports u32
    let name_bytes: Vec<u8> = data[off..off + 0x14]
        .iter()
        .take_while(|&&b| b != 0 && b != 0xFF)
        .copied()
        .collect();
    let name = String::from_utf8(name_bytes).unwrap_or_default();
    let abs_start = read_u32_le(data, off + 0x14);
    let local_fix_rel = read_u32_le(data, off + 0x18);
    let global_fix_rel = read_u32_le(data, off + 0x1C);
    let virt_fix_rel = read_u32_le(data, off + 0x20);
    let exports_rel = read_u32_le(data, off + 0x24);
    (
        name,
        SectionInfo {
            abs_start,
            local_fix_rel,
            global_fix_rel,
            virt_fix_rel,
            exports_rel,
        },
    )
}

// ---------------------------------------------------------------------------
// Test 1: magic bytes and minimum size
// ---------------------------------------------------------------------------

#[test]
fn tetrahedron_starts_with_pf_magic() {
    let blob = build_fo4_polytope_collision(&tetrahedron_verts(), &default_opts())
        .expect("build_fo4_polytope_collision should succeed for tetrahedron");
    assert!(blob.len() >= 0x100, "blob must be at least 0x100 bytes");
    assert_eq!(&blob[..8], PF_MAGIC, "must start with Havok packfile magic");
}

// ---------------------------------------------------------------------------
// Test 2: section structure walk
// ---------------------------------------------------------------------------

#[test]
fn packfile_sections_are_correct() {
    let blob =
        build_fo4_polytope_collision(&tetrahedron_verts(), &default_opts()).expect("build failed");

    // File header: 0x40 bytes; 3 section headers follow at 0x40, 0x80, 0xC0
    let (name0, info0) = read_section_header(&blob, 0x40);
    let (name1, info1) = read_section_header(&blob, 0x80);
    let (name2, info2) = read_section_header(&blob, 0xC0);

    assert_eq!(name0, "__classnames__");
    assert_eq!(name1, "__types__");
    assert_eq!(name2, "__data__");

    // classnames section starts at 0x100
    assert_eq!(info0.abs_start, 0x100);

    // types section: empty, starts where classnames ends
    assert_eq!(info1.abs_start, info0.abs_start + info0.exports_rel);

    // data section: starts where types ends
    assert_eq!(info2.abs_start, info1.abs_start);

    // data section must have nonzero content
    assert!(info2.exports_rel > 0, "data section must have content");

    // All fixup offsets must be within the data section
    assert!(info2.local_fix_rel <= info2.exports_rel);
    assert!(info2.global_fix_rel <= info2.exports_rel);
    assert!(info2.virt_fix_rel <= info2.exports_rel);
}

// ---------------------------------------------------------------------------
// Test 3: cube produces blob larger than tetrahedron (more geometry)
// ---------------------------------------------------------------------------

#[test]
fn cube_blob_larger_than_tetrahedron() {
    let tet_blob = build_fo4_polytope_collision(&tetrahedron_verts(), &default_opts())
        .expect("tetrahedron build failed");
    let cube_blob =
        build_fo4_polytope_collision(&cube_verts(), &default_opts()).expect("cube build failed");
    assert!(
        cube_blob.len() > tet_blob.len(),
        "cube must produce more data than tetrahedron"
    );
    assert_eq!(&cube_blob[..8], PF_MAGIC);
}

// ---------------------------------------------------------------------------
// Test 4: degenerate input rejection
// ---------------------------------------------------------------------------

#[test]
fn too_few_vertices_errors() {
    let verts = vec![[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    let err = build_fo4_polytope_collision(&verts, &default_opts())
        .expect_err("should reject fewer than 4 vertices");
    assert!(
        err.to_string().contains("4"),
        "error message should mention '4'"
    );
}

#[test]
fn coplanar_vertices_errors() {
    let verts = vec![
        [0.0f32, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.5, 0.5, 0.0],
        [0.25, 0.75, 0.0],
    ];
    let result = build_fo4_polytope_collision(&verts, &default_opts());
    assert!(result.is_err(), "coplanar vertices should produce an error");
}

// ---------------------------------------------------------------------------
// layer must be written to collisionFilterInfo (0x14) of the
// hknpBodyCinfo block, NOT qualityId (0x10).
// ---------------------------------------------------------------------------

/// Locate the hknpBodyCinfo block via the virtual fixup table.  Returns
/// (abs_block_start, 0x60 bytes) or None.
fn find_body_cinfo_block(blob: &[u8]) -> Option<(usize, [u8; 0x60])> {
    let (_, data_info) = read_section_header(blob, 0xC0);
    let data_start = data_info.abs_start as usize;
    let virt_fix_abs = data_start + data_info.virt_fix_rel as usize;
    let exports_abs = data_start + data_info.exports_rel as usize;
    let cn_section_start = 0x100usize;

    // hknpBodyCinfo is referenced by a local fixup, not a virtual one.  We
    // walk the body_cinfo array indirectly via hknpPhysicsSystemData: the
    // psd has a ptr at +0x40 to the body_cinfo array (0x60 bytes per entry).
    // For a single-body file the cinfo block sits right after body_props
    // (0x50 bytes) which sits right after the psd (0x80 bytes).
    let mut pos = virt_fix_abs;
    while pos + 12 <= exports_abs {
        let obj_rel = read_u32_le(blob, pos);
        let sec_idx = read_u32_le(blob, pos + 4);
        let name_off = read_u32_le(blob, pos + 8);
        if obj_rel == 0xFFFF_FFFF {
            break;
        }
        if sec_idx == 0 {
            let name_abs = cn_section_start + name_off as usize;
            if name_abs + 30 < blob.len() {
                let name_bytes: Vec<u8> = blob[name_abs..]
                    .iter()
                    .take_while(|&&b| b != 0)
                    .copied()
                    .collect();
                let name = String::from_utf8_lossy(&name_bytes);
                if name == "hknpPhysicsSystemData" {
                    // psd at obj_rel; layout: psd 0x80, body_props 0x50, body_cinfo 0x60.
                    let psd_abs = data_start + obj_rel as usize;
                    let body_cinfo_abs = psd_abs + 0x80 + 0x50;
                    if body_cinfo_abs + 0x60 <= blob.len() {
                        let mut block = [0u8; 0x60];
                        block.copy_from_slice(&blob[body_cinfo_abs..body_cinfo_abs + 0x60]);
                        return Some((body_cinfo_abs, block));
                    }
                }
            }
        }
        pos += 12;
    }
    None
}

#[test]
fn polytope_layer_lands_in_collision_filter_info_not_quality_id() {
    // Per hknpBodyCinfo_2.xml: qualityId is hkUint8 at offset 16,
    // collisionFilterInfo is hkUint32 at offset 20.
    let layer: u8 = 7;
    let opts = BuildOptions {
        friction: 0.5,
        restitution: 0.4,
        layer,
        mass: 0.0,
        ..BuildOptions::default()
    };
    let blob = build_fo4_polytope_collision(&cube_verts(), &opts).expect("build");
    let (_, block) = find_body_cinfo_block(&blob).expect("hknpBodyCinfo block must be locatable");

    // qualityId at 0x10 must NOT be the layer value — the template default
    // (0xFF in PF_BODY_CINFO) must survive.
    assert_ne!(
        block[0x10], layer,
        "layer={layer} must not land at qualityId offset 0x10"
    );
    // collisionFilterInfo low byte at 0x14 must be the layer value.
    assert_eq!(
        block[0x14], layer,
        "layer={layer} must land in low byte of collisionFilterInfo at 0x14"
    );
}

// ---------------------------------------------------------------------------
// mass parameter reaches the hknpShapeMassProperties block
// ---------------------------------------------------------------------------

/// Locate the hknpShapeMassProperties object via the virtual fixup table and
/// return (abs_block_start, 0x30 bytes).  Returns None if not found.
fn find_mass_properties_block(blob: &[u8]) -> Option<(usize, [u8; 0x30])> {
    let (_, data_info) = read_section_header(blob, 0xC0);
    let data_start = data_info.abs_start as usize;
    let virt_fix_abs = data_start + data_info.virt_fix_rel as usize;
    let exports_abs = data_start + data_info.exports_rel as usize;
    let cn_section_start = 0x100usize;

    let mut pos = virt_fix_abs;
    while pos + 12 <= exports_abs {
        let obj_rel = read_u32_le(blob, pos);
        let sec_idx = read_u32_le(blob, pos + 4);
        let name_off = read_u32_le(blob, pos + 8);
        if obj_rel == 0xFFFF_FFFF {
            break;
        }
        if sec_idx == 0 {
            let name_abs = cn_section_start + name_off as usize;
            if name_abs + 30 < blob.len() {
                let name_bytes: Vec<u8> = blob[name_abs..]
                    .iter()
                    .take_while(|&&b| b != 0)
                    .copied()
                    .collect();
                let name = String::from_utf8_lossy(&name_bytes);
                if name == "hknpShapeMassProperties" {
                    let abs = data_start + obj_rel as usize;
                    if abs + 0x30 <= blob.len() {
                        let mut block = [0u8; 0x30];
                        block.copy_from_slice(&blob[abs..abs + 0x30]);
                        return Some((abs, block));
                    }
                }
            }
        }
        pos += 12;
    }
    None
}

#[test]
fn polytope_static_mass_zero_keeps_block_zeroed() {
    let opts = BuildOptions {
        friction: 0.5,
        restitution: 0.4,
        layer: 5,
        mass: 0.0,
        ..BuildOptions::default()
    };
    let blob = build_fo4_polytope_collision(&cube_verts(), &opts).expect("build");
    let (_, block) =
        find_mass_properties_block(&blob).expect("hknpShapeMassProperties block must be present");
    // mass and volume f32s at +0x28, +0x2C must both be 0.0 for static body.
    let stored_mass = f32::from_le_bytes(block[0x28..0x2C].try_into().unwrap());
    let stored_volume = f32::from_le_bytes(block[0x2C..0x30].try_into().unwrap());
    assert_eq!(stored_mass, 0.0, "static body mass must be 0");
    assert_eq!(stored_volume, 0.0, "static body volume must be 0");
}

#[test]
fn polytope_dynamic_mass_reaches_packfile() {
    let opts = BuildOptions {
        friction: 0.5,
        restitution: 0.4,
        layer: 5,
        mass: 10.0,
        ..BuildOptions::default()
    };
    let blob = build_fo4_polytope_collision(&cube_verts(), &opts).expect("build");
    let (_, block) =
        find_mass_properties_block(&blob).expect("hknpShapeMassProperties block must be present");
    let stored_mass = f32::from_le_bytes(block[0x28..0x2C].try_into().unwrap());
    assert!(
        (stored_mass - 10.0).abs() < 1e-4,
        "mass=10.0 must reach packfile, got {stored_mass}"
    );
    // Volume of the [-1,1]^3 cube hull is 8.0.
    let stored_volume = f32::from_le_bytes(block[0x2C..0x30].try_into().unwrap());
    assert!(
        (stored_volume - 8.0).abs() < 1e-3,
        "AABB volume=8.0 expected, got {stored_volume}"
    );
}

// ---------------------------------------------------------------------------
// Test 5: virtual fixup table contains expected classnames
// ---------------------------------------------------------------------------

#[test]
fn virtual_fixup_references_hknp_classes() {
    let blob = build_fo4_polytope_collision(&cube_verts(), &default_opts()).expect("build failed");

    let (_, data_info) = read_section_header(&blob, 0xC0);
    let data_start = data_info.abs_start as usize;

    let virt_fix_abs = data_start + data_info.virt_fix_rel as usize;
    let exports_abs = data_start + data_info.exports_rel as usize;

    // Parse virtual fixup table: entries are 12 bytes (obj_rel, sec_idx, name_off)
    let mut found_psd = false;
    let mut found_polytope = false;
    let cn_section_start = 0x100usize; // classnames at 0x100

    let mut pos = virt_fix_abs;
    while pos + 12 <= exports_abs {
        let _obj_rel = read_u32_le(&blob, pos);
        let sec_idx = read_u32_le(&blob, pos + 4);
        let name_off = read_u32_le(&blob, pos + 8);
        if _obj_rel == 0xFFFF_FFFF {
            break;
        }

        // sec_idx == 0 = classnames section
        if sec_idx == 0 {
            let name_abs = cn_section_start + name_off as usize;
            if name_abs + 30 < blob.len() {
                let name_bytes: Vec<u8> = blob[name_abs..]
                    .iter()
                    .take_while(|&&b| b != 0)
                    .copied()
                    .collect();
                let name = String::from_utf8_lossy(&name_bytes);
                if name == "hknpPhysicsSystemData" {
                    found_psd = true;
                }
                if name == "hknpConvexPolytopeShape" {
                    found_polytope = true;
                }
            }
        }
        pos += 12;
    }

    assert!(
        found_psd,
        "virtual fixups must reference hknpPhysicsSystemData"
    );
    assert!(
        found_polytope,
        "virtual fixups must reference hknpConvexPolytopeShape"
    );
}

// ---------------------------------------------------------------------------
// Hull refinement — vertex welding and collinear fallback
// ---------------------------------------------------------------------------

#[test]
fn hull_weld_deduplicates_near_coincident_vertices() {
    use havok_native::collision::hull::{max_extent, weld_vertices};
    // 8 cube corners with a slight jitter: pairs within epsilon should merge
    let verts = vec![
        [0.0f32, 0.0, 0.0],
        [0.0, 0.0, 0.0001], // should weld (dist < epsilon)
        [1.0, 0.0, 0.0],
        [1.0, 0.0, 0.0], // duplicate
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 1.0],
        [1.0, 0.0, 1.0],
    ];
    let ext = max_extent(&verts);
    let eps = (ext * 1e-3).max(1e-3); // generous epsilon for test
    let (welded, _remap) = weld_vertices(&verts, eps);
    // Should deduplicate (0,0,0) pair and (1,0,0) duplicate
    assert!(
        welded.len() < verts.len(),
        "welding must reduce vertex count"
    );
}

#[test]
fn hull_robust_handles_collinear_input() {
    use havok_native::collision::hull::compute_hull_topology_robust;
    // 3 collinear points — robust version must not error
    let verts = vec![[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]];
    let hull =
        compute_hull_topology_robust(&verts).expect("robust hull must handle collinear input");
    assert!(!hull.vertices.is_empty(), "hull must have vertices");
    assert!(!hull.planes.is_empty(), "hull must have planes");
}

#[test]
fn hull_robust_produces_valid_hull_for_cube() {
    use havok_native::collision::hull::compute_hull_topology_robust;
    let verts = cube_verts();
    let hull = compute_hull_topology_robust(&verts).expect("robust hull for cube");
    assert_eq!(hull.vertices.len(), 8, "cube hull must have 8 vertices");
    // Coplanar Quickhull facets are merged into FO4-style polygonal faces.
    assert_eq!(
        hull.planes.len(),
        6,
        "cube hull must have 6 polygonal faces"
    );
}
