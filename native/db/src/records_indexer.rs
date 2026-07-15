//! Index ESP authoring-dir record YAML into the records SQLite/FTS5 database.
//!
//! Walks `<yaml_root>/<plugin>/records/<SIG>/*.yaml`, parses each file with
//! `serde-saphyr`, extracts the searchable fields (display name, keywords,
//! cross-references, ADDN node index), and bulk-inserts into the `records`
//! and `record_refs` tables. Replaces the prior pure-Python pipeline in
//! `py_creation_lib/python/creation_lib/preprocessor/records.py`.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Instant;

use rayon::prelude::*;
use rusqlite::Connection;
use rusqlite::types::Value as SqlValue;
use serde_json::Value as JsonValue;

use crate::bulk::BulkInserter;
use crate::error::{DbError, DbResult};
use crate::tokenizer::tokenize_str;

/// zstd compression level for `records.content`. L9 compresses to 39.2% of raw
/// vs L19's 38.7% — the extra 0.5 pp costs 12× the indexer time.
const RECORDS_CONTENT_ZSTD_LEVEL: i32 = 9;

/// Compress YAML text for the `records.content` BLOB column. Always returns a
/// valid zstd frame; empty input produces an empty zstd frame, not `Vec::new()`,
/// so the magic-byte invariant in `decompress_content` always holds.
pub fn compress_content(text: &str) -> Vec<u8> {
    zstd::stream::encode_all(text.as_bytes(), RECORDS_CONTENT_ZSTD_LEVEL)
        .expect("zstd encode of in-memory buffer cannot fail")
}

/// Decompress a `records.content` BLOB. Tolerates the legacy schema where the
/// column was TEXT — a plain UTF-8 byte slice with no zstd magic falls through.
pub fn decompress_content(blob: &[u8]) -> DbResult<String> {
    if blob.is_empty() {
        return Ok(String::new());
    }
    // Legacy TEXT rows: just decode as UTF-8. zstd frames start with the magic
    // 0x28 0xB5 0x2F 0xFD; anything else is treated as raw text.
    if blob.len() < 4 || blob[..4] != [0x28, 0xb5, 0x2f, 0xfd] {
        return std::str::from_utf8(blob)
            .map(|s| s.to_owned())
            .map_err(|e| DbError::Other(format!("records.content legacy TEXT not utf-8: {e}")));
    }
    let mut decoder = zstd::stream::read::Decoder::new(blob)
        .map_err(|e| DbError::Other(format!("zstd decoder init failed: {e}")))?;
    let mut out = String::new();
    decoder
        .read_to_string(&mut out)
        .map_err(|e| DbError::Other(format!("zstd decode failed: {e}")))?;
    Ok(out)
}

const RECORD_COLS: &[&str] = &[
    "form_key",
    "editor_id",
    "editor_id_tokens",
    "record_type",
    "name",
    "name_tokens",
    "source",
    "keywords",
    "yaml_path",
    "content",
    "node_index",
];

const REF_COLS: &[&str] = &["referencing_form_key", "referenced_form_key"];

const ADDN_SIGNATURE: &str = "ADDN";
const MAX_CONTENT_LEN: usize = 4096;
const FLUSH_THRESHOLD_RECORDS: usize = 4000;
const FLUSH_THRESHOLD_REFS: usize = 8000;

#[derive(Debug, Clone, Default)]
pub struct RecordsIndexSummary {
    pub indexed: usize,
    pub refs: usize,
    pub elapsed_seconds: f64,
    pub type_counts: HashMap<String, usize>,
    pub source_counts: HashMap<String, usize>,
}

#[derive(Debug)]
struct RecordTask {
    yaml_path: PathBuf,
    file_name: String,
    record_type: String,
}

#[derive(Debug)]
struct ExtractedRecord {
    form_key: String,
    editor_id: String,
    editor_id_tokens: String,
    record_type: String,
    name: String,
    name_tokens: String,
    source: String,
    keywords: String,
    yaml_path: String,
    content: String,
    node_index: Option<i64>,
    refs: Vec<String>,
}

pub fn index_records_to_bulk(
    bulk: &mut BulkInserter,
    yaml_root: &Path,
    sources_filter: Option<&HashSet<String>>,
    workers: usize,
) -> DbResult<RecordsIndexSummary> {
    let t0 = Instant::now();

    let tasks = collect_tasks(yaml_root)?;

    let extract = || tasks.par_iter().filter_map(extract_one).collect::<Vec<_>>();
    let extracted: Vec<ExtractedRecord> = if workers > 0 {
        rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .map_err(|e| DbError::Other(format!("rayon pool error: {e}")))?
            .install(extract)
    } else {
        extract()
    };

    let mut summary = RecordsIndexSummary::default();
    let mut buf = ColumnarBuffer::new();

    for rec in extracted {
        if let Some(filter) = sources_filter {
            if !filter.contains(&rec.source) {
                continue;
            }
        }
        *summary
            .type_counts
            .entry(rec.record_type.clone())
            .or_insert(0) += 1;
        *summary.source_counts.entry(rec.source.clone()).or_insert(0) += 1;
        summary.indexed += 1;
        summary.refs += rec.refs.len();
        buf.append(&rec);
        buf.flush_if_full(bulk)?;
    }

    buf.flush_all(bulk)?;
    summary.elapsed_seconds = t0.elapsed().as_secs_f64();
    Ok(summary)
}

fn collect_tasks(yaml_root: &Path) -> DbResult<Vec<RecordTask>> {
    if !yaml_root.is_dir() {
        return Err(DbError::Other(format!(
            "ESM YAML directory not found: {}",
            yaml_root.display()
        )));
    }
    let mut tasks = Vec::new();
    for plugin_dir in read_dir_sorted(yaml_root)? {
        if !plugin_dir.is_dir() {
            continue;
        }
        let records_root = plugin_dir.join("records");
        if !records_root.is_dir() {
            continue;
        }
        for type_dir in read_dir_sorted(&records_root)? {
            if !type_dir.is_dir() {
                continue;
            }
            let record_type = match type_dir.file_name().and_then(|n| n.to_str()) {
                Some(name) => name.to_string(),
                None => continue,
            };
            for entry in fs::read_dir(&type_dir)? {
                let entry = entry?;
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                let file_name = match path.file_name().and_then(|n| n.to_str()) {
                    Some(name) if name.ends_with(".yaml") => name.to_string(),
                    _ => continue,
                };
                tasks.push(RecordTask {
                    yaml_path: path,
                    file_name,
                    record_type: record_type.clone(),
                });
            }
        }
    }
    Ok(tasks)
}

fn read_dir_sorted(path: &Path) -> DbResult<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for entry in fs::read_dir(path)? {
        paths.push(entry?.path());
    }
    paths.sort();
    Ok(paths)
}

fn extract_one(task: &RecordTask) -> Option<ExtractedRecord> {
    let (editor_id, form_key) = parse_filename(&task.file_name)?;
    let source = form_key
        .split_once(':')
        .map(|(_, s)| s.to_string())
        .unwrap_or_default();

    let yaml_text = fs::read_to_string(&task.yaml_path).ok()?;

    // Match the parser settings used by the ESP authoring import path:
    // disable serde-saphyr's anti-amplification budget so large records
    // (CELL/QUST) don't get rejected.
    let opts = serde_saphyr::options! {
        budget: None,
    };
    let parsed: JsonValue = serde_saphyr::from_str_with_options(&yaml_text, opts).ok()?;

    let mut display_name = String::new();
    let mut keywords: Vec<String> = Vec::new();
    let mut node_index: Option<i64> = None;
    let mut refs: Vec<String> = Vec::new();

    if let Some(obj) = parsed.as_object() {
        if let Some(JsonValue::Array(fields)) = obj.get("fields") {
            for entry in fields {
                let map = match entry.as_object() {
                    Some(m) if m.len() == 1 => m,
                    _ => continue,
                };
                let (key, value) = map.iter().next().unwrap();
                match key.as_str() {
                    // Display-name fields. Bethesda records use either `Name`
                    // (AMMO, MISC, NPC_) or `FULL` (ARMO, WEAP) — accept either.
                    "Name" | "FULL" if display_name.is_empty() => {
                        display_name = resolve_localized(value);
                    }
                    "Keywords" if keywords.is_empty() => {
                        keywords = extract_keyword_form_keys(value);
                    }
                    "Index" if task.record_type == ADDN_SIGNATURE => {
                        if let Some(n) = value.as_i64() {
                            node_index = Some(n);
                        }
                    }
                    _ => {}
                }
            }
        }
        let mut seen_refs: HashSet<String> = HashSet::new();
        walk_form_refs(&parsed, &mut |fk| {
            if fk != form_key && seen_refs.insert(fk.clone()) {
                refs.push(fk);
            }
        });
    }

    let editor_id_tokens = tokenize_str(&editor_id);
    let name_tokens = if display_name.is_empty() {
        String::new()
    } else {
        tokenize_str(&display_name)
    };
    let keywords_str = keywords.join(" ");
    let content = truncate_to_chars(&yaml_text, MAX_CONTENT_LEN);
    let yaml_path = task.yaml_path.to_string_lossy().to_string();

    Some(ExtractedRecord {
        form_key,
        editor_id,
        editor_id_tokens,
        record_type: task.record_type.clone(),
        name: display_name,
        name_tokens,
        source,
        keywords: keywords_str,
        yaml_path,
        content,
        node_index,
        refs,
    })
}

fn parse_filename(filename: &str) -> Option<(String, String)> {
    let stem = filename.strip_suffix(".yaml")?;
    let (editor_id, fk_raw) = stem.split_once(" - ")?;
    let editor_id = editor_id.trim().to_string();
    let fk_raw = fk_raw.trim();
    let form_key = match fk_raw.find('_') {
        Some(idx) if idx > 0 => format!("{}:{}", &fk_raw[..idx], &fk_raw[idx + 1..]),
        _ => fk_raw.to_string(),
    };
    Some((editor_id, form_key))
}

fn resolve_localized(value: &JsonValue) -> String {
    if let Some(s) = value.as_str() {
        return s.to_string();
    }
    let obj = match value.as_object() {
        Some(o) => o,
        None => return String::new(),
    };
    // Current authoring shape: { TargetLanguage, Values: [{Language, String}] }.
    // Strings are inline, so no .STRINGS sidecar lookup needed.
    if let Some(JsonValue::Array(values)) = obj.get("Values") {
        let mut first_nonempty: Option<&str> = None;
        for v in values {
            let m = match v.as_object() {
                Some(m) => m,
                None => continue,
            };
            let lang = m.get("Language").and_then(|x| x.as_str()).unwrap_or("");
            let s = m.get("String").and_then(|x| x.as_str()).unwrap_or("");
            if s.is_empty() {
                continue;
            }
            if lang.eq_ignore_ascii_case("English") {
                return s.to_string();
            }
            if first_nonempty.is_none() {
                first_nonempty = Some(s);
            }
        }
        if let Some(s) = first_nonempty {
            return s.to_string();
        }
    }
    // Legacy shape that older exporter emissions used: { raw_hex }.
    // Strings tables aren't available here, so the raw hex stands in as
    // a stable identifier that keeps the row searchable.
    if let Some(rh) = obj.get("raw_hex").and_then(|v| v.as_str()) {
        return rh.to_string();
    }
    String::new()
}

fn extract_keyword_form_keys(value: &JsonValue) -> Vec<String> {
    let arr = match value.as_array() {
        Some(a) => a,
        None => return Vec::new(),
    };
    let mut out = Vec::new();
    for entry in arr {
        if let Some(m) = entry.as_object() {
            if let Some(reference) = m.get("reference") {
                if let Some(fk) = form_key_string(reference) {
                    out.push(fk);
                }
            }
        }
    }
    out
}

fn form_key_string(value: &JsonValue) -> Option<String> {
    let obj = value.as_object()?;
    let plugin = obj.get("plugin").and_then(|v| v.as_str())?;
    if plugin.is_empty() {
        return None;
    }
    let obj_id = obj.get("object_id")?;
    let raw = match obj_id {
        JsonValue::String(s) => s.clone(),
        JsonValue::Number(n) => {
            if let Some(i) = n.as_u64() {
                format!("{:X}", i)
            } else {
                return None;
            }
        }
        _ => return None,
    };
    let upper = raw.to_uppercase();
    let padded = if upper.len() < 6 {
        format!("{:0>6}", upper)
    } else {
        upper
    };
    Some(format!("{}:{}", padded, plugin))
}

fn walk_form_refs<F: FnMut(String)>(node: &JsonValue, out: &mut F) {
    match node {
        JsonValue::Object(map) => {
            if let Some(reference) = map.get("reference") {
                if let Some(fk) = form_key_string(reference) {
                    out(fk);
                }
            }
            for (k, v) in map {
                if k == "reference" {
                    continue;
                }
                walk_form_refs(v, out);
            }
        }
        JsonValue::Array(arr) => {
            for v in arr {
                walk_form_refs(v, out);
            }
        }
        _ => {}
    }
}

/// Truncate `text` to at most `max_chars` Unicode chars without splitting
/// a character. Avoids the `text[..n]` byte-slice panic when `n` lands
/// mid-codepoint.
fn truncate_to_chars(text: &str, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        Some((idx, _)) => text[..idx].to_string(),
        None => text.to_string(),
    }
}

struct ColumnarBuffer {
    records: HashMap<String, Vec<SqlValue>>,
    refs: HashMap<String, Vec<SqlValue>>,
}

impl ColumnarBuffer {
    fn new() -> Self {
        let mut records = HashMap::new();
        for col in RECORD_COLS {
            records.insert((*col).to_string(), Vec::new());
        }
        let mut refs = HashMap::new();
        for col in REF_COLS {
            refs.insert((*col).to_string(), Vec::new());
        }
        Self { records, refs }
    }

    fn append(&mut self, rec: &ExtractedRecord) {
        push_text(&mut self.records, "form_key", &rec.form_key);
        push_text(&mut self.records, "editor_id", &rec.editor_id);
        push_text(&mut self.records, "editor_id_tokens", &rec.editor_id_tokens);
        push_text(&mut self.records, "record_type", &rec.record_type);
        push_text(&mut self.records, "name", &rec.name);
        push_text(&mut self.records, "name_tokens", &rec.name_tokens);
        push_text(&mut self.records, "source", &rec.source);
        push_text(&mut self.records, "keywords", &rec.keywords);
        push_text(&mut self.records, "yaml_path", &rec.yaml_path);
        push_blob(&mut self.records, "content", compress_content(&rec.content));
        match rec.node_index {
            Some(n) => push_int(&mut self.records, "node_index", n),
            None => push_null(&mut self.records, "node_index"),
        }

        for r in &rec.refs {
            push_text(&mut self.refs, "referencing_form_key", &rec.form_key);
            push_text(&mut self.refs, "referenced_form_key", r);
        }
    }

    fn flush_if_full(&mut self, bulk: &mut BulkInserter) -> DbResult<()> {
        if column_len(&self.records) >= FLUSH_THRESHOLD_RECORDS {
            flush_table(bulk, "records", &mut self.records)?;
        }
        if column_len(&self.refs) >= FLUSH_THRESHOLD_REFS {
            flush_table(bulk, "record_refs", &mut self.refs)?;
        }
        Ok(())
    }

    fn flush_all(mut self, bulk: &mut BulkInserter) -> DbResult<()> {
        if column_len(&self.records) > 0 {
            flush_table(bulk, "records", &mut self.records)?;
        }
        if column_len(&self.refs) > 0 {
            flush_table(bulk, "record_refs", &mut self.refs)?;
        }
        Ok(())
    }
}

fn flush_table(
    bulk: &mut BulkInserter,
    table: &str,
    cols: &mut HashMap<String, Vec<SqlValue>>,
) -> DbResult<()> {
    let snapshot: HashMap<String, Vec<SqlValue>> = cols.drain().map(|(k, v)| (k, v)).collect();
    bulk.add_chunk(table, snapshot)?;
    // Re-init empty columns so the buffer is reusable.
    let init_cols: &[&str] = if table == "records" {
        RECORD_COLS
    } else {
        REF_COLS
    };
    for col in init_cols {
        cols.insert((*col).to_string(), Vec::new());
    }
    Ok(())
}

fn column_len(cols: &HashMap<String, Vec<SqlValue>>) -> usize {
    cols.values().next().map(|v| v.len()).unwrap_or(0)
}

fn push_text(cols: &mut HashMap<String, Vec<SqlValue>>, col: &str, value: &str) {
    cols.get_mut(col)
        .expect("column exists")
        .push(SqlValue::Text(value.to_string()));
}

fn push_blob(cols: &mut HashMap<String, Vec<SqlValue>>, col: &str, value: Vec<u8>) {
    cols.get_mut(col)
        .expect("column exists")
        .push(SqlValue::Blob(value));
}

fn push_int(cols: &mut HashMap<String, Vec<SqlValue>>, col: &str, value: i64) {
    cols.get_mut(col)
        .expect("column exists")
        .push(SqlValue::Integer(value));
}

fn push_null(cols: &mut HashMap<String, Vec<SqlValue>>, col: &str) {
    cols.get_mut(col)
        .expect("column exists")
        .push(SqlValue::Null);
}

/// Rebuild the `records_fts` inverted index after `records.content` switched
/// to a zstd BLOB.
///
/// FTS5's built-in `INSERT INTO records_fts(records_fts) VALUES('rebuild')`
/// would tokenize the raw zstd bytes as an empty string (FTS5 treats non-text
/// values as ""), so content-only terms would silently vanish from search. We
/// instead clear the inverted index and re-insert each row with the
/// decompressed text.
pub fn rebuild_records_fts_decompressed(conn: &mut Connection) -> DbResult<()> {
    // FTS5's 'delete-all' command works on external-content tables and clears
    // the inverted index without trying to repopulate from the content table.
    conn.execute(
        "INSERT INTO records_fts(records_fts) VALUES('delete-all')",
        [],
    )
    .map_err(DbError::from)?;

    // The caller (BulkInserter) already holds an outer transaction; nesting a
    // new one would error with "cannot start a transaction within a
    // transaction". Run the prepared statements on the existing connection.
    let mut select = conn
        .prepare("SELECT rowid, editor_id_tokens, name_tokens, keywords, content FROM records")
        .map_err(DbError::from)?;
    let mut insert = conn
        .prepare(
            "INSERT INTO records_fts(rowid, editor_id_tokens, name_tokens, keywords, content) \
             VALUES(?1, ?2, ?3, ?4, ?5)",
        )
        .map_err(DbError::from)?;
    let mut rows = select.query([]).map_err(DbError::from)?;
    while let Some(row) = rows.next().map_err(DbError::from)? {
        let rid: i64 = row.get(0).map_err(DbError::from)?;
        let eit: Option<String> = row.get(1).map_err(DbError::from)?;
        let nt: Option<String> = row.get(2).map_err(DbError::from)?;
        let kw: Option<String> = row.get(3).map_err(DbError::from)?;
        let blob: Option<Vec<u8>> = row.get(4).map_err(DbError::from)?;
        let text = match blob {
            Some(b) => decompress_content(&b)?,
            None => String::new(),
        };
        insert
            .execute(rusqlite::params![rid, eit, nt, kw, text])
            .map_err(DbError::from)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_filename_splits_editor_id_form_id_plugin() {
        let (eid, fk) = parse_filename("Ammo10mm - 01F276_Fallout4.esm.yaml").unwrap();
        assert_eq!(eid, "Ammo10mm");
        assert_eq!(fk, "01F276:Fallout4.esm");
    }

    #[test]
    fn parse_filename_handles_quoted_form_id() {
        let (eid, fk) = parse_filename(
            "ccBGSFO4044_Armor_Power_Hellfire_ArmLeft - 000800_ccBGSFO4044-HellfirePowerArmor.esl.yaml",
        )
        .unwrap();
        assert_eq!(eid, "ccBGSFO4044_Armor_Power_Hellfire_ArmLeft");
        assert_eq!(fk, "000800:ccBGSFO4044-HellfirePowerArmor.esl");
    }

    #[test]
    fn form_key_string_pads_short_object_id() {
        let v = serde_json::json!({"plugin": "Fallout4.esm", "object_id": "F4AE8"});
        assert_eq!(form_key_string(&v).as_deref(), Some("0F4AE8:Fallout4.esm"));
    }

    #[test]
    fn form_key_string_uppercases_hex() {
        let v = serde_json::json!({"plugin": "Fallout4.esm", "object_id": "1cc46a"});
        assert_eq!(form_key_string(&v).as_deref(), Some("1CC46A:Fallout4.esm"));
    }

    #[test]
    fn resolve_localized_prefers_english() {
        let v = serde_json::json!({
            "TargetLanguage": "English",
            "Values": [
                {"Language": "Chinese", "String": "10mm彈藥"},
                {"Language": "English", "String": "10mm Round"},
                {"Language": "German", "String": "10-mm-Patrone"},
            ],
        });
        assert_eq!(resolve_localized(&v), "10mm Round");
    }

    #[test]
    fn resolve_localized_falls_back_to_first_nonempty() {
        let v = serde_json::json!({
            "TargetLanguage": "English",
            "Values": [
                {"Language": "ChineseTraditional", "String": ""},
                {"Language": "German", "String": "Fallback"},
            ],
        });
        assert_eq!(resolve_localized(&v), "Fallback");
    }

    #[test]
    fn resolve_localized_passes_plain_string() {
        let v = serde_json::Value::String("Plain".to_string());
        assert_eq!(resolve_localized(&v), "Plain");
    }

    #[test]
    fn resolve_localized_uses_raw_hex_when_no_values() {
        let v = serde_json::json!({"TargetLanguage": "English", "raw_hex": "DB030000"});
        assert_eq!(resolve_localized(&v), "DB030000");
    }

    #[test]
    fn walk_form_refs_collects_nested_references() {
        let v = serde_json::json!({
            "fields": [
                {"PreviewTransform": {"reference": {"plugin": "Fallout4.esm", "object_id": "07B9C1"}}},
                {"Keywords": [
                    {"reference": {"plugin": "Fallout4.esm", "object_id": "0F4AE8"}},
                    {"reference": {"plugin": "DLCRobot.esm", "object_id": "111111"}},
                ]},
                {"DNAM": {"Projectile": {"reference": {"plugin": "DLCRobot.esm", "object_id": "004174"}}}},
            ],
        });
        let mut refs = Vec::new();
        walk_form_refs(&v, &mut |fk| refs.push(fk));
        refs.sort();
        assert_eq!(
            refs,
            vec![
                "004174:DLCRobot.esm".to_string(),
                "07B9C1:Fallout4.esm".to_string(),
                "0F4AE8:Fallout4.esm".to_string(),
                "111111:DLCRobot.esm".to_string(),
            ]
        );
    }

    #[test]
    fn extract_keyword_form_keys_handles_padding() {
        let v = serde_json::json!([
            {"reference": {"plugin": "Fallout4.esm", "object_id": "0F4AE8"}},
            {"reference": {"plugin": "ccX.esl", "object_id": "812"}},
        ]);
        assert_eq!(
            extract_keyword_form_keys(&v),
            vec![
                "0F4AE8:Fallout4.esm".to_string(),
                "000812:ccX.esl".to_string()
            ]
        );
    }

    #[test]
    fn truncate_to_chars_handles_unicode() {
        // 4-byte char to ensure we don't slice in the middle.
        let s = "abc🎯def";
        assert_eq!(truncate_to_chars(s, 4), "abc🎯");
        assert_eq!(truncate_to_chars(s, 2), "ab");
        assert_eq!(truncate_to_chars(s, 100), "abc🎯def");
    }
}
