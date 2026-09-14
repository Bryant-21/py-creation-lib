use super::*;

#[path = "authoring_record.rs"]
mod authoring_record;
pub(crate) use authoring_record::*;

// ---------------------------------------------------------------------------
// Canonical top-level group order per game.
//
// The engine resolves cross-references in one forward pass, so top-level GRUPs
// must follow the vanilla record-type order. KYWD must precede every type that
// references keywords (ACTI, CONT, COBJ, ARMO, ...); with alphabetical groups
// the engine hits a COBJ before any KYWD and reports
// `[FORMS] Unable to find keyword (XXXXXXXX)`. CK rewrites canonical order on save.
//
// `generated/group_order.rs` holds per-game lists produced by
// `tools/schema_forge/extract_group_order.py` from each vanilla master, so
// builds work without the game installed. Live extraction from the user's
// master (`cached_master_group_order`) wins when reachable, which picks up
// Bethesda content updates. Signatures in neither list are emitted
// alphabetically after the canonical tail.
// ---------------------------------------------------------------------------

#[path = "../generated/group_order.rs"]
mod bundled_group_order;

fn top_level_group_order_for_game(game: Option<&str>) -> Option<&'static [&'static str]> {
    match game? {
        "fo4" => Some(bundled_group_order::FO4_GROUP_ORDER),
        "skyrimse" => Some(bundled_group_order::SKYRIMSE_GROUP_ORDER),
        "starfield" => Some(bundled_group_order::STARFIELD_GROUP_ORDER),
        "fo76" => Some(bundled_group_order::FO76_GROUP_ORDER),
        "fo3" => Some(bundled_group_order::FO3_GROUP_ORDER),
        "fnv" => Some(bundled_group_order::FNV_GROUP_ORDER),
        _ => None,
    }
}

/// Walks GRUP headers in a master ESM and returns the deduped first-occurrence
/// list of top-level (group_type=0) signatures.
///
/// A GRUP's size covers its children, so the walk jumps over group contents and
/// reads only top-level headers (low milliseconds on the ~330 MB Fallout4.esm).
/// Returns None on any I/O or parse error; the caller falls back to the bundled list.
fn extract_top_level_group_order_from_esm(esm_path: &Path) -> Option<Vec<String>> {
    let buf = fs::read(esm_path).ok()?;
    if buf.len() < 24 || &buf[..4] != b"TES4" {
        return None;
    }
    let tes4_data_size = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]) as usize;
    // Try modern (24-byte) header first; fall back to legacy (20) for FO3/FNV.
    let mut offset = 24usize.checked_add(tes4_data_size)?;
    if offset + 4 > buf.len() || &buf[offset..offset + 4] != b"GRUP" {
        offset = 20usize.checked_add(tes4_data_size)?;
        if offset + 4 > buf.len() || &buf[offset..offset + 4] != b"GRUP" {
            return None;
        }
    }
    let mut order: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    while offset + 24 <= buf.len() {
        if &buf[offset..offset + 4] == b"GRUP" {
            let size = u32::from_le_bytes([
                buf[offset + 4],
                buf[offset + 5],
                buf[offset + 6],
                buf[offset + 7],
            ]) as usize;
            let label = &buf[offset + 8..offset + 12];
            let group_type = i32::from_le_bytes([
                buf[offset + 12],
                buf[offset + 13],
                buf[offset + 14],
                buf[offset + 15],
            ]);
            if group_type == 0 {
                if let Ok(s) = std::str::from_utf8(label) {
                    if seen.insert(s.to_string()) {
                        order.push(s.to_string());
                    }
                }
            }
            // GRUP size includes the header itself; advance past the whole group.
            if size < 24 {
                return None;
            }
            offset = offset.checked_add(size)?;
        } else {
            // Past the group region — done.
            break;
        }
    }
    if order.is_empty() { None } else { Some(order) }
}

/// Cached lookup keyed by (path, mtime). One scan per master ESM per session.
/// Returns owned Vec because the Rust strings need to outlive the function call;
/// the streaming builder only uses borrowed `&str` views during its own scope.
fn cached_master_group_order(esm_path: &Path) -> Option<Vec<String>> {
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::sync::OnceLock;
    static CACHE: OnceLock<Mutex<HashMap<(PathBuf, u64), Option<Vec<String>>>>> = OnceLock::new();
    let mtime_secs = fs::metadata(esm_path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let key = (esm_path.to_path_buf(), mtime_secs);
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    {
        let guard = cache.lock().ok()?;
        if let Some(v) = guard.get(&key) {
            return v.clone();
        }
    }
    let computed = extract_top_level_group_order_from_esm(esm_path);
    if let Ok(mut guard) = cache.lock() {
        guard.insert(key, computed.clone());
    }
    computed
}

// ---------------------------------------------------------------------------
// GIL-free record serializer stub (parallel export path)
// ---------------------------------------------------------------------------

fn serialize_record_for_export(
    record: &ParsedRecord,
    plugin: &ParsedPlugin,
    strings: &LocalizedStringsState,
) -> serde_json::Value {
    serialize_record_payload_to_json(record, plugin, strings)
}

fn export_trace_setting() -> Option<&'static str> {
    static SETTING: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    SETTING
        .get_or_init(|| {
            let value = std::env::var("MODBOX21_NATIVE_EXPORT_TRACE").ok()?;
            let trimmed = value.trim();
            if trimmed.is_empty()
                || trimmed.eq_ignore_ascii_case("0")
                || trimmed.eq_ignore_ascii_case("false")
                || trimmed.eq_ignore_ascii_case("off")
            {
                return None;
            }
            Some(trimmed.to_ascii_lowercase())
        })
        .as_deref()
}

fn export_trace_enabled() -> bool {
    export_trace_setting().is_some()
}

fn export_trace_verbose() -> bool {
    matches!(
        export_trace_setting(),
        Some("2") | Some("full") | Some("record") | Some("records") | Some("verbose")
    )
}

fn export_trace_progress_interval() -> usize {
    static INTERVAL: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *INTERVAL.get_or_init(|| {
        std::env::var("MODBOX21_NATIVE_EXPORT_PROGRESS_INTERVAL")
            .ok()
            .and_then(|value| value.trim().parse::<usize>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(1000)
    })
}

fn should_trace_export_progress(
    trace_enabled: bool,
    verbose: bool,
    index: usize,
    total: usize,
    progress_interval: usize,
) -> bool {
    trace_enabled && (verbose || index == 1 || index == total || index % progress_interval == 0)
}

fn parsed_record_payload_len(record: &ParsedRecord) -> usize {
    record
        .raw_payload
        .as_ref()
        .map(Bytes::len)
        .unwrap_or_else(|| record.subrecords.iter().map(|sub| sub.data.len()).sum())
}

fn trace_record_export_start(
    kind: &str,
    index: usize,
    total: usize,
    output_path: &Path,
    record: &ParsedRecord,
    verbose: bool,
    progress_interval: usize,
) {
    if should_trace_export_progress(
        export_trace_enabled(),
        verbose,
        index,
        total,
        progress_interval,
    ) {
        eprintln!(
            "[creation_lib::_native::esp_export] {kind} {index}/{total}: sig={} form_id={:08X} subrecords={} payload_bytes={} raw_payload={} path={}",
            record.signature,
            record.form_id,
            record.subrecords.len(),
            parsed_record_payload_len(record),
            record.raw_payload.as_ref().map(Bytes::len).unwrap_or(0),
            output_path.display()
        );
    }
}

fn trace_record_export_done(
    kind: &str,
    index: usize,
    total: usize,
    output_path: &Path,
    record: &ParsedRecord,
    text_len: usize,
    verbose: bool,
) {
    if export_trace_enabled() && verbose {
        eprintln!(
            "[creation_lib::_native::esp_export] {kind} done {index}/{total}: sig={} form_id={:08X} yaml_bytes={} path={}",
            record.signature,
            record.form_id,
            text_len,
            output_path.display()
        );
    }
}

fn trace_export_phase(message: std::fmt::Arguments<'_>) {
    if export_trace_enabled() {
        eprintln!("[creation_lib::_native::esp_export] {message}");
    }
}

fn quote_yaml_plain_scalars_with_trailing_spaces(text: String) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let (body, newline) = line
            .strip_suffix('\n')
            .map(|body| (body, "\n"))
            .unwrap_or((line, ""));
        if body.ends_with(' ') {
            if let Some((prefix, value)) = body.split_once(": ") {
                let trimmed_start = value.trim_start();
                if !value.is_empty()
                    && !trimmed_start.starts_with('"')
                    && !trimmed_start.starts_with('\'')
                    && !trimmed_start.starts_with('[')
                    && !trimmed_start.starts_with('{')
                {
                    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
                    out.push_str(prefix);
                    out.push_str(": \"");
                    out.push_str(&escaped);
                    out.push('"');
                    out.push_str(newline);
                    continue;
                }
            }
        }
        out.push_str(body);
        out.push_str(newline);
    }
    out
}

fn dump_json_value_text_native(
    value: &serde_json::Value,
    fmt: &str,
    pretty_json: bool,
) -> Result<String, String> {
    if fmt == "yaml" {
        // serde-saphyr 0.0.25 emits `|N` block scalars with N as the absolute
        // body indent instead of relative-to-parent (per YAML 1.2 §8.1.1.1),
        // so multi-line strings whose first line has leading whitespace round-
        // trip as un-parseable YAML. Disabling block scalars routes them to
        // double-quoted form, which is byte-exact and parses back cleanly.
        let opts = serde_saphyr::ser_options! { prefer_block_scalars: false };
        let mut text = serde_saphyr::to_string_with_options(value, opts)
            .map_err(|err| format!("failed to serialize yaml payload: {err}"))?;
        if let Some(stripped) = text.strip_prefix("---\n") {
            text = stripped.to_string();
        }
        text = quote_yaml_plain_scalars_with_trailing_spaces(text);
        return Ok(text);
    }
    if pretty_json {
        serde_json::to_string_pretty(value)
            .map_err(|err| format!("failed to serialize json payload: {err}"))
    } else {
        serde_json::to_string(value)
            .map_err(|err| format!("failed to serialize json payload: {err}"))
    }
}

fn compact_header_manifest_payload_json(
    plugin: &ParsedPlugin,
    record_count: usize,
) -> serde_json::Value {
    let header = &plugin.header;
    let mut payload = serde_json::Map::new();
    payload.insert(
        "version".to_string(),
        serde_json::Value::from(header.version),
    );
    payload.insert(
        "num_records".to_string(),
        serde_json::Value::from(record_count),
    );
    payload.insert(
        "next_object_id".to_string(),
        serde_json::Value::String(format!("{:06X}", header.next_object_id)),
    );
    payload.insert(
        "author".to_string(),
        serde_json::Value::String(header.author.clone()),
    );
    payload.insert(
        "description".to_string(),
        serde_json::Value::String(header.description.clone()),
    );
    payload.insert(
        "masters".to_string(),
        serde_json::Value::Array(
            header
                .masters
                .iter()
                .cloned()
                .map(serde_json::Value::String)
                .collect(),
        ),
    );
    payload.insert(
        "master_sizes".to_string(),
        serde_json::Value::Array(
            header
                .master_sizes
                .iter()
                .copied()
                .map(serde_json::Value::from)
                .collect(),
        ),
    );
    payload.insert(
        "overridden_forms".to_string(),
        serde_json::Value::Array(
            header
                .overridden_forms
                .iter()
                .map(|raw| serde_json::Value::String(format!("{raw:08X}")))
                .collect(),
        ),
    );
    payload.insert(
        "version_control".to_string(),
        serde_json::Value::from(header.version_control),
    );
    let flags = header.flags;
    let enabled_flags: Vec<serde_json::Value> = HEADER_FLAG_DEFINITIONS
        .iter()
        .filter_map(|(_, value, label)| {
            if flags & value == 0 {
                return None;
            }
            Some(serde_json::Value::String(authoring_camel_case(label)))
        })
        .collect();
    if !enabled_flags.is_empty() {
        payload.insert("flags".to_string(), serde_json::Value::Array(enabled_flags));
    }
    payload.insert(
        "extra_subrecords".to_string(),
        serde_json::Value::Array(
            header
                .extra_subrecords
                .iter()
                .map(|sub| {
                    let mut item = serde_json::Map::new();
                    item.insert(
                        "signature".to_string(),
                        serde_json::Value::String(sub.signature.to_string()),
                    );
                    item.insert("size".to_string(), serde_json::Value::from(sub.data.len()));
                    item.insert(
                        "data_hex".to_string(),
                        serde_json::Value::String(hex::encode_upper(&sub.data)),
                    );
                    serde_json::Value::Object(item)
                })
                .collect(),
        ),
    );
    if let Some(form_version) = header.form_version {
        if form_version != 0 {
            payload.insert(
                "form_version".to_string(),
                serde_json::Value::from(form_version),
            );
        }
    }
    if let Some(version2) = header.version2 {
        if version2 != 0 {
            payload.insert("version2".to_string(), serde_json::Value::from(version2));
        }
    }
    serde_json::Value::Object(payload)
}

fn authoring_manifest_payload_json(
    plugin: &ParsedPlugin,
    record_count: usize,
) -> serde_json::Value {
    let mut manifest = serde_json::Map::new();
    manifest.insert("format_version".to_string(), serde_json::Value::from(1));
    manifest.insert(
        "plugin".to_string(),
        serde_json::Value::String(plugin.plugin_name.clone()),
    );
    manifest.insert(
        "game".to_string(),
        plugin
            .game
            .as_ref()
            .map(|game| serde_json::Value::String(game.clone()))
            .unwrap_or(serde_json::Value::Null),
    );
    manifest.insert(
        "header_size".to_string(),
        serde_json::Value::from(plugin.header_size),
    );
    manifest.insert(
        "header".to_string(),
        compact_header_manifest_payload_json(plugin, record_count),
    );
    serde_json::Value::Object(manifest)
}

fn write_authoring_manifest_native(
    plugin: &ParsedPlugin,
    record_count: usize,
    out_dir: &Path,
    fmt: &str,
) -> PyResult<()> {
    let manifest_path = out_dir.join(if fmt == "yaml" {
        "plugin.yaml"
    } else {
        "plugin.json"
    });
    let manifest = authoring_manifest_payload_json(plugin, record_count);
    let text = dump_json_value_text_native(&manifest, fmt, false).map_err(value_error)?;
    write_text_file(manifest_path.as_path(), text.as_str())
}

// ---------------------------------------------------------------------------
// Parallel export helpers
// ---------------------------------------------------------------------------

/// Flatten all records and their output paths from a parsed plugin into a
/// flat Vec. Groups that contain records recurse and flatten.
/// Projected WRLD/CELL groups are skipped (they fall back to the sequential path).
/// Records whose signature is in `skip_set` are omitted at every nesting level.
fn collect_export_tasks<'a>(
    plugin: &'a ParsedPlugin,
    records_dir: &Path,
    extension: &str,
    plugin_index: &PluginIndex<'_>,
    skip_set: &std::collections::HashSet<String>,
) -> PyResult<Vec<(PathBuf, &'a ParsedRecord)>> {
    use std::collections::BTreeMap;

    let mut tasks: Vec<(PathBuf, &'a ParsedRecord)> = Vec::new();

    fn walk_group<'b>(
        group: &'b ParsedGroup,
        group_dir: &Path,
        plugin_name: &str,
        extension: &str,
        plugin_index: &PluginIndex<'_>,
        skip_set: &std::collections::HashSet<String>,
        tasks: &mut Vec<(PathBuf, &'b ParsedRecord)>,
    ) {
        let mut used_names: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut signature_dirs: BTreeMap<String, PathBuf> = BTreeMap::new();
        let mut child_group_dirs: BTreeMap<String, PathBuf> = BTreeMap::new();
        let direct_signature = group_label_text_native(group);
        for child in &group.children {
            match child {
                ParsedItem::Record(child_record) => {
                    let signature = child_record.signature.as_str();
                    if skip_set.contains(signature) {
                        continue;
                    }
                    let record_dir = if direct_signature.as_deref() == Some(signature) {
                        group_dir.to_path_buf()
                    } else if let Some(existing) = signature_dirs.get(signature) {
                        existing.clone()
                    } else {
                        let dir_name = unique_child_name(signature, &mut used_names);
                        let created = group_dir.join(dir_name);
                        signature_dirs.insert(signature.to_string(), created.clone());
                        created
                    };
                    let filename = record_filename_native(child_record, plugin_name, extension);
                    let output_path = record_dir.join(filename);
                    tasks.push((output_path, child_record));
                }
                ParsedItem::Group(child_group) => {
                    let group_basename =
                        group_dir_basename_native(child_group, plugin_index, plugin_name, false);
                    let child_group_dir = if let Some(existing) =
                        child_group_dirs.get(&group_basename)
                    {
                        existing.clone()
                    } else {
                        let dir_name = unique_child_name(group_basename.as_str(), &mut used_names);
                        let created = group_dir.join(dir_name);
                        child_group_dirs.insert(group_basename, created.clone());
                        created
                    };
                    walk_group(
                        child_group,
                        child_group_dir.as_path(),
                        plugin_name,
                        extension,
                        plugin_index,
                        skip_set,
                        tasks,
                    );
                }
            }
        }
    }

    let mut root_used_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut root_signature_dirs: BTreeMap<String, PathBuf> = BTreeMap::new();
    let mut root_group_dirs: BTreeMap<String, PathBuf> = BTreeMap::new();

    for item in &plugin.root_items {
        match item {
            ParsedItem::Record(record) => {
                let signature = record.signature.as_str();
                if skip_set.contains(signature) {
                    continue;
                }
                let record_dir = if let Some(existing) = root_signature_dirs.get(signature) {
                    existing.clone()
                } else {
                    let dir_name = unique_child_name(signature, &mut root_used_names);
                    let created = records_dir.join(dir_name);
                    root_signature_dirs.insert(signature.to_string(), created.clone());
                    created
                };
                let filename = record_filename_native(record, &plugin.plugin_name, extension);
                tasks.push((record_dir.join(filename), record));
            }
            ParsedItem::Group(group) => {
                // Skip projected WRLD and CELL groups — they fall back to
                // the sequential path in export_authoring_dir_parallel.
                if group.group_type == 0 {
                    match group_label_text_native(group).as_deref() {
                        Some("WRLD") if can_project_wrld_group_native(group) => continue,
                        Some("CELL") if can_project_cell_group_native(group) => continue,
                        _ => {}
                    }
                }
                let basename =
                    group_dir_basename_native(group, plugin_index, &plugin.plugin_name, true);
                let group_dir = if let Some(existing) = root_group_dirs.get(&basename) {
                    existing.clone()
                } else {
                    let dir_name = unique_child_name(basename.as_str(), &mut root_used_names);
                    let created = records_dir.join(dir_name);
                    root_group_dirs.insert(basename, created.clone());
                    created
                };
                walk_group(
                    group,
                    group_dir.as_path(),
                    &plugin.plugin_name,
                    extension,
                    plugin_index,
                    skip_set,
                    &mut tasks,
                );
            }
        }
    }

    Ok(tasks)
}

/// Collect all unique parent directories from a task list.
fn collect_task_dirs(tasks: &[(PathBuf, &ParsedRecord)]) -> Vec<PathBuf> {
    use std::collections::HashSet;
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut dirs: Vec<PathBuf> = Vec::new();
    for (path, _) in tasks {
        if let Some(parent) = path.parent() {
            if seen.insert(parent.to_path_buf()) {
                dirs.push(parent.to_path_buf());
            }
        }
    }
    dirs
}

enum ProjectedJsonTask<'a> {
    WorldRecordData {
        path: PathBuf,
        record: &'a ParsedRecord,
        top_cell: Option<&'a ParsedRecord>,
        top_cell_child_group: Option<&'a ParsedGroup>,
    },
    Cell {
        path: PathBuf,
        record: &'a ParsedRecord,
        child_group: Option<&'a ParsedGroup>,
    },
    GroupRecordData {
        path: PathBuf,
        group_type: Option<i32>,
    },
}

impl ProjectedJsonTask<'_> {
    fn path(&self) -> &Path {
        match self {
            Self::WorldRecordData { path, .. }
            | Self::Cell { path, .. }
            | Self::GroupRecordData { path, .. } => path.as_path(),
        }
    }
}

fn collect_projected_task_dirs(tasks: &[ProjectedJsonTask<'_>]) -> Vec<PathBuf> {
    use std::collections::HashSet;
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut dirs: Vec<PathBuf> = Vec::new();
    for task in tasks {
        if let Some(parent) = task.path().parent() {
            if seen.insert(parent.to_path_buf()) {
                dirs.push(parent.to_path_buf());
            }
        }
    }
    dirs
}

fn compact_record_payload_base_json(
    record: &ParsedRecord,
    plugin: &ParsedPlugin,
    strings: &LocalizedStringsState,
) -> serde_json::Value {
    record_as_authoring_value(record, plugin, strings)
}

fn write_authoring_strings_native(
    plugin: &ParsedPlugin,
    strings: &LocalizedStringsState,
    out_dir: &Path,
) -> PyResult<()> {
    let plugin_path = out_dir.join(plugin.plugin_name.as_str());
    write_localized_strings_for_parsed(plugin, strings, plugin_path.to_string_lossy().as_ref())
        .map(|_| ())
}

fn load_authoring_strings_into_context(context: &mut NativeImportContext, authoring_dir: &Path) {
    let strings_dir = authoring_dir.join("Strings");
    if !strings_dir.is_dir() {
        return;
    }
    let plugin_path = authoring_dir.join(context.plugin_name.as_str());
    let strings = strings::hydrate_strings_state(
        plugin_path.to_string_lossy().as_ref(),
        context.plugin_name.as_str(),
        Some(strings_dir.to_string_lossy().as_ref()),
        None,
    );
    if !strings.by_language.is_empty() {
        context.strings = strings;
        context.refresh_next_localized_string_id(1);
    }
}

fn cell_payload_json(
    record: &ParsedRecord,
    child_group: Option<&ParsedGroup>,
    plugin: &ParsedPlugin,
    strings: &LocalizedStringsState,
) -> serde_json::Value {
    let mut payload = match compact_record_payload_base_json(record, plugin, strings) {
        serde_json::Value::Object(map) => map,
        value => {
            let mut map = serde_json::Map::new();
            map.insert("value".to_string(), value);
            map
        }
    };
    let Some(child_group) = child_group else {
        return serde_json::Value::Object(payload);
    };
    for child in &child_group.children {
        match child {
            ParsedItem::Record(child_record) => match child_record.signature.as_str() {
                "LAND" => {
                    payload.insert(
                        "Landscape".to_string(),
                        compact_record_payload_base_json(child_record, plugin, strings),
                    );
                }
                "NAVM" => {
                    let entry = payload
                        .entry("NavigationMeshes".to_string())
                        .or_insert_with(|| serde_json::Value::Array(Vec::new()));
                    if let serde_json::Value::Array(items) = entry {
                        items.push(compact_record_payload_base_json(
                            child_record,
                            plugin,
                            strings,
                        ));
                    }
                }
                _ => {}
            },
            ParsedItem::Group(child_group_inner) => {
                let section_name = match child_group_inner.group_type {
                    PERSISTENT_GROUP => Some("Persistent"),
                    TEMPORARY_GROUP => Some("Temporary"),
                    VISIBLE_DISTANT_GROUP => Some("VisibleWhenDistant"),
                    _ => None,
                };
                let Some(section_name) = section_name else {
                    continue;
                };
                let mut items = Vec::new();
                for inner in &child_group_inner.children {
                    if let ParsedItem::Record(r) = inner {
                        let mut entry = match compact_record_payload_base_json(r, plugin, strings) {
                            serde_json::Value::Object(map) => map,
                            value => {
                                let mut map = serde_json::Map::new();
                                map.insert("value".to_string(), value);
                                map
                            }
                        };
                        entry.insert(
                            "signature".to_string(),
                            serde_json::Value::String(r.signature.to_string()),
                        );
                        items.push(serde_json::Value::Object(entry));
                    }
                }
                payload.insert(section_name.to_string(), serde_json::Value::Array(items));
            }
        }
    }
    serde_json::Value::Object(payload)
}

fn world_payload_json(
    record: &ParsedRecord,
    top_cell: Option<&ParsedRecord>,
    top_cell_child_group: Option<&ParsedGroup>,
    plugin: &ParsedPlugin,
    strings: &LocalizedStringsState,
) -> serde_json::Value {
    let mut payload = match compact_record_payload_base_json(record, plugin, strings) {
        serde_json::Value::Object(map) => map,
        value => {
            let mut map = serde_json::Map::new();
            map.insert("value".to_string(), value);
            map
        }
    };
    if let Some(top_cell) = top_cell {
        payload.insert(
            "TopCell".to_string(),
            cell_payload_json(top_cell, top_cell_child_group, plugin, strings),
        );
    }
    serde_json::Value::Object(payload)
}

fn group_record_data_json(group_type: Option<i32>) -> serde_json::Value {
    let mut payload = serde_json::Map::new();
    if let Some(group_type) = group_type {
        if let Some(name) = group_type_name(group_type) {
            payload.insert(
                "group_type".to_string(),
                serde_json::Value::String(name.to_string()),
            );
        }
    }
    serde_json::Value::Object(payload)
}

fn collect_projected_wrld_tasks<'a>(
    group: &'a ParsedGroup,
    records_dir: &Path,
    plugin: &ParsedPlugin,
    fmt: &str,
    tasks: &mut Vec<ProjectedJsonTask<'a>>,
) -> PyResult<()> {
    let wrld_dir = records_dir.join("WRLD");
    let mut world_children_groups: HashMap<u32, &ParsedGroup> = HashMap::new();
    for child in &group.children {
        if let ParsedItem::Group(child_group) = child {
            if child_group.group_type != 1 {
                continue;
            }
            if let Some(form_id) = decode_group_index(&child_group.label) {
                world_children_groups.insert(form_id, child_group);
            }
        }
    }
    for child in &group.children {
        let ParsedItem::Record(child_record) = child else {
            continue;
        };
        if child_record.signature != "WRLD" {
            continue;
        }
        let world_dir = wrld_dir.join(special_record_dir_name_native(
            child_record,
            &plugin.plugin_name,
        ));
        let Some(world_group) = world_children_groups.get(&child_record.form_id) else {
            tasks.push(ProjectedJsonTask::WorldRecordData {
                path: world_dir.join(record_data_filename(fmt)),
                record: child_record,
                top_cell: None,
                top_cell_child_group: None,
            });
            continue;
        };
        let mut cell_records: HashMap<u32, &ParsedRecord> = HashMap::new();
        let mut cell_children: HashMap<u32, &ParsedGroup> = HashMap::new();
        let mut direct_cell_records: Vec<&ParsedRecord> = Vec::new();
        let mut block_groups: Vec<&ParsedGroup> = Vec::new();
        for world_child in &world_group.children {
            match world_child {
                ParsedItem::Record(r) => {
                    if r.signature == "CELL" {
                        cell_records.insert(r.form_id, r);
                        direct_cell_records.push(r);
                    }
                }
                ParsedItem::Group(g) => {
                    if g.group_type == CELL_CHILD_GROUP {
                        if let Some(form_id) = decode_group_index(&g.label) {
                            cell_children.insert(form_id, g);
                        }
                        continue;
                    }
                    if g.group_type != EXTERIOR_CELL_BLOCK {
                        continue;
                    }
                    block_groups.push(g);
                    for subblock_item in &g.children {
                        if let ParsedItem::Group(subblock) = subblock_item {
                            for cell_item in &subblock.children {
                                match cell_item {
                                    ParsedItem::Record(r) => {
                                        if r.signature == "CELL" {
                                            cell_records.insert(r.form_id, r);
                                        }
                                    }
                                    ParsedItem::Group(cell_group) => {
                                        if cell_group.group_type == CELL_CHILD_GROUP {
                                            if let Some(form_id) =
                                                decode_group_index(&cell_group.label)
                                            {
                                                cell_children.insert(form_id, cell_group);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        let top_cell = direct_cell_records.first().copied();
        let top_cell_child_group =
            top_cell.and_then(|record| cell_children.get(&record.form_id).copied());
        tasks.push(ProjectedJsonTask::WorldRecordData {
            path: world_dir.join(record_data_filename(fmt)),
            record: child_record,
            top_cell,
            top_cell_child_group,
        });
        for record in direct_cell_records.into_iter().skip(1) {
            let child_group = cell_children.get(&record.form_id).copied();
            let cell_dir =
                world_dir.join(special_record_dir_name_native(record, &plugin.plugin_name));
            tasks.push(ProjectedJsonTask::Cell {
                path: cell_dir.join(record_data_filename(fmt)),
                record,
                child_group,
            });
        }
        for block in block_groups {
            let block_dir = world_dir.join(grid_dir_name_from_group_native(block)?);
            tasks.push(ProjectedJsonTask::GroupRecordData {
                path: block_dir.join(group_record_data_filename(fmt)),
                group_type: Some(EXTERIOR_CELL_BLOCK),
            });
            for subblock_item in &block.children {
                let ParsedItem::Group(subblock) = subblock_item else {
                    continue;
                };
                let subblock_dir = block_dir.join(grid_dir_name_from_group_native(subblock)?);
                tasks.push(ProjectedJsonTask::GroupRecordData {
                    path: subblock_dir.join(group_record_data_filename(fmt)),
                    group_type: Some(EXTERIOR_CELL_SUBBLOCK),
                });
                let mut emitted: HashSet<u32> = HashSet::new();
                for cell_item in &subblock.children {
                    if let ParsedItem::Record(record) = cell_item {
                        if record.signature != "CELL" {
                            continue;
                        }
                        let form_id = record.form_id;
                        emitted.insert(form_id);
                        let child_group = cell_children.get(&form_id).copied();
                        let cell_dir = subblock_dir
                            .join(special_record_dir_name_native(record, &plugin.plugin_name));
                        tasks.push(ProjectedJsonTask::Cell {
                            path: cell_dir.join(record_data_filename(fmt)),
                            record,
                            child_group,
                        });
                    }
                }
                for cell_item in &subblock.children {
                    let ParsedItem::Group(cell_group) = cell_item else {
                        continue;
                    };
                    if cell_group.group_type != CELL_CHILD_GROUP {
                        continue;
                    }
                    let Some(form_id) = decode_group_index(&cell_group.label) else {
                        continue;
                    };
                    if !emitted.insert(form_id) {
                        continue;
                    }
                    let Some(record) = cell_records.get(&form_id).copied() else {
                        continue;
                    };
                    let cell_dir = subblock_dir
                        .join(special_record_dir_name_native(record, &plugin.plugin_name));
                    tasks.push(ProjectedJsonTask::Cell {
                        path: cell_dir.join(record_data_filename(fmt)),
                        record,
                        child_group: Some(cell_group),
                    });
                }
            }
        }
    }
    Ok(())
}

fn collect_projected_cell_tasks<'a>(
    group: &'a ParsedGroup,
    records_dir: &Path,
    plugin: &ParsedPlugin,
    fmt: &str,
    tasks: &mut Vec<ProjectedJsonTask<'a>>,
) -> PyResult<()> {
    let cells_dir = records_dir.join("CELL");
    tasks.push(ProjectedJsonTask::GroupRecordData {
        path: cells_dir.join(group_record_data_filename(fmt)),
        group_type: None,
    });
    for block_item in &group.children {
        let ParsedItem::Group(block) = block_item else {
            continue;
        };
        let block_dir = cells_dir.join(index_dir_name_from_group_native(block)?);
        tasks.push(ProjectedJsonTask::GroupRecordData {
            path: block_dir.join(group_record_data_filename(fmt)),
            group_type: Some(INTERIOR_CELL_BLOCK),
        });
        for subblock_item in &block.children {
            let ParsedItem::Group(subblock) = subblock_item else {
                continue;
            };
            let subblock_dir = block_dir.join(index_dir_name_from_group_native(subblock)?);
            tasks.push(ProjectedJsonTask::GroupRecordData {
                path: subblock_dir.join(group_record_data_filename(fmt)),
                group_type: Some(INTERIOR_CELL_SUBBLOCK),
            });
            let mut cell_records: Vec<(u32, &ParsedRecord)> = Vec::new();
            let mut cell_children: HashMap<u32, &ParsedGroup> = HashMap::new();
            for item in &subblock.children {
                match item {
                    ParsedItem::Record(r) => {
                        cell_records.push((r.form_id, r));
                    }
                    ParsedItem::Group(g) => {
                        if let Some(form_id) = decode_group_index(&g.label) {
                            cell_children.insert(form_id, g);
                        }
                    }
                }
            }
            for (form_id, record) in cell_records {
                let child_group = cell_children.get(&form_id).copied();
                let cell_dir =
                    subblock_dir.join(special_record_dir_name_native(record, &plugin.plugin_name));
                tasks.push(ProjectedJsonTask::Cell {
                    path: cell_dir.join(record_data_filename(fmt)),
                    record,
                    child_group,
                });
            }
        }
    }
    Ok(())
}

fn export_projected_tasks_native(
    py: Python<'_>,
    plugin: &ParsedPlugin,
    strings: &LocalizedStringsState,
    tasks: &[ProjectedJsonTask<'_>],
    fmt: &str,
    jobs: usize,
    pretty_json: bool,
) -> PyResult<()> {
    use pyo3::exceptions::PyValueError;
    use std::sync::Mutex;

    trace_export_phase(format_args!(
        "projected export starting: tasks={} jobs={} format={fmt}",
        tasks.len(),
        jobs
    ));

    for dir in collect_projected_task_dirs(tasks) {
        fs::create_dir_all(&dir).map_err(|err| {
            io_error(format!(
                "failed to create projected export directory '{}': {err}",
                dir.display()
            ))
        })?;
    }

    let errors: Vec<String> = py.detach(|| {
        use rayon::prelude::*;
        use std::sync::atomic::{AtomicUsize, Ordering};

        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(jobs)
            .build()
            .unwrap();
        let errors = Mutex::new(Vec::<String>::new());
        let counter = AtomicUsize::new(0);
        let total = tasks.len();
        let progress_interval = export_trace_progress_interval();
        let verbose = export_trace_verbose();
        pool.install(|| {
            tasks.par_iter().for_each(|task| {
                let index = counter.fetch_add(1, Ordering::Relaxed) + 1;
                let (output_path, value, record_for_trace) = match task {
                    ProjectedJsonTask::WorldRecordData {
                        path,
                        record,
                        top_cell,
                        top_cell_child_group,
                    } => {
                        trace_record_export_start(
                            "projected-world",
                            index,
                            total,
                            path.as_path(),
                            record,
                            verbose,
                            progress_interval,
                        );
                        (
                            path.as_path(),
                            world_payload_json(
                                record,
                                *top_cell,
                                *top_cell_child_group,
                                plugin,
                                strings,
                            ),
                            Some(*record),
                        )
                    }
                    ProjectedJsonTask::Cell {
                        path,
                        record,
                        child_group,
                    }
                    => {
                        trace_record_export_start(
                            "projected-cell",
                            index,
                            total,
                            path.as_path(),
                            record,
                            verbose,
                            progress_interval,
                        );
                        (
                            path.as_path(),
                            cell_payload_json(record, *child_group, plugin, strings),
                            Some(*record),
                        )
                    }
                    ProjectedJsonTask::GroupRecordData { path, group_type } => {
                        if should_trace_export_progress(
                            export_trace_enabled(),
                            verbose,
                            index,
                            total,
                            progress_interval,
                        ) {
                            eprintln!(
                                "[creation_lib::_native::esp_export] projected-group {index}/{total}: group_type={group_type:?} path={}",
                                path.display()
                            );
                        }
                        (path.as_path(), group_record_data_json(*group_type), None)
                    }
                };
                let text = match dump_json_value_text_native(&value, fmt, pretty_json) {
                    Ok(s) => s,
                    Err(e) => {
                        errors.lock().unwrap().push(e);
                        return;
                    }
                };
                if let Err(e) = fs::write(output_path, text.as_bytes()) {
                    errors
                        .lock()
                        .unwrap()
                        .push(format!("{}: {e}", output_path.display()));
                }
                if let Some(record) = record_for_trace {
                    trace_record_export_done(
                        "projected",
                        index,
                        total,
                        output_path,
                        record,
                        text.len(),
                        verbose,
                    );
                }
            });
        });
        errors.into_inner().unwrap()
    });

    if !errors.is_empty() {
        return Err(PyValueError::new_err(format!(
            "parallel projected export errors: {}",
            errors.join("; ")
        )));
    }
    Ok(())
}

fn collect_projected_tasks<'a>(
    plugin: &'a ParsedPlugin,
    records_dir: &Path,
    fmt: &str,
    skip_set: &std::collections::HashSet<String>,
) -> PyResult<Vec<ProjectedJsonTask<'a>>> {
    let mut projected_tasks = Vec::new();
    for item in &plugin.root_items {
        if let ParsedItem::Group(group) = item {
            if group.group_type == 0 {
                match group_label_text_native(group).as_deref() {
                    Some("WRLD") if can_project_wrld_group_native(group) => {
                        // Skip the entire WRLD projection if either WRLD or CELL is in
                        // the skip set. The bounded plugin-port use case always passes
                        // both together, so the looser "either" semantic is fine and
                        // keeps the implementation simple.
                        if skip_set.contains("WRLD") || skip_set.contains("CELL") {
                            continue;
                        }
                        collect_projected_wrld_tasks(
                            group,
                            records_dir,
                            plugin,
                            fmt,
                            &mut projected_tasks,
                        )?;
                    }
                    Some("CELL") if can_project_cell_group_native(group) => {
                        // Skip top-level (interior) CELL projection when CELL is suppressed.
                        if skip_set.contains("CELL") {
                            continue;
                        }
                        collect_projected_cell_tasks(
                            group,
                            records_dir,
                            plugin,
                            fmt,
                            &mut projected_tasks,
                        )?;
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(projected_tasks)
}

fn export_record_tasks_native(
    py: Python<'_>,
    plugin: &ParsedPlugin,
    strings: &LocalizedStringsState,
    tasks: &[(PathBuf, &ParsedRecord)],
    fmt: &str,
    jobs: usize,
    pretty_json: bool,
) -> PyResult<()> {
    use pyo3::exceptions::PyValueError;
    use std::sync::Mutex;

    trace_export_phase(format_args!(
        "record export starting: tasks={} jobs={} format={fmt}",
        tasks.len(),
        jobs
    ));

    let errors: Vec<String> = py.detach(|| {
        use rayon::prelude::*;
        use std::sync::atomic::{AtomicUsize, Ordering};

        let errors = Mutex::new(Vec::<String>::new());
        let counter = AtomicUsize::new(0);
        let total = tasks.len();
        let progress_interval = export_trace_progress_interval();
        let verbose = export_trace_verbose();
        if jobs > 1 {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(jobs)
                .build()
                .unwrap();
            pool.install(|| {
                tasks.par_iter().for_each(|(output_path, record)| {
                    let index = counter.fetch_add(1, Ordering::Relaxed) + 1;
                    trace_record_export_start(
                        "record",
                        index,
                        total,
                        output_path.as_path(),
                        record,
                        verbose,
                        progress_interval,
                    );
                    let json_value = serialize_record_for_export(record, plugin, strings);
                    let text = match dump_json_value_text_native(&json_value, fmt, pretty_json) {
                        Ok(s) => s,
                        Err(e) => {
                            errors.lock().unwrap().push(e);
                            return;
                        }
                    };
                    if let Err(e) = fs::write(output_path, text.as_bytes()) {
                        errors
                            .lock()
                            .unwrap()
                            .push(format!("{}: {e}", output_path.display()));
                    }
                    trace_record_export_done(
                        "record",
                        index,
                        total,
                        output_path.as_path(),
                        record,
                        text.len(),
                        verbose,
                    );
                });
            });
        } else {
            for (output_path, record) in tasks {
                let index = counter.fetch_add(1, Ordering::Relaxed) + 1;
                trace_record_export_start(
                    "record",
                    index,
                    total,
                    output_path.as_path(),
                    record,
                    verbose,
                    progress_interval,
                );
                let json_value = serialize_record_for_export(record, plugin, strings);
                let text = match dump_json_value_text_native(&json_value, fmt, pretty_json) {
                    Ok(s) => s,
                    Err(e) => {
                        errors.lock().unwrap().push(e);
                        continue;
                    }
                };
                if let Err(e) = fs::write(output_path, text.as_bytes()) {
                    errors
                        .lock()
                        .unwrap()
                        .push(format!("{}: {e}", output_path.display()));
                }
                trace_record_export_done(
                    "record",
                    index,
                    total,
                    output_path.as_path(),
                    record,
                    text.len(),
                    verbose,
                );
            }
        }
        errors.into_inner().unwrap()
    });

    if !errors.is_empty() {
        return Err(PyValueError::new_err(format!(
            "authoring record export errors: {}",
            errors.join("; ")
        )));
    }
    Ok(())
}

pub(crate) fn export_authoring_dir_parallel(
    py: Python<'_>,
    plugin: &ParsedPlugin,
    strings: &LocalizedStringsState,
    record_count: usize,
    out_dir: &Path,
    fmt: &str,
    jobs: usize,
    skip_set: &std::collections::HashSet<String>,
) -> PyResult<()> {
    let extension = if fmt == "yaml" { ".yaml" } else { ".json" };
    trace_export_phase(format_args!(
        "authoring dir export starting: plugin={} game={:?} records={} jobs={} format={} out_dir={}",
        plugin.plugin_name,
        plugin.game.as_deref(),
        record_count,
        jobs,
        fmt,
        out_dir.display()
    ));

    // ------------------------------------------------------------------
    // 1. Write manifest (same as sequential path)
    // ------------------------------------------------------------------
    if out_dir.exists() {
        fs::remove_dir_all(out_dir).map_err(|err| {
            io_error(format!(
                "failed to remove authoring directory '{}': {err}",
                out_dir.display()
            ))
        })?;
    }
    fs::create_dir_all(out_dir).map_err(|err| {
        io_error(format!(
            "failed to create authoring directory '{}': {err}",
            out_dir.display()
        ))
    })?;
    write_authoring_manifest_native(plugin, record_count, out_dir, fmt)?;
    write_authoring_strings_native(plugin, strings, out_dir)?;

    // ------------------------------------------------------------------
    // 2. Create records/ directory
    // ------------------------------------------------------------------
    let records_dir = out_dir.join("records");
    fs::create_dir_all(&records_dir).map_err(|err| {
        io_error(format!(
            "failed to create records directory '{}': {err}",
            records_dir.display()
        ))
    })?;

    let plugin_index = PluginIndex::build(plugin);

    // ------------------------------------------------------------------
    // 3. Handle projected groups (WRLD / CELL) with native JSON/YAML tasks.
    // ------------------------------------------------------------------
    let projected_tasks = collect_projected_tasks(plugin, records_dir.as_path(), fmt, skip_set)?;
    trace_export_phase(format_args!(
        "projected tasks collected: {}",
        projected_tasks.len()
    ));
    export_projected_tasks_native(py, plugin, strings, &projected_tasks, fmt, jobs, true)?;

    // ------------------------------------------------------------------
    // 4. Collect flat task list (non-projected groups + root records)
    // ------------------------------------------------------------------
    let tasks = collect_export_tasks(
        plugin,
        records_dir.as_path(),
        extension,
        &plugin_index,
        skip_set,
    )?;
    trace_export_phase(format_args!("record tasks collected: {}", tasks.len()));

    // ------------------------------------------------------------------
    // 5. Pre-create all needed directories (sequential — avoids races)
    // ------------------------------------------------------------------
    for dir in collect_task_dirs(&tasks) {
        fs::create_dir_all(&dir).map_err(|err| {
            io_error(format!(
                "failed to create record directory '{}': {err}",
                dir.display()
            ))
        })?;
    }

    // ------------------------------------------------------------------
    // 6. Release GIL and serialize/write files in parallel.
    // ------------------------------------------------------------------
    export_record_tasks_native(py, plugin, strings, &tasks, fmt, jobs, true)
}

pub(crate) fn export_authoring_dir_from_parsed<'py>(
    py: Python<'py>,
    plugin: &ParsedPlugin,
    strings: &LocalizedStringsState,
    record_count: usize,
    out_dir: &Path,
    fmt: &str,
    skip_set: &std::collections::HashSet<String>,
) -> PyResult<()> {
    let extension = if fmt == "yaml" { ".yaml" } else { ".json" };
    trace_export_phase(format_args!(
        "authoring dir export starting: plugin={} game={:?} records={} jobs=1 format={} out_dir={}",
        plugin.plugin_name,
        plugin.game.as_deref(),
        record_count,
        fmt,
        out_dir.display()
    ));
    if out_dir.exists() {
        fs::remove_dir_all(out_dir).map_err(|err| {
            io_error(format!(
                "failed to remove authoring directory '{}': {err}",
                out_dir.display()
            ))
        })?;
    }
    fs::create_dir_all(out_dir).map_err(|err| {
        io_error(format!(
            "failed to create authoring directory '{}': {err}",
            out_dir.display()
        ))
    })?;
    write_authoring_manifest_native(plugin, record_count, out_dir, fmt)?;
    write_authoring_strings_native(plugin, strings, out_dir)?;

    let records_dir = out_dir.join("records");
    fs::create_dir_all(&records_dir).map_err(|err| {
        io_error(format!(
            "failed to create records directory '{}': {err}",
            records_dir.display()
        ))
    })?;

    // Thread &LocalizedStringsState directly through the serialize chain.
    let plugin_index = PluginIndex::build(plugin);

    let projected_tasks = collect_projected_tasks(plugin, records_dir.as_path(), fmt, skip_set)?;
    trace_export_phase(format_args!(
        "projected tasks collected: {}",
        projected_tasks.len()
    ));
    export_projected_tasks_native(py, plugin, strings, &projected_tasks, fmt, 1, false)?;

    let tasks = collect_export_tasks(
        plugin,
        records_dir.as_path(),
        extension,
        &plugin_index,
        skip_set,
    )?;
    trace_export_phase(format_args!("record tasks collected: {}", tasks.len()));
    for dir in collect_task_dirs(&tasks) {
        fs::create_dir_all(&dir).map_err(|err| {
            io_error(format!(
                "failed to create record directory '{}': {err}",
                dir.display()
            ))
        })?;
    }
    export_record_tasks_native(py, plugin, strings, &tasks, fmt, 1, false)
}

/// Count records in a plugin for manifest generation (GIL-free version).
fn count_records_no_py(items: &[ParsedItem]) -> usize {
    let mut n = 0;
    for item in items {
        match item {
            ParsedItem::Record(_) => n += 1,
            ParsedItem::Group(group) => n += count_records_no_py(&group.children),
        }
    }
    n
}

/// GIL-free authoring-dir export, equivalent to `export_authoring_dir_from_parsed`
/// with `jobs=1`. Safe inside `py.detach()`, where acquiring the GIL would deadlock.
pub fn export_authoring_dir_no_py(
    plugin: &ParsedPlugin,
    strings: &LocalizedStringsState,
    out_dir: &Path,
    fmt: &str,
) -> Result<(), String> {
    let extension = if fmt == "yaml" { ".yaml" } else { ".json" };
    let record_count = count_records_no_py(&plugin.root_items);

    if out_dir.exists() {
        fs::remove_dir_all(out_dir).map_err(|e| {
            format!(
                "failed to remove authoring dir '{}': {e}",
                out_dir.display()
            )
        })?;
    }
    fs::create_dir_all(out_dir).map_err(|e| {
        format!(
            "failed to create authoring dir '{}': {e}",
            out_dir.display()
        )
    })?;

    write_authoring_manifest_native(plugin, record_count, out_dir, fmt)
        .map_err(|e| e.to_string())?;
    write_authoring_strings_native(plugin, strings, out_dir).map_err(|e| e.to_string())?;

    let records_dir = out_dir.join("records");
    fs::create_dir_all(&records_dir).map_err(|e| {
        format!(
            "failed to create records dir '{}': {e}",
            records_dir.display()
        )
    })?;

    let plugin_index = PluginIndex::build(plugin);
    let skip_set = std::collections::HashSet::<String>::new();

    let projected_tasks = collect_projected_tasks(plugin, records_dir.as_path(), fmt, &skip_set)
        .map_err(|e| e.to_string())?;

    // Sequential projected-tasks export (no rayon, GIL-free).
    for dir in collect_projected_task_dirs(&projected_tasks) {
        fs::create_dir_all(&dir).map_err(|e| format!("mkdir '{}': {e}", dir.display()))?;
    }
    for task in &projected_tasks {
        let (output_path, value) = match task {
            ProjectedJsonTask::WorldRecordData {
                path,
                record,
                top_cell,
                top_cell_child_group,
            } => (
                path.as_path(),
                world_payload_json(record, *top_cell, *top_cell_child_group, plugin, strings),
            ),
            ProjectedJsonTask::Cell {
                path,
                record,
                child_group,
            } => (
                path.as_path(),
                cell_payload_json(record, *child_group, plugin, strings),
            ),
            ProjectedJsonTask::GroupRecordData { path, group_type } => {
                (path.as_path(), group_record_data_json(*group_type))
            }
        };
        let text = dump_json_value_text_native(&value, fmt, false)
            .map_err(|e| format!("serialize projected task '{}': {e}", output_path.display()))?;
        fs::write(output_path, text.as_bytes())
            .map_err(|e| format!("write '{}': {e}", output_path.display()))?;
    }

    let tasks = collect_export_tasks(
        plugin,
        records_dir.as_path(),
        extension,
        &plugin_index,
        &skip_set,
    )
    .map_err(|e| e.to_string())?;

    for dir in collect_task_dirs(&tasks) {
        fs::create_dir_all(&dir).map_err(|e| format!("mkdir '{}': {e}", dir.display()))?;
    }
    for (output_path, record) in &tasks {
        let json_value = serialize_record_for_export(record, plugin, strings);
        let text = dump_json_value_text_native(&json_value, fmt, false)
            .map_err(|e| format!("serialize '{}': {e}", output_path.display()))?;
        fs::write(output_path, text.as_bytes())
            .map_err(|e| format!("write '{}': {e}", output_path.display()))?;
    }

    Ok(())
}

/// GIL-free authoring-dir export from a plugin handle ID.
pub fn export_authoring_dir_from_handle_no_py(
    handle_id: u64,
    out_dir: &Path,
    fmt: &str,
) -> Result<(), String> {
    let (parsed, strings) =
        crate::plugin_runtime::clone_plugin_handle_state_for_authoring_no_py(handle_id)?;
    export_authoring_dir_no_py(&parsed, &strings, out_dir, fmt)
}

pub(crate) fn load_mapping_payload_value_native(path: &Path) -> PyResult<JsonValue> {
    let text = fs::read_to_string(path)
        .map_err(|err| io_error(format!("failed to read '{}': {err}", path.display())))?;
    let format = detect_text_format(path.to_string_lossy().as_ref(), None);
    parse_text_payload_value_native(text.as_str(), format.as_str())
}

fn load_record_from_authoring_file_compact_native(
    path: &Path,
    record_signature: &str,
    context: &mut NativeImportContext,
) -> PyResult<ParsedRecord> {
    let payload_value = load_mapping_payload_value_native(path)?;
    let mut payload = json_object(&payload_value, &path.display().to_string())?.clone();
    if !payload.contains_key("signature") {
        payload.insert(
            "signature".to_string(),
            JsonValue::String(record_signature.to_string()),
        );
    }
    parse_record_from_json_compact_native(&payload, context)
}

/// Per-record sub-context aggregate produced by the parallel streaming decoders.
struct CompactRecordTaskResult {
    record: ParsedRecord,
    masters: Vec<String>,
    strings: LocalizedStringsState,
    allocated_string_ids: Vec<u32>,
}

/// Stride of localized string IDs reserved per-record sub-task. Records rarely
/// allocate >16 string IDs (FULL/DESC/etc.), so 64 gives ample headroom while
/// keeping renumber-merge cheap.
const COMPACT_PARALLEL_RECORD_ID_STRIDE: u32 = 64;

fn parse_group_record_data_native(
    directory: &Path,
    expected_group_type: Option<i32>,
) -> PyResult<Option<i32>> {
    let Some(group_path) = find_named_payload_file(directory, GROUP_RECORD_DATA_STEM) else {
        return Ok(expected_group_type);
    };
    let payload = load_mapping_payload_value_native(group_path.as_path())?;
    let payload = json_object(&payload, &group_path.display().to_string())?;
    let parsed_group_type = match payload.get("group_type") {
        Some(value) if value.is_null() => None,
        Some(value) => {
            if let Some(name) = value.as_str() {
                parse_group_type_name(name).or_else(|| name.parse::<i32>().ok())
            } else {
                value.as_i64().map(|value| value as i32)
            }
        }
        None => None,
    };
    if let Some(expected) = expected_group_type {
        if let Some(parsed) = parsed_group_type {
            if parsed != expected {
                return Err(value_error(format!(
                    "{} declares group_type={}, expected {}",
                    group_path.display(),
                    parsed,
                    expected
                )));
            }
        }
    }
    Ok(parsed_group_type.or(expected_group_type))
}

fn renumber_localized_string_ids_in_items(items: &mut [ParsedItem], id_map: &HashMap<u32, u32>) {
    for item in items.iter_mut() {
        match item {
            ParsedItem::Record(record) => {
                for subrecord in &mut record.subrecords {
                    if subrecord.semantic_type.as_deref() != Some("localized_string")
                        || subrecord.data.len() != 4
                    {
                        continue;
                    }
                    let raw = u32::from_le_bytes([
                        subrecord.data[0],
                        subrecord.data[1],
                        subrecord.data[2],
                        subrecord.data[3],
                    ]);
                    if let Some(remapped) = id_map.get(&raw) {
                        // subrecord.data is `Bytes` (refcount slice into the
                        // source mmap) and immutable. Materialize an owned
                        // 4-byte copy with the remapped ID. Cheap because
                        // localized string subrecords are exactly 4 bytes.
                        subrecord.data = Bytes::copy_from_slice(&remapped.to_le_bytes());
                    }
                }
            }
            ParsedItem::Group(group) => {
                renumber_localized_string_ids_in_items(&mut group.children, id_map)
            }
        }
    }
}

fn renumber_localized_string_ids_in_strings(
    strings: &mut LocalizedStringsState,
    id_map: &HashMap<u32, u32>,
) {
    if id_map.is_empty() {
        return;
    }
    strings.materialize_all();
    for table in strings.by_language.values_mut() {
        let mut rewritten = HashMap::with_capacity(table.len());
        for (string_id, text) in table.drain() {
            let target_id = id_map.get(&string_id).copied().unwrap_or(string_id);
            rewritten.insert(target_id, text);
        }
        *table = rewritten;
    }
    let mut rewritten_types = HashMap::with_capacity(strings.table_types.len());
    for (string_id, table_type) in strings.table_types.drain() {
        let target_id = id_map.get(&string_id).copied().unwrap_or(string_id);
        rewritten_types.insert(target_id, table_type);
    }
    strings.table_types = rewritten_types;
}

fn load_mapping_payload_object(
    _context: &NativeImportContext,
    path: &Path,
) -> PyResult<JsonMap<String, JsonValue>> {
    let value = load_mapping_payload_value_native(path)?;
    Ok(json_object(&value, &path.display().to_string())?.clone())
}

/// Tracks bytes written + a list of GRUP header offsets whose size fields will
/// be patched once the body length is known. Patches are applied in a final
/// pass after all writes complete, avoiding seek-back during streaming.
struct StreamingEspBuilder<W: Write> {
    writer: W,
    bytes_written: u64,
    record_count: usize,
    group_patches: Vec<(u64, u32)>,
}

impl<W: Write> StreamingEspBuilder<W> {
    fn new(writer: W) -> Self {
        Self {
            writer,
            bytes_written: 0,
            record_count: 0,
            group_patches: Vec::new(),
        }
    }

    fn write_all(&mut self, data: &[u8]) -> std::io::Result<()> {
        self.writer.write_all(data)?;
        self.bytes_written += data.len() as u64;
        Ok(())
    }

    /// Write a GRUP header with placeholder size, return the start offset so
    /// the caller can patch the size when the body is complete.
    fn begin_group(
        &mut self,
        label: [u8; 4],
        group_type: i32,
        header_size: usize,
    ) -> std::io::Result<u64> {
        let start = self.bytes_written;
        self.write_all(b"GRUP")?;
        self.write_all(&0u32.to_le_bytes())?; // placeholder size
        self.write_all(&label)?;
        self.write_all(&group_type.to_le_bytes())?;
        let tail_len = header_size.saturating_sub(16);
        if tail_len > 0 {
            self.write_all(&vec![0u8; tail_len])?;
        }
        Ok(start)
    }

    fn end_group(&mut self, start_offset: u64) {
        let total = (self.bytes_written - start_offset) as u32;
        self.group_patches.push((start_offset, total));
    }

    fn write_record(&mut self, record: &ParsedRecord, header_size: usize) -> PyResult<()> {
        let bytes = record_bytes_from_parsed(record, header_size)?;
        self.write_all(&bytes)
            .map_err(|e| io_error(format!("write record: {e}")))?;
        self.record_count += 1;
        Ok(())
    }
}

fn hedr_num_records_offset(record_bytes: &[u8], header_size: usize) -> Option<usize> {
    let mut offset = header_size;
    while offset + 6 <= record_bytes.len() {
        let data_len =
            u16::from_le_bytes([record_bytes[offset + 4], record_bytes[offset + 5]]) as usize;
        let data_start = offset + 6;
        let data_end = data_start.checked_add(data_len)?;
        if data_end > record_bytes.len() {
            return None;
        }
        if &record_bytes[offset..offset + 4] == b"HEDR" && data_len >= 8 {
            return Some(data_start + 4);
        }
        offset = data_end;
    }
    None
}

fn write_tes4_header_streaming<W: Write>(
    builder: &mut StreamingEspBuilder<W>,
    context: &NativeImportContext,
    header_size: usize,
) -> PyResult<Option<u64>> {
    // header_subrecords_from_parsed needs a ParsedPlugin shell; build a minimal one
    let plugin_shell = ParsedPlugin {
        plugin_name: context.plugin_name.clone(),
        file_path: String::new(),
        header_size,
        header: context.header.clone(),
        root_items: Vec::new(),
        game: context.game.clone(),
    };
    let header_subrecords = header_subrecords_from_parsed(&plugin_shell);
    let mut header_payload: Vec<u8> = Vec::new();
    for sub in &header_subrecords {
        header_payload.extend_from_slice(sub);
    }
    let tes4 = ParsedRecord {
        signature: SmolStr::new_static("TES4"),
        form_id: 0,
        flags: context.header.flags,
        version_control: context.header.version_control,
        form_version: if header_size == MODERN_HEADER_SIZE {
            context.header.form_version
        } else {
            None
        },
        version2: if header_size == MODERN_HEADER_SIZE {
            context.header.version2
        } else {
            None
        },
        subrecords: Vec::new(),
        raw_payload: Some(Bytes::from(header_payload)),
        parse_error: None,
    };
    let bytes = record_bytes_from_parsed(&tes4, header_size)?;
    let hedr_offset = hedr_num_records_offset(&bytes, header_size)
        .map(|offset| builder.bytes_written + offset as u64);
    builder
        .write_all(&bytes)
        .map_err(|e| io_error(format!("write TES4: {e}")))?;
    Ok(hedr_offset)
}

fn walk_root_signature_streaming<W: Write>(
    builder: &mut StreamingEspBuilder<W>,
    entry: &Path,
    entry_name: &str,
    context: &mut NativeImportContext,
    header_size: usize,
) -> PyResult<()> {
    let mut label = [0u8; 4];
    for (i, b) in entry_name.as_bytes().iter().take(4).enumerate() {
        label[i] = *b;
    }
    let group_start = builder
        .begin_group(label, 0, header_size)
        .map_err(|e| io_error(format!("begin root GRUP: {e}")))?;

    if entry_name == "WRLD" && detect_special_layout(entry, "WRLD")? == "projected" {
        walk_projected_wrld_streaming(builder, entry, context, header_size)?;
    } else if entry_name == "CELL" && detect_special_layout(entry, "CELL")? == "projected" {
        walk_projected_cell_root_streaming(builder, entry, context, header_size)?;
    } else {
        walk_directory_items_streaming(
            builder,
            entry,
            context,
            Some(entry_name),
            false,
            header_size,
        )?;
    }

    builder.end_group(group_start);
    Ok(())
}

/// Above this many sibling record files in a single dir, parse them in
/// parallel via the bounded-queue streaming pipeline. Below it, the per-record
/// thread/channel overhead loses to serial parsing.
const STREAM_PARALLEL_RECORD_THRESHOLD: usize = 32;

/// Bounded-queue streaming parallel decode: parses N record files concurrently
/// with N worker threads, each pulling from a bounded input channel. Workers
/// emit `(idx, CompactRecordTaskResult)` to a bounded output channel. The
/// caller reorders by idx, performs master/string-ID reconciliation, encodes
/// to .esp bytes, writes, and drops the record before pulling the next.
///
/// Peak in-flight records ≤ workers + workers (channel buffers) + workers
/// (out-of-order reorder buffer) ≈ 3*workers. This bounds peak RSS by thread
/// count instead of by chunk size, so we get full parallelism without the
/// "collect 1024 results then drain" memory spike of the chunked variant.
fn parallel_streaming_decode_records(
    inputs: &[(PathBuf, String)],
    parent: &mut NativeImportContext,
    workers: usize,
    mut write_record: impl FnMut(&ParsedRecord) -> PyResult<()>,
) -> PyResult<()> {
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::sync::mpsc::sync_channel;

    if inputs.is_empty() {
        return Ok(());
    }

    let workers = workers.max(1).min(inputs.len());

    // Snapshot parent state for sub-contexts (cheap clones; schema stays Arc).
    let parent_next_id = parent.next_localized_string_id();
    let parent_master_count = parent.header.masters.len();
    let plugin_name = parent.plugin_name.clone();
    let game = parent.game.clone();
    let header_size = parent.header_size;
    let base_header = parent.header.clone();
    let base_default_lang = parent.strings.default_language.clone();

    let (tx_in, rx_in) = sync_channel::<(usize, PathBuf, String)>(workers * 2);
    let (tx_out, rx_out) = sync_channel::<(usize, PyResult<CompactRecordTaskResult>)>(workers * 2);
    let rx_in_shared = Arc::new(Mutex::new(rx_in));

    std::thread::scope(|scope| -> PyResult<()> {
        // Producer: feed all inputs, then drop the input sender.
        scope.spawn(move || {
            for (idx, (path, sig)) in inputs.iter().enumerate() {
                if tx_in.send((idx, path.clone(), sig.clone())).is_err() {
                    return;
                }
            }
            // tx_in dropped here, signaling no more inputs.
        });

        // Worker pool: each worker pulls from rx_in and pushes to tx_out.
        for _ in 0..workers {
            let rx_in = Arc::clone(&rx_in_shared);
            let tx_out = tx_out.clone();
            let plugin_name = plugin_name.clone();
            let game = game.clone();
            let base_header = base_header.clone();
            let base_default_lang = base_default_lang.clone();
            scope.spawn(move || {
                loop {
                    let item = match rx_in.lock().unwrap().recv() {
                        Ok(x) => x,
                        Err(_) => break,
                    };
                    let (idx, path, sig) = item;
                    let result = (|| -> PyResult<CompactRecordTaskResult> {
                        let offset = (idx as u32)
                            .checked_mul(COMPACT_PARALLEL_RECORD_ID_STRIDE)
                            .ok_or_else(|| value_error("record stride overflow"))?;
                        let start_id = parent_next_id
                            .checked_add(offset)
                            .ok_or_else(|| value_error("record stride start overflow"))?;
                        let mut sub = NativeImportContext::new(
                            plugin_name.clone(),
                            game.clone(),
                            header_size,
                            base_header.clone(),
                        );
                        sub.strings.default_language = base_default_lang.clone();
                        sub.set_next_localized_string_id(start_id);
                        let record = load_record_from_authoring_file_compact_native(
                            path.as_path(),
                            sig.as_str(),
                            &mut sub,
                        )?;
                        Ok(CompactRecordTaskResult {
                            record,
                            masters: sub.header.masters,
                            strings: sub.strings,
                            allocated_string_ids: sub.allocated_localized_string_ids,
                        })
                    })();
                    if tx_out.send((idx, result)).is_err() {
                        break;
                    }
                }
            });
        }
        // Drop the original tx_out so the consumer's rx_out unblocks once all
        // worker clones drop their senders.
        drop(tx_out);

        // Consumer (main thread): reorder by idx, reconcile, encode, write.
        // On error we MUST keep draining rx_out (and the producer must drain
        // rx_in) so the worker pool unblocks — otherwise std::thread::scope
        // blocks forever waiting for threads stuck on channel sends.
        let mut buffer: HashMap<usize, CompactRecordTaskResult> = HashMap::new();
        let mut next_idx = 0usize;
        let mut next_string_id = parent_next_id;
        let mut deferred_err: Option<PyErr> = None;
        for (idx, result) in rx_out.iter() {
            if deferred_err.is_some() {
                // Drop incoming results until rx_out closes naturally.
                continue;
            }
            let r = match result {
                Ok(r) => r,
                Err(e) => {
                    deferred_err = Some(e);
                    continue;
                }
            };
            buffer.insert(idx, r);
            while let Some(mut r) = buffer.remove(&next_idx) {
                // Phase A: master reconciliation (only when sub-context
                // discovered new masters, which is rare in roundtrip).
                for master in r.masters.iter().skip(parent_master_count) {
                    parent.ensure_master_index(master.as_str());
                }
                let parent_own_index = parent.own_index() as u8;
                let parent_masters_now = parent.header.masters.clone();

                // Phase B: collision-free localized string ID assignment.
                let mut id_map: HashMap<u32, u32> =
                    HashMap::with_capacity(r.allocated_string_ids.len());
                for source_id in &r.allocated_string_ids {
                    if id_map.contains_key(source_id) {
                        continue;
                    }
                    id_map.insert(*source_id, next_string_id);
                    next_string_id = next_string_id.saturating_add(1);
                }
                if !id_map.is_empty() {
                    let mut wrapper = vec![ParsedItem::Record(r.record)];
                    renumber_localized_string_ids_in_items(&mut wrapper, &id_map);
                    renumber_localized_string_ids_in_strings(&mut r.strings, &id_map);
                    let ParsedItem::Record(rec) = wrapper.pop().expect("single record") else {
                        unreachable!("wrapper preserves variant");
                    };
                    r.record = rec;
                }

                // Phase C: FormID remap when sub-context's master list diverged
                // from the parent's post-merge view.
                if r.masters != parent_masters_now {
                    let source_own = r.masters.len() as u8;
                    let mut wrapper = vec![ParsedItem::Record(r.record)];
                    remap_formids_in_items(
                        &mut wrapper,
                        &r.masters,
                        &parent_masters_now,
                        source_own,
                        parent_own_index,
                    );
                    let ParsedItem::Record(rec) = wrapper.pop().expect("single record") else {
                        unreachable!("wrapper preserves variant");
                    };
                    r.record = rec;
                }

                if let Err(e) = parent.merge_localized_strings_from(&r.strings) {
                    deferred_err = Some(e);
                    break;
                }
                if let Err(e) = write_record(&r.record) {
                    deferred_err = Some(e);
                    break;
                }
                next_idx += 1;
            }
        }
        if let Some(err) = deferred_err {
            return Err(err);
        }
        parent.set_next_localized_string_id(next_string_id);
        Ok(())
    })
}

fn walk_directory_items_streaming<W: Write>(
    builder: &mut StreamingEspBuilder<W>,
    path: &Path,
    context: &mut NativeImportContext,
    default_signature: Option<&str>,
    is_root: bool,
    header_size: usize,
) -> PyResult<()> {
    // First pass: partition entries into record files vs sub-dirs. The sorted
    // order from sorted_directory_entries places files before dirs, so the
    // record-file batch fed into the parallel decoder is already in
    // canonical order.
    let entries = sorted_directory_entries(path)?;
    let mut record_batch: Vec<(PathBuf, String)> = Vec::new();
    let mut subdirs: Vec<PathBuf> = Vec::new();
    for entry in entries {
        if entry.is_file() {
            if !is_record_file(&entry) {
                continue;
            }
            let signature = default_signature.ok_or_else(|| {
                value_error(format!(
                    "cannot infer record signature for '{}'",
                    entry.display()
                ))
            })?;
            record_batch.push((entry, signature.to_string()));
        } else {
            subdirs.push(entry);
        }
    }

    // At the plugin root, top-level GRUPs must appear in the engine's canonical
    // record-type order so cross-references (FNAM, KSIZ/KWDA, CNAM…) resolve
    // during the forward load pass. Without this, CK reports
    // `[FORMS] Unable to find keyword (XXXXXXXX)` when a record references a
    // KYWD that appears later in the file.
    if is_root {
        if let Some(order) = top_level_group_order_for_game(context.game.as_deref()) {
            let rank: std::collections::HashMap<&str, usize> =
                order.iter().enumerate().map(|(i, sig)| (*sig, i)).collect();
            subdirs.sort_by(|a, b| {
                let key = |p: &Path| -> (usize, String) {
                    let name = p
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or_default()
                        .to_string();
                    let r = rank.get(name.as_str()).copied().unwrap_or(usize::MAX);
                    (r, name.to_ascii_lowercase())
                };
                key(a).cmp(&key(b))
            });
        }
    }

    // Process record files. Use the bounded-queue streaming decoder when the
    // batch is large enough to amortize the per-record thread/channel overhead.
    if default_signature == Some("INFO") {
        let mut records = Vec::with_capacity(record_batch.len());
        for (entry, signature) in &record_batch {
            records.push(load_record_from_authoring_file_compact_native(
                entry.as_path(),
                signature.as_str(),
                context,
            )?);
        }
        sort_info_records_by_previous(&mut records);
        for record in &records {
            builder.write_record(record, header_size)?;
        }
    } else if record_batch.len() >= STREAM_PARALLEL_RECORD_THRESHOLD {
        let workers = rayon::current_num_threads();
        parallel_streaming_decode_records(&record_batch, context, workers, |record| {
            builder.write_record(record, header_size)
        })?;
    } else {
        for (entry, signature) in &record_batch {
            let record = load_record_from_authoring_file_compact_native(
                entry.as_path(),
                signature.as_str(),
                context,
            )?;
            builder.write_record(&record, header_size)?;
        }
    }

    // Process subdirs in original order
    for entry in subdirs {
        let entry_name = entry
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| value_error(format!("invalid directory name '{}'", entry.display())))?
            .to_string();

        if let Some((group_type, label_bytes)) = parse_group_dir_name(entry_name.as_str(), is_root)
        {
            let mut label = [0u8; 4];
            if label_bytes.len() > 4 {
                return Err(value_error("group label must be at most 4 bytes"));
            }
            for (i, value) in label_bytes.iter().copied().enumerate() {
                label[i] = value;
            }
            let group_signature = if group_type == 0 {
                let text = String::from_utf8_lossy(&label_bytes)
                    .trim_end_matches('\0')
                    .to_string();
                if text.len() == 4 { Some(text) } else { None }
            } else {
                None
            };
            let group_start = builder
                .begin_group(label, group_type, header_size)
                .map_err(|e| io_error(format!("begin GRUP: {e}")))?;
            walk_directory_items_streaming(
                builder,
                entry.as_path(),
                context,
                group_signature.as_deref(),
                false,
                header_size,
            )?;
            builder.end_group(group_start);
            continue;
        }

        if entry_name.len() == 4 {
            // Nested signature dir within an existing group
            walk_directory_items_streaming(
                builder,
                entry.as_path(),
                context,
                Some(entry_name.as_str()),
                false,
                header_size,
            )?;
            continue;
        }

        return Err(value_error(format!(
            "unsupported authoring directory entry '{}'",
            entry.display()
        )));
    }
    Ok(())
}

fn sort_info_records_by_previous(records: &mut Vec<ParsedRecord>) {
    if records.len() <= 1 {
        return;
    }

    let original = records.clone();
    let mut index_by_object_id = std::collections::HashMap::<u32, usize>::new();
    for (index, record) in original.iter().enumerate() {
        index_by_object_id.insert(record.form_id & 0x00FF_FFFF, index);
    }

    let mut edges = vec![Vec::<usize>::new(); original.len()];
    let mut indegree = vec![0usize; original.len()];
    for (index, record) in original.iter().enumerate() {
        let Some(previous_object_id) = previous_info_object_id(record) else {
            continue;
        };
        let Some(previous_index) = index_by_object_id.get(&previous_object_id).copied() else {
            continue;
        };
        if previous_index == index {
            continue;
        }
        edges[previous_index].push(index);
        indegree[index] += 1;
    }

    let mut ready = std::collections::VecDeque::<usize>::new();
    for (index, degree) in indegree.iter().enumerate() {
        if *degree == 0 {
            ready.push_back(index);
        }
    }

    let mut emitted = vec![false; original.len()];
    let mut sorted = Vec::with_capacity(original.len());
    while let Some(index) = ready.pop_front() {
        if emitted[index] {
            continue;
        }
        emitted[index] = true;
        sorted.push(original[index].clone());
        for dependent in &edges[index] {
            indegree[*dependent] = indegree[*dependent].saturating_sub(1);
            if indegree[*dependent] == 0 {
                ready.push_back(*dependent);
            }
        }
    }

    for (index, record) in original.into_iter().enumerate() {
        if !emitted[index] {
            sorted.push(record);
        }
    }
    *records = sorted;
}

fn previous_info_object_id(record: &ParsedRecord) -> Option<u32> {
    let previous = record
        .subrecords
        .iter()
        .find(|subrecord| subrecord.signature.as_str() == "PNAM")?;
    let bytes = previous.data.as_ref();
    if bytes.len() != 4 {
        return None;
    }
    let form_id = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let object_id = form_id & 0x00FF_FFFF;
    if object_id == 0 {
        None
    } else {
        Some(object_id)
    }
}

/// A fully-decoded projected cell, ready for serial emit. On the parallel path
/// this is produced on a worker thread from a per-cell sub-context;
/// the `masters` / `strings` / `allocated_string_ids` deltas are then reconciled
/// into the parent by the serial writer. On the serial path the records are
/// decoded directly into the parent context and those delta fields stay empty.
///
/// `has_persistent` / `has_visible_when_distant` track key *presence* (not vec
/// emptiness): a present-but-empty section must still emit an empty GRUP.
struct DecodedProjectedCell {
    cell_record: ParsedRecord,
    landscape: Option<ParsedRecord>,
    navmeshes: Vec<ParsedRecord>,
    persistent: Vec<ParsedRecord>,
    temporary: Vec<ParsedRecord>,
    visible_when_distant: Vec<ParsedRecord>,
    has_persistent: bool,
    needs_temporary: bool,
    has_visible_when_distant: bool,
    masters: Vec<String>,
    strings: LocalizedStringsState,
    allocated_string_ids: Vec<u32>,
}

fn decode_refr_section(
    payload: &JsonMap<String, JsonValue>,
    name: &str,
    context: &mut NativeImportContext,
) -> PyResult<Vec<ParsedRecord>> {
    let mut out = Vec::new();
    if let Some(value) = payload.get(name) {
        for entry in json_array(value, name)? {
            let mut mapping = json_object(entry, name)?.clone();
            if !mapping.contains_key("signature") {
                mapping.insert(
                    "signature".to_string(),
                    JsonValue::String("REFR".to_string()),
                );
            }
            out.push(parse_record_from_json_compact_native(&mapping, context)?);
        }
    }
    Ok(out)
}

/// Decode a projected CELL payload into records using `context`. The parse order
/// (CELL → Persistent → LAND → NAVM → Temporary → VisibleWhenDistant) fixes the
/// localized-string-ID allocation sequence, and therefore the output bytes.
/// Decoding into a fresh sub-context yields a `DecodedProjectedCell`
/// whose deltas the writer reconciles; decoding into the parent leaves the delta
/// fields empty and the writer skips reconciliation.
fn decode_projected_cell_payload(
    payload: &JsonMap<String, JsonValue>,
    context: &mut NativeImportContext,
) -> PyResult<DecodedProjectedCell> {
    // Build a minimal CELL-only view by selecting just the record's own fields.
    // Cloning `payload` whole would copy nested children (Persistent/Temporary
    // arrays of REFR records) — for a Starfield exterior cell that's hundreds
    // of MB per call. Selecting individual top-level keys keeps the clone
    // bounded by the CELL's own subrecord array.
    let mut cell_only = JsonMap::with_capacity(8);
    for key in [
        "signature",
        "form_id",
        "flags",
        "version_control",
        "form_version",
        "version2",
        "fields",
        "fields_by_signature",
        "subrecords",
        "raw_payload_hex",
        "parse_error",
    ] {
        if let Some(value) = payload.get(key) {
            cell_only.insert(key.to_string(), value.clone());
        }
    }
    if !cell_only.contains_key("signature") {
        cell_only.insert(
            "signature".to_string(),
            JsonValue::String("CELL".to_string()),
        );
    }
    let cell_record = parse_record_from_json_compact_native(&cell_only, context)?;
    drop(cell_only);

    let has_landscape = payload.get("Landscape").is_some();
    let has_navmeshes = payload.get("NavigationMeshes").is_some();
    let has_persistent = payload.get("Persistent").is_some();
    let has_visible_when_distant = payload.get("VisibleWhenDistant").is_some();
    // LAND and NAVM are temporary-children content: Fallout 4 only streams
    // exterior terrain/navmesh from the Cell Temporary Children group (type 9),
    // so the Temporary group must exist whenever either is present — even when
    // the cell has no temporary REFRs (e.g. a land-only terrain pass).
    let needs_temporary = has_landscape || has_navmeshes || payload.get("Temporary").is_some();

    // Persistent REFRs are parsed before LAND/NAVM, matching the former emit.
    let persistent = decode_refr_section(payload, "Persistent", context)?;

    let landscape = if let Some(value) = payload.get("Landscape") {
        let mut mapping = json_object(value, "Landscape")?.clone();
        mapping.insert(
            "signature".to_string(),
            JsonValue::String("LAND".to_string()),
        );
        Some(parse_record_from_json_compact_native(&mapping, context)?)
    } else {
        None
    };

    let mut navmeshes = Vec::new();
    if let Some(value) = payload.get("NavigationMeshes") {
        for entry in json_array(value, "NavigationMeshes")? {
            let mut mapping = json_object(entry, "NavigationMeshes[]")?.clone();
            mapping.insert(
                "signature".to_string(),
                JsonValue::String("NAVM".to_string()),
            );
            navmeshes.push(parse_record_from_json_compact_native(&mapping, context)?);
        }
    }

    let temporary = decode_refr_section(payload, "Temporary", context)?;
    let visible_when_distant = decode_refr_section(payload, "VisibleWhenDistant", context)?;

    Ok(DecodedProjectedCell {
        cell_record,
        landscape,
        navmeshes,
        persistent,
        temporary,
        visible_when_distant,
        has_persistent,
        needs_temporary,
        has_visible_when_distant,
        masters: Vec::new(),
        strings: LocalizedStringsState::default(),
        allocated_string_ids: Vec::new(),
    })
}

/// Reconcile a decoded cell's sub-context deltas into `parent` (when
/// `reconcile`) and stream-emit its GRUP tree: the cell record itself + a
/// CELL_CHILD_GROUP containing Landscape, NavigationMeshes, and Persistent /
/// Temporary / VisibleWhenDistant section groups.
///
/// `reconcile = false` is the serial-shared path: the records were parsed
/// directly into `parent`, so their FormIDs/string-IDs are already in the
/// parent namespace and `next_string_id` is untouched. `reconcile = true`
/// applies the same string-ID renumber + FormID remap + string-merge the
/// per-record consumer does (`parallel_streaming_decode_records`), across all
/// of the cell's records at once.
fn write_decoded_projected_cell<W: Write>(
    builder: &mut StreamingEspBuilder<W>,
    cell: DecodedProjectedCell,
    parent: &mut NativeImportContext,
    next_string_id: &mut u32,
    header_size: usize,
    reconcile: bool,
) -> PyResult<()> {
    let DecodedProjectedCell {
        mut cell_record,
        mut landscape,
        mut navmeshes,
        mut persistent,
        mut temporary,
        mut visible_when_distant,
        has_persistent,
        needs_temporary,
        has_visible_when_distant,
        masters,
        mut strings,
        allocated_string_ids,
    } = cell;

    if reconcile {
        for master in &masters {
            parent.ensure_master_index(master.as_str());
        }
        let parent_own_index = parent.own_index() as u8;
        let parent_masters_now = parent.header.masters.clone();

        let mut id_map: HashMap<u32, u32> = HashMap::with_capacity(allocated_string_ids.len());
        for source_id in &allocated_string_ids {
            if id_map.contains_key(source_id) {
                continue;
            }
            id_map.insert(*source_id, *next_string_id);
            *next_string_id = next_string_id.saturating_add(1);
        }

        // Flatten in the same order records were decoded, reconcile the whole
        // batch (renumber/remap are position-independent), then split back out.
        let land_present = landscape.is_some();
        let n_persistent = persistent.len();
        let n_navm = navmeshes.len();
        let n_temporary = temporary.len();
        let n_vwd = visible_when_distant.len();

        let mut items: Vec<ParsedItem> = Vec::with_capacity(
            1 + n_persistent + land_present as usize + n_navm + n_temporary + n_vwd,
        );
        items.push(ParsedItem::Record(cell_record));
        items.extend(persistent.into_iter().map(ParsedItem::Record));
        if let Some(land) = landscape {
            items.push(ParsedItem::Record(land));
        }
        items.extend(navmeshes.into_iter().map(ParsedItem::Record));
        items.extend(temporary.into_iter().map(ParsedItem::Record));
        items.extend(visible_when_distant.into_iter().map(ParsedItem::Record));

        if !id_map.is_empty() {
            renumber_localized_string_ids_in_items(&mut items, &id_map);
            renumber_localized_string_ids_in_strings(&mut strings, &id_map);
        }
        if masters != parent_masters_now {
            let source_own = masters.len() as u8;
            remap_formids_in_items(
                &mut items,
                &masters,
                &parent_masters_now,
                source_own,
                parent_own_index,
            );
        }
        parent.merge_localized_strings_from(&strings)?;

        let mut it = items.into_iter().map(|item| match item {
            ParsedItem::Record(record) => record,
            _ => unreachable!("flattened cell items are all records"),
        });
        cell_record = it.next().expect("cell record");
        persistent = (0..n_persistent)
            .map(|_| it.next().expect("persistent record"))
            .collect();
        landscape = if land_present {
            Some(it.next().expect("landscape record"))
        } else {
            None
        };
        navmeshes = (0..n_navm)
            .map(|_| it.next().expect("navmesh record"))
            .collect();
        temporary = (0..n_temporary)
            .map(|_| it.next().expect("temporary record"))
            .collect();
        visible_when_distant = (0..n_vwd)
            .map(|_| it.next().expect("visible-when-distant record"))
            .collect();
    } else {
        let _ = (&masters, &strings, &allocated_string_ids, &next_string_id);
    }

    let form_id = cell_record.form_id;
    builder.write_record(&cell_record, header_size)?;
    drop(cell_record);

    let sections_present: Vec<(&str, i32)> = [
        ("Persistent", PERSISTENT_GROUP, has_persistent),
        ("Temporary", TEMPORARY_GROUP, needs_temporary),
        (
            "VisibleWhenDistant",
            VISIBLE_DISTANT_GROUP,
            has_visible_when_distant,
        ),
    ]
    .into_iter()
    .filter_map(|(name, gt, present)| present.then_some((name, gt)))
    .collect();

    if sections_present.is_empty() {
        return Ok(());
    }

    let child_label = form_id.to_le_bytes();
    let child_start = builder
        .begin_group(child_label, CELL_CHILD_GROUP, header_size)
        .map_err(|e| io_error(format!("begin CELL_CHILD_GROUP: {e}")))?;

    for (section_name, group_type) in sections_present {
        let section_start = builder
            .begin_group(form_id.to_le_bytes(), group_type, header_size)
            .map_err(|e| io_error(format!("begin {section_name} group: {e}")))?;

        // Landscape + navmeshes lead the Temporary group, before its REFRs. As
        // direct children of the Cell Children group (type 6) the engine never
        // streams the terrain and shows flat default ground.
        if group_type == TEMPORARY_GROUP {
            if let Some(land) = &landscape {
                builder.write_record(land, header_size)?;
            }
            for navm in &navmeshes {
                builder.write_record(navm, header_size)?;
            }
        }

        let records = match group_type {
            PERSISTENT_GROUP => &persistent,
            TEMPORARY_GROUP => &temporary,
            VISIBLE_DISTANT_GROUP => &visible_when_distant,
            _ => unreachable!("unexpected cell section group type"),
        };
        for record in records {
            builder.write_record(record, header_size)?;
        }

        builder.end_group(section_start);
    }

    builder.end_group(child_start);
    Ok(())
}

/// Stream-write a projected CELL by decoding it into the parent context and
/// emitting it (serial-shared path: no reconciliation needed).
fn write_projected_cell_payload_streaming<W: Write>(
    builder: &mut StreamingEspBuilder<W>,
    payload: &JsonMap<String, JsonValue>,
    context: &mut NativeImportContext,
    header_size: usize,
) -> PyResult<()> {
    let decoded = decode_projected_cell_payload(payload, context)?;
    // Unused on the serial path: decode already advanced context's counter in
    // place, so the writer must not touch it (reconcile = false).
    let mut next_string_id = context.next_localized_string_id();
    write_decoded_projected_cell(
        builder,
        decoded,
        context,
        &mut next_string_id,
        header_size,
        false,
    )
}

/// Load a projected cell directory's RecordData payload and stream-emit it.
/// Load + structurally validate a projected cell directory's RecordData
/// payload. Runs the same dir-name and dir-contents checks the serial writer
/// did. `context` is only forwarded to the (context-agnostic) payload loader, so
/// this is safe to call on a worker thread with a per-cell sub-context.
fn load_projected_cell_payload_from_dir(
    cell_dir: &Path,
    context: &NativeImportContext,
) -> PyResult<JsonMap<String, JsonValue>> {
    if parse_group_dir_name(
        cell_dir
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default(),
        false,
    )
    .is_some()
    {
        return Err(mixed_layout_error(cell_dir.parent().unwrap_or(cell_dir)));
    }
    let record_path = find_named_payload_file(cell_dir, RECORD_DATA_STEM).ok_or_else(|| {
        value_error(format!(
            "Missing {RECORD_DATA_STEM} in {}",
            cell_dir.display()
        ))
    })?;
    let payload = load_mapping_payload_object(context, record_path.as_path())?;
    // Validate that the cell dir contains nothing else unexpected.
    for entry in sorted_directory_entries(cell_dir)? {
        if entry.is_file() {
            if entry == record_path
                || entry
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or_default()
                    .starts_with(GROUP_RECORD_DATA_STEM)
            {
                continue;
            }
            if is_record_file(entry.as_path()) {
                return Err(mixed_layout_error(cell_dir));
            }
            continue;
        }
        return Err(value_error(format!(
            "Unsupported projected cell entry: {}",
            entry.display()
        )));
    }
    Ok(payload)
}

fn write_projected_cell_dir_streaming<W: Write>(
    builder: &mut StreamingEspBuilder<W>,
    cell_dir: &Path,
    context: &mut NativeImportContext,
    header_size: usize,
) -> PyResult<()> {
    let payload = load_projected_cell_payload_from_dir(cell_dir, context)?;
    write_projected_cell_payload_streaming(builder, &payload, context, header_size)
}

/// Stream-write the body of a projected CELL root group (the records/ entry
/// named "CELL" when its layout is projected). The caller has already opened
/// the wrapping GRUP type=0; here we emit the block/subblock/cell tree.
fn walk_projected_cell_root_streaming<W: Write>(
    builder: &mut StreamingEspBuilder<W>,
    path: &Path,
    context: &mut NativeImportContext,
    header_size: usize,
) -> PyResult<()> {
    for block_dir in sorted_directory_entries(path)? {
        if block_dir.is_file() {
            if block_dir
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .starts_with(GROUP_RECORD_DATA_STEM)
            {
                continue;
            }
            if is_record_file(block_dir.as_path()) {
                return Err(mixed_layout_error(path));
            }
            continue;
        }
        let block_name = block_dir
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        let block_label = block_name.parse::<i32>().map_err(|_| {
            value_error(format!(
                "Unsupported projected CELL block dir: {}",
                block_dir.display()
            ))
        })?;
        let block_group_type =
            parse_group_record_data_native(block_dir.as_path(), Some(INTERIOR_CELL_BLOCK))?
                .unwrap_or(INTERIOR_CELL_BLOCK);
        let mut block_label_bytes = [0u8; 4];
        block_label_bytes.copy_from_slice(&block_label.to_le_bytes());
        let block_start = builder
            .begin_group(block_label_bytes, block_group_type, header_size)
            .map_err(|e| io_error(format!("begin interior block: {e}")))?;

        for subblock_dir in sorted_directory_entries(block_dir.as_path())? {
            if subblock_dir.is_file() {
                if subblock_dir
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or_default()
                    .starts_with(GROUP_RECORD_DATA_STEM)
                {
                    continue;
                }
                if is_record_file(subblock_dir.as_path()) {
                    return Err(mixed_layout_error(block_dir.as_path()));
                }
                continue;
            }
            let subblock_name = subblock_dir
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or_default();
            let subblock_label = subblock_name.parse::<i32>().map_err(|_| {
                value_error(format!(
                    "Unsupported projected CELL subblock dir: {}",
                    subblock_dir.display()
                ))
            })?;
            let subblock_group_type = parse_group_record_data_native(
                subblock_dir.as_path(),
                Some(INTERIOR_CELL_SUBBLOCK),
            )?
            .unwrap_or(INTERIOR_CELL_SUBBLOCK);
            let mut sub_label_bytes = [0u8; 4];
            sub_label_bytes.copy_from_slice(&subblock_label.to_le_bytes());
            let sub_start = builder
                .begin_group(sub_label_bytes, subblock_group_type, header_size)
                .map_err(|e| io_error(format!("begin interior subblock: {e}")))?;

            for cell_dir in sorted_directory_entries(subblock_dir.as_path())? {
                if cell_dir.is_file() {
                    if cell_dir
                        .file_name()
                        .and_then(|value| value.to_str())
                        .unwrap_or_default()
                        .starts_with(GROUP_RECORD_DATA_STEM)
                    {
                        continue;
                    }
                    if is_record_file(cell_dir.as_path()) {
                        return Err(mixed_layout_error(subblock_dir.as_path()));
                    }
                    continue;
                }
                write_projected_cell_dir_streaming(
                    builder,
                    cell_dir.as_path(),
                    context,
                    header_size,
                )?;
            }
            builder.end_group(sub_start);
        }
        builder.end_group(block_start);
    }
    Ok(())
}

/// How a projected cell nests under the WRLD children group. A `Loose` cell is
/// emitted directly under the children group (no block/subblock wrapper); a
/// `Grid` cell lives inside an exterior block + subblock group that the serial
/// emitter opens/closes on coordinate changes.
enum CellGrouping {
    Loose,
    Grid {
        block_label: [u8; 4],
        block_group_type: i32,
        subblock_label: [u8; 4],
        subblock_group_type: i32,
    },
}

/// A flat, ordered cell work-item discovered during the projected-WRLD walk.
/// Collected in `sorted_directory_entries` order so the serial emitter — and the
/// parallel pipeline — reproduce the original GRUP nesting.
struct ProjectedCellTask {
    grouping: CellGrouping,
    cell_dir: PathBuf,
}

/// Walk a projected worldspace dir into a flat list of cell tasks in exact
/// `sorted_directory_entries` order, applying every `mixed_layout_error` /
/// unsupported-entry guard. The WRLD record and its TopCell are handled by the
/// caller; this collects only the block/subblock/cell tree plus loose cells
/// directly under the world.
///
/// A grid block/subblock dir with zero cell dirs yields no task, so no empty
/// GRUP is emitted for it. Export only writes populated grid dirs.
fn collect_projected_wrld_cell_tasks(
    world_dir: &Path,
    world_record_path: &Path,
) -> PyResult<Vec<ProjectedCellTask>> {
    let mut tasks = Vec::new();
    for block_dir in sorted_directory_entries(world_dir)? {
        if block_dir == world_record_path {
            continue;
        }
        if block_dir.is_file() {
            let bn = block_dir
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or_default();
            if bn.starts_with(GROUP_RECORD_DATA_STEM) {
                continue;
            }
            if is_record_file(block_dir.as_path()) {
                return Err(mixed_layout_error(world_dir));
            }
            continue;
        }
        if parse_group_dir_name(
            block_dir
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or_default(),
            false,
        )
        .is_some()
        {
            return Err(mixed_layout_error(world_dir));
        }
        let Some((block_x, block_y)) = parse_grid_dir_name(
            block_dir
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or_default(),
        ) else {
            if find_named_payload_file(block_dir.as_path(), RECORD_DATA_STEM).is_some() {
                tasks.push(ProjectedCellTask {
                    grouping: CellGrouping::Loose,
                    cell_dir: block_dir,
                });
                continue;
            }
            return Err(value_error(format!(
                "Unsupported projected worldspace entry: {}",
                block_dir.display()
            )));
        };

        let block_group_type =
            parse_group_record_data_native(block_dir.as_path(), Some(EXTERIOR_CELL_BLOCK))?
                .unwrap_or(EXTERIOR_CELL_BLOCK);
        let block_label = encode_exterior_grid_label(block_x, block_y);

        for subblock_dir in sorted_directory_entries(block_dir.as_path())? {
            if subblock_dir.is_file() {
                if subblock_dir
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or_default()
                    .starts_with(GROUP_RECORD_DATA_STEM)
                {
                    continue;
                }
                if is_record_file(subblock_dir.as_path()) {
                    return Err(mixed_layout_error(block_dir.as_path()));
                }
                continue;
            }
            if parse_group_dir_name(
                subblock_dir
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or_default(),
                false,
            )
            .is_some()
            {
                return Err(mixed_layout_error(block_dir.as_path()));
            }
            let (sub_x, sub_y) = parse_grid_dir_name(
                subblock_dir
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or_default(),
            )
            .ok_or_else(|| {
                value_error(format!(
                    "Unsupported projected worldspace subblock: {}",
                    subblock_dir.display()
                ))
            })?;
            let sub_group_type = parse_group_record_data_native(
                subblock_dir.as_path(),
                Some(EXTERIOR_CELL_SUBBLOCK),
            )?
            .unwrap_or(EXTERIOR_CELL_SUBBLOCK);
            let subblock_label = encode_exterior_grid_label(sub_x, sub_y);

            for cell_dir in sorted_directory_entries(subblock_dir.as_path())? {
                if cell_dir.is_file() {
                    if cell_dir
                        .file_name()
                        .and_then(|value| value.to_str())
                        .unwrap_or_default()
                        .starts_with(GROUP_RECORD_DATA_STEM)
                    {
                        continue;
                    }
                    if is_record_file(cell_dir.as_path()) {
                        return Err(mixed_layout_error(subblock_dir.as_path()));
                    }
                    continue;
                }
                tasks.push(ProjectedCellTask {
                    grouping: CellGrouping::Grid {
                        block_label,
                        block_group_type,
                        subblock_label,
                        subblock_group_type: sub_group_type,
                    },
                    cell_dir,
                });
            }
        }
    }
    Ok(tasks)
}

/// Tracks the currently-open exterior block + subblock GRUPs while emitting a
/// stream of cell tasks, opening/closing nesting on coordinate changes exactly
/// like the former nested-loop walk. Shared by the serial emitter and the
/// parallel consumer so the byte-exact GRUP tree has a single source of truth.
struct GroupCursor {
    open: Option<([u8; 4], [u8; 4])>,
    block_start: u64,
    sub_start: u64,
}

impl GroupCursor {
    fn new() -> Self {
        Self {
            open: None,
            block_start: 0,
            sub_start: 0,
        }
    }

    fn enter<W: Write>(
        &mut self,
        builder: &mut StreamingEspBuilder<W>,
        grouping: &CellGrouping,
        header_size: usize,
    ) -> PyResult<()> {
        match grouping {
            CellGrouping::Loose => {
                if self.open.take().is_some() {
                    builder.end_group(self.sub_start);
                    builder.end_group(self.block_start);
                }
            }
            CellGrouping::Grid {
                block_label,
                block_group_type,
                subblock_label,
                subblock_group_type,
            } => match self.open {
                Some((cur_block, cur_sub))
                    if cur_block == *block_label && cur_sub == *subblock_label => {}
                Some((cur_block, _)) if cur_block == *block_label => {
                    builder.end_group(self.sub_start);
                    self.sub_start = builder
                        .begin_group(*subblock_label, *subblock_group_type, header_size)
                        .map_err(|e| io_error(format!("begin exterior subblock: {e}")))?;
                    self.open = Some((*block_label, *subblock_label));
                }
                _ => {
                    if self.open.is_some() {
                        builder.end_group(self.sub_start);
                        builder.end_group(self.block_start);
                    }
                    self.block_start = builder
                        .begin_group(*block_label, *block_group_type, header_size)
                        .map_err(|e| io_error(format!("begin exterior block: {e}")))?;
                    self.sub_start = builder
                        .begin_group(*subblock_label, *subblock_group_type, header_size)
                        .map_err(|e| io_error(format!("begin exterior subblock: {e}")))?;
                    self.open = Some((*block_label, *subblock_label));
                }
            },
        }
        Ok(())
    }

    fn close<W: Write>(&mut self, builder: &mut StreamingEspBuilder<W>) {
        if self.open.take().is_some() {
            builder.end_group(self.sub_start);
            builder.end_group(self.block_start);
        }
    }
}

/// Serially emit collected cell tasks, opening/closing exterior block &
/// subblock groups on coordinate changes so the GRUP nesting matches the former
/// nested-loop walk byte-for-byte. Each cell is loaded + validated + written by
/// `write_projected_cell_dir_streaming` — the same call the old loops made.
fn emit_projected_cell_tasks_serial<W: Write>(
    builder: &mut StreamingEspBuilder<W>,
    tasks: &[ProjectedCellTask],
    context: &mut NativeImportContext,
    header_size: usize,
) -> PyResult<()> {
    let mut cursor = GroupCursor::new();
    for task in tasks {
        cursor.enter(builder, &task.grouping, header_size)?;
        write_projected_cell_dir_streaming(builder, task.cell_dir.as_path(), context, header_size)?;
    }
    cursor.close(builder);
    Ok(())
}

/// Parallel projected-cell decode with a serial ordered write. Worker threads
/// load + decode each cell (the expensive YAML→JSON→ParsedRecord work) into a
/// per-cell sub-context; a single consumer reorders results by index,
/// opens/closes block & subblock GRUPs on coordinate changes, and reconciles
/// each cell's sub-context deltas into the parent before writing. Output is
/// byte-identical to the serial path.
///
/// String-ID determinism: the consumer renumbers each cell's allocated IDs to a
/// fresh contiguous parent range in index order, so the global assignment order
/// matches serial regardless of the per-cell sub-context stride.
fn parallel_decode_projected_cells<W: Write>(
    builder: &mut StreamingEspBuilder<W>,
    tasks: &[ProjectedCellTask],
    parent: &mut NativeImportContext,
    workers: usize,
    header_size: usize,
) -> PyResult<()> {
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::sync::mpsc::sync_channel;

    if tasks.is_empty() {
        return Ok(());
    }
    let workers = workers.max(1).min(tasks.len());

    let parent_next_id = parent.next_localized_string_id();
    let plugin_name = parent.plugin_name.clone();
    let game = parent.game.clone();
    let base_header = parent.header.clone();
    let base_default_lang = parent.strings.default_language.clone();

    let (tx_in, rx_in) = sync_channel::<(usize, &ProjectedCellTask)>(workers * 2);
    let (tx_out, rx_out) = sync_channel::<(usize, PyResult<DecodedProjectedCell>)>(workers * 2);
    let rx_in_shared = Arc::new(Mutex::new(rx_in));

    std::thread::scope(|scope| -> PyResult<()> {
        scope.spawn(move || {
            for (idx, task) in tasks.iter().enumerate() {
                if tx_in.send((idx, task)).is_err() {
                    return;
                }
            }
        });

        for _ in 0..workers {
            let rx_in = Arc::clone(&rx_in_shared);
            let tx_out = tx_out.clone();
            let plugin_name = plugin_name.clone();
            let game = game.clone();
            let base_header = base_header.clone();
            let base_default_lang = base_default_lang.clone();
            scope.spawn(move || {
                loop {
                    let item = match rx_in.lock().unwrap().recv() {
                        Ok(x) => x,
                        Err(_) => break,
                    };
                    let (idx, task) = item;
                    let result = (|| -> PyResult<DecodedProjectedCell> {
                        let offset = (idx as u32)
                            .checked_mul(COMPACT_PARALLEL_RECORD_ID_STRIDE)
                            .ok_or_else(|| value_error("cell stride overflow"))?;
                        let start_id = parent_next_id
                            .checked_add(offset)
                            .ok_or_else(|| value_error("cell stride start overflow"))?;
                        let mut sub = NativeImportContext::new(
                            plugin_name.clone(),
                            game.clone(),
                            header_size,
                            base_header.clone(),
                        );
                        sub.strings.default_language = base_default_lang.clone();
                        sub.set_next_localized_string_id(start_id);
                        let payload =
                            load_projected_cell_payload_from_dir(task.cell_dir.as_path(), &sub)?;
                        let mut decoded = decode_projected_cell_payload(&payload, &mut sub)?;
                        decoded.masters = std::mem::take(&mut sub.header.masters);
                        decoded.strings = std::mem::take(&mut sub.strings);
                        decoded.allocated_string_ids =
                            std::mem::take(&mut sub.allocated_localized_string_ids);
                        Ok(decoded)
                    })();
                    if tx_out.send((idx, result)).is_err() {
                        break;
                    }
                }
            });
        }
        drop(tx_out);

        // Serial consumer: same deadlock-avoidance drain as the per-record
        // pipeline. Reorder by index, manage GRUP nesting, reconcile + write.
        let mut buffer: HashMap<usize, DecodedProjectedCell> = HashMap::new();
        let mut next_idx = 0usize;
        let mut next_string_id = parent_next_id;
        let mut cursor = GroupCursor::new();
        let mut deferred_err: Option<PyErr> = None;
        for (idx, result) in rx_out.iter() {
            if deferred_err.is_some() {
                continue;
            }
            let decoded = match result {
                Ok(decoded) => decoded,
                Err(e) => {
                    deferred_err = Some(e);
                    continue;
                }
            };
            buffer.insert(idx, decoded);
            while let Some(decoded) = buffer.remove(&next_idx) {
                if let Err(e) = cursor.enter(builder, &tasks[next_idx].grouping, header_size) {
                    deferred_err = Some(e);
                    break;
                }
                if let Err(e) = write_decoded_projected_cell(
                    builder,
                    decoded,
                    parent,
                    &mut next_string_id,
                    header_size,
                    true,
                ) {
                    deferred_err = Some(e);
                    break;
                }
                next_idx += 1;
            }
        }
        if let Some(err) = deferred_err {
            return Err(err);
        }
        cursor.close(builder);
        parent.set_next_localized_string_id(next_string_id);
        Ok(())
    })
}

/// Stream-write the body of a projected WRLD root group, emitting world
/// records and their child cell groups as .esp bytes directly.
fn walk_projected_wrld_streaming<W: Write>(
    builder: &mut StreamingEspBuilder<W>,
    path: &Path,
    context: &mut NativeImportContext,
    header_size: usize,
) -> PyResult<()> {
    for world_dir in sorted_directory_entries(path)? {
        if world_dir.is_file() {
            if is_record_file(world_dir.as_path()) {
                return Err(mixed_layout_error(path));
            }
            continue;
        }
        if parse_group_dir_name(
            world_dir
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or_default(),
            false,
        )
        .is_some()
        {
            return Err(mixed_layout_error(path));
        }

        let world_record_path = find_named_payload_file(world_dir.as_path(), RECORD_DATA_STEM)
            .ok_or_else(|| {
                value_error(format!(
                    "Missing {RECORD_DATA_STEM} in {}",
                    world_dir.display()
                ))
            })?;
        let mut world_payload = load_mapping_payload_object(context, world_record_path.as_path())?;
        if !world_payload.contains_key("signature") {
            world_payload.insert(
                "signature".to_string(),
                JsonValue::String("WRLD".to_string()),
            );
        }
        let world_record = parse_record_from_json_compact_native(&world_payload, context)?;
        let world_form_id = world_record.form_id;
        builder.write_record(&world_record, header_size)?;
        drop(world_record);

        // Open WRLD_CHILDREN_GROUP (group_type=1, label=worldspace form_id)
        let world_children_start = builder
            .begin_group(world_form_id.to_le_bytes(), 1, header_size)
            .map_err(|e| io_error(format!("begin WRLD children: {e}")))?;

        if let Some(top_cell_value) = world_payload.get("TopCell") {
            let top_cell_payload = json_object(top_cell_value, "TopCell")?;
            write_projected_cell_payload_streaming(
                builder,
                top_cell_payload,
                context,
                header_size,
            )?;
        }

        let tasks =
            collect_projected_wrld_cell_tasks(world_dir.as_path(), world_record_path.as_path())?;
        if tasks.len() >= STREAM_PARALLEL_RECORD_THRESHOLD {
            // rayon::current_num_threads() honors the `jobs` pool that
            // build_authoring_dir_streaming_native wraps this walk in.
            let workers = rayon::current_num_threads();
            parallel_decode_projected_cells(builder, &tasks, context, workers, header_size)?;
        } else {
            emit_projected_cell_tasks_serial(builder, &tasks, context, header_size)?;
        }

        builder.end_group(world_children_start);
    }
    Ok(())
}

/// Public entry point: stream-build a .esp from an authoring directory without
/// materializing the full plugin tree. Returns Ok on success.
///
/// `jobs` controls the parallel-decode thread count. None = use the global
/// rayon pool (= num_cpus, fastest, peak RSS up to ~9 GB on Starfield-scale
/// plugins). Lower values trade speed for bounded memory; jobs=1 forces
/// serial decode (peak ~1 GB on Starfield, ~30 min wall-clock).
pub(crate) fn build_authoring_dir_streaming_native(
    source_dir: &str,
    output_path: &str,
    game: Option<&str>,
    jobs: Option<usize>,
    master_esm_paths: Option<&[String]>,
) -> PyResult<()> {
    let authoring_dir = Path::new(source_dir);
    let manifest_path = if authoring_dir.join("plugin.json").is_file() {
        authoring_dir.join("plugin.json")
    } else if authoring_dir.join("plugin.yaml").is_file() {
        authoring_dir.join("plugin.yaml")
    } else {
        return Err(pyo3::exceptions::PyFileNotFoundError::new_err(format!(
            "no authoring-dir manifest in '{}'",
            authoring_dir.display()
        )));
    };
    let manifest_value = load_mapping_payload_value_native(manifest_path.as_path())?;
    let manifest_mapping = json_object(&manifest_value, "authoring-dir manifest")?;

    let plugin_name = manifest_mapping
        .get("plugin")
        .and_then(|value| value.as_str())
        .unwrap_or("Plugin.esp")
        .to_string();
    let manifest_game = manifest_mapping
        .get("game")
        .and_then(|value| value.as_str())
        .map(|value| value.to_string());
    let resolved_game = game.map(|value| value.to_string()).or(manifest_game);
    let header_size = match manifest_mapping.get("header_size") {
        Some(value) if !value.is_null() => {
            json_parse_int(value, "header_size", MODERN_HEADER_SIZE as i64)? as usize
        }
        _ => match resolved_game.as_deref() {
            Some("oblivion" | "fo3" | "fnv") => LEGACY_HEADER_SIZE,
            _ => MODERN_HEADER_SIZE,
        },
    };
    let header_payload = json_object(
        manifest_mapping
            .get("header")
            .ok_or_else(|| value_error("missing header payload"))?,
        "header",
    )?;
    let header = parse_plugin_header_from_json_native(header_payload)?;
    let mut context =
        NativeImportContext::new(plugin_name.clone(), resolved_game, header_size, header);
    load_authoring_strings_into_context(&mut context, authoring_dir);

    trace_export_phase(format_args!(
        "streaming build starting: out='{}' game={:?} header_size={}",
        output_path, context.game, header_size
    ));

    let file = File::create(output_path)
        .map_err(|err| io_error(format!("failed to create '{output_path}': {err}")))?;
    let writer = BufWriter::with_capacity(8 * 1024 * 1024, file);
    let mut builder = StreamingEspBuilder::new(writer);

    let hedr_num_records_offset = write_tes4_header_streaming(&mut builder, &context, header_size)?;

    let records_dir = authoring_dir.join("records");
    let walk_records = |builder: &mut StreamingEspBuilder<BufWriter<File>>,
                        context: &mut NativeImportContext|
     -> PyResult<()> {
        if !records_dir.is_dir() {
            return Ok(());
        }
        // Sort top-level record-type subdirs by the engine's canonical GRUP
        // order (KYWD before COBJ etc.; see the top of this file). Live
        // extraction from the first available master ESM wins; the bundled
        // per-game list covers builds without game data (e.g. CI). With
        // neither (no masters and no game), entries stay alphabetical.
        let mut root_entries: Vec<PathBuf> = sorted_directory_entries(records_dir.as_path())?
            .into_iter()
            .filter(|p| !p.is_file())
            .collect();
        let live_order: Option<Vec<String>> = master_esm_paths.and_then(|paths| {
            paths
                .iter()
                .find_map(|p| cached_master_group_order(Path::new(p)))
        });
        let hardcoded = top_level_group_order_for_game(context.game.as_deref());
        let order_strs: Option<Vec<&str>> = match (live_order.as_ref(), hardcoded) {
            (Some(live), _) => Some(live.iter().map(String::as_str).collect()),
            (None, Some(baseline)) => Some(baseline.iter().copied().collect()),
            (None, None) => None,
        };
        if let Some(order) = order_strs {
            let rank: std::collections::HashMap<&str, usize> =
                order.iter().enumerate().map(|(i, sig)| (*sig, i)).collect();
            root_entries.sort_by(|a, b| {
                let key = |p: &Path| -> (usize, String) {
                    let name = p
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or_default()
                        .to_string();
                    let r = rank.get(name.as_str()).copied().unwrap_or(usize::MAX);
                    (r, name.to_ascii_lowercase())
                };
                key(a).cmp(&key(b))
            });
        }
        for entry in root_entries {
            let entry_name = entry
                .file_name()
                .and_then(|value| value.to_str())
                .ok_or_else(|| {
                    value_error(format!("invalid directory name '{}'", entry.display()))
                })?
                .to_string();
            if entry_name.len() != 4 {
                return Err(value_error(format!(
                    "unsupported authoring root entry '{}'",
                    entry.display()
                )));
            }
            let root_start_bytes = builder.bytes_written;
            trace_export_phase(format_args!(
                "streaming root '{}' starting (bytes_so_far={})",
                entry_name, root_start_bytes
            ));
            walk_root_signature_streaming(
                builder,
                entry.as_path(),
                entry_name.as_str(),
                context,
                header_size,
            )?;
            trace_export_phase(format_args!(
                "streaming root '{}' done: bytes_in_root={}",
                entry_name,
                builder.bytes_written - root_start_bytes
            ));
        }
        Ok(())
    };

    // If `jobs` is set, scope all parallel decodes inside a custom rayon pool
    // so the user's choice bounds peak RSS. Otherwise use the global pool
    // (fastest but uncapped).
    if let Some(jobs) = jobs {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(jobs.max(1))
            .build()
            .map_err(|err| value_error(format!("rayon pool error: {err}")))?;
        pool.install(|| walk_records(&mut builder, &mut context))?;
    } else {
        walk_records(&mut builder, &mut context)?;
    }

    let total_groups = builder.group_patches.len();
    let record_count = builder.record_count;
    trace_export_phase(format_args!(
        "streaming build write done: bytes={} groups={} records={}",
        builder.bytes_written, total_groups, record_count
    ));

    // Flush BufWriter and patch group sizes.
    let mut file = builder
        .writer
        .into_inner()
        .map_err(|err| io_error(format!("flush failed: {err}")))?;
    for (offset, size) in &builder.group_patches {
        file.seek(SeekFrom::Start(offset + 4))
            .map_err(|e| io_error(format!("seek for patch: {e}")))?;
        file.write_all(&size.to_le_bytes())
            .map_err(|e| io_error(format!("patch group size: {e}")))?;
    }
    let offset = hedr_num_records_offset
        .ok_or_else(|| value_error("TES4 HEDR missing num_records field"))?;
    // HEDR.NumRecords = records + groups (excluding TES4) per UESP spec.
    let hedr_entries = record_count + total_groups;
    if hedr_entries > u32::MAX as usize {
        return Err(value_error(format!(
            "authoring directory has too many records+groups for TES4.HEDR: {hedr_entries}"
        )));
    }
    let num_records = hedr_entries as u32;
    file.seek(SeekFrom::Start(offset))
        .map_err(|e| io_error(format!("seek for HEDR.NumRecords patch: {e}")))?;
    file.write_all(&num_records.to_le_bytes())
        .map_err(|e| io_error(format!("patch HEDR.NumRecords: {e}")))?;
    context.header.num_records = num_records;
    file.flush()
        .map_err(|err| io_error(format!("final flush: {err}")))?;

    trace_export_phase(format_args!(
        "streaming build complete: patched {} group sizes",
        total_groups
    ));

    // Strings sidecar (uses a minimal ParsedPlugin shell)
    let plugin_shell = ParsedPlugin {
        plugin_name,
        file_path: String::new(),
        header_size: context.header_size,
        header: context.header.clone(),
        root_items: Vec::new(),
        game: context.game.clone(),
    };
    write_localized_strings_for_parsed(&plugin_shell, &context.strings, output_path)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_record(signature: &str, form_id: u32) -> ParsedRecord {
        ParsedRecord {
            signature: SmolStr::new(signature),
            form_id,
            flags: 0,
            version_control: 0,
            form_version: None,
            version2: None,
            subrecords: Vec::new(),
            raw_payload: None,
            parse_error: None,
        }
    }

    fn make_info_record(form_id: u32, previous_form_id: u32) -> ParsedRecord {
        let mut record = make_record("INFO", form_id);
        record.subrecords.push(ParsedSubrecord {
            signature: SmolStr::new_static("PNAM"),
            data: Bytes::from(previous_form_id.to_le_bytes().to_vec()),
            semantic_type: None,
        });
        record
    }

    #[test]
    fn info_records_sort_by_previous_info_chain() {
        let mut records = vec![
            make_info_record(0x0001D8, 0x030001DA),
            make_info_record(0x0001D9, 0x030001E5),
            make_info_record(0x0001DA, 0x030001D9),
            make_info_record(0x0001E5, 0x030001ED),
            make_info_record(0x0001ED, 0x030001EE),
            make_info_record(0x0001EE, 0x00000000),
        ];

        sort_info_records_by_previous(&mut records);

        let order: Vec<u32> = records
            .iter()
            .map(|record| record.form_id & 0x00FF_FFFF)
            .collect();
        assert_eq!(
            order,
            vec![0x0001EE, 0x0001ED, 0x0001E5, 0x0001D9, 0x0001DA, 0x0001D8]
        );
    }

    #[test]
    fn exterior_grid_projection_uses_x_y_directory_order() {
        let group = ParsedGroup {
            label: encode_exterior_grid_label(-17, 0),
            group_type: EXTERIOR_CELL_BLOCK,
            tail: Bytes::new(),
            children: Vec::new(),
        };

        assert_eq!(grid_dir_name_from_group_native(&group).unwrap(), "-17, 0");
    }

    #[test]
    fn trace_progress_is_disabled_when_trace_is_off() {
        assert!(!should_trace_export_progress(false, false, 1, 10, 1000));
        assert!(!should_trace_export_progress(false, true, 10, 10, 1000));
        assert!(!should_trace_export_progress(false, false, 500, 1000, 250));
    }

    #[test]
    fn trace_progress_respects_verbose_and_interval_when_enabled() {
        assert!(should_trace_export_progress(true, false, 1, 10, 1000));
        assert!(should_trace_export_progress(true, false, 10, 10, 1000));
        assert!(should_trace_export_progress(true, false, 500, 1000, 250));
        assert!(should_trace_export_progress(true, true, 7, 10, 1000));
    }

    #[test]
    fn compact_manifest_flags_emit_camel_case_strings() {
        let mut plugin = empty_parsed_plugin("Patch.esp", Some("fo4"));
        plugin.header.flags = 0x0000_0001 | 0x0000_0080;

        let manifest = authoring_manifest_payload_json(&plugin, 0);
        let flags = manifest
            .get("header")
            .and_then(JsonValue::as_object)
            .and_then(|header| header.get("flags"))
            .and_then(JsonValue::as_array)
            .expect("header flags");

        assert_eq!(
            flags,
            &vec![
                JsonValue::String("MasterFile".to_string()),
                JsonValue::String("Localized".to_string()),
            ]
        );
        assert_eq!(
            json_parse_header_flags(Some(&JsonValue::Array(flags.clone()))).unwrap(),
            plugin.header.flags
        );
    }

    #[test]
    fn world_payload_embeds_top_cell_navigation_meshes() {
        let plugin = empty_parsed_plugin("Fallout3.esm", Some("fo3"));
        let strings = LocalizedStringsState::default();
        let world = make_record("WRLD", 0x0000_003C);
        let cell = make_record("CELL", 0x0000_2DB4);
        let navmesh = make_record("NAVM", 0x0004_503B);
        let cell_group = ParsedGroup {
            label: cell.form_id.to_le_bytes(),
            group_type: CELL_CHILD_GROUP,
            tail: Bytes::new(),
            children: vec![ParsedItem::Record(navmesh)],
        };

        let payload = world_payload_json(&world, Some(&cell), Some(&cell_group), &plugin, &strings);
        let top_cell = payload
            .get("TopCell")
            .and_then(JsonValue::as_object)
            .expect("TopCell payload");
        let navigation_meshes = top_cell
            .get("NavigationMeshes")
            .and_then(JsonValue::as_array)
            .expect("NavigationMeshes payload");

        assert_eq!(
            top_cell.get("form_id").and_then(JsonValue::as_str),
            Some("002DB4")
        );
        assert_eq!(navigation_meshes.len(), 1);
        assert_eq!(
            navigation_meshes[0]
                .get("form_id")
                .and_then(JsonValue::as_str),
            Some("04503B")
        );
    }

    fn make_cell_with_temporary(
        cell_form_id: u32,
        refr_form_id: u32,
    ) -> (ParsedRecord, ParsedGroup) {
        let cell = make_record("CELL", cell_form_id);
        let refr = make_record("REFR", refr_form_id);
        let temporary = ParsedGroup {
            label: cell_form_id.to_le_bytes(),
            group_type: TEMPORARY_GROUP,
            tail: Bytes::new(),
            children: vec![ParsedItem::Record(refr)],
        };
        let cell_child = ParsedGroup {
            label: cell_form_id.to_le_bytes(),
            group_type: CELL_CHILD_GROUP,
            tail: Bytes::new(),
            children: vec![ParsedItem::Group(temporary)],
        };
        (cell, cell_child)
    }

    /// Build a minimal exterior worldspace (one block, one subblock, `num_cells`
    /// cells, each with a Temporary REFR) as the projected-WRLD authoring layout
    /// expects. Exported to disk it is a valid projected dir; building it back is
    /// the unit under test.
    fn build_exterior_wrld_plugin(num_cells: u32) -> ParsedPlugin {
        let mut plugin = empty_parsed_plugin("B21_PWrldTest.esm", Some("fo4"));
        plugin.header.masters = vec!["Fallout4.esm".to_string()];
        plugin.header.master_sizes = vec![0];

        let world = make_record("WRLD", 0x0000_0800);
        let world_form_id = world.form_id;

        let mut subblock_children: Vec<ParsedItem> = Vec::new();
        for i in 0..num_cells {
            let (cell, cell_child) = make_cell_with_temporary(0x0000_0900 + i, 0x0000_4000 + i);
            subblock_children.push(ParsedItem::Record(cell));
            subblock_children.push(ParsedItem::Group(cell_child));
        }
        let subblock = ParsedGroup {
            label: encode_exterior_grid_label(0, 0),
            group_type: EXTERIOR_CELL_SUBBLOCK,
            tail: Bytes::new(),
            children: subblock_children,
        };
        let block = ParsedGroup {
            label: encode_exterior_grid_label(0, 0),
            group_type: EXTERIOR_CELL_BLOCK,
            tail: Bytes::new(),
            children: vec![ParsedItem::Group(subblock)],
        };
        let wrld_children = ParsedGroup {
            label: world_form_id.to_le_bytes(),
            group_type: 1,
            tail: Bytes::new(),
            children: vec![ParsedItem::Group(block)],
        };
        let wrld_top = ParsedGroup {
            label: *b"WRLD",
            group_type: 0,
            tail: Bytes::new(),
            children: vec![ParsedItem::Record(world), ParsedItem::Group(wrld_children)],
        };
        plugin.root_items = vec![ParsedItem::Group(wrld_top)];
        plugin
    }

    #[test]
    fn projected_wrld_streaming_build_is_worker_count_invariant() {
        let plugin = build_exterior_wrld_plugin(40);
        let strings = LocalizedStringsState::default();
        let tmp = std::env::temp_dir().join(format!("modbox_pwrld_build_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let authoring = tmp.join("yaml");
        export_authoring_dir_no_py(&plugin, &strings, &authoring, "yaml").unwrap();

        // Sanity: export produced the projected WRLD layout we intend to exercise.
        let wrld_dir = authoring.join("records").join("WRLD");
        assert!(wrld_dir.is_dir(), "expected projected WRLD records dir");

        let out_serial = tmp.join("serial.esm");
        let out_parallel = tmp.join("parallel.esm");
        build_authoring_dir_streaming_native(
            authoring.to_str().unwrap(),
            out_serial.to_str().unwrap(),
            Some("fo4"),
            Some(1),
            None,
        )
        .unwrap();
        build_authoring_dir_streaming_native(
            authoring.to_str().unwrap(),
            out_parallel.to_str().unwrap(),
            Some("fo4"),
            Some(8),
            None,
        )
        .unwrap();

        let a = std::fs::read(&out_serial).unwrap();
        let b = std::fs::read(&out_parallel).unwrap();
        let _ = std::fs::remove_dir_all(&tmp);

        assert_eq!(&a[..4], b"TES4");
        assert!(
            a.windows(4).any(|w| w == b"GRUP"),
            "expected GRUP in output"
        );
        assert_eq!(
            a, b,
            "streaming build output must be identical for jobs=1 vs jobs=8"
        );
    }

    /// Build an exterior worldspace spanning `num_blocks` blocks ×
    /// `num_subblocks` subblocks × `cells_per_subblock` cells, so the projected
    /// build's emitter has to open/close block and subblock GRUPs across
    /// coordinate changes (same-subblock, new-subblock, new-block).
    fn build_multi_block_wrld_plugin(
        num_blocks: i16,
        num_subblocks: i16,
        cells_per_subblock: u32,
    ) -> ParsedPlugin {
        let mut plugin = empty_parsed_plugin("B21_PWrldMultiTest.esm", Some("fo4"));
        plugin.header.masters = vec!["Fallout4.esm".to_string()];
        plugin.header.master_sizes = vec![0];

        let world = make_record("WRLD", 0x0000_0800);
        let world_form_id = world.form_id;

        let mut next_cell: u32 = 0x0000_0900;
        let mut next_refr: u32 = 0x0000_4000;
        let mut block_items: Vec<ParsedItem> = Vec::new();
        for bx in 0..num_blocks {
            let mut subblock_items: Vec<ParsedItem> = Vec::new();
            for sx in 0..num_subblocks {
                let mut cells: Vec<ParsedItem> = Vec::new();
                for _ in 0..cells_per_subblock {
                    let (cell, cell_child) = make_cell_with_temporary(next_cell, next_refr);
                    next_cell += 1;
                    next_refr += 1;
                    cells.push(ParsedItem::Record(cell));
                    cells.push(ParsedItem::Group(cell_child));
                }
                subblock_items.push(ParsedItem::Group(ParsedGroup {
                    label: encode_exterior_grid_label(sx, 0),
                    group_type: EXTERIOR_CELL_SUBBLOCK,
                    tail: Bytes::new(),
                    children: cells,
                }));
            }
            block_items.push(ParsedItem::Group(ParsedGroup {
                label: encode_exterior_grid_label(bx, 0),
                group_type: EXTERIOR_CELL_BLOCK,
                tail: Bytes::new(),
                children: subblock_items,
            }));
        }
        let wrld_children = ParsedGroup {
            label: world_form_id.to_le_bytes(),
            group_type: 1,
            tail: Bytes::new(),
            children: block_items,
        };
        let wrld_top = ParsedGroup {
            label: *b"WRLD",
            group_type: 0,
            tail: Bytes::new(),
            children: vec![ParsedItem::Record(world), ParsedItem::Group(wrld_children)],
        };
        plugin.root_items = vec![ParsedItem::Group(wrld_top)];
        plugin
    }

    #[test]
    fn projected_wrld_multi_block_streaming_build_is_worker_count_invariant() {
        // 3 blocks × 2 subblocks × 7 cells = 42 cells (≥ the parallel threshold),
        // exercising every group transition in the emitter.
        let plugin = build_multi_block_wrld_plugin(3, 2, 7);
        let strings = LocalizedStringsState::default();
        let tmp = std::env::temp_dir().join(format!("modbox_pwrld_multi_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let authoring = tmp.join("yaml");
        export_authoring_dir_no_py(&plugin, &strings, &authoring, "yaml").unwrap();

        let out_serial = tmp.join("serial.esm");
        let out_parallel = tmp.join("parallel.esm");
        build_authoring_dir_streaming_native(
            authoring.to_str().unwrap(),
            out_serial.to_str().unwrap(),
            Some("fo4"),
            Some(1),
            None,
        )
        .unwrap();
        build_authoring_dir_streaming_native(
            authoring.to_str().unwrap(),
            out_parallel.to_str().unwrap(),
            Some("fo4"),
            Some(8),
            None,
        )
        .unwrap();

        let a = std::fs::read(&out_serial).unwrap();
        let b = std::fs::read(&out_parallel).unwrap();
        let _ = std::fs::remove_dir_all(&tmp);

        assert_eq!(&a[..4], b"TES4");
        // Floor check: at least the 3 block + 6 subblock GRUPs must be present.
        let grup_count = a.windows(4).filter(|w| *w == b"GRUP").count();
        assert!(
            grup_count >= 3 + 6,
            "expected block + subblock GRUPs, got {grup_count}"
        );
        assert_eq!(
            a, b,
            "multi-block build output must be identical for jobs=1 vs jobs=8"
        );
    }

    /// Build a *localized* exterior worldspace where every cell carries a
    /// resolvable `FULL` display name. Exported to the authoring dir these
    /// FULLs externalize to text (no raw_hex), so on rebuild each cell
    /// RE-ALLOCATES a fresh localized string ID — exercising the parallel
    /// consumer's non-empty `id_map` renumber path that the bare-REFR fixtures
    /// never reach. Returns the plugin plus the strings table the export needs
    /// to resolve each FULL.
    fn build_localized_wrld_plugin(num_cells: u32) -> (ParsedPlugin, LocalizedStringsState) {
        let mut plugin = empty_parsed_plugin("B21_PWrldLocTest.esm", Some("fo4"));
        plugin.header.flags |= 0x80; // Localized
        plugin.header.masters = vec!["Fallout4.esm".to_string()];
        plugin.header.master_sizes = vec![0];

        let world = make_record("WRLD", 0x0000_0800);
        let world_form_id = world.form_id;

        let mut en: HashMap<u32, String> = HashMap::new();
        let mut subblock_children: Vec<ParsedItem> = Vec::new();
        for i in 0..num_cells {
            let string_id = i + 1;
            en.insert(string_id, format!("Projected Cell {i}"));
            let (mut cell, cell_child) = make_cell_with_temporary(0x0000_0900 + i, 0x0000_4000 + i);
            cell.subrecords.push(ParsedSubrecord {
                signature: SmolStr::new_static("FULL"),
                data: Bytes::from(string_id.to_le_bytes().to_vec()),
                semantic_type: None,
            });
            subblock_children.push(ParsedItem::Record(cell));
            subblock_children.push(ParsedItem::Group(cell_child));
        }

        let strings = LocalizedStringsState {
            by_language: HashMap::from([("en".to_string(), en)]),
            default_language: "en".to_string(),
            ..LocalizedStringsState::default()
        };

        let subblock = ParsedGroup {
            label: encode_exterior_grid_label(0, 0),
            group_type: EXTERIOR_CELL_SUBBLOCK,
            tail: Bytes::new(),
            children: subblock_children,
        };
        let block = ParsedGroup {
            label: encode_exterior_grid_label(0, 0),
            group_type: EXTERIOR_CELL_BLOCK,
            tail: Bytes::new(),
            children: vec![ParsedItem::Group(subblock)],
        };
        let wrld_children = ParsedGroup {
            label: world_form_id.to_le_bytes(),
            group_type: 1,
            tail: Bytes::new(),
            children: vec![ParsedItem::Group(block)],
        };
        let wrld_top = ParsedGroup {
            label: *b"WRLD",
            group_type: 0,
            tail: Bytes::new(),
            children: vec![ParsedItem::Record(world), ParsedItem::Group(wrld_children)],
        };
        plugin.root_items = vec![ParsedItem::Group(wrld_top)];
        (plugin, strings)
    }

    /// Count occurrences of `needle` across every `RecordData.yaml` under `dir`.
    fn count_in_record_yaml(dir: &Path, needle: &str) -> usize {
        let mut total = 0;
        let mut stack = vec![dir.to_path_buf()];
        while let Some(path) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&path) else {
                continue;
            };
            for entry in entries.flatten() {
                let entry_path = entry.path();
                if entry_path.is_dir() {
                    stack.push(entry_path);
                } else if entry_path.file_name().and_then(|n| n.to_str()) == Some("RecordData.yaml")
                {
                    if let Ok(text) = std::fs::read_to_string(&entry_path) {
                        total += text.matches(needle).count();
                    }
                }
            }
        }
        total
    }

    #[test]
    fn projected_wrld_localized_streaming_build_is_worker_count_invariant() {
        let num_cells = 40u32;
        let (plugin, strings) = build_localized_wrld_plugin(num_cells);
        let tmp = std::env::temp_dir().join(format!("modbox_pwrld_loc_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let authoring = tmp.join("yaml");
        export_authoring_dir_no_py(&plugin, &strings, &authoring, "yaml").unwrap();

        // Self-check: every cell externalized its FULL as a localized value
        // (TargetLanguage marker, no raw_hex), so the rebuild allocates a fresh
        // string ID per cell. Without this the byte-identity assertion below
        // would pass trivially (empty id_map), like the bare-REFR fixtures.
        let wrld_dir = authoring.join("records").join("WRLD");
        let externalized = count_in_record_yaml(&wrld_dir, "TargetLanguage");
        assert!(
            externalized >= num_cells as usize,
            "expected >= {num_cells} externalized FULLs (one per cell), got {externalized}; \
             rebuild would not exercise the renumber path"
        );

        let out_serial = tmp.join("serial.esm");
        let out_parallel = tmp.join("parallel.esm");
        build_authoring_dir_streaming_native(
            authoring.to_str().unwrap(),
            out_serial.to_str().unwrap(),
            Some("fo4"),
            Some(1),
            None,
        )
        .unwrap();
        build_authoring_dir_streaming_native(
            authoring.to_str().unwrap(),
            out_parallel.to_str().unwrap(),
            Some("fo4"),
            Some(8),
            None,
        )
        .unwrap();

        let a = std::fs::read(&out_serial).unwrap();
        let b = std::fs::read(&out_parallel).unwrap();
        let _ = std::fs::remove_dir_all(&tmp);

        assert_eq!(&a[..4], b"TES4");
        assert_eq!(
            a, b,
            "localized build output (strided string-ID alloc + per-cell renumber) \
             must be identical for jobs=1 vs jobs=8"
        );
    }
}
