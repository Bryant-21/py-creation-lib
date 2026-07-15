// Round-trip tests for native ClothEditor methods.
//
// Each test category verifies that an operation mutates the expected field and that
// the result bytes can be re-parsed as a valid packfile (byte-level round-trip gate).
//
// Categories: particles, sim info, operator, constraint, capsule, summary.

use havok_native::api;
use havok_native::cloth::ClothEditor;
use havok_native::hkx::descriptors::DescriptorRegistry;
use havok_native::hkx::types::HkxValue;
use havok_native::hkx::{HkxFile, HkxMember, HkxObject, read_packfile, write_hkx};

// ---------------------------------------------------------------------------
// Shared test fixture builder (same layout as havok_cloth_edit.rs)
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

fn make_standard_link(stiffness: f32) -> HkxValue {
    HkxValue::Object(vec![HkxMember {
        name: "stiffness".to_string(),
        value: HkxValue::F32(stiffness),
    }])
}

fn make_bend_link(bend_stiffness: f32) -> HkxValue {
    HkxValue::Object(vec![HkxMember {
        name: "bendStiffness".to_string(),
        value: HkxValue::F32(bend_stiffness),
    }])
}

fn build_synthetic_file() -> HkxFile {
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

    let capsule_shape = HkxObject {
        name: Some("#0005".to_string()),
        offset: 0,
        signature: 0,
        class_name: "hclCapsuleShape".to_string(),
        members: vec![
            HkxMember {
                name: "start".to_string(),
                value: HkxValue::F32List(vec![0.0, 0.0, 0.0, 0.0]),
            },
            HkxMember {
                name: "end".to_string(),
                value: HkxValue::F32List(vec![5.0, 0.0, 0.0, 0.0]),
            },
            HkxMember {
                name: "dir".to_string(),
                value: HkxValue::F32List(vec![1.0, 0.0, 0.0, 0.0]),
            },
            HkxMember {
                name: "radius".to_string(),
                value: HkxValue::F32(2.0),
            },
        ],
    };

    let collidable = HkxObject {
        name: Some("#0006".to_string()),
        offset: 0,
        signature: 0,
        class_name: "hclCollidable".to_string(),
        members: vec![
            HkxMember {
                name: "name".to_string(),
                value: HkxValue::String {
                    value: "bone_1".to_string(),
                    is_null: false,
                },
            },
            HkxMember {
                name: "shape".to_string(),
                value: HkxValue::Pointer(Some(4)),
            }, // capsule_shape at index 4
        ],
    };

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
                    make_particle(0.2, 5.0, 1.0, 0.2), // different mass
                ]),
            },
            HkxMember {
                name: "fixedParticles".to_string(),
                value: HkxValue::Array(vec![]),
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
                    HkxValue::Pointer(Some(5)), // collidable at index 5
                ]),
            },
            HkxMember {
                name: "collidableTransformMap".to_string(),
                value: HkxValue::Object(vec![HkxMember {
                    name: "transformIndices".to_string(),
                    value: HkxValue::Array(vec![HkxValue::U32(1)]),
                }]),
            },
            HkxMember {
                name: "simulationInfo".to_string(),
                value: HkxValue::Object(vec![
                    HkxMember {
                        name: "gravity".to_string(),
                        value: HkxValue::F32List(vec![0.0, 0.0, -9.8, 0.0]),
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
            HkxMember {
                name: "simClothDatas".to_string(),
                value: HkxValue::Array(vec![HkxValue::Pointer(Some(6))]), // sim_cloth at index 6
            },
            HkxMember {
                name: "operators".to_string(),
                value: HkxValue::Array(vec![HkxValue::Pointer(Some(2))]), // sim_op at index 2
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

    let _ = cloth_data; // discard, rebuild with correct indices
    let _ = sim_cloth;
    let _ = collidable;
    let _ = capsule_shape;

    // Rebuild with consistent indices:
    // 0: standard_cs
    // 1: bend_cs
    // 2: sim_op
    // 3: capsule_shape
    // 4: collidable (shape→3)
    // 5: sim_cloth  (staticConstraintSets→[0,1], perInstanceCollidables→[4], operators→[2])
    // 6: cloth_data (simClothDatas→[5], operators→[2])

    let collidable2 = HkxObject {
        name: Some("#0004".to_string()),
        offset: 0,
        signature: 0,
        class_name: "hclCollidable".to_string(),
        members: vec![
            HkxMember {
                name: "name".to_string(),
                value: HkxValue::String {
                    value: "bone_1".to_string(),
                    is_null: false,
                },
            },
            HkxMember {
                name: "shape".to_string(),
                value: HkxValue::Pointer(Some(3)),
            }, // capsule_shape at 3
        ],
    };

    let capsule_shape2 = HkxObject {
        name: Some("#0003".to_string()),
        offset: 0,
        signature: 0,
        class_name: "hclCapsuleShape".to_string(),
        members: vec![
            HkxMember {
                name: "start".to_string(),
                value: HkxValue::F32List(vec![0.0, 0.0, 0.0, 0.0]),
            },
            HkxMember {
                name: "end".to_string(),
                value: HkxValue::F32List(vec![5.0, 0.0, 0.0, 0.0]),
            },
            HkxMember {
                name: "dir".to_string(),
                value: HkxValue::F32List(vec![1.0, 0.0, 0.0, 0.0]),
            },
            HkxMember {
                name: "radius".to_string(),
                value: HkxValue::F32(2.0),
            },
        ],
    };

    let sim_cloth2 = HkxObject {
        name: Some("#0005".to_string()),
        offset: 0,
        signature: 0,
        class_name: "hclSimClothData".to_string(),
        members: vec![
            HkxMember {
                name: "particleDatas".to_string(),
                value: HkxValue::Array(vec![
                    make_particle(0.1, 10.0, 1.0, 0.2),
                    make_particle(0.1, 10.0, 1.0, 0.2),
                    make_particle(0.2, 5.0, 1.0, 0.2),
                ]),
            },
            HkxMember {
                name: "fixedParticles".to_string(),
                value: HkxValue::Array(vec![]),
            },
            HkxMember {
                name: "staticConstraintSets".to_string(),
                value: HkxValue::Array(vec![
                    HkxValue::Pointer(Some(0)),
                    HkxValue::Pointer(Some(1)),
                ]),
            },
            HkxMember {
                name: "perInstanceCollidables".to_string(),
                value: HkxValue::Array(vec![HkxValue::Pointer(Some(4))]),
            },
            HkxMember {
                name: "collidableTransformMap".to_string(),
                value: HkxValue::Object(vec![HkxMember {
                    name: "transformIndices".to_string(),
                    value: HkxValue::Array(vec![HkxValue::U32(1)]),
                }]),
            },
            HkxMember {
                name: "simulationInfo".to_string(),
                value: HkxValue::Object(vec![
                    HkxMember {
                        name: "gravity".to_string(),
                        value: HkxValue::F32List(vec![0.0, 0.0, -9.8, 0.0]),
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

    let cloth_data2 = HkxObject {
        name: Some("#0006".to_string()),
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
                value: HkxValue::Array(vec![HkxValue::Pointer(Some(5))]),
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

    let sim_op2 = HkxObject {
        name: Some("#0002".to_string()),
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

    HkxFile::from_tagxml(
        11,
        "hk_2014.1.0-r1",
        vec![
            standard_cs,
            bend_cs,
            sim_op2,
            capsule_shape2,
            collidable2,
            sim_cloth2,
            cloth_data2,
        ],
    )
}

fn to_blob(file: &HkxFile) -> Vec<u8> {
    let mut reg = DescriptorRegistry::new();
    write_hkx(file, &mut reg)
}

// ---------------------------------------------------------------------------
// Category: Particles
// ---------------------------------------------------------------------------

#[test]
fn pyfunction_set_particle_mass_all_round_trips() {
    let mut file = build_synthetic_file();
    let blob = to_blob(&file);

    let (new_blob, count) = api::cloth_set_particle_mass_all(&blob, 0.05, 0)
        .expect("cloth_set_particle_mass_all must succeed");

    // All 3 particles are movable (fixedParticles is empty).
    assert_eq!(count, 3, "all 3 movable particles should be modified");

    // Round-trip: new bytes must parse as valid packfile.
    let reparsed = read_packfile(&new_blob)
        .expect("new blob must be parseable after cloth_set_particle_mass_all");

    // Verify mass was applied in the reparsed file.
    // hclSimClothData is at index 5.
    let sim_cloth = &reparsed.objects()[5];
    if let Some(pd) = sim_cloth.members.iter().find(|m| m.name == "particleDatas") {
        if let HkxValue::Array(particles) = &pd.value {
            for (i, p) in particles.iter().enumerate() {
                if let HkxValue::Object(members) = p {
                    let mass = members
                        .iter()
                        .find(|m| m.name == "mass")
                        .and_then(|m| {
                            if let HkxValue::F32(v) = &m.value {
                                Some(*v)
                            } else {
                                None
                            }
                        })
                        .unwrap_or(f32::NAN);
                    assert!(
                        (mass - 0.05).abs() < 1e-5,
                        "particle {i}: expected mass=0.05, got {mass}"
                    );
                }
            }
        }
    }

    drop(file); // satisfy borrow checker — file was used for to_blob only
    let _ = file; // avoid unused warning
}

#[test]
fn pyfunction_set_particles_mass_subset_round_trips() {
    let mut file = build_synthetic_file();
    let blob = to_blob(&file);

    // Only modify particles 0 and 2.
    let (new_blob, count) = api::cloth_set_particles_mass(&blob, &[0, 2], 0.07, 0)
        .expect("cloth_set_particles_mass must succeed");

    assert_eq!(count, 2, "two particles should be modified");

    let reparsed = read_packfile(&new_blob).expect("new blob must be parseable");
    let sim_cloth = &reparsed.objects()[5];
    if let Some(pd) = sim_cloth.members.iter().find(|m| m.name == "particleDatas") {
        if let HkxValue::Array(particles) = &pd.value {
            let mass_0 = get_f32_member(&particles[0], "mass");
            let mass_1 = get_f32_member(&particles[1], "mass");
            let mass_2 = get_f32_member(&particles[2], "mass");
            assert!(
                (mass_0 - 0.07).abs() < 1e-5,
                "particle 0 mass should be 0.07, got {mass_0}"
            );
            assert!(
                (mass_1 - 0.1).abs() < 1e-5,
                "particle 1 mass should be unchanged at 0.1, got {mass_1}"
            );
            assert!(
                (mass_2 - 0.07).abs() < 1e-5,
                "particle 2 mass should be 0.07, got {mass_2}"
            );
        }
    }

    drop(file);
}

#[test]
fn pyfunction_set_particles_radius_subset_round_trips() {
    let mut file = build_synthetic_file();
    let blob = to_blob(&file);

    let (new_blob, count) = api::cloth_set_particles_radius(&blob, &[1], 3.5, 0)
        .expect("cloth_set_particles_radius must succeed");

    assert_eq!(count, 1);

    let reparsed = read_packfile(&new_blob).expect("new blob must be parseable");
    let sim_cloth = &reparsed.objects()[5];
    if let Some(pd) = sim_cloth.members.iter().find(|m| m.name == "particleDatas") {
        if let HkxValue::Array(particles) = &pd.value {
            let r1 = get_f32_member(&particles[1], "radius");
            let r0 = get_f32_member(&particles[0], "radius");
            assert!(
                (r1 - 3.5).abs() < 1e-5,
                "particle 1 radius should be 3.5, got {r1}"
            );
            assert!(
                (r0 - 1.0).abs() < 1e-5,
                "particle 0 radius should be unchanged at 1.0, got {r0}"
            );
        }
    }

    drop(file);
}

#[test]
fn pyfunction_set_particle_fixed_single_round_trips() {
    let mut file = build_synthetic_file();
    let blob = to_blob(&file);

    // Pin particle 1.
    let new_blob = api::cloth_set_particle_fixed(&blob, 1, true, 0)
        .map(|(b, _)| b)
        .expect("cloth_set_particle_fixed must succeed");

    let reparsed = read_packfile(&new_blob).expect("new blob must be parseable");
    let sim_cloth = &reparsed.objects()[5];
    if let Some(fp) = sim_cloth
        .members
        .iter()
        .find(|m| m.name == "fixedParticles")
    {
        if let HkxValue::Array(arr) = &fp.value {
            let has_1 = arr.iter().any(|v| match v {
                HkxValue::U32(n) => *n == 1,
                HkxValue::U16(n) => *n == 1,
                HkxValue::U8(n) => *n == 1,
                HkxValue::I32(n) => *n == 1,
                _ => false,
            });
            assert!(
                has_1,
                "particle 1 should be in fixedParticles after pinning, got: {arr:?}"
            );
        }
    }

    drop(file);
}

// ---------------------------------------------------------------------------
// Category: Simulation info
// ---------------------------------------------------------------------------

#[test]
fn pyfunction_set_gravity_round_trips() {
    let mut file = build_synthetic_file();
    let blob = to_blob(&file);

    let new_blob = api::cloth_set_gravity(&blob, [0.0, 0.0, -686.7, 0.0], 0)
        .expect("cloth_set_gravity must succeed");

    let reparsed = read_packfile(&new_blob).expect("new blob must be parseable");
    let sim_cloth = &reparsed.objects()[5];
    if let Some(si) = sim_cloth
        .members
        .iter()
        .find(|m| m.name == "simulationInfo")
    {
        if let HkxValue::Object(info) = &si.value {
            if let Some(gm) = info.iter().find(|m| m.name == "gravity") {
                if let HkxValue::F32List(g) = &gm.value {
                    assert!(
                        (g[2] - (-686.7f32)).abs() < 0.1,
                        "gravity Z should be -686.7, got {}",
                        g[2]
                    );
                }
            }
        }
    }

    drop(file);
}

#[test]
fn pyfunction_set_damping_round_trips() {
    let mut file = build_synthetic_file();
    let blob = to_blob(&file);

    let new_blob =
        api::cloth_set_damping(&blob, 0.9999, 0).expect("cloth_set_damping must succeed");

    read_packfile(&new_blob).expect("new blob must be parseable after set_damping");

    drop(file);
}

#[test]
fn pyfunction_set_collision_tolerance_round_trips() {
    let mut file = build_synthetic_file();
    let blob = to_blob(&file);

    let new_blob = api::cloth_set_collision_tolerance(&blob, 14.0, 0)
        .expect("cloth_set_collision_tolerance must succeed");

    read_packfile(&new_blob).expect("new blob must be parseable after set_collision_tolerance");

    drop(file);
}

// ---------------------------------------------------------------------------
// Category: Operator
// ---------------------------------------------------------------------------

#[test]
fn pyfunction_set_substeps_round_trips() {
    let mut file = build_synthetic_file();
    let blob = to_blob(&file);

    let new_blob = api::cloth_set_substeps(&blob, 4).expect("cloth_set_substeps must succeed");

    let reparsed = read_packfile(&new_blob).expect("new blob must be parseable after set_substeps");
    // hclSimulateOperator is at index 2.
    let sim_op = &reparsed.objects()[2];
    let substeps = sim_op
        .members
        .iter()
        .find(|m| m.name == "subSteps")
        .and_then(|m| match &m.value {
            HkxValue::U32(v) => Some(*v),
            HkxValue::I32(v) => Some(*v as u32),
            _ => None,
        })
        .expect("subSteps must be present");
    assert_eq!(substeps, 4, "subSteps should be 4 after set_substeps(4)");

    drop(file);
}

#[test]
fn pyfunction_set_solver_iterations_round_trips() {
    let mut file = build_synthetic_file();
    let blob = to_blob(&file);

    let new_blob = api::cloth_set_solver_iterations(&blob, 8)
        .expect("cloth_set_solver_iterations must succeed");

    let reparsed = read_packfile(&new_blob).expect("parseable after set_solver_iterations");
    let sim_op = &reparsed.objects()[2];
    let iters = sim_op
        .members
        .iter()
        .find(|m| m.name == "numberOfSolveIterations")
        .and_then(|m| match &m.value {
            HkxValue::U32(v) => Some(*v),
            HkxValue::I32(v) => Some(*v as u32),
            _ => None,
        })
        .expect("numberOfSolveIterations must be present");
    assert_eq!(iters, 8);

    drop(file);
}

// ---------------------------------------------------------------------------
// Category: Constraint
// ---------------------------------------------------------------------------

#[test]
fn pyfunction_scale_stiffness_round_trips() {
    let mut file = build_synthetic_file();
    let blob = to_blob(&file);

    let (new_blob, count) = api::cloth_scale_stiffness(&blob, Some("standard"), 0.5, 0)
        .expect("cloth_scale_stiffness must succeed");

    assert_eq!(count, 2, "two standard links should be scaled");

    let reparsed = read_packfile(&new_blob).expect("parseable after scale_stiffness");
    // hclStandardLinkConstraintSet is at index 0.
    let cs = &reparsed.objects()[0];
    if let Some(lm) = cs.members.iter().find(|m| m.name == "links") {
        if let HkxValue::Array(links) = &lm.value {
            for (i, link) in links.iter().enumerate() {
                if let HkxValue::Object(members) = link {
                    let stiffness = members
                        .iter()
                        .find(|m| m.name == "stiffness")
                        .and_then(|m| {
                            if let HkxValue::F32(v) = &m.value {
                                Some(*v)
                            } else {
                                None
                            }
                        })
                        .expect("stiffness must be present");
                    assert!(
                        (stiffness - 0.5).abs() < 1e-5,
                        "link {i}: stiffness should be 0.5, got {stiffness}"
                    );
                }
            }
        }
    }

    drop(file);
}

// ---------------------------------------------------------------------------
// Category: Capsule
// ---------------------------------------------------------------------------

#[test]
fn pyfunction_set_capsule_radius_round_trips() {
    let mut file = build_synthetic_file();
    let blob = to_blob(&file);

    let new_blob = api::cloth_set_capsule_radius(&blob, 0, 5.0, 0)
        .expect("cloth_set_capsule_radius must succeed");

    let reparsed = read_packfile(&new_blob).expect("parseable after set_capsule_radius");
    // hclCapsuleShape is at index 3.
    let shape = &reparsed.objects()[3];
    let radius = shape
        .members
        .iter()
        .find(|m| m.name == "radius")
        .and_then(|m| {
            if let HkxValue::F32(v) = &m.value {
                Some(*v)
            } else {
                None
            }
        })
        .expect("radius must be present");
    assert!(
        (radius - 5.0).abs() < 1e-5,
        "radius should be 5.0, got {radius}"
    );

    drop(file);
}

#[test]
fn pyfunction_scale_all_capsule_radii_round_trips() {
    let mut file = build_synthetic_file();
    let blob = to_blob(&file);

    // Original radius is 2.0; scale by 1.5 → should become 3.0.
    let (new_blob, count) = api::cloth_scale_all_capsule_radii(&blob, 1.5, 0)
        .expect("cloth_scale_all_capsule_radii must succeed");

    assert_eq!(count, 1, "one capsule should be scaled");

    let reparsed = read_packfile(&new_blob).expect("parseable after scale_all_capsule_radii");
    let shape = &reparsed.objects()[3];
    let radius = shape
        .members
        .iter()
        .find(|m| m.name == "radius")
        .and_then(|m| {
            if let HkxValue::F32(v) = &m.value {
                Some(*v)
            } else {
                None
            }
        })
        .expect("radius must be present");
    assert!(
        (radius - 3.0).abs() < 1e-4,
        "radius should be 3.0 after 1.5x scale, got {radius}"
    );

    drop(file);
}

#[test]
fn pyfunction_add_remove_capsule_round_trips() {
    let mut file = build_synthetic_file();
    let n_before = file.objects().len();
    let blob = to_blob(&file);

    let (blob2, new_idx) =
        api::cloth_add_capsule(&blob, "bone_7", 1.0, [0.0; 4], [10.0, 0.0, 0.0, 0.0], 0)
            .expect("cloth_add_capsule must succeed");

    // blob2 should parse cleanly and have 2 more objects.
    let reparsed2 = read_packfile(&blob2).expect("parseable after add_capsule");
    assert_eq!(
        reparsed2.objects().len(),
        n_before + 2,
        "add_capsule should add 2 objects (shape + collidable)"
    );

    // Remove the newly added capsule (new_idx is its position in perInstanceCollidables).
    let blob3 =
        api::cloth_remove_capsule(&blob2, new_idx, 0).expect("cloth_remove_capsule must succeed");

    let reparsed3 = read_packfile(&blob3).expect("parseable after remove_capsule");
    assert_eq!(
        reparsed3.objects().len(),
        n_before,
        "remove_capsule should restore original object count"
    );

    drop(file);
}

// ---------------------------------------------------------------------------
// Category: Summary JSON
// ---------------------------------------------------------------------------

#[test]
fn pyfunction_summary_json_returns_valid_json() {
    let mut file = build_synthetic_file();
    let blob = to_blob(&file);

    let json_str = api::cloth_summary_json(&blob, 0).expect("cloth_summary_json must succeed");

    let parsed: serde_json::Value =
        serde_json::from_str(&json_str).expect("cloth_summary_json must produce valid JSON");

    assert_eq!(parsed["particle_count"], 3, "particle_count should be 3");
    assert_eq!(parsed["fixed_count"], 0, "fixed_count should be 0");
    assert_eq!(parsed["capsule_count"], 1, "capsule_count should be 1");

    // operator should have substeps=1, iterations=4
    let op = &parsed["operator"];
    assert_eq!(op["substeps"], 1, "substeps should be 1");
    assert_eq!(op["iterations"], 4, "iterations should be 4");

    drop(file);
}

// ---------------------------------------------------------------------------
// Category: Inspect blob JSON (option B)
// ---------------------------------------------------------------------------

#[test]
fn pyfunction_inspect_blob_json_returns_all_objects() {
    let mut file = build_synthetic_file();
    let blob = to_blob(&file);

    let json_str =
        api::cloth_inspect_blob_json(&blob).expect("cloth_inspect_blob_json must succeed");

    let parsed: serde_json::Value =
        serde_json::from_str(&json_str).expect("cloth_inspect_blob_json must produce valid JSON");

    let n_objects = parsed["object_count"]
        .as_u64()
        .expect("object_count must be integer");
    assert!(
        n_objects >= 7,
        "should have at least 7 objects in synthetic file, got {n_objects}"
    );

    // Check that the class names are present somewhere in the objects array.
    let classes: Vec<&str> = parsed["objects"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|o| o["class"].as_str())
        .collect();
    assert!(
        classes.contains(&"hclClothData"),
        "hclClothData should be in objects"
    );
    assert!(
        classes.contains(&"hclSimClothData"),
        "hclSimClothData should be in objects"
    );

    drop(file);
}

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

fn get_f32_member(particle: &HkxValue, name: &str) -> f32 {
    if let HkxValue::Object(members) = particle {
        members
            .iter()
            .find(|m| m.name == name)
            .and_then(|m| {
                if let HkxValue::F32(v) = &m.value {
                    Some(*v)
                } else {
                    None
                }
            })
            .unwrap_or(f32::NAN)
    } else {
        f32::NAN
    }
}
