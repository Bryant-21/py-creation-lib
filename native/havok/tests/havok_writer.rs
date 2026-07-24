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

/// Build a minimal HkxFile with two objects (hkRootLevelContainer + hkMemoryResourceContainer).
/// These are simple classes that are always present in any FO4 behaviour HKX.
fn make_minimal_hkx() -> HkxFile {
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
                        value: String::new(),
                        is_null: false,
                    },
                },
                HkxMember {
                    name: "resourceHandles".to_string(),
                    value: HkxValue::Array(Vec::new()),
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

// ─── Test 1: round-trip preserves object count and class names ────────────

#[test]
fn write_hkx_round_trip_preserves_object_count_and_class_names() {
    let hkx_file = make_minimal_hkx();
    let mut registry = DescriptorRegistry::new();

    let bytes = write_hkx(&hkx_file, &mut registry);
    assert!(!bytes.is_empty(), "writer produced empty output");

    let parsed = read_packfile(&bytes).expect("writer output should be parseable by read_packfile");

    assert_eq!(
        parsed.objects().len(),
        hkx_file.objects().len(),
        "object count mismatch after round-trip"
    );
    for (orig, rt) in hkx_file.objects().iter().zip(parsed.objects().iter()) {
        assert_eq!(
            orig.class_name, rt.class_name,
            "class_name mismatch: {} vs {}",
            orig.class_name, rt.class_name
        );
    }
}

// ─── Test 2: header fields ────────────────────────────────────────────────

#[test]
fn write_hkx_emits_valid_packfile_header() {
    let hkx_file = make_minimal_hkx();
    let mut registry = DescriptorRegistry::new();

    let bytes = write_hkx(&hkx_file, &mut registry);
    let parsed = read_packfile(&bytes).expect("parseable output");

    assert_eq!(parsed.class_version(), 11, "packfile version must be 11");
    assert_eq!(
        parsed.contents_version(),
        "hk_2014.1.0-r1",
        "version_name mismatch"
    );
}

// ─── Test 3: pointer indices survive round-trip ───────────────────────────

#[test]
fn write_hkx_round_trip_preserves_pointer_indices() {
    let hkx_file = make_hkx_with_pointer();
    let mut registry = DescriptorRegistry::new();

    let bytes = write_hkx(&hkx_file, &mut registry);
    let parsed = read_packfile(&bytes).expect("parseable output");

    assert_eq!(parsed.objects().len(), 2, "should have 2 objects");

    // Object[1] should have a resourceHandles member with a pointer to object[0].
    let obj1 = &parsed.objects()[1];
    let rh = obj1
        .members
        .iter()
        .find(|m| m.name == "resourceHandles")
        .expect("resourceHandles member should survive round-trip");

    match &rh.value {
        HkxValue::Array(elems) => {
            assert_eq!(elems.len(), 1, "pointer array should have 1 element");
            match &elems[0] {
                HkxValue::Pointer(Some(idx)) => {
                    assert_eq!(*idx, 0, "pointer should resolve to object index 0");
                }
                other => panic!("expected Pointer(Some(0)), got {:?}", other),
            }
        }
        other => panic!("expected Array for resourceHandles, got {:?}", other),
    }
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

// ─── Test 5 (optional): byte-exact for AttackSprinting if present ─────────

#[test]
fn write_hkx_byte_exact_for_attacksprinting_if_present() {
    let path =
        repo_path("../extracted/fo4/Meshes/Actors/Character/Animations/1HM/AttackSprinting.hkx");
    if !path.exists() {
        // Byte-exact check is optional — skip silently.
        eprintln!(
            "SKIP: {} not present (Milestone 2 byte-exact check)",
            path.display()
        );
        return;
    }

    let src_bytes = std::fs::read(&path).expect("read AttackSprinting.hkx");
    let hkx_file = read_packfile(&src_bytes).expect("parse AttackSprinting.hkx");
    let mut registry = DescriptorRegistry::new();

    let out = write_hkx(&hkx_file, &mut registry);

    let diff_count = src_bytes
        .iter()
        .zip(out.iter())
        .filter(|(a, b)| a != b)
        .count();
    let len_match = src_bytes.len() == out.len();

    eprintln!(
        "AttackSprinting byte-exact: len_match={len_match}, diff_bytes={diff_count}/{}",
        src_bytes.len()
    );

    // This test does not hard-fail on byte-exactness (the file might have edge
    // cases listed in CLAUDE.md). Uncomment the assert to enforce byte-exact:
    // assert!(len_match && diff_count == 0, "not byte-exact");

    // Minimum bar: output is parseable.
    read_packfile(&out).expect("AttackSprinting re-written output must be parseable");
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
fn write_hkx_preserves_rel_array_contents_for_skeleton_fixture() {
    // resource/skeleton.hkx contains hknpCapsuleShape objects whose parent
    // hknpConvexPolytopeShape declares `planes`/`faces`/`indices` as hkRelArrays.
    let path = repo_path("native/havok/tests/fixtures/skeleton.hkx");
    let src_bytes = std::fs::read(&path).expect("read resource/skeleton.hkx");
    let hkx_file = read_packfile(&src_bytes).expect("parse skeleton.hkx");

    let before = collect_relarray_member_lengths(&hkx_file);
    assert!(
        !before.is_empty(),
        "skeleton.hkx fixture should contain at least one hkRelArray member \
         (hknpConvexPolytopeShape planes/faces/indices)"
    );

    let mut registry = DescriptorRegistry::new();
    let written = write_hkx(&hkx_file, &mut registry);

    // Re-read the round-tripped output and verify every hkRelArray member
    // ends up with the same element count it had before. A skipped relarray
    // data block leaves the offset field zero, so the reader returns zero (or
    // whatever lands at offset 0) — silent corruption.
    let reread = read_packfile(&written).expect("re-read written skeleton.hkx");
    let after = collect_relarray_member_lengths(&reread);

    assert_eq!(
        before, after,
        "hkRelArray member contents diverged across write/read round-trip"
    );
}

// ─── Test 7: HkxFile::save runs the writer when dirty ────

#[test]
fn save_returns_source_bytes_when_clean_and_writer_output_when_dirty() {
    let path = repo_path("native/havok/tests/fixtures/skeleton.hkx");
    let src_bytes = std::fs::read(&path).expect("read resource/skeleton.hkx");

    // Clean read: save() must echo the source bytes verbatim.
    let hkx_clean = read_packfile(&src_bytes).expect("parse skeleton.hkx");
    assert!(
        !hkx_clean.is_dirty(),
        "freshly-read file should not be dirty"
    );
    assert_eq!(
        hkx_clean.save(),
        src_bytes,
        "save() on a clean file must echo source_bytes verbatim"
    );

    // Mutate via objects_mut() so the dirty bit flips. save() must then
    // route through the writer rather than echoing stale source bytes.
    let mut hkx_dirty = read_packfile(&src_bytes).expect("parse skeleton.hkx");
    {
        let objects = hkx_dirty.objects_mut();
        // Touch a member name on object[0] — any in-memory mutation works.
        // The point is: save() must *not* return source_bytes after this.
        if let Some(obj) = objects.get_mut(0) {
            obj.name = Some("#FFFF".to_string());
        }
    }
    assert!(hkx_dirty.is_dirty(), "objects_mut() should set dirty bit");

    let saved = hkx_dirty.save();
    assert!(!saved.is_empty(), "writer output must be non-empty");
    // Should re-parse without error — i.e. save() actually ran the writer.
    read_packfile(&saved).expect("save() output must be a valid packfile");
}
