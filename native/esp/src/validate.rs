//! Native error checker for the editor session.
//!
//! Validates a single plugin handle against the rest of the load order.
//! Categories mirror the Python validator under
//! `py_creation_lib/python/creation_lib/esp/editor/validate.py`:
//!   - missing_master, broken_reference, parse_error, itm, udr
//!
//! All record walking, form-id resolution, and master-byte byte-equality
//! checks happen against lazy per-handle index sections — no Python objects
//! are materialized in the hot path.

use pyo3::exceptions::PyKeyError;
use pyo3::prelude::*;
use std::collections::HashMap;
use std::sync::Arc;

use crate::plugin_runtime::{
    CoreSection, ParsedItem, ParsedPlugin, ParsedRecord, ParsedSubrecord, RefsSection,
    build_refs_section, compiled_schema_for_game, ensure_core_section, ensure_records_section,
    form_key_refs, is_known_formid_array_subrecord, is_known_formid_subrecord,
    lazy_materialize_record, plugin_handle_store_ref, resolve_form_id_to_form_key,
};

const RECORD_FLAG_DELETED: u32 = 0x0000_0020;
const HEADER_FLAG_MASTER: u32 = 0x0000_0001;
const LOCAL_FORM_INDEX: u8 = 0xFF;

struct PluginMeta {
    name: String,
    own_index: u8,
    masters: Vec<String>,
    /// Lowercased master plugin names in load order.
    masters_lower: Vec<String>,
    is_master: bool,
}

struct Issue {
    severity: &'static str,
    category: &'static str,
    plugin_handle: u64,
    plugin_name: String,
    message: String,
    form_id: Option<u32>,
    path: Option<String>,
    signature: Option<String>,
}

type IssuePayload = (
    String,
    String,
    u64,
    String,
    String,
    Option<u32>,
    Option<String>,
    Option<String>,
);

/// Pull every raw FormID referenced by a record's subrecords, keeping the high
/// byte intact so the caller can route refs to the right master plugin.
fn iter_raw_referenced_form_ids(record: &ParsedRecord) -> Vec<u32> {
    let mut out = Vec::new();
    for sub in &record.subrecords {
        if sub.semantic_type.as_deref() == Some("formid") && sub.data.len() >= 4 {
            out.push(read_le_u32(&sub.data));
            continue;
        }
        if sub.semantic_type.as_deref() == Some("formid_array")
            && !sub.data.is_empty()
            && sub.data.len() % 4 == 0
        {
            for chunk in sub.data.chunks_exact(4) {
                out.push(u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
            }
            continue;
        }
        if sub.data.len() == 4 && is_known_formid_subrecord(sub.signature.as_str()) {
            out.push(read_le_u32(&sub.data));
            continue;
        }
        if !sub.data.is_empty()
            && sub.data.len() % 4 == 0
            && is_known_formid_array_subrecord(sub.signature.as_str())
        {
            for chunk in sub.data.chunks_exact(4) {
                out.push(u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
            }
        }
    }
    out
}

fn read_le_u32(data: &[u8]) -> u32 {
    u32::from_le_bytes([data[0], data[1], data[2], data[3]])
}

/// Try to resolve a raw form_id (from the active plugin's perspective) to
/// any plugin's core form-id tables. Returns true if found.
fn resolve_form_id(
    raw_fid: u32,
    active_meta: &PluginMeta,
    metas: &HashMap<u64, PluginMeta>,
    cores: &HashMap<u64, Arc<CoreSection>>,
    name_to_handle: &HashMap<String, u64>,
    active_handle: u64,
) -> bool {
    if raw_fid == 0 {
        return true;
    }
    let object_id = raw_fid & 0x00FF_FFFF;
    if object_id == 0 {
        return true;
    }
    let high = ((raw_fid >> 24) & 0xFF) as u8;
    // Active or local-marker — look in active's own index.
    if high == LOCAL_FORM_INDEX || high == active_meta.own_index {
        if let Some(core) = cores.get(&active_handle) {
            let probe = ((active_meta.own_index as u32) << 24) | object_id;
            if core_contains_form_id(core, probe) {
                return true;
            }
            // Also accept the raw key in case it was stored differently.
            if core_contains_form_id(core, raw_fid) {
                return true;
            }
        }
        return false;
    }
    // Master reference — find the master plugin and probe with master's own_index.
    if (high as usize) < active_meta.masters_lower.len() {
        let master_name = &active_meta.masters_lower[high as usize];
        if let Some(other_handle) = name_to_handle.get(master_name) {
            let other_meta = match metas.get(other_handle) {
                Some(m) => m,
                None => return false,
            };
            let other_core = match cores.get(other_handle) {
                Some(i) => i,
                None => return false,
            };
            let probe = ((other_meta.own_index as u32) << 24) | object_id;
            if core_contains_form_id(other_core, probe) {
                return true;
            }
            if core_contains_form_id(other_core, raw_fid) {
                return true;
            }
        }
        return false;
    }
    false
}

/// Build a Class C issue. Message mirrors xEdit `TwbFormIDChecked.Check`:
/// `Found a {found} reference, expected: {expected}` where `expected` is
/// `RefTargetSpec::expected_label()` — the shared accessor's CommaText
/// (declared sigs joined with ',', ",NULL" appended when null_allowed). Using
/// it keeps our wording byte-identical to conv-refs' Pass-2 diagnostics and to
/// xEdit's `fidcValidRefs.CommaText`.
fn make_ref_type_issue(
    handle_id: u64,
    plugin_name: &str,
    record: &ParsedRecord,
    sub_sig: &str,
    found_sig: &str,
    expected: &str,
) -> Issue {
    Issue {
        severity: "warning",
        category: "ref_type_mismatch",
        plugin_handle: handle_id,
        plugin_name: plugin_name.to_string(),
        message: format!(
            "{} \\ {} -> Found a {} reference, expected: {}",
            record.signature.as_str(),
            sub_sig,
            found_sig,
            expected
        ),
        form_id: Some(record.form_id),
        path: None,
        signature: Some(record.signature.to_string()),
    }
}

fn core_contains_form_id(core: &CoreSection, form_id: u32) -> bool {
    let object_id = form_id & 0x00FF_FFFF;
    core.form_ids_by_object_id
        .get(&object_id)
        .is_some_and(|form_ids| form_ids.contains(&form_id))
}

/// Class C support: resolve a raw form_id to the *signature* of the record it
/// points at, routing through masters exactly like `resolve_form_id`. Returns
/// None when unresolved (that is the Class B / broken_reference case, already
/// emitted by the resolve loop). The signature is read from the owning plugin's
/// materialized records, probing with that plugin's own_index.
fn resolve_form_id_target_sig(
    raw_fid: u32,
    active_meta: &PluginMeta,
    metas: &HashMap<u64, PluginMeta>,
    cores: &HashMap<u64, Arc<CoreSection>>,
    signatures_by_handle: &HashMap<u64, HashMap<u32, String>>,
    name_to_handle: &HashMap<String, u64>,
    active_handle: u64,
) -> Option<String> {
    let object_id = raw_fid & 0x00FF_FFFF;
    if object_id == 0 {
        return None;
    }
    let high = ((raw_fid >> 24) & 0xFF) as u8;
    let (owner_handle, owner_index) = if high == LOCAL_FORM_INDEX || high == active_meta.own_index {
        (active_handle, active_meta.own_index)
    } else if (high as usize) < active_meta.masters_lower.len() {
        let master_name = &active_meta.masters_lower[high as usize];
        let owner = *name_to_handle.get(master_name)?;
        (owner, metas.get(&owner)?.own_index)
    } else {
        return None;
    };
    let core = cores.get(&owner_handle)?;
    let probe = ((owner_index as u32) << 24) | object_id;
    if !core_contains_form_id(core, probe) && !core_contains_form_id(core, raw_fid) {
        return None;
    }
    signatures_by_handle
        .get(&owner_handle)?
        .get(&object_id)
        .cloned()
}

/// Class C support: yield `(subrecord_sig, raw_fid)` for each subrecord-level
/// reference field, so the caller can look up `allowed_targets(rec_sig,
/// sub_sig)`. Mirrors the subrecord-level cases of
/// `iter_raw_referenced_form_ids` (the struct-internal FK fields it does not
/// decode are out of scope here — see the C parity note).
fn iter_subrecord_level_refs(record: &ParsedRecord) -> Vec<(String, u32)> {
    let mut out = Vec::new();
    for sub in &record.subrecords {
        let is_single_formid = (sub.semantic_type.as_deref() == Some("formid")
            && sub.data.len() >= 4)
            || (sub.data.len() == 4 && is_known_formid_subrecord(sub.signature.as_str()));
        if is_single_formid {
            out.push((sub.signature.to_string(), read_le_u32(&sub.data)));
        }
    }
    out
}

fn count_inbound_refs(
    target_form_key: &str,
    refs_by_handle: &HashMap<u64, Arc<RefsSection>>,
) -> usize {
    let mut total = 0usize;
    for refs_section in refs_by_handle.values() {
        if let Some(refs) = form_key_refs(&refs_section.reverse_refs_by_form_key, target_form_key) {
            total += refs.len();
        }
    }
    total
}

fn collect_record_refs<'a>(items: &'a [ParsedItem], out: &mut Vec<&'a ParsedRecord>) {
    for item in items {
        match item {
            ParsedItem::Record(record) => out.push(record),
            ParsedItem::Group(group) => collect_record_refs(&group.children, out),
        }
    }
}

fn records_identical(a: &ParsedRecord, b: &ParsedRecord) -> bool {
    if a.flags != b.flags {
        return false;
    }
    if a.subrecords.len() != b.subrecords.len() {
        return false;
    }
    for (l, r) in a.subrecords.iter().zip(b.subrecords.iter()) {
        if !subrecords_identical(l, r) {
            return false;
        }
    }
    true
}

fn subrecords_identical(a: &ParsedSubrecord, b: &ParsedSubrecord) -> bool {
    a.signature == b.signature && a.data.as_ref() == b.data.as_ref()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issue_struct_carries_path_and_signature() {
        let issue = Issue {
            severity: "error",
            category: "broken_reference",
            plugin_handle: 1,
            plugin_name: "p.esp".to_string(),
            message: "msg".to_string(),
            form_id: Some(0x1234),
            path: Some("\\ p.esp \\ [GRUP Top \"REFR\"] \\ [REFR:00001234]".to_string()),
            signature: Some("REFR".to_string()),
        };
        assert_eq!(
            issue.path.as_deref(),
            Some("\\ p.esp \\ [GRUP Top \"REFR\"] \\ [REFR:00001234]")
        );
        assert_eq!(issue.signature.as_deref(), Some("REFR"));
    }
}

fn run_structural_validation(
    handle_id: u64,
    load_order: &[(u64, String)],
    active_plugin: &ParsedPlugin,
) -> PyResult<Vec<Issue>> {
    let mut active_records = Vec::new();
    collect_record_refs(&active_plugin.root_items, &mut active_records);
    let active_refs = Arc::new(build_refs_section(active_plugin));
    let mut store = plugin_handle_store_ref().lock().unwrap();

    let mut cores: HashMap<u64, Arc<CoreSection>> = HashMap::new();
    let mut signatures_by_handle: HashMap<u64, HashMap<u32, String>> = HashMap::new();
    let mut metas: HashMap<u64, PluginMeta> = HashMap::new();
    let mut name_to_handle: HashMap<String, u64> = HashMap::new();

    for (h, _name) in load_order {
        if let Some(slot) = store.get_mut(h) {
            let core = ensure_core_section(slot);
            signatures_by_handle.insert(
                *h,
                core.by_form_key
                    .values()
                    .map(|entry| (entry.object_id, entry.signature.to_string()))
                    .collect(),
            );
            cores.insert(*h, core);
            let masters_lower: Vec<String> = slot
                .parsed
                .header
                .masters
                .iter()
                .map(|m| m.to_ascii_lowercase())
                .collect();
            let meta = PluginMeta {
                name: slot.parsed.plugin_name.clone(),
                own_index: (slot.parsed.header.masters.len() & 0xFF) as u8,
                masters: slot.parsed.header.masters.clone(),
                masters_lower,
                is_master: slot.parsed.header.flags & HEADER_FLAG_MASTER != 0,
            };
            name_to_handle.insert(meta.name.to_ascii_lowercase(), *h);
            metas.insert(*h, meta);
        }
    }
    let active_meta = metas
        .get(&handle_id)
        .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;

    let mut itm_master_records: HashMap<(u64, u32), ParsedRecord> = HashMap::new();
    if !active_meta.is_master {
        for record in &active_records {
            let high = ((record.form_id >> 24) & 0xFF) as usize;
            let Some(master_name) = active_meta.masters_lower.get(high) else {
                continue;
            };
            let Some(master_handle) = name_to_handle.get(master_name).copied() else {
                continue;
            };
            let Some(master_meta) = metas.get(&master_handle) else {
                continue;
            };
            if !master_meta.is_master {
                continue;
            }
            let object_id = record.form_id & 0x00FF_FFFF;
            let probe = ((master_meta.own_index as u32) << 24) | object_id;
            let Some(master_slot) = store.get_mut(&master_handle) else {
                continue;
            };
            let master_record = lazy_materialize_record(master_slot, probe)
                .or_else(|| lazy_materialize_record(master_slot, record.form_id))
                .or_else(|| {
                    let records = ensure_records_section(master_slot);
                    records
                        .record(&master_slot.parsed, probe)
                        .or_else(|| records.record(&master_slot.parsed, record.form_id))
                        .cloned()
                });
            if let Some(master_record) = master_record {
                itm_master_records.insert((master_handle, object_id), master_record);
            }
        }
    }
    drop(store);

    let refs_by_handle = HashMap::from([(handle_id, active_refs)]);

    // Schema for Class C reference-target-type checks. Best-effort: if the game
    // is unset or unsupported, C is skipped (B/UDR/ITM still run).
    let schema = active_plugin
        .game
        .as_deref()
        .filter(|g| !g.is_empty())
        .and_then(|g| compiled_schema_for_game(g).ok());

    let mut issues = Vec::new();

    // 1. Missing masters.
    for master in &active_meta.masters_lower {
        if !name_to_handle.contains_key(master) {
            issues.push(Issue {
                severity: "error",
                category: "missing_master",
                plugin_handle: handle_id,
                plugin_name: active_meta.name.clone(),
                message: format!("Master '{master}' is not loaded"),
                form_id: None,
                path: None,
                signature: None,
            });
        }
    }

    // 2..5: per-record checks.
    use rayon::prelude::*;
    let per_record_issues: Vec<Vec<Issue>> = active_records
        .par_iter()
        .map(|record| {
            let record = *record;
            let mut record_issues = Vec::new();
            // 2. Parse error.
            if let Some(pe) = &record.parse_error {
                record_issues.push(Issue {
                    severity: "error",
                    category: "parse_error",
                    plugin_handle: handle_id,
                    plugin_name: active_meta.name.clone(),
                    message: format!("Parse error in {}: {pe}", record.signature.as_str()),
                    form_id: Some(record.form_id),
                    path: None,
                    signature: Some(record.signature.to_string()),
                });
            }

            // 3. Broken outbound references.
            for raw_fid in iter_raw_referenced_form_ids(record) {
                if raw_fid == 0 {
                    continue;
                }
                let resolved = resolve_form_id(
                    raw_fid,
                    active_meta,
                    &metas,
                    &cores,
                    &name_to_handle,
                    handle_id,
                );
                if !resolved {
                    record_issues.push(Issue {
                        severity: "warning",
                        category: "broken_reference",
                        plugin_handle: handle_id,
                        plugin_name: active_meta.name.clone(),
                        message: format!(
                            "{} 0x{:08X} -> unresolved 0x{:08X}",
                            record.signature.as_str(),
                            record.form_id,
                            raw_fid
                        ),
                        form_id: Some(record.form_id),
                        path: None,
                        signature: Some(record.signature.to_string()),
                    });
                }
            }

            // 3b. Class C — reference target-type mismatch (wbFormIDCk parity).
            // Subrecord-level FK fields only; struct-internal FK fields are a
            // follow-up (they need the per-field decoder, ~23% of C errors).
            // Mirrors xEdit TwbFormIDChecked.Check (wbInterface.pas:19850):
            // a resolved ref whose target signature is not in the allowed set, or
            // a NULL where NULL is not allowed.
            if let Some(schema) = schema.as_ref() {
                let record_sig = record.signature.as_str();
                for (sub_sig, raw_fid) in iter_subrecord_level_refs(record) {
                    let Some(spec) = schema.allowed_targets(record_sig, &sub_sig) else {
                        continue;
                    };
                    if raw_fid == 0 {
                        if !spec.null_allowed {
                            record_issues.push(make_ref_type_issue(
                                handle_id,
                                &active_meta.name,
                                record,
                                &sub_sig,
                                "NULL",
                                &spec.expected_label(),
                            ));
                        }
                        continue;
                    }
                    // Resolved-to-wrong-type. Unresolved refs are the Class B case
                    // already emitted above — don't double-report here.
                    if let Some(target_sig) = resolve_form_id_target_sig(
                        raw_fid,
                        active_meta,
                        &metas,
                        &cores,
                        &signatures_by_handle,
                        &name_to_handle,
                        handle_id,
                    ) {
                        if !spec.allows_target(&target_sig) {
                            record_issues.push(make_ref_type_issue(
                                handle_id,
                                &active_meta.name,
                                record,
                                &sub_sig,
                                &target_sig,
                                &spec.expected_label(),
                            ));
                        }
                    }
                }
            }

            // 4. UDR — deleted record still referenced.
            if record.flags & RECORD_FLAG_DELETED != 0 {
                let active_plugin_name: Arc<str> = Arc::from(active_meta.name.as_str());
                let target_form_key = resolve_form_id_to_form_key(
                    record.form_id,
                    &active_plugin_name,
                    &active_meta.masters,
                );
                let count = count_inbound_refs(target_form_key.render().as_str(), &refs_by_handle);
                if count > 0 {
                    record_issues.push(Issue {
                        severity: "warning",
                        category: "udr",
                        plugin_handle: handle_id,
                        plugin_name: active_meta.name.clone(),
                        message: format!(
                            "Deleted record 0x{:08X} is still referenced by {count} record(s)",
                            record.form_id
                        ),
                        form_id: Some(record.form_id),
                        path: None,
                        signature: Some(record.signature.to_string()),
                    });
                }
            }

            // 5. ITM — only meaningful for non-master plugins overriding a master.
            if !active_meta.is_master {
                let high = ((record.form_id >> 24) & 0xFF) as u8;
                if (high as usize) < active_meta.masters_lower.len() {
                    let master_name = &active_meta.masters_lower[high as usize];
                    if let Some(other_handle) = name_to_handle.get(master_name) {
                        if let Some(other_meta) = metas.get(other_handle) {
                            if other_meta.is_master {
                                let object_id = record.form_id & 0x00FF_FFFF;
                                if let Some(other_record) =
                                    itm_master_records.get(&(*other_handle, object_id))
                                {
                                    if records_identical(record, other_record) {
                                        record_issues.push(Issue {
                                            severity: "info",
                                            category: "itm",
                                            plugin_handle: handle_id,
                                            plugin_name: active_meta.name.clone(),
                                            message: format!(
                                                "{} 0x{:08X} is identical to master ({})",
                                                record.signature.as_str(),
                                                record.form_id,
                                                other_meta.name
                                            ),
                                            form_id: Some(record.form_id),
                                            path: None,
                                            signature: Some(record.signature.to_string()),
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
            record_issues
        })
        .collect();
    issues.extend(per_record_issues.into_iter().flatten());

    Ok(issues)
}

fn issue_to_payload(issue: &Issue) -> IssuePayload {
    (
        issue.severity.to_string(),
        issue.category.to_string(),
        issue.plugin_handle,
        issue.plugin_name.clone(),
        issue.message.clone(),
        issue.form_id,
        issue.path.clone(),
        issue.signature.clone(),
    )
}

/// Walk the plugin tree element-by-element and emit xEdit-parity issues.
///
/// Returns issues with: severity, category, plugin_handle, plugin_name,
/// message, form_id, path, signature. Includes everything `validate_plugin_native`
/// emits PLUS schema-driven subrecord ordering and unused-data warnings.
#[pyfunction(name = "validate_plugin_deep_native", signature = (handle_id, load_order))]
pub(crate) fn validate_plugin_deep_native(
    py: Python<'_>,
    handle_id: u64,
    load_order: Vec<(u64, String)>,
) -> PyResult<Vec<IssuePayload>> {
    use crate::validate_walker::walk_and_check;

    let (structural_issues, walker_issues) = py.detach(move || -> PyResult<_> {
        let plugin = {
            let store = plugin_handle_store_ref().lock().unwrap();
            store
                .get(&handle_id)
                .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?
                .parsed
                .clone()
        };
        let structural_issues = run_structural_validation(handle_id, &load_order, &plugin)?;
        let game_str = plugin.game.as_deref().unwrap_or("");
        let schema = compiled_schema_for_game(game_str)?;
        let load_index = load_order
            .iter()
            .position(|(h, _)| *h == handle_id)
            .unwrap_or(0);
        let mut out = Vec::new();
        walk_and_check(&plugin, load_index, handle_id, &schema, &mut out);
        Ok((structural_issues, out))
    })?;

    let mut payload = Vec::with_capacity(structural_issues.len() + walker_issues.len());
    payload.extend(structural_issues.iter().map(issue_to_payload));
    payload.extend(walker_issues.iter().map(walker_issue_to_payload));
    Ok(payload)
}

fn walker_issue_to_payload(issue: &crate::validate_walker::WalkerIssue) -> IssuePayload {
    (
        issue.severity.to_string(),
        issue.category.to_string(),
        issue.plugin_handle,
        issue.plugin_name.clone(),
        issue.message.clone(),
        issue.form_id,
        issue.path.clone(),
        issue.signature.clone(),
    )
}
