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
fn same_length_overlay_patches_saved_bytes_and_reparses() {
    let data = synthetic_v8_packfile();
    let hkx = HkxFile::read(&data).expect("read synthetic packfile");
    assert!(!hkx.is_dirty());
    assert_eq!(hkx.save(), data);
    assert_eq!(hkx.save_unchanged(), data);

    let mut hkx = read_packfile(&data).expect("read synthetic packfile");
    hkx.apply_patch(PatchRange::new(0x170, 4, [0xAA, 0xBB, 0xCC, 0xDD]))
        .expect("apply same-length patch");
    let saved = hkx.save();
    assert!(hkx.is_dirty());
    assert_eq!(&saved[0x170..0x174], &[0xAA, 0xBB, 0xCC, 0xDD]);
    assert_eq!(&saved[..0x170], &data[..0x170]);
    assert_eq!(&saved[0x174..], &data[0x174..]);

    parse_packfile(&saved).expect("parse patched bytes");
    let reparsed = HkxFile::read(&saved).expect("reparse patched synthetic packfile");
    assert_eq!(reparsed.save(), saved);
    assert!(!reparsed.is_dirty());
}

#[test]
fn length_changing_or_out_of_bounds_overlays_are_rejected_without_dirtying() {
    let data = synthetic_v8_packfile();
    for (patch, message) in [
        (PatchRange::new(0x170, 4, [1, 2, 3]), "same length"),
        (
            PatchRange::new(data.len() - 1, 2, [1, 2]),
            "outside source bytes",
        ),
    ] {
        let mut hkx = read_packfile(&data).expect("read synthetic packfile");
        let error = hkx.apply_patch(patch).unwrap_err();
        assert!(error.to_string().contains(message), "{message}: {error}");
        assert!(!hkx.is_dirty());
        assert_eq!(hkx.save(), data);
    }
}

fn read_skeleton_fixture() -> Vec<u8> {
    std::fs::read(fixture_path("native/havok/tests/fixtures/skeleton.hkx")).expect("read skeleton")
}

#[test]
fn patch_hkx_is_byte_exact_when_unchanged_and_overlays_in_place_array_mutation() {
    let data = read_skeleton_fixture();
    let mut hkx = read_packfile(&data).expect("parse skeleton");
    assert!(!hkx.array_sources().is_empty());
    assert_eq!(patch_hkx(&hkx).expect("no-op patch"), data);

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
    let HkxValue::Array(items) =
        walk_array_mut(hkx.objects_mut(), entry.object_index, &entry.member_path)
            .expect("walk to array")
    else {
        panic!("expected array");
    };
    items.iter_mut().for_each(zero_out);

    let patched = patch_hkx(&hkx).expect("patch_hkx after mutation");
    let (start, end) = (
        entry.content_offset,
        entry.content_offset + entry.content_length,
    );
    assert_eq!(patched.len(), data.len());
    assert_eq!(&patched[..start], &data[..start]);
    assert_eq!(&patched[end..], &data[end..]);
    assert!(patched[start..end].iter().all(|&b| b == 0));
}

#[test]
fn patch_hkx_rejects_array_length_changes_and_models_without_source_bytes() {
    let data = read_skeleton_fixture();
    let mut hkx = read_packfile(&data).expect("parse skeleton");
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
    let HkxValue::Array(items) =
        walk_array_mut(hkx.objects_mut(), entry.object_index, &entry.member_path)
            .expect("walk to array")
    else {
        panic!("expected array");
    };
    items.push(items[0].clone());
    let msg = patch_hkx(&hkx).expect_err("length change").to_string();
    assert!(
        msg.contains("array length changed") || msg.contains("cannot patch"),
        "{msg}"
    );

    let empty = HkxFile::from_tagxml(11, "hk_2014.1.0-r1", Vec::new());
    let err = patch_hkx(&empty).expect_err("no source bytes should fail");
    assert!(err.to_string().contains("no source bytes"), "{err}");
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
