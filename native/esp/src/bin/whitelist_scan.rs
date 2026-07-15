use serde_json::{Map as JsonMap, Value as JsonValue};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Default)]
struct RecordTypeScan {
    fields: BTreeSet<String>,
    nested: BTreeMap<String, BTreeSet<String>>,
    canonical_order: Vec<String>,
}

fn append_unique(values: &mut Vec<String>, value: &str) {
    if !values.iter().any(|existing| existing == value) {
        values.push(value.to_string());
    }
}

fn indent_width(line: &str) -> usize {
    line.bytes().take_while(|byte| *byte == b' ').count()
}

fn mapping_key(text: &str) -> Option<&str> {
    let text = text.trim_start();
    let key = text.split_once(':')?.0.trim();
    if key.is_empty() { None } else { Some(key) }
}

fn list_mapping_key(text: &str) -> Option<&str> {
    let item = text.trim_start().strip_prefix("- ")?;
    mapping_key(item)
}

fn scan_record_text(scan: &mut RecordTypeScan, text: &str) {
    let mut in_fields = false;
    let mut fields_indent = 0usize;
    let mut current_field: Option<String> = None;
    let mut current_field_indent = 0usize;
    let mut nested_indent: Option<usize> = None;
    let mut nested_is_list = false;

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let indent = indent_width(line);
        if !in_fields {
            if trimmed == "fields:" {
                in_fields = true;
                fields_indent = indent;
            }
            continue;
        }

        if indent <= fields_indent && !trimmed.starts_with("- ") {
            break;
        }

        if indent == fields_indent {
            if let Some(label) = list_mapping_key(trimmed) {
                let label = label.to_string();
                scan.fields.insert(label.clone());
                append_unique(&mut scan.canonical_order, &label);
                current_field = Some(label);
                current_field_indent = indent;
                nested_indent = None;
                nested_is_list = false;
            }
            continue;
        }

        let Some(current_label) = current_field.as_ref() else {
            continue;
        };
        if indent <= current_field_indent {
            continue;
        }

        let direct_indent = match nested_indent {
            Some(value) => value,
            None => {
                nested_is_list = trimmed.starts_with("- ");
                nested_indent = Some(indent);
                indent
            }
        };
        if indent != direct_indent {
            if nested_is_list && indent == direct_indent + 2 {
                if let Some(key) = mapping_key(trimmed) {
                    scan.nested
                        .entry(current_label.clone())
                        .or_default()
                        .insert(key.to_string());
                }
            }
            continue;
        }

        let key = list_mapping_key(trimmed).or_else(|| mapping_key(trimmed));
        if let Some(key) = key {
            scan.nested
                .entry(current_label.clone())
                .or_default()
                .insert(key.to_string());
        }
    }
}

fn sorted_dirs(path: &Path) -> Result<Vec<PathBuf>, String> {
    let mut dirs = Vec::new();
    for entry in fs::read_dir(path).map_err(|err| format!("{}: {err}", path.display()))? {
        let entry = entry.map_err(|err| format!("{}: {err}", path.display()))?;
        let path = entry.path();
        if path.is_dir() {
            dirs.push(path);
        }
    }
    dirs.sort();
    Ok(dirs)
}

fn sorted_yaml_files(path: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    for entry in fs::read_dir(path).map_err(|err| format!("{}: {err}", path.display()))? {
        let entry = entry.map_err(|err| format!("{}: {err}", path.display()))?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("yaml") {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

fn collect_yaml_files(path: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in fs::read_dir(path).map_err(|err| format!("{}: {err}", path.display()))? {
        let entry = entry.map_err(|err| format!("{}: {err}", path.display()))?;
        let path = entry.path();
        if path.is_dir() {
            collect_yaml_files(&path, files)?;
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("yaml") {
            files.push(path);
        }
    }
    Ok(())
}

fn record_files(authoring_root: &Path) -> Result<Vec<(String, PathBuf)>, String> {
    let mut files = Vec::new();
    for plugin_dir in sorted_dirs(authoring_root)? {
        let records_dir = plugin_dir.join("records");
        if !records_dir.is_dir() {
            continue;
        }
        for record_type_dir in sorted_dirs(&records_dir)? {
            let record_type = record_type_dir
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| format!("invalid record type path: {}", record_type_dir.display()))?
                .to_string();
            let mut record_type_files = sorted_yaml_files(&record_type_dir)?;
            collect_yaml_files(&record_type_dir, &mut record_type_files)?;
            record_type_files.sort();
            record_type_files.dedup();
            for path in record_type_files {
                files.push((record_type.clone(), path));
            }
        }
    }
    Ok(files)
}

fn scan_record_file(scan: &mut RecordTypeScan, path: &Path) -> Result<(), String> {
    let text = fs::read_to_string(path).map_err(|err| format!("{}: {err}", path.display()))?;
    scan_record_text(scan, &text);
    Ok(())
}

fn sorted_array(values: &BTreeSet<String>) -> JsonValue {
    JsonValue::Array(values.iter().cloned().map(JsonValue::String).collect())
}

fn build_whitelist(authoring_root: &Path, game: &str) -> Result<JsonValue, String> {
    let mut scans: BTreeMap<String, RecordTypeScan> = BTreeMap::new();
    for (record_type, path) in record_files(authoring_root)? {
        scan_record_file(scans.entry(record_type).or_default(), &path)?;
    }

    let mut record_types = JsonMap::new();
    let mut nested = JsonMap::new();
    let mut canonical_order = JsonMap::new();

    for (record_type, scan) in scans {
        record_types.insert(record_type.clone(), sorted_array(&scan.fields));
        if !scan.nested.is_empty() {
            let mut nested_fields = JsonMap::new();
            for (field, subfields) in scan.nested {
                nested_fields.insert(field, sorted_array(&subfields));
            }
            nested.insert(record_type.clone(), JsonValue::Object(nested_fields));
        }
        canonical_order.insert(
            record_type,
            JsonValue::Array(
                scan.canonical_order
                    .into_iter()
                    .map(JsonValue::String)
                    .collect(),
            ),
        );
    }

    let mut root = JsonMap::new();
    root.insert("game".to_string(), JsonValue::String(game.to_string()));
    root.insert("record_types".to_string(), JsonValue::Object(record_types));
    root.insert("nested".to_string(), JsonValue::Object(nested));
    root.insert(
        "canonical_order".to_string(),
        JsonValue::Object(canonical_order),
    );
    Ok(JsonValue::Object(root))
}

fn usage() -> &'static str {
    "usage: whitelist_scan --game <game> --authoring-root <path>"
}

fn parse_args() -> Result<(String, PathBuf), String> {
    let mut args = env::args().skip(1);
    let mut game = None;
    let mut authoring_root = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--game" => game = args.next(),
            "--authoring-root" => authoring_root = args.next().map(PathBuf::from),
            "--help" | "-h" => return Err(usage().to_string()),
            _ => return Err(format!("unknown argument {arg:?}\n{}", usage())),
        }
    }

    let game = game.ok_or_else(|| format!("missing --game\n{}", usage()))?;
    let authoring_root =
        authoring_root.ok_or_else(|| format!("missing --authoring-root\n{}", usage()))?;
    if !authoring_root.is_dir() {
        return Err(format!(
            "authoring root not found: {}",
            authoring_root.display()
        ));
    }
    Ok((game, authoring_root))
}

fn main() {
    let (game, authoring_root) = match parse_args() {
        Ok(args) => args,
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(2);
        }
    };
    let whitelist = match build_whitelist(&authoring_root, &game) {
        Ok(value) => value,
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    };
    println!(
        "{}",
        serde_json::to_string(&whitelist).expect("whitelist JSON serialization failed")
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_record_text_collects_fields_and_immediate_nested_keys() {
        let mut scan = RecordTypeScan::default();
        scan_record_text(
            &mut scan,
            &[
                "form_id: 000001",
                "eid: CreatureRace",
                "fields:",
                "- FULL: Creature",
                "- Properties:",
                "  - PropertiesActorValue:",
                "      reference:",
                "        plugin: Fallout4.esm",
                "        object_id: 0002D4",
                "    PropertiesValue: 50.0",
                "- BehaviourGraph: Actors\\Creature\\Behavior.hkx",
            ]
            .join("\n"),
        );

        assert_eq!(
            scan.fields.into_iter().collect::<Vec<_>>(),
            vec!["BehaviourGraph", "FULL", "Properties"]
        );
        assert_eq!(
            scan.nested["Properties"]
                .iter()
                .cloned()
                .collect::<Vec<_>>(),
            vec!["PropertiesActorValue", "PropertiesValue"]
        );
        assert_eq!(
            scan.canonical_order,
            vec!["FULL", "Properties", "BehaviourGraph"]
        );
    }

    #[test]
    fn build_whitelist_scans_authoring_records() {
        let temp = tempfile::tempdir().unwrap();
        let record_dir = temp.path().join("Fallout4").join("records").join("RACE");
        fs::create_dir_all(&record_dir).unwrap();
        fs::write(
            record_dir.join("CreatureRace - 000001_Fallout4.esm.yaml"),
            [
                "form_id: 000001",
                "eid: CreatureRace",
                "fields:",
                "- FULL: Creature",
                "- Properties:",
                "  - PropertiesActorValue:",
                "      reference:",
                "        plugin: Fallout4.esm",
                "        object_id: 0002D4",
                "    PropertiesValue: 50.0",
                "- BehaviourGraph: Actors\\Creature\\Behavior.hkx",
            ]
            .join("\n"),
        )
        .unwrap();

        let whitelist = build_whitelist(temp.path(), "fo4").unwrap();

        assert_eq!(whitelist["game"], "fo4");
        assert_eq!(
            whitelist["record_types"]["RACE"],
            serde_json::json!(["BehaviourGraph", "FULL", "Properties"])
        );
        assert_eq!(
            whitelist["nested"]["RACE"]["Properties"],
            serde_json::json!(["PropertiesActorValue", "PropertiesValue"])
        );
        assert_eq!(
            whitelist["canonical_order"]["RACE"],
            serde_json::json!(["FULL", "Properties", "BehaviourGraph"])
        );
    }
}
