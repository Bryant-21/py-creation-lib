use std::path::PathBuf;

use havok_native::api;
use havok_native::hkx::packfile::parse_packfile;
use havok_native::hkx::types::HkxValue;
use havok_native::hkx::{HkxFile, HkxMember, HkxObject, read_packfile};

const HKX_MAGIC: &[u8; 8] = b"\x57\xE0\xE0\x57\x10\xC0\xC0\x10";

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

fn fo4_fixture_paths() -> [&'static str; 2] {
    [
        "native/havok/tests/fixtures/skeleton.hkx",
        "../bacup/py_bacup_lib/python/bacup_lib/tests/fixtures/creatures/deathclaw/expected/character.hkx",
    ]
}

fn write_u32(data: &mut [u8], offset: usize, value: u32) {
    data[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

struct SectionHeaderFields {
    name: &'static str,
    offset: u32,
    data1: u32,
    data2: u32,
    data3: u32,
    exports: u32,
    imports: u32,
    end: u32,
}

fn write_section_header(data: &mut [u8], base: usize, fields: SectionHeaderFields) {
    data[base..base + fields.name.len()].copy_from_slice(fields.name.as_bytes());
    write_u32(data, base + 0x14, fields.offset);
    write_u32(data, base + 0x18, fields.data1);
    write_u32(data, base + 0x1C, fields.data2);
    write_u32(data, base + 0x20, fields.data3);
    write_u32(data, base + 0x24, fields.exports);
    write_u32(data, base + 0x28, fields.imports);
    write_u32(data, base + 0x2C, fields.end);
}

fn assert_monotonic_section_ranges(
    relative: &str,
    parsed: &havok_native::hkx::packfile::ParsedPackfile,
) {
    for section in &parsed.sections {
        assert!(
            section.offset <= section.data1
                && section.data1 <= section.data2
                && section.data2 <= section.data3
                && section.data3 <= section.exports
                && section.exports <= section.imports
                && section.imports <= section.end,
            "{relative} section {} should have monotonic ranges",
            section.name
        );
    }
}

fn synthetic_v8_packfile() -> Vec<u8> {
    let mut data = vec![0; 0x200];
    data[0..8].copy_from_slice(HKX_MAGIC);
    write_u32(&mut data, 0x0C, 8);
    data[0x10] = 8;
    data[0x28..0x28 + 14].copy_from_slice(b"Havok-5.5.0-r1");

    write_section_header(
        &mut data,
        0x40,
        SectionHeaderFields {
            name: "__classnames__",
            offset: 0xD0,
            data1: 0x20,
            data2: 0x20,
            data3: 0x20,
            exports: 0x20,
            imports: 0x20,
            end: 0x20,
        },
    );
    write_section_header(
        &mut data,
        0x70,
        SectionHeaderFields {
            name: "__types__",
            offset: 0x110,
            data1: 0,
            data2: 0,
            data3: 0,
            exports: 0,
            imports: 0,
            end: 0,
        },
    );
    write_section_header(
        &mut data,
        0xA0,
        SectionHeaderFields {
            name: "__data__",
            offset: 0x140,
            data1: 0,
            data2: 0,
            data3: 0,
            exports: 0x0C,
            imports: 0x0C,
            end: 0x0C,
        },
    );

    write_u32(&mut data, 0xD0, 0x12345678);
    data[0xD4] = 0x09;
    data[0xD5..0xD5 + 20].copy_from_slice(b"hkRootLevelContainer");

    write_u32(&mut data, 0x140, 0);
    write_u32(&mut data, 0x144, 0);
    write_u32(&mut data, 0x148, 0);

    data
}

#[test]
fn fo4_fixtures_parse_and_round_trip_byte_exact() {
    for relative in fo4_fixture_paths() {
        let data = fixture_bytes(relative);
        let parsed = parse_packfile(&data).unwrap_or_else(|error| {
            panic!("failed to parse {relative}: {error}");
        });
        assert_eq!(parsed.header.version, 11, "{relative}");
        assert_eq!(parsed.header.pointer_size, 8, "{relative}");
        assert!(parsed.header.version_name.contains("2014"), "{relative}");
        for section in ["__classnames__", "__types__", "__data__"] {
            assert!(parsed.section(section).is_some(), "{relative} {section}");
        }
        assert_monotonic_section_ranges(relative, &parsed);
        assert!(
            parsed
                .classnames
                .iter()
                .any(|entry| entry.name == "hkRootLevelContainer"),
            "{relative}"
        );
        assert!(!parsed.virtual_fixups.is_empty(), "{relative}");

        let hkx = read_packfile(&data).unwrap_or_else(|error| {
            panic!("failed to read {relative}: {error}");
        });
        assert!(!hkx.is_dirty(), "{relative}");
        assert_eq!(hkx.save_unchanged(), data, "{relative}");
        assert_eq!(api::hkx_roundtrip_bytes(&data).unwrap(), data, "{relative}");
    }
}

#[test]
fn v8_layout_is_parsed_and_emitted_with_48_byte_section_headers() {
    let data = synthetic_v8_packfile();
    let parsed = parse_packfile(&data).unwrap();
    assert_eq!(parsed.header.version, 8);
    assert_eq!(parsed.header.section_header_size, 0x30);
    for section in ["__classnames__", "__types__", "__data__"] {
        assert!(parsed.section(section).is_some(), "{section}");
    }
    assert_monotonic_section_ranges("synthetic v8", &parsed);
    assert_eq!(parsed.section("__classnames__").unwrap().offset, 0xD0);
    assert_eq!(parsed.section("__data__").unwrap().imports, 0x14C);

    let hkx = HkxFile::from_tagxml(
        8,
        "Havok-5.5.0-r1",
        vec![HkxObject {
            name: Some("#0001".to_string()),
            offset: 0,
            signature: 0,
            class_name: "hkRootLevelContainer".to_string(),
            members: vec![HkxMember {
                name: "namedVariants".to_string(),
                value: HkxValue::Array(vec![]),
            }],
        }],
    );
    let parsed = parse_packfile(&hkx.save()).expect("v8 output must be parseable");
    assert_eq!(parsed.header.version, 8);
    assert_eq!(parsed.header.section_header_size, 0x30);
    assert_eq!(parsed.section("__classnames__").unwrap().offset & 0xF, 0);
}

#[test]
fn malformed_packfiles_are_rejected_instead_of_echoed() {
    let mut missing_types = synthetic_v8_packfile();
    missing_types[0x70..0x70 + 11].copy_from_slice(b"__objects__");
    let error = parse_packfile(&missing_types).unwrap_err();
    assert!(
        error.to_string().contains("missing __types__ section"),
        "{error}"
    );

    let error = parse_packfile(b"\x57\xE0\xE0\x57").unwrap_err();
    assert!(error.to_string().contains("packfile header"), "{error}");

    let error = api::hkx_roundtrip_bytes(HKX_MAGIC).unwrap_err();
    assert!(error.to_string().contains("packfile header"), "{error}");
}
