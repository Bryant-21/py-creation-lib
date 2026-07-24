use std::path::PathBuf;

use havok_native::api::havok_collision_summary;
#[allow(unused_imports)]
use havok_native::collision::compressed_mesh;
use havok_native::collision::payload::{parse_tagged_collision, rebuild_tag0_collision};
use havok_native::collision::preview::extract_preview_meshes_from_hkx;
use havok_native::collision::{
    collision_preview_json, parse_fo4_compressed_mesh, parse_tag0_collision_payload,
    unpack_vertex_11_11_10, unpack_vertex_21_21_22,
};
use havok_native::hkx::model::{HkxFile, HkxMember, HkxObject};
use havok_native::hkx::types::HkxValue;

fn repo_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

fn fixture_bytes(relative: &str) -> Vec<u8> {
    std::fs::read(repo_path(relative)).unwrap_or_else(|error| {
        panic!("failed to read fixture {relative}: {error}");
    })
}

#[test]
fn parses_tracked_tag0_collision_payload_scaffold() {
    let blob = fixture_bytes("python/creation_lib/havok/tests/novablast_reference.bin");

    let payload = parse_tag0_collision_payload(&blob).expect("parse TAG0 collision payload");

    assert_eq!(payload.format, "tag0");
    assert!(payload.sections.iter().any(|section| section.tag == "DATA"));
    assert!(payload.items.len() >= 8);
    assert_eq!(payload.vertices.len(), 20);
    assert!(!payload.planes.is_empty());
    assert!(!payload.faces.is_empty());
    assert!(!payload.indices.is_empty());
}

#[test]
fn collision_preview_json_on_unknown_blob_returns_empty_mesh_list() {
    // An unrecognized blob (all zeros) should return an empty mesh list, not an error.
    let blob = vec![0u8; 16];
    let json =
        collision_preview_json(&blob, 10.0, None).expect("collision_preview_json should not error");
    assert!(json.contains("\"meshes\""), "JSON must contain meshes key");
    assert!(
        json.contains("[]"),
        "empty blob should produce empty meshes array"
    );
}

#[test]
fn fo4_compressed_mesh_parser_rejects_non_packfile_without_fake_output() {
    let error = parse_fo4_compressed_mesh(&[0; 16]).expect_err("invalid packfile rejected");

    assert!(error.to_string().contains("missing Havok packfile magic"));
}

// ---------------------------------------------------------------------------
// TAG0 round-trip and box collision build
// ---------------------------------------------------------------------------

#[test]
fn novablast_round_trip_byte_identical() {
    let blob = fixture_bytes("python/creation_lib/havok/tests/novablast_reference.bin");
    let parsed = parse_tagged_collision(&blob).expect("parse TAG0 collision payload");
    let rebuilt = rebuild_tag0_collision(&parsed).expect("rebuild TAG0 collision");
    assert_eq!(
        rebuilt, blob,
        "rebuilt TAG0 blob must be byte-identical to original"
    );
}

#[test]
fn build_box_collision_round_trips() {
    use havok_native::collision::payload::build_convex_collision;
    let verts: Vec<[f32; 3]> = vec![
        [-1.0, -1.0, -1.0],
        [1.0, -1.0, -1.0],
        [-1.0, 1.0, -1.0],
        [1.0, 1.0, -1.0],
        [-1.0, -1.0, 1.0],
        [1.0, -1.0, 1.0],
        [-1.0, 1.0, 1.0],
        [1.0, 1.0, 1.0],
    ];
    let blob = build_convex_collision(&verts).expect("build convex collision");
    assert_eq!(&blob[4..8], b"TAG0", "blob must start with TAG0 magic");
    let parsed = parse_tagged_collision(&blob).expect("parse built TAG0");
    assert_eq!(parsed.vertices.len(), 8);
    assert_eq!(
        parsed.planes.len(),
        6,
        "box hull has 6 merged polygonal faces"
    );
    // Planes should be unit normals
    for plane in &parsed.planes {
        let mag = (plane[0] * plane[0] + plane[1] * plane[1] + plane[2] * plane[2]).sqrt();
        assert!(
            (mag - 1.0).abs() < 0.01,
            "plane normal magnitude {mag} not close to 1.0"
        );
    }
}

// ---------------------------------------------------------------------------
// FO4 compressed mesh build → parse round-trip
// ---------------------------------------------------------------------------

fn unit_cube_vertices() -> Vec<[f32; 3]> {
    vec![
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 1.0],
        [1.0, 0.0, 1.0],
        [1.0, 1.0, 1.0],
        [0.0, 1.0, 1.0],
    ]
}

fn unit_cube_triangles() -> Vec<[u32; 3]> {
    vec![
        [0, 1, 2],
        [0, 2, 3], // bottom
        [4, 6, 5],
        [4, 7, 6], // top
        [0, 5, 1],
        [0, 4, 5], // front
        [1, 6, 2],
        [1, 5, 6], // right
        [2, 7, 3],
        [2, 6, 7], // back
        [3, 4, 0],
        [3, 7, 4], // left
    ]
}

fn assert_vertices_match_unordered(expected: &[[f32; 3]], actual: &[[f32; 3]]) {
    assert_eq!(actual.len(), expected.len(), "vertex count preserved");
    let mut matched = vec![false; actual.len()];
    for exp in expected {
        let found = actual.iter().enumerate().find(|(idx, got)| {
            !matched[*idx] && (0..3).all(|axis| (exp[axis] - got[axis]).abs() < 0.01)
        });
        if let Some((idx, _)) = found {
            matched[idx] = true;
        } else {
            panic!("missing recovered vertex near {exp:?}");
        }
    }
}

#[test]
fn fo4_compressed_mesh_build_then_parse_recovers_geometry() {
    use havok_native::collision::compressed_mesh::{
        BuildOptions, build_compressed_mesh_collision, parse_fo4_compressed_mesh,
    };
    let vertices = unit_cube_vertices();
    let triangles = unit_cube_triangles();
    let bytes = build_compressed_mesh_collision(&vertices, &triangles, BuildOptions::default())
        .expect("build compressed mesh collision");
    let mesh = parse_fo4_compressed_mesh(&bytes).expect("parse compressed mesh");
    assert_eq!(mesh.sections.len(), 1);
    let section = &mesh.sections[0];
    assert_eq!(section.triangles.len(), triangles.len());
    assert_vertices_match_unordered(&vertices, &section.vertices);
}

#[test]
fn fo4_compressed_mesh_rejects_repeated_vertex_triangle() {
    use havok_native::collision::compressed_mesh::{BuildOptions, build_compressed_mesh_collision};
    let vertices = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    let triangles = vec![[0, 0, 2]];

    let err = build_compressed_mesh_collision(&vertices, &triangles, BuildOptions::default())
        .expect_err("repeated triangle indices must fail");

    assert!(err.to_string().contains("repeated"));
}

#[test]
fn fo4_compressed_mesh_rejects_zero_area_triangle() {
    use havok_native::collision::compressed_mesh::{BuildOptions, build_compressed_mesh_collision};
    let vertices = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]];
    let triangles = vec![[0, 1, 2]];

    let err = build_compressed_mesh_collision(&vertices, &triangles, BuildOptions::default())
        .expect_err("zero-area triangle must fail");

    assert!(err.to_string().contains("near-zero area"));
}

#[test]
fn fo4_compressed_mesh_pack_helpers_round_trip() {
    use havok_native::collision::compressed_mesh::{pack_vertex_11_11_10, pack_vertex_21_21_22};
    use havok_native::collision::{unpack_vertex_11_11_10, unpack_vertex_21_21_22};

    let packed = pack_vertex_11_11_10(100, 200, 300);
    assert_eq!(unpack_vertex_11_11_10(packed), (100, 200, 300));

    let packed = pack_vertex_11_11_10(0, 0, 0);
    assert_eq!(unpack_vertex_11_11_10(packed), (0, 0, 0));

    let packed = pack_vertex_11_11_10(2047, 2047, 1023);
    assert_eq!(unpack_vertex_11_11_10(packed), (2047, 2047, 1023));

    let packed64 = pack_vertex_21_21_22(1000, 2000, 3000);
    assert_eq!(unpack_vertex_21_21_22(packed64), (1000, 2000, 3000));
}

// ---------------------------------------------------------------------------
// fo4_compressed_mesh_collision_blob byte-identity vs Python
// ---------------------------------------------------------------------------

/// Verify that the Rust builder produces a parseable blob and that geometry
/// round-trips within 11-11-10 quantization tolerance.  Byte-identity vs the
/// Python implementation is verified by the Python pytest suite
/// (py_creation_lib/python/creation_lib/havok/tests/test_collision_payload.py::test_compressed_mesh_roundtrip_rust_matches_python).
#[test]
fn c10_fo4_compressed_mesh_blob_is_parseable_packfile() {
    use havok_native::collision::compressed_mesh::{
        BuildOptions, build_compressed_mesh_collision, parse_fo4_compressed_mesh,
    };

    let verts: Vec<[f32; 3]> = vec![
        [0.0, 0.0, 0.0],
        [2.0, 0.0, 0.0],
        [2.0, 3.0, 0.0],
        [0.0, 3.0, 0.0],
        [0.0, 0.0, 1.5],
        [2.0, 0.0, 1.5],
        [2.0, 3.0, 1.5],
        [0.0, 3.0, 1.5],
    ];
    let tris: Vec<[u32; 3]> = vec![
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
    ];
    let opts = BuildOptions {
        friction: 0.5,
        restitution: 0.4,
        layer: 5,
        mass: 0.0,
        ..BuildOptions::default()
    };
    let blob = build_compressed_mesh_collision(&verts, &tris, opts)
        .expect("build_fo4_compressed_mesh_collision must succeed");

    // Must start with Havok packfile magic
    assert_eq!(
        &blob[0..8],
        b"\x57\xE0\xE0\x57\x10\xC0\xC0\x10",
        "blob must start with Havok packfile magic"
    );

    // Round-trip: parse back and verify geometry
    let mesh = parse_fo4_compressed_mesh(&blob).expect("parse built compressed mesh blob");
    assert_eq!(mesh.sections.len(), 1, "single-section blob");
    let sec = &mesh.sections[0];
    assert_eq!(sec.triangles.len(), tris.len(), "triangle count preserved");
    assert_vertices_match_unordered(&verts, &sec.vertices);
}

// ---------------------------------------------------------------------------
// Regression: vanilla FO4 physics packfiles use padding_size=0 (section table
// starts at 0x40, NOT 0x50). Auto-emitting padding_size=16 shifts every
// section header by 16 bytes; the in-game Havok runtime hardcodes 0x40 as the
// section table offset (only animation HKX loaders honor the padding byte), so
// it reads garbage and crashes when the broadphase derefs a null sub-shape.
// ---------------------------------------------------------------------------

#[test]
fn fo4_physics_packfile_emits_padding_size_zero() {
    use havok_native::collision::compound::{CompoundChild, CompoundChildKind};
    use havok_native::collision::compressed_mesh::BuildOptions;
    use havok_native::collision::multi_body::MultiBodyShape;
    use havok_native::collision::{
        build_compressed_mesh_collision, build_fo4_compound_collision,
        build_fo4_multi_body_collision, build_fo4_polytope_collision,
    };

    let verts: Vec<[f32; 3]> = unit_cube_vertices();
    let tris: Vec<[u32; 3]> = unit_cube_triangles();
    let opts = BuildOptions::default();

    let mut blobs: Vec<(&'static str, Vec<u8>)> = Vec::new();
    blobs.push((
        "polytope",
        build_fo4_polytope_collision(&verts, &opts).expect("polytope"),
    ));
    blobs.push((
        "compressed_mesh",
        build_compressed_mesh_collision(&verts, &tris, opts.clone()).expect("compressed mesh"),
    ));
    blobs.push((
        "compound",
        build_fo4_compound_collision(
            &[CompoundChild {
                transform: CompoundChild::identity_transform(),
                kind: CompoundChildKind::Polytope {
                    vertices: verts.clone(),
                },
            }],
            &opts,
        )
        .expect("compound"),
    ));
    blobs.push((
        "multi_body[polytope]",
        build_fo4_multi_body_collision(
            &[MultiBodyShape::Polytope {
                vertices: verts.clone(),
            }],
            &opts,
            None,
            None,
        )
        .expect("multi_body polytope"),
    ));
    blobs.push((
        "multi_body[compressed_mesh]",
        build_fo4_multi_body_collision(
            &[MultiBodyShape::CompressedMesh {
                vertices: verts.clone(),
                triangles: tris.clone(),
            }],
            &opts,
            None,
            None,
        )
        .expect("multi_body mesh"),
    ));

    for (tag, blob) in &blobs {
        assert_eq!(
            blob[0x3E], 0x00,
            "{tag}: padding byte at 0x3E must be 0 for FO4 physics packfile (vanilla layout); \
             non-zero shifts the section table from 0x40 to 0x50 and crashes the in-game Havok runtime"
        );
        // __classnames__ section name must start at 0x40 (no header padding).
        let cn_off = blob
            .windows(14)
            .position(|w| w == b"__classnames__")
            .unwrap_or(usize::MAX);
        assert_eq!(
            cn_off, 0x40,
            "{tag}: __classnames__ must start at 0x40 (vanilla layout), got 0x{cn_off:x}"
        );
    }
}

// ---------------------------------------------------------------------------
// Regression: hknpCompressedMeshShapeData.m_simdTree.m_nodes must always have
// at least 2 entries. The in-game Havok runtime's hkcdSimdTree::isEmpty() reads
// `m_nodes[1].isAllocated()` directly (no bounds check), so an empty m_nodes
// array means a null-data ptr is dereferenced, crashing in workshop-placement
// sphere casts. Vanilla FO4 set-dressing meshes (e.g. Safe01.nif) always emit
// 2 zero-cleared nodes (224 bytes total), even when the tree is "logically
// empty". The cleared layout uses Havok's +/-HK_REAL_HIGH sentinel pattern.
// ---------------------------------------------------------------------------

#[test]
fn fo4_compressed_mesh_emits_at_least_two_simdtree_nodes() {
    use havok_native::collision::compressed_mesh::{BuildOptions, build_compressed_mesh_collision};
    use havok_native::collision::multi_body::MultiBodyShape;
    use havok_native::collision::{build_fo4_multi_body_collision, parse_fo4_compressed_mesh};
    use havok_native::hkx::model::HkxFile;
    use havok_native::hkx::types::HkxValue;

    fn assert_simdtree_two_cleared_nodes(blob: &[u8], tag: &str) {
        let file = HkxFile::read(blob).expect("packfile parses");
        let data_obj = file
            .objects()
            .iter()
            .find(|obj| obj.class_name == "hknpCompressedMeshShapeData")
            .unwrap_or_else(|| panic!("{tag}: hknpCompressedMeshShapeData missing"));
        let simd_member = data_obj
            .members
            .iter()
            .find(|m| m.name == "simdTree")
            .unwrap_or_else(|| panic!("{tag}: simdTree member missing"));
        let HkxValue::Object(simd_members) = &simd_member.value else {
            panic!("{tag}: simdTree must be an inline struct");
        };
        let nodes_member = simd_members
            .iter()
            .find(|m| m.name == "nodes")
            .unwrap_or_else(|| panic!("{tag}: simdTree.nodes missing"));
        let HkxValue::Array(nodes) = &nodes_member.value else {
            panic!("{tag}: simdTree.nodes must be an array");
        };
        assert!(
            nodes.len() >= 2,
            "{tag}: simdTree.nodes must have at least 2 entries (got {}); \
             the FO4 Havok runtime reads m_nodes[1] directly in isEmpty()",
            nodes.len()
        );
        // Sanity: ensure parse round-trip still recovers the geometry.
        let mesh =
            parse_fo4_compressed_mesh(blob).unwrap_or_else(|e| panic!("{tag}: parse failed: {e}"));
        assert!(
            !mesh.sections.is_empty(),
            "{tag}: must have at least 1 section"
        );
    }

    let verts: Vec<[f32; 3]> = unit_cube_vertices();
    let tris: Vec<[u32; 3]> = unit_cube_triangles();
    let opts = BuildOptions::default();

    let direct =
        build_compressed_mesh_collision(&verts, &tris, opts.clone()).expect("direct build");
    assert_simdtree_two_cleared_nodes(&direct, "direct compressed_mesh");

    let multi_body = build_fo4_multi_body_collision(
        &[MultiBodyShape::CompressedMesh {
            vertices: verts.clone(),
            triangles: tris.clone(),
        }],
        &opts,
        None,
        None,
    )
    .expect("multi_body build");
    assert_simdtree_two_cleared_nodes(&multi_body, "multi_body compressed_mesh");
}

#[test]
fn fo4_compressed_mesh_populates_shape_bitfields() {
    use havok_native::collision::compressed_mesh::{BuildOptions, build_compressed_mesh_collision};
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
        let num_bits = int_value(member(storage_members, "numBits"), "numBits");
        assert_eq!(num_bits, expected_num_bits, "{name}.storage.numBits");
        assert_eq!(
            words.len(),
            ((expected_num_bits as usize) + 31) / 32,
            "{name}.storage.words count"
        );
        assert!(
            !words.is_empty(),
            "{name}.storage.words must have a real backing array"
        );
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

    let verts: Vec<[f32; 3]> = unit_cube_vertices();
    let tris: Vec<[u32; 3]> = unit_cube_triangles();
    let blob = build_compressed_mesh_collision(&verts, &tris, BuildOptions::default())
        .expect("build compressed mesh");
    let file = HkxFile::read(&blob).expect("packfile parses");
    let shape = file
        .objects()
        .iter()
        .find(|obj| obj.class_name == "hknpCompressedMeshShape")
        .expect("hknpCompressedMeshShape missing");

    let triangle_bits = tris.len() as i32;
    let quad_bits = (triangle_bits + 1) / 2;
    let quad_words = assert_bitfield(&shape.members, "quadIsFlat", quad_bits);
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
// Collision preview mesh extraction
// ---------------------------------------------------------------------------

fn make_box_hkx_file() -> HkxFile {
    let half_extents_member = HkxMember {
        name: "halfExtents".to_string(),
        value: HkxValue::F32List(vec![1.0_f32, 2.0, 3.0, 0.0]),
    };
    let convex_radius_member = HkxMember {
        name: "convexRadius".to_string(),
        value: HkxValue::F32(0.125),
    };
    let box_object = HkxObject {
        name: Some("#0001".to_string()),
        offset: 0,
        signature: 0,
        class_name: "hknpBoxShape".to_string(),
        members: vec![half_extents_member, convex_radius_member],
    };
    HkxFile::from_tagxml(11, "hk_2014.1.0-r1", vec![box_object])
}

#[test]
fn extract_preview_meshes_from_hkx_box_shape() {
    let hkx = make_box_hkx_file();
    let meshes = extract_preview_meshes_from_hkx(&hkx, 10.0, None);
    assert_eq!(meshes.len(), 1);
    assert_eq!(meshes[0].shape_type, "box");
    assert_eq!(meshes[0].vertices.len(), 8);
    assert_eq!(meshes[0].triangles.len(), 12);
    // First vertex should be (-hx, -hy, -hz) scaled: (-1*10, -2*10, -3*10)
    assert!((meshes[0].vertices[0][0] - (-10.0_f32)).abs() < 1e-5);
    assert!((meshes[0].vertices[0][1] - (-20.0_f32)).abs() < 1e-5);
    assert!((meshes[0].vertices[0][2] - (-30.0_f32)).abs() < 1e-5);
}

#[test]
fn extract_preview_meshes_from_hkx_body_scopes_hknp_compound_shape_children() {
    let body = HkxValue::Object(vec![HkxMember {
        name: "shape".to_string(),
        value: HkxValue::Pointer(Some(1)),
    }]);
    let physics_system = HkxObject {
        name: Some("#0001".to_string()),
        offset: 0,
        signature: 0,
        class_name: "hknpPhysicsSystemData".to_string(),
        members: vec![HkxMember {
            name: "bodyCinfos".to_string(),
            value: HkxValue::Array(vec![body]),
        }],
    };
    let compound = HkxObject {
        name: Some("#0002".to_string()),
        offset: 0,
        signature: 0,
        class_name: "hknpCompoundShape".to_string(),
        members: vec![HkxMember {
            name: "instances".to_string(),
            value: HkxValue::Object(vec![HkxMember {
                name: "elements".to_string(),
                value: HkxValue::Array(vec![
                    HkxValue::Object(vec![HkxMember {
                        name: "shape".to_string(),
                        value: HkxValue::Pointer(Some(2)),
                    }]),
                    HkxValue::Object(vec![HkxMember {
                        name: "shape".to_string(),
                        value: HkxValue::Pointer(Some(3)),
                    }]),
                ]),
            }]),
        }],
    };
    let capsule_a = make_capsule_hkx_object("#0003", [0.0, 0.0, 0.0], [1.0, 0.0, 0.0]);
    let capsule_b = make_capsule_hkx_object("#0004", [0.0, 1.0, 0.0], [1.0, 1.0, 0.0]);
    let hkx = HkxFile::from_tagxml(
        11,
        "hk_2015.1.0-r1",
        vec![physics_system, compound, capsule_a, capsule_b],
    );

    let meshes = extract_preview_meshes_from_hkx(&hkx, 10.0, Some(0));

    assert_eq!(meshes.len(), 2);
    assert!(meshes.iter().all(|mesh| mesh.shape_type == "capsule"));
    assert!(meshes.iter().all(|mesh| !mesh.vertices.is_empty()));
    assert!(meshes.iter().all(|mesh| !mesh.triangles.is_empty()));
}

#[test]
fn extract_preview_meshes_from_hkx_does_not_fallback_to_whole_blob_for_empty_compound() {
    let body = HkxValue::Object(vec![HkxMember {
        name: "shape".to_string(),
        value: HkxValue::Pointer(Some(1)),
    }]);
    let physics_system = HkxObject {
        name: Some("#0001".to_string()),
        offset: 0,
        signature: 0,
        class_name: "hknpPhysicsSystemData".to_string(),
        members: vec![HkxMember {
            name: "bodyCinfos".to_string(),
            value: HkxValue::Array(vec![body]),
        }],
    };
    let empty_compound = HkxObject {
        name: Some("#0002".to_string()),
        offset: 0,
        signature: 0,
        class_name: "hknpCompoundShape".to_string(),
        members: vec![HkxMember {
            name: "instances".to_string(),
            value: HkxValue::Object(vec![HkxMember {
                name: "elements".to_string(),
                value: HkxValue::Array(vec![]),
            }]),
        }],
    };
    let unrelated_capsule = make_capsule_hkx_object("#0003", [0.0, 0.0, 0.0], [1.0, 0.0, 0.0]);
    let hkx = HkxFile::from_tagxml(
        11,
        "hk_2015.1.0-r1",
        vec![physics_system, empty_compound, unrelated_capsule],
    );

    let scoped = extract_preview_meshes_from_hkx(&hkx, 10.0, Some(0));
    let unscoped = extract_preview_meshes_from_hkx(&hkx, 10.0, None);

    assert!(
        scoped.is_empty(),
        "body-local empty compound must fail closed"
    );
    assert_eq!(
        unscoped.len(),
        1,
        "unscoped preview still sees unrelated shapes"
    );
}

#[test]
fn extract_preview_meshes_from_hkx_uses_body_local_compound_backing_data() {
    let physics_system = HkxObject {
        name: Some("#0001".to_string()),
        offset: 0,
        signature: 0,
        class_name: "hknpPhysicsSystemData".to_string(),
        members: vec![HkxMember {
            name: "bodyCinfos".to_string(),
            value: HkxValue::Array(vec![
                HkxValue::Object(vec![HkxMember {
                    name: "shape".to_string(),
                    value: HkxValue::Pointer(Some(1)),
                }]),
                HkxValue::Object(vec![HkxMember {
                    name: "shape".to_string(),
                    value: HkxValue::Pointer(Some(2)),
                }]),
            ]),
        }],
    };
    let compound_a = empty_compound_with_backing("#0002", 3);
    let compound_b = empty_compound_with_backing("#0003", 6);
    let backing_a = dynamic_compound_backing_data("#0004", 2);
    let leaf_a0 = make_capsule_hkx_object("#0005", [0.0, 0.0, 0.0], [1.0, 0.0, 0.0]);
    let leaf_a1 = make_capsule_hkx_object("#0006", [2.0, 0.0, 0.0], [3.0, 0.0, 0.0]);
    let backing_b = dynamic_compound_backing_data("#0007", 1);
    let leaf_b0 = make_capsule_hkx_object("#0008", [100.0, 0.0, 0.0], [101.0, 0.0, 0.0]);
    let hkx = HkxFile::from_tagxml(
        11,
        "hk_2015.1.0-r1",
        vec![
            physics_system,
            compound_a,
            compound_b,
            backing_a,
            leaf_a0,
            leaf_a1,
            backing_b,
            leaf_b0,
        ],
    );

    let body0 = extract_preview_meshes_from_hkx(&hkx, 10.0, Some(0));
    let body1 = extract_preview_meshes_from_hkx(&hkx, 10.0, Some(1));

    assert_eq!(body0.len(), 2);
    assert_eq!(body1.len(), 1);
    let body0_max_x = body0
        .iter()
        .flat_map(|mesh| mesh.vertices.iter().map(|vertex| vertex[0]))
        .fold(f32::NEG_INFINITY, f32::max);
    let body1_min_x = body1[0]
        .vertices
        .iter()
        .map(|vertex| vertex[0])
        .fold(f32::INFINITY, f32::min);
    assert!(
        body0_max_x < 40.0,
        "body 0 should not steal body 1's leaf, got max_x={body0_max_x}"
    );
    assert!(
        body1_min_x > 990.0,
        "body 1 should resolve only its own backing-data leaf, got min_x={body1_min_x}"
    );
}

#[test]
fn extract_preview_meshes_from_hkx_bakes_compound_instance_translation() {
    let body = HkxValue::Object(vec![HkxMember {
        name: "shape".to_string(),
        value: HkxValue::Pointer(Some(1)),
    }]);
    let physics_system = HkxObject {
        name: Some("#0001".to_string()),
        offset: 0,
        signature: 0,
        class_name: "hknpPhysicsSystemData".to_string(),
        members: vec![HkxMember {
            name: "bodyCinfos".to_string(),
            value: HkxValue::Array(vec![body]),
        }],
    };
    let compound = HkxObject {
        name: Some("#0002".to_string()),
        offset: 0,
        signature: 0,
        class_name: "hknpCompoundShape".to_string(),
        members: vec![HkxMember {
            name: "instances".to_string(),
            value: HkxValue::Object(vec![HkxMember {
                name: "elements".to_string(),
                value: HkxValue::Array(vec![HkxValue::Object(vec![
                    HkxMember {
                        name: "shape".to_string(),
                        value: HkxValue::Pointer(Some(2)),
                    },
                    HkxMember {
                        name: "transform".to_string(),
                        value: HkxValue::F32List(vec![
                            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 2.0, 0.0,
                            0.0, 1.0,
                        ]),
                    },
                ])]),
            }]),
        }],
    };
    let capsule = make_capsule_hkx_object("#0003", [0.0, 0.0, 0.0], [1.0, 0.0, 0.0]);
    let hkx = HkxFile::from_tagxml(
        11,
        "hk_2015.1.0-r1",
        vec![physics_system, compound, capsule],
    );

    let meshes = extract_preview_meshes_from_hkx(&hkx, 10.0, Some(0));

    assert_eq!(meshes.len(), 1);
    let min_x = meshes[0]
        .vertices
        .iter()
        .map(|v| v[0])
        .fold(f32::INFINITY, f32::min);
    assert!(
        min_x > 15.0,
        "instance translation should move capsule by about +20 game units, got min_x={min_x}"
    );
}

#[test]
fn extract_preview_meshes_from_hkx_bakes_compound_instance_rotation() {
    let body = HkxValue::Object(vec![HkxMember {
        name: "shape".to_string(),
        value: HkxValue::Pointer(Some(1)),
    }]);
    let physics_system = HkxObject {
        name: Some("#0001".to_string()),
        offset: 0,
        signature: 0,
        class_name: "hknpPhysicsSystemData".to_string(),
        members: vec![HkxMember {
            name: "bodyCinfos".to_string(),
            value: HkxValue::Array(vec![body]),
        }],
    };
    let compound = HkxObject {
        name: Some("#0002".to_string()),
        offset: 0,
        signature: 0,
        class_name: "hknpCompoundShape".to_string(),
        members: vec![HkxMember {
            name: "instances".to_string(),
            value: HkxValue::Object(vec![HkxMember {
                name: "elements".to_string(),
                value: HkxValue::Array(vec![HkxValue::Object(vec![
                    HkxMember {
                        name: "shape".to_string(),
                        value: HkxValue::Pointer(Some(2)),
                    },
                    HkxMember {
                        name: "transform".to_string(),
                        value: HkxValue::F32List(vec![
                            0.0, 1.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0,
                            0.0, 1.0,
                        ]),
                    },
                ])]),
            }]),
        }],
    };
    let capsule = make_capsule_hkx_object("#0003", [0.0, 0.0, 0.0], [1.0, 0.0, 0.0]);
    let hkx = HkxFile::from_tagxml(
        11,
        "hk_2015.1.0-r1",
        vec![physics_system, compound, capsule],
    );

    let meshes = extract_preview_meshes_from_hkx(&hkx, 10.0, Some(0));

    assert_eq!(meshes.len(), 1);
    let min_x = meshes[0]
        .vertices
        .iter()
        .map(|v| v[0])
        .fold(f32::INFINITY, f32::min);
    let max_x = meshes[0]
        .vertices
        .iter()
        .map(|v| v[0])
        .fold(f32::NEG_INFINITY, f32::max);
    let min_y = meshes[0]
        .vertices
        .iter()
        .map(|v| v[1])
        .fold(f32::INFINITY, f32::min);
    let max_y = meshes[0]
        .vertices
        .iter()
        .map(|v| v[1])
        .fold(f32::NEG_INFINITY, f32::max);
    let x_span = max_x - min_x;
    let y_span = max_y - min_y;
    assert!(
        y_span > x_span * 2.0 && max_y > 9.0,
        "column-major instance rotation should rotate the capsule onto +Y, got x_span={x_span} y_span={y_span} max_y={max_y}"
    );
}

#[test]
fn extract_preview_meshes_from_hkx_ignores_packed_shape_instance_w_fields() {
    let body = HkxValue::Object(vec![HkxMember {
        name: "shape".to_string(),
        value: HkxValue::Pointer(Some(1)),
    }]);
    let physics_system = HkxObject {
        name: Some("#0001".to_string()),
        offset: 0,
        signature: 0,
        class_name: "hknpPhysicsSystemData".to_string(),
        members: vec![HkxMember {
            name: "bodyCinfos".to_string(),
            value: HkxValue::Array(vec![body]),
        }],
    };
    let compound = HkxObject {
        name: Some("#0002".to_string()),
        offset: 0,
        signature: 0,
        class_name: "hknpCompoundShape".to_string(),
        members: vec![HkxMember {
            name: "instances".to_string(),
            value: HkxValue::Object(vec![HkxMember {
                name: "elements".to_string(),
                value: HkxValue::Array(vec![HkxValue::Object(vec![
                    HkxMember {
                        name: "shape".to_string(),
                        value: HkxValue::Pointer(Some(2)),
                    },
                    HkxMember {
                        name: "transform".to_string(),
                        value: HkxValue::F32List(vec![
                            1.0,
                            0.0,
                            0.0,
                            f32::from_bits(0x3F00_0040),
                            0.0,
                            1.0,
                            0.0,
                            0.0,
                            0.0,
                            0.0,
                            1.0,
                            f32::from_bits(0x3F00_0090),
                            0.0,
                            0.0,
                            0.0,
                            f32::from_bits(0x3F00_0001),
                        ]),
                    },
                ])]),
            }]),
        }],
    };
    let capsule = make_capsule_hkx_object("#0003", [0.0, 0.0, 0.0], [1.0, 0.0, 0.0]);
    let hkx = HkxFile::from_tagxml(
        11,
        "hk_2015.1.0-r1",
        vec![physics_system, compound, capsule],
    );

    let meshes = extract_preview_meshes_from_hkx(&hkx, 10.0, Some(0));

    assert_eq!(meshes.len(), 1);
    let min_x = meshes[0]
        .vertices
        .iter()
        .map(|v| v[0])
        .fold(f32::INFINITY, f32::min);
    assert!(
        min_x < 1.0,
        "packed W flag fields must not translate the child, got min_x={min_x}"
    );
}

#[test]
fn extract_preview_meshes_from_hkx_decodes_hknp_convex_shape_vertices() {
    let body = HkxValue::Object(vec![HkxMember {
        name: "shape".to_string(),
        value: HkxValue::Pointer(Some(1)),
    }]);
    let physics_system = HkxObject {
        name: Some("#0001".to_string()),
        offset: 0,
        signature: 0,
        class_name: "hknpPhysicsSystemData".to_string(),
        members: vec![HkxMember {
            name: "bodyCinfos".to_string(),
            value: HkxValue::Array(vec![body]),
        }],
    };
    let convex = HkxObject {
        name: Some("#0002".to_string()),
        offset: 0,
        signature: 0,
        class_name: "hknpConvexShape".to_string(),
        members: vec![
            HkxMember {
                name: "convexRadius".to_string(),
                value: HkxValue::F32(0.0),
            },
            HkxMember {
                name: "vertices".to_string(),
                value: HkxValue::Array(vec![
                    HkxValue::F32List(vec![-1.0, 0.0, 0.0, 0.5]),
                    HkxValue::F32List(vec![1.0, 0.0, 0.0, 0.5]),
                    HkxValue::F32List(vec![1.0, 2.0, 0.0, 0.5]),
                    HkxValue::F32List(vec![-1.0, 2.0, 0.0, 0.5]),
                ]),
            },
        ],
    };
    let hkx = HkxFile::from_tagxml(11, "hk_2015.1.0-r1", vec![physics_system, convex]);

    let meshes = extract_preview_meshes_from_hkx(&hkx, 10.0, Some(0));

    assert_eq!(meshes.len(), 1);
    assert_eq!(meshes[0].shape_type, "convex_hull");
    assert_eq!(meshes[0].vertices.len(), 4);
    assert_eq!(meshes[0].triangles.len(), 2);
    assert!(
        meshes[0].vertices.iter().any(|vertex| vertex[0] <= -10.0),
        "vertices should be decoded from hknpConvexShape, got {:?}",
        meshes[0].vertices
    );
}

#[test]
fn extract_preview_meshes_from_hkx_body_scopes_scaled_convex_shape() {
    let body = HkxValue::Object(vec![HkxMember {
        name: "shape".to_string(),
        value: HkxValue::Pointer(Some(1)),
    }]);
    let physics_system = HkxObject {
        name: Some("#0001".to_string()),
        offset: 0,
        signature: 0,
        class_name: "hknpPhysicsSystemData".to_string(),
        members: vec![HkxMember {
            name: "bodyCinfos".to_string(),
            value: HkxValue::Array(vec![body]),
        }],
    };
    let scaled = HkxObject {
        name: Some("#0002".to_string()),
        offset: 0,
        signature: 0,
        class_name: "hknpScaledConvexShape".to_string(),
        members: vec![
            HkxMember {
                name: "coreShape".to_string(),
                value: HkxValue::String {
                    value: "#0003".to_string(),
                    is_null: false,
                },
            },
            HkxMember {
                name: "scale".to_string(),
                value: HkxValue::F32List(vec![2.0, 3.0, 4.0, 0.0]),
            },
            HkxMember {
                name: "translation".to_string(),
                value: HkxValue::F32List(vec![0.5, -1.0, 0.25, 0.0]),
            },
        ],
    };
    let core = HkxObject {
        name: Some("#0003".to_string()),
        offset: 0,
        signature: 0,
        class_name: "hknpConvexPolytopeShape".to_string(),
        members: vec![HkxMember {
            name: "vertices".to_string(),
            value: HkxValue::Array(vec![
                HkxValue::F32List(vec![1.0, 0.0, 0.0, 0.0]),
                HkxValue::F32List(vec![0.0, 1.0, 0.0, 0.0]),
                HkxValue::F32List(vec![0.0, 0.0, 1.0, 0.0]),
                HkxValue::F32List(vec![-1.0, -1.0, -1.0, 0.0]),
            ]),
        }],
    };
    let hkx = HkxFile::from_tagxml(11, "hk_2015.1.0-r1", vec![physics_system, scaled, core]);

    let meshes = extract_preview_meshes_from_hkx(&hkx, 10.0, Some(0));

    assert_eq!(meshes.len(), 1);
    assert_eq!(meshes[0].shape_type, "convex_hull");
    assert!(!meshes[0].triangles.is_empty());
    assert_vertices_match_unordered(
        &[
            [25.0, -10.0, 2.5],
            [5.0, 20.0, 2.5],
            [5.0, -10.0, 42.5],
            [-15.0, -40.0, -37.5],
        ],
        &meshes[0].vertices,
    );
}

fn empty_compound_with_backing(name: &str, backing_data_index: usize) -> HkxObject {
    HkxObject {
        name: Some(name.to_string()),
        offset: 0,
        signature: 0,
        class_name: "hknpCompoundShape".to_string(),
        members: vec![
            HkxMember {
                name: "instances".to_string(),
                value: HkxValue::Object(vec![HkxMember {
                    name: "elements".to_string(),
                    value: HkxValue::Array(vec![]),
                }]),
            },
            HkxMember {
                name: "boundingVolumeData".to_string(),
                value: HkxValue::Pointer(Some(backing_data_index)),
            },
        ],
    }
}

fn dynamic_compound_backing_data(name: &str, num_leaves: u32) -> HkxObject {
    HkxObject {
        name: Some(name.to_string()),
        offset: 0,
        signature: 0,
        class_name: "hknpDynamicCompoundShapeData".to_string(),
        members: vec![HkxMember {
            name: "aabbTree".to_string(),
            value: HkxValue::Object(vec![HkxMember {
                name: "numLeaves".to_string(),
                value: HkxValue::U32(num_leaves),
            }]),
        }],
    }
}

fn make_capsule_hkx_object(name: &str, a: [f32; 3], b: [f32; 3]) -> HkxObject {
    HkxObject {
        name: Some(name.to_string()),
        offset: 0,
        signature: 0,
        class_name: "hknpCapsuleShape".to_string(),
        members: vec![
            HkxMember {
                name: "a".to_string(),
                value: HkxValue::F32List(vec![a[0], a[1], a[2], 0.0]),
            },
            HkxMember {
                name: "b".to_string(),
                value: HkxValue::F32List(vec![b[0], b[1], b[2], 0.0]),
            },
            HkxMember {
                name: "convexRadius".to_string(),
                value: HkxValue::F32(0.25),
            },
        ],
    }
}

#[test]
fn collision_preview_json_serializes_box_shape() {
    // Verify the JSON structure using the novablast fixture (a real TAG0 blob):
    // it should either parse a shape or return empty meshes.
    let novablast = fixture_bytes("python/creation_lib/havok/tests/novablast_reference.bin");
    let json = collision_preview_json(&novablast, 10.0, None)
        .expect("collision_preview_json must not fail on novablast blob");
    // Must be valid JSON with the expected shape
    assert!(json.starts_with('{'), "JSON output must start with {{");
    assert!(json.contains("\"meshes\""), "JSON must contain meshes key");
    // Parse to verify structure: each mesh entry is {shape_type, mesh: {vertices, triangles}}.
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("must be valid JSON");
    assert!(
        parsed.get("meshes").is_some(),
        "top-level key 'meshes' must exist"
    );
    let meshes = parsed["meshes"]
        .as_array()
        .expect("meshes must be an array");
    if !meshes.is_empty() {
        let entry = &meshes[0];
        assert!(
            entry.get("shape_type").is_some(),
            "each entry must have shape_type"
        );
        let inner = entry
            .get("mesh")
            .expect("each entry must have nested 'mesh' object");
        assert!(inner.get("vertices").is_some(), "mesh.vertices must exist");
        assert!(
            inner.get("triangles").is_some(),
            "mesh.triangles must exist"
        );
    }
}

#[test]
fn extract_preview_meshes_from_hknp_convex_polytope() {
    // Construct a synthetic hknpConvexPolytopeShape object.
    // vertices array: 4 vertices forming a tetrahedron.
    let v0 = HkxValue::F32List(vec![0.0_f32, 0.0, 0.0, 0.0]);
    let v1 = HkxValue::F32List(vec![1.0_f32, 0.0, 0.0, 0.0]);
    let v2 = HkxValue::F32List(vec![0.0_f32, 1.0, 0.0, 0.0]);
    let v3 = HkxValue::F32List(vec![0.0_f32, 0.0, 1.0, 0.0]);
    let vertices_member = HkxMember {
        name: "vertices".to_string(),
        value: HkxValue::Array(vec![v0, v1, v2, v3]),
    };
    let poly_object = HkxObject {
        name: Some("#0002".to_string()),
        offset: 0,
        signature: 0,
        class_name: "hknpConvexPolytopeShape".to_string(),
        members: vec![vertices_member],
    };
    let hkx = HkxFile::from_tagxml(11, "hk_2014.1.0-r1", vec![poly_object]);
    let meshes = extract_preview_meshes_from_hkx(&hkx, 1.0, None);
    assert_eq!(meshes.len(), 1);
    assert_eq!(meshes[0].shape_type, "convex_hull");
    assert_eq!(meshes[0].vertices.len(), 4);
    // With no faces/indices, a fan triangulation is used: 2 triangles from 4 vertices.
    assert!(!meshes[0].triangles.is_empty());
}

// ---------------------------------------------------------------------------

#[test]
fn compressed_mesh_vertex_unpack_helpers_match_python_reference_cases() {
    assert_eq!(unpack_vertex_11_11_10(0), (0, 0, 0));
    assert_eq!(unpack_vertex_11_11_10(0xFFFF_FFFF), (2047, 2047, 1023));
    assert_eq!(
        unpack_vertex_11_11_10((300 << 22) | (200 << 11) | 100),
        (100, 200, 300)
    );

    assert_eq!(unpack_vertex_21_21_22(0), (0, 0, 0));
    assert_eq!(
        unpack_vertex_21_21_22(u64::MAX),
        ((1 << 21) - 1, (1 << 21) - 1, (1 << 22) - 1)
    );
    assert_eq!(
        unpack_vertex_21_21_22((3000u64 << 42) | (2000u64 << 21) | 1000),
        (1000, 2000, 3000)
    );
}

// ---------------------------------------------------------------------------
// havok_collision_summary
// ---------------------------------------------------------------------------

/// Build a minimal FO4 convex-polytope packfile using the polytope builder,
/// then verify that havok_collision_summary emits the shape expected by
/// `ui/editor/panels/collision_info.py::_parse_packfile_summary`.
#[test]
fn collision_summary_convex_polytope_shape_kind_and_class_names() {
    use havok_native::collision::compressed_mesh::BuildOptions;
    use havok_native::collision::polytope::build_fo4_polytope_collision;

    let verts = vec![
        [0.0f32, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 1.0],
    ];
    let opts = BuildOptions {
        friction: 0.5,
        restitution: 0.4,
        layer: 1,
        mass: 0.0,
        ..BuildOptions::default()
    };
    let blob = build_fo4_polytope_collision(&verts, &opts)
        .expect("build_fo4_polytope_collision should succeed");

    let json_str = havok_collision_summary(&blob)
        .expect("havok_collision_summary must not fail on a valid FO4 polytope packfile");
    let v: serde_json::Value =
        serde_json::from_str(&json_str).expect("havok_collision_summary must return valid JSON");

    // shape_kind must be "convex_polytope" — this drives the panel's if-branch.
    assert_eq!(
        v["shape_kind"], "convex_polytope",
        "shape_kind must be 'convex_polytope' for a single hknpConvexPolytopeShape blob"
    );

    // blob_size must match the actual blob length.
    assert_eq!(
        v["blob_size"],
        blob.len() as u64,
        "blob_size must equal the byte length of the input"
    );

    // objects must be a non-empty array.
    let objects = v["objects"]
        .as_array()
        .expect("objects must be a JSON array");
    assert!(!objects.is_empty(), "objects array must not be empty");

    // The panel branches on exact class_name strings.  Objects[1] must be
    // "hknpConvexPolytopeShape" for the convex_polytope classification to hold.
    let class_names: Vec<&str> = objects
        .iter()
        .map(|o| o["class_name"].as_str().unwrap_or(""))
        .collect();
    assert!(
        class_names.contains(&"hknpPhysicsSystemData"),
        "objects must contain hknpPhysicsSystemData; got: {class_names:?}"
    );
    assert!(
        class_names.contains(&"hknpConvexPolytopeShape"),
        "objects must contain hknpConvexPolytopeShape; got: {class_names:?}"
    );

    // n_subshapes must be null for a single convex polytope (no compound).
    assert!(
        v["n_subshapes"].is_null(),
        "n_subshapes must be null for a non-compound shape"
    );

    // Each object entry must carry the four count fields (null is acceptable
    // when the member is absent in that class).
    for obj in objects {
        assert!(
            obj.get("class_name").is_some(),
            "each object must have class_name"
        );
        assert!(
            obj.get("n_vertices").is_some(),
            "each object must have n_vertices key"
        );
        assert!(
            obj.get("n_faces").is_some(),
            "each object must have n_faces key"
        );
        assert!(
            obj.get("n_planes").is_some(),
            "each object must have n_planes key"
        );
        assert!(
            obj.get("n_instances").is_some(),
            "each object must have n_instances key"
        );
    }

    // The hknpConvexPolytopeShape object must report non-null vertex and face counts.
    let polytope_obj = objects
        .iter()
        .find(|o| o["class_name"] == "hknpConvexPolytopeShape")
        .expect("hknpConvexPolytopeShape object must be present");
    assert!(
        !polytope_obj["n_vertices"].is_null(),
        "hknpConvexPolytopeShape must report n_vertices"
    );
}

/// havok_collision_summary must return an error (not panic) on a non-packfile blob.
#[test]
fn collision_summary_rejects_non_packfile() {
    let result = havok_collision_summary(b"\x00\x01\x02\x03\x04\x05\x06\x07");
    assert!(result.is_err(), "invalid blob must produce an error");
}

// ---------------------------------------------------------------------------
// convex_hull_simple — convex hull path for NIF collision shape creation.
// ---------------------------------------------------------------------------

#[test]
fn convex_hull_simple_unit_cube_has_eight_vertices_and_outward_planes() {
    use havok_native::api::convex_hull_simple;

    let verts: Vec<[f32; 3]> = vec![
        [-1.0, -1.0, -1.0],
        [1.0, -1.0, -1.0],
        [-1.0, 1.0, -1.0],
        [1.0, 1.0, -1.0],
        [-1.0, -1.0, 1.0],
        [1.0, -1.0, 1.0],
        [-1.0, 1.0, 1.0],
        [1.0, 1.0, 1.0],
    ];
    let (hull_verts, planes) = convex_hull_simple(&verts).expect("hull computed");

    assert_eq!(hull_verts.len(), 8, "unit cube has 8 hull vertices");
    assert!(!planes.is_empty(), "hull must produce at least one plane");

    let center = [0.0_f32, 0.0, 0.0];

    for (i, plane) in planes.iter().enumerate() {
        let n = [plane[0], plane[1], plane[2]];
        let mag = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        assert!(
            (mag - 1.0).abs() < 1e-4,
            "plane {i} normal must be unit length, got mag={mag}",
        );

        // offset is the Python convention (-d_scipy). For interior point P:
        // sign(n·P - offset) must be negative (interior on the negative side
        // of the outward-facing plane).
        let signed = n[0] * center[0] + n[1] * center[1] + n[2] * center[2] - plane[3];
        assert!(
            signed < -0.5,
            "cube center must be strictly inside plane {i} (signed={signed})",
        );

        // Every input vertex must satisfy n·v ≤ offset (lie on or behind
        // the outward-facing plane).
        for (vi, v) in verts.iter().enumerate() {
            let lhs = n[0] * v[0] + n[1] * v[1] + n[2] * v[2];
            assert!(
                lhs - plane[3] <= 1e-3,
                "vertex {vi} {v:?} must lie behind plane {i} (lhs={lhs}, offset={})",
                plane[3],
            );
        }
    }
}

#[test]
fn convex_hull_simple_rejects_coplanar_input() {
    use havok_native::api::convex_hull_simple;

    let verts: Vec<[f32; 3]> = vec![
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [1.0, 1.0, 0.0],
    ];
    assert!(
        convex_hull_simple(&verts).is_err(),
        "coplanar input must produce an error, not a degenerate hull",
    );
}
