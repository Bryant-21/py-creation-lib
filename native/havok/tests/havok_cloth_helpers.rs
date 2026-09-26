use havok_native::api;

#[test]
fn catalog_lookups_are_case_insensitive_and_material_apply_patches_setup() {
    let silk: serde_json::Value =
        serde_json::from_str(&api::cloth_material_get("  silk ").unwrap()).unwrap();
    assert_eq!(silk["name"], "Silk");
    let thin: serde_json::Value =
        serde_json::from_str(&api::cloth_topology_get("thin_cloth").unwrap()).unwrap();
    assert!(thin["use_standard_links"].as_bool().unwrap());
    assert!(!thin["use_local_range"].as_bool().unwrap());
    let robe: serde_json::Value =
        serde_json::from_str(&api::cloth_template_get("bathrobe").unwrap()).unwrap();
    assert_eq!(robe["name"], "Bathrobe");
    assert!(!robe["capsules"].as_array().unwrap().is_empty());

    assert!(api::cloth_material_get("Plutonium").is_err());
    assert!(api::cloth_topology_get("bogus_topo").is_err());
    assert!(api::cloth_template_get("NotATemplate").is_err());

    let setup_json = serde_json::json!({
        "name": "test",
        "sim_cloth_setups": [{
            "name": "sc0",
            "particle_mass": {"type": 0, "constant_value": 0.0},
            "particle_radius": {"type": 0, "constant_value": 0.0},
            "particle_friction": {"type": 0, "constant_value": 0.0},
            "global_damping_per_second": 0.0,
            "collision_tolerance": 0.0,
            "constraint_setups": []
        }],
        "buffer_setups": [],
        "transform_set_setups": [],
        "operator_setups": [],
        "state_setups": []
    })
    .to_string();
    let patched: serde_json::Value =
        serde_json::from_str(&api::cloth_material_apply(&setup_json, "Silk").unwrap()).unwrap();
    let sc = &patched["sim_cloth_setups"][0];
    let mass = sc["particle_mass"]["constant_value"].as_f64().unwrap();
    let damping = sc["global_damping_per_second"].as_f64().unwrap();
    assert!((mass - silk["particle_mass"].as_f64().unwrap()).abs() < 1e-6);
    assert!(damping > 0.0, "damping must be patched from the preset");
}

#[test]
fn cloth_region_generate_minimal_grid() {
    let region_json = serde_json::json!({
        "name": "TestRegion",
        "positions": [
            [0.0, 0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0, 0.0]
        ],
        "triangles": [[0, 1, 2], [1, 3, 2]],
        "fixed_indices": [0, 1]
    })
    .to_string();

    let result_json = api::cloth_region_generate(&region_json, "thin_cloth", "{}").unwrap();
    let v: serde_json::Value = serde_json::from_str(&result_json).unwrap();
    let sc = &v["sim_cloth_setups"][0];
    assert_eq!(sc["name"], "TestRegion");
    let constraints = sc["constraint_setups"].as_array().unwrap();
    assert!(!constraints.is_empty(), "expected at least one constraint");
}

#[test]
fn generated_bones_convert_to_transform_set() {
    let positions_json = serde_json::json!([
        [0.0, 0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0, 0.0],
    ])
    .to_string();
    let args_json = serde_json::json!({
        "bone_prefix": "Cloth_BN",
        "rows": 2,
        "cols": 2,
        "parent_bone": "COM"
    })
    .to_string();

    let bones_json = api::cloth_generate_bones_from_particles(&positions_json, &args_json).unwrap();
    let bones: Vec<serde_json::Value> = serde_json::from_str(&bones_json).unwrap();
    assert_eq!(bones.len(), 4, "expected 4 bones for 2x2 grid");
    for bone in &bones {
        assert_eq!(bone["parent_bone"], "COM");
        assert!(bone["name"].as_str().unwrap().starts_with("Cloth_BN_"));
    }

    let ts: serde_json::Value =
        serde_json::from_str(&api::cloth_bones_to_transform_set(&bones_json).unwrap()).unwrap();
    let names = ts["names"].as_array().unwrap();
    let positions = ts["positions"].as_array().unwrap();
    assert_eq!(names.len(), 4);
    assert_eq!(positions.len(), 4);
    for (i, bone) in bones.iter().enumerate() {
        assert_eq!(names[i], bone["name"]);
        assert_eq!(positions[i], bone["position"]);
    }
}
