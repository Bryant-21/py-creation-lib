// Authoring write-side — serialize_authoring_subrecord and helpers.
//
// This module is the Rust replacement for
// py_creation_lib/python/creation_lib/esp/authoring.py's
// serialize_authoring_subrecord family.

use super::super::{
    CompiledSchema, LocalizedStringsState, SchemaArrayJson, SchemaConditionJson, SchemaEnumJson,
    SchemaFieldJson, SchemaRecordJson, SchemaSubrecordJson, SchemaUnionVariantJson,
    authoring_camel_case, authoring_key_name, compiled_schema_for_game, schema_game_for_plugin,
    schema_record_spec,
};
use crate::plugin_runtime::condition_functions::{
    CtdaParamKey, infer_game_from_plugins, lookup_ctda_function,
};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

/// Display-name order used for localized authoring payloads. Matches
/// `creation_lib.esp.strings.LANGUAGE_DISPLAY_ORDER` exactly — byte-exact YAML output
/// depends on this ordering.
pub const LANGUAGE_DISPLAY_ORDER: [&str; 14] = [
    "Chinese",
    "ChineseSimplified",
    "ChineseTraditional",
    "German",
    "English",
    "Spanish",
    "Spanish_Mexico",
    "French",
    "Italian",
    "Japanese",
    "Korean",
    "Polish",
    "Portuguese_Brazil",
    "Russian",
];

/// Map an internal language code (as stored in `LocalizedStringsState`) to the
/// display name used in authoring-dir YAML. Mirrors
/// `creation_lib.esp.strings.LANGUAGE_DISPLAY_NAMES` — unknown codes pass through.
pub fn language_display_name(code: &str) -> &str {
    match code {
        "cn" => "Chinese",
        "zhhans" => "ChineseSimplified",
        "zhhant" => "ChineseTraditional",
        "de" => "German",
        "en" => "English",
        "es" => "Spanish",
        "esmx" => "Spanish_Mexico",
        "fr" => "French",
        "it" => "Italian",
        "ja" => "Japanese",
        "ko" => "Korean",
        "pl" => "Polish",
        "ptbr" => "Portuguese_Brazil",
        "ru" => "Russian",
        other => other,
    }
}

/// Pure-Rust equivalent of `native_layout_for_subrecord` in plugin_runtime.rs,
/// working directly off `SchemaSubrecordJson`. Returns a `&'static str` to
/// avoid per-call allocations.
pub fn runtime_layout_for_subrecord_schema(
    subrecord: &SchemaSubrecordJson,
) -> Option<&'static str> {
    if let Some(layout) = subrecord.authoring_layout.as_ref() {
        return match layout.as_str() {
            "mapping" => Some("mapping"),
            "row_array" => Some("row_array"),
            "union" => Some("union"),
            "vmad" => Some("vmad"),
            _ => None,
        };
    }
    if !subrecord.union_variants.is_empty() {
        return Some("union");
    }
    if let Some(codec) = subrecord.codec.as_ref() {
        if codec.starts_with("array_struct:") {
            return Some("row_array");
        }
        if codec.starts_with("struct:") {
            return Some("mapping");
        }
    }
    if subrecord.id == "VMAD" {
        return Some("vmad");
    }
    None
}

// -------------------------------------------------------------------------
// validate_record — pure Rust port of `creation_lib.esp.authoring.validate_record`.
//
// Checks that a Python `Record` (PyRecord) conforms to its `RecordSpec`:
//   * no duplicate non-repeatable subrecords,
//   * no missing required subrecords.
//
// Ports `py_creation_lib/python/creation_lib/esp/authoring.py::validate_record` + `_missing_required_signatures`.
// Byte-exact invariant is not affected — this function only raises errors on
// bad input and never mutates the record.
//
// -------------------------------------------------------------------------

/// Pure-Rust validation over a `SchemaRecordJson`. Returns the user-facing
/// error message on failure, or `Ok(())` if validation passes.
///
/// `subrecord_signatures` must be the in-order signatures of every subrecord
/// in the record. Matches the Python logic byte-for-byte (including the
/// sorted-duplicate list and the join-by-`, ` format).
pub fn validate_record_signatures(
    record_signature: &str,
    subrecord_signatures: &[&str],
    record_spec: &SchemaRecordJson,
) -> Result<(), String> {
    // Build repeatable map in declaration order — the first occurrence wins,
    // matching `repeatable.setdefault(spec.sig, spec.repeatable)` in Python.
    let mut repeatable: std::collections::HashMap<&str, bool> =
        std::collections::HashMap::with_capacity(record_spec.subrecords.len());
    for spec in &record_spec.subrecords {
        repeatable
            .entry(spec.id.as_str())
            .or_insert(spec.repeatable);
    }

    // Count non-repeatable signatures only — Python only populates `counts`
    // for signatures that appear in `repeatable`, ignoring unknown signatures
    // entirely. Identical behavior here.
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for signature in subrecord_signatures {
        if repeatable.contains_key(signature) {
            *counts.entry(signature).or_insert(0) += 1;
        }
    }

    // Duplicates: any signature seen more than once whose spec is not
    // repeatable. Sort alphabetically to match Python's `sorted(...)`.
    let mut duplicates: Vec<&str> = counts
        .iter()
        .filter_map(|(signature, count)| {
            if *count > 1 && !repeatable.get(signature).copied().unwrap_or(false) {
                Some(*signature)
            } else {
                None
            }
        })
        .collect();
    duplicates.sort_unstable();
    if !duplicates.is_empty() {
        return Err(format!(
            "{record_signature} has duplicate non-repeatable subrecords: {}",
            duplicates.join(", ")
        ));
    }

    // Missing required: iterate record_spec.subrecords preserving order, skip
    // duplicates, check each required signature against `subrecord_signatures`.
    // Matches `_missing_required_signatures` byte-for-byte.
    let mut seen_required: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut missing: Vec<&str> = Vec::new();
    let present: std::collections::HashSet<&str> = subrecord_signatures.iter().copied().collect();
    for spec in &record_spec.subrecords {
        if !spec.required || seen_required.contains(spec.id.as_str()) {
            continue;
        }
        seen_required.insert(spec.id.as_str());
        if !present.contains(spec.id.as_str()) {
            missing.push(spec.id.as_str());
        }
    }
    if !missing.is_empty() {
        return Err(format!(
            "{record_signature} is missing required subrecords: {}",
            missing.join(", ")
        ));
    }
    Ok(())
}

/// Shared entry point for `validate_record` — called from the top-level
/// `#[pyfunction]` wrapper in `creation_lib.rs`. Pulls the record's game from the
/// plugin, resolves the schema, and delegates to
/// `validate_record_signatures`. Silent-OK when schema is not available
/// for the plugin (matches Python: `if record_spec is None: return`).
pub fn validate_record_impl(plugin: &Bound<'_, PyAny>, record: &Bound<'_, PyAny>) -> PyResult<()> {
    let Some(game) = schema_game_for_plugin(plugin, None)? else {
        return Ok(());
    };
    let Ok(schema) = compiled_schema_for_game(game.as_str()) else {
        return Ok(());
    };
    let record_signature: String = record.getattr("signature")?.extract()?;
    let Some(record_spec) = schema_record_spec(schema.as_ref(), record_signature.as_str()) else {
        return Ok(());
    };

    // Collect subrecord signatures into owned Strings so they outlive the
    // &str borrows fed into validate_record_signatures.
    let subrecords = record.getattr("subrecords")?;
    let mut owned: Vec<String> = Vec::new();
    for item in subrecords.try_iter()? {
        let item = item?;
        owned.push(item.getattr("signature")?.extract::<String>()?);
    }
    let sig_refs: Vec<&str> = owned.iter().map(|value| value.as_str()).collect();

    match validate_record_signatures(record_signature.as_str(), &sig_refs, record_spec) {
        Ok(()) => Ok(()),
        Err(message) => Err(PyValueError::new_err(message)),
    }
}

// =========================================================================
// GIL-free JSON serialization path (rayon export wiring)
// =========================================================================
//
// These functions form the pure-Rust parallel export path.  They never hold
// the Python GIL and return `serde_json::Value` instead of `Py<PyAny>`.
//
// Entry point: `serialize_record_payload_to_json` in plugin_runtime.rs
// calls `schema_subrecord_to_decode_spec` (Step 1) and then
// `compact_subrecord_to_json` (Step 2) for every subrecord.

// -------------------------------------------------------------------------
// Step 1 — schema_subrecord_to_decode_spec
//
// Converts a SchemaSubrecordJson directly to a crate::DecodeSpec. Returns None
// for subrecords that cannot be decoded natively (raw kind, vmad layout,
// unsupported codec, nested field arrays).
// -------------------------------------------------------------------------

/// Convert the schema condition's serde_json::Value into a crate::ConditionValue.
fn schema_condition_value_to_rust(v: &serde_json::Value) -> Option<crate::ConditionValue> {
    match v {
        serde_json::Value::Null => None,
        serde_json::Value::Bool(b) => Some(crate::ConditionValue::Bool(*b)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Some(crate::ConditionValue::Int(i as i128))
            } else if let Some(u) = n.as_u64() {
                Some(crate::ConditionValue::Int(u as i128))
            } else if let Some(f) = n.as_f64() {
                Some(crate::ConditionValue::Float(f))
            } else {
                None
            }
        }
        serde_json::Value::String(s) => Some(crate::ConditionValue::Str(s.clone())),
        _ => None,
    }
}

fn schema_conditions_to_rust(conditions: &[SchemaConditionJson]) -> Vec<crate::Condition> {
    conditions
        .iter()
        .map(|c| crate::Condition {
            field: c.field.clone(),
            operator: c.operator.clone(),
            value: c.value.as_ref().and_then(schema_condition_value_to_rust),
            values: c
                .values
                .iter()
                .filter_map(schema_condition_value_to_rust)
                .collect(),
        })
        .collect()
}

/// Mirror token_width from plugin_runtime (private there; replicated here).
fn token_width_rs(token: &str) -> Option<usize> {
    match token {
        "b" | "B" => Some(1),
        "h" | "H" => Some(2),
        "i" | "I" | "f" => Some(4),
        "q" | "Q" => Some(8),
        "x" => Some(1),
        _ if token.starts_with('s') => token[1..].parse::<usize>().ok(),
        _ => None,
    }
}

fn struct_tokens_rs(codec: &str) -> Vec<&str> {
    let payload = codec
        .strip_prefix("struct:")
        .or_else(|| codec.strip_prefix("array_struct:"))
        .unwrap_or_default();
    if payload.is_empty() {
        return Vec::new();
    }
    payload
        .split(',')
        .map(|t| t.trim())
        .filter(|t| !t.is_empty())
        .collect()
}

fn scalar_codec_supported_rs(codec: &str) -> bool {
    matches!(
        codec,
        "bytes"
            | "zstring"
            | "lenstring8"
            | "lenstring16"
            | "lenstring32"
            | "lstring"
            | "int8"
            | "uint8"
            | "int16"
            | "uint16"
            | "uint16le"
            | "int32"
            | "uint32"
            | "int64"
            | "uint64"
            | "float32"
            | "formid"
            | "formid_array"
    ) || codec.starts_with("fixed_string:")
}

/// Parse a codec string into a crate::FieldCodec.  Returns None for unknown codecs.
fn field_codec_from_str(codec: &str) -> Option<crate::FieldCodec> {
    match codec {
        "bytes" => Some(crate::FieldCodec::Bytes),
        "zstring" => Some(crate::FieldCodec::ZString),
        "lenstring8" => Some(crate::FieldCodec::LenString8),
        "lenstring16" => Some(crate::FieldCodec::LenString16),
        "lenstring32" => Some(crate::FieldCodec::LenString32),
        "lstring" => Some(crate::FieldCodec::LString),
        "int8" => Some(crate::FieldCodec::Int8),
        "uint8" => Some(crate::FieldCodec::Uint8),
        "int16" => Some(crate::FieldCodec::Int16),
        "uint16" | "uint16le" => Some(crate::FieldCodec::Uint16),
        "int32" => Some(crate::FieldCodec::Int32),
        "uint32" => Some(crate::FieldCodec::Uint32),
        "formid" => Some(crate::FieldCodec::FormId),
        "float32" => Some(crate::FieldCodec::Float32),
        "int64" => Some(crate::FieldCodec::Int64),
        "uint64" => Some(crate::FieldCodec::Uint64),
        "formid_array" => Some(crate::FieldCodec::FormIdArray),
        s if s.starts_with("fixed_string:") => s
            .split_once(':')
            .and_then(|(_, n)| n.parse::<usize>().ok())
            .map(crate::FieldCodec::FixedString),
        _ => None,
    }
}

/// Build a DecodeSpec for a union of SchemaUnionVariantJson entries.
/// Returns None if any variant lacks a codec or has an unsupported layout.
///
/// `parse_partial` is forwarded into each variant's struct decoder so that
/// truncatable wbUnion-of-wbStruct subrecords (kind=parsed_with_raw_fallback)
/// accept short payloads — e.g. MOVT.SPED variant {1} declares 124 bytes but
/// real records carry only 44; xEdit tolerates the missing tail by filling
/// defaults, and our runtime mirrors that here.
fn decode_spec_for_union_rs(
    sig: &str,
    variants: &[SchemaUnionVariantJson],
    parse_partial: bool,
) -> Option<crate::DecodeSpec> {
    let mut rust_variants = Vec::with_capacity(variants.len());
    for variant in variants {
        let codec = variant.codec.as_deref()?;
        let inner_spec = decode_spec_for_fields_rs(sig, codec, &variant.fields, parse_partial)?;
        rust_variants.push(crate::Variant {
            name: variant.id.clone(),
            spec: Box::new(inner_spec),
            conditions: schema_conditions_to_rust(&variant.conditions),
        });
    }
    Some(crate::DecodeSpec::Union {
        variants: rust_variants,
    })
}

/// Build a DecodeSpec for a codec + fields list (struct/array_struct/scalar/empty).
/// Returns None for unsupported or too-complex layouts.
fn decode_spec_for_fields_rs(
    sig: &str,
    codec: &str,
    fields: &[SchemaFieldJson],
    parse_partial: bool,
) -> Option<crate::DecodeSpec> {
    let _ = sig; // kept for parity with native_decode_spec_for_fields signature
    if codec == "empty" {
        return Some(crate::DecodeSpec::Empty {
            empty_fields: fields.iter().map(|f| f.id.clone()).collect(),
        });
    }
    if scalar_codec_supported_rs(codec) {
        return field_codec_from_str(codec).map(|fc| crate::DecodeSpec::Scalar { codec: fc });
    }
    if !(codec.starts_with("struct:") || codec.starts_with("array_struct:")) {
        return None;
    }
    // Variable-offset struct: any field has a length-prefixed array or nested
    // sub-struct fields. The fixed `struct:tokens` layout cannot describe
    // these; route to the VariableStruct builder which walks fields by their
    // own declared shape rather than by parent codec tokens. The matching
    // encoder lives in plugin_runtime::encode_variable_struct_json.
    //
    // Only enabled for `struct:` codecs — `array_struct:` rows must remain
    // fixed-size.
    if codec.starts_with("struct:")
        && (struct_codec_has_variable_string_token(codec)
            || fields
                .iter()
                .any(|f| f.array.is_some() || !f.fields.is_empty() || !f.union_variants.is_empty()))
    {
        return decode_spec_for_var_fields_rs(sig, fields, parse_partial);
    }
    if fields
        .iter()
        .any(|f| f.array.is_some() || !f.fields.is_empty() || !f.union_variants.is_empty())
    {
        return None;
    }
    let tokens = struct_tokens_rs(codec);
    if tokens.is_empty() {
        return None;
    }
    let row_size: usize = tokens.iter().map(|t| token_width_rs(t).unwrap_or(0)).sum();
    let mut offset = 0usize;
    let mut token_index = 0usize;
    let mut segments: Vec<crate::Segment> = Vec::new();
    let mut tail_segment: Option<crate::TailSegment> = None;

    for (index, field) in fields.iter().enumerate() {
        // Skip padding tokens.
        while token_index < tokens.len() && tokens[token_index] == "x" {
            offset += token_width_rs(tokens[token_index]).unwrap_or(0);
            token_index += 1;
        }
        if field.kind == "empty" {
            continue;
        }
        if token_index >= tokens.len() {
            // Tail segment: the last field has no remaining `struct:tokens` to
            // bind against. Permitted for ``bytes`` (raw passthrough, matching
            // native_tail_segment_payload), and for variable-length string
            // codecs (``zstring`` / ``lstring`` / ``lenstring*``) so that
            // structs whose final field is an xEdit ``wbString`` round-trip
            // without raw_hex (e.g. STAG.TNAM = ``struct:I`` formid + trailing
            // zstring action; see decode_row_json's tail handling).
            let is_last = index + 1 == fields.len();
            let kind_supported = matches!(
                field.kind.as_str(),
                "bytes" | "zstring" | "lstring" | "lenstring8" | "lenstring16" | "lenstring32"
            );
            if is_last
                && token_index == tokens.len()
                && kind_supported
                && field.array.is_none()
                && field.fields.is_empty()
                && field.union_variants.is_empty()
            {
                tail_segment = Some(crate::TailSegment {
                    name: field.id.clone(),
                    offset,
                    kind: field.kind.clone(),
                });
                continue;
            }
            return None;
        }
        let token = tokens[token_index];
        let size = token_width_rs(token).unwrap_or(0);
        if size == 0 {
            return None;
        }
        let (nested_spec, codec_for_segment) = if !field.union_variants.is_empty() {
            // Inner union (sibling-discriminated, inside a parent struct).
            // Field-level unions don't carry their own kind annotation, so we
            // inherit the parent's parse_partial setting.
            let union_spec =
                decode_spec_for_union_rs(field.id.as_str(), &field.union_variants, parse_partial)?;
            (Some(Box::new(union_spec)), crate::FieldCodec::Uint32) // codec unused when nested_spec is Some
        } else {
            let scalar_codec_str = match field.kind.as_str() {
                "int8" | "uint8" | "int16" | "uint16" | "int32" | "uint32" | "int64" | "uint64"
                | "float32" | "formid" => field.kind.clone(),
                "enum" | "flags" => match token {
                    "b" => "int8".to_string(),
                    "B" => "uint8".to_string(),
                    "h" => "int16".to_string(),
                    "H" => "uint16".to_string(),
                    "i" => "int32".to_string(),
                    "I" => "uint32".to_string(),
                    "q" => "int64".to_string(),
                    "Q" => "uint64".to_string(),
                    _ => return None,
                },
                "fixed_string" => format!("fixed_string:{size}"),
                _ => return None,
            };
            let fc = field_codec_from_str(scalar_codec_str.as_str())?;
            (None, fc)
        };
        let presence_conditions = schema_conditions_to_rust(&field.presence_conditions);
        segments.push(crate::Segment {
            name: field.id.clone(),
            codec: codec_for_segment,
            offset,
            size,
            nested_spec,
            presence_conditions,
        });
        offset += size;
        token_index += 1;
    }
    // Skip trailing padding.
    while token_index < tokens.len() && tokens[token_index] == "x" {
        token_index += 1;
    }
    if token_index != tokens.len() {
        return None;
    }
    if codec.starts_with("array_struct:") {
        Some(crate::DecodeSpec::ArrayStruct { row_size, segments })
    } else {
        Some(crate::DecodeSpec::Struct {
            row_size,
            segments,
            tail_segment,
            parse_partial,
        })
    }
}

// ---------------------------------------------------------------------------
// Variable-offset struct builder.
// ---------------------------------------------------------------------------
//
// Used by NAVI, RACE, WTHR record types whose layout interleaves fixed scalars
// with length-prefixed arrays, nested structs, and sibling-discriminated
// unions. Walks the fields list left-to-right and emits one VarSegment per
// field; refuses (returns None) for any shape it cannot represent so the
// caller falls back to raw_hex.

fn scalar_codec_from_field_kind(kind: &str) -> Option<crate::FieldCodec> {
    match kind {
        "int8" => Some(crate::FieldCodec::Int8),
        "uint8" => Some(crate::FieldCodec::Uint8),
        "int16" => Some(crate::FieldCodec::Int16),
        "uint16" => Some(crate::FieldCodec::Uint16),
        "int32" => Some(crate::FieldCodec::Int32),
        "uint32" => Some(crate::FieldCodec::Uint32),
        "int64" => Some(crate::FieldCodec::Int64),
        "uint64" => Some(crate::FieldCodec::Uint64),
        "float32" => Some(crate::FieldCodec::Float32),
        "formid" => Some(crate::FieldCodec::FormId),
        _ => None,
    }
}

fn variable_string_codec_from_field_kind(kind: &str) -> Option<crate::FieldCodec> {
    match kind {
        "zstring" => Some(crate::FieldCodec::ZString),
        "lenstring8" => Some(crate::FieldCodec::LenString8),
        "lenstring16" => Some(crate::FieldCodec::LenString16),
        "lenstring32" => Some(crate::FieldCodec::LenString32),
        _ => None,
    }
}

fn struct_codec_has_variable_string_token(codec: &str) -> bool {
    codec.strip_prefix("struct:").is_some_and(|rest| {
        rest.split(',')
            .map(|token| token.trim())
            .any(|token| variable_string_codec_from_field_kind(token).is_some())
    })
}

fn condition_value_to_selector(value: &serde_json::Value) -> Option<crate::ConditionValue> {
    match value {
        serde_json::Value::Bool(b) => Some(crate::ConditionValue::Bool(*b)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Some(crate::ConditionValue::Int(i128::from(i)))
            } else if let Some(u) = n.as_u64() {
                Some(crate::ConditionValue::Int(i128::from(u)))
            } else {
                n.as_f64().map(crate::ConditionValue::Float)
            }
        }
        serde_json::Value::String(s) => Some(crate::ConditionValue::Str(s.clone())),
        _ => None,
    }
}

fn variant_to_selector_condition(
    variant: &SchemaUnionVariantJson,
) -> Option<crate::SelectorCondition> {
    if variant.conditions.is_empty() {
        return Some(crate::SelectorCondition::Always);
    }
    // Take the first condition as the discriminator; ignore additional
    // refinement conditions for now (NAVI/RACE/WTHR use single-condition
    // discriminators).
    let condition = &variant.conditions[0];
    let value = condition.value.as_ref()?;
    let cv = condition_value_to_selector(value)?;
    let operator = condition_default_operator(&condition.operator);
    match operator.as_str() {
        "eq" => Some(crate::SelectorCondition::Equals(cv)),
        "ne" => Some(crate::SelectorCondition::NotEquals(cv)),
        "lt" => Some(crate::SelectorCondition::LessThan(cv)),
        "lte" => Some(crate::SelectorCondition::LessThanOrEqual(cv)),
        "gt" => Some(crate::SelectorCondition::GreaterThan(cv)),
        "gte" => Some(crate::SelectorCondition::GreaterThanOrEqual(cv)),
        _ => None,
    }
}

fn condition_default_operator(op: &str) -> String {
    if op.is_empty() {
        "eq".to_string()
    } else {
        op.to_string()
    }
}

fn parse_array_element_size(element_codec: &str) -> Option<usize> {
    let payload = element_codec
        .strip_prefix("struct:")
        .or_else(|| element_codec.strip_prefix("array_struct:"))
        .unwrap_or(element_codec);
    if payload.is_empty() {
        return None;
    }
    let mut total = 0usize;
    for token in payload
        .split(',')
        .map(|t| t.trim())
        .filter(|t| !t.is_empty())
    {
        total += token_width_rs(token)?;
    }
    Some(total)
}

/// Returns whether a subrecord-codec string can
/// accommodate a payload of the given byte length. Used by the dispatcher to
/// disambiguate between multiple specs at the same (sig, scope) pair when the
/// occurrence-counter alone would mis-route a record (e.g. TERM has two SNAM
/// specs at top-level — Looping Sound formid (4 bytes) and Marker Parameters
/// array_struct (24-byte rows) — and a record may carry only the second).
///
/// * ``Some(true)``  — the codec definitely accepts ``payload_len``.
/// * ``Some(false)`` — the codec definitely rejects ``payload_len``.
/// * ``None``        — the codec is variable-length or unrecognised; the caller
///                     must not strict-reject this candidate by length alone.
pub fn codec_accepts_payload_length(codec: &str, payload_len: usize) -> Option<bool> {
    let fixed_size: Option<usize> = match codec {
        "uint8" | "int8" => Some(1),
        "uint16" | "uint16le" | "int16" => Some(2),
        "uint32" | "int32" | "float32" | "formid" => Some(4),
        "uint64" | "int64" => Some(8),
        s if s.starts_with("fixed_string:") => s["fixed_string:".len()..].parse::<usize>().ok(),
        _ => None,
    };
    if let Some(size) = fixed_size {
        return Some(payload_len == size);
    }
    if let Some(rest) = codec.strip_prefix("array_struct:") {
        let mut row_size = 0usize;
        for token in rest.split(',').map(|t| t.trim()).filter(|t| !t.is_empty()) {
            row_size += token_width_rs(token)?;
        }
        if row_size == 0 {
            return None;
        }
        return Some(payload_len % row_size == 0);
    }
    if let Some(rest) = codec.strip_prefix("struct:") {
        let mut total = 0usize;
        for token in rest.split(',').map(|t| t.trim()).filter(|t| !t.is_empty()) {
            total += token_width_rs(token)?;
        }
        if total == 0 {
            return None;
        }
        return Some(payload_len == total);
    }
    None
}

/// Map a single struct-token character ("I", "f", "B", "h", ...) to its
/// `FieldCodec`. Used by `build_array_element_spec` because schema array
/// element codecs may use the compact struct-token format rather than the
/// friendly codec name (e.g. NVMI edge_links has element_codec="I").
fn field_codec_from_struct_token(token: &str) -> Option<crate::FieldCodec> {
    match token {
        "b" => Some(crate::FieldCodec::Int8),
        "B" => Some(crate::FieldCodec::Uint8),
        "h" => Some(crate::FieldCodec::Int16),
        "H" => Some(crate::FieldCodec::Uint16),
        "i" => Some(crate::FieldCodec::Int32),
        "I" => Some(crate::FieldCodec::Uint32),
        "f" => Some(crate::FieldCodec::Float32),
        "q" => Some(crate::FieldCodec::Int64),
        "Q" => Some(crate::FieldCodec::Uint64),
        s if s.starts_with('s') => s[1..]
            .parse::<usize>()
            .ok()
            .map(crate::FieldCodec::FixedString),
        _ => None,
    }
}

fn build_array_element_spec(
    sig: &str,
    element_codec: &str,
    nested_fields: &[SchemaFieldJson],
) -> Option<(crate::DecodeSpec, usize)> {
    // Single-token element (e.g. "I" for formid array, "f" for float array).
    // Try struct-token form first since schema arrays use the compact format.
    if !element_codec.contains(',') && !element_codec.starts_with("struct:") {
        let codec = field_codec_from_struct_token(element_codec)
            .or_else(|| field_codec_from_str(element_codec))?;
        let size = scalar_codec_fixed_size(&codec)?;
        return Some((crate::DecodeSpec::Scalar { codec }, size));
    }
    // Multi-field row (e.g. "I,I" for door_links): build a fixed-size struct
    // by reusing the existing builder with codec "struct:<element_codec>".
    let row_size = parse_array_element_size(element_codec)?;
    let synthesized_codec = if element_codec.starts_with("struct:") {
        element_codec.to_string()
    } else {
        format!("struct:{element_codec}")
    };
    let inner = decode_spec_for_fields_rs(sig, &synthesized_codec, nested_fields, false)?;
    Some((inner, row_size))
}

fn build_var_segment(sig: &str, field: &SchemaFieldJson) -> Option<crate::VarSegment> {
    let presence_conditions = schema_conditions_to_rust(&field.presence_conditions);
    // Length-prefixed array.
    if let Some(array) = field.array.as_ref() {
        let count_codec_str = array.count_codec.as_deref()?;
        let count_codec = field_codec_from_str(count_codec_str)?;
        let element_codec_str = array.element_codec.as_deref()?;
        let (element_spec, element_size) =
            build_array_element_spec(sig, element_codec_str, &field.fields)?;
        return Some(crate::VarSegment::Array {
            name: field.id.clone(),
            count_codec,
            element_spec: Box::new(element_spec),
            element_size: Some(element_size),
            presence_conditions,
        });
    }
    // Sibling-discriminated union.
    if !field.union_variants.is_empty() {
        let selector_name = field
            .union_variants
            .iter()
            .filter_map(|v| v.conditions.first())
            .map(|c| c.field.clone())
            .next()?;
        let mut variants = Vec::with_capacity(field.union_variants.len());
        for variant in &field.union_variants {
            let condition = variant_to_selector_condition(variant)?;
            let codec = variant.codec.as_deref()?;
            let spec = if codec == "empty" {
                crate::DecodeSpec::Empty {
                    empty_fields: variant.fields.iter().map(|f| f.id.clone()).collect(),
                }
            } else {
                decode_spec_for_fields_rs(sig, codec, &variant.fields, false)?
            };
            variants.push(crate::VarUnionVariant {
                name: variant.id.clone(),
                spec,
                condition,
            });
        }
        return Some(crate::VarSegment::Union {
            name: field.id.clone(),
            variants,
            selector: crate::SelectorRef::Sibling(selector_name),
            presence_conditions,
        });
    }
    // Nested struct without an array (rare): not supported here; the parent
    // `struct:` codec would need to describe its layout.
    if !field.fields.is_empty() {
        if !presence_conditions.is_empty() {
            return Some(crate::VarSegment::UnsupportedConditional {
                name: field.id.clone(),
                presence_conditions,
            });
        }
        return None;
    }
    if let Some(codec) = variable_string_codec_from_field_kind(field.kind.as_str()) {
        return Some(crate::VarSegment::VariableString {
            name: field.id.clone(),
            codec,
            presence_conditions,
        });
    }
    // Plain scalar.
    let Some(codec) = scalar_codec_from_field_kind(&field.kind) else {
        if !presence_conditions.is_empty() {
            return Some(crate::VarSegment::UnsupportedConditional {
                name: field.id.clone(),
                presence_conditions,
            });
        }
        return None;
    };
    Some(crate::VarSegment::Scalar {
        name: field.id.clone(),
        codec,
        presence_conditions,
    })
}

/// Build a `DecodeSpec::VariableStruct` from a list of fields whose shapes are
/// not expressible via a single fixed `struct:tokens` codec.
fn decode_spec_for_var_fields_rs(
    sig: &str,
    fields: &[SchemaFieldJson],
    parse_partial: bool,
) -> Option<crate::DecodeSpec> {
    let mut segments = Vec::with_capacity(fields.len());
    for field in fields {
        if field.kind == "empty" {
            continue;
        }
        let segment = build_var_segment(sig, field)?;
        segments.push(segment);
    }
    Some(crate::DecodeSpec::VariableStruct {
        segments,
        parse_partial,
    })
}

/// Convert a `SchemaSubrecordJson` directly into a `crate::DecodeSpec` (Step 1).
///
/// Returns `None` when the subrecord cannot be decoded natively:
///   * `kind == "raw"` — caller must use raw-hex fallback.
///   * `vmad` layout — VMAD has bespoke non-trivial encoding.
///   * Any unsupported codec / nested array field.
pub fn schema_subrecord_to_decode_spec(
    spec: &SchemaSubrecordJson,
    _schema: &CompiledSchema,
) -> Option<crate::DecodeSpec> {
    if spec.kind == "raw" {
        return None;
    }
    // `custom_codec` delegates parse/write to an external Rust module named
    // by `spec.codec` (e.g. `esp_authoring_core::nvnm`). The struct decoder
    // doesn't know how to talk to it, so we fall through to raw-only on the
    // YAML side — the bytes still roundtrip verbatim through ParsedSubrecord.
    if spec.kind == "custom_codec" {
        return None;
    }
    if runtime_layout_for_subrecord_schema(spec) == Some("vmad") {
        return None;
    }
    // `parse_partial` lets the struct decoder accept short data and emit only
    // the fields that fit. Hybrid kind (`parsed_with_raw_fallback`) keeps
    // raw_hex on the YAML so the encoder rebuilds the original bytes via
    // raw_hex fallback (preservation_mode=hybrid in plugin_runtime.rs).
    //
    // For wbUnion-of-wbStruct subrecords, the probe sets the parent kind to
    // `parsed_with_raw_fallback` so each variant struct decodes leniently —
    // matches xEdit's runtime behavior of filling missing trailing fields with
    // defaults (e.g. MOVT.SPED).
    let parse_partial = spec.kind == "parsed_with_raw_fallback";
    if !spec.union_variants.is_empty() {
        return decode_spec_for_union_rs(spec.id.as_str(), &spec.union_variants, parse_partial);
    }
    let codec = spec.codec.as_deref()?;
    decode_spec_for_fields_rs(spec.id.as_str(), codec, &spec.fields, parse_partial)
}

// -------------------------------------------------------------------------
// Step 2 — compact_subrecord_to_json
//
// GIL-free decode + compact, returning serde_json::Value at every branch.
// -------------------------------------------------------------------------

// --- Low-level scalar decoders → serde_json::Value ---------------------------

fn decode_cp1252_json(data: &[u8]) -> String {
    let (decoded, _, _) = encoding_rs::WINDOWS_1252.decode(data);
    decoded.into_owned()
}

fn trim_nuls_json(data: &[u8]) -> &[u8] {
    let mut end = data.len();
    while end > 0 && data[end - 1] == 0 {
        end -= 1;
    }
    &data[..end]
}

fn decode_scalar_codec_json(codec: &crate::FieldCodec, data: &[u8]) -> serde_json::Value {
    use crate::FieldCodec::*;
    match codec {
        Bytes => serde_json::Value::String(hex::encode_upper(data)),
        ZString => serde_json::Value::String(decode_cp1252_json(trim_nuls_json(data))),
        LenString8 => decode_lenstring_json(data, 1),
        LenString16 => decode_lenstring_json(data, 2),
        LenString32 => decode_lenstring_json(data, 4),
        LString => {
            if data.len() == 4 {
                let v = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
                serde_json::Value::Number(v.into())
            } else {
                serde_json::Value::String(decode_cp1252_json(trim_nuls_json(data)))
            }
        }
        Int8 if data.len() == 1 => serde_json::json!(data[0] as i8),
        Uint8 if data.len() == 1 => serde_json::json!(data[0]),
        Int16 if data.len() == 2 => serde_json::json!(i16::from_le_bytes([data[0], data[1]])),
        Uint16 if data.len() == 2 => serde_json::json!(u16::from_le_bytes([data[0], data[1]])),
        Int32 if data.len() == 4 => {
            serde_json::json!(i32::from_le_bytes([data[0], data[1], data[2], data[3]]))
        }
        Uint32 | FormId if data.len() == 4 => {
            serde_json::json!(u32::from_le_bytes([data[0], data[1], data[2], data[3]]))
        }
        Float32 if data.len() == 4 => {
            let f = f32::from_le_bytes([data[0], data[1], data[2], data[3]]);
            if let Some(n) = serde_json::Number::from_f64(f as f64) {
                serde_json::Value::Number(n)
            } else {
                serde_json::Value::Null
            }
        }
        Int64 if data.len() == 8 => serde_json::json!(i64::from_le_bytes([
            data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7]
        ])),
        Uint64 if data.len() == 8 => serde_json::json!(u64::from_le_bytes([
            data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7]
        ])),
        FormIdArray => {
            if data.len() % 4 != 0 {
                return serde_json::Value::Null;
            }
            serde_json::Value::Array(
                data.chunks_exact(4)
                    .map(|c| serde_json::json!(u32::from_le_bytes([c[0], c[1], c[2], c[3]])))
                    .collect(),
            )
        }
        FixedString(size) if data.len() == *size => {
            serde_json::Value::String(decode_cp1252_json(trim_nuls_json(data)))
        }
        // Wrong-length / unmatched guard arms → raw hex.
        _ => serde_json::Value::String(hex::encode_upper(data)),
    }
}

fn scalar_string_decode_is_raw_fallback(
    spec: &crate::DecodeSpec,
    decoded: &serde_json::Value,
) -> bool {
    if !decoded.is_string() {
        return false;
    }
    let crate::DecodeSpec::Scalar { codec } = spec else {
        return false;
    };
    matches!(
        codec,
        crate::FieldCodec::Int8
            | crate::FieldCodec::Uint8
            | crate::FieldCodec::Int16
            | crate::FieldCodec::Uint16
            | crate::FieldCodec::Int32
            | crate::FieldCodec::Uint32
            | crate::FieldCodec::FormId
            | crate::FieldCodec::Float32
            | crate::FieldCodec::Int64
            | crate::FieldCodec::Uint64
            | crate::FieldCodec::FormIdArray
    )
}

fn decode_lenstring_json(data: &[u8], prefix_size: usize) -> serde_json::Value {
    if data.len() < prefix_size {
        return serde_json::Value::Null;
    }
    let payload_size = match prefix_size {
        1 => data[0] as usize,
        2 => u16::from_le_bytes([data[0], data[1]]) as usize,
        4 => u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize,
        _ => return serde_json::Value::Null,
    };
    let start = prefix_size;
    let end = start + payload_size;
    if end > data.len() {
        return serde_json::Value::Null;
    }
    serde_json::Value::String(decode_cp1252_json(trim_nuls_json(&data[start..end])))
}

// --- Condition evaluation (pure Rust context map) ----------------------------

fn json_value_as_i128(v: &serde_json::Value) -> Option<i128> {
    match v {
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Some(i as i128)
            } else if let Some(u) = n.as_u64() {
                Some(u as i128)
            } else {
                None
            }
        }
        serde_json::Value::Bool(b) => Some(if *b { 1 } else { 0 }),
        _ => None,
    }
}

fn condition_value_matches(actual: &serde_json::Value, cv: &crate::ConditionValue) -> bool {
    match cv {
        crate::ConditionValue::Int(i) => json_value_as_i128(actual) == Some(*i),
        crate::ConditionValue::Float(f) => match actual {
            serde_json::Value::Number(n) => n.as_f64().map(|a| a == *f).unwrap_or(false),
            _ => false,
        },
        crate::ConditionValue::Str(s) => matches!(actual, serde_json::Value::String(a) if a == s),
        crate::ConditionValue::Bool(b) => matches!(actual, serde_json::Value::Bool(a) if a == b),
    }
}

fn conditions_match_json(
    context: &std::collections::HashMap<String, serde_json::Value>,
    conditions: &[crate::Condition],
) -> bool {
    for condition in conditions {
        let actual = context
            .get(condition.field.as_str())
            .unwrap_or(&serde_json::Value::Null);
        let matched = match condition.operator.as_str() {
            "eq" => {
                if let Some(cv) = condition.value.as_ref() {
                    condition_value_matches(actual, cv)
                } else {
                    actual.is_null()
                }
            }
            "ne" => {
                if let Some(cv) = condition.value.as_ref() {
                    !condition_value_matches(actual, cv)
                } else {
                    !actual.is_null()
                }
            }
            "in" => {
                if !condition.values.is_empty() {
                    condition
                        .values
                        .iter()
                        .any(|cv| condition_value_matches(actual, cv))
                } else if let Some(cv) = condition.value.as_ref() {
                    condition_value_matches(actual, cv)
                } else {
                    false
                }
            }
            "not_in" => {
                if !condition.values.is_empty() {
                    !condition
                        .values
                        .iter()
                        .any(|cv| condition_value_matches(actual, cv))
                } else if let Some(cv) = condition.value.as_ref() {
                    !condition_value_matches(actual, cv)
                } else {
                    true
                }
            }
            "truthy" => {
                !actual.is_null()
                    && actual != &serde_json::Value::Bool(false)
                    && actual != &serde_json::Value::Number(0u64.into())
            }
            "falsy" => {
                actual.is_null()
                    || actual == &serde_json::Value::Bool(false)
                    || actual == &serde_json::Value::Number(0u64.into())
            }
            "lt" => {
                if let (Some(a), Some(cv)) = (json_value_as_i128(actual), condition.value.as_ref())
                {
                    match cv {
                        crate::ConditionValue::Int(e) => a < *e,
                        _ => false,
                    }
                } else {
                    false
                }
            }
            "lte" => {
                if let (Some(a), Some(cv)) = (json_value_as_i128(actual), condition.value.as_ref())
                {
                    match cv {
                        crate::ConditionValue::Int(e) => a <= *e,
                        _ => false,
                    }
                } else {
                    false
                }
            }
            "gt" => {
                if let (Some(a), Some(cv)) = (json_value_as_i128(actual), condition.value.as_ref())
                {
                    match cv {
                        crate::ConditionValue::Int(e) => a > *e,
                        _ => false,
                    }
                } else {
                    false
                }
            }
            "gte" => {
                if let (Some(a), Some(cv)) = (json_value_as_i128(actual), condition.value.as_ref())
                {
                    match cv {
                        crate::ConditionValue::Int(e) => a >= *e,
                        _ => false,
                    }
                } else {
                    false
                }
            }
            "bit_set" => {
                if let (Some(a), Some(cv)) = (json_value_as_i128(actual), condition.value.as_ref())
                {
                    let e = match cv {
                        crate::ConditionValue::Int(i) => *i,
                        crate::ConditionValue::Bool(b) => {
                            if *b {
                                1
                            } else {
                                0
                            }
                        }
                        _ => return false,
                    };
                    (a & e) == e
                } else {
                    false
                }
            }
            "bit_unset" => {
                if let (Some(a), Some(cv)) = (json_value_as_i128(actual), condition.value.as_ref())
                {
                    let e = match cv {
                        crate::ConditionValue::Int(i) => *i,
                        crate::ConditionValue::Bool(b) => {
                            if *b {
                                1
                            } else {
                                0
                            }
                        }
                        _ => return false,
                    };
                    (a & e) == 0
                } else {
                    false
                }
            }
            // Default: equality (same as "eq").
            _ => {
                if let Some(cv) = condition.value.as_ref() {
                    condition_value_matches(actual, cv)
                } else {
                    actual.is_null()
                }
            }
        };
        if !matched {
            return false;
        }
    }
    true
}

// --- GIL-free decode functions -----------------------------------------------

/// Filter `segments` by presence_conditions evaluated against `context`, and
/// recompute offsets so the filtered list packs back-to-back. Returns the
/// effective row size (sum of present segment sizes) and the filtered list.
///
/// Used by `DecodeSpec::Struct` / `DecodeSpec::ArrayStruct` to handle
/// `wbFromVersion`-style conditional fields (e.g. ARMO/WEAP DAMA curve_table
/// only present in records with form_version >= 152).
fn effective_row_layout(
    segments: &[crate::Segment],
    context: &std::collections::HashMap<String, serde_json::Value>,
) -> (usize, Vec<crate::Segment>) {
    let mut filtered: Vec<crate::Segment> = Vec::with_capacity(segments.len());
    let mut offset = 0usize;
    for seg in segments {
        if !seg.presence_conditions.is_empty()
            && !conditions_match_json(context, &seg.presence_conditions)
        {
            continue;
        }
        let mut s = seg.clone();
        s.offset = offset;
        offset += s.size;
        filtered.push(s);
    }
    (offset, filtered)
}

fn segments_have_conditions(segments: &[crate::Segment]) -> bool {
    segments.iter().any(|s| !s.presence_conditions.is_empty())
}

fn decode_row_json(
    row: &[u8],
    segments: &[crate::Segment],
    tail_segment: Option<&crate::TailSegment>,
    context: &std::collections::HashMap<String, serde_json::Value>,
) -> Option<serde_json::Value> {
    let mut map = serde_json::Map::new();
    for segment in segments {
        let end = segment.offset + segment.size;
        if end > row.len() {
            // Variable-length struct (parse_partial): the input doesn't cover
            // every declared segment, so stop early and emit only what fits.
            // Round-trip safety relies on kind=parsed_with_raw_fallback
            // so the encoder falls back to raw_hex.
            break;
        }
        let slice = &row[segment.offset..end];
        let value = if let Some(nested) = segment.nested_spec.as_ref() {
            let mut seg_ctx = context.clone();
            for (k, v) in &map {
                seg_ctx.entry(k.clone()).or_insert_with(|| v.clone());
            }
            decode_subrecord_json(nested, slice, &seg_ctx)?
        } else {
            decode_scalar_codec_json(&segment.codec, slice)
        };
        map.insert(segment.name.clone(), value);
    }
    if let Some(tail) = tail_segment {
        if tail.offset <= row.len() {
            let tail_bytes = &row[tail.offset..];
            let value = match field_codec_from_str(tail.kind.as_str()) {
                Some(codec) => {
                    // Round-trip safety: only emit the decoded form when the
                    // encoder will reproduce the original bytes exactly. For
                    // zstring this means a single trailing NUL with no other
                    // embedded NULs (so trim_nuls + re-append NUL is identity).
                    // Other codecs return the bytes verbatim or fall through.
                    if !tail_codec_roundtrip_safe(&codec, tail_bytes) {
                        return None;
                    }
                    decode_scalar_codec_json(&codec, tail_bytes)
                }
                None => serde_json::Value::String(hex::encode_upper(tail_bytes)),
            };
            map.insert(tail.name.clone(), value);
        }
    }
    Some(serde_json::Value::Object(map))
}

/// Whether encoding then re-encoding ``tail`` under ``codec`` would reproduce
/// the original bytes exactly. Used by the struct-tail decoder to refuse the
/// decoded form when round-trip would drift, so the caller falls back to
/// raw_hex (preserving byte-exact YAML round-trip).
fn tail_codec_roundtrip_safe(codec: &crate::FieldCodec, tail: &[u8]) -> bool {
    match codec {
        crate::FieldCodec::Bytes => true,
        crate::FieldCodec::ZString => {
            // Single trailing NUL terminator; no embedded or extra NULs (the
            // encoder emits exactly one NUL at the end).
            !tail.is_empty() && *tail.last().unwrap() == 0 && !tail[..tail.len() - 1].contains(&0)
        }
        crate::FieldCodec::LenString8
        | crate::FieldCodec::LenString16
        | crate::FieldCodec::LenString32 => {
            // Length-prefix + payload must match. The encoder rebuilds prefix
            // from the decoded string's encoded length, so we accept iff the
            // declared prefix equals the actual payload length.
            let prefix_size = match codec {
                crate::FieldCodec::LenString8 => 1,
                crate::FieldCodec::LenString16 => 2,
                _ => 4,
            };
            if tail.len() < prefix_size {
                return false;
            }
            let declared = match prefix_size {
                1 => tail[0] as usize,
                2 => u16::from_le_bytes([tail[0], tail[1]]) as usize,
                _ => u32::from_le_bytes([tail[0], tail[1], tail[2], tail[3]]) as usize,
            };
            declared + prefix_size == tail.len()
        }
        // LString with 4-byte localized id is a fixed 4-byte payload. With a
        // string body it round-trips through encode_cp1252(.., true) — same
        // shape as ZString. Accept the safe subset.
        crate::FieldCodec::LString => {
            tail.len() == 4
                || (!tail.is_empty()
                    && *tail.last().unwrap() == 0
                    && !tail[..tail.len() - 1].contains(&0))
        }
        // Numeric and form-id codecs aren't currently used as struct tails;
        // refuse to be safe.
        _ => false,
    }
}

fn decode_subrecord_json(
    spec: &crate::DecodeSpec,
    data: &[u8],
    context: &std::collections::HashMap<String, serde_json::Value>,
) -> Option<serde_json::Value> {
    match spec {
        crate::DecodeSpec::Empty { empty_fields } => {
            let mut map = serde_json::Map::new();
            for field in empty_fields {
                map.insert(field.clone(), serde_json::Value::Null);
            }
            Some(serde_json::Value::Object(map))
        }
        crate::DecodeSpec::Scalar { codec } => Some(decode_scalar_codec_json(codec, data)),
        crate::DecodeSpec::Struct {
            row_size,
            segments,
            tail_segment,
            parse_partial,
        } => {
            let has_tail = tail_segment.is_some();
            // If any segment is conditional on context (e.g. wbFromVersion),
            // recompute the effective row layout. Tail segments are not
            // compatible with conditional segments — fall back to the
            // declared row size in that case.
            let (effective_row_size, effective_segments_owned) =
                if !has_tail && segments_have_conditions(segments) {
                    let (sz, filtered) = effective_row_layout(segments, context);
                    (sz, Some(filtered))
                } else {
                    (*row_size, None)
                };
            let effective_segments: &[crate::Segment] = effective_segments_owned
                .as_deref()
                .unwrap_or(segments.as_slice());
            if has_tail {
                if !*parse_partial && data.len() < effective_row_size {
                    return None;
                }
            } else if *parse_partial {
                // Variable-length struct: accept any data length up to
                // effective_row_size. decode_row_json drops segments that
                // don't fit.
                if data.len() > effective_row_size {
                    return None;
                }
            } else if data.len() != effective_row_size {
                return None;
            }
            decode_row_json(data, effective_segments, tail_segment.as_ref(), context)
        }
        crate::DecodeSpec::ArrayStruct { row_size, segments } => {
            // If any segment is conditional on context (e.g. wbFromVersion
            // for ARMO/WEAP DAMA curve_table tail), filter segments by
            // context-matched presence_conditions and recompute the
            // effective row size before chunking.
            let (effective_row_size, effective_segments_owned) =
                if segments_have_conditions(segments) {
                    let (sz, filtered) = effective_row_layout(segments, context);
                    (sz, Some(filtered))
                } else {
                    (*row_size, None)
                };
            let effective_segments: &[crate::Segment] = effective_segments_owned
                .as_deref()
                .unwrap_or(segments.as_slice());
            if effective_row_size == 0 || data.len() % effective_row_size != 0 {
                return None;
            }
            let rows: Option<Vec<serde_json::Value>> = data
                .chunks_exact(effective_row_size)
                .map(|row| decode_row_json(row, effective_segments, None, context))
                .collect();
            rows.map(serde_json::Value::Array)
        }
        crate::DecodeSpec::Union { variants } => decode_union_json(data, variants, context),
        crate::DecodeSpec::VariableStruct {
            segments,
            parse_partial,
        } => decode_variable_struct_json(data, segments, *parse_partial, context),
    }
}

/// Decode a `DecodeSpec::VariableStruct`. Walks segments left-to-right; each
/// segment may consume a variable number of bytes (length-prefixed arrays,
/// sibling-discriminated unions). Returns `None` on any decode failure so the
/// caller can fall back to raw_hex.
fn decode_variable_struct_json(
    data: &[u8],
    segments: &[crate::VarSegment],
    parse_partial: bool,
    context: &std::collections::HashMap<String, serde_json::Value>,
) -> Option<serde_json::Value> {
    let (value, consumed) = decode_variable_struct_with_length(data, segments, context)?;
    if !parse_partial && consumed != data.len() {
        return None;
    }
    Some(value)
}

/// Decode a VariableStruct and report total bytes consumed. Used both as the
/// top-level decode entry and as a length-aware decoder for unions whose
/// variants are themselves VariableStructs.
fn decode_variable_struct_with_length(
    data: &[u8],
    segments: &[crate::VarSegment],
    context: &std::collections::HashMap<String, serde_json::Value>,
) -> Option<(serde_json::Value, usize)> {
    let mut offset = 0usize;
    let mut output = serde_json::Map::new();
    let mut cond_ctx = context.clone();
    for segment in segments {
        let presence_conditions = segment.presence_conditions();
        if !presence_conditions.is_empty() && !conditions_match_json(&cond_ctx, presence_conditions)
        {
            continue;
        }
        let (value, consumed) = decode_var_segment_json(segment, &data[offset..], &cond_ctx)?;
        let name = segment.name().to_string();
        cond_ctx.insert(name.clone(), value.clone());
        output.insert(name, value);
        offset += consumed;
    }
    Some((serde_json::Value::Object(output), offset))
}

fn decode_var_segment_json(
    segment: &crate::VarSegment,
    bytes: &[u8],
    cond_ctx: &std::collections::HashMap<String, serde_json::Value>,
) -> Option<(serde_json::Value, usize)> {
    match segment {
        crate::VarSegment::Scalar { codec, .. } => {
            let size = scalar_codec_fixed_size(codec)?;
            if bytes.len() < size {
                return None;
            }
            let value = decode_scalar_codec_json(codec, &bytes[..size]);
            Some((value, size))
        }
        crate::VarSegment::VariableString { codec, .. } => {
            decode_variable_string_segment_json(codec, bytes)
        }
        crate::VarSegment::Array {
            count_codec,
            element_spec,
            element_size,
            ..
        } => {
            let count_size = scalar_codec_fixed_size(count_codec)?;
            if bytes.len() < count_size {
                return None;
            }
            let count_val = decode_scalar_codec_json(count_codec, &bytes[..count_size]);
            let count = count_val.as_u64()? as usize;
            let consumed = (*element_size)?;
            if consumed == 0 {
                if count != 0 {
                    return None;
                }
                return Some((serde_json::Value::Array(Vec::new()), count_size));
            }
            let remaining = bytes.len().saturating_sub(count_size);
            if count > remaining / consumed {
                return None;
            }
            let mut offset = count_size;
            let mut elements = Vec::with_capacity(count);
            for _ in 0..count {
                if bytes.len() < offset + consumed {
                    return None;
                }
                let elem = decode_subrecord_json(
                    element_spec,
                    &bytes[offset..offset + consumed],
                    cond_ctx,
                )?;
                elements.push(elem);
                offset += consumed;
            }
            Some((serde_json::Value::Array(elements), offset))
        }
        crate::VarSegment::NestedStruct { spec, .. } => {
            // Nested struct must be decodable from the remaining bytes. The
            // spec is responsible for declaring its own length expectation.
            // For fixed-size structs we delegate to decode_subrecord_json with
            // exactly row_size bytes; for nested VariableStructs we recurse.
            match spec.as_ref() {
                crate::DecodeSpec::Struct { row_size, .. } => {
                    if bytes.len() < *row_size {
                        return None;
                    }
                    let value = decode_subrecord_json(spec, &bytes[..*row_size], cond_ctx)?;
                    Some((value, *row_size))
                }
                crate::DecodeSpec::Scalar { codec } => {
                    let size = scalar_codec_fixed_size(codec)?;
                    if bytes.len() < size {
                        return None;
                    }
                    Some((decode_scalar_codec_json(codec, &bytes[..size]), size))
                }
                _ => None,
            }
        }
        crate::VarSegment::Union {
            variants, selector, ..
        } => {
            let selector_val = match selector {
                crate::SelectorRef::Sibling(name) => cond_ctx.get(name)?,
            };
            for variant in variants {
                if !var_selector_matches(&variant.condition, selector_val) {
                    continue;
                }
                let (consumed, decoded) =
                    decode_variant_with_length(&variant.spec, bytes, cond_ctx)?;
                let mut map = serde_json::Map::new();
                map.insert(
                    "variant".to_string(),
                    serde_json::Value::String(variant.name.clone()),
                );
                map.insert("value".to_string(), decoded);
                return Some((serde_json::Value::Object(map), consumed));
            }
            None
        }
        crate::VarSegment::UnsupportedConditional { .. } => None,
    }
}

fn decode_variable_string_segment_json(
    codec: &crate::FieldCodec,
    bytes: &[u8],
) -> Option<(serde_json::Value, usize)> {
    match codec {
        crate::FieldCodec::ZString => {
            let consumed = bytes
                .iter()
                .position(|byte| *byte == 0)
                .map(|index| index + 1)
                .unwrap_or(bytes.len());
            let text_end = if consumed > 0 && bytes.get(consumed - 1) == Some(&0) {
                consumed - 1
            } else {
                consumed
            };
            Some((
                serde_json::Value::String(decode_cp1252_json(&bytes[..text_end])),
                consumed,
            ))
        }
        crate::FieldCodec::LenString8
        | crate::FieldCodec::LenString16
        | crate::FieldCodec::LenString32 => {
            let prefix_size = match codec {
                crate::FieldCodec::LenString8 => 1,
                crate::FieldCodec::LenString16 => 2,
                crate::FieldCodec::LenString32 => 4,
                _ => unreachable!(),
            };
            if bytes.len() < prefix_size {
                return None;
            }
            let payload_size = match prefix_size {
                1 => bytes[0] as usize,
                2 => u16::from_le_bytes([bytes[0], bytes[1]]) as usize,
                4 => u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize,
                _ => unreachable!(),
            };
            let consumed = prefix_size + payload_size;
            if bytes.len() < consumed {
                return None;
            }
            Some((
                decode_lenstring_json(&bytes[..consumed], prefix_size),
                consumed,
            ))
        }
        _ => None,
    }
}

fn decode_variant_with_length(
    spec: &crate::DecodeSpec,
    bytes: &[u8],
    cond_ctx: &std::collections::HashMap<String, serde_json::Value>,
) -> Option<(usize, serde_json::Value)> {
    match spec {
        crate::DecodeSpec::Empty { empty_fields } => {
            let mut map = serde_json::Map::new();
            for field in empty_fields {
                map.insert(field.clone(), serde_json::Value::Null);
            }
            Some((0, serde_json::Value::Object(map)))
        }
        crate::DecodeSpec::Scalar { codec } => {
            let size = scalar_codec_fixed_size(codec)?;
            if bytes.len() < size {
                return None;
            }
            Some((size, decode_scalar_codec_json(codec, &bytes[..size])))
        }
        crate::DecodeSpec::Struct {
            row_size,
            segments: _,
            tail_segment,
            parse_partial: _,
        } => {
            if tail_segment.is_some() {
                return None;
            }
            if bytes.len() < *row_size {
                return None;
            }
            let value = decode_subrecord_json(spec, &bytes[..*row_size], cond_ctx)?;
            Some((*row_size, value))
        }
        crate::DecodeSpec::VariableStruct { segments, .. } => {
            decode_variable_struct_with_length(bytes, segments, cond_ctx)
                .map(|(value, consumed)| (consumed, value))
        }
        crate::DecodeSpec::ArrayStruct { .. } | crate::DecodeSpec::Union { .. } => None,
    }
}

fn var_selector_matches(condition: &crate::SelectorCondition, actual: &serde_json::Value) -> bool {
    match condition {
        crate::SelectorCondition::Always => true,
        crate::SelectorCondition::Equals(expected) => {
            json_matches_condition_value(actual, expected)
        }
        crate::SelectorCondition::NotEquals(expected) => {
            !json_matches_condition_value(actual, expected)
        }
        crate::SelectorCondition::LessThan(expected) => {
            json_compare_condition_value(actual, expected).is_some_and(|ordering| ordering.is_lt())
        }
        crate::SelectorCondition::LessThanOrEqual(expected) => {
            json_compare_condition_value(actual, expected).is_some_and(|ordering| !ordering.is_gt())
        }
        crate::SelectorCondition::GreaterThan(expected) => {
            json_compare_condition_value(actual, expected).is_some_and(|ordering| ordering.is_gt())
        }
        crate::SelectorCondition::GreaterThanOrEqual(expected) => {
            json_compare_condition_value(actual, expected).is_some_and(|ordering| !ordering.is_lt())
        }
    }
}

fn json_compare_condition_value(
    actual: &serde_json::Value,
    expected: &crate::ConditionValue,
) -> Option<std::cmp::Ordering> {
    match expected {
        crate::ConditionValue::Int(expected) => {
            json_value_as_i128(actual).map(|actual| actual.cmp(expected))
        }
        crate::ConditionValue::Float(expected) => actual
            .as_f64()
            .and_then(|actual| actual.partial_cmp(expected)),
        crate::ConditionValue::Bool(expected) => {
            json_value_as_i128(actual).map(|actual| actual.cmp(&i128::from(*expected as i8)))
        }
        crate::ConditionValue::Str(expected) => {
            actual.as_str().map(|actual| actual.cmp(expected.as_str()))
        }
    }
}

fn json_matches_condition_value(
    actual: &serde_json::Value,
    expected: &crate::ConditionValue,
) -> bool {
    match (actual, expected) {
        (serde_json::Value::Bool(a), crate::ConditionValue::Bool(b)) => a == b,
        (serde_json::Value::Number(a), crate::ConditionValue::Int(b)) => {
            a.as_i64().map(|v| i128::from(v) == *b).unwrap_or(false)
                || a.as_u64().map(|v| i128::from(v) == *b).unwrap_or(false)
        }
        (serde_json::Value::Number(a), crate::ConditionValue::Float(b)) => a
            .as_f64()
            .map(|v| (v - *b).abs() <= f64::EPSILON)
            .unwrap_or(false),
        (serde_json::Value::String(a), crate::ConditionValue::Str(b)) => a == b,
        (serde_json::Value::Number(a), crate::ConditionValue::Bool(b)) => {
            a.as_i64().map(|v| (v != 0) == *b).unwrap_or(false)
        }
        _ => false,
    }
}

fn scalar_codec_fixed_size(codec: &crate::FieldCodec) -> Option<usize> {
    match codec {
        crate::FieldCodec::Int8 | crate::FieldCodec::Uint8 => Some(1),
        crate::FieldCodec::Int16 | crate::FieldCodec::Uint16 => Some(2),
        crate::FieldCodec::Int32
        | crate::FieldCodec::Uint32
        | crate::FieldCodec::FormId
        | crate::FieldCodec::Float32 => Some(4),
        crate::FieldCodec::Int64 | crate::FieldCodec::Uint64 => Some(8),
        crate::FieldCodec::FixedString(size) => Some(*size),
        // Variable-length codecs cannot be embedded in a VariableStruct without
        // a sibling length-prefix; return None to refuse decoding.
        crate::FieldCodec::Bytes
        | crate::FieldCodec::ZString
        | crate::FieldCodec::LenString8
        | crate::FieldCodec::LenString16
        | crate::FieldCodec::LenString32
        | crate::FieldCodec::LString
        | crate::FieldCodec::FormIdArray => None,
    }
}

fn decode_union_json(
    data: &[u8],
    variants: &[crate::Variant],
    context: &std::collections::HashMap<String, serde_json::Value>,
) -> Option<serde_json::Value> {
    for variant in variants {
        // Variable-width unions (SNDR.BNAM, AECH.DNAM, etc.) have variants
        // whose codecs disagree on row size; a wrong-size decode for one
        // variant must not abort the whole union — skip and try the next.
        let Some(decoded) = decode_subrecord_json(&variant.spec, data, context) else {
            continue;
        };
        // Build condition context: base context + decoded fields.
        let mut cond_ctx = context.clone();
        if let serde_json::Value::Object(ref fields) = decoded {
            for (k, v) in fields {
                cond_ctx.entry(k.clone()).or_insert_with(|| v.clone());
            }
        }
        if !conditions_match_json(&cond_ctx, &variant.conditions) {
            continue;
        }
        let mut map = serde_json::Map::new();
        map.insert(
            "variant".to_string(),
            serde_json::Value::String(variant.name.clone()),
        );
        map.insert("value".to_string(), decoded);
        return Some(serde_json::Value::Object(map));
    }
    None
}

// --- Payload helpers → serde_json::Value -------------------------------------

/// Dispatch a `custom_codec` subrecord to its structured-yaml emitter.
///
/// Returns a payload of shape `{ ...structured fields..., raw_hex,
/// semantic_type? }` so downstream consumers see the structured form *and*
/// keep the raw_hex fallback for byte-exact roundtrip on import. If the spec's
/// codec name isn't recognised or the bytes fail to parse, returns `None` and
/// callers fall through to raw-only.
fn custom_codec_payload_json(
    sub_spec: &SchemaSubrecordJson,
    data: &[u8],
    raw_hex: &str,
    semantic_type: Option<&str>,
) -> Option<serde_json::Value> {
    let codec = sub_spec.codec.as_deref()?;
    let structured: serde_json::Value = match codec {
        "esp_authoring_core::nvnm" => {
            let payload = crate::nvnm::parse_nvnm(data).ok()?;
            crate::nvnm::nvnm_to_yaml(&payload)
        }
        "esp_authoring_core::land::heightmap" => match sub_spec.id.as_str() {
            "VHGT" => {
                let parsed = crate::land::heightmap::parse_heightmap(data).ok()?;
                crate::land::heightmap::heightmap_to_yaml(&parsed)
            }
            "VNML" => {
                let parsed = crate::land::heightmap::parse_vertex_normals(data).ok()?;
                crate::land::heightmap::vertex_normals_to_yaml(&parsed)
            }
            _ => return None,
        },
        _ => return None,
    };
    let mut map = match structured {
        serde_json::Value::Object(m) => m,
        _ => return None,
    };
    map.insert(
        "raw_hex".to_string(),
        serde_json::Value::String(raw_hex.to_string()),
    );
    if let Some(v) = semantic_type {
        map.insert(
            "semantic_type".to_string(),
            serde_json::Value::String(v.to_string()),
        );
    }
    Some(serde_json::Value::Object(map))
}

/// GIL-free equivalent of raw_only_payload.
fn raw_only_payload_json(
    raw_hex: &str,
    semantic_type: Option<&str>,
    display_value: Option<&str>,
) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    if let Some(v) = display_value {
        map.insert(
            "display_value".to_string(),
            serde_json::Value::String(v.to_string()),
        );
    }
    map.insert(
        "raw_hex".to_string(),
        serde_json::Value::String(raw_hex.to_string()),
    );
    if let Some(v) = semantic_type {
        map.insert(
            "semantic_type".to_string(),
            serde_json::Value::String(v.to_string()),
        );
    }
    serde_json::Value::Object(map)
}

fn read_u16_le_json(data: &[u8], offset: usize) -> Option<u16> {
    let bytes = data.get(offset..offset.checked_add(2)?)?;
    Some(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_u32_le_json(data: &[u8], offset: usize) -> Option<u32> {
    let bytes = data.get(offset..offset.checked_add(4)?)?;
    Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn read_i16_le_json(data: &[u8], offset: usize) -> Option<i16> {
    let bytes = data.get(offset..offset.checked_add(2)?)?;
    Some(i16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_i32_le_json(data: &[u8], offset: usize) -> Option<i32> {
    let bytes = data.get(offset..offset.checked_add(4)?)?;
    Some(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn read_f32_le_json(data: &[u8], offset: usize) -> Option<f32> {
    let bytes = data.get(offset..offset.checked_add(4)?)?;
    Some(f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn decode_vmad_string_json(data: &[u8], offset: &mut usize) -> Option<String> {
    let len = read_u16_le_json(data, *offset)? as usize;
    *offset = (*offset).checked_add(2)?;
    let end = (*offset).checked_add(len)?;
    let bytes = data.get(*offset..end)?;
    *offset = end;
    let (decoded, _, _) = encoding_rs::WINDOWS_1252.decode(bytes);
    Some(decoded.into_owned())
}

fn vmad_type_label(value: u8) -> &'static str {
    // Labels match xEdit's wbPropTypeEnum (refs/xedit/Core/wbDefinitionsFO4.pas:4087)
    // so type 6 ("Variable") and type 0 ("None") never collide on round-trip.
    match value {
        0 => "None",
        1 => "Object",
        2 => "String",
        3 => "Int32",
        4 => "Float",
        5 => "Bool",
        6 => "Variable",
        7 => "Struct",
        11 => "Array of Object",
        12 => "Array of String",
        13 => "Array of Int32",
        14 => "Array of Float",
        15 => "Array of Bool",
        16 => "Array of Variable",
        17 => "Array of Struct",
        _ => "Unknown",
    }
}

fn read_vmad_object_value_json(
    data: &[u8],
    offset: &mut usize,
    object_format: u16,
    masters: &[String],
    plugin_name: &str,
) -> Option<serde_json::Value> {
    let mut object = serde_json::Map::new();
    if object_format == 2 {
        let unused = read_u16_le_json(data, *offset)?;
        let alias = read_i16_le_json(data, (*offset).checked_add(2)?)?;
        let formid = read_u32_le_json(data, (*offset).checked_add(4)?)?;
        *offset = (*offset).checked_add(8)?;
        if unused != 0 {
            object.insert(
                "Unused".to_string(),
                serde_json::Value::Number(unused.into()),
            );
        }
        object.insert("Alias".to_string(), serde_json::Value::Number(alias.into()));
        object.insert(
            "FormID".to_string(),
            serialize_formid_to_json(formid, plugin_name, masters, None),
        );
    } else {
        let formid = read_u32_le_json(data, *offset)?;
        let alias = read_i16_le_json(data, (*offset).checked_add(4)?)?;
        let unused = read_u16_le_json(data, (*offset).checked_add(6)?)?;
        *offset = (*offset).checked_add(8)?;
        object.insert(
            "FormID".to_string(),
            serialize_formid_to_json(formid, plugin_name, masters, None),
        );
        object.insert("Alias".to_string(), serde_json::Value::Number(alias.into()));
        if unused != 0 {
            object.insert(
                "Unused".to_string(),
                serde_json::Value::Number(unused.into()),
            );
        }
    }
    Some(serde_json::Value::Object(object))
}

fn read_vmad_property_value_json(
    data: &[u8],
    offset: &mut usize,
    property_type: u8,
    object_format: u16,
    masters: &[String],
    plugin_name: &str,
) -> Option<serde_json::Value> {
    match property_type {
        // Types 0 ("None") and 6 ("Variable") both have no payload bytes per
        // xEdit's wbScriptPropertyDecider — both map to wbNull in FO4/SF1.
        0 | 6 => Some(serde_json::Value::Null),
        1 => read_vmad_object_value_json(data, offset, object_format, masters, plugin_name),
        2 => decode_vmad_string_json(data, offset).map(serde_json::Value::String),
        3 => {
            let value = read_i32_le_json(data, *offset)?;
            *offset = (*offset).checked_add(4)?;
            Some(serde_json::Value::Number(value.into()))
        }
        4 => {
            let value = read_f32_le_json(data, *offset)?;
            *offset = (*offset).checked_add(4)?;
            serde_json::Number::from_f64(value as f64).map(serde_json::Value::Number)
        }
        5 => {
            let value = *data.get(*offset)? != 0;
            *offset = (*offset).checked_add(1)?;
            Some(serde_json::Value::Bool(value))
        }
        11 => {
            let count = read_i32_le_json(data, *offset)?;
            *offset = (*offset).checked_add(4)?;
            if count < 0 {
                return None;
            }
            let count = count as usize;
            let mut elements = Vec::with_capacity(count);
            for _ in 0..count {
                elements.push(read_vmad_object_value_json(
                    data,
                    offset,
                    object_format,
                    masters,
                    plugin_name,
                )?);
            }
            Some(serde_json::Value::Array(elements))
        }
        12 => {
            let count = read_i32_le_json(data, *offset)?;
            *offset = (*offset).checked_add(4)?;
            if count < 0 {
                return None;
            }
            let count = count as usize;
            let mut elements = Vec::with_capacity(count);
            for _ in 0..count {
                let s = decode_vmad_string_json(data, offset)?;
                elements.push(serde_json::Value::String(s));
            }
            Some(serde_json::Value::Array(elements))
        }
        13 => {
            let count = read_i32_le_json(data, *offset)?;
            *offset = (*offset).checked_add(4)?;
            if count < 0 {
                return None;
            }
            let count = count as usize;
            let mut elements = Vec::with_capacity(count);
            for _ in 0..count {
                let value = read_i32_le_json(data, *offset)?;
                *offset = (*offset).checked_add(4)?;
                elements.push(serde_json::Value::Number(value.into()));
            }
            Some(serde_json::Value::Array(elements))
        }
        14 => {
            let count = read_i32_le_json(data, *offset)?;
            *offset = (*offset).checked_add(4)?;
            if count < 0 {
                return None;
            }
            let count = count as usize;
            let mut elements = Vec::with_capacity(count);
            for _ in 0..count {
                let value = read_f32_le_json(data, *offset)?;
                *offset = (*offset).checked_add(4)?;
                elements.push(
                    serde_json::Number::from_f64(value as f64).map(serde_json::Value::Number)?,
                );
            }
            Some(serde_json::Value::Array(elements))
        }
        15 => {
            let count = read_i32_le_json(data, *offset)?;
            *offset = (*offset).checked_add(4)?;
            if count < 0 {
                return None;
            }
            let count = count as usize;
            let mut elements = Vec::with_capacity(count);
            for _ in 0..count {
                let raw = *data.get(*offset)?;
                *offset = (*offset).checked_add(1)?;
                elements.push(serde_json::Value::Bool(raw != 0));
            }
            Some(serde_json::Value::Array(elements))
        }
        7 => read_vmad_struct_value_json(data, offset, object_format, masters, plugin_name),
        16 => {
            // xEdit FO4 wbScriptPropertyDecider:{16} models Array of Variable
            // as a struct with only an u32 element count — no element bytes
            // follow. There are zero vanilla samples; if a real type-16 with
            // elements ever appears, the next property's parse will misalign
            // and the outer codec falls back to raw_hex preservation.
            let count = read_u32_le_json(data, *offset)?;
            *offset = (*offset).checked_add(4)?;
            let mut payload = serde_json::Map::new();
            payload.insert(
                "Element Count".to_string(),
                serde_json::Value::Number(count.into()),
            );
            Some(serde_json::Value::Object(payload))
        }
        17 => {
            let count = read_i32_le_json(data, *offset)?;
            *offset = (*offset).checked_add(4)?;
            if count < 0 {
                return None;
            }
            let count = count as usize;
            let mut elements = Vec::with_capacity(count);
            for _ in 0..count {
                elements.push(read_vmad_struct_value_json(
                    data,
                    offset,
                    object_format,
                    masters,
                    plugin_name,
                )?);
            }
            Some(serde_json::Value::Array(elements))
        }
        _ => None,
    }
}

/// Decode a single VMAD `wbScriptPropertyStruct` (type 7 payload, also reused
/// per element of type 17 "Array of Struct"). Layout per xEdit FO4
/// wbScriptPropertyStruct (refs/xedit/Core/wbDefinitionsFO4.pas:4137):
/// `<i32 member_count><member...>` where each member is
/// `<u16 name_len><name><u8 type><u8 flags><value bytes per type>`.
fn read_vmad_struct_value_json(
    data: &[u8],
    offset: &mut usize,
    object_format: u16,
    masters: &[String],
    plugin_name: &str,
) -> Option<serde_json::Value> {
    let count = read_i32_le_json(data, *offset)?;
    *offset = (*offset).checked_add(4)?;
    if count < 0 {
        return None;
    }
    let count = count as usize;
    let mut members = Vec::with_capacity(count);
    for _ in 0..count {
        let member_name = decode_vmad_string_json(data, offset)?;
        let member_type = *data.get(*offset)?;
        *offset = (*offset).checked_add(1)?;
        let member_flags = *data.get(*offset)?;
        *offset = (*offset).checked_add(1)?;
        let member_value = read_vmad_property_value_json(
            data,
            offset,
            member_type,
            object_format,
            masters,
            plugin_name,
        )?;
        let mut entry = serde_json::Map::new();
        entry.insert(
            "memberName".to_string(),
            serde_json::Value::String(member_name),
        );
        entry.insert(
            "Type".to_string(),
            serde_json::Value::String(vmad_type_label(member_type).to_string()),
        );
        if member_flags != 0 {
            entry.insert(
                "Flags".to_string(),
                serde_json::Value::Number(member_flags.into()),
            );
        }
        entry.insert("Value".to_string(), member_value);
        members.push(serde_json::Value::Object(entry));
    }
    Some(serde_json::Value::Array(members))
}

/// Decode a single VMAD `wbScriptEntry` (script_name + flags + properties).
/// Used both for the top-level Scripts array and for fragment payloads
/// (INFO/PACK/SCEN/PERK/TERM all embed wbScriptEntry; QUST embeds the inner
/// flags+properties pair conditionally).
fn read_vmad_script_entry_json(
    data: &[u8],
    offset: &mut usize,
    object_format: u16,
    masters: &[String],
    plugin_name: &str,
) -> Option<serde_json::Value> {
    let script_name = decode_vmad_string_json(data, offset)?;
    let flags = *data.get(*offset)?;
    *offset = (*offset).checked_add(1)?;
    let property_count = read_u16_le_json(data, *offset)? as usize;
    *offset = (*offset).checked_add(2)?;

    let mut properties = Vec::with_capacity(property_count);
    for _ in 0..property_count {
        let property_name = decode_vmad_string_json(data, offset)?;
        let property_type = *data.get(*offset)?;
        *offset = (*offset).checked_add(1)?;
        let property_flags = *data.get(*offset)?;
        *offset = (*offset).checked_add(1)?;
        let value = read_vmad_property_value_json(
            data,
            offset,
            property_type,
            object_format,
            masters,
            plugin_name,
        )?;
        let mut property = serde_json::Map::new();
        property.insert(
            "propertyName".to_string(),
            serde_json::Value::String(property_name),
        );
        property.insert(
            "Type".to_string(),
            serde_json::Value::String(vmad_type_label(property_type).to_string()),
        );
        if property_flags != 0 {
            property.insert(
                "Flags".to_string(),
                serde_json::Value::Number(property_flags.into()),
            );
        }
        property.insert("Value".to_string(), value);
        properties.push(serde_json::Value::Object(property));
    }

    let mut script = serde_json::Map::new();
    script.insert(
        "ScriptName".to_string(),
        serde_json::Value::String(script_name),
    );
    if flags != 0 {
        script.insert("Flags".to_string(), serde_json::Value::Number(flags.into()));
    }
    script.insert(
        "Properties".to_string(),
        serde_json::Value::Array(properties),
    );
    Some(serde_json::Value::Object(script))
}

/// Decode an array of `Fragment` rows (the simple `<i8 unknown><scriptname>
/// <fragmentname>` shape used by INFO/PACK/SCEN). The caller supplies the
/// element count.
fn read_vmad_simple_fragments_json(
    data: &[u8],
    offset: &mut usize,
    count: usize,
) -> Option<serde_json::Value> {
    let mut fragments = Vec::with_capacity(count);
    for _ in 0..count {
        let unknown = *data.get(*offset)? as i8;
        *offset = (*offset).checked_add(1)?;
        let script_name = decode_vmad_string_json(data, offset)?;
        let fragment_name = decode_vmad_string_json(data, offset)?;
        let mut fragment = serde_json::Map::new();
        fragment.insert(
            "Unknown".to_string(),
            serde_json::Value::Number((unknown as i64).into()),
        );
        fragment.insert(
            "ScriptName".to_string(),
            serde_json::Value::String(script_name),
        );
        fragment.insert(
            "FragmentName".to_string(),
            serde_json::Value::String(fragment_name),
        );
        fragments.push(serde_json::Value::Object(fragment));
    }
    Some(serde_json::Value::Array(fragments))
}

/// Decode INFO/PACK fragment block: version, flags, ScriptEntry, fragments
/// array (count = popcount(flags)). PACK adds bit 4 ("OnChange") to the flags
/// enum but the binary layout is identical.
fn read_vmad_fragments_info_or_pack_json(
    data: &[u8],
    offset: &mut usize,
    object_format: u16,
    masters: &[String],
    plugin_name: &str,
) -> Option<serde_json::Value> {
    let version = *data.get(*offset)? as i8;
    *offset = (*offset).checked_add(1)?;
    let flags = *data.get(*offset)?;
    *offset = (*offset).checked_add(1)?;
    let script = read_vmad_script_entry_json(data, offset, object_format, masters, plugin_name)?;
    let count = (flags as u32).count_ones() as usize;
    let fragments = read_vmad_simple_fragments_json(data, offset, count)?;
    let mut block = serde_json::Map::new();
    block.insert(
        "Version".to_string(),
        serde_json::Value::Number((version as i64).into()),
    );
    block.insert("Flags".to_string(), serde_json::Value::Number(flags.into()));
    block.insert("Script".to_string(), script);
    block.insert("Fragments".to_string(), fragments);
    Some(serde_json::Value::Object(block))
}

/// Decode SCEN fragment block: identical to INFO plus a trailing u16-prefixed
/// Phase Fragments array.
fn read_vmad_fragments_scen_json(
    data: &[u8],
    offset: &mut usize,
    object_format: u16,
    masters: &[String],
    plugin_name: &str,
) -> Option<serde_json::Value> {
    let block =
        read_vmad_fragments_info_or_pack_json(data, offset, object_format, masters, plugin_name)?;
    let mut block = match block {
        serde_json::Value::Object(m) => m,
        _ => return None,
    };
    let phase_count = read_u16_le_json(data, *offset)? as usize;
    *offset = (*offset).checked_add(2)?;
    let mut phase_fragments = Vec::with_capacity(phase_count);
    for _ in 0..phase_count {
        let phase_flag = *data.get(*offset)?;
        *offset = (*offset).checked_add(1)?;
        let phase_index = *data.get(*offset)?;
        *offset = (*offset).checked_add(1)?;
        let unknown_s16 = read_i16_le_json(data, *offset)?;
        *offset = (*offset).checked_add(2)?;
        let unknown_s8_a = *data.get(*offset)? as i8;
        *offset = (*offset).checked_add(1)?;
        let unknown_s8_b = *data.get(*offset)? as i8;
        *offset = (*offset).checked_add(1)?;
        let script_name = decode_vmad_string_json(data, offset)?;
        let fragment_name = decode_vmad_string_json(data, offset)?;
        let mut entry = serde_json::Map::new();
        entry.insert(
            "Phase Flag".to_string(),
            serde_json::Value::Number(phase_flag.into()),
        );
        entry.insert(
            "Phase Index".to_string(),
            serde_json::Value::Number(phase_index.into()),
        );
        entry.insert(
            "Unknown".to_string(),
            serde_json::Value::Number(unknown_s16.into()),
        );
        entry.insert(
            "Unknown1".to_string(),
            serde_json::Value::Number((unknown_s8_a as i64).into()),
        );
        entry.insert(
            "Unknown2".to_string(),
            serde_json::Value::Number((unknown_s8_b as i64).into()),
        );
        entry.insert(
            "ScriptName".to_string(),
            serde_json::Value::String(script_name),
        );
        entry.insert(
            "FragmentName".to_string(),
            serde_json::Value::String(fragment_name),
        );
        phase_fragments.push(serde_json::Value::Object(entry));
    }
    block.insert(
        "Phase Fragments".to_string(),
        serde_json::Value::Array(phase_fragments),
    );
    Some(serde_json::Value::Object(block))
}

/// Decode PERK/TERM fragment block (`wbScriptFragments`): version, ScriptEntry,
/// then u16-prefixed Fragments array of `<u16 fragment_index><i16 unused>
/// <i8 unknown><scriptname><fragmentname>`.
fn read_vmad_fragments_perk_term_json(
    data: &[u8],
    offset: &mut usize,
    object_format: u16,
    masters: &[String],
    plugin_name: &str,
) -> Option<serde_json::Value> {
    let version = *data.get(*offset)? as i8;
    *offset = (*offset).checked_add(1)?;
    let script = read_vmad_script_entry_json(data, offset, object_format, masters, plugin_name)?;
    let count = read_u16_le_json(data, *offset)? as usize;
    *offset = (*offset).checked_add(2)?;
    let mut fragments = Vec::with_capacity(count);
    for _ in 0..count {
        let fragment_index = read_u16_le_json(data, *offset)?;
        *offset = (*offset).checked_add(2)?;
        let unused = read_i16_le_json(data, *offset)?;
        *offset = (*offset).checked_add(2)?;
        let unknown = *data.get(*offset)? as i8;
        *offset = (*offset).checked_add(1)?;
        let script_name = decode_vmad_string_json(data, offset)?;
        let fragment_name = decode_vmad_string_json(data, offset)?;
        let mut fragment = serde_json::Map::new();
        fragment.insert(
            "Fragment Index".to_string(),
            serde_json::Value::Number(fragment_index.into()),
        );
        fragment.insert(
            "Unused".to_string(),
            serde_json::Value::Number(unused.into()),
        );
        fragment.insert(
            "Unknown".to_string(),
            serde_json::Value::Number((unknown as i64).into()),
        );
        fragment.insert(
            "ScriptName".to_string(),
            serde_json::Value::String(script_name),
        );
        fragment.insert(
            "FragmentName".to_string(),
            serde_json::Value::String(fragment_name),
        );
        fragments.push(serde_json::Value::Object(fragment));
    }
    let mut block = serde_json::Map::new();
    block.insert(
        "Version".to_string(),
        serde_json::Value::Number((version as i64).into()),
    );
    block.insert("Script".to_string(), script);
    block.insert("Fragments".to_string(), serde_json::Value::Array(fragments));
    Some(serde_json::Value::Object(block))
}

/// Decode QUST fragment block (`wbScriptFragmentsQuest`) plus the trailing
/// Aliases array. Layout:
///   <i8 version><u16 fragment_count><u16-prefixed script_name>
///   if script_name != "": <u8 script_flags><u16 prop_count><properties...>
///   <fragment_count entries: u16 stage, i16 unknown, i32 stage_index,
///                            i8 unknown, scriptname, fragmentname>
///   <u16 alias_count><alias entries: object(8 bytes), i16 version,
///                                    i16 object_format, u16 script_count,
///                                    scripts[]>
fn read_vmad_fragments_quest_json(
    data: &[u8],
    offset: &mut usize,
    object_format: u16,
    masters: &[String],
    plugin_name: &str,
) -> Option<serde_json::Value> {
    let version = *data.get(*offset)? as i8;
    *offset = (*offset).checked_add(1)?;
    let fragment_count = read_u16_le_json(data, *offset)? as usize;
    *offset = (*offset).checked_add(2)?;
    let script_name = decode_vmad_string_json(data, offset)?;
    let mut script = serde_json::Map::new();
    script.insert(
        "ScriptName".to_string(),
        serde_json::Value::String(script_name.clone()),
    );
    if !script_name.is_empty() {
        let script_flags = *data.get(*offset)?;
        *offset = (*offset).checked_add(1)?;
        if script_flags != 0 {
            script.insert(
                "Flags".to_string(),
                serde_json::Value::Number(script_flags.into()),
            );
        }
        let property_count = read_u16_le_json(data, *offset)? as usize;
        *offset = (*offset).checked_add(2)?;
        let mut properties = Vec::with_capacity(property_count);
        for _ in 0..property_count {
            let property_name = decode_vmad_string_json(data, offset)?;
            let property_type = *data.get(*offset)?;
            *offset = (*offset).checked_add(1)?;
            let property_flags = *data.get(*offset)?;
            *offset = (*offset).checked_add(1)?;
            let value = read_vmad_property_value_json(
                data,
                offset,
                property_type,
                object_format,
                masters,
                plugin_name,
            )?;
            let mut property = serde_json::Map::new();
            property.insert(
                "propertyName".to_string(),
                serde_json::Value::String(property_name),
            );
            property.insert(
                "Type".to_string(),
                serde_json::Value::String(vmad_type_label(property_type).to_string()),
            );
            if property_flags != 0 {
                property.insert(
                    "Flags".to_string(),
                    serde_json::Value::Number(property_flags.into()),
                );
            }
            property.insert("Value".to_string(), value);
            properties.push(serde_json::Value::Object(property));
        }
        script.insert(
            "Properties".to_string(),
            serde_json::Value::Array(properties),
        );
    }

    let mut fragments = Vec::with_capacity(fragment_count);
    for _ in 0..fragment_count {
        let stage = read_u16_le_json(data, *offset)?;
        *offset = (*offset).checked_add(2)?;
        let unknown_s16 = read_i16_le_json(data, *offset)?;
        *offset = (*offset).checked_add(2)?;
        let stage_index = read_i32_le_json(data, *offset)?;
        *offset = (*offset).checked_add(4)?;
        let unknown_s8 = *data.get(*offset)? as i8;
        *offset = (*offset).checked_add(1)?;
        let frag_script = decode_vmad_string_json(data, offset)?;
        let fragment_name = decode_vmad_string_json(data, offset)?;
        let mut fragment = serde_json::Map::new();
        fragment.insert(
            "Quest Stage".to_string(),
            serde_json::Value::Number(stage.into()),
        );
        fragment.insert(
            "Unknown".to_string(),
            serde_json::Value::Number(unknown_s16.into()),
        );
        fragment.insert(
            "Quest Stage Index".to_string(),
            serde_json::Value::Number(stage_index.into()),
        );
        fragment.insert(
            "Unknown1".to_string(),
            serde_json::Value::Number((unknown_s8 as i64).into()),
        );
        fragment.insert(
            "ScriptName".to_string(),
            serde_json::Value::String(frag_script),
        );
        fragment.insert(
            "FragmentName".to_string(),
            serde_json::Value::String(fragment_name),
        );
        fragments.push(serde_json::Value::Object(fragment));
    }

    let alias_count = read_u16_le_json(data, *offset)? as usize;
    *offset = (*offset).checked_add(2)?;
    let mut aliases = Vec::with_capacity(alias_count);
    for _ in 0..alias_count {
        let alias_object =
            read_vmad_object_value_json(data, offset, object_format, masters, plugin_name)?;
        let alias_version = read_i16_le_json(data, *offset)?;
        *offset = (*offset).checked_add(2)?;
        let alias_object_format = read_i16_le_json(data, *offset)?;
        *offset = (*offset).checked_add(2)?;
        let alias_script_count = read_u16_le_json(data, *offset)? as usize;
        *offset = (*offset).checked_add(2)?;
        let mut alias_scripts = Vec::with_capacity(alias_script_count);
        for _ in 0..alias_script_count {
            alias_scripts.push(read_vmad_script_entry_json(
                data,
                offset,
                alias_object_format as u16,
                masters,
                plugin_name,
            )?);
        }
        let mut alias = serde_json::Map::new();
        alias.insert("Object".to_string(), alias_object);
        alias.insert(
            "Version".to_string(),
            serde_json::Value::Number(alias_version.into()),
        );
        alias.insert(
            "Object Format".to_string(),
            serde_json::Value::Number(alias_object_format.into()),
        );
        alias.insert(
            "Alias Scripts".to_string(),
            serde_json::Value::Array(alias_scripts),
        );
        aliases.push(serde_json::Value::Object(alias));
    }

    let mut block = serde_json::Map::new();
    block.insert(
        "Version".to_string(),
        serde_json::Value::Number((version as i64).into()),
    );
    block.insert(
        "FragmentCount".to_string(),
        serde_json::Value::Number(fragment_count.into()),
    );
    block.insert("Script".to_string(), serde_json::Value::Object(script));
    block.insert("Fragments".to_string(), serde_json::Value::Array(fragments));
    block.insert("Aliases".to_string(), serde_json::Value::Array(aliases));
    Some(serde_json::Value::Object(block))
}

fn compact_vmad_payload_json(
    data: &[u8],
    masters: &[String],
    plugin_name: &str,
    semantic_type: Option<&str>,
) -> Option<serde_json::Value> {
    if data.len() < 6 {
        return None;
    }

    let version = read_u16_le_json(data, 0)?;
    let object_format = read_u16_le_json(data, 2)?;
    let script_count = read_u16_le_json(data, 4)? as usize;
    let mut offset = 6usize;
    let mut scripts = Vec::with_capacity(script_count);

    for _ in 0..script_count {
        scripts.push(read_vmad_script_entry_json(
            data,
            &mut offset,
            object_format,
            masters,
            plugin_name,
        )?);
    }

    // Parse the trailing fragment block when the host record's
    // signature implies one. Decoded fragments live under a top-level key
    // (e.g. "Script Fragments") so they round-trip through YAML; the
    // unparsed-tail / raw_hex fallback preserves byte-exactness if a
    // fragment decoder fails.
    let fragments = if offset < data.len() {
        match semantic_type {
            Some("INFO") | Some("PACK") => read_vmad_fragments_info_or_pack_json(
                data,
                &mut offset,
                object_format,
                masters,
                plugin_name,
            ),
            Some("SCEN") => read_vmad_fragments_scen_json(
                data,
                &mut offset,
                object_format,
                masters,
                plugin_name,
            ),
            Some("PERK") | Some("TERM") => read_vmad_fragments_perk_term_json(
                data,
                &mut offset,
                object_format,
                masters,
                plugin_name,
            ),
            Some("QUST") => read_vmad_fragments_quest_json(
                data,
                &mut offset,
                object_format,
                masters,
                plugin_name,
            ),
            _ => None,
        }
    } else {
        None
    };

    let mut payload = serde_json::Map::new();
    payload.insert(
        "kind".to_string(),
        serde_json::Value::String("vmad".to_string()),
    );
    payload.insert(
        "size".to_string(),
        serde_json::Value::Number(data.len().into()),
    );
    payload.insert(
        "Version".to_string(),
        serde_json::Value::Number(version.into()),
    );
    payload.insert(
        "Object Format".to_string(),
        serde_json::Value::Number(object_format.into()),
    );
    payload.insert("Scripts".to_string(), serde_json::Value::Array(scripts));
    if let Some(value) = fragments {
        payload.insert("Script Fragments".to_string(), value);
    }
    if let Some(value) = semantic_type {
        payload.insert(
            "semantic_type".to_string(),
            serde_json::Value::String(value.to_string()),
        );
    }

    // When the parsed payload round-trips byte-exactly,
    // omit `raw_hex` so the YAML doesn't carry a redundant copy of the bytes.
    // If validation fails (decoder bug, unknown fragment layout, an unparsed
    // tail), keep `raw_hex` and emit `tail_hex` as the safety net — the
    // encoder will fall back to the raw bytes in `build_subrecord_from_*`.
    let fully_consumed = offset == data.len();
    let mut omit_raw_hex = false;
    if fully_consumed {
        let candidate = serde_json::Value::Object(payload.clone());
        if let Some(reencoded) =
            crate::plugin_runtime::build_vmad_bytes_from_payload(&candidate, masters, plugin_name)
            && reencoded == data
        {
            omit_raw_hex = true;
        }
    }
    if !fully_consumed {
        payload.insert(
            "tail_hex".to_string(),
            serde_json::Value::String(hex::encode_upper(&data[offset..])),
        );
    }
    if !omit_raw_hex {
        payload.insert(
            "raw_hex".to_string(),
            serde_json::Value::String(hex::encode_upper(data)),
        );
    }
    Some(serde_json::Value::Object(payload))
}

pub fn semantic_vmad_payload_json(data: &[u8]) -> serde_json::Value {
    compact_vmad_payload_json(data, &[], "", None).unwrap_or_else(|| {
        let mut payload = serde_json::Map::new();
        payload.insert(
            "kind".to_string(),
            serde_json::Value::String("vmad".to_string()),
        );
        payload.insert(
            "size".to_string(),
            serde_json::Value::Number(data.len().into()),
        );
        payload.insert(
            "raw_hex".to_string(),
            serde_json::Value::String(hex::encode_upper(data)),
        );
        serde_json::Value::Object(payload)
    })
}

/// Decode an OMOD DATA subrecord (codec=`omod_data`) into a structured JSON
/// object whose keys match the schema's field ids. The caller passes the
/// result through `serialize_struct_value_json` to convert ids → display
/// labels, drop default fields, and resolve formids.
///
/// Layout (FO4/FO76):
///   * 20-byte header `<IIBBIBBI`: include_count, property_count,
///     unknown_bool_1, unknown_bool_2, form_type, max_rank,
///     level_tier_scaled_offset, attach_point.
///   * u32 attach-parent slot count, then that many u32 slots.
///   * 1 item row `<I` (4 bytes): value_1.
///   * `include_count` rows of `<IBBB` (7 bytes each).
///   * `property_count` rows of `<B3xB3xH2xIIf` (24 bytes each, explicit pad).
fn decode_omod_data_to_field_map_json(data: &[u8]) -> Option<serde_json::Value> {
    const HEADER_SIZE: usize = 20;
    const SLOT_COUNT_SIZE: usize = 4;
    const ITEM_SIZE: usize = 4;
    const INCLUDE_SIZE: usize = 7;
    const PROPERTY_SIZE: usize = 24;

    if data.len() < HEADER_SIZE + SLOT_COUNT_SIZE {
        return None;
    }
    let include_count = read_u32_le_json(data, 0)? as usize;
    let property_count = read_u32_le_json(data, 4)? as usize;
    let unknown_bool_1 = data[8];
    let unknown_bool_2 = data[9];
    let form_type = read_u32_le_json(data, 10)?;
    let max_rank = data[14];
    let level_tier_scaled_offset = data[15];
    let attach_point = read_u32_le_json(data, 16)?;
    let attach_parent_slot_count = read_u32_le_json(data, HEADER_SIZE)? as usize;

    let attach_slots_size = attach_parent_slot_count.checked_mul(4)?;
    let expected_size = HEADER_SIZE
        .checked_add(SLOT_COUNT_SIZE)?
        .checked_add(attach_slots_size)?
        .checked_add(ITEM_SIZE)?
        .checked_add(include_count.checked_mul(INCLUDE_SIZE)?)?
        .checked_add(property_count.checked_mul(PROPERTY_SIZE)?)?;
    if data.len() < expected_size {
        return None;
    }

    let mut cursor = HEADER_SIZE + SLOT_COUNT_SIZE;
    let mut attach_parent_slots: Vec<serde_json::Value> =
        Vec::with_capacity(attach_parent_slot_count);
    for _ in 0..attach_parent_slot_count {
        let slot = read_u32_le_json(data, cursor)?;
        attach_parent_slots.push(serde_json::Value::Number(slot.into()));
        cursor += 4;
    }

    let item_value_1 = read_u32_le_json(data, cursor)?;
    cursor += ITEM_SIZE;
    let mut item_row = serde_json::Map::new();
    item_row.insert(
        "value_1".to_string(),
        serde_json::Value::Number(item_value_1.into()),
    );
    let items = vec![serde_json::Value::Object(item_row)];

    let mut includes: Vec<serde_json::Value> = Vec::with_capacity(include_count);
    for _ in 0..include_count {
        let mod_id = read_u32_le_json(data, cursor)?;
        let minimum_level = data[cursor + 4];
        let optional = data[cursor + 5];
        let dont_use_all = data[cursor + 6];
        cursor += INCLUDE_SIZE;
        let mut row = serde_json::Map::new();
        row.insert("mod".to_string(), serde_json::Value::Number(mod_id.into()));
        row.insert(
            "minimum_level".to_string(),
            serde_json::Value::Number(minimum_level.into()),
        );
        row.insert(
            "optional".to_string(),
            serde_json::Value::Number(optional.into()),
        );
        row.insert(
            "dont_use_all".to_string(),
            serde_json::Value::Number(dont_use_all.into()),
        );
        includes.push(serde_json::Value::Object(row));
    }

    let mut properties: Vec<serde_json::Value> = Vec::with_capacity(property_count);
    for _ in 0..property_count {
        let value_type = data[cursor];
        let function_type = data[cursor + 4];
        let property_id = read_u16_le_json(data, cursor + 8)?;
        let value_1 = read_u32_le_json(data, cursor + 12)?;
        let value_2 = read_u32_le_json(data, cursor + 16)?;
        let step = read_f32_le_json(data, cursor + 20)?;
        cursor += PROPERTY_SIZE;
        let mut row = serde_json::Map::new();
        row.insert(
            "value_type".to_string(),
            serde_json::Value::Number(value_type.into()),
        );
        row.insert(
            "function_type".to_string(),
            serde_json::Value::Number(function_type.into()),
        );
        row.insert(
            "property".to_string(),
            serde_json::Value::Number(property_id.into()),
        );
        row.insert(
            "value_1".to_string(),
            serde_json::Value::Number(value_1.into()),
        );
        row.insert(
            "value_2".to_string(),
            serde_json::Value::Number(value_2.into()),
        );
        row.insert(
            "step".to_string(),
            serde_json::Number::from_f64(step as f64)
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null),
        );
        properties.push(serde_json::Value::Object(row));
    }

    if cursor != data.len() {
        return None;
    }

    let mut out = serde_json::Map::new();
    out.insert(
        "include_count".to_string(),
        serde_json::Value::Number(include_count.into()),
    );
    out.insert(
        "property_count".to_string(),
        serde_json::Value::Number(property_count.into()),
    );
    out.insert(
        "unknown_bool_1".to_string(),
        serde_json::Value::Number(unknown_bool_1.into()),
    );
    out.insert(
        "unknown_bool_2".to_string(),
        serde_json::Value::Number(unknown_bool_2.into()),
    );
    out.insert(
        "form_type".to_string(),
        serde_json::Value::Number(form_type.into()),
    );
    out.insert(
        "max_rank".to_string(),
        serde_json::Value::Number(max_rank.into()),
    );
    out.insert(
        "level_tier_scaled_offset".to_string(),
        serde_json::Value::Number(level_tier_scaled_offset.into()),
    );
    out.insert(
        "attach_point".to_string(),
        serde_json::Value::Number(attach_point.into()),
    );
    out.insert(
        "attach_parent_slots".to_string(),
        serde_json::Value::Array(attach_parent_slots),
    );
    out.insert("items".to_string(), serde_json::Value::Array(items));
    out.insert("includes".to_string(), serde_json::Value::Array(includes));
    out.insert(
        "properties".to_string(),
        serde_json::Value::Array(properties),
    );
    Some(serde_json::Value::Object(out))
}

/// Compact-decode the OMOD DATA subrecord and run it through the standard
/// struct serializer so display labels, formid resolution, and default-field
/// elision all happen consistently with other struct subrecords. Returns
/// `None` to let the caller fall back to raw-hex preservation.
fn compact_omod_data_payload_json(
    data: &[u8],
    sub_spec: &SchemaSubrecordJson,
    schema: &CompiledSchema,
    masters: &[String],
    plugin_name: &str,
) -> Option<serde_json::Value> {
    let decoded = decode_omod_data_to_field_map_json(data)?;
    Some(serialize_struct_value_json(
        &decoded,
        &sub_spec.fields,
        schema,
        masters,
        plugin_name,
    ))
}

/// Decode a MODT / Model-Info subrecord (codec=`model_info`) into a
/// structured JSON object whose keys match the authoring JSON contract
/// shared with the schema generator (`textures`, `addon_nodes`,
/// `srgb_count`, `materials`). The caller passes the result through
/// `serialize_struct_value_json` to convert ids → display labels and drop
/// default fields.
///
/// Layout (gmTES5+ = FO4/FO76/Starfield):
///   * u32 counter_count, MUST be 4 — older/variant layouts fall back to
///     raw-hex preservation via `None`.
///   * u32 counters[4] = [num_textures, num_addon_nodes, srgb_count,
///     num_materials]. `srgb_count` is a bare count with no backing array
///     and is preserved verbatim.
///   * Texture[num_textures], then addon_nodes[num_addon_nodes] (u32 each),
///     then Material[num_materials]. Texture/Material rows are 12 bytes:
///     `<I4sI>` (file_hash, 4-byte ascii extension, folder_hash).
fn decode_model_info_to_field_map_json(data: &[u8]) -> Option<serde_json::Value> {
    const HEADER_SIZE: usize = 20;
    const ENTRY_SIZE: usize = 12;

    if data.len() < HEADER_SIZE {
        return None;
    }
    if read_u32_le_json(data, 0)? != 4 {
        return None;
    }
    let num_textures = read_u32_le_json(data, 4)? as usize;
    let num_addon_nodes = read_u32_le_json(data, 8)? as usize;
    let srgb_count = read_u32_le_json(data, 12)?;
    let num_materials = read_u32_le_json(data, 16)? as usize;

    let expected_size = HEADER_SIZE
        .checked_add(num_textures.checked_mul(ENTRY_SIZE)?)?
        .checked_add(num_addon_nodes.checked_mul(4)?)?
        .checked_add(num_materials.checked_mul(ENTRY_SIZE)?)?;
    if data.len() != expected_size {
        return None;
    }

    let mut cursor = HEADER_SIZE;
    let mut textures: Vec<serde_json::Value> = Vec::with_capacity(num_textures);
    for _ in 0..num_textures {
        textures.push(decode_model_info_entry_json(data, cursor)?);
        cursor += ENTRY_SIZE;
    }

    let mut addon_nodes: Vec<serde_json::Value> = Vec::with_capacity(num_addon_nodes);
    for _ in 0..num_addon_nodes {
        addon_nodes.push(serde_json::Value::Number(
            read_u32_le_json(data, cursor)?.into(),
        ));
        cursor += 4;
    }

    let mut materials: Vec<serde_json::Value> = Vec::with_capacity(num_materials);
    for _ in 0..num_materials {
        materials.push(decode_model_info_entry_json(data, cursor)?);
        cursor += ENTRY_SIZE;
    }

    if cursor != data.len() {
        return None;
    }

    let mut out = serde_json::Map::new();
    out.insert("textures".to_string(), serde_json::Value::Array(textures));
    out.insert(
        "addon_nodes".to_string(),
        serde_json::Value::Array(addon_nodes),
    );
    out.insert(
        "srgb_count".to_string(),
        serde_json::Value::Number(srgb_count.into()),
    );
    out.insert("materials".to_string(), serde_json::Value::Array(materials));
    Some(serde_json::Value::Object(out))
}

/// Decode one 12-byte Texture/Material row: `<I4sI>` (file_hash, 4-byte
/// ascii extension, folder_hash). The extension is trimmed of trailing NUL
/// padding; bails to `None` (whole-subrecord raw-hex fallback) if any byte
/// past the first NUL is non-zero, since that padding can't be reconstructed
/// byte-exact on encode.
fn decode_model_info_entry_json(data: &[u8], offset: usize) -> Option<serde_json::Value> {
    let file_hash = read_u32_le_json(data, offset)?;
    let ext_bytes = data.get(offset + 4..offset + 8)?;
    let ext_len = ext_bytes
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(ext_bytes.len());
    if ext_bytes[ext_len..].iter().any(|&b| b != 0) {
        return None;
    }
    let extension = std::str::from_utf8(&ext_bytes[..ext_len]).ok()?.to_string();
    let folder_hash = read_u32_le_json(data, offset + 8)?;

    let mut row = serde_json::Map::new();
    row.insert(
        "file_hash".to_string(),
        serde_json::Value::Number(file_hash.into()),
    );
    row.insert(
        "extension".to_string(),
        serde_json::Value::String(extension),
    );
    row.insert(
        "folder_hash".to_string(),
        serde_json::Value::Number(folder_hash.into()),
    );
    Some(serde_json::Value::Object(row))
}

/// Compact-decode the MODT subrecord and run it through the standard struct
/// serializer so display labels and default-field elision happen
/// consistently with other struct subrecords. Returns `None` to let the
/// caller fall back to raw-hex preservation.
///
/// `pub` (rather than crate-private like the omod_data counterpart) so
/// `esp/tests/modt_roundtrip.rs` can exercise the codec against the real
/// generated schema once schema_forge has wired MODT to codec=`model_info`.
pub fn compact_model_info_payload_json(
    data: &[u8],
    sub_spec: &SchemaSubrecordJson,
    schema: &CompiledSchema,
    masters: &[String],
    plugin_name: &str,
) -> Option<serde_json::Value> {
    let decoded = decode_model_info_to_field_map_json(data)?;
    Some(serialize_struct_value_json(
        &decoded,
        &sub_spec.fields,
        schema,
        masters,
        plugin_name,
    ))
}

fn compact_lvlo_entry_payload_json(
    data: &[u8],
    masters: &[String],
    plugin_name: &str,
    semantic_type: Option<&str>,
) -> Option<serde_json::Value> {
    if data.len() != 12 {
        return None;
    }

    let level = read_u16_le_json(data, 0)?;
    let unknown_1 = read_u16_le_json(data, 2)?;
    let reference = read_u32_le_json(data, 4)?;
    let count = read_u16_le_json(data, 8)?;
    let unknown_2 = read_u16_le_json(data, 10)?;

    let mut entry = serde_json::Map::new();
    entry.insert("Level".to_string(), serde_json::Value::Number(level.into()));
    if unknown_1 != 0 {
        entry.insert(
            "Unknown1".to_string(),
            serde_json::Value::Number(unknown_1.into()),
        );
    }
    entry.insert(
        "Reference".to_string(),
        serialize_formid_to_json(reference, plugin_name, masters, None),
    );
    entry.insert("Count".to_string(), serde_json::Value::Number(count.into()));
    if unknown_2 != 0 {
        entry.insert(
            "Unknown2".to_string(),
            serde_json::Value::Number(unknown_2.into()),
        );
    }

    let raw_hex = hex::encode_upper(data);
    let mut payload = serde_json::Map::new();
    payload.insert("Data".to_string(), serde_json::Value::Object(entry));
    payload.insert("raw_hex".to_string(), serde_json::Value::String(raw_hex));
    if let Some(value) = semantic_type {
        payload.insert(
            "semantic_type".to_string(),
            serde_json::Value::String(value.to_string()),
        );
    }
    Some(serde_json::Value::Object(payload))
}

/// GIL-free equivalent of serialize_formid_value.
fn serialize_formid_to_json(
    raw_value: u32,
    plugin_name: &str,
    masters: &[String],
    target: Option<&str>,
) -> serde_json::Value {
    if raw_value == 0 {
        return serde_json::Value::Null;
    }
    let object_id = raw_value & 0x00FF_FFFF;
    let index = ((raw_value >> 24) & 0xFF) as usize;
    let mapping_len = masters.len() + 1;

    let mut reference_map = serde_json::Map::new();
    let missing_index = if index < mapping_len {
        let resolved_plugin = if index == masters.len() {
            plugin_name
        } else {
            masters[index].as_str()
        };
        reference_map.insert(
            "plugin".to_string(),
            serde_json::Value::String(resolved_plugin.to_string()),
        );
        reference_map.insert(
            "object_id".to_string(),
            serde_json::Value::String(format!("{object_id:06X}")),
        );
        false
    } else if index == 0xFF {
        reference_map.insert(
            "object_id".to_string(),
            serde_json::Value::String(format!("{object_id:06X}")),
        );
        false
    } else {
        reference_map.insert(
            "object_id".to_string(),
            serde_json::Value::String(format!("{object_id:06X}")),
        );
        reference_map.insert(
            "missing_index".to_string(),
            serde_json::Value::Number(index.into()),
        );
        true
    };

    let mut map = serde_json::Map::new();
    map.insert(
        "reference".to_string(),
        serde_json::Value::Object(reference_map),
    );
    if missing_index {
        map.insert(
            "raw".to_string(),
            serde_json::Value::String(format!("{raw_value:08X}")),
        );
    }
    let _ = target;
    serde_json::Value::Object(map)
}

/// GIL-free equivalent of enum_payload_impl (using SchemaEnumJson from compiled schema).
fn enum_payload_to_json(enum_def: &SchemaEnumJson, value: i128) -> serde_json::Value {
    if enum_def.is_bool_enum() {
        if value == 0 {
            return serde_json::Value::Bool(false);
        }
        if value == 1 {
            return serde_json::Value::Bool(true);
        }
    }
    if enum_def.is_flags() {
        return flags_payload_to_json(enum_def, value);
    }
    if let Some(display) = enum_def.display_for_value(value) {
        return serde_json::Value::String(authoring_camel_case(display));
    }
    enum_verbose_payload_to_json(enum_def, value)
}

fn integer_payload_to_json(value: i128) -> serde_json::Value {
    if let Ok(i) = i64::try_from(value) {
        serde_json::Value::Number(i.into())
    } else if let Ok(u) = u64::try_from(value) {
        serde_json::Value::Number(u.into())
    } else {
        serde_json::Value::Null
    }
}

fn flags_payload_to_json(enum_def: &SchemaEnumJson, value: i128) -> serde_json::Value {
    if value < 0 {
        return enum_verbose_payload_to_json(enum_def, value);
    }
    let mut remaining = value;
    let mut entries: Vec<serde_json::Value> = Vec::new();
    let mut flags: Vec<(i128, String)> = enum_def
        .values
        .iter()
        .filter(|entry| entry.value > 0)
        .filter_map(|entry| {
            let label = enum_def
                .display_for_value(entry.value)
                .map(authoring_camel_case)
                .filter(|label| !label.is_empty())
                .or_else(|| unknown_flag_label_for_value(entry.value))?;
            Some((entry.value, label))
        })
        .collect();
    flags.sort_by_key(|(flag, _)| *flag);

    for (flag, label) in flags {
        if remaining & flag == flag {
            entries.push(serde_json::Value::String(label));
            remaining &= !flag;
        }
    }

    if remaining != 0 {
        entries.push(integer_payload_to_json(remaining));
    }
    serde_json::Value::Array(entries)
}

fn unknown_flag_label_for_value(value: i128) -> Option<String> {
    if value <= 0 || value.count_ones() != 1 {
        return None;
    }
    Some(format!("Unknown{}", value.trailing_zeros()))
}

fn enum_verbose_payload_to_json(enum_def: &SchemaEnumJson, value: i128) -> serde_json::Value {
    let token = enum_def.token_for_value(value);
    let label = enum_def.label_for_value(value);
    let mut map = serde_json::Map::new();
    // value as i64 / u64 — match Python output (avoids i128 which serde_json doesn't support directly).
    map.insert("value".to_string(), integer_payload_to_json(value));
    if let Some(t) = token {
        map.insert(
            "token".to_string(),
            serde_json::Value::String(t.to_string()),
        );
    }
    if let Some(l) = label.or(token) {
        map.insert(
            "label".to_string(),
            serde_json::Value::String(l.to_string()),
        );
    }
    map.insert(
        "enum".to_string(),
        serde_json::Value::String(enum_def.id.clone()),
    );
    map.insert(
        "scope".to_string(),
        serde_json::Value::String(enum_def.scope.clone()),
    );
    if enum_def.storage_kind != "enum" {
        map.insert(
            "storage_kind".to_string(),
            serde_json::Value::String(enum_def.storage_kind.clone()),
        );
    }
    serde_json::Value::Object(map)
}

/// GIL-free equivalent of localized_value_payload_native.
fn localized_value_payload_to_json(
    strings: &LocalizedStringsState,
    string_id: u32,
    raw_hex: &str,
    semantic_type: Option<&str>,
) -> serde_json::Value {
    if string_id == 0 {
        // Null lstring — emit a structural marker (TargetLanguage only) so the
        // subrecord round-trips through the encoder, but skip raw_hex because
        // the four zero bytes carry no value. The encoder reaches the
        // `values_by_language.is_empty()` branch and emits string_id = 0.
        let mut map = serde_json::Map::new();
        map.insert(
            "TargetLanguage".to_string(),
            serde_json::Value::String("English".to_string()),
        );
        if let Some(v) = semantic_type {
            map.insert(
                "semantic_type".to_string(),
                serde_json::Value::String(v.to_string()),
            );
        }
        return serde_json::Value::Object(map);
    }
    let mut codes: Vec<&str> = strings.by_language.keys().map(|s| s.as_str()).collect();
    codes.sort_unstable();

    let mut entries: Vec<(String, String)> = Vec::new();
    for code in &codes {
        if let Some(table) = strings.by_language.get(*code) {
            if let Some(text) = table.get(&string_id) {
                entries.push((language_display_name(code).to_string(), text.clone()));
            }
        }
    }

    if entries.is_empty() {
        return raw_only_payload_json(raw_hex, semantic_type, None);
    }

    let mut ordered: Vec<(String, String)> = Vec::with_capacity(entries.len());
    for wanted in LANGUAGE_DISPLAY_ORDER {
        if let Some(index) = entries.iter().position(|(name, _)| name == wanted) {
            ordered.push(entries.remove(index));
        }
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    ordered.extend(entries);

    let target_language = if ordered.iter().any(|(name, _)| name == "English") {
        "English".to_string()
    } else {
        ordered
            .first()
            .map(|(name, _)| name.clone())
            .unwrap_or_else(|| "English".to_string())
    };

    // Emit the resolved value(s) inline. No raw_hex when the string resolves —
    // the YAML is human-readable, and on round-trip the encoder allocates a
    // fresh string ID (semantic ESP equivalence; the original Bethesda ID is
    // not preserved). Authoring workflows (creating/editing mods) get clean
    // text-driven YAML; for byte-exact preservation of an existing localized
    // plugin's IDs, use the lossless JSON export instead of authoring-dir.
    //
    // Single-language case → `Value: <text>` (cleaner). Multi-language case →
    // `Values: [{Language, String}, ...]` so all translations are visible.
    let _ = raw_hex;
    let mut map = serde_json::Map::new();
    map.insert(
        "TargetLanguage".to_string(),
        serde_json::Value::String(target_language.clone()),
    );
    if ordered.len() == 1 {
        let (_, text) = ordered.into_iter().next().expect("len==1 checked");
        map.insert("Value".to_string(), serde_json::Value::String(text));
    } else {
        let values: serde_json::Value = serde_json::Value::Array(
            ordered
                .into_iter()
                .map(|(language, text)| {
                    let mut row = serde_json::Map::new();
                    row.insert("Language".to_string(), serde_json::Value::String(language));
                    row.insert("String".to_string(), serde_json::Value::String(text));
                    serde_json::Value::Object(row)
                })
                .collect(),
        );
        map.insert("Values".to_string(), values);
    }
    if let Some(v) = semantic_type {
        map.insert(
            "semantic_type".to_string(),
            serde_json::Value::String(v.to_string()),
        );
    }
    serde_json::Value::Object(map)
}

fn wrap_compact_json(
    preservation_is_typed: bool,
    value: serde_json::Value,
    raw_hex: &str,
    semantic_type: Option<&str>,
) -> serde_json::Value {
    if preservation_is_typed {
        return value;
    }
    let mut map = serde_json::Map::new();
    map.insert("value".to_string(), value);
    map.insert(
        "raw_hex".to_string(),
        serde_json::Value::String(raw_hex.to_string()),
    );
    if let Some(v) = semantic_type {
        map.insert(
            "semantic_type".to_string(),
            serde_json::Value::String(v.to_string()),
        );
    }
    serde_json::Value::Object(map)
}

fn json_value_as_integer(v: &serde_json::Value) -> Option<i128> {
    match v {
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Some(i as i128)
            } else if let Some(u) = n.as_u64() {
                Some(u as i128)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Serialize a single field value in a struct/union to JSON.
/// Mirrors serialize_field_simple / serialize_mapping / compact_union_value logic.
fn serialize_field_value_json(
    value: &serde_json::Value,
    field: &SchemaFieldJson,
    schema: &CompiledSchema,
    masters: &[String],
    plugin_name: &str,
) -> serde_json::Value {
    if value.is_null() {
        return serde_json::Value::Null;
    }
    // Union variants
    if !field.union_variants.is_empty() {
        if let serde_json::Value::Object(m) = value {
            if let Some(serde_json::Value::String(variant_name)) = m.get("variant") {
                let mut out = serde_json::Map::new();
                out.insert(
                    "variant".to_string(),
                    serde_json::Value::String(variant_name.clone()),
                );
                if let Some(variant) = field.union_variants.iter().find(|v| &v.id == variant_name) {
                    if let Some(inner_val) = m.get("value") {
                        if !variant.fields.is_empty() {
                            let serialized = serialize_struct_value_json(
                                inner_val,
                                &variant.fields,
                                schema,
                                masters,
                                plugin_name,
                            );
                            out.insert("value".to_string(), serialized);
                        } else {
                            out.insert("value".to_string(), inner_val.clone());
                        }
                    }
                } else if let Some(inner_val) = m.get("value") {
                    out.insert("value".to_string(), inner_val.clone());
                }
                return serde_json::Value::Object(out);
            }
        }
    }
    // Nested struct fields
    if !field.fields.is_empty() {
        return serialize_struct_value_json(value, &field.fields, schema, masters, plugin_name);
    }
    // Array
    if field.array.is_some() {
        if let serde_json::Value::Array(items) = value {
            if field.kind == "formid" {
                let serialized: Vec<serde_json::Value> = items
                    .iter()
                    .filter_map(|item| {
                        let raw = json_value_as_integer(item)?;
                        if raw >= 0 && raw <= u32::MAX as i128 {
                            let fv = serialize_formid_to_json(
                                raw as u32,
                                plugin_name,
                                masters,
                                field.formlink_target.as_deref(),
                            );
                            if fv.is_null() { None } else { Some(fv) }
                        } else {
                            None
                        }
                    })
                    .collect();
                return serde_json::Value::Array(serialized);
            }
            return value.clone();
        }
    }
    // Enum
    if let Some(enum_ref) = field.enum_ref.as_deref() {
        if let Some(enum_def) = schema.enums.get(enum_ref) {
            if let Some(int_val) = json_value_as_integer(value) {
                return enum_payload_to_json(enum_def, int_val);
            }
        }
    }
    // FormId. Null FormID (raw=0) emits JSON null — the encoder accepts null
    // and re-encodes to four zero bytes (encode_form_reference_json:5695).
    if field.kind == "formid" {
        if let Some(raw) = json_value_as_integer(value) {
            if raw >= 0 && raw <= u32::MAX as i128 {
                return serialize_formid_to_json(
                    raw as u32,
                    plugin_name,
                    masters,
                    field.formlink_target.as_deref(),
                );
            }
        }
    }
    value.clone()
}

fn struct_count_field_names(fields: &[SchemaFieldJson]) -> std::collections::HashSet<&str> {
    fields
        .iter()
        .filter_map(|field| field.array.as_ref()?.count_field.as_deref())
        .collect()
}

fn json_value_is_zero(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Number(number) => {
            number.as_i64() == Some(0)
                || number.as_u64() == Some(0)
                || number.as_f64().is_some_and(|value| value == 0.0)
        }
        _ => false,
    }
}

fn authoring_field_is_default_json(
    raw_value: &serde_json::Value,
    serialized: &serde_json::Value,
    field: &SchemaFieldJson,
) -> bool {
    if raw_value.is_null() || serialized.is_null() {
        return true;
    }
    if field.array.is_some() {
        return matches!(raw_value, serde_json::Value::Array(values) if values.is_empty())
            || matches!(serialized, serde_json::Value::Array(values) if values.is_empty());
    }
    if let Some(default) = &field.default_value {
        if json_values_numerically_equal(raw_value, default) {
            return true;
        }
        return false;
    }
    if json_value_is_zero(raw_value) {
        return true;
    }
    match raw_value {
        serde_json::Value::String(value) => value.is_empty(),
        serde_json::Value::Bool(value) => !*value,
        _ => false,
    }
}

fn json_values_numerically_equal(a: &serde_json::Value, b: &serde_json::Value) -> bool {
    match (a, b) {
        (serde_json::Value::Number(x), serde_json::Value::Number(y)) => {
            if let (Some(xi), Some(yi)) = (x.as_i64(), y.as_i64()) {
                return xi == yi;
            }
            if let (Some(xu), Some(yu)) = (x.as_u64(), y.as_u64()) {
                return xu == yu;
            }
            if let (Some(xf), Some(yf)) = (x.as_f64(), y.as_f64()) {
                return xf == yf;
            }
            false
        }
        _ => a == b,
    }
}

fn serialize_struct_value_json(
    value: &serde_json::Value,
    fields: &[SchemaFieldJson],
    schema: &CompiledSchema,
    masters: &[String],
    plugin_name: &str,
) -> serde_json::Value {
    match value {
        serde_json::Value::Object(m) => {
            let mut out = serde_json::Map::new();
            let count_fields = struct_count_field_names(fields);
            for field in fields {
                if count_fields.contains(field.id.as_str()) {
                    continue;
                }
                let key = authoring_key_name(field.display_label.as_deref(), field.id.as_str());
                let raw_val = m.get(field.id.as_str()).unwrap_or(&serde_json::Value::Null);
                let serialized =
                    serialize_field_value_json(raw_val, field, schema, masters, plugin_name);
                if !authoring_field_is_default_json(raw_val, &serialized, field) {
                    out.insert(key, serialized);
                }
            }
            serde_json::Value::Object(out)
        }
        serde_json::Value::Array(rows) => {
            let out: Vec<serde_json::Value> = rows
                .iter()
                .map(|row| serialize_struct_value_json(row, fields, schema, masters, plugin_name))
                .collect();
            serde_json::Value::Array(out)
        }
        _ => value.clone(),
    }
}

fn ctda_param_key_name(key: CtdaParamKey) -> &'static str {
    match key {
        CtdaParamKey::ParameterOneRecord => "ParameterOneRecord",
        CtdaParamKey::FirstParameter => "FirstParameter",
    }
}

fn enrich_ctda_authoring_payload_json(
    compact: &mut serde_json::Value,
    data: &[u8],
    masters: &[String],
    plugin_name: &str,
) {
    if data.len() < 16 {
        return;
    }
    let Some(function_id) = read_u16_le_json(data, 8) else {
        return;
    };
    let game = infer_game_from_plugins(masters.iter(), plugin_name);
    let Some(meta) = lookup_ctda_function(game, function_id) else {
        return;
    };
    let serde_json::Value::Object(map) = compact else {
        return;
    };

    map.insert(
        "FunctionName".to_string(),
        serde_json::Value::String(meta.name.to_string()),
    );

    let Some(parameter_key) = meta.parameter_one_formkey else {
        return;
    };
    let Some(raw_parameter) = read_u32_le_json(data, 12) else {
        return;
    };
    if raw_parameter == 0 {
        return;
    }
    map.insert(
        ctda_param_key_name(parameter_key).to_string(),
        serialize_formid_to_json(raw_parameter, plugin_name, masters, None),
    );
    if parameter_key == CtdaParamKey::ParameterOneRecord {
        map.insert(
            "ParameterOneNumber".to_string(),
            serde_json::Value::Number(raw_parameter.into()),
        );
    }
}

fn schema_field_codec_for_token_json(
    field: &SchemaFieldJson,
    token: &str,
) -> Option<crate::FieldCodec> {
    let scalar_codec = match field.kind.as_str() {
        "int8" | "uint8" | "int16" | "uint16" | "int32" | "uint32" | "int64" | "uint64"
        | "float32" | "formid" => field.kind.clone(),
        "enum" | "flags" => match token {
            "b" => "int8".to_string(),
            "B" => "uint8".to_string(),
            "h" => "int16".to_string(),
            "H" => "uint16".to_string(),
            "i" => "int32".to_string(),
            "I" => "uint32".to_string(),
            "q" => "int64".to_string(),
            "Q" => "uint64".to_string(),
            _ => return None,
        },
        "fixed_string" => format!("fixed_string:{}", token_width_rs(token)?),
        _ => return None,
    };
    field_codec_from_str(scalar_codec.as_str())
}

fn read_schema_field_token_json(
    data: &[u8],
    offset: &mut usize,
    field: &SchemaFieldJson,
    token: &str,
) -> Option<serde_json::Value> {
    let size = token_width_rs(token)?;
    let end = offset.checked_add(size)?;
    let slice = data.get(*offset..end)?;
    *offset = end;
    Some(decode_scalar_codec_json(
        &schema_field_codec_for_token_json(field, token)?,
        slice,
    ))
}

fn read_implicit_array_count_json(data: &[u8], offset: &mut usize) -> Option<usize> {
    let value = *data.get(*offset)? as usize;
    *offset += 1;
    Some(value)
}

fn read_array_count_codec_json(data: &[u8], offset: &mut usize, codec: &str) -> Option<usize> {
    let value = match codec {
        "payload_div_76" => {
            if data.len() % 76 != 0 {
                return None;
            }
            data.len() / 76
        }
        "uint8" | "B" => {
            let value = *data.get(*offset)? as usize;
            *offset += 1;
            value
        }
        "uint16" | "H" => {
            let end = offset.checked_add(2)?;
            let bytes = data.get(*offset..end)?;
            *offset = end;
            u16::from_le_bytes([bytes[0], bytes[1]]) as usize
        }
        "uint32" | "I" => {
            let end = offset.checked_add(4)?;
            let bytes = data.get(*offset..end)?;
            *offset = end;
            u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize
        }
        _ => return None,
    };
    Some(value)
}

fn apply_array_count_transform_json(count: usize, transform: Option<&str>) -> Option<usize> {
    match transform {
        None => Some(count),
        Some("square") => count.checked_mul(count),
        Some(_) => None,
    }
}

fn schema_array_count_json(
    array: &SchemaArrayJson,
    decoded: &serde_json::Map<String, serde_json::Value>,
    context: Option<&std::collections::HashMap<String, serde_json::Value>>,
) -> Option<Option<usize>> {
    if let Some(count_record_field) = array.count_record_field.as_deref() {
        // Cross-subrecord count from record-level context. Missing key → 0:
        // default-elision in the prior subrecord's compact serialization
        // (e.g. XCNT.SwimmingCount=0 stripped from the emitted object) means
        // the merged record context never received the key. Treat as zero
        // rather than failing the decode.
        let ctx = context?;
        let count = ctx
            .get(count_record_field)
            .and_then(json_value_as_integer)
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or(0);
        let count = apply_array_count_transform_json(count, array.count_transform.as_deref())?;
        return Some(Some(count));
    }
    let Some(count_field) = array.count_field.as_deref() else {
        return Some(None);
    };
    let value = decoded.get(count_field)?;
    let count = json_value_as_integer(value)
        .and_then(|value| usize::try_from(value).ok())
        .and_then(|count| {
            apply_array_count_transform_json(count, array.count_transform.as_deref())
        })?;
    Some(Some(count))
}

fn decode_schema_array_elements_json(
    data: &[u8],
    offset: &mut usize,
    field: &SchemaFieldJson,
    element_codec: &str,
    count: usize,
    context: Option<&std::collections::HashMap<String, serde_json::Value>>,
) -> Option<serde_json::Value> {
    let row_codec = format!("struct:{element_codec}");
    let tokens = struct_tokens_rs(row_codec.as_str());
    if tokens.is_empty() {
        return if count == 0 {
            Some(serde_json::Value::Array(Vec::new()))
        } else {
            None
        };
    }
    let row_size: usize = tokens
        .iter()
        .filter_map(|token| token_width_rs(token))
        .sum();
    let remaining = data.len().saturating_sub(*offset);
    if row_size == 0 || count > remaining / row_size {
        trace_schema_array_bounds_mismatch(
            field.id.as_str(),
            element_codec,
            count,
            row_size,
            *offset,
            data.len(),
            remaining,
        );
        return None;
    }
    let mut values = Vec::with_capacity(count);
    if field.fields.is_empty() {
        if tokens.len() != 1 {
            return None;
        }
        for _ in 0..count {
            values.push(read_schema_field_token_json(
                data, offset, field, tokens[0],
            )?);
        }
        return Some(serde_json::Value::Array(values));
    }

    for _ in 0..count {
        values.push(decode_schema_struct_with_arrays_partial_json(
            data,
            offset,
            row_codec.as_str(),
            &field.fields,
            false,
            context,
        )?);
    }
    Some(serde_json::Value::Array(values))
}

thread_local! {
    static EXPORT_DECODE_CONTEXT: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

pub fn export_decode_diagnostics_enabled() -> bool {
    std::env::var("MODBOX21_NATIVE_EXPORT_TRACE")
        .ok()
        .map(|value| {
            let trimmed = value.trim();
            !trimmed.is_empty()
                && !trimmed.eq_ignore_ascii_case("0")
                && !trimmed.eq_ignore_ascii_case("false")
                && !trimmed.eq_ignore_ascii_case("off")
        })
        .unwrap_or(false)
}

struct ExportDecodeContextGuard;

impl Drop for ExportDecodeContextGuard {
    fn drop(&mut self) {
        EXPORT_DECODE_CONTEXT.with(|context| {
            *context.borrow_mut() = None;
        });
    }
}

fn set_export_decode_context(context: String) -> ExportDecodeContextGuard {
    EXPORT_DECODE_CONTEXT.with(|slot| {
        *slot.borrow_mut() = Some(context);
    });
    ExportDecodeContextGuard
}

fn current_export_decode_context() -> Option<String> {
    EXPORT_DECODE_CONTEXT.with(|context| context.borrow().clone())
}

fn trace_schema_array_bounds_mismatch(
    field_id: &str,
    element_codec: &str,
    count: usize,
    row_size: usize,
    offset: usize,
    data_len: usize,
    remaining: usize,
) {
    if !export_decode_diagnostics_enabled() {
        return;
    }
    use std::sync::atomic::{AtomicUsize, Ordering};

    static WARNINGS: AtomicUsize = AtomicUsize::new(0);
    let index = WARNINGS.fetch_add(1, Ordering::Relaxed);
    if index >= 100 {
        return;
    }
    if let Some(context) = current_export_decode_context() {
        eprintln!(
            "[creation_lib::_native::esp_export] schema array count exceeds available subrecord bytes: {context} field={field_id} element_codec={element_codec} count={count} row_size={row_size} offset={offset} data_len={data_len} remaining={remaining}"
        );
    } else {
        eprintln!(
            "[creation_lib::_native::esp_export] schema array count exceeds available subrecord bytes: field={field_id} element_codec={element_codec} count={count} row_size={row_size} offset={offset} data_len={data_len} remaining={remaining}"
        );
    }
    if index == 99 {
        eprintln!(
            "[creation_lib::_native::esp_export] suppressing further schema array bounds warnings"
        );
    }
}

fn decode_schema_struct_with_arrays_partial_json(
    data: &[u8],
    offset: &mut usize,
    codec: &str,
    fields: &[SchemaFieldJson],
    require_eof: bool,
    context: Option<&std::collections::HashMap<String, serde_json::Value>>,
) -> Option<serde_json::Value> {
    let tokens = struct_tokens_rs(codec);
    // Empty token list is valid when every field is an array (e.g. FSTS.DATA's
    // codec='struct:' wraps only sibling-counted formid arrays). Reject only
    // when fields would consume struct tokens but none are available.
    if tokens.is_empty()
        && fields
            .iter()
            .any(|field| field.array.is_none() && field.kind != "empty")
    {
        return None;
    }
    let mut token_index = 0usize;
    let mut decoded = serde_json::Map::new();

    for field in fields {
        while token_index < tokens.len() && tokens[token_index] == "x" {
            *offset = offset.checked_add(token_width_rs(tokens[token_index])?)?;
            token_index += 1;
        }
        if field.kind == "empty" {
            continue;
        }
        if let Some(array) = field.array.as_ref() {
            let count = match schema_array_count_json(array, &decoded, context)? {
                Some(value) => value,
                None => {
                    if let Some(count_codec) = array.count_codec.as_deref() {
                        let count = read_array_count_codec_json(data, offset, count_codec)?;
                        apply_array_count_transform_json(count, array.count_transform.as_deref())?
                    } else if let Some(element_codec) = array.element_codec.as_deref() {
                        // No explicit count info — derive count from the
                        // remaining payload assuming fixed-size elements.
                        // xEdit's ``wbArrayS(label, struct, 0, ...)`` pattern
                        // (LCTN.{ACEC,LCEC,RCEC} cells, etc.) has no count
                        // prefix; the array fills the rest of the subrecord
                        // bytes. Falls back to a 1-byte implicit count only
                        // when the element size can't be computed.
                        let row_size = parse_array_element_size(element_codec).unwrap_or(0);
                        let remaining = data.len().saturating_sub(*offset);
                        if row_size > 0 {
                            apply_array_count_transform_json(
                                remaining / row_size,
                                array.count_transform.as_deref(),
                            )?
                        } else {
                            let count = read_implicit_array_count_json(data, offset)?;
                            apply_array_count_transform_json(
                                count,
                                array.count_transform.as_deref(),
                            )?
                        }
                    } else {
                        let count = read_implicit_array_count_json(data, offset)?;
                        apply_array_count_transform_json(count, array.count_transform.as_deref())?
                    }
                }
            };
            let element_codec = array.element_codec.as_deref()?;
            let value = decode_schema_array_elements_json(
                data,
                offset,
                field,
                element_codec,
                count,
                context,
            )?;
            decoded.insert(field.id.clone(), value);
            continue;
        }
        let token = *tokens.get(token_index)?;
        if !field.union_variants.is_empty() {
            let size = token_width_rs(token)?;
            let end = offset.checked_add(size)?;
            let slice = data.get(*offset..end)?;
            let mut union_ctx = context.cloned().unwrap_or_default();
            for (key, value) in &decoded {
                union_ctx
                    .entry(key.clone())
                    .or_insert_with(|| value.clone());
            }
            let union_spec =
                decode_spec_for_union_rs(field.id.as_str(), &field.union_variants, false)?;
            let crate::DecodeSpec::Union { variants } = union_spec else {
                return None;
            };
            let value = decode_union_json(slice, &variants, &union_ctx)?;
            decoded.insert(field.id.clone(), value);
            *offset = end;
            token_index += 1;
            continue;
        }
        let value = read_schema_field_token_json(data, offset, field, token)?;
        decoded.insert(field.id.clone(), value);
        token_index += 1;
    }
    while token_index < tokens.len() && tokens[token_index] == "x" {
        *offset = offset.checked_add(token_width_rs(tokens[token_index])?)?;
        token_index += 1;
    }
    if token_index != tokens.len() || (require_eof && *offset != data.len()) {
        return None;
    }
    Some(serde_json::Value::Object(decoded))
}

fn decode_schema_struct_with_arrays_json(
    data: &[u8],
    spec: &SchemaSubrecordJson,
    schema: &CompiledSchema,
    masters: &[String],
    plugin_name: &str,
    context: Option<&std::collections::HashMap<String, serde_json::Value>>,
) -> Option<serde_json::Value> {
    let codec = spec.codec.as_deref()?;
    let mut offset = 0usize;
    let decoded = decode_schema_struct_with_arrays_partial_json(
        data,
        &mut offset,
        codec,
        &spec.fields,
        true,
        context,
    )?;
    Some(serialize_struct_value_json(
        &decoded,
        &spec.fields,
        schema,
        masters,
        plugin_name,
    ))
}

/// GIL-free equivalent of compact_typed_value, returning serde_json::Value.
fn compact_typed_value_json(
    decoded: serde_json::Value,
    spec: &SchemaSubrecordJson,
    schema: &CompiledSchema,
    data: &[u8],
    semantic_type: Option<&str>,
    strings: &LocalizedStringsState,
    masters: &[String],
    plugin_name: &str,
) -> serde_json::Value {
    let preservation_is_typed = spec.kind == "parsed";
    let raw_hex = hex::encode_upper(data);

    // Empty marker subrecords carry meaning by presence. Emit a compact
    // presence value instead of an empty/null object.
    if spec.codec.as_deref() == Some("empty") {
        return wrap_compact_json(
            preservation_is_typed,
            serde_json::Value::Bool(true),
            raw_hex.as_str(),
            semantic_type,
        );
    }

    // Localized path.
    if spec.localized {
        if let Some(int_val) = json_value_as_integer(&decoded) {
            if int_val >= 0 && int_val <= u32::MAX as i128 {
                return localized_value_payload_to_json(
                    strings,
                    int_val as u32,
                    raw_hex.as_str(),
                    semantic_type,
                );
            }
        }
    }

    // Enum path (top-level enum_ref on the subrecord spec).
    if let Some(enum_ref) = spec.enum_ref.as_deref() {
        if let Some(enum_def) = schema.enums.get(enum_ref) {
            if let Some(int_val) = json_value_as_integer(&decoded) {
                let payload = enum_payload_to_json(enum_def, int_val);
                return wrap_compact_json(
                    preservation_is_typed,
                    payload,
                    raw_hex.as_str(),
                    semantic_type,
                );
            }
        }
    }

    // FormId path. Null FormID (raw=0) emits explicit JSON null — the encoder
    // accepts JSON null and re-encodes to four zero bytes (see
    // `encode_form_reference_json` in plugin_runtime.rs:5695). We never fall
    // back to raw_hex on a successful decode of a null reference.
    if spec.codec.as_deref() == Some("formid") {
        if let Some(int_val) = json_value_as_integer(&decoded) {
            if int_val >= 0 && int_val <= u32::MAX as i128 {
                let payload = serialize_formid_to_json(
                    int_val as u32,
                    plugin_name,
                    masters,
                    spec.formlink_target.as_deref(),
                );
                return wrap_compact_json(
                    preservation_is_typed,
                    payload,
                    raw_hex.as_str(),
                    semantic_type,
                );
            }
        }
    }

    // FormId array path. Preserve every slot — including NULL FormIDs (raw=0)
    // — so the array length round-trips. The encoder accepts JSON null and
    // returns 0, so emitting Value::Null for NULL refs is byte-faithful.
    if spec.codec.as_deref() == Some("formid_array") {
        if let serde_json::Value::Array(ref items) = decoded {
            let serialized: Vec<serde_json::Value> = items
                .iter()
                .map(|item| {
                    let Some(raw) = json_value_as_integer(item) else {
                        return serde_json::Value::Null;
                    };
                    if raw >= 0 && raw <= u32::MAX as i128 {
                        serialize_formid_to_json(
                            raw as u32,
                            plugin_name,
                            masters,
                            spec.formlink_target.as_deref(),
                        )
                    } else {
                        serde_json::Value::Null
                    }
                })
                .collect();
            return wrap_compact_json(
                preservation_is_typed,
                serde_json::Value::Array(serialized),
                raw_hex.as_str(),
                semantic_type,
            );
        }
    }

    // Union variants path.
    if !spec.union_variants.is_empty() {
        if let serde_json::Value::Object(ref m) = decoded {
            if let Some(serde_json::Value::String(variant_name)) = m.get("variant") {
                let mut out = serde_json::Map::new();
                out.insert(
                    "variant".to_string(),
                    serde_json::Value::String(variant_name.clone()),
                );
                if let Some(variant) = spec.union_variants.iter().find(|v| &v.id == variant_name) {
                    if let Some(inner_val) = m.get("value") {
                        if !variant.fields.is_empty() {
                            let serialized = serialize_struct_value_json(
                                inner_val,
                                &variant.fields,
                                schema,
                                masters,
                                plugin_name,
                            );
                            out.insert("value".to_string(), serialized);
                        } else {
                            out.insert("value".to_string(), inner_val.clone());
                        }
                    }
                } else if let Some(inner_val) = m.get("value") {
                    out.insert("value".to_string(), inner_val.clone());
                }
                let compact = serde_json::Value::Object(out);
                return wrap_compact_json(
                    preservation_is_typed,
                    compact,
                    raw_hex.as_str(),
                    semantic_type,
                );
            }
        }
    }

    // Struct fields path.
    if !spec.fields.is_empty() {
        let mut compact =
            serialize_struct_value_json(&decoded, &spec.fields, schema, masters, plugin_name);
        if matches!(spec.id.as_str(), "CTDA" | "CTDT") {
            enrich_ctda_authoring_payload_json(&mut compact, data, masters, plugin_name);
        }
        return wrap_compact_json(
            preservation_is_typed,
            compact,
            raw_hex.as_str(),
            semantic_type,
        );
    }

    wrap_compact_json(
        preservation_is_typed,
        decoded,
        raw_hex.as_str(),
        semantic_type,
    )
}

/// GIL-free compact subrecord serialization (Step 2).
///
/// Mirrors the legacy schema-driven authoring serializer but returns
/// `serde_json::Value` and requires no `py: Python<'_>` parameter.
///
/// `spec` is the pre-computed `DecodeSpec` from `schema_subrecord_to_decode_spec`
/// (or `None` when no schema is available — falls back to raw-hex).
///
/// `context` carries the per-record conditional-decode state built from
/// already-processed fields (record_signature, record_form_version, etc.)
/// plus the content of any preceding subrecords that the union conditions
/// need to inspect.
pub fn compact_subrecord_to_json(
    data: &[u8],
    spec: Option<&crate::DecodeSpec>,
    spec_schema: Option<(&SchemaSubrecordJson, &CompiledSchema)>,
    strings: &LocalizedStringsState,
    masters: &[String],
    plugin_name: &str,
    semantic_type: Option<&str>,
    context: Option<&std::collections::HashMap<String, serde_json::Value>>,
    diagnostics_context: Option<&str>,
) -> serde_json::Value {
    let _diagnostics_guard =
        diagnostics_context.map(|context| set_export_decode_context(context.to_string()));
    let raw_hex = hex::encode_upper(data);

    if let Some((sub_spec, schema)) = spec_schema {
        if runtime_layout_for_subrecord_schema(sub_spec) == Some("vmad") {
            if let Some(decoded) =
                compact_vmad_payload_json(data, masters, plugin_name, semantic_type)
            {
                return decoded;
            }
        }
        if sub_spec.kind == "raw" && sub_spec.id == "LVLO" {
            if let Some(decoded) =
                compact_lvlo_entry_payload_json(data, masters, plugin_name, semantic_type)
            {
                return decoded;
            }
        }
        if sub_spec
            .codec
            .as_deref()
            .is_some_and(|codec| codec.starts_with("struct:"))
            && sub_spec.fields.iter().any(|field| field.array.is_some())
        {
            if let Some(decoded) = decode_schema_struct_with_arrays_json(
                data,
                sub_spec,
                schema,
                masters,
                plugin_name,
                context,
            ) {
                return decoded;
            }
        }
        if sub_spec.codec.as_deref() == Some("omod_data") {
            if let Some(decoded) =
                compact_omod_data_payload_json(data, sub_spec, schema, masters, plugin_name)
            {
                return decoded;
            }
        }
        if sub_spec.codec.as_deref() == Some("model_info") {
            if let Some(decoded) =
                compact_model_info_payload_json(data, sub_spec, schema, masters, plugin_name)
            {
                return decoded;
            }
        }
    }

    // `custom_codec` delegates parse/write to a named external Rust module
    // (e.g. esp_authoring_core::nvnm). schema_subrecord_to_decode_spec returns
    // None for custom_codec so we must intercept here, BEFORE the "no
    // decode_spec → raw-only" early exit. NVNM and the LAND heightmap codec
    // hook in here so the structured fields ride alongside raw_hex; any
    // other custom codec falls through to raw-only until wired here.
    if let Some((sub_spec, _)) = spec_schema {
        if sub_spec.kind == "custom_codec" {
            if let Some(decoded) =
                custom_codec_payload_json(sub_spec, data, raw_hex.as_str(), semantic_type)
            {
                return decoded;
            }
            return raw_only_payload_json(raw_hex.as_str(), semantic_type, None);
        }
    }

    // No spec → raw only.
    let Some(decode_spec) = spec else {
        return raw_only_payload_json(raw_hex.as_str(), semantic_type, None);
    };
    // Raw-only subrecords (kind == "raw") and vmad layouts → raw only.
    if let Some((sub_spec, _)) = spec_schema {
        if sub_spec.kind == "raw" || runtime_layout_for_subrecord_schema(sub_spec) == Some("vmad") {
            return raw_only_payload_json(raw_hex.as_str(), semantic_type, None);
        }
    }

    // Build a working context that always carries subrecord_size, so
    // wbFromSize-style presence_conditions (e.g. PERK.DATA's playable/hidden
    // tail bytes gated on subrecord_size >= 4 / >= 5) can resolve. Cloning is
    // cheap relative to the per-subrecord decode work and keeps the caller's
    // context unchanged.
    let mut owned_ctx = match context {
        Some(c) => c.clone(),
        None => std::collections::HashMap::new(),
    };
    owned_ctx.insert(
        "subrecord_size".to_string(),
        serde_json::Value::Number(serde_json::Number::from(data.len() as u64)),
    );
    let context = &owned_ctx;

    // Decode the bytes into a structured JSON value.
    let decoded = match decode_subrecord_json(decode_spec, data, context) {
        Some(v) => v,
        None => return raw_only_payload_json(raw_hex.as_str(), semantic_type, None),
    };

    // Raw scalars that decoded to null or a wrong-length hex string → raw only.
    if decoded.is_null() || scalar_string_decode_is_raw_fallback(decode_spec, &decoded) {
        return raw_only_payload_json(raw_hex.as_str(), semantic_type, None);
    }

    // Compact / enrich the decoded value if we have schema info.
    if let Some((sub_spec, schema)) = spec_schema {
        compact_typed_value_json(
            decoded,
            sub_spec,
            schema,
            data,
            semantic_type,
            strings,
            masters,
            plugin_name,
        )
    } else {
        // No schema enrichment — wrap as typed.
        wrap_compact_json(true, decoded, raw_hex.as_str(), semantic_type)
    }
}

// -------------------------------------------------------------------------
// Walker FormID extraction — schema-aware nested FK extractor used by the
// per-plugin refs index (plugin_index.rs::build_refs_section). Catches
// FormIDs that aren't exposed as flat top-level `formid`/`formid_array`
// subrecords: WEAP/ARMO OBTS includes[].mod (→ OMOD), OMOD properties,
// RACE/PERK structured payloads, etc.
// -------------------------------------------------------------------------

/// Walk a subrecord's schema spec + raw bytes, emitting any FormID values
/// nested inside compound layouts. The caller is expected to have already
/// handled flat top-level formid/formid_array signatures via the legacy
/// heuristic; this helper covers everything else.
///
/// Skips raw subrecords, VMAD layouts, and signatures whose schema has no
/// FormID field anywhere (cheap pre-check before invoking the decoder).
pub fn extract_nested_form_ids(
    sub_spec: &SchemaSubrecordJson,
    schema: &CompiledSchema,
    data: &[u8],
    out: &mut Vec<u32>,
) {
    if sub_spec.kind == "raw" {
        return;
    }
    if sub_spec.kind == "custom_codec" {
        extract_custom_codec_form_ids(sub_spec, data, out);
        return;
    }
    if runtime_layout_for_subrecord_schema(sub_spec) == Some("vmad") {
        return;
    }
    // Top-level scalar/array FormID subrecords (e.g. WEAP BIDS->IPDS,
    // AnimationSound, PreviewTransform). The decode+walk path below only
    // collects FormIDs nested inside struct/array fields, so a subrecord whose
    // payload *is* a single FormID would be dropped — leaving the dependency
    // walk unable to reach those records. Mirror the top-level handling in
    // `rewrite_schema_form_ids_in_subrecord`.
    match sub_spec.codec.as_deref() {
        Some("formid") => {
            if data.len() >= 4 {
                out.push(u32::from_le_bytes([data[0], data[1], data[2], data[3]]));
            }
            return;
        }
        Some("formid_array") => {
            for chunk in data.chunks_exact(4) {
                out.push(u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
            }
            return;
        }
        _ => {}
    }
    if !subrecord_schema_has_formid(sub_spec) {
        return;
    }
    let Some(decoded) = decode_subrecord_for_walker(sub_spec, schema, data) else {
        return;
    };
    if !sub_spec.union_variants.is_empty() {
        if let serde_json::Value::Object(map) = &decoded {
            if let (Some(serde_json::Value::String(name)), Some(inner)) =
                (map.get("variant"), map.get("value"))
            {
                if let Some(variant) = sub_spec.union_variants.iter().find(|v| &v.id == name) {
                    walker_collect_formids_from_fields(&variant.fields, inner, out);
                }
            }
        }
    } else {
        walker_collect_formids_from_fields(&sub_spec.fields, &decoded, out);
    }
}

/// Rewrite schema-declared FormIDs inside a raw subrecord payload in place.
///
/// This is the mutating counterpart to [`extract_nested_form_ids`]. It handles
/// top-level `formid` / `formid_array`, fixed `struct:*`, fixed
/// `array_struct:*`, and simple count-prefixed row-array fields. Callers
/// provide the policy for a raw FormID: return `Some(new_raw)` to rewrite, or
/// `None` to leave it unchanged.
pub fn rewrite_schema_form_ids_in_subrecord(
    sub_spec: &SchemaSubrecordJson,
    _schema: &CompiledSchema,
    data: &mut [u8],
    rewrite_formid: &mut dyn FnMut(u32) -> Option<u32>,
) -> bool {
    if sub_spec.kind == "raw" {
        return false;
    }
    if sub_spec.kind == "custom_codec" {
        return rewrite_custom_codec_form_ids(sub_spec, data, rewrite_formid);
    }
    if runtime_layout_for_subrecord_schema(sub_spec) == Some("vmad") {
        return false;
    }

    let Some(codec) = sub_spec.codec.as_deref() else {
        return false;
    };
    match codec {
        "formid" => rewrite_raw_formid_at(data, 0, rewrite_formid),
        "formid_array" => rewrite_raw_formid_array(data, rewrite_formid),
        "omod_data" => false,
        "model_info" => false,
        other if other.starts_with("array_struct:") => {
            rewrite_array_struct_form_ids(other, &sub_spec.fields, data, rewrite_formid)
        }
        other if other.starts_with("struct:") => {
            if sub_spec.fields.iter().any(|field| field.array.is_some()) {
                rewrite_struct_with_arrays_form_ids(other, &sub_spec.fields, data, rewrite_formid)
            } else {
                rewrite_fixed_struct_row_form_ids(other, &sub_spec.fields, data, 0, rewrite_formid)
            }
        }
        _ => false,
    }
}

/// FormID extractor for `custom_codec` subrecords. Today this covers NVNM
/// (door_refs[].door_ref_form_id + the Interior.cell / Exterior.world parent
/// pointer); LAND VHGT/VNML carry no FormIDs. Without this, `plugin_index`
/// misses every door portal target and master-shuffle leaves them at the
/// source plugin's master index.
fn extract_custom_codec_form_ids(sub_spec: &SchemaSubrecordJson, data: &[u8], out: &mut Vec<u32>) {
    let Some(codec) = sub_spec.codec.as_deref() else {
        return;
    };
    match codec {
        "esp_authoring_core::nvnm" => {
            if data.get(0..4) == Some(12u32.to_le_bytes().as_slice()) {
                crate::nvnm::collect_skyrim_nvnm_form_ids(data, out);
                return;
            }
            let Ok(payload) = crate::nvnm::parse_nvnm(data) else {
                return;
            };
            match payload.parent {
                crate::nvnm::NvnmParent::Interior { cell } if cell != 0 => out.push(cell),
                crate::nvnm::NvnmParent::Exterior { world, .. } if world != 0 => out.push(world),
                _ => {}
            }
            for door in &payload.door_refs {
                if door.door_ref_form_id != 0 {
                    out.push(door.door_ref_form_id);
                }
            }
        }
        // LAND heightmap codecs (VHGT/VNML) carry no FormIDs.
        "esp_authoring_core::land::heightmap" => {}
        _ => {}
    }
}

/// Inverse of [`extract_custom_codec_form_ids`]: rewrite FormIDs in place by
/// re-serialising the structured payload. Returns true if any byte changed.
fn rewrite_custom_codec_form_ids(
    sub_spec: &SchemaSubrecordJson,
    data: &mut [u8],
    rewrite_formid: &mut dyn FnMut(u32) -> Option<u32>,
) -> bool {
    let Some(codec) = sub_spec.codec.as_deref() else {
        return false;
    };
    match codec {
        "esp_authoring_core::nvnm" => {
            if data.get(0..4) == Some(12u32.to_le_bytes().as_slice()) {
                return crate::nvnm::rewrite_skyrim_nvnm_form_ids(data, rewrite_formid);
            }
            let Ok(mut payload) = crate::nvnm::parse_nvnm(data) else {
                return false;
            };
            let mut changed = false;
            match &mut payload.parent {
                crate::nvnm::NvnmParent::Interior { cell } => {
                    if *cell != 0 {
                        if let Some(new) = rewrite_formid(*cell) {
                            if new != *cell {
                                *cell = new;
                                changed = true;
                            }
                        }
                    }
                }
                crate::nvnm::NvnmParent::Exterior { world, .. } => {
                    if *world != 0 {
                        if let Some(new) = rewrite_formid(*world) {
                            if new != *world {
                                *world = new;
                                changed = true;
                            }
                        }
                    }
                }
            }
            for door in &mut payload.door_refs {
                if door.door_ref_form_id == 0 {
                    continue;
                }
                if let Some(new) = rewrite_formid(door.door_ref_form_id) {
                    if new != door.door_ref_form_id {
                        door.door_ref_form_id = new;
                        changed = true;
                    }
                }
            }
            if !changed {
                return false;
            }
            let rewritten = crate::nvnm::write_nvnm(&payload);
            // Only mutate in place when the new bytes fit the original buffer
            // exactly — the caller's `data: &mut [u8]` has fixed length. NVNM
            // byte length is fully determined by the structural counts (no
            // strings / variable widths), so a FormID-only rewrite must
            // preserve length; bail loudly if not so we don't silently
            // truncate.
            if rewritten.len() != data.len() {
                return false;
            }
            data.copy_from_slice(&rewritten);
            true
        }
        "esp_authoring_core::land::heightmap" => false,
        _ => false,
    }
}

pub fn extract_skyrim_nvmi_form_ids(data: &[u8], out: &mut Vec<u32>) {
    let Some(offsets) = skyrim_nvmi_form_id_offsets(data) else {
        return;
    };
    for offset in offsets {
        let raw = u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap());
        if raw != 0 {
            out.push(raw);
        }
    }
}

pub fn rewrite_skyrim_nvmi_form_ids(
    data: &mut [u8],
    rewrite_formid: &mut dyn FnMut(u32) -> Option<u32>,
) -> bool {
    let Some(offsets) = skyrim_nvmi_form_id_offsets(data) else {
        return false;
    };
    let mut changed = false;
    for offset in offsets {
        changed |= rewrite_raw_formid_at(data, offset, rewrite_formid);
    }
    changed
}

fn skyrim_nvmi_form_id_offsets(data: &[u8]) -> Option<Vec<usize>> {
    let mut offsets = vec![0];
    let mut offset = 24_usize;
    data.get(0..offset)?;

    for _ in 0..2 {
        let count = read_u32_le_json(data, offset)? as usize;
        offset = offset.checked_add(4)?;
        for _ in 0..count {
            offsets.push(offset);
            offset = offset.checked_add(4)?;
            data.get(0..offset)?;
        }
    }

    let door_count = read_u32_le_json(data, offset)? as usize;
    offset = offset.checked_add(4)?;
    for _ in 0..door_count {
        let door_ref_offset = offset.checked_add(4)?;
        offsets.push(door_ref_offset);
        offset = offset.checked_add(8)?;
        data.get(0..offset)?;
    }

    let has_island = *data.get(offset)?;
    offset = offset.checked_add(1)?;
    match has_island {
        0 => {}
        1 => {
            offset = offset.checked_add(24)?;
            let triangle_count = read_u32_le_json(data, offset)? as usize;
            offset = offset
                .checked_add(4)?
                .checked_add(triangle_count.checked_mul(6)?)?;
            let vertex_count = read_u32_le_json(data, offset)? as usize;
            offset = offset
                .checked_add(4)?
                .checked_add(vertex_count.checked_mul(12)?)?;
            data.get(0..offset)?;
        }
        _ => return None,
    }

    offset = offset.checked_add(4)?;
    let parent_world_offset = offset;
    let parent_world = read_u32_le_json(data, parent_world_offset)?;
    offset = offset.checked_add(4)?;
    if parent_world == 0 {
        offsets.push(offset);
    } else {
        offsets.push(parent_world_offset);
    }
    offset = offset.checked_add(4)?;
    (offset == data.len()).then_some(offsets)
}

fn rewrite_raw_formid_at(
    data: &mut [u8],
    offset: usize,
    rewrite_formid: &mut dyn FnMut(u32) -> Option<u32>,
) -> bool {
    let Some(chunk) = data.get_mut(offset..offset.saturating_add(4)) else {
        return false;
    };
    let raw = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
    let Some(rewritten) = rewrite_formid(raw) else {
        return false;
    };
    if rewritten == raw {
        return false;
    }
    chunk.copy_from_slice(&rewritten.to_le_bytes());
    true
}

fn rewrite_raw_formid_array(
    data: &mut [u8],
    rewrite_formid: &mut dyn FnMut(u32) -> Option<u32>,
) -> bool {
    if data.len() % 4 != 0 {
        return false;
    }
    let mut changed = false;
    for offset in (0..data.len()).step_by(4) {
        changed |= rewrite_raw_formid_at(data, offset, rewrite_formid);
    }
    changed
}

fn rewrite_array_struct_form_ids(
    codec: &str,
    fields: &[SchemaFieldJson],
    data: &mut [u8],
    rewrite_formid: &mut dyn FnMut(u32) -> Option<u32>,
) -> bool {
    let row_size = struct_row_size(codec);
    if row_size == 0 || data.len() % row_size != 0 {
        return false;
    }

    let mut changed = false;
    for row_start in (0..data.len()).step_by(row_size) {
        changed |=
            rewrite_fixed_struct_row_form_ids(codec, fields, data, row_start, rewrite_formid);
    }
    changed
}

fn rewrite_fixed_struct_row_form_ids(
    codec: &str,
    fields: &[SchemaFieldJson],
    data: &mut [u8],
    row_start: usize,
    rewrite_formid: &mut dyn FnMut(u32) -> Option<u32>,
) -> bool {
    let tokens = struct_tokens_rs(codec);
    if tokens.is_empty() {
        return false;
    }

    let mut changed = false;
    let mut offset = 0usize;
    let mut token_index = 0usize;
    for field in fields {
        while token_index < tokens.len() && tokens[token_index] == "x" {
            offset += token_width_rs(tokens[token_index]).unwrap_or(0);
            token_index += 1;
        }
        if field.kind == "empty" {
            continue;
        }
        if token_index >= tokens.len() {
            break;
        }

        let token = tokens[token_index];
        let size = token_width_rs(token).unwrap_or(0);
        if size == 0 {
            return changed;
        }

        if field.kind == "formid" && size == 4 {
            changed |= rewrite_raw_formid_at(data, row_start + offset, rewrite_formid);
        }
        offset += size;
        token_index += 1;
    }
    changed
}

fn rewrite_struct_with_arrays_form_ids(
    codec: &str,
    fields: &[SchemaFieldJson],
    data: &mut [u8],
    rewrite_formid: &mut dyn FnMut(u32) -> Option<u32>,
) -> bool {
    let tokens = struct_tokens_rs(codec);
    if tokens.is_empty() {
        return false;
    }

    let mut changed = false;
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut offset = 0usize;
    let mut token_index = 0usize;

    for field in fields {
        while token_index < tokens.len() && tokens[token_index] == "x" {
            offset += token_width_rs(tokens[token_index]).unwrap_or(0);
            token_index += 1;
        }

        if let Some(array) = field.array.as_ref() {
            let Some(count) = array_row_count(array, &counts, data, &mut offset, &mut token_index)
            else {
                return changed;
            };
            changed |= rewrite_array_field_form_ids(
                field,
                array,
                count,
                data,
                &mut offset,
                rewrite_formid,
            );
            continue;
        }

        if field.kind == "empty" {
            continue;
        }
        if token_index >= tokens.len() {
            break;
        }
        let token = tokens[token_index];
        let size = token_width_rs(token).unwrap_or(0);
        if size == 0 {
            return changed;
        }
        if field.kind == "formid" && size == 4 {
            changed |= rewrite_raw_formid_at(data, offset, rewrite_formid);
        }
        if is_integer_count_kind(field.kind.as_str()) {
            if let Some(value) = read_count_value(data, offset, field.kind.as_str()) {
                counts.insert(field.id.clone(), value);
            }
        }
        offset += size;
        token_index += 1;
    }
    changed
}

fn array_row_count(
    array: &SchemaArrayJson,
    counts: &std::collections::HashMap<String, usize>,
    data: &[u8],
    offset: &mut usize,
    token_index: &mut usize,
) -> Option<usize> {
    if let Some(count_field) = array.count_field.as_deref() {
        return counts.get(count_field).copied();
    }

    let count_codec = array.count_codec.as_deref()?;
    let count = read_count_value(data, *offset, count_codec)?;
    *offset = offset.checked_add(scalar_width(count_codec)?)?;
    *token_index = token_index.checked_add(1)?;
    Some(count)
}

fn rewrite_array_field_form_ids(
    field: &SchemaFieldJson,
    array: &SchemaArrayJson,
    count: usize,
    data: &mut [u8],
    offset: &mut usize,
    rewrite_formid: &mut dyn FnMut(u32) -> Option<u32>,
) -> bool {
    let Some(element_codec) = array.element_codec.as_deref() else {
        return false;
    };
    if count == 0 {
        return false;
    }

    let mut changed = false;
    if field.kind == "formid" {
        let row_size = scalar_width(element_codec).unwrap_or(0);
        if row_size == 0 {
            return false;
        }
        for index in 0..count {
            if element_codec == "I" || element_codec == "formid" {
                changed |= rewrite_raw_formid_at(data, *offset + index * row_size, rewrite_formid);
            }
        }
        *offset = (*offset).saturating_add(count.saturating_mul(row_size));
        return changed;
    }

    if field.kind == "struct" {
        let row_size = struct_row_size(element_codec);
        if row_size == 0 {
            return false;
        }
        for index in 0..count {
            changed |= rewrite_fixed_struct_row_form_ids(
                element_codec,
                &field.fields,
                data,
                *offset + index * row_size,
                rewrite_formid,
            );
        }
        *offset = (*offset).saturating_add(count.saturating_mul(row_size));
        return changed;
    }

    if let Some(row_size) = scalar_width(element_codec) {
        *offset = (*offset).saturating_add(count.saturating_mul(row_size));
    }
    changed
}

fn struct_row_size(codec: &str) -> usize {
    struct_tokens_rs(codec)
        .iter()
        .map(|token| token_width_rs(token).unwrap_or(0))
        .sum()
}

fn scalar_width(codec: &str) -> Option<usize> {
    match codec {
        "b" | "B" | "int8" | "uint8" => Some(1),
        "h" | "H" | "int16" | "uint16" | "uint16le" => Some(2),
        "i" | "I" | "f" | "int32" | "uint32" | "float32" | "formid" => Some(4),
        "q" | "Q" | "int64" | "uint64" => Some(8),
        _ => None,
    }
}

fn is_integer_count_kind(kind: &str) -> bool {
    matches!(
        kind,
        "int8" | "uint8" | "int16" | "uint16" | "uint16le" | "int32" | "uint32"
    )
}

fn read_count_value(data: &[u8], offset: usize, kind: &str) -> Option<usize> {
    match kind {
        "B" | "uint8" => data.get(offset).copied().map(usize::from),
        "b" | "int8" => data
            .get(offset)
            .and_then(|value| usize::try_from(*value as i8).ok()),
        "H" | "uint16" | "uint16le" => data
            .get(offset..offset.checked_add(2)?)
            .map(|bytes| usize::from(u16::from_le_bytes([bytes[0], bytes[1]]))),
        "h" | "int16" => data
            .get(offset..offset.checked_add(2)?)
            .and_then(|bytes| usize::try_from(i16::from_le_bytes([bytes[0], bytes[1]])).ok()),
        "I" | "uint32" => data.get(offset..offset.checked_add(4)?).and_then(|bytes| {
            usize::try_from(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])).ok()
        }),
        "i" | "int32" => data.get(offset..offset.checked_add(4)?).and_then(|bytes| {
            usize::try_from(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])).ok()
        }),
        _ => None,
    }
}

fn decode_subrecord_for_walker(
    sub_spec: &SchemaSubrecordJson,
    schema: &CompiledSchema,
    data: &[u8],
) -> Option<serde_json::Value> {
    // Path A: struct-with-arrays codec (OBTS, FSTS.DATA, etc.) — the
    // higher-level dispatcher inside compact_subrecord_to_json takes this
    // branch; mirror it here to get raw decoded values keyed by field.id.
    if sub_spec
        .codec
        .as_deref()
        .is_some_and(|codec| codec.starts_with("struct:"))
        && sub_spec.fields.iter().any(|field| field.array.is_some())
    {
        let codec = sub_spec.codec.as_deref()?;
        let mut offset = 0usize;
        return decode_schema_struct_with_arrays_partial_json(
            data,
            &mut offset,
            codec,
            &sub_spec.fields,
            true,
            None,
        );
    }
    // Path B: general DecodeSpec.
    let decode_spec = schema_subrecord_to_decode_spec(sub_spec, schema)?;
    let mut context = std::collections::HashMap::new();
    context.insert(
        "subrecord_size".to_string(),
        serde_json::Value::Number(serde_json::Number::from(data.len() as u64)),
    );
    decode_subrecord_json(&decode_spec, data, &context)
}

fn subrecord_schema_has_formid(sub_spec: &SchemaSubrecordJson) -> bool {
    if matches!(
        sub_spec.codec.as_deref(),
        Some("formid") | Some("formid_array")
    ) {
        return true;
    }
    if !sub_spec.union_variants.is_empty() {
        return sub_spec
            .union_variants
            .iter()
            .any(|v| schema_fields_have_formid(&v.fields));
    }
    schema_fields_have_formid(&sub_spec.fields)
}

fn schema_fields_have_formid(fields: &[SchemaFieldJson]) -> bool {
    fields.iter().any(|field| {
        if field.kind == "formid" {
            return true;
        }
        if field.kind == "struct" && schema_fields_have_formid(&field.fields) {
            return true;
        }
        if !field.union_variants.is_empty()
            && field
                .union_variants
                .iter()
                .any(|v| schema_fields_have_formid(&v.fields))
        {
            return true;
        }
        false
    })
}

fn walker_collect_formids_from_fields(
    fields: &[SchemaFieldJson],
    value: &serde_json::Value,
    out: &mut Vec<u32>,
) {
    let serde_json::Value::Object(map) = value else {
        return;
    };
    for field in fields {
        let Some(field_value) = map.get(&field.id) else {
            continue;
        };
        walker_collect_formids_from_field(field, field_value, out);
    }
}

fn walker_collect_formids_from_field(
    field: &SchemaFieldJson,
    value: &serde_json::Value,
    out: &mut Vec<u32>,
) {
    if field.array.is_some() {
        let serde_json::Value::Array(items) = value else {
            return;
        };
        for item in items {
            match field.kind.as_str() {
                "formid" => walker_emit_formid(item, out),
                "struct" => walker_collect_formids_from_fields(&field.fields, item, out),
                _ => {}
            }
        }
        return;
    }

    match field.kind.as_str() {
        "formid" => walker_emit_formid(value, out),
        "struct" => walker_collect_formids_from_fields(&field.fields, value, out),
        _ => {
            // Field-level union: JSON shape is {"variant": "name", "value": ...}.
            if !field.union_variants.is_empty() {
                let serde_json::Value::Object(map) = value else {
                    return;
                };
                let Some(serde_json::Value::String(name)) = map.get("variant") else {
                    return;
                };
                let Some(inner) = map.get("value") else {
                    return;
                };
                if let Some(variant) = field.union_variants.iter().find(|v| &v.id == name) {
                    walker_collect_formids_from_fields(&variant.fields, inner, out);
                }
            }
        }
    }
}

fn walker_emit_formid(value: &serde_json::Value, out: &mut Vec<u32>) {
    if let Some(n) = value.as_u64() {
        if n != 0 && n <= u32::MAX as u64 {
            out.push(n as u32);
        }
    }
}

// -------------------------------------------------------------------------
// Tests — in-module unit tests for the pure helpers. Exercised by
// `cargo test --lib` and nightly CI; they don't require Python fixtures.
// -------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin_runtime::{SchemaEnumLabelJson, SchemaEnumValueJson, encode_model_info_json};

    #[test]
    fn display_name_matches_python_mapping() {
        assert_eq!(language_display_name("en"), "English");
        assert_eq!(language_display_name("fr"), "French");
        assert_eq!(language_display_name("esmx"), "Spanish_Mexico");
        assert_eq!(language_display_name("zhhans"), "ChineseSimplified");
        // Unknown codes pass through unchanged.
        assert_eq!(language_display_name("xx"), "xx");
    }

    #[test]
    fn codec_accepts_payload_length_fixed_size_scalars() {
        assert_eq!(codec_accepts_payload_length("formid", 4), Some(true));
        assert_eq!(codec_accepts_payload_length("formid", 24), Some(false));
        assert_eq!(codec_accepts_payload_length("uint16", 2), Some(true));
        assert_eq!(codec_accepts_payload_length("uint16", 1), Some(false));
        assert_eq!(codec_accepts_payload_length("uint8", 1), Some(true));
        assert_eq!(codec_accepts_payload_length("uint8", 4), Some(false));
        assert_eq!(
            codec_accepts_payload_length("fixed_string:12", 12),
            Some(true)
        );
        assert_eq!(
            codec_accepts_payload_length("fixed_string:12", 8),
            Some(false)
        );
    }

    #[test]
    fn codec_accepts_payload_length_struct_codecs() {
        // struct:I,H = 4+2 = 6
        assert_eq!(codec_accepts_payload_length("struct:I,H", 6), Some(true));
        assert_eq!(codec_accepts_payload_length("struct:I,H", 5), Some(false));
        // struct:f,f,f = 12
        assert_eq!(codec_accepts_payload_length("struct:f,f,f", 12), Some(true));
    }

    #[test]
    fn codec_accepts_payload_length_array_struct_marker_params() {
        // TERM Marker Parameters: 4*f + I + 4*B = 24 bytes per row.
        let codec = "array_struct:f,f,f,f,I,B,B,B,B";
        assert_eq!(codec_accepts_payload_length(codec, 24), Some(true));
        assert_eq!(codec_accepts_payload_length(codec, 48), Some(true));
        assert_eq!(codec_accepts_payload_length(codec, 4), Some(false));
        assert_eq!(codec_accepts_payload_length(codec, 25), Some(false));
        // Empty payload is technically a multiple of 24; that case is rare in
        // practice and the dispatcher's caller filters for >=2 candidates.
        assert_eq!(codec_accepts_payload_length(codec, 0), Some(true));
    }

    #[test]
    fn codec_accepts_payload_length_variable_codecs_return_none() {
        assert_eq!(codec_accepts_payload_length("zstring", 4), None);
        assert_eq!(codec_accepts_payload_length("lstring", 4), None);
        assert_eq!(codec_accepts_payload_length("bytes", 4), None);
        assert_eq!(codec_accepts_payload_length("formid_array", 8), None);
        assert_eq!(codec_accepts_payload_length("", 4), None);
        assert_eq!(codec_accepts_payload_length("unknown_codec", 4), None);
    }

    #[test]
    fn decode_omod_data_reads_count_prefixed_attach_parent_slots() {
        let mut data = Vec::new();
        data.extend_from_slice(&1_u32.to_le_bytes());
        data.extend_from_slice(&1_u32.to_le_bytes());
        data.extend_from_slice(&[0, 0]);
        data.extend_from_slice(&u32::from_le_bytes(*b"WEAP").to_le_bytes());
        data.extend_from_slice(&[0, 0]);
        data.extend_from_slice(&0x0007_F058_u32.to_le_bytes());
        data.extend_from_slice(&1_u32.to_le_bytes());
        data.extend_from_slice(&0x0042_A5D1_u32.to_le_bytes());
        data.extend_from_slice(&0_u32.to_le_bytes());
        data.extend_from_slice(&0x005F_6DC1_u32.to_le_bytes());
        data.extend_from_slice(&[0, 0, 1]);
        data.extend_from_slice(&[0, 0, 0, 0]);
        data.extend_from_slice(&[0, 0, 0, 0]);
        data.extend_from_slice(&3_u16.to_le_bytes());
        data.extend_from_slice(&[0, 0]);
        data.extend_from_slice(&0x005B_2FFF_u32.to_le_bytes());
        data.extend_from_slice(&0_u32.to_le_bytes());
        data.extend_from_slice(&0_f32.to_le_bytes());

        let decoded = decode_omod_data_to_field_map_json(&data)
            .expect("count-prefixed OMOD DATA should decode");
        let object = decoded.as_object().expect("decoded object");
        assert_eq!(object["attach_parent_slots"][0], 0x0042_A5D1_u32);
        assert_eq!(object["items"][0]["value_1"], 0_u32);
        assert!(
            !object["items"][0]
                .as_object()
                .expect("item row object")
                .contains_key("value_2")
        );
        assert_eq!(object["includes"][0]["mod"], 0x005F_6DC1_u32);
        assert_eq!(object["properties"][0]["property"], 3_u16);
    }

    /// Fixture spec for `model_info` round-trip tests: field ids match the
    /// authoring JSON contract (`textures`, `addon_nodes`, `srgb_count`,
    /// `materials`), so `schema_mapping_value_json`'s field-id fallback picks
    /// up the snake_case keys `decode_model_info_to_field_map_json` emits.
    fn model_info_test_spec() -> SchemaSubrecordJson {
        let entry_fields = vec![
            SchemaFieldJson {
                id: "file_hash".to_string(),
                kind: "uint32".to_string(),
                ..Default::default()
            },
            SchemaFieldJson {
                id: "extension".to_string(),
                kind: "fixed_string".to_string(),
                ..Default::default()
            },
            SchemaFieldJson {
                id: "folder_hash".to_string(),
                kind: "uint32".to_string(),
                ..Default::default()
            },
        ];
        SchemaSubrecordJson {
            id: "MODT".to_string(),
            kind: "parsed_with_raw_fallback".to_string(),
            codec: Some("model_info".to_string()),
            fields: vec![
                SchemaFieldJson {
                    id: "textures".to_string(),
                    kind: "array".to_string(),
                    fields: entry_fields.clone(),
                    ..Default::default()
                },
                SchemaFieldJson {
                    id: "addon_nodes".to_string(),
                    kind: "uint32".to_string(),
                    ..Default::default()
                },
                SchemaFieldJson {
                    id: "srgb_count".to_string(),
                    kind: "uint32".to_string(),
                    ..Default::default()
                },
                SchemaFieldJson {
                    id: "materials".to_string(),
                    kind: "array".to_string(),
                    fields: entry_fields,
                    ..Default::default()
                },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn decode_model_info_reads_textures_addon_nodes_and_materials() {
        // TERM DN035_RobotControlTerminal MODT (13CB50:Fallout4.esm), captured
        // via `modkit esp export --mode lossless` against DLCRobot.esm: 21
        // textures (ext "dds"), 1 addon node, srgb_count=13, 3 materials
        // (ext "bgsm").
        let data = hex::decode(
            "0400000015000000010000000D000000030000004B25D0F3646473008FBEDB9F9249D690\
             646473008FBEDB9F7C4F12F2646473008FBEDB9FA5231491646473008FBEDB9F25F154F0\
             646473008FBEDB9FFC9D5293646473008FBEDB9F77F33006646473007B24D06CBF79ECA7\
             64647300BE643C4C3C60073E646473008FBEDB9F9308AE366464730038973CEA24389F9F\
             646473001CDB88C5B076E385646473007B24D06CFF6512FD6464730038973CEA7EEF02FC\
             64647300582C5533C80FD0FC6464730038973CEAE2748773646473008FBEDB9FD000F877\
             646473001CDB88C5BBCAC171646473008FBEDB9F8CA00370646473008FBEDB9F62F091FC\
             6464730038973CEAD1A562886464730038973CEAC3000000E0FECFBC6267736D12D53ECB\
             5AAFC6256267736D12D53ECBCC9FC1526267736D12D53ECB",
        )
        .expect("valid hex fixture");

        let decoded =
            decode_model_info_to_field_map_json(&data).expect("well-formed MODT should decode");
        let object = decoded.as_object().expect("decoded object");
        assert_eq!(object["textures"].as_array().unwrap().len(), 21);
        assert_eq!(object["textures"][0]["extension"], "dds");
        assert_eq!(object["addon_nodes"].as_array().unwrap().len(), 1);
        assert_eq!(object["addon_nodes"][0], 195_u32);
        assert_eq!(object["srgb_count"], 13_u32);
        let materials = object["materials"].as_array().unwrap();
        assert_eq!(materials.len(), 3);
        assert_eq!(materials[0]["extension"], "bgsm");
    }

    #[test]
    fn model_info_roundtrips_textures_and_single_material() {
        // STAT MetalBarrel01Fire01_Static MODT (048280:Fallout4.esm), captured
        // via `modkit esp get-record --authoring` against Fallout4.esm: 19
        // textures (ext "dds"), 0 addon nodes, srgb_count=15, 1 material
        // (ext "bgsm").
        let data = hex::decode(
            "0400000013000000000000000F0000000100000075DB5AEF6464730038973CEAB3AEEF3B\
             646473007A7C3A5ADAE0E40B646473000BD80002038CE268646473000BD800020717F56F\
             6464730038973CEA2D0D94F664647300582C55331D653788646473000BD80002B92277AE\
             646473001CDB88C54D1A1046646473001CDB88C5528FF9866464730038973CEA29F70F00\
             64647300582C5533AD473ADB646473007A7C3A5A791608CF646473000786F88DF6E39FC7\
             64647300582C5533F4441C8564647300582C553332A999016464730038973CEA7F44DDF8\
             64647300582C553362F091FC6464730038973CEAE8C1B2DE6464730038973CEA7FFC0CAC\
             6267736DC23D6406",
        )
        .expect("valid hex fixture");

        let decoded =
            decode_model_info_to_field_map_json(&data).expect("well-formed MODT should decode");
        let mapping = decoded.as_object().expect("decoded object").clone();
        let spec = model_info_test_spec();
        let encoded =
            encode_model_info_json(&spec, &mapping, "MODT").expect("model_info should re-encode");
        assert_eq!(encoded, data, "byte-exact round-trip failed");
    }

    #[test]
    fn model_info_roundtrips_addon_nodes_and_multiple_materials() {
        // TERM DN035_RobotControlTerminal MODT (13CB50:Fallout4.esm), same
        // fixture as the decode test above — exercises addon_nodes and a
        // multi-row materials array in the same round trip.
        let data = hex::decode(
            "0400000015000000010000000D000000030000004B25D0F3646473008FBEDB9F9249D690\
             646473008FBEDB9F7C4F12F2646473008FBEDB9FA5231491646473008FBEDB9F25F154F0\
             646473008FBEDB9FFC9D5293646473008FBEDB9F77F33006646473007B24D06CBF79ECA7\
             64647300BE643C4C3C60073E646473008FBEDB9F9308AE366464730038973CEA24389F9F\
             646473001CDB88C5B076E385646473007B24D06CFF6512FD6464730038973CEA7EEF02FC\
             64647300582C5533C80FD0FC6464730038973CEAE2748773646473008FBEDB9FD000F877\
             646473001CDB88C5BBCAC171646473008FBEDB9F8CA00370646473008FBEDB9F62F091FC\
             6464730038973CEAD1A562886464730038973CEAC3000000E0FECFBC6267736D12D53ECB\
             5AAFC6256267736D12D53ECBCC9FC1526267736D12D53ECB",
        )
        .expect("valid hex fixture");

        let decoded =
            decode_model_info_to_field_map_json(&data).expect("well-formed MODT should decode");
        let mapping = decoded.as_object().expect("decoded object").clone();
        let spec = model_info_test_spec();
        let encoded =
            encode_model_info_json(&spec, &mapping, "MODT").expect("model_info should re-encode");
        assert_eq!(encoded, data, "byte-exact round-trip failed");
    }

    #[test]
    fn model_info_roundtrips_all_zero_counters() {
        // WEAP "10mm" MODT (004822:Fallout4.esm): header-only edge case with
        // no texture/addon-node/material rows at all.
        let data =
            hex::decode("0400000000000000000000000000000000000000").expect("valid hex fixture");

        let decoded =
            decode_model_info_to_field_map_json(&data).expect("well-formed MODT should decode");
        let mapping = decoded.as_object().expect("decoded object").clone();
        let spec = model_info_test_spec();
        let encoded =
            encode_model_info_json(&spec, &mapping, "MODT").expect("model_info should re-encode");
        assert_eq!(encoded, data, "byte-exact round-trip failed");
    }

    #[test]
    fn language_display_order_has_fourteen_entries() {
        // Guards against accidental additions/removals that would break
        // byte-exact YAML output matching py_creation_lib/python/creation_lib/esp/strings.LANGUAGE_DISPLAY_ORDER.
        assert_eq!(LANGUAGE_DISPLAY_ORDER.len(), 14);
        assert_eq!(LANGUAGE_DISPLAY_ORDER[4], "English");
    }

    #[test]
    fn authoring_key_names_are_camel_case() {
        assert_eq!(
            authoring_key_name(Some("Don't Use All"), "don_t_use_all"),
            "DontUseAll"
        );
        assert_eq!(
            authoring_key_name(Some("Editor ID"), "editor_id"),
            "EditorID"
        );
    }

    fn make_subrecord_spec(id: &str, required: bool, repeatable: bool) -> SchemaSubrecordJson {
        SchemaSubrecordJson {
            id: id.to_string(),
            kind: "raw".to_string(),
            repeatable,
            required,
            ..Default::default()
        }
    }

    fn make_record_spec(subrecords: Vec<SchemaSubrecordJson>) -> SchemaRecordJson {
        SchemaRecordJson {
            id: "TEST".to_string(),
            subrecords,
            ..Default::default()
        }
    }

    fn make_enum(id: &str, storage_kind: &str) -> SchemaEnumJson {
        SchemaEnumJson {
            id: id.to_string(),
            values: vec![
                SchemaEnumValueJson {
                    value: 0,
                    id: "false".to_string(),
                },
                SchemaEnumValueJson {
                    value: 1,
                    id: "true".to_string(),
                },
            ],
            labels: vec![
                SchemaEnumLabelJson {
                    value: 0,
                    label: "False".to_string(),
                },
                SchemaEnumLabelJson {
                    value: 1,
                    label: "True".to_string(),
                },
            ],
            aliases: Vec::new(),
            scope: "scoped".to_string(),
            storage_kind: storage_kind.to_string(),
            byte_width: 4,
            ..Default::default()
        }
    }

    fn make_empty_schema() -> CompiledSchema {
        CompiledSchema {
            records: std::collections::HashMap::new(),
            enums: std::collections::HashMap::new(),
        }
    }

    fn make_field(id: &str, kind: &str, display_label: &str) -> SchemaFieldJson {
        SchemaFieldJson {
            id: id.to_string(),
            kind: kind.to_string(),
            display_label: Some(display_label.to_string()),
            enum_ref: None,
            formlink_target: None,
            formlink_targets: Vec::new(),
            null_allowed: false,
            union_variants: Vec::new(),
            array: None,
            fields: Vec::new(),
            default_value: None,
            presence_conditions: Vec::new(),
        }
    }

    fn make_array_field(
        id: &str,
        count_codec: &str,
        element_codec: &str,
        display_label: &str,
    ) -> SchemaFieldJson {
        SchemaFieldJson {
            id: id.to_string(),
            kind: "array".to_string(),
            display_label: Some(display_label.to_string()),
            enum_ref: None,
            formlink_target: None,
            formlink_targets: Vec::new(),
            null_allowed: false,
            union_variants: Vec::new(),
            array: Some(SchemaArrayJson {
                _layout: "count_prefixed".to_string(),
                element_codec: Some(element_codec.to_string()),
                count_field: None,
                count_codec: Some(count_codec.to_string()),
                count_transform: None,
                count_record_field: None,
            }),
            fields: Vec::new(),
            default_value: None,
            presence_conditions: Vec::new(),
        }
    }

    fn rewrite_test_policy(raw: u32) -> Option<u32> {
        match raw {
            0x0012_3456 => Some(0x0712_3456),
            0x0056_789A => Some(0x0756_789A),
            _ => None,
        }
    }

    fn parsed_subrecord_spec(
        id: &str,
        codec: &str,
        fields: Vec<SchemaFieldJson>,
    ) -> SchemaSubrecordJson {
        let mut spec = make_subrecord_spec(id, false, false);
        spec.kind = "parsed".to_string();
        spec.codec = Some(codec.to_string());
        spec.fields = fields;
        spec
    }

    #[test]
    fn rewrite_schema_form_ids_rewrites_top_level_formid() {
        let schema = make_empty_schema();
        let spec =
            parsed_subrecord_spec("XNAM", "formid", vec![make_field("ref", "formid", "Ref")]);
        let mut data = 0x0012_3456_u32.to_le_bytes().to_vec();

        assert!(rewrite_schema_form_ids_in_subrecord(
            &spec,
            &schema,
            &mut data,
            &mut rewrite_test_policy
        ));
        assert_eq!(
            u32::from_le_bytes(data[0..4].try_into().unwrap()),
            0x0712_3456
        );
    }

    #[test]
    fn rewrite_schema_form_ids_rewrites_nested_struct_formid_only() {
        let schema = make_empty_schema();
        let spec = parsed_subrecord_spec(
            "DATA",
            "struct:I,I",
            vec![
                make_field("material_swap", "formid", "Material Swap"),
                make_field("not_a_ref", "uint32", "Not A Ref"),
            ],
        );
        let mut data = Vec::new();
        data.extend_from_slice(&0x0012_3456_u32.to_le_bytes());
        data.extend_from_slice(&0x0056_789A_u32.to_le_bytes());

        assert!(rewrite_schema_form_ids_in_subrecord(
            &spec,
            &schema,
            &mut data,
            &mut rewrite_test_policy
        ));
        assert_eq!(
            u32::from_le_bytes(data[0..4].try_into().unwrap()),
            0x0712_3456
        );
        assert_eq!(
            u32::from_le_bytes(data[4..8].try_into().unwrap()),
            0x0056_789A
        );
    }

    #[test]
    fn rewrite_schema_form_ids_rewrites_array_struct_rows() {
        let schema = make_empty_schema();
        let spec = parsed_subrecord_spec(
            "RDWT",
            "array_struct:I,I",
            vec![
                make_field("weather", "formid", "Weather"),
                make_field("chance", "uint32", "Chance"),
            ],
        );
        let mut data = Vec::new();
        data.extend_from_slice(&0x0012_3456_u32.to_le_bytes());
        data.extend_from_slice(&25_u32.to_le_bytes());
        data.extend_from_slice(&0x0056_789A_u32.to_le_bytes());
        data.extend_from_slice(&75_u32.to_le_bytes());

        assert!(rewrite_schema_form_ids_in_subrecord(
            &spec,
            &schema,
            &mut data,
            &mut rewrite_test_policy
        ));
        assert_eq!(
            u32::from_le_bytes(data[0..4].try_into().unwrap()),
            0x0712_3456
        );
        assert_eq!(u32::from_le_bytes(data[4..8].try_into().unwrap()), 25);
        assert_eq!(
            u32::from_le_bytes(data[8..12].try_into().unwrap()),
            0x0756_789A
        );
        assert_eq!(u32::from_le_bytes(data[12..16].try_into().unwrap()), 75);
    }

    #[test]
    fn rewrite_schema_form_ids_rewrites_counted_formid_array_field() {
        let schema = make_empty_schema();
        let mut refs = make_field("refs", "formid", "Refs");
        refs.array = Some(SchemaArrayJson {
            _layout: "row_array".to_string(),
            element_codec: Some("I".to_string()),
            count_field: Some("count".to_string()),
            count_codec: None,
            count_transform: None,
            count_record_field: None,
        });
        let spec = parsed_subrecord_spec(
            "DATA",
            "struct:I",
            vec![make_field("count", "uint32", "Count"), refs],
        );
        let mut data = Vec::new();
        data.extend_from_slice(&2_u32.to_le_bytes());
        data.extend_from_slice(&0x0012_3456_u32.to_le_bytes());
        data.extend_from_slice(&0x0056_789A_u32.to_le_bytes());

        assert!(rewrite_schema_form_ids_in_subrecord(
            &spec,
            &schema,
            &mut data,
            &mut rewrite_test_policy
        ));
        assert_eq!(u32::from_le_bytes(data[0..4].try_into().unwrap()), 2);
        assert_eq!(
            u32::from_le_bytes(data[4..8].try_into().unwrap()),
            0x0712_3456
        );
        assert_eq!(
            u32::from_le_bytes(data[8..12].try_into().unwrap()),
            0x0756_789A
        );
    }

    fn make_form_version_condition(operator: &str, value: u64) -> SchemaConditionJson {
        SchemaConditionJson {
            field: "record_form_version".to_string(),
            operator: operator.to_string(),
            value: Some(serde_json::json!(value)),
            values: Vec::new(),
        }
    }

    fn make_actor_value_union_field() -> SchemaFieldJson {
        SchemaFieldJson {
            id: "actor_value".to_string(),
            kind: "union".to_string(),
            display_label: Some("Actor Value".to_string()),
            enum_ref: None,
            formlink_target: None,
            formlink_targets: Vec::new(),
            null_allowed: false,
            union_variants: vec![
                SchemaUnionVariantJson {
                    id: "actor_value".to_string(),
                    codec: Some("uint8".to_string()),
                    enum_ref: None,
                    fields: vec![make_field(
                        "actor_value_actor_value",
                        "uint8",
                        "Actor Value",
                    )],
                    conditions: vec![make_form_version_condition("lt", 78)],
                },
                SchemaUnionVariantJson {
                    id: "actor_value".to_string(),
                    codec: Some("formid".to_string()),
                    enum_ref: None,
                    fields: vec![make_field(
                        "actor_value_actor_value",
                        "formid",
                        "Actor Value",
                    )],
                    conditions: vec![make_form_version_condition("gte", 78)],
                },
            ],
            array: None,
            fields: Vec::new(),
            default_value: None,
            presence_conditions: Vec::new(),
        }
    }

    fn make_ctda_spec() -> SchemaSubrecordJson {
        SchemaSubrecordJson {
            id: "CTDA".to_string(),
            kind: "parsed".to_string(),
            display_label: None,
            codec: Some("struct:B,B,B,B,I,H,B,B,I,I,I,I,i".to_string()),
            fields: vec![
                make_field("type", "uint8", "Type"),
                make_field("unknown_u8_1", "uint8", "Unknown U8 1"),
                make_field("unknown_u8_2", "uint8", "Unknown U8 2"),
                make_field("unknown_u8_3", "uint8", "Unknown U8 3"),
                make_field("comparison_value", "uint32", "Comparison Value"),
                make_field("function", "uint16", "Function"),
                make_field("unknown_u8_6", "uint8", "Unknown U8 6"),
                make_field("unknown_u8_7", "uint8", "Unknown U8 7"),
                make_field("parameter_1", "uint32", "Parameter #1"),
                make_field("parameter_2", "uint32", "Parameter #2"),
                make_field("run_on", "uint32", "Run On"),
                make_field("reference", "uint32", "Reference"),
                make_field("parameter_3", "int32", "Parameter #3"),
            ],
            repeatable: true,
            required: false,
            localized: false,
            enum_ref: None,
            formlink_target: None,
            formlink_targets: Vec::new(),
            null_allowed: false,
            union_selector: None,
            union_variants: Vec::new(),
            _array: None,
            row_label: None,
            authoring_layout: None,
            authoring_key: None,
            scope_id: None,
        }
    }

    fn make_ctda_bytes(function_id: u16, parameter_one: u32) -> [u8; 32] {
        let mut data = [0u8; 32];
        data[8..10].copy_from_slice(&function_id.to_le_bytes());
        data[12..16].copy_from_slice(&parameter_one.to_le_bytes());
        data
    }

    fn make_vmad_subrecord_spec() -> SchemaSubrecordJson {
        let mut spec = make_subrecord_spec("VMAD", false, false);
        spec.kind = "parsed".to_string();
        spec.codec = Some("struct:h,h".to_string());
        spec.authoring_layout = Some("vmad".to_string());
        spec
    }

    #[test]
    fn enum_payload_compacts_bool_enum_to_yaml_bool() {
        let enum_def = make_enum("bool_enum", "enum");
        assert_eq!(
            enum_payload_to_json(&enum_def, 1),
            serde_json::Value::Bool(true)
        );
        assert_eq!(
            enum_payload_to_json(&enum_def, 0),
            serde_json::Value::Bool(false)
        );
    }

    #[test]
    fn enum_payload_compacts_scalar_enum_to_camel_case_label() {
        let enum_def = SchemaEnumJson {
            id: "stagger_enum".to_string(),
            values: vec![SchemaEnumValueJson {
                value: 1,
                id: "small".to_string(),
            }],
            labels: vec![SchemaEnumLabelJson {
                value: 1,
                label: "Small".to_string(),
            }],
            aliases: Vec::new(),
            scope: "scoped".to_string(),
            storage_kind: "enum".to_string(),
            byte_width: 4,
            default_value: None,
        };
        assert_eq!(
            enum_payload_to_json(&enum_def, 1),
            serde_json::Value::String("Small".to_string())
        );
    }

    #[test]
    fn enum_payload_compacts_flags_to_camel_case_label_list() {
        let enum_def = SchemaEnumJson {
            id: "WEAP.DNAM.flags".to_string(),
            values: vec![
                SchemaEnumValueJson {
                    value: 256,
                    id: "crit_effect_on_death".to_string(),
                },
                SchemaEnumValueJson {
                    value: 4_194_304,
                    id: "bolt_action".to_string(),
                },
            ],
            labels: vec![
                SchemaEnumLabelJson {
                    value: 256,
                    label: "Crit Effect - on Death".to_string(),
                },
                SchemaEnumLabelJson {
                    value: 4_194_304,
                    label: "Bolt Action".to_string(),
                },
            ],
            aliases: Vec::new(),
            scope: "scoped".to_string(),
            storage_kind: "flags".to_string(),
            byte_width: 4,
            default_value: None,
        };
        assert_eq!(
            enum_payload_to_json(&enum_def, 4_194_560),
            serde_json::Value::Array(vec![
                serde_json::Value::String("CritEffectOnDeath".to_string()),
                serde_json::Value::String("BoltAction".to_string()),
            ])
        );
    }

    #[test]
    fn enum_payload_compacts_blank_flag_labels_to_unknown_bit_names() {
        let enum_def = SchemaEnumJson {
            id: "MGEF.DATA.flags".to_string(),
            values: vec![
                SchemaEnumValueJson {
                    value: 8,
                    id: "value".to_string(),
                },
                SchemaEnumValueJson {
                    value: 536_870_912,
                    id: "value".to_string(),
                },
            ],
            labels: vec![
                SchemaEnumLabelJson {
                    value: 8,
                    label: "".to_string(),
                },
                SchemaEnumLabelJson {
                    value: 536_870_912,
                    label: "????".to_string(),
                },
            ],
            aliases: vec![],
            scope: "scoped".to_string(),
            storage_kind: "flags".to_string(),
            byte_width: 4,
            default_value: None,
        };

        assert_eq!(
            enum_payload_to_json(&enum_def, 536_870_920),
            serde_json::Value::Array(vec![
                serde_json::Value::String("Unknown3".to_string()),
                serde_json::Value::String("Unknown29".to_string()),
            ])
        );
    }

    #[test]
    fn compact_subrecord_decodes_raw_lvlo_entry_layout() {
        let spec = make_subrecord_spec("LVLO", false, true);
        let schema = make_empty_schema();
        let masters = vec!["Fallout4.esm".to_string()];
        let data = [
            0x01, 0x00, 0x00, 0x00, 0xF1, 0x62, 0x24, 0x00, 0x01, 0x00, 0x00, 0x00,
        ];

        let decoded = compact_subrecord_to_json(
            &data,
            None,
            Some((&spec, &schema)),
            &LocalizedStringsState::default(),
            &masters,
            "Patch.esp",
            None,
            None,
            None,
        );

        assert_eq!(
            decoded,
            serde_json::json!({
                "Data": {
                    "Level": 1,
                    "Reference": {
                        "reference": {
                            "plugin": "Fallout4.esm",
                            "object_id": "2462F1"
                        }
                    },
                    "Count": 1
                },
                "raw_hex": "01000000F162240001000000"
            })
        );
    }

    #[test]
    fn compact_subrecord_emits_null_for_null_formid() {
        // FormID 0x00000000 (null reference) must NOT fall back to raw_hex.
        // The encoder accepts JSON null and re-encodes to four zero bytes.
        let mut spec = make_subrecord_spec("MNAM", false, false);
        spec.kind = "parsed".to_string();
        spec.codec = Some("formid".to_string());
        spec.fields = vec![make_field(
            "precipitation_type",
            "formid",
            "Precipitation Type",
        )];
        let schema = make_empty_schema();
        let data = [0x00, 0x00, 0x00, 0x00];

        let decode_spec = schema_subrecord_to_decode_spec(&spec, &schema)
            .expect("formid subrecord must produce a decode spec");

        let decoded = compact_subrecord_to_json(
            &data,
            Some(&decode_spec),
            Some((&spec, &schema)),
            &LocalizedStringsState::default(),
            &[],
            "Patch.esp",
            None,
            None,
            None,
        );

        // kind=parsed → bare null, no raw_hex wrapper, no fallback object.
        assert_eq!(decoded, serde_json::Value::Null);
    }

    #[test]
    fn compact_subrecord_decodes_vmad_scripts_and_omits_raw_hex_when_roundtrip_clean() {
        // When the parsed payload re-encodes byte-exactly the
        // decoder omits `raw_hex`. The encoder reconstructs the bytes from
        // the parsed value via build_vmad_bytes_from_payload. raw_hex is now
        // only emitted when the round-trip would lose data (unparsed tail or
        // re-encode mismatch).
        let spec = make_vmad_subrecord_spec();
        let schema = make_empty_schema();
        let data = [
            0x06, 0x00, 0x02, 0x00, 0x01, 0x00, // header + script count
            0x08, 0x00, b'M', b'y', b'S', b'c', b'r', b'i', b'p', b't', // script name
            0x00, 0x02, 0x00, // flags + property count
            0x08, 0x00, b'G', b'r', b'e', b'e', b't', b'i', b'n', b'g', // property name
            0x02, 0x00, // string type + flags
            0x05, 0x00, b'H', b'e', b'l', b'l', b'o', // string value
            0x07, 0x00, b'E', b'n', b'a', b'b', b'l', b'e', b'd', // property name
            0x05, 0x01, 0x01, // bool type + flags + value
        ];

        let decoded = compact_subrecord_to_json(
            &data,
            None,
            Some((&spec, &schema)),
            &LocalizedStringsState::default(),
            &[],
            "Patch.esp",
            None,
            None,
            None,
        );

        assert_eq!(decoded["Version"], serde_json::json!(6));
        assert_eq!(decoded["Object Format"], serde_json::json!(2));
        assert_eq!(
            decoded["Scripts"][0]["ScriptName"],
            serde_json::json!("MyScript")
        );
        assert_eq!(
            decoded["Scripts"][0]["Properties"][0]["Value"],
            serde_json::json!("Hello")
        );
        assert_eq!(
            decoded["Scripts"][0]["Properties"][1]["Flags"],
            serde_json::json!(1)
        );
        assert_eq!(decoded.get("raw_hex"), None);
        assert_eq!(decoded.get("tail_hex"), None);
    }

    #[test]
    fn compact_subrecord_adds_function_metadata_for_ctda_formkey_params() {
        let spec = make_ctda_spec();
        let schema = make_empty_schema();
        let decode_spec = schema_subrecord_to_decode_spec(&spec, &schema);
        let masters = vec!["Fallout4.esm".to_string()];
        let data = make_ctda_bytes(277, 0x0000_02C7);

        let decoded = compact_subrecord_to_json(
            &data,
            decode_spec.as_ref(),
            Some((&spec, &schema)),
            &LocalizedStringsState::default(),
            &masters,
            "Patch.esp",
            None,
            None,
            None,
        );

        assert_eq!(decoded["Function"], serde_json::json!(277));
        assert_eq!(decoded["FunctionName"], serde_json::json!("GetBaseValue"));
        assert_eq!(
            decoded["ParameterOneRecord"],
            serde_json::json!({
                "reference": {
                    "plugin": "Fallout4.esm",
                    "object_id": "0002C7"
                }
            })
        );
        assert_eq!(decoded["ParameterOneNumber"], serde_json::json!(711));
    }

    #[test]
    fn compact_subrecord_uses_starfield_first_parameter_key_for_quest_completed() {
        let spec = make_ctda_spec();
        let schema = make_empty_schema();
        let decode_spec = schema_subrecord_to_decode_spec(&spec, &schema);
        let masters = vec!["Starfield.esm".to_string()];

        for function_id in [56, 543] {
            let data = make_ctda_bytes(function_id, 0x000B_8633);

            let decoded = compact_subrecord_to_json(
                &data,
                decode_spec.as_ref(),
                Some((&spec, &schema)),
                &LocalizedStringsState::default(),
                &masters,
                "Patch.esp",
                None,
                None,
                None,
            );

            assert_eq!(
                decoded["FunctionName"],
                serde_json::json!("GetQuestCompletedConditionData")
            );
            assert_eq!(
                decoded["FirstParameter"],
                serde_json::json!({
                    "reference": {
                        "plugin": "Starfield.esm",
                        "object_id": "0B8633"
                    }
                })
            );
            assert!(decoded.get("ParameterOneNumber").is_none());
        }
    }

    #[test]
    fn compact_subrecord_keeps_lvlo_unknown_words_when_nonzero() {
        let spec = make_subrecord_spec("LVLO", false, true);
        let schema = make_empty_schema();
        let masters = vec!["FalloutNV.esm".to_string()];
        let data = [
            0x01, 0x00, 0x0C, 0x0B, 0xA9, 0xB5, 0x0C, 0x00, 0x05, 0x00, 0x0C, 0x0B,
        ];

        let decoded = compact_subrecord_to_json(
            &data,
            None,
            Some((&spec, &schema)),
            &LocalizedStringsState::default(),
            &masters,
            "Patch.esp",
            None,
            None,
            None,
        );

        assert_eq!(
            decoded,
            serde_json::json!({
                "Data": {
                    "Level": 1,
                    "Unknown1": 2828,
                    "Reference": {
                        "reference": {
                            "plugin": "FalloutNV.esm",
                            "object_id": "0CB5A9"
                        }
                    },
                    "Count": 5,
                    "Unknown2": 2828
                },
                "raw_hex": "01000C0BA9B50C0005000C0B"
            })
        );
    }

    #[test]
    fn compact_subrecord_leaves_unsupported_lvlo_size_raw() {
        let spec = make_subrecord_spec("LVLO", false, true);
        let schema = make_empty_schema();
        let data = [0x01, 0x00, 0x00, 0x00];

        let decoded = compact_subrecord_to_json(
            &data,
            None,
            Some((&spec, &schema)),
            &LocalizedStringsState::default(),
            &[],
            "Patch.esp",
            None,
            None,
            None,
        );

        assert_eq!(
            decoded,
            serde_json::json!({
                "raw_hex": "01000000"
            })
        );
    }

    #[test]
    fn localized_value_payload_to_json_preserves_raw_hex() {
        let mut strings = LocalizedStringsState::default();
        strings.default_language = "en".to_string();
        strings
            .by_language
            .entry("en".to_string())
            .or_default()
            .insert(0x1234, "Far Harbor".to_string());
        strings
            .by_language
            .entry("fr".to_string())
            .or_default()
            .insert(0x1234, "Far Harbor FR".to_string());

        let payload =
            localized_value_payload_to_json(&strings, 0x9999, "99990000", Some("localized_string"));

        assert_eq!(payload["raw_hex"], serde_json::json!("99990000"));
        assert_eq!(
            payload["semantic_type"],
            serde_json::json!("localized_string")
        );
        assert!(payload.get("TargetLanguage").is_none());
        assert!(payload.get("Values").is_none());
    }

    #[test]
    fn localized_value_payload_to_json_emits_values_without_raw_hex() {
        let mut strings = LocalizedStringsState::default();
        strings.default_language = "en".to_string();
        strings
            .by_language
            .entry("en".to_string())
            .or_default()
            .insert(0x1234, "Far Harbor".to_string());
        strings
            .by_language
            .entry("fr".to_string())
            .or_default()
            .insert(0x1234, "Far Harbor FR".to_string());

        let payload = localized_value_payload_to_json(&strings, 0x1234, "", None);

        assert_eq!(payload["TargetLanguage"], serde_json::json!("English"));
        assert_eq!(
            payload["Values"][0]["Language"],
            serde_json::json!("English")
        );
        assert_eq!(
            payload["Values"][0]["String"],
            serde_json::json!("Far Harbor")
        );
    }

    #[test]
    fn validate_record_passes_for_required_present() {
        let spec = make_record_spec(vec![
            make_subrecord_spec("EDID", true, false),
            make_subrecord_spec("FULL", false, false),
        ]);
        assert!(validate_record_signatures("TEST", &["EDID", "FULL"], &spec).is_ok());
    }

    #[test]
    fn validate_record_passes_for_missing_optional() {
        let spec = make_record_spec(vec![
            make_subrecord_spec("EDID", true, false),
            make_subrecord_spec("FULL", false, false),
        ]);
        assert!(validate_record_signatures("TEST", &["EDID"], &spec).is_ok());
    }

    #[test]
    fn validate_record_detects_missing_required() {
        let spec = make_record_spec(vec![make_subrecord_spec("EDID", true, false)]);
        let err = validate_record_signatures("TEST", &["FULL"], &spec).unwrap_err();
        assert_eq!(err, "TEST is missing required subrecords: EDID");
    }

    #[test]
    fn validate_record_detects_duplicate_non_repeatable() {
        let spec = make_record_spec(vec![make_subrecord_spec("EDID", true, false)]);
        let err = validate_record_signatures("TEST", &["EDID", "EDID"], &spec).unwrap_err();
        assert_eq!(err, "TEST has duplicate non-repeatable subrecords: EDID");
    }

    #[test]
    fn validate_record_allows_duplicate_repeatable() {
        let spec = make_record_spec(vec![make_subrecord_spec("KWDA", false, true)]);
        assert!(validate_record_signatures("TEST", &["KWDA", "KWDA"], &spec).is_ok());
    }

    #[test]
    fn validate_record_ignores_unknown_signatures() {
        // Python counts only for signatures in the repeatable map.
        let spec = make_record_spec(vec![make_subrecord_spec("EDID", false, false)]);
        assert!(validate_record_signatures("TEST", &["EDID", "UNKN", "UNKN"], &spec).is_ok());
    }

    #[test]
    fn validate_record_reports_duplicates_sorted() {
        let spec = make_record_spec(vec![
            make_subrecord_spec("ZZZZ", false, false),
            make_subrecord_spec("AAAA", false, false),
        ]);
        let err = validate_record_signatures("TEST", &["ZZZZ", "ZZZZ", "AAAA", "AAAA"], &spec)
            .unwrap_err();
        assert_eq!(
            err,
            "TEST has duplicate non-repeatable subrecords: AAAA, ZZZZ"
        );
    }

    // ----- VMAD property type 11-15 round-trip ----------

    fn vmad_string_bytes(value: &str) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&(value.len() as u16).to_le_bytes());
        out.extend_from_slice(value.as_bytes());
        out
    }

    fn build_synthetic_vmad(
        property_type: u8,
        property_name: &str,
        property_payload: &[u8],
    ) -> Vec<u8> {
        let mut out = Vec::new();
        // header: version=6, object_format=2, script_count=1
        out.extend_from_slice(&6u16.to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        // script: name, flags=0, property_count=1
        out.extend(vmad_string_bytes("TestScript"));
        out.push(0);
        out.extend_from_slice(&1u16.to_le_bytes());
        // property
        out.extend(vmad_string_bytes(property_name));
        out.push(property_type);
        out.push(0); // property_flags
        out.extend_from_slice(property_payload);
        out
    }

    fn assert_vmad_roundtrip(blob: &[u8]) {
        let payload = compact_vmad_payload_json(blob, &[], "Patch.esp", None)
            .expect("decoder should produce a payload");
        let encoded =
            crate::plugin_runtime::build_vmad_bytes_from_payload(&payload, &[], "Patch.esp")
                .expect("encoder should succeed");
        assert_eq!(encoded, blob, "byte-exact round-trip failed");
    }

    #[test]
    fn vmad_property_type_11_array_of_object_round_trip() {
        // count=2, each object is 8 bytes at object_format=2
        // object1: unused=0, alias=-1 (0xFFFF), formid=0xFF000123 (own plugin)
        // object2: unused=0, alias=0,        formid=0x00000456 (master[0])
        // Note: with masters=[], own plugin is index 0 -> 0x00000123. Use no masters.
        let mut payload = Vec::new();
        payload.extend_from_slice(&2i32.to_le_bytes());
        payload.extend_from_slice(&0u16.to_le_bytes());
        payload.extend_from_slice(&(-1i16).to_le_bytes());
        payload.extend_from_slice(&0x00000123u32.to_le_bytes());
        payload.extend_from_slice(&0u16.to_le_bytes());
        payload.extend_from_slice(&0i16.to_le_bytes());
        payload.extend_from_slice(&0x00000456u32.to_le_bytes());
        let blob = build_synthetic_vmad(11, "MyArray", &payload);
        assert_vmad_roundtrip(&blob);
    }

    #[test]
    fn vmad_property_type_12_array_of_string_round_trip() {
        let mut payload = Vec::new();
        payload.extend_from_slice(&3i32.to_le_bytes());
        payload.extend(vmad_string_bytes("alpha"));
        payload.extend(vmad_string_bytes(""));
        payload.extend(vmad_string_bytes("gamma"));
        let blob = build_synthetic_vmad(12, "MyStrings", &payload);
        assert_vmad_roundtrip(&blob);
    }

    #[test]
    fn vmad_property_type_13_array_of_int32_round_trip() {
        let mut payload = Vec::new();
        payload.extend_from_slice(&4i32.to_le_bytes());
        for v in [0i32, -1, 42, i32::MIN] {
            payload.extend_from_slice(&v.to_le_bytes());
        }
        let blob = build_synthetic_vmad(13, "MyInts", &payload);
        assert_vmad_roundtrip(&blob);
    }

    #[test]
    fn vmad_property_type_14_array_of_float_round_trip() {
        let mut payload = Vec::new();
        payload.extend_from_slice(&3i32.to_le_bytes());
        for v in [0.0f32, 1.5, -3.25] {
            payload.extend_from_slice(&v.to_le_bytes());
        }
        let blob = build_synthetic_vmad(14, "MyFloats", &payload);
        assert_vmad_roundtrip(&blob);
    }

    #[test]
    fn vmad_property_type_15_array_of_bool_round_trip() {
        let mut payload = Vec::new();
        payload.extend_from_slice(&5i32.to_le_bytes());
        for v in [0u8, 1, 0, 1, 1] {
            payload.push(v);
        }
        let blob = build_synthetic_vmad(15, "MyBools", &payload);
        assert_vmad_roundtrip(&blob);
    }

    fn vmad_struct_member_bytes(
        name: &str,
        member_type: u8,
        member_flags: u8,
        value_payload: &[u8],
    ) -> Vec<u8> {
        let mut out = vmad_string_bytes(name);
        out.push(member_type);
        out.push(member_flags);
        out.extend_from_slice(value_payload);
        out
    }

    #[test]
    fn vmad_property_type_6_variable_round_trip() {
        // Type 6 ("Variable" per xEdit) carries no payload bytes; the prior
        // decoder labelled it "None", which collided with type 0 on encode
        // and corrupted the byte. Distinct labels close the round-trip.
        let blob = build_synthetic_vmad(6, "MaybeSet", &[]);
        let payload = compact_vmad_payload_json(&blob, &[], "Patch.esp", None)
            .expect("decoder should produce a payload");
        assert_eq!(
            payload["Scripts"][0]["Properties"][0]["Type"],
            serde_json::json!("Variable")
        );
        assert_vmad_roundtrip(&blob);
    }

    #[test]
    fn vmad_property_type_7_struct_round_trip() {
        // Single Struct property: i32 member_count, then members.
        // Members: u16 name_len + name + u8 type + u8 flags + value bytes.
        let mut payload = Vec::new();
        payload.extend_from_slice(&2i32.to_le_bytes());
        payload.extend(vmad_struct_member_bytes(
            "Count",
            3,
            0,
            &42i32.to_le_bytes(),
        ));
        let s_value = vmad_string_bytes("hello");
        payload.extend(vmad_struct_member_bytes("Greeting", 2, 1, &s_value));
        let blob = build_synthetic_vmad(7, "MyStruct", &payload);
        assert_vmad_roundtrip(&blob);
    }

    #[test]
    fn vmad_property_type_16_array_of_variable_round_trip() {
        // xEdit models type 16 as a single u32 element count, no element bytes.
        let mut payload = Vec::new();
        payload.extend_from_slice(&3u32.to_le_bytes());
        let blob = build_synthetic_vmad(16, "MaybeList", &payload);
        assert_vmad_roundtrip(&blob);
    }

    #[test]
    fn vmad_property_type_17_array_of_struct_round_trip() {
        // Two struct elements, each with two members of mixed types.
        let mut struct1 = Vec::new();
        struct1.extend_from_slice(&2i32.to_le_bytes());
        struct1.extend(vmad_struct_member_bytes("Index", 3, 0, &7i32.to_le_bytes()));
        struct1.extend(vmad_struct_member_bytes(
            "Ratio",
            4,
            0,
            &0.5f32.to_le_bytes(),
        ));

        let mut struct2 = Vec::new();
        struct2.extend_from_slice(&1i32.to_le_bytes());
        struct2.extend(vmad_struct_member_bytes("Active", 5, 0, &[1u8]));

        let mut payload = Vec::new();
        payload.extend_from_slice(&2i32.to_le_bytes());
        payload.extend_from_slice(&struct1);
        payload.extend_from_slice(&struct2);
        let blob = build_synthetic_vmad(17, "Items", &payload);
        assert_vmad_roundtrip(&blob);
    }

    #[test]
    fn vmad_property_type_17_nested_struct_round_trip() {
        // Type-17 payload whose member is itself a type-7 struct exercises
        // recursion through write_vmad_struct → write_vmad_property_value → write_vmad_struct.
        let mut inner = Vec::new();
        inner.extend_from_slice(&1i32.to_le_bytes());
        inner.extend(vmad_struct_member_bytes(
            "InnerInt",
            3,
            0,
            &99i32.to_le_bytes(),
        ));

        let mut outer_struct = Vec::new();
        outer_struct.extend_from_slice(&1i32.to_le_bytes());
        outer_struct.extend(vmad_struct_member_bytes("Nested", 7, 0, &inner));

        let mut payload = Vec::new();
        payload.extend_from_slice(&1i32.to_le_bytes());
        payload.extend_from_slice(&outer_struct);
        let blob = build_synthetic_vmad(17, "Outer", &payload);
        assert_vmad_roundtrip(&blob);
    }

    // ----- VMAD fragment round-trip --------------------

    fn build_vmad_with_fragments_tail(scripts_payload: Option<Vec<u8>>, tail: &[u8]) -> Vec<u8> {
        // Canonical VMAD with explicit script_count + 0/1 scripts + a fragment
        // tail. When `scripts_payload` is Some, it's used verbatim for the
        // first (and only) script; otherwise no scripts are emitted.
        let mut out = Vec::new();
        out.extend_from_slice(&6u16.to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        let script_count = if scripts_payload.is_some() {
            1u16
        } else {
            0u16
        };
        out.extend_from_slice(&script_count.to_le_bytes());
        if let Some(s) = scripts_payload {
            out.extend_from_slice(&s);
        }
        out.extend_from_slice(tail);
        out
    }

    fn empty_script_entry_bytes(name: &str) -> Vec<u8> {
        // wbScriptEntry with an empty Properties array.
        let mut out = vmad_string_bytes(name);
        out.push(0); // flags
        out.extend_from_slice(&0u16.to_le_bytes()); // property count
        out
    }

    fn assert_vmad_fragments_roundtrip(blob: &[u8], semantic_type: &str) {
        let payload = compact_vmad_payload_json(blob, &[], "Patch.esp", Some(semantic_type))
            .expect("decoder should produce a payload");
        let encoded =
            crate::plugin_runtime::build_vmad_bytes_from_payload(&payload, &[], "Patch.esp")
                .expect("encoder should succeed");
        assert_eq!(encoded, blob, "byte-exact fragment round-trip failed");
    }

    #[test]
    fn vmad_fragments_info_round_trip() {
        // INFO/PACK fragment block: version i8, flags u8, ScriptEntry,
        // popcount(flags) Fragment rows.
        let mut tail = Vec::new();
        tail.push(3); // version
        tail.push(0b011); // flags = OnBegin | OnEnd → 2 fragments
        tail.extend(empty_script_entry_bytes("FragScript"));
        // Fragment 1
        tail.push(0); // unknown i8
        tail.extend(vmad_string_bytes("FragScript"));
        tail.extend(vmad_string_bytes("Fragment_0"));
        // Fragment 2
        tail.push(1);
        tail.extend(vmad_string_bytes("FragScript"));
        tail.extend(vmad_string_bytes("Fragment_1"));
        let blob = build_vmad_with_fragments_tail(None, &tail);
        assert_vmad_fragments_roundtrip(&blob, "INFO");
    }

    #[test]
    fn vmad_fragments_pack_round_trip() {
        // PACK adds bit 4 (OnChange); binary layout matches INFO.
        let mut tail = Vec::new();
        tail.push(3);
        tail.push(0b101); // OnBegin | OnChange = 2 fragments
        tail.extend(empty_script_entry_bytes("PackScript"));
        tail.push(-1i8 as u8);
        tail.extend(vmad_string_bytes("PackScript"));
        tail.extend(vmad_string_bytes("Frag_OnBegin"));
        tail.push(2);
        tail.extend(vmad_string_bytes("PackScript"));
        tail.extend(vmad_string_bytes("Frag_OnChange"));
        let blob = build_vmad_with_fragments_tail(None, &tail);
        assert_vmad_fragments_roundtrip(&blob, "PACK");
    }

    #[test]
    fn vmad_fragments_scen_round_trip() {
        let mut tail = Vec::new();
        tail.push(3);
        tail.push(0b001); // 1 fragment
        tail.extend(empty_script_entry_bytes("ScenScript"));
        tail.push(0);
        tail.extend(vmad_string_bytes("ScenScript"));
        tail.extend(vmad_string_bytes("Begin"));
        // Phase Fragments u16 count + 1 entry
        tail.extend_from_slice(&1u16.to_le_bytes());
        tail.push(0b01); // phase flag (OnStart)
        tail.push(0); // phase index
        tail.extend_from_slice(&0i16.to_le_bytes()); // unknown s16
        tail.push(0); // unknown s8 a
        tail.push(0); // unknown s8 b
        tail.extend(vmad_string_bytes("ScenScript"));
        tail.extend(vmad_string_bytes("Phase_OnStart_0"));
        let blob = build_vmad_with_fragments_tail(None, &tail);
        assert_vmad_fragments_roundtrip(&blob, "SCEN");
    }

    #[test]
    fn vmad_fragments_perk_round_trip() {
        // wbScriptFragments: version i8, ScriptEntry, u16 count, fragment rows.
        let mut tail = Vec::new();
        tail.push(3);
        tail.extend(empty_script_entry_bytes("PerkScript"));
        tail.extend_from_slice(&2u16.to_le_bytes()); // 2 fragments
        // Fragment 0
        tail.extend_from_slice(&0u16.to_le_bytes()); // fragment index
        tail.extend_from_slice(&0i16.to_le_bytes()); // unused
        tail.push(0); // unknown i8
        tail.extend(vmad_string_bytes("PerkScript"));
        tail.extend(vmad_string_bytes("Fragment_0"));
        // Fragment 1
        tail.extend_from_slice(&1u16.to_le_bytes());
        tail.extend_from_slice(&0i16.to_le_bytes());
        tail.push(1);
        tail.extend(vmad_string_bytes("PerkScript"));
        tail.extend(vmad_string_bytes("Fragment_1"));
        let blob = build_vmad_with_fragments_tail(None, &tail);
        assert_vmad_fragments_roundtrip(&blob, "PERK");
    }

    #[test]
    fn vmad_fragments_term_round_trip_zero_fragments() {
        // TERM uses the same shape as PERK; this exercises the empty-fragment
        // path (count=0, no rows).
        let mut tail = Vec::new();
        tail.push(3);
        tail.extend(empty_script_entry_bytes("TermScript"));
        tail.extend_from_slice(&0u16.to_le_bytes());
        let blob = build_vmad_with_fragments_tail(None, &tail);
        assert_vmad_fragments_roundtrip(&blob, "TERM");
    }

    #[test]
    fn vmad_fragments_quest_round_trip_with_aliases() {
        // QUST: version i8, fragment_count u16, scriptname,
        // (if scriptname != "") flags u8 + properties u16-prefixed,
        // then fragment_count rows, then alias_count u16 + aliases.
        let mut tail = Vec::new();
        tail.push(3);
        tail.extend_from_slice(&1u16.to_le_bytes()); // 1 fragment
        tail.extend(vmad_string_bytes("QuestScript"));
        tail.push(0); // script flags
        tail.extend_from_slice(&0u16.to_le_bytes()); // property count
        // 1 fragment row
        tail.extend_from_slice(&10u16.to_le_bytes()); // quest stage
        tail.extend_from_slice(&0i16.to_le_bytes()); // unknown s16
        tail.extend_from_slice(&0i32.to_le_bytes()); // quest stage index
        tail.push(0); // unknown s8
        tail.extend(vmad_string_bytes("QuestScript"));
        tail.extend(vmad_string_bytes("Fragment_Stage10"));
        // alias_count = 1
        tail.extend_from_slice(&1u16.to_le_bytes());
        // alias.Object — object_format=2 layout is u16 unused, i16 alias, u32 formid
        tail.extend_from_slice(&0u16.to_le_bytes()); // unused
        tail.extend_from_slice(&(-1i16).to_le_bytes()); // alias
        tail.extend_from_slice(&0x00000123u32.to_le_bytes()); // formid
        // alias version + object_format
        tail.extend_from_slice(&6i16.to_le_bytes());
        tail.extend_from_slice(&2i16.to_le_bytes());
        // alias_script_count
        tail.extend_from_slice(&1u16.to_le_bytes());
        tail.extend(empty_script_entry_bytes("AliasScript"));
        let blob = build_vmad_with_fragments_tail(None, &tail);
        assert_vmad_fragments_roundtrip(&blob, "QUST");
    }

    #[test]
    fn vmad_fragments_quest_round_trip_empty_script() {
        // QUST allows scriptname == "" — flags+properties are skipped per
        // wbScriptFragmentsEmptyScriptDecider.
        let mut tail = Vec::new();
        tail.push(3);
        tail.extend_from_slice(&0u16.to_le_bytes()); // fragment count = 0
        tail.extend(vmad_string_bytes("")); // empty scriptname
        // No flags/properties because scriptname is empty.
        tail.extend_from_slice(&0u16.to_le_bytes()); // alias count
        let blob = build_vmad_with_fragments_tail(None, &tail);
        assert_vmad_fragments_roundtrip(&blob, "QUST");
    }

    // -----------------------------------------------------------------------
    // STAG.TNAM (Sound) struct:I + trailing zstring tail
    // -----------------------------------------------------------------------
    //
    // xEdit defines TNAM as wbStruct(formid, wbString) — the schema codec is
    // ``struct:I`` (one token) but the field list has two entries. Without the
    // string-tail support exercised here, the trailing ``action`` zstring
    // would have no token to bind to, the builder would fall through to
    // ``return None``, and every TNAM in the audit would emit raw_hex
    // (193 hits across 21 records on Fallout4.esm).
    fn make_stag_tnam_spec() -> SchemaSubrecordJson {
        SchemaSubrecordJson {
            id: "TNAM".to_string(),
            kind: "parsed".to_string(),
            display_label: Some("Sound".to_string()),
            codec: Some("struct:I".to_string()),
            fields: vec![
                make_field("sound", "formid", "Sound"),
                make_field("action", "zstring", "Action"),
            ],
            repeatable: true,
            required: false,
            localized: false,
            enum_ref: None,
            formlink_target: None,
            formlink_targets: Vec::new(),
            null_allowed: false,
            union_selector: None,
            union_variants: Vec::new(),
            _array: None,
            row_label: None,
            authoring_layout: None,
            authoring_key: None,
            scope_id: None,
        }
    }

    #[test]
    fn stag_tnam_decode_spec_emits_zstring_tail_segment() {
        let spec = make_stag_tnam_spec();
        let schema = make_empty_schema();
        let decode_spec = schema_subrecord_to_decode_spec(&spec, &schema)
            .expect("STAG.TNAM should build a DecodeSpec with a zstring tail");
        match decode_spec {
            crate::DecodeSpec::Struct {
                row_size,
                segments,
                tail_segment,
                parse_partial,
            } => {
                assert_eq!(row_size, 4, "row_size = formid width");
                assert_eq!(segments.len(), 1, "single segment for `sound` formid");
                assert_eq!(segments[0].name, "sound");
                assert!(!parse_partial);
                let tail = tail_segment.expect("tail_segment present for trailing zstring");
                assert_eq!(tail.name, "action");
                assert_eq!(tail.offset, 4);
                assert_eq!(tail.kind, "zstring");
            }
            _ => panic!("expected DecodeSpec::Struct, got something else"),
        }
    }

    #[test]
    fn stag_tnam_decode_row_decodes_zstring_action() {
        // Real Fallout4.esm STAG.TNAM payload:
        //   formid = 0x00219C25 (little-endian "259C2100")
        //   action = "NPCRobotAssaultronAttackPowerStanding\x00"
        let payload = hex::decode(
            "259C21004E5043526F626F7441737361756C74726F6E41747461636B506F7765725374616E64696E6700",
        )
        .unwrap();
        let spec = make_stag_tnam_spec();
        let schema = make_empty_schema();
        let decode_spec = schema_subrecord_to_decode_spec(&spec, &schema).unwrap();
        let ctx: std::collections::HashMap<String, serde_json::Value> =
            std::collections::HashMap::new();
        let value = decode_subrecord_json(&decode_spec, &payload, &ctx)
            .expect("STAG.TNAM payload should decode");
        let obj = value.as_object().unwrap();
        assert_eq!(
            obj.get("sound").and_then(|v| v.as_u64()),
            Some(0x0021_9C25),
            "sound formid decodes as integer"
        );
        assert_eq!(
            obj.get("action").and_then(|v| v.as_str()),
            Some("NPCRobotAssaultronAttackPowerStanding"),
            "action zstring decodes (NUL stripped)"
        );
    }

    #[test]
    fn stag_tnam_decode_row_handles_null_formid_action() {
        // Also-real payload with NULL formid + non-empty action
        // (e.g. NPCRadscorpionATS "TunnelExit"): formid=00000000,
        // action="TunnelExit\x00".
        let payload = hex::decode("0000000054756E6E656C4578697400").unwrap();
        let spec = make_stag_tnam_spec();
        let schema = make_empty_schema();
        let decode_spec = schema_subrecord_to_decode_spec(&spec, &schema).unwrap();
        let ctx: std::collections::HashMap<String, serde_json::Value> =
            std::collections::HashMap::new();
        let value = decode_subrecord_json(&decode_spec, &payload, &ctx).unwrap();
        let obj = value.as_object().unwrap();
        assert_eq!(obj.get("sound").and_then(|v| v.as_u64()), Some(0));
        assert_eq!(
            obj.get("action").and_then(|v| v.as_str()),
            Some("TunnelExit")
        );
    }

    fn make_debr_data_spec() -> SchemaSubrecordJson {
        SchemaSubrecordJson {
            id: "DATA".to_string(),
            kind: "parsed".to_string(),
            display_label: Some("Data".to_string()),
            codec: Some("struct:B,zstring,B".to_string()),
            fields: vec![
                make_field("percentage", "uint8", "Percentage"),
                make_field("model_file_name", "zstring", "Model FileName"),
                make_field("has_collision", "uint8", "Has Collision"),
            ],
            repeatable: true,
            required: true,
            localized: false,
            enum_ref: None,
            formlink_target: None,
            formlink_targets: Vec::new(),
            null_allowed: false,
            union_selector: None,
            union_variants: Vec::new(),
            _array: None,
            row_label: None,
            authoring_layout: None,
            authoring_key: None,
            scope_id: Some("models".to_string()),
        }
    }

    #[test]
    fn variable_struct_decodes_interior_zstring() {
        let payload = [75, b'M', b'e', b's', b'h', 0, 1];
        let spec = make_debr_data_spec();
        let schema = make_empty_schema();
        let decode_spec = schema_subrecord_to_decode_spec(&spec, &schema)
            .expect("DEBR.DATA should build a variable struct decode spec");
        match &decode_spec {
            crate::DecodeSpec::VariableStruct { segments, .. } => {
                assert_eq!(segments.len(), 3);
            }
            _ => panic!("expected DecodeSpec::VariableStruct"),
        }
        let ctx: std::collections::HashMap<String, serde_json::Value> =
            std::collections::HashMap::new();
        let value = decode_subrecord_json(&decode_spec, &payload, &ctx)
            .expect("DEBR.DATA payload should decode");
        assert_eq!(
            value,
            serde_json::json!({
                "percentage": 75,
                "model_file_name": "Mesh",
                "has_collision": 1
            })
        );
    }

    #[test]
    fn variable_struct_decode_spec_skips_absent_conditional_bytes_field() {
        let mut future_bytes = make_field("future_bytes", "bytes", "Future Bytes");
        future_bytes
            .presence_conditions
            .push(make_form_version_condition("gte", 208));
        let spec = SchemaSubrecordJson {
            id: "BPND".to_string(),
            kind: "parsed_with_raw_fallback".to_string(),
            display_label: Some("Node Data".to_string()),
            codec: Some("struct:B".to_string()),
            fields: vec![
                make_array_field("values", "uint8", "B", "Values"),
                future_bytes,
            ],
            repeatable: true,
            required: false,
            localized: false,
            enum_ref: None,
            formlink_target: None,
            formlink_targets: Vec::new(),
            null_allowed: false,
            union_selector: None,
            union_variants: Vec::new(),
            _array: None,
            row_label: None,
            authoring_layout: None,
            authoring_key: None,
            scope_id: None,
        };
        let schema = make_empty_schema();
        let decode_spec = schema_subrecord_to_decode_spec(&spec, &schema)
            .expect("absent conditional bytes field must not disable structured decode");
        let mut context = std::collections::HashMap::new();
        context.insert("record_form_version".to_string(), serde_json::json!(131));

        let value = decode_subrecord_json(&decode_spec, &[2, 0xAA, 0xBB], &context)
            .expect("form-version 131 payload should decode without the future bytes tail");
        let obj = value.as_object().unwrap();
        assert_eq!(obj.get("values"), Some(&serde_json::json!([0xAA, 0xBB])));
        assert!(!obj.contains_key("future_bytes"));
    }

    #[test]
    fn bpnd_like_field_union_uses_variable_width_decode() {
        let mut future_bytes = make_field("future_bytes", "bytes", "Future Bytes");
        future_bytes
            .presence_conditions
            .push(make_form_version_condition("gte", 208));
        let spec = SchemaSubrecordJson {
            id: "BPND".to_string(),
            kind: "parsed_with_raw_fallback".to_string(),
            display_label: Some("Node Data".to_string()),
            codec: Some("struct:B,B,B".to_string()),
            fields: vec![
                make_field("prefix", "uint8", "Prefix"),
                make_actor_value_union_field(),
                make_field("after", "uint8", "After"),
                future_bytes,
            ],
            repeatable: true,
            required: false,
            localized: false,
            enum_ref: None,
            formlink_target: None,
            formlink_targets: Vec::new(),
            null_allowed: false,
            union_selector: None,
            union_variants: Vec::new(),
            _array: None,
            row_label: None,
            authoring_layout: None,
            authoring_key: None,
            scope_id: None,
        };
        let schema = make_empty_schema();
        let decode_spec = schema_subrecord_to_decode_spec(&spec, &schema)
            .expect("BPND-like field union should build a decode spec");
        let mut context = std::collections::HashMap::new();
        context.insert("record_form_version".to_string(), serde_json::json!(131));

        let value = decode_subrecord_json(
            &decode_spec,
            &[0x11, 0x78, 0x56, 0x34, 0x12, 0x22],
            &context,
        )
        .expect("form-version 131 union payload should decode as prefix + formid + after");
        let obj = value.as_object().unwrap();
        assert_eq!(obj.get("prefix"), Some(&serde_json::json!(0x11)));
        assert_eq!(
            obj.get("actor_value"),
            Some(&serde_json::json!({
                "variant": "actor_value",
                "value": 0x12345678_u64
            }))
        );
        assert_eq!(obj.get("after"), Some(&serde_json::json!(0x22)));
        assert!(!obj.contains_key("future_bytes"));
    }

    #[test]
    fn fo76_bpnd_schema_uses_variable_struct_decode_spec() {
        let schema = compiled_schema_for_game("fo76").unwrap();
        let record = schema.records.get("BPTD").unwrap();
        let spec = record
            .subrecords
            .iter()
            .find(|subrecord| subrecord.id == "BPND")
            .unwrap();
        let decode_spec = schema_subrecord_to_decode_spec(spec, schema.as_ref())
            .expect("FO76 BPTD.BPND should build a structured decode spec");

        match decode_spec {
            crate::DecodeSpec::VariableStruct { segments, .. } => {
                assert!(
                    segments
                        .iter()
                        .any(|segment| segment.name() == "actor_value")
                );
                assert!(segments.iter().any(|segment| segment.name() == "bytes_37"));
            }
            _ => panic!("FO76 BPTD.BPND must use VariableStruct for the actor_value union"),
        }
    }

    fn make_nvnm_subrecord_spec() -> SchemaSubrecordJson {
        let mut spec = make_subrecord_spec("NVNM", true, false);
        spec.kind = "custom_codec".to_string();
        spec.codec = Some("esp_authoring_core::nvnm".to_string());
        spec
    }

    fn build_nvnm_bytes_with_door_and_exterior_parent() -> Vec<u8> {
        use crate::nvnm::{
            NvnmDoorRef, NvnmGrid, NvnmParent, NvnmPayload, NvnmTriangle, NvnmVertex,
        };
        let payload = NvnmPayload {
            version: 15,
            flags: 0,
            parent: NvnmParent::Exterior {
                world: 0x0025_DA15,
                grid_x: 1,
                grid_y: 2,
            },
            vertices: vec![
                NvnmVertex {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                NvnmVertex {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                },
                NvnmVertex {
                    x: 0.0,
                    y: 1.0,
                    z: 0.0,
                },
            ],
            triangles: vec![NvnmTriangle {
                vertices: [0, 1, 2],
                links: [-1, -1, -1],
                cover_marker: [0u8; 9],
                flags: 0,
            }],
            edge_links: vec![],
            door_refs: vec![
                NvnmDoorRef {
                    triangle_index: 0,
                    padding: [0xAA, 0xBB, 0xCC, 0xDD],
                    door_ref_form_id: 0x0010_0042,
                },
                NvnmDoorRef {
                    triangle_index: 0,
                    padding: [0, 0, 0, 0],
                    door_ref_form_id: 0x0020_5678,
                },
            ],
            cover_array: vec![],
            cover_triangle_mappings: vec![],
            waypoints: vec![],
            grid: NvnmGrid::default(),
        };
        crate::nvnm::write_nvnm(&payload)
    }

    #[test]
    fn extract_nested_form_ids_collects_nvnm_door_refs_and_exterior_world() {
        let spec = make_nvnm_subrecord_spec();
        let schema = make_empty_schema();
        let bytes = build_nvnm_bytes_with_door_and_exterior_parent();
        let mut out = Vec::new();
        extract_nested_form_ids(&spec, &schema, &bytes, &mut out);
        assert!(
            out.contains(&0x0025_DA15),
            "exterior world form_id missing: {:?}",
            out
        );
        assert!(
            out.contains(&0x0010_0042),
            "door[0] form_id missing: {:?}",
            out
        );
        assert!(
            out.contains(&0x0020_5678),
            "door[1] form_id missing: {:?}",
            out
        );
    }

    #[test]
    fn rewrite_schema_form_ids_rewrites_nvnm_door_refs_and_parent() {
        let spec = make_nvnm_subrecord_spec();
        let schema = make_empty_schema();
        let mut bytes = build_nvnm_bytes_with_door_and_exterior_parent();
        let original_len = bytes.len();
        let mut rewrite = |raw: u32| -> Option<u32> {
            // Add 0x01000000 to every FormID.
            Some(raw.wrapping_add(0x0100_0000))
        };
        let changed =
            rewrite_schema_form_ids_in_subrecord(&spec, &schema, &mut bytes, &mut rewrite);
        assert!(changed, "expected NVNM rewrite to report mutation");
        assert_eq!(
            bytes.len(),
            original_len,
            "NVNM length must be preserved by FormID rewrite"
        );
        let reparsed = crate::nvnm::parse_nvnm(&bytes).expect("reparse");
        match reparsed.parent {
            crate::nvnm::NvnmParent::Exterior {
                world,
                grid_x,
                grid_y,
            } => {
                assert_eq!(world, 0x0125_DA15, "world form_id must be remapped");
                assert_eq!(grid_x, 1);
                assert_eq!(grid_y, 2);
            }
            _ => panic!("expected Exterior parent"),
        }
        assert_eq!(reparsed.door_refs.len(), 2);
        assert_eq!(reparsed.door_refs[0].door_ref_form_id, 0x0110_0042);
        assert_eq!(reparsed.door_refs[1].door_ref_form_id, 0x0120_5678);
        // Padding bytes survive the FormID-only rewrite (codec is structural).
        assert_eq!(reparsed.door_refs[0].padding, [0xAA, 0xBB, 0xCC, 0xDD]);
    }

    #[test]
    fn rewrite_schema_form_ids_nvnm_returns_false_when_policy_keeps_all() {
        let spec = make_nvnm_subrecord_spec();
        let schema = make_empty_schema();
        let mut bytes = build_nvnm_bytes_with_door_and_exterior_parent();
        let original = bytes.clone();
        let mut rewrite = |_raw: u32| -> Option<u32> { None };
        let changed =
            rewrite_schema_form_ids_in_subrecord(&spec, &schema, &mut bytes, &mut rewrite);
        assert!(!changed);
        assert_eq!(bytes, original);
    }
}
