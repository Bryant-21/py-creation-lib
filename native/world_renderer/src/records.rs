use crate::error::{Result, WorldRendererError};
use crate::model::{CellBounds, Report, WorldSession};
use esp_authoring_core::plugin_runtime::{
    ParsedGroup, ParsedItem, ParsedPlugin, ParsedRecord, ParsedSubrecord,
    parse_plugin_file_lazy_compressed,
};
use serde_json::json;
use std::collections::BTreeMap;
use std::path::Path;

pub const WORLD_CHILD_GROUP: i32 = 1;
pub const CELL_CHILD_GROUP: i32 = 6;
pub const PERSISTENT_GROUP: i32 = 8;
pub const TEMPORARY_GROUP: i32 = 9;
pub const VISIBLE_DISTANT_GROUP: i32 = 10;

pub fn is_static_renderable_signature(signature: &str) -> bool {
    matches!(
        signature,
        "STAT" | "SCOL" | "MSTT" | "TREE" | "FLOR" | "ACTI" | "DOOR" | "FURN" | "LIGH"
    )
}

pub fn is_marker_signature(signature: &str) -> bool {
    matches!(
        signature,
        "REFR" | "PMIS" | "PGRE" | "PBEA" | "PFLA" | "MSTT"
    )
}

pub fn is_placed_child_signature(signature: &str) -> bool {
    matches!(
        signature,
        "REFR" | "ACHR" | "ACRE" | "PMIS" | "PGRE" | "PBEA" | "PFLA" | "PHZD" | "PGRD"
    )
}

pub fn list_worldspaces(session: &WorldSession) -> Result<Report> {
    let plugins = parse_plugins(session)?;
    let worldspaces = collect_worldspaces_from_plugins(&plugins);
    Ok(Report::ok(json!({ "worldspaces": worldspaces }))
        .with_count("worldspaces", worldspaces.len() as u64))
}

pub fn parse_plugins(session: &WorldSession) -> Result<Vec<ParsedPlugin>> {
    let mut plugins = Vec::with_capacity(session.plugin_paths.len());
    for plugin_path in &session.plugin_paths {
        if !Path::new(plugin_path).is_file() {
            return Err(WorldRendererError::Message(format!(
                "Plugin path does not exist: {plugin_path}"
            )));
        }
        let plugin =
            parse_plugin_file_lazy_compressed(plugin_path, Some(session.game_id().to_string()))
                .map_err(|err| WorldRendererError::Message(err.to_string()))?;
        plugins.push(plugin);
    }
    Ok(plugins)
}

pub fn collect_worldspaces_from_plugins(plugins: &[ParsedPlugin]) -> Vec<serde_json::Value> {
    let mut worldspaces = Vec::new();
    for plugin in plugins {
        for record in records_with_signature(&plugin.root_items, "WRLD") {
            let editor_id = subrecord_zstring(record, "EDID").unwrap_or_default();
            let name = subrecord_zstring(record, "FULL").unwrap_or_else(|| editor_id.clone());
            worldspaces.push(json!({
                "form_key": render_form_key(plugin, record.form_id),
                "editor_id": editor_id,
                "name": name,
                "source_plugin": plugin.plugin_name,
            }));
        }
    }
    worldspaces
}

pub fn records_with_signature<'a>(
    items: &'a [ParsedItem],
    signature: &str,
) -> Vec<&'a ParsedRecord> {
    let mut records = Vec::new();
    collect_records_with_signature(items, signature, &mut records);
    records
}

pub fn all_records<'a>(items: &'a [ParsedItem], records: &mut Vec<&'a ParsedRecord>) {
    for item in items {
        match item {
            ParsedItem::Record(record) => records.push(record),
            ParsedItem::Group(group) => all_records(&group.children, records),
        }
    }
}

pub fn top_group<'a>(plugin: &'a ParsedPlugin, signature: &str) -> Option<&'a ParsedGroup> {
    let wanted = signature.as_bytes();
    if wanted.len() != 4 {
        return None;
    }
    plugin.root_items.iter().find_map(|item| {
        let ParsedItem::Group(group) = item else {
            return None;
        };
        if group.group_type == 0 && group.label == [wanted[0], wanted[1], wanted[2], wanted[3]] {
            Some(group)
        } else {
            None
        }
    })
}

pub fn decode_group_form_id(group: &ParsedGroup) -> u32 {
    u32::from_le_bytes(group.label)
}

pub fn find_world<'a>(plugin: &'a ParsedPlugin, worldspace: &str) -> Option<&'a ParsedRecord> {
    let wrld_group = top_group(plugin, "WRLD")?;
    wrld_group.children.iter().find_map(|item| {
        let ParsedItem::Record(record) = item else {
            return None;
        };
        if record.signature.as_str() != "WRLD" {
            return None;
        }
        let form_key_matches =
            render_form_key(plugin, record.form_id).eq_ignore_ascii_case(worldspace);
        let edid_matches = subrecord_zstring(record, "EDID")
            .is_some_and(|editor_id| editor_id.eq_ignore_ascii_case(worldspace));
        let name_matches = subrecord_zstring(record, "FULL")
            .is_some_and(|name| name.eq_ignore_ascii_case(worldspace));
        (form_key_matches || edid_matches || name_matches).then_some(record)
    })
}

pub fn find_world_children_group<'a>(
    wrld_group: &'a ParsedGroup,
    world_form_id: u32,
) -> Option<&'a ParsedGroup> {
    wrld_group.children.iter().find_map(|item| {
        let ParsedItem::Group(group) = item else {
            return None;
        };
        (group.group_type == WORLD_CHILD_GROUP && decode_group_form_id(group) == world_form_id)
            .then_some(group)
    })
}

pub fn collect_cell_child_groups<'a>(
    group: &'a ParsedGroup,
    out: &mut BTreeMap<u32, &'a ParsedGroup>,
) {
    if group.group_type == CELL_CHILD_GROUP {
        out.entry(decode_group_form_id(group)).or_insert(group);
    }
    for child in &group.children {
        if let ParsedItem::Group(child_group) = child {
            collect_cell_child_groups(child_group, out);
        }
    }
}

pub fn collect_group_records<'a>(
    group: &'a ParsedGroup,
    signature: &str,
    records: &mut Vec<&'a ParsedRecord>,
) {
    for child in &group.children {
        match child {
            ParsedItem::Record(record) if record.signature.as_str() == signature => {
                records.push(record);
            }
            ParsedItem::Group(child_group) => {
                collect_group_records(child_group, signature, records);
            }
            ParsedItem::Record(_) => {}
        }
    }
}

pub fn collect_placed_records<'a>(group: &'a ParsedGroup, records: &mut Vec<&'a ParsedRecord>) {
    for child in &group.children {
        match child {
            ParsedItem::Record(record) if is_placed_child_signature(record.signature.as_str()) => {
                records.push(record);
            }
            ParsedItem::Group(child_group)
                if matches!(
                    child_group.group_type,
                    PERSISTENT_GROUP | TEMPORARY_GROUP | VISIBLE_DISTANT_GROUP
                ) =>
            {
                collect_placed_records(child_group, records);
            }
            ParsedItem::Group(child_group) => {
                collect_placed_records(child_group, records);
            }
            ParsedItem::Record(_) => {}
        }
    }
}

pub fn cell_grid(record: &ParsedRecord) -> Option<[i32; 2]> {
    let subrecord = subrecord(record, "XCLC")?;
    let bytes = subrecord.data.as_ref();
    if bytes.len() < 8 {
        return None;
    }
    Some([
        i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        i32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
    ])
}

pub fn inside_bounds(cell: [i32; 2], bounds: CellBounds) -> bool {
    bounds.min_x <= cell[0]
        && cell[0] <= bounds.max_x
        && bounds.min_y <= cell[1]
        && cell[1] <= bounds.max_y
}

fn collect_records_with_signature<'a>(
    items: &'a [ParsedItem],
    signature: &str,
    records: &mut Vec<&'a ParsedRecord>,
) {
    for item in items {
        match item {
            ParsedItem::Record(record) if record.signature.as_str() == signature => {
                records.push(record);
            }
            ParsedItem::Group(group) => {
                collect_records_with_signature(&group.children, signature, records)
            }
            ParsedItem::Record(_) => {}
        }
    }
}

pub fn render_form_key(plugin: &ParsedPlugin, raw_form_id: u32) -> String {
    let mod_index = (raw_form_id >> 24) as usize;
    let object_id = raw_form_id & 0x00FF_FFFF;
    let plugin_name = if mod_index < plugin.header.masters.len() {
        plugin.header.masters[mod_index].as_str()
    } else {
        plugin.plugin_name.as_str()
    };
    format!("{plugin_name}:{object_id:06X}")
}

pub fn subrecord<'a>(record: &'a ParsedRecord, signature: &str) -> Option<&'a ParsedSubrecord> {
    record
        .subrecords
        .iter()
        .find(|subrecord| subrecord.signature.as_str() == signature)
}

pub fn subrecord_zstring(record: &ParsedRecord, signature: &str) -> Option<String> {
    let subrecord = subrecord(record, signature)?;
    let bytes = subrecord.data.as_ref();
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    if end == 0 {
        return None;
    }
    Some(String::from_utf8_lossy(&bytes[..end]).to_string())
}

pub fn subrecord_u32(record: &ParsedRecord, signature: &str) -> Option<u32> {
    let subrecord = subrecord(record, signature)?;
    let bytes = subrecord.data.as_ref();
    if bytes.len() < 4 {
        return None;
    }
    Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

pub fn subrecord_f32(record: &ParsedRecord, signature: &str) -> Option<f32> {
    let subrecord = subrecord(record, signature)?;
    let bytes = subrecord.data.as_ref();
    if bytes.len() < 4 {
        return None;
    }
    Some(f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

pub fn refr_data_transform(record: &ParsedRecord) -> Option<([f32; 3], [f32; 3])> {
    let subrecord = subrecord(record, "DATA")?;
    let bytes = subrecord.data.as_ref();
    if bytes.len() < 24 {
        return None;
    }
    let read = |offset: usize| {
        f32::from_le_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ])
    };
    let position = [read(0), read(4), read(8)];
    let rotation = [
        read(12).to_degrees(),
        read(16).to_degrees(),
        read(20).to_degrees(),
    ];
    Some((position, rotation))
}
