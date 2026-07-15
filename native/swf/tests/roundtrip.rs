//! Byte-identity gate: the splitter must tile each real menu SWF exactly and
//! SymbolClass tags must survive parse→encode unchanged. This is the make-or-
//! break foundation for marker injection (untouched tags spliced as opaque
//! bytes).

use std::path::PathBuf;

use swf_native::container::{decompress, split_tags, tags_offset};
use swf_native::symbolclass::{encode_symbol_table, parse_symbol_table};

fn extracted(rel: &str) -> Vec<u8> {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("../../../extracted");
    p.push(rel);
    std::fs::read(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
}

fn check(rel: &str) -> (usize, usize) {
    let raw = extracted(rel);
    let movie = decompress(&raw).expect("decompress");
    let spans = split_tags(&movie.body).expect("split_tags");

    // Tags must tile [tags_offset .. body.len()] contiguously and end on End(0).
    let off = tags_offset(&movie.body).expect("tags_offset");
    let mut cursor = off;
    for s in &spans {
        assert_eq!(s.start, cursor, "{rel}: tag {} not contiguous", s.code);
        cursor = s.end();
    }
    assert_eq!(
        spans.last().expect("at least one tag").code,
        0,
        "{rel}: does not end on End tag"
    );
    assert_eq!(cursor, movie.body.len(), "{rel}: leftover bytes after tags");

    // SymbolClass (76) parse→encode byte-identity.
    let mut symbols = 0usize;
    for s in &spans {
        if s.code == 76 {
            let body = &movie.body[s.body_range()];
            let entries = parse_symbol_table(body).expect("parse SymbolClass");
            assert_eq!(
                encode_symbol_table(&entries),
                body,
                "{rel}: SymbolClass not byte-identical"
            );
            symbols += entries.len();
        }
    }
    (spans.len(), symbols)
}

#[test]
fn roundtrip_marker_swfs() {
    let cases = [
        "fo76/interface/mapmarkerlibrary.swf",
        "fo4/Interface/MapMarkers.swf",
        "fo4/Interface/Pipboy_MapPage.swf",
        "fo4/Interface/HUDMenu.swf",
    ];
    for rel in cases {
        let (tags, symbols) = check(rel);
        println!("{rel}: {tags} tags, {symbols} SymbolClass entries");
        assert!(symbols > 0, "{rel}: no SymbolClass entries found");
    }
}
