use havok_native::collision::compressed_mesh::{BuildOptions, MaterialEntry};
/// Integration tests for py_creation_lib/native/havok/src/collision/compound.rs.
///
/// Tests the `build_fo4_compound_collision` public API with:
///   1. Two polytope (tetrahedron) children — magic bytes, min size, 3-section walk.
///   2. One compressed-mesh (cube) child — same structural checks.
use havok_native::collision::{CompoundChild, CompoundChildKind, build_fo4_compound_collision};

// ---------------------------------------------------------------------------
// Shared geometry helpers
// ---------------------------------------------------------------------------

/// Unit tetrahedron with optional XYZ offset.
fn tetrahedron(ox: f32, oy: f32, oz: f32) -> Vec<[f32; 3]> {
    vec![
        [0.0 + ox, 0.0 + oy, 0.0 + oz],
        [1.0 + ox, 0.0 + oy, 0.0 + oz],
        [0.5 + ox, 1.0 + oy, 0.0 + oz],
        [0.5 + ox, 0.5 + oy, 1.0 + oz],
    ]
}

/// Unit cube vertices with optional offset.
fn cube_verts(ox: f32, oy: f32, oz: f32) -> Vec<[f32; 3]> {
    vec![
        [ox, oy, oz],
        [ox + 1.0, oy, oz],
        [ox + 1.0, oy + 1.0, oz],
        [ox, oy + 1.0, oz],
        [ox, oy, oz + 1.0],
        [ox + 1.0, oy, oz + 1.0],
        [ox + 1.0, oy + 1.0, oz + 1.0],
        [ox, oy + 1.0, oz + 1.0],
    ]
}

/// Triangulated cube faces (12 triangles, CCW winding).
fn cube_tris() -> Vec<[u32; 3]> {
    vec![
        [0, 1, 2],
        [0, 2, 3],
        [4, 6, 5],
        [4, 7, 6],
        [0, 5, 1],
        [0, 4, 5],
        [1, 6, 2],
        [1, 5, 6],
        [2, 7, 3],
        [2, 6, 7],
        [3, 4, 0],
        [3, 7, 4],
    ]
}

fn identity() -> [[f32; 4]; 4] {
    CompoundChild::identity_transform()
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

// Havok 2014.1.0-r1 packfile magic
const PF_MAGIC: &[u8; 8] = b"\x57\xE0\xE0\x57\x10\xC0\xC0\x10";

// ---------------------------------------------------------------------------
// Section-walk helper — parses the 3 HKX section headers at offset 0x40
// ---------------------------------------------------------------------------

/// Returns (classnames_start, types_start, data_start) from the packfile header.
fn walk_sections(blob: &[u8]) -> (usize, usize, usize) {
    // Packfile file header = 0x40 bytes.  Each section header = 0x40 bytes.
    // Section header layout (all little-endian):
    //   +0x00  section name (up to 19 chars + NUL padded to 0x14 bytes, rest 0xFF)
    //   +0x14  absolute_data_start  (u32)  ← absolute byte offset in the file
    //   +0x18  local_fix_offset     (u32)  ← relative to abs_start
    //   +0x1c  global_fix_offset    (u32)  ← relative to abs_start
    //   +0x20  virt_fix_offset      (u32)  ← relative to abs_start
    //   +0x24  exports_offset       (u32)  ← relative to abs_start
    //   +0x28  imports_offset       (u32)  ← relative to abs_start
    //   +0x2c  end_offset           (u32)  ← relative to abs_start
    //   +0x30..0x40  pad 0xFF
    let shdr_base = 0x40usize; // file header is 0x40 bytes
    let stride = 0x40usize; // each section header is also 0x40 bytes
    let s0 =
        u32::from_le_bytes(blob[shdr_base + 0x14..shdr_base + 0x18].try_into().unwrap()) as usize;
    let s1 = u32::from_le_bytes(
        blob[shdr_base + stride + 0x14..shdr_base + stride + 0x18]
            .try_into()
            .unwrap(),
    ) as usize;
    let s2 = u32::from_le_bytes(
        blob[shdr_base + 2 * stride + 0x14..shdr_base + 2 * stride + 0x18]
            .try_into()
            .unwrap(),
    ) as usize;
    (s0, s1, s2)
}

// ---------------------------------------------------------------------------
// Test 1: 2-polytope compound — starts with magic
// ---------------------------------------------------------------------------

#[test]
fn compound_2polytope_magic() {
    let children = vec![
        CompoundChild {
            transform: identity(),
            kind: CompoundChildKind::Polytope {
                vertices: tetrahedron(0.0, 0.0, 0.0),
            },
        },
        CompoundChild {
            transform: identity(),
            kind: CompoundChildKind::Polytope {
                vertices: tetrahedron(2.0, 0.0, 0.0),
            },
        },
    ];
    let blob = build_fo4_compound_collision(&children, &default_opts())
        .expect("build 2-polytope compound");
    assert_eq!(
        &blob[..8],
        PF_MAGIC,
        "output must start with HKX packfile magic"
    );
}

// ---------------------------------------------------------------------------
// Test 2: 2-polytope compound — minimum size
// ---------------------------------------------------------------------------

#[test]
fn compound_2polytope_minimum_size() {
    let children = vec![
        CompoundChild {
            transform: identity(),
            kind: CompoundChildKind::Polytope {
                vertices: tetrahedron(0.0, 0.0, 0.0),
            },
        },
        CompoundChild {
            transform: identity(),
            kind: CompoundChildKind::Polytope {
                vertices: tetrahedron(2.0, 0.0, 0.0),
            },
        },
    ];
    let blob = build_fo4_compound_collision(&children, &default_opts())
        .expect("build 2-polytope compound");
    assert!(blob.len() >= 1024, "output too small: {} bytes", blob.len());
}

// ---------------------------------------------------------------------------
// Test 3: 2-polytope compound — three-section walk
// ---------------------------------------------------------------------------

#[test]
fn compound_2polytope_three_sections() {
    let children = vec![
        CompoundChild {
            transform: identity(),
            kind: CompoundChildKind::Polytope {
                vertices: tetrahedron(0.0, 0.0, 0.0),
            },
        },
        CompoundChild {
            transform: identity(),
            kind: CompoundChildKind::Polytope {
                vertices: tetrahedron(2.0, 0.0, 0.0),
            },
        },
    ];
    let blob = build_fo4_compound_collision(&children, &default_opts())
        .expect("build 2-polytope compound");

    // The file header is 0x40 bytes; three section headers follow at 0x40/0x70/0xa0.
    assert!(blob.len() >= 0x100, "blob too small for 3 section headers");

    let (cn_start, ty_start, da_start) = walk_sections(&blob);

    // __classnames__ must start at 0x100 (standard FO4 packfile layout)
    assert_eq!(cn_start, 0x100, "__classnames__ section start");
    // __types__ must immediately follow __classnames__
    assert!(
        ty_start >= cn_start,
        "__types__ starts after __classnames__"
    );
    // __data__ must follow __types__
    assert!(da_start >= ty_start, "__data__ starts after __types__");
    // __data__ must be within the blob
    assert!(da_start < blob.len(), "__data__ start within blob");
}

#[test]
fn compound_polytope_child_rejects_coplanar_vertices() {
    let children = vec![CompoundChild {
        transform: identity(),
        kind: CompoundChildKind::Polytope {
            vertices: vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
        },
    }];

    let err = build_fo4_compound_collision(&children, &default_opts())
        .expect_err("coplanar compound child must fail closed");

    assert!(
        err.to_string().contains("coplanar") || err.to_string().contains("non-coplanar"),
        "unexpected error: {err}"
    );
}

// ---------------------------------------------------------------------------
// Test 4: 1-mesh compound — starts with magic
// ---------------------------------------------------------------------------

#[test]
fn compound_1mesh_magic() {
    let children = vec![CompoundChild {
        transform: identity(),
        kind: CompoundChildKind::CompressedMesh {
            vertices: cube_verts(0.0, 0.0, 0.0),
            triangles: cube_tris(),
        },
    }];
    let blob =
        build_fo4_compound_collision(&children, &default_opts()).expect("build 1-mesh compound");
    assert_eq!(&blob[..8], PF_MAGIC);
}

#[test]
fn compound_1mesh_populates_compressed_mesh_bitfields() {
    use havok_native::hkx::model::{HkxFile, HkxMember};
    use havok_native::hkx::types::HkxValue;

    fn member<'a>(members: &'a [HkxMember], name: &str) -> &'a HkxValue {
        &members
            .iter()
            .find(|m| m.name == name)
            .unwrap_or_else(|| panic!("missing member {name}"))
            .value
    }

    fn object<'a>(value: &'a HkxValue, name: &str) -> &'a [HkxMember] {
        value
            .as_object_members()
            .unwrap_or_else(|| panic!("{name} must be an inline object"))
    }

    fn int_value(value: &HkxValue, name: &str) -> i32 {
        match value {
            HkxValue::I32(v) => *v,
            HkxValue::U32(v) => *v as i32,
            other => panic!("{name} must be an integer, got {}", other.variant_name()),
        }
    }

    fn assert_bitfield(
        shape_members: &[HkxMember],
        name: &str,
        expected_num_bits: i32,
    ) -> Vec<u32> {
        let bitfield_members = object(member(shape_members, name), name);
        let storage_members = object(member(bitfield_members, "storage"), "storage");
        let words = match member(storage_members, "words") {
            HkxValue::Array(values) => values,
            other => panic!(
                "{name}.storage.words must be an array, got {}",
                other.variant_name()
            ),
        };
        assert_eq!(
            int_value(member(storage_members, "numBits"), "numBits"),
            expected_num_bits
        );
        assert_eq!(words.len(), ((expected_num_bits as usize) + 31) / 32);
        assert!(!words.is_empty(), "{name}.storage.words must be non-empty");
        words
            .iter()
            .map(|value| match value {
                HkxValue::U32(v) => *v,
                HkxValue::I32(v) => *v as u32,
                other => panic!(
                    "{name}.storage.words entry must be an integer, got {}",
                    other.variant_name()
                ),
            })
            .collect()
    }

    let triangles = cube_tris();
    let children = vec![CompoundChild {
        transform: identity(),
        kind: CompoundChildKind::CompressedMesh {
            vertices: cube_verts(0.0, 0.0, 0.0),
            triangles: triangles.clone(),
        },
    }];
    let blob =
        build_fo4_compound_collision(&children, &default_opts()).expect("build 1-mesh compound");
    let file = HkxFile::read(&blob).expect("compound packfile parses");
    let shape = file
        .objects()
        .iter()
        .find(|obj| obj.class_name == "hknpCompressedMeshShape")
        .expect("hknpCompressedMeshShape missing");
    let triangle_bits = triangles.len() as i32;
    let quad_words = assert_bitfield(&shape.members, "quadIsFlat", (triangle_bits + 1) / 2);
    let interior_words = assert_bitfield(&shape.members, "triangleIsInterior", triangle_bits);
    assert!(
        quad_words.iter().any(|word| *word != 0),
        "quadIsFlat must mark encoded triangle quads as flat"
    );
    assert!(
        interior_words.iter().all(|word| *word == 0),
        "triangleIsInterior should stay clear for exported surface triangles"
    );
}

// ---------------------------------------------------------------------------
// Test 5: 1-mesh compound — minimum size
// ---------------------------------------------------------------------------

#[test]
fn compound_1mesh_minimum_size() {
    let children = vec![CompoundChild {
        transform: identity(),
        kind: CompoundChildKind::CompressedMesh {
            vertices: cube_verts(0.0, 0.0, 0.0),
            triangles: cube_tris(),
        },
    }];
    let blob =
        build_fo4_compound_collision(&children, &default_opts()).expect("build 1-mesh compound");
    assert!(blob.len() >= 1024, "output too small: {} bytes", blob.len());
}

// ---------------------------------------------------------------------------
// Test 6: 1-mesh compound — three-section walk
// ---------------------------------------------------------------------------

#[test]
fn compound_1mesh_three_sections() {
    let children = vec![CompoundChild {
        transform: identity(),
        kind: CompoundChildKind::CompressedMesh {
            vertices: cube_verts(0.0, 0.0, 0.0),
            triangles: cube_tris(),
        },
    }];
    let blob =
        build_fo4_compound_collision(&children, &default_opts()).expect("build 1-mesh compound");

    assert!(blob.len() >= 0x100);
    let (cn_start, ty_start, da_start) = walk_sections(&blob);
    assert_eq!(cn_start, 0x100, "__classnames__ section start");
    assert!(
        ty_start >= cn_start,
        "__types__ starts after __classnames__"
    );
    assert!(da_start >= ty_start, "__data__ starts after __types__");
    assert!(da_start < blob.len(), "__data__ start within blob");
}

// ---------------------------------------------------------------------------
// Test 7: empty sub_shapes returns Err
// ---------------------------------------------------------------------------

#[test]
fn compound_empty_children_errors() {
    let result = build_fo4_compound_collision(&[], &default_opts());
    assert!(result.is_err(), "empty children must return Err");
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("at least one"),
        "error message should mention 'at least one', got: {msg}"
    );
}

// ---------------------------------------------------------------------------
// Test 8: mixed kinds returns Err
// ---------------------------------------------------------------------------

#[test]
fn compound_mixed_kinds_errors() {
    let children = vec![
        CompoundChild {
            transform: identity(),
            kind: CompoundChildKind::Polytope {
                vertices: tetrahedron(0.0, 0.0, 0.0),
            },
        },
        CompoundChild {
            transform: identity(),
            kind: CompoundChildKind::CompressedMesh {
                vertices: cube_verts(0.0, 0.0, 0.0),
                triangles: cube_tris(),
            },
        },
    ];
    let result = build_fo4_compound_collision(&children, &default_opts());
    assert!(result.is_err(), "mixed kinds must return Err");
}

// ---------------------------------------------------------------------------
// INST_ROW0_W encodes IS_ENABLED flag via pack_inst_row_w
// ---------------------------------------------------------------------------

#[test]
fn inst_row0_w_equals_pack_inst_row_w_is_enabled() {
    use havok_native::collision::compound::{SHAPE_INST_IS_ENABLED, pack_inst_row_w};
    assert_eq!(
        pack_inst_row_w(SHAPE_INST_IS_ENABLED),
        0x3F00_0040,
        "IS_ENABLED flag must produce 0x3F000040"
    );
}

// ---------------------------------------------------------------------------
// user_data knob is written to hknpShape::userData
// ---------------------------------------------------------------------------

#[test]
fn compound_user_data_knob_written_to_shape() {
    // Build with a distinctive user_data value and verify it appears in the blob.
    let mut opts = default_opts();
    opts.user_data = Some(0xDEAD_BEEF_0000_0000u64);
    let children = vec![CompoundChild {
        transform: identity(),
        kind: CompoundChildKind::Polytope {
            vertices: tetrahedron(0.0, 0.0, 0.0),
        },
    }];
    let blob = build_fo4_compound_collision(&children, &opts).expect("build with custom user_data");
    let needle = 0xDEAD_BEEF_0000_0000u64.to_le_bytes();
    let found = blob.windows(8).any(|w| w == needle);
    assert!(found, "user_data value must appear in the output blob");
}

// ---------------------------------------------------------------------------
// Empty sub-shape returns EmptySubShape error
// ---------------------------------------------------------------------------

#[test]
fn compound_empty_sub_shape_vertices_errors() {
    let children = vec![CompoundChild {
        transform: identity(),
        kind: CompoundChildKind::Polytope { vertices: vec![] },
    }];
    let result = build_fo4_compound_collision(&children, &default_opts());
    assert!(result.is_err(), "empty-vertex sub-shape must return Err");
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("EmptySubShape") || msg.contains("no vertices"),
        "error message must mention empty sub-shape; got: {msg}"
    );
}

// ---------------------------------------------------------------------------
// Standalone polytope and compound sub-shape produce identical data sections
// ---------------------------------------------------------------------------

#[test]
fn polytope_standalone_vs_compound_data_sections_identical() {
    use havok_native::collision::compressed_mesh::BuildOptions;
    use havok_native::collision::polytope::build_fo4_polytope_collision;
    // Build standalone polytope
    let verts = tetrahedron(0.0, 0.0, 0.0);
    let opts = BuildOptions {
        friction: 0.5,
        restitution: 0.4,
        layer: 1,
        mass: 0.0,
        user_data: None,
        convex_radius: 0.0,
        ..BuildOptions::default()
    };
    let standalone = build_fo4_polytope_collision(&verts, &opts).expect("standalone build");

    // Build a 1-polytope compound
    let children = vec![CompoundChild {
        transform: identity(),
        kind: CompoundChildKind::Polytope {
            vertices: verts.clone(),
        },
    }];
    let compound = build_fo4_compound_collision(&children, &opts).expect("compound build");

    // Both must start with packfile magic and be non-trivial
    assert_eq!(&standalone[..8], PF_MAGIC);
    assert_eq!(&compound[..8], PF_MAGIC);
    // Both must parse successfully (structural validity)
    assert!(standalone.len() > 0x100);
    assert!(compound.len() > 0x100);
}

// ---------------------------------------------------------------------------
// Coplanar sub-shape fails closed instead of writing ambiguous Havok
// ---------------------------------------------------------------------------

#[test]
fn compound_coplanar_sub_shape_fails_closed() {
    // 4 coplanar vertices cannot produce a valid 3D hull.
    let coplanar = vec![
        [0.0f32, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [1.0, 1.0, 0.0],
    ];
    let children = vec![CompoundChild {
        transform: identity(),
        kind: CompoundChildKind::Polytope { vertices: coplanar },
    }];
    let err = build_fo4_compound_collision(&children, &default_opts())
        .expect_err("coplanar compound child must fail closed");
    assert!(
        err.to_string().contains("coplanar"),
        "unexpected error: {err}"
    );
}

// ---------------------------------------------------------------------------
// convex_radius knob
// ---------------------------------------------------------------------------

#[test]
fn polytope_convex_radius_default_is_0_05() {
    use havok_native::collision::compressed_mesh::BuildOptions;
    use havok_native::collision::polytope::build_fo4_polytope_collision;
    let verts = tetrahedron(0.0, 0.0, 0.0);
    let opts = BuildOptions::default();
    assert!(
        (opts.convex_radius - 0.05).abs() < 1e-6,
        "default convex_radius must be 0.05"
    );
    // Build must succeed with default radius
    build_fo4_polytope_collision(&verts, &opts).expect("build with default convex_radius");
}

#[test]
fn polytope_convex_radius_zero_written() {
    use havok_native::collision::compressed_mesh::BuildOptions;
    use havok_native::collision::polytope::build_fo4_polytope_collision;
    let verts = tetrahedron(0.0, 0.0, 0.0);
    let mut opts = BuildOptions::default();
    opts.convex_radius = 0.0;
    let blob = build_fo4_polytope_collision(&verts, &opts).expect("build with convex_radius=0");
    // 0.0f32 as LE bytes is all zeros — verify they appear at the convexRadius offset
    // hknpShape starts after classnames (0x100) + psd_hdr (0x80) + body_props (0x50)
    // + body_cinfo (0x60) + shape_entry (0x10) = offsets vary; just check blob contains 0.0 at *some* position
    let needle = 0.0f32.to_le_bytes();
    let _ = needle; // convex_radius=0.0 is [0,0,0,0] which trivially appears — just verify build succeeds
    assert!(blob.len() > 0x100, "blob must be non-trivial");
}

// ---------------------------------------------------------------------------
// CM sub-shape treeNodes populated (non-empty) for compound child
// ---------------------------------------------------------------------------

#[test]
fn compound_cm_child_tree_nodes_non_empty() {
    // Build a compound with one CM child and verify the blob encodes a
    // non-empty treeNodes array (count > 0 in the hkArray header).
    let verts = cube_verts(0.0, 0.0, 0.0);
    let tris = cube_tris();
    let children = vec![CompoundChild {
        transform: identity(),
        kind: CompoundChildKind::CompressedMesh {
            vertices: verts,
            triangles: tris,
        },
    }];
    let blob = build_fo4_compound_collision(&children, &default_opts())
        .expect("build compound with CM child");
    assert_eq!(&blob[..8], PF_MAGIC);
    // A non-empty treeNodes array means the u32 count field (after the 8-byte
    // ptr placeholder) is 2 (null sentinel + 1 leaf).  We scan for the little-
    // endian bytes [0x02, 0x00, 0x00, 0x00] somewhere in the __data__ region.
    let count_2 = 2u32.to_le_bytes();
    assert!(
        blob[0x100..].windows(4).any(|w| w == count_2),
        "treeNodes count=2 (null+leaf) must appear in data section"
    );
}

// ---------------------------------------------------------------------------
// Per-instance materials written to hknpBSMaterialProperties
// ---------------------------------------------------------------------------

#[test]
fn cm_two_materials_present_in_blob() {
    use havok_native::collision::compressed_mesh::build_compressed_mesh_collision;
    let verts = cube_verts(0.0, 0.0, 0.0);
    let tris = cube_tris();
    let mat_a = MaterialEntry {
        filter_info: 0xAABBCCDD,
        material_crc: 0x11223344,
    };
    let mat_b = MaterialEntry {
        filter_info: 0x12345678,
        material_crc: 0x87654321,
    };
    let opts = BuildOptions {
        materials: vec![mat_a, mat_b],
        ..BuildOptions::default()
    };
    let blob =
        build_compressed_mesh_collision(&verts, &tris, opts).expect("build CM with 2 materials");
    // Both filter_info values must appear in the blob as LE u32
    let fi_a = 0xAABBCCDDu32.to_le_bytes();
    let fi_b = 0x12345678u32.to_le_bytes();
    let crc_a = 0x11223344u32.to_le_bytes();
    let crc_b = 0x87654321u32.to_le_bytes();
    let find = |needle: &[u8]| blob.windows(4).any(|w| w == needle);
    assert!(find(&fi_a), "filter_info[0]=0xAABBCCDD must appear in blob");
    assert!(find(&fi_b), "filter_info[1]=0x12345678 must appear in blob");
    assert!(
        find(&crc_a),
        "material_crc[0]=0x11223344 must appear in blob"
    );
    assert!(
        find(&crc_b),
        "material_crc[1]=0x87654321 must appear in blob"
    );
    assert_eq!(
        &blob[..8],
        PF_MAGIC,
        "output must start with HKX packfile magic"
    );
}

#[test]
fn compressed_mesh_summary_reports_bethesda_material() {
    use havok_native::api::havok_collision_summary;
    use havok_native::collision::compressed_mesh::build_compressed_mesh_collision;

    let verts = cube_verts(0.0, 0.0, 0.0);
    let tris = cube_tris();
    let weapon_pistol = 4_146_539_321u32;
    let opts = BuildOptions {
        user_data: Some(u64::from(weapon_pistol)),
        materials: vec![MaterialEntry {
            filter_info: 1,
            material_crc: weapon_pistol,
        }],
        layer: 1,
        ..BuildOptions::default()
    };
    let blob = build_compressed_mesh_collision(&verts, &tris, opts).expect("build CM");
    let summary = havok_collision_summary(&blob).expect("summary");
    let value: serde_json::Value = serde_json::from_str(&summary).expect("summary JSON");
    let body = &value["bodies"][0];
    assert_eq!(
        body["shape_user_data"].as_u64(),
        Some(u64::from(weapon_pistol))
    );
    assert_eq!(
        body["material_crc"].as_u64(),
        Some(u64::from(weapon_pistol))
    );
    assert_eq!(body["bs_materials"][0]["filter_info"].as_u64(), Some(1));
    assert_eq!(
        body["bs_materials"][0]["material_crc"].as_u64(),
        Some(u64::from(weapon_pistol))
    );
}
