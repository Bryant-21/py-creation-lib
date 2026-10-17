use super::skyrim_animation::{
    reconstruct_fo4_graph, validate_graph_preserved, validate_target_descriptor_signatures,
};
use crate::error::{HavokError, HavokResult};
use crate::hkx::HkxFile;
use crate::hkx::model::HkxMember;
use crate::hkx::types::HkxValue;

pub fn reemit_skyrim_fx_asset_to_fo4(data: &[u8]) -> HavokResult<Vec<u8>> {
    let source = HkxFile::read(data)?;
    if source.contents_version() == "hk_2014.1.0-r1" {
        return Ok(data.to_vec());
    }
    if source.class_version() != 8
        || source.contents_version() != "hk_2010.2.0-r1"
        || source.packfile().header.pointer_size != 8
    {
        return Err(HavokError::InvalidInput(
            "FX conversion requires a 64-bit Skyrim 2010 packfile".into(),
        ));
    }
    if source.objects().iter().any(|object| {
        matches!(
            object.class_name.as_str(),
            "hkaSkeleton" | "hkaSplineCompressedAnimation"
        )
    }) {
        return super::skyrim_animation::reemit_skyrim_2010_animation_asset_to_fo4(data);
    }
    let effect_asset = source.objects().iter().any(|object| {
        matches!(
            object.class_name.as_str(),
            "hkbProjectStringData"
                | "hkbCharacterStringData"
                | "BGSGamebryoSequenceGenerator"
                | "hkbClipGenerator"
        )
    });
    if !effect_asset {
        return Err(HavokError::InvalidInput(
            "FX asset has no project, character, clip or NIF sequence generator".into(),
        ));
    }
    for object in source.objects() {
        if object.class_name == "BGSGamebryoSequenceGenerator" {
            validate_sequence_generator(object)?;
        }
        if !matches!(
            object.class_name.as_str(),
            "hkRootLevelContainer"
                | "hkbProjectData"
                | "hkbProjectStringData"
                | "hkbCharacterData"
                | "hkbCharacterStringData"
                | "hkbMirroredSkeletonInfo"
                | "hkbVariableValueSet"
                | "hkbBehaviorGraph"
                | "hkbBehaviorGraphData"
                | "hkbBehaviorGraphStringData"
                | "hkbStateMachine"
                | "hkbStateMachineStateInfo"
                | "hkbStateMachineTransitionInfoArray"
                | "hkbBlendingTransitionEffect"
                | "BGSGamebryoSequenceGenerator"
                | "hkbClipGenerator"
                | "hkbClipTriggerArray"
                | "hkbBlenderGenerator"
                | "hkbBlenderGeneratorChild"
                | "hkbModifierGenerator"
                | "hkbModifierList"
                | "hkbDampingModifier"
                | "hkbVariableBindingSet"
                | "hkbExpressionCondition"
                | "hkbEvaluateExpressionModifier"
                | "hkbExpressionDataArray"
                | "hkbEventPropertyArray"
                | "hkbReferencePoseGenerator"
                | "hkbBehaviorReferenceGenerator"
                | "hkbTimerModifier"
                | "BSTimerModifier"
                | "BSAssignVariablesModifier"
                | "hkbStringEventPayload"
        ) {
            return Err(HavokError::InvalidInput(format!(
                "FX conversion has unsupported source class {}",
                object.class_name
            )));
        }
    }
    let mut normalized = source.clone();
    for object in normalized.objects_mut() {
        if object.class_name == "hkbCharacterData" {
            for member in &mut object.members {
                if member.name == "characterControllerInfo" {
                    member.value = controller_setup(&member.value)?;
                    member.name = "characterControllerSetup".into();
                }
            }
        }
    }
    let target = reconstruct_fo4_graph(&normalized)?;
    validate_target_descriptor_signatures(&target)?;
    let bytes = target.save();
    let reread = HkxFile::read(&bytes)?;
    validate_target_descriptor_signatures(&reread)?;
    validate_graph_preserved(&normalized, &reread, "FO4 FX graph")?;
    Ok(bytes)
}

fn validate_sequence_generator(object: &crate::hkx::model::HkxObject) -> HavokResult<()> {
    let value = |name: &str| {
        object
            .members
            .iter()
            .find(|member| member.name == name)
            .map(|member| &member.value)
    };
    let integer = |name: &str| match value(name) {
        Some(HkxValue::I8(value)) => Some(i32::from(*value)),
        Some(HkxValue::U8(value)) => Some(i32::from(*value)),
        Some(HkxValue::I32(value)) => Some(*value),
        _ => None,
    };
    if !matches!(value("pSequence"), Some(HkxValue::String { value, is_null: false }) if !value.is_empty())
        || !matches!(integer("eBlendModeFunction"), Some(0..=2))
        || !matches!(integer("eUseTimePercentage"), Some(0..=1))
        || !matches!(value("fPercent"), Some(HkxValue::F32(value)) if value.is_finite())
        || !matches!(value("fTimePercent"), Some(HkxValue::F32(value)) if value.is_finite())
    {
        return Err(HavokError::InvalidInput(format!(
            "FX sequence generator {:?} has invalid sequence/blend semantics",
            object.name
        )));
    }
    Ok(())
}

fn controller_setup(source: &HkxValue) -> HavokResult<HkxValue> {
    let members = source.as_object_members().ok_or_else(|| {
        HavokError::InvalidInput("FX characterControllerInfo is not an inline struct".into())
    })?;
    let value = |name: &str| -> HavokResult<HkxValue> {
        members
            .iter()
            .find(|member| member.name == name)
            .map(|member| member.value.clone())
            .ok_or_else(|| {
                HavokError::InvalidInput(format!("FX characterControllerInfo lacks {name}"))
            })
    };
    if members.len() != 4 || value("characterControllerCinfo")? != HkxValue::Pointer(None) {
        return Err(HavokError::InvalidInput(
            "FX character controller has unsupported fields or non-null legacy physics cinfo"
                .into(),
        ));
    }
    let member = |name: &str, value| HkxMember {
        name: name.into(),
        value,
    };
    // hkbCharacterControllerSetup's FO4 constructor (official PDB, retail
    // 1.10.155 RVA 0x198de20) uses INVALID (-1), CAPSULE (0) and null cinfo.
    // The 9->10 patch replaces the old inline struct; its four source fields
    // map to the nested setup without inventing a physical body for an FX rig.
    Ok(HkxValue::Object(vec![
        member(
            "rigidBodySetup",
            HkxValue::Object(vec![
                member("collisionFilterInfo", value("collisionFilterInfo")?),
                member("type", HkxValue::I8(-1)),
                member(
                    "shapeSetup",
                    HkxValue::Object(vec![
                        member("capsuleHeight", value("capsuleHeight")?),
                        member("capsuleRadius", value("capsuleRadius")?),
                        member(
                            "fileName",
                            HkxValue::String {
                                value: String::new(),
                                is_null: true,
                            },
                        ),
                        member("type", HkxValue::I8(0)),
                    ]),
                ),
            ]),
        ),
        member("controllerCinfo", value("characterControllerCinfo")?),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_fx_controller_preserves_capsule_and_filter_and_rejects_physics() {
        let mut source = HkxValue::Object(vec![
            HkxMember {
                name: "capsuleHeight".into(),
                value: HkxValue::F32(3.25),
            },
            HkxMember {
                name: "capsuleRadius".into(),
                value: HkxValue::F32(0.75),
            },
            HkxMember {
                name: "collisionFilterInfo".into(),
                value: HkxValue::U32(0x12345678),
            },
            HkxMember {
                name: "characterControllerCinfo".into(),
                value: HkxValue::Pointer(None),
            },
        ]);
        let target = controller_setup(&source).unwrap();
        let members = target.as_object_members().unwrap();
        let body = members[0].value.as_object_members().unwrap();
        assert_eq!(body[0].value, HkxValue::U32(0x12345678));
        assert_eq!(body[1].value, HkxValue::I8(-1));
        let shape = body[2].value.as_object_members().unwrap();
        assert_eq!(shape[0].value, HkxValue::F32(3.25));
        assert_eq!(shape[1].value, HkxValue::F32(0.75));
        let HkxValue::Object(ref mut fields) = source else {
            unreachable!()
        };
        fields[3].value = HkxValue::Pointer(Some(7));
        assert!(controller_setup(&source).is_err());
    }

    #[test]
    #[ignore = "requires extracted Skyrim FX assets"]
    fn source_creature_fx_graphs_preserve_events_sequences_and_variables() {
        let root = std::env::var_os("MODKIT_SKYRIM_MESH_ROOT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../../extracted/skyrimse/Meshes")
            });
        for relative in [
            "uniquebehaviors/sprigganfx/sprigganfx01.hkx",
            "uniquebehaviors/sprigganfx/characters/sprigganfx01character.hkx",
            "uniquebehaviors/sprigganfx/characterassets/singleboneskeleton.hkx",
            "uniquebehaviors/sprigganfx/behaviors/sprigganfx01behavior.hkx",
            "uniquebehaviors/dragonskin/behaviors/dragonskin.hkx",
            "uniquebehaviors/dragonskin/characters/dragonskin.hkx",
            "dlc01/sharedbehaviors/blackreachcreatures/behaviors/behavior.hkx",
            "genericbehaviors/blendbetweenstatesvariable/behaviors/behavior.hkx",
            "genericbehaviors/behaviors/autoplay/autoplaybehavior.hkx",
            "genericbehaviors/stagesnoloops/behaviors/behavior.hkx",
        ] {
            let source = std::fs::read(root.join(relative))
                .unwrap_or_else(|error| panic!("{relative}: {error}"));
            let bytes = reemit_skyrim_fx_asset_to_fo4(&source)
                .unwrap_or_else(|error| panic!("{relative}: {error}"));
            let target = HkxFile::read(&bytes).unwrap();
            assert_eq!(target.contents_version(), "hk_2014.1.0-r1");
            assert_eq!(target.packfile().header.pointer_size, 8);
            assert_eq!(target.save(), bytes);
            if relative == "uniquebehaviors/dragonskin/characters/dragonskin.hkx" {
                let strings = target
                    .objects()
                    .iter()
                    .find(|object| object.class_name == "hkbCharacterStringData")
                    .unwrap();
                for (field, expected) in [
                    ("name", "DragonSkin"),
                    ("rigName", "CharacterAssets\\SingleBoneSkeleton.hkx"),
                    ("behaviorFilename", "Behaviors\\DragonSkin.hkx"),
                ] {
                    assert_eq!(
                        strings
                            .members
                            .iter()
                            .find(|member| member.name == field)
                            .unwrap()
                            .value,
                        HkxValue::String {
                            value: expected.into(),
                            is_null: false
                        }
                    );
                }
                for field in [
                    "luaScriptOnCharacterActivated",
                    "luaScriptOnCharacterDeactivated",
                ] {
                    assert_eq!(
                        strings
                            .members
                            .iter()
                            .find(|member| member.name == field)
                            .unwrap()
                            .value,
                        HkxValue::String {
                            value: String::new(),
                            is_null: true
                        }
                    );
                }
                assert_eq!(
                    strings
                        .members
                        .iter()
                        .find(|member| member.name == "luaFiles")
                        .unwrap()
                        .value,
                    HkxValue::Array(vec![])
                );
            }
            if relative == "genericbehaviors/blendbetweenstatesvariable/behaviors/behavior.hkx" {
                let damping = target
                    .objects()
                    .iter()
                    .find(|object| object.class_name == "hkbDampingModifier")
                    .unwrap();
                for (name, expected) in [
                    ("enable", HkxValue::Bool(true)),
                    ("kP", HkxValue::F32(0.2)),
                    ("enableScalarDamping", HkxValue::Bool(true)),
                    ("enableVectorDamping", HkxValue::Bool(false)),
                ] {
                    assert_eq!(
                        damping
                            .members
                            .iter()
                            .find(|member| member.name == name)
                            .unwrap()
                            .value,
                        expected,
                        "{name}"
                    );
                }
            }
            if relative.ends_with("sprigganfx01behavior.hkx") {
                let sequences: std::collections::BTreeSet<_> = target
                    .objects()
                    .iter()
                    .filter(|object| object.class_name == "BGSGamebryoSequenceGenerator")
                    .map(|object| {
                        validate_sequence_generator(object).unwrap();
                        assert_eq!(
                            object
                                .members
                                .iter()
                                .find(|member| member.name == "fPercent")
                                .unwrap()
                                .value,
                            HkxValue::F32(1.0)
                        );
                        let member = object
                            .members
                            .iter()
                            .find(|member| member.name == "pSequence")
                            .unwrap();
                        let HkxValue::String { value, .. } = &member.value else {
                            panic!("missing sequence name")
                        };
                        value.as_str()
                    })
                    .collect();
                assert_eq!(
                    sequences,
                    std::collections::BTreeSet::from([
                        "LeavesOffHandDark",
                        "LeavesAwayLoop",
                        "LeavesToHandDark",
                        "LeavesOnHandDarkLoop",
                        "LeavesLanding",
                        "LeavesScared",
                        "KillFX",
                        "LeavesOffHand",
                        "LeavesLandedLoop",
                        "LeavesToHand",
                        "LeavesOnHandLoop",
                    ])
                );
            }
        }
    }
}
