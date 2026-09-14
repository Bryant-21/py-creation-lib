// Integration test for the `model_info` MODT codec against the REAL
// generated schema, as opposed to authoring_serialize.rs's
// `model_info_roundtrips_*` unit tests, which use the same fixture bytes but
// a hand-built `SchemaSubrecordJson`. Each test prints a reason and skips if
// the generated schema doesn't report codec=`model_info` for MODT.
//
// Fixtures are real MODT bytes, not synthesized:
//   - `048280:Fallout4.esm` (STAT MetalBarrel01Fire01_Static) via
//     `modkit esp get-record --authoring`.
//   - `13CB50:Fallout4.esm` (TERM DN035_RobotControlTerminal, sourced from
//     DLCRobot.esm) via `modkit esp export --mode lossless`: the only MODT
//     among 2326 scanned with a non-empty addon_nodes array and multiple
//     materials.

use esp_authoring_core::plugin_runtime::authoring::authoring_serialize::compact_model_info_payload_json;
use esp_authoring_core::plugin_runtime::{compiled_schema_for_game_str, encode_model_info_json};

fn assert_model_info_roundtrips(record_signature: &str, hex: &str) {
    let schema = compiled_schema_for_game_str("fo4").expect("fo4 schema must load");
    let record_def = schema
        .record_def(record_signature)
        .unwrap_or_else(|| panic!("{record_signature} must be defined in the fo4 schema"));
    let spec = record_def
        .subrecords
        .iter()
        .find(|sub| sub.id == "MODT")
        .unwrap_or_else(|| panic!("{record_signature} must declare a MODT subrecord"))
        .clone();

    if spec.codec.as_deref() != Some("model_info") {
        eprintln!(
            "skipping modt_roundtrip ({record_signature}): MODT still codec={:?}, \
             pending schema_forge regen",
            spec.codec
        );
        return;
    }

    let data = hex::decode(hex).expect("valid hex fixture");
    let decoded = compact_model_info_payload_json(&data, &spec, &schema, &[], "Fallout4.esm")
        .unwrap_or_else(|| panic!("{record_signature} MODT should decode through the real schema"));
    let mapping = decoded.as_object().expect("decoded object").clone();
    let encoded = encode_model_info_json(&spec, &mapping, "MODT")
        .expect("model_info should re-encode through the real schema");
    assert_eq!(
        encoded, data,
        "byte-exact round-trip failed for {record_signature} MODT"
    );
}

#[test]
fn modt_roundtrips_stat_metalbarrel01fire01() {
    // 19 textures (ext "dds"), 0 addon nodes, srgb_count=15, 1 material
    // (ext "bgsm").
    assert_model_info_roundtrips(
        "STAT",
        "0400000013000000000000000F0000000100000075DB5AEF6464730038973CEAB3AEEF3B\
         646473007A7C3A5ADAE0E40B646473000BD80002038CE268646473000BD800020717F56F\
         6464730038973CEA2D0D94F664647300582C55331D653788646473000BD80002B92277AE\
         646473001CDB88C54D1A1046646473001CDB88C5528FF9866464730038973CEA29F70F00\
         64647300582C5533AD473ADB646473007A7C3A5A791608CF646473000786F88DF6E39FC7\
         64647300582C5533F4441C8564647300582C553332A999016464730038973CEA7F44DDF8\
         64647300582C553362F091FC6464730038973CEAE8C1B2DE6464730038973CEA7FFC0CAC\
         6267736DC23D6406",
    );
}

#[test]
fn modt_roundtrips_term_dn035_robotcontrolterminal() {
    // 21 textures, 1 addon node, srgb_count=13, 3 materials (ext "bgsm").
    assert_model_info_roundtrips(
        "TERM",
        "0400000015000000010000000D000000030000004B25D0F3646473008FBEDB9F9249D690\
         646473008FBEDB9F7C4F12F2646473008FBEDB9FA5231491646473008FBEDB9F25F154F0\
         646473008FBEDB9FFC9D5293646473008FBEDB9F77F33006646473007B24D06CBF79ECA7\
         64647300BE643C4C3C60073E646473008FBEDB9F9308AE366464730038973CEA24389F9F\
         646473001CDB88C5B076E385646473007B24D06CFF6512FD6464730038973CEA7EEF02FC\
         64647300582C5533C80FD0FC6464730038973CEAE2748773646473008FBEDB9FD000F877\
         646473001CDB88C5BBCAC171646473008FBEDB9F8CA00370646473008FBEDB9F62F091FC\
         6464730038973CEAD1A562886464730038973CEAC3000000E0FECFBC6267736D12D53ECB\
         5AAFC6256267736D12D53ECBCC9FC1526267736D12D53ECB",
    );
}
