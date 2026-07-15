// Native Havok conversion corpus. Hand-maintained.

use super::super::manager::PatchManager;
use super::super::ops::{ClassVersion, Patch, PatchOperation, PatchValue};

pub(super) fn register(manager: &mut PatchManager) {
    // version_id = 48
    // common.py
    manager.register(
        48,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new(
                "hkBitFieldStoragehkArrayunsignedinthkContainerHeapAllocator",
                0,
            ),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "words".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "numBits".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        48,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new(
                "hkBitFieldBasehkBitFieldStoragehkArrayunsignedinthkContainerHeapAllocator",
                0,
            ),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "storage".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkBitFieldStoragehkArrayunsignedinthkContainerHeapAllocator".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkBitFieldStoragehkArrayunsignedinthkContainerHeapAllocator".to_string(),
            version: 0,
        }),
    );
    manager.register(
        48,
        Patch::new(
            ClassVersion::new("hkBitField", 0),
            ClassVersion::new("hkBitField_new", 1),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some(
                "hkBitFieldBasehkBitFieldStoragehkArrayunsignedinthkContainerHeapAllocator"
                    .to_string(),
            ),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkBitFieldBasehkBitFieldStoragehkArrayunsignedinthkContainerHeapAllocator"
                .to_string(),
            version: 0,
        })
        .with_custom_hook("hkBitField_0_hkBitField_new_1")
        .with_operation(PatchOperation::MemberRemove {
            name: "words".to_string(),
            type_name: "array".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "numBits".to_string(),
            type_name: "int".to_string(),
        }),
    );
    manager.register(
        48,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new(
                "hkOffsetBitFieldStoragehkArrayunsignedinthkContainerHeapAllocator",
                0,
            ),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "words".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "offset".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        48,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new(
                "hkBitFieldBasehkOffsetBitFieldStoragehkArrayunsignedinthkContainerHeapAllocator",
                0,
            ),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "storage".to_string(),
            type_name: "struct".to_string(),
            ctype: Some(
                "hkOffsetBitFieldStoragehkArrayunsignedinthkContainerHeapAllocator".to_string(),
            ),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkOffsetBitFieldStoragehkArrayunsignedinthkContainerHeapAllocator"
                .to_string(),
            version: 0,
        }),
    );
    manager.register(
        48,
        Patch::new(
            ClassVersion::new("hkMemoryMeshMaterial", 1),
            ClassVersion::new("hkMemoryMeshMaterial", 2),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "userData".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "tesselationFactor".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "displacementAmount".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        48,
        Patch::new(
            ClassVersion::new("hkxMaterial", 4),
            ClassVersion::new("hkxMaterial", 5),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "userData".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        }),
    );
    manager.register(
        48,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkMeshTextureRawBufferDescriptor", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "offset".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "stride".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "numElements".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        48,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkOffsetBitField", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some(
                "hkBitFieldBasehkOffsetBitFieldStoragehkArrayunsignedinthkContainerHeapAllocator"
                    .to_string(),
            ),
        })
        .with_operation(PatchOperation::Depends {
            class_name:
                "hkBitFieldBasehkOffsetBitFieldStoragehkArrayunsignedinthkContainerHeapAllocator"
                    .to_string(),
            version: 0,
        }),
    );
    manager.register(
        48,
        Patch::new(
            ClassVersion::new("hkBitField_new", 1),
            ClassVersion::new("hkBitField", 2),
        ),
    );
    manager.register(
        48,
        Patch::new(
            ClassVersion::new("hkPackfileSectionHeader", 0),
            ClassVersion::new("hkPackfileSectionHeader", 1),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "pad".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        48,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new(
                "hkSetunsignedlonglonghkContainerHeapAllocatorhkMapOperationsunsignedlonglong",
                0,
            ),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "elem".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "numElems".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        48,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkSetUint64", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some(
                "hkSetunsignedlonglonghkContainerHeapAllocatorhkMapOperationsunsignedlonglong"
                    .to_string(),
            ),
        })
        .with_operation(PatchOperation::Depends {
            class_name:
                "hkSetunsignedlonglonghkContainerHeapAllocatorhkMapOperationsunsignedlonglong"
                    .to_string(),
            version: 0,
        }),
    );
    // behavior.py
    manager.register(
        48,
        Patch::new(
            ClassVersion::new("hkbLayer", 0),
            ClassVersion::new("hkbLayer", 1),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "forceFullFadeDurations".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        }),
    );
    manager.register(
        48,
        Patch::new(
            ClassVersion::new("hkbGeneratorTransitionEffectInternalState", 0),
            ClassVersion::new("hkbGeneratorTransitionEffectInternalState", 1),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "echoToGenerator".to_string(),
            type_name: "int8".to_string(),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "toGeneratorSelfTransitionMode".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        48,
        Patch::new(
            ClassVersion::new("hkbBlendingTransitionEffectInternalState", 0),
            ClassVersion::new("hkbBlendingTransitionEffectInternalState", 1),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "applySelfTransition".to_string(),
            type_name: "int8".to_string(),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "resetToGenerator".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "toGeneratorSelfTranstitionMode".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        48,
        Patch::new(
            ClassVersion::new("hkbCustomTestGenerator", 1),
            ClassVersion::new("hkbCustomTestGenerator", 2),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "boneIndexOld".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "boneIndex".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "boneChainIndex0".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "boneChainIndex1".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "boneChainIndex2".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "boneGroupIndex0".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "boneGroupIndex1".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "boneGroupIndex2".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
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
            class_name: "hkbBoneIndexArray".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbBoneWeightArray".to_string(),
            version: 0,
        }),
    );
    // physics.py
    manager.register(
        48,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkpVehicleSimulation", 0),
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
        48,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkpVehicleDefaultSimulation", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkpVehicleSimulation".to_string()),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "frictionStatus".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkpVehicleFrictionStatus".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "frictionDescription".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkpVehicleFrictionDescription".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkBaseObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkpVehicleFrictionDescription".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkpVehicleSimulation".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkpVehicleFrictionStatus".to_string(),
            version: 0,
        }),
    );
    manager.register(
        48,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkpVehiclePerWheelSimulationWheelData", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "axle".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkpWheelFrictionConstraintAtomAxle".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "frictionData".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkpWheelFrictionConstraintData".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "forwardDirectionWs".to_string(),
            type_name: "vec4".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "sideDirectionWs".to_string(),
            type_name: "vec4".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "contactLocal".to_string(),
            type_name: "vec4".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkpConstraintInstance".to_string(),
            version: 1,
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
            class_name: "hkpWheelFrictionConstraintAtomAxle".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkpWheelFrictionConstraintData".to_string(),
            version: 0,
        }),
    );
    manager.register(
        48,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkpVehiclePerWheelSimulation", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkpVehicleSimulation".to_string()),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "instance".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkpVehicleInstance".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "slipDamping".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "impulseScaling".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "maxImpulse".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "takeDynamicVelocity".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "curbDamping".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "wheelData".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hkpVehiclePerWheelSimulationWheelData".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkpVehiclePerWheelSimulationWheelData".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkBaseObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkpVehicleInstance".to_string(),
            version: 2,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkpAction".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkpVehicleSimulation".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkpUnaryAction".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        }),
    );
    manager.register(
        48,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkpVehicleSteeringAckerman", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkpVehicleSteering".to_string()),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "maxSteeringAngle".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "maxSpeedFullSteeringAngle".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "doesWheelSteer".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "trackWidth".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "wheelBaseLength".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkpVehicleSteering".to_string(),
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
        48,
        Patch::new(
            ClassVersion::new("hkpVehicleInstance", 1),
            ClassVersion::new("hkpVehicleInstance", 2),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "frictionStatus".to_string(),
            type_name: "struct".to_string(),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "vehicleSimulation".to_string(),
            type_name: "pointer".to_string(),
            ctype: Some("hkpVehicleSimulation".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkpVehicleFrictionStatus".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkpVehicleSimulation".to_string(),
            version: 0,
        }),
    );
    manager.register(
        48,
        Patch::new(
            ClassVersion::new("hkpVehicleData", 1),
            ClassVersion::new("hkpVehicleData", 2),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "frictionDescription".to_string(),
            type_name: "struct".to_string(),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkpVehicleFrictionDescription".to_string(),
            version: 0,
        }),
    );
    manager.register(
        48,
        Patch::new(
            ClassVersion::new("hkpCollidable", 0),
            ClassVersion::new("hkpCollidable", 2),
        ),
    );
    manager.register(
        48,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkpWheelFrictionConstraintAtomAxle", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "spinVelocity".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "sumVelocity".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "numWheels".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "wheelsSolved".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "stepsSolved".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "invInertia".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "inertia".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "impulseScaling".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "impulseMax".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "isFixed".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "numWheelsOnGround".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        48,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkpWheelFrictionConstraintAtom", 0),
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
            name: "forwardAxis".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "sideAxis".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "maxFrictionForce".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "torque".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "radius".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "frictionImpulse".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "slipImpulse".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "axle".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkpWheelFrictionConstraintAtomAxle".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkpWheelFrictionConstraintAtomAxle".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkpConstraintAtom".to_string(),
            version: 0,
        }),
    );
    manager.register(
        48,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkpWheelFrictionConstraintDataAtoms", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "transforms".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkpSetLocalTransformsConstraintAtom".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "friction".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkpWheelFrictionConstraintAtom".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkpWheelFrictionConstraintAtom".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkpSetLocalTransformsConstraintAtom".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkpConstraintAtom".to_string(),
            version: 0,
        }),
    );
    manager.register(
        48,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkpWheelFrictionConstraintData", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkpConstraintData".to_string()),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "atoms".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkpWheelFrictionConstraintDataAtoms".to_string()),
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
            class_name: "hkpWheelFrictionConstraintDataAtoms".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        }),
    );
    manager.register(
        48,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkpWheelFrictionConstraintDataRuntime", 0),
        ),
    );
    manager.register(
        48,
        Patch::new(
            ClassVersion::new("hkpVehicleFrictionDescription", 0),
            ClassVersion::new("hkpVehicleFrictionDescription", 1),
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
    // cloth.py
    manager.register(
        48,
        Patch::new(
            ClassVersion::new("hclUpdateAllVertexFramesOperator", 2),
            ClassVersion::new("hclUpdateAllVertexFramesOperator", 3),
        )
        .with_operation(PatchOperation::MemberRename {
            old_name: "triangleFlips".to_string(),
            new_name: "old_triangleFlips".to_string(),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "triangleFlips".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_custom_hook("hclUpdateAllVertexFramesOperator_2_to_3")
        .with_operation(PatchOperation::MemberRemove {
            name: "old_triangleFlips".to_string(),
            type_name: "array".to_string(),
        }),
    );
    manager.register(
        48,
        Patch::new(
            ClassVersion::new("hclUpdateSomeVertexFramesOperator", 2),
            ClassVersion::new("hclUpdateSomeVertexFramesOperator", 3),
        )
        .with_operation(PatchOperation::MemberRename {
            old_name: "triangleFlips".to_string(),
            new_name: "old_triangleFlips".to_string(),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "triangleFlips".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_custom_hook("hclUpdateSomeVertexFramesOperator_2_to_3")
        .with_operation(PatchOperation::MemberRemove {
            name: "old_triangleFlips".to_string(),
            type_name: "array".to_string(),
        }),
    );
    manager.register(
        48,
        Patch::new(
            ClassVersion::new("hclSimClothData", 9),
            ClassVersion::new("hclSimClothData", 10),
        )
        .with_operation(PatchOperation::MemberRename {
            old_name: "triangleFlips".to_string(),
            new_name: "old_triangleFlips".to_string(),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "triangleFlips".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_custom_hook("hclSimClothData_9_to_10")
        .with_operation(PatchOperation::MemberRemove {
            name: "old_triangleFlips".to_string(),
            type_name: "array".to_string(),
        }),
    );
}
