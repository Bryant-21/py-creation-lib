use havok_native::api::hkx_detect_format_full;
use havok_native::hkx::tagfile2014::{parse_header, read_tagfile2014};
use havok_native::hkx::types::HkxValue;
use std::path::{Path, PathBuf};

fn tutorial_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .join("refs/newcreature/Samples")
        .join(relative)
}

fn read_fixture(relative: &str) -> Vec<u8> {
    let path = tutorial_path(relative);
    std::fs::read(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

#[test]
fn hct_2014_animation_uses_eight_byte_stream_prefix() {
    let data = read_fixture("Sample Behavior - Seeker Mine/Export/Animations/Idle.hkt");
    let header = parse_header(&data).expect("recognize HCT 2014 animation tagfile");
    assert_eq!(header.stream_offset, 8);
    assert!(!header.swap_bytes);

    let detected = hkx_detect_format_full(&data).expect("detect HCT 2014 animation tagfile");
    assert_eq!(detected.kind, "binary_tagfile");
    assert_eq!(detected.version, "hk_2014.1.0-r1");

    let hkx = read_tagfile2014(&data).expect("decode HCT 2014 animation tagfile");
    assert_eq!(hkx.contents_version(), "hk_2014.1.0-r1");
    assert!(!hkx.objects().is_empty());
}

fn assert_decodes(relative: &str, expected_class: &str) {
    let data = read_fixture(relative);
    let hkx = read_tagfile2014(&data).unwrap_or_else(|error| panic!("decode {relative}: {error}"));
    assert_eq!(hkx.contents_version(), "hk_2014.1.0-r1");
    assert!(
        hkx.objects()
            .iter()
            .any(|object| object.class_name == expected_class),
        "{relative} did not contain {expected_class}"
    );
}

fn skeleton_mapping_payload_offset(data: &[u8]) -> usize {
    // This anchor spans the struct presence byte and all four corpus mapping
    // rows, so a coincidental short VLE sequence cannot select the offset.
    let mapping_payload_prefix = [
        0x01, 0x04, 0x08, 0x08, 0x06, 0x04, 0x08, 0x08, 0x06, 0x04, 0x08, 0x08, 0x06, 0x04, 0x08,
        0x08, 0x06,
    ];
    let mut payload_matches = data
        .windows(mapping_payload_prefix.len())
        .enumerate()
        .filter_map(|(offset, bytes)| (bytes == mapping_payload_prefix).then_some(offset));
    let expected_offset = payload_matches
        .next()
        .expect("find hkaMeshBindingMapping mapping payload")
        + 1;
    assert!(
        payload_matches.next().is_none(),
        "mapping payload prefix must be unique"
    );
    expected_offset
}

#[test]
fn decodes_hct_2014_skeleton_nested_mapping_arrays() {
    let relative = "Sample Behavior - Seeker Mine/Export/CharacterAssets/Skeleton.hkt";
    let data = read_fixture(relative);
    let hkx = read_tagfile2014(&data).expect("decode mesh-bearing HCT 2014 skeleton");
    assert_eq!(hkx.contents_version(), "hk_2014.1.0-r1");

    let mesh_binding = hkx
        .objects()
        .iter()
        .find(|object| object.class_name == "hkaMeshBinding")
        .expect("hkaMeshBinding object");
    let mappings = mesh_binding
        .members
        .iter()
        .find(|member| member.name == "mappings")
        .expect("mappings member");
    let HkxValue::Array(mapping_rows) = &mappings.value else {
        panic!("expected mappings array, got {:?}", mappings.value);
    };
    assert_eq!(mapping_rows.len(), 4);
    for row in mapping_rows {
        let HkxValue::TypedObject {
            class_name,
            members,
        } = row
        else {
            panic!("expected mapping struct, got {row:?}");
        };
        assert_eq!(class_name, "hkaMeshBindingMapping");
        let mapping = members
            .iter()
            .find(|member| member.name == "mapping")
            .expect("mapping member");
        assert_eq!(
            mapping.value,
            HkxValue::Array(vec![HkxValue::I64(4), HkxValue::I64(3)])
        );
    }
}

#[test]
fn rejects_negative_hct_2014_nested_mapping_length_with_row_offset() {
    let relative = "Sample Behavior - Seeker Mine/Export/CharacterAssets/Skeleton.hkt";
    let mut data = read_fixture(relative);
    let mapping_offset = skeleton_mapping_payload_offset(&data);
    data[mapping_offset] = 0x03; // VLE -1

    let error = read_tagfile2014(&data).expect_err("reject negative nested mapping length");
    let message = error.to_string();
    assert!(message.contains("hkaMeshBindingMapping.mapping within mappings row 0"));
    assert!(message.contains(&format!("offset {mapping_offset:#X}")));
    assert!(message.contains("array size -1 not in usize"));
}

#[test]
fn rejects_truncated_hct_2014_nested_mapping_row_with_start_offset() {
    let relative = "Sample Behavior - Seeker Mine/Export/CharacterAssets/Skeleton.hkt";
    let mut data = read_fixture(relative);
    let mapping_offset = skeleton_mapping_payload_offset(&data);
    let second_row_offset = mapping_offset + 4;
    data.truncate(second_row_offset + 3);

    let error = read_tagfile2014(&data).expect_err("reject truncated nested mapping row");
    let message = error.to_string();
    assert!(message.contains("hkaMeshBindingMapping.mapping within mappings row 1"));
    assert!(message.contains(&format!("offset {second_row_offset:#X}")));
    assert!(message.contains("VLE: truncated at first byte"));
}

#[test]
fn decodes_hct_2014_ragdoll() {
    assert_decodes(
        "Sample Behavior - Sentry Machinegun Turret/Export/CharacterAssets/Ragdoll.hkt",
        "hkpPhysicsData",
    );
}

#[test]
fn decodes_hct_2014_root_behavior() {
    assert_decodes(
        "Sample Behavior - Seeker Mine/Export/Behaviors/SeekerMineRootBehavior.hkt",
        "hkbBehaviorGraph",
    );
}

#[test]
fn decodes_hct_2014_core_behavior() {
    assert_decodes(
        "Sample Behavior - Seeker Mine/Export/Behaviors/SeekerMineCoreBehavior.hkt",
        "hkbBehaviorGraph",
    );
}
