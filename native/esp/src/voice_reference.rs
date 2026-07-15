use crate::plugin_runtime::{
    LocalizedStringsState, ParsedItem, ParsedPlugin, ParsedRecord, ParsedSubrecord,
    parse_plugin_file, strings,
};
use encoding_rs::WINDOWS_1252;
use pyo3::exceptions::{PyIOError, PyRuntimeError};
use pyo3::prelude::*;
use rusqlite::{Connection, params};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

const TES4_FLAG_LOCALIZED: u32 = 0x00000080;
const LOCAL_FORM_INDEX: u8 = 0xFF;
const SCHEMA_VERSION: &str = "3";

#[derive(Clone)]
struct ArchiveEntry {
    archive_path: String,
    member_path: String,
    plugin: String,
    voice_type: String,
    filename: String,
    form_id: u32,
    index: Option<u32>,
}

#[derive(Clone)]
struct ResponseSeed {
    plugin: String,
    info_form_id: u32,
    response_number: u32,
    response_text: String,
    topic_form_id: u32,
    topic_text: String,
    voice_form_id: u32,
}

#[derive(Clone)]
struct VoiceLineRow {
    game: String,
    plugin: String,
    info_form_id: String,
    response_number: u32,
    response_text: String,
    response_filename: String,
    voice_type: String,
    characters: Vec<String>,
    archive_path: String,
    member_path: String,
    topic_form_id: String,
    topic_text: String,
}

#[pyfunction(name = "voice_reference_build_index")]
#[pyo3(signature = (db_path, game, data_dir, strings_dir, language, cache_key, plugin_paths, archive_paths, force=false))]
pub(crate) fn voice_reference_build_index_native(
    py: Python<'_>,
    db_path: &str,
    game: &str,
    data_dir: &str,
    strings_dir: &str,
    language: &str,
    cache_key: &str,
    plugin_paths: Vec<String>,
    archive_paths: Vec<String>,
    force: bool,
) -> PyResult<(String, i64, bool)> {
    let db_path = PathBuf::from(db_path);
    let game = game.to_string();
    let data_dir = data_dir.to_string();
    let strings_dir = strings_dir.to_string();
    let language = language.to_string();
    let cache_key = cache_key.to_string();
    let summary = py.detach(move || {
        build_index_impl(
            &db_path,
            &game,
            &data_dir,
            &strings_dir,
            &language,
            &cache_key,
            &plugin_paths,
            &archive_paths,
            force,
        )
    })?;
    Ok((summary.db_path, summary.line_count, summary.reused))
}

#[pyfunction(name = "voice_reference_read_index")]
pub(crate) fn voice_reference_read_index_native(
    py: Python<'_>,
    db_path: &str,
) -> PyResult<
    Vec<(
        String,
        String,
        String,
        u32,
        String,
        String,
        String,
        Vec<String>,
        String,
        String,
        String,
        String,
    )>,
> {
    let db_path = PathBuf::from(db_path);
    let rows = py.detach(move || read_lines_impl(&db_path))?;
    Ok(rows
        .into_iter()
        .map(|row| {
            (
                row.game,
                row.plugin,
                row.info_form_id,
                row.response_number,
                row.response_text,
                row.response_filename,
                row.voice_type,
                row.characters,
                row.archive_path,
                row.member_path,
                row.topic_form_id,
                row.topic_text,
            )
        })
        .collect())
}

struct BuildSummary {
    db_path: String,
    line_count: i64,
    reused: bool,
}

fn build_index_impl(
    db_path: &Path,
    game: &str,
    data_dir: &str,
    strings_dir: &str,
    language: &str,
    cache_key: &str,
    plugin_paths: &[String],
    archive_paths: &[String],
    force: bool,
) -> PyResult<BuildSummary> {
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| PyIOError::new_err(err.to_string()))?;
    }
    let mut conn = Connection::open(db_path).map_err(sql_err)?;
    ensure_schema(&conn)?;
    if !force
        && meta_value(&conn, "cache_key")?.as_deref() == Some(cache_key)
        && meta_value(&conn, "schema_version")?.as_deref() == Some(SCHEMA_VERSION)
    {
        let line_count = count_lines(&conn)?;
        return Ok(BuildSummary {
            db_path: db_path.display().to_string(),
            line_count,
            reused: true,
        });
    }

    let started = Instant::now();
    let archive_entries = scan_archives(archive_paths)?;
    eprintln!(
        "[voice-reference-native] scanned {} archive voice member(s) in {:.2}s",
        archive_entries.len(),
        started.elapsed().as_secs_f32()
    );
    let lines = build_lines(game, strings_dir, language, plugin_paths, &archive_entries)?;
    write_index(
        &mut conn,
        game,
        data_dir,
        strings_dir,
        language,
        cache_key,
        plugin_paths,
        archive_paths,
        &lines,
    )?;
    Ok(BuildSummary {
        db_path: db_path.display().to_string(),
        line_count: lines.len() as i64,
        reused: false,
    })
}

fn ensure_schema(conn: &Connection) -> PyResult<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS voice_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
    )
    .map_err(sql_err)
}

fn write_index(
    conn: &mut Connection,
    game: &str,
    data_dir: &str,
    strings_dir: &str,
    language: &str,
    cache_key: &str,
    plugin_paths: &[String],
    archive_paths: &[String],
    lines: &[VoiceLineRow],
) -> PyResult<()> {
    let tx = conn.transaction().map_err(sql_err)?;
    tx.execute_batch(
        r#"
        DROP TABLE IF EXISTS voice_lines_fts;
        DROP TABLE IF EXISTS voice_line_characters;
        DROP TABLE IF EXISTS voice_lines;
        DROP TABLE IF EXISTS archives;
        CREATE TABLE archives (
            id   INTEGER PRIMARY KEY,
            path TEXT NOT NULL UNIQUE
        );
        CREATE TABLE voice_lines (
            id              INTEGER PRIMARY KEY,
            game            TEXT NOT NULL,
            plugin          TEXT NOT NULL,
            info_form_id    TEXT NOT NULL,
            response_number INTEGER NOT NULL,
            response_text   TEXT NOT NULL,
            response_filename TEXT NOT NULL,
            voice_type      TEXT NOT NULL,
            archive_id      INTEGER,
            member_path     TEXT NOT NULL,
            topic_form_id   TEXT NOT NULL,
            topic_text      TEXT NOT NULL,
            group_label     TEXT NOT NULL,
            available       INTEGER NOT NULL
        );
        CREATE INDEX idx_voice_lines_group ON voice_lines(group_label);
        CREATE INDEX idx_voice_lines_plugin ON voice_lines(plugin);
        CREATE VIRTUAL TABLE voice_lines_fts USING fts5(search_text, content='', columnsize=0);
        DELETE FROM voice_meta;
        "#,
    )
    .map_err(sql_err)?;

    for (key, val) in [
        ("schema_version", SCHEMA_VERSION),
        ("cache_key", cache_key),
        ("game", game),
        ("data_dir", data_dir),
        ("strings_dir", strings_dir),
        ("language", language),
    ] {
        tx.execute(
            "INSERT INTO voice_meta(key, value) VALUES(?1, ?2)",
            params![key, val],
        )
        .map_err(sql_err)?;
    }
    let joined_plugins = plugin_paths.join("\n");
    let joined_archives = archive_paths.join("\n");
    tx.execute(
        "INSERT INTO voice_meta(key, value) VALUES('plugin_paths', ?1)",
        params![joined_plugins],
    )
    .map_err(sql_err)?;
    tx.execute(
        "INSERT INTO voice_meta(key, value) VALUES('archive_paths', ?1)",
        params![joined_archives],
    )
    .map_err(sql_err)?;

    let mut archive_id_cache: HashMap<String, i64> = HashMap::new();
    for line in lines {
        let archive_id: Option<i64> = if line.archive_path.is_empty() {
            None
        } else if let Some(&id) = archive_id_cache.get(&line.archive_path) {
            Some(id)
        } else {
            tx.execute(
                "INSERT OR IGNORE INTO archives(path) VALUES(?1)",
                params![&line.archive_path],
            )
            .map_err(sql_err)?;
            let id: i64 = tx
                .query_row(
                    "SELECT id FROM archives WHERE path = ?1",
                    params![&line.archive_path],
                    |row| row.get(0),
                )
                .map_err(sql_err)?;
            archive_id_cache.insert(line.archive_path.clone(), id);
            Some(id)
        };

        let group_label = group_label(line);
        let search_text = search_text(line);
        tx.execute(
            r#"INSERT INTO voice_lines(
                game, plugin, info_form_id, response_number, response_text,
                response_filename, voice_type, archive_id, member_path,
                topic_form_id, topic_text, group_label, available
            ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)"#,
            params![
                &line.game,
                &line.plugin,
                &line.info_form_id,
                line.response_number,
                &line.response_text,
                &line.response_filename,
                &line.voice_type,
                archive_id,
                &line.member_path,
                &line.topic_form_id,
                &line.topic_text,
                &group_label,
                if archive_id.is_some() { 1i32 } else { 0i32 },
            ],
        )
        .map_err(sql_err)?;
        let line_id = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO voice_lines_fts(rowid, search_text) VALUES(?1, ?2)",
            params![line_id, &search_text],
        )
        .map_err(sql_err)?;
    }
    tx.commit().map_err(sql_err)?;
    conn.execute_batch("VACUUM;").map_err(sql_err)
}

fn read_lines_impl(db_path: &Path) -> PyResult<Vec<VoiceLineRow>> {
    let conn = Connection::open(db_path).map_err(sql_err)?;
    let schema_version: String = conn
        .query_row(
            "SELECT value FROM voice_meta WHERE key = 'schema_version'",
            [],
            |row| row.get(0),
        )
        .unwrap_or_default();
    if schema_version == SCHEMA_VERSION {
        read_lines_v3(&conn)
    } else if schema_version == "2" {
        read_lines_v2(&conn)
    } else {
        read_lines_v1(&conn)
    }
}

fn read_lines_v3(conn: &Connection) -> PyResult<Vec<VoiceLineRow>> {
    let mut stmt = conn
        .prepare(
            r#"
            SELECT vl.id, vl.game, vl.plugin, vl.info_form_id, vl.response_number,
                   vl.response_text, vl.response_filename, vl.voice_type,
                   COALESCE(a.path, ''), vl.member_path, vl.topic_form_id, vl.topic_text
            FROM voice_lines vl
            LEFT JOIN archives a ON vl.archive_id = a.id
            ORDER BY lower(vl.group_label), lower(vl.plugin), lower(vl.response_filename)
            "#,
        )
        .map_err(sql_err)?;
    collect_voice_line_rows(&mut stmt, conn, false)
}

fn read_lines_v2(conn: &Connection) -> PyResult<Vec<VoiceLineRow>> {
    let mut stmt = conn
        .prepare(
            r#"
            SELECT vl.id, vl.game, vl.plugin, vl.info_form_id, vl.response_number,
                   vl.response_text, vl.response_filename, vl.voice_type,
                   COALESCE(a.path, ''), vl.member_path, vl.topic_form_id, vl.topic_text
            FROM voice_lines vl
            LEFT JOIN archives a ON vl.archive_id = a.id
            ORDER BY lower(vl.group_label), lower(vl.plugin), lower(vl.response_filename)
            "#,
        )
        .map_err(sql_err)?;
    collect_voice_line_rows(&mut stmt, conn, true)
}

fn read_lines_v1(conn: &Connection) -> PyResult<Vec<VoiceLineRow>> {
    let mut stmt = conn
        .prepare(
            r#"
            SELECT id, game, plugin, info_form_id, response_number, response_text,
                   response_filename, voice_type, archive_path, member_path,
                   topic_form_id, topic_text
            FROM voice_lines
            ORDER BY lower(group_label), lower(plugin), lower(response_filename)
            "#,
        )
        .map_err(sql_err)?;
    collect_voice_line_rows(&mut stmt, conn, true)
}

fn collect_voice_line_rows(
    stmt: &mut rusqlite::Statement<'_>,
    conn: &Connection,
    include_characters: bool,
) -> PyResult<Vec<VoiceLineRow>> {
    let mut rows = stmt.query([]).map_err(sql_err)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().map_err(sql_err)? {
        let line_id: i64 = row.get(0).map_err(sql_err)?;
        let characters = if include_characters {
            read_characters(conn, line_id)?
        } else {
            Vec::new()
        };
        out.push(VoiceLineRow {
            game: row.get(1).map_err(sql_err)?,
            plugin: row.get(2).map_err(sql_err)?,
            info_form_id: row.get(3).map_err(sql_err)?,
            response_number: row.get::<_, i64>(4).map_err(sql_err)? as u32,
            response_text: row.get(5).map_err(sql_err)?,
            response_filename: row.get(6).map_err(sql_err)?,
            voice_type: row.get(7).map_err(sql_err)?,
            archive_path: row.get(8).map_err(sql_err)?,
            member_path: row.get(9).map_err(sql_err)?,
            topic_form_id: row.get(10).map_err(sql_err)?,
            topic_text: row.get(11).map_err(sql_err)?,
            characters,
        });
    }
    Ok(out)
}

fn read_characters(conn: &Connection, line_id: i64) -> PyResult<Vec<String>> {
    let mut stmt = conn
        .prepare(
            "SELECT character FROM voice_line_characters WHERE line_id = ?1 ORDER BY character",
        )
        .map_err(sql_err)?;
    let mut rows = stmt.query(params![line_id]).map_err(sql_err)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().map_err(sql_err)? {
        out.push(row.get(0).map_err(sql_err)?);
    }
    Ok(out)
}

fn meta_value(conn: &Connection, key: &str) -> PyResult<Option<String>> {
    let mut stmt = conn
        .prepare("SELECT value FROM voice_meta WHERE key = ?1")
        .map_err(sql_err)?;
    let mut rows = stmt.query(params![key]).map_err(sql_err)?;
    match rows.next().map_err(sql_err)? {
        Some(row) => Ok(Some(row.get(0).map_err(sql_err)?)),
        None => Ok(None),
    }
}

fn count_lines(conn: &Connection) -> PyResult<i64> {
    conn.query_row("SELECT COUNT(*) FROM voice_lines", [], |row| row.get(0))
        .map_err(sql_err)
}

fn scan_archives(archive_paths: &[String]) -> PyResult<Vec<ArchiveEntry>> {
    let mut entries = Vec::new();
    for archive in archive_paths {
        let path = PathBuf::from(archive);
        let members = bsarchive_native::list_archive_files(&path)
            .map_err(|err| PyRuntimeError::new_err(format!("{}: {err}", path.display())))?;
        for member in members {
            if let Some(entry) = parse_voice_member(archive, &member) {
                entries.push(entry);
            }
        }
    }
    Ok(entries)
}

fn parse_voice_member(archive_path: &str, member: &str) -> Option<ArchiveEntry> {
    let normalized = member.replace('\\', "/").to_ascii_lowercase();
    let parts: Vec<&str> = normalized.split('/').collect();
    if parts.len() != 5 || parts[0] != "sound" || parts[1] != "voice" {
        return None;
    }
    let filename = parts[4];
    let stem_ext = filename.rsplit_once('.')?;
    let (stem, ext) = stem_ext;
    if !matches!(ext, "fuz" | "xwm" | "wav" | "ogg" | "wem") {
        return None;
    }
    let (form_id, index) = parse_voice_filename_stem(stem)?;
    Some(ArchiveEntry {
        archive_path: archive_path.to_string(),
        member_path: normalized.clone(),
        plugin: parts[2].to_string(),
        voice_type: parts[3].to_string(),
        filename: filename.to_string(),
        form_id,
        index,
    })
}

fn parse_voice_filename_stem(stem: &str) -> Option<(u32, Option<u32>)> {
    if let Some((head, tail)) = stem.rsplit_once('_') {
        if let Ok(idx) = tail.parse::<u32>() {
            let form_token = head.rsplit_once('_').map(|(_, t)| t).unwrap_or(head);
            if let Some(form) = parse_voice_form_id(form_token) {
                return Some((form, Some(idx)));
            }
        }
    }
    parse_voice_form_id(stem).map(|form| (form, None))
}

fn parse_voice_form_id(token: &str) -> Option<u32> {
    if token.len() != 8 || !token.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(token, 16).ok()
}

fn build_lines(
    game: &str,
    strings_dir: &str,
    language: &str,
    plugin_paths: &[String],
    archive_entries: &[ArchiveEntry],
) -> PyResult<Vec<VoiceLineRow>> {
    let mut archive_by_form: HashMap<(String, u32, Option<u32>), Vec<ArchiveEntry>> =
        HashMap::new();
    for entry in archive_entries {
        archive_by_form
            .entry((entry.plugin.clone(), entry.form_id, entry.index))
            .or_default()
            .push(entry.clone());
    }

    let mut parsed_plugins = Vec::new();
    for path in plugin_paths {
        let started = Instant::now();
        let parsed = parse_plugin_file(path, Some(game.to_string()), true)?;
        let is_localized = (parsed.header.flags & TES4_FLAG_LOCALIZED) != 0;
        let strings = if is_localized {
            strings::hydrate_strings_state(
                path,
                parsed.plugin_name.as_str(),
                Some(strings_dir),
                Some(language),
            )
        } else {
            LocalizedStringsState::default()
        };
        eprintln!(
            "[voice-reference-native] loaded {} in {:.2}s",
            parsed.plugin_name,
            started.elapsed().as_secs_f32()
        );
        parsed_plugins.push((parsed, strings, is_localized));
    }

    let mut response_sets = Vec::new();
    for (parsed, strings, is_localized) in &parsed_plugins {
        let started = Instant::now();
        let records = collect_relevant_records(parsed);
        let topic_labels =
            collect_topic_labels(parsed, strings, language, *is_localized, &records.dial);
        let seeds = collect_response_seeds(
            parsed,
            strings,
            language,
            *is_localized,
            &topic_labels,
            &records.info,
            game,
        );
        response_sets.push(seeds);
        eprintln!(
            "[voice-reference-native] collected relevant records from {} in {:.2}s",
            parsed.plugin_name,
            started.elapsed().as_secs_f32()
        );
    }

    let mut lines = Vec::new();
    for seeds in response_sets {
        for seed in seeds {
            let plugin_key = seed.plugin.to_ascii_lowercase();
            let form_key = seed.voice_form_id & 0x00FF_FFFF;
            let entries = archive_by_form
                .get(&(plugin_key.clone(), form_key, Some(seed.response_number)))
                .cloned()
                .or_else(|| archive_by_form.get(&(plugin_key, form_key, None)).cloned())
                .unwrap_or_default();
            if entries.is_empty() {
                let fallback = response_filename(seed.info_form_id, seed.response_number);
                lines.push(line_from_seed(game, &seed, &fallback, None));
                continue;
            }
            for entry in entries {
                let filename = entry.filename.clone();
                lines.push(line_from_seed(game, &seed, &filename, Some(&entry)));
            }
        }
    }
    lines.sort_by_key(|line| {
        (
            group_label(line).to_ascii_lowercase(),
            line.plugin.to_ascii_lowercase(),
            line.response_filename.to_ascii_lowercase(),
        )
    });
    Ok(lines)
}

struct RelevantRecords<'a> {
    dial: Vec<&'a ParsedRecord>,
    info: Vec<&'a ParsedRecord>,
}

fn collect_relevant_records(plugin: &ParsedPlugin) -> RelevantRecords<'_> {
    let mut out = RelevantRecords {
        dial: Vec::new(),
        info: Vec::new(),
    };
    collect_relevant_items(&plugin.root_items, &mut out);
    out
}

fn collect_relevant_items<'a>(items: &'a [ParsedItem], out: &mut RelevantRecords<'a>) {
    for item in items {
        match item {
            ParsedItem::Record(record) => match record.signature.as_str() {
                "DIAL" => out.dial.push(record),
                "INFO" => out.info.push(record),
                _ => {}
            },
            ParsedItem::Group(group) => collect_relevant_items(&group.children, out),
        }
    }
}

fn collect_topic_labels(
    plugin: &ParsedPlugin,
    strings: &LocalizedStringsState,
    language: &str,
    localized: bool,
    records: &[&ParsedRecord],
) -> HashMap<(String, u32), String> {
    let mut labels = HashMap::new();
    for record in records {
        if let Some(label) =
            name_text(record, strings, language, localized).or_else(|| editor_id(record))
        {
            labels.insert(record_ref_key(plugin, record), label);
        }
    }
    labels
}

fn collect_response_seeds(
    plugin: &ParsedPlugin,
    strings: &LocalizedStringsState,
    language: &str,
    localized: bool,
    topic_labels: &HashMap<(String, u32), String>,
    records: &[&ParsedRecord],
    game: &str,
) -> Vec<ResponseSeed> {
    let plugin_name = plugin.plugin_name.clone();
    let starfield_wem = game == "starfield";
    let mut seeds = Vec::new();
    for info in records {
        let topic_ref = first_subrecord(info, "TPIC")
            .filter(|sub| sub.data.len() >= 4)
            .and_then(|sub| normalize_form_ref(plugin, read_u32(&sub.data, 0)));
        let topic_text = topic_ref
            .as_ref()
            .and_then(|key| topic_labels.get(key))
            .cloned()
            .unwrap_or_default();
        let mut response_number: Option<u32> = None;
        let mut wem_form_id: Option<u32> = None;
        let mut ordinal = 0u32;
        for subrecord in &info.subrecords {
            match subrecord.signature.as_str() {
                "TRDA" | "TRDT" => {
                    if starfield_wem
                        && subrecord.signature.as_str() == "TRDA"
                        && subrecord.data.len() >= 8
                    {
                        wem_form_id = Some(read_u32(&subrecord.data, 4));
                    } else {
                        response_number =
                            decode_response_number(subrecord.signature.as_str(), &subrecord.data);
                    }
                }
                "NAM1" => {
                    ordinal = ordinal.saturating_add(1);
                    let number = response_number.take().unwrap_or(ordinal).max(1);
                    let voice_form_id = wem_form_id.take().unwrap_or(info.form_id);
                    seeds.push(ResponseSeed {
                        plugin: plugin_name.clone(),
                        info_form_id: info.form_id,
                        response_number: number,
                        response_text: decode_text(&subrecord.data, strings, language, localized),
                        topic_form_id: topic_ref.as_ref().map(|item| item.1).unwrap_or(0),
                        topic_text: topic_text.clone(),
                        voice_form_id,
                    });
                }
                _ => {}
            }
        }
    }
    seeds
}

fn line_from_seed(
    game: &str,
    seed: &ResponseSeed,
    filename: &str,
    archive_entry: Option<&ArchiveEntry>,
) -> VoiceLineRow {
    VoiceLineRow {
        game: game.to_string(),
        plugin: seed.plugin.clone(),
        info_form_id: form_hex(seed.info_form_id),
        response_number: seed.response_number,
        response_text: seed.response_text.clone(),
        response_filename: filename.to_string(),
        voice_type: archive_entry
            .map(|entry| entry.voice_type.clone())
            .unwrap_or_default(),
        characters: Vec::new(),
        archive_path: archive_entry
            .map(|entry| entry.archive_path.clone())
            .unwrap_or_default(),
        member_path: archive_entry
            .map(|entry| entry.member_path.clone())
            .unwrap_or_default(),
        topic_form_id: if seed.topic_form_id == 0 {
            String::new()
        } else {
            form_hex(seed.topic_form_id)
        },
        topic_text: seed.topic_text.clone(),
    }
}

fn group_label(line: &VoiceLineRow) -> String {
    if !line.characters.is_empty() {
        return line
            .characters
            .iter()
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
    }
    if !line.voice_type.is_empty() {
        return line.voice_type.clone();
    }
    "Unknown Voice".to_string()
}

fn search_text(line: &VoiceLineRow) -> String {
    [
        line.plugin.clone(),
        line.info_form_id.clone(),
        line.response_filename.clone(),
        line.response_text.clone(),
        line.voice_type.clone(),
        line.topic_form_id.clone(),
        line.topic_text.clone(),
    ]
    .join(" ")
}

fn first_subrecord<'a>(record: &'a ParsedRecord, signature: &str) -> Option<&'a ParsedSubrecord> {
    record
        .subrecords
        .iter()
        .find(|sub| sub.signature == signature)
}

fn editor_id(record: &ParsedRecord) -> Option<String> {
    first_subrecord(record, "EDID")
        .map(|sub| decode_cp1252_trimmed(&sub.data))
        .filter(|value| !value.is_empty())
}

fn name_text(
    record: &ParsedRecord,
    strings: &LocalizedStringsState,
    language: &str,
    localized: bool,
) -> Option<String> {
    for signature in ["FULL", "RNAM"] {
        if let Some(sub) = first_subrecord(record, signature) {
            let text = decode_text(&sub.data, strings, language, localized);
            if !text.is_empty() {
                return Some(text);
            }
        }
    }
    None
}

fn decode_text(
    data: &[u8],
    strings: &LocalizedStringsState,
    language: &str,
    localized: bool,
) -> String {
    if localized && data.len() == 4 {
        let id = read_u32(data, 0);
        if let Some(value) = resolve_string(strings, id, Some(language)) {
            return value;
        }
        return String::new();
    }
    decode_cp1252_trimmed(data)
}

fn resolve_string(
    strings: &LocalizedStringsState,
    string_id: u32,
    language: Option<&str>,
) -> Option<String> {
    if let Some(language) = language {
        let key = strings::language_code(Some(language));
        if let Some(table) = strings.by_language.get(key.as_str()) {
            if let Some(value) = table.get(&string_id) {
                return Some(value.clone());
            }
        }
    }
    if let Some(table) = strings.by_language.get(strings.default_language.as_str()) {
        if let Some(value) = table.get(&string_id) {
            return Some(value.clone());
        }
    }
    if let Some(table) = strings.by_language.get("en") {
        if let Some(value) = table.get(&string_id) {
            return Some(value.clone());
        }
    }
    for table in strings.by_language.values() {
        if let Some(value) = table.get(&string_id) {
            return Some(value.clone());
        }
    }
    None
}

fn normalize_form_ref(plugin: &ParsedPlugin, raw: u32) -> Option<(String, u32)> {
    if raw == 0 {
        return None;
    }
    let index = (raw >> 24) as usize;
    let object_id = raw & 0x00FF_FFFF;
    if index == LOCAL_FORM_INDEX as usize {
        return Some((plugin.plugin_name.to_ascii_lowercase(), object_id));
    }
    let mut mapping = plugin.header.masters.clone();
    mapping.push(plugin.plugin_name.clone());
    let plugin_name = mapping
        .get(index)
        .cloned()
        .unwrap_or_else(|| plugin.plugin_name.clone());
    Some((plugin_name.to_ascii_lowercase(), object_id))
}

fn record_ref_key(plugin: &ParsedPlugin, record: &ParsedRecord) -> (String, u32) {
    (
        plugin.plugin_name.to_ascii_lowercase(),
        record.form_id & 0x00FF_FFFF,
    )
}

fn decode_response_number(signature: &str, data: &[u8]) -> Option<u32> {
    match signature {
        "TRDA" if data.len() >= 5 => Some(data[4] as u32),
        "TRDT" if data.len() >= 13 => Some(data[12] as u32),
        _ => None,
    }
}

fn response_filename(info_form_id: u32, response_number: u32) -> String {
    format!(
        "{}_{}.fuz",
        voice_file_form_hex(info_form_id),
        response_number
    )
}

fn voice_file_form_hex(value: u32) -> String {
    let text = format!("{value:08x}");
    format!("{}0{}", &text[0..1], &text[2..])
}

fn form_hex(value: u32) -> String {
    format!("{value:08X}")
}

fn read_u32(data: &[u8], offset: usize) -> u32 {
    let mut buf = [0u8; 4];
    buf.copy_from_slice(&data[offset..offset + 4]);
    u32::from_le_bytes(buf)
}

fn decode_cp1252_trimmed(data: &[u8]) -> String {
    let payload = data.strip_suffix(&[0]).unwrap_or(data);
    let (decoded, _, _) = WINDOWS_1252.decode(payload);
    decoded.trim_end().to_string()
}

fn sql_err(err: rusqlite::Error) -> PyErr {
    PyRuntimeError::new_err(err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use smol_str::SmolStr;

    fn record_with_subrecords(
        signature: &str,
        form_id: u32,
        subrecords: Vec<ParsedSubrecord>,
    ) -> ParsedRecord {
        ParsedRecord {
            signature: SmolStr::new(signature),
            form_id,
            flags: 0,
            version_control: 0,
            form_version: None,
            version2: None,
            subrecords,
            raw_payload: None,
            parse_error: None,
        }
    }

    fn subrecord(signature: &str, data: Vec<u8>) -> ParsedSubrecord {
        ParsedSubrecord {
            signature: SmolStr::new(signature),
            data: Bytes::from(data),
            semantic_type: None,
        }
    }

    #[test]
    fn parse_voice_member_fo4_style() {
        let entry = parse_voice_member(
            "Fallout4 - Voices_en.ba2",
            "Sound\\Voice\\Fallout4.esm\\MaleEvenToned\\00001234_1.fuz",
        )
        .expect("entry");
        assert_eq!(entry.plugin, "fallout4.esm");
        assert_eq!(entry.voice_type, "maleeventoned");
        assert_eq!(entry.form_id, 0x0000_1234);
        assert_eq!(entry.index, Some(1));
    }

    #[test]
    fn parse_voice_member_skyrim_style() {
        let entry = parse_voice_member(
            "Skyrim - Voices_en0.bsa",
            "sound/voice/dawnguard.esm/crdragonvoice/dlc1vqdrag_dlc1vqdragonint_0001156b_2.fuz",
        )
        .expect("entry");
        assert_eq!(entry.plugin, "dawnguard.esm");
        assert_eq!(entry.voice_type, "crdragonvoice");
        assert_eq!(entry.form_id, 0x0001_156B);
        assert_eq!(entry.index, Some(2));
    }

    #[test]
    fn parse_voice_member_fnv_style_ogg() {
        let entry = parse_voice_member(
            "Fallout - Voices1.bsa",
            "sound/voice/falloutnv.esm/creatureferalghoul/genericferalghoul_attack_0007ae76_1.ogg",
        )
        .expect("entry");
        assert_eq!(entry.plugin, "falloutnv.esm");
        assert_eq!(entry.voice_type, "creatureferalghoul");
        assert_eq!(entry.form_id, 0x0007_AE76);
        assert_eq!(entry.index, Some(1));
    }

    #[test]
    fn parse_voice_member_starfield_style_wem() {
        let entry = parse_voice_member(
            "Starfield - Voices01.ba2",
            "sound/voice/starfield.esm/announcerfcydoniadetonation/00071610.wem",
        )
        .expect("entry");
        assert_eq!(entry.plugin, "starfield.esm");
        assert_eq!(entry.voice_type, "announcerfcydoniadetonation");
        assert_eq!(entry.form_id, 0x0007_1610);
        assert_eq!(entry.index, None);
    }

    #[test]
    fn parse_voice_member_skips_lip_sidecar() {
        assert!(
            parse_voice_member(
                "Fallout - Voices1.bsa",
                "sound/voice/falloutnv.esm/x/foo_bar_0007ae76_1.lip"
            )
            .is_none()
        );
    }

    #[test]
    fn unresolved_localized_name_does_not_decode_string_id_as_text() {
        let strings = LocalizedStringsState::default();
        let record = record_with_subrecords(
            "NPC_",
            0x01001234,
            vec![subrecord("FULL", 1234u32.to_le_bytes().to_vec())],
        );

        assert_eq!(
            decode_text(&1234u32.to_le_bytes(), &strings, "English", true),
            ""
        );
        assert_eq!(name_text(&record, &strings, "English", true), None);
    }

    #[test]
    fn write_index_uses_contentless_fts_and_drops_per_line_characters() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("voice_reference.db");
        let mut conn = Connection::open(&db_path).expect("open sqlite");
        ensure_schema(&conn).expect("base schema");
        let line = VoiceLineRow {
            game: "skyrimse".to_string(),
            plugin: "Skyrim.esm".to_string(),
            info_form_id: "00001234".to_string(),
            response_number: 1,
            response_text: "Raven Rock needs help.".to_string(),
            response_filename: "00001234_1.fuz".to_string(),
            voice_type: "malenord".to_string(),
            characters: vec!["Balgruuf the Greater".to_string()],
            archive_path: "Skyrim - Voices_en0.bsa".to_string(),
            member_path: "sound/voice/skyrim.esm/malenord/00001234_1.fuz".to_string(),
            topic_form_id: String::new(),
            topic_text: String::new(),
        };

        write_index(
            &mut conn,
            "skyrimse",
            "Data",
            "Strings",
            "English",
            "cache-key",
            &["Skyrim.esm".to_string()],
            &["Skyrim - Voices_en0.bsa".to_string()],
            &[line],
        )
        .expect("write index");

        let schema_version: String = conn
            .query_row(
                "SELECT value FROM voice_meta WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .expect("schema version");
        assert_eq!(schema_version, SCHEMA_VERSION);
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='voice_line_characters'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("character table check"),
            0
        );

        let voice_line_columns: Vec<String> = conn
            .prepare("PRAGMA table_info('voice_lines')")
            .expect("table info")
            .query_map([], |row| row.get(1))
            .expect("columns")
            .collect::<Result<_, _>>()
            .expect("columns");
        assert!(!voice_line_columns.iter().any(|name| name == "search_text"));
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM voice_lines_fts WHERE voice_lines_fts MATCH 'Raven'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("fts raven"),
            1
        );
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM voice_lines_fts WHERE voice_lines_fts MATCH 'Balgruuf'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("fts character"),
            0
        );
        drop(conn);

        let rows = read_lines_impl(&db_path).expect("read slim index");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].archive_path, "Skyrim - Voices_en0.bsa");
        assert!(rows[0].characters.is_empty());
    }
}
