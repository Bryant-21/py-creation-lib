//! Stamps PCMB/XCRI and record-header dates onto a baked precombine result.
//! Pure ESP mutation; the [`BakedCell`] comes from `precombine::bake`.

use std::collections::HashMap;

use esp_authoring_core::plugin_runtime::{
    COMPRESSED_RECORD_FLAG, ParsedItem, ParsedRecord, ParsedSubrecord,
    decode_compressed_subrecords_from_payload, plugin_handle_store_ref,
};
use esp_authoring_core::xcri::{XcriReference, XcriTable, encode_fo4};
use esp_authoring_core::{RECORD_FLAG_NO_PREVIS, upsert_ordered_subrecord};

use super::bake::BakedCell;

#[derive(Debug)]
pub struct StampStats {
    pub refs_stamped: u32,
}

/// Vis/precombine subrecords dropped outright (no replacement) before the
/// new PCMB/XCRI are inserted.
const STALE_VIS_SUBRECORDS: [&str; 3] = ["VISI", "RVIS", "XPRI"];

pub fn stamp_cell(
    target_handle_id: u64,
    baked: &BakedCell,
    pcmb_date: u16,
    no_previs: bool,
) -> Result<StampStats, String> {
    let mut store = plugin_handle_store_ref().lock().unwrap();
    let slot = store
        .get_mut(&target_handle_id)
        .ok_or_else(|| format!("no plugin handle: {target_handle_id}"))?;

    // own_index must fit the XCRI reference's top byte (own_index << 24).
    let masters_len = slot.parsed.header.masters.len();
    if masters_len > 0xFF {
        return Err(format!(
            "own index overflow: {masters_len} masters exceeds the 1-byte XCRI reference field"
        ));
    }
    let own_index = masters_len as u8;

    // Preflight: the CELL and every baked ref must exist, be owned by this
    // plugin (top byte == own_index), and, if compressed and lazily loaded,
    // decode cleanly, all before any mutation, so a failure leaves the plugin
    // untouched. A lazily loaded compressed record has empty `subrecords` and
    // its content only in `raw_payload`; editing `subrecords` directly would
    // drop everything else it carries, so bodies are decoded into a side
    // table here and installed in the mutate phase below.
    let cell_inflate = {
        let cell = find_record_mut(&mut slot.parsed.root_items, "CELL", baked.cell_form_id)
            .ok_or_else(|| format!("unknown cell: {:08X}", baked.cell_form_id))?;
        inflate_if_needed(cell)?
    };
    let mut refr_inflates: HashMap<u32, Vec<ParsedSubrecord>> = HashMap::new();
    for mesh in &baked.meshes {
        for &refr_form_id in &mesh.refs {
            if (refr_form_id >> 24) != u32::from(own_index) {
                return Err(format!(
                    "reference {refr_form_id:08X} is not owned by this plugin (own index {own_index:02X})"
                ));
            }
            let refr = find_record_mut(&mut slot.parsed.root_items, "REFR", refr_form_id)
                .ok_or_else(|| format!("unknown reference: {refr_form_id:08X}"))?;
            if let Some(subrecords) = inflate_if_needed(refr)? {
                refr_inflates.insert(refr_form_id, subrecords);
            }
        }
    }

    let mut mesh_ids: Vec<u32> = baked.meshes.iter().map(|mesh| mesh.mesh_id).collect();
    mesh_ids.sort_unstable();

    // Post-preflight, every refr_form_id's top byte already equals
    // own_index, so it doubles as the XCRI reference value verbatim.
    let mut rows: Vec<(u32, u32)> = Vec::new();
    for mesh in &baked.meshes {
        for &refr_form_id in &mesh.refs {
            rows.push((refr_form_id, mesh.mesh_id));
        }
    }
    rows.sort_unstable();

    let table = XcriTable {
        meshes: mesh_ids,
        references: rows
            .into_iter()
            .map(|(reference, mesh_id)| XcriReference { reference, mesh_id })
            .collect(),
    };
    let xcri_bytes =
        encode_fo4(&table).ok_or_else(|| "XCRI table exceeds encodable size".to_string())?;
    let pcmb_bytes = pcmb_date.to_le_bytes().to_vec();

    let cell = find_record_mut(&mut slot.parsed.root_items, "CELL", baked.cell_form_id)
        .expect("preflight already confirmed the cell exists");
    if let Some(subrecords) = cell_inflate {
        cell.subrecords = subrecords;
    }
    cell.subrecords
        .retain(|sub| !STALE_VIS_SUBRECORDS.contains(&sub.signature.as_str()));
    upsert_ordered_subrecord(cell, ordered_subrecord("PCMB", pcmb_bytes));
    upsert_ordered_subrecord(cell, ordered_subrecord("XCRI", xcri_bytes));
    if no_previs {
        cell.flags |= RECORD_FLAG_NO_PREVIS;
    } else {
        cell.flags &= !RECORD_FLAG_NO_PREVIS;
    }
    stamp_date(cell, pcmb_date);

    let mut refs_stamped = 0u32;
    for mesh in &baked.meshes {
        for &refr_form_id in &mesh.refs {
            let refr = find_record_mut(&mut slot.parsed.root_items, "REFR", refr_form_id)
                .expect("preflight already confirmed the reference exists");
            if let Some(subrecords) = refr_inflates.remove(&refr_form_id) {
                refr.subrecords = subrecords;
            }
            stamp_date(refr, pcmb_date);
            refs_stamped += 1;
        }
    }

    slot.invalidate_sections();

    Ok(StampStats { refs_stamped })
}

/// Constructs a [`ParsedSubrecord`] without naming `bytes`/`smol_str` types
/// directly — their `Into` impls are resolved from the field types on
/// `ParsedSubrecord`, so this crate needs no direct dependency on either.
fn ordered_subrecord(signature: &str, data: Vec<u8>) -> ParsedSubrecord {
    ParsedSubrecord {
        signature: signature.into(),
        data: data.into(),
        semantic_type: None,
    }
}

/// If `record` is compressed and lazily loaded (subrecords not materialized,
/// the common case in a multi-hundred-MB ESM), decode its `raw_payload` and
/// return the subrecords for the caller to install. `None` when uncompressed
/// or already materialized.
fn inflate_if_needed(record: &ParsedRecord) -> Result<Option<Vec<ParsedSubrecord>>, String> {
    if record.flags & COMPRESSED_RECORD_FLAG == 0 || !record.subrecords.is_empty() {
        return Ok(None);
    }
    let raw_payload = record.raw_payload.as_ref().ok_or_else(|| {
        format!(
            "compressed record {:08X} has no raw payload to inflate",
            record.form_id
        )
    })?;
    let decoded = decode_compressed_subrecords_from_payload(raw_payload).map_err(|err| {
        format!(
            "failed to inflate compressed record {:08X}: {err}",
            record.form_id
        )
    })?;
    Ok(Some(decoded.subrecords))
}

fn stamp_date(record: &mut ParsedRecord, pcmb_date: u16) {
    record.version_control = (record.version_control & 0xFFFF_0000) | u32::from(pcmb_date);
    record.raw_payload = None;
}

fn find_record_mut<'a>(
    items: &'a mut [ParsedItem],
    signature: &str,
    form_id: u32,
) -> Option<&'a mut ParsedRecord> {
    for item in items {
        match item {
            ParsedItem::Record(record)
                if record.signature.as_str() == signature && record.form_id == form_id =>
            {
                return Some(record);
            }
            ParsedItem::Group(group) => {
                if let Some(found) = find_record_mut(&mut group.children, signature, form_id) {
                    return Some(found);
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::super::bake::BakedMesh;
    use super::*;
    use bytes::Bytes;
    use esp_authoring_core::plugin_runtime::{
        compress_subrecords_payload, insert_parsed_record, plugin_handle_add_master_native,
        plugin_handle_new_native,
    };
    use smol_str::SmolStr;

    const CELL_INTERIOR_FLAG: u8 = 0x01;
    const SENTINEL_RAW_PAYLOAD: [u8; 2] = [0xAA, 0xBB];

    fn sub(sig: &str, data: Vec<u8>) -> ParsedSubrecord {
        ParsedSubrecord {
            signature: SmolStr::new(sig),
            data: Bytes::from(data),
            semantic_type: None,
        }
    }

    fn edid(name: &str) -> ParsedSubrecord {
        let mut bytes = name.as_bytes().to_vec();
        bytes.push(0);
        sub("EDID", bytes)
    }

    fn stale_cell(form_id: u32, version_control: u32) -> ParsedRecord {
        ParsedRecord {
            signature: SmolStr::new("CELL"),
            form_id,
            flags: RECORD_FLAG_NO_PREVIS,
            version_control,
            form_version: Some(131),
            version2: None,
            subrecords: vec![
                edid("StaleCell"),
                sub("DATA", vec![CELL_INTERIOR_FLAG, 0x00]),
                sub("VISI", vec![0u8; 8]),
                sub("RVIS", vec![0u8; 4]),
                sub("PCMB", vec![0xEF, 0xBE]),
                sub("XPRI", vec![0u8; 4]),
            ],
            raw_payload: Some(Bytes::from(SENTINEL_RAW_PAYLOAD.to_vec())),
            parse_error: None,
        }
    }

    fn refr(form_id: u32, version_control: u32) -> ParsedRecord {
        ParsedRecord {
            signature: SmolStr::new("REFR"),
            form_id,
            flags: 0,
            version_control,
            form_version: Some(131),
            version2: None,
            subrecords: vec![sub("NAME", vec![0u8; 4])],
            raw_payload: Some(Bytes::from(SENTINEL_RAW_PAYLOAD.to_vec())),
            parse_error: None,
        }
    }

    fn new_target() -> u64 {
        plugin_handle_new_native("Test.esm", Some("fo4")).expect("target handle")
    }

    fn find_installed<'a>(
        store: &'a std::sync::MutexGuard<
            '_,
            std::collections::HashMap<u64, esp_authoring_core::plugin_runtime::NativePluginSlot>,
        >,
        handle: u64,
        signature: &str,
        form_id: u32,
    ) -> &'a ParsedRecord {
        fn walk<'a>(
            items: &'a [ParsedItem],
            signature: &str,
            form_id: u32,
        ) -> Option<&'a ParsedRecord> {
            for item in items {
                match item {
                    ParsedItem::Record(record)
                        if record.signature.as_str() == signature && record.form_id == form_id =>
                    {
                        return Some(record);
                    }
                    ParsedItem::Group(group) => {
                        if let Some(found) = walk(&group.children, signature, form_id) {
                            return Some(found);
                        }
                    }
                    _ => {}
                }
            }
            None
        }
        let slot = store.get(&handle).expect("handle present");
        walk(&slot.parsed.root_items, signature, form_id).expect("record present")
    }

    #[test]
    fn stamps_cell_and_refs_with_exact_xcri_and_preserved_upper_vc_bits() {
        let target = new_target();
        insert_parsed_record(target, stale_cell(0x001000, 0xAAAA_0000)).unwrap();
        insert_parsed_record(target, refr(0x000600, 0xBBBB_0000)).unwrap();
        insert_parsed_record(target, refr(0x000601, 0xCCCC_1111)).unwrap();

        // Mesh ids and ref order deliberately reversed/unsorted on input.
        let baked = BakedCell {
            cell_form_id: 0x001000,
            meshes: vec![
                BakedMesh {
                    mesh_id: 0x2000,
                    refs: vec![0x000601],
                    rel_path: "meshes\\precombined\\test\\00100000_00002000_oc.nif".into(),
                },
                BakedMesh {
                    mesh_id: 0x1000,
                    refs: vec![0x000600],
                    rel_path: "meshes\\precombined\\test\\00100000_00001000_oc.nif".into(),
                },
            ],
        };

        let stats = stamp_cell(target, &baked, 0x1F24, true).expect("stamp succeeds");
        assert_eq!(stats.refs_stamped, 2);

        let store = plugin_handle_store_ref().lock().unwrap();
        let cell = find_installed(&store, target, "CELL", 0x001000);

        let sigs: Vec<&str> = cell
            .subrecords
            .iter()
            .map(|s| s.signature.as_str())
            .collect();
        assert_eq!(
            sigs,
            vec!["EDID", "DATA", "PCMB", "XCRI"],
            "VISI/RVIS/XPRI removed; PCMB/XCRI land at their ranked position"
        );

        let pcmb = cell
            .subrecords
            .iter()
            .find(|s| s.signature.as_str() == "PCMB")
            .unwrap();
        assert_eq!(pcmb.data.as_ref(), &0x1F24u16.to_le_bytes());

        let xcri = cell
            .subrecords
            .iter()
            .find(|s| s.signature.as_str() == "XCRI")
            .unwrap();
        let expected_table = XcriTable {
            meshes: vec![0x1000, 0x2000],
            references: vec![
                XcriReference {
                    reference: 0x000600,
                    mesh_id: 0x1000,
                },
                XcriReference {
                    reference: 0x000601,
                    mesh_id: 0x2000,
                },
            ],
        };
        let expected_bytes = encode_fo4(&expected_table).unwrap();
        assert_eq!(xcri.data.as_ref(), expected_bytes.as_slice());

        assert_ne!(
            cell.flags & RECORD_FLAG_NO_PREVIS,
            0,
            "no_previs=true keeps the flag set"
        );
        assert_eq!(
            cell.version_control, 0xAAAA_1F24,
            "upper VC bits preserved on CELL"
        );
        assert!(cell.raw_payload.is_none(), "raw_payload cleared on CELL");

        let refr1 = find_installed(&store, target, "REFR", 0x000600);
        assert_eq!(refr1.version_control, 0xBBBB_1F24);
        assert!(refr1.raw_payload.is_none());

        let refr2 = find_installed(&store, target, "REFR", 0x000601);
        assert_eq!(refr2.version_control, 0xCCCC_1F24);
        assert!(refr2.raw_payload.is_none());
    }

    /// Real-world FO4 interior CELL subrecord set (no PCMB/XCRI/VISI/RVIS/XPRI
    /// present yet — matches production WhitespringMall01 before stamping).
    /// Order and sizes mirror the field evidence: EDID(18) FULL(4) DATA(2)
    /// XCLL(136) LTMP(4) XCLW(4) XLCN(4) XCAS(4) XEZN(4) XCMO(4) XCIM(4).
    fn real_world_cell_subrecords() -> Vec<ParsedSubrecord> {
        vec![
            edid("WhitespringMall01"),
            sub("FULL", vec![0xF1; 4]),
            sub("DATA", vec![CELL_INTERIOR_FLAG, 0x00]),
            sub("XCLL", vec![0xCC; 136]),
            sub("LTMP", vec![0x11; 4]),
            sub("XCLW", vec![0x22; 4]),
            sub("XLCN", vec![0x33; 4]),
            sub("XCAS", vec![0x44; 4]),
            sub("XEZN", vec![0x55; 4]),
            sub("XCMO", vec![0x66; 4]),
            sub("XCIM", vec![0x77; 4]),
        ]
    }

    fn compressed_cell(
        form_id: u32,
        version_control: u32,
        subrecords: &[ParsedSubrecord],
    ) -> ParsedRecord {
        let raw_payload = compress_subrecords_payload(subrecords).expect("compress fixture");
        ParsedRecord {
            signature: SmolStr::new("CELL"),
            form_id,
            flags: COMPRESSED_RECORD_FLAG,
            version_control,
            form_version: Some(131),
            version2: None,
            subrecords: Vec::new(), // lazily unloaded, matching a non-eager plugin load
            raw_payload: Some(Bytes::from(raw_payload)),
            parse_error: None,
        }
    }

    #[test]
    fn stamps_compressed_cell_preserving_all_original_subrecords() {
        let target = new_target();
        let original = real_world_cell_subrecords();
        insert_parsed_record(target, compressed_cell(0x001000, 0xAAAA_0000, &original)).unwrap();
        insert_parsed_record(target, refr(0x000600, 0xBBBB_0000)).unwrap();

        let baked = BakedCell {
            cell_form_id: 0x001000,
            meshes: vec![BakedMesh {
                mesh_id: 0x1000,
                refs: vec![0x000600],
                rel_path: "meshes\\precombined\\test\\00100000_00001000_oc.nif".into(),
            }],
        };

        stamp_cell(target, &baked, 0x1F24, true).expect("stamp succeeds on a compressed cell");

        let store = plugin_handle_store_ref().lock().unwrap();
        let cell = find_installed(&store, target, "CELL", 0x001000);

        assert_ne!(
            cell.flags & COMPRESSED_RECORD_FLAG,
            0,
            "compressed flag must remain set"
        );
        assert!(
            cell.raw_payload.is_none(),
            "raw_payload cleared so the writer rebuilds a fresh compressed body"
        );
        assert_eq!(
            cell.version_control, 0xAAAA_1F24,
            "upper VC bits preserved on CELL"
        );

        // PCMB (rank 5) inserts before XCLL (rank 7, the first pre-existing
        // subrecord ranked above it); XCRI (rank 20) appends at the tail —
        // nothing pre-existing outranks it. LTMP has no rank and is inert to
        // both insertions, so it stays exactly where it was.
        let sigs: Vec<&str> = cell
            .subrecords
            .iter()
            .map(|s| s.signature.as_str())
            .collect();
        assert_eq!(
            sigs,
            vec![
                "EDID", "FULL", "DATA", "PCMB", "XCLL", "LTMP", "XCLW", "XLCN", "XCAS", "XEZN",
                "XCMO", "XCIM", "XCRI",
            ],
            "every original subrecord retained; PCMB/XCRI inserted at their ranked position"
        );

        for original_sub in &original {
            let sig = original_sub.signature.as_str();
            let found = cell
                .subrecords
                .iter()
                .find(|s| s.signature.as_str() == sig)
                .unwrap_or_else(|| panic!("{sig} must survive the stamp"));
            assert_eq!(
                found.data.as_ref(),
                original_sub.data.as_ref(),
                "{sig} must be retained byte-identically"
            );
        }

        let pcmb = cell
            .subrecords
            .iter()
            .find(|s| s.signature.as_str() == "PCMB")
            .unwrap();
        assert_eq!(pcmb.data.as_ref(), &0x1F24u16.to_le_bytes());

        let xcri = cell
            .subrecords
            .iter()
            .find(|s| s.signature.as_str() == "XCRI")
            .unwrap();
        let expected_table = XcriTable {
            meshes: vec![0x1000],
            references: vec![XcriReference {
                reference: 0x000600,
                mesh_id: 0x1000,
            }],
        };
        assert_eq!(
            xcri.data.as_ref(),
            encode_fo4(&expected_table).unwrap().as_slice()
        );

        assert_ne!(
            cell.flags & RECORD_FLAG_NO_PREVIS,
            0,
            "no_previs=true keeps the flag set"
        );

        // "Body still zlib-valid": run the exact same compress/decompress
        // primitives the (private) writer uses on `raw_payload=None` +
        // compressed, and confirm the round trip reproduces this subrecord
        // list exactly.
        let recompressed = compress_subrecords_payload(&cell.subrecords).expect("recompress");
        let redecoded = decode_compressed_subrecords_from_payload(&Bytes::from(recompressed))
            .expect("redecode");
        assert_eq!(redecoded.subrecords.len(), cell.subrecords.len());
        for (decoded_sub, original_sub) in redecoded.subrecords.iter().zip(cell.subrecords.iter()) {
            assert_eq!(
                decoded_sub.signature.as_str(),
                original_sub.signature.as_str()
            );
            assert_eq!(decoded_sub.data.as_ref(), original_sub.data.as_ref());
        }
    }

    #[test]
    fn compressed_cell_with_corrupt_body_is_rejected_and_plugin_unchanged() {
        let target = new_target();
        let garbage = Bytes::from(vec![0xFFu8; 8]); // not a valid zlib stream
        let broken_cell = ParsedRecord {
            signature: SmolStr::new("CELL"),
            form_id: 0x001000,
            flags: COMPRESSED_RECORD_FLAG,
            version_control: 0x1234_5678,
            form_version: Some(131),
            version2: None,
            subrecords: Vec::new(),
            raw_payload: Some(garbage.clone()),
            parse_error: None,
        };
        insert_parsed_record(target, broken_cell).unwrap();
        insert_parsed_record(target, refr(0x000600, 0)).unwrap();

        let baked = BakedCell {
            cell_form_id: 0x001000,
            meshes: vec![BakedMesh {
                mesh_id: 0x1000,
                refs: vec![0x000600],
                rel_path: "meshes\\precombined\\test\\00100000_00001000_oc.nif".into(),
            }],
        };
        let err = stamp_cell(target, &baked, 0x1F24, true)
            .expect_err("undecodable compressed body must fail preflight");
        assert!(
            err.contains("001000"),
            "error should name the offending cell: {err}"
        );

        let store = plugin_handle_store_ref().lock().unwrap();
        let cell = find_installed(&store, target, "CELL", 0x001000);
        assert!(
            cell.subrecords.is_empty(),
            "plugin must be untouched on decode failure"
        );
        assert_eq!(cell.raw_payload.as_deref(), Some(garbage.as_ref()));
        assert_eq!(cell.version_control, 0x1234_5678);
    }

    #[test]
    fn no_previs_false_clears_a_preexisting_no_previs_flag() {
        let target = new_target();
        insert_parsed_record(target, stale_cell(0x001000, 0)).unwrap();
        insert_parsed_record(target, refr(0x000600, 0)).unwrap();

        let baked = BakedCell {
            cell_form_id: 0x001000,
            meshes: vec![BakedMesh {
                mesh_id: 0x1000,
                refs: vec![0x000600],
                rel_path: "meshes\\precombined\\test\\00100000_00001000_oc.nif".into(),
            }],
        };

        stamp_cell(target, &baked, 0x1F24, false).expect("stamp succeeds");

        let store = plugin_handle_store_ref().lock().unwrap();
        let cell = find_installed(&store, target, "CELL", 0x001000);
        assert_eq!(
            cell.flags & RECORD_FLAG_NO_PREVIS,
            0,
            "no_previs=false clears a pre-existing No-Previs flag"
        );
    }

    #[test]
    fn missing_cell_is_rejected_and_plugin_unchanged() {
        let target = new_target();
        // A decoy CELL at a different form id — must remain untouched.
        insert_parsed_record(target, stale_cell(0x002000, 0x1234_5678)).unwrap();

        let baked = BakedCell {
            cell_form_id: 0x001000,
            meshes: vec![],
        };
        let err = stamp_cell(target, &baked, 0x1F24, true).expect_err("missing cell must fail");
        assert!(err.contains("001000"), "error should name the cell: {err}");

        let store = plugin_handle_store_ref().lock().unwrap();
        let decoy = find_installed(&store, target, "CELL", 0x002000);
        assert!(
            decoy
                .subrecords
                .iter()
                .any(|s| s.signature.as_str() == "VISI")
        );
        assert!(
            decoy
                .subrecords
                .iter()
                .any(|s| s.signature.as_str() == "XPRI")
        );
        assert!(
            !decoy
                .subrecords
                .iter()
                .any(|s| s.signature.as_str() == "XCRI")
        );
        assert_eq!(decoy.version_control, 0x1234_5678);
    }

    #[test]
    fn missing_ref_is_rejected_and_cell_unchanged() {
        let target = new_target();
        insert_parsed_record(target, stale_cell(0x001000, 0x1234_5678)).unwrap();
        // No REFR 0x000600 is ever installed.

        let baked = BakedCell {
            cell_form_id: 0x001000,
            meshes: vec![BakedMesh {
                mesh_id: 0x1000,
                refs: vec![0x000600],
                rel_path: "meshes\\precombined\\test\\00100000_00001000_oc.nif".into(),
            }],
        };
        let err = stamp_cell(target, &baked, 0x1F24, true).expect_err("missing ref must fail");
        assert!(err.contains("000600"), "error should name the ref: {err}");

        let store = plugin_handle_store_ref().lock().unwrap();
        let cell = find_installed(&store, target, "CELL", 0x001000);
        assert!(
            cell.subrecords
                .iter()
                .any(|s| s.signature.as_str() == "VISI"),
            "preflight must fail before the CELL is touched"
        );
        assert!(
            !cell
                .subrecords
                .iter()
                .any(|s| s.signature.as_str() == "XCRI")
        );
        assert_eq!(cell.version_control, 0x1234_5678);
    }

    #[test]
    fn non_own_index_ref_is_rejected_preflight_and_plugin_unchanged() {
        let target = new_target();
        insert_parsed_record(target, stale_cell(0x001000, 0x1234_5678)).unwrap();
        // A REFR whose own form id belongs to a different plugin index (e.g.
        // a master-owned/override ref) — top byte != own_index (0 here).
        insert_parsed_record(target, refr(0x01_000600, 0x1111_2222)).unwrap();

        let baked = BakedCell {
            cell_form_id: 0x001000,
            meshes: vec![BakedMesh {
                mesh_id: 0x1000,
                refs: vec![0x01_000600],
                rel_path: "meshes\\precombined\\test\\00100000_00001000_oc.nif".into(),
            }],
        };
        let err = stamp_cell(target, &baked, 0x1F24, true)
            .expect_err("non-own-index ref must fail preflight");
        assert!(
            err.contains("01000600"),
            "error should name the offending ref: {err}"
        );

        let store = plugin_handle_store_ref().lock().unwrap();
        let cell = find_installed(&store, target, "CELL", 0x001000);
        assert!(
            cell.subrecords
                .iter()
                .any(|s| s.signature.as_str() == "VISI"),
            "preflight must fail before the CELL is touched"
        );
        assert!(
            !cell
                .subrecords
                .iter()
                .any(|s| s.signature.as_str() == "XCRI")
        );
        assert_eq!(cell.version_control, 0x1234_5678);

        let untouched_refr = find_installed(&store, target, "REFR", 0x01_000600);
        assert_eq!(
            untouched_refr.version_control, 0x1111_2222,
            "the offending ref itself must remain untouched too"
        );
    }

    #[test]
    fn own_index_overflow_is_rejected() {
        let target = new_target();
        for i in 0..300u32 {
            plugin_handle_add_master_native(target, &format!("Master{i}.esm"), None)
                .expect("add master");
        }

        let baked = BakedCell {
            cell_form_id: 0x001000,
            meshes: vec![],
        };
        let err = stamp_cell(target, &baked, 0x1F24, true).expect_err("overflow must fail");
        assert!(
            err.to_lowercase().contains("overflow"),
            "error should mention overflow: {err}"
        );
    }
}
