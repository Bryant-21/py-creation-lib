//! Cross-handle collection of asset paths already cached in plugin indices.

use super::*;

struct CollectedAsset {
    asset_type: String,
    source_path: String,
    source_form_key: String,
    source_record_signature: String,
    source_subrecord_sig: String,
    owner_claims: Vec<AssetOwnerClaim>,
    workshop_wire_point: Option<(f32, f32, f32)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AssetOwnerClaim {
    source_form_key: String,
    source_record_signature: String,
    source_subrecord_sig: String,
    idle_topology: Vec<IdleTopologyNode>,
    armor_family: Vec<super::starfield_armor_closure::ArmorFamilyModel>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct IdleTopologyNode {
    form_key: String,
    model_path: Option<String>,
    parent_form_key: Option<String>,
    previous_form_key: Option<String>,
    conditions: Vec<(String, String)>,
}

type IdleTopologyNodePayload = (
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Vec<(String, String)>,
);

type ArmorFamilyModelPayload = (String, String, String, String);
type AssetOwnerClaimPayload = (String, String, String, Vec<IdleTopologyNodePayload>, Vec<ArmorFamilyModelPayload>);

type CollectedAssetPayload = (
    String,
    String,
    String,
    String,
    String,
    Option<(f32, f32, f32)>,
    Vec<AssetOwnerClaimPayload>,
);

type AssetKey = (SmolStr, String);

#[derive(Default)]
struct WorkshopWirePointIndex {
    by_template: HashMap<FormKey, (f32, f32, f32)>,
}

struct AssetCollectFilters {
    kind_filter: Option<HashSet<String>>,
    signature_filter: Option<HashSet<SmolStr>>,
    form_key_filter: Option<Vec<String>>,
}

#[derive(Default)]
struct IdleTopologyIndex {
    by_form_key: HashMap<String, IdleTopologyNode>,
}

impl IdleTopologyIndex {
    fn add_plugin(&mut self, plugin: &ParsedPlugin) {
        let own_plugin_name: Arc<str> = Arc::from(plugin.plugin_name.as_str());
        for record in records(plugin).filter(|record| record.signature.as_str() == "IDLE") {
            let subrecords = effective_subrecords_for_record(record);
            let form_key = resolve_form_id_to_form_key(
                record.form_id,
                &own_plugin_name,
                &plugin.header.masters,
            )
            .to_string();
            let model_path = subrecords
                .iter()
                .find(|subrecord| subrecord.signature.as_str() == "MODL")
                .and_then(subrecord_as_path);
            let animation_links = subrecords
                .iter()
                .find(|subrecord| subrecord.signature.as_str() == "ANAM");
            let parent_form_key = animation_links.and_then(|subrecord| {
                resolve_idle_link(&subrecord.data, 0, &own_plugin_name, &plugin.header.masters)
            });
            let previous_form_key = animation_links.and_then(|subrecord| {
                resolve_idle_link(&subrecord.data, 4, &own_plugin_name, &plugin.header.masters)
            });
            let conditions = subrecords
                .iter()
                .filter(|subrecord| matches!(subrecord.signature.as_str(), "CTDA" | "CTDT"))
                .map(|subrecord| {
                    (
                        subrecord.signature.to_string(),
                        hex::encode_upper(&subrecord.data),
                    )
                })
                .collect();
            self.by_form_key
                .entry(form_key.clone())
                .or_insert_with(|| IdleTopologyNode {
                    form_key,
                    model_path,
                    parent_form_key,
                    previous_form_key,
                    conditions,
                });
        }
    }

    fn ancestry_for(&self, record: &ParsedRecord, form_key: &FormKey) -> Vec<IdleTopologyNode> {
        if record.signature.as_str() != "IDLE" {
            return Vec::new();
        }
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        self.append_ancestry(&form_key.to_string(), &mut seen, &mut out);
        out
    }

    fn append_ancestry(
        &self,
        form_key: &str,
        seen: &mut HashSet<String>,
        out: &mut Vec<IdleTopologyNode>,
    ) {
        if !seen.insert(form_key.to_string()) {
            return;
        }
        let Some(node) = self.by_form_key.get(form_key) else {
            return;
        };
        out.push(node.clone());
        if let Some(parent) = node.parent_form_key.as_deref() {
            self.append_ancestry(parent, seen, out);
        }
    }
}

fn resolve_idle_link(
    data: &[u8],
    offset: usize,
    own_plugin_name: &Arc<str>,
    masters: &[String],
) -> Option<String> {
    let raw = u32::from_le_bytes(data.get(offset..offset + 4)?.try_into().ok()?);
    (raw != 0).then(|| resolve_form_id_to_form_key(raw, own_plugin_name, masters).to_string())
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
        let mut seen = HashMap::new();
        let mut ambiguous_wire_points = HashSet::new();
        let mut out = Vec::new();
        let mut idle_topology = IdleTopologyIndex::default();
        let needs_idle_topology = filters
            .kind_filter
            .as_ref()
            .map(|kinds| kinds.contains("kf_animation"))
            .unwrap_or(true);
        if needs_idle_topology {
            for handle_id in source_handles.iter().chain(master_handles.iter()) {
                if let Some(slot) = store.get(handle_id) {
                    idle_topology.add_plugin(&slot.parsed);
                }
            }
        }
        for handle_id in source_handles.iter().chain(master_handles.iter()) {
            let Some(slot) = store.get(handle_id) else {
                continue;
            };
            let workshop_wire_points = WorkshopWirePointIndex::from_plugin(&slot.parsed);
            if filters.form_key_filter.is_some() {
                collect_assets_from_form_keys(
                    &slot.parsed,
                    &filters,
                    &idle_topology,
                    &workshop_wire_points,
                    &mut seen,
                    &mut ambiguous_wire_points,
                    &mut out,
                );
            } else {
                collect_assets_from_items(
                    &slot.parsed,
                    &filters,
                    &idle_topology,
                    &workshop_wire_points,
                    &mut seen,
                    &mut ambiguous_wire_points,
                    &mut out,
                );
            }
        }
        let mut armor_families = super::starfield_armor_closure::ArmorFamilyIndex::default();
        if filters.kind_filter.as_ref().is_none_or(|kinds| kinds.contains("nif"))
            && filters.signature_filter.as_ref().is_none_or(|sigs| sigs.contains("ARMA"))
        {
            for handle_id in source_handles.iter().chain(master_handles.iter()) {
                if let Some(slot) = store.get(handle_id) {
                    armor_families.add_plugin(&slot.parsed);
                }
            }
            for asset in &mut out {
                for claim in &mut asset.owner_claims {
                    if claim.source_record_signature == "ARMA" {
                        claim.armor_family = armor_families.companions(
                            &claim.source_form_key, &claim.source_subrecord_sig,
                        );
                    }
                }
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
                        asset.workshop_wire_point,
                        asset_owner_claim_payloads(asset.owner_claims),
                    )
                })
                .collect::<Vec<_>>(),
        )
    })
}

fn collect_assets_from_items(
    plugin: &ParsedPlugin,
    filters: &AssetCollectFilters,
    idle_topology: &IdleTopologyIndex,
    workshop_wire_points: &WorkshopWirePointIndex,
    seen: &mut HashMap<AssetKey, usize>,
    ambiguous_wire_points: &mut HashSet<AssetKey>,
    out: &mut Vec<CollectedAsset>,
) {
    let own_plugin_name: Arc<str> = Arc::from(plugin.plugin_name.as_str());
    collect_assets_from_parsed_items(
        &plugin.root_items,
        plugin,
        &own_plugin_name,
        filters,
        idle_topology,
        workshop_wire_points,
        seen,
        ambiguous_wire_points,
        out,
    );
}

fn collect_assets_from_form_keys(
    plugin: &ParsedPlugin,
    filters: &AssetCollectFilters,
    idle_topology: &IdleTopologyIndex,
    workshop_wire_points: &WorkshopWirePointIndex,
    seen: &mut HashMap<AssetKey, usize>,
    ambiguous_wire_points: &mut HashSet<AssetKey>,
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
        collect_assets_from_record_with_form_key(
            record,
            &form_key,
            plugin,
            idle_topology,
            workshop_wire_points,
            filters,
            seen,
            ambiguous_wire_points,
            out,
        );
    }
}

fn collect_assets_from_parsed_items(
    items: &[ParsedItem],
    plugin: &ParsedPlugin,
    own_plugin_name: &Arc<str>,
    filters: &AssetCollectFilters,
    idle_topology: &IdleTopologyIndex,
    workshop_wire_points: &WorkshopWirePointIndex,
    seen: &mut HashMap<AssetKey, usize>,
    ambiguous_wire_points: &mut HashSet<AssetKey>,
    out: &mut Vec<CollectedAsset>,
) {
    for item in items {
        match item {
            ParsedItem::Record(record) => collect_assets_from_record(
                record,
                plugin,
                own_plugin_name,
                idle_topology,
                workshop_wire_points,
                filters,
                seen,
                ambiguous_wire_points,
                out,
            ),
            ParsedItem::Group(group) => collect_assets_from_parsed_items(
                &group.children,
                plugin,
                own_plugin_name,
                filters,
                idle_topology,
                workshop_wire_points,
                seen,
                ambiguous_wire_points,
                out,
            ),
        }
    }
}

fn collect_assets_from_record(
    record: &ParsedRecord,
    plugin: &ParsedPlugin,
    own_plugin_name: &Arc<str>,
    idle_topology: &IdleTopologyIndex,
    workshop_wire_points: &WorkshopWirePointIndex,
    filters: &AssetCollectFilters,
    seen: &mut HashMap<AssetKey, usize>,
    ambiguous_wire_points: &mut HashSet<AssetKey>,
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
    collect_assets_from_record_with_form_key(
        record,
        &form_key,
        plugin,
        idle_topology,
        workshop_wire_points,
        filters,
        seen,
        ambiguous_wire_points,
        out,
    );
}

fn collect_assets_from_record_with_form_key(
    record: &ParsedRecord,
    form_key: &FormKey,
    plugin: &ParsedPlugin,
    idle_topology: &IdleTopologyIndex,
    workshop_wire_points: &WorkshopWirePointIndex,
    filters: &AssetCollectFilters,
    seen: &mut HashMap<AssetKey, usize>,
    ambiguous_wire_points: &mut HashSet<AssetKey>,
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
        let owner_claim = AssetOwnerClaim {
            source_form_key: form_key.to_string(),
            source_record_signature: record.signature.to_string(),
            source_subrecord_sig: asset.source_subrecord_sig.to_string(),
            idle_topology: idle_topology.ancestry_for(record, form_key),
            armor_family: Vec::new(),
        };
        let workshop_wire_point = (asset.kind.as_str() == "nif"
            && asset.source_subrecord_sig.as_str() == "MODL")
            .then(|| workshop_wire_points.for_record(record, plugin))
            .flatten();
        if let Some(&existing_index) = seen.get(&dedup_key) {
            if !out[existing_index].owner_claims.iter().any(|claim| {
                claim.source_form_key == owner_claim.source_form_key
                    && claim.source_record_signature == owner_claim.source_record_signature
                    && claim.source_subrecord_sig == owner_claim.source_subrecord_sig
            }) {
                out[existing_index].owner_claims.push(owner_claim);
            }
            merge_workshop_wire_point(
                &dedup_key,
                workshop_wire_point,
                existing_index,
                ambiguous_wire_points,
                out,
            );
            continue;
        }
        seen.insert(dedup_key, out.len());
        out.push(CollectedAsset {
            asset_type: asset.kind.to_string(),
            source_path: asset.path,
            source_form_key: form_key.to_string(),
            source_record_signature: record.signature.to_string(),
            source_subrecord_sig: asset.source_subrecord_sig.to_string(),
            owner_claims: vec![owner_claim],
            workshop_wire_point,
        });
    }
}

impl WorkshopWirePointIndex {
    fn from_plugin(plugin: &ParsedPlugin) -> Self {
        if plugin.game.as_deref() != Some("fo76") {
            return Self::default();
        }

        let own_plugin_name: Arc<str> = Arc::from(plugin.plugin_name.as_str());
        let mut wire_attach_nodes = HashSet::new();
        let mut wire_candidates = HashMap::new();
        for record in records(plugin) {
            let form_key = resolve_form_id_to_form_key(
                record.form_id,
                &own_plugin_name,
                &plugin.header.masters,
            );
            match record.signature.as_str() {
                "STND" => {
                    if record_editor_id(record).is_some_and(|value| {
                        value.eq_ignore_ascii_case("defaultSnapNode_WireAttach")
                    }) {
                        wire_attach_nodes.insert(form_key);
                    }
                }
                "STMP" => {
                    let candidates = record
                        .subrecords
                        .iter()
                        .filter_map(|subrecord| {
                            if subrecord.signature.as_str() != "ENAM" || subrecord.data.len() < 20 {
                                return None;
                            }
                            let node = resolve_form_id_to_form_key(
                                u32::from_le_bytes(subrecord.data[4..8].try_into().ok()?),
                                &own_plugin_name,
                                &plugin.header.masters,
                            );
                            let point = (
                                f32::from_le_bytes(subrecord.data[8..12].try_into().ok()?),
                                f32::from_le_bytes(subrecord.data[12..16].try_into().ok()?),
                                f32::from_le_bytes(subrecord.data[16..20].try_into().ok()?),
                            );
                            (point.0.is_finite() && point.1.is_finite() && point.2.is_finite())
                                .then_some((node, point))
                        })
                        .collect::<Vec<_>>();
                    wire_candidates.insert(form_key, candidates);
                }
                _ => {}
            }
        }

        let by_template = wire_candidates
            .into_iter()
            .filter_map(|(key, candidates)| {
                candidates
                    .iter()
                    .find(|(node, _)| wire_attach_nodes.contains(node))
                    .map(|(_, point)| (key, *point))
            })
            .collect();
        Self { by_template }
    }

    fn for_record(&self, record: &ParsedRecord, plugin: &ParsedPlugin) -> Option<(f32, f32, f32)> {
        let snap_template_raw = record
            .subrecords
            .iter()
            .find(|subrecord| subrecord.signature.as_str() == "SNTP")
            .and_then(|subrecord| {
                (subrecord.data.len() >= 4)
                    .then(|| u32::from_le_bytes(subrecord.data[..4].try_into().unwrap()))
            })?;
        let own_plugin_name: Arc<str> = Arc::from(plugin.plugin_name.as_str());
        let snap_template = resolve_form_id_to_form_key(
            snap_template_raw,
            &own_plugin_name,
            &plugin.header.masters,
        );
        self.by_template.get(&snap_template).copied()
    }
}

fn records(plugin: &ParsedPlugin) -> impl Iterator<Item = &ParsedRecord> {
    fn collect<'a>(items: &'a [ParsedItem], out: &mut Vec<&'a ParsedRecord>) {
        for item in items {
            match item {
                ParsedItem::Record(record) => out.push(record),
                ParsedItem::Group(group) => collect(&group.children, out),
            }
        }
    }

    let mut out = Vec::new();
    collect(&plugin.root_items, &mut out);
    out.into_iter()
}

fn record_editor_id(record: &ParsedRecord) -> Option<&str> {
    record
        .subrecords
        .iter()
        .find(|subrecord| subrecord.signature.as_str() == "EDID")
        .and_then(|subrecord| std::str::from_utf8(&subrecord.data).ok())
        .map(|value| value.trim_end_matches('\0'))
}

fn merge_workshop_wire_point(
    key: &AssetKey,
    candidate: Option<(f32, f32, f32)>,
    existing_index: usize,
    ambiguous: &mut HashSet<AssetKey>,
    out: &mut [CollectedAsset],
) {
    let Some(candidate) = candidate else {
        return;
    };
    if ambiguous.contains(key) {
        return;
    }
    let existing = &mut out[existing_index].workshop_wire_point;
    match *existing {
        None => *existing = Some(candidate),
        Some(point) if wire_points_equal(point, candidate) => {}
        Some(_) => {
            *existing = None;
            ambiguous.insert(key.clone());
        }
    }
}

fn asset_owner_claim_payloads(claims: Vec<AssetOwnerClaim>) -> Vec<AssetOwnerClaimPayload> {
    claims
        .into_iter()
        .map(|claim| {
            (
                claim.source_form_key,
                claim.source_record_signature,
                claim.source_subrecord_sig,
                claim
                    .idle_topology
                    .into_iter()
                    .map(|node| {
                        (
                            node.form_key,
                            node.model_path,
                            node.parent_form_key,
                            node.previous_form_key,
                            node.conditions,
                        )
                    })
                    .collect(),
                claim.armor_family.into_iter().map(|model| (
                    model.armor_form_key, model.owner_form_key,
                    model.source_subrecord_sig, model.source_path,
                )).collect(),
            )
        })
        .collect()
}

fn wire_points_equal(left: (f32, f32, f32), right: (f32, f32, f32)) -> bool {
    (left.0 - right.0).abs() <= 0.001
        && (left.1 - right.1).abs() <= 0.001
        && (left.2 - right.2).abs() <= 0.001
}

#[cfg(test)]
mod tests {
    use super::*;

    fn subrecord(signature: &str, path: &str) -> ParsedSubrecord {
        let mut data = path.as_bytes().to_vec();
        data.push(0);
        raw_subrecord(signature, data)
    }

    fn raw_subrecord(signature: &str, data: Vec<u8>) -> ParsedSubrecord {
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
        plugin_for_game("SeventySix.esm", "fo76", records)
    }

    fn plugin_for_game(name: &str, game: &str, records: Vec<ParsedRecord>) -> ParsedPlugin {
        ParsedPlugin {
            plugin_name: name.to_string(),
            file_path: String::new(),
            header_size: 0,
            header: ParsedPluginHeader::default_for_test(),
            root_items: records.into_iter().map(ParsedItem::Record).collect(),
            game: Some(game.to_string()),
        }
    }

    #[test]
    fn duplicate_asset_keeps_primary_owner_and_all_owner_claims() {
        // Relocation doesn't depend on owner signature, so a duplicate path
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
        let wire_points = WorkshopWirePointIndex::from_plugin(&plugin);
        let mut idle_topology = IdleTopologyIndex::default();
        idle_topology.add_plugin(&plugin);
        let mut seen = HashMap::new();
        let mut ambiguous = HashSet::new();
        let mut assets = Vec::new();

        collect_assets_from_items(
            &plugin,
            &filters,
            &idle_topology,
            &wire_points,
            &mut seen,
            &mut ambiguous,
            &mut assets,
        );

        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].source_record_signature, "FURN");
        assert_eq!(assets[0].source_form_key, "SeventySix.esm:000801");
        assert_eq!(assets[0].owner_claims.len(), 2);
        assert_eq!(
            assets[0]
                .owner_claims
                .iter()
                .map(|claim| claim.source_form_key.as_str())
                .collect::<Vec<_>>(),
            ["SeventySix.esm:000801", "SeventySix.esm:000802"]
        );
    }

    #[test]
    fn shared_arma_models_keep_both_perspectives_in_owner_claims() {
        let mut addon = record("ARMA", 0x003A_8DE7, "unused.nif");
        addon.subrecords = vec![
            subrecord("MOD2", "Clothes/ChronoWatch/Watch_M.nif"),
            subrecord("MOD3", "Clothes/ChronoWatch/Watch_F.nif"),
            subrecord("MOD4", "clothes/chronowatch/watch_m.NIF"),
            subrecord("MOD5", "Clothes/ChronoWatch/Watch_F.nif"),
            subrecord("MOD4", "Clothes/ChronoWatch/Watch_M.nif"),
        ];
        let plugin = plugin_for_game("Starfield.esm", "starfield", vec![addon]);
        let filters = AssetCollectFilters {
            kind_filter: None,
            signature_filter: None,
            form_key_filter: None,
        };
        let wire_points = WorkshopWirePointIndex::from_plugin(&plugin);
        let mut idle_topology = IdleTopologyIndex::default();
        idle_topology.add_plugin(&plugin);
        let mut seen = HashMap::new();
        let mut ambiguous = HashSet::new();
        let mut assets = Vec::new();

        collect_assets_from_items(
            &plugin,
            &filters,
            &idle_topology,
            &wire_points,
            &mut seen,
            &mut ambiguous,
            &mut assets,
        );

        assert_eq!(assets.len(), 2);
        for (asset, expected_roles) in assets.iter().zip([["MOD2", "MOD4"], ["MOD3", "MOD5"]]) {
            assert_eq!(asset.source_subrecord_sig, expected_roles[0]);
            assert_eq!(asset.owner_claims.len(), 2);
            assert!(asset.owner_claims.iter().all(|claim| {
                claim.source_record_signature == "ARMA"
                    && claim.source_form_key == "Starfield.esm:3A8DE7"
            }));
            assert_eq!(
                asset
                    .owner_claims
                    .iter()
                    .map(|claim| claim.source_subrecord_sig.as_str())
                    .collect::<Vec<_>>(),
                expected_roles,
            );
        }
    }

    #[test]
    fn fnv_idle_kf_claims_preserve_directory_ancestry_previous_and_conditions() {
        fn idle_record(
            form_id: u32,
            model_path: &str,
            parent: u32,
            previous: u32,
            condition: &[u8],
        ) -> ParsedRecord {
            let mut links = Vec::new();
            links.extend_from_slice(&parent.to_le_bytes());
            links.extend_from_slice(&previous.to_le_bytes());
            ParsedRecord {
                signature: SmolStr::new("IDLE"),
                form_id,
                flags: 0,
                version_control: 0,
                form_version: Some(15),
                version2: None,
                subrecords: vec![
                    subrecord("MODL", model_path),
                    raw_subrecord("CTDA", condition.to_vec()),
                    raw_subrecord("ANAM", links),
                ],
                raw_payload: None,
                parse_error: None,
            }
        }

        let plugin = plugin_for_game(
            "FalloutNV.esm",
            "fnv",
            vec![
                idle_record(
                    0x0011_8199,
                    "creatures\\NVGecko\\IdleAnims",
                    0,
                    0,
                    &[0x10, 0x20],
                ),
                idle_record(
                    0x0011_819B,
                    "creatures\\NVGecko\\IdleAnims\\MT_SpecialIdle_EyeLickLeft.kf",
                    0x0011_8199,
                    0,
                    &[0x30, 0x40],
                ),
                idle_record(
                    0x0011_819C,
                    "CREATURES\\nvgecko\\idleanims\\mt_specialidle_eyelickleft.KF",
                    0x0011_8199,
                    0x0011_819B,
                    &[0x50, 0x60],
                ),
            ],
        );
        let filters = AssetCollectFilters {
            kind_filter: Some(HashSet::from(["kf_animation".to_string()])),
            signature_filter: Some(HashSet::from([SmolStr::new("IDLE")])),
            form_key_filter: None,
        };
        let wire_points = WorkshopWirePointIndex::from_plugin(&plugin);
        let mut idle_topology = IdleTopologyIndex::default();
        idle_topology.add_plugin(&plugin);
        let mut seen = HashMap::new();
        let mut ambiguous = HashSet::new();
        let mut assets = Vec::new();

        collect_assets_from_items(
            &plugin,
            &filters,
            &idle_topology,
            &wire_points,
            &mut seen,
            &mut ambiguous,
            &mut assets,
        );

        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].asset_type, "kf_animation");
        assert_eq!(assets[0].owner_claims.len(), 2);
        let second_claim = &assets[0].owner_claims[1];
        assert_eq!(second_claim.source_form_key, "FalloutNV.esm:11819C");
        assert_eq!(
            second_claim
                .idle_topology
                .iter()
                .map(|node| node.form_key.as_str())
                .collect::<Vec<_>>(),
            ["FalloutNV.esm:11819C", "FalloutNV.esm:118199",]
        );
        assert_eq!(
            second_claim.idle_topology[0].parent_form_key.as_deref(),
            Some("FalloutNV.esm:118199")
        );
        assert_eq!(
            second_claim.idle_topology[0].previous_form_key.as_deref(),
            Some("FalloutNV.esm:11819B")
        );
        assert_eq!(
            second_claim.idle_topology[0].conditions,
            [("CTDA".to_string(), "5060".to_string())]
        );
        assert_eq!(
            second_claim.idle_topology[1].model_path.as_deref(),
            Some("creatures/NVGecko/IdleAnims")
        );
    }

    #[test]
    fn fo76_snap_template_wire_attach_is_collected_with_main_nif() {
        let wire_node = ParsedRecord {
            signature: SmolStr::new("STND"),
            form_id: 0x0001_2595,
            flags: 0,
            version_control: 0,
            form_version: Some(131),
            version2: None,
            subrecords: vec![subrecord("EDID", "defaultSnapNode_WireAttach")],
            raw_payload: None,
            parse_error: None,
        };
        let mut enam = Vec::new();
        enam.extend_from_slice(&0u32.to_le_bytes());
        enam.extend_from_slice(&0x0001_2595u32.to_le_bytes());
        enam.extend_from_slice(&3.5f32.to_le_bytes());
        enam.extend_from_slice(&15.0f32.to_le_bytes());
        enam.extend_from_slice(&77.0f32.to_le_bytes());
        enam.extend_from_slice(&[0; 16]);
        let template = ParsedRecord {
            signature: SmolStr::new("STMP"),
            form_id: 0x0076_BE76,
            flags: 0,
            version_control: 0,
            form_version: Some(201),
            version2: None,
            subrecords: vec![raw_subrecord("ENAM", enam)],
            raw_payload: None,
            parse_error: None,
        };
        let activator = ParsedRecord {
            signature: SmolStr::new("ACTI"),
            form_id: 0x0076_B545,
            flags: 0,
            version_control: 0,
            form_version: Some(208),
            version2: None,
            subrecords: vec![
                raw_subrecord("SNTP", 0x0076_BE76u32.to_le_bytes().to_vec()),
                subrecord(
                    "MODL",
                    "ATX/Workshop/Tato_GeneratorSmall/S17_Tato_GeneratorSmall.nif",
                ),
            ],
            raw_payload: None,
            parse_error: None,
        };
        let plugin = plugin(vec![wire_node, template, activator]);
        let filters = AssetCollectFilters {
            kind_filter: Some(HashSet::from(["nif".to_string()])),
            signature_filter: None,
            form_key_filter: None,
        };
        let wire_points = WorkshopWirePointIndex::from_plugin(&plugin);
        let mut idle_topology = IdleTopologyIndex::default();
        idle_topology.add_plugin(&plugin);
        let mut seen = HashMap::new();
        let mut ambiguous = HashSet::new();
        let mut assets = Vec::new();

        collect_assets_from_items(
            &plugin,
            &filters,
            &idle_topology,
            &wire_points,
            &mut seen,
            &mut ambiguous,
            &mut assets,
        );

        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].workshop_wire_point, Some((3.5, 15.0, 77.0)));
    }
}
