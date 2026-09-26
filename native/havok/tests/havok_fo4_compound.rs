// Integration tests for `build_fo4_compound_collision` and compressed-mesh materials.

use havok_native::collision::compressed_mesh::{
    BuildOptions, MaterialEntry, build_compressed_mesh_collision,
};
use havok_native::collision::{CompoundChild, CompoundChildKind, build_fo4_compound_collision};
use havok_native::hkx::model::{HkxFile, HkxMember};
use havok_native::hkx::types::HkxValue;

const PF_MAGIC: &[u8; 8] = b"\x57\xE0\xE0\x57\x10\xC0\xC0\x10";

fn tetrahedron(ox: f32) -> Vec<[f32; 3]> {
    vec![
        [ox, 0.0, 0.0],
        [1.0 + ox, 0.0, 0.0],
        [0.5 + ox, 1.0, 0.0],
        [0.5 + ox, 0.5, 1.0],
    ]
}

fn cube_verts() -> Vec<[f32; 3]> {
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

fn polytope_child(vertices: Vec<[f32; 3]>) -> CompoundChild {
    CompoundChild {
        transform: CompoundChild::identity_transform(),
        kind: CompoundChildKind::Polytope { vertices },
    }
}

fn mesh_child() -> CompoundChild {
    CompoundChild {
        transform: CompoundChild::identity_transform(),
        kind: CompoundChildKind::CompressedMesh {
            vertices: cube_verts(),
            triangles: cube_tris(),
        },
    }
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

fn section_start(blob: &[u8], index: usize) -> usize {
    let off = 0x40 + index * 0x40 + 0x14;
    u32::from_le_bytes(blob[off..off + 4].try_into().unwrap()) as usize
}

fn member<'a>(members: &'a [HkxMember], name: &str) -> &'a HkxValue {
    &members
        .iter()
        .find(|m| m.name == name)
        .unwrap_or_else(|| panic!("missing member {name}"))
        .value
}

fn bitfield_words(shape_members: &[HkxMember], name: &str, expected_num_bits: i32) -> Vec<u32> {
    let storage = member(
        member(shape_members, name).as_object_members().unwrap(),
        "storage",
    )
    .as_object_members()
    .unwrap();
    let num_bits = match member(storage, "numBits") {
        HkxValue::I32(v) => *v,
        HkxValue::U32(v) => *v as i32,
        other => panic!("{name}.numBits: {}", other.variant_name()),
    };
    assert_eq!(num_bits, expected_num_bits, "{name}.numBits");
    let HkxValue::Array(words) = member(storage, "words") else {
        panic!("{name}.storage.words must be an array");
    };
    assert_eq!(words.len(), (expected_num_bits as usize).div_ceil(32));
    words
        .iter()
        .map(|value| match value {
            HkxValue::U32(v) => *v,
            HkxValue::I32(v) => *v as u32,
            other => panic!("{name} word: {}", other.variant_name()),
        })
        .collect()
}

#[test]
fn compound_packfiles_use_vanilla_layout() {
    let cases = [
        (
            "2 polytopes",
            vec![
                polytope_child(tetrahedron(0.0)),
                polytope_child(tetrahedron(2.0)),
            ],
        ),
        ("1 compressed mesh", vec![mesh_child()]),
    ];
    for (label, children) in cases {
        let blob = build_fo4_compound_collision(&children, &default_opts())
            .unwrap_or_else(|e| panic!("{label}: {e}"));
        assert_eq!(&blob[..8], PF_MAGIC, "{label}");
        assert!(blob.len() >= 1024, "{label}: {} bytes", blob.len());
        let (classnames, types, data) = (
            section_start(&blob, 0),
            section_start(&blob, 1),
            section_start(&blob, 2),
        );
        assert_eq!(classnames, 0x100, "{label}: __classnames__ start");
        assert!(
            classnames <= types && types <= data && data < blob.len(),
            "{label}"
        );
    }

    // A compressed-mesh child must carry real quadIsFlat / triangleIsInterior
    // backing arrays, like a standalone compressed mesh.
    let blob = build_fo4_compound_collision(&[mesh_child()], &default_opts()).unwrap();
    let file = HkxFile::read(&blob).expect("compound packfile parses");
    let shape = file
        .objects()
        .iter()
        .find(|obj| obj.class_name == "hknpCompressedMeshShape")
        .expect("hknpCompressedMeshShape missing");
    let triangle_bits = cube_tris().len() as i32;
    let quad_words = bitfield_words(&shape.members, "quadIsFlat", (triangle_bits + 1) / 2);
    let interior_words = bitfield_words(&shape.members, "triangleIsInterior", triangle_bits);
    assert!(
        quad_words.iter().any(|w| *w != 0),
        "quadIsFlat marks flat quads"
    );
    assert!(
        interior_words.iter().all(|w| *w == 0),
        "surface triangles are not interior"
    );

    let user_data = 0xDEAD_BEEF_0000_0000u64;
    let blob = build_fo4_compound_collision(
        &[polytope_child(tetrahedron(0.0))],
        &BuildOptions {
            user_data: Some(user_data),
            ..default_opts()
        },
    )
    .unwrap();
    assert!(blob.windows(8).any(|w| w == user_data.to_le_bytes()));
}

#[test]
fn compound_rejects_invalid_children() {
    let coplanar = vec![
        [0.0f32, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [1.0, 1.0, 0.0],
    ];
    let cases: [(&str, Vec<CompoundChild>, &[&str]); 4] = [
        ("empty", vec![], &["at least one"]),
        (
            "mixed kinds",
            vec![polytope_child(tetrahedron(0.0)), mesh_child()],
            &[""],
        ),
        (
            "no vertices",
            vec![polytope_child(vec![])],
            &["EmptySubShape", "no vertices"],
        ),
        ("coplanar", vec![polytope_child(coplanar)], &["coplanar"]),
    ];
    for (label, children, messages) in cases {
        let err = build_fo4_compound_collision(&children, &default_opts())
            .expect_err(label)
            .to_string();
        assert!(messages.iter().any(|m| err.contains(m)), "{label}: {err}");
    }
}

#[test]
fn compressed_mesh_materials_reach_blob_and_summary() {
    let two_materials = BuildOptions {
        materials: vec![
            MaterialEntry {
                filter_info: 0xAABBCCDD,
                material_crc: 0x11223344,
            },
            MaterialEntry {
                filter_info: 0x12345678,
                material_crc: 0x87654321,
            },
        ],
        ..BuildOptions::default()
    };
    let blob = build_compressed_mesh_collision(&cube_verts(), &cube_tris(), two_materials)
        .expect("build CM with 2 materials");
    for needle in [0xAABBCCDDu32, 0x12345678, 0x11223344, 0x87654321] {
        assert!(
            blob.windows(4).any(|w| w == needle.to_le_bytes()),
            "{needle:#x} missing from blob"
        );
    }

    let weapon_pistol = 4_146_539_321u32;
    let blob = build_compressed_mesh_collision(
        &cube_verts(),
        &cube_tris(),
        BuildOptions {
            user_data: Some(u64::from(weapon_pistol)),
            materials: vec![MaterialEntry {
                filter_info: 1,
                material_crc: weapon_pistol,
            }],
            layer: 1,
            ..BuildOptions::default()
        },
    )
    .expect("build CM");
    let summary: serde_json::Value =
        serde_json::from_str(&havok_native::api::havok_collision_summary(&blob).expect("summary"))
            .expect("summary JSON");
    let body = &summary["bodies"][0];
    let pistol = Some(u64::from(weapon_pistol));
    assert_eq!(body["shape_user_data"].as_u64(), pistol);
    assert_eq!(body["material_crc"].as_u64(), pistol);
    assert_eq!(body["bs_materials"][0]["filter_info"].as_u64(), Some(1));
    assert_eq!(body["bs_materials"][0]["material_crc"].as_u64(), pistol);
}
