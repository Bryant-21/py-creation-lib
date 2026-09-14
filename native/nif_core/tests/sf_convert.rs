use nif_core_native::model::{NifFile, NifValue};
use nif_core_native::sf_convert::{SfConvertOptions, convert_starfield_nif};
use std::path::PathBuf;

fn starfield_extracted_dir() -> PathBuf {
    std::env::var_os("STARFIELD_EXTRACTED_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../extracted/starfield")
        })
}

fn assert_nifskope_compatible_shader_tail(nif: &NifFile) {
    for shader in nif
        .blocks
        .iter()
        .filter(|block| block.type_name == "BSLightingShaderProperty")
    {
        assert!(matches!(
            shader.get_field("Rimlight Power"),
            Some(NifValue::Float(value)) if *value == f32::MAX as f64
        ));
        assert!(matches!(
            shader.get_field("Backlight Power"),
            Some(NifValue::Float(value)) if *value == 0.0
        ));
    }
}

#[test]
fn converts_starfield_static_nif_to_fo4() {
    let root = starfield_extracted_dir();
    let src = root.join("meshes/setdressing/akila/ak_soccer/ak_soccer_ball_01.nif");
    if !src.exists() {
        eprintln!("skip: starfield extracted data not present");
        return;
    }
    let geometries_root = root.join("geometries");

    let dest_dir = std::env::temp_dir().join("nif_core_sf_convert_test");
    std::fs::create_dir_all(&dest_dir).expect("create dest dir");
    let dest = dest_dir.join("ak_soccer_ball_01_fo4.nif");

    let opts = SfConvertOptions {
        geometries_root,
        material_path_rewriter: None,
    };
    let report = convert_starfield_nif(&src, &dest, &opts).expect("converts");

    assert!(report.shapes >= 1);
    assert!(report.collision_decoded + report.collision_fallback + report.collision_none >= 1);

    let dest_nif = NifFile::load(dest.clone()).expect("dest parses as a NIF");
    assert_eq!(dest_nif.header.bs_version, 130);
    assert_eq!(dest_nif.header.user_version, 12);

    let trishape_count = dest_nif
        .blocks
        .iter()
        .filter(|block| block.type_name == "BSTriShape")
        .count();
    assert!(trishape_count >= 1, "expected at least one BSTriShape");

    let geometry_count = dest_nif
        .blocks
        .iter()
        .filter(|block| block.type_name == "BSGeometry")
        .count();
    assert_eq!(geometry_count, 0, "no BSGeometry blocks should remain");

    assert_nifskope_compatible_shader_tail(&dest_nif);
}

#[test]
fn converts_starborn_ship_with_nifskope_compatible_shader_tail() {
    let root = starfield_extracted_dir();
    let src = root.join("Meshes/Ships/Starborn/StarbornShipExt.nif");
    if !src.exists() {
        eprintln!("skip: Starfield extracted data not present");
        return;
    }
    let opts = SfConvertOptions {
        geometries_root: root.join("geometries"),
        material_path_rewriter: None,
    };
    let dest = std::env::temp_dir()
        .join("nif_core_sf_convert_test")
        .join("starborn_ship_ext_fo4.nif");
    std::fs::create_dir_all(dest.parent().unwrap()).expect("create dest dir");

    let report = convert_starfield_nif(&src, &dest, &opts).expect("converts Starborn ship");
    assert_eq!(report.shapes, 22);

    let converted = NifFile::load(dest).expect("converted ship parses as a NIF");
    assert_nifskope_compatible_shader_tail(&converted);
}
