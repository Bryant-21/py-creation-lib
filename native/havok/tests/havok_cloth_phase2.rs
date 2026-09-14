// Cloth bake/reverse tests, each asserting the SDK-correct shape of one member set.

use std::collections::HashSet;

use havok_native::cloth::ClothData;
use havok_native::cloth::bake::bake_cloth_setup;
use havok_native::cloth::reverse::{ReverseError, reverse_cloth_data, reverse_cloth_data_lossy};
use havok_native::cloth::setup::collidable_setup::{
    CapsuleShapeSetup, CollidableSetup, TaperedCapsuleShapeSetup,
};
use havok_native::cloth::setup::constraint_setup::{ConstraintSetupObject, StandardLinkSetup};
use havok_native::cloth::setup::mesh::SimulationSetupMesh;
use havok_native::cloth::setup::types::VertexFloatInput;
use havok_native::cloth::setup::{ClothSetupObject, SimClothSetupObject};
use havok_native::hkx::model::HkxMember;
use havok_native::hkx::types::HkxValue;
use havok_native::hkx::{HkxFile, HkxObject};

// ---------------------------------------------------------------------------
// Shared fixture builders
// ---------------------------------------------------------------------------

fn grid_sim_mesh() -> SimulationSetupMesh {
    let positions = vec![
        [0.0f32, 1.0, 0.0, 0.0],
        [1.0f32, 1.0, 0.0, 0.0],
        [0.0f32, 0.0, 0.0, 0.0],
        [1.0f32, 0.0, 0.0, 0.0],
    ];
    let triangles = vec![[0u32, 1, 2], [1u32, 3, 2]];
    SimulationSetupMesh {
        positions,
        triangles,
        ..Default::default()
    }
}

fn cape_with_capsule(capsule: CapsuleShapeSetup) -> ClothSetupObject {
    let constraint = ConstraintSetupObject::StandardLink(StandardLinkSetup {
        name: "Links".to_string(),
        stiffness: VertexFloatInput::constant(1.0),
        ..Default::default()
    });

    let collidable = CollidableSetup {
        name: "TestCol".to_string(),
        shape: Some(capsule),
        driving_bone_name: "bone_0".to_string(),
        ..Default::default()
    };

    let sim_cloth = SimClothSetupObject {
        name: "Grid".to_string(),
        simulation_mesh: Some(grid_sim_mesh()),
        constraint_setups: vec![constraint],
        collidable_setups: vec![collidable],
        ..Default::default()
    };

    ClothSetupObject {
        name: "GridSetup".to_string(),
        sim_cloth_setups: vec![sim_cloth],
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// Capsule emits only SDK members (radius + capLenSqrdInv, no smallRadius)
// ---------------------------------------------------------------------------

#[test]
fn capsule_emits_only_sdk_fields() {
    let setup = cape_with_capsule(CapsuleShapeSetup {
        start: [0.0, 0.0, 0.0, 0.0],
        end: [10.0, 0.0, 0.0, 0.0],
        big_radius: 1.5,
        small_radius: 1.5,
    });
    let hkx = bake_cloth_setup(&setup).expect("bake failed");
    let caps = hkx
        .objects()
        .iter()
        .find(|o| o.class_name == "hclCapsuleShape")
        .expect("hclCapsuleShape missing");
    let names: HashSet<&str> = caps.members.iter().map(|m| m.name.as_str()).collect();

    assert!(names.contains("radius"), "missing radius: {:?}", names);
    assert!(
        names.contains("capLenSqrdInv"),
        "missing capLenSqrdInv: {:?}",
        names
    );
    assert!(
        !names.contains("smallRadius"),
        "smallRadius is on hclTaperedCapsuleShape, not hclCapsuleShape"
    );

    // capLenSqrdInv = 1/|end-start|^2 = 1/100 = 0.01
    let cap_len_sqrd_inv = caps
        .members
        .iter()
        .find(|m| m.name == "capLenSqrdInv")
        .map(|m| match &m.value {
            HkxValue::F32(v) => *v,
            other => panic!("capLenSqrdInv has unexpected type: {:?}", other),
        })
        .unwrap();
    assert!(
        (cap_len_sqrd_inv - 0.01).abs() < 1e-6,
        "capLenSqrdInv = 1/|end-start|^2 expected 0.01, got {cap_len_sqrd_inv}"
    );
}

// ---------------------------------------------------------------------------
// hclTaperedCapsuleShape support
// ---------------------------------------------------------------------------

fn cape_with_tapered_capsule(tc: TaperedCapsuleShapeSetup) -> ClothSetupObject {
    let constraint = ConstraintSetupObject::StandardLink(StandardLinkSetup {
        name: "Links".to_string(),
        stiffness: VertexFloatInput::constant(1.0),
        ..Default::default()
    });

    let collidable = CollidableSetup {
        name: "TaperedCol".to_string(),
        tapered_shape: Some(tc),
        driving_bone_name: "bone_3".to_string(),
        ..Default::default()
    };

    let sim_cloth = SimClothSetupObject {
        name: "Grid".to_string(),
        simulation_mesh: Some(grid_sim_mesh()),
        constraint_setups: vec![constraint],
        collidable_setups: vec![collidable],
        ..Default::default()
    };

    ClothSetupObject {
        name: "TaperedSetup".to_string(),
        sim_cloth_setups: vec![sim_cloth],
        ..Default::default()
    }
}

#[test]
fn tapered_capsule_emits_with_required_sdk_fields() {
    let setup = cape_with_tapered_capsule(TaperedCapsuleShapeSetup {
        small: [0.0, 0.0, 0.0, 0.0],
        big: [10.0, 0.0, 0.0, 0.0],
        small_radius: 1.0,
        big_radius: 2.0,
    });
    let hkx = bake_cloth_setup(&setup).expect("bake failed");
    let tc = hkx
        .objects()
        .iter()
        .find(|o| o.class_name == "hclTaperedCapsuleShape")
        .expect("hclTaperedCapsuleShape not emitted");
    let names: HashSet<&str> = tc.members.iter().map(|m| m.name.as_str()).collect();

    // Required SDK members from refs/hk2018_1_0_r1/Source/Cloth/Cloth/Collide/
    // Shape/TaperedCapsule/hclTaperedCapsuleShape.h:
    for required in [
        "small",
        "big",
        "smallRadius",
        "bigRadius",
        "coneApex",
        "coneAxis",
        "lVec",
        "dVec",
        "tanThetaVecNeg",
        "l",
        "d",
        "cosTheta",
        "sinTheta",
        "tanTheta",
        "tanThetaSqr",
    ] {
        assert!(
            names.contains(required),
            "tapered capsule missing SDK member '{required}', present={names:?}"
        );
    }

    let small_radius = tc
        .members
        .iter()
        .find(|m| m.name == "smallRadius")
        .map(|m| match &m.value {
            HkxValue::F32(v) => *v,
            o => panic!("smallRadius wrong type {o:?}"),
        })
        .unwrap();
    let big_radius = tc
        .members
        .iter()
        .find(|m| m.name == "bigRadius")
        .map(|m| match &m.value {
            HkxValue::F32(v) => *v,
            o => panic!("bigRadius wrong type {o:?}"),
        })
        .unwrap();
    assert!((small_radius - 1.0).abs() < 1e-6);
    assert!((big_radius - 2.0).abs() < 1e-6);
}

// ---------------------------------------------------------------------------
// Sim cloth emits m_triangleFlips parallel to triangleIndices
// ---------------------------------------------------------------------------

#[test]
fn sim_cloth_emits_triangle_flips_default_zero() {
    let setup = cape_with_capsule(CapsuleShapeSetup {
        start: [0.0, 0.0, 0.0, 0.0],
        end: [10.0, 0.0, 0.0, 0.0],
        big_radius: 1.0,
        small_radius: 1.0,
    });
    let hkx = bake_cloth_setup(&setup).expect("bake failed");
    let scd = hkx
        .objects()
        .iter()
        .find(|o| o.class_name == "hclSimClothData")
        .expect("hclSimClothData missing");

    let tri_indices = match &scd
        .members
        .iter()
        .find(|m| m.name == "triangleIndices")
        .expect("triangleIndices missing")
        .value
    {
        HkxValue::Array(arr) => arr.clone(),
        other => panic!("triangleIndices not Array: {other:?}"),
    };
    let flips = match &scd
        .members
        .iter()
        .find(|m| m.name == "triangleFlips")
        .expect("triangleFlips missing — Task 2.3 fix not applied")
        .value
    {
        HkxValue::Array(arr) => arr.clone(),
        other => panic!("triangleFlips not Array: {other:?}"),
    };

    assert_eq!(
        flips.len(),
        tri_indices.len() / 3,
        "triangleFlips must have one byte per triangle (got {}, expected {})",
        flips.len(),
        tri_indices.len() / 3
    );
    for v in &flips {
        match v {
            HkxValue::U8(b) => assert_eq!(*b, 0u8, "default triangle flip must be 0"),
            other => panic!("triangleFlips entry has wrong type: {other:?}"),
        }
    }
}

// ---------------------------------------------------------------------------
// collidableTransformMap emits offsets and respects transformSetIndex
// ---------------------------------------------------------------------------

#[test]
fn collidable_transform_map_carries_offsets_and_set_index() {
    let constraint = ConstraintSetupObject::StandardLink(StandardLinkSetup {
        name: "Links".to_string(),
        stiffness: VertexFloatInput::constant(1.0),
        ..Default::default()
    });

    let collidable = CollidableSetup {
        name: "Col".to_string(),
        shape: Some(CapsuleShapeSetup {
            start: [0.0; 4],
            end: [5.0, 0.0, 0.0, 0.0],
            big_radius: 1.0,
            small_radius: 1.0,
        }),
        driving_bone_name: "bone_2".to_string(),
        ..Default::default()
    };

    // identity-ish offset matrix as a 16-float row-major matrix
    let offset: [f32; 16] = [
        1.0, 0.0, 0.0, 1.5, // translate +X by 1.5
        0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];

    let sim_cloth = SimClothSetupObject {
        name: "Grid".to_string(),
        simulation_mesh: Some(grid_sim_mesh()),
        constraint_setups: vec![constraint],
        collidable_setups: vec![collidable],
        collidable_transform_set_index: 1,
        collidable_offsets: vec![offset],
        ..Default::default()
    };

    let setup = ClothSetupObject {
        name: "Setup".to_string(),
        sim_cloth_setups: vec![sim_cloth],
        ..Default::default()
    };

    let hkx = bake_cloth_setup(&setup).expect("bake failed");
    let scd = hkx
        .objects()
        .iter()
        .find(|o| o.class_name == "hclSimClothData")
        .expect("hclSimClothData missing");

    let map = scd
        .members
        .iter()
        .find(|m| m.name == "collidableTransformMap")
        .expect("collidableTransformMap missing");
    let HkxValue::Object(fields) = &map.value else {
        panic!("collidableTransformMap not Object");
    };

    let tsi = fields
        .iter()
        .find(|m| m.name == "transformSetIndex")
        .expect("transformSetIndex missing");
    let tsi_value = match &tsi.value {
        HkxValue::U32(v) => *v as i64,
        HkxValue::I32(v) => *v as i64,
        other => panic!("transformSetIndex unexpected type: {other:?}"),
    };
    assert_eq!(
        tsi_value, 1,
        "demo uses transform set index 1; bake should not hard-code 0"
    );

    let offsets = fields
        .iter()
        .find(|m| m.name == "offsets")
        .expect("offsets missing — Task 2.4 not applied");
    let HkxValue::Array(arr) = &offsets.value else {
        panic!("offsets not Array");
    };
    assert_eq!(arr.len(), 1, "one collidable, one offset");
}

// ---------------------------------------------------------------------------
// passthrough_members on SimClothSetupObject re-emit verbatim
// ---------------------------------------------------------------------------

#[test]
fn passthrough_members_preserved_through_bake() {
    let constraint = ConstraintSetupObject::StandardLink(StandardLinkSetup {
        name: "Links".to_string(),
        stiffness: VertexFloatInput::constant(1.0),
        ..Default::default()
    });

    // Synthetic passthrough: a future Havok field the bake doesn't yet model.
    let synthetic = vec![
        HkxMember {
            name: "maxParticleRadius".to_string(),
            value: HkxValue::F32(0.75),
        },
        HkxMember {
            name: "minPinchedParticleIndex".to_string(),
            value: HkxValue::U16(0),
        },
        HkxMember {
            name: "actions".to_string(),
            value: HkxValue::Array(vec![]),
        },
        // Should be ignored — name collides with a field bake already emits.
        HkxMember {
            name: "totalMass".to_string(),
            value: HkxValue::F32(999.0),
        },
    ];

    let sim_cloth = SimClothSetupObject {
        name: "Grid".to_string(),
        simulation_mesh: Some(grid_sim_mesh()),
        constraint_setups: vec![constraint],
        passthrough_members: synthetic,
        ..Default::default()
    };

    let setup = ClothSetupObject {
        name: "Setup".to_string(),
        sim_cloth_setups: vec![sim_cloth],
        ..Default::default()
    };

    let hkx = bake_cloth_setup(&setup).expect("bake failed");
    let scd = hkx
        .objects()
        .iter()
        .find(|o| o.class_name == "hclSimClothData")
        .expect("hclSimClothData missing");

    let names: HashSet<&str> = scd.members.iter().map(|m| m.name.as_str()).collect();
    for required in ["maxParticleRadius", "minPinchedParticleIndex", "actions"] {
        assert!(
            names.contains(required),
            "passthrough member '{required}' was not preserved through bake"
        );
    }

    let max_radius = scd
        .members
        .iter()
        .find(|m| m.name == "maxParticleRadius")
        .map(|m| match &m.value {
            HkxValue::F32(v) => *v,
            other => panic!("maxParticleRadius wrong type {other:?}"),
        })
        .unwrap();
    assert!(
        (max_radius - 0.75).abs() < 1e-6,
        "passthrough value lost during bake"
    );

    // Confirm the collision case: bake's own totalMass wins over the passthrough.
    let total_mass = scd
        .members
        .iter()
        .find(|m| m.name == "totalMass")
        .map(|m| match &m.value {
            HkxValue::F32(v) => *v,
            other => panic!("totalMass wrong type {other:?}"),
        })
        .unwrap();
    assert!(
        (total_mass - 999.0).abs() > 1.0,
        "passthrough must NOT overwrite a field the bake itself emits — got {total_mass}"
    );
}

// ---------------------------------------------------------------------------
// Strict reverse refuses unknown operator/constraint classes
// ---------------------------------------------------------------------------

fn synth_hkx_with_unknown_operator() -> HkxFile {
    fn ptr(idx: usize) -> HkxValue {
        HkxValue::Pointer(Some(idx))
    }
    fn s(v: &str) -> HkxValue {
        HkxValue::String {
            value: v.to_string(),
            is_null: false,
        }
    }

    // object[0] hclClothData → operators=[ptr(1)]
    let cloth_obj = HkxObject {
        name: Some("#0000".to_string()),
        offset: 0,
        signature: 0,
        class_name: "hclClothData".to_string(),
        members: vec![
            HkxMember {
                name: "name".to_string(),
                value: s(""),
            },
            HkxMember {
                name: "simClothDatas".to_string(),
                value: HkxValue::Array(vec![]),
            },
            HkxMember {
                name: "operators".to_string(),
                value: HkxValue::Array(vec![ptr(1)]),
            },
            HkxMember {
                name: "clothStateDatas".to_string(),
                value: HkxValue::Array(vec![]),
            },
            HkxMember {
                name: "bufferDefinitions".to_string(),
                value: HkxValue::Array(vec![]),
            },
            HkxMember {
                name: "transformSetDefinitions".to_string(),
                value: HkxValue::Array(vec![]),
            },
        ],
    };

    // object[1] hclMysteryFutureOperator — unrecognised class name
    let unknown_op = HkxObject {
        name: Some("#0001".to_string()),
        offset: 1,
        signature: 0,
        class_name: "hclMysteryFutureOperator".to_string(),
        members: vec![HkxMember {
            name: "name".to_string(),
            value: s("Mystery"),
        }],
    };

    HkxFile::from_tagxml(11, "hk_2014.1.0-r1", vec![cloth_obj, unknown_op])
}

#[test]
fn reverse_strict_refuses_unknown_operator() {
    let file = synth_hkx_with_unknown_operator();
    let cloth = ClothData::from_hkx_file(&file).expect("from_hkx_file must find hclClothData");

    let result = reverse_cloth_data(&cloth);
    let Err(err) = result else {
        panic!("strict reverse must fail on unknown operator class, got Ok");
    };
    match err {
        ReverseError::UnknownClass { kind, class_name } => {
            assert_eq!(kind, "operator", "kind must be 'operator', got {kind}");
            assert_eq!(
                class_name, "hclMysteryFutureOperator",
                "must surface the offending class name verbatim"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Gravity reconciled to Z-up m/s² across solver, setup, bake
// ---------------------------------------------------------------------------

#[test]
fn gravity_is_z_up_m_per_s2_everywhere() {
    use havok_native::cloth::solver::SolverConfig;

    let cfg = SolverConfig::default();
    assert!(
        (cfg.gravity[2] - (-9.81)).abs() < 0.01,
        "solver default gravity Z component must be -9.81 m/s², got {:?}",
        cfg.gravity
    );
    assert!(
        cfg.gravity[1].abs() < 0.01,
        "solver default gravity Y must be 0 (Z-up), got {:?}",
        cfg.gravity
    );

    let setup_default = SimClothSetupObject::default();
    assert!(
        (setup_default.gravity[2] - (-9.81)).abs() < 0.01,
        "SimClothSetupObject default gravity Z must be -9.81 m/s², got {:?}",
        setup_default.gravity
    );
    assert!(
        setup_default.gravity[1].abs() < 0.01,
        "SimClothSetupObject default gravity Y must be 0 (Z-up), got {:?}",
        setup_default.gravity
    );

    // Bake forwards the setup gravity verbatim — confirm it lands in
    // simulationInfo.gravity unchanged.
    let setup = cape_with_capsule(CapsuleShapeSetup {
        start: [0.0, 0.0, 0.0, 0.0],
        end: [10.0, 0.0, 0.0, 0.0],
        big_radius: 1.0,
        small_radius: 1.0,
    });
    let hkx = bake_cloth_setup(&setup).expect("bake failed");
    let scd = hkx
        .objects()
        .iter()
        .find(|o| o.class_name == "hclSimClothData")
        .expect("hclSimClothData missing");
    let sim_info = scd
        .members
        .iter()
        .find(|m| m.name == "simulationInfo")
        .expect("simulationInfo missing");
    let HkxValue::Object(fields) = &sim_info.value else {
        panic!("simulationInfo not Object");
    };
    let g = fields
        .iter()
        .find(|m| m.name == "gravity")
        .expect("gravity missing");
    let g_vec: Vec<f32> = match &g.value {
        HkxValue::F32List(v) if v.len() >= 3 => v.clone(),
        other => panic!("gravity wrong shape: {other:?}"),
    };
    assert!(
        (g_vec[2] - (-9.81)).abs() < 0.01,
        "baked simulationInfo.gravity Z must be -9.81 m/s², got {g_vec:?}"
    );
    assert!(
        g_vec[1].abs() < 0.01,
        "baked simulationInfo.gravity Y must be 0 (Z-up)"
    );
}

#[test]
fn reverse_lossy_tolerates_unknown_operator() {
    let file = synth_hkx_with_unknown_operator();
    let cloth = ClothData::from_hkx_file(&file).expect("from_hkx_file must find hclClothData");

    // Lossy mode stubs unknown classes for the inspector path.
    let setup = reverse_cloth_data_lossy(&cloth);
    assert_eq!(
        setup.operator_setups.len(),
        1,
        "stubbed operator should survive"
    );
}
