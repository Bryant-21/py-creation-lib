// Native Havok conversion corpus. Hand-maintained.

use super::super::manager::PatchManager;
use super::super::ops::{ClassVersion, Patch, PatchOperation};

pub(super) fn register(manager: &mut PatchManager) {
    // version_id = 56
    // common.py
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkPseudoRandomGenerator", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "seed".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "current".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        56,
        Patch::new(ClassVersion::new("", -1), ClassVersion::new("hkFrustum", 0))
            .with_operation(PatchOperation::MemberAdd {
                name: "planes".to_string(),
                type_name: "array".to_string(),
                ctype: Some("hkPlane".to_string()),
                default: None,
            })
            .with_operation(PatchOperation::Depends {
                class_name: "hkPlane".to_string(),
                version: 0,
            }),
    );
    manager.register(
        56,
        Patch::new(ClassVersion::new("", -1), ClassVersion::new("hkPlane", 0)).with_operation(
            PatchOperation::MemberAdd {
                name: "equation".to_string(),
                type_name: "vec4".to_string(),
                ctype: None,
                default: None,
            },
        ),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("hkMemoryMeshVertexBuffer", 1),
            ClassVersion::new("hkMemoryMeshVertexBuffer", 2),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "locked".to_string(),
            type_name: "int8".to_string(),
        }),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkPropertyDesc", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "name".to_string(),
            type_name: "string".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkPropertyId", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "name".to_string(),
            type_name: "string".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkHashMap< hkPropertyId, hkReflect::Var >", 0),
        ),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("hkHashMap< hkPropertyBag::Key, hkReflect::Var >", 0),
            ClassVersion::new("", -2),
        ),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkHashMap< hkPropertyId, hkReflect::Any >", 0),
        ),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkDefaultPropertyBag", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkPropertyBag".to_string()),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "propertyMap".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkHashMap< hkPropertyId, hkReflect::Var >".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkHashMap< hkPropertyId, hkReflect::Var >".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkPropertyBag".to_string(),
            version: 0,
        }),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkPropertyBag", 0),
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
        56,
        Patch::new(
            ClassVersion::new("hkDefaultPropertyBag", 0),
            ClassVersion::new("hkDefaultPropertyBag", 1),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "propertyMap".to_string(),
            type_name: "struct".to_string(),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "propertyMap".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkHashMap< hkPropertyId, hkReflect::Any >".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkHashMap< hkPropertyId, hkReflect::Var >".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkHashMap< hkPropertyId, hkReflect::Any >".to_string(),
            version: 0,
        }),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("hkHashMap< hkPropertyId, hkReflect::Var >", 0),
            ClassVersion::new("", -2),
        ),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("hkReferencedObject", 0),
            ClassVersion::new("hkReferencedObject", 1),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "propertyBag".to_string(),
            type_name: "pointer".to_string(),
            ctype: Some("hkPropertyBag".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkPropertyBag".to_string(),
            version: 0,
        }),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkColorUbBase", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "r".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "g".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "b".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "a".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkColorUbGamma", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkColorUbBase".to_string()),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkColorUbBase".to_string(),
            version: 0,
        }),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkColorUbLinear", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkColorUbBase".to_string()),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkColorUbBase".to_string(),
            version: 0,
        }),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkViewport", 0),
        ),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkReflect::TypeName", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "name".to_string(),
            type_name: "string".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("hkCustomAttributes", 0),
            ClassVersion::new("", -2),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "attributes".to_string(),
            type_name: "array".to_string(),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkCustomAttributesAttribute".to_string(),
            version: 0,
        }),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("hkCustomAttributesAttribute", 0),
            ClassVersion::new("", -2),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "value".to_string(),
            type_name: "array".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "name".to_string(),
            type_name: "string".to_string(),
        }),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("hkArrayTypeAttribute", 0),
            ClassVersion::new("", -2),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "type".to_string(),
            type_name: "int".to_string(),
        }),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("hkCamera3d", 0),
            ClassVersion::new("", -2),
        ),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("hkCameraData", 0),
            ClassVersion::new("", -2),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "handedness".to_string(),
            type_name: "int8".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "isOrthographic".to_string(),
            type_name: "int8".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "far".to_string(),
            type_name: "real".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "near".to_string(),
            type_name: "real".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "fovyDegrees".to_string(),
            type_name: "real".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "up".to_string(),
            type_name: "vec4".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "to".to_string(),
            type_name: "vec4".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "from".to_string(),
            type_name: "vec4".to_string(),
        }),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new(
                "hkOffsetBitFieldStoragehkArrayunsignedinthkContainerHeapAllocator",
                0,
            ),
            ClassVersion::new(
                "hkOffsetBitFieldStorage< hkArray< hkUint32, hkContainerHeapAllocator > >",
                0,
            ),
        ),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("hknpMaterial::FreeListArrayOperations", 0),
            ClassVersion::new("", -2),
        ),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("hknpMotionProperties::FreeListArrayOperations", 0),
            ClassVersion::new("", -2),
        ),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("hkTraceStream", 0),
            ClassVersion::new("", -2),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "titles".to_string(),
            type_name: "array".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "counter".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkTraceStream::Title".to_string(),
            version: 0,
        }),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkHandle< hkUint32, 2147483647 >", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "value".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("hkReferencedObject", 1),
            ClassVersion::new("hkReferencedObject", 2),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "dynamicProperties".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hkDefaultPropertyBag".to_string()),
            default: None,
        })
        .with_custom_hook("_hkReferencedObject_1_to_2")
        .with_operation(PatchOperation::MemberRemove {
            name: "propertyBag".to_string(),
            type_name: "pointer".to_string(),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkPropertyBag".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkDefaultPropertyBag".to_string(),
            version: 1,
        }),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("hkReferencedObject", 2),
            ClassVersion::new("hkReferencedObject", 3),
        )
        .with_operation(PatchOperation::MemberRename {
            old_name: "dynamicProperties".to_string(),
            new_name: "propertyBag".to_string(),
        }),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkPropertyFlags", 0),
        ),
    );
    // behavior.py
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("hkbNullPhysicsInterface", 0),
            ClassVersion::new("", -2),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: Some("hkbPhysicsInterface".to_string()),
            new_parent: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbPhysicsInterface".to_string(),
            version: 0,
        }),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("hkbPhysicsInterface", 0),
            ClassVersion::new("", -2),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: Some("hkReferencedObject".to_string()),
            new_parent: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 1,
        }),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("hkbAiDriverSetup", 0),
            ClassVersion::new("", -2),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "driverInfo".to_string(),
            type_name: "pointer".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "character".to_string(),
            type_name: "pointer".to_string(),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbAiDriverInfo".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbCharacter".to_string(),
            version: 4,
        }),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("hkbAiDriver", 0),
            ClassVersion::new("", -2),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: Some("hkReferencedObject".to_string()),
            new_parent: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 1,
        }),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("hkbpPhysicsInterface", 0),
            ClassVersion::new("", -2),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: Some("hkbPhysicsInterface".to_string()),
            new_parent: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbPhysicsInterface".to_string(),
            version: 0,
        }),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("hkbnpPhysicsInterface", 0),
            ClassVersion::new("", -2),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: Some("hkbPhysicsInterface".to_string()),
            new_parent: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbPhysicsInterface".to_string(),
            version: 0,
        }),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("hkbBodyIkControlBits", 0),
            ClassVersion::new("", -2),
        ),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("hkbBodyIkTaskList", 0),
            ClassVersion::new("", -2),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "animationInfluences".to_string(),
            type_name: "array".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "tasks".to_string(),
            type_name: "array".to_string(),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbBodyIkTask".to_string(),
            version: 0,
        }),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("hkbBodyIkTask", 0),
            ClassVersion::new("", -2),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "effectorsOffsetLS".to_string(),
            type_name: "vec4".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "effectors".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "targetRotationWeight".to_string(),
            type_name: "real".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "targetRotationMS".to_string(),
            type_name: "vec4".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "targetPositionWeight".to_string(),
            type_name: "real".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "targetPositionMS".to_string(),
            type_name: "vec4".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "taskInfluenceDistance".to_string(),
            type_name: "int8".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "priority".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "boneIdx".to_string(),
            type_name: "int".to_string(),
        }),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("hkbBodyIkControlPriority", 0),
            ClassVersion::new("", -2),
        ),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("hkbBodyIkControllerSetup", 0),
            ClassVersion::new("", -2),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "controllerCinfo".to_string(),
            type_name: "pointer".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "skeleton".to_string(),
            type_name: "pointer".to_string(),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbBodyIkControllerCinfo".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkaSkeleton".to_string(),
            version: 6,
        }),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("hkbNullBodyIkInterface", 0),
            ClassVersion::new("", -2),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: Some("hkbBodyIkInterface".to_string()),
            new_parent: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbBodyIkInterface".to_string(),
            version: 0,
        }),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("hkbBodyIkInterface", 0),
            ClassVersion::new("", -2),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: Some("hkReferencedObject".to_string()),
            new_parent: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 1,
        }),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("hkbAiInterface", 0),
            ClassVersion::new("", -2),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: Some("hkReferencedObject".to_string()),
            new_parent: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 1,
        }),
    );
    // physics.py
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new(
                "hkpGroupFilterBase< hkpGroupFilterTypes::Config< 5, 5, 5, 16 > >",
                0,
            ),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkpCollisionFilter".to_string()),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "nextFreeSystemGroup".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "collisionLookupTable".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkBaseObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkpCollisionFilter".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        }),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new(
                "hkpGroupFilterBase< hkpGroupFilterTypes::Config< 6, 5, 5, 16 > >",
                0,
            ),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkpCollisionFilter".to_string()),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "nextFreeSystemGroup".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "collisionLookupTable".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkBaseObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkpCollisionFilter".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        }),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("hkpGroupFilter", 0),
            ClassVersion::new("hkpGroupFilter", 1),
        )
        .with_operation(PatchOperation::MemberRename {
            old_name: "nextFreeSystemGroup".to_string(),
            new_name: "old_nextFreeSystemGroup".to_string(),
        })
        .with_operation(PatchOperation::MemberRename {
            old_name: "collisionLookupTable".to_string(),
            new_name: "old_collisionLookupTable".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "pad256".to_string(),
            type_name: "vec4".to_string(),
        })
        .with_operation(PatchOperation::ParentSet {
            old_parent: Some("hkpCollisionFilter".to_string()),
            new_parent: Some(
                "hkpGroupFilterBase< hkpGroupFilterTypes::Config< 5, 5, 5, 16 > >".to_string(),
            ),
        })
        .with_custom_hook("_hkpGroupFilter_0_to_1")
        .with_operation(PatchOperation::MemberRemove {
            name: "old_nextFreeSystemGroup".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "old_collisionLookupTable".to_string(),
            type_name: "array".to_string(),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkpGroupFilterBase< hkpGroupFilterTypes::Config< 5, 5, 5, 16 > >"
                .to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkpCollisionFilter".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        }),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkpGroupFilter64Layers", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some(
                "hkpGroupFilterBase< hkpGroupFilterTypes::Config< 6, 5, 5, 16 > >".to_string(),
            ),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkBaseObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkpCollisionFilter".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkpGroupFilterBase< hkpGroupFilterTypes::Config< 6, 5, 5, 16 > >"
                .to_string(),
            version: 0,
        }),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("hkpCollidable", 2),
            ClassVersion::new("hkpCollidable", 3),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "forceCollideOntoPpu".to_string(),
            type_name: "int8".to_string(),
        }),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("hknpMaterial", 2),
            ClassVersion::new("hknpMaterial", 3),
        )
        .with_operation(PatchOperation::MemberRename {
            old_name: "softContactSeperationVelocity".to_string(),
            new_name: "softContactSeparationVelocity".to_string(),
        }),
    );
    // physics_np.py
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("hknpWorldCinfo", 9),
            ClassVersion::new("hknpWorldCinfo", 10),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "enableCollideWorkStealing".to_string(),
            type_name: "bool".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "unitScale".to_string(),
            type_name: "real".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "applyUnitScaleToStaticConstants".to_string(),
            type_name: "bool".to_string(),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "enableSdfEdgeCollisions".to_string(),
            type_name: "bool".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "constraintGroupBufferCapacity".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "deleteCachesOnDeactivation".to_string(),
            type_name: "bool".to_string(),
        }),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("hknpBody", 6),
            ClassVersion::new("hknpBody", 7),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "indexIntoActiveListOrDeactivatedIslandId".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "indexIntoActiveList".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("hknpConstraintId", 0),
            ClassVersion::new("hknpConstraintId", 1),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkHandle< hkUint32, 2147483647 >".to_string()),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "value".to_string(),
            type_name: "int".to_string(),
        }),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("hknpShapeInstance", 1),
            ClassVersion::new("hknpShapeInstance", 2),
        ),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("hkFreeListArrayElement< hknpShapeInstance >", 1),
            ClassVersion::new("hkFreeListArrayElement< hknpShapeInstance >", 2),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hknpShapeInstance".to_string()),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "destructionTag_copy".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "padding_copy".to_string(),
            type_name: "array".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "shapeTag_copy".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "shape_copy".to_string(),
            type_name: "pointer".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "scale_copy".to_string(),
            type_name: "vec4".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "transform_copy".to_string(),
            type_name: "vec16".to_string(),
        }),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("hknpMotionProperties", 3),
            ClassVersion::new("hknpMotionProperties", 4),
        ),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("hkFreeListArrayElement< hknpMotionProperties >", 3),
            ClassVersion::new("hkFreeListArrayElement< hknpMotionProperties >", 4),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hknpMotionProperties".to_string()),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "timeFactor_copy".to_string(),
            type_name: "real".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "flags_copy".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "maxAngularSpeed_copy".to_string(),
            type_name: "real".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "maxLinearSpeed_copy".to_string(),
            type_name: "real".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "minimumSpikingVelocityScaleSquared_copy".to_string(),
            type_name: "uint8".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "spikingVelocityScaleThresholdSquared_copy".to_string(),
            type_name: "uint8".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "minimumPathingVelocityScaleSquare_copy".to_string(),
            type_name: "uint8".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "deactivationVelocityScaleSquare_copy".to_string(),
            type_name: "uint8".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "numDeactivationFrequencyPasses_copy".to_string(),
            type_name: "uint8".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "pathingLowerThreshold_copy".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "pathingUpperThreshold_copy".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "invBlockSize_copy".to_string(),
            type_name: "real".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "maxRotSqrd_copy".to_string(),
            type_name: "real".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "maxDistSqrd_copy".to_string(),
            type_name: "real".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "solverStabilizationSpeedReduction_copy".to_string(),
            type_name: "real".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "solverStabilizationSpeedThreshold_copy".to_string(),
            type_name: "real".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "gravityFactor_copy".to_string(),
            type_name: "real".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "angularDamping_copy".to_string(),
            type_name: "real".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "linearDamping_copy".to_string(),
            type_name: "real".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "isExclusive_copy".to_string(),
            type_name: "int".to_string(),
        }),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("hknpMaterial", 2),
            ClassVersion::new("hknpMaterial", 3),
        )
        .with_operation(PatchOperation::MemberRename {
            old_name: "softContactSeperationVelocity".to_string(),
            new_name: "softContactSeparationVelocity".to_string(),
        }),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("hkFreeListArrayElement< hknpMaterial >", 2),
            ClassVersion::new("hkFreeListArrayElement< hknpMaterial >", 3),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: Some("hkReferencedObject".to_string()),
            new_parent: Some("hknpMaterial".to_string()),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "userData_copy".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "fractionOfClippedImpulseToApply_copy".to_string(),
            type_name: "real".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "restitution_copy".to_string(),
            type_name: "real".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "staticFriction_copy".to_string(),
            type_name: "real".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "dynamicFriction_copy".to_string(),
            type_name: "real".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "isShared_copy".to_string(),
            type_name: "uint8".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "disablingCollisionsBetweenCvxCvxDynamicObjectsDistance_copy".to_string(),
            type_name: "real".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "surfaceVelocity_copy".to_string(),
            type_name: "pointer".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "softContactSeperationVelocity_copy".to_string(),
            type_name: "struct".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "softContactDampFactor_copy".to_string(),
            type_name: "real".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "softContactForceFactor_copy".to_string(),
            type_name: "real".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "maxContactImpulse_copy".to_string(),
            type_name: "real".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "massChangerHeavyObjectFactor_copy".to_string(),
            type_name: "real".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "massChangerCategory_copy".to_string(),
            type_name: "uint8".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "triggerManifoldTolerance_copy".to_string(),
            type_name: "struct".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "triggerType_copy".to_string(),
            type_name: "uint8".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "weldingTolerance_copy".to_string(),
            type_name: "real".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "restitutionCombinePolicy_copy".to_string(),
            type_name: "uint8".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "frictionCombinePolicy_copy".to_string(),
            type_name: "uint8".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "flags_copy".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "isExclusive_copy".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "name_copy".to_string(),
            type_name: "string".to_string(),
        }),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("hknpLodMeshShape", 4),
            ClassVersion::new("hknpLodMeshShape", 5),
        ),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("hknpLodShape", 4),
            ClassVersion::new("hknpLodShape", 5),
        ),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("hknpCompoundShapeBase", 3),
            ClassVersion::new("hknpCompoundShapeBase", 4),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "instanceVelocities".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hknpCompoundShapeBase::VelocityInfo".to_string()),
            default: None,
        }),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("hknpConstraint", 4),
            ClassVersion::new("hknpConstraint", 5),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "groupId".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hknpConstraintGroupId".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "nextInGroup".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hknpConstraintId".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "prevInGroup".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hknpConstraintId".to_string()),
            default: None,
        }),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("hknpConstraintCinfo", 4),
            ClassVersion::new("hknpConstraintCinfo", 5),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "constraintGroupId".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hknpConstraintGroupId".to_string()),
            default: None,
        })
        .with_custom_hook("hknpConstraintCinfo_4_to_5"),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("hknpWorldSnapshot", 0),
            ClassVersion::new("hknpWorldSnapshot", 1),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "constraintGroupInfos".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hknpWorldSnapshot::ConstraintGroupInfo".to_string()),
            default: None,
        }),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("hknpMaterialData", 0),
            ClassVersion::new("hknpMaterialData", 1),
        )
        .with_operation(PatchOperation::MemberRename {
            old_name: "softContactSeperationVelocity".to_string(),
            new_name: "softContactSeparationVelocity".to_string(),
        }),
    );
    // cloth.py
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("hclBendStiffnessConstraintSet", 1),
            ClassVersion::new("hclBendStiffnessConstraintSet", 2),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "clampBendStiffness".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "maxRestPoseHeightSq".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("hclBendStiffnessConstraintSetMx", 0),
            ClassVersion::new("hclBendStiffnessConstraintSetMx", 1),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "clampBendStiffness".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "maxRestPoseHeightSq".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        56,
        Patch::new(
            ClassVersion::new("hclStateOperatorMask", 0),
            ClassVersion::new("", -2),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: Some("hkReferencedObject".to_string()),
            new_parent: None,
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "usedTransformSets".to_string(),
            type_name: "array".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "usedBuffers".to_string(),
            type_name: "array".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "operatorStepMask".to_string(),
            type_name: "array".to_string(),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 1,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hclClothState::TransformSetAccess".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hclClothState::BufferAccess".to_string(),
            version: 2,
        }),
    );
}
