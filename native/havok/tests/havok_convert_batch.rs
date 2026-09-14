use std::path::{Path, PathBuf};

fn repo_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

fn clean_temp(name: &str) -> PathBuf {
    let temp = std::env::temp_dir().join(format!("{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp);
    temp
}

fn copy_fixture(relative: &str, destination: &Path) {
    std::fs::copy(repo_path(relative), destination).unwrap_or_else(|error| {
        panic!(
            "failed to copy fixture {relative} to {}: {error}",
            destination.display()
        );
    });
}

#[test]
fn convert_file_creates_parent_dirs_and_writes_structurally_valid_output() {
    let temp = clean_temp("havok_convert_file");
    let src = repo_path("native/havok/tests/fixtures/skeleton.hkx");
    let dst = temp.join("nested/out.hkx");

    havok_native::api::havok_convert_file(&src, &dst, "fo4").unwrap();

    let original = std::fs::read(&src).unwrap();
    let converted = std::fs::read(&dst).unwrap();
    assert_eq!(converted, original);
    havok_native::hkx::read_packfile(&converted).unwrap();
}

#[test]
fn convert_batch_runs_mixed_fixture_tree_with_deterministic_results() {
    let temp = clean_temp("havok_convert_batch");
    let src_root = temp.join("src");
    let dst_root = temp.join("dst");
    std::fs::create_dir_all(src_root.join("a")).unwrap();
    std::fs::create_dir_all(src_root.join("b")).unwrap();
    copy_fixture(
        "native/havok/tests/fixtures/skeleton.hkx",
        &src_root.join("a/skeleton.hkx"),
    );
    copy_fixture(
        "../bacup/py_bacup_lib/python/bacup_lib/tests/fixtures/creatures/deathclaw/expected/character.hkx",
        &src_root.join("b/character.hkx"),
    );
    copy_fixture(
        "python/creation_lib/hkxpack/tests/fixtures/fo76_snallygastercharacter.hkx",
        &src_root.join("b/fo76.hkx"),
    );

    let result = havok_native::api::havok_convert_batch(&src_root, &dst_root, "fo4", true).unwrap();

    // FO76 file converts successfully — all three files should be converted.
    assert_eq!(result.converted, 3);
    assert_eq!(result.skipped, 0);
    assert_eq!(result.errors.len(), 0);
    havok_native::hkx::read_packfile(&std::fs::read(dst_root.join("a/skeleton.hkx")).unwrap())
        .unwrap();
    havok_native::hkx::read_packfile(&std::fs::read(dst_root.join("b/character.hkx")).unwrap())
        .unwrap();
    // The FO76→FO4 converted file should also be a valid packfile.
    havok_native::hkx::read_packfile(&std::fs::read(dst_root.join("b/fo76.hkx")).unwrap()).unwrap();
}

#[test]
fn convert_batch_rejects_flattened_duplicate_destinations() {
    let temp = clean_temp("havok_convert_batch_duplicates");
    let src_root = temp.join("src");
    let dst_root = temp.join("dst");
    std::fs::create_dir_all(src_root.join("a")).unwrap();
    std::fs::create_dir_all(src_root.join("b")).unwrap();
    copy_fixture(
        "native/havok/tests/fixtures/skeleton.hkx",
        &src_root.join("a/same.hkx"),
    );
    copy_fixture(
        "../bacup/py_bacup_lib/python/bacup_lib/tests/fixtures/creatures/deathclaw/expected/character.hkx",
        &src_root.join("b/same.hkx"),
    );

    let error =
        havok_native::api::havok_convert_batch(&src_root, &dst_root, "fo4", false).unwrap_err();

    assert!(
        error.to_string().contains("duplicate batch destination"),
        "unexpected error: {error}"
    );
    assert!(!dst_root.join("same.hkx").exists());
}

#[test]
fn ps4_conversion_uses_64g_layout_and_preserves_the_model() {
    let source_bytes =
        std::fs::read(repo_path("native/havok/tests/fixtures/skeleton.hkx")).unwrap();
    let source = havok_native::hkx::read_packfile(&source_bytes).unwrap();

    let converted_bytes = havok_native::api::havok_convert_ps4_bytes(&source_bytes).unwrap();
    let header = havok_native::hkx::packfile::parse_header(&converted_bytes).unwrap();
    assert_eq!(
        (
            header.pointer_size,
            header.little_endian,
            header.reuse_padding_optimization,
            header.empty_base_class_optimization,
        ),
        (8, 1, 1, 1)
    );

    let converted = havok_native::hkx::read_packfile(&converted_bytes).unwrap();
    assert_eq!(source.objects().len(), converted.objects().len());
    for (source_object, converted_object) in source.objects().iter().zip(converted.objects()) {
        assert_eq!(source_object.class_name, converted_object.class_name);
        assert_eq!(source_object.signature, converted_object.signature);
        assert_eq!(source_object.members, converted_object.members);
    }
}

#[test]
fn ps4_conversion_preserves_zero_packfile_padding() {
    let source_bytes = std::fs::read(repo_path(
        "../bacup/py_bacup_lib/python/bacup_lib/tests/fixtures/creatures/deathclaw/expected/character.hkx",
    ))
    .unwrap();
    let source_header = havok_native::hkx::packfile::parse_header(&source_bytes).unwrap();
    assert_eq!(source_header.padding_size, 0);

    let converted_bytes = havok_native::api::havok_convert_ps4_bytes(&source_bytes).unwrap();
    let converted_header = havok_native::hkx::packfile::parse_header(&converted_bytes).unwrap();

    assert_eq!(converted_header.padding_size, 0);
    assert_eq!(converted_header.reuse_padding_optimization, 1);
}

#[test]
fn ps4_layout_honors_explicit_member_alignment() {
    use havok_native::hkx::descriptors::{DescriptorRegistry, StructureLayout};

    let mut registry = DescriptorRegistry::new();
    registry.set_structure_layout(StructureLayout::Generic);
    let members = registry
        .get_all_members("hkbRigidBodyRagdollControlsModifier")
        .unwrap();
    let control_data = members
        .iter()
        .find(|member| member.name == "controlData")
        .unwrap();

    assert_eq!(control_data.offset, 96);
}

#[test]
fn ps4_layout_honors_explicit_align_8_after_compact_base_class() {
    use havok_native::hkx::descriptors::{DescriptorRegistry, StructureLayout};

    let mut registry = DescriptorRegistry::new();
    registry.set_structure_layout(StructureLayout::Generic);
    let members = registry.get_all_members("hknpShapeMassProperties").unwrap();
    let compressed = members
        .iter()
        .find(|member| member.name == "compressedMassProperties")
        .unwrap();

    assert_eq!(compressed.offset, 16);
}

#[test]
fn ps4_object_array_payloads_reuse_base_class_padding() {
    use havok_native::hkx::descriptors::{DescriptorRegistry, StructureLayout};
    use havok_native::hkx::types::HkxValue;
    use havok_native::hkx::{HkxFile, HkxMember, HkxObject};

    let named_variant = HkxValue::Object(vec![
        HkxMember {
            name: "name".to_string(),
            value: HkxValue::String {
                value: "hkaAnimationContainer".to_string(),
                is_null: false,
            },
        },
        HkxMember {
            name: "className".to_string(),
            value: HkxValue::String {
                value: "hkaAnimationContainer".to_string(),
                is_null: false,
            },
        },
        HkxMember {
            name: "variant".to_string(),
            value: HkxValue::Pointer(None),
        },
    ]);
    let file = HkxFile::from_tagxml(
        11,
        "hk_2014.1.0-r1",
        vec![HkxObject {
            name: Some("#0000".to_string()),
            offset: 0,
            signature: 0x2772_c11e,
            class_name: "hkRootLevelContainer".to_string(),
            members: vec![HkxMember {
                name: "namedVariants".to_string(),
                value: HkxValue::Array(vec![named_variant]),
            }],
        }],
    );
    let mut registry = DescriptorRegistry::for_contents_version(file.contents_version());

    let bytes =
        havok_native::hkx::write_hkx_with_layout(&file, &mut registry, StructureLayout::Generic);
    let parsed = havok_native::hkx::packfile::parse_packfile(&bytes).unwrap();
    let fixups: Vec<_> = parsed
        .local_fixups
        .iter()
        .map(|fixup| (fixup.source, fixup.target))
        .collect();

    assert_eq!(fixups, vec![(0, 16), (16, 40), (24, 64)]);
}

#[test]
fn ps4_batch_uses_nested_ps4_folder_without_reprocessing_it() {
    let temp = clean_temp("havok_convert_ps4_batch");
    let src_root = temp.join("src");
    let dst_root = src_root.join("ps4");
    std::fs::create_dir_all(src_root.join("nested")).unwrap();
    std::fs::create_dir_all(&dst_root).unwrap();
    copy_fixture(
        "native/havok/tests/fixtures/skeleton.hkx",
        &src_root.join("nested/skeleton.hkx"),
    );
    std::fs::write(dst_root.join("ignored.hkx"), b"not an hkx").unwrap();

    let first = havok_native::api::havok_convert_ps4_batch(&src_root, &dst_root, true).unwrap();
    assert_eq!(first.converted, 1);
    assert_eq!(first.skipped, 0);
    assert!(first.errors.is_empty());
    let output = dst_root.join("nested/skeleton.hkx");
    assert!(output.exists());

    let second = havok_native::api::havok_convert_ps4_batch(&src_root, &dst_root, true).unwrap();
    assert_eq!(second.converted, 0);
    assert_eq!(second.skipped, 1);
    assert!(second.errors.is_empty());
}

#[test]
fn ps4_batch_can_replace_sources_without_processing_the_ps4_subtree() {
    let temp = clean_temp("havok_convert_ps4_replace_sources");
    let src_root = temp.join("src");
    let source = src_root.join("nested/skeleton.hkx");
    std::fs::create_dir_all(source.parent().unwrap()).unwrap();
    std::fs::create_dir_all(src_root.join("ps4")).unwrap();
    copy_fixture("native/havok/tests/fixtures/skeleton.hkx", &source);
    std::fs::write(src_root.join("ps4/ignored.hkx"), b"not an hkx").unwrap();

    let result = havok_native::api::havok_convert_ps4_batch(&src_root, &src_root, true).unwrap();
    assert_eq!(result.converted, 1);
    assert_eq!(result.skipped, 0);
    assert!(result.errors.is_empty());

    let header =
        havok_native::hkx::packfile::parse_header(&std::fs::read(source).unwrap()).unwrap();
    assert_eq!(header.reuse_padding_optimization, 1);
}
