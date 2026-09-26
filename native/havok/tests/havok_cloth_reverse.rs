// Reverse: runtime → setup.

use havok_native::cloth::ClothData;
use havok_native::cloth::reverse::reverse_cloth_data;
use havok_native::hkx::types::HkxValue;
use havok_native::hkx::{HkxFile, HkxMember, HkxObject};

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
