use super::*;
use crate::plugin_runtime::codec_constants::{
    is_known_formid_array_subrecord, is_known_formid_subrecord, is_localized_string_subrecord,
    is_textual_subrecord,
};
use crate::plugin_runtime::condition_functions::{
    CtdaParamKey, infer_game_from_plugins, lookup_ctda_function,
};

pub(crate) fn export_text_payload_value_from_parsed(
    plugin: &ParsedPlugin,
    strings: &LocalizedStringsState,
    mode: &str,
) -> PyResult<JsonValue> {
    if !matches!(mode, "lossless" | "semantic" | "authoring") {
        return Err(value_error(format!("unsupported export mode: {mode}")));
    }

    let mut payload = JsonMap::new();
    payload.insert(
        "plugin".to_string(),
        JsonValue::String(plugin.plugin_name.clone()),
    );
    payload.insert(
        "game".to_string(),
        plugin
            .game
            .as_ref()
            .map(|game| JsonValue::String(game.clone()))
            .unwrap_or(JsonValue::Null),
    );
    payload.insert("mode".to_string(), JsonValue::String(mode.to_string()));
    payload.insert(
        "header_size".to_string(),
        JsonValue::Number(plugin.header_size.into()),
    );
    payload.insert(
        "record_count".to_string(),
        JsonValue::Number(count_records(&plugin.root_items).into()),
    );
    payload.insert(
        "header".to_string(),
        serialize_header_payload_value(plugin, mode)?,
    );
    let mut items = Vec::with_capacity(plugin.root_items.len());
    for item in &plugin.root_items {
        items.push(match item {
            ParsedItem::Group(group) => {
                serialize_group_payload_value(plugin, strings, group, mode)?
            }
            ParsedItem::Record(record) => {
                serialize_record_payload_text_value(plugin, strings, record, mode, false)?
            }
        });
    }
    payload.insert("items".to_string(), JsonValue::Array(items));
    Ok(JsonValue::Object(payload))
}

fn json_number_f64(value: f64, field: &str) -> PyResult<JsonValue> {
    serde_json::Number::from_f64(value)
        .map(JsonValue::Number)
        .ok_or_else(|| {
            value_error(format!(
                "cannot serialize non-finite float value for {field}"
            ))
        })
}

fn serialize_header_flags_authoring_value(flags: u32) -> JsonValue {
    let mut payload = JsonMap::new();
    payload.insert(
        "raw".to_string(),
        JsonValue::String(format!("{:08X}", flags & 0xFFFF_FFFF)),
    );
    let items = HEADER_FLAG_DEFINITIONS
        .iter()
        .map(|(token, value, label)| {
            let mut item = JsonMap::new();
            item.insert("token".to_string(), JsonValue::String((*token).to_string()));
            item.insert("value".to_string(), JsonValue::Number((*value).into()));
            item.insert("label".to_string(), JsonValue::String((*label).to_string()));
            item.insert(
                "enabled".to_string(),
                JsonValue::Bool((flags & *value) != 0),
            );
            JsonValue::Object(item)
        })
        .collect();
    payload.insert("flags".to_string(), JsonValue::Array(items));
    JsonValue::Object(payload)
}

fn serialize_header_flags_semantic_value(flags: u32) -> JsonValue {
    let mut payload = JsonMap::new();
    payload.insert(
        "raw".to_string(),
        JsonValue::String(format!("{:08X}", flags & 0xFFFF_FFFF)),
    );
    payload.insert(
        "master".to_string(),
        JsonValue::Bool((flags & 0x0000_0001) != 0),
    );
    payload.insert(
        "localized".to_string(),
        JsonValue::Bool((flags & 0x0000_0080) != 0),
    );
    payload.insert(
        "light".to_string(),
        JsonValue::Bool((flags & 0x0000_0200) != 0),
    );
    JsonValue::Object(payload)
}

fn serialize_header_payload_value(plugin: &ParsedPlugin, mode: &str) -> PyResult<JsonValue> {
    let header = &plugin.header;
    let mut payload = JsonMap::new();
    payload.insert(
        "version".to_string(),
        json_number_f64(header.version as f64, "header.version")?,
    );
    payload.insert(
        "num_records".to_string(),
        JsonValue::Number(header.num_records.into()),
    );
    payload.insert(
        "next_object_id".to_string(),
        JsonValue::String(format!("{:06X}", header.next_object_id)),
    );
    payload.insert(
        "author".to_string(),
        JsonValue::String(header.author.clone()),
    );
    payload.insert(
        "description".to_string(),
        JsonValue::String(header.description.clone()),
    );
    payload.insert(
        "masters".to_string(),
        JsonValue::Array(
            header
                .masters
                .iter()
                .cloned()
                .map(JsonValue::String)
                .collect(),
        ),
    );
    payload.insert(
        "master_sizes".to_string(),
        JsonValue::Array(
            header
                .master_sizes
                .iter()
                .map(|value| JsonValue::Number((*value).into()))
                .collect(),
        ),
    );
    payload.insert(
        "overridden_forms".to_string(),
        JsonValue::Array(
            header
                .overridden_forms
                .iter()
                .map(|raw| JsonValue::String(format!("{raw:08X}")))
                .collect(),
        ),
    );
    payload.insert(
        "flags".to_string(),
        match mode {
            "authoring" => serialize_header_flags_authoring_value(header.flags),
            "semantic" => serialize_header_flags_semantic_value(header.flags),
            _ => JsonValue::String(format!("{:08X}", header.flags)),
        },
    );
    payload.insert(
        "version_control".to_string(),
        JsonValue::Number(header.version_control.into()),
    );
    payload.insert(
        "extra_subrecords".to_string(),
        JsonValue::Array(
            header
                .extra_subrecords
                .iter()
                .map(|sub| {
                    let mut item = JsonMap::new();
                    item.insert(
                        "signature".to_string(),
                        JsonValue::String(sub.signature.to_string()),
                    );
                    item.insert("size".to_string(), JsonValue::Number(sub.data.len().into()));
                    item.insert(
                        "data_hex".to_string(),
                        JsonValue::String(hex::encode_upper(&sub.data)),
                    );
                    JsonValue::Object(item)
                })
                .collect(),
        ),
    );
    if let Some(form_version) = header.form_version {
        payload.insert(
            "form_version".to_string(),
            JsonValue::Number(form_version.into()),
        );
    }
    if let Some(version2) = header.version2 {
        payload.insert("version2".to_string(), JsonValue::Number(version2.into()));
    }
    Ok(JsonValue::Object(payload))
}

fn semantic_form_ref_value(masters: &[String], plugin_name: &str, raw: u32) -> JsonValue {
    let (plugin_ref, object_id, raw_opt, missing_index) =
        form_ref_from_raw_native(raw, masters, plugin_name);
    let mut payload = JsonMap::new();
    if let Some(plugin_ref) = plugin_ref {
        payload.insert("plugin".to_string(), JsonValue::String(plugin_ref));
    }
    payload.insert(
        "object_id".to_string(),
        JsonValue::String(format!("{object_id:06X}")),
    );
    if let Some(raw) = raw_opt {
        payload.insert("raw".to_string(), JsonValue::String(format!("{raw:08X}")));
    }
    if let Some(missing_index) = missing_index {
        payload.insert(
            "missing_index".to_string(),
            JsonValue::Number(missing_index.into()),
        );
    }
    JsonValue::Object(payload)
}

fn semantic_string_payload_value(
    strings: &LocalizedStringsState,
    plugin_is_localized: bool,
    data: &[u8],
    localized_override: Option<bool>,
) -> PyResult<JsonValue> {
    let use_localized = localized_override.unwrap_or(plugin_is_localized);
    let mut payload = JsonMap::new();
    if use_localized && data.len() == 4 {
        let string_id = read_u32(data, 0)?;
        payload.insert(
            "kind".to_string(),
            JsonValue::String("localized_string".to_string()),
        );
        payload.insert("string_id".to_string(), JsonValue::Number(string_id.into()));
        payload.insert(
            "text".to_string(),
            resolve_localized_string(strings, string_id)
                .cloned()
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        );
        return Ok(JsonValue::Object(payload));
    }
    payload.insert("kind".to_string(), JsonValue::String("string".to_string()));
    payload.insert("text".to_string(), JsonValue::String(decode_cp1252(data)));
    Ok(JsonValue::Object(payload))
}

fn semantic_formid_payload_value(
    masters: &[String],
    plugin_name: &str,
    data: &[u8],
) -> PyResult<Option<JsonValue>> {
    if data.len() != 4 {
        return Ok(None);
    }
    let raw = read_u32(data, 0)?;
    let mut payload = JsonMap::new();
    payload.insert("kind".to_string(), JsonValue::String("formid".to_string()));
    payload.insert("raw".to_string(), JsonValue::String(format!("{raw:08X}")));
    payload.insert(
        "reference".to_string(),
        semantic_form_ref_value(masters, plugin_name, raw),
    );
    Ok(Some(JsonValue::Object(payload)))
}

fn semantic_formid_array_payload_value(
    masters: &[String],
    plugin_name: &str,
    data: &[u8],
) -> PyResult<Option<JsonValue>> {
    if data.is_empty() || data.len() % 4 != 0 {
        return Ok(None);
    }
    let mut values = Vec::with_capacity(data.len() / 4);
    for offset in (0..data.len()).step_by(4) {
        values.push(semantic_form_ref_value(
            masters,
            plugin_name,
            read_u32(data, offset)?,
        ));
    }
    let mut payload = JsonMap::new();
    payload.insert(
        "kind".to_string(),
        JsonValue::String("formid_array".to_string()),
    );
    payload.insert("count".to_string(), JsonValue::Number(values.len().into()));
    payload.insert("values".to_string(), JsonValue::Array(values));
    Ok(Some(JsonValue::Object(payload)))
}

fn semantic_condition_payload_value(
    masters: &[String],
    plugin_name: &str,
    signature: &str,
    data: &[u8],
) -> PyResult<JsonValue> {
    let mut payload = JsonMap::new();
    payload.insert(
        "kind".to_string(),
        JsonValue::String("condition".to_string()),
    );
    payload.insert("size".to_string(), JsonValue::Number(data.len().into()));
    payload.insert(
        "variant".to_string(),
        JsonValue::String(format!("size_{}", data.len())),
    );
    payload.insert(
        "raw_hex".to_string(),
        JsonValue::String(hex::encode_upper(data)),
    );
    payload.insert(
        "signature".to_string(),
        JsonValue::String(signature.to_string()),
    );
    if !data.is_empty() {
        payload.insert(
            "operator_flags".to_string(),
            JsonValue::Number(data[0].into()),
        );
    }
    if data.len() >= 4 {
        payload.insert(
            "operator_padding_hex".to_string(),
            JsonValue::String(hex::encode_upper(&data[1..4])),
        );
    }
    if data.len() >= 8 {
        payload.insert(
            "comparison_value".to_string(),
            json_number_f64(
                f32::from_le_bytes([data[4], data[5], data[6], data[7]]) as f64,
                "condition.comparison_value",
            )?,
        );
    }
    if data.len() >= 10 {
        let function_index = read_u16(data, 8)?;
        payload.insert(
            "function_index".to_string(),
            JsonValue::Number(function_index.into()),
        );
        let game = infer_game_from_plugins(masters.iter(), plugin_name);
        if let Some(meta) = lookup_ctda_function(game, function_index) {
            payload.insert(
                "function_name".to_string(),
                JsonValue::String(meta.name.to_string()),
            );
        }
    }
    if data.len() >= 12 {
        payload.insert(
            "function_padding_hex".to_string(),
            JsonValue::String(hex::encode_upper(&data[10..12])),
        );
    }
    if data.len() >= 16 {
        let raw = read_u32(data, 12)?;
        let mut parameter = JsonMap::new();
        parameter.insert("raw".to_string(), JsonValue::String(format!("{raw:08X}")));
        parameter.insert(
            "reference".to_string(),
            semantic_form_ref_value(masters, plugin_name, raw),
        );
        if data.len() >= 10 {
            let game = infer_game_from_plugins(masters.iter(), plugin_name);
            if let Some(meta) = lookup_ctda_function(game, read_u16(data, 8)?) {
                if let Some(key) = meta.parameter_one_formkey {
                    parameter.insert("type".to_string(), JsonValue::String("formid".to_string()));
                    parameter.insert(
                        "authoring_key".to_string(),
                        JsonValue::String(
                            match key {
                                CtdaParamKey::ParameterOneRecord => "ParameterOneRecord",
                                CtdaParamKey::FirstParameter => "FirstParameter",
                            }
                            .to_string(),
                        ),
                    );
                }
            }
        }
        payload.insert("parameter_1".to_string(), JsonValue::Object(parameter));
    }
    if data.len() >= 20 {
        let raw = read_u32(data, 16)?;
        let mut parameter = JsonMap::new();
        parameter.insert("raw".to_string(), JsonValue::String(format!("{raw:08X}")));
        parameter.insert(
            "reference".to_string(),
            semantic_form_ref_value(masters, plugin_name, raw),
        );
        payload.insert("parameter_2".to_string(), JsonValue::Object(parameter));
    }
    if data.len() > 20 {
        payload.insert(
            "tail_hex".to_string(),
            JsonValue::String(hex::encode_upper(&data[20..])),
        );
        if data.len() >= 24 {
            payload.insert(
                "tail_uint32".to_string(),
                JsonValue::Number(read_u32(data, data.len() - 4)?.into()),
            );
        }
    }
    Ok(JsonValue::Object(payload))
}

fn semantic_conditions_payload_value(
    masters: &[String],
    plugin_name: &str,
    record: &ParsedRecord,
) -> PyResult<Option<JsonValue>> {
    let mut conditions = Vec::new();
    for sub in &record.subrecords {
        if matches!(sub.signature.as_str(), "CTDA" | "CTDT") {
            conditions.push(semantic_condition_payload_value(
                masters,
                plugin_name,
                sub.signature.as_str(),
                &sub.data,
            )?);
        }
    }
    if conditions.is_empty() {
        Ok(None)
    } else {
        Ok(Some(JsonValue::Array(conditions)))
    }
}

fn semantic_vmad_payload_value(sub: &ParsedSubrecord) -> JsonValue {
    super::authoring::authoring_serialize::semantic_vmad_payload_json(&sub.data)
}

fn first_subrecord<'a>(record: &'a ParsedRecord, signature: &str) -> Option<&'a ParsedSubrecord> {
    record
        .subrecords
        .iter()
        .find(|subrecord| subrecord.signature == signature)
}

fn collect_subrecords<'a>(record: &'a ParsedRecord, signature: &str) -> Vec<&'a ParsedSubrecord> {
    record
        .subrecords
        .iter()
        .filter(|subrecord| subrecord.signature == signature)
        .collect()
}

fn semantic_record_text_payload_value(
    strings: &LocalizedStringsState,
    plugin_is_localized: bool,
    record: &ParsedRecord,
    signatures: &[&str],
    localized_override: Option<bool>,
) -> PyResult<Option<JsonValue>> {
    for signature in signatures {
        if let Some(sub) = first_subrecord(record, signature) {
            return Ok(Some(semantic_string_payload_value(
                strings,
                plugin_is_localized,
                &sub.data,
                localized_override,
            )?));
        }
    }
    Ok(None)
}

fn semantic_subrecord_payload_value(
    plugin: &ParsedPlugin,
    strings: &LocalizedStringsState,
    record_is_present: bool,
    sub: &ParsedSubrecord,
) -> PyResult<Option<JsonValue>> {
    let signature = sub.signature.as_str();
    let data = &sub.data;
    let semantic_type = sub.semantic_type.as_deref();
    let masters = plugin.header.masters.as_slice();
    let plugin_name = plugin.plugin_name.as_str();
    let plugin_is_localized = (plugin.header.flags & TES4_FLAG_LOCALIZED) != 0;
    if semantic_type == Some("formid") {
        return semantic_formid_payload_value(masters, plugin_name, data);
    }
    if semantic_type == Some("formid_array") {
        return semantic_formid_array_payload_value(masters, plugin_name, data);
    }
    if signature == "VMAD" {
        return Ok(Some(semantic_vmad_payload_value(sub)));
    }
    if matches!(signature, "CTDA" | "CTDT") {
        return Ok(Some(semantic_condition_payload_value(
            masters,
            plugin_name,
            signature,
            data,
        )?));
    }
    if data.len() == 4 && is_known_formid_subrecord(signature) {
        return semantic_formid_payload_value(masters, plugin_name, data);
    }
    if !data.is_empty() && data.len() % 4 == 0 && is_known_formid_array_subrecord(signature) {
        return semantic_formid_array_payload_value(masters, plugin_name, data);
    }
    if is_localized_string_subrecord(signature) && (data.len() == 4 || looks_like_text(data)) {
        return Ok(Some(semantic_string_payload_value(
            strings,
            plugin_is_localized,
            data,
            None,
        )?));
    }
    if is_textual_subrecord(signature) || looks_like_text(data) {
        let localized = if record_is_present && !is_localized_string_subrecord(signature) {
            Some(false)
        } else {
            None
        };
        return Ok(Some(semantic_string_payload_value(
            strings,
            plugin_is_localized,
            data,
            localized,
        )?));
    }
    Ok(None)
}

fn semantic_record_payload_value(
    plugin: &ParsedPlugin,
    strings: &LocalizedStringsState,
    record: &ParsedRecord,
) -> PyResult<Option<JsonValue>> {
    let masters = plugin.header.masters.as_slice();
    let plugin_name = plugin.plugin_name.as_str();
    let plugin_is_localized = (plugin.header.flags & TES4_FLAG_LOCALIZED) != 0;
    let signature = record.signature.as_str();
    if signature == "DIAL" {
        let mut payload = JsonMap::new();
        payload.insert(
            "kind".to_string(),
            JsonValue::String("dialogue_topic".to_string()),
        );
        if let Some(editor_id) = editor_id_from_parsed(record) {
            payload.insert("editor_id".to_string(), JsonValue::String(editor_id));
        }
        payload.insert(
            "title".to_string(),
            semantic_record_text_payload_value(
                strings,
                plugin_is_localized,
                record,
                &["FULL", "RNAM"],
                None,
            )?
            .unwrap_or(JsonValue::Null),
        );
        if let Some(quest) = first_subrecord(record, "QSTI") {
            if let Some(decoded) = semantic_formid_payload_value(masters, plugin_name, &quest.data)?
            {
                payload.insert("quest".to_string(), decoded);
            }
        }
        return Ok(Some(JsonValue::Object(payload)));
    }
    if signature == "INFO" {
        let mut payload = JsonMap::new();
        payload.insert(
            "kind".to_string(),
            JsonValue::String("dialogue_info".to_string()),
        );
        if let Some(editor_id) = editor_id_from_parsed(record) {
            payload.insert("editor_id".to_string(), JsonValue::String(editor_id));
        }
        payload.insert(
            "prompt".to_string(),
            semantic_record_text_payload_value(
                strings,
                plugin_is_localized,
                record,
                &["RNAM"],
                None,
            )?
            .unwrap_or(JsonValue::Null),
        );
        let mut responses = Vec::new();
        for response in collect_subrecords(record, "NAM1") {
            responses.push(semantic_string_payload_value(
                strings,
                plugin_is_localized,
                &response.data,
                None,
            )?);
        }
        payload.insert("responses".to_string(), JsonValue::Array(responses));
        if let Some(topic) = first_subrecord(record, "TPIC") {
            if let Some(decoded) = semantic_formid_payload_value(masters, plugin_name, &topic.data)?
            {
                payload.insert("topic".to_string(), decoded);
            }
        }
        if let Some(conditions) = semantic_conditions_payload_value(masters, plugin_name, record)? {
            payload.insert("conditions".to_string(), conditions);
        }
        return Ok(Some(JsonValue::Object(payload)));
    }
    if signature == "QUST" {
        let mut payload = JsonMap::new();
        payload.insert("kind".to_string(), JsonValue::String("quest".to_string()));
        if let Some(editor_id) = editor_id_from_parsed(record) {
            payload.insert("editor_id".to_string(), JsonValue::String(editor_id));
        }
        payload.insert(
            "name".to_string(),
            semantic_record_text_payload_value(
                strings,
                plugin_is_localized,
                record,
                &["FULL"],
                None,
            )?
            .unwrap_or(JsonValue::Null),
        );
        let mut stages: Vec<JsonValue> = Vec::new();
        let mut current_stage: Option<JsonMap<String, JsonValue>> = None;
        for sub in &record.subrecords {
            match sub.signature.as_str() {
                "INDX" => {
                    if let Some(stage) = current_stage.take() {
                        stages.push(JsonValue::Object(stage));
                    }
                    let mut stage = JsonMap::new();
                    let index = if sub.data.len() >= 4 {
                        JsonValue::Number(
                            u32::from_le_bytes([
                                sub.data[0],
                                sub.data[1],
                                sub.data[2],
                                sub.data[3],
                            ])
                            .into(),
                        )
                    } else if sub.data.len() >= 2 {
                        JsonValue::Number(u16::from_le_bytes([sub.data[0], sub.data[1]]).into())
                    } else {
                        JsonValue::Null
                    };
                    stage.insert("index".to_string(), index);
                    stage.insert(
                        "raw_hex".to_string(),
                        JsonValue::String(hex::encode_upper(&sub.data)),
                    );
                    stage.insert("log_entries".to_string(), JsonValue::Array(Vec::new()));
                    current_stage = Some(stage);
                }
                "QSDT" => {
                    if let Some(stage) = current_stage.as_mut() {
                        stage.insert(
                            "flags_raw".to_string(),
                            JsonValue::String(hex::encode_upper(&sub.data)),
                        );
                    }
                }
                "CNAM" => {
                    if let Some(stage) = current_stage.as_mut() {
                        if let Some(JsonValue::Array(log_entries)) = stage.get_mut("log_entries") {
                            log_entries.push(semantic_string_payload_value(
                                strings,
                                plugin_is_localized,
                                &sub.data,
                                None,
                            )?);
                        }
                    }
                }
                _ => {}
            }
        }
        if let Some(stage) = current_stage.take() {
            stages.push(JsonValue::Object(stage));
        }
        if !stages.is_empty() {
            payload.insert(
                "stage_count".to_string(),
                JsonValue::Number(stages.len().into()),
            );
            payload.insert("stages".to_string(), JsonValue::Array(stages));
        }
        if let Some(vmad) = first_subrecord(record, "VMAD") {
            payload.insert("vmad".to_string(), semantic_vmad_payload_value(vmad));
        }
        if let Some(conditions) = semantic_conditions_payload_value(masters, plugin_name, record)? {
            payload.insert("conditions".to_string(), conditions);
        }
        return Ok(Some(JsonValue::Object(payload)));
    }

    let mut payload = JsonMap::new();
    if let Some(vmad) = first_subrecord(record, "VMAD") {
        payload.insert("vmad".to_string(), semantic_vmad_payload_value(vmad));
    }
    if let Some(conditions) = semantic_conditions_payload_value(masters, plugin_name, record)? {
        payload.insert("conditions".to_string(), conditions);
    }
    if payload.is_empty() {
        return Ok(None);
    }
    payload.insert("kind".to_string(), JsonValue::String("record".to_string()));
    Ok(Some(JsonValue::Object(payload)))
}

fn serialize_subrecord_lossless_value(
    plugin: &ParsedPlugin,
    strings: &LocalizedStringsState,
    record: Option<&ParsedRecord>,
    subrecord: &ParsedSubrecord,
    mode: &str,
) -> PyResult<JsonValue> {
    let mut payload = JsonMap::new();
    payload.insert(
        "signature".to_string(),
        JsonValue::String(subrecord.signature.to_string()),
    );
    payload.insert(
        "size".to_string(),
        JsonValue::Number(subrecord.data.len().into()),
    );
    payload.insert(
        "data_hex".to_string(),
        JsonValue::String(hex::encode_upper(&subrecord.data)),
    );
    if let Some(semantic_type) = &subrecord.semantic_type {
        payload.insert(
            "semantic_type".to_string(),
            JsonValue::String(semantic_type.clone()),
        );
    }
    if mode == "semantic" {
        if let Some(semantic) =
            semantic_subrecord_payload_value(plugin, strings, record.is_some(), subrecord)?
        {
            payload.insert("semantic".to_string(), semantic);
        }
    }
    Ok(JsonValue::Object(payload))
}

fn canonicalize_union_value_json(
    value: &JsonValue,
    variants: &[SchemaUnionVariantJson],
) -> JsonValue {
    let Some(mapping) = value.as_object() else {
        return value.clone();
    };
    let Some(variant_name) = mapping.get("variant").and_then(|value| value.as_str()) else {
        return value.clone();
    };
    let Some(variant) = variants
        .iter()
        .find(|candidate| candidate.id == variant_name)
    else {
        return value.clone();
    };
    let mut payload = JsonMap::new();
    payload.insert(
        "variant".to_string(),
        JsonValue::String(variant_name.to_string()),
    );
    if let Some(raw_hex) = mapping.get("raw_hex") {
        payload.insert("raw_hex".to_string(), raw_hex.clone());
    }
    if let Some(semantic_type) = mapping.get("semantic_type") {
        payload.insert("semantic_type".to_string(), semantic_type.clone());
    }
    if let Some(inner) = mapping.get("value") {
        let canonical = if let Some(rows) = inner.as_array() {
            JsonValue::Array(
                rows.iter()
                    .map(|row| canonicalize_display_mapping_json(row, &variant.fields))
                    .collect(),
            )
        } else {
            canonicalize_display_mapping_json(inner, &variant.fields)
        };
        payload.insert("value".to_string(), canonical);
    }
    JsonValue::Object(payload)
}

fn canonicalize_display_mapping_json(value: &JsonValue, fields: &[SchemaFieldJson]) -> JsonValue {
    let Some(mapping) = value.as_object() else {
        return value.clone();
    };
    let mut payload = JsonMap::new();
    for field in fields {
        let Some(raw_value) = schema_mapping_value_json(mapping, field) else {
            continue;
        };
        let converted = if !field.union_variants.is_empty() {
            canonicalize_union_value_json(raw_value, &field.union_variants)
        } else if !field.fields.is_empty() {
            canonicalize_display_mapping_json(raw_value, &field.fields)
        } else {
            raw_value.clone()
        };
        payload.insert(field.id.clone(), converted);
    }
    JsonValue::Object(payload)
}

fn native_text_field_payload_value(
    signature: &str,
    compact_payload: &JsonValue,
    spec: Option<&SchemaSubrecordJson>,
) -> JsonValue {
    let mut payload = JsonMap::new();
    payload.insert(
        "signature".to_string(),
        JsonValue::String(signature.to_string()),
    );
    payload.insert(
        "preservation_mode".to_string(),
        JsonValue::String(
            spec.map(|value| native_preservation_mode(value.kind.as_str()))
                .unwrap_or("raw_only")
                .to_string(),
        ),
    );
    payload.insert(
        "codec".to_string(),
        spec.and_then(|spec| spec.codec.clone())
            .map(JsonValue::String)
            .unwrap_or(JsonValue::Null),
    );
    payload.insert(
        "layout".to_string(),
        spec.and_then(native_layout_for_subrecord)
            .map(JsonValue::String)
            .unwrap_or(JsonValue::Null),
    );
    payload.insert(
        "row_label".to_string(),
        spec.and_then(|spec| spec.row_label.clone())
            .map(JsonValue::String)
            .unwrap_or(JsonValue::Null),
    );

    let Some(mapping) = compact_payload.as_object() else {
        payload.insert("value".to_string(), compact_payload.clone());
        return JsonValue::Object(payload);
    };

    if mapping.contains_key("TargetLanguage")
        || mapping.contains_key("Values")
        || mapping.contains_key("Value")
    {
        payload.insert(
            "preservation_mode".to_string(),
            JsonValue::String("hybrid".to_string()),
        );
        let mut localized = JsonMap::new();
        for (key, value) in mapping {
            if key != "raw_hex" && key != "semantic_type" {
                localized.insert(key.clone(), value.clone());
            }
        }
        payload.insert("value".to_string(), JsonValue::Object(localized));
        if let Some(raw_hex) = mapping.get("raw_hex").filter(|value| !value.is_null()) {
            payload.insert("raw_hex".to_string(), raw_hex.clone());
        }
        if let Some(semantic_type) = mapping
            .get("semantic_type")
            .filter(|value| !value.is_null())
        {
            payload.insert("semantic_type".to_string(), semantic_type.clone());
        }
        return JsonValue::Object(payload);
    }

    if signature == "VMAD" {
        if let Some(raw_hex) = mapping.get("raw_hex").filter(|value| !value.is_null()) {
            payload.insert(
                "preservation_mode".to_string(),
                JsonValue::String("hybrid".to_string()),
            );
            payload.insert("layout".to_string(), JsonValue::String("vmad".to_string()));
            payload.insert("raw_hex".to_string(), raw_hex.clone());
            payload.insert("value".to_string(), JsonValue::Object(mapping.clone()));
            if let Some(semantic_type) = mapping
                .get("semantic_type")
                .filter(|value| !value.is_null())
            {
                payload.insert("semantic_type".to_string(), semantic_type.clone());
            }
            return JsonValue::Object(payload);
        }
    }

    let preservation_mode = payload
        .get("preservation_mode")
        .and_then(|value| value.as_str())
        .unwrap_or("raw_only");
    if preservation_mode == "raw_only" {
        if signature == "VMAD" {
            payload.insert(
                "preservation_mode".to_string(),
                JsonValue::String("hybrid".to_string()),
            );
            payload.insert("layout".to_string(), JsonValue::String("vmad".to_string()));
            if let Some(raw_hex) = mapping.get("raw_hex") {
                payload.insert("raw_hex".to_string(), raw_hex.clone());
                let size = raw_hex
                    .as_str()
                    .and_then(|text| hex::decode(text).ok())
                    .map(|raw| raw.len())
                    .unwrap_or(0);
                let mut vmad_value = JsonMap::new();
                vmad_value.insert("kind".to_string(), JsonValue::String("vmad".to_string()));
                vmad_value.insert("size".to_string(), JsonValue::Number(size.into()));
                vmad_value.insert("raw_hex".to_string(), raw_hex.clone());
                payload.insert("value".to_string(), JsonValue::Object(vmad_value));
            } else {
                payload.insert("raw_hex".to_string(), JsonValue::Null);
            }
            if let Some(semantic_type) = mapping
                .get("semantic_type")
                .filter(|value| !value.is_null())
            {
                payload.insert("semantic_type".to_string(), semantic_type.clone());
            }
            return JsonValue::Object(payload);
        }
        if let Some(display_value) = mapping
            .get("display_value")
            .filter(|value| !value.is_null())
        {
            payload.insert("display_value".to_string(), display_value.clone());
        }
        payload.insert(
            "raw_hex".to_string(),
            mapping.get("raw_hex").cloned().unwrap_or(JsonValue::Null),
        );
        if let Some(semantic_type) = mapping
            .get("semantic_type")
            .filter(|value| !value.is_null())
        {
            payload.insert("semantic_type".to_string(), semantic_type.clone());
        }
        return JsonValue::Object(payload);
    }

    if mapping.contains_key("variant") {
        if let Some(value) = mapping.get("variant") {
            payload.insert("variant".to_string(), value.clone());
        }
        if let Some(spec) = spec {
            if !spec.union_variants.is_empty() {
                payload.insert(
                    "variants".to_string(),
                    JsonValue::Array(
                        spec.union_variants
                            .iter()
                            .map(|variant| JsonValue::String(variant.id.clone()))
                            .collect(),
                    ),
                );
            }
        }
        let variant_value = mapping
            .get("fields")
            .or_else(|| mapping.get("rows"))
            .or_else(|| mapping.get("value"))
            .cloned()
            .unwrap_or(JsonValue::Null);
        if mapping.contains_key("fields") {
            payload.insert("fields".to_string(), variant_value.clone());
        } else if mapping.contains_key("rows") {
            payload.insert("rows".to_string(), variant_value.clone());
        }
        let canonical = spec
            .map(|spec| canonicalize_union_value_json(compact_payload, &spec.union_variants))
            .unwrap_or_else(|| variant_value.clone());
        if let Some(value) = canonical.as_object().and_then(|map| map.get("value")) {
            payload.insert("value".to_string(), value.clone());
        } else {
            payload.insert("value".to_string(), canonical);
        }
        if let Some(raw_hex) = mapping.get("raw_hex").filter(|value| !value.is_null()) {
            payload.insert("raw_hex".to_string(), raw_hex.clone());
        }
        if let Some(semantic_type) = mapping
            .get("semantic_type")
            .filter(|value| !value.is_null())
        {
            payload.insert("semantic_type".to_string(), semantic_type.clone());
        }
        return JsonValue::Object(payload);
    }

    let layout = payload.get("layout").and_then(|value| value.as_str());
    if let Some(value) = mapping.get("fields") {
        payload.insert("fields".to_string(), value.clone());
        payload.insert(
            "value".to_string(),
            spec.map(|spec| canonicalize_display_mapping_json(value, &spec.fields))
                .unwrap_or_else(|| value.clone()),
        );
    } else if let Some(value) = mapping.get("rows") {
        payload.insert("rows".to_string(), value.clone());
        payload.insert(
            "value".to_string(),
            spec.map(|spec| {
                JsonValue::Array(
                    value
                        .as_array()
                        .map(|rows| {
                            rows.iter()
                                .map(|row| canonicalize_display_mapping_json(row, &spec.fields))
                                .collect()
                        })
                        .unwrap_or_default(),
                )
            })
            .unwrap_or_else(|| value.clone()),
        );
    } else if let Some(value) = mapping.get("value") {
        match layout {
            Some("mapping") => {
                payload.insert("fields".to_string(), value.clone());
                payload.insert(
                    "value".to_string(),
                    spec.map(|spec| canonicalize_display_mapping_json(value, &spec.fields))
                        .unwrap_or_else(|| value.clone()),
                );
            }
            Some("row_array") => {
                payload.insert("rows".to_string(), value.clone());
                payload.insert(
                    "value".to_string(),
                    spec.map(|spec| {
                        JsonValue::Array(
                            value
                                .as_array()
                                .map(|rows| {
                                    rows.iter()
                                        .map(|row| {
                                            canonicalize_display_mapping_json(row, &spec.fields)
                                        })
                                        .collect()
                                })
                                .unwrap_or_default(),
                        )
                    })
                    .unwrap_or_else(|| value.clone()),
                );
            }
            _ => {
                payload.insert("value".to_string(), value.clone());
            }
        }
    } else if signature == "VMAD" {
        payload.insert(
            "preservation_mode".to_string(),
            JsonValue::String("hybrid".to_string()),
        );
        payload.insert("layout".to_string(), JsonValue::String("vmad".to_string()));
        if let Some(raw_hex) = mapping.get("raw_hex") {
            payload.insert("raw_hex".to_string(), raw_hex.clone());
            let size = raw_hex
                .as_str()
                .and_then(|text| hex::decode(text).ok())
                .map(|raw| raw.len())
                .unwrap_or(0);
            let mut vmad_value = JsonMap::new();
            vmad_value.insert("kind".to_string(), JsonValue::String("vmad".to_string()));
            vmad_value.insert("size".to_string(), JsonValue::Number(size.into()));
            vmad_value.insert("raw_hex".to_string(), raw_hex.clone());
            payload.insert("value".to_string(), JsonValue::Object(vmad_value));
        }
    } else if let Some(spec) = spec {
        if native_layout_for_subrecord(spec).as_deref() == Some("mapping") {
            payload.insert("fields".to_string(), compact_payload.clone());
            payload.insert(
                "value".to_string(),
                canonicalize_display_mapping_json(compact_payload, &spec.fields),
            );
        } else {
            payload.insert("value".to_string(), compact_payload.clone());
        }
    } else {
        payload.insert("value".to_string(), compact_payload.clone());
    }

    if let Some(raw_hex) = mapping.get("raw_hex").filter(|value| !value.is_null()) {
        payload.insert("raw_hex".to_string(), raw_hex.clone());
    }
    if let Some(semantic_type) = mapping
        .get("semantic_type")
        .filter(|value| !value.is_null())
    {
        payload.insert("semantic_type".to_string(), semantic_type.clone());
    }
    JsonValue::Object(payload)
}

pub(crate) fn serialize_record_payload_text_value(
    plugin: &ParsedPlugin,
    strings: &LocalizedStringsState,
    record: &ParsedRecord,
    mode: &str,
    compact_for_authoring_dir: bool,
) -> PyResult<JsonValue> {
    if mode == "authoring" {
        let mut payload = serialize_record_payload_to_json(record, plugin, strings);
        if !compact_for_authoring_dir {
            let Some(map) = payload.as_object_mut() else {
                return Ok(payload);
            };
            let mut full = JsonMap::new();
            full.insert("type".to_string(), JsonValue::String("record".to_string()));
            full.insert(
                "signature".to_string(),
                JsonValue::String(record.signature.to_string()),
            );
            for (key, value) in std::mem::take(map) {
                full.insert(key, value);
            }
            if let Some(editor_id) = editor_id_from_parsed(record).filter(|value| !value.is_empty())
            {
                full.entry("eid".to_string())
                    .or_insert_with(|| JsonValue::String(editor_id));
            }
            if let Some(full_name) = full_name_from_parsed(record).filter(|value| !value.is_empty())
            {
                full.insert("full_name".to_string(), JsonValue::String(full_name));
            }
            let fields = full
                .remove("fields")
                .and_then(|value| value.as_array().cloned())
                .unwrap_or_default();
            let schema_game = schema_game_for_parsed(plugin);
            let schema = schema_game
                .as_deref()
                .and_then(|game| compiled_schema_for_game(game).ok());
            let record_spec = schema.as_ref().and_then(|schema| {
                schema_record_spec(schema.as_ref(), record.signature.as_str()).cloned()
            });
            let mut occurrence_counts: HashMap<&str, usize> = HashMap::new();
            let mut expanded_fields = Vec::with_capacity(record.subrecords.len());
            let mut compact_field_index = 0usize;
            for sub in &record.subrecords {
                let signature = sub.signature.as_str();
                if signature == "EDID" {
                    occurrence_counts.insert(
                        signature,
                        occurrence_counts.get(signature).copied().unwrap_or(0) + 1,
                    );
                    continue;
                }
                let occurrence = *occurrence_counts.get(signature).unwrap_or(&0);
                let spec_json = lookup_subrecord_spec_for_parsed(
                    schema.as_deref(),
                    record_spec.as_ref(),
                    signature,
                    occurrence,
                );
                let compact_payload = fields
                    .get(compact_field_index)
                    .and_then(|entry| entry.as_object())
                    .and_then(|entry| entry.values().next())
                    .cloned()
                    .unwrap_or(JsonValue::Null);
                compact_field_index += 1;
                expanded_fields.push(native_text_field_payload_value(
                    signature,
                    &compact_payload,
                    spec_json.as_ref().map(|(spec, _)| *spec),
                ));
                occurrence_counts.insert(signature, occurrence + 1);
            }
            full.insert("fields".to_string(), JsonValue::Array(expanded_fields));
            return Ok(JsonValue::Object(full));
        }
        return Ok(payload);
    }

    let mut payload = JsonMap::new();
    if !compact_for_authoring_dir {
        payload.insert("type".to_string(), JsonValue::String("record".to_string()));
        payload.insert(
            "signature".to_string(),
            JsonValue::String(record.signature.to_string()),
        );
    }
    payload.insert(
        "form_id".to_string(),
        JsonValue::String(format_record_form_id_native(
            record.form_id,
            plugin.header.masters.as_slice(),
            plugin.plugin_name.as_str(),
        )),
    );
    if !compact_for_authoring_dir || record.flags != 0 {
        payload.insert(
            "flags".to_string(),
            JsonValue::String(format!("{:08X}", record.flags)),
        );
    }
    if !compact_for_authoring_dir || record.version_control != 0 {
        payload.insert(
            "version_control".to_string(),
            JsonValue::Number(record.version_control.into()),
        );
    }
    if let Some(form_version) = record.form_version {
        if !compact_for_authoring_dir || form_version != 0 {
            payload.insert(
                "form_version".to_string(),
                JsonValue::Number(form_version.into()),
            );
        }
    }
    if let Some(version2) = record.version2 {
        if !compact_for_authoring_dir || version2 != 0 {
            payload.insert("version2".to_string(), JsonValue::Number(version2.into()));
        }
    }
    if !compact_for_authoring_dir {
        if let Some(editor_id) = editor_id_from_parsed(record).filter(|value| !value.is_empty()) {
            payload.insert("editor_id".to_string(), JsonValue::String(editor_id));
        }
        if let Some(full_name) = full_name_from_parsed(record).filter(|value| !value.is_empty()) {
            payload.insert("full_name".to_string(), JsonValue::String(full_name));
        }
    }
    if let Some(raw_bytes) = &record.raw_payload {
        payload.insert(
            "raw_payload_hex".to_string(),
            JsonValue::String(hex::encode_upper(raw_bytes)),
        );
    }
    if let Some(parse_error) = &record.parse_error {
        payload.insert(
            "parse_error".to_string(),
            JsonValue::String(parse_error.clone()),
        );
    }

    let mut subrecords = Vec::with_capacity(record.subrecords.len());
    for subrecord in &record.subrecords {
        subrecords.push(serialize_subrecord_lossless_value(
            plugin,
            strings,
            Some(record),
            subrecord,
            mode,
        )?);
    }
    payload.insert("subrecords".to_string(), JsonValue::Array(subrecords));
    if mode == "semantic" {
        if let Some(semantic) = semantic_record_payload_value(plugin, strings, record)? {
            payload.insert("semantic".to_string(), semantic);
        }
    }
    Ok(JsonValue::Object(payload))
}

fn serialize_group_payload_value(
    plugin: &ParsedPlugin,
    strings: &LocalizedStringsState,
    group: &ParsedGroup,
    mode: &str,
) -> PyResult<JsonValue> {
    let mut payload = JsonMap::new();
    payload.insert("type".to_string(), JsonValue::String("group".to_string()));
    payload.insert(
        "group_type".to_string(),
        JsonValue::Number(group.group_type.into()),
    );
    payload.insert(
        "label_hex".to_string(),
        JsonValue::String(hex::encode_upper(group.label)),
    );
    payload.insert(
        "label_text".to_string(),
        group_label_text_native(group)
            .map(JsonValue::String)
            .unwrap_or(JsonValue::Null),
    );
    payload.insert(
        "tail_hex".to_string(),
        JsonValue::String(hex::encode_upper(&group.tail)),
    );
    let mut children = Vec::with_capacity(group.children.len());
    for child in &group.children {
        children.push(match child {
            ParsedItem::Group(child_group) => {
                serialize_group_payload_value(plugin, strings, child_group, mode)?
            }
            ParsedItem::Record(child_record) => {
                serialize_record_payload_text_value(plugin, strings, child_record, mode, false)?
            }
        });
    }
    payload.insert("children".to_string(), JsonValue::Array(children));
    Ok(JsonValue::Object(payload))
}

/// Format a finite f64 to match Python's `repr(float)` output byte-for-byte.
///
/// Rust's `{:?}` f64 formatter (ryu) matches Python's shortest-round-trip
/// representation but diverges on:
/// - Python switches to scientific for abs(value) < 1e-4 (e.g. `1e-5`),
///   while ryu keeps `0.00001` as fixed-point.
/// - Python pads the exponent to at least two digits (`e-06`), while ryu
///   emits the minimum (`e-6`).
///
/// Input must be finite; callers verify with `f64::is_finite`.
fn format_float_python_repr(value: f64) -> String {
    let ryu_str = format!("{value:?}");
    let abs = value.abs();
    let python_wants_scientific = abs != 0.0 && (abs < 1e-4 || abs >= 1e16);
    let ryu_is_scientific = ryu_str.contains('e') || ryu_str.contains('E');
    let normalized = if python_wants_scientific && !ryu_is_scientific {
        // ryu kept fixed-point (e.g. 1e-5 -> "0.00001"); force scientific.
        format!("{value:e}")
    } else {
        ryu_str
    };
    if let Some(e_idx) = normalized.find(|c: char| c == 'e' || c == 'E') {
        let (mantissa_raw, exp_part) = normalized.split_at(e_idx);
        let exp = &exp_part[1..];
        let (sign, digits) = match exp.as_bytes().first() {
            Some(b'+') => ("+", &exp[1..]),
            Some(b'-') => ("-", &exp[1..]),
            _ => ("+", exp),
        };
        let mantissa = if mantissa_raw.contains('.') {
            mantissa_raw.to_string()
        } else {
            format!("{mantissa_raw}.0")
        };
        let digits_padded = if digits.len() < 2 {
            format!("0{digits}")
        } else {
            digits.to_string()
        };
        format!("{mantissa}e{sign}{digits_padded}")
    } else {
        normalized
    }
}

struct PythonFloatFormatter<'a> {
    inner: serde_json::ser::PrettyFormatter<'a>,
}

impl<'a> PythonFloatFormatter<'a> {
    fn new() -> Self {
        Self {
            inner: serde_json::ser::PrettyFormatter::new(),
        }
    }
}

impl<'a> serde_json::ser::Formatter for PythonFloatFormatter<'a> {
    fn write_f32<W: ?Sized + std::io::Write>(
        &mut self,
        writer: &mut W,
        value: f32,
    ) -> std::io::Result<()> {
        let s = format_float_python_repr(value as f64);
        writer.write_all(s.as_bytes())
    }

    fn write_f64<W: ?Sized + std::io::Write>(
        &mut self,
        writer: &mut W,
        value: f64,
    ) -> std::io::Result<()> {
        let s = format_float_python_repr(value);
        writer.write_all(s.as_bytes())
    }

    fn write_null<W: ?Sized + std::io::Write>(&mut self, writer: &mut W) -> std::io::Result<()> {
        self.inner.write_null(writer)
    }
    fn write_bool<W: ?Sized + std::io::Write>(
        &mut self,
        writer: &mut W,
        value: bool,
    ) -> std::io::Result<()> {
        self.inner.write_bool(writer, value)
    }
    fn write_i8<W: ?Sized + std::io::Write>(
        &mut self,
        writer: &mut W,
        value: i8,
    ) -> std::io::Result<()> {
        self.inner.write_i8(writer, value)
    }
    fn write_i16<W: ?Sized + std::io::Write>(
        &mut self,
        writer: &mut W,
        value: i16,
    ) -> std::io::Result<()> {
        self.inner.write_i16(writer, value)
    }
    fn write_i32<W: ?Sized + std::io::Write>(
        &mut self,
        writer: &mut W,
        value: i32,
    ) -> std::io::Result<()> {
        self.inner.write_i32(writer, value)
    }
    fn write_i64<W: ?Sized + std::io::Write>(
        &mut self,
        writer: &mut W,
        value: i64,
    ) -> std::io::Result<()> {
        self.inner.write_i64(writer, value)
    }
    fn write_i128<W: ?Sized + std::io::Write>(
        &mut self,
        writer: &mut W,
        value: i128,
    ) -> std::io::Result<()> {
        self.inner.write_i128(writer, value)
    }
    fn write_u8<W: ?Sized + std::io::Write>(
        &mut self,
        writer: &mut W,
        value: u8,
    ) -> std::io::Result<()> {
        self.inner.write_u8(writer, value)
    }
    fn write_u16<W: ?Sized + std::io::Write>(
        &mut self,
        writer: &mut W,
        value: u16,
    ) -> std::io::Result<()> {
        self.inner.write_u16(writer, value)
    }
    fn write_u32<W: ?Sized + std::io::Write>(
        &mut self,
        writer: &mut W,
        value: u32,
    ) -> std::io::Result<()> {
        self.inner.write_u32(writer, value)
    }
    fn write_u64<W: ?Sized + std::io::Write>(
        &mut self,
        writer: &mut W,
        value: u64,
    ) -> std::io::Result<()> {
        self.inner.write_u64(writer, value)
    }
    fn write_u128<W: ?Sized + std::io::Write>(
        &mut self,
        writer: &mut W,
        value: u128,
    ) -> std::io::Result<()> {
        self.inner.write_u128(writer, value)
    }
    fn write_number_str<W: ?Sized + std::io::Write>(
        &mut self,
        writer: &mut W,
        value: &str,
    ) -> std::io::Result<()> {
        self.inner.write_number_str(writer, value)
    }
    fn begin_string<W: ?Sized + std::io::Write>(&mut self, writer: &mut W) -> std::io::Result<()> {
        self.inner.begin_string(writer)
    }
    fn end_string<W: ?Sized + std::io::Write>(&mut self, writer: &mut W) -> std::io::Result<()> {
        self.inner.end_string(writer)
    }
    fn write_string_fragment<W: ?Sized + std::io::Write>(
        &mut self,
        writer: &mut W,
        fragment: &str,
    ) -> std::io::Result<()> {
        self.inner.write_string_fragment(writer, fragment)
    }
    fn write_char_escape<W: ?Sized + std::io::Write>(
        &mut self,
        writer: &mut W,
        char_escape: serde_json::ser::CharEscape,
    ) -> std::io::Result<()> {
        self.inner.write_char_escape(writer, char_escape)
    }
    fn write_byte_array<W: ?Sized + std::io::Write>(
        &mut self,
        writer: &mut W,
        value: &[u8],
    ) -> std::io::Result<()> {
        self.inner.write_byte_array(writer, value)
    }
    fn begin_array<W: ?Sized + std::io::Write>(&mut self, writer: &mut W) -> std::io::Result<()> {
        self.inner.begin_array(writer)
    }
    fn end_array<W: ?Sized + std::io::Write>(&mut self, writer: &mut W) -> std::io::Result<()> {
        self.inner.end_array(writer)
    }
    fn begin_array_value<W: ?Sized + std::io::Write>(
        &mut self,
        writer: &mut W,
        first: bool,
    ) -> std::io::Result<()> {
        self.inner.begin_array_value(writer, first)
    }
    fn end_array_value<W: ?Sized + std::io::Write>(
        &mut self,
        writer: &mut W,
    ) -> std::io::Result<()> {
        self.inner.end_array_value(writer)
    }
    fn begin_object<W: ?Sized + std::io::Write>(&mut self, writer: &mut W) -> std::io::Result<()> {
        self.inner.begin_object(writer)
    }
    fn end_object<W: ?Sized + std::io::Write>(&mut self, writer: &mut W) -> std::io::Result<()> {
        self.inner.end_object(writer)
    }
    fn begin_object_key<W: ?Sized + std::io::Write>(
        &mut self,
        writer: &mut W,
        first: bool,
    ) -> std::io::Result<()> {
        self.inner.begin_object_key(writer, first)
    }
    fn end_object_key<W: ?Sized + std::io::Write>(
        &mut self,
        writer: &mut W,
    ) -> std::io::Result<()> {
        self.inner.end_object_key(writer)
    }
    fn begin_object_value<W: ?Sized + std::io::Write>(
        &mut self,
        writer: &mut W,
    ) -> std::io::Result<()> {
        self.inner.begin_object_value(writer)
    }
    fn end_object_value<W: ?Sized + std::io::Write>(
        &mut self,
        writer: &mut W,
    ) -> std::io::Result<()> {
        self.inner.end_object_value(writer)
    }
    fn write_raw_fragment<W: ?Sized + std::io::Write>(
        &mut self,
        writer: &mut W,
        fragment: &str,
    ) -> std::io::Result<()> {
        self.inner.write_raw_fragment(writer, fragment)
    }
}

fn serialize_json_python_compat(value: &JsonValue) -> std::io::Result<String> {
    let mut buf: Vec<u8> = Vec::new();
    let formatter = PythonFloatFormatter::new();
    let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);
    serde::Serialize::serialize(value, &mut ser)?;
    String::from_utf8(buf).map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))
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

fn serialize_text_payload_value(value: &JsonValue, format: &str) -> Result<String, String> {
    let format = format.to_string();
    if format.eq_ignore_ascii_case("yaml") || format.eq_ignore_ascii_case("yml") {
        // See authoring_dir::dump_json_value_text_native for why block scalars
        // are disabled (serde-saphyr 0.0.25 emits invalid `|N` indicators).
        let opts = serde_saphyr::ser_options! { prefer_block_scalars: false };
        let mut text = serde_saphyr::to_string_with_options(value, opts)
            .map_err(|err| format!("failed to serialize yaml payload: {err}"))?;
        if let Some(stripped) = text.strip_prefix("---\n") {
            text = stripped.to_string();
        }
        text = quote_yaml_plain_scalars_with_trailing_spaces(text);
        return Ok(text);
    }
    serialize_json_python_compat(value)
        .map_err(|err| format!("failed to serialize json payload: {err}"))
}

pub(crate) fn dump_text_payload_value(value: JsonValue, format: &str) -> PyResult<String> {
    serialize_text_payload_value(&value, format).map_err(value_error)
}

pub(crate) fn detect_text_format(path: &str, explicit: Option<&str>) -> String {
    if let Some(value) = explicit {
        let normalized = value.trim().to_ascii_lowercase();
        if normalized == "yaml" || normalized == "yml" {
            return "yaml".to_string();
        }
        return "json".to_string();
    }
    match Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
    {
        Some(ext) if ext == "yaml" || ext == "yml" => "yaml".to_string(),
        _ => "json".to_string(),
    }
}

pub(crate) fn import_text_payload_lossless_native(
    text: &str,
    format: &str,
) -> PyResult<Option<(ParsedPlugin, LocalizedStringsState)>> {
    let payload = parse_text_payload_value_native(text, format)?;
    let payload = json_object(&payload, "plugin payload")?;
    let plugin_name = payload
        .get("plugin")
        .and_then(|value| value.as_str())
        .unwrap_or("Plugin.esp")
        .to_string();
    let game = payload
        .get("game")
        .and_then(|value| value.as_str())
        .map(|value| value.to_string());
    let header_size = match payload.get("header_size") {
        Some(value) if !value.is_null() => {
            json_parse_int(value, "header_size", MODERN_HEADER_SIZE as i64)? as usize
        }
        _ => match game.as_deref() {
            Some("oblivion" | "fo3" | "fnv") => LEGACY_HEADER_SIZE,
            _ => MODERN_HEADER_SIZE,
        },
    };
    let header_payload = json_object(
        payload
            .get("header")
            .ok_or_else(|| value_error("missing header payload"))?,
        "header",
    )?;
    let header = parse_plugin_header_from_json_native(header_payload)?;
    let mut context =
        NativeImportContext::new(plugin_name.clone(), game.clone(), header_size, header);
    let mut root_items = Vec::new();
    if let Some(items) = payload.get("items") {
        for item in json_array(items, "items")? {
            let item = json_object(item, "items[]")?;
            let item_type = item
                .get("type")
                .and_then(|value| value.as_str())
                .unwrap_or("record")
                .to_ascii_lowercase();
            if item_type == "group" {
                let Some(group) = parse_group_from_json_lossless_native(item, &mut context)? else {
                    return Ok(None);
                };
                root_items.push(ParsedItem::Group(group));
            } else {
                let Some(record) = parse_record_from_json_lossless_native(item, &mut context)?
                else {
                    return Ok(None);
                };
                root_items.push(ParsedItem::Record(record));
            }
        }
    }
    Ok(Some((
        ParsedPlugin {
            plugin_name,
            file_path: String::new(),
            header_size: context.header_size,
            header: context.header,
            root_items,
            game: context.game,
        },
        context.strings,
    )))
}

pub(crate) fn import_text_payload_compact_native(
    text: &str,
    format: &str,
) -> PyResult<(ParsedPlugin, LocalizedStringsState)> {
    let payload_value = parse_text_payload_value_native(text, format)?;
    let payload_mapping = json_object(&payload_value, "plugin payload")?;

    let plugin_name = payload_mapping
        .get("plugin")
        .and_then(|value| value.as_str())
        .unwrap_or("Plugin.esp")
        .to_string();
    let game = payload_mapping
        .get("game")
        .and_then(|value| value.as_str())
        .map(|value| value.to_string());
    let header_size = match payload_mapping.get("header_size") {
        Some(value) if !value.is_null() => {
            json_parse_int(value, "header_size", MODERN_HEADER_SIZE as i64)? as usize
        }
        _ => match game.as_deref() {
            Some("oblivion" | "fo3" | "fnv") => LEGACY_HEADER_SIZE,
            _ => MODERN_HEADER_SIZE,
        },
    };
    let header_payload = json_object(
        payload_mapping
            .get("header")
            .ok_or_else(|| value_error("missing header payload"))?,
        "header",
    )?;
    let header = parse_plugin_header_from_json_native(header_payload)?;
    let mut context =
        NativeImportContext::new(plugin_name.clone(), game.clone(), header_size, header);

    let mut root_items = Vec::new();
    if let Some(items) = payload_mapping.get("items") {
        for item in json_array(items, "items")? {
            let item = json_object(item, "items[]")?;
            let item_type = item
                .get("type")
                .and_then(|value| value.as_str())
                .unwrap_or("record")
                .to_ascii_lowercase();
            let parsed_item = if item_type == "group" {
                ParsedItem::Group(parse_group_from_json_compact_native(item, &mut context)?)
            } else {
                ParsedItem::Record(parse_record_from_json_compact_native(item, &mut context)?)
            };
            root_items.push(parsed_item);
        }
    }
    Ok((
        ParsedPlugin {
            plugin_name,
            file_path: String::new(),
            header_size: context.header_size,
            header: context.header,
            root_items,
            game: context.game,
        },
        context.strings,
    ))
}
