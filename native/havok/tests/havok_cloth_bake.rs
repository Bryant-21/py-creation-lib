use std::collections::HashSet;

use havok_native::cloth::bake::bake_cloth_setup;
use havok_native::cloth::reverse::{reverse_cloth_data, reverse_cloth_data_lossy};
use havok_native::cloth::runtime::ClothData;
use havok_native::cloth::setup::buffer_setup::BufferType;
use havok_native::cloth::setup::collidable_setup::CollidableSetup;
use havok_native::cloth::setup::constraint_setup::{
    BendStiffnessSetup, ConstraintSetupObject, OpaqueConstraintSetup, StandardLinkSetup,
    VolumeSetup,
};
use havok_native::cloth::setup::mesh::{SetupMesh, SimulationSetupMesh};
use havok_native::cloth::setup::operator_setup::{
    OpaqueOperatorSetup, OperatorSetupObject, SkinSetup,
};
use havok_native::cloth::setup::types::VertexFloatInput;
use havok_native::cloth::setup::{
    BufferSetupObject, ClothSetupObject, SimClothSetupObject, TransformSetSetupObject,
};
use havok_native::cloth::validate::validate_cloth_data;
use havok_native::hkx::model::HkxObject;
use havok_native::hkx::types::HkxValue;
use havok_native::hkx::{HkxFile, HkxMember};

const FIXTURE_PATH: &str = "tests/fixtures/cloth_setup_min.json";

fn line_mesh(n: usize, triangles: Vec<[u32; 3]>) -> SimulationSetupMesh {
    SimulationSetupMesh {
        positions: (0..n).map(|i| [i as f32, 0.0, 0.0, 0.0]).collect(),
        triangles,
        ..Default::default()
    }
}

/// 2×2 grid: 0-1 top, 2-3 bottom; triangles (0,1,2),(1,3,2) → 5 unique edges.
fn grid_mesh() -> SimulationSetupMesh {
    SimulationSetupMesh {
        positions: vec![
            [0.0, 1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0, 0.0],
        ],
        triangles: vec![[0, 1, 2], [1, 3, 2]],
        ..Default::default()
    }
}

fn standard_links() -> ConstraintSetupObject {
    ConstraintSetupObject::StandardLink(StandardLinkSetup {
        name: "Links".to_string(),
        stiffness: VertexFloatInput::constant(1.0),
        ..Default::default()
    })
}

fn single_cloth(sim_cloth: SimClothSetupObject) -> ClothSetupObject {
    ClothSetupObject {
        name: "Setup".to_string(),
        sim_cloth_setups: vec![sim_cloth],
        ..Default::default()
    }
}

fn grid_setup(extra_constraints: Vec<ConstraintSetupObject>) -> ClothSetupObject {
    let mut constraint_setups = vec![standard_links()];
    constraint_setups.extend(extra_constraints);
    single_cloth(SimClothSetupObject {
        name: "Grid".to_string(),
        simulation_mesh: Some(grid_mesh()),
        constraint_setups,
        ..Default::default()
    })
}

fn find<'a>(hkx: &'a HkxFile, class_name: &str) -> &'a HkxObject {
    hkx.objects()
        .iter()
        .find(|o| o.class_name == class_name)
        .unwrap_or_else(|| panic!("{class_name} not emitted"))
}

fn member<'a>(members: &'a [HkxMember], name: &str) -> &'a HkxValue {
    &members
        .iter()
        .find(|m| m.name == name)
        .unwrap_or_else(|| panic!("member '{name}' not found"))
        .value
}

fn array<'a>(value: &'a HkxValue) -> &'a [HkxValue] {
    match value {
        HkxValue::Array(items) => items,
        other => panic!("expected Array, got {other:?}"),
    }
}

fn fields(value: &HkxValue) -> &[HkxMember] {
    match value {
        HkxValue::Object(members) => members,
        other => panic!("expected Object, got {other:?}"),
    }
}

fn names(members: &[HkxMember]) -> HashSet<&str> {
    members.iter().map(|m| m.name.as_str()).collect()
}

fn as_u32(value: &HkxValue) -> u32 {
    match value {
        HkxValue::U32(v) => *v,
        HkxValue::U16(v) => *v as u32,
        other => panic!("expected integer, got {other:?}"),
    }
}

#[test]
fn min_fixture_json_round_trips_bakes_and_reverses() {
    let src = std::fs::read_to_string(FIXTURE_PATH).expect("cloth_setup_min.json fixture missing");
    let setup = ClothSetupObject::from_json(&src).expect("from_json failed");
    let v_src: serde_json::Value = serde_json::from_str(&src).unwrap();
    let v_out: serde_json::Value = serde_json::from_str(&setup.to_json().unwrap()).unwrap();
    assert_eq!(v_src, v_out, "JSON round-trip mismatch");

    let hkx = bake_cloth_setup(&setup).expect("bake failed");
    assert!(hkx.objects().iter().any(|o| o.class_name == "hkRootLevelContainer"));
    let cloth_data_count = hkx
        .objects()
        .iter()
        .filter(|o| o.class_name == "hclClothData")
        .count();
    assert_eq!(cloth_data_count, 1);

    let cloth = ClothData::from_hkx_file(&hkx).expect("baked HKX has hclClothData");
    let reversed = reverse_cloth_data(&cloth).expect("strict reverse on min fixture");
    assert_eq!(reversed.buffer_setups.len(), setup.buffer_setups.len());
    assert_eq!(reversed.transform_set_setups.len(), setup.transform_set_setups.len());
    assert_eq!(reversed.sim_cloth_setups.len(), setup.sim_cloth_setups.len());
    assert!(reversed.operator_setups.len() >= setup.operator_setups.len());
}

#[test]
fn bake_rejects_invalid_setups() {
    assert!(bake_cloth_setup(&ClothSetupObject::default()).is_err());

    // 65,536 = u16::MAX + 1 particles.
    let too_many = single_cloth(SimClothSetupObject {
        simulation_mesh: Some(line_mesh(65536, vec![[0, 1, 2], [1, 3, 2]])),
        constraint_setups: vec![standard_links()],
        particle_mass: VertexFloatInput::constant(0.1),
        ..Default::default()
    });
    let msg = bake_cloth_setup(&too_many).unwrap_err().to_string();
    assert!(msg.contains("65536") || msg.contains("particle"), "{msg}");
}

/// Member shapes that silently break the runtime parse when wrong.
#[test]
fn baked_grid_matches_sdk_member_shapes() {
    let setup = grid_setup(vec![
        ConstraintSetupObject::BendStiffness(BendStiffnessSetup {
            name: "Bend".to_string(),
            bend_stiffness: VertexFloatInput::constant(0.5),
            ..Default::default()
        }),
        ConstraintSetupObject::Volume(VolumeSetup {
            name: "Vol".to_string(),
            ..Default::default()
        }),
    ]);
    let hkx = bake_cloth_setup(&setup).expect("bake failed");

    // HCL_PLATFORM_X64 = 2; 0 (invalid) may disable cloth.
    assert_eq!(
        member(&find(&hkx, "hclClothData").members, "targetPlatform"),
        &HkxValue::U32(2)
    );

    let scd = find(&hkx, "hclSimClothData");
    let sim_info = names(fields(member(&scd.members, "simulationInfo")));
    assert_eq!(sim_info, HashSet::from(["gravity", "globalDampingPerSecond"]));
    let scd_names = names(&scd.members);
    assert!(scd_names.contains("pinchDetectionEnabled"));
    assert!(scd_names.contains("transferMotionEnabled"));
    assert!(!scd_names.contains("numConstraints"));

    // SDK class has no `Set` suffix and no stiffness field.
    assert!(!hkx.objects().iter().any(|o| o.class_name == "hclVolumeConstraintSet"));
    let volume = names(&find(&hkx, "hclVolumeConstraint").members);
    assert!(volume.contains("frameDatas") && volume.contains("applyDatas"));
    assert!(!volume.contains("stiffness"));

    let bend_links = array(member(&find(&hkx, "hclBendStiffnessConstraintSet").members, "links"));
    assert!(!bend_links.is_empty());
    assert_eq!(
        names(fields(&bend_links[0])),
        HashSet::from([
            "particleA",
            "particleB",
            "particleC",
            "particleD",
            "weightA",
            "weightB",
            "weightC",
            "weightD",
            "restCurvature",
            "bendStiffness",
        ])
    );
}

#[test]
fn standard_links_cover_each_edge_in_disjoint_batches() {
    let hkx = bake_cloth_setup(&grid_setup(vec![])).expect("bake failed");
    let cset = find(&hkx, "hclStandardLinkConstraintSet");
    let links: Vec<(u32, u32)> = array(member(&cset.members, "links"))
        .iter()
        .map(|l| {
            let f = fields(l);
            (as_u32(member(f, "particleA")), as_u32(member(f, "particleB")))
        })
        .collect();
    assert_eq!(links.len(), 5, "one link per unique grid edge");

    for (bi, batch) in array(member(&cset.members, "batches")).iter().enumerate() {
        let f = fields(batch);
        let start = as_u32(member(f, "startLink"));
        let num = as_u32(member(f, "numLinks"));
        let mut seen = HashSet::new();
        for li in start..start + num {
            let (a, b) = links[li as usize];
            assert!(seen.insert(a) && seen.insert(b), "batch {bi} reuses a particle");
        }
    }
}

#[test]
fn skin_weights_bin_by_bone_count() {
    let weights = |n: usize| -> Vec<[f32; 2]> {
        (0..n).map(|b| [b as f32, 1.0 / n as f32]).collect()
    };
    // ≤5 bones → five, 6 → six, 7 → seven, 8 → eight.
    let bone_weights = vec![weights(2), weights(5), weights(6), weights(7), weights(8)];
    let positions = vec![
        [0.0, 0.0, 0.0, 1.0],
        [1.0, 0.0, 0.0, 1.0],
        [0.0, 1.0, 0.0, 1.0],
        [1.0, 1.0, 0.0, 1.0],
        [0.5, 0.5, 1.0, 1.0],
    ];
    let triangles = vec![[0, 1, 2], [1, 3, 2], [0, 2, 4]];
    let source_mesh = Box::new(SetupMesh {
        bone_names: (0..8).map(|i| format!("Bone_{i}")).collect(),
        bone_weights,
        positions: positions.clone(),
        triangles: triangles.clone(),
        ..Default::default()
    });
    let setup = ClothSetupObject {
        name: "SkinWeightSetup".to_string(),
        sim_cloth_setups: vec![SimClothSetupObject {
            name: "SkinCloth".to_string(),
            simulation_mesh: Some(SimulationSetupMesh {
                positions,
                triangles,
                source_mesh: Some(source_mesh),
                ..Default::default()
            }),
            ..Default::default()
        }],
        transform_set_setups: vec![TransformSetSetupObject {
            name: "BoneTS".to_string(),
            ..Default::default()
        }],
        buffer_setups: vec![BufferSetupObject {
            name: "OutBuf".to_string(),
            ..Default::default()
        }],
        operator_setups: vec![OperatorSetupObject::Skin(SkinSetup {
            name: "SkinOp".to_string(),
            transform_set_name: "BoneTS".to_string(),
            output_buffer_name: "OutBuf".to_string(),
            ..Default::default()
        })],
        ..Default::default()
    };

    let hkx = bake_cloth_setup(&setup).expect("bake must succeed with skin weights");
    let skin = &find(&hkx, "hclObjectSpaceSkinPNOperator").members;
    for (bin, expected) in [
        ("fiveBoneEntries", 2),
        ("sixBoneEntries", 1),
        ("sevenBoneEntries", 1),
        ("eightBoneEntries", 1),
    ] {
        assert_eq!(array(member(skin, bin)).len(), expected, "{bin}");
    }
    let transform_indices: Vec<u32> = array(member(skin, "transformIndices"))
        .iter()
        .map(as_u32)
        .collect();
    assert_eq!(transform_indices, (0..8).collect::<Vec<u32>>());
}

#[test]
fn buffer_types_map_to_runtime_type() {
    let mut setup = grid_setup(vec![]);
    setup.buffer_setups = [
        BufferType::Display,
        BufferType::StaticDisplay,
        BufferType::SimCloth,
        BufferType::Scratch,
    ]
    .iter()
    .enumerate()
    .map(|(i, &bt)| BufferSetupObject {
        name: format!("buf{i}"),
        buffer_type: bt as u8,
        ..Default::default()
    })
    .collect();
    let hkx = bake_cloth_setup(&setup).expect("bake failed");

    let mut types: Vec<(u32, &str)> = hkx
        .objects()
        .iter()
        .filter(|o| {
            o.class_name == "hclBufferDefinition" || o.class_name == "hclScratchBufferDefinition"
        })
        .map(|o| (as_u32(member(&o.members, "type")), o.class_name.as_str()))
        .collect();
    types.sort();
    assert_eq!(
        types,
        vec![
            (0, "hclScratchBufferDefinition"),
            (1, "hclBufferDefinition"),
            (2, "hclBufferDefinition"),
            (6, "hclBufferDefinition"),
        ]
    );
}

#[test]
fn particle_mass_channel_rescale_and_zero_mass_autofix() {
    let mut channel_mesh = line_mesh(3, vec![[0, 1, 2]]);
    channel_mesh
        .vertex_float_channels
        .insert("mass".to_string(), vec![0.1, 0.2, 0.3]);
    let hkx = bake_cloth_setup(&single_cloth(SimClothSetupObject {
        simulation_mesh: Some(channel_mesh),
        particle_mass: VertexFloatInput::channel("mass"),
        ..Default::default()
    }))
    .expect("bake with mass channel");
    let masses: Vec<f32> = array(member(&find(&hkx, "hclSimClothData").members, "particleDatas"))
        .iter()
        .map(|p| match member(fields(p), "mass") {
            HkxValue::F32(f) => *f,
            other => panic!("mass not F32: {other:?}"),
        })
        .collect();
    assert_eq!(masses.len(), 3);
    for (got, want) in masses.iter().zip([0.1, 0.2, 0.3]) {
        assert!((got - want).abs() < 1e-5, "mass {got} != {want}");
    }

    let hkx = bake_cloth_setup(&single_cloth(SimClothSetupObject {
        simulation_mesh: Some(line_mesh(3, vec![[0, 1, 2]])),
        particle_mass: VertexFloatInput::constant(0.1),
        rescale_mass: true,
        total_mass: 1.5,
        ..Default::default()
    }))
    .expect("bake with rescale_mass");
    match member(&find(&hkx, "hclSimClothData").members, "totalMass") {
        HkxValue::F32(tm) => assert!((tm - 1.5).abs() < 1e-4, "totalMass {tm}"),
        other => panic!("totalMass not F32: {other:?}"),
    }

    let hkx = bake_cloth_setup(&single_cloth(SimClothSetupObject {
        simulation_mesh: Some(line_mesh(4, vec![[0, 1, 2], [1, 3, 2]])),
        constraint_setups: vec![standard_links()],
        particle_mass: VertexFloatInput::constant(0.0),
        ..Default::default()
    }))
    .expect("bake with zero-mass particles");
    let result = validate_cloth_data(ClothData::from_hkx_file(&hkx).as_ref());
    assert!(
        !result.warnings().iter().any(|i| i.code == "ZERO_MASS_MOVABLE"),
        "zero-mass movable particles must be auto-pinned"
    );
}

#[test]
fn opaque_classes_survive_bake_and_reverse() {
    let setup = ClothSetupObject {
        name: "OpaqueSetup".to_string(),
        sim_cloth_setups: vec![SimClothSetupObject {
            name: "OpaqueCloth".to_string(),
            simulation_mesh: Some(line_mesh(3, vec![[0, 1, 2]])),
            constraint_setups: vec![ConstraintSetupObject::Opaque(OpaqueConstraintSetup {
                name: "MyOpaqueCSet".to_string(),
                class_name: "hclSentinelFakeConstraintSet".to_string(),
                members: vec![HkxMember {
                    name: "sentinelConstraintField".to_string(),
                    value: HkxValue::U32(99),
                }],
            })],
            ..Default::default()
        }],
        operator_setups: vec![OperatorSetupObject::Opaque(OpaqueOperatorSetup {
            name: "MyOpaqueOp".to_string(),
            class_name: "hclSentinelFakeOperator".to_string(),
            members: vec![HkxMember {
                name: "sentinelField".to_string(),
                value: HkxValue::U32(42),
            }],
        })],
        ..Default::default()
    };
    let hkx = bake_cloth_setup(&setup).expect("bake with opaque classes");
    find(&hkx, "hclSentinelFakeOperator");
    find(&hkx, "hclSentinelFakeConstraintSet");

    let cloth = ClothData::from_hkx_file(&hkx).expect("baked HKX must parse as ClothData");
    let reversed = reverse_cloth_data_lossy(&cloth);
    assert!(reversed.operator_setups.iter().any(|op| {
        matches!(op, OperatorSetupObject::Opaque(s) if s.class_name == "hclSentinelFakeOperator")
    }));
    let rebaked = bake_cloth_setup(&reversed).expect("re-bake after reverse");
    find(&rebaked, "hclSentinelFakeOperator");
}

#[test]
fn collidable_bone_name_resolves_via_transform_set() {
    let setup = ClothSetupObject {
        name: "CollidableBoneSetup".to_string(),
        sim_cloth_setups: vec![SimClothSetupObject {
            name: "Cloth".to_string(),
            simulation_mesh: Some(line_mesh(3, vec![[0, 1, 2]])),
            collidable_setups: vec![CollidableSetup {
                name: "BodyCapsule".to_string(),
                driving_bone_name: "Spine1".to_string(),
                ..Default::default()
            }],
            collidable_transform_set_index: 0,
            ..Default::default()
        }],
        transform_set_setups: vec![TransformSetSetupObject {
            name: "BoneTS".to_string(),
            bone_names: ["Root", "Hip", "Spine1", "Chest"].map(String::from).to_vec(),
            skeleton_name: "Skeleton".to_string(),
        }],
        ..Default::default()
    };

    let hkx = bake_cloth_setup(&setup).expect("bake with named collidable bone");
    let map = fields(member(&find(&hkx, "hclSimClothData").members, "collidableTransformMap"));
    let indices: Vec<u32> = array(member(map, "transformIndices")).iter().map(as_u32).collect();
    assert_eq!(indices, vec![2], "Spine1 is transform 2");
}
