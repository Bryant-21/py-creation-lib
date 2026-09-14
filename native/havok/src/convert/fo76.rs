use crate::error::{HavokError, HavokResult};
use crate::hkx::descriptors::DescriptorRegistry;
use crate::hkx::types::{HkxType, HkxValue};
use crate::hkx::{HkxFile, HkxMember, HkxObject, Tagfile};

#[path = "fo76_bone_weights.rs"]
mod bone_weights;

pub const FO76_TO_FO4_ROUTE: &str = "tag0-fo76-to-fo4";

const HK_INTERLEAVED_ANIMATION_TYPE: i32 = 1;
const HK_SPLINE_COMPRESSED_ANIMATION_TYPE: i32 = 3;

const ALWAYS_ON_TRANSFORMS: &[&str] = &[
    "_strip_heap_allocator",
    "_strip_resource_data",
    "_rename_variant",
    "_fix_version_metadata",
    "_stamp_fo76_schema_versions",
    "_synthesize_memory_resource_container",
    "_auto_fix_human_bone_tracks",
    "_migrate_skeleton_physics",
    "_reclassify_fo76_physics_system_data",
    "_migrate_compound_shape_to_physics_system",
    "_reclassify_mass_distributions",
    "_strip_shape_connectivity",
    "_convert_polytope_to_capsule",
    "_flatten_compound_shapes_in_psd",
    "_wrap_capsules_in_compound_shape",
    "_strip_fo76_skeleton_classes",
    "_convert_limited_hinge_to_ragdoll",
    "_fix_physics_referenced_objects",
    "_normalize_bumper_body_cinfos",
    "_normalize_ragdoll_body_cinfos",
    "_inject_ragdoll_motors",
    "_synthesize_motion_cinfos",
    "_fix_character_extras",
    "_inject_capsule_defaults",
    "_fix_sphere_dispatch_type",
    "_reclassify_fo76_hkb_layer",
    "_flatten_nested_class_names",
    "_migrate_unsupported_behavior_nodes",
    "_strip_runtime_members",
    "_compact_null_blender_children",
    "_compact_null_state_machine_states",
    "_compact_null_pointer_arrays",
    "_fix_state_machine_typed_refs",
    "_fix_serialized_bool_to_int32",
    "_drop_transitions_to_missing_states",
    "_fix_dangling_pointers",
    "_fix_variable_value_set",
    "_fix_clip_generator_defaults",
    "_apply_classxml_defaults",
    "_populate_event_property_arrays",
    "_reorder_behavior_metadata_to_end",
    "_fix_behavior_variable_infos",
];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Fo76MigrationOptions {
    pub infer_roles: bool,
    /// Decompress hkaSplineCompressedAnimation → hkaInterleavedUncompressedAnimation.
    /// Must run before `strip_bones` and `recompress`.
    pub decompress_spline: bool,
    /// Strip FO76-specific AimSource bone track (index 12) from 96-track interleaved
    /// animations and remap transformTrackToBoneIndices to FO4 skeleton ordering.
    /// Requires `decompress_spline` to have run first.
    pub strip_bones: bool,
    /// Recompress hkaInterleavedUncompressedAnimation → hkaSplineCompressedAnimation.
    /// Runs after `strip_bones`.
    pub recompress: bool,
}

pub fn always_on_transform_names() -> &'static [&'static str] {
    ALWAYS_ON_TRANSFORMS
}

pub fn migrate_2015_tag0_to_2014(
    tagfile: &Tagfile,
    options: Fo76MigrationOptions,
) -> HavokResult<HkxFile> {
    migrate_2015_tag0_to_2014_with_warnings(tagfile, options).map(|result| result.hkx)
}

#[derive(Debug, Clone)]
pub struct Fo76MigrationResult {
    pub hkx: HkxFile,
    pub warnings: Vec<String>,
}

pub fn migrate_2015_tag0_to_2014_with_warnings(
    tagfile: &Tagfile,
    options: Fo76MigrationOptions,
) -> HavokResult<Fo76MigrationResult> {
    let hkx = tagfile.materialize_hkx()?;
    migrate_2015_hkx_to_2014_with_warnings(hkx, options, || {
        format!("TAG0 SDK {} materialized and applied", tagfile.sdk_version)
    })
}

pub fn migrate_2015_packfile_to_2014_with_warnings(
    hkx: HkxFile,
    options: Fo76MigrationOptions,
) -> HavokResult<Fo76MigrationResult> {
    migrate_2015_hkx_to_2014_with_warnings(hkx, options, || "packfile applied".to_string())
}

fn migrate_2015_hkx_to_2014_with_warnings(
    mut hkx: HkxFile,
    options: Fo76MigrationOptions,
    source_detail: impl FnOnce() -> String,
) -> HavokResult<Fo76MigrationResult> {
    let _options = options;
    let mut registry = DescriptorRegistry::new();
    let mut warnings = Vec::new();
    let last_applied_transform =
        apply_implemented_transforms(&mut hkx, &mut registry, &mut warnings);

    if last_applied_transform + 1 < ALWAYS_ON_TRANSFORMS.len() {
        let next_transform = last_applied_transform + 1;
        return Err(HavokError::UnportedEdgeCase {
            route: FO76_TO_FO4_ROUTE.to_string(),
            edge_case: ALWAYS_ON_TRANSFORMS[next_transform].to_string(),
            detail: format!(
                "{} through {}; {} and the remaining py_creation_lib/python/creation_lib/hkxpack/migration.py transforms are not ported yet; changed FO4 packfile writer support is also required before this route can emit bytes",
                source_detail(),
                ALWAYS_ON_TRANSFORMS[last_applied_transform],
                ALWAYS_ON_TRANSFORMS[next_transform]
            ),
        });
    }

    // Opt-in transforms — only run when explicitly requested.
    if _options.decompress_spline {
        decompress_spline_animations(&mut hkx)?;
    }
    if _options.strip_bones {
        strip_extra_bone_tracks(&mut hkx);
    }
    if _options.recompress {
        recompress_animations(&mut hkx)?;
    }

    Ok(Fo76MigrationResult { hkx, warnings })
}

fn apply_implemented_transforms(
    hkx: &mut HkxFile,
    registry: &mut DescriptorRegistry,
    warnings: &mut Vec<String>,
) -> usize {
    strip_heap_allocator(hkx);
    strip_resource_data(hkx);
    rename_variant(hkx);
    synthesize_fo4_weapon_character_property_aliases(hkx);
    remap_human_character_property_bone_indices(hkx);
    fix_version_metadata(hkx);
    stamp_fo76_schema_versions(hkx);
    synthesize_memory_resource_container(hkx);

    auto_fix_human_bone_tracks(hkx);
    #[allow(unused_assignments)]
    let mut last_applied = 6;

    // _auto_fix_human_bone_tracks (transform 6) is a no-op for files without
    // animation classes; the physics block continues below.

    // Physics transforms 7..=21 (FO76→FO4). Each call below is tagged with its
    // ALWAYS_ON_TRANSFORMS slice index. No-ops on FO4: 14
    // wrap_capsules_in_compound_shape and 16 convert_limited_hinge_to_ragdoll
    // (FO4 supports limited hinges). 17 fix_physics_referenced_objects is partial.

    migrate_skeleton_physics(hkx); // 7
    reclassify_fo76_physics_system_data(hkx); // 8
    convert_box_shapes_to_polytopes(hkx, warnings);
    migrate_compound_shape_to_physics_system(hkx); // 9 — Phase 7C
    reclassify_mass_distributions(hkx); // 10
    synthesize_ragdoll_shape_geometry(hkx);
    normalize_sphere_support_vertices(hkx);
    normalize_shape_mass_properties(hkx);
    strip_shape_connectivity(hkx); // 11
    convert_polytope_to_capsule(hkx); // 12
    flatten_compound_shapes_in_psd(hkx); // 13 — Phase 7C
    wrap_capsules_in_compound_shape(hkx); // 14 — Phase 7C (no-op, mirrors Python)
    // Build shape→mass cache BEFORE strip removes the `properties` pointer from
    // shape objects.  Keyed by object name so it survives the index remap done
    // inside strip_fo76_skeleton_classes.
    let shape_mass_cache = extract_shape_mass_cache(hkx);
    strip_fo76_skeleton_classes(hkx); // 15
    normalize_dynamic_compound_shapes(hkx);
    populate_dynamic_compound_instances(hkx);
    simplify_compound_ragdoll_body_shapes(hkx);
    strip_unmapped_trailing_ragdoll_controller_bodies(hkx);
    convert_limited_hinge_to_ragdoll(hkx); // 16 — Phase 7C
    fix_physics_referenced_objects(hkx); // 17
    normalize_bumper_body_cinfos(hkx); // 18
    normalize_ragdoll_body_cinfos(hkx); // 19
    normalize_ragdoll_constraint_offsets(hkx);
    inject_ragdoll_motors(hkx); // 20
    synthesize_motion_cinfos(hkx, &shape_mass_cache); // 21
    normalize_ragdoll_body_position_w(hkx);

    // hkaAnimationBinding.blendHint is carried through untouched. FO76 and FO4
    // share the enum (NORMAL=0, ADDITIVE_DEPRECATED=1, ADDITIVE=2), so 1 and 2
    // are two different additive conventions, not one value renumbered across
    // games: 1 expresses the delta in the animated bone's parent space, 2 in
    // bone-local space. Promoting 1→2 without also rebasing the track data
    // rotates every delta by the bone's rest transform — on the 1st-person rig
    // that is COM's -90° about Y, which turns weapon yaw into roll.
    inject_capsule_defaults(hkx);
    fix_sphere_dispatch_type(hkx);
    fix_polytope_dispatch_type(hkx);
    fix_compressed_mesh_shape_headers(hkx);
    reclassify_fo76_hkb_layer(hkx);
    flatten_nested_class_names(hkx);
    migrate_unsupported_behavior_nodes(hkx, warnings);
    downgrade_fo76_cloth_to_fo4(hkx, warnings);
    strip_runtime_members(hkx, registry);
    compact_null_blender_children(hkx);
    compact_null_state_machine_states(hkx);
    compact_null_pointer_arrays(hkx);
    fix_state_machine_typed_refs(hkx);
    fix_serialized_bool_to_int32(hkx);
    drop_transitions_to_missing_states(hkx);
    fix_dangling_pointers(hkx);
    fix_variable_value_set(hkx);
    fix_clip_generator_defaults(hkx);
    normalize_behavior_reference_names(hkx);
    apply_classxml_defaults(hkx);
    rewire_character_driver_pointers(hkx);
    populate_event_property_arrays(hkx);
    reorder_behavior_metadata_to_end(hkx);
    fix_behavior_variable_infos(hkx);
    last_applied = 41;

    last_applied
}

/// FO4 resolves `hkbBehaviorReferenceGenerator::behaviorName` as a file path and
/// needs the `.hkx` extension; FO76 authors it bare, and the shared
/// `_hkbBehaviorReferenceGenerator_0_to_1` class patch truncates any extension it
/// does find. A bare name resolves to a null subgraph and FO4 faults in
/// `hkbBehaviorReferenceGenerator::updateSync`.
fn normalize_behavior_reference_names(hkx: &mut HkxFile) {
    for object in hkx.objects_mut() {
        if object.class_name != "hkbBehaviorReferenceGenerator" {
            continue;
        }
        let Some(member) = object
            .members
            .iter_mut()
            .find(|member| member.name == "behaviorName")
        else {
            continue;
        };
        let HkxValue::String { value, .. } = &mut member.value else {
            continue;
        };
        if value.is_empty() {
            continue;
        }
        let lowered = value.to_ascii_lowercase();
        if lowered.ends_with(".hkx") {
            continue;
        }
        if lowered.ends_with(".hkt") || lowered.ends_with(".hkb") {
            value.truncate(value.len() - 4);
        }
        value.push_str(".hkx");
    }
}

fn downgrade_fo76_cloth_to_fo4(hkx: &mut HkxFile, warnings: &mut Vec<String>) {
    // Newer HCL moved FO4's direct solver fields into config/feature structs;
    // preserve the authored values before the target descriptor strips them.
    for object in hkx.objects_mut() {
        for collection_name in ["localPs", "localPNs", "localPNTs", "localPNTBs"] {
            let Some(collection) = object
                .members
                .iter_mut()
                .find(|member| member.name == collection_name)
            else {
                continue;
            };
            if let Err(error) = downgrade_packed_local_blocks(&mut collection.value) {
                warnings.push(format!("{}.{collection_name}: {error}", object.class_name));
            }
        }

        for deformer_name in ["objectSpaceDeformer", "boneSpaceDeformer"] {
            let Some(deformer) = object
                .members
                .iter_mut()
                .find(|member| member.name == deformer_name)
                .and_then(|member| member.value.as_object_members_mut())
            else {
                continue;
            };
            if !deformer.iter().any(|member| member.name == "batchSizeSpu") {
                set_member_value(deformer, "batchSizeSpu", HkxValue::U16(512));
            }
        }

        if object.class_name == "hclSimulateOperator" {
            let configs = object
                .members
                .iter()
                .find(|member| member.name == "simulateOpConfigs")
                .and_then(|member| match &member.value {
                    HkxValue::Array(configs) => Some(configs),
                    _ => None,
                });
            let Some(configs) = configs else {
                continue;
            };
            if configs.len() > 1 {
                warnings.push(format!(
                    "hclSimulateOperator has {} solver configurations; FO4 supports one, using the first",
                    configs.len()
                ));
            }
            let Some(config_members) = configs.first().and_then(HkxValue::as_object_members) else {
                continue;
            };

            let sub_steps = config_members
                .iter()
                .find(|member| member.name == "subSteps")
                .and_then(|member| extract_int(&member.value));
            let solve_iterations = config_members
                .iter()
                .find(|member| member.name == "numberOfSolveIterations")
                .and_then(|member| extract_int(&member.value));
            let constraint_execution = config_members
                .iter()
                .find(|member| member.name == "constraintExecution")
                .map(|member| member.value.clone());
            let adapt_constraint_stiffness = config_members
                .iter()
                .find(|member| member.name == "adaptConstraintStiffness")
                .and_then(|member| extract_int(&member.value))
                .map(|value| value != 0);

            if let Some(value) = sub_steps {
                set_member_value(&mut object.members, "subSteps", HkxValue::U32(value as u32));
            }
            if let Some(value) = solve_iterations {
                set_member_value(
                    &mut object.members,
                    "numberOfSolveIterations",
                    HkxValue::I32(value),
                );
            }
            if let Some(value) = constraint_execution {
                set_member_value(&mut object.members, "constraintExecution", value);
            }
            if let Some(value) = adapt_constraint_stiffness {
                set_member_value(
                    &mut object.members,
                    "adaptConstraintStiffness",
                    HkxValue::Bool(value),
                );
            }
        } else if object.class_name == "hclSimClothData" {
            let simulation_info = object
                .members
                .iter()
                .find(|member| member.name == "simulationInfo")
                .and_then(|member| member.value.as_object_members());
            let collision_tolerance = simulation_info
                .and_then(|members| f32_member(members, "collisionTolerance"))
                .or_else(|| {
                    object
                        .members
                        .iter()
                        .find(|member| member.name == "landscapeCollisionData")
                        .and_then(|member| member.value.as_object_members())
                        .and_then(|members| f32_member(members, "collisionTolerance"))
                });
            let pinch_detection = simulation_info
                .and_then(|members| bool_member(members, "pinchDetectionEnabled"))
                .or_else(|| bool_member(&object.members, "pinchDetectionEnabled"));
            let landscape_collision = simulation_info
                .and_then(|members| bool_member(members, "landscapeCollisionEnabled"))
                .or_else(|| bool_member(&object.members, "landscapeCollisionEnabled"));
            let transfer_motion = simulation_info
                .and_then(|members| bool_member(members, "transferMotionEnabled"))
                .or_else(|| bool_member(&object.members, "transferMotionEnabled"));

            let Some(simulation_info) = object
                .members
                .iter_mut()
                .find(|member| member.name == "simulationInfo")
                .and_then(|member| member.value.as_object_members_mut())
            else {
                continue;
            };
            if let Some(value) = collision_tolerance {
                set_member_value(simulation_info, "collisionTolerance", HkxValue::F32(value));
            }
            if let Some(value) = pinch_detection {
                set_member_value(
                    simulation_info,
                    "pinchDetectionEnabled",
                    HkxValue::Bool(value),
                );
            }
            if let Some(value) = landscape_collision {
                set_member_value(
                    simulation_info,
                    "landscapeCollisionEnabled",
                    HkxValue::Bool(value),
                );
            }
            if let Some(value) = transfer_motion {
                set_member_value(
                    simulation_info,
                    "transferMotionEnabled",
                    HkxValue::Bool(value),
                );
            }
        }
    }
}

fn downgrade_packed_local_blocks(value: &mut HkxValue) -> Result<(), String> {
    let HkxValue::Array(blocks) = value else {
        return Ok(());
    };
    for (block_index, block) in blocks.iter_mut().enumerate() {
        let Some(members) = block.as_object_members_mut() else {
            continue;
        };
        for component_name in [
            "localPosition",
            "localNormal",
            "localTangent",
            "localBiTangent",
        ] {
            let Some(component) = members
                .iter_mut()
                .find(|member| member.name == component_name)
            else {
                continue;
            };
            let Some(flattened) = flatten_packed_vector3_array(&component.value)
                .map_err(|error| format!("block {block_index} {component_name}: {error}"))?
            else {
                continue;
            };
            component.value = HkxValue::Array(flattened);
        }
    }
    Ok(())
}

fn flatten_packed_vector3_array(value: &HkxValue) -> Result<Option<Vec<HkxValue>>, String> {
    let HkxValue::Array(vectors) = value else {
        return Ok(None);
    };
    if vectors.is_empty() || vectors[0].as_object_members().is_none() {
        return Ok(None);
    }
    if vectors.len() != 16 {
        return Err(format!(
            "expected 16 hkPackedVector3 values, got {}",
            vectors.len()
        ));
    }

    let mut flattened = Vec::with_capacity(64);
    for (vector_index, vector) in vectors.iter().enumerate() {
        let members = vector
            .as_object_members()
            .ok_or_else(|| format!("packed vector {vector_index} is not an inline object"))?;
        let values = members
            .iter()
            .find(|member| member.name == "values")
            .and_then(|member| match &member.value {
                HkxValue::Array(values) => Some(values),
                _ => None,
            })
            .ok_or_else(|| format!("packed vector {vector_index} has no values array"))?;
        if values.len() != 4 {
            return Err(format!(
                "packed vector {vector_index} expected 4 values, got {}",
                values.len()
            ));
        }
        for packed_value in values {
            flattened.push(match packed_value {
                HkxValue::I16(value) => HkxValue::I16(*value),
                HkxValue::U16(value) => HkxValue::I16(*value as i16),
                _ => {
                    return Err(format!(
                        "packed vector {vector_index} contains a non-16-bit value"
                    ));
                }
            });
        }
    }
    Ok(Some(flattened))
}

fn inject_capsule_defaults(hkx: &mut HkxFile) {
    for object in hkx.objects_mut() {
        if object.class_name != "hkbCharacterData" {
            continue;
        }
        let Some(controller_setup) = object
            .members
            .iter_mut()
            .find(|member| member.name == "characterControllerSetup")
        else {
            continue;
        };
        let HkxValue::Object(controller_members) = &mut controller_setup.value else {
            continue;
        };
        let Some(rigid_body_setup) = controller_members
            .iter_mut()
            .find(|member| member.name == "rigidBodySetup")
        else {
            continue;
        };
        let HkxValue::Object(rigid_body_members) = &mut rigid_body_setup.value else {
            continue;
        };
        if rigid_body_members
            .iter()
            .any(|member| member.name == "shapeSetup")
        {
            continue;
        }
        let Some(profiles_index) = rigid_body_members
            .iter()
            .position(|member| member.name == "collisionShapeProfiles")
        else {
            continue;
        };
        if !matches!(
            &rigid_body_members[profiles_index].value,
            HkxValue::Array(values) if values.is_empty()
        ) {
            continue;
        }

        rigid_body_members[profiles_index] = HkxMember {
            name: "shapeSetup".to_string(),
            value: HkxValue::Object(vec![
                HkxMember {
                    name: "class".to_string(),
                    value: HkxValue::String {
                        value: "hkbShapeSetup".to_string(),
                        is_null: false,
                    },
                },
                HkxMember {
                    name: "capsuleHeight".to_string(),
                    value: HkxValue::F32(1.7),
                },
                HkxMember {
                    name: "capsuleRadius".to_string(),
                    value: HkxValue::F32(0.4),
                },
                HkxMember {
                    name: "fileName".to_string(),
                    value: HkxValue::String {
                        value: String::new(),
                        is_null: false,
                    },
                },
                HkxMember {
                    name: "type".to_string(),
                    value: HkxValue::String {
                        value: "CAPSULE".to_string(),
                        is_null: false,
                    },
                },
            ]),
        };
    }
}

fn reclassify_fo76_hkb_layer(hkx: &mut HkxFile) {
    let mut layer_bindings = std::collections::BTreeSet::new();
    for object in hkx.objects_mut() {
        let is_misclassified_layer = object.class_name == "hkbBoneWeightArray"
            && object
                .members
                .iter()
                .any(|member| member.name == "generator");
        if object.class_name != "hkbLayer" && !is_misclassified_layer {
            continue;
        }

        object.class_name = "hkbLayer".to_string();
        object.signature = 1; // hkbLayer_1.xml
        if let Some(binding) = pointer_member_value(&object.members, "variableBindingSet") {
            layer_bindings.insert(binding);
        }

        let Some(bcd_index) = object
            .members
            .iter()
            .position(|member| member.name == "blendingControlData")
        else {
            continue;
        };

        let bcd_member = object.members.remove(bcd_index);
        let HkxValue::Object(sub_members) = bcd_member.value else {
            continue;
        };

        for sub_member in sub_members {
            if sub_member.name == "internalState" || sub_member.name == "fadeInOutCurve" {
                continue;
            }
            if sub_member.name == "onEventId" || sub_member.name == "offEventId" {
                if let Some(int_value) = bool_or_int_to_event_sentinel(&sub_member.value) {
                    object.members.push(HkxMember {
                        name: sub_member.name,
                        value: HkxValue::I32(int_value),
                    });
                } else {
                    object.members.push(sub_member);
                }
            } else {
                object.members.push(sub_member);
            }
        }
    }
    for index in layer_bindings {
        let Some(binding_set) = hkx.objects_mut().get_mut(index) else {
            continue;
        };
        let Some(HkxValue::Array(bindings)) = binding_set
            .members
            .iter_mut()
            .find(|member| member.name == "bindings")
            .map(|member| &mut member.value)
        else {
            continue;
        };
        for binding in bindings {
            let HkxValue::Object(members) = binding else {
                continue;
            };
            for member in members
                .iter_mut()
                .filter(|member| member.name == "memberPath")
            {
                if let HkxValue::String { value, .. } = &mut member.value {
                    if let Some(flattened) = value.strip_prefix("blendingControlData/") {
                        *value = flattened.to_string();
                    }
                }
            }
        }
    }
}

fn bool_or_int_to_event_sentinel(value: &HkxValue) -> Option<i32> {
    match value {
        HkxValue::Bool(true) => Some(-1),
        HkxValue::Bool(false) => Some(0),
        HkxValue::I8(v) => Some(if *v == -1 { -1 } else { *v as i32 }),
        HkxValue::U8(v) => Some(if *v == 0xFF { -1 } else { *v as i32 }),
        HkxValue::I16(v) => Some(if *v == -1 { -1 } else { *v as i32 }),
        HkxValue::U16(v) => Some(if *v as i64 == 0xFF { -1 } else { *v as i32 }),
        HkxValue::I32(v) => Some(if *v as i64 == 0xFF { -1 } else { *v }),
        HkxValue::U32(v) => Some(if *v as i64 == 0xFF { -1 } else { *v as i32 }),
        HkxValue::I64(v) => Some(if *v == 0xFF { -1 } else { *v as i32 }),
        HkxValue::U64(v) => Some(if *v == 0xFF { -1 } else { *v as i32 }),
        _ => None,
    }
}

/// Sample `::`-nested class names used as test-fixture assertions. The transform
/// flattens ANY class name containing `::`, not just these.
#[cfg(test)]
const KNOWN_NESTED_CLASS_RENAMES: &[(&str, &str)] = &[
    ("hkbStateMachine::StateInfo", "hkbStateMachineStateInfo"),
    (
        "hkbStateMachine::TransitionInfoArray",
        "hkbStateMachineTransitionInfoArray",
    ),
    (
        "hkbStateMachine::EventPropertyArray",
        "hkbStateMachineEventPropertyArray",
    ),
    (
        "hkbStateMachine::TransitionInfo",
        "hkbStateMachineTransitionInfo",
    ),
    (
        "hkbStateMachine::TimeInterval",
        "hkbStateMachineTimeInterval",
    ),
    (
        "hkbVariableBindingSet::Binding",
        "hkbVariableBindingSetBinding",
    ),
    (
        "hkRootLevelContainer::NamedVariant",
        "hkRootLevelContainerNamedVariant",
    ),
    (
        "hkbHandIkControlsModifier::Hand",
        "hkbHandIkControlsModifierHand",
    ),
];

/// Rename FO76 `::`-nested class names (e.g. `hkbStateMachine::StateInfo`) to
/// FO4's flat form (`hkbStateMachineStateInfo`). FO4's classxml uses flat names;
/// the SDK uses the `::` form. Any class name containing `::` is flattened by
/// stripping all `::` occurrences. Operates on top-level objects only.
fn flatten_nested_class_names(hkx: &mut HkxFile) {
    for object in hkx.objects_mut() {
        if object.class_name.contains("::") {
            object.class_name = object.class_name.replace("::", "");
        }
    }
}

fn strip_runtime_members(hkx: &mut HkxFile, registry: &mut DescriptorRegistry) {
    // First pass: strip/retype/inject on every top-level object.
    let class_names: Vec<String> = hkx.objects().iter().map(|o| o.class_name.clone()).collect();
    for (i, class_name) in class_names.iter().enumerate() {
        let members = std::mem::take(&mut hkx.objects_mut()[i].members);
        let new_members = strip_object_runtime_members(members, class_name, registry);
        hkx.objects_mut()[i].members = new_members;
    }

    // Second pass: recurse into TYPE_ARRAY members whose ctype names a known struct.
    // We need to do this on the already-processed members.
    let object_count = hkx.objects().len();
    for obj_idx in 0..object_count {
        let class_name = hkx.objects()[obj_idx].class_name.clone();
        let Ok(all_members) = registry.get_all_members(&class_name) else {
            continue;
        };
        let member_count = hkx.objects()[obj_idx].members.len();
        for member_idx in 0..member_count {
            let member_name = hkx.objects()[obj_idx].members[member_idx].name.clone();
            // Find the MemberTemplate for this member.
            let Some(mt) = all_members.iter().find(|mt| mt.name == member_name) else {
                continue;
            };
            if !matches!(
                mt.vtype,
                HkxType::Array | HkxType::SimpleArray | HkxType::RelArray
            ) {
                continue;
            }
            let ctype = mt.ctype.clone();
            if ctype.is_empty() {
                continue;
            }
            // Check registry knows this ctype.
            if registry.get(&ctype).ok().flatten().is_none() {
                continue;
            }
            // Recurse into each Object entry in the array.
            let HkxValue::Array(arr) = &mut hkx.objects_mut()[obj_idx].members[member_idx].value
            else {
                continue;
            };
            for entry in arr.iter_mut() {
                let HkxValue::Object(inline_members) = entry else {
                    continue;
                };
                let taken = std::mem::take(inline_members);
                *inline_members = strip_object_runtime_members(taken, &ctype, registry);
            }
        }
    }
}

/// Strip SERIALIZE_IGNORED members, retype mismatched serializable members,
/// and inject defaults for missing serializable members.
/// Returns the new member list. Leaves members unchanged if class is unknown.
fn strip_object_runtime_members(
    members: Vec<HkxMember>,
    class_name: &str,
    registry: &mut DescriptorRegistry,
) -> Vec<HkxMember> {
    let all_members = match registry.get_all_members(class_name) {
        Ok(m) => m,
        Err(_) => return members,
    };
    if all_members.is_empty() {
        // Unknown class — leave as-is.
        // Exception: hkRootLevelContainer / hkRootLevelContainerNamedVariant are
        // known-but-empty, so we still pass through without mutating.
        return members;
    }

    // Build map of serializable members (flags does NOT contain "SERIALIZE_IGNORED").
    // Preserve insertion order for deterministic output.
    let serializable: Vec<&crate::hkx::descriptors::MemberTemplate> = all_members
        .iter()
        .filter(|mt| !mt.flags.contains("SERIALIZE_IGNORED"))
        .collect();

    let serializable_names: std::collections::HashSet<&str> =
        serializable.iter().map(|mt| mt.name.as_str()).collect();

    // Walk existing members: drop non-serializable, retype serializable.
    let existing_names: Vec<String> = members.iter().map(|m| m.name.clone()).collect();
    let mut new_members: Vec<HkxMember> = Vec::with_capacity(serializable.len());

    for m in members {
        if !serializable_names.contains(m.name.as_str()) {
            // Not in serializable map — strip it.
            continue;
        }
        let mt = serializable.iter().find(|mt| mt.name == m.name).unwrap();
        new_members.push(HkxMember {
            name: m.name,
            value: retype_member(m.value, mt.vtype),
        });
    }

    // Inject defaults for serializable members not present in existing members.
    for mt in &serializable {
        if existing_names.iter().any(|n| n == &mt.name) {
            continue;
        }
        let Some(default_value) = default_member_for_type(mt.vtype) else {
            continue;
        };
        new_members.push(HkxMember {
            name: mt.name.clone(),
            value: default_value,
        });
    }

    new_members
}

fn retype_member(value: HkxValue, vtype: HkxType) -> HkxValue {
    match vtype {
        HkxType::Pointer | HkxType::FunctionPointer => {
            // Only retype if not already a Pointer variant.
            if matches!(value, HkxValue::Pointer(_)) {
                value
            } else {
                HkxValue::Pointer(None)
            }
        }
        HkxType::Enum | HkxType::Flags => {
            // Normalize to I32. Extract from numeric variants; default to 0.
            let int_val = match &value {
                HkxValue::I8(v) => *v as i32,
                HkxValue::U8(v) => *v as i32,
                HkxValue::I16(v) => *v as i32,
                HkxValue::U16(v) => *v as i32,
                HkxValue::I32(v) => *v,
                HkxValue::U32(v) => *v as i32,
                HkxValue::I64(v) => *v as i32,
                HkxValue::U64(v) => *v as i32,
                _ => 0,
            };
            HkxValue::I32(int_val)
        }
        HkxType::StringPtr | HkxType::CString => {
            if matches!(value, HkxValue::String { .. }) {
                value
            } else {
                let existing = match &value {
                    HkxValue::String { value, .. } => value.clone(),
                    _ => String::new(),
                };
                HkxValue::String {
                    value: existing,
                    is_null: false,
                }
            }
        }
        HkxType::Uint32 => {
            if let Some(serial_and_index) = serial_and_index_value(&value) {
                HkxValue::U32(serial_and_index as u32)
            } else {
                value
            }
        }
        _ => value,
    }
}

fn serial_and_index_value(value: &HkxValue) -> Option<i32> {
    let members = value.as_object_members()?;
    members
        .iter()
        .find(|m| m.name == "serialAndIndex")
        .and_then(|m| extract_int(&m.value))
}

fn default_member_for_type(vtype: HkxType) -> Option<HkxValue> {
    match vtype {
        HkxType::Pointer | HkxType::FunctionPointer => Some(HkxValue::Pointer(None)),
        HkxType::StringPtr | HkxType::CString => Some(HkxValue::String {
            value: String::new(),
            is_null: false,
        }),
        HkxType::Enum | HkxType::Flags => Some(HkxValue::I32(0)),
        HkxType::Array | HkxType::SimpleArray | HkxType::RelArray => Some(HkxValue::Array(vec![])),
        HkxType::Bool => Some(HkxValue::Bool(false)),
        HkxType::Int8 => Some(HkxValue::I8(0)),
        HkxType::Uint8 => Some(HkxValue::U8(0)),
        HkxType::Int16 => Some(HkxValue::I16(0)),
        HkxType::Uint16 | HkxType::Half => Some(HkxValue::U16(0)),
        HkxType::Int32 => Some(HkxValue::I32(0)),
        HkxType::Uint32 => Some(HkxValue::U32(0)),
        HkxType::Int64 => Some(HkxValue::I64(0)),
        HkxType::Uint64 | HkxType::Ulong => Some(HkxValue::U64(0)),
        HkxType::Real => Some(HkxValue::F32(0.0)),
        // Struct / Vector4 / Quaternion / Matrix* / Transform / QsTransform / Void
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Behavior-graph cleanup transforms (indices 28..=34)
// ---------------------------------------------------------------------------

fn compact_null_blender_children(hkx: &mut HkxFile) {
    for object in hkx.objects_mut() {
        if object.class_name != "hkbBlenderGenerator"
            && object.class_name != "hkbPoseMatchingGenerator"
        {
            continue;
        }
        let Some(children_member) = object.members.iter_mut().find(|m| m.name == "children") else {
            continue;
        };
        let HkxValue::Array(values) = &mut children_member.value else {
            continue;
        };
        values.retain(|v| matches!(v, HkxValue::Pointer(Some(_))));
    }
}

fn compact_null_state_machine_states(hkx: &mut HkxFile) {
    // Standalone behavior files: recover orphan StateInfos before truncating.
    // NIF-embedded blobs (no hkRootLevelContainer): strip nulls only.
    let is_standalone = hkx
        .objects()
        .iter()
        .any(|o| o.class_name == "hkRootLevelContainer");

    if !is_standalone {
        for object in hkx.objects_mut() {
            if object.class_name != "hkbStateMachine" {
                continue;
            }
            let Some(states_member) = object.members.iter_mut().find(|m| m.name == "states") else {
                continue;
            };
            let HkxValue::Array(values) = &mut states_member.value else {
                continue;
            };
            values.retain(|v| matches!(v, HkxValue::Pointer(Some(_))));
        }
        return;
    }

    // --- Standalone path: recover orphans ---

    // 1. Collect all StateInfo object indices and (per-index) name + stateId.
    let mut all_state_indices: Vec<usize> = Vec::new();
    let mut state_name: std::collections::HashMap<usize, String> = std::collections::HashMap::new();
    let mut state_id: std::collections::HashMap<usize, i32> = std::collections::HashMap::new();
    for (idx, obj) in hkx.objects().iter().enumerate() {
        if obj.class_name != "hkbStateMachineStateInfo" {
            continue;
        }
        all_state_indices.push(idx);
        let name = obj
            .members
            .iter()
            .find(|m| m.name == "name")
            .and_then(|m| match &m.value {
                HkxValue::String { value, .. } => Some(value.clone()),
                _ => None,
            })
            .unwrap_or_default();
        let id = obj
            .members
            .iter()
            .find(|m| m.name == "stateId")
            .map(|m| match m.value {
                HkxValue::I32(v) => v,
                HkxValue::U32(v) => v as i32,
                _ => i32::MIN,
            })
            .unwrap_or(i32::MIN);
        state_name.insert(idx, name);
        state_id.insert(idx, id);
    }

    // 2. Collect indices of state machines with null slots, with their name.
    struct SmEntry {
        obj_index: usize,
        sm_name: String,
        null_count: usize,
    }
    let mut sm_entries: Vec<SmEntry> = Vec::new();
    let mut owned_refs: std::collections::HashSet<usize> = std::collections::HashSet::new();
    for (idx, obj) in hkx.objects().iter().enumerate() {
        if obj.class_name != "hkbStateMachine" {
            continue;
        }
        let sm_name = obj
            .members
            .iter()
            .find(|m| m.name == "name")
            .and_then(|m| match &m.value {
                HkxValue::String { value, .. } => Some(value.clone()),
                _ => None,
            })
            .unwrap_or_default();
        let mut null_count = 0;
        if let Some(states_member) = obj.members.iter().find(|m| m.name == "states") {
            if let HkxValue::Array(arr) = &states_member.value {
                for v in arr {
                    match v {
                        HkxValue::Pointer(Some(target)) => {
                            owned_refs.insert(*target);
                        }
                        HkxValue::Pointer(None) => {
                            null_count += 1;
                        }
                        _ => {}
                    }
                }
            }
        }
        if null_count > 0 {
            sm_entries.push(SmEntry {
                obj_index: idx,
                sm_name,
                null_count,
            });
        }
    }

    if sm_entries.is_empty() {
        return;
    }

    // 3. Orphan pool: StateInfo indices not referenced by any SM.
    let mut orphan_pool: Vec<usize> = all_state_indices
        .iter()
        .copied()
        .filter(|i| !owned_refs.contains(i))
        .collect();
    orphan_pool.sort_by(|left, right| {
        state_sort_key(*left, &state_id, &state_name).cmp(&state_sort_key(
            *right,
            &state_id,
            &state_name,
        ))
    });

    // 4. Identify the "root" SM (highest null count); others are sub-SMs that
    //    get matched first by name affinity + stateId range.
    let root_pos = sm_entries
        .iter()
        .enumerate()
        .max_by_key(|(_, e)| e.null_count)
        .map(|(i, _)| i)
        .unwrap();

    let mut used_orphans: std::collections::HashSet<usize> = std::collections::HashSet::new();

    // Phase 1: sub-SMs.
    let sub_indices: Vec<usize> = (0..sm_entries.len()).filter(|i| *i != root_pos).collect();
    for sub_idx in sub_indices {
        let entry = &sm_entries[sub_idx];
        let sm_obj_index = entry.obj_index;
        let sm_name = entry.sm_name.clone();
        let null_count = entry.null_count;
        let sm_base = sm_base_name(&sm_name);

        // Snapshot existing slots' names + ids before mutation.
        let mut existing_names: Vec<String> = Vec::new();
        let mut existing_ids: std::collections::HashSet<i32> = std::collections::HashSet::new();
        let original: Vec<HkxValue> = match &hkx.objects()[sm_obj_index]
            .members
            .iter()
            .find(|m| m.name == "states")
            .map(|m| &m.value)
        {
            Some(HkxValue::Array(arr)) => arr.clone(),
            _ => continue,
        };
        for v in &original {
            if let HkxValue::Pointer(Some(target)) = v {
                if let Some(n) = state_name.get(target) {
                    existing_names.push(n.clone());
                }
                if let Some(id) = state_id.get(target) {
                    existing_ids.insert(*id);
                }
            }
        }

        // Find matching orphans.
        let mut matches: Vec<(i32, usize)> = Vec::new();
        for &cand in orphan_pool.iter() {
            if used_orphans.contains(&cand) {
                continue;
            }
            let oid = *state_id.get(&cand).unwrap_or(&i32::MIN);
            if existing_ids.contains(&oid) {
                continue;
            }
            let oname = state_name.get(&cand).cloned().unwrap_or_default();
            if orphan_matches_sub_sm(
                &oname,
                oid,
                &sm_base,
                &existing_names,
                &existing_ids,
                null_count,
            ) {
                matches.push((oid, cand));
            }
        }
        matches.sort_by(|(_, left), (_, right)| {
            state_sort_key(*left, &state_id, &state_name).cmp(&state_sort_key(
                *right,
                &state_id,
                &state_name,
            ))
        });

        // Fill null slots.
        let mut new_contents: Vec<HkxValue> = Vec::with_capacity(original.len());
        let mut match_iter = matches.into_iter();
        for v in original {
            match v {
                HkxValue::Pointer(Some(_)) => new_contents.push(v),
                HkxValue::Pointer(None) => {
                    if let Some((_, cand)) = match_iter.next() {
                        new_contents.push(HkxValue::Pointer(Some(cand)));
                        used_orphans.insert(cand);
                    }
                    // else: drop the null slot
                }
                other => new_contents.push(other),
            }
        }
        if let Some(states_member) = hkx.objects_mut()[sm_obj_index]
            .members
            .iter_mut()
            .find(|m| m.name == "states")
        {
            states_member.value = HkxValue::Array(new_contents);
        }
    }

    // Refresh orphan_pool by removing used orphans.
    orphan_pool.retain(|i| !used_orphans.contains(i));

    // Phase 2: root SM gets all remaining orphans (sorted by stateId), filling
    // null slots first, then appending if any extras remain.
    let root_entry = &sm_entries[root_pos];
    let root_obj_index = root_entry.obj_index;

    let original: Vec<HkxValue> = match &hkx.objects()[root_obj_index]
        .members
        .iter()
        .find(|m| m.name == "states")
        .map(|m| &m.value)
    {
        Some(HkxValue::Array(arr)) => arr.clone(),
        _ => return,
    };
    let mut existing_root_ids: std::collections::HashSet<i32> = std::collections::HashSet::new();
    for v in &original {
        if let HkxValue::Pointer(Some(target)) = v {
            if let Some(id) = state_id.get(target) {
                existing_root_ids.insert(*id);
            }
        }
    }

    let mut remaining: Vec<usize> = orphan_pool
        .iter()
        .copied()
        .filter(|cand| {
            let oid = *state_id.get(cand).unwrap_or(&i32::MIN);
            !existing_root_ids.contains(&oid)
        })
        .collect();
    remaining.sort_by(|left, right| {
        state_sort_key(*left, &state_id, &state_name).cmp(&state_sort_key(
            *right,
            &state_id,
            &state_name,
        ))
    });

    let mut new_contents: Vec<HkxValue> = Vec::with_capacity(original.len() + remaining.len());
    let mut remain_iter = remaining.into_iter();
    for v in original {
        match v {
            HkxValue::Pointer(Some(_)) => new_contents.push(v),
            HkxValue::Pointer(None) => {
                if let Some(cand) = remain_iter.next() {
                    new_contents.push(HkxValue::Pointer(Some(cand)));
                    used_orphans.insert(cand);
                }
            }
            other => new_contents.push(other),
        }
    }
    // Append any leftover orphans (more orphans than null slots).
    for cand in remain_iter {
        new_contents.push(HkxValue::Pointer(Some(cand)));
        used_orphans.insert(cand);
    }
    if let Some(states_member) = hkx.objects_mut()[root_obj_index]
        .members
        .iter_mut()
        .find(|m| m.name == "states")
    {
        states_member.value = HkxValue::Array(new_contents);
    }

    // Final sweep: strip any null slots still present in any SM.
    for object in hkx.objects_mut() {
        if object.class_name != "hkbStateMachine" {
            continue;
        }
        let Some(states_member) = object.members.iter_mut().find(|m| m.name == "states") else {
            continue;
        };
        let HkxValue::Array(values) = &mut states_member.value else {
            continue;
        };
        values.retain(|v| matches!(v, HkxValue::Pointer(Some(_))));
    }
}

fn state_sort_key<'a>(
    index: usize,
    state_id: &std::collections::HashMap<usize, i32>,
    state_name: &'a std::collections::HashMap<usize, String>,
) -> (i32, &'a str, usize) {
    (
        *state_id.get(&index).unwrap_or(&i32::MIN),
        state_name.get(&index).map(String::as_str).unwrap_or(""),
        index,
    )
}

fn sm_base_name(sm_name: &str) -> String {
    for suffix in ["_NonStrafing_SM", "_SM", "StateMachine"] {
        if let Some(stripped) = sm_name.strip_suffix(suffix) {
            return stripped.to_string();
        }
    }
    sm_name.to_string()
}

fn name_tokens(name: &str) -> std::collections::HashSet<String> {
    // Split on '_' and whitespace, then split camelCase. Drop tokens shorter
    // than 2 chars and the noise tokens "sm" / "statemachine".
    let mut tokens = std::collections::HashSet::new();
    for raw_part in name.split(|c: char| c == '_' || c.is_whitespace()) {
        if raw_part.is_empty() {
            continue;
        }
        // Always include the lowercased whole part.
        tokens.insert(raw_part.to_lowercase());
        // Split camelCase: each upper-case starts a new sub-token.
        let mut current = String::new();
        for ch in raw_part.chars() {
            if ch.is_uppercase() && !current.is_empty() {
                if current.len() > 1 {
                    tokens.insert(current.to_lowercase());
                }
                current = String::new();
            }
            current.push(ch);
        }
        if current.len() > 1 {
            tokens.insert(current.to_lowercase());
        }
    }
    tokens.remove("");
    tokens.remove("sm");
    tokens.remove("statemachine");
    tokens
}

fn orphan_matches_sub_sm(
    orphan_name: &str,
    orphan_id: i32,
    sm_base: &str,
    existing_names: &[String],
    existing_ids: &std::collections::HashSet<i32>,
    null_count: usize,
) -> bool {
    let sm_base_lower = sm_base.to_lowercase();
    let name_lower = orphan_name.to_lowercase();

    let mut name_match = name_lower.starts_with(&sm_base_lower)
        || sm_base_lower.starts_with(&name_lower)
        || name_lower.contains(&sm_base_lower)
        || sm_base_lower.contains(&name_lower);

    if !name_match {
        let orph_tokens = name_tokens(orphan_name);
        for existing in existing_names {
            let ex_tokens = name_tokens(existing);
            if orph_tokens.intersection(&ex_tokens).next().is_some() {
                name_match = true;
                break;
            }
        }
    }

    if !name_match {
        return false;
    }

    let max_id = existing_ids.iter().copied().max().unwrap_or(-1);
    orphan_id <= max_id + null_count as i32 + 2
}

// (class_name, member_name) pairs that hold pointer arrays which need null compaction.
const NULL_POINTER_ARRAY_TARGETS: &[(&str, &str)] = &[
    ("hkbLayerGenerator", "layers"),
    ("hkbModifierList", "modifiers"),
    ("hkbManualSelectorGenerator", "generators"),
];

fn compact_null_pointer_arrays(hkx: &mut HkxFile) {
    for object in hkx.objects_mut() {
        let Some(&(_, member_name)) = NULL_POINTER_ARRAY_TARGETS
            .iter()
            .find(|(cn, _)| *cn == object.class_name.as_str())
        else {
            continue;
        };
        let Some(target_member) = object.members.iter_mut().find(|m| m.name == member_name) else {
            continue;
        };
        let HkxValue::Array(values) = &mut target_member.value else {
            continue;
        };
        values.retain(|v| matches!(v, HkxValue::Pointer(Some(_))));
    }
}

const STATE_MACHINE_UNSET_REFS: &[&str] = &[
    "returnToPreviousStateEventId",
    "randomTransitionEventId",
    "transitionToNextHigherStateEventId",
    "transitionToNextLowerStateEventId",
    "syncVariableIndex",
];

fn fix_state_machine_typed_refs(hkx: &mut HkxFile) {
    for object in hkx.objects_mut() {
        if object.class_name != "hkbStateMachine" {
            continue;
        }
        for member in &mut object.members {
            if !STATE_MACHINE_UNSET_REFS.contains(&member.name.as_str()) {
                continue;
            }
            // Coerce any integer-width or bool value → I32(-1).
            // Fresh-from-TAG0 reads may produce I8/I16/U8/U16 for these fields;
            // FO4 classxml expects i32 for all five ref fields.
            match &member.value {
                HkxValue::Bool(_) => member.value = HkxValue::I32(-1),
                HkxValue::I8(v) => member.value = HkxValue::I32(*v as i32),
                HkxValue::I16(v) => member.value = HkxValue::I32(*v as i32),
                HkxValue::U8(v) => member.value = HkxValue::I32(*v as i32),
                HkxValue::U16(v) => member.value = HkxValue::I32(*v as i32),
                _ => {}
            }
        }
    }
}

// (class_name, direct_field, companion_array_name)
const SERIALIZED_BOOL_INT32_FIELDS: &[(&str, &str, &str)] = &[
    ("hkbStateMachine", "startStateId", "states"),
    (
        "hkbVariableBindingSet",
        "indexOfBindingToEnable",
        "bindings",
    ),
];

fn fix_serialized_bool_to_int32(hkx: &mut HkxFile) {
    for object in hkx.objects_mut() {
        let Some(&(_, direct_field, companion_array_name)) = SERIALIZED_BOOL_INT32_FIELDS
            .iter()
            .find(|(cn, _, _)| *cn == object.class_name.as_str())
        else {
            continue;
        };

        // Find companion array length.
        let companion_len = object
            .members
            .iter()
            .find(|m| m.name == companion_array_name)
            .and_then(|m| {
                if let HkxValue::Array(values) = &m.value {
                    Some(values.len())
                } else {
                    None
                }
            })
            .unwrap_or(0);

        // Find and fix the direct field.
        let Some(direct_member) = object.members.iter_mut().find(|m| m.name == direct_field) else {
            continue;
        };
        let HkxValue::Bool(b) = direct_member.value else {
            continue;
        };
        let raw: usize = if b { 1 } else { 0 };
        direct_member.value = if raw >= companion_len {
            HkxValue::I32(-1)
        } else {
            HkxValue::I32(raw as i32)
        };
    }
}

fn drop_transitions_to_missing_states(hkx: &mut HkxFile) {
    use std::collections::HashSet;

    let object_count = hkx.objects().len();

    // Collect all state machine indices.
    let sm_indices: Vec<usize> = (0..object_count)
        .filter(|&i| hkx.objects()[i].class_name == "hkbStateMachine")
        .collect();

    for sm_idx in sm_indices {
        // Collect valid state IDs by resolving the SM's `states` array.
        let valid_state_ids: HashSet<i32> = {
            let states_array: Vec<usize> = hkx.objects()[sm_idx]
                .members
                .iter()
                .find(|m| m.name == "states")
                .and_then(|m| {
                    if let HkxValue::Array(values) = &m.value {
                        Some(
                            values
                                .iter()
                                .filter_map(|v| {
                                    if let HkxValue::Pointer(Some(idx)) = v {
                                        Some(*idx)
                                    } else {
                                        None
                                    }
                                })
                                .collect(),
                        )
                    } else {
                        None
                    }
                })
                .unwrap_or_default();

            states_array
                .iter()
                .filter_map(|&si_idx| {
                    if si_idx >= object_count {
                        return None;
                    }
                    hkx.objects()[si_idx]
                        .members
                        .iter()
                        .find(|m| m.name == "stateId")
                        .and_then(|m| match &m.value {
                            HkxValue::I8(v) => Some(*v as i32),
                            HkxValue::U8(v) => Some(*v as i32),
                            HkxValue::I16(v) => Some(*v as i32),
                            HkxValue::U16(v) => Some(*v as i32),
                            HkxValue::I32(v) => Some(*v),
                            HkxValue::U32(v) => Some(*v as i32),
                            HkxValue::I64(v) => Some(*v as i32),
                            HkxValue::U64(v) => Some(*v as i32),
                            _ => None,
                        })
                })
                .collect()
        };

        if valid_state_ids.is_empty() {
            continue;
        }

        // Collect transition array indices reachable from this SM.
        let mut transition_array_indices: Vec<usize> = Vec::new();

        // SM's wildcardTransitions pointer.
        if let Some(wc_idx) = hkx.objects()[sm_idx]
            .members
            .iter()
            .find(|m| m.name == "wildcardTransitions")
            .and_then(|m| {
                if let HkxValue::Pointer(Some(idx)) = &m.value {
                    Some(*idx)
                } else {
                    None
                }
            })
        {
            if wc_idx < object_count
                && hkx.objects()[wc_idx].class_name == "hkbStateMachineTransitionInfoArray"
            {
                transition_array_indices.push(wc_idx);
            }
        }

        // Each state info's `transitions` pointer.
        let state_info_indices: Vec<usize> = hkx.objects()[sm_idx]
            .members
            .iter()
            .find(|m| m.name == "states")
            .and_then(|m| {
                if let HkxValue::Array(values) = &m.value {
                    Some(
                        values
                            .iter()
                            .filter_map(|v| {
                                if let HkxValue::Pointer(Some(idx)) = v {
                                    Some(*idx)
                                } else {
                                    None
                                }
                            })
                            .collect(),
                    )
                } else {
                    None
                }
            })
            .unwrap_or_default();

        for si_idx in state_info_indices {
            if si_idx >= object_count {
                continue;
            }
            if let Some(ta_idx) = hkx.objects()[si_idx]
                .members
                .iter()
                .find(|m| m.name == "transitions")
                .and_then(|m| {
                    if let HkxValue::Pointer(Some(idx)) = &m.value {
                        Some(*idx)
                    } else {
                        None
                    }
                })
            {
                if ta_idx < object_count
                    && hkx.objects()[ta_idx].class_name == "hkbStateMachineTransitionInfoArray"
                {
                    transition_array_indices.push(ta_idx);
                }
            }
        }

        // For each transition array, filter out entries whose toStateId is not in valid_state_ids.
        for ta_idx in transition_array_indices {
            let Some(transitions_member) = hkx.objects_mut()[ta_idx]
                .members
                .iter_mut()
                .find(|m| m.name == "transitions")
            else {
                continue;
            };
            let HkxValue::Array(entries) = &mut transitions_member.value else {
                continue;
            };
            entries.retain(|entry| {
                let HkxValue::Object(sub_members) = entry else {
                    return true;
                };
                let Some(to_state_id) = sub_members
                    .iter()
                    .find(|m| m.name == "toStateId")
                    .and_then(|m| match &m.value {
                        HkxValue::I8(v) => Some(*v as i32),
                        HkxValue::U8(v) => Some(*v as i32),
                        HkxValue::I16(v) => Some(*v as i32),
                        HkxValue::U16(v) => Some(*v as i32),
                        HkxValue::I32(v) => Some(*v),
                        HkxValue::U32(v) => Some(*v as i32),
                        HkxValue::I64(v) => Some(*v as i32),
                        HkxValue::U64(v) => Some(*v as i32),
                        _ => None,
                    })
                else {
                    // toStateId absent or non-integer — keep entry, don't panic.
                    return true;
                };
                valid_state_ids.contains(&to_state_id)
            });
        }
    }
}

fn fix_dangling_pointers(hkx: &mut HkxFile) {
    let object_count = hkx.objects().len();

    fn fix_value(value: &mut HkxValue, object_count: usize) {
        match value {
            HkxValue::Pointer(Some(idx)) if *idx >= object_count => {
                *value = HkxValue::Pointer(None);
            }
            HkxValue::Object(members) => {
                for m in members {
                    fix_value(&mut m.value, object_count);
                }
            }
            HkxValue::Array(values) => {
                for v in values {
                    fix_value(v, object_count);
                }
            }
            _ => {}
        }
    }

    let obj_count = object_count;
    for object in hkx.objects_mut() {
        for member in &mut object.members {
            fix_value(&mut member.value, obj_count);
        }
    }
}

fn rewire_character_driver_pointers(hkx: &mut HkxFile) {
    const DRIVER_FIELDS: [(&str, &str); 4] = [
        ("mirroredSkeletonInfo", "hkbMirroredSkeletonInfo"),
        ("footIkDriverInfo", "hkbFootIkDriverInfo"),
        ("handIkDriverInfo", "hkbHandIkDriverInfo"),
        ("aiControlDriverInfo", "hkbAiControlDriverInfo"),
    ];

    let targets: Vec<Option<usize>> = DRIVER_FIELDS
        .iter()
        .map(|(_, class_name)| {
            hkx.objects()
                .iter()
                .position(|object| object.class_name == *class_name)
        })
        .collect();

    for object in hkx.objects_mut() {
        if object.class_name != "hkbCharacterData" {
            continue;
        }
        for ((field_name, _), target) in DRIVER_FIELDS.iter().zip(targets.iter()) {
            let Some(target) = target else {
                continue;
            };
            let Some(member) = object
                .members
                .iter_mut()
                .find(|member| member.name == *field_name)
            else {
                continue;
            };
            if matches!(member.value, HkxValue::Pointer(None)) {
                member.value = HkxValue::Pointer(Some(*target));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Behavior-graph transforms (ALWAYS_ON_TRANSFORMS indices 35..=41)
// ---------------------------------------------------------------------------

fn fix_variable_value_set(hkx: &mut HkxFile) {
    for object in hkx.objects_mut() {
        if object.class_name != "hkbVariableValueSet" {
            continue;
        }
        let Some(word_values) = object
            .members
            .iter_mut()
            .find(|m| m.name == "wordVariableValues")
        else {
            continue;
        };
        let HkxValue::Array(entries) = &mut word_values.value else {
            continue;
        };
        if entries.is_empty() {
            continue;
        }
        // Already wrapped — nothing to do.
        if matches!(entries[0], HkxValue::Object(_)) {
            continue;
        }
        // Wrap raw int scalars as hkbVariableValue inline objects.
        let wrapped: Vec<HkxValue> = entries
            .iter()
            .map(|v| {
                let int_val: i32 = match v {
                    HkxValue::I8(n) => *n as i32,
                    HkxValue::U8(n) => *n as i32,
                    HkxValue::I16(n) => *n as i32,
                    HkxValue::U16(n) => *n as i32,
                    HkxValue::I32(n) => *n,
                    HkxValue::U32(n) => *n as i32,
                    HkxValue::I64(n) => *n as i32,
                    HkxValue::U64(n) => *n as i32,
                    _ => 0,
                };
                HkxValue::Object(vec![HkxMember {
                    name: "value".to_string(),
                    value: HkxValue::I32(int_val),
                }])
            })
            .collect();
        *entries = wrapped;
    }
}

fn fix_clip_generator_defaults(hkx: &mut HkxFile) {
    for object in hkx.objects_mut() {
        if object.class_name != "hkbClipGenerator" {
            continue;
        }
        if let Some(member) = object
            .members
            .iter_mut()
            .find(|m| m.name == "animationBindingIndex")
        {
            // Force to -1 regardless of current int width.
            match &mut member.value {
                HkxValue::I8(v) if *v != -1 => *v = -1,
                HkxValue::I16(v) if *v != -1 => *v = -1,
                HkxValue::I32(v) if *v != -1 => *v = -1,
                HkxValue::I64(v) if *v != -1 => *v = -1,
                _ => {}
            }
        } else {
            object.members.push(HkxMember {
                name: "animationBindingIndex".to_string(),
                value: HkxValue::I16(-1),
            });
        }
    }
}

// Returns (class_name, member_name, default_value) for known-zero fields in FO76→FO4.
fn classxml_safe_defaults() -> &'static [(&'static str, &'static str, fn() -> HkxValue)] {
    &[
        (
            "hkbLayer",
            "weight",
            (|| HkxValue::F32(1.0)) as fn() -> HkxValue,
        ),
        ("hkbLayer", "onEventId", || HkxValue::I32(-1)),
        ("hkbLayer", "offEventId", || HkxValue::I32(-1)),
        ("hkbBlenderGeneratorChild", "worldFromModelWeight", || {
            HkxValue::F32(1.0)
        }),
        ("hkbStateMachineStateInfo", "probability", || {
            HkxValue::F32(1.0)
        }),
        ("hkbStateMachineStateInfo", "enable", || {
            HkxValue::Bool(true)
        }),
        ("hkbModifier", "enable", || HkxValue::Bool(true)),
    ]
}

fn is_zero_equivalent(value: &HkxValue) -> bool {
    matches!(
        value,
        HkxValue::I8(0)
            | HkxValue::U8(0)
            | HkxValue::I16(0)
            | HkxValue::U16(0)
            | HkxValue::I32(0)
            | HkxValue::U32(0)
            | HkxValue::I64(0)
            | HkxValue::U64(0)
            | HkxValue::Bool(false)
    ) || matches!(value, HkxValue::F32(f) if *f == 0.0)
}

fn apply_classxml_defaults(hkx: &mut HkxFile) {
    // Inline objects have no class_name (lost at TAG0 materialization), so
    // inline-struct defaults are not applied.
    let object_count = hkx.objects().len();
    for i in 0..object_count {
        let class_name = hkx.objects()[i].class_name.clone();
        let member_count = hkx.objects()[i].members.len();
        for j in 0..member_count {
            let member_name = hkx.objects()[i].members[j].name.clone();
            let Some((_, _, make_default)) = classxml_safe_defaults()
                .iter()
                .find(|(cn, mn, _)| *cn == class_name && *mn == member_name)
            else {
                continue;
            };
            if is_zero_equivalent(&hkx.objects()[i].members[j].value) {
                hkx.objects_mut()[i].members[j].value = make_default();
            }
        }
    }

    // Pass 2: for each hkbLayerGenerator, set first layer's onByDefault = true.
    for i in 0..hkx.objects().len() {
        if hkx.objects()[i].class_name != "hkbLayerGenerator" {
            continue;
        }
        let first_layer_ptr: Option<usize> = hkx.objects()[i]
            .members
            .iter()
            .find(|m| m.name == "layers")
            .and_then(|m| {
                if let HkxValue::Array(arr) = &m.value {
                    arr.first().and_then(|v| {
                        if let HkxValue::Pointer(Some(idx)) = v {
                            Some(*idx)
                        } else {
                            None
                        }
                    })
                } else {
                    None
                }
            });
        let Some(layer_idx) = first_layer_ptr else {
            continue;
        };
        if layer_idx >= hkx.objects().len() {
            continue;
        }
        if hkx.objects()[layer_idx].class_name != "hkbLayer" {
            continue;
        }
        let on_by_default_idx = hkx.objects()[layer_idx]
            .members
            .iter()
            .position(|m| m.name == "onByDefault");
        let Some(mj) = on_by_default_idx else {
            continue;
        };
        if hkx.objects()[layer_idx].members[mj].value == HkxValue::Bool(false) {
            hkx.objects_mut()[layer_idx].members[mj].value = HkxValue::Bool(true);
        }
    }
}

// ---------------------------------------------------------------------------
// EPA population
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EpaDir {
    Enter,
    Exit,
}

/// One entry in the EPA rule table: (keyword, list of (direction, lowercase
/// event-name pattern, optional payload string)).
struct EpaRule {
    keyword: &'static str,
    actions: &'static [(EpaDir, &'static str, Option<&'static str>)],
}

const EPA_RULES: &[EpaRule] = &[
    // Stagger states: EXIT staggerStop
    EpaRule {
        keyword: "stagger",
        actions: &[(EpaDir::Exit, "staggerstop", None)],
    },
    // Death states: ENTER AddRagdollToWorld
    EpaRule {
        keyword: "death",
        actions: &[(EpaDir::Enter, "addragdolltoworld", None)],
    },
    // Ragdoll states
    EpaRule {
        keyword: "ragdoll",
        actions: &[
            (EpaDir::Enter, "removecharactercontrollerfromworld", None),
            (EpaDir::Enter, "enterfullyragdoll", None),
        ],
    },
    // Recoil states: EXIT recoilStop
    EpaRule {
        keyword: "recoil",
        actions: &[(EpaDir::Exit, "recoilstop", None)],
    },
    // Melee attack states: EXIT AttackStop + startAnimationDriven
    EpaRule {
        keyword: "meleestart",
        actions: &[
            (EpaDir::Exit, "attackstop", None),
            (EpaDir::Exit, "startanimationdriven", None),
        ],
    },
    EpaRule {
        keyword: "melee_",
        actions: &[
            (EpaDir::Exit, "attackstop", None),
            (EpaDir::Exit, "startanimationdriven", None),
        ],
    },
    // Fire/ranged attack: ENTER+EXIT AttackState2 with Enter/Exit payloads.
    EpaRule {
        keyword: "firesingle",
        actions: &[
            (EpaDir::Enter, "attackstate2", Some("Enter")),
            (EpaDir::Exit, "attackstate2", Some("Exit")),
        ],
    },
    EpaRule {
        keyword: "fireauto",
        actions: &[
            (EpaDir::Enter, "attackstate2", Some("Enter")),
            (EpaDir::Exit, "attackstate2", Some("Exit")),
        ],
    },
    // Hit reaction: ENTER HeadTrackingOff, EXIT HeadTrackingOn
    EpaRule {
        keyword: "hitreaction",
        actions: &[
            (EpaDir::Enter, "headtrackingoff", None),
            (EpaDir::Exit, "headtrackingon", None),
        ],
    },
    EpaRule {
        keyword: "flinch",
        actions: &[
            (EpaDir::Enter, "headtrackingoff", None),
            (EpaDir::Exit, "headtrackingon", None),
        ],
    },
    // Paired animations
    EpaRule {
        keyword: "paired",
        actions: &[
            (EpaDir::Enter, "attackstop", None),
            (EpaDir::Enter, "pairedstop", None),
        ],
    },
    EpaRule {
        keyword: "killmove",
        actions: &[
            (EpaDir::Enter, "attackstop", None),
            (EpaDir::Enter, "pairedstop", None),
        ],
    },
    // Draw/Sheathe (combat toggle)
    EpaRule {
        keyword: "weapondraw",
        actions: &[
            (EpaDir::Enter, "weapondraw", None),
            (EpaDir::Enter, "enablebumper", None),
        ],
    },
    EpaRule {
        keyword: "weaponsheathe",
        actions: &[
            (EpaDir::Enter, "weaponsheathe", None),
            (EpaDir::Enter, "disablebumper", None),
        ],
    },
    EpaRule {
        keyword: "forceequip",
        actions: &[
            (EpaDir::Enter, "weapondraw", None),
            (EpaDir::Enter, "enablebumper", None),
        ],
    },
    EpaRule {
        keyword: "draw",
        actions: &[
            (EpaDir::Enter, "weapondraw", None),
            (EpaDir::Enter, "enablebumper", None),
        ],
    },
    EpaRule {
        keyword: "sheathe",
        actions: &[
            (EpaDir::Enter, "weaponsheathe", None),
            (EpaDir::Enter, "disablebumper", None),
        ],
    },
    // Paired animation (duplicate entry)
    EpaRule {
        keyword: "paired",
        actions: &[
            (EpaDir::Enter, "attackstop", None),
            (EpaDir::Enter, "pairedstop", None),
        ],
    },
    EpaRule {
        keyword: "killmove",
        actions: &[
            (EpaDir::Enter, "attackstop", None),
            (EpaDir::Enter, "pairedstop", None),
        ],
    },
];

const GETUP_EXIT_EVENT: &str = "GetUpEnd";

/// Match a state's signal set against the EPA rule table. Returns the first
/// matching rule's actions (first keyword match wins).
fn match_epa_rule(
    signals: &[String],
) -> Option<&'static [(EpaDir, &'static str, Option<&'static str>)]> {
    for rule in EPA_RULES {
        let kw = rule.keyword; // already lowercase in the table
        for sig in signals {
            // case-insensitive substring (kw is already lowercase)
            if sig.to_ascii_lowercase().contains(kw) {
                return Some(rule.actions);
            }
        }
    }
    None
}

fn populate_event_property_arrays(hkx: &mut HkxFile) {
    // Quick check: must be a behavior file.
    if !hkx
        .objects()
        .iter()
        .any(|o| o.class_name.contains("BehaviorGraphStringData"))
    {
        return;
    }

    // --- Build event-name lookup from hkbBehaviorGraphStringData.eventNames ---
    let (event_name_to_id, event_name_ci, id_to_name) = build_event_name_table(hkx);
    if event_name_to_id.is_empty() {
        return;
    }

    // --- Index name → object index ---
    let name_to_idx: std::collections::HashMap<String, usize> = hkx
        .objects()
        .iter()
        .enumerate()
        .filter_map(|(i, o)| o.name.clone().map(|n| (n, i)))
        .collect();

    // --- Per-state-machine: collect incoming-transition event names per state ---
    let mut incoming_transitions: std::collections::HashMap<
        usize,
        std::collections::HashSet<String>,
    > = std::collections::HashMap::new();
    for sm_idx in 0..hkx.objects().len() {
        if hkx.objects()[sm_idx].class_name != "hkbStateMachine" {
            continue;
        }
        collect_sm_transitions(
            hkx.objects(),
            sm_idx,
            &name_to_idx,
            &id_to_name,
            &mut incoming_transitions,
        );
    }

    // --- Generator-chain keywords for each state ---
    let mut generator_keywords: std::collections::HashMap<
        usize,
        std::collections::HashSet<String>,
    > = std::collections::HashMap::new();
    for state_idx in 0..hkx.objects().len() {
        if hkx.objects()[state_idx].class_name != "hkbStateMachineStateInfo" {
            continue;
        }
        let kws = get_generator_chain_keywords(hkx.objects(), state_idx, &id_to_name);
        if !kws.is_empty() {
            generator_keywords.insert(state_idx, kws);
        }
    }

    // --- Walk states and decide what to inject ---
    // Build a plan first (immutable scan), then apply (mutating).
    struct PlanEntry {
        state_idx: usize,
        enter: Vec<(String, Option<String>)>,
        exit: Vec<(String, Option<String>)>,
    }
    let mut plan: Vec<PlanEntry> = Vec::new();
    // Rule 2 states are handled after the plan, as they need mutable access
    // during the scan itself.
    let mut rule2_states: Vec<usize> = Vec::new();

    for state_idx in 0..hkx.objects().len() {
        if hkx.objects()[state_idx].class_name != "hkbStateMachineStateInfo" {
            continue;
        }

        let gen_class = state_generator_class(hkx.objects(), state_idx);

        // Direct child state machines own their transition events. Treating
        // those events as wrapper signals duplicates their child EPAs.
        let mut signals: Vec<String> = Vec::new();
        if let Some(inc) = incoming_transitions.get(&state_idx) {
            signals.extend(inc.iter().cloned());
        }
        if gen_class.as_deref() != Some("hkbStateMachine") {
            if let Some(gk) = generator_keywords.get(&state_idx) {
                signals.extend(gk.iter().cloned());
            }
        }

        let mut enter_events: Vec<(String, Option<String>)> = Vec::new();
        let mut exit_events: Vec<(String, Option<String>)> = Vec::new();
        let mut matched = false;

        // Rule 1: pattern-match.
        if let Some(actions) = match_epa_rule(&signals) {
            for (dir, ev_pattern, payload) in actions {
                if let Some(actual) = event_name_ci.get(*ev_pattern) {
                    let entry = (actual.clone(), payload.map(|s| s.to_string()));
                    match dir {
                        EpaDir::Enter => enter_events.push(entry),
                        EpaDir::Exit => exit_events.push(entry),
                    }
                }
            }
            matched = true;
        }

        // Rule 2: event-holding state (ReferencePoseGenerator with both EPAs).
        // Collect for a separate mutable pass after the plan is applied.
        if !matched && gen_class.as_deref() == Some("hkbReferencePoseGenerator") {
            if has_both_epas(hkx.objects(), state_idx) {
                rule2_states.push(state_idx);
                continue;
            }
        }

        // A null source EPA can belong to a death wrapper; inventing GetUpEnd
        // there queues Havok removal just as the corpse enters Fully Ragdoll.
        if !matched
            && gen_class.as_deref() == Some("hkbStateMachine")
            && signals.is_empty()
            && matches!(
                state_epa_ptr(hkx.objects(), state_idx, "exitNotifyEvents"),
                Some(EpaPtr::Linked(_))
            )
        {
            let key = GETUP_EXIT_EVENT.to_ascii_lowercase();
            if let Some(actual) = event_name_ci.get(key.as_str()) {
                exit_events.push((actual.clone(), None));
            }
        }

        if enter_events.is_empty() && exit_events.is_empty() {
            continue;
        }

        plan.push(PlanEntry {
            state_idx,
            enter: enter_events,
            exit: exit_events,
        });
    }

    if plan.is_empty() && rule2_states.is_empty() {
        return;
    }

    // --- Build payload-string cache (existing hkbStringEventPayload objects) ---
    let mut payload_cache: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    for (i, obj) in hkx.objects().iter().enumerate() {
        if obj.class_name != "hkbStringEventPayload" {
            continue;
        }
        for m in &obj.members {
            if m.name == "data" {
                if let HkxValue::String { value, .. } = &m.value {
                    payload_cache.insert(value.clone(), i);
                }
            }
        }
    }

    // --- Apply the plan: create EPAs (and payloads), wire pointers ---
    for entry in plan {
        for (field_name, events_to_add) in [
            ("enterNotifyEvents", &entry.enter),
            ("exitNotifyEvents", &entry.exit),
        ] {
            if events_to_add.is_empty() {
                continue;
            }

            // Existing EPA pointer? If it points at a real object, inject into
            // it. If it's null/empty, create a fresh EPA.
            let existing_ptr = state_epa_ptr(hkx.objects(), entry.state_idx, field_name);

            match existing_ptr {
                Some(EpaPtr::Linked(epa_idx)) => {
                    // Populate empty events array.
                    populate_existing_epa(
                        hkx,
                        epa_idx,
                        events_to_add,
                        &event_name_to_id,
                        &mut payload_cache,
                    );
                }
                _ => {
                    // Build event sub-objects, possibly creating payloads.
                    let event_inline_structs = build_event_inline_structs(
                        hkx,
                        events_to_add,
                        &event_name_to_id,
                        &mut payload_cache,
                    );
                    if event_inline_structs.is_empty() {
                        continue;
                    }
                    let epa_idx = create_event_property_array(hkx, event_inline_structs);
                    set_state_epa_ptr(hkx, entry.state_idx, field_name, epa_idx);
                }
            }
        }
    }

    // --- Rule 2: populate event-holding states ---
    for state_idx in rule2_states {
        populate_event_holding_state(hkx, state_idx, &event_name_to_id, &mut payload_cache);
    }
}

fn build_event_name_table(
    hkx: &HkxFile,
) -> (
    std::collections::HashMap<String, i32>,
    std::collections::HashMap<String, String>,
    std::collections::HashMap<i32, String>,
) {
    let mut name_to_id = std::collections::HashMap::new();
    let mut name_ci = std::collections::HashMap::new();
    let mut id_to_name = std::collections::HashMap::new();

    for obj in hkx.objects() {
        if !obj.class_name.contains("BehaviorGraphStringData") {
            continue;
        }
        for m in &obj.members {
            if m.name != "eventNames" {
                continue;
            }
            if let HkxValue::Array(items) = &m.value {
                for (i, item) in items.iter().enumerate() {
                    if let HkxValue::String { value, .. } = item {
                        if !value.is_empty() {
                            let id = i as i32;
                            name_to_id.insert(value.clone(), id);
                            name_ci.insert(value.to_ascii_lowercase(), value.clone());
                            id_to_name.insert(id, value.clone());
                        }
                    }
                }
            }
        }
        break;
    }

    (name_to_id, name_ci, id_to_name)
}

/// Resolve the generator's class name for a state, walking through the
/// pointer indirection.
fn state_generator_class(objects: &[HkxObject], state_idx: usize) -> Option<String> {
    let state = &objects[state_idx];
    for m in &state.members {
        if m.name == "generator" {
            if let HkxValue::Pointer(Some(idx)) = m.value {
                if idx < objects.len() {
                    return Some(objects[idx].class_name.clone());
                }
            }
            return None;
        }
    }
    None
}

fn has_both_epas(objects: &[HkxObject], state_idx: usize) -> bool {
    let mut has_enter = false;
    let mut has_exit = false;
    for m in &objects[state_idx].members {
        if let HkxValue::Pointer(Some(_)) = m.value {
            if m.name == "enterNotifyEvents" {
                has_enter = true;
            } else if m.name == "exitNotifyEvents" {
                has_exit = true;
            }
        }
    }
    has_enter && has_exit
}

#[derive(Debug)]
enum EpaPtr {
    Linked(usize),
    Empty,
    Missing,
}

fn state_epa_ptr(objects: &[HkxObject], state_idx: usize, field: &str) -> Option<EpaPtr> {
    for m in &objects[state_idx].members {
        if m.name == field {
            return Some(match &m.value {
                HkxValue::Pointer(Some(i)) => EpaPtr::Linked(*i),
                HkxValue::Pointer(None) => EpaPtr::Empty,
                _ => EpaPtr::Empty,
            });
        }
    }
    Some(EpaPtr::Missing)
}

fn set_state_epa_ptr(hkx: &mut HkxFile, state_idx: usize, field: &str, target_idx: usize) {
    let state = &mut hkx.objects_mut()[state_idx];
    for m in state.members.iter_mut() {
        if m.name == field {
            m.value = HkxValue::Pointer(Some(target_idx));
            return;
        }
    }
    state.members.push(HkxMember {
        name: field.to_string(),
        value: HkxValue::Pointer(Some(target_idx)),
    });
}

fn collect_sm_transitions(
    objects: &[HkxObject],
    sm_idx: usize,
    name_to_idx: &std::collections::HashMap<String, usize>,
    id_to_name: &std::collections::HashMap<i32, String>,
    incoming: &mut std::collections::HashMap<usize, std::collections::HashSet<String>>,
) {
    let sm_obj = &objects[sm_idx];

    // Build state-id → state-obj-idx map for THIS state machine.
    let mut stateid_to_idx: std::collections::HashMap<i32, usize> =
        std::collections::HashMap::new();
    for m in &sm_obj.members {
        if m.name != "states" {
            continue;
        }
        if let HkxValue::Array(items) = &m.value {
            for item in items {
                if let HkxValue::Pointer(Some(state_idx)) = item {
                    if *state_idx < objects.len() {
                        for sm in &objects[*state_idx].members {
                            if sm.name == "stateId" {
                                if let Some(v) = direct_member_as_i32(&sm.value) {
                                    stateid_to_idx.insert(v, *state_idx);
                                }
                            }
                        }
                    }
                }
            }
        }
        break;
    }

    let process_trans_array = |trans_idx: usize,
                               incoming: &mut std::collections::HashMap<
        usize,
        std::collections::HashSet<String>,
    >| {
        if trans_idx >= objects.len() {
            return;
        }
        for tm in &objects[trans_idx].members {
            if tm.name != "transitions" {
                continue;
            }
            let HkxValue::Array(items) = &tm.value else {
                continue;
            };
            for item in items {
                let Some(members) = item.as_object_members() else {
                    continue;
                };
                let mut to_state_id: Option<i32> = None;
                let mut event_id: Option<i32> = None;
                for sub in members {
                    if sub.name == "toStateId" {
                        to_state_id = direct_member_as_i32(&sub.value);
                    } else if sub.name == "eventId" {
                        event_id = direct_member_as_i32(&sub.value);
                    }
                }
                if let (Some(tsi), Some(eid)) = (to_state_id, event_id) {
                    if let (Some(target_idx), Some(ev_name)) =
                        (stateid_to_idx.get(&tsi), id_to_name.get(&eid))
                    {
                        incoming
                            .entry(*target_idx)
                            .or_default()
                            .insert(ev_name.clone());
                    }
                }
            }
        }
    };

    // Wildcard transitions
    for m in &sm_obj.members {
        if m.name == "wildcardTransitions" {
            if let HkxValue::Pointer(Some(idx)) = m.value {
                process_trans_array(idx, incoming);
            }
        }
    }

    // Per-state transitions
    for m in &sm_obj.members {
        if m.name != "states" {
            continue;
        }
        let HkxValue::Array(items) = &m.value else {
            continue;
        };
        for item in items {
            if let HkxValue::Pointer(Some(state_idx)) = item {
                if *state_idx < objects.len() {
                    for sm in &objects[*state_idx].members {
                        if sm.name == "transitions" {
                            if let HkxValue::Pointer(Some(trans_idx)) = sm.value {
                                process_trans_array(trans_idx, incoming);
                            }
                        }
                    }
                }
            }
        }
        break;
    }

    let _ = name_to_idx; // not currently used; kept for parity with Python signature
}

fn direct_member_as_i32(value: &HkxValue) -> Option<i32> {
    match value {
        HkxValue::I8(v) => Some(*v as i32),
        HkxValue::U8(v) => Some(*v as i32),
        HkxValue::I16(v) => Some(*v as i32),
        HkxValue::U16(v) => Some(*v as i32),
        HkxValue::I32(v) => Some(*v),
        HkxValue::U32(v) => Some(*v as i32),
        HkxValue::I64(v) => Some(*v as i32),
        HkxValue::U64(v) => Some(*v as i32),
        _ => None,
    }
}

fn get_generator_chain_keywords(
    objects: &[HkxObject],
    state_idx: usize,
    id_to_name: &std::collections::HashMap<i32, String>,
) -> std::collections::HashSet<String> {
    let mut keywords = std::collections::HashSet::new();
    let mut visited = std::collections::HashSet::new();

    fn walk(
        objects: &[HkxObject],
        idx: usize,
        depth: usize,
        visited: &mut std::collections::HashSet<usize>,
        id_to_name: &std::collections::HashMap<i32, String>,
        keywords: &mut std::collections::HashSet<String>,
    ) {
        if depth > 5 || idx >= objects.len() || !visited.insert(idx) {
            return;
        }
        let obj = &objects[idx];

        if obj.class_name == "hkbStateMachine" {
            collect_sm_event_names(objects, idx, id_to_name, keywords);
            return;
        }

        for m in &obj.members {
            if m.name == "generator" {
                if let HkxValue::Pointer(Some(child)) = m.value {
                    walk(objects, child, depth + 1, visited, id_to_name, keywords);
                }
            } else if m.name == "layers" {
                if let HkxValue::Array(items) = &m.value {
                    for item in items {
                        if let HkxValue::Pointer(Some(layer_idx)) = item {
                            if *layer_idx < objects.len() {
                                for lm in &objects[*layer_idx].members {
                                    if lm.name == "generator" {
                                        if let HkxValue::Pointer(Some(child)) = lm.value {
                                            walk(
                                                objects,
                                                child,
                                                depth + 1,
                                                visited,
                                                id_to_name,
                                                keywords,
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Start from the state's generator.
    for m in &objects[state_idx].members {
        if m.name == "generator" {
            if let HkxValue::Pointer(Some(child)) = m.value {
                walk(objects, child, 0, &mut visited, id_to_name, &mut keywords);
            }
            break;
        }
    }

    keywords
}

fn collect_sm_event_names(
    objects: &[HkxObject],
    sm_idx: usize,
    id_to_name: &std::collections::HashMap<i32, String>,
    keywords: &mut std::collections::HashSet<String>,
) {
    let sm_obj = &objects[sm_idx];

    let grab = |trans_idx: usize, keywords: &mut std::collections::HashSet<String>| {
        if trans_idx >= objects.len() {
            return;
        }
        for tm in &objects[trans_idx].members {
            if tm.name != "transitions" {
                continue;
            }
            let HkxValue::Array(items) = &tm.value else {
                continue;
            };
            for item in items {
                let Some(members) = item.as_object_members() else {
                    continue;
                };
                for sub in members {
                    if sub.name == "eventId" {
                        if let Some(v) = direct_member_as_i32(&sub.value) {
                            if let Some(nm) = id_to_name.get(&v) {
                                keywords.insert(nm.clone());
                            }
                        }
                    }
                }
            }
        }
    };

    for m in &sm_obj.members {
        if m.name == "wildcardTransitions" {
            if let HkxValue::Pointer(Some(idx)) = m.value {
                grab(idx, keywords);
            }
        }
    }

    for m in &sm_obj.members {
        if m.name != "states" {
            continue;
        }
        let HkxValue::Array(items) = &m.value else {
            continue;
        };
        for item in items {
            if let HkxValue::Pointer(Some(state_idx)) = item {
                if *state_idx < objects.len() {
                    for sm in &objects[*state_idx].members {
                        if sm.name == "transitions" {
                            if let HkxValue::Pointer(Some(trans_idx)) = sm.value {
                                grab(trans_idx, keywords);
                            }
                        }
                    }
                }
            }
        }
        break;
    }
}

/// Find or create a hkbStringEventPayload for the given string. Returns
/// the object index in `hkx.objects()`.
fn find_or_create_string_payload(
    hkx: &mut HkxFile,
    payload_cache: &mut std::collections::HashMap<String, usize>,
    payload_string: &str,
) -> usize {
    if let Some(&idx) = payload_cache.get(payload_string) {
        return idx;
    }

    let max_id = max_numbered_object_name(hkx);
    let new_name = format!("#{:04}", max_id + 1);
    let idx = hkx.push_object(HkxObject {
        name: Some(new_name),
        offset: 0,
        signature: 0,
        class_name: "hkbStringEventPayload".to_string(),
        members: vec![
            HkxMember {
                name: "memSizeAndFlags".to_string(),
                value: HkxValue::U16(0),
            },
            HkxMember {
                name: "refCount".to_string(),
                value: HkxValue::I16(0),
            },
            HkxMember {
                name: "data".to_string(),
                value: HkxValue::String {
                    value: payload_string.to_string(),
                    is_null: false,
                },
            },
        ],
    });
    payload_cache.insert(payload_string.to_string(), idx);
    idx
}

/// Build the inline-struct list (hkbEventProperty) for an EPA's events array.
fn build_event_inline_structs(
    hkx: &mut HkxFile,
    events: &[(String, Option<String>)],
    event_name_to_id: &std::collections::HashMap<String, i32>,
    payload_cache: &mut std::collections::HashMap<String, usize>,
) -> Vec<HkxValue> {
    let mut out: Vec<HkxValue> = Vec::with_capacity(events.len());
    for (ev_name, payload_str) in events {
        let Some(&ev_id) = event_name_to_id.get(ev_name) else {
            continue;
        };
        let payload_value = match payload_str {
            Some(s) if !s.is_empty() => {
                let idx = find_or_create_string_payload(hkx, payload_cache, s);
                HkxValue::Pointer(Some(idx))
            }
            _ => HkxValue::Pointer(None),
        };
        out.push(HkxValue::TypedObject {
            class_name: "hkbEventProperty".to_string(),
            members: vec![
                HkxMember {
                    name: "id".to_string(),
                    value: HkxValue::I32(ev_id),
                },
                HkxMember {
                    name: "payload".to_string(),
                    value: payload_value,
                },
            ],
        });
    }
    out
}

/// Append a fresh hkbStateMachineEventPropertyArray and return its index.
fn create_event_property_array(hkx: &mut HkxFile, events: Vec<HkxValue>) -> usize {
    let max_id = max_numbered_object_name(hkx);
    let new_name = format!("#{:04}", max_id + 1);
    hkx.push_object(HkxObject {
        name: Some(new_name),
        offset: 0,
        signature: 0,
        class_name: "hkbStateMachineEventPropertyArray".to_string(),
        members: vec![
            HkxMember {
                name: "memSizeAndFlags".to_string(),
                value: HkxValue::U16(0),
            },
            HkxMember {
                name: "refCount".to_string(),
                value: HkxValue::I16(0),
            },
            HkxMember {
                name: "events".to_string(),
                value: HkxValue::Array(events),
            },
        ],
    })
}

/// If an EPA has an empty `events` array, fill it in place.
fn populate_existing_epa(
    hkx: &mut HkxFile,
    epa_idx: usize,
    events_to_add: &[(String, Option<String>)],
    event_name_to_id: &std::collections::HashMap<String, i32>,
    payload_cache: &mut std::collections::HashMap<String, usize>,
) {
    // Check `events` is currently empty.
    let is_empty = hkx.objects()[epa_idx]
        .members
        .iter()
        .find(|m| m.name == "events")
        .map(|m| matches!(&m.value, HkxValue::Array(v) if v.is_empty()))
        .unwrap_or(false);
    if !is_empty {
        return;
    }

    let new_events =
        build_event_inline_structs(hkx, events_to_add, event_name_to_id, payload_cache);
    if new_events.is_empty() {
        return;
    }

    // Re-borrow after build_event_inline_structs may have appended payload objects.
    if let Some(events_member) = hkx.objects_mut()[epa_idx]
        .members
        .iter_mut()
        .find(|m| m.name == "events")
    {
        if let HkxValue::Array(arr) = &mut events_member.value {
            arr.extend(new_events);
        }
    }
}

// ---------------------------------------------------------------------------
// Rule 2 — event-holding state (hkbReferencePoseGenerator with both EPAs)
// ---------------------------------------------------------------------------

/// Events that were already assigned to other EPA rules — exclude from clip-annotation scan.
const OTHER_EPA_EVENTS: &[&str] = &[
    "staggerstop",
    "attackstop",
    "recoilstop",
    "pairedstop",
    "startanimationdriven",
    "headtrackingoff",
    "headtrackingon",
    "addragdolltoworld",
    "removecharactercontrollerfromworld",
    "enterfullyragdoll",
    "getupend",
    "pairend",
    "weapondraw",
    "weaponsheathe",
    "enablebumper",
    "disablebumper",
    "attackstate2",
];

/// Internal graph-control events to exclude.
const GRAPH_CONTROL_EVENTS: &[&str] = &[
    "reevaluategraphstate",
    "syncdeferdeath",
    "attackinterrupt",
    "killmovestart",
    "killmoveend",
    "startallowrotation",
    "pathtweenerend",
];

/// Prefix patterns to exclude (body-part hit reactions, weapon sweeps).
const EXCLUDE_PREFIXES: &[&str] = &["hitreact", "weaponsweep"];

/// Prefix patterns for classifying events as ENTER (passive/ambient).
/// Events matching these go on enterNotifyEvents; all others go on exitNotifyEvents.
const ENTER_PREFIXES: &[&str] = &["foot", "flinch", "camera", "death", "getup"];

/// Collect the set of event IDs that are referenced by transitions or any
/// direct member whose name contains "event" (excluding the member named
/// "events").
fn collect_used_event_ids(objects: &[HkxObject]) -> std::collections::HashSet<i32> {
    let mut used: std::collections::HashSet<i32> = std::collections::HashSet::new();
    for obj in objects {
        // Transitions: objects whose class name contains "TransitionInfoArray"
        if obj.class_name.contains("TransitionInfoArray") {
            for m in &obj.members {
                if m.name != "transitions" {
                    continue;
                }
                if let HkxValue::Array(items) = &m.value {
                    for item in items {
                        if let Some(members) = item.as_object_members() {
                            for sub in members {
                                if sub.name == "eventId" {
                                    if let Some(v) = direct_member_as_i32(&sub.value) {
                                        if v >= 0 {
                                            used.insert(v);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        // Any direct member with "event" in name (but not literally "events")
        for m in &obj.members {
            if m.name == "events" {
                continue;
            }
            let name_lower = m.name.to_ascii_lowercase();
            if !name_lower.contains("event") {
                continue;
            }
            if let Some(v) = direct_member_as_i32(&m.value) {
                if v >= 0 {
                    used.insert(v);
                }
            }
        }
    }
    used
}

/// Port of `_populate_event_holding_state`: for a ReferencePoseGenerator state
/// that already has both enter+exit EPA pointers, scan clip-annotation events
/// (those not used in transitions) and partition them into enter/exit by prefix,
/// then append them to the existing (empty) EPA events arrays.
fn populate_event_holding_state(
    hkx: &mut HkxFile,
    state_idx: usize,
    event_name_to_id: &std::collections::HashMap<String, i32>,
    payload_cache: &mut std::collections::HashMap<String, usize>,
) {
    let used_event_ids = collect_used_event_ids(hkx.objects());

    // Collect enter/exit event names, sorted by event ID.
    let mut sorted_events: Vec<(&String, i32)> = event_name_to_id
        .iter()
        .map(|(name, &id)| (name, id))
        .collect();
    sorted_events.sort_by_key(|&(_, id)| id);

    let mut enter_events: Vec<(String, Option<String>)> = Vec::new();
    let mut exit_events: Vec<(String, Option<String>)> = Vec::new();

    for (event_name, event_id) in &sorted_events {
        if used_event_ids.contains(event_id) {
            continue;
        }
        let name_lower = event_name.to_ascii_lowercase();
        if OTHER_EPA_EVENTS.contains(&name_lower.as_str()) {
            continue;
        }
        if GRAPH_CONTROL_EVENTS.contains(&name_lower.as_str()) {
            continue;
        }
        if EXCLUDE_PREFIXES.iter().any(|p| name_lower.starts_with(p)) {
            continue;
        }
        if ENTER_PREFIXES.iter().any(|p| name_lower.starts_with(p)) {
            enter_events.push(((*event_name).clone(), None));
        } else {
            exit_events.push(((*event_name).clone(), None));
        }
    }

    for (field_name, events_to_add) in [
        ("enterNotifyEvents", &enter_events),
        ("exitNotifyEvents", &exit_events),
    ] {
        if events_to_add.is_empty() {
            continue;
        }
        let epa_idx = match state_epa_ptr(hkx.objects(), state_idx, field_name) {
            Some(EpaPtr::Linked(idx)) => idx,
            _ => continue,
        };
        populate_existing_epa(hkx, epa_idx, events_to_add, event_name_to_id, payload_cache);
    }
}

const BEHAVIOR_METADATA_CLASSES: &[&str] = &[
    "hkbBehaviorGraphData",
    "hkbVariableValueSet",
    "hkbBehaviorGraphStringData",
];

fn reorder_behavior_metadata_to_end(hkx: &mut HkxFile) {
    let has_metadata = hkx
        .objects()
        .iter()
        .any(|o| BEHAVIOR_METADATA_CLASSES.contains(&o.class_name.as_str()));
    if !has_metadata {
        return;
    }

    let n = hkx.objects().len();
    // Partition: rest indices first (in original order), then metadata in canonical order.
    let mut rest: Vec<usize> = Vec::with_capacity(n);
    let mut metadata: [Vec<usize>; 3] = [vec![], vec![], vec![]];

    for i in 0..n {
        let class_name = hkx.objects()[i].class_name.as_str();
        if let Some(slot) = BEHAVIOR_METADATA_CLASSES
            .iter()
            .position(|&c| c == class_name)
        {
            metadata[slot].push(i);
        } else {
            rest.push(i);
        }
    }

    let mut new_order = rest;
    for slot in &metadata {
        new_order.extend_from_slice(slot);
    }

    hkx.reorder_objects_remap_pointers(new_order);
}

fn fix_behavior_variable_infos(hkx: &mut HkxFile) {
    // Enums stay HkxValue::I32 and the FO4 packfile writer emits the raw i32,
    // so variableInfos needs no conversion.
    let _ = hkx;
}

fn fix_sphere_dispatch_type(hkx: &mut HkxFile) {
    for object in hkx.objects_mut() {
        if object.class_name != "hknpSphereShape" {
            continue;
        }
        for member in &mut object.members {
            if member.name != "dispatchType" {
                continue;
            }
            match &mut member.value {
                HkxValue::I8(v) if *v != 1 => *v = 1,
                HkxValue::U8(v) if *v != 1 => *v = 1,
                HkxValue::I16(v) if *v != 1 => *v = 1,
                HkxValue::U16(v) if *v != 1 => *v = 1,
                HkxValue::I32(v) if *v != 1 => *v = 1,
                HkxValue::U32(v) if *v != 1 => *v = 1,
                HkxValue::I64(v) if *v != 1 => *v = 1,
                HkxValue::U64(v) if *v != 1 => *v = 1,
                _ => {}
            }
        }
    }
}

/// Force every `hknpConvexPolytopeShape` to dispatch as CONVEX (`dispatchType=1`).
///
/// FO76 tags convex polytopes with `dispatchType=2` (COMPOSITE); vanilla FO4
/// uses `1` (CONVEX). Left at `2`, the FO4 physics engine dispatches the convex
/// polytope as a composite shape, walks phantom child shape-keys via the shape
/// codec, and resolves a null `hknpBody` on the first real contact — crashing the
/// MT physics-settle worker at `hknpBSShapeCodec::decodeImpl` (`Fallout4.exe+18C4195`)
/// on cell load. The CK never runs this dispatch, so it loads such assets fine.
/// Sibling of [`fix_sphere_dispatch_type`]; capsules are normalized in
/// `migrate_skeleton_physics`.
fn fix_polytope_dispatch_type(hkx: &mut HkxFile) {
    for object in hkx.objects_mut() {
        if object.class_name != "hknpConvexPolytopeShape" {
            continue;
        }
        for member in &mut object.members {
            if member.name == "dispatchType" {
                set_int_member(&mut member.value, 1);
            }
        }
    }
}

fn fix_compressed_mesh_shape_headers(hkx: &mut HkxFile) {
    for object in hkx.objects_mut() {
        if object.class_name != "hknpCompressedMeshShape" {
            continue;
        }

        // FO76's COMPOSITE enum value is FO4's DISTANCE_FIELD value.
        set_member_value(&mut object.members, "flags", HkxValue::U16(0x0204));
        set_member_value(&mut object.members, "dispatchType", HkxValue::U8(2));
    }
}

// ---------------------------------------------------------------------------
// FO76 → FO4 physics transforms
// ---------------------------------------------------------------------------

/// Renames FO76 physics classes to their FO4 equivalents
/// (hkpBallAndSocketConstraintData → hkpRagdollConstraintData,
/// hknpCompressedMeshShapeTree → hknpCompressedMeshShapeData,
/// hknpConvexShape → hknpSphereShape) and normalizes hknpCapsuleShape
/// fields (a/b w-component, dispatchType, flags, planes pad, faces.firstIndex).
fn migrate_skeleton_physics(hkx: &mut HkxFile) {
    let has_physics = hkx.objects().iter().any(|o| {
        matches!(
            o.class_name.as_str(),
            "hknpPhysicsSystemData"
                | "hknpRagdollData"
                | "hknpCapsuleShape"
                | "hknpConvexPolytopeShape"
                | "hkpBallAndSocketConstraintData"
                | "hkpRagdollConstraintData"
        )
    });
    if !has_physics {
        return;
    }

    let drop_meta = |members: Vec<HkxMember>| -> Vec<HkxMember> {
        members
            .into_iter()
            .filter(|m| !matches!(m.name.as_str(), "memSizeAndFlags" | "refCount"))
            .collect()
    };

    // A ball-and-socket's single `pivots` atom has no name-match in the FO4
    // ragdoll atom layout, so renaming the class alone drops the pivot and
    // serializes an all-zero, TYPE_INVALID constraint frame. Model the new
    // atoms on a sibling ragdoll constraint from the same file instead.
    let ragdoll_atoms_template = hkx
        .objects()
        .iter()
        .find(|o| o.class_name == "hkpRagdollConstraintData")
        .and_then(|o| o.members.iter().find(|m| m.name == "atoms"))
        .map(|m| m.value.clone());

    for object in hkx.objects_mut() {
        match object.class_name.as_str() {
            "hkpBallAndSocketConstraintData" => {
                let members = std::mem::take(&mut object.members);
                let mut members = drop_meta(members);
                if let Some(atoms) = ragdoll_atoms_template
                    .as_ref()
                    .and_then(|template| ragdoll_atoms_from_ball_socket(template, &members))
                {
                    object.class_name = "hkpRagdollConstraintData".to_string();
                    object.signature = 0; // hkpRagdollConstraintData_0.xml
                    set_member_value(&mut members, "atoms", atoms);
                }
                // Without a sibling to model the atom layout on, keep the class:
                // FO4 registers hkpBallAndSocketConstraintData (vanilla
                // meshes\traps\grenadebouquet*.nif use it), which beats emitting
                // a half-built ragdoll constraint.
                object.members = members;
            }
            "hknpCompressedMeshShapeTree" => {
                object.class_name = "hknpCompressedMeshShapeData".to_string();
                object.signature = 0; // hknpCompressedMeshShapeData_0.xml
                let members = std::mem::take(&mut object.members);
                object.members = drop_meta(members);
            }
            "hknpConvexShape" if is_compact_sphere_shape(object) => {
                object.class_name = "hknpSphereShape".to_string();
                object.signature = 0; // hknpSphereShape_0.xml
                // Set canonical sphere flags / dispatch.
                for m in object.members.iter_mut() {
                    match m.name.as_str() {
                        "flags" => set_int_member(&mut m.value, 273),
                        "dispatchType" => set_int_member(&mut m.value, 1),
                        _ => {}
                    }
                }
                // Strip serialization metadata + obsolete `type` member.
                let members = std::mem::take(&mut object.members);
                object.members = members
                    .into_iter()
                    .filter(|m| !matches!(m.name.as_str(), "memSizeAndFlags" | "refCount" | "type"))
                    .collect();
            }
            _ => {}
        }
    }

    // Capsule normalization. Detect standalone vs embedded for w-component.
    let is_standalone = hkx
        .objects()
        .iter()
        .any(|o| o.class_name == "hkRootLevelContainer");
    let target_w: f32 = if is_standalone { 0.0 } else { 1.0 };
    const FLT_MIN: f32 = -3.402_823_5e38;
    const SENTINEL: [f32; 4] = [0.0, 0.0, 0.0, FLT_MIN];
    const CANON_FLAGS: i32 = 451;

    for object in hkx.objects_mut() {
        if object.class_name != "hknpCapsuleShape" {
            continue;
        }
        synthesize_capsule_hull_if_empty(object);
        for m in object.members.iter_mut() {
            match m.name.as_str() {
                "a" | "b" => {
                    if let HkxValue::F32List(values) = &mut m.value {
                        if values.len() >= 4 && (values[3] - target_w).abs() > f32::EPSILON {
                            values[3] = target_w;
                        }
                    }
                }
                "dispatchType" => set_int_member(&mut m.value, 1),
                "flags" => set_int_member(&mut m.value, CANON_FLAGS),
                "planes" => {
                    if let HkxValue::Array(values) = &mut m.value {
                        while values.len() < 8 {
                            values.push(HkxValue::F32List(SENTINEL.to_vec()));
                        }
                    }
                }
                "faces" => {
                    if let HkxValue::Array(values) = &mut m.value {
                        let mut cum: i32 = 0;
                        for face in values.iter_mut() {
                            let HkxValue::Object(face_members) = face else {
                                continue;
                            };
                            let mut ni: i32 = 0;
                            for sm in face_members.iter() {
                                if sm.name == "numIndices" {
                                    ni = extract_int(&sm.value).unwrap_or(0) & 0xFF;
                                }
                            }
                            for sm in face_members.iter_mut() {
                                if sm.name == "firstIndex" {
                                    set_int_member(&mut sm.value, cum);
                                }
                            }
                            cum += ni;
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

/// Rebuild a FO76 `hkpBallAndSocketConstraintDataAtoms` as FO4
/// `hkpRagdollConstraintDataAtoms`.
///
/// `template` is the `atoms` value of a sibling `hkpRagdollConstraintData` from
/// the same file, so every atom the ball-and-socket has no counterpart for keeps
/// a decoded value of the right variant (and a wired motor pointer — FO4
/// dereferences those during ragdoll activation). The pivot becomes the
/// `transforms` translation column, and every angular limit plus friction is
/// disabled, which is what a ball-and-socket joint means.
fn ragdoll_atoms_from_ball_socket(
    template: &HkxValue,
    ball_socket_members: &[HkxMember],
) -> Option<HkxValue> {
    const TYPE_SET_LOCAL_TRANSFORMS: i32 = 2;

    let source_atoms = ball_socket_members
        .iter()
        .find(|m| m.name == "atoms")?
        .value
        .as_object_members()?
        .to_vec();
    let pivots = source_atoms
        .iter()
        .find(|m| m.name == "pivots")?
        .value
        .as_object_members()?;
    let pivot = |name: &str| -> [f32; 3] {
        pivots
            .iter()
            .find(|m| m.name == name)
            .and_then(|m| match &m.value {
                HkxValue::F32List(values) if values.len() >= 3 => {
                    Some([values[0], values[1], values[2]])
                }
                _ => None,
            })
            .unwrap_or([0.0; 3])
    };
    let (translation_a, translation_b) = (pivot("translationA"), pivot("translationB"));

    let mut atoms = template.clone();
    for member in atoms.as_object_members_mut()?.iter_mut() {
        match member.name.as_str() {
            "transforms" => {
                let Some(transforms) = member.value.as_object_members_mut() else {
                    continue;
                };
                for m in transforms.iter_mut() {
                    match m.name.as_str() {
                        "type" => set_int_member(&mut m.value, TYPE_SET_LOCAL_TRANSFORMS),
                        "transformA" => m.value = local_transform(translation_a),
                        "transformB" => m.value = local_transform(translation_b),
                        _ => {}
                    }
                }
            }
            "angFriction" | "twistLimit" | "coneLimit" | "planesLimit" => {
                let Some(atom) = member.value.as_object_members_mut() else {
                    continue;
                };
                for m in atom.iter_mut() {
                    if m.name == "isEnabled" {
                        set_int_member(&mut m.value, 0);
                    }
                }
            }
            "setupStabilization" | "ballSocket" => {
                if let Some(source) = source_atoms.iter().find(|m| m.name == member.name) {
                    member.value = source.value.clone();
                }
            }
            _ => {}
        }
    }
    Some(atoms)
}

/// `hkTransform` as an identity basis with `translation` in the fourth column.
fn local_transform(translation: [f32; 3]) -> HkxValue {
    HkxValue::F32List(vec![
        1.0,
        0.0,
        0.0,
        0.0,
        0.0,
        1.0,
        0.0,
        0.0,
        0.0,
        0.0,
        1.0,
        0.0,
        translation[0],
        translation[1],
        translation[2],
        0.0,
    ])
}

fn is_compact_sphere_shape(object: &HkxObject) -> bool {
    let radius = object.members.iter().find_map(|member| match member {
        HkxMember {
            name,
            value: HkxValue::F32(value),
        } if name == "convexRadius" => Some(*value),
        _ => None,
    });
    let vertices = object.members.iter().find_map(|member| {
        (member.name == "vertices").then(|| match &member.value {
            HkxValue::Array(vertices) => Some(vertices),
            _ => None,
        })?
    });
    let one_support_point = vertices.is_some_and(|vertices| {
        let Some(HkxValue::F32List(first)) = vertices.first() else {
            return false;
        };
        !vertices.is_empty()
            && vertices.len() <= 4
            && first.len() >= 3
            && vertices.iter().all(|vertex| {
                matches!(vertex, HkxValue::F32List(value) if value.len() >= 3 && value[..3] == first[..3])
            })
    });
    radius.is_some_and(|radius| radius.is_finite() && radius > 0.0) && one_support_point
}

fn synthesize_ragdoll_shape_geometry(hkx: &mut HkxFile) {
    let object_count = hkx.objects().len();
    let mut sphere_updates: Vec<(usize, [f32; 4])> = Vec::new();

    for object in hkx.objects() {
        if object.class_name != "hknpRagdollData" {
            continue;
        }
        let Some(body_member) = object.members.iter().find(|m| m.name == "bodyCinfos") else {
            continue;
        };
        let HkxValue::Array(bodies) = &body_member.value else {
            continue;
        };
        for body in bodies {
            let Some(body_members) = body.as_object_members() else {
                continue;
            };
            let Some(shape_idx) = pointer_member_value(body_members, "shape") else {
                continue;
            };
            if shape_idx >= object_count
                || hkx.objects()[shape_idx].class_name != "hknpSphereShape"
                || !array_member_is_empty(&hkx.objects()[shape_idx].members, "vertices")
            {
                continue;
            }
            let Some(mass_idx) = pointer_member_value(body_members, "massDistribution") else {
                continue;
            };
            let Some(mass_object) = hkx.objects().get(mass_idx) else {
                continue;
            };
            let Some(center) = center_of_mass_member(&mass_object.members) else {
                continue;
            };
            sphere_updates.push((shape_idx, [center[0], center[1], center[2], 0.5]));
        }
    }

    for (shape_idx, vertex) in sphere_updates {
        set_member_value(
            &mut hkx.objects_mut()[shape_idx].members,
            "vertices",
            HkxValue::Array(vec![HkxValue::F32List(vertex.to_vec())]),
        );
    }
}

/// FO4 sphere shapes carry a four-lane support-point array even when all four
/// points describe the same sphere center. FO76 commonly stores only one lane.
/// The FO4 narrow-phase reads the native-width quartet once a non-root ragdoll
/// sphere begins colliding, so pad compact sphere arrays with their authored
/// support point instead of leaving adjacent packfile data as implicit lanes.
fn normalize_sphere_support_vertices(hkx: &mut HkxFile) {
    const FO4_SPHERE_SUPPORT_LANES: usize = 4;

    for object in hkx.objects_mut() {
        if object.class_name != "hknpSphereShape" {
            continue;
        }
        let Some(vertices_member) = object
            .members
            .iter_mut()
            .find(|member| member.name == "vertices")
        else {
            continue;
        };
        let HkxValue::Array(vertices) = &mut vertices_member.value else {
            continue;
        };
        if vertices.is_empty() || vertices.len() >= FO4_SPHERE_SUPPORT_LANES {
            continue;
        }
        let first = vertices[0].clone();
        let HkxValue::F32List(first_values) = &first else {
            continue;
        };
        if first_values.len() < 4 || !first_values.iter().all(|value| value.is_finite()) {
            continue;
        }
        let all_same_support = vertices
            .iter()
            .all(|vertex| matches!(vertex, HkxValue::F32List(values) if values == first_values));
        if !all_same_support {
            continue;
        }
        vertices.resize(FO4_SPHERE_SUPPORT_LANES, first);
    }
}

/// FO76 wraps packed COM/inertia words in an inline `values` object. FO4's older
/// descriptor expects the four packed words directly. Flatten the wrapper
/// without decoding/repacking so the exact authored words survive the target
/// writer.
fn normalize_shape_mass_properties(hkx: &mut HkxFile) {
    for object in hkx.objects_mut() {
        if object.class_name != "hknpShapeMassProperties" {
            continue;
        }
        let Some(compressed) = object
            .members
            .iter_mut()
            .find(|member| member.name == "compressedMassProperties")
            .and_then(|member| member.value.as_object_members_mut())
        else {
            continue;
        };

        for field_name in ["centerOfMass", "inertia", "majorAxisSpace"] {
            let Some(field) = compressed
                .iter_mut()
                .find(|member| member.name == field_name)
            else {
                continue;
            };
            let Some(values) = field
                .value
                .as_object_members()
                .and_then(|members| members.iter().find(|member| member.name == "values"))
                .and_then(|member| match &member.value {
                    HkxValue::Array(values)
                        if values.len() == 4
                            && values.iter().all(|value| matches!(value, HkxValue::I16(_))) =>
                    {
                        Some(values.clone())
                    }
                    _ => None,
                })
            else {
                continue;
            };
            field.value = HkxValue::Array(values);
        }
    }
}

fn synthesize_capsule_hull_if_empty(object: &mut HkxObject) {
    if !(array_member_is_empty(&object.members, "vertices")
        && array_member_is_empty(&object.members, "planes")
        && array_member_is_empty(&object.members, "faces")
        && array_member_is_empty(&object.members, "indices"))
    {
        return;
    }

    let Some(a) = f32_list4_member(&object.members, "a") else {
        return;
    };
    let Some(b) = f32_list4_member(&object.members, "b") else {
        return;
    };
    let Some(convex_radius) = f32_member(&object.members, "convexRadius") else {
        return;
    };
    let Some((vertices, planes, faces, indices)) = capsule_hull_from_endpoints(a, b, convex_radius)
    else {
        return;
    };

    set_member_value(&mut object.members, "vertices", HkxValue::Array(vertices));
    set_member_value(&mut object.members, "planes", HkxValue::Array(planes));
    set_member_value(&mut object.members, "faces", HkxValue::Array(faces));
    set_member_value(&mut object.members, "indices", HkxValue::Array(indices));
}

fn capsule_hull_from_endpoints(
    a: [f32; 4],
    b: [f32; 4],
    convex_radius: f32,
) -> Option<(Vec<HkxValue>, Vec<HkxValue>, Vec<HkxValue>, Vec<HkxValue>)> {
    const FLT_MIN: f32 = -3.402_823_5e38;
    const FACE_INDICES: [u8; 24] = [
        2, 6, 4, 0, 1, 5, 7, 3, 1, 0, 4, 5, 7, 6, 2, 3, 3, 2, 0, 1, 7, 5, 4, 6,
    ];

    let half = (a[3].abs() - convex_radius).abs();
    if half <= 1e-7 {
        return None;
    }

    let start = [b[0], b[1], b[2]];
    let end = [a[0], a[1], a[2]];
    let axis = normalize3(sub3(end, start))?;
    let xy_len = (axis[0] * axis[0] + axis[1] * axis[1]).sqrt();
    let side = if xy_len > 1e-7 {
        [axis[1] / xy_len, -axis[0] / xy_len, 0.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let up = normalize3(cross3(side, axis))?;
    let p0 = sub3(start, scale3(axis, half));
    let p1 = add3(end, scale3(axis, half));

    let vertices = [
        add3(add3(p0, scale3(side, half)), scale3(up, half)),
        add3(add3(p1, scale3(side, half)), scale3(up, half)),
        add3(sub3(p0, scale3(side, half)), scale3(up, half)),
        add3(sub3(p1, scale3(side, half)), scale3(up, half)),
        sub3(add3(p0, scale3(side, half)), scale3(up, half)),
        sub3(add3(p1, scale3(side, half)), scale3(up, half)),
        sub3(sub3(p0, scale3(side, half)), scale3(up, half)),
        sub3(sub3(p1, scale3(side, half)), scale3(up, half)),
    ]
    .into_iter()
    .map(|v| HkxValue::F32List(vec![v[0], v[1], v[2], 0.5]))
    .collect();

    let plane_specs = [
        (scale3(axis, -1.0), p0),
        (axis, p1),
        (side, add3(p0, scale3(side, half))),
        (scale3(side, -1.0), sub3(p0, scale3(side, half))),
        (up, add3(p0, scale3(up, half))),
        (scale3(up, -1.0), sub3(p0, scale3(up, half))),
    ];
    let mut planes: Vec<HkxValue> = plane_specs
        .into_iter()
        .map(|(normal, point)| {
            HkxValue::F32List(vec![normal[0], normal[1], normal[2], -dot3(normal, point)])
        })
        .collect();
    planes.push(HkxValue::F32List(vec![0.0, 0.0, 0.0, FLT_MIN]));
    planes.push(HkxValue::F32List(vec![0.0, 0.0, 0.0, FLT_MIN]));

    let faces = (0..6)
        .map(|i| {
            HkxValue::Object(vec![
                HkxMember {
                    name: "firstIndex".to_string(),
                    value: HkxValue::U16((i * 4) as u16),
                },
                HkxMember {
                    name: "numIndices".to_string(),
                    value: HkxValue::U8(4),
                },
                HkxMember {
                    name: "minHalfAngle".to_string(),
                    value: HkxValue::U8(4),
                },
            ])
        })
        .collect();
    let indices = FACE_INDICES.into_iter().map(HkxValue::U8).collect();

    Some((vertices, planes, faces, indices))
}

fn pointer_member_value(members: &[HkxMember], name: &str) -> Option<usize> {
    members.iter().find_map(|m| {
        if m.name == name {
            if let HkxValue::Pointer(Some(idx)) = &m.value {
                return Some(*idx);
            }
        }
        None
    })
}

fn center_of_mass_member(members: &[HkxMember]) -> Option<[f32; 3]> {
    for member in members {
        if matches!(
            member.name.as_str(),
            "centerOfMassAndVolume" | "centerOfMass"
        ) {
            if let HkxValue::F32List(values) = &member.value {
                if values.len() >= 3 {
                    return Some([values[0], values[1], values[2]]);
                }
            }
        }
        if let Some(inner) = member.value.as_object_members() {
            if let Some(center) = center_of_mass_member(inner) {
                return Some(center);
            }
        }
    }
    None
}

fn f32_list4_member(members: &[HkxMember], name: &str) -> Option<[f32; 4]> {
    members.iter().find_map(|m| {
        if m.name == name {
            if let HkxValue::F32List(values) = &m.value {
                if values.len() >= 4 {
                    return Some([values[0], values[1], values[2], values[3]]);
                }
            }
        }
        None
    })
}

fn f32_member(members: &[HkxMember], name: &str) -> Option<f32> {
    members.iter().find_map(|m| {
        if m.name == name {
            match &m.value {
                HkxValue::F32(v) => Some(*v),
                HkxValue::I32(v) => Some(*v as f32),
                HkxValue::U32(v) => Some(*v as f32),
                _ => None,
            }
        } else {
            None
        }
    })
}

fn bool_member(members: &[HkxMember], name: &str) -> Option<bool> {
    members.iter().find_map(|member| {
        if member.name == name {
            match &member.value {
                HkxValue::Bool(value) => Some(*value),
                value => extract_int(value).map(|value| value != 0),
            }
        } else {
            None
        }
    })
}

fn array_member_is_empty(members: &[HkxMember], name: &str) -> bool {
    members
        .iter()
        .find(|m| m.name == name)
        .map(|m| matches!(&m.value, HkxValue::Array(values) if values.is_empty()))
        .unwrap_or(true)
}

fn set_member_value(members: &mut Vec<HkxMember>, name: &str, value: HkxValue) {
    if let Some(member) = members.iter_mut().find(|m| m.name == name) {
        member.value = value;
    } else {
        members.push(HkxMember {
            name: name.to_string(),
            value,
        });
    }
}

fn add3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale3(v: [f32; 3], s: f32) -> [f32; 3] {
    [v[0] * s, v[1] * s, v[2] * s]
}

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize3(v: [f32; 3]) -> Option<[f32; 3]> {
    let len = dot3(v, v).sqrt();
    if len <= 1e-7 {
        None
    } else {
        Some(scale3(v, 1.0 / len))
    }
}

/// FO76 stores PSD under generic `hkReferencedObject`. Detect via `bodyCinfos`
/// member presence and rename. Insert empty `motionCinfos` if absent.
fn reclassify_fo76_physics_system_data(hkx: &mut HkxFile) {
    for object in hkx.objects_mut() {
        if object.class_name != "hkReferencedObject" {
            continue;
        }
        if !object.members.iter().any(|m| m.name == "bodyCinfos") {
            continue;
        }
        object.class_name = "hknpPhysicsSystemData".to_string();
        object.signature = 0; // hknpPhysicsSystemData_0.xml
        let has_motion_cinfos = object.members.iter().any(|m| m.name == "motionCinfos");
        if !has_motion_cinfos {
            if let Some(insert_idx) = object
                .members
                .iter()
                .position(|m| m.name == "motionProperties")
                .map(|i| i + 1)
            {
                object.members.insert(
                    insert_idx,
                    HkxMember {
                        name: "motionCinfos".to_string(),
                        value: HkxValue::Array(vec![]),
                    },
                );
            }
        }
    }
}

/// FO76 hknpRefMassDistribution objects land on `hkUint16` due to TAG0 reader
/// fallback. Detect via mass-field presence and rename so the connectivity
/// strip pass leaves them alone.
fn reclassify_mass_distributions(hkx: &mut HkxFile) {
    for object in hkx.objects_mut() {
        if object.class_name != "hkUint16" {
            continue;
        }
        if has_mass_fields(&object.members) {
            object.class_name = "hknpRefMassDistribution".to_string();
            object.signature = 0; // synthesized; no FO4 classxml, version 0
        }
    }
}

fn has_mass_fields(members: &[HkxMember]) -> bool {
    for m in members {
        if matches!(
            m.name.as_str(),
            "centerOfMassAndVolume" | "centerOfMass" | "inertiaTensor" | "massDistribution"
        ) {
            return true;
        }
        if let HkxValue::Object(sub) = &m.value {
            if has_mass_fields(sub) {
                return true;
            }
        }
    }
    false
}

/// FO76 has `hknpBoxShape`; FO4's hk_2014.1 classxml does not. FO76 box
/// shapes still carry a full convex hull payload, so preserve that payload
/// under FO4's supported `hknpConvexPolytopeShape` class.
fn convert_box_shapes_to_polytopes(hkx: &mut HkxFile, warnings: &mut Vec<String>) {
    const KEEP_MEMBERS: &[&str] = &[
        "flags",
        "numShapeKeyBits",
        "dispatchType",
        "convexRadius",
        "userData",
        "vertices",
        "planes",
        "faces",
        "indices",
        "properties",
    ];

    for object in hkx.objects_mut() {
        if object.class_name != "hknpBoxShape" {
            continue;
        }

        let object_name = object.name.as_deref().unwrap_or("<unnamed>").to_string();
        if !has_required_box_hull_members(&object.members) {
            warnings.push(format!(
                "unsupported hknpBoxShape {object_name} has no complete convex hull payload; leaving class unchanged"
            ));
            continue;
        }

        object.class_name = "hknpConvexPolytopeShape".to_string();
        object.signature = 1; // hknpConvexPolytopeShape_1.xml
        let members = std::mem::take(&mut object.members);
        object.members = members
            .into_iter()
            .filter(|member| KEEP_MEMBERS.contains(&member.name.as_str()))
            .collect();
        warnings.push(format!(
            "converted unsupported hknpBoxShape {object_name} to hknpConvexPolytopeShape using its existing hull payload"
        ));
    }
}

fn has_required_box_hull_members(members: &[HkxMember]) -> bool {
    ["vertices", "planes", "faces", "indices"]
        .into_iter()
        .all(|name| has_non_empty_array_member(members, name))
}

fn has_non_empty_array_member(members: &[HkxMember], name: &str) -> bool {
    members
        .iter()
        .find(|member| member.name == name)
        .map(|member| matches!(&member.value, HkxValue::Array(values) if !values.is_empty()))
        .unwrap_or(false)
}

fn migrate_unsupported_behavior_nodes(hkx: &mut HkxFile, warnings: &mut Vec<String>) {
    convert_locomotion_blend_generators(hkx, warnings);
    let delegated = collapse_zero_weight_action_layers(hkx, warnings);
    bone_weights::lower(hkx, &delegated);
    bypass_assign_bone_weights_modifiers(hkx, warnings);
}

#[derive(Debug)]
struct LocomotionBlendRewrite {
    object_index: usize,
    object_name: String,
    display_name: String,
    walk_generator: usize,
    jog_generator: usize,
    run_generator: usize,
    direction_variable_index: Option<i32>,
    movement_variable_index: Option<i32>,
    walk_event_id: Option<i32>,
    jog_event_id: Option<i32>,
    run_event_id: Option<i32>,
    transition_duration: f32,
}

fn convert_locomotion_blend_generators(hkx: &mut HkxFile, warnings: &mut Vec<String>) {
    let (event_name_to_id, _, _) = build_event_name_table(hkx);
    let rewrites: Vec<_> = hkx
        .objects()
        .iter()
        .enumerate()
        .filter_map(|(object_index, object)| {
            if object.class_name != "BSLocomotionBlendGenerator" {
                return None;
            }

            let object_name = object.name.as_deref().unwrap_or("<unnamed>").to_string();
            let Some(walk_generator) = pointer_member_value(&object.members, "pWalkBlendGenerator")
            else {
                warnings.push(format!(
                    "unsupported BSLocomotionBlendGenerator {object_name} has no pWalkBlendGenerator; leaving class unchanged"
                ));
                return None;
            };
            let Some(jog_generator) = pointer_member_value(&object.members, "pJogBlendGenerator")
            else {
                warnings.push(format!(
                    "unsupported BSLocomotionBlendGenerator {object_name} has no pJogBlendGenerator; leaving class unchanged"
                ));
                return None;
            };
            let Some(run_generator) = pointer_member_value(&object.members, "pRunBlendGenerator")
            else {
                warnings.push(format!(
                    "unsupported BSLocomotionBlendGenerator {object_name} has no pRunBlendGenerator; leaving class unchanged"
                ));
                return None;
            };

            let direction_variable_index = pointer_member_value(&object.members, "variableBindingSet")
                .and_then(|binding_set| {
                    binding_variable_index(hkx, binding_set, "fDirectionParameter")
                })
                .or_else(|| behavior_variable_index(hkx, "Direction"));
            // `iLocomotionSpeed` is the name FO76 creature/wrapping graphs actually
            // use for the int walk/jog/run tier — of the 33 source graphs carrying a
            // BSLocomotionBlendGenerator, 17 have neither sync name and 9 of those
            // expose `iLocomotionSpeed`. Without it `startStateId` stays unbound and
            // the selector freezes in its default state, so the actor never reaches
            // jog/run (converted Liberator: tracks but never charges).
            let movement_variable_index = behavior_variable_index(hkx, "iSyncLocomotionSpeed")
                .or_else(|| behavior_variable_index(hkx, "iMovementSpeed"))
                .or_else(|| behavior_variable_index(hkx, "iLocomotionSpeed"));
            let display_name = string_member_value(&object.members, "name")
                .unwrap_or("LocomotionBlend")
                .to_string();

            Some(LocomotionBlendRewrite {
                object_index,
                object_name,
                display_name,
                walk_generator,
                jog_generator,
                run_generator,
                direction_variable_index,
                movement_variable_index,
                walk_event_id: event_name_to_id.get("Walk").copied(),
                jog_event_id: event_name_to_id.get("Jog").copied(),
                run_event_id: event_name_to_id.get("Run").copied(),
                transition_duration: f32_member(&object.members, "fTransitionDuration")
                    .unwrap_or(0.2),
            })
        })
        .collect();

    for rewrite in rewrites {
        let walk_generator = create_cyclic_blend_wrapper(
            hkx,
            &format!("{}_Walk", rewrite.display_name),
            rewrite.walk_generator,
            rewrite.direction_variable_index,
            rewrite.transition_duration,
        );
        let jog_generator = create_cyclic_blend_wrapper(
            hkx,
            &format!("{}_Jog", rewrite.display_name),
            rewrite.jog_generator,
            rewrite.direction_variable_index,
            rewrite.transition_duration,
        );
        let run_generator = create_cyclic_blend_wrapper(
            hkx,
            &format!("{}_Run", rewrite.display_name),
            rewrite.run_generator,
            rewrite.direction_variable_index,
            rewrite.transition_duration,
        );

        let transition_effect = create_locomotion_transition_effect(
            hkx,
            &rewrite.display_name,
            rewrite.transition_duration,
        );
        let walk_transitions = create_locomotion_transition_array(
            hkx,
            [(rewrite.jog_event_id, 1), (rewrite.run_event_id, 2)],
            transition_effect,
        );
        let jog_transitions = create_locomotion_transition_array(
            hkx,
            [(rewrite.walk_event_id, 0), (rewrite.run_event_id, 2)],
            transition_effect,
        );
        let run_transitions = create_locomotion_transition_array(
            hkx,
            [(rewrite.walk_event_id, 0), (rewrite.jog_event_id, 1)],
            transition_effect,
        );

        let walk_state = push_named_object(
            hkx,
            "hkbStateMachineStateInfo",
            4,
            state_info_members("Walk", 0, walk_generator, walk_transitions),
        );
        let jog_state = push_named_object(
            hkx,
            "hkbStateMachineStateInfo",
            4,
            state_info_members("Jog", 1, jog_generator, jog_transitions),
        );
        let run_state = push_named_object(
            hkx,
            "hkbStateMachineStateInfo",
            4,
            state_info_members("Run", 2, run_generator, run_transitions),
        );

        let start_state_binding = rewrite
            .movement_variable_index
            .map(|variable_index| create_binding_set(hkx, "startStateId", variable_index));
        let members = state_machine_members(
            &rewrite.display_name,
            start_state_binding,
            [walk_state, jog_state, run_state],
        );
        let object = &mut hkx.objects_mut()[rewrite.object_index];
        object.class_name = "hkbStateMachine".to_string();
        object.signature = 5;
        object.members = members;

        let binding_note = if rewrite.movement_variable_index.is_some() {
            "bound startStateId to the locomotion-speed sync variable"
        } else {
            "left startStateId unbound because no locomotion-speed sync variable was found"
        };
        warnings.push(format!(
            "converted unsupported BSLocomotionBlendGenerator {} to hkbStateMachine selector; {binding_note}",
            rewrite.object_name
        ));
    }
}

#[derive(Debug)]
struct BoneWeightBypass {
    wrapper_index: usize,
    modifier_index: usize,
    generator_index: usize,
    wrapper_name: String,
    modifier_name: String,
}

#[derive(Debug)]
struct ZeroWeightActionLayerCollapse {
    layer_generator_index: usize,
    base_layer_index: usize,
    base_generator_index: usize,
    action_layer_index: usize,
    clear_action_motion_binding: bool,
    idle_wrapper_indices: Vec<usize>,
    layer_generator_name: String,
    base_generator_name: String,
    action_generator_name: String,
}

fn collapse_zero_weight_action_layers(hkx: &mut HkxFile, warnings: &mut Vec<String>) -> std::collections::HashSet<usize> {
    let mut delegated = std::collections::HashSet::new();
    let collapses: Vec<_> = hkx
        .objects()
        .iter()
        .enumerate()
        .filter_map(|(layer_generator_index, object)| {
            if object.class_name != "hkbLayerGenerator" {
                return None;
            }
            let layers = pointer_array_member_values(&object.members, "layers")?;
            let base_layer_index = *layers.first()?;
            let base_layer = hkx.objects().get(base_layer_index)?;
            if base_layer.class_name != "hkbLayer" {
                return None;
            }
            let base_generator_index = pointer_member_value(&base_layer.members, "generator")?;
            let base_generator = hkx.objects().get(base_generator_index)?;

            layers.iter().skip(1).find_map(|action_layer_index| {
                let action_layer = hkx.objects().get(*action_layer_index)?;
                if action_layer.class_name != "hkbLayer"
                    || action_layer
                        .members
                        .iter()
                        .find(|member| member.name == "onByDefault")
                        .and_then(|member| extract_int(&member.value))
                        != Some(1)
                {
                    return None;
                }
                let action_generator_index =
                    pointer_member_value(&action_layer.members, "generator")?;
                let action_generator = hkx.objects().get(action_generator_index)?;
                if action_generator.class_name != "hkbStateMachine" {
                    return None;
                }

                let idle_wrapper_indices: Vec<_> =
                    pointer_array_member_values(&action_generator.members, "states")?
                        .into_iter()
                        .filter_map(|state_index| {
                            let state = hkx.objects().get(state_index)?;
                            if state.class_name != "hkbStateMachineStateInfo" {
                                return None;
                            }
                            let wrapper_index = pointer_member_value(&state.members, "generator")?;
                            let wrapper = hkx.objects().get(wrapper_index)?;
                            if wrapper.class_name != "hkbModifierGenerator" {
                                return None;
                            }
                            let child_index = pointer_member_value(&wrapper.members, "generator")?;
                            if hkx.objects().get(child_index)?.class_name
                                != "hkbReferencePoseGenerator"
                            {
                                return None;
                            }
                            let modifier_index =
                                pointer_member_value(&wrapper.members, "modifier")?;
                            modifier_tree_contains_zero_weight_assignment(hkx, modifier_index)
                                .then_some(wrapper_index)
                        })
                        .collect();
                if idle_wrapper_indices.is_empty() {
                    return None;
                }

                Some(ZeroWeightActionLayerCollapse {
                    layer_generator_index,
                    base_layer_index,
                    base_generator_index,
                    action_layer_index: *action_layer_index,
                    clear_action_motion_binding: pointer_member_value(
                        &action_layer.members,
                        "variableBindingSet",
                    )
                    .is_some_and(|binding_set_index| {
                        binding_set_only_binds_member(hkx, binding_set_index, "useMotion")
                    }),
                    idle_wrapper_indices,
                    layer_generator_name: string_member_value(&object.members, "name")
                        .or(object.name.as_deref())
                        .unwrap_or("<unnamed>")
                        .to_string(),
                    base_generator_name: string_member_value(&base_generator.members, "name")
                        .or(base_generator.name.as_deref())
                        .unwrap_or("<unnamed>")
                        .to_string(),
                    action_generator_name: string_member_value(&action_generator.members, "name")
                        .or(action_generator.name.as_deref())
                        .unwrap_or("<unnamed>")
                        .to_string(),
                })
            })
        })
        .collect();

    for collapse in collapses {
        let removed_base_layer = hkx.objects_mut()[collapse.layer_generator_index]
            .members
            .iter_mut()
            .find(|member| member.name == "layers")
            .and_then(|member| match &mut member.value {
                HkxValue::Array(layers)
                    if layers.first()
                        == Some(&HkxValue::Pointer(Some(collapse.base_layer_index))) =>
                {
                    layers.remove(0);
                    Some(())
                }
                _ => None,
            })
            .is_some();
        if !removed_base_layer {
            continue;
        }

        for wrapper_index in collapse.idle_wrapper_indices {
            delegated.insert(wrapper_index);
            if let Some(member) = hkx.objects_mut()[wrapper_index]
                .members
                .iter_mut()
                .find(|member| member.name == "generator")
            {
                member.value = HkxValue::Pointer(Some(collapse.base_generator_index));
            }
        }

        let action_layer = &mut hkx.objects_mut()[collapse.action_layer_index];
        if collapse.clear_action_motion_binding {
            if let Some(member) = action_layer
                .members
                .iter_mut()
                .find(|member| member.name == "variableBindingSet")
            {
                member.value = HkxValue::Pointer(None);
            }
        }
        if let Some(member) = action_layer
            .members
            .iter_mut()
            .find(|member| member.name == "useMotion")
        {
            set_int_member(&mut member.value, 1);
        }

        if let Some(member) = hkx.objects_mut()[collapse.layer_generator_index]
            .members
            .iter_mut()
            .find(|member| member.name == "indexOfSyncMasterChild")
        {
            if let Some(index) = extract_int(&member.value).filter(|index| *index > 0) {
                set_int_member(&mut member.value, index - 1);
            }
        }

        warnings.push(format!(
            "collapsed zero-weight action state machine {} over base generator {} in {}; idle reference pose now delegates to base locomotion and the collapsed layer remains motion-enabled",
            collapse.action_generator_name,
            collapse.base_generator_name,
            collapse.layer_generator_name
        ));
    }
    delegated
}

fn pointer_array_member_values(members: &[HkxMember], name: &str) -> Option<Vec<usize>> {
    let member = members.iter().find(|member| member.name == name)?;
    let HkxValue::Array(values) = &member.value else {
        return None;
    };
    Some(
        values
            .iter()
            .filter_map(|value| match value {
                HkxValue::Pointer(Some(index)) => Some(*index),
                _ => None,
            })
            .collect(),
    )
}

fn modifier_tree_contains_zero_weight_assignment(hkx: &HkxFile, index: usize) -> bool {
    let Some(modifier) = hkx.objects().get(index) else {
        return false;
    };
    match modifier.class_name.as_str() {
        "BSAssignBoneWeightsModifier" => is_zero_weight_assignment(hkx, modifier),
        "hkbModifierList" => pointer_array_member_values(&modifier.members, "modifiers")
            .is_some_and(|modifiers| {
                modifiers
                    .into_iter()
                    .any(|index| modifier_tree_contains_zero_weight_assignment(hkx, index))
            }),
        _ => false,
    }
}

fn is_zero_weight_assignment(hkx: &HkxFile, modifier: &HkxObject) -> bool {
    let normalized_name: String = string_member_value(&modifier.members, "name")
        .or(modifier.name.as_deref())
        .unwrap_or_default()
        .chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    if normalized_name.contains("zeroweights") {
        return true;
    }

    // Naming is not a reliable gate: the floater calls its zero assignment
    // `AssignBoneWeights_Zero`, which normalizes to `assignboneweightszero` and
    // misses the substring above, and binds no variable. Read the mask instead.
    if pointer_member_value(&modifier.members, "boneWeights1")
        .is_some_and(|index| is_all_zero_bone_weight_array(hkx, index))
    {
        return true;
    }

    let Some(binding_set_index) = pointer_member_value(&modifier.members, "variableBindingSet")
    else {
        return false;
    };
    let Some(character_property_index) =
        binding_character_property_index(hkx, binding_set_index, "boneWeights1")
    else {
        return false;
    };
    behavior_character_property_name(hkx, character_property_index)
        .is_some_and(|name| name.eq_ignore_ascii_case("ZeroBoneWeights"))
}

/// An empty `boneWeights` array means "no mask", i.e. full weight — the opposite
/// of a zero assignment — so emptiness must not read as all-zero here.
fn is_all_zero_bone_weight_array(hkx: &HkxFile, index: usize) -> bool {
    let Some(array) = hkx.objects().get(index) else {
        return false;
    };
    if array.class_name != "hkbBoneWeightArray" {
        return false;
    }
    let Some(member) = array
        .members
        .iter()
        .find(|member| member.name == "boneWeights")
    else {
        return false;
    };
    let weights = match &member.value {
        HkxValue::F32List(weights) => weights.clone(),
        HkxValue::Array(values) => {
            let Some(weights) = values
                .iter()
                .map(extract_float)
                .collect::<Option<Vec<f32>>>()
            else {
                return false;
            };
            weights
        }
        _ => return false,
    };
    !weights.is_empty() && !weights.iter().any(|weight| weight.abs() > f32::EPSILON)
}

fn extract_float(value: &HkxValue) -> Option<f32> {
    match value {
        HkxValue::F32(value) | HkxValue::Half(value) => Some(*value),
        other => extract_int(other).map(|value| value as f32),
    }
}

fn binding_character_property_index(
    hkx: &HkxFile,
    binding_set_index: usize,
    member_path: &str,
) -> Option<usize> {
    let binding_set = hkx.objects().get(binding_set_index)?;
    binding_set.members.iter().find_map(|member| {
        if member.name != "bindings" {
            return None;
        }
        let HkxValue::Array(bindings) = &member.value else {
            return None;
        };
        bindings.iter().find_map(|binding| {
            let members = binding.as_object_members()?;
            let path = members
                .iter()
                .find(|member| member.name == "memberPath")
                .and_then(|member| string_value(&member.value))?;
            let binding_type = members
                .iter()
                .find(|member| member.name == "bindingType")
                .and_then(|member| extract_int(&member.value))?;
            if path != member_path || binding_type != 1 {
                return None;
            }
            members
                .iter()
                .find(|member| member.name == "variableIndex")
                .and_then(|member| extract_int(&member.value))
                .and_then(|index| usize::try_from(index).ok())
        })
    })
}

fn behavior_character_property_name(hkx: &HkxFile, index: usize) -> Option<&str> {
    hkx.objects().iter().find_map(|object| {
        object.members.iter().find_map(|member| {
            if member.name != "characterPropertyNames" {
                return None;
            }
            let HkxValue::Array(names) = &member.value else {
                return None;
            };
            names.get(index).and_then(string_value)
        })
    })
}

fn bypass_assign_bone_weights_modifiers(hkx: &mut HkxFile, warnings: &mut Vec<String>) {
    let bypasses: Vec<_> = hkx
        .objects()
        .iter()
        .enumerate()
        .filter_map(|(wrapper_index, object)| {
            if object.class_name != "hkbModifierGenerator" {
                return None;
            }
            let modifier_index = pointer_member_value(&object.members, "modifier")?;
            let modifier = hkx.objects().get(modifier_index)?;
            if modifier.class_name != "BSAssignBoneWeightsModifier" {
                return None;
            }
            let Some(generator_index) = pointer_member_value(&object.members, "generator") else {
                let object_name = object.name.as_deref().unwrap_or("<unnamed>");
                warnings.push(format!(
                    "unsupported BSAssignBoneWeightsModifier referenced by {object_name} has no child generator; leaving class unchanged"
                ));
                return None;
            };
            Some(BoneWeightBypass {
                wrapper_index,
                modifier_index,
                generator_index,
                wrapper_name: object.name.as_deref().unwrap_or("<unnamed>").to_string(),
                modifier_name: modifier
                    .name
                    .as_deref()
                    .unwrap_or("<unnamed>")
                    .to_string(),
            })
        })
        .collect();

    for bypass in &bypasses {
        replace_object_references(hkx, bypass.wrapper_index, bypass.generator_index);
        warnings.push(format!(
            "bypassed unsupported BSAssignBoneWeightsModifier {} by rewiring hkbModifierGenerator {} to its child generator",
            bypass.modifier_name, bypass.wrapper_name
        ));
    }

    let mut remove: std::collections::HashSet<usize> = bypasses
        .iter()
        .flat_map(|bypass| [bypass.wrapper_index, bypass.modifier_index])
        .collect();

    // The class is not registered in the FO4 runtime, so any surviving
    // instance (typically referenced from hkbModifierList.modifiers rather
    // than an hkbModifierGenerator wrapper) aborts the whole graph load.
    // Removing the object lets pointer remapping drop it from modifier arrays.
    for (index, object) in hkx.objects().iter().enumerate() {
        if object.class_name == "BSAssignBoneWeightsModifier" && !remove.contains(&index) {
            let display_name = string_member_value(&object.members, "name")
                .or(object.name.as_deref())
                .unwrap_or("<unnamed>");
            warnings.push(format!(
                "removed unsupported BSAssignBoneWeightsModifier {display_name} referenced outside an hkbModifierGenerator wrapper"
            ));
            remove.insert(index);
        }
    }

    if remove.is_empty() {
        return;
    }
    hkx.retain_objects_remap_pointers(|index, _| !remove.contains(&index));
}

fn create_cyclic_blend_wrapper(
    hkx: &mut HkxFile,
    name: &str,
    blender_generator: usize,
    direction_variable_index: Option<i32>,
    transition_duration: f32,
) -> usize {
    let binding_set = direction_variable_index
        .map(|variable_index| create_binding_set(hkx, "fBlendParameter", variable_index));
    push_named_object(
        hkx,
        "BSCyclicBlendTransitionGenerator",
        1,
        vec![
            member_value("variableBindingSet", HkxValue::Pointer(binding_set)),
            member_value("userData", HkxValue::U64(0)),
            member_value("name", string_hkx_value(name)),
            member_value(
                "pBlenderGenerator",
                HkxValue::Pointer(Some(blender_generator)),
            ),
            member_value("EventToFreezeBlendValue", event_property(-1)),
            member_value("EventToCrossBlend", event_property(-1)),
            member_value("TransitionOutEvent", event_property(-1)),
            member_value("TransitionInEvent", event_property(-1)),
            member_value("fBlendParameter", HkxValue::F32(0.0)),
            member_value("fTransitionDuration", HkxValue::F32(transition_duration)),
            member_value("eBlendCurve", HkxValue::I8(0)),
        ],
    )
}

fn create_locomotion_transition_effect(hkx: &mut HkxFile, name: &str, duration: f32) -> usize {
    push_named_object(
        hkx,
        "hkbBlendingTransitionEffect",
        1,
        vec![
            member_value("variableBindingSet", HkxValue::Pointer(None)),
            member_value("userData", HkxValue::U64(0)),
            member_value("name", string_hkx_value(&format!("{name}_SpeedTransition"))),
            member_value("selfTransitionMode", HkxValue::I8(1)),
            member_value("eventMode", HkxValue::I8(0)),
            member_value("duration", HkxValue::F32(duration)),
            member_value("toGeneratorStartTimeFraction", HkxValue::F32(0.0)),
            member_value("flags", HkxValue::I32(0)),
            member_value("endMode", HkxValue::I8(0)),
            member_value("blendCurve", HkxValue::I8(0)),
            member_value("alignmentBone", HkxValue::I16(-1)),
        ],
    )
}

fn create_locomotion_transition_array(
    hkx: &mut HkxFile,
    transitions: [(Option<i32>, i32); 2],
    transition_effect: usize,
) -> Option<usize> {
    let transitions: Vec<_> = transitions
        .into_iter()
        .filter_map(|(event_id, to_state_id)| {
            event_id.map(|event_id| {
                HkxValue::Object(vec![
                    member_value(
                        "triggerInterval",
                        HkxValue::Object(vec![
                            member_value("enterEventId", HkxValue::I32(-1)),
                            member_value("exitEventId", HkxValue::I32(-1)),
                            member_value("enterTime", HkxValue::F32(0.0)),
                            member_value("exitTime", HkxValue::F32(0.0)),
                        ]),
                    ),
                    member_value(
                        "initiateInterval",
                        HkxValue::Object(vec![
                            member_value("enterEventId", HkxValue::I32(-1)),
                            member_value("exitEventId", HkxValue::I32(-1)),
                            member_value("enterTime", HkxValue::F32(0.0)),
                            member_value("exitTime", HkxValue::F32(0.0)),
                        ]),
                    ),
                    member_value("transition", HkxValue::Pointer(Some(transition_effect))),
                    member_value("condition", HkxValue::Pointer(None)),
                    member_value("eventId", HkxValue::I32(event_id)),
                    member_value("toStateId", HkxValue::I32(to_state_id)),
                    member_value("fromNestedStateId", HkxValue::I32(0)),
                    member_value("toNestedStateId", HkxValue::I32(0)),
                    member_value("priority", HkxValue::I32(0)),
                    member_value("flags", HkxValue::I32(0)),
                ])
            })
        })
        .collect();
    if transitions.is_empty() {
        return None;
    }

    Some(push_named_object(
        hkx,
        "hkbStateMachineTransitionInfoArray",
        1,
        vec![member_value("transitions", HkxValue::Array(transitions))],
    ))
}

fn state_info_members(
    name: &str,
    state_id: i32,
    generator: usize,
    transitions: Option<usize>,
) -> Vec<HkxMember> {
    vec![
        member_value("variableBindingSet", HkxValue::Pointer(None)),
        member_value("listeners", HkxValue::Array(vec![])),
        member_value("enterNotifyEvents", HkxValue::Pointer(None)),
        member_value("exitNotifyEvents", HkxValue::Pointer(None)),
        member_value("transitions", HkxValue::Pointer(transitions)),
        member_value("generator", HkxValue::Pointer(Some(generator))),
        member_value("name", string_hkx_value(name)),
        member_value("stateId", HkxValue::I32(state_id)),
        member_value("probability", HkxValue::F32(1.0)),
        member_value("enable", HkxValue::Bool(true)),
    ]
}

fn state_machine_members(
    name: &str,
    binding_set: Option<usize>,
    states: [usize; 3],
) -> Vec<HkxMember> {
    vec![
        member_value("variableBindingSet", HkxValue::Pointer(binding_set)),
        member_value("userData", HkxValue::U64(0)),
        member_value("name", string_hkx_value(name)),
        member_value(
            "eventToSendWhenStateOrTransitionChanges",
            event_property(-1),
        ),
        member_value("startStateIdSelector", HkxValue::Pointer(None)),
        member_value("startStateId", HkxValue::I32(0)),
        member_value("returnToPreviousStateEventId", HkxValue::I32(-1)),
        member_value("randomTransitionEventId", HkxValue::I32(-1)),
        member_value("transitionToNextHigherStateEventId", HkxValue::I32(-1)),
        member_value("transitionToNextLowerStateEventId", HkxValue::I32(-1)),
        member_value("syncVariableIndex", HkxValue::I32(-1)),
        member_value("wrapAroundStateId", HkxValue::Bool(false)),
        member_value("maxSimultaneousTransitions", HkxValue::I8(32)),
        member_value("startStateMode", HkxValue::I8(0)),
        member_value("selfTransitionMode", HkxValue::I8(0)),
        member_value(
            "states",
            HkxValue::Array(
                states
                    .into_iter()
                    .map(|state| HkxValue::Pointer(Some(state)))
                    .collect(),
            ),
        ),
        member_value("wildcardTransitions", HkxValue::Pointer(None)),
    ]
}

fn create_binding_set(hkx: &mut HkxFile, member_path: &str, variable_index: i32) -> usize {
    push_named_object(
        hkx,
        "hkbVariableBindingSet",
        2,
        vec![
            member_value(
                "bindings",
                HkxValue::Array(vec![HkxValue::Object(vec![
                    member_value("memberPath", string_hkx_value(member_path)),
                    member_value("variableIndex", HkxValue::I32(variable_index)),
                    member_value("bitIndex", HkxValue::I8(-1)),
                    member_value("bindingType", HkxValue::I8(0)),
                ])]),
            ),
            member_value("indexOfBindingToEnable", HkxValue::I32(-1)),
        ],
    )
}

fn behavior_variable_index(hkx: &HkxFile, variable_name: &str) -> Option<i32> {
    hkx.objects().iter().find_map(|object| {
        object.members.iter().find_map(|member| {
            if member.name != "variableNames" {
                return None;
            }
            let HkxValue::Array(values) = &member.value else {
                return None;
            };
            values.iter().enumerate().find_map(|(index, value)| {
                (string_value(value) == Some(variable_name)).then_some(index as i32)
            })
        })
    })
}

fn binding_variable_index(
    hkx: &HkxFile,
    binding_set_index: usize,
    member_path: &str,
) -> Option<i32> {
    let binding_set = hkx.objects().get(binding_set_index)?;
    binding_set.members.iter().find_map(|member| {
        if member.name != "bindings" {
            return None;
        }
        let HkxValue::Array(bindings) = &member.value else {
            return None;
        };
        bindings.iter().find_map(|binding| {
            let members = binding.as_object_members()?;
            let path = members
                .iter()
                .find(|member| member.name == "memberPath")
                .and_then(|member| string_value(&member.value))?;
            if path != member_path {
                return None;
            }
            members
                .iter()
                .find(|member| member.name == "variableIndex")
                .and_then(|member| extract_int(&member.value))
        })
    })
}

fn binding_set_only_binds_member(
    hkx: &HkxFile,
    binding_set_index: usize,
    member_path: &str,
) -> bool {
    let Some(binding_set) = hkx.objects().get(binding_set_index) else {
        return false;
    };
    let Some(HkxValue::Array(bindings)) = binding_set
        .members
        .iter()
        .find(|member| member.name == "bindings")
        .map(|member| &member.value)
    else {
        return false;
    };
    !bindings.is_empty()
        && bindings.iter().all(|binding| {
            binding
                .as_object_members()
                .and_then(|members| string_member_value(members, "memberPath"))
                == Some(member_path)
        })
}

fn replace_object_references(hkx: &mut HkxFile, from_index: usize, to_index: usize) {
    let map = std::collections::HashMap::from([(from_index, to_index)]);
    for object in hkx.objects_mut() {
        for member in &mut object.members {
            remap_pointers(&mut member.value, &map);
        }
    }
}

fn push_named_object(
    hkx: &mut HkxFile,
    class_name: &str,
    signature: u32,
    members: Vec<HkxMember>,
) -> usize {
    let name = format!("#{:04}", max_numbered_object_name(hkx) + 1);
    hkx.push_object(HkxObject {
        name: Some(name),
        offset: 0,
        signature,
        class_name: class_name.to_string(),
        members,
    })
}

fn member_value(name: &str, value: HkxValue) -> HkxMember {
    HkxMember {
        name: name.to_string(),
        value,
    }
}

fn string_hkx_value(value: &str) -> HkxValue {
    HkxValue::String {
        value: value.to_string(),
        is_null: false,
    }
}

fn event_property(id: i32) -> HkxValue {
    HkxValue::Object(vec![
        member_value("id", HkxValue::I32(id)),
        member_value("payload", HkxValue::Pointer(None)),
    ])
}

fn string_member_value<'a>(members: &'a [HkxMember], name: &str) -> Option<&'a str> {
    members
        .iter()
        .find(|member| member.name == name)
        .and_then(|member| string_value(&member.value))
}

/// Null connectivity pointers on capsule/polytope shapes and drop the
/// connectivity wrapper objects (hkUint16 / hknpConvexPolytopeShape::Connectivity).
fn strip_shape_connectivity(hkx: &mut HkxFile) {
    // Null connectivity pointers on shapes.
    for object in hkx.objects_mut() {
        if !matches!(
            object.class_name.as_str(),
            "hknpConvexPolytopeShape" | "hknpCapsuleShape"
        ) {
            continue;
        }
        for m in object.members.iter_mut() {
            if m.name == "connectivity" {
                if let HkxValue::Pointer(target) = &mut m.value {
                    *target = None;
                }
            }
        }
    }

    // Drop connectivity wrapper objects, remapping pointers.
    hkx.retain_objects_remap_pointers(|_, object| {
        !matches!(
            object.class_name.as_str(),
            "hkUint16" | "hknpConvexPolytopeShape::Connectivity"
        )
    });
}

/// Replace each `hknpConvexPolytopeShape` with a structurally-equivalent
/// `hknpCapsuleShape` whose endpoints span the polytope's longest AABB axis.
/// Only fires on NIF-embedded blobs (no hkRootLevelContainer) that contain
/// an `hknpPhysicsSystemData` root.
///
/// The replacement is in-place so existing pointer indices remain valid.
fn convert_polytope_to_capsule(hkx: &mut HkxFile) {
    let has_psd = hkx
        .objects()
        .iter()
        .any(|o| o.class_name == "hknpPhysicsSystemData");
    if !has_psd {
        return;
    }
    let is_standalone = hkx
        .objects()
        .iter()
        .any(|o| o.class_name == "hkRootLevelContainer");
    if is_standalone {
        return;
    }

    let polytope_indices: Vec<usize> = hkx
        .objects()
        .iter()
        .enumerate()
        .filter_map(|(i, o)| (o.class_name == "hknpConvexPolytopeShape").then_some(i))
        .collect();
    if polytope_indices.is_empty() {
        return;
    }

    for idx in polytope_indices {
        let Some(capsule) = polytope_to_capsule(&hkx.objects()[idx]) else {
            continue;
        };
        let object = &mut hkx.objects_mut()[idx];
        object.class_name = "hknpCapsuleShape".to_string();
        // Preserve `name` (so existing pointer indices stay valid — the index
        // doesn't change anyway) but replace the members entirely.
        object.members = capsule;
    }
}

/// Build a capsule member list from a polytope's AABB. Returns the fields a
/// vanilla FO4 hknpCapsuleShape carries; the `_migrate_skeleton_physics`
/// pass downstream handles flag/dispatch/plane normalization.
fn polytope_to_capsule(polytope: &HkxObject) -> Option<Vec<HkxMember>> {
    let mut vertices: Vec<[f32; 3]> = Vec::new();
    let mut convex_radius: f32 = 0.01;
    let mut user_data: i64 = 0;

    for m in &polytope.members {
        match m.name.as_str() {
            "vertices" => {
                if let HkxValue::Array(values) = &m.value {
                    for v in values {
                        if let HkxValue::F32List(xyz) = v {
                            if xyz.len() >= 3 {
                                vertices.push([xyz[0], xyz[1], xyz[2]]);
                            }
                        }
                    }
                }
            }
            "convexRadius" => {
                if let HkxValue::F32(r) = m.value {
                    convex_radius = r;
                }
            }
            "userData" => {
                user_data = extract_int(&m.value).unwrap_or(0) as i64;
            }
            _ => {}
        }
    }

    if vertices.is_empty() {
        return None;
    }

    let mut min_v = vertices[0];
    let mut max_v = vertices[0];
    for v in &vertices {
        for i in 0..3 {
            if v[i] < min_v[i] {
                min_v[i] = v[i];
            }
            if v[i] > max_v[i] {
                max_v[i] = v[i];
            }
        }
    }
    let extents = [
        max_v[0] - min_v[0],
        max_v[1] - min_v[1],
        max_v[2] - min_v[2],
    ];
    let mut axis = 0usize;
    for i in 1..3 {
        if extents[i] > extents[axis] {
            axis = i;
        }
    }
    let center = [
        (min_v[0] + max_v[0]) * 0.5,
        (min_v[1] + max_v[1]) * 0.5,
        (min_v[2] + max_v[2]) * 0.5,
    ];
    let half = extents[axis] * 0.5;
    // Capsule radius = half the larger perpendicular extent.
    let perp = [(axis + 1) % 3, (axis + 2) % 3];
    let radius = (extents[perp[0]].max(extents[perp[1]])) * 0.5;

    let mut a = [center[0], center[1], center[2], 0.0];
    let mut b = [center[0], center[1], center[2], 0.0];
    a[axis] = center[axis] - half;
    b[axis] = center[axis] + half;

    Some(vec![
        HkxMember {
            name: "userData".to_string(),
            value: HkxValue::U64(user_data as u64),
        },
        HkxMember {
            name: "convexRadius".to_string(),
            value: HkxValue::F32(if convex_radius > 0.0 {
                convex_radius
            } else {
                radius.max(0.01)
            }),
        },
        HkxMember {
            name: "a".to_string(),
            value: HkxValue::F32List(a.to_vec()),
        },
        HkxMember {
            name: "b".to_string(),
            value: HkxValue::F32List(b.to_vec()),
        },
        HkxMember {
            name: "flags".to_string(),
            value: HkxValue::I32(451),
        },
        HkxMember {
            name: "dispatchType".to_string(),
            value: HkxValue::I32(1),
        },
    ])
}

const FO76_ONLY_STRIP_CLASSES: &[&str] = &[
    "hkCompressedMassProperties",
    "hknpCompressedMeshShape",
    "hknpCompressedMeshShapeData",
    "hknpCompressedMeshShapeTree",
    "hknpSparseCompactMapunsignedshort",
    "hkcdSimdTreeNode",
    "hkcdSimdTree",
    "hkBitField",
    "hkBitFieldStoragehkArrayunsignedinthkContainerHeapAllocator",
    // Additional FO76-only compound/mesh internals absent from FO4 hk_2014.1 classxml
    // (SDK 2014_2_5 hknpPatches references these as CLASS_ADDED; FO4's earlier fork omits them)
    "hknpStaticCompoundShapeData",
    "hknpStaticCompoundShapeTree",
    "hknpDynamicCompoundShapeKeyMask",
    "hknpStaticCompoundShapeKeyMask",
    "hknpCompoundShapeInternalsKeyMask",
    // Flat form of `hknpCompressedMeshShapeInternals::KeyMask` after nested-class flattening
    "hknpCompressedMeshShapeInternalsKeyMask",
    // Added after FO4's hk_2014.1 target; FO4 cloth states serialize operator lists directly.
    "hclStateDependencyGraph",
];

/// Drop FO76-only classes that have no FO4 classxml entry. Pointers
/// referencing them get nulled. Strip the `properties` member from
/// shape classes (FO4 has no `properties` slot on shapes).
fn strip_fo76_skeleton_classes(hkx: &mut HkxFile) {
    let any_match = hkx
        .objects()
        .iter()
        .any(|o| FO76_ONLY_STRIP_CLASSES.contains(&o.class_name.as_str()));
    if !any_match {
        return;
    }

    hkx.retain_objects_remap_pointers(|_, object| {
        !FO76_ONLY_STRIP_CLASSES.contains(&object.class_name.as_str())
    });

    for object in hkx.objects_mut() {
        if !matches!(
            object.class_name.as_str(),
            "hknpCapsuleShape"
                | "hknpConvexPolytopeShape"
                | "hknpSphereShape"
                | "hknpCompressedMeshShapeData"
        ) {
            continue;
        }
        object.members.retain(|m| m.name != "properties");
    }
}

/// Decoded mass data extracted from a shape's `hknpShapeMassProperties` block,
/// keyed by the shape object's name so the mapping survives the index remapping
/// performed by `strip_fo76_skeleton_classes`.
struct ShapeMassInfo {
    /// 1/mass — ready to write into `hknpMotionCinfo.inverseMass`.
    inverse_mass: f32,
    /// Forward principal-axis inertia (actual mass, NOT unit-mass).
    /// Invert per-axis to get `inverseInertiaLocal`.
    forward_inertia: [f32; 3],
    /// Center of mass in shape-local space.
    center_of_mass: [f32; 3],
    /// Unit quaternion (x,y,z,w): inertia major-axis space → shape space.
    major_axis_space: [f32; 4],
}

/// Build a map from shape-object-name → [`ShapeMassInfo`] by traversing the
/// `properties → hkRefCountedProperties → entries[key=0xF100] → hknpShapeMassProperties`
/// chain while it is still intact (call this **before**
/// `strip_fo76_skeleton_classes` removes the `properties` member).
///
/// The map is keyed by object name rather than by index so it remains valid
/// after `retain_objects_remap_pointers` reorders the object list.
fn extract_shape_mass_cache(hkx: &HkxFile) -> std::collections::HashMap<String, ShapeMassInfo> {
    use crate::collision::constants::REFCOUNTED_PROPS_KEY_MASS_PROPS;
    use crate::collision::mass_properties::{unpack_unit_quat, unpack_vector3};

    let objects = hkx.objects();
    let mut cache = std::collections::HashMap::new();

    // Real FO76 packed vectors may wrap their array in an inline `values` object.
    let extract_i16x4_bytes = |v: &HkxValue| -> Option<[u8; 8]> {
        let arr = match v {
            HkxValue::Array(arr) => arr,
            _ => v
                .as_object_members()?
                .iter()
                .find(|member| member.name == "values")
                .and_then(|member| match &member.value {
                    HkxValue::Array(values) => Some(values),
                    _ => None,
                })?,
        };
        if arr.len() != 4 {
            return None;
        }
        let mut bytes = [0u8; 8];
        for (i, item) in arr.iter().enumerate() {
            let HkxValue::I16(x) = item else {
                return None;
            };
            bytes[i * 2..i * 2 + 2].copy_from_slice(&x.to_le_bytes());
        }
        Some(bytes)
    };

    for (shape_idx, shape_obj) in objects.iter().enumerate() {
        // Shape must have a name (used as stable cache key across remapping).
        let shape_name = match &shape_obj.name {
            Some(n) => n.clone(),
            None => continue,
        };

        // Follow shape.properties → hkRefCountedProperties.
        let props_idx = match shape_obj
            .members
            .iter()
            .find(|m| m.name == "properties")
            .and_then(|m| {
                if let HkxValue::Pointer(Some(i)) = m.value {
                    Some(i)
                } else {
                    None
                }
            }) {
            Some(i) => i,
            None => continue,
        };

        let props_obj = match objects.get(props_idx) {
            Some(o) if o.class_name == "hkRefCountedProperties" => o,
            _ => continue,
        };

        // Scan entries for key == REFCOUNTED_PROPS_KEY_MASS_PROPS (0xF100).
        let entries = match props_obj
            .members
            .iter()
            .find(|m| m.name == "entries")
            .and_then(|m| {
                if let HkxValue::Array(e) = &m.value {
                    Some(e)
                } else {
                    None
                }
            }) {
            Some(e) => e,
            None => continue,
        };

        let mass_props_idx = 'entry_search: {
            for entry in entries {
                let Some(em) = entry.as_object_members() else {
                    continue;
                };
                let key_matches = em.iter().any(|m| {
                    m.name == "key"
                        && matches!(m.value, HkxValue::U16(k) if k == REFCOUNTED_PROPS_KEY_MASS_PROPS)
                });
                if !key_matches {
                    continue;
                }
                if let Some(obj_ptr) = em.iter().find(|m| m.name == "object").and_then(|m| {
                    if let HkxValue::Pointer(Some(i)) = m.value {
                        Some(i)
                    } else {
                        None
                    }
                }) {
                    break 'entry_search Some(obj_ptr);
                }
            }
            None
        };

        let mass_props_idx = match mass_props_idx {
            Some(i) => i,
            None => continue,
        };

        let mass_props_obj = match objects.get(mass_props_idx) {
            Some(o) if o.class_name == "hknpShapeMassProperties" => o,
            _ => continue,
        };

        // Read hkCompressedMassProperties from compressedMassProperties member.
        let cmp_members = match mass_props_obj
            .members
            .iter()
            .find(|m| m.name == "compressedMassProperties")
            .and_then(|m| m.value.as_object_members())
        {
            Some(m) => m,
            None => continue,
        };

        // mass (plain f32 — no packing).
        let mass = match cmp_members.iter().find(|m| m.name == "mass").and_then(|m| {
            if let HkxValue::F32(v) = m.value {
                Some(v)
            } else {
                None
            }
        }) {
            Some(v) if v > 0.0 && v.is_finite() => v,
            _ => continue,
        };

        // Unpack centerOfMass (hkPackedVector3 → 3 f32).
        let com_bytes = cmp_members
            .iter()
            .find(|m| m.name == "centerOfMass")
            .and_then(|m| extract_i16x4_bytes(&m.value));
        let center_of_mass = com_bytes.map(|b| unpack_vector3(&b)).unwrap_or([0.0; 3]);

        // Unpack inertia — stored as FORWARD principal inertia (see serialize_mass_properties_block).
        let inertia_bytes = cmp_members
            .iter()
            .find(|m| m.name == "inertia")
            .and_then(|m| extract_i16x4_bytes(&m.value));
        let forward_inertia = inertia_bytes
            .map(|b| unpack_vector3(&b))
            .unwrap_or([0.0; 3]);

        // Unpack majorAxisSpace (hkPackedUnitVector<4> → unit quaternion xyzw).
        let major_bytes = cmp_members
            .iter()
            .find(|m| m.name == "majorAxisSpace")
            .and_then(|m| extract_i16x4_bytes(&m.value));
        let major_axis_space = major_bytes
            .map(|b| unpack_unit_quat(&b))
            .unwrap_or([0.0, 0.0, 0.0, 1.0]);

        // Suppress unused warning for the shape_idx binding (used implicitly via the loop).
        let _ = shape_idx;

        cache.insert(
            shape_name,
            ShapeMassInfo {
                inverse_mass: 1.0 / mass,
                forward_inertia,
                center_of_mass,
                major_axis_space,
            },
        );
    }

    cache
}

fn normalize_dynamic_compound_shapes(hkx: &mut HkxFile) {
    let dynamic_data_index = hkx
        .objects()
        .iter()
        .position(|object| object.class_name == "hknpDynamicCompoundShapeData");
    if dynamic_data_index.is_none() {
        return;
    }
    let dynamic_data_index = dynamic_data_index.unwrap();

    for object in hkx.objects_mut() {
        if object.class_name != "hknpCompoundShape" {
            continue;
        }
        let has_mutable_member = object
            .members
            .iter()
            .any(|member| member.name == "isMutable");
        if !has_mutable_member {
            continue;
        }

        object.class_name = "hknpDynamicCompoundShape".to_string();
        object.signature = 1;
        for member in &mut object.members {
            match member.name.as_str() {
                "dispatchType" => set_int_member(&mut member.value, 2),
                "isMutable" => member.value = HkxValue::Bool(true),
                _ => {}
            }
        }
        if !object
            .members
            .iter()
            .any(|member| member.name == "boundingVolumeData")
        {
            object.members.push(HkxMember {
                name: "boundingVolumeData".to_string(),
                value: HkxValue::Pointer(Some(dynamic_data_index)),
            });
        }
    }
}

/// Repopulate the `instances` free-list array on dynamic compounds migrated
/// from a FO76 *static* `hknpCompoundShape`.
///
/// FO76 stores a multi-shape ragdoll body as a static `hknpCompoundShape`
/// (`isMutable=0`) whose children + per-child transforms are baked into the
/// `boundingVolumeData` BVH tree, leaving the `instances` free-list **empty**.
/// `normalize_dynamic_compound_shapes` renames it to FO4's
/// `hknpDynamicCompoundShape`, which reads its children from `instances` — so an
/// empty array yields a degenerate compound that crashes the CK on load (reads
/// `element[0]` of a 0-length array) and the game on the physics-settle worker
/// (null `hknpBody`, `Fallout4.exe+18C4195`).
///
/// Each child leaf shape is one that no body references directly; its body-space
/// placement is the matching tree leaf AABB (matched by translation-invariant
/// extent), giving an identity-rotation transform whose translation is
/// `leaf_center - child_center`. The baked tree stays untouched — for a dynamic
/// compound the runtime rebuilds it from `instances` on load.
fn populate_dynamic_compound_instances(hkx: &mut HkxFile) {
    fn vec4_member(obj: &HkxObject, name: &str) -> Option<[f32; 4]> {
        obj.members
            .iter()
            .find(|m| m.name == name)
            .and_then(|m| match &m.value {
                HkxValue::F32List(f) if f.len() >= 4 => Some([f[0], f[1], f[2], f[3]]),
                _ => None,
            })
    }

    /// Child-local AABB (min, max) of a leaf collision shape, inflated by the
    /// `convexRadius` so it matches the radius the FO76 tree baked into the
    /// node AABBs. The radius lives in `convexRadius` post-conversion (FO4
    /// capsules carry `a.w`/`b.w` = 1.0), so reading `a.w` would be wrong.
    fn shape_local_aabb(obj: &HkxObject) -> Option<([f32; 3], [f32; 3])> {
        let convex_radius = obj
            .members
            .iter()
            .find(|m| m.name == "convexRadius")
            .and_then(|m| match m.value {
                HkxValue::F32(v) if v.is_finite() && v > 0.0 => Some(v),
                _ => None,
            })
            .unwrap_or(0.0);

        let (mut mn, mut mx) = if obj.class_name == "hknpCapsuleShape" {
            let a = vec4_member(obj, "a")?;
            let b = vec4_member(obj, "b")?;
            let mut mn = [0.0f32; 3];
            let mut mx = [0.0f32; 3];
            for i in 0..3 {
                mn[i] = a[i].min(b[i]);
                mx[i] = a[i].max(b[i]);
            }
            (mn, mx)
        } else {
            // Polytope / box / anything carrying an explicit vertices array.
            let m = obj.members.iter().find(|m| m.name == "vertices")?;
            let HkxValue::Array(items) = &m.value else {
                return None;
            };
            let mut mn = [f32::INFINITY; 3];
            let mut mx = [f32::NEG_INFINITY; 3];
            let mut any = false;
            for it in items {
                if let HkxValue::F32List(f) = it {
                    if f.len() >= 3 {
                        any = true;
                        for i in 0..3 {
                            mn[i] = mn[i].min(f[i]);
                            mx[i] = mx[i].max(f[i]);
                        }
                    }
                }
            }
            if !any {
                return None;
            }
            (mn, mx)
        };

        for i in 0..3 {
            mn[i] -= convex_radius;
            mx[i] += convex_radius;
        }
        Some((mn, mx))
    }

    /// (center, extent) for each non-null node in a compound's BVH tree.
    fn tree_node_boxes(data_obj: &HkxObject) -> Vec<([f32; 3], [f32; 3])> {
        fn min_max(members: &[HkxMember]) -> Option<([f32; 3], [f32; 3])> {
            // The node may carry `min`/`max` directly or nested under an `aabb`.
            let direct = |ms: &[HkxMember], name: &str| -> Option<[f32; 3]> {
                ms.iter()
                    .find(|m| m.name == name)
                    .and_then(|m| match &m.value {
                        HkxValue::F32List(f) if f.len() >= 3 => Some([f[0], f[1], f[2]]),
                        _ => None,
                    })
            };
            if let (Some(mn), Some(mx)) = (direct(members, "min"), direct(members, "max")) {
                return Some((mn, mx));
            }
            let aabb = members.iter().find(|m| m.name == "aabb")?;
            let ams = aabb.value.as_object_members()?;
            Some((direct(ams, "min")?, direct(ams, "max")?))
        }

        let Some(tree) = data_obj.members.iter().find(|m| m.name == "aabbTree") else {
            return Vec::new();
        };
        let Some(tree_ms) = tree.value.as_object_members() else {
            return Vec::new();
        };
        let Some(nodes) = tree_ms.iter().find(|m| m.name == "nodes") else {
            return Vec::new();
        };
        let HkxValue::Array(node_items) = &nodes.value else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for node in node_items {
            let Some(nms) = node.as_object_members() else {
                continue;
            };
            let Some((mn, mx)) = min_max(nms) else {
                continue;
            };
            // Skip the null sentinel node (all-zero AABB).
            if mn.iter().all(|&x| x == 0.0) && mx.iter().all(|&x| x == 0.0) {
                continue;
            }
            let center = [
                (mn[0] + mx[0]) * 0.5,
                (mn[1] + mx[1]) * 0.5,
                (mn[2] + mx[2]) * 0.5,
            ];
            let extent = [mx[0] - mn[0], mx[1] - mn[1], mx[2] - mn[2]];
            out.push((center, extent));
        }
        out
    }

    fn build_instance(shape_idx: usize, translation: [f32; 3]) -> HkxValue {
        let transform = vec![
            1.0,
            0.0,
            0.0,
            translation[0], //
            0.0,
            1.0,
            0.0,
            translation[1], //
            0.0,
            0.0,
            1.0,
            translation[2], //
            0.0,
            0.0,
            0.0,
            1.0,
        ];
        HkxValue::Object(vec![
            HkxMember {
                name: "transform".to_string(),
                value: HkxValue::F32List(transform),
            },
            HkxMember {
                name: "scale".to_string(),
                value: HkxValue::F32List(vec![1.0, 1.0, 1.0, 1.0]),
            },
            HkxMember {
                name: "shape".to_string(),
                value: HkxValue::Pointer(Some(shape_idx)),
            },
            HkxMember {
                name: "shapeTag".to_string(),
                value: HkxValue::U16(65535),
            },
            HkxMember {
                name: "destructionTag".to_string(),
                value: HkxValue::U16(65535),
            },
            HkxMember {
                name: "padding".to_string(),
                value: HkxValue::Array(vec![HkxValue::U8(0); 30]),
            },
        ])
    }

    const LEAF_CLASSES: &[&str] = &[
        "hknpCapsuleShape",
        "hknpConvexPolytopeShape",
        "hknpSphereShape",
        "hknpBoxShape",
    ];

    let objects = hkx.objects();

    // Shapes referenced directly by a body — never compound children.
    let mut body_shapes: Vec<usize> = Vec::new();
    for obj in objects {
        if !matches!(
            obj.class_name.as_str(),
            "hknpRagdollData" | "hknpPhysicsSystemData"
        ) {
            continue;
        }
        if let Some(bc) = obj.members.iter().find(|m| m.name == "bodyCinfos") {
            if let HkxValue::Array(bodies) = &bc.value {
                for body in bodies {
                    if let Some(ms) = body.as_object_members() {
                        for mm in ms {
                            if mm.name == "shape" {
                                if let HkxValue::Pointer(Some(idx)) = mm.value {
                                    body_shapes.push(idx);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Plan instance arrays without mutating, then apply (borrow discipline).
    let mut plan: Vec<(usize, Vec<HkxValue>)> = Vec::new();
    for (ci, compound) in objects.iter().enumerate() {
        if compound.class_name != "hknpDynamicCompoundShape" {
            continue;
        }
        let instances_empty = compound
            .members
            .iter()
            .find(|m| m.name == "instances")
            .and_then(|m| m.value.as_object_members())
            .and_then(|ms| ms.iter().find(|m| m.name == "elements"))
            .map(|m| matches!(&m.value, HkxValue::Array(a) if a.is_empty()))
            .unwrap_or(false);
        if !instances_empty {
            continue;
        }

        let Some(data_idx) = compound
            .members
            .iter()
            .find(|m| m.name == "boundingVolumeData")
            .and_then(|m| match m.value {
                HkxValue::Pointer(Some(i)) => Some(i),
                _ => None,
            })
        else {
            continue;
        };
        let nodes = tree_node_boxes(&objects[data_idx]);
        if nodes.is_empty() {
            continue;
        }

        // Orphan leaf shapes: leaf class, not this compound, not body-referenced.
        let mut children: Vec<(usize, [f32; 3], [f32; 3])> = Vec::new();
        for (si, s) in objects.iter().enumerate() {
            if si == ci || body_shapes.contains(&si) {
                continue;
            }
            if !LEAF_CLASSES.contains(&s.class_name.as_str()) {
                continue;
            }
            if let Some((mn, mx)) = shape_local_aabb(s) {
                let center = [
                    (mn[0] + mx[0]) * 0.5,
                    (mn[1] + mx[1]) * 0.5,
                    (mn[2] + mx[2]) * 0.5,
                ];
                let extent = [mx[0] - mn[0], mx[1] - mn[1], mx[2] - mn[2]];
                children.push((si, center, extent));
            }
        }
        if children.is_empty() {
            continue;
        }

        // Match each child to the closest-extent unused tree node (the leaves),
        // recovering its body-space translation from that node's center.
        let mut node_used = vec![false; nodes.len()];
        let mut instances = Vec::new();
        for (cidx, ccenter, cextent) in &children {
            let mut best: Option<usize> = None;
            let mut best_d = f32::INFINITY;
            for (ni, (_, nextent)) in nodes.iter().enumerate() {
                if node_used[ni] {
                    continue;
                }
                let d = (cextent[0] - nextent[0]).abs()
                    + (cextent[1] - nextent[1]).abs()
                    + (cextent[2] - nextent[2]).abs();
                if d < best_d {
                    best_d = d;
                    best = Some(ni);
                }
            }
            let Some(ni) = best else {
                break;
            };
            node_used[ni] = true;
            let (ncenter, _) = nodes[ni];
            let translation = [
                ncenter[0] - ccenter[0],
                ncenter[1] - ccenter[1],
                ncenter[2] - ccenter[2],
            ];
            instances.push(build_instance(*cidx, translation));
        }
        if !instances.is_empty() {
            plan.push((ci, instances));
        }
    }

    for (ci, instances) in plan {
        let compound = &mut hkx.objects_mut()[ci];
        if let Some(inst) = compound.members.iter_mut().find(|m| m.name == "instances") {
            if let Some(ms) = inst.value.as_object_members_mut() {
                if let Some(el) = ms.iter_mut().find(|m| m.name == "elements") {
                    el.value = HkxValue::Array(instances);
                }
            }
        }
    }
}

/// FO4 cannot instantiate a ragdoll body whose shape is an `hknpDynamicCompoundShape`.
/// The FO76 compound serialization is tolerated by the FO76 runtime but rejected by
/// FO4's body instantiation, so the body is never created; its `BodyID` then falls
/// outside the ragdoll's `bodyIdToIndexMap` and `bhkNPCollisionObject::AddToWorld`
/// dereferences a null body → CTD when the actor's 3D loads (e.g. placing the FO76
/// AntiAir turret crashes the CK). `flatten_compound_shapes_in_psd` deliberately
/// skips ragdolls (one-body-per-leaf would destroy the bone→body map) and
/// `wrap_capsules_in_compound_shape` is disabled, so nothing else covers this case.
///
/// Repoint each compound-backed ragdoll body to the first leaf shape its compound
/// instances reference. Keeps exactly one body per bone (bone map intact) and hands
/// FO4 a plain convex/capsule it can instantiate. Collision on that body collapses
/// to the leaf shape — acceptable next to a hard crash.
fn simplify_compound_ragdoll_body_shapes(hkx: &mut HkxFile) {
    use std::collections::HashMap;
    // compound object index -> its instance leaf shape pointers (instance order)
    let mut compound_leaves: HashMap<usize, Vec<usize>> = HashMap::new();
    for (ci, o) in hkx.objects().iter().enumerate() {
        if o.class_name != "hknpDynamicCompoundShape" {
            continue;
        }
        let mut leaves = Vec::new();
        if let Some(inst) = o.members.iter().find(|m| m.name == "instances") {
            if let Some(ms) = inst.value.as_object_members() {
                if let Some(el) = ms.iter().find(|m| m.name == "elements") {
                    if let HkxValue::Array(items) = &el.value {
                        for it in items {
                            if let Some(ims) = it.as_object_members() {
                                for im in ims {
                                    if im.name == "shape" {
                                        if let HkxValue::Pointer(Some(s)) = im.value {
                                            leaves.push(s);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        if !leaves.is_empty() {
            compound_leaves.insert(ci, leaves);
        }
    }
    if compound_leaves.is_empty() {
        return;
    }

    // (ragdoll obj idx, body idx) -> replacement leaf shape idx
    let mut plan: Vec<(usize, usize, usize)> = Vec::new();
    for (oi, o) in hkx.objects().iter().enumerate() {
        if o.class_name != "hknpRagdollData" {
            continue;
        }
        let Some(bc) = o.members.iter().find(|m| m.name == "bodyCinfos") else {
            continue;
        };
        let HkxValue::Array(bodies) = &bc.value else {
            continue;
        };
        for (bi, body) in bodies.iter().enumerate() {
            let Some(ms) = body.as_object_members() else {
                continue;
            };
            for mm in ms {
                if mm.name != "shape" {
                    continue;
                }
                if let HkxValue::Pointer(Some(sidx)) = mm.value {
                    if let Some(leaf) = compound_leaves.get(&sidx).and_then(|l| l.first()) {
                        plan.push((oi, bi, *leaf));
                    }
                }
            }
        }
    }

    for (oi, bi, leaf) in plan {
        let o = &mut hkx.objects_mut()[oi];
        if let Some(bc) = o.members.iter_mut().find(|m| m.name == "bodyCinfos") {
            if let HkxValue::Array(bodies) = &mut bc.value {
                if let Some(body) = bodies.get_mut(bi) {
                    if let Some(ms) = body.as_object_members_mut() {
                        for mm in ms.iter_mut() {
                            if mm.name == "shape" {
                                mm.value = HkxValue::Pointer(Some(leaf));
                            }
                        }
                    }
                }
            }
        }
    }
}

fn is_character_controller_body(members: &[HkxMember]) -> bool {
    members
        .iter()
        .find(|member| member.name == "name")
        .and_then(|member| match &member.value {
            HkxValue::String { value, .. } => Some(value.as_str()),
            _ => None,
        })
        .map(|name| {
            name.eq_ignore_ascii_case("CharacterBumper")
                || name.eq_ignore_ascii_case("CharacterController")
        })
        .unwrap_or(false)
}

/// FO4 loads controller physics from `skeleton.nif`; duplicate trailing bodies in
/// the external ragdoll can make actor initialization register them twice.
fn strip_unmapped_trailing_ragdoll_controller_bodies(hkx: &mut HkxFile) {
    use std::collections::HashSet;

    for object in hkx.objects_mut() {
        if object.class_name != "hknpRagdollData" {
            continue;
        }

        let mapped_bodies: HashSet<usize> = object
            .members
            .iter()
            .find(|member| member.name == "boneToBodyMap")
            .and_then(|member| match &member.value {
                HkxValue::Array(values) => Some(values),
                _ => None,
            })
            .into_iter()
            .flatten()
            .filter_map(extract_int)
            .filter_map(|body_index| usize::try_from(body_index).ok())
            .collect();
        let constrained_bodies: HashSet<usize> = object
            .members
            .iter()
            .find(|member| member.name == "constraintCinfos")
            .and_then(|member| match &member.value {
                HkxValue::Array(values) => Some(values),
                _ => None,
            })
            .into_iter()
            .flatten()
            .flat_map(|constraint| constraint.as_object_members().into_iter().flatten())
            .filter(|member| member.name == "bodyA" || member.name == "bodyB")
            .filter_map(|member| extract_int(&member.value))
            .filter_map(|body_index| usize::try_from(body_index).ok())
            .collect();

        let Some(HkxValue::Array(bodies)) = object
            .members
            .iter()
            .find(|member| member.name == "bodyCinfos")
            .map(|member| &member.value)
        else {
            continue;
        };

        let original_body_count = bodies.len();
        let mut kept_body_count = original_body_count;
        while kept_body_count > 0 {
            let body_index = kept_body_count - 1;
            if mapped_bodies.contains(&body_index) || constrained_bodies.contains(&body_index) {
                break;
            }
            let Some(body_members) = bodies[body_index].as_object_members() else {
                break;
            };
            if !is_character_controller_body(body_members) {
                break;
            }
            kept_body_count -= 1;
        }

        if kept_body_count == original_body_count {
            continue;
        }

        for member in &mut object.members {
            if member.name == "bodyCinfos" {
                if let HkxValue::Array(values) = &mut member.value {
                    values.truncate(kept_body_count);
                }
            } else if member.name == "motionCinfos" {
                if let HkxValue::Array(values) = &mut member.value {
                    if values.len() == original_body_count {
                        values.truncate(kept_body_count);
                    }
                }
            }
        }
    }
}

/// Collect all valid pointer targets from an array of inline objects, from any
/// member whose name matches `field_name`. Used by `fix_physics_referenced_objects`.
fn collect_pointers_from_cinfo_array(
    array_value: &HkxValue,
    field_name: &str,
    object_count: usize,
) -> Vec<usize> {
    let HkxValue::Array(items) = array_value else {
        return vec![];
    };
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        if let HkxValue::Object(members) = item {
            for m in members {
                if m.name == field_name {
                    if let HkxValue::Pointer(Some(idx)) = m.value {
                        if idx < object_count {
                            out.push(idx);
                        }
                    }
                }
            }
        }
    }
    out
}

/// Rebuild the `referencedObjects` array on `hknpPhysicsSystemData` by unioning
/// all pointer targets reachable from `bodyCinfos[*].shape`,
/// `motionCinfos[*].massDistribution` (or other pointer fields),
/// and `constraintCinfos[*].data` — preserving any entry not contradicted by
/// the rebuilt set. This prevents constraint objects that are only referenced via
/// `constraintCinfos` from being orphaned by a downstream reachability sweep.
fn fix_physics_referenced_objects(hkx: &mut HkxFile) {
    let object_count = hkx.objects().len();
    let psd_indices: Vec<usize> = (0..object_count)
        .filter(|&i| hkx.objects()[i].class_name == "hknpPhysicsSystemData")
        .collect();

    for psd_idx in psd_indices {
        // Collect pointer targets from bodyCinfos, motionCinfos, and constraintCinfos.
        let (body_shapes, constraint_data) = {
            let psd = &hkx.objects()[psd_idx];

            let bodies_val = psd
                .members
                .iter()
                .find(|m| m.name == "bodyCinfos")
                .map(|m| m.value.clone())
                .unwrap_or(HkxValue::Array(vec![]));
            let body_shapes = collect_pointers_from_cinfo_array(&bodies_val, "shape", object_count);

            let constraint_val = psd
                .members
                .iter()
                .find(|m| m.name == "constraintCinfos")
                .map(|m| m.value.clone())
                .unwrap_or(HkxValue::Array(vec![]));
            // constraintCinfos entries carry a `data` pointer to the constraint object
            let constraint_data =
                collect_pointers_from_cinfo_array(&constraint_val, "data", object_count);

            (body_shapes, constraint_data)
        };

        // Union all collected targets, deduplicating while preserving order:
        // body shapes first, then constraint data pointers.
        let mut seen = std::collections::HashSet::new();
        let mut all_targets: Vec<usize> = Vec::new();
        for &idx in body_shapes.iter().chain(constraint_data.iter()) {
            if seen.insert(idx) {
                all_targets.push(idx);
            }
        }

        // Rewrite referencedObjects.
        let psd = &mut hkx.objects_mut()[psd_idx];
        if let Some(refs_member) = psd
            .members
            .iter_mut()
            .find(|m| m.name == "referencedObjects")
        {
            refs_member.value = HkxValue::Array(
                all_targets
                    .iter()
                    .map(|&i| HkxValue::Pointer(Some(i)))
                    .collect(),
            );
        }
    }

    let ragdoll_indices: Vec<usize> = (0..object_count)
        .filter(|&i| hkx.objects()[i].class_name == "hknpRagdollData")
        .collect();

    for ragdoll_idx in ragdoll_indices {
        let body_shapes = {
            let ragdoll = &hkx.objects()[ragdoll_idx];
            let bodies_val = ragdoll
                .members
                .iter()
                .find(|m| m.name == "bodyCinfos")
                .map(|m| m.value.clone())
                .unwrap_or(HkxValue::Array(vec![]));
            collect_pointers_from_cinfo_array(&bodies_val, "shape", object_count)
        };

        let mut all_targets: Vec<usize> = body_shapes
            .into_iter()
            .filter(|&idx| {
                hkx.objects()
                    .get(idx)
                    .map(|o| {
                        matches!(
                            o.class_name.as_str(),
                            "hknpCapsuleShape" | "hknpSphereShape" | "hknpConvexPolytopeShape"
                        )
                    })
                    .unwrap_or(false)
            })
            .collect();
        all_targets.sort_unstable();
        all_targets.dedup();

        let ragdoll = &mut hkx.objects_mut()[ragdoll_idx];
        if let Some(refs_member) = ragdoll
            .members
            .iter_mut()
            .find(|m| m.name == "referencedObjects")
        {
            refs_member.value = HkxValue::Array(
                all_targets
                    .iter()
                    .map(|&i| HkxValue::Pointer(Some(i)))
                    .collect(),
            );
        }
    }
}

/// True iff `object.motionCinfos[motion_id].inverseMass` resolves to a finite,
/// strictly positive value — i.e. a real dynamic motion (as wired by the
/// compound→PSD migration). Infinite mass (`inverseMass == 0`), a missing
/// cinfo, or a non-numeric value all read as "no dynamic motion".
fn motion_index_has_dynamic_motion(object: &HkxObject, motion_id: u32) -> bool {
    let Some(m) = object.members.iter().find(|m| m.name == "motionCinfos") else {
        return false;
    };
    let HkxValue::Array(cinfos) = &m.value else {
        return false;
    };
    let Some(cinfo) = cinfos.get(motion_id as usize) else {
        return false;
    };
    // Production motionCinfos are TypedObject ("hknpMotionCinfo"); accept both
    // Object and TypedObject so the guard fires on real migrated weapons.
    let Some(members) = cinfo.as_object_members() else {
        return false;
    };
    let Some(im) = members.iter().find(|m| m.name == "inverseMass") else {
        return false;
    };
    let inv_mass = match &im.value {
        HkxValue::F32(v) | HkxValue::Half(v) => *v,
        _ => return false,
    };
    inv_mass.is_finite() && inv_mass > 0.0
}

/// Fix three FO76→FO4 schema gaps in `hknpPhysicsSystemData.bodyCinfos`:
/// 1. `motionId`: 0 → 0x7FFFFFFF (Havok INVALID_ID, "skip motion lookup")
/// 2. `reservedBodyId`: nested struct or 0 → 0x7FFFFFFF
/// 3. `orientation`: scalar/missing → identity quaternion (0,0,0,1)
fn normalize_bumper_body_cinfos(hkx: &mut HkxFile) {
    const INVALID_ID: i64 = 2_147_483_647; // 0x7FFFFFFF
    let identity_quat: Vec<f32> = vec![0.0, 0.0, 0.0, 1.0];

    for object in hkx.objects_mut() {
        if object.class_name != "hknpPhysicsSystemData" {
            continue;
        }
        // The compound→PSD migrate pass wires body0's motionId=0 to a real
        // dynamic motionCinfos[0]. Resolve that BEFORE the &mut bodyCinfos
        // borrow so the read doesn't conflict with the mutable iteration.
        let motion0_is_dynamic = motion_index_has_dynamic_motion(object, 0);
        let Some(body_member) = object.members.iter_mut().find(|m| m.name == "bodyCinfos") else {
            continue;
        };
        let HkxValue::Array(bodies) = &mut body_member.value else {
            continue;
        };
        for body in bodies.iter_mut() {
            let HkxValue::Object(body_members) = body else {
                continue;
            };

            // motionId: 0 → INVALID_ID, unless it points at a real dynamic
            // motion (wired by the compound→PSD migration) — clobbering that
            // would leave a dynamic shape with an INVALID motion id.
            if let Some(m) = body_members.iter_mut().find(|m| m.name == "motionId") {
                let cur = extract_int(&m.value).unwrap_or(0) as i64;
                if cur == 0 && !motion0_is_dynamic {
                    m.value = HkxValue::U32(INVALID_ID as u32);
                }
            } else {
                body_members.push(HkxMember {
                    name: "motionId".to_string(),
                    value: HkxValue::U32(INVALID_ID as u32),
                });
            }

            // reservedBodyId: replace nested-struct or zero scalar with INVALID_ID.
            if let Some(m) = body_members.iter_mut().find(|m| m.name == "reservedBodyId") {
                let needs_fix = match &m.value {
                    HkxValue::Object(_) => true,
                    other => extract_int(other).unwrap_or(0) == 0,
                };
                if needs_fix {
                    m.value = HkxValue::U32(INVALID_ID as u32);
                }
            }

            // orientation: must be a 4-element F32List with non-zero norm.
            if let Some(m) = body_members.iter_mut().find(|m| m.name == "orientation") {
                let needs_identity = match &m.value {
                    HkxValue::F32List(values) if values.len() == 4 => {
                        let norm_sq = values
                            .iter()
                            .map(|v| (v as &f32) * (v as &f32))
                            .sum::<f32>();
                        norm_sq < 1e-10
                    }
                    _ => true,
                };
                if needs_identity {
                    m.value = HkxValue::F32List(identity_quat.clone());
                }
            }
        }
    }
}

/// Fix FO76→FO4 schema gaps in `hknpRagdollData.bodyCinfos`:
/// 1. `motionId`: assign sequential IDs to dynamic ragdoll bodies. Retained
///    controller/bumper bodies stay in `bodyCinfos` as static bodies with
///    `motionId = INVALID_ID`, matching native FO4 creature ragdolls.
/// 2. `reservedBodyId`: set to 0x7FFFFFFF (INVALID_ID).
/// 3. `orientation`: preserve a valid authored body transform. Only recover a
///    missing or malformed value from `hkaSkeleton.referencePose[bone]`, falling
///    back to identity `(0,0,0,1)` when no skeleton data is available.
/// 4. `collisionFilterInfo` on body[0]: if < 256 (no group bit), set to 520
///    (layer 8 + bit 9) so FO4's ragdoll linker can register it as the root body.
fn normalize_ragdoll_body_cinfos(hkx: &mut HkxFile) {
    const INVALID_ID: u32 = 0x7FFF_FFFF;
    let identity_quat: Vec<f32> = vec![0.0, 0.0, 0.0, 1.0];

    // Build skeleton name → per-bone rotation lookup from hkaSkeleton.referencePose.
    // referencePose layout per hkQsTransformf: 12 floats; rotation at indices 4..7.
    let mut skel_rots_by_name: std::collections::HashMap<String, Vec<Vec<f32>>> =
        std::collections::HashMap::new();
    for obj in hkx.objects() {
        if obj.class_name != "hkaSkeleton" {
            continue;
        }
        let skel_name = obj.name.clone().unwrap_or_default();
        let mut rotations: Vec<Vec<f32>> = Vec::new();
        for m in &obj.members {
            if m.name != "referencePose" {
                continue;
            }
            let HkxValue::Array(entries) = &m.value else {
                continue;
            };
            for entry in entries {
                let rot = if let HkxValue::F32List(vals) = entry {
                    if vals.len() >= 8 {
                        vec![vals[4], vals[5], vals[6], vals[7]]
                    } else {
                        identity_quat.clone()
                    }
                } else {
                    identity_quat.clone()
                };
                rotations.push(rot);
            }
            break;
        }
        skel_rots_by_name.insert(skel_name, rotations);
    }

    // Pre-pass: for each hknpRagdollData, resolve the skeleton target name and
    // boneToBodyMap from the read-only object slice, avoiding borrow conflicts.
    struct RagdollInfo {
        obj_idx: usize,
        skel_target: String,
        body_to_bone: std::collections::HashMap<usize, usize>,
    }
    let ragdoll_infos: Vec<RagdollInfo> = hkx
        .objects()
        .iter()
        .enumerate()
        .filter(|(_, obj)| obj.class_name == "hknpRagdollData")
        .map(|(obj_idx, obj)| {
            let mut skel_target = String::new();
            let mut bone_to_body: Vec<i32> = Vec::new();
            for m in &obj.members {
                if m.name == "skeleton" {
                    if let HkxValue::Pointer(Some(skel_idx)) = m.value {
                        if let Some(skel_obj) = hkx.objects().get(skel_idx) {
                            skel_target = skel_obj.name.clone().unwrap_or_default();
                        }
                    }
                } else if m.name == "boneToBodyMap" {
                    if let HkxValue::Array(entries) = &m.value {
                        for e in entries {
                            bone_to_body.push(extract_int(e).unwrap_or(-1));
                        }
                    }
                }
            }
            let mut body_to_bone: std::collections::HashMap<usize, usize> =
                std::collections::HashMap::new();
            for (bone_idx, &body_idx) in bone_to_body.iter().enumerate() {
                if body_idx >= 0 {
                    body_to_bone.entry(body_idx as usize).or_insert(bone_idx);
                }
            }
            RagdollInfo {
                obj_idx,
                skel_target,
                body_to_bone,
            }
        })
        .collect();

    for info in &ragdoll_infos {
        let pose_rots = skel_rots_by_name.get(&info.skel_target);
        let object = &mut hkx.objects_mut()[info.obj_idx];

        let Some(body_member) = object.members.iter_mut().find(|m| m.name == "bodyCinfos") else {
            continue;
        };
        let HkxValue::Array(bodies) = &mut body_member.value else {
            continue;
        };

        let mut next_motion_id = 0_u32;
        for (body_idx, body) in bodies.iter_mut().enumerate() {
            let HkxValue::Object(body_members) = body else {
                continue;
            };

            let is_controller = is_character_controller_body(body_members);
            let motion_id = if is_controller {
                INVALID_ID
            } else {
                let motion_id = next_motion_id;
                next_motion_id += 1;
                motion_id
            };
            if let Some(m) = body_members.iter_mut().find(|m| m.name == "motionId") {
                m.value = HkxValue::U32(motion_id);
            } else {
                body_members.push(HkxMember {
                    name: "motionId".to_string(),
                    value: HkxValue::U32(motion_id),
                });
            }

            if is_controller {
                if let Some(m) = body_members.iter_mut().find(|m| m.name == "flags") {
                    m.value = HkxValue::U32(1);
                } else {
                    body_members.push(HkxMember {
                        name: "flags".to_string(),
                        value: HkxValue::U32(1),
                    });
                }
            }

            // reservedBodyId: always INVALID_ID.
            if let Some(m) = body_members.iter_mut().find(|m| m.name == "reservedBodyId") {
                m.value = HkxValue::U32(INVALID_ID);
            } else {
                body_members.push(HkxMember {
                    name: "reservedBodyId".to_string(),
                    value: HkxValue::U32(INVALID_ID),
                });
            }

            // orientation: preserve authored body transforms. A skeleton's local
            // reference-pose rotation is only a fallback for missing/bad source data.
            let bone_idx = info
                .body_to_bone
                .get(&body_idx)
                .copied()
                .unwrap_or(body_idx);
            let pose_target_rot = pose_rots
                .and_then(|rots| rots.get(bone_idx))
                .filter(|rot| rot.len() == 4 && rot.iter().map(|v| v * v).sum::<f32>() > 1e-6)
                .cloned();
            let target_rot = pose_target_rot
                .clone()
                .unwrap_or_else(|| identity_quat.clone());

            if let Some(m) = body_members.iter_mut().find(|m| m.name == "orientation") {
                let needs_fix = match &m.value {
                    HkxValue::F32List(v) if v.len() == 4 => {
                        !v.iter().all(|x| x.is_finite())
                            || v.iter().map(|x| x * x).sum::<f32>() < 1e-10
                    }
                    _ => true,
                };
                if needs_fix {
                    m.value = HkxValue::F32List(target_rot);
                }
            } else {
                body_members.push(HkxMember {
                    name: "orientation".to_string(),
                    value: HkxValue::F32List(target_rot),
                });
            }

            // collisionFilterInfo on body[0]: ensure group bit (>=256) is set.
            if body_idx == 0 {
                if let Some(m) = body_members
                    .iter_mut()
                    .find(|m| m.name == "collisionFilterInfo")
                {
                    if let Some(cfi) = extract_int(&m.value) {
                        if (cfi as u32) < 256 {
                            m.value = HkxValue::U32(520);
                        }
                    }
                }
            }
        }
    }
}

/// FO4 ragdoll body transforms require a homogeneous position lane of zero.
/// Some FO76 ragdolls retain nonzero data there on sphere and polytope bodies;
/// FO4's SIMD broadphase consumes it while building body bounds and can produce
/// invalid closest-point queries. Run after motion synthesis so the source lane
/// remains available to `centerOfMassWorld`, matching native FO4 motion cinfos.
fn normalize_ragdoll_body_position_w(hkx: &mut HkxFile) {
    for object in hkx.objects_mut() {
        if object.class_name != "hknpRagdollData" {
            continue;
        }
        let Some(body_member) = object.members.iter_mut().find(|m| m.name == "bodyCinfos") else {
            continue;
        };
        let HkxValue::Array(bodies) = &mut body_member.value else {
            continue;
        };
        for body in bodies {
            let Some(body_members) = body.as_object_members_mut() else {
                continue;
            };
            let Some(position) = body_members.iter_mut().find(|m| m.name == "position") else {
                continue;
            };
            if let HkxValue::F32List(values) = &mut position.value {
                if values.len() >= 4 {
                    values[3] = 0.0;
                }
            }
        }
    }
}

/// FO76 serializes the cone atom's runtime angle-offset displacement for its
/// larger constraint layout. FO4 follows this byte offset while activating the
/// solver, so retain the authored limits but stamp the FO4 layout displacement.
fn normalize_ragdoll_constraint_offsets(hkx: &mut HkxFile) {
    const FO4_CONE_ANGLE_OFFSET: i32 = 56;

    for object in hkx.objects_mut() {
        if object.class_name != "hkpRagdollConstraintData" {
            continue;
        }
        let Some(atoms) = object
            .members
            .iter_mut()
            .find(|member| member.name == "atoms")
            .and_then(|member| member.value.as_object_members_mut())
        else {
            continue;
        };
        let Some(cone_limit) = atoms
            .iter_mut()
            .find(|member| member.name == "coneLimit")
            .and_then(|member| member.value.as_object_members_mut())
        else {
            continue;
        };
        if let Some(offset) = cone_limit
            .iter_mut()
            .find(|member| member.name == "memOffsetToAngleOffset")
        {
            set_int_member(&mut offset.value, FO4_CONE_ANGLE_OFFSET);
        }
    }
}

/// FO76 `hkpRagdollConstraintData` constraints have null motor pointers, which
/// FO4 dereferences on ragdoll activation. Wire a shared
/// `hkpPositionConstraintMotor` into each constraint's `ragdollMotors` atom.
///
/// Motor values (`maxForce=100`, `tau=0.8`, `damping=1.0`, ...) come from the FO4
/// Snallygaster ragdoll and are used for every FO76 ragdoll; an existing
/// `hkpPositionConstraintMotor` in the file is reused verbatim. Large creatures
/// (e.g. Deathclaw mass class) would need mass-scaled `tau`/`damping`.
fn inject_ragdoll_motors(hkx: &mut HkxFile) {
    let has_constraints = hkx
        .objects()
        .iter()
        .any(|o| o.class_name == "hkpRagdollConstraintData");
    if !has_constraints {
        return;
    }

    // Tested per constraint, not per file: a file where only some constraints
    // carry motors still leaves the rest null for FO4 to dereference.
    let has_motors = |obj: &HkxObject| -> bool {
        obj.members.iter().any(|m| {
            if m.name != "atoms" {
                return false;
            }
            let Some(atoms_members) = m.value.as_object_members() else {
                return false;
            };
            atoms_members.iter().any(|am| {
                if am.name != "ragdollMotors" {
                    return false;
                }
                let Some(rm_members) = am.value.as_object_members() else {
                    return false;
                };
                rm_members.iter().any(|rm| {
                    rm.name == "motors"
                        && match &rm.value {
                            HkxValue::Pointer(Some(_)) => true,
                            HkxValue::Array(values) => values
                                .iter()
                                .any(|v| matches!(v, HkxValue::Pointer(Some(_)))),
                            _ => false,
                        }
                })
            })
        })
    };
    let all_populated = hkx
        .objects()
        .iter()
        .filter(|o| o.class_name == "hkpRagdollConstraintData")
        .all(has_motors);
    if all_populated {
        return;
    }

    // Reuse existing hkpPositionConstraintMotor if present, else create one.
    let motor_idx = hkx
        .objects()
        .iter()
        .position(|o| o.class_name == "hkpPositionConstraintMotor")
        .unwrap_or_else(|| {
            hkx.push_object(HkxObject {
                name: Some("#motor_injected".to_string()),
                offset: 0,
                signature: 0,
                class_name: "hkpPositionConstraintMotor".to_string(),
                members: vec![
                    HkxMember {
                        name: "type".to_string(),
                        value: HkxValue::I32(3),
                    }, // TYPE_POSITION
                    HkxMember {
                        name: "minForce".to_string(),
                        value: HkxValue::F32(-1_000_000.0),
                    },
                    HkxMember {
                        name: "maxForce".to_string(),
                        value: HkxValue::F32(100.0),
                    },
                    HkxMember {
                        name: "tau".to_string(),
                        value: HkxValue::F32(0.8),
                    },
                    HkxMember {
                        name: "damping".to_string(),
                        value: HkxValue::F32(1.0),
                    },
                    HkxMember {
                        name: "proportionalRecoveryVelocity".to_string(),
                        value: HkxValue::F32(5.0),
                    },
                    HkxMember {
                        name: "constantRecoveryVelocity".to_string(),
                        value: HkxValue::F32(0.2),
                    },
                ],
            })
        });

    for obj in hkx.objects_mut() {
        if obj.class_name != "hkpRagdollConstraintData" || has_motors(obj) {
            continue;
        }
        for m in &mut obj.members {
            if m.name != "atoms" {
                continue;
            }
            let Some(atoms_members) = m.value.as_object_members_mut() else {
                continue;
            };
            for am in atoms_members.iter_mut() {
                if am.name != "ragdollMotors" {
                    continue;
                }
                let Some(rm_members) = am.value.as_object_members_mut() else {
                    continue;
                };
                for rm in rm_members.iter_mut() {
                    if rm.name == "motors" {
                        rm.value = HkxValue::Array(vec![
                            HkxValue::Pointer(Some(motor_idx)),
                            HkxValue::Pointer(Some(motor_idx)),
                            HkxValue::Pointer(Some(motor_idx)),
                        ]);
                    }
                }
            }
            break;
        }
    }
}

/// Populate empty `motionCinfos` arrays from `bodyCinfos` for both
/// `hknpRagdollData` and `hknpPhysicsSystemData`. FO76 omits `motionCinfos`;
/// FO4 crashes on indexed access into an empty array during ragdoll init.
///
/// For `hknpPhysicsSystemData`: only fires when at least one body has
/// `flags == 128` (dynamic body). Bumper bodies (flags=16) intentionally
/// keep empty `motionCinfos` in vanilla FO4.
///
/// Inertia derivation: reads `hknpRefMassDistribution` data linked via each
/// body's `massDistribution` pointer, rotates its body-space COM into world,
/// emits `mass / volume` as FO4's mass factor, and scales unit-mass inertia by
/// the body's inverse mass.
fn synthesize_motion_cinfos(
    hkx: &mut HkxFile,
    shape_mass_cache: &std::collections::HashMap<String, ShapeMassInfo>,
) {
    const FLT_CAP: f32 = 1.844_672_6e19;
    const BODY_FLAGS_DYNAMIC: i32 = 128;

    // Build name→index map for mass distribution object lookups.
    let obj_by_name: std::collections::HashMap<String, usize> = hkx
        .objects()
        .iter()
        .enumerate()
        .filter_map(|(i, o)| o.name.as_ref().map(|n| (n.clone(), i)))
        .collect();

    // Read-only pass: per-body mass distribution COM, inertia and majorAxisSpace.
    //
    // Source layout (hknpRefMassDistribution.massDistribution per the 2018 SDK):
    //   centerOfMassAndVolume: vec4 (xyz = COM in body space, w = volume)
    //   inertiaTensor:         vec4 (diagonalized; w unused)
    //   majorAxisSpace:        vec4 (quaternion xyzw, body←major-axis rotation)
    //
    // The source inertia is already diagonal; `diagonalize_inertia` returns
    // (diag, identity) for it and also handles a full 3×3.
    fn resolve_mass_dist(
        objects: &[HkxObject],
        obj_by_name: &std::collections::HashMap<String, usize>,
        target_name: &str,
    ) -> (Option<Vec<f32>>, Option<Vec<f32>>, Option<Vec<f32>>) {
        let idx = match obj_by_name.get(target_name) {
            Some(&i) => i,
            None => return (None, None, None),
        };
        let obj = &objects[idx];
        let mut com: Option<Vec<f32>> = None;
        let mut inertia: Option<Vec<f32>> = None;
        let mut major_axis: Option<Vec<f32>> = None;

        fn walk(
            members: &[HkxMember],
            com: &mut Option<Vec<f32>>,
            inertia: &mut Option<Vec<f32>>,
            major_axis: &mut Option<Vec<f32>>,
        ) {
            for m in members {
                match m.name.as_str() {
                    "centerOfMassAndVolume" | "centerOfMass" => {
                        if let HkxValue::F32List(v) = &m.value {
                            if v.len() >= 4 {
                                *com = Some(v[..4].to_vec());
                            }
                        }
                    }
                    "inertiaTensor" => {
                        if let HkxValue::F32List(v) = &m.value {
                            if v.len() >= 4 {
                                *inertia = Some(v[..4].to_vec());
                            }
                        }
                    }
                    "majorAxisSpace" => {
                        if let HkxValue::F32List(v) = &m.value {
                            if v.len() == 4 {
                                *major_axis = Some(v.clone());
                            }
                        }
                    }
                    _ => {
                        if let Some(inner) = m.value.as_object_members() {
                            walk(inner, com, inertia, major_axis);
                        }
                    }
                }
            }
        }

        walk(&obj.members, &mut com, &mut inertia, &mut major_axis);
        (com, inertia, major_axis)
    }

    // Collect each container's new motionCinfos first (mass-dist lookup needs
    // immutable access to hkx.objects()), then insert them.
    let n_objects = hkx.objects().len();
    let mut insertions: Vec<(usize, usize, Vec<HkxMember>)> = Vec::new(); // (obj_idx, insert_pos, new_motion_infos)
    let default_inverse_inertia_w = if hkx
        .objects()
        .iter()
        .any(|o| o.class_name == "hkRootLevelContainer")
    {
        0.0
    } else {
        1.0
    };

    for obj_idx in 0..n_objects {
        let obj = &hkx.objects()[obj_idx];
        if !matches!(
            obj.class_name.as_str(),
            "hknpPhysicsSystemData" | "hknpRagdollData"
        ) {
            continue;
        }
        let is_ragdoll = obj.class_name == "hknpRagdollData";
        let inverse_inertia_w = if is_ragdoll {
            1.0
        } else {
            default_inverse_inertia_w
        };

        // Find bodyCinfos and existing motionCinfos.
        let body_arr = obj.members.iter().find(|m| m.name == "bodyCinfos");
        let motion_arr = obj.members.iter().find(|m| m.name == "motionCinfos");
        let body_arr = match body_arr {
            Some(m) => m,
            None => continue,
        };
        let HkxValue::Array(bodies) = &body_arr.value else {
            continue;
        };
        if bodies.is_empty() {
            continue;
        }
        let motion_properties_count = obj
            .members
            .iter()
            .find(|m| m.name == "motionProperties")
            .and_then(|m| match &m.value {
                HkxValue::Array(values) => Some(values.len()),
                _ => None,
            })
            .unwrap_or(0);
        // Skip if motionCinfos already populated.
        if let Some(ma) = motion_arr {
            if let HkxValue::Array(entries) = &ma.value {
                if !entries.is_empty() {
                    continue;
                }
            }
        }

        // For PSD: only fire when at least one dynamic body (flags=128).
        if !is_ragdoll {
            let has_dynamic = bodies.iter().any(|body| {
                let Some(members) = body.as_object_members() else {
                    return false;
                };
                members.iter().any(|bm| {
                    bm.name == "flags" && extract_int(&bm.value) == Some(BODY_FLAGS_DYNAMIC)
                })
            });
            if !has_dynamic {
                continue;
            }
        }

        // Find insertion position (after motionProperties if present, else at end).
        let insert_pos = obj
            .members
            .iter()
            .position(|m| m.name == "motionProperties")
            .map(|i| i + 1)
            .unwrap_or(obj.members.len());

        // Synthesize one hknpMotionCinfo per dynamic body. Native FO4 keeps
        // static character controller bodies in bodyCinfos with INVALID_ID.
        let mut motion_infos: Vec<HkxMember> = Vec::new();
        for body in bodies {
            let Some(body_members) = body.as_object_members() else {
                continue;
            };
            let has_invalid_motion_id = body_members
                .iter()
                .find(|member| member.name == "motionId")
                .and_then(|member| extract_int(&member.value))
                == Some(0x7FFF_FFFF);
            if is_ragdoll && has_invalid_motion_id {
                continue;
            }

            let motion_properties_id = body_members
                .iter()
                .find(|m| m.name == "motionPropertiesId")
                .and_then(|m| extract_int(&m.value))
                .and_then(|id| usize::try_from(id).ok())
                .filter(|&id| id < motion_properties_count)
                .and_then(|id| u16::try_from(id).ok())
                .unwrap_or(0);

            let mut position = vec![0.0f32, 0.0, 0.0, 0.0];
            let mut orientation = vec![0.0f32, 0.0, 0.0, 1.0];
            let mut mass_val: f32 = 1.0;
            let mut linvel = vec![0.0f32, 0.0, 0.0, 0.0];
            let mut angvel = vec![0.0f32, 0.0, 0.0, 0.0];
            let mut mass_dist_target = String::new();
            // Index of the shape object linked via body.shape (Pointer).
            let mut body_shape_idx: Option<usize> = None;

            for bm in body_members {
                match bm.name.as_str() {
                    "position" => {
                        if let HkxValue::F32List(v) = &bm.value {
                            if v.len() >= 4 {
                                position = v[..4].to_vec();
                            }
                        }
                    }
                    "orientation" => {
                        if let HkxValue::F32List(v) = &bm.value {
                            if v.len() == 4 {
                                orientation = v.clone();
                            }
                        }
                    }
                    "mass" => {
                        if let HkxValue::F32(v) = bm.value {
                            mass_val = v;
                        }
                    }
                    "linearVelocity" => {
                        if let HkxValue::F32List(v) = &bm.value {
                            if v.len() >= 4 {
                                linvel = v[..4].to_vec();
                            }
                        }
                    }
                    "angularVelocity" => {
                        if let HkxValue::F32List(v) = &bm.value {
                            if v.len() >= 4 {
                                angvel = v[..4].to_vec();
                            }
                        }
                    }
                    "massDistribution" => {
                        // In Rust model this is a Pointer(Some(idx)); look up the
                        // object name via that index for the name→idx map.
                        if let HkxValue::Pointer(Some(idx)) = bm.value {
                            if let Some(tgt) = hkx.objects().get(idx) {
                                mass_dist_target = tgt.name.clone().unwrap_or_default();
                            }
                        }
                    }
                    "shape" => {
                        if let HkxValue::Pointer(Some(idx)) = bm.value {
                            body_shape_idx = Some(idx);
                        }
                    }
                    _ => {}
                }
            }

            // --- Mass / inertia / COM resolution ---
            //
            // Priority order:
            //  1. body.massDistribution (hknpRefMassDistribution) — standard FO76 path.
            //  2. shape_mass_cache keyed by body.shape object name — fallback when
            //     body.mass is the FO76 sentinel (-1.0) and massDistribution is absent.
            //     The cache was built from hknpShapeMassProperties before
            //     `strip_fo76_skeleton_classes` stripped the shapes' `properties` pointer.
            //  3. Placeholder 1×1×1 identity inertia (last resort).

            let inv_mass_from_body = if mass_val > 1e-6 { 1.0 / mass_val } else { 1.0 };

            // Attempt shape-mass cache lookup when the body has no usable mass.
            let use_shape_cache = mass_val <= 0.0 && mass_dist_target.is_empty();
            let shape_cache_hit: Option<&ShapeMassInfo> = use_shape_cache
                .then(|| {
                    body_shape_idx.and_then(|si| {
                        hkx.objects()
                            .get(si)
                            .and_then(|o| o.name.as_deref())
                            .and_then(|name| shape_mass_cache.get(name))
                    })
                })
                .flatten();

            let (inv_mass, mass_factor, com_world, inv_inertia, composed_orientation) =
                if let Some(cache) = shape_cache_hit {
                    // Path 2: real mass from hknpShapeMassProperties.
                    // inverseMass = 1/mass; massFactor = 1.0 (valid constant, matches FO4 vanilla).
                    let im = cache.inverse_mass;

                    let rotated_com = quat_rotate_vector_xyzw(
                        [
                            orientation[0],
                            orientation[1],
                            orientation[2],
                            orientation[3],
                        ],
                        cache.center_of_mass,
                    );
                    let cw = vec![
                        position[0] + rotated_com[0],
                        position[1] + rotated_com[1],
                        position[2] + rotated_com[2],
                        position[3],
                    ];

                    // forward_inertia is the actual-mass forward inertia (not unit-mass).
                    // Invert per-axis to get inverseInertia (no additional mass scaling needed).
                    let ii = vec![
                        if cache.forward_inertia[0] > 1e-12 {
                            1.0 / cache.forward_inertia[0]
                        } else {
                            0.0
                        },
                        if cache.forward_inertia[1] > 1e-12 {
                            1.0 / cache.forward_inertia[1]
                        } else {
                            0.0
                        },
                        if cache.forward_inertia[2] > 1e-12 {
                            1.0 / cache.forward_inertia[2]
                        } else {
                            0.0
                        },
                        inverse_inertia_w,
                    ];

                    let maq = cache.major_axis_space;
                    let orient = quat_mul_xyzw(
                        [
                            orientation[0],
                            orientation[1],
                            orientation[2],
                            orientation[3],
                        ],
                        maq,
                    );

                    (im, 1.0_f32, cw, ii, orient)
                } else {
                    // Path 1 / 3: existing massDistribution or identity fallback.
                    let inv_mass = inv_mass_from_body;

                    let (com4, inertia4, major_axis4) =
                        resolve_mass_dist(hkx.objects(), &obj_by_name, &mass_dist_target);
                    let cw = if let Some(ref c) = com4 {
                        let rotated_com = quat_rotate_vector_xyzw(
                            [
                                orientation[0],
                                orientation[1],
                                orientation[2],
                                orientation[3],
                            ],
                            [c[0], c[1], c[2]],
                        );
                        vec![
                            position[0] + rotated_com[0],
                            position[1] + rotated_com[1],
                            position[2] + rotated_com[2],
                            position[3],
                        ]
                    } else {
                        position.clone()
                    };

                    // Compute principal-axis inertia + body←majorAxis quaternion.
                    // For the FO76 source schema, inertiaTensor is already diagonal.
                    // We still pass it through `diagonalize_inertia` so the convert
                    // path is correct if a future content variant supplies a full 3×3.
                    let (principal_inertia, derived_major_axis) = if let Some(ref it) = inertia4 {
                        // FO76 stores diagonal inertia as a vec4 (w unused). Build a
                        // diagonal 3×3, diagonalize → identity quat for diagonal input.
                        let i3x3 = [
                            it[0], 0.0, 0.0, //
                            0.0, it[1], 0.0, //
                            0.0, 0.0, it[2],
                        ];
                        let (diag, quat) =
                            crate::collision::mass_properties::diagonalize_inertia(i3x3);
                        (diag, quat)
                    } else {
                        ([1.0, 1.0, 1.0], [0.0, 0.0, 0.0, 1.0])
                    };
                    let density = com4
                        .as_ref()
                        .map(|com| com[3])
                        .filter(|volume| volume.is_finite() && *volume > 1e-12)
                        .filter(|_| mass_val.is_finite() && mass_val > 1e-6)
                        .map(|volume| mass_val / volume);
                    let mass_factor = density.unwrap_or(mass_val);
                    let ii = vec![
                        if principal_inertia[0].abs() > 1e-12 {
                            (1.0 / principal_inertia[0]) * inv_mass
                        } else {
                            0.0
                        },
                        if principal_inertia[1].abs() > 1e-12 {
                            (1.0 / principal_inertia[1]) * inv_mass
                        } else {
                            0.0
                        },
                        if principal_inertia[2].abs() > 1e-12 {
                            (1.0 / principal_inertia[2]) * inv_mass
                        } else {
                            0.0
                        },
                        inverse_inertia_w,
                    ];

                    // Body orientation in world. If the source carries an explicit
                    // majorAxisSpace quaternion (rotation from inertia major-axis to
                    // body space), or our diagonalization produced a non-identity
                    // rotation, compose: world←body * body←majorAxis. The motionCinfo
                    // `orientation` in FO4 must rotate the principal-axis inertia
                    // into world.
                    let major_axis_q = if let Some(ref m) = major_axis4 {
                        // Use source value when present (typical FO76 path).
                        if m.iter().map(|x| x * x).sum::<f32>() > 1e-6 {
                            [m[0], m[1], m[2], m[3]]
                        } else {
                            derived_major_axis
                        }
                    } else {
                        derived_major_axis
                    };
                    let orient = quat_mul_xyzw(
                        [
                            orientation[0],
                            orientation[1],
                            orientation[2],
                            orientation[3],
                        ],
                        major_axis_q,
                    );

                    (inv_mass, mass_factor, cw, ii, orient)
                };

            let orientation = vec![
                composed_orientation[0],
                composed_orientation[1],
                composed_orientation[2],
                composed_orientation[3],
            ];

            // Each entry is a TypedObject carrying the class name.
            motion_infos.push(HkxMember {
                name: "".to_string(),
                value: HkxValue::TypedObject {
                    class_name: "hknpMotionCinfo".to_string(),
                    members: vec![
                        HkxMember {
                            name: "motionPropertiesId".to_string(),
                            value: HkxValue::U16(motion_properties_id),
                        },
                        HkxMember {
                            name: "enableDeactivation".to_string(),
                            value: HkxValue::Bool(true),
                        },
                        HkxMember {
                            name: "inverseMass".to_string(),
                            value: HkxValue::F32(inv_mass),
                        },
                        HkxMember {
                            name: "massFactor".to_string(),
                            value: HkxValue::F32(mass_factor),
                        },
                        HkxMember {
                            name: "maxLinearAccelerationDistancePerStep".to_string(),
                            value: HkxValue::F32(FLT_CAP),
                        },
                        HkxMember {
                            name: "maxRotationToPreventTunneling".to_string(),
                            value: HkxValue::F32(FLT_CAP),
                        },
                        HkxMember {
                            name: "inverseInertiaLocal".to_string(),
                            value: HkxValue::F32List(inv_inertia),
                        },
                        HkxMember {
                            name: "centerOfMassWorld".to_string(),
                            value: HkxValue::F32List(com_world),
                        },
                        HkxMember {
                            name: "orientation".to_string(),
                            value: HkxValue::F32List(orientation),
                        },
                        HkxMember {
                            name: "linearVelocity".to_string(),
                            value: HkxValue::F32List(linvel),
                        },
                        HkxMember {
                            name: "angularVelocity".to_string(),
                            value: HkxValue::F32List(angvel),
                        },
                    ],
                },
            });
        }

        if !motion_infos.is_empty() {
            insertions.push((obj_idx, insert_pos, motion_infos));
        }
    }

    // Apply insertions.
    for (obj_idx, insert_pos, motion_infos) in insertions {
        let obj = &mut hkx.objects_mut()[obj_idx];
        let array_value = HkxValue::Array(motion_infos.into_iter().map(|m| m.value).collect());
        // If motionCinfos member already exists (empty), replace; otherwise insert.
        if let Some(m) = obj.members.iter_mut().find(|m| m.name == "motionCinfos") {
            m.value = array_value;
        } else {
            obj.members.insert(
                insert_pos,
                HkxMember {
                    name: "motionCinfos".to_string(),
                    value: array_value,
                },
            );
        }

        if let Some(motion_properties) = obj
            .members
            .iter_mut()
            .find(|m| m.name == "motionProperties")
        {
            if matches!(&motion_properties.value, HkxValue::Array(values) if values.is_empty()) {
                motion_properties.value = HkxValue::Array(vec![
                    crate::convert::templates::motion_properties_prototype(),
                ]);
            }
        } else {
            let motion_cinfos_pos = obj
                .members
                .iter()
                .position(|m| m.name == "motionCinfos")
                .unwrap_or(obj.members.len());
            obj.members.insert(
                motion_cinfos_pos,
                HkxMember {
                    name: "motionProperties".to_string(),
                    value: HkxValue::Array(vec![
                        crate::convert::templates::motion_properties_prototype(),
                    ]),
                },
            );
        }
    }

    // Drop hknpRefMassDistribution objects now that their data has been consumed.
    // FO4 has no classxml entry for this class.
    let ref_mass_indices: std::collections::HashSet<usize> = hkx
        .objects()
        .iter()
        .enumerate()
        .filter(|(_, o)| o.class_name == "hknpRefMassDistribution")
        .map(|(i, _)| i)
        .collect();
    if !ref_mass_indices.is_empty() {
        hkx.retain_objects_remap_pointers(|i, _| !ref_mass_indices.contains(&i));
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────

fn set_int_member(value: &mut HkxValue, target: i32) {
    match value {
        HkxValue::I8(v) => *v = target as i8,
        HkxValue::U8(v) => *v = target as u8,
        HkxValue::I16(v) => *v = target as i16,
        HkxValue::U16(v) => *v = target as u16,
        HkxValue::I32(v) => *v = target,
        HkxValue::U32(v) => *v = target as u32,
        HkxValue::I64(v) => *v = target as i64,
        HkxValue::U64(v) => *v = target as u64,
        HkxValue::Bool(v) => *v = target != 0,
        _ => {}
    }
}

/// Hamilton product of two quaternions in (x, y, z, w) order.
/// Returns `a * b` (apply b first, then a, in active rotation convention).
fn quat_mul_xyzw(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let (ax, ay, az, aw) = (a[0], a[1], a[2], a[3]);
    let (bx, by, bz, bw) = (b[0], b[1], b[2], b[3]);
    [
        aw * bx + ax * bw + ay * bz - az * by,
        aw * by - ax * bz + ay * bw + az * bx,
        aw * bz + ax * by - ay * bx + az * bw,
        aw * bw - ax * bx - ay * by - az * bz,
    ]
}

fn quat_rotate_vector_xyzw(quaternion: [f32; 4], vector: [f32; 3]) -> [f32; 3] {
    let norm_sq = quaternion.iter().map(|value| value * value).sum::<f32>();
    if !norm_sq.is_finite() || norm_sq < 1e-12 {
        return vector;
    }
    let inverse_norm = norm_sq.sqrt().recip();
    let q = [
        quaternion[0] * inverse_norm,
        quaternion[1] * inverse_norm,
        quaternion[2] * inverse_norm,
        quaternion[3] * inverse_norm,
    ];
    let t = [
        2.0 * (q[1] * vector[2] - q[2] * vector[1]),
        2.0 * (q[2] * vector[0] - q[0] * vector[2]),
        2.0 * (q[0] * vector[1] - q[1] * vector[0]),
    ];
    [
        vector[0] + q[3] * t[0] + q[1] * t[2] - q[2] * t[1],
        vector[1] + q[3] * t[1] + q[2] * t[0] - q[0] * t[2],
        vector[2] + q[3] * t[2] + q[0] * t[1] - q[1] * t[0],
    ]
}

fn extract_int(value: &HkxValue) -> Option<i32> {
    match value {
        HkxValue::I8(v) => Some(*v as i32),
        HkxValue::U8(v) => Some(*v as i32),
        HkxValue::I16(v) => Some(*v as i32),
        HkxValue::U16(v) => Some(*v as i32),
        HkxValue::I32(v) => Some(*v),
        HkxValue::U32(v) => Some(*v as i32),
        HkxValue::I64(v) => Some(*v as i32),
        HkxValue::U64(v) => Some(*v as i32),
        HkxValue::Bool(b) => Some(if *b { 1 } else { 0 }),
        _ => None,
    }
}

fn synthesize_fo4_weapon_character_property_aliases(hkx: &mut HkxFile) {
    if !hkx.contents_version().contains("2015") {
        return;
    }

    const PROPERTY_ALIASES: [(&str, &str); 2] = [
        ("DirectAtWeaponBoneIndex", "WeaponGripBoneIndex"),
        ("DirectAtWeaponLeftBoneIndex", "WeaponGripMirroredBoneIndex"),
    ];

    let mut patches = Vec::new();
    for (character_data_index, character_data) in hkx
        .objects()
        .iter()
        .enumerate()
        .filter(|(_, object)| object.class_name == "hkbCharacterData")
    {
        let Some(string_data_index) = pointer_member_value(&character_data.members, "stringData")
        else {
            continue;
        };
        let Some(property_values_index) =
            pointer_member_value(&character_data.members, "characterPropertyValues")
        else {
            continue;
        };
        let Some(HkxValue::Array(property_infos)) = character_data
            .members
            .iter()
            .find(|member| member.name == "characterPropertyInfos")
            .map(|member| &member.value)
        else {
            continue;
        };
        let Some(HkxValue::Array(property_names)) = hkx
            .objects()
            .get(string_data_index)
            .and_then(|object| {
                object
                    .members
                    .iter()
                    .find(|member| member.name == "characterPropertyNames")
            })
            .map(|member| &member.value)
        else {
            continue;
        };
        let Some(HkxValue::Array(property_values)) = hkx
            .objects()
            .get(property_values_index)
            .and_then(|object| {
                object
                    .members
                    .iter()
                    .find(|member| member.name == "wordVariableValues")
            })
            .map(|member| &member.value)
        else {
            continue;
        };
        if property_infos.len() != property_names.len()
            || property_values.len() != property_names.len()
        {
            continue;
        }

        for (target_name, source_name) in PROPERTY_ALIASES {
            if property_names
                .iter()
                .filter_map(string_value)
                .any(|name| name.eq_ignore_ascii_case(target_name))
            {
                continue;
            }
            let Some(source_index) = property_names
                .iter()
                .position(|name| string_value(name).is_some_and(|name| name == source_name))
            else {
                continue;
            };
            patches.push((
                character_data_index,
                string_data_index,
                property_values_index,
                target_name,
                property_infos[source_index].clone(),
                property_values[source_index].clone(),
            ));
        }
    }

    for (
        character_data_index,
        string_data_index,
        property_values_index,
        target_name,
        property_info,
        property_value,
    ) in patches
    {
        let Some(HkxValue::Array(property_infos)) = hkx
            .objects_mut()
            .get_mut(character_data_index)
            .and_then(|object| {
                object
                    .members
                    .iter_mut()
                    .find(|member| member.name == "characterPropertyInfos")
            })
            .map(|member| &mut member.value)
        else {
            continue;
        };
        property_infos.push(property_info);

        let Some(HkxValue::Array(property_names)) = hkx
            .objects_mut()
            .get_mut(string_data_index)
            .and_then(|object| {
                object
                    .members
                    .iter_mut()
                    .find(|member| member.name == "characterPropertyNames")
            })
            .map(|member| &mut member.value)
        else {
            continue;
        };
        property_names.push(HkxValue::String {
            value: target_name.to_string(),
            is_null: false,
        });

        let Some(HkxValue::Array(property_values)) = hkx
            .objects_mut()
            .get_mut(property_values_index)
            .and_then(|object| {
                object
                    .members
                    .iter_mut()
                    .find(|member| member.name == "wordVariableValues")
            })
            .map(|member| &mut member.value)
        else {
            continue;
        };
        property_values.push(property_value);
    }
}

fn remap_human_character_property_bone_indices(hkx: &mut HkxFile) {
    use std::collections::HashMap;

    if !hkx.contents_version().contains("2015") {
        return;
    }

    let fo4_indices: HashMap<&str, i32> = FO4_BONES
        .iter()
        .enumerate()
        .map(|(index, &name)| (name, index as i32))
        .collect();
    let mut replacements = Vec::new();

    for character_data in hkx
        .objects()
        .iter()
        .filter(|object| object.class_name == "hkbCharacterData")
    {
        let Some(string_data_index) = pointer_member_value(&character_data.members, "stringData")
        else {
            continue;
        };
        let Some(property_values_index) =
            pointer_member_value(&character_data.members, "characterPropertyValues")
        else {
            continue;
        };
        let Some(string_data) = hkx.objects().get(string_data_index) else {
            continue;
        };
        let Some(rig_name) = string_member_value(&string_data.members, "rigName") else {
            continue;
        };
        let normalized_rig_name = rig_name.replace('/', "\\").to_ascii_lowercase();
        if !normalized_rig_name.ends_with(r"\character\characterassets\skeleton.hkt")
            && !normalized_rig_name.ends_with(r"\character\characterassets\skeleton.hkx")
        {
            continue;
        }
        let Some(HkxValue::Array(property_names)) = string_data
            .members
            .iter()
            .find(|member| member.name == "characterPropertyNames")
            .map(|member| &member.value)
        else {
            continue;
        };
        let Some(HkxValue::Array(property_values)) = hkx
            .objects()
            .get(property_values_index)
            .and_then(|object| {
                object
                    .members
                    .iter()
                    .find(|member| member.name == "wordVariableValues")
            })
            .map(|member| &member.value)
        else {
            continue;
        };

        for (property_index, property_name) in property_names.iter().enumerate() {
            let Some(property_name) = string_value(property_name) else {
                continue;
            };
            let normalized_property_name = property_name.to_ascii_lowercase();
            let is_bone_index = (normalized_property_name.starts_with("directat")
                && normalized_property_name.ends_with("index"))
                || (normalized_property_name.contains("grip")
                    && normalized_property_name.ends_with("boneindex"));
            if !is_bone_index {
                continue;
            }
            let Some(source_index) = property_values
                .get(property_index)
                .and_then(HkxValue::as_object_members)
                .and_then(|members| members.iter().find(|member| member.name == "value"))
                .and_then(|member| extract_int(&member.value))
            else {
                continue;
            };
            let Some(&bone_name) = usize::try_from(source_index)
                .ok()
                .and_then(|index| FO76_BONES.get(index))
            else {
                continue;
            };
            let Some(&target_index) = fo4_indices.get(bone_name) else {
                continue;
            };
            if source_index != target_index {
                replacements.push((property_values_index, property_index, target_index));
            }
        }
    }

    for (object_index, property_index, target_index) in replacements {
        let Some(HkxValue::Array(property_values)) = hkx
            .objects_mut()
            .get_mut(object_index)
            .and_then(|object| {
                object
                    .members
                    .iter_mut()
                    .find(|member| member.name == "wordVariableValues")
            })
            .map(|member| &mut member.value)
        else {
            continue;
        };
        let Some(value_member) = property_values
            .get_mut(property_index)
            .and_then(HkxValue::as_object_members_mut)
            .and_then(|members| members.iter_mut().find(|member| member.name == "value"))
        else {
            continue;
        };
        value_member.value = HkxValue::I32(target_index);
    }
}

#[cfg(test)]
fn auto_fix_human_bone_tracks_is_noop(hkx: &HkxFile) -> bool {
    !hkx.objects().iter().any(|object| {
        is_real_animation_class(&object.class_name)
            || object
                .members
                .iter()
                .any(|member| member.name == "numberOfTransformTracks")
    })
}

/// Convert FO76 human-skeleton animation tracks to FO4 bone order.
///
/// For 96-track spline: decompress → strip AimSource (track 12) → reorder to FO4
/// bone order → recompress, then set identity bone indices on the binding.
/// For 96-track interleaved: same strip+reorder directly (no decompress/recompress).
/// For 90-95 track spline with non-identity indices: decompress → reorder → recompress,
/// set identity indices.
/// For 90-95 track interleaved with non-identity indices: remap indices in-place.
fn auto_fix_human_bone_tracks(hkx: &mut HkxFile) {
    use std::collections::HashMap;

    // Build FO4 bone name → index map.
    let fo4_index: HashMap<&str, usize> = FO4_BONES
        .iter()
        .enumerate()
        .map(|(i, &name)| (name, i))
        .collect();

    // Build animation object-name → binding-object-index map.
    let binding_by_anim_name: HashMap<Option<String>, usize> = hkx
        .objects()
        .iter()
        .enumerate()
        .filter(|(_, obj)| obj.class_name == "hkaAnimationBinding")
        .filter_map(|(binding_idx, obj)| {
            let anim_member = obj.members.iter().find(|m| m.name == "animation")?;
            if let HkxValue::Pointer(Some(anim_idx)) = anim_member.value {
                let anim_name = hkx.objects().get(anim_idx)?.name.clone();
                Some((anim_name, binding_idx))
            } else {
                None
            }
        })
        .collect();

    // Categorize animations.
    let mut strip_spline: Vec<usize> = Vec::new(); // 96-track spline
    let mut strip_interleaved: Vec<usize> = Vec::new(); // 96-track interleaved
    let mut reorder_spline: Vec<usize> = Vec::new(); // 90-95 track spline, non-identity binding
    let mut remap_interleaved: Vec<usize> = Vec::new(); // 90-95 track interleaved, non-identity

    for (i, obj) in hkx.objects().iter().enumerate() {
        let num_tracks = match obj
            .members
            .iter()
            .find(|m| m.name == "numberOfTransformTracks")
            .and_then(|m| direct_member_as_i32(&m.value))
        {
            Some(v) => v,
            None => continue,
        };

        match obj.class_name.as_str() {
            "hkaSplineCompressedAnimation" if num_tracks == 96 => strip_spline.push(i),
            "hkaInterleavedUncompressedAnimation" if num_tracks == 96 => strip_interleaved.push(i),
            "hkaSplineCompressedAnimation" if (90..=95).contains(&num_tracks) => {
                let binding_idx = binding_by_anim_name.get(&obj.name).copied();
                if let Some(bidx) = binding_idx {
                    if needs_fo76_remap(&hkx.objects()[bidx], num_tracks) {
                        reorder_spline.push(i);
                    }
                }
            }
            "hkaInterleavedUncompressedAnimation" if (90..=95).contains(&num_tracks) => {
                let binding_idx = binding_by_anim_name.get(&obj.name).copied();
                if let Some(bidx) = binding_idx {
                    if needs_fo76_remap(&hkx.objects()[bidx], num_tracks) {
                        remap_interleaved.push(i);
                    }
                }
            }
            _ => {}
        }
    }

    if strip_spline.is_empty()
        && strip_interleaved.is_empty()
        && reorder_spline.is_empty()
        && remap_interleaved.is_empty()
    {
        return;
    }

    // Build the FO76-post-strip → FO4 permutation for 96-track animations.
    // bone_mapping96[post_strip_track] = FO4 bone index.
    const STRIP_INDEX: usize = 12; // AimSource
    let bone_mapping96: Vec<i32> = (0..96_usize)
        .filter(|&i| i != STRIP_INDEX)
        .filter_map(|fo76_idx| {
            FO76_BONES
                .get(fo76_idx)
                .and_then(|name| fo4_index.get(name))
                .map(|&fo4_idx| fo4_idx as i32)
        })
        .collect();

    // Process 96-track interleaved animations in-place.
    {
        let objects = hkx.objects_mut();
        for &anim_idx in &strip_interleaved {
            let binding_idx = binding_by_anim_name.get(&objects[anim_idx].name).copied();
            strip_and_reorder_interleaved(&mut objects[anim_idx], STRIP_INDEX, &bone_mapping96);
            if let Some(bidx) = binding_idx {
                // Safety: anim_idx != bidx (different class names).
                let identity: Vec<HkxValue> = (0_i32..95).map(HkxValue::I32).collect();
                set_binding_bone_indices(&mut objects[bidx], identity);
            }
        }
    }

    // Process 90-95 track interleaved animations in-place.
    {
        let objects = hkx.objects_mut();
        for &anim_idx in &remap_interleaved {
            let binding_idx = match binding_by_anim_name.get(&objects[anim_idx].name).copied() {
                Some(b) => b,
                None => continue,
            };
            // Clone the FO76 indices before mutating.
            let fo76_indices: Vec<i32> =
                get_array_i32s(&objects[binding_idx], "transformTrackToBoneIndices");
            if fo76_indices.is_empty() {
                continue;
            }
            let num_tracks = fo76_indices.len();
            // Build FO4 index mapping from FO76 indices.
            let fo4_indices: Vec<i32> = fo76_indices
                .iter()
                .map(|&fo76_idx| {
                    let fo76_idx = fo76_idx as usize;
                    FO76_BONES
                        .get(fo76_idx)
                        .and_then(|name| fo4_index.get(name))
                        .map(|&fi| fi as i32)
                        .unwrap_or(fo76_idx as i32)
                })
                .collect();
            reorder_interleaved_transforms(&mut objects[anim_idx], num_tracks, &fo4_indices);
            reorder_annotation_tracks(&mut objects[anim_idx], num_tracks, &fo4_indices);
            let identity: Vec<HkxValue> = (0..num_tracks as i32).map(HkxValue::I32).collect();
            set_binding_bone_indices(&mut objects[binding_idx], identity);
        }
    }

    // Process spline animations: decompress, mutate, recompress.
    let spline_targets: Vec<(usize, bool)> = strip_spline
        .iter()
        .map(|&i| (i, true))
        .chain(reorder_spline.iter().map(|&i| (i, false)))
        .collect();

    if !spline_targets.is_empty() {
        decompress_then_mutate_recompress(
            hkx,
            &spline_targets,
            &binding_by_anim_name,
            &bone_mapping96,
            &fo4_index,
        );
    }
}

/// Returns true if a binding's `transformTrackToBoneIndices` is non-empty and
/// not already the identity sequence `[0, 1, …, num_tracks-1]`.
fn needs_fo76_remap(binding: &HkxObject, num_tracks: i32) -> bool {
    let indices = get_array_i32s(binding, "transformTrackToBoneIndices");
    if indices.is_empty() {
        return false; // empty = identity
    }
    if indices.len() != num_tracks as usize {
        return true; // wrong length — treat as needing remap
    }
    indices.iter().enumerate().any(|(i, &v)| v != i as i32)
}

/// Get a member's array as `Vec<i32>`. Returns empty if missing or wrong type.
fn get_array_i32s(obj: &HkxObject, member_name: &str) -> Vec<i32> {
    obj.members
        .iter()
        .find(|m| m.name == member_name)
        .and_then(|m| {
            if let HkxValue::Array(arr) = &m.value {
                Some(arr.iter().filter_map(|v| direct_member_as_i32(v)).collect())
            } else {
                None
            }
        })
        .unwrap_or_default()
}

/// Strip AimSource track (`strip_idx`) from an interleaved animation, then
/// reorder the remaining tracks from FO76 bone order to FO4 bone order.
///
/// `bone_mapping[post_strip_track] = FO4 bone index` — used to build the
/// inverse permutation so track data lands in FO4 bone order.
fn strip_and_reorder_interleaved(obj: &mut HkxObject, strip_idx: usize, bone_mapping: &[i32]) {
    let num_tracks = match obj
        .members
        .iter()
        .find(|m| m.name == "numberOfTransformTracks")
        .and_then(|m| direct_member_as_i32(&m.value))
    {
        Some(v) if v > 0 => v as usize,
        _ => return,
    };

    let expected = num_tracks - 1;

    // --- Strip transforms ---
    if let Some(m) = obj.members.iter_mut().find(|m| m.name == "transforms") {
        if let HkxValue::Array(ref mut transforms) = m.value {
            let total = transforms.len();
            if total % num_tracks == 0 {
                let mut kept = Vec::with_capacity(total - total / num_tracks);
                for (flat_idx, val) in transforms.drain(..).enumerate() {
                    if flat_idx % num_tracks != strip_idx {
                        kept.push(val);
                    }
                }
                *transforms = kept;
            }
        }
    }

    // --- Strip annotation tracks ---
    if let Some(m) = obj
        .members
        .iter_mut()
        .find(|m| m.name == "annotationTracks")
    {
        if let HkxValue::Array(ref mut tracks) = m.value {
            if tracks.len() == num_tracks {
                tracks.remove(strip_idx);
            }
        }
    }

    // --- Update numberOfTransformTracks ---
    if let Some(m) = obj
        .members
        .iter_mut()
        .find(|m| m.name == "numberOfTransformTracks")
    {
        m.value = HkxValue::I32(expected as i32);
    }

    // --- Reorder tracks to FO4 bone order ---
    // bone_mapping[fo76_post_strip_track] = fo4_bone_index.
    // Build inverse: fo4_pos -> fo76_post_strip_track.
    if bone_mapping.len() == expected {
        reorder_interleaved_transforms(obj, expected, bone_mapping);
        reorder_annotation_tracks(obj, expected, bone_mapping);
    }
}

/// Reorder transform tracks from source order to FO4 bone order.
///
/// `bone_mapping[src_track] = fo4_bone_index`. Builds inverse permutation:
/// for each FO4 output position `j`, reads from `src_track` where
/// `bone_mapping[src_track] == j`.
fn reorder_interleaved_transforms(obj: &mut HkxObject, num_tracks: usize, bone_mapping: &[i32]) {
    let m = match obj.members.iter_mut().find(|m| m.name == "transforms") {
        Some(m) => m,
        None => return,
    };
    let HkxValue::Array(ref mut transforms) = m.value else {
        return;
    };
    if transforms.is_empty() {
        return;
    }
    let total = transforms.len();
    if total % num_tracks != 0 {
        return;
    }
    let num_frames = total / num_tracks;

    // Build inverse: fo4_pos -> source track index.
    let mut inverse = vec![0usize; num_tracks];
    for (src_track, &fo4_idx) in bone_mapping.iter().enumerate() {
        let fo4_idx = fo4_idx as usize;
        if fo4_idx < num_tracks {
            inverse[fo4_idx] = src_track;
        }
    }

    let mut reordered = Vec::with_capacity(total);
    for frame in 0..num_frames {
        let frame_start = frame * num_tracks;
        for fo4_pos in 0..num_tracks {
            reordered.push(transforms[frame_start + inverse[fo4_pos]].clone());
        }
    }
    *transforms = reordered;
}

/// Reorder annotation tracks from source order to FO4 bone order.
fn reorder_annotation_tracks(obj: &mut HkxObject, num_tracks: usize, bone_mapping: &[i32]) {
    let m = match obj
        .members
        .iter_mut()
        .find(|m| m.name == "annotationTracks")
    {
        Some(m) => m,
        None => return,
    };
    let HkxValue::Array(ref mut tracks) = m.value else {
        return;
    };
    if tracks.len() != num_tracks {
        return;
    }

    let mut inverse = vec![0usize; num_tracks];
    for (src_track, &fo4_idx) in bone_mapping.iter().enumerate() {
        let fo4_idx = fo4_idx as usize;
        if fo4_idx < num_tracks {
            inverse[fo4_idx] = src_track;
        }
    }

    let reordered: Vec<HkxValue> = (0..num_tracks)
        .map(|fo4_pos| tracks[inverse[fo4_pos]].clone())
        .collect();
    *tracks = reordered;
}

/// Set `transformTrackToBoneIndices` on a binding object.
fn set_binding_bone_indices(binding: &mut HkxObject, indices: Vec<HkxValue>) {
    if let Some(m) = binding
        .members
        .iter_mut()
        .find(|m| m.name == "transformTrackToBoneIndices")
    {
        m.value = HkxValue::Array(indices);
    } else {
        binding.members.push(HkxMember {
            name: "transformTrackToBoneIndices".to_string(),
            value: HkxValue::Array(indices),
        });
    }
}

fn remap_annotation_tracks_member(
    obj: &HkxObject,
    num_tracks_out: usize,
    strip_idx: Option<usize>,
    inverse: &[usize],
) -> Option<HkxMember> {
    let member = obj.members.iter().find(|m| m.name == "annotationTracks")?;
    let HkxValue::Array(src_tracks) = &member.value else {
        return Some(member.clone());
    };

    if src_tracks.is_empty() {
        return Some(member.clone());
    }

    let mut tracks = src_tracks.clone();
    if let Some(strip_idx) = strip_idx {
        if tracks.len() == num_tracks_out + 1 && strip_idx < tracks.len() {
            tracks.remove(strip_idx);
        }
    }

    if tracks.len() == num_tracks_out && inverse.len() == num_tracks_out {
        tracks = (0..num_tracks_out)
            .map(|fo4_pos| tracks[inverse[fo4_pos]].clone())
            .collect();
    } else if tracks.len() > num_tracks_out {
        tracks.truncate(num_tracks_out);
    }

    Some(HkxMember {
        name: member.name.clone(),
        value: HkxValue::Array(tracks),
    })
}

/// Decompress spline animations at `targets`, apply strip-or-reorder, recompress.
///
/// `targets`: `(object_index, is_strip_96)` — true = 96-track strip+reorder,
/// false = 90-95-track reorder only.
fn decompress_then_mutate_recompress(
    hkx: &mut HkxFile,
    targets: &[(usize, bool)],
    binding_by_anim_name: &std::collections::HashMap<Option<String>, usize>,
    bone_mapping96: &[i32],
    fo4_index: &std::collections::HashMap<&str, usize>,
) {
    use crate::animation::spline::{compress_spline, decompress_spline};

    let mut replacements: Vec<(usize, HkxObject, Option<(usize, Vec<HkxValue>)>)> = Vec::new();

    for &(obj_idx, is_strip) in targets {
        let obj = &hkx.objects()[obj_idx];

        let get_i32 = |name: &str| -> Option<i32> {
            obj.members
                .iter()
                .find(|m| m.name == name)
                .and_then(|m| direct_member_as_i32(&m.value))
        };
        let get_f32 = |name: &str| -> Option<f32> {
            obj.members.iter().find(|m| m.name == name).and_then(|m| {
                if let HkxValue::F32(v) = m.value {
                    Some(v)
                } else {
                    None
                }
            })
        };

        let num_tracks_orig = match get_i32("numberOfTransformTracks") {
            Some(v) if v > 0 => v as usize,
            _ => continue,
        };
        let num_frames = match get_i32("numFrames").or_else(|| get_i32("numberOfFrames")) {
            Some(v) if v > 0 => v as u32,
            _ => continue,
        };
        let num_blocks = match get_i32("numBlocks") {
            Some(v) if v > 0 => v as u32,
            _ => continue,
        };
        let max_frames_per_block = get_i32("maxFramesPerBlock").unwrap_or(256).max(1) as u32;
        let num_floats = get_i32("numberOfFloatTracks").unwrap_or(0).max(0) as u32;
        let duration = get_f32("duration").unwrap_or(0.0);
        let frame_duration = get_f32("frameDuration").unwrap_or(if num_frames > 1 {
            duration / (num_frames - 1) as f32
        } else {
            1.0 / 30.0
        });

        let block_offsets: Vec<u32> = obj
            .members
            .iter()
            .find(|m| m.name == "blockOffsets")
            .and_then(|m| {
                if let HkxValue::Array(arr) = &m.value {
                    Some(
                        arr.iter()
                            .filter_map(|v| direct_member_as_i32(v).map(|i| i as u32))
                            .collect(),
                    )
                } else {
                    None
                }
            })
            .unwrap_or_default();

        let float_block_offsets: Vec<u32> = obj
            .members
            .iter()
            .find(|m| m.name == "floatBlockOffsets")
            .and_then(|m| {
                if let HkxValue::Array(arr) = &m.value {
                    Some(
                        arr.iter()
                            .filter_map(|v| direct_member_as_i32(v).map(|i| i as u32))
                            .collect(),
                    )
                } else {
                    None
                }
            })
            .unwrap_or_default();

        let data_bytes: Vec<u8> = obj
            .members
            .iter()
            .find(|m| m.name == "data")
            .and_then(|m| {
                if let HkxValue::Array(arr) = &m.value {
                    Some(
                        arr.iter()
                            .filter_map(|v| match v {
                                HkxValue::U8(b) => Some(*b),
                                _ => direct_member_as_i32(v).map(|i| i as u8),
                            })
                            .collect(),
                    )
                } else {
                    None
                }
            })
            .unwrap_or_default();

        if data_bytes.is_empty() || block_offsets.is_empty() {
            continue;
        }

        let mask_and_quant_size = ((4 * num_tracks_orig as u32 + num_floats + 3) / 4) * 4;
        let block_duration = if num_frames > 1 {
            duration / (num_frames - 1) as f32 * (max_frames_per_block - 1) as f32
        } else {
            (max_frames_per_block - 1) as f32 * frame_duration
        };
        let block_inverse_duration = if block_duration > 0.0 {
            1.0 / block_duration
        } else {
            0.0
        };

        let mut frames = match decompress_spline(
            &data_bytes,
            num_tracks_orig as u32,
            num_floats,
            num_frames,
            max_frames_per_block,
            num_blocks,
            &block_offsets,
            &float_block_offsets,
            mask_and_quant_size,
            block_duration,
            block_inverse_duration,
            frame_duration,
        ) {
            Ok(f) => f,
            Err(_) => continue,
        };

        let (num_tracks_out, bone_indices_out, annotation_tracks_out) = if is_strip {
            // 96-track: strip AimSource, reorder to FO4 bone order.
            const STRIP_IDX: usize = 12;
            for frame in &mut frames {
                frame.transforms.remove(STRIP_IDX);
            }
            let expected = num_tracks_orig - 1;
            // Reorder frame transforms using bone_mapping96 inverse.
            let mut inverse = vec![0usize; expected];
            for (src, &fo4_idx) in bone_mapping96.iter().enumerate() {
                let fo4_idx = fo4_idx as usize;
                if fo4_idx < expected {
                    inverse[fo4_idx] = src;
                }
            }
            for frame in &mut frames {
                let old = frame.transforms.clone();
                for fo4_pos in 0..expected {
                    frame.transforms[fo4_pos] = old[inverse[fo4_pos]].clone();
                }
            }
            let identity: Vec<HkxValue> = (0..expected as i32).map(HkxValue::I32).collect();
            let annotation_tracks_out =
                remap_annotation_tracks_member(obj, expected, Some(STRIP_IDX), &inverse);
            (expected, identity, annotation_tracks_out)
        } else {
            // 90-95 track: reorder using binding's FO76 indices.
            let binding_idx = match binding_by_anim_name
                .get(&hkx.objects()[obj_idx].name)
                .copied()
            {
                Some(b) => b,
                None => continue,
            };
            let fo76_indices: Vec<i32> =
                get_array_i32s(&hkx.objects()[binding_idx], "transformTrackToBoneIndices");
            if fo76_indices.is_empty() || fo76_indices.len() != num_tracks_orig {
                continue;
            }
            let fo4_indices: Vec<i32> = fo76_indices
                .iter()
                .map(|&fo76_idx| {
                    let fo76_idx = fo76_idx as usize;
                    FO76_BONES
                        .get(fo76_idx)
                        .and_then(|name| fo4_index.get(name))
                        .map(|&fi| fi as i32)
                        .unwrap_or(fo76_idx as i32)
                })
                .collect();

            // Build inverse permutation.
            let mut inverse = vec![0usize; num_tracks_orig];
            for (src, &fo4_idx) in fo4_indices.iter().enumerate() {
                let fo4_idx = fo4_idx as usize;
                if fo4_idx < num_tracks_orig {
                    inverse[fo4_idx] = src;
                }
            }
            for frame in &mut frames {
                let old = frame.transforms.clone();
                for fo4_pos in 0..num_tracks_orig {
                    frame.transforms[fo4_pos] = old[inverse[fo4_pos]].clone();
                }
            }
            let identity: Vec<HkxValue> = (0..num_tracks_orig as i32).map(HkxValue::I32).collect();
            let annotation_tracks_out =
                remap_annotation_tracks_member(obj, num_tracks_orig, None, &inverse);
            (num_tracks_orig, identity, annotation_tracks_out)
        };

        // Recompress.
        let blob = match compress_spline(&frames, duration, 30.0) {
            Ok(b) => b,
            Err(_) => continue,
        };

        // Build replacement spline object.
        let src = &hkx.objects()[obj_idx];
        let mut new_members: Vec<HkxMember> = Vec::new();
        new_members.push(HkxMember {
            name: "type".to_string(),
            value: HkxValue::I32(HK_SPLINE_COMPRESSED_ANIMATION_TYPE),
        });
        new_members.push(HkxMember {
            name: "duration".to_string(),
            value: HkxValue::F32(duration),
        });
        new_members.push(HkxMember {
            name: "numberOfTransformTracks".to_string(),
            value: HkxValue::I32(num_tracks_out as i32),
        });
        for name in &["numberOfFloatTracks", "extractedMotion"] {
            if let Some(m) = src.members.iter().find(|m| &m.name == name) {
                new_members.push(m.clone());
            }
        }
        if let Some(member) = annotation_tracks_out {
            new_members.push(member);
        }
        new_members.push(HkxMember {
            name: "numFrames".to_string(),
            value: HkxValue::I32(frames.len() as i32),
        });
        new_members.push(HkxMember {
            name: "numBlocks".to_string(),
            value: HkxValue::I32(blob.num_blocks as i32),
        });
        new_members.push(HkxMember {
            name: "maxFramesPerBlock".to_string(),
            value: HkxValue::I32(blob.max_frames_per_block as i32),
        });
        new_members.push(HkxMember {
            name: "maskAndQuantizationSize".to_string(),
            value: HkxValue::I32(blob.mask_and_quant_size as i32),
        });
        new_members.push(HkxMember {
            name: "blockDuration".to_string(),
            value: HkxValue::F32(blob.block_duration),
        });
        new_members.push(HkxMember {
            name: "blockInverseDuration".to_string(),
            value: HkxValue::F32(blob.block_inverse_duration),
        });
        new_members.push(HkxMember {
            name: "frameDuration".to_string(),
            value: HkxValue::F32(blob.frame_duration),
        });
        new_members.push(HkxMember {
            name: "blockOffsets".to_string(),
            value: HkxValue::Array(
                blob.block_offsets
                    .iter()
                    .map(|&o| HkxValue::U32(o))
                    .collect(),
            ),
        });
        new_members.push(HkxMember {
            name: "floatBlockOffsets".to_string(),
            value: HkxValue::Array(
                blob.float_block_offsets
                    .iter()
                    .map(|&o| HkxValue::U32(o))
                    .collect(),
            ),
        });
        new_members.push(HkxMember {
            name: "transformOffsets".to_string(),
            value: HkxValue::Array(Vec::new()),
        });
        new_members.push(HkxMember {
            name: "floatOffsets".to_string(),
            value: HkxValue::Array(Vec::new()),
        });
        new_members.push(HkxMember {
            name: "data".to_string(),
            value: HkxValue::Array(blob.data.iter().map(|&b| HkxValue::U8(b)).collect()),
        });

        let replacement = HkxObject {
            name: src.name.clone(),
            offset: src.offset,
            signature: src.signature,
            class_name: "hkaSplineCompressedAnimation".to_string(),
            members: new_members,
        };

        // Determine if we need to update the binding.
        let binding_update = binding_by_anim_name
            .get(&hkx.objects()[obj_idx].name)
            .copied()
            .map(|bidx| (bidx, bone_indices_out.clone()));

        replacements.push((obj_idx, replacement, binding_update));
    }

    // Apply replacements to hkx.
    let objects = hkx.objects_mut();
    for (obj_idx, replacement, binding_update) in replacements {
        objects[obj_idx] = replacement;
        if let Some((bidx, indices)) = binding_update {
            set_binding_bone_indices(&mut objects[bidx], indices);
        }
    }
}

fn strip_heap_allocator(hkx: &mut HkxFile) {
    hkx.retain_objects_remap_pointers(|_, object| {
        !object.class_name.contains("hkContainerHeapAllocator")
    });
}

fn strip_resource_data(hkx: &mut HkxFile) {
    for object in hkx.objects_mut() {
        if object.class_name != "hkRootLevelContainer" {
            continue;
        }
        for member in &mut object.members {
            if member.name == "namedVariants" {
                remove_resource_data_variants(&mut member.value);
            }
        }
    }
    hkx.retain_objects_remap_pointers(|_, object| object.class_name != "hkResourceContainer");
}

fn remove_resource_data_variants(value: &mut HkxValue) {
    if let HkxValue::Array(values) = value {
        values.retain(|value| !is_named_variant(value, "Resource Data"));
    }
}

fn rename_variant(hkx: &mut HkxFile) {
    if hkx.objects().iter().any(|object| {
        matches!(
            object.class_name.as_str(),
            "hknpPhysicsSceneData" | "hknpRagdollData"
        )
    }) {
        return;
    }

    for object in hkx.objects_mut() {
        if object.class_name != "hkRootLevelContainer" {
            continue;
        }
        for member in &mut object.members {
            if member.name == "namedVariants" {
                rename_merged_animation_variant(&mut member.value);
            }
        }
    }
}

fn rename_merged_animation_variant(value: &mut HkxValue) {
    let HkxValue::Array(values) = value else {
        return;
    };
    for value in values {
        let HkxValue::Object(members) = value else {
            continue;
        };
        for member in members {
            if member.name == "name"
                && string_value(&member.value) == Some("Merged Animation Container")
            {
                member.value = HkxValue::String {
                    value: "Animation Container".to_string(),
                    is_null: false,
                };
            }
        }
    }
}

fn fix_version_metadata(hkx: &mut HkxFile) {
    hkx.set_class_version(11);
    hkx.set_contents_version("hk_2014.1.0-r1");
}

/// FO4 class version numbers for classes whose FO76 files may carry a wrong version.
/// Values come from the `_N` suffix of the FO4 classxml filename in `resource/classxml/`.
const FO4_CLASS_VERSIONS: &[(&str, u32)] = &[
    ("hkbCharacterData", 11), // FO4 internal version; classxml shows v10 but packfile stamps 11
    ("hkbCharacter", 4),      // hkbCharacter_4.xml
    ("hkbStateMachine", 5),   // hkbStateMachine_5.xml
    ("hkbClipGenerator", 4),  // hkbClipGenerator_4.xml
    // Physics/collision renames — all version 0 in FO4 classxml
    ("hkpRagdollConstraintData", 0), // hkpRagdollConstraintData_0.xml
    ("hknpPhysicsSystemData", 0),    // hknpPhysicsSystemData_0.xml
    ("hknpCompressedMeshShapeData", 0), // hknpCompressedMeshShapeData_0.xml
    ("hknpRefMassDistribution", 0),  // synthesized; no classxml
    ("hknpSphereShape", 0),          // hknpSphereShape_0.xml
    ("hkbLayer", 1),                 // hkbLayer_1.xml
];

fn stamp_fo76_schema_versions(hkx: &mut HkxFile) {
    for object in hkx.objects_mut() {
        if let Some(&(_, version)) = FO4_CLASS_VERSIONS
            .iter()
            .find(|(name, _)| *name == object.class_name.as_str())
        {
            object.signature = version;
        }
    }
}

fn synthesize_memory_resource_container(hkx: &mut HkxFile) {
    if !hkx
        .objects()
        .iter()
        .any(|object| is_real_animation_class(&object.class_name))
    {
        return;
    }
    if hkx
        .objects()
        .iter()
        .any(|object| object.class_name == "hkMemoryResourceContainer")
    {
        return;
    }

    let Some(root_index) = hkx
        .objects()
        .iter()
        .position(|object| object.class_name == "hkRootLevelContainer")
    else {
        return;
    };

    let Some(named_variants_index) = hkx.objects()[root_index].members.iter().position(|member| {
        member.name == "namedVariants" && matches!(member.value, HkxValue::Array(_))
    }) else {
        return;
    };

    let container_name = format!("#{:04}", max_numbered_object_name(hkx) + 1);
    let container_index = hkx.push_object(HkxObject {
        name: Some(container_name),
        offset: 0,
        signature: 1,
        class_name: "hkMemoryResourceContainer".to_string(),
        members: vec![
            HkxMember {
                name: "resourceHandles".to_string(),
                value: HkxValue::Array(vec![]),
            },
            HkxMember {
                name: "externalLinks".to_string(),
                value: HkxValue::Array(vec![]),
            },
            HkxMember {
                name: "name".to_string(),
                value: HkxValue::String {
                    value: String::new(),
                    is_null: false,
                },
            },
        ],
    });

    let HkxValue::Array(variants) =
        &mut hkx.objects_mut()[root_index].members[named_variants_index].value
    else {
        return;
    };
    variants.push(HkxValue::Object(vec![
        HkxMember {
            name: "name".to_string(),
            value: HkxValue::String {
                value: "Resource Data".to_string(),
                is_null: false,
            },
        },
        HkxMember {
            name: "className".to_string(),
            value: HkxValue::String {
                value: "hkMemoryResourceContainer".to_string(),
                is_null: false,
            },
        },
        HkxMember {
            name: "variant".to_string(),
            value: HkxValue::Pointer(Some(container_index)),
        },
    ]));
}

fn is_real_animation_class(class_name: &str) -> bool {
    matches!(
        class_name,
        "hkaAnimationBinding"
            | "hkaSplineCompressedAnimation"
            | "hkaInterleavedUncompressedAnimation"
            | "hkaLosslessCompressedAnimation"
    )
}

fn max_numbered_object_name(hkx: &HkxFile) -> u32 {
    hkx.objects()
        .iter()
        .filter_map(|object| object.name.as_deref())
        .filter_map(|name| name.strip_prefix('#'))
        .filter_map(|number| number.parse::<u32>().ok())
        .max()
        .unwrap_or(0)
}

fn is_named_variant(value: &HkxValue, expected_name: &str) -> bool {
    let HkxValue::Object(members) = value else {
        return false;
    };
    members
        .iter()
        .any(|member| member.name == "name" && string_value(&member.value) == Some(expected_name))
}

fn string_value(value: &HkxValue) -> Option<&str> {
    match value {
        HkxValue::String {
            value,
            is_null: false,
        } => Some(value.as_str()),
        _ => None,
    }
}

// ── FO76 → FO4 weapon collision: helpers and transforms ──────────────────
// These transforms emit new inline structs whose class differs from the parent
// member's `ctype` template, hence `HkxValue::TypedObject`.

/// Walk an `HkxValue` recursively and remap any `Pointer(Some(idx))` whose
/// `idx` is a key in `map` to the mapped value.
pub(crate) fn remap_pointers(value: &mut HkxValue, map: &std::collections::HashMap<usize, usize>) {
    match value {
        HkxValue::Pointer(Some(idx)) => {
            if let Some(&new_idx) = map.get(idx) {
                *idx = new_idx;
            }
        }
        HkxValue::Array(values) => {
            for v in values {
                remap_pointers(v, map);
            }
        }
        HkxValue::Object(members) | HkxValue::TypedObject { members, .. } => {
            for m in members {
                remap_pointers(&mut m.value, map);
            }
        }
        _ => {}
    }
}

// Pure geometry helper: given 6 polytope planes (each `[nx, ny, nz, d]`),
// return `(a, b)` capsule endpoints as 4-element vectors `[x, y, z, 0]`,
// or `None` if the planes don't form 3 opposing pairs.
#[allow(dead_code)]
pub(crate) fn compute_capsule_endpoints(planes: &[[f32; 4]]) -> Option<([f32; 4], [f32; 4])> {
    if planes.len() != 6 {
        return None;
    }

    let mut pairs: Vec<([f32; 4], [f32; 4])> = Vec::with_capacity(3);
    let mut used = [false; 6];
    for i in 0..6 {
        if used[i] {
            continue;
        }
        let ni = [planes[i][0], planes[i][1], planes[i][2]];
        for j in (i + 1)..6 {
            if used[j] {
                continue;
            }
            let nj = [planes[j][0], planes[j][1], planes[j][2]];
            let dot = ni[0] * nj[0] + ni[1] * nj[1] + ni[2] * nj[2];
            if dot < -0.99 {
                pairs.push((planes[i], planes[j]));
                used[i] = true;
                used[j] = true;
                break;
            }
        }
    }
    if pairs.len() != 3 {
        return None;
    }

    // (normal_xyz, center_along, extent)
    let mut axes: Vec<([f32; 3], f32, f32)> = pairs
        .iter()
        .map(|(p1, p2)| {
            let n = [p1[0], p1[1], p1[2]];
            let pos1 = -p1[3];
            let pos2 = p2[3];
            let center_along = (pos1 + pos2) / 2.0;
            let extent = (pos1 - pos2).abs();
            (n, center_along, extent)
        })
        .collect();
    axes.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));

    let mut center = [0.0f32; 3];
    for (n, c, _) in &axes {
        for i in 0..3 {
            center[i] += n[i] * c;
        }
    }

    let (longest_n, _, longest_ext) = axes[0];
    let half = longest_ext / 2.0;
    let a = [
        center[0] - longest_n[0] * half,
        center[1] - longest_n[1] * half,
        center[2] - longest_n[2] * half,
        0.0,
    ];
    let b = [
        center[0] + longest_n[0] * half,
        center[1] + longest_n[1] * half,
        center[2] + longest_n[2] * half,
        0.0,
    ];
    Some((a, b))
}

// No-op: FO4 supports `hkpLimitedHingeConstraintData` directly (vanilla Deathclaw
// keeps two limited-hinge constraints in its ragdoll), so reclassifying them to
// ragdoll constraints would produce a loadable but non-vanilla class graph. The
// slot keeps the always-on pipeline order stable.
fn convert_limited_hinge_to_ragdoll(_hkx: &mut HkxFile) {}

// Would wrap bare `hknpCapsuleShape` body-cinfo references in a synthesized
// `hknpDynamicCompoundShape` + `hknpDynamicCompoundShapeData`. The helper
// builders are unit-tested, but this pipeline step is a no-op.
fn wrap_capsules_in_compound_shape(_hkx: &mut HkxFile) {
    // DISABLED until the writer lays out newly synthesized
    // hknpDynamicCompoundShapeData correctly.
}

// Strip `hknpDynamicCompoundShape` wrappers from PSD blobs by replacing
// the single bodyCinfo+compound-shape pointer with one bodyCinfo per leaf
// shape and dropping the compound shape objects.
fn flatten_compound_shapes_in_psd(hkx: &mut HkxFile) {
    // Skip standalone skeleton blobs — flattening would destroy ragdoll bone mappings.
    let has_root = hkx
        .objects()
        .iter()
        .any(|o| o.class_name == "hkRootLevelContainer");
    if has_root {
        return;
    }
    let has_ragdoll = hkx.objects().iter().any(|o| {
        matches!(
            o.class_name.as_str(),
            "hknpRagdollData" | "hkpRagdollConstraintData" | "hkpLimitedHingeConstraintData"
        )
    });
    if has_ragdoll {
        return;
    }

    // Locate the PSD root.
    let psd_idx = match hkx
        .objects()
        .iter()
        .position(|o| o.class_name == "hknpPhysicsSystemData")
    {
        Some(i) => i,
        None => return,
    };
    // Indices of all hknpDynamicCompoundShape objects.
    let compound_indices: std::collections::HashSet<usize> = hkx
        .objects()
        .iter()
        .enumerate()
        .filter(|(_, o)| o.class_name == "hknpDynamicCompoundShape")
        .map(|(i, _)| i)
        .collect();
    if compound_indices.is_empty() {
        return;
    }

    // Inspect bodyCinfos for direct shape pointers into compound shapes.
    let mut points_at_compound = false;
    let body_cinfos_clone: Vec<HkxValue> = {
        let psd = &hkx.objects()[psd_idx];
        let Some(body_arr) = psd.members.iter().find(|m| m.name == "bodyCinfos") else {
            return;
        };
        let HkxValue::Array(values) = &body_arr.value else {
            return;
        };
        for bc in values {
            if let Some(members) = bc.as_object_members() {
                for sm in members {
                    if sm.name == "shape" {
                        if let HkxValue::Pointer(Some(tgt)) = sm.value {
                            if compound_indices.contains(&tgt) {
                                points_at_compound = true;
                            }
                        }
                    }
                }
            }
        }
        values.clone()
    };
    if !points_at_compound || body_cinfos_clone.is_empty() {
        return;
    }

    // Collect leaf shape indices (capsules / polytopes / spheres).
    let leaf_indices: Vec<usize> = hkx
        .objects()
        .iter()
        .enumerate()
        .filter(|(_, o)| {
            matches!(
                o.class_name.as_str(),
                "hknpCapsuleShape" | "hknpConvexPolytopeShape" | "hknpSphereShape"
            )
        })
        .map(|(i, _)| i)
        .collect();
    if leaf_indices.is_empty() {
        return;
    }

    // Build new bodyCinfos: one per leaf shape, cloned from the first template.
    let template_bc = body_cinfos_clone[0].clone();
    let mut new_bodies: Vec<HkxValue> = Vec::with_capacity(leaf_indices.len());
    for &shape_idx in &leaf_indices {
        let mut bc = template_bc.clone();
        if let Some(members) = bc.as_object_members_mut() {
            for sm in members {
                if sm.name == "shape" {
                    sm.value = HkxValue::Pointer(Some(shape_idx));
                    break;
                }
            }
        }
        new_bodies.push(bc);
    }
    let n_bodies = new_bodies.len();

    // Resize parallel arrays to match new body count.
    {
        let psd = &mut hkx.objects_mut()[psd_idx];
        for m in &mut psd.members {
            match m.name.as_str() {
                "bodyCinfos" => {
                    m.value = HkxValue::Array(std::mem::take(&mut new_bodies));
                }
                "materials" | "motionProperties" | "motionCinfos" | "referencedObjects" => {
                    if let HkxValue::Array(values) = &mut m.value {
                        if !values.is_empty() && values.len() < n_bodies {
                            let proto = values[0].clone();
                            while values.len() < n_bodies {
                                values.push(proto.clone());
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // Drop hknpDynamicCompoundShape* objects, remap pointers.
    let strip_set: std::collections::HashSet<&str> = [
        "hknpDynamicCompoundShape",
        "hknpDynamicCompoundShapeData",
        "hknpDynamicCompoundShapeTree",
    ]
    .into_iter()
    .collect();
    hkx.retain_objects_remap_pointers(|_, o| !strip_set.contains(o.class_name.as_str()));
}

// Wraps a FO76 bare-compound-shape weapon collision blob in a FO4-format
// `hknpPhysicsSystemData` root by deep-cloning the embedded vanilla FO4
// weapon PSD template, re-targeting its body-cinfo shape pointers, and
// renumbering the merged object graph.
fn migrate_compound_shape_to_physics_system(hkx: &mut HkxFile) {
    use crate::convert::templates::fo4_weapon_psd_object_template;

    // Gate: at least one hknpConvexPolytopeShape, no PSD/scene/ragdoll root.
    let shape_indices: Vec<usize> = hkx
        .objects()
        .iter()
        .enumerate()
        .filter(|(_, o)| o.class_name == "hknpConvexPolytopeShape")
        .map(|(i, _)| i)
        .collect();
    if shape_indices.is_empty() {
        return;
    }
    let has_physics_root = hkx.objects().iter().any(|o| {
        matches!(
            o.class_name.as_str(),
            "hknpPhysicsSystemData" | "hknpPhysicsSceneData" | "hknpRagdollData"
        )
    });
    if has_physics_root {
        return;
    }
    let n_shapes = shape_indices.len();

    // Collect kept companion objects (referenced by shapes). Each shape
    // can carry an `hkRefCountedProperties` companion via its `properties`
    // member. `hkUint16` connectivity pointers were already nulled by
    // `strip_shape_connectivity`, which runs earlier.
    let mut kept_indices: Vec<usize> = Vec::new();
    let mut kept_seen: std::collections::HashSet<usize> = Default::default();
    let keep = |idx: usize,
                kept_indices: &mut Vec<usize>,
                kept_seen: &mut std::collections::HashSet<usize>| {
        if kept_seen.insert(idx) {
            kept_indices.push(idx);
        }
    };
    for &shape_idx in &shape_indices {
        keep(shape_idx, &mut kept_indices, &mut kept_seen);
        let shape = &hkx.objects()[shape_idx];
        for m in &shape.members {
            if let HkxValue::Pointer(Some(tgt)) = m.value {
                let tgt_class = hkx.objects().get(tgt).map(|o| o.class_name.as_str());
                if tgt_class == Some("hkRefCountedProperties") {
                    keep(tgt, &mut kept_indices, &mut kept_seen);
                }
            }
        }
    }
    // One hknpShapeMassProperties per shape.
    let mass_prop_indices: Vec<usize> = hkx
        .objects()
        .iter()
        .enumerate()
        .filter(|(_, o)| o.class_name == "hknpShapeMassProperties")
        .map(|(i, _)| i)
        .collect();
    for &mp in mass_prop_indices.iter().take(n_shapes) {
        keep(mp, &mut kept_indices, &mut kept_seen);
    }

    // The new object graph is: [PSD] + [kept-from-source]. We deep-clone
    // only the PSD object. Materials, motion properties, motion cinfos, and
    // body cinfos are inline structs inside the PSD's parallel arrays, so they
    // don't need separate object slots.
    let mut new_psd = fo4_weapon_psd_object_template();

    // Resize each inline-array member to N entries (clone first proto).
    fn resize_array_member(obj: &mut HkxObject, name: &str, n: usize) {
        for m in &mut obj.members {
            if m.name != name {
                continue;
            }
            if let HkxValue::Array(values) = &mut m.value {
                if !values.is_empty() {
                    let proto = values[0].clone();
                    values.clear();
                    for _ in 0..n {
                        values.push(proto.clone());
                    }
                }
            }
            return;
        }
    }
    for arr in [
        "materials",
        "motionProperties",
        "motionCinfos",
        "bodyCinfos",
        "referencedObjects",
    ] {
        resize_array_member(&mut new_psd, arr, n_shapes);
    }

    // Wire per-body indices. Body shape pointers get a SENTINEL value
    // (usize::MAX - shape_ordinal), remapped to final indices in the second pass
    // after assembling new_objects; this avoids an invalid-index window during clone.
    const SHAPE_SENTINEL_BASE: usize = usize::MAX - 1024;
    fn set_body_index_member(body: &mut HkxValue, name: &str, value: HkxValue) {
        let Some(members) = body.as_object_members_mut() else {
            return;
        };
        for sm in members {
            if sm.name == name {
                sm.value = value;
                return;
            }
        }
    }
    fn set_body_shape_ptr(body: &mut HkxValue, target: usize) {
        let Some(members) = body.as_object_members_mut() else {
            return;
        };
        for sm in members {
            if sm.name == "shape" {
                sm.value = HkxValue::Pointer(Some(target));
                return;
            }
        }
    }

    {
        let new_psd_mut = &mut new_psd;
        for m in &mut new_psd_mut.members {
            if m.name == "bodyCinfos" {
                if let HkxValue::Array(bodies) = &mut m.value {
                    for (i, body) in bodies.iter_mut().enumerate() {
                        set_body_index_member(body, "motionId", HkxValue::U32(i as u32));
                        set_body_index_member(body, "materialId", HkxValue::U32(i as u32));
                        set_body_shape_ptr(body, SHAPE_SENTINEL_BASE + i);
                    }
                }
            } else if m.name == "motionCinfos" {
                if let HkxValue::Array(cinfos) = &mut m.value {
                    for (i, ci) in cinfos.iter_mut().enumerate() {
                        set_body_index_member(ci, "motionPropertiesId", HkxValue::U16(i as u16));
                    }
                }
            } else if m.name == "referencedObjects" {
                if let HkxValue::Array(refs) = &mut m.value {
                    for (i, r) in refs.iter_mut().enumerate() {
                        *r = HkxValue::Pointer(Some(SHAPE_SENTINEL_BASE + i));
                    }
                }
            }
        }
    }

    // Build new object list: [new_psd] + [kept objects in source order].
    let original_objects = hkx.objects().to_vec();
    let mut new_objects: Vec<HkxObject> = Vec::with_capacity(1 + kept_indices.len());
    new_objects.push(new_psd);

    // Build remap: old_index → new_index. Anything not in kept_indices
    // becomes None and pointers to it become null.
    let mut old_to_new: Vec<Option<usize>> = vec![None; original_objects.len()];
    for (i, &kept) in kept_indices.iter().enumerate() {
        old_to_new[kept] = Some(i + 1); // +1 for the PSD at index 0
        new_objects.push(original_objects[kept].clone());
    }

    // Remap pointers inside kept objects: old indices → new indices, and
    // resolve shape sentinels in the PSD to the corresponding new index of
    // each source shape (in shape_indices order).
    let mut shape_sentinel_map: std::collections::HashMap<usize, usize> = Default::default();
    for (ord, &old_shape_idx) in shape_indices.iter().enumerate() {
        if let Some(new_idx) = old_to_new[old_shape_idx] {
            shape_sentinel_map.insert(SHAPE_SENTINEL_BASE + ord, new_idx);
        }
    }

    // Remap pointers in all new_objects (PSD + kept).
    for obj in new_objects.iter_mut() {
        for m in obj.members.iter_mut() {
            // Sentinel resolve first (PSD inline pointers).
            remap_pointers(&mut m.value, &shape_sentinel_map);
            // Then null any pointer that targets a dropped object.
            null_dropped_pointers(&mut m.value, &old_to_new);
            // Then remap remaining valid pointers.
            remap_pointers_optional(&mut m.value, &old_to_new);
        }
    }

    // Replace hkx.objects with the new graph. We do this by retaining
    // none + pushing all — easiest is to rebuild via from_tagxml-like state
    // but HkxFile doesn't expose that — clear via retain then push.
    hkx.retain_objects_remap_pointers(|_, _| false);
    for obj in new_objects {
        hkx.push_object(obj);
    }
}

/// Null out any pointer whose old index maps to None in `old_to_new`.
fn null_dropped_pointers(value: &mut HkxValue, old_to_new: &[Option<usize>]) {
    match value {
        HkxValue::Pointer(Some(idx)) => {
            // Skip sentinel range — those are migrated separately.
            if *idx < old_to_new.len() && old_to_new[*idx].is_none() {
                *value = HkxValue::Pointer(None);
            }
        }
        HkxValue::Array(values) => {
            for v in values {
                null_dropped_pointers(v, old_to_new);
            }
        }
        HkxValue::Object(members) | HkxValue::TypedObject { members, .. } => {
            for m in members {
                null_dropped_pointers(&mut m.value, old_to_new);
            }
        }
        _ => {}
    }
}

/// Remap pointers to their new indices using `old_to_new`. Skips sentinels
/// (already mapped) and `None` entries (already nulled).
fn remap_pointers_optional(value: &mut HkxValue, old_to_new: &[Option<usize>]) {
    match value {
        HkxValue::Pointer(Some(idx)) => {
            if *idx < old_to_new.len() {
                if let Some(new_idx) = old_to_new[*idx] {
                    *idx = new_idx;
                }
            }
        }
        HkxValue::Array(values) => {
            for v in values {
                remap_pointers_optional(v, old_to_new);
            }
        }
        HkxValue::Object(members) | HkxValue::TypedObject { members, .. } => {
            for m in members {
                remap_pointers_optional(&mut m.value, old_to_new);
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Opt-in transforms: decompress_spline, strip_bones, recompress
// ---------------------------------------------------------------------------

/// Decompress all `hkaSplineCompressedAnimation` objects in `hkx` to
/// `hkaInterleavedUncompressedAnimation`, replacing them in-place.
fn decompress_spline_animations(hkx: &mut HkxFile) -> HavokResult<()> {
    use crate::animation::spline::decompress_spline_full;

    // Collect indices of spline animation objects.
    let spline_indices: Vec<usize> = hkx
        .objects()
        .iter()
        .enumerate()
        .filter(|(_, obj)| obj.class_name == "hkaSplineCompressedAnimation")
        .map(|(i, _)| i)
        .collect();

    if spline_indices.is_empty() {
        return Ok(());
    }

    // Build replacement objects for each spline animation.
    let mut replacements: Vec<(usize, HkxObject)> = Vec::with_capacity(spline_indices.len());

    for obj_idx in spline_indices {
        let obj = &hkx.objects()[obj_idx];

        // Helper closures over obj's members.
        let get_i32 = |name: &str| -> Option<i32> {
            obj.members
                .iter()
                .find(|m| m.name == name)
                .and_then(|m| direct_member_as_i32(&m.value))
        };
        let get_f32 = |name: &str| -> Option<f32> {
            obj.members
                .iter()
                .find(|m| m.name == name)
                .and_then(|m| match &m.value {
                    HkxValue::F32(v) => Some(*v),
                    _ => None,
                })
        };

        let num_frames = match get_i32("numFrames").or_else(|| get_i32("numberOfFrames")) {
            Some(v) if v > 0 => v as u32,
            _ => continue,
        };
        let num_blocks = match get_i32("numBlocks") {
            Some(v) if v > 0 => v as u32,
            _ => continue,
        };
        let max_frames_per_block = match get_i32("maxFramesPerBlock") {
            Some(v) if v > 0 => v as u32,
            _ => 256,
        };
        let num_tracks = match get_i32("numberOfTransformTracks") {
            Some(v) => v as u32,
            None => continue,
        };
        let num_floats = get_i32("numberOfFloatTracks").unwrap_or(0) as u32;
        let duration = get_f32("duration").unwrap_or(0.0);
        let frame_duration = get_f32("frameDuration").unwrap_or(if num_frames > 1 {
            duration / (num_frames - 1) as f32
        } else {
            1.0 / 30.0
        });

        // Read blockOffsets array.
        let block_offsets: Vec<u32> = obj
            .members
            .iter()
            .find(|m| m.name == "blockOffsets")
            .and_then(|m| {
                if let HkxValue::Array(arr) = &m.value {
                    Some(
                        arr.iter()
                            .filter_map(|v| direct_member_as_i32(v).map(|i| i as u32))
                            .collect(),
                    )
                } else {
                    None
                }
            })
            .unwrap_or_default();

        let float_block_offsets: Vec<u32> = obj
            .members
            .iter()
            .find(|m| m.name == "floatBlockOffsets")
            .and_then(|m| {
                if let HkxValue::Array(arr) = &m.value {
                    Some(
                        arr.iter()
                            .filter_map(|v| direct_member_as_i32(v).map(|i| i as u32))
                            .collect(),
                    )
                } else {
                    None
                }
            })
            .unwrap_or_default();

        // mask_and_quant_size = 4 * num_tracks rounded up to 4
        let mask_and_quant_size = ((4 * num_tracks + num_floats + 3) / 4) * 4;
        let block_duration = if num_frames > 1 {
            duration / (num_frames - 1) as f32 * (max_frames_per_block - 1) as f32
        } else {
            (max_frames_per_block - 1) as f32 * frame_duration
        };
        let block_inverse_duration = if block_duration > 0.0 {
            1.0 / block_duration
        } else {
            0.0
        };

        // Extract the raw data bytes.
        let data_bytes: Vec<u8> = obj
            .members
            .iter()
            .find(|m| m.name == "data")
            .and_then(|m| {
                if let HkxValue::Array(arr) = &m.value {
                    Some(
                        arr.iter()
                            .filter_map(|v| match v {
                                HkxValue::U8(b) => Some(*b),
                                _ => direct_member_as_i32(v).map(|i| i as u8),
                            })
                            .collect(),
                    )
                } else {
                    None
                }
            })
            .unwrap_or_default();

        if data_bytes.is_empty() || block_offsets.is_empty() {
            continue;
        }

        let decompressed = match decompress_spline_full(
            &data_bytes,
            num_tracks,
            num_floats,
            num_frames,
            max_frames_per_block,
            num_blocks,
            &block_offsets,
            &float_block_offsets,
            mask_and_quant_size,
            block_duration,
            block_inverse_duration,
            frame_duration,
        ) {
            Ok(f) => f,
            Err(_) => continue,
        };
        let frames = &decompressed.frames;
        let recovered_float_tracks = &decompressed.float_tracks;

        // Build transforms as flat Vec<HkxValue::F32List> (QsTransform = 12 floats each).
        let mut transforms: Vec<HkxValue> =
            Vec::with_capacity(num_frames as usize * num_tracks as usize);
        for frame in frames {
            for t in &frame.transforms {
                transforms.push(HkxValue::F32List(vec![
                    t.translation[0],
                    t.translation[1],
                    t.translation[2],
                    0.0,
                    t.rotation[0],
                    t.rotation[1],
                    t.rotation[2],
                    t.rotation[3],
                    t.scale[0],
                    t.scale[1],
                    t.scale[2],
                    0.0,
                ]));
            }
        }

        // Build floats as flat per-frame interleaved array
        // (`floats[frame * num_floats + ft_idx] = value`).
        let mut floats_flat: Vec<HkxValue> =
            Vec::with_capacity(num_frames as usize * num_floats as usize);
        if num_floats > 0 {
            for fi in 0..num_frames as usize {
                for ti in 0..num_floats as usize {
                    let v = recovered_float_tracks
                        .get(ti)
                        .and_then(|t| t.get(fi))
                        .copied()
                        .unwrap_or(0.0);
                    floats_flat.push(HkxValue::F32(v));
                }
            }
        }

        // Copy members that transfer to the interleaved object.
        let src = &hkx.objects()[obj_idx];
        let mut new_members: Vec<HkxMember> = Vec::new();

        // type enum
        new_members.push(HkxMember {
            name: "type".to_string(),
            value: HkxValue::I32(HK_INTERLEAVED_ANIMATION_TYPE),
        });

        // Copy: duration, numberOfTransformTracks, numberOfFloatTracks
        for name in &["duration", "numberOfTransformTracks", "numberOfFloatTracks"] {
            if let Some(m) = src.members.iter().find(|m| &m.name == name) {
                new_members.push(m.clone());
            }
        }
        // extractedMotion pointer
        if let Some(m) = src.members.iter().find(|m| m.name == "extractedMotion") {
            new_members.push(m.clone());
        }
        // annotationTracks array
        if let Some(m) = src.members.iter().find(|m| m.name == "annotationTracks") {
            new_members.push(m.clone());
        }

        // transforms array
        new_members.push(HkxMember {
            name: "transforms".to_string(),
            value: HkxValue::Array(transforms),
        });
        // floats array — interleaved per-frame, recovered from spline float tracks.
        new_members.push(HkxMember {
            name: "floats".to_string(),
            value: HkxValue::Array(floats_flat),
        });

        let replacement = HkxObject {
            name: src.name.clone(),
            offset: src.offset,
            signature: src.signature,
            class_name: "hkaInterleavedUncompressedAnimation".to_string(),
            members: new_members,
        };
        replacements.push((obj_idx, replacement));
    }

    // Apply replacements.
    if replacements.is_empty() {
        return Ok(());
    }
    let objects = hkx.objects_mut();
    for (idx, replacement) in replacements {
        objects[idx] = replacement;
    }
    Ok(())
}

/// FO76 → FO4 bone lists (96 and 95 bones respectively).
const FO76_BONES: &[&str] = &[
    "Root",
    "COM",
    "Pelvis",
    "LLeg_Thigh",
    "LLeg_Calf",
    "LLeg_Foot",
    "RLeg_Thigh",
    "RLeg_Calf",
    "RLeg_Foot",
    "Spine1",
    "Spine2",
    "Chest",
    "AimSource",
    "Neck",
    "Head",
    "LArm_Collarbone",
    "LArm_UpperArm",
    "LArm_ForeArm1",
    "LArm_ForeArm2",
    "LArm_ForeArm3",
    "LArm_Hand",
    "RArm_Collarbone",
    "RArm_UpperArm",
    "RArm_ForeArm1",
    "RArm_ForeArm2",
    "RArm_ForeArm3",
    "PipboyBone",
    "RArm_Hand",
    "WeaponLeft",
    "Weapon",
    "WeaponIKTargetL",
    "WeaponIKTargetR",
    "WeaponIKTargetLMirror",
    "WeaponIKTargetRMirror",
    "WeaponBolt",
    "WeaponExtra1",
    "WeaponExtra2",
    "WeaponExtra3",
    "WeaponMagazine",
    "WeaponMagazineChild1",
    "WeaponMagazineChild2",
    "WeaponMagazineChild3",
    "WeaponMagazineChild4",
    "WeaponMagazineChild5",
    "WeaponOptics1",
    "WeaponOptics2",
    "WeaponTrigger",
    "LArm_UpperTwist1",
    "LArm_UpperTwist2",
    "RArm_UpperTwist1",
    "RArm_UpperTwist2",
    "LLeg_Toe1",
    "RLeg_Toe1",
    "LArm_Finger11",
    "LArm_Finger12",
    "LArm_Finger13",
    "LArm_Finger21",
    "LArm_Finger22",
    "LArm_Finger23",
    "LArm_Finger31",
    "LArm_Finger32",
    "LArm_Finger33",
    "LArm_Finger41",
    "LArm_Finger42",
    "LArm_Finger43",
    "LArm_Finger51",
    "LArm_Finger52",
    "LArm_Finger53",
    "RArm_Finger11",
    "RArm_Finger12",
    "RArm_Finger13",
    "RArm_Finger21",
    "RArm_Finger22",
    "RArm_Finger23",
    "RArm_Finger31",
    "RArm_Finger32",
    "RArm_Finger33",
    "RArm_Finger41",
    "RArm_Finger42",
    "RArm_Finger43",
    "RArm_Finger51",
    "RArm_Finger52",
    "RArm_Finger53",
    "Camera",
    "Camera Control",
    "AnimObjectA",
    "AnimObjectB",
    "AnimObjectL1",
    "AnimObjectL2",
    "AnimObjectL3",
    "AnimObjectR1",
    "AnimObjectR2",
    "AnimObjectR3",
    "L_RibHelper",
    "R_RibHelper",
    "CamTarget",
];

const FO4_BONES: &[&str] = &[
    "Root",
    "COM",
    "Pelvis",
    "LLeg_Thigh",
    "LLeg_Calf",
    "LLeg_Foot",
    "RLeg_Thigh",
    "RLeg_Calf",
    "RLeg_Foot",
    "Spine1",
    "Spine2",
    "Chest",
    "Neck",
    "Head",
    "LArm_Collarbone",
    "LArm_UpperArm",
    "LArm_ForeArm1",
    "LArm_ForeArm2",
    "LArm_ForeArm3",
    "LArm_Hand",
    "RArm_Collarbone",
    "RArm_UpperArm",
    "RArm_ForeArm1",
    "RArm_ForeArm2",
    "RArm_ForeArm3",
    "PipboyBone",
    "RArm_Hand",
    "WeaponLeft",
    "Weapon",
    "WeaponBolt",
    "WeaponExtra1",
    "WeaponExtra2",
    "WeaponExtra3",
    "WeaponMagazine",
    "WeaponMagazineChild1",
    "WeaponMagazineChild2",
    "WeaponMagazineChild3",
    "WeaponMagazineChild4",
    "WeaponMagazineChild5",
    "WeaponOptics1",
    "WeaponOptics2",
    "WeaponTrigger",
    "LArm_UpperTwist1",
    "LArm_UpperTwist2",
    "RArm_UpperTwist1",
    "RArm_UpperTwist2",
    "LLeg_Toe1",
    "RLeg_Toe1",
    "LArm_Finger11",
    "LArm_Finger12",
    "LArm_Finger13",
    "LArm_Finger21",
    "LArm_Finger22",
    "LArm_Finger23",
    "LArm_Finger31",
    "LArm_Finger32",
    "LArm_Finger33",
    "LArm_Finger41",
    "LArm_Finger42",
    "LArm_Finger43",
    "LArm_Finger51",
    "LArm_Finger52",
    "LArm_Finger53",
    "RArm_Finger11",
    "RArm_Finger12",
    "RArm_Finger13",
    "RArm_Finger21",
    "RArm_Finger22",
    "RArm_Finger23",
    "RArm_Finger31",
    "RArm_Finger32",
    "RArm_Finger33",
    "RArm_Finger41",
    "RArm_Finger42",
    "RArm_Finger43",
    "RArm_Finger51",
    "RArm_Finger52",
    "RArm_Finger53",
    "Camera",
    "Camera Control",
    "AnimObjectA",
    "AnimObjectB",
    "WeaponIKTargetL",
    "WeaponIKTargetR",
    "WeaponIKTargetLMirror",
    "WeaponIKTargetRMirror",
    "AnimObjectL1",
    "AnimObjectL2",
    "AnimObjectL3",
    "AnimObjectR1",
    "AnimObjectR2",
    "AnimObjectR3",
    "L_RibHelper",
    "R_RibHelper",
    "CamTarget",
];

/// Strip FO76-specific AimSource bone track (index 12) from
/// `hkaInterleavedUncompressedAnimation` objects with 96 transform tracks,
/// and remap `transformTrackToBoneIndices` on the associated binding to FO4 ordering.
fn strip_extra_bone_tracks(hkx: &mut HkxFile) {
    const FO76_TRACK_COUNT: i32 = 96;
    const STRIP_INDEX: usize = 12; // AimSource

    // Build FO4 bone name → index map once.
    let fo4_index: std::collections::HashMap<&str, usize> = FO4_BONES
        .iter()
        .enumerate()
        .map(|(i, &name)| (name, i))
        .collect();

    // Build animation-name → binding-object-index map.
    let binding_map: std::collections::HashMap<Option<String>, usize> = hkx
        .objects()
        .iter()
        .enumerate()
        .filter(|(_, obj)| obj.class_name == "hkaAnimationBinding")
        .filter_map(|(binding_idx, obj)| {
            let target = obj.members.iter().find(|m| m.name == "animation")?;
            if let HkxValue::Pointer(Some(anim_idx)) = target.value {
                Some((hkx.objects().get(anim_idx)?.name.clone(), binding_idx))
            } else {
                None
            }
        })
        .collect();

    // Find interleaved animations with 96 tracks.
    let mut candidates: Vec<(usize, Option<usize>)> = Vec::new();
    for (i, obj) in hkx.objects().iter().enumerate() {
        if obj.class_name != "hkaInterleavedUncompressedAnimation" {
            continue;
        }
        let num_tracks = obj
            .members
            .iter()
            .find(|m| m.name == "numberOfTransformTracks")
            .and_then(|m| direct_member_as_i32(&m.value));
        if num_tracks != Some(FO76_TRACK_COUNT) {
            continue;
        }
        let binding_idx = binding_map.get(&obj.name).copied();
        candidates.push((i, binding_idx));
    }

    if candidates.is_empty() {
        return;
    }

    // Build track-to-bone mapping for 95 surviving tracks.
    let bone_mapping: Vec<i32> = (0..FO76_TRACK_COUNT as usize)
        .filter(|&i| i != STRIP_INDEX)
        .filter_map(|fo76_idx| {
            FO76_BONES
                .get(fo76_idx)
                .and_then(|name| fo4_index.get(name))
                .map(|&fo4_idx| fo4_idx as i32)
        })
        .collect();

    let objects = hkx.objects_mut();

    for (anim_idx, binding_idx_opt) in candidates {
        let obj = &mut objects[anim_idx];

        // Update numberOfTransformTracks
        if let Some(m) = obj
            .members
            .iter_mut()
            .find(|m| m.name == "numberOfTransformTracks")
        {
            m.value = HkxValue::I32(95);
        }

        // Strip interleaved transforms: remove every (STRIP_INDEX + k*96) element.
        if let Some(m) = obj.members.iter_mut().find(|m| m.name == "transforms") {
            if let HkxValue::Array(ref mut transforms) = m.value {
                let total = transforms.len();
                let num_frames = total / FO76_TRACK_COUNT as usize;
                let mut new_transforms = Vec::with_capacity(num_frames * 95);
                for frame in 0..num_frames {
                    for track in 0..FO76_TRACK_COUNT as usize {
                        if track == STRIP_INDEX {
                            continue;
                        }
                        new_transforms
                            .push(transforms[frame * FO76_TRACK_COUNT as usize + track].clone());
                    }
                }
                *transforms = new_transforms;
            }
        }

        // Strip annotation tracks
        if let Some(m) = obj
            .members
            .iter_mut()
            .find(|m| m.name == "annotationTracks")
        {
            if let HkxValue::Array(ref mut tracks) = m.value {
                if tracks.len() == FO76_TRACK_COUNT as usize {
                    tracks.remove(STRIP_INDEX);
                }
            }
        }

        // Update binding's transformTrackToBoneIndices
        if let Some(binding_idx) = binding_idx_opt {
            let binding = &mut objects[binding_idx];
            if let Some(m) = binding
                .members
                .iter_mut()
                .find(|m| m.name == "transformTrackToBoneIndices")
            {
                m.value = HkxValue::Array(bone_mapping.iter().map(|&i| HkxValue::I32(i)).collect());
            } else {
                binding.members.push(HkxMember {
                    name: "transformTrackToBoneIndices".to_string(),
                    value: HkxValue::Array(
                        bone_mapping.iter().map(|&i| HkxValue::I32(i)).collect(),
                    ),
                });
            }
        }
    }
}

/// Recompress `hkaInterleavedUncompressedAnimation` objects back to
/// `hkaSplineCompressedAnimation`.
fn recompress_animations(hkx: &mut HkxFile) -> HavokResult<()> {
    use crate::animation::spline::{
        SplineCompressionParams, SplineFrame, SplineTransform, compress_spline_with_params,
    };

    let interleaved_indices: Vec<usize> = hkx
        .objects()
        .iter()
        .enumerate()
        .filter(|(_, obj)| obj.class_name == "hkaInterleavedUncompressedAnimation")
        .map(|(i, _)| i)
        .collect();

    if interleaved_indices.is_empty() {
        return Ok(());
    }

    let mut replacements: Vec<(usize, HkxObject)> = Vec::with_capacity(interleaved_indices.len());

    for obj_idx in interleaved_indices {
        let obj = &hkx.objects()[obj_idx];

        let num_tracks = match obj
            .members
            .iter()
            .find(|m| m.name == "numberOfTransformTracks")
            .and_then(|m| direct_member_as_i32(&m.value))
        {
            Some(v) if v > 0 => v as usize,
            _ => continue,
        };

        let duration = obj
            .members
            .iter()
            .find(|m| m.name == "duration")
            .and_then(|m| {
                if let HkxValue::F32(v) = m.value {
                    Some(v)
                } else {
                    None
                }
            })
            .unwrap_or(0.0);

        // Extract transforms array (each element is F32List of 12 floats).
        let raw_transforms: Vec<HkxValue> = match obj
            .members
            .iter()
            .find(|m| m.name == "transforms")
            .map(|m| &m.value)
        {
            Some(HkxValue::Array(arr)) => arr.clone(),
            _ => continue,
        };

        if raw_transforms.is_empty() {
            continue;
        }

        let num_frames = raw_transforms.len() / num_tracks;
        if num_frames == 0 {
            continue;
        }

        let mut frames: Vec<SplineFrame> = Vec::with_capacity(num_frames);
        for frame_idx in 0..num_frames {
            let mut transforms: Vec<SplineTransform> = Vec::with_capacity(num_tracks);
            for track_idx in 0..num_tracks {
                let flat = match &raw_transforms[frame_idx * num_tracks + track_idx] {
                    HkxValue::F32List(v) => v.clone(),
                    _ => vec![0.0; 12],
                };
                transforms.push(SplineTransform {
                    translation: [
                        *flat.get(0).unwrap_or(&0.0),
                        *flat.get(1).unwrap_or(&0.0),
                        *flat.get(2).unwrap_or(&0.0),
                    ],
                    rotation: [
                        *flat.get(4).unwrap_or(&0.0),
                        *flat.get(5).unwrap_or(&0.0),
                        *flat.get(6).unwrap_or(&0.0),
                        *flat.get(7).unwrap_or(&1.0),
                    ],
                    scale: [
                        *flat.get(8).unwrap_or(&1.0),
                        *flat.get(9).unwrap_or(&1.0),
                        *flat.get(10).unwrap_or(&1.0),
                    ],
                });
            }
            frames.push(SplineFrame { transforms });
        }

        // Extract `floats` array (interleaved per-frame) into per-track Vec<f32>
        // to plumb through to compress_spline_with_params. Without this, float
        // tracks (IK weights, facial weights, etc.) are silently zeroed on
        // FO76->FO4 migration.
        let num_floats = obj
            .members
            .iter()
            .find(|m| m.name == "numberOfFloatTracks")
            .and_then(|m| direct_member_as_i32(&m.value))
            .map(|v| v.max(0) as usize)
            .unwrap_or(0);
        let raw_floats: Vec<HkxValue> = match obj
            .members
            .iter()
            .find(|m| m.name == "floats")
            .map(|m| &m.value)
        {
            Some(HkxValue::Array(arr)) => arr.clone(),
            _ => Vec::new(),
        };
        let mut float_tracks: Vec<Vec<f32>> = Vec::with_capacity(num_floats);
        if num_floats > 0 && raw_floats.len() >= num_floats * num_frames {
            for ti in 0..num_floats {
                let mut samples = Vec::with_capacity(num_frames);
                for fi in 0..num_frames {
                    let v = match &raw_floats[fi * num_floats + ti] {
                        HkxValue::F32(v) => *v,
                        other => direct_member_as_i32(other).map(|i| i as f32).unwrap_or(0.0),
                    };
                    samples.push(v);
                }
                float_tracks.push(samples);
            }
        }

        let blob = match compress_spline_with_params(
            &frames,
            &float_tracks,
            duration,
            30.0,
            &SplineCompressionParams::default(),
        ) {
            Ok(b) => b,
            Err(_) => continue,
        };

        // Build replacement spline animation object.
        let src = &hkx.objects()[obj_idx];
        let mut new_members: Vec<HkxMember> = Vec::new();

        // type enum
        new_members.push(HkxMember {
            name: "type".to_string(),
            value: HkxValue::I32(HK_SPLINE_COMPRESSED_ANIMATION_TYPE),
        });

        // Copy common hkaAnimation members
        for name in &[
            "duration",
            "numberOfTransformTracks",
            "numberOfFloatTracks",
            "extractedMotion",
            "annotationTracks",
        ] {
            if let Some(m) = src.members.iter().find(|m| &m.name == name) {
                new_members.push(m.clone());
            }
        }

        // Spline-specific metadata
        new_members.push(HkxMember {
            name: "numFrames".to_string(),
            value: HkxValue::I32(num_frames as i32),
        });
        new_members.push(HkxMember {
            name: "numBlocks".to_string(),
            value: HkxValue::I32(blob.num_blocks as i32),
        });
        new_members.push(HkxMember {
            name: "maxFramesPerBlock".to_string(),
            value: HkxValue::I32(blob.max_frames_per_block as i32),
        });
        new_members.push(HkxMember {
            name: "maskAndQuantizationSize".to_string(),
            value: HkxValue::I32(blob.mask_and_quant_size as i32),
        });
        new_members.push(HkxMember {
            name: "blockDuration".to_string(),
            value: HkxValue::F32(blob.block_duration),
        });
        new_members.push(HkxMember {
            name: "blockInverseDuration".to_string(),
            value: HkxValue::F32(blob.block_inverse_duration),
        });
        new_members.push(HkxMember {
            name: "frameDuration".to_string(),
            value: HkxValue::F32(blob.frame_duration),
        });
        new_members.push(HkxMember {
            name: "blockOffsets".to_string(),
            value: HkxValue::Array(
                blob.block_offsets
                    .iter()
                    .map(|&o| HkxValue::U32(o))
                    .collect(),
            ),
        });
        new_members.push(HkxMember {
            name: "floatBlockOffsets".to_string(),
            value: HkxValue::Array(
                blob.float_block_offsets
                    .iter()
                    .map(|&o| HkxValue::U32(o))
                    .collect(),
            ),
        });
        new_members.push(HkxMember {
            name: "transformOffsets".to_string(),
            value: HkxValue::Array(Vec::new()),
        });
        new_members.push(HkxMember {
            name: "floatOffsets".to_string(),
            value: HkxValue::Array(Vec::new()),
        });
        new_members.push(HkxMember {
            name: "data".to_string(),
            value: HkxValue::Array(blob.data.iter().map(|&b| HkxValue::U8(b)).collect()),
        });

        let replacement = HkxObject {
            name: src.name.clone(),
            offset: src.offset,
            signature: src.signature,
            class_name: "hkaSplineCompressedAnimation".to_string(),
            members: new_members,
        };
        replacements.push((obj_idx, replacement));
    }

    if replacements.is_empty() {
        return Ok(());
    }
    let objects = hkx.objects_mut();
    for (idx, replacement) in replacements {
        objects[idx] = replacement;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hkx::descriptors::DescriptorRegistry;
    use crate::hkx::types::HkxValue;
    use crate::hkx::{HkxMember, HkxObject};

    fn apply_for_test(hkx: &mut HkxFile) -> usize {
        let mut registry = DescriptorRegistry::new();
        let mut warnings = Vec::new();
        apply_implemented_transforms(hkx, &mut registry, &mut warnings)
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

    fn string(value: &str) -> HkxValue {
        HkxValue::String {
            value: value.to_string(),
            is_null: false,
        }
    }

    fn character_property_file(
        contents_version: &str,
        rig_name: &str,
        properties: &[(&str, i32)],
    ) -> HkxFile {
        HkxFile::from_tagxml(
            12,
            contents_version,
            vec![
                object(
                    "hkbCharacterData",
                    vec![
                        member("stringData", HkxValue::Pointer(Some(1))),
                        member("characterPropertyValues", HkxValue::Pointer(Some(2))),
                        member(
                            "characterPropertyInfos",
                            HkxValue::Array(
                                properties
                                    .iter()
                                    .map(|_| {
                                        HkxValue::Object(vec![
                                            member(
                                                "role",
                                                HkxValue::Object(vec![
                                                    member("role", HkxValue::I32(0)),
                                                    member("flags", HkxValue::I32(0)),
                                                ]),
                                            ),
                                            member("type", HkxValue::I32(2)),
                                        ])
                                    })
                                    .collect(),
                            ),
                        ),
                    ],
                ),
                object(
                    "hkbCharacterStringData",
                    vec![
                        member("rigName", string(rig_name)),
                        member(
                            "characterPropertyNames",
                            HkxValue::Array(
                                properties.iter().map(|(name, _)| string(name)).collect(),
                            ),
                        ),
                    ],
                ),
                object(
                    "hkbVariableValueSet",
                    vec![member(
                        "wordVariableValues",
                        HkxValue::Array(
                            properties
                                .iter()
                                .map(|(_, value)| {
                                    HkxValue::Object(vec![member("value", HkxValue::I32(*value))])
                                })
                                .collect(),
                        ),
                    )],
                ),
            ],
        )
    }

    fn character_property_values(hkx: &HkxFile) -> Vec<i32> {
        let HkxValue::Array(values) = &hkx.objects()[2].members[0].value else {
            panic!("wordVariableValues should be an array");
        };
        values
            .iter()
            .map(|value| {
                value
                    .as_object_members()
                    .and_then(|members| members.iter().find(|member| member.name == "value"))
                    .and_then(|member| extract_int(&member.value))
                    .expect("character property value")
            })
            .collect()
    }

    fn character_property_names(hkx: &HkxFile) -> Vec<&str> {
        let HkxValue::Array(names) = &hkx.objects()[1].members[1].value else {
            panic!("characterPropertyNames should be an array");
        };
        names
            .iter()
            .map(|name| string_value(name).expect("character property name"))
            .collect()
    }

    #[test]
    fn downgrade_cloth_flattens_newer_packed_local_vectors_for_fo4() {
        let packed_vectors = (0_i16..16)
            .map(|vector_index| {
                HkxValue::Object(vec![member(
                    "values",
                    HkxValue::Array(
                        (0_i16..4)
                            .map(|lane| HkxValue::I16(vector_index * 4 + lane))
                            .collect(),
                    ),
                )])
            })
            .collect::<Vec<_>>();
        let mut hkx = HkxFile::from_tagxml(
            12,
            "hk_2018.1.0-r1",
            vec![object(
                "hclObjectSpaceSkinPNOperator",
                vec![member(
                    "localPNs",
                    HkxValue::Array(vec![HkxValue::Object(vec![
                        member("localPosition", HkxValue::Array(packed_vectors.clone())),
                        member("localNormal", HkxValue::Array(packed_vectors)),
                    ])]),
                )],
            )],
        );
        let mut warnings = Vec::new();

        downgrade_fo76_cloth_to_fo4(&mut hkx, &mut warnings);

        assert!(warnings.is_empty(), "{warnings:?}");
        let local_pns = hkx.objects()[0]
            .members
            .iter()
            .find(|member| member.name == "localPNs")
            .and_then(|member| match &member.value {
                HkxValue::Array(values) => values.first(),
                _ => None,
            })
            .and_then(HkxValue::as_object_members)
            .expect("flattened localPN block");
        let expected = HkxValue::Array((0_i16..64).map(HkxValue::I16).collect());
        assert_eq!(
            local_pns
                .iter()
                .find(|member| member.name == "localPosition")
                .map(|member| &member.value),
            Some(&expected)
        );
        assert_eq!(
            local_pns
                .iter()
                .find(|member| member.name == "localNormal")
                .map(|member| &member.value),
            Some(&expected)
        );
    }

    fn test_binding_set(member_path: &str, variable_index: i32) -> HkxObject {
        object(
            "hkbVariableBindingSet",
            vec![
                member(
                    "bindings",
                    HkxValue::Array(vec![HkxValue::Object(vec![
                        member("memberPath", string(member_path)),
                        member("variableIndex", HkxValue::I32(variable_index)),
                        member("bitIndex", HkxValue::I8(-1)),
                        member("bindingType", HkxValue::I8(0)),
                    ])]),
                ),
                member("indexOfBindingToEnable", HkxValue::I32(-1)),
            ],
        )
    }

    fn behavior_string_data_with_variables(variable_names: &[&str]) -> HkxObject {
        object(
            "hkbBehaviorGraphStringData",
            vec![member(
                "variableNames",
                HkxValue::Array(variable_names.iter().map(|name| string(name)).collect()),
            )],
        )
    }

    fn named_variant(name: &str, class_name: &str, variant: Option<usize>) -> HkxValue {
        HkxValue::Object(vec![
            member("name", string(name)),
            member("className", string(class_name)),
            member("variant", HkxValue::Pointer(variant)),
        ])
    }

    #[test]
    fn strip_heap_allocator_removes_objects_and_remaps_pointers() {
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2015.1.0-r1",
            vec![
                object("hkContainerHeapAllocator", vec![]),
                object("Survivor", vec![]),
                object(
                    "Holder",
                    vec![
                        member("removed", HkxValue::Pointer(Some(0))),
                        member("surviving", HkxValue::Pointer(Some(1))),
                        member(
                            "array",
                            HkxValue::Array(vec![
                                HkxValue::Pointer(Some(0)),
                                HkxValue::Pointer(Some(1)),
                                HkxValue::I32(5),
                            ]),
                        ),
                    ],
                ),
            ],
        );

        strip_heap_allocator(&mut hkx);

        assert_eq!(hkx.objects().len(), 2);
        assert_eq!(hkx.objects()[0].class_name, "Survivor");
        let holder = &hkx.objects()[1];
        assert_eq!(holder.members[0].value, HkxValue::Pointer(None));
        assert_eq!(holder.members[1].value, HkxValue::Pointer(Some(0)));
        assert_eq!(
            holder.members[2].value,
            HkxValue::Array(vec![HkxValue::Pointer(Some(0)), HkxValue::I32(5)])
        );
    }

    #[test]
    fn strip_resource_data_removes_resource_container_and_named_variant() {
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2015.1.0-r1",
            vec![
                object(
                    "hkRootLevelContainer",
                    vec![member(
                        "namedVariants",
                        HkxValue::Array(vec![
                            named_variant("Resource Data", "hkResourceContainer", Some(1)),
                            named_variant("Animation Container", "hkaAnimationContainer", Some(2)),
                        ]),
                    )],
                ),
                object("hkResourceContainer", vec![]),
                object("hkaAnimationContainer", vec![]),
            ],
        );

        strip_resource_data(&mut hkx);

        assert_eq!(hkx.objects().len(), 2);
        assert!(
            !hkx.objects()
                .iter()
                .any(|obj| obj.class_name == "hkResourceContainer")
        );
        let HkxValue::Array(variants) = &hkx.objects()[0].members[0].value else {
            panic!("namedVariants should be an array");
        };
        assert_eq!(variants.len(), 1);
        let HkxValue::Object(variant_members) = &variants[0] else {
            panic!("namedVariants entry should be an object");
        };
        assert_eq!(variant_members[0].value, string("Animation Container"));
        assert_eq!(variant_members[2].value, HkxValue::Pointer(Some(1)));
    }

    #[test]
    fn rename_variant_renames_merged_animation_container_without_physics() {
        let mut animation_hkx = HkxFile::from_tagxml(
            11,
            "hk_2015.1.0-r1",
            vec![object(
                "hkRootLevelContainer",
                vec![member(
                    "namedVariants",
                    HkxValue::Array(vec![named_variant(
                        "Merged Animation Container",
                        "hkaAnimationContainer",
                        Some(1),
                    )]),
                )],
            )],
        );
        let mut physics_hkx = HkxFile::from_tagxml(
            11,
            "hk_2015.1.0-r1",
            vec![
                object(
                    "hkRootLevelContainer",
                    vec![member(
                        "namedVariants",
                        HkxValue::Array(vec![named_variant(
                            "Merged Animation Container",
                            "hkaAnimationContainer",
                            Some(1),
                        )]),
                    )],
                ),
                object("hknpRagdollData", vec![]),
            ],
        );

        rename_variant(&mut animation_hkx);
        rename_variant(&mut physics_hkx);

        let HkxValue::Array(animation_variants) = &animation_hkx.objects()[0].members[0].value
        else {
            panic!("namedVariants should be an array");
        };
        let HkxValue::Object(animation_members) = &animation_variants[0] else {
            panic!("namedVariants entry should be an object");
        };
        assert_eq!(animation_members[0].value, string("Animation Container"));

        let HkxValue::Array(physics_variants) = &physics_hkx.objects()[0].members[0].value else {
            panic!("namedVariants should be an array");
        };
        let HkxValue::Object(physics_members) = &physics_variants[0] else {
            panic!("namedVariants entry should be an object");
        };
        assert_eq!(
            physics_members[0].value,
            string("Merged Animation Container")
        );
    }

    #[test]
    fn fix_version_metadata_sets_fo4_metadata() {
        let mut hkx = HkxFile::from_tagxml(12, "hk_2015.1.0-r1", vec![]);

        fix_version_metadata(&mut hkx);

        assert_eq!(hkx.class_version(), 11);
        assert_eq!(hkx.contents_version(), "hk_2014.1.0-r1");
    }

    #[test]
    fn remap_human_character_property_bone_indices_uses_fo4_skeleton_order() {
        let properties = [
            ("DirectAtWeaponBoneIndex", 29),
            ("WeaponGripBoneIndex", 30),
            ("DirectAtSpine1BoneIndex", 9),
            ("DirectAtChestBoneIndex", 11),
            ("DirectAtRightUpperArmIndex", 22),
            ("DirectAtHeadBoneIndex", 14),
            ("DirectAtWeaponLeftBoneIndex", 28),
            ("WeaponGripMirroredBoneIndex", 32),
            ("WeaponAssemblyFullBlend", 123),
        ];
        let mut hkx = character_property_file(
            "hk_2015.1.0-r1",
            r"..\Character\CharacterAssets\skeleton.HKT",
            &properties,
        );

        remap_human_character_property_bone_indices(&mut hkx);

        assert_eq!(
            character_property_values(&hkx),
            [28, 82, 9, 11, 21, 13, 27, 84, 123]
        );
    }

    #[test]
    fn synthesize_fo4_weapon_character_property_aliases_copies_weapon_bone_indices() {
        let properties = [
            ("WeaponGripBoneIndex", 26),
            ("WeaponGripMirroredBoneIndex", -1),
        ];
        let mut hkx = character_property_file(
            "hk_2015.1.0-r1",
            r"Actors\MoleMiner\CharacterAssets\skeleton.hkx",
            &properties,
        );

        synthesize_fo4_weapon_character_property_aliases(&mut hkx);

        assert_eq!(
            character_property_names(&hkx),
            [
                "WeaponGripBoneIndex",
                "WeaponGripMirroredBoneIndex",
                "DirectAtWeaponBoneIndex",
                "DirectAtWeaponLeftBoneIndex",
            ]
        );
        assert_eq!(character_property_values(&hkx), [26, -1, 26, -1]);
        let HkxValue::Array(property_infos) = &hkx.objects()[0].members[2].value else {
            panic!("characterPropertyInfos should be an array");
        };
        assert_eq!(property_infos.len(), 4);
        assert_eq!(property_infos[2], property_infos[0]);
        assert_eq!(property_infos[3], property_infos[1]);
    }

    #[test]
    fn remap_human_character_property_bone_indices_skips_custom_rigs() {
        let properties = [("DirectAtWeaponBoneIndex", 29), ("WeaponGripBoneIndex", 30)];
        let mut hkx = character_property_file(
            "hk_2015.1.0-r1",
            r"CharacterAssets\skeleton.hkt",
            &properties,
        );

        remap_human_character_property_bone_indices(&mut hkx);

        assert_eq!(character_property_values(&hkx), [29, 30]);
    }

    #[test]
    fn remap_human_character_property_bone_indices_skips_fo4_files() {
        let properties = [("DirectAtWeaponBoneIndex", 28), ("WeaponGripBoneIndex", 82)];
        let mut hkx = character_property_file(
            "hk_2014.1.0-r1",
            r"..\Character\CharacterAssets\skeleton.HKT",
            &properties,
        );

        remap_human_character_property_bone_indices(&mut hkx);

        assert_eq!(character_property_values(&hkx), [28, 82]);
    }

    #[test]
    fn stamp_fo76_schema_versions_covers_all_fo4_class_versions() {
        for &(class_name, expected_sig) in FO4_CLASS_VERSIONS {
            let mut hkx =
                HkxFile::from_tagxml(12, "hk_2015.1.0-r1", vec![object(class_name, vec![])]);
            stamp_fo76_schema_versions(&mut hkx);
            assert_eq!(
                hkx.objects()[0].signature,
                expected_sig,
                "expected {class_name}.signature = {expected_sig}"
            );
        }
    }

    #[test]
    fn fix_sphere_dispatch_type_rewrites_i32_two_to_one_in_same_variant() {
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2014.1.0-r1",
            vec![object(
                "hknpSphereShape",
                vec![member("dispatchType", HkxValue::I32(2))],
            )],
        );

        fix_sphere_dispatch_type(&mut hkx);

        assert_eq!(hkx.objects()[0].members[0].value, HkxValue::I32(1));
    }

    #[test]
    fn fix_sphere_dispatch_type_rewrites_u8_two_to_one_in_same_variant() {
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2014.1.0-r1",
            vec![object(
                "hknpSphereShape",
                vec![member("dispatchType", HkxValue::U8(2))],
            )],
        );

        fix_sphere_dispatch_type(&mut hkx);

        assert_eq!(hkx.objects()[0].members[0].value, HkxValue::U8(1));
    }

    #[test]
    fn fix_sphere_dispatch_type_is_idempotent_when_already_one() {
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2014.1.0-r1",
            vec![object(
                "hknpSphereShape",
                vec![member("dispatchType", HkxValue::I32(1))],
            )],
        );

        fix_sphere_dispatch_type(&mut hkx);

        assert_eq!(hkx.objects()[0].members[0].value, HkxValue::I32(1));
    }

    #[test]
    fn fix_sphere_dispatch_type_leaves_non_sphere_classes_untouched() {
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2014.1.0-r1",
            vec![object(
                "hknpCapsuleShape",
                vec![member("dispatchType", HkxValue::I32(2))],
            )],
        );

        fix_sphere_dispatch_type(&mut hkx);

        assert_eq!(hkx.objects()[0].members[0].value, HkxValue::I32(2));
    }

    #[test]
    fn fix_polytope_dispatch_type_rewrites_composite_to_convex() {
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2014.1.0-r1",
            vec![object(
                "hknpConvexPolytopeShape",
                vec![member("dispatchType", HkxValue::U8(2))],
            )],
        );

        fix_polytope_dispatch_type(&mut hkx);

        assert_eq!(hkx.objects()[0].members[0].value, HkxValue::U8(1));
    }

    #[test]
    fn fix_polytope_dispatch_type_is_idempotent_when_already_convex() {
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2014.1.0-r1",
            vec![object(
                "hknpConvexPolytopeShape",
                vec![member("dispatchType", HkxValue::I32(1))],
            )],
        );

        fix_polytope_dispatch_type(&mut hkx);

        assert_eq!(hkx.objects()[0].members[0].value, HkxValue::I32(1));
    }

    #[test]
    fn fix_compressed_mesh_shape_headers_rewrites_fo76_values() {
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2014.1.0-r1",
            vec![object(
                "hknpCompressedMeshShape",
                vec![
                    member("flags", HkxValue::U16(4)),
                    member("numShapeKeyBits", HkxValue::U8(7)),
                    member("dispatchType", HkxValue::U8(3)),
                ],
            )],
        );

        fix_compressed_mesh_shape_headers(&mut hkx);

        assert_eq!(
            hkx.objects()[0].members,
            vec![
                member("flags", HkxValue::U16(0x0204)),
                member("numShapeKeyBits", HkxValue::U8(7)),
                member("dispatchType", HkxValue::U8(2)),
            ]
        );
    }

    #[test]
    fn apply_implemented_transforms_replaces_empty_collision_shape_profiles_with_capsule_shape_setup()
     {
        let mut hkx = HkxFile::from_tagxml(
            12,
            "hk_2015.1.0-r1",
            vec![character_data_with_rigid_body_setup(vec![member(
                "collisionShapeProfiles",
                HkxValue::Array(vec![]),
            )])],
        );

        let last_applied = apply_for_test(&mut hkx);

        assert_eq!(
            ALWAYS_ON_TRANSFORMS[last_applied],
            "_fix_behavior_variable_infos"
        );
        let rb_members = rigid_body_setup_members(&hkx);
        assert!(
            rb_members
                .iter()
                .all(|member| member.name != "collisionShapeProfiles")
        );
        let shape_setup = rb_members
            .iter()
            .find(|member| member.name == "shapeSetup")
            .expect("shapeSetup should be injected");
        let HkxValue::Object(shape_members) = &shape_setup.value else {
            panic!("shapeSetup should be an inline object");
        };
        assert_eq!(
            shape_member_value(shape_members, "class"),
            Some("hkbShapeSetup")
        );
        assert_eq!(shape_member_f32(shape_members, "capsuleHeight"), Some(1.7));
        assert_eq!(shape_member_f32(shape_members, "capsuleRadius"), Some(0.4));
        assert_eq!(shape_member_value(shape_members, "fileName"), Some(""));
        assert_eq!(shape_member_value(shape_members, "type"), Some("CAPSULE"));
    }

    #[test]
    fn apply_implemented_transforms_does_not_overwrite_existing_or_non_empty_shape_setup() {
        let existing_shape = HkxValue::Object(vec![
            member("class", string("hkbShapeSetup")),
            member("capsuleHeight", HkxValue::F32(9.0)),
        ]);
        let mut with_existing = HkxFile::from_tagxml(
            12,
            "hk_2015.1.0-r1",
            vec![character_data_with_rigid_body_setup(vec![
                member("shapeSetup", existing_shape.clone()),
                member("collisionShapeProfiles", HkxValue::Array(vec![])),
            ])],
        );
        let mut non_empty_profiles = HkxFile::from_tagxml(
            12,
            "hk_2015.1.0-r1",
            vec![character_data_with_rigid_body_setup(vec![member(
                "collisionShapeProfiles",
                HkxValue::Array(vec![HkxValue::I32(7)]),
            )])],
        );

        let existing_last = apply_for_test(&mut with_existing);
        let non_empty_last = apply_for_test(&mut non_empty_profiles);

        assert_eq!(
            ALWAYS_ON_TRANSFORMS[existing_last],
            "_fix_behavior_variable_infos"
        );
        assert_eq!(
            ALWAYS_ON_TRANSFORMS[non_empty_last],
            "_fix_behavior_variable_infos"
        );
        let existing_members = rigid_body_setup_members(&with_existing);
        assert_eq!(
            existing_members
                .iter()
                .filter(|member| member.name == "shapeSetup")
                .count(),
            1
        );
        assert_eq!(
            existing_members
                .iter()
                .find(|member| member.name == "shapeSetup")
                .map(|member| &member.value),
            Some(&existing_shape)
        );
        let non_empty_members = rigid_body_setup_members(&non_empty_profiles);
        assert!(
            non_empty_members
                .iter()
                .all(|member| member.name != "shapeSetup")
        );
        assert_eq!(
            non_empty_members
                .iter()
                .find(|member| member.name == "collisionShapeProfiles")
                .map(|member| &member.value),
            Some(&HkxValue::Array(vec![HkxValue::I32(7)]))
        );
    }

    #[test]
    fn apply_implemented_transforms_stops_before_physics_noops_for_physics_payloads() {
        // All cases run to the behavior tail (41): transforms 19–21 are no-ops on
        // this minimal fixture (no hknpRagdollData/hknpPhysicsSystemData), and
        // nothing else halts the pipeline.
        struct Case {
            class_name: &'static str,
            stops_at: usize,
        }
        let cases = [
            Case {
                class_name: "hkcdSimdTreeNode",
                stops_at: 41,
            },
            Case {
                class_name: "hkBitField",
                stops_at: 41,
            },
            Case {
                class_name: "hkCompressedMassProperties",
                stops_at: 41,
            },
            Case {
                class_name: "hkpLimitedHingeConstraintData",
                stops_at: 41,
            },
        ];
        for case in &cases {
            let mut hkx = HkxFile::from_tagxml(
                12,
                "hk_2015.1.0-r1",
                vec![
                    object("hkbCharacterData", vec![]),
                    object(case.class_name, vec![]),
                ],
            );

            let last_applied = apply_for_test(&mut hkx);

            assert_eq!(
                last_applied, case.stops_at,
                "{}: pipeline last_applied mismatch",
                case.class_name
            );
        }
    }

    fn character_data_with_rigid_body_setup(rigid_body_members: Vec<HkxMember>) -> HkxObject {
        object(
            "hkbCharacterData",
            vec![member(
                "characterControllerSetup",
                HkxValue::Object(vec![member(
                    "rigidBodySetup",
                    HkxValue::Object(rigid_body_members),
                )]),
            )],
        )
    }

    fn rigid_body_setup_members(hkx: &HkxFile) -> &[HkxMember] {
        let HkxValue::Object(controller_members) = &hkx.objects()[0].members[0].value else {
            panic!("characterControllerSetup should be an inline object");
        };
        let HkxValue::Object(rigid_body_members) = &controller_members[0].value else {
            panic!("rigidBodySetup should be an inline object");
        };
        rigid_body_members
    }

    fn shape_member_value<'a>(members: &'a [HkxMember], name: &str) -> Option<&'a str> {
        members
            .iter()
            .find(|member| member.name == name)
            .and_then(|member| string_value(&member.value))
    }

    fn shape_member_f32(members: &[HkxMember], name: &str) -> Option<f32> {
        members
            .iter()
            .find(|member| member.name == name)
            .and_then(|member| match member.value {
                HkxValue::F32(value) => Some(value),
                _ => None,
            })
    }

    #[test]
    fn synthesize_memory_resource_container_adds_container_for_animation_files() {
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2014.1.0-r1",
            vec![
                HkxObject {
                    name: Some("#0004".to_string()),
                    ..object(
                        "hkRootLevelContainer",
                        vec![member("namedVariants", HkxValue::Array(vec![]))],
                    )
                },
                HkxObject {
                    name: Some("#0010".to_string()),
                    ..object("hkaAnimationBinding", vec![])
                },
            ],
        );

        synthesize_memory_resource_container(&mut hkx);

        assert_eq!(hkx.objects().len(), 3);
        let container = &hkx.objects()[2];
        assert_eq!(container.name.as_deref(), Some("#0011"));
        assert_eq!(container.class_name, "hkMemoryResourceContainer");
        assert_eq!(container.signature, 1);
        assert_eq!(
            container.members,
            vec![
                member("resourceHandles", HkxValue::Array(vec![])),
                member("externalLinks", HkxValue::Array(vec![])),
                member("name", string("")),
            ]
        );
        let HkxValue::Array(variants) = &hkx.objects()[0].members[0].value else {
            panic!("namedVariants should be an array");
        };
        assert_eq!(
            variants,
            &vec![named_variant(
                "Resource Data",
                "hkMemoryResourceContainer",
                Some(2)
            )]
        );
    }

    #[test]
    fn synthesize_memory_resource_container_is_idempotent() {
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2014.1.0-r1",
            vec![
                object(
                    "hkRootLevelContainer",
                    vec![member("namedVariants", HkxValue::Array(vec![]))],
                ),
                object("hkaAnimationBinding", vec![]),
                object("hkMemoryResourceContainer", vec![]),
            ],
        );

        synthesize_memory_resource_container(&mut hkx);

        assert_eq!(hkx.objects().len(), 3);
        let containers = hkx
            .objects()
            .iter()
            .filter(|object| object.class_name == "hkMemoryResourceContainer")
            .count();
        assert_eq!(containers, 1);
        assert_eq!(hkx.objects()[0].members[0].value, HkxValue::Array(vec![]));
    }

    #[test]
    fn synthesize_memory_resource_container_skips_non_array_named_variants() {
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2014.1.0-r1",
            vec![
                object(
                    "hkRootLevelContainer",
                    vec![
                        member("namedVariants", HkxValue::I32(7)),
                        member("namedVariants", HkxValue::Array(vec![])),
                    ],
                ),
                object("hkaAnimationBinding", vec![]),
            ],
        );

        synthesize_memory_resource_container(&mut hkx);

        assert_eq!(hkx.objects().len(), 3);
        assert_eq!(hkx.objects()[2].class_name, "hkMemoryResourceContainer");
        assert_eq!(hkx.objects()[0].members[0].value, HkxValue::I32(7));
        let HkxValue::Array(variants) = &hkx.objects()[0].members[1].value else {
            panic!("second namedVariants should be an array");
        };
        assert_eq!(
            variants,
            &vec![named_variant(
                "Resource Data",
                "hkMemoryResourceContainer",
                Some(2)
            )]
        );
    }

    #[test]
    fn synthesize_memory_resource_container_does_not_orphan_without_named_variants_array() {
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2014.1.0-r1",
            vec![
                object(
                    "hkRootLevelContainer",
                    vec![member("namedVariants", HkxValue::I32(7))],
                ),
                object("hkaAnimationBinding", vec![]),
            ],
        );

        synthesize_memory_resource_container(&mut hkx);

        assert_eq!(hkx.objects().len(), 2);
        assert!(
            !hkx.objects()
                .iter()
                .any(|object| object.class_name == "hkMemoryResourceContainer")
        );
        assert_eq!(hkx.objects()[0].members[0].value, HkxValue::I32(7));
    }

    #[test]
    fn synthesize_memory_resource_container_keeps_existing_container_parity() {
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2014.1.0-r1",
            vec![
                object(
                    "hkRootLevelContainer",
                    vec![member(
                        "namedVariants",
                        HkxValue::Array(vec![named_variant(
                            "Animation Container",
                            "hkaAnimationContainer",
                            Some(1),
                        )]),
                    )],
                ),
                object("hkaAnimationContainer", vec![]),
                object("hkaAnimationBinding", vec![]),
                object("hkMemoryResourceContainer", vec![]),
            ],
        );

        synthesize_memory_resource_container(&mut hkx);

        assert_eq!(hkx.objects().len(), 4);
        let containers = hkx
            .objects()
            .iter()
            .filter(|object| object.class_name == "hkMemoryResourceContainer")
            .count();
        assert_eq!(containers, 1);
        assert_eq!(
            hkx.objects()[0].members[0].value,
            HkxValue::Array(vec![named_variant(
                "Animation Container",
                "hkaAnimationContainer",
                Some(1),
            )])
        );
    }

    #[test]
    fn synthesize_memory_resource_container_skips_non_animation_files() {
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2014.1.0-r1",
            vec![object(
                "hkRootLevelContainer",
                vec![member("namedVariants", HkxValue::Array(vec![]))],
            )],
        );

        synthesize_memory_resource_container(&mut hkx);

        assert_eq!(hkx.objects().len(), 1);
        assert_eq!(hkx.objects()[0].members[0].value, HkxValue::Array(vec![]));
    }

    #[test]
    fn flatten_nested_class_names_renames_top_level_known_classes() {
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2014.1.0-r1",
            vec![
                object("hkbStateMachine::StateInfo", vec![]),
                object("hkbStateMachine::TransitionInfoArray", vec![]),
                object("hkbStateMachine::EventPropertyArray", vec![]),
                object("hkbStateMachine::TransitionInfo", vec![]),
                object("hkbStateMachine::TimeInterval", vec![]),
                object("hkbVariableBindingSet::Binding", vec![]),
                object("hkRootLevelContainer::NamedVariant", vec![]),
                object("hkbHandIkControlsModifier::Hand", vec![]),
                object("hkRootLevelContainer", vec![]),
            ],
        );

        flatten_nested_class_names(&mut hkx);

        let names: Vec<&str> = hkx
            .objects()
            .iter()
            .map(|o| o.class_name.as_str())
            .collect();
        assert_eq!(
            names,
            vec![
                "hkbStateMachineStateInfo",
                "hkbStateMachineTransitionInfoArray",
                "hkbStateMachineEventPropertyArray",
                "hkbStateMachineTransitionInfo",
                "hkbStateMachineTimeInterval",
                "hkbVariableBindingSetBinding",
                "hkRootLevelContainerNamedVariant",
                "hkbHandIkControlsModifierHand",
                "hkRootLevelContainer",
            ]
        );
    }

    #[test]
    fn flatten_nested_class_names_does_not_recurse_into_inline_objects() {
        let nested_named_variant = HkxValue::Array(vec![HkxValue::Object(vec![
            member("name", string("Animation Container")),
            member("className", string("hkbStateMachine::StateInfo")),
            member("variant", HkxValue::Pointer(Some(1))),
        ])]);
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2014.1.0-r1",
            vec![object(
                "hkRootLevelContainer",
                vec![member("namedVariants", nested_named_variant.clone())],
            )],
        );

        flatten_nested_class_names(&mut hkx);

        assert_eq!(hkx.objects()[0].class_name, "hkRootLevelContainer");
        assert_eq!(hkx.objects()[0].members[0].name, "namedVariants");
        assert_eq!(hkx.objects()[0].members[0].value, nested_named_variant);
    }

    #[test]
    fn reclassify_fo76_hkb_layer_handles_int_off_event_id_sentinel_0xff() {
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2014.1.0-r1",
            vec![object(
                "hkbBoneWeightArray",
                vec![
                    member("generator", HkxValue::Pointer(Some(0))),
                    member(
                        "blendingControlData",
                        HkxValue::Object(vec![
                            member("onEventId", HkxValue::U8(0xFF)),
                            member("offEventId", HkxValue::I32(7)),
                        ]),
                    ),
                ],
            )],
        );

        reclassify_fo76_hkb_layer(&mut hkx);

        let obj = &hkx.objects()[0];
        assert_eq!(obj.class_name, "hkbLayer");
        assert_eq!(
            obj.members
                .iter()
                .find(|m| m.name == "onEventId")
                .map(|m| &m.value),
            Some(&HkxValue::I32(-1))
        );
        assert_eq!(
            obj.members
                .iter()
                .find(|m| m.name == "offEventId")
                .map(|m| &m.value),
            Some(&HkxValue::I32(7))
        );
    }

    #[test]
    fn bool_or_int_to_event_sentinel_handles_signed_minus_one_sentinel() {
        // i8/i16 carry the "none" sentinel as -1 (the signed representation of 0xFF).
        // The cast-to-i64 comparison is unreachable for signed types; check *v == -1.
        assert_eq!(bool_or_int_to_event_sentinel(&HkxValue::I8(-1)), Some(-1));
        assert_eq!(bool_or_int_to_event_sentinel(&HkxValue::I16(-1)), Some(-1));
        // Non-sentinel values pass through unchanged.
        assert_eq!(bool_or_int_to_event_sentinel(&HkxValue::I8(7)), Some(7));
        assert_eq!(bool_or_int_to_event_sentinel(&HkxValue::I16(7)), Some(7));
        // Unsigned 0xFF still maps to -1 (the sentinel is the bit pattern, not sign).
        assert_eq!(bool_or_int_to_event_sentinel(&HkxValue::U8(0xFF)), Some(-1));
    }

    #[test]
    fn reclassify_fo76_hkb_layer_handles_missing_blending_control_data() {
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2014.1.0-r1",
            vec![object(
                "hkbBoneWeightArray",
                vec![
                    member("generator", HkxValue::Pointer(Some(0))),
                    member("name", string("LeftArm")),
                ],
            )],
        );

        reclassify_fo76_hkb_layer(&mut hkx);

        let obj = &hkx.objects()[0];
        assert_eq!(obj.class_name, "hkbLayer");
        assert_eq!(obj.members.len(), 2);
        assert_eq!(obj.members[0].name, "generator");
        assert_eq!(obj.members[0].value, HkxValue::Pointer(Some(0)));
        assert_eq!(obj.members[1].name, "name");
        assert_eq!(obj.members[1].value, string("LeftArm"));
    }

    #[test]
    fn reclassify_fo76_hkb_layer_skips_real_bone_weight_array_without_generator() {
        let bone_weights = HkxValue::Array(vec![HkxValue::F32(1.0), HkxValue::F32(0.5)]);
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2014.1.0-r1",
            vec![object(
                "hkbBoneWeightArray",
                vec![member("boneWeights", bone_weights.clone())],
            )],
        );

        reclassify_fo76_hkb_layer(&mut hkx);

        let obj = &hkx.objects()[0];
        assert_eq!(obj.class_name, "hkbBoneWeightArray");
        assert_eq!(obj.members.len(), 1);
        assert_eq!(obj.members[0].name, "boneWeights");
        assert_eq!(obj.members[0].value, bone_weights);
    }

    #[test]
    fn reclassify_fo76_hkb_layer_renames_and_flattens_blending_control_data() {
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2014.1.0-r1",
            vec![object(
                "hkbBoneWeightArray",
                vec![
                    member("generator", HkxValue::Pointer(Some(0))),
                    member(
                        "blendingControlData",
                        HkxValue::Object(vec![
                            member("weight", HkxValue::F32(0.5)),
                            member("fadeInDuration", HkxValue::F32(0.1)),
                            member("fadeOutDuration", HkxValue::F32(0.2)),
                            member("onEventId", HkxValue::Bool(true)),
                            member("offEventId", HkxValue::Bool(false)),
                            member("onByDefault", HkxValue::Bool(true)),
                            member("forceFullFadeDurations", HkxValue::Bool(false)),
                            member("internalState", HkxValue::Bool(false)),
                            member("fadeInOutCurve", HkxValue::I32(0)),
                        ]),
                    ),
                ],
            )],
        );

        reclassify_fo76_hkb_layer(&mut hkx);

        let obj = &hkx.objects()[0];
        assert_eq!(obj.class_name, "hkbLayer");
        assert!(
            obj.members.iter().all(|m| m.name != "blendingControlData"),
            "blendingControlData should be removed",
        );
        let names: Vec<&str> = obj.members.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "generator",
                "weight",
                "fadeInDuration",
                "fadeOutDuration",
                "onEventId",
                "offEventId",
                "onByDefault",
                "forceFullFadeDurations",
            ],
        );
        assert_eq!(
            obj.members
                .iter()
                .find(|m| m.name == "weight")
                .map(|m| &m.value),
            Some(&HkxValue::F32(0.5))
        );
        assert_eq!(
            obj.members
                .iter()
                .find(|m| m.name == "onEventId")
                .map(|m| &m.value),
            Some(&HkxValue::I32(-1))
        );
        assert_eq!(
            obj.members
                .iter()
                .find(|m| m.name == "offEventId")
                .map(|m| &m.value),
            Some(&HkxValue::I32(0))
        );
        assert_eq!(
            obj.members
                .iter()
                .find(|m| m.name == "onByDefault")
                .map(|m| &m.value),
            Some(&HkxValue::Bool(true))
        );
        assert_eq!(
            obj.members
                .iter()
                .find(|m| m.name == "forceFullFadeDurations")
                .map(|m| &m.value),
            Some(&HkxValue::Bool(false))
        );
    }

    #[test]
    fn reclassify_fo76_hkb_layer_updates_only_its_binding_paths() {
        fn bindings() -> HkxObject {
            object(
                "hkbVariableBindingSet",
                vec![member(
                    "bindings",
                    HkxValue::Array(
                        [
                            "blendingControlData/weight",
                            "blendingControlData/onByDefault",
                            "boneWeights",
                        ]
                        .into_iter()
                        .enumerate()
                        .map(|(index, path)| {
                            HkxValue::Object(vec![
                                member(
                                    "memberPath",
                                    HkxValue::String {
                                        value: path.to_string(),
                                        is_null: false,
                                    },
                                ),
                                member("variableIndex", HkxValue::I32(index as i32)),
                            ])
                        })
                        .collect(),
                    ),
                )],
            )
        }
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2015.1.0-r1",
            vec![
                object(
                    "hkbLayer",
                    vec![
                        member("generator", HkxValue::Pointer(None)),
                        member("variableBindingSet", HkxValue::Pointer(Some(1))),
                        member(
                            "blendingControlData",
                            HkxValue::Object(vec![member("weight", HkxValue::F32(1.0))]),
                        ),
                    ],
                ),
                bindings(),
                object(
                    "hkbModifier",
                    vec![member("variableBindingSet", HkxValue::Pointer(Some(3)))],
                ),
                bindings(),
            ],
        );
        let unrelated = hkx.objects()[3].members.clone();
        reclassify_fo76_hkb_layer(&mut hkx);
        let HkxValue::Array(entries) = &hkx.objects()[1].members[0].value else {
            panic!()
        };
        for (index, expected) in ["weight", "onByDefault", "boneWeights"]
            .into_iter()
            .enumerate()
        {
            let HkxValue::Object(members) = &entries[index] else {
                panic!()
            };
            assert_eq!(string_member_value(members, "memberPath"), Some(expected));
            assert_eq!(members[1].value, HkxValue::I32(index as i32));
        }
        assert_eq!(hkx.objects()[3].members, unrelated);
        reclassify_fo76_hkb_layer(&mut hkx);
        let HkxValue::Array(entries) = &hkx.objects()[1].members[0].value else {
            panic!()
        };
        let HkxValue::Object(members) = &entries[0] else {
            panic!()
        };
        assert_eq!(string_member_value(members, "memberPath"), Some("weight"));
    }

    #[test]
    fn reclassify_fo76_hkb_layer_preserves_each_real_layer_enabled_state() {
        fn layer(generator: usize, on_by_default: bool) -> HkxObject {
            object(
                "hkbLayer",
                vec![
                    member("generator", HkxValue::Pointer(Some(generator))),
                    member(
                        "blendingControlData",
                        HkxValue::Object(vec![member(
                            "onByDefault",
                            HkxValue::Bool(on_by_default),
                        )]),
                    ),
                ],
            )
        }

        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2015.1.0-r1",
            vec![
                object(
                    "hkbLayerGenerator",
                    vec![member(
                        "layers",
                        HkxValue::Array(vec![
                            HkxValue::Pointer(Some(1)),
                            HkxValue::Pointer(Some(2)),
                            HkxValue::Pointer(Some(3)),
                        ]),
                    )],
                ),
                layer(4, true),
                layer(5, true),
                layer(6, false),
                object("hkbStateMachine", vec![]),
                object("hkbStateMachine", vec![]),
                object("hkbStateMachine", vec![]),
            ],
        );

        reclassify_fo76_hkb_layer(&mut hkx);
        apply_classxml_defaults(&mut hkx);

        for (layer_index, expected) in [(1, true), (2, true), (3, false)] {
            let layer = &hkx.objects()[layer_index];
            assert_eq!(layer.signature, 1);
            assert!(
                layer
                    .members
                    .iter()
                    .all(|member| member.name != "blendingControlData")
            );
            assert_eq!(
                layer
                    .members
                    .iter()
                    .find(|member| member.name == "onByDefault")
                    .map(|member| &member.value),
                Some(&HkxValue::Bool(expected))
            );
        }
    }

    // ── strip_runtime_members tests ──────────────────────────────────────────

    #[test]
    fn strip_runtime_members_drops_runtime_only_members() {
        // hkbBehaviorGraph has SERIALIZE_IGNORED members like `uniqueIdPool`,
        // `idToStateMachineTemplateMap`, `pseudoRandomGenerator`, etc.
        // and serializable members `variableMode`, `rootGenerator`, `data`.
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2014.1.0-r1",
            vec![object(
                "hkbBehaviorGraph",
                vec![
                    member("variableMode", HkxValue::I32(0)),
                    member("rootGenerator", HkxValue::Pointer(Some(0))),
                    member("data", HkxValue::Pointer(Some(0))),
                    // SERIALIZE_IGNORED — should be stripped
                    member("uniqueIdPool", HkxValue::Array(vec![])),
                    member("idToStateMachineTemplateMap", HkxValue::Pointer(None)),
                    member("pseudoRandomGenerator", HkxValue::Pointer(None)),
                    member("isActive", HkxValue::Bool(false)),
                ],
            )],
        );
        let mut registry = DescriptorRegistry::new();

        strip_runtime_members(&mut hkx, &mut registry);

        let obj = &hkx.objects()[0];
        let names: Vec<&str> = obj.members.iter().map(|m| m.name.as_str()).collect();
        assert!(
            names.contains(&"variableMode"),
            "variableMode should be kept"
        );
        assert!(
            names.contains(&"rootGenerator"),
            "rootGenerator should be kept"
        );
        assert!(names.contains(&"data"), "data should be kept");
        assert!(
            !names.contains(&"uniqueIdPool"),
            "uniqueIdPool is SERIALIZE_IGNORED and should be stripped"
        );
        assert!(
            !names.contains(&"idToStateMachineTemplateMap"),
            "idToStateMachineTemplateMap is SERIALIZE_IGNORED and should be stripped"
        );
        assert!(
            !names.contains(&"pseudoRandomGenerator"),
            "pseudoRandomGenerator is SERIALIZE_IGNORED and should be stripped"
        );
        assert!(
            !names.contains(&"isActive"),
            "isActive is SERIALIZE_IGNORED and should be stripped"
        );
    }

    #[test]
    fn strip_runtime_members_injects_missing_serializable_defaults() {
        // hkbBehaviorGraph serializable: variableMode (enum/i8), rootGenerator (pointer),
        // data (pointer). Start with an empty object — all should be injected.
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2014.1.0-r1",
            vec![object("hkbBehaviorGraph", vec![])],
        );
        let mut registry = DescriptorRegistry::new();

        strip_runtime_members(&mut hkx, &mut registry);

        let obj = &hkx.objects()[0];
        let root_gen = obj.members.iter().find(|m| m.name == "rootGenerator");
        let data_m = obj.members.iter().find(|m| m.name == "data");
        assert_eq!(
            root_gen.map(|m| &m.value),
            Some(&HkxValue::Pointer(None)),
            "missing pointer member should be injected as Pointer(None)"
        );
        assert_eq!(
            data_m.map(|m| &m.value),
            Some(&HkxValue::Pointer(None)),
            "missing pointer member should be injected as Pointer(None)"
        );
    }

    #[test]
    fn strip_runtime_members_retypes_pointer_field_from_direct_zero() {
        // rootGenerator has vtype=TYPE_POINTER in classxml; if the existing value is I32(0),
        // it should be retyped to Pointer(None).
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2014.1.0-r1",
            vec![object(
                "hkbBehaviorGraph",
                vec![
                    member("rootGenerator", HkxValue::I32(0)),
                    member("data", HkxValue::I32(0)),
                ],
            )],
        );
        let mut registry = DescriptorRegistry::new();

        strip_runtime_members(&mut hkx, &mut registry);

        let obj = &hkx.objects()[0];
        let root_gen = obj.members.iter().find(|m| m.name == "rootGenerator");
        assert_eq!(
            root_gen.map(|m| &m.value),
            Some(&HkxValue::Pointer(None)),
            "I32(0) pointer field should be retyped to Pointer(None)"
        );
    }

    #[test]
    fn strip_runtime_members_leaves_unknown_class_untouched() {
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2014.1.0-r1",
            vec![object(
                "hkUnknownNonExistent",
                vec![member("runtimeJunk", HkxValue::I32(42))],
            )],
        );
        let mut registry = DescriptorRegistry::new();

        strip_runtime_members(&mut hkx, &mut registry);

        // Unknown class — members left untouched
        let obj = &hkx.objects()[0];
        assert_eq!(obj.members.len(), 1);
        assert_eq!(obj.members[0].name, "runtimeJunk");
        assert_eq!(obj.members[0].value, HkxValue::I32(42));
    }

    #[test]
    fn strip_runtime_members_recurses_into_inline_struct_arrays() {
        // hkbStateMachine has member `states` (vtype=TYPE_ARRAY, ctype=hkbStateMachineStateInfo).
        // hkbStateMachineStateInfo has SERIALIZE_IGNORED `hasEventlessTransitions`.
        // We put that member in an inline struct entry; verify it gets stripped.
        let inline_state = HkxValue::Object(vec![
            member("stateId", HkxValue::I32(0)),
            member(
                "name",
                HkxValue::String {
                    value: "Idle".to_string(),
                    is_null: false,
                },
            ),
            member("enable", HkxValue::Bool(true)),
            // SERIALIZE_IGNORED — should be stripped from inline struct
            member("hasEventlessTransitions", HkxValue::Bool(false)),
        ]);
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2014.1.0-r1",
            vec![object(
                "hkbStateMachine",
                vec![member("states", HkxValue::Array(vec![inline_state]))],
            )],
        );
        let mut registry = DescriptorRegistry::new();

        strip_runtime_members(&mut hkx, &mut registry);

        let obj = &hkx.objects()[0];
        let states_member = obj.members.iter().find(|m| m.name == "states").unwrap();
        let HkxValue::Array(states) = &states_member.value else {
            panic!("states should be an array");
        };
        assert_eq!(states.len(), 1, "inline struct entry should be preserved");
        let HkxValue::Object(state_members) = &states[0] else {
            panic!("inline struct entry should be an object");
        };
        let member_names: Vec<&str> = state_members.iter().map(|m| m.name.as_str()).collect();
        assert!(
            !member_names.contains(&"hasEventlessTransitions"),
            "SERIALIZE_IGNORED member should be stripped from inline struct"
        );
        assert!(member_names.contains(&"stateId"), "stateId should be kept");
    }

    #[test]
    fn strip_runtime_members_flattens_hknp_constraint_body_handles() {
        let constraint = HkxValue::Object(vec![
            member("constraintData", HkxValue::Pointer(Some(1))),
            member(
                "bodyA",
                HkxValue::Object(vec![member("serialAndIndex", HkxValue::U32(7))]),
            ),
            member(
                "bodyB",
                HkxValue::Object(vec![member("serialAndIndex", HkxValue::U32(3))]),
            ),
            member("flags", HkxValue::U8(0)),
        ]);
        let mut hkx = HkxFile::from_tagxml(
            12,
            "hk_2015.1.0-r1",
            vec![object(
                "hknpRagdollData",
                vec![member(
                    "constraintCinfos",
                    HkxValue::Array(vec![constraint]),
                )],
            )],
        );
        let mut registry = DescriptorRegistry::new();

        strip_runtime_members(&mut hkx, &mut registry);

        let constraints = hkx.objects()[0]
            .members
            .iter()
            .find(|m| m.name == "constraintCinfos")
            .unwrap();
        let HkxValue::Array(items) = &constraints.value else {
            panic!("constraintCinfos should be an array");
        };
        let HkxValue::Object(members) = &items[0] else {
            panic!("constraint entry should be an object");
        };
        assert_eq!(
            members.iter().find(|m| m.name == "bodyA").unwrap().value,
            HkxValue::U32(7)
        );
        assert_eq!(
            members.iter().find(|m| m.name == "bodyB").unwrap().value,
            HkxValue::U32(3)
        );
    }

    // ── Behavior-graph cleanup transforms (28..=34) ───────────────────────────

    #[test]
    fn compact_null_blender_children_drops_null_pointers_in_children() {
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2014.1.0-r1",
            vec![
                object("hkbBlenderGeneratorChild", vec![]),
                object(
                    "hkbBlenderGenerator",
                    vec![member(
                        "children",
                        HkxValue::Array(vec![
                            HkxValue::Pointer(Some(0)),
                            HkxValue::Pointer(None),
                            HkxValue::Pointer(Some(0)),
                            HkxValue::Pointer(None),
                        ]),
                    )],
                ),
                // Unrelated class — must be untouched.
                object(
                    "hkbManualSelectorGenerator",
                    vec![member(
                        "children",
                        HkxValue::Array(vec![HkxValue::Pointer(None)]),
                    )],
                ),
            ],
        );

        compact_null_blender_children(&mut hkx);

        let blender = &hkx.objects()[1];
        let HkxValue::Array(children) = &blender.members[0].value else {
            panic!("children should be an array");
        };
        assert_eq!(children.len(), 2, "null slots should be dropped");
        assert!(
            children
                .iter()
                .all(|v| matches!(v, HkxValue::Pointer(Some(_))))
        );

        // hkbManualSelectorGenerator.children should be untouched by this transform.
        let unrelated = &hkx.objects()[2];
        let HkxValue::Array(unrelated_children) = &unrelated.members[0].value else {
            panic!("children should be an array");
        };
        assert_eq!(
            unrelated_children.len(),
            1,
            "unrelated class should be untouched"
        );
    }

    #[test]
    fn compact_null_blender_children_compacts_every_generator_not_just_first() {
        // Every blender/pose-matching generator must be compacted, not just the
        // first. FO4 walks the packed `children` array by count and derefs each
        // slot, so a null child CTDs the CK/runtime (the converted Scorched
        // `ScorchedRootBehavior.hkx` has 4 such generators).
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2014.1.0-r1",
            vec![
                object("hkbBlenderGeneratorChild", vec![]),
                // First blender: already clean — the old `break` "succeeded" here.
                object(
                    "hkbBlenderGenerator",
                    vec![member(
                        "children",
                        HkxValue::Array(vec![HkxValue::Pointer(Some(0))]),
                    )],
                ),
                // Second blender: has a null — must STILL be compacted.
                object(
                    "hkbBlenderGenerator",
                    vec![member(
                        "children",
                        HkxValue::Array(vec![HkxValue::Pointer(Some(0)), HkxValue::Pointer(None)]),
                    )],
                ),
                // Pose-matching generator (the Scorched crash class): also compacted.
                object(
                    "hkbPoseMatchingGenerator",
                    vec![member(
                        "children",
                        HkxValue::Array(vec![HkxValue::Pointer(None), HkxValue::Pointer(Some(0))]),
                    )],
                ),
            ],
        );

        compact_null_blender_children(&mut hkx);

        for idx in [2usize, 3usize] {
            let HkxValue::Array(children) = &hkx.objects()[idx].members[0].value else {
                panic!("children should be an array");
            };
            assert_eq!(
                children.len(),
                1,
                "generator {idx} should have its null dropped"
            );
            assert!(
                children
                    .iter()
                    .all(|v| matches!(v, HkxValue::Pointer(Some(_)))),
                "generator {idx} must have null children compacted",
            );
        }
    }

    #[test]
    fn simplify_compound_ragdoll_body_shapes_repoints_to_first_leaf() {
        // A ragdoll body backed by an hknpDynamicCompoundShape (FO4 can't
        // instantiate it → AddToWorld null-deref CTD) must be repointed to the
        // compound's first instance leaf, keeping exactly one body for the bone.
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2014.1.0-r1",
            vec![
                object("hknpCapsuleShape", vec![]),        // 0 — leaf
                object("hknpConvexPolytopeShape", vec![]), // 1 — leaf
                object(
                    "hknpDynamicCompoundShape",
                    vec![member(
                        "instances",
                        HkxValue::Object(vec![member(
                            "elements",
                            HkxValue::Array(vec![
                                HkxValue::Object(vec![member("shape", HkxValue::Pointer(Some(0)))]),
                                HkxValue::Object(vec![member("shape", HkxValue::Pointer(Some(1)))]),
                            ]),
                        )]),
                    )],
                ), // 2 — compound
                object(
                    "hknpRagdollData",
                    vec![member(
                        "bodyCinfos",
                        HkxValue::Array(vec![HkxValue::Object(vec![member(
                            "shape",
                            HkxValue::Pointer(Some(2)),
                        )])]),
                    )],
                ), // 3 — ragdoll, body points at the compound
            ],
        );

        simplify_compound_ragdoll_body_shapes(&mut hkx);

        let HkxValue::Array(bodies) = &hkx.objects()[3].members[0].value else {
            panic!("bodyCinfos should be an array");
        };
        let HkxValue::Object(body0) = &bodies[0] else {
            panic!("body cinfo should be an object");
        };
        let shape = body0.iter().find(|m| m.name == "shape").unwrap();
        assert!(
            matches!(shape.value, HkxValue::Pointer(Some(0))),
            "compound-backed body must repoint to the first leaf (0), got {:?}",
            shape.value,
        );
    }

    #[test]
    fn compact_null_state_machine_states_drops_null_pointers_in_states() {
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2014.1.0-r1",
            vec![
                object("hkbStateMachineStateInfo", vec![]),
                object(
                    "hkbStateMachine",
                    vec![member(
                        "states",
                        HkxValue::Array(vec![
                            HkxValue::Pointer(Some(0)),
                            HkxValue::Pointer(None),
                            HkxValue::Pointer(Some(0)),
                        ]),
                    )],
                ),
                // Unrelated class — must be untouched.
                object(
                    "hkbBlenderGenerator",
                    vec![member(
                        "states",
                        HkxValue::Array(vec![HkxValue::Pointer(None)]),
                    )],
                ),
            ],
        );

        compact_null_state_machine_states(&mut hkx);

        let sm = &hkx.objects()[1];
        let HkxValue::Array(states) = &sm.members[0].value else {
            panic!("states should be an array");
        };
        assert_eq!(states.len(), 2, "null slots should be dropped");
        assert!(
            states
                .iter()
                .all(|v| matches!(v, HkxValue::Pointer(Some(_))))
        );

        // hkbBlenderGenerator.states should be untouched.
        let unrelated = &hkx.objects()[2];
        let HkxValue::Array(unrelated_states) = &unrelated.members[0].value else {
            panic!("states should be an array");
        };
        assert_eq!(
            unrelated_states.len(),
            1,
            "unrelated class should be untouched"
        );
    }

    fn state_info(name: &str, state_id: i32) -> HkxObject {
        object(
            "hkbStateMachineStateInfo",
            vec![
                member("name", string(name)),
                member("stateId", HkxValue::I32(state_id)),
            ],
        )
    }

    #[test]
    fn compact_null_state_machine_states_recovers_orphans_in_standalone_files() {
        // Standalone behavior file (has hkRootLevelContainer): a single SM
        // owning two states; two more StateInfo objects exist with matching
        // name affinity but are orphaned (not referenced). Null slots must
        // recover orphans, not be silently truncated.
        //
        // Object indices:
        //   0: hkRootLevelContainer (marks file as standalone)
        //   1: StateInfo "RunRoot"  (owned by SM at slot 0)
        //   2: StateInfo "WalkRoot" (owned by SM at slot 2)
        //   3: StateInfo "IdleRoot" (orphan — sibling-name match)
        //   4: StateInfo "JumpRoot" (orphan — sibling-name match)
        //   5: hkbStateMachine "Root_SM" with 4 slots, 2 null
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2014.1.0-r1",
            vec![
                object("hkRootLevelContainer", vec![]),
                state_info("RunRoot", 0),
                state_info("WalkRoot", 2),
                state_info("IdleRoot", 1),
                state_info("JumpRoot", 3),
                object(
                    "hkbStateMachine",
                    vec![
                        member("name", string("Root_SM")),
                        member(
                            "states",
                            HkxValue::Array(vec![
                                HkxValue::Pointer(Some(1)),
                                HkxValue::Pointer(None),
                                HkxValue::Pointer(Some(2)),
                                HkxValue::Pointer(None),
                            ]),
                        ),
                    ],
                ),
            ],
        );

        compact_null_state_machine_states(&mut hkx);

        let sm = &hkx.objects()[5];
        let states_member = sm.members.iter().find(|m| m.name == "states").unwrap();
        let HkxValue::Array(arr) = &states_member.value else {
            panic!("states should be an array");
        };

        assert_eq!(
            arr.len(),
            4,
            "all four orphan-but-named states recovered, not truncated"
        );
        let pointer_targets: Vec<usize> = arr
            .iter()
            .filter_map(|v| {
                if let HkxValue::Pointer(Some(i)) = v {
                    Some(*i)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(
            pointer_targets.len(),
            4,
            "every recovered slot must point at a real StateInfo",
        );
        let target_set: std::collections::HashSet<usize> =
            pointer_targets.iter().copied().collect();
        assert!(
            target_set.contains(&1) && target_set.contains(&2),
            "owned states preserved"
        );
        assert!(
            target_set.contains(&3) && target_set.contains(&4),
            "orphan StateInfos at indices 3/4 must be reattached, got {:?}",
            pointer_targets,
        );
    }

    #[test]
    fn compact_null_state_machine_states_orders_same_id_orphans_deterministically() {
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2014.1.0-r1",
            vec![
                object("hkRootLevelContainer", vec![]),
                state_info("OwnedRoot", 0),
                state_info("ZetaRoot", 1),
                state_info("AlphaRoot", 1),
                object(
                    "hkbStateMachine",
                    vec![
                        member("name", string("Root_SM")),
                        member(
                            "states",
                            HkxValue::Array(vec![
                                HkxValue::Pointer(Some(1)),
                                HkxValue::Pointer(None),
                                HkxValue::Pointer(None),
                            ]),
                        ),
                    ],
                ),
            ],
        );

        compact_null_state_machine_states(&mut hkx);

        let sm = &hkx.objects()[4];
        let states_member = sm.members.iter().find(|m| m.name == "states").unwrap();
        let HkxValue::Array(arr) = &states_member.value else {
            panic!("states should be an array");
        };
        let pointer_targets: Vec<usize> = arr
            .iter()
            .filter_map(|v| {
                if let HkxValue::Pointer(Some(i)) = v {
                    Some(*i)
                } else {
                    None
                }
            })
            .collect();

        assert_eq!(pointer_targets, vec![1, 3, 2]);
    }

    #[test]
    fn compact_null_state_machine_states_strips_nulls_for_nif_embedded_files() {
        // NIF-embedded behavior blob (no hkRootLevelContainer) with orphan
        // StateInfos must NOT recover them — strip nulls only.
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2014.1.0-r1",
            vec![
                state_info("Owned", 0),
                state_info("Orphan", 1),
                object(
                    "hkbStateMachine",
                    vec![
                        member("name", string("Embedded_SM")),
                        member(
                            "states",
                            HkxValue::Array(vec![
                                HkxValue::Pointer(Some(0)),
                                HkxValue::Pointer(None),
                            ]),
                        ),
                    ],
                ),
            ],
        );

        compact_null_state_machine_states(&mut hkx);

        let sm = &hkx.objects()[2];
        let HkxValue::Array(arr) = &sm
            .members
            .iter()
            .find(|m| m.name == "states")
            .unwrap()
            .value
        else {
            panic!("states should be an array");
        };
        assert_eq!(
            arr.len(),
            1,
            "nif-embedded path strips nulls without recovery"
        );
        assert!(matches!(arr[0], HkxValue::Pointer(Some(0))));
    }

    #[test]
    fn compact_null_pointer_arrays_drops_nulls_in_known_class_members() {
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2014.1.0-r1",
            vec![
                object("SomeLayer", vec![]),
                object(
                    "hkbLayerGenerator",
                    vec![member(
                        "layers",
                        HkxValue::Array(vec![HkxValue::Pointer(Some(0)), HkxValue::Pointer(None)]),
                    )],
                ),
                object(
                    "hkbModifierList",
                    vec![member(
                        "modifiers",
                        HkxValue::Array(vec![
                            HkxValue::Pointer(None),
                            HkxValue::Pointer(Some(0)),
                            HkxValue::Pointer(None),
                        ]),
                    )],
                ),
                // Unrelated class — untouched.
                object(
                    "hkbBehaviorGraph",
                    vec![member(
                        "modifiers",
                        HkxValue::Array(vec![HkxValue::Pointer(None)]),
                    )],
                ),
            ],
        );

        compact_null_pointer_arrays(&mut hkx);

        let layer_gen = &hkx.objects()[1];
        let HkxValue::Array(layers) = &layer_gen.members[0].value else {
            panic!("layers should be an array");
        };
        assert_eq!(layers.len(), 1);
        assert!(matches!(layers[0], HkxValue::Pointer(Some(_))));

        let mod_list = &hkx.objects()[2];
        let HkxValue::Array(modifiers) = &mod_list.members[0].value else {
            panic!("modifiers should be an array");
        };
        assert_eq!(modifiers.len(), 1);
        assert!(matches!(modifiers[0], HkxValue::Pointer(Some(_))));

        // hkbBehaviorGraph.modifiers should be untouched.
        let unrelated = &hkx.objects()[3];
        let HkxValue::Array(unrelated_mods) = &unrelated.members[0].value else {
            panic!("modifiers should be an array");
        };
        assert_eq!(
            unrelated_mods.len(),
            1,
            "unrelated class should be untouched"
        );
    }

    #[test]
    fn fix_state_machine_typed_refs_replaces_bool_with_negative_one_int32() {
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2014.1.0-r1",
            vec![
                object(
                    "hkbStateMachine",
                    vec![
                        member("returnToPreviousStateEventId", HkxValue::Bool(true)),
                        member("syncVariableIndex", HkxValue::Bool(false)),
                        // Already an int32 — must be left alone.
                        member("randomTransitionEventId", HkxValue::I32(5)),
                    ],
                ),
                // Unrelated class — untouched.
                object(
                    "hkbBlenderGenerator",
                    vec![member("returnToPreviousStateEventId", HkxValue::Bool(true))],
                ),
            ],
        );

        fix_state_machine_typed_refs(&mut hkx);

        let sm = &hkx.objects()[0];
        assert_eq!(
            sm.members
                .iter()
                .find(|m| m.name == "returnToPreviousStateEventId")
                .map(|m| &m.value),
            Some(&HkxValue::I32(-1))
        );
        assert_eq!(
            sm.members
                .iter()
                .find(|m| m.name == "syncVariableIndex")
                .map(|m| &m.value),
            Some(&HkxValue::I32(-1))
        );
        // Already-int32 value must not be mutated.
        assert_eq!(
            sm.members
                .iter()
                .find(|m| m.name == "randomTransitionEventId")
                .map(|m| &m.value),
            Some(&HkxValue::I32(5))
        );
        // Unrelated class: Bool value must not be changed.
        let unrelated = &hkx.objects()[1];
        assert_eq!(
            unrelated.members[0].value,
            HkxValue::Bool(true),
            "unrelated class should be untouched"
        );
    }

    #[test]
    fn fix_serialized_bool_to_int32_uses_negative_one_for_oob_index() {
        // Single-state SM: Bool(true) → raw=1, companion_len=1 → 1 >= 1 → sentinel -1.
        // Two-state SM: Bool(true) → raw=1, companion_len=2 → 1 < 2 → passthrough 1.
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2014.1.0-r1",
            vec![
                // 1 state: Bool(true) should become I32(-1)
                object(
                    "hkbStateMachine",
                    vec![
                        member("startStateId", HkxValue::Bool(true)),
                        member("states", HkxValue::Array(vec![HkxValue::Pointer(Some(0))])),
                    ],
                ),
                // 2 states: Bool(true) should become I32(1) (valid in-bounds index)
                object(
                    "hkbStateMachine",
                    vec![
                        member("startStateId", HkxValue::Bool(true)),
                        member(
                            "states",
                            HkxValue::Array(vec![
                                HkxValue::Pointer(Some(0)),
                                HkxValue::Pointer(Some(0)),
                            ]),
                        ),
                    ],
                ),
                // Already I32 — must be left alone.
                object(
                    "hkbStateMachine",
                    vec![
                        member("startStateId", HkxValue::I32(0)),
                        member("states", HkxValue::Array(vec![HkxValue::Pointer(Some(0))])),
                    ],
                ),
            ],
        );

        fix_serialized_bool_to_int32(&mut hkx);

        assert_eq!(
            hkx.objects()[0]
                .members
                .iter()
                .find(|m| m.name == "startStateId")
                .map(|m| &m.value),
            Some(&HkxValue::I32(-1)),
            "1-state SM Bool(true) should become I32(-1)"
        );
        assert_eq!(
            hkx.objects()[1]
                .members
                .iter()
                .find(|m| m.name == "startStateId")
                .map(|m| &m.value),
            Some(&HkxValue::I32(1)),
            "2-state SM Bool(true) should become I32(1) as a valid index"
        );
        assert_eq!(
            hkx.objects()[2]
                .members
                .iter()
                .find(|m| m.name == "startStateId")
                .map(|m| &m.value),
            Some(&HkxValue::I32(0)),
            "already-I32 startStateId should be left alone"
        );
    }

    #[test]
    fn drop_transitions_to_missing_states_removes_invalid_to_state_id_entries() {
        // SM has 1 state with stateId=0. Transition array has two entries:
        //   entry0 → toStateId=0 (valid — keep)
        //   entry1 → toStateId=5 (missing — drop)
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2014.1.0-r1",
            vec![
                // obj 0: hkbStateMachineStateInfo with stateId=0
                HkxObject {
                    name: Some("#0000".to_string()),
                    ..object(
                        "hkbStateMachineStateInfo",
                        vec![
                            member("stateId", HkxValue::I32(0)),
                            member("transitions", HkxValue::Pointer(Some(2))),
                        ],
                    )
                },
                // obj 1: hkbStateMachine with one state → obj 0
                HkxObject {
                    name: Some("#0001".to_string()),
                    ..object(
                        "hkbStateMachine",
                        vec![member(
                            "states",
                            HkxValue::Array(vec![HkxValue::Pointer(Some(0))]),
                        )],
                    )
                },
                // obj 2: transition array reachable via state info transitions pointer
                HkxObject {
                    name: Some("#0002".to_string()),
                    ..object(
                        "hkbStateMachineTransitionInfoArray",
                        vec![member(
                            "transitions",
                            HkxValue::Array(vec![
                                HkxValue::Object(vec![member("toStateId", HkxValue::I32(0))]),
                                HkxValue::Object(vec![member("toStateId", HkxValue::I32(5))]),
                            ]),
                        )],
                    )
                },
            ],
        );

        drop_transitions_to_missing_states(&mut hkx);

        let ta = &hkx.objects()[2];
        let HkxValue::Array(transitions) = &ta
            .members
            .iter()
            .find(|m| m.name == "transitions")
            .unwrap()
            .value
        else {
            panic!("transitions should be an array");
        };
        assert_eq!(
            transitions.len(),
            1,
            "only the valid toStateId=0 entry should remain"
        );
        let HkxValue::Object(entry_members) = &transitions[0] else {
            panic!("transition entry should be an object");
        };
        assert_eq!(
            entry_members
                .iter()
                .find(|m| m.name == "toStateId")
                .map(|m| &m.value),
            Some(&HkxValue::I32(0))
        );
    }

    #[test]
    fn fix_dangling_pointers_nulls_out_of_range_indices() {
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2014.1.0-r1",
            vec![
                object("ValidTarget", vec![]),
                object(
                    "Holder",
                    vec![
                        // Valid pointer — should stay.
                        member("valid", HkxValue::Pointer(Some(0))),
                        // Out-of-range — should become Pointer(None).
                        member("dangling", HkxValue::Pointer(Some(99))),
                        // Already null — should stay null.
                        member("already_null", HkxValue::Pointer(None)),
                        // Dangling inside nested array.
                        member(
                            "arr",
                            HkxValue::Array(vec![
                                HkxValue::Pointer(Some(0)),
                                HkxValue::Pointer(Some(50)),
                            ]),
                        ),
                    ],
                ),
            ],
        );

        fix_dangling_pointers(&mut hkx);

        let holder = &hkx.objects()[1];
        assert_eq!(
            holder.members[0].value,
            HkxValue::Pointer(Some(0)),
            "valid pointer kept"
        );
        assert_eq!(
            holder.members[1].value,
            HkxValue::Pointer(None),
            "dangling nulled"
        );
        assert_eq!(
            holder.members[2].value,
            HkxValue::Pointer(None),
            "already-null unchanged"
        );
        let HkxValue::Array(arr) = &holder.members[3].value else {
            panic!("arr should be an array");
        };
        assert_eq!(arr[0], HkxValue::Pointer(Some(0)), "valid array entry kept");
        assert_eq!(
            arr[1],
            HkxValue::Pointer(None),
            "dangling array entry nulled"
        );
    }

    // ── Behavior-graph transform unit tests ──────────────────────────────────

    #[test]
    fn fix_variable_value_set_wraps_int_entries_as_inline_value_objects() {
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2015.1.0-r1",
            vec![object(
                "hkbVariableValueSet",
                vec![member(
                    "wordVariableValues",
                    HkxValue::Array(vec![HkxValue::I32(42), HkxValue::I32(-1)]),
                )],
            )],
        );

        fix_variable_value_set(&mut hkx);

        let HkxValue::Array(entries) = &hkx.objects()[0].members[0].value else {
            panic!("wordVariableValues should be an array");
        };
        assert_eq!(entries.len(), 2);
        let HkxValue::Object(first_members) = &entries[0] else {
            panic!("entry should be wrapped object");
        };
        assert_eq!(first_members[0].name, "value");
        assert_eq!(first_members[0].value, HkxValue::I32(42));
        let HkxValue::Object(second_members) = &entries[1] else {
            panic!("entry should be wrapped object");
        };
        assert_eq!(second_members[0].value, HkxValue::I32(-1));
    }

    #[test]
    fn fix_clip_generator_defaults_sets_animation_binding_index_to_negative_one() {
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2015.1.0-r1",
            vec![object(
                "hkbClipGenerator",
                vec![member("animationBindingIndex", HkxValue::I16(0))],
            )],
        );

        fix_clip_generator_defaults(&mut hkx);

        assert_eq!(hkx.objects()[0].members[0].value, HkxValue::I16(-1));
    }

    #[test]
    fn fix_clip_generator_defaults_injects_when_member_missing() {
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2015.1.0-r1",
            vec![object("hkbClipGenerator", vec![])],
        );

        fix_clip_generator_defaults(&mut hkx);

        let m = hkx.objects()[0]
            .members
            .iter()
            .find(|m| m.name == "animationBindingIndex")
            .expect("animationBindingIndex should be injected");
        assert_eq!(m.value, HkxValue::I16(-1));
    }

    #[test]
    fn apply_classxml_defaults_replaces_zero_with_safe_default_for_known_pair() {
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2015.1.0-r1",
            vec![object(
                "hkbLayer",
                vec![
                    member("weight", HkxValue::F32(0.0)),
                    member("onEventId", HkxValue::I32(0)),
                ],
            )],
        );

        apply_classxml_defaults(&mut hkx);

        let obj = &hkx.objects()[0];
        assert_eq!(
            obj.members
                .iter()
                .find(|m| m.name == "weight")
                .map(|m| &m.value),
            Some(&HkxValue::F32(1.0))
        );
        assert_eq!(
            obj.members
                .iter()
                .find(|m| m.name == "onEventId")
                .map(|m| &m.value),
            Some(&HkxValue::I32(-1))
        );
    }

    #[test]
    fn apply_classxml_defaults_sets_first_layer_on_by_default_true() {
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2015.1.0-r1",
            vec![
                object(
                    "hkbLayer",
                    vec![member("onByDefault", HkxValue::Bool(false))],
                ),
                object(
                    "hkbLayerGenerator",
                    vec![member(
                        "layers",
                        HkxValue::Array(vec![HkxValue::Pointer(Some(0))]),
                    )],
                ),
            ],
        );

        apply_classxml_defaults(&mut hkx);

        assert_eq!(
            hkx.objects()[0]
                .members
                .iter()
                .find(|m| m.name == "onByDefault")
                .map(|m| &m.value),
            Some(&HkxValue::Bool(true))
        );
    }

    #[test]
    fn reorder_behavior_metadata_to_end_moves_metadata_classes_in_canonical_order() {
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2015.1.0-r1",
            vec![
                object("hkbBehaviorGraphData", vec![]),
                object("hkbStateMachine", vec![]),
                object("hkbVariableValueSet", vec![]),
                object("hkbBehaviorGraphStringData", vec![]),
                object("hkbRootLevelContainer", vec![]),
            ],
        );

        reorder_behavior_metadata_to_end(&mut hkx);

        let class_names: Vec<&str> = hkx
            .objects()
            .iter()
            .map(|o| o.class_name.as_str())
            .collect();
        assert_eq!(
            class_names,
            vec![
                "hkbStateMachine",
                "hkbRootLevelContainer",
                "hkbBehaviorGraphData",
                "hkbVariableValueSet",
                "hkbBehaviorGraphStringData",
            ]
        );
    }

    #[test]
    fn reorder_behavior_metadata_to_end_remaps_pointers_correctly() {
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2015.1.0-r1",
            vec![
                // obj 0: hkbBehaviorGraphData — will move to new index 1
                object("hkbBehaviorGraphData", vec![]),
                // obj 1: hkbStateMachine pointing to obj 0
                object(
                    "hkbStateMachine",
                    vec![member("data", HkxValue::Pointer(Some(0)))],
                ),
            ],
        );

        reorder_behavior_metadata_to_end(&mut hkx);

        assert_eq!(hkx.objects()[0].class_name, "hkbStateMachine");
        assert_eq!(hkx.objects()[1].class_name, "hkbBehaviorGraphData");
        assert_eq!(
            hkx.objects()[0].members[0].value,
            HkxValue::Pointer(Some(1))
        );
    }

    #[test]
    fn populate_event_property_arrays_is_a_no_op() {
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2015.1.0-r1",
            vec![object(
                "hkbStateMachineEventPropertyArray",
                vec![member("events", HkxValue::Array(vec![]))],
            )],
        );
        let before = hkx.objects().to_vec();

        populate_event_property_arrays(&mut hkx);

        assert_eq!(hkx.objects(), before.as_slice());
    }

    #[test]
    fn fix_behavior_variable_infos_is_a_no_op() {
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2015.1.0-r1",
            vec![object(
                "hkbBehaviorGraphData",
                vec![member(
                    "variableInfos",
                    HkxValue::Array(vec![HkxValue::I32(3)]),
                )],
            )],
        );
        let before = hkx.objects().to_vec();

        fix_behavior_variable_infos(&mut hkx);

        assert_eq!(hkx.objects(), before.as_slice());
    }

    #[test]
    fn converts_bs_locomotion_blend_generator_to_supported_state_machine() {
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2015.1.0-r1",
            vec![
                object(
                    "hkbBehaviorGraphStringData",
                    vec![
                        member(
                            "variableNames",
                            HkxValue::Array(
                                ["Speed", "Direction", "iSyncLocomotionSpeed"]
                                    .into_iter()
                                    .map(string)
                                    .collect(),
                            ),
                        ),
                        member(
                            "eventNames",
                            HkxValue::Array(
                                ["Jog", "Run", "Walk"].into_iter().map(string).collect(),
                            ),
                        ),
                    ],
                ),
                object(
                    "hkbBlenderGenerator",
                    vec![member("name", string("WalkBlend"))],
                ),
                object(
                    "hkbBlenderGenerator",
                    vec![member("name", string("JogBlend"))],
                ),
                object(
                    "hkbBlenderGenerator",
                    vec![member("name", string("RunBlend"))],
                ),
                test_binding_set("fDirectionParameter", 1),
                object(
                    "BSLocomotionBlendGenerator",
                    vec![
                        member("variableBindingSet", HkxValue::Pointer(Some(4))),
                        member("name", string("CombatLocomotionBlendGenerator")),
                        member("pRunBlendGenerator", HkxValue::Pointer(Some(3))),
                        member("pJogBlendGenerator", HkxValue::Pointer(Some(2))),
                        member("pWalkBlendGenerator", HkxValue::Pointer(Some(1))),
                        member("fTransitionDuration", HkxValue::F32(0.2)),
                    ],
                ),
            ],
        );
        let mut warnings = Vec::new();

        migrate_unsupported_behavior_nodes(&mut hkx, &mut warnings);

        assert!(
            hkx.objects()
                .iter()
                .all(|object| object.class_name != "BSLocomotionBlendGenerator")
        );
        let state_machine = &hkx.objects()[5];
        assert_eq!(state_machine.class_name, "hkbStateMachine");
        assert_eq!(state_machine.signature, 5);
        let state_binding = pointer_member_value(&state_machine.members, "variableBindingSet")
            .expect("state machine binding set");
        assert_eq!(
            hkx.objects()[state_binding].class_name,
            "hkbVariableBindingSet"
        );
        assert_eq!(
            binding_variable_index(&hkx, state_binding, "startStateId"),
            Some(2)
        );
        let states_member = state_machine
            .members
            .iter()
            .find(|member| member.name == "states")
            .expect("states member");
        let HkxValue::Array(states) = &states_member.value else {
            panic!("states should be an array");
        };
        assert_eq!(states.len(), 3);
        let expected_transitions = [
            vec![(0, 1), (1, 2)],
            vec![(2, 0), (1, 2)],
            vec![(2, 0), (0, 1)],
        ];
        for (state, expected) in states.iter().zip(expected_transitions) {
            let HkxValue::Pointer(Some(state_index)) = state else {
                panic!("state should be a pointer");
            };
            let transitions_index =
                pointer_member_value(&hkx.objects()[*state_index].members, "transitions")
                    .expect("state transition array");
            let transitions = hkx.objects()[transitions_index]
                .members
                .iter()
                .find(|member| member.name == "transitions")
                .expect("transitions member");
            let HkxValue::Array(transitions) = &transitions.value else {
                panic!("transitions should be an array");
            };
            let actual: Vec<_> = transitions
                .iter()
                .map(|transition| {
                    let members = transition
                        .as_object_members()
                        .expect("transition should be an object");
                    let event_id = members
                        .iter()
                        .find(|member| member.name == "eventId")
                        .and_then(|member| extract_int(&member.value))
                        .expect("transition event id");
                    let to_state_id = members
                        .iter()
                        .find(|member| member.name == "toStateId")
                        .and_then(|member| extract_int(&member.value))
                        .expect("transition destination");
                    (event_id, to_state_id)
                })
                .collect();
            assert_eq!(actual, expected);
        }
        assert!(warnings
            .iter()
            .any(|warning| warning.contains("converted unsupported BSLocomotionBlendGenerator")));
    }

    /// Most FO76 graphs name the int walk/jog/run tier `iLocomotionSpeed`, not
    /// `iSyncLocomotionSpeed`. Without it `startStateId` stays unbound and the
    /// selector freezes in its default state (no jog or run). Ground truth:
    /// `pioneercorebehavior.hkx` (Liberator) exposes only `iLocomotionSpeed`
    /// (int, index 14) and ships with `variableBindingSet = null`.
    #[test]
    fn locomotion_blend_binds_start_state_to_ilocomotionspeed_variable() {
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2015.1.0-r1",
            vec![
                object(
                    "hkbBehaviorGraphStringData",
                    vec![
                        member(
                            "variableNames",
                            HkxValue::Array(
                                ["Speed", "Direction", "iLocomotionSpeed"]
                                    .into_iter()
                                    .map(string)
                                    .collect(),
                            ),
                        ),
                        member(
                            "eventNames",
                            HkxValue::Array(
                                ["Jog", "Run", "Walk"].into_iter().map(string).collect(),
                            ),
                        ),
                    ],
                ),
                object(
                    "hkbBlenderGenerator",
                    vec![member("name", string("WalkBlend"))],
                ),
                object(
                    "hkbBlenderGenerator",
                    vec![member("name", string("JogBlend"))],
                ),
                object(
                    "hkbBlenderGenerator",
                    vec![member("name", string("RunBlend"))],
                ),
                test_binding_set("fDirectionParameter", 1),
                object(
                    "BSLocomotionBlendGenerator",
                    vec![
                        member("variableBindingSet", HkxValue::Pointer(Some(4))),
                        member("name", string("BSLocomotionBlendGenerator")),
                        member("pRunBlendGenerator", HkxValue::Pointer(Some(3))),
                        member("pJogBlendGenerator", HkxValue::Pointer(Some(2))),
                        member("pWalkBlendGenerator", HkxValue::Pointer(Some(1))),
                        member("fTransitionDuration", HkxValue::F32(0.2)),
                    ],
                ),
            ],
        );
        let mut warnings = Vec::new();

        migrate_unsupported_behavior_nodes(&mut hkx, &mut warnings);

        let state_machine = &hkx.objects()[5];
        assert_eq!(state_machine.class_name, "hkbStateMachine");
        let state_binding = pointer_member_value(&state_machine.members, "variableBindingSet")
            .expect("startStateId must be bound, not left null");
        assert_eq!(
            binding_variable_index(&hkx, state_binding, "startStateId"),
            Some(2),
            "startStateId must bind to iLocomotionSpeed"
        );
        assert!(
            !warnings
                .iter()
                .any(|warning| warning.contains("left startStateId unbound"))
        );
    }

    #[test]
    fn behavior_reference_names_gain_the_hkx_extension_fo4_requires() {
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2015.1.0-r1",
            vec![
                object(
                    "hkbBehaviorReferenceGenerator",
                    vec![member(
                        "behaviorName",
                        string("Behaviors\\DialogueBehavior"),
                    )],
                ),
                object(
                    "hkbBehaviorReferenceGenerator",
                    vec![member("behaviorName", string("Behaviors\\Already.hkx"))],
                ),
                object(
                    "hkbBehaviorReferenceGenerator",
                    vec![member("behaviorName", string("Behaviors\\Tool.hkt"))],
                ),
                object(
                    "hkbBehaviorReferenceGenerator",
                    vec![member("behaviorName", string(""))],
                ),
            ],
        );

        normalize_behavior_reference_names(&mut hkx);

        let names: Vec<String> = hkx
            .objects()
            .iter()
            .map(|object| {
                let member = object
                    .members
                    .iter()
                    .find(|member| member.name == "behaviorName")
                    .expect("behaviorName member");
                match &member.value {
                    HkxValue::String { value, .. } => value.clone(),
                    other => panic!("behaviorName should be a string, got {other:?}"),
                }
            })
            .collect();

        assert_eq!(names[0], "Behaviors\\DialogueBehavior.hkx");
        assert_eq!(names[1], "Behaviors\\Already.hkx");
        assert_eq!(names[2], "Behaviors\\Tool.hkx");
        assert_eq!(names[3], "");
    }

    #[test]
    fn zero_weight_action_idle_delegates_to_base_locomotion() {
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2015.1.0-r1",
            vec![
                object(
                    "hkbBehaviorGraphStringData",
                    vec![member(
                        "characterPropertyNames",
                        HkxValue::Array(vec![string("ZeroBoneWeights")]),
                    )],
                ),
                object(
                    "hkbLayerGenerator",
                    vec![
                        member("name", string("Sheepsquatch_LayerGenerator")),
                        member(
                            "layers",
                            HkxValue::Array(vec![
                                HkxValue::Pointer(Some(2)),
                                HkxValue::Pointer(Some(3)),
                            ]),
                        ),
                        member("indexOfSyncMasterChild", HkxValue::U16(1)),
                    ],
                ),
                object(
                    "hkbLayer",
                    vec![
                        member("generator", HkxValue::Pointer(Some(4))),
                        member("onByDefault", HkxValue::Bool(true)),
                    ],
                ),
                object(
                    "hkbLayer",
                    vec![
                        member("variableBindingSet", HkxValue::Pointer(Some(13))),
                        member("generator", HkxValue::Pointer(Some(5))),
                        member("onByDefault", HkxValue::Bool(true)),
                        member("useMotion", HkxValue::Bool(true)),
                    ],
                ),
                object(
                    "hkbStateMachine",
                    vec![member("name", string("BaseLayer_SM"))],
                ),
                object(
                    "hkbStateMachine",
                    vec![
                        member("name", string("ActionLayer_SM")),
                        member("states", HkxValue::Array(vec![HkxValue::Pointer(Some(6))])),
                    ],
                ),
                object(
                    "hkbStateMachineStateInfo",
                    vec![
                        member("name", string("IdleAction")),
                        member("generator", HkxValue::Pointer(Some(7))),
                    ],
                ),
                object(
                    "hkbModifierGenerator",
                    vec![
                        member("name", string("IdleAction_MG")),
                        member("modifier", HkxValue::Pointer(Some(8))),
                        member("generator", HkxValue::Pointer(Some(10))),
                    ],
                ),
                object(
                    "hkbModifierList",
                    vec![
                        member("name", string("IdleAction_ML")),
                        member(
                            "modifiers",
                            HkxValue::Array(vec![
                                HkxValue::Pointer(Some(9)),
                                HkxValue::Pointer(Some(11)),
                            ]),
                        ),
                    ],
                ),
                object(
                    "BSAssignBoneWeightsModifier",
                    vec![
                        member("name", string("WeightAssignment")),
                        member("variableBindingSet", HkxValue::Pointer(Some(12))),
                    ],
                ),
                object(
                    "hkbReferencePoseGenerator",
                    vec![member("name", string("IdleReferencePose"))],
                ),
                object(
                    "BSIsActiveModifier",
                    vec![member("name", string("IdleAction_IsActive"))],
                ),
                object(
                    "hkbVariableBindingSet",
                    vec![member(
                        "bindings",
                        HkxValue::Array(vec![HkxValue::Object(vec![
                            member("memberPath", string("boneWeights1")),
                            member("variableIndex", HkxValue::I32(0)),
                            member("bitIndex", HkxValue::I8(-1)),
                            member("bindingType", HkxValue::I8(1)),
                        ])]),
                    )],
                ),
                object(
                    "hkbVariableBindingSet",
                    vec![member(
                        "bindings",
                        HkxValue::Array(vec![HkxValue::Object(vec![
                            member("memberPath", string("useMotion")),
                            member("variableIndex", HkxValue::I32(35)),
                            member("bitIndex", HkxValue::I8(-1)),
                            member("bindingType", HkxValue::I8(0)),
                        ])]),
                    )],
                ),
            ],
        );
        let mut warnings = Vec::new();

        migrate_unsupported_behavior_nodes(&mut hkx, &mut warnings);

        assert!(
            hkx.objects()
                .iter()
                .all(|object| object.class_name != "BSAssignBoneWeightsModifier")
        );
        let layer_generator = hkx
            .objects()
            .iter()
            .find(|object| {
                string_member_value(&object.members, "name") == Some("Sheepsquatch_LayerGenerator")
            })
            .expect("layer generator survives");
        let layers = pointer_array_member_values(&layer_generator.members, "layers")
            .expect("layer array survives");
        assert_eq!(layers.len(), 1);
        let action_layer = &hkx.objects()[layers[0]];
        assert_eq!(
            action_layer
                .members
                .iter()
                .find(|member| member.name == "variableBindingSet")
                .map(|member| &member.value),
            Some(&HkxValue::Pointer(None))
        );
        assert_eq!(
            action_layer
                .members
                .iter()
                .find(|member| member.name == "useMotion")
                .and_then(|member| extract_int(&member.value)),
            Some(1)
        );
        let action_generator = &hkx.objects()
            [pointer_member_value(&action_layer.members, "generator").expect("action generator")];
        assert_eq!(
            string_member_value(&action_generator.members, "name"),
            Some("ActionLayer_SM")
        );
        assert_eq!(
            layer_generator
                .members
                .iter()
                .find(|member| member.name == "indexOfSyncMasterChild")
                .and_then(|member| extract_int(&member.value)),
            Some(0)
        );

        let idle_wrapper = hkx
            .objects()
            .iter()
            .find(|object| string_member_value(&object.members, "name") == Some("IdleAction_MG"))
            .expect("idle wrapper survives with its supported modifier");
        let idle_child = &hkx.objects()
            [pointer_member_value(&idle_wrapper.members, "generator").expect("idle child")];
        assert_eq!(
            string_member_value(&idle_child.members, "name"),
            Some("BaseLayer_SM")
        );
        assert!(warnings.iter().any(|warning| warning.contains(
            "collapsed zero-weight action state machine ActionLayer_SM over base generator BaseLayer_SM"
        )));
    }

    #[test]
    fn all_zero_bone_weight_mask_collapses_without_a_zeroweights_name() {
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2015.1.0-r1",
            vec![
                object(
                    "hkbLayerGenerator",
                    vec![
                        member("name", string("Floater_LayerGenerator")),
                        member(
                            "layers",
                            HkxValue::Array(vec![
                                HkxValue::Pointer(Some(1)),
                                HkxValue::Pointer(Some(2)),
                            ]),
                        ),
                        member("indexOfSyncMasterChild", HkxValue::U16(1)),
                    ],
                ),
                object(
                    "hkbLayer",
                    vec![
                        member("generator", HkxValue::Pointer(Some(3))),
                        member("onByDefault", HkxValue::Bool(true)),
                    ],
                ),
                object(
                    "hkbLayer",
                    vec![
                        member("generator", HkxValue::Pointer(Some(4))),
                        member("onByDefault", HkxValue::Bool(true)),
                        member("useMotion", HkxValue::Bool(true)),
                    ],
                ),
                object(
                    "hkbStateMachine",
                    vec![member("name", string("StandingLocomotion_SM"))],
                ),
                object(
                    "hkbStateMachine",
                    vec![
                        member("name", string("Action_SM")),
                        member("states", HkxValue::Array(vec![HkxValue::Pointer(Some(5))])),
                    ],
                ),
                object(
                    "hkbStateMachineStateInfo",
                    vec![
                        member("name", string("DefaultState")),
                        member("generator", HkxValue::Pointer(Some(6))),
                    ],
                ),
                object(
                    "hkbModifierGenerator",
                    vec![
                        member("name", string("DefaultState_MG")),
                        member("modifier", HkxValue::Pointer(Some(7))),
                        member("generator", HkxValue::Pointer(Some(9))),
                    ],
                ),
                object(
                    "hkbModifierList",
                    vec![
                        member("name", string("DefaultState_ML")),
                        member(
                            "modifiers",
                            HkxValue::Array(vec![HkxValue::Pointer(Some(8))]),
                        ),
                    ],
                ),
                object(
                    "BSAssignBoneWeightsModifier",
                    vec![
                        member("name", string("AssignBoneWeights_Zero")),
                        member("boneWeights1", HkxValue::Pointer(Some(10))),
                    ],
                ),
                object(
                    "hkbReferencePoseGenerator",
                    vec![member("name", string("ReferencePoseGenerator"))],
                ),
                object(
                    "hkbBoneWeightArray",
                    vec![member(
                        "boneWeights",
                        HkxValue::Array(vec![
                            HkxValue::F32(0.0),
                            HkxValue::F32(0.0),
                            HkxValue::F32(0.0),
                        ]),
                    )],
                ),
            ],
        );
        let mut warnings = Vec::new();

        migrate_unsupported_behavior_nodes(&mut hkx, &mut warnings);

        let layer_generator = hkx
            .objects()
            .iter()
            .find(|object| {
                string_member_value(&object.members, "name") == Some("Floater_LayerGenerator")
            })
            .expect("layer generator survives");
        assert_eq!(
            pointer_array_member_values(&layer_generator.members, "layers")
                .expect("layer array survives")
                .len(),
            1
        );

        // Without the collapse the idle state points straight at the reference
        // pose, which then plays unmasked at full weight — the T-pose.
        let idle_wrapper = hkx
            .objects()
            .iter()
            .find(|object| string_member_value(&object.members, "name") == Some("DefaultState_MG"))
            .expect("idle wrapper survives");
        let idle_child = &hkx.objects()
            [pointer_member_value(&idle_wrapper.members, "generator").expect("idle child")];
        assert_eq!(
            string_member_value(&idle_child.members, "name"),
            Some("StandingLocomotion_SM")
        );
        assert!(warnings.iter().any(|warning| warning.contains(
            "collapsed zero-weight action state machine Action_SM over base generator StandingLocomotion_SM"
        )));
    }

    #[test]
    fn empty_bone_weight_mask_is_not_treated_as_a_zero_assignment() {
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2015.1.0-r1",
            vec![
                object(
                    "BSAssignBoneWeightsModifier",
                    vec![
                        member("name", string("AssignBoneWeights_FullBody")),
                        member("boneWeights1", HkxValue::Pointer(Some(1))),
                    ],
                ),
                object(
                    "hkbBoneWeightArray",
                    vec![member("boneWeights", HkxValue::Array(Vec::new()))],
                ),
            ],
        );
        let modifier = &hkx.objects()[0];

        assert!(!is_zero_weight_assignment(&hkx, modifier));
    }

    #[test]
    fn removes_bs_assign_bone_weights_modifier_referenced_from_modifier_list() {
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2015.1.0-r1",
            vec![
                object(
                    "hkbEventDrivenModifier",
                    vec![member("name", string("KeptModifier"))],
                ),
                object(
                    "BSAssignBoneWeightsModifier",
                    vec![member("name", string("ListBoneWeightModifier"))],
                ),
                object(
                    "hkbModifierList",
                    vec![
                        member("name", string("ModifierList")),
                        member(
                            "modifiers",
                            HkxValue::Array(vec![
                                HkxValue::Pointer(Some(0)),
                                HkxValue::Pointer(Some(1)),
                            ]),
                        ),
                    ],
                ),
            ],
        );
        let mut warnings = Vec::new();

        migrate_unsupported_behavior_nodes(&mut hkx, &mut warnings);

        assert!(
            hkx.objects()
                .iter()
                .all(|object| object.class_name != "BSAssignBoneWeightsModifier")
        );
        let list = hkx
            .objects()
            .iter()
            .find(|object| object.class_name == "hkbModifierList")
            .expect("modifier list survives");
        let modifiers = list
            .members
            .iter()
            .find(|member| member.name == "modifiers")
            .expect("modifiers member");
        let HkxValue::Array(modifiers) = &modifiers.value else {
            panic!("modifiers should be an array");
        };
        assert_eq!(modifiers.len(), 1);
        assert_eq!(modifiers[0], HkxValue::Pointer(Some(0)));
        assert!(
            warnings.iter().any(|warning| warning.contains(
                "removed unsupported BSAssignBoneWeightsModifier ListBoneWeightModifier"
            ))
        );
    }

    #[test]
    fn bypasses_bs_assign_bone_weights_modifier_without_empty_object() {
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2015.1.0-r1",
            vec![
                object(
                    "hkbReferencePoseGenerator",
                    vec![member("name", string("Child"))],
                ),
                object(
                    "BSAssignBoneWeightsModifier",
                    vec![member("name", string("BoneWeightModifier"))],
                ),
                object(
                    "hkbModifierGenerator",
                    vec![
                        member("name", string("Wrapper")),
                        member("modifier", HkxValue::Pointer(Some(1))),
                        member("generator", HkxValue::Pointer(Some(0))),
                    ],
                ),
                object(
                    "hkbStateMachineStateInfo",
                    vec![member("generator", HkxValue::Pointer(Some(2)))],
                ),
            ],
        );
        let mut warnings = Vec::new();

        migrate_unsupported_behavior_nodes(&mut hkx, &mut warnings);

        assert!(
            hkx.objects()
                .iter()
                .all(|object| object.class_name != "BSAssignBoneWeightsModifier")
        );
        assert!(
            hkx.objects()
                .iter()
                .all(|object| object.class_name != "hkbModifierGenerator")
        );
        let state = hkx
            .objects()
            .iter()
            .find(|object| object.class_name == "hkbStateMachineStateInfo")
            .expect("state survives");
        assert_eq!(pointer_member_value(&state.members, "generator"), Some(0));
        assert!(warnings
            .iter()
            .any(|warning| warning.contains("bypassed unsupported BSAssignBoneWeightsModifier")));
    }

    #[test]
    fn behavior_unsupported_node_transform_removes_target_classxml_misses() {
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2015.1.0-r1",
            vec![
                behavior_string_data_with_variables(&["Speed", "Direction", "iMovementSpeed"]),
                object(
                    "hkbBlenderGenerator",
                    vec![member("name", string("WalkBlend"))],
                ),
                object(
                    "hkbBlenderGenerator",
                    vec![member("name", string("JogBlend"))],
                ),
                object(
                    "hkbBlenderGenerator",
                    vec![member("name", string("RunBlend"))],
                ),
                test_binding_set("fDirectionParameter", 1),
                object(
                    "BSLocomotionBlendGenerator",
                    vec![
                        member("variableBindingSet", HkxValue::Pointer(Some(4))),
                        member("name", string("CombatLocomotionBlendGenerator")),
                        member("pRunBlendGenerator", HkxValue::Pointer(Some(3))),
                        member("pJogBlendGenerator", HkxValue::Pointer(Some(2))),
                        member("pWalkBlendGenerator", HkxValue::Pointer(Some(1))),
                    ],
                ),
                object(
                    "hkbReferencePoseGenerator",
                    vec![member("name", string("Child"))],
                ),
                object("BSAssignBoneWeightsModifier", vec![]),
                object(
                    "hkbModifierGenerator",
                    vec![
                        member("modifier", HkxValue::Pointer(Some(7))),
                        member("generator", HkxValue::Pointer(Some(6))),
                    ],
                ),
            ],
        );
        let mut warnings = Vec::new();

        migrate_unsupported_behavior_nodes(&mut hkx, &mut warnings);

        let mut registry = DescriptorRegistry::for_contents_version("hk_2014.1.0-r1");
        for object in hkx.objects() {
            assert!(
                registry.get(&object.class_name).unwrap().is_some(),
                "{} should exist in FO4 classxml",
                object.class_name
            );
        }
    }

    // ── FO76 → FO4 physics transform unit tests ──────────────────────────────

    /// Sibling `hkpRagdollConstraintData` whose `atoms` the ball-and-socket
    /// migration is modelled on, shaped like the real decoded FO76 atom.
    fn ragdoll_constraint_template() -> HkxObject {
        let limit = |name: &str| {
            member(
                name,
                HkxValue::Object(vec![
                    member("type", HkxValue::U16(15)),
                    member("isEnabled", HkxValue::U8(1)),
                    member("minAngle", HkxValue::F32(-3.141_592_7)),
                    member("maxAngle", HkxValue::F32(3.141_592_7)),
                ]),
            )
        };
        object(
            "hkpRagdollConstraintData",
            vec![member(
                "atoms",
                HkxValue::Object(vec![
                    member(
                        "transforms",
                        HkxValue::Object(vec![
                            member("type", HkxValue::U16(2)),
                            member("transformA", local_transform([9.0, 9.0, 9.0])),
                            member("transformB", local_transform([9.0, 9.0, 9.0])),
                        ]),
                    ),
                    member(
                        "setupStabilization",
                        HkxValue::Object(vec![member("type", HkxValue::U16(23))]),
                    ),
                    member(
                        "ragdollMotors",
                        HkxValue::Object(vec![
                            member("type", HkxValue::U16(19)),
                            member("isEnabled", HkxValue::Bool(false)),
                            member(
                                "motors",
                                HkxValue::Array(vec![
                                    HkxValue::Pointer(Some(7)),
                                    HkxValue::Pointer(Some(7)),
                                    HkxValue::Pointer(Some(7)),
                                ]),
                            ),
                        ]),
                    ),
                    member(
                        "angFriction",
                        HkxValue::Object(vec![
                            member("type", HkxValue::U16(17)),
                            member("isEnabled", HkxValue::U8(1)),
                        ]),
                    ),
                    limit("twistLimit"),
                    limit("coneLimit"),
                    limit("planesLimit"),
                    member(
                        "ballSocket",
                        HkxValue::Object(vec![member("type", HkxValue::U16(5))]),
                    ),
                ]),
            )],
        )
    }

    fn ball_and_socket_constraint() -> HkxObject {
        object(
            "hkpBallAndSocketConstraintData",
            vec![
                member("memSizeAndFlags", HkxValue::U32(0xDEAD_BEEF)),
                member("refCount", HkxValue::U16(2)),
                member(
                    "atoms",
                    HkxValue::Object(vec![
                        member(
                            "pivots",
                            HkxValue::Object(vec![
                                member("type", HkxValue::U16(3)),
                                member("translationA", HkxValue::F32List(vec![0.0, 0.0, 0.0, 1.0])),
                                member(
                                    "translationB",
                                    HkxValue::F32List(vec![0.5, -0.25, 1.75, 0.0]),
                                ),
                            ]),
                        ),
                        member(
                            "setupStabilization",
                            HkxValue::Object(vec![
                                member("type", HkxValue::U16(23)),
                                member("enabled", HkxValue::U8(1)),
                            ]),
                        ),
                        member(
                            "ballSocket",
                            HkxValue::Object(vec![
                                member("type", HkxValue::U16(5)),
                                member("bodiesToNotify", HkxValue::U16(0)),
                            ]),
                        ),
                    ]),
                ),
            ],
        )
    }

    fn atom<'a>(hkx: &'a HkxFile, object_index: usize, name: &str) -> &'a [HkxMember] {
        hkx.objects()[object_index]
            .members
            .iter()
            .find(|m| m.name == "atoms")
            .and_then(|m| m.value.as_object_members())
            .and_then(|atoms| atoms.iter().find(|m| m.name == name))
            .and_then(|m| m.value.as_object_members())
            .unwrap_or_else(|| panic!("missing atom {name}"))
    }

    fn atom_field(hkx: &HkxFile, object_index: usize, atom_name: &str, field: &str) -> HkxValue {
        atom(hkx, object_index, atom_name)
            .iter()
            .find(|m| m.name == field)
            .map(|m| m.value.clone())
            .unwrap_or_else(|| panic!("missing {atom_name}.{field}"))
    }

    #[test]
    fn migrate_skeleton_physics_ball_and_socket_keeps_pivot_as_local_transforms() {
        let mut hkx = HkxFile::from_tagxml(
            12,
            "hk_2015.1.0-r1",
            vec![ball_and_socket_constraint(), ragdoll_constraint_template()],
        );
        migrate_skeleton_physics(&mut hkx);

        assert_eq!(hkx.objects()[0].class_name, "hkpRagdollConstraintData");
        assert_eq!(hkx.objects()[0].members.len(), 1);
        assert_eq!(hkx.objects()[0].members[0].name, "atoms");

        // The pivot survives as the transform translation column, not as zeros.
        assert_eq!(
            atom_field(&hkx, 0, "transforms", "type"),
            HkxValue::U16(2),
            "transforms atom must be TYPE_SET_LOCAL_TRANSFORMS, not TYPE_INVALID"
        );
        assert_eq!(
            atom_field(&hkx, 0, "transforms", "transformA"),
            local_transform([0.0, 0.0, 0.0])
        );
        assert_eq!(
            atom_field(&hkx, 0, "transforms", "transformB"),
            local_transform([0.5, -0.25, 1.75])
        );

        // Ball-and-socket means free rotation: every angular limit is disabled,
        // but the atoms are still typed so FO4 can walk them.
        for name in ["twistLimit", "coneLimit", "planesLimit"] {
            assert_eq!(atom_field(&hkx, 0, name, "isEnabled"), HkxValue::U8(0));
            assert_eq!(atom_field(&hkx, 0, name, "type"), HkxValue::U16(15));
        }
        assert_eq!(
            atom_field(&hkx, 0, "angFriction", "isEnabled"),
            HkxValue::U8(0)
        );

        // FO4 dereferences the motors during ragdoll activation.
        assert_eq!(
            atom_field(&hkx, 0, "ragdollMotors", "motors"),
            HkxValue::Array(vec![
                HkxValue::Pointer(Some(7)),
                HkxValue::Pointer(Some(7)),
                HkxValue::Pointer(Some(7)),
            ])
        );

        // The constraint's own stabilization/ball-socket values are preserved.
        assert_eq!(
            atom_field(&hkx, 0, "setupStabilization", "enabled"),
            HkxValue::U8(1)
        );
        assert_eq!(
            atom_field(&hkx, 0, "ballSocket", "bodiesToNotify"),
            HkxValue::U16(0)
        );
    }

    #[test]
    fn migrate_skeleton_physics_keeps_ball_and_socket_without_a_ragdoll_sibling() {
        let mut hkx =
            HkxFile::from_tagxml(12, "hk_2015.1.0-r1", vec![ball_and_socket_constraint()]);
        migrate_skeleton_physics(&mut hkx);

        // FO4 registers hkpBallAndSocketConstraintData, so keeping the class
        // beats emitting a ragdoll constraint with no atom layout to copy.
        assert_eq!(
            hkx.objects()[0].class_name,
            "hkpBallAndSocketConstraintData"
        );
        assert_eq!(hkx.objects()[0].members.len(), 1);
        assert_eq!(hkx.objects()[0].members[0].name, "atoms");
    }

    #[test]
    fn inject_ragdoll_motors_wires_constraints_the_source_left_null() {
        let mut null_motors = ragdoll_constraint_template();
        if let Some(motors) = null_motors
            .members
            .iter_mut()
            .find(|m| m.name == "atoms")
            .and_then(|m| m.value.as_object_members_mut())
            .and_then(|atoms| atoms.iter_mut().find(|m| m.name == "ragdollMotors"))
            .and_then(|m| m.value.as_object_members_mut())
            .and_then(|rm| rm.iter_mut().find(|m| m.name == "motors"))
        {
            motors.value = HkxValue::Array(vec![HkxValue::Pointer(None); 3]);
        }
        let mut hkx = HkxFile::from_tagxml(
            12,
            "hk_2015.1.0-r1",
            vec![ragdoll_constraint_template(), null_motors],
        );
        inject_ragdoll_motors(&mut hkx);

        // A populated sibling must not stop the null one from being wired.
        let HkxValue::Array(motors) = atom_field(&hkx, 1, "ragdollMotors", "motors") else {
            panic!("motors is not an array");
        };
        assert!(
            motors
                .iter()
                .all(|m| matches!(m, HkxValue::Pointer(Some(_))))
        );
        // The already-populated constraint keeps its own pointer.
        assert_eq!(
            atom_field(&hkx, 0, "ragdollMotors", "motors"),
            HkxValue::Array(vec![HkxValue::Pointer(Some(7)); 3])
        );
    }

    #[test]
    fn migrate_skeleton_physics_renames_compressed_mesh_tree() {
        // Gate requires a physics-trigger class — pair tree with PSD root.
        let mut hkx = HkxFile::from_tagxml(
            12,
            "hk_2015.1.0-r1",
            vec![
                object("hknpPhysicsSystemData", vec![]),
                object(
                    "hknpCompressedMeshShapeTree",
                    vec![member("memSizeAndFlags", HkxValue::U32(0))],
                ),
            ],
        );
        migrate_skeleton_physics(&mut hkx);
        assert_eq!(hkx.objects()[1].class_name, "hknpCompressedMeshShapeData");
        assert!(hkx.objects()[1].members.is_empty());
    }

    #[test]
    fn migrate_skeleton_physics_promotes_convex_shape_to_sphere() {
        let mut hkx = HkxFile::from_tagxml(
            12,
            "hk_2015.1.0-r1",
            vec![
                object("hknpPhysicsSystemData", vec![]),
                object(
                    "hknpConvexShape",
                    vec![
                        member("flags", HkxValue::I32(515)),
                        member("dispatchType", HkxValue::I32(2)),
                        member("type", HkxValue::I32(0)),
                        member("convexRadius", HkxValue::F32(0.25)),
                        member(
                            "vertices",
                            HkxValue::Array(vec![HkxValue::F32List(vec![0.0, 0.0, 0.0, 0.5]); 4]),
                        ),
                    ],
                ),
            ],
        );
        migrate_skeleton_physics(&mut hkx);
        let obj = &hkx.objects()[1];
        assert_eq!(obj.class_name, "hknpSphereShape");
        let flags = obj.members.iter().find(|m| m.name == "flags").unwrap();
        assert_eq!(flags.value, HkxValue::I32(273));
        let dispatch = obj
            .members
            .iter()
            .find(|m| m.name == "dispatchType")
            .unwrap();
        assert_eq!(dispatch.value, HkxValue::I32(1));
        assert!(obj.members.iter().all(|m| m.name != "type"));
    }

    #[test]
    fn migrate_skeleton_physics_keeps_true_generic_convex_shape() {
        let mut hkx = HkxFile::from_tagxml(
            12,
            "hk_2015.1.0-r1",
            vec![
                object("hknpPhysicsSystemData", vec![]),
                object(
                    "hknpConvexShape",
                    vec![
                        member("flags", HkxValue::I32(1)),
                        member("dispatchType", HkxValue::I32(2)),
                        member("convexRadius", HkxValue::F32(0.0)),
                        member(
                            "vertices",
                            HkxValue::Array(vec![
                                HkxValue::F32List(vec![-1.0, 0.0, 0.0, 0.5]),
                                HkxValue::F32List(vec![1.0, 0.0, 0.0, 0.5]),
                            ]),
                        ),
                    ],
                ),
            ],
        );
        migrate_skeleton_physics(&mut hkx);
        assert_eq!(hkx.objects()[1].class_name, "hknpConvexShape");
    }

    #[test]
    fn migrate_skeleton_physics_normalizes_capsule_for_embedded_blob() {
        // No hkRootLevelContainer = embedded blob => target_w = 1.0
        let mut hkx = HkxFile::from_tagxml(
            12,
            "hk_2015.1.0-r1",
            vec![object(
                "hknpCapsuleShape",
                vec![
                    member("a", HkxValue::F32List(vec![1.0, 0.0, 0.0, 0.5])),
                    member("b", HkxValue::F32List(vec![0.0, 1.0, 0.0, 0.5])),
                    member("flags", HkxValue::I32(515)),
                    member("dispatchType", HkxValue::I32(2)),
                    member(
                        "planes",
                        HkxValue::Array(vec![
                            HkxValue::F32List(vec![1.0, 0.0, 0.0, 0.0]),
                            HkxValue::F32List(vec![-1.0, 0.0, 0.0, 0.0]),
                            HkxValue::F32List(vec![0.0, 1.0, 0.0, 0.0]),
                            HkxValue::F32List(vec![0.0, -1.0, 0.0, 0.0]),
                            HkxValue::F32List(vec![0.0, 0.0, 1.0, 0.0]),
                            HkxValue::F32List(vec![0.0, 0.0, -1.0, 0.0]),
                        ]),
                    ),
                ],
            )],
        );
        migrate_skeleton_physics(&mut hkx);
        let obj = &hkx.objects()[0];
        let HkxValue::F32List(a) = &obj.members.iter().find(|m| m.name == "a").unwrap().value
        else {
            panic!("a missing");
        };
        assert_eq!(a[3], 1.0);
        let flags = obj.members.iter().find(|m| m.name == "flags").unwrap();
        assert_eq!(flags.value, HkxValue::I32(451));
        let HkxValue::Array(planes) = &obj
            .members
            .iter()
            .find(|m| m.name == "planes")
            .unwrap()
            .value
        else {
            panic!();
        };
        assert_eq!(planes.len(), 8);
    }

    #[test]
    fn migrate_skeleton_physics_synthesizes_capsule_hull_from_fo76_radius_w() {
        let mut hkx = HkxFile::from_tagxml(
            12,
            "hk_2015.1.0-r1",
            vec![object(
                "hknpCapsuleShape",
                vec![
                    member("flags", HkxValue::I32(451)),
                    member("numShapeKeyBits", HkxValue::I32(0)),
                    member("dispatchType", HkxValue::I32(2)),
                    member("convexRadius", HkxValue::F32(0.077017)),
                    member("userData", HkxValue::U32(1423343525)),
                    member("vertices", HkxValue::Array(vec![])),
                    member("planes", HkxValue::Array(vec![])),
                    member("faces", HkxValue::Array(vec![])),
                    member("indices", HkxValue::Array(vec![])),
                    member("a", HkxValue::F32List(vec![0.569411, 0.0, 0.0, 0.077795])),
                    member("b", HkxValue::F32List(vec![-0.023189, 0.0, 0.0, 1.0])),
                ],
            )],
        );

        migrate_skeleton_physics(&mut hkx);

        let obj = &hkx.objects()[0];
        let vertices = obj.members.iter().find(|m| m.name == "vertices").unwrap();
        let planes = obj.members.iter().find(|m| m.name == "planes").unwrap();
        let faces = obj.members.iter().find(|m| m.name == "faces").unwrap();
        let indices = obj.members.iter().find(|m| m.name == "indices").unwrap();
        let HkxValue::Array(vertices) = &vertices.value else {
            panic!("vertices should be an array");
        };
        let HkxValue::Array(planes) = &planes.value else {
            panic!("planes should be an array");
        };
        let HkxValue::Array(faces) = &faces.value else {
            panic!("faces should be an array");
        };
        let HkxValue::Array(indices) = &indices.value else {
            panic!("indices should be an array");
        };
        assert_eq!(vertices.len(), 8);
        assert_eq!(planes.len(), 8);
        assert_eq!(faces.len(), 6);
        assert_eq!(indices.len(), 24);
        let HkxValue::F32List(first_vertex) = &vertices[0] else {
            panic!("first vertex should be a vector");
        };
        assert!((first_vertex[0] - -0.023967).abs() < 0.000002);
        assert!((first_vertex[1] - -0.000778).abs() < 0.000002);
        assert!((first_vertex[2] - 0.000778).abs() < 0.000002);
        assert_eq!(first_vertex[3], 0.5);
    }

    #[test]
    fn reclassify_fo76_physics_system_data_renames_referenced_object_with_body_cinfos() {
        let mut hkx = HkxFile::from_tagxml(
            12,
            "hk_2015.1.0-r1",
            vec![object(
                "hkReferencedObject",
                vec![
                    member("motionProperties", HkxValue::Array(vec![])),
                    member("bodyCinfos", HkxValue::Array(vec![])),
                ],
            )],
        );
        reclassify_fo76_physics_system_data(&mut hkx);
        let obj = &hkx.objects()[0];
        assert_eq!(obj.class_name, "hknpPhysicsSystemData");
        // Empty motionCinfos was inserted right after motionProperties.
        let names: Vec<&str> = obj.members.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["motionProperties", "motionCinfos", "bodyCinfos"]
        );
    }

    #[test]
    fn reclassify_fo76_physics_system_data_skips_referenced_object_without_body_cinfos() {
        let mut hkx = HkxFile::from_tagxml(
            12,
            "hk_2015.1.0-r1",
            vec![object(
                "hkReferencedObject",
                vec![member("name", HkxValue::I32(0))],
            )],
        );
        reclassify_fo76_physics_system_data(&mut hkx);
        assert_eq!(hkx.objects()[0].class_name, "hkReferencedObject");
    }

    #[test]
    fn reclassify_mass_distributions_renames_uint16_with_mass_fields() {
        let mut hkx = HkxFile::from_tagxml(
            12,
            "hk_2015.1.0-r1",
            vec![
                object(
                    "hkUint16",
                    vec![member(
                        "centerOfMassAndVolume",
                        HkxValue::F32List(vec![0.0; 4]),
                    )],
                ),
                object("hkUint16", vec![member("unrelated", HkxValue::I16(0))]),
            ],
        );
        reclassify_mass_distributions(&mut hkx);
        assert_eq!(hkx.objects()[0].class_name, "hknpRefMassDistribution");
        assert_eq!(hkx.objects()[1].class_name, "hkUint16");
    }

    #[test]
    fn synthesize_ragdoll_shape_geometry_uses_mass_distribution_for_sphere_vertex() {
        let body = HkxValue::Object(vec![
            member("shape", HkxValue::Pointer(Some(1))),
            member("massDistribution", HkxValue::Pointer(Some(2))),
        ]);
        let mut mass = object(
            "hknpRefMassDistribution",
            vec![member(
                "massDistribution",
                HkxValue::Object(vec![member(
                    "centerOfMassAndVolume",
                    HkxValue::F32List(vec![-0.0, -0.109804, -0.062091, 0.032527]),
                )]),
            )],
        );
        mass.name = Some("#mass".to_string());
        let mut hkx = HkxFile::from_tagxml(
            12,
            "hk_2015.1.0-r1",
            vec![
                object(
                    "hknpRagdollData",
                    vec![member("bodyCinfos", HkxValue::Array(vec![body]))],
                ),
                object(
                    "hknpSphereShape",
                    vec![
                        member("convexRadius", HkxValue::F32(0.198025)),
                        member("vertices", HkxValue::Array(vec![])),
                    ],
                ),
                mass,
            ],
        );

        synthesize_ragdoll_shape_geometry(&mut hkx);

        let vertices = hkx.objects()[1]
            .members
            .iter()
            .find(|m| m.name == "vertices")
            .unwrap();
        let HkxValue::Array(vertices) = &vertices.value else {
            panic!("vertices should be an array");
        };
        assert_eq!(vertices.len(), 1);
        assert_eq!(
            vertices[0],
            HkxValue::F32List(vec![-0.0, -0.109804, -0.062091, 0.5])
        );
    }

    #[test]
    fn normalize_sphere_support_vertices_pads_compact_sphere_to_fo4_width() {
        let support = HkxValue::F32List(vec![0.049218, 0.011694, -0.000945, 0.5]);
        let mut hkx = HkxFile::from_tagxml(
            12,
            "hk_2015.1.0-r1",
            vec![object(
                "hknpSphereShape",
                vec![member("vertices", HkxValue::Array(vec![support.clone()]))],
            )],
        );

        normalize_sphere_support_vertices(&mut hkx);

        let HkxValue::Array(vertices) = &hkx.objects()[0]
            .members
            .iter()
            .find(|member| member.name == "vertices")
            .expect("sphere vertices")
            .value
        else {
            panic!("sphere vertices must be an array");
        };
        assert_eq!(vertices, &vec![support; 4]);
    }

    #[test]
    fn normalize_shape_mass_properties_flattens_fo76_packed_vectors() {
        let center_words = vec![21621_i16, -1203, 674, 12032];
        let inertia_words = vec![11300_i16, 22813, 14204, 12160];
        let wrapped = |words: &[i16]| {
            HkxValue::Object(vec![member(
                "values",
                HkxValue::Array(words.iter().copied().map(HkxValue::I16).collect()),
            )])
        };
        let mut hkx = HkxFile::from_tagxml(
            12,
            "hk_2015.1.0-r1",
            vec![object(
                "hknpShapeMassProperties",
                vec![member(
                    "compressedMassProperties",
                    HkxValue::Object(vec![
                        member("centerOfMass", wrapped(&center_words)),
                        member("inertia", wrapped(&inertia_words)),
                        member(
                            "majorAxisSpace",
                            HkxValue::Array(
                                [-32768_i16, -32768, 32150, -2775]
                                    .into_iter()
                                    .map(HkxValue::I16)
                                    .collect(),
                            ),
                        ),
                        member("mass", HkxValue::F32(12.565272)),
                        member("volume", HkxValue::F32(0.012565272)),
                    ]),
                )],
            )],
        );

        normalize_shape_mass_properties(&mut hkx);

        let compressed = hkx.objects()[0].members[0]
            .value
            .as_object_members()
            .expect("compressed mass properties");
        let words = |name: &str| {
            compressed
                .iter()
                .find(|member| member.name == name)
                .map(|member| &member.value)
                .expect("packed vector")
        };
        assert_eq!(
            words("centerOfMass"),
            &HkxValue::Array(center_words.into_iter().map(HkxValue::I16).collect())
        );
        assert_eq!(
            words("inertia"),
            &HkxValue::Array(inertia_words.into_iter().map(HkxValue::I16).collect())
        );
        assert_eq!(words("volume"), &HkxValue::F32(0.012565272));
    }

    #[test]
    fn strip_shape_connectivity_nulls_pointer_and_drops_target() {
        let mut hkx = HkxFile::from_tagxml(
            12,
            "hk_2015.1.0-r1",
            vec![
                // Object 0: shape pointing at object 1's connectivity.
                object(
                    "hknpCapsuleShape",
                    vec![member("connectivity", HkxValue::Pointer(Some(1)))],
                ),
                // Object 1: connectivity wrapper to drop.
                object("hkUint16", vec![]),
            ],
        );
        strip_shape_connectivity(&mut hkx);
        assert_eq!(hkx.objects().len(), 1);
        assert_eq!(hkx.objects()[0].class_name, "hknpCapsuleShape");
        let conn = hkx.objects()[0]
            .members
            .iter()
            .find(|m| m.name == "connectivity")
            .unwrap();
        assert_eq!(conn.value, HkxValue::Pointer(None));
    }

    #[test]
    fn convert_box_shapes_to_polytopes_preserves_hull_payload() {
        let box_shape = HkxObject {
            name: Some("#0015".to_string()),
            offset: 0,
            signature: 0,
            class_name: "hknpBoxShape".to_string(),
            members: vec![
                member("memSizeAndFlags", HkxValue::U16(0)),
                member("refCount", HkxValue::U16(0)),
                member("flags", HkxValue::U16(259)),
                member("numShapeKeyBits", HkxValue::U8(0)),
                member("dispatchType", HkxValue::U8(1)),
                member("convexRadius", HkxValue::F32(0.0)),
                member("userData", HkxValue::U64(0)),
                member(
                    "vertices",
                    HkxValue::Array(vec![HkxValue::F32List(vec![1.0, 2.0, 3.0, 0.0])]),
                ),
                member(
                    "planes",
                    HkxValue::Array(vec![HkxValue::F32List(vec![0.0, 0.0, 1.0, 1.0])]),
                ),
                member(
                    "faces",
                    HkxValue::Array(vec![HkxValue::Object(vec![
                        member("firstIndex", HkxValue::U16(0)),
                        member("numIndices", HkxValue::U8(3)),
                        member("minHalfAngle", HkxValue::U8(0)),
                    ])]),
                ),
                member("indices", HkxValue::Array(vec![HkxValue::U8(0)])),
                member("properties", HkxValue::Pointer(Some(1))),
                member("connectivity", HkxValue::Pointer(Some(1))),
                member("obb", HkxValue::Object(vec![])),
            ],
        };
        let mut hkx = HkxFile::from_tagxml(
            12,
            "hk_2015.1.0-r1",
            vec![box_shape, object("hkRefCountedProperties", vec![])],
        );
        let mut warnings = Vec::new();

        convert_box_shapes_to_polytopes(&mut hkx, &mut warnings);

        let converted = &hkx.objects()[0];
        assert_eq!(converted.class_name, "hknpConvexPolytopeShape");
        assert_eq!(converted.signature, 1);
        assert!(warnings.iter().any(|warning| warning.contains("#0015")));

        let names: Vec<&str> = converted
            .members
            .iter()
            .map(|member| member.name.as_str())
            .collect();
        assert!(names.contains(&"vertices"));
        assert!(names.contains(&"planes"));
        assert!(names.contains(&"faces"));
        assert!(names.contains(&"indices"));
        assert!(names.contains(&"properties"));
        assert!(!names.contains(&"memSizeAndFlags"));
        assert!(!names.contains(&"refCount"));
        assert!(!names.contains(&"connectivity"));
        assert!(!names.contains(&"obb"));
    }

    #[test]
    fn convert_polytope_to_capsule_replaces_polytope_in_embedded_psd_file() {
        // Build a unit-cube polytope: extents (1,1,1), longest axis = 0 (tie).
        let vertices = vec![
            HkxValue::F32List(vec![-1.0, -1.0, -1.0, 0.0]),
            HkxValue::F32List(vec![1.0, -1.0, -1.0, 0.0]),
            HkxValue::F32List(vec![-1.0, 1.0, -1.0, 0.0]),
            HkxValue::F32List(vec![1.0, 1.0, 1.0, 0.0]),
        ];
        let mut hkx = HkxFile::from_tagxml(
            12,
            "hk_2015.1.0-r1",
            vec![
                // Embedded blob: PSD root, no hkRootLevelContainer.
                object("hknpPhysicsSystemData", vec![]),
                object(
                    "hknpConvexPolytopeShape",
                    vec![
                        member("vertices", HkxValue::Array(vertices)),
                        member("convexRadius", HkxValue::F32(0.05)),
                    ],
                ),
            ],
        );
        convert_polytope_to_capsule(&mut hkx);
        assert_eq!(hkx.objects()[1].class_name, "hknpCapsuleShape");
        let names: Vec<&str> = hkx.objects()[1]
            .members
            .iter()
            .map(|m| m.name.as_str())
            .collect();
        assert!(names.contains(&"a"));
        assert!(names.contains(&"b"));
        assert!(names.contains(&"convexRadius"));
    }

    #[test]
    fn convert_polytope_to_capsule_skips_standalone_skeleton_files() {
        let mut hkx = HkxFile::from_tagxml(
            12,
            "hk_2015.1.0-r1",
            vec![
                object("hkRootLevelContainer", vec![]),
                object("hknpPhysicsSystemData", vec![]),
                object(
                    "hknpConvexPolytopeShape",
                    vec![member(
                        "vertices",
                        HkxValue::Array(vec![HkxValue::F32List(vec![0.0; 4])]),
                    )],
                ),
            ],
        );
        convert_polytope_to_capsule(&mut hkx);
        // Standalone skeleton.hkx polytopes left untouched.
        assert_eq!(hkx.objects()[2].class_name, "hknpConvexPolytopeShape");
    }

    #[test]
    fn strip_fo76_skeleton_classes_keeps_fo4_supported_properties_objects() {
        let mut hkx = HkxFile::from_tagxml(
            12,
            "hk_2015.1.0-r1",
            vec![
                // Survives; hkRefCountedProperties exists in FO4 classxml.
                object(
                    "hknpCapsuleShape",
                    vec![
                        member("a", HkxValue::F32List(vec![0.0; 4])),
                        member("properties", HkxValue::Pointer(Some(1))),
                    ],
                ),
                object("hkRefCountedProperties", vec![]),
                // Also FO76-only.
                object("hkBitField", vec![]),
            ],
        );
        strip_fo76_skeleton_classes(&mut hkx);
        assert_eq!(hkx.objects().len(), 2);
        let obj = &hkx.objects()[0];
        assert_eq!(obj.class_name, "hknpCapsuleShape");
        assert!(obj.members.iter().all(|m| m.name != "properties"));
        assert_eq!(hkx.objects()[1].class_name, "hkRefCountedProperties");
    }

    #[test]
    fn strip_fo76_skeleton_classes_removes_unsupported_cloth_dependency_graph() {
        let mut hkx = HkxFile::from_tagxml(
            12,
            "hk_2015.1.0-r1",
            vec![
                object(
                    "hclClothState",
                    vec![
                        member("dependencyGraph", HkxValue::Pointer(Some(2))),
                        member(
                            "operators",
                            HkxValue::Array(vec![HkxValue::Pointer(Some(1))]),
                        ),
                    ],
                ),
                object("hclSimulateOperator", vec![]),
                object("hclStateDependencyGraph", vec![]),
            ],
        );

        strip_fo76_skeleton_classes(&mut hkx);

        assert_eq!(hkx.objects().len(), 2);
        assert!(
            hkx.objects()
                .iter()
                .all(|object| object.class_name != "hclStateDependencyGraph")
        );
        let state = &hkx.objects()[0];
        let dependency_graph = state
            .members
            .iter()
            .find(|member| member.name == "dependencyGraph")
            .expect("dependencyGraph member survives until descriptor-based write");
        assert_eq!(dependency_graph.value, HkxValue::Pointer(None));
        let operators = state
            .members
            .iter()
            .find(|member| member.name == "operators")
            .expect("operators member survives");
        assert_eq!(
            operators.value,
            HkxValue::Array(vec![HkxValue::Pointer(Some(1))])
        );
    }

    #[test]
    fn fix_physics_referenced_objects_rebuilds_referenced_objects_from_body_cinfos() {
        let body0 = HkxValue::Object(vec![member("shape", HkxValue::Pointer(Some(2)))]);
        let body1 = HkxValue::Object(vec![member("shape", HkxValue::Pointer(Some(3)))]);
        let mut hkx = HkxFile::from_tagxml(
            12,
            "hk_2015.1.0-r1",
            vec![
                object(
                    "hknpPhysicsSystemData",
                    vec![
                        member("bodyCinfos", HkxValue::Array(vec![body0, body1])),
                        member(
                            "referencedObjects",
                            HkxValue::Array(vec![HkxValue::Pointer(Some(99))]),
                        ),
                    ],
                ),
                object("hknpFiller", vec![]),
                object("hknpCapsuleShape", vec![]),
                object("hknpCapsuleShape", vec![]),
            ],
        );
        fix_physics_referenced_objects(&mut hkx);
        let psd = &hkx.objects()[0];
        let refs = psd
            .members
            .iter()
            .find(|m| m.name == "referencedObjects")
            .unwrap();
        assert_eq!(
            refs.value,
            HkxValue::Array(vec![HkxValue::Pointer(Some(2)), HkxValue::Pointer(Some(3))])
        );
    }

    #[test]
    fn fix_physics_referenced_objects_preserves_constraint_cinfos_data_pointers() {
        let body0 = HkxValue::Object(vec![member("shape", HkxValue::Pointer(Some(2)))]);
        let constraint0 = HkxValue::Object(vec![member("data", HkxValue::Pointer(Some(3)))]);
        let mut hkx = HkxFile::from_tagxml(
            12,
            "hk_2015.1.0-r1",
            vec![
                object(
                    "hknpPhysicsSystemData",
                    vec![
                        member("bodyCinfos", HkxValue::Array(vec![body0])),
                        member("constraintCinfos", HkxValue::Array(vec![constraint0])),
                        member("referencedObjects", HkxValue::Array(vec![])),
                    ],
                ),
                object("hknpFiller", vec![]),
                object("hknpCapsuleShape", vec![]),
                object("hkpRagdollConstraintData", vec![]),
            ],
        );
        fix_physics_referenced_objects(&mut hkx);
        let psd = &hkx.objects()[0];
        let refs = psd
            .members
            .iter()
            .find(|m| m.name == "referencedObjects")
            .unwrap();
        // both shape (idx 2) and constraint data (idx 3) must appear
        assert_eq!(
            refs.value,
            HkxValue::Array(vec![HkxValue::Pointer(Some(2)), HkxValue::Pointer(Some(3))])
        );
    }

    #[test]
    fn fix_physics_referenced_objects_rebuilds_ragdoll_shape_refs_without_nulls() {
        let body0 = HkxValue::Object(vec![member("shape", HkxValue::Pointer(Some(3)))]);
        let body1 = HkxValue::Object(vec![member("shape", HkxValue::Pointer(Some(2)))]);
        let mut hkx = HkxFile::from_tagxml(
            12,
            "hk_2015.1.0-r1",
            vec![
                object(
                    "hknpRagdollData",
                    vec![
                        member("bodyCinfos", HkxValue::Array(vec![body0, body1])),
                        member(
                            "referencedObjects",
                            HkxValue::Array(vec![
                                HkxValue::Pointer(Some(3)),
                                HkxValue::Pointer(None),
                                HkxValue::Pointer(Some(2)),
                                HkxValue::Pointer(None),
                            ]),
                        ),
                    ],
                ),
                object("hknpFiller", vec![]),
                object("hknpCapsuleShape", vec![]),
                object("hknpSphereShape", vec![]),
            ],
        );

        fix_physics_referenced_objects(&mut hkx);

        let refs = hkx.objects()[0]
            .members
            .iter()
            .find(|m| m.name == "referencedObjects")
            .unwrap();
        assert_eq!(
            refs.value,
            HkxValue::Array(vec![HkxValue::Pointer(Some(2)), HkxValue::Pointer(Some(3))])
        );
    }

    #[test]
    fn normalize_bumper_body_cinfos_sets_invalid_id_and_identity_quat() {
        let body = HkxValue::Object(vec![
            member("motionId", HkxValue::U32(0)),
            member("reservedBodyId", HkxValue::Object(vec![])),
            member("orientation", HkxValue::F32(0.0)),
        ]);
        let mut hkx = HkxFile::from_tagxml(
            12,
            "hk_2015.1.0-r1",
            vec![object(
                "hknpPhysicsSystemData",
                vec![member("bodyCinfos", HkxValue::Array(vec![body]))],
            )],
        );
        normalize_bumper_body_cinfos(&mut hkx);
        let bodies = hkx.objects()[0]
            .members
            .iter()
            .find(|m| m.name == "bodyCinfos")
            .unwrap();
        let HkxValue::Array(bodies) = &bodies.value else {
            panic!();
        };
        let HkxValue::Object(body0) = &bodies[0] else {
            panic!();
        };
        let motion_id = body0.iter().find(|m| m.name == "motionId").unwrap();
        assert_eq!(motion_id.value, HkxValue::U32(0x7FFF_FFFF));
        let res_id = body0.iter().find(|m| m.name == "reservedBodyId").unwrap();
        assert_eq!(res_id.value, HkxValue::U32(0x7FFF_FFFF));
        let orient = body0.iter().find(|m| m.name == "orientation").unwrap();
        assert_eq!(orient.value, HkxValue::F32List(vec![0.0, 0.0, 0.0, 1.0]));
    }

    fn body0_motion_id(hkx: &HkxFile) -> u32 {
        let bodies = hkx.objects()[0]
            .members
            .iter()
            .find(|m| m.name == "bodyCinfos")
            .unwrap();
        let HkxValue::Array(bodies) = &bodies.value else {
            panic!("bodyCinfos must be an array");
        };
        let HkxValue::Object(body0) = &bodies[0] else {
            panic!("body0 must be an object");
        };
        let motion_id = body0.iter().find(|m| m.name == "motionId").unwrap();
        extract_int(&motion_id.value).unwrap() as u32
    }

    #[test]
    fn keeps_motion_id_when_dynamic_motion_is_wired() {
        // Migrated weapon PSD: body0 motionId=0 -> motionCinfos[0].inverseMass=2.0
        // (a real dynamic motion the compound→PSD migrate pass wired). The
        // migrate pass emits motionCinfos as TypedObject("hknpMotionCinfo"),
        // so the fixture must too — an Object-only guard would no-op here.
        let body = HkxValue::Object(vec![member("motionId", HkxValue::U32(0))]);
        let motion = HkxValue::TypedObject {
            class_name: "hknpMotionCinfo".to_string(),
            members: vec![member("inverseMass", HkxValue::F32(2.0))],
        };
        let mut hkx = HkxFile::from_tagxml(
            12,
            "hk_2015.1.0-r1",
            vec![object(
                "hknpPhysicsSystemData",
                vec![
                    member("bodyCinfos", HkxValue::Array(vec![body])),
                    member("motionCinfos", HkxValue::Array(vec![motion])),
                ],
            )],
        );
        normalize_bumper_body_cinfos(&mut hkx);
        assert_eq!(
            body0_motion_id(&hkx),
            0,
            "wired dynamic motion must be preserved"
        );
    }

    #[test]
    fn still_invalidates_motion_id_for_bumper_without_motion() {
        // Genuine bumper PSD: body0 motionId=0 but no resolvable dynamic motion
        // (inverseMass=0 → infinite mass / keyframed bumper).
        let body = HkxValue::Object(vec![member("motionId", HkxValue::U32(0))]);
        let motion = HkxValue::Object(vec![member("inverseMass", HkxValue::F32(0.0))]);
        let mut hkx = HkxFile::from_tagxml(
            12,
            "hk_2015.1.0-r1",
            vec![object(
                "hknpPhysicsSystemData",
                vec![
                    member("bodyCinfos", HkxValue::Array(vec![body])),
                    member("motionCinfos", HkxValue::Array(vec![motion])),
                ],
            )],
        );
        normalize_bumper_body_cinfos(&mut hkx);
        assert_eq!(
            body0_motion_id(&hkx),
            0x7FFF_FFFF,
            "motionless bumper still gets INVALID"
        );
    }

    #[test]
    fn blend_hint_survives_the_route_unchanged() {
        // ADDITIVE_DEPRECATED (1) and ADDITIVE (2) are distinct additive
        // conventions in both games' schema, so promoting 1→2 silently reframes
        // the delta from parent space to bone-local space.
        let mut hkx = HkxFile::from_tagxml(
            12,
            "hk_2015.1.0-r1",
            vec![
                object(
                    "hkaAnimationBinding",
                    vec![member("blendHint", HkxValue::I32(1))],
                ),
                object(
                    "hkaAnimationBinding",
                    vec![member("blendHint", HkxValue::I32(2))],
                ),
                object(
                    "hkaAnimationBinding",
                    vec![member("blendHint", HkxValue::I32(0))],
                ),
            ],
        );
        apply_for_test(&mut hkx);
        let hints: Vec<Option<i32>> = hkx
            .objects()
            .iter()
            .filter(|object| object.class_name == "hkaAnimationBinding")
            .map(|object| {
                object
                    .members
                    .iter()
                    .find(|member| member.name == "blendHint")
                    .and_then(|member| extract_int(&member.value))
            })
            .collect();
        assert_eq!(hints, vec![Some(1), Some(2), Some(0)]);
    }

    // ── Pipeline-progression tests ────────────────────────────────────────────

    #[test]
    fn apply_implemented_transforms_reports_writer_blocker_after_character_fixture_noops() {
        let mut hkx = HkxFile::from_tagxml(
            12,
            "hk_2015.1.0-r1",
            vec![object("hkbCharacterData", vec![])],
        );

        let last_applied = apply_for_test(&mut hkx);

        assert_eq!(last_applied, 41);
        assert_eq!(
            ALWAYS_ON_TRANSFORMS[last_applied],
            "_fix_behavior_variable_infos"
        );
    }

    // ── Compound-shape / capsule transform tests ─────────────────────────────

    #[test]
    fn compute_capsule_endpoints_returns_endpoints_for_unit_box_planes() {
        // 6 axis-aligned planes forming a 2x2x2 cube centered at origin.
        let planes: Vec<[f32; 4]> = vec![
            [1.0, 0.0, 0.0, -1.0],
            [-1.0, 0.0, 0.0, -1.0],
            [0.0, 1.0, 0.0, -1.0],
            [0.0, -1.0, 0.0, -1.0],
            [0.0, 0.0, 1.0, -1.0],
            [0.0, 0.0, -1.0, -1.0],
        ];
        let result = compute_capsule_endpoints(&planes);
        assert!(result.is_some());
        let (a, b) = result.unwrap();
        // Symmetric across origin along the longest axis.
        for i in 0..3 {
            assert!(
                (a[i] + b[i]).abs() < 1e-5,
                "should be symmetric on axis {i}"
            );
        }
    }

    #[test]
    fn compute_capsule_endpoints_returns_none_for_unbalanced_planes() {
        // Only 3 planes — no opposing pairs.
        let planes: Vec<[f32; 4]> = vec![
            [1.0, 0.0, 0.0, -1.0],
            [0.0, 1.0, 0.0, -1.0],
            [0.0, 0.0, 1.0, -1.0],
        ];
        assert!(compute_capsule_endpoints(&planes).is_none());
    }

    #[test]
    fn compute_capsule_endpoints_picks_longest_axis() {
        // Box 4 along X (-2..2), 2 along Y/Z. Longest axis is X.
        let planes: Vec<[f32; 4]> = vec![
            [1.0, 0.0, 0.0, -2.0],
            [-1.0, 0.0, 0.0, -2.0],
            [0.0, 1.0, 0.0, -1.0],
            [0.0, -1.0, 0.0, -1.0],
            [0.0, 0.0, 1.0, -1.0],
            [0.0, 0.0, -1.0, -1.0],
        ];
        let (a, b) = compute_capsule_endpoints(&planes).unwrap();
        // Endpoints are (±2, 0, 0).
        assert!((a[0] + 2.0).abs() < 1e-5 || (a[0] - 2.0).abs() < 1e-5);
        assert!(a[1].abs() < 1e-5);
        assert!(a[2].abs() < 1e-5);
        assert!(b[1].abs() < 1e-5);
        assert!(b[2].abs() < 1e-5);
    }

    #[test]
    fn convert_limited_hinge_to_ragdoll_preserves_limited_hinges() {
        let atoms = HkxValue::Object(vec![
            member("transforms", HkxValue::F32List(vec![1.0; 12])),
            member("setupStabilization", HkxValue::Object(vec![])),
            member(
                "angMotor",
                HkxValue::Object(vec![member("isEnabled", HkxValue::Bool(true))]),
            ),
            member(
                "angFriction",
                HkxValue::Object(vec![member("numFrictionAxes", HkxValue::I32(1))]),
            ),
            member(
                "angLimit",
                HkxValue::Object(vec![
                    member("minAngle", HkxValue::F32(-0.7)),
                    member("maxAngle", HkxValue::F32(0.9)),
                ]),
            ),
            member("ballSocket", HkxValue::Object(vec![])),
        ]);
        let mut hkx = HkxFile::from_tagxml(
            12,
            "hk_2015.1.0-r1",
            vec![object(
                "hkpLimitedHingeConstraintData",
                vec![member("atoms", atoms)],
            )],
        );

        convert_limited_hinge_to_ragdoll(&mut hkx);

        let obj = &hkx.objects()[0];
        assert_eq!(obj.class_name, "hkpLimitedHingeConstraintData");
        let atoms_member = obj.members.iter().find(|m| m.name == "atoms").unwrap();
        let HkxValue::Object(atoms_members) = &atoms_member.value else {
            panic!();
        };
        let names: Vec<&str> = atoms_members.iter().map(|m| m.name.as_str()).collect();
        assert!(names.contains(&"transforms"));
        assert!(names.contains(&"setupStabilization"));
        assert!(names.contains(&"angMotor"));
        assert!(names.contains(&"angFriction"));
        assert!(names.contains(&"angLimit"));
        assert!(names.contains(&"ballSocket"));
        assert!(!names.contains(&"ragdollMotors"));
        let af = atoms_members
            .iter()
            .find(|m| m.name == "angFriction")
            .unwrap();
        let HkxValue::Object(af_members) = &af.value else {
            panic!();
        };
        let nf = af_members
            .iter()
            .find(|m| m.name == "numFrictionAxes")
            .unwrap();
        assert_eq!(nf.value, HkxValue::I32(1));
    }

    #[test]
    fn flatten_compound_shapes_in_psd_replaces_compound_with_leaves() {
        // PSD pointing at a compound shape (idx 1) which has two leaf
        // capsules (idx 2, 3) that are unreferenced by bodyCinfos.
        let body0 = HkxValue::Object(vec![
            member("shape", HkxValue::Pointer(Some(1))),
            member("motionId", HkxValue::U32(0)),
        ]);
        let psd = object(
            "hknpPhysicsSystemData",
            vec![
                member("bodyCinfos", HkxValue::Array(vec![body0])),
                member("materials", HkxValue::Array(vec![])),
                member(
                    "referencedObjects",
                    HkxValue::Array(vec![HkxValue::Pointer(Some(2))]),
                ),
            ],
        );
        let mut hkx = HkxFile::from_tagxml(
            12,
            "hk_2015.1.0-r1",
            vec![
                psd,
                object("hknpDynamicCompoundShape", vec![]),
                object("hknpCapsuleShape", vec![]),
                object("hknpCapsuleShape", vec![]),
            ],
        );

        flatten_compound_shapes_in_psd(&mut hkx);

        // Compound shape stripped; should have PSD + 2 capsules.
        assert_eq!(hkx.objects().len(), 3);
        let psd = &hkx.objects()[0];
        assert_eq!(psd.class_name, "hknpPhysicsSystemData");
        let bodies = psd.members.iter().find(|m| m.name == "bodyCinfos").unwrap();
        let HkxValue::Array(bodies) = &bodies.value else {
            panic!();
        };
        assert_eq!(bodies.len(), 2, "one bodyCinfo per leaf shape");
        // Each new bodyCinfo points at a unique leaf shape (the new
        // indices 1 and 2 after compound-shape removal + remap).
        for body in bodies {
            let Some(members) = body.as_object_members() else {
                panic!();
            };
            let shape = members.iter().find(|m| m.name == "shape").unwrap();
            let HkxValue::Pointer(Some(idx)) = shape.value else {
                panic!("shape pointer should be Some");
            };
            assert!(
                hkx.objects()[idx].class_name == "hknpCapsuleShape",
                "shape pointer should target a capsule"
            );
        }
    }

    #[test]
    fn flatten_compound_shapes_in_psd_skips_files_with_root_container() {
        let mut hkx = HkxFile::from_tagxml(
            12,
            "hk_2015.1.0-r1",
            vec![
                object("hkRootLevelContainer", vec![]),
                object(
                    "hknpPhysicsSystemData",
                    vec![member(
                        "bodyCinfos",
                        HkxValue::Array(vec![HkxValue::Object(vec![member(
                            "shape",
                            HkxValue::Pointer(Some(2)),
                        )])]),
                    )],
                ),
                object("hknpDynamicCompoundShape", vec![]),
                object("hknpCapsuleShape", vec![]),
            ],
        );
        let before = hkx.objects().len();
        flatten_compound_shapes_in_psd(&mut hkx);
        assert_eq!(hkx.objects().len(), before, "skeleton blob untouched");
    }

    #[test]
    fn migrate_compound_shape_to_physics_system_wraps_polytopes_in_psd() {
        // Source: 2 polytope shapes, no PSD root.
        let mut hkx = HkxFile::from_tagxml(
            12,
            "hk_2015.1.0-r1",
            vec![
                object("hknpConvexPolytopeShape", vec![]),
                object("hknpConvexPolytopeShape", vec![]),
            ],
        );

        migrate_compound_shape_to_physics_system(&mut hkx);

        // Now should have a PSD root + the 2 shapes.
        let classes: Vec<&str> = hkx
            .objects()
            .iter()
            .map(|o| o.class_name.as_str())
            .collect();
        assert!(
            classes.contains(&"hknpPhysicsSystemData"),
            "PSD root must be installed at object[0]"
        );
        assert_eq!(hkx.objects()[0].class_name, "hknpPhysicsSystemData");
        let polytope_count = classes
            .iter()
            .filter(|c| **c == "hknpConvexPolytopeShape")
            .count();
        assert_eq!(polytope_count, 2);

        // Body cinfos should target real shape indices (>0).
        let psd = &hkx.objects()[0];
        let bodies = psd.members.iter().find(|m| m.name == "bodyCinfos").unwrap();
        let HkxValue::Array(bodies) = &bodies.value else {
            panic!();
        };
        assert_eq!(bodies.len(), 2);
        for body in bodies {
            let members = body.as_object_members().unwrap();
            let shape = members.iter().find(|m| m.name == "shape").unwrap();
            let HkxValue::Pointer(Some(idx)) = shape.value else {
                panic!("body.shape must be a real pointer");
            };
            assert!(idx > 0 && idx < hkx.objects().len());
            assert_eq!(hkx.objects()[idx].class_name, "hknpConvexPolytopeShape");
        }
    }

    #[test]
    fn migrate_compound_shape_to_physics_system_skips_files_with_existing_psd() {
        let mut hkx = HkxFile::from_tagxml(
            12,
            "hk_2015.1.0-r1",
            vec![
                object("hknpPhysicsSystemData", vec![]),
                object("hknpConvexPolytopeShape", vec![]),
            ],
        );
        let before = hkx.objects().len();
        migrate_compound_shape_to_physics_system(&mut hkx);
        assert_eq!(hkx.objects().len(), before);
    }

    #[test]
    fn migrate_compound_shape_to_physics_system_skips_files_with_no_polytope() {
        let mut hkx = HkxFile::from_tagxml(
            12,
            "hk_2015.1.0-r1",
            vec![object("hknpCapsuleShape", vec![])],
        );
        let before = hkx.objects().len();
        migrate_compound_shape_to_physics_system(&mut hkx);
        assert_eq!(hkx.objects().len(), before);
    }

    #[test]
    fn remap_pointers_walks_arrays_and_typed_objects() {
        let mut value = HkxValue::Array(vec![
            HkxValue::Pointer(Some(1)),
            HkxValue::TypedObject {
                class_name: "hkpRagdollMotorConstraintAtom".to_string(),
                members: vec![member("motors", HkxValue::Pointer(Some(2)))],
            },
            HkxValue::Object(vec![member("inner", HkxValue::Pointer(Some(3)))]),
        ]);
        let mut map = std::collections::HashMap::new();
        map.insert(1, 100);
        map.insert(2, 200);
        map.insert(3, 300);
        remap_pointers(&mut value, &map);

        let HkxValue::Array(values) = &value else {
            panic!();
        };
        assert_eq!(values[0], HkxValue::Pointer(Some(100)));
        let HkxValue::TypedObject { members, .. } = &values[1] else {
            panic!();
        };
        assert_eq!(members[0].value, HkxValue::Pointer(Some(200)));
        let HkxValue::Object(omembers) = &values[2] else {
            panic!();
        };
        assert_eq!(omembers[0].value, HkxValue::Pointer(Some(300)));
    }

    #[test]
    fn trailing_controller_body_pruning_preserves_mapped_or_constrained_bodies() {
        let named_body = |name: &str| {
            HkxValue::Object(vec![member(
                "name",
                HkxValue::String {
                    value: name.to_string(),
                    is_null: false,
                },
            )])
        };
        let constraint = |body_a: u32, body_b: u32| {
            HkxValue::Object(vec![
                member("bodyA", HkxValue::U32(body_a)),
                member("bodyB", HkxValue::U32(body_b)),
            ])
        };
        let mut hkx = HkxFile::from_tagxml(
            12,
            "hk_2015.1.0-r1",
            vec![
                object(
                    "hknpRagdollData",
                    vec![
                        member(
                            "boneToBodyMap",
                            HkxValue::Array(vec![HkxValue::I32(0), HkxValue::I32(1)]),
                        ),
                        member(
                            "bodyCinfos",
                            HkxValue::Array(vec![
                                named_body("Ragdoll_COM"),
                                named_body("Ragdoll_Head"),
                                named_body("CharacterBumper"),
                                named_body("CharacterController"),
                            ]),
                        ),
                        member(
                            "motionCinfos",
                            HkxValue::Array(vec![
                                HkxValue::Object(vec![]),
                                HkxValue::Object(vec![]),
                                HkxValue::Object(vec![]),
                                HkxValue::Object(vec![]),
                            ]),
                        ),
                        member("constraintCinfos", HkxValue::Array(vec![constraint(1, 0)])),
                    ],
                ),
                object(
                    "hknpRagdollData",
                    vec![
                        member("boneToBodyMap", HkxValue::Array(vec![HkxValue::I32(0)])),
                        member(
                            "bodyCinfos",
                            HkxValue::Array(vec![
                                named_body("Ragdoll_COM"),
                                named_body("CharacterBumper"),
                            ]),
                        ),
                        member("constraintCinfos", HkxValue::Array(vec![constraint(1, 0)])),
                    ],
                ),
            ],
        );

        strip_unmapped_trailing_ragdoll_controller_bodies(&mut hkx);

        let array_len = |object_index: usize, member_name: &str| {
            let member = hkx.objects()[object_index]
                .members
                .iter()
                .find(|member| member.name == member_name)
                .unwrap();
            let HkxValue::Array(values) = &member.value else {
                panic!("{member_name} must be an array")
            };
            values.len()
        };
        assert_eq!(array_len(0, "bodyCinfos"), 2);
        assert_eq!(array_len(0, "motionCinfos"), 2);
        assert_eq!(array_len(1, "bodyCinfos"), 2);
    }

    #[test]
    fn ragdoll_controller_bodies_are_preserved_as_static_without_motions() {
        let body = |name: &str, motion_properties_id: u16| {
            HkxValue::Object(vec![
                member(
                    "name",
                    HkxValue::String {
                        value: name.to_string(),
                        is_null: false,
                    },
                ),
                member("flags", HkxValue::U32(0)),
                member("motionId", HkxValue::U32(99)),
                member("reservedBodyId", HkxValue::U32(0)),
                member("motionPropertiesId", HkxValue::U16(motion_properties_id)),
                member("mass", HkxValue::F32(2.0)),
                member("position", HkxValue::F32List(vec![0.0, 0.0, 0.0, 0.0])),
                member("orientation", HkxValue::F32List(vec![0.0, 0.0, 0.0, 1.0])),
            ])
        };
        let mut hkx = HkxFile::from_tagxml(
            12,
            "hk_2015.1.0-r1",
            vec![object(
                "hknpRagdollData",
                vec![
                    member(
                        "boneToBodyMap",
                        HkxValue::Array(vec![HkxValue::I32(0), HkxValue::I32(1)]),
                    ),
                    member(
                        "bodyCinfos",
                        HkxValue::Array(vec![
                            body("Ragdoll_COM", 2),
                            body("Ragdoll_Head", 3),
                            body("CharacterBumper", 0),
                            body("CharacterController", 1),
                        ]),
                    ),
                    member(
                        "motionProperties",
                        HkxValue::Array(vec![
                            HkxValue::Object(vec![]),
                            HkxValue::Object(vec![]),
                            HkxValue::Object(vec![]),
                            HkxValue::Object(vec![]),
                        ]),
                    ),
                ],
            )],
        );

        normalize_ragdoll_body_cinfos(&mut hkx);
        synthesize_motion_cinfos(&mut hkx, &std::collections::HashMap::new());

        let ragdoll = &hkx.objects()[0];
        let bodies = ragdoll
            .members
            .iter()
            .find(|member| member.name == "bodyCinfos")
            .unwrap();
        let HkxValue::Array(bodies) = &bodies.value else {
            panic!("bodyCinfos must be an array")
        };
        assert_eq!(bodies.len(), 4);

        let motion_ids: Vec<i32> = bodies
            .iter()
            .map(|body| {
                body.as_object_members()
                    .and_then(|members| members.iter().find(|member| member.name == "motionId"))
                    .and_then(|member| extract_int(&member.value))
                    .expect("motionId")
            })
            .collect();
        assert_eq!(motion_ids, vec![0, 1, 0x7FFF_FFFF, 0x7FFF_FFFF]);

        for body in &bodies[2..] {
            let flags = body
                .as_object_members()
                .and_then(|members| members.iter().find(|member| member.name == "flags"))
                .and_then(|member| extract_int(&member.value));
            assert_eq!(flags, Some(1));
        }

        let motions = ragdoll
            .members
            .iter()
            .find(|member| member.name == "motionCinfos")
            .expect("motionCinfos");
        let HkxValue::Array(motions) = &motions.value else {
            panic!("motionCinfos must be an array")
        };
        assert_eq!(motions.len(), 2);
    }

    // ── normalize_ragdoll_body_cinfos ───────────────────────────

    #[test]
    fn normalize_ragdoll_body_cinfos_sets_sequential_motion_id_and_invalid_reserved() {
        let body0 = HkxValue::Object(vec![
            member("motionId", HkxValue::U32(99)),
            member("reservedBodyId", HkxValue::Object(vec![])),
            member("orientation", HkxValue::F32(0.0)),
        ]);
        let body1 = HkxValue::Object(vec![
            member("motionId", HkxValue::U32(99)),
            member("reservedBodyId", HkxValue::U32(0)),
            member("orientation", HkxValue::F32(0.0)),
        ]);
        let mut hkx = HkxFile::from_tagxml(
            12,
            "hk_2015.1.0-r1",
            vec![object(
                "hknpRagdollData",
                vec![member("bodyCinfos", HkxValue::Array(vec![body0, body1]))],
            )],
        );
        normalize_ragdoll_body_cinfos(&mut hkx);
        let bodies = hkx.objects()[0]
            .members
            .iter()
            .find(|m| m.name == "bodyCinfos")
            .unwrap();
        let HkxValue::Array(bodies) = &bodies.value else {
            panic!()
        };

        // body[0]: motionId=0, reservedBodyId=INVALID, orientation=identity
        let HkxValue::Object(b0) = &bodies[0] else {
            panic!()
        };
        assert_eq!(
            b0.iter().find(|m| m.name == "motionId").unwrap().value,
            HkxValue::U32(0)
        );
        assert_eq!(
            b0.iter()
                .find(|m| m.name == "reservedBodyId")
                .unwrap()
                .value,
            HkxValue::U32(0x7FFF_FFFF)
        );
        assert_eq!(
            b0.iter().find(|m| m.name == "orientation").unwrap().value,
            HkxValue::F32List(vec![0.0, 0.0, 0.0, 1.0])
        );

        // body[1]: motionId=1
        let HkxValue::Object(b1) = &bodies[1] else {
            panic!()
        };
        assert_eq!(
            b1.iter().find(|m| m.name == "motionId").unwrap().value,
            HkxValue::U32(1)
        );
        assert_eq!(
            b1.iter()
                .find(|m| m.name == "reservedBodyId")
                .unwrap()
                .value,
            HkxValue::U32(0x7FFF_FFFF)
        );
    }

    #[test]
    fn normalize_ragdoll_body_cinfos_preserves_valid_source_orientation() {
        let skeleton = HkxObject {
            name: Some("#skel".to_string()),
            offset: 0,
            signature: 0,
            class_name: "hkaSkeleton".to_string(),
            members: vec![member(
                "referencePose",
                HkxValue::Array(vec![
                    HkxValue::F32List(vec![
                        0.0, 0.0, 0.0, 0.0, //
                        0.0, 0.0, 0.0, 1.0, //
                        1.0, 1.0, 1.0, 0.0,
                    ]),
                    HkxValue::F32List(vec![
                        0.0, 0.0, 0.0, 0.0, //
                        0.0, 0.0, -0.433327, 0.901237, //
                        1.0, 1.0, 1.0, 0.0,
                    ]),
                ]),
            )],
        };
        let body0 = HkxValue::Object(vec![member(
            "orientation",
            HkxValue::F32List(vec![0.0, 0.0, 0.0, 1.0]),
        )]);
        let body1 = HkxValue::Object(vec![member(
            "orientation",
            HkxValue::F32List(vec![-0.179972, 0.620723, -0.246546, 0.722169]),
        )]);
        let ragdoll = object(
            "hknpRagdollData",
            vec![
                member("skeleton", HkxValue::Pointer(Some(0))),
                member(
                    "boneToBodyMap",
                    HkxValue::Array(vec![HkxValue::I32(0), HkxValue::I32(1)]),
                ),
                member("bodyCinfos", HkxValue::Array(vec![body0, body1])),
            ],
        );
        let mut hkx = HkxFile::from_tagxml(12, "hk_2015.1.0-r1", vec![skeleton, ragdoll]);

        normalize_ragdoll_body_cinfos(&mut hkx);

        let rd = &hkx.objects()[1];
        let bodies = rd.members.iter().find(|m| m.name == "bodyCinfos").unwrap();
        let HkxValue::Array(bodies) = &bodies.value else {
            panic!()
        };
        let HkxValue::Object(body1_members) = &bodies[1] else {
            panic!()
        };
        assert_eq!(
            body1_members
                .iter()
                .find(|m| m.name == "orientation")
                .unwrap()
                .value,
            HkxValue::F32List(vec![-0.179972, 0.620723, -0.246546, 0.722169])
        );
    }

    #[test]
    fn normalize_ragdoll_body_cinfos_skips_hknp_physics_system_data() {
        // Must not touch hknpPhysicsSystemData — that's normalize_bumper_body_cinfos.
        let body = HkxValue::Object(vec![member("motionId", HkxValue::U32(0))]);
        let mut hkx = HkxFile::from_tagxml(
            12,
            "hk_2015.1.0-r1",
            vec![object(
                "hknpPhysicsSystemData",
                vec![member("bodyCinfos", HkxValue::Array(vec![body]))],
            )],
        );
        let before = hkx.objects().to_vec();
        normalize_ragdoll_body_cinfos(&mut hkx);
        // motionId should still be 0 — bumper transform (not ragdoll) handles PSD.
        assert_eq!(hkx.objects(), before.as_slice());
    }

    #[test]
    fn normalize_ragdoll_body_position_w_preserves_motion_center_lane() {
        let body = HkxValue::Object(vec![member(
            "position",
            HkxValue::F32List(vec![0.1, -0.7, 0.3, -0.089]),
        )]);
        let motion = HkxValue::TypedObject {
            class_name: "hknpMotionCinfo".to_string(),
            members: vec![member(
                "centerOfMassWorld",
                HkxValue::F32List(vec![0.2, -0.7, 0.3, -0.089]),
            )],
        };
        let mut hkx = HkxFile::from_tagxml(
            12,
            "hk_2015.1.0-r1",
            vec![object(
                "hknpRagdollData",
                vec![
                    member("bodyCinfos", HkxValue::Array(vec![body])),
                    member("motionCinfos", HkxValue::Array(vec![motion])),
                ],
            )],
        );

        normalize_ragdoll_body_position_w(&mut hkx);

        let ragdoll = &hkx.objects()[0];
        let HkxValue::Array(bodies) = &ragdoll
            .members
            .iter()
            .find(|m| m.name == "bodyCinfos")
            .unwrap()
            .value
        else {
            panic!("bodyCinfos should be an array");
        };
        let Some(body_members) = bodies[0].as_object_members() else {
            panic!("body cinfo should be an object");
        };
        assert_eq!(
            body_members
                .iter()
                .find(|m| m.name == "position")
                .unwrap()
                .value,
            HkxValue::F32List(vec![0.1, -0.7, 0.3, 0.0])
        );

        let HkxValue::Array(motions) = &ragdoll
            .members
            .iter()
            .find(|m| m.name == "motionCinfos")
            .unwrap()
            .value
        else {
            panic!("motionCinfos should be an array");
        };
        let Some(motion_members) = motions[0].as_object_members() else {
            panic!("motion cinfo should be an object");
        };
        assert_eq!(
            motion_members
                .iter()
                .find(|m| m.name == "centerOfMassWorld")
                .unwrap()
                .value,
            HkxValue::F32List(vec![0.2, -0.7, 0.3, -0.089])
        );
    }

    #[test]
    fn normalize_ragdoll_constraint_offsets_uses_fo4_cone_layout() {
        let cone_limit =
            HkxValue::Object(vec![member("memOffsetToAngleOffset", HkxValue::I16(160))]);
        let atoms = HkxValue::Object(vec![
            member("coneLimit", cone_limit),
            member(
                "planesLimit",
                HkxValue::Object(vec![member("memOffsetToAngleOffset", HkxValue::I16(0))]),
            ),
        ]);
        let mut hkx = HkxFile::from_tagxml(
            12,
            "hk_2015.1.0-r1",
            vec![object(
                "hkpRagdollConstraintData",
                vec![member("atoms", atoms)],
            )],
        );

        normalize_ragdoll_constraint_offsets(&mut hkx);

        let atoms = hkx.objects()[0].members[0]
            .value
            .as_object_members()
            .expect("atoms object");
        let cone_limit = atoms
            .iter()
            .find(|member| member.name == "coneLimit")
            .and_then(|member| member.value.as_object_members())
            .expect("cone limit object");
        assert_eq!(
            extract_int(
                &cone_limit
                    .iter()
                    .find(|member| member.name == "memOffsetToAngleOffset")
                    .expect("cone offset")
                    .value
            ),
            Some(56)
        );
        let planes_limit = atoms
            .iter()
            .find(|member| member.name == "planesLimit")
            .and_then(|member| member.value.as_object_members())
            .expect("planes limit object");
        assert_eq!(
            extract_int(
                &planes_limit
                    .iter()
                    .find(|member| member.name == "memOffsetToAngleOffset")
                    .expect("planes offset")
                    .value
            ),
            Some(0)
        );
    }

    // ── inject_ragdoll_motors ──────────────────────────────────

    #[test]
    fn inject_ragdoll_motors_creates_motor_object_and_wires_pointer() {
        let ragdoll_motors = HkxValue::Object(vec![member("motors", HkxValue::Pointer(None))]);
        let atoms = HkxValue::Object(vec![member("ragdollMotors", ragdoll_motors)]);
        let mut hkx = HkxFile::from_tagxml(
            12,
            "hk_2015.1.0-r1",
            vec![object(
                "hkpRagdollConstraintData",
                vec![member("atoms", atoms)],
            )],
        );

        inject_ragdoll_motors(&mut hkx);

        // A hkpPositionConstraintMotor should have been created.
        let motor_idx = hkx
            .objects()
            .iter()
            .position(|o| o.class_name == "hkpPositionConstraintMotor")
            .expect("motor object must be created");

        // The motors pointer inside ragdollMotors should now point to it.
        let constraint = &hkx.objects()[0];
        let atoms_m = constraint
            .members
            .iter()
            .find(|m| m.name == "atoms")
            .unwrap();
        let Some(atoms_members) = atoms_m.value.as_object_members() else {
            panic!()
        };
        let rm_atom = atoms_members
            .iter()
            .find(|m| m.name == "ragdollMotors")
            .unwrap();
        let Some(rm_members) = rm_atom.value.as_object_members() else {
            panic!()
        };
        let motors = rm_members.iter().find(|m| m.name == "motors").unwrap();
        assert_eq!(
            motors.value,
            HkxValue::Array(vec![
                HkxValue::Pointer(Some(motor_idx)),
                HkxValue::Pointer(Some(motor_idx)),
                HkxValue::Pointer(Some(motor_idx)),
            ])
        );

        // Motor object has correct class and key parameters.
        let motor = &hkx.objects()[motor_idx];
        assert_eq!(motor.class_name, "hkpPositionConstraintMotor");
        let tau = motor.members.iter().find(|m| m.name == "tau").unwrap();
        assert_eq!(tau.value, HkxValue::F32(0.8));
    }

    #[test]
    fn inject_ragdoll_motors_is_noop_when_no_constraints_present() {
        let mut hkx = HkxFile::from_tagxml(
            12,
            "hk_2015.1.0-r1",
            vec![object("hknpRagdollData", vec![])],
        );
        let before_len = hkx.objects().len();
        inject_ragdoll_motors(&mut hkx);
        assert_eq!(hkx.objects().len(), before_len);
    }

    // ── synthesize_motion_cinfos ───────────────────────────────

    #[test]
    fn synthesize_motion_cinfos_emits_typed_objects_with_correct_class_name() {
        let body = HkxValue::Object(vec![
            member("flags", HkxValue::I32(128)), // dynamic
            member("mass", HkxValue::F32(5.0)),
            member("position", HkxValue::F32List(vec![1.0, 2.0, 3.0, 0.0])),
            member("orientation", HkxValue::F32List(vec![0.0, 0.0, 0.0, 1.0])),
            member(
                "linearVelocity",
                HkxValue::F32List(vec![0.0, 0.0, 0.0, 0.0]),
            ),
            member(
                "angularVelocity",
                HkxValue::F32List(vec![0.0, 0.0, 0.0, 0.0]),
            ),
        ]);
        let mut hkx = HkxFile::from_tagxml(
            12,
            "hk_2015.1.0-r1",
            vec![object(
                "hknpPhysicsSystemData",
                vec![member("bodyCinfos", HkxValue::Array(vec![body]))],
            )],
        );

        synthesize_motion_cinfos(&mut hkx, &std::collections::HashMap::new());

        let motion_arr = hkx.objects()[0]
            .members
            .iter()
            .find(|m| m.name == "motionCinfos")
            .expect("motionCinfos must be created");
        let HkxValue::Array(entries) = &motion_arr.value else {
            panic!()
        };
        assert_eq!(entries.len(), 1, "one motionCinfo per body");

        let HkxValue::TypedObject {
            class_name,
            members,
        } = &entries[0]
        else {
            panic!("motionCinfo must be TypedObject");
        };
        assert_eq!(class_name, "hknpMotionCinfo");

        // inverseMass = 1/mass = 0.2
        let inv_mass_m = members.iter().find(|m| m.name == "inverseMass").unwrap();
        if let HkxValue::F32(v) = inv_mass_m.value {
            assert!(
                (v - 0.2).abs() < 1e-5,
                "inverseMass should be ~0.2, got {v}"
            );
        } else {
            panic!("inverseMass must be F32");
        }

        // massFactor = mass = 5.0
        let mf = members.iter().find(|m| m.name == "massFactor").unwrap();
        assert_eq!(mf.value, HkxValue::F32(5.0));

        let motion_properties_id = members
            .iter()
            .find(|m| m.name == "motionPropertiesId")
            .unwrap();
        assert_eq!(motion_properties_id.value, HkxValue::U16(0));

        // centerOfMassWorld = position (no mass dist target)
        let com = members
            .iter()
            .find(|m| m.name == "centerOfMassWorld")
            .unwrap();
        assert_eq!(com.value, HkxValue::F32List(vec![1.0, 2.0, 3.0, 0.0]));
    }

    #[test]
    fn synthesize_motion_cinfos_preserves_nonsequential_motion_properties_ids() {
        let bodies = [0_u16, 3, 1]
            .into_iter()
            .map(|motion_properties_id| {
                HkxValue::Object(vec![member(
                    "motionPropertiesId",
                    HkxValue::U16(motion_properties_id),
                )])
            })
            .collect();
        let motion_properties = (0..4).map(|_| HkxValue::Object(vec![])).collect();
        let mut hkx = HkxFile::from_tagxml(
            12,
            "hk_2015.1.0-r1",
            vec![object(
                "hknpRagdollData",
                vec![
                    member("motionProperties", HkxValue::Array(motion_properties)),
                    member("bodyCinfos", HkxValue::Array(bodies)),
                ],
            )],
        );

        synthesize_motion_cinfos(&mut hkx, &std::collections::HashMap::new());

        let motion_arr = hkx.objects()[0]
            .members
            .iter()
            .find(|m| m.name == "motionCinfos")
            .expect("motionCinfos must be created");
        let HkxValue::Array(entries) = &motion_arr.value else {
            panic!()
        };
        let ids: Vec<i32> = entries
            .iter()
            .map(|entry| {
                let HkxValue::TypedObject { members, .. } = entry else {
                    panic!("entry must be TypedObject")
                };
                members
                    .iter()
                    .find(|m| m.name == "motionPropertiesId")
                    .and_then(|m| extract_int(&m.value))
                    .expect("motionPropertiesId must be present")
            })
            .collect();
        assert_eq!(ids, vec![0, 3, 1]);
    }

    #[test]
    fn synthesize_motion_cinfos_skips_non_dynamic_psd_bodies() {
        // flags=16 (bumper) → no dynamic body → motionCinfos should stay empty.
        let body = HkxValue::Object(vec![
            member("flags", HkxValue::I32(16)),
            member("mass", HkxValue::F32(1.0)),
        ]);
        let mut hkx = HkxFile::from_tagxml(
            12,
            "hk_2015.1.0-r1",
            vec![object(
                "hknpPhysicsSystemData",
                vec![member("bodyCinfos", HkxValue::Array(vec![body]))],
            )],
        );
        synthesize_motion_cinfos(&mut hkx, &std::collections::HashMap::new());
        // motionCinfos should not have been inserted.
        let has_motion_arr = hkx.objects()[0]
            .members
            .iter()
            .any(|m| m.name == "motionCinfos");
        assert!(
            !has_motion_arr,
            "non-dynamic PSD bodies must not get motionCinfos"
        );
    }

    #[test]
    fn synthesize_motion_cinfos_ragdoll_always_synthesizes() {
        // hknpRagdollData always gets motionCinfos regardless of flags.
        let body = HkxValue::Object(vec![
            member("flags", HkxValue::I32(16)), // would skip for PSD
            member("mass", HkxValue::F32(2.0)),
            member("position", HkxValue::F32List(vec![0.0, 0.0, 0.0, 0.0])),
            member("orientation", HkxValue::F32List(vec![0.0, 0.0, 0.0, 1.0])),
            member(
                "linearVelocity",
                HkxValue::F32List(vec![0.0, 0.0, 0.0, 0.0]),
            ),
            member(
                "angularVelocity",
                HkxValue::F32List(vec![0.0, 0.0, 0.0, 0.0]),
            ),
        ]);
        let mut hkx = HkxFile::from_tagxml(
            12,
            "hk_2015.1.0-r1",
            vec![object(
                "hknpRagdollData",
                vec![
                    member("motionProperties", HkxValue::Array(vec![])),
                    member("bodyCinfos", HkxValue::Array(vec![body])),
                ],
            )],
        );
        synthesize_motion_cinfos(&mut hkx, &std::collections::HashMap::new());
        let ragdoll = &hkx.objects()[0];
        let motion_properties = ragdoll
            .members
            .iter()
            .find(|m| m.name == "motionProperties")
            .expect("motionProperties must be populated");
        let HkxValue::Array(properties) = &motion_properties.value else {
            panic!()
        };
        assert_eq!(properties.len(), 1);

        let motion_arr = ragdoll
            .members
            .iter()
            .find(|m| m.name == "motionCinfos")
            .expect("hknpRagdollData always needs motionCinfos");
        let HkxValue::Array(entries) = &motion_arr.value else {
            panic!()
        };
        assert_eq!(entries.len(), 1);
    }

    /// When the source `hknpRefMassDistribution` carries a non-identity
    /// `majorAxisSpace`, the synthesized `hknpMotionCinfo.orientation` must
    /// compose `body_orientation * majorAxisSpace` so the principal-axis
    /// inverse inertia is correctly rotated into world space.
    #[test]
    fn synthesize_motion_cinfos_composes_source_major_axis_into_orientation() {
        // 90° rotation around Z, in (x,y,z,w) form: (0, 0, sin45, cos45).
        let s45 = std::f32::consts::FRAC_1_SQRT_2;
        let major_axis_q = vec![0.0_f32, 0.0, s45, s45];

        // Build the mass distribution object first; its index will be 0.
        let mass_dist = HkxObject {
            name: Some("#massdist".to_string()),
            offset: 0,
            signature: 0,
            class_name: "hknpRefMassDistribution".to_string(),
            members: vec![member(
                "massDistribution",
                HkxValue::Object(vec![
                    member(
                        "centerOfMassAndVolume",
                        HkxValue::F32List(vec![0.0, 0.0, 0.0, 0.5]),
                    ),
                    member("inertiaTensor", HkxValue::F32List(vec![2.0, 5.0, 8.0, 0.0])),
                    member("majorAxisSpace", HkxValue::F32List(major_axis_q.clone())),
                ]),
            )],
        };
        let body = HkxValue::Object(vec![
            member("flags", HkxValue::I32(128)), // dynamic
            member("mass", HkxValue::F32(4.0)),
            member("position", HkxValue::F32List(vec![0.0, 0.0, 0.0, 0.0])),
            // Body orientation = identity → composed should equal majorAxisSpace.
            member("orientation", HkxValue::F32List(vec![0.0, 0.0, 0.0, 1.0])),
            member("massDistribution", HkxValue::Pointer(Some(0))),
            member(
                "linearVelocity",
                HkxValue::F32List(vec![0.0, 0.0, 0.0, 0.0]),
            ),
            member(
                "angularVelocity",
                HkxValue::F32List(vec![0.0, 0.0, 0.0, 0.0]),
            ),
        ]);
        let psd = object(
            "hknpPhysicsSystemData",
            vec![member("bodyCinfos", HkxValue::Array(vec![body]))],
        );
        let root = object("hkRootLevelContainer", vec![]);
        let mut hkx = HkxFile::from_tagxml(12, "hk_2015.1.0-r1", vec![mass_dist, psd, root]);

        synthesize_motion_cinfos(&mut hkx, &std::collections::HashMap::new());

        // After synth the hknpRefMassDistribution is dropped → PSD is at index 0.
        let psd = &hkx.objects()[0];
        assert_eq!(psd.class_name, "hknpPhysicsSystemData");
        let motion_arr = psd
            .members
            .iter()
            .find(|m| m.name == "motionCinfos")
            .expect("motionCinfos was not created");
        let HkxValue::Array(entries) = &motion_arr.value else {
            panic!("motionCinfos must be Array");
        };
        assert_eq!(entries.len(), 1);
        let HkxValue::TypedObject { members, .. } = &entries[0] else {
            panic!("entry must be TypedObject");
        };
        let orient = members
            .iter()
            .find(|m| m.name == "orientation")
            .expect("orientation field missing");
        let HkxValue::F32List(v) = &orient.value else {
            panic!("orientation must be F32List");
        };
        assert_eq!(v.len(), 4);
        // identity * majorAxisSpace = majorAxisSpace (xyzw order).
        for i in 0..4 {
            assert!(
                (v[i] - major_axis_q[i]).abs() < 1e-5,
                "orientation[{}] = {}, expected {}",
                i,
                v[i],
                major_axis_q[i]
            );
        }

        let mass_factor = members
            .iter()
            .find(|m| m.name == "massFactor")
            .expect("massFactor field missing");
        assert_eq!(mass_factor.value, HkxValue::F32(8.0));

        // Inverse inertia should be (1/I_i) * inv_mass for each principal axis.
        let inv_mass = 0.25_f32;
        let inv_inertia = members
            .iter()
            .find(|m| m.name == "inverseInertiaLocal")
            .expect("inverseInertiaLocal field missing");
        let HkxValue::F32List(ii) = &inv_inertia.value else {
            panic!("inverseInertiaLocal must be F32List");
        };
        assert_eq!(ii.len(), 4);
        let expected = [
            (1.0_f32 / 2.0) * inv_mass,
            (1.0_f32 / 5.0) * inv_mass,
            (1.0_f32 / 8.0) * inv_mass,
        ];
        for i in 0..3 {
            assert!(
                (ii[i] - expected[i]).abs() < 1e-5,
                "inv_inertia[{}] = {}, expected {}",
                i,
                ii[i],
                expected[i]
            );
        }
        assert!(
            ii[3].abs() < 1e-5,
            "inverseInertiaLocal[3] = {}, expected 0",
            ii[3]
        );
    }

    #[test]
    fn synthesize_motion_cinfos_composes_ragdoll_major_axis_and_rotates_com() {
        let body_orientation = vec![-0.000038_f32, -0.001566, -0.118366, -0.992969];
        let major_axis = [0.000147_f32, -0.000018, -0.118366, 0.992970];
        let body_position = [0.5_f32, -0.25, 1.0, 0.0];
        let local_com = [0.1_f32, 0.2, -0.3];
        let mass_dist = HkxObject {
            name: Some("#massdist".to_string()),
            offset: 0,
            signature: 0,
            class_name: "hknpRefMassDistribution".to_string(),
            members: vec![member(
                "massDistribution",
                HkxValue::Object(vec![
                    member(
                        "centerOfMassAndVolume",
                        HkxValue::F32List(vec![local_com[0], local_com[1], local_com[2], 0.25]),
                    ),
                    member(
                        "inertiaTensor",
                        HkxValue::F32List(vec![0.060597, 0.046659, 0.060597, 0.0]),
                    ),
                    member("majorAxisSpace", HkxValue::F32List(major_axis.to_vec())),
                ]),
            )],
        };
        let body = HkxValue::Object(vec![
            member("flags", HkxValue::I32(128)),
            member("mass", HkxValue::F32(1.0)),
            member("position", HkxValue::F32List(body_position.to_vec())),
            member("orientation", HkxValue::F32List(body_orientation.clone())),
            member("massDistribution", HkxValue::Pointer(Some(0))),
        ]);
        let ragdoll = object(
            "hknpRagdollData",
            vec![member("bodyCinfos", HkxValue::Array(vec![body]))],
        );
        let mut hkx = HkxFile::from_tagxml(12, "hk_2015.1.0-r1", vec![mass_dist, ragdoll]);

        synthesize_motion_cinfos(&mut hkx, &std::collections::HashMap::new());

        let ragdoll = &hkx.objects()[0];
        assert_eq!(ragdoll.class_name, "hknpRagdollData");
        let motion_arr = ragdoll
            .members
            .iter()
            .find(|m| m.name == "motionCinfos")
            .expect("motionCinfos must be created");
        let HkxValue::Array(entries) = &motion_arr.value else {
            panic!()
        };
        let HkxValue::TypedObject { members, .. } = &entries[0] else {
            panic!("entry must be TypedObject");
        };
        let orient = members
            .iter()
            .find(|m| m.name == "orientation")
            .expect("orientation field missing");
        let HkxValue::F32List(orientation) = &orient.value else {
            panic!("orientation must be F32List")
        };
        let expected_orientation = quat_mul_xyzw(
            [
                body_orientation[0],
                body_orientation[1],
                body_orientation[2],
                body_orientation[3],
            ],
            major_axis,
        );
        for axis in 0..4 {
            assert!((orientation[axis] - expected_orientation[axis]).abs() < 1e-6);
        }

        let rotated_com = quat_rotate_vector_xyzw(
            [
                body_orientation[0],
                body_orientation[1],
                body_orientation[2],
                body_orientation[3],
            ],
            local_com,
        );
        let HkxValue::F32List(center_of_mass) = &members
            .iter()
            .find(|m| m.name == "centerOfMassWorld")
            .expect("centerOfMassWorld field missing")
            .value
        else {
            panic!("centerOfMassWorld must be F32List")
        };
        for axis in 0..3 {
            assert!(
                (center_of_mass[axis] - (body_position[axis] + rotated_com[axis])).abs() < 1e-6
            );
        }
        assert_eq!(
            members
                .iter()
                .find(|m| m.name == "massFactor")
                .expect("massFactor field missing")
                .value,
            HkxValue::F32(4.0)
        );
    }

    #[test]
    fn synthesize_motion_cinfos_uses_embedded_inverse_inertia_w_one() {
        let body = HkxValue::Object(vec![
            member("flags", HkxValue::I32(128)),
            member("mass", HkxValue::F32(1.0)),
            member("position", HkxValue::F32List(vec![0.0, 0.0, 0.0, 0.0])),
            member("orientation", HkxValue::F32List(vec![0.0, 0.0, 0.0, 1.0])),
        ]);
        let mut hkx = HkxFile::from_tagxml(
            12,
            "hk_2015.1.0-r1",
            vec![object(
                "hknpRagdollData",
                vec![member("bodyCinfos", HkxValue::Array(vec![body]))],
            )],
        );

        synthesize_motion_cinfos(&mut hkx, &std::collections::HashMap::new());

        let motion_arr = hkx.objects()[0]
            .members
            .iter()
            .find(|m| m.name == "motionCinfos")
            .expect("motionCinfos must be created");
        let HkxValue::Array(entries) = &motion_arr.value else {
            panic!()
        };
        let HkxValue::TypedObject { members, .. } = &entries[0] else {
            panic!("entry must be TypedObject");
        };
        let inv_inertia = members
            .iter()
            .find(|m| m.name == "inverseInertiaLocal")
            .expect("inverseInertiaLocal field missing");
        let HkxValue::F32List(ii) = &inv_inertia.value else {
            panic!("inverseInertiaLocal must be F32List");
        };
        assert_eq!(ii[3], 1.0);
    }

    #[test]
    fn synthesize_motion_cinfos_uses_ragdoll_inverse_inertia_w_one_with_root_container() {
        let body = HkxValue::Object(vec![
            member("flags", HkxValue::I32(128)),
            member("mass", HkxValue::F32(1.0)),
            member("position", HkxValue::F32List(vec![0.0, 0.0, 0.0, 0.0])),
            member("orientation", HkxValue::F32List(vec![0.0, 0.0, 0.0, 1.0])),
        ]);
        let mut hkx = HkxFile::from_tagxml(
            12,
            "hk_2015.1.0-r1",
            vec![
                object("hkRootLevelContainer", vec![]),
                object(
                    "hknpRagdollData",
                    vec![member("bodyCinfos", HkxValue::Array(vec![body]))],
                ),
            ],
        );

        synthesize_motion_cinfos(&mut hkx, &std::collections::HashMap::new());

        let ragdoll = hkx
            .objects()
            .iter()
            .find(|object| object.class_name == "hknpRagdollData")
            .expect("ragdoll must remain present");
        let motion_arr = ragdoll
            .members
            .iter()
            .find(|m| m.name == "motionCinfos")
            .expect("motionCinfos must be created");
        let HkxValue::Array(entries) = &motion_arr.value else {
            panic!()
        };
        let HkxValue::TypedObject { members, .. } = &entries[0] else {
            panic!("entry must be TypedObject");
        };
        let inv_inertia = members
            .iter()
            .find(|m| m.name == "inverseInertiaLocal")
            .expect("inverseInertiaLocal field missing");
        let HkxValue::F32List(ii) = &inv_inertia.value else {
            panic!("inverseInertiaLocal must be F32List");
        };
        assert_eq!(ii[3], 1.0);
    }

    /// Build a `HkxValue::Array([I16;4])` from an 8-byte packed-vector block
    /// (as returned by `pack_vector3` / `pack_unit_quat`).
    fn packed_bytes_to_i16_array(bytes: [u8; 8]) -> HkxValue {
        HkxValue::Array(
            (0..4)
                .map(|i| HkxValue::I16(i16::from_le_bytes([bytes[i * 2], bytes[i * 2 + 1]])))
                .collect(),
        )
    }

    /// Build a named `HkxObject` for test use (mimics a real parsed file where
    /// every object carries a `#XXXX` name that survives index remaps).
    fn named_object(name: &str, class_name: &str, members: Vec<HkxMember>) -> HkxObject {
        HkxObject {
            name: Some(name.to_string()),
            offset: 0,
            signature: 0,
            class_name: class_name.to_string(),
            members,
        }
    }

    /// Deathclaw-style parity: a ragdoll body with body.mass = -1.0 (FO76
    /// sentinel) and no massDistribution, but whose shape carries a
    /// `hknpShapeMassProperties` with mass=5.0 and non-trivial inertia/COM.
    /// After `extract_shape_mass_cache` + `synthesize_motion_cinfos`, the
    /// emitted motionCinfo must have:
    ///   - `inverseMass ≈ 1/5.0 = 0.2`
    ///   - `massFactor  == 1.0`
    ///   - `inverseInertiaLocal[3] == 1.0` (w-component, ragdoll file)
    ///   - `inverseInertiaLocal[0..3] ≈ [0.1, 0.05, 1/30]` (1/forward_inertia)
    ///   - `centerOfMassWorld ≈ position + shape_com`
    #[test]
    fn synthesize_motion_cinfos_uses_shape_mass_properties_when_body_mass_is_sentinel() {
        use crate::collision::mass_properties::{pack_unit_quat, pack_vector3};

        // Real forward inertia and COM we'll pack into the shape's mass props.
        let fwd_inertia = [10.0_f32, 20.0, 30.0];
        let shape_com = [0.1_f32, 0.2, -0.1];
        let mass_m = 5.0_f32;

        // Pack the values into hkPackedVector3 / hkPackedUnitVector byte blocks.
        let com_i16 = HkxValue::Object(vec![member(
            "values",
            packed_bytes_to_i16_array(pack_vector3(shape_com)),
        )]);
        let inertia_i16 = HkxValue::Object(vec![member(
            "values",
            packed_bytes_to_i16_array(pack_vector3(fwd_inertia)),
        )]);
        let major_i16 = packed_bytes_to_i16_array(pack_unit_quat([0.0, 0.0, 0.0, 1.0])); // identity

        // hkCompressedMassProperties inline struct.
        let compressed = HkxValue::Object(vec![
            member("centerOfMass", com_i16),
            member("inertia", inertia_i16),
            member("majorAxisSpace", major_i16),
            member("mass", HkxValue::F32(mass_m)),
            member("volume", HkxValue::F32(mass_m)),
        ]);

        // hkRefCountedPropertiesEntry: key=0xF100, object=Pointer(3).
        let entry = HkxValue::Object(vec![
            member("object", HkxValue::Pointer(Some(3))), // → hknpShapeMassProperties at idx 3
            member("key", HkxValue::U16(0xF100)),
            member("flags", HkxValue::U16(0)),
        ]);

        // Object layout (indices):
        //  0: hknpRagdollData  (physics container)
        //  1: hknpConvexPolytopeShape  (#shape01 — has "properties" → idx 2)
        //  2: hkRefCountedProperties   (#props01 — has entry key 0xF100 → idx 3)
        //  3: hknpShapeMassProperties  (#mprops01)
        let body = HkxValue::Object(vec![
            member("flags", HkxValue::I32(128)), // dynamic
            member("mass", HkxValue::F32(-1.0)), // FO76 sentinel
            member("position", HkxValue::F32List(vec![1.0, 2.0, 3.0, 0.0])),
            member("orientation", HkxValue::F32List(vec![0.0, 0.0, 0.0, 1.0])),
            member("shape", HkxValue::Pointer(Some(1))), // → #shape01
            member("linearVelocity", HkxValue::F32List(vec![0.0; 4])),
            member("angularVelocity", HkxValue::F32List(vec![0.0; 4])),
        ]);

        let mut hkx = HkxFile::from_tagxml(
            12,
            "hk_2015.1.0-r1",
            vec![
                object(
                    "hknpRagdollData",
                    vec![member("bodyCinfos", HkxValue::Array(vec![body]))],
                ),
                named_object(
                    "#shape01",
                    "hknpConvexPolytopeShape",
                    vec![member("properties", HkxValue::Pointer(Some(2)))],
                ),
                named_object(
                    "#props01",
                    "hkRefCountedProperties",
                    vec![member("entries", HkxValue::Array(vec![entry]))],
                ),
                named_object(
                    "#mprops01",
                    "hknpShapeMassProperties",
                    vec![member("compressedMassProperties", compressed)],
                ),
            ],
        );

        // Build cache before stripping (simulates pre-Transform-15 state).
        let cache = extract_shape_mass_cache(&hkx);
        assert!(
            cache.contains_key("#shape01"),
            "cache must contain an entry for #shape01"
        );

        synthesize_motion_cinfos(&mut hkx, &cache);

        // Locate the emitted motionCinfo.
        let ragdoll = &hkx.objects()[0];
        let motion_arr = ragdoll
            .members
            .iter()
            .find(|m| m.name == "motionCinfos")
            .expect("motionCinfos must be synthesized");
        let HkxValue::Array(entries) = &motion_arr.value else {
            panic!("motionCinfos must be Array");
        };
        assert_eq!(entries.len(), 1);
        let HkxValue::TypedObject { members, .. } = &entries[0] else {
            panic!("motionCinfo entry must be TypedObject");
        };

        let get_f32 = |name: &str| -> f32 {
            if let HkxValue::F32(v) = members.iter().find(|m| m.name == name).unwrap().value {
                v
            } else {
                panic!("{name} must be F32");
            }
        };
        let get_f32list = |name: &str| -> Vec<f32> {
            if let HkxValue::F32List(v) = &members.iter().find(|m| m.name == name).unwrap().value {
                v.clone()
            } else {
                panic!("{name} must be F32List");
            }
        };

        // inverseMass ≈ 1/5.0 (allow 2% pack/unpack tolerance — mass is plain f32, exact).
        let inv_mass = get_f32("inverseMass");
        assert!(
            (inv_mass - 1.0 / mass_m).abs() < 1e-5,
            "inverseMass must be ≈1/5.0, got {inv_mass}"
        );

        // massFactor must be exactly 1.0 (not the -1.0 sentinel).
        let mf = get_f32("massFactor");
        assert_eq!(mf, 1.0_f32, "massFactor from shape cache must be 1.0");

        // inverseInertiaLocal[3] == 1.0 (ragdoll file: no hkRootLevelContainer).
        let ii = get_f32list("inverseInertiaLocal");
        assert_eq!(
            ii[3], 1.0,
            "inverseInertiaLocal.w must be 1.0 for ragdoll files"
        );

        // inverseInertiaLocal[0..3] ≈ 1/forward_inertia (within 2% pack tolerance).
        for (i, &expected_fwd) in fwd_inertia.iter().enumerate() {
            let expected_inv = 1.0 / expected_fwd;
            let tol = expected_inv * 0.03; // 3% — slightly wider than pack round-trip
            assert!(
                (ii[i] - expected_inv).abs() < tol,
                "inverseInertiaLocal[{i}] ≈ {expected_inv:.4}, got {:.4}",
                ii[i]
            );
        }

        // centerOfMassWorld ≈ position + shape_com (within 2% pack tolerance on COM).
        let body_pos = [1.0_f32, 2.0, 3.0];
        let com_world = get_f32list("centerOfMassWorld");
        for (i, &bp) in body_pos.iter().enumerate() {
            let expected = bp + shape_com[i];
            let tol = shape_com[i].abs().max(0.01) * 0.05; // 5% on packed COM
            assert!(
                (com_world[i] - expected).abs() < tol,
                "centerOfMassWorld[{i}] ≈ {expected:.4}, got {:.4}",
                com_world[i]
            );
        }
    }

    // -----------------------------------------------------------------
    // EPA population
    // -----------------------------------------------------------------

    fn behavior_string_data(event_names: &[&str]) -> HkxObject {
        let items: Vec<HkxValue> = event_names.iter().map(|n| string(n)).collect();
        let mut obj = object(
            "hkbBehaviorGraphStringData",
            vec![member("eventNames", HkxValue::Array(items))],
        );
        obj.name = Some("#strings".into());
        obj
    }

    #[test]
    fn match_epa_rule_first_match_wins() {
        let signals = vec!["someStaggerStart".to_string()];
        let actions = match_epa_rule(&signals).expect("matches stagger");
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].0, EpaDir::Exit);
        assert_eq!(actions[0].1, "staggerstop");
    }

    #[test]
    fn match_epa_rule_case_insensitive() {
        let signals = vec!["weapForceEquipInstant".to_string()];
        let actions = match_epa_rule(&signals).expect("matches forceequip");
        // forceequip → enter weapondraw + enter enablebumper
        assert_eq!(actions.len(), 2);
        assert!(
            actions
                .iter()
                .any(|(d, n, _)| *d == EpaDir::Enter && *n == "weapondraw")
        );
        assert!(
            actions
                .iter()
                .any(|(d, n, _)| *d == EpaDir::Enter && *n == "enablebumper")
        );
    }

    #[test]
    fn match_epa_rule_returns_none_when_no_keyword_matches() {
        let signals = vec!["unrelatedEvent".to_string(), "moveStop".to_string()];
        assert!(match_epa_rule(&signals).is_none());
    }

    #[test]
    fn create_event_property_array_appends_top_level_object() {
        let mut hkx = HkxFile::from_tagxml(11, "hk_2014.1.0-r1", vec![]);
        let events = vec![HkxValue::TypedObject {
            class_name: "hkbEventProperty".to_string(),
            members: vec![
                member("id", HkxValue::I32(48)),
                member("payload", HkxValue::Pointer(None)),
            ],
        }];
        let idx = create_event_property_array(&mut hkx, events);
        assert_eq!(idx, 0);
        let obj = &hkx.objects()[0];
        assert_eq!(obj.class_name, "hkbStateMachineEventPropertyArray");
        assert_eq!(obj.name.as_deref(), Some("#0001"));
        let events_member = obj
            .members
            .iter()
            .find(|m| m.name == "events")
            .expect("events array");
        let HkxValue::Array(items) = &events_member.value else {
            panic!("events should be Array");
        };
        assert_eq!(items.len(), 1);
        let HkxValue::TypedObject { class_name, .. } = &items[0] else {
            panic!("event should be TypedObject hkbEventProperty");
        };
        assert_eq!(class_name, "hkbEventProperty");
    }

    #[test]
    fn find_or_create_string_payload_caches_and_appends() {
        let mut hkx = HkxFile::from_tagxml(11, "hk_2014.1.0-r1", vec![]);
        let mut cache = std::collections::HashMap::new();

        let idx1 = find_or_create_string_payload(&mut hkx, &mut cache, "Enter");
        let idx2 = find_or_create_string_payload(&mut hkx, &mut cache, "Enter");
        assert_eq!(idx1, idx2, "same payload should be reused");
        assert_eq!(hkx.objects().len(), 1);

        let idx3 = find_or_create_string_payload(&mut hkx, &mut cache, "Exit");
        assert_ne!(idx1, idx3);
        assert_eq!(hkx.objects().len(), 2);
        assert_eq!(hkx.objects()[1].class_name, "hkbStringEventPayload");
        let data_member = hkx.objects()[1]
            .members
            .iter()
            .find(|m| m.name == "data")
            .unwrap();
        if let HkxValue::String { value, .. } = &data_member.value {
            assert_eq!(value, "Exit");
        } else {
            panic!("data should be String");
        }
    }

    #[test]
    fn populate_event_property_arrays_creates_epa_for_stagger_state() {
        // Layout:
        //   #0 hkbBehaviorGraphStringData (eventNames: [moveStart, staggerStart, staggerStop])
        //   #1 hkbStateMachineStateInfo (stateId=10, no enterNotifyEvents/exitNotifyEvents)
        //   #2 hkbStateMachineTransitionInfoArray (transitions: [{toStateId=10, eventId=1}])
        //   #3 hkbStateMachine (states: [&#1], wildcardTransitions: &#2)
        //
        // Signal for state #1 is "staggerStart" → matches keyword "stagger"
        //   → exit staggerstop (id=2). Should append a new EPA.
        let mut state_info = object(
            "hkbStateMachineStateInfo",
            vec![
                member("stateId", HkxValue::I32(10)),
                member("name", string("Stagger")),
                member("enterNotifyEvents", HkxValue::Pointer(None)),
                member("exitNotifyEvents", HkxValue::Pointer(None)),
                member("generator", HkxValue::Pointer(None)),
            ],
        );
        state_info.name = Some("#state".into());

        let mut trans_array = object(
            "hkbStateMachineTransitionInfoArray",
            vec![member(
                "transitions",
                HkxValue::Array(vec![HkxValue::Object(vec![
                    member("toStateId", HkxValue::I32(10)),
                    member("eventId", HkxValue::I32(1)),
                ])]),
            )],
        );
        trans_array.name = Some("#trans".into());

        let mut state_machine = object(
            "hkbStateMachine",
            vec![
                member("states", HkxValue::Array(vec![HkxValue::Pointer(Some(1))])),
                member("wildcardTransitions", HkxValue::Pointer(Some(2))),
            ],
        );
        state_machine.name = Some("#sm".into());

        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2014.1.0-r1",
            vec![
                behavior_string_data(&["moveStart", "staggerStart", "staggerStop"]),
                state_info,
                trans_array,
                state_machine,
            ],
        );

        let pre_count = hkx.objects().len();
        populate_event_property_arrays(&mut hkx);
        assert_eq!(
            hkx.objects().len(),
            pre_count + 1,
            "one EPA should have been appended"
        );

        let new_epa = hkx.objects().last().unwrap();
        assert_eq!(new_epa.class_name, "hkbStateMachineEventPropertyArray");
        let events_member = new_epa.members.iter().find(|m| m.name == "events").unwrap();
        let HkxValue::Array(items) = &events_member.value else {
            panic!()
        };
        assert_eq!(items.len(), 1);
        let HkxValue::TypedObject { members, .. } = &items[0] else {
            panic!()
        };
        let id = members
            .iter()
            .find(|m| m.name == "id")
            .map(|m| direct_member_as_i32(&m.value))
            .unwrap();
        assert_eq!(id, Some(2)); // staggerStop is id 2

        // The state's exitNotifyEvents pointer should now point at the new EPA.
        let new_epa_idx = hkx.objects().len() - 1;
        let state = &hkx.objects()[1];
        let exit_ptr = state
            .members
            .iter()
            .find(|m| m.name == "exitNotifyEvents")
            .unwrap();
        assert_eq!(exit_ptr.value, HkxValue::Pointer(Some(new_epa_idx)));
        // enterNotifyEvents is unchanged.
        let enter_ptr = state
            .members
            .iter()
            .find(|m| m.name == "enterNotifyEvents")
            .unwrap();
        assert_eq!(enter_ptr.value, HkxValue::Pointer(None));
    }

    #[test]
    fn populate_event_property_arrays_keeps_child_events_off_state_machine_wrapper() {
        let existing_exit_epa = object(
            "hkbStateMachineEventPropertyArray",
            vec![member(
                "events",
                HkxValue::Array(vec![HkxValue::TypedObject {
                    class_name: "hkbEventProperty".to_string(),
                    members: vec![
                        member("id", HkxValue::I32(3)),
                        member("payload", HkxValue::Pointer(None)),
                    ],
                }]),
            )],
        );
        let inner_state = object(
            "hkbStateMachineStateInfo",
            vec![
                member("stateId", HkxValue::I32(5)),
                member("enterNotifyEvents", HkxValue::Pointer(None)),
                member("exitNotifyEvents", HkxValue::Pointer(None)),
                member("transitions", HkxValue::Pointer(None)),
                member("generator", HkxValue::Pointer(None)),
            ],
        );
        let inner_transitions = object(
            "hkbStateMachineTransitionInfoArray",
            vec![member(
                "transitions",
                HkxValue::Array(vec![HkxValue::Object(vec![
                    member("toStateId", HkxValue::I32(5)),
                    member("eventId", HkxValue::I32(0)),
                ])]),
            )],
        );
        let inner_state_machine = object(
            "hkbStateMachine",
            vec![
                member("states", HkxValue::Array(vec![HkxValue::Pointer(Some(2))])),
                member("wildcardTransitions", HkxValue::Pointer(Some(3))),
            ],
        );
        let outer_state = object(
            "hkbStateMachineStateInfo",
            vec![
                member("stateId", HkxValue::I32(8)),
                member("enterNotifyEvents", HkxValue::Pointer(None)),
                member("exitNotifyEvents", HkxValue::Pointer(Some(1))),
                member("transitions", HkxValue::Pointer(None)),
                member("generator", HkxValue::Pointer(Some(4))),
            ],
        );
        let outer_state_machine = object(
            "hkbStateMachine",
            vec![
                member("states", HkxValue::Array(vec![HkxValue::Pointer(Some(5))])),
                member("wildcardTransitions", HkxValue::Pointer(None)),
            ],
        );
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2014.1.0-r1",
            vec![
                behavior_string_data(&[
                    "Ragdoll",
                    "RemoveCharacterControllerFromWorld",
                    "EnterFullyRagdoll",
                    "GetUpEnd",
                ]),
                existing_exit_epa,
                inner_state,
                inner_transitions,
                inner_state_machine,
                outer_state,
                outer_state_machine,
            ],
        );

        let pre_count = hkx.objects().len();
        populate_event_property_arrays(&mut hkx);

        assert_eq!(hkx.objects().len(), pre_count + 1);
        let outer_enter = hkx.objects()[5]
            .members
            .iter()
            .find(|member| member.name == "enterNotifyEvents")
            .unwrap();
        assert_eq!(outer_enter.value, HkxValue::Pointer(None));
        let inner_enter = hkx.objects()[2]
            .members
            .iter()
            .find(|member| member.name == "enterNotifyEvents")
            .unwrap();
        assert_eq!(inner_enter.value, HkxValue::Pointer(Some(pre_count)));
    }

    #[test]
    fn populate_event_property_arrays_requires_source_epa_for_getup_exit() {
        for (name, exit_epa) in [("DeathBackward", None), ("RagdollAndGetUp", Some(3))] {
            let state = object(
                "hkbStateMachineStateInfo",
                vec![
                    member("name", string(name)),
                    member("enterNotifyEvents", HkxValue::Pointer(None)),
                    member("exitNotifyEvents", HkxValue::Pointer(exit_epa)),
                    member("generator", HkxValue::Pointer(Some(1))),
                ],
            );
            let mut hkx = HkxFile::from_tagxml(
                11,
                "hk_2014.1.0-r1",
                vec![
                    behavior_string_data(&["GetUpEnd"]),
                    object("hkbStateMachine", vec![]),
                    state.clone(),
                    object(
                        "hkbStateMachineEventPropertyArray",
                        vec![member("events", HkxValue::Array(vec![]))],
                    ),
                ],
            );

            populate_event_property_arrays(&mut hkx);

            assert_eq!(hkx.objects().len(), 4, "{name}");
            assert_eq!(hkx.objects()[2], state, "{name}");
            let HkxValue::Array(events) = &hkx.objects()[3].members[0].value else {
                panic!("expected exit event array");
            };
            if exit_epa.is_some() {
                assert_eq!(events.len(), 1);
                let HkxValue::TypedObject { members, .. } = &events[0] else {
                    panic!("expected event property");
                };
                assert_eq!(
                    members
                        .iter()
                        .find(|member| member.name == "id")
                        .unwrap()
                        .value,
                    HkxValue::I32(0)
                );
            } else {
                assert!(events.is_empty());
            }
        }
    }

    #[test]
    fn populate_event_property_arrays_no_op_for_non_behavior_file() {
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2014.1.0-r1",
            vec![object("hkRootLevelContainer", vec![])],
        );
        let pre_count = hkx.objects().len();
        populate_event_property_arrays(&mut hkx);
        assert_eq!(hkx.objects().len(), pre_count, "should not add objects");
    }

    /// Rule 2 unit test: a ReferencePoseGenerator state with both enter+exit
    /// EPAs already present (but empty) gets clip-annotation events partitioned
    /// into enter (foot* prefix) and exit (everything else).
    ///
    /// Event table:
    ///   0: "transitionEvent"   — used in transition → excluded
    ///   1: "footLeft"         — enter prefix → goes to enterNotifyEvents
    ///   2: "someFire"         — no enter prefix, not excluded → exitNotifyEvents
    ///   3: "attackStop"       — in OTHER_EPA_EVENTS → excluded
    ///   4: "hitReactLight"    — starts with "hitreact" (EXCLUDE_PREFIXES) → excluded
    #[test]
    fn populate_event_holding_state_partitions_clip_annotation_events() {
        // Object layout:
        //   0: hkbBehaviorGraphStringData
        //   1: enter EPA (hkbStateMachineEventPropertyArray, empty)
        //   2: exit  EPA (hkbStateMachineEventPropertyArray, empty)
        //   3: hkbReferencePoseGenerator
        //   4: hkbStateMachineStateInfo  (state under test)
        //   5: hkbStateMachineTransitionInfoArray  (uses event 0)
        //   6: hkbStateMachine

        let mut enter_epa = object(
            "hkbStateMachineEventPropertyArray",
            vec![
                member("memSizeAndFlags", HkxValue::U16(0)),
                member("refCount", HkxValue::I16(0)),
                member("events", HkxValue::Array(vec![])),
            ],
        );
        enter_epa.name = Some("#enter_epa".into());

        let mut exit_epa = object(
            "hkbStateMachineEventPropertyArray",
            vec![
                member("memSizeAndFlags", HkxValue::U16(0)),
                member("refCount", HkxValue::I16(0)),
                member("events", HkxValue::Array(vec![])),
            ],
        );
        exit_epa.name = Some("#exit_epa".into());

        let mut ref_pose_gen = object("hkbReferencePoseGenerator", vec![]);
        ref_pose_gen.name = Some("#gen".into());

        let mut state = object(
            "hkbStateMachineStateInfo",
            vec![
                member("stateId", HkxValue::I32(1)),
                member("name", string("IdleHold")),
                member("enterNotifyEvents", HkxValue::Pointer(Some(1))), // → enter_epa
                member("exitNotifyEvents", HkxValue::Pointer(Some(2))),  // → exit_epa
                member("generator", HkxValue::Pointer(Some(3))),         // → ref_pose_gen
            ],
        );
        state.name = Some("#state".into());

        let mut trans_array = object(
            "hkbStateMachineTransitionInfoArray",
            vec![member(
                "transitions",
                HkxValue::Array(vec![HkxValue::Object(vec![
                    member("toStateId", HkxValue::I32(1)),
                    member("eventId", HkxValue::I32(0)), // transitionEvent used
                ])]),
            )],
        );
        trans_array.name = Some("#trans".into());

        let mut sm = object(
            "hkbStateMachine",
            vec![
                member("states", HkxValue::Array(vec![HkxValue::Pointer(Some(4))])),
                member("wildcardTransitions", HkxValue::Pointer(Some(5))),
            ],
        );
        sm.name = Some("#sm".into());

        let hkx_objects = vec![
            behavior_string_data(&[
                "transitionEvent", // id 0 — used in transition
                "footLeft",        // id 1 — enter prefix
                "someFire",        // id 2 — exit (no enter prefix)
                "attackStop",      // id 3 — in OTHER_EPA_EVENTS
                "hitReactLight",   // id 4 — EXCLUDE_PREFIXES
            ]),
            enter_epa,    // idx 1
            exit_epa,     // idx 2
            ref_pose_gen, // idx 3
            state,        // idx 4
            trans_array,  // idx 5
            sm,           // idx 6
        ];

        let mut hkx = HkxFile::from_tagxml(11, "hk_2014.1.0-r1", hkx_objects);

        let pre_count = hkx.objects().len();
        populate_event_property_arrays(&mut hkx);
        // Rule 2 populates existing EPAs — no new objects should be created.
        assert_eq!(
            hkx.objects().len(),
            pre_count,
            "Rule 2 should not create new EPA objects"
        );

        // enter EPA (idx 1) should contain footLeft (event id 1).
        let enter_events_member = hkx.objects()[1]
            .members
            .iter()
            .find(|m| m.name == "events")
            .unwrap();
        let HkxValue::Array(enter_items) = &enter_events_member.value else {
            panic!("not array")
        };
        assert_eq!(
            enter_items.len(),
            1,
            "enter EPA should have exactly 1 event (footLeft)"
        );
        let HkxValue::TypedObject {
            members: ref em, ..
        } = enter_items[0]
        else {
            panic!()
        };
        let enter_id = em
            .iter()
            .find(|m| m.name == "id")
            .map(|m| direct_member_as_i32(&m.value))
            .unwrap();
        assert_eq!(enter_id, Some(1), "enter event id should be 1 (footLeft)");

        // exit EPA (idx 2) should contain someFire (event id 2).
        let exit_events_member = hkx.objects()[2]
            .members
            .iter()
            .find(|m| m.name == "events")
            .unwrap();
        let HkxValue::Array(exit_items) = &exit_events_member.value else {
            panic!("not array")
        };
        assert_eq!(
            exit_items.len(),
            1,
            "exit EPA should have exactly 1 event (someFire)"
        );
        let HkxValue::TypedObject {
            members: ref xm, ..
        } = exit_items[0]
        else {
            panic!()
        };
        let exit_id = xm
            .iter()
            .find(|m| m.name == "id")
            .map(|m| direct_member_as_i32(&m.value))
            .unwrap();
        assert_eq!(exit_id, Some(2), "exit event id should be 2 (someFire)");
    }

    // ---------------------------------------------------------------------------
    // auto_fix_human_bone_tracks tests
    // ---------------------------------------------------------------------------

    /// Build a minimal 96-track interleaved animation + binding pair.
    /// Returns (HkxFile, anim_obj_idx, binding_obj_idx).
    fn make_96track_interleaved_hkx() -> HkxFile {
        // 96 identity QsTransform values per frame, 2 frames = 192 entries.
        let identity_qs: HkxValue = HkxValue::F32List(vec![
            0.0, 0.0, 0.0, 0.0, // translation (xyz, w=0)
            0.0, 0.0, 0.0, 1.0, // rotation (x y z w)
            1.0, 1.0, 1.0, 0.0, // scale (xyz, w=0)
        ]);
        let num_frames = 2usize;
        let num_tracks = 96usize;
        let transforms: Vec<HkxValue> = std::iter::repeat(identity_qs.clone())
            .take(num_frames * num_tracks)
            .collect();
        let annotation_tracks: Vec<HkxValue> =
            (0..num_tracks).map(|_| HkxValue::Object(vec![])).collect();

        let mut anim_obj = object(
            "hkaInterleavedUncompressedAnimation",
            vec![
                member("duration", HkxValue::F32(1.0 / 30.0)),
                member("numberOfTransformTracks", HkxValue::I32(96)),
                member("numberOfFloatTracks", HkxValue::I32(0)),
                member("transforms", HkxValue::Array(transforms)),
                member("annotationTracks", HkxValue::Array(annotation_tracks)),
            ],
        );
        anim_obj.name = Some("#anim".into());

        // Binding with non-identity FO76 bone indices (identity[0..96] matches FO76 order).
        let fo76_identity: Vec<HkxValue> = (0_i32..96).map(HkxValue::I32).collect();
        let mut binding_obj = object(
            "hkaAnimationBinding",
            vec![
                member("animation", HkxValue::Pointer(Some(0))),
                member(
                    "transformTrackToBoneIndices",
                    HkxValue::Array(fo76_identity),
                ),
            ],
        );
        binding_obj.name = Some("#binding".into());

        HkxFile::from_tagxml(11, "hk_2015.1.0-r1", vec![anim_obj, binding_obj])
    }

    fn make_spline_hkx(num_tracks: usize) -> HkxFile {
        use crate::animation::spline::{SplineFrame, SplineTransform, compress_spline};

        let identity = SplineTransform {
            translation: [0.0, 0.0, 0.0],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0, 1.0, 1.0],
        };
        let frames = vec![
            SplineFrame {
                transforms: vec![identity.clone(); num_tracks],
            },
            SplineFrame {
                transforms: vec![identity; num_tracks],
            },
        ];
        let duration = 1.0 / 30.0;
        let blob = compress_spline(&frames, duration, 30.0).expect("compress test spline");
        let annotation_tracks: Vec<HkxValue> =
            (0..num_tracks).map(|_| HkxValue::Object(vec![])).collect();

        let mut anim_obj = object(
            "hkaSplineCompressedAnimation",
            vec![
                member("duration", HkxValue::F32(duration)),
                member("numberOfTransformTracks", HkxValue::I32(num_tracks as i32)),
                member("numberOfFloatTracks", HkxValue::I32(0)),
                member("annotationTracks", HkxValue::Array(annotation_tracks)),
                member("numFrames", HkxValue::I32(blob.num_frames as i32)),
                member("numBlocks", HkxValue::I32(blob.num_blocks as i32)),
                member(
                    "maxFramesPerBlock",
                    HkxValue::I32(blob.max_frames_per_block as i32),
                ),
                member(
                    "blockOffsets",
                    HkxValue::Array(
                        blob.block_offsets
                            .iter()
                            .map(|&o| HkxValue::U32(o))
                            .collect(),
                    ),
                ),
                member(
                    "floatBlockOffsets",
                    HkxValue::Array(
                        blob.float_block_offsets
                            .iter()
                            .map(|&o| HkxValue::U32(o))
                            .collect(),
                    ),
                ),
                member(
                    "data",
                    HkxValue::Array(blob.data.iter().map(|&b| HkxValue::U8(b)).collect()),
                ),
            ],
        );
        anim_obj.name = Some("#spline".into());

        let fo76_identity: Vec<HkxValue> = (0..num_tracks)
            .map(|track| HkxValue::I32(track as i32))
            .collect();
        let mut binding_obj = object(
            "hkaAnimationBinding",
            vec![
                member("animation", HkxValue::Pointer(Some(0))),
                member(
                    "transformTrackToBoneIndices",
                    HkxValue::Array(fo76_identity),
                ),
            ],
        );
        binding_obj.name = Some("#binding".into());

        HkxFile::from_tagxml(11, "hk_2015.1.0-r1", vec![anim_obj, binding_obj])
    }

    fn make_96track_spline_hkx() -> HkxFile {
        make_spline_hkx(96)
    }

    #[test]
    fn auto_fix_human_bone_tracks_96track_interleaved_strips_and_reorders() {
        let mut hkx = make_96track_interleaved_hkx();

        auto_fix_human_bone_tracks(&mut hkx);

        let anim = &hkx.objects()[0];
        // (a) numberOfTransformTracks should be 95.
        let num_tracks = anim
            .members
            .iter()
            .find(|m| m.name == "numberOfTransformTracks")
            .and_then(|m| direct_member_as_i32(&m.value));
        assert_eq!(
            num_tracks,
            Some(95),
            "numberOfTransformTracks should be 95 after strip"
        );

        // (b) transforms array length = 2 frames * 95 tracks = 190.
        let transforms_len = anim
            .members
            .iter()
            .find(|m| m.name == "transforms")
            .and_then(|m| {
                if let HkxValue::Array(a) = &m.value {
                    Some(a.len())
                } else {
                    None
                }
            });
        assert_eq!(
            transforms_len,
            Some(190),
            "transforms should have 190 entries (2 frames * 95)"
        );

        // (c) binding transformTrackToBoneIndices should be identity [0..95].
        let binding = &hkx.objects()[1];
        let indices: Vec<i32> = binding
            .members
            .iter()
            .find(|m| m.name == "transformTrackToBoneIndices")
            .and_then(|m| {
                if let HkxValue::Array(a) = &m.value {
                    Some(a.iter().filter_map(|v| direct_member_as_i32(v)).collect())
                } else {
                    None
                }
            })
            .unwrap_or_default();
        let expected_identity: Vec<i32> = (0..95).collect();
        assert_eq!(
            indices, expected_identity,
            "binding indices should be identity [0..95]"
        );
    }

    #[test]
    fn auto_fix_human_bone_tracks_90track_interleaved_non_identity_remaps_indices() {
        // 90-track interleaved with non-identity FO76 bone indices — should remap to FO4.
        let identity_qs = HkxValue::F32List(vec![
            0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0,
        ]);
        let num_tracks = 90usize;
        let transforms: Vec<HkxValue> = std::iter::repeat(identity_qs).take(num_tracks).collect();

        let mut anim_obj = object(
            "hkaInterleavedUncompressedAnimation",
            vec![
                member("duration", HkxValue::F32(1.0 / 30.0)),
                member("numberOfTransformTracks", HkxValue::I32(90)),
                member("numberOfFloatTracks", HkxValue::I32(0)),
                member("transforms", HkxValue::Array(transforms)),
                member("annotationTracks", HkxValue::Array(Vec::new())),
            ],
        );
        anim_obj.name = Some("#anim90".into());

        // Non-identity: FO76 bone indices 0..90.
        let fo76_indices: Vec<HkxValue> = (0_i32..90).map(HkxValue::I32).collect();
        let mut binding_obj = object(
            "hkaAnimationBinding",
            vec![
                member("animation", HkxValue::Pointer(Some(0))),
                member("transformTrackToBoneIndices", HkxValue::Array(fo76_indices)),
            ],
        );
        binding_obj.name = Some("#binding90".into());

        let mut hkx = HkxFile::from_tagxml(11, "hk_2015.1.0-r1", vec![anim_obj, binding_obj]);

        auto_fix_human_bone_tracks(&mut hkx);

        // After remap, binding indices should be identity [0..90] (FO4 order).
        let binding = &hkx.objects()[1];
        let indices: Vec<i32> = binding
            .members
            .iter()
            .find(|m| m.name == "transformTrackToBoneIndices")
            .and_then(|m| {
                if let HkxValue::Array(a) = &m.value {
                    Some(a.iter().filter_map(|v| direct_member_as_i32(v)).collect())
                } else {
                    None
                }
            })
            .unwrap_or_default();
        let expected_identity: Vec<i32> = (0..90).collect();
        assert_eq!(
            indices, expected_identity,
            "90-track binding indices should be identity after remap"
        );
    }

    #[test]
    fn auto_fix_human_bone_tracks_no_animation_objects_is_noop() {
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2015.1.0-r1",
            vec![object("hkbStateMachine", vec![])],
        );
        // Should not panic; no animation classes so noop.
        auto_fix_human_bone_tracks(&mut hkx);
        assert_eq!(hkx.objects().len(), 1);
    }

    #[test]
    fn auto_fix_human_bone_tracks_96track_interleaved_annotation_tracks_stripped() {
        let mut hkx = make_96track_interleaved_hkx();

        auto_fix_human_bone_tracks(&mut hkx);

        let anim = &hkx.objects()[0];
        let annotation_len = anim
            .members
            .iter()
            .find(|m| m.name == "annotationTracks")
            .and_then(|m| {
                if let HkxValue::Array(a) = &m.value {
                    Some(a.len())
                } else {
                    None
                }
            });
        assert_eq!(
            annotation_len,
            Some(95),
            "annotationTracks should have 95 entries after stripping AimSource"
        );
    }

    #[test]
    fn auto_fix_human_bone_tracks_96track_spline_annotation_tracks_stripped() {
        let mut hkx = make_96track_spline_hkx();

        auto_fix_human_bone_tracks(&mut hkx);

        let anim = &hkx.objects()[0];
        let num_tracks = anim
            .members
            .iter()
            .find(|m| m.name == "numberOfTransformTracks")
            .and_then(|m| direct_member_as_i32(&m.value));
        assert_eq!(num_tracks, Some(95));

        let annotation_len = anim
            .members
            .iter()
            .find(|m| m.name == "annotationTracks")
            .and_then(|m| {
                if let HkxValue::Array(a) = &m.value {
                    Some(a.len())
                } else {
                    None
                }
            });
        assert_eq!(
            annotation_len,
            Some(95),
            "spline annotationTracks should follow the stripped transform tracks"
        );
    }

    #[test]
    fn auto_fix_human_bone_tracks_spline_keeps_serializable_animation_type() {
        let mut hkx = make_96track_spline_hkx();

        auto_fix_human_bone_tracks(&mut hkx);

        let animation_type = hkx.objects()[0]
            .members
            .iter()
            .find(|member| member.name == "type")
            .map(|member| &member.value);
        assert_eq!(
            animation_type,
            Some(&HkxValue::I32(HK_SPLINE_COMPRESSED_ANIMATION_TYPE)),
            "hkaAnimation.AnimationType must remain the numeric spline enum"
        );
    }

    #[test]
    fn opt_in_spline_rewrite_preserves_94_track_binding_and_annotations() {
        let input = make_spline_hkx(94);

        let default = migrate_2015_packfile_to_2014_with_warnings(
            input.clone(),
            Fo76MigrationOptions::default(),
        )
        .expect("default migration");
        assert_eq!(
            default.hkx.objects()[0].class_name,
            "hkaSplineCompressedAnimation"
        );

        let rewritten = migrate_2015_packfile_to_2014_with_warnings(
            input,
            Fo76MigrationOptions {
                decompress_spline: true,
                recompress: true,
                ..Default::default()
            },
        )
        .expect("migration with spline rewrite");

        let animation = &rewritten.hkx.objects()[0];
        assert_eq!(animation.class_name, "hkaSplineCompressedAnimation");
        assert_eq!(
            animation
                .members
                .iter()
                .find(|member| member.name == "annotationTracks")
                .and_then(|member| match &member.value {
                    HkxValue::Array(values) => Some(values.len()),
                    _ => None,
                }),
            Some(94)
        );
        assert_eq!(
            animation
                .members
                .iter()
                .find(|member| member.name == "floatBlockOffsets")
                .and_then(|member| match &member.value {
                    HkxValue::Array(values) => Some(values.len()),
                    _ => None,
                }),
            Some(1)
        );

        let binding = &rewritten.hkx.objects()[1];
        assert_eq!(
            binding
                .members
                .iter()
                .find(|member| member.name == "animation")
                .map(|member| &member.value),
            Some(&HkxValue::Pointer(Some(0)))
        );
        assert_eq!(
            binding
                .members
                .iter()
                .find(|member| member.name == "transformTrackToBoneIndices")
                .and_then(|member| match &member.value {
                    HkxValue::Array(values) => Some(values.len()),
                    _ => None,
                }),
            Some(94)
        );
    }

    #[test]
    fn flatten_nested_class_names_flattens_any_double_colon() {
        // Any class name with :: is flattened (not just the original 8 hard-coded names).
        let mut hkx = HkxFile::from_tagxml(
            12,
            "hk_2015.1.0-r1",
            vec![object("hkbStateMachine::EventInfo", vec![])],
        );
        flatten_nested_class_names(&mut hkx);
        assert_eq!(hkx.objects()[0].class_name, "hkbStateMachineEventInfo");
    }

    #[test]
    fn flatten_nested_class_names_covers_all_known_renames() {
        for (old, expected) in KNOWN_NESTED_CLASS_RENAMES {
            let mut hkx = HkxFile::from_tagxml(12, "hk_2015.1.0-r1", vec![object(old, vec![])]);
            flatten_nested_class_names(&mut hkx);
            assert_eq!(
                hkx.objects()[0].class_name,
                *expected,
                "expected {old} -> {expected}"
            );
        }
    }

    #[test]
    fn auto_fix_human_bone_tracks_pipeline_advances_past_transform_6() {
        // Verify apply_implemented_transforms advances past index 6 for anim-free files.
        let mut hkx = HkxFile::from_tagxml(
            11,
            "hk_2015.1.0-r1",
            vec![object(
                "hkRootLevelContainer",
                vec![member("namedVariants", HkxValue::Array(vec![]))],
            )],
        );
        let last = apply_for_test(&mut hkx);
        assert!(
            last > 6,
            "pipeline should advance past transform 6, got {last}"
        );
    }

    #[test]
    fn convert_recompress_preserves_float_tracks() {
        // 4 frames, 1 transform track (identity), 1 float track with rising values.
        // After recompress_animations the spline blob must carry the float-track
        // payload, recoverable by decompress_spline_full within tolerance.
        let original_floats = [0.5_f32, 1.0, 1.5, 2.0];
        let num_frames = original_floats.len();
        let duration = (num_frames - 1) as f32 / 30.0;

        // Build interleaved transforms: identity QsTransform per frame.
        let mut transforms_arr: Vec<HkxValue> = Vec::with_capacity(num_frames);
        for _ in 0..num_frames {
            transforms_arr.push(HkxValue::F32List(vec![
                0.0, 0.0, 0.0, 0.0, // translation + pad
                0.0, 0.0, 0.0, 1.0, // rotation (identity)
                1.0, 1.0, 1.0, 0.0, // scale + pad
            ]));
        }

        // Float-track payload (interleaved per-frame).
        let floats_arr: Vec<HkxValue> = original_floats.iter().map(|v| HkxValue::F32(*v)).collect();

        let interleaved = object(
            "hkaInterleavedUncompressedAnimation",
            vec![
                member(
                    "type",
                    HkxValue::String {
                        value: "HK_INTERLEAVED_ANIMATION".to_string(),
                        is_null: false,
                    },
                ),
                member("duration", HkxValue::F32(duration)),
                member("numberOfTransformTracks", HkxValue::I32(1)),
                member("numberOfFloatTracks", HkxValue::I32(1)),
                member("extractedMotion", HkxValue::Pointer(None)),
                member("annotationTracks", HkxValue::Array(Vec::new())),
                member("transforms", HkxValue::Array(transforms_arr)),
                member("floats", HkxValue::Array(floats_arr)),
            ],
        );

        let mut hkx = HkxFile::from_tagxml(11, "hk_2014.2.0-r1", vec![interleaved]);

        recompress_animations(&mut hkx).expect("recompress");

        // The replacement should now be hkaSplineCompressedAnimation.
        let spline = hkx
            .objects()
            .iter()
            .find(|o| o.class_name == "hkaSplineCompressedAnimation")
            .expect("spline animation present after recompress");

        let get_i32 = |name: &str| -> i32 {
            spline
                .members
                .iter()
                .find(|m| m.name == name)
                .and_then(|m| direct_member_as_i32(&m.value))
                .unwrap_or_else(|| panic!("missing {name}"))
        };
        let get_f32 = |name: &str| -> f32 {
            spline
                .members
                .iter()
                .find(|m| m.name == name)
                .and_then(|m| {
                    if let HkxValue::F32(v) = m.value {
                        Some(v)
                    } else {
                        None
                    }
                })
                .unwrap_or_else(|| panic!("missing {name}"))
        };

        let num_tracks = get_i32("numberOfTransformTracks") as u32;
        let num_floats = get_i32("numberOfFloatTracks") as u32;
        assert_eq!(num_floats, 1, "float track count preserved on recompress");

        let nf = get_i32("numFrames") as u32;
        let nb = get_i32("numBlocks") as u32;
        let max_frames = get_i32("maxFramesPerBlock") as u32;
        let mqs = get_i32("maskAndQuantizationSize") as u32;
        let bd = get_f32("blockDuration");
        let bid = get_f32("blockInverseDuration");
        let fd = get_f32("frameDuration");

        let block_offsets: Vec<u32> = match &spline
            .members
            .iter()
            .find(|m| m.name == "blockOffsets")
            .unwrap()
            .value
        {
            HkxValue::Array(a) => a
                .iter()
                .map(|v| direct_member_as_i32(v).unwrap() as u32)
                .collect(),
            _ => panic!("blockOffsets array"),
        };
        let float_block_offsets: Vec<u32> = match &spline
            .members
            .iter()
            .find(|m| m.name == "floatBlockOffsets")
            .unwrap()
            .value
        {
            HkxValue::Array(a) => a
                .iter()
                .map(|v| direct_member_as_i32(v).unwrap() as u32)
                .collect(),
            _ => panic!("floatBlockOffsets array"),
        };
        let data_bytes: Vec<u8> = match &spline
            .members
            .iter()
            .find(|m| m.name == "data")
            .unwrap()
            .value
        {
            HkxValue::Array(a) => a
                .iter()
                .filter_map(|v| {
                    if let HkxValue::U8(b) = v {
                        Some(*b)
                    } else {
                        direct_member_as_i32(v).map(|i| i as u8)
                    }
                })
                .collect(),
            _ => panic!("data array"),
        };

        assert!(
            !float_block_offsets.is_empty(),
            "floatBlockOffsets must be populated when there are float tracks"
        );

        let parsed = crate::animation::spline::decompress_spline_full(
            &data_bytes,
            num_tracks,
            num_floats,
            nf,
            max_frames,
            nb,
            &block_offsets,
            &float_block_offsets,
            mqs,
            bd,
            bid,
            fd,
        )
        .expect("decompress_spline_full");

        assert_eq!(parsed.float_tracks.len(), 1, "one recovered float track");
        let recovered = &parsed.float_tracks[0];
        assert_eq!(
            recovered.len(),
            num_frames,
            "recovered float track has {num_frames} samples"
        );
        for (i, (a, b)) in recovered.iter().zip(original_floats.iter()).enumerate() {
            assert!(
                (a - b).abs() < 0.01,
                "float sample {i} drift: got {a}, expected {b}"
            );
        }
    }
}
