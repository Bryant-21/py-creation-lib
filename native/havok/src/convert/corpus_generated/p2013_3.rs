// Native Havok conversion corpus. Hand-maintained.

use super::super::manager::PatchManager;
use super::super::ops::{ClassVersion, Patch, PatchOperation, PatchValue};

pub(super) fn register(manager: &mut PatchManager) {
    // version_id = 52
    // common.py
    manager.register(
        52,
        Patch::new(
            ClassVersion::new("hkxVertexDescriptionElementDecl", 3),
            ClassVersion::new("hkxVertexDescriptionElementDecl", 4),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "hint".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "channelID".to_string(),
            type_name: "string".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        52,
        Patch::new(
            ClassVersion::new("hkxMeshSection", 4),
            ClassVersion::new("hkxMeshSection", 5),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "boneMatrixMap".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hkMeshBoneIndexMapping".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkMeshBoneIndexMapping".to_string(),
            version: 0,
        }),
    );
    manager.register(
        52,
        Patch::new(
            ClassVersion::new("hkMeshSectionCinfo", 1),
            ClassVersion::new("hkMeshSectionCinfo", 2),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "boneMatrixMap".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkMeshBoneIndexMapping".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkMeshBoneIndexMapping".to_string(),
            version: 0,
        }),
    );
    manager.register(
        52,
        Patch::new(
            ClassVersion::new("hkMeshSection", 1),
            ClassVersion::new("hkMeshSection", 2),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "boneMatrixMap".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkMeshBoneIndexMapping".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkMeshBoneIndexMapping".to_string(),
            version: 0,
        }),
    );
    manager.register(
        52,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkMemoryMeshShapeSection", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "vertexBuffer".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkMeshVertexBuffer".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "material".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkMeshMaterial".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "boneMatrixMap".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkMeshBoneIndexMapping".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "primitiveType".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "numPrimitives".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "indexType".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "vertexStartIndex".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "transformIndex".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "indexBufferOffset".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkMeshBoneIndexMapping".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkBaseObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkMeshVertexBuffer".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkMeshMaterial".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        }),
    );
    manager.register(
        52,
        Patch::new(
            ClassVersion::new("hkMemoryMeshShape", 0),
            ClassVersion::new("hkMemoryMeshShape", 1),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "sections".to_string(),
            type_name: "array".to_string(),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "sections".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hkMemoryMeshShapeSection".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkMeshSectionCinfo".to_string(),
            version: 2,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkMemoryMeshShapeSection".to_string(),
            version: 0,
        }),
    );
    manager.register(
        52,
        Patch::new(
            ClassVersion::new("hkUiAttribute", 3),
            ClassVersion::new("hkUiAttribute", 4),
        )
        .with_operation(PatchOperation::MemberRename {
            old_name: "hideInModeler".to_string(),
            new_name: "hideCriteria".to_string(),
        }),
    );
    manager.register(
        52,
        Patch::new(
            ClassVersion::new("hkxVertexBufferVertexData", 1),
            ClassVersion::new("hkxVertexBufferVertexData", 2),
        )
        .with_operation(PatchOperation::MemberRename {
            old_name: "floatData".to_string(),
            new_name: "floatDataOld".to_string(),
        })
        .with_operation(PatchOperation::MemberRename {
            old_name: "vectorData".to_string(),
            new_name: "vectorDataOld".to_string(),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "floatData".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "vectorData".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_custom_hook("_hkxVertexBufferVertexData_1_to_2")
        .with_operation(PatchOperation::MemberRemove {
            name: "floatDataOld".to_string(),
            type_name: "array".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "vectorDataOld".to_string(),
            type_name: "array".to_string(),
        }),
    );
    manager.register(
        52,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkQTransform", 2),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkQTransformf".to_string()),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkQTransformf".to_string(),
            version: 0,
        }),
    );
    // behavior.py
    manager.register(
        52,
        Patch::new(
            ClassVersion::new("hkbFootIkControlData", 0),
            ClassVersion::new("hkbFootIkControlData", 1),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "enabled".to_string(),
            type_name: "vec4".to_string(),
            ctype: None,
            default: None,
        })
        .with_custom_hook("_hkbFootIkControlData_0_to_1"),
    );
    manager.register(
        52,
        Patch::new(
            ClassVersion::new("hkbFootIkControlsModifierLeg", 0),
            ClassVersion::new("hkbFootIkControlsModifierLeg", 1),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "enabled".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(1)),
        }),
    );
    manager.register(
        52,
        Patch::new(
            ClassVersion::new("hkbFootIkDriverInfo", 0),
            ClassVersion::new("hkbFootIkDriverInfo", 1),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "keepSourceFootEndAboveGround".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(1)),
        }),
    );
    manager.register(
        52,
        Patch::new(
            ClassVersion::new("hkbFootIkModifier", 3),
            ClassVersion::new("hkbFootIkModifier", 4),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "keepSourceFootEndAboveGround".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(1)),
        }),
    );
    manager.register(
        52,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkbCustomTestGeneratorHiddenTypes", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkbReferencePoseGenerator".to_string()),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "inheritedHiddenMember".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "protectedInheritedHiddenMember".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "privateInheritedHiddenMember".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
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
            class_name: "hkbReferencePoseGenerator".to_string(),
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
        52,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkbCustomTestGeneratorSimpleTypes", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkbCustomTestGeneratorHiddenTypes".to_string()),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "simpleHiddenTypeCopyStart".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "simpleTypeBool".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "simpleTypeHkBool".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "simpleTypeCString".to_string(),
            type_name: "string".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "simpleTypeHkStringPtr".to_string(),
            type_name: "string".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "simpleTypeHkInt8".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "simpleTypeHkInt16".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "simpleTypeHkInt32".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "simpleTypeHkUint8".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "simpleTypeHkUint16".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "simpleTypeHkUint32".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "simpleTypeHkReal".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "simpleTypeHkInt8Default".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(1)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "simpleTypeHkInt16Default".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(1)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "simpleTypeHkInt32Default".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(1)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "simpleTypeHkUint8Default".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(1)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "simpleTypeHkUint16Default".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(1)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "simpleTypeHkUint32Default".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(1)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "simpleTypeHkRealDefault".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: Some(PatchValue::Real(1.0_f32)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "simpleTypeHkInt8Clamp".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "simpleTypeHkInt16Clamp".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "simpleTypeHkInt32Clamp".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "simpleTypeHkUint8Clamp".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "simpleTypeHkUint16Clamp".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "simpleTypeHkUint32Clamp".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "simpleTypeHkRealClamp".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "simpleTypeHkInt64".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "simpleTypeHkUint64".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "simpleHiddenTypeCopyEnd".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
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
            class_name: "hkbReferencePoseGenerator".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbBindable".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbCustomTestGeneratorHiddenTypes".to_string(),
            version: 0,
        }),
    );
    manager.register(
        52,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkbCustomTestGeneratorComplexTypes", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkbCustomTestGeneratorSimpleTypes".to_string()),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "complexTypeHkObjectPtr".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkReferencedObject".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "complexHiddenTypeCopyStart".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "complexTypeHkQuaternion".to_string(),
            type_name: "vec4".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "complexTypeHkVector4".to_string(),
            type_name: "vec4".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "complexTypeEnumHkInt8".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "complexTypeEnumHkInt16".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "complexTypeEnumHkInt32".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "complexTypeEnumHkUint8".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "complexTypeEnumHkUint16".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "complexTypeEnumHkUint32".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "complexTypeEnumHkInt8InvalidCheck".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "complexTypeEnumHkInt16InvalidCheck".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "complexTypeEnumHkInt32InvalidCheck".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "complexTypeEnumHkUint8InvalidCheck".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "complexTypeEnumHkUint16InvalidCheck".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "complexTypeEnumHkUint32InvalidCheck".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "complexTypeFlagsHkInt8".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "complexTypeFlagsHkInt16".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "complexTypeFlagsHkInt32".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "complexTypeFlagsHkUint8".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "complexTypeFlagsHkUint16".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "complexTypeFlagsHkUint32".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "complexTypeFlagsHkInt8InvalidCheck".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "complexTypeFlagsHkInt16InvalidCheck".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "complexTypeFlagsHkInt32InvalidCheck".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "complexTypeFlagsHkUint8InvalidCheck".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "complexTypeFlagsHkUint16InvalidCheck".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "complexTypeFlagsHkUint32InvalidCheck".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "complexHiddenTypeCopyEnd".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
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
            class_name: "hkbCustomTestGeneratorSimpleTypes".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbReferencePoseGenerator".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbBindable".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbCustomTestGeneratorHiddenTypes".to_string(),
            version: 0,
        }),
    );
    manager.register(
        52,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkbCustomTestGeneratorNestedTypesBase", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkbCustomTestGeneratorComplexTypes".to_string()),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "nestedTypeHkbGeneratorPtr".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkbGenerator".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "nestedTypeHkbGeneratorRefPtr".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkbGenerator".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "nestedTypeHkbModifierPtr".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkbModifier".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "nestedTypeHkbModifierRefPtr".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkbModifier".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "nestedTypeHkbCustomIdSelectorPtr".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkbCustomIdSelector".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "nestedTypeHkbCustomIdSelectorRefPtr".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkbCustomIdSelector".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "nestedTypeArrayBool".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "nestedTypeArrayHkBool".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "nestedTypeArrayCString".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "nestedTypeArrayHkStringPtr".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "nestedTypeArrayHkInt8".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "nestedTypeArrayHkInt16".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "nestedTypeArrayHkInt32".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "nestedTypeArrayHkUint8".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "nestedTypeArrayHkUint16".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "nestedTypeArrayHkUint32".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "nestedTypeArrayHkReal".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "nestedTypeArrayHkbGeneratorPtr".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hkbGenerator".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "nestedTypeArrayHkbGeneratorRefPtr".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hkbGenerator".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "nestedTypeArrayHkbModifierPtr".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hkbModifier".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "nestedTypeArrayHkbModifierRefPtr".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hkbModifier".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "nestedTypeArrayHkbCustomIdSelectorPtr".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hkbCustomIdSelector".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "nestedTypeArrayHkbCustomIdSelectorRefPtr".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hkbCustomIdSelector".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbNode".to_string(),
            version: 1,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbCustomIdSelector".to_string(),
            version: 0,
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
            class_name: "hkbCustomTestGeneratorSimpleTypes".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbReferencePoseGenerator".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbBindable".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbModifier".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbCustomTestGeneratorComplexTypes".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbCustomTestGeneratorHiddenTypes".to_string(),
            version: 0,
        }),
    );
    manager.register(
        52,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkbCustomTestGeneratorNestedTypes", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkbCustomTestGeneratorNestedTypesBase".to_string()),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "nestedTypeStruct".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkbCustomTestGeneratorNestedTypesBase".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "nestedTypeArrayStruct".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hkbCustomTestGeneratorNestedTypesBase".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbNode".to_string(),
            version: 1,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbCustomTestGeneratorNestedTypesBase".to_string(),
            version: 0,
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
            class_name: "hkbCustomTestGeneratorSimpleTypes".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbReferencePoseGenerator".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbBindable".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbCustomTestGeneratorComplexTypes".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbCustomTestGeneratorHiddenTypes".to_string(),
            version: 0,
        }),
    );
    manager.register(
        52,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkbCustomTestGeneratorBoneTypes", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkbCustomTestGeneratorNestedTypes".to_string()),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "boneHiddenTypeCopyStart".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "oldBoneIndex".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "oldBoneIndexNoVar".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "boneIndex".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "boneIndexNoVar".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "boneChainIndex0".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "boneChainIndex1".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "boneChainIndex2".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "boneContractIndex0".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "boneContractIndex1".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "boneContractIndex2".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "boneHiddenTypeCopyEnd".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "boneWeightArray".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkbBoneWeightArray".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "boneIndexArray".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkbBoneIndexArray".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbNode".to_string(),
            version: 1,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbCustomTestGeneratorNestedTypesBase".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbBoneIndexArray".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbCustomTestGeneratorNestedTypes".to_string(),
            version: 0,
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
            class_name: "hkbCustomTestGeneratorSimpleTypes".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbReferencePoseGenerator".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbBindable".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbCustomTestGeneratorComplexTypes".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbCustomTestGeneratorHiddenTypes".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbBoneWeightArray".to_string(),
            version: 0,
        }),
    );
    manager.register(
        52,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkbCustomTestGeneratorAnnotatedTypes", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkbCustomTestGeneratorBoneTypes".to_string()),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "annotatedTypeCStringFilename".to_string(),
            type_name: "string".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "annotatedTypeHkStringPtrFilename".to_string(),
            type_name: "string".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "annotatedTypeCStringScript".to_string(),
            type_name: "string".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "annotatedTypeHkStringPtrScript".to_string(),
            type_name: "string".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "annotatedTypeCStringBoneAttachment".to_string(),
            type_name: "string".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "annotatedTypeHkStringPtrBoneAttachment".to_string(),
            type_name: "string".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "annotatedTypeCStringLocalFrame".to_string(),
            type_name: "string".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "annotatedTypeHkStringPtrLocalFrame".to_string(),
            type_name: "string".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "annotatedHiddenTypeCopyStart".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "annotatedTypeHkInt32EventID".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "annotatedTypeHkInt32VariableIndex".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "annotatedTypeHkInt32AttributeIndex".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "annotatedTypeHkRealTime".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "annotatedTypeBoolNoVar".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "annotatedTypeHkBoolNoVar".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "annotatedTypeHkInt8NoVar".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "annotatedTypeHkInt16NoVar".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "annotatedTypeHkInt32NoVar".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "annotatedTypeHkUint8NoVar".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "annotatedTypeHkUint16NoVar".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "annotatedTypeHkUint32NoVar".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "annotatedTypeHkRealNoVar".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "annotatedTypeBoolOutput".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "annotatedTypeHkBoolOutput".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "annotatedTypeHkInt8Output".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "annotatedTypeHkInt16Output".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "annotatedTypeHkInt32Output".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "annotatedTypeHkUint8Output".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "annotatedTypeHkUint16Output".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "annotatedTypeHkUint32Output".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "annotatedTypeHkRealOutput".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "annotatedHiddenTypeBool".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "annotatedHiddenTypeHkBool".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "annotatedHiddenTypeCString1".to_string(),
            type_name: "string".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "annotatedHiddenTypeHkStringPtr1".to_string(),
            type_name: "string".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "annotatedHiddenTypeCString2".to_string(),
            type_name: "string".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "annotatedHiddenTypeHkStringPtr2".to_string(),
            type_name: "string".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "annotatedHiddenTypeHkInt8".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "annotatedHiddenTypeHkInt16".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "annotatedHiddenTypeHkInt32".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "annotatedHiddenTypeHkUint8".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "annotatedHiddenTypeHkUint16".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "annotatedHiddenTypeHkUint32".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "annotatedHiddenTypeCopyEnd".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbNode".to_string(),
            version: 1,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbCustomTestGeneratorNestedTypesBase".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbCustomTestGeneratorBoneTypes".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbCustomTestGeneratorNestedTypes".to_string(),
            version: 0,
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
            class_name: "hkbCustomTestGeneratorSimpleTypes".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbReferencePoseGenerator".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbBindable".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbCustomTestGeneratorComplexTypes".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbCustomTestGeneratorHiddenTypes".to_string(),
            version: 0,
        }),
    );
    manager.register(
        52,
        Patch::new(
            ClassVersion::new("hkbCustomTestGenerator", 3),
            ClassVersion::new("hkbCustomTestGenerator", 4),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: Some("hkbReferencePoseGenerator".to_string()),
            new_parent: Some("hkbCustomTestGeneratorAnnotatedTypes".to_string()),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "hkBool".to_string(),
            type_name: "int8".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "string".to_string(),
            type_name: "string".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "int".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "hkInt8".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "hkInt16".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "hkInt32".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "hkUint8".to_string(),
            type_name: "int8".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "hkUint16".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "hkUint32".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "hkReal".to_string(),
            type_name: "real".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "hkVector4".to_string(),
            type_name: "vec4".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "hkQuaternion".to_string(),
            type_name: "vec4".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "mode_hkInt8".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "mode_hkInt16".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "mode_hkInt32".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "mode_hkUint8".to_string(),
            type_name: "int8".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "mode_hkUint16".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "mode_hkUint32".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "flags_hkInt8".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "flags_hkInt16".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "flags_hkInt32".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "flags_hkUint8".to_string(),
            type_name: "int8".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "flags_hkUint16".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "flags_hkUint32".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "myInt".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "generator1".to_string(),
            type_name: "struct".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "generator2".to_string(),
            type_name: "struct".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "modifier1".to_string(),
            type_name: "struct".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "modifier2".to_string(),
            type_name: "struct".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "array_hkBool".to_string(),
            type_name: "array".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "array_int".to_string(),
            type_name: "array".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "array_hkInt8".to_string(),
            type_name: "array".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "array_hkInt16".to_string(),
            type_name: "array".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "array_hkInt32".to_string(),
            type_name: "array".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "array_hkUint8".to_string(),
            type_name: "array".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "array_hkUint16".to_string(),
            type_name: "array".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "array_hkUint32".to_string(),
            type_name: "array".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "array_hkReal".to_string(),
            type_name: "array".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "array_hkbGenerator".to_string(),
            type_name: "array".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "array_hkbModifier".to_string(),
            type_name: "array".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "hkRigidBody".to_string(),
            type_name: "struct".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "boneIndexOld".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "boneIndex".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "boneChainIndex0".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "boneChainIndex1".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "boneChainIndex2".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "boneGroupIndex0".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "boneGroupIndex1".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "boneGroupIndex2".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "boneWeightArray".to_string(),
            type_name: "struct".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "boneIndexArray".to_string(),
            type_name: "struct".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "idSelector".to_string(),
            type_name: "struct".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "Struck".to_string(),
            type_name: "struct".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "array_Struck".to_string(),
            type_name: "array".to_string(),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "protectedHiddenMember".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "privateHiddenMember".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbModifier".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbBoneIndexArray".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbCustomIdSelector".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbBoneWeightArray".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbCustomTestGeneratorStruck".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbReferencePoseGenerator".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbCustomTestGeneratorAnnotatedTypes".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbGenerator".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        }),
    );
    manager.register(
        52,
        Patch::new(
            ClassVersion::new("hkbCustomTestGeneratorStruck", 0),
            ClassVersion::new("", -2),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "hkBool".to_string(),
            type_name: "int8".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "string".to_string(),
            type_name: "string".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "int".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "hkInt8".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "hkInt16".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "hkInt32".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "hkUint8".to_string(),
            type_name: "int8".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "hkUint16".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "hkUint32".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "hkReal".to_string(),
            type_name: "real".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "mode_hkInt8".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "mode_hkInt16".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "mode_hkInt32".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "mode_hkUint8".to_string(),
            type_name: "int8".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "mode_hkUint16".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "mode_hkUint32".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "flags_hkInt8".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "flags_hkInt16".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "flags_hkInt32".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "flags_hkUint8".to_string(),
            type_name: "int8".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "flags_hkUint16".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "flags_hkUint32".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "generator1".to_string(),
            type_name: "struct".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "generator2".to_string(),
            type_name: "struct".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "modifier1".to_string(),
            type_name: "struct".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "modifier2".to_string(),
            type_name: "struct".to_string(),
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
            class_name: "hkbModifier".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        }),
    );
    manager.register(
        52,
        Patch::new(
            ClassVersion::new("hkbFootIkGains", 0),
            ClassVersion::new("hkbFootIkGains", 1),
        )
        .with_operation(PatchOperation::MemberRename {
            old_name: "footUnlockGain".to_string(),
            new_name: "footLockingGain".to_string(),
        }),
    );
    // physics.py
    manager.register(
        52,
        Patch::new(
            ClassVersion::new("hkpBreakableConstraintData", 1),
            ClassVersion::new("hkpBreakableConstraintData", 2),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "childRuntimeSize".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "childNumSolverResults".to_string(),
            type_name: "int".to_string(),
        }),
    );
    manager.register(
        52,
        Patch::new(
            ClassVersion::new("hkpSetupStabilizationAtom", 2),
            ClassVersion::new("hkpSetupStabilizationAtom", 3),
        ),
    );
    // cloth.py
    manager.register(
        52,
        Patch::new(
            ClassVersion::new("hclSimulateSetupObject", 2),
            ClassVersion::new("hclSimulateSetupObject", 3),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "adaptConstraintStiffness".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        52,
        Patch::new(
            ClassVersion::new("hclStretchLinkSetupObject", 1),
            ClassVersion::new("hclStretchLinkSetupObject", 2),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "allowDynamicLinks".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "useTopologicalStretchDistance".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        52,
        Patch::new(
            ClassVersion::new("hclSimClothData", 10),
            ClassVersion::new("hclSimClothData", 11),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "fixedParticles".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        52,
        Patch::new(
            ClassVersion::new("hclSimulateOperator", 2),
            ClassVersion::new("hclSimulateOperator", 3),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "adaptConstraintStiffness".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        52,
        Patch::new(
            ClassVersion::new("hclSimClothDataOverridableSimulationInfo", 0),
            ClassVersion::new("hclSimClothDataOverridableSimulationInfo", 1),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "subSteps".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        52,
        Patch::new(
            ClassVersion::new("hclBonePlanesSetupObject", 0),
            ClassVersion::new("hclBonePlanesSetupObject", 1),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "angleSpecifiedInDegrees".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: None,
        }),
    );
}
