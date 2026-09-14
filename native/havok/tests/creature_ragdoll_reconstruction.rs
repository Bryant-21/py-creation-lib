use havok_native::convert::creature_ragdoll::{
    BoneMappingIr, ConstraintIr, ConstraintKindIr, ConvexHullIr, CreatureRagdollError,
    CreatureRagdollIr, LimitedHingeIr, MassPropertiesIr, QsTransformIr, RagdollLimitsIr,
    RagdollShapeIr, RigBoneIr, RigSkeletonIr, RigidBodyIr, extract_skyrim_2010_creature_ragdoll,
    lower_primitive_shapes_to_fo4_convex_hulls, reconstruct_fo4_creature_ragdoll_packfile,
    reconstruct_fo4_embedded_creature_ragdoll,
};
use havok_native::hkx::HkxFile;
use havok_native::hkx::model::HkxObject;
use havok_native::hkx::types::HkxValue;

const SKYRIM_RAGDOLL_CORPUS: &[&str] = &[
    "extracted/skyrimse/meshes/actors/ambient/chicken/character assets/skeleton.hkx",
    "extracted/skyrimse/meshes/actors/ambient/hare/character assets/skeleton.hkx",
    "extracted/skyrimse/meshes/actors/atronachflame/character assets/skeleton.hkx",
    "extracted/skyrimse/meshes/actors/atronachfrost/character assets/skeleton.hkx",
    "extracted/skyrimse/meshes/actors/atronachstorm/character assets/skeleton.hkx",
    "extracted/skyrimse/meshes/actors/bear/character assets/skeleton.hkx",
    "extracted/skyrimse/meshes/actors/canine/character assets dog/skeleton.hkx",
    "extracted/skyrimse/meshes/actors/canine/character assets wolf/skeleton.hkx",
    "extracted/skyrimse/meshes/actors/character/character assets female/skeleton_female.hkx",
    "extracted/skyrimse/meshes/actors/character/character assets/skeleton.hkx",
    "extracted/skyrimse/meshes/actors/chaurus/character assets/skeleton.hkx",
    "extracted/skyrimse/meshes/actors/cow/character assets/skeleton.hkx",
    "extracted/skyrimse/meshes/actors/deer/character assets/skeleton.hkx",
    "extracted/skyrimse/meshes/actors/dlc01/chaurusflyer/character assets/skeleton.hkx",
    "extracted/skyrimse/meshes/actors/dlc01/vampirebrute/character assets/skeleton.hkx",
    "extracted/skyrimse/meshes/actors/dlc02/benthiclurker/character assets/skeleton.hkx",
    "extracted/skyrimse/meshes/actors/dlc02/boarriekling/character assets/skeleton.hkx",
    "extracted/skyrimse/meshes/actors/dlc02/dwarvenballistacenturion/character assets/skeleton.hkx",
    "extracted/skyrimse/meshes/actors/dlc02/hmdaedra/character assets/skeleton.hkx",
    "extracted/skyrimse/meshes/actors/dlc02/netch/characterassets/skeleton.hkx",
    "extracted/skyrimse/meshes/actors/dlc02/riekling/character assets/skeleton.hkx",
    "extracted/skyrimse/meshes/actors/dlc02/scrib/character assets/skeleton.hkx",
    "extracted/skyrimse/meshes/actors/dragon/character assets/skeleton.hkx",
    "extracted/skyrimse/meshes/actors/dragonpriest/character assets/skeleton.hkx",
    "extracted/skyrimse/meshes/actors/draugr/character assets/skeleton.hkx",
    "extracted/skyrimse/meshes/actors/draugr/character assets/skeletonf.hkx",
    "extracted/skyrimse/meshes/actors/draugr/character assets/skeletons.hkx",
    "extracted/skyrimse/meshes/actors/dwarvenspherecenturion/character assets/skeleton.hkx",
    "extracted/skyrimse/meshes/actors/dwarvenspider/character assets/skeleton.hkx",
    "extracted/skyrimse/meshes/actors/dwarvensteamcenturion/character assets/skeleton.hkx",
    "extracted/skyrimse/meshes/actors/falmer/character assets/skeleton.hkx",
    "extracted/skyrimse/meshes/actors/frostbitespider/character assets/skeleton.hkx",
    "extracted/skyrimse/meshes/actors/giant/character assets/skeleton.hkx",
    "extracted/skyrimse/meshes/actors/goat/character assets/skeleton.hkx",
    "extracted/skyrimse/meshes/actors/hagraven/character assets/skeleton.hkx",
    "extracted/skyrimse/meshes/actors/horker/character assets/skeleton.hkx",
    "extracted/skyrimse/meshes/actors/horse/character assets/skeleton.hkx",
    "extracted/skyrimse/meshes/actors/icewraith/character assets/skeleton.hkx",
    "extracted/skyrimse/meshes/actors/mammoth/character assets/skeleton.hkx",
    "extracted/skyrimse/meshes/actors/mudcrab/character assets/skeleton.hkx",
    "extracted/skyrimse/meshes/actors/sabrecat/character assets/skeleton.hkx",
    "extracted/skyrimse/meshes/actors/skeever/character assets/skeleton.hkx",
    "extracted/skyrimse/meshes/actors/slaughterfish/character assets/skeleton.hkx",
    "extracted/skyrimse/meshes/actors/spriggan/character assets/skeleton.hkx",
    "extracted/skyrimse/meshes/actors/troll/character assets/skeleton.hkx",
    "extracted/skyrimse/meshes/actors/vampirelord/character assets/skeleton.hkx",
    "extracted/skyrimse/meshes/actors/werewolfbeast/character assets/skeleton.hkx",
    "extracted/skyrimse/meshes/actors/wisp/character assets/skeleton.hkx",
];

// Ordered identities and parents decoded from Skyrim SE's actors/canine/character assets wolf/skeleton.hkx.
const WOLF_BONES: [(&str, Option<usize>); 50] = [
    ("NPC Root [Root]", None),
    ("Canine_COM", Some(0)),
    ("Canine_Pelvis", Some(1)),
    ("Canine_Spine1", Some(1)),
    ("Canine_Spine2", Some(3)),
    ("Canine_Spine3", Some(4)),
    ("Canine_Ribcage", Some(5)),
    ("Canine_Neck1", Some(6)),
    ("Canine_Neck2", Some(7)),
    ("Canine_Head", Some(8)),
    ("Canine_LFrontLegShoulderblade", Some(6)),
    ("Canine_RFrontLegShoulderblade", Some(6)),
    ("Canine_LFrontLeg1", Some(10)),
    ("Canine_RFrontLeg1", Some(11)),
    ("Canine_LBackLeg1", Some(2)),
    ("Canine_RBackLeg1", Some(2)),
    ("Canine_LFrontLeg2", Some(12)),
    ("Canine_RFrontLeg2", Some(13)),
    ("Canine_LBackLeg2", Some(14)),
    ("Canine_RBackLeg2", Some(15)),
    ("Canine_LFrontLegPalm", Some(16)),
    ("Canine_RFrontLegPalm", Some(17)),
    ("Canine_LBackLegPalm", Some(18)),
    ("Canine_RBackLegPalm", Some(19)),
    ("Canine_Tail1", Some(2)),
    ("Canine_Tail2", Some(24)),
    ("Canine_Tail3", Some(25)),
    ("Canine_LFrontLegToe", Some(20)),
    ("Canine_RFrontLegToe", Some(21)),
    ("Canine_LBackLegToe", Some(22)),
    ("Canine_RBackLegToe", Some(23)),
    ("Canine_JawBone", Some(9)),
    ("Canine_LEar_Wolf", Some(9)),
    ("Canine_REar_Wolf", Some(9)),
    ("Canine_LEar01", Some(9)),
    ("Canine_LEar02", Some(34)),
    ("Canine_REar01", Some(9)),
    ("Canine_REar02", Some(36)),
    ("Canine_Dog_LEyelid", Some(9)),
    ("Canine_Dog_REyelid", Some(9)),
    ("Canine_LEye", Some(9)),
    ("Canine_REye", Some(9)),
    ("Canine_FrontLip", Some(9)),
    ("Canine_Tongue01", Some(31)),
    ("Canine_Tongue02", Some(43)),
    ("Canine_Tongue03", Some(44)),
    ("Canine_LUpperLip", Some(9)),
    ("Canine_RUpperLip", Some(9)),
    ("Canine_Dog_LBrow", Some(9)),
    ("Canine_Dog_RBrow", Some(9)),
];

fn bone(name: &str, parent: Option<usize>) -> RigBoneIr {
    RigBoneIr {
        name: name.to_string(),
        parent,
        reference_pose: QsTransformIr::identity(),
        lock_translation: false,
    }
}

fn cube() -> RagdollShapeIr {
    RagdollShapeIr::ConvexHull(ConvexHullIr {
        vertices: vec![
            [-0.5, -0.5, -0.5],
            [-0.5, -0.5, 0.5],
            [-0.5, 0.5, -0.5],
            [-0.5, 0.5, 0.5],
            [0.5, -0.5, -0.5],
            [0.5, -0.5, 0.5],
            [0.5, 0.5, -0.5],
            [0.5, 0.5, 0.5],
        ],
        planes: vec![
            [1.0, 0.0, 0.0, -0.5],
            [-1.0, 0.0, 0.0, -0.5],
            [0.0, 1.0, 0.0, -0.5],
            [0.0, -1.0, 0.0, -0.5],
            [0.0, 0.0, 1.0, -0.5],
            [0.0, 0.0, -1.0, -0.5],
        ],
        convex_radius: 0.05,
    })
}

fn body(name: &str, ragdoll_bone: &str) -> RigidBodyIr {
    RigidBodyIr {
        name: name.to_string(),
        ragdoll_bone: ragdoll_bone.to_string(),
        shape: cube(),
        world_from_body: QsTransformIr::identity(),
        mass_properties: MassPropertiesIr {
            mass: 1.0,
            center_of_mass: [0.0; 3],
            inertia_diagonal: [0.2; 3],
        },
        friction: 0.5,
        restitution: 0.0,
        linear_damping: 0.1,
        angular_damping: 0.1,
        collision_filter_info: 0,
    }
}

fn hinge(name: &str, body_a: &str, body_b: &str) -> ConstraintIr {
    ConstraintIr {
        name: name.to_string(),
        body_a: body_a.to_string(),
        body_b: body_b.to_string(),
        kind: ConstraintKindIr::LimitedHinge(LimitedHingeIr {
            frame_a: QsTransformIr::identity(),
            frame_b: QsTransformIr::identity(),
            min_angle: -0.5,
            max_angle: 0.5,
            max_friction_torque: 1.0,
        }),
    }
}

fn mapping(ragdoll_bone: &str, animation_bone: &str) -> BoneMappingIr {
    BoneMappingIr {
        ragdoll_bone: ragdoll_bone.to_string(),
        animation_bone: animation_bone.to_string(),
        ragdoll_from_animation: QsTransformIr::identity(),
    }
}

fn wolf_ir() -> CreatureRagdollIr {
    CreatureRagdollIr {
        name: "SkyrimWolfOwnedRig".to_string(),
        animation_skeleton: RigSkeletonIr {
            name: "WolfAnimationSkeleton".to_string(),
            bones: WOLF_BONES
                .iter()
                .map(|(name, parent)| bone(name, *parent))
                .collect(),
        },
        ragdoll_skeleton: RigSkeletonIr {
            name: "WolfRagdollSkeleton".to_string(),
            bones: vec![
                bone("Canine_COM", None),
                bone("Canine_Pelvis", Some(0)),
                bone("Canine_Head", Some(1)),
            ],
        },
        bodies: vec![
            body("Wolf_COM", "Canine_COM"),
            body("Wolf_Pelvis", "Canine_Pelvis"),
            body("Wolf_Head", "Canine_Head"),
        ],
        constraints: vec![
            hinge("Wolf_COM_Pelvis", "Wolf_COM", "Wolf_Pelvis"),
            hinge("Wolf_Pelvis_Head", "Wolf_Pelvis", "Wolf_Head"),
        ],
        mappings: vec![
            mapping("Canine_COM", "Canine_COM"),
            mapping("Canine_Pelvis", "Canine_Pelvis"),
            mapping("Canine_Head", "Canine_Head"),
        ],
    }
}

fn mechanical_ir() -> CreatureRagdollIr {
    let animation_skeleton = RigSkeletonIr {
        name: "MechanicalAnimationSkeleton".to_string(),
        bones: vec![bone("MachineRoot", None), bone("MachineRotor", Some(0))],
    };
    CreatureRagdollIr {
        name: "MechanicalOwnedRig".to_string(),
        ragdoll_skeleton: animation_skeleton.clone(),
        animation_skeleton,
        bodies: vec![
            body("MachineRootBody", "MachineRoot"),
            body("MachineRotorBody", "MachineRotor"),
        ],
        constraints: vec![hinge(
            "MachineRotorHinge",
            "MachineRootBody",
            "MachineRotorBody",
        )],
        mappings: vec![
            mapping("MachineRoot", "MachineRoot"),
            mapping("MachineRotor", "MachineRotor"),
        ],
    }
}

fn class_count(objects: &[HkxObject], class_name: &str) -> usize {
    objects
        .iter()
        .filter(|object| object.class_name == class_name)
        .count()
}

fn skeleton_bone_count(object: &HkxObject) -> usize {
    let value = &object
        .members
        .iter()
        .find(|member| member.name == "bones")
        .expect("skeleton bones")
        .value;
    let HkxValue::Array(bones) = value else {
        panic!("hkaSkeleton.bones is not an array")
    };
    bones.len()
}

#[test]
fn skyrim_wolf_owned_topology_packs_and_rereads_as_fo4_ragdoll() {
    let packed = reconstruct_fo4_creature_ragdoll_packfile(&wolf_ir()).expect("pack wolf rig");
    assert_eq!(&packed[..4], &[0x57, 0xe0, 0xe0, 0x57]);

    let reread = HkxFile::read(&packed).expect("reread wolf rig");
    assert_eq!(reread.class_version(), 11);
    assert_eq!(reread.contents_version(), "hk_2014.1.0-r1");
    assert_eq!(class_count(reread.objects(), "hkpRigidBody"), 3);
    assert_eq!(class_count(reread.objects(), "hkpConstraintInstance"), 4);
    assert_eq!(class_count(reread.objects(), "hkaSkeletonMapper"), 2);

    let animation_skeleton = reread
        .objects()
        .iter()
        .find(|object| {
            object.class_name == "hkaSkeleton"
                && object.members.iter().any(|member| {
                    member.name == "name"
                        && member.value
                            == HkxValue::String {
                                value: "WolfAnimationSkeleton".to_string(),
                                is_null: false,
                            }
                })
        })
        .expect("wolf animation skeleton");
    assert_eq!(skeleton_bone_count(animation_skeleton), WOLF_BONES.len());
}

#[test]
fn skyrim_wolf_ragdoll_embeds_as_fo4_hknp_data_with_bone_targets() {
    let ir = wolf_ir();
    let embedded = reconstruct_fo4_embedded_creature_ragdoll(&ir)
        .expect("embed wolf ragdoll for Skeleton.nif");
    let reread = HkxFile::read(&embedded.binary_data).expect("reread embedded wolf ragdoll");

    assert_eq!(reread.objects()[0].class_name, "hknpRagdollData");
    assert_eq!(class_count(reread.objects(), "hkaSkeleton"), 1);
    assert_eq!(embedded.animation_bone_targets.len(), ir.bodies.len());
    assert_eq!(
        embedded.animation_bone_targets,
        ir.bodies
            .iter()
            .map(|body| body.ragdoll_bone.clone())
            .collect::<Vec<_>>()
    );
}

#[test]
fn representative_mechanical_owned_rig_round_trips_without_donor_objects() {
    let packed =
        reconstruct_fo4_creature_ragdoll_packfile(&mechanical_ir()).expect("pack mechanical rig");
    let reread = HkxFile::read(&packed).expect("reread mechanical rig");
    assert_eq!(class_count(reread.objects(), "hkpRigidBody"), 2);
    assert_eq!(
        class_count(reread.objects(), "hkpLimitedHingeConstraintData"),
        1
    );
    assert_eq!(class_count(reread.objects(), "hkpConstraintInstance"), 2);
    assert!(!reread.objects().iter().any(|object| {
        matches!(
            object.class_name.as_str(),
            "hkpCapsuleShape" | "hkpBoxShape" | "hkpSphereShape"
        )
    }));
}

#[test]
fn unsupported_legacy_capsule_is_a_typed_target_blocker() {
    let mut ir = mechanical_ir();
    ir.bodies[0].shape = RagdollShapeIr::Capsule {
        vertex_a: [0.0, 0.0, -0.5],
        vertex_b: [0.0, 0.0, 0.5],
        radius: 0.25,
    };
    assert!(matches!(
        reconstruct_fo4_creature_ragdoll_packfile(&ir),
        Err(CreatureRagdollError::UnsupportedTargetShape { shape, .. }) if shape == "capsule"
    ));
}

#[test]
fn fo4_ragdoll_and_fixed_constraints_pack_with_target_signatures() {
    let mut ir = mechanical_ir();
    ir.constraints[0].kind = ConstraintKindIr::Fixed {
        frame_a: QsTransformIr::identity(),
        frame_b: QsTransformIr::identity(),
    };
    ir.constraints.push(ConstraintIr {
        name: "MachineRotorRagdoll".to_string(),
        body_a: "MachineRootBody".to_string(),
        body_b: "MachineRotorBody".to_string(),
        kind: ConstraintKindIr::Ragdoll(RagdollLimitsIr {
            frame_a: QsTransformIr::identity(),
            frame_b: QsTransformIr::identity(),
            cone_limit: 0.6,
            plane_min: -0.4,
            plane_max: 0.7,
            twist_min: -0.2,
            twist_max: 0.2,
            max_friction_torque: 3.0,
        }),
    });
    let packed = reconstruct_fo4_creature_ragdoll_packfile(&ir).expect("pack constraints");
    let reread = HkxFile::read(&packed).expect("reread constraints");
    let fixed = reread
        .objects()
        .iter()
        .find(|object| object.class_name == "hkpFixedConstraintData")
        .expect("fixed constraint data");
    let ragdoll = reread
        .objects()
        .iter()
        .find(|object| object.class_name == "hkpRagdollConstraintData")
        .expect("ragdoll constraint data");
    assert_eq!(fixed.signature, 0x302a_dc45);
    assert_eq!(ragdoll.signature, 0xb77d_2036);
    assert_eq!(class_count(reread.objects(), "hkpConstraintInstance"), 4);

    let motors = ragdoll
        .members
        .iter()
        .find(|member| member.name == "atoms")
        .and_then(|member| member.value.as_object_members())
        .and_then(|atoms| atoms.iter().find(|member| member.name == "ragdollMotors"))
        .and_then(|member| member.value.as_object_members())
        .and_then(|fields| fields.iter().find(|member| member.name == "motors"))
        .expect("ragdoll motor slots");
    assert_eq!(
        motors.value,
        HkxValue::Array(vec![
            HkxValue::Pointer(None),
            HkxValue::Pointer(None),
            HkxValue::Pointer(None),
        ])
    );
}

#[test]
fn source_primitives_convexify_without_donor_geometry_and_pack() {
    let shapes = [
        RagdollShapeIr::Capsule {
            vertex_a: [0.0, 0.0, -0.5],
            vertex_b: [0.0, 0.0, 0.5],
            radius: 0.25,
        },
        RagdollShapeIr::Sphere {
            center: [0.2, -0.1, 0.3],
            radius: 0.4,
        },
        RagdollShapeIr::Box {
            half_extents: [0.25, 0.5, 0.75],
            convex_radius: 0.05,
        },
    ];
    for shape in shapes {
        let mut ir = mechanical_ir();
        ir.bodies[0].shape = shape;
        let lowered = lower_primitive_shapes_to_fo4_convex_hulls(&ir, 12)
            .expect("convexify source primitive");
        assert!(matches!(
            lowered.bodies[0].shape,
            RagdollShapeIr::ConvexHull(_)
        ));
        reconstruct_fo4_creature_ragdoll_packfile(&lowered)
            .expect("pack convexified source primitive");
    }
}

#[test]
fn body_without_animation_mapping_is_rejected_before_packing() {
    let mut ir = mechanical_ir();
    ir.mappings.pop();
    assert_eq!(
        reconstruct_fo4_creature_ragdoll_packfile(&ir),
        Err(CreatureRagdollError::BodyMappingMissing {
            body: "MachineRotorBody".to_string(),
        })
    );
}

#[test]
fn constraint_must_close_over_emitted_rigid_bodies() {
    let mut ir = mechanical_ir();
    ir.constraints[0].body_b = "MissingBody".to_string();
    assert_eq!(
        reconstruct_fo4_creature_ragdoll_packfile(&ir),
        Err(CreatureRagdollError::ConstraintBodyMissing {
            constraint: "MachineRotorHinge".to_string(),
            body: "MissingBody".to_string(),
        })
    );
}

#[test]
fn non_finite_physics_values_are_rejected_before_packing() {
    let mut ir = mechanical_ir();
    ir.bodies[0].mass_properties.inertia_diagonal[1] = f32::NAN;
    assert!(matches!(
        reconstruct_fo4_creature_ragdoll_packfile(&ir),
        Err(CreatureRagdollError::NonFinite { path }) if path.ends_with("inertia[1]")
    ));
}

#[test]
fn real_skyrim_wolf_extracts_its_own_complete_ragdoll_and_packs_for_fo4() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "../../../extracted/skyrimse/meshes/actors/canine/character assets wolf/skeleton.hkx",
    );
    let source = HkxFile::read(&std::fs::read(path).expect("read wolf skeleton"))
        .expect("decode wolf skeleton");
    let ir = extract_skyrim_2010_creature_ragdoll(&source).expect("extract wolf ragdoll");
    assert_eq!(ir.animation_skeleton.bones.len(), 50);
    assert_eq!(ir.ragdoll_skeleton.bones.len(), 22);
    assert_eq!(ir.bodies.len(), 22);
    assert_eq!(ir.constraints.len(), 21);
    assert_eq!(ir.mappings.len(), 22);
    assert!(
        ir.bodies
            .iter()
            .all(|body| matches!(body.shape, RagdollShapeIr::Capsule { .. }))
    );
    assert_eq!(
        ir.constraints
            .iter()
            .filter(|constraint| matches!(constraint.kind, ConstraintKindIr::Ragdoll(_)))
            .count(),
        11
    );
    assert_eq!(
        ir.constraints
            .iter()
            .filter(|constraint| matches!(constraint.kind, ConstraintKindIr::LimitedHinge(_)))
            .count(),
        10
    );
    assert_eq!(ir.mappings[0].ragdoll_bone, "Ragdoll_Canine_COM");
    assert_eq!(ir.mappings[0].animation_bone, "Canine_COM");
    assert_ne!(
        ir.mappings[0].ragdoll_from_animation,
        QsTransformIr::identity()
    );

    let lowered =
        lower_primitive_shapes_to_fo4_convex_hulls(&ir, 12).expect("convexify wolf capsules");
    let packed =
        reconstruct_fo4_creature_ragdoll_packfile(&lowered).expect("pack real wolf owned ragdoll");
    let reread = HkxFile::read(&packed).expect("reread real wolf owned ragdoll");
    assert_eq!(class_count(reread.objects(), "hkpRigidBody"), 22);
    assert_eq!(class_count(reread.objects(), "hkpConstraintInstance"), 42);
    assert_eq!(
        class_count(reread.objects(), "hkpRagdollConstraintData"),
        11
    );
    assert_eq!(
        class_count(reread.objects(), "hkpLimitedHingeConstraintData"),
        10
    );
}

#[test]
fn all_48_skyrim_ragdolls_have_complete_source_owned_fo4_pack_closure() {
    assert_eq!(SKYRIM_RAGDOLL_CORPUS.len(), 48);
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let mut failures = Vec::new();
    for relative in SKYRIM_RAGDOLL_CORPUS {
        let result = std::fs::read(root.join(relative))
            .map_err(|error| error.to_string())
            .and_then(|bytes| HkxFile::read(&bytes).map_err(|error| error.to_string()))
            .and_then(|source| {
                extract_skyrim_2010_creature_ragdoll(&source).map_err(|error| error.to_string())
            })
            .and_then(|ir| {
                lower_primitive_shapes_to_fo4_convex_hulls(&ir, 12)
                    .map_err(|error| error.to_string())
            })
            .and_then(|ir| {
                reconstruct_fo4_creature_ragdoll_packfile(&ir)
                    .map(|_| ())
                    .map_err(|error| error.to_string())
            });
        if let Err(error) = result {
            failures.push(format!("{relative}: {error}"));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}
