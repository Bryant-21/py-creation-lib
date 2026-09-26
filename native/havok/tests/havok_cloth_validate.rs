use havok_native::cloth::{ClothData, validate_cloth_data};
use havok_native::hkx::types::HkxValue;
use havok_native::hkx::{HkxFile, HkxMember, HkxObject};

fn string_val(s: &str) -> HkxValue {
    HkxValue::String {
        value: s.to_string(),
        is_null: false,
    }
}

fn ptr(index: usize) -> HkxValue {
    HkxValue::Pointer(Some(index))
}

fn member(name: &str, value: HkxValue) -> HkxMember {
    HkxMember {
        name: name.to_string(),
        value,
    }
}

fn make_object(class_name: &str, members: Vec<HkxMember>) -> HkxObject {
    HkxObject {
        name: None,
        offset: 0,
        signature: 0,
        class_name: class_name.to_string(),
        members,
    }
}

/// hclClothData (index 0) → optional hclSimClothData (index 1) with
/// `n_particles` movable particles and the given fixed indices.
fn cloth_objects(sim_cloth: Option<(usize, Vec<u32>)>) -> Vec<HkxObject> {
    let mut objects = vec![make_object(
        "hclClothData",
        vec![
            member("name", string_val("TestCloth")),
            member(
                "simClothDatas",
                HkxValue::Array(sim_cloth.iter().map(|_| ptr(1)).collect()),
            ),
            member("clothStateDatas", HkxValue::Array(vec![])),
            member("operators", HkxValue::Array(vec![])),
            member("bufferDefinitions", HkxValue::Array(vec![])),
            member("transformSetDefinitions", HkxValue::Array(vec![])),
        ],
    )];
    if let Some((n_particles, fixed)) = sim_cloth {
        let particles = (0..n_particles)
            .map(|_| HkxValue::Object(vec![member("invMass", HkxValue::F32(1.0))]))
            .collect();
        objects.push(make_object(
            "hclSimClothData",
            vec![
                member("name", string_val("TestSimCloth")),
                member("particleDatas", HkxValue::Array(particles)),
                member(
                    "fixedParticles",
                    HkxValue::Array(fixed.into_iter().map(HkxValue::U32).collect()),
                ),
                member("staticConstraintSets", HkxValue::Array(vec![])),
                member("perInstanceCollidables", HkxValue::Array(vec![])),
                member("simClothPoses", HkxValue::Array(vec![])),
            ],
        ));
    }
    objects
}

fn issue_codes(objects: Vec<HkxObject>) -> (Vec<String>, Vec<String>) {
    let file = HkxFile::from_tagxml(11, "hk_2014.1.0-r1", objects);
    let result = validate_cloth_data(ClothData::from_hkx_file(&file).as_ref());
    (
        result.errors().iter().map(|i| i.code.clone()).collect(),
        result.warnings().iter().map(|i| i.code.clone()).collect(),
    )
}

#[test]
fn structural_and_particle_lints() {
    let missing = validate_cloth_data(None);
    assert!(!missing.is_valid());
    let errors = missing.errors();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, "NO_CLOTH_DATA");
    assert!(errors[0].to_string().starts_with("[error] NO_CLOTH_DATA: "));
    let summary = missing.to_summary();
    assert_eq!(summary["valid"], serde_json::Value::Bool(false));
    let first = &summary["issues"][0];
    for key in ["severity", "code", "message"] {
        assert!(first.get(key).is_some(), "summary issue missing {key}");
    }

    let (errors, _) = issue_codes(cloth_objects(None));
    assert!(errors.contains(&"NO_SIM_CLOTH".to_string()), "{errors:?}");
    let (_, warnings) = issue_codes(cloth_objects(Some((3, vec![0, 1, 2]))));
    assert!(warnings.contains(&"ALL_PARTICLES_FIXED".to_string()), "{warnings:?}");
    let (errors, _) = issue_codes(cloth_objects(Some((3, vec![5]))));
    assert!(errors.contains(&"INVALID_FIXED_INDEX".to_string()), "{errors:?}");
}

#[test]
fn broken_operator_and_buffer_objects_emit_errors() {
    let element = || {
        HkxValue::Object(
            ["vectorConversion", "vectorSize", "slotId", "slotStart"]
                .iter()
                .map(|n| member(n, HkxValue::U8(0)))
                .collect(),
        )
    };
    let slot = || {
        HkxValue::Object(vec![
            member("flags", HkxValue::U8(0)),
            member("stride", HkxValue::U8(0)),
        ])
    };
    let zeroes = HkxValue::Array((0..64).map(|_| HkxValue::I16(0)).collect());
    let cases = [
        (
            "operators",
            make_object(
                "hclSimulateOperator",
                vec![
                    member("subSteps", HkxValue::U32(0)),
                    member("numberOfSolveIterations", HkxValue::I32(0)),
                ],
            ),
            vec!["ZERO_SUBSTEPS", "ZERO_SOLVE_ITERATIONS"],
        ),
        (
            "bufferDefinitions",
            make_object(
                "hclBufferDefinition",
                vec![
                    member("name", string_val("BrokenBuffer")),
                    member(
                        "bufferLayout",
                        HkxValue::Object(vec![
                            member(
                                "elementsLayout",
                                HkxValue::Array((0..4).map(|_| element()).collect()),
                            ),
                            member("slots", HkxValue::Array((0..4).map(|_| slot()).collect())),
                            member("numSlots", HkxValue::U8(2)),
                        ]),
                    ),
                ],
            ),
            vec!["INVALID_BUFFER_LAYOUT"],
        ),
        (
            "operators",
            make_object(
                "hclObjectSpaceSkinPNOperator",
                vec![member(
                    "localPNs",
                    HkxValue::Array(vec![HkxValue::Object(vec![
                        member("localPosition", zeroes.clone()),
                        member("localNormal", zeroes),
                    ])]),
                )],
            ),
            vec!["INVALID_PACKED_LOCAL_BLOCK"],
        ),
    ];

    for (slot_name, broken, expected) in cases {
        let mut objects = cloth_objects(Some((3, vec![0])));
        objects.push(broken);
        objects[0]
            .members
            .iter_mut()
            .find(|m| m.name == slot_name)
            .expect("cloth data member")
            .value = HkxValue::Array(vec![ptr(2)]);
        let (errors, _) = issue_codes(objects);
        for code in expected {
            assert!(errors.contains(&code.to_string()), "{code} missing: {errors:?}");
        }
    }
}
