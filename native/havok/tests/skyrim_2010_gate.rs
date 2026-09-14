use std::path::{Path, PathBuf};

use havok_native::api;
use havok_native::convert::PatchManager;
use havok_native::error::HavokError;
use havok_native::hkx::HkxFile;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

#[test]
fn skyrim_wolf_runtime_packfiles_decode_or_report_precise_legacy_blockers() {
    let canine = repo_root().join("extracted/skyrimse/meshes/actors/canine");
    if !canine.exists() {
        return;
    }

    let cases = [
        ("wolfproject.hkx", "hkRootLevelContainer"),
        ("character assets wolf/skeleton.hkx", "hkaSkeleton"),
        ("behaviors wolf/wolfbehavior.hkx", "hkbBehaviorGraph"),
        (
            "animations/mt_idle_wolf.hkx",
            "hkaSplineCompressedAnimation",
        ),
        ("characters wolf/wolf.hkx", "hkbCharacterData"),
    ];

    for (relative, expected_class) in cases {
        let path = canine.join(Path::new(relative));
        let bytes = std::fs::read(&path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        let hkx = HkxFile::read(&bytes)
            .unwrap_or_else(|error| panic!("failed to decode {relative}: {error}"));

        assert_eq!(hkx.class_version(), 8, "{relative}");
        assert_eq!(hkx.contents_version(), "hk_2010.2.0-r1", "{relative}");
        assert!(
            hkx.objects()
                .iter()
                .any(|object| object.class_name == expected_class),
            "{relative} did not contain {expected_class}"
        );
        let xml = api::havok_hkx_to_xml(&bytes)
            .unwrap_or_else(|error| panic!("failed to export {relative} as XML: {error}"));
        assert!(xml.contains("contentsversion=\"hk_2010.2.0-r1\""));
        assert!(xml.contains(&format!("class=\"{expected_class}\"")));
        assert_eq!(hkx.save(), bytes, "{relative} changed without a model edit");
    }
}

#[test]
fn skyrim_creature_v40_layout_regressions_read_xml_and_roundtrip() {
    let meshes = repo_root().join("extracted/skyrimse/meshes");
    if !meshes.exists() {
        return;
    }
    let cases = [
        (
            "actors/cow/characters/h_cowcharater.hkx",
            "hkbCharacterData",
        ),
        (
            "actors/dlc02/hmdaedra/character assets/skeleton.hkx",
            "hkaSkeletonMapper",
        ),
        (
            "actors/canine/behaviors wolf/quadrupedbehavior.hkx",
            "BSLookAtModifier",
        ),
        (
            "actors/dragonpriest/behaviors/dragon_priest.hkx",
            "BSLookAtModifier",
        ),
        (
            "actors/atronachfrost/behaviors/atronachfrostbehavior.hkx",
            "hkbFootIkControlsModifier",
        ),
        (
            "actors/dragon/behaviors/dragonbehavior.hkx",
            "hkbFootIkModifier",
        ),
        (
            "actors/ambient/hare/behaviors/harebehavior.hkx",
            "hkbPoseMatchingGenerator",
        ),
        (
            "actors/mudcrab/behaviors/mudcrabbehavior.hkx",
            "hkbModifierList",
        ),
    ];

    for (relative, expected_class) in cases {
        let path = meshes.join(relative);
        let bytes = std::fs::read(&path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        let hkx = HkxFile::read(&bytes)
            .unwrap_or_else(|error| panic!("failed to decode {relative}: {error}"));
        assert!(
            hkx.objects()
                .iter()
                .any(|object| object.class_name == expected_class),
            "{relative} did not contain {expected_class}"
        );
        api::havok_hkx_to_xml(&bytes)
            .unwrap_or_else(|error| panic!("failed to export {relative}: {error}"));
        assert_eq!(hkx.save(), bytes, "{relative} changed without a model edit");
    }
}

#[test]
fn skyrim_2010_to_fo4_conversion_remains_behind_the_patch_corpus_gate() {
    let path = repo_root().join("extracted/skyrimse/meshes/actors/canine/wolfproject.hkx");
    if !path.exists() {
        return;
    }

    let bytes = std::fs::read(&path).expect("read wolf project");
    let mut hkx = HkxFile::read(&bytes).expect("decode wolf project");
    let manager = PatchManager::with_native_corpus();
    for version_id in 41..=45 {
        assert_eq!(manager.patch_count(version_id), 0, "version {version_id}");
    }
    assert!(manager.patch_count(46) > 0);

    let error = manager
        .convert_hkx(&mut hkx, 40, 53)
        .expect_err("40 -> 53 must remain gated until native patch parity exists");

    assert!(matches!(
        error,
        HavokError::UnportedEdgeCase { ref edge_case, .. }
            if edge_case == "native patch corpus parity"
    ));
}

#[test]
fn skyrim_wolf_character_preserves_the_legacy_controller_and_model_basis() {
    let path = repo_root().join("extracted/skyrimse/meshes/actors/canine/characters wolf/wolf.hkx");
    if !path.exists() {
        return;
    }
    let bytes = std::fs::read(&path).expect("read wolf character");
    let hkx = HkxFile::read(&bytes).expect("decode wolf character");
    let character = hkx
        .objects()
        .iter()
        .find(|object| object.class_name == "hkbCharacterData")
        .expect("character data");
    assert_eq!(character.signature, 0x300d_6808);
    let controller = character
        .members
        .iter()
        .find(|member| member.name == "characterControllerInfo")
        .expect("legacy controller info");
    let havok_native::hkx::types::HkxValue::TypedObject {
        class_name,
        members,
    } = &controller.value
    else {
        panic!("legacy controller must carry its inline class");
    };
    assert_eq!(class_name, "hkbCharacterDataCharacterControllerInfo");
    assert_eq!(
        members[0].value,
        havok_native::hkx::types::HkxValue::F32(1.7)
    );
    assert_eq!(
        members[1].value,
        havok_native::hkx::types::HkxValue::F32(0.4)
    );
    assert_eq!(members[2].value, havok_native::hkx::types::HkxValue::U32(1));
    assert_eq!(
        members[3].value,
        havok_native::hkx::types::HkxValue::Pointer(None)
    );
    let member = |name| {
        &character
            .members
            .iter()
            .find(|member| member.name == name)
            .unwrap_or_else(|| panic!("missing {name}"))
            .value
    };
    assert_eq!(
        member("modelUpMS"),
        &havok_native::hkx::types::HkxValue::F32List(vec![0.0, 0.0, 1.0, 0.0])
    );
    assert_eq!(
        member("modelForwardMS"),
        &havok_native::hkx::types::HkxValue::F32List(vec![0.0, 1.0, 0.0, 0.0])
    );
    assert_eq!(
        member("modelRightMS"),
        &havok_native::hkx::types::HkxValue::F32List(vec![1.0, -0.0, -0.0, 0.0])
    );
    assert_eq!(
        member("scale"),
        &havok_native::hkx::types::HkxValue::F32(1.0)
    );
    assert_eq!(hkx.save(), bytes);
}
