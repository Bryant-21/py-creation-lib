use std::path::PathBuf;

use havok_native::api;
use havok_native::convert::{
    ClassVersion, ConversionContext, CustomHookRegistry, HavokVersion, Patch, PatchDirection,
    PatchManager, PatchOperation, PatchValue, detect_version_id, get_version_chain,
    parse_target_version,
};
use havok_native::error::HavokError;
use havok_native::hkx::descriptors::DescriptorRegistry;
use havok_native::hkx::types::HkxValue;
use havok_native::hkx::{HkxFile, HkxObject, write_hkx};

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

fn synthetic_object(name: &str, class_name: &str) -> HkxObject {
    HkxObject {
        name: Some(name.to_string()),
        offset: 0,
        signature: 0,
        class_name: class_name.to_string(),
        members: Vec::new(),
    }
}

fn synthetic_packfile(objects: Vec<HkxObject>) -> Vec<u8> {
    let hkx = HkxFile::from_tagxml(11, "hk_2014.1.0-r1", objects);
    let mut registry = DescriptorRegistry::for_contents_version("hk_2014.1.0-r1");
    write_hkx(&hkx, &mut registry)
}

#[test]
fn hkx_class_summary_flags_setup_only_cloth_packfile() {
    let bytes = synthetic_packfile(vec![synthetic_object("#0001", "hclClothSetupContainer")]);

    let summary = api::hkx_class_summary(&bytes).expect("class summary");

    assert_eq!(summary.contents_version, "hk_2014.1.0-r1");
    assert_eq!(summary.class_counts.get("hclClothSetupContainer"), Some(&1));
    assert!(summary.has_cloth_setup_data);
    assert!(!summary.has_cloth_data);
    assert!(summary.is_setup_only_cloth);
}

#[test]
fn hkx_class_summary_routes_tag0_fixture() {
    let bytes =
        fixture_bytes("python/creation_lib/hkxpack/tests/fixtures/fo76_snallygastercharacter.hkx");

    let summary = api::hkx_class_summary(&bytes).expect("class summary");

    assert_eq!(summary.contents_version, "hk_2015.1.0-r1");
    assert!(summary.class_counts.contains_key("hkRootLevelContainer"));
    assert!(summary.class_counts.contains_key("hkbCharacterData"));
}

#[test]
fn target_version_parser_accepts_ids_names_and_game_aliases() {
    assert_eq!(parse_target_version("53").unwrap().id, 53);
    assert_eq!(parse_target_version("hk_2014.1.0-r1").unwrap().id, 53);
    assert_eq!(parse_target_version("fo4").unwrap().id, 53);
    assert_eq!(parse_target_version("FO76").unwrap().id, 56);
    assert_eq!(parse_target_version("skyrim_se").unwrap().id, 40);
    assert_eq!(
        detect_version_id("hk_2010.2.0-r1").unwrap(),
        parse_target_version("skyrimse").unwrap().id
    );
}

#[test]
fn version_chain_traversal_matches_existing_python_directionality() {
    let upgrade: Vec<u8> = get_version_chain(46, 56)
        .unwrap()
        .into_iter()
        .map(|version| version.id)
        .collect();
    assert_eq!(upgrade.first(), Some(&46));
    assert_eq!(upgrade.last(), Some(&56));
    assert!(upgrade.windows(2).all(|pair| pair[1] > pair[0]));

    let downgrade: Vec<u8> = get_version_chain(56, 46)
        .unwrap()
        .into_iter()
        .map(|version| version.id)
        .collect();
    assert_eq!(downgrade.first(), Some(&56));
    assert_eq!(downgrade.last(), Some(&46));
    assert!(downgrade.windows(2).all(|pair| pair[1] < pair[0]));

    assert_eq!(
        get_version_chain(53, 53).unwrap(),
        vec![HavokVersion::new(53, "hk_2014.1.0-r1")]
    );
}

#[test]
fn patch_manager_refuses_routes_crossing_unimplemented_version_packages() {
    let manager = PatchManager::new();

    for (source, target) in [(56_u8, 57_u8), (56, 61), (61, 56), (57, 58)] {
        let error = manager.route(source, target).unwrap_err();
        match error {
            HavokError::ConversionNotImplemented {
                source_version,
                target_version,
                route,
                reason,
            } => {
                assert_eq!(source_version, source);
                assert_eq!(target_version, target);
                assert!(route.contains("hk_"), "unexpected route: {route}");
                assert!(
                    reason.contains("unimplemented patch package"),
                    "unexpected reason: {reason}"
                );
            }
            other => panic!("unexpected error: {other}"),
        }
    }
}

#[test]
fn patch_manager_records_declarative_ops_and_custom_hooks_for_route_steps() {
    let mut manager = PatchManager::new();
    manager.register(
        47,
        Patch::new(
            ClassVersion::new("hkbFoo", 2),
            ClassVersion::new("hkbFoo", 3),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "weight".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: Some(PatchValue::Real(1.0)),
        })
        .with_custom_hook("fix_property_sheets"),
    );

    let route = manager.route(46, 48).unwrap();

    assert_eq!(route.steps.len(), 2);
    assert_eq!(route.steps[0].version.id, 47);
    assert_eq!(route.steps[0].direction, PatchDirection::Upgrade);
    assert_eq!(route.steps[0].patches.len(), 1);
    assert_eq!(
        route.steps[0].patches[0].custom_hooks,
        ["fix_property_sheets"]
    );
    assert!(matches!(
        route.steps[0].patches[0].operations[0],
        PatchOperation::MemberAdd { .. }
    ));
}

#[test]
fn patch_manager_downgrade_route_uses_inverse_patches_in_reverse_order() {
    let mut manager = PatchManager::new();
    manager.register(
        47,
        Patch::new(
            ClassVersion::new("hkbFoo", 2),
            ClassVersion::new("hkbFoo", 3),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "weight".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: Some(PatchValue::Real(1.0)),
        })
        .with_operation(PatchOperation::MemberRename {
            old_name: "old".to_string(),
            new_name: "new".to_string(),
        }),
    );

    let route = manager.route(48, 46).unwrap();

    assert!(!route.upgrading);
    assert_eq!(route.steps.len(), 2);
    assert_eq!(route.steps[1].version.id, 47);
    assert_eq!(route.steps[1].direction, PatchDirection::Downgrade);
    let patch = &route.steps[1].patches[0];
    assert_eq!(patch.old, ClassVersion::new("hkbFoo", 3));
    assert_eq!(patch.new, ClassVersion::new("hkbFoo", 2));
    assert!(matches!(
        patch.operations[0],
        PatchOperation::MemberRename { ref old_name, ref new_name }
            if old_name == "new" && new_name == "old"
    ));
    assert!(matches!(
        patch.operations[1],
        PatchOperation::MemberRemove { ref name, ref type_name }
            if name == "weight" && type_name == "real"
    ));
}

#[test]
fn patch_manager_convert_updates_contents_version_after_route() {
    let mut manager = PatchManager::new();
    manager.register(
        54,
        Patch::new(
            ClassVersion::new("TestObj", 0),
            ClassVersion::new("TestObj", 1),
        ),
    );
    let mut hkx = havok_native::hkx::HkxFile::from_tagxml(
        11,
        "hk_2014.1.0-r1",
        vec![havok_native::hkx::HkxObject {
            name: Some("#0001".into()),
            offset: 0,
            signature: 0,
            class_name: "TestObj".into(),
            members: vec![],
        }],
    );

    manager.convert_hkx(&mut hkx, 53, 54).unwrap();

    assert_eq!(hkx.contents_version(), "hk_2014.1.0-r2");
}

#[test]
fn declarative_patch_ops_mutate_rust_hkx_objects() {
    use havok_native::hkx::HkxMember;
    use havok_native::hkx::types::HkxValue;

    let mut obj = havok_native::hkx::HkxObject {
        name: Some("#0001".into()),
        offset: 0,
        signature: 0,
        class_name: "TestObj".into(),
        members: vec![HkxMember {
            name: "oldName".into(),
            value: HkxValue::I32(7),
        }],
    };
    let patch = Patch::new(
        ClassVersion::new("TestObj", 0),
        ClassVersion::new("RenamedObj", 1),
    )
    .with_operation(PatchOperation::MemberRename {
        old_name: "oldName".into(),
        new_name: "newName".into(),
    })
    .with_operation(PatchOperation::MemberAdd {
        name: "weight".into(),
        type_name: "real".into(),
        ctype: None,
        default: Some(PatchValue::Real(1.0)),
    });

    assert!(patch.matches_object(&obj));
    patch.apply_to_object(&mut obj).unwrap();

    assert_eq!(obj.class_name, "RenamedObj");
    assert_eq!(obj.signature, 1);
    assert!(obj.members.iter().any(|m| m.name == "newName"));
    assert!(
        obj.members
            .iter()
            .any(|m| m.name == "weight" && m.value == HkxValue::F32(1.0))
    );
}

#[test]
fn native_patch_corpus_refuses_changed_conversion_until_complete() {
    let manager = PatchManager::with_native_corpus();
    let mut hkx = havok_native::hkx::HkxFile::from_tagxml(
        11,
        "hk_2014.1.0-r1",
        vec![havok_native::hkx::HkxObject {
            name: Some("#0001".into()),
            offset: 0,
            signature: 0,
            class_name: "hkbBehaviorReferenceGenerator".into(),
            members: vec![],
        }],
    );

    let error = manager.convert_hkx(&mut hkx, 53, 56).unwrap_err();

    match error {
        HavokError::UnportedEdgeCase {
            route,
            edge_case,
            detail,
        } => {
            assert_eq!(route, "packfile-version-patch-chain");
            assert_eq!(edge_case, "native patch corpus parity");
            assert!(detail.contains("incomplete"), "unexpected detail: {detail}");
        }
        other => panic!("unexpected error: {other}"),
    }
}

fn hkb_character_data_10_to_11_patch() -> Patch {
    Patch::new(
        ClassVersion::new("hkbCharacterData", 10),
        ClassVersion::new("hkbCharacterData", 11),
    )
    .with_operation(PatchOperation::MemberAdd {
        name: "propertySheets".into(),
        type_name: "array".into(),
        ctype: Some("hkbCustomPropertySheet".into()),
        default: None,
    })
    .with_reversible_custom_hook("_hkbCharacterData_10_to_11", "_hkbCharacterData_11_to_10")
    .with_operation(PatchOperation::MemberRemove {
        name: "mirroredSkeletonInfo".into(),
        type_name: "struct".into(),
    })
    .with_operation(PatchOperation::MemberRemove {
        name: "footIkDriverInfo".into(),
        type_name: "struct".into(),
    })
    .with_operation(PatchOperation::MemberRemove {
        name: "handIkDriverInfo".into(),
        type_name: "struct".into(),
    })
    .with_operation(PatchOperation::MemberRemove {
        name: "aiControlDriverInfo".into(),
        type_name: "struct".into(),
    })
    .with_operation(PatchOperation::Depends {
        class_name: "hkbFootIkDriverInfo".into(),
        version: 1,
    })
    .with_operation(PatchOperation::Depends {
        class_name: "hkbHandIkDriverInfo".into(),
        version: 0,
    })
    .with_operation(PatchOperation::Depends {
        class_name: "hkbMirroredSkeletonInfo".into(),
        version: 1,
    })
    .with_operation(PatchOperation::Depends {
        class_name: "hkbCustomPropertySheet".into(),
        version: 0,
    })
    .with_operation(PatchOperation::Depends {
        class_name: "hkReferencedObject".into(),
        version: 0,
    })
}

#[test]
fn native_hkb_character_data_upgrade_hook_populates_property_sheets_in_python_order() {
    use havok_native::hkx::HkxMember;
    use havok_native::hkx::types::HkxValue;

    let mut manager = PatchManager::new();
    manager.register_native_hooks();
    manager.register(55, hkb_character_data_10_to_11_patch());
    let mut hkx = havok_native::hkx::HkxFile::from_tagxml(
        11,
        "hk_2014.1.0-r1",
        vec![havok_native::hkx::HkxObject {
            name: Some("#0000".into()),
            offset: 0,
            signature: 10,
            class_name: "hkbCharacterData".into(),
            members: vec![
                HkxMember {
                    name: "mirroredSkeletonInfo".into(),
                    value: HkxValue::Pointer(Some(1)),
                },
                HkxMember {
                    name: "footIkDriverInfo".into(),
                    value: HkxValue::Pointer(None),
                },
                HkxMember {
                    name: "handIkDriverInfo".into(),
                    value: HkxValue::Pointer(Some(3)),
                },
                HkxMember {
                    name: "aiControlDriverInfo".into(),
                    value: HkxValue::Pointer(Some(4)),
                },
            ],
        }],
    );

    manager.convert_hkx(&mut hkx, 54, 55).unwrap();

    let object = &hkx.objects()[0];
    assert_eq!(object.signature, 11);
    let property_sheets = object
        .members
        .iter()
        .find(|member| member.name == "propertySheets")
        .expect("propertySheets should be present");
    assert_eq!(
        property_sheets.value,
        HkxValue::Array(vec![
            HkxValue::Pointer(Some(1)),
            HkxValue::Pointer(Some(3)),
            HkxValue::Pointer(Some(4)),
        ])
    );
    for removed in [
        "mirroredSkeletonInfo",
        "footIkDriverInfo",
        "handIkDriverInfo",
        "aiControlDriverInfo",
    ] {
        assert!(
            !object.members.iter().any(|member| member.name == removed),
            "{removed} should be removed"
        );
    }
}

/// A downgrade that crosses a `MemberRemove` op fails: `MemberRemove::inverse` is
/// `Unsupported`, so the correct contract is `Err` rather than silent corruption.
/// This test asserts that contract.
#[test]
fn native_hkb_character_data_downgrade_refuses_with_unsupported_error() {
    use havok_native::hkx::HkxMember;
    use havok_native::hkx::types::HkxValue;

    let mut manager = PatchManager::new();
    manager.register_native_hooks();
    manager.register(55, hkb_character_data_10_to_11_patch());
    let mut hkx = havok_native::hkx::HkxFile::from_tagxml(
        11,
        "hk_2014.2.0-r1",
        vec![havok_native::hkx::HkxObject {
            name: Some("#0000".into()),
            offset: 0,
            signature: 11,
            class_name: "hkbCharacterData".into(),
            members: vec![HkxMember {
                name: "propertySheets".into(),
                value: HkxValue::Array(vec![]),
            }],
        }],
    );
    // The downgrade chain includes a MemberRemove op whose inverse is Unsupported,
    // so conversion returns Err rather than silently corrupting.
    let result = manager.convert_hkx(&mut hkx, 55, 54);
    assert!(
        result.is_err(),
        "expected downgrade to fail with Unsupported error"
    );
}

#[test]
fn native_2014_2_5_clip_generator_patch_merges_bundle_name() {
    use havok_native::hkx::HkxMember;
    use havok_native::hkx::types::HkxValue;

    let mut manager = PatchManager::new();
    manager.register_native_hooks();
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbClipGenerator", 4),
            ClassVersion::new("hkbClipGenerator", 5),
        )
        .with_custom_hook("_hkbClipGenerator_4_to_5")
        .with_operation(PatchOperation::MemberAdd {
            name: "animationInternalId".into(),
            type_name: "int".into(),
            ctype: None,
            default: Some(PatchValue::Int(-1)),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "animationBindingIndex".into(),
            type_name: "int".into(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "animationBundleName".into(),
            type_name: "string".into(),
        }),
    );
    let mut hkx = havok_native::hkx::HkxFile::from_tagxml(
        11,
        "hk_2014.2.0-r1",
        vec![havok_native::hkx::HkxObject {
            name: Some("#0001".into()),
            offset: 0,
            signature: 4,
            class_name: "hkbClipGenerator".into(),
            members: vec![
                HkxMember {
                    name: "animationName".into(),
                    value: HkxValue::String {
                        value: "actors/dog/walk.hkx".into(),
                        is_null: false,
                    },
                },
                HkxMember {
                    name: "animationBundleName".into(),
                    value: HkxValue::String {
                        value: "dogbundle".into(),
                        is_null: false,
                    },
                },
                HkxMember {
                    name: "animationBindingIndex".into(),
                    value: HkxValue::I32(3),
                },
            ],
        }],
    );

    manager.convert_hkx(&mut hkx, 54, 55).unwrap();

    let object = &hkx.objects()[0];
    assert_eq!(object.signature, 5);
    assert!(object.members.iter().any(|member| {
        member.name == "animationName"
            && member.value
                == HkxValue::String {
                    value: "actors/dog/dogbundle:walk.hkx".into(),
                    is_null: false,
                }
    }));
    assert!(object.members.iter().any(|member| {
        member.name == "animationInternalId" && member.value == HkxValue::I32(-1)
    }));
    assert!(
        !object
            .members
            .iter()
            .any(|member| member.name == "animationBindingIndex")
    );
    assert!(
        !object
            .members
            .iter()
            .any(|member| member.name == "animationBundleName")
    );
}

#[test]
fn native_behavior_reference_hook_respects_current_object() {
    use havok_native::hkx::HkxMember;
    use havok_native::hkx::types::HkxValue;

    let mut manager = PatchManager::new();
    manager.register_native_hooks();
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbBehaviorReferenceGenerator", 0),
            ClassVersion::new("hkbBehaviorReferenceGenerator", 1),
        )
        .with_custom_hook("_hkbBehaviorReferenceGenerator_0_to_1"),
    );
    let mut hkx = havok_native::hkx::HkxFile::from_tagxml(
        11,
        "hk_2014.2.0-r1",
        vec![
            havok_native::hkx::HkxObject {
                name: Some("#0001".into()),
                offset: 0,
                signature: 0,
                class_name: "hkbBehaviorReferenceGenerator".into(),
                members: vec![HkxMember {
                    name: "behaviorName".into(),
                    value: HkxValue::String {
                        value: "actors/dog/behavior.test.hkx".into(),
                        is_null: false,
                    },
                }],
            },
            havok_native::hkx::HkxObject {
                name: Some("#0002".into()),
                offset: 0,
                signature: 0,
                class_name: "hkbBehaviorReferenceGenerator".into(),
                members: vec![HkxMember {
                    name: "behaviorName".into(),
                    value: HkxValue::String {
                        value: "actors/cat/behavior.test.hkx".into(),
                        is_null: false,
                    },
                }],
            },
        ],
    );

    manager.convert_hkx(&mut hkx, 54, 55).unwrap();

    assert_eq!(
        hkx.objects()[0].members[0].value,
        HkxValue::String {
            value: "actors/dog/behavior.test".into(),
            is_null: false,
        }
    );
    assert_eq!(
        hkx.objects()[1].members[0].value,
        HkxValue::String {
            value: "actors/cat/behavior.test".into(),
            is_null: false,
        }
    );
}

#[test]
fn reversible_custom_hook_survives_downgrade_route() {
    use havok_native::hkx::HkxMember;
    use havok_native::hkx::types::HkxValue;

    let mut manager = PatchManager::new();
    manager.register_hook("forward_marker", |context| {
        let index = context
            .object_index
            .expect("hook should receive object index");
        context.hkx.objects_mut()[index].members.push(HkxMember {
            name: "forward_hook_ran".into(),
            value: HkxValue::Bool(true),
        });
        Ok(())
    });
    manager.register_hook("inverse_marker", |context| {
        let index = context
            .object_index
            .expect("hook should receive object index");
        context.hkx.objects_mut()[index].members.push(HkxMember {
            name: "inverse_hook_ran".into(),
            value: HkxValue::Bool(true),
        });
        Ok(())
    });
    manager.register(
        48,
        Patch::new(
            ClassVersion::new("HookedObject", 0),
            ClassVersion::new("HookedObject", 1),
        )
        .with_reversible_custom_hook("forward_marker", "inverse_marker"),
    );
    let route = manager.route(48, 47).unwrap();
    assert_eq!(route.steps[0].patches[0].custom_hooks, ["inverse_marker"]);

    let mut hkx = havok_native::hkx::HkxFile::from_tagxml(
        11,
        "hk_2013.1.0-r1",
        vec![havok_native::hkx::HkxObject {
            name: Some("#0001".into()),
            offset: 0,
            signature: 1,
            class_name: "HookedObject".into(),
            members: vec![],
        }],
    );

    manager.convert_hkx(&mut hkx, 48, 47).unwrap();

    let object = &hkx.objects()[0];
    assert_eq!(object.signature, 0);
    assert!(
        object
            .members
            .iter()
            .any(|member| member.name == "inverse_hook_ran"),
        "downgrade route did not run inverse hook"
    );
    assert!(
        !object
            .members
            .iter()
            .any(|member| member.name == "forward_hook_ran"),
        "downgrade route ran forward hook instead of inverse hook"
    );
}

#[test]
fn custom_hook_receives_whole_file_context() {
    use havok_native::hkx::HkxMember;
    use havok_native::hkx::types::HkxValue;

    let mut registry = CustomHookRegistry::new();
    registry.register("touch_all", |context| {
        for object in context.hkx.objects_mut() {
            object.members.push(HkxMember {
                name: "hooked".into(),
                value: HkxValue::Bool(true),
            });
        }
        Ok(())
    });
    let mut hkx = havok_native::hkx::HkxFile::from_tagxml(
        11,
        "hk_2014.1.0-r1",
        vec![
            havok_native::hkx::HkxObject {
                name: Some("#0001".into()),
                offset: 0,
                signature: 0,
                class_name: "A".into(),
                members: vec![],
            },
            havok_native::hkx::HkxObject {
                name: Some("#0002".into()),
                offset: 0,
                signature: 0,
                class_name: "B".into(),
                members: vec![],
            },
        ],
    );
    let mut context = ConversionContext::new(&mut hkx, 53, 56, "test-hook");

    registry.invoke("touch_all", &mut context).unwrap();

    assert!(
        hkx.objects()
            .iter()
            .all(|object| object.members.iter().any(|member| member.name == "hooked"))
    );
}

#[test]
fn patch_manager_records_and_runs_custom_hook_operations_inline() {
    use havok_native::hkx::HkxMember;
    use havok_native::hkx::types::HkxValue;

    let mut manager = PatchManager::new();
    manager.register_hook("copy_group_filter_legacy_values", |context| {
        let index = context
            .object_index
            .expect("custom patch hooks should receive the current object index");
        let object = &mut context.hkx.objects_mut()[index];
        let old_next = object
            .members
            .iter()
            .find(|member| member.name == "old_nextFreeSystemGroup")
            .map(|member| member.value.clone());
        let old_lookup = object
            .members
            .iter()
            .find(|member| member.name == "old_collisionLookupTable")
            .map(|member| member.value.clone());
        if let Some(value) = old_next {
            if let Some(member) = object
                .members
                .iter_mut()
                .find(|member| member.name == "nextFreeSystemGroup")
            {
                member.value = value;
            }
        }
        if let Some(value) = old_lookup {
            if let Some(member) = object
                .members
                .iter_mut()
                .find(|member| member.name == "collisionLookupTable")
            {
                member.value = value;
            }
        }
        Ok(())
    });
    let patch = Patch::new(
        ClassVersion::new("hkpGroupFilter", 0),
        ClassVersion::new("hkpGroupFilter", 1),
    )
    .with_operation(PatchOperation::MemberRename {
        old_name: "nextFreeSystemGroup".into(),
        new_name: "old_nextFreeSystemGroup".into(),
    })
    .with_operation(PatchOperation::MemberRename {
        old_name: "collisionLookupTable".into(),
        new_name: "old_collisionLookupTable".into(),
    })
    .with_operation(PatchOperation::MemberRemove {
        name: "pad256".into(),
        type_name: "vec4".into(),
    })
    .with_operation(PatchOperation::CustomHook {
        name: "copy_group_filter_legacy_values".into(),
        inverse_name: None,
    })
    .with_operation(PatchOperation::MemberRemove {
        name: "old_nextFreeSystemGroup".into(),
        type_name: "int".into(),
    })
    .with_operation(PatchOperation::MemberRemove {
        name: "old_collisionLookupTable".into(),
        type_name: "array".into(),
    });
    assert_eq!(patch.custom_hooks, ["copy_group_filter_legacy_values"]);
    manager.register(56, patch);
    let route = manager.route(55, 56).unwrap();
    assert_eq!(
        route.steps[0].patches[0].custom_hooks,
        ["copy_group_filter_legacy_values"]
    );
    let mut hkx = havok_native::hkx::HkxFile::from_tagxml(
        11,
        "hk_2014.2.0-r1",
        vec![
            havok_native::hkx::HkxObject {
                name: Some("#0001".into()),
                offset: 0,
                signature: 0,
                class_name: "hkpGroupFilter".into(),
                members: vec![
                    HkxMember {
                        name: "nextFreeSystemGroup".into(),
                        value: HkxValue::I32(7),
                    },
                    HkxMember {
                        name: "nextFreeSystemGroup".into(),
                        value: HkxValue::I32(0),
                    },
                    HkxMember {
                        name: "collisionLookupTable".into(),
                        value: HkxValue::Array(vec![HkxValue::I32(4), HkxValue::I32(5)]),
                    },
                    HkxMember {
                        name: "collisionLookupTable".into(),
                        value: HkxValue::Array(vec![HkxValue::I32(0), HkxValue::I32(0)]),
                    },
                    HkxMember {
                        name: "pad256".into(),
                        value: HkxValue::F32List(vec![0.0; 4]),
                    },
                ],
            },
            havok_native::hkx::HkxObject {
                name: Some("#0002".into()),
                offset: 0,
                signature: 0,
                class_name: "hkpGroupFilter".into(),
                members: vec![
                    HkxMember {
                        name: "nextFreeSystemGroup".into(),
                        value: HkxValue::I32(11),
                    },
                    HkxMember {
                        name: "nextFreeSystemGroup".into(),
                        value: HkxValue::I32(0),
                    },
                    HkxMember {
                        name: "collisionLookupTable".into(),
                        value: HkxValue::Array(vec![HkxValue::I32(8), HkxValue::I32(9)]),
                    },
                    HkxMember {
                        name: "collisionLookupTable".into(),
                        value: HkxValue::Array(vec![HkxValue::I32(0), HkxValue::I32(0)]),
                    },
                    HkxMember {
                        name: "pad256".into(),
                        value: HkxValue::F32List(vec![0.0; 4]),
                    },
                ],
            },
        ],
    );

    manager.convert_hkx(&mut hkx, 55, 56).unwrap();

    let object = &hkx.objects()[0];
    assert_eq!(object.signature, 1);
    assert!(!object.members.iter().any(|member| member.name == "pad256"));
    assert!(
        !object
            .members
            .iter()
            .any(|member| member.name == "old_nextFreeSystemGroup")
    );
    assert!(
        !object
            .members
            .iter()
            .any(|member| member.name == "old_collisionLookupTable")
    );
    assert!(
        object
            .members
            .iter()
            .any(|member| member.name == "nextFreeSystemGroup" && member.value == HkxValue::I32(7))
    );
    assert!(object.members.iter().any(|member| {
        member.name == "collisionLookupTable"
            && member.value == HkxValue::Array(vec![HkxValue::I32(4), HkxValue::I32(5)])
    }));
    let second_object = &hkx.objects()[1];
    assert!(second_object.members.iter().any(|member| {
        member.name == "nextFreeSystemGroup" && member.value == HkxValue::I32(11)
    }));
    assert!(second_object.members.iter().any(|member| {
        member.name == "collisionLookupTable"
            && member.value == HkxValue::Array(vec![HkxValue::I32(8), HkxValue::I32(9)])
    }));
}

#[test]
fn convert_bytes_supported_noop_outputs_are_structurally_valid() {
    for fixture in [
        "native/havok/tests/fixtures/skeleton.hkx",
        "../bacup/py_bacup_lib/python/bacup_lib/tests/fixtures/creatures/deathclaw/expected/character.hkx",
    ] {
        let data = fixture_bytes(fixture);

        let converted = api::havok_convert_bytes(&data, "fo4").unwrap();

        assert_eq!(converted, data);
        havok_native::hkx::read_packfile(&converted).unwrap();
    }
}

// FO76 stores multi-shape ragdoll bodies as a *static* `hknpCompoundShape`
// (isMutable=0) with an EMPTY `instances` array — children + per-child
// transforms are baked into the boundingVolumeData BVH tree. The FO4 target,
// `hknpDynamicCompoundShape`, reads `instances`; left empty it crashes the CK on
// load (reads element[0] of a 0-len array) and the game on the physics-settle
// worker (null hknpBody, +18C4195). Each child's translation is recovered from
// the baked tree leaf AABBs. The AntiAirTurret ragdoll body[1] is a capsule +
// two side polytopes at ±~1.714 on X, so identity transforms would collapse
// both panels to center.
#[test]
fn fo76_static_ragdoll_compound_instances_are_repopulated() {
    let data = fixture_bytes("native/havok/tests/fixtures/fo76_antiairturret_ragdoll.hkx");
    let bytes = api::havok_convert_bytes(&data, "fo4").expect("FO76→FO4 should succeed");
    let parsed = havok_native::hkx::read_packfile(&bytes).expect("output parseable");

    let compound = parsed
        .objects()
        .iter()
        .find(|o| o.class_name == "hknpDynamicCompoundShape")
        .expect("turret ragdoll should produce a dynamic compound");

    let instances = compound
        .members
        .iter()
        .find(|m| m.name == "instances")
        .expect("compound must have an instances member");
    let elements = instances
        .value
        .as_object_members()
        .and_then(|members| members.iter().find(|m| m.name == "elements"))
        .map(|m| &m.value)
        .expect("instances must contain an elements member");
    let HkxValue::Array(items) = elements else {
        panic!("instances.elements must be an array");
    };

    assert_eq!(
        items.len(),
        3,
        "degenerate empty compound must be repopulated with the 3 child instances"
    );

    // hkTransform is three rotation columns then the translation column.
    let mut txs: Vec<f32> = items
        .iter()
        .map(|item| {
            let members = item.as_object_members().expect("instance is a struct");
            let transform = members
                .iter()
                .find(|m| m.name == "transform")
                .expect("instance has a transform");
            let HkxValue::F32List(f) = &transform.value else {
                panic!("transform must be an F32List");
            };
            assert_eq!(f.len(), 16, "hkTransform is 16 floats");
            f[12]
        })
        .collect();
    txs.sort_by(|a, b| a.partial_cmp(b).unwrap());

    assert!(
        (txs[0] - (-1.714)).abs() < 0.05,
        "left side-panel translation X ~ -1.714, got {}",
        txs[0]
    );
    assert!(
        txs[1].abs() < 0.05,
        "capsule translation X ~ 0, got {}",
        txs[1]
    );
    assert!(
        (txs[2] - 1.714).abs() < 0.05,
        "right side-panel translation X ~ +1.714, got {}",
        txs[2]
    );

    // Every converted convex polytope must dispatch as CONVEX (1), not the
    // FO76 COMPOSITE (2). A convex polytope left at dispatchType=2 makes the
    // FO4 physics engine treat it as a composite shape, walk phantom child
    // shape-keys, and resolve a null hknpBody on contact → CTD at
    // hknpBSShapeCodec::decodeImpl (Fallout4.exe+18C4195) during cell settle.
    for poly in parsed
        .objects()
        .iter()
        .filter(|o| o.class_name == "hknpConvexPolytopeShape")
    {
        let dispatch = poly
            .members
            .iter()
            .find(|m| m.name == "dispatchType")
            .map(|m| &m.value);
        assert!(
            matches!(
                dispatch,
                Some(HkxValue::I8(1))
                    | Some(HkxValue::U8(1))
                    | Some(HkxValue::I16(1))
                    | Some(HkxValue::U16(1))
                    | Some(HkxValue::I32(1))
                    | Some(HkxValue::U32(1))
            ),
            "convex polytope must have dispatchType=1 (CONVEX), got {dispatch:?}"
        );
    }
}

/// FO76-only classes that must NOT survive into the FO4 output. Absence of
/// these is a structural invariant of any successful FO76→FO4 convert.
/// Cross-checked against `resource/classxml/` to keep this list strictly to
/// classes that have no FO4 classxml entry.
const FO76_ONLY_CLASSES: &[&str] = &["hknpRefMassDistribution", "hknpMassDistribution"];

fn assert_fo4_packfile_invariants(bytes: &[u8], fixture: &str) {
    assert_eq!(&bytes[..8], b"\x57\xE0\xE0\x57\x10\xC0\xC0\x10", "{fixture}: packfile magic");
    let parsed = havok_native::hkx::read_packfile(bytes).unwrap_or_else(|err| {
        panic!("{fixture}: converted output must parse as FO4 packfile: {err}");
    });
    assert_eq!(
        parsed.class_version(),
        11,
        "{fixture}: FO4 packfile version must be 11 (got {})",
        parsed.class_version()
    );
    assert_eq!(
        parsed.contents_version(),
        "hk_2014.1.0-r1",
        "{fixture}: contents_version must be hk_2014.1.0-r1 (got {:?})",
        parsed.contents_version()
    );
    assert!(
        !parsed.objects().is_empty(),
        "{fixture}: converted output must have at least one object"
    );
    assert!(
        parsed
            .objects()
            .iter()
            .any(|o| o.class_name == "hkRootLevelContainer"),
        "{fixture}: converted output must contain hkRootLevelContainer"
    );
    for obj in parsed.objects() {
        assert!(
            !FO76_ONLY_CLASSES.contains(&obj.class_name.as_str()),
            "{fixture}: FO76-only class '{}' survived into FO4 output",
            obj.class_name
        );
    }
}

#[test]
fn fo76_to_fo4_convert_produces_well_formed_fo4_packfiles() {
    for fixture in [
        "python/creation_lib/hkxpack/tests/fixtures/fo76_snallygastercharacter.hkx",
        "../bacup/py_bacup_lib/python/bacup_lib/tests/fixtures/creatures/deathclaw/source/character.hkx",
        "../bacup/py_bacup_lib/python/bacup_lib/tests/fixtures/creatures/snallygaster/source/character.hkx",
        "../bacup/py_bacup_lib/python/bacup_lib/tests/fixtures/creatures/floater/source/character.hkx",
    ] {
        let converted = api::havok_convert_bytes(&fixture_bytes(fixture), "fo4")
            .unwrap_or_else(|err| panic!("{fixture}: FO76→FO4 conversion failed: {err}"));
        assert_fo4_packfile_invariants(&converted, fixture);
    }
}

#[test]
fn fo76_to_fo4_deathclaw_character_rewires_driver_pointers() {
    let source = fixture_bytes(
        "../bacup/py_bacup_lib/python/bacup_lib/tests/fixtures/creatures/deathclaw/source/character.hkx",
    );
    let converted = api::havok_convert_bytes(&source, "fo4")
        .expect("FO76→FO4 deathclaw conversion must succeed");
    let parsed = havok_native::hkx::read_packfile(&converted).expect("output parseable");
    let character = parsed
        .objects()
        .iter()
        .find(|object| object.class_name == "hkbCharacterData")
        .expect("hkbCharacterData present");

    for (member_name, target_class) in [
        ("footIkDriverInfo", "hkbFootIkDriverInfo"),
        ("mirroredSkeletonInfo", "hkbMirroredSkeletonInfo"),
    ] {
        let member = character
            .members
            .iter()
            .find(|member| member.name == member_name)
            .unwrap_or_else(|| panic!("{member_name} member present"));
        let HkxValue::Pointer(Some(target_index)) = member.value else {
            panic!(
                "{member_name} should point at {target_class}, got {:?}",
                member.value
            );
        };
        assert_eq!(
            parsed.objects()[target_index].class_name,
            target_class,
            "{member_name} should target {target_class}"
        );
    }
}

#[test]
fn tag0_var0_object_materializes_t_n_inline_fixed_array() {
    // T[N] is the TAG0 generic for fixed-size inline C arrays (e.g.
    // hkUint32[8] for hkbGeneratorPartitionInfo.boneMask). The element
    // count is encoded as the type's total size divided by the element
    // stride, with no runtime header.
    use havok_native::hkx::tagfile::{
        TagField, TagType, TagTypeRegistry, Tagfile, TagfileItem, TagfileSection,
    };
    use havok_native::hkx::types::HkxValue;

    let mut payload = Vec::new();
    for value in [1u32, 2, 3, 4, 5, 6, 7, 8] {
        payload.extend_from_slice(&value.to_le_bytes());
    }

    let tagfile = Tagfile::from_synthetic_parts(
        "20150100",
        "hk_2015.1.0-r1",
        vec![TagfileSection {
            tag: "DATA".into(),
            offset: 0,
            size: payload.len(),
            content_offset: 0,
            content_size: payload.len(),
            scope: 1,
        }],
        TagTypeRegistry {
            types: vec![
                TagType {
                    id: 0,
                    name: "hkRootLevelContainer".into(),
                    parent_id: 0,
                    kind: 0,
                    subtype_id: 0,
                    size: 0,
                    align: 0,
                    version: 0,
                    format_value: 0,
                    signed: false,
                    fields: vec![],
                },
                TagType {
                    id: 1,
                    name: "hkUint32".into(),
                    parent_id: 0,
                    kind: 4,
                    subtype_id: 0,
                    size: 4,
                    align: 4,
                    version: 0,
                    format_value: 0,
                    signed: false,
                    fields: vec![],
                },
                TagType {
                    id: 2,
                    name: "T[N]".into(),
                    parent_id: 0,
                    kind: 8,
                    subtype_id: 1,
                    size: 32,
                    align: 4,
                    version: 0,
                    format_value: 0,
                    signed: false,
                    fields: vec![],
                },
                TagType {
                    id: 3,
                    name: "PartitionInfoLike".into(),
                    parent_id: 0,
                    kind: 6,
                    subtype_id: 0,
                    size: 32,
                    align: 4,
                    version: 0,
                    format_value: 0,
                    signed: false,
                    fields: vec![TagField {
                        name: "boneMask".into(),
                        type_id: 2,
                        offset: 0,
                        flags: 0,
                    }],
                },
            ],
        },
        vec![
            TagfileItem {
                kind: 0,
                flags: 0,
                type_id: 0,
                offset: 0,
                count: 0,
            },
            TagfileItem {
                kind: 1,
                flags: 0,
                type_id: 3,
                offset: 0,
                count: 1,
            },
        ],
        payload,
    );

    let hkx = tagfile.materialize_hkx().unwrap();

    assert_eq!(hkx.objects().len(), 1);
    let object = &hkx.objects()[0];
    assert_eq!(object.class_name, "PartitionInfoLike");
    assert_eq!(object.members.len(), 1);
    assert_eq!(object.members[0].name, "boneMask");
    let HkxValue::Array(elements) = &object.members[0].value else {
        panic!(
            "expected boneMask to materialize as Array, got {:?}",
            object.members[0].value
        );
    };
    assert_eq!(elements.len(), 8);
    for (index, element) in elements.iter().enumerate() {
        let expected = (index + 1) as u32;
        assert_eq!(element, &HkxValue::U32(expected), "element {index}");
    }
}

#[test]
fn tag0_var0_object_materializes_string_member_via_item_index_when_in_ptch() {
    // hkStringPtr fields are item-indexed (the u32 is a VARN item index, not a
    // raw DATA offset), and only treated as a string when the field offset is in
    // PTCH. See
    // py_creation_lib/python/creation_lib/hkxpack/tagfile_reader.py:983-986 + _read_string_ref.
    use havok_native::hkx::tagfile::{
        TagField, TagType, TagTypeRegistry, Tagfile, TagfileItem, TagfileSection,
    };
    use havok_native::hkx::types::HkxValue;

    let payload = b"hello\0";
    let payload_offset = 8usize;
    let mut data = 2_u64.to_le_bytes().to_vec(); // u32 item index = 2 in low half
    data.extend_from_slice(payload);

    let mut tagfile = Tagfile::from_synthetic_parts(
        "20150100",
        "hk_2015.1.0-r1",
        vec![TagfileSection {
            tag: "DATA".into(),
            offset: 0,
            size: data.len(),
            content_offset: 0,
            content_size: data.len(),
            scope: 1,
        }],
        TagTypeRegistry {
            types: vec![
                TagType {
                    id: 0,
                    name: "hkRootLevelContainer".into(),
                    parent_id: 0,
                    kind: 0,
                    subtype_id: 0,
                    size: 0,
                    align: 0,
                    version: 0,
                    format_value: 0,
                    signed: false,
                    fields: vec![],
                },
                TagType {
                    id: 1,
                    name: "hkStringPtr".into(),
                    parent_id: 0,
                    kind: 3,
                    subtype_id: 0,
                    size: 8,
                    align: 8,
                    version: 0,
                    format_value: 0,
                    signed: false,
                    fields: vec![],
                },
                TagType {
                    id: 2,
                    name: "TestObject".into(),
                    parent_id: 0,
                    kind: 7,
                    subtype_id: 0,
                    size: 8,
                    align: 8,
                    version: 17,
                    format_value: 0,
                    signed: false,
                    fields: vec![TagField {
                        name: "label".into(),
                        type_id: 1,
                        offset: 0,
                        flags: 0,
                    }],
                },
            ],
        },
        vec![
            TagfileItem {
                kind: 0,
                flags: 0,
                type_id: 0,
                offset: 0,
                count: 0,
            },
            TagfileItem {
                kind: 1,
                flags: 0,
                type_id: 2,
                offset: 0,
                count: 1,
            },
            // index 2: VARN char payload
            TagfileItem {
                kind: 2,
                flags: 0,
                type_id: 1,
                offset: payload_offset,
                count: payload.len(),
            },
        ],
        data,
    );
    tagfile.set_pointer_offsets_for_test(vec![0]);

    let hkx = tagfile.materialize_hkx().unwrap();

    assert_eq!(hkx.objects()[0].members[0].name, "label");
    assert_eq!(
        hkx.objects()[0].members[0].value,
        HkxValue::String {
            value: "hello".into(),
            is_null: false,
        }
    );
}

#[test]
fn tag0_var0_object_materializes_hkarray_scalar_varn_member() {
    use havok_native::hkx::tagfile::{
        TagField, TagType, TagTypeRegistry, Tagfile, TagfileItem, TagfileSection,
    };
    use havok_native::hkx::types::HkxValue;

    let mut data = Vec::new();
    // Synthetic TAG0 hkArray header:
    // u32 VARN item index, u32 element count, u64 reserved.
    data.extend_from_slice(&2_u32.to_le_bytes());
    data.extend_from_slice(&3_u32.to_le_bytes());
    data.extend_from_slice(&0_u64.to_le_bytes());
    for value in [7_i32, 8, 9] {
        data.extend_from_slice(&value.to_le_bytes());
    }

    let tagfile = Tagfile::from_synthetic_parts(
        "20150100",
        "hk_2015.1.0-r1",
        vec![TagfileSection {
            tag: "DATA".into(),
            offset: 0,
            size: data.len(),
            content_offset: 0,
            content_size: data.len(),
            scope: 1,
        }],
        TagTypeRegistry {
            types: vec![
                TagType {
                    id: 0,
                    name: "hkRootLevelContainer".into(),
                    parent_id: 0,
                    kind: 0,
                    subtype_id: 0,
                    size: 0,
                    align: 0,
                    version: 0,
                    format_value: 0,
                    signed: false,
                    fields: vec![],
                },
                TagType {
                    id: 1,
                    name: "int".into(),
                    parent_id: 0,
                    kind: 4,
                    subtype_id: 0,
                    size: 4,
                    align: 4,
                    version: 0,
                    format_value: 0,
                    signed: true,
                    fields: vec![],
                },
                TagType {
                    id: 2,
                    name: "hkArray".into(),
                    parent_id: 0,
                    kind: 8,
                    subtype_id: 1,
                    size: 16,
                    align: 8,
                    version: 0,
                    format_value: 0,
                    signed: false,
                    fields: vec![],
                },
                TagType {
                    id: 3,
                    name: "TestObject".into(),
                    parent_id: 0,
                    kind: 6,
                    subtype_id: 0,
                    size: 16,
                    align: 8,
                    version: 17,
                    format_value: 0,
                    signed: false,
                    fields: vec![TagField {
                        name: "values".into(),
                        type_id: 2,
                        offset: 0,
                        flags: 0,
                    }],
                },
            ],
        },
        vec![
            TagfileItem {
                kind: 0,
                flags: 0,
                type_id: 0,
                offset: 0,
                count: 0,
            },
            TagfileItem {
                kind: 1,
                flags: 0,
                type_id: 3,
                offset: 0,
                count: 1,
            },
            TagfileItem {
                kind: 2,
                flags: 0,
                type_id: 1,
                offset: 16,
                count: 3,
            },
        ],
        data,
    );

    let hkx = tagfile.materialize_hkx().unwrap();

    assert_eq!(hkx.objects()[0].members[0].name, "values");
    assert_eq!(
        hkx.objects()[0].members[0].value,
        HkxValue::Array(vec![HkxValue::I32(7), HkxValue::I32(8), HkxValue::I32(9)])
    );
}

#[test]
fn tag0_var0_object_materializes_nested_kind7_object_member() {
    use havok_native::hkx::HkxMember;
    use havok_native::hkx::tagfile::{
        TagField, TagType, TagTypeRegistry, Tagfile, TagfileItem, TagfileSection,
    };
    use havok_native::hkx::types::HkxValue;

    let mut data = Vec::new();
    data.extend_from_slice(&42_i32.to_le_bytes());
    data.extend_from_slice(&[0; 12]);
    for value in [1.0_f32, 2.0, 3.0, 4.0] {
        data.extend_from_slice(&value.to_le_bytes());
    }

    let tagfile = Tagfile::from_synthetic_parts(
        "20150100",
        "hk_2015.1.0-r1",
        vec![TagfileSection {
            tag: "DATA".into(),
            offset: 0,
            size: data.len(),
            content_offset: 0,
            content_size: data.len(),
            scope: 1,
        }],
        TagTypeRegistry {
            types: vec![
                TagType {
                    id: 0,
                    name: "hkRootLevelContainer".into(),
                    parent_id: 0,
                    kind: 0,
                    subtype_id: 0,
                    size: 0,
                    align: 0,
                    version: 0,
                    format_value: 0,
                    signed: false,
                    fields: vec![],
                },
                TagType {
                    id: 1,
                    name: "int".into(),
                    parent_id: 0,
                    kind: 4,
                    subtype_id: 0,
                    size: 4,
                    align: 4,
                    version: 0,
                    format_value: 0,
                    signed: true,
                    fields: vec![],
                },
                TagType {
                    id: 2,
                    name: "hkVector4".into(),
                    parent_id: 0,
                    kind: 0,
                    subtype_id: 0,
                    size: 16,
                    align: 16,
                    version: 0,
                    format_value: 0,
                    signed: false,
                    fields: vec![],
                },
                TagType {
                    id: 3,
                    name: "NestedSetup".into(),
                    parent_id: 0,
                    kind: 7,
                    subtype_id: 0,
                    size: 32,
                    align: 16,
                    version: 0,
                    format_value: 0,
                    signed: false,
                    fields: vec![
                        TagField {
                            name: "priority".into(),
                            type_id: 1,
                            offset: 0,
                            flags: 0,
                        },
                        TagField {
                            name: "axis".into(),
                            type_id: 2,
                            offset: 16,
                            flags: 0,
                        },
                    ],
                },
                TagType {
                    id: 4,
                    name: "ParentObject".into(),
                    parent_id: 0,
                    kind: 6,
                    subtype_id: 0,
                    size: 32,
                    align: 16,
                    version: 3,
                    format_value: 0,
                    signed: false,
                    fields: vec![TagField {
                        name: "setup".into(),
                        type_id: 3,
                        offset: 0,
                        flags: 0,
                    }],
                },
            ],
        },
        vec![
            TagfileItem {
                kind: 0,
                flags: 0,
                type_id: 0,
                offset: 0,
                count: 0,
            },
            TagfileItem {
                kind: 1,
                flags: 0,
                type_id: 4,
                offset: 0,
                count: 1,
            },
        ],
        data,
    );

    let hkx = tagfile.materialize_hkx().unwrap();

    assert_eq!(hkx.objects()[0].members[0].name, "setup");
    assert_eq!(
        hkx.objects()[0].members[0].value,
        HkxValue::Object(vec![
            HkxMember {
                name: "priority".into(),
                value: HkxValue::I32(42),
            },
            HkxMember {
                name: "axis".into(),
                value: HkxValue::F32List(vec![1.0, 2.0, 3.0, 4.0]),
            },
        ])
    );
}

#[test]
fn tag0_var0_object_materializes_non_null_pointer_as_object_index() {
    use havok_native::hkx::tagfile::{
        TagField, TagType, TagTypeRegistry, Tagfile, TagfileItem, TagfileSection,
    };
    use havok_native::hkx::tagxml::write_tagxml_string;
    use havok_native::hkx::types::HkxValue;

    let mut data = 3_u32.to_le_bytes().to_vec();
    data.extend_from_slice(&0_u32.to_le_bytes());
    data.extend_from_slice(&42_i32.to_le_bytes());
    let tagfile = Tagfile::from_synthetic_parts(
        "20150100",
        "hk_2015.1.0-r1",
        vec![TagfileSection {
            tag: "DATA".into(),
            offset: 0,
            size: data.len(),
            content_offset: 0,
            content_size: data.len(),
            scope: 1,
        }],
        TagTypeRegistry {
            types: vec![
                TagType {
                    id: 0,
                    name: "hkRootLevelContainer".into(),
                    parent_id: 0,
                    kind: 0,
                    subtype_id: 0,
                    size: 0,
                    align: 0,
                    version: 0,
                    format_value: 0,
                    signed: false,
                    fields: vec![],
                },
                TagType {
                    id: 1,
                    name: "hkRefPtr".into(),
                    parent_id: 0,
                    kind: 6,
                    subtype_id: 0,
                    size: 8,
                    align: 8,
                    version: 0,
                    format_value: 0,
                    signed: false,
                    fields: vec![],
                },
                TagType {
                    id: 2,
                    name: "int".into(),
                    parent_id: 0,
                    kind: 4,
                    subtype_id: 0,
                    size: 4,
                    align: 4,
                    version: 0,
                    format_value: 0,
                    signed: true,
                    fields: vec![],
                },
                TagType {
                    id: 3,
                    name: "TestObject".into(),
                    parent_id: 0,
                    kind: 6,
                    subtype_id: 0,
                    size: 8,
                    align: 8,
                    version: 17,
                    format_value: 0,
                    signed: false,
                    fields: vec![TagField {
                        name: "target".into(),
                        type_id: 1,
                        offset: 0,
                        flags: 0,
                    }],
                },
                TagType {
                    id: 4,
                    name: "TargetObject".into(),
                    parent_id: 0,
                    kind: 6,
                    subtype_id: 0,
                    size: 4,
                    align: 4,
                    version: 23,
                    format_value: 0,
                    signed: false,
                    fields: vec![TagField {
                        name: "answer".into(),
                        type_id: 2,
                        offset: 0,
                        flags: 0,
                    }],
                },
            ],
        },
        vec![
            TagfileItem {
                kind: 0,
                flags: 0,
                type_id: 0,
                offset: 0,
                count: 0,
            },
            TagfileItem {
                kind: 1,
                flags: 0,
                type_id: 3,
                offset: 0,
                count: 1,
            },
            TagfileItem {
                kind: 2,
                flags: 0,
                type_id: 2,
                offset: 8,
                count: 1,
            },
            TagfileItem {
                kind: 1,
                flags: 0,
                type_id: 4,
                offset: 8,
                count: 1,
            },
        ],
        data,
    );

    let hkx = tagfile.materialize_hkx().unwrap();

    assert_eq!(hkx.objects().len(), 2);
    assert_eq!(hkx.objects()[1].name.as_deref(), Some("#0002"));
    assert_eq!(hkx.objects()[0].members[0].name, "target");
    assert_eq!(
        hkx.objects()[0].members[0].value,
        HkxValue::Pointer(Some(1))
    );

    let tagxml = write_tagxml_string(&hkx).unwrap();
    assert!(tagxml.contains("name=\"#0002\""));
    assert!(tagxml.contains(">#0002</hkparam>"));
}

#[test]
fn tag0_pointer_rejects_out_of_range_item_index() {
    use havok_native::hkx::tagfile::{
        TagField, TagType, TagTypeRegistry, Tagfile, TagfileItem, TagfileSection,
    };

    let data = 2_u32.to_le_bytes().to_vec();
    let tagfile = Tagfile::from_synthetic_parts(
        "20150100",
        "hk_2015.1.0-r1",
        vec![TagfileSection {
            tag: "DATA".into(),
            offset: 0,
            size: data.len(),
            content_offset: 0,
            content_size: data.len(),
            scope: 1,
        }],
        TagTypeRegistry {
            types: vec![
                TagType {
                    id: 0,
                    name: "hkRootLevelContainer".into(),
                    parent_id: 0,
                    kind: 0,
                    subtype_id: 0,
                    size: 0,
                    align: 0,
                    version: 0,
                    format_value: 0,
                    signed: false,
                    fields: vec![],
                },
                TagType {
                    id: 1,
                    name: "hkRefPtr".into(),
                    parent_id: 0,
                    kind: 6,
                    subtype_id: 0,
                    size: 4,
                    align: 4,
                    version: 0,
                    format_value: 0,
                    signed: false,
                    fields: vec![],
                },
                TagType {
                    id: 2,
                    name: "TestObject".into(),
                    parent_id: 0,
                    kind: 6,
                    subtype_id: 0,
                    size: 4,
                    align: 4,
                    version: 17,
                    format_value: 0,
                    signed: false,
                    fields: vec![TagField {
                        name: "target".into(),
                        type_id: 1,
                        offset: 0,
                        flags: 0,
                    }],
                },
            ],
        },
        vec![
            TagfileItem {
                kind: 0,
                flags: 0,
                type_id: 0,
                offset: 0,
                count: 0,
            },
            TagfileItem {
                kind: 1,
                flags: 0,
                type_id: 2,
                offset: 0,
                count: 1,
            },
        ],
        data,
    );

    let error = tagfile.materialize_hkx().unwrap_err();
    let message = error.to_string();

    assert!(message.contains("TAG0 pointer item index 2 is not in ITEM table"));
    assert!(message.contains("member target"));
}

#[test]
fn tag0_pointer_rejects_truncated_data_slot() {
    use havok_native::hkx::tagfile::{
        TagField, TagType, TagTypeRegistry, Tagfile, TagfileItem, TagfileSection,
    };

    let tagfile = Tagfile::from_synthetic_parts(
        "20150100",
        "hk_2015.1.0-r1",
        vec![TagfileSection {
            tag: "DATA".into(),
            offset: 0,
            size: 3,
            content_offset: 0,
            content_size: 3,
            scope: 1,
        }],
        TagTypeRegistry {
            types: vec![
                TagType {
                    id: 0,
                    name: "hkRootLevelContainer".into(),
                    parent_id: 0,
                    kind: 0,
                    subtype_id: 0,
                    size: 0,
                    align: 0,
                    version: 0,
                    format_value: 0,
                    signed: false,
                    fields: vec![],
                },
                TagType {
                    id: 1,
                    name: "hkRefPtr".into(),
                    parent_id: 0,
                    kind: 6,
                    subtype_id: 0,
                    size: 4,
                    align: 4,
                    version: 0,
                    format_value: 0,
                    signed: false,
                    fields: vec![],
                },
                TagType {
                    id: 2,
                    name: "TestObject".into(),
                    parent_id: 0,
                    kind: 6,
                    subtype_id: 0,
                    size: 4,
                    align: 4,
                    version: 17,
                    format_value: 0,
                    signed: false,
                    fields: vec![TagField {
                        name: "target".into(),
                        type_id: 1,
                        offset: 0,
                        flags: 0,
                    }],
                },
            ],
        },
        vec![
            TagfileItem {
                kind: 0,
                flags: 0,
                type_id: 0,
                offset: 0,
                count: 0,
            },
            TagfileItem {
                kind: 1,
                flags: 0,
                type_id: 2,
                offset: 0,
                count: 1,
            },
        ],
        vec![0; 3],
    );

    let error = tagfile.materialize_hkx().unwrap_err();
    let message = error.to_string();

    assert!(message.contains("TAG0 pointer field at offset 0 size 4 is outside DATA"));
    assert!(message.contains("member target"));
}

#[test]
fn tag0_nested_object_materialization_detects_recursive_type_cycle() {
    use havok_native::hkx::tagfile::{
        TagField, TagType, TagTypeRegistry, Tagfile, TagfileItem, TagfileSection,
    };

    let tagfile = Tagfile::from_synthetic_parts(
        "20150100",
        "hk_2015.1.0-r1",
        vec![TagfileSection {
            tag: "DATA".into(),
            offset: 0,
            size: 4,
            content_offset: 0,
            content_size: 4,
            scope: 1,
        }],
        TagTypeRegistry {
            types: vec![
                TagType {
                    id: 0,
                    name: "hkRootLevelContainer".into(),
                    parent_id: 0,
                    kind: 0,
                    subtype_id: 0,
                    size: 0,
                    align: 0,
                    version: 0,
                    format_value: 0,
                    signed: false,
                    fields: vec![],
                },
                TagType {
                    id: 1,
                    name: "RecursiveSetup".into(),
                    parent_id: 0,
                    kind: 7,
                    subtype_id: 0,
                    size: 4,
                    align: 4,
                    version: 0,
                    format_value: 0,
                    signed: false,
                    fields: vec![TagField {
                        name: "next".into(),
                        type_id: 1,
                        offset: 0,
                        flags: 0,
                    }],
                },
                TagType {
                    id: 2,
                    name: "ParentObject".into(),
                    parent_id: 0,
                    kind: 6,
                    subtype_id: 0,
                    size: 4,
                    align: 4,
                    version: 0,
                    format_value: 0,
                    signed: false,
                    fields: vec![TagField {
                        name: "setup".into(),
                        type_id: 1,
                        offset: 0,
                        flags: 0,
                    }],
                },
            ],
        },
        vec![
            TagfileItem {
                kind: 0,
                flags: 0,
                type_id: 0,
                offset: 0,
                count: 0,
            },
            TagfileItem {
                kind: 1,
                flags: 0,
                type_id: 2,
                offset: 0,
                count: 1,
            },
        ],
        vec![0; 4],
    );

    let error = tagfile.materialize_hkx().unwrap_err();

    assert_eq!(
        error.to_string(),
        "invalid input: TAG0 field materialization failed for class ParentObject, member setup, type id 1 (RecursiveSetup) kind 7: invalid input: TAG0 nested member path setup.next: invalid input: TAG0 recursive nested object type cycle: RecursiveSetup -> RecursiveSetup"
    );
}

#[test]
fn tag0_nested_object_materialization_rejects_long_acyclic_type_chain() {
    use havok_native::hkx::tagfile::{
        TagField, TagType, TagTypeRegistry, Tagfile, TagfileItem, TagfileSection,
    };

    let mut types = vec![TagType {
        id: 0,
        name: "hkRootLevelContainer".into(),
        parent_id: 0,
        kind: 0,
        subtype_id: 0,
        size: 0,
        align: 0,
        version: 0,
        format_value: 0,
        signed: false,
        fields: vec![],
    }];
    for id in 1..80 {
        types.push(TagType {
            id,
            name: format!("Nested{id}"),
            parent_id: 0,
            kind: 7,
            subtype_id: 0,
            size: 4,
            align: 4,
            version: 0,
            format_value: 0,
            signed: false,
            fields: vec![TagField {
                name: "next".into(),
                type_id: id + 1,
                offset: 0,
                flags: 0,
            }],
        });
    }
    types.push(TagType {
        id: 80,
        name: "int".into(),
        parent_id: 0,
        kind: 4,
        subtype_id: 0,
        size: 4,
        align: 4,
        version: 0,
        format_value: 0,
        signed: true,
        fields: vec![],
    });
    types.push(TagType {
        id: 81,
        name: "ParentObject".into(),
        parent_id: 0,
        kind: 6,
        subtype_id: 0,
        size: 4,
        align: 4,
        version: 0,
        format_value: 0,
        signed: false,
        fields: vec![TagField {
            name: "setup".into(),
            type_id: 1,
            offset: 0,
            flags: 0,
        }],
    });

    let tagfile = Tagfile::from_synthetic_parts(
        "20150100",
        "hk_2015.1.0-r1",
        vec![TagfileSection {
            tag: "DATA".into(),
            offset: 0,
            size: 4,
            content_offset: 0,
            content_size: 4,
            scope: 1,
        }],
        TagTypeRegistry { types },
        vec![
            TagfileItem {
                kind: 0,
                flags: 0,
                type_id: 0,
                offset: 0,
                count: 0,
            },
            TagfileItem {
                kind: 1,
                flags: 0,
                type_id: 81,
                offset: 0,
                count: 1,
            },
        ],
        7_i32.to_le_bytes().to_vec(),
    );

    let error = tagfile.materialize_hkx().unwrap_err();
    let message = error.to_string();

    assert!(
        message.contains("TAG0 nested object materialization depth limit exceeded"),
        "unexpected error: {message}"
    );
    assert!(
        message.contains("setup.next.next"),
        "unexpected error: {message}"
    );
}

#[test]
fn tag0_var0_object_returns_error_for_huge_string_field_offset() {
    use havok_native::hkx::tagfile::{
        TagField, TagType, TagTypeRegistry, Tagfile, TagfileItem, TagfileSection,
    };

    let data = b"hello\0".to_vec();
    let tagfile = Tagfile::from_synthetic_parts(
        "20150100",
        "hk_2015.1.0-r1",
        vec![TagfileSection {
            tag: "DATA".into(),
            offset: 0,
            size: data.len(),
            content_offset: 0,
            content_size: data.len(),
            scope: 1,
        }],
        TagTypeRegistry {
            types: vec![
                TagType {
                    id: 0,
                    name: "hkRootLevelContainer".into(),
                    parent_id: 0,
                    kind: 0,
                    subtype_id: 0,
                    size: 0,
                    align: 0,
                    version: 0,
                    format_value: 0,
                    signed: false,
                    fields: vec![],
                },
                TagType {
                    id: 1,
                    name: "hkStringPtr".into(),
                    parent_id: 0,
                    kind: 3,
                    subtype_id: 0,
                    size: 8,
                    align: 8,
                    version: 0,
                    format_value: 0,
                    signed: false,
                    fields: vec![],
                },
                TagType {
                    id: 2,
                    name: "TestObject".into(),
                    parent_id: 0,
                    kind: 6,
                    subtype_id: 0,
                    size: 8,
                    align: 8,
                    version: 17,
                    format_value: 0,
                    signed: false,
                    fields: vec![TagField {
                        name: "label".into(),
                        type_id: 1,
                        offset: 0,
                        flags: 0,
                    }],
                },
            ],
        },
        vec![
            TagfileItem {
                kind: 0,
                flags: 0,
                type_id: 0,
                offset: 0,
                count: 0,
            },
            TagfileItem {
                kind: 1,
                flags: 0,
                type_id: 2,
                offset: usize::MAX,
                count: 1,
            },
        ],
        data,
    );

    let error = tagfile.materialize_hkx().unwrap_err();
    // Without PTCH the kind=3 field falls through to the scratch-int path,
    // which still must refuse to read past DATA at offset usize::MAX.
    assert_eq!(
        error.to_string(),
        "invalid input: TAG0 field materialization failed for class TestObject, member label, type id 1 (hkStringPtr) kind 3: invalid input: TAG0 string-as-int field at offset 18446744073709551615 is outside DATA"
    );
}

// ---------------------------------------------------------------------------
// Tests for callable hooks and the general patch-chain route in
// havok_convert_bytes.
// ---------------------------------------------------------------------------

fn build_hkx_with_object(
    class_name: &str,
    members: Vec<havok_native::hkx::HkxMember>,
) -> havok_native::hkx::HkxFile {
    havok_native::hkx::HkxFile::from_tagxml(
        11,
        "hk_2014.1.0-r1",
        vec![havok_native::hkx::HkxObject {
            name: Some("#0001".into()),
            offset: 0,
            signature: 0,
            class_name: class_name.into(),
            members,
        }],
    )
}

#[test]
fn hkb_character_3_to_4_hook_marks_capabilities_invalid() {
    use havok_native::convert::{ClassVersion, Patch, PatchManager};
    use havok_native::hkx::HkxMember;
    use havok_native::hkx::types::HkxValue;

    let mut hkx = build_hkx_with_object(
        "hkbCharacter",
        vec![
            HkxMember {
                name: "capabilities".into(),
                value: HkxValue::I32(7),
            },
            HkxMember {
                name: "effectiveCapabilities".into(),
                value: HkxValue::I32(99),
            },
        ],
    );
    hkx.objects_mut()[0].signature = 3;

    let mut manager = PatchManager::with_native_corpus();
    manager.force_corpus_complete();
    manager.register(
        50,
        Patch::new(
            ClassVersion::new("hkbCharacter", 3),
            ClassVersion::new("hkbCharacter", 4),
        )
        .with_custom_hook("_hkbCharacter_3_to_4"),
    );
    manager.convert_hkx(&mut hkx, 49, 50).unwrap();

    let object = &hkx.objects()[0];
    let cap = object
        .members
        .iter()
        .find(|m| m.name == "capabilities")
        .unwrap();
    let eff = object
        .members
        .iter()
        .find(|m| m.name == "effectiveCapabilities")
        .unwrap();
    assert_eq!(cap.value, HkxValue::I32(-1));
    assert_eq!(eff.value, HkxValue::I32(-1));
}

#[test]
fn hkp_ang_constraint_atom_hook_expands_axis_to_three_entries() {
    use havok_native::convert::{ClassVersion, Patch, PatchManager};
    use havok_native::hkx::HkxMember;
    use havok_native::hkx::types::HkxValue;

    let mut hkx = build_hkx_with_object(
        "hkpAngConstraintAtom",
        vec![
            HkxMember {
                name: "firstConstrainedAxis".into(),
                value: HkxValue::I8(2),
            },
            HkxMember {
                name: "constrainedAxes".into(),
                value: HkxValue::F32List(vec![0.0; 3]),
            },
        ],
    );

    let mut manager = PatchManager::with_native_corpus();
    manager.force_corpus_complete();
    manager.register(
        53,
        Patch::new(
            ClassVersion::new("hkpAngConstraintAtom", 0),
            ClassVersion::new("hkpAngConstraintAtom", 1),
        )
        .with_custom_hook("_hkpAngConstraintAtom_0_to_1"),
    );
    manager.convert_hkx(&mut hkx, 52, 53).unwrap();

    let axes = hkx.objects()[0]
        .members
        .iter()
        .find(|m| m.name == "constrainedAxes")
        .unwrap();
    // (2+0)%3=2, (2+1)%3=0, (2+2)%3=1
    assert_eq!(axes.value, HkxValue::F32List(vec![2.0, 0.0, 1.0]));
}

#[test]
fn hk_aabb_half_hook_merges_old_data_with_extras() {
    use havok_native::convert::{ClassVersion, Patch, PatchManager};
    use havok_native::hkx::HkxMember;
    use havok_native::hkx::types::HkxValue;

    let mut hkx = build_hkx_with_object(
        "hkAabbHalf",
        vec![
            HkxMember {
                name: "data_old".into(),
                value: HkxValue::F32List(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]),
            },
            HkxMember {
                name: "extras".into(),
                value: HkxValue::F32List(vec![7.0, 8.0]),
            },
            HkxMember {
                name: "data".into(),
                value: HkxValue::F32List(vec![0.0; 8]),
            },
        ],
    );

    let mut manager = PatchManager::with_native_corpus();
    manager.force_corpus_complete();
    manager.register(
        50,
        Patch::new(
            ClassVersion::new("hkAabbHalf", 0),
            ClassVersion::new("hkAabbHalf", 1),
        )
        .with_custom_hook("_hkAabbHalf_0_to_1"),
    );
    manager.convert_hkx(&mut hkx, 49, 50).unwrap();

    let data = hkx.objects()[0]
        .members
        .iter()
        .find(|m| m.name == "data")
        .unwrap();
    assert_eq!(
        data.value,
        HkxValue::F32List(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0])
    );
}

#[test]
fn havok_convert_bytes_general_route_refuses_incomplete_chain() {
    // The packfile-version-patch-chain route returns the parity-gate error when
    // the corpus is incomplete. The TAG0 → FO4 fo76 route is unaffected (it uses
    // fo76.rs heuristics, not the patch chain).
    let data = fixture_bytes(
        "../bacup/py_bacup_lib/python/bacup_lib/tests/fixtures/creatures/deathclaw/expected/character.hkx",
    );
    let result = havok_native::api::havok_convert_bytes(&data, "skyrimse");
    assert!(
        result.is_err(),
        "packfile patch-chain route must surface the corpus-incomplete error \
         rather than silently force-flipping the parity gate (Task 6.3)",
    );
}

// --- Triangle-flip hook tests ---
//
// Python source: py_creation_lib/python/creation_lib/havok_convert/patches/p2013_1/cloth.py:_update_triangle_flips
// Each int32 in old_triangleFlips is expanded to 4 little-endian bytes.

fn make_triangle_flip_hkx(class_name: &str) -> havok_native::hkx::HkxFile {
    use havok_native::hkx::HkxMember;
    use havok_native::hkx::types::HkxValue;
    havok_native::hkx::HkxFile::from_tagxml(
        11,
        "hk_2013.1.0-r1",
        vec![havok_native::hkx::HkxObject {
            name: Some("#0001".into()),
            offset: 0,
            signature: 2,
            class_name: class_name.into(),
            members: vec![
                HkxMember {
                    name: "old_triangleFlips".into(),
                    // Two int32 values: 0x00_00_00_01 and 0x01_02_03_04
                    value: HkxValue::Array(vec![HkxValue::I32(1), HkxValue::I32(0x0102_0304)]),
                },
                HkxMember {
                    name: "triangleFlips".into(),
                    value: HkxValue::Array(vec![]),
                },
            ],
        }],
    )
}

fn run_triangle_flip_hook(hook_name: &str, class_name: &str) {
    use havok_native::hkx::types::HkxValue;

    let mut manager = PatchManager::new();
    manager.register_native_hooks();
    // Register a minimal patch that fires the hook on the relevant class.
    manager.register(
        48,
        Patch::new(
            ClassVersion::new(class_name, 2),
            ClassVersion::new(class_name, 3),
        )
        .with_custom_hook(hook_name),
    );

    let mut hkx = make_triangle_flip_hkx(class_name);
    manager.convert_hkx(&mut hkx, 47, 48).unwrap();

    let object = &hkx.objects()[0];
    let flips = object
        .members
        .iter()
        .find(|m| m.name == "triangleFlips")
        .expect("triangleFlips member should be present");

    // 1 → [0x01, 0x00, 0x00, 0x00] (LE), 0x01020304 → [0x04, 0x03, 0x02, 0x01]
    assert_eq!(
        flips.value,
        HkxValue::Array(vec![
            HkxValue::U8(0x01),
            HkxValue::U8(0x00),
            HkxValue::U8(0x00),
            HkxValue::U8(0x00),
            HkxValue::U8(0x04),
            HkxValue::U8(0x03),
            HkxValue::U8(0x02),
            HkxValue::U8(0x01),
        ])
    );
}

#[test]
fn hcl_update_all_vertex_frames_operator_triangle_flips_int32_to_le_bytes() {
    run_triangle_flip_hook(
        "hclUpdateAllVertexFramesOperator_2_to_3",
        "hclUpdateAllVertexFramesOperator",
    );
}

/// Integration check: convert FO76's `weaponbehavior.hkx` and confirm the EPA
/// population pass produced the three EPAs that the parity test expects.
/// Skipped when `FO76_EXTRACTED_DIR` is unset or the fixture is missing —
/// mirrors `tests/test_fo76_to_fo4_weapon_conversion_parity.py`.
#[test]
fn populate_event_property_arrays_appends_three_epas_on_weapon_behavior() {
    let env_dir = match std::env::var("FO76_EXTRACTED_DIR") {
        Ok(value) if !value.is_empty() => value,
        _ => {
            eprintln!("FO76_EXTRACTED_DIR unset; skipping weaponbehavior EPA check");
            return;
        }
    };
    let path = PathBuf::from(env_dir).join("meshes/actors/character/behaviors/weaponbehavior.hkx");
    if !path.exists() {
        eprintln!("weaponbehavior.hkx missing at {}; skipping", path.display());
        return;
    }

    let data = std::fs::read(&path).expect("read weaponbehavior.hkx");
    let out = api::havok_convert_bytes(&data, "fo4")
        .expect("FO76→FO4 conversion of weaponbehavior.hkx must succeed");
    assert!(!out.is_empty(), "converter produced empty bytes");

    // Reload the produced FO4 packfile and count EPAs. Python's reference
    // implementation outputs 392 hkbStateMachineEventPropertyArray objects
    // for this fixture (389 inherited + 3 synthesized by the EPA pass).
    let hkx = havok_native::hkx::HkxFile::read(&out).expect("re-read produced FO4 packfile");
    let epa_count = hkx
        .objects()
        .iter()
        .filter(|o| o.class_name == "hkbStateMachineEventPropertyArray")
        .count();
    assert_eq!(
        epa_count, 392,
        "EPA count mismatch — Python reference produces 392 EPAs after \
         _populate_event_property_arrays",
    );
}

#[test]
fn manager_refuses_chain_with_unsupported_inverse() {
    let mut manager = PatchManager::new();
    manager.register(
        47,
        Patch::new(
            ClassVersion::new("hkbBody", 0),
            ClassVersion::new("hkbBody", 1),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "uid".to_string(),
            type_name: "uint64".to_string(),
        }),
    );

    let mut hkx = havok_native::hkx::HkxFile::from_tagxml(
        11,
        "hk_2014.1.0-r1",
        vec![havok_native::hkx::HkxObject {
            name: Some("#0001".into()),
            offset: 0,
            signature: 1,
            class_name: "hkbBody".into(),
            members: vec![],
        }],
    );

    let result = manager.convert_hkx(&mut hkx, 47, 46);
    assert!(
        result.is_err(),
        "manager must refuse to run a downgrade chain whose inverse is Unsupported",
    );
}

// --- hknp* schema patches in 2014/2015 corpus ---
//
// Each test below counts the hknp* patch population per corpus file by reading
// the generated source. SDK reference:
// refs/hk2018_1_0_r1/Source/Common/Compat/Patches/<ver>/hknpPatches_<ver>.hxx.
// Thresholds reflect actual SDK-derived counts (2014_2 ships only 9 hknp
// patches at the SDK level).
