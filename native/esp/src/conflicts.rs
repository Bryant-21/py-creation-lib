//! Cross-plugin conflict scanner.
//!
//! Walks every loaded handle's `parsed.root_items`, buckets records by
//! FormID, hashes (flags + subrecord sig+data) per record, and emits
//! one report per FormID overridden by ≥2 plugins.
//!
//! All work happens directly against the in-memory `ParsedRecord` graph —
//! no records are marshalled into Python objects.

use pyo3::prelude::*;
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::plugin_runtime::{ParsedItem, ParsedRecord, plugin_handle_store_ref};

const MERGEABLE_SIGS: &[&str] = &["LVLI", "LVLN", "LVSP", "FLST", "MUSC"];

const KWDA_BEARING_SIGS: &[&str] = &[
    "WEAP", "ARMO", "OMOD", "MISC", "ARMA", "BOOK", "FURN", "AMMO", "INGR", "ALCH", "KEYM", "LIGH",
    "ACTI", "STAT", "FLOR", "TREE", "NPC_", "RACE", "MGEF", "ENCH", "SPEL", "PROJ", "EXPL", "CONT",
    "DOOR", "IDLM",
];

const LOCAL_FORM_INDEX: u8 = 0xFF;

struct RecordEntry {
    load_order_index: i32,
    handle: u64,
    plugin_name: String,
    /// Raw on-disk form_id, relative to *this* plugin's masters list.
    form_id: u32,
    hash: u64,
    editor_id: Option<String>,
    has_kwda: bool,
    signature: String,
}

type ConflictChainPayload = (u64, String, i32, u32, u64);
type ConflictReportPayload = (
    u32,
    String,
    Option<String>,
    String,
    bool,
    Vec<ConflictChainPayload>,
);

/// Per-plugin lookup data needed to resolve a raw form_id's master byte to
/// the load-order index of the plugin that originally declares the record.
struct PluginCtx<'a> {
    handle: u64,
    plugin_name: &'a str,
    lo_index: i32,
    own_index: u8,
    masters_lower: &'a [String],
    name_to_lo: &'a HashMap<String, i32>,
}

/// Map a record's on-disk form_id to (origin_lo_index, object_id) so two
/// plugins authoring distinct records that happen to share the same raw
/// u32 form_id (e.g. each plugin's first own record at `0x01000800`) do
/// not collide. Returns `None` when the record references a master that
/// is not currently loaded — those records can't conflict with anything.
fn canonical_origin(record: &ParsedRecord, ctx: &PluginCtx<'_>) -> Option<(i32, u32)> {
    let object_id = record.form_id & 0x00FF_FFFF;
    let high = ((record.form_id >> 24) & 0xFF) as u8;
    if high == LOCAL_FORM_INDEX || high == ctx.own_index {
        return Some((ctx.lo_index, object_id));
    }
    if (high as usize) < ctx.masters_lower.len() {
        let master_name = &ctx.masters_lower[high as usize];
        return ctx.name_to_lo.get(master_name).map(|lo| (*lo, object_id));
    }
    // Unrecognised high byte — treat as self so the record still appears in
    // its plugin's own bucket and never collides with another plugin's records.
    Some((ctx.lo_index, object_id))
}

fn hash_record(record: &ParsedRecord) -> u64 {
    let mut h = DefaultHasher::new();
    record.flags.hash(&mut h);
    for sub in &record.subrecords {
        sub.signature.as_str().as_bytes().hash(&mut h);
        sub.data.as_ref().hash(&mut h);
    }
    h.finish()
}

fn record_editor_id(record: &ParsedRecord) -> Option<String> {
    for sub in &record.subrecords {
        if sub.signature.as_str() == "EDID" {
            let bytes = sub.data.as_ref();
            let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
            return Some(String::from_utf8_lossy(&bytes[..end]).into_owned());
        }
    }
    None
}

fn record_has_kwda(record: &ParsedRecord) -> bool {
    record
        .subrecords
        .iter()
        .any(|s| s.signature.as_str() == "KWDA")
}

fn walk_collect(
    items: &[ParsedItem],
    ctx: &PluginCtx<'_>,
    sig_filter: Option<&[String]>,
    out: &mut HashMap<(i32, u32), Vec<RecordEntry>>,
) {
    for item in items {
        match item {
            ParsedItem::Group(g) => walk_collect(&g.children, ctx, sig_filter, out),
            ParsedItem::Record(r) => {
                let sig = r.signature.as_str();
                if let Some(filter) = sig_filter {
                    if !filter.iter().any(|s| s.eq_ignore_ascii_case(sig)) {
                        continue;
                    }
                }
                let Some(key) = canonical_origin(r, ctx) else {
                    continue;
                };
                out.entry(key).or_default().push(RecordEntry {
                    load_order_index: ctx.lo_index,
                    handle: ctx.handle,
                    plugin_name: ctx.plugin_name.to_string(),
                    form_id: r.form_id,
                    hash: hash_record(r),
                    editor_id: record_editor_id(r),
                    has_kwda: record_has_kwda(r),
                    signature: sig.to_string(),
                });
            }
        }
    }
}

/// Scan loaded plugins for cross-plugin overrides.
///
/// `handles` is `[(handle_id, plugin_name, load_order_index), ...]` in any
/// order — the scanner sorts each chain by `load_order_index` ascending so
/// `chain[0]` is the master and `chain[-1]` is the winner.
///
/// `signatures` (optional) restricts the scan to records of those record
/// signatures (case-insensitive).
///
/// Returns `(form_id, signature, editor_id, status, mergeable, chain)` tuples,
/// where `status` is "override" or "conflict" and each `chain` entry is
/// `(plugin_handle, plugin_name, load_order_index, raw_form_id, payload_hash)`.
#[pyfunction(name = "scan_conflicts_native", signature = (handles, signatures=None))]
pub(crate) fn scan_conflicts_native(
    py: Python<'_>,
    handles: Vec<(u64, String, i32)>,
    signatures: Option<Vec<String>>,
) -> PyResult<Vec<ConflictReportPayload>> {
    let buckets: HashMap<(i32, u32), Vec<RecordEntry>> = py.detach(move || {
        let store = plugin_handle_store_ref().lock().unwrap();

        // First pass: build the load-order map and per-plugin master metadata
        // so canonical_origin() can resolve each record's master byte.
        let name_to_lo: HashMap<String, i32> = handles
            .iter()
            .map(|(_, name, lo)| (name.to_ascii_lowercase(), *lo))
            .collect();

        struct PluginMeta {
            handle: u64,
            plugin_name: String,
            lo_index: i32,
            own_index: u8,
            masters_lower: Vec<String>,
        }

        let mut metas: Vec<PluginMeta> = Vec::with_capacity(handles.len());
        for (handle_id, plugin_name, lo_index) in &handles {
            let slot = match store.get(handle_id) {
                Some(s) => s,
                None => continue,
            };
            metas.push(PluginMeta {
                handle: *handle_id,
                plugin_name: plugin_name.clone(),
                lo_index: *lo_index,
                own_index: (slot.parsed.header.masters.len() & 0xFF) as u8,
                masters_lower: slot
                    .parsed
                    .header
                    .masters
                    .iter()
                    .map(|m| m.to_ascii_lowercase())
                    .collect(),
            });
        }

        let mut buckets: HashMap<(i32, u32), Vec<RecordEntry>> = HashMap::new();
        let sig_filter = signatures.as_deref();
        for meta in &metas {
            let slot = match store.get(&meta.handle) {
                Some(s) => s,
                None => continue,
            };
            let ctx = PluginCtx {
                handle: meta.handle,
                plugin_name: &meta.plugin_name,
                lo_index: meta.lo_index,
                own_index: meta.own_index,
                masters_lower: &meta.masters_lower,
                name_to_lo: &name_to_lo,
            };
            walk_collect(&slot.parsed.root_items, &ctx, sig_filter, &mut buckets);
        }
        buckets
    });

    let mut result = Vec::new();
    for (_canonical_key, mut entries) in buckets {
        if entries.len() < 2 {
            continue;
        }
        entries.sort_by_key(|e| e.load_order_index);
        let unique_hashes: std::collections::HashSet<u64> =
            entries.iter().map(|e| e.hash).collect();
        let status = if unique_hashes.len() == 1 {
            "override"
        } else {
            "conflict"
        };
        let winner = entries.last().unwrap();
        let signature = winner.signature.clone();
        let editor_id = winner.editor_id.clone();
        let any_kwda = entries.iter().any(|e| e.has_kwda);
        // Report-level form_id is the winner's raw on-disk form_id, so
        // `session.resolve_form_id()` (which probes plugins with the raw
        // value) finds the winner directly. Per-chain entries keep their
        // own raw form_ids for plugin-scoped lookup.
        let report_form_id = winner.form_id;

        let sig_str = signature.as_str();
        let mergeable = MERGEABLE_SIGS.iter().any(|s| *s == sig_str)
            || (KWDA_BEARING_SIGS.iter().any(|s| *s == sig_str) && any_kwda);

        let chain = entries
            .iter()
            .map(|entry| {
                (
                    entry.handle,
                    entry.plugin_name.clone(),
                    entry.load_order_index,
                    entry.form_id,
                    entry.hash,
                )
            })
            .collect::<Vec<_>>();
        result.push((
            report_form_id,
            signature,
            editor_id,
            status.to_string(),
            mergeable,
            chain,
        ));
    }
    Ok(result)
}
