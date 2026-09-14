use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use nif_core_native::convert_file::{ConvertFileOptions, ConvertFileReport, convert_nif_file};

const CORPUS: &[(&str, &str)] = &[
    (
        "CM0040510F",
        "extracted/fo76/Meshes/SCOL/SeventySix.esm/CM0040510F.NIF",
    ),
    (
        "CM0084274B",
        "extracted/fo76/Meshes/SCOL/SeventySix.esm/CM0084274B.NIF",
    ),
    (
        "redrocketstatue_destroyed",
        "extracted/fo76/Meshes/atx/workshop/atx_redrocketstatue/atx_redrocketstatuepart1_destroyed.nif",
    ),
];

fn report_without_timings(report: &ConvertFileReport) -> serde_json::Value {
    let dependencies = report.final_dependencies.as_ref().map(|dependencies| {
        serde_json::json!({
            "digest": dependencies
                .digest
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>(),
            "materials": dependencies.materials,
        })
    });
    serde_json::json!({
        "supported": report.supported,
        "changes": report.changes,
        "warnings": report.warnings,
        "errors": report.errors,
        "emitted_bgsms": report.emitted_bgsms,
        "emitted_textures": report.emitted_textures,
        "emitted_first_person": report.emitted_first_person,
        "final_dependencies": dependencies,
        "shapes_skinned": report.shapes_skinned,
        "vertices_repacked": report.vertices_repacked,
        "bones_remapped": report.bones_remapped,
        "bones_dropped_unmapped": report.bones_dropped_unmapped,
        "weights_redistributed": report.weights_redistributed,
        "vertices_morph_weighted": report.vertices_morph_weighted,
    })
}

fn output_root(repo: &Path) -> PathBuf {
    std::env::var_os("NIF_HOTSPOT_BENCH_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo.join("tmp/nif_hotspot_audit_20260910/current"))
}

#[test]
#[ignore = "real FO76 corpus performance and byte-equivalence audit"]
fn fo76_conversion_hotspots_report_steps_and_match_baseline() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let output_root = output_root(&repo);
    let baseline_root = std::env::var_os("NIF_HOTSPOT_BASELINE_ROOT").map(PathBuf::from);
    fs::create_dir_all(&output_root).expect("create benchmark output directory");

    for (label, relative_source) in CORPUS {
        let source = repo.join(relative_source);
        assert!(
            source.is_file(),
            "missing benchmark input: {}",
            source.display()
        );
        let output = output_root.join(format!("{label}.nif"));
        let started = Instant::now();
        let report = convert_nif_file(
            &source,
            &output,
            "fo76",
            "fo4",
            None,
            &ConvertFileOptions::default(),
        )
        .unwrap_or_else(|error| panic!("{label}: conversion failed: {error}"));
        let wall_ms = started.elapsed().as_millis();
        assert!(report.supported, "{label}: conversion was unsupported");
        assert!(report.errors.is_empty(), "{label}: {:?}", report.errors);

        let stable_report = report_without_timings(&report);
        let report_path = output_root.join(format!("{label}.report.json"));
        fs::write(
            &report_path,
            serde_json::to_vec_pretty(&stable_report).expect("serialize stable report"),
        )
        .expect("write stable report");

        let mut timings = BTreeMap::<&str, u64>::new();
        for (step, elapsed_ms) in &report.timings_ms {
            *timings.entry(step).or_default() += elapsed_ms;
        }
        println!(
            "{}",
            serde_json::json!({
                "input": relative_source,
                "input_bytes": fs::metadata(&source).expect("source metadata").len(),
                "label": label,
                "output_bytes": fs::metadata(&output).expect("output metadata").len(),
                "step_ms": timings,
                "wall_ms": wall_ms,
            })
        );

        if let Some(baseline_root) = &baseline_root {
            let baseline_output = baseline_root.join(format!("{label}.nif"));
            let baseline_report = baseline_root.join(format!("{label}.report.json"));
            let actual = fs::read(&output).expect("read current output");
            let expected = fs::read(&baseline_output).expect("read baseline output");
            assert!(
                actual == expected,
                "{label}: converted bytes differ from baseline (current={}, baseline={})",
                blake3::hash(&actual),
                blake3::hash(&expected),
            );
            assert_eq!(
                stable_report,
                serde_json::from_slice::<serde_json::Value>(
                    &fs::read(&baseline_report).expect("read baseline report")
                )
                .expect("parse baseline report"),
                "{label}: report differs from baseline after excluding timings"
            );
        }
    }
}
