// Native Havok conversion corpus. Hand-maintained.

use super::super::manager::PatchManager;
use super::super::ops::{ClassVersion, Patch, PatchOperation, PatchValue};

pub(super) fn register(manager: &mut PatchManager) {
    // version_id = 50
    // common.py
    manager.register(
        50,
        Patch::new(
            ClassVersion::new("hkPackfileHeader", 1),
            ClassVersion::new("hkPackfileHeader", 2),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "pad".to_string(),
            type_name: "array".to_string(),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "maxpredicate".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "predicateArraySizePlusPadding".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        50,
        Patch::new(
            ClassVersion::new("hkAabbHalf", 0),
            ClassVersion::new("hkAabbHalf", 1),
        )
        .with_operation(PatchOperation::MemberRename {
            old_name: "data".to_string(),
            new_name: "data_old".to_string(),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "data".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_custom_hook("_hkAabbHalf_0_to_1")
        .with_operation(PatchOperation::MemberRemove {
            name: "data_old".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "extras".to_string(),
            type_name: "int".to_string(),
        }),
    );
    manager.register(
        50,
        Patch::new(ClassVersion::new("", -1), ClassVersion::new("hkUuid", 0)).with_operation(
            PatchOperation::MemberAdd {
                name: "data".to_string(),
                type_name: "int".to_string(),
                ctype: None,
                default: None,
            },
        ),
    );
    manager.register(
        50,
        Patch::new(
            ClassVersion::new("hkUuid", 0),
            ClassVersion::new("hkUuid", 1),
        ),
    );
    manager.register(
        50,
        Patch::new(
            ClassVersion::new("hkUiAttribute", 2),
            ClassVersion::new("hkUiAttribute", 3),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "editable".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        }),
    );
    manager.register(
        50,
        Patch::new(ClassVersion::new("", -1), ClassVersion::new("hkAabbD", 0))
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
    // behavior.py
    manager.register(
        50,
        Patch::new(
            ClassVersion::new("hkbCharacter", 3),
            ClassVersion::new("hkbCharacter", 4),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "capabilities".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "effectiveCapabilities".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_custom_hook("_hkbCharacter_3_to_4"),
    );
    manager.register(
        50,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkbCustomIdSelector", 0),
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
        50,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkbTestIdSelector", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkbCustomIdSelector".to_string()),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "int".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "real".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
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
            class_name: "hkbCustomIdSelector".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        }),
    );
    manager.register(
        50,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkbStateChooserWrapper", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkbCustomIdSelector".to_string()),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "wrappedChooser".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkbStateChooser".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkBaseObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbCustomIdSelector".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbStateChooser".to_string(),
            version: 0,
        }),
    );
    manager.register(
        50,
        Patch::new(
            ClassVersion::new("hkbStateMachine", 4),
            ClassVersion::new("hkbStateMachine", 5),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "startStateIdSelector".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkbCustomIdSelector".to_string()),
            default: None,
        })
        .with_custom_hook("_hkbStateMachine_4_to_5")
        .with_operation(PatchOperation::MemberRemove {
            name: "startStateChooser".to_string(),
            type_name: "struct".to_string(),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbStateChooser".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbStateChooserWrapper".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbCustomIdSelector".to_string(),
            version: 0,
        }),
    );
    manager.register(
        50,
        Patch::new(
            ClassVersion::new("hkbTestStateChooser", 0),
            ClassVersion::new("", -2),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: Some("hkbStateChooser".to_string()),
            new_parent: None,
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "int".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "real".to_string(),
            type_name: "real".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "string".to_string(),
            type_name: "string".to_string(),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkBaseObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbStateChooser".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        }),
    );
    manager.register(
        50,
        Patch::new(
            ClassVersion::new("hkbCustomTestGenerator", 2),
            ClassVersion::new("hkbCustomTestGenerator", 3),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "idSelector".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkbCustomIdSelector".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbCustomIdSelector".to_string(),
            version: 0,
        }),
    );
    manager.register(
        50,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkbManualSelectorTransitionEffect", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkbTransitionEffect".to_string()),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "transitionEffects".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hkbTransitionEffect".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "selectedIndex".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "indexSelector".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkbCustomIdSelector".to_string()),
            default: None,
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
            class_name: "hkbTransitionEffect".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbBindable".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbCustomIdSelector".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        }),
    );
    manager.register(
        50,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkbManualSelectorTransitionEffectInternalState", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkReferencedObject".to_string()),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "currentTransitionEffectIndex".to_string(),
            type_name: "int8".to_string(),
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
        50,
        Patch::new(
            ClassVersion::new("hkbDockingGenerator", 0),
            ClassVersion::new("hkbDockingGenerator", 1),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "previousLocalTime".to_string(),
            type_name: "real".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "intervalStartLocalTime".to_string(),
            type_name: "real".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "intervalEndLocalTime".to_string(),
            type_name: "real".to_string(),
        }),
    );
    manager.register(
        50,
        Patch::new(
            ClassVersion::new("hkbManualSelectorGenerator", 2),
            ClassVersion::new("hkbManualSelectorGenerator", 3),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "currentGeneratorIndex".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "generatorIndexAtActivate".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "indexSelector".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkbCustomIdSelector".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbCustomIdSelector".to_string(),
            version: 0,
        }),
    );
    manager.register(
        50,
        Patch::new(
            ClassVersion::new("hkbFootIkDriverInfoLeg", 0),
            ClassVersion::new("hkbFootIkDriverInfoLeg", 1),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "hipSiblingIndex".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(-1)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "kneeSiblingIndex".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(-1)),
        }),
    );
    manager.register(
        50,
        Patch::new(
            ClassVersion::new("hkbBlendingTransitionEffectInternalState", 1),
            ClassVersion::new("hkbBlendingTransitionEffectInternalState", 2),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "resetToGenerator".to_string(),
            type_name: "int8".to_string(),
        }),
    );
    // physics.py
    manager.register(
        50,
        Patch::new(
            ClassVersion::new("hkpVehicleLinearCastWheelCollide", 0),
            ClassVersion::new("hkpVehicleLinearCastWheelCollide", 1),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "collectStartPointHits".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        }),
    );
    manager.register(
        50,
        Patch::new(
            ClassVersion::new("hkpBreakableConstraintData", 0),
            ClassVersion::new("hkpBreakableConstraintData", 1),
        )
        .with_operation(PatchOperation::Depends {
            class_name: "hkpWrappedConstraintData".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::ParentSet {
            old_parent: Some("hkpConstraintData".to_string()),
            new_parent: Some("hkpWrappedConstraintData".to_string()),
        })
        .with_operation(PatchOperation::MemberRename {
            old_name: "constraintData".to_string(),
            new_name: "constraintDataOld".to_string(),
        })
        .with_custom_hook("_hkpBreakableConstraintData_0_to_1")
        .with_operation(PatchOperation::MemberRemove {
            name: "constraintDataOld".to_string(),
            type_name: "struct".to_string(),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkpConstraintData".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        }),
    );
    manager.register(
        50,
        Patch::new(
            ClassVersion::new("hkpMalleableConstraintData", 0),
            ClassVersion::new("hkpMalleableConstraintData", 1),
        )
        .with_operation(PatchOperation::Depends {
            class_name: "hkpWrappedConstraintData".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::ParentSet {
            old_parent: Some("hkpConstraintData".to_string()),
            new_parent: Some("hkpWrappedConstraintData".to_string()),
        })
        .with_operation(PatchOperation::MemberRename {
            old_name: "constraintData".to_string(),
            new_name: "constraintDataOld".to_string(),
        })
        .with_custom_hook("_hkpMalleableConstraintData_0_to_1")
        .with_operation(PatchOperation::MemberRemove {
            name: "constraintDataOld".to_string(),
            type_name: "struct".to_string(),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkpConstraintData".to_string(),
            version: 0,
        }),
    );
    manager.register(
        50,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkpWrappedConstraintData", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkpConstraintData".to_string()),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "constraintData".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkpConstraintData".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkBaseObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkpConstraintData".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        }),
    );
    // cloth.py
    manager.register(
        50,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hclTransformSetUsageTransformTracker", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "read".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkBitField".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "readBeforeWrite".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkBitField".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "written".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkBitField".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkBitField".to_string(),
            version: 2,
        }),
    );
    manager.register(
        50,
        Patch::new(
            ClassVersion::new("hclTransformSetUsage", 0),
            ClassVersion::new("hclTransformSetUsage", 1),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "perComponentTransformTrackers".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hclTransformSetUsageTransformTracker".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hclTransformSetUsageTransformTracker".to_string(),
            version: 0,
        }),
    );
}
