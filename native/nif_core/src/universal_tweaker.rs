use std::path::Path;

use regex::{NoExpand, Regex, RegexBuilder};
use serde_json::{Value, json};

use crate::model::{NifFile, NifValue};
use crate::schema::SCHEMA;

#[derive(Clone, Debug)]
enum PathPart {
    Field(String),
    Index(usize),
    Wildcard,
    Component(usize),
}

#[derive(Clone, Debug)]
struct ValueHandle {
    block_id: usize,
    steps: Vec<PathPart>,
    type_name: Option<String>,
}

#[derive(Clone, Debug)]
enum CursorLocation {
    Block(usize),
    Value(ValueHandle),
}

#[derive(Clone, Debug)]
struct Cursor {
    location: CursorLocation,
    type_name: Option<String>,
}

#[derive(Clone, Debug)]
struct TweakTarget {
    value: ValueHandle,
    context: Cursor,
}

#[derive(Clone, Debug)]
struct TweakOptions {
    blocks: Vec<String>,
    inherited: bool,
    path: Vec<PathPart>,
    value: String,
    value_mode: String,
    old_value_check: bool,
    old_path: Vec<PathPart>,
    old_mode: String,
    old_value: String,
}

impl TweakOptions {
    fn from_json(options: &Value) -> Result<Self, String> {
        let blocks = options
            .get("blocks")
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .flat_map(|value| value.split(','))
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let path_text = options
            .get("field_path")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if path_text.is_empty() {
            return Err("Field path can not be empty".to_string());
        }
        let value_mode = options
            .get("value_mode")
            .and_then(Value::as_str)
            .unwrap_or("set")
            .to_ascii_lowercase();
        let old_value_check = options
            .get("old_value_check")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let old_mode = options
            .get("old_mode")
            .and_then(Value::as_str)
            .unwrap_or("equal")
            .to_ascii_lowercase();
        let value = options
            .get("value")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if matches!(
            value_mode.as_str(),
            "add" | "multiply" | "and" | "and-not" | "or" | "multiply-round"
        ) {
            parse_number(&value).map_err(|_| "Value must be a number".to_string())?;
        }
        if value_mode == "round" && !value.is_empty() {
            parse_number(&value).map_err(|_| "Value must be a number".to_string())?;
        }
        if value_mode == "replace"
            && (!old_value_check
                || !matches!(
                    old_mode.as_str(),
                    "contains" | "starts-with" | "ends-with" | "regex"
                )
                || options
                    .get("old_value")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .is_empty())
        {
            return Err("Replace requires a non-empty old-value check using contains, starts-with, ends-with, or regex".to_string());
        }
        let old_value = options
            .get("old_value")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if old_value_check && matches!(old_mode.as_str(), "greater" | "lesser" | "and" | "and-not")
        {
            parse_number(&old_value)
                .map_err(|_| "Another field's value must be a number".to_string())?;
        }
        if old_value_check && old_mode == "regex" {
            RegexBuilder::new(&old_value)
                .case_insensitive(true)
                .build()
                .map_err(|error| format!("Invalid regular expression: {error}"))?;
        }
        Ok(Self {
            blocks,
            inherited: options
                .get("inherited")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            path: parse_path(path_text)?,
            value,
            value_mode,
            old_value_check,
            old_path: parse_path(
                options
                    .get("old_path")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            )?,
            old_mode,
            old_value,
        })
    }
}

pub fn tweak_nif(nif: &mut NifFile, options: &Value) -> Result<Vec<String>, String> {
    let config = TweakOptions::from_json(options)?;
    add_missing_bsx_flags(nif, &config);
    let mut changes = Vec::new();
    if config.blocks.is_empty()
        || config
            .blocks
            .iter()
            .any(|block| block.eq_ignore_ascii_case("NiHeader"))
    {
        let mut header = nif_header_json(nif);
        let header_changes = tweak_json_value(&mut header, &config)?;
        if !header_changes.is_empty() {
            apply_nif_header_json(nif, &header)?;
            changes.extend(
                header_changes
                    .into_iter()
                    .map(|change| format!("NiHeader\\{change}")),
            );
        }
    }
    if config.blocks.is_empty()
        || config
            .blocks
            .iter()
            .any(|block| block.eq_ignore_ascii_case("NiFooter"))
    {
        let mut footer = json!({
            "Roots": nif.header.footer_roots.iter().map(|root| root.to_string()).collect::<Vec<_>>()
        });
        let footer_changes = tweak_json_value(&mut footer, &config)?;
        if !footer_changes.is_empty() {
            nif.header.footer_roots = footer
                .get("Roots")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|root| root.as_str().and_then(parse_i32))
                .collect();
            changes.extend(
                footer_changes
                    .into_iter()
                    .map(|change| format!("NiFooter\\{change}")),
            );
        }
    }
    let block_ids = selected_blocks(nif, &config);
    let mut targets = Vec::new();
    for block_id in block_ids {
        let cursor = Cursor {
            location: CursorLocation::Block(block_id),
            type_name: Some(nif.blocks[block_id].type_name.clone()),
        };
        resolve_targets(nif, cursor.clone(), &config.path, 0, cursor, &mut targets);
    }

    let regex = if config.old_value_check && config.old_mode == "regex" {
        Some(
            RegexBuilder::new(&config.old_value)
                .case_insensitive(true)
                .build()
                .map_err(|error| error.to_string())?,
        )
    } else {
        None
    };
    for target in targets {
        let Some(current) = read_handle(nif, &target.value) else {
            continue;
        };
        let old_handle = if config.old_path.is_empty() {
            Some(target.value.clone())
        } else {
            let mut old_targets = Vec::new();
            resolve_targets(
                nif,
                target.context.clone(),
                &config.old_path,
                0,
                target.context.clone(),
                &mut old_targets,
            );
            old_targets.into_iter().next().map(|target| target.value)
        };
        let old_checked = old_handle
            .as_ref()
            .and_then(|handle| read_handle(nif, handle));
        if config.old_value_check
            && !old_checked.as_ref().is_some_and(|value| {
                old_matches(
                    value,
                    old_handle
                        .as_ref()
                        .and_then(|handle| handle.type_name.as_deref()),
                    &config,
                    regex.as_ref(),
                )
            })
        {
            continue;
        }
        let current_text = edit_value(&current, target.value.type_name.as_deref());
        let old_text = old_checked
            .as_ref()
            .map(|value| {
                edit_value(
                    value,
                    old_handle
                        .as_ref()
                        .and_then(|handle| handle.type_name.as_deref()),
                )
            })
            .unwrap_or_else(|| current_text.clone());
        let replacement =
            replacement_value(&current, &current_text, &old_text, &config, regex.as_ref())?;
        if current_text == replacement {
            continue;
        }
        set_handle(nif, &target.value, &replacement)?;
        changes.push(format!(
            "{}: Changed from \"{}\" to \"{}\"",
            handle_path(nif, &target.value),
            current_text,
            replacement
        ));
    }
    Ok(changes)
}

fn nif_header_json(nif: &NifFile) -> Value {
    json!({
        "Magic": nif.header.header_string.trim_end_matches(['\r', '\n']),
        "Version": format!("{}.{}.{}.{}", nif.header.version.0, nif.header.version.1, nif.header.version.2, nif.header.version.3),
        "Endian Type": if nif.header.endian_type == 0 { "ENDIAN_BIG" } else { "ENDIAN_LITTLE" },
        "User Version": nif.header.user_version.to_string(),
        "Num Blocks": nif.blocks.len().to_string(),
        "User Version 2": nif.header.bs_version.to_string(),
        "Export Info": {
            "Author": nif.header.creator,
            "Process Script": nif.header.export_info.first().cloned().unwrap_or_default(),
            "Export Script": nif.header.export_info.get(1).cloned().unwrap_or_default(),
            "Max Filepath": nif.header.export_info.get(2).cloned().unwrap_or_default(),
        },
        "Block Types": nif.header.block_type_names,
        "Block Type Index": nif.header.block_type_index.iter().map(|value| value.to_string()).collect::<Vec<_>>(),
        "Block Size": nif.header.block_sizes.iter().map(|value| value.to_string()).collect::<Vec<_>>(),
        "Num Strings": nif.header.strings.len().to_string(),
        "Max String Length": nif.header.max_string_length.to_string(),
        "Strings": nif.header.strings,
        "Num Groups": nif.header.num_groups.to_string(),
        "Groups": nif.header.groups.iter().map(|value| value.to_string()).collect::<Vec<_>>(),
    })
}

fn apply_nif_header_json(nif: &mut NifFile, value: &Value) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| "NiHeader must be an object".to_string())?;
    if let Some(value) = object.get("Magic").and_then(Value::as_str) {
        nif.header.header_string = value.to_string();
    }
    if let Some(version) = object
        .get("Version")
        .and_then(Value::as_str)
        .and_then(parse_version)
    {
        nif.header.version = version;
        nif.header.version_packed = ((version.0 as u32) << 24)
            | ((version.1 as u32) << 16)
            | ((version.2 as u32) << 8)
            | version.3 as u32;
    }
    if let Some(value) = object.get("Endian Type").and_then(Value::as_str) {
        nif.header.endian_type = u8::from(!value.eq_ignore_ascii_case("ENDIAN_BIG"));
    }
    if let Some(value) = object
        .get("User Version")
        .and_then(Value::as_str)
        .and_then(parse_u32)
    {
        nif.header.user_version = value;
    }
    if let Some(value) = object
        .get("User Version 2")
        .and_then(Value::as_str)
        .and_then(parse_u32)
    {
        nif.header.bs_version = value;
    }
    if let Some(export) = object.get("Export Info").and_then(Value::as_object) {
        if let Some(value) = export.get("Author").and_then(Value::as_str) {
            nif.header.creator = value.to_string();
        }
        nif.header.export_info = ["Process Script", "Export Script", "Max Filepath"]
            .into_iter()
            .map(|name| {
                export
                    .get(name)
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string()
            })
            .collect();
    }
    if let Some(values) = json_string_array(object.get("Block Types")) {
        nif.header.block_type_names = values;
    }
    if let Some(values) = json_string_array(object.get("Block Type Index")) {
        nif.header.block_type_index = values
            .iter()
            .map(|value| {
                value
                    .parse()
                    .map_err(|_| format!("Invalid block type index: {value}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
    }
    if let Some(values) = json_string_array(object.get("Block Size")) {
        nif.header.block_sizes = values
            .iter()
            .map(|value| parse_u32(value).ok_or_else(|| format!("Invalid block size: {value}")))
            .collect::<Result<Vec<_>, _>>()?;
    }
    if let Some(values) = json_string_array(object.get("Strings")) {
        nif.header.strings = values;
    }
    if let Some(values) = json_string_array(object.get("Groups")) {
        nif.header.groups = values
            .iter()
            .map(|value| parse_u32(value).ok_or_else(|| format!("Invalid group: {value}")))
            .collect::<Result<Vec<_>, _>>()?;
        nif.header.num_groups = nif.header.groups.len() as u32;
    }
    Ok(())
}

fn json_string_array(value: Option<&Value>) -> Option<Vec<String>> {
    Some(
        value?
            .as_array()?
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
    )
}

fn parse_version(value: &str) -> Option<(u8, u8, u8, u8)> {
    let values = value
        .split('.')
        .map(str::parse)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    Some((
        *values.first()?,
        *values.get(1)?,
        *values.get(2)?,
        *values.get(3)?,
    ))
}

fn parse_u32(value: &str) -> Option<u32> {
    value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .map_or_else(
            || value.parse().ok(),
            |value| u32::from_str_radix(value, 16).ok(),
        )
}

fn parse_i32(value: &str) -> Option<i32> {
    value
        .split_whitespace()
        .next()
        .and_then(|value| value.parse().ok())
}

pub fn tweak_material_file(input: &Path, output: &Path, options: &Value) -> Result<Value, String> {
    let config = TweakOptions::from_json(options)?;
    let extension = input
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let bytes = std::fs::read(input).map_err(|error| error.to_string())?;
    let mut material = match extension.as_str() {
        "bgsm" => serde_json::to_value(
            materials_native::bgsm::parse(&bytes).map_err(|error| error.to_string())?,
        ),
        "bgem" => serde_json::to_value(
            materials_native::bgem::parse(&bytes).map_err(|error| error.to_string())?,
        ),
        _ => return Err(format!("Unsupported material extension: {extension}")),
    }
    .map_err(|error| error.to_string())?;
    let changes = tweak_json_value(&mut material, &config)?;
    let report_only = options
        .get("report_only")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !report_only {
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        if changes.is_empty() && input != output {
            std::fs::copy(input, output).map_err(|error| error.to_string())?;
        } else if !changes.is_empty() {
            let output_bytes = match extension.as_str() {
                "bgsm" => materials_native::bgsm::write(
                    &serde_json::from_value(material).map_err(|error| error.to_string())?,
                ),
                "bgem" => materials_native::bgem::write(
                    &serde_json::from_value(material).map_err(|error| error.to_string())?,
                ),
                _ => unreachable!(),
            };
            std::fs::write(output, output_bytes).map_err(|error| error.to_string())?;
        }
    }
    Ok(json!({
        "processor": "universal-tweaker",
        "path": input,
        "output": output,
        "game": "fo4-material",
        "report_only": report_only,
        "changed": !changes.is_empty(),
        "changes": changes,
    }))
}

pub fn replace_material_assets(
    input: &Path,
    output: &Path,
    options: &Value,
) -> Result<Value, String> {
    let extension = input
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let bytes = std::fs::read(input).map_err(|error| error.to_string())?;
    let mut material = match extension.as_str() {
        "bgsm" => serde_json::to_value(
            materials_native::bgsm::parse(&bytes).map_err(|error| error.to_string())?,
        ),
        "bgem" => serde_json::to_value(
            materials_native::bgem::parse(&bytes).map_err(|error| error.to_string())?,
        ),
        _ => return Err(format!("Unsupported material extension: {extension}")),
    }
    .map_err(|error| error.to_string())?;
    let pairs = material_replacement_pairs(options)?;
    let use_regex = options
        .get("regex")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let case_sensitive = options
        .get("case_sensitive")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let fix_absolute = options
        .get("fix_absolute")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let patterns = pairs
        .iter()
        .map(|(search, replacement)| {
            if search.is_empty() {
                return Ok((None, replacement.as_str()));
            }
            let source = if use_regex {
                search.clone()
            } else {
                regex::escape(search)
            };
            RegexBuilder::new(&source)
                .case_insensitive(!case_sensitive)
                .build()
                .map(|pattern| (Some(pattern), replacement.as_str()))
                .map_err(|error| error.to_string())
        })
        .collect::<Result<Vec<_>, String>>()?;
    let mut changes = Vec::new();
    replace_material_texture_values(
        &mut material,
        &patterns,
        use_regex,
        fix_absolute,
        "",
        &mut changes,
    );
    let report_only = options
        .get("report_only")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !report_only {
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        if changes.is_empty() && input != output {
            std::fs::copy(input, output).map_err(|error| error.to_string())?;
        } else if !changes.is_empty() {
            let output_bytes = match extension.as_str() {
                "bgsm" => materials_native::bgsm::write(
                    &serde_json::from_value(material).map_err(|error| error.to_string())?,
                ),
                "bgem" => materials_native::bgem::write(
                    &serde_json::from_value(material).map_err(|error| error.to_string())?,
                ),
                _ => unreachable!(),
            };
            std::fs::write(output, output_bytes).map_err(|error| error.to_string())?;
        }
    }
    Ok(json!({
        "processor": "replace-assets",
        "path": input,
        "output": output,
        "game": "fo4-material",
        "report_only": report_only,
        "changed": !changes.is_empty(),
        "changes": changes,
    }))
}

fn material_replacement_pairs(options: &Value) -> Result<Vec<(String, String)>, String> {
    if let Some(pairs) = options.get("pairs").and_then(Value::as_array) {
        let pairs = pairs
            .iter()
            .enumerate()
            .map(|(index, pair)| {
                let pair = pair
                    .as_array()
                    .ok_or_else(|| format!("pairs[{index}] must be a two-item array"))?;
                if pair.len() != 2 {
                    return Err(format!(
                        "pairs[{index}] must contain search and replacement"
                    ));
                }
                let search = pair[0]
                    .as_str()
                    .ok_or_else(|| format!("pairs[{index}][0] must be a string"))?;
                let replacement = pair[1]
                    .as_str()
                    .ok_or_else(|| format!("pairs[{index}][1] must be a string"))?;
                Ok((search.to_string(), replacement.to_string()))
            })
            .collect::<Result<Vec<_>, String>>()?;
        if !pairs.is_empty() {
            return Ok(pairs);
        }
    }
    let search = options
        .get("search")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let replacement = options
        .get("replace")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if search.is_empty() && replacement.is_empty() {
        Ok(Vec::new())
    } else {
        Ok(vec![(search.to_string(), replacement.to_string())])
    }
}

fn replace_material_texture_values(
    value: &mut Value,
    patterns: &[(Option<Regex>, &str)],
    use_regex: bool,
    fix_absolute: bool,
    path: &str,
    changes: &mut Vec<String>,
) {
    let Value::Object(fields) = value else {
        return;
    };
    for (name, value) in fields {
        let field_path = if path.is_empty() {
            name.clone()
        } else {
            format!("{path}.{name}")
        };
        if name.ends_with("Texture") {
            let Some(text) = value.as_str() else {
                continue;
            };
            if text.is_empty() {
                continue;
            }
            let original = text.to_string();
            let mut replaced = text.trim().to_string();
            for (pattern, replacement) in patterns {
                if let Some(pattern) = pattern {
                    replaced = if use_regex {
                        pattern.replace_all(&replaced, *replacement).into_owned()
                    } else {
                        pattern
                            .replace_all(&replaced, NoExpand(*replacement))
                            .into_owned()
                    };
                } else {
                    replaced = format!("{replacement}{replaced}");
                }
            }
            if fix_absolute {
                replaced = truncate_material_asset_path(&replaced);
            }
            if replaced != original {
                *value = Value::String(replaced.clone());
                changes.push(format!(
                    "{field_path}: Replaced {original:?} with {replaced:?}"
                ));
            }
        } else {
            replace_material_texture_values(
                value,
                patterns,
                use_regex,
                fix_absolute,
                &field_path,
                changes,
            );
        }
    }
}

fn truncate_material_asset_path(path: &str) -> String {
    if path.as_bytes().get(1) != Some(&b':') {
        return path.to_string();
    }
    let lower = path.to_ascii_lowercase().replace('/', "\\");
    for marker in ["\\data\\", "\\data files\\"] {
        if let Some(index) = lower.find(marker) {
            return path[index + marker.len()..].to_string();
        }
    }
    path.to_string()
}

#[derive(Clone, Debug)]
enum JsonStep {
    Key(String),
    Index(usize),
}

#[derive(Clone, Debug)]
struct JsonTarget {
    value: Vec<JsonStep>,
    context: Vec<JsonStep>,
}

fn tweak_json_value(root: &mut Value, config: &TweakOptions) -> Result<Vec<String>, String> {
    let mut targets = Vec::new();
    resolve_json_targets(root, &[], &config.path, 0, &[], &mut targets);
    let regex = if config.old_value_check && config.old_mode == "regex" {
        Some(
            RegexBuilder::new(&config.old_value)
                .case_insensitive(true)
                .build()
                .map_err(|error| error.to_string())?,
        )
    } else {
        None
    };
    let mut changes = Vec::new();
    for target in targets {
        let Some(current_json) = json_at(root, &target.value).cloned() else {
            continue;
        };
        let Some(current) = json_scalar_to_nif(&current_json) else {
            continue;
        };
        let old_path = if config.old_path.is_empty() {
            Some(target.value.clone())
        } else {
            let mut matches = Vec::new();
            resolve_json_targets(
                root,
                &target.context,
                &config.old_path,
                0,
                &target.context,
                &mut matches,
            );
            matches.into_iter().next().map(|target| target.value)
        };
        let old_checked = old_path
            .as_ref()
            .and_then(|path| json_at(root, path))
            .and_then(json_scalar_to_nif);
        if config.old_value_check
            && !old_checked
                .as_ref()
                .is_some_and(|value| old_matches(value, None, config, regex.as_ref()))
        {
            continue;
        }
        let current_text = json_edit_value(&current_json);
        let old_text = old_checked
            .as_ref()
            .map(|value| edit_value(value, None))
            .unwrap_or_else(|| current_text.clone());
        let replacement =
            replacement_value(&current, &current_text, &old_text, config, regex.as_ref())?;
        if current_text == replacement {
            continue;
        }
        let value = json_at_mut(root, &target.value)
            .ok_or_else(|| "Material field no longer exists".to_string())?;
        set_json_scalar(value, &replacement)?;
        changes.push(format!(
            "{}: Changed from \"{}\" to \"{}\"",
            json_path(&target.value),
            current_text,
            replacement
        ));
    }
    Ok(changes)
}

fn resolve_json_targets(
    root: &Value,
    cursor: &[JsonStep],
    path: &[PathPart],
    index: usize,
    context: &[JsonStep],
    output: &mut Vec<JsonTarget>,
) {
    if index == path.len() {
        output.push(JsonTarget {
            value: cursor.to_vec(),
            context: context.to_vec(),
        });
        return;
    }
    let Some(value) = json_at(root, cursor) else {
        return;
    };
    match &path[index] {
        PathPart::Field(name) => {
            let mut next = cursor.to_vec();
            if let Some(key) = json_matching_key(value, name) {
                next.push(JsonStep::Key(key));
            } else if cursor.is_empty() {
                let Some(header) = value.get("header") else {
                    return;
                };
                let Some(key) = json_matching_key(header, name) else {
                    return;
                };
                next.extend([JsonStep::Key("header".to_string()), JsonStep::Key(key)]);
            } else if let Some(component) = json_component_index(value, name) {
                next.push(JsonStep::Index(component));
            } else {
                return;
            }
            resolve_json_targets(root, &next, path, index + 1, context, output);
        }
        PathPart::Index(array_index) => {
            let Some(values) = value.as_array() else {
                return;
            };
            if values.get(*array_index).is_none() {
                return;
            }
            let mut next = cursor.to_vec();
            next.push(JsonStep::Index(*array_index));
            resolve_json_targets(root, &next, path, index + 1, context, output);
        }
        PathPart::Wildcard => {
            let Some(values) = value.as_array() else {
                return;
            };
            for array_index in 0..values.len() {
                let mut next = cursor.to_vec();
                next.push(JsonStep::Index(array_index));
                resolve_json_targets(root, &next, path, index + 1, &next, output);
            }
        }
        PathPart::Component(_) => {}
    }
}

fn json_matching_key(value: &Value, name: &str) -> Option<String> {
    let object = value.as_object()?;
    let normalized = normalize_name(name);
    object
        .keys()
        .find(|key| normalize_name(key) == normalized)
        .cloned()
}

fn json_component_index(value: &Value, name: &str) -> Option<usize> {
    let values = value.as_array()?;
    let index = match normalize_name(name).as_str() {
        "x" | "r" | "red" => 0,
        "y" | "g" | "green" => 1,
        "z" | "b" | "blue" => 2,
        "w" | "a" | "alpha" => 3,
        _ => return None,
    };
    (index < values.len()).then_some(index)
}

fn normalize_name(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn json_at<'a>(root: &'a Value, path: &[JsonStep]) -> Option<&'a Value> {
    let mut value = root;
    for step in path {
        value = match step {
            JsonStep::Key(key) => value.get(key)?,
            JsonStep::Index(index) => value.get(*index)?,
        };
    }
    Some(value)
}

fn json_at_mut<'a>(root: &'a mut Value, path: &[JsonStep]) -> Option<&'a mut Value> {
    let mut value = root;
    for step in path {
        value = match step {
            JsonStep::Key(key) => value.get_mut(key)?,
            JsonStep::Index(index) => value.get_mut(*index)?,
        };
    }
    Some(value)
}

fn json_scalar_to_nif(value: &Value) -> Option<NifValue> {
    match value {
        Value::Bool(value) => Some(NifValue::Bool(*value)),
        Value::Number(value) if value.is_i64() => Some(NifValue::Int(value.as_i64()?)),
        Value::Number(value) if value.is_u64() => Some(NifValue::UInt(value.as_u64()?)),
        Value::Number(value) => Some(NifValue::Float(value.as_f64()?)),
        Value::String(value) => Some(NifValue::String(value.trim_end_matches('\0').to_string())),
        _ => None,
    }
}

fn json_edit_value(value: &Value) -> String {
    match value {
        Value::String(value) => value.trim_end_matches('\0').to_string(),
        _ => json_scalar_to_nif(value)
            .as_ref()
            .map(|value| edit_value(value, None))
            .unwrap_or_default(),
    }
}

fn set_json_scalar(value: &mut Value, replacement: &str) -> Result<(), String> {
    *value = match value {
        Value::Bool(_) => Value::Bool(match replacement.trim().to_ascii_lowercase().as_str() {
            "true" | "yes" | "1" => true,
            "false" | "no" | "0" => false,
            _ => return Err(format!("Expected a boolean: {replacement}")),
        }),
        Value::Number(number) if number.is_i64() => Value::from(parse_integer(replacement)?),
        Value::Number(number) if number.is_u64() => Value::from(
            u64::try_from(parse_integer(replacement)?)
                .map_err(|_| format!("Expected an unsigned integer: {replacement}"))?,
        ),
        Value::Number(_) => Value::from(parse_number(replacement)?),
        Value::String(_) => Value::String(replacement.to_string()),
        _ => return Err("Universal Tweaker can only assign scalar material fields".to_string()),
    };
    Ok(())
}

fn json_path(path: &[JsonStep]) -> String {
    path.iter()
        .map(|step| match step {
            JsonStep::Key(key) => key.clone(),
            JsonStep::Index(index) => format!("[{index}]"),
        })
        .collect::<Vec<_>>()
        .join("\\")
}

fn selected_blocks(nif: &NifFile, options: &TweakOptions) -> Vec<usize> {
    if options.blocks.len() == 1 && options.blocks[0].contains('\\') {
        return block_by_path(nif, &options.blocks[0]).into_iter().collect();
    }
    nif.blocks
        .iter()
        .filter(|block| {
            options.blocks.is_empty()
                || options.blocks.iter().any(|selected| {
                    block.type_name.eq_ignore_ascii_case(selected)
                        || (options.inherited && SCHEMA.is_subtype_of(&block.type_name, selected))
                })
        })
        .map(|block| block.block_id)
        .collect()
}

fn block_by_path(nif: &NifFile, path: &str) -> Option<usize> {
    let parts = path
        .split('\\')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let first = *parts.first()?;
    let mut current = nif
        .blocks
        .iter()
        .find(|block| block_matches_path_part(block, first))?
        .block_id;
    for part in parts.into_iter().skip(1) {
        current = nif.blocks[current]
            .get_refs(&SCHEMA)
            .into_iter()
            .filter_map(|reference| usize::try_from(reference).ok())
            .filter_map(|block_id| nif.blocks.get(block_id))
            .find(|block| block_matches_path_part(block, part))?
            .block_id;
    }
    Some(current)
}

fn block_matches_path_part(block: &crate::model::NifBlock, part: &str) -> bool {
    block.type_name.eq_ignore_ascii_case(part)
        || block
            .get_field("Name")
            .and_then(value_string)
            .is_some_and(|name| name.eq_ignore_ascii_case(part))
}

fn add_missing_bsx_flags(nif: &mut NifFile, options: &TweakOptions) {
    if !options
        .blocks
        .first()
        .is_some_and(|block| block == "BSXFlags")
        || crate::validation::nif_game_label(nif) == "morrowind"
        || nif.blocks.iter().any(|block| block.type_name == "BSXFlags")
        || nif.blocks.is_empty()
        || !SCHEMA.is_subtype_of(&nif.blocks[0].type_name, "NiNode")
    {
        return;
    }
    let block_id = nif.add_block("BSXFlags", None);
    nif.blocks[block_id].set_field("Name", NifValue::String("BSX".to_string()));
    let root = &mut nif.blocks[0];
    let count = match root.get_field_mut("Extra Data List") {
        Some(NifValue::Array(values)) => {
            values.push(NifValue::Ref(block_id as i32));
            values.len()
        }
        _ => {
            root.set_field(
                "Extra Data List",
                NifValue::Array(vec![NifValue::Ref(block_id as i32)]),
            );
            1
        }
    };
    root.set_field("Num Extra Data List", NifValue::UInt(count as u64));
}

fn parse_path(path: &str) -> Result<Vec<PathPart>, String> {
    let mut result = Vec::new();
    for part in path
        .split(['\\', '/'])
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        if part == "[*]" {
            result.push(PathPart::Wildcard);
        } else if let Some(index) = part
            .strip_prefix('[')
            .and_then(|part| part.strip_suffix(']'))
        {
            result.push(PathPart::Index(
                index
                    .parse()
                    .map_err(|_| format!("Invalid array index: {part}"))?,
            ));
        } else {
            result.push(PathPart::Field(part.to_string()));
        }
    }
    Ok(result)
}

fn resolve_targets(
    nif: &NifFile,
    cursor: Cursor,
    path: &[PathPart],
    index: usize,
    context: Cursor,
    output: &mut Vec<TweakTarget>,
) {
    if index == path.len() {
        if let CursorLocation::Value(value) = cursor.location {
            output.push(TweakTarget { value, context });
        }
        return;
    }
    let Some(cursor) = follow_reference(nif, cursor) else {
        return;
    };
    match &path[index] {
        PathPart::Field(name) => {
            let Some(next) = field_cursor(nif, &cursor, name) else {
                return;
            };
            resolve_targets(nif, next, path, index + 1, context, output);
        }
        PathPart::Index(array_index) => {
            let Some(next) = index_cursor(nif, &cursor, *array_index) else {
                return;
            };
            resolve_targets(nif, next, path, index + 1, context, output);
        }
        PathPart::Wildcard => {
            let Some(length) = cursor_array_len(nif, &cursor) else {
                return;
            };
            for array_index in 0..length {
                let Some(next) = index_cursor(nif, &cursor, array_index) else {
                    continue;
                };
                resolve_targets(nif, next.clone(), path, index + 1, next, output);
            }
        }
        PathPart::Component(_) => {}
    }
}

fn follow_reference(nif: &NifFile, cursor: Cursor) -> Option<Cursor> {
    let CursorLocation::Value(handle) = &cursor.location else {
        return Some(cursor);
    };
    let NifValue::Ref(reference) = read_handle(nif, handle)? else {
        return Some(cursor);
    };
    let block_id = usize::try_from(reference).ok()?;
    let block = nif.blocks.get(block_id)?;
    Some(Cursor {
        location: CursorLocation::Block(block_id),
        type_name: Some(block.type_name.clone()),
    })
}

fn field_cursor(nif: &NifFile, cursor: &Cursor, name: &str) -> Option<Cursor> {
    let field_type = cursor
        .type_name
        .as_deref()
        .and_then(|type_name| schema_field_type(type_name, name));
    match &cursor.location {
        CursorLocation::Block(block_id) => {
            let key = matching_key(&nif.blocks[*block_id].fields, name)?;
            Some(Cursor {
                location: CursorLocation::Value(ValueHandle {
                    block_id: *block_id,
                    steps: vec![PathPart::Field(key)],
                    type_name: field_type.clone(),
                }),
                type_name: field_type,
            })
        }
        CursorLocation::Value(handle) => {
            let value = read_handle(nif, handle)?;
            if let NifValue::Struct(fields) = value {
                let key = matching_key(&fields, name)?;
                let mut next = handle.clone();
                next.steps.push(PathPart::Field(key));
                next.type_name = field_type.clone();
                return Some(Cursor {
                    location: CursorLocation::Value(next),
                    type_name: field_type,
                });
            }
            let component = component_index(value, name)?;
            let mut next = handle.clone();
            next.steps.push(PathPart::Component(component));
            next.type_name = Some("float".to_string());
            Some(Cursor {
                location: CursorLocation::Value(next),
                type_name: Some("float".to_string()),
            })
        }
    }
}

fn index_cursor(nif: &NifFile, cursor: &Cursor, index: usize) -> Option<Cursor> {
    let CursorLocation::Value(handle) = &cursor.location else {
        return None;
    };
    let NifValue::Array(values) = read_handle(nif, handle)? else {
        return None;
    };
    values.get(index)?;
    let mut next = handle.clone();
    next.steps.push(PathPart::Index(index));
    Some(Cursor {
        location: CursorLocation::Value(next),
        type_name: cursor.type_name.clone(),
    })
}

fn cursor_array_len(nif: &NifFile, cursor: &Cursor) -> Option<usize> {
    let CursorLocation::Value(handle) = &cursor.location else {
        return None;
    };
    let NifValue::Array(values) = read_handle(nif, handle)? else {
        return None;
    };
    Some(values.len())
}

fn schema_field_type(type_name: &str, name: &str) -> Option<String> {
    SCHEMA
        .get_all_fields(type_name)
        .into_iter()
        .find(|field| field.name.eq_ignore_ascii_case(name))
        .map(|field| field.type_name.to_string())
}

fn matching_key(fields: &indexmap::IndexMap<String, NifValue>, name: &str) -> Option<String> {
    fields
        .keys()
        .find(|field| bare_name(field).eq_ignore_ascii_case(name))
        .cloned()
}

fn read_handle(nif: &NifFile, handle: &ValueHandle) -> Option<NifValue> {
    let mut value = None;
    for step in &handle.steps {
        value = Some(match step {
            PathPart::Field(name) => match value {
                None => nif.blocks.get(handle.block_id)?.fields.get(name)?.clone(),
                Some(NifValue::Struct(fields)) => fields.get(name)?.clone(),
                _ => return None,
            },
            PathPart::Index(index) => match value {
                Some(NifValue::Array(values)) => values.get(*index)?.clone(),
                _ => return None,
            },
            PathPart::Component(index) => {
                NifValue::Float(vector_component(value.as_ref()?, *index)? as f64)
            }
            PathPart::Wildcard => return None,
        });
    }
    value
}

fn set_handle(nif: &mut NifFile, handle: &ValueHandle, replacement: &str) -> Result<(), String> {
    if matches!(handle.steps.last(), Some(PathPart::Component(_))) {
        return set_component(nif, handle, replacement);
    }
    let path = handle_path(nif, handle);
    let value = value_mut(nif, handle).ok_or_else(|| format!("Field no longer exists: {path}"))?;
    let parsed = parse_edit_value(value, handle.type_name.as_deref(), replacement)?;
    *value = parsed;
    Ok(())
}

fn value_mut<'a>(nif: &'a mut NifFile, handle: &ValueHandle) -> Option<&'a mut NifValue> {
    let (first, rest) = handle.steps.split_first()?;
    let PathPart::Field(name) = first else {
        return None;
    };
    let mut value = nif.blocks.get_mut(handle.block_id)?.fields.get_mut(name)?;
    for step in rest {
        value = match step {
            PathPart::Field(name) => match value {
                NifValue::Struct(fields) => fields.get_mut(name)?,
                _ => return None,
            },
            PathPart::Index(index) => match value {
                NifValue::Array(values) => values.get_mut(*index)?,
                _ => return None,
            },
            PathPart::Component(_) | PathPart::Wildcard => return None,
        };
    }
    Some(value)
}

fn set_component(nif: &mut NifFile, handle: &ValueHandle, replacement: &str) -> Result<(), String> {
    let mut parent = handle.clone();
    let Some(PathPart::Component(index)) = parent.steps.pop() else {
        return Err("Invalid vector component path".to_string());
    };
    let number =
        parse_number(replacement).map_err(|_| format!("Expected a number: {replacement}"))? as f32;
    let value =
        value_mut(nif, &parent).ok_or_else(|| "Vector field no longer exists".to_string())?;
    match value {
        NifValue::Vec3(values) | NifValue::Color3(values) => {
            *values
                .get_mut(index)
                .ok_or_else(|| "Invalid vector component".to_string())? = number;
        }
        NifValue::Vec4(values) | NifValue::Color4(values) | NifValue::Quaternion(values) => {
            *values
                .get_mut(index)
                .ok_or_else(|| "Invalid vector component".to_string())? = number;
        }
        _ => return Err("Field is not a vector".to_string()),
    }
    Ok(())
}

fn component_index(value: NifValue, name: &str) -> Option<usize> {
    let index = match name.to_ascii_lowercase().as_str() {
        "x" | "r" => 0,
        "y" | "g" => 1,
        "z" | "b" => 2,
        "w" | "a" => 3,
        _ => return None,
    };
    match value {
        NifValue::Vec3(_) | NifValue::Color3(_) if index < 3 => Some(index),
        NifValue::Vec4(_) | NifValue::Color4(_) | NifValue::Quaternion(_) => Some(index),
        _ => None,
    }
}

fn vector_component(value: &NifValue, index: usize) -> Option<f32> {
    match value {
        NifValue::Vec3(values) | NifValue::Color3(values) => values.get(index).copied(),
        NifValue::Vec4(values) | NifValue::Color4(values) | NifValue::Quaternion(values) => {
            values.get(index).copied()
        }
        _ => None,
    }
}

fn old_matches(
    value: &NifValue,
    type_name: Option<&str>,
    options: &TweakOptions,
    regex: Option<&Regex>,
) -> bool {
    let edit = edit_value(value, type_name);
    let expected_number = parse_number(&options.old_value).ok();
    match options.old_mode.as_str() {
        "equal" => expected_number.map_or_else(
            || edit.eq_ignore_ascii_case(&options.old_value),
            |expected| native_number(value).is_some_and(|number| number == expected),
        ),
        "not-equal" => expected_number.map_or_else(
            || !edit.eq_ignore_ascii_case(&options.old_value),
            |expected| native_number(value).is_some_and(|number| number != expected),
        ),
        "greater" => native_number(value)
            .zip(expected_number)
            .is_some_and(|(number, expected)| number > expected),
        "lesser" => native_number(value)
            .zip(expected_number)
            .is_some_and(|(number, expected)| number < expected),
        "contains" => contains_case_insensitive(&edit, &options.old_value),
        "doesnt-contain" => !contains_case_insensitive(&edit, &options.old_value),
        "starts-with" => edit
            .get(..options.old_value.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(&options.old_value)),
        "ends-with" => edit
            .get(edit.len().saturating_sub(options.old_value.len())..)
            .is_some_and(|suffix| suffix.eq_ignore_ascii_case(&options.old_value)),
        "and" => native_number(value)
            .zip(expected_number)
            .is_some_and(|(number, expected)| (number as i64 & expected as i64) != 0),
        "and-not" => native_number(value)
            .zip(expected_number)
            .is_some_and(|(number, expected)| (number as i64 & expected as i64) == 0),
        "regex" => regex.is_some_and(|regex| regex.is_match(&edit)),
        _ => false,
    }
}

fn replacement_value(
    current: &NifValue,
    current_text: &str,
    old_text: &str,
    options: &TweakOptions,
    regex: Option<&Regex>,
) -> Result<String, String> {
    let number = native_number(current).unwrap_or(0.0);
    let operand = || parse_number(&options.value).map_err(|_| "Value must be a number".to_string());
    Ok(match options.value_mode.as_str() {
        "set" => options.value.clone(),
        "add" => format_number(number + operand()?),
        "multiply" => format_number(number * operand()?),
        "round" => {
            let multiple = if options.value.is_empty() {
                1.0
            } else {
                operand()?
            };
            if multiple == 0.0 {
                return Err("Round value can not be zero".to_string());
            }
            format_number((number / multiple).round() * multiple)
        }
        "multiply-round" => format_number((number * operand()?).round()),
        "and" => format_number((number as i64 & operand()? as i64) as f64),
        "and-not" => format_number((number as i64 & !(operand()? as i64)) as f64),
        "or" => format_number((number as i64 | operand()? as i64) as f64),
        "replace" => match options.old_mode.as_str() {
            "contains" => {
                replace_case_insensitive(current_text, &options.old_value, &options.value)
            }
            "starts-with" => format!(
                "{}{}",
                options.value,
                &current_text[options.old_value.len()..]
            ),
            "ends-with" => format!(
                "{}{}",
                &current_text[..current_text.len() - options.old_value.len()],
                options.value
            ),
            "regex" => regex
                .ok_or_else(|| "Missing regular expression".to_string())?
                .replace_all(old_text, normalize_regex_replacement(&options.value))
                .into_owned(),
            _ => return Err("Unsupported replacement predicate".to_string()),
        },
        "prepend" => format!("{}{}", options.value, current_text),
        "append" => format!("{}{}", current_text, options.value),
        "remove" => replace_case_insensitive(current_text, &options.value, ""),
        mode => return Err(format!("Unsupported value mode: {mode}")),
    })
}

fn edit_value(value: &NifValue, type_name: Option<&str>) -> String {
    if let Some(type_name) = type_name {
        if let Some(definition) = SCHEMA.get_enum(type_name) {
            if let Some(number) = native_number(value) {
                if let Some(option) = definition
                    .options
                    .iter()
                    .find(|option| option.value == number as i64)
                {
                    return option.name.to_string();
                }
            }
        }
        if let Some(definition) = SCHEMA.get_bitflag(type_name) {
            if let Some(number) = native_number(value) {
                let bits = number as i64;
                let names = definition
                    .options
                    .iter()
                    .filter(|option| option.value != 0 && bits & option.value == option.value)
                    .map(|option| option.name)
                    .collect::<Vec<_>>();
                if !names.is_empty() {
                    return names.join(" | ");
                }
            }
        }
    }
    match value {
        NifValue::Null => String::new(),
        NifValue::Bool(value) => u8::from(*value).to_string(),
        NifValue::Int(value) => value.to_string(),
        NifValue::UInt(value) => value.to_string(),
        NifValue::Float(value) => format_number(*value),
        NifValue::FloatNan(_) => "NaN".to_string(),
        NifValue::String(value) | NifValue::Char(value) => value.clone(),
        NifValue::Ref(value) => value.to_string(),
        NifValue::Vec3(value) | NifValue::Color3(value) => value
            .iter()
            .map(|value| format_number(*value as f64))
            .collect::<Vec<_>>()
            .join(" "),
        NifValue::Vec4(value) | NifValue::Color4(value) | NifValue::Quaternion(value) => value
            .iter()
            .map(|value| format_number(*value as f64))
            .collect::<Vec<_>>()
            .join(" "),
        NifValue::Matrix33(_)
        | NifValue::Matrix44(_)
        | NifValue::Array(_)
        | NifValue::Struct(_) => String::new(),
        NifValue::Bytes(value) => value.iter().map(|byte| format!("{byte:02X}")).collect(),
    }
}

fn parse_edit_value(
    current: &NifValue,
    type_name: Option<&str>,
    replacement: &str,
) -> Result<NifValue, String> {
    if let Some(type_name) = type_name {
        if let Some(definition) = SCHEMA.get_enum(type_name) {
            let value = definition
                .options
                .iter()
                .find(|option| option.name.eq_ignore_ascii_case(replacement.trim()))
                .map(|option| option.value)
                .or_else(|| parse_integer(replacement).ok())
                .ok_or_else(|| format!("Unknown {type_name} value: {replacement}"))?;
            return Ok(numeric_variant(current, value));
        }
        if let Some(definition) = SCHEMA.get_bitflag(type_name) {
            let mut bits = 0i64;
            for part in replacement
                .split('|')
                .map(str::trim)
                .filter(|part| !part.is_empty())
            {
                let value = definition
                    .options
                    .iter()
                    .find(|option| option.name.eq_ignore_ascii_case(part))
                    .map(|option| option.value)
                    .or_else(|| parse_integer(part).ok())
                    .ok_or_else(|| format!("Unknown {type_name} flag: {part}"))?;
                bits |= value;
            }
            return Ok(numeric_variant(current, bits));
        }
    }
    Ok(match current {
        NifValue::Bool(_) => {
            NifValue::Bool(match replacement.trim().to_ascii_lowercase().as_str() {
                "true" | "yes" | "1" => true,
                "false" | "no" | "0" => false,
                _ => return Err(format!("Expected a boolean: {replacement}")),
            })
        }
        NifValue::Int(_) => NifValue::Int(parse_integer(replacement)?),
        NifValue::UInt(_) => NifValue::UInt(
            u64::try_from(parse_integer(replacement)?)
                .map_err(|_| format!("Expected an unsigned integer: {replacement}"))?,
        ),
        NifValue::Float(_) | NifValue::FloatNan(_) => NifValue::Float(parse_number(replacement)?),
        NifValue::String(_) => NifValue::String(replacement.to_string()),
        NifValue::Char(_) => NifValue::Char(replacement.to_string()),
        NifValue::Ref(_) => NifValue::Ref(
            i32::try_from(parse_integer(replacement)?)
                .map_err(|_| format!("Reference is out of range: {replacement}"))?,
        ),
        _ => return Err("Universal Tweaker can only assign scalar fields".to_string()),
    })
}

fn numeric_variant(current: &NifValue, value: i64) -> NifValue {
    match current {
        NifValue::Int(_) => NifValue::Int(value),
        _ => NifValue::UInt(value as u64),
    }
}

fn native_number(value: &NifValue) -> Option<f64> {
    match value {
        NifValue::Bool(value) => Some(u8::from(*value) as f64),
        NifValue::Int(value) => Some(*value as f64),
        NifValue::UInt(value) => Some(*value as f64),
        NifValue::Float(value) => Some(*value),
        NifValue::Ref(value) => Some(*value as f64),
        _ => None,
    }
}

fn parse_number(value: &str) -> Result<f64, String> {
    value
        .trim()
        .parse()
        .map_err(|_| format!("Expected a number: {value}"))
}

fn parse_integer(value: &str) -> Result<i64, String> {
    let value = value.trim();
    if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        i64::from_str_radix(hex, 16).map_err(|_| format!("Expected an integer: {value}"))
    } else {
        value
            .parse()
            .map_err(|_| format!("Expected an integer: {value}"))
    }
}

fn format_number(value: f64) -> String {
    if value.fract() == 0.0 && value >= i64::MIN as f64 && value <= i64::MAX as f64 {
        (value as i64).to_string()
    } else {
        value.to_string()
    }
}

fn contains_case_insensitive(value: &str, pattern: &str) -> bool {
    value.to_lowercase().contains(&pattern.to_lowercase())
}

fn replace_case_insensitive(value: &str, pattern: &str, replacement: &str) -> String {
    if pattern.is_empty() {
        return value.to_string();
    }
    RegexBuilder::new(&regex::escape(pattern))
        .case_insensitive(true)
        .build()
        .map(|regex| regex.replace_all(value, NoExpand(replacement)).into_owned())
        .unwrap_or_else(|_| value.to_string())
}

fn normalize_regex_replacement(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut characters = value.chars().peekable();
    while let Some(character) = characters.next() {
        if character != '$' || !characters.peek().is_some_and(char::is_ascii_digit) {
            result.push(character);
            continue;
        }
        result.push_str("${");
        while characters.peek().is_some_and(char::is_ascii_digit) {
            result.push(characters.next().unwrap());
        }
        result.push('}');
    }
    result
}

fn handle_path(nif: &NifFile, handle: &ValueHandle) -> String {
    let mut path = format!(
        "{} [{}]",
        nif.blocks[handle.block_id].type_name, handle.block_id
    );
    for step in &handle.steps {
        match step {
            PathPart::Field(name) => {
                path.push('\\');
                path.push_str(bare_name(name));
            }
            PathPart::Index(index) => path.push_str(&format!("\\[{index}]")),
            PathPart::Component(index) => {
                path.push('\\');
                path.push_str(["X", "Y", "Z", "W"].get(*index).copied().unwrap_or("?"));
            }
            PathPart::Wildcard => {}
        }
    }
    path
}

fn value_string(value: &NifValue) -> Option<&str> {
    match value {
        NifValue::String(value) | NifValue::Char(value) => Some(value),
        _ => None,
    }
}

fn bare_name(name: &str) -> &str {
    name.split_once(':').map(|(name, _)| name).unwrap_or(name)
}

#[cfg(test)]
mod tests {
    use indexmap::IndexMap;
    use serde_json::json;

    use super::*;
    use crate::model::NifBlock;

    #[test]
    fn tweaks_wildcard_entries_using_relative_old_path() {
        let mut nif = NifFile::new("fnv");
        let mut sequence = NifBlock::new(1, "NiControllerSequence");
        sequence.set_field(
            "Controlled Blocks",
            NifValue::Array(vec![
                entry("Bip01 Neck", 3),
                entry("Bip01 Hand", 4),
                entry("Bip01 Head", 5),
            ]),
        );
        nif.blocks.push(sequence);

        let changes = tweak_nif(
            &mut nif,
            &json!({
                "blocks": ["NiControllerSequence"],
                "field_path": "Controlled Blocks\\[*]\\Priority",
                "value_mode": "set",
                "value": "10",
                "old_value_check": true,
                "old_path": "Node Name",
                "old_mode": "regex",
                "old_value": "Neck|Head"
            }),
        )
        .unwrap();

        assert_eq!(changes.len(), 2);
        let NifValue::Array(entries) = nif.blocks[1].get_field("Controlled Blocks").unwrap() else {
            panic!("expected controlled blocks");
        };
        assert_eq!(entry_priority(&entries[0]), 10);
        assert_eq!(entry_priority(&entries[1]), 4);
        assert_eq!(entry_priority(&entries[2]), 10);
    }

    #[test]
    fn follows_references_and_resolves_symbolic_enums() {
        let mut nif = NifFile::new("skyrimse");
        let mut shader = NifBlock::new(1, "BSLightingShaderProperty");
        shader.set_field("Shader Type", NifValue::UInt(0));
        shader.set_field("Texture Set", NifValue::Ref(2));
        let mut textures = NifBlock::new(2, "BSShaderTextureSet");
        textures.set_field(
            "Textures",
            NifValue::Array(vec![
                NifValue::String("textures\\test_d.dds".to_string()),
                NifValue::String("textures\\test_n.dds".to_string()),
                NifValue::String(String::new()),
                NifValue::String("textures\\test_p.dds".to_string()),
            ]),
        );
        nif.blocks.extend([shader, textures]);

        let changes = tweak_nif(
            &mut nif,
            &json!({
                "blocks": ["BSLightingShaderProperty"],
                "field_path": "Shader Type",
                "value_mode": "set",
                "value": "Parallax",
                "old_value_check": true,
                "old_path": "Texture Set\\Textures\\[3]",
                "old_mode": "contains",
                "old_value": ".dds"
            }),
        )
        .unwrap();

        assert_eq!(changes.len(), 1);
        assert_ne!(
            nif.blocks[1].get_field("Shader Type"),
            Some(&NifValue::UInt(0))
        );
    }

    #[test]
    fn regex_replace_can_derive_target_from_another_field() {
        let mut nif = NifFile::new("skyrimse");
        nif.convert_block_type(0, "BSShaderTextureSet").unwrap();
        nif.blocks[0].set_field(
            "Textures",
            NifValue::Array(vec![
                NifValue::String("textures\\armor\\test.dds".to_string()),
                NifValue::String(String::new()),
            ]),
        );
        tweak_nif(
            &mut nif,
            &json!({
                "blocks": ["BSShaderTextureSet"],
                "field_path": "Textures\\[1]",
                "value_mode": "replace",
                "value": "$1_n.dds",
                "old_value_check": true,
                "old_path": "Textures\\[0]",
                "old_mode": "regex",
                "old_value": "(.+)\\.dds"
            }),
        )
        .unwrap();
        let NifValue::Array(textures) = nif.blocks[0].get_field("Textures").unwrap() else {
            panic!("expected textures");
        };
        assert_eq!(
            value_string(&textures[1]),
            Some("textures\\armor\\test_n.dds")
        );
    }

    #[test]
    fn tweaks_nif_header_export_info() {
        let mut nif = NifFile::new("fo4");
        nif.header.creator = "Old Tool".to_string();
        let changes = tweak_nif(
            &mut nif,
            &json!({
                "blocks": ["NiHeader"],
                "field_path": "Export Info\\Author",
                "value_mode": "set",
                "value": "Modkit"
            }),
        )
        .unwrap();

        assert_eq!(nif.header.creator, "Modkit");
        assert_eq!(changes.len(), 1);
    }

    #[test]
    fn adding_missing_bsx_flags_creates_the_root_extra_data_array() {
        let mut nif = NifFile::new("oblivion");
        nif.blocks[0].fields.shift_remove("Extra Data List");
        let changes = tweak_nif(
            &mut nif,
            &json!({
                "blocks": ["BSXFlags"],
                "field_path": "Integer Data",
                "value_mode": "set",
                "value": "3"
            }),
        )
        .unwrap();

        assert_eq!(changes.len(), 1);
        let bsx = nif
            .blocks
            .iter()
            .find(|block| block.type_name == "BSXFlags")
            .unwrap();
        assert_eq!(bsx.get_field("Integer Data"), Some(&NifValue::UInt(3)));
        assert_eq!(
            nif.blocks[0].get_field("Extra Data List"),
            Some(&NifValue::Array(vec![NifValue::Ref(bsx.block_id as i32)]))
        );
    }

    #[test]
    fn universal_tweaker_does_not_add_bsx_flags_to_morrowind() {
        let mut nif = NifFile::new("morrowind");
        let changes = tweak_nif(
            &mut nif,
            &json!({
                "blocks": ["BSXFlags"],
                "field_path": "Integer Data",
                "value_mode": "set",
                "value": "3"
            }),
        )
        .unwrap();

        assert!(changes.is_empty());
        assert!(nif.blocks.iter().all(|block| block.type_name != "BSXFlags"));
    }

    #[test]
    fn tweaks_and_rewrites_bgsm_fields() {
        let directory = tempfile::tempdir().unwrap();
        let input = directory.path().join("input.bgsm");
        let output = directory.path().join("output.bgsm");
        std::fs::write(
            &input,
            include_bytes!("../../materials/tests/fixtures/default001solid.bgsm"),
        )
        .unwrap();
        let result = tweak_material_file(
            &input,
            &output,
            &json!({
                "field_path": "Diffuse Texture",
                "value_mode": "set",
                "value": "Textures\\test_d.dds"
            }),
        )
        .unwrap();

        assert_eq!(result["changed"], true);
        let material = materials_native::bgsm::parse(&std::fs::read(output).unwrap()).unwrap();
        assert_eq!(
            material.DiffuseTexture.trim_end_matches('\0'),
            "Textures\\test_d.dds"
        );
    }

    #[test]
    fn replace_assets_rewrites_bgsm_texture_fields() {
        let directory = tempfile::tempdir().unwrap();
        let input = directory.path().join("input.bgsm");
        let output = directory.path().join("output.bgsm");
        std::fs::write(
            &input,
            include_bytes!("../../materials/tests/fixtures/default001solid.bgsm"),
        )
        .unwrap();
        let result =
            replace_material_assets(&input, &output, &json!({"pairs": [["", "converted\\"]]}))
                .unwrap();

        assert_eq!(result["changed"], true);
        let material = materials_native::bgsm::parse(&std::fs::read(output).unwrap()).unwrap();
        assert!(
            material
                .DiffuseTexture
                .trim_end_matches('\0')
                .starts_with("converted\\")
        );
    }

    fn entry(name: &str, priority: u64) -> NifValue {
        NifValue::Struct(IndexMap::from([
            ("Node Name".to_string(), NifValue::String(name.to_string())),
            ("Priority".to_string(), NifValue::UInt(priority)),
        ]))
    }

    fn entry_priority(value: &NifValue) -> u64 {
        let NifValue::Struct(fields) = value else {
            return 0;
        };
        let Some(NifValue::UInt(value)) = fields.get("Priority") else {
            return 0;
        };
        *value
    }
}
