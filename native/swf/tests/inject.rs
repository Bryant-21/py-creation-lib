//! A2 gate: inject a FO76-only marker symbol into the FO4 MapMarkers SWF and
//! prove (a) the new export appears, (b) untouched destination bytes are
//! preserved exactly around the splice, and (c) the result re-parses cleanly
//! and survives a full assemble→decompress cycle.

use std::path::PathBuf;

use swf_native::container::{assemble, decompress, split_tags};
use swf_native::inject::inject_symbols;
use swf_native::symbolclass::parse_symbol_table;

fn extracted(rel: &str) -> Vec<u8> {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("../../../extracted");
    p.push(rel);
    std::fs::read(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
}

fn symbol_names(body: &[u8]) -> Vec<String> {
    let mut names = Vec::new();
    for s in split_tags(body).unwrap() {
        if s.code == 76 {
            for e in parse_symbol_table(&body[s.body_range()]).unwrap() {
                names.push(e.name);
            }
        }
    }
    names
}

#[test]
fn inject_fo76_marker_into_fo4() {
    let src = decompress(&extracted("fo76/interface/mapmarkerlibrary.swf")).unwrap();
    let dst = decompress(&extracted("fo4/Interface/MapMarkers.swf")).unwrap();

    let before = symbol_names(&dst.body);
    assert!(!before.iter().any(|n| n == "Vault63Marker"));

    let out = inject_symbols(&src, &dst, &["Vault63Marker"]).unwrap();

    // (a) the new export is present, exactly one added.
    let after = symbol_names(&out.body);
    assert_eq!(after.len(), before.len() + 1);
    assert!(after.iter().any(|n| n == "Vault63Marker"));

    // (b) untouched destination bytes preserved around the splice. New defines
    // and the rewritten SymbolClass were inserted at the last SymbolClass tag;
    // everything before it and after it must be byte-identical.
    let dst_spans = split_tags(&dst.body).unwrap();
    let sc = *dst_spans
        .iter()
        .filter(|s| s.code == 76)
        .next_back()
        .unwrap();
    assert_eq!(
        &out.body[..sc.start],
        &dst.body[..sc.start],
        "prefix changed"
    );
    let dst_tail = &dst.body[sc.end()..];
    assert_eq!(
        &out.body[out.body.len() - dst_tail.len()..],
        dst_tail,
        "suffix changed"
    );

    // (c) result tiles cleanly and ends on End.
    let out_spans = split_tags(&out.body).unwrap();
    assert_eq!(out_spans.last().unwrap().code, 0);
    assert_eq!(out_spans.last().unwrap().end(), out.body.len());

    // (d) full assemble → decompress → re-list cycle is stable.
    let bytes = assemble(out.signature, out.version, &out.body).unwrap();
    let reparsed = decompress(&bytes).unwrap();
    assert_eq!(symbol_names(&reparsed.body), after);
}

#[test]
fn inject_many_dedups_shared_art() {
    let src = decompress(&extracted("fo76/interface/mapmarkerlibrary.swf")).unwrap();
    let dst = decompress(&extracted("fo4/Interface/MapMarkers.swf")).unwrap();

    let names = ["Vault63Marker", "Vault76Marker", "WhitespringResort"];
    let out = inject_symbols(&src, &dst, &names).unwrap();
    let after = symbol_names(&out.body);
    for n in names {
        assert!(after.iter().any(|x| x == n), "missing {n}");
    }
    assert_eq!(after.len(), symbol_names(&dst.body).len() + names.len());
    // No duplicate character ids in the result's define tags.
    let spans = split_tags(&out.body).unwrap();
    let mut cids = std::collections::HashSet::new();
    for s in &spans {
        let b = &out.body[s.body_range()];
        // DefineSprite/DefineShape* carry a leading character id.
        if matches!(s.code, 2 | 22 | 32 | 83 | 39 | 46 | 84) && b.len() >= 2 {
            let cid = u16::from_le_bytes([b[0], b[1]]);
            assert!(cids.insert(cid), "duplicate character id {cid}");
        }
    }
}
