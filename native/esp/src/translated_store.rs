//! Rust-side store for the conversion pipeline's translated record list.
//!
//! Holds `Vec<serde_json::Value>` (records) + `Vec<Vec<String>>` (per-record
//! warnings) keyed by an opaque handle id. As Python objects, the ~371k records
//! of a full FO76→FO4 plugin dominated peak RSS (~10 GB).
//!
//! Bulk ops (rewrite_formkeys, replace_formkeys, find_stale_formkeys) run on
//! the store without a Python round trip; see `formkey_ops` for the walks.

use serde_json::Value as JsonValue;
use std::collections::{HashMap, HashSet};
use std::sync::{
    Mutex, OnceLock,
    atomic::{AtomicU64, Ordering},
};

use crate::formkey_ops;

pub(crate) struct TranslatedStore {
    pub(crate) records: Vec<JsonValue>,
    pub(crate) warnings: Vec<Vec<String>>,
}

fn stores() -> &'static Mutex<HashMap<u64, TranslatedStore>> {
    static STORES: OnceLock<Mutex<HashMap<u64, TranslatedStore>>> = OnceLock::new();
    STORES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn next_handle_id() -> u64 {
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

fn missing(handle: u64) -> String {
    format!("unknown translated_store handle: {handle}")
}

// ---------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------

pub(crate) fn create_from_json(records_json: &str, warnings_json: &str) -> Result<u64, String> {
    let records: JsonValue =
        serde_json::from_str(records_json).map_err(|e| format!("invalid records JSON: {e}"))?;
    let records: Vec<JsonValue> = match records {
        JsonValue::Array(items) => items,
        _ => return Err("records must be a JSON array".to_string()),
    };

    let warnings: JsonValue =
        serde_json::from_str(warnings_json).map_err(|e| format!("invalid warnings JSON: {e}"))?;
    let warnings: Vec<Vec<String>> = match warnings {
        JsonValue::Array(items) => items
            .into_iter()
            .map(|item| match item {
                JsonValue::Array(strings) => strings
                    .into_iter()
                    .filter_map(|s| match s {
                        JsonValue::String(s) => Some(s),
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            })
            .collect(),
        _ => return Err("warnings must be a JSON array of arrays".to_string()),
    };

    if records.len() != warnings.len() {
        return Err(format!(
            "records ({}) and warnings ({}) length mismatch",
            records.len(),
            warnings.len()
        ));
    }

    let handle = next_handle_id();
    stores()
        .lock()
        .unwrap()
        .insert(handle, TranslatedStore { records, warnings });
    Ok(handle)
}

pub(crate) fn create_empty() -> u64 {
    let handle = next_handle_id();
    stores().lock().unwrap().insert(
        handle,
        TranslatedStore {
            records: Vec::new(),
            warnings: Vec::new(),
        },
    );
    handle
}

pub(crate) fn free(handle: u64) -> Result<(), String> {
    let mut guard = stores().lock().unwrap();
    if guard.remove(&handle).is_none() {
        return Err(missing(handle));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// List-like protocol support
// ---------------------------------------------------------------------------

pub(crate) fn len(handle: u64) -> Result<usize, String> {
    let guard = stores().lock().unwrap();
    let store = guard.get(&handle).ok_or_else(|| missing(handle))?;
    Ok(store.records.len())
}

pub(crate) fn get_record_json(handle: u64, index: usize) -> Result<String, String> {
    let guard = stores().lock().unwrap();
    let store = guard.get(&handle).ok_or_else(|| missing(handle))?;
    let value = store
        .records
        .get(index)
        .ok_or_else(|| format!("index {index} out of range (len={})", store.records.len()))?;
    serde_json::to_string(value).map_err(|e| format!("failed to serialize record: {e}"))
}

pub(crate) fn get_warnings(handle: u64, index: usize) -> Result<Vec<String>, String> {
    let guard = stores().lock().unwrap();
    let store = guard.get(&handle).ok_or_else(|| missing(handle))?;
    store
        .warnings
        .get(index)
        .cloned()
        .ok_or_else(|| format!("index {index} out of range (len={})", store.warnings.len()))
}

pub(crate) fn set_record_json(handle: u64, index: usize, json: &str) -> Result<(), String> {
    let value: JsonValue =
        serde_json::from_str(json).map_err(|e| format!("invalid record JSON: {e}"))?;
    let mut guard = stores().lock().unwrap();
    let store = guard.get_mut(&handle).ok_or_else(|| missing(handle))?;
    if index >= store.records.len() {
        return Err(format!(
            "index {index} out of range (len={})",
            store.records.len()
        ));
    }
    store.records[index] = value;
    Ok(())
}

pub(crate) fn set_warnings(handle: u64, index: usize, warnings: Vec<String>) -> Result<(), String> {
    let mut guard = stores().lock().unwrap();
    let store = guard.get_mut(&handle).ok_or_else(|| missing(handle))?;
    if index >= store.warnings.len() {
        return Err(format!(
            "index {index} out of range (len={})",
            store.warnings.len()
        ));
    }
    store.warnings[index] = warnings;
    Ok(())
}

pub(crate) fn set_pair_json(
    handle: u64,
    index: usize,
    record_json: &str,
    warnings: Vec<String>,
) -> Result<(), String> {
    let value: JsonValue =
        serde_json::from_str(record_json).map_err(|e| format!("invalid record JSON: {e}"))?;
    let mut guard = stores().lock().unwrap();
    let store = guard.get_mut(&handle).ok_or_else(|| missing(handle))?;
    if index >= store.records.len() {
        return Err(format!(
            "index {index} out of range (len={})",
            store.records.len()
        ));
    }
    store.records[index] = value;
    store.warnings[index] = warnings;
    Ok(())
}

pub(crate) fn append_json(
    handle: u64,
    record_json: &str,
    warnings: Vec<String>,
) -> Result<(), String> {
    let value: JsonValue =
        serde_json::from_str(record_json).map_err(|e| format!("invalid record JSON: {e}"))?;
    let mut guard = stores().lock().unwrap();
    let store = guard.get_mut(&handle).ok_or_else(|| missing(handle))?;
    store.records.push(value);
    store.warnings.push(warnings);
    Ok(())
}

/// Pull a contiguous slice as a JSON array of `[record, warnings]` pairs —
/// efficient streaming `__iter__` for the Python view wrapper.
pub(crate) fn fetch_chunk_json(handle: u64, start: usize, end: usize) -> Result<String, String> {
    let guard = stores().lock().unwrap();
    let store = guard.get(&handle).ok_or_else(|| missing(handle))?;
    let total = store.records.len();
    let end = end.min(total);
    if start > end {
        return Err(format!("invalid chunk range: {start}..{end}"));
    }
    let mut out = Vec::with_capacity(end - start);
    for i in start..end {
        let pair = JsonValue::Array(vec![
            store.records[i].clone(),
            JsonValue::Array(
                store.warnings[i]
                    .iter()
                    .map(|s| JsonValue::String(s.clone()))
                    .collect(),
            ),
        ]);
        out.push(pair);
    }
    serde_json::to_string(&JsonValue::Array(out))
        .map_err(|e| format!("failed to serialize chunk: {e}"))
}

/// Remove records at the given indices in descending order. Mirrors the
/// descending-index prune pattern used by `_prune_orphaned_records`.
pub(crate) fn prune_indices(handle: u64, mut indices: Vec<usize>) -> Result<usize, String> {
    indices.sort_unstable();
    indices.dedup();
    let mut guard = stores().lock().unwrap();
    let store = guard.get_mut(&handle).ok_or_else(|| missing(handle))?;
    let len = store.records.len();
    if let Some(&max) = indices.last() {
        if max >= len {
            return Err(format!("index {max} out of range (len={len})"));
        }
    }
    // Remove in reverse to preserve indices.
    for &i in indices.iter().rev() {
        store.records.remove(i);
        store.warnings.remove(i);
    }
    Ok(indices.len())
}

// ---------------------------------------------------------------------------
// Bulk operations on the store (no Python materialization)
// ---------------------------------------------------------------------------

pub(crate) fn rewrite_formkeys_inplace(handle: u64, mappings_json: &str) -> Result<usize, String> {
    let mappings: JsonValue =
        serde_json::from_str(mappings_json).map_err(|e| format!("invalid mappings JSON: {e}"))?;
    let lookup = formkey_ops::build_rewrite_lookup_pub(&mappings);
    if lookup.is_empty() {
        return Ok(0);
    }
    let mut guard = stores().lock().unwrap();
    let store = guard.get_mut(&handle).ok_or_else(|| missing(handle))?;
    let n = store.records.len();
    // Take ownership, walk, write back. take/replace dance avoids cloning.
    let records = std::mem::take(&mut store.records);
    let mut out = Vec::with_capacity(records.len());
    for record in records {
        out.push(formkey_ops::rewrite_formkeys_walk_pub(record, &lookup));
    }
    store.records = out;
    Ok(n)
}

pub(crate) fn replace_formkeys_inplace(
    handle: u64,
    replacements_json: &str,
) -> Result<usize, String> {
    let replacements: JsonValue = serde_json::from_str(replacements_json)
        .map_err(|e| format!("invalid replacements JSON: {e}"))?;
    let lookup = formkey_ops::build_replace_lookup_pub(&replacements)?;
    if lookup.is_empty() {
        return Ok(0);
    }
    let mut guard = stores().lock().unwrap();
    let store = guard.get_mut(&handle).ok_or_else(|| missing(handle))?;
    let n = store.records.len();
    let records = std::mem::take(&mut store.records);
    let warnings = std::mem::take(&mut store.warnings);
    let mut out_records = Vec::with_capacity(records.len());
    let mut out_warnings = Vec::with_capacity(warnings.len());
    // _replace_formkeys can return _REMOVE for the top-level value; in that
    // case we drop the whole record (matches the array-level behavior in
    // formkey_ops::replace_formkeys_batch_json).
    for (record, w) in records.into_iter().zip(warnings.into_iter()) {
        if let Some(v) = formkey_ops::replace_formkeys_walk_pub(record, &lookup) {
            out_records.push(v);
            out_warnings.push(w);
        }
    }
    store.records = out_records;
    store.warnings = out_warnings;
    Ok(n)
}

pub(crate) fn find_stale_formkeys(
    handle: u64,
    source_plugins_json: &str,
) -> Result<String, String> {
    let plugins: JsonValue = serde_json::from_str(source_plugins_json)
        .map_err(|e| format!("invalid source_plugins JSON: {e}"))?;
    let plugins: HashSet<String> = match plugins {
        JsonValue::Array(items) => items
            .into_iter()
            .filter_map(|v| match v {
                JsonValue::String(s) => Some(s.to_ascii_lowercase()),
                _ => None,
            })
            .collect(),
        _ => return Err("source_plugins must be a JSON array of strings".to_string()),
    };

    let guard = stores().lock().unwrap();
    let store = guard.get(&handle).ok_or_else(|| missing(handle))?;
    let mut found: HashSet<String> = HashSet::new();
    for record in store.records.iter() {
        formkey_ops::find_stale_walk_pub(record, &plugins, &mut found);
    }
    let mut as_vec: Vec<String> = found.into_iter().collect();
    as_vec.sort();
    serde_json::to_string(&as_vec).map_err(|e| format!("failed to serialize found set: {e}"))
}

/// Return indices whose top-level `signature` field matches one of the given
/// signatures. Used by Python fixups to find "all COBJs", "all NPCs", etc.
/// without materializing every record.
pub(crate) fn indices_by_signature(
    handle: u64,
    signatures_json: &str,
) -> Result<Vec<usize>, String> {
    let sigs: JsonValue = serde_json::from_str(signatures_json)
        .map_err(|e| format!("invalid signatures JSON: {e}"))?;
    let sigs: HashSet<String> = match sigs {
        JsonValue::Array(items) => items
            .into_iter()
            .filter_map(|v| match v {
                JsonValue::String(s) => Some(s),
                _ => None,
            })
            .collect(),
        _ => return Err("signatures must be a JSON array of strings".to_string()),
    };
    let guard = stores().lock().unwrap();
    let store = guard.get(&handle).ok_or_else(|| missing(handle))?;
    let mut out = Vec::new();
    for (i, record) in store.records.iter().enumerate() {
        let record_sig = record
            .get("signature")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if sigs.contains(record_sig) {
            out.push(i);
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Final extraction (for phase_build_esp streaming)
// ---------------------------------------------------------------------------

/// Drain a single record by index into a JSON string. Caller is responsible
/// for using sequential or reverse-iter access; we do NOT compact the vec
/// because the index space matters to the orchestrator.
pub(crate) fn take_record_json(handle: u64, index: usize) -> Result<String, String> {
    let mut guard = stores().lock().unwrap();
    let store = guard.get_mut(&handle).ok_or_else(|| missing(handle))?;
    if index >= store.records.len() {
        return Err(format!(
            "index {index} out of range (len={})",
            store.records.len()
        ));
    }
    let value = std::mem::replace(&mut store.records[index], JsonValue::Null);
    serde_json::to_string(&value).map_err(|e| format!("failed to serialize record: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fresh_store(records: JsonValue, warnings: JsonValue) -> u64 {
        create_from_json(&records.to_string(), &warnings.to_string()).unwrap()
    }

    #[test]
    fn round_trip_create_get_set() {
        let records = json!([{"a": 1}, {"b": "591667:SeventySix.esm"}]);
        let warnings = json!([["w1"], []]);
        let h = fresh_store(records, warnings);
        assert_eq!(len(h).unwrap(), 2);
        assert_eq!(get_warnings(h, 0).unwrap(), vec!["w1".to_string()]);
        let r0 = get_record_json(h, 0).unwrap();
        assert_eq!(r0, r#"{"a":1}"#);
        set_record_json(h, 0, r#"{"a":42}"#).unwrap();
        assert_eq!(get_record_json(h, 0).unwrap(), r#"{"a":42}"#);
        free(h).unwrap();
        assert!(len(h).is_err());
    }

    #[test]
    fn rewrite_in_store_mirrors_batch() {
        let records = json!([{"k": "591667:SeventySix.esm"}]);
        let warnings = json!([[]]);
        let h = fresh_store(records, warnings);
        let mappings = json!({"591667:SeventySix.esm": {"new_formkey": "013F42:Fallout4.esm"}});
        rewrite_formkeys_inplace(h, &mappings.to_string()).unwrap();
        let r0 = get_record_json(h, 0).unwrap();
        assert!(r0.contains("013F42:Fallout4.esm"));
        free(h).unwrap();
    }

    #[test]
    fn replace_in_store_drops_records_when_root_is_null_replaced() {
        // A record that is a canonical-ref dict at top level + a regular record.
        let records = json!([
            {"reference": {"plugin": "SeventySix.esm", "object_id": "591667"}},
            {"k": "keep"}
        ]);
        let warnings = json!([[], []]);
        let h = fresh_store(records, warnings);
        let replacements = json!({"591667:SeventySix.esm": null});
        replace_formkeys_inplace(h, &replacements.to_string()).unwrap();
        assert_eq!(len(h).unwrap(), 1);
        let r0 = get_record_json(h, 0).unwrap();
        assert!(r0.contains("keep"));
        free(h).unwrap();
    }

    #[test]
    fn find_stale_in_store() {
        let records = json!([
            {"k": "591667:SeventySix.esm"},
            {"k": "AAAAAA:Other.esm"}
        ]);
        let warnings = json!([[], []]);
        let h = fresh_store(records, warnings);
        let plugins = json!(["seventysix.esm"]);
        let out = find_stale_formkeys(h, &plugins.to_string()).unwrap();
        let parsed: Vec<String> = serde_json::from_str(&out).unwrap();
        assert_eq!(parsed, vec!["591667:SeventySix.esm".to_string()]);
        free(h).unwrap();
    }

    #[test]
    fn indices_by_signature_filters() {
        let records = json!([
            {"signature": "COBJ", "k": 1},
            {"signature": "WEAP", "k": 2},
            {"signature": "COBJ", "k": 3}
        ]);
        let warnings = json!([[], [], []]);
        let h = fresh_store(records, warnings);
        let sigs = json!(["COBJ"]);
        assert_eq!(
            indices_by_signature(h, &sigs.to_string()).unwrap(),
            vec![0usize, 2]
        );
        free(h).unwrap();
    }

    #[test]
    fn prune_indices_removes_in_reverse() {
        let records = json!([{"a": 1}, {"a": 2}, {"a": 3}, {"a": 4}]);
        let warnings = json!([[], [], [], []]);
        let h = fresh_store(records, warnings);
        prune_indices(h, vec![1, 3]).unwrap();
        assert_eq!(len(h).unwrap(), 2);
        assert_eq!(get_record_json(h, 0).unwrap(), r#"{"a":1}"#);
        assert_eq!(get_record_json(h, 1).unwrap(), r#"{"a":3}"#);
        free(h).unwrap();
    }

    #[test]
    fn fetch_chunk_returns_pairs() {
        let records = json!([{"a": 1}, {"a": 2}, {"a": 3}]);
        let warnings = json!([["w1"], [], ["w3a", "w3b"]]);
        let h = fresh_store(records, warnings);
        let chunk = fetch_chunk_json(h, 0, 2).unwrap();
        let parsed: JsonValue = serde_json::from_str(&chunk).unwrap();
        assert_eq!(parsed[0][0]["a"], 1);
        assert_eq!(parsed[0][1][0], "w1");
        assert_eq!(parsed[1][0]["a"], 2);
        assert_eq!(parsed[1][1].as_array().unwrap().len(), 0);
        free(h).unwrap();
    }

    #[test]
    fn append_grows_store() {
        let h = create_empty();
        append_json(h, r#"{"a":1}"#, vec!["w".to_string()]).unwrap();
        append_json(h, r#"{"b":2}"#, vec![]).unwrap();
        assert_eq!(len(h).unwrap(), 2);
        free(h).unwrap();
    }
}
