use nif_core_native::convert_file::{
    ConvertFileError, ConvertFileOptions, SourceRigNifKind, stage_preserve_source_rig_nif,
};

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

