use std::fmt;

use serde_json::{Value, json};

use super::runtime::{ClothData, ClothObjectRef, SimClothData};
use crate::hkx::types::HkxValue;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Severity::Error => write!(f, "error"),
            Severity::Warning => write!(f, "warning"),
            Severity::Info => write!(f, "info"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct LintIssue {
    pub severity: Severity,
    pub code: String,
    pub message: String,
}

impl fmt::Display for LintIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}: {}", self.severity, self.code, self.message)
    }
}

#[derive(Debug, Clone)]
pub struct ValidationResult {
    pub issues: Vec<LintIssue>,
}

impl ValidationResult {
    pub fn new() -> Self {
        Self { issues: Vec::new() }
    }

    pub fn errors(&self) -> Vec<&LintIssue> {
        self.issues
            .iter()
            .filter(|i| i.severity == Severity::Error)
            .collect()
    }

    pub fn warnings(&self) -> Vec<&LintIssue> {
        self.issues
            .iter()
            .filter(|i| i.severity == Severity::Warning)
            .collect()
    }

    pub fn infos(&self) -> Vec<&LintIssue> {
        self.issues
            .iter()
            .filter(|i| i.severity == Severity::Info)
            .collect()
    }

    pub fn is_valid(&self) -> bool {
        self.errors().is_empty()
    }

    pub fn to_summary(&self) -> Value {
        json!({
            "valid": self.is_valid(),
            "errors": self.errors().len(),
            "warnings": self.warnings().len(),
            "info": self.infos().len(),
            "issues": self.issues.iter().map(|i| json!({
                "severity": i.severity.to_string(),
                "code": i.code,
                "message": i.message,
            })).collect::<Vec<_>>(),
        })
    }

    fn push(&mut self, severity: Severity, code: &str, message: String) {
        self.issues.push(LintIssue {
            severity,
            code: code.to_string(),
            message,
        });
    }
}

// ---------------------------------------------------------------------------
// Main entry point
// ---------------------------------------------------------------------------

/// Validate a ClothData runtime graph for common issues.
///
/// Takes `Option<&ClothData>` so that a file with no `hclClothData` (e.g. an
/// animation HKX) can be passed as `None` and produces a `NO_CLOTH_DATA` error.
pub fn validate_cloth_data(cloth_data: Option<&ClothData<'_>>) -> ValidationResult {
    let mut result = ValidationResult::new();

    let cloth = match cloth_data {
        None => {
            result.push(
                Severity::Error,
                "NO_CLOTH_DATA",
                "No hclClothData found in the HKX file".to_string(),
            );
            return result;
        }
        Some(c) => c,
    };

    // Cloth name
    if cloth.name().is_empty() {
        result.push(
            Severity::Warning,
            "EMPTY_CLOTH_NAME",
            "Cloth data has no name".to_string(),
        );
    }

    // Sim cloth datas
    let sim_cloths = cloth.sim_cloth_datas();
    if sim_cloths.is_empty() {
        result.push(
            Severity::Error,
            "NO_SIM_CLOTH",
            "No simClothDatas — cloth has no simulation".to_string(),
        );
        return result;
    }

    // Cloth states
    if cloth.cloth_states().is_empty() {
        result.push(
            Severity::Error,
            "NO_CLOTH_STATES",
            "No clothStateDatas — cloth has no active states".to_string(),
        );
    }

    // Operators
    let operators = cloth.operators();
    if operators.is_empty() {
        result.push(
            Severity::Error,
            "NO_OPERATORS",
            "No operators — cloth has no operation pipeline".to_string(),
        );
    } else {
        validate_simulate_operators(&operators, &mut result);
        validate_packed_local_blocks(&operators, &mut result);
    }

    // Buffer definitions
    let buffer_definitions = cloth.buffer_definitions();
    if buffer_definitions.is_empty() {
        result.push(
            Severity::Warning,
            "NO_BUFFERS",
            "No buffer definitions — cloth cannot output to render mesh".to_string(),
        );
    } else {
        validate_buffer_layouts(&buffer_definitions, &mut result);
    }

    // Transform set definitions
    if cloth.transform_set_definitions().is_empty() {
        result.push(
            Severity::Warning,
            "NO_TRANSFORM_SETS",
            "No transform set definitions — cloth bones may not be wired".to_string(),
        );
    }

    // Per sim cloth checks
    for (si, sim) in sim_cloths.iter().enumerate() {
        validate_sim_cloth(sim, si, &mut result);
    }

    result
}

fn validate_simulate_operators(operators: &[ClothObjectRef<'_>], result: &mut ValidationResult) {
    let simulate_operators = operators
        .iter()
        .filter(|operator| operator.class_name() == "hclSimulateOperator")
        .collect::<Vec<_>>();
    if simulate_operators.is_empty() {
        result.push(
            Severity::Error,
            "NO_SIMULATE_OPERATOR",
            "No hclSimulateOperator — cloth particles will not be simulated".to_string(),
        );
        return;
    }

    for (operator_index, operator) in simulate_operators.into_iter().enumerate() {
        let configs = operator.get_array("simulateOpConfigs");
        if operator.has_member("simulateOpConfigs") {
            if configs.is_empty() {
                result.push(
                    Severity::Error,
                    "NO_SIMULATE_CONFIGS",
                    format!("simulateOperator[{operator_index}] has no solver configurations"),
                );
            }
            for (config_index, config) in configs.iter().enumerate() {
                validate_solver_settings(
                    get_int_field(config, "subSteps"),
                    get_int_field(config, "numberOfSolveIterations"),
                    &format!("simulateOperator[{operator_index}].config[{config_index}]"),
                    result,
                );
            }
        } else {
            validate_solver_settings(
                operator.get_int("subSteps"),
                operator.get_int("numberOfSolveIterations"),
                &format!("simulateOperator[{operator_index}]"),
                result,
            );
        }
    }
}

fn validate_solver_settings(
    sub_steps: Option<i64>,
    solve_iterations: Option<i64>,
    prefix: &str,
    result: &mut ValidationResult,
) {
    if sub_steps.unwrap_or(0) <= 0 {
        result.push(
            Severity::Error,
            "ZERO_SUBSTEPS",
            format!("{prefix}: subSteps must be greater than zero"),
        );
    }
    if solve_iterations.unwrap_or(0) <= 0 {
        result.push(
            Severity::Error,
            "ZERO_SOLVE_ITERATIONS",
            format!("{prefix}: numberOfSolveIterations must be greater than zero"),
        );
    }
}

fn validate_packed_local_blocks(operators: &[ClothObjectRef<'_>], result: &mut ValidationResult) {
    for (operator_index, operator) in operators.iter().enumerate() {
        let object_space = operator.class_name().contains("ObjectSpace");
        let bone_space = operator.class_name().contains("BoneSpace");
        if !object_space && !bone_space {
            continue;
        }

        for collection_name in ["localPs", "localPNs", "localPNTs", "localPNTBs"] {
            if !operator.has_member(collection_name) {
                continue;
            }
            let blocks = operator.get_array(collection_name);
            let mut any_nonzero = false;
            for (block_index, block) in blocks.iter().enumerate() {
                let Some(members) = block.as_object_members() else {
                    result.push(
                        Severity::Error,
                        "INVALID_PACKED_LOCAL_BLOCK",
                        format!(
                            "operator[{operator_index}].{collection_name}[{block_index}] is not an inline object"
                        ),
                    );
                    continue;
                };
                for component_name in [
                    "localPosition",
                    "localNormal",
                    "localTangent",
                    "localBiTangent",
                ] {
                    if bone_space && component_name == "localPosition" {
                        continue;
                    }
                    let Some(component) =
                        members.iter().find(|member| member.name == component_name)
                    else {
                        continue;
                    };
                    match packed_local_component_has_nonzero(&component.value) {
                        Ok(nonzero) => any_nonzero |= nonzero,
                        Err(error) => result.push(
                            Severity::Error,
                            "INVALID_PACKED_LOCAL_BLOCK",
                            format!(
                                "operator[{operator_index}].{collection_name}[{block_index}].{component_name}: {error}"
                            ),
                        ),
                    }
                }
            }
            if !blocks.is_empty() && !any_nonzero {
                result.push(
                    Severity::Error,
                    "INVALID_PACKED_LOCAL_BLOCK",
                    format!(
                        "operator[{operator_index}].{collection_name} contains no nonzero packed skinning data"
                    ),
                );
            }
        }
    }
}

fn packed_local_component_has_nonzero(value: &HkxValue) -> Result<bool, String> {
    let HkxValue::Array(values) = value else {
        return Err("expected an array".to_string());
    };
    if values.len() == 64 {
        if values.iter().any(|value| get_int_value(value).is_none()) {
            return Err("expected 64 packed 16-bit integers".to_string());
        }
        return Ok(values
            .iter()
            .any(|value| get_int_value(value).is_some_and(|value| value != 0)));
    }
    if values.len() != 16 {
        return Err(format!(
            "expected 64 packed integers or 16 hkPackedVector3 values, got {} entries",
            values.len()
        ));
    }

    let mut any_nonzero = false;
    for (vector_index, vector) in values.iter().enumerate() {
        let packed_values = vector
            .as_object_members()
            .and_then(|members| members.iter().find(|member| member.name == "values"))
            .and_then(|member| match &member.value {
                HkxValue::Array(values) => Some(values),
                _ => None,
            })
            .ok_or_else(|| format!("hkPackedVector3[{vector_index}] has no values array"))?;
        if packed_values.len() != 4
            || packed_values
                .iter()
                .any(|value| get_int_value(value).is_none())
        {
            return Err(format!(
                "hkPackedVector3[{vector_index}] must contain four packed integers"
            ));
        }
        any_nonzero |= packed_values
            .iter()
            .any(|value| get_int_value(value).is_some_and(|value| value != 0));
    }
    Ok(any_nonzero)
}

fn validate_buffer_layouts(buffers: &[ClothObjectRef<'_>], result: &mut ValidationResult) {
    for (index, buffer) in buffers.iter().enumerate() {
        let Some(message) = buffer_layout_error(*buffer) else {
            continue;
        };
        let name = buffer.get_string("name").unwrap_or(buffer.class_name());
        result.push(
            Severity::Error,
            "INVALID_BUFFER_LAYOUT",
            format!("buffer[{index}] {name}: {message}"),
        );
    }
}

fn buffer_layout_error(buffer: ClothObjectRef<'_>) -> Option<String> {
    let Some(layout) = buffer
        .get_member("bufferLayout")
        .and_then(|member| member.value.as_object_members())
    else {
        return Some("missing bufferLayout".to_string());
    };
    let num_slots = layout
        .iter()
        .find(|member| member.name == "numSlots")
        .and_then(|member| get_int_value(&member.value))
        .unwrap_or(0);
    if !(1..=4).contains(&num_slots) {
        return Some(format!("numSlots must be in [1, 4], got {num_slots}"));
    }

    let elements = layout
        .iter()
        .find(|member| member.name == "elementsLayout")
        .and_then(|member| match &member.value {
            HkxValue::Array(values) => Some(values.as_slice()),
            _ => None,
        })
        .unwrap_or(&[]);
    let slots = layout
        .iter()
        .find(|member| member.name == "slots")
        .and_then(|member| match &member.value {
            HkxValue::Array(values) => Some(values.as_slice()),
            _ => None,
        })
        .unwrap_or(&[]);
    if elements.len() != 4 || slots.len() != 4 {
        return Some(format!(
            "expected four element descriptors and four slots, got {} and {}",
            elements.len(),
            slots.len()
        ));
    }

    for (element_index, element) in elements.iter().enumerate() {
        let conversion = get_int_field(element, "vectorConversion").unwrap_or(250);
        let vector_size = get_int_field(element, "vectorSize").unwrap_or(0);
        if conversion == 250 {
            if vector_size != 0 {
                return Some(format!(
                    "element {element_index} uses VC_NONE with nonzero vectorSize {vector_size}"
                ));
            }
            if element_index == 0 {
                return Some("position element is absent".to_string());
            }
            continue;
        }
        if vector_size <= 0 {
            return Some(format!(
                "element {element_index} has conversion {conversion} but zero vectorSize"
            ));
        }

        let slot_id = get_int_field(element, "slotId").unwrap_or(-1);
        if slot_id < 0 || slot_id >= num_slots {
            return Some(format!(
                "element {element_index} references slot {slot_id} outside [0, {num_slots})"
            ));
        }
        let slot_start = get_int_field(element, "slotStart").unwrap_or(0);
        let stride = slots
            .get(slot_id as usize)
            .and_then(|slot| get_int_field(slot, "stride"))
            .unwrap_or(0);
        if stride <= 0 || slot_start + vector_size > stride {
            return Some(format!(
                "element {element_index} byte range {slot_start}..{} exceeds slot {slot_id} stride {stride}",
                slot_start + vector_size
            ));
        }
    }

    None
}

// ---------------------------------------------------------------------------
// Per-sim-cloth validation
// ---------------------------------------------------------------------------

fn validate_sim_cloth(sim: &SimClothData<'_>, index: usize, result: &mut ValidationResult) {
    let prefix = format!("simCloth[{index}]");

    let n_particles = sim.num_particles();
    if n_particles == 0 {
        result.push(
            Severity::Error,
            "NO_PARTICLES",
            format!("{prefix}: no particles — simulation is empty"),
        );
        return;
    }

    // Zero-mass movable particle check
    let fixed_indices = sim.fixed_particle_indices();
    let fixed_set: std::collections::HashSet<u32> = fixed_indices.iter().copied().collect();
    let mut zero_mass_movable: Vec<usize> = Vec::new();
    for (pi, particle) in sim.particles().iter().enumerate() {
        if !fixed_set.contains(&(pi as u32)) {
            if let Some(inv_mass) = get_f32_field(particle, "invMass") {
                if inv_mass == 0.0 {
                    zero_mass_movable.push(pi);
                }
            }
        }
    }
    if !zero_mass_movable.is_empty() {
        let indices_str = format_indices(&zero_mass_movable, 5);
        result.push(
            Severity::Warning,
            "ZERO_MASS_MOVABLE",
            format!(
                "{prefix}: {} movable particle(s) with zero invMass \
                 (effectively pinned but not in fixedParticles): indices {}",
                zero_mass_movable.len(),
                indices_str,
            ),
        );
    }

    // Fixed particle checks
    if fixed_indices.is_empty() {
        result.push(
            Severity::Warning,
            "NO_FIXED_PARTICLES",
            format!("{prefix}: no fixed particles — cloth will fall freely under gravity"),
        );
    }

    // Out-of-range fixed indices
    let bad_fixed: Vec<u32> = fixed_indices
        .iter()
        .copied()
        .filter(|&i| i as usize >= n_particles)
        .collect();
    if !bad_fixed.is_empty() {
        let s: Vec<String> = bad_fixed.iter().take(5).map(|i| i.to_string()).collect();
        result.push(
            Severity::Error,
            "INVALID_FIXED_INDEX",
            format!(
                "{prefix}: fixed particle indices out of range: [{}]",
                s.join(", ")
            ),
        );
    }

    // All particles fixed
    if fixed_indices.len() >= n_particles {
        result.push(
            Severity::Warning,
            "ALL_PARTICLES_FIXED",
            format!("{prefix}: all {n_particles} particles are fixed — nothing to simulate"),
        );
    }

    // Constraint sets
    let constraint_sets = sim.constraint_sets();
    if constraint_sets.is_empty() {
        result.push(
            Severity::Error,
            "NO_CONSTRAINTS",
            format!("{prefix}: no constraint sets — cloth will have no structural integrity"),
        );
    } else {
        validate_constraints(&constraint_sets, &prefix, n_particles, result);
    }

    // Collidables
    if sim.per_instance_collidables().is_empty() {
        result.push(
            Severity::Info,
            "NO_COLLIDABLES",
            format!("{prefix}: no collidables — cloth will clip through body"),
        );
    }

    // Default pose
    if sim.default_pose().is_none() {
        result.push(
            Severity::Warning,
            "NO_DEFAULT_POSE",
            format!("{prefix}: no default cloth pose — animation blending may not work"),
        );
    }
}

// ---------------------------------------------------------------------------
// Constraint validation
// ---------------------------------------------------------------------------

fn validate_constraints(
    constraint_sets: &[super::runtime::base::ClothObjectRef<'_>],
    prefix: &str,
    n_particles: usize,
    result: &mut ValidationResult,
) {
    let mut has_standard = false;
    let mut total_links: usize = 0;

    for cs in constraint_sets {
        let cls_name = cs.class_name().to_string();

        if cls_name.contains("StandardLink") {
            has_standard = true;
            let links = cs.get_array("links");
            total_links += links.len();
            check_link_indices(links, n_particles, prefix, &cls_name, result);
        } else if cls_name.contains("StretchLink") {
            let links = cs.get_array("links");
            total_links += links.len();
            check_link_indices(links, n_particles, prefix, &cls_name, result);
        } else if cls_name.contains("BendStiffness") {
            let links = cs.get_array("links");
            total_links += links.len();
            check_link_indices(links, n_particles, prefix, &cls_name, result);
        }
    }

    if !has_standard {
        result.push(
            Severity::Warning,
            "NO_STANDARD_LINKS",
            format!("{prefix}: no StandardLink constraint set — cloth may lack structural edges"),
        );
    }

    if total_links == 0 {
        result.push(
            Severity::Error,
            "EMPTY_CONSTRAINTS",
            format!("{prefix}: constraint sets exist but contain zero links"),
        );
    }
}

fn check_link_indices(
    links: &[HkxValue],
    n_particles: usize,
    prefix: &str,
    cls_name: &str,
    result: &mut ValidationResult,
) {
    for (li, link) in links.iter().enumerate() {
        let pa = get_int_field(link, "particleA");
        let pb = get_int_field(link, "particleB");

        if let Some(pa) = pa {
            if pa < 0 || pa as usize >= n_particles {
                result.push(
                    Severity::Error,
                    "LINK_INDEX_OOB",
                    format!(
                        "{prefix}/{cls_name}: link[{li}].particleA={pa} out of range [0, {n_particles})"
                    ),
                );
                return; // don't spam
            }
        }
        if let Some(pb) = pb {
            if pb < 0 || pb as usize >= n_particles {
                result.push(
                    Severity::Error,
                    "LINK_INDEX_OOB",
                    format!(
                        "{prefix}/{cls_name}: link[{li}].particleB={pb} out of range [0, {n_particles})"
                    ),
                );
                return;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Field extraction helpers
// ---------------------------------------------------------------------------

fn get_f32_field(value: &HkxValue, field_name: &str) -> Option<f32> {
    match value {
        HkxValue::Object(members) => {
            for m in members {
                if m.name == field_name {
                    return match &m.value {
                        HkxValue::F32(v) => Some(*v),
                        _ => None,
                    };
                }
            }
            None
        }
        _ => None,
    }
}

fn get_int_field(value: &HkxValue, field_name: &str) -> Option<i64> {
    for member in value.as_object_members()? {
        if member.name == field_name {
            return get_int_value(&member.value);
        }
    }
    None
}

fn get_int_value(value: &HkxValue) -> Option<i64> {
    match value {
        HkxValue::I8(value) => Some(i64::from(*value)),
        HkxValue::U8(value) => Some(i64::from(*value)),
        HkxValue::I16(value) => Some(i64::from(*value)),
        HkxValue::U16(value) => Some(i64::from(*value)),
        HkxValue::I32(value) => Some(i64::from(*value)),
        HkxValue::U32(value) => Some(i64::from(*value)),
        HkxValue::I64(value) => Some(*value),
        HkxValue::U64(value) => Some(*value as i64),
        _ => None,
    }
}

fn format_indices(indices: &[usize], max_display: usize) -> String {
    let shown: Vec<String> = indices
        .iter()
        .take(max_display)
        .map(|i| i.to_string())
        .collect();
    let suffix = if indices.len() > max_display {
        "..."
    } else {
        ""
    };
    format!("[{}]{suffix}", shown.join(", "))
}
