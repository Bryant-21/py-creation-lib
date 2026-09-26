//! Byte-identity gate: the splitter must tile a movie exactly (short and long
//! headers alike) and SymbolClass tags must survive parse→encode unchanged. This
//! is the foundation for marker injection (untouched tags spliced as opaque bytes).

use swf_native::container::{
    Signature, assemble, decompress, split_tags, tags_offset, write_tag_header,
};
use swf_native::symbolclass::{SymbolEntry, encode_symbol_table, parse_symbol_table};

#[test]
fn movies_tile_exactly_and_symbol_tables_round_trip() {
    let symbols = encode_symbol_table(&[
        SymbolEntry {
            character_id: 0,
            name: "Main".into(),
        },
        SymbolEntry {
            character_id: 7,
            name: "pkg.Marker".into(),
        },
    ]);
    // Non-zero RECT Nbits (2 → 13 bits → 2 bytes) so tags_offset is not the trivial case.
    let mut body = vec![0b0001_0000, 0x00, 0x00, 0x1E, 0x01, 0x00];
    body.extend(write_tag_header(9, 3, false));
    body.extend([1, 2, 3]);
    body.extend(write_tag_header(2, 4, true)); // long header on a short body survives
    body.extend([7, 0, 0xAA, 0xBB]);
    body.extend(write_tag_header(76, symbols.len(), false));
    body.extend(&symbols);
    body.extend(write_tag_header(1, 0, false));
    body.extend(write_tag_header(0, 0, false));
    body.extend([0xEE, 0xEE]); // trailing bytes after End are not tags

    assert_eq!(tags_offset(&body).unwrap(), 6);
    for signature in [Signature::Uncompressed, Signature::Zlib] {
        let movie = decompress(&assemble(signature, 14, &body).unwrap()).unwrap();
        assert_eq!((movie.signature, movie.version), (signature, 14));
        assert_eq!(movie.body, body);

        let spans = split_tags(&movie.body).unwrap();
        let codes: Vec<u16> = spans.iter().map(|s| s.code).collect();
        assert_eq!(codes, [9, 2, 76, 1, 0]);
        assert_eq!(spans[1].header_len, 6);
        let mut cursor = 6;
        for s in &spans {
            assert_eq!(s.start, cursor, "tag {} not contiguous", s.code);
            cursor = s.end();
        }
        assert_eq!(cursor, body.len() - 2);

        let table = &movie.body[spans[2].body_range()];
        let entries = parse_symbol_table(table).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[1].name, "pkg.Marker");
        assert_eq!(encode_symbol_table(&entries), table);
    }

    let mut truncated = body.clone();
    truncated.truncate(20); // cuts the long-header tag body short
    assert!(split_tags(&truncated).unwrap_err().contains("overruns"));
    assert!(decompress(b"XWS\x0e\0\0\0\0").is_err());
}
