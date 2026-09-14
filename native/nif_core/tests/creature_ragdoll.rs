use havok_native::convert::creature_ragdoll::{
    ConstraintKindIr, RagdollShapeIr, extract_skyrim_2010_creature_ragdoll,
    lower_primitive_shapes_to_fo4_convex_hulls, reconstruct_fo4_creature_ragdoll_packfile,
};
use havok_native::hkx::HkxFile;
use havok_native::hkx::types::HkxValue;
use nif_core_native::convert_file::{
    ConvertFileOptions, SourceRigNifKind, stage_preserve_source_rig_nif,
};
use nif_core_native::creature_ragdoll::{
    extract_fnv_fo3_creature_ragdoll, install_fo4_creature_controller_collision,
    install_fo4_creature_ragdoll_collision,
};
use nif_core_native::model::{NifFile, NifValue};

fn fixture(relative: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)
}

fn class_count(file: &HkxFile, class_name: &str) -> usize {
    file.objects()
        .iter()
        .filter(|object| object.class_name == class_name)
        .count()
}

fn member_array_len(file: &HkxFile, class_name: &str, member_name: &str) -> usize {
    let object = file
        .objects()
        .iter()
        .find(|object| object.class_name == class_name)
        .unwrap_or_else(|| panic!("missing {class_name}"));
    match &object
        .members
        .iter()
        .find(|member| member.name == member_name)
        .unwrap_or_else(|| panic!("{class_name} has no {member_name}"))
        .value
    {
        HkxValue::Array(values) => values.len(),
        other => panic!("{class_name}.{member_name} is not an array: {other:?}"),
    }
}

fn embedded_havok_blob(nif: &NifFile, block_type: &str) -> Vec<u8> {
    let block = nif
        .blocks
        .iter()
        .find(|block| block.type_name == block_type)
        .unwrap_or_else(|| panic!("missing {block_type}"));
    let Some(NifValue::Struct(binary_data)) = block.get_field("Binary Data") else {
        panic!("{block_type} has no Binary Data struct");
    };
    match binary_data.get("Data") {
        Some(NifValue::Bytes(bytes)) => bytes.clone(),
        Some(NifValue::Array(values)) => values.iter().map(|value| value.as_i64() as u8).collect(),
        other => panic!("{block_type} has malformed Binary Data: {other:?}"),
    }
}

fn assert_filter_suppresses_exact_constraint_pairs(
    ir: &havok_native::convert::creature_ragdoll::CreatureRagdollIr,
) {
    let constrained = ir
        .constraints
        .iter()
        .map(|constraint| {
            if constraint.body_a < constraint.body_b {
                (constraint.body_a.as_str(), constraint.body_b.as_str())
            } else {
                (constraint.body_b.as_str(), constraint.body_a.as_str())
            }
        })
        .collect::<std::collections::HashSet<_>>();
    for (index, left) in ir.bodies.iter().enumerate() {
        for right in &ir.bodies[index + 1..] {
            let left_subsystem = (left.collision_filter_info >> 5) & 0x1f;
            let left_exclusion = (left.collision_filter_info >> 10) & 0x1f;
            let right_subsystem = (right.collision_filter_info >> 5) & 0x1f;
            let right_exclusion = (right.collision_filter_info >> 10) & 0x1f;
            let suppressed = left_subsystem == right_exclusion || right_subsystem == left_exclusion;
            let key = if left.name.as_str() < right.name.as_str() {
                (left.name.as_str(), right.name.as_str())
            } else {
                (right.name.as_str(), left.name.as_str())
            };
            assert_eq!(
                suppressed,
                constrained.contains(&key),
                "filter pair {key:?}"
            );
        }
    }
}

const ORDINARY_ARTICULATED_CREATURES: &[&str] = &[
    "extracted/fnv/meshes/creatures/alien/alien.nif",
    "extracted/fnv/meshes/creatures/blowfly/skeleton.nif",
    "extracted/fnv/meshes/creatures/brahmin/idleanims/skeletondeathpose.nif",
    "extracted/fnv/meshes/creatures/brahmin/skeleton.nif",
    "extracted/fnv/meshes/creatures/centaur/skeleton.nif",
    "extracted/fnv/meshes/creatures/centaur/skeletonevolved.nif",
    "extracted/fnv/meshes/creatures/deathclaw/skeleton.nif",
    "extracted/fnv/meshes/creatures/dog/skeleton_sonicbark.nif",
    "extracted/fnv/meshes/creatures/dog/skeleton.nif",
    "extracted/fnv/meshes/creatures/eyebot/skeleton_low.nif",
    "extracted/fnv/meshes/creatures/eyebot/skeleton.nif",
    "extracted/fnv/meshes/creatures/failedfevsubject/failedfevsubject.nif",
    "extracted/fnv/meshes/creatures/ghoul/skeleton.nif",
    "extracted/fnv/meshes/creatures/giantant/skeleton.nif",
    "extracted/fnv/meshes/creatures/libertyprime/skeleton.nif",
    "extracted/fnv/meshes/creatures/mirelurk/skeleton.nif",
    "extracted/fnv/meshes/creatures/mirelurk/skeleton2.nif",
    "extracted/fnv/meshes/creatures/mirelurkking/skeleton.nif",
    "extracted/fnv/meshes/creatures/mistergutsy/skeleton.nif",
    "extracted/fnv/meshes/creatures/molerat/skeleton.nif",
    "extracted/fnv/meshes/creatures/nightstalker/skeleton.nif",
    "extracted/fnv/meshes/creatures/nvbighorner/skeleton.nif",
    "extracted/fnv/meshes/creatures/nvcazadores/skeleton.nif",
    "extracted/fnv/meshes/creatures/nvgecko/skeleton.nif",
    "extracted/fnv/meshes/creatures/nvgiantrat/skeleton.nif",
    "extracted/fnv/meshes/creatures/nvmantis/skeleton.nif",
    "extracted/fnv/meshes/creatures/nvmrhouse/skeleton.nif",
    "extracted/fnv/meshes/creatures/nvraven/skeleton.nif",
    "extracted/fnv/meshes/creatures/nvsecuritron/skeleton.nif",
    "extracted/fnv/meshes/creatures/nvsporecarrier/skeleton.nif",
    "extracted/fnv/meshes/creatures/nvsporeplant/skeleton.nif",
    "extracted/fnv/meshes/creatures/nvtumbleweed/skeleton.nif",
    "extracted/fnv/meshes/creatures/nvvoid/skeleton.nif",
    "extracted/fnv/meshes/creatures/queenant/skeleton.nif",
    "extracted/fnv/meshes/creatures/radscorpion/skeleton_roboscopion.nif",
    "extracted/fnv/meshes/creatures/radscorpion/skeleton.nif",
    "extracted/fnv/meshes/creatures/robobrain/skeleton.nif",
    "extracted/fnv/meshes/creatures/sentrybot/skeleton.nif",
    "extracted/fnv/meshes/creatures/smbehemoth/skeleton.nif",
    "extracted/fnv/meshes/creatures/smspinebreaker/skeleton.nif",
    "extracted/fnv/meshes/creatures/yaoguai/skeleton.nif",
    "extracted/fo3/meshes/creatures/alien/alien.nif",
    "extracted/fo3/meshes/creatures/blowfly/skeleton.nif",
    "extracted/fo3/meshes/creatures/brahmin/idleanims/skeletondeathpose.nif",
    "extracted/fo3/meshes/creatures/brahmin/skeleton.nif",
    "extracted/fo3/meshes/creatures/centaur/skeleton.nif",
    "extracted/fo3/meshes/creatures/deathclaw/skeleton.nif",
    "extracted/fo3/meshes/creatures/dog/skeleton.nif",
    "extracted/fo3/meshes/creatures/eyebot/skeleton.nif",
    "extracted/fo3/meshes/creatures/failedfevsubject/failedfevsubject.nif",
    "extracted/fo3/meshes/creatures/ghoul/skeleton.nif",
    "extracted/fo3/meshes/creatures/giantant/skeleton.nif",
    "extracted/fo3/meshes/creatures/libertyprime/skeleton.nif",
    "extracted/fo3/meshes/creatures/mirelurk/skeleton.nif",
    "extracted/fo3/meshes/creatures/mirelurk/skeleton2.nif",
    "extracted/fo3/meshes/creatures/mirelurkking/skeleton.nif",
    "extracted/fo3/meshes/creatures/mistergutsy/skeleton.nif",
    "extracted/fo3/meshes/creatures/molerat/skeleton.nif",
    "extracted/fo3/meshes/creatures/queenant/skeleton.nif",
    "extracted/fo3/meshes/creatures/radscorpion/skeleton.nif",
    "extracted/fo3/meshes/creatures/robobrain/skeleton.nif",
    "extracted/fo3/meshes/creatures/sentrybot/skeleton.nif",
    "extracted/fo3/meshes/creatures/smbehemoth/skeleton.nif",
    "extracted/fo3/meshes/creatures/smspinebreaker/skeleton.nif",
    "extracted/fo3/meshes/creatures/yaoguai/skeleton.nif",
];

const SPECIAL_ARTICULATED_CREATURES: &[&str] = &[
    "extracted/fnv/meshes/creatures/minisentryturret/skeleton.nif",
    "extracted/fnv/meshes/creatures/protectron/skeleton.nif",
    "extracted/fnv/meshes/creatures/radroach/skeleton.nif",
    "extracted/fnv/meshes/creatures/sentryturret/skeleton.nif",
    "extracted/fnv/meshes/creatures/zaxeye/skeleton.nif",
    "extracted/fo3/meshes/creatures/minisentryturret/skeleton.nif",
    "extracted/fo3/meshes/creatures/protectron/skeleton.nif",
    "extracted/fo3/meshes/creatures/radroach/skeleton.nif",
    "extracted/fo3/meshes/creatures/sentryturret/skeleton.nif",
    "extracted/fo3/meshes/creatures/zaxeye/skeleton.nif",
];

#[test]
fn real_fnv_dog_extracts_its_own_rig_and_packs_for_fo4() {
    let nif = NifFile::load(fixture(
        "../../../extracted/fnv/meshes/creatures/dog/skeleton.nif",
    ))
    .expect("load FNV dog skeleton");
    let ir = extract_fnv_fo3_creature_ragdoll(&nif).expect("extract FNV dog ragdoll");

    assert_eq!(ir.animation_skeleton.bones[0].name, "Bip01");
    assert_eq!(ir.ragdoll_skeleton.bones[0].name, "Bip01 Spine0");
    assert_eq!(ir.bodies[0].name, "Bip01 Spine0_body");
    assert_eq!(ir.bodies.len(), 21);
    assert_eq!(ir.ragdoll_skeleton.bones.len(), 21);
    assert_eq!(ir.mappings.len(), 21);
    assert_eq!(ir.constraints.len(), 20);
    assert_eq!(
        ir.bodies
            .iter()
            .filter(|body| matches!(body.shape, RagdollShapeIr::Capsule { .. }))
            .count(),
        18
    );
    assert_eq!(
        ir.bodies
            .iter()
            .filter(|body| matches!(body.shape, RagdollShapeIr::Sphere { .. }))
            .count(),
        3
    );
    assert_eq!(
        ir.constraints
            .iter()
            .filter(|constraint| matches!(constraint.kind, ConstraintKindIr::Ragdoll(_)))
            .count(),
        12
    );
    assert_eq!(
        ir.constraints
            .iter()
            .filter(|constraint| matches!(constraint.kind, ConstraintKindIr::LimitedHinge(_)))
            .count(),
        8
    );
    assert_eq!(ir.bodies[0].collision_filter_info, 0x0001_0420);
    assert!(ir.bodies.iter().all(|body| {
        body.world_from_body
            .translation
            .iter()
            .all(|value| value.is_finite())
    }));

    let neck = ir
        .bodies
        .iter()
        .find(|body| body.name == "Bip01 Neck_body")
        .expect("neck body");
    let RagdollShapeIr::Capsule {
        vertex_a,
        vertex_b,
        radius,
    } = neck.shape
    else {
        panic!("neck must retain its source capsule")
    };
    assert!((radius - 6.202_3).abs() < 0.01);
    assert!((vertex_a[0] - 4.843).abs() < 0.01);
    assert!((vertex_b[0] - 3.301).abs() < 0.01);

    let lowered = lower_primitive_shapes_to_fo4_convex_hulls(&ir, 12)
        .expect("convexify source-owned dog primitives");
    let packed =
        reconstruct_fo4_creature_ragdoll_packfile(&lowered).expect("pack source-owned dog ragdoll");
    let reread = HkxFile::read(&packed).expect("reread source-owned dog ragdoll");
    assert_eq!(reread.class_version(), 11);
    assert_eq!(reread.contents_version(), "hk_2014.1.0-r1");
    assert_eq!(class_count(&reread, "hkpRigidBody"), 21);
    assert_eq!(class_count(&reread, "hkpConstraintInstance"), 40);
    assert_eq!(class_count(&reread, "hkpRagdollConstraintData"), 12);
    assert_eq!(class_count(&reread, "hkpLimitedHingeConstraintData"), 8);
}

#[test]
fn real_fnv_dog_stages_with_embedded_fo4_ragdoll_and_controller_collision() {
    let source = fixture("../../../extracted/fnv/meshes/creatures/dog/skeleton.nif");
    let source_nif = NifFile::load(&source).expect("load FNV dog skeleton");
    let ir = extract_fnv_fo3_creature_ragdoll(&source_nif).expect("extract FNV dog ragdoll");
    let lowered = lower_primitive_shapes_to_fo4_convex_hulls(&ir, 12)
        .expect("convexify source-owned dog primitives");
    let temp = tempfile::tempdir().expect("tempdir");
    let target = temp.path().join("skeleton.nif");
    stage_preserve_source_rig_nif(
        SourceRigNifKind::Skeleton,
        &source,
        &target,
        r"Meshes\Actors\SourceRig\Dog\Skeleton.nif",
        "fnv",
        None,
        &ConvertFileOptions::default(),
    )
    .expect("stage FNV dog visual skeleton");

    let mut staged = NifFile::load(&target).expect("load staged dog skeleton");
    assert!(
        staged
            .blocks
            .iter()
            .all(|block| !block.type_name.starts_with("bhk"))
    );
    let body_count = install_fo4_creature_ragdoll_collision(&mut staged, &lowered)
        .expect("install articulated ragdoll collision");
    assert_eq!(body_count, 21);
    install_fo4_creature_controller_collision(
        &mut staged,
        80.0 / 69.99125,
        20.0 / 69.99125,
        [0.0, 0.0, 1.0],
        1,
    )
    .expect("install controller collision");
    staged
        .save(Some(target.clone()))
        .expect("save collision-bearing dog skeleton");

    let reread = NifFile::load(target).expect("reread collision-bearing dog skeleton");
    assert_eq!(
        reread
            .blocks
            .iter()
            .filter(|block| block.type_name == "bhkRagdollSystem")
            .count(),
        1
    );
    assert_eq!(
        reread
            .blocks
            .iter()
            .filter(|block| block.type_name == "bhkPhysicsSystem")
            .count(),
        1
    );
    assert_eq!(
        reread
            .blocks
            .iter()
            .filter(|block| block.type_name == "bhkNPCollisionObject")
            .count(),
        body_count + 1
    );

    let ragdoll = HkxFile::read(&embedded_havok_blob(&reread, "bhkRagdollSystem"))
        .expect("parse embedded ragdoll");
    assert_eq!(class_count(&ragdoll, "hknpRagdollData"), 1);
    assert_eq!(
        member_array_len(&ragdoll, "hknpRagdollData", "bodyCinfos"),
        body_count
    );
    let controller = HkxFile::read(&embedded_havok_blob(&reread, "bhkPhysicsSystem"))
        .expect("parse embedded controller");
    assert_eq!(class_count(&controller, "hknpCapsuleShape"), 1);
}

#[test]
fn sphere_equivalent_controller_bound_installs_fo4_sphere_collision() {
    let source = fixture("../../../extracted/fnv/meshes/creatures/dog/skeleton.nif");
    let temp = tempfile::tempdir().expect("tempdir");
    let target = temp.path().join("skeleton.nif");
    stage_preserve_source_rig_nif(
        SourceRigNifKind::Skeleton,
        &source,
        &target,
        r"Meshes\Actors\SourceRig\Dog\Skeleton.nif",
        "fnv",
        None,
        &ConvertFileOptions::default(),
    )
    .expect("stage FNV dog visual skeleton");

    let mut staged = NifFile::load(&target).expect("load staged dog skeleton");
    install_fo4_creature_controller_collision(&mut staged, 1.0, 0.5, [0.0, 0.0, 1.0], 1)
        .expect("install sphere-equivalent controller collision");
    staged
        .save(Some(target.clone()))
        .expect("save controller-bearing dog skeleton");

    let reread = NifFile::load(target).expect("reread controller-bearing dog skeleton");
    let controller = HkxFile::read(&embedded_havok_blob(&reread, "bhkPhysicsSystem"))
        .expect("parse embedded controller");
    assert_eq!(class_count(&controller, "hknpSphereShape"), 1);
    assert_eq!(class_count(&controller, "hknpCapsuleShape"), 0);
}

#[test]
fn real_skyrim_giant_stages_with_embedded_fo4_ragdoll_and_controller_collision() {
    let source_nif_path =
        fixture("../../../extracted/skyrimse/meshes/actors/giant/character assets/skeleton.nif");
    let source_hkx_path =
        fixture("../../../extracted/skyrimse/meshes/actors/giant/character assets/skeleton.hkx");
    let source_hkx = HkxFile::read(&std::fs::read(source_hkx_path).expect("read giant HKX"))
        .expect("parse giant HKX");
    let ir =
        extract_skyrim_2010_creature_ragdoll(&source_hkx).expect("extract Skyrim giant ragdoll");
    let lowered = lower_primitive_shapes_to_fo4_convex_hulls(&ir, 12)
        .expect("convexify Skyrim giant primitives");
    let temp = tempfile::tempdir().expect("tempdir");
    let target = temp.path().join("skeleton.nif");
    stage_preserve_source_rig_nif(
        SourceRigNifKind::Skeleton,
        &source_nif_path,
        &target,
        r"Meshes\Actors\SourceRig\Giant\Skeleton.nif",
        "skyrimse",
        None,
        &ConvertFileOptions::default(),
    )
    .expect("stage Skyrim giant visual skeleton");

    let mut staged = NifFile::load(&target).expect("load staged giant skeleton");
    assert!(
        staged
            .blocks
            .iter()
            .all(|block| !block.type_name.starts_with("bhk"))
    );
    let body_count = install_fo4_creature_ragdoll_collision(&mut staged, &lowered)
        .expect("install giant articulated ragdoll collision");
    assert_eq!(body_count, ir.bodies.len());
    install_fo4_creature_controller_collision(&mut staged, 2.0, 0.5, [0.0, 0.0, 1.0], 1)
        .expect("install giant controller collision");
    staged
        .save(Some(target.clone()))
        .expect("save collision-bearing giant skeleton");

    let reread = NifFile::load(target).expect("reread collision-bearing giant skeleton");
    assert_eq!(
        reread
            .blocks
            .iter()
            .filter(|block| block.type_name == "bhkNPCollisionObject")
            .count(),
        body_count + 1
    );
    let ragdoll = HkxFile::read(&embedded_havok_blob(&reread, "bhkRagdollSystem"))
        .expect("parse embedded giant ragdoll");
    assert_eq!(ragdoll.objects()[0].class_name, "hknpRagdollData");
    assert_eq!(
        member_array_len(&ragdoll, "hknpRagdollData", "bodyCinfos"),
        body_count
    );
    HkxFile::read(&embedded_havok_blob(&reread, "bhkPhysicsSystem"))
        .expect("parse embedded giant controller");
}

#[test]
fn real_fo3_dog_uses_the_same_source_owned_ragdoll_contract() {
    let nif = NifFile::load(fixture(
        "../../../extracted/fo3/meshes/creatures/dog/skeleton.nif",
    ))
    .expect("load FO3 dog skeleton");
    let ir = extract_fnv_fo3_creature_ragdoll(&nif).expect("extract FO3 dog ragdoll");
    assert_eq!(ir.bodies.len(), 21);
    assert_eq!(ir.constraints.len(), 20);
    assert_eq!(ir.animation_skeleton.bones[0].name, "Bip01");
}

#[test]
fn real_fnv_gecko_preserves_source_box_shapes_and_packs() {
    let nif = NifFile::load(fixture(
        "../../../extracted/fnv/meshes/creatures/nvgecko/skeleton.nif",
    ))
    .expect("load FNV Gecko skeleton");
    let ir = extract_fnv_fo3_creature_ragdoll(&nif).expect("extract FNV Gecko ragdoll");
    assert!(
        ir.bodies
            .iter()
            .any(|body| matches!(body.shape, RagdollShapeIr::Box { .. }))
    );
    let lowered = lower_primitive_shapes_to_fo4_convex_hulls(&ir, 12)
        .expect("convexify Gecko source primitives");
    reconstruct_fo4_creature_ragdoll_packfile(&lowered).expect("pack source-owned Gecko ragdoll");
}

#[test]
fn fnv_radroach_compound_shape_packs_as_fo4_list_shape() {
    let nif = NifFile::load(fixture(
        "../../../extracted/fnv/meshes/creatures/radroach/skeleton.nif",
    ))
    .expect("load FNV radroach skeleton");
    let ir = extract_fnv_fo3_creature_ragdoll(&nif).expect("extract FNV radroach ragdoll");
    assert_eq!(
        ir.bodies
            .iter()
            .filter(|body| matches!(body.shape, RagdollShapeIr::Compound { .. }))
            .count(),
        1
    );
    let lowered = lower_primitive_shapes_to_fo4_convex_hulls(&ir, 12)
        .expect("convexify radroach compound children");
    let packed = reconstruct_fo4_creature_ragdoll_packfile(&lowered)
        .expect("pack source-owned radroach ragdoll");
    let reread = HkxFile::read(&packed).expect("reread radroach ragdoll");
    assert_eq!(class_count(&reread, "hkpListShape"), 1);
}

#[test]
fn fnv_zaxeye_unrestricted_hinge_packs_as_fo4_hinge() {
    let nif = NifFile::load(fixture(
        "../../../extracted/fnv/meshes/creatures/zaxeye/skeleton.nif",
    ))
    .expect("load FNV ZAX eye skeleton");
    let ir = extract_fnv_fo3_creature_ragdoll(&nif).expect("extract FNV ZAX eye ragdoll");
    assert_eq!(
        ir.constraints
            .iter()
            .filter(|constraint| matches!(constraint.kind, ConstraintKindIr::Hinge(_)))
            .count(),
        1
    );
    let lowered =
        lower_primitive_shapes_to_fo4_convex_hulls(&ir, 12).expect("convexify ZAX eye shapes");
    let packed = reconstruct_fo4_creature_ragdoll_packfile(&lowered)
        .expect("pack source-owned ZAX eye ragdoll");
    let reread = HkxFile::read(&packed).expect("reread ZAX eye ragdoll");
    assert_eq!(class_count(&reread, "hkpHingeConstraintData"), 1);
}

#[test]
fn ordinary_fnv_fo3_articulated_corpus_has_complete_fo4_pack_closure() {
    assert_eq!(ORDINARY_ARTICULATED_CREATURES.len(), 65);
    let mut failures = Vec::new();
    for relative in ORDINARY_ARTICULATED_CREATURES {
        let result = NifFile::load(fixture(&format!("../../../{relative}")))
            .map_err(|error| error.to_string())
            .and_then(|nif| {
                extract_fnv_fo3_creature_ragdoll(&nif).map_err(|error| error.to_string())
            })
            .and_then(|ir| {
                assert_filter_suppresses_exact_constraint_pairs(&ir);
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

#[test]
fn special_fnv_fo3_articulated_corpus_has_complete_fo4_pack_closure() {
    assert_eq!(SPECIAL_ARTICULATED_CREATURES.len(), 10);
    let mut class_totals = std::collections::HashMap::<String, usize>::new();
    for relative in SPECIAL_ARTICULATED_CREATURES {
        let nif = NifFile::load(fixture(&format!("../../../{relative}")))
            .unwrap_or_else(|error| panic!("load {relative}: {error}"));
        let ir = extract_fnv_fo3_creature_ragdoll(&nif)
            .unwrap_or_else(|error| panic!("extract {relative}: {error}"));
        assert_filter_suppresses_exact_constraint_pairs(&ir);
        let lowered = lower_primitive_shapes_to_fo4_convex_hulls(&ir, 12)
            .unwrap_or_else(|error| panic!("convexify {relative}: {error}"));
        let packed = reconstruct_fo4_creature_ragdoll_packfile(&lowered)
            .unwrap_or_else(|error| panic!("pack {relative}: {error}"));
        let reread =
            HkxFile::read(&packed).unwrap_or_else(|error| panic!("reread {relative}: {error}"));
        for class_name in [
            "hkpHingeConstraintData",
            "hkpPrismaticConstraintData",
            "hknpBreakableConstraintData",
            "hkpListShape",
        ] {
            *class_totals.entry(class_name.to_string()).or_default() +=
                class_count(&reread, class_name);
        }
    }
    assert_eq!(class_totals["hkpHingeConstraintData"], 6);
    assert_eq!(class_totals["hkpPrismaticConstraintData"], 4);
    assert_eq!(class_totals["hknpBreakableConstraintData"], 2);
    assert_eq!(class_totals["hkpListShape"], 2);
}
