use std::hint::black_box;
use std::path::Path;
use std::time::{Duration, Instant};

use nif_core_native::model::NifFile;
use nif_core_native::model::NifValue;

const CORPUS: &[(&str, &str)] = &[
    ("fo4", "extracted/fo4/Meshes/SetDressing/Vault/Vault_Cart_01.nif"),
    ("fo76-large", "extracted/fo76/Meshes/SCOL/SeventySix.esm/CM0040510F.NIF"),
    ("fo76-statue", "extracted/fo76/Meshes/atx/workshop/atx_redrocketstatue/atx_redrocketstatuepart1_destroyed.nif"),
    ("skyrimse", "extracted/skyrimse/meshes/actors/draugr/character assets/draugrmale.nif"),
    ("fnv", "extracted/fnv/meshes/architecture/wasteland/powerstationlow.nif"),
];

fn assert_same_decode(lossless: &NifFile, lean: &NifFile) {
    assert_eq!(format!("{:?}", lean.header), format!("{:?}", lossless.header));
    assert_eq!(lean.blocks.len(), lossless.blocks.len());
    for (lean_block, lossless_block) in lean.blocks.iter().zip(&lossless.blocks) {
        assert_eq!(lean_block.block_id, lossless_block.block_id);
        assert_eq!(lean_block.type_name, lossless_block.type_name);
        assert_eq!(lean_block.fields, lossless_block.fields);
        assert_eq!(lean_block.remainder, lossless_block.remainder);
        assert!(lean_block.original_bytes.is_none());
        assert!(lean_block.original_content_hash.is_none());
    }
    assert_eq!(lean.raw_block_context, lossless.raw_block_context);
    assert_eq!(lean.referenced_asset_paths(), lossless.referenced_asset_paths());
}

fn timed_parse(path: &Path, lean: bool) -> (NifFile, f64, f64, usize) {
    let read_started = Instant::now();
    let bytes = std::fs::read(path).expect("read fixture");
    let read_ms = read_started.elapsed().as_secs_f64() * 1_000.0;
    let parse_started = Instant::now();
    let nif = if lean {
        NifFile::from_bytes_lean(&bytes, Some(path.to_owned())).expect("lean parse")
    } else {
        NifFile::from_bytes(&bytes, Some(path.to_owned())).expect("lossless parse")
    };
    let parse_ms = parse_started.elapsed().as_secs_f64() * 1_000.0;
    let retained = nif
        .blocks
        .iter()
        .filter_map(|block| block.original_bytes.as_ref())
        .map(Vec::len)
        .sum();
    (nif, read_ms, parse_ms, retained)
}

#[test]
#[ignore = "real-corpus lossless versus lean full-reader timing and parity"]
fn lean_reader_matches_lossless_corpus() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    for (label, relative) in CORPUS {
        let path = repo.join(relative);
        assert!(path.is_file(), "missing fixture: {}", path.display());
        for iteration in 0..4 {
            let lean_first = iteration % 2 == 0;
            let ((lossless, lossless_read, lossless_parse, retained), (lean, lean_read, lean_parse, lean_retained)) =
                if lean_first {
                    let lean = timed_parse(&path, true);
                    let lossless = timed_parse(&path, false);
                    (lossless, lean)
                } else {
                    let lossless = timed_parse(&path, false);
                    let lean = timed_parse(&path, true);
                    (lossless, lean)
                };
            println!(
                "{}",
                serde_json::json!({
                    "label": label,
                    "path": relative,
                    "iteration": iteration,
                    "lean_first": lean_first,
                    "input_bytes": std::fs::metadata(&path).expect("metadata").len(),
                    "lossless_read_ms": lossless_read,
                    "lean_read_ms": lean_read,
                    "lossless_parse_ms": lossless_parse,
                    "lean_parse_ms": lean_parse,
                    "lossless_original_bytes": retained,
                    "lean_original_bytes": lean_retained,
                })
            );
            assert_same_decode(&lossless, &lean);
            assert!(retained > 0);
            assert_eq!(lean_retained, 0);
            drop((lossless, lean));
        }
    }
}

#[test]
#[ignore = "fresh-process large-NIF peak-memory probe"]
fn lean_reader_large_memory_probe() {
    let mode = std::env::var("B21_NIF_LEAN_MEMORY_MODE").expect("memory mode");
    let path = std::env::var("B21_NIF_LEAN_MEMORY_PATH").expect("memory path");
    let bytes = std::fs::read(&path).expect("read fixture");
    let nif = match mode.as_str() {
        "lossless" => NifFile::from_bytes(&bytes, Some(path.into())).expect("lossless parse"),
        "lean" => NifFile::from_bytes_lean(&bytes, Some(path.into())).expect("lean parse"),
        _ => panic!("mode must be lossless or lean"),
    };
    println!("memory_probe_ready mode={mode} blocks={}", nif.blocks.len());
    black_box(&nif);
    std::thread::sleep(Duration::from_secs(3));
}

#[test]
#[ignore = "real-corpus IndexMap struct capacity inventory"]
fn lean_reader_struct_capacity_inventory() {
    fn visit(value: &NifValue, counts: &mut std::collections::BTreeMap<(usize, usize), u64>) {
        match value {
            NifValue::Struct(fields) => {
                *counts.entry((fields.len(), fields.capacity())).or_default() += 1;
                for value in fields.values() {
                    visit(value, counts);
                }
            }
            NifValue::Array(values) => {
                for value in values {
                    visit(value, counts);
                }
            }
            _ => {}
        }
    }
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    for (label, relative) in CORPUS {
        let path = repo.join(relative);
        let bytes = std::fs::read(&path).expect("read fixture");
        let nif = NifFile::from_bytes_lean(&bytes, Some(path)).expect("lean parse");
        let mut counts = std::collections::BTreeMap::new();
        for block in &nif.blocks {
            for value in block.fields.values() {
                visit(value, &mut counts);
            }
        }
        println!("{}", serde_json::json!({"label":label,"len_capacity_counts":counts.iter().map(|((len,capacity),count)| serde_json::json!({"len":len,"capacity":capacity,"count":count})).collect::<Vec<_>>() }));
    }
}
