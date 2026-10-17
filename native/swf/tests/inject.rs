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
use swf_native::shape::dangling_bitmap_fills;
use swf_native::symbolclass::{SymbolEntry, encode_symbol_table, parse_symbol_table};

const DEFINE_SHAPE: u16 = 2;
const DEFINE_SPRITE: u16 = 39;
const SYMBOL_CLASS: u16 = 76;

fn tag(code: u16, body: &[u8]) -> Vec<u8> {
    let mut out = write_tag_header(code, body.len(), false);
    out.extend_from_slice(body);
    out
}

/// A DefineShape with an empty bounds RECT, the given fill styles, no line
/// styles and no edges.
fn shape_with_fills(cid: u16, fills: &[u8], count: u8) -> Vec<u8> {
    let mut body = cid.to_le_bytes().to_vec();
    body.push(0); // RECT with 0-bit fields
    body.push(count);
    body.extend_from_slice(fills);
    body.push(0); // line style count
    body.push(0); // NumFillBits/NumLineBits
    body.push(0); // EndShapeRecord
    tag(DEFINE_SHAPE, &body)
}

fn shape(cid: u16) -> Vec<u8> {
    shape_with_fills(cid, &[], 0)
}

/// A clipped bitmap fill style naming `bitmap`, with an empty MATRIX.
fn bitmap_fill(bitmap: u16) -> Vec<u8> {
    let mut fill = vec![0x41];
    fill.extend(bitmap.to_le_bytes());
    fill.push(0);
    fill
}

fn bitmap(cid: u16) -> Vec<u8> {
    let mut body = cid.to_le_bytes().to_vec();
    body.extend_from_slice(&[5, 1, 0, 1, 0, 0x78, 0x9C]);
    tag(36, &body)
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
            shape(1),
            shape(2),
            shape(9), // unrelated, must not be copied
            sprite(3, &[1]),
            sprite(4, &[1, 2]),
        ],
        &[(3, "MarkerA"), (4, "MarkerB"), (9, "Unused")],
    );
    let dst = movie(
        &(1..=5).map(shape).collect::<Vec<_>>(),
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

/// FO76's ImSelling1 emote icon fills a shape with a bitmap. Shapes reference
/// bitmap fills by id, not placement; the bitmap must travel and the fill must
/// follow the remap, or it names whatever lands on the old id in `dst`.
#[test]
fn injects_bitmaps_named_by_shape_fills() {
    let mut fills = bitmap_fill(65535); // Flash's unused-fill placeholder
    fills.extend(bitmap_fill(4));
    let src = movie(
        &[bitmap(4), shape_with_fills(5, &fills, 2), sprite(6, &[5])],
        &[(6, "Selling")],
    );
    let dst = movie(&(1..=9).map(shape).collect::<Vec<_>>(), &[(9, "Existing")]);

    let out = inject_symbols(&src, &dst, &["Selling"]).unwrap();

    let spans = split_tags(&out.body).unwrap();
    let defines = |code: u16| -> Vec<&[u8]> {
        spans
            .iter()
            .filter(|s| s.code == code)
            .map(|s| &out.body[s.body_range()])
            .collect()
    };
    let bitmaps = defines(36);
    assert_eq!(bitmaps.len(), 1, "bitmap copied once");
    assert_eq!(u16::from_le_bytes([bitmaps[0][0], bitmaps[0][1]]), 14);
    let selling = defines(DEFINE_SHAPE)
        .into_iter()
        .find(|b| u16::from_le_bytes([b[0], b[1]]) == 15)
        .expect("remapped shape");
    let mut expected = bitmap_fill(65535);
    expected.extend(bitmap_fill(14));
    assert_eq!(&selling[4..4 + expected.len()], expected.as_slice());
    assert_eq!(dangling_bitmap_fills(&out.body).unwrap(), []);

    // What the injector used to emit: the fill names a sprite.
    let broken = movie(&[sprite(3, &[]), shape_with_fills(5, &bitmap_fill(3), 1)], &[]);
    assert_eq!(dangling_bitmap_fills(&broken.body).unwrap(), [(5, 3)]);
}
