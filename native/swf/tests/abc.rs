//! A3 evidence gate: parse the ABC constant-pool string table of the FO4 menu
//! SWFs and check whether every SymbolClass export name also appears as an AS3
//! identifier. If it does, each marker symbol is backed by a class (so injected
//! FO76 symbols may need a synthesized backing class); if not, the engine resolves
//! markers purely by SymbolClass name (SymbolClass-only injection is sufficient).

use std::collections::HashSet;
use std::path::PathBuf;

use swf_native::abc::{DO_ABC, DO_ABC_DEFINE, parse_abc_strings};
use swf_native::container::{decompress, split_tags};
use swf_native::symbolclass::parse_symbol_table;

fn extracted(rel: &str) -> Vec<u8> {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("../../../extracted");
    p.push(rel);
    std::fs::read(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
}

fn abc_strings(body: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    for s in split_tags(body).unwrap() {
        if s.code == DO_ABC_DEFINE || s.code == DO_ABC {
            out.extend(
                parse_abc_strings(s.code, &body[s.body_range()])
                    .unwrap()
                    .strings,
            );
        }
    }
    out
}

fn symbol_names(body: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    for s in split_tags(body).unwrap() {
        if s.code == 76 {
            for e in parse_symbol_table(&body[s.body_range()]).unwrap() {
                out.push(e.name);
            }
        }
    }
    out
}

#[test]
fn fo4_marker_symbols_are_class_backed() {
    let movie = decompress(&extracted("fo4/Interface/MapMarkers.swf")).unwrap();
    let strings: HashSet<String> = abc_strings(&movie.body).into_iter().collect();
    let symbols = symbol_names(&movie.body);
    assert!(!symbols.is_empty(), "no SymbolClass exports parsed");

    // Self-validation + evidence: every SymbolClass export name resolves to an AS3
    // identifier in the ABC string pool (a correctly-parsed pool *and* proof the
    // markers are backed 1:1 by classes).
    let missing: Vec<&String> = symbols.iter().filter(|n| !strings.contains(*n)).collect();
    assert!(
        missing.is_empty(),
        "{} of {} SymbolClass exports absent from ABC string pool: {:?}",
        missing.len(),
        symbols.len(),
        missing
    );
}

#[test]
fn abc_string_pool_parses_for_all_marker_swfs() {
    for rel in [
        "fo4/Interface/MapMarkers.swf",
        "fo4/Interface/HUDMenu.swf",
        "fo4/Interface/Pipboy_MapPage.swf",
    ] {
        let movie = decompress(&extracted(rel)).unwrap();
        let strings = abc_strings(&movie.body);
        assert!(
            !strings.is_empty(),
            "{rel}: empty ABC string pool (parse likely wrong)"
        );
    }
}
