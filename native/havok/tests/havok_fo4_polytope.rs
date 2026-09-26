// Integration tests for the FO4 hknpConvexPolytopeShape builder.

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

fn opts(layer: u8, mass: f32) -> BuildOptions {
    BuildOptions {
        friction: 0.5,
        restitution: 0.4,
        layer,
        mass,
        ..BuildOptions::default()
    }
}

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

/// Section header (0x40 bytes): name padded to 0x14 with 0xFF, then abs_start,
/// local/global/virtual fixup and exports offsets (relative to abs_start).
fn read_section_header(data: &[u8], off: usize) -> (String, SectionInfo) {
    let name_bytes: Vec<u8> = data[off..off + 0x14]
        .iter()
        .take_while(|&&b| b != 0 && b != 0xFF)
        .copied()
        .collect();
    (
        String::from_utf8(name_bytes).unwrap_or_default(),
        SectionInfo {
            abs_start: read_u32_le(data, off + 0x14),
            local_fix_rel: read_u32_le(data, off + 0x18),
            global_fix_rel: read_u32_le(data, off + 0x1C),
            virt_fix_rel: read_u32_le(data, off + 0x20),
            exports_rel: read_u32_le(data, off + 0x24),
        },
    )
}

/// Absolute offsets of every object in the virtual fixup table, by class name.
fn virtual_objects(blob: &[u8]) -> Vec<(String, usize)> {
    let (_, data_info) = read_section_header(blob, 0xC0);
    let data_start = data_info.abs_start as usize;
    let exports_abs = data_start + data_info.exports_rel as usize;
    let cn_section_start = 0x100usize;
    let mut objects = Vec::new();
    let mut pos = data_start + data_info.virt_fix_rel as usize;
    while pos + 12 <= exports_abs {
        let obj_rel = read_u32_le(blob, pos);
        if obj_rel == 0xFFFF_FFFF {
            break;
        }
        if read_u32_le(blob, pos + 4) == 0 {
            let name_abs = cn_section_start + read_u32_le(blob, pos + 8) as usize;
            let name_bytes: Vec<u8> = blob[name_abs..]
                .iter()
                .take_while(|&&b| b != 0)
                .copied()
                .collect();
            objects.push((
                String::from_utf8_lossy(&name_bytes).into_owned(),
                data_start + obj_rel as usize,
            ));
        }
        pos += 12;
    }
    objects
}

fn object_abs(blob: &[u8], class: &str) -> usize {
    virtual_objects(blob)
        .into_iter()
        .find(|(name, _)| name == class)
        .map(|(_, abs)| abs)
        .unwrap_or_else(|| panic!("{class} not in virtual fixups"))
}

#[test]
fn polytope_packfile_layout() {
    let blob =
        build_fo4_polytope_collision(&tetrahedron_verts(), &opts(5, 0.0)).expect("build failed");
    assert_eq!(&blob[..8], PF_MAGIC);
    let (name0, info0) = read_section_header(&blob, 0x40);
    let (name1, info1) = read_section_header(&blob, 0x80);
    let (name2, info2) = read_section_header(&blob, 0xC0);
    assert_eq!(
        [name0.as_str(), name1.as_str(), name2.as_str()],
        ["__classnames__", "__types__", "__data__"]
    );
    assert_eq!(info0.abs_start, 0x100);
    assert_eq!(info1.abs_start, info0.abs_start + info0.exports_rel);
    assert_eq!(info2.abs_start, info1.abs_start);
    assert!(info2.exports_rel > 0, "data section must have content");
    assert!(info2.local_fix_rel <= info2.exports_rel);
    assert!(info2.global_fix_rel <= info2.exports_rel);
    assert!(info2.virt_fix_rel <= info2.exports_rel);

    // Per hknpBodyCinfo_2.xml the layer belongs in collisionFilterInfo (0x14),
    // not qualityId (0x10, template default 0xFF). The single-body cinfo sits
    // after the psd (0x80) and body_props (0x50).
    let layer = 7;
    let cube = build_fo4_polytope_collision(&cube_verts(), &opts(layer, 0.0)).expect("build");
    let names: Vec<String> = virtual_objects(&cube).into_iter().map(|(n, _)| n).collect();
    assert!(names.iter().any(|n| n == "hknpPhysicsSystemData"));
    assert!(names.iter().any(|n| n == "hknpConvexPolytopeShape"));
    let cinfo = object_abs(&cube, "hknpPhysicsSystemData") + 0x80 + 0x50;
    assert_ne!(
        cube[cinfo + 0x10],
        layer,
        "layer must not land in qualityId"
    );
    assert_eq!(
        cube[cinfo + 0x14],
        layer,
        "layer must land in collisionFilterInfo"
    );
}

#[test]
fn polytope_mass_reaches_mass_properties_block() {
    let mass_and_volume = |mass: f32| {
        let blob = build_fo4_polytope_collision(&cube_verts(), &opts(5, mass)).expect("build");
        let block = object_abs(&blob, "hknpShapeMassProperties");
        (
            f32::from_le_bytes(blob[block + 0x28..block + 0x2C].try_into().unwrap()),
            f32::from_le_bytes(blob[block + 0x2C..block + 0x30].try_into().unwrap()),
        )
    };
    assert_eq!(
        mass_and_volume(0.0),
        (0.0, 0.0),
        "static body block stays zeroed"
    );
    let (mass, volume) = mass_and_volume(10.0);
    assert!((mass - 10.0).abs() < 1e-4, "mass {mass}");
    assert!((volume - 8.0).abs() < 1e-3, "[-1,1]^3 volume {volume}");
}

#[test]
fn polytope_rejects_degenerate_input_and_hull_helpers_recover() {
    use havok_native::collision::hull::{compute_hull_topology_robust, max_extent, weld_vertices};

    let three = vec![[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    let err = build_fo4_polytope_collision(&three, &opts(5, 0.0)).expect_err("3 vertices");
    assert!(err.to_string().contains('4'), "{err}");
    let coplanar = vec![
        [0.0f32, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.5, 0.5, 0.0],
        [0.25, 0.75, 0.0],
    ];
    assert!(build_fo4_polytope_collision(&coplanar, &opts(5, 0.0)).is_err());

    let jittered = vec![
        [0.0f32, 0.0, 0.0],
        [0.0, 0.0, 0.0001],
        [1.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 1.0],
        [1.0, 0.0, 1.0],
    ];
    let eps = (max_extent(&jittered) * 1e-3).max(1e-3);
    let (welded, _) = weld_vertices(&jittered, eps);
    assert!(
        welded.len() < jittered.len(),
        "near-coincident vertices weld"
    );

    let collinear = vec![[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]];
    let hull = compute_hull_topology_robust(&collinear).expect("robust hull handles collinear");
    assert!(!hull.vertices.is_empty() && !hull.planes.is_empty());
}
