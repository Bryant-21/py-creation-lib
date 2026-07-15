// Port of `py_creation_lib/python/creation_lib/havok_cloth/bake.py` — 10-phase bake pipeline.
//
// Converts a `ClothSetupObject` into an `HkxFile` containing all HKX objects
// needed for a complete BSClothExtraData packfile.
//
// Pointer encoding
// ----------------
// During bake, all intra-file pointers are stored as pending String values
// with the target's "#NNNN" name. The final-pass resolution walks every
// value and rewrites "#NNNN" strings to `HkxValue::Pointer(Some(index))`.
//
// The '#NNNN' approach matches Python's `_counter`-based naming, keeps the
// bake logic simple, and the Rust HKX builder already uses index-based
// pointers — the resolve step is a one-shot post-pass.

use std::collections::HashMap;

use crate::error::{HavokError, HavokResult};
use crate::hkx::model::{HkxFile, HkxMember, HkxObject};
use crate::hkx::types::HkxValue;

use super::setup::cloth_setup::ClothSetupObject;
use super::setup::constraint_setup::{ConstraintSetupObject, OpaqueConstraintSetup};
use super::setup::mesh::SimulationSetupMesh;
use super::setup::operator_setup::{MeshBoneDeformSetup, OpaqueOperatorSetup, OperatorSetupObject};
use super::setup::sim_cloth_setup::SimClothSetupObject;

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Bake a `ClothSetupObject` into an `HkxFile`.
pub fn bake_cloth_setup(setup: &ClothSetupObject) -> HavokResult<HkxFile> {
    if setup.sim_cloth_setups.is_empty() {
        return Err(HavokError::InvalidInput(
            "bake_cloth_setup: no sim_cloth_setups — at least one is required".to_string(),
        ));
    }

    let mut ctx = BakeContext::new(setup);

    phase0_resolve_meshes(&mut ctx)?;
    phase1_particles(&mut ctx)?;
    phase2_constraints(&mut ctx)?;
    phase3_batch_constraints(&mut ctx)?;
    phase4_collidables(&mut ctx);
    phase5_buffers(&mut ctx);
    phase6_operators(&mut ctx);
    phase7_states(&mut ctx);
    phase8_transform_sets(&mut ctx);
    phase9_finalize(&mut ctx);

    // Resolve '#NNNN' name → index pointers in the final pass.
    let objects = resolve_pointers(ctx.objects, &ctx.name_to_index);

    Ok(HkxFile::from_tagxml(11, "hk_2014.1.0-r1", objects))
}

// ---------------------------------------------------------------------------
// Internal structures
// ---------------------------------------------------------------------------

/// Per-constraint-entry data built during phase 2 and consumed by phase 3.
struct ConstraintBakeEntry {
    class_name: &'static str,
    constraint_name: String,
    /// (particleA, particleB, restLength, stiffness) — used by Standard /
    /// Stretch link constraints. For BendStiffness this is built solely so
    /// that batch coloring can run on (particleA, particleB); the emission
    /// path uses `bend_links` for the 10-field SDK shape.
    links: Vec<(u16, u16, f32, f32)>,
    /// SDK-shaped bend links (10 fields). Populated only when
    /// `class_name == "hclBendStiffnessConstraintSet"`.
    bend_links: Vec<BendLink>,
    /// For LocalRange: stiffness scalar and shape variant
    local_range_stiffness: f32,
    local_range_shape: i32,
    /// For BendStiffness: use_rest_pose_config flag
    bend_use_rest_pose: bool,
    /// For Volume: stiffness scalar
    volume_stiffness: f32,
    /// For Opaque pass-through: raw HKX members to emit verbatim.
    opaque_members: Option<Vec<HkxMember>>,
    /// For Opaque pass-through: the actual SDK class name.
    opaque_class_name: String,
}

/// SDK `hclBendStiffnessConstraintSet::Link` (10 fields). Field semantics:
/// - particleA, particleB: opposing-vertex pair across a shared edge
/// - particleC, particleD: the two endpoints of the shared edge
/// - weightA..D: linear-combination coefficients for the bending vector
///   `R = pA*wA + pB*wB + pC*wC + pD*wD` (SDK ctor leaves all four at 0,
///   which is a no-op; topology-aware setups should compute proper weights)
/// - restCurvature: target curvature scalar (0 = flat rest pose)
/// - bendStiffness: per-link stiffness
#[derive(Debug, Clone, Copy)]
struct BendLink {
    particle_a: u16,
    particle_b: u16,
    particle_c: u16,
    particle_d: u16,
    weight_a: f32,
    weight_b: f32,
    weight_c: f32,
    weight_d: f32,
    rest_curvature: f32,
    bend_stiffness: f32,
}

/// Intermediate per-sim-cloth state accumulated across phases.
struct SimClothBakeData<'a> {
    setup: &'a SimClothSetupObject,
    sim_mesh: Option<&'a SimulationSetupMesh>,
    /// Names of the inline particle data objects (phase 1)
    particle_member_values: Vec<HkxValue>, // HkxValue::Object per particle
    fixed_indices: Vec<u16>,
    positions: Vec<[f32; 4]>,
    pose_object_name: String,
    /// Per-constraint-entry data (phase 2 input, phase 3 output)
    constraint_entries: Vec<ConstraintBakeEntry>,
    /// Names of the emitted constraint set objects (phase 3 output)
    batched_constraint_names: Vec<String>,
    /// Names of hclCollidable objects (phase 4)
    collidable_names: Vec<String>,
    collidable_transform_indices: Vec<u32>,
    /// Name of the final hclSimClothData object (phase 9)
    sim_cloth_object_name: String,
}

impl<'a> SimClothBakeData<'a> {
    fn new(setup: &'a SimClothSetupObject) -> Self {
        Self {
            setup,
            sim_mesh: None,
            particle_member_values: Vec::new(),
            fixed_indices: Vec::new(),
            positions: Vec::new(),
            pose_object_name: String::new(),
            constraint_entries: Vec::new(),
            batched_constraint_names: Vec::new(),
            collidable_names: Vec::new(),
            collidable_transform_indices: Vec::new(),
            sim_cloth_object_name: String::new(),
        }
    }
}

/// Shared state across all bake phases.
pub struct BakeContext<'a> {
    setup: &'a ClothSetupObject,
    /// Emitted top-level objects, in emission order.
    objects: Vec<HkxObject>,
    counter: u32,
    /// Maps '#NNNN' name → object index for pointer resolution.
    name_to_index: HashMap<String, usize>,

    /// Per-sim-cloth intermediate state.
    sim_cloth_data: Vec<SimClothBakeData<'a>>,

    /// Setup name → index maps for cross-referencing (phase 6, 7, 8).
    buffer_name_to_idx: HashMap<String, usize>,
    transform_set_name_to_idx: HashMap<String, usize>,
    sim_cloth_name_to_idx: HashMap<String, usize>,

    /// Phase 5 output: buffer object names (in order)
    buffer_object_names: Vec<String>,
    /// Phase 6 output: operator object names (in order)
    operator_object_names: Vec<String>,
    /// Phase 7 output: state object names
    state_object_names: Vec<String>,
    /// Phase 8 output: transform-set object names
    transform_set_object_names: Vec<String>,
}

impl<'a> BakeContext<'a> {
    fn new(setup: &'a ClothSetupObject) -> Self {
        let mut buffer_name_to_idx = HashMap::new();
        let mut transform_set_name_to_idx = HashMap::new();
        let mut sim_cloth_name_to_idx = HashMap::new();

        for (i, b) in setup.buffer_setups.iter().enumerate() {
            buffer_name_to_idx.insert(b.name.clone(), i);
        }
        for (i, t) in setup.transform_set_setups.iter().enumerate() {
            transform_set_name_to_idx.insert(t.name.clone(), i);
        }
        for (i, sc) in setup.sim_cloth_setups.iter().enumerate() {
            sim_cloth_name_to_idx.insert(sc.name.clone(), i);
        }

        let sim_cloth_data = setup
            .sim_cloth_setups
            .iter()
            .map(|sc| SimClothBakeData::new(sc))
            .collect();

        Self {
            setup,
            objects: Vec::new(),
            counter: 0,
            name_to_index: HashMap::new(),
            sim_cloth_data,
            buffer_name_to_idx,
            transform_set_name_to_idx,
            sim_cloth_name_to_idx,
            buffer_object_names: Vec::new(),
            operator_object_names: Vec::new(),
            state_object_names: Vec::new(),
            transform_set_object_names: Vec::new(),
        }
    }

    fn next_name(&mut self) -> String {
        self.counter += 1;
        format!("#{:04}", self.counter)
    }

    /// Add a new top-level object and return its name + index.
    fn add_object(&mut self, class_name: &str) -> (&mut HkxObject, String) {
        let name = self.next_name();
        let index = self.objects.len();
        self.name_to_index.insert(name.clone(), index);
        self.objects.push(HkxObject {
            name: Some(name.clone()),
            offset: 0,
            signature: 0,
            class_name: class_name.to_string(),
            members: Vec::new(),
        });
        let obj = &mut self.objects[index];
        (obj, name)
    }
}

// ---------------------------------------------------------------------------
// Builder helpers — mirrors Python's _add_direct / _add_array / _add_string /
// _add_pointer / _add_enum.
// ---------------------------------------------------------------------------

fn add_direct(obj: &mut HkxObject, name: &str, value: HkxValue) {
    obj.members.push(HkxMember {
        name: name.to_string(),
        value,
    });
}

fn add_string(obj: &mut HkxObject, name: &str, value: &str) {
    obj.members.push(HkxMember {
        name: name.to_string(),
        value: HkxValue::String {
            value: value.to_string(),
            is_null: false,
        },
    });
}

/// Add a pointer member that will be resolved in the final pass.
/// `target_name` is '#NNNN' or empty (None pointer).
fn add_pointer(obj: &mut HkxObject, name: &str, target_name: String) {
    // Store as a special marker: String value holds the name temporarily.
    // We resolve it in the final pass.
    obj.members.push(HkxMember {
        name: name.to_string(),
        value: pending_ptr(target_name),
    });
}

/// Add an array of pointers (names resolved in the final pass).
fn add_ptr_array(obj: &mut HkxObject, name: &str, target_names: Vec<String>) {
    let items: Vec<HkxValue> = target_names.into_iter().map(pending_ptr).collect();
    obj.members.push(HkxMember {
        name: name.to_string(),
        value: HkxValue::Array(items),
    });
}

/// Add a typed scalar array (u16, u32, f32, bool, …).
fn add_scalar_array(obj: &mut HkxObject, name: &str, values: Vec<HkxValue>) {
    obj.members.push(HkxMember {
        name: name.to_string(),
        value: HkxValue::Array(values),
    });
}

/// Add an inline-struct array (each element is HkxValue::Object).
fn add_struct_array(obj: &mut HkxObject, name: &str, elements: Vec<HkxValue>) {
    obj.members.push(HkxMember {
        name: name.to_string(),
        value: HkxValue::Array(elements),
    });
}

/// Encode a deferred pointer by object name. Resolved in the final pass by
/// `resolve_pointers` which converts each `PendingPtr` to `Pointer(Some(index))`.
fn pending_ptr(target_name: String) -> HkxValue {
    if target_name.is_empty() {
        HkxValue::Pointer(None)
    } else {
        HkxValue::PendingPtr(target_name)
    }
}

// ---------------------------------------------------------------------------
// Phase 0: Resolve simulation meshes
// ---------------------------------------------------------------------------

fn phase0_resolve_meshes(ctx: &mut BakeContext) -> HavokResult<()> {
    for scd in &mut ctx.sim_cloth_data {
        if let Some(mesh) = &scd.setup.simulation_mesh {
            if mesh.positions.len() > usize::from(u16::MAX) {
                return Err(HavokError::InvalidInput(format!(
                    "SimClothSetup {:?} has {} particles; max is {} (hkUint16 limit)",
                    scd.setup.name,
                    mesh.positions.len(),
                    u16::MAX
                )));
            }
            scd.sim_mesh = Some(mesh);
        } else {
            return Err(HavokError::InvalidInput(format!(
                "SimClothSetup {:?} has no simulation_mesh. \
                 Either provide one or build from a SetupMesh.",
                scd.setup.name
            )));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Phase 1: Particle data generation
// ---------------------------------------------------------------------------

fn phase1_particles(ctx: &mut BakeContext) -> HavokResult<()> {
    // We need to iterate over sim_cloth_data but also call add_object on ctx,
    // so we extract the needed data first.
    let n_cloths = ctx.sim_cloth_data.len();

    for ci in 0..n_cloths {
        let (_, fixed_set, positions, masses, radii, frictions) = {
            let scd = &ctx.sim_cloth_data[ci];
            let sc = scd.setup;
            let mesh = scd.sim_mesh.unwrap(); // guaranteed by phase0
            let n = mesh.positions.len();

            let mut masses =
                resolve_float_input(&sc.particle_mass, &mesh.vertex_float_channels, n, 0.02);
            let radii =
                resolve_float_input(&sc.particle_radius, &mesh.vertex_float_channels, n, 0.5);
            let frictions =
                resolve_float_input(&sc.particle_friction, &mesh.vertex_float_channels, n, 0.35);

            // Rescale masses so their sum equals total_mass when requested.
            if sc.rescale_mass && sc.total_mass > 0.0 {
                let sum: f32 = masses.iter().sum();
                if sum > 0.0 {
                    let scale = sc.total_mass / sum;
                    for m in &mut masses {
                        *m *= scale;
                    }
                }
            }

            // Determine fixed particles (kind 0=ALL, 1=NONE, 2=CHANNEL, 3=INVERSE_CHANNEL)
            let fixed_set = match sc.fixed_particles.kind {
                0 => (0..n).collect::<std::collections::HashSet<usize>>(), // ALL
                1 => std::collections::HashSet::new(),                     // NONE
                2 => {
                    // CHANNEL
                    let ch_name = &sc.fixed_particles.channel_name;
                    if let Some(ch) = mesh.vertex_selection_channels.get(ch_name) {
                        ch.iter()
                            .filter_map(|&v| usize::try_from(v).ok())
                            .filter(|&i| i < n)
                            .collect()
                    } else {
                        std::collections::HashSet::new()
                    }
                }
                3 => {
                    // INVERSE_CHANNEL
                    let ch_name = &sc.fixed_particles.channel_name;
                    if let Some(ch) = mesh.vertex_selection_channels.get(ch_name) {
                        let ch_set: std::collections::HashSet<usize> = ch
                            .iter()
                            .filter_map(|&v| usize::try_from(v).ok())
                            .filter(|&i| i < n)
                            .collect();
                        (0..n).filter(|i| !ch_set.contains(i)).collect()
                    } else {
                        (0..n).collect()
                    }
                }
                _ => std::collections::HashSet::new(),
            };

            (
                mesh,
                fixed_set,
                mesh.positions.clone(),
                masses,
                radii,
                frictions,
            )
        };

        let n = positions.len();

        // Auto-promote zero-mass movable particles into the fixed set: a movable
        // particle with mass<=0 would produce inv_mass=0, creating a non-SDK
        // "zero-inv-mass movable" state that the validator surfaces as a warning.
        let mut fixed_set = fixed_set;
        for i in 0..n {
            if !fixed_set.contains(&i) && masses[i] <= 0.0 {
                fixed_set.insert(i);
            }
        }

        // Build per-particle inline struct values
        let mut particle_values: Vec<HkxValue> = Vec::with_capacity(n);
        for i in 0..n {
            let pos = positions[i];
            let mass = masses[i];
            let inv_mass = if fixed_set.contains(&i) {
                0.0f32
            } else {
                1.0 / mass
            };
            let radius_val = radii[i];
            let friction_val = frictions[i];

            let members = vec![
                HkxMember {
                    name: "mass".to_string(),
                    value: HkxValue::F32(mass),
                },
                HkxMember {
                    name: "invMass".to_string(),
                    value: HkxValue::F32(inv_mass),
                },
                HkxMember {
                    name: "radius".to_string(),
                    value: HkxValue::F32(radius_val),
                },
                HkxMember {
                    name: "friction".to_string(),
                    value: HkxValue::F32(friction_val),
                },
                HkxMember {
                    name: "position".to_string(),
                    value: HkxValue::F32List(vec![pos[0], pos[1], pos[2], pos[3]]),
                },
            ];
            particle_values.push(HkxValue::Object(members));
        }

        // Build default cloth pose object
        let pose_positions: Vec<HkxValue> = positions
            .iter()
            .map(|p| HkxValue::F32List(vec![p[0], p[1], p[2], p[3]]))
            .collect();

        let pose_name = {
            let (pose_obj, name) = ctx.add_object("hclSimClothPose");
            add_string(pose_obj, "name", "DefaultClothPose");
            add_struct_array(pose_obj, "positions", pose_positions);
            name
        };

        // Now store results back in scd
        let mut fixed_indices: Vec<u16> = fixed_set
            .iter()
            .map(|&i| {
                u16::try_from(i).map_err(|_| {
                    HavokError::InvalidInput(format!("fixed particle index {i} exceeds u16"))
                })
            })
            .collect::<HavokResult<Vec<_>>>()?;
        fixed_indices.sort();

        let scd = &mut ctx.sim_cloth_data[ci];
        scd.particle_member_values = particle_values;
        scd.fixed_indices = fixed_indices;
        scd.positions = positions.to_vec();
        scd.pose_object_name = pose_name;
    }
    Ok(())
}

fn resolve_float_input(
    input: &super::setup::types::VertexFloatInput,
    channels: &std::collections::HashMap<String, Vec<f32>>,
    n: usize,
    default: f32,
) -> Vec<f32> {
    if input.kind == 1 {
        // CHANNEL
        if let Some(ch) = channels.get(&input.channel_name) {
            let mut v = ch.clone();
            v.resize(n, default);
            return v;
        }
    }
    // CONSTANT (or channel not found — fall back to constant)
    let v = if input.constant_value == 0.0 && default != 0.0 {
        default
    } else {
        input.constant_value
    };
    vec![v; n]
}

// ---------------------------------------------------------------------------
// Phase 2: Constraint generation
// ---------------------------------------------------------------------------

fn phase2_constraints(ctx: &mut BakeContext) -> HavokResult<()> {
    let n_cloths = ctx.sim_cloth_data.len();
    for ci in 0..n_cloths {
        let entries = build_constraint_entries(&ctx.sim_cloth_data[ci])?;
        ctx.sim_cloth_data[ci].constraint_entries = entries;
    }
    Ok(())
}

fn build_constraint_entries(scd: &SimClothBakeData) -> HavokResult<Vec<ConstraintBakeEntry>> {
    let mesh = match scd.sim_mesh {
        Some(m) => m,
        None => return Ok(Vec::new()),
    };

    // Build unique edge set from triangles
    let mut edges: Vec<(usize, usize)> = {
        let mut edge_set = std::collections::BTreeSet::new();
        for tri in &mesh.triangles {
            let (a, b, c) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
            edge_set.insert((a.min(b), a.max(b)));
            edge_set.insert((b.min(c), b.max(c)));
            edge_set.insert((a.min(c), a.max(c)));
        }
        edge_set.into_iter().collect()
    };
    // Sort edges for determinism (already sorted from BTreeSet)
    edges.sort();

    // Build adjacency for bend stiffness
    let mut edge_to_tris: std::collections::BTreeMap<(usize, usize), Vec<usize>> =
        std::collections::BTreeMap::new();
    for (ti, tri) in mesh.triangles.iter().enumerate() {
        let (a, b, c) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
        edge_to_tris
            .entry((a.min(b), a.max(b)))
            .or_default()
            .push(ti);
        edge_to_tris
            .entry((b.min(c), b.max(c)))
            .or_default()
            .push(ti);
        edge_to_tris
            .entry((a.min(c), a.max(c)))
            .or_default()
            .push(ti);
    }

    let n = mesh.positions.len();
    if n > usize::from(u16::MAX) + 1 {
        return Err(HavokError::InvalidInput(format!(
            "cloth mesh has {n} particles; constraint indices are u16"
        )));
    }
    let fixed_set: std::collections::HashSet<usize> =
        scd.fixed_indices.iter().map(|&i| i as usize).collect();

    let mut entries = Vec::new();

    for cs in &scd.setup.constraint_setups {
        match cs {
            ConstraintSetupObject::StandardLink(s) => {
                let stiffness_val = s.stiffness.constant_value;
                let links = build_edge_links(&edges, &mesh.positions, n, stiffness_val);
                entries.push(ConstraintBakeEntry {
                    class_name: "hclStandardLinkConstraintSet",
                    constraint_name: s.name.clone(),
                    links,
                    bend_links: Vec::new(),
                    local_range_stiffness: 1.0,
                    local_range_shape: 0,
                    bend_use_rest_pose: false,
                    volume_stiffness: 1.0,
                    opaque_members: None,
                    opaque_class_name: String::new(),
                });
            }

            ConstraintSetupObject::StretchLink(s) => {
                let stiffness_val = s.stiffness.constant_value;
                let mut links = Vec::new();
                for &(a, b) in &edges {
                    if a < n && b < n {
                        let a_fixed = fixed_set.contains(&a);
                        let b_fixed = fixed_set.contains(&b);
                        if a_fixed != b_fixed {
                            let rest = dist3(mesh.positions[a], mesh.positions[b]);
                            links.push((a as u16, b as u16, rest, stiffness_val));
                        }
                    }
                }
                entries.push(ConstraintBakeEntry {
                    class_name: "hclStretchLinkConstraintSet",
                    constraint_name: s.name.clone(),
                    links,
                    bend_links: Vec::new(),
                    local_range_stiffness: 1.0,
                    local_range_shape: 0,
                    bend_use_rest_pose: false,
                    volume_stiffness: 1.0,
                    opaque_members: None,
                    opaque_class_name: String::new(),
                });
            }

            ConstraintSetupObject::BendStiffness(s) => {
                let stiffness_val = s.bend_stiffness.constant_value;
                // Build SDK-shape bend links (4 particles + 4 weights +
                // restCurvature + bendStiffness). `links` is also populated
                // with (a, b, rest, stiffness) so the existing greedy graph
                // colorer can batch on the (particleA, particleB) pair.
                //
                // Weight defaults: SDK ctor leaves weightA..D at 0 (no-op
                // bend); without topology-aware weight computation we emit
                // 1.0 across the board so the link is at least non-trivial.
                // restCurvature defaults to 0.0 (flat rest pose).
                let mut links: Vec<(u16, u16, f32, f32)> = Vec::new();
                let mut bend_links: Vec<BendLink> = Vec::new();
                for (&(ea, eb), tris) in &edge_to_tris {
                    if tris.len() >= 2 {
                        let t0 = mesh.triangles[tris[0]];
                        let t1 = mesh.triangles[tris[1]];
                        let t0v: [usize; 3] = [t0[0] as usize, t0[1] as usize, t0[2] as usize];
                        let t1v: [usize; 3] = [t1[0] as usize, t1[1] as usize, t1[2] as usize];
                        let opp0: Vec<usize> = t0v
                            .iter()
                            .copied()
                            .filter(|&v| v != ea && v != eb)
                            .collect();
                        let opp1: Vec<usize> = t1v
                            .iter()
                            .copied()
                            .filter(|&v| v != ea && v != eb)
                            .collect();
                        if let (Some(&a), Some(&b)) = (opp0.first(), opp1.first()) {
                            if a < n && b < n && ea < n && eb < n {
                                let rest = dist3(mesh.positions[a], mesh.positions[b]);
                                links.push((a as u16, b as u16, rest, stiffness_val));
                                bend_links.push(BendLink {
                                    particle_a: a as u16,
                                    particle_b: b as u16,
                                    particle_c: ea as u16,
                                    particle_d: eb as u16,
                                    weight_a: 1.0,
                                    weight_b: 1.0,
                                    weight_c: 1.0,
                                    weight_d: 1.0,
                                    rest_curvature: 0.0,
                                    bend_stiffness: stiffness_val,
                                });
                            }
                        }
                    }
                }
                entries.push(ConstraintBakeEntry {
                    class_name: "hclBendStiffnessConstraintSet",
                    constraint_name: s.name.clone(),
                    links,
                    bend_links,
                    local_range_stiffness: 1.0,
                    local_range_shape: 0,
                    bend_use_rest_pose: s.use_rest_pose_config,
                    volume_stiffness: 1.0,
                    opaque_members: None,
                    opaque_class_name: String::new(),
                });
            }

            ConstraintSetupObject::LocalRange(s) => {
                entries.push(ConstraintBakeEntry {
                    class_name: "hclLocalRangeConstraintSet",
                    constraint_name: s.name.clone(),
                    links: Vec::new(),
                    bend_links: Vec::new(),
                    local_range_stiffness: s.stiffness,
                    local_range_shape: s.local_range_shape,
                    bend_use_rest_pose: false,
                    volume_stiffness: 1.0,
                    opaque_members: None,
                    opaque_class_name: String::new(),
                });
            }

            ConstraintSetupObject::BonePlanes(s) => {
                entries.push(ConstraintBakeEntry {
                    class_name: "hclBonePlanesConstraintSet",
                    constraint_name: s.name.clone(),
                    links: Vec::new(),
                    bend_links: Vec::new(),
                    local_range_stiffness: 1.0,
                    local_range_shape: 0,
                    bend_use_rest_pose: false,
                    volume_stiffness: 1.0,
                    opaque_members: None,
                    opaque_class_name: String::new(),
                });
            }

            ConstraintSetupObject::Volume(s) => {
                let stiffness_val = s.stiffness.constant_value;
                entries.push(ConstraintBakeEntry {
                    // SDK class name is `hclVolumeConstraint` (no Set suffix).
                    class_name: "hclVolumeConstraint",
                    constraint_name: s.name.clone(),
                    links: Vec::new(),
                    bend_links: Vec::new(),
                    local_range_stiffness: 1.0,
                    local_range_shape: 0,
                    bend_use_rest_pose: false,
                    volume_stiffness: stiffness_val,
                    opaque_members: None,
                    opaque_class_name: String::new(),
                });
            }

            ConstraintSetupObject::Opaque(s) => {
                entries.push(ConstraintBakeEntry {
                    class_name: "hclStandardLinkConstraintSet", // placeholder, overridden by opaque path
                    constraint_name: s.name.clone(),
                    links: Vec::new(),
                    bend_links: Vec::new(),
                    local_range_stiffness: 1.0,
                    local_range_shape: 0,
                    bend_use_rest_pose: false,
                    volume_stiffness: 1.0,
                    opaque_members: Some(s.members.clone()),
                    opaque_class_name: s.class_name.clone(),
                });
            }
        }
    }

    Ok(entries)
}

fn build_edge_links(
    edges: &[(usize, usize)],
    positions: &[[f32; 4]],
    n: usize,
    stiffness: f32,
) -> Vec<(u16, u16, f32, f32)> {
    edges
        .iter()
        .filter(|&&(a, b)| a < n && b < n)
        .map(|&(a, b)| {
            let rest = dist3(positions[a], positions[b]);
            (a as u16, b as u16, rest, stiffness)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Phase 3: Constraint batching (greedy graph coloring)
// ---------------------------------------------------------------------------

fn phase3_batch_constraints(ctx: &mut BakeContext) -> HavokResult<()> {
    let n_cloths = ctx.sim_cloth_data.len();
    for ci in 0..n_cloths {
        let n_entries = ctx.sim_cloth_data[ci].constraint_entries.len();
        for ei in 0..n_entries {
            emit_constraint_batch(ctx, ci, ei)?;
        }
    }
    Ok(())
}

fn emit_constraint_batch(ctx: &mut BakeContext, ci: usize, ei: usize) -> HavokResult<()> {
    let class_name;
    let constraint_name;
    let has_links;
    let bend_use_rest_pose;
    let local_range_stiffness;
    let local_range_shape;
    let volume_stiffness;
    let opaque_members: Option<Vec<HkxMember>>;
    let opaque_class_name: String;
    {
        let entry = &ctx.sim_cloth_data[ci].constraint_entries[ei];
        class_name = entry.class_name;
        constraint_name = entry.constraint_name.clone();
        has_links = !entry.links.is_empty();
        bend_use_rest_pose = entry.bend_use_rest_pose;
        local_range_stiffness = entry.local_range_stiffness;
        local_range_shape = entry.local_range_shape;
        volume_stiffness = entry.volume_stiffness;
        opaque_members = entry.opaque_members.clone();
        opaque_class_name = entry.opaque_class_name.clone();
    }

    if let Some(members) = opaque_members {
        let (cset_obj, cset_name) = ctx.add_object(&opaque_class_name);
        cset_obj.members.extend(members);
        ctx.sim_cloth_data[ci]
            .batched_constraint_names
            .push(cset_name);
        return Ok(());
    }

    if !has_links {
        // Emit empty constraint set
        let (cset_obj, cset_name) = ctx.add_object(class_name);
        add_string(cset_obj, "name", &constraint_name);
        match class_name {
            "hclStandardLinkConstraintSet" => {
                add_struct_array(cset_obj, "links", vec![]);
                add_struct_array(cset_obj, "batches", vec![]);
            }
            "hclStretchLinkConstraintSet" => {
                add_struct_array(cset_obj, "links", vec![]);
                add_struct_array(cset_obj, "batches", vec![]);
            }
            "hclBendStiffnessConstraintSet" => {
                add_struct_array(cset_obj, "links", vec![]);
                add_struct_array(cset_obj, "batches", vec![]);
                add_direct(
                    cset_obj,
                    "useRestPoseConfig",
                    HkxValue::Bool(bend_use_rest_pose),
                );
            }
            "hclLocalRangeConstraintSet" => {
                add_direct(cset_obj, "stiffness", HkxValue::F32(local_range_stiffness));
                add_direct(
                    cset_obj,
                    "localRangeShape",
                    HkxValue::I32(local_range_shape),
                );
            }
            "hclBonePlanesConstraintSet" => {
                add_struct_array(cset_obj, "perParticlePlanes", vec![]);
                add_struct_array(cset_obj, "globalPlanes", vec![]);
                add_struct_array(cset_obj, "perParticleAngles", vec![]);
            }
            "hclVolumeConstraint" => {
                // SDK fields: m_frameDatas (FrameData[]) and m_applyDatas
                // (ApplyData[]). No stiffness field on the constraint itself
                // — per-particle stiffness lives on each ApplyData. The
                // setup-side `stiffness` scalar (`volume_stiffness`) is not
                // currently propagated; topology-aware FrameData / ApplyData
                // emission is deferred.
                let _ = volume_stiffness;
                add_struct_array(cset_obj, "frameDatas", vec![]);
                add_struct_array(cset_obj, "applyDatas", vec![]);
            }
            _ => {}
        }
        ctx.sim_cloth_data[ci]
            .batched_constraint_names
            .push(cset_name);
        return Ok(());
    }

    // Greedy graph coloring to produce batches. For BendStiffness we also
    // permute the parallel `bend_links` vec so its order tracks `links`.
    let batches = {
        let entry = &mut ctx.sim_cloth_data[ci].constraint_entries[ei];
        greedy_color_links_with_bend(&mut entry.links, &mut entry.bend_links)
    };

    // Check disjointness (mirrors Python's _assert_batch_disjointness).
    check_batch_disjointness(
        &batches,
        &ctx.sim_cloth_data[ci].constraint_entries[ei].links,
        class_name,
    )?;

    // Build link struct values. BendStiffness emits the full SDK 10-field
    // shape; other link constraints keep the legacy 4-field shape.
    let link_structs: Vec<HkxValue> = if class_name == "hclBendStiffnessConstraintSet" {
        ctx.sim_cloth_data[ci].constraint_entries[ei]
            .bend_links
            .iter()
            .map(|link| {
                HkxValue::Object(vec![
                    HkxMember {
                        name: "particleA".to_string(),
                        value: HkxValue::U16(link.particle_a),
                    },
                    HkxMember {
                        name: "particleB".to_string(),
                        value: HkxValue::U16(link.particle_b),
                    },
                    HkxMember {
                        name: "particleC".to_string(),
                        value: HkxValue::U16(link.particle_c),
                    },
                    HkxMember {
                        name: "particleD".to_string(),
                        value: HkxValue::U16(link.particle_d),
                    },
                    HkxMember {
                        name: "weightA".to_string(),
                        value: HkxValue::F32(link.weight_a),
                    },
                    HkxMember {
                        name: "weightB".to_string(),
                        value: HkxValue::F32(link.weight_b),
                    },
                    HkxMember {
                        name: "weightC".to_string(),
                        value: HkxValue::F32(link.weight_c),
                    },
                    HkxMember {
                        name: "weightD".to_string(),
                        value: HkxValue::F32(link.weight_d),
                    },
                    HkxMember {
                        name: "restCurvature".to_string(),
                        value: HkxValue::F32(link.rest_curvature),
                    },
                    HkxMember {
                        name: "bendStiffness".to_string(),
                        value: HkxValue::F32(link.bend_stiffness),
                    },
                ])
            })
            .collect()
    } else {
        ctx.sim_cloth_data[ci].constraint_entries[ei]
            .links
            .iter()
            .map(|&(pa, pb, rest, stiffness)| {
                HkxValue::Object(vec![
                    HkxMember {
                        name: "particleA".to_string(),
                        value: HkxValue::U16(pa),
                    },
                    HkxMember {
                        name: "particleB".to_string(),
                        value: HkxValue::U16(pb),
                    },
                    HkxMember {
                        name: "restLength".to_string(),
                        value: HkxValue::F32(rest),
                    },
                    HkxMember {
                        name: "stiffness".to_string(),
                        value: HkxValue::F32(stiffness),
                    },
                ])
            })
            .collect()
    };

    // Build batch struct values.
    let mut offset: u32 = 0;
    let batch_structs: Vec<HkxValue> = batches
        .iter()
        .map(|batch| {
            let num = batch.len() as u32;
            let v = HkxValue::Object(vec![
                HkxMember {
                    name: "startLink".to_string(),
                    value: HkxValue::U32(offset),
                },
                HkxMember {
                    name: "numLinks".to_string(),
                    value: HkxValue::U32(num),
                },
            ]);
            offset += num;
            v
        })
        .collect();

    let (cset_obj, cset_name) = ctx.add_object(class_name);
    add_string(cset_obj, "name", &constraint_name);
    add_struct_array(cset_obj, "links", link_structs);
    add_struct_array(cset_obj, "batches", batch_structs);
    if class_name == "hclBendStiffnessConstraintSet" {
        add_direct(
            cset_obj,
            "useRestPoseConfig",
            HkxValue::Bool(bend_use_rest_pose),
        );
    }

    ctx.sim_cloth_data[ci]
        .batched_constraint_names
        .push(cset_name);
    Ok(())
}

/// Greedy graph coloring that also permutes a parallel `bend_links` vec
/// in step with `links` so the SDK 10-field bend emission stays aligned
/// with the batch order. When `bend_links` is empty (non-bend constraints)
/// it's left untouched.
fn greedy_color_links_with_bend(
    links: &mut Vec<(u16, u16, f32, f32)>,
    bend_links: &mut Vec<BendLink>,
) -> Vec<Vec<usize>> {
    if links.is_empty() {
        return Vec::new();
    }

    let mut batches: Vec<Vec<usize>> = Vec::new();
    let mut batch_particles: Vec<std::collections::HashSet<u16>> = Vec::new();

    for (i, &(a, b, _, _)) in links.iter().enumerate() {
        let mut placed = false;
        for (bi, bp) in batch_particles.iter_mut().enumerate() {
            if !bp.contains(&a) && !bp.contains(&b) {
                batches[bi].push(i);
                bp.insert(a);
                bp.insert(b);
                placed = true;
                break;
            }
        }
        if !placed {
            let mut s = std::collections::HashSet::new();
            s.insert(a);
            s.insert(b);
            batches.push(vec![i]);
            batch_particles.push(s);
        }
    }

    let original_links = links.clone();
    let original_bend = bend_links.clone();
    let permute_bend = !original_bend.is_empty();
    debug_assert!(
        !permute_bend || original_bend.len() == original_links.len(),
        "bend_links must be parallel to links"
    );

    let mut reordered: Vec<(u16, u16, f32, f32)> = Vec::with_capacity(links.len());
    let mut reordered_bend: Vec<BendLink> = if permute_bend {
        Vec::with_capacity(bend_links.len())
    } else {
        Vec::new()
    };
    let mut new_batches: Vec<Vec<usize>> = Vec::with_capacity(batches.len());

    for batch in &batches {
        let start = reordered.len();
        for &idx in batch {
            reordered.push(original_links[idx]);
            if permute_bend {
                reordered_bend.push(original_bend[idx]);
            }
        }
        new_batches.push((start..reordered.len()).collect());
    }

    *links = reordered;
    if permute_bend {
        *bend_links = reordered_bend;
    }
    new_batches
}

/// Greedy graph coloring — assign each link to the first batch where neither
/// particle appears. Reorders `links` in-place to match batch order.
/// Returns batches as lists of link indices into the (reordered) links vec.
#[allow(dead_code)]
fn greedy_color_links(links: &mut Vec<(u16, u16, f32, f32)>) -> Vec<Vec<usize>> {
    if links.is_empty() {
        return Vec::new();
    }

    let mut batches: Vec<Vec<usize>> = Vec::new();
    let mut batch_particles: Vec<std::collections::HashSet<u16>> = Vec::new();

    for (i, &(a, b, _, _)) in links.iter().enumerate() {
        let mut placed = false;
        for (bi, bp) in batch_particles.iter_mut().enumerate() {
            if !bp.contains(&a) && !bp.contains(&b) {
                batches[bi].push(i);
                bp.insert(a);
                bp.insert(b);
                placed = true;
                break;
            }
        }
        if !placed {
            let mut s = std::collections::HashSet::new();
            s.insert(a);
            s.insert(b);
            batches.push(vec![i]);
            batch_particles.push(s);
        }
    }

    // Reorder links to match batch order.
    let original_links = links.clone();
    let mut reordered: Vec<(u16, u16, f32, f32)> = Vec::with_capacity(links.len());
    let mut new_batches: Vec<Vec<usize>> = Vec::with_capacity(batches.len());

    for batch in &batches {
        let start = reordered.len();
        for &idx in batch {
            reordered.push(original_links[idx]);
        }
        new_batches.push((start..reordered.len()).collect());
    }

    *links = reordered;
    new_batches
}

fn check_batch_disjointness(
    batches: &[Vec<usize>],
    links: &[(u16, u16, f32, f32)],
    class_name: &str,
) -> HavokResult<()> {
    for (bi, batch) in batches.iter().enumerate() {
        let mut seen: std::collections::HashSet<u16> = std::collections::HashSet::new();
        for &li in batch {
            let (a, b, _, _) = links[li];
            if seen.contains(&a) {
                return Err(HavokError::InvalidInput(format!(
                    "Constraint batch disjointness violation in {class_name} batch {bi}: \
                     particle {a} appears in multiple links"
                )));
            }
            if seen.contains(&b) {
                return Err(HavokError::InvalidInput(format!(
                    "Constraint batch disjointness violation in {class_name} batch {bi}: \
                     particle {b} appears in multiple links"
                )));
            }
            seen.insert(a);
            seen.insert(b);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Phase 4: Collidable generation
// ---------------------------------------------------------------------------

fn phase4_collidables(ctx: &mut BakeContext) {
    let n_cloths = ctx.sim_cloth_data.len();
    for ci in 0..n_cloths {
        let n_cols = ctx.sim_cloth_data[ci].setup.collidable_setups.len();
        for coli in 0..n_cols {
            let (
                capsule_shape,
                tapered_shape,
                name,
                pinch_enabled,
                pinch_priority,
                pinch_radius,
                bone_name,
            ) = {
                let col_setup = &ctx.sim_cloth_data[ci].setup.collidable_setups[coli];
                (
                    col_setup.shape.clone(),
                    col_setup.tapered_shape.clone(),
                    col_setup.name.clone(),
                    col_setup.pinch_detection_enabled,
                    col_setup.pinch_detection_priority,
                    col_setup.pinch_detection_radius,
                    col_setup.driving_bone_name.clone(),
                )
            };

            let shape_name = if let Some(tc) = tapered_shape {
                emit_tapered_capsule_shape(ctx, &tc)
            } else {
                let shape = capsule_shape.unwrap_or_default();
                emit_capsule_shape(ctx, &shape)
            };

            // Emit hclCollidable
            let (col_obj, col_name) = ctx.add_object("hclCollidable");
            add_string(col_obj, "name", &name);
            add_pointer(col_obj, "shape", shape_name);
            add_direct(
                col_obj,
                "pinchDetectionEnabled",
                HkxValue::Bool(pinch_enabled),
            );
            add_direct(
                col_obj,
                "pinchDetectionPriority",
                HkxValue::I8(pinch_priority as i8),
            );
            add_direct(col_obj, "pinchDetectionRadius", HkxValue::F32(pinch_radius));

            let ts_idx = ctx.sim_cloth_data[ci].setup.collidable_transform_set_index as usize;
            let transform_set = ctx.setup.transform_set_setups.get(ts_idx);
            let ti = resolve_bone_index(&bone_name, transform_set);
            ctx.sim_cloth_data[ci].collidable_names.push(col_name);
            ctx.sim_cloth_data[ci].collidable_transform_indices.push(ti);
        }
    }
}

/// Emit hclCapsuleShape — SDK members: m_start, m_end, m_dir, m_radius,
/// m_capLenSqrdInv (refs/hk2018_1_0_r1/Source/Cloth/Cloth/Collide/Shape/
/// Capsule/hclCapsuleShape.h).
fn emit_capsule_shape(
    ctx: &mut BakeContext,
    shape: &super::setup::collidable_setup::CapsuleShapeSetup,
) -> String {
    let dir = capsule_dir(shape.start, shape.end);
    let (shape_obj, shape_name) = ctx.add_object("hclCapsuleShape");
    add_direct(shape_obj, "start", HkxValue::F32List(shape.start.to_vec()));
    add_direct(shape_obj, "end", HkxValue::F32List(shape.end.to_vec()));
    add_direct(shape_obj, "dir", HkxValue::F32List(dir.to_vec()));
    add_direct(shape_obj, "radius", HkxValue::F32(shape.big_radius));
    let len_sqrd = {
        let dx = shape.end[0] - shape.start[0];
        let dy = shape.end[1] - shape.start[1];
        let dz = shape.end[2] - shape.start[2];
        dx * dx + dy * dy + dz * dz
    };
    let cap_len_sqrd_inv = if len_sqrd > 1e-8 { 1.0 / len_sqrd } else { 0.0 };
    add_direct(shape_obj, "capLenSqrdInv", HkxValue::F32(cap_len_sqrd_inv));
    shape_name
}

/// Emit hclTaperedCapsuleShape — SDK members per
/// refs/hk2018_1_0_r1/Source/Cloth/Cloth/Collide/Shape/TaperedCapsule/
/// hclTaperedCapsuleShape.h. The four authored inputs (small/big sphere
/// centers + radii) drive every other field via runtime geometry of the
/// double-cone connecting the two spheres.
fn emit_tapered_capsule_shape(
    ctx: &mut BakeContext,
    tc: &super::setup::collidable_setup::TaperedCapsuleShapeSetup,
) -> String {
    let small = tc.small;
    let big = tc.big;
    let r_small = tc.small_radius;
    let r_big = tc.big_radius;

    // axis from small → big and its length
    let axis_raw = [big[0] - small[0], big[1] - small[1], big[2] - small[2]];
    let axis_len =
        (axis_raw[0] * axis_raw[0] + axis_raw[1] * axis_raw[1] + axis_raw[2] * axis_raw[2]).sqrt();
    let axis = if axis_len > 1e-10 {
        [
            axis_raw[0] / axis_len,
            axis_raw[1] / axis_len,
            axis_raw[2] / axis_len,
        ]
    } else {
        [0.0, 1.0, 0.0]
    };

    // Cone geometry. The cone tangent connects the surfaces of the two
    // spheres; theta is the cone half-angle satisfying
    // sin(theta) = (r_big - r_small) / l where l is the center distance.
    let l = axis_len;
    let dr = r_big - r_small;
    let sin_theta = if l > 1e-10 {
        (dr / l).clamp(-1.0, 1.0)
    } else {
        0.0
    };
    let cos_theta = (1.0 - sin_theta * sin_theta).sqrt();
    let tan_theta = if cos_theta.abs() > 1e-10 {
        sin_theta / cos_theta
    } else {
        0.0
    };
    let tan_theta_sqr = tan_theta * tan_theta;
    // d = projected radius along the axis (depth from sphere center to cone-tangent
    // touchpoint): d = r * sin_theta. Used to clip the per-sphere region.
    let d = r_small * sin_theta;

    // Cone apex sits on the small side along the axis; its location is the
    // small-sphere center pulled back by r_small / sin_theta when the radii
    // differ. Degenerate when the two radii are equal (apex at infinity) —
    // the runtime only consults coneApex when sin_theta > 0.
    let apex_offset = if sin_theta.abs() > 1e-10 {
        r_small / sin_theta
    } else {
        0.0
    };
    let cone_apex = [
        small[0] - axis[0] * apex_offset,
        small[1] - axis[1] * apex_offset,
        small[2] - axis[2] * apex_offset,
        0.0,
    ];
    let cone_axis = [axis[0], axis[1], axis[2], 0.0];

    let l_vec = [axis_raw[0], axis_raw[1], axis_raw[2], 0.0];
    let d_vec = [axis[0] * d, axis[1] * d, axis[2] * d, 0.0];
    let tan_theta_vec_neg = [-tan_theta, -tan_theta, -tan_theta, 0.0];

    let (shape_obj, shape_name) = ctx.add_object("hclTaperedCapsuleShape");
    add_direct(shape_obj, "small", HkxValue::F32List(small.to_vec()));
    add_direct(shape_obj, "big", HkxValue::F32List(big.to_vec()));
    add_direct(shape_obj, "coneApex", HkxValue::F32List(cone_apex.to_vec()));
    add_direct(shape_obj, "coneAxis", HkxValue::F32List(cone_axis.to_vec()));
    add_direct(shape_obj, "lVec", HkxValue::F32List(l_vec.to_vec()));
    add_direct(shape_obj, "dVec", HkxValue::F32List(d_vec.to_vec()));
    add_direct(
        shape_obj,
        "tanThetaVecNeg",
        HkxValue::F32List(tan_theta_vec_neg.to_vec()),
    );
    add_direct(shape_obj, "smallRadius", HkxValue::F32(r_small));
    add_direct(shape_obj, "bigRadius", HkxValue::F32(r_big));
    add_direct(shape_obj, "l", HkxValue::F32(l));
    add_direct(shape_obj, "d", HkxValue::F32(d));
    add_direct(shape_obj, "cosTheta", HkxValue::F32(cos_theta));
    add_direct(shape_obj, "sinTheta", HkxValue::F32(sin_theta));
    add_direct(shape_obj, "tanTheta", HkxValue::F32(tan_theta));
    add_direct(shape_obj, "tanThetaSqr", HkxValue::F32(tan_theta_sqr));
    shape_name
}

fn capsule_dir(start: [f32; 4], end: [f32; 4]) -> [f32; 4] {
    let dx = end[0] - start[0];
    let dy = end[1] - start[1];
    let dz = end[2] - start[2];
    let len = (dx * dx + dy * dy + dz * dz).sqrt();
    if len > 1e-10 {
        [dx / len, dy / len, dz / len, 0.0]
    } else {
        [0.0, 1.0, 0.0, 0.0]
    }
}

/// Resolve a bone name to a transform index.
///
/// Looks the name up in `transform_set.bone_names` first for an exact match.
/// Falls back to parsing the `"bone_N"` synthetic pattern that the reverse
/// path emits when the real bone name wasn't available.
fn resolve_bone_index(
    bone_name: &str,
    transform_set: Option<&super::setup::buffer_setup::TransformSetSetupObject>,
) -> u32 {
    if let Some(ts) = transform_set {
        if let Some(idx) = ts.bone_names.iter().position(|n| n == bone_name) {
            return idx as u32;
        }
    }
    if let Some(suffix) = bone_name.strip_prefix("bone_") {
        suffix.parse::<u32>().unwrap_or(0)
    } else {
        0
    }
}

// ---------------------------------------------------------------------------
// Phase 5: Buffer layout
// ---------------------------------------------------------------------------

fn phase5_buffers(ctx: &mut BakeContext) {
    let n_buffers = ctx.setup.buffer_setups.len();
    for bi in 0..n_buffers {
        let buf_setup = &ctx.setup.buffer_setups[bi];
        // SDK enum: Display=1, StaticDisplay=2, SimCloth=6, Scratch=0.
        // Scratch uses a separate class (hclScratchBufferDefinition).
        let (class_name, type_val): (&str, u32) = match buf_setup.buffer_type {
            0 => ("hclBufferDefinition", 1),        // Display → type 1
            1 => ("hclBufferDefinition", 2),        // StaticDisplay → type 2
            2 => ("hclBufferDefinition", 6),        // SimCloth → type 6
            3 => ("hclScratchBufferDefinition", 0), // Scratch → type 0 + class flip
            _ => ("hclBufferDefinition", 1),        // unknown → Display fallback
        };

        // Vertex/tri counts
        let (num_verts, num_tris) = if let Some(setup_mesh) = &buf_setup.setup_mesh {
            (
                setup_mesh.positions.len() as u32,
                setup_mesh.triangles.len() as u32,
            )
        } else {
            let mut verts = 0u32;
            let mut tris = 0u32;
            for scd in &ctx.sim_cloth_data {
                if let Some(m) = scd.sim_mesh {
                    verts = verts.max(m.positions.len() as u32);
                    tris = tris.max(m.triangles.len() as u32);
                }
            }
            (verts, tris)
        };

        let has_tris = buf_setup.has_triangles;
        let has_normals = buf_setup.has_normals;
        let has_tangents = buf_setup.has_tangents;
        let name = buf_setup.name.clone();

        let (buf_obj, buf_name) = ctx.add_object(class_name);
        add_string(buf_obj, "name", &name);
        add_direct(buf_obj, "type", HkxValue::U32(type_val));
        add_direct(buf_obj, "numVertices", HkxValue::U32(num_verts));
        add_direct(
            buf_obj,
            "numTriangles",
            HkxValue::U32(if has_tris { num_tris } else { 0 }),
        );
        add_direct(buf_obj, "storeNormals", HkxValue::Bool(has_normals));
        add_direct(
            buf_obj,
            "storeTangentsAndBiTangents",
            HkxValue::Bool(has_tangents),
        );

        ctx.buffer_object_names.push(buf_name);
    }
}

// ---------------------------------------------------------------------------
// Skin weight binning — port of py_creation_lib/python/creation_lib/havok_cloth/bake.py `_bin_skin_weights`
// ---------------------------------------------------------------------------

/// Bin vertex skin weights into fiveBoneEntries / sixBoneEntries / sevenBoneEntries /
/// eightBoneEntries on an already-created `hclObjectSpaceSkinPNOperator` object.
///
/// `bone_weights` is `source_mesh.bone_weights`: a per-vertex list of `[bone_index_f32, weight]`
/// pairs.  Vertices with ≤4 non-zero bones after filtering are still placed in fiveBoneEntries
/// (padded to 4 entries, n_bones tracks the raw count before padding).
fn bin_skin_weights(op_obj: &mut HkxObject, bone_weights: &[Vec<[f32; 2]>]) {
    // Collect unique used bone indices (weight > 0).
    let mut used: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();
    for vertex in bone_weights {
        for pair in vertex {
            let bi = pair[0] as u32;
            let w = pair[1];
            if w > 0.0 {
                used.insert(bi);
            }
        }
    }
    let transform_indices: Vec<u32> = used.iter().copied().collect();
    let remap: std::collections::HashMap<u32, u16> = transform_indices
        .iter()
        .enumerate()
        .map(|(new_idx, &old_idx)| (old_idx, new_idx as u16))
        .collect();

    // Update the transformIndices member to the sorted unique-bone list.
    for m in op_obj.members.iter_mut() {
        if m.name == "transformIndices" {
            m.value = HkxValue::Array(
                transform_indices
                    .iter()
                    .map(|&i| HkxValue::U16(i as u16))
                    .collect(),
            );
            break;
        }
    }

    let mut five: Vec<HkxValue> = Vec::new();
    let mut six: Vec<HkxValue> = Vec::new();
    let mut seven: Vec<HkxValue> = Vec::new();
    let mut eight: Vec<HkxValue> = Vec::new();

    for (vi, vertex) in bone_weights.iter().enumerate() {
        // Filter to non-zero weights and remap bone indices.
        let mut pairs: Vec<(u16, f32)> = vertex
            .iter()
            .filter(|p| p[1] > 0.0)
            .map(|p| {
                (
                    *remap.get(&(p[0] as u32)).copied().as_ref().unwrap_or(&0u16),
                    p[1],
                )
            })
            .collect();
        if pairs.is_empty() {
            continue;
        }

        // Sort by weight descending.
        pairs.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // Normalize weights.
        let total: f32 = pairs.iter().map(|(_, w)| w).sum();
        if total > 0.0 {
            for p in pairs.iter_mut() {
                p.1 /= total;
            }
        }

        let n_bones = pairs.len();

        // Pad to at least 4 entries.
        while pairs.len() < 4 {
            pairs.push((0u16, 0.0f32));
        }

        // Build inline struct (HkxValue::Object) for this vertex entry.
        let mut members: Vec<crate::hkx::model::HkxMember> =
            Vec::with_capacity(1 + pairs.len() * 2);
        members.push(crate::hkx::model::HkxMember {
            name: "vertexIndex".to_string(),
            value: HkxValue::U16(vi as u16),
        });
        for (j, (bi, w)) in pairs.iter().take(8).enumerate() {
            members.push(crate::hkx::model::HkxMember {
                name: format!("boneIndex{j}"),
                value: HkxValue::U16(*bi),
            });
            members.push(crate::hkx::model::HkxMember {
                name: format!("boneWeight{j}"),
                value: HkxValue::F32(*w),
            });
        }
        let entry = HkxValue::Object(members);

        match n_bones {
            n if n <= 5 => five.push(entry),
            6 => six.push(entry),
            7 => seven.push(entry),
            _ => eight.push(entry),
        }
    }

    // Replace the four bin members on the operator object.
    for m in op_obj.members.iter_mut() {
        match m.name.as_str() {
            "fiveBoneEntries" => m.value = HkxValue::Array(five.clone()),
            "sixBoneEntries" => m.value = HkxValue::Array(six.clone()),
            "sevenBoneEntries" => m.value = HkxValue::Array(seven.clone()),
            "eightBoneEntries" => m.value = HkxValue::Array(eight.clone()),
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Phase 6: Operator generation
// ---------------------------------------------------------------------------

fn phase6_operators(ctx: &mut BakeContext) {
    let n_ops = ctx.setup.operator_setups.len();
    for oi in 0..n_ops {
        let op_name = emit_operator(ctx, oi);
        if let Some(name) = op_name {
            ctx.operator_object_names.push(name);
        }
    }
}

fn emit_operator(ctx: &mut BakeContext, oi: usize) -> Option<String> {
    let op = &ctx.setup.operator_setups[oi];
    match op {
        OperatorSetupObject::Simulate(s) => {
            let op_name = s.name.clone();
            let sc_idx = ctx
                .sim_cloth_name_to_idx
                .get(&s.sim_cloth_setup_name)
                .copied()
                .unwrap_or(0);
            let (sub_steps, n_iter, adapt, constraint_exec) = if let Some(cfg) = s.configs.first() {
                let exec: Vec<HkxValue> = cfg
                    .constraint_execution_order_names
                    .iter()
                    .map(|name_or_idx| HkxValue::U32(name_or_idx.parse::<u32>().unwrap_or(0)))
                    .collect();
                (
                    cfg.num_substeps as u32,
                    cfg.num_solve_iterations as u32,
                    cfg.adapt_constraint_stiffness,
                    exec,
                )
            } else {
                (1, 3, false, vec![])
            };

            let (op_obj, name) = ctx.add_object("hclSimulateOperator");
            add_string(op_obj, "name", &op_name);
            add_direct(op_obj, "simClothIndex", HkxValue::U32(sc_idx as u32));
            add_direct(op_obj, "subSteps", HkxValue::U32(sub_steps));
            add_direct(op_obj, "numberOfSolveIterations", HkxValue::U32(n_iter));
            add_direct(op_obj, "adaptConstraintStiffness", HkxValue::Bool(adapt));
            add_scalar_array(op_obj, "constraintExecution", constraint_exec);
            Some(name)
        }

        OperatorSetupObject::Skin(s) => {
            let op_name = s.name.clone();
            let ts_idx = ctx
                .transform_set_name_to_idx
                .get(&s.transform_set_name)
                .copied()
                .unwrap_or(0);
            let out_buf_idx = ctx
                .buffer_name_to_idx
                .get(&s.output_buffer_name)
                .copied()
                .unwrap_or(0);

            // Collect bone_weights before the mutable ctx.add_object borrow.
            let bone_weights: Option<Vec<Vec<[f32; 2]>>> =
                ctx.sim_cloth_data.iter().find_map(|scd| {
                    scd.sim_mesh
                        .and_then(|sm| sm.source_mesh.as_deref())
                        .filter(|src| !src.bone_weights.is_empty())
                        .map(|src| src.bone_weights.clone())
                });

            let (op_obj, name) = ctx.add_object("hclObjectSpaceSkinPNOperator");
            add_string(op_obj, "name", &op_name);
            add_direct(op_obj, "transformSetIndex", HkxValue::U32(ts_idx as u32));
            add_direct(
                op_obj,
                "outputBufferIndex",
                HkxValue::U32(out_buf_idx as u32),
            );
            add_struct_array(op_obj, "boneFromSkinMeshTransforms", vec![]);
            add_scalar_array(op_obj, "transformIndices", vec![]);
            add_struct_array(op_obj, "fiveBoneEntries", vec![]);
            add_struct_array(op_obj, "sixBoneEntries", vec![]);
            add_struct_array(op_obj, "sevenBoneEntries", vec![]);
            add_struct_array(op_obj, "eightBoneEntries", vec![]);
            if let Some(bw) = bone_weights {
                bin_skin_weights(op_obj, &bw);
            }
            Some(name)
        }

        OperatorSetupObject::MeshBoneDeform(s) => {
            let op_name = s.name.clone();
            let in_buf_idx = ctx
                .buffer_name_to_idx
                .get(&s.input_buffer_name)
                .copied()
                .unwrap_or(0);
            let out_ts_idx = ctx
                .transform_set_name_to_idx
                .get(&s.output_transform_set_name)
                .copied()
                .unwrap_or(0);

            let (pairs, local_transforms) = compute_triangle_bone_binding(ctx, s);

            let (op_obj, name) = ctx.add_object("hclSimpleMeshBoneDeformOperator");
            add_string(op_obj, "name", &op_name);
            add_direct(op_obj, "inputBufferIdx", HkxValue::U32(in_buf_idx as u32));
            add_direct(
                op_obj,
                "outputTransformSetIdx",
                HkxValue::U32(out_ts_idx as u32),
            );
            add_struct_array(op_obj, "triangleBonePairs", pairs);
            add_struct_array(op_obj, "localBoneTransforms", local_transforms);
            Some(name)
        }

        OperatorSetupObject::CopyVertices(s) => {
            let op_name = s.name.clone();
            let in_buf_idx = ctx
                .buffer_name_to_idx
                .get(&s.input_buffer_name)
                .copied()
                .unwrap_or(0);
            let out_buf_idx = ctx
                .buffer_name_to_idx
                .get(&s.output_buffer_name)
                .copied()
                .unwrap_or(0);
            let copy_normals = s.copy_normals;

            let (op_obj, name) = ctx.add_object("hclCopyVerticesOperator");
            add_string(op_obj, "name", &op_name);
            add_direct(op_obj, "inputBufferIdx", HkxValue::U32(in_buf_idx as u32));
            add_direct(op_obj, "outputBufferIdx", HkxValue::U32(out_buf_idx as u32));
            add_direct(op_obj, "copyNormals", HkxValue::Bool(copy_normals));
            Some(name)
        }

        OperatorSetupObject::MoveParticles(s) => {
            let op_name = s.name.clone();
            let sc_idx = ctx
                .sim_cloth_name_to_idx
                .get(&s.sim_cloth_setup_name)
                .copied()
                .unwrap_or(0);
            let ref_buf_idx = ctx
                .buffer_name_to_idx
                .get(&s.display_buffer_name)
                .copied()
                .unwrap_or(0);

            let (op_obj, name) = ctx.add_object("hclMoveParticlesOperator");
            add_string(op_obj, "name", &op_name);
            add_direct(op_obj, "simClothIndex", HkxValue::U32(sc_idx as u32));
            add_direct(op_obj, "refBufferIdx", HkxValue::U32(ref_buf_idx as u32));
            Some(name)
        }

        OperatorSetupObject::GatherAllVertices(s) => {
            let op_name = s.name.clone();
            let in_buf_idx = ctx
                .buffer_name_to_idx
                .get(&s.input_buffer_name)
                .copied()
                .unwrap_or(0);
            let out_buf_idx = ctx
                .buffer_name_to_idx
                .get(&s.output_buffer_name)
                .copied()
                .unwrap_or(0);
            let indices = s.vertex_input_from_vertex_output.clone();
            let derived_partial = s.partial_gather || indices.iter().any(|&v| v < 0);
            let gather_normals = s.gather_normals;

            let (op_obj, name) = ctx.add_object("hclGatherAllVerticesOperator");
            add_string(op_obj, "name", &op_name);
            add_scalar_array(
                op_obj,
                "vertexInputFromVertexOutput",
                indices.into_iter().map(HkxValue::I16).collect(),
            );
            add_direct(op_obj, "inputBufferIdx", HkxValue::U32(in_buf_idx as u32));
            add_direct(op_obj, "outputBufferIdx", HkxValue::U32(out_buf_idx as u32));
            add_direct(op_obj, "gatherNormals", HkxValue::Bool(gather_normals));
            add_direct(op_obj, "partialGather", HkxValue::Bool(derived_partial));
            Some(name)
        }

        OperatorSetupObject::Opaque(s) => {
            let (op_obj, name) = ctx.add_object(&s.class_name.clone());
            op_obj.members.extend(s.members.clone());
            Some(name)
        }
    }
}

fn compute_triangle_bone_binding(
    ctx: &BakeContext,
    op_setup: &MeshBoneDeformSetup,
) -> (Vec<HkxValue>, Vec<HkxValue>) {
    let ts_idx = ctx
        .transform_set_name_to_idx
        .get(&op_setup.output_transform_set_name)
        .copied()
        .unwrap_or(0);
    if ts_idx >= ctx.setup.transform_set_setups.len() {
        return (vec![], vec![]);
    }
    let tset = &ctx.setup.transform_set_setups[ts_idx];
    let n_bones = tset.bone_names.len();
    if n_bones == 0 {
        return (vec![], vec![]);
    }

    // Find the first sim mesh with triangles
    let sim_mesh = ctx
        .sim_cloth_data
        .iter()
        .find_map(|scd| scd.sim_mesh.filter(|m| !m.triangles.is_empty()));
    let sim_mesh = match sim_mesh {
        Some(m) => m,
        None => return (vec![], vec![]),
    };

    let sim_positions = &sim_mesh.positions;
    let sim_triangles = &sim_mesh.triangles;
    let n_tris = sim_triangles.len();
    if n_tris == 0 || sim_positions.is_empty() {
        return (vec![], vec![]);
    }

    // Precompute triangle centroids
    let tri_centroids: Vec<(f32, f32, f32)> = sim_triangles
        .iter()
        .map(|t| {
            let p0 = sim_positions[t[0] as usize];
            let p1 = sim_positions[t[1] as usize];
            let p2 = sim_positions[t[2] as usize];
            (
                (p0[0] + p1[0] + p2[0]) / 3.0,
                (p0[1] + p1[1] + p2[1]) / 3.0,
                (p0[2] + p1[2] + p2[2]) / 3.0,
            )
        })
        .collect();

    let rest_positions = &op_setup.bone_rest_positions;
    let have_rest = rest_positions.len() == n_bones;

    let tri_assignments: Vec<usize> = if have_rest {
        rest_positions
            .iter()
            .map(|rp| {
                let rpx = rp.first().copied().unwrap_or(0.0);
                let rpy = rp.get(1).copied().unwrap_or(0.0);
                let rpz = rp.get(2).copied().unwrap_or(0.0);
                tri_centroids
                    .iter()
                    .enumerate()
                    .min_by(|(_, a), (_, b)| {
                        let da = (a.0 - rpx).powi(2) + (a.1 - rpy).powi(2) + (a.2 - rpz).powi(2);
                        let db = (b.0 - rpx).powi(2) + (b.1 - rpy).powi(2) + (b.2 - rpz).powi(2);
                        da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .map(|(i, _)| i)
                    .unwrap_or(0)
            })
            .collect()
    } else {
        (0..n_bones).map(|i| (i * n_tris) / n_bones).collect()
    };

    let mut pairs: Vec<HkxValue> = Vec::with_capacity(n_bones);
    let mut local_transforms: Vec<HkxValue> = Vec::with_capacity(n_bones);

    for (i, tri_idx) in tri_assignments.iter().enumerate() {
        let bone_off = (i as u16) * 64;
        let tri_off = (*tri_idx as u16) * 6;
        pairs.push(HkxValue::Object(vec![
            HkxMember {
                name: "boneOffset".to_string(),
                value: HkxValue::U16(bone_off),
            },
            HkxMember {
                name: "triangleOffset".to_string(),
                value: HkxValue::U16(tri_off),
            },
        ]));

        let tri = sim_triangles[*tri_idx];
        let p0 = sim_positions[tri[0] as usize];
        let p1 = sim_positions[tri[1] as usize];
        let p2 = sim_positions[tri[2] as usize];
        let centroid = tri_centroids[*tri_idx];

        let bone_rest = if have_rest {
            let rp = &rest_positions[i];
            (
                rp.first().copied().unwrap_or(centroid.0),
                rp.get(1).copied().unwrap_or(centroid.1),
                rp.get(2).copied().unwrap_or(centroid.2),
            )
        } else {
            centroid
        };

        let tri_frame = triangle_frame_matrix(p0, p1, p2);
        let tri_frame_inv = invert_affine(tri_frame);
        let bone_rest_mat = translation_matrix(bone_rest.0, bone_rest.1, bone_rest.2);
        let local = matmul4(tri_frame_inv, bone_rest_mat);
        // Column-major (Havok convention): flatten [r][c] → for c in 0..4, for r in 0..4
        let flat: Vec<f32> = (0..4)
            .flat_map(|c| (0..4).map(move |r| local[r][c]))
            .collect();
        local_transforms.push(HkxValue::F32List(flat));
    }

    (pairs, local_transforms)
}

// ---------------------------------------------------------------------------
// Phase 7: State assembly
// ---------------------------------------------------------------------------

fn phase7_states(ctx: &mut BakeContext) {
    let n_states = ctx.setup.state_setups.len();
    for si in 0..n_states {
        let state_json = &ctx.setup.state_setups[si];
        let name = state_json
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let op_indices: Vec<u32> = state_json
            .get("operator_indices")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_u64().map(|n| n as u32))
                    .collect()
            })
            .unwrap_or_default();

        let mut used_buffers = std::collections::BTreeSet::new();
        let mut used_tsets = std::collections::BTreeSet::new();
        let mut used_scs = std::collections::BTreeSet::new();

        for &oi in &op_indices {
            if let Some(op) = ctx.setup.operator_setups.get(oi as usize) {
                match op {
                    OperatorSetupObject::Simulate(s) => {
                        used_scs.insert(
                            ctx.sim_cloth_name_to_idx
                                .get(&s.sim_cloth_setup_name)
                                .copied()
                                .unwrap_or(0) as u32,
                        );
                    }
                    OperatorSetupObject::Skin(s) => {
                        used_tsets.insert(
                            ctx.transform_set_name_to_idx
                                .get(&s.transform_set_name)
                                .copied()
                                .unwrap_or(0) as u32,
                        );
                        used_buffers.insert(
                            ctx.buffer_name_to_idx
                                .get(&s.output_buffer_name)
                                .copied()
                                .unwrap_or(0) as u32,
                        );
                    }
                    OperatorSetupObject::CopyVertices(s) => {
                        used_buffers.insert(
                            ctx.buffer_name_to_idx
                                .get(&s.input_buffer_name)
                                .copied()
                                .unwrap_or(0) as u32,
                        );
                        used_buffers.insert(
                            ctx.buffer_name_to_idx
                                .get(&s.output_buffer_name)
                                .copied()
                                .unwrap_or(0) as u32,
                        );
                    }
                    OperatorSetupObject::MoveParticles(s) => {
                        used_scs.insert(
                            ctx.sim_cloth_name_to_idx
                                .get(&s.sim_cloth_setup_name)
                                .copied()
                                .unwrap_or(0) as u32,
                        );
                        used_buffers.insert(
                            ctx.buffer_name_to_idx
                                .get(&s.display_buffer_name)
                                .copied()
                                .unwrap_or(0) as u32,
                        );
                    }
                    OperatorSetupObject::GatherAllVertices(s) => {
                        used_buffers.insert(
                            ctx.buffer_name_to_idx
                                .get(&s.input_buffer_name)
                                .copied()
                                .unwrap_or(0) as u32,
                        );
                        used_buffers.insert(
                            ctx.buffer_name_to_idx
                                .get(&s.output_buffer_name)
                                .copied()
                                .unwrap_or(0) as u32,
                        );
                    }
                    OperatorSetupObject::MeshBoneDeform(s) => {
                        used_buffers.insert(
                            ctx.buffer_name_to_idx
                                .get(&s.input_buffer_name)
                                .copied()
                                .unwrap_or(0) as u32,
                        );
                        used_tsets.insert(
                            ctx.transform_set_name_to_idx
                                .get(&s.output_transform_set_name)
                                .copied()
                                .unwrap_or(0) as u32,
                        );
                    }
                    OperatorSetupObject::Opaque(_) => {}
                }
            }
        }

        let (state_obj, state_name) = ctx.add_object("hclClothState");
        add_string(state_obj, "name", &name);
        add_scalar_array(
            state_obj,
            "operators",
            op_indices.into_iter().map(HkxValue::U32).collect(),
        );
        add_scalar_array(
            state_obj,
            "usedBuffers",
            used_buffers.into_iter().map(HkxValue::U32).collect(),
        );
        add_scalar_array(
            state_obj,
            "usedTransformSets",
            used_tsets.into_iter().map(HkxValue::U32).collect(),
        );
        add_scalar_array(
            state_obj,
            "usedSimCloths",
            used_scs.into_iter().map(HkxValue::U32).collect(),
        );
        ctx.state_object_names.push(state_name);
    }
}

// ---------------------------------------------------------------------------
// Phase 8: Transform set wiring
// ---------------------------------------------------------------------------

fn phase8_transform_sets(ctx: &mut BakeContext) {
    let n_tsets = ctx.setup.transform_set_setups.len();
    for ti in 0..n_tsets {
        let ts = &ctx.setup.transform_set_setups[ti];
        let ts_name = ts.name.clone();
        let n_bones = ts.bone_names.len() as u32;
        let (ts_obj, obj_name) = ctx.add_object("hclTransformSetDefinition");
        add_string(ts_obj, "name", &ts_name);
        add_direct(ts_obj, "numTransforms", HkxValue::U32(n_bones));
        ctx.transform_set_object_names.push(obj_name);
    }
}

// ---------------------------------------------------------------------------
// Phase 9: Finalize — hclSimClothData + hclClothData + hkRootLevelContainer
// ---------------------------------------------------------------------------

fn phase9_finalize(ctx: &mut BakeContext) {
    let n_cloths = ctx.sim_cloth_data.len();

    for ci in 0..n_cloths {
        emit_sim_cloth_data(ctx, ci);
    }

    // Build hclClothData
    let cloth_name = ctx.setup.name.clone();
    let buf_names = ctx.buffer_object_names.clone();
    let ts_names = ctx.transform_set_object_names.clone();
    let sc_names: Vec<String> = ctx
        .sim_cloth_data
        .iter()
        .map(|scd| scd.sim_cloth_object_name.clone())
        .collect();
    let state_names = ctx.state_object_names.clone();
    let op_names = ctx.operator_object_names.clone();

    let (cloth_obj, _cloth_obj_name) = ctx.add_object("hclClothData");
    add_string(cloth_obj, "name", &cloth_name);
    // SDK `hclClothData::m_targetPlatform` (`hkEnum<Platform, hkUint32>`).
    // Bethesda PC value: HCL_PLATFORM_X64 = 1 << 1 = 2. Without it the
    // runtime treats the field as HCL_PLATFORM_INVALID (0) and may silently
    // disable cloth simulation.
    const HCL_PLATFORM_X64: u32 = 1 << 1;
    add_direct(cloth_obj, "targetPlatform", HkxValue::U32(HCL_PLATFORM_X64));
    add_ptr_array(cloth_obj, "bufferDefinitions", buf_names);
    add_ptr_array(cloth_obj, "transformSetDefinitions", ts_names);
    add_ptr_array(cloth_obj, "simClothDatas", sc_names);
    add_ptr_array(cloth_obj, "clothStateDatas", state_names);
    add_ptr_array(cloth_obj, "operators", op_names);
    add_struct_array(cloth_obj, "stateTransitions", vec![]);
    add_ptr_array(cloth_obj, "actions", vec![]);

    // Build hkRootLevelContainer
    let cloth_obj_name = _cloth_obj_name;
    let variant = HkxValue::Object(vec![
        HkxMember {
            name: "name".to_string(),
            value: HkxValue::String {
                value: "Cloth Data".to_string(),
                is_null: false,
            },
        },
        HkxMember {
            name: "className".to_string(),
            value: HkxValue::String {
                value: "hclClothData".to_string(),
                is_null: false,
            },
        },
        HkxMember {
            name: "variant".to_string(),
            value: pending_ptr(cloth_obj_name),
        },
    ]);

    let (root_obj, _) = ctx.add_object("hkRootLevelContainer");
    add_struct_array(root_obj, "namedVariants", vec![variant]);
}

fn emit_sim_cloth_data(ctx: &mut BakeContext, ci: usize) {
    // Collect data needed before borrowing ctx mutably.
    let sc_name;
    let gravity;
    let global_damping;
    let collision_tolerance;
    let enable_pinch;
    let enable_transfer;
    let do_normals;
    let particle_values: Vec<HkxValue>;
    let fixed_indices: Vec<u16>;
    let total_mass: f32;
    let total_links: usize;
    let batched_constraint_names: Vec<String>;
    let collidable_names: Vec<String>;
    let collidable_transform_indices: Vec<u32>;
    let collidable_transform_set_index: u32;
    let collidable_offsets: Vec<[f32; 16]>;
    let tri_indices: Vec<u32>;
    let triangle_flips_authored: Option<Vec<u8>>;
    let passthrough_members: Vec<HkxMember>;
    let pose_name: String;
    {
        let scd = &ctx.sim_cloth_data[ci];
        sc_name = scd.setup.name.clone();
        gravity = scd.setup.gravity;
        global_damping = scd.setup.global_damping_per_second;
        collision_tolerance = scd.setup.collision_tolerance;
        enable_pinch = scd.setup.enable_pinch_detection;
        enable_transfer = scd.setup.enable_transfer_motion;
        do_normals = scd.setup.do_normals;
        particle_values = scd.particle_member_values.clone();
        fixed_indices = scd.fixed_indices.clone();
        // Compute total_mass
        total_mass = particle_values
            .iter()
            .map(|pv| {
                if let HkxValue::Object(members) = pv {
                    members
                        .iter()
                        .find(|m| m.name == "mass")
                        .and_then(|m| {
                            if let HkxValue::F32(v) = m.value {
                                Some(v)
                            } else {
                                None
                            }
                        })
                        .unwrap_or(0.0)
                } else {
                    0.0
                }
            })
            .sum::<f32>()
            .max(1.0);
        total_links = scd.constraint_entries.iter().map(|e| e.links.len()).sum();
        batched_constraint_names = scd.batched_constraint_names.clone();
        collidable_names = scd.collidable_names.clone();
        collidable_transform_indices = scd.collidable_transform_indices.clone();
        collidable_transform_set_index = scd.setup.collidable_transform_set_index;
        collidable_offsets = scd.setup.collidable_offsets.clone();
        tri_indices = scd
            .sim_mesh
            .map(|m| m.triangles.iter().flat_map(|t| t.iter().copied()).collect())
            .unwrap_or_default();
        triangle_flips_authored = scd.setup.triangle_flips.clone();
        passthrough_members = scd.setup.passthrough_members.clone();
        pose_name = scd.pose_object_name.clone();
    }

    // Build the simulationInfo sub-struct inline.
    //
    // SDK `hclSimClothData::OverridableSimulationInfo` has only two fields:
    // m_gravity and m_globalDampingPerSecond. The misplaced
    // collisionTolerance / pinchDetectionEnabled / transferMotionEnabled
    // bools are top-level on `hclSimClothData` (collisionTolerance lives on
    // m_landscapeCollisionData, not currently emitted).
    let _ = collision_tolerance;
    let sim_info = HkxValue::Object(vec![
        HkxMember {
            name: "gravity".to_string(),
            value: HkxValue::F32List(vec![gravity[0], gravity[1], gravity[2], gravity[3]]),
        },
        HkxMember {
            name: "globalDampingPerSecond".to_string(),
            value: HkxValue::F32(global_damping),
        },
    ]);

    // SDK m_collidableTransformMap requires three parallel arrays: a single
    // transformSetIndex (which transform set drives every entry), one
    // transformIndex per collidable, and one hkMatrix4 offset per collidable.
    // Pad offsets with identity if the user supplied fewer than n_collidables.
    let n_col = collidable_transform_indices.len();
    let identity_4x4 = [
        1.0f32, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];
    let mut offsets_padded: Vec<[f32; 16]> = collidable_offsets.clone();
    while offsets_padded.len() < n_col {
        offsets_padded.push(identity_4x4);
    }
    offsets_padded.truncate(n_col);
    let offset_values: Vec<HkxValue> = offsets_padded
        .iter()
        .map(|m| HkxValue::F32List(m.to_vec()))
        .collect();
    let ctm = HkxValue::Object(vec![
        HkxMember {
            name: "transformSetIndex".to_string(),
            value: HkxValue::U32(collidable_transform_set_index),
        },
        HkxMember {
            name: "transformIndices".to_string(),
            value: HkxValue::Array(
                collidable_transform_indices
                    .iter()
                    .map(|&i| HkxValue::U32(i))
                    .collect(),
            ),
        },
        HkxMember {
            name: "offsets".to_string(),
            value: HkxValue::Array(offset_values),
        },
    ]);

    let (sc_obj, sc_obj_name) = ctx.add_object("hclSimClothData");
    add_string(sc_obj, "name", &sc_name);
    sc_obj.members.push(HkxMember {
        name: "simulationInfo".to_string(),
        value: sim_info,
    });
    // Top-level bools displaced from OverridableSimulationInfo.
    add_direct(
        sc_obj,
        "pinchDetectionEnabled",
        HkxValue::Bool(enable_pinch),
    );
    add_direct(
        sc_obj,
        "transferMotionEnabled",
        HkxValue::Bool(enable_transfer),
    );
    add_struct_array(sc_obj, "particleDatas", particle_values);
    add_scalar_array(
        sc_obj,
        "fixedParticles",
        fixed_indices.iter().map(|&i| HkxValue::U16(i)).collect(),
    );
    add_direct(sc_obj, "totalMass", HkxValue::F32(total_mass));
    // numConstraints is not an SDK field — constraint count is implicit in
    // staticConstraintSets array lengths; strict tagfile readers reject it.
    add_ptr_array(sc_obj, "staticConstraintSets", batched_constraint_names);
    add_ptr_array(sc_obj, "perInstanceCollidables", collidable_names);
    sc_obj.members.push(HkxMember {
        name: "collidableTransformMap".to_string(),
        value: ctm,
    });
    add_scalar_array(
        sc_obj,
        "triangleIndices",
        tri_indices.iter().map(|&i| HkxValue::U32(i)).collect(),
    );
    // SDK m_triangleFlips: hkArray<hkUint8>, one byte per triangle. Used for
    // per-triangle normal inversion when m_doNormals=true. Default to all-zero
    // (no flip) unless the setup carries authored flags.
    let num_tris = tri_indices.len() / 3;
    let flip_bytes: Vec<u8> = match triangle_flips_authored {
        Some(mut v) => {
            v.resize(num_tris, 0);
            v
        }
        None => vec![0u8; num_tris],
    };
    add_scalar_array(
        sc_obj,
        "triangleFlips",
        flip_bytes.iter().map(|&b| HkxValue::U8(b)).collect(),
    );
    add_direct(sc_obj, "doNormals", HkxValue::Bool(do_normals));

    let pose_ptrs = if pose_name.is_empty() {
        vec![]
    } else {
        vec![pose_name]
    };
    add_ptr_array(sc_obj, "simClothPoses", pose_ptrs);

    // Pass-through preservation: append every member captured by reverse
    // (or set directly by callers building synthetic fixtures) that we
    // haven't already modeled semantically. Keeps round-trip honest for
    // m_simOpIds, m_actions, m_landscapeCollisionData,
    // m_virtualCollisionPointsData, etc.
    let already_emitted: std::collections::HashSet<String> =
        sc_obj.members.iter().map(|m| m.name.clone()).collect();
    for m in passthrough_members {
        if !already_emitted.contains(&m.name) {
            sc_obj.members.push(m);
        }
    }

    ctx.sim_cloth_data[ci].sim_cloth_object_name = sc_obj_name;
}

// ---------------------------------------------------------------------------
// Pointer resolution — final pass
// ---------------------------------------------------------------------------

/// Walk every HkxValue in every object and resolve pending_ptr (stored as
/// `HkxValue::String { value: "#NNNN", is_null: false }`) to
/// `HkxValue::Pointer(Some(index))` using the name→index map.
///
/// Real strings from `add_string` are stored with the same variant, so we
/// distinguish them: names used as pointer targets all start with '#'.
fn resolve_pointers(
    mut objects: Vec<HkxObject>,
    name_to_index: &HashMap<String, usize>,
) -> Vec<HkxObject> {
    for obj in &mut objects {
        for member in &mut obj.members {
            resolve_value(&mut member.value, name_to_index);
        }
    }
    objects
}

fn resolve_value(value: &mut HkxValue, name_to_index: &HashMap<String, usize>) {
    match value {
        HkxValue::PendingPtr(name) => {
            let resolved = name_to_index.get(name.as_str()).copied();
            *value = HkxValue::Pointer(resolved);
        }
        HkxValue::Array(items) => {
            for item in items.iter_mut() {
                resolve_value(item, name_to_index);
            }
        }
        HkxValue::Object(members) => {
            for m in members.iter_mut() {
                resolve_value(&mut m.value, name_to_index);
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Math helpers (no nalgebra)
// ---------------------------------------------------------------------------

/// 3D Euclidean distance between two Vec4 positions.
fn dist3(a: [f32; 4], b: [f32; 4]) -> f32 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    let dz = a[2] - b[2];
    (dx * dx + dy * dy + dz * dz).sqrt()
}

/// Build a 4×4 world-from-triangle basis (row-major).
/// Origin = p0. X = normalize(p1-p0). Z = normalize(cross(X, p2-p0)). Y = cross(Z, X).
fn triangle_frame_matrix(p0: [f32; 4], p1: [f32; 4], p2: [f32; 4]) -> [[f32; 4]; 4] {
    let e1 = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
    let e2 = [p2[0] - p0[0], p2[1] - p0[1], p2[2] - p0[2]];

    let x = normalize3(e1);
    let z_raw = cross3(e1, e2);
    let z = normalize3(z_raw);

    match (x, z) {
        (Some(x), Some(z)) => {
            let y_raw = cross3(z, x);
            let y = normalize3(y_raw).unwrap_or([0.0, 1.0, 0.0]);
            [
                [x[0], y[0], z[0], p0[0]],
                [x[1], y[1], z[1], p0[1]],
                [x[2], y[2], z[2], p0[2]],
                [0.0, 0.0, 0.0, 1.0],
            ]
        }
        _ => [
            [1.0, 0.0, 0.0, p0[0]],
            [0.0, 1.0, 0.0, p0[1]],
            [0.0, 0.0, 1.0, p0[2]],
            [0.0, 0.0, 0.0, 1.0],
        ],
    }
}

fn translation_matrix(tx: f32, ty: f32, tz: f32) -> [[f32; 4]; 4] {
    [
        [1.0, 0.0, 0.0, tx],
        [0.0, 1.0, 0.0, ty],
        [0.0, 0.0, 1.0, tz],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

fn matmul4(a: [[f32; 4]; 4], b: [[f32; 4]; 4]) -> [[f32; 4]; 4] {
    let mut out = [[0.0f32; 4]; 4];
    for r in 0..4 {
        for c in 0..4 {
            out[r][c] =
                a[r][0] * b[0][c] + a[r][1] * b[1][c] + a[r][2] * b[2][c] + a[r][3] * b[3][c];
        }
    }
    out
}

/// Invert an affine 4×4 with orthonormal rotation part.
fn invert_affine(m: [[f32; 4]; 4]) -> [[f32; 4]; 4] {
    // Extract R (3×3) and t
    let r = [
        [m[0][0], m[0][1], m[0][2]],
        [m[1][0], m[1][1], m[1][2]],
        [m[2][0], m[2][1], m[2][2]],
    ];
    let t = [m[0][3], m[1][3], m[2][3]];

    // Check orthonormality
    let col0 = r[0][0] * r[0][0] + r[1][0] * r[1][0] + r[2][0] * r[2][0];
    let col1 = r[0][1] * r[0][1] + r[1][1] * r[1][1] + r[2][1] * r[2][1];
    let col2 = r[0][2] * r[0][2] + r[1][2] * r[1][2] + r[2][2] * r[2][2];

    if (col0 - 1.0).abs() < 1e-4 && (col1 - 1.0).abs() < 1e-4 && (col2 - 1.0).abs() < 1e-4 {
        // RT = transpose of R
        let rt = [
            [r[0][0], r[1][0], r[2][0]],
            [r[0][1], r[1][1], r[2][1]],
            [r[0][2], r[1][2], r[2][2]],
        ];
        let neg_rt_t = [
            -(rt[0][0] * t[0] + rt[0][1] * t[1] + rt[0][2] * t[2]),
            -(rt[1][0] * t[0] + rt[1][1] * t[1] + rt[1][2] * t[2]),
            -(rt[2][0] * t[0] + rt[2][1] * t[1] + rt[2][2] * t[2]),
        ];
        [
            [rt[0][0], rt[0][1], rt[0][2], neg_rt_t[0]],
            [rt[1][0], rt[1][1], rt[1][2], neg_rt_t[1]],
            [rt[2][0], rt[2][1], rt[2][2], neg_rt_t[2]],
            [0.0, 0.0, 0.0, 1.0],
        ]
    } else {
        // Fallback: identity (degenerate triangle)
        [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ]
    }
}

fn normalize3(v: [f32; 3]) -> Option<[f32; 3]> {
    let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if n < 1e-12 {
        None
    } else {
        Some([v[0] / n, v[1] / n, v[2] / n])
    }
}

fn cross3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

#[cfg(test)]
mod tests {
    use super::check_batch_disjointness;

    #[test]
    fn check_batch_disjointness_ok_for_valid_batches() {
        // batch 0: links 0 and 1 touch particles {0,1} and {2,3} — disjoint
        let links: Vec<(u16, u16, f32, f32)> =
            vec![(0, 1, 1.0, 1.0), (2, 3, 1.0, 1.0), (0, 2, 1.0, 1.0)];
        // batch 0 = [link0, link1], batch 1 = [link2]
        let batches = vec![vec![0usize, 1], vec![2]];
        assert!(check_batch_disjointness(&batches, &links, "hclStandardLinkConstraintSet").is_ok());
    }

    #[test]
    fn check_batch_disjointness_err_on_particle_collision() {
        // Both links in the same batch share particle 1 — should fail.
        let links: Vec<(u16, u16, f32, f32)> = vec![(0, 1, 1.0, 1.0), (1, 2, 1.0, 1.0)];
        let batches = vec![vec![0usize, 1]]; // overlap: particle 1 in both links
        let result = check_batch_disjointness(&batches, &links, "hclStandardLinkConstraintSet");
        assert!(result.is_err(), "overlapping batch must return Err");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("batch") || msg.contains("particle") || msg.contains("disjointness"),
            "error message must describe the violation: {msg}"
        );
    }
}
