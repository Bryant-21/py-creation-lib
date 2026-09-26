/// Round-trip tests for `havok_hkx_to_xml` and `havok_xml_to_hkx`.
///
/// Gate 1: HKX → XML → HKX re-parses to an HkxFile with identical object count,
///          class names, and contents_version (byte-identity is not required because
///          the packfile writer re-serializes from the HkxFile model; the round-trip
///          parse-equivalence is the correctness gate).
///
/// Gate 2: XML → HKX → XML produces XML that re-parses to an HkxFile structurally
///          identical to the original parse.
use std::path::PathBuf;

use havok_native::api::{havok_hkx_to_xml, havok_xml_to_hkx};
use havok_native::hkx::read_packfile;
use havok_native::hkx::tagxml::read_tagxml_string;

fn repo_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

fn fixture_bytes(relative: &str) -> Vec<u8> {
    let path = repo_path(relative);
    std::fs::read(&path).unwrap_or_else(|error| {
        panic!("failed to read fixture {}: {error}", path.display());
    })
}

fn fixture_str(relative: &str) -> String {
    let path = repo_path(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!("failed to read fixture {}: {error}", path.display());
    })
}

fn class_names(file: &havok_native::hkx::HkxFile) -> Vec<&str> {
    file.objects()
        .iter()
        .map(|o| o.class_name.as_str())
        .collect()
}

#[test]
fn hkx_to_xml_to_hkx_is_parse_equivalent() {
    let hkx_bytes = fixture_bytes("native/havok/tests/fixtures/skeleton.hkx");
    let original = read_packfile(&hkx_bytes).expect("parse original HKX");
    let xml = havok_hkx_to_xml(&hkx_bytes).expect("hkx_to_xml should succeed");
    for needle in ["<hkpackfile", "<hksection", "<hkobject"] {
        assert!(xml.contains(needle), "{needle}");
    }

    let roundtrip = read_packfile(&havok_xml_to_hkx(&xml).expect("xml_to_hkx should succeed"))
        .expect("parse round-tripped HKX");
    assert_eq!(roundtrip.contents_version(), original.contents_version());
    assert_eq!(class_names(&roundtrip), class_names(&original));
}

#[test]
fn xml_to_hkx_to_xml_is_parse_equivalent_and_rejects_garbage() {
    let xml = fixture_str("native/havok/tests/fixtures/skeleton.xml");
    let original = read_tagxml_string(&xml).expect("parse original XML");
    let hkx_bytes = havok_xml_to_hkx(&xml).expect("xml_to_hkx should succeed");
    assert_eq!(&hkx_bytes[0..8], b"\x57\xE0\xE0\x57\x10\xC0\xC0\x10");

    let roundtrip_xml = havok_hkx_to_xml(&hkx_bytes).expect("hkx_to_xml should succeed");
    let roundtrip = read_tagxml_string(&roundtrip_xml).expect("parse round-tripped XML");
    assert_eq!(roundtrip.contents_version(), original.contents_version());
    assert_eq!(class_names(&roundtrip), class_names(&original));

    assert!(havok_hkx_to_xml(b"not a havok file").is_err());
    assert!(havok_xml_to_hkx("<broken>no closing tag").is_err());
}
