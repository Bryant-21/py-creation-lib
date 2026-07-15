use havok_native::cloth::bake::bake_cloth_setup;
use havok_native::cloth::reverse::reverse_cloth_data_lossy;
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
use havok_native::hkx::HkxMember;
use havok_native::hkx::types::HkxValue;

const FIXTURE_PATH: &str = "tests/fixtures/cloth_setup_min.json";

/// Build a minimal 2×2 grid setup: 4 particles, 2 triangles.
///
/// Grid layout:
///   0 --- 1
///   |  \  |
///   2 --- 3
///
/// Triangles: (0,1,2) and (1,3,2)
/// Edges from triangles: (0,1),(0,2),(1,2),(1,3),(2,3) — 5 unique edges.
fn make_grid_setup() -> ClothSetupObject {
    let positions = vec![
        [0.0f32, 1.0, 0.0, 0.0], // 0 top-left
        [1.0f32, 1.0, 0.0, 0.0], // 1 top-right
        [0.0f32, 0.0, 0.0, 0.0], // 2 bottom-left
        [1.0f32, 0.0, 0.0, 0.0], // 3 bottom-right
    ];
    let triangles = vec![[0u32, 1, 2], [1u32, 3, 2]];

    let sim_mesh = SimulationSetupMesh {
        positions,
        triangles,
        ..Default::default()
    };

    let constraint = ConstraintSetupObject::StandardLink(StandardLinkSetup {
        name: "Links".to_string(),
        stiffness: VertexFloatInput::constant(1.0),
        ..Default::default()
    });

    let sim_cloth = SimClothSetupObject {
        name: "Grid".to_string(),
        simulation_mesh: Some(sim_mesh),
        constraint_setups: vec![constraint],
        ..Default::default()
    };

    ClothSetupObject {
        name: "GridSetup".to_string(),
        sim_cloth_setups: vec![sim_cloth],
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------

#[test]
fn bake_min_fixture_produces_root_and_cloth_data() {
    let src = std::fs::read_to_string(FIXTURE_PATH).expect("cloth_setup_min.json fixture missing");
    let setup = ClothSetupObject::from_json(&src).expect("from_json failed");
    let hkx = bake_cloth_setup(&setup).expect("bake failed");

    let objects = hkx.objects();

    let root_count = objects
        .iter()
        .filter(|o| o.class_name == "hkRootLevelContainer")
        .count();
    assert!(
        root_count >= 1,
        "expected at least one hkRootLevelContainer, got {root_count}"
    );

    let cloth_data_count = objects
        .iter()
        .filter(|o| o.class_name == "hclClothData")
        .count();
    assert_eq!(
        cloth_data_count, 1,
        "expected exactly one hclClothData, got {cloth_data_count}"
    );
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------

#[test]
fn bake_empty_setup_returns_error() {
    let setup = ClothSetupObject::default();
    let result = bake_cloth_setup(&setup);
    assert!(
        result.is_err(),
        "expected Err for empty setup with no sim cloths"
    );
}

#[test]
fn sim_cloth_data_simulation_info_only_has_gravity_and_damping() {
    // SDK hclSimClothData::OverridableSimulationInfo declares only m_gravity +
    // m_globalDampingPerSecond. collisionTolerance, pinchDetectionEnabled, and
    // transferMotionEnabled belong on hclSimClothData itself (tolerance on
    // landscapeCollisionData).
    use std::collections::HashSet;
    let setup = make_grid_setup();
    let hkx = bake_cloth_setup(&setup).expect("bake failed");
    let scd = hkx
        .objects()
        .iter()
        .find(|o| o.class_name == "hclSimClothData")
        .expect("hclSimClothData present");
    let sim_info = scd
        .members
        .iter()
        .find(|m| m.name == "simulationInfo")
        .expect("simulationInfo missing");
    let HkxValue::Object(fields) = &sim_info.value else {
        panic!("simulationInfo not Object");
    };
    let names: HashSet<&str> = fields.iter().map(|m| m.name.as_str()).collect();
    let expected: HashSet<&str> = ["gravity", "globalDampingPerSecond"].into_iter().collect();
    assert_eq!(
        names, expected,
        "simulationInfo must contain only gravity + globalDampingPerSecond"
    );

    assert!(
        scd.members
            .iter()
            .any(|m| m.name == "pinchDetectionEnabled"),
        "pinchDetectionEnabled must be a top-level hclSimClothData member"
    );
    assert!(
        scd.members
            .iter()
            .any(|m| m.name == "transferMotionEnabled"),
        "transferMotionEnabled must be a top-level hclSimClothData member"
    );
}

/// Build a grid setup with a BendStiffness constraint added — the 2x2 grid
/// has two triangles sharing edge (1,2), so exactly one bend link should be
/// produced.
fn make_grid_setup_with_bend() -> ClothSetupObject {
    let mut setup = make_grid_setup();
    let bend = ConstraintSetupObject::BendStiffness(BendStiffnessSetup {
        name: "Bend".to_string(),
        bend_stiffness: VertexFloatInput::constant(0.5),
        ..Default::default()
    });
    setup.sim_cloth_setups[0].constraint_setups.push(bend);
    setup
}

fn make_grid_setup_with_volume() -> ClothSetupObject {
    let mut setup = make_grid_setup();
    let vol = ConstraintSetupObject::Volume(VolumeSetup {
        name: "Vol".to_string(),
        ..Default::default()
    });
    setup.sim_cloth_setups[0].constraint_setups.push(vol);
    setup
}

#[test]
fn volume_constraint_class_and_fields_match_sdk() {
    // SDK class is `hclVolumeConstraint` (no `Set` suffix), fields m_frameDatas
    // + m_applyDatas, no `stiffness` field. A misnamed `hclVolumeConstraintSet`
    // with a `stiffness` field fails the runtime parse silently.
    let setup = make_grid_setup_with_volume();
    let hkx = bake_cloth_setup(&setup).expect("bake failed");
    let objects = hkx.objects();
    assert!(
        objects
            .iter()
            .any(|o| o.class_name == "hclVolumeConstraint"),
        "volume constraint class must be hclVolumeConstraint"
    );
    assert!(
        !objects
            .iter()
            .any(|o| o.class_name == "hclVolumeConstraintSet"),
        "the misnamed hclVolumeConstraintSet must not be emitted"
    );
    let cset = objects
        .iter()
        .find(|o| o.class_name == "hclVolumeConstraint")
        .unwrap();
    assert!(
        cset.members.iter().any(|m| m.name == "frameDatas"),
        "frameDatas missing"
    );
    assert!(
        cset.members.iter().any(|m| m.name == "applyDatas"),
        "applyDatas missing"
    );
    assert!(
        cset.members.iter().all(|m| m.name != "stiffness"),
        "hclVolumeConstraint has no stiffness field"
    );
}

#[test]
fn bend_constraint_emits_full_sdk_shape() {
    // SDK hclBendStiffnessConstraintSet::Link has 10 members. restLength is not
    // on the SDK struct; particleC/D defaulting to 0 pulls every bend link
    // toward vertex 0.
    use std::collections::HashSet;
    let setup = make_grid_setup_with_bend();
    let hkx = bake_cloth_setup(&setup).expect("bake failed");
    let cset = hkx
        .objects()
        .iter()
        .find(|o| o.class_name == "hclBendStiffnessConstraintSet")
        .expect("hclBendStiffnessConstraintSet present");
    let links = cset
        .members
        .iter()
        .find(|m| m.name == "links")
        .expect("links missing");
    let HkxValue::Array(arr) = &links.value else {
        panic!("links not an Array");
    };
    assert!(
        !arr.is_empty(),
        "grid with bend setup must produce at least one bend link"
    );
    let HkxValue::Object(first) = &arr[0] else {
        panic!("link element not an Object");
    };
    let names: HashSet<&str> = first.iter().map(|m| m.name.as_str()).collect();
    let required: HashSet<&str> = [
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
    ]
    .into_iter()
    .collect();
    assert_eq!(names, required, "bend link member set must match SDK");
    assert!(
        !names.contains("restLength"),
        "restLength is not on bend link"
    );
}

#[test]
fn cloth_data_emits_target_platform() {
    // Without the platform stamp the runtime treats m_targetPlatform as
    // HCL_PLATFORM_INVALID (0) and may silently disable cloth simulation.
    // Bethesda PC: HCL_PLATFORM_X64 = 2.
    let setup = make_grid_setup();
    let hkx = bake_cloth_setup(&setup).expect("bake failed");
    let cloth = hkx
        .objects()
        .iter()
        .find(|o| o.class_name == "hclClothData")
        .expect("hclClothData present");
    let tp = cloth
        .members
        .iter()
        .find(|m| m.name == "targetPlatform")
        .expect("targetPlatform member missing");
    assert_eq!(
        tp.value,
        HkxValue::U32(2),
        "PC platform stamp must be HCL_PLATFORM_X64 = 2"
    );
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------

#[test]
fn bake_simple_grid_produces_constraint_links() {
    let setup = make_grid_setup();
    let hkx = bake_cloth_setup(&setup).expect("bake failed");

    let objects = hkx.objects();
    let cset = objects
        .iter()
        .find(|o| o.class_name == "hclStandardLinkConstraintSet")
        .expect("no hclStandardLinkConstraintSet object found");

    let links_member = cset
        .members
        .iter()
        .find(|m| m.name == "links")
        .expect("no 'links' member in hclStandardLinkConstraintSet");

    let link_count = match &links_member.value {
        HkxValue::Array(items) => items.len(),
        _ => panic!("links member is not an Array"),
    };

    // 2×2 grid triangles (0,1,2) and (1,3,2) produce edges:
    // (0,1),(0,2),(1,2),(1,3),(2,3) → 5 unique edges
    assert_eq!(
        link_count, 5,
        "expected 5 links for 2×2 grid, got {link_count}"
    );
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------

#[test]
fn bake_constraint_batches_are_disjoint() {
    let setup = make_grid_setup();
    let hkx = bake_cloth_setup(&setup).expect("bake failed");

    let objects = hkx.objects();

    for obj in objects
        .iter()
        .filter(|o| o.class_name == "hclStandardLinkConstraintSet")
    {
        let links_member = obj.members.iter().find(|m| m.name == "links");
        let batches_member = obj.members.iter().find(|m| m.name == "batches");

        let (Some(links_m), Some(batches_m)) = (links_member, batches_member) else {
            continue;
        };

        let links: Vec<(u16, u16)> = match &links_m.value {
            HkxValue::Array(items) => items
                .iter()
                .map(|item| match item {
                    HkxValue::Object(members) => {
                        let pa = members
                            .iter()
                            .find(|m| m.name == "particleA")
                            .map(|m| match &m.value {
                                HkxValue::U16(v) => *v,
                                _ => 0,
                            })
                            .unwrap_or(0);
                        let pb = members
                            .iter()
                            .find(|m| m.name == "particleB")
                            .map(|m| match &m.value {
                                HkxValue::U16(v) => *v,
                                _ => 0,
                            })
                            .unwrap_or(0);
                        (pa, pb)
                    }
                    _ => (0, 0),
                })
                .collect(),
            _ => vec![],
        };

        if links.is_empty() {
            continue;
        }

        let batches: Vec<(u32, u32)> = match &batches_m.value {
            HkxValue::Array(items) => items
                .iter()
                .map(|item| match item {
                    HkxValue::Object(members) => {
                        let start = members
                            .iter()
                            .find(|m| m.name == "startLink")
                            .map(|m| match &m.value {
                                HkxValue::U32(v) => *v,
                                _ => 0,
                            })
                            .unwrap_or(0);
                        let num = members
                            .iter()
                            .find(|m| m.name == "numLinks")
                            .map(|m| match &m.value {
                                HkxValue::U32(v) => *v,
                                _ => 0,
                            })
                            .unwrap_or(0);
                        (start, num)
                    }
                    _ => (0, 0),
                })
                .collect(),
            _ => vec![],
        };

        // Assert disjointness: within each batch, no particle index appears twice
        for (bi, (start, num)) in batches.iter().enumerate() {
            let mut seen_particles: std::collections::HashSet<u16> =
                std::collections::HashSet::new();
            for li in *start..(*start + *num) {
                let (pa, pb) = links[li as usize];
                assert!(
                    !seen_particles.contains(&pa),
                    "batch {bi}: particle {pa} appears in multiple links"
                );
                assert!(
                    !seen_particles.contains(&pb),
                    "batch {bi}: particle {pb} appears in multiple links"
                );
                seen_particles.insert(pa);
                seen_particles.insert(pb);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------

#[test]
fn bake_with_skin_weights_no_skin_operator_succeeds() {
    // A setup with bone_weights but no Skin operator bakes successfully;
    // the bone data is simply unused.
    let source_mesh = Box::new(SetupMesh {
        positions: vec![
            [0.0, 0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
        ],
        triangles: vec![[0, 1, 2]],
        bone_names: vec!["Bone_A".to_string()],
        bone_weights: vec![vec![[0.0, 1.0]], vec![[0.0, 1.0]], vec![[0.0, 1.0]]],
        ..Default::default()
    });

    let sim_mesh = SimulationSetupMesh {
        positions: vec![
            [0.0, 0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
        ],
        triangles: vec![[0, 1, 2]],
        source_mesh: Some(source_mesh),
        ..Default::default()
    };

    let sim_cloth = SimClothSetupObject {
        name: "SkinCloth".to_string(),
        simulation_mesh: Some(sim_mesh),
        ..Default::default()
    };

    let setup = ClothSetupObject {
        name: "SkinSetup".to_string(),
        sim_cloth_setups: vec![sim_cloth],
        ..Default::default()
    };

    bake_cloth_setup(&setup).expect("bake must succeed with bone_weights and no Skin operator");
}

// ---------------------------------------------------------------------------
// Helper: extract an Array member value from a named member on an HkxObject.
// ---------------------------------------------------------------------------

fn member_array<'a>(obj: &'a havok_native::hkx::model::HkxObject, name: &str) -> &'a [HkxValue] {
    match &obj
        .members
        .iter()
        .find(|m| m.name == name)
        .expect(&format!("member '{name}' not found"))
        .value
    {
        HkxValue::Array(items) => items.as_slice(),
        other => panic!("member '{name}' is not an Array, got {other:?}"),
    }
}

fn member_u16_list(obj: &havok_native::hkx::model::HkxObject, name: &str) -> Vec<u16> {
    let arr = member_array(obj, name);
    arr.iter()
        .map(|v| match v {
            HkxValue::U16(x) => *x,
            other => panic!("expected U16 in '{name}', got {other:?}"),
        })
        .collect()
}

/// Build a ClothSetupObject whose sim_cloth_setups[0].simulation_mesh.source_mesh.bone_weights
/// has vertices covering all four bin paths:
///   ≤4 bones (no entry emitted), 5 bones → fiveBoneEntries,
///   6 bones → sixBoneEntries, 7 bones → sevenBoneEntries, 8 bones → eightBoneEntries.
///
/// The setup also includes a Skin operator so bin_skin_weights is actually invoked.
fn build_skin_weights_test_setup() -> ClothSetupObject {
    // 8 bones total so all bin paths are reachable.
    let bone_names: Vec<String> = (0..8).map(|i| format!("Bone_{i}")).collect();

    // n_bones = pair count BEFORE padding: ≤5 → five, 6 → six, 7 → seven, 8 → eight.
    // Use: v0=2 (→five), v1=5 (→five), v2=6 (→six), v3=7 (→seven), v4=8 (→eight).
    let bone_weights: Vec<Vec<[f32; 2]>> = vec![
        // vertex 0: 2 bones → fiveBoneEntries
        vec![[0.0, 0.5], [1.0, 0.5]],
        // vertex 1: 5 bones → fiveBoneEntries
        vec![[0.0, 0.3], [1.0, 0.25], [2.0, 0.2], [3.0, 0.15], [4.0, 0.1]],
        // vertex 2: 6 bones → sixBoneEntries
        vec![
            [0.0, 0.25],
            [1.0, 0.2],
            [2.0, 0.2],
            [3.0, 0.15],
            [4.0, 0.1],
            [5.0, 0.1],
        ],
        // vertex 3: 7 bones → sevenBoneEntries
        vec![
            [0.0, 0.2],
            [1.0, 0.17],
            [2.0, 0.15],
            [3.0, 0.13],
            [4.0, 0.12],
            [5.0, 0.12],
            [6.0, 0.11],
        ],
        // vertex 4: 8 bones → eightBoneEntries
        vec![
            [0.0, 0.15],
            [1.0, 0.14],
            [2.0, 0.13],
            [3.0, 0.12],
            [4.0, 0.12],
            [5.0, 0.12],
            [6.0, 0.11],
            [7.0, 0.11],
        ],
    ];

    let source_mesh = Box::new(SetupMesh {
        bone_names,
        bone_weights,
        positions: vec![
            [0.0, 0.0, 0.0, 1.0],
            [1.0, 0.0, 0.0, 1.0],
            [0.0, 1.0, 0.0, 1.0],
            [1.0, 1.0, 0.0, 1.0],
            [0.5, 0.5, 1.0, 1.0],
        ],
        triangles: vec![[0, 1, 2], [1, 3, 2], [0, 2, 4]],
        ..Default::default()
    });

    let sim_mesh = SimulationSetupMesh {
        positions: vec![
            [0.0, 0.0, 0.0, 1.0],
            [1.0, 0.0, 0.0, 1.0],
            [0.0, 1.0, 0.0, 1.0],
            [1.0, 1.0, 0.0, 1.0],
            [0.5, 0.5, 1.0, 1.0],
        ],
        triangles: vec![[0, 1, 2], [1, 3, 2], [0, 2, 4]],
        source_mesh: Some(source_mesh),
        ..Default::default()
    };

    let sim_cloth = SimClothSetupObject {
        name: "SkinCloth".to_string(),
        simulation_mesh: Some(sim_mesh),
        ..Default::default()
    };

    let transform_set = TransformSetSetupObject {
        name: "BoneTS".to_string(),
        ..Default::default()
    };

    let buffer = BufferSetupObject {
        name: "OutBuf".to_string(),
        ..Default::default()
    };

    let skin_op = OperatorSetupObject::Skin(SkinSetup {
        name: "SkinOp".to_string(),
        transform_set_name: "BoneTS".to_string(),
        output_buffer_name: "OutBuf".to_string(),
        ..Default::default()
    });

    ClothSetupObject {
        name: "SkinWeightSetup".to_string(),
        sim_cloth_setups: vec![sim_cloth],
        transform_set_setups: vec![transform_set],
        buffer_setups: vec![buffer],
        operator_setups: vec![skin_op],
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------

#[test]
fn bake_with_real_skin_weights_matches_python() {
    let setup = build_skin_weights_test_setup();
    let hkx = bake_cloth_setup(&setup).expect("bake must succeed with skin weights");

    let objects = hkx.objects();

    let skin_op = objects
        .iter()
        .find(|o| o.class_name == "hclObjectSpaceSkinPNOperator")
        .expect("hclObjectSpaceSkinPNOperator must be emitted");

    let five = member_array(skin_op, "fiveBoneEntries");
    let six = member_array(skin_op, "sixBoneEntries");
    let seven = member_array(skin_op, "sevenBoneEntries");
    let eight = member_array(skin_op, "eightBoneEntries");

    assert!(
        !(five.is_empty() && six.is_empty() && seven.is_empty() && eight.is_empty()),
        "no bone entries emitted — bin_skin_weights did not populate any bin"
    );

    // v0 (2 bones) + v1 (5 bones) → fiveBoneEntries (2 entries)
    assert_eq!(
        five.len(),
        2,
        "expected 2 fiveBoneEntries (vertices with ≤5 bones)"
    );
    // v2 (6 bones) → sixBoneEntries
    assert_eq!(six.len(), 1, "expected 1 sixBoneEntry");
    // v3 (7 bones) → sevenBoneEntries
    assert_eq!(seven.len(), 1, "expected 1 sevenBoneEntry");
    // v4 (8 bones) → eightBoneEntries
    assert_eq!(eight.len(), 1, "expected 1 eightBoneEntry");

    // transformIndices must equal the sorted list of unique bone indices used.
    // Vertices use bones 0..7, so transformIndices should be [0,1,2,3,4,5,6,7].
    let ti = member_u16_list(skin_op, "transformIndices");
    assert_eq!(
        ti,
        vec![0, 1, 2, 3, 4, 5, 6, 7],
        "transformIndices mismatch"
    );
}

// ---------------------------------------------------------------------------
// Each setup BufferType maps to a distinct runtime `m_type` value.
// ---------------------------------------------------------------------------

fn make_setup_with_buffer_types(types: &[BufferType]) -> ClothSetupObject {
    let buffers = types
        .iter()
        .enumerate()
        .map(|(i, &bt)| BufferSetupObject {
            name: format!("buf{i}"),
            buffer_type: bt as u8,
            ..Default::default()
        })
        .collect();

    // Minimal sim cloth with a tiny grid so bake can complete.
    let positions = vec![
        [0.0f32, 0.0, 0.0, 0.0],
        [1.0f32, 0.0, 0.0, 0.0],
        [0.0f32, 1.0, 0.0, 0.0],
        [1.0f32, 1.0, 0.0, 0.0],
    ];
    let triangles = vec![[0u32, 1, 2], [1u32, 3, 2]];
    let sim_mesh = SimulationSetupMesh {
        positions,
        triangles,
        ..Default::default()
    };
    let constraint = ConstraintSetupObject::StandardLink(StandardLinkSetup {
        name: "links".to_string(),
        stiffness: VertexFloatInput::constant(1.0),
        ..Default::default()
    });
    let sim_cloth = SimClothSetupObject {
        name: "SimCloth".to_string(),
        simulation_mesh: Some(sim_mesh),
        constraint_setups: vec![constraint],
        ..Default::default()
    };

    ClothSetupObject {
        name: "Setup".to_string(),
        buffer_setups: buffers,
        sim_cloth_setups: vec![sim_cloth],
        ..Default::default()
    }
}

fn get_buffer_type_vals(hkx: &havok_native::hkx::HkxFile) -> Vec<u32> {
    hkx.objects()
        .iter()
        .filter(|o| {
            o.class_name == "hclBufferDefinition" || o.class_name == "hclScratchBufferDefinition"
        })
        .filter_map(|o| {
            o.members
                .iter()
                .find(|m| m.name == "type")
                .map(|m| match m.value {
                    HkxValue::U32(v) => v,
                    _ => u32::MAX,
                })
        })
        .collect()
}

#[test]
fn buffer_type_display_emits_runtime_type_1() {
    let setup = make_setup_with_buffer_types(&[BufferType::Display]);
    let hkx = bake_cloth_setup(&setup).expect("bake failed");
    let types = get_buffer_type_vals(&hkx);
    assert_eq!(
        types,
        vec![1],
        "Display must emit runtime type 1, got {types:?}"
    );
}

#[test]
fn buffer_type_static_display_emits_runtime_type_2() {
    let setup = make_setup_with_buffer_types(&[BufferType::StaticDisplay]);
    let hkx = bake_cloth_setup(&setup).expect("bake failed");
    let types = get_buffer_type_vals(&hkx);
    assert_eq!(
        types,
        vec![2],
        "StaticDisplay must emit runtime type 2, got {types:?}"
    );
}

#[test]
fn buffer_type_sim_cloth_emits_runtime_type_6() {
    let setup = make_setup_with_buffer_types(&[BufferType::SimCloth]);
    let hkx = bake_cloth_setup(&setup).expect("bake failed");
    let types = get_buffer_type_vals(&hkx);
    assert_eq!(
        types,
        vec![6],
        "SimCloth must emit runtime type 6, got {types:?}"
    );
}

#[test]
fn buffer_type_scratch_emits_distinct_class_and_type_0() {
    let setup = make_setup_with_buffer_types(&[BufferType::Scratch]);
    let hkx = bake_cloth_setup(&setup).expect("bake failed");
    // Scratch uses hclScratchBufferDefinition with type=0
    let scratch_objs: Vec<_> = hkx
        .objects()
        .iter()
        .filter(|o| o.class_name == "hclScratchBufferDefinition")
        .collect();
    assert_eq!(
        scratch_objs.len(),
        1,
        "expected one hclScratchBufferDefinition"
    );
    let types = get_buffer_type_vals(&hkx);
    assert_eq!(
        types,
        vec![0],
        "Scratch must emit runtime type 0, got {types:?}"
    );
}

// ---------------------------------------------------------------------------
// No fabricated numConstraints field on hclSimClothData.
// ---------------------------------------------------------------------------

#[test]
fn bake_sim_cloth_has_no_num_constraints_member() {
    let setup = make_grid_setup();
    let hkx = bake_cloth_setup(&setup).expect("bake failed");
    for obj in hkx
        .objects()
        .iter()
        .filter(|o| o.class_name == "hclSimClothData")
    {
        assert!(
            !obj.members.iter().any(|m| m.name == "numConstraints"),
            "hclSimClothData must not have fabricated numConstraints member"
        );
    }
}

#[test]
fn all_four_buffer_types_produce_distinct_m_type_values() {
    let setup = make_setup_with_buffer_types(&[
        BufferType::Display,
        BufferType::StaticDisplay,
        BufferType::SimCloth,
        BufferType::Scratch,
    ]);
    let hkx = bake_cloth_setup(&setup).expect("bake failed");
    let mut types = get_buffer_type_vals(&hkx);
    types.sort();
    assert_eq!(
        types,
        vec![0, 1, 2, 6],
        "all 4 types must be distinct, got {types:?}"
    );
}

// ---------------------------------------------------------------------------
// Fixed-particle index range + zero-mass-movable auto-fix.
// ---------------------------------------------------------------------------

fn make_cloth_with_n_particles(n: usize, particle_mass: f32) -> ClothSetupObject {
    let positions: Vec<[f32; 4]> = (0..n).map(|i| [i as f32, 0.0, 0.0, 0.0]).collect();
    // Need at least 2 triangles (4 particles)
    let triangles = if n >= 4 {
        vec![[0u32, 1, 2], [1u32, 3, 2]]
    } else {
        vec![]
    };
    let sim_mesh = SimulationSetupMesh {
        positions,
        triangles,
        ..Default::default()
    };
    let constraint = ConstraintSetupObject::StandardLink(StandardLinkSetup {
        name: "links".to_string(),
        stiffness: VertexFloatInput::constant(1.0),
        ..Default::default()
    });
    let sim_cloth = SimClothSetupObject {
        name: "SimCloth".to_string(),
        simulation_mesh: Some(sim_mesh),
        constraint_setups: vec![constraint],
        particle_mass: VertexFloatInput::constant(particle_mass),
        ..Default::default()
    };
    ClothSetupObject {
        name: "Setup".to_string(),
        sim_cloth_setups: vec![sim_cloth],
        ..Default::default()
    }
}

#[test]
fn bake_65536_particles_returns_error() {
    // 65,536 = u16::MAX + 1 — must be rejected
    let setup = make_cloth_with_n_particles(65536, 0.1);
    let result = bake_cloth_setup(&setup);
    assert!(
        result.is_err(),
        "65536 particles must be rejected (exceeds u16::MAX)"
    );
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("65536") || msg.contains("particle"),
        "error message must mention particle count: {msg}"
    );
}

#[test]
fn zero_mass_movable_particle_is_auto_fixed_to_pinned() {
    use havok_native::cloth::runtime::ClothData;
    use havok_native::cloth::validate::validate_cloth_data;

    // 4 particles, particle_mass=0 → all should be auto-promoted to fixed
    let setup = make_cloth_with_n_particles(4, 0.0);
    let hkx = bake_cloth_setup(&setup).expect("bake with zero-mass particles succeeded");
    let cloth_data = ClothData::from_hkx_file(&hkx);
    let result = validate_cloth_data(cloth_data.as_ref());
    let zero_mass_warnings: Vec<_> = result
        .warnings()
        .into_iter()
        .filter(|i| i.code == "ZERO_MASS_MOVABLE")
        .collect();
    assert!(
        zero_mass_warnings.is_empty(),
        "zero-mass movable particles must be auto-fixed; ZERO_MASS_MOVABLE warnings: {zero_mass_warnings:?}"
    );
}

// ---------------------------------------------------------------------------
// Authorable mass / rescale_mass support.
// ---------------------------------------------------------------------------

fn make_grid_setup_with_mass_channel(masses: Vec<f32>) -> ClothSetupObject {
    use havok_native::cloth::setup::types::VertexFloatInput;
    use std::collections::HashMap;

    let n = masses.len();
    let positions: Vec<[f32; 4]> = (0..n).map(|i| [i as f32, 0.0, 0.0, 0.0]).collect();
    let triangles = if n >= 3 { vec![[0u32, 1, 2]] } else { vec![] };

    let mut float_channels: HashMap<String, Vec<f32>> = HashMap::new();
    float_channels.insert("mass".to_string(), masses);

    let sim_mesh = SimulationSetupMesh {
        positions,
        triangles,
        vertex_float_channels: float_channels,
        ..Default::default()
    };

    let sim_cloth = SimClothSetupObject {
        name: "MassChannel".to_string(),
        simulation_mesh: Some(sim_mesh),
        particle_mass: VertexFloatInput::channel("mass"),
        ..Default::default()
    };

    ClothSetupObject {
        name: "MassChannelSetup".to_string(),
        sim_cloth_setups: vec![sim_cloth],
        ..Default::default()
    }
}

#[test]
fn particle_mass_channel_is_read_from_mesh() {
    // 3 free particles with distinct masses provided via a channel.
    let setup = make_grid_setup_with_mass_channel(vec![0.1, 0.2, 0.3]);
    let hkx = bake_cloth_setup(&setup).expect("bake with mass channel succeeded");

    let objects = hkx.objects();
    let sc = objects
        .iter()
        .find(|o| o.class_name == "hclSimClothData")
        .expect("hclSimClothData not found");

    let particles = sc
        .members
        .iter()
        .find(|m| m.name == "particleDatas")
        .expect("particleDatas member not found");

    if let HkxValue::Array(arr) = &particles.value {
        let mass_vals: Vec<f32> = arr
            .iter()
            .filter_map(|v| {
                if let HkxValue::Object(members) = v {
                    members.iter().find(|m| m.name == "mass").and_then(|m| {
                        if let HkxValue::F32(f) = m.value {
                            Some(f)
                        } else {
                            None
                        }
                    })
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(mass_vals.len(), 3, "expected 3 particle mass entries");
        assert!(
            (mass_vals[0] - 0.1).abs() < 1e-5,
            "particle 0 mass should be 0.1, got {}",
            mass_vals[0]
        );
        assert!(
            (mass_vals[1] - 0.2).abs() < 1e-5,
            "particle 1 mass should be 0.2, got {}",
            mass_vals[1]
        );
        assert!(
            (mass_vals[2] - 0.3).abs() < 1e-5,
            "particle 2 mass should be 0.3, got {}",
            mass_vals[2]
        );
    } else {
        panic!("particles is not an Array");
    }
}

#[test]
fn rescale_mass_normalizes_total_mass() {
    use havok_native::cloth::setup::types::VertexFloatInput;

    // 3 free particles with mass 0.1 each, rescale to total_mass=1.5
    let positions = vec![
        [0.0f32, 0.0, 0.0, 0.0],
        [1.0f32, 0.0, 0.0, 0.0],
        [0.5f32, 1.0, 0.0, 0.0],
    ];
    let triangles = vec![[0u32, 1, 2]];
    let sim_mesh = SimulationSetupMesh {
        positions,
        triangles,
        ..Default::default()
    };
    let sim_cloth = SimClothSetupObject {
        name: "Rescale".to_string(),
        simulation_mesh: Some(sim_mesh),
        particle_mass: VertexFloatInput::constant(0.1),
        rescale_mass: true,
        total_mass: 1.5,
        ..Default::default()
    };
    let setup = ClothSetupObject {
        name: "RescaleSetup".to_string(),
        sim_cloth_setups: vec![sim_cloth],
        ..Default::default()
    };

    let hkx = bake_cloth_setup(&setup).expect("bake with rescale_mass succeeded");
    let objects = hkx.objects();
    let sc = objects
        .iter()
        .find(|o| o.class_name == "hclSimClothData")
        .expect("hclSimClothData not found");

    let total_mass_member = sc
        .members
        .iter()
        .find(|m| m.name == "totalMass")
        .expect("totalMass member not found");

    if let HkxValue::F32(tm) = total_mass_member.value {
        assert!((tm - 1.5).abs() < 1e-4, "totalMass should be 1.5, got {tm}");
    } else {
        panic!("totalMass is not F32");
    }
}

// ---------------------------------------------------------------------------
// Opaque operator/constraint pass-through round-trip.
// ---------------------------------------------------------------------------

/// Build a setup that has one OpaqueOperatorSetup with a sentinel class name.
fn make_opaque_operator_setup() -> ClothSetupObject {
    let positions = vec![
        [0.0f32, 0.0, 0.0, 0.0],
        [1.0f32, 0.0, 0.0, 0.0],
        [0.5f32, 1.0, 0.0, 0.0],
    ];
    let triangles = vec![[0u32, 1, 2]];
    let sim_mesh = SimulationSetupMesh {
        positions,
        triangles,
        ..Default::default()
    };
    let sim_cloth = SimClothSetupObject {
        name: "OpaqueCloth".to_string(),
        simulation_mesh: Some(sim_mesh),
        ..Default::default()
    };

    let opaque_op = OperatorSetupObject::Opaque(OpaqueOperatorSetup {
        name: "MyOpaqueOp".to_string(),
        class_name: "hclSentinelFakeOperator".to_string(),
        members: vec![HkxMember {
            name: "sentinelField".to_string(),
            value: HkxValue::U32(42),
        }],
    });

    ClothSetupObject {
        name: "OpaqueOpSetup".to_string(),
        sim_cloth_setups: vec![sim_cloth],
        operator_setups: vec![opaque_op],
        ..Default::default()
    }
}

#[test]
fn opaque_operator_class_name_survives_bake() {
    let setup = make_opaque_operator_setup();
    let hkx = bake_cloth_setup(&setup).expect("bake with opaque operator should succeed");
    let objects = hkx.objects();

    let sentinel = objects
        .iter()
        .find(|o| o.class_name == "hclSentinelFakeOperator");
    assert!(
        sentinel.is_some(),
        "hclSentinelFakeOperator must appear in baked HKX; objects: {:?}",
        objects.iter().map(|o| &o.class_name).collect::<Vec<_>>()
    );
}

#[test]
fn opaque_operator_round_trips_through_reverse() {
    let setup = make_opaque_operator_setup();
    let hkx = bake_cloth_setup(&setup).expect("bake with opaque operator");

    let cloth_data = ClothData::from_hkx_file(&hkx).expect("baked HKX must parse as ClothData");
    let reversed = reverse_cloth_data_lossy(&cloth_data);

    let opaque = reversed.operator_setups.iter().find(|op| {
        matches!(op, OperatorSetupObject::Opaque(s) if s.class_name == "hclSentinelFakeOperator")
    });
    assert!(
        opaque.is_some(),
        "reverse_cloth_data_lossy must produce Opaque operator for unknown class; operators: {:?}",
        reversed
            .operator_setups
            .iter()
            .map(|o| o.name())
            .collect::<Vec<_>>()
    );

    // Re-bake: class name must still appear
    let hkx2 = bake_cloth_setup(&reversed).expect("re-bake after reverse must succeed");
    let objects2 = hkx2.objects();
    let sentinel2 = objects2
        .iter()
        .find(|o| o.class_name == "hclSentinelFakeOperator");
    assert!(
        sentinel2.is_some(),
        "class name must survive re-bake after reverse"
    );
}

/// Build a setup with an OpaqueConstraintSetup in the sim cloth's constraint list.
fn make_opaque_constraint_setup() -> ClothSetupObject {
    let positions = vec![
        [0.0f32, 0.0, 0.0, 0.0],
        [1.0f32, 0.0, 0.0, 0.0],
        [0.5f32, 1.0, 0.0, 0.0],
    ];
    let triangles = vec![[0u32, 1, 2]];
    let sim_mesh = SimulationSetupMesh {
        positions,
        triangles,
        ..Default::default()
    };

    let opaque_cset = ConstraintSetupObject::Opaque(OpaqueConstraintSetup {
        name: "MyOpaqueCSet".to_string(),
        class_name: "hclSentinelFakeConstraintSet".to_string(),
        members: vec![HkxMember {
            name: "sentinelConstraintField".to_string(),
            value: HkxValue::U32(99),
        }],
    });

    let sim_cloth = SimClothSetupObject {
        name: "OpaqueCSetCloth".to_string(),
        simulation_mesh: Some(sim_mesh),
        constraint_setups: vec![opaque_cset],
        ..Default::default()
    };

    ClothSetupObject {
        name: "OpaqueCSetSetup".to_string(),
        sim_cloth_setups: vec![sim_cloth],
        ..Default::default()
    }
}

#[test]
fn opaque_constraint_class_name_survives_bake() {
    let setup = make_opaque_constraint_setup();
    let hkx = bake_cloth_setup(&setup).expect("bake with opaque constraint should succeed");
    let objects = hkx.objects();

    let sentinel = objects
        .iter()
        .find(|o| o.class_name == "hclSentinelFakeConstraintSet");
    assert!(
        sentinel.is_some(),
        "hclSentinelFakeConstraintSet must appear in baked HKX; objects: {:?}",
        objects.iter().map(|o| &o.class_name).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Collidable bone name lookup uses the transform set, not a heuristic.
// ---------------------------------------------------------------------------

#[test]
fn collidable_bone_name_resolves_via_transform_set() {
    // Bone order in the transform set: ["Root", "Hip", "Spine1", "Chest"]
    // Collidable references "Spine1" → should get transform index 2, not 0.
    let bone_names = vec![
        "Root".to_string(),
        "Hip".to_string(),
        "Spine1".to_string(),
        "Chest".to_string(),
    ];

    let transform_set = TransformSetSetupObject {
        name: "BoneTS".to_string(),
        bone_names: bone_names.clone(),
        skeleton_name: "Skeleton".to_string(),
    };

    let sim_mesh = SimulationSetupMesh {
        positions: vec![
            [0.0f32, 0.0, 0.0, 0.0],
            [1.0f32, 0.0, 0.0, 0.0],
            [0.5f32, 1.0, 0.0, 0.0],
        ],
        triangles: vec![[0u32, 1, 2]],
        ..Default::default()
    };

    let collidable = CollidableSetup {
        name: "BodyCapsule".to_string(),
        driving_bone_name: "Spine1".to_string(),
        ..Default::default()
    };

    let sim_cloth = SimClothSetupObject {
        name: "Cloth".to_string(),
        simulation_mesh: Some(sim_mesh),
        collidable_setups: vec![collidable],
        collidable_transform_set_index: 0, // refers to transform_set_setups[0]
        ..Default::default()
    };

    let setup = ClothSetupObject {
        name: "CollidableBoneSetup".to_string(),
        sim_cloth_setups: vec![sim_cloth],
        transform_set_setups: vec![transform_set],
        ..Default::default()
    };

    let hkx = bake_cloth_setup(&setup).expect("bake with named collidable bone");
    let objects = hkx.objects();

    // Find the hclSimClothData and extract collidableTransformMap.transformIndices.
    let sc = objects
        .iter()
        .find(|o| o.class_name == "hclSimClothData")
        .expect("hclSimClothData must be emitted");

    let ctm = sc
        .members
        .iter()
        .find(|m| m.name == "collidableTransformMap")
        .expect("collidableTransformMap must be present");

    let transform_indices = if let HkxValue::Object(members) = &ctm.value {
        members
            .iter()
            .find(|m| m.name == "transformIndices")
            .and_then(|m| {
                if let HkxValue::Array(arr) = &m.value {
                    Some(arr.clone())
                } else {
                    None
                }
            })
            .unwrap_or_default()
    } else {
        panic!("collidableTransformMap value is not Object");
    };

    assert_eq!(
        transform_indices.len(),
        1,
        "expected 1 transform index for 1 collidable"
    );

    let idx = match transform_indices[0] {
        HkxValue::U32(i) => i,
        ref v => panic!("expected U32, got {v:?}"),
    };

    assert_eq!(
        idx, 2,
        "Spine1 is at position 2 in the transform set; got index {idx}"
    );
}
