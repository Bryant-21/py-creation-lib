//! Inject symbols with shared art from a source movie into a destination and
//! prove (a) the new exports appear, (b) untouched destination bytes are preserved
//! exactly around the splice, (c) character ids are remapped without collisions
//! and shared dependencies are emitted once, and (d) the result survives a full
//! assemble→decompress cycle.

use std::collections::HashSet;

use swf_native::container::{
    Movie, Signature, assemble, decompress, split_tags, split_tags_at, write_tag_header,
};
use swf_native::inject::inject_symbols;
use swf_native::symbolclass::{SymbolEntry, encode_symbol_table, parse_symbol_table};

const DEFINE_SHAPE: u16 = 2;
const DEFINE_SPRITE: u16 = 39;
const SYMBOL_CLASS: u16 = 76;

fn tag(code: u16, body: &[u8]) -> Vec<u8> {
    let mut out = write_tag_header(code, body.len(), false);
    out.extend_from_slice(body);
    out
}

fn shape(cid: u16, art: &[u8]) -> Vec<u8> {
    let mut body = cid.to_le_bytes().to_vec();
    body.extend_from_slice(art);
    tag(DEFINE_SHAPE, &body)
}

/// A one-frame sprite placing each child with PlaceObject2 (HasCharacter).
fn sprite(cid: u16, children: &[u16]) -> Vec<u8> {
    let mut body = cid.to_le_bytes().to_vec();
    body.extend(1u16.to_le_bytes());
    for (depth, child) in children.iter().enumerate() {
        let mut place = vec![0x02];
        place.extend((depth as u16 + 1).to_le_bytes());
        place.extend(child.to_le_bytes());
        body.extend(tag(26, &place));
    }
    body.extend(tag(1, &[]));
    body.extend(tag(0, &[]));
    tag(DEFINE_SPRITE, &body)
}

fn movie(tags: &[Vec<u8>], symbols: &[(u16, &str)]) -> Movie {
    let mut body = vec![0, 0, 0x1E, 1, 0];
    for t in tags {
        body.extend(t);
    }
    let rows: Vec<SymbolEntry> = symbols
        .iter()
        .map(|(id, name)| SymbolEntry {
            character_id: *id,
            name: (*name).into(),
        })
        .collect();
    body.extend(tag(SYMBOL_CLASS, &encode_symbol_table(&rows)));
    body.extend(tag(1, &[]));
    body.extend(tag(0, &[]));
    Movie {
        signature: Signature::Zlib,
        version: 14,
        body,
    }
}

fn symbol_rows(body: &[u8]) -> Vec<(u16, String)> {
    split_tags(body)
        .unwrap()
        .iter()
        .filter(|s| s.code == SYMBOL_CLASS)
        .flat_map(|s| parse_symbol_table(&body[s.body_range()]).unwrap())
        .map(|e| (e.character_id, e.name))
        .collect()
}

#[test]
fn injects_symbol_closures_with_remapped_ids_and_preserved_bytes() {
    // Shape 1 is shared art: both markers place it.
    let src = movie(
        &[
            shape(1, &[0xAA, 0xBB]),
            shape(2, &[0xCC]),
            shape(9, &[0xDD]), // unrelated, must not be copied
            sprite(3, &[1]),
            sprite(4, &[1, 2]),
        ],
        &[(3, "MarkerA"), (4, "MarkerB"), (9, "Unused")],
    );
    let dst = movie(
        &(1..=5).map(|cid| shape(cid, &[0x11])).collect::<Vec<_>>(),
        &[(5, "Existing")],
    );

    let out = inject_symbols(&src, &dst, &["MarkerA", "MarkerB"]).unwrap();

    // Ids shift by max(dst cid) + 1 = 6, so src 1,2,3,4 become 7,8,9,10.
    assert_eq!(
        symbol_rows(&out.body),
        [
            (5, "Existing".to_string()),
            (9, "MarkerA".to_string()),
            (10, "MarkerB".to_string())
        ]
    );

    let dst_spans = split_tags(&dst.body).unwrap();
    let sc = *dst_spans.iter().rfind(|s| s.code == SYMBOL_CLASS).unwrap();
    assert_eq!(&out.body[..sc.start], &dst.body[..sc.start], "prefix changed");
    let dst_tail = &dst.body[sc.end()..];
    assert!(out.body.ends_with(dst_tail), "suffix changed");

    let spans = split_tags(&out.body).unwrap();
    assert_eq!(spans.last().unwrap().code, 0);
    let mut cids = HashSet::new();
    let mut sprite_children = Vec::new();
    for s in &spans {
        let b = &out.body[s.body_range()];
        if matches!(s.code, DEFINE_SHAPE | DEFINE_SPRITE) {
            let cid = u16::from_le_bytes([b[0], b[1]]);
            assert!(cids.insert(cid), "duplicate character id {cid}");
        }
        if s.code == DEFINE_SPRITE {
            let children: Vec<u16> = split_tags_at(b, 4)
                .unwrap()
                .iter()
                .filter(|t| t.code == 26)
                .map(|t| {
                    let p = &b[t.body_range()];
                    u16::from_le_bytes([p[3], p[4]])
                })
                .collect();
            sprite_children.push(children);
        }
    }
    let expected: HashSet<u16> = (1..=5).chain(7..=10).collect();
    assert_eq!(cids, expected, "shared shape copied once, unused skipped");
    assert_eq!(sprite_children, [vec![7], vec![7, 8]]);

    let bytes = assemble(out.signature, out.version, &out.body).unwrap();
    assert_eq!(decompress(&bytes).unwrap().body, out.body);

    let err = inject_symbols(&src, &dst, &["Missing"]).unwrap_err();
    assert!(err.contains("not found"), "{err}");
}
