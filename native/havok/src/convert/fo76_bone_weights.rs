use super::*;
use std::collections::HashSet;

pub(super) fn lower(hkx: &mut HkxFile, delegated: &HashSet<usize>) {
    let wrappers: Vec<_> = hkx
        .objects()
        .iter()
        .enumerate()
        .filter_map(|(index, object)| {
            (object.class_name == "hkbModifierGenerator").then_some(index)
        })
        .collect();
    for wrapper in wrappers {
        if delegated.contains(&wrapper) {
            continue;
        }
        let object = &hkx.objects()[wrapper];
        let Some(modifier) = pointer_member_value(&object.members, "modifier") else {
            continue;
        };
        let Some(child) = pointer_member_value(&object.members, "generator") else {
            continue;
        };
        let list = hkx.objects()[modifier].class_name == "hkbModifierList";
        let modifiers = if list {
            pointer_array_member_values(&hkx.objects()[modifier].members, "modifiers")
                .unwrap_or_default()
        } else {
            vec![modifier]
        };
        let original_count = hkx.objects().len();
        let base = if list { wrapper } else { child };
        let mut masked = base;
        for modifier in modifiers {
            if hkx.objects()[modifier].class_name == "BSAssignBoneWeightsModifier" {
                masked = mask_generator(hkx, modifier, masked);
            }
        }
        if masked != base {
            if list {
                let mapping = std::collections::HashMap::from([(wrapper, masked)]);
                for object in &mut hkx.objects_mut()[..original_count] {
                    for member in &mut object.members {
                        remap_pointers(&mut member.value, &mapping);
                    }
                }
            } else {
                hkx.objects_mut()[wrapper]
                    .members
                    .iter_mut()
                    .find(|m| m.name == "generator")
                    .unwrap()
                    .value = HkxValue::Pointer(Some(masked));
            }
        }
    }
}

fn binding(hkx: &HkxFile, modifier: usize, member: &str) -> Option<Vec<HkxMember>> {
    let set = pointer_member_value(&hkx.objects()[modifier].members, "variableBindingSet")?;
    let HkxValue::Array(entries) = &hkx.objects()[set]
        .members
        .iter()
        .find(|m| m.name == "bindings")?
        .value
    else {
        return None;
    };
    entries.iter().find_map(|entry| {
        let fields = entry.as_object_members()?;
        (string_member_value(fields, "memberPath") == Some(member)).then(|| fields.to_vec())
    })
}

fn copy_binding(hkx: &mut HkxFile, mut fields: Vec<HkxMember>, member: &str) -> usize {
    fields
        .iter_mut()
        .find(|m| m.name == "memberPath")
        .unwrap()
        .value = string_hkx_value(member);
    push_named_object(
        hkx,
        "hkbVariableBindingSet",
        2,
        vec![
            member_value("bindings", HkxValue::Array(vec![HkxValue::Object(fields)])),
            member_value("indexOfBindingToEnable", HkxValue::I32(-1)),
        ],
    )
}

fn mask_generator(hkx: &mut HkxFile, modifier: usize, child: usize) -> usize {
    let mut choices = vec![child];
    for number in 1..=2 {
        let member = format!("boneWeights{number}");
        let weights = pointer_member_value(&hkx.objects()[modifier].members, &member);
        let bound =
            binding(hkx, modifier, &member).map(|fields| copy_binding(hkx, fields, "boneWeights"));
        if weights.is_none() && bound.is_none() {
            choices.push(child);
            continue;
        }
        let blend_child = push_named_object(
            hkx,
            "hkbBlenderGeneratorChild",
            2,
            vec![
                member_value("variableBindingSet", HkxValue::Pointer(bound)),
                member_value("generator", HkxValue::Pointer(Some(child))),
                member_value("boneWeights", HkxValue::Pointer(weights)),
                member_value("weight", HkxValue::F32(1.0)),
                member_value("worldFromModelWeight", HkxValue::F32(1.0)),
            ],
        );
        choices.push(push_named_object(
            hkx,
            "hkbBlenderGenerator",
            1,
            vec![
                member_value("variableBindingSet", HkxValue::Pointer(None)),
                member_value(
                    "name",
                    string_hkx_value(&format!("FO76_BoneMask_{modifier}_{number}")),
                ),
                member_value(
                    "children",
                    HkxValue::Array(vec![HkxValue::Pointer(Some(blend_child))]),
                ),
                member_value("referencePoseWeightThreshold", HkxValue::F32(0.0)),
                member_value("blendParameter", HkxValue::F32(1.0)),
                member_value("indexOfSyncMasterChild", HkxValue::I16(-1)),
                member_value("flags", HkxValue::I16(0)),
            ],
        ));
    }
    let selected = hkx.objects()[modifier]
        .members
        .iter()
        .find(|m| m.name == "boneWeightActionVar")
        .and_then(|m| extract_int(&m.value))
        .unwrap_or(0);
    if let Some(mut fields) = binding(hkx, modifier, "boneWeightActionVar") {
        let source_index = fields
            .iter()
            .find(|m| m.name == "variableIndex")
            .and_then(|m| extract_int(&m.value));
        let variable_binding = fields
            .iter()
            .find(|m| m.name == "bindingType")
            .and_then(|m| extract_int(&m.value))
            == Some(0);
        if variable_binding
            && source_index.is_some()
            && source_index == behavior_variable_index(hkx, "iBoneWeightsAction")
        {
            // FO4 never writes FO76's mask-action input. Its locomotion input selects
            // the unmasked stationary pose (0) or the authored moving-body mask (1).
            if let Some(index) = behavior_variable_index(hkx, "iSyncIdleLocomotion") {
                fields
                    .iter_mut()
                    .find(|m| m.name == "variableIndex")
                    .unwrap()
                    .value = HkxValue::I32(index);
            }
        }
        let bound = copy_binding(hkx, fields, "selectedGeneratorIndex");
        push_named_object(
            hkx,
            "hkbManualSelectorGenerator",
            3,
            vec![
                member_value("variableBindingSet", HkxValue::Pointer(Some(bound))),
                member_value(
                    "name",
                    string_hkx_value(&format!("FO76_BoneMaskSelector_{modifier}")),
                ),
                member_value(
                    "generators",
                    HkxValue::Array(
                        choices
                            .into_iter()
                            .map(|i| HkxValue::Pointer(Some(i)))
                            .collect(),
                    ),
                ),
                member_value("selectedGeneratorIndex", HkxValue::I16(selected as i16)),
                member_value("selectedIndexCanChangeAfterActivate", HkxValue::Bool(true)),
            ],
        )
    } else {
        choices.get(selected as usize).copied().unwrap_or(child)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(list: bool) -> HkxFile {
        let object = |class: &str, members| HkxObject {
            name: None,
            offset: 0,
            signature: 0,
            class_name: class.into(),
            members,
        };
        HkxFile::from_tagxml(
            11,
            "hk_2014.1.0-r1",
            vec![
                object(
                    "hkbStateMachineStateInfo",
                    vec![member_value("generator", HkxValue::Pointer(Some(1)))],
                ),
                object(
                    "hkbModifierGenerator",
                    vec![
                        member_value(
                            "modifier",
                            HkxValue::Pointer(Some(if list { 4 } else { 2 })),
                        ),
                        member_value("generator", HkxValue::Pointer(Some(3))),
                    ],
                ),
                object(
                    "BSAssignBoneWeightsModifier",
                    vec![
                        member_value("boneWeights1", HkxValue::Pointer(Some(5))),
                        member_value("boneWeightActionVar", HkxValue::I32(1)),
                    ],
                ),
                object("hkbClipGenerator", vec![]),
                object(
                    "hkbModifierList",
                    vec![member_value(
                        "modifiers",
                        HkxValue::Array(vec![HkxValue::Pointer(Some(2))]),
                    )],
                ),
                object(
                    "hkbBoneWeightArray",
                    vec![member_value(
                        "boneWeights",
                        HkxValue::F32List(vec![0.0, 1.0]),
                    )],
                ),
            ],
        )
    }
    #[test]
    fn moving_reload_selects_its_mask_from_the_fo4_locomotion_input() {
        let mut file = fixture(false);
        push_named_object(
            &mut file,
            "hkbBehaviorGraphStringData",
            1,
            vec![member_value(
                "variableNames",
                HkxValue::Array(vec![
                    string_hkx_value("iBoneWeightsAction"),
                    string_hkx_value("iSyncIdleLocomotion"),
                ]),
            )],
        );
        let source_binding = create_binding_set(&mut file, "boneWeightActionVar", 0);
        file.objects_mut()[2].members.push(member_value(
            "variableBindingSet",
            HkxValue::Pointer(Some(source_binding)),
        ));
        lower(&mut file, &HashSet::new());
        let selector = file
            .objects()
            .iter()
            .find(|o| o.class_name == "hkbManualSelectorGenerator")
            .unwrap();
        let selected = pointer_member_value(&selector.members, "variableBindingSet").unwrap();
        assert_eq!(
            binding_variable_index(&file, selected, "selectedGeneratorIndex"),
            Some(1)
        );
        assert_eq!(
            binding_variable_index(&file, source_binding, "boneWeightActionVar"),
            Some(0)
        );
        let choices = pointer_array_member_values(&selector.members, "generators").unwrap();
        assert_eq!(choices[0], 3);
        let mask = &file.objects()[choices[1]];
        let child = pointer_array_member_values(&mask.members, "children").unwrap()[0];
        assert_eq!(
            pointer_member_value(&file.objects()[child].members, "generator"),
            Some(3)
        );
        assert_eq!(
            pointer_member_value(&file.objects()[child].members, "boneWeights"),
            Some(5)
        );
    }
    #[test]
    fn missing_locomotion_input_and_unrelated_action_bindings_are_preserved() {
        for names in [
            vec!["iBoneWeightsAction"],
            vec!["OtherAction", "iSyncIdleLocomotion"],
        ] {
            let mut file = fixture(false);
            push_named_object(
                &mut file,
                "hkbBehaviorGraphStringData",
                1,
                vec![member_value(
                    "variableNames",
                    HkxValue::Array(names.into_iter().map(string_hkx_value).collect()),
                )],
            );
            let set = create_binding_set(&mut file, "boneWeightActionVar", 0);
            file.objects_mut()[2].members.push(member_value(
                "variableBindingSet",
                HkxValue::Pointer(Some(set)),
            ));
            lower(&mut file, &HashSet::new());
            let selector = file
                .objects()
                .iter()
                .find(|o| o.class_name == "hkbManualSelectorGenerator")
                .unwrap();
            let set = pointer_member_value(&selector.members, "variableBindingSet").unwrap();
            assert_eq!(
                binding_variable_index(&file, set, "selectedGeneratorIndex"),
                Some(0)
            );
        }
    }

    #[test]
    #[ignore = "requires B21_RELOAD_TEST_GUN pointing to extracted FO76 gunbehavior.hkx"]
    fn extracted_gun_masks_use_locomotion_after_conversion_and_pack_roundtrip() {
        let path = std::env::var("B21_RELOAD_TEST_GUN").unwrap();
        let source = HkxFile::read(&std::fs::read(path).unwrap()).unwrap();
        let clips = |file: &HkxFile| -> Vec<String> {
            file.objects()
                .iter()
                .filter(|o| o.class_name == "hkbClipGenerator")
                .map(|o| {
                    string_member_value(&o.members, "animationName")
                        .unwrap()
                        .to_owned()
                })
                .collect()
        };
        let original_clips = clips(&source);
        let result =
            migrate_2015_packfile_to_2014_with_warnings(source, Default::default()).unwrap();
        let roundtrip = HkxFile::read(&result.hkx.save()).unwrap();
        assert_eq!(clips(&roundtrip), original_clips);
        let locomotion = behavior_variable_index(&roundtrip, "iSyncIdleLocomotion").unwrap();
        let mut selectors = 0;
        for object in roundtrip.objects() {
            if object.class_name != "hkbManualSelectorGenerator"
                || !string_member_value(&object.members, "name")
                    .unwrap_or("")
                    .starts_with("FO76_BoneMaskSelector_")
            {
                continue;
            }
            selectors += 1;
            let set = pointer_member_value(&object.members, "variableBindingSet").unwrap();
            assert_eq!(
                binding_variable_index(&roundtrip, set, "selectedGeneratorIndex"),
                Some(locomotion)
            );
            let choices = pointer_array_member_values(&object.members, "generators").unwrap();
            assert_eq!(choices.len(), 3);
            for mask in &choices[1..] {
                let child =
                    pointer_array_member_values(&roundtrip.objects()[*mask].members, "children")
                        .unwrap()[0];
                assert_eq!(
                    pointer_member_value(&roundtrip.objects()[child].members, "generator"),
                    Some(choices[0])
                );
            }
        }
        assert!(
            selectors >= 2,
            "standing and sneaking reload selectors must survive"
        );
        println!(
            "{selectors} converted mask selectors; {} original clips retained",
            original_clips.len()
        );
    }

    #[test]
    fn preserves_static_masks_on_direct_and_list_wrappers() {
        for list in [false, true] {
            let mut file = fixture(list);
            lower(&mut file, &HashSet::new());
            let blend = file
                .objects()
                .iter()
                .position(|o| o.class_name == "hkbBlenderGenerator")
                .unwrap();
            let child =
                pointer_array_member_values(&file.objects()[blend].members, "children").unwrap()[0];
            assert_eq!(
                pointer_member_value(&file.objects()[child].members, "boneWeights"),
                Some(5)
            );
            assert_eq!(
                pointer_member_value(&file.objects()[child].members, "generator"),
                Some(if list { 1 } else { 3 })
            );
            if list {
                assert_eq!(
                    pointer_member_value(&file.objects()[0].members, "generator"),
                    Some(blend)
                );
            }
        }
    }
    #[test]
    fn delegated_base_locomotion_never_gets_the_action_idle_mask() {
        for list in [false, true] {
            let mut file = fixture(list);
            lower(&mut file, &HashSet::from([1]));
            assert!(file
                .objects()
                .iter()
                .all(|o| o.class_name != "hkbBlenderGenerator"));
            assert_eq!(
                pointer_member_value(&file.objects()[1].members, "generator"),
                Some(3)
            );
        }
    }

    #[test]
    fn dynamic_mask_preserves_character_property_binding_and_action_selector() {
        let mut file = fixture(false);
        let binding = |member: &str, index| {
            HkxValue::Object(vec![
                member_value("memberPath", string_hkx_value(member)),
                member_value("variableIndex", HkxValue::I32(index)),
                member_value("bindingType", HkxValue::I8(1)),
                member_value("bitIndex", HkxValue::I8(-1)),
            ])
        };
        let set = push_named_object(
            &mut file,
            "hkbVariableBindingSet",
            2,
            vec![member_value(
                "bindings",
                HkxValue::Array(vec![
                    binding("boneWeights1", 4),
                    binding("boneWeightActionVar", 7),
                ]),
            )],
        );
        file.objects_mut()[2].members.push(member_value(
            "variableBindingSet",
            HkxValue::Pointer(Some(set)),
        ));
        lower(&mut file, &HashSet::new());
        let selector = file
            .objects()
            .iter()
            .find(|o| o.class_name == "hkbManualSelectorGenerator")
            .unwrap();
        assert_eq!(selector.signature, 3);
        assert_eq!(
            selector
                .members
                .iter()
                .find(|m| m.name == "selectedGeneratorIndex")
                .unwrap()
                .value,
            HkxValue::I16(1)
        );
        let set = pointer_member_value(&selector.members, "variableBindingSet").unwrap();
        let HkxValue::Array(entries) = &file.objects()[set]
            .members
            .iter()
            .find(|m| m.name == "bindings")
            .unwrap()
            .value
        else {
            panic!()
        };
        let fields = entries[0].as_object_members().unwrap();
        assert_eq!(
            string_member_value(fields, "memberPath"),
            Some("selectedGeneratorIndex")
        );
        assert_eq!(
            fields
                .iter()
                .find(|m| m.name == "variableIndex")
                .and_then(|m| extract_int(&m.value)),
            Some(7)
        );
        assert_eq!(
            fields
                .iter()
                .find(|m| m.name == "bindingType")
                .and_then(|m| extract_int(&m.value)),
            Some(1)
        );
    }
}
