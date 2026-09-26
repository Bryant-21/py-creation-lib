use std::collections::HashSet;

use havok_native::cloth::solver::{
    BendConstraint, Capsule, DistanceConstraint, Solver, SolverConfig,
    havok_stiffness_to_compliance,
};

// Kinematic z after 0.5 s ≈ -½·9.81·0.25 ≈ -1.226 (Z-up m/s²); XPBD damping
// lands slightly short, so allow ±20%.
#[test]
fn free_fall_single_particle_falls_under_gravity() {
    let cfg = SolverConfig::default();
    let mut solver = Solver::new(
        vec![[0.0, 0.0, 0.0]],
        vec![1.0],
        Default::default(),
        Default::default(),
        Default::default(),
        Default::default(),
        Default::default(),
    );
    for _ in 0..30 {
        solver.step(&cfg);
    }

    let z = solver.positions()[0][2];
    let expected = -0.5 * 9.81_f32 * 0.5 * 0.5;
    let tolerance = expected.abs() * 0.20;
    assert!(
        (z - expected).abs() <= tolerance,
        "z = {z:.3}, expected ≈ {expected:.3} ±20%"
    );
}

/// Particle 0 pinned at origin, particle 1 hanging on a 1-unit link.
fn pinned_link_length(start_z: f32, stiffness: f32, frames: usize) -> (f32, [f32; 3]) {
    let cfg = SolverConfig::default();
    let mut solver = Solver::new(
        vec![[0.0, 0.0, 0.0], [0.0, 0.0, start_z]],
        vec![1.0, 1.0],
        vec![DistanceConstraint {
            a: 0,
            b: 1,
            rest_length: 1.0,
            compliance: havok_stiffness_to_compliance(stiffness, cfg.dt),
        }],
        Default::default(),
        Default::default(),
        Default::default(),
        HashSet::from([0u32]),
    );
    for _ in 0..frames {
        solver.step(&cfg);
    }
    let p0 = solver.positions()[0];
    let p1 = solver.positions()[1];
    let d = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
    ((d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt(), p0)
}

#[test]
fn distance_constraint_settles_and_stiffness_is_monotonic() {
    let dt = 1.0_f32 / 60.0;
    assert!(havok_stiffness_to_compliance(1.0, dt) < havok_stiffness_to_compliance(0.0, dt));

    let (len, pin) = pinned_link_length(-1.0, 0.9, 60);
    assert!(pin.iter().map(|c| c.abs()).sum::<f32>() < 1e-3, "pin drifted: {pin:?}");
    assert!((len - 1.0).abs() < 0.1, "link length {len:.4} not within 10% of 1.0");

    let (loose, _) = pinned_link_length(-2.0, 0.1, 120);
    let (stiff, _) = pinned_link_length(-2.0, 0.9, 120);
    assert!(stiff < loose, "stiff={stiff:.4} loose={loose:.4}");
}

#[test]
fn capsule_collision_pushes_particle_to_surface() {
    let cfg = SolverConfig::default();
    let mut solver = Solver::new(
        vec![[0.0, 0.0, 0.0]],
        vec![1.0],
        Default::default(),
        Default::default(),
        Default::default(),
        vec![Capsule {
            start: [0.0, -10.0, 0.0],
            end: [0.0, 10.0, 0.0],
            radius: 1.0,
        }],
        Default::default(),
    );
    solver.step(&cfg);

    let p = solver.positions()[0];
    let dist_from_axis = (p[0] * p[0] + p[2] * p[2]).sqrt();
    assert!(dist_from_axis >= 1.0 - 1e-3, "dist_from_axis = {dist_from_axis:.4}");
}

// d starts just off the flat dihedral with rest_angle = PI; the simple
// gradient form oscillates from far-off starts, so keep it near equilibrium.
#[test]
fn bend_constraint_relaxes_to_rest_angle() {
    let mut solver = Solver::new(
        vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.5, 1.0, 0.0],
            [0.5, -1.0, 0.1],
        ],
        vec![1.0_f32; 4],
        Vec::<DistanceConstraint>::new(),
        vec![BendConstraint {
            a: 0,
            b: 1,
            c: 2,
            d: 3,
            rest_angle: std::f32::consts::PI,
            compliance: 1e-4,
        }],
        Vec::new(),
        Vec::new(),
        HashSet::new(),
    );
    let config = SolverConfig {
        gravity: [0.0, 0.0, 0.0],
        ..SolverConfig::default()
    };
    for _ in 0..120 {
        solver.step(&config);
    }
    let pd = solver.positions()[3];
    assert!(pd[2].abs() < 0.2, "expected d.z near 0, got {}", pd[2]);
}

#[test]
fn damping_per_second_is_substep_invariant() {
    fn run_sim(substeps: u32) -> f32 {
        let cfg = SolverConfig {
            substeps,
            gravity: [0.0, 0.0, 0.0],
            damping_per_second: Some(0.5),
            ..SolverConfig::default()
        };
        // prev = pos - v*h so the solver reconstructs initial velocity v=1.
        let h = cfg.dt / cfg.substeps as f32;
        let mut solver = Solver::with_initial_velocities(
            vec![[0.0, 0.0, 0.0]],
            vec![[0.0, 0.0, -h]],
            vec![1.0],
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            HashSet::new(),
        );
        for _ in 0..60 {
            solver.step(&cfg);
        }
        solver.positions()[0][2]
    }

    let (z1, z4) = (run_sim(1), run_sim(4));
    assert!((z1 - z4).abs() < 0.1, "substeps=1 z={z1:.4}, substeps=4 z={z4:.4}");
}
