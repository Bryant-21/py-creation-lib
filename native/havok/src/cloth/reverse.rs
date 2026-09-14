// Runtime → setup reverse: builds a ClothSetupObject from ClothData.
//
// Every VertexFloatInput comes back as CONSTANT (channel info is lost in bake),
// mesh topology is inferred from constraint link pairs, and buffer/transform
// references are by name.

use crate::hkx::types::HkxValue;

use super::runtime::base::ClothObjectRef;
use super::runtime::cloth_data::ClothData;
use super::runtime::sim_cloth_data::SimClothData;
use super::runtime::sim_cloth_pose::SimClothPose;
use super::setup::buffer_setup::{BufferSetupObject, TransformSetSetupObject};
use super::setup::cloth_setup::ClothSetupObject;
use super::setup::collidable_setup::{CapsuleShapeSetup, CollidableSetup};
use super::setup::constraint_setup::{
    BendStiffnessSetup, ConstraintSetupObject, LocalRangeSetup, OpaqueConstraintSetup,
    StandardLinkSetup, StretchLinkSetup,
};
use super::setup::mesh::{SetupMesh, SimulationSetupMesh};
use super::setup::operator_setup::{
    CopyVerticesSetup, GatherAllVerticesSetup, MeshBoneDeformSetup, MoveParticlesSetup,
    OpaqueOperatorSetup, OperatorSetupObject, SimulateSetup, SimulateSetupConfig, SkinSetup,
};
use super::setup::sim_cloth_setup::SimClothSetupObject;
use super::setup::types::{VertexFloatInput, VertexSelectionInput};

// ---------------------------------------------------------------------------
// Errors / mode
// ---------------------------------------------------------------------------

/// Errors that the strict reverse path may surface. Lossy mode never returns
/// these — it stubs unknowns silently and keeps going.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReverseError {
    /// A runtime object class is not modelled in setup. `kind` is "operator"
    /// or "constraint" so callers can format a helpful diagnostic.
    UnknownClass {
        kind: &'static str,
        class_name: String,
    },
}

impl std::fmt::Display for ReverseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReverseError::UnknownClass { kind, class_name } => write!(
                f,
                "reverse: unknown {kind} class '{class_name}' — strict mode refuses \
                 to silently stub it (use reverse_cloth_data_lossy to tolerate)"
            ),
        }
    }
}

impl std::error::Error for ReverseError {}

/// Mode flag controlling how the reverse path handles unknown classes.
#[derive(Debug, Clone, Copy)]
pub struct LossyMode {
    /// When true, unknown operator/constraint classes are stubbed instead of
    /// raising ReverseError::UnknownClass.
    pub allow_unknown_classes_as_stubs: bool,
}

impl LossyMode {
    pub const STRICT: Self = Self {
        allow_unknown_classes_as_stubs: false,
    };
    pub const LOSSY: Self = Self {
        allow_unknown_classes_as_stubs: true,
    };
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Strict reverse: fails with `ReverseError::UnknownClass` on an unmodeled
/// operator or constraint class. Use it for inspect → edit → bake; the lossy
/// mode turns unknown operators into Simulate and unknown constraints into
/// StandardLink, which silently breaks e.g. a vanilla FO4 cape.
pub fn reverse_cloth_data(cloth_data: &ClothData<'_>) -> Result<ClothSetupObject, ReverseError> {
    reverse_with_mode(cloth_data, LossyMode::STRICT)
}

/// Lossy reverse: stubs unknown classes, for callers that only want a
/// best-effort overview (e.g. modkit cloth inspect).
pub fn reverse_cloth_data_lossy(cloth_data: &ClothData<'_>) -> ClothSetupObject {
    reverse_with_mode(cloth_data, LossyMode::LOSSY).expect("lossy mode never returns ReverseError")
}

fn reverse_with_mode(
    cloth_data: &ClothData<'_>,
    mode: LossyMode,
) -> Result<ClothSetupObject, ReverseError> {
    let buffer_setups = reverse_buffers(cloth_data);
    let transform_set_setups = reverse_transform_sets(cloth_data);
    let sim_cloth_setups = reverse_sim_cloths(cloth_data, mode)?;
    let operator_setups = reverse_operators(
        cloth_data,
        &buffer_setups,
        &transform_set_setups,
        &sim_cloth_setups,
        mode,
    )?;
    let state_setups = reverse_states(cloth_data);

    Ok(ClothSetupObject {
        name: cloth_data.name().to_string(),
        buffer_setups,
        transform_set_setups,
        sim_cloth_setups,
        operator_setups,
        state_setups,
    })
}

// ---------------------------------------------------------------------------
// Buffers
// ---------------------------------------------------------------------------

fn reverse_buffers(cloth_data: &ClothData<'_>) -> Vec<BufferSetupObject> {
    cloth_data
        .buffer_definitions()
        .iter()
        .map(|buf| {
            let name = get_str(buf, "name");
            let buf_type_val = get_u32(buf, "type");
            let num_tris = get_u32(buf, "numTriangles");
            let store_normals = get_bool(buf, "storeNormals").unwrap_or(true);
            let store_tb = get_bool(buf, "storeTangentsAndBiTangents").unwrap_or(false);

            // Classify buffer type — matches Python reverse.py logic.
            // BufferType enum in Rust: Display=0, StaticDisplay=1, SimCloth=2, Scratch=3.
            let buffer_type: u8 = if buf.class_name() == "hclScratchBufferDefinition" {
                3 // Scratch
            } else if buf_type_val == 1 {
                1 // StaticDisplay (Python: DISPLAY)
            } else if buf_type_val == 6 {
                2 // SimCloth
            } else {
                (buf_type_val.min(3)) as u8
            };

            BufferSetupObject {
                name,
                buffer_type,
                setup_mesh: None,
                has_normals: store_normals,
                has_tangents: store_tb,
                has_bitangents: store_tb,
                has_triangles: num_tris > 0,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Transform sets
// ---------------------------------------------------------------------------

fn reverse_transform_sets(cloth_data: &ClothData<'_>) -> Vec<TransformSetSetupObject> {
    cloth_data
        .transform_set_definitions()
        .iter()
        .map(|tsd| TransformSetSetupObject {
            name: get_str(tsd, "name"),
            bone_names: vec![], // not stored in runtime transform set def
            skeleton_name: String::new(),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Sim cloths
// ---------------------------------------------------------------------------

fn reverse_sim_cloths(
    cloth_data: &ClothData<'_>,
    mode: LossyMode,
) -> Result<Vec<SimClothSetupObject>, ReverseError> {
    cloth_data
        .sim_cloth_datas()
        .iter()
        .map(|scd| reverse_sim_cloth(scd, mode))
        .collect()
}

fn reverse_sim_cloth(
    scd: &SimClothData<'_>,
    mode: LossyMode,
) -> Result<SimClothSetupObject, ReverseError> {
    let particles = scd.particles();
    let num_particles = particles.len();

    // Collect per-particle mass, radius, friction for median computation.
    let mut masses: Vec<f32> = Vec::with_capacity(num_particles);
    let mut radii: Vec<f32> = Vec::with_capacity(num_particles);
    let mut frictions: Vec<f32> = Vec::with_capacity(num_particles);

    for p in particles {
        if let HkxValue::Object(members) = p {
            let mass = struct_float_from_members(members, "mass").unwrap_or(0.0);
            let radius = struct_float_from_members(members, "radius").unwrap_or(0.0);
            let friction = struct_float_from_members(members, "friction").unwrap_or(0.0);
            masses.push(mass);
            radii.push(radius);
            frictions.push(friction);
        }
    }

    // Pull rest-pose positions from the default cloth pose.
    let mut pose_positions: Vec<[f32; 4]> = Vec::new();
    if let Some(pose_ref) = scd.default_pose() {
        let pose = SimClothPose::new(pose_ref);
        for xyz in pose.positions() {
            pose_positions.push([xyz[0], xyz[1], xyz[2], 0.0]);
        }
    }
    // Pad to particle count if needed.
    while pose_positions.len() < num_particles {
        pose_positions.push([0.0, 0.0, 0.0, 0.0]);
    }
    let positions = pose_positions[..num_particles].to_vec();

    // Build triangles from triangleIndices if available.
    let triangles: Vec<[u32; 3]> = {
        let ref_obj = scd.as_ref();
        let arr = ref_obj.get_array("triangleIndices");
        if arr.len() >= 3 {
            arr.chunks(3)
                .filter_map(|chunk| {
                    if chunk.len() == 3 {
                        let a = hkx_value_to_u32(&chunk[0])?;
                        let b = hkx_value_to_u32(&chunk[1])?;
                        let c = hkx_value_to_u32(&chunk[2])?;
                        Some([a, b, c])
                    } else {
                        None
                    }
                })
                .collect()
        } else {
            vec![]
        }
    };

    // Fixed particle indices for selection channel.
    let fixed_indices = scd.fixed_particle_indices();

    let mut fixed_channels: std::collections::HashMap<String, Vec<i32>> =
        std::collections::HashMap::new();
    if !fixed_indices.is_empty() && fixed_indices.len() < num_particles {
        fixed_channels.insert(
            "FixedParticles".to_string(),
            fixed_indices.iter().map(|&i| i as i32).collect(),
        );
    }

    let setup_mesh = SetupMesh {
        name: scd.name().to_string(),
        positions: positions.clone(),
        triangles: triangles.clone(),
        vertex_selection_channels: fixed_channels.clone(),
        ..Default::default()
    };

    let render_to_sim_map: Vec<u32> = (0..num_particles as u32).collect();
    let sim_to_render_map: Vec<Vec<u32>> = (0..num_particles as u32).map(|i| vec![i]).collect();

    let sim_mesh = SimulationSetupMesh {
        positions: positions.clone(),
        triangles: triangles.clone(),
        sim_to_render_map,
        render_to_sim_map,
        vertex_selection_channels: fixed_channels,
        source_mesh: Some(Box::new(setup_mesh)),
        ..Default::default()
    };

    // Simulation info
    let ref_obj = scd.as_ref();
    let gravity = get_struct_vec4(&ref_obj, "simulationInfo", "gravity").unwrap_or([
        0.0,
        0.0,
        super::units::GRAVITY_Z,
        0.0,
    ]);
    let damping =
        get_struct_f32(&ref_obj, "simulationInfo", "globalDampingPerSecond").unwrap_or(0.0);
    let collision_tolerance =
        get_struct_f32(&ref_obj, "simulationInfo", "collisionTolerance").unwrap_or(0.5);
    let pinch_enabled = get_struct_f32(&ref_obj, "simulationInfo", "pinchDetectionEnabled")
        .map(|v| v != 0.0)
        .unwrap_or(false);
    let transfer_motion_enabled =
        get_struct_f32(&ref_obj, "simulationInfo", "transferMotionEnabled")
            .map(|v| v != 0.0)
            .unwrap_or(false);

    let median_mass = median(&masses).unwrap_or(0.02);
    let median_radius = median(&radii).unwrap_or(0.5);
    let median_friction = median(&frictions).unwrap_or(0.35);

    let do_normals = get_bool(&ref_obj, "doNormals").unwrap_or(true);

    // Total mass
    let total_mass: f32 = ref_obj
        .get_float("totalMass")
        .unwrap_or_else(|| masses.iter().sum::<f32>().max(1.0));

    let constraint_setups = reverse_constraints(scd, mode)?;
    let collidable_setups = reverse_collidables(scd);

    // Fixed particle selection input
    let fixed_particles = if !fixed_indices.is_empty() && fixed_indices.len() == num_particles {
        VertexSelectionInput::all()
    } else if fixed_indices.is_empty() {
        VertexSelectionInput::none()
    } else {
        VertexSelectionInput {
            kind: 2,
            channel_name: "FixedParticles".to_string(),
        }
    };

    Ok(SimClothSetupObject {
        name: scd.name().to_string(),
        simulation_mesh: Some(sim_mesh),
        gravity,
        global_damping_per_second: damping,
        do_normals,
        specify_density: false,
        rescale_mass: false,
        total_mass,
        particle_mass: VertexFloatInput::constant(median_mass),
        particle_radius: VertexFloatInput::constant(median_radius),
        particle_friction: VertexFloatInput::constant(median_friction),
        fixed_particles,
        enable_pinch_detection: pinch_enabled,
        collision_tolerance,
        enable_transfer_motion: transfer_motion_enabled,
        constraint_setups,
        collidable_setups,
        ..Default::default()
    })
}

// ---------------------------------------------------------------------------
// Constraints
// ---------------------------------------------------------------------------

fn reverse_constraints(
    scd: &SimClothData<'_>,
    mode: LossyMode,
) -> Result<Vec<ConstraintSetupObject>, ReverseError> {
    scd.constraint_sets()
        .iter()
        .map(|cset| {
            let cn = cset.class_name();
            let name = get_str_default(cset, "name", cn);

            if cn == "hclStandardLinkConstraintSet" {
                let stiffness = median_link_field(cset, "links", "stiffness").unwrap_or(1.0);
                Ok(ConstraintSetupObject::StandardLink(StandardLinkSetup {
                    name,
                    stiffness: VertexFloatInput::constant(stiffness),
                    ..Default::default()
                }))
            } else if cn == "hclStretchLinkConstraintSet" {
                let stiffness = median_link_field(cset, "links", "stiffness").unwrap_or(1.0);
                Ok(ConstraintSetupObject::StretchLink(StretchLinkSetup {
                    name,
                    stiffness: VertexFloatInput::constant(stiffness),
                    ..Default::default()
                }))
            } else if cn == "hclBendStiffnessConstraintSet" {
                let mut bend = median_link_field(cset, "links", "bendStiffness").unwrap_or(0.5);
                if bend == 0.5 {
                    // Fallback to "strength" field name
                    if let Some(v) = median_link_field(cset, "links", "strength") {
                        bend = v;
                    }
                }
                let use_rest_pose = get_bool(cset, "useRestPoseConfig").unwrap_or(true);
                Ok(ConstraintSetupObject::BendStiffness(BendStiffnessSetup {
                    name,
                    bend_stiffness: VertexFloatInput::constant(bend),
                    use_rest_pose_config: use_rest_pose,
                    ..Default::default()
                }))
            } else if cn == "hclLocalRangeConstraintSet" {
                Ok(ConstraintSetupObject::LocalRange(LocalRangeSetup {
                    name,
                    ..Default::default()
                }))
            } else if mode.allow_unknown_classes_as_stubs {
                Ok(ConstraintSetupObject::Opaque(OpaqueConstraintSetup {
                    name,
                    class_name: cn.to_string(),
                    members: cset.obj.members.clone(),
                }))
            } else {
                Err(ReverseError::UnknownClass {
                    kind: "constraint",
                    class_name: cn.to_string(),
                })
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Collidables
// ---------------------------------------------------------------------------

fn reverse_collidables(scd: &SimClothData<'_>) -> Vec<CollidableSetup> {
    let ref_obj = scd.as_ref();

    // Collect transform indices for driving bone name lookup.
    let transform_indices: Vec<u32> = ref_obj
        .get_member("collidableTransformMap")
        .and_then(|m| {
            if let HkxValue::Object(members) = &m.value {
                members
                    .iter()
                    .find(|sm| sm.name == "transformIndices")
                    .map(|sm| {
                        if let HkxValue::Array(arr) = &sm.value {
                            arr.iter().filter_map(hkx_value_to_u32).collect()
                        } else {
                            vec![]
                        }
                    })
            } else {
                None
            }
        })
        .unwrap_or_default();

    scd.per_instance_collidables()
        .iter()
        .enumerate()
        .map(|(i, col)| {
            let col_name = get_str_default(col, "name", &format!("Collidable_{i}"));
            let pinch_enabled = get_bool(col, "pinchDetectionEnabled").unwrap_or(false);
            let pinch_priority = col.get_int("pinchDetectionPriority").unwrap_or(0) as i32;
            let pinch_radius = col.get_float("pinchDetectionRadius").unwrap_or(0.0);

            // Capsule shape from the "shape" pointer member.
            let shape_setup = col
                .get_member("shape")
                .and_then(|m| {
                    if let HkxValue::Pointer(Some(idx)) = &m.value {
                        col.resolve_ptr(&HkxValue::Pointer(Some(*idx)))
                    } else {
                        None
                    }
                })
                .and_then(|shape_ref| {
                    if shape_ref.class_name() == "hclCapsuleShape" {
                        let start = get_vec4_from_ref(&shape_ref, "start");
                        let end = get_vec4_from_ref(&shape_ref, "end");
                        let radius = shape_ref.get_float("radius").unwrap_or(0.0);
                        Some(CapsuleShapeSetup {
                            start,
                            end,
                            big_radius: radius,
                            small_radius: radius,
                        })
                    } else {
                        None
                    }
                });

            let driving_bone = transform_indices
                .get(i)
                .map(|idx| format!("bone_{idx}"))
                .unwrap_or_default();

            CollidableSetup {
                name: col_name,
                shape: shape_setup,
                driving_bone_name: driving_bone,
                pinch_detection_enabled: pinch_enabled,
                pinch_detection_priority: pinch_priority,
                pinch_detection_radius: pinch_radius,
                ..Default::default()
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Operators
// ---------------------------------------------------------------------------

fn reverse_operators(
    cloth_data: &ClothData<'_>,
    buffer_setups: &[BufferSetupObject],
    transform_set_setups: &[TransformSetSetupObject],
    sim_cloth_setups: &[SimClothSetupObject],
    mode: LossyMode,
) -> Result<Vec<OperatorSetupObject>, ReverseError> {
    let buf_name = |idx: usize| {
        buffer_setups
            .get(idx)
            .map(|b| b.name.clone())
            .unwrap_or_else(|| format!("buffer_{idx}"))
    };
    let ts_name = |idx: usize| {
        transform_set_setups
            .get(idx)
            .map(|t| t.name.clone())
            .unwrap_or_else(|| format!("transformset_{idx}"))
    };
    let sc_name = |idx: usize| {
        sim_cloth_setups
            .get(idx)
            .map(|s| s.name.clone())
            .unwrap_or_else(|| format!("simcloth_{idx}"))
    };

    cloth_data
        .operators()
        .iter()
        .map(|op| {
            let cn = op.class_name();
            let op_name = get_str_default(op, "name", cn);

            if cn == "hclSimulateOperator" {
                let sc_idx = get_u32(op, "simClothIndex") as usize;
                let substeps = get_u32(op, "subSteps").max(1) as usize;
                let num_solve = get_u32(op, "numberOfSolveIterations").max(1) as usize;
                let adapt = get_bool(op, "adaptConstraintStiffness").unwrap_or(false);
                let constraint_exec: Vec<String> = op
                    .get_array("constraintExecution")
                    .iter()
                    .map(|v| match v {
                        HkxValue::U32(n) => n.to_string(),
                        HkxValue::I32(n) => n.to_string(),
                        _ => String::new(),
                    })
                    .collect();

                Ok(OperatorSetupObject::Simulate(SimulateSetup {
                    name: op_name,
                    sim_cloth_setup_name: sc_name(sc_idx),
                    configs: vec![SimulateSetupConfig {
                        name: "default".to_string(),
                        num_substeps: substeps,
                        num_solve_iterations: num_solve,
                        adapt_constraint_stiffness: adapt,
                        constraint_execution_order_names: constraint_exec,
                        ..Default::default()
                    }],
                }))
            } else if cn == "hclSimpleMeshBoneDeformOperator" {
                let in_buf = get_u32(op, "inputBufferIdx") as usize;
                let out_ts = get_u32(op, "outputTransformSetIdx") as usize;
                Ok(OperatorSetupObject::MeshBoneDeform(MeshBoneDeformSetup {
                    name: op_name,
                    input_buffer_name: buf_name(in_buf),
                    output_transform_set_name: ts_name(out_ts),
                    ..Default::default()
                }))
            } else if cn == "hclObjectSpaceSkinPNOperator" {
                let out_buf = get_u32(op, "outputBufferIndex") as usize;
                let ts_idx = get_u32(op, "transformSetIndex") as usize;
                Ok(OperatorSetupObject::Skin(SkinSetup {
                    name: op_name,
                    transform_set_name: ts_name(ts_idx),
                    output_buffer_name: buf_name(out_buf),
                    skin_normals: true,
                    ..Default::default()
                }))
            } else if cn == "hclCopyVerticesOperator" {
                let in_buf = get_u32(op, "inputBufferIdx") as usize;
                let out_buf = get_u32(op, "outputBufferIdx") as usize;
                let copy_normals = get_bool(op, "copyNormals").unwrap_or(true);
                Ok(OperatorSetupObject::CopyVertices(CopyVerticesSetup {
                    name: op_name,
                    input_buffer_name: buf_name(in_buf),
                    output_buffer_name: buf_name(out_buf),
                    copy_normals,
                }))
            } else if cn == "hclMoveParticlesOperator" {
                let sc_idx = get_u32(op, "simClothIndex") as usize;
                let ref_buf = get_u32(op, "refBufferIdx") as usize;
                Ok(OperatorSetupObject::MoveParticles(MoveParticlesSetup {
                    name: op_name,
                    sim_cloth_setup_name: sc_name(sc_idx),
                    display_buffer_name: buf_name(ref_buf),
                }))
            } else if cn == "hclGatherAllVerticesOperator" {
                let in_buf = get_u32(op, "inputBufferIdx") as usize;
                let out_buf = get_u32(op, "outputBufferIdx") as usize;
                let gather_normals = get_bool(op, "gatherNormals").unwrap_or(true);
                let partial_gather = get_bool(op, "partialGather").unwrap_or(false);
                let indices: Vec<i16> = op
                    .get_array("vertexInputFromVertexOutput")
                    .iter()
                    .map(|v| match v {
                        HkxValue::I16(n) => *n,
                        HkxValue::U16(n) => *n as i16,
                        HkxValue::I32(n) => *n as i16,
                        HkxValue::U32(n) => *n as i16,
                        _ => -1,
                    })
                    .collect();
                Ok(OperatorSetupObject::GatherAllVertices(
                    GatherAllVerticesSetup {
                        name: op_name,
                        input_buffer_name: buf_name(in_buf),
                        output_buffer_name: buf_name(out_buf),
                        vertex_input_from_vertex_output: indices,
                        gather_normals,
                        partial_gather,
                    },
                ))
            } else if mode.allow_unknown_classes_as_stubs {
                Ok(OperatorSetupObject::Opaque(OpaqueOperatorSetup {
                    name: op_name,
                    class_name: cn.to_string(),
                    members: op.obj.members.clone(),
                }))
            } else {
                Err(ReverseError::UnknownClass {
                    kind: "operator",
                    class_name: cn.to_string(),
                })
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// States
// ---------------------------------------------------------------------------

fn reverse_states(cloth_data: &ClothData<'_>) -> Vec<serde_json::Value> {
    cloth_data
        .cloth_states()
        .iter()
        .map(|cs| {
            let name = get_str(cs, "name");
            let op_indices: Vec<serde_json::Value> = cs
                .get_array("operators")
                .iter()
                .map(|v| match v {
                    HkxValue::U32(n) => serde_json::Value::Number((*n).into()),
                    HkxValue::I32(n) => serde_json::Value::Number((*n).into()),
                    _ => serde_json::Value::Null,
                })
                .collect();
            serde_json::json!({
                "name": name,
                "operator_indices": op_indices,
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Internal helpers — safe member accessors on ClothObjectRef
// ---------------------------------------------------------------------------

fn get_str(obj: &ClothObjectRef<'_>, name: &str) -> String {
    obj.get_string(name).unwrap_or("").to_string()
}

fn get_str_default<'a>(obj: &ClothObjectRef<'_>, name: &str, default: &'a str) -> String {
    obj.get_string(name).unwrap_or(default).to_string()
}

fn get_u32(obj: &ClothObjectRef<'_>, name: &str) -> u32 {
    obj.get_int(name).unwrap_or(0).max(0) as u32
}

fn get_bool(obj: &ClothObjectRef<'_>, name: &str) -> Option<bool> {
    obj.get_bool(name)
}

// ---------------------------------------------------------------------------
// Struct-member helpers (for sim info sub-object fields)
// ---------------------------------------------------------------------------

/// Walk the member named `sub_name`; if its value is a Struct, find `field`
/// within its members and extract as f32.
fn get_struct_f32(obj: &ClothObjectRef<'_>, sub_name: &str, field: &str) -> Option<f32> {
    let m = obj.get_member(sub_name)?;
    if let HkxValue::Object(members) = &m.value {
        struct_float_from_members(members, field)
    } else {
        None
    }
}

fn get_struct_vec4(obj: &ClothObjectRef<'_>, sub_name: &str, field: &str) -> Option<[f32; 4]> {
    let m = obj.get_member(sub_name)?;
    if let HkxValue::Object(members) = &m.value {
        struct_vec4_from_members(members, field)
    } else {
        None
    }
}

fn struct_float_from_members(members: &[crate::hkx::HkxMember], field: &str) -> Option<f32> {
    for m in members {
        if m.name == field {
            return match &m.value {
                HkxValue::F32(v) => Some(*v),
                HkxValue::I32(v) => Some(*v as f32),
                HkxValue::U32(v) => Some(*v as f32),
                HkxValue::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
                _ => None,
            };
        }
    }
    None
}

fn struct_vec4_from_members(members: &[crate::hkx::HkxMember], field: &str) -> Option<[f32; 4]> {
    for m in members {
        if m.name == field {
            if let HkxValue::F32List(v) = &m.value {
                if v.len() >= 4 {
                    return Some([v[0], v[1], v[2], v[3]]);
                }
            }
        }
    }
    None
}

/// Extract a vec4 from a direct member of a ClothObjectRef.
fn get_vec4_from_ref(obj: &ClothObjectRef<'_>, name: &str) -> [f32; 4] {
    obj.get_member(name)
        .and_then(|m| {
            if let HkxValue::F32List(v) = &m.value {
                if v.len() >= 4 {
                    return Some([v[0], v[1], v[2], v[3]]);
                }
            }
            None
        })
        .unwrap_or([0.0; 4])
}

// ---------------------------------------------------------------------------
// Link field median — mirrors Python _median_link_field
// ---------------------------------------------------------------------------

fn median_link_field(cset: &ClothObjectRef<'_>, links_member: &str, field: &str) -> Option<f32> {
    let links = cset.get_array(links_member);
    if links.is_empty() {
        return None;
    }
    let mut values: Vec<f32> = Vec::new();
    for link in links {
        if let HkxValue::Object(members) = link {
            if let Some(v) = struct_float_from_members(members, field) {
                values.push(v);
            }
        }
    }
    median(&values)
}

// ---------------------------------------------------------------------------
// Statistical helper
// ---------------------------------------------------------------------------

fn median(values: &[f32]) -> Option<f32> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = sorted.len() / 2;
    if sorted.len() % 2 == 0 {
        Some((sorted[mid - 1] + sorted[mid]) / 2.0)
    } else {
        Some(sorted[mid])
    }
}

// ---------------------------------------------------------------------------
// HkxValue → u32 coercion helper
// ---------------------------------------------------------------------------

fn hkx_value_to_u32(v: &HkxValue) -> Option<u32> {
    match v {
        HkxValue::U8(n) => Some(u32::from(*n)),
        HkxValue::U16(n) => Some(u32::from(*n)),
        HkxValue::U32(n) => Some(*n),
        HkxValue::I32(n) => Some(*n as u32),
        HkxValue::U64(n) => Some(*n as u32),
        HkxValue::I64(n) => Some(*n as u32),
        _ => None,
    }
}
