// Blob-level ClothEditor API: every edit must mutate the expected field and
// produce bytes that re-parse as a valid packfile.

use havok_native::api;
use havok_native::hkx::descriptors::DescriptorRegistry;
use havok_native::hkx::types::HkxValue;
use havok_native::hkx::{HkxFile, HkxMember, HkxObject, read_packfile, write_hkx};

const STANDARD_CS: usize = 0;
const BEND_CS: usize = 1;
const SIM_OP: usize = 2;
const CAPSULE: usize = 3;
const SIM_CLOTH: usize = 5;

fn member(name: &str, value: HkxValue) -> HkxMember {
    HkxMember {
        name: name.to_string(),
        value,
    }
}

fn object(name: &str, class_name: &str, members: Vec<HkxMember>) -> HkxObject {
    HkxObject {
        name: Some(name.to_string()),
        offset: 0,
        signature: 0,
        class_name: class_name.to_string(),
        members,
    }
}

fn string(value: &str) -> HkxValue {
    HkxValue::String {
        value: value.to_string(),
        is_null: false,
    }
}

fn particle(mass: f32, inv_mass: f32) -> HkxValue {
    HkxValue::Object(vec![
        member("mass", HkxValue::F32(mass)),
        member("invMass", HkxValue::F32(inv_mass)),
        member("radius", HkxValue::F32(1.0)),
        member("friction", HkxValue::F32(0.2)),
    ])
}

fn link(field: &str, stiffness: f32) -> HkxValue {
    HkxValue::Object(vec![member(field, HkxValue::F32(stiffness))])
}

fn build_synthetic_blob() -> Vec<u8> {
    let objects = vec![
        object(
            "#0000",
            "hclStandardLinkConstraintSet",
            vec![member(
                "links",
                HkxValue::Array(vec![link("stiffness", 1.0), link("stiffness", 1.0)]),
            )],
        ),
        object(
            "#0001",
            "hclBendStiffnessConstraintSet",
            vec![member("links", HkxValue::Array(vec![link("bendStiffness", 0.5)]))],
        ),
        object(
            "#0002",
            "hclSimulateOperator",
            vec![
                member("subSteps", HkxValue::U32(1)),
                member("numberOfSolveIterations", HkxValue::U32(4)),
            ],
        ),
        object(
            "#0003",
            "hclCapsuleShape",
            vec![
                member("start", HkxValue::F32List(vec![0.0, 0.0, 0.0, 0.0])),
                member("end", HkxValue::F32List(vec![5.0, 0.0, 0.0, 0.0])),
                member("dir", HkxValue::F32List(vec![1.0, 0.0, 0.0, 0.0])),
                member("radius", HkxValue::F32(2.0)),
            ],
        ),
        object(
            "#0004",
            "hclCollidable",
            vec![
                member("name", string("bone_1")),
                member("shape", HkxValue::Pointer(Some(CAPSULE))),
            ],
        ),
        object(
            "#0005",
            "hclSimClothData",
            vec![
                member(
                    "particleDatas",
                    HkxValue::Array(vec![
                        particle(0.1, 10.0),
                        particle(0.1, 10.0),
                        particle(0.2, 5.0),
                    ]),
                ),
                member("fixedParticles", HkxValue::Array(vec![])),
                member(
                    "staticConstraintSets",
                    HkxValue::Array(vec![
                        HkxValue::Pointer(Some(STANDARD_CS)),
                        HkxValue::Pointer(Some(BEND_CS)),
                    ]),
                ),
                member(
                    "perInstanceCollidables",
                    HkxValue::Array(vec![HkxValue::Pointer(Some(4))]),
                ),
                member(
                    "collidableTransformMap",
                    HkxValue::Object(vec![member(
                        "transformIndices",
                        HkxValue::Array(vec![HkxValue::U32(1)]),
                    )]),
                ),
                member(
                    "simulationInfo",
                    HkxValue::Object(vec![
                        member("gravity", HkxValue::F32List(vec![0.0, 0.0, -9.8, 0.0])),
                        member("globalDampingPerSecond", HkxValue::F32(0.1)),
                        member("collisionTolerance", HkxValue::F32(0.05)),
                    ]),
                ),
            ],
        ),
        object(
            "#0006",
            "hclClothData",
            vec![
                member("name", string("TestCloth")),
                member(
                    "simClothDatas",
                    HkxValue::Array(vec![HkxValue::Pointer(Some(SIM_CLOTH))]),
                ),
                member("operators", HkxValue::Array(vec![HkxValue::Pointer(Some(SIM_OP))])),
                member("clothStateDatas", HkxValue::Array(vec![])),
                member("bufferDefinitions", HkxValue::Array(vec![])),
                member("transformSetDefinitions", HkxValue::Array(vec![])),
            ],
        ),
    ];
    let file = HkxFile::from_tagxml(11, "hk_2014.1.0-r1", objects);
    write_hkx(&file, &mut DescriptorRegistry::new())
}

fn reparse(blob: &[u8]) -> HkxFile {
    read_packfile(blob).expect("edited blob must re-parse as a packfile")
}

fn field<'a>(members: &'a [HkxMember], name: &str) -> &'a HkxValue {
    &members
        .iter()
        .find(|m| m.name == name)
        .unwrap_or_else(|| panic!("member {name} missing"))
        .value
}

fn object_members<'a>(value: &'a HkxValue) -> &'a [HkxMember] {
    match value {
        HkxValue::Object(members) => members,
        other => panic!("expected Object, got {other:?}"),
    }
}

fn array<'a>(value: &'a HkxValue) -> &'a [HkxValue] {
    match value {
        HkxValue::Array(items) => items,
        other => panic!("expected Array, got {other:?}"),
    }
}

fn f32_of(value: &HkxValue) -> f32 {
    match value {
        HkxValue::F32(v) => *v,
        other => panic!("expected F32, got {other:?}"),
    }
}

fn u32_of(value: &HkxValue) -> u32 {
    match value {
        HkxValue::U32(v) => *v,
        HkxValue::I32(v) => *v as u32,
        HkxValue::U16(v) => *v as u32,
        HkxValue::U8(v) => *v as u32,
        other => panic!("expected integer, got {other:?}"),
    }
}

fn particle_field(file: &HkxFile, index: usize, name: &str) -> f32 {
    let particles = array(field(&file.objects()[SIM_CLOTH].members, "particleDatas"));
    f32_of(field(object_members(&particles[index]), name))
}

fn sim_info_field(file: &HkxFile) -> &[HkxMember] {
    object_members(field(&file.objects()[SIM_CLOTH].members, "simulationInfo"))
}

#[test]
fn particle_edits_round_trip() {
    let blob = build_synthetic_blob();

    let (all, count) = api::cloth_set_particle_mass_all(&blob, 0.05, 0).unwrap();
    assert_eq!(count, 3);
    let file = reparse(&all);
    for i in 0..3 {
        assert!((particle_field(&file, i, "mass") - 0.05).abs() < 1e-5);
        assert!((particle_field(&file, i, "invMass") - 20.0).abs() < 1e-3);
    }

    let (subset, count) = api::cloth_set_particles_mass(&blob, &[0, 2], 0.07, 0).unwrap();
    assert_eq!(count, 2);
    let file = reparse(&subset);
    let masses: Vec<f32> = (0..3).map(|i| particle_field(&file, i, "mass")).collect();
    assert!((masses[0] - 0.07).abs() < 1e-5);
    assert!((masses[1] - 0.1).abs() < 1e-5, "unselected particle must keep its mass");
    assert!((masses[2] - 0.07).abs() < 1e-5);

    let (radius, count) = api::cloth_set_particles_radius(&blob, &[1], 3.5, 0).unwrap();
    assert_eq!(count, 1);
    let file = reparse(&radius);
    assert!((particle_field(&file, 1, "radius") - 3.5).abs() < 1e-5);
    assert!((particle_field(&file, 0, "radius") - 1.0).abs() < 1e-5);

    let (fixed, _) = api::cloth_set_particle_fixed(&blob, 1, true, 0).unwrap();
    let file = reparse(&fixed);
    let pinned: Vec<u32> = array(field(&file.objects()[SIM_CLOTH].members, "fixedParticles"))
        .iter()
        .map(u32_of)
        .collect();
    assert!(pinned.contains(&1), "particle 1 must be pinned, got {pinned:?}");
}

#[test]
fn simulation_and_operator_edits_round_trip() {
    let blob = build_synthetic_blob();

    let file = reparse(&api::cloth_set_gravity(&blob, [0.0, 0.0, -686.7, 0.0], 0).unwrap());
    match field(sim_info_field(&file), "gravity") {
        HkxValue::F32List(g) => assert!((g[2] + 686.7).abs() < 0.1, "gravity {g:?}"),
        other => panic!("gravity not F32List: {other:?}"),
    }

    let file = reparse(&api::cloth_set_damping(&blob, 0.9999, 0).unwrap());
    assert!((f32_of(field(sim_info_field(&file), "globalDampingPerSecond")) - 0.9999).abs() < 1e-6);

    let file = reparse(&api::cloth_set_collision_tolerance(&blob, 14.0, 0).unwrap());
    assert!((f32_of(field(sim_info_field(&file), "collisionTolerance")) - 14.0).abs() < 1e-6);

    let file = reparse(&api::cloth_set_substeps(&blob, 4).unwrap());
    assert_eq!(u32_of(field(&file.objects()[SIM_OP].members, "subSteps")), 4);

    let file = reparse(&api::cloth_set_solver_iterations(&blob, 8).unwrap());
    assert_eq!(
        u32_of(field(&file.objects()[SIM_OP].members, "numberOfSolveIterations")),
        8
    );
}

#[test]
fn constraint_and_capsule_edits_round_trip() {
    let blob = build_synthetic_blob();

    let (scaled, count) = api::cloth_scale_stiffness(&blob, Some("standard"), 0.5, 0).unwrap();
    assert_eq!(count, 2, "only the standard-link set matches the filter");
    let file = reparse(&scaled);
    for l in array(field(&file.objects()[STANDARD_CS].members, "links")) {
        assert!((f32_of(field(object_members(l), "stiffness")) - 0.5).abs() < 1e-5);
    }
    let bend = &array(field(&file.objects()[BEND_CS].members, "links"))[0];
    assert!((f32_of(field(object_members(bend), "bendStiffness")) - 0.5).abs() < 1e-5);

    let capsule_radius =
        |blob: &[u8]| f32_of(field(&reparse(blob).objects()[CAPSULE].members, "radius"));
    let set = api::cloth_set_capsule_radius(&blob, 0, 5.0, 0).unwrap();
    assert!((capsule_radius(&set) - 5.0).abs() < 1e-5);
    let (scaled, count) = api::cloth_scale_all_capsule_radii(&blob, 1.5, 0).unwrap();
    assert_eq!(count, 1);
    assert!((capsule_radius(&scaled) - 3.0).abs() < 1e-4);

    let n_before = reparse(&blob).objects().len();
    let (added, new_idx) =
        api::cloth_add_capsule(&blob, "bone_7", 1.0, [0.0; 4], [10.0, 0.0, 0.0, 0.0], 0).unwrap();
    let file = reparse(&added);
    assert_eq!(file.objects().len(), n_before + 2);
    assert_eq!(file.objects()[n_before].class_name, "hclCapsuleShape");
    assert_eq!(file.objects()[n_before + 1].class_name, "hclCollidable");
    let removed = api::cloth_remove_capsule(&added, new_idx, 0).unwrap();
    assert_eq!(reparse(&removed).objects().len(), n_before);
}

#[test]
fn summary_and_inspect_json_describe_blob() {
    let blob = build_synthetic_blob();

    let summary: serde_json::Value =
        serde_json::from_str(&api::cloth_summary_json(&blob, 0).unwrap()).unwrap();
    assert_eq!(summary["particle_count"], 3);
    assert_eq!(summary["fixed_count"], 0);
    assert_eq!(summary["capsule_count"], 1);
    assert_eq!(summary["operator"]["substeps"], 1);
    assert_eq!(summary["operator"]["iterations"], 4);

    let inspect: serde_json::Value =
        serde_json::from_str(&api::cloth_inspect_blob_json(&blob).unwrap()).unwrap();
    assert!(inspect["object_count"].as_u64().unwrap() >= 7);
    let classes: Vec<&str> = inspect["objects"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|o| o["class"].as_str())
        .collect();
    assert!(classes.contains(&"hclClothData"));
    assert!(classes.contains(&"hclSimClothData"));
}
