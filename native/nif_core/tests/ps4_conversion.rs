use std::path::PathBuf;

use nif_core_native::model::{NifFile, NifValue};
use nif_core_native::ps4::convert_nif_to_ps4;

fn deathclaw_skeleton() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
        "../../../bacup/py_bacup_lib/python/bacup_lib/tests/fixtures/creatures/deathclaw/expected/skeleton.nif",
    )
}

fn embedded_bytes(value: &NifValue) -> &[u8] {
    let NifValue::Struct(binary) = value else {
        panic!("binary data struct");
    };
    let NifValue::Bytes(bytes) = &binary["Data"] else {
        panic!("binary data bytes");
    };
    bytes
}

#[test]
fn converts_embedded_havok_and_keeps_collision_ids_in_nif_order() {
    let source = deathclaw_skeleton();
    let mut nif = NifFile::load(&source).expect("load FO4 skeleton fixture");

    let changes = convert_nif_to_ps4(&mut nif, &source).expect("convert PS4 NIF");

    assert!(
        changes
            .iter()
            .any(|change| change.contains("embedded Havok")),
        "{changes:?}"
    );
    for system in nif.blocks.iter().filter(|block| {
        matches!(
            block.type_name.as_str(),
            "bhkPhysicsSystem" | "bhkRagdollSystem"
        )
    }) {
        let bytes = embedded_bytes(system.get_field("Binary Data").expect("binary data"));
        let header = havok_native::hkx::packfile::parse_header(bytes).expect("PS4 packfile header");
        assert_eq!(header.reuse_padding_optimization, 1);

        let body_ids = nif
            .blocks
            .iter()
            .filter(|block| block.type_name == "bhkNPCollisionObject")
            .filter(|block| {
                matches!(block.get_field("Data"), Some(NifValue::Ref(id)) if *id == system.block_id as i32)
            })
            .map(|block| block.get_field("Body ID").unwrap().as_usize())
            .collect::<Vec<_>>();
        assert_eq!(body_ids, (0..body_ids.len()).collect::<Vec<_>>());
    }

    let output = tempfile::NamedTempFile::new().expect("temp output");
    nif.save(Some(output.path().to_path_buf()))
        .expect("save PS4 NIF");
    NifFile::load(output.path()).expect("reload PS4 NIF");
}
