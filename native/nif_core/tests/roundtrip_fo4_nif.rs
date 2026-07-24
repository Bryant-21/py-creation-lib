use std::fs;
use std::path::PathBuf;

use nif_core_native::io::{NifReader, NifWriter};
use nif_core_native::schema::NifSchema;

fn first_existing(candidates: &[PathBuf]) -> Option<PathBuf> {
    candidates.iter().find(|p| p.exists()).cloned()
}

fn ammo_generator_candidates() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Ok(p) = std::env::var("FO4_TEST_NIF") {
        v.push(PathBuf::from(p));
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    v.push(root.join("mods/B21_ArmCo/data/Meshes/ArmCo/AmmoGenerator.nif"));
    v.push(root.join("mods/B21_ArmCo/data/Meshes/ArmCo/AmmoConverter.nif"));
    v
}

fn extracted_fo4_dir() -> PathBuf {
    std::env::var("FO4_EXTRACTED_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../extracted/fo4")
        })
}

fn first_diff(a: &[u8], b: &[u8]) -> Option<usize> {
    let n = a.len().min(b.len());
    for i in 0..n {
        if a[i] != b[i] {
            return Some(i);
        }
    }
    if a.len() != b.len() { Some(n) } else { None }
}

fn assert_roundtrip(path: &PathBuf) {
    let original = fs::read(path).expect("read nif bytes");
    let schema = NifSchema::from_generated();
    let mut nif = NifReader::read(&original, &schema).expect("rust reader ok");
    let written = NifWriter::write_to_bytes(&mut nif, &schema).expect("rust writer ok");

    if original != written {
        let idx = first_diff(&original, &written).unwrap_or(0);
        let lo = idx.saturating_sub(16);
        let hi_a = (idx + 16).min(original.len());
        let hi_b = (idx + 16).min(written.len());
        eprintln!(
            "{}: roundtrip diff at byte {} (original len {}, written len {})",
            path.display(),
            idx,
            original.len(),
            written.len()
        );
        eprintln!("  orig [{}..{}] = {:02x?}", lo, hi_a, &original[lo..hi_a]);
        eprintln!("  writ [{}..{}] = {:02x?}", lo, hi_b, &written[lo..hi_b]);
    }

    assert_eq!(
        original.len(),
        written.len(),
        "{}: byte count mismatch: original {} vs written {}",
        path.display(),
        original.len(),
        written.len()
    );
    assert_eq!(
        original,
        written,
        "{}: roundtrip not byte-exact",
        path.display()
    );
}

#[test]
fn roundtrip_fo4_ammo_generator() {
    let Some(path) = first_existing(&ammo_generator_candidates()) else {
        eprintln!("skip: no FO4 test NIF available");
        return;
    };
    assert_roundtrip(&path);
}

#[test]
fn roundtrip_fo4_safe01() {
    let Some(path) =
        first_existing(&[extracted_fo4_dir().join("Meshes/SetDressing/Safe/Safe01.nif")])
    else {
        eprintln!("skip: Safe01.nif not extracted");
        return;
    };
    assert_roundtrip(&path);
}
