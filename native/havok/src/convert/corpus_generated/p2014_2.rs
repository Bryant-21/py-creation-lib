// Native Havok conversion corpus. Hand-maintained.

use super::super::manager::PatchManager;
use super::super::ops::{ClassVersion, Patch, PatchOperation};

pub(super) fn register(manager: &mut PatchManager) {
    // version_id = 55
    // common.py
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkAabb24_16_24", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "min".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "max".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        }),
    );
    // physics_np.py
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpBody", 2),
            ClassVersion::new("hknpBody", 3),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "shapeSizeDiv16".to_string(),
            type_name: "uint8".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "serial".to_string(),
            type_name: "uint8".to_string(),
        })
        .with_operation(PatchOperation::MemberRename {
            old_name: "timAngle".to_string(),
            new_name: "timAngle_old".to_string(),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "timAngle".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_custom_hook("hknpBody_2_to_3")
        .with_operation(PatchOperation::MemberRemove {
            name: "timAngle_old".to_string(),
            type_name: "uint8".to_string(),
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpShape", 2),
            ClassVersion::new("hknpShape", 3),
        )
        .with_custom_hook("hknpShape_2_to_3"),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpDecoratorShape", 0),
            ClassVersion::new("hknpDecoratorShape", 1),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "coreShapeSize".to_string(),
            type_name: "int".to_string(),
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpMaskedShape", 0),
            ClassVersion::new("hknpMaskedShape", 1),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "maskSize".to_string(),
            type_name: "int".to_string(),
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpLodShape", 1),
            ClassVersion::new("hknpLodShape", 2),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "shapesMemorySizes".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "indexCurrentShapeOnSpu".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "currentShapePpuAddress".to_string(),
            type_name: "pointer".to_string(),
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpConvexPolytopeShape", 3),
            ClassVersion::new("hknpConvexPolytopeShape", 4),
        )
        .with_custom_hook("hknpConvexPolytopeShape_3_to_4"),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpHeightFieldShape", 2),
            ClassVersion::new("hknpHeightFieldShape", 3),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hknpDynamicCompoundShapeKeyMask", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hknpCompoundShapeInternalsKeyMask".to_string()),
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpConstraint", 1),
            ClassVersion::new("hknpConstraint", 2),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "bodyIdA".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "bodyIdB".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_custom_hook("hknpConstraint_1_to_2")
        .with_operation(PatchOperation::MemberRemove {
            name: "bodyUidA".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "bodyUidB".to_string(),
            type_name: "int".to_string(),
        }),
    );
    // cloth.py
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclSimClothData", 11),
            ClassVersion::new("hclSimClothData", 12),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "transferMotionEnabled".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hclSimClothDataOverridableSimulationInfo".to_string(),
            version: 1,
        })
        .with_custom_hook("hclSimClothData_11_to_12"),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclSimClothDataOverridableSimulationInfo", 1),
            ClassVersion::new("hclSimClothDataOverridableSimulationInfo", 2),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "transferMotionEnabled".to_string(),
            type_name: "int8".to_string(),
        }),
    );
}
