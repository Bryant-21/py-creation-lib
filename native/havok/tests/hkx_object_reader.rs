use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use havok_native::error::HavokError;
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
            little_endian: 1,
            reuse_padding_optimization: 0,
            empty_base_class_optimization: 1,
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

fn write_f32(data: &mut [u8], offset: usize, value: f32) {
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

#[test]
fn legacy_asset_bundle_entries_use_their_2010_string_pointer_stride() {
    let dir = temp_classxml_dir("legacy_asset_bundle");
    write_temp_classxml(
        &dir,
        "LegacyAssetOwner_0.xml",
        "<class name='LegacyAssetOwner' version='0'><members><member name='bundles' type='hkArray&lt;struct hkbAssetBundleStringData&gt;' ctype='hkbAssetBundleStringData' offset='0' vtype='TYPE_ARRAY' vsubtype='TYPE_STRUCT' arrsize='0' flags='FLAGS_NONE'/></members></class>",
    );
    write_temp_classxml(
        &dir,
        "hkbAssetBundleStringData_0.xml",
        "<struct name='hkbAssetBundleStringData' version='0'><members><member name='bundleName' type='hkStringPtr' offset='0' vtype='TYPE_STRINGPTR' vsubtype='TYPE_VOID' arrsize='0' flags='FLAGS_NONE'/><member name='assetNames' type='hkArray&lt;hkStringPtr&gt;' offset='8' vtype='TYPE_ARRAY' vsubtype='TYPE_STRINGPTR' arrsize='0' flags='FLAGS_NONE'/></members></struct>",
    );

    let mut data = vec![0_u8; 0x60];
    write_u32(&mut data, 8, 2);
    data[0x40..0x46].copy_from_slice(b"first\0");
    data[0x48..0x4f].copy_from_slice(b"second\0");
    let mut packfile = synthetic_packfile_for_class("LegacyAssetOwner", data.len());
    packfile.header.version = 8;
    packfile.header.version_name = "hk_2010.2.0-r1".to_string();
    packfile.local_fixups.extend([
        LocalFixup {
            source: 0,
            target: 0x20,
        },
        LocalFixup {
            source: 0x20,
            target: 0x40,
        },
        LocalFixup {
            source: 0x28,
            target: 0x48,
        },
    ]);
    let mut registry = DescriptorRegistry::from_dir(&dir).expect("temp descriptors");

    let objects = read_objects_with_registry(&data, &packfile, &mut registry).expect("decode");
    let HkxValue::Array(bundles) = &objects[0].members[0].value else {
        panic!("bundles should be an array");
    };
    assert_eq!(bundles.len(), 2);
    for (bundle, expected) in bundles.iter().zip(["first", "second"]) {
        let HkxValue::Object(members) = bundle else {
            panic!("bundle should be an inline struct");
        };
        assert_eq!(
            members[0].value,
            HkxValue::String {
                value: expected.to_string(),
                is_null: false,
            }
        );
        assert_eq!(members[1].value, HkxValue::Array(Vec::new()));
    }

    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn legacy_modifier_list_recovers_the_array_header_from_its_local_fixup() {
    let dir = temp_classxml_dir("legacy_modifier_list");
    write_temp_classxml(
        &dir,
        "hkbModifierList_0.xml",
        "<class name='hkbModifierList' version='0'><members><member name='modifiers' type='hkArray&lt;hkbModifier*&gt;' ctype='hkbModifier' offset='8' vtype='TYPE_ARRAY' vsubtype='TYPE_POINTER' arrsize='0' flags='FLAGS_NONE'/></members></class>",
    );
    let mut data = vec![0_u8; 0x40];
    write_u32(&mut data, 8, 2);
    let mut packfile = synthetic_packfile_for_class("hkbModifierList", data.len());
    packfile.header.version = 8;
    packfile.header.version_name = "hk_2010.2.0-r1".to_string();
    packfile.local_fixups.push(LocalFixup {
        source: 0,
        target: 0x20,
    });
    let mut registry = DescriptorRegistry::from_dir(&dir).expect("temp descriptors");

    let objects = read_objects_with_registry(&data, &packfile, &mut registry).expect("decode");
    let HkxValue::Array(modifiers) = &objects[0].members[0].value else {
        panic!("modifiers should be an array");
    };
    assert_eq!(modifiers.len(), 2);

    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn legacy_skeleton_does_not_read_post_2010_partitions_from_following_payload() {
    let dir = temp_classxml_dir("legacy_skeleton_partitions");
    write_temp_classxml(
        &dir,
        "hkaSkeleton_5.xml",
        "<class name='hkaSkeleton' version='5'><members><member name='partitions' type='hkArray&lt;struct hkaSkeletonPartition&gt;' ctype='hkaSkeletonPartition' offset='0' vtype='TYPE_ARRAY' vsubtype='TYPE_STRUCT' arrsize='0' flags='FLAGS_NONE'/></members></class>",
    );
    let mut data = vec![0_u8; 0x20];
    write_u32(&mut data, 8, 99);
    let mut packfile = synthetic_packfile_for_class("hkaSkeleton", data.len());
    packfile.header.version = 8;
    packfile.header.version_name = "hk_2010.2.0-r1".to_string();
    let mut registry = DescriptorRegistry::from_dir(&dir).expect("temp descriptors");

    let objects = read_objects_with_registry(&data, &packfile, &mut registry).expect("decode");
    assert_eq!(objects[0].members[0].value, HkxValue::Array(Vec::new()));

    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn legacy_mirrored_skeleton_stops_before_the_following_object() {
    let dir = temp_classxml_dir("legacy_mirrored_skeleton");
    write_temp_classxml(
        &dir,
        "hkbMirroredSkeletonInfo_1.xml",
        "<class name='hkbMirroredSkeletonInfo' version='1'><members><member name='partitionPairMap' type='hkArray&lt;hkInt16&gt;' offset='48' vtype='TYPE_ARRAY' vsubtype='TYPE_INT16' arrsize='0' flags='FLAGS_NONE'/></members></class>",
    );
    write_temp_classxml(
        &dir,
        "FollowingObject_0.xml",
        "<class name='FollowingObject' version='0'><members><member name='value' type='hkUint32' offset='8' vtype='TYPE_UINT32' vsubtype='TYPE_VOID' arrsize='0' flags='FLAGS_NONE'/></members></class>",
    );
    let mut data = vec![0_u8; 0x80];
    write_u32(&mut data, 0x38, 7);
    let mut packfile = synthetic_packfile_for_class("hkbMirroredSkeletonInfo", data.len());
    packfile.header.version = 8;
    packfile.header.version_name = "hk_2010.2.0-r1".to_string();
    packfile.classnames.push(ClassnameEntry {
        position: 1,
        signature: 0,
        name: "FollowingObject".to_string(),
    });
    packfile.virtual_fixups.push(VirtualFixup {
        source: 0x30,
        section: 0,
        classname_offset: 1,
    });
    let mut registry = DescriptorRegistry::from_dir(&dir).expect("temp descriptors");

    let objects = read_objects_with_registry(&data, &packfile, &mut registry).expect("decode");
    assert_eq!(objects[0].members[0].value, HkxValue::Array(Vec::new()));
    assert_eq!(objects[1].members[0].value, HkxValue::U32(7));

    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn legacy_skeleton_mapper_uses_the_v40_inline_layout() {
    let dir = temp_classxml_dir("legacy_skeleton_mapper");
    write_temp_classxml(
        &dir,
        "hkaSkeletonMapperData_2.xml",
        "<class name='hkaSkeletonMapperData' version='2'><members><member name='partitionMap' type='hkArray&lt;hkUint32&gt;' offset='16' vtype='TYPE_ARRAY' vsubtype='TYPE_UINT32' arrsize='0' flags='FLAGS_NONE'/><member name='simpleMappingPartitionRanges' type='hkArray&lt;hkUint32&gt;' offset='32' vtype='TYPE_ARRAY' vsubtype='TYPE_UINT32' arrsize='0' flags='FLAGS_NONE'/><member name='chainMappingPartitionRanges' type='hkArray&lt;hkUint32&gt;' offset='48' vtype='TYPE_ARRAY' vsubtype='TYPE_UINT32' arrsize='0' flags='FLAGS_NONE'/><member name='simpleMappings' type='hkArray&lt;hkUint32&gt;' offset='64' vtype='TYPE_ARRAY' vsubtype='TYPE_UINT32' arrsize='0' flags='FLAGS_NONE'/><member name='chainMappings' type='hkArray&lt;hkUint32&gt;' offset='80' vtype='TYPE_ARRAY' vsubtype='TYPE_UINT32' arrsize='0' flags='FLAGS_NONE'/><member name='unmappedBones' type='hkArray&lt;hkUint32&gt;' offset='96' vtype='TYPE_ARRAY' vsubtype='TYPE_UINT32' arrsize='0' flags='FLAGS_NONE'/><member name='extractedMotionMapping' type='hkQsTransform' offset='112' vtype='TYPE_QSTRANSFORM' vsubtype='TYPE_VOID' arrsize='0' flags='FLAGS_NONE'/><member name='keepUnmappedLocal' type='hkBool' offset='160' vtype='TYPE_BOOL' vsubtype='TYPE_VOID' arrsize='0' flags='FLAGS_NONE'/><member name='mappingType' type='hkInt32' offset='164' vtype='TYPE_INT32' vsubtype='TYPE_VOID' arrsize='0' flags='FLAGS_NONE'/></members></class>",
    );
    let mut data = vec![0_u8; 0x100];
    for source in [0x10, 0x20, 0x30] {
        write_u32(&mut data, source + 8, 1);
    }
    for (index, value) in [11, 22, 33].into_iter().enumerate() {
        write_u32(&mut data, 0xd0 + index * 4, value);
    }
    for (index, value) in [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0]
        .into_iter()
        .enumerate()
    {
        write_f32(&mut data, 0x40 + index * 4, value);
    }
    data[0x70] = 1;
    write_u32(&mut data, 0x74, 1);
    let mut packfile = synthetic_packfile_for_class("hkaSkeletonMapperData", data.len());
    packfile.header.version = 8;
    packfile.header.version_name = "hk_2010.2.0-r1".to_string();
    packfile.local_fixups.extend([
        LocalFixup {
            source: 0x10,
            target: 0xd0,
        },
        LocalFixup {
            source: 0x20,
            target: 0xd4,
        },
        LocalFixup {
            source: 0x30,
            target: 0xd8,
        },
    ]);
    let mut registry = DescriptorRegistry::from_dir(&dir).expect("temp descriptors");

    let objects = read_objects_with_registry(&data, &packfile, &mut registry).expect("decode");
    let members = &objects[0].members;
    assert_eq!(members[0].value, HkxValue::Array(vec![HkxValue::U32(11)]));
    assert_eq!(members[1].value, HkxValue::Array(Vec::new()));
    assert_eq!(members[2].value, HkxValue::Array(Vec::new()));
    assert_eq!(members[3].value, HkxValue::Array(Vec::new()));
    assert_eq!(members[4].value, HkxValue::Array(vec![HkxValue::U32(22)]));
    assert_eq!(members[5].value, HkxValue::Array(vec![HkxValue::U32(33)]));
    assert_eq!(
        members[6].value,
        HkxValue::F32List(vec![
            0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0,
        ])
    );
    assert_eq!(members[7].value, HkxValue::Bool(true));
    assert_eq!(members[8].value, HkxValue::I32(1));

    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn legacy_blender_generator_uses_the_v40_class_member_offsets() {
    let dir = temp_classxml_dir("legacy_blender_generator");
    write_temp_classxml(
        &dir,
        "hkbBlenderGenerator_1.xml",
        "<class name='hkbBlenderGenerator' version='1'><members><member name='referencePoseWeightThreshold' type='hkReal' offset='136' vtype='TYPE_REAL' vsubtype='TYPE_VOID' arrsize='0' flags='FLAGS_NONE'/><member name='blendParameter' type='hkReal' offset='140' vtype='TYPE_REAL' vsubtype='TYPE_VOID' arrsize='0' flags='FLAGS_NONE'/><member name='minCyclicBlendParameter' type='hkReal' offset='144' vtype='TYPE_REAL' vsubtype='TYPE_VOID' arrsize='0' flags='FLAGS_NONE'/><member name='maxCyclicBlendParameter' type='hkReal' offset='148' vtype='TYPE_REAL' vsubtype='TYPE_VOID' arrsize='0' flags='FLAGS_NONE'/><member name='indexOfSyncMasterChild' type='hkInt16' offset='152' vtype='TYPE_INT16' vsubtype='TYPE_VOID' arrsize='0' flags='FLAGS_NONE'/><member name='flags' type='hkInt16' offset='154' vtype='TYPE_INT16' vsubtype='TYPE_VOID' arrsize='0' flags='FLAGS_NONE'/><member name='subtractLastChild' type='hkBool' offset='156' vtype='TYPE_BOOL' vsubtype='TYPE_VOID' arrsize='0' flags='FLAGS_NONE'/><member name='children' type='hkArray&lt;hkbBlenderGeneratorChild*&gt;' ctype='hkbBlenderGeneratorChild' offset='160' vtype='TYPE_ARRAY' vsubtype='TYPE_POINTER' arrsize='0' flags='FLAGS_NONE'/></members></class>",
    );
    let mut data = vec![0_u8; 0x100];
    for (offset, value) in [(0x48, 0.25), (0x4c, 0.5), (0x50, -1.0), (0x54, 2.0)] {
        write_f32(&mut data, offset, value);
    }
    data[0x58..0x5a].copy_from_slice(&(-1_i16).to_le_bytes());
    data[0x5a..0x5c].copy_from_slice(&(16_i16).to_le_bytes());
    data[0x5c] = 1;
    write_u32(&mut data, 0x68, 2);
    let mut packfile = synthetic_packfile_for_class("hkbBlenderGenerator", data.len());
    packfile.header.version = 8;
    packfile.header.version_name = "hk_2010.2.0-r1".to_string();
    packfile.local_fixups.push(LocalFixup {
        source: 0x60,
        target: 0xc0,
    });
    let mut registry = DescriptorRegistry::from_dir(&dir).expect("temp descriptors");

    let objects = read_objects_with_registry(&data, &packfile, &mut registry).expect("decode");
    let members = &objects[0].members;
    assert_eq!(members[0].value, HkxValue::F32(0.25));
    assert_eq!(members[1].value, HkxValue::F32(0.5));
    assert_eq!(members[2].value, HkxValue::F32(-1.0));
    assert_eq!(members[3].value, HkxValue::F32(2.0));
    assert_eq!(members[4].value, HkxValue::I16(-1));
    assert_eq!(members[5].value, HkxValue::I16(16));
    assert_eq!(members[6].value, HkxValue::Bool(true));
    let HkxValue::Array(children) = &members[7].value else {
        panic!("children should be an array");
    };
    assert_eq!(children.len(), 2);

    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn unknown_legacy_array_layout_reports_the_exact_member_and_source() {
    let dir = temp_classxml_dir("unknown_legacy_array");
    write_temp_classxml(
        &dir,
        "UnsupportedLegacyRoot_0.xml",
        "<class name='UnsupportedLegacyRoot' version='0'><members><member name='futureValues' type='hkArray&lt;hkUint32&gt;' offset='16' vtype='TYPE_ARRAY' vsubtype='TYPE_UINT32' arrsize='0' flags='FLAGS_NONE'/></members></class>",
    );
    let mut data = vec![0_u8; 0x40];
    write_u32(&mut data, 0x18, 3);
    let mut packfile = synthetic_packfile_for_class("UnsupportedLegacyRoot", data.len());
    packfile.header.version = 8;
    packfile.header.version_name = "hk_2010.2.0-r1".to_string();
    let mut registry = DescriptorRegistry::from_dir(&dir).expect("temp descriptors");

    let error = read_objects_with_registry(&data, &packfile, &mut registry).unwrap_err();
    assert!(matches!(
        error,
        HavokError::FeatureNotImplemented { feature, reason }
            if feature == "hk_2010.2.0-r1 class layout UnsupportedLegacyRoot.futureValues"
                && reason.contains("__data__+0x10")
    ));

    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn skyrim_character_data_signature_decodes_the_legacy_direct_controller_layout() {
    let dir = temp_classxml_dir("skyrim_character_controller");
    write_temp_classxml(
        &dir,
        "hkbCharacterData_10.xml",
        "<class name='hkbCharacterData' version='10'><members><member name='characterControllerSetup' type='struct hkbCharacterControllerSetup' ctype='hkbCharacterControllerSetup' offset='16' vtype='TYPE_STRUCT' vsubtype='TYPE_VOID' arrsize='0' flags='FLAGS_NONE'/><member name='modelUpMS' type='hkVector4' offset='64' vtype='TYPE_VECTOR4' vsubtype='TYPE_VOID' arrsize='0' flags='FLAGS_NONE'/><member name='modelForwardMS' type='hkVector4' offset='80' vtype='TYPE_VECTOR4' vsubtype='TYPE_VOID' arrsize='0' flags='FLAGS_NONE'/><member name='modelRightMS' type='hkVector4' offset='96' vtype='TYPE_VECTOR4' vsubtype='TYPE_VOID' arrsize='0' flags='FLAGS_NONE'/></members></class>",
    );
    let mut data = vec![0_u8; 0x80];
    write_f32(&mut data, 0x10, 1.7);
    write_f32(&mut data, 0x14, 0.4);
    write_u32(&mut data, 0x18, 1);
    for (offset, values) in [
        (0x30, [0.0, 0.0, 1.0, 0.0]),
        (0x40, [0.0, 1.0, 0.0, 0.0]),
        (0x50, [1.0, 0.0, 0.0, 0.0]),
    ] {
        for (index, value) in values.into_iter().enumerate() {
            write_f32(&mut data, offset + index * 4, value);
        }
    }
    let mut packfile = synthetic_packfile_for_class("hkbCharacterData", data.len());
    packfile.header.version = 8;
    packfile.header.version_name = "hk_2010.2.0-r1".to_string();
    packfile.classnames[0].signature = 0x300d_6808;
    let mut registry = DescriptorRegistry::from_dir(&dir).expect("temp descriptors");

    let objects = read_objects_with_registry(&data, &packfile, &mut registry).expect("decode");
    let members = &objects[0].members;
    assert_eq!(members[0].name, "characterControllerInfo");
    let HkxValue::TypedObject {
        class_name,
        members: controller,
    } = &members[0].value
    else {
        panic!("legacy controller must preserve its inline class");
    };
    assert_eq!(class_name, "hkbCharacterDataCharacterControllerInfo");
    assert_eq!(controller[0].value, HkxValue::F32(1.7));
    assert_eq!(controller[1].value, HkxValue::F32(0.4));
    assert_eq!(controller[2].value, HkxValue::U32(1));
    assert_eq!(controller[3].value, HkxValue::Pointer(None));
    assert_eq!(
        members[1].value,
        HkxValue::F32List(vec![0.0, 0.0, 1.0, 0.0])
    );
    assert_eq!(
        members[2].value,
        HkxValue::F32List(vec![0.0, 1.0, 0.0, 0.0])
    );
    assert_eq!(
        members[3].value,
        HkxValue::F32List(vec![1.0, 0.0, 0.0, 0.0])
    );

    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn unknown_skyrim_character_data_signature_is_a_typed_layout_blocker() {
    let dir = temp_classxml_dir("unknown_skyrim_character_controller");
    write_temp_classxml(
        &dir,
        "hkbCharacterData_10.xml",
        "<class name='hkbCharacterData' version='10'><members><member name='characterControllerSetup' type='struct hkbCharacterControllerSetup' ctype='hkbCharacterControllerSetup' offset='16' vtype='TYPE_STRUCT' vsubtype='TYPE_VOID' arrsize='0' flags='FLAGS_NONE'/></members></class>",
    );
    let data = vec![0_u8; 0x40];
    let mut packfile = synthetic_packfile_for_class("hkbCharacterData", data.len());
    packfile.header.version = 8;
    packfile.header.version_name = "hk_2010.2.0-r1".to_string();
    packfile.classnames[0].signature = 0xdead_beef;
    let mut registry = DescriptorRegistry::from_dir(&dir).expect("temp descriptors");

    let error = read_objects_with_registry(&data, &packfile, &mut registry).unwrap_err();
    assert!(matches!(
        error,
        HavokError::FeatureNotImplemented { feature, reason }
            if feature == "hk_2010.2.0-r1 class layout hkbCharacterData signature 0xdeadbeef"
                && reason.contains("0x300d6808")
    ));

    std::fs::remove_dir_all(dir).ok();
}
