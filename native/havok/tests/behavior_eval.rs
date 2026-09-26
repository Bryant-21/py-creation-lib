use havok_native::behavior_eval::{
    AnimationPackfile, BehaviorEvalError, BehaviorEvaluator, GeneratorSelector, LoadOptions,
    PathAction, RootMotionProjection, VariableValue,
};
use havok_native::hkx::types::HkxValue;
use havok_native::hkx::{HkxFile, HkxMember, HkxObject};

struct GraphFixture {
    objects: Vec<HkxObject>,
    variables: Vec<(String, i32, i32)>,
    events: Vec<String>,
}

impl GraphFixture {
    fn new() -> Self {
        let mut objects = Vec::new();
        for _ in 0..4 {
            objects.push(object("placeholder", Vec::new()));
        }
        Self {
            objects,
            variables: Vec::new(),
            events: Vec::new(),
        }
    }

    fn real_variable(&mut self, name: &str, value: f32) -> usize {
        let index = self.variables.len();
        self.variables
            .push((name.to_string(), 4, value.to_bits() as i32));
        index
    }

    fn int_variable(&mut self, name: &str, value: i32) -> usize {
        let index = self.variables.len();
        self.variables.push((name.to_string(), 3, value));
        index
    }

    fn event(&mut self, name: &str) -> i32 {
        let index = self.events.len() as i32;
        self.events.push(name.to_string());
        index
    }

    fn push(&mut self, class_name: &str, members: Vec<HkxMember>) -> usize {
        let index = self.objects.len();
        self.objects.push(object(class_name, members));
        index
    }

    fn finish(mut self, root: usize) -> HkxFile {
        self.objects[0] = object(
            "hkbBehaviorGraph",
            vec![
                member("rootGenerator", pointer(root)),
                member("data", pointer(1)),
            ],
        );
        self.objects[1] = object(
            "hkbBehaviorGraphData",
            vec![
                member(
                    "variableInfos",
                    HkxValue::Array(
                        self.variables
                            .iter()
                            .map(|(_, variable_type, _)| {
                                inline(vec![member("type", HkxValue::I8(*variable_type as i8))])
                            })
                            .collect(),
                    ),
                ),
                member("variableInitialValues", pointer(2)),
                member("stringData", pointer(3)),
            ],
        );
        self.objects[2] = object(
            "hkbVariableValueSet",
            vec![member(
                "wordVariableValues",
                HkxValue::Array(
                    self.variables
                        .iter()
                        .map(|(_, _, value)| inline(vec![member("value", HkxValue::I32(*value))]))
                        .collect(),
                ),
            )],
        );
        self.objects[3] = object(
            "hkbBehaviorGraphStringData",
            vec![
                member(
                    "variableNames",
                    HkxValue::Array(
                        self.variables
                            .iter()
                            .map(|(name, _, _)| string(name))
                            .collect(),
                    ),
                ),
                member(
                    "eventNames",
                    HkxValue::Array(self.events.iter().map(|name| string(name)).collect()),
                ),
            ],
        );
        for (index, object) in self.objects.iter_mut().enumerate() {
            object.offset = index;
            object.name = Some(format!("#{index:04}"));
        }
        HkxFile::from_tagxml(11, "hk_2014.1.0-r1", self.objects)
    }
}

fn object(class_name: &str, members: Vec<HkxMember>) -> HkxObject {
    HkxObject {
        name: None,
        offset: 0,
        signature: 0,
        class_name: class_name.to_string(),
        members,
    }
}

fn member(name: &str, value: HkxValue) -> HkxMember {
    HkxMember {
        name: name.to_string(),
        value,
    }
}

fn pointer(index: usize) -> HkxValue {
    HkxValue::Pointer(Some(index))
}

fn null_pointer() -> HkxValue {
    HkxValue::Pointer(None)
}

fn string(value: &str) -> HkxValue {
    HkxValue::String {
        value: value.to_string(),
        is_null: false,
    }
}

fn inline(members: Vec<HkxMember>) -> HkxValue {
    HkxValue::Object(members)
}

fn binding_set(fixture: &mut GraphFixture, bindings: &[(&str, usize)]) -> usize {
    fixture.push(
        "hkbVariableBindingSet",
        vec![
            member(
                "bindings",
                HkxValue::Array(
                    bindings
                        .iter()
                        .map(|(path, variable)| {
                            inline(vec![
                                member("memberPath", string(path)),
                                member("variableIndex", HkxValue::I32(*variable as i32)),
                                member("bitIndex", HkxValue::I8(-1)),
                                member("bindingType", HkxValue::I8(0)),
                            ])
                        })
                        .collect(),
                ),
            ),
            member("indexOfBindingToEnable", HkxValue::I32(-1)),
        ],
    )
}

struct ClipSpec<'a> {
    name: &'a str,
    animation: &'a str,
    binding: Option<usize>,
    triggers: Option<usize>,
    speed: f32,
    enforced_duration: f32,
    mode: i32,
    crop_start: f32,
    crop_end: f32,
    start_time: f32,
}

fn clip(fixture: &mut GraphFixture, spec: ClipSpec<'_>) -> usize {
    fixture.push(
        "hkbClipGenerator",
        vec![
            member(
                "variableBindingSet",
                spec.binding.map(pointer).unwrap_or_else(null_pointer),
            ),
            member("name", string(spec.name)),
            member("animationName", string(spec.animation)),
            member(
                "triggers",
                spec.triggers.map(pointer).unwrap_or_else(null_pointer),
            ),
            member("cropStartAmountLocalTime", HkxValue::F32(spec.crop_start)),
            member("cropEndAmountLocalTime", HkxValue::F32(spec.crop_end)),
            member("startTime", HkxValue::F32(spec.start_time)),
            member("playbackSpeed", HkxValue::F32(spec.speed)),
            member("enforcedDuration", HkxValue::F32(spec.enforced_duration)),
            member("mode", HkxValue::I8(spec.mode as i8)),
        ],
    )
}

fn linear_animation(duration: f32, end_x: f32) -> HkxFile {
    sampled_animation(duration, &[[0.0, 0.0, 0.0], [end_x, 0.0, 0.0]])
}

fn sampled_animation(duration: f32, samples: &[[f32; 3]]) -> HkxFile {
    let mut objects = vec![
        object(
            "hkaSplineCompressedAnimation",
            vec![
                member("duration", HkxValue::F32(duration)),
                member("extractedMotion", pointer(1)),
            ],
        ),
        object(
            "hkaDefaultAnimatedReferenceFrame",
            vec![
                member("duration", HkxValue::F32(duration)),
                member(
                    "referenceFrameSamples",
                    HkxValue::Array(
                        samples
                            .iter()
                            .map(|sample| {
                                HkxValue::F32List(vec![sample[0], sample[1], sample[2], 0.0])
                            })
                            .collect(),
                    ),
                ),
            ],
        ),
    ];
    for (index, object) in objects.iter_mut().enumerate() {
        object.offset = index;
    }
    HkxFile::from_tagxml(11, "hk_2014.1.0-r1", objects)
}

fn default_clip<'a>(name: &'a str, animation: &'a str) -> ClipSpec<'a> {
    ClipSpec {
        name,
        animation,
        binding: None,
        triggers: None,
        speed: 1.0,
        enforced_duration: 0.0,
        mode: 1,
        crop_start: 0.0,
        crop_end: 0.0,
        start_time: 0.0,
    }
}

fn state_info(
    fixture: &mut GraphFixture,
    name: &str,
    state_id: i32,
    generator: usize,
    transitions: Option<usize>,
    enter_events: Option<usize>,
) -> usize {
    state_info_with_notifications(
        fixture,
        name,
        state_id,
        generator,
        transitions,
        enter_events,
        None,
    )
}

fn state_info_with_notifications(
    fixture: &mut GraphFixture,
    name: &str,
    state_id: i32,
    generator: usize,
    transitions: Option<usize>,
    enter_events: Option<usize>,
    exit_events: Option<usize>,
) -> usize {
    fixture.push(
        "hkbStateMachineStateInfo",
        vec![
            member("name", string(name)),
            member("stateId", HkxValue::I32(state_id)),
            member("generator", pointer(generator)),
            member(
                "transitions",
                transitions.map(pointer).unwrap_or_else(null_pointer),
            ),
            member(
                "enterNotifyEvents",
                enter_events.map(pointer).unwrap_or_else(null_pointer),
            ),
            member(
                "exitNotifyEvents",
                exit_events.map(pointer).unwrap_or_else(null_pointer),
            ),
        ],
    )
}

fn transition_array(
    fixture: &mut GraphFixture,
    transitions: &[(i32, i32, Option<usize>)],
) -> usize {
    let transitions: Vec<_> = transitions
        .iter()
        .map(|(event, state, effect)| (*event, *state, *effect, 0))
        .collect();
    transition_array_with_flags(fixture, &transitions)
}

fn transition_array_with_flags(
    fixture: &mut GraphFixture,
    transitions: &[(i32, i32, Option<usize>, i32)],
) -> usize {
    fixture.push(
        "hkbStateMachineTransitionInfoArray",
        vec![member(
            "transitions",
            HkxValue::Array(
                transitions
                    .iter()
                    .map(|(event, state, effect, flags)| {
                        inline(vec![
                            member(
                                "triggerInterval",
                                inline(vec![
                                    member("enterEventId", HkxValue::I32(-1)),
                                    member("exitEventId", HkxValue::I32(-1)),
                                    member("enterTime", HkxValue::F32(0.0)),
                                    member("exitTime", HkxValue::F32(0.0)),
                                ]),
                            ),
                            member(
                                "initiateInterval",
                                inline(vec![
                                    member("enterEventId", HkxValue::I32(-1)),
                                    member("exitEventId", HkxValue::I32(-1)),
                                    member("enterTime", HkxValue::F32(0.0)),
                                    member("exitTime", HkxValue::F32(0.0)),
                                ]),
                            ),
                            member(
                                "transition",
                                effect.map(pointer).unwrap_or_else(null_pointer),
                            ),
                            member("condition", null_pointer()),
                            member("eventId", HkxValue::I32(*event)),
                            member("toStateId", HkxValue::I32(*state)),
                            member("fromNestedStateId", HkxValue::I32(0)),
                            member("toNestedStateId", HkxValue::I32(0)),
                            member("priority", HkxValue::I32(0)),
                            member("flags", HkxValue::I32(*flags)),
                        ])
                    })
                    .collect(),
            ),
        )],
    )
}

fn transition_effect(fixture: &mut GraphFixture, name: &str, duration: f32) -> usize {
    transition_effect_with_flags(fixture, name, duration, 0)
}

fn transition_effect_with_flags(
    fixture: &mut GraphFixture,
    name: &str,
    duration: f32,
    flags: i32,
) -> usize {
    fixture.push(
        "hkbBlendingTransitionEffect",
        vec![
            member("name", string(name)),
            member("selfTransitionMode", HkxValue::I8(1)),
            member("eventMode", HkxValue::I8(0)),
            member("duration", HkxValue::F32(duration)),
            member("toGeneratorStartTimeFraction", HkxValue::F32(0.0)),
            member("flags", HkxValue::I32(flags)),
            member("endMode", HkxValue::I8(0)),
            member("blendCurve", HkxValue::I8(0)),
            member("alignmentBone", HkxValue::I16(-1)),
        ],
    )
}

fn state_machine(
    fixture: &mut GraphFixture,
    name: &str,
    start_state: i32,
    states: &[usize],
    wildcard: Option<usize>,
) -> usize {
    fixture.push(
        "hkbStateMachine",
        vec![
            member("variableBindingSet", null_pointer()),
            member("name", string(name)),
            member("startStateId", HkxValue::I32(start_state)),
            member("maxSimultaneousTransitions", HkxValue::I32(32)),
            member("startStateMode", HkxValue::I8(0)),
            member("selfTransitionMode", HkxValue::I8(1)),
            member(
                "states",
                HkxValue::Array(states.iter().copied().map(pointer).collect()),
            ),
            member(
                "wildcardTransitions",
                wildcard.map(pointer).unwrap_or_else(null_pointer),
            ),
            member(
                "eventToSendWhenStateOrTransitionChanges",
                inline(vec![member("id", HkxValue::I32(-1))]),
            ),
        ],
    )
}

fn event_array(fixture: &mut GraphFixture, events: &[i32]) -> usize {
    fixture.push(
        "hkbStateMachineEventPropertyArray",
        vec![member(
            "events",
            HkxValue::Array(
                events
                    .iter()
                    .map(|event| inline(vec![member("id", HkxValue::I32(*event))]))
                    .collect(),
            ),
        )],
    )
}

fn trigger_array(fixture: &mut GraphFixture, triggers: &[(f32, i32)]) -> usize {
    fixture.push(
        "hkbClipTriggerArray",
        vec![member(
            "triggers",
            HkxValue::Array(
                triggers
                    .iter()
                    .map(|(local_time, event)| {
                        inline(vec![
                            member("localTime", HkxValue::F32(*local_time)),
                            member("event", inline(vec![member("id", HkxValue::I32(*event))])),
                            member("relativeToEndOfClip", HkxValue::Bool(false)),
                            member("acyclic", HkxValue::Bool(false)),
                            member("isAnnotation", HkxValue::Bool(false)),
                        ])
                    })
                    .collect(),
            ),
        )],
    )
}

fn blender_child(
    fixture: &mut GraphFixture,
    generator: usize,
    parameter: f32,
    world_weight: f32,
) -> usize {
    fixture.push(
        "hkbBlenderGeneratorChild",
        vec![
            member("variableBindingSet", null_pointer()),
            member("generator", pointer(generator)),
            member("weight", HkxValue::F32(parameter)),
            member("worldFromModelWeight", HkxValue::F32(world_weight)),
        ],
    )
}

fn blender(
    fixture: &mut GraphFixture,
    name: &str,
    binding: Option<usize>,
    parameter: f32,
    flags: i32,
    sync_master: i32,
    children: &[usize],
) -> usize {
    fixture.push(
        "hkbBlenderGenerator",
        vec![
            member(
                "variableBindingSet",
                binding.map(pointer).unwrap_or_else(null_pointer),
            ),
            member("name", string(name)),
            member("blendParameter", HkxValue::F32(parameter)),
            member("minCyclicBlendParameter", HkxValue::F32(0.0)),
            member("maxCyclicBlendParameter", HkxValue::F32(1.0)),
            member("indexOfSyncMasterChild", HkxValue::I32(sync_master)),
            member("flags", HkxValue::I32(flags)),
            member(
                "children",
                HkxValue::Array(children.iter().copied().map(pointer).collect()),
            ),
        ],
    )
}

#[test]
fn modifier_assignment_list_and_event_range_semantics() {
    {
        let mut fixture = GraphFixture::new();
        let speed = fixture.real_variable("Speed", 1.0);
        let modifier_bindings = binding_set(&mut fixture, &[("floatVariable1", speed)]);
        let clip_bindings = binding_set(&mut fixture, &[("playbackSpeed", speed)]);
        let modifier = fixture.push(
            "BSAssignVariablesModifier",
            vec![
                member("variableBindingSet", pointer(modifier_bindings)),
                member("enable", HkxValue::Bool(true)),
                member("floatVariable1", HkxValue::F32(0.0)),
                member("floatValue1", HkxValue::F32(2.0)),
            ],
        );
        let clip = clip(
            &mut fixture,
            ClipSpec {
                binding: Some(clip_bindings),
                ..default_clip("moving", "move.hkt")
            },
        );
        let root = fixture.push(
            "hkbModifierGenerator",
            vec![
                member("name", string("root")),
                member("modifier", pointer(modifier)),
                member("generator", pointer(clip)),
            ],
        );
        let behavior = fixture.finish(root);
        let animation = linear_animation(1.0, 10.0);
        let sources = [AnimationPackfile::new("move.hkt", &animation)];
        let mut evaluator =
            BehaviorEvaluator::load(&behavior, &sources, LoadOptions::default()).unwrap();

        let generated = evaluator.advance(0.25).unwrap();
        assert_eq!(generated.x.to_bits(), 5.0_f32.to_bits());
        assert_eq!(evaluator.variable("Speed"), Some(VariableValue::Real(2.0)));
        assert_eq!(
            evaluator.accumulated_root_translation().to_bits(),
            [5.0_f32.to_bits(), 0, 0]
        );
        assert_eq!(
            evaluator.drain_root_translation().x.to_bits(),
            5.0_f32.to_bits()
        );
        assert_eq!(
            evaluator.accumulated_root_translation().to_bits(),
            [0, 0, 0]
        );
        evaluator
            .set_variable("Speed", VariableValue::Real(1.0))
            .unwrap();
        assert_eq!(
            evaluator.advance(0.25).unwrap().x.to_bits(),
            2.5_f32.to_bits()
        );
        assert_eq!(evaluator.variable("Speed"), Some(VariableValue::Real(1.0)));
    }
    {
        let mut fixture = GraphFixture::new();
        let speed = fixture.real_variable("Speed", 1.0);
        let first_bindings = binding_set(&mut fixture, &[("floatVariable1", speed)]);
        let second_bindings = binding_set(&mut fixture, &[("floatVariable1", speed)]);
        let clip_bindings = binding_set(&mut fixture, &[("playbackSpeed", speed)]);
        let first = fixture.push(
            "BSAssignVariablesModifier",
            vec![
                member("variableBindingSet", pointer(first_bindings)),
                member("enable", HkxValue::Bool(true)),
                member("floatVariable1", HkxValue::F32(0.0)),
                member("floatValue1", HkxValue::F32(2.0)),
            ],
        );
        let second = fixture.push(
            "BSAssignVariablesModifier",
            vec![
                member("variableBindingSet", pointer(second_bindings)),
                member("enable", HkxValue::Bool(true)),
                member("floatVariable1", HkxValue::F32(0.0)),
                member("floatValue1", HkxValue::F32(3.0)),
            ],
        );
        let direct_at = fixture.push(
            "BSDirectAtModifier",
            vec![member("enable", HkxValue::Bool(true))],
        );
        let mirror = fixture.push(
            "hkbMirrorModifier",
            vec![member("enable", HkxValue::Bool(true))],
        );
        let modifiers = fixture.push(
            "hkbModifierList",
            vec![
                member("enable", HkxValue::Bool(true)),
                member(
                    "modifiers",
                    HkxValue::Array(vec![
                        pointer(first),
                        pointer(direct_at),
                        pointer(mirror),
                        pointer(second),
                    ]),
                ),
            ],
        );
        let moving = clip(
            &mut fixture,
            ClipSpec {
                binding: Some(clip_bindings),
                ..default_clip("moving", "move.hkt")
            },
        );
        let root = fixture.push(
            "hkbModifierGenerator",
            vec![
                member("name", string("root")),
                member("modifier", pointer(modifiers)),
                member("generator", pointer(moving)),
            ],
        );
        let behavior = fixture.finish(root);
        let animation = linear_animation(1.0, 10.0);
        let sources = [AnimationPackfile::new("move.hkt", &animation)];
        let mut vector =
            BehaviorEvaluator::load(&behavior, &sources, LoadOptions::default()).unwrap();
        assert!(matches!(
            vector.advance(0.25),
            Err(BehaviorEvalError::UnsupportedActiveClass {
                object_index,
                class_name,
            }) if object_index == mirror && class_name == "hkbMirrorModifier"
        ));

        let mut evaluator = BehaviorEvaluator::load(
            &behavior,
            &sources,
            LoadOptions {
                root_motion_projection: RootMotionProjection::MagnitudeOnly,
                ..LoadOptions::default()
            },
        )
        .unwrap();

        assert_eq!(
            evaluator.advance(0.25).unwrap().x.to_bits(),
            7.5_f32.to_bits()
        );
        assert_eq!(evaluator.variable("Speed"), Some(VariableValue::Real(3.0)));
    }
    {
        let mut fixture = GraphFixture::new();
        let modifier = fixture.push(
            "hkbEventsFromRangeModifier",
            vec![member("enable", HkxValue::Bool(true))],
        );
        let moving = clip(&mut fixture, default_clip("moving", "move.hkt"));
        let root = fixture.push(
            "hkbModifierGenerator",
            vec![
                member("name", string("root")),
                member("modifier", pointer(modifier)),
                member("generator", pointer(moving)),
            ],
        );
        let behavior = fixture.finish(root);
        let animation = linear_animation(1.0, 10.0);
        let sources = [AnimationPackfile::new("move.hkt", &animation)];

        let mut vector =
            BehaviorEvaluator::load(&behavior, &sources, LoadOptions::default()).unwrap();
        assert!(matches!(
            vector.advance(0.25),
            Err(BehaviorEvalError::UnsupportedActiveClass {
                object_index,
                class_name,
            }) if object_index == modifier && class_name == "hkbEventsFromRangeModifier"
        ));

        let mut magnitude = BehaviorEvaluator::load(
            &behavior,
            &sources,
            LoadOptions {
                root_motion_projection: RootMotionProjection::MagnitudeOnly,
                ..LoadOptions::default()
            },
        )
        .unwrap();
        assert_eq!(
            magnitude.advance(0.25).unwrap().x.to_bits(),
            2.5_f32.to_bits()
        );
    }
    {
        let mut fixture = GraphFixture::new();
        let value = fixture.real_variable("Assigned", 0.0);
        let leave = fixture.event("leave");
        let enter = fixture.event("enter");
        let bindings = binding_set(&mut fixture, &[("floatVariable1", value)]);
        let modifier = fixture.push(
            "BSAssignVariablesModifier",
            vec![
                member("variableBindingSet", pointer(bindings)),
                member("enable", HkxValue::Bool(true)),
                member("floatVariable1", HkxValue::F32(0.0)),
                member("floatValue1", HkxValue::F32(2.0)),
            ],
        );
        let active_clip = clip(&mut fixture, default_clip("active", "still.hkt"));
        let modifier_generator = fixture.push(
            "hkbModifierGenerator",
            vec![
                member("name", string("assignment")),
                member("modifier", pointer(modifier)),
                member("generator", pointer(active_clip)),
            ],
        );
        let inactive_clip = clip(&mut fixture, default_clip("inactive", "still.hkt"));
        let leave_transition = transition_array(&mut fixture, &[(leave, 1, None)]);
        let enter_transition = transition_array(&mut fixture, &[(enter, 0, None)]);
        let state0 = state_info(
            &mut fixture,
            "active",
            0,
            modifier_generator,
            Some(leave_transition),
            None,
        );
        let state1 = state_info(
            &mut fixture,
            "inactive",
            1,
            inactive_clip,
            Some(enter_transition),
            None,
        );
        let root = state_machine(&mut fixture, "lifecycle", 0, &[state0, state1], None);
        let behavior = fixture.finish(root);
        let animation = linear_animation(1.0, 0.0);
        let sources = [AnimationPackfile::new("still.hkt", &animation)];
        let mut evaluator =
            BehaviorEvaluator::load(&behavior, &sources, LoadOptions::default()).unwrap();

        evaluator.advance(0.0).unwrap();
        assert_eq!(
            evaluator.variable("Assigned"),
            Some(VariableValue::Real(2.0))
        );
        evaluator
            .set_variable("Assigned", VariableValue::Real(5.0))
            .unwrap();
        evaluator.advance(0.0).unwrap();
        assert_eq!(
            evaluator.variable("Assigned"),
            Some(VariableValue::Real(5.0))
        );

        evaluator.send_event("leave").unwrap();
        evaluator.advance(0.0).unwrap();
        evaluator
            .set_variable("Assigned", VariableValue::Real(7.0))
            .unwrap();
        evaluator.send_event("enter").unwrap();
        evaluator.advance(0.0).unwrap();
        assert_eq!(
            evaluator.variable("Assigned"),
            Some(VariableValue::Real(2.0))
        );
    }
}

#[test]
fn transition_weights_events_notifications_and_flags() {
    {
        let mut fixture = GraphFixture::new();
        let speed_event = fixture.event("speedTransition");
        let direction_event = fixture.event("directionTransition");
        let a = clip(&mut fixture, default_clip("a", "a.hkt"));
        let b = clip(&mut fixture, default_clip("b", "b.hkt"));
        let c = clip(&mut fixture, default_clip("c", "c.hkt"));
        let half = transition_effect(&mut fixture, "speed_0.5", 0.5);
        let quarter = transition_effect(&mut fixture, "direction_0.25", 0.25);
        let per_state = transition_array(&mut fixture, &[(speed_event, 1, Some(half))]);
        let wildcard = transition_array(&mut fixture, &[(direction_event, 2, Some(quarter))]);
        let state0 = state_info(&mut fixture, "zero", 0, a, Some(per_state), None);
        let state1 = state_info(&mut fixture, "one", 1, b, None, None);
        let state2 = state_info(&mut fixture, "two", 2, c, None, None);
        let root = state_machine(
            &mut fixture,
            "locomotion",
            0,
            &[state0, state1, state2],
            Some(wildcard),
        );
        let behavior = fixture.finish(root);
        let anim_a = linear_animation(1.0, 1.0);
        let anim_b = linear_animation(1.0, 3.0);
        let anim_c = linear_animation(1.0, 5.0);
        let sources = [
            AnimationPackfile::new("a.hkt", &anim_a),
            AnimationPackfile::new("b.hkt", &anim_b),
            AnimationPackfile::new("c.hkt", &anim_c),
        ];
        let mut evaluator =
            BehaviorEvaluator::load_with_trace(&behavior, &sources, LoadOptions::default())
                .unwrap();

        evaluator.send_event("speedTransition").unwrap();
        assert_eq!(
            evaluator.advance(0.25).unwrap().x.to_bits(),
            0.5_f32.to_bits()
        );
        assert_eq!(
            evaluator.advance(0.25).unwrap().x.to_bits(),
            0.75_f32.to_bits()
        );
        evaluator.send_event("directionTransition").unwrap();
        assert_eq!(
            evaluator.advance(0.125).unwrap().x.to_bits(),
            0.5_f32.to_bits()
        );
        let transitions: Vec<_> = evaluator
            .trace()
            .advances
            .iter()
            .flat_map(|advance| &advance.transitions)
            .collect();
        assert_eq!(transitions.len(), 3);
        assert_eq!(transitions[0].elapsed_before.to_bits(), 0.0_f32.to_bits());
        assert_eq!(transitions[0].elapsed.to_bits(), 0.25_f32.to_bits());
        assert_eq!(transitions[0].duration.to_bits(), 0.5_f32.to_bits());
        assert_eq!(transitions[0].from_weight.to_bits(), 0.5_f32.to_bits());
        assert_eq!(transitions[0].to_weight.to_bits(), 0.5_f32.to_bits());
        assert_eq!(transitions[1].elapsed.to_bits(), 0.5_f32.to_bits());
        assert_eq!(transitions[2].elapsed.to_bits(), 0.125_f32.to_bits());
        assert_eq!(transitions[2].duration.to_bits(), 0.25_f32.to_bits());
    }
    {
        let mut fixture = GraphFixture::new();
        let go = fixture.event("go");
        let automatic = fixture.event("automatic");
        let still = linear_animation(1.0, 0.0);
        let a = clip(&mut fixture, default_clip("a", "still.hkt"));
        let b = clip(&mut fixture, default_clip("b", "still.hkt"));
        let c = clip(&mut fixture, default_clip("c", "still.hkt"));
        let first = transition_array(&mut fixture, &[(go, 1, None)]);
        let second = transition_array(&mut fixture, &[(automatic, 2, None)]);
        let enter = event_array(&mut fixture, &[automatic]);
        let state0 = state_info(&mut fixture, "zero", 0, a, Some(first), None);
        let state1 = state_info(&mut fixture, "one", 1, b, Some(second), Some(enter));
        let state2 = state_info(&mut fixture, "two", 2, c, None, None);
        let root = state_machine(&mut fixture, "events", 0, &[state0, state1, state2], None);
        let behavior = fixture.finish(root);
        let sources = [AnimationPackfile::new("still.hkt", &still)];
        let mut evaluator =
            BehaviorEvaluator::load(&behavior, &sources, LoadOptions::default()).unwrap();

        evaluator.send_event("go").unwrap();
        evaluator.advance(0.0).unwrap();
        assert_eq!(
            evaluator
                .active_state(&GeneratorSelector::Name("events".to_string()))
                .unwrap(),
            2
        );
    }
    {
        let mut fixture = GraphFixture::new();
        let go = fixture.event("go");
        let exited = fixture.event("exited");
        let entered = fixture.event("entered");
        let still = linear_animation(1.0, 0.0);
        let generators: Vec<_> = (0..4)
            .map(|index| {
                clip(
                    &mut fixture,
                    default_clip(&format!("clip{index}"), "still.hkt"),
                )
            })
            .collect();
        let from_zero = transition_array(&mut fixture, &[(go, 1, None)]);
        let from_one = transition_array(&mut fixture, &[(exited, 2, None)]);
        let from_two = transition_array(&mut fixture, &[(entered, 3, None)]);
        let exit_events = event_array(&mut fixture, &[exited]);
        let enter_events = event_array(&mut fixture, &[entered]);
        let state0 = state_info_with_notifications(
            &mut fixture,
            "zero",
            0,
            generators[0],
            Some(from_zero),
            None,
            Some(exit_events),
        );
        let state1 = state_info(
            &mut fixture,
            "one",
            1,
            generators[1],
            Some(from_one),
            Some(enter_events),
        );
        let state2 = state_info(&mut fixture, "two", 2, generators[2], Some(from_two), None);
        let state3 = state_info(&mut fixture, "three", 3, generators[3], None, None);
        let root = state_machine(
            &mut fixture,
            "ordered",
            0,
            &[state0, state1, state2, state3],
            None,
        );
        let behavior = fixture.finish(root);
        let sources = [AnimationPackfile::new("still.hkt", &still)];
        let mut evaluator =
            BehaviorEvaluator::load(&behavior, &sources, LoadOptions::default()).unwrap();

        evaluator.advance(0.0).unwrap();
        evaluator.send_event("go").unwrap();
        evaluator.advance(0.0).unwrap();
        assert_eq!(
            evaluator
                .active_state(&GeneratorSelector::Name("ordered".to_string()))
                .unwrap(),
            3
        );
    }
    {
        let mut fixture = GraphFixture::new();
        let go = fixture.event("go");
        let from = clip(&mut fixture, default_clip("from", "from.hkt"));
        let to = clip(&mut fixture, default_clip("to", "to.hkt"));
        let effect = transition_effect_with_flags(&mut fixture, "synced", 0.25, 3);
        let transitions = transition_array(&mut fixture, &[(go, 1, Some(effect))]);
        let state0 = state_info(&mut fixture, "zero", 0, from, Some(transitions), None);
        let state1 = state_info(&mut fixture, "one", 1, to, None, None);
        let root = state_machine(&mut fixture, "transition flags", 0, &[state0, state1], None);
        let behavior = fixture.finish(root);
        let from_animation = linear_animation(1.0, 10.0);
        let to_animation = linear_animation(2.0, 20.0);
        let sources = [
            AnimationPackfile::new("from.hkt", &from_animation),
            AnimationPackfile::new("to.hkt", &to_animation),
        ];
        let mut evaluator =
            BehaviorEvaluator::load(&behavior, &sources, LoadOptions::default()).unwrap();

        evaluator.send_event("go").unwrap();
        assert_eq!(
            evaluator.advance(0.125).unwrap().x.to_bits(),
            1.25_f32.to_bits()
        );
    }
}

#[test]
fn unsupported_graph_features_fail_closed() {
    {
        let cases = [
            (1, "trigger interval"),
            (1 << 1, "initiate interval"),
            (1 << 3, "uninterruptible while delayed"),
            (1 << 4, "delayed state change"),
            (1 << 12, "from-nested-state constraint"),
            (1 << 13, "to-nested-state constraint"),
            (1 << 14, "abut-at-end transition"),
        ];
        for (flags, expected_feature) in cases {
            let mut fixture = GraphFixture::new();
            let go = fixture.event("go");
            let still = linear_animation(1.0, 0.0);
            let a = clip(&mut fixture, default_clip("a", "still.hkt"));
            let b = clip(&mut fixture, default_clip("b", "still.hkt"));
            let transitions = transition_array_with_flags(&mut fixture, &[(go, 1, None, flags)]);
            let transition_array_index = transitions;
            let state0 = state_info(&mut fixture, "zero", 0, a, Some(transitions), None);
            let state1 = state_info(&mut fixture, "one", 1, b, None, None);
            let root = state_machine(&mut fixture, "flags", 0, &[state0, state1], None);
            let behavior = fixture.finish(root);
            let sources = [AnimationPackfile::new("still.hkt", &still)];
            let mut evaluator =
                BehaviorEvaluator::load(&behavior, &sources, LoadOptions::default()).unwrap();

            evaluator.send_event("go").unwrap();
            assert!(matches!(
                evaluator.advance(0.0),
                Err(BehaviorEvalError::UnsupportedTransitionFeature {
                    object_index,
                    feature,
                }) if object_index == transition_array_index && feature == expected_feature
            ));
        }
    }
    {
        let mut self_fixture = GraphFixture::new();
        let again = self_fixture.event("again");
        let still = linear_animation(1.0, 0.0);
        let clip0 = clip(&mut self_fixture, default_clip("self", "still.hkt"));
        let self_transitions = transition_array(&mut self_fixture, &[(again, 0, None)]);
        let state0 = state_info(
            &mut self_fixture,
            "zero",
            0,
            clip0,
            Some(self_transitions),
            None,
        );
        let self_root = state_machine(&mut self_fixture, "self", 0, &[state0], None);
        let self_behavior = self_fixture.finish(self_root);
        let sources = [AnimationPackfile::new("still.hkt", &still)];
        let mut evaluator =
            BehaviorEvaluator::load(&self_behavior, &sources, LoadOptions::default()).unwrap();
        evaluator.send_event("again").unwrap();
        assert!(matches!(
            evaluator.advance(0.0),
            Err(BehaviorEvalError::UnsupportedTransitionFeature {
                object_index,
                feature,
            }) if object_index == self_root && feature == "self-transition mode 1"
        ));

        let mut fixture = GraphFixture::new();
        let go = fixture.event("go");
        let replace = fixture.event("replace");
        let a = clip(&mut fixture, default_clip("a", "still.hkt"));
        let b = clip(&mut fixture, default_clip("b", "still.hkt"));
        let c = clip(&mut fixture, default_clip("c", "still.hkt"));
        let effect = transition_effect(&mut fixture, "slow", 1.0);
        let first = transition_array(&mut fixture, &[(go, 1, Some(effect))]);
        let second = transition_array(&mut fixture, &[(replace, 2, None)]);
        let state0 = state_info(&mut fixture, "zero", 0, a, Some(first), None);
        let state1 = state_info(&mut fixture, "one", 1, b, Some(second), None);
        let state2 = state_info(&mut fixture, "two", 2, c, None, None);
        let root = state_machine(
            &mut fixture,
            "replacement",
            0,
            &[state0, state1, state2],
            None,
        );
        let behavior = fixture.finish(root);
        let mut evaluator =
            BehaviorEvaluator::load(&behavior, &sources, LoadOptions::default()).unwrap();
        evaluator.send_event("go").unwrap();
        evaluator.advance(0.1).unwrap();
        evaluator.send_event("replace").unwrap();
        assert!(matches!(
            evaluator.advance(0.0),
            Err(BehaviorEvalError::UnsupportedTransitionFeature {
                object_index,
                feature,
            }) if object_index == root && feature.contains("transition stacking/replacement")
        ));
    }
    {
        let mut fixture = GraphFixture::new();
        let go = fixture.event("go");
        let still = linear_animation(1.0, 0.0);
        let a = clip(&mut fixture, default_clip("a", "still.hkt"));
        let b = clip(&mut fixture, default_clip("b", "still.hkt"));
        let effect = transition_effect(&mut fixture, "events", 0.25);
        fixture.objects[effect]
            .members
            .iter_mut()
            .find(|member| member.name == "eventMode")
            .unwrap()
            .value = HkxValue::I8(1);
        let transitions = transition_array(&mut fixture, &[(go, 1, Some(effect))]);
        let state0 = state_info(&mut fixture, "zero", 0, a, Some(transitions), None);
        let state1 = state_info(&mut fixture, "one", 1, b, None, None);
        let root = state_machine(&mut fixture, "effect mode", 0, &[state0, state1], None);
        let behavior = fixture.finish(root);
        let sources = [AnimationPackfile::new("still.hkt", &still)];
        let mut evaluator =
            BehaviorEvaluator::load(&behavior, &sources, LoadOptions::default()).unwrap();

        evaluator.send_event("go").unwrap();
        assert!(matches!(
            evaluator.advance(0.0),
            Err(BehaviorEvalError::UnsupportedTransitionFeature {
                object_index,
                feature,
            }) if object_index == effect && feature == "transition event mode 1"
        ));
    }
    {
        let mut fixture = GraphFixture::new();
        let root = fixture.push(
            "hkbPoseMatchingGenerator",
            vec![member("name", string("unsupported"))],
        );
        let behavior = fixture.finish(root);
        let mut evaluator =
            BehaviorEvaluator::load(&behavior, &[], LoadOptions::default()).unwrap();

        assert!(matches!(
            evaluator.advance(0.1),
            Err(BehaviorEvalError::UnsupportedActiveClass {
                class_name,
                object_index
            }) if class_name == "hkbPoseMatchingGenerator" && object_index == root
        ));
    }
}

#[test]
fn cyclic_transitions_blend_wrap_and_freeze() {
    {
        let mut fixture = GraphFixture::new();
        let direction = fixture.real_variable("Direction", 0.9 * std::f32::consts::TAU);
        let cyclic_binding = binding_set(&mut fixture, &[("fBlendParameter", direction)]);
        let still = clip(&mut fixture, default_clip("still", "still.hkt"));
        let moving = clip(&mut fixture, default_clip("moving", "moving.hkt"));
        let child0 = blender_child(&mut fixture, still, 0.0, 1.0);
        let child1 = blender_child(&mut fixture, moving, 1.0, 1.0);
        let blender = blender(
            &mut fixture,
            "direction",
            None,
            0.0,
            (1 << 4) | (1 << 5),
            -1,
            &[child0, child1],
        );
        let root = fixture.push(
            "BSCyclicBlendTransitionGenerator",
            vec![
                member("variableBindingSet", pointer(cyclic_binding)),
                member("name", string("cyclic")),
                member("pBlenderGenerator", pointer(blender)),
                member("fBlendParameter", HkxValue::F32(0.0)),
                member("fTransitionDuration", HkxValue::F32(0.2)),
                member(
                    "EventToFreezeBlendValue",
                    inline(vec![member("id", HkxValue::I32(-1))]),
                ),
                member(
                    "EventToCrossBlend",
                    inline(vec![member("id", HkxValue::I32(-1))]),
                ),
            ],
        );
        let behavior = fixture.finish(root);
        let still_anim = linear_animation(1.0, 0.0);
        let moving_anim = linear_animation(1.0, 10.0);
        let sources = [
            AnimationPackfile::new("still.hkt", &still_anim),
            AnimationPackfile::new("moving.hkt", &moving_anim),
        ];
        let mut evaluator =
            BehaviorEvaluator::load(&behavior, &sources, LoadOptions::default()).unwrap();

        assert_eq!(
            evaluator.advance(0.1).unwrap().x.to_bits(),
            0.9_f32.to_bits()
        );
        evaluator
            .set_variable(
                "Direction",
                VariableValue::Real(0.1 * std::f32::consts::TAU),
            )
            .unwrap();
        assert_eq!(
            evaluator.advance(0.1).unwrap().x.to_bits(),
            0.0_f32.to_bits()
        );
        assert_eq!(evaluator.advance(0.1).unwrap().x.to_bits(), 0x3dcc_ccd0);

        let mut wrapped = BehaviorEvaluator::load(
            &behavior,
            &sources,
            LoadOptions {
                actions: vec![PathAction::SetVariable {
                    name: "Direction".to_string(),
                    value: VariableValue::Real(0.25 * std::f32::consts::TAU),
                }],
                ..LoadOptions::default()
            },
        )
        .unwrap();
        assert_eq!(
            wrapped.advance(0.1).unwrap().x.to_bits(),
            0.25_f32.to_bits()
        );
    }
    {
        let mut fixture = GraphFixture::new();
        let direction = fixture.real_variable("Direction", 0.8 * std::f32::consts::TAU);
        let transition_out = fixture.event("transitionOut");
        let transition_in = fixture.event("transitionIn");
        let cyclic_binding = binding_set(&mut fixture, &[("fBlendParameter", direction)]);
        let still = clip(&mut fixture, default_clip("still", "still.hkt"));
        let moving = clip(&mut fixture, default_clip("moving", "moving.hkt"));
        let child0 = blender_child(&mut fixture, still, 0.0, 1.0);
        let child1 = blender_child(&mut fixture, moving, 1.0, 1.0);
        let blender = blender(
            &mut fixture,
            "direction",
            None,
            0.0,
            (1 << 4) | (1 << 5),
            -1,
            &[child0, child1],
        );
        let cyclic = fixture.push(
            "BSCyclicBlendTransitionGenerator",
            vec![
                member("variableBindingSet", pointer(cyclic_binding)),
                member("name", string("cyclic transition")),
                member("pBlenderGenerator", pointer(blender)),
                member("fBlendParameter", HkxValue::F32(0.0)),
                member("fTransitionDuration", HkxValue::F32(0.2)),
                member("eBlendCurve", HkxValue::I8(0)),
                member(
                    "EventToFreezeBlendValue",
                    inline(vec![member("id", HkxValue::I32(-1))]),
                ),
                member(
                    "EventToCrossBlend",
                    inline(vec![member("id", HkxValue::I32(-1))]),
                ),
                member(
                    "TransitionOutEvent",
                    inline(vec![member("id", HkxValue::I32(transition_out))]),
                ),
                member(
                    "TransitionInEvent",
                    inline(vec![member("id", HkxValue::I32(transition_in))]),
                ),
            ],
        );
        let enter_events = event_array(&mut fixture, &[transition_in]);
        let state = state_info(&mut fixture, "active", 0, cyclic, None, Some(enter_events));
        let root = state_machine(&mut fixture, "cyclic state", 0, &[state], None);
        let behavior = fixture.finish(root);
        let still_animation = linear_animation(1.0, 0.0);
        let moving_animation = linear_animation(1.0, 10.0);
        let sources = [
            AnimationPackfile::new("still.hkt", &still_animation),
            AnimationPackfile::new("moving.hkt", &moving_animation),
        ];
        let mut evaluator =
            BehaviorEvaluator::load(&behavior, &sources, LoadOptions::default()).unwrap();

        let first = evaluator.advance(0.1).unwrap().x;
        assert!(
            (first - 0.9).abs() < 1.0e-6,
            "transition-in first={first:?}"
        );
        evaluator.send_event("transitionOut").unwrap();
        let frozen = evaluator.advance(0.1).unwrap().x;
        assert!(
            (frozen - 0.9).abs() < 1.0e-6,
            "transition-out frozen={frozen:?}"
        );
        evaluator
            .set_variable(
                "Direction",
                VariableValue::Real(0.2 * std::f32::consts::TAU),
            )
            .unwrap();
        let still_frozen = evaluator.advance(0.1).unwrap().x;
        assert!(
            (still_frozen - 0.9).abs() < 1.0e-6,
            "changed target while frozen={still_frozen:?}"
        );
        evaluator.send_event("transitionIn").unwrap();
        let resumed = evaluator.advance(0.1).unwrap().x;
        assert!(
            (resumed - 0.05).abs() < 1.0e-6,
            "transition-in resumed={resumed:?}"
        );
    }
}

#[test]
fn blender_child_order_and_sync_master() {
    {
        let mut fixture = GraphFixture::new();
        let a = clip(&mut fixture, default_clip("a", "a.hkt"));
        let b = clip(&mut fixture, default_clip("b", "b.hkt"));
        let child_a = blender_child(&mut fixture, a, 1.0, 1.0);
        let child_b = blender_child(&mut fixture, b, 3.0, 1.0);
        let root = blender(
            &mut fixture,
            "weighted",
            None,
            0.0,
            0,
            -1,
            &[child_a, child_b],
        );
        let behavior = fixture.finish(root);
        let anim_a = linear_animation(1.0, 4.0);
        let anim_b = linear_animation(1.0, 8.0);
        let sources = [
            AnimationPackfile::new("a.hkt", &anim_a),
            AnimationPackfile::new("b.hkt", &anim_b),
        ];
        let mut evaluator =
            BehaviorEvaluator::load(&behavior, &sources, LoadOptions::default()).unwrap();

        assert_eq!(
            evaluator.advance(0.25).unwrap().x.to_bits(),
            1.75_f32.to_bits()
        );
    }
    {
        let mut fixture = GraphFixture::new();
        let master = clip(&mut fixture, default_clip("master", "master.hkt"));
        let follower = clip(&mut fixture, default_clip("follower", "follower.hkt"));
        let child_master = blender_child(&mut fixture, master, 0.0, 1.0);
        let child_follower = blender_child(&mut fixture, follower, 1.0, 1.0);
        let root = blender(
            &mut fixture,
            "synced",
            None,
            0.5,
            (1 << 4) | 1,
            -1,
            &[child_master, child_follower],
        );
        let behavior = fixture.finish(root);
        let master_anim = linear_animation(1.0, 10.0);
        let follower_anim = linear_animation(2.0, 20.0);
        let sources = [
            AnimationPackfile::new("master.hkt", &master_anim),
            AnimationPackfile::new("follower.hkt", &follower_anim),
        ];
        let mut evaluator =
            BehaviorEvaluator::load(&behavior, &sources, LoadOptions::default()).unwrap();

        assert_eq!(
            evaluator.advance(0.5).unwrap().x.to_bits(),
            7.5_f32.to_bits()
        );
    }
}

#[test]
fn clip_playback_timing_is_exact() {
    {
        let mut fixture = GraphFixture::new();
        let root = clip(
            &mut fixture,
            ClipSpec {
                name: "cropped",
                animation: "curve.hkt",
                binding: None,
                triggers: None,
                speed: 2.0,
                enforced_duration: 0.0,
                mode: 1,
                crop_start: 0.25,
                crop_end: 0.25,
                start_time: 0.125,
            },
        );
        let behavior = fixture.finish(root);
        let animation =
            sampled_animation(1.0, &[[0.0, 0.0, 0.0], [4.0, 0.0, 0.0], [10.0, 0.0, 0.0]]);
        let sources = [AnimationPackfile::new("curve.hkt", &animation)];
        let mut evaluator =
            BehaviorEvaluator::load(&behavior, &sources, LoadOptions::default()).unwrap();

        assert_eq!(
            evaluator.advance(0.25).unwrap().x.to_bits(),
            5.0_f32.to_bits()
        );
        assert_eq!(
            evaluator.advance(0.125).unwrap().x.to_bits(),
            2.5_f32.to_bits()
        );
        assert_eq!(
            evaluator.advance(0.5).unwrap().x.to_bits(),
            10.0_f32.to_bits()
        );
    }
    {
        let mut fixture = GraphFixture::new();
        let root = clip(
            &mut fixture,
            ClipSpec {
                speed: 4.0,
                enforced_duration: 4.0,
                ..default_clip("enforced", "move.hkt")
            },
        );
        let behavior = fixture.finish(root);
        let animation = linear_animation(2.0, 20.0);
        let sources = [AnimationPackfile::new("move.hkt", &animation)];
        let mut evaluator =
            BehaviorEvaluator::load(&behavior, &sources, LoadOptions::default()).unwrap();

        assert_eq!(
            evaluator.advance(1.0).unwrap().x.to_bits(),
            5.0_f32.to_bits()
        );
    }
    {
        let mut fixture = GraphFixture::new();
        let wrapped = fixture.event("wrapped");
        let triggers = trigger_array(&mut fixture, &[(0.0, wrapped)]);
        let moving = clip(
            &mut fixture,
            ClipSpec {
                triggers: Some(triggers),
                ..default_clip("loop", "move.hkt")
            },
        );
        let still = clip(&mut fixture, default_clip("done", "still.hkt"));
        let transitions = transition_array(&mut fixture, &[(wrapped, 1, None)]);
        let state0 = state_info(&mut fixture, "looping", 0, moving, Some(transitions), None);
        let state1 = state_info(&mut fixture, "done", 1, still, None, None);
        let root = state_machine(&mut fixture, "trigger", 0, &[state0, state1], None);
        let behavior = fixture.finish(root);
        let moving_animation = linear_animation(1.0, 10.0);
        let still_animation = linear_animation(1.0, 0.0);
        let sources = [
            AnimationPackfile::new("move.hkt", &moving_animation),
            AnimationPackfile::new("still.hkt", &still_animation),
        ];
        let mut evaluator =
            BehaviorEvaluator::load(&behavior, &sources, LoadOptions::default()).unwrap();

        assert_eq!(
            evaluator.advance(1.0).unwrap().x.to_bits(),
            10.0_f32.to_bits()
        );
        assert_eq!(
            evaluator
                .active_state(&GeneratorSelector::Name("trigger".to_string()))
                .unwrap(),
            0
        );
        evaluator.advance(0.0).unwrap();
        assert_eq!(
            evaluator
                .active_state(&GeneratorSelector::Name("trigger".to_string()))
                .unwrap(),
            1
        );
    }
    {
        let mut fixture = GraphFixture::new();
        let root = clip(&mut fixture, default_clip("moving", "moving.hkt"));
        let behavior = fixture.finish(root);
        let animation = linear_animation(10.0, 10.0);
        let sources = [AnimationPackfile::new("moving.hkt", &animation)];
        let mut current =
            BehaviorEvaluator::load(&behavior, &sources, LoadOptions::default()).unwrap();
        let mut og = BehaviorEvaluator::load(&behavior, &sources, LoadOptions::default()).unwrap();
        let dt = f32::from_bits(0x3d08_8889);

        assert_eq!(
            current.advance_repeated(dt, 9).unwrap().x.to_bits(),
            (dt * 9.0).to_bits()
        );
        assert_eq!(
            og.advance_repeated(dt, 8).unwrap().x.to_bits(),
            (dt * 8.0).to_bits()
        );
    }
}

#[test]
fn start_state_selection_by_path_and_int_binding() {
    {
        let mut fixture = GraphFixture::new();
        let entered = fixture.event("enteredSeven");
        let still = linear_animation(1.0, 0.0);
        let a = clip(&mut fixture, default_clip("a", "still.hkt"));
        let b = clip(&mut fixture, default_clip("b", "still.hkt"));
        let c = clip(&mut fixture, default_clip("c", "still.hkt"));
        let followup = transition_array(&mut fixture, &[(entered, 8, None)]);
        let enter_events = event_array(&mut fixture, &[entered]);
        let state0 = state_info(&mut fixture, "zero", 0, a, None, None);
        let state7 = state_info(
            &mut fixture,
            "seven",
            7,
            b,
            Some(followup),
            Some(enter_events),
        );
        let state8 = state_info(&mut fixture, "eight", 8, c, None, None);
        let root = state_machine(&mut fixture, "path", 0, &[state0, state7, state8], None);
        let behavior = fixture.finish(root);
        let sources = [AnimationPackfile::new("still.hkt", &still)];
        let mut evaluator = BehaviorEvaluator::load(
            &behavior,
            &sources,
            LoadOptions {
                root: GeneratorSelector::GraphRoot,
                actions: vec![PathAction::SetState {
                    state_machine: GeneratorSelector::Name("path".to_string()),
                    state_id: 7,
                }],
                ..LoadOptions::default()
            },
        )
        .unwrap();

        assert_eq!(
            evaluator
                .active_state(&GeneratorSelector::Name("path".to_string()))
                .unwrap(),
            7
        );
        evaluator.advance(0.0).unwrap();
        assert_eq!(
            evaluator
                .active_state(&GeneratorSelector::Name("path".to_string()))
                .unwrap(),
            8
        );
    }
    {
        let mut fixture = GraphFixture::new();
        let selector = fixture.int_variable("State", 4);
        let state_binding = binding_set(&mut fixture, &[("startStateId", selector)]);
        let still = linear_animation(1.0, 0.0);
        let a = clip(&mut fixture, default_clip("a", "still.hkt"));
        let b = clip(&mut fixture, default_clip("b", "still.hkt"));
        let state0 = state_info(&mut fixture, "zero", 0, a, None, None);
        let state4 = state_info(&mut fixture, "four", 4, b, None, None);
        let root = state_machine(&mut fixture, "bound", 0, &[state0, state4], None);
        fixture.objects[root]
            .members
            .iter_mut()
            .find(|member| member.name == "variableBindingSet")
            .unwrap()
            .value = pointer(state_binding);
        let behavior = fixture.finish(root);
        let sources = [AnimationPackfile::new("still.hkt", &still)];
        let mut evaluator =
            BehaviorEvaluator::load(&behavior, &sources, LoadOptions::default()).unwrap();

        assert_eq!(
            evaluator
                .active_state(&GeneratorSelector::Name("bound".to_string()))
                .unwrap(),
            4
        );
    }
}
