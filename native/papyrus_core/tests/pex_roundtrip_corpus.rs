//! Byte round-trip over a real .pex corpus. Skips unless PAPYRUS_PEX_CORPUS is set
//! to a directory of .pex files (e.g. a decompiled FO76 dump or FO4 Base scripts).
use std::path::PathBuf;

#[test]
fn corpus_round_trips_byte_identical() {
    let Ok(dir) = std::env::var("PAPYRUS_PEX_CORPUS") else {
        eprintln!("PAPYRUS_PEX_CORPUS not set — skipping corpus round-trip");
        return;
    };
    let mut checked = 0usize;
    let mut mismatches: Vec<String> = Vec::new();
    let mut stack = vec![PathBuf::from(dir)];
    while let Some(p) = stack.pop() {
        if p.is_dir() {
            for e in std::fs::read_dir(&p).unwrap() {
                stack.push(e.unwrap().path());
            }
            continue;
        }
        if p.extension().and_then(|e| e.to_str()) != Some("pex") {
            continue;
        }
        let original = std::fs::read(&p).unwrap();
        let payload = match papyrus_core::pex::parse_pex_bytes(&original) {
            Ok(pl) => pl,
            Err(e) => {
                mismatches.push(format!("{}: parse error {e}", p.display()));
                continue;
            }
        };
        // PC files are little-endian; the writer targets LE. Skip big-endian console files.
        if original.get(0..4) != Some(&papyrus_core::pex::PEX_MAGIC.to_le_bytes()) {
            continue;
        }
        match papyrus_core::pex_writer::write_pex_bytes(&payload) {
            Ok(written) if written == original => {
                checked += 1;
            }
            Ok(written) => mismatches.push(format!(
                "{}: byte mismatch (orig {} bytes, ours {} bytes, first diff @ {})",
                p.display(),
                original.len(),
                written.len(),
                written
                    .iter()
                    .zip(&original)
                    .position(|(a, b)| a != b)
                    .map(|i| i.to_string())
                    .unwrap_or_else(|| "len".into())
            )),
            Err(e) => mismatches.push(format!("{}: write error {e}", p.display())),
        }
    }
    eprintln!(
        "corpus round-trip: {checked} files byte-identical, {} mismatches",
        mismatches.len()
    );
    assert!(
        mismatches.is_empty(),
        "byte-diff mismatches:\n{}",
        mismatches.join("\n")
    );
}
