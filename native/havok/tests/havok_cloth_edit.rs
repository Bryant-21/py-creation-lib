// ClothEditor integration tests.
//
// All tests use synthetic HkxFile instances built in-process — no real
// packfile fixtures required. Each test targets one method on ClothEditor.

use havok_native::cloth::ClothEditor;
use havok_native::hkx::descriptors::DescriptorRegistry;
use havok_native::hkx::types::HkxValue;
use havok_native::hkx::{HkxFile, HkxMember, HkxObject, read_packfile, write_hkx};

// ---------------------------------------------------------------------------
// Synthetic file builders
// ---------------------------------------------------------------------------

fn make_particle(mass: f32, inv_mass: f32, radius: f32, friction: f32) -> HkxValue {
    HkxValue::Object(vec![
        HkxMember {
            name: "mass".to_string(),
            value: HkxValue::F32(mass),
        },
        HkxMember {
            name: "invMass".to_string(),
            value: HkxValue::F32(inv_mass),
        },
        HkxMember {
            name: "radius".to_string(),
            value: HkxValue::F32(radius),
        },
        HkxMember {
            name: "friction".to_string(),
            value: HkxValue::F32(friction),
        },
    ])
}

/// Build a link object with a `stiffness` member.
fn make_standard_link(stiffness: f32) -> HkxValue {
    HkxValue::Object(vec![HkxMember {
        name: "stiffness".to_string(),
        value: HkxValue::F32(stiffness),
    }])
}

/// Build a link object with a `bendStiffness` member.
fn make_bend_link(bend_stiffness: f32) -> HkxValue {
    HkxValue::Object(vec![HkxMember {
        name: "bendStiffness".to_string(),
        value: HkxValue::F32(bend_stiffness),
    }])
}

/// Build a gravity vector as F32List [x, y, z, w].
fn make_gravity(x: f32, y: f32, z: f32, w: f32) -> HkxValue {
    HkxValue::F32List(vec![x, y, z, w])
}

/// Build a full synthetic HkxFile with:
/// - hclClothData at index 0
/// - hclSimClothData at index 1 (pointed to by simClothDatas[0])
/// - hclStandardLinkConstraintSet at index 2 (pointed to by staticConstraintSets[0])
/// - hclBendStiffnessConstraintSet at index 3 (pointed to by staticConstraintSets[1])
/// - hclSimulateOperator at index 4 (pointed to by operators[0])
fn build_synthetic_file() -> HkxFile {
    // Object 1: hclStandardLinkConstraintSet with 2 links
    let standard_cs = HkxObject {
        name: Some("#0002".to_string()),
        offset: 0,
        signature: 0,
        class_name: "hclStandardLinkConstraintSet".to_string(),
        members: vec![HkxMember {
            name: "links".to_string(),
            value: HkxValue::Array(vec![make_standard_link(1.0), make_standard_link(1.0)]),
        }],
    };

    // Object 2: hclBendStiffnessConstraintSet with 1 link
    let bend_cs = HkxObject {
        name: Some("#0003".to_string()),
        offset: 0,
        signature: 0,
        class_name: "hclBendStiffnessConstraintSet".to_string(),
        members: vec![HkxMember {
            name: "links".to_string(),
            value: HkxValue::Array(vec![make_bend_link(0.5)]),
        }],
    };

    // Object 3: hclSimulateOperator
    let sim_op = HkxObject {
        name: Some("#0004".to_string()),
        offset: 0,
        signature: 0,
        class_name: "hclSimulateOperator".to_string(),
        members: vec![
            HkxMember {
                name: "subSteps".to_string(),
                value: HkxValue::U32(1),
            },
            HkxMember {
                name: "numberOfSolveIterations".to_string(),
                value: HkxValue::U32(4),
            },
        ],
    };

    // Object 4: hclSimClothData — 3 particles, 0 fixed, points to constraint sets
    let sim_cloth = HkxObject {
        name: Some("#0001".to_string()),
        offset: 0,
        signature: 0,
        class_name: "hclSimClothData".to_string(),
        members: vec![
            HkxMember {
                name: "particleDatas".to_string(),
                value: HkxValue::Array(vec![
                    make_particle(0.1, 10.0, 1.0, 0.2),
                    make_particle(0.1, 10.0, 1.0, 0.2),
                    make_particle(0.1, 10.0, 1.0, 0.2),
                ]),
            },
            HkxMember {
                name: "fixedParticles".to_string(),
                value: HkxValue::Array(vec![]),
            },
            // staticConstraintSets → [ptr→index 0 (StandardLink), ptr→index 1 (Bend)]
            HkxMember {
                name: "staticConstraintSets".to_string(),
                value: HkxValue::Array(vec![
                    HkxValue::Pointer(Some(0)), // standard_cs at index 0
                    HkxValue::Pointer(Some(1)), // bend_cs at index 1
                ]),
            },
            HkxMember {
                name: "perInstanceCollidables".to_string(),
                value: HkxValue::Array(vec![]),
            },
            HkxMember {
                name: "simClothPoses".to_string(),
                value: HkxValue::Array(vec![]),
            },
            // simulationInfo as inline struct
            HkxMember {
                name: "simulationInfo".to_string(),
                value: HkxValue::Object(vec![
                    HkxMember {
                        name: "gravity".to_string(),
                        value: make_gravity(0.0, 0.0, -9.8, 0.0),
                    },
                    HkxMember {
                        name: "globalDampingPerSecond".to_string(),
                        value: HkxValue::F32(0.1),
                    },
                    HkxMember {
                        name: "collisionTolerance".to_string(),
                        value: HkxValue::F32(0.05),
                    },
                ]),
            },
        ],
    };

    // Object 5: hclClothData — root, points to sim cloth and operators
    let cloth_data = HkxObject {
        name: Some("#0000".to_string()),
        offset: 0,
        signature: 0,
        class_name: "hclClothData".to_string(),
        members: vec![
            HkxMember {
                name: "name".to_string(),
                value: HkxValue::String {
                    value: "TestCloth".to_string(),
                    is_null: false,
                },
            },
            // simClothDatas → [ptr→index 3 (sim_cloth)]
            HkxMember {
                name: "simClothDatas".to_string(),
                value: HkxValue::Array(vec![
                    HkxValue::Pointer(Some(3)), // sim_cloth at index 3
                ]),
            },
            // operators → [ptr→index 2 (sim_op)]
            HkxMember {
                name: "operators".to_string(),
                value: HkxValue::Array(vec![
                    HkxValue::Pointer(Some(2)), // sim_op at index 2
                ]),
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

    // Object order: [standard_cs(0), bend_cs(1), sim_op(2), sim_cloth(3), cloth_data(4)]
    HkxFile::from_tagxml(
        11,
        "hk_2014.1.0-r1",
        vec![standard_cs, bend_cs, sim_op, sim_cloth, cloth_data],
    )
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------

#[test]
fn set_particle_mass_all_updates_mass_and_inv_mass() {
    let mut file = build_synthetic_file();
    let mut editor = ClothEditor::new(&mut file).expect("ClothEditor::new must succeed");

    let count = editor
        .set_particle_mass_all(0.05, 0)
        .expect("set_particle_mass_all must succeed");

    assert_eq!(count, 3, "all 3 movable particles should be modified");

    // Drop editor to release the mutable borrow, then inspect directly.
    drop(editor);

    // The hclSimClothData is at index 3 in the objects array.
    let sim_cloth = &file.objects()[3];
    let particles = match sim_cloth
        .members
        .iter()
        .find(|m| m.name == "particleDatas")
        .map(|m| &m.value)
    {
        Some(HkxValue::Array(arr)) => arr,
        _ => panic!("particleDatas array not found"),
    };

    assert_eq!(particles.len(), 3);
    for (i, p) in particles.iter().enumerate() {
        match p {
            HkxValue::Object(members) => {
                let mass = members
                    .iter()
                    .find(|m| m.name == "mass")
                    .map(|m| match &m.value {
                        HkxValue::F32(v) => *v,
                        _ => panic!("mass is not F32"),
                    })
                    .expect("mass member must exist");
                let inv_mass = members
                    .iter()
                    .find(|m| m.name == "invMass")
                    .map(|m| match &m.value {
                        HkxValue::F32(v) => *v,
                        _ => panic!("invMass is not F32"),
                    })
                    .expect("invMass member must exist");

                assert!(
                    (mass - 0.05_f32).abs() < 1e-6,
                    "particle {i}: expected mass=0.05, got {mass}"
                );
                assert!(
                    (inv_mass - 20.0_f32).abs() < 1e-4,
                    "particle {i}: expected invMass=20.0, got {inv_mass}"
                );
            }
            _ => panic!("particle {i} is not an Object value"),
        }
    }
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------

#[test]
fn scale_stiffness_filtered_by_short_name() {
    let mut file = build_synthetic_file();
    let mut editor = ClothEditor::new(&mut file).expect("ClothEditor::new must succeed");

    let count = editor
        .scale_stiffness(Some("standard"), 0.5, 0)
        .expect("scale_stiffness must succeed");

    assert_eq!(count, 2, "two StandardLink links should be modified");

    drop(editor);

    // StandardLinkConstraintSet is at index 0, BendStiffnessConstraintSet at index 1.
    let standard_cs = &file.objects()[0];
    let bend_cs = &file.objects()[1];

    // Assert StandardLink stiffness halved to 0.5
    let standard_links = match standard_cs
        .members
        .iter()
        .find(|m| m.name == "links")
        .map(|m| &m.value)
    {
        Some(HkxValue::Array(arr)) => arr,
        _ => panic!("links not found on standard CS"),
    };
    for (i, link) in standard_links.iter().enumerate() {
        if let HkxValue::Object(members) = link {
            let stiffness = members
                .iter()
                .find(|m| m.name == "stiffness")
                .map(|m| match &m.value {
                    HkxValue::F32(v) => *v,
                    _ => panic!("stiffness not F32"),
                })
                .expect("stiffness member");
            assert!(
                (stiffness - 0.5_f32).abs() < 1e-6,
                "link {i}: expected stiffness=0.5, got {stiffness}"
            );
        } else {
            panic!("link {i} is not Object");
        }
    }

    // Assert Bend stiffness unchanged at 0.5
    let bend_links = match bend_cs
        .members
        .iter()
        .find(|m| m.name == "links")
        .map(|m| &m.value)
    {
        Some(HkxValue::Array(arr)) => arr,
        _ => panic!("links not found on bend CS"),
    };
    if let HkxValue::Object(members) = &bend_links[0] {
        let bs = members
            .iter()
            .find(|m| m.name == "bendStiffness")
            .map(|m| match &m.value {
                HkxValue::F32(v) => *v,
                _ => panic!("bendStiffness not F32"),
            })
            .expect("bendStiffness member");
        assert!(
            (bs - 0.5_f32).abs() < 1e-6,
            "bend stiffness should be unchanged at 0.5, got {bs}"
        );
    } else {
        panic!("bend link is not Object");
    }
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------

#[test]
fn set_gravity_updates_simulation_info() {
    let mut file = build_synthetic_file();
    let mut editor = ClothEditor::new(&mut file).expect("ClothEditor::new must succeed");

    editor
        .set_gravity([0.0, 0.0, -686.7, 0.0], 0)
        .expect("set_gravity must succeed");

    drop(editor);

    // hclSimClothData is at index 3
    let sim_cloth = &file.objects()[3];
    let sim_info = sim_cloth
        .members
        .iter()
        .find(|m| m.name == "simulationInfo")
        .expect("simulationInfo must be present");

    if let HkxValue::Object(info_members) = &sim_info.value {
        let gravity_member = info_members
            .iter()
            .find(|m| m.name == "gravity")
            .expect("gravity member must exist");

        if let HkxValue::F32List(g) = &gravity_member.value {
            assert_eq!(g.len(), 4, "gravity must be a 4-component vector");
            assert!(
                (g[2] - (-686.7_f32)).abs() < 0.01,
                "gravity Z should be -686.7, got {}",
                g[2]
            );
        } else {
            panic!("gravity is not F32List, found {:?}", gravity_member.value);
        }
    } else {
        panic!("simulationInfo is not an Object");
    }
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------

#[test]
fn set_substeps_updates_simulate_operator() {
    let mut file = build_synthetic_file();
    let mut editor = ClothEditor::new(&mut file).expect("ClothEditor::new must succeed");

    editor.set_substeps(4).expect("set_substeps must succeed");

    drop(editor);

    // hclSimulateOperator is at index 2
    let sim_op = &file.objects()[2];
    let substeps_member = sim_op
        .members
        .iter()
        .find(|m| m.name == "subSteps")
        .expect("subSteps member must exist");

    let substeps = match &substeps_member.value {
        HkxValue::U32(v) => *v,
        HkxValue::I32(v) => *v as u32,
        other => panic!("subSteps has unexpected type: {:?}", other),
    };

    assert_eq!(substeps, 4, "subSteps should be 4 after set_substeps(4)");
}

// ---------------------------------------------------------------------------
// Helper: serialize an HkxFile to bytes via write_hkx and re-parse it.
// Used for the round-trip correctness gate in remove_capsule tests.
// ---------------------------------------------------------------------------
fn serialize_back(file: &HkxFile) -> Vec<u8> {
    let mut registry = DescriptorRegistry::new();
    write_hkx(file, &mut registry)
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------
#[test]
fn add_capsule_appends_shape_and_collidable() {
    let mut file = build_synthetic_file();
    let n_before = file.objects().len();

    {
        let mut editor = ClothEditor::new(&mut file).expect("ClothEditor::new must succeed");
        let new_idx = editor
            .add_capsule(
                "bone_5",
                1.5,
                [0.0, 0.0, 0.0, 0.0],
                [10.0, 0.0, 0.0, 0.0],
                0,
            )
            .expect("add_capsule must succeed");
        // new_idx is the position appended to perInstanceCollidables (0-based)
        assert_eq!(
            new_idx, 0,
            "first capsule should be at perInstanceCollidables[0]"
        );
    }

    let n_after = file.objects().len();
    assert_eq!(
        n_after,
        n_before + 2,
        "should append shape + collidable (2 new objects)"
    );

    // Confirm the new objects have the right class names (last two appended)
    let shape_obj = &file.objects()[n_before];
    let col_obj = &file.objects()[n_before + 1];
    assert_eq!(shape_obj.class_name, "hclCapsuleShape");
    assert_eq!(col_obj.class_name, "hclCollidable");
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------
#[test]
fn remove_capsule_drops_shape_and_collidable_and_remaps_pointers() {
    let mut file = build_synthetic_file();

    // Add one capsule so we have something to remove.
    {
        let mut editor = ClothEditor::new(&mut file).expect("editor");
        editor
            .add_capsule("bone_1", 1.0, [0.0; 4], [5.0, 0.0, 0.0, 0.0], 0)
            .expect("add_capsule must succeed");
    }

    let n_before = file.objects().len();

    // Remove the capsule we just added (index 0 in perInstanceCollidables).
    {
        let mut editor = ClothEditor::new(&mut file).expect("editor");
        editor
            .remove_capsule(0, 0)
            .expect("remove_capsule must succeed");
    }

    let n_after = file.objects().len();
    assert_eq!(
        n_after,
        n_before - 2,
        "remove_capsule should drop 2 objects (shape + collidable)"
    );

    // Round-trip: write to bytes and re-parse — validates that pointer remap kept the graph valid.
    let bytes = serialize_back(&file);
    read_packfile(&bytes)
        .expect("file must be re-parseable after remove_capsule (pointer remap gate)");
}
