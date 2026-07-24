//! Shared FO4/FO76 CELL `XCRI` (combined-reference index) codec.
//!
//! FO4's `reference_count` header field counts u32 *words*, i.e. it is 2x the
//! logical reference-row count (proof: vanilla `Fallout4.esm` CELL
//! `0x00000FC9` has XCRI length `3224 = 8 + 38*4 + 383*8` with
//! `reference_count=766`, so `766/2=383` logical rows). FO76's
//! `reference_count` field is likewise 2x the logical row count, but its
//! reference rows are 16 bytes wide, carrying two extra unknown u32s (proof:
//! `SeventySix.esm` CELL `0x0062781C` has XCRI length
//! `79144 = 16 + 409*8 + 4741*16` with `reference_count=9482`, so
//! `9482/2=4741` logical rows).

/// One decoded XCRI reference row, format-agnostic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct XcriReference {
    pub reference: u32,
    pub mesh_id: u32,
}

/// A decoded XCRI table: mesh ids in file order plus reference rows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct XcriTable {
    pub meshes: Vec<u32>,
    pub references: Vec<XcriReference>,
}

fn read_u32(data: &[u8], offset: usize) -> Option<u32> {
    data.get(offset..offset + 4)
        .map(|slice| u32::from_le_bytes(slice.try_into().unwrap()))
}

fn read_u64(data: &[u8], offset: usize) -> Option<u64> {
    data.get(offset..offset + 8)
        .map(|slice| u64::from_le_bytes(slice.try_into().unwrap()))
}

/// Decode an FO4-format XCRI payload: `u32 mesh_count`, `u32 reference_count`
/// (2x the logical row count), `mesh_count` 4-byte mesh-id rows, then
/// `reference_count/2` 8-byte `{reference, mesh_id}` rows.
///
/// Returns `None` if the header is missing, `reference_count` is odd, the
/// declared layout overflows `usize`, or the payload length does not exactly
/// match the declared layout.
pub fn decode_fo4(data: &[u8]) -> Option<XcriTable> {
    if data.len() < 8 {
        return None;
    }
    let mesh_count = read_u32(data, 0)? as usize;
    let reference_count_field = read_u32(data, 4)? as usize;
    if reference_count_field % 2 != 0 {
        return None;
    }
    let row_count = reference_count_field / 2;

    let mesh_start = 8usize;
    let mesh_end = mesh_start.checked_add(mesh_count.checked_mul(4)?)?;
    let reference_end = mesh_end.checked_add(row_count.checked_mul(8)?)?;
    if reference_end != data.len() {
        return None;
    }

    let meshes = (0..mesh_count)
        .map(|index| read_u32(data, mesh_start + index * 4))
        .collect::<Option<Vec<_>>>()?;
    let references = (0..row_count)
        .map(|index| {
            let offset = mesh_end + index * 8;
            Some(XcriReference {
                reference: read_u32(data, offset)?,
                mesh_id: read_u32(data, offset + 4)?,
            })
        })
        .collect::<Option<Vec<_>>>()?;

    Some(XcriTable { meshes, references })
}

/// Computes the FO4 header fields for a table of `mesh_len` meshes and
/// `row_len` logical reference rows, or `None` on overflow. Split out from
/// [`encode_fo4`] so the overflow guard is testable without materializing
/// billions of `XcriReference` rows.
fn encode_counts(mesh_len: usize, row_len: usize) -> Option<(u32, u32)> {
    let mesh_count = u32::try_from(mesh_len).ok()?;
    let row_count = u32::try_from(row_len).ok()?;
    let reference_count = row_count.checked_mul(2)?;
    Some((mesh_count, reference_count))
}

/// Encode an FO4-format XCRI payload. Returns `None` if `table.meshes.len()`
/// or `table.references.len() * 2` overflows `u32`, rather than truncating
/// `usize` to `u32`.
pub fn encode_fo4(table: &XcriTable) -> Option<Vec<u8>> {
    let (mesh_count, reference_count) = encode_counts(table.meshes.len(), table.references.len())?;

    let mut out = Vec::with_capacity(8 + table.meshes.len() * 4 + table.references.len() * 8);
    out.extend_from_slice(&mesh_count.to_le_bytes());
    out.extend_from_slice(&reference_count.to_le_bytes());
    for mesh_id in &table.meshes {
        out.extend_from_slice(&mesh_id.to_le_bytes());
    }
    for reference in &table.references {
        out.extend_from_slice(&reference.reference.to_le_bytes());
        out.extend_from_slice(&reference.mesh_id.to_le_bytes());
    }
    Some(out)
}

/// Decode an FO76-format XCRI payload: `u64 mesh_count`, `u64 reference_count`
/// (2x the logical row count), `mesh_count` 8-byte `{mesh_id, count}` mesh
/// rows, then `reference_count/2` 16-byte `{reference, unknown, mesh_id,
/// unknown}` reference rows. The mesh row's `count` and the reference row's
/// two unknown u32s are discarded; only bytes `0..4` and `8..12` of each
/// reference row are kept.
pub fn decode_fo76(data: &[u8]) -> Option<XcriTable> {
    if data.len() < 16 {
        return None;
    }
    let mesh_count = usize::try_from(read_u64(data, 0)?).ok()?;
    let reference_count_field = usize::try_from(read_u64(data, 8)?).ok()?;
    if reference_count_field % 2 != 0 {
        return None;
    }
    let row_count = reference_count_field / 2;

    let mesh_start = 16usize;
    let mesh_end = mesh_start.checked_add(mesh_count.checked_mul(8)?)?;
    let reference_end = mesh_end.checked_add(row_count.checked_mul(16)?)?;
    if reference_end != data.len() {
        return None;
    }

    let meshes = (0..mesh_count)
        .map(|index| read_u32(data, mesh_start + index * 8))
        .collect::<Option<Vec<_>>>()?;
    let references = (0..row_count)
        .map(|index| {
            let offset = mesh_end + index * 16;
            Some(XcriReference {
                reference: read_u32(data, offset)?,
                mesh_id: read_u32(data, offset + 8)?,
            })
        })
        .collect::<Option<Vec<_>>>()?;

    Some(XcriTable { meshes, references })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn le(value: u32) -> [u8; 4] {
        value.to_le_bytes()
    }

    #[test]
    fn decode_fo4_reads_two_meshes_three_references() {
        let mut data = Vec::new();
        data.extend_from_slice(&le(2)); // mesh_count
        data.extend_from_slice(&le(6)); // reference_count field = 2x3 rows
        data.extend_from_slice(&le(0x1000)); // mesh row 0
        data.extend_from_slice(&le(0x2000)); // mesh row 1
        data.extend_from_slice(&le(0x01_000801)); // ref 0: reference
        data.extend_from_slice(&le(0x1000)); //         mesh_id
        data.extend_from_slice(&le(0x01_000802)); // ref 1: reference
        data.extend_from_slice(&le(0x1000)); //         mesh_id
        data.extend_from_slice(&le(0x01_000803)); // ref 2: reference
        data.extend_from_slice(&le(0x2000)); //         mesh_id

        let table = decode_fo4(&data).expect("valid FO4 XCRI decodes");
        assert_eq!(table.meshes, vec![0x1000, 0x2000]);
        assert_eq!(
            table.references,
            vec![
                XcriReference {
                    reference: 0x01_000801,
                    mesh_id: 0x1000
                },
                XcriReference {
                    reference: 0x01_000802,
                    mesh_id: 0x1000
                },
                XcriReference {
                    reference: 0x01_000803,
                    mesh_id: 0x2000
                },
            ]
        );
    }

    #[test]
    fn encode_fo4_round_trips_decode_fo4() {
        let table = XcriTable {
            meshes: vec![0x1000, 0x2000],
            references: vec![
                XcriReference {
                    reference: 0x01_000801,
                    mesh_id: 0x1000,
                },
                XcriReference {
                    reference: 0x01_000802,
                    mesh_id: 0x1000,
                },
                XcriReference {
                    reference: 0x01_000803,
                    mesh_id: 0x2000,
                },
            ],
        };

        let encoded = encode_fo4(&table).expect("table encodes");
        assert_eq!(encoded.len(), 8 + 2 * 4 + 3 * 8);
        assert_eq!(&encoded[0..4], &le(2)); // mesh_count
        assert_eq!(&encoded[4..8], &le(6)); // reference_count field = 2x3

        let decoded = decode_fo4(&encoded).expect("re-decodes");
        assert_eq!(decoded, table);
    }

    #[test]
    fn decode_fo4_rejects_odd_reference_count() {
        let mut data = Vec::new();
        data.extend_from_slice(&le(0)); // mesh_count
        data.extend_from_slice(&le(1)); // odd reference_count field: invalid
        assert!(decode_fo4(&data).is_none());
    }

    #[test]
    fn decode_fo4_rejects_truncated_tail() {
        let mut data = Vec::new();
        data.extend_from_slice(&le(1)); // mesh_count = 1
        data.extend_from_slice(&le(2)); // reference_count field = 2x1 row
        data.extend_from_slice(&le(0x1000)); // mesh row present
        // The one reference row is entirely missing.
        assert!(decode_fo4(&data).is_none());
    }

    #[test]
    fn encode_fo4_rejects_mesh_count_overflow() {
        assert!(encode_counts(usize::MAX, 0).is_none());
    }

    #[test]
    fn encode_fo4_rejects_reference_count_overflow() {
        // row_count fits in u32 but row_count * 2 does not.
        let row_len = u32::MAX as usize / 2 + 1;
        assert!(encode_counts(0, row_len).is_none());
    }

    #[test]
    fn decode_fo76_keeps_reference_and_mesh_id_bytes_only() {
        let mut data = Vec::new();
        data.extend_from_slice(&1u64.to_le_bytes()); // mesh_count = 1 row
        data.extend_from_slice(&4u64.to_le_bytes()); // reference_count field = 2x2 rows
        // mesh row: {mesh_id, count} — count is discarded.
        data.extend_from_slice(&le(0xAABB_CCDD));
        data.extend_from_slice(&le(0x1122_3344));
        // reference row 0: {reference, unknown, mesh_id, unknown}
        data.extend_from_slice(&le(0x0100_0801));
        data.extend_from_slice(&le(0xDEAD_BEEF));
        data.extend_from_slice(&le(0xAABB_CCDD));
        data.extend_from_slice(&le(0xCAFE_BABE));
        // reference row 1
        data.extend_from_slice(&le(0x0100_0802));
        data.extend_from_slice(&le(0x1111_1111));
        data.extend_from_slice(&le(0xAABB_CCDD));
        data.extend_from_slice(&le(0x2222_2222));

        let table = decode_fo76(&data).expect("valid FO76 XCRI decodes");
        assert_eq!(table.meshes, vec![0xAABB_CCDD]);
        assert_eq!(
            table.references,
            vec![
                XcriReference {
                    reference: 0x0100_0801,
                    mesh_id: 0xAABB_CCDD
                },
                XcriReference {
                    reference: 0x0100_0802,
                    mesh_id: 0xAABB_CCDD
                },
            ]
        );
    }

    #[test]
    fn decode_fo76_rejects_truncated_reference_row() {
        let mut data = Vec::new();
        data.extend_from_slice(&0u64.to_le_bytes()); // mesh_count = 0
        data.extend_from_slice(&2u64.to_le_bytes()); // reference_count field = 2x1 row
        data.extend_from_slice(&le(0x0100_0801));
        data.extend_from_slice(&le(0xDEAD_BEEF));
        data.extend_from_slice(&le(0xAABB_CCDD));
        // Final unknown u32 of the 16-byte row is missing.
        assert!(decode_fo76(&data).is_none());
    }

    #[test]
    fn fo4_ground_truth_size_equation_38_meshes_383_references() {
        let table = XcriTable {
            meshes: (0..38u32).collect(),
            references: (0..383u32)
                .map(|i| XcriReference {
                    reference: 0x01_000000 + i,
                    mesh_id: i % 38,
                })
                .collect(),
        };

        let encoded = encode_fo4(&table).expect("table encodes");
        assert_eq!(encoded.len(), 3224);
        assert_eq!(&encoded[4..8], &le(766));

        let decoded = decode_fo4(&encoded).expect("decodes back");
        assert_eq!(decoded.meshes.len(), 38);
        assert_eq!(decoded.references.len(), 383);
    }

    #[test]
    fn fo76_ground_truth_size_equation_409_meshes_4741_references() {
        let mesh_count = 409u64;
        let logical_rows = 4741u32;
        let reference_count = u64::from(logical_rows) * 2;

        let mut data = Vec::new();
        data.extend_from_slice(&mesh_count.to_le_bytes());
        data.extend_from_slice(&reference_count.to_le_bytes());
        for i in 0..mesh_count as u32 {
            data.extend_from_slice(&le(i));
            data.extend_from_slice(&le(0));
        }
        for i in 0..logical_rows {
            data.extend_from_slice(&le(0x01_000000 + i));
            data.extend_from_slice(&le(0));
            data.extend_from_slice(&le(i % mesh_count as u32));
            data.extend_from_slice(&le(0));
        }
        assert_eq!(data.len(), 79144);

        let table = decode_fo76(&data).expect("decodes real-scale FO76 XCRI");
        assert_eq!(table.meshes.len(), 409);
        assert_eq!(table.references.len(), 4741);
    }
}
