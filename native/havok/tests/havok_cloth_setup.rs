use havok_native::cloth::materials::get_preset;
use havok_native::cloth::setup::ClothSetupObject;
use havok_native::cloth::skinning::auto_skin_to_cloth_bones;
use havok_native::cloth::topology::THIN_CLOTH;

const FIXTURE_PATH: &str = "tests/fixtures/cloth_setup_min.json";

#[test]
fn cloth_setup_round_trips_min_fixture() {
    let src = std::fs::read_to_string(FIXTURE_PATH)
        .expect("cloth_setup_min.json fixture missing — run generation step");
    let parsed = ClothSetupObject::from_json(&src).expect("from_json failed");
    let re_serialized = parsed.to_json().expect("to_json failed");

    let v_src: serde_json::Value = serde_json::from_str(&src).unwrap();
    let v_out: serde_json::Value = serde_json::from_str(&re_serialized).unwrap();
    assert_eq!(v_src, v_out, "JSON round-trip mismatch");
}

#[test]
fn material_preset_lookup_is_case_insensitive() {
    let a = get_preset("Cotton").expect("Cotton not found");
    let b = get_preset("cotton").expect("cotton not found");
    let c = get_preset("  Cotton  ").expect("  Cotton   not found");
    assert_eq!(a.name, b.name);
    assert_eq!(a.name, c.name);
    assert_eq!(a.particle_mass, b.particle_mass);
}

#[test]
fn material_preset_unknown_returns_error() {
    let result = get_preset("Plutonium");
    assert!(
        result.is_err(),
        "Expected error for unknown preset 'Plutonium'"
    );
}

#[test]
fn topology_thin_cloth_builds_standard_and_bend_constraints() {
    let fixed: Vec<usize> = vec![0, 1, 2];
    let constraints = THIN_CLOTH.build_constraints("Top", 100, &fixed);
    assert!(!constraints.is_empty());

    let has_standard = constraints.iter().any(|c| c.setup_type() == "StandardLink");
    let has_bend = constraints
        .iter()
        .any(|c| c.setup_type() == "BendStiffness");
    assert!(
        has_standard,
        "Expected at least one StandardLink constraint"
    );
    assert!(has_bend, "Expected at least one BendStiffness constraint");
}

#[test]
fn auto_skin_assigns_nearest_bone_full_weight() {
    // Vertex at origin; bone A at origin, bone B far away.
    let vertex_positions = vec![[0.0f32, 0.0, 0.0, 0.0]];
    let bone_positions = vec![[0.0f32, 0.0, 0.0, 0.0], [10.0, 0.0, 0.0, 0.0]];

    let weights = auto_skin_to_cloth_bones(&vertex_positions, &bone_positions, 4, 2.0);
    assert_eq!(weights.len(), 1);
    let vw = &weights[0];
    // Bone 0 is at exact position — should get full weight
    assert_eq!(
        vw.len(),
        1,
        "Expected exactly one bone entry (bone 0 at exact position)"
    );
    assert_eq!(vw[0].0, 0, "Expected bone index 0");
    assert!(
        (vw[0].1 - 1.0).abs() < 1e-6,
        "Expected weight ~1.0 for nearest bone"
    );
}
