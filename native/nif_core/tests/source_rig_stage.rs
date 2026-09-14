use std::path::PathBuf;

use nif_core_native::convert_file::{
    ConvertFileError, ConvertFileOptions, SourceRigNifKind, SourceRigNifOutputProvenance,
    stage_preserve_source_rig_nif,
};
use nif_core_native::model::NifFile;
use nif_core_native::skin::LegacySkinPolicy;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

#[test]
fn source_rig_stage_rejects_nonrelative_runtime_paths_before_conversion() {
    let temp = tempfile::tempdir().expect("tempdir");
    let error = stage_preserve_source_rig_nif(
        SourceRigNifKind::Body,
        &temp.path().join("missing.nif"),
        &temp.path().join("body.nif"),
        r"C:\Meshes\Actors\Body.nif",
        "skyrimse",
        None,
        &ConvertFileOptions::default(),
    )
    .expect_err("absolute target-relative path");

    assert!(matches!(
        error,
        ConvertFileError::InvalidSourceRigTargetPath { .. }
    ));
}

#[test]
fn optional_real_wolf_body_and_skeleton_stage_with_explicit_source_rig_policy() {
    let source_dir =
        repo_root().join("extracted/skyrimse/meshes/actors/canine/character assets wolf");
    let body = source_dir.join("wolf.nif");
    let skeleton = source_dir.join("skeleton.nif");
    if !body.exists() || !skeleton.exists() {
        eprintln!("skipping optional real wolf source-rig staging fixture");
        return;
    }

    let temp = tempfile::tempdir().expect("tempdir");
    let options = ConvertFileOptions {
        skin_policy: LegacySkinPolicy::TranslateSkeleton,
        ..ConvertFileOptions::default()
    };
    let body_target = temp.path().join("body.nif");
    let body_receipt = stage_preserve_source_rig_nif(
        SourceRigNifKind::Body,
        &body,
        &body_target,
        r"Meshes\Actors\SourceRig\Wolf\Body.NIF",
        "skyrimse",
        None,
        &options,
    )
    .expect("stage body");
    assert_eq!(body_receipt.kind, SourceRigNifKind::Body);
    assert_eq!(body_receipt.source_path, body);
    assert_eq!(body_receipt.staged_target_path, body_target);
    assert_eq!(
        body_receipt.output_provenance,
        SourceRigNifOutputProvenance::PreserveSourceRigConversion
    );
    assert_eq!(
        body_receipt.target_relative_path,
        "meshes/actors/sourcerig/wolf/body.nif"
    );
    assert!(body_receipt.output_len > 0);
    assert_ne!(body_receipt.output_fingerprint, 0);
    assert!(body_receipt.report.supported);
    assert!(body_receipt.report.shapes_skinned > 0);
    let staged_body = NifFile::load(body_target).expect("load staged body");
    assert!(
        staged_body
            .blocks
            .iter()
            .any(|block| block.type_name == "BSSkin::Instance")
    );

    let skeleton_target = temp.path().join("skeleton.nif");
    let skeleton_receipt = stage_preserve_source_rig_nif(
        SourceRigNifKind::Skeleton,
        &skeleton,
        &skeleton_target,
        r"Meshes\Actors\SourceRig\Wolf\Skeleton.NIF",
        "skyrimse",
        None,
        &options,
    )
    .expect("stage skeleton");
    assert_eq!(skeleton_receipt.kind, SourceRigNifKind::Skeleton);
    assert_eq!(skeleton_receipt.source_path, skeleton);
    assert_eq!(skeleton_receipt.staged_target_path, skeleton_target);
    assert!(skeleton_receipt.report.supported);
    NifFile::load(skeleton_target).expect("load staged skeleton");
}
