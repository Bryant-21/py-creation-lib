use std::path::PathBuf;

use havok_native::cloth::reverse::reverse_cloth_data_lossy;
use havok_native::cloth::schema::{expand_from_fixture, is_known};
use havok_native::cloth::{
    ClothData, cloth_metadata_from_blob, load_cloth_hkx, validate_cloth_blob, validate_cloth_data,
};
use havok_native::hkx::read_packfile;

fn repo_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

fn fixture_bytes(relative: &str) -> Vec<u8> {
    std::fs::read(repo_path(relative)).unwrap_or_else(|error| {
        panic!("failed to read fixture {relative}: {error}");
    })
}

#[test]
fn animation_blob_reports_no_cloth_data() {
    let blob = fixture_bytes("native/havok/tests/fixtures/skeleton.hkx");

    let metadata = cloth_metadata_from_blob(&blob).expect("parse cloth metadata scaffold");
    assert!(metadata.object_count > 0);
    assert!(
        metadata
            .class_inventory
            .iter()
            .any(|entry| entry.class_name == "hkRootLevelContainer")
    );
    assert!(!metadata.has_cloth_data);
    assert!(!metadata.has_setup_data);
    assert!(!metadata.has_runtime_data);

    let summary = validate_cloth_blob(&blob).expect("validation should succeed for parseable blob");
    let issues = summary
        .get("issues")
        .and_then(|v| v.as_array())
        .expect("issues array");
    assert!(
        issues
            .iter()
            .any(|i| i.get("code").and_then(|c| c.as_str()) == Some("NO_CLOTH_DATA")),
        "expected NO_CLOTH_DATA error in: {summary:?}",
    );
}

/// End-to-end over the checked-in bathrobe cloth blob: metadata, runtime
/// view, lint, lossy reverse and the inspector JSON.
#[test]
fn bathrobe_fixture_loads_validates_reverses_and_inspects() {
    let blob = fixture_bytes("tests/fixtures/cloth/bathrobe_outfitm_cloth.hkx");

    let metadata = cloth_metadata_from_blob(&blob).expect("parse bathrobe cloth metadata");
    assert!(metadata.has_cloth_data);
    assert!(metadata.has_runtime_data);
    for entry in &metadata.class_inventory {
        assert!(is_known(&entry.class_name), "unknown class {}", entry.class_name);
    }
    let fixture_names =
        expand_from_fixture(&repo_path("tests/fixtures/cloth/bathrobe_outfitm_classnames.json"))
            .expect("read bathrobe classnames json");
    assert!(fixture_names.contains("hclClothData"));
    for name in &fixture_names {
        assert!(is_known(name), "fixture references unknown HCL class {name}");
    }

    let hkx = load_cloth_hkx(&blob).expect("load cloth HKX from bathrobe blob");
    let cloth = ClothData::from_hkx_file(&hkx).expect("bathrobe HKX must contain hclClothData");
    assert!(!cloth.name().is_empty());
    assert!(!cloth.operators().is_empty());
    let sims = cloth.sim_cloth_datas();
    assert!(!sims.is_empty());
    assert!(!sims[0].fixed_particle_indices().is_empty());

    let result = validate_cloth_data(Some(&cloth));
    assert!(
        result.is_valid(),
        "bathrobe cloth data must be lint-clean; errors: {:?}",
        result
            .errors()
            .iter()
            .map(|i| format!("{}: {}", i.code, i.message))
            .collect::<Vec<_>>(),
    );

    let sim_names: Vec<String> = sims.iter().map(|s| s.name().to_string()).collect();
    let setup = reverse_cloth_data_lossy(&cloth);
    assert!(!setup.sim_cloth_setups.is_empty());
    for (sc_setup, runtime_name) in setup.sim_cloth_setups.iter().zip(&sim_names) {
        assert_eq!(&sc_setup.name, runtime_name);
    }

    let json_str = havok_native::api::cloth_inspect_full_json(&blob)
        .expect("cloth_inspect_full_json must succeed on bathrobe");
    let root: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    let particles = root["sim_cloths"][0]["particles"]
        .as_array()
        .expect("bathrobe sim cloth particles");
    assert!(!particles.is_empty());
    for p in particles {
        let pos = p["position"].as_array().expect("particle must have position");
        assert!(pos.len() >= 3);
    }
}

/// A vanilla FO4 cape cloth blob must round-trip byte-exact through
/// `read_packfile` -> writer.
#[test]
fn test_vanilla_cape_roundtrip() {
    let original = fixture_bytes("native/havok/tests/fixtures/cloth/vanilla_cape_outfitm.bin");

    let mut parsed = read_packfile(&original).expect("parse vanilla cape blob");
    // Force the writer path (otherwise save() short-circuits to source bytes).
    let _ = parsed.objects_mut();
    let reemitted = parsed.save();

    if original != reemitted {
        let divergence = original
            .iter()
            .zip(reemitted.iter())
            .position(|(a, b)| a != b);
        let len_msg = format!(
            "len(original)={} len(reemitted)={}",
            original.len(),
            reemitted.len()
        );
        let div_msg = match divergence {
            Some(off) => {
                let lo = off.saturating_sub(8);
                let hi_o = (off + 24).min(original.len());
                let hi_r = (off + 24).min(reemitted.len());
                format!(
                    "first divergence at offset 0x{off:x} ({off}); \
                     original[{lo:#x}..{hi_o:#x}]={:02x?}; \
                     reemitted[{lo:#x}..{hi_r:#x}]={:02x?}",
                    &original[lo..hi_o],
                    &reemitted[lo..hi_r],
                )
            }
            None => format!(
                "no byte differs but len mismatch (truncation at {})",
                original.len().min(reemitted.len())
            ),
        };
        panic!("vanilla cape blob must round-trip byte-exact; {len_msg}; {div_msg}");
    }
}
