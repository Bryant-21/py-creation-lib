use std::path::PathBuf;

use havok_native::api::havok_collision_summary;
use havok_native::collision::compressed_mesh::{
    BuildOptions, build_compressed_mesh_collision, pack_vertex_11_11_10, pack_vertex_21_21_22,
};
use havok_native::collision::multi_body::MultiBodyShape;
use havok_native::collision::payload::{parse_tagged_collision, rebuild_tag0_collision};
use havok_native::collision::preview::extract_preview_meshes_from_hkx;
use havok_native::collision::{
    build_fo4_multi_body_collision, collision_preview_json, parse_fo4_compressed_mesh,
    parse_tag0_collision_payload, unpack_vertex_11_11_10, unpack_vertex_21_21_22,
};
use havok_native::hkx::model::{HkxFile, HkxMember, HkxObject};
use havok_native::hkx::types::HkxValue;

const PF_MAGIC: &[u8; 8] = b"\x57\xE0\xE0\x57\x10\xC0\xC0\x10";

fn novablast_blob() -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../python/creation_lib/havok/tests/novablast_reference.bin");
    std::fs::read(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

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

fn assert_vertices_match_unordered(expected: &[[f32; 3]], actual: &[[f32; 3]]) {
    assert_eq!(actual.len(), expected.len(), "vertex count preserved");
    let mut matched = vec![false; actual.len()];
    for exp in expected {
        let (idx, _) = actual
            .iter()
            .enumerate()
            .find(|(idx, got)| {
                !matched[*idx] && (0..3).all(|axis| (exp[axis] - got[axis]).abs() < 0.01)
            })
            .unwrap_or_else(|| panic!("missing recovered vertex near {exp:?}"));
        matched[idx] = true;
    }
}

fn member<'a>(members: &'a [HkxMember], name: &str) -> &'a HkxValue {
    &members
        .iter()
        .find(|m| m.name == name)
        .unwrap_or_else(|| panic!("missing member {name}"))
        .value
}

fn object(name: &str, class_name: &str, members: Vec<HkxMember>) -> HkxObject {
    HkxObject {
        name: Some(name.to_string()),
        offset: 0,
        signature: 0,
        class_name: class_name.to_string(),
        members,
    }
}

fn mem(name: &str, value: HkxValue) -> HkxMember {
    HkxMember {
        name: name.to_string(),
        value,
    }
}

fn vec4s(values: &[[f32; 4]]) -> HkxValue {
    HkxValue::Array(
        values
            .iter()
            .map(|v| HkxValue::F32List(v.to_vec()))
            .collect(),
    )
}

fn physics_system(body_shapes: &[usize]) -> HkxObject {
    let bodies = body_shapes
        .iter()
        .map(|index| HkxValue::Object(vec![mem("shape", HkxValue::Pointer(Some(*index)))]))
        .collect();
    object(
        "#0001",
        "hknpPhysicsSystemData",
        vec![mem("bodyCinfos", HkxValue::Array(bodies))],
    )
}

fn capsule(name: &str, a: [f32; 3], b: [f32; 3]) -> HkxObject {
    object(
        name,
        "hknpCapsuleShape",
        vec![
            mem("a", HkxValue::F32List(vec![a[0], a[1], a[2], 0.0])),
            mem("b", HkxValue::F32List(vec![b[0], b[1], b[2], 0.0])),
            mem("convexRadius", HkxValue::F32(0.25)),
        ],
    )
}

fn compound(name: &str, instances: Vec<Vec<HkxMember>>, extra: Vec<HkxMember>) -> HkxObject {
    let mut members = vec![mem(
        "instances",
        HkxValue::Object(vec![mem(
            "elements",
            HkxValue::Array(instances.into_iter().map(HkxValue::Object).collect()),
        )]),
    )];
    members.extend(extra);
    object(name, "hknpCompoundShape", members)
}

fn backing_data(name: &str, num_leaves: u32) -> HkxObject {
    object(
        name,
        "hknpDynamicCompoundShapeData",
        vec![mem(
            "aabbTree",
            HkxValue::Object(vec![mem("numLeaves", HkxValue::U32(num_leaves))]),
        )],
    )
}

fn axis_range(vertices: &[[f32; 3]], axis: usize) -> (f32, f32) {
    vertices
        .iter()
        .map(|v| v[axis])
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), x| {
            (lo.min(x), hi.max(x))
        })
}

#[test]
fn tag0_payload_parses_and_round_trips_byte_identical() {
    let blob = novablast_blob();
    let payload = parse_tag0_collision_payload(&blob).expect("parse TAG0 collision payload");
    assert_eq!(payload.format, "tag0");
    assert!(payload.sections.iter().any(|section| section.tag == "DATA"));
    assert!(payload.items.len() >= 8);
    assert_eq!(payload.vertices.len(), 20);
    assert!(!payload.planes.is_empty() && !payload.faces.is_empty() && !payload.indices.is_empty());

    let parsed = parse_tagged_collision(&blob).expect("parse TAG0 collision payload");
    assert_eq!(rebuild_tag0_collision(&parsed).expect("rebuild"), blob);

    let json: serde_json::Value =
        serde_json::from_str(&collision_preview_json(&blob, 10.0, None).expect("preview json"))
            .expect("valid JSON");
    for entry in json["meshes"].as_array().expect("meshes array") {
        assert!(entry.get("shape_type").is_some());
        assert!(
            entry["mesh"].get("vertices").is_some() && entry["mesh"].get("triangles").is_some()
        );
    }
}

#[test]
fn build_box_convex_collision_round_trips() {
    use havok_native::collision::payload::build_convex_collision;
    let verts: Vec<[f32; 3]> = unit_cube_vertices()
        .into_iter()
        .map(|v| [v[0] * 2.0 - 1.0, v[1] * 2.0 - 1.0, v[2] * 2.0 - 1.0])
        .collect();
    let blob = build_convex_collision(&verts).expect("build convex collision");
    assert_eq!(&blob[4..8], b"TAG0");
    let parsed = parse_tagged_collision(&blob).expect("parse built TAG0");
    assert_eq!(parsed.vertices.len(), 8);
    assert_eq!(
        parsed.planes.len(),
        6,
        "box hull has 6 merged polygonal faces"
    );
    for plane in &parsed.planes {
        let mag = (plane[0] * plane[0] + plane[1] * plane[1] + plane[2] * plane[2]).sqrt();
        assert!((mag - 1.0).abs() < 0.01, "plane normal magnitude {mag}");
    }
}

#[test]
fn compressed_mesh_build_parse_and_runtime_layout() {
    let verts = unit_cube_vertices();
    let tris = unit_cube_triangles();
    let opts = BuildOptions {
        friction: 0.5,
        restitution: 0.4,
        layer: 5,
        ..BuildOptions::default()
    };
    let direct = build_compressed_mesh_collision(&verts, &tris, opts.clone()).expect("build");
    assert_eq!(&direct[..8], PF_MAGIC);
    let mesh = parse_fo4_compressed_mesh(&direct).expect("parse compressed mesh");
    assert_eq!(mesh.sections.len(), 1);
    assert_eq!(mesh.sections[0].triangles.len(), tris.len());
    assert_vertices_match_unordered(&verts, &mesh.sections[0].vertices);

    let file = HkxFile::read(&direct).expect("packfile parses");
    let shape = file
        .objects()
        .iter()
        .find(|obj| obj.class_name == "hknpCompressedMeshShape")
        .expect("hknpCompressedMeshShape missing");
    let words = |name: &str, bits: i32| -> Vec<u32> {
        let storage = member(
            member(&shape.members, name).as_object_members().unwrap(),
            "storage",
        )
        .as_object_members()
        .unwrap();
        assert!(
            matches!(member(storage, "numBits"), HkxValue::I32(v) if *v == bits)
                || matches!(member(storage, "numBits"), HkxValue::U32(v) if *v as i32 == bits),
            "{name}.numBits"
        );
        let HkxValue::Array(words) = member(storage, "words") else {
            panic!("{name}.words must be an array");
        };
        assert_eq!(
            words.len(),
            (bits as usize).div_ceil(32),
            "{name} word count"
        );
        words
            .iter()
            .map(|w| match w {
                HkxValue::U32(v) => *v,
                HkxValue::I32(v) => *v as u32,
                other => panic!("{name} word {}", other.variant_name()),
            })
            .collect()
    };
    let triangle_bits = tris.len() as i32;
    assert!(
        words("quadIsFlat", (triangle_bits + 1) / 2)
            .iter()
            .any(|w| *w != 0)
    );
    assert!(
        words("triangleIsInterior", triangle_bits)
            .iter()
            .all(|w| *w == 0)
    );

    // hkcdSimdTree::isEmpty() reads m_nodes[1] without a bounds check, so an
    // empty node array null-derefs in workshop-placement sphere casts; vanilla
    // always ships at least 2 cleared nodes.
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
    for (tag, blob) in [("direct", &direct), ("multi_body", &multi_body)] {
        let file = HkxFile::read(blob).expect("packfile parses");
        let data = file
            .objects()
            .iter()
            .find(|obj| obj.class_name == "hknpCompressedMeshShapeData")
            .unwrap_or_else(|| panic!("{tag}: shape data missing"));
        let simd_tree = member(&data.members, "simdTree")
            .as_object_members()
            .unwrap();
        let HkxValue::Array(nodes) = member(simd_tree, "nodes") else {
            panic!("{tag}: simdTree.nodes must be an array");
        };
        assert!(
            nodes.len() >= 2,
            "{tag}: simdTree.nodes has {}",
            nodes.len()
        );
        assert!(!parse_fo4_compressed_mesh(blob).unwrap().sections.is_empty());
    }
}

/// Vanilla FO4 physics packfiles use padding_size=0 (section table at 0x40).
/// The in-game runtime hardcodes 0x40 (only animation loaders honor the padding
/// byte), so padding_size=16 shifts every header and crashes the broadphase.
#[test]
fn fo4_physics_packfile_emits_padding_size_zero() {
    use havok_native::collision::compound::{CompoundChild, CompoundChildKind};
    use havok_native::collision::{build_fo4_compound_collision, build_fo4_polytope_collision};

    let verts = unit_cube_vertices();
    let tris = unit_cube_triangles();
    let opts = BuildOptions::default();
    let blobs = [
        (
            "polytope",
            build_fo4_polytope_collision(&verts, &opts).expect("polytope"),
        ),
        (
            "compressed_mesh",
            build_compressed_mesh_collision(&verts, &tris, opts.clone()).expect("mesh"),
        ),
        (
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
        ),
        (
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
        ),
        (
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
        ),
    ];
    for (tag, blob) in &blobs {
        assert_eq!(blob[0x3E], 0x00, "{tag}: padding byte at 0x3E");
        let classnames = blob.windows(14).position(|w| w == b"__classnames__");
        assert_eq!(
            classnames,
            Some(0x40),
            "{tag}: section table must start at 0x40"
        );
    }
}

#[test]
fn vertex_pack_unpack_helpers() {
    for (x, y, z) in [(0, 0, 0), (100, 200, 300), (2047, 2047, 1023)] {
        assert_eq!(
            unpack_vertex_11_11_10(pack_vertex_11_11_10(x, y, z)),
            (x, y, z)
        );
    }
    assert_eq!(unpack_vertex_11_11_10(0xFFFF_FFFF), (2047, 2047, 1023));
    assert_eq!(
        unpack_vertex_11_11_10((300 << 22) | (200 << 11) | 100),
        (100, 200, 300)
    );
    assert_eq!(
        unpack_vertex_21_21_22(pack_vertex_21_21_22(1000, 2000, 3000)),
        (1000, 2000, 3000)
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

#[test]
fn preview_decodes_leaf_shapes() {
    let box_file = HkxFile::from_tagxml(
        11,
        "hk_2014.1.0-r1",
        vec![object(
            "#0001",
            "hknpBoxShape",
            vec![
                mem("halfExtents", HkxValue::F32List(vec![1.0, 2.0, 3.0, 0.0])),
                mem("convexRadius", HkxValue::F32(0.125)),
            ],
        )],
    );
    let meshes = extract_preview_meshes_from_hkx(&box_file, 10.0, None);
    assert_eq!(meshes.len(), 1);
    assert_eq!(meshes[0].shape_type, "box");
    assert_eq!(
        (meshes[0].vertices.len(), meshes[0].triangles.len()),
        (8, 12)
    );
    assert_eq!(meshes[0].vertices[0], [-10.0, -20.0, -30.0]);

    // Polytope without faces/indices falls back to a fan triangulation.
    let tetra = HkxFile::from_tagxml(
        11,
        "hk_2014.1.0-r1",
        vec![object(
            "#0002",
            "hknpConvexPolytopeShape",
            vec![mem(
                "vertices",
                vec4s(&[
                    [0.0, 0.0, 0.0, 0.0],
                    [1.0, 0.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0, 0.0],
                ]),
            )],
        )],
    );
    let meshes = extract_preview_meshes_from_hkx(&tetra, 1.0, None);
    assert_eq!(meshes.len(), 1);
    assert_eq!(meshes[0].shape_type, "convex_hull");
    assert_eq!(meshes[0].vertices.len(), 4);
    assert!(!meshes[0].triangles.is_empty());

    let convex = HkxFile::from_tagxml(
        11,
        "hk_2015.1.0-r1",
        vec![
            physics_system(&[1]),
            object(
                "#0002",
                "hknpConvexShape",
                vec![
                    mem("convexRadius", HkxValue::F32(0.0)),
                    mem(
                        "vertices",
                        vec4s(&[
                            [-1.0, 0.0, 0.0, 0.5],
                            [1.0, 0.0, 0.0, 0.5],
                            [1.0, 2.0, 0.0, 0.5],
                            [-1.0, 2.0, 0.0, 0.5],
                        ]),
                    ),
                ],
            ),
        ],
    );
    let meshes = extract_preview_meshes_from_hkx(&convex, 10.0, Some(0));
    assert_eq!(meshes.len(), 1);
    assert_eq!(meshes[0].shape_type, "convex_hull");
    assert_eq!(
        (meshes[0].vertices.len(), meshes[0].triangles.len()),
        (4, 2)
    );
    assert!(meshes[0].vertices.iter().any(|v| v[0] <= -10.0));

    // Scaled convex: core shape resolved by name, scale then translation.
    let scaled = HkxFile::from_tagxml(
        11,
        "hk_2015.1.0-r1",
        vec![
            physics_system(&[1]),
            object(
                "#0002",
                "hknpScaledConvexShape",
                vec![
                    mem(
                        "coreShape",
                        HkxValue::String {
                            value: "#0003".to_string(),
                            is_null: false,
                        },
                    ),
                    mem("scale", HkxValue::F32List(vec![2.0, 3.0, 4.0, 0.0])),
                    mem("translation", HkxValue::F32List(vec![0.5, -1.0, 0.25, 0.0])),
                ],
            ),
            object(
                "#0003",
                "hknpConvexPolytopeShape",
                vec![mem(
                    "vertices",
                    vec4s(&[
                        [1.0, 0.0, 0.0, 0.0],
                        [0.0, 1.0, 0.0, 0.0],
                        [0.0, 0.0, 1.0, 0.0],
                        [-1.0, -1.0, -1.0, 0.0],
                    ]),
                )],
            ),
        ],
    );
    let meshes = extract_preview_meshes_from_hkx(&scaled, 10.0, Some(0));
    assert_eq!(meshes.len(), 1);
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

#[test]
fn preview_scopes_compound_children_to_their_body() {
    let shape_instance = |index: usize| vec![mem("shape", HkxValue::Pointer(Some(index)))];
    let two_children = HkxFile::from_tagxml(
        11,
        "hk_2015.1.0-r1",
        vec![
            physics_system(&[1]),
            compound("#0002", vec![shape_instance(2), shape_instance(3)], vec![]),
            capsule("#0003", [0.0, 0.0, 0.0], [1.0, 0.0, 0.0]),
            capsule("#0004", [0.0, 1.0, 0.0], [1.0, 1.0, 0.0]),
        ],
    );
    let meshes = extract_preview_meshes_from_hkx(&two_children, 10.0, Some(0));
    assert_eq!(meshes.len(), 2);
    assert!(
        meshes
            .iter()
            .all(|m| m.shape_type == "capsule" && !m.triangles.is_empty())
    );

    let empty = HkxFile::from_tagxml(
        11,
        "hk_2015.1.0-r1",
        vec![
            physics_system(&[1]),
            compound("#0002", vec![], vec![]),
            capsule("#0003", [0.0, 0.0, 0.0], [1.0, 0.0, 0.0]),
        ],
    );
    assert!(
        extract_preview_meshes_from_hkx(&empty, 10.0, Some(0)).is_empty(),
        "a body-local empty compound must not fall back to the whole blob"
    );
    assert_eq!(extract_preview_meshes_from_hkx(&empty, 10.0, None).len(), 1);

    // Each empty-instance compound resolves only its own backing-data leaves.
    let backing = |index: usize| vec![mem("boundingVolumeData", HkxValue::Pointer(Some(index)))];
    let two_bodies = HkxFile::from_tagxml(
        11,
        "hk_2015.1.0-r1",
        vec![
            physics_system(&[1, 2]),
            compound("#0002", vec![], backing(3)),
            compound("#0003", vec![], backing(6)),
            backing_data("#0004", 2),
            capsule("#0005", [0.0, 0.0, 0.0], [1.0, 0.0, 0.0]),
            capsule("#0006", [2.0, 0.0, 0.0], [3.0, 0.0, 0.0]),
            backing_data("#0007", 1),
            capsule("#0008", [100.0, 0.0, 0.0], [101.0, 0.0, 0.0]),
        ],
    );
    let body0 = extract_preview_meshes_from_hkx(&two_bodies, 10.0, Some(0));
    let body1 = extract_preview_meshes_from_hkx(&two_bodies, 10.0, Some(1));
    assert_eq!((body0.len(), body1.len()), (2, 1));
    let body0_max_x = body0
        .iter()
        .map(|m| axis_range(&m.vertices, 0).1)
        .fold(f32::NEG_INFINITY, f32::max);
    assert!(
        body0_max_x < 40.0,
        "body 0 stole body 1's leaf: {body0_max_x}"
    );
    assert!(axis_range(&body1[0].vertices, 0).0 > 990.0);
}

#[test]
fn preview_bakes_compound_instance_transforms() {
    let packed_w = |bits: u32| f32::from_bits(bits);
    let translation = vec![
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 2.0, 0.0, 0.0, 1.0,
    ];
    let rotation = vec![
        0.0, 1.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];
    // The W lanes of a packed hknpShapeInstance carry flags, not translation.
    let flag_lanes = vec![
        1.0,
        0.0,
        0.0,
        packed_w(0x3F00_0040),
        0.0,
        1.0,
        0.0,
        0.0,
        0.0,
        0.0,
        1.0,
        packed_w(0x3F00_0090),
        0.0,
        0.0,
        0.0,
        packed_w(0x3F00_0001),
    ];
    type Check = fn(&[[f32; 3]]) -> bool;
    let cases: [(&str, Vec<f32>, Check); 3] = [
        ("translation", translation, |v| axis_range(v, 0).0 > 15.0),
        ("column-major rotation onto +Y", rotation, |v| {
            let (x0, x1) = axis_range(v, 0);
            let (_, y1) = axis_range(v, 1);
            let (y0, _) = axis_range(v, 1);
            (y1 - y0) > (x1 - x0) * 2.0 && y1 > 9.0
        }),
        ("packed W flags ignored", flag_lanes, |v| {
            axis_range(v, 0).0 < 1.0
        }),
    ];
    for (label, transform, check) in cases {
        let hkx = HkxFile::from_tagxml(
            11,
            "hk_2015.1.0-r1",
            vec![
                physics_system(&[1]),
                compound(
                    "#0002",
                    vec![vec![
                        mem("shape", HkxValue::Pointer(Some(2))),
                        mem("transform", HkxValue::F32List(transform)),
                    ]],
                    vec![],
                ),
                capsule("#0003", [0.0, 0.0, 0.0], [1.0, 0.0, 0.0]),
            ],
        );
        let meshes = extract_preview_meshes_from_hkx(&hkx, 10.0, Some(0));
        assert_eq!(meshes.len(), 1, "{label}");
        assert!(
            check(&meshes[0].vertices),
            "{label}: {:?}",
            meshes[0].vertices
        );
    }
}

/// The editor collision panel branches on `shape_kind` and exact class names.
#[test]
fn collision_summary_reports_convex_polytope() {
    use havok_native::collision::polytope::build_fo4_polytope_collision;
    let verts = vec![
        [0.0f32, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 1.0],
    ];
    let blob = build_fo4_polytope_collision(&verts, &BuildOptions::default()).expect("build");
    let v: serde_json::Value =
        serde_json::from_str(&havok_collision_summary(&blob).expect("summary")).expect("JSON");
    assert_eq!(v["shape_kind"], "convex_polytope");
    assert_eq!(v["blob_size"], blob.len() as u64);
    assert!(v["n_subshapes"].is_null());
    let objects = v["objects"].as_array().expect("objects array");
    let class_names: Vec<&str> = objects
        .iter()
        .map(|o| o["class_name"].as_str().unwrap_or(""))
        .collect();
    assert!(
        class_names.contains(&"hknpPhysicsSystemData"),
        "{class_names:?}"
    );
    assert!(
        class_names.contains(&"hknpConvexPolytopeShape"),
        "{class_names:?}"
    );
    for obj in objects {
        for key in ["n_vertices", "n_faces", "n_planes", "n_instances"] {
            assert!(obj.get(key).is_some(), "{key} missing");
        }
    }
    let polytope = objects
        .iter()
        .find(|o| o["class_name"] == "hknpConvexPolytopeShape")
        .unwrap();
    assert!(!polytope["n_vertices"].is_null());
}

#[test]
fn garbage_input_errors_or_returns_empty_without_panicking() {
    let json = collision_preview_json(&[0u8; 16], 10.0, None).expect("preview must not error");
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["meshes"].as_array().map(Vec::len), Some(0));

    let error = parse_fo4_compressed_mesh(&[0; 16]).expect_err("invalid packfile rejected");
    assert!(error.to_string().contains("missing Havok packfile magic"));
    assert!(havok_collision_summary(b"\x00\x01\x02\x03\x04\x05\x06\x07").is_err());
    assert!(havok_native::api::validate_collision_blob(b"NOT_A_HAVOK_BLOB", "{}").is_err());
    assert!(
        havok_native::collision::convert_fo76_embedded_static_collision_direct(PF_MAGIC)
            .expect_err("a packfile is not a FO76 TAG0 payload")
            .to_string()
            .contains("input is not an embedded TAG0 tagfile")
    );
}

#[test]
fn convex_hull_simple_builds_outward_planes_and_rejects_coplanar() {
    use havok_native::api::convex_hull_simple;
    let verts: Vec<[f32; 3]> = unit_cube_vertices()
        .into_iter()
        .map(|v| [v[0] * 2.0 - 1.0, v[1] * 2.0 - 1.0, v[2] * 2.0 - 1.0])
        .collect();
    let (hull_verts, planes) = convex_hull_simple(&verts).expect("hull computed");
    assert_eq!(hull_verts.len(), 8);
    assert!(!planes.is_empty());
    for (i, plane) in planes.iter().enumerate() {
        let mag = (plane[0] * plane[0] + plane[1] * plane[1] + plane[2] * plane[2]).sqrt();
        assert!((mag - 1.0).abs() < 1e-4, "plane {i} not unit: {mag}");
        // offset is the Python convention (-d_scipy): the origin lies strictly
        // behind every outward plane, every input vertex on or behind it.
        assert!(-plane[3] < -0.5, "cube center must be inside plane {i}");
        for v in &verts {
            let lhs = plane[0] * v[0] + plane[1] * v[1] + plane[2] * v[2];
            assert!(lhs - plane[3] <= 1e-3, "vertex {v:?} in front of plane {i}");
        }
    }
    let coplanar = vec![
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [1.0, 1.0, 0.0],
    ];
    assert!(convex_hull_simple(&coplanar).is_err());
}

#[test]
fn fo76_flat_convex_compound_child_keeps_instance_transform() {
    use havok_native::collision::extract_preview_meshes_from_blob;
    // CharGen_Vlt76_Door01 body 2 (Panel01): hknpCompoundShape whose flat-convex
    // compressed-mesh child (local z -0.419..0.420) is instanced at z +0.4198.
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/fo76_chargen_vlt76_door01_physics.bin");
    let blob = std::fs::read(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    let meshes = extract_preview_meshes_from_blob(&blob, 1.0, Some(2)).expect("preview");
    let compressed = meshes
        .iter()
        .find(|mesh| mesh.shape_type == "compressed_mesh")
        .expect("compressed mesh child");
    let (min_z, max_z) = compressed
        .vertices
        .iter()
        .fold((f32::MAX, f32::MIN), |(lo, hi), v| (lo.min(v[2]), hi.max(v[2])));
    assert!((min_z - 0.0008).abs() < 0.002, "min z {min_z}");
    assert!((max_z - 0.8398).abs() < 0.002, "max z {max_z}");
}
