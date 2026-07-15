// ---------------------------------------------------------------------------
// cloth_material_list — returns JSON array of preset names
// ---------------------------------------------------------------------------

#[test]
fn cloth_material_list_returns_nine_names() {
    let json = havok_native::api::cloth_material_list().unwrap();
    let names: Vec<String> = serde_json::from_str(&json).unwrap();
    assert_eq!(names.len(), 9, "expected 9 material presets");
    assert!(names.contains(&"Silk".to_string()));
    assert!(names.contains(&"Cotton".to_string()));
    assert!(names.contains(&"Chain Mail".to_string()));
}

// ---------------------------------------------------------------------------
// cloth_material_get — returns JSON with preset fields
// ---------------------------------------------------------------------------

#[test]
fn cloth_material_get_silk_fields() {
    let json = havok_native::api::cloth_material_get("Silk").unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v["name"], "Silk");
    let mass: f64 = v["particle_mass"].as_f64().unwrap();
    assert!(
        (mass - 0.01).abs() < 1e-6,
        "Silk particle_mass should be 0.01, got {mass}"
    );
    let bend: f64 = v["bend_stiffness"].as_f64().unwrap();
    assert!(
        (bend - 0.05).abs() < 1e-6,
        "Silk bend_stiffness should be 0.05, got {bend}"
    );
}

#[test]
fn cloth_material_get_unknown_returns_error() {
    assert!(havok_native::api::cloth_material_get("UnknownXYZ").is_err());
}

// ---------------------------------------------------------------------------
// cloth_material_apply — patches sim_cloth_setups in a setup JSON
// ---------------------------------------------------------------------------

#[test]
fn cloth_material_apply_patches_particle_mass() {
    // Build a minimal setup JSON with one sim_cloth_setup
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

    let patched_json = havok_native::api::cloth_material_apply(&setup_json, "Silk").unwrap();
    let v: serde_json::Value = serde_json::from_str(&patched_json).unwrap();
    let sc = &v["sim_cloth_setups"][0];
    let mass = sc["particle_mass"]["constant_value"].as_f64().unwrap();
    assert!(
        (mass - 0.01).abs() < 1e-6,
        "Silk particle_mass should be 0.01 after apply, got {mass}"
    );
    let damping = sc["global_damping_per_second"].as_f64().unwrap();
    assert!(
        (damping - 0.05).abs() < 1e-6,
        "Silk damping should be 0.05, got {damping}"
    );
}

// ---------------------------------------------------------------------------
// cloth_topology_list — returns JSON array with 5 presets
// ---------------------------------------------------------------------------

#[test]
fn cloth_topology_list_returns_five_presets() {
    let json = havok_native::api::cloth_topology_list().unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    let arr = v.as_array().unwrap();
    assert_eq!(arr.len(), 5, "expected 5 topology presets");
    let names: Vec<&str> = arr.iter().map(|x| x["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"thin_cloth"));
    assert!(names.contains(&"chain"));
    assert!(names.contains(&"soft_body"));
}

// ---------------------------------------------------------------------------
// cloth_topology_get — returns JSON with topology details
// ---------------------------------------------------------------------------

#[test]
fn cloth_topology_get_thin_cloth_fields() {
    let json = havok_native::api::cloth_topology_get("thin_cloth").unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v["name"], "thin_cloth");
    // thin_cloth: use_standard_links=true, use_bend_stiffness=true
    assert!(v["use_standard_links"].as_bool().unwrap());
    assert!(v["use_bend_stiffness"].as_bool().unwrap());
    // thin_cloth does NOT use local range
    assert!(!v["use_local_range"].as_bool().unwrap());
}

#[test]
fn cloth_topology_get_unknown_returns_error() {
    assert!(havok_native::api::cloth_topology_get("bogus_topo").is_err());
}

// ---------------------------------------------------------------------------
// cloth_region_generate — returns setup JSON from a minimal region def
// ---------------------------------------------------------------------------

#[test]
fn cloth_region_generate_minimal_grid() {
    // 4-particle 2x2 grid, 2 triangles
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

    let result_json =
        havok_native::api::cloth_region_generate(&region_json, "thin_cloth", "{}").unwrap();
    let v: serde_json::Value = serde_json::from_str(&result_json).unwrap();
    // Should have a ClothSetupObject structure
    assert!(
        v["sim_cloth_setups"].as_array().is_some(),
        "missing sim_cloth_setups"
    );
    let sc = &v["sim_cloth_setups"][0];
    // The sim_cloth_setup should be named "TestRegion"
    assert_eq!(sc["name"], "TestRegion");
    // Should have at least one constraint (StandardLink for thin_cloth)
    let constraints = sc["constraint_setups"].as_array().unwrap();
    assert!(!constraints.is_empty(), "expected at least one constraint");
}

// ---------------------------------------------------------------------------
// cloth_reverse_to_setup — bake a known setup, reverse blob, check structure
// ---------------------------------------------------------------------------

#[test]
fn cloth_reverse_to_setup_round_trip_preserves_particle_count() {
    use havok_native::cloth::bake::bake_cloth_setup;
    use havok_native::cloth::setup::mesh::SimulationSetupMesh;
    use havok_native::cloth::setup::operator_setup::{
        OperatorSetupObject, SimulateSetup, SimulateSetupConfig,
    };
    use havok_native::cloth::setup::types::VertexFloatInput;
    use havok_native::cloth::setup::{
        BufferSetupObject, ClothSetupObject, SimClothSetupObject, TransformSetSetupObject,
    };
    use havok_native::hkx;

    // Build a minimal 4-particle setup and bake it
    let positions = vec![
        [0.0f32, 0.0, 0.0, 0.0],
        [1.0f32, 0.0, 0.0, 0.0],
        [0.0f32, 1.0, 0.0, 0.0],
        [1.0f32, 1.0, 0.0, 0.0],
    ];
    let triangles = vec![[0u32, 1, 2], [1u32, 3, 2]];

    let sim_mesh = SimulationSetupMesh {
        positions: positions.clone(),
        triangles,
        sim_to_render_map: (0..4).map(|i| vec![i]).collect(),
        render_to_sim_map: (0..4).collect(),
        ..Default::default()
    };

    let sc = SimClothSetupObject {
        name: "TestSC".to_string(),
        simulation_mesh: Some(sim_mesh),
        particle_mass: VertexFloatInput::constant(0.02),
        particle_radius: VertexFloatInput::constant(0.5),
        particle_friction: VertexFloatInput::constant(0.35),
        ..Default::default()
    };

    let setup = ClothSetupObject {
        name: "TestCloth".to_string(),
        sim_cloth_setups: vec![sc],
        buffer_setups: vec![],
        transform_set_setups: vec![],
        operator_setups: vec![OperatorSetupObject::Simulate(SimulateSetup {
            name: "sim".to_string(),
            sim_cloth_setup_name: "TestSC".to_string(),
            configs: vec![SimulateSetupConfig::default()],
        })],
        state_setups: vec![],
    };

    let hkx_file = bake_cloth_setup(&setup).expect("bake failed");
    let blob = {
        let mut reg = hkx::descriptors::DescriptorRegistry::new();
        hkx::write_hkx(&hkx_file, &mut reg)
    };

    // Now reverse it
    let setup_json = havok_native::api::cloth_reverse_to_setup(&blob).unwrap();
    let v: serde_json::Value = serde_json::from_str(&setup_json).unwrap();
    let sc_arr = v["sim_cloth_setups"].as_array().unwrap();
    assert_eq!(
        sc_arr.len(),
        1,
        "should have one sim_cloth_setup after reverse"
    );
}

// ---------------------------------------------------------------------------
// cloth_generate_bones_from_particles — JSON in / JSON out
// ---------------------------------------------------------------------------

#[test]
fn cloth_generate_bones_from_particles_count_matches() {
    let positions_json = serde_json::json!([
        [0.0, 0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0, 0.0],
    ])
    .to_string();

    // 2 rows x 2 cols = 4 bones for 4 particles
    let args_json = serde_json::json!({
        "bone_prefix": "Cloth_BN",
        "rows": 2,
        "cols": 2,
        "parent_bone": "COM"
    })
    .to_string();

    let bones_json =
        havok_native::api::cloth_generate_bones_from_particles(&positions_json, &args_json)
            .unwrap();
    let bones: Vec<serde_json::Value> = serde_json::from_str(&bones_json).unwrap();
    assert_eq!(bones.len(), 4, "expected 4 bones for 2x2 grid");
    // All bones should have parent_bone = "COM"
    for bone in &bones {
        assert_eq!(bone["parent_bone"], "COM");
    }
    // Name format: {prefix}_{row_label}_{col_label}
    let first_name = bones[0]["name"].as_str().unwrap();
    assert!(
        first_name.starts_with("Cloth_BN_"),
        "bone name should start with prefix, got {first_name}"
    );
}

// ---------------------------------------------------------------------------
// cloth_bones_to_transform_set — JSON bones → JSON {names, positions}
// ---------------------------------------------------------------------------

#[test]
fn cloth_bones_to_transform_set_extracts_names_and_positions() {
    let bones_json = serde_json::json!([
        {"name": "Cloth_BN_A_001", "position": [1.0, 2.0, 3.0, 0.0], "parent_bone": "COM"},
        {"name": "Cloth_BN_A_002", "position": [4.0, 5.0, 6.0, 0.0], "parent_bone": "COM"}
    ])
    .to_string();

    let ts_json = havok_native::api::cloth_bones_to_transform_set(&bones_json).unwrap();
    let v: serde_json::Value = serde_json::from_str(&ts_json).unwrap();
    let names = v["names"].as_array().unwrap();
    let positions = v["positions"].as_array().unwrap();
    assert_eq!(names.len(), 2);
    assert_eq!(positions.len(), 2);
    assert_eq!(names[0], "Cloth_BN_A_001");
    let pos0 = positions[0].as_array().unwrap();
    assert!((pos0[0].as_f64().unwrap() - 1.0).abs() < 1e-6);
}

// ---------------------------------------------------------------------------
// cloth_auto_skin — setup_json + args → setup JSON with skin data
// ---------------------------------------------------------------------------

#[test]
fn cloth_auto_skin_weights_sum_to_one() {
    // Minimal setup with 4 positions + 2 bones
    let positions_json = serde_json::json!([
        [0.0, 0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0, 0.0],
        [2.0, 0.0, 0.0, 0.0],
        [3.0, 0.0, 0.0, 0.0],
    ])
    .to_string();
    let bone_positions_json =
        serde_json::json!([[0.5, 0.0, 0.0, 0.0], [2.5, 0.0, 0.0, 0.0],]).to_string();
    let args_json = serde_json::json!({
        "max_bones_per_vertex": 2,
        "falloff_power": 2.0
    })
    .to_string();

    let weights_json =
        havok_native::api::cloth_auto_skin(&positions_json, &bone_positions_json, &args_json)
            .unwrap();
    let weights: Vec<Vec<(usize, f64)>> = serde_json::from_str(&weights_json).unwrap();
    assert_eq!(weights.len(), 4);
    for (vi, vertex_weights) in weights.iter().enumerate() {
        let sum: f64 = vertex_weights.iter().map(|(_, w)| w).sum();
        assert!(
            (sum - 1.0).abs() < 1e-5,
            "vertex {vi} weights should sum to 1.0, got {sum}"
        );
    }
}

// ---------------------------------------------------------------------------
// cloth_template_list — returns JSON array with known template names
// ---------------------------------------------------------------------------

#[test]
fn cloth_template_list_returns_five_templates() {
    let json = havok_native::api::cloth_template_list().unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    let arr = v.as_array().unwrap();
    assert_eq!(arr.len(), 5, "expected 5 templates");
    let names: Vec<&str> = arr.iter().map(|x| x["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"Bathrobe"), "missing Bathrobe");
    assert!(names.contains(&"Cape"), "missing Cape");
}

// ---------------------------------------------------------------------------
// cloth_template_get — returns JSON with template metadata
// ---------------------------------------------------------------------------

#[test]
fn cloth_template_get_bathrobe_fields() {
    let json = havok_native::api::cloth_template_get("Bathrobe").unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v["name"], "Bathrobe");
    // Bathrobe: 11x6 grid = 66 particles
    let num_particles = v["num_particles"].as_u64().unwrap();
    assert_eq!(
        num_particles, 66,
        "Bathrobe should have 66 particles (11x6)"
    );
    // Should have capsules
    let capsules = v["capsules"].as_array().unwrap();
    assert!(!capsules.is_empty(), "Bathrobe should have capsules");
    // Should have material_preset field
    assert!(v["material_preset"].is_string());
}

#[test]
fn cloth_template_get_unknown_returns_error() {
    assert!(havok_native::api::cloth_template_get("NotATemplate").is_err());
}
