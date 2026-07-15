use std::fs;
use std::path::PathBuf;

use nif_core_native::io::NifReader;
use nif_core_native::schema::NifSchema;

fn fixture_candidates() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Ok(p) = std::env::var("FO4_TEST_NIF") {
        v.push(PathBuf::from(p));
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    v.push(root.join("mods/B21_ArmCo/data/Meshes/ArmCo/AmmoGenerator.nif"));
    v.push(root.join("mods/B21_ArmCo/data/Meshes/ArmCo/AmmoConverter.nif"));
    v
}

fn fixture_path() -> Option<PathBuf> {
    fixture_candidates().into_iter().find(|p| p.exists())
}

#[test]
fn fo4_ammogenerator_header_matches_python_ground_truth() {
    let Some(path) = fixture_path() else {
        eprintln!("skip: no FO4 test NIF available");
        return;
    };
    let bytes = fs::read(&path).expect("read nif bytes");
    let schema = NifSchema::from_generated();
    let nif = NifReader::read(&bytes, &schema).expect("rust reader ok");

    assert_eq!(nif.header.version, (20, 2, 0, 7));
    assert_eq!(nif.header.version_packed, 0x14020007);
    assert_eq!(nif.header.user_version, 12);
    assert_eq!(nif.header.bs_version, 130);
    assert_eq!(nif.header.num_blocks, 45);
    assert_eq!(nif.header.block_type_names.len(), 15);

    let expected_names = [
        "NiNode",
        "BSXFlags",
        "NiControllerManager",
        "NiMultiTargetTransformController",
        "NiControllerSequence",
        "NiTransformInterpolator",
        "NiTransformData",
        "NiTextKeyExtraData",
        "NiDefaultAVObjectPalette",
        "BSTriShape",
        "BSLightingShaderProperty",
        "BSShaderTextureSet",
        "bhkNPCollisionObject",
        "bhkPhysicsSystem",
        "BSConnectPoint::Parents",
    ];
    for (i, name) in expected_names.iter().enumerate() {
        assert_eq!(
            nif.header.block_type_names[i], *name,
            "block_type_names[{}] mismatch",
            i
        );
    }

    assert_eq!(nif.blocks.len(), 45);
    let expected_block_types = [
        "NiNode",
        "NiNode",
        "BSXFlags",
        "NiControllerManager",
        "NiMultiTargetTransformController",
    ];
    for (i, t) in expected_block_types.iter().enumerate() {
        assert_eq!(
            nif.blocks[i].type_name, *t,
            "blocks[{}].type_name mismatch",
            i
        );
        assert_eq!(nif.blocks[i].block_id, i);
    }
}
