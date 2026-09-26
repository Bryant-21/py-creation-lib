use havok_native::cloth::runtime::ClothData;
use havok_native::hkx::types::HkxValue;
use havok_native::hkx::{HkxFile, HkxMember, HkxObject};

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------

/// Build a minimal HkxFile containing one hclClothData with a "name" string
/// member and an empty "simClothDatas" array member, to exercise
/// ClothData::from_hkx_file without a real packfile.
fn minimal_cloth_hkx_file() -> HkxFile {
    let cloth_object = HkxObject {
        name: Some("#0001".to_string()),
        offset: 0,
        signature: 0,
        class_name: "hclClothData".to_string(),
        members: vec![
            HkxMember {
                name: "name".to_string(),
                value: HkxValue::String {
                    value: "TestCloth".to_string(),
                    is_null: false,
                },
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
                value: HkxValue::Array(vec![]),
            },
            HkxMember {
                name: "transformSetDefinitions".to_string(),
                value: HkxValue::Array(vec![]),
            },
        ],
    };

    HkxFile::from_tagxml(11, "hk_2014.1.0-r1", vec![cloth_object])
}

#[test]
fn synthetic_cloth_data_found_and_name_matches() {
    let file = minimal_cloth_hkx_file();
    let cloth = ClothData::from_hkx_file(&file)
        .expect("from_hkx_file must find hclClothData in synthetic file");

    assert_eq!(cloth.name(), "TestCloth");
    assert_eq!(cloth.sim_cloth_datas().len(), 0);
    assert_eq!(cloth.operators().len(), 0);
}
