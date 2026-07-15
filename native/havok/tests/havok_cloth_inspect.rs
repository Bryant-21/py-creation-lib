// cloth_inspect_full_json: parse blob into a complete UI-display JSON.

use havok_native;
use havok_native::api;
use havok_native::hkx::descriptors::DescriptorRegistry;
use havok_native::hkx::types::HkxValue;
use havok_native::hkx::{HkxFile, HkxMember, HkxObject, write_hkx};

// ---------------------------------------------------------------------------
// Synthetic file builder — mirrors havok_cloth_edit.rs but adds pose data
// and a capsule collidable so all JSON keys exercised.
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

fn make_standard_link(a: u32, b: u32, stiffness: f32) -> HkxValue {
    HkxValue::Object(vec![
        HkxMember {
            name: "particleA".to_string(),
            value: HkxValue::U32(a),
        },
        HkxMember {
            name: "particleB".to_string(),
            value: HkxValue::U32(b),
        },
        HkxMember {
            name: "stiffness".to_string(),
            value: HkxValue::F32(stiffness),
        },
    ])
}

fn make_bend_link(a: u32, b: u32, c: u32, d: u32, stiffness: f32) -> HkxValue {
    HkxValue::Object(vec![
        HkxMember {
            name: "particleA".to_string(),
            value: HkxValue::U32(a),
        },
        HkxMember {
            name: "particleB".to_string(),
            value: HkxValue::U32(b),
        },
        HkxMember {
            name: "particleC".to_string(),
            value: HkxValue::U32(c),
        },
        HkxMember {
            name: "particleD".to_string(),
            value: HkxValue::U32(d),
        },
        HkxMember {
            name: "bendStiffness".to_string(),
            value: HkxValue::F32(stiffness),
        },
    ])
}

/// Build a synthetic HkxFile with:
/// Objects (indices):
///   0: hclStandardLinkConstraintSet  (2 links)
///   1: hclBendStiffnessConstraintSet (1 link, 4-particle)
///   2: hclSimulateOperator
///   3: hclCapsuleShape
///   4: hclCollidable  → shape ptr→3
///   5: hclSimClothPose ("DefaultClothPose", 3 positions)
///   6: hclSimClothData → constraint[0,1], collidable[4], pose[5]
///   7: hclClothData   → simClothDatas[6], operators[2]
fn build_synthetic_file() -> HkxFile {
    // 0: hclStandardLinkConstraintSet
    let standard_cs = HkxObject {
        name: Some("#0000".to_string()),
        offset: 0,
        signature: 0,
        class_name: "hclStandardLinkConstraintSet".to_string(),
        members: vec![
            HkxMember {
                name: "name".to_string(),
                value: HkxValue::String {
                    value: "StandardLinks".to_string(),
                    is_null: false,
                },
            },
            HkxMember {
                name: "links".to_string(),
                value: HkxValue::Array(vec![
                    make_standard_link(0, 1, 0.9),
                    make_standard_link(1, 2, 0.9),
                ]),
            },
        ],
    };

    // 1: hclBendStiffnessConstraintSet
    let bend_cs = HkxObject {
        name: Some("#0001".to_string()),
        offset: 0,
        signature: 0,
        class_name: "hclBendStiffnessConstraintSet".to_string(),
        members: vec![
            HkxMember {
                name: "name".to_string(),
                value: HkxValue::String {
                    value: "BendLinks".to_string(),
                    is_null: false,
                },
            },
            HkxMember {
                name: "links".to_string(),
                value: HkxValue::Array(vec![make_bend_link(0, 1, 2, 0, 0.5)]),
            },
        ],
    };

    // 2: hclSimulateOperator
    let sim_op = HkxObject {
        name: Some("#0002".to_string()),
        offset: 0,
        signature: 0,
        class_name: "hclSimulateOperator".to_string(),
        members: vec![
            HkxMember {
                name: "subSteps".to_string(),
                value: HkxValue::U32(3),
            },
            HkxMember {
                name: "numberOfSolveIterations".to_string(),
                value: HkxValue::U32(2),
            },
        ],
    };

    // 3: hclCapsuleShape
    let capsule_shape = HkxObject {
        name: Some("#0003".to_string()),
        offset: 0,
        signature: 0,
        class_name: "hclCapsuleShape".to_string(),
        members: vec![
            HkxMember {
                name: "start".to_string(),
                value: HkxValue::F32List(vec![1.0, 2.0, 3.0, 0.0]),
            },
            HkxMember {
                name: "end".to_string(),
                value: HkxValue::F32List(vec![1.0, 2.0, 5.0, 0.0]),
            },
            HkxMember {
                name: "radius".to_string(),
                value: HkxValue::F32(2.5),
            },
        ],
    };

    // 4: hclCollidable → shape→3
    let collidable = HkxObject {
        name: Some("#0004".to_string()),
        offset: 0,
        signature: 0,
        class_name: "hclCollidable".to_string(),
        members: vec![
            HkxMember {
                name: "name".to_string(),
                value: HkxValue::String {
                    value: "Collidable_LLeg_Thigh".to_string(),
                    is_null: false,
                },
            },
            HkxMember {
                name: "shape".to_string(),
                value: HkxValue::Pointer(Some(3)),
            },
        ],
    };

    // 5: hclSimClothPose
    let pose = HkxObject {
        name: Some("#0005".to_string()),
        offset: 0,
        signature: 0,
        class_name: "hclSimClothPose".to_string(),
        members: vec![
            HkxMember {
                name: "name".to_string(),
                value: HkxValue::String {
                    value: "DefaultClothPose".to_string(),
                    is_null: false,
                },
            },
            HkxMember {
                name: "positions".to_string(),
                value: HkxValue::Array(vec![
                    HkxValue::F32List(vec![0.0, 0.0, 10.0, 1.0]),
                    HkxValue::F32List(vec![1.0, 0.0, 10.0, 1.0]),
                    HkxValue::F32List(vec![2.0, 0.0, 10.0, 1.0]),
                ]),
            },
        ],
    };

    // 6: hclSimClothData
    let sim_cloth = HkxObject {
        name: Some("#0006".to_string()),
        offset: 0,
        signature: 0,
        class_name: "hclSimClothData".to_string(),
        members: vec![
            HkxMember {
                name: "name".to_string(),
                value: HkxValue::String {
                    value: "BodyCloth".to_string(),
                    is_null: false,
                },
            },
            HkxMember {
                name: "particleDatas".to_string(),
                value: HkxValue::Array(vec![
                    make_particle(0.1, 10.0, 1.4, 0.2),
                    make_particle(0.1, 10.0, 1.4, 0.2),
                    make_particle(0.0, 0.0, 1.4, 0.0), // fixed particle
                ]),
            },
            HkxMember {
                name: "fixedParticles".to_string(),
                value: HkxValue::Array(vec![HkxValue::U32(2)]),
            },
            HkxMember {
                name: "staticConstraintSets".to_string(),
                value: HkxValue::Array(vec![
                    HkxValue::Pointer(Some(0)), // standard_cs
                    HkxValue::Pointer(Some(1)), // bend_cs
                ]),
            },
            HkxMember {
                name: "perInstanceCollidables".to_string(),
                value: HkxValue::Array(vec![
                    HkxValue::Pointer(Some(4)), // collidable
                ]),
            },
            HkxMember {
                name: "simClothPoses".to_string(),
                value: HkxValue::Array(vec![
                    HkxValue::Pointer(Some(5)), // pose
                ]),
            },
            HkxMember {
                name: "simulationInfo".to_string(),
                value: HkxValue::Object(vec![
                    HkxMember {
                        name: "gravity".to_string(),
                        value: HkxValue::F32List(vec![0.0, 0.0, -686.7, 1.0]),
                    },
                    HkxMember {
                        name: "globalDampingPerSecond".to_string(),
                        value: HkxValue::F32(0.9999),
                    },
                    HkxMember {
                        name: "collisionTolerance".to_string(),
                        value: HkxValue::F32(14.0),
                    },
                ]),
            },
            HkxMember {
                name: "collidableTransformMap".to_string(),
                value: HkxValue::Object(vec![HkxMember {
                    name: "transformIndices".to_string(),
                    value: HkxValue::Array(vec![]),
                }]),
            },
        ],
    };

    // 7: hclClothData (root)
    let cloth_data = HkxObject {
        name: Some("#0007".to_string()),
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
            HkxMember {
                name: "simClothDatas".to_string(),
                value: HkxValue::Array(vec![HkxValue::Pointer(Some(6))]),
            },
            HkxMember {
                name: "operators".to_string(),
                value: HkxValue::Array(vec![HkxValue::Pointer(Some(2))]),
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

    HkxFile::from_tagxml(
        11,
        "hk_2014.1.0-r1",
        vec![
            standard_cs,
            bend_cs,
            sim_op,
            capsule_shape,
            collidable,
            pose,
            sim_cloth,
            cloth_data,
        ],
    )
}

fn blob_from_synthetic() -> Vec<u8> {
    let file = build_synthetic_file();
    let mut registry = DescriptorRegistry::default();
    write_hkx(&file, &mut registry)
}

// ---------------------------------------------------------------------------
// cloth_inspect_full_json returns a walkable JSON tree
// ---------------------------------------------------------------------------

#[test]
fn cloth_inspect_full_json_returns_walkable_tree() {
    let blob = blob_from_synthetic();

    let json_str =
        api::cloth_inspect_full_json(&blob).expect("cloth_inspect_full_json must succeed");

    let root: serde_json::Value =
        serde_json::from_str(&json_str).expect("output must be valid JSON");

    assert_eq!(root["name"].as_str().unwrap_or(""), "TestCloth");

    let sim_cloths = root["sim_cloths"]
        .as_array()
        .expect("sim_cloths must be an array");
    assert_eq!(sim_cloths.len(), 1, "expected 1 sim cloth");

    let sc = &sim_cloths[0];
    assert_eq!(sc["name"].as_str().unwrap_or(""), "BodyCloth");

    let particles = sc["particles"]
        .as_array()
        .expect("sim_cloths[0].particles must be array");
    assert_eq!(particles.len(), 3, "expected 3 particles");

    // positions come from the default pose
    let p0 = &particles[0];
    let pos = p0["position"]
        .as_array()
        .expect("particle[0].position must be array");
    assert!((pos[0].as_f64().unwrap_or(-1.0) - 0.0).abs() < 1e-4, "p0.x");
    assert!(
        (pos[2].as_f64().unwrap_or(-1.0) - 10.0).abs() < 1e-4,
        "p0.z"
    );

    // mass/inv_mass/radius/friction on particle struct
    assert!((p0["mass"].as_f64().unwrap_or(-1.0) - 0.1).abs() < 1e-4);
    assert!((p0["radius"].as_f64().unwrap_or(-1.0) - 1.4).abs() < 1e-4);

    let fixed = sc["fixed_particle_indices"]
        .as_array()
        .expect("fixed_particle_indices must be array");
    assert_eq!(fixed.len(), 1);
    assert_eq!(fixed[0].as_u64().unwrap(), 2);

    let sim_info = &sc["simulation_info"];
    let gravity = sim_info["gravity"]
        .as_array()
        .expect("gravity must be array");
    assert!((gravity[2].as_f64().unwrap_or(0.0) - (-686.7)).abs() < 0.1);
    assert!((sim_info["globalDampingPerSecond"].as_f64().unwrap_or(0.0) - 0.9999).abs() < 1e-4);
    assert!((sim_info["collisionTolerance"].as_f64().unwrap_or(0.0) - 14.0).abs() < 1e-4);

    // constraint_sets: 2 entries (standard + bend)
    let constraint_sets = sc["constraint_sets"]
        .as_array()
        .expect("constraint_sets must be array");
    assert_eq!(constraint_sets.len(), 2, "expected 2 constraint sets");

    let standard = constraint_sets
        .iter()
        .find(|cs| cs["class_name"].as_str() == Some("hclStandardLinkConstraintSet"))
        .expect("must have StandardLink constraint set");
    assert_eq!(standard["link_count"].as_u64().unwrap(), 2);

    let standard_links = standard["links"]
        .as_array()
        .expect("constraint_set.links must be array");
    assert_eq!(standard_links.len(), 2);
    assert_eq!(standard_links[0]["particleA"].as_u64().unwrap(), 0);
    assert_eq!(standard_links[0]["particleB"].as_u64().unwrap(), 1);
    assert!((standard_links[0]["stiffness"].as_f64().unwrap_or(0.0) - 0.9).abs() < 1e-4);

    let bend = constraint_sets
        .iter()
        .find(|cs| cs["class_name"].as_str() == Some("hclBendStiffnessConstraintSet"))
        .expect("must have Bend constraint set");
    let bend_links = bend["links"].as_array().expect("bend links must be array");
    assert_eq!(bend_links.len(), 1);
    assert_eq!(bend_links[0]["particleC"].as_u64().unwrap(), 2);
    assert!((bend_links[0]["bendStiffness"].as_f64().unwrap_or(0.0) - 0.5).abs() < 1e-4);

    // collidables: 1 capsule
    let collidables = sc["collidables"]
        .as_array()
        .expect("collidables must be array");
    assert_eq!(collidables.len(), 1, "expected 1 collidable");
    let cap = &collidables[0];
    assert_eq!(cap["name"].as_str().unwrap_or(""), "Collidable_LLeg_Thigh");
    assert_eq!(cap["shape_class"].as_str().unwrap_or(""), "hclCapsuleShape");
    let start = cap["start"]
        .as_array()
        .expect("capsule.start must be array");
    assert!(
        (start[2].as_f64().unwrap_or(0.0) - 3.0).abs() < 1e-4,
        "start.z"
    );
    assert!((cap["radius"].as_f64().unwrap_or(0.0) - 2.5).abs() < 1e-4);

    // poses: 1 entry
    let poses = sc["poses"].as_array().expect("poses must be array");
    assert_eq!(poses.len(), 1);
    assert_eq!(poses[0]["name"].as_str().unwrap_or(""), "DefaultClothPose");
    let pose_positions = poses[0]["positions"]
        .as_array()
        .expect("pose.positions must be array");
    assert_eq!(pose_positions.len(), 3);

    // operators: synthetic fixture contains a single hclSimulateOperator at index 2.
    let operators = root["operators"]
        .as_array()
        .expect("operators must be array");
    assert_eq!(operators.len(), 1, "expected 1 operator in fixture");
    assert_eq!(
        operators[0]["class_name"].as_str().unwrap_or(""),
        "hclSimulateOperator",
    );

    // cloth_states: fixture has clothStateDatas: [], so the array is present but empty.
    let cloth_states = root["cloth_states"]
        .as_array()
        .expect("cloth_states must be array");
    assert_eq!(cloth_states.len(), 0, "fixture has no clothStateDatas");
}

// ---------------------------------------------------------------------------
// cloth_inspect_full_json exposes positions for cloth_skin_bind path
// ---------------------------------------------------------------------------

#[test]
fn cloth_inspect_full_json_positions_usable_for_skin_bind() {
    let blob = blob_from_synthetic();

    let json_str =
        api::cloth_inspect_full_json(&blob).expect("cloth_inspect_full_json must succeed");
    let root: serde_json::Value = serde_json::from_str(&json_str).unwrap();

    // The cloth_skin_bind path only needs positions[i] from sim_cloths[0].particles
    let particles = root["sim_cloths"][0]["particles"].as_array().unwrap();
    let positions: Vec<(f64, f64, f64)> = particles
        .iter()
        .map(|p| {
            let pos = p["position"].as_array().unwrap();
            (
                pos[0].as_f64().unwrap(),
                pos[1].as_f64().unwrap(),
                pos[2].as_f64().unwrap(),
            )
        })
        .collect();

    assert_eq!(positions.len(), 3);
    // Positions match the hclSimClothPose "DefaultClothPose" positions
    assert!((positions[0].0 - 0.0).abs() < 1e-4);
    assert!((positions[1].0 - 1.0).abs() < 1e-4);
    assert!((positions[2].0 - 2.0).abs() < 1e-4);
}

// ---------------------------------------------------------------------------
// bathrobe fixture (skipped if absent)
// ---------------------------------------------------------------------------

fn bathrobe_blob() -> Option<Vec<u8>> {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("../tests/fixtures/cloth/bathrobe_outfitm_cloth.hkx");
    if !path.exists() {
        return None;
    }
    Some(std::fs::read(&path).expect("bathrobe read"))
}

#[test]
fn cloth_inspect_full_json_bathrobe_has_particles_and_collidables() {
    let blob = match bathrobe_blob() {
        Some(b) => b,
        None => return, // skip if fixture absent
    };

    let json_str = api::cloth_inspect_full_json(&blob)
        .expect("cloth_inspect_full_json must succeed on bathrobe");

    let root: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    let sim_cloths = root["sim_cloths"].as_array().unwrap();
    assert!(
        !sim_cloths.is_empty(),
        "bathrobe must have at least one sim cloth"
    );

    let sc = &sim_cloths[0];
    let particles = sc["particles"].as_array().unwrap();
    assert!(
        !particles.is_empty(),
        "bathrobe sim cloth must have particles"
    );

    // Every particle must have a position array with 3+ elements
    for p in particles {
        let pos = p["position"]
            .as_array()
            .expect("particle must have position");
        assert!(pos.len() >= 3, "position must have at least 3 components");
    }
}
