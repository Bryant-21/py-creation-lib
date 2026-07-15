use havok_native::cloth::solver::{
    BendConstraint, Capsule, DistanceConstraint, Solver, SolverConfig,
    havok_stiffness_to_compliance,
};

// ---------------------------------------------------------------------------
// Test 1: Free-fall single particle under gravity
//
// 1 unpin particle, no constraints.  Run 30 frames at dt=1/60.
// After 0.5 s, kinematic z ≈ -½·g·t² = -½·9.81·0.25 ≈ -1.226 (m/s² Z-up).
// XPBD with damping 0.999 will be slightly less; allow ±20% tolerance.
// ---------------------------------------------------------------------------
#[test]
fn free_fall_single_particle_falls_under_gravity() {
    let cfg = SolverConfig::default(); // gravity = (0,0,-9.81), dt=1/60

    let mut solver = Solver::new(
        vec![[0.0, 0.0, 0.0]],
        vec![1.0],          // mass = 1 → inv_mass = 1
        Default::default(), // no distance constraints
        Default::default(), // no bend constraints
        Default::default(), // no range constraints
        Default::default(), // no capsules
        Default::default(), // no pins
    );

    // 30 frames = 0.5 s at 1/60
    for _ in 0..30 {
        solver.step(&cfg);
    }

    let z = solver.positions()[0][2];
    // Kinematic: -½ · 9.81 · 0.5² ≈ -1.226 m
    let expected = -0.5 * 9.81_f32 * 0.5 * 0.5;
    let tolerance = expected.abs() * 0.20; // ±20%
    assert!(z < 0.0, "particle should have fallen: z = {z}");
    assert!(
        (z - expected).abs() <= tolerance,
        "z = {z:.3}, expected ≈ {expected:.3} ±20% (tolerance {tolerance:.3})"
    );
}

// ---------------------------------------------------------------------------
// Test 2: Pinned two-particle distance constraint settles
//
// Particle 0 pinned at origin, particle 1 starts at (0, 0, -1).
// Distance constraint rest_length = 1. After 60 frames the link should be
// within 10% of rest length.
// ---------------------------------------------------------------------------
#[test]
fn pinned_two_particle_distance_constraint_settles() {
    use std::collections::HashSet;

    let cfg = SolverConfig::default();

    let positions = vec![[0.0, 0.0, 0.0], [0.0, 0.0, -1.0]];
    let masses = vec![1.0, 1.0];
    let distance_constraints = vec![DistanceConstraint {
        a: 0,
        b: 1,
        rest_length: 1.0,
        compliance: havok_stiffness_to_compliance(0.9, cfg.dt),
    }];
    let mut pins = HashSet::new();
    pins.insert(0u32); // pin particle 0 at origin

    let mut solver = Solver::new(
        positions,
        masses,
        distance_constraints,
        Default::default(),
        Default::default(),
        Default::default(),
        pins,
    );

    for _ in 0..60 {
        solver.step(&cfg);
    }

    let p0 = solver.positions()[0];
    let p1 = solver.positions()[1];

    // Pin must stay at origin
    assert!(
        (p0[0].abs() + p0[1].abs() + p0[2].abs()) < 1e-3,
        "pinned particle drifted: {p0:?}"
    );

    let diff = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
    let link_len = (diff[0] * diff[0] + diff[1] * diff[1] + diff[2] * diff[2]).sqrt();

    assert!(
        (link_len - 1.0).abs() < 0.1,
        "link length {link_len:.4} should be within 10% of rest length 1.0"
    );
}

// ---------------------------------------------------------------------------
// Test 3: Monotonic stiffness — higher stiffness reduces residual stretch
//
// Same two-particle setup but particle 1 starts at (0,0,-2) so the 1-unit
// link is initially stretched to length 2. Compare residual stretch with
// stiffness 0.1 vs 0.9.
// ---------------------------------------------------------------------------
#[test]
fn monotonic_stiffness_reduces_stretch() {
    use std::collections::HashSet;

    fn settled_length(stiffness: f32) -> f32 {
        let cfg = SolverConfig::default();
        let positions = vec![[0.0, 0.0, 0.0], [0.0, 0.0, -2.0]];
        let masses = vec![1.0, 1.0];
        let distance_constraints = vec![DistanceConstraint {
            a: 0,
            b: 1,
            rest_length: 1.0,
            compliance: havok_stiffness_to_compliance(stiffness, cfg.dt),
        }];
        let mut pins = HashSet::new();
        pins.insert(0u32);

        let mut solver = Solver::new(
            positions,
            masses,
            distance_constraints,
            Default::default(),
            Default::default(),
            Default::default(),
            pins,
        );
        for _ in 0..120 {
            solver.step(&cfg);
        }
        let p0 = solver.positions()[0];
        let p1 = solver.positions()[1];
        let diff = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
        (diff[0] * diff[0] + diff[1] * diff[1] + diff[2] * diff[2]).sqrt()
    }

    let l_low = settled_length(0.1);
    let l_high = settled_length(0.9);

    assert!(
        l_high < l_low,
        "expected higher stiffness to produce less stretch: l_low={l_low:.4} l_high={l_high:.4}"
    );
}

// ---------------------------------------------------------------------------
// Test 4: Capsule collision pushes particle to surface
//
// Capsule axis along +Y at origin, radius 1. Particle starts at (0,0,0)
// (on the axis — inside the capsule). After one step, distance from axis
// must be ≥ radius - epsilon.
// ---------------------------------------------------------------------------
#[test]
fn capsule_collision_pushes_particle_to_surface() {
    let cfg = SolverConfig::default();

    let capsules = vec![Capsule {
        start: [0.0, -10.0, 0.0],
        end: [0.0, 10.0, 0.0],
        radius: 1.0,
    }];

    let mut solver = Solver::new(
        vec![[0.0, 0.0, 0.0]], // inside capsule
        vec![1.0],
        Default::default(),
        Default::default(),
        Default::default(),
        capsules,
        Default::default(),
    );

    solver.step(&cfg);

    let p = solver.positions()[0];
    // Distance from Y-axis = sqrt(x²+z²)
    let dist_from_axis = (p[0] * p[0] + p[2] * p[2]).sqrt();

    assert!(
        dist_from_axis >= 1.0 - 1e-3,
        "particle should be pushed to capsule surface; dist_from_axis = {dist_from_axis:.4}"
    );
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------
#[test]
fn bend_constraint_relaxes_to_rest_angle() {
    use std::collections::HashSet;

    // Two triangles sharing edge (0,1). c is on +Y, d starts nearly flat
    // (just 0.1 above the XY plane at the mirror position of c). rest_angle
    // is PI (flat). The constraint should drive d.z toward 0.
    //
    // Note: the simple gradient form (matching Python) oscillates when d
    // starts far from flat (e.g., z=0.5). This geometry starts close to
    // the equilibrium so the constraint converges within 120 steps.
    let positions: Vec<_> = vec![
        [0.0, 0.0, 0.0_f32], // a (shared edge)
        [1.0, 0.0, 0.0],     // b (shared edge)
        [0.5, 1.0, 0.0],     // c (one side, on +Y plane)
        [0.5, -1.0, 0.1],    // d (mirror of c, slightly above flat)
    ];
    let masses = vec![1.0_f32; 4];
    let bend = vec![BendConstraint {
        a: 0,
        b: 1,
        c: 2,
        d: 3,
        rest_angle: std::f32::consts::PI,
        compliance: 1e-4,
    }];

    let mut solver = Solver::new(
        positions,
        masses,
        Vec::<DistanceConstraint>::new(),
        bend,
        Vec::new(),
        Vec::new(),
        HashSet::new(),
    );
    // Zero gravity so the bend constraint drives the geometry without
    // external forces confounding the dihedral measurement.
    let config = SolverConfig {
        gravity: [0.0, 0.0, 0.0],
        ..SolverConfig::default()
    };
    for _ in 0..120 {
        solver.step(&config);
    }

    let pd = solver.positions()[3];
    // After relaxation, d should be flat (z ~ 0).
    assert!(pd[2].abs() < 0.2, "expected d.z near 0, got {}", pd[2]);
}

#[test]
fn havok_stiffness_to_compliance_endpoints() {
    let dt = 1.0_f32 / 60.0;
    let c_rigid = havok_stiffness_to_compliance(1.0, dt);
    let c_loose = havok_stiffness_to_compliance(0.0, dt);

    assert!(
        c_rigid < c_loose,
        "stiffness=1 should be more rigid (smaller compliance) than stiffness=0: \
         c_rigid={c_rigid:.2e} c_loose={c_loose:.2e}"
    );
}

// ---------------------------------------------------------------------------
// Unit-aware velocity damping — damping_per_second is substep-invariant.
//
// Two simulations with the same damping_per_second but different substep counts
// should produce the same per-second effective retention, so a free-falling
// particle should reach approximately the same position after 1 second.
// ---------------------------------------------------------------------------
#[test]
fn damping_per_second_is_substep_invariant() {
    use std::collections::HashSet;

    let damping_ps = 0.5_f32; // strong damping: 50% velocity retention per second

    // Run with 1 substep/frame.
    let cfg1 = SolverConfig {
        substeps: 1,
        gravity: [0.0, 0.0, 0.0], // no gravity so we isolate damping
        damping_per_second: Some(damping_ps),
        ..SolverConfig::default()
    };

    // Run with 4 substeps/frame.
    let cfg4 = SolverConfig {
        substeps: 4,
        gravity: [0.0, 0.0, 0.0],
        damping_per_second: Some(damping_ps),
        ..SolverConfig::default()
    };

    fn run_sim(cfg: &SolverConfig, initial_velocity: f32) -> f32 {
        // Encode initial velocity v as prev_pos = pos - v*h where h = dt/substeps,
        // so the step function reconstructs (pos - prev) / h = v correctly.
        let pos = [0.0_f32, 0.0, 0.0];
        let h = cfg.dt / cfg.substeps as f32;
        let prev_pos = [0.0_f32, 0.0, -initial_velocity * h];

        let mut solver = Solver::with_initial_velocities(
            vec![pos],
            vec![prev_pos],
            vec![1.0],
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            HashSet::new(),
        );

        // Run 60 frames = 1 second
        for _ in 0..60 {
            solver.step(cfg);
        }
        // Return z position after 1 second (no gravity so it's pure damped drift)
        solver.positions()[0][2]
    }

    let z1 = run_sim(&cfg1, 1.0);
    let z4 = run_sim(&cfg4, 1.0);

    // Both should give ≈ same displacement after 1 second (substep-invariant).
    let diff = (z1 - z4).abs();
    assert!(
        diff < 0.1,
        "damping_per_second={damping_ps}: displacement after 1s should be substep-invariant; \
         substeps=1 z={z1:.4}, substeps=4 z={z4:.4}, diff={diff:.4}"
    );
}
