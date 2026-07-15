use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use havok_native::hkx::descriptors::DescriptorRegistry;
use havok_native::hkx::packfile::{
    ClassnameEntry, GlobalFixup, LocalFixup, PackfileHeader, ParsedPackfile, SectionHeader,
    VirtualFixup,
};
use havok_native::hkx::read_packfile;
use havok_native::hkx::reader::read_objects_with_registry;
use havok_native::hkx::types::HkxValue;

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

fn fo4_fixture_paths() -> [(&'static str, &'static str); 2] {
    [
        ("native/havok/tests/fixtures/skeleton.hkx", "hkaSkeleton"),
        (
            "../bacup/py_bacup_lib/python/bacup_lib/tests/fixtures/creatures/deathclaw/expected/character.hkx",
            "hkaAnimationContainer",
        ),
    ]
}

fn walk_value(value: &HkxValue, counts: &mut ValueCounts) {
    match value {
        HkxValue::Array(values) => {
            counts.arrays += 1;
            for value in values {
                walk_value(value, counts);
            }
        }
        HkxValue::Object(members) => {
            counts.objects += 1;
            for member in members {
                walk_value(&member.value, counts);
            }
        }
        HkxValue::String { .. } => counts.strings += 1,
        HkxValue::Pointer(_) => counts.pointers += 1,
        _ => {}
    }
}

fn temp_classxml_dir(name: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("havok_native_object_reader_{name}_{stamp}"));
    std::fs::create_dir_all(&dir).expect("create temp classxml dir");
    dir
}

fn write_temp_classxml(dir: &std::path::Path, filename: &str, xml: &str) {
    std::fs::write(dir.join(filename), xml).expect("write temp classxml");
}

fn synthetic_packfile_for_class(class_name: &str, data_len: usize) -> ParsedPackfile {
    ParsedPackfile {
        header: PackfileHeader {
            version: 11,
            version_name: "hk_2014.1.0-r1".to_string(),
            padding_size: 0,
            pointer_size: 8,
            section_header_size: 0x40,
            contents_section_index: 2,
            contents_section_offset: 0,
            contents_class_name_section_index: 0,
            contents_class_name_section_offset: 0,
        },
        sections: vec![
            SectionHeader {
                name: "__classnames__".to_string(),
                offset: data_len,
                data1: data_len,
                data2: data_len,
                data3: data_len,
                exports: data_len,
                imports: data_len,
                end: data_len,
            },
            SectionHeader {
                name: "__types__".to_string(),
                offset: data_len,
                data1: data_len,
                data2: data_len,
                data3: data_len,
                exports: data_len,
                imports: data_len,
                end: data_len,
            },
            SectionHeader {
                name: "__data__".to_string(),
                offset: 0,
                data1: data_len,
                data2: data_len,
                data3: data_len,
                exports: data_len,
                imports: data_len,
                end: data_len,
            },
        ],
        classnames: vec![ClassnameEntry {
            position: 0,
            signature: 0,
            name: class_name.to_string(),
        }],
        local_fixups: Vec::new(),
        global_fixups: Vec::new(),
        virtual_fixups: vec![VirtualFixup {
            source: 0,
            section: 0,
            classname_offset: 0,
        }],
    }
}

fn write_u32(data: &mut [u8], offset: usize, value: u32) {
    data[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

#[derive(Default)]
struct ValueCounts {
    arrays: usize,
    objects: usize,
    strings: usize,
    pointers: usize,
}

#[test]
fn reads_fo4_packfile_fixtures_into_descriptor_backed_objects() {
    for (relative, fixture_class) in fo4_fixture_paths() {
        let data = fixture_bytes(relative);
        let hkx = read_packfile(&data).unwrap_or_else(|error| {
            panic!("failed to read {relative}: {error}");
        });

        assert!(
            !hkx.objects().is_empty(),
            "{relative} should contain objects"
        );
        assert_eq!(hkx.class_version(), 11, "{relative}");
        assert!(hkx.contents_version().contains("2014"), "{relative}");
        assert_eq!(
            hkx.padding_size(),
            hkx.packfile().header.padding_size,
            "{relative}"
        );
        assert_eq!(hkx.save_unchanged(), data, "{relative}");
        assert!(
            hkx.objects()
                .iter()
                .any(|object| object.class_name == "hkRootLevelContainer"),
            "{relative} should contain hkRootLevelContainer"
        );

        let packfile_declares_fixture_class = hkx
            .packfile()
            .classnames
            .iter()
            .any(|entry| entry.name == fixture_class);
        if packfile_declares_fixture_class {
            assert!(
                hkx.objects()
                    .iter()
                    .any(|object| object.class_name == fixture_class),
                "{relative} should contain fixture-specific class {fixture_class}"
            );
        }
    }
}

#[test]
fn descriptor_reader_handles_arrays_pointers_strings_and_nested_structs_without_panics() {
    for (relative, _) in fo4_fixture_paths() {
        let data = fixture_bytes(relative);
        let hkx = read_packfile(&data).unwrap_or_else(|error| {
            panic!("failed to read {relative}: {error}");
        });
        let mut counts = ValueCounts::default();

        for object in hkx.objects() {
            assert!(
                object.offset < data.len(),
                "{relative} object offset in bounds"
            );
            assert!(!object.class_name.is_empty(), "{relative} object has class");
            for member in &object.members {
                assert!(!member.name.is_empty(), "{relative} member has name");
                walk_value(&member.value, &mut counts);
            }
        }

        assert!(
            counts.arrays > 0,
            "{relative} should decode at least one array"
        );
        assert!(
            counts.pointers > 0,
            "{relative} should decode at least one pointer"
        );
        assert!(
            counts.strings > 0,
            "{relative} should decode at least one string"
        );
        assert!(
            counts.objects > 0,
            "{relative} should decode at least one nested struct"
        );
    }
}

#[test]
fn struct_stride_counts_fixed_c_arrays_and_recurses_for_nested_alignment() {
    let dir = temp_classxml_dir("nested_stride");
    write_temp_classxml(
        &dir,
        "hkStrideRoot_0.xml",
        "<class name='hkStrideRoot' version='0' signature='0x1'><members><member name='items' type='hkArray&lt;struct hkNestedStride&gt;' ctype='hkNestedStride' offset='0' vtype='TYPE_ARRAY' vsubtype='TYPE_STRUCT' arrsize='0' flags='FLAGS_NONE'/></members></class>",
    );
    write_temp_classxml(
        &dir,
        "hkNestedStride_0.xml",
        "<struct name='hkNestedStride' version='0' signature='0x2'><members><member name='children' type='struct hkFixedArrayChild[2]' ctype='hkFixedArrayChild' offset='0' vtype='TYPE_STRUCT' vsubtype='TYPE_VOID' arrsize='2' flags='FLAGS_NONE'/><member name='after' type='hkUint32' offset='96' vtype='TYPE_UINT32' vsubtype='TYPE_VOID' arrsize='0' flags='FLAGS_NONE'/></members></struct>",
    );
    write_temp_classxml(
        &dir,
        "hkFixedArrayChild_0.xml",
        "<struct name='hkFixedArrayChild' version='0' signature='0x3'><members><member name='vectors' type='hkVector4[2]' offset='0' vtype='TYPE_VECTOR4' vsubtype='TYPE_VOID' arrsize='2' flags='FLAGS_NONE'/><member name='tail' type='hkUint32' offset='32' vtype='TYPE_UINT32' vsubtype='TYPE_VOID' arrsize='0' flags='FLAGS_NONE'/></members></struct>",
    );

    let mut data = vec![0_u8; 0x200];
    write_u32(&mut data, 8, 2);
    write_u32(&mut data, 12, 0x8000_0000);
    write_u32(&mut data, 0x20 + 96, 0x1111_1111);
    write_u32(&mut data, 0x20 + 112 + 96, 0x2222_2222);
    let mut packfile = synthetic_packfile_for_class("hkStrideRoot", data.len());
    packfile.local_fixups.push(LocalFixup {
        source: 0,
        target: 0x20,
    });
    let mut registry = DescriptorRegistry::from_dir(&dir).expect("temp descriptors");

    let objects =
        read_objects_with_registry(&data, &packfile, &mut registry).expect("read objects");
    let HkxValue::Array(items) = &objects[0].members[0].value else {
        panic!("items should be an array");
    };
    let HkxValue::Object(second_members) = &items[1] else {
        panic!("second item should be a nested object");
    };

    assert_eq!(second_members[1].name, "after");
    assert_eq!(second_members[1].value, HkxValue::U32(0x2222_2222));

    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn nonempty_array_without_local_payload_fixup_is_invalid() {
    let dir = temp_classxml_dir("missing_array_fixup");
    write_temp_classxml(
        &dir,
        "hkArrayRoot_0.xml",
        "<class name='hkArrayRoot' version='0' signature='0x1'><members><member name='values' type='hkArray&lt;hkUint32&gt;' offset='0' vtype='TYPE_ARRAY' vsubtype='TYPE_UINT32' arrsize='0' flags='FLAGS_NONE'/></members></class>",
    );
    let mut data = vec![0_u8; 0x40];
    write_u32(&mut data, 8, 1);
    let packfile = synthetic_packfile_for_class("hkArrayRoot", data.len());
    let mut registry = DescriptorRegistry::from_dir(&dir).expect("temp descriptors");

    let error = read_objects_with_registry(&data, &packfile, &mut registry).unwrap_err();

    assert!(
        error.to_string().contains("missing local fixup"),
        "unexpected error: {error}"
    );
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn strings_preserve_null_state_separately_from_empty_string() {
    let dir = temp_classxml_dir("string_null_state");
    write_temp_classxml(
        &dir,
        "hkStringRoot_0.xml",
        "<class name='hkStringRoot' version='0' signature='0x1'><members><member name='name' type='hkStringPtr' offset='0' vtype='TYPE_STRINGPTR' vsubtype='TYPE_VOID' arrsize='0' flags='FLAGS_NONE'/></members></class>",
    );
    let mut registry = DescriptorRegistry::from_dir(&dir).expect("temp descriptors");
    let data = vec![0_u8; 0x40];
    let packfile = synthetic_packfile_for_class("hkStringRoot", data.len());

    let objects =
        read_objects_with_registry(&data, &packfile, &mut registry).expect("read null string");
    assert_eq!(
        objects[0].members[0].value,
        HkxValue::String {
            value: String::new(),
            is_null: true,
        }
    );

    let mut packfile = synthetic_packfile_for_class("hkStringRoot", data.len());
    packfile.local_fixups.push(LocalFixup {
        source: 0,
        target: 0x20,
    });
    let objects =
        read_objects_with_registry(&data, &packfile, &mut registry).expect("read empty string");
    assert_eq!(
        objects[0].members[0].value,
        HkxValue::String {
            value: String::new(),
            is_null: false,
        }
    );
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn fixups_validate_section_indices_and_payload_ranges() {
    let dir = temp_classxml_dir("fixup_validation");
    write_temp_classxml(
        &dir,
        "hkPointerRoot_0.xml",
        "<class name='hkPointerRoot' version='0' signature='0x1'><members><member name='target' type='struct hkPointerRoot*' ctype='hkPointerRoot' offset='0' vtype='TYPE_POINTER' vsubtype='TYPE_STRUCT' arrsize='0' flags='FLAGS_NONE'/></members></class>",
    );
    let data = vec![0_u8; 0x40];

    let mut packfile = synthetic_packfile_for_class("hkPointerRoot", data.len());
    packfile.global_fixups.push(GlobalFixup {
        source: 0,
        section: 99,
        target: 0,
    });
    let mut registry = DescriptorRegistry::from_dir(&dir).expect("temp descriptors");
    let error = read_objects_with_registry(&data, &packfile, &mut registry).unwrap_err();
    assert!(
        error.to_string().contains("section out of bounds"),
        "unexpected error: {error}"
    );

    let mut packfile = synthetic_packfile_for_class("hkPointerRoot", data.len());
    packfile.local_fixups.push(LocalFixup {
        source: 0,
        target: 0x80,
    });
    let error = read_objects_with_registry(&data, &packfile, &mut registry).unwrap_err();
    assert!(
        error.to_string().contains("outside __data__ payload"),
        "unexpected error: {error}"
    );

    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn virtual_object_fixups_reject_in_bounds_wrong_section() {
    let dir = temp_classxml_dir("wrong_virtual_section");
    write_temp_classxml(
        &dir,
        "hkVirtualRoot_0.xml",
        "<class name='hkVirtualRoot' version='0' signature='0x1'><members><member name='value' type='hkUint32' offset='0' vtype='TYPE_UINT32' vsubtype='TYPE_VOID' arrsize='0' flags='FLAGS_NONE'/></members></class>",
    );
    let data = vec![0_u8; 0x40];
    let mut packfile = synthetic_packfile_for_class("hkVirtualRoot", data.len());
    packfile.sections[1].offset = 0;
    packfile.sections[1].data1 = data.len();
    packfile.virtual_fixups[0].section = 1;
    let mut registry = DescriptorRegistry::from_dir(&dir).expect("temp descriptors");

    let error = read_objects_with_registry(&data, &packfile, &mut registry).unwrap_err();

    assert!(
        error
            .to_string()
            .contains("virtual fixup section must be __data__"),
        "unexpected error: {error}"
    );
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn object_pointer_global_fixups_reject_in_bounds_wrong_target_section() {
    let dir = temp_classxml_dir("wrong_global_section");
    write_temp_classxml(
        &dir,
        "hkPointerRoot_0.xml",
        "<class name='hkPointerRoot' version='0' signature='0x1'><members><member name='target' type='struct hkPointerRoot*' ctype='hkPointerRoot' offset='0' vtype='TYPE_POINTER' vsubtype='TYPE_STRUCT' arrsize='0' flags='FLAGS_NONE'/></members></class>",
    );
    let data = vec![0_u8; 0x40];
    let mut packfile = synthetic_packfile_for_class("hkPointerRoot", data.len());
    packfile.sections[1].offset = 0;
    packfile.sections[1].data1 = data.len();
    packfile.global_fixups.push(GlobalFixup {
        source: 0,
        section: 1,
        target: 0,
    });
    let mut registry = DescriptorRegistry::from_dir(&dir).expect("temp descriptors");

    let error = read_objects_with_registry(&data, &packfile, &mut registry).unwrap_err();

    assert!(
        error
            .to_string()
            .contains("global fixup target section must be __data__"),
        "unexpected error: {error}"
    );
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn read_array_with_huge_count_returns_error_not_oom() {
    // a malformed array header with count = i32::MAX must return a
    // typed error instead of attempting a ~2 GiB allocation.
    let dir = temp_classxml_dir("huge_count");
    write_temp_classxml(
        &dir,
        "BigArray_0.xml",
        "<struct name='BigArray' version='0' signature='0xDEAD0001'>\
          <members>\
            <member name='items' type='hkArray&lt;hkUint32&gt;' offset='0' \
              vtype='TYPE_ARRAY' vsubtype='TYPE_UINT32' arrsize='0' flags='FLAGS_NONE'/>\
          </members>\
        </struct>",
    );

    // Object body: 16-byte hkArray header + a local fixup pointing to the body.
    // count = i32::MAX = 0x7FFFFFFF.
    let mut data = vec![0u8; 32];
    // hkArray header: ptr(8) = 0, count(4) = i32::MAX, flags(4) = 0
    data[8..12].copy_from_slice(&0x7FFFFFFFu32.to_le_bytes());
    // Put the "array data" at offset 16 (just the end of the object body).
    // local fixup: src=0, dst=16.

    let mut packfile = synthetic_packfile_for_class("BigArray", data.len());
    packfile.local_fixups.push(LocalFixup {
        source: 0,
        target: 16,
    });

    let mut registry = DescriptorRegistry::from_dir(&dir).expect("temp descriptors");
    let error = read_objects_with_registry(&data, &packfile, &mut registry).unwrap_err();

    assert!(
        error.to_string().contains("exceeds available data") || error.to_string().contains("array"),
        "expected array count error, got: {error}"
    );
    std::fs::remove_dir_all(dir).ok();
}
