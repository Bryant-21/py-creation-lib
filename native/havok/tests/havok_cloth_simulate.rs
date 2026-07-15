use havok_native::cloth::setup::constraint_setup::{ConstraintSetupObject, StandardLinkSetup};
use havok_native::cloth::setup::mesh::SimulationSetupMesh;
use havok_native::cloth::setup::types::VertexSelectionInput;
use havok_native::cloth::setup::{ClothSetupObject, SimClothSetupObject};
use havok_native::hkx::types::HkxValue;

// ---------------------------------------------------------------------------
// Helper: build a 4×4 grid ClothSetupObject with two pinned top corners
// and one StandardLinkSetup.
//
// Grid layout (index = row * 4 + col):
//   row 0: indices  0..3  (top row — pinned at corners 0 and 3)
//   row 3: indices 12..15 (bottom row — should fall under gravity)
//
// Positions: x = col, y = row, z = 0  (XY plane, rows go in +Y).
// After gravity steps, z should decrease for unpinned particles.
// ---------------------------------------------------------------------------
fn build_simple_grid_setup() -> ClothSetupObject {
    let mut positions = Vec::new();
    let mut triangles = Vec::new();

    // 4×4 grid of particles in XY plane
    for row in 0u32..4 {
        for col in 0u32..4 {
            positions.push([col as f32, row as f32, 0.0f32, 1.0f32]);
        }
    }

    // Triangulate: each 1×1 cell → 2 triangles
    for row in 0u32..3 {
        for col in 0u32..3 {
            let tl = row * 4 + col;
            let tr = row * 4 + col + 1;
            let bl = (row + 1) * 4 + col;
            let br = (row + 1) * 4 + col + 1;
            triangles.push([tl, tr, bl]);
            triangles.push([tr, br, bl]);
        }
    }

    let mesh = SimulationSetupMesh {
        positions,
        triangles,
        ..Default::default()
    };

    // Pin top-left (0) and top-right (3) corners via CHANNEL.
    // vertex_selection_channels["fixed"] = [0, 3]
    let mut vertex_selection_channels = std::collections::HashMap::new();
    vertex_selection_channels.insert("fixed".to_string(), vec![0i32, 3i32]);

    let mesh_with_channels = SimulationSetupMesh {
        vertex_selection_channels,
        ..mesh
    };

    // fixed_particles = CHANNEL("fixed")
    let fixed_particles = VertexSelectionInput {
        kind: 2, // CHANNEL
        channel_name: "fixed".to_string(),
    };

    let standard_link = ConstraintSetupObject::StandardLink(StandardLinkSetup::default());

    let scd = SimClothSetupObject {
        simulation_mesh: Some(mesh_with_channels),
        fixed_particles,
        constraint_setups: vec![standard_link],
        ..Default::default()
    };

    ClothSetupObject {
        sim_cloth_setups: vec![scd],
        ..Default::default()
    }
}

fn build_selection_setup(selection_kind: u8) -> ClothSetupObject {
    let positions = vec![
        [0.0, 0.0, 0.0, 1.0],
        [1.0, 0.0, 0.0, 1.0],
        [0.0, 1.0, 0.0, 1.0],
        [1.0, 1.0, 0.0, 1.0],
    ];
    let triangles = vec![[0, 1, 2], [1, 3, 2]];
    let mut vertex_selection_channels = std::collections::HashMap::new();
    vertex_selection_channels.insert("fixed".to_string(), vec![1, 3]);
    let mesh = SimulationSetupMesh {
        positions,
        triangles,
        vertex_selection_channels,
        ..Default::default()
    };
    let scd = SimClothSetupObject {
        simulation_mesh: Some(mesh),
        fixed_particles: VertexSelectionInput {
            kind: selection_kind,
            channel_name: "fixed".to_string(),
        },
        constraint_setups: vec![ConstraintSetupObject::StandardLink(
            StandardLinkSetup::default(),
        )],
        ..Default::default()
    };
    ClothSetupObject {
        sim_cloth_setups: vec![scd],
        ..Default::default()
    }
}

fn baked_fixed_indices(setup: ClothSetupObject) -> Vec<u16> {
    let setup_json = serde_json::to_string(&setup).unwrap();
    let blob = havok_native::api::cloth_bake(&setup_json).expect("bake cloth");
    let hkx = havok_native::hkx::read_packfile(&blob).expect("parse baked cloth");
    let sim_cloth = hkx
        .objects()
        .iter()
        .find(|object| object.class_name == "hclSimClothData")
        .expect("hclSimClothData object");
    let fixed_particles = sim_cloth
        .members
        .iter()
        .find(|member| member.name == "fixedParticles")
        .expect("fixedParticles member");
    let HkxValue::Array(values) = &fixed_particles.value else {
        panic!("fixedParticles should be an array");
    };
    values
        .iter()
        .map(|value| match value {
            HkxValue::U16(value) => *value,
            other => panic!("fixed particle should be U16, got {other:?}"),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Test: cloth_simulate with a setup_json round-trips a 4×4 grid.
//
// After 30 steps under gravity the bottom row (row 3, indices 12-15) should
// have dropped (z < 0), while the two pinned corners (0 and 3) should stay
// near their original z=0.
// ---------------------------------------------------------------------------
#[test]
fn cloth_simulate_setup_round_trips_grid() {
    let setup = build_simple_grid_setup();
    let setup_json = serde_json::to_string(&setup).unwrap();

    let out = havok_native::api::cloth_simulate(&setup_json, 30, None).unwrap();
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let positions = v["positions"].as_array().expect("positions array");

    assert_eq!(positions.len(), 16, "expected 16 particles in 4×4 grid");

    // Pinned top corners must stay near z=0.
    let z_pin_0 = positions[0][2].as_f64().unwrap();
    let z_pin_3 = positions[3][2].as_f64().unwrap();
    assert!(
        z_pin_0.abs() < 0.5,
        "pinned corner 0 should not move much: z = {z_pin_0}"
    );
    assert!(
        z_pin_3.abs() < 0.5,
        "pinned corner 3 should not move much: z = {z_pin_3}"
    );

    // Top-left pinned corner z is our reference.
    let p_top_left: Vec<f64> = positions[0]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_f64().unwrap())
        .collect();
    let p_bottom_left: Vec<f64> = positions[12]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_f64().unwrap())
        .collect();

    assert!(
        p_top_left[2] > p_bottom_left[2],
        "gravity should pull bottom row below top row: top_z={} bottom_z={}",
        p_top_left[2],
        p_bottom_left[2]
    );
}

#[test]
fn cloth_bake_treats_vertex_selection_channel_as_indices() {
    assert_eq!(baked_fixed_indices(build_selection_setup(2)), vec![1, 3]);
}

#[test]
fn cloth_bake_inverts_vertex_selection_channel_indices() {
    assert_eq!(baked_fixed_indices(build_selection_setup(3)), vec![0, 2]);
}
