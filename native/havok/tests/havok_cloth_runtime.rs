use std::path::PathBuf;

use havok_native::cloth::runtime::ClothData;
use havok_native::hkx::types::HkxValue;
use havok_native::hkx::{HkxFile, HkxMember, HkxObject};

fn repo_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

fn bathrobe_fixture(relative: &str) -> Option<Vec<u8>> {
    let path = repo_path(relative);
    if !path.exists() {
        return None;
    }
    Some(std::fs::read(&path).unwrap_or_else(|e| {
        panic!("failed to read fixture {relative}: {e}");
    }))
}

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

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------

fn load_bathrobe_hkx() -> Option<HkxFile> {
    let blob = bathrobe_fixture("../tests/fixtures/cloth/bathrobe_outfitm_cloth.hkx")?;
    let hkx = havok_native::cloth::load_cloth_hkx(&blob)
        .expect("load_cloth_hkx must parse bathrobe cloth blob");
    Some(hkx)
}

#[test]
fn bathrobe_cloth_data_has_sim_cloth_datas_and_operators() {
    // Skip when bathrobe fixture is not present on this machine.
    let hkx = match load_bathrobe_hkx() {
        Some(f) => f,
        None => return,
    };

    let cloth = ClothData::from_hkx_file(&hkx).expect("bathrobe HKX must contain hclClothData");

    // Name is a non-empty string.
    assert!(
        !cloth.name().is_empty(),
        "bathrobe cloth name must be non-empty"
    );
    // Bathrobe has at least one sim cloth data.
    assert!(
        cloth.sim_cloth_datas().len() >= 1,
        "bathrobe must have >= 1 simClothData, got {}",
        cloth.sim_cloth_datas().len()
    );
    // Bathrobe has at least one operator.
    assert!(
        cloth.operators().len() >= 1,
        "bathrobe must have >= 1 operator, got {}",
        cloth.operators().len()
    );
}

#[test]
fn bathrobe_sim_cloth_data_has_fixed_particle_indices() {
    // Skip when bathrobe fixture is not present on this machine.
    let hkx = match load_bathrobe_hkx() {
        Some(f) => f,
        None => return,
    };

    let cloth = ClothData::from_hkx_file(&hkx).expect("bathrobe HKX must contain hclClothData");

    let sims = cloth.sim_cloth_datas();
    assert!(
        !sims.is_empty(),
        "bathrobe must have at least one SimClothData"
    );

    let first_sim = &sims[0];
    let indices = first_sim.fixed_particle_indices();
    assert!(
        !indices.is_empty(),
        "bathrobe first SimClothData must have >= 1 fixed particle index, got 0"
    );
}
