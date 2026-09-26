use havok_native::api::{cloth_simulate_from_blob, cloth_step_from_blob_state};
use std::collections::HashSet;
use std::path::PathBuf;

fn bathrobe_blob() -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/cloth/bathrobe_outfitm_cloth.hkx");
    std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn simulate(blob: &[u8], steps: u32, config: Option<&str>) -> serde_json::Value {
    let out = cloth_simulate_from_blob(blob, steps, config)
        .unwrap_or_else(|e| panic!("{steps}-step simulation failed: {e}"));
    serde_json::from_str(&out).expect("output is valid JSON")
}

fn points(v: &serde_json::Value) -> Vec<[f64; 3]> {
    v.as_array()
        .expect("position array")
        .iter()
        .map(|p| {
            let a = p.as_array().expect("position is [x,y,z]");
            assert_eq!(a.len(), 3);
            [0, 1, 2].map(|k| {
                let f = a[k].as_f64().expect("coordinate is a float");
                assert!(f.is_finite(), "non-finite coordinate {f}");
                f
            })
        })
        .collect()
}

fn dist(a: [f64; 3], b: [f64; 3]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

#[test]
fn bathrobe_gravity_moves_free_particles_and_keeps_pins() {
    let blob = bathrobe_blob();

    let v0 = simulate(&blob, 0, None);
    let v30 = simulate(&blob, 30, None);
    let n_particles = v30["n_particles"].as_u64().expect("n_particles") as usize;
    let fixed_count = v30["fixed_count"].as_u64().expect("fixed_count") as usize;
    assert!(n_particles > 0);
    assert!(fixed_count < n_particles, "fixed={fixed_count} total={n_particles}");

    let pos0 = points(&v0["positions"]);
    let initial = points(&v30["initial_positions"]);
    let pos30 = points(&v30["positions"]);
    assert_eq!(pos0.len(), n_particles);
    assert_eq!(pos30.len(), n_particles);

    let pinned: HashSet<usize> = v30["fixed_particle_indices"]
        .as_array()
        .expect("fixed_particle_indices")
        .iter()
        .map(|i| i.as_u64().unwrap() as usize)
        .collect();
    let mut max_free_dz = 0.0f64;
    for i in 0..n_particles {
        if pinned.contains(&i) {
            let drift = dist(initial[i], pos30[i]);
            assert!(drift < 0.001, "pinned particle {i} drifted {drift:.5}");
        } else {
            max_free_dz = max_free_dz.max((pos30[i][2] - initial[i][2]).abs());
        }
    }
    assert!(max_free_dz > 0.01, "no free particle moved (max dz {max_free_dz:.4})");

    let still = points(&simulate(&blob, 30, Some(r#"{"gravity": [0.0, 0.0, 0.0]}"#))["positions"]);
    for i in 0..n_particles {
        let d = dist(pos0[i], still[i]);
        assert!(d < 5.0, "particle {i} moved {d:.3} with zero gravity");
    }
}

#[test]
fn cloth_step_from_blob_state_matches_two_frame_replay() {
    let blob = bathrobe_blob();

    let positions_0 = serde_json::to_string(&simulate(&blob, 0, None)["positions"]).unwrap();
    let out_1 = cloth_step_from_blob_state(&blob, &positions_0, &positions_0, None)
        .expect("incremental step 1 should succeed");
    let v1: serde_json::Value = serde_json::from_str(&out_1).unwrap();
    let positions_1 = serde_json::to_string(&v1["positions"]).unwrap();
    let prev_positions_1 = serde_json::to_string(&v1["prev_positions"]).unwrap();

    let out_2 = cloth_step_from_blob_state(&blob, &positions_1, &prev_positions_1, None)
        .expect("incremental step 2 should succeed");
    let inc: serde_json::Value = serde_json::from_str(&out_2).unwrap();
    assert_eq!(inc["positions"], simulate(&blob, 2, None)["positions"]);
}
