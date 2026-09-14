//! Batch FormKey rewrite/scan/replace over `serde_json::Value` trees.
//!
//! Rewrite matches `bacup_lib.formkey.formkey_mapper.FormKeyMapper.rewrite_formkeys`.
//! All three batch entry points take a JSON string (an array of records),
//! parse once, walk natively, and return a JSON string the caller decodes.

use serde_json::{Map as JsonMap, Value as JsonValue};

/// Match the Python regex `^[0-9A-Fa-f]{2,6}:.+\.(esm|esp|esl)$` (fullmatch).
/// Case-sensitive on the `.es?` suffix to match Python's default flags.
fn is_form_key_string(s: &str) -> bool {
    let bytes = s.as_bytes();
    let n = bytes.len();
    // Minimum length: 2 hex + ':' + 1 char + ".es?" = 8
    if n < 8 {
        return false;
    }
    let mut hex_end = 0;
    while hex_end < n && hex_end < 7 {
        let b = bytes[hex_end];
        if matches!(b, b'0'..=b'9' | b'A'..=b'F' | b'a'..=b'f') {
            hex_end += 1;
        } else {
            break;
        }
    }
    if !(2..=6).contains(&hex_end) {
        return false;
    }
    if bytes[hex_end] != b':' {
        return false;
    }
    if n < hex_end + 1 + 1 + 4 {
        return false;
    }
    let tail = &bytes[n - 4..];
    tail[0] == b'.' && tail[1] == b'e' && tail[2] == b's' && matches!(tail[3], b'm' | b'p' | b'l')
}

/// Mirror of `bacup_lib.yaml_helpers.from_ref`.
/// Returns `Some("OBJID:Plugin.esm")` when `value` is a canonical-ref dict.
fn from_ref(value: &JsonValue) -> Option<String> {
    let outer = value.as_object()?;
    let inner = outer.get("reference")?.as_object()?;

    let plugin = match inner.get("plugin") {
        Some(JsonValue::String(s)) => s.as_str(),
        _ => return None,
    };
    if plugin.is_empty() {
        return None;
    }

    // Python: obj_id = inner.get("object_id", ""); if obj_id is None: return None
    // Then str(obj_id); skip null FK if lstrip("0") == "".
    let obj_id_str = match inner.get("object_id") {
        None => return None,
        Some(JsonValue::Null) => return None,
        Some(JsonValue::String(s)) => s.clone(),
        Some(JsonValue::Number(n)) => n.to_string(),
        Some(JsonValue::Bool(b)) => b.to_string(),
        Some(_) => return None,
    };
    if obj_id_str.is_empty() || obj_id_str.bytes().all(|c| c == b'0') {
        return None;
    }
    Some(format!("{}:{}", obj_id_str, plugin))
}

/// Mirror of `bacup_lib.yaml_helpers.to_ref`.
fn to_ref(fk: &str) -> Option<JsonValue> {
    let (obj_id, plugin) = fk.split_once(':')?;
    let mut inner = JsonMap::new();
    inner.insert("plugin".to_string(), JsonValue::String(plugin.to_string()));
    inner.insert(
        "object_id".to_string(),
        JsonValue::String(obj_id.to_string()),
    );
    let mut outer = JsonMap::new();
    outer.insert("reference".to_string(), JsonValue::Object(inner));
    Some(JsonValue::Object(outer))
}

// ---------------------------------------------------------------------------
// rewrite_formkeys
// ---------------------------------------------------------------------------

/// `mappings` is a JSON object: `{ "src_fk": { "new_formkey": "...", ... } }`.
/// Build a flat lookup `src_fk -> new_formkey` for the recursive walk.
pub(crate) fn build_rewrite_lookup_pub(
    mappings: &JsonValue,
) -> std::collections::HashMap<String, String> {
    build_rewrite_lookup(mappings)
}

fn build_rewrite_lookup(mappings: &JsonValue) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    if let Some(obj) = mappings.as_object() {
        for (src_fk, mapping) in obj.iter() {
            let new_fk = mapping
                .as_object()
                .and_then(|m| m.get("new_formkey"))
                .and_then(|v| v.as_str());
            if let Some(new_fk) = new_fk {
                out.insert(src_fk.clone(), new_fk.to_string());
            }
        }
    }
    out
}

pub(crate) fn rewrite_formkeys_walk_pub(
    value: JsonValue,
    lookup: &std::collections::HashMap<String, String>,
) -> JsonValue {
    rewrite_formkeys_walk(value, lookup)
}

fn rewrite_formkeys_walk(
    value: JsonValue,
    lookup: &std::collections::HashMap<String, String>,
) -> JsonValue {
    match value {
        JsonValue::String(s) => {
            if is_form_key_string(&s) {
                if let Some(new_fk) = lookup.get(&s) {
                    return JsonValue::String(new_fk.clone());
                }
            }
            JsonValue::String(s)
        }
        JsonValue::Object(obj) => {
            // Canonical-ref shape: dict whose "reference" key is a dict with
            // both "plugin" and "object_id". Match Python: when this shape is
            // present, return either the rewritten dict or the original
            // unchanged — do NOT recurse into siblings.
            let is_canonical_ref = matches!(
                obj.get("reference"),
                Some(JsonValue::Object(inner))
                    if inner.contains_key("plugin") && inner.contains_key("object_id")
            );
            let owned = JsonValue::Object(obj);
            if is_canonical_ref {
                if let Some(src_fk) = from_ref(&owned) {
                    if let Some(new_fk) = lookup.get(&src_fk) {
                        if let Some(new_ref) = to_ref(new_fk) {
                            // Preserve sibling keys (Python: dict(data); rewritten["reference"] = ...)
                            let mut rewritten = match owned {
                                JsonValue::Object(map) => map,
                                _ => unreachable!(),
                            };
                            if let JsonValue::Object(mut new_ref_obj) = new_ref {
                                if let Some(new_inner) = new_ref_obj.remove("reference") {
                                    rewritten.insert("reference".to_string(), new_inner);
                                }
                            }
                            return JsonValue::Object(rewritten);
                        }
                    }
                }
                return owned;
            }
            // Non-canonical dict: recurse into every value.
            let map = match owned {
                JsonValue::Object(m) => m,
                _ => unreachable!(),
            };
            let mut out = JsonMap::with_capacity(map.len());
            for (k, v) in map {
                out.insert(k, rewrite_formkeys_walk(v, lookup));
            }
            JsonValue::Object(out)
        }
        JsonValue::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(rewrite_formkeys_walk(item, lookup));
            }
            JsonValue::Array(out)
        }
        other => other,
    }
}

pub(crate) fn rewrite_formkeys_batch_json(
    records_json: &str,
    mappings_json: &str,
) -> Result<String, String> {
    let records: JsonValue =
        serde_json::from_str(records_json).map_err(|e| format!("invalid records JSON: {e}"))?;
    let mappings: JsonValue =
        serde_json::from_str(mappings_json).map_err(|e| format!("invalid mappings JSON: {e}"))?;
    let lookup = build_rewrite_lookup(&mappings);

    let rewritten = match records {
        JsonValue::Array(items) => {
            // Empty mapping is a no-op; preserve identity to avoid unnecessary churn.
            if lookup.is_empty() {
                JsonValue::Array(items)
            } else {
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    out.push(rewrite_formkeys_walk(item, &lookup));
                }
                JsonValue::Array(out)
            }
        }
        // Single value tolerated for parity with Python static method.
        other => rewrite_formkeys_walk(other, &lookup),
    };

    serde_json::to_string(&rewritten).map_err(|e| format!("failed to serialize rewritten: {e}"))
}

// ---------------------------------------------------------------------------
// find_stale_formkeys
// ---------------------------------------------------------------------------

pub(crate) fn find_stale_walk_pub(
    value: &JsonValue,
    source_plugins: &std::collections::HashSet<String>,
    found: &mut std::collections::HashSet<String>,
) {
    find_stale_walk(value, source_plugins, found)
}

fn find_stale_walk(
    value: &JsonValue,
    source_plugins: &std::collections::HashSet<String>,
    found: &mut std::collections::HashSet<String>,
) {
    match value {
        JsonValue::String(s) => {
            if is_form_key_string(s) {
                if let Some((_, plugin)) = s.split_once(':') {
                    let plugin_lc = plugin.to_ascii_lowercase();
                    if source_plugins.contains(&plugin_lc) {
                        found.insert(s.clone());
                    }
                }
            }
        }
        JsonValue::Object(obj) => {
            // Try canonical-ref (Python checks every dict, regardless of
            // whether it looks like one — from_ref returns None for non-refs).
            if let Some(fk) = from_ref(value) {
                if let Some((_, plugin)) = fk.split_once(':') {
                    let plugin_lc = plugin.to_ascii_lowercase();
                    if source_plugins.contains(&plugin_lc) {
                        found.insert(fk);
                    }
                }
            }
            for v in obj.values() {
                find_stale_walk(v, source_plugins, found);
            }
        }
        JsonValue::Array(items) => {
            for item in items {
                find_stale_walk(item, source_plugins, found);
            }
        }
        _ => {}
    }
}

pub(crate) fn find_stale_formkeys_batch_json(
    records_json: &str,
    source_plugins_json: &str,
) -> Result<String, String> {
    let records: JsonValue =
        serde_json::from_str(records_json).map_err(|e| format!("invalid records JSON: {e}"))?;
    let source_plugins_value: JsonValue = serde_json::from_str(source_plugins_json)
        .map_err(|e| format!("invalid source_plugins JSON: {e}"))?;

    let source_plugins: std::collections::HashSet<String> = match source_plugins_value {
        JsonValue::Array(items) => items
            .into_iter()
            .filter_map(|v| match v {
                JsonValue::String(s) => Some(s.to_ascii_lowercase()),
                _ => None,
            })
            .collect(),
        _ => return Err("source_plugins must be a JSON array of strings".to_string()),
    };

    let mut found: std::collections::HashSet<String> = std::collections::HashSet::new();
    find_stale_walk(&records, &source_plugins, &mut found);

    let mut as_vec: Vec<String> = found.into_iter().collect();
    as_vec.sort();
    serde_json::to_string(&as_vec).map_err(|e| format!("failed to serialize found set: {e}"))
}

// ---------------------------------------------------------------------------
// replace_formkeys (with null=remove)
// ---------------------------------------------------------------------------

/// Lookup value: `Some(replacement)` to substitute, `None` to remove.
pub(crate) type ReplaceLookup = std::collections::HashMap<String, Option<String>>;

pub(crate) fn build_replace_lookup_pub(replacements: &JsonValue) -> Result<ReplaceLookup, String> {
    build_replace_lookup(replacements)
}

fn build_replace_lookup(replacements: &JsonValue) -> Result<ReplaceLookup, String> {
    let obj = replacements
        .as_object()
        .ok_or_else(|| "replacements must be a JSON object".to_string())?;
    let mut out = ReplaceLookup::with_capacity(obj.len());
    for (src_fk, v) in obj.iter() {
        match v {
            JsonValue::Null => {
                out.insert(src_fk.clone(), None);
            }
            JsonValue::String(replacement) => {
                out.insert(src_fk.clone(), Some(replacement.clone()));
            }
            _ => {
                return Err(format!("replacement for {src_fk:?} must be string or null"));
            }
        }
    }
    Ok(out)
}

pub(crate) fn replace_formkeys_walk_pub(
    value: JsonValue,
    lookup: &ReplaceLookup,
) -> Option<JsonValue> {
    replace_formkeys_walk(value, lookup)
}

/// Walk that returns `None` to mean "drop this value" (mirrors Python's
/// `_REMOVE_SENTINEL`). Caller filters dict keys / list items accordingly.
fn replace_formkeys_walk(value: JsonValue, lookup: &ReplaceLookup) -> Option<JsonValue> {
    match value {
        JsonValue::String(s) => {
            if let Some(entry) = lookup.get(&s) {
                match entry {
                    None => None,
                    Some(replacement) => Some(JsonValue::String(replacement.clone())),
                }
            } else {
                Some(JsonValue::String(s))
            }
        }
        JsonValue::Object(obj) => {
            let owned = JsonValue::Object(obj);
            // Try canonical-ref first.
            if let Some(fk) = from_ref(&owned) {
                if let Some(entry) = lookup.get(&fk) {
                    match entry {
                        None => return None, // remove the whole canonical-ref dict
                        Some(replacement) => {
                            if let Some(new_ref) = to_ref(replacement) {
                                let mut rewritten = match owned {
                                    JsonValue::Object(m) => m,
                                    _ => unreachable!(),
                                };
                                if let JsonValue::Object(mut new_ref_obj) = new_ref {
                                    if let Some(new_inner) = new_ref_obj.remove("reference") {
                                        rewritten.insert("reference".to_string(), new_inner);
                                    }
                                }
                                return Some(JsonValue::Object(rewritten));
                            }
                            // to_ref None (replacement lacks ':') -> fall through to general recursion.
                        }
                    }
                }
            }
            // General recursion: drop keys whose value resolves to remove.
            let map = match owned {
                JsonValue::Object(m) => m,
                _ => unreachable!(),
            };
            let mut out = JsonMap::with_capacity(map.len());
            for (k, v) in map {
                if let Some(new_v) = replace_formkeys_walk(v, lookup) {
                    out.insert(k, new_v);
                }
            }
            Some(JsonValue::Object(out))
        }
        JsonValue::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                if let Some(new_v) = replace_formkeys_walk(item, lookup) {
                    out.push(new_v);
                }
            }
            Some(JsonValue::Array(out))
        }
        other => Some(other),
    }
}

pub(crate) fn replace_formkeys_batch_json(
    records_json: &str,
    replacements_json: &str,
) -> Result<String, String> {
    let records: JsonValue =
        serde_json::from_str(records_json).map_err(|e| format!("invalid records JSON: {e}"))?;
    let replacements: JsonValue = serde_json::from_str(replacements_json)
        .map_err(|e| format!("invalid replacements JSON: {e}"))?;
    let lookup = build_replace_lookup(&replacements)?;

    let rewritten = match records {
        JsonValue::Array(items) => {
            if lookup.is_empty() {
                JsonValue::Array(items)
            } else {
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    if let Some(v) = replace_formkeys_walk(item, &lookup) {
                        out.push(v);
                    }
                }
                JsonValue::Array(out)
            }
        }
        other => replace_formkeys_walk(other, &lookup).unwrap_or(JsonValue::Null),
    };

    serde_json::to_string(&rewritten).map_err(|e| format!("failed to serialize rewritten: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn fk_pattern_matches_canonical() {
        assert!(is_form_key_string("000800:Foo.esp"));
        assert!(is_form_key_string("AB:Bar.esm"));
        assert!(is_form_key_string("AAFFEE:Baz.esl"));
        assert!(is_form_key_string("123abc:plugin.with.dots.esp"));
    }

    #[test]
    fn fk_pattern_rejects() {
        assert!(!is_form_key_string("0:Short.esm"));
        assert!(!is_form_key_string("ABCDEFG:Toolong.esm"));
        assert!(!is_form_key_string("XYZZ:Nothex.esm"));
        assert!(!is_form_key_string("000800:NoExt"));
        assert!(!is_form_key_string("000800:Bad.txt"));
        assert!(!is_form_key_string("000800.esp"));
        assert!(!is_form_key_string(""));
        assert!(!is_form_key_string("hello world"));
    }

    #[test]
    fn rewrite_simple_string() {
        let records = r#"["591667:SeventySix.esm"]"#;
        let mappings = r#"{"591667:SeventySix.esm":{"new_formkey":"013F42:Fallout4.esm"}}"#;
        let out = rewrite_formkeys_batch_json(records, mappings).unwrap();
        assert_eq!(out, r#"["013F42:Fallout4.esm"]"#);
    }

    #[test]
    fn rewrite_canonical_ref_preserves_siblings() {
        let records = json!([
            {
                "reference": {"plugin": "SeventySix.esm", "object_id": "591667"},
                "sibling": "untouched"
            }
        ]);
        let mappings = json!({
            "591667:SeventySix.esm": {"new_formkey": "013F42:Fallout4.esm"}
        });
        let out = rewrite_formkeys_batch_json(&records.to_string(), &mappings.to_string()).unwrap();
        let parsed: JsonValue = serde_json::from_str(&out).unwrap();
        assert_eq!(parsed[0]["reference"]["plugin"], "Fallout4.esm");
        assert_eq!(parsed[0]["reference"]["object_id"], "013F42");
        assert_eq!(parsed[0]["sibling"], "untouched");
    }

    #[test]
    fn rewrite_unmapped_unchanged() {
        let records = r#"["AAAAAA:Other.esm","not a fk"]"#;
        let mappings = r#"{"591667:SeventySix.esm":{"new_formkey":"013F42:Fallout4.esm"}}"#;
        let out = rewrite_formkeys_batch_json(records, mappings).unwrap();
        let parsed: JsonValue = serde_json::from_str(&out).unwrap();
        assert_eq!(parsed[0], "AAAAAA:Other.esm");
        assert_eq!(parsed[1], "not a fk");
    }

    #[test]
    fn rewrite_nested_dict_recursion() {
        let records = json!([
            {
                "FormKey": "55C153:SeventySix.esm",
                "Keywords": ["591667:SeventySix.esm", "AAAAAA:Other.esm"],
                "Nested": {"Ref": "55C153:SeventySix.esm"},
                "NotAFormKey": "hello",
                "Number": 42
            }
        ]);
        let mappings = json!({
            "591667:SeventySix.esm": {"new_formkey": "013F42:Fallout4.esm"},
            "55C153:SeventySix.esm": {"new_formkey": "000800:B21_Test.esp"}
        });
        let out = rewrite_formkeys_batch_json(&records.to_string(), &mappings.to_string()).unwrap();
        let parsed: JsonValue = serde_json::from_str(&out).unwrap();
        assert_eq!(parsed[0]["FormKey"], "000800:B21_Test.esp");
        assert_eq!(parsed[0]["Keywords"][0], "013F42:Fallout4.esm");
        assert_eq!(parsed[0]["Keywords"][1], "AAAAAA:Other.esm");
        assert_eq!(parsed[0]["Nested"]["Ref"], "000800:B21_Test.esp");
        assert_eq!(parsed[0]["NotAFormKey"], "hello");
        assert_eq!(parsed[0]["Number"], 42);
    }

    #[test]
    fn find_stale_dedupes_and_filters() {
        let records = json!([
            "591667:SeventySix.esm",
            "AAAAAA:Other.esm",
            ["591667:SeventySix.esm", {"reference": {"plugin": "SeventySix.esm", "object_id": "55C153"}}]
        ]);
        let plugins = json!(["seventysix.esm"]);
        let out =
            find_stale_formkeys_batch_json(&records.to_string(), &plugins.to_string()).unwrap();
        let parsed: Vec<String> = serde_json::from_str(&out).unwrap();
        assert_eq!(
            parsed,
            vec![
                "55C153:SeventySix.esm".to_string(),
                "591667:SeventySix.esm".to_string()
            ]
        );
    }

    #[test]
    fn replace_with_null_drops_keys_and_items() {
        let records = json!([
            {
                "Keep": "no_fk",
                "Drop": "591667:SeventySix.esm",
                "List": ["591667:SeventySix.esm", "Keep me"]
            }
        ]);
        let replacements = json!({"591667:SeventySix.esm": null});
        let out =
            replace_formkeys_batch_json(&records.to_string(), &replacements.to_string()).unwrap();
        let parsed: JsonValue = serde_json::from_str(&out).unwrap();
        assert_eq!(parsed[0]["Keep"], "no_fk");
        assert!(parsed[0].as_object().unwrap().get("Drop").is_none());
        assert_eq!(parsed[0]["List"].as_array().unwrap().len(), 1);
        assert_eq!(parsed[0]["List"][0], "Keep me");
    }

    #[test]
    fn replace_canonical_ref_dict() {
        let records = json!([
            {
                "reference": {"plugin": "SeventySix.esm", "object_id": "591667"},
                "sibling": "x"
            }
        ]);
        // Replace the canonical-ref FK with a string -> rewrite reference inner
        let replacements = json!({"591667:SeventySix.esm": "013F42:Fallout4.esm"});
        let out =
            replace_formkeys_batch_json(&records.to_string(), &replacements.to_string()).unwrap();
        let parsed: JsonValue = serde_json::from_str(&out).unwrap();
        assert_eq!(parsed[0]["reference"]["plugin"], "Fallout4.esm");
        assert_eq!(parsed[0]["reference"]["object_id"], "013F42");
        assert_eq!(parsed[0]["sibling"], "x");
    }

    #[test]
    fn replace_canonical_ref_null_drops_dict() {
        let records = json!([
            {
                "container": {
                    "reference": {"plugin": "SeventySix.esm", "object_id": "591667"},
                    "sibling": "x"
                },
                "keep": 1
            }
        ]);
        let replacements = json!({"591667:SeventySix.esm": null});
        let out =
            replace_formkeys_batch_json(&records.to_string(), &replacements.to_string()).unwrap();
        let parsed: JsonValue = serde_json::from_str(&out).unwrap();
        assert!(parsed[0].as_object().unwrap().get("container").is_none());
        assert_eq!(parsed[0]["keep"], 1);
    }

    #[test]
    fn from_ref_skips_null_object_id() {
        let v = json!({"reference": {"plugin": "P.esm", "object_id": "000000"}});
        assert_eq!(from_ref(&v), None);
        let v = json!({"reference": {"plugin": "P.esm", "object_id": ""}});
        assert_eq!(from_ref(&v), None);
        let v = json!({"reference": {"plugin": "", "object_id": "591667"}});
        assert_eq!(from_ref(&v), None);
        let v = json!({"reference": {"plugin": "P.esm", "object_id": "591667"}});
        assert_eq!(from_ref(&v), Some("591667:P.esm".to_string()));
    }

    #[test]
    fn empty_mapping_is_noop() {
        let records = r#"[{"a":"591667:SeventySix.esm"}]"#;
        let out = rewrite_formkeys_batch_json(records, "{}").unwrap();
        let parsed: JsonValue = serde_json::from_str(&out).unwrap();
        assert_eq!(parsed[0]["a"], "591667:SeventySix.esm");
    }
}
