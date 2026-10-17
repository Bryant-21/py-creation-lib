//! CELL previs/precombine metadata (VISI, RVIS, PCMB, XPRI, XCRI).

use std::io::{BufReader, BufWriter, Read, Seek, Write};
#[cfg(test)]
use std::io::SeekFrom;
use std::path::Path;

use rustc_hash::FxHashMap;

use crate::error::{PrevisError, Result};
use crate::plugin::{COMPRESSED_FLAG, HEADER_SIZE, Plugin, SubrecordIter};

pub(crate) const REPLACED: [&[u8; 4]; 5] = [b"PCMB", b"RVIS", b"VISI", b"XCRI", b"XPRI"];

/// CELL records in the plugin (overrides included: the winning record's
/// plugin owns a CELL's precombine files) carrying any precombine or previs stamp.
pub fn stamped_cells(plugin: &Plugin) -> Result<Vec<u32>> {
    let mut cells = Vec::new();
    for record in plugin.records_of(b"CELL") {
        if plugin.subrecords_at(&record)?.iter().any(|(signature, _)| REPLACED.contains(&&signature)) {
            cells.push(record.form_id);
        }
    }
    Ok(cells)
}

/// CK's packed generation date: `(year - 2000) << 9 | month << 5 | day`.
pub fn pack_generation_date(year: i32, month: u32, day: u32) -> Result<u16> {
    let year_offset = year - 2000;
    if !(0..=0x7F).contains(&year_offset) || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return Err(PrevisError::invalid("CK generation dates support 2000-01-01..2127-12-31"));
    }
    Ok(((year_offset as u16) << 9) | ((month as u16) << 5) | day as u16)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Xcri {
    pub mesh_ids: Vec<u32>,
    pub reference_mesh_pairs: Vec<(u32, u32)>,
}

impl Xcri {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(8 + 4 * self.mesh_ids.len() + 8 * self.reference_mesh_pairs.len());
        out.extend_from_slice(&(self.mesh_ids.len() as u32).to_le_bytes());
        out.extend_from_slice(&((self.reference_mesh_pairs.len() * 2) as u32).to_le_bytes());
        for id in &self.mesh_ids {
            out.extend_from_slice(&id.to_le_bytes());
        }
        for (reference, mesh) in &self.reference_mesh_pairs {
            out.extend_from_slice(&reference.to_le_bytes());
            out.extend_from_slice(&mesh.to_le_bytes());
        }
        out
    }

    /// With previs, CK writes pairs grouped by mesh-ID order, sorted within.
    /// Retail has pairs naming a mesh outside the list (Fallout4.esm 00E071
    /// maps a ref to mesh 0); those are kept, after the grouped ones.
    pub fn normalized_for_previs(&self) -> Xcri {
        let mut sorted = self.reference_mesh_pairs.clone();
        sorted.sort();
        let mut pairs: Vec<(u32, u32)> = self
            .mesh_ids
            .iter()
            .flat_map(|mesh| sorted.iter().copied().filter(move |pair| pair.1 == *mesh))
            .collect();
        pairs.extend(sorted.iter().copied().filter(|pair| !self.mesh_ids.contains(&pair.1)));
        Xcri {
            mesh_ids: self.mesh_ids.clone(),
            reference_mesh_pairs: pairs,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct CellMetadata {
    pub previs: bool,
    pub precombine: bool,
    pub root_visibility_cell: Option<u32>,
    pub previs_reference_ids: Vec<u32>,
    pub xcri: Option<Xcri>,
}

pub(crate) fn encode_subrecord(out: &mut Vec<u8>, signature: &[u8; 4], payload: &[u8]) {
    if payload.len() <= 0xFFFF {
        out.extend_from_slice(signature);
        out.extend_from_slice(&(payload.len() as u16).to_le_bytes());
    } else {
        out.extend_from_slice(b"XXXX\x04\0");
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(signature);
        out.extend_from_slice(&[0, 0]);
    }
    out.extend_from_slice(payload);
}

/// Rewrites one complete CELL record with the requested metadata.
pub fn patch_cell_record(record: &[u8], decoded: &[u8], metadata: &CellMetadata, packed_date: u16) -> Result<Vec<u8>> {
    if record.len() < HEADER_SIZE || &record[..4] != b"CELL" {
        return Err(PrevisError::invalid("input is not a complete CELL record"));
    }
    if !metadata.previs && (metadata.root_visibility_cell.is_some() || !metadata.previs_reference_ids.is_empty()) {
        return Err(PrevisError::invalid("RVIS/XPRI metadata requires previs"));
    }
    if metadata.xcri.is_some() && !metadata.precombine {
        return Err(PrevisError::invalid("XCRI metadata requires precombine"));
    }
    let flags = u32::from_le_bytes(record[8..12].try_into().unwrap());
    let retained: Vec<([u8; 4], &[u8])> = SubrecordIter::new(decoded)
        .filter(|(signature, _)| !REPLACED.contains(&signature))
        .collect();
    let date = packed_date.to_le_bytes();
    let root = metadata.root_visibility_cell.map(u32::to_le_bytes);
    let mut inserted: Vec<([u8; 4], &[u8])> = Vec::new();
    if metadata.previs {
        inserted.push((*b"VISI", &date));
        if let Some(root) = &root {
            inserted.push((*b"RVIS", root));
        }
    }
    if metadata.precombine {
        inserted.push((*b"PCMB", &date));
    }
    let insertion = retained
        .iter()
        .position(|(signature, _)| signature == b"DATA")
        .map_or(retained.len(), |index| index + 1);

    let mut payload = Vec::with_capacity(decoded.len() + 64);
    for (signature, data) in retained[..insertion].iter().chain(&inserted).chain(&retained[insertion..]) {
        encode_subrecord(&mut payload, signature, data);
    }
    if !metadata.previs_reference_ids.is_empty() {
        let ids: Vec<u8> = metadata.previs_reference_ids.iter().flat_map(|id| id.to_le_bytes()).collect();
        encode_subrecord(&mut payload, b"XPRI", &ids);
    }
    if let Some(xcri) = &metadata.xcri {
        let written = if metadata.previs { xcri.normalized_for_previs() } else { xcri.clone() };
        encode_subrecord(&mut payload, b"XCRI", &written.encode());
    }
    if flags & COMPRESSED_FLAG != 0 {
        let mut compressed = (payload.len() as u32).to_le_bytes().to_vec();
        let mut encoder = flate2::write::ZlibEncoder::new(&mut compressed, flate2::Compression::default());
        encoder
            .write_all(&payload)
            .and_then(|_| encoder.finish().map(|_| ()))
            .map_err(|e| PrevisError::io("compress CELL", e))?;
        payload = compressed;
    }
    let mut out = record[..HEADER_SIZE].to_vec();
    out[4..8].copy_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&payload);
    Ok(out)
}

const CELL_CHILDREN_GROUP: u32 = 6;

/// While a CELL's records load, the runtime wipes its precalc data (XCRI,
/// VISI, PCMB) for any record whose version-control date is 0 or newer than
/// PCMB (1.11.221 0x2F1050 -> ClearPreCalcedData). Converted records carry
/// no date, so a stamped CELL and everything under it get the stamp's date.
fn date_version_control(header: &mut [u8], packed_date: u16) {
    let current = u16::from_le_bytes(header[16..18].try_into().unwrap());
    if current == 0 || current > packed_date {
        header[16..18].copy_from_slice(&packed_date.to_le_bytes());
    }
}

/// Writes `plugin` to `output` with patched CELL records, fixing group sizes.
pub fn write_patched_plugin(
    plugin: &Plugin,
    output: &Path,
    cells: &FxHashMap<u32, CellMetadata>,
    packed_date: u16,
) -> Result<usize> {
    let file = std::fs::File::create(output).map_err(|e| PrevisError::io(output.display().to_string(), e))?;
    let mut writer = BufWriter::with_capacity(1 << 20, file);
    let data = plugin.bytes();
    let header_size = HEADER_SIZE + u32::from_le_bytes(data[4..8].try_into().unwrap()) as usize;
    let io = |e| PrevisError::io(output.display().to_string(), e);
    writer.write_all(&data[..header_size]).map_err(io)?;
    let directory = output.parent().filter(|path| !path.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
    let mut replacements = tempfile::tempfile_in(directory).map_err(io)?;
    let mut group_sizes = FxHashMap::default();
    let mut patched = 0;
    {
        let mut spool = BufWriter::with_capacity(1 << 20, &mut replacements);
        plan_region(plugin, &mut spool, header_size, data.len(), cells, packed_date, &mut group_sizes, &mut patched)?;
        spool.flush().map_err(io)?;
    }
    replacements.rewind().map_err(io)?;
    let mut spool = BufReader::with_capacity(1 << 20, replacements);
    copy_region(data, &mut writer, &mut spool, header_size, data.len(), cells, packed_date, false, &group_sizes)?;
    writer.flush().map_err(io)?;
    if patched != cells.len() {
        return Err(PrevisError::invalid(format!(
            "patched {patched} CELL records but {} were requested",
            cells.len()
        )));
    }
    Ok(patched)
}

// Spool only changed CELLs; keeping all replacements or a whole WRLD in memory
// would trade the seek bottleneck for another peak on small machines.
fn plan_region(
    plugin: &Plugin,
    spool: &mut impl Write,
    start: usize,
    end: usize,
    cells: &FxHashMap<u32, CellMetadata>,
    packed_date: u16,
    group_sizes: &mut FxHashMap<usize, u32>,
    patched: &mut usize,
) -> Result<u64> {
    let data = plugin.bytes();
    let io = |e| PrevisError::io("spool CELL stamps", e);
    let mut written = 0u64;
    let mut cursor = start;
    while cursor < end {
        let size = u32::from_le_bytes(data[cursor + 4..cursor + 8].try_into().unwrap()) as usize;
        if &data[cursor..cursor + 4] == b"GRUP" {
            let total = HEADER_SIZE as u64 + plan_region(plugin, spool, cursor + HEADER_SIZE, cursor + size, cells, packed_date, group_sizes, patched)?;
            if total != size as u64 {
                let total = u32::try_from(total).map_err(|_| PrevisError::invalid("stamped GRUP exceeds 4 GiB"))?;
                group_sizes.insert(cursor, total);
            }
            written += total;
            cursor += size;
            continue;
        }
        let total = HEADER_SIZE + size;
        let form_id = u32::from_le_bytes(data[cursor + 12..cursor + 16].try_into().unwrap());
        if let Some(metadata) = cells.get(&form_id).filter(|_| &data[cursor..cursor + 4] == b"CELL") {
            let reference = plugin.record(form_id).ok_or_else(|| PrevisError::invalid(format!("CELL {form_id:08X} is not indexed")))?;
            let decoded = plugin.subrecords_at(&reference)?;
            let mut replacement = patch_cell_record(&data[cursor..cursor + total], decoded.raw(), metadata, packed_date)?;
            date_version_control(&mut replacement, packed_date);
            spool.write_all(&(replacement.len() as u64).to_le_bytes()).map_err(io)?;
            spool.write_all(&replacement).map_err(io)?;
            written += replacement.len() as u64;
            *patched += 1;
        } else {
            written += total as u64;
        }
        cursor += total;
    }
    Ok(written)
}

fn copy_region(
    data: &[u8],
    writer: &mut impl Write,
    spool: &mut impl Read,
    start: usize,
    end: usize,
    cells: &FxHashMap<u32, CellMetadata>,
    packed_date: u16,
    in_stamped_cell: bool,
    group_sizes: &FxHashMap<usize, u32>,
) -> Result<()> {
    let io = |e| PrevisError::io("write plugin", e);
    let mut cursor = start;
    while cursor < end {
        let size = u32::from_le_bytes(data[cursor + 4..cursor + 8].try_into().unwrap()) as usize;
        if &data[cursor..cursor + 4] == b"GRUP" {
            let mut header = [0u8; HEADER_SIZE];
            header.copy_from_slice(&data[cursor..cursor + HEADER_SIZE]);
            if let Some(size) = group_sizes.get(&cursor) {
                header[4..8].copy_from_slice(&size.to_le_bytes());
            }
            writer.write_all(&header).map_err(io)?;
            let label = u32::from_le_bytes(header[8..12].try_into().unwrap());
            let group_type = u32::from_le_bytes(header[12..16].try_into().unwrap());
            let stamped = in_stamped_cell || (group_type == CELL_CHILDREN_GROUP && cells.contains_key(&label));
            copy_region(data, writer, spool, cursor + HEADER_SIZE, cursor + size, cells, packed_date, stamped, group_sizes)?;
            cursor += size;
            continue;
        }
        let total = HEADER_SIZE + size;
        let form_id = u32::from_le_bytes(data[cursor + 12..cursor + 16].try_into().unwrap());
        let record = &data[cursor..cursor + total];
        if &record[..4] == b"CELL" && cells.contains_key(&form_id) {
            let mut length = [0u8; 8];
            spool.read_exact(&mut length).map_err(io)?;
            let length = u64::from_le_bytes(length);
            if std::io::copy(&mut spool.take(length), writer).map_err(io)? != length {
                return Err(PrevisError::invalid("CELL stamp spool is truncated"));
            }
        } else if in_stamped_cell {
            let mut header = [0u8; HEADER_SIZE];
            header.copy_from_slice(&record[..HEADER_SIZE]);
            date_version_control(&mut header, packed_date);
            writer.write_all(&header).map_err(io)?;
            writer.write_all(&record[HEADER_SIZE..]).map_err(io)?;
        } else {
            writer.write_all(record).map_err(io)?;
        }
        cursor += total;
    }
    Ok(())
}

#[cfg(test)]
fn copy_region_legacy<W: Write + Seek>(
    plugin: &Plugin,
    writer: &mut W,
    start: usize,
    end: usize,
    cells: &FxHashMap<u32, CellMetadata>,
    packed_date: u16,
    in_stamped_cell: bool,
    patched: &mut usize,
) -> Result<u64> {
    let data = plugin.bytes();
    let io = |e| PrevisError::io("write plugin", e);
    let mut written = 0u64;
    let mut cursor = start;
    while cursor < end {
        let size = u32::from_le_bytes(data[cursor + 4..cursor + 8].try_into().unwrap()) as usize;
        if &data[cursor..cursor + 4] == b"GRUP" {
            let header_position = writer.stream_position().map_err(io)?;
            writer.write_all(&data[cursor..cursor + HEADER_SIZE]).map_err(io)?;
            let label = u32::from_le_bytes(data[cursor + 8..cursor + 12].try_into().unwrap());
            let group_type = u32::from_le_bytes(data[cursor + 12..cursor + 16].try_into().unwrap());
            let stamped = in_stamped_cell || (group_type == CELL_CHILDREN_GROUP && cells.contains_key(&label));
            let children =
                copy_region_legacy(plugin, writer, cursor + HEADER_SIZE, cursor + size, cells, packed_date, stamped, patched)?;
            let total = HEADER_SIZE as u64 + children;
            if total != size as u64 {
                let resume = writer.stream_position().map_err(io)?;
                writer.seek(SeekFrom::Start(header_position + 4)).map_err(io)?;
                writer.write_all(&(total as u32).to_le_bytes()).map_err(io)?;
                writer.seek(SeekFrom::Start(resume)).map_err(io)?;
            }
            written += total;
            cursor += size;
            continue;
        }
        let total = HEADER_SIZE + size;
        let form_id = u32::from_le_bytes(data[cursor + 12..cursor + 16].try_into().unwrap());
        let record = &data[cursor..cursor + total];
        match cells.get(&form_id).filter(|_| &data[cursor..cursor + 4] == b"CELL") {
            Some(metadata) => {
                let reference = plugin
                    .record(form_id)
                    .ok_or_else(|| PrevisError::invalid(format!("CELL {form_id:08X} is not indexed")))?;
                let decoded = plugin.subrecords_at(&reference)?;
                let mut replacement = patch_cell_record(record, decoded.raw(), metadata, packed_date)?;
                date_version_control(&mut replacement, packed_date);
                writer.write_all(&replacement).map_err(io)?;
                written += replacement.len() as u64;
                *patched += 1;
            }
            None if in_stamped_cell => {
                let mut header = [0u8; HEADER_SIZE];
                header.copy_from_slice(&record[..HEADER_SIZE]);
                date_version_control(&mut header, packed_date);
                writer.write_all(&header).map_err(io)?;
                writer.write_all(&record[HEADER_SIZE..]).map_err(io)?;
                written += total as u64;
            }
            None => {
                writer.write_all(record).map_err(io)?;
                written += total as u64;
            }
        }
        cursor += total;
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(subrecords: &[(&[u8; 4], &[u8])]) -> Vec<u8> {
        let mut payload = Vec::new();
        for (signature, data) in subrecords {
            encode_subrecord(&mut payload, signature, data);
        }
        let mut out = b"CELL".to_vec();
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&0x0100_0800u32.to_le_bytes());
        out.extend_from_slice(&[0; 8]);
        out.extend_from_slice(&payload);
        out
    }

    #[test]
    fn version_control_dates_never_postdate_the_stamp() {
        let date = pack_generation_date(2026, 9, 24).unwrap();
        let mut undated = [0u8; HEADER_SIZE];
        undated[18] = 0x2A;
        date_version_control(&mut undated, date);
        assert_eq!(u16::from_le_bytes([undated[16], undated[17]]), date);
        assert_eq!(undated[18], 0x2A, "user ids in the upper half are kept");

        let older = pack_generation_date(2016, 5, 24).unwrap();
        let mut retail = [0u8; HEADER_SIZE];
        retail[16..18].copy_from_slice(&older.to_le_bytes());
        date_version_control(&mut retail, date);
        assert_eq!(u16::from_le_bytes([retail[16], retail[17]]), older);
    }

    #[test]
    fn generation_date_packing_matches_ck() {
        assert_eq!(pack_generation_date(2026, 9, 22).unwrap(), (26 << 9) | (9 << 5) | 22);
    }

    #[test]
    fn previs_metadata_is_inserted_after_data_and_xpri_appended() {
        let source = record(&[(b"EDID", b"C\0"), (b"DATA", &[0, 0]), (b"XCLC", &[0; 12]), (b"VISI", &[9, 9])]);
        let metadata = CellMetadata {
            previs: true,
            root_visibility_cell: Some(0x0100_0800),
            previs_reference_ids: vec![0x0100_0801],
            ..CellMetadata::default()
        };
        let patched = patch_cell_record(&source, &source[HEADER_SIZE..], &metadata, 0x3536).unwrap();
        let signatures: Vec<[u8; 4]> = SubrecordIter::new(&patched[HEADER_SIZE..]).map(|(s, _)| s).collect();
        assert_eq!(signatures, [*b"EDID", *b"DATA", *b"VISI", *b"RVIS", *b"XCLC", *b"XPRI"]);
    }

    #[test]
    fn previs_xcri_pairs_follow_mesh_order() {
        let xcri = Xcri {
            mesh_ids: vec![2, 1],
            reference_mesh_pairs: vec![(10, 1), (12, 2), (11, 2)],
        };
        assert_eq!(xcri.normalized_for_previs().reference_mesh_pairs, vec![(11, 2), (12, 2), (10, 1)]);
    }

    #[test]
    fn previs_xcri_keeps_pairs_naming_an_unlisted_mesh() {
        let xcri = Xcri {
            mesh_ids: vec![2],
            reference_mesh_pairs: vec![(10, 0), (12, 2)],
        };
        assert_eq!(xcri.normalized_for_previs().reference_mesh_pairs, vec![(12, 2), (10, 0)]);
    }

    #[test]
    fn sequential_stamps_match_legacy_nested_groups_and_compression() {
        fn group(kind: u32, label: u32, body: &[u8]) -> Vec<u8> {
            let mut bytes = b"GRUP".to_vec();
            bytes.extend_from_slice(&((HEADER_SIZE + body.len()) as u32).to_le_bytes());
            bytes.extend_from_slice(&label.to_le_bytes());
            bytes.extend_from_slice(&kind.to_le_bytes());
            bytes.extend_from_slice(&[0; 8]);
            bytes.extend_from_slice(body);
            bytes
        }
        for compressed in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let mut cell = record(&[(b"DATA", &[1, 0]), (b"XPRI", &[7; 80])]);
            cell[12..16].copy_from_slice(&0x800u32.to_le_bytes());
            if compressed {
                let mut payload = (cell.len() as u32 - HEADER_SIZE as u32).to_le_bytes().to_vec();
                let mut compressor = flate2::write::ZlibEncoder::new(&mut payload, flate2::Compression::default());
                compressor.write_all(&cell[HEADER_SIZE..]).unwrap();
                compressor.finish().unwrap();
                cell.truncate(HEADER_SIZE);
                cell[4..8].copy_from_slice(&(payload.len() as u32).to_le_bytes());
                cell[8..12].copy_from_slice(&COMPRESSED_FLAG.to_le_bytes());
                cell.extend(payload);
            }
            let mut child = record(&[(b"NAME", &[1, 2, 3, 4])]);
            child[..4].copy_from_slice(b"REFR");
            child[12..16].copy_from_slice(&0x801u32.to_le_bytes());
            let children = group(6, 0x800, &group(9, 0x800, &child));
            cell.extend(children);
            let mut bytes = record(&[(b"HEDR", &[0; 12])]);
            bytes[..4].copy_from_slice(b"TES4");
            bytes[12..16].fill(0);
            let header_size = bytes.len();
            bytes.extend(group(0, u32::from_le_bytes(*b"CELL"), &group(2, 0, &group(3, 0, &cell))));
            let input = dir.path().join("Input.esp");
            std::fs::write(&input, &bytes).unwrap();
            let plugin = Plugin::open(&input).unwrap();
            for metadata in [CellMetadata::default(), CellMetadata { previs: true, previs_reference_ids: vec![0x801; 40000], ..CellMetadata::default() }] {
                let cells = FxHashMap::from_iter([(0x800, metadata)]);
                let mut expected = std::io::Cursor::new(bytes[..header_size].to_vec());
                expected.set_position(header_size as u64);
                let mut count = 0;
                copy_region_legacy(&plugin, &mut expected, header_size, bytes.len(), &cells, 0x3548, false, &mut count).unwrap();
                let output = dir.path().join("Output.esp");
                assert_eq!(write_patched_plugin(&plugin, &output, &cells, 0x3548).unwrap(), count);
                assert_eq!(std::fs::read(&output).unwrap(), expected.into_inner());
                let parsed = Plugin::open(&output).unwrap();
                assert!(parsed.record(0x800).is_some());
                assert!(parsed.record(0x801).is_some());
            }
            let missing = FxHashMap::from_iter([(0x802, CellMetadata::default())]);
            assert!(write_patched_plugin(&plugin, &dir.path().join("Missing.esp"), &missing, 0x3548).is_err());
        }
    }
}
