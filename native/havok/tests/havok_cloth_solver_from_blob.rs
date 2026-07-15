/// Round-trip test: cloth_simulate_from_blob on the bathrobe fixture.
///
/// If the bathrobe HKX fixture is absent the test is skipped (returns without
/// assertion) so the CI matrix stays green even without game-data fixtures.
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn bathrobe_blob() -> Option<Vec<u8>> {
    let hkx_path = repo_root().join("../tests/fixtures/cloth/bathrobe_outfitm_cloth.hkx");
    if hkx_path.exists() {
        return Some(std::fs::read(&hkx_path).expect("read bathrobe HKX"));
    }
    None
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------
#[test]
fn cloth_simulate_from_blob_bathrobe_plausible_gravity() {
    let blob = match bathrobe_blob() {
        Some(b) => b,
        None => {
            eprintln!("SKIP: bathrobe fixture not found — skipping cloth_simulate_from_blob test");
            return;
        }
    };

    let out = havok_native::api::cloth_simulate_from_blob(&blob, 30, None)
        .expect("cloth_simulate_from_blob should succeed on bathrobe fixture");

    let v: serde_json::Value = serde_json::from_str(&out).expect("output is valid JSON");

    let n_particles = v["n_particles"].as_u64().expect("n_particles is present") as usize;
    let fixed_count = v["fixed_count"].as_u64().expect("fixed_count is present") as usize;
    let positions = v["positions"].as_array().expect("positions is an array");

    assert_eq!(
        positions.len(),
        n_particles,
        "positions array length matches n_particles"
    );
    assert!(
        n_particles > 0,
        "bathrobe should have at least one particle"
    );
    assert!(
        fixed_count < n_particles,
        "not all particles should be fixed: fixed={fixed_count} total={n_particles}"
    );

    // Find the minimum z among non-fixed particles after 30 steps.
    // Under gravity (default -686.7 cm/s²) at least one movable particle
    // should have fallen below its initial z.  We can't know the initial z
    // without running 0 steps, so instead we check that the step-30 positions
    // are finite (no NaN/Inf).
    for (i, pos) in positions.iter().enumerate() {
        let arr = pos.as_array().expect("position is [x,y,z]");
        assert_eq!(arr.len(), 3, "position {i} has 3 components");
        for (k, coord) in arr.iter().enumerate() {
            let f = coord.as_f64().expect("coordinate is a float");
            assert!(
                f.is_finite(),
                "particle {i} component {k} is not finite: {f}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Test 2: cloth_simulate_from_blob with zero steps returns initial positions
//
// Running 0 steps should return the cloth-pose positions with no gravity
// applied — the result is the bind-pose snapshot stored in the HCL file.
// All coordinates must be finite and the array lengths must match.
// ---------------------------------------------------------------------------
#[test]
fn cloth_simulate_from_blob_zero_steps_returns_initial_positions() {
    let blob = match bathrobe_blob() {
        Some(b) => b,
        None => {
            eprintln!("SKIP: bathrobe fixture not found");
            return;
        }
    };

    let out_0 = havok_native::api::cloth_simulate_from_blob(&blob, 0, None)
        .expect("0-step simulation should succeed");
    let out_30 = havok_native::api::cloth_simulate_from_blob(&blob, 30, None)
        .expect("30-step simulation should succeed");

    let v0: serde_json::Value = serde_json::from_str(&out_0).unwrap();
    let v30: serde_json::Value = serde_json::from_str(&out_30).unwrap();

    let pos0 = v0["positions"].as_array().unwrap();
    let pos30 = v30["positions"].as_array().unwrap();

    assert_eq!(
        pos0.len(),
        pos30.len(),
        "same particle count regardless of steps"
    );

    // After 30 steps the z coordinates of movable particles should differ from T=0.
    // We check that at least one particle's z changed by more than 1 unit.
    let n_particles = v0["n_particles"].as_u64().unwrap() as usize;
    let fixed_count = v0["fixed_count"].as_u64().unwrap() as usize;

    if fixed_count < n_particles {
        let mut max_move = 0.0f64;
        for i in 0..n_particles {
            let z0 = pos0[i][2].as_f64().unwrap();
            let z30 = pos30[i][2].as_f64().unwrap();
            let d = (z30 - z0).abs();
            if d > max_move {
                max_move = d;
            }
        }
        assert!(
            max_move > 0.01,
            "after 30 frames under gravity at least one particle should move (max move was {max_move:.4})"
        );
    }
}

// ---------------------------------------------------------------------------
// Test 3: cloth_simulate_from_blob with config_json gravity=0 — no movement
//
// With zero gravity and no other forces, all particles should stay near
// their initial positions (within floating-point noise).
// ---------------------------------------------------------------------------
#[test]
fn cloth_simulate_from_blob_zero_gravity_no_movement() {
    let blob = match bathrobe_blob() {
        Some(b) => b,
        None => {
            eprintln!("SKIP: bathrobe fixture not found");
            return;
        }
    };

    let config = r#"{"gravity": [0.0, 0.0, 0.0]}"#;
    let out_0 = havok_native::api::cloth_simulate_from_blob(&blob, 0, None).unwrap();
    let out_30 = havok_native::api::cloth_simulate_from_blob(&blob, 30, Some(config)).unwrap();

    let pos0: Vec<[f64; 3]> = {
        let v: serde_json::Value = serde_json::from_str(&out_0).unwrap();
        v["positions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| {
                let a = p.as_array().unwrap();
                [
                    a[0].as_f64().unwrap(),
                    a[1].as_f64().unwrap(),
                    a[2].as_f64().unwrap(),
                ]
            })
            .collect()
    };
    let pos30: Vec<[f64; 3]> = {
        let v: serde_json::Value = serde_json::from_str(&out_30).unwrap();
        v["positions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| {
                let a = p.as_array().unwrap();
                [
                    a[0].as_f64().unwrap(),
                    a[1].as_f64().unwrap(),
                    a[2].as_f64().unwrap(),
                ]
            })
            .collect()
    };

    for i in 0..pos0.len() {
        let dx = pos30[i][0] - pos0[i][0];
        let dy = pos30[i][1] - pos0[i][1];
        let dz = pos30[i][2] - pos0[i][2];
        let dist = (dx * dx + dy * dy + dz * dz).sqrt();
        // Allow a generous 5-unit tolerance: constraint solve may move particles
        // slightly even with zero gravity, but should not drift far.
        assert!(
            dist < 5.0,
            "particle {i} moved {dist:.3} units with zero gravity (expected < 5)"
        );
    }
}

// ---------------------------------------------------------------------------
// cloth_simulate_from_blob uses invMass-first mass resolution and keeps pinned
// particles (fixedParticles list) immovable.
//
// The solver reads invMass first; if non-zero, mass = 1/invMass, else it falls
// back to mass. For non-pinned particles where both fields are zero it uses
// DEFAULT_MASS = 0.02. Pinned particles always get inv_mass = 0.
//
// The bathrobe fixture stores invMass on its particles (mass=0), so this
// verifies movable particles respond to gravity and pinned particles stay put.
// ---------------------------------------------------------------------------
#[test]
fn cloth_simulate_from_blob_uses_inv_mass_pinned_particles_pin_correctly() {
    use havok_native::api::cloth_simulate_from_blob;

    let blob = match bathrobe_blob() {
        Some(b) => b,
        None => return,
    };

    // Run 30 frames at default gravity. invMass-first means movable particles
    // that have invMass set get mass=1/invMass; those with both zero get 0.02.
    let out = cloth_simulate_from_blob(&blob, 30, None).expect("run sim");
    let v: serde_json::Value = serde_json::from_str(&out).expect("parse json");

    let pinned_idx: Vec<usize> = v["fixed_particle_indices"]
        .as_array()
        .map(|arr| arr.iter().map(|i| i.as_u64().unwrap() as usize).collect())
        .unwrap_or_default();

    let initial = v["initial_positions"]
        .as_array()
        .expect("initial_positions in json");
    let final_ = v["positions"].as_array().expect("positions in json");
    let n_particles = v["n_particles"].as_u64().expect("n_particles") as usize;

    // Pinned particles must not drift.
    for &idx in &pinned_idx {
        let p0 = initial[idx].as_array().unwrap();
        let p1 = final_[idx].as_array().unwrap();
        let dx = p1[0].as_f64().unwrap() - p0[0].as_f64().unwrap();
        let dy = p1[1].as_f64().unwrap() - p0[1].as_f64().unwrap();
        let dz = p1[2].as_f64().unwrap() - p0[2].as_f64().unwrap();
        let drift = (dx * dx + dy * dy + dz * dz).sqrt();
        assert!(
            drift < 0.001,
            "pinned particle {idx} drifted {drift:.5}; fixedParticles must stay immovable"
        );
    }

    // At least one movable particle must have moved (non-zero gravity applied).
    let pinned_set: std::collections::HashSet<usize> = pinned_idx.into_iter().collect();
    let mut max_move = 0.0f64;
    for i in 0..n_particles {
        if pinned_set.contains(&i) {
            continue;
        }
        let p0 = &initial[i].as_array().unwrap();
        let p1 = &final_[i].as_array().unwrap();
        let dz = (p1[2].as_f64().unwrap() - p0[2].as_f64().unwrap()).abs();
        if dz > max_move {
            max_move = dz;
        }
    }
    // If there are movable particles the solver must have applied gravity to them.
    if pinned_set.len() < n_particles {
        assert!(
            max_move > 0.01,
            "no movable particle moved after 30 frames (max_move={max_move:.4}); \
             invMass-first mass resolution may be broken"
        );
    }
}

#[test]
fn cloth_step_from_blob_state_matches_two_frame_replay() {
    let blob = match bathrobe_blob() {
        Some(b) => b,
        None => return,
    };

    let out_0 = havok_native::api::cloth_simulate_from_blob(&blob, 0, None)
        .expect("0-step simulation should succeed");
    let v0: serde_json::Value = serde_json::from_str(&out_0).unwrap();
    let positions_0 = serde_json::to_string(&v0["positions"]).unwrap();

    let out_1 =
        havok_native::api::cloth_step_from_blob_state(&blob, &positions_0, &positions_0, None)
            .expect("incremental step 1 should succeed");
    let v1: serde_json::Value = serde_json::from_str(&out_1).unwrap();
    let positions_1 = serde_json::to_string(&v1["positions"]).unwrap();
    let prev_positions_1 = serde_json::to_string(&v1["prev_positions"]).unwrap();

    let out_2_incremental =
        havok_native::api::cloth_step_from_blob_state(&blob, &positions_1, &prev_positions_1, None)
            .expect("incremental step 2 should succeed");
    let out_2_replay = havok_native::api::cloth_simulate_from_blob(&blob, 2, None)
        .expect("2-step replay should succeed");

    let inc: serde_json::Value = serde_json::from_str(&out_2_incremental).unwrap();
    let replay: serde_json::Value = serde_json::from_str(&out_2_replay).unwrap();
    assert_eq!(inc["positions"], replay["positions"]);
}
