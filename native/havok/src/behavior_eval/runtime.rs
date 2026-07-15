use std::collections::{HashMap, HashSet, VecDeque};

use crate::hkx::HkxObject;

use super::error::{BehaviorEvalError, BehaviorEvalResult};
use super::model::{
    AnimationPackfile, Binding, GeneratorSelector, LoadOptions, LoadedGraph, PathAction,
    RootMotionProjection, RootTranslation, VariableKind, VariableValue, array, member_value,
    members_f32, members_i32, members_pointer, normalize_animation_name, object_bool, object_f32,
    object_i32, object_members, object_string, optional_pointer, pointer_array, required_pointer,
};
use super::trace::{
    ActivePathTrace, AdvanceTrace, BehaviorTrace, BlenderChildTrace, BlenderTrace, ClipTrace,
    CyclicModeTrace, CyclicTrace, NodeOutputTrace, PathNodeTrace, PhaseTrace, TraceOperation,
    TransitionTrace, VariableWriteSourceTrace,
};

const PARAMETRIC_BLEND_FLAG: i32 = 1 << 4;
const CYCLIC_BLEND_FLAG: i32 = 1 << 5;
const SYNC_BLEND_FLAG: i32 = 1;
const USE_TRIGGER_INTERVAL_FLAG: i32 = 1;
const USE_INITIATE_INTERVAL_FLAG: i32 = 1 << 1;
const UNINTERRUPTIBLE_WHILE_PLAYING_FLAG: i32 = 1 << 2;
const UNINTERRUPTIBLE_WHILE_DELAYED_FLAG: i32 = 1 << 3;
const DELAY_STATE_CHANGE_FLAG: i32 = 1 << 4;
const DISABLED_TRANSITION_FLAG: i32 = 1 << 5;
const DISABLE_CONDITION_FLAG: i32 = 1 << 8;
const ALLOW_SELF_WILDCARD_FLAG: i32 = 1 << 9;
const FROM_NESTED_STATE_FLAG: i32 = 1 << 12;
const TO_NESTED_STATE_FLAG: i32 = 1 << 13;
const ABUT_AT_END_FLAG: i32 = 1 << 14;
const KNOWN_TRANSITION_FLAGS: i32 = 0x7fff;
const IGNORE_FROM_ROOT_FLAG: i32 = 1;
const SYNC_TRANSITION_FLAG: i32 = 1 << 1;
const IGNORE_TO_ROOT_FLAG: i32 = 1 << 2;
const SUPPORTED_EFFECT_FLAGS: i32 = 0x7;
const EVENT_DISPATCH_LIMIT: usize = 256;

#[derive(Debug, Clone, Copy)]
struct SyncInterval {
    old_phase: f32,
    new_phase: f32,
}

#[derive(Debug, Default)]
struct Generated {
    translation: RootTranslation,
    sync: Option<SyncInterval>,
    events: Vec<i32>,
}

#[derive(Debug, Clone)]
struct ActiveTransition {
    from_generator: usize,
    to_generator: usize,
    from_state: i32,
    to_state: i32,
    elapsed: f32,
    duration: f32,
    effect_flags: i32,
    transition_flags: i32,
}

#[derive(Debug, Clone)]
struct StateMachineRuntime {
    current_state: i32,
    transition: Option<ActiveTransition>,
}

#[derive(Debug, Clone, Copy)]
struct ClipRuntime {
    local_time: f32,
}

#[derive(Debug, Clone, Copy)]
struct CyclicRuntime {
    current: f32,
    start: f32,
    target: f32,
    elapsed: f32,
    initialized: bool,
    frozen: bool,
}

impl Default for CyclicRuntime {
    fn default() -> Self {
        Self {
            current: 0.0,
            start: 0.0,
            target: 0.0,
            elapsed: 0.0,
            initialized: false,
            frozen: false,
        }
    }
}

#[derive(Debug, Clone)]
struct TransitionCandidate {
    transition_array: usize,
    to_state: i32,
    effect: Option<usize>,
    condition: Option<usize>,
    priority: i32,
    flags: i32,
    per_state: bool,
}

#[derive(Debug, Clone, Copy)]
struct BlenderChild {
    child_object: usize,
    generator: usize,
    parameter: f32,
    world_from_model_weight: f32,
}

struct BlenderSelection {
    parameter: Option<f32>,
    children: Vec<(BlenderChild, f32)>,
}

pub struct BehaviorEvaluator {
    graph: LoadedGraph,
    root: usize,
    state_overrides: HashMap<usize, i32>,
    state_machines: HashMap<usize, StateMachineRuntime>,
    clips: HashMap<usize, ClipRuntime>,
    cyclic: HashMap<usize, CyclicRuntime>,
    active_nodes: HashSet<usize>,
    pending_events: VecDeque<i32>,
    accumulator: RootTranslation,
    trace_enabled: bool,
    trace: BehaviorTrace,
    pending_trace_operations: Vec<TraceOperation>,
    current_trace: Option<AdvanceTrace>,
    trace_path: Vec<PathNodeTrace>,
    advance_sequence: u64,
    root_motion_projection: RootMotionProjection,
}

impl BehaviorEvaluator {
    pub fn load(
        behavior: &crate::hkx::HkxFile,
        animations: &[AnimationPackfile<'_>],
        options: LoadOptions,
    ) -> BehaviorEvalResult<Self> {
        Self::load_internal(behavior, animations, options, false)
    }

    pub fn load_with_trace(
        behavior: &crate::hkx::HkxFile,
        animations: &[AnimationPackfile<'_>],
        options: LoadOptions,
    ) -> BehaviorEvalResult<Self> {
        Self::load_internal(behavior, animations, options, true)
    }

    fn load_internal(
        behavior: &crate::hkx::HkxFile,
        animations: &[AnimationPackfile<'_>],
        options: LoadOptions,
        trace_enabled: bool,
    ) -> BehaviorEvalResult<Self> {
        let graph = LoadedGraph::load(behavior, animations)?;
        let root = graph.resolve_generator(&options.root)?;
        let mut evaluator = Self {
            graph,
            root,
            state_overrides: HashMap::new(),
            state_machines: HashMap::new(),
            clips: HashMap::new(),
            cyclic: HashMap::new(),
            active_nodes: HashSet::new(),
            pending_events: VecDeque::new(),
            accumulator: RootTranslation::ZERO,
            trace_enabled,
            trace: BehaviorTrace::default(),
            pending_trace_operations: Vec::new(),
            current_trace: None,
            trace_path: Vec::new(),
            advance_sequence: 0,
            root_motion_projection: options.root_motion_projection,
        };
        for action in options.actions {
            evaluator.apply_action(action)?;
        }
        Ok(evaluator)
    }

    pub fn apply_action(&mut self, action: PathAction) -> BehaviorEvalResult<()> {
        match action {
            PathAction::SetVariable { name, value } => self.set_variable(&name, value),
            PathAction::SetState {
                state_machine,
                state_id,
            } => {
                let object_index = self.graph.resolve_generator(&state_machine)?;
                let object = self.graph.object(object_index)?;
                if object.class_name != "hkbStateMachine" {
                    return Err(BehaviorEvalError::InvalidGraph(format!(
                        "object {object_index} selected for SetState is {}",
                        object.class_name
                    )));
                }
                self.state_info(object_index, state_id)?;
                self.state_overrides.insert(object_index, state_id);
                self.record_trace_operation(TraceOperation::StateSet {
                    state_machine: object_index,
                    state: state_id,
                });
                let previous = self.state_machines.get(&object_index).cloned();
                if let Some(runtime) = self.state_machines.get_mut(&object_index) {
                    runtime.current_state = state_id;
                    runtime.transition = None;
                }
                if self.active_nodes.contains(&object_index) {
                    let mut emitted = Vec::new();
                    if let Some(previous) = previous {
                        let previous_info =
                            self.state_info(object_index, previous.current_state)?;
                        emitted
                            .extend(self.state_notify_events(previous_info, "exitNotifyEvents")?);
                        if let Some(transition) = previous.transition {
                            self.reset_subtree(transition.from_generator);
                            self.reset_subtree(transition.to_generator);
                        } else {
                            let previous_generator =
                                self.state_generator(object_index, previous.current_state)?;
                            self.reset_subtree(previous_generator);
                        }
                    }
                    let generator = self.state_generator(object_index, state_id)?;
                    self.reset_subtree(generator);
                    emitted.extend(self.activate_generator(generator)?);
                    let state_info = self.state_info(object_index, state_id)?;
                    emitted.extend(self.state_notify_events(state_info, "enterNotifyEvents")?);
                    self.pending_events.extend(emitted);
                }
                Ok(())
            }
            PathAction::SendEvent { name } => self.send_event(&name),
        }
    }

    pub fn set_variable(&mut self, name: &str, value: VariableValue) -> BehaviorEvalResult<()> {
        let Some(index) = self
            .graph
            .variable_indices
            .get(&name.to_ascii_lowercase())
            .copied()
        else {
            return Err(BehaviorEvalError::UnknownVariable(name.to_string()));
        };
        self.set_variable_index(index, value, VariableWriteSourceTrace::External)
    }

    pub fn variable(&self, name: &str) -> Option<VariableValue> {
        self.graph
            .variable_indices
            .get(&name.to_ascii_lowercase())
            .and_then(|index| self.graph.variables.get(*index))
            .map(|slot| slot.value)
    }

    pub fn send_event(&mut self, name: &str) -> BehaviorEvalResult<()> {
        let Some(event_id) = self
            .graph
            .event_indices
            .get(&name.to_ascii_lowercase())
            .copied()
        else {
            return Err(BehaviorEvalError::UnknownEvent(name.to_string()));
        };
        self.pending_events.push_back(event_id);
        Ok(())
    }

    pub fn set_trace_enabled(&mut self, enabled: bool) {
        self.trace_enabled = enabled;
        if !enabled {
            self.pending_trace_operations.clear();
            self.current_trace = None;
            self.trace_path.clear();
        }
    }

    pub fn trace_enabled(&self) -> bool {
        self.trace_enabled
    }

    pub fn trace(&self) -> &BehaviorTrace {
        &self.trace
    }

    pub fn take_trace(&mut self) -> BehaviorTrace {
        std::mem::take(&mut self.trace)
    }

    pub fn clear_trace(&mut self) {
        self.trace = BehaviorTrace::default();
        self.pending_trace_operations.clear();
        self.current_trace = None;
        self.trace_path.clear();
    }

    pub fn advance(&mut self, dt: f32) -> BehaviorEvalResult<RootTranslation> {
        if !dt.is_finite() || dt < 0.0 {
            return Err(BehaviorEvalError::InvalidTimestep(dt));
        }
        if self.trace_enabled {
            let mut trace = AdvanceTrace::new(self.advance_sequence, dt);
            trace.operations = std::mem::take(&mut self.pending_trace_operations);
            self.current_trace = Some(trace);
        }
        let result = self.advance_inner(dt);
        self.trace_path.clear();
        match result {
            Ok(translation) => {
                if let Some(mut trace) = self.current_trace.take() {
                    trace.output = translation;
                    trace.accumulator = self.accumulator;
                    self.trace.advances.push(trace);
                }
                self.advance_sequence = self.advance_sequence.wrapping_add(1);
                Ok(translation)
            }
            Err(error) => {
                self.current_trace = None;
                Err(error)
            }
        }
    }

    fn advance_inner(&mut self, dt: f32) -> BehaviorEvalResult<RootTranslation> {
        let activation_events = self.activate_generator(self.root)?;
        for event in activation_events.into_iter().rev() {
            self.pending_events.push_front(event);
        }
        self.dispatch_pending_events()?;
        let generated = self.evaluate_generator(self.root, dt, None, None)?;
        self.pending_events.extend(generated.events);
        self.accumulator = self.accumulator.add(generated.translation);
        Ok(generated.translation)
    }

    pub fn advance_repeated(
        &mut self,
        dt: f32,
        update_count: u32,
    ) -> BehaviorEvalResult<RootTranslation> {
        let mut total = RootTranslation::ZERO;
        for _ in 0..update_count {
            total = total.add(self.advance(dt)?);
        }
        Ok(total)
    }

    pub fn accumulated_root_translation(&self) -> RootTranslation {
        self.accumulator
    }

    pub fn reset_root_translation(&mut self) {
        self.accumulator = RootTranslation::ZERO;
    }

    pub fn drain_root_translation(&mut self) -> RootTranslation {
        let value = self.accumulator;
        self.accumulator = RootTranslation::ZERO;
        value
    }

    pub fn active_state(&mut self, selector: &GeneratorSelector) -> BehaviorEvalResult<i32> {
        let object_index = self.graph.resolve_generator(selector)?;
        self.ensure_state_machine(object_index)?;
        Ok(self.state_machines[&object_index].current_state)
    }

    fn set_variable_index(
        &mut self,
        index: usize,
        value: VariableValue,
        source: VariableWriteSourceTrace,
    ) -> BehaviorEvalResult<()> {
        let Some(slot) = self.graph.variables.get_mut(index) else {
            return Err(BehaviorEvalError::InvalidGraph(format!(
                "variable index {index} is out of range"
            )));
        };
        if let VariableKind::Unsupported(variable_type) = slot.kind {
            return Err(BehaviorEvalError::UnsupportedVariableType {
                name: slot.name.clone(),
                variable_type,
            });
        }
        let matches = matches!(
            (slot.kind, value),
            (VariableKind::Bool, VariableValue::Bool(_))
                | (VariableKind::Int, VariableValue::Int(_))
                | (VariableKind::Real, VariableValue::Real(_))
        );
        if !matches {
            let expected = match slot.kind {
                VariableKind::Bool => "bool",
                VariableKind::Int => "int",
                VariableKind::Real => "real",
                VariableKind::Unsupported(_) => unreachable!(),
            };
            return Err(BehaviorEvalError::VariableTypeMismatch {
                name: slot.name.clone(),
                expected,
                actual: value.type_name(),
            });
        }
        let old = slot.value;
        let name = slot.name.clone();
        slot.value = value;
        self.record_trace_operation(TraceOperation::VariableWrite {
            source,
            index,
            name,
            old,
            new: value,
        });
        Ok(())
    }

    fn dispatch_pending_events(&mut self) -> BehaviorEvalResult<()> {
        let mut dispatched = 0;
        while let Some(event_id) = self.pending_events.pop_front() {
            if dispatched >= EVENT_DISPATCH_LIMIT {
                return Err(BehaviorEvalError::EventDispatchLimit);
            }
            dispatched += 1;
            self.record_trace_operation(TraceOperation::Event {
                id: event_id,
                name: usize::try_from(event_id)
                    .ok()
                    .and_then(|index| self.graph.event_names.get(index))
                    .cloned(),
            });
            let mut visited = HashSet::new();
            let emitted = self.dispatch_event(self.root, event_id, &mut visited)?;
            self.pending_events.extend(emitted);
        }
        Ok(())
    }

    fn dispatch_event(
        &mut self,
        object_index: usize,
        event_id: i32,
        visited: &mut HashSet<usize>,
    ) -> BehaviorEvalResult<Vec<i32>> {
        if !visited.insert(object_index) {
            return Ok(Vec::new());
        }
        let object = self.graph.object(object_index)?.clone();
        match object.class_name.as_str() {
            "hkbStateMachine" => {
                self.ensure_state_machine(object_index)?;
                let current_state = self.state_machines[&object_index].current_state;
                let mut emitted = Vec::new();
                if let Some(candidate) =
                    self.find_transition(object_index, current_state, event_id)?
                {
                    emitted.extend(self.start_transition(
                        object_index,
                        current_state,
                        candidate,
                    )?);
                }

                let runtime = self.state_machines[&object_index].clone();
                if let Some(transition) = runtime.transition {
                    emitted.extend(self.dispatch_event(
                        transition.from_generator,
                        event_id,
                        visited,
                    )?);
                    emitted.extend(self.dispatch_event(
                        transition.to_generator,
                        event_id,
                        visited,
                    )?);
                } else {
                    let generator = self.state_generator(object_index, runtime.current_state)?;
                    emitted.extend(self.dispatch_event(generator, event_id, visited)?);
                }
                Ok(emitted)
            }
            "hkbBlenderGenerator" => {
                let mut emitted = Vec::new();
                for (child, weight) in self.blender_children(object_index, None)?.children {
                    if weight != 0.0 {
                        emitted.extend(self.dispatch_event(child.generator, event_id, visited)?);
                    }
                }
                Ok(emitted)
            }
            "BSCyclicBlendTransitionGenerator" => {
                let freeze = inline_event_id(&object, "EventToFreezeBlendValue");
                let cross = inline_event_id(&object, "EventToCrossBlend");
                let transition_out = inline_event_id(&object, "TransitionOutEvent");
                let transition_in = inline_event_id(&object, "TransitionInEvent");
                if freeze == Some(event_id) {
                    self.cyclic.entry(object_index).or_default().frozen = true;
                }
                if cross == Some(event_id) {
                    let target = self.cyclic_target(object_index)?;
                    self.begin_cyclic_blend(object_index, target);
                }
                if transition_out == Some(event_id) {
                    self.cyclic.entry(object_index).or_default().frozen = true;
                }
                if transition_in == Some(event_id) {
                    let target = self.cyclic_target(object_index)?;
                    self.begin_cyclic_blend(object_index, target);
                }
                let blender = required_pointer(&object, "pBlenderGenerator")?;
                self.dispatch_event(blender, event_id, visited)
            }
            "hkbModifierGenerator" => {
                let generator = required_pointer(&object, "generator")?;
                self.dispatch_event(generator, event_id, visited)
            }
            "hkbClipGenerator" => Ok(Vec::new()),
            _ => Err(BehaviorEvalError::UnsupportedActiveClass {
                object_index,
                class_name: object.class_name,
            }),
        }
    }

    fn find_transition(
        &self,
        state_machine: usize,
        current_state: i32,
        event_id: i32,
    ) -> BehaviorEvalResult<Option<TransitionCandidate>> {
        let state_info = self.state_info(state_machine, current_state)?;
        let state_object = self.graph.object(state_info)?;
        let machine_object = self.graph.object(state_machine)?;
        let mut candidates = Vec::new();
        if let Some(array_index) = optional_pointer(state_object, "transitions") {
            candidates.extend(self.transition_candidates(array_index, event_id, true)?);
        }
        if let Some(array_index) = optional_pointer(machine_object, "wildcardTransitions") {
            candidates.extend(self.transition_candidates(array_index, event_id, false)?);
        }
        let self_transition_mode = object_i32(machine_object, "selfTransitionMode").unwrap_or(0);
        if !(0..=3).contains(&self_transition_mode) {
            return Err(BehaviorEvalError::InvalidMember {
                object_index: state_machine,
                class_name: machine_object.class_name.clone(),
                member: "selfTransitionMode".to_string(),
                detail: format!("unknown self-transition mode {self_transition_mode}"),
            });
        }
        candidates.retain(|candidate| {
            if candidate.to_state != current_state {
                return true;
            }
            if !candidate.per_state && candidate.flags & ALLOW_SELF_WILDCARD_FLAG == 0 {
                return false;
            }
            self_transition_mode != 0
        });
        candidates.sort_by(|left, right| {
            right
                .priority
                .cmp(&left.priority)
                .then_with(|| right.per_state.cmp(&left.per_state))
        });
        Ok(candidates.into_iter().next())
    }

    fn transition_candidates(
        &self,
        array_index: usize,
        event_id: i32,
        per_state: bool,
    ) -> BehaviorEvalResult<Vec<TransitionCandidate>> {
        let transition_array = self.graph.object(array_index)?;
        if transition_array.class_name != "hkbStateMachineTransitionInfoArray" {
            return Err(BehaviorEvalError::InvalidGraph(format!(
                "object {array_index} referenced as transitions is {}",
                transition_array.class_name
            )));
        }
        let mut candidates = Vec::new();
        for value in array(member_value(transition_array, "transitions")) {
            let Some(members) = object_members(value) else {
                continue;
            };
            if members_i32(members, "eventId") != Some(event_id) {
                continue;
            }
            let flags = members_i32(members, "flags").unwrap_or(0);
            if flags & DISABLED_TRANSITION_FLAG != 0 {
                continue;
            }
            let Some(to_state) = members_i32(members, "toStateId") else {
                continue;
            };
            candidates.push(TransitionCandidate {
                transition_array: array_index,
                to_state,
                effect: members_pointer(members, "transition"),
                condition: members_pointer(members, "condition"),
                priority: members_i32(members, "priority").unwrap_or(0),
                flags,
                per_state,
            });
        }
        Ok(candidates)
    }

    fn start_transition(
        &mut self,
        state_machine: usize,
        from_state: i32,
        candidate: TransitionCandidate,
    ) -> BehaviorEvalResult<Vec<i32>> {
        if candidate.flags & !KNOWN_TRANSITION_FLAGS != 0 {
            return Err(self.unsupported_transition(
                candidate.transition_array,
                format!("unknown transition flags {:#x}", candidate.flags),
            ));
        }
        let unsupported_flags = [
            (USE_TRIGGER_INTERVAL_FLAG, "trigger interval"),
            (USE_INITIATE_INTERVAL_FLAG, "initiate interval"),
            (
                UNINTERRUPTIBLE_WHILE_DELAYED_FLAG,
                "uninterruptible while delayed",
            ),
            (DELAY_STATE_CHANGE_FLAG, "delayed state change"),
            (FROM_NESTED_STATE_FLAG, "from-nested-state constraint"),
            (TO_NESTED_STATE_FLAG, "to-nested-state constraint"),
            (ABUT_AT_END_FLAG, "abut-at-end transition"),
        ];
        for (flag, feature) in unsupported_flags {
            if candidate.flags & flag != 0 {
                return Err(self.unsupported_transition(candidate.transition_array, feature));
            }
        }
        if candidate.to_state == from_state {
            let mode =
                object_i32(self.graph.object(state_machine)?, "selfTransitionMode").unwrap_or(0);
            return Err(
                self.unsupported_transition(state_machine, format!("self-transition mode {mode}"))
            );
        }
        if let Some(active) = self.state_machines[&state_machine].transition.as_ref() {
            if active.transition_flags & UNINTERRUPTIBLE_WHILE_PLAYING_FLAG != 0 {
                return Ok(Vec::new());
            }
            let max_transitions = object_i32(
                self.graph.object(state_machine)?,
                "maxSimultaneousTransitions",
            )
            .unwrap_or(1);
            return Err(self.unsupported_transition(
                state_machine,
                format!("transition stacking/replacement (limit {max_transitions})"),
            ));
        }
        if let Some(condition_index) = candidate.condition
            && candidate.flags & DISABLE_CONDITION_FLAG == 0
        {
            let condition = self.graph.object(condition_index)?;
            return Err(BehaviorEvalError::UnsupportedActiveCondition {
                object_index: condition_index,
                class_name: condition.class_name.clone(),
            });
        }
        let from_generator = self.state_generator(state_machine, from_state)?;
        let to_generator = self.state_generator(state_machine, candidate.to_state)?;
        let (duration, effect_flags) = if let Some(effect_index) = candidate.effect {
            let effect = self.graph.object(effect_index)?;
            if effect.class_name != "hkbBlendingTransitionEffect" {
                return Err(BehaviorEvalError::UnsupportedActiveClass {
                    object_index: effect_index,
                    class_name: effect.class_name.clone(),
                });
            }
            let self_transition_mode = object_i32(effect, "selfTransitionMode").unwrap_or(0);
            if !(0..=3).contains(&self_transition_mode) {
                return Err(BehaviorEvalError::InvalidMember {
                    object_index: effect_index,
                    class_name: effect.class_name.clone(),
                    member: "selfTransitionMode".to_string(),
                    detail: format!("unknown self-transition mode {self_transition_mode}"),
                });
            }
            let event_mode = object_i32(effect, "eventMode").unwrap_or(0);
            if event_mode != 0 {
                return Err(self.unsupported_transition(
                    effect_index,
                    format!("transition event mode {event_mode}"),
                ));
            }
            let curve = object_i32(effect, "blendCurve").unwrap_or(0);
            if curve != 0 {
                return Err(
                    self.unsupported_transition(effect_index, format!("blend curve {curve}"))
                );
            }
            let start_fraction = object_f32(effect, "toGeneratorStartTimeFraction").unwrap_or(0.0);
            if start_fraction != 0.0 {
                return Err(self.unsupported_transition(
                    effect_index,
                    format!("to-generator start fraction {start_fraction:?}"),
                ));
            }
            let end_mode = object_i32(effect, "endMode").unwrap_or(0);
            if end_mode != 0 {
                return Err(self.unsupported_transition(
                    effect_index,
                    format!("transition end mode {end_mode}"),
                ));
            }
            let flags = object_i32(effect, "flags").unwrap_or(0);
            if flags & !SUPPORTED_EFFECT_FLAGS != 0 {
                return Err(self.unsupported_transition(
                    effect_index,
                    format!("blending-effect flags {flags:#x}"),
                ));
            }
            let alignment_bone = object_i32(effect, "alignmentBone").unwrap_or(-1);
            if alignment_bone != -1 {
                return Err(self.unsupported_transition(
                    effect_index,
                    format!("alignment bone {alignment_bone}"),
                ));
            }
            let duration = object_f32(effect, "duration").unwrap_or(0.0);
            if !duration.is_finite() || duration < 0.0 {
                return Err(BehaviorEvalError::InvalidMember {
                    object_index: effect_index,
                    class_name: effect.class_name.clone(),
                    member: "duration".to_string(),
                    detail: format!("duration must be finite and non-negative, got {duration:?}"),
                });
            }
            (duration, flags)
        } else {
            (0.0, 0)
        };

        let old_info = self.state_info(state_machine, from_state)?;
        let new_info = self.state_info(state_machine, candidate.to_state)?;
        let mut emitted = self.state_notify_events(old_info, "exitNotifyEvents")?;
        if duration == 0.0 {
            self.reset_subtree(from_generator);
        }
        self.reset_subtree(to_generator);
        emitted.extend(self.activate_generator(to_generator)?);
        emitted.extend(self.state_notify_events(new_info, "enterNotifyEvents")?);
        if let Some(event_id) = inline_event_id(
            self.graph.object(state_machine)?,
            "eventToSendWhenStateOrTransitionChanges",
        ) && event_id >= 0
        {
            emitted.push(event_id);
        }

        let runtime = self.state_machines.get_mut(&state_machine).unwrap();
        runtime.current_state = candidate.to_state;
        runtime.transition = if duration > 0.0 {
            Some(ActiveTransition {
                from_generator,
                to_generator,
                from_state,
                to_state: candidate.to_state,
                elapsed: 0.0,
                duration,
                effect_flags,
                transition_flags: candidate.flags,
            })
        } else {
            None
        };
        Ok(emitted)
    }

    fn state_notify_events(&self, state_info: usize, member: &str) -> BehaviorEvalResult<Vec<i32>> {
        let state = self.graph.object(state_info)?;
        let Some(event_array_index) = optional_pointer(state, member) else {
            return Ok(Vec::new());
        };
        let event_array = self.graph.object(event_array_index)?;
        let mut result = Vec::new();
        for value in array(member_value(event_array, "events")) {
            if let Some(members) = object_members(value)
                && let Some(event_id) = members_i32(members, "id")
                && event_id >= 0
            {
                result.push(event_id);
            }
        }
        Ok(result)
    }

    fn activate_generator(&mut self, object_index: usize) -> BehaviorEvalResult<Vec<i32>> {
        if self.active_nodes.contains(&object_index) {
            return Ok(Vec::new());
        }
        if !self.node_enabled(object_index)? {
            return Ok(Vec::new());
        }

        let object = self.graph.object(object_index)?.clone();
        self.active_nodes.insert(object_index);
        let result = match object.class_name.as_str() {
            "hkbStateMachine" => {
                self.ensure_state_machine(object_index)?;
                let state_id = self.state_machines[&object_index].current_state;
                let state_info = self.state_info(object_index, state_id)?;
                let generator = self.state_generator(object_index, state_id)?;
                let mut emitted = self.activate_generator(generator)?;
                emitted.extend(self.state_notify_events(state_info, "enterNotifyEvents")?);
                Ok(emitted)
            }
            "hkbBlenderGenerator" => {
                let mut emitted = Vec::new();
                for (child, weight) in self.blender_children(object_index, None)?.children {
                    if weight != 0.0 {
                        emitted.extend(self.activate_generator(child.generator)?);
                    }
                }
                Ok(emitted)
            }
            "BSCyclicBlendTransitionGenerator" => {
                let curve = object_i32(&object, "eBlendCurve").unwrap_or(0);
                if curve != 0 {
                    return Err(BehaviorEvalError::InvalidMember {
                        object_index,
                        class_name: object.class_name,
                        member: "eBlendCurve".to_string(),
                        detail: format!("only linear curve 0 is supported, got {curve}"),
                    });
                }
                let duration = object_f32(&object, "fTransitionDuration").unwrap_or(0.0);
                if !duration.is_finite() || duration < 0.0 {
                    return Err(BehaviorEvalError::InvalidMember {
                        object_index,
                        class_name: object.class_name,
                        member: "fTransitionDuration".to_string(),
                        detail: format!(
                            "duration must be finite and non-negative, got {duration:?}"
                        ),
                    });
                }
                let blender_index = required_pointer(&object, "pBlenderGenerator")?;
                let blender = self.graph.object(blender_index)?;
                let min = object_f32(blender, "minCyclicBlendParameter").unwrap_or(0.0);
                let max = object_f32(blender, "maxCyclicBlendParameter").unwrap_or(1.0);
                let has_transition_in =
                    inline_event_id(&object, "TransitionInEvent").is_some_and(|event| event >= 0);
                let parameter = if has_transition_in {
                    normalize_cyclic_input(
                        object_f32(&object, "fBlendParameter").unwrap_or(0.0),
                        min,
                        max,
                    )
                } else {
                    self.cyclic_target(object_index)?
                };
                self.cyclic.insert(
                    object_index,
                    CyclicRuntime {
                        current: parameter,
                        start: parameter,
                        target: parameter,
                        elapsed: duration,
                        initialized: true,
                        frozen: has_transition_in,
                    },
                );

                self.active_nodes.insert(blender_index);
                let mut emitted = Vec::new();
                for (child, weight) in self
                    .blender_children(blender_index, Some(parameter))?
                    .children
                {
                    if weight != 0.0 {
                        emitted.extend(self.activate_generator(child.generator)?);
                    }
                }
                Ok(emitted)
            }
            "hkbModifierGenerator" => {
                let modifier = required_pointer(&object, "modifier")?;
                self.apply_modifier(modifier)?;
                let generator = required_pointer(&object, "generator")?;
                self.activate_generator(generator)
            }
            "hkbClipGenerator" => Ok(Vec::new()),
            _ => Err(BehaviorEvalError::UnsupportedActiveClass {
                object_index,
                class_name: object.class_name,
            }),
        };
        if result.is_err() {
            self.active_nodes.remove(&object_index);
        }
        result
    }

    fn cyclic_target(&self, object_index: usize) -> BehaviorEvalResult<f32> {
        let object = self.graph.object(object_index)?;
        let blender_index = required_pointer(object, "pBlenderGenerator")?;
        let blender = self.graph.object(blender_index)?;
        let min = object_f32(blender, "minCyclicBlendParameter").unwrap_or(0.0);
        let max = object_f32(blender, "maxCyclicBlendParameter").unwrap_or(1.0);
        let raw = self
            .bound_f32(object_index, "fBlendParameter")?
            .unwrap_or_else(|| object_f32(object, "fBlendParameter").unwrap_or(0.0));
        Ok(normalize_cyclic_input(raw, min, max))
    }

    fn begin_cyclic_blend(&mut self, object_index: usize, target: f32) {
        let state = self.cyclic.entry(object_index).or_default();
        state.start = state.current;
        state.target = target;
        state.elapsed = 0.0;
        state.initialized = true;
        state.frozen = false;
    }

    fn unsupported_transition(
        &self,
        object_index: usize,
        feature: impl Into<String>,
    ) -> BehaviorEvalError {
        BehaviorEvalError::UnsupportedTransitionFeature {
            object_index,
            feature: feature.into(),
        }
    }

    fn evaluate_generator(
        &mut self,
        object_index: usize,
        dt: f32,
        sync: Option<SyncInterval>,
        blend_parameter_override: Option<f32>,
    ) -> BehaviorEvalResult<Generated> {
        if !self.node_enabled(object_index)? {
            self.reset_subtree(object_index);
            return Ok(Generated::default());
        }
        let mut activation_events = self.activate_generator(object_index)?;
        let object = self.graph.object(object_index)?.clone();
        let path = if self.trace_enabled {
            self.trace_path.push(self.path_node(object_index));
            self.trace_path.clone()
        } else {
            Vec::new()
        };
        let generated_result = match object.class_name.as_str() {
            "hkbStateMachine" => self.evaluate_state_machine(object_index, dt, sync),
            "hkbBlenderGenerator" => {
                self.evaluate_blender(object_index, dt, sync, blend_parameter_override)
            }
            "hkbClipGenerator" => self.evaluate_clip(object_index, dt, sync),
            "BSCyclicBlendTransitionGenerator" => self.evaluate_cyclic(object_index, dt, sync),
            "hkbModifierGenerator" => self.evaluate_modifier_generator(object_index, dt, sync),
            _ => Err(BehaviorEvalError::UnsupportedActiveClass {
                object_index,
                class_name: object.class_name,
            }),
        };
        if self.trace_enabled {
            self.trace_path.pop();
        }
        let mut generated = generated_result?;
        if !activation_events.is_empty() {
            activation_events.extend(generated.events);
            generated.events = activation_events;
        }
        if let Some(trace) = self.current_trace.as_mut() {
            trace.node_outputs.push(NodeOutputTrace {
                object: object_index,
                path,
                root_translation: generated.translation,
            });
        }
        Ok(generated)
    }

    fn evaluate_state_machine(
        &mut self,
        object_index: usize,
        dt: f32,
        sync: Option<SyncInterval>,
    ) -> BehaviorEvalResult<Generated> {
        self.ensure_state_machine(object_index)?;
        let runtime = self.state_machines[&object_index].clone();
        let Some(mut transition) = runtime.transition else {
            let generator = self.state_generator(object_index, runtime.current_state)?;
            return self.evaluate_generator(generator, dt, sync, None);
        };

        let from = self.evaluate_generator(transition.from_generator, dt, sync, None)?;
        let to_sync = if transition.effect_flags & SYNC_TRANSITION_FLAG != 0 {
            from.sync.or(sync)
        } else {
            sync
        };
        let to = self.evaluate_generator(transition.to_generator, dt, to_sync, None)?;
        let elapsed_before = transition.elapsed;
        transition.elapsed = (transition.elapsed + dt).min(transition.duration);
        let weight = if transition.duration > 0.0 {
            (transition.elapsed / transition.duration).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let from_translation = if transition.effect_flags & IGNORE_FROM_ROOT_FLAG != 0 {
            RootTranslation::ZERO
        } else {
            from.translation
        };
        let to_translation = if transition.effect_flags & IGNORE_TO_ROOT_FLAG != 0 {
            RootTranslation::ZERO
        } else {
            to.translation
        };
        let translation = from_translation
            .scale(1.0 - weight)
            .add(to_translation.scale(weight));
        let finished = transition.elapsed >= transition.duration;
        if let Some(trace) = self.current_trace.as_mut() {
            trace.transitions.push(TransitionTrace {
                state_machine: object_index,
                from_state: transition.from_state,
                to_state: transition.to_state,
                from_generator: transition.from_generator,
                to_generator: transition.to_generator,
                elapsed_before,
                elapsed: transition.elapsed,
                duration: transition.duration,
                from_weight: 1.0 - weight,
                to_weight: weight,
            });
        }
        if finished {
            self.reset_subtree(transition.from_generator);
        }
        let machine = self.state_machines.get_mut(&object_index).unwrap();
        machine.current_state = transition.to_state;
        machine.transition = if finished { None } else { Some(transition) };

        let mut events = from.events;
        events.extend(to.events);
        Ok(Generated {
            translation,
            sync: to.sync.or(from.sync),
            events,
        })
    }

    fn evaluate_blender(
        &mut self,
        object_index: usize,
        dt: f32,
        sync: Option<SyncInterval>,
        blend_parameter_override: Option<f32>,
    ) -> BehaviorEvalResult<Generated> {
        let object = self.graph.object(object_index)?.clone();
        let selection = self.blender_children(object_index, blend_parameter_override)?;
        let parameter = selection.parameter;
        let weighted_children = selection.children;
        if weighted_children.is_empty() {
            if let Some(trace) = self.current_trace.as_mut() {
                trace.blenders.push(BlenderTrace {
                    object: object_index,
                    parameter,
                    sync_master: None,
                    input_phase: sync.map(phase_trace),
                    children: Vec::new(),
                });
            }
            return Ok(Generated::default());
        }
        let all_children = pointer_array(&object, "children");
        let flags = object_i32(&object, "flags").unwrap_or(0);
        let sync_master = self
            .bound_i32(object_index, "indexOfSyncMasterChild")?
            .unwrap_or_else(|| object_i32(&object, "indexOfSyncMasterChild").unwrap_or(-1));
        let explicit_master = usize::try_from(sync_master)
            .ok()
            .and_then(|index| all_children.get(index).copied());
        let automatic_master = weighted_children
            .iter()
            .copied()
            .reduce(|best, candidate| {
                if candidate.1 > best.1 {
                    candidate
                } else {
                    best
                }
            })
            .map(|(child, _)| child.child_object);
        let master_object = (flags & SYNC_BLEND_FLAG != 0)
            .then(|| explicit_master.or(automatic_master))
            .flatten();

        let mut cached_master = None;
        let mut master_sync = sync;
        if let Some(master_child_object) = master_object {
            let child = self.parse_blender_child(master_child_object)?;
            let generated = self.evaluate_generator(child.generator, dt, sync, None)?;
            master_sync = generated.sync.or(sync);
            cached_master = Some((master_child_object, generated));
        }

        let mut translation = RootTranslation::ZERO;
        let mut events = Vec::new();
        let mut output_sync = master_sync;
        let mut child_traces = Vec::new();
        for (child, weight) in weighted_children {
            let generated =
                if cached_master.as_ref().map(|entry| entry.0) == Some(child.child_object) {
                    cached_master.take().unwrap().1
                } else {
                    self.evaluate_generator(child.generator, dt, master_sync, None)?
                };
            let root_weight = weight * child.world_from_model_weight;
            translation = translation.add(generated.translation.scale(root_weight));
            output_sync = output_sync.or(generated.sync);
            child_traces.push(BlenderChildTrace {
                child: child.child_object,
                generator: child.generator,
                weight,
                root_weight,
                phase: generated.sync.map(phase_trace),
            });
            events.extend(generated.events);
        }
        if let Some(trace) = self.current_trace.as_mut() {
            trace.blenders.push(BlenderTrace {
                object: object_index,
                parameter,
                sync_master: master_object,
                input_phase: sync.map(phase_trace),
                children: child_traces,
            });
        }

        Ok(Generated {
            translation,
            sync: output_sync,
            events,
        })
    }

    fn blender_children(
        &self,
        object_index: usize,
        blend_parameter_override: Option<f32>,
    ) -> BehaviorEvalResult<BlenderSelection> {
        let object = self.graph.object(object_index)?;
        let flags = object_i32(object, "flags").unwrap_or(0);
        let mut children = Vec::new();
        for child_index in pointer_array(object, "children") {
            children.push(self.parse_blender_child(child_index)?);
        }
        if children.is_empty() {
            return Ok(BlenderSelection {
                parameter: None,
                children: Vec::new(),
            });
        }

        if flags & PARAMETRIC_BLEND_FLAG == 0 {
            let total = children
                .iter()
                .fold(0.0_f32, |sum, child| sum + child.parameter.max(0.0));
            if total == 0.0 {
                return Ok(BlenderSelection {
                    parameter: None,
                    children: Vec::new(),
                });
            }
            return Ok(BlenderSelection {
                parameter: None,
                children: children
                    .into_iter()
                    .filter_map(|child| {
                        let weight = child.parameter.max(0.0) / total;
                        (weight != 0.0).then_some((child, weight))
                    })
                    .collect(),
            });
        }

        let mut parameter = blend_parameter_override
            .or(self.bound_f32(object_index, "blendParameter")?)
            .unwrap_or_else(|| object_f32(object, "blendParameter").unwrap_or(0.0));
        children.sort_by(|left, right| left.parameter.total_cmp(&right.parameter));
        if flags & CYCLIC_BLEND_FLAG != 0 {
            let min = object_f32(object, "minCyclicBlendParameter").unwrap_or(0.0);
            let max = object_f32(object, "maxCyclicBlendParameter").unwrap_or(1.0);
            parameter = wrap_parameter(parameter, min, max);
        }

        if parameter <= children[0].parameter {
            return Ok(BlenderSelection {
                parameter: Some(parameter),
                children: vec![(children[0], 1.0)],
            });
        }
        if parameter >= children[children.len() - 1].parameter {
            return Ok(BlenderSelection {
                parameter: Some(parameter),
                children: vec![(children[children.len() - 1], 1.0)],
            });
        }
        for pair in children.windows(2) {
            let lower = pair[0];
            let upper = pair[1];
            if parameter <= upper.parameter {
                let width = upper.parameter - lower.parameter;
                if width == 0.0 {
                    return Ok(BlenderSelection {
                        parameter: Some(parameter),
                        children: vec![(upper, 1.0)],
                    });
                }
                let upper_weight = (parameter - lower.parameter) / width;
                return Ok(BlenderSelection {
                    parameter: Some(parameter),
                    children: vec![(lower, 1.0 - upper_weight), (upper, upper_weight)],
                });
            }
        }
        Ok(BlenderSelection {
            parameter: Some(parameter),
            children: Vec::new(),
        })
    }

    fn parse_blender_child(&self, object_index: usize) -> BehaviorEvalResult<BlenderChild> {
        let object = self.graph.object(object_index)?;
        if object.class_name != "hkbBlenderGeneratorChild" {
            return Err(BehaviorEvalError::InvalidGraph(format!(
                "blender child object {object_index} is {}",
                object.class_name
            )));
        }
        Ok(BlenderChild {
            child_object: object_index,
            generator: required_pointer(object, "generator")?,
            parameter: self
                .bound_f32(object_index, "weight")?
                .unwrap_or_else(|| object_f32(object, "weight").unwrap_or(0.0)),
            world_from_model_weight: self
                .bound_f32(object_index, "worldFromModelWeight")?
                .unwrap_or_else(|| object_f32(object, "worldFromModelWeight").unwrap_or(1.0)),
        })
    }

    fn evaluate_cyclic(
        &mut self,
        object_index: usize,
        dt: f32,
        sync: Option<SyncInterval>,
    ) -> BehaviorEvalResult<Generated> {
        let object = self.graph.object(object_index)?.clone();
        let blender_index = required_pointer(&object, "pBlenderGenerator")?;
        let blender = self.graph.object(blender_index)?;
        let min = object_f32(blender, "minCyclicBlendParameter").unwrap_or(0.0);
        let max = object_f32(blender, "maxCyclicBlendParameter").unwrap_or(1.0);
        let raw_target = self
            .bound_f32(object_index, "fBlendParameter")?
            .unwrap_or_else(|| object_f32(&object, "fBlendParameter").unwrap_or(0.0));
        let target = normalize_cyclic_input(raw_target, min, max);
        let duration = object_f32(&object, "fTransitionDuration")
            .unwrap_or(0.0)
            .max(0.0);

        let state = self.cyclic.entry(object_index).or_default();
        if !state.initialized {
            state.current = target;
            state.start = target;
            state.target = target;
            state.elapsed = duration;
            state.initialized = true;
        } else if target.to_bits() != state.target.to_bits() {
            state.start = state.current;
            state.target = target;
            state.elapsed = 0.0;
        }
        if !state.frozen {
            state.elapsed = (state.elapsed + dt).min(duration);
            let weight = if duration > 0.0 {
                (state.elapsed / duration).clamp(0.0, 1.0)
            } else {
                1.0
            };
            state.current = cyclic_lerp(state.start, state.target, weight, min, max);
        }
        let parameter = state.current;
        let mode = if state.frozen {
            CyclicModeTrace::Frozen
        } else if state.elapsed < duration {
            CyclicModeTrace::Blending
        } else {
            CyclicModeTrace::Settled
        };
        let cyclic_trace = CyclicTrace {
            object: object_index,
            current: state.current,
            start: state.start,
            target: state.target,
            elapsed: state.elapsed,
            duration,
            mode,
        };
        if let Some(trace) = self.current_trace.as_mut() {
            trace.cyclic.push(cyclic_trace);
        }
        self.evaluate_generator(blender_index, dt, sync, Some(parameter))
    }

    fn evaluate_modifier_generator(
        &mut self,
        object_index: usize,
        dt: f32,
        sync: Option<SyncInterval>,
    ) -> BehaviorEvalResult<Generated> {
        let object = self.graph.object(object_index)?.clone();
        let generator = required_pointer(&object, "generator")?;
        self.evaluate_generator(generator, dt, sync, None)
    }

    fn apply_modifier(&mut self, object_index: usize) -> BehaviorEvalResult<()> {
        if !self.node_enabled(object_index)? {
            return Ok(());
        }
        let object = self.graph.object(object_index)?.clone();
        if object.class_name == "hkbModifierList" {
            for modifier in pointer_array(&object, "modifiers") {
                self.apply_modifier(modifier)?;
            }
            return Ok(());
        }
        // Direct-at changes the sampled pose only; this evaluator projects graph output to root motion.
        if object.class_name == "BSDirectAtModifier" {
            return Ok(());
        }
        if object.class_name == "hkbMirrorModifier"
            && self.root_motion_projection == RootMotionProjection::MagnitudeOnly
        {
            return Ok(());
        }
        if object.class_name != "BSAssignVariablesModifier" {
            return Err(BehaviorEvalError::UnsupportedActiveClass {
                object_index,
                class_name: object.class_name,
            });
        }
        let bindings = self.graph.bindings(object_index)?;
        for binding in bindings {
            if binding.binding_type != 0 {
                return Err(BehaviorEvalError::UnsupportedBinding {
                    object_index,
                    member_path: binding.member_path,
                    binding_type: binding.binding_type,
                });
            }
            if let Some(suffix) = binding.member_path.strip_prefix("floatVariable") {
                let value_member = format!("floatValue{suffix}");
                let value = self
                    .bound_f32(object_index, &value_member)?
                    .unwrap_or_else(|| object_f32(&object, &value_member).unwrap_or(0.0));
                self.set_assignment(binding.variable_index, VariableValue::Real(value))?;
            } else if let Some(suffix) = binding.member_path.strip_prefix("intVariable") {
                let value_member = format!("intValue{suffix}");
                let value = self
                    .bound_i32(object_index, &value_member)?
                    .unwrap_or_else(|| object_i32(&object, &value_member).unwrap_or(0));
                self.set_assignment(binding.variable_index, VariableValue::Int(value))?;
            }
        }
        Ok(())
    }

    fn set_assignment(&mut self, index: usize, value: VariableValue) -> BehaviorEvalResult<()> {
        let Some(slot) = self.graph.variables.get(index) else {
            return Err(BehaviorEvalError::InvalidGraph(format!(
                "assignment variable index {index} is out of range"
            )));
        };
        if let VariableKind::Unsupported(variable_type) = slot.kind {
            return Err(BehaviorEvalError::UnsupportedVariableType {
                name: slot.name.clone(),
                variable_type,
            });
        }
        let converted = match (slot.kind, value) {
            (VariableKind::Bool, VariableValue::Int(value)) => VariableValue::Bool(value != 0),
            _ => value,
        };
        self.set_variable_index(index, converted, VariableWriteSourceTrace::Assignment)
    }

    fn evaluate_clip(
        &mut self,
        object_index: usize,
        dt: f32,
        sync: Option<SyncInterval>,
    ) -> BehaviorEvalResult<Generated> {
        let object = self.graph.object(object_index)?.clone();
        let animation_name = object_string(&object, "animationName").unwrap_or("");
        let animation_key = normalize_animation_name(animation_name);
        let reference = self
            .graph
            .animations
            .get(&animation_key)
            .cloned()
            .ok_or_else(|| BehaviorEvalError::MissingAnimation {
                clip_object: object_index,
                animation: animation_name.to_string(),
            })?;
        let mode = object_i32(&object, "mode").unwrap_or(0);
        if mode != 0 && mode != 1 {
            return Err(BehaviorEvalError::UnsupportedClipMode { object_index, mode });
        }
        let crop_start = object_f32(&object, "cropStartAmountLocalTime")
            .unwrap_or(0.0)
            .clamp(0.0, reference.duration.max(0.0));
        let crop_end = object_f32(&object, "cropEndAmountLocalTime")
            .unwrap_or(0.0)
            .clamp(0.0, (reference.duration - crop_start).max(0.0));
        let end = (reference.duration - crop_end).max(crop_start);
        let span = end - crop_start;
        let start_time = object_f32(&object, "startTime").unwrap_or(0.0);
        let initial_time = (crop_start + start_time).clamp(crop_start, end);
        let playback_speed = self
            .bound_f32(object_index, "playbackSpeed")?
            .unwrap_or_else(|| object_f32(&object, "playbackSpeed").unwrap_or(1.0));
        let enforced_duration = self
            .bound_f32(object_index, "enforcedDuration")?
            .unwrap_or_else(|| object_f32(&object, "enforcedDuration").unwrap_or(0.0));
        if !playback_speed.is_finite() {
            return Err(BehaviorEvalError::InvalidMember {
                object_index,
                class_name: object.class_name,
                member: "playbackSpeed".to_string(),
                detail: format!("playback speed must be finite, got {playback_speed:?}"),
            });
        }
        if !enforced_duration.is_finite() || enforced_duration < 0.0 {
            return Err(BehaviorEvalError::InvalidMember {
                object_index,
                class_name: object.class_name,
                member: "enforcedDuration".to_string(),
                detail: format!(
                    "enforced duration must be finite and non-negative, got {enforced_duration:?}"
                ),
            });
        }
        let effective_playback_speed = if enforced_duration > 0.0 {
            span / enforced_duration
        } else {
            playback_speed
        };

        let old_time = self
            .clips
            .get(&object_index)
            .map(|runtime| runtime.local_time)
            .unwrap_or(initial_time);
        let (new_time, translation, old_phase, new_phase, wraps, event_segments) =
            if let Some(sync) = sync {
                let old_sync_time = phase_time(sync.old_phase, crop_start, span);
                let phase_distance = (sync.new_phase - sync.old_phase) * span;
                let advanced = advance_clip_time(
                    &reference,
                    old_sync_time,
                    phase_distance,
                    crop_start,
                    end,
                    mode == 1,
                );
                (
                    advanced.new_time,
                    advanced.translation,
                    sync.old_phase,
                    sync.new_phase,
                    advanced.wraps,
                    advanced.segments,
                )
            } else {
                let distance = dt * effective_playback_speed;
                let advanced =
                    advance_clip_time(&reference, old_time, distance, crop_start, end, mode == 1);
                let old_phase = if span > 0.0 {
                    (old_time - crop_start) / span
                } else {
                    0.0
                };
                let new_phase = old_phase + advanced.phase_delta;
                (
                    advanced.new_time,
                    advanced.translation,
                    old_phase,
                    new_phase,
                    advanced.wraps,
                    advanced.segments,
                )
            };
        self.clips.insert(
            object_index,
            ClipRuntime {
                local_time: new_time,
            },
        );
        let events = self.clip_events(&object, &event_segments, end)?;
        let path = self.trace_path.clone();
        if let Some(trace) = self.current_trace.as_mut() {
            trace.active_paths.push(ActivePathTrace {
                nodes: path.clone(),
            });
            trace.clips.push(ClipTrace {
                object: object_index,
                path,
                animation: animation_name.to_string(),
                mode,
                old_local_time: old_time,
                new_local_time: new_time,
                wraps,
                effective_rate: effective_playback_speed,
                phase: PhaseTrace {
                    old: old_phase,
                    new: new_phase,
                },
                raw_reference_delta: translation,
            });
        }
        Ok(Generated {
            translation,
            sync: Some(SyncInterval {
                old_phase,
                new_phase,
            }),
            events,
        })
    }

    fn clip_events(
        &self,
        clip: &HkxObject,
        segments: &[(f32, f32)],
        clip_end: f32,
    ) -> BehaviorEvalResult<Vec<i32>> {
        let Some(trigger_array_index) = optional_pointer(clip, "triggers") else {
            return Ok(Vec::new());
        };
        let trigger_array = self.graph.object(trigger_array_index)?;
        let mut events = Vec::new();
        for value in array(member_value(trigger_array, "triggers")) {
            let Some(members) = object_members(value) else {
                continue;
            };
            let mut local_time = members_f32(members, "localTime").unwrap_or(0.0);
            if members_i32(members, "relativeToEndOfClip").unwrap_or(0) != 0 {
                local_time = clip_end + local_time;
            }
            let event_id = members
                .iter()
                .find(|member| member.name == "event")
                .and_then(|member| object_members(&member.value))
                .and_then(|members| members_i32(members, "id"));
            let Some(event_id) = event_id else {
                continue;
            };
            if event_id < 0 {
                continue;
            }
            for (from, to) in segments {
                let crossed = if from == to {
                    local_time.to_bits() == from.to_bits()
                } else if from < to {
                    local_time > *from && local_time <= *to
                } else {
                    local_time < *from && local_time >= *to
                };
                if crossed {
                    events.push(event_id);
                }
            }
        }
        Ok(events)
    }

    fn ensure_state_machine(&mut self, object_index: usize) -> BehaviorEvalResult<()> {
        if self.state_machines.contains_key(&object_index) {
            return Ok(());
        }
        let object = self.graph.object(object_index)?;
        if object.class_name != "hkbStateMachine" {
            return Err(BehaviorEvalError::UnsupportedActiveClass {
                object_index,
                class_name: object.class_name.clone(),
            });
        }
        let start_state_mode = object_i32(object, "startStateMode").unwrap_or(0);
        if start_state_mode != 0 {
            return Err(self.unsupported_transition(
                object_index,
                format!("state-machine start mode {start_state_mode}"),
            ));
        }
        let self_transition_mode = object_i32(object, "selfTransitionMode").unwrap_or(0);
        if !(0..=3).contains(&self_transition_mode) {
            return Err(BehaviorEvalError::InvalidMember {
                object_index,
                class_name: object.class_name.clone(),
                member: "selfTransitionMode".to_string(),
                detail: format!("unknown self-transition mode {self_transition_mode}"),
            });
        }
        let state_id = if let Some(state_id) = self.state_overrides.get(&object_index).copied() {
            state_id
        } else if let Some(state_id) = self.bound_i32(object_index, "startStateId")? {
            state_id
        } else {
            object_i32(object, "startStateId").unwrap_or(0)
        };
        self.state_info(object_index, state_id)?;
        self.state_machines.insert(
            object_index,
            StateMachineRuntime {
                current_state: state_id,
                transition: None,
            },
        );
        Ok(())
    }

    fn state_info(&self, state_machine: usize, state_id: i32) -> BehaviorEvalResult<usize> {
        let object = self.graph.object(state_machine)?;
        for state_index in pointer_array(object, "states") {
            let state = self.graph.object(state_index)?;
            if object_i32(state, "stateId") == Some(state_id) {
                return Ok(state_index);
            }
        }
        Err(BehaviorEvalError::UnknownState {
            state_machine: object_string(object, "name")
                .unwrap_or("<unnamed>")
                .to_string(),
            state_id,
        })
    }

    fn state_generator(&self, state_machine: usize, state_id: i32) -> BehaviorEvalResult<usize> {
        let state_info = self.state_info(state_machine, state_id)?;
        required_pointer(self.graph.object(state_info)?, "generator")
    }

    fn node_enabled(&self, object_index: usize) -> BehaviorEvalResult<bool> {
        let object = self.graph.object(object_index)?;
        if object_bool(object, "enable") == Some(false) {
            return Ok(false);
        }
        let Some(binding_set_index) = optional_pointer(object, "variableBindingSet") else {
            return Ok(true);
        };
        let binding_set = self.graph.object(binding_set_index)?;
        let enable_index = object_i32(binding_set, "indexOfBindingToEnable").unwrap_or(-1);
        if enable_index < 0 {
            return Ok(true);
        }
        let bindings = self.graph.bindings(object_index)?;
        let Some(binding) = bindings.get(enable_index as usize) else {
            return Err(BehaviorEvalError::InvalidGraph(format!(
                "binding enable index {enable_index} is out of range at object {object_index}"
            )));
        };
        let value = self.binding_value(object_index, binding)?;
        Ok(value.as_bool().unwrap_or(false))
    }

    fn bound_f32(&self, object_index: usize, member: &str) -> BehaviorEvalResult<Option<f32>> {
        let Some(value) = self.bound_value(object_index, member)? else {
            return Ok(None);
        };
        value
            .as_f32()
            .map(Some)
            .ok_or_else(|| self.invalid_bound_type(object_index, member, "real", value.type_name()))
    }

    fn bound_i32(&self, object_index: usize, member: &str) -> BehaviorEvalResult<Option<i32>> {
        let Some(value) = self.bound_value(object_index, member)? else {
            return Ok(None);
        };
        value
            .as_i32()
            .map(Some)
            .ok_or_else(|| self.invalid_bound_type(object_index, member, "int", value.type_name()))
    }

    fn bound_value(
        &self,
        object_index: usize,
        member: &str,
    ) -> BehaviorEvalResult<Option<VariableValue>> {
        let bindings = self.graph.bindings(object_index)?;
        let Some(binding) = bindings
            .iter()
            .find(|binding| binding.member_path == member)
        else {
            return Ok(None);
        };
        self.binding_value(object_index, binding).map(Some)
    }

    fn binding_value(
        &self,
        object_index: usize,
        binding: &Binding,
    ) -> BehaviorEvalResult<VariableValue> {
        if binding.binding_type != 0 {
            return Err(BehaviorEvalError::UnsupportedBinding {
                object_index,
                member_path: binding.member_path.clone(),
                binding_type: binding.binding_type,
            });
        }
        let Some(slot) = self.graph.variables.get(binding.variable_index) else {
            return Err(BehaviorEvalError::InvalidGraph(format!(
                "binding variable index {} is out of range at object {object_index}",
                binding.variable_index
            )));
        };
        if let VariableKind::Unsupported(variable_type) = slot.kind {
            return Err(BehaviorEvalError::UnsupportedVariableType {
                name: slot.name.clone(),
                variable_type,
            });
        }
        if binding.bit_index >= 0 {
            let Some(value) = slot.value.as_i32() else {
                return Err(self.invalid_bound_type(
                    object_index,
                    &binding.member_path,
                    "int bit source",
                    slot.value.type_name(),
                ));
            };
            let bit = 1_i32.checked_shl(binding.bit_index as u32).unwrap_or(0);
            return Ok(VariableValue::Bool(value & bit != 0));
        }
        Ok(slot.value)
    }

    fn invalid_bound_type(
        &self,
        object_index: usize,
        member: &str,
        expected: &str,
        actual: &str,
    ) -> BehaviorEvalError {
        let object = &self.graph.objects[object_index];
        BehaviorEvalError::InvalidMember {
            object_index,
            class_name: object.class_name.clone(),
            member: member.to_string(),
            detail: format!("binding expects {expected}, got {actual}"),
        }
    }

    fn path_node(&self, object_index: usize) -> PathNodeTrace {
        let object = &self.graph.objects[object_index];
        PathNodeTrace {
            object: object_index,
            class: object.class_name.clone(),
            name: object_string(object, "name").map(str::to_string),
            state: self
                .state_machines
                .get(&object_index)
                .map(|runtime| runtime.current_state),
        }
    }

    fn record_trace_operation(&mut self, operation: TraceOperation) {
        if !self.trace_enabled {
            return;
        }
        if let Some(trace) = self.current_trace.as_mut() {
            trace.operations.push(operation);
        } else {
            self.pending_trace_operations.push(operation);
        }
    }

    fn reset_subtree(&mut self, root: usize) {
        let mut stack = vec![root];
        let mut visited = HashSet::new();
        while let Some(index) = stack.pop() {
            if !visited.insert(index) {
                continue;
            }
            self.active_nodes.remove(&index);
            self.state_machines.remove(&index);
            self.clips.remove(&index);
            self.cyclic.remove(&index);
            let Some(object) = self.graph.objects.get(index) else {
                continue;
            };
            match object.class_name.as_str() {
                "hkbStateMachine" => {
                    for state_info in pointer_array(object, "states") {
                        if let Some(state) = self.graph.objects.get(state_info)
                            && let Some(generator) = optional_pointer(state, "generator")
                        {
                            stack.push(generator);
                        }
                    }
                }
                "hkbBlenderGenerator" => {
                    for child in pointer_array(object, "children") {
                        if let Some(child) = self.graph.objects.get(child)
                            && let Some(generator) = optional_pointer(child, "generator")
                        {
                            stack.push(generator);
                        }
                    }
                }
                "BSCyclicBlendTransitionGenerator" => {
                    if let Some(blender) = optional_pointer(object, "pBlenderGenerator") {
                        stack.push(blender);
                    }
                }
                "hkbModifierGenerator" => {
                    if let Some(generator) = optional_pointer(object, "generator") {
                        stack.push(generator);
                    }
                }
                _ => {}
            }
        }
    }
}

fn phase_trace(interval: SyncInterval) -> PhaseTrace {
    PhaseTrace {
        old: interval.old_phase,
        new: interval.new_phase,
    }
}

#[derive(Debug)]
struct ClipAdvance {
    new_time: f32,
    translation: RootTranslation,
    phase_delta: f32,
    wraps: i32,
    segments: Vec<(f32, f32)>,
}

fn advance_clip_time(
    reference: &super::model::ReferenceFrame,
    old_time: f32,
    distance: f32,
    start: f32,
    end: f32,
    looping: bool,
) -> ClipAdvance {
    let span = end - start;
    if span <= 0.0 || distance == 0.0 {
        return ClipAdvance {
            new_time: old_time.clamp(start, end),
            translation: RootTranslation::ZERO,
            phase_delta: 0.0,
            wraps: 0,
            segments: Vec::new(),
        };
    }

    let mut current = old_time.clamp(start, end);
    let mut remaining = distance;
    let mut translation = RootTranslation::ZERO;
    let mut segments = Vec::new();
    let mut consumed = 0.0_f32;
    let mut wraps = 0_i32;
    let mut guard = 0;
    while remaining != 0.0 && guard < 1_000_000 {
        guard += 1;
        if remaining > 0.0 {
            let available = end - current;
            if remaining < available || !looping {
                let step = remaining.min(available);
                let next = current + step;
                translation =
                    translation.add(reference.sample(next).sub(reference.sample(current)));
                if next != current {
                    segments.push((current, next));
                }
                current = next;
                consumed += step;
                remaining = 0.0;
            } else {
                translation = translation.add(reference.sample(end).sub(reference.sample(current)));
                if current != end {
                    segments.push((current, end));
                }
                remaining -= available;
                consumed += available;
                current = start;
                wraps += 1;
                segments.push((start, start));
            }
        } else {
            let available = current - start;
            if -remaining < available || !looping {
                let step = (-remaining).min(available);
                let next = current - step;
                translation =
                    translation.add(reference.sample(next).sub(reference.sample(current)));
                if next != current {
                    segments.push((current, next));
                }
                current = next;
                consumed -= step;
                remaining = 0.0;
            } else {
                translation =
                    translation.add(reference.sample(start).sub(reference.sample(current)));
                if current != start {
                    segments.push((current, start));
                }
                remaining += available;
                consumed -= available;
                current = end;
                wraps -= 1;
            }
        }
        if looping && remaining != 0.0 && current == start && remaining > span {
            let loops = (remaining / span) as u32;
            if loops > 0 {
                let cycle = reference.sample(end).sub(reference.sample(start));
                for _ in 0..loops {
                    translation = translation.add(cycle);
                    segments.push((start, end));
                    segments.push((start, start));
                }
                let amount = span * loops as f32;
                remaining -= amount;
                consumed += amount;
                wraps = wraps.saturating_add(i32::try_from(loops).unwrap_or(i32::MAX));
            }
        }
    }
    ClipAdvance {
        new_time: current,
        translation,
        phase_delta: consumed / span,
        wraps,
        segments,
    }
}

fn phase_time(phase: f32, start: f32, span: f32) -> f32 {
    if span <= 0.0 {
        return start;
    }
    let wrapped = phase - phase.floor();
    start + wrapped * span
}

fn wrap_parameter(value: f32, min: f32, max: f32) -> f32 {
    let range = max - min;
    if range <= 0.0 {
        return min;
    }
    let offset = value - min;
    min + (offset - (offset / range).floor() * range)
}

fn normalize_cyclic_input(value: f32, min: f32, max: f32) -> f32 {
    // BSCyclic receives radians, while its blender children occupy the configured cyclic range.
    let span = max - min;
    if !value.is_finite() || !span.is_finite() || span <= 0.0 {
        return min;
    }
    min + value.rem_euclid(std::f32::consts::TAU) * (span / std::f32::consts::TAU)
}

fn cyclic_lerp(start: f32, target: f32, weight: f32, min: f32, max: f32) -> f32 {
    let range = max - min;
    if range <= 0.0 {
        return target;
    }
    let mut delta = target - start;
    let half = range * 0.5;
    if delta > half {
        delta -= range;
    } else if delta < -half {
        delta += range;
    }
    wrap_parameter(start + delta * weight, min, max)
}

fn inline_event_id(object: &HkxObject, member: &str) -> Option<i32> {
    member_value(object, member)
        .and_then(object_members)
        .and_then(|members| members_i32(members, "id"))
}
