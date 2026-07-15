use std::collections::HashMap;

use serde::Serialize;

use crate::hkx::types::HkxValue;
use crate::hkx::{HkxFile, HkxMember, HkxObject};

use super::error::{BehaviorEvalError, BehaviorEvalResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VariableKind {
    Bool,
    Int,
    Real,
    Unsupported(i32),
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub enum VariableValue {
    Bool(bool),
    Int(i32),
    Real(f32),
}

impl VariableValue {
    pub(crate) fn type_name(self) -> &'static str {
        match self {
            Self::Bool(_) => "bool",
            Self::Int(_) => "int",
            Self::Real(_) => "real",
        }
    }

    pub(crate) fn as_bool(self) -> Option<bool> {
        match self {
            Self::Bool(value) => Some(value),
            _ => None,
        }
    }

    pub(crate) fn as_i32(self) -> Option<i32> {
        match self {
            Self::Int(value) => Some(value),
            _ => None,
        }
    }

    pub(crate) fn as_f32(self) -> Option<f32> {
        match self {
            Self::Real(value) => Some(value),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AnimationPackfile<'a> {
    pub name: &'a str,
    pub packfile: &'a HkxFile,
}

impl<'a> AnimationPackfile<'a> {
    pub fn new(name: &'a str, packfile: &'a HkxFile) -> Self {
        Self { name, packfile }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GeneratorSelector {
    GraphRoot,
    ObjectIndex(usize),
    Name(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum PathAction {
    SetVariable {
        name: String,
        value: VariableValue,
    },
    SetState {
        state_machine: GeneratorSelector,
        state_id: i32,
    },
    SendEvent {
        name: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct LoadOptions {
    pub root: GeneratorSelector,
    pub actions: Vec<PathAction>,
    pub root_motion_projection: RootMotionProjection,
}

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub enum RootMotionProjection {
    #[default]
    Vector,
    MagnitudeOnly,
}

impl Default for LoadOptions {
    fn default() -> Self {
        Self {
            root: GeneratorSelector::GraphRoot,
            actions: Vec::new(),
            root_motion_projection: RootMotionProjection::Vector,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct RootTranslation {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl RootTranslation {
    pub const ZERO: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };

    pub fn to_bits(self) -> [u32; 3] {
        [self.x.to_bits(), self.y.to_bits(), self.z.to_bits()]
    }

    pub(crate) fn add(self, rhs: Self) -> Self {
        Self {
            x: self.x + rhs.x,
            y: self.y + rhs.y,
            z: self.z + rhs.z,
        }
    }

    pub(crate) fn sub(self, rhs: Self) -> Self {
        Self {
            x: self.x - rhs.x,
            y: self.y - rhs.y,
            z: self.z - rhs.z,
        }
    }

    pub(crate) fn scale(self, weight: f32) -> Self {
        Self {
            x: self.x * weight,
            y: self.y * weight,
            z: self.z * weight,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct VariableSlot {
    pub name: String,
    pub kind: VariableKind,
    pub value: VariableValue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Binding {
    pub member_path: String,
    pub variable_index: usize,
    pub bit_index: i32,
    pub binding_type: i32,
}

#[derive(Debug, Clone)]
pub(crate) struct ReferenceFrame {
    pub duration: f32,
    pub samples: Vec<RootTranslation>,
}

impl ReferenceFrame {
    pub fn sample(&self, time: f32) -> RootTranslation {
        if self.samples.is_empty() || self.duration <= 0.0 {
            return RootTranslation::ZERO;
        }
        if self.samples.len() == 1 {
            return self.samples[0];
        }

        let clamped = time.clamp(0.0, self.duration);
        let frame = (clamped / self.duration) * ((self.samples.len() - 1) as f32);
        let lower = (frame as usize).min(self.samples.len() - 1);
        let upper = (lower + 1).min(self.samples.len() - 1);
        let fraction = frame - lower as f32;
        let a = self.samples[lower];
        let b = self.samples[upper];
        RootTranslation {
            x: a.x + (b.x - a.x) * fraction,
            y: a.y + (b.y - a.y) * fraction,
            z: a.z + (b.z - a.z) * fraction,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct LoadedGraph {
    pub objects: Vec<HkxObject>,
    pub graph_root: usize,
    pub variables: Vec<VariableSlot>,
    pub variable_indices: HashMap<String, usize>,
    pub event_names: Vec<String>,
    pub event_indices: HashMap<String, i32>,
    pub animations: HashMap<String, ReferenceFrame>,
}

impl LoadedGraph {
    pub fn load(
        behavior: &HkxFile,
        animations: &[AnimationPackfile<'_>],
    ) -> BehaviorEvalResult<Self> {
        let objects = behavior.objects().to_vec();
        let behavior_graph_index = objects
            .iter()
            .position(|object| object.class_name == "hkbBehaviorGraph")
            .ok_or_else(|| {
                BehaviorEvalError::InvalidGraph(
                    "packfile does not contain hkbBehaviorGraph".to_string(),
                )
            })?;
        let behavior_graph = &objects[behavior_graph_index];
        let graph_root = required_pointer(behavior_graph, "rootGenerator")?;
        let data_index = required_pointer(behavior_graph, "data")?;
        let data = object_at(&objects, data_index)?;
        let string_data_index = required_pointer(data, "stringData")?;
        let string_data = object_at(&objects, string_data_index)?;
        let variable_names = string_array(member_value(string_data, "variableNames"));
        let event_names = string_array(member_value(string_data, "eventNames"));

        let variable_info_values = array(member_value(data, "variableInfos"));
        let initial_values_index = required_pointer(data, "variableInitialValues")?;
        let initial_values = object_at(&objects, initial_values_index)?;
        let words = array(member_value(initial_values, "wordVariableValues"));

        if variable_info_values.len() < variable_names.len() {
            return Err(BehaviorEvalError::InvalidGraph(format!(
                "{} variable names but only {} variable infos",
                variable_names.len(),
                variable_info_values.len()
            )));
        }

        let mut variables = Vec::with_capacity(variable_names.len());
        for (index, name) in variable_names.iter().enumerate() {
            let info_members = object_members(&variable_info_values[index]).ok_or_else(|| {
                BehaviorEvalError::InvalidGraph(format!(
                    "variable info {index} is not an inline object"
                ))
            })?;
            let variable_type = members_i32(info_members, "type").ok_or_else(|| {
                BehaviorEvalError::InvalidGraph(format!(
                    "variable info {index} has no numeric type"
                ))
            })?;
            let kind = match variable_type {
                0 => VariableKind::Bool,
                1..=3 => VariableKind::Int,
                4 => VariableKind::Real,
                _ => VariableKind::Unsupported(variable_type),
            };
            let raw = words
                .get(index)
                .and_then(object_members)
                .and_then(|members| members_i32(members, "value"))
                .unwrap_or(0);
            let value = match kind {
                VariableKind::Bool => VariableValue::Bool(raw != 0),
                VariableKind::Int => VariableValue::Int(raw),
                VariableKind::Real => VariableValue::Real(f32::from_bits(raw as u32)),
                VariableKind::Unsupported(_) => VariableValue::Int(raw),
            };
            variables.push(VariableSlot {
                name: name.clone(),
                kind,
                value,
            });
        }

        let variable_indices = variables
            .iter()
            .enumerate()
            .map(|(index, slot)| (slot.name.to_ascii_lowercase(), index))
            .collect();
        let event_indices = event_names
            .iter()
            .enumerate()
            .map(|(index, name)| (name.to_ascii_lowercase(), index as i32))
            .collect();

        let mut loaded_animations = HashMap::new();
        for source in animations {
            loaded_animations.insert(
                normalize_animation_name(source.name),
                parse_reference_frame(source.packfile)?,
            );
        }

        Ok(Self {
            objects,
            graph_root,
            variables,
            variable_indices,
            event_names,
            event_indices,
            animations: loaded_animations,
        })
    }

    pub fn object(&self, index: usize) -> BehaviorEvalResult<&HkxObject> {
        object_at(&self.objects, index)
    }

    pub fn resolve_generator(&self, selector: &GeneratorSelector) -> BehaviorEvalResult<usize> {
        match selector {
            GeneratorSelector::GraphRoot => Ok(self.graph_root),
            GeneratorSelector::ObjectIndex(index) => {
                self.object(*index)?;
                Ok(*index)
            }
            GeneratorSelector::Name(name) => {
                let matches: Vec<usize> = self
                    .objects
                    .iter()
                    .enumerate()
                    .filter(|(_, object)| object_string(object, "name") == Some(name.as_str()))
                    .map(|(index, _)| index)
                    .collect();
                match matches.as_slice() {
                    [index] => Ok(*index),
                    [] => Err(BehaviorEvalError::UnknownGenerator(name.clone())),
                    _ => Err(BehaviorEvalError::AmbiguousGenerator(name.clone())),
                }
            }
        }
    }

    pub fn bindings(&self, object_index: usize) -> BehaviorEvalResult<Vec<Binding>> {
        let object = self.object(object_index)?;
        let Some(binding_set_index) = optional_pointer(object, "variableBindingSet") else {
            return Ok(Vec::new());
        };
        let binding_set = self.object(binding_set_index)?;
        if binding_set.class_name != "hkbVariableBindingSet" {
            return Err(BehaviorEvalError::InvalidGraph(format!(
                "object {binding_set_index} referenced as a variable binding set is {}",
                binding_set.class_name
            )));
        }
        let mut result = Vec::new();
        for value in array(member_value(binding_set, "bindings")) {
            let Some(members) = object_members(value) else {
                continue;
            };
            let Some(member_path) = members_string(members, "memberPath") else {
                continue;
            };
            let variable_index = members_i32(members, "variableIndex").unwrap_or(-1);
            if variable_index < 0 {
                continue;
            }
            result.push(Binding {
                member_path: member_path.to_string(),
                variable_index: variable_index as usize,
                bit_index: members_i32(members, "bitIndex").unwrap_or(-1),
                binding_type: members_i32(members, "bindingType").unwrap_or(0),
            });
        }
        Ok(result)
    }
}

fn parse_reference_frame(packfile: &HkxFile) -> BehaviorEvalResult<ReferenceFrame> {
    let objects = packfile.objects();
    let animation = objects
        .iter()
        .find(|object| {
            object.class_name.starts_with("hka")
                && object.class_name.contains("Animation")
                && object.class_name != "hkaAnimationContainer"
                && object.class_name != "hkaAnimationBinding"
        })
        .ok_or_else(|| {
            BehaviorEvalError::InvalidGraph(
                "animation packfile does not contain an hka animation".to_string(),
            )
        })?;
    let duration = object_f32(animation, "duration").unwrap_or(0.0);
    let Some(reference_index) = optional_pointer(animation, "extractedMotion") else {
        return Ok(ReferenceFrame {
            duration,
            samples: Vec::new(),
        });
    };
    let reference = object_at(objects, reference_index)?;
    if reference.class_name != "hkaDefaultAnimatedReferenceFrame" {
        return Err(BehaviorEvalError::UnsupportedActiveClass {
            object_index: reference_index,
            class_name: reference.class_name.clone(),
        });
    }
    let reference_duration = object_f32(reference, "duration").unwrap_or(duration);
    let samples = array(member_value(reference, "referenceFrameSamples"))
        .iter()
        .filter_map(|value| match value {
            HkxValue::F32List(values) if values.len() >= 3 => Some(RootTranslation {
                x: values[0],
                y: values[1],
                z: values[2],
            }),
            _ => None,
        })
        .collect();
    Ok(ReferenceFrame {
        duration: reference_duration,
        samples,
    })
}

pub(crate) fn normalize_animation_name(name: &str) -> String {
    let normalized = name.replace('/', "\\").to_ascii_lowercase();
    normalized
        .strip_suffix(".hkt")
        .or_else(|| normalized.strip_suffix(".hkx"))
        .unwrap_or(&normalized)
        .to_string()
}

pub(crate) fn member_value<'a>(object: &'a HkxObject, name: &str) -> Option<&'a HkxValue> {
    object
        .members
        .iter()
        .find(|member| member.name == name)
        .map(|member| &member.value)
}

pub(crate) fn object_i32(object: &HkxObject, name: &str) -> Option<i32> {
    member_value(object, name).and_then(value_i32)
}

pub(crate) fn object_f32(object: &HkxObject, name: &str) -> Option<f32> {
    member_value(object, name).and_then(value_f32)
}

pub(crate) fn object_bool(object: &HkxObject, name: &str) -> Option<bool> {
    member_value(object, name).and_then(value_bool)
}

pub(crate) fn object_string<'a>(object: &'a HkxObject, name: &str) -> Option<&'a str> {
    member_value(object, name).and_then(value_string)
}

pub(crate) fn optional_pointer(object: &HkxObject, name: &str) -> Option<usize> {
    match member_value(object, name) {
        Some(HkxValue::Pointer(index)) => *index,
        _ => None,
    }
}

pub(crate) fn required_pointer(object: &HkxObject, name: &str) -> BehaviorEvalResult<usize> {
    optional_pointer(object, name).ok_or_else(|| BehaviorEvalError::MissingMember {
        object_index: object.offset,
        class_name: object.class_name.clone(),
        member: name.to_string(),
    })
}

pub(crate) fn pointer_array(object: &HkxObject, name: &str) -> Vec<usize> {
    array(member_value(object, name))
        .iter()
        .filter_map(|value| match value {
            HkxValue::Pointer(Some(index)) => Some(*index),
            _ => None,
        })
        .collect()
}

pub(crate) fn array(value: Option<&HkxValue>) -> &[HkxValue] {
    match value {
        Some(HkxValue::Array(values)) => values,
        _ => &[],
    }
}

pub(crate) fn object_members(value: &HkxValue) -> Option<&[HkxMember]> {
    value.as_object_members()
}

pub(crate) fn members_i32(members: &[HkxMember], name: &str) -> Option<i32> {
    members
        .iter()
        .find(|member| member.name == name)
        .and_then(|member| value_i32(&member.value))
}

pub(crate) fn members_f32(members: &[HkxMember], name: &str) -> Option<f32> {
    members
        .iter()
        .find(|member| member.name == name)
        .and_then(|member| value_f32(&member.value))
}

pub(crate) fn members_string<'a>(members: &'a [HkxMember], name: &str) -> Option<&'a str> {
    members
        .iter()
        .find(|member| member.name == name)
        .and_then(|member| value_string(&member.value))
}

pub(crate) fn members_pointer(members: &[HkxMember], name: &str) -> Option<usize> {
    members
        .iter()
        .find(|member| member.name == name)
        .and_then(|member| match &member.value {
            HkxValue::Pointer(index) => *index,
            _ => None,
        })
}

pub(crate) fn value_i32(value: &HkxValue) -> Option<i32> {
    match value {
        HkxValue::Bool(value) => Some(i32::from(*value)),
        HkxValue::I8(value) => Some(*value as i32),
        HkxValue::U8(value) => Some(*value as i32),
        HkxValue::I16(value) => Some(*value as i32),
        HkxValue::U16(value) => Some(*value as i32),
        HkxValue::I32(value) => Some(*value),
        HkxValue::U32(value) => Some(*value as i32),
        HkxValue::I64(value) => i32::try_from(*value).ok(),
        HkxValue::U64(value) => i32::try_from(*value).ok(),
        _ => None,
    }
}

pub(crate) fn value_f32(value: &HkxValue) -> Option<f32> {
    match value {
        HkxValue::F32(value) | HkxValue::Half(value) => Some(*value),
        _ => value_i32(value).map(|value| value as f32),
    }
}

pub(crate) fn value_bool(value: &HkxValue) -> Option<bool> {
    match value {
        HkxValue::Bool(value) => Some(*value),
        _ => value_i32(value).map(|value| value != 0),
    }
}

pub(crate) fn value_string(value: &HkxValue) -> Option<&str> {
    match value {
        HkxValue::String { value, .. } => Some(value),
        _ => None,
    }
}

fn string_array(value: Option<&HkxValue>) -> Vec<String> {
    array(value)
        .iter()
        .filter_map(value_string)
        .map(str::to_string)
        .collect()
}

fn object_at(objects: &[HkxObject], index: usize) -> BehaviorEvalResult<&HkxObject> {
    objects.get(index).ok_or_else(|| {
        BehaviorEvalError::InvalidGraph(format!("object reference {index} is out of range"))
    })
}
