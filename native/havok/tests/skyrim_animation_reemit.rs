use std::path::{Path, PathBuf};

use havok_native::animation::clip::extract_clip;
use havok_native::animation::parsers::{parse_animation_xml_str, parse_skeleton_xml};
use havok_native::api;
use havok_native::error::HavokError;
use havok_native::hkx::model::{HkxFile, HkxObject};
use havok_native::hkx::types::HkxValue;

const SKYRIM_VERSION: &str = "hk_2010.2.0-r1";
const FO4_VERSION: &str = "hk_2014.1.0-r1";
const ROOT_SIGNATURE: u32 = 0x2772_c11e;
const FO4_ANIMATION_CONTAINER_SIGNATURE: u32 = 0x2685_9f4c;
const FO4_SKELETON_SIGNATURE: u32 = 0xfec1_cedb;
const FO4_SPLINE_SIGNATURE: u32 = 0x8c3b_5f7e;
const FO4_BINDING_SIGNATURE: u32 = 0x0faf_9150;
const FO4_MEMORY_CONTAINER_SIGNATURE: u32 = 0x1de1_3a73;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn canine_root() -> PathBuf {
    repo_root().join("extracted/skyrimse/meshes/actors/canine")
}

fn read(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()))
}

fn assert_class_signature(file: &HkxFile, class_name: &str, expected: u32) {
    let mut found = false;
    for object in file
        .objects()
        .iter()
        .filter(|object| object.class_name == class_name)
    {
        found = true;
        assert_eq!(
            object.signature, expected,
            "unexpected {class_name} signature"
        );
    }
    assert!(found, "missing {class_name}");
}

fn member_value<'a>(file: &'a HkxFile, class_name: &str, member_name: &str) -> &'a HkxValue {
    &file
        .objects()
        .iter()
        .find(|object| object.class_name == class_name)
        .unwrap_or_else(|| panic!("missing {class_name}"))
        .members
        .iter()
        .find(|member| member.name == member_name)
        .unwrap_or_else(|| panic!("missing {class_name}.{member_name}"))
        .value
}

fn assert_fo4_pack_read_xml_roundtrip(bytes: &[u8]) -> String {
    let target = HkxFile::read(bytes).expect("reread emitted FO4 packfile");
    assert_eq!(target.class_version(), 11);
    assert_eq!(target.contents_version(), FO4_VERSION);
    assert_eq!(target.packfile().header.pointer_size, 8);
    assert_eq!(target.packfile().header.section_header_size, 0x40);
    assert_eq!(target.save(), bytes);

    let xml = api::havok_hkx_to_xml(bytes).expect("export emitted packfile to XML");
    assert!(xml.contains("classversion=\"11\""));
    assert!(xml.contains("contentsversion=\"hk_2014.1.0-r1\""));
    let xml_repacked = api::havok_xml_to_hkx(&xml).expect("repack emitted XML");
    let xml_roundtrip = HkxFile::read(&xml_repacked).expect("reread emitted XML packfile");
    assert_eq!(xml_roundtrip.class_version(), 11);
    assert_eq!(xml_roundtrip.contents_version(), FO4_VERSION);
    assert_eq!(xml_roundtrip.packfile().header.pointer_size, 8);
    xml
}

#[test]
fn real_wolf_skeleton_reemits_only_the_50_bone_animation_skeleton() {
    let path = canine_root().join("character assets wolf/skeleton.hkx");
    if !path.exists() {
        return;
    }

    let source = read(&path);
    let source_hkx = HkxFile::read(&source).expect("read source skeleton graph");
    assert!(
        source_hkx
            .objects()
            .iter()
            .any(|object| object.class_name.starts_with("hkp"))
    );
    assert!(source_hkx.objects().iter().any(|object| matches!(
        object.class_name.as_str(),
        "hkaRagdollInstance" | "hkaSkeletonMapper"
    )));
    assert_class_signature(&source_hkx, "hkaAnimationContainer", 0x8dc2_0333);
    assert_class_signature(&source_hkx, "hkaSkeleton", 0x366e_8220);
    let source_xml = api::havok_hkx_to_xml(&source).expect("source skeleton XML");
    let source_skeleton = parse_skeleton_xml(&source_xml).expect("source skeleton semantics");
    let output = api::havok_reemit_skyrim_2010_animation_asset_to_fo4(&source)
        .expect("reemit wolf skeleton");
    let target_hkx = HkxFile::read(&output).expect("read minimal target skeleton graph");
    assert_eq!(
        target_hkx
            .objects()
            .iter()
            .map(|object| object.class_name.as_str())
            .collect::<Vec<_>>(),
        vec![
            "hkRootLevelContainer",
            "hkaAnimationContainer",
            "hkaSkeleton"
        ]
    );
    assert!(target_hkx.objects().iter().all(|object| {
        !object.class_name.starts_with("hkp")
            && !matches!(
                object.class_name.as_str(),
                "hkaRagdollInstance"
                    | "hkaSkeletonMapper"
                    | "hkbMirroredSkeletonInfo"
                    | "hkMemoryResourceContainer"
                    | "hkMemoryResourceHandle"
            )
    }));
    assert_class_signature(&target_hkx, "hkRootLevelContainer", ROOT_SIGNATURE);
    assert_class_signature(
        &target_hkx,
        "hkaAnimationContainer",
        FO4_ANIMATION_CONTAINER_SIGNATURE,
    );
    assert_class_signature(&target_hkx, "hkaSkeleton", FO4_SKELETON_SIGNATURE);

    let vanilla_fixture =
        repo_root().join("py_creation_lib/native/havok/tests/fixtures/skeleton.hkx");
    if vanilla_fixture.exists() {
        let vanilla = HkxFile::read(&read(&vanilla_fixture)).expect("read vanilla FO4 skeleton");
        assert_class_signature(&vanilla, "hkRootLevelContainer", ROOT_SIGNATURE);
        assert_class_signature(
            &vanilla,
            "hkaAnimationContainer",
            FO4_ANIMATION_CONTAINER_SIGNATURE,
        );
        assert_class_signature(&vanilla, "hkaSkeleton", FO4_SKELETON_SIGNATURE);
    }
    let target_xml = assert_fo4_pack_read_xml_roundtrip(&output);
    let target_skeleton = parse_skeleton_xml(&target_xml).expect("target skeleton semantics");

    assert_eq!(source_skeleton.bone_count, 50);
    assert_eq!(target_skeleton.bone_count, 50);

    let vanilla_clip_path =
        repo_root().join("extracted/fo4/meshes/actors/yaoguai/animations/walkforward.hkx");
    if vanilla_clip_path.exists() {
        let vanilla = HkxFile::read(&read(&vanilla_clip_path)).expect("read vanilla FO4 clip");
        assert_class_signature(&vanilla, "hkRootLevelContainer", ROOT_SIGNATURE);
        assert_class_signature(
            &vanilla,
            "hkaAnimationContainer",
            FO4_ANIMATION_CONTAINER_SIGNATURE,
        );
        assert_class_signature(
            &vanilla,
            "hkaSplineCompressedAnimation",
            FO4_SPLINE_SIGNATURE,
        );
        assert_class_signature(&vanilla, "hkaAnimationBinding", FO4_BINDING_SIGNATURE);
        assert_class_signature(
            &vanilla,
            "hkMemoryResourceContainer",
            FO4_MEMORY_CONTAINER_SIGNATURE,
        );
    }
    assert_eq!(target_skeleton.bone_names, source_skeleton.bone_names);
    assert_eq!(
        target_skeleton.parent_indices,
        source_skeleton.parent_indices
    );
    assert_eq!(
        target_skeleton.reference_pose,
        source_skeleton.reference_pose
    );
    assert_eq!(
        target_skeleton.lock_translation,
        source_skeleton.lock_translation
    );
}

#[test]
fn real_wolf_clips_reemit_tracks_bindings_duration_and_annotations() {
    let canine = canine_root();
    let animations = canine.join("animations");
    let skeleton_path = canine.join("character assets wolf/skeleton.hkx");
    if !animations.exists() || !skeleton_path.exists() {
        return;
    }

    let target_skeleton_output =
        api::havok_reemit_skyrim_2010_animation_asset_to_fo4(&read(&skeleton_path))
            .expect("reemit binding skeleton");
    let target_skeleton_xml = assert_fo4_pack_read_xml_roundtrip(&target_skeleton_output);
    let target_skeleton =
        parse_skeleton_xml(&target_skeleton_xml).expect("target binding skeleton semantics");
    assert_eq!(target_skeleton.name, "NPC Root [Root]");
    assert_eq!(target_skeleton.bone_count, 50);

    let cases = [
        ("Idle", "mt_idle_wolf.hkx"),
        ("WalkForward", "walkforward_wolf.hkx"),
        ("TurnL90", "turncannedl90.hkx"),
        ("TurnR90", "turncannedr90.hkx"),
        ("Attack1", "attack1.hkx"),
    ];

    for (label, filename) in cases {
        let path = animations.join(filename);
        let source = read(&path);
        let source_hkx = HkxFile::read(&source).expect("read source clip");
        assert_eq!(source_hkx.class_version(), 8, "{label}");
        assert_eq!(source_hkx.contents_version(), SKYRIM_VERSION, "{label}");
        assert_class_signature(&source_hkx, "hkaAnimationContainer", 0x8dc2_0333);
        assert_class_signature(&source_hkx, "hkaSplineCompressedAnimation", 0x792e_e0bb);
        assert_class_signature(&source_hkx, "hkaAnimationBinding", 0x66ea_c971);
        let source_xml = api::havok_hkx_to_xml(&source).expect("source clip XML");
        let source_record = parse_animation_xml_str(&source_xml).expect("source clip semantics");
        let source_clip = extract_clip(&source_xml, None).expect("source clip binding");

        let output = api::havok_reemit_skyrim_2010_animation_asset_to_fo4(&source)
            .unwrap_or_else(|error| panic!("failed to reemit {label}: {error}"));
        let target_hkx = HkxFile::read(&output).expect("read target clip signatures");
        assert_class_signature(&target_hkx, "hkRootLevelContainer", ROOT_SIGNATURE);
        assert_class_signature(
            &target_hkx,
            "hkaAnimationContainer",
            FO4_ANIMATION_CONTAINER_SIGNATURE,
        );
        assert_class_signature(
            &target_hkx,
            "hkaSplineCompressedAnimation",
            FO4_SPLINE_SIGNATURE,
        );
        assert_class_signature(&target_hkx, "hkaAnimationBinding", FO4_BINDING_SIGNATURE);
        assert_class_signature(
            &target_hkx,
            "hkMemoryResourceContainer",
            FO4_MEMORY_CONTAINER_SIGNATURE,
        );
        let target_xml = assert_fo4_pack_read_xml_roundtrip(&output);
        let target_record = parse_animation_xml_str(&target_xml).expect("target clip semantics");
        let target_clip = extract_clip(&target_xml, None).expect("target clip binding");

        assert_eq!(source_record.bone_count, 50, "{label}");
        assert_eq!(target_record.bone_count, 50, "{label}");
        assert_eq!(
            target_record.duration.to_bits(),
            source_record.duration.to_bits(),
            "{label}"
        );
        assert_eq!(
            target_clip.events.len(),
            source_clip.events.len(),
            "{label}"
        );
        for (source, target) in source_clip.events.iter().zip(&target_clip.events) {
            assert_eq!(target.time.to_bits(), source.time.to_bits(), "{label}");
            assert_eq!(target.text, source.text, "{label}");
        }
        assert_eq!(
            target_clip.original_skeleton_name.as_deref(),
            Some(target_skeleton.name.as_str()),
            "{label}"
        );
        assert_eq!(
            target_clip.original_skeleton_name, source_clip.original_skeleton_name,
            "{label}"
        );
        assert_eq!(
            target_clip.track_to_bone_indices, source_clip.track_to_bone_indices,
            "{label}"
        );
        let mapping = &target_clip.track_to_bone_indices;
        assert!(
            mapping.is_empty()
                || (mapping.len() == target_skeleton.bone_count
                    && mapping
                        .iter()
                        .enumerate()
                        .all(|(index, &bone)| index as u32 == bone)),
            "{label} has a non-identity binding: {mapping:?}"
        );
        assert_eq!(
            target_clip.channels.len(),
            target_skeleton.bone_count,
            "{label}"
        );

        if label == "Attack1" {
            for (time, text) in [
                (0.2_f32, "weaponSwing"),
                (0.266667_f32, "preHitFrame"),
                (0.433333_f32, "HitFrame"),
            ] {
                assert!(
                    target_clip.events.iter().any(|event| {
                        event.time.to_bits() == time.to_bits() && event.text == text
                    }),
                    "Attack1 is missing exact annotation {text:?} at {time}"
                );
            }
        }
    }
}

#[test]
fn reemitter_fails_closed_for_non_animation_graphs_and_non_skyrim_sources() {
    let unsupported = HkxFile::from_tagxml(
        8,
        SKYRIM_VERSION,
        vec![HkxObject {
            name: Some("#0001".to_string()),
            offset: 0,
            signature: 0,
            class_name: "hkRootLevelContainer".to_string(),
            members: Vec::new(),
        }],
    )
    .save();
    let error = api::havok_reemit_skyrim_2010_animation_asset_to_fo4(&unsupported).unwrap_err();
    assert!(error.to_string().contains("unsupported source graph"));

    let canine = canine_root();
    if canine.exists() {
        for relative in [
            "wolfproject.hkx",
            "behaviors wolf/wolfbehavior.hkx",
            "characters wolf/wolf.hkx",
        ] {
            let path = canine.join(relative);
            let error =
                api::havok_reemit_skyrim_2010_animation_asset_to_fo4(&read(&path)).unwrap_err();
            assert!(
                error.to_string().contains("unsupported source graph"),
                "{relative}: {error}"
            );
        }
    }

    let fo4_fixture = repo_root().join("py_creation_lib/native/havok/tests/fixtures/skeleton.hkx");
    if fo4_fixture.exists() {
        let error =
            api::havok_reemit_skyrim_2010_animation_asset_to_fo4(&read(&fo4_fixture)).unwrap_err();
        assert!(error.to_string().contains("expected classversion 8"));
    }
}

#[test]
fn creature_skeletons_partial_tracks_root_motion_and_float_tracks_reemit() {
    let meshes = repo_root().join("extracted/skyrimse/meshes");
    if !meshes.exists() {
        return;
    }

    for relative in [
        "actors/dragon/character assets/skeleton.hkx",
        "actors/dlc02/hmdaedra/character assets/skeleton.hkx",
    ] {
        let output =
            api::havok_reemit_skyrim_2010_animation_asset_to_fo4(&read(&meshes.join(relative)))
                .unwrap_or_else(|error| panic!("failed to reemit {relative}: {error}"));
        let target = HkxFile::read(&output).expect("read emitted creature skeleton");
        assert_eq!(target.objects().len(), 3, "{relative}");
        assert!(target.objects().iter().all(|object| matches!(
            object.class_name.as_str(),
            "hkRootLevelContainer" | "hkaAnimationContainer" | "hkaSkeleton"
        )));
        assert_fo4_pack_read_xml_roundtrip(&output);
    }

    for relative in [
        "actors/dragon/animations/mttakeoff45.hkx",
        "actors/dragon/animations/special_perchtowerlaunch.hkx",
        "actors/canine/animations/dog_idle_spice_tail1.hkx",
        "actors/canine/animations/dog_idle_spice_head1.hkx",
        "actors/canine/animations/wolf_face_offset.hkx",
        "actors/dlc02/hmdaedra/animations/special_readenter.hkx",
    ] {
        let source_bytes = read(&meshes.join(relative));
        let source = HkxFile::read(&source_bytes).expect("read source creature clip");
        assert_eq!(
            member_value(&source, "hkaSplineCompressedAnimation", "type"),
            &HkxValue::I32(5),
            "{relative}"
        );
        let source_xml = api::havok_hkx_to_xml(&source_bytes).expect("source XML");
        let source_clip = extract_clip(&source_xml, None).expect("source clip semantics");
        let output = api::havok_reemit_skyrim_2010_animation_asset_to_fo4(&source_bytes)
            .unwrap_or_else(|error| panic!("failed to reemit {relative}: {error}"));
        let target = HkxFile::read(&output).expect("read emitted creature clip");
        assert_eq!(
            member_value(&target, "hkaSplineCompressedAnimation", "type"),
            &HkxValue::I32(3),
            "{relative}"
        );
        let target_xml = assert_fo4_pack_read_xml_roundtrip(&output);
        let target_clip = extract_clip(&target_xml, None).expect("target clip semantics");
        assert_eq!(
            target_clip.track_to_bone_indices, source_clip.track_to_bone_indices,
            "{relative}"
        );
        assert_eq!(
            target_clip.original_skeleton_name, source_clip.original_skeleton_name,
            "{relative}"
        );
        if relative.contains("dog_idle_spice_") {
            let mapping = &target_clip.track_to_bone_indices;
            assert_eq!(mapping.len(), target_clip.channels.len(), "{relative}");
            assert!(mapping.iter().all(|&index| index < 50), "{relative}");
        }
        if relative.contains("dragon/animations") {
            assert_class_signature(&target, "hkaDefaultAnimatedReferenceFrame", 0x60f8_e0b8);
        }
    }
}

#[test]
fn paired_root_clips_are_a_separate_typed_pipeline() {
    let path = repo_root().join(
        "extracted/skyrimse/meshes/actors/sharedkillmoves/1stperson/human&bear/paired_1hmkillmovebeara.hkx",
    );
    if !path.exists() {
        return;
    }
    let error = api::havok_reemit_skyrim_2010_animation_asset_to_fo4(&read(&path)).unwrap_err();
    assert!(matches!(
        error,
        HavokError::UnportedEdgeCase { ref route, ref edge_case, .. }
            if route == "skyrim_2010_animation_to_fo4" && edge_case == "paired_root_binding"
    ));
}
