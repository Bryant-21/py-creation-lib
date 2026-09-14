//! Streaming walk over a plugin's record headers.
//!
//! Reading one record does not require the `ParsedItem` tree: the tree costs
//! roughly 5.6x the file in committed heap (5.1 GB for SeventySix.esm) because
//! of the per-record and per-subrecord `Vec` spines. This walks the mmap-backed
//! bytes instead, materializing only what a caller asks for.

use crate::plugin_runtime::{ParsedRecord, parse_record};
use bytes::Bytes;
use pyo3::PyResult;
use std::ops::ControlFlow;

const GRUP: &[u8; 4] = b"GRUP";

/// A record's header fields plus its still-encoded payload. Borrowed from the
/// source buffer — constructing one allocates nothing, which is what lets a
/// full scan stay flat in memory.
pub(crate) struct RecordHeaderView<'a> {
    pub signature: &'a [u8; 4],
    pub form_id: u32,
    pub flags: u32,
    pub offset: usize,
    /// Raw payload. Still zlib-framed when `flags` has the COMPRESSED bit.
    pub payload: &'a [u8],
}

/// Why a scan stopped. `Truncated` keeps a malformed plugin distinguishable
/// from "no such record".
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ScanOutcome {
    Complete,
    Halted,
    Truncated { offset: usize },
}

pub(crate) struct RecordCursor<'a> {
    data: &'a Bytes,
    header_size: usize,
    root_start: usize,
}

fn u32le(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ])
}

impl<'a> RecordCursor<'a> {
    pub(crate) fn new(data: &'a Bytes, header_size: usize, root_start: usize) -> Self {
        Self {
            data,
            header_size,
            root_start,
        }
    }

    /// Byte offset of the record with `target`, or `None`. Stops at the match
    /// rather than indexing the file, so a hit early in load order costs a
    /// fraction of a full scan and allocates nothing.
    pub(crate) fn find_form_id(&self, target: u32) -> Option<usize> {
        let mut found = None;
        self.scan(&mut |view| {
            if view.form_id == target {
                found = Some(view.offset);
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        });
        found
    }

    /// Materialize the record at `offset`, decompressing if flagged.
    pub(crate) fn parse_at(&self, offset: usize) -> PyResult<ParsedRecord> {
        parse_record(self.data, offset, self.header_size, true).map(|(record, _)| record)
    }

    /// Direct children of each top-level GRUP, as `(label, count)`.
    ///
    /// Mirrors what the eager path reads off `ParsedGroup::children`: direct
    /// children only, nested GRUPs counted as one each, no descent. Counting
    /// every record instead would disagree wherever a plugin nests groups,
    /// which WRLD and CELL always do.
    pub(crate) fn top_level_groups(&self) -> Vec<([u8; 4], usize)> {
        let data: &[u8] = self.data;
        let mut groups = Vec::new();
        let mut cursor = self.root_start;
        while cursor + self.header_size <= data.len() {
            if cursor + 12 > data.len() {
                break;
            }
            let size = u32le(data, cursor + 4) as usize;
            if &data[cursor..cursor + 4] != GRUP {
                let next = cursor + self.header_size + size;
                if next <= cursor {
                    break;
                }
                cursor = next;
                continue;
            }
            let group_end = cursor + size;
            if group_end < cursor + self.header_size || group_end > data.len() {
                break;
            }
            let label: [u8; 4] = data[cursor + 8..cursor + 12].try_into().expect("4 bytes");
            groups.push((
                label,
                crate::plugin_runtime::count_children(
                    data,
                    cursor + self.header_size,
                    group_end,
                    self.header_size,
                ),
            ));
            cursor = group_end;
        }
        groups
    }

    /// Number of records at every nesting level, matching `count_records` over
    /// the tree.
    pub(crate) fn count_records(&self) -> usize {
        let mut total = 0usize;
        self.scan(&mut |_| {
            total += 1;
            ControlFlow::Continue(())
        });
        total
    }

    pub(crate) fn scan<F>(&self, visit: &mut F) -> ScanOutcome
    where
        F: FnMut(RecordHeaderView<'a>) -> ControlFlow<()>,
    {
        self.scan_within(self.data.len(), visit)
    }

    /// [`Self::scan`] bounded to `end`, for callers walking a sub-range.
    pub(crate) fn scan_within<F>(&self, end: usize, visit: &mut F) -> ScanOutcome
    where
        F: FnMut(RecordHeaderView<'a>) -> ControlFlow<()>,
    {
        self.walk(self.root_start, end.min(self.data.len()), visit)
    }

    fn walk<F>(&self, offset: usize, end: usize, visit: &mut F) -> ScanOutcome
    where
        F: FnMut(RecordHeaderView<'a>) -> ControlFlow<()>,
    {
        let data: &'a [u8] = self.data;
        let mut cursor = offset;
        while cursor + self.header_size <= end {
            if cursor + 8 > data.len() {
                return ScanOutcome::Truncated { offset: cursor };
            }
            let size = u32le(data, cursor + 4) as usize;

            if &data[cursor..cursor + 4] == GRUP {
                let group_end = cursor + size;
                // size == header_size is a legal EMPTY group; shipped ESMs
                // contain them (Fallout4.esm's TREE group). Rejecting one drops
                // every record after it.
                if group_end < cursor + self.header_size || group_end > end {
                    return ScanOutcome::Truncated { offset: cursor };
                }
                match self.walk(cursor + self.header_size, group_end, visit) {
                    ScanOutcome::Complete => {}
                    other => return other,
                }
                cursor = group_end;
                continue;
            }

            if cursor + 16 > data.len() {
                return ScanOutcome::Truncated { offset: cursor };
            }
            let body = cursor + self.header_size;
            let body_end = body + size;
            if body_end > data.len() || body_end > end {
                return ScanOutcome::Truncated { offset: cursor };
            }
            let signature: &'a [u8; 4] = data[cursor..cursor + 4].try_into().expect("4 bytes");
            let view = RecordHeaderView {
                signature,
                form_id: u32le(data, cursor + 12),
                flags: u32le(data, cursor + 8),
                offset: cursor,
                payload: &data[body..body_end],
            };
            if visit(view).is_break() {
                return ScanOutcome::Halted;
            }
            if body_end <= cursor {
                return ScanOutcome::Truncated { offset: cursor };
            }
            cursor = body_end;
        }
        ScanOutcome::Complete
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin_runtime::test_support::{group, record, subrecord};

    const MODERN_HEADER_SIZE: usize = 24;
    const TOP_GROUP: i32 = 0;

    fn fixture() -> Bytes {
        let a = record(b"MISC", 0x0100_0001, 0, &subrecord(b"EDID", b"Alpha\0"));
        let b = record(b"MISC", 0x0100_0002, 0, &subrecord(b"EDID", b"Beta\0"));
        let nested = group(*b"MISC", TOP_GROUP, &[a, b].concat());
        let c = record(b"STAT", 0x0100_0003, 0, &subrecord(b"EDID", b"Gamma\0"));
        Bytes::from([nested, group(*b"STAT", TOP_GROUP, &c)].concat())
    }

    #[test]
    fn find_form_id_locates_records_at_every_nesting_level() {
        let data = fixture();
        let cursor = RecordCursor::new(&data, MODERN_HEADER_SIZE, 0);
        assert!(cursor.find_form_id(0x0100_0001).is_some());
        assert!(cursor.find_form_id(0x0100_0003).is_some());
        assert_eq!(cursor.find_form_id(0x0100_00FF), None);
    }

    #[test]
    fn find_form_id_returns_offsets_in_file_order() {
        let data = fixture();
        let cursor = RecordCursor::new(&data, MODERN_HEADER_SIZE, 0);
        let first = cursor.find_form_id(0x0100_0001).expect("present");
        let last = cursor.find_form_id(0x0100_0003).expect("present");
        assert!(first < last, "offsets must reflect file order");
    }

    #[test]
    fn scan_visits_every_record_and_reports_completion() {
        let data = fixture();
        let cursor = RecordCursor::new(&data, MODERN_HEADER_SIZE, 0);
        let mut seen = Vec::new();
        let outcome = cursor.scan(&mut |view| {
            seen.push(view.form_id);
            ControlFlow::Continue(())
        });
        assert_eq!(seen, vec![0x0100_0001, 0x0100_0002, 0x0100_0003]);
        assert_eq!(outcome, ScanOutcome::Complete);
    }

    #[test]
    fn scan_reports_halted_when_the_visitor_breaks() {
        let data = fixture();
        let cursor = RecordCursor::new(&data, MODERN_HEADER_SIZE, 0);
        let mut seen = 0usize;
        let outcome = cursor.scan(&mut |_| {
            seen += 1;
            ControlFlow::Break(())
        });
        assert_eq!(seen, 1);
        assert_eq!(outcome, ScanOutcome::Halted);
    }

    #[test]
    fn scan_reports_truncation_rather_than_silently_stopping() {
        let full = fixture();
        let data = full.slice(0..full.len() - 8);
        let cursor = RecordCursor::new(&data, MODERN_HEADER_SIZE, 0);
        let outcome = cursor.scan(&mut |_| ControlFlow::Continue(()));
        assert!(
            matches!(outcome, ScanOutcome::Truncated { .. }),
            "a truncated plugin must be distinguishable from a clean miss, got {outcome:?}"
        );
    }

    #[test]
    fn scan_continues_past_an_empty_group() {
        // Shipped ESMs contain header-only GRUPs (Fallout4.esm's TREE group).
        // Treating one as malformed drops every record after it.
        let empty = group(*b"TREE", TOP_GROUP, &[]);
        let after = record(b"STAT", 0x0100_0009, 0, &[]);
        let data = Bytes::from([empty, group(*b"STAT", TOP_GROUP, &after)].concat());
        let cursor = RecordCursor::new(&data, MODERN_HEADER_SIZE, 0);
        let mut seen = Vec::new();
        let outcome = cursor.scan(&mut |view| {
            seen.push(view.form_id);
            ControlFlow::Continue(())
        });
        assert_eq!(seen, vec![0x0100_0009]);
        assert_eq!(outcome, ScanOutcome::Complete);
    }

    #[test]
    fn parse_at_materializes_the_record_found_by_offset() {
        let data = fixture();
        let cursor = RecordCursor::new(&data, MODERN_HEADER_SIZE, 0);
        let offset = cursor.find_form_id(0x0100_0002).expect("present");
        let record = cursor.parse_at(offset).expect("parses");
        assert_eq!(record.form_id, 0x0100_0002);
        assert_eq!(record.signature, "MISC");
    }
}
