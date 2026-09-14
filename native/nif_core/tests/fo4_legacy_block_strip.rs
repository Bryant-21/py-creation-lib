//! FNV/FO3 -> FO4 conversions must not ship a block type the runtime cannot
//! instantiate. FO4 resolves blocks by RTTI name, so one unknown type fails the
//! whole file load and the engine draws the red "!" marker instead of the mesh
//! -- with no conversion error and nothing wrong on the record side.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use nif_core_native::convert_file::{ConvertFileOptions, convert_nif_file};
use nif_core_native::fo4_block_types::is_fo4_block_type;
use nif_core_native::model::NifFile;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn translation_maps_dir() -> PathBuf {
    repo_root().join("bacup/py_bacup_lib/native/conversion/src/embedded/translation_maps")
}

/// One real source per legacy block type that has no FO4 RTTI entry. Between
/// them these 16 files cover all 32 types measured in the shipped conversion.
const LEGACY_BLOCK_COVERAGE: &[&str] = &[
    "extracted/fnv/Meshes/DLC04/Weapons/2HandMelee/DLC04Axe.NIF",
    "extracted/fnv/Meshes/Armor/1950StyleCasual01/F/Outfit.NIF",
    "extracted/fnv/Meshes/architecture/NoVac/Bungalow_windowExt.NIF",
    "extracted/fnv/Meshes/scol/scolbld06georgetown01.nif",
    "extracted/fnv/Meshes/Characters/_Male/Skeleton.NIF",
    "extracted/fnv/Meshes/animobjects/hchamberFXLSO.NIF",
    "extracted/fnv/Meshes/Armor/AdvancedPowerArmor/AdPowerArmor.NIF",
    "extracted/fnv/Meshes/DLCPitt/Architecture/BlastFurnace/DLCPittBlastFurnace03.NIF",
    "extracted/fnv/Meshes/Characters/_Male/M01.NIF",
    "extracted/fnv/Meshes/DLC03/Dungeons/PresidentialMetro/PrMetroColCab/DLC03PrMetroColCab01.NIF",
    "extracted/fnv/Meshes/Water_Placeable/LamplightMainCaveWater.NIF",
    "extracted/fnv/Meshes/DLC05/Traps/DLC05CrushingMachineLong.NIF",
    "extracted/fnv/Meshes/Armor/1950StyleCasual01/M/GO.NIF",
    "extracted/fnv/Meshes/Water_Placeable/PoolWastelandWater01.NIF",
    "extracted/fnv/Meshes/NVDLC03/creatures/spidermine/skeleton.nif",
    "extracted/fnv/Meshes/DLC03/Effects/DLC03TeslaChainStep01.NIF",
];

#[test]
fn every_legacy_block_class_converts_to_a_loadable_fo4_nif() {
    let maps = translation_maps_dir();
    let mut checked = 0usize;
    let mut offenders = Vec::new();
    for relative in LEGACY_BLOCK_COVERAGE {
        let src = repo_root().join(relative);
        if !src.exists() {
            continue;
        }
        let dir = temp_dir("coverage");
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let dst = dir.join("out").join("converted.nif");
        let options = ConvertFileOptions {
            // Production sets this for every fnv/fo3 -> fo4 run, and the armor
            // skin path is inert without it.
            translation_maps_dir: Some(maps.clone()),
            ..ConvertFileOptions::default()
        };
        let report = convert_nif_file(&src, &dst, "fnv", "fo4", None, &options)
            .unwrap_or_else(|error| panic!("{relative}: convert failed: {error}"));
        if !report.errors.is_empty() {
            offenders.push(format!("{relative}: {:?}", report.errors));
            let _ = std::fs::remove_dir_all(&dir);
            continue;
        }
        let converted = NifFile::load(dst).expect("load converted nif");
        let unsupported: Vec<&str> = converted
            .blocks
            .iter()
            .map(|block| block.type_name.as_str())
            .filter(|type_name| !is_fo4_block_type(type_name))
            .collect();
        if !unsupported.is_empty() {
            offenders.push(format!("{relative}: {unsupported:?}"));
        }
        let _ = std::fs::remove_dir_all(&dir);
        checked += 1;
    }
    if checked == 0 {
        eprintln!("skipping: extracted/fnv not present");
        return;
    }
    assert!(
        offenders.is_empty(),
        "{checked} converted, but these still fail FO4 load: {offenders:#?}"
    );
}

fn temp_dir(name: &str) -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "nif_core_legacy_strip_{name}_{}_{}",
        std::process::id(),
        suffix
    ))
}

/// Convert `relative_source` and return the block types the output still holds
/// that FO4 has no RTTI entry for. Returns `None` when the source is not
/// extracted locally, so the test skips instead of failing.
fn unsupported_types_after_convert(relative_source: &str, name: &str) -> Option<Vec<String>> {
    let src = repo_root().join(relative_source);
    if !src.exists() {
        return None;
    }
    let dir = temp_dir(name);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir.join("out").join("converted.nif");

    let report = convert_nif_file(
        &src,
        &dst,
        "fnv",
        "fo4",
        None,
        &ConvertFileOptions::default(),
    )
    .expect("convert nif");
    assert!(
        report.errors.is_empty(),
        "conversion reported errors: {:?}",
        report.errors
    );
    assert!(report.supported, "conversion reported unsupported");

    let converted = NifFile::load(dst).expect("load converted nif");
    let _ = std::fs::remove_dir_all(&dir);
    Some(
        converted
            .blocks
            .iter()
            .map(|block| block.type_name.clone())
            .filter(|type_name| !is_fo4_block_type(type_name))
            .collect(),
    )
}

#[test]
fn buffalo_gourd_drops_refraction_controllers_fo4_cannot_load() {
    // The record (15F0EF BuffaloGourdPickable), its MODL path, geometry and
    // materials were all correct; the file failed to load purely on its
    // BSRefractionFirePeriodController / BSRefractionStrengthController blocks.
    let Some(unsupported) = unsupported_types_after_convert(
        "extracted/fnv/Meshes/landscape/plants/NVBuffaloGourd_Fruited.NIF",
        "gourd",
    ) else {
        eprintln!("skipping: extracted/fnv not present");
        return;
    };
    assert!(
        unsupported.is_empty(),
        "converted gourd still holds FO4-unloadable blocks: {unsupported:?}"
    );
}

#[test]
fn laser_tripwire_drops_legacy_havok_fo4_cannot_load() {
    // Legacy bhk* collision the FO4 rebuild path does not consume.
    let Some(unsupported) = unsupported_types_after_convert(
        "extracted/fnv/Meshes/Traps/LaserTripwire01-256.NIF",
        "trip",
    ) else {
        eprintln!("skipping: extracted/fnv not present");
        return;
    };
    assert!(
        unsupported.is_empty(),
        "converted tripwire still holds FO4-unloadable blocks: {unsupported:?}"
    );
}
