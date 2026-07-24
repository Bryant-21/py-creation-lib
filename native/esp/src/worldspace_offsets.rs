use super::*;
use pyo3::exceptions::{PyKeyError, PyValueError};
use serde::Serialize;

// FO4 WRLD OFST/CLSZ cell seek tables (TESWorldSpace::offsetDataMap).
//
// Conversion strips the FO76 source tables (they encode source-file offsets);
// this rebuilds FO4-native ones against the target plugin's own layout. Format
// decoded empirically from Fallout4.esm + every DLC ESM (2026-07), all 18
// OFST-bearing vanilla worldspaces verified entry-exact:
//
// - Grid rect: NAM0/NAM9 world coords / 4096. Every vanilla (and converted)
//   worldspace is cell-aligned; an unaligned NAM pair means the engine's
//   rounding is unverifiable, so that worldspace is skipped with a warning
//   (wrong dimensions would make the engine seek to wrong bytes — worse than
//   shipping no table).
// - Index: y-major, `idx = (y - min_y) * width + (x - min_x)`; 0 = no cell.
// - OFST[idx]: u32 byte offset of the CELL record header relative to the WRLD
//   record header in the serialized file.
// - CLSZ[idx]: CELL record size + its cell-children group size + 24. Vanilla
//   mixes the +24 form (base-game Commonwealth: all 36864 cells) with the
//   exact form (DLCs, partially), so the engine tolerates both; we match the
//   base game.
// - Only CELLs under exterior sub-block groups are indexed. The worldspace
//   persistent cell also carries XCLC (0,0) but sits directly under the world
//   children group and is excluded; the wilderness (0,0) cell wins.
// - The tables are the last two WRLD subrecords (OFST then CLSZ) and exceed
//   64 KiB on large worlds — XXXX framing is applied by the serializer.
//
// Offsets encode the final byte layout, so this must be the last mutation
// before the plugin is written; the measure-walk below sizes every item with
// the serializer's own framing functions, which makes measure→save agreement
// structural (compressed records re-emit their raw payload verbatim).

const CLSZ_TRAILING_PAD: u64 = 24;
const CELL_GRID_UNITS: f32 = 4096.0;
const MAX_GRID_ENTRIES: u64 = 4096 * 4096;

#[derive(Default, Serialize)]
pub struct WorldspaceCellOffsetsPayload {
    pub worldspaces_rebuilt: u32,
    pub cells_indexed: u64,
    pub cells_out_of_rect: u64,
    pub cells_missing_xclc: u64,
    pub duplicate_grid_cells: u64,
    pub warnings: Vec<String>,
}

struct Grid {
    min_x: i64,
    min_y: i64,
    width: i64,
    height: i64,
}

impl Grid {
    fn entries(&self) -> u64 {
        (self.width * self.height) as u64
    }

    fn index_of(&self, x: i64, y: i64) -> Option<usize> {
        if x < self.min_x
            || y < self.min_y
            || x >= self.min_x + self.width
            || y >= self.min_y + self.height
        {
            return None;
        }
        Some(((y - self.min_y) * self.width + (x - self.min_x)) as usize)
    }
}

struct CellEntry {
    xy: Option<(i64, i64)>,
    rel_offset: u64,
    cell_size: u64,
}

pub fn rebuild_worldspace_cell_offsets(
    plugin: &mut ParsedPlugin,
) -> PyResult<WorldspaceCellOffsetsPayload> {
    let mut payload = WorldspaceCellOffsetsPayload::default();
    let header_size = plugin.header_size;
    if header_size != MODERN_HEADER_SIZE {
        payload.warnings.push(format!(
            "skipped OFST rebuild: legacy header size {header_size} is not the FO4 record layout"
        ));
        return Ok(payload);
    }

    for item in &mut plugin.root_items {
        let ParsedItem::Group(group) = item else {
            continue;
        };
        if group.group_type != 0 || group.label != *b"WRLD" {
            continue;
        }
        rebuild_top_group(group, header_size, &mut payload)?;
    }
    Ok(payload)
}

fn rebuild_top_group(
    top: &mut ParsedGroup,
    header_size: usize,
    payload: &mut WorldspaceCellOffsetsPayload,
) -> PyResult<()> {
    // (WRLD record index, world-children group index) pairs.
    let mut pairs: Vec<(usize, usize)> = Vec::new();
    let mut last_wrld: Option<(usize, u32)> = None;
    for (idx, child) in top.children.iter().enumerate() {
        match child {
            ParsedItem::Record(record) if record.signature.as_str() == "WRLD" => {
                last_wrld = Some((idx, record.form_id));
            }
            ParsedItem::Group(group) if group.group_type == 1 => {
                if let Some((wrld_idx, form_id)) = last_wrld {
                    if group.label == form_id.to_le_bytes() {
                        pairs.push((wrld_idx, idx));
                    }
                }
                last_wrld = None;
            }
            _ => {}
        }
    }

    for (wrld_idx, children_idx) in pairs {
        rebuild_worldspace(top, wrld_idx, children_idx, header_size, payload)?;
    }
    Ok(())
}

fn rebuild_worldspace(
    top: &mut ParsedGroup,
    wrld_idx: usize,
    children_idx: usize,
    header_size: usize,
    payload: &mut WorldspaceCellOffsetsPayload,
) -> PyResult<()> {
    let has_blocks = match &top.children[children_idx] {
        ParsedItem::Group(group) => group
            .children
            .iter()
            .any(|c| matches!(c, ParsedItem::Group(g) if g.group_type == EXTERIOR_CELL_BLOCK)),
        _ => false,
    };

    let grid = {
        let ParsedItem::Record(record) = &mut top.children[wrld_idx] else {
            return Ok(());
        };
        let editor_id = record_editor_id(record);
        if !has_blocks {
            strip_tables(record);
            return Ok(());
        }
        match prepare_wrld_record(record) {
            Ok(grid) => grid,
            Err(reason) => {
                payload
                    .warnings
                    .push(format!("skipped OFST rebuild for '{editor_id}': {reason}"));
                return Ok(());
            }
        }
    };

    let wrld_len = match &top.children[wrld_idx] {
        ParsedItem::Record(record) => record_bytes_from_parsed(record, header_size)?.len() as u64,
        _ => unreachable!(),
    };

    let mut cells: Vec<CellEntry> = Vec::new();
    let mut cursor = wrld_len;
    if let ParsedItem::Group(group) = &top.children[children_idx] {
        measure_group(group, header_size, &mut cursor, &mut cells)?;
    }

    let entries = grid.entries() as usize;
    let mut ofst = vec![0u8; entries * 4];
    let mut clsz = vec![0u8; entries * 4];
    for cell in &cells {
        let Some((x, y)) = cell.xy else {
            payload.cells_missing_xclc += 1;
            continue;
        };
        let Some(idx) = grid.index_of(x, y) else {
            payload.cells_out_of_rect += 1;
            continue;
        };
        if cell.rel_offset > u32::MAX as u64 || cell.cell_size > u32::MAX as u64 {
            payload.warnings.push(format!(
                "cell ({x},{y}) exceeds u32 offset range; entry left empty"
            ));
            continue;
        }
        let at = idx * 4;
        if ofst[at..at + 4] != [0, 0, 0, 0] {
            payload.duplicate_grid_cells += 1;
        }
        ofst[at..at + 4].copy_from_slice(&(cell.rel_offset as u32).to_le_bytes());
        clsz[at..at + 4].copy_from_slice(&(cell.cell_size as u32).to_le_bytes());
        payload.cells_indexed += 1;
    }

    let ParsedItem::Record(record) = &mut top.children[wrld_idx] else {
        return Ok(());
    };
    for subrecord in record.subrecords.iter_mut().rev() {
        match subrecord.signature.as_str() {
            "OFST" => subrecord.data = Bytes::from(std::mem::take(&mut ofst)),
            "CLSZ" => subrecord.data = Bytes::from(std::mem::take(&mut clsz)),
            _ => {}
        }
    }
    payload.worldspaces_rebuilt += 1;
    Ok(())
}

/// Materialize the WRLD record's subrecords (it may be compressed or lazy),
/// drop any stale tables, and append zero-filled OFST/CLSZ placeholders of
/// their exact final size so the measure pass sees the final record length.
fn prepare_wrld_record(record: &mut ParsedRecord) -> Result<Grid, String> {
    let subrecords = materialized_subrecords(record)?;
    let grid = grid_from_nam_bounds(&subrecords)?;

    record.subrecords = subrecords
        .into_iter()
        .filter(|s| !matches!(s.signature.as_str(), "OFST" | "CLSZ"))
        .collect();
    record.raw_payload = None;
    record.flags &= !COMPRESSED_RECORD_FLAG;

    let table_len = grid.entries() as usize * 4;
    for sig in ["OFST", "CLSZ"] {
        record.subrecords.push(ParsedSubrecord {
            signature: SmolStr::new(sig),
            data: Bytes::from(vec![0u8; table_len]),
            semantic_type: None,
        });
    }
    Ok(grid)
}

fn record_editor_id(record: &ParsedRecord) -> String {
    effective_subrecords_for_record(record)
        .iter()
        .find(|s| s.signature.as_str() == "EDID")
        .map(|s| {
            let end = s.data.iter().position(|&b| b == 0).unwrap_or(s.data.len());
            String::from_utf8_lossy(&s.data[..end]).into_owned()
        })
        .unwrap_or_else(|| format!("{:08X}", record.form_id))
}

fn materialized_subrecords(record: &ParsedRecord) -> Result<Vec<ParsedSubrecord>, String> {
    if !record.subrecords.is_empty() {
        return Ok(record.subrecords.clone());
    }
    let Some(raw) = &record.raw_payload else {
        return Ok(Vec::new());
    };
    if (record.flags & COMPRESSED_RECORD_FLAG) != 0 {
        decode_compressed_subrecords_from_payload(raw).map(|decoded| decoded.subrecords)
    } else {
        split_record_payload(raw)
    }
}

fn strip_tables(record: &mut ParsedRecord) {
    if record
        .subrecords
        .iter()
        .any(|s| matches!(s.signature.as_str(), "OFST" | "CLSZ"))
    {
        record
            .subrecords
            .retain(|s| !matches!(s.signature.as_str(), "OFST" | "CLSZ"));
        record.raw_payload = None;
    }
}

fn grid_from_nam_bounds(subrecords: &[ParsedSubrecord]) -> Result<Grid, String> {
    let bounds = |sig: &str| -> Result<(f32, f32), String> {
        let sub = subrecords
            .iter()
            .find(|s| s.signature.as_str() == sig)
            .ok_or_else(|| format!("missing {sig}"))?;
        if sub.data.len() < 8 {
            return Err(format!("{sig} shorter than 8 bytes"));
        }
        Ok((
            f32::from_le_bytes(sub.data[0..4].try_into().unwrap()),
            f32::from_le_bytes(sub.data[4..8].try_into().unwrap()),
        ))
    };
    let (min_wx, min_wy) = bounds("NAM0")?;
    let (max_wx, max_wy) = bounds("NAM9")?;

    let cell = |v: f32, label: &str| -> Result<i64, String> {
        let c = v / CELL_GRID_UNITS;
        if !c.is_finite() || c.fract() != 0.0 {
            return Err(format!(
                "{label} {v} is not cell-aligned; engine grid derivation unverified for unaligned bounds"
            ));
        }
        Ok(c as i64)
    };
    let min_x = cell(min_wx, "NAM0.x")?;
    let min_y = cell(min_wy, "NAM0.y")?;
    let max_x = cell(max_wx, "NAM9.x")?;
    let max_y = cell(max_wy, "NAM9.y")?;
    if [min_x, min_y, max_x, max_y]
        .iter()
        .any(|c| c.unsigned_abs() > 1_000_000)
    {
        return Err(format!(
            "implausible cell bounds ({min_x},{min_y})..({max_x},{max_y})"
        ));
    }
    if max_x < min_x || max_y < min_y {
        return Err(format!(
            "inverted bounds: ({min_x},{min_y})..({max_x},{max_y})"
        ));
    }
    let grid = Grid {
        min_x,
        min_y,
        width: max_x - min_x + 1,
        height: max_y - min_y + 1,
    };
    if grid.entries() == 0 || grid.entries() > MAX_GRID_ENTRIES {
        return Err(format!("implausible grid {}x{}", grid.width, grid.height));
    }
    Ok(grid)
}

/// Advance `cursor` across the group's serialized bytes exactly as the writer
/// lays them out, recording every exterior sub-block CELL. Returns the group's
/// total serialized size.
fn measure_group(
    group: &ParsedGroup,
    header_size: usize,
    cursor: &mut u64,
    cells: &mut Vec<CellEntry>,
) -> PyResult<u64> {
    let start = *cursor;
    *cursor += header_size as u64;
    let indexing = group.group_type == EXTERIOR_CELL_SUBBLOCK;
    let mut i = 0;
    while i < group.children.len() {
        match &group.children[i] {
            ParsedItem::Group(inner) => {
                measure_group(inner, header_size, cursor, cells)?;
            }
            ParsedItem::Record(record) => {
                let record_start = *cursor;
                let record_len = record_bytes_from_parsed(record, header_size)?.len() as u64;
                *cursor += record_len;
                if record.signature.as_str() == "CELL" {
                    let mut children_len = 0u64;
                    if let Some(ParsedItem::Group(cell_children)) = group.children.get(i + 1) {
                        if cell_children.group_type == CELL_CHILD_GROUP {
                            children_len =
                                measure_group(cell_children, header_size, cursor, cells)?;
                            i += 1;
                        }
                    }
                    if indexing {
                        cells.push(CellEntry {
                            xy: cell_grid_coords(record),
                            rel_offset: record_start,
                            cell_size: record_len + children_len + CLSZ_TRAILING_PAD,
                        });
                    }
                }
            }
        }
        i += 1;
    }
    Ok(*cursor - start)
}

fn cell_grid_coords(record: &ParsedRecord) -> Option<(i64, i64)> {
    let subrecords = effective_subrecords_for_record(record);
    let xclc = subrecords.iter().find(|s| s.signature.as_str() == "XCLC")?;
    if xclc.data.len() < 8 {
        return None;
    }
    let x = i32::from_le_bytes(xclc.data[0..4].try_into().unwrap());
    let y = i32::from_le_bytes(xclc.data[4..8].try_into().unwrap());
    Some((x as i64, y as i64))
}

pub(crate) fn plugin_handle_rebuild_worldspace_cell_offsets_json(
    handle_id: u64,
) -> PyResult<String> {
    let mut store = plugin_handle_store_ref().lock().unwrap();
    let slot = store
        .get_mut(&handle_id)
        .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
    let payload = rebuild_worldspace_cell_offsets(&mut slot.parsed)?;
    if payload.worldspaces_rebuilt > 0 {
        slot.clear_record_count_cache();
        slot.invalidate_sections();
    }
    serde_json::to_string(&payload).map_err(|err| {
        PyValueError::new_err(format!("failed to encode cell offsets result: {err}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sub(signature: &str, data: Vec<u8>) -> ParsedSubrecord {
        ParsedSubrecord {
            signature: SmolStr::new(signature),
            data: Bytes::from(data),
            semantic_type: None,
        }
    }

    fn edid(name: &str) -> ParsedSubrecord {
        let mut data = name.as_bytes().to_vec();
        data.push(0);
        sub("EDID", data)
    }

    fn nam_pair(min_cell: i32, max_cell: i32) -> [ParsedSubrecord; 2] {
        let lo = (min_cell as f32) * CELL_GRID_UNITS;
        let hi = (max_cell as f32) * CELL_GRID_UNITS;
        [
            sub("NAM0", [lo.to_le_bytes(), lo.to_le_bytes()].concat()),
            sub("NAM9", [hi.to_le_bytes(), hi.to_le_bytes()].concat()),
        ]
    }

    fn record(signature: &str, form_id: u32, subs: Vec<ParsedSubrecord>) -> ParsedItem {
        ParsedItem::Record(ParsedRecord {
            signature: SmolStr::new(signature),
            form_id,
            flags: 0,
            version_control: 0,
            form_version: Some(131),
            version2: Some(1),
            subrecords: subs,
            raw_payload: None,
            parse_error: None,
        })
    }

    fn cell_subs(x: i32, y: i32, filler: usize) -> Vec<ParsedSubrecord> {
        vec![
            sub("DATA", vec![2, 0]),
            sub("XCLC", [x.to_le_bytes(), y.to_le_bytes()].concat().to_vec()),
            sub("MHDT", vec![0xAB; filler]),
        ]
    }

    fn compressed_cell(form_id: u32, x: i32, y: i32, filler: usize, lazy: bool) -> ParsedItem {
        let subs = cell_subs(x, y, filler);
        if lazy {
            let payload = compress_subrecords_payload(&subs).expect("compress cell payload");
            ParsedItem::Record(ParsedRecord {
                signature: SmolStr::new("CELL"),
                form_id,
                flags: COMPRESSED_RECORD_FLAG,
                version_control: 0,
                form_version: Some(131),
                version2: Some(1),
                subrecords: Vec::new(),
                raw_payload: Some(Bytes::from(payload)),
                parse_error: None,
            })
        } else {
            let ParsedItem::Record(mut rec) = record("CELL", form_id, subs) else {
                unreachable!()
            };
            rec.flags = COMPRESSED_RECORD_FLAG;
            ParsedItem::Record(rec)
        }
    }

    fn group(group_type: i32, label: [u8; 4], children: Vec<ParsedItem>) -> ParsedItem {
        ParsedItem::Group(ParsedGroup {
            label,
            group_type,
            tail: Bytes::new(),
            children,
        })
    }

    fn cell_children_group(cell_form_id: u32, temp_children: Vec<ParsedItem>) -> ParsedItem {
        group(
            CELL_CHILD_GROUP,
            cell_form_id.to_le_bytes(),
            vec![group(
                TEMPORARY_GROUP,
                cell_form_id.to_le_bytes(),
                temp_children,
            )],
        )
    }

    fn land(form_id: u32, filler: usize) -> ParsedItem {
        record("LAND", form_id, vec![sub("DATA", vec![0x11; filler])])
    }

    const WRLD_ID: u32 = 0x0000_0F99;

    fn worldspace_plugin(min_cell: i32, max_cell: i32, lazy_cells: bool) -> ParsedPlugin {
        // Persistent cell shares XCLC (0,0) with the wilderness (0,0) cell; the
        // table must point at the latter.
        let persistent = compressed_cell(0x100, 0, 0, 4, lazy_cells);
        let mut sub_block = Vec::new();
        for (i, (x, y)) in [(0, 0), (1, 0), (0, 1)].iter().enumerate() {
            let cell_id = 0x200 + i as u32;
            sub_block.push(compressed_cell(cell_id, *x, *y, 8 + i * 32, lazy_cells));
            sub_block.push(cell_children_group(
                cell_id,
                vec![land(0x300 + i as u32, 16 + i * 8)],
            ));
        }
        let wrld_subs = {
            let mut subs = vec![edid("TESTWORLD")];
            subs.extend(nam_pair(min_cell, max_cell));
            subs.push(sub("DNAM", vec![0; 8]));
            subs
        };
        let children = group(
            1,
            WRLD_ID.to_le_bytes(),
            vec![
                persistent,
                cell_children_group(0x100, vec![land(0x301, 4)]),
                group(
                    EXTERIOR_CELL_BLOCK,
                    [0, 0, 0, 0],
                    vec![group(EXTERIOR_CELL_SUBBLOCK, [0, 0, 0, 0], sub_block)],
                ),
            ],
        );
        let mut header = ParsedPluginHeader::default_for_test();
        header.masters = vec!["Fallout4.esm".to_string()];
        ParsedPlugin {
            plugin_name: "Test.esm".to_string(),
            file_path: String::new(),
            header_size: MODERN_HEADER_SIZE,
            header,
            root_items: vec![group(
                0,
                *b"WRLD",
                vec![record("WRLD", WRLD_ID, wrld_subs), children],
            )],
            game: Some("fo4".to_string()),
        }
    }

    fn wrld_record(plugin: &ParsedPlugin) -> &ParsedRecord {
        for item in &plugin.root_items {
            if let ParsedItem::Group(g) = item {
                for child in &g.children {
                    if let ParsedItem::Record(r) = child {
                        if r.signature.as_str() == "WRLD" {
                            return r;
                        }
                    }
                }
            }
        }
        panic!("no WRLD record");
    }

    fn table(plugin: &ParsedPlugin, sig: &str) -> Vec<u32> {
        let rec = wrld_record(plugin);
        let data = &rec
            .subrecords
            .iter()
            .find(|s| s.signature.as_str() == sig)
            .unwrap_or_else(|| panic!("no {sig}"))
            .data;
        data.chunks_exact(4)
            .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
            .collect()
    }

    // Independent byte-level walker: locate the WRLD record and every CELL in
    // the serialized bytes, byte-scanning rather than reusing the measure-walk.
    fn scan_serialized(bytes: &[u8]) -> (usize, Vec<(u32, usize)>) {
        let tes4_size = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
        let mut pos = MODERN_HEADER_SIZE + tes4_size;
        let mut wrld_pos = None;
        let mut cell_positions = Vec::new();
        fn walk(
            bytes: &[u8],
            start: usize,
            end: usize,
            wrld_pos: &mut Option<usize>,
            cells: &mut Vec<(u32, usize)>,
        ) {
            let mut pos = start;
            while pos < end {
                if &bytes[pos..pos + 4] == b"GRUP" {
                    let gsize =
                        u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap()) as usize;
                    walk(
                        bytes,
                        pos + MODERN_HEADER_SIZE,
                        pos + gsize,
                        wrld_pos,
                        cells,
                    );
                    pos += gsize;
                    continue;
                }
                let dsize =
                    u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap()) as usize;
                let form_id = u32::from_le_bytes(bytes[pos + 12..pos + 16].try_into().unwrap());
                match &bytes[pos..pos + 4] {
                    b"WRLD" => *wrld_pos = Some(pos),
                    b"CELL" => cells.push((form_id, pos)),
                    _ => {}
                }
                pos += MODERN_HEADER_SIZE + dsize;
            }
        }
        while pos < bytes.len() {
            let gsize = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap()) as usize;
            walk(
                bytes,
                pos + MODERN_HEADER_SIZE,
                pos + gsize,
                &mut wrld_pos,
                &mut cell_positions,
            );
            pos += gsize;
        }
        (wrld_pos.expect("WRLD in serialized bytes"), cell_positions)
    }

    #[test]
    fn keystone_offsets_land_on_cell_headers_in_serialized_bytes() {
        let mut plugin = worldspace_plugin(-2, 2, false);
        let payload = rebuild_worldspace_cell_offsets(&mut plugin).expect("rebuild");
        assert_eq!(payload.worldspaces_rebuilt, 1);
        assert_eq!(payload.cells_indexed, 3);
        assert_eq!(payload.cells_out_of_rect, 0);
        assert_eq!(payload.duplicate_grid_cells, 0);

        let ofst = table(&plugin, "OFST");
        let clsz = table(&plugin, "CLSZ");
        assert_eq!(ofst.len(), 25);
        assert_eq!(clsz.len(), 25);

        let bytes = build_plugin_bytes(&mut plugin).expect("serialize");
        let (wrld_pos, cells) = scan_serialized(&bytes);

        let grid = Grid {
            min_x: -2,
            min_y: -2,
            width: 5,
            height: 5,
        };
        let mut nonzero = 0;
        for (x, y, cell_id) in [(0i64, 0i64, 0x200u32), (1, 0, 0x201), (0, 1, 0x202)] {
            let idx = grid.index_of(x, y).unwrap();
            let target = wrld_pos + ofst[idx] as usize;
            assert_eq!(&bytes[target..target + 4], b"CELL", "OFST({x},{y})");
            let form_id = u32::from_le_bytes(bytes[target + 12..target + 16].try_into().unwrap());
            assert_eq!(form_id, cell_id, "OFST({x},{y}) form id");
            let dsize =
                u32::from_le_bytes(bytes[target + 4..target + 8].try_into().unwrap()) as usize;
            let rec_total = MODERN_HEADER_SIZE + dsize;
            let gsize = u32::from_le_bytes(
                bytes[target + rec_total + 4..target + rec_total + 8]
                    .try_into()
                    .unwrap(),
            ) as usize;
            assert_eq!(
                clsz[idx] as usize,
                rec_total + gsize + 24,
                "CLSZ({x},{y}) must be record + children group + 24"
            );
            nonzero += 1;
        }
        assert_eq!(
            ofst.iter().filter(|v| **v != 0).count(),
            nonzero,
            "no stray nonzero OFST entries"
        );
        // The persistent cell (also XCLC 0,0) must not win the (0,0) slot.
        let persistent_pos = cells.iter().find(|(id, _)| *id == 0x100).unwrap().1;
        let idx00 = grid.index_of(0, 0).unwrap();
        assert_ne!(wrld_pos + ofst[idx00] as usize, persistent_pos);
    }

    #[test]
    fn lazy_compressed_cells_index_without_touching_raw_payload() {
        let mut plugin = worldspace_plugin(-2, 2, true);
        let payload = rebuild_worldspace_cell_offsets(&mut plugin).expect("rebuild");
        assert_eq!(payload.cells_indexed, 3, "warnings={:?}", payload.warnings);
        assert_eq!(payload.cells_missing_xclc, 0);

        for item in &plugin.root_items {
            let ParsedItem::Group(g) = item else { continue };
            fn check(items: &[ParsedItem]) {
                for item in items {
                    match item {
                        ParsedItem::Record(r) if r.signature.as_str() == "CELL" => {
                            assert!(r.raw_payload.is_some(), "CELL raw_payload consumed");
                            assert!(r.subrecords.is_empty(), "CELL subrecords materialized");
                        }
                        ParsedItem::Group(g) => check(&g.children),
                        _ => {}
                    }
                }
            }
            check(&g.children);
        }
    }

    #[test]
    fn idempotent_rebuild_produces_identical_bytes() {
        let mut plugin = worldspace_plugin(-2, 2, false);
        rebuild_worldspace_cell_offsets(&mut plugin).expect("first rebuild");
        let first = build_plugin_bytes(&mut plugin).expect("serialize");
        rebuild_worldspace_cell_offsets(&mut plugin).expect("second rebuild");
        let second = build_plugin_bytes(&mut plugin).expect("serialize");
        assert_eq!(first, second);
    }

    #[test]
    fn rebuild_after_save_and_lazy_reload_is_byte_stable() {
        let mut plugin = worldspace_plugin(-2, 2, true);
        rebuild_worldspace_cell_offsets(&mut plugin).expect("rebuild");
        let bytes = build_plugin_bytes(&mut plugin).expect("serialize");

        let dir = std::env::temp_dir().join("esp_worldspace_offsets_test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("roundtrip_{}.esm", std::process::id()));
        std::fs::write(&path, &bytes).unwrap();
        let mut reloaded =
            parse_plugin_file_lazy_compressed(path.to_str().unwrap(), Some("fo4".to_string()))
                .expect("reload");
        std::fs::remove_file(&path).ok();

        rebuild_worldspace_cell_offsets(&mut reloaded).expect("rebuild on reload");
        let bytes2 = build_plugin_bytes(&mut reloaded).expect("serialize reloaded");
        assert_eq!(bytes, bytes2);
    }

    #[test]
    fn streaming_writer_matches_buffered_for_rebuilt_plugin() {
        let mut plugin = worldspace_plugin(-2, 2, false);
        rebuild_worldspace_cell_offsets(&mut plugin).expect("rebuild");
        let buffered = build_plugin_bytes(&mut plugin).expect("buffered");
        let mut streamed = Vec::new();
        write_plugin_to(&mut plugin, &mut streamed).expect("streamed");
        assert_eq!(buffered, streamed);
    }

    #[test]
    fn large_grid_tables_round_trip_through_xxxx_framing() {
        // 129x129 grid: 66564-byte tables force XXXX framing.
        let mut plugin = worldspace_plugin(-64, 64, false);
        rebuild_worldspace_cell_offsets(&mut plugin).expect("rebuild");
        let entries = 129usize * 129;
        assert_eq!(table(&plugin, "OFST").len(), entries);

        let bytes = build_plugin_bytes(&mut plugin).expect("serialize");
        assert!(
            bytes.windows(4).any(|w| w == b"XXXX"),
            "expected XXXX framing"
        );
        let (wrld_pos, _) = scan_serialized(&bytes);
        let dsize = u32::from_le_bytes(bytes[wrld_pos + 4..wrld_pos + 8].try_into().unwrap());
        let payload = Bytes::copy_from_slice(
            &bytes[wrld_pos + MODERN_HEADER_SIZE..wrld_pos + MODERN_HEADER_SIZE + dsize as usize],
        );
        let subs = split_record_payload(&payload).expect("parse WRLD payload");
        let ofst = subs
            .iter()
            .find(|s| s.signature.as_str() == "OFST")
            .expect("OFST after round trip");
        assert_eq!(ofst.data.len(), entries * 4);
        let tail: Vec<&str> = subs
            .iter()
            .rev()
            .take(2)
            .map(|s| s.signature.as_str())
            .collect();
        assert_eq!(
            tail,
            ["CLSZ", "OFST"],
            "tables must be the last two subrecords"
        );
    }

    #[test]
    fn small_grid_tables_use_plain_framing() {
        let mut plugin = worldspace_plugin(-2, 2, false);
        rebuild_worldspace_cell_offsets(&mut plugin).expect("rebuild");
        let bytes = build_plugin_bytes(&mut plugin).expect("serialize");
        assert!(!bytes.windows(4).any(|w| w == b"XXXX"));
    }

    #[test]
    fn unaligned_bounds_skip_with_warning() {
        let mut plugin = worldspace_plugin(-2, 2, false);
        if let ParsedItem::Group(g) = &mut plugin.root_items[0] {
            if let ParsedItem::Record(r) = &mut g.children[0] {
                let nam0 = r
                    .subrecords
                    .iter_mut()
                    .find(|s| s.signature.as_str() == "NAM0")
                    .unwrap();
                nam0.data =
                    Bytes::from([(-8200.5f32).to_le_bytes(), (-8192.0f32).to_le_bytes()].concat());
            }
        }
        let payload = rebuild_worldspace_cell_offsets(&mut plugin).expect("rebuild");
        assert_eq!(payload.worldspaces_rebuilt, 0);
        assert!(payload.warnings.iter().any(|w| w.contains("cell-aligned")));
        assert!(
            wrld_record(&plugin)
                .subrecords
                .iter()
                .all(|s| s.signature.as_str() != "OFST")
        );
    }

    #[test]
    fn missing_nam_bounds_skip_with_warning() {
        let mut plugin = worldspace_plugin(-2, 2, false);
        if let ParsedItem::Group(g) = &mut plugin.root_items[0] {
            if let ParsedItem::Record(r) = &mut g.children[0] {
                r.subrecords.retain(|s| s.signature.as_str() != "NAM0");
            }
        }
        let payload = rebuild_worldspace_cell_offsets(&mut plugin).expect("rebuild");
        assert_eq!(payload.worldspaces_rebuilt, 0);
        assert!(payload.warnings.iter().any(|w| w.contains("missing NAM0")));
    }

    #[test]
    fn out_of_rect_cells_are_counted_not_indexed() {
        // Grid only covers -1..1 but cells sit at (0,0),(1,0),(0,1) plus none
        // outside; shrink to 0..0 so (1,0)/(0,1) fall outside.
        let mut plugin = worldspace_plugin(0, 0, false);
        let payload = rebuild_worldspace_cell_offsets(&mut plugin).expect("rebuild");
        assert_eq!(payload.cells_indexed, 1);
        assert_eq!(payload.cells_out_of_rect, 2);
        assert_eq!(table(&plugin, "OFST").len(), 1);
    }

    #[test]
    fn duplicate_grid_cells_last_wins_and_counted() {
        let mut plugin = worldspace_plugin(-2, 2, false);
        if let ParsedItem::Group(top) = &mut plugin.root_items[0] {
            if let ParsedItem::Group(children) = &mut top.children[1] {
                if let ParsedItem::Group(block) = &mut children.children[2] {
                    if let ParsedItem::Group(sub_block) = &mut block.children[0] {
                        sub_block
                            .children
                            .push(compressed_cell(0x999, 0, 0, 4, false));
                    }
                }
            }
        }
        let payload = rebuild_worldspace_cell_offsets(&mut plugin).expect("rebuild");
        assert_eq!(payload.duplicate_grid_cells, 1);
        assert_eq!(payload.cells_indexed, 4);
    }

    #[test]
    fn blockless_worldspace_gets_no_tables_and_sheds_stale_ones() {
        let mut plugin = worldspace_plugin(-2, 2, false);
        if let ParsedItem::Group(top) = &mut plugin.root_items[0] {
            if let ParsedItem::Record(r) = &mut top.children[0] {
                r.subrecords.push(sub("OFST", vec![1, 2, 3, 4]));
                r.subrecords.push(sub("CLSZ", vec![1, 2, 3, 4]));
            }
            if let ParsedItem::Group(children) = &mut top.children[1] {
                children.children.retain(
                    |c| !matches!(c, ParsedItem::Group(g) if g.group_type == EXTERIOR_CELL_BLOCK),
                );
            }
        }
        let payload = rebuild_worldspace_cell_offsets(&mut plugin).expect("rebuild");
        assert_eq!(payload.worldspaces_rebuilt, 0);
        assert!(payload.warnings.is_empty());
        assert!(
            wrld_record(&plugin)
                .subrecords
                .iter()
                .all(|s| !matches!(s.signature.as_str(), "OFST" | "CLSZ"))
        );
    }

    #[test]
    fn cell_without_children_group_sizes_record_plus_pad() {
        let mut plugin = worldspace_plugin(-2, 2, false);
        if let ParsedItem::Group(top) = &mut plugin.root_items[0] {
            if let ParsedItem::Group(children) = &mut top.children[1] {
                if let ParsedItem::Group(block) = &mut children.children[2] {
                    if let ParsedItem::Group(sub_block) = &mut block.children[0] {
                        sub_block
                            .children
                            .push(compressed_cell(0x400, 2, 2, 4, false));
                    }
                }
            }
        }
        let payload = rebuild_worldspace_cell_offsets(&mut plugin).expect("rebuild");
        assert_eq!(payload.cells_indexed, 4);

        let ofst = table(&plugin, "OFST");
        let clsz = table(&plugin, "CLSZ");
        let grid = Grid {
            min_x: -2,
            min_y: -2,
            width: 5,
            height: 5,
        };
        let idx = grid.index_of(2, 2).unwrap();
        let bytes = build_plugin_bytes(&mut plugin).expect("serialize");
        let (wrld_pos, _) = scan_serialized(&bytes);
        let target = wrld_pos + ofst[idx] as usize;
        assert_eq!(&bytes[target..target + 4], b"CELL");
        let dsize = u32::from_le_bytes(bytes[target + 4..target + 8].try_into().unwrap()) as usize;
        assert_eq!(clsz[idx] as usize, MODERN_HEADER_SIZE + dsize + 24);
    }

    #[test]
    fn missing_xclc_counted_and_skipped() {
        let mut plugin = worldspace_plugin(-2, 2, false);
        if let ParsedItem::Group(top) = &mut plugin.root_items[0] {
            if let ParsedItem::Group(children) = &mut top.children[1] {
                if let ParsedItem::Group(block) = &mut children.children[2] {
                    if let ParsedItem::Group(sub_block) = &mut block.children[0] {
                        sub_block.children.push(record(
                            "CELL",
                            0x500,
                            vec![sub("DATA", vec![2, 0])],
                        ));
                    }
                }
            }
        }
        let payload = rebuild_worldspace_cell_offsets(&mut plugin).expect("rebuild");
        assert_eq!(payload.cells_missing_xclc, 1);
        assert_eq!(payload.cells_indexed, 3);
    }
}
