use std::path::{Path, PathBuf};

use nif_core_native::convert_file::{ConvertFileOptions, convert_nif_file};
use nif_core_native::model::{NifBlock, NifFile, NifValue};
use nif_core_native::schema::NifSchema;

const GUITAR: &str = "AnimObjectAcousticGuitar.nif";
const MALLET: &str = "AnimObject_ATX_AlienWhackAMole_Mallet.nif";

fn source_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../extracted/fo76/Meshes/AnimObjects")
        .join(name)
}

fn interpolator(nif: &NifFile) -> &NifBlock {
    let controller = nif.blocks[0].get_field("Controller").unwrap().as_usize();
    let interpolator = nif.blocks[controller]
        .get_field("Interpolator")
        .unwrap()
        .as_usize();
    &nif.blocks[interpolator]
}

fn component(nif: &NifFile, field: &str, axis: &str) -> f64 {
    let Some(NifValue::Struct(transform)) = interpolator(nif).get_field("Transform") else {
        panic!("missing transform");
    };
    let NifValue::Struct(value) = &transform[field] else {
        panic!("missing component")
    };
    let NifValue::Float(value) = value[axis] else {
        panic!("missing axis")
    };
    value
}

fn convert(source: &Path, target: &Path, repaired: bool) -> NifFile {
    let report = convert_nif_file(
        source,
        target,
        "fo76",
        "fo4",
        None,
        &ConvertFileOptions::default(),
    )
    .expect("convert held prop");
    assert!(
        report.supported && report.errors.is_empty(),
        "{:?}",
        report.errors
    );
    assert_eq!(
        report
            .changes
            .iter()
            .any(|change| change.starts_with("Pinned FO76 held-prop")),
        repaired
    );
    let nif = NifFile::load(target).expect("reload converted prop");
    let schema = NifSchema::from_generated();
    for block in &nif.blocks {
        for reference in block.get_refs(&schema) {
            assert!(reference < 0 || (reference as usize) < nif.blocks.len());
        }
    }
    nif
}

#[test]
fn guitar_conversion_preserves_visibility_and_removes_confirmed_offset() {
    let source = source_path(GUITAR);
    if !source.exists() {
        eprintln!("missing fixture {}", source.display());
        return;
    }
    let before = NifFile::load(&source).unwrap();
    assert!((component(&before, "Translation", "z") + 29.392746).abs() < 0.001);
    let dir = tempfile::tempdir().unwrap();
    let after = convert(&source, &dir.path().join(GUITAR), true);
    assert_eq!(before.blocks.len(), after.blocks.len());
    for axis in ["x", "y", "z"] {
        assert_eq!(component(&after, "Translation", axis), 0.0);
    }
    for axis in ["w", "x", "y", "z"] {
        assert_eq!(
            component(&before, "Rotation", axis),
            component(&after, "Rotation", axis)
        );
    }
    for (before, after) in before.blocks.iter().zip(&after.blocks) {
        if before.type_name.starts_with("NiVis") || before.type_name.starts_with("NiBool") {
            assert_eq!(before.fields, after.fields);
        }
    }
}

#[test]
fn mallet_conversion_uses_hand_motion_and_preserves_geometry_offset() {
    let source = source_path(MALLET);
    if !source.exists() {
        eprintln!("missing fixture {}", source.display());
        return;
    }
    let before = NifFile::load(&source).unwrap();
    assert!(interpolator(&before).get_field("Data").unwrap().as_i64() >= 0);
    let dir = tempfile::tempdir().unwrap();
    let after = convert(&source, &dir.path().join(MALLET), true);
    assert_eq!(interpolator(&after).get_field("Data").unwrap().as_i64(), -1);
    for axis in ["x", "y", "z"] {
        assert_eq!(component(&after, "Translation", axis), 0.0);
        assert_eq!(component(&after, "Rotation", axis), 0.0);
    }
    assert_eq!(component(&after, "Rotation", "w"), 1.0);
    assert!(
        !after
            .blocks
            .iter()
            .any(|block| block.type_name == "NiTransformData")
    );
    let shape_before = before
        .blocks
        .iter()
        .find(|block| block.type_name == "BSTriShape")
        .unwrap();
    let shape_after = after
        .blocks
        .iter()
        .find(|block| block.type_name == "BSTriShape")
        .unwrap();
    for field in [
        "Translation",
        "Rotation",
        "Scale",
        "Vertex Data",
        "Triangles",
    ] {
        assert_eq!(
            shape_before.get_field(field),
            shape_after.get_field(field),
            "{field}"
        );
    }
}

#[test]
fn unrelated_prop_and_wrong_attachment_keep_their_motion() {
    let source = source_path(MALLET);
    if !source.exists() {
        eprintln!("missing fixture {}", source.display());
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let renamed = dir.path().join("UnrelatedAnimObject.nif");
    std::fs::copy(&source, &renamed).unwrap();
    let unchanged = convert(&renamed, &dir.path().join("unrelated-output.nif"), false);
    assert!(interpolator(&unchanged).get_field("Data").unwrap().as_i64() >= 0);

    let mut wrong_attachment = NifFile::load(&source).unwrap();
    let prn = wrong_attachment
        .blocks
        .iter_mut()
        .find(|block| block.type_name == "NiStringExtraData")
        .unwrap();
    prn.set_field("String Data", NifValue::String("Weapon".into()));
    let path = dir.path().join(MALLET);
    wrong_attachment.save(Some(path.clone())).unwrap();
    let unchanged = convert(
        &path,
        &dir.path().join("wrong-attachment-output.nif"),
        false,
    );
    assert!(interpolator(&unchanged).get_field("Data").unwrap().as_i64() >= 0);
}
