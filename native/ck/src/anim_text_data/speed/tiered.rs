use super::*;

pub(super) struct Source {
    generator: usize,
    speed_variable: String,
    direction_variable: String,
    speed_min: f32,
    speed_max: f32,
    transition_duration: f32,
    pub(super) clips: Vec<usize>,
}

pub(super) fn source(
    graph: &BehaviorGraph,
    file: usize,
    state_machine: usize,
    sapt: &[String],
) -> Option<Source> {
    let objects = graph.objs(file);
    let mut blenders = Vec::new();
    collect_objects_of_class(
        objects,
        state_machine,
        "hkbBlenderGenerator",
        &mut HashSet::new(),
        &mut blenders,
    );
    for generator in blenders {
        if bound_var(
            &objects[generator],
            "blendParameter",
            objects,
            graph.vars(file),
        )
        .as_deref()
            != Some("B21_LocomotionSpeed")
        {
            continue;
        }
        let children = ptr_array(&objects[generator], "children");
        if children.len() < 2 {
            continue;
        }
        let mut result = Source {
            generator,
            speed_variable: "B21_LocomotionSpeed".to_string(),
            direction_variable: String::new(),
            speed_min: f32::INFINITY,
            speed_max: f32::NEG_INFINITY,
            transition_duration: 0.0,
            clips: Vec::new(),
        };
        let valid = children.into_iter().all(|child| {
            let Some(weight) = f32_member(&objects[child], "weight") else {
                return false;
            };
            let Some(cyclic) = first_ptr(&objects[child], "generator") else {
                return false;
            };
            if objects[cyclic].class_name != "BSCyclicBlendTransitionGenerator" {
                return false;
            }
            let Some(direction) = bound_var(
                &objects[cyclic],
                "fBlendParameter",
                objects,
                graph.vars(file),
            ) else {
                return false;
            };
            if !result.direction_variable.is_empty() && result.direction_variable != direction {
                return false;
            }
            let Some(fan) = first_ptr(&objects[cyclic], "pBlenderGenerator") else {
                return false;
            };
            if f32_member(&objects[fan], "minCyclicBlendParameter") != Some(0.0)
                || f32_member(&objects[fan], "maxCyclicBlendParameter") != Some(1.0)
                || i64_member(&objects[fan], "flags").unwrap_or(0)
                    & BLENDER_FLAG_PARAMETRIC_BLEND_CYCLIC
                    == 0
            {
                return false;
            }
            let fan_children = ptr_array(&objects[fan], "children");
            if fan_children.len() < 4 {
                return false;
            }
            for fan_child in fan_children {
                let Some(clip) = first_ptr(&objects[fan_child], "generator")
                    .and_then(|index| linear_clip_descendant(objects, index))
                else {
                    return false;
                };
                if horizontal_clip_path(graph, file, clip, sapt).is_none() {
                    return false;
                }
                result.clips.push(clip);
            }
            result.direction_variable = direction;
            result.speed_min = result.speed_min.min(weight);
            result.speed_max = result.speed_max.max(weight);
            result.transition_duration = result
                .transition_duration
                .max(f32_member(&objects[cyclic], "fTransitionDuration").unwrap_or(0.0));
            true
        });
        if valid && result.speed_max > result.speed_min {
            return Some(result);
        }
    }
    None
}

pub(super) fn recipe_root(
    builder: &mut RecipeBuilder,
    file: usize,
    state_machine: usize,
    state_machine_path: String,
    source: Source,
) -> Result<SpeedInfoRootRecipe, SpeedInfoProducerError> {
    let record = builder.record(
        ProducerClass::SpeedSampled,
        RecipeRecordParentage::SourceRoot {
            behavior_file: file,
            state_machine,
        },
        file,
        source.generator,
    )?;
    // Pose layers and actor selectors do not contribute to this locomotion subtree's root motion.
    let replay = vec![BehaviorReplay {
        owner: BehaviorGraphOwner {
            behavior_file: file,
            relative_path: builder.graph.files[file].rel.clone(),
            graph_name: builder.graph.files[file].core.graph_name.clone(),
        },
        root: GeneratorSelector::ObjectIndex(source.generator),
        actions: Vec::new(),
    }];
    let mut domain = SampleDomain::historical_fo4(
        source.direction_variable,
        source.speed_variable,
        -std::f32::consts::PI,
        std::f32::consts::PI,
        source.speed_min,
        source.speed_max,
    );
    domain.warmup_updates = domain
        .warmup_updates
        .max((source.transition_duration / domain.timestep).ceil() as u32 + 1);
    let directional_summary = source
        .clips
        .iter()
        .map(|clip| {
            let object = &builder.graph.objs(file)[*clip];
            DirectionalSummaryEvaluation {
                animation_name: string_member(object, "animationName").unwrap(),
                animation_path: clip_loop_path(object, builder.graph.roots, builder.sapt_chain)
                    .unwrap(),
            }
        })
        .collect();
    let evaluation = builder.request(EvaluationRequest::SpeedSampled(SpeedSampledEvaluation {
        record,
        domain,
        direction_input_scale: -1.0,
        directional_summary,
        replay: replay.clone(),
    }))?;
    let metadata = builder.root_metadata(record, replay)?;
    Ok(SpeedInfoRootRecipe {
        record,
        state_machine_path,
        contour: RecipeContour::SpeedSampled {
            record,
            center_mode: CenterMode::ZeroCentered,
            clip: String::new(),
            condition: String::new(),
            entry: RecipeEntry::RootMetadata(metadata),
            evaluation,
        },
        metadata: RootMetadataRecipe::DirectSpeedSampled {
            evaluation: metadata,
        },
    })
}
