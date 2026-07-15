//! Inventory dump (run with `--ignored`): prints the SymbolClass export list
//! for the marker SWFs so the FO76→FO4 icon assignment table can be authored
//! from the real symbol order. Not part of the byte-identity gate.

use std::path::PathBuf;

use swf_native::container::{decompress, split_tags};
use swf_native::symbolclass::parse_symbol_table;

fn extracted(rel: &str) -> Vec<u8> {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("../../../extracted");
    p.push(rel);
    std::fs::read(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
}

fn dump(rel: &str) {
    let raw = extracted(rel);
    let movie = decompress(&raw).expect("decompress");
    let spans = split_tags(&movie.body).expect("split_tags");
    println!("==== {rel} ====");
    for s in &spans {
        if s.code == 76 {
            for e in parse_symbol_table(&movie.body[s.body_range()]).expect("symtab") {
                println!("{}\t{}", e.character_id, e.name);
            }
        }
    }
}

#[test]
#[ignore]
fn dump_fo76_marker_library() {
    dump("fo76/interface/mapmarkerlibrary.swf");
}

#[test]
#[ignore]
fn dump_fo4_marker_swfs() {
    dump("fo4/Interface/MapMarkers.swf");
}
