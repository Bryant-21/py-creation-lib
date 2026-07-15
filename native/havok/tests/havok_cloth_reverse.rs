// Reverse: runtime → setup integration tests.
//
// Bathrobe-conditional tests early-return when the fixture files are absent
// (they are not tracked in git; only .gitkeep is present).

use std::path::PathBuf;

use havok_native::cloth::ClothData;
use havok_native::cloth::bake::bake_cloth_setup;
use havok_native::cloth::reverse::{reverse_cloth_data, reverse_cloth_data_lossy};
use havok_native::cloth::setup::ClothSetupObject;
use havok_native::hkx::types::HkxValue;
use havok_native::hkx::{HkxFile, HkxMember, HkxObject};

fn repo_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

// ---------------------------------------------------------------------------
// Synthetic HkxFile builder helpers — no disk I/O required
// ---------------------------------------------------------------------------

fn ptr(index: usize) -> HkxValue {
    HkxValue::Pointer(Some(index))
}

fn string_val(s: &str) -> HkxValue {
    HkxValue::String {
        value: s.to_string(),
        is_null: false,
    }
}

fn u32_val(n: u32) -> HkxValue {
    HkxValue::U32(n)
}

fn bool_val(b: bool) -> HkxValue {
    HkxValue::Bool(b)
}

/// Construct a synthetic HkxFile with:
///   object[0]  hclClothData with one buffer def (ptr→[1]) and one transform set def (ptr→[2])
///   object[1]  hclBufferDefinition "Sim", type=6
///   object[2]  hclTransformSetDefinition "Bones", numTransforms=2
fn synthetic_hkx_with_buffer_and_transform_set() -> HkxFile {
    let cloth_obj = HkxObject {
        name: Some("#0000".to_string()),
        offset: 0,
        signature: 0,
        class_name: "hclClothData".to_string(),
        members: vec![
            HkxMember {
                name: "name".to_string(),
                value: string_val(""),
            },
            HkxMember {
                name: "simClothDatas".to_string(),
                value: HkxValue::Array(vec![]),
            },
            HkxMember {
                name: "operators".to_string(),
                value: HkxValue::Array(vec![]),
            },
            HkxMember {
                name: "clothStateDatas".to_string(),
                value: HkxValue::Array(vec![]),
            },
            HkxMember {
                name: "bufferDefinitions".to_string(),
                value: HkxValue::Array(vec![ptr(1)]),
            },
            HkxMember {
                name: "transformSetDefinitions".to_string(),
                value: HkxValue::Array(vec![ptr(2)]),
            },
        ],
    };

    let buf_obj = HkxObject {
        name: Some("#0001".to_string()),
        offset: 1,
        signature: 0,
        class_name: "hclBufferDefinition".to_string(),
        members: vec![
            HkxMember {
                name: "name".to_string(),
                value: string_val("Sim"),
            },
            HkxMember {
                name: "type".to_string(),
                value: u32_val(6),
            },
            HkxMember {
                name: "numVertices".to_string(),
                value: u32_val(0),
            },
            HkxMember {
                name: "numTriangles".to_string(),
                value: u32_val(0),
            },
            HkxMember {
                name: "storeNormals".to_string(),
                value: bool_val(true),
            },
            HkxMember {
                name: "storeTangentsAndBiTangents".to_string(),
                value: bool_val(false),
            },
        ],
    };

    let ts_obj = HkxObject {
        name: Some("#0002".to_string()),
        offset: 2,
        signature: 0,
        class_name: "hclTransformSetDefinition".to_string(),
        members: vec![
            HkxMember {
                name: "name".to_string(),
                value: string_val("Bones"),
            },
            HkxMember {
                name: "numTransforms".to_string(),
                value: u32_val(2),
            },
        ],
    };

    HkxFile::from_tagxml(11, "hk_2014.1.0-r1", vec![cloth_obj, buf_obj, ts_obj])
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------

#[test]
fn reverse_synthetic_cloth_data_returns_setup_with_buffers() {
    let file = synthetic_hkx_with_buffer_and_transform_set();
    let cloth = ClothData::from_hkx_file(&file).expect("from_hkx_file must find hclClothData");

    let setup = reverse_cloth_data(&cloth).expect("reverse synthetic fixture");

    assert_eq!(setup.buffer_setups.len(), 1, "expected 1 buffer setup");
    assert_eq!(
        setup.buffer_setups[0].name, "Sim",
        "buffer name must be 'Sim'"
    );

    // Runtime buffer_type 6 → SIM_CLOTH, stored as the u8 value 2.
    assert_eq!(
        setup.buffer_setups[0].buffer_type, 2,
        "buffer_type 6 → SIM_CLOTH (2)"
    );

    assert_eq!(
        setup.transform_set_setups.len(),
        1,
        "expected 1 transform set setup"
    );
    assert_eq!(
        setup.transform_set_setups[0].name, "Bones",
        "transform set name must be 'Bones'"
    );

    // No sim cloths, operators or states in this minimal fixture
    assert_eq!(setup.sim_cloth_setups.len(), 0);
    assert_eq!(setup.operator_setups.len(), 0);
    assert_eq!(setup.state_setups.len(), 0);
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------

fn load_bathrobe_hkx() -> Option<HkxFile> {
    let path = repo_path("../tests/fixtures/cloth/bathrobe_outfitm_cloth.hkx");
    if !path.exists() {
        // Fixture not present on this machine — skip silently.
        return None;
    }
    let blob = std::fs::read(&path).unwrap_or_else(|e| {
        panic!("failed to read bathrobe fixture: {e}");
    });
    let hkx = havok_native::cloth::load_cloth_hkx(&blob)
        .expect("load_cloth_hkx must parse bathrobe cloth blob");
    Some(hkx)
}

#[test]
fn reverse_bathrobe_cloth_data_when_fixture_present() {
    // Skip when fixture is absent (not in git; only .gitkeep present).
    let hkx = match load_bathrobe_hkx() {
        Some(f) => f,
        None => return,
    };

    let cloth = ClothData::from_hkx_file(&hkx).expect("bathrobe HKX must contain hclClothData");

    // Capture sim cloth names before move
    let sim_names: Vec<String> = cloth
        .sim_cloth_datas()
        .iter()
        .map(|s| s.name().to_string())
        .collect();

    // Real-game blob may contain unknown operator/constraint classes — use
    // the lossy variant so the test exercises the inspect path.
    let setup = reverse_cloth_data_lossy(&cloth);

    // At least one sim cloth setup whose name matches a runtime sim cloth name.
    assert!(
        !setup.sim_cloth_setups.is_empty(),
        "bathrobe reverse must produce at least one sim cloth setup"
    );

    // Every reversed sim cloth name must match the corresponding runtime name.
    for (i, (sc_setup, runtime_name)) in setup
        .sim_cloth_setups
        .iter()
        .zip(sim_names.iter())
        .enumerate()
    {
        assert_eq!(
            sc_setup.name, *runtime_name,
            "sim cloth setup[{i}] name '{}' must match runtime name '{runtime_name}'",
            sc_setup.name
        );
    }
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------

#[test]
fn bake_reverse_min_fixture_round_trips() {
    let src = std::fs::read_to_string("tests/fixtures/cloth_setup_min.json")
        .expect("cloth_setup_min.json fixture missing");
    let original = ClothSetupObject::from_json(&src).expect("from_json failed");

    let hkx = bake_cloth_setup(&original).expect("bake_cloth_setup failed");

    let cloth_data = ClothData::from_hkx_file(&hkx)
        .expect("ClothData::from_hkx_file returned None — baked HKX has no hclClothData");

    let result = reverse_cloth_data(&cloth_data).expect("strict reverse on min fixture");

    assert_eq!(
        result.buffer_setups.len(),
        original.buffer_setups.len(),
        "buffer_setups count must round-trip"
    );
    assert_eq!(
        result.transform_set_setups.len(),
        original.transform_set_setups.len(),
        "transform_set_setups count must round-trip"
    );
    assert_eq!(
        result.sim_cloth_setups.len(),
        original.sim_cloth_setups.len(),
        "sim_cloth_setups count must round-trip"
    );
    // Operator count may diverge when reverse adds default operators not present in the
    // minimal fixture (which has zero operators). Accept >= original count.
    assert!(
        result.operator_setups.len() >= original.operator_setups.len(),
        "reversed operator count ({}) must be >= original ({})",
        result.operator_setups.len(),
        original.operator_setups.len(),
    );
}
