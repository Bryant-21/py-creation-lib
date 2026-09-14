use std::path::Path;
use std::time::Instant;

use nif_core_native::io::NifReader;
use nif_core_native::model::NifFile;
use nif_core_native::schema::SCHEMA;

const CORPUS: &[(&str, &str)] = &[
    (
        "fo4-sized",
        "extracted/fo4/Meshes/SetDressing/Vault/Vault_Cart_01.nif",
    ),
    (
        "fo76-sized-large",
        "extracted/fo76/Meshes/SCOL/SeventySix.esm/CM0040510F.NIF",
    ),
    (
        "fo76-sized-small",
        "extracted/fo76/Meshes/atx/workshop/atx_redrocketstatue/atx_redrocketstatuepart1_destroyed.nif",
    ),
    (
        "skyrimse-sized",
        "extracted/skyrimse/meshes/actors/draugr/character assets/draugrmale.nif",
    ),
    (
        "fnv",
        "extracted/fnv/meshes/architecture/wasteland/powerstationlow.nif",
    ),
];

#[test]
#[ignore = "real-corpus dependency reader timing and equivalence audit"]
fn selective_dependency_reader_matches_full_nif_load() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    for (label, relative) in CORPUS {
        let path = repo.join(relative);
        assert!(path.is_file(), "missing corpus NIF: {}", path.display());

        let uses_selective_reader = NifReader::read_referenced_asset_paths(
            std::fs::File::open(&path).expect("open corpus NIF"),
            &SCHEMA,
        )
        .expect("inspect reader eligibility")
        .is_some();
        let mut full_ms = Vec::new();
        let mut selective_ms = Vec::new();
        let mut final_refs = None;
        for selective_first in [true, false, true, false] {
            let read_full = || {
                let started = Instant::now();
                let refs = NifFile::load(&path)
                    .expect("full parse")
                    .referenced_asset_paths();
                (refs, started.elapsed().as_secs_f64() * 1_000.0)
            };
            let read_selective = || {
                let started = Instant::now();
                let refs = NifFile::load_referenced_asset_paths(&path).expect("selective parse");
                (refs, started.elapsed().as_secs_f64() * 1_000.0)
            };
            let ((full, full_elapsed), (selective, selective_elapsed)) = if selective_first {
                let selective = read_selective();
                let full = read_full();
                (full, selective)
            } else {
                let full = read_full();
                let selective = read_selective();
                (full, selective)
            };
            assert_eq!(selective, full, "dependency set differs for {relative}");
            full_ms.push(full_elapsed);
            selective_ms.push(selective_elapsed);
            final_refs = Some(selective);
        }
        let final_refs = final_refs.expect("benchmark samples");
        println!(
            "{}",
            serde_json::json!({
                "path": relative,
                "label": label,
                "input_bytes": std::fs::metadata(&path).expect("metadata").len(),
                "uses_selective_reader": uses_selective_reader,
                "full_ms": full_ms,
                "selective_ms": selective_ms,
                "materials": final_refs.materials.len(),
                "textures": final_refs.textures.len(),
            })
        );
    }
}
