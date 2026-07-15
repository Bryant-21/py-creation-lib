//! Cross-handle collection of asset paths already cached in plugin indices.

use super::*;

struct CollectedAsset {
    asset_type: String,
    source_path: String,
    source_form_key: String,
    source_record_signature: String,
    source_subrecord_sig: String,
}

type CollectedAssetPayload = (String, String, String, String, String);

struct AssetCollectFilters {
    kind_filter: Option<HashSet<String>>,
    signature_filter: Option<HashSet<SmolStr>>,
    form_key_filter: Option<Vec<String>>,
}

#[pyfunction(name = "plugin_handle_collect_assets")]
pub(crate) fn plugin_handle_collect_assets_native(
    py: Python<'_>,
    source_handles: Vec<u64>,
    master_handles: Vec<u64>,
    asset_kinds: Option<Vec<String>>,
    signatures: Option<Vec<String>>,
    form_keys: Option<Vec<String>>,
) -> PyResult<Vec<CollectedAssetPayload>> {
    let filters = AssetCollectFilters {
        kind_filter: asset_kinds.map(|values| {
            values
                .into_iter()
                .map(|value| value.to_ascii_lowercase())
                .collect::<HashSet<_>>()
        }),
        signature_filter: signatures.map(|values| {
            values
                .into_iter()
                .map(|value| SmolStr::new(value.trim().to_ascii_uppercase()))
                .collect::<HashSet<_>>()
        }),
        form_key_filter: form_keys.map(|values| {
            values
                .into_iter()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>()
        }),
    };
    py.detach(move || {
        let store = plugin_handle_store().lock().unwrap();
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for handle_id in source_handles.iter().chain(master_handles.iter()) {
            let Some(slot) = store.get(handle_id) else {
                continue;
            };
            if filters.form_key_filter.is_some() {
                collect_assets_from_form_keys(&slot.parsed, &filters, &mut seen, &mut out);
            } else {
                collect_assets_from_items(&slot.parsed, &filters, &mut seen, &mut out);
            }
        }
        Ok::<_, PyErr>(
            out.into_iter()
                .map(|asset| {
                    (
                        asset.asset_type,
                        asset.source_path,
                        asset.source_form_key,
                        asset.source_record_signature,
                        asset.source_subrecord_sig,
                    )
                })
                .collect::<Vec<_>>(),
        )
    })
}

fn collect_assets_from_items(
    plugin: &ParsedPlugin,
    filters: &AssetCollectFilters,
    seen: &mut HashSet<(SmolStr, String)>,
    out: &mut Vec<CollectedAsset>,
) {
    let own_plugin_name: Arc<str> = Arc::from(plugin.plugin_name.as_str());
    collect_assets_from_parsed_items(
        &plugin.root_items,
        plugin,
        &own_plugin_name,
        filters,
        seen,
        out,
    );
}

fn collect_assets_from_form_keys(
    plugin: &ParsedPlugin,
    filters: &AssetCollectFilters,
    seen: &mut HashSet<(SmolStr, String)>,
    out: &mut Vec<CollectedAsset>,
) {
    let Some(form_keys) = &filters.form_key_filter else {
        return;
    };
    let locator = build_locator_section(plugin);
    let own_plugin_name: Arc<str> = Arc::from(plugin.plugin_name.as_str());
    for form_key in form_keys {
        let Some(entry) = locator_entry_by_form_key(&locator, form_key.as_str()) else {
            continue;
        };
        let Some(record) = locator.record(plugin, entry) else {
            continue;
        };
        let form_key =
            resolve_form_id_to_form_key(record.form_id, &own_plugin_name, &plugin.header.masters);
        collect_assets_from_record_with_form_key(record, &form_key, filters, seen, out);
    }
}

fn collect_assets_from_parsed_items(
    items: &[ParsedItem],
    plugin: &ParsedPlugin,
    own_plugin_name: &Arc<str>,
    filters: &AssetCollectFilters,
    seen: &mut HashSet<(SmolStr, String)>,
    out: &mut Vec<CollectedAsset>,
) {
    for item in items {
        match item {
            ParsedItem::Record(record) => {
                collect_assets_from_record(record, plugin, own_plugin_name, filters, seen, out)
            }
            ParsedItem::Group(group) => collect_assets_from_parsed_items(
                &group.children,
                plugin,
                own_plugin_name,
                filters,
                seen,
                out,
            ),
        }
    }
}

fn collect_assets_from_record(
    record: &ParsedRecord,
    plugin: &ParsedPlugin,
    own_plugin_name: &Arc<str>,
    filters: &AssetCollectFilters,
    seen: &mut HashSet<(SmolStr, String)>,
    out: &mut Vec<CollectedAsset>,
) {
    if let Some(filter) = &filters.signature_filter {
        if !filter.contains(&record.signature) {
            return;
        }
    }
    let form_key =
        resolve_form_id_to_form_key(record.form_id, own_plugin_name, &plugin.header.masters);
    if let Some(filter) = &filters.form_key_filter {
        if !filter
            .iter()
            .any(|query| form_key_matches(&form_key, query))
        {
            return;
        }
    }
    collect_assets_from_record_with_form_key(record, &form_key, filters, seen, out);
}

fn collect_assets_from_record_with_form_key(
    record: &ParsedRecord,
    form_key: &FormKey,
    filters: &AssetCollectFilters,
    seen: &mut HashSet<(SmolStr, String)>,
    out: &mut Vec<CollectedAsset>,
) {
    if let Some(filter) = &filters.signature_filter {
        if !filter.contains(&record.signature) {
            return;
        }
    }
    for asset in extract_asset_paths(record) {
        if let Some(filter) = &filters.kind_filter {
            if !filter.contains(&asset.kind.as_str().to_ascii_lowercase()) {
                continue;
            }
        }
        let dedup_key = (asset.kind.clone(), asset.path.to_ascii_lowercase());
        if !seen.insert(dedup_key) {
            continue;
        }
        out.push(CollectedAsset {
            asset_type: asset.kind.to_string(),
            source_path: asset.path,
            source_form_key: form_key.to_string(),
            source_record_signature: record.signature.to_string(),
            source_subrecord_sig: asset.source_subrecord_sig.to_string(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn subrecord(signature: &str, path: &str) -> ParsedSubrecord {
        let mut data = path.as_bytes().to_vec();
        data.push(0);
        ParsedSubrecord {
            signature: SmolStr::new(signature),
            data: Bytes::from(data),
            semantic_type: None,
        }
    }

    fn record(signature: &str, form_id: u32, path: &str) -> ParsedRecord {
        ParsedRecord {
            signature: SmolStr::new(signature),
            form_id,
            flags: 0,
            version_control: 0,
            form_version: None,
            version2: None,
            subrecords: vec![subrecord("MODL", path)],
            raw_payload: None,
            parse_error: None,
        }
    }

    fn plugin(records: Vec<ParsedRecord>) -> ParsedPlugin {
        ParsedPlugin {
            plugin_name: "SeventySix.esm".to_string(),
            file_path: String::new(),
            header_size: 0,
            header: ParsedPluginHeader::default_for_test(),
            root_items: records.into_iter().map(ParsedItem::Record).collect(),
            game: Some("fo76".to_string()),
        }
    }

    #[test]
    fn duplicate_asset_keeps_first_seen_owner() {
        // Relocation no longer depends on owner signature, so a duplicate path
        // keeps the first record that referenced it (FURN), not a STAT override.
        let plugin = plugin(vec![
            record("FURN", 0x0000_0801, "Meshes/Furniture/StoneBench01.nif"),
            record("STAT", 0x0000_0802, "meshes/furniture/stonebench01.NIF"),
        ]);
        let filters = AssetCollectFilters {
            kind_filter: None,
            signature_filter: None,
            form_key_filter: None,
        };
        let mut seen = HashSet::new();
        let mut assets = Vec::new();

        collect_assets_from_items(&plugin, &filters, &mut seen, &mut assets);

        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].source_record_signature, "FURN");
        assert_eq!(assets[0].source_form_key, "SeventySix.esm:000801");
    }
}
