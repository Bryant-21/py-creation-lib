use std::path::PathBuf;

use havok_native::hkx::packfile::parse_packfile;
use havok_native::hkx::patcher::{PatchRange, patch_hkx};
use havok_native::hkx::types::{HkxType, HkxValue};
use havok_native::hkx::{HkxFile, read_packfile};

fn fixture_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

const HKX_MAGIC: &[u8; 8] = b"\x57\xE0\xE0\x57\x10\xC0\xC0\x10";

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
            data1: 0x10,
            data2: 0x10,
            data3: 0x10,
            exports: 0x1C,
            imports: 0x1C,
            end: 0x1C,
        },
    );

    write_u32(&mut data, 0xD0, 0x12345678);
    data[0xD4] = 0x09;
    data[0xD5..0xD5 + 20].copy_from_slice(b"hkRootLevelContainer");
    write_u32(&mut data, 0x150, 0);
    write_u32(&mut data, 0x154, 0);
    write_u32(&mut data, 0x158, 5);

    data
}

#[test]
fn same_length_byte_overlay_updates_saved_bytes() {
    let data = synthetic_v8_packfile();
    let mut hkx = read_packfile(&data).expect("read synthetic packfile");
    let patch = PatchRange::new(0x170, 4, [0xAA, 0xBB, 0xCC, 0xDD]);

    hkx.apply_patch(patch).expect("apply same-length patch");
    let saved = hkx.save();

    assert!(hkx.is_dirty());
    assert_eq!(&saved[0x170..0x174], &[0xAA, 0xBB, 0xCC, 0xDD]);
    assert_eq!(&saved[..0x170], &data[..0x170]);
    assert_eq!(&saved[0x174..], &data[0x174..]);
}

#[test]
fn length_changing_overlay_is_rejected() {
    let data = synthetic_v8_packfile();
    let mut hkx = read_packfile(&data).expect("read synthetic packfile");
    let patch = PatchRange::new(0x170, 4, [1, 2, 3]);

    let error = hkx.apply_patch(patch).unwrap_err();

    assert!(
        error.to_string().contains("same length"),
        "unexpected error: {error}"
    );
    assert!(!hkx.is_dirty());
    assert_eq!(hkx.save(), data);
}

#[test]
fn out_of_bounds_overlay_is_rejected() {
    let data = synthetic_v8_packfile();
    let mut hkx = read_packfile(&data).expect("read synthetic packfile");
    let patch = PatchRange::new(data.len() - 1, 2, [1, 2]);

    let error = hkx.apply_patch(patch).unwrap_err();

    assert!(
        error.to_string().contains("outside source bytes"),
        "unexpected error: {error}"
    );
    assert!(!hkx.is_dirty());
    assert_eq!(hkx.save(), data);
}

#[test]
fn dirty_save_returns_patched_bytes() {
    let data = synthetic_v8_packfile();
    let mut hkx = HkxFile::read(&data).expect("read synthetic packfile");

    hkx.apply_patch(PatchRange::new(0x170, 2, [0x11, 0x22]))
        .expect("apply patch");

    assert_ne!(hkx.save(), data);
    assert_eq!(&hkx.save()[0x170..0x172], &[0x11, 0x22]);
}

#[test]
fn unchanged_save_remains_byte_identical() {
    let data = synthetic_v8_packfile();
    let hkx = HkxFile::read(&data).expect("read synthetic packfile");

    assert!(!hkx.is_dirty());
    assert_eq!(hkx.save(), data);
    assert_eq!(hkx.save_unchanged(), data);
}

#[test]
fn patched_synthetic_safe_data_can_be_parsed_again() {
    let data = synthetic_v8_packfile();
    let mut hkx = HkxFile::read(&data).expect("read synthetic packfile");

    hkx.apply_patch(PatchRange::new(0x170, 4, [0xFE, 0xED, 0xFA, 0xCE]))
        .expect("apply patch");
    let saved = hkx.save();
    let reparsed = HkxFile::read(&saved).expect("reparse patched synthetic packfile");

    parse_packfile(&saved).expect("parse patched bytes");
    assert_eq!(reparsed.save(), saved);
    assert!(!reparsed.is_dirty());
}

#[test]
fn patch_hkx_returns_byte_identical_when_unchanged() {
    let data = std::fs::read(fixture_path("native/havok/tests/fixtures/skeleton.hkx"))
        .expect("read skeleton");
    let hkx = read_packfile(&data).expect("parse skeleton");

    assert!(
        !hkx.array_sources().is_empty(),
        "reader should have populated array_sources"
    );

    let patched = patch_hkx(&hkx).expect("patch_hkx round-trip");
    assert_eq!(patched, data, "no-op patch must be byte-exact");
}

#[test]
fn patch_hkx_overlays_in_place_array_mutation() {
    let data = std::fs::read(fixture_path("native/havok/tests/fixtures/skeleton.hkx"))
        .expect("read skeleton");
    let mut hkx = read_packfile(&data).expect("parse skeleton");

    // Find the first DIRECT (scalar) array we can mutate without changing length.
    let entry = hkx
        .array_sources()
        .iter()
        .find(|src| {
            matches!(
                src.element_subtype,
                HkxType::Uint8
                    | HkxType::Int8
                    | HkxType::Int16
                    | HkxType::Uint16
                    | HkxType::Int32
                    | HkxType::Uint32
                    | HkxType::Real
            )
        })
        .cloned()
        .expect("skeleton has at least one scalar array");
    let object_index = entry.object_index;
    let path = entry.member_path.clone();
    let content_offset = entry.content_offset;
    let content_length = entry.content_length;

    // Mutate the model: replace each item with type-appropriate zero.
    let original_len = {
        let array = walk_array_mut(hkx.objects_mut(), object_index, &path).expect("walk to array");
        let HkxValue::Array(items) = array else {
            panic!("expected array");
        };
        let original_len = items.len();
        for item in items.iter_mut() {
            zero_out(item);
        }
        original_len
    };

    let patched = patch_hkx(&hkx).expect("patch_hkx after mutation");
    assert_eq!(patched.len(), data.len(), "length must be preserved");
    // Bytes outside the mutated array must match the source.
    assert_eq!(&patched[..content_offset], &data[..content_offset]);
    assert_eq!(
        &patched[content_offset + content_length..],
        &data[content_offset + content_length..]
    );
    // Mutated bytes must all be zero.
    assert!(
        patched[content_offset..content_offset + content_length]
            .iter()
            .all(|&b| b == 0),
        "mutated array region should be all zero"
    );
    let _ = original_len;
}

#[test]
fn patch_hkx_rejects_array_length_change() {
    let data = std::fs::read(fixture_path("native/havok/tests/fixtures/skeleton.hkx"))
        .expect("read skeleton");
    let mut hkx = read_packfile(&data).expect("parse skeleton");

    // Find a scalar array with at least one element.
    let entry = hkx
        .array_sources()
        .iter()
        .find(|src| {
            matches!(
                src.element_subtype,
                HkxType::Uint8 | HkxType::Int32 | HkxType::Uint32 | HkxType::Real
            ) && src.content_length > 0
        })
        .cloned()
        .expect("skeleton has a non-empty scalar array");
    let object_index = entry.object_index;
    let path = entry.member_path.clone();

    // Resize the array (push a duplicate element).
    {
        let array = walk_array_mut(hkx.objects_mut(), object_index, &path).expect("walk to array");
        let HkxValue::Array(items) = array else {
            panic!("expected array");
        };
        let extra = items[0].clone();
        items.push(extra);
    }

    let err = patch_hkx(&hkx).expect_err("length change should fail");
    let msg = err.to_string();
    assert!(
        msg.contains("array length changed") || msg.contains("cannot patch"),
        "expected length-change error, got: {msg}"
    );
}

#[test]
fn patch_hkx_rejects_when_no_source_bytes() {
    let hkx = HkxFile::from_tagxml(11, "hk_2014.1.0-r1", Vec::new());
    let err = patch_hkx(&hkx).expect_err("no source bytes should fail");
    assert!(
        err.to_string().contains("no source bytes"),
        "unexpected error: {err}"
    );
}

fn walk_array_mut<'a>(
    objects: &'a mut [havok_native::hkx::HkxObject],
    object_index: usize,
    path: &[String],
) -> Option<&'a mut HkxValue> {
    let object = objects.get_mut(object_index)?;
    if path.is_empty() {
        return None;
    }
    // First step is always a member name on the object.
    let first = &path[0];
    let m = object.members.iter_mut().find(|m| m.name == *first)?;
    let mut current = &mut m.value;
    for step in &path[1..] {
        if let Some(idx) = step
            .strip_prefix('[')
            .and_then(|s| s.strip_suffix(']'))
            .and_then(|s| s.parse::<usize>().ok())
        {
            let HkxValue::Array(items) = current else {
                return None;
            };
            current = items.get_mut(idx)?;
            continue;
        }
        let HkxValue::Object(nested) = current else {
            return None;
        };
        let m = nested.iter_mut().find(|m| m.name == *step)?;
        current = &mut m.value;
    }
    Some(current)
}

fn zero_out(value: &mut HkxValue) {
    match value {
        HkxValue::I8(v) => *v = 0,
        HkxValue::U8(v) => *v = 0,
        HkxValue::I16(v) => *v = 0,
        HkxValue::U16(v) => *v = 0,
        HkxValue::I32(v) => *v = 0,
        HkxValue::U32(v) => *v = 0,
        HkxValue::I64(v) => *v = 0,
        HkxValue::U64(v) => *v = 0,
        HkxValue::F32(v) => *v = 0.0,
        HkxValue::F32List(list) => list.iter_mut().for_each(|v| *v = 0.0),
        HkxValue::Bool(v) => *v = false,
        _ => {}
    }
}
