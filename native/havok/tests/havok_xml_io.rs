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

// ---------------------------------------------------------------------------
// Gate 1: HKX → XML → HKX parse equivalence
// ---------------------------------------------------------------------------

#[test]
fn hkx_to_xml_returns_valid_tagxml_string() {
    let hkx_bytes = fixture_bytes("native/havok/tests/fixtures/skeleton.hkx");

    let xml = havok_hkx_to_xml(&hkx_bytes).expect("hkx_to_xml should succeed");

    assert!(
        xml.contains("<hkpackfile"),
        "output must be TagXML with hkpackfile root"
    );
    assert!(xml.contains("<hksection"), "output must contain hksection");
    assert!(
        xml.contains("<hkobject"),
        "output must contain at least one hkobject"
    );
}

#[test]
fn hkx_to_xml_to_hkx_parse_equivalent() {
    let hkx_bytes = fixture_bytes("native/havok/tests/fixtures/skeleton.hkx");

    let original = read_packfile(&hkx_bytes).expect("parse original HKX");
    let xml = havok_hkx_to_xml(&hkx_bytes).expect("hkx_to_xml should succeed");
    let roundtrip_bytes = havok_xml_to_hkx(&xml).expect("xml_to_hkx should succeed");
    let roundtrip = read_packfile(&roundtrip_bytes).expect("parse round-tripped HKX");

    assert_eq!(
        roundtrip.contents_version(),
        original.contents_version(),
        "contents_version must survive round-trip"
    );
    assert_eq!(
        roundtrip.objects().len(),
        original.objects().len(),
        "object count must survive round-trip"
    );

    // Class names must be preserved for all objects.
    for (i, (orig_obj, rt_obj)) in original
        .objects()
        .iter()
        .zip(roundtrip.objects())
        .enumerate()
    {
        assert_eq!(
            rt_obj.class_name, orig_obj.class_name,
            "object[{i}] class_name mismatch"
        );
    }
}

// ---------------------------------------------------------------------------
// Gate 2: XML → HKX → XML parse equivalence
// ---------------------------------------------------------------------------

#[test]
fn xml_to_hkx_produces_valid_packfile_magic() {
    let xml = fixture_str("native/havok/tests/fixtures/skeleton.xml");

    let hkx_bytes = havok_xml_to_hkx(&xml).expect("xml_to_hkx should succeed");

    // Packfile magic: first 8 bytes 57 E0 E0 57 10 C0 C0 10
    const PACKFILE_MAGIC: &[u8; 8] = b"\x57\xE0\xE0\x57\x10\xC0\xC0\x10";
    assert!(
        hkx_bytes.len() >= 8 && &hkx_bytes[0..8] == PACKFILE_MAGIC,
        "xml_to_hkx must produce a valid packfile (magic check failed)"
    );
}

#[test]
fn xml_to_hkx_to_xml_parse_equivalent() {
    let xml = fixture_str("native/havok/tests/fixtures/skeleton.xml");

    let original = read_tagxml_string(&xml).expect("parse original XML");
    let hkx_bytes = havok_xml_to_hkx(&xml).expect("xml_to_hkx should succeed");
    let roundtrip_xml = havok_hkx_to_xml(&hkx_bytes).expect("hkx_to_xml should succeed");
    let roundtrip = read_tagxml_string(&roundtrip_xml).expect("parse round-tripped XML");

    assert_eq!(
        roundtrip.contents_version(),
        original.contents_version(),
        "contents_version must survive round-trip"
    );
    assert_eq!(
        roundtrip.objects().len(),
        original.objects().len(),
        "object count must survive round-trip"
    );
    for (i, (orig_obj, rt_obj)) in original
        .objects()
        .iter()
        .zip(roundtrip.objects())
        .enumerate()
    {
        assert_eq!(
            rt_obj.class_name, orig_obj.class_name,
            "object[{i}] class_name mismatch"
        );
    }
}

// ---------------------------------------------------------------------------
// Error cases
// ---------------------------------------------------------------------------

#[test]
fn hkx_to_xml_rejects_garbage_bytes() {
    let error = havok_hkx_to_xml(b"not a havok file").expect_err("should reject garbage");
    assert!(
        !error.to_string().is_empty(),
        "error message must be non-empty"
    );
}

#[test]
fn xml_to_hkx_rejects_malformed_xml() {
    let error =
        havok_xml_to_hkx("<broken>no closing tag").expect_err("should reject malformed XML");
    assert!(
        !error.to_string().is_empty(),
        "error message must be non-empty"
    );
}
