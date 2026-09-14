//! XPBD preview solver for FO4 cloth parameter tuning: distance, bend and range
//! constraints plus capsule collision.
//!
//! Editor preview only. The Havok runtime uses PGS constraint resolution,
//! continuous collision and material parameters with no XPBD equivalent, so the
//! goal is output that responds monotonically to parameter changes, not parity.
//!
//! Gravity is m/s², Z-up (`cloth::units::GRAVITY_Z`). Havok stiffness [0, 1] maps
//! to compliance `(1 - clamp(stiffness, 0, 0.9999)) * 1e-3 + 1e-6`: stiffness 1.0
//! gives ~1e-6 (stiff), 0.0 gives ~1e-3 (loose).

use std::collections::HashSet;

// ---------------------------------------------------------------------------
// Vec3 arithmetic helpers — hand-rolled f32, no external crates
// ---------------------------------------------------------------------------

pub(crate) type Vec3 = [f32; 3];

#[inline]
fn add(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

#[inline]
fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

#[inline]
fn scale(a: Vec3, s: f32) -> Vec3 {
    [a[0] * s, a[1] * s, a[2] * s]
}

#[inline]
fn dot(a: Vec3, b: Vec3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[inline]
fn length(a: Vec3) -> f32 {
    dot(a, a).sqrt()
}

#[inline]
fn normalize(a: Vec3) -> Option<Vec3> {
    let len = length(a);
    if len < 1e-8 {
        None
    } else {
        Some(scale(a, 1.0 / len))
    }
}

#[allow(dead_code)]
#[inline]
fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Solver configuration mirroring Python's `SolverConfig` dataclass.
#[derive(Clone, Debug)]
pub struct SolverConfig {
    /// Timestep in seconds.
    pub dt: f32,
    /// Number of substeps per frame.
    pub substeps: u32,
    /// Constraint projection iterations per substep.
    pub constraint_iterations: u32,
    /// Gravity vector. Canonical units: **m/s² along world Z** (Z-up).
    /// Matches `cloth::units::GRAVITY_Z` and the SDK convention; see
    /// `cloth::units` for the rationale.
    pub gravity: Vec3,
    /// Wind force vector.
    pub wind: Vec3,
    /// Velocity damping factor applied once per substep.
    ///
    /// When `damping_per_second` is `Some(r)`, this field is ignored and the
    /// per-substep factor is derived as `r.powf(substep_dt)` so the effective
    /// retention is substep-count–invariant.
    pub damping: f32,
    /// Optional velocity retention per second. When set, the per-substep factor
    /// is `damping_per_second.powf(dt / substeps)`, independent of substep count.
    pub damping_per_second: Option<f32>,
    /// Capsule collision push-out margin.
    pub collision_epsilon: f32,
}

impl Default for SolverConfig {
    fn default() -> Self {
        Self {
            dt: 1.0 / 60.0,
            substeps: 2,
            constraint_iterations: 12,
            gravity: [0.0, 0.0, super::units::GRAVITY_Z],
            wind: [0.0, 0.0, 0.0],
            damping: 0.999,
            damping_per_second: None,
            collision_epsilon: 0.1,
        }
    }
}

/// XPBD distance constraint between two particles.
#[derive(Clone, Debug)]
pub struct DistanceConstraint {
    pub a: u32,
    pub b: u32,
    pub rest_length: f32,
    pub compliance: f32,
}

/// Dihedral-angle bend constraint.
///
/// Particles `a` and `b` share the edge; `c` and `d` are opposite vertices of
/// the two triangles. Solved in `project_bend_constraints`.
#[derive(Clone, Debug)]
pub struct BendConstraint {
    pub a: u32,
    pub b: u32,
    pub c: u32,
    pub d: u32,
    pub rest_angle: f32,
    pub compliance: f32,
}

/// Local-range position clamp constraint.
#[derive(Clone, Debug)]
pub struct RangeConstraint {
    pub particle: u32,
    pub rest_pos: Vec3,
    pub max_range: f32,
}

/// Collision capsule (infinite-length segment capped by hemispherical ends).
#[derive(Clone, Debug)]
pub struct Capsule {
    pub start: Vec3,
    pub end: Vec3,
    pub radius: f32,
}

// ---------------------------------------------------------------------------
// Solver
// ---------------------------------------------------------------------------

/// XPBD cloth solver.
///
/// Construct via `Solver::new`, then call `step` each frame.
/// Read current positions from `positions()`.
pub struct Solver {
    pub positions: Vec<Vec3>,
    pub prev_positions: Vec<Vec3>,
    pub masses: Vec<f32>,
    pub inv_masses: Vec<f32>,
    pub distance_constraints: Vec<DistanceConstraint>,
    pub bend_constraints: Vec<BendConstraint>,
    pub range_constraints: Vec<RangeConstraint>,
    pub capsules: Vec<Capsule>,
    /// Pinned particle indices — these are held at their initial positions.
    pub pins: HashSet<u32>,

    // Per-particle constraint valence for Jacobi damping
    valence: Vec<f32>,
}

impl Solver {
    /// Create a new solver.
    ///
    /// `masses` are interpreted as particle mass (not inverse mass); any
    /// particle listed in `pins` is treated as having infinite mass.
    pub fn new(
        positions: Vec<Vec3>,
        masses: Vec<f32>,
        distance_constraints: Vec<DistanceConstraint>,
        bend_constraints: Vec<BendConstraint>,
        range_constraints: Vec<RangeConstraint>,
        capsules: Vec<Capsule>,
        pins: HashSet<u32>,
    ) -> Self {
        let n = positions.len();
        let inv_masses: Vec<f32> = masses
            .iter()
            .enumerate()
            .map(|(i, &m)| {
                if pins.contains(&(i as u32)) {
                    0.0
                } else if m > 0.0 {
                    1.0 / m
                } else {
                    1.0 // default for zero-mass movable particles
                }
            })
            .collect();

        let valence = Self::compute_valence(n, &distance_constraints);
        let prev_positions = positions.clone();

        Self {
            positions,
            prev_positions,
            masses,
            inv_masses,
            distance_constraints,
            bend_constraints,
            range_constraints,
            capsules,
            pins,
            valence,
        }
    }

    /// Create a solver whose particles start with velocity
    /// `(positions[i] - prev_positions[i]) / dt`.
    #[allow(clippy::too_many_arguments)]
    pub fn with_initial_velocities(
        positions: Vec<Vec3>,
        prev_positions: Vec<Vec3>,
        masses: Vec<f32>,
        distance_constraints: Vec<DistanceConstraint>,
        bend_constraints: Vec<BendConstraint>,
        range_constraints: Vec<RangeConstraint>,
        capsules: Vec<Capsule>,
        pins: HashSet<u32>,
    ) -> Self {
        let n = positions.len();
        let inv_masses: Vec<f32> = masses
            .iter()
            .enumerate()
            .map(|(i, &m)| {
                if pins.contains(&(i as u32)) {
                    0.0
                } else if m > 0.0 {
                    1.0 / m
                } else {
                    1.0
                }
            })
            .collect();
        let valence = Self::compute_valence(n, &distance_constraints);
        Self {
            positions,
            prev_positions,
            masses,
            inv_masses,
            distance_constraints,
            bend_constraints,
            range_constraints,
            capsules,
            pins,
            valence,
        }
    }

    /// Return a slice of current particle positions.
    pub fn positions(&self) -> &[Vec3] {
        &self.positions
    }

    /// Advance one frame (dt) using substeps × constraint_iterations.
    pub fn step(&mut self, config: &SolverConfig) {
        let n = self.positions.len();
        if n == 0 {
            return;
        }

        let h = config.dt / config.substeps as f32;

        // Per-substep velocity retention factor.
        // When damping_per_second is set, derive from the per-second rate so
        // the effective damping is invariant to the number of substeps:
        //   retention_per_substep = retention_per_second ^ substep_dt
        let damp = match config.damping_per_second {
            Some(r) => r.powf(h),
            None => config.damping,
        };

        // Allocate predicted buffer once per frame, reuse across substeps.
        let mut predicted = self.positions.clone();
        // Track per-particle velocity implicitly via (predicted - prev) / h.
        // We carry velocities as (pos - prev_pos) from last substep.
        let mut velocities: Vec<Vec3> = self
            .positions
            .iter()
            .zip(self.prev_positions.iter())
            .map(|(&cur, &prev)| scale(sub(cur, prev), 1.0 / h))
            .collect();

        for _ in 0..config.substeps {
            // 1. Integrate external forces into velocity, then predict position.
            for i in 0..n {
                // Pinned particles don't move.
                if self.inv_masses[i] == 0.0 {
                    predicted[i] = self.positions[i];
                    continue;
                }
                let v = &mut velocities[i];
                v[0] = (v[0] + config.gravity[0] * h + config.wind[0] * h) * damp;
                v[1] = (v[1] + config.gravity[1] * h + config.wind[1] * h) * damp;
                v[2] = (v[2] + config.gravity[2] * h + config.wind[2] * h) * damp;
                predicted[i] = add(self.positions[i], scale(*v, h));
            }

            // 2. Project constraints.
            for _ in 0..config.constraint_iterations {
                self.project_distance_constraints(&mut predicted, h);
                self.project_bend_constraints(&mut predicted, h);
                self.project_range_constraints(&mut predicted);
            }

            // 3. Capsule collision projection.
            for cap_idx in 0..self.capsules.len() {
                let cap = self.capsules[cap_idx].clone();
                Self::project_capsule(
                    &mut predicted,
                    &self.inv_masses,
                    &cap,
                    config.collision_epsilon,
                );
            }

            // 4. Re-pin fixed particles (collision must not displace them).
            for &pin in &self.pins {
                predicted[pin as usize] = self.positions[pin as usize];
            }

            // 5. Recover velocities from position difference.
            for i in 0..n {
                velocities[i] = scale(sub(predicted[i], self.positions[i]), 1.0 / h);
            }

            // Advance position state for next substep.
            std::mem::swap(&mut self.prev_positions, &mut self.positions);
            self.positions.copy_from_slice(&predicted);
        }
    }

    // -----------------------------------------------------------------------
    // Constraint projection
    // -----------------------------------------------------------------------

    fn project_distance_constraints(&self, predicted: &mut Vec<Vec3>, h: f32) {
        if self.distance_constraints.is_empty() {
            return;
        }

        // Accumulate corrections; apply at end (Jacobi style with valence normalization).
        let n = predicted.len();
        let mut corrections: Vec<Vec3> = vec![[0.0; 3]; n];

        for dc in &self.distance_constraints {
            let a = dc.a as usize;
            let b = dc.b as usize;

            let pa = predicted[a];
            let pb = predicted[b];

            let diff = sub(pa, pb);
            let dist = length(diff).max(1e-8);

            let alpha = dc.compliance / (h * h);
            let w_a = self.inv_masses[a];
            let w_b = self.inv_masses[b];
            let w_sum = w_a + w_b;

            if w_sum < 1e-8 {
                continue; // both pinned
            }

            let c = dist - dc.rest_length;
            let delta_lambda = -c / (w_sum + alpha);

            let direction = scale(diff, 1.0 / dist);

            // Per-endpoint valence normalization (Jacobi average stability).
            let inv_val_a = 1.0 / self.valence[a].max(1.0);
            let inv_val_b = 1.0 / self.valence[b].max(1.0);

            let corr_mag_a = delta_lambda * w_a * inv_val_a;
            let corr_mag_b = delta_lambda * w_b * inv_val_b;

            let ca = scale(direction, corr_mag_a);
            let cb = scale(direction, corr_mag_b);

            corrections[a] = add(corrections[a], ca);
            corrections[b] = add(corrections[b], scale(cb, -1.0));
        }

        for i in 0..n {
            predicted[i] = add(predicted[i], corrections[i]);
        }
    }

    fn project_bend_constraints(&self, predicted: &mut Vec<Vec3>, h: f32) {
        for bc in &self.bend_constraints {
            let a = bc.a as usize;
            let b = bc.b as usize;
            let c = bc.c as usize;
            let d = bc.d as usize;

            let pa = predicted[a];
            let pb = predicted[b];
            let pc = predicted[c];
            let pd = predicted[d];

            // Edge vector pa→pb.
            let edge = sub(pb, pa);
            let edge_n = match normalize(edge) {
                Some(n) => n,
                None => continue,
            };

            // Vectors from edge start (pa) to opposite vertices.
            let vc = sub(pc, pa);
            let vd = sub(pd, pa);

            // Project out edge component to get perpendicular components.
            let vc_perp = sub(vc, scale(edge_n, dot(vc, edge_n)));
            let vd_perp = sub(vd, scale(edge_n, dot(vd, edge_n)));

            let vc_perp_unit = match normalize(vc_perp) {
                Some(n) => n,
                None => continue,
            };
            let vd_perp_unit = match normalize(vd_perp) {
                Some(n) => n,
                None => continue,
            };

            let cos_angle = dot(vc_perp_unit, vd_perp_unit).clamp(-1.0, 1.0);
            let angle = cos_angle.acos();

            let c_val = angle - bc.rest_angle;
            if c_val.abs() < 1e-6 {
                continue;
            }

            let alpha = bc.compliance / (h * h);
            let w_c = self.inv_masses[c];
            let w_d = self.inv_masses[d];
            let w_sum = w_c + w_d;
            if w_sum < 1e-8 {
                continue;
            }

            let delta = -c_val / (w_sum + alpha);

            predicted[c] = add(predicted[c], scale(vc_perp_unit, delta * w_c));
            predicted[d] = add(predicted[d], scale(vd_perp_unit, -(delta * w_d)));
        }
    }

    fn project_range_constraints(&self, predicted: &mut Vec<Vec3>) {
        for rc in &self.range_constraints {
            let i = rc.particle as usize;
            if self.inv_masses[i] == 0.0 {
                continue;
            }
            let diff = sub(predicted[i], rc.rest_pos);
            let dist = length(diff);
            if dist > rc.max_range && dist > 1e-8 {
                predicted[i] = add(rc.rest_pos, scale(diff, rc.max_range / dist));
            }
        }
    }

    fn project_capsule(
        predicted: &mut Vec<Vec3>,
        inv_masses: &[f32],
        cap: &Capsule,
        collision_epsilon: f32,
    ) {
        let ab = sub(cap.end, cap.start);
        let ab_len_sq = dot(ab, ab);

        let target_dist = cap.radius + collision_epsilon;

        for i in 0..predicted.len() {
            if inv_masses[i] == 0.0 {
                continue; // pinned — don't move
            }

            let p = predicted[i];

            // Find closest point on the capsule segment.
            let closest = if ab_len_sq < 1e-8 {
                // Degenerate capsule — treat as sphere at start.
                cap.start
            } else {
                let ap = sub(p, cap.start);
                let t = (dot(ap, ab) / ab_len_sq).clamp(0.0, 1.0);
                add(cap.start, scale(ab, t))
            };

            let diff = sub(p, closest);
            let dist = length(diff);

            if dist < target_dist {
                // Particle is inside or too close — push to surface.
                let direction = if dist < 1e-6 {
                    // On the axis — pick perpendicular to capsule axis.
                    Self::capsule_axis_perp(ab, ab_len_sq)
                } else {
                    scale(diff, 1.0 / dist)
                };
                predicted[i] = add(closest, scale(direction, target_dist));
            }
        }
    }

    /// Build a unit vector perpendicular to the capsule axis (used when a
    /// particle is exactly on the axis and diff is zero).
    fn capsule_axis_perp(ab: Vec3, ab_len_sq: f32) -> Vec3 {
        let ab_norm = if ab_len_sq < 1e-8 {
            [0.0, 1.0, 0.0]
        } else {
            scale(ab, 1.0 / ab_len_sq.sqrt())
        };

        // Pick the world axis least aligned with the capsule axis.
        let candidates: [Vec3; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let perp_seed = candidates
            .iter()
            .copied()
            .min_by(|a, b| {
                dot(*a, ab_norm)
                    .abs()
                    .partial_cmp(&dot(*b, ab_norm).abs())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap_or([0.0, 0.0, 1.0]);

        // Gram-Schmidt: subtract component along ab_norm.
        let proj = dot(perp_seed, ab_norm);
        let perp = sub(perp_seed, scale(ab_norm, proj));
        normalize(perp).unwrap_or([1.0, 0.0, 0.0])
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn compute_valence(n_particles: usize, constraints: &[DistanceConstraint]) -> Vec<f32> {
        let mut valence = vec![0.0_f32; n_particles.max(1)];
        for dc in constraints {
            let a = dc.a as usize;
            let b = dc.b as usize;
            if a < valence.len() {
                valence[a] += 1.0;
            }
            if b < valence.len() {
                valence[b] += 1.0;
            }
        }
        // Ensure minimum of 1 to avoid division by zero.
        valence.iter_mut().for_each(|v| *v = v.max(1.0));
        valence
    }
}

// ---------------------------------------------------------------------------
// Stiffness ↔ compliance conversion
// ---------------------------------------------------------------------------

/// Convert Havok stiffness [0, 1] to XPBD compliance. `_dt` is unused because
/// `project_distance_constraints` already divides by h².
pub fn havok_stiffness_to_compliance(stiffness: f32, _dt: f32) -> f32 {
    let s = stiffness.clamp(0.0, 0.9999);
    (1.0 - s) * 1.0e-3 + 1.0e-6
}
