// Cloth bake/reverse tests asserting SDK-correct member sets.

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

fn grid_sim_cloth(collidable_setups: Vec<CollidableSetup>) -> SimClothSetupObject {
    SimClothSetupObject {
        name: "Grid".to_string(),
        simulation_mesh: Some(SimulationSetupMesh {
            positions: vec![
                [0.0, 1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0, 0.0],
            ],
            triangles: vec![[0, 1, 2], [1, 3, 2]],
            ..Default::default()
        }),
        constraint_setups: vec![ConstraintSetupObject::StandardLink(StandardLinkSetup {
            name: "Links".to_string(),
            stiffness: VertexFloatInput::constant(1.0),
            ..Default::default()
        })],
        collidable_setups,
        ..Default::default()
    }
}

fn bake_one(sim_cloth: SimClothSetupObject) -> HkxFile {
    bake_cloth_setup(&ClothSetupObject {
        name: "Setup".to_string(),
        sim_cloth_setups: vec![sim_cloth],
        ..Default::default()
    })
    .expect("bake failed")
}

fn find<'a>(hkx: &'a HkxFile, class_name: &str) -> &'a HkxObject {
    hkx.objects()
        .iter()
        .find(|o| o.class_name == class_name)
        .unwrap_or_else(|| panic!("{class_name} missing"))
}

fn value<'a>(members: &'a [HkxMember], name: &str) -> &'a HkxValue {
    &members
        .iter()
        .find(|m| m.name == name)
        .unwrap_or_else(|| panic!("member {name} missing"))
        .value
}

fn f32_of(v: &HkxValue) -> f32 {
    match v {
        HkxValue::F32(f) => *f,
        other => panic!("expected F32, got {other:?}"),
    }
}

#[test]
fn capsule_shapes_emit_only_sdk_fields() {
    let hkx = bake_one(grid_sim_cloth(vec![
        CollidableSetup {
            name: "Capsule".to_string(),
            shape: Some(CapsuleShapeSetup {
                start: [0.0, 0.0, 0.0, 0.0],
                end: [10.0, 0.0, 0.0, 0.0],
                big_radius: 1.5,
                small_radius: 1.5,
            }),
            driving_bone_name: "bone_0".to_string(),
            ..Default::default()
        },
        CollidableSetup {
            name: "Tapered".to_string(),
            tapered_shape: Some(TaperedCapsuleShapeSetup {
                small: [0.0, 0.0, 0.0, 0.0],
                big: [10.0, 0.0, 0.0, 0.0],
                small_radius: 1.0,
                big_radius: 2.0,
            }),
            driving_bone_name: "bone_3".to_string(),
            ..Default::default()
        },
    ]));

    let caps = &find(&hkx, "hclCapsuleShape").members;
    let names: HashSet<&str> = caps.iter().map(|m| m.name.as_str()).collect();
    assert!(names.contains("radius"));
    assert!(!names.contains("smallRadius"), "smallRadius is tapered-only");
    // 1/|end-start|^2 = 1/100.
    assert!((f32_of(value(caps, "capLenSqrdInv")) - 0.01).abs() < 1e-6);

    let tc = &find(&hkx, "hclTaperedCapsuleShape").members;
    let names: HashSet<&str> = tc.iter().map(|m| m.name.as_str()).collect();
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
        assert!(names.contains(required), "tapered capsule missing '{required}'");
    }
    assert!((f32_of(value(tc, "smallRadius")) - 1.0).abs() < 1e-6);
    assert!((f32_of(value(tc, "bigRadius")) - 2.0).abs() < 1e-6);
}

#[test]
fn sim_cloth_members_flips_transform_map_passthrough_and_gravity() {
    let mut sim_cloth = grid_sim_cloth(vec![CollidableSetup {
        name: "Col".to_string(),
        shape: Some(CapsuleShapeSetup {
            start: [0.0; 4],
            end: [5.0, 0.0, 0.0, 0.0],
            big_radius: 1.0,
            small_radius: 1.0,
        }),
        driving_bone_name: "bone_2".to_string(),
        ..Default::default()
    }]);
    sim_cloth.collidable_transform_set_index = 1;
    sim_cloth.collidable_offsets = vec![[
        1.0, 0.0, 0.0, 1.5, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ]];
    sim_cloth.passthrough_members = vec![
        HkxMember {
            name: "maxParticleRadius".to_string(),
            value: HkxValue::F32(0.75),
        },
        HkxMember {
            name: "actions".to_string(),
            value: HkxValue::Array(vec![]),
        },
        // Collides with a member bake emits itself; bake must win.
        HkxMember {
            name: "totalMass".to_string(),
            value: HkxValue::F32(999.0),
        },
    ];
    let hkx = bake_one(sim_cloth);
    let scd = &find(&hkx, "hclSimClothData").members;

    let HkxValue::Array(tri_indices) = value(scd, "triangleIndices") else {
        panic!("triangleIndices not Array");
    };
    let HkxValue::Array(flips) = value(scd, "triangleFlips") else {
        panic!("triangleFlips not Array");
    };
    assert_eq!(flips.len(), tri_indices.len() / 3, "one flip byte per triangle");
    assert!(flips.iter().all(|v| *v == HkxValue::U8(0)));

    let HkxValue::Object(map) = value(scd, "collidableTransformMap") else {
        panic!("collidableTransformMap not Object");
    };
    let tsi = match value(map, "transformSetIndex") {
        HkxValue::U32(v) => *v as i64,
        HkxValue::I32(v) => *v as i64,
        other => panic!("transformSetIndex unexpected type: {other:?}"),
    };
    assert_eq!(tsi, 1, "bake must not hard-code transform set 0");
    let HkxValue::Array(offsets) = value(map, "offsets") else {
        panic!("offsets not Array");
    };
    assert_eq!(offsets.len(), 1);

    assert!((f32_of(value(scd, "maxParticleRadius")) - 0.75).abs() < 1e-6);
    value(scd, "actions");
    assert!((f32_of(value(scd, "totalMass")) - 999.0).abs() > 1.0);

    // Setup default gravity is Z-up m/s² and is forwarded verbatim.
    let HkxValue::Object(sim_info) = value(scd, "simulationInfo") else {
        panic!("simulationInfo not Object");
    };
    match value(sim_info, "gravity") {
        HkxValue::F32List(g) => {
            assert!((g[2] + 9.81).abs() < 0.01 && g[1].abs() < 0.01, "gravity {g:?}")
        }
        other => panic!("gravity wrong shape: {other:?}"),
    }
}

#[test]
fn reverse_strict_refuses_unknown_operator_lossy_keeps_it() {
    let s = |v: &str| HkxValue::String {
        value: v.to_string(),
        is_null: false,
    };
    let m = |name: &str, value: HkxValue| HkxMember {
        name: name.to_string(),
        value,
    };
    let cloth_obj = HkxObject {
        name: Some("#0000".to_string()),
        offset: 0,
        signature: 0,
        class_name: "hclClothData".to_string(),
        members: vec![
            m("name", s("")),
            m("simClothDatas", HkxValue::Array(vec![])),
            m("operators", HkxValue::Array(vec![HkxValue::Pointer(Some(1))])),
            m("clothStateDatas", HkxValue::Array(vec![])),
            m("bufferDefinitions", HkxValue::Array(vec![])),
            m("transformSetDefinitions", HkxValue::Array(vec![])),
        ],
    };
    let unknown_op = HkxObject {
        name: Some("#0001".to_string()),
        offset: 1,
        signature: 0,
        class_name: "hclMysteryFutureOperator".to_string(),
        members: vec![m("name", s("Mystery"))],
    };
    let file = HkxFile::from_tagxml(11, "hk_2014.1.0-r1", vec![cloth_obj, unknown_op]);
    let cloth = ClothData::from_hkx_file(&file).expect("from_hkx_file must find hclClothData");

    match reverse_cloth_data(&cloth) {
        Err(ReverseError::UnknownClass { kind, class_name }) => {
            assert_eq!(kind, "operator");
            assert_eq!(class_name, "hclMysteryFutureOperator");
        }
        Ok(_) => panic!("strict reverse must fail on unknown operator class"),
    }
    assert_eq!(reverse_cloth_data_lossy(&cloth).operator_setups.len(), 1);
}
