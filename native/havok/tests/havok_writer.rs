use std::path::PathBuf;

use havok_native::api;
use havok_native::error::HavokError;
use havok_native::hkx::descriptors::DescriptorRegistry;
use havok_native::hkx::model::HkxObject;
use havok_native::hkx::types::HkxValue;
use havok_native::hkx::{HkxFile, HkxMember, read_packfile, write_hkx};

fn repo_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

/// Make a minimal HkxFile where object[1] has a Pointer(Some(0)) pointing at object[0].
fn make_hkx_with_pointer() -> HkxFile {
    let objects = vec![
        HkxObject {
            name: Some("#0000".to_string()),
            offset: 0,
            signature: 0,
            class_name: "hkRootLevelContainer".to_string(),
            members: vec![HkxMember {
                name: "namedVariants".to_string(),
                value: HkxValue::Array(Vec::new()),
            }],
        },
        HkxObject {
            name: Some("#0001".to_string()),
            offset: 0,
            signature: 0,
            class_name: "hkMemoryResourceContainer".to_string(),
            members: vec![
                HkxMember {
                    name: "name".to_string(),
                    value: HkxValue::String {
                        value: "container".to_string(),
                        is_null: false,
                    },
                },
                HkxMember {
                    name: "resourceHandles".to_string(),
                    // An array of pointers; element 0 points to object[0].
                    value: HkxValue::Array(vec![HkxValue::Pointer(Some(0))]),
                },
                HkxMember {
                    name: "children".to_string(),
                    value: HkxValue::Array(Vec::new()),
                },
            ],
        },
    ];
    HkxFile::from_tagxml(11, "hk_2014.1.0-r1", objects)
}

#[test]
fn write_hkx_round_trips_header_classes_and_pointer_indices() {
    let hkx_file = make_hkx_with_pointer();
    let mut registry = DescriptorRegistry::new();

    let bytes = write_hkx(&hkx_file, &mut registry);
    let parsed = read_packfile(&bytes).expect("writer output should be parseable");
    assert_eq!(parsed.class_version(), 11);
    assert_eq!(parsed.contents_version(), "hk_2014.1.0-r1");
    let classes: Vec<_> = parsed.objects().iter().map(|o| &o.class_name).collect();
    let expected: Vec<_> = hkx_file.objects().iter().map(|o| &o.class_name).collect();
    assert_eq!(classes, expected);

    let handles = parsed.objects()[1]
        .members
        .iter()
        .find(|m| m.name == "resourceHandles")
        .expect("resourceHandles member should survive round-trip");
    assert_eq!(
        handles.value,
        HkxValue::Array(vec![HkxValue::Pointer(Some(0))])
    );
}

#[test]
fn write_hkx_round_trip_preserves_fixed_pointer_arrays() {
    let constraint = HkxObject {
        name: Some("#0000".to_string()),
        offset: 0,
        signature: 0,
        class_name: "hkpRagdollConstraintData".to_string(),
        members: vec![
            HkxMember {
                name: "userData".to_string(),
                value: HkxValue::U64(0),
            },
            HkxMember {
                name: "atoms".to_string(),
                value: HkxValue::Object(vec![HkxMember {
                    name: "ragdollMotors".to_string(),
                    value: HkxValue::Object(vec![
                        HkxMember {
                            name: "type".to_string(),
                            value: HkxValue::I32(19),
                        },
                        HkxMember {
                            name: "isEnabled".to_string(),
                            value: HkxValue::Bool(false),
                        },
                        HkxMember {
                            name: "initializedOffset".to_string(),
                            value: HkxValue::I16(0),
                        },
                        HkxMember {
                            name: "previousTargetAnglesOffset".to_string(),
                            value: HkxValue::I16(0),
                        },
                        HkxMember {
                            name: "target_bRca".to_string(),
                            value: HkxValue::F32List(vec![
                                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0,
                            ]),
                        },
                        HkxMember {
                            name: "motors".to_string(),
                            value: HkxValue::Array(vec![
                                HkxValue::Pointer(Some(1)),
                                HkxValue::Pointer(Some(1)),
                                HkxValue::Pointer(Some(1)),
                            ]),
                        },
                    ]),
                }]),
            },
        ],
    };
    let motor = HkxObject {
        name: Some("#0001".to_string()),
        offset: 0,
        signature: 0,
        class_name: "hkpPositionConstraintMotor".to_string(),
        members: vec![
            HkxMember {
                name: "type".to_string(),
                value: HkxValue::I32(1),
            },
            HkxMember {
                name: "minForce".to_string(),
                value: HkxValue::F32(-1_000_000.0),
            },
            HkxMember {
                name: "maxForce".to_string(),
                value: HkxValue::F32(100.0),
            },
            HkxMember {
                name: "tau".to_string(),
                value: HkxValue::F32(0.8),
            },
            HkxMember {
                name: "damping".to_string(),
                value: HkxValue::F32(1.0),
            },
            HkxMember {
                name: "proportionalRecoveryVelocity".to_string(),
                value: HkxValue::F32(5.0),
            },
            HkxMember {
                name: "constantRecoveryVelocity".to_string(),
                value: HkxValue::F32(0.2),
            },
        ],
    };
    let hkx_file = HkxFile::from_tagxml(11, "hk_2014.1.0-r1", vec![constraint, motor]);
    let mut registry = DescriptorRegistry::new();

    let bytes = write_hkx(&hkx_file, &mut registry);
    let parsed = read_packfile(&bytes).expect("writer output should be parseable");

    let constraint = &parsed.objects()[0];
    let atoms = constraint
        .members
        .iter()
        .find(|m| m.name == "atoms")
        .and_then(|m| m.value.as_object_members())
        .expect("atoms should parse as an inline struct");
    let motors = atoms
        .iter()
        .find(|m| m.name == "ragdollMotors")
        .and_then(|m| m.value.as_object_members())
        .and_then(|members| members.iter().find(|m| m.name == "motors"))
        .expect("ragdollMotors.motors should be present");

    assert_eq!(
        motors.value,
        HkxValue::Array(vec![
            HkxValue::Pointer(Some(1)),
            HkxValue::Pointer(Some(1)),
            HkxValue::Pointer(Some(1)),
        ])
    );
}

// ─── Test 4: havok_convert_bytes FO76→FO4 returns real bytes ─────────────

#[test]
fn havok_convert_bytes_fo76_to_fo4_returns_bytes_for_fixture() {
    let fixture_path =
        repo_path("python/creation_lib/hkxpack/tests/fixtures/fo76_snallygastercharacter.hkx");
    let data = std::fs::read(&fixture_path).unwrap_or_else(|e| {
        panic!(
            "failed to read fo76 fixture {}: {}",
            fixture_path.display(),
            e
        )
    });

    let result = api::havok_convert_bytes(&data, "fo4");

    match result {
        Ok(bytes) => {
            assert!(!bytes.is_empty(), "converted bytes should be non-empty");
            // Verify the output is a valid packfile.
            read_packfile(&bytes)
                .expect("FO76→FO4 converted output should be a valid HKX packfile");
        }
        Err(HavokError::UnportedEdgeCase { edge_case, .. }) => {
            panic!(
                "FO76→FO4 conversion still returns UnportedEdgeCase({edge_case}); writer may not be wired up"
            );
        }
        Err(other) => {
            panic!("FO76→FO4 conversion returned unexpected error: {other}");
        }
    }
}

// ─── Test 6: hkRelArray contents survive round-trip ──────

/// Recursively pull every (member name, element count) for an HkxValue::Array
/// member sitting under any object so we can compare element counts before
/// and after the writer round-trips a file containing hkRelArray data
/// (e.g. hknpCapsuleShape inheriting from hknpConvexPolytopeShape, whose
/// `planes`/`faces`/`indices` all live in hkRelArrays).
fn collect_relarray_member_lengths(file: &HkxFile) -> Vec<(String, String, usize)> {
    let relarray_member_names = [
        ("hknpConvexPolytopeShape", "planes"),
        ("hknpConvexPolytopeShape", "faces"),
        ("hknpConvexPolytopeShape", "indices"),
    ];
    let mut found = Vec::new();
    for obj in file.objects() {
        for (klass, member_name) in &relarray_member_names {
            // Either the object is exactly the class, or it's a subclass that
            // inherits the relarray members (hknpCapsuleShape inherits from
            // hknpConvexPolytopeShape, so reader resolves all parent members).
            let _ = klass;
            for m in &obj.members {
                if m.name == *member_name {
                    if let HkxValue::Array(values) = &m.value {
                        found.push((obj.class_name.clone(), m.name.clone(), values.len()));
                    }
                }
            }
        }
    }
    found
}

#[test]
fn skeleton_fixture_rewrite_preserves_rel_arrays_and_save_tracks_dirty_state() {
    // skeleton.hkx holds hknpCapsuleShape objects whose parent
    // hknpConvexPolytopeShape declares planes/faces/indices as hkRelArrays; a
    // skipped relarray block would silently re-read as empty.
    let src_bytes =
        std::fs::read(repo_path("native/havok/tests/fixtures/skeleton.hkx")).expect("read fixture");
    let hkx_file = read_packfile(&src_bytes).expect("parse skeleton.hkx");
    let before = collect_relarray_member_lengths(&hkx_file);
    assert!(!before.is_empty());

    let mut registry = DescriptorRegistry::new();
    let reread = read_packfile(&write_hkx(&hkx_file, &mut registry)).expect("re-read written");
    assert_eq!(before, collect_relarray_member_lengths(&reread));

    assert!(!hkx_file.is_dirty());
    assert_eq!(hkx_file.save(), src_bytes);
    let mut dirty = read_packfile(&src_bytes).expect("parse skeleton.hkx");
    dirty.objects_mut()[0].name = Some("#FFFF".to_string());
    assert!(dirty.is_dirty());
    read_packfile(&dirty.save()).expect("save() output must be a valid packfile");
}
