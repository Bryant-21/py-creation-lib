use crate::hkx::HkxMember;
use crate::hkx::types::HkxValue;

use super::corpus_generated;
use super::hooks;
use super::hooks::CustomHookRegistry;
use super::manager::PatchManager;
use super::ops::{ClassVersion, Patch};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativePatchCorpusManifest {
    pub total_patches: usize,
    pub patches_by_version_package: &'static [(u8, usize)],
    pub member_add_ops: usize,
    pub member_remove_ops: usize,
    pub member_rename_ops: usize,
    pub parent_set_ops: usize,
    pub depends_ops: usize,
    pub callable_hooks: usize,
    pub reversible_callable_hooks: usize,
    pub class_added_ops: usize,
    pub class_removed_ops: usize,
    pub named_hooks: &'static [&'static str],
}

pub const NATIVE_PATCH_CORPUS_MANIFEST: NativePatchCorpusManifest = NativePatchCorpusManifest {
    total_patches: 780,
    patches_by_version_package: &[
        (46, 82),
        (48, 34),
        (50, 25),
        (52, 31),
        (53, 43),
        (55, 494),
        (56, 71),
    ],
    member_add_ops: 843,
    member_remove_ops: 416,
    member_rename_ops: 39,
    parent_set_ops: 120,
    depends_ops: 488,
    callable_hooks: 87,
    reversible_callable_hooks: 1,
    class_added_ops: 189,
    class_removed_ops: 44,
    named_hooks: &[
        "_hclClothData_2_to_3",
        "_hclMeshMeshDeformSetupObject_1_to_2",
        "_hclSimClothData_12_to_13",
        "_hclSimClothSetupObject_5_to_6",
        "_hclSimulateOperator_3_to_4",
        "_hclSimulateSetupObject_3_to_4",
        "_hkAabbHalf_0_to_1",
        "_hkMotionState_2_to_3",
        "_hkReferencedObject_1_to_2",
        "_hkSkinnedMeshShapeBoneSection_0_to_1",
        "_hkSkinnedMeshShapePart_0_to_1",
        "_hkSkinnedRefMeshShape_0_to_1",
        "_hkStorageSkinnedMeshShape_0_to_1",
        "_hkbBehaviorReferenceGenerator_0_to_1",
        "_hkbCharacterControllerModifier_1_to_2",
        "_hkbCharacterData_10_to_11",
        "_hkbCharacterData_11_to_10",
        "_hkbCharacterData_9_to_10",
        "_hkbCharacterStringData_9_to_10",
        "_hkbCharacter_3_to_4",
        "_hkbClipGenerator_4_to_5",
        "_hkbFootIkControlData_0_to_1",
        "_hkbLayer_1_to_2",
        "_hkbRigidBodyRagdollControlData_1_to_2",
        "_hkbRigidBodySetup_0_to_1",
        "_hkbStateMachine_4_to_5",
        "_hkpAngConstraintAtom_0_to_1",
        "_hkpAngLimitConstraintAtom_0_to_1",
        "_hkpBreakableConstraintData_0_to_1",
        "_hkpConvexVerticesShape_4_to_5",
        "_hkpConvexVerticesShape_5_to_6",
        "_hkpDeformableAngConstraintAtom_0_to_1",
        "_hkpDeformableLinConstraintAtom_0_to_1",
        "_hkpEntity_3_to_4",
        "_hkpGroupFilter_0_to_1",
        "_hkpMalleableConstraintData_0_to_1",
        "_hkxAnimatedMatrix_1_to_2",
        "_hkxAnimatedQuaternion_1_to_2",
        "_hkxAnimatedVector_1_to_2",
        "_hkxNode_4_to_5",
        "_hkxVertexBufferVertexData_0_to_1",
        "_hkxVertexBufferVertexData_1_to_2",
        "_hkxVertexVectorDataChannel_1_to_2",
        "_noop_type_change",
        "hclSimClothData_11_to_12",
        "hclSimClothData_9_to_10",
        "hclUpdateAllVertexFramesOperator_2_to_3",
        "hclUpdateSomeVertexFramesOperator_2_to_3",
        "hkBitField_0_hkBitField_new_1",
        "hknpBody_1_to_2",
        "hknpBody_2_to_3",
        "hknpBodyCinfo_2_to_3",
        "hknpBodyCinfo_3_to_4",
        "hknpCharacterRigidBodyCinfo_2_to_3",
        "hknpConstraint_0_to_1",
        "hknpConstraint_1_to_2",
        "hknpConstraintCinfo_4_to_5",
        "hknpConvexPolytopeShape_2_to_3",
        "hknpConvexPolytopeShape_3_to_4",
        "hknpShape_2_to_3",
        "hknpShape_3_to_4",
        "hknpBody_3_to_4",
        "hknpBody_5_to_6",
    ],
};

pub fn native_patch_corpus_manifest() -> &'static NativePatchCorpusManifest {
    &NATIVE_PATCH_CORPUS_MANIFEST
}

const HKB_CHARACTER_DATA_DRIVER_FIELDS: [(&str, &str); 4] = [
    ("mirroredSkeletonInfo", "hkbMirroredSkeletonInfo"),
    ("footIkDriverInfo", "hkbFootIkDriverInfo"),
    ("handIkDriverInfo", "hkbHandIkDriverInfo"),
    ("aiControlDriverInfo", "hkbAiControlDriverInfo"),
];

pub(crate) fn register_native_patches(manager: &mut PatchManager) {
    corpus_generated::register_generated_patches(manager);
    // hkReferencedObject 0->1 also applies at version step 47 (inter-patch-package boundary).
    // The Python corpus registers it only at 46; this explicit registration covers 47.
    manager.register(
        47,
        Patch::new(
            ClassVersion::new("hkReferencedObject", 0),
            ClassVersion::new("hkReferencedObject", 1),
        ),
    );
}

pub(crate) fn register_native_hooks(registry: &mut CustomHookRegistry) {
    registry.register("_hkbCharacterData_10_to_11", |context| {
        let object_indices: Vec<usize> = match context.object_index {
            Some(index) => vec![index],
            None => (0..context.hkx.objects().len()).collect(),
        };
        for index in object_indices {
            let object = &mut context.hkx.objects_mut()[index];
            if object.class_name != "hkbCharacterData" {
                continue;
            }
            let targets: Vec<usize> = HKB_CHARACTER_DATA_DRIVER_FIELDS
                .iter()
                .filter_map(|(field_name, _)| {
                    object
                        .members
                        .iter()
                        .find(|member| member.name == *field_name)
                        .and_then(|member| match member.value {
                            HkxValue::Pointer(Some(target)) => Some(target),
                            _ => None,
                        })
                })
                .collect();
            if let Some(member) = object
                .members
                .iter_mut()
                .find(|member| member.name == "propertySheets")
            {
                if let HkxValue::Array(contents) = &mut member.value {
                    contents.extend(
                        targets
                            .into_iter()
                            .map(|target| HkxValue::Pointer(Some(target))),
                    );
                }
            }
        }
        Ok(())
    });

    registry.register("_hkbCharacterData_11_to_10", |context| {
        let object_indices: Vec<usize> = match context.object_index {
            Some(index) => vec![index],
            None => (0..context.hkx.objects().len()).collect(),
        };
        for index in object_indices {
            if context.hkx.objects()[index].class_name != "hkbCharacterData" {
                continue;
            }

            let mut target_by_driver = [None; 4];
            if let Some(HkxValue::Array(property_sheets)) = context.hkx.objects()[index]
                .members
                .iter()
                .find(|member| member.name == "propertySheets")
                .map(|member| &member.value)
            {
                for sheet in property_sheets {
                    let HkxValue::Pointer(Some(target)) = sheet else {
                        continue;
                    };
                    let Some(target_object) = context.hkx.objects().get(*target) else {
                        continue;
                    };
                    if let Some(driver_index) = HKB_CHARACTER_DATA_DRIVER_FIELDS
                        .iter()
                        .position(|(_, class_name)| target_object.class_name == *class_name)
                    {
                        target_by_driver[driver_index].get_or_insert(*target);
                    }
                }
            }
            for (other_index, other) in context.hkx.objects().iter().enumerate() {
                if let Some(driver_index) = HKB_CHARACTER_DATA_DRIVER_FIELDS
                    .iter()
                    .position(|(_, class_name)| other.class_name == *class_name)
                {
                    target_by_driver[driver_index].get_or_insert(other_index);
                }
            }

            let object = &mut context.hkx.objects_mut()[index];
            if !object
                .members
                .iter()
                .any(|member| member.name == "modelUpMS")
            {
                let insert_at = object
                    .members
                    .iter()
                    .position(|member| member.name == "characterControllerSetup")
                    .map_or(1, |position| position + 1)
                    .min(object.members.len());
                object.members.insert(
                    insert_at,
                    HkxMember {
                        name: "modelUpMS".into(),
                        value: HkxValue::F32List(vec![0.0, 0.0, 1.0, 0.0]),
                    },
                );
                object.members.insert(
                    insert_at + 1,
                    HkxMember {
                        name: "modelForwardMS".into(),
                        value: HkxValue::F32List(vec![1.0, 0.0, 0.0, 0.0]),
                    },
                );
                object.members.insert(
                    insert_at + 2,
                    HkxMember {
                        name: "modelRightMS".into(),
                        value: HkxValue::F32List(vec![0.0, -1.0, 0.0, 0.0]),
                    },
                );
            }
            for ((field_name, _), target) in HKB_CHARACTER_DATA_DRIVER_FIELDS
                .iter()
                .zip(target_by_driver)
            {
                if let Some(member) = object
                    .members
                    .iter_mut()
                    .find(|member| member.name == *field_name)
                {
                    member.value = HkxValue::Pointer(target);
                }
            }
            if let Some(member) = object
                .members
                .iter_mut()
                .find(|member| member.name == "stringData")
            {
                if !matches!(member.value, HkxValue::Pointer(_)) {
                    member.value = HkxValue::Pointer(None);
                }
            }
            if let Some(member) = object
                .members
                .iter_mut()
                .find(|member| member.name == "scale")
            {
                member.value = HkxValue::F32(1.0);
            }
        }
        Ok(())
    });

    registry.register("_hkpGroupFilter_0_to_1", |context| {
        let object_indices: Vec<usize> = match context.object_index {
            Some(index) => vec![index],
            None => (0..context.hkx.objects().len()).collect(),
        };
        for index in object_indices {
            let object = &mut context.hkx.objects_mut()[index];
            if object.class_name != "hkpGroupFilter" {
                continue;
            }
            let next_free = object
                .members
                .iter()
                .find(|member| member.name == "old_nextFreeSystemGroup")
                .map(|member| member.value.clone());
            let collision_lookup = object
                .members
                .iter()
                .find(|member| member.name == "old_collisionLookupTable")
                .map(|member| member.value.clone());
            if let Some(value) = next_free {
                if let Some(member) = object
                    .members
                    .iter_mut()
                    .find(|member| member.name == "nextFreeSystemGroup")
                {
                    member.value = value;
                }
            }
            if let Some(value) = collision_lookup {
                if let Some(member) = object
                    .members
                    .iter_mut()
                    .find(|member| member.name == "collisionLookupTable")
                {
                    member.value = value;
                }
            }
        }
        Ok(())
    });

    registry.register("_hkbBehaviorReferenceGenerator_0_to_1", |context| {
        let object_indices: Vec<usize> = match context.object_index {
            Some(index) => vec![index],
            None => (0..context.hkx.objects().len()).collect(),
        };
        for index in object_indices {
            let object = &mut context.hkx.objects_mut()[index];
            if object.class_name != "hkbBehaviorReferenceGenerator" {
                continue;
            }
            if let Some(member) = object
                .members
                .iter_mut()
                .find(|member| member.name == "behaviorName")
            {
                if let HkxValue::String { value, .. } = &mut member.value {
                    if let Some(dot) = value.rfind('.') {
                        if dot > 0 {
                            value.truncate(dot);
                        }
                    }
                }
            }
        }
        Ok(())
    });

    registry.register("_hkbCharacterStringData_9_to_10", |_context| Ok(()));

    registry.register("_hkbClipGenerator_4_to_5", |context| {
        let object_indices: Vec<usize> = match context.object_index {
            Some(index) => vec![index],
            None => (0..context.hkx.objects().len()).collect(),
        };
        for index in object_indices {
            let object = &mut context.hkx.objects_mut()[index];
            if object.class_name != "hkbClipGenerator" {
                continue;
            }
            let bundle_name = object
                .members
                .iter()
                .find(|member| member.name == "animationBundleName")
                .and_then(|member| match &member.value {
                    HkxValue::String { value, .. } if !value.is_empty() => Some(value.clone()),
                    _ => None,
                });
            let animation_name = object
                .members
                .iter()
                .find(|member| member.name == "animationName")
                .and_then(|member| match &member.value {
                    HkxValue::String { value, .. } if !value.is_empty() => Some(value.clone()),
                    _ => None,
                });
            let (Some(bundle_name), Some(animation_name)) = (bundle_name, animation_name) else {
                continue;
            };
            let split_at = animation_name
                .rfind(['/', '\\'])
                .map_or(0, |separator| separator + 1);
            let merged_name = format!(
                "{}{}:{}",
                &animation_name[..split_at],
                bundle_name,
                &animation_name[split_at..]
            );
            if let Some(member) = object
                .members
                .iter_mut()
                .find(|member| member.name == "animationName")
            {
                member.value = HkxValue::String {
                    value: merged_name,
                    is_null: false,
                };
            }
        }
        Ok(())
    });

    // --- Real hook implementations ported from py_creation_lib/python/creation_lib/havok_convert/patches/ ---

    // _hkbCharacter_3_to_4: set capabilities/effectiveCapabilities to -1.
    // Python: py_creation_lib/python/creation_lib/havok_convert/patches/p2013_2/behavior.py:11
    registry.register("_hkbCharacter_3_to_4", |context| {
        for index in object_indices_for(context) {
            let object = &mut context.hkx.objects_mut()[index];
            if object.class_name != "hkbCharacter" {
                continue;
            }
            for member in object.members.iter_mut() {
                if member.name == "capabilities" || member.name == "effectiveCapabilities" {
                    member.value = HkxValue::I32(-1);
                }
            }
        }
        Ok(())
    });

    // _hkbStateMachine_4_to_5: copy startStateChooser pointer to startStateIdSelector.
    // Python: py_creation_lib/python/creation_lib/havok_convert/patches/p2013_2/behavior.py:20
    registry.register("_hkbStateMachine_4_to_5", |context| {
        for index in object_indices_for(context) {
            let object = &mut context.hkx.objects_mut()[index];
            if object.class_name != "hkbStateMachine" {
                continue;
            }
            let chooser = object
                .members
                .iter()
                .find(|m| m.name == "startStateChooser")
                .map(|m| m.value.clone());
            if let Some(value) = chooser {
                if let Some(target) = object
                    .members
                    .iter_mut()
                    .find(|m| m.name == "startStateIdSelector")
                {
                    target.value = value;
                }
            }
        }
        Ok(())
    });

    // _hkAabbHalf_0_to_1: merge data_old (6) + extras (2) into data (8).
    // Python: py_creation_lib/python/creation_lib/havok_convert/patches/p2013_2/common.py:10
    registry.register("_hkAabbHalf_0_to_1", |context| {
        for index in object_indices_for(context) {
            let object = &mut context.hkx.objects_mut()[index];
            if object.class_name != "hkAabbHalf" {
                continue;
            }
            let old_data = object
                .members
                .iter()
                .find(|m| m.name == "data_old")
                .map(|m| m.value.clone());
            let extras = object
                .members
                .iter()
                .find(|m| m.name == "extras")
                .map(|m| m.value.clone());
            if let Some(target) = object.members.iter_mut().find(|m| m.name == "data") {
                if let (Some(HkxValue::F32List(old_vec)), Some(HkxValue::F32List(extras_vec))) =
                    (old_data, extras)
                {
                    let mut merged = Vec::with_capacity(8);
                    merged.extend(old_vec.iter().take(6).copied());
                    merged.extend(extras_vec.iter().take(2).copied());
                    target.value = HkxValue::F32List(merged);
                }
            }
        }
        Ok(())
    });

    // _hkpAngConstraintAtom_0_to_1: constrainedAxes[i] = (firstConstrainedAxis + i) % 3
    // Python: py_creation_lib/python/creation_lib/havok_convert/patches/p2014_1/physics.py:17
    registry.register("_hkpAngConstraintAtom_0_to_1", |context| {
        for index in object_indices_for(context) {
            let object = &mut context.hkx.objects_mut()[index];
            if object.class_name != "hkpAngConstraintAtom" {
                continue;
            }
            let first = object
                .members
                .iter()
                .find(|m| m.name == "firstConstrainedAxis")
                .map(|m| match &m.value {
                    HkxValue::I8(v) => *v as i32,
                    HkxValue::I16(v) => *v as i32,
                    HkxValue::I32(v) => *v,
                    HkxValue::U8(v) => *v as i32,
                    _ => 0,
                })
                .unwrap_or(0);
            if let Some(target) = object
                .members
                .iter_mut()
                .find(|m| m.name == "constrainedAxes")
            {
                target.value =
                    HkxValue::F32List((0..3).map(|i| ((first + i).rem_euclid(3)) as f32).collect());
            }
        }
        Ok(())
    });

    // _hkpAngLimitConstraintAtom_0_to_1: cosineAxis = (limitAxis + 1) % 3
    // Python: py_creation_lib/python/creation_lib/havok_convert/patches/p2014_1/physics.py:29
    registry.register("_hkpAngLimitConstraintAtom_0_to_1", |context| {
        for index in object_indices_for(context) {
            let object = &mut context.hkx.objects_mut()[index];
            if object.class_name != "hkpAngLimitConstraintAtom" {
                continue;
            }
            let limit = object
                .members
                .iter()
                .find(|m| m.name == "limitAxis")
                .map(|m| match &m.value {
                    HkxValue::I8(v) => *v as i32,
                    HkxValue::I16(v) => *v as i32,
                    HkxValue::I32(v) => *v,
                    HkxValue::U8(v) => *v as i32,
                    _ => 0,
                })
                .unwrap_or(0);
            if let Some(target) = object.members.iter_mut().find(|m| m.name == "cosineAxis") {
                target.value = HkxValue::I8(((limit + 1).rem_euclid(3)) as i8);
            }
        }
        Ok(())
    });

    // _hkbCharacterControllerModifier_1_to_2:
    // gravityFactor = (applyGravity > 0) ? 1.0 : 0.0
    // Python: py_creation_lib/python/creation_lib/havok_convert/patches/p2014_1/behavior.py:16
    registry.register("_hkbCharacterControllerModifier_1_to_2", |context| {
        for index in object_indices_for(context) {
            let object = &mut context.hkx.objects_mut()[index];
            if object.class_name != "hkbCharacterControllerModifier" {
                continue;
            }
            let apply_gravity = object
                .members
                .iter()
                .find(|m| m.name == "applyGravity")
                .map(|m| match &m.value {
                    HkxValue::Bool(v) => *v,
                    HkxValue::I8(v) => *v > 0,
                    HkxValue::U8(v) => *v > 0,
                    HkxValue::I16(v) => *v > 0,
                    HkxValue::I32(v) => *v > 0,
                    _ => false,
                })
                .unwrap_or(false);
            if let Some(target) = object
                .members
                .iter_mut()
                .find(|m| m.name == "gravityFactor")
            {
                target.value = HkxValue::F32(if apply_gravity { 1.0 } else { 0.0 });
            }
        }
        Ok(())
    });

    // _hkxNode_4_to_5: assign random 16-byte UUID to uuid member.
    // Python: py_creation_lib/python/creation_lib/havok_convert/patches/p2014_1/common.py:17
    registry.register("_hkxNode_4_to_5", |context| {
        for index in object_indices_for(context) {
            let object = &mut context.hkx.objects_mut()[index];
            if object.class_name != "hkxNode" {
                continue;
            }
            if let Some(target) = object.members.iter_mut().find(|m| m.name == "uuid") {
                // Generate 16 random bytes and store as 4 u32 values matching
                // Python's int.from_bytes(os.urandom(16), 'little').
                let bytes = pseudo_random_bytes_16();
                target.value = HkxValue::F32List(vec![
                    f32::from_bits(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])),
                    f32::from_bits(u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]])),
                    f32::from_bits(u32::from_le_bytes([
                        bytes[8], bytes[9], bytes[10], bytes[11],
                    ])),
                    f32::from_bits(u32::from_le_bytes([
                        bytes[12], bytes[13], bytes[14], bytes[15],
                    ])),
                ]);
            }
        }
        Ok(())
    });

    // _hkpEntity_3_to_4: copy motion_old → motion (type change hkpMotion → hkpMaxSizeMotion).
    // Python: py_creation_lib/python/creation_lib/havok_convert/patches/p2014_2_5/physics.py:18
    registry.register("_hkpEntity_3_to_4", |context| {
        for index in object_indices_for(context) {
            let object = &mut context.hkx.objects_mut()[index];
            if object.class_name != "hkpEntity" {
                continue;
            }
            let old_val = object
                .members
                .iter()
                .find(|m| m.name == "motion_old")
                .map(|m| m.value.clone());
            if let Some(value) = old_val {
                if let Some(target) = object.members.iter_mut().find(|m| m.name == "motion") {
                    target.value = value;
                }
            }
        }
        Ok(())
    });

    // _hkpBreakableConstraintData_0_to_1: copy constraintDataOld → constraintData.
    // Python: py_creation_lib/python/creation_lib/havok_convert/patches/p2013_2/physics.py:13
    registry.register("_hkpBreakableConstraintData_0_to_1", |context| {
        copy_member_value(
            context,
            "hkpBreakableConstraintData",
            "constraintDataOld",
            "constraintData",
        );
        Ok(())
    });

    // _hkpMalleableConstraintData_0_to_1: copy constraintDataOld → constraintData.
    // Python: py_creation_lib/python/creation_lib/havok_convert/patches/p2013_2/physics.py:27
    registry.register("_hkpMalleableConstraintData_0_to_1", |context| {
        copy_member_value(
            context,
            "hkpMalleableConstraintData",
            "constraintDataOld",
            "constraintData",
        );
        Ok(())
    });

    // _hkSkinnedMeshShapePart_0_to_1: copy boneIndex → boneSetId.
    // Python: py_creation_lib/python/creation_lib/havok_convert/patches/p2012_2/common.py:55
    registry.register("_hkSkinnedMeshShapePart_0_to_1", |context| {
        copy_member_value(context, "hkSkinnedMeshShapePart", "boneIndex", "boneSetId");
        Ok(())
    });

    // _hkSkinnedMeshShapeBoneSection_0_to_1: copy startBoneIndex → startBoneSetId, numBones → numBoneSets.
    // Python: py_creation_lib/python/creation_lib/havok_convert/patches/p2012_2/common.py:68
    registry.register("_hkSkinnedMeshShapeBoneSection_0_to_1", |context| {
        copy_member_value(
            context,
            "hkSkinnedMeshShapeBoneSection",
            "startBoneIndex",
            "startBoneSetId",
        );
        copy_member_value(
            context,
            "hkSkinnedMeshShapeBoneSection",
            "numBones",
            "numBoneSets",
        );
        Ok(())
    });

    // _hkReferencedObject_1_to_2: append propertyBag pointer to dynamicProperties array.
    // Python: py_creation_lib/python/creation_lib/havok_convert/patches/p2015_1/common.py:13
    registry.register("_hkReferencedObject_1_to_2", |context| {
        for index in object_indices_for(context) {
            let object = &mut context.hkx.objects_mut()[index];
            // hkReferencedObject is the parent of nearly every class — apply
            // wherever the propertyBag/dynamicProperties pair is present.
            let prop_bag = object
                .members
                .iter()
                .find(|m| m.name == "propertyBag")
                .and_then(|m| match m.value {
                    HkxValue::Pointer(target) => Some(target),
                    _ => None,
                });
            if let Some(Some(target)) = prop_bag {
                if let Some(target_member) = object
                    .members
                    .iter_mut()
                    .find(|m| m.name == "dynamicProperties")
                {
                    if let HkxValue::Array(contents) = &mut target_member.value {
                        contents.push(HkxValue::Pointer(Some(target)));
                    }
                }
            }
        }
        Ok(())
    });

    // _hkbFootIkControlData_0_to_1: Python implementation has no semantic effect
    // (the loop body is `break` after the first match without mutation).
    // Mirror as no-op here. Default value from MemberAdd ("vec4") already provides
    // a vec4 of zeros — the C++ SDK would set them to 1.0 but Python does not.
    registry.register("_hkbFootIkControlData_0_to_1", |_context| Ok(()));

    // _hkxVertexBufferVertexData_1_to_2: reinterpret floats as uint32 bit patterns.
    // Python: py_creation_lib/python/creation_lib/havok_convert/patches/p2013_3/common.py:11
    registry.register("_hkxVertexBufferVertexData_1_to_2", |context| {
        for index in object_indices_for(context) {
            let object = &mut context.hkx.objects_mut()[index];
            if object.class_name != "hkxVertexBufferVertexData" {
                continue;
            }
            for field in ["floatData", "vectorData"] {
                let old_name = format!("{field}Old");
                let src = object
                    .members
                    .iter()
                    .find(|m| m.name == old_name)
                    .and_then(|m| match &m.value {
                        HkxValue::F32List(items) => Some(items.clone()),
                        HkxValue::Array(items) => {
                            let collected: Vec<f32> = items
                                .iter()
                                .filter_map(|v| match v {
                                    HkxValue::F32(value) => Some(*value),
                                    _ => None,
                                })
                                .collect();
                            Some(collected)
                        }
                        _ => None,
                    })
                    .unwrap_or_default();
                if let Some(target) = object.members.iter_mut().find(|m| m.name == field) {
                    let bits: Vec<HkxValue> = src
                        .iter()
                        .map(|value| HkxValue::U32(value.to_bits()))
                        .collect();
                    target.value = HkxValue::Array(bits);
                }
            }
        }
        Ok(())
    });

    // Real implementations for the triangle-flip cloth hooks.
    // Python: py_creation_lib/python/creation_lib/havok_convert/patches/p2013_1/cloth.py — calls _update_triangle_flips,
    // which expands each old int32 into 4 little-endian bytes.
    registry.register(
        "hclUpdateAllVertexFramesOperator_2_to_3",
        hooks::hcl_update_all_vertex_frames_operator_2_to_3,
    );
    registry.register(
        "hclUpdateSomeVertexFramesOperator_2_to_3",
        hooks::hcl_update_some_vertex_frames_operator_2_to_3,
    );
    registry.register("hclSimClothData_9_to_10", hooks::hcl_sim_cloth_data_9_to_10);

    // hknp 2014.1 callbacks (version_id = 53).
    registry.register(
        "hknpCharacterRigidBodyCinfo_2_to_3",
        hooks::hknp_character_rigid_body_cinfo_2_to_3,
    );
    registry.register("hknpBody_1_to_2", hooks::hknp_body_1_to_2);
    registry.register("hknpBodyCinfo_2_to_3", hooks::hknp_body_cinfo_2_to_3);
    registry.register("hknpBodyCinfo_3_to_4", hooks::hknp_body_cinfo_3_to_4);
    registry.register("hknpConstraint_0_to_1", hooks::hknp_constraint_0_to_1);
    registry.register(
        "hknpConvexPolytopeShape_2_to_3",
        hooks::hknp_convex_polytope_shape_2_to_3,
    );

    // hknp 2014.2 callbacks (version_id = 55, shared with 2014_2_5).
    registry.register("hknpBody_2_to_3", hooks::hknp_body_2_to_3);
    registry.register("hknpShape_2_to_3", hooks::hknp_shape_2_to_3);
    registry.register("hknpConstraint_1_to_2", hooks::hknp_constraint_1_to_2);
    registry.register(
        "hknpConvexPolytopeShape_3_to_4",
        hooks::hknp_convex_polytope_shape_3_to_4,
    );

    // hknp 2014.2.5 callbacks (version_id = 55, same package as 2014_2).
    registry.register("hknpShape_3_to_4", hooks::hknp_shape_3_to_4);
    registry.register("hknpBody_3_to_4", hooks::hknp_body_3_to_4);
    registry.register("hknpBody_5_to_6", hooks::hknp_body_5_to_6);

    // hknp 2015.1 callbacks (version_id = 56).
    registry.register(
        "hknpConstraintCinfo_4_to_5",
        hooks::hknp_constraint_cinfo_4_to_5,
    );

    // No-op stubs — Python source is pass-equivalent for each:
    //
    // - _hclClothData_2_to_3: pass (p2014_2_5/cloth.py:34)
    // - _hclMeshMeshDeformSetupObject_1_to_2: pass (p2012_2/cloth.py:15)
    // - _hclSimClothData_12_to_13: pass (p2014_2_5/cloth.py:29)
    // - _hclSimClothSetupObject_5_to_6: pass (p2014_2_5/cloth.py:39)
    // - _hclSimulateOperator_3_to_4: pass (p2014_2_5/cloth.py:19)
    // - _hclSimulateSetupObject_3_to_4: pass (p2014_2_5/cloth.py:24)
    // - _hkMotionState_2_to_3: pass (p2012_2/common.py:20)
    // - _hkSkinnedRefMeshShape_0_to_1: pass (p2012_2/common.py:25)
    // - _hkStorageSkinnedMeshShape_0_to_1: pass (p2012_2/common.py:63)
    // - _hkbCharacterData_9_to_10: pass (p2012_2/behavior.py:16)
    // - _hkbCharacterStringData_9_to_10: pass (p2014_2_5/behavior.py:44)
    // - _hkbLayer_1_to_2: pass (p2014_2_5/behavior.py:186)
    // - _hkbRigidBodyRagdollControlData_1_to_2: pass (p2012_2/behavior.py:26)
    // - _hkbRigidBodySetup_0_to_1: pass (p2014_2_5/behavior.py:195)
    // - _hkpConvexVerticesShape_4_to_5: pass (p2012_2/physics.py:28)
    // - _hkpConvexVerticesShape_5_to_6: pass (p2012_2/physics.py:33)
    // - _hkpDeformableAngConstraintAtom_0_to_1: pass (p2012_2/physics.py:23)
    // - _hkpDeformableLinConstraintAtom_0_to_1: pass (p2012_2/physics.py:16)
    // - _hkxAnimatedMatrix_1_to_2: pass (p2012_2/common.py:50)
    // - _hkxAnimatedQuaternion_1_to_2: pass (p2012_2/common.py:45)
    // - _hkxAnimatedVector_1_to_2: pass (p2012_2/common.py:40)
    // - _hkxVertexBufferVertexData_0_to_1: pass (p2012_2/common.py:30)
    // - _hkxVertexVectorDataChannel_1_to_2: pass (p2012_2/common.py:35)
    // - _noop_type_change: pass (p2014_2_5/common.py:21 + animation.py + behavior.py + cloth.py)
    // - hclSimClothData_11_to_12: debug-log only, no data mutation (p2014_2/cloth.py:28)
    // - hkBitField_0_hkBitField_new_1: debug-log only, no data mutation (p2013_1/common.py:28)
    // - _hkbFootIkControlData_0_to_1: loop breaks on first match without mutating (p2013_3/behavior.py:10)
    for name in [
        "_hclClothData_2_to_3",
        "_hclMeshMeshDeformSetupObject_1_to_2",
        "_hclSimClothData_12_to_13",
        "_hclSimClothSetupObject_5_to_6",
        "_hclSimulateOperator_3_to_4",
        "_hclSimulateSetupObject_3_to_4",
        "_hkMotionState_2_to_3",
        "_hkSkinnedRefMeshShape_0_to_1",
        "_hkStorageSkinnedMeshShape_0_to_1",
        "_hkbCharacterData_9_to_10",
        "_hkbCharacterStringData_9_to_10",
        "_hkbLayer_1_to_2",
        "_hkbRigidBodyRagdollControlData_1_to_2",
        "_hkbRigidBodySetup_0_to_1",
        "_hkpConvexVerticesShape_4_to_5",
        "_hkpConvexVerticesShape_5_to_6",
        "_hkpDeformableAngConstraintAtom_0_to_1",
        "_hkpDeformableLinConstraintAtom_0_to_1",
        "_hkxAnimatedMatrix_1_to_2",
        "_hkxAnimatedQuaternion_1_to_2",
        "_hkxAnimatedVector_1_to_2",
        "_hkxVertexBufferVertexData_0_to_1",
        "_hkxVertexVectorDataChannel_1_to_2",
        "_noop_type_change",
        "hclSimClothData_11_to_12",
        "hkBitField_0_hkBitField_new_1",
    ] {
        registry.register(name, |_context| Ok(()));
    }
}

fn object_indices_for(context: &super::hooks::ConversionContext<'_>) -> Vec<usize> {
    match context.object_index {
        Some(index) => vec![index],
        None => (0..context.hkx.objects().len()).collect(),
    }
}

fn copy_member_value(
    context: &mut super::hooks::ConversionContext<'_>,
    class_name: &str,
    src_name: &str,
    dst_name: &str,
) {
    for index in object_indices_for(context) {
        let object = &mut context.hkx.objects_mut()[index];
        if object.class_name != class_name {
            continue;
        }
        let src = object
            .members
            .iter()
            .find(|m| m.name == src_name)
            .map(|m| m.value.clone());
        if let Some(value) = src {
            if let Some(target) = object.members.iter_mut().find(|m| m.name == dst_name) {
                target.value = value;
            }
        }
    }
}

/// Pseudo-random 16 bytes derived from a per-call seed — used by
/// `_hkxNode_4_to_5` to mirror Python's `os.urandom(16)`.
///
/// Real OS randomness isn't available without pulling in `getrandom`. Use a
/// simple xorshift PRNG seeded from a process-wide atomic counter mixed with
/// the system time. The output is unique-per-call within the process, which
/// matches the C++ SDK's intent (assign a fresh UUID).
fn pseudo_random_bytes_16() -> [u8; 16] {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static COUNTER: AtomicU64 = AtomicU64::new(0xdead_beef_1234_5678);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut state = nanos.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ counter ^ 0xA5A5_5A5A_F00D_BAAD;
    let mut out = [0u8; 16];
    for chunk in out.chunks_mut(8) {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        chunk.copy_from_slice(&state.to_le_bytes());
    }
    out
}
