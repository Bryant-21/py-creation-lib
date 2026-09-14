use super::*;

fn visit_vmad_values(value: &mut JsonValue, rewrite: &mut dyn FnMut(u32) -> Option<u32>) -> usize {
    match value {
        JsonValue::Array(values) => values
            .iter_mut()
            .map(|value| visit_vmad_values(value, rewrite))
            .sum(),
        JsonValue::Object(object) => {
            let mut changed = 0;
            if let Some(value) = object.get_mut("FormID") {
                if let Some(raw) = parse_vmad_formid(Some(value), &[], "") {
                    if let Some(replacement) =
                        rewrite(raw).filter(|replacement| *replacement != raw)
                    {
                        *value = serde_json::json!({"raw": format!("{replacement:08X}")});
                        changed += 1;
                    }
                }
            }
            for (key, value) in object {
                if key != "FormID" {
                    changed += visit_vmad_values(value, rewrite);
                }
            }
            changed
        }
        _ => 0,
    }
}

fn rewrite_vmad(
    data: &[u8],
    signature: &str,
    rewrite: &mut dyn FnMut(u32) -> Option<u32>,
) -> Result<Option<Vec<u8>>, String> {
    let mut payload =
        authoring::authoring_serialize::compact_vmad_payload_json(data, &[], "", Some(signature))
            .ok_or_else(|| format!("Cannot edit masters: undecodable {signature} VMAD"))?;
    if payload.get("raw_hex").is_some() || payload.get("tail_hex").is_some() {
        return Err(format!(
            "Cannot edit masters: incomplete {signature} VMAD reference coverage"
        ));
    }
    if visit_vmad_values(&mut payload, rewrite) == 0 {
        return Ok(None);
    }
    build_vmad_bytes_from_payload(&payload, &[], "")
        .map(Some)
        .ok_or_else(|| format!("Cannot encode remapped {signature} VMAD"))
}

fn rewrite_omod(
    data: &[u8],
    rewrite: &mut dyn FnMut(u32) -> Option<u32>,
) -> Result<Option<Vec<u8>>, String> {
    let invalid = || "Cannot edit masters: unsupported OMOD DATA layout".to_string();
    let read = |offset: usize| -> Result<usize, String> {
        let bytes: [u8; 4] = data
            .get(offset..offset + 4)
            .ok_or_else(invalid)?
            .try_into()
            .unwrap();
        Ok(u32::from_le_bytes(bytes) as usize)
    };
    let includes = read(0)?;
    let properties = read(4)?;
    let slots = read(20)?;
    let include_start = 28 + slots * 4;
    let property_start = include_start + includes * 7;
    if property_start + properties * 24 != data.len() {
        return Err(invalid());
    }
    let mut offsets = vec![16];
    offsets.extend((0..slots).map(|index| 24 + index * 4));
    offsets.extend((0..includes).map(|index| include_start + index * 7));
    for index in 0..properties {
        let offset = property_start + index * 24;
        match data[offset] {
            4 | 6 => offsets.push(offset + 12),
            0 | 1 | 2 | 5 => {}
            _ => return Err(invalid()),
        }
    }
    let mut result = None;
    for offset in offsets {
        let raw = read(offset)? as u32;
        if let Some(replacement) = rewrite(raw).filter(|value| *value != raw) {
            let bytes = result.get_or_insert_with(|| data.to_vec());
            bytes[offset..offset + 4].copy_from_slice(&replacement.to_le_bytes());
        }
    }
    Ok(result)
}

fn rewrite_items(
    items: &mut [ParsedItem],
    schema: Option<&CompiledSchema>,
    headers: bool,
    rewrite: &mut dyn FnMut(u32) -> Option<u32>,
) -> Result<(), String> {
    for item in items {
        match item {
            ParsedItem::Group(group) => {
                if headers && matches!(group.group_type, 1 | 6 | 7 | 8 | 9 | 10) {
                    if let Some(replacement) = rewrite(u32::from_le_bytes(group.label)) {
                        group.label = replacement.to_le_bytes();
                    }
                }
                rewrite_items(&mut group.children, schema, headers, rewrite)?;
            }
            ParsedItem::Record(record) => {
                if record.parse_error.is_some() {
                    return Err(format!(
                        "Cannot edit masters: {} {:08X} has a parse error",
                        record.signature, record.form_id
                    ));
                }
                if headers && record.signature != "NAVI" {
                    if let Some(replacement) = rewrite(record.form_id) {
                        record.form_id = replacement;
                    }
                }
                let mut changed = false;
                for subrecord in &mut record.subrecords {
                    if subrecord.signature == "VMAD" {
                        if let Some(bytes) =
                            rewrite_vmad(&subrecord.data, record.signature.as_str(), rewrite)?
                        {
                            subrecord.data = Bytes::from(bytes);
                            changed = true;
                        }
                    }
                    if record.signature == "OMOD" && subrecord.signature == "DATA" {
                        if let Some(bytes) = rewrite_omod(&subrecord.data, rewrite)? {
                            subrecord.data = Bytes::from(bytes);
                            changed = true;
                        }
                    }
                }
                changed |= rewrite_referenced_form_ids_in_subrecords(
                    record.signature.as_str(),
                    &mut record.subrecords,
                    schema,
                    rewrite,
                );
                if changed {
                    record.raw_payload = None;
                }
            }
        }
    }
    Ok(())
}

fn schema_for_edit(slot: &NativePluginSlot) -> Result<Option<Arc<CompiledSchema>>, String> {
    if slot.lazy.is_some() {
        return Err(
            "Master edits require a fully loaded plugin; reopen without lazy_index".to_string(),
        );
    }
    slot.parsed
        .game
        .as_deref()
        .map(compiled_schema_for_game)
        .transpose()
        .map_err(|error| error.to_string())
}

pub(super) fn copy_record(
    record: &mut ParsedRecord,
    source_masters: &[String],
    source_plugin_name: &str,
    target_masters: &[String],
    source_own_index: u8,
    schema: Option<&CompiledSchema>,
) -> Result<(), String> {
    let mut items = [ParsedItem::Record(record.clone())];
    let mut rewrite = |raw: u32| {
        let replacement = remap_formid_for_copy(
            raw, source_masters, source_plugin_name, target_masters, source_own_index,
        );
        (raw != replacement).then_some(replacement)
    };
    rewrite_items(&mut items, schema, true, &mut rewrite)?;
    let [ParsedItem::Record(remapped)] = items else { unreachable!() };
    *record = remapped;
    Ok(())
}

pub(super) fn remap(
    slot: &mut NativePluginSlot,
    old: &[String],
    new: &[String],
) -> Result<(), String> {
    let schema = schema_for_edit(slot)?;
    let mut items = slot.parsed.root_items.clone();
    let mut overridden_forms = slot.parsed.header.overridden_forms.clone();
    let mut missing = BTreeSet::new();
    let mut rewrite = |raw: u32| {
        if raw != 0 && raw != u32::MAX {
            if let Some(master) = old.get((raw >> 24) as usize) {
                if !new
                    .iter()
                    .any(|candidate| candidate.eq_ignore_ascii_case(master))
                {
                    missing.insert(master.clone());
                    return None;
                }
            }
        }
        let replacement = remap_formid_index(raw, old, new, old.len() as u8, new.len() as u8);
        (replacement != raw).then_some(replacement)
    };
    rewrite_items(&mut items, schema.as_deref(), true, &mut rewrite)?;
    for raw in &mut overridden_forms {
        if let Some(replacement) = rewrite(*raw) {
            *raw = replacement;
        }
    }
    if !missing.is_empty() {
        return Err(format!(
            "Cannot remove masters still used by records or parent groups: {}",
            missing.into_iter().collect::<Vec<_>>().join(", ")
        ));
    }
    slot.parsed.root_items = items;
    slot.parsed.header.overridden_forms = overridden_forms;
    slot.parsed.header.raw_subrecords.clear();
    Ok(())
}

pub(super) fn used_indices(slot: &NativePluginSlot) -> Result<Vec<u8>, String> {
    let schema = schema_for_edit(slot)?;
    let own_index = slot.parsed.header.masters.len();
    let mut used = BTreeSet::new();
    let mut visit = |raw: u32| {
        let index = (raw >> 24) as u8;
        if raw != 0 && (index as usize) < own_index {
            used.insert(index);
        }
        None
    };
    rewrite_items(
        &mut slot.parsed.root_items.clone(),
        schema.as_deref(),
        true,
        &mut visit,
    )?;
    for raw in &slot.parsed.header.overridden_forms {
        visit(*raw);
    }
    Ok(used.into_iter().collect())
}

pub(super) fn null_refs(slot: &mut NativePluginSlot, index: u8) -> Result<usize, String> {
    let schema = schema_for_edit(slot)?;
    let mut items = slot.parsed.root_items.clone();
    let mut count = 0;
    let mut rewrite = |raw: u32| {
        if raw != 0 && raw != u32::MAX && (raw >> 24) as u8 == index {
            count += 1;
            Some(0)
        } else {
            None
        }
    };
    rewrite_items(&mut items, schema.as_deref(), false, &mut rewrite)?;
    slot.parsed.root_items = items;
    slot.parsed
        .header
        .overridden_forms
        .retain(|raw| (*raw >> 24) as u8 != index);
    slot.parsed.header.raw_subrecords.clear();
    Ok(count)
}
