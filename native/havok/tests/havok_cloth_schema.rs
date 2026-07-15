use havok_native::cloth::schema::{HAVOK_VERSION, KNOWN_CLASSES, expand_from_fixture, is_known};
use std::path::PathBuf;

fn repo_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

#[test]
fn version_pins_to_fo4_havok_2014() {
    assert_eq!(HAVOK_VERSION, "hk_2014.1.0-r1");
}

#[test]
fn registry_recognises_root_and_cloth_data_classes() {
    assert!(is_known("hkRootLevelContainer"));
    assert!(is_known("hclClothData"));
    assert!(is_known("hclSimClothData"));
    assert!(is_known("hclSimulateOperator"));
    assert!(!is_known("ThisClassDoesNotExist"));
}

#[test]
fn registry_covers_full_sdk_class_enumeration() {
    // KNOWN_CLASSES enumerates every concrete hcl* / hk* class declared under
    // refs/hk2018_1_0_r1/Source/Cloth/ plus the HKX metadata + root container
    // types. Floor at 80 catches accidental truncation regressions; the exact
    // count is intentionally not pinned so extending the enumeration is churn-free.
    assert!(
        KNOWN_CLASSES.len() >= 80,
        "KNOWN_CLASSES shrunk to {} entries — likely regression",
        KNOWN_CLASSES.len()
    );
}

#[test]
fn known_classes_covers_vanilla_cape() {
    // Every class observed in the vanilla cape blob (FO4 OutfitM bathrobe)
    // must classify as known.
    let observed = [
        "hclClothData",
        "hclSimClothData",
        "hclBufferDefinition",
        "hclTransformSetDefinition",
        "hclSimulateOperator",
        "hclStandardLinkConstraintSet",
        "hclStretchLinkConstraintSet",
        "hclBendStiffnessConstraintSet",
        "hclLocalRangeConstraintSet",
        "hclBonePlanesConstraintSet",
        "hclVolumeConstraint",
        "hclCapsuleShape",
        "hclTaperedCapsuleShape",
        "hclCollidable",
    ];
    for c in observed {
        assert!(is_known(c), "class {c} not in KNOWN_CLASSES");
    }
}

#[test]
fn expand_from_fixture_loads_bathrobe_class_names_when_present() {
    let path = repo_path(
        "python/creation_lib/havok_cloth/tests/fixtures/bathrobe_outfitm_classnames.json",
    );
    if !path.exists() {
        return; // skip when fixture not prepared
    }

    let names = expand_from_fixture(&path).expect("read bathrobe classnames json");

    assert!(names.contains("hkRootLevelContainer"));
    assert!(names.contains("hclClothData"));
    // Every name in the fixture should be a known class — otherwise the schema
    // is drifting from the vanilla seed and KNOWN_CLASSES needs an update.
    for name in &names {
        assert!(
            is_known(name),
            "fixture references unknown HCL class {name}; update KNOWN_CLASSES",
        );
    }
}
