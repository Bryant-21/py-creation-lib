use super::*;
use std::borrow::Cow;
use std::collections::{BTreeSet, HashSet};

const SKIP_AUTHORING_FILES: [&str; 2] = ["plugin.yaml", "plugin.json"];

#[derive(Clone)]
struct AuthoringRef {
    path: String,
    line: usize,
    form_key: String,
}

struct AuthoringScan {
    internal: BTreeSet<String>,
    references: Vec<AuthoringRef>,
}

struct PluginManifest {
    mod_key: String,
    masters: Vec<String>,
}

type AuthoringErrorPayload = (String, usize, String, String);
type AuthoringValidationPayload = (Vec<AuthoringErrorPayload>, usize);

/// Validate a mod's authoring YAML directory. Returns
/// `(errors, checked_count)` where each error row is
/// `(file, line, formkey, reason)` and `checked_count` is the number of
/// FormKey references scanned. Policy applied:
///
/// * Internal refs (FormKey whose plugin name matches the mod's own plugin
///   name) must resolve to a record defined somewhere under the authoring
///   dir.
/// * External refs must target a plugin that is declared in
///   `plugin.yaml -> header.masters`.
/// * ESL constraints: the plugin may not list `.esp` masters, and every
///   own-plugin FormID must be ≤ `0x000FFF`.
///
/// No external content is consulted — only the mod's own YAML.
#[pyfunction(name = "validate_authoring")]
pub(crate) fn validate_authoring_native(
    py: Python<'_>,
    yaml_dir: &str,
) -> PyResult<AuthoringValidationPayload> {
    let yaml_root = PathBuf::from(yaml_dir);
    if !yaml_root.is_dir() {
        return Err(PyValueError::new_err(format!("{yaml_dir} not found")));
    }

    let yaml_dir_owned = yaml_dir.to_string();
    let (manifest, scan) = py.detach(move || -> PyResult<(PluginManifest, AuthoringScan)> {
        let manifest = parse_plugin_manifest(yaml_root.as_path())?;
        let mut internal = BTreeSet::<String>::new();
        let mut references = Vec::<AuthoringRef>::new();
        scan_authoring_dir(
            yaml_root.as_path(),
            manifest.mod_key.as_str(),
            &mut internal,
            &mut references,
        )?;
        Ok((
            manifest,
            AuthoringScan {
                internal,
                references,
            },
        ))
    })?;

    let mod_key = manifest.mod_key;
    let masters_set: HashSet<String> = manifest
        .masters
        .iter()
        .map(|m| m.to_ascii_lowercase())
        .collect();
    let is_esl = mod_key.to_ascii_lowercase().ends_with(".esl");
    let mod_root = yaml_root_parent(yaml_dir);
    let _ = yaml_dir_owned; // marker so detach closure isn't accidentally pulled back in

    let mut errors = Vec::new();
    let mut checked: usize = 0;

    if is_esl {
        for master in &manifest.masters {
            if master.to_ascii_lowercase().ends_with(".esp") {
                push_error(
                    &mut errors,
                    "plugin.yaml",
                    0,
                    mod_key.as_str(),
                    format!(
                        "{mod_key} is a light plugin (.esl) but lists an .esp master: {master}",
                    ),
                );
            }
        }
        for fk in &scan.internal {
            let Some((object_id, source)) = fk.split_once(':') else {
                continue;
            };
            if source != mod_key {
                continue;
            }
            if let Ok(value) = u32::from_str_radix(object_id, 16) {
                if value > 0x000FFF {
                    push_error(
                        &mut errors,
                        "plugin.yaml",
                        0,
                        fk.as_str(),
                        format!(
                            "{fk} exceeds the ESL FormID limit (0x000FFF) — light plugins are capped at 4096 forms",
                        ),
                    );
                }
            }
        }
    }

    for AuthoringRef {
        path,
        line,
        form_key,
    } in &scan.references
    {
        checked += 1;
        let Some((_, source)) = form_key.split_once(':') else {
            continue;
        };
        let rel_path = relpath_from(mod_root.as_deref(), path);

        if source.eq_ignore_ascii_case(&mod_key) {
            if !scan.internal.contains(form_key) {
                push_error(
                    &mut errors,
                    rel_path.as_str(),
                    *line,
                    form_key,
                    format!("internal ref not found in mod (no YAML file defines {form_key})",),
                );
            }
            continue;
        }

        if !masters_set.contains(&source.to_ascii_lowercase()) {
            push_error(
                &mut errors,
                rel_path.as_str(),
                *line,
                form_key,
                format!("references {source} which is not listed as a master in plugin.yaml",),
            );
        }
    }

    Ok((errors, checked))
}

fn push_error(
    errors: &mut Vec<AuthoringErrorPayload>,
    file: &str,
    line: usize,
    formkey: &str,
    reason: String,
) {
    errors.push((file.to_string(), line, formkey.to_string(), reason));
}

fn yaml_root_parent(yaml_dir: &str) -> Option<String> {
    let p = Path::new(yaml_dir);
    p.parent()
        .map(|parent| parent.to_string_lossy().into_owned())
}

fn relpath_from(base: Option<&str>, absolute: &str) -> String {
    let Some(base) = base else {
        return absolute.to_string();
    };
    let base_path = Path::new(base);
    let abs_path = Path::new(absolute);
    match abs_path.strip_prefix(base_path) {
        Ok(rel) => rel.to_string_lossy().into_owned(),
        Err(_) => absolute.to_string(),
    }
}

fn parse_plugin_manifest(yaml_root: &Path) -> PyResult<PluginManifest> {
    let manifest_path = if yaml_root.join("plugin.yaml").is_file() {
        yaml_root.join("plugin.yaml")
    } else if yaml_root.join("plugin.json").is_file() {
        yaml_root.join("plugin.json")
    } else {
        return Err(PyValueError::new_err(format!(
            "no plugin.yaml/plugin.json in {}",
            yaml_root.display()
        )));
    };

    let text = fs::read_to_string(manifest_path.as_path()).map_err(|err| {
        io_error(format!(
            "failed to read '{}': {err}",
            manifest_path.display()
        ))
    })?;
    let format = if manifest_path.extension().and_then(|v| v.to_str()) == Some("json") {
        "json"
    } else {
        "yaml"
    };
    let payload = parse_text_payload_value_native(text.as_str(), format)?;
    let map = payload.as_object().ok_or_else(|| {
        PyValueError::new_err(format!(
            "{} is not a mapping at top level",
            manifest_path.display()
        ))
    })?;

    let mod_key = map
        .get("plugin")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| {
            PyValueError::new_err(format!(
                "{} missing required 'plugin' field",
                manifest_path.display()
            ))
        })?
        .to_string();

    let mut masters = Vec::<String>::new();
    if let Some(header) = map.get("header").and_then(JsonValue::as_object) {
        if let Some(JsonValue::Array(items)) = header.get("masters") {
            for item in items {
                if let Some(s) = item.as_str() {
                    masters.push(s.to_string());
                }
            }
        }
    }

    Ok(PluginManifest { mod_key, masters })
}

fn scan_authoring_dir(
    yaml_root: &Path,
    mod_key: &str,
    internal: &mut BTreeSet<String>,
    references: &mut Vec<AuthoringRef>,
) -> PyResult<()> {
    let mut dirs = vec![yaml_root.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        let mut entries = sorted_directory_entries(dir.as_path())?;
        entries.reverse();
        for entry in entries {
            if entry.is_dir() {
                if let Some(name) = entry.file_name().and_then(|value| value.to_str()) {
                    if let Some(form_key) = parse_formkey_from_name(name) {
                        internal.insert(form_key);
                    }
                }
                dirs.push(entry);
                continue;
            }

            if !is_yaml_record_file(entry.as_path()) {
                continue;
            }
            if let Some(stem) = entry.file_stem().and_then(|value| value.to_str()) {
                if let Some(form_key) = parse_formkey_from_name(stem) {
                    internal.insert(form_key);
                }
            }

            let text = fs::read_to_string(entry.as_path())
                .map_err(|err| io_error(format!("failed to read '{}': {err}", entry.display())))?;
            let parse_text = quote_authoring_id_scalars(text.as_str());
            let payload = parse_text_payload_value_native(parse_text.as_ref(), "yaml")?;
            collect_projected_form_id_definitions(&payload, mod_key, internal);

            let mut canonical = Vec::<String>::new();
            walk_canonical_refs(&payload, &mut canonical);
            for form_key in canonical {
                references.push(AuthoringRef {
                    path: entry.to_string_lossy().into_owned(),
                    line: 0,
                    form_key,
                });
            }

            let mut seen_legacy = HashSet::<String>::new();
            for (idx, line) in text.lines().enumerate() {
                if is_form_id_definition_line(line) {
                    continue;
                }
                for form_key in find_legacy_formkeys(line) {
                    if seen_legacy.insert(form_key.clone()) {
                        references.push(AuthoringRef {
                            path: entry.to_string_lossy().into_owned(),
                            line: idx + 1,
                            form_key,
                        });
                    }
                }
            }
        }
    }
    Ok(())
}

fn sorted_directory_entries(path: &Path) -> PyResult<Vec<PathBuf>> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(path)
        .map_err(|err| io_error(format!("failed to read '{}': {err}", path.display())))?
    {
        entries.push(
            entry
                .map_err(|err| io_error(format!("failed to read '{}': {err}", path.display())))?
                .path(),
        );
    }
    entries.sort_by(|a, b| {
        a.to_string_lossy()
            .to_ascii_lowercase()
            .cmp(&b.to_string_lossy().to_ascii_lowercase())
    });
    Ok(entries)
}

fn is_yaml_record_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    if SKIP_AUTHORING_FILES.contains(&name) {
        return false;
    }
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|ext| ext == "yaml")
}

fn parse_formkey_from_name(name: &str) -> Option<String> {
    let fk_raw = name
        .split_once(" - ")
        .map_or(name, |(_editor_id, fk_raw)| fk_raw);
    let fk_raw = fk_raw.trim();
    if let Some(index) = fk_raw.find('_') {
        if index > 0 {
            let object_id = &fk_raw[..index];
            let plugin = &fk_raw[index + 1..];
            if is_hex_len(object_id, 6) && has_plugin_extension(plugin) {
                return Some(format!("{}:{}", object_id.to_ascii_uppercase(), plugin));
            }
        }
    }
    if let Some((object_id, plugin)) = fk_raw.split_once(':') {
        if is_hex_len(object_id, 6) && has_plugin_extension(plugin) {
            return Some(format!("{}:{}", object_id.to_ascii_uppercase(), plugin));
        }
    }
    None
}

fn walk_canonical_refs(node: &JsonValue, out: &mut Vec<String>) {
    match node {
        JsonValue::Object(map) => {
            if let Some(reference) = map.get("reference") {
                if let Some(form_key) = form_key_from_reference_block(reference) {
                    out.push(form_key);
                }
                return;
            }
            for value in map.values() {
                walk_canonical_refs(value, out);
            }
        }
        JsonValue::Array(items) => {
            for item in items {
                walk_canonical_refs(item, out);
            }
        }
        _ => {}
    }
}

fn form_key_from_reference_block(reference: &JsonValue) -> Option<String> {
    let inner = reference.as_object()?;
    let plugin = inner.get("plugin")?.as_str()?;
    if plugin.is_empty() {
        return None;
    }
    let object_id = inner.get("object_id")?;
    let object_id_text = match object_id {
        JsonValue::String(value) => value.clone(),
        JsonValue::Number(value) => number_to_object_id_text(value),
        JsonValue::Bool(value) => value.to_string(),
        _ => return None,
    };
    if object_id_text.trim_start_matches('0').is_empty() {
        return None;
    }
    Some(format!("{object_id_text}:{plugin}"))
}

fn quote_authoring_id_scalars(text: &str) -> Cow<'_, str> {
    if !text.lines().any(line_needs_authoring_id_quote) {
        return Cow::Borrowed(text);
    }

    let mut normalized = String::with_capacity(text.len() + 16);
    for (idx, line) in text.lines().enumerate() {
        if idx > 0 {
            normalized.push('\n');
        }
        if let Some(line) = quote_authoring_id_scalar_line(line) {
            normalized.push_str(line.as_str());
        } else {
            normalized.push_str(line);
        }
    }
    if text.ends_with('\n') {
        normalized.push('\n');
    }
    Cow::Owned(normalized)
}

fn line_needs_authoring_id_quote(line: &str) -> bool {
    quote_authoring_id_scalar_line(line).is_some()
}

fn quote_authoring_id_scalar_line(line: &str) -> Option<String> {
    let leading_len = line.len() - line.trim_start().len();
    let leading = &line[..leading_len];
    let trimmed = &line[leading_len..];
    let (key, raw_value) = if let Some(raw_value) = trimmed.strip_prefix("object_id:") {
        ("object_id", raw_value)
    } else if let Some(raw_value) = trimmed.strip_prefix("form_id:") {
        ("form_id", raw_value)
    } else {
        return None;
    };

    let raw_value = raw_value.trim_start();
    if raw_value.is_empty()
        || matches!(
            raw_value.as_bytes().first().copied(),
            Some(b'"' | b'\'' | b'{' | b'[' | b'|' | b'>' | b'#')
        )
    {
        return None;
    }

    let (value, comment) = raw_value
        .split_once(" #")
        .map_or((raw_value, None), |(value, comment)| {
            (value.trim_end(), Some(comment))
        });
    if value.is_empty() {
        return None;
    }
    let suffix = comment.map_or(String::new(), |comment| format!(" #{comment}"));
    Some(format!("{leading}{key}: \"{value}\"{suffix}"))
}

fn number_to_object_id_text(value: &serde_json::Number) -> String {
    if let Some(value) = value.as_u64() {
        return format!("{value:06}");
    }
    value.to_string()
}

fn collect_projected_form_id_definitions(
    node: &JsonValue,
    mod_key: &str,
    out: &mut BTreeSet<String>,
) {
    match node {
        JsonValue::Object(map) => {
            if is_projected_record_mapping(map) {
                if let Some(form_key) = normalize_local_form_id(map.get("form_id"), mod_key) {
                    out.insert(form_key);
                }
            }
            for value in map.values() {
                collect_projected_form_id_definitions(value, mod_key, out);
            }
        }
        JsonValue::Array(items) => {
            for item in items {
                collect_projected_form_id_definitions(item, mod_key, out);
            }
        }
        _ => {}
    }
}

fn is_projected_record_mapping(map: &JsonMap<String, JsonValue>) -> bool {
    map.get("signature").and_then(JsonValue::as_str).is_some()
        && map.contains_key("form_id")
        && [
            "subrecords",
            "fields",
            "fields_by_signature",
            "raw_payload_hex",
        ]
        .iter()
        .any(|key| map.contains_key(*key))
}

fn normalize_local_form_id(value: Option<&JsonValue>, mod_key: &str) -> Option<String> {
    let text = match value? {
        JsonValue::String(value) => value.trim().to_string(),
        JsonValue::Number(value) => value.to_string(),
        _ => return None,
    };
    if text.is_empty() {
        return None;
    }
    if let Some((object_id, plugin)) = text.split_once(':') {
        let object_id = object_id.trim();
        if plugin != mod_key || !is_hex_len(object_id, 6) {
            return None;
        }
        return Some(format!("{}:{mod_key}", object_id.to_ascii_uppercase()));
    }
    if is_hex_len(text.as_str(), 6) {
        return Some(format!("{}:{mod_key}", text.to_ascii_uppercase()));
    }
    None
}

fn find_legacy_formkeys(line: &str) -> Vec<String> {
    let bytes = line.as_bytes();
    let mut found = Vec::new();
    let mut colon_idx = 0usize;
    while let Some(offset) = bytes[colon_idx..].iter().position(|value| *value == b':') {
        let idx = colon_idx + offset;
        if idx < 6 {
            colon_idx = idx + 1;
            continue;
        }
        let start = idx - 6;
        if !is_hex_slice(&bytes[start..idx]) {
            colon_idx = idx + 1;
            continue;
        }
        if start > 0 && is_legacy_word_byte(bytes[start - 1]) {
            colon_idx = idx + 1;
            continue;
        }
        let plugin_start = idx + 1;
        let mut plugin_end = plugin_start;
        while plugin_end < bytes.len() && is_plugin_name_byte(bytes[plugin_end]) {
            plugin_end += 1;
        }
        if plugin_end > plugin_start
            && plugin_end + 4 <= bytes.len()
            && &bytes[plugin_end..plugin_end + 3] == b".es"
            && matches!(bytes[plugin_end + 3], b'm' | b'l' | b'p')
        {
            let end = plugin_end + 4;
            if end == bytes.len() || !is_legacy_word_byte(bytes[end]) {
                found.push(line[start..end].to_string());
                colon_idx = end;
                continue;
            }
        }
        colon_idx = idx + 1;
    }
    found
}

fn is_form_id_definition_line(line: &str) -> bool {
    line.trim_start().starts_with("form_id:")
}

fn is_hex_slice(bytes: &[u8]) -> bool {
    bytes.len() == 6 && bytes.iter().all(|value| value.is_ascii_hexdigit())
}

fn is_hex_len(value: &str, len: usize) -> bool {
    value.len() == len && value.as_bytes().iter().all(|byte| byte.is_ascii_hexdigit())
}

fn is_plugin_name_byte(value: u8) -> bool {
    value.is_ascii_alphanumeric() || value == b'_'
}

fn has_plugin_extension(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.ends_with(".esp") || lower.ends_with(".esm") || lower.ends_with(".esl")
}

fn is_legacy_word_byte(value: u8) -> bool {
    value.is_ascii_alphanumeric() || value == b'_'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_formkey_from_editor_id_record_name() {
        assert_eq!(
            parse_formkey_from_name("B21_Record - 000800_B21_Test.esp"),
            Some("000800:B21_Test.esp".to_string())
        );
    }

    #[test]
    fn parse_formkey_from_bare_dialogue_record_name() {
        assert_eq!(
            parse_formkey_from_name("0000B1_B21_PepperShaker.esp"),
            Some("0000B1:B21_PepperShaker.esp".to_string())
        );
    }

    #[test]
    fn parse_formkey_ignores_group_directory_name() {
        assert_eq!(parse_formkey_from_name("0000AF__group_7__AF000003"), None);
    }
}
