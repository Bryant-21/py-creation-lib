//! Byte-accuracy gate for synthesized class definitions: what
//! `build_movieclip_class_abc` writes must read back as exactly the classes that
//! were asked for, must sit in a tag stream that still tiles the movie exactly,
//! and must satisfy the SymbolClass validator that gates `pack`.
//!
//! Everything here is self-contained — no extracted game assets — so the gate
//! runs anywhere. The comparison against a shipping file lives in the Python
//! tests, which have the real SWFs to hand.

use swf_native::abc::{DO_ABC_DEFINE, parse_abc_class_names, parse_abc_strings};
use swf_native::class_abc::{build_movieclip_class_abc, do_abc_define_body};
use swf_native::container::{split_tags, write_tag_header};
use swf_native::symbolclass::{SymbolEntry, encode_symbol_table};
use swf_native::unbacked_symbol_class_names;

/// A movie body around an arbitrary tag stream: a zero FrameSize RECT (Nbits =
/// 0, so 5 bits rounded to one byte), a 8.8 frame rate, and a frame count.
fn movie_body(tags: &[(u16, Vec<u8>)]) -> Vec<u8> {
    let mut body = vec![0u8, 0x00, 0x1E, 0x01, 0x00];
    for (code, tag_body) in tags {
        body.extend_from_slice(&write_tag_header(*code, tag_body.len(), false));
        body.extend_from_slice(tag_body);
    }
    body
}

fn swf_with_classes(names: &[&str]) -> Vec<u8> {
    let abc = build_movieclip_class_abc(names).expect("build abc");
    let symbols: Vec<SymbolEntry> = names
        .iter()
        .enumerate()
        .map(|(i, n)| SymbolEntry {
            character_id: i as u16,
            name: n.to_string(),
        })
        .collect();
    movie_body(&[
        (69, vec![0x08, 0, 0, 0]),                 // FileAttributes, ActionScript3
        (DO_ABC_DEFINE, do_abc_define_body(&abc)), // must precede SymbolClass
        (76, encode_symbol_table(&symbols)),
        (1, Vec::new()), // ShowFrame
        (0, Vec::new()), // End
    ])
}

#[test]
fn emitted_abc_defines_exactly_the_requested_classes() {
    let abc = build_movieclip_class_abc(&["B21_LegendaryStarRow", "B21_LegendaryStars"]).unwrap();
    let body = do_abc_define_body(&abc);

    let names = parse_abc_class_names(DO_ABC_DEFINE, &body).unwrap();
    assert_eq!(names, ["B21_LegendaryStarRow", "B21_LegendaryStars"]);

    // The read-only string-pool view must see the class names and the supertype
    // that the emitter claims to have written.
    let pool = parse_abc_strings(DO_ABC_DEFINE, &body).unwrap();
    assert_eq!((pool.major, pool.minor), (46, 16));
    for expected in [
        "B21_LegendaryStarRow",
        "B21_LegendaryStars",
        "flash.display",
        "MovieClip",
    ] {
        assert!(
            pool.strings.iter().any(|s| s == expected),
            "string pool is missing {expected:?}: {:?}",
            pool.strings
        );
    }
    assert_eq!(pool.int_count, 0);
    assert_eq!(pool.uint_count, 0);
    assert_eq!(pool.double_count, 0);
}

/// The `DoABCDefine` header and ABC version must match the shipping reference
/// (`weaponcnd.swf`): `flags = 1`, empty name, then minor 16 / major 46.
#[test]
fn do_abc_body_header_matches_the_reference_layout() {
    let abc = build_movieclip_class_abc(&["Main"]).unwrap();
    let body = do_abc_define_body(&abc);
    assert_eq!(&body[..5], &[0x01, 0x00, 0x00, 0x00, 0x00]);
    assert_eq!(&body[5..9], &[0x10, 0x00, 0x2E, 0x00]);
    assert_eq!(body.len(), abc.len() + 5);
}

// The full-byte lock on a one-class block lives with the compiler that now
// produces it — `as3_native`'s `compile::a_dynamic_class_compiles_to_the_locked_
// bytes`. `the_class_synthesizer_is_the_compiler` in `as3_equivalence.rs` pins
// this entry point to that one, so a second copy of the array here would only
// be another thing to keep in step.

#[test]
fn synthesized_swf_has_no_unbacked_symbol_classes() {
    let body = swf_with_classes(&["B21_LegendaryStarRow", "B21_LegendaryStars"]);

    // The tag stream must still tile the body exactly and end on End.
    let spans = split_tags(&body).unwrap();
    assert_eq!(spans.last().unwrap().code, 0);
    assert_eq!(spans.last().unwrap().end(), body.len());

    // DoABC must precede SymbolClass, as it does in the reference file.
    let abc_at = spans.iter().position(|s| s.code == DO_ABC_DEFINE).unwrap();
    let symbols_at = spans.iter().position(|s| s.code == 76).unwrap();
    assert!(abc_at < symbols_at);

    assert!(unbacked_symbol_class_names(&body).unwrap().is_empty());
}

#[test]
fn a_symbol_class_with_no_definition_is_reported() {
    let symbols = encode_symbol_table(&[
        SymbolEntry {
            character_id: 10,
            name: "B21_LegendaryStarRow".into(),
        },
        SymbolEntry {
            character_id: 0,
            name: "B21_LegendaryStars".into(),
        },
    ]);
    // No DoABC at all — the exact defect this validator exists to catch.
    let body = movie_body(&[(76, symbols), (1, Vec::new()), (0, Vec::new())]);
    assert_eq!(
        unbacked_symbol_class_names(&body).unwrap(),
        ["B21_LegendaryStarRow", "B21_LegendaryStars"]
    );
}

#[test]
fn only_the_missing_name_is_reported_when_some_resolve() {
    let abc = build_movieclip_class_abc(&["Present"]).unwrap();
    let symbols = encode_symbol_table(&[
        SymbolEntry {
            character_id: 1,
            name: "Present".into(),
        },
        SymbolEntry {
            character_id: 2,
            name: "Absent".into(),
        },
    ]);
    let body = movie_body(&[
        (DO_ABC_DEFINE, do_abc_define_body(&abc)),
        (76, symbols),
        (0, Vec::new()),
    ]);
    assert_eq!(unbacked_symbol_class_names(&body).unwrap(), ["Absent"]);
}

/// A packaged name has to survive the split-and-rejoin unchanged, because the
/// SymbolClass entry spells it the same way (`Shared.AS3.BSButtonHint`).
#[test]
fn package_qualified_names_round_trip() {
    let names = ["Shared.AS3.BSButtonHint", "HUDMenu_fla.tick_119", "Bare"];
    let abc = build_movieclip_class_abc(&names).unwrap();
    let body = do_abc_define_body(&abc);
    assert_eq!(parse_abc_class_names(DO_ABC_DEFINE, &body).unwrap(), names);
    assert!(
        unbacked_symbol_class_names(&swf_with_classes(&names))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn repeated_or_empty_class_names_are_rejected() {
    assert!(build_movieclip_class_abc(&[]).is_err());
    assert!(build_movieclip_class_abc(&[""]).is_err());
    assert!(build_movieclip_class_abc(&["Trailing."]).is_err());
    assert!(build_movieclip_class_abc(&["Same", "Same"]).is_err());
}
