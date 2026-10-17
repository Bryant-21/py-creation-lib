use super::*;

fn member<'a>(members: &'a [HkxMember], name: &str) -> &'a HkxValue {
    &members
        .iter()
        .find(|member| member.name == name)
        .unwrap()
        .value
}

fn named<'a>(objects: &'a [HkxObject], name: &str) -> &'a HkxObject {
    objects
        .iter()
        .find(|object| {
            object.members.iter().any(|entry| {
                entry.name == "name"
                    && matches!(&entry.value, HkxValue::String { value, .. } if value == name)
            })
        })
        .unwrap()
}

fn pointed<'a>(objects: &'a [HkxObject], value: &HkxValue) -> &'a HkxObject {
    let HkxValue::Pointer(Some(index)) = value else {
        panic!("missing recovery graph pointer: {value:?}")
    };
    &objects[*index]
}

#[test]
fn recovery_layouts_are_gated_by_source_version_and_signature() {
    for (class, signature, field, source_offset, target_offset, source_value, target_value) in [
        (
            "hkbPoseMatchingGenerator",
            SKYRIM_POSE_MATCHING_GENERATOR_SIGNATURE,
            "blendSpeed",
            176,
            240,
            HkxValue::F32(1.25),
            HkxValue::F32(2.5),
        ),
        (
            "hkbGetUpModifier",
            SKYRIM_GET_UP_MODIFIER_SIGNATURE,
            "duration",
            96,
            112,
            HkxValue::F32(1.25),
            HkxValue::F32(2.5),
        ),
        (
            "hkbEventDrivenModifier",
            SKYRIM_EVENT_DRIVEN_MODIFIER_SIGNATURE,
            "activateEventId",
            88,
            96,
            HkxValue::I32(42),
            HkxValue::I32(77),
        ),
        (
            "hkbPoweredRagdollControlsModifier",
            SKYRIM_POWERED_RAGDOLL_CONTROLS_SIGNATURE,
            "bones",
            112,
            128,
            HkxValue::Pointer(Some(1)),
            HkxValue::Pointer(Some(2)),
        ),
        (
            "hkbRigidBodyRagdollControlsModifier",
            SKYRIM_RIGID_BODY_RAGDOLL_CONTROLS_SIGNATURE,
            "bones",
            144,
            160,
            HkxValue::Pointer(Some(1)),
            HkxValue::Pointer(Some(2)),
        ),
        (
            "hkbKeyframeBonesModifier",
            SKYRIM_KEYFRAME_BONES_MODIFIER_SIGNATURE,
            "keyframedBonesList",
            96,
            104,
            HkxValue::Pointer(Some(1)),
            HkxValue::Pointer(Some(2)),
        ),
    ] {
        let mut data = vec![0u8; 512];
        let mut global_fixups = HashMap::new();
        for (offset, value) in [
            (source_offset, &source_value),
            (target_offset, &target_value),
        ] {
            match value {
                HkxValue::F32(value) => {
                    data[offset..offset + 4].copy_from_slice(&value.to_le_bytes())
                }
                HkxValue::I32(value) => {
                    data[offset..offset + 4].copy_from_slice(&value.to_le_bytes())
                }
                HkxValue::Pointer(Some(index)) => {
                    global_fixups.insert(offset, 256 + index * 64);
                }
                _ => unreachable!(),
            }
        }
        if class.contains("RagdollControls") {
            let offset = if class == "hkbPoweredRagdollControlsModifier" {
                152
            } else {
                168
            };
            data[offset..offset + 4].copy_from_slice(&0.75f32.to_le_bytes());
        }
        let section = SectionHeader {
            name: "__data__".into(),
            offset: 0,
            data1: data.len(),
            data2: data.len(),
            data3: data.len(),
            exports: data.len(),
            imports: data.len(),
            end: data.len(),
        };
        let empty = HashMap::new();
        let object_indices = HashMap::from([(320, 1), (384, 2)]);
        for (version, actual_signature, source) in [
            ("hk_2010.2.0-r1", signature, true),
            ("hk_2010.2.0-r1", signature ^ 1, false),
            ("hk_2014.1.0-r1", signature, false),
        ] {
            let signatures = HashMap::from([(0, actual_signature)]);
            let context = ReadContext {
                data: &data,
                data_section: &section,
                contents_version: version,
                local_fixups: &empty,
                global_fixups: &global_fixups,
                object_index_by_offset: &object_indices,
                object_signature_by_offset: &signatures,
            };
            let values = read_members(
                &mut DescriptorRegistry::for_contents_version(version),
                &context,
                0,
                class,
                0,
                &mut Vec::new(),
                &mut Vec::new(),
            )
            .unwrap();
            assert_eq!(
                member(&values, field),
                if source { &source_value } else { &target_value },
                "{class} {version} {actual_signature:x}"
            );
            if class.contains("RagdollControls") {
                assert_eq!(
                    member(&values, "animationBlendFraction"),
                    &HkxValue::F32(if source { 0.0 } else { 0.75 })
                );
            }
        }
    }
}

#[test]
#[ignore = "requires the extracted Skyrim hagraven behavior"]
fn skyrim_hagraven_recovery_matches_source_fields_and_fixups() {
    let root = std::env::var_os("MODKIT_REPO_ROOT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.."));
    let mut bytes = std::fs::read(
        root.join("extracted/skyrimse/meshes/actors/hagraven/behaviors/havgravenbehavior.hkx"),
    )
    .unwrap();
    let file = crate::hkx::HkxFile::read(&bytes).unwrap();
    let objects = file.objects();
    for (name, other, another) in [
        ("Reanimate Pose Matcher", 13, 12),
        ("Get Up Pose Matcher", 11, 10),
    ] {
        let pose = named(objects, name);
        assert_eq!(pose.signature, SKYRIM_POSE_MATCHING_GENERATOR_SIGNATURE);
        assert_eq!(
            member(&pose.members, "worldFromModelRotation"),
            &HkxValue::F32List(vec![0.0, 0.0, 0.0, 1.0])
        );
        for (field, expected) in [
            ("blendSpeed", 1.0),
            ("minSpeedToSwitch", 0.2),
            ("minSwitchTimeNoError", 0.2),
            ("minSwitchTimeFullError", 0.0),
            ("maxCyclicBlendParameter", 1.0),
        ] {
            assert_eq!(
                member(&pose.members, field),
                &HkxValue::F32(expected),
                "{name} {field}"
            );
        }
        for (field, expected) in [("startPlayingEventId", 61), ("startMatchingEventId", 59)] {
            assert_eq!(member(&pose.members, field), &HkxValue::I32(expected));
        }
        for (field, expected) in [
            ("rootBoneIndex", 0),
            ("otherBoneIndex", other),
            ("anotherBoneIndex", another),
            ("pelvisIndex", 0),
            ("indexOfSyncMasterChild", -1),
        ] {
            assert_eq!(member(&pose.members, field), &HkxValue::I16(expected));
        }
        assert_eq!(member(&pose.members, "mode"), &HkxValue::I8(0));
        let HkxValue::Array(children) = member(&pose.members, "children") else {
            panic!("missing pose children")
        };
        assert_eq!(children.len(), 2);
        for child in children {
            assert_eq!(
                pointed(objects, child).class_name,
                "hkbBlenderGeneratorChild"
            );
        }
        assert_eq!(
            f32::from_le_bytes(
                bytes[pose.offset + 176..pose.offset + 180]
                    .try_into()
                    .unwrap()
            ),
            1.0
        );
    }

    let get_up = named(objects, "Get Up Modifier");
    assert_eq!(member(&get_up.members, "enable"), &HkxValue::Bool(true));
    assert_eq!(
        member(&get_up.members, "groundNormal"),
        &HkxValue::F32List(vec![0.0, 0.0, 1.0, 0.0])
    );
    assert_eq!(member(&get_up.members, "duration"), &HkxValue::F32(1.0));
    assert_eq!(
        member(&get_up.members, "alignWithGroundDuration"),
        &HkxValue::F32(0.25)
    );
    for (field, expected) in [
        ("rootBoneIndex", 0),
        ("otherBoneIndex", 13),
        ("anotherBoneIndex", 12),
    ] {
        assert_eq!(member(&get_up.members, field), &HkxValue::I16(expected));
    }

    for (name, force, mode) in [
        ("PoweredRagdoll No Matching", 0.0, 4),
        ("PoweredRagdollMatching", 200.0, 2),
    ] {
        let powered = named(objects, name);
        assert_eq!(member(&powered.members, "enable"), &HkxValue::Bool(true));
        assert_eq!(
            pointed(objects, member(&powered.members, "bones")).class_name,
            "hkbBoneIndexArray"
        );
        assert_eq!(
            pointed(objects, member(&powered.members, "boneWeights")).class_name,
            "hkbBoneWeightArray"
        );
        let control = member(&powered.members, "controlData")
            .as_object_members()
            .unwrap();
        for (field, expected) in [
            ("maxForce", force),
            ("tau", 0.8),
            ("damping", 1.0),
            ("proportionalRecoveryVelocity", 2.0),
            ("constantRecoveryVelocity", 1.0),
        ] {
            assert_eq!(member(control, field), &HkxValue::F32(expected));
        }
        let world = member(&powered.members, "worldFromModelModeData")
            .as_object_members()
            .unwrap();
        for (field, expected) in [
            ("poseMatchingBone0", 0),
            ("poseMatchingBone1", 11),
            ("poseMatchingBone2", 10),
        ] {
            assert_eq!(member(world, field), &HkxValue::I16(expected));
        }
        assert_eq!(member(world, "mode"), &HkxValue::I8(mode));
        assert_eq!(
            member(&powered.members, "animationBlendFraction"),
            &HkxValue::F32(0.0)
        );
    }

    for (name, activate, deactivate, active, child_class) in [
        (
            "FullRagdoll",
            -1,
            62,
            true,
            "hkbPoweredRagdollControlsModifier",
        ),
        ("TurnOnMatchingRagdoll", 62, -1, false, "hkbModifierList"),
    ] {
        let driven = named(objects, name);
        assert_eq!(member(&driven.members, "enable"), &HkxValue::Bool(true));
        assert_eq!(
            pointed(objects, member(&driven.members, "modifier")).class_name,
            child_class
        );
        assert_eq!(
            member(&driven.members, "activateEventId"),
            &HkxValue::I32(activate)
        );
        assert_eq!(
            member(&driven.members, "deactivateEventId"),
            &HkxValue::I32(deactivate)
        );
        assert_eq!(
            member(&driven.members, "activeByDefault"),
            &HkxValue::Bool(active)
        );
    }
    for name in ["KeyframeLowerBody", "KeyframeFullRagdoll"] {
        let keyframe = named(objects, name);
        assert_eq!(member(&keyframe.members, "enable"), &HkxValue::Bool(true));
        assert_eq!(
            member(&keyframe.members, "keyframeInfo"),
            &HkxValue::Array(Vec::new())
        );
        assert_eq!(
            pointed(objects, member(&keyframe.members, "keyframedBonesList")).class_name,
            "hkbBoneIndexArray"
        );
    }
    let timer = named(objects, "GetUpTimerMod");
    assert_eq!(member(&timer.members, "enable"), &HkxValue::Bool(true));
    assert_eq!(
        member(&timer.members, "alarmTimeSeconds"),
        &HkxValue::F32(0.5)
    );
    assert_eq!(
        member(
            member(&timer.members, "alarmEvent")
                .as_object_members()
                .unwrap(),
            "id"
        ),
        &HkxValue::I32(61)
    );
    let rigid = named(objects, "DriveRagdollRB");
    assert_eq!(
        pointed(objects, member(&rigid.members, "bones")).class_name,
        "hkbBoneIndexArray"
    );
    assert_eq!(
        member(&rigid.members, "animationBlendFraction"),
        &HkxValue::F32(0.0)
    );
    let control = member(&rigid.members, "controlData")
        .as_object_members()
        .unwrap();
    assert_eq!(member(control, "durationToBlend"), &HkxValue::F32(0.5));
    let keyframe = member(control, "keyFrameControlData")
        .as_object_members()
        .unwrap();
    assert_eq!(member(keyframe, "hierarchyGain"), &HkxValue::F32(0.17));
    assert_eq!(
        member(keyframe, "positionMaxAngularVelocity"),
        &HkxValue::F32(1.8)
    );

    let pose = named(objects, "Get Up Pose Matcher");
    bytes[pose.offset + 176..pose.offset + 180].copy_from_slice(&3.25f32.to_le_bytes());
    let changed = crate::hkx::HkxFile::read(&bytes).unwrap();
    assert_eq!(
        member(
            &named(changed.objects(), "Get Up Pose Matcher").members,
            "blendSpeed"
        ),
        &HkxValue::F32(3.25)
    );
}
