// Native Havok conversion corpus. Hand-maintained.

use super::super::manager::PatchManager;
use super::super::ops::{ClassVersion, Patch, PatchOperation, PatchValue};

pub(super) fn register(manager: &mut PatchManager) {
    // version_id = 46
    // common.py
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("hkxVertexBufferVertexData", 0),
            ClassVersion::new("hkxVertexBufferVertexData", 1),
        )
        .with_operation(PatchOperation::MemberRename {
            old_name: "vectorData".to_string(),
            new_name: "old_vectorData".to_string(),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "vectorData".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_custom_hook("_hkxVertexBufferVertexData_0_to_1")
        .with_operation(PatchOperation::MemberRemove {
            name: "old_vectorData".to_string(),
            type_name: "array".to_string(),
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("hkxAnimatedQuaternion", 1),
            ClassVersion::new("hkxAnimatedQuaternion", 2),
        )
        .with_operation(PatchOperation::MemberRename {
            old_name: "quaternions".to_string(),
            new_name: "old_quaternions".to_string(),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "quaternions".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_custom_hook("_hkxAnimatedQuaternion_1_to_2")
        .with_operation(PatchOperation::MemberRemove {
            name: "old_quaternions".to_string(),
            type_name: "array".to_string(),
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("hkxAnimatedMatrix", 1),
            ClassVersion::new("hkxAnimatedMatrix", 2),
        )
        .with_operation(PatchOperation::MemberRename {
            old_name: "matrices".to_string(),
            new_name: "old_matrices".to_string(),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "matrices".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_custom_hook("_hkxAnimatedMatrix_1_to_2")
        .with_operation(PatchOperation::MemberRemove {
            name: "old_matrices".to_string(),
            type_name: "array".to_string(),
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("hkxVertexVectorDataChannel", 1),
            ClassVersion::new("hkxVertexVectorDataChannel", 2),
        )
        .with_operation(PatchOperation::MemberRename {
            old_name: "perVertexVectors".to_string(),
            new_name: "old_perVertexVectors".to_string(),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "perVertexVectors".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_custom_hook("_hkxVertexVectorDataChannel_1_to_2")
        .with_operation(PatchOperation::MemberRemove {
            name: "old_perVertexVectors".to_string(),
            type_name: "array".to_string(),
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("hkxAnimatedVector", 1),
            ClassVersion::new("hkxAnimatedVector", 2),
        )
        .with_operation(PatchOperation::MemberRename {
            old_name: "vectors".to_string(),
            new_name: "old_vectors".to_string(),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "vectors".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_custom_hook("_hkxAnimatedVector_1_to_2")
        .with_operation(PatchOperation::MemberRemove {
            name: "old_vectors".to_string(),
            type_name: "array".to_string(),
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("hkSweptTransform", 0),
            ClassVersion::new("hkSweptTransformf", 0),
        ),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("hkQTransform", 0),
            ClassVersion::new("hkQTransformf", 0),
        ),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("hkFourTransposedPoints", 0),
            ClassVersion::new("hkFourTransposedPointsf", 0),
        ),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("hkMotionState", 2),
            ClassVersion::new("hkMotionState", 3),
        )
        .with_operation(PatchOperation::MemberRename {
            old_name: "sweptTransform".to_string(),
            new_name: "sweptTransform_old".to_string(),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "sweptTransform".to_string(),
            type_name: "vec4".to_string(),
            ctype: None,
            default: None,
        })
        .with_custom_hook("_hkMotionState_2_to_3")
        .with_operation(PatchOperation::MemberRemove {
            name: "sweptTransform_old".to_string(),
            type_name: "struct".to_string(),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkSweptTransformf".to_string(),
            version: 0,
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("hkSkinnedRefMeshShape", 0),
            ClassVersion::new("hkSkinnedRefMeshShape", 1),
        )
        .with_operation(PatchOperation::MemberRename {
            old_name: "localFromRootTransforms".to_string(),
            new_name: "localFromRootTransforms_old".to_string(),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "localFromRootTransforms".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_custom_hook("_hkSkinnedRefMeshShape_0_to_1")
        .with_operation(PatchOperation::MemberRemove {
            name: "localFromRootTransforms_old".to_string(),
            type_name: "array".to_string(),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkQTransformf".to_string(),
            version: 0,
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("hkSymmetricMatrix3", 0),
            ClassVersion::new("", -2),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "diag".to_string(),
            type_name: "vec4".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "offDiag".to_string(),
            type_name: "vec4".to_string(),
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("hkpPropertyValue", 0),
            ClassVersion::new("hkSimplePropertyValue", 1),
        ),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("hkpProperty", 1),
            ClassVersion::new("hkSimpleProperty", 2),
        ),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkStringObject", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkReferencedObject".to_string()),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "string".to_string(),
            type_name: "string".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkBaseObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkSkinnedMeshShapeBoneSet", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "boneBufferOffset".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "numBones".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkSkinBinding", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkMeshShape".to_string()),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "skin".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkMeshShape".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "worldFromBoneTransforms".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "boneNames".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkMeshShape".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkBaseObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("hkSkinnedMeshShapePart", 0),
            ClassVersion::new("hkSkinnedMeshShapePart", 1),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "boneSetId".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_custom_hook("_hkSkinnedMeshShapePart_0_to_1")
        .with_operation(PatchOperation::MemberRemove {
            name: "boneIndex".to_string(),
            type_name: "int".to_string(),
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("hkStorageSkinnedMeshShape", 0),
            ClassVersion::new("hkStorageSkinnedMeshShape", 1),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "bonesBuffer".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "boneSets".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hkSkinnedMeshShapeBoneSet".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkSkinnedMeshShapeBoneSet".to_string(),
            version: 0,
        })
        .with_custom_hook("_hkStorageSkinnedMeshShape_0_to_1"),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("hkSkinnedMeshShapeBoneSection", 0),
            ClassVersion::new("hkSkinnedMeshShapeBoneSection", 1),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "startBoneSetId".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "numBoneSets".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_custom_hook("_hkSkinnedMeshShapeBoneSection_0_to_1")
        .with_operation(PatchOperation::MemberRemove {
            name: "startBoneIndex".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "numBones".to_string(),
            type_name: "int".to_string(),
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("hkGeometry", 0),
            ClassVersion::new("hkGeometry", 1),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkReferencedObject".to_string()),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkIntRealPair", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "key".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "value".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkSetIntFloatPair", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some(
                "hkSethkIntRealPairhkContainerHeapAllocatorhkMapOperationshkIntRealPair"
                    .to_string(),
            ),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkSethkIntRealPairhkContainerHeapAllocatorhkMapOperationshkIntRealPair"
                .to_string(),
            version: 0,
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new(
                "hkSethkIntRealPairhkContainerHeapAllocatorhkMapOperationshkIntRealPair",
                0,
            ),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "elem".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hkIntRealPair".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "numElems".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkIntRealPair".to_string(),
            version: 0,
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("hkRefCountedProperties", 0),
            ClassVersion::new("hkRefCountedProperties", 1),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: Some("hkReferencedObject".to_string()),
            new_parent: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkRefCountedPropertiesEntry".to_string(),
            version: 0,
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("hkpMassProperties", 0),
            ClassVersion::new("hkMassProperties", 1),
        ),
    );
    // behavior.py
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkbProceduralBlenderGenerator", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkbGenerator".to_string()),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbNode".to_string(),
            version: 1,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkBaseObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbGenerator".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbBindable".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("hkbProceduralBlenderGenerator", 0),
            ClassVersion::new("hkbProceduralBlenderGenerator", 1),
        ),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("hkbParametricMotionGenerator", 1),
            ClassVersion::new("hkbParametricMotionGenerator", 2),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: Some("hkbGenerator".to_string()),
            new_parent: Some("hkbProceduralBlenderGenerator".to_string()),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbGenerator".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbProceduralBlenderGenerator".to_string(),
            version: 1,
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkbConstraintSetup", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "type".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkbRagdollControllerSetup", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "type".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkbShapeSetup", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "capsuleHeight".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: Some(PatchValue::Real(1.7_f32)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "capsuleRadius".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: Some(PatchValue::Real(0.4_f32)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "fileName".to_string(),
            type_name: "string".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "type".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkbRigidBodySetup", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "collisionFilterInfo".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(1)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "type".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "shapeSetup".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkbShapeSetup".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbShapeSetup".to_string(),
            version: 0,
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkbCharacterControllerSetup", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "rigidBodySetup".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkbRigidBodySetup".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "controllerCinfo".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkReferencedObject".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbRigidBodySetup".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkbRagdollInterface", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkReferencedObject".to_string()),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkBaseObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkbPhysicsInterface", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkReferencedObject".to_string()),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkBaseObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkbNullPhysicsInterface", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkbPhysicsInterface".to_string()),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbPhysicsInterface".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkBaseObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkbKeyFrameControlData", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "hierarchyGain".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: Some(PatchValue::Real(0.17_f32)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "velocityDamping".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: Some(PatchValue::Real(0.0_f32)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "accelerationGain".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: Some(PatchValue::Real(1.0_f32)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "velocityGain".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: Some(PatchValue::Real(0.6_f32)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "positionGain".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: Some(PatchValue::Real(0.05_f32)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "positionMaxLinearVelocity".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: Some(PatchValue::Real(1.4_f32)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "positionMaxAngularVelocity".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: Some(PatchValue::Real(1.8_f32)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "snapGain".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: Some(PatchValue::Real(0.1_f32)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "snapMaxLinearVelocity".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: Some(PatchValue::Real(0.3_f32)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "snapMaxAngularVelocity".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: Some(PatchValue::Real(0.3_f32)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "snapMaxLinearDistance".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: Some(PatchValue::Real(0.03_f32)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "snapMaxAngularDistance".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: Some(PatchValue::Real(0.1_f32)),
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("hkbCharacterData", 8),
            ClassVersion::new("hkbCharacterData", 9),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "aiControlDriverInfo".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkReferencedObject".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("hkbCharacterData", 9),
            ClassVersion::new("hkbCharacterData", 10),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "characterControllerSetup".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkbCharacterControllerSetup".to_string()),
            default: None,
        })
        .with_custom_hook("_hkbCharacterData_9_to_10")
        .with_operation(PatchOperation::MemberRemove {
            name: "characterControllerInfo".to_string(),
            type_name: "struct".to_string(),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbCharacterControllerSetup".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbRigidBodySetup".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbCharacterDataCharacterControllerInfo".to_string(),
            version: 0,
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("hkbRigidBodyRagdollControlData", 1),
            ClassVersion::new("hkbRigidBodyRagdollControlData", 2),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "keyFrameControlData".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkbKeyFrameControlData".to_string()),
            default: None,
        })
        .with_custom_hook("_hkbRigidBodyRagdollControlData_1_to_2")
        .with_operation(PatchOperation::MemberRemove {
            name: "keyFrameHierarchyControlData".to_string(),
            type_name: "struct".to_string(),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkaKeyFrameHierarchyUtilityControlData".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbKeyFrameControlData".to_string(),
            version: 0,
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkbpRagdollInterface", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkbRagdollInterface".to_string()),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkBaseObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbRagdollInterface".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkbpPhysicsInterface", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkbPhysicsInterface".to_string()),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbPhysicsInterface".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkBaseObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("hkbCustomTestGenerator", 0),
            ClassVersion::new("hkbCustomTestGenerator", 1),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "hkRigidBody".to_string(),
            type_name: "struct".to_string(),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "hkRigidBody".to_string(),
            type_name: "pointer".to_string(),
            ctype: Some("hkReferencedObject".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkpRigidBody".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkpEntity".to_string(),
            version: 3,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkpWorldObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("hkbHandle", 1),
            ClassVersion::new("hkbHandle", 2),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "rigidBody".to_string(),
            type_name: "struct".to_string(),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "rigidBody".to_string(),
            type_name: "pointer".to_string(),
            ctype: Some("hkReferencedObject".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkpRigidBody".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkpEntity".to_string(),
            version: 3,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkpWorldObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("hkbCharacterDataCharacterControllerInfo", 0),
            ClassVersion::new("", -2),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "capsuleHeight".to_string(),
            type_name: "real".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "capsuleRadius".to_string(),
            type_name: "real".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "collisionFilterInfo".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "characterControllerCinfo".to_string(),
            type_name: "struct".to_string(),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkBaseObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkpCharacterControllerCinfo".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("hkbTarget", 2),
            ClassVersion::new("hkbpTarget", 3),
        ),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("hkbTargetRigidBodyModifier", 3),
            ClassVersion::new("hkbpTargetRigidBodyModifier", 4),
        )
        .with_operation(PatchOperation::Depends {
            class_name: "hkbpTarget".to_string(),
            version: 3,
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("hkbReachTowardTargetModifierHand", 1),
            ClassVersion::new("hkbpReachTowardTargetModifierHand", 2),
        ),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("hkbReachTowardTargetModifier", 1),
            ClassVersion::new("hkbpReachTowardTargetModifier", 2),
        )
        .with_operation(PatchOperation::Depends {
            class_name: "hkbpTarget".to_string(),
            version: 3,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbpReachTowardTargetModifierHand".to_string(),
            version: 2,
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("hkbReachModifierHand", 0),
            ClassVersion::new("hkbpReachModifierHand", 1),
        ),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("hkbReachModifier", 0),
            ClassVersion::new("hkbpReachModifier", 1),
        )
        .with_operation(PatchOperation::Depends {
            class_name: "hkbpReachModifierHand".to_string(),
            version: 1,
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("hkbMoveBoneTowardTargetModifier", 2),
            ClassVersion::new("hkbpMoveBoneTowardTargetModifier", 3),
        )
        .with_operation(PatchOperation::Depends {
            class_name: "hkbpTarget".to_string(),
            version: 3,
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("hkbFaceTargetModifier", 1),
            ClassVersion::new("hkbpFaceTargetModifier", 2),
        )
        .with_operation(PatchOperation::Depends {
            class_name: "hkbpTarget".to_string(),
            version: 3,
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("hkbControlledReachModifier", 0),
            ClassVersion::new("hkbpControlledReachModifier", 1),
        ),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("hkbConstrainRigidBodyModifier", 1),
            ClassVersion::new("hkbpConstrainRigidBodyModifier", 2),
        )
        .with_operation(PatchOperation::Depends {
            class_name: "hkbpTarget".to_string(),
            version: 3,
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("hkbCheckRagdollSpeedModifier", 1),
            ClassVersion::new("hkbpCheckRagdollSpeedModifier", 2),
        ),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("hkbCheckBalanceModifier", 0),
            ClassVersion::new("hkbpCheckBalanceModifier", 1),
        ),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("hkbBalanceModifierStepInfo", 0),
            ClassVersion::new("hkbpBalanceModifierStepInfo", 1),
        ),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("hkbBalanceModifier", 0),
            ClassVersion::new("hkbpBalanceModifier", 1),
        )
        .with_operation(PatchOperation::Depends {
            class_name: "hkbpBalanceModifierStepInfo".to_string(),
            version: 1,
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("hkbCatchFallModifierHand", 1),
            ClassVersion::new("hkbpCatchFallModifierHand", 2),
        ),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("hkbCatchFallModifier", 1),
            ClassVersion::new("hkbpCatchFallModifier", 2),
        )
        .with_operation(PatchOperation::Depends {
            class_name: "hkbpCatchFallModifierHand".to_string(),
            version: 2,
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("hkbBalanceRadialSelectorGenerator", 0),
            ClassVersion::new("hkbpBalanceRadialSelectorGenerator", 1),
        )
        .with_operation(PatchOperation::Depends {
            class_name: "hkbpCheckBalanceModifier".to_string(),
            version: 1,
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkbAiControlControlDataNonBlendable", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "canControl".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(1)),
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkbAiControlControlDataBlendable", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "desiredSpeed".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: Some(PatchValue::Real(5.0_f32)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "maximumSpeed".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: Some(PatchValue::Real(5.0_f32)),
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkbAiControlControlData", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "blendable".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkbAiControlControlDataBlendable".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "nonBlendable".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkbAiControlControlDataNonBlendable".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbAiControlControlDataBlendable".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbAiControlControlDataNonBlendable".to_string(),
            version: 0,
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkbAiControlCancelPathCommand", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkReferencedObject".to_string()),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "characterId".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkBaseObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkbAiControlPathToCommand", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkReferencedObject".to_string()),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "characterId".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "goalPoint".to_string(),
            type_name: "vec4".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkBaseObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("hkbDemoConfigStickVariableInfo", 1),
            ClassVersion::new("", -2),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "variableName".to_string(),
            type_name: "string".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "minValue".to_string(),
            type_name: "real".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "maxValue".to_string(),
            type_name: "real".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "minStickValue".to_string(),
            type_name: "real".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "maxStickValue".to_string(),
            type_name: "real".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "stickAxis".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "stick".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "complimentVariableValue".to_string(),
            type_name: "int8".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "negateVariableValue".to_string(),
            type_name: "int8".to_string(),
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("hkbDemoConfigCharacterInfo", 3),
            ClassVersion::new("", -2),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: Some("hkReferencedObject".to_string()),
            new_parent: None,
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "overrideCharacterDataFilename".to_string(),
            type_name: "string".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "initialPosition".to_string(),
            type_name: "vec4".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "initialRotation".to_string(),
            type_name: "vec4".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "modelUpAxis".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "ragdollBoneLayers".to_string(),
            type_name: "array".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "overrideBehaviorFilename".to_string(),
            type_name: "string".to_string(),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkBaseObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("hkbDemoConfig", 5),
            ClassVersion::new("", -2),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: Some("hkReferencedObject".to_string()),
            new_parent: None,
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "characterInfo".to_string(),
            type_name: "array".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "terrainInfo".to_string(),
            type_name: "array".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "skinAttributeIndices".to_string(),
            type_name: "array".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "buttonPressToEventMap".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "buttonReleaseToEventMap".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "worldUpAxis".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "extraCharacterClones".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "numTracks".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "proxyHeight".to_string(),
            type_name: "real".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "proxyRadius".to_string(),
            type_name: "real".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "proxyOffset".to_string(),
            type_name: "real".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "rootPath".to_string(),
            type_name: "string".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "projectDataFilename".to_string(),
            type_name: "string".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "useAttachments".to_string(),
            type_name: "int8".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "useProxy".to_string(),
            type_name: "int8".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "useSkyBox".to_string(),
            type_name: "int8".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "useTrackingCamera".to_string(),
            type_name: "int8".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "accumulateMotion".to_string(),
            type_name: "int8".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "testCloning".to_string(),
            type_name: "int8".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "useSplineCompression".to_string(),
            type_name: "int8".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "gamePadToRotateTerrainAboutItsAxisMap".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "gamePadToAddRemoveCharacterMap".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "filter".to_string(),
            type_name: "struct".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "stickVariables".to_string(),
            type_name: "array".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "forceLoad".to_string(),
            type_name: "int8".to_string(),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkBaseObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbDemoConfigCharacterInfo".to_string(),
            version: 3,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbDemoConfigStickVariableInfo".to_string(),
            version: 1,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkpGroupFilter".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkpCollisionFilter".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbDemoConfigTerrainInfo".to_string(),
            version: 0,
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("hkbDemoConfigTerrainInfo", 0),
            ClassVersion::new("", -2),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "filename".to_string(),
            type_name: "string".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "layer".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "systemGroup".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "createDisplayObjects".to_string(),
            type_name: "int8".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "terrainRigidBody".to_string(),
            type_name: "struct".to_string(),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkpRigidBody".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkpEntity".to_string(),
            version: 3,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkBaseObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkpWorldObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        }),
    );
    // physics.py
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("hkpDeformableLinConstraintAtom", 0),
            ClassVersion::new("hkpDeformableLinConstraintAtom", 1),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "yieldStrengthDiag".to_string(),
            type_name: "vec4".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "yieldStrengthOffDiag".to_string(),
            type_name: "vec4".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "ultimateStrengthDiag".to_string(),
            type_name: "vec4".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "ultimateStrengthOffDiag".to_string(),
            type_name: "vec4".to_string(),
            ctype: None,
            default: None,
        })
        .with_custom_hook("_hkpDeformableLinConstraintAtom_0_to_1")
        .with_operation(PatchOperation::MemberRemove {
            name: "yieldStrength".to_string(),
            type_name: "struct".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "ultimateStrength".to_string(),
            type_name: "struct".to_string(),
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("hkpDeformableAngConstraintAtom", 0),
            ClassVersion::new("hkpDeformableAngConstraintAtom", 1),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "yieldStrengthDiag".to_string(),
            type_name: "vec4".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "yieldStrengthOffDiag".to_string(),
            type_name: "vec4".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "ultimateStrengthDiag".to_string(),
            type_name: "vec4".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "ultimateStrengthOffDiag".to_string(),
            type_name: "vec4".to_string(),
            ctype: None,
            default: None,
        })
        .with_custom_hook("_hkpDeformableAngConstraintAtom_0_to_1")
        .with_operation(PatchOperation::MemberRemove {
            name: "yieldStrength".to_string(),
            type_name: "struct".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "ultimateStrength".to_string(),
            type_name: "struct".to_string(),
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("hkpConvexVerticesShape", 4),
            ClassVersion::new("hkpConvexVerticesShape", 5),
        )
        .with_operation(PatchOperation::MemberRename {
            old_name: "rotatedVertices".to_string(),
            new_name: "rotatedVertices_old".to_string(),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "rotatedVertices".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_custom_hook("_hkpConvexVerticesShape_4_to_5")
        .with_operation(PatchOperation::MemberRemove {
            name: "rotatedVertices_old".to_string(),
            type_name: "array".to_string(),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkFourTransposedPointsf".to_string(),
            version: 0,
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("hkpConvexVerticesShape", 5),
            ClassVersion::new("hkpConvexVerticesShape", 6),
        )
        .with_operation(PatchOperation::MemberRename {
            old_name: "rotatedVertices".to_string(),
            new_name: "rotatedVertices_old".to_string(),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "rotatedVertices".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_custom_hook("_hkpConvexVerticesShape_5_to_6")
        .with_operation(PatchOperation::MemberRemove {
            name: "rotatedVertices_old".to_string(),
            type_name: "array".to_string(),
        }),
    );
    // cloth.py
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("hclMeshMeshDeformSetupObject", 1),
            ClassVersion::new("hclMeshMeshDeformSetupObject", 2),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "influenceRadiusPerVertex".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hclVertexFloatInput".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "useMeshTopology".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hclVertexFloatInput".to_string(),
            version: 0,
        })
        .with_custom_hook("_hclMeshMeshDeformSetupObject_1_to_2")
        .with_operation(PatchOperation::MemberRemove {
            name: "influenceRadius".to_string(),
            type_name: "real".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "inputTrianglesSubsetThreshold".to_string(),
            type_name: "real".to_string(),
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("hclLocalRangeSetupObject", 1),
            ClassVersion::new("hclLocalRangeSetupObject", 2),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "localRangeShape".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("hclLocalRangeConstraintSet", 2),
            ClassVersion::new("hclLocalRangeConstraintSet", 3),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "shapeType".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("hclStretchLinkSetupObject", 0),
            ClassVersion::new("hclStretchLinkSetupObject", 1),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "useStretchDirection".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "useMeshTopology".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("hclBoneSpaceDeformerThreeBlendEntryBlock", 0),
            ClassVersion::new("hclBoneSpaceDeformerThreeBlendEntryBlock", 1),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "padding".to_string(),
            type_name: "int8".to_string(),
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("hclBoneSpaceDeformerFourBlendEntryBlock", 0),
            ClassVersion::new("hclBoneSpaceDeformerFourBlendEntryBlock", 1),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "padding".to_string(),
            type_name: "int8".to_string(),
        }),
    );
    manager.register(
        46,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hclConvexPlanesShape", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hclShape".to_string()),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "planeEquations".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "localFromWorld".to_string(),
            type_name: "transform".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "worldFromLocal".to_string(),
            type_name: "transform".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "objAabb".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkAabb".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "geomCentroid".to_string(),
            type_name: "vec4".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hclShape".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkBaseObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkAabb".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        }),
    );
}
