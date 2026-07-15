use super::*;

pub(crate) fn record_as_authoring_value(
    record: &ParsedRecord,
    plugin: &ParsedPlugin,
    strings: &LocalizedStringsState,
) -> serde_json::Value {
    serialize_record_payload_to_json(record, plugin, strings)
}

pub(crate) fn authoring_value_to_record(
    value: &serde_json::Value,
    context: &mut NativeImportContext,
) -> PyResult<ParsedRecord> {
    let payload = json_object(value, "authoring record")?;
    parse_record_from_json_compact_native(payload, context)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_header() -> ParsedPluginHeader {
        ParsedPluginHeader {
            version: 1.0,
            num_records: 0,
            next_object_id: 0x800,
            author: String::new(),
            description: String::new(),
            masters: Vec::new(),
            master_sizes: Vec::new(),
            overridden_forms: Vec::new(),
            flags: 0,
            extra_subrecords: Vec::new(),
            version_control: 0,
            form_version: Some(131),
            version2: Some(0),
            hedr_raw: None,
            raw_subrecords: Vec::new(),
        }
    }

    #[test]
    fn record_as_authoring_value_uses_compact_shape() {
        let plugin = ParsedPlugin {
            plugin_name: "AuthoringRecord.esp".to_string(),
            file_path: String::new(),
            header_size: 24,
            header: empty_header(),
            root_items: Vec::new(),
            game: Some("fo4".to_string()),
        };
        let strings = LocalizedStringsState::default();
        let record = ParsedRecord {
            signature: SmolStr::new("MISC"),
            form_id: 0xFF00_0800,
            flags: 0,
            version_control: 0,
            form_version: None,
            version2: None,
            subrecords: vec![ParsedSubrecord {
                signature: SmolStr::new("EDID"),
                data: Bytes::from_static(b"NativeAuthoringRecord\0"),
                semantic_type: Some("text".to_string()),
            }],
            raw_payload: None,
            parse_error: None,
        };

        let payload = record_as_authoring_value(&record, &plugin, &strings);

        assert_eq!(payload["form_id"], "000800");
        assert_eq!(payload["eid"], "NativeAuthoringRecord");
        assert_eq!(payload["fields"], serde_json::json!([]));
    }
}
