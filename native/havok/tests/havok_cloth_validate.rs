use havok_native::cloth::validate::{LintIssue, Severity};
use havok_native::cloth::{ClothData, validate_cloth_data};
use havok_native::hkx::types::HkxValue;
use havok_native::hkx::{HkxFile, HkxMember, HkxObject};

// ---------------------------------------------------------------------------
// Synthetic HkxFile helpers
// ---------------------------------------------------------------------------

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

/// Build a minimal HkxFile with one hclClothData (at index 0) pointing to
/// one hclSimClothData (at index 1).  The sim cloth has `n_particles`
/// particle entries and the given `fixed_particles`.  Constraint sets,
/// collidables, and pose are left empty so we can test just particle checks.
fn minimal_cloth_file(n_particles: usize, fixed_particles: Vec<u32>) -> HkxFile {
    // index 1: the sim cloth object
    let particle_entries: Vec<HkxValue> = (0..n_particles)
        .map(|_| {
            // each particle is an Object with invMass field (non-zero → movable)
            HkxValue::Object(vec![member("invMass", HkxValue::F32(1.0))])
        })
        .collect();

    let fixed_values: Vec<HkxValue> = fixed_particles.iter().map(|&i| HkxValue::U32(i)).collect();

    let sim_cloth = make_object(
        "hclSimClothData",
        vec![
            member("name", string_val("TestSimCloth")),
            member("particleDatas", HkxValue::Array(particle_entries)),
            member("fixedParticles", HkxValue::Array(fixed_values)),
            member("staticConstraintSets", HkxValue::Array(vec![])),
            member("perInstanceCollidables", HkxValue::Array(vec![])),
            member("simClothPoses", HkxValue::Array(vec![])),
        ],
    );

    // index 0: the cloth data object pointing to sim cloth at index 1
    let cloth_data = make_object(
        "hclClothData",
        vec![
            member("name", string_val("TestCloth")),
            member("simClothDatas", HkxValue::Array(vec![ptr(1)])),
            member("clothStateDatas", HkxValue::Array(vec![])),
            member("operators", HkxValue::Array(vec![])),
            member("bufferDefinitions", HkxValue::Array(vec![])),
            member("transformSetDefinitions", HkxValue::Array(vec![])),
        ],
    );

    HkxFile::from_tagxml(11, "hk_2012.2.0-r1", vec![cloth_data, sim_cloth])
}

/// HkxFile with hclClothData but empty simClothDatas array.
fn cloth_file_no_sim_cloth() -> HkxFile {
    let cloth_data = make_object(
        "hclClothData",
        vec![
            member("name", string_val("TestCloth")),
            member("simClothDatas", HkxValue::Array(vec![])),
            member("clothStateDatas", HkxValue::Array(vec![])),
            member("operators", HkxValue::Array(vec![])),
            member("bufferDefinitions", HkxValue::Array(vec![])),
            member("transformSetDefinitions", HkxValue::Array(vec![])),
        ],
    );
    HkxFile::from_tagxml(11, "hk_2012.2.0-r1", vec![cloth_data])
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn no_cloth_data_returns_no_cloth_data_error() {
    let result = validate_cloth_data(None);

    assert!(!result.is_valid(), "None cloth_data must not be valid");
    let errors = result.errors();
    assert_eq!(errors.len(), 1, "expected exactly one error");
    assert_eq!(errors[0].code, "NO_CLOTH_DATA");
}

#[test]
fn empty_sim_cloth_datas_returns_no_sim_cloth_error() {
    let file = cloth_file_no_sim_cloth();
    let cloth_data = ClothData::from_hkx_file(&file);

    let result = validate_cloth_data(cloth_data.as_ref());

    assert!(
        !result.is_valid(),
        "cloth with no sim cloths must not be valid"
    );
    let codes: Vec<&str> = result
        .errors()
        .into_iter()
        .map(|i| i.code.as_str())
        .collect();
    assert!(
        codes.contains(&"NO_SIM_CLOTH"),
        "expected NO_SIM_CLOTH error, got: {codes:?}",
    );
}

#[test]
fn all_particles_fixed_emits_warning() {
    // 3 particles, all 3 fixed
    let file = minimal_cloth_file(3, vec![0, 1, 2]);
    let cloth_data = ClothData::from_hkx_file(&file);

    let result = validate_cloth_data(cloth_data.as_ref());

    let warn_codes: Vec<&str> = result
        .warnings()
        .into_iter()
        .map(|i| i.code.as_str())
        .collect();
    assert!(
        warn_codes.contains(&"ALL_PARTICLES_FIXED"),
        "expected ALL_PARTICLES_FIXED warning, got warnings: {warn_codes:?}",
    );
}

#[test]
fn out_of_range_fixed_particle_emits_error() {
    // 3 particles, but fixed index 5 is out of range
    let file = minimal_cloth_file(3, vec![5]);
    let cloth_data = ClothData::from_hkx_file(&file);

    let result = validate_cloth_data(cloth_data.as_ref());

    let error_codes: Vec<&str> = result
        .errors()
        .into_iter()
        .map(|i| i.code.as_str())
        .collect();
    assert!(
        error_codes.contains(&"INVALID_FIXED_INDEX"),
        "expected INVALID_FIXED_INDEX error, got errors: {error_codes:?}",
    );
}

#[test]
fn zero_solver_settings_emit_errors() {
    let file = minimal_cloth_file(3, vec![0]);
    let mut objects = file.objects().to_vec();
    objects.push(make_object(
        "hclSimulateOperator",
        vec![
            member("subSteps", HkxValue::U32(0)),
            member("numberOfSolveIterations", HkxValue::I32(0)),
        ],
    ));
    let operators = objects[0]
        .members
        .iter_mut()
        .find(|member| member.name == "operators")
        .expect("operators member");
    operators.value = HkxValue::Array(vec![ptr(2)]);
    let file = HkxFile::from_tagxml(11, "hk_2012.2.0-r1", objects);
    let cloth_data = ClothData::from_hkx_file(&file);

    let result = validate_cloth_data(cloth_data.as_ref());
    let error_codes = result
        .errors()
        .into_iter()
        .map(|issue| issue.code.as_str())
        .collect::<Vec<_>>();

    assert!(error_codes.contains(&"ZERO_SUBSTEPS"), "{error_codes:?}");
    assert!(
        error_codes.contains(&"ZERO_SOLVE_ITERATIONS"),
        "{error_codes:?}"
    );
}

#[test]
fn zeroed_buffer_layout_emits_error() {
    let file = minimal_cloth_file(3, vec![0]);
    let mut objects = file.objects().to_vec();
    let element = || {
        HkxValue::Object(vec![
            member("vectorConversion", HkxValue::U8(0)),
            member("vectorSize", HkxValue::U8(0)),
            member("slotId", HkxValue::U8(0)),
            member("slotStart", HkxValue::U8(0)),
        ])
    };
    let slot = || {
        HkxValue::Object(vec![
            member("flags", HkxValue::U8(0)),
            member("stride", HkxValue::U8(0)),
        ])
    };
    objects.push(make_object(
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
    ));
    objects[0]
        .members
        .iter_mut()
        .find(|member| member.name == "bufferDefinitions")
        .expect("bufferDefinitions member")
        .value = HkxValue::Array(vec![ptr(2)]);
    let file = HkxFile::from_tagxml(11, "hk_2012.2.0-r1", objects);
    let cloth_data = ClothData::from_hkx_file(&file);

    let result = validate_cloth_data(cloth_data.as_ref());
    let error_codes = result
        .errors()
        .into_iter()
        .map(|issue| issue.code.as_str())
        .collect::<Vec<_>>();

    assert!(
        error_codes.contains(&"INVALID_BUFFER_LAYOUT"),
        "{error_codes:?}"
    );
}

#[test]
fn zeroed_packed_local_skinning_data_emits_error() {
    let file = minimal_cloth_file(3, vec![0]);
    let mut objects = file.objects().to_vec();
    let zeroes = HkxValue::Array((0..64).map(|_| HkxValue::I16(0)).collect());
    objects.push(make_object(
        "hclObjectSpaceSkinPNOperator",
        vec![member(
            "localPNs",
            HkxValue::Array(vec![HkxValue::Object(vec![
                member("localPosition", zeroes.clone()),
                member("localNormal", zeroes),
            ])]),
        )],
    ));
    objects[0]
        .members
        .iter_mut()
        .find(|member| member.name == "operators")
        .expect("operators member")
        .value = HkxValue::Array(vec![ptr(2)]);
    let file = HkxFile::from_tagxml(11, "hk_2014.1.0-r1", objects);
    let cloth_data = ClothData::from_hkx_file(&file);

    let result = validate_cloth_data(cloth_data.as_ref());
    let error_codes = result
        .errors()
        .into_iter()
        .map(|issue| issue.code.as_str())
        .collect::<Vec<_>>();

    assert!(
        error_codes.contains(&"INVALID_PACKED_LOCAL_BLOCK"),
        "{error_codes:?}"
    );
}

#[test]
fn bathrobe_validates_cleanly_when_fixture_present() {
    use std::path::PathBuf;

    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("../tests/fixtures/cloth/bathrobe_outfitm_cloth.hkx");

    if !path.exists() {
        return; // skip when fixture not prepared
    }

    let blob =
        std::fs::read(&path).unwrap_or_else(|e| panic!("failed to read bathrobe fixture: {e}"));

    let hkx =
        havok_native::hkx::read_packfile(&blob).expect("bathrobe blob must parse as packfile");

    let cloth_data = ClothData::from_hkx_file(&hkx);

    let result = validate_cloth_data(cloth_data.as_ref());

    assert!(
        result.is_valid(),
        "bathrobe cloth data must be lint-clean; errors: {:?}",
        result
            .errors()
            .iter()
            .map(|i| format!("{}: {}", i.code, i.message))
            .collect::<Vec<_>>(),
    );
}

// ---------------------------------------------------------------------------
// to_summary shape test
// ---------------------------------------------------------------------------

#[test]
fn validation_result_to_summary_matches_python_shape() {
    let result = validate_cloth_data(None);
    let summary = result.to_summary();

    assert_eq!(summary["valid"], serde_json::Value::Bool(false));
    assert!(summary["errors"].as_u64().unwrap() >= 1);
    let issues = summary["issues"].as_array().expect("issues must be array");
    assert!(!issues.is_empty());
    let first = &issues[0];
    assert!(first.get("severity").is_some(), "issue must have severity");
    assert!(first.get("code").is_some(), "issue must have code");
    assert!(first.get("message").is_some(), "issue must have message");
}

// ---------------------------------------------------------------------------
// Display format test
// ---------------------------------------------------------------------------

#[test]
fn lint_issue_display_matches_python_format() {
    let issue = LintIssue {
        severity: Severity::Error,
        code: "NO_CLOTH_DATA".to_string(),
        message: "No hclClothData found in the HKX file".to_string(),
    };
    let s = issue.to_string();
    assert_eq!(
        s,
        "[error] NO_CLOTH_DATA: No hclClothData found in the HKX file"
    );
}
