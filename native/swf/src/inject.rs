//! Symbol injection: lift a SymbolClass-named character (and its full character
//! dependency closure) from a source SWF into a destination SWF, remapping
//! character IDs above the destination's range so nothing collides, and
//! register the new SymbolClass export.
//!
//! Untouched destination tags are preserved as opaque byte ranges (the splice
//! inserts the new defines immediately before the SymbolClass tag and replaces
//! only that tag). Fonts/text/buttons in a closure are rejected — the FO76
//! marker icons are pure shape/sprite art (no bitmaps), which is the supported
//! MVP scope.

use std::collections::{HashMap, HashSet};

use crate::container::{Movie, TagSpan, split_tags, split_tags_at, write_tag_header};
use crate::symbolclass::{SymbolEntry, encode_symbol_table, parse_symbol_table};

const DEFINE_SPRITE: u16 = 39;
const SYMBOL_CLASS: u16 = 76;

/// The defining character ID of a Define* tag (the leading u16), or `None` for
/// tags that don't define a character.
fn defining_cid(code: u16, body: &[u8]) -> Option<u16> {
    let defines = matches!(
        code,
        2 | 22 | 32 | 83          // DefineShape 1/2/3/4
        | 39                      // DefineSprite
        | 46 | 84                 // DefineMorphShape 1/2
        | 6 | 21 | 35 | 90        // DefineBits / JPEG2 / JPEG3 / JPEG4
        | 20 | 36                 // DefineBitsLossless 1/2
        | 10 | 48 | 75            // DefineFont 1/2/3
        | 11 | 33                 // DefineText 1/2
        | 37                      // DefineEditText
        | 7 | 34                  // DefineButton 1/2
        | 60 // DefineVideoStream
    );
    if defines && body.len() >= 2 {
        Some(u16::from_le_bytes([body[0], body[1]]))
    } else {
        None
    }
}

/// Byte offset (within a PlaceObject tag body) of its referenced character ID,
/// if the tag references one. Handles PlaceObject (4), PlaceObject2 (26), and
/// PlaceObject3 (70) — the latter must skip an optional class-name string.
fn place_object_charid_offset(code: u16, body: &[u8]) -> Result<Option<usize>, String> {
    match code {
        4 => Ok(Some(0)), // PlaceObject: CharacterId is first
        26 => {
            // PlaceObject2: flags(1), depth(2), [HasCharacter] CharacterId(2)
            let flags = *body.first().ok_or("PlaceObject2 empty")?;
            Ok((flags & 0x02 != 0).then_some(3))
        }
        70 => {
            // PlaceObject3: flags(2), depth(2), [ClassName], [HasCharacter] id(2)
            if body.len() < 4 {
                return Err("PlaceObject3 too short".into());
            }
            let has_character = body[0] & 0x02 != 0;
            if !has_character {
                return Ok(None);
            }
            let has_class_name = body[1] & 0x08 != 0;
            let has_image = body[1] & 0x10 != 0;
            let mut p = 4;
            if has_class_name || (has_image && has_character) {
                while p < body.len() && body[p] != 0 {
                    p += 1;
                }
                if p >= body.len() {
                    return Err("PlaceObject3 unterminated class name".into());
                }
                p += 1; // skip NUL
            }
            Ok(Some(p))
        }
        _ => Ok(None),
    }
}

/// Character IDs referenced from inside a DefineSprite body (its control-tag
/// stream begins after a u16 sprite id + u16 frame count).
fn sprite_char_refs(sprite_body: &[u8]) -> Result<Vec<u16>, String> {
    let mut refs = Vec::new();
    for t in split_tags_at(sprite_body, 4)? {
        let tbody = &sprite_body[t.body_range()];
        if let Some(off) = place_object_charid_offset(t.code, tbody)? {
            if off + 2 > tbody.len() {
                return Err("PlaceObject character id overruns tag".into());
            }
            refs.push(u16::from_le_bytes([tbody[off], tbody[off + 1]]));
        }
    }
    Ok(refs)
}

/// Characters that `code` references. Shapes/morphs/bitmaps are leaves (the
/// FO76 marker library has no bitmaps); fonts/text/buttons are out of scope.
fn internal_refs(code: u16, body: &[u8]) -> Result<Vec<u16>, String> {
    match code {
        DEFINE_SPRITE => sprite_char_refs(body),
        2 | 22 | 32 | 83 | 46 | 84 | 20 | 36 | 6 | 21 | 35 | 90 => Ok(Vec::new()),
        7 | 34 => Err("button in marker closure is not supported".into()),
        10 | 11 | 33 | 37 | 48 | 75 => Err("font/text in marker closure is not supported".into()),
        _ => Ok(Vec::new()),
    }
}

struct SourceIndex<'a> {
    spans: Vec<TagSpan>,
    body: &'a [u8],
    define_at: HashMap<u16, usize>,
    name_to_cid: HashMap<String, u16>,
}

fn index_source(movie: &Movie) -> Result<SourceIndex<'_>, String> {
    let spans = split_tags(&movie.body)?;
    let mut define_at = HashMap::new();
    let mut name_to_cid = HashMap::new();
    for (i, s) in spans.iter().enumerate() {
        let body = &movie.body[s.body_range()];
        if let Some(cid) = defining_cid(s.code, body) {
            define_at.insert(cid, i);
        }
        if s.code == SYMBOL_CLASS {
            for e in parse_symbol_table(body)? {
                name_to_cid.insert(e.name, e.character_id);
            }
        }
    }
    Ok(SourceIndex {
        spans,
        body: &movie.body,
        define_at,
        name_to_cid,
    })
}

/// Transitive character closure of `root`, returned as source span indices in
/// file order (defines already precede their uses in a valid SWF).
fn closure_spans(root: u16, src: &SourceIndex) -> Result<Vec<usize>, String> {
    let mut visited: HashSet<u16> = HashSet::new();
    let mut stack = vec![root];
    while let Some(cid) = stack.pop() {
        if !visited.insert(cid) {
            continue;
        }
        let &idx = src
            .define_at
            .get(&cid)
            .ok_or_else(|| format!("dangling character {cid}: no defining tag in source"))?;
        let span = &src.spans[idx];
        for r in internal_refs(span.code, &src.body[span.body_range()])? {
            stack.push(r);
        }
    }
    let mut indices: Vec<usize> = visited.iter().map(|cid| src.define_at[cid]).collect();
    indices.sort_unstable();
    Ok(indices)
}

/// Copy a define tag body with every character id (its own and any internal
/// references) shifted by `offset`.
fn remap_define_body(code: u16, body: &[u8], offset: u16) -> Result<Vec<u8>, String> {
    let mut out = body.to_vec();
    let own = u16::from_le_bytes([out[0], out[1]]);
    out[0..2].copy_from_slice(&own.wrapping_add(offset).to_le_bytes());
    if code == DEFINE_SPRITE {
        for t in split_tags_at(body, 4)? {
            let tbody = &body[t.body_range()];
            if let Some(off) = place_object_charid_offset(t.code, tbody)? {
                let abs = t.body_range().start + off;
                let cur = u16::from_le_bytes([out[abs], out[abs + 1]]);
                out[abs..abs + 2].copy_from_slice(&cur.wrapping_add(offset).to_le_bytes());
            }
        }
    }
    Ok(out)
}

fn max_cid(spans: &[TagSpan], body: &[u8]) -> u16 {
    spans
        .iter()
        .filter_map(|s| defining_cid(s.code, &body[s.body_range()]))
        .max()
        .unwrap_or(0)
}

/// The SymbolClass spans in `dst`. New defines are inserted before the last one
/// and the new export rows are appended to it.
fn symbol_class_spans(spans: &[TagSpan]) -> Vec<usize> {
    spans
        .iter()
        .enumerate()
        .filter(|(_, s)| s.code == SYMBOL_CLASS)
        .map(|(i, _)| i)
        .collect()
}

/// Inject the named symbols (and their closures) from `src` into `dst`, using the
/// same SymbolClass export name in `dst` as in `src`. Convenience wrapper over
/// [`inject_symbols_renamed`].
pub fn inject_symbols(src: &Movie, dst: &Movie, names: &[&str]) -> Result<Movie, String> {
    let pairs: Vec<(&str, &str)> = names.iter().map(|&n| (n, n)).collect();
    inject_symbols_renamed(src, dst, &pairs)
}

/// Inject symbols (and their closures) from `src` into `dst`, exporting each under
/// a chosen name. Each pair is `(source_symbol, export_name)`: the source symbol
/// names the art in `src`; the export name is the SymbolClass export registered in
/// `dst` (rename to avoid colliding with an existing `dst` export). Returns a new
/// movie body. Injected character ids occupy `max(dst cid)+1 ..`, so they never
/// collide; shared dependencies across the requested symbols are emitted once.
pub fn inject_symbols_renamed(
    src: &Movie,
    dst: &Movie,
    pairs: &[(&str, &str)],
) -> Result<Movie, String> {
    let src_idx = index_source(src)?;
    let dst_spans = split_tags(&dst.body)?;

    let sc_indices = symbol_class_spans(&dst_spans);
    let &sc_idx = sc_indices
        .last()
        .ok_or("destination has no SymbolClass tag")?;
    let sc = dst_spans[sc_idx];

    let offset = max_cid(&dst_spans, &dst.body).wrapping_add(1);

    // Union closure across all requested symbols (dedups shared art).
    let mut wanted: Vec<usize> = Vec::new();
    let mut seen: HashSet<usize> = HashSet::new();
    let mut new_rows: Vec<SymbolEntry> = Vec::new();
    for &(source, export) in pairs {
        let &root = src_idx
            .name_to_cid
            .get(source)
            .ok_or_else(|| format!("symbol '{source}' not found in source"))?;
        for idx in closure_spans(root, &src_idx)? {
            if seen.insert(idx) {
                wanted.push(idx);
            }
        }
        new_rows.push(SymbolEntry {
            character_id: root.wrapping_add(offset),
            name: export.to_string(),
        });
    }
    wanted.sort_unstable();

    // Emit the remapped define tags.
    let mut new_defines: Vec<u8> = Vec::new();
    for idx in wanted {
        let span = &src_idx.spans[idx];
        let remapped = remap_define_body(span.code, &src_idx.body[span.body_range()], offset)?;
        new_defines.extend_from_slice(&write_tag_header(span.code, remapped.len(), false));
        new_defines.extend_from_slice(&remapped);
    }

    // Rebuild the destination SymbolClass with the appended rows.
    let mut entries = parse_symbol_table(&dst.body[sc.body_range()])?;
    entries.extend(new_rows);
    let new_sc_body = encode_symbol_table(&entries);
    let mut new_sc_tag = write_tag_header(SYMBOL_CLASS, new_sc_body.len(), false);
    new_sc_tag.extend_from_slice(&new_sc_body);

    // Splice: [..sc.start] + new defines + new SymbolClass + [sc.end()..].
    let mut new_body = Vec::with_capacity(dst.body.len() + new_defines.len() + new_sc_tag.len());
    new_body.extend_from_slice(&dst.body[..sc.start]);
    new_body.extend_from_slice(&new_defines);
    new_body.extend_from_slice(&new_sc_tag);
    new_body.extend_from_slice(&dst.body[sc.end()..]);

    Ok(Movie {
        signature: dst.signature,
        version: dst.version,
        body: new_body,
    })
}
