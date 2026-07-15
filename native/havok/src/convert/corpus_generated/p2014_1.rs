// Native Havok conversion corpus. Hand-maintained.

use super::super::manager::PatchManager;
use super::super::ops::{ClassVersion, Patch, PatchOperation, PatchValue};

pub(super) fn register(manager: &mut PatchManager) {
    // version_id = 53
    // common.py
    manager.register(
        53,
        Patch::new(ClassVersion::new("hkAabbD", 0), ClassVersion::new("", -2))
            .with_operation(PatchOperation::MemberRemove {
                name: "min".to_string(),
                type_name: "int".to_string(),
            })
            .with_operation(PatchOperation::MemberRemove {
                name: "max".to_string(),
                type_name: "int".to_string(),
            }),
    );
    manager.register(
        53,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkAttributeHideCriteria", 0),
        ),
    );
    manager.register(
        53,
        Patch::new(
            ClassVersion::new("hkDocumentationAttribute", 0),
            ClassVersion::new("hkDocumentationAttribute", 1),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "docsSectionTag".to_string(),
            type_name: "string".to_string(),
        }),
    );
    manager.register(
        53,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkUuidObject", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkReferencedObject".to_string()),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "uuid".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkUuid".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkBaseObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkUuid".to_string(),
            version: 1,
        }),
    );
    manager.register(
        53,
        Patch::new(
            ClassVersion::new("hkxNode", 4),
            ClassVersion::new("hkxNode", 5),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "uuid".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkUuid".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkUuid".to_string(),
            version: 1,
        })
        .with_custom_hook("_hkxNode_4_to_5"),
    );
    // behavior.py
    manager.register(
        53,
        Patch::new(
            ClassVersion::new("hkbCharacterControllerModifier", 1),
            ClassVersion::new("hkbCharacterControllerModifier", 2),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "gravityFactor".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: Some(PatchValue::Real(1.0_f32)),
        })
        .with_custom_hook("_hkbCharacterControllerModifier_1_to_2")
        .with_operation(PatchOperation::MemberRemove {
            name: "forceDownwardMomentum".to_string(),
            type_name: "int8".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "applyGravity".to_string(),
            type_name: "int8".to_string(),
        }),
    );
    manager.register(
        53,
        Patch::new(
            ClassVersion::new("hkbCharacterControllerModifierInternalState", 0),
            ClassVersion::new("hkbCharacterControllerModifierInternalState", 1),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "gravity".to_string(),
            type_name: "vec4".to_string(),
        }),
    );
    manager.register(
        53,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkbCharacterSkeletonChangedCommand", 0),
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
            name: "skeleton".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkaSkeleton".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "padding".to_string(),
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
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkaSkeleton".to_string(),
            version: 5,
        }),
    );
    manager.register(
        53,
        Patch::new(
            ClassVersion::new("hkbLeanRocketboxCharacterController", 0),
            ClassVersion::new("hkbLeanRocketboxCharacterController", 1),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "effectiveAngularSpeed".to_string(),
            type_name: "real".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "torsoTiltAngle".to_string(),
            type_name: "real".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "angularSpeed".to_string(),
            type_name: "real".to_string(),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "effectiveHeading".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "idleAnimationIdx".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "localHeading".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "facingChange".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "numIdleAnimations".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "switchIdleAnimationEvent".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkbEventProperty".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbEventProperty".to_string(),
            version: 1,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbEventBase".to_string(),
            version: 0,
        }),
    );
    // physics.py
    manager.register(
        53,
        Patch::new(
            ClassVersion::new("hkpBallSocketChainDataConstraintInfo", 0),
            ClassVersion::new("hkpBallSocketChainDataConstraintInfo", 1),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "flags".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        53,
        Patch::new(
            ClassVersion::new("hkpBallSocketChainData", 1),
            ClassVersion::new("hkpBallSocketChainData", 2),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "link0PivotBVelocity".to_string(),
            type_name: "vec4".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "inertiaPerMeter".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: Some(PatchValue::Real(20.0_f32)),
        }),
    );
    manager.register(
        53,
        Patch::new(
            ClassVersion::new("hkpAngConstraintAtom", 0),
            ClassVersion::new("hkpAngConstraintAtom", 1),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "constrainedAxes".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: None,
        })
        .with_custom_hook("_hkpAngConstraintAtom_0_to_1")
        .with_operation(PatchOperation::MemberRemove {
            name: "firstConstrainedAxis".to_string(),
            type_name: "int8".to_string(),
        }),
    );
    manager.register(
        53,
        Patch::new(
            ClassVersion::new("hkpAngLimitConstraintAtom", 0),
            ClassVersion::new("hkpAngLimitConstraintAtom", 1),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "cosineAxis".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: None,
        })
        .with_custom_hook("_hkpAngLimitConstraintAtom_0_to_1")
        .with_operation(PatchOperation::MemberAdd {
            name: "angularLimitsDampFactor".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: Some(PatchValue::Real(1.0_f32)),
        }),
    );
    manager.register(
        53,
        Patch::new(
            ClassVersion::new("hkpConeLimitConstraintAtom", 0),
            ClassVersion::new("hkpConeLimitConstraintAtom", 1),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "angularLimitsDampFactor".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: Some(PatchValue::Real(1.0_f32)),
        }),
    );
    manager.register(
        53,
        Patch::new(
            ClassVersion::new("hkpTwistLimitConstraintAtom", 0),
            ClassVersion::new("hkpTwistLimitConstraintAtom", 1),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "angularLimitsDampFactor".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: Some(PatchValue::Real(1.0_f32)),
        }),
    );
    manager.register(
        53,
        Patch::new(
            ClassVersion::new("hkpStiffSpringConstraintAtom", 1),
            ClassVersion::new("hkpStiffSpringConstraintAtom", 2),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "springConstant".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: Some(PatchValue::Real(-1.0_f32)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "springDamping".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: Some(PatchValue::Real(-1.0_f32)),
        }),
    );
    manager.register(
        53,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkpEllipticalLimitConstraintAtom", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkpConstraintAtom".to_string()),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "isEnabled".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "elipticalLimitEnabled".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "coneLimitEnabled".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "angle0".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "angle1".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "coneAngle".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "angleCorrected0".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "angleCorrected1".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "coneAngleCorrected".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "angleCorrected0Inv".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "angleCorrected1Inv".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "angularLimitsTauFactor".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: Some(PatchValue::Real(1.0_f32)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "angularLimitsDampFactor".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: Some(PatchValue::Real(1.0_f32)),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkpConstraintAtom".to_string(),
            version: 0,
        }),
    );
    manager.register(
        53,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkp6DofConstraintDataBlueprints", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "linearIsFixed".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "transforms".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkpSetLocalTransformsConstraintAtom".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "setupStabilization".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkpSetupStabilizationAtom".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "ragdollMotors".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkpRagdollMotorConstraintAtom".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "angFriction".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkpAngFrictionConstraintAtom".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "twistLimit".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkpTwistLimitConstraintAtom".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "ellipticalLimit".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkpEllipticalLimitConstraintAtom".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "stiffSpring".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkpStiffSpringConstraintAtom".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "linearMotor0".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkpLinMotorConstraintAtom".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "linearMotor1".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkpLinMotorConstraintAtom".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "linearMotor2".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkpLinMotorConstraintAtom".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "ballSocket".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkpBallSocketConstraintAtom".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkpSetupStabilizationAtom".to_string(),
            version: 3,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkpSetLocalTransformsConstraintAtom".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkpRagdollMotorConstraintAtom".to_string(),
            version: 1,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkpBallSocketConstraintAtom".to_string(),
            version: 5,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkpEllipticalLimitConstraintAtom".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkpTwistLimitConstraintAtom".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkpAngFrictionConstraintAtom".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkpStiffSpringConstraintAtom".to_string(),
            version: 1,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkpConstraintAtom".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkpLinMotorConstraintAtom".to_string(),
            version: 1,
        }),
    );
    manager.register(
        53,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkp6DofConstraintData", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkpConstraintData".to_string()),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "blueprints".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkp6DofConstraintDataBlueprints".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "isDirty".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "numRuntimeElements".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "atomToCompiledAtomOffset".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "resultToRuntime".to_string(),
            type_name: "int".to_string(),
            ctype: None,
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
            class_name: "hkp6DofConstraintDataBlueprints".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        }),
    );
    // physics_np.py
    manager.register(
        53,
        Patch::new(
            ClassVersion::new("hknpWorldCinfo", 5),
            ClassVersion::new("hknpWorldCinfo", 6),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "enableSolverDynamicScheduling".to_string(),
            type_name: "uint8".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "contactSolverType".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "leavingBroadPhaseBehavior".to_string(),
            type_name: "uint8".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "largeIslandSize".to_string(),
            type_name: "int".to_string(),
        }),
    );
    manager.register(
        53,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hknpConvexPolytopeShapeConnectivityEdge", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "faceIndex".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "edgeIndex".to_string(),
            type_name: "uint8".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        53,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hknpConvexPolytopeShapeConnectivity", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkReferencedObject".to_string()),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "vertexEdges".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hknpConvexPolytopeShapeConnectivityEdge".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "faceLinks".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hknpConvexPolytopeShapeConnectivityEdge".to_string()),
            default: None,
        }),
    );
    manager.register(
        53,
        Patch::new(
            ClassVersion::new("hknpCompressedMeshShapeData", 0),
            ClassVersion::new("hknpCompressedMeshShapeData", 1),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "connectivity".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkcdStaticMeshTreeBaseConnectivity".to_string()),
            default: None,
        }),
    );
    manager.register(
        53,
        Patch::new(
            ClassVersion::new("hknpConvexPolytopeShape", 1),
            ClassVersion::new("hknpConvexPolytopeShape", 2),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "connectivity".to_string(),
            type_name: "pointer".to_string(),
            ctype: Some("hknpConvexPolytopeShapeConnectivity".to_string()),
            default: None,
        }),
    );
    manager.register(
        53,
        Patch::new(
            ClassVersion::new("hknpCharacterRigidBodyCinfo", 2),
            ClassVersion::new("hknpCharacterRigidBodyCinfo", 3),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "activationMode".to_string(),
            type_name: "uint8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_custom_hook("hknpCharacterRigidBodyCinfo_2_to_3")
        .with_operation(PatchOperation::MemberRemove {
            name: "additionFlags".to_string(),
            type_name: "uint8".to_string(),
        }),
    );
    manager.register(
        53,
        Patch::new(
            ClassVersion::new("hknpBody", 1),
            ClassVersion::new("hknpBody", 2),
        )
        .with_custom_hook("hknpBody_1_to_2")
        .with_operation(PatchOperation::MemberRemove {
            name: "spuFlags".to_string(),
            type_name: "uint8".to_string(),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "serial".to_string(),
            type_name: "uint8".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        53,
        Patch::new(
            ClassVersion::new("hknpBodyCinfo", 2),
            ClassVersion::new("hknpBodyCinfo", 3),
        )
        .with_operation(PatchOperation::MemberRename {
            old_name: "motionId".to_string(),
            new_name: "reservedMotionId".to_string(),
        })
        .with_custom_hook("hknpBodyCinfo_2_to_3")
        .with_operation(PatchOperation::MemberRemove {
            name: "spuFlags".to_string(),
            type_name: "uint8".to_string(),
        }),
    );
    manager.register(
        53,
        Patch::new(
            ClassVersion::new("hknpConstraint", 0),
            ClassVersion::new("hknpConstraint", 1),
        )
        .with_operation(PatchOperation::MemberRename {
            old_name: "bodyIdA".to_string(),
            new_name: "bodyIdA_old".to_string(),
        })
        .with_operation(PatchOperation::MemberRename {
            old_name: "bodyIdB".to_string(),
            new_name: "bodyIdB_old".to_string(),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "bodyUidA".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "bodyUidB".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_custom_hook("hknpConstraint_0_to_1")
        .with_operation(PatchOperation::MemberRemove {
            name: "bodyIdA_old".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "bodyIdB_old".to_string(),
            type_name: "int".to_string(),
        }),
    );
    manager.register(
        53,
        Patch::new(
            ClassVersion::new("hknpConstraintCinfo", 2),
            ClassVersion::new("hknpConstraintCinfo", 3),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "name".to_string(),
            type_name: "string".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        53,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hknpMassDistribution", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "centerOfMassAndVolume".to_string(),
            type_name: "vec4".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "inertiaTensor".to_string(),
            type_name: "vec4".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "majorAxisSpace".to_string(),
            type_name: "vec4".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        53,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hknpRefMassDistribution", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkReferencedObject".to_string()),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "massDistribution".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hknpMassDistribution".to_string()),
            default: None,
        }),
    );
    manager.register(
        53,
        Patch::new(
            ClassVersion::new("hknpMotion", 2),
            ClassVersion::new("hknpMotion", 3),
        )
        .with_operation(PatchOperation::MemberRename {
            old_name: "centerOfMassAndMassFactor".to_string(),
            new_name: "centerOfMass".to_string(),
        })
        .with_operation(PatchOperation::MemberRename {
            old_name: "maxRotationToPreventTunneling".to_string(),
            new_name: "maxRotationPerStep".to_string(),
        }),
    );
    manager.register(
        53,
        Patch::new(
            ClassVersion::new("hknpMotionCinfo", 1),
            ClassVersion::new("hknpMotionCinfo", 2),
        )
        .with_operation(PatchOperation::MemberRename {
            old_name: "maxRotationToPreventTunneling".to_string(),
            new_name: "maxRotationPerStep".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "massFactor".to_string(),
            type_name: "real".to_string(),
        }),
    );
    manager.register(
        53,
        Patch::new(
            ClassVersion::new("hknpBodyCinfo", 3),
            ClassVersion::new("hknpBodyCinfo", 4),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "motionType".to_string(),
            type_name: "uint8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "motionPropertiesId".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "linearVelocity".to_string(),
            type_name: "vec4".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "angularVelocity".to_string(),
            type_name: "vec4".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "mass".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: Some(PatchValue::Real(-1.0_f32)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "massDistribution".to_string(),
            type_name: "pointer".to_string(),
            ctype: Some("hknpRefMassDistribution".to_string()),
            default: None,
        })
        .with_custom_hook("hknpBodyCinfo_3_to_4"),
    );
    manager.register(
        53,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hknpMountedBallGun", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hknpBallGun".to_string()),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "position".to_string(),
            type_name: "vec4".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        53,
        Patch::new(
            ClassVersion::new("hknpDestructionShapeProperties", 0),
            ClassVersion::new("hknpDestructionShapeProperties", 1),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "sceneNodeUuid".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkUuid".to_string()),
            default: None,
        }),
    );
    manager.register(
        53,
        Patch::new(
            ClassVersion::new("hknpConvexPolytopeShape", 2),
            ClassVersion::new("hknpConvexPolytopeShape", 3),
        )
        .with_custom_hook("hknpConvexPolytopeShape_2_to_3"),
    );
    // cloth.py
    manager.register(
        53,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hclObjectSpaceDeformerFiveBlendEntryBlock", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "vertexIndices".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "boneIndices".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "boneWeights".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        53,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hclObjectSpaceDeformerEightBlendEntryBlock", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "vertexIndices".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "boneIndices".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "boneWeights".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        53,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hclObjectSpaceDeformerSevenBlendEntryBlock", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "vertexIndices".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "boneIndices".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "boneWeights".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        53,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hclObjectSpaceDeformerSixBlendEntryBlock", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "vertexIndices".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "boneIndices".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "boneWeights".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        53,
        Patch::new(
            ClassVersion::new("hclObjectSpaceMeshMeshDeformOperator", 0),
            ClassVersion::new("hclObjectSpaceMeshMeshDeformOperator", 1),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "customSkinDeform".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        53,
        Patch::new(
            ClassVersion::new("hclObjectSpaceDeformer", 0),
            ClassVersion::new("hclObjectSpaceDeformer", 1),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "eightBlendEntries".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hclObjectSpaceDeformerEightBlendEntryBlock".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "sevenBlendEntries".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hclObjectSpaceDeformerSevenBlendEntryBlock".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "sixBlendEntries".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hclObjectSpaceDeformerSixBlendEntryBlock".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "fiveBlendEntries".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hclObjectSpaceDeformerFiveBlendEntryBlock".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hclObjectSpaceDeformerSixBlendEntryBlock".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hclObjectSpaceDeformerFiveBlendEntryBlock".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hclObjectSpaceDeformerEightBlendEntryBlock".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hclObjectSpaceDeformerSevenBlendEntryBlock".to_string(),
            version: 0,
        }),
    );
}
