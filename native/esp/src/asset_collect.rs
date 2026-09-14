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
    workshop_snap_points: Option<Vec<WorkshopSnapPoint>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AssetOwnerClaim {
    source_form_key: String,
    source_record_signature: String,
    source_subrecord_sig: String,
    idle_topology: Vec<IdleTopologyNode>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct IdleTopologyNode {
    form_key: String,
    model_path: Option<String>,
    parent_form_key: Option<String>,
    previous_form_key: Option<String>,
    conditions: Vec<(String, String)>,
}

type WorkshopSnapPointPayload = (String, (f32, f32, f32), (f32, f32, f32, f32), f32);

type IdleTopologyNodePayload = (
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Vec<(String, String)>,
);

type AssetOwnerClaimPayload = (String, String, String, Vec<IdleTopologyNodePayload>);

type CollectedAssetPayload = (
    String,
    String,
    String,
    String,
    String,
    Option<(f32, f32, f32)>,
    Option<Vec<WorkshopSnapPointPayload>>,
    Vec<AssetOwnerClaimPayload>,
);

type AssetKey = (SmolStr, String);

#[derive(Default)]
struct WorkshopWirePointIndex {
    by_template: HashMap<FormKey, (f32, f32, f32)>,
}

#[derive(Clone, Debug)]
struct WorkshopSnapPoint {
    name: String,
    translation: (f32, f32, f32),
    rotation: (f32, f32, f32, f32),
    scale: f32,
}

#[derive(Clone, Debug)]
struct TemplateNode {
    id: u32,
    node: FormKey,
    translation: (f32, f32, f32),
    rotation_degrees: (f32, f32, f32),
}

#[derive(Clone, Debug, Default)]
struct SnapTemplate {
    parent: Option<FormKey>,
    nodes: Vec<TemplateNode>,
    parent_overrides: HashMap<u32, ((f32, f32, f32), (f32, f32, f32))>,
}

#[derive(Clone, Debug, Default)]
struct SnapNode {
    editor_id: Option<String>,
    no_self_snap: bool,
    adjacent: HashSet<FormKey>,
    angles: Vec<f32>,
}

#[derive(Default)]
struct WorkshopSnapPointIndex {
    by_template: HashMap<FormKey, Vec<WorkshopSnapPoint>>,
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
            let (workshop_wire_points, workshop_snap_points) =
                workshop_indexes_from_plugin(&slot.parsed);
            if filters.form_key_filter.is_some() {
                collect_assets_from_form_keys(
                    &slot.parsed,
                    &filters,
                    &idle_topology,
                    &workshop_wire_points,
                    &workshop_snap_points,
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
                    &workshop_snap_points,
                    &mut seen,
                    &mut ambiguous_wire_points,
                    &mut out,
                );
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
                        asset.workshop_snap_points.map(workshop_snap_point_payloads),
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
    workshop_snap_points: &WorkshopSnapPointIndex,
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
        workshop_snap_points,
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
    workshop_snap_points: &WorkshopSnapPointIndex,
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
            workshop_snap_points,
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
    workshop_snap_points: &WorkshopSnapPointIndex,
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
                workshop_snap_points,
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
                workshop_snap_points,
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
    workshop_snap_points: &WorkshopSnapPointIndex,
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
        workshop_snap_points,
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
    workshop_snap_points: &WorkshopSnapPointIndex,
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
        };
        let workshop_wire_point = (asset.kind.as_str() == "nif"
            && asset.source_subrecord_sig.as_str() == "MODL")
            .then(|| workshop_wire_points.for_record(record, plugin))
            .flatten();
        let workshop_snap_points = (asset.kind.as_str() == "nif"
            && asset.source_subrecord_sig.as_str() == "MODL")
            .then(|| workshop_snap_points.for_record(record, plugin))
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
            merge_workshop_snap_points(
                &dedup_key,
                workshop_snap_points,
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
            workshop_snap_points,
        });
    }
}

impl WorkshopWirePointIndex {
    #[cfg(test)]
    fn from_plugin(plugin: &ParsedPlugin) -> Self {
        workshop_indexes_from_plugin(plugin).0
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

impl WorkshopSnapPointIndex {
    #[cfg(test)]
    fn from_plugin(plugin: &ParsedPlugin) -> Self {
        workshop_indexes_from_plugin(plugin).1
    }

    fn from_parts(
        templates: HashMap<FormKey, SnapTemplate>,
        mut node_defs: HashMap<FormKey, SnapNode>,
    ) -> Self {
        let edges = node_defs
            .iter()
            .flat_map(|(node, definition)| {
                definition
                    .adjacent
                    .iter()
                    .cloned()
                    .map(|adjacent| (node.clone(), adjacent))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        for (node, adjacent) in edges {
            node_defs
                .entry(adjacent)
                .or_insert_with(SnapNode::default)
                .adjacent
                .insert(node);
        }

        let template_keys = templates.keys().cloned().collect::<Vec<_>>();
        let mut flattened = HashMap::new();
        let mut by_template = HashMap::new();
        for template_key in template_keys {
            let nodes = flatten_snap_template(
                &template_key,
                &templates,
                &mut flattened,
                &mut HashSet::new(),
            );
            let points = expand_snap_points(&nodes, &node_defs);
            if !points.is_empty() {
                by_template.insert(template_key, points);
            }
        }
        Self { by_template }
    }

    fn for_record(
        &self,
        record: &ParsedRecord,
        plugin: &ParsedPlugin,
    ) -> Option<Vec<WorkshopSnapPoint>> {
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
        self.by_template.get(&snap_template).cloned()
    }
}

fn workshop_indexes_from_plugin(
    plugin: &ParsedPlugin,
) -> (WorkshopWirePointIndex, WorkshopSnapPointIndex) {
    if plugin.game.as_deref() != Some("fo76") {
        return Default::default();
    }

    let own_plugin_name: Arc<str> = Arc::from(plugin.plugin_name.as_str());
    let mut wire_attach_nodes = HashSet::new();
    let mut wire_candidates = HashMap::new();
    let mut node_defs = HashMap::new();
    let mut templates = HashMap::new();
    for record in records(plugin) {
        let form_key =
            resolve_form_id_to_form_key(record.form_id, &own_plugin_name, &plugin.header.masters);
        match record.signature.as_str() {
            "STND" => {
                let editor_id = record_editor_id(record);
                if editor_id
                    .is_some_and(|value| value.eq_ignore_ascii_case("defaultSnapNode_WireAttach"))
                {
                    wire_attach_nodes.insert(form_key.clone());
                }
                let no_self_snap = editor_id
                    .is_some_and(|value| value.to_ascii_lowercase().contains("noselfsnap"));
                let adjacent = record
                    .subrecords
                    .iter()
                    .filter(|subrecord| {
                        subrecord.signature.as_str() == "NNAM" && subrecord.data.len() >= 4
                    })
                    .map(|subrecord| {
                        resolve_form_id_to_form_key(
                            u32::from_le_bytes(subrecord.data[..4].try_into().unwrap()),
                            &own_plugin_name,
                            &plugin.header.masters,
                        )
                    })
                    .collect();
                let angles = record
                    .subrecords
                    .iter()
                    .filter(|subrecord| {
                        subrecord.signature.as_str() == "FLTV" && subrecord.data.len() >= 4
                    })
                    .filter_map(|subrecord| {
                        let value = f32::from_le_bytes(
                            subrecord.data[..4].try_into().expect("checked length"),
                        );
                        value.is_finite().then_some(value)
                    })
                    .collect();
                node_defs.insert(
                    form_key,
                    SnapNode {
                        editor_id: editor_id.map(str::to_string),
                        no_self_snap,
                        adjacent,
                        angles,
                    },
                );
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
                wire_candidates.insert(form_key.clone(), candidates);
                templates.insert(
                    form_key,
                    parse_snap_template(record, plugin, &own_plugin_name),
                );
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
    (
        WorkshopWirePointIndex { by_template },
        WorkshopSnapPointIndex::from_parts(templates, node_defs),
    )
}

fn parse_snap_template(
    record: &ParsedRecord,
    plugin: &ParsedPlugin,
    own_plugin_name: &Arc<str>,
) -> SnapTemplate {
    let mut template = SnapTemplate::default();
    let mut pending_parent_node = None;
    for subrecord in &record.subrecords {
        match subrecord.signature.as_str() {
            "PNAM" if subrecord.data.len() >= 4 => {
                template.parent = Some(resolve_form_id_to_form_key(
                    u32::from_le_bytes(subrecord.data[..4].try_into().unwrap()),
                    own_plugin_name,
                    &plugin.header.masters,
                ));
            }
            "ENAM" if subrecord.data.len() >= 24 => {
                let id = u32::from_le_bytes(subrecord.data[..4].try_into().unwrap());
                let node = resolve_form_id_to_form_key(
                    u32::from_le_bytes(subrecord.data[4..8].try_into().unwrap()),
                    own_plugin_name,
                    &plugin.header.masters,
                );
                if let Some((translation, rotation_degrees)) =
                    parse_snap_transform(&subrecord.data, 8)
                {
                    template.nodes.push(TemplateNode {
                        id,
                        node,
                        translation,
                        rotation_degrees,
                    });
                }
            }
            "ONAM" if subrecord.data.len() >= 4 => {
                pending_parent_node =
                    Some(u32::from_le_bytes(subrecord.data[..4].try_into().unwrap()));
            }
            "TNAM" => {
                if let (Some(id), Some(transform)) = (
                    pending_parent_node.take(),
                    parse_snap_transform(&subrecord.data, 0),
                ) {
                    template.parent_overrides.insert(id, transform);
                }
            }
            _ => {}
        }
    }
    template
}

fn parse_snap_transform(data: &[u8], offset: usize) -> Option<((f32, f32, f32), (f32, f32, f32))> {
    let remaining = data.len().checked_sub(offset)?;
    let count = if remaining >= 24 {
        6
    } else if remaining >= 16 {
        4
    } else {
        return None;
    };
    let mut values = [0.0f32; 6];
    for (index, value) in values.iter_mut().take(count).enumerate() {
        let start = offset + index * 4;
        *value = f32::from_le_bytes(data[start..start + 4].try_into().ok()?);
        if !value.is_finite() {
            return None;
        }
    }
    let rotation = if count == 6 {
        (values[3], values[4], values[5])
    } else {
        (0.0, 0.0, values[3])
    };
    Some(((values[0], values[1], values[2]), rotation))
}

fn flatten_snap_template(
    key: &FormKey,
    templates: &HashMap<FormKey, SnapTemplate>,
    cache: &mut HashMap<FormKey, Vec<TemplateNode>>,
    visiting: &mut HashSet<FormKey>,
) -> Vec<TemplateNode> {
    if let Some(nodes) = cache.get(key) {
        return nodes.clone();
    }
    if !visiting.insert(key.clone()) {
        return Vec::new();
    }
    let Some(template) = templates.get(key) else {
        visiting.remove(key);
        return Vec::new();
    };
    let mut nodes = template
        .parent
        .as_ref()
        .map(|parent| flatten_snap_template(parent, templates, cache, visiting))
        .unwrap_or_default();
    for node in &mut nodes {
        if let Some((translation, rotation_degrees)) = template.parent_overrides.get(&node.id) {
            node.translation = *translation;
            node.rotation_degrees = *rotation_degrees;
        }
    }
    for node in &template.nodes {
        if let Some(existing) = nodes.iter_mut().find(|existing| existing.id == node.id) {
            *existing = node.clone();
        } else {
            nodes.push(node.clone());
        }
    }
    visiting.remove(key);
    cache.insert(key.clone(), nodes.clone());
    nodes
}

fn expand_snap_points(
    nodes: &[TemplateNode],
    node_defs: &HashMap<FormKey, SnapNode>,
) -> Vec<WorkshopSnapPoint> {
    let mut points = Vec::new();
    for node in nodes {
        let definition = node_defs.get(&node.node).cloned().unwrap_or_default();
        if let Some(name) = definition
            .editor_id
            .as_deref()
            .and_then(fo4_workshop_snap_name)
        {
            points.push(WorkshopSnapPoint {
                name: name.to_string(),
                translation: node.translation,
                rotation: euler_degrees_to_quaternion(
                    node.rotation_degrees.0,
                    node.rotation_degrees.1,
                    node.rotation_degrees.2,
                ),
                scale: 1.0,
            });
        }
        let mut names = Vec::new();
        if !definition.no_self_snap {
            names.push(format!("P-76-{}", snap_node_token(&node.node)));
        }
        names.extend(definition.adjacent.iter().map(|adjacent| {
            let mut pair = [snap_node_token(&node.node), snap_node_token(adjacent)];
            pair.sort();
            format!("P-76-{}-{}", pair[0], pair[1])
        }));
        names.sort();
        names.dedup();

        let mut angles = vec![0.0];
        angles.extend(definition.angles);
        angles.sort_by(f32::total_cmp);
        angles.dedup_by(|left, right| (*left - *right).abs() <= 0.001);
        for name in names {
            for angle in &angles {
                points.push(WorkshopSnapPoint {
                    name: name.clone(),
                    translation: node.translation,
                    rotation: euler_degrees_to_quaternion(
                        node.rotation_degrees.0,
                        node.rotation_degrees.1,
                        node.rotation_degrees.2 + angle,
                    ),
                    scale: 1.0,
                });
            }
        }
    }
    points.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.translation.0.total_cmp(&right.translation.0))
            .then_with(|| left.translation.1.total_cmp(&right.translation.1))
            .then_with(|| left.translation.2.total_cmp(&right.translation.2))
            .then_with(|| left.rotation.3.total_cmp(&right.rotation.3))
    });
    points.dedup_by(|left, right| workshop_snap_points_equal(left, right));
    points
}

/// Every FO4 name [`fo4_workshop_snap_name`] can return. `conversion_native` validates
/// incoming snap-point names against this list, so the two must not drift.
pub const FO4_WORKSHOP_SNAP_ALIASES: &[&str] = &[
    "P-WS-Origin",
    "P-WS-Rotation",
    "P-WS-SinkMax",
    "P-Floor",
    "P-Balcony01-Dif",
    "P-Balcony01",
    "P-WrhsRoofTwinPk01",
    "P-WrhsRoof02-Dif",
    "P-WrhsRoof02-Dif2",
];

/// FO4 snapping matches connect points by name, so a FO76 node only interoperates with
/// vanilla pieces if it is also emitted under the name FO4 uses for that position.
/// Entries were derived by matching converted connect-point transforms against the FO4
/// meshes the kit was ported from (FO76 warehouse/barn ← FO4 `dlc03barn*`); nodes with no
/// corroborated counterpart stay unmapped and keep only their opaque `P-76-*` name.
fn fo4_workshop_snap_name(editor_id: &str) -> Option<&'static str> {
    let normalized = editor_id.to_ascii_lowercase();
    match normalized.as_str() {
        "defaultsnapnode_origin" => Some("P-WS-Origin"),
        "defaultsnapnode_rotation" => Some("P-WS-Rotation"),
        "defaultsnapnode_sinkmax" => Some("P-WS-SinkMax"),
        "snapnode_floor01"
        | "snapnode_floor01_nofoundationsnap"
        | "snapnode_floor01foundation"
        | "snapnode_floor01foundation_ramp"
        | "snapnode_floor01foundation_stairside"
        | "snapnode_stairswithturn_top" => Some("P-Floor"),
        // `_noselfsnap` variants are deliberately excluded: giving them a shared FO4 name
        // would reintroduce the self-snapping they opt out of.
        "snapnode_floor01sm_a" => Some("P-Balcony01-Dif"),
        "snapnode_floor01sm_b" => Some("P-Balcony01"),
        "snapnode_roof01twinpeak" => Some("P-WrhsRoofTwinPk01"),
        "snapnode_roof01l" => Some("P-WrhsRoof02-Dif"),
        "snapnode_roof01r" => Some("P-WrhsRoof02-Dif2"),
        _ => None,
    }
}

fn snap_node_token(key: &FormKey) -> String {
    key.to_string()
        .rsplit(':')
        .next()
        .unwrap_or("UNKNOWN")
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_uppercase()
}

fn euler_degrees_to_quaternion(x: f32, y: f32, z: f32) -> (f32, f32, f32, f32) {
    let (sx, cx) = (x.to_radians() * 0.5).sin_cos();
    let (sy, cy) = (y.to_radians() * 0.5).sin_cos();
    let (sz, cz) = (z.to_radians() * 0.5).sin_cos();
    (
        cx * cy * cz + sx * sy * sz,
        sx * cy * cz - cx * sy * sz,
        cx * sy * cz + sx * cy * sz,
        cx * cy * sz - sx * sy * cz,
    )
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

fn merge_workshop_snap_points(
    key: &AssetKey,
    candidate: Option<Vec<WorkshopSnapPoint>>,
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
    let existing = &mut out[existing_index].workshop_snap_points;
    match existing {
        None => *existing = Some(candidate),
        Some(points) if workshop_snap_point_lists_equal(points, &candidate) => {}
        Some(_) => {
            *existing = None;
            ambiguous.insert(key.clone());
        }
    }
}

fn workshop_snap_point_payloads(points: Vec<WorkshopSnapPoint>) -> Vec<WorkshopSnapPointPayload> {
    points
        .into_iter()
        .map(|point| (point.name, point.translation, point.rotation, point.scale))
        .collect()
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
            )
        })
        .collect()
}

fn workshop_snap_point_lists_equal(
    left: &[WorkshopSnapPoint],
    right: &[WorkshopSnapPoint],
) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| workshop_snap_points_equal(left, right))
}

fn workshop_snap_points_equal(left: &WorkshopSnapPoint, right: &WorkshopSnapPoint) -> bool {
    left.name == right.name
        && tuple3_close(left.translation, right.translation)
        && tuple4_close(left.rotation, right.rotation)
        && (left.scale - right.scale).abs() <= 0.0001
}

fn tuple3_close(left: (f32, f32, f32), right: (f32, f32, f32)) -> bool {
    (left.0 - right.0).abs() <= 0.001
        && (left.1 - right.1).abs() <= 0.001
        && (left.2 - right.2).abs() <= 0.001
}

fn tuple4_close(left: (f32, f32, f32, f32), right: (f32, f32, f32, f32)) -> bool {
    (left.0 - right.0).abs() <= 0.0001
        && (left.1 - right.1).abs() <= 0.0001
        && (left.2 - right.2).abs() <= 0.0001
        && (left.3 - right.3).abs() <= 0.0001
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
        let snap_points = WorkshopSnapPointIndex::from_plugin(&plugin);
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
            &snap_points,
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
        let snap_points = WorkshopSnapPointIndex::from_plugin(&plugin);
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
            &snap_points,
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
        let snap_points = WorkshopSnapPointIndex::from_plugin(&plugin);
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
            &snap_points,
            &mut seen,
            &mut ambiguous,
            &mut assets,
        );

        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].workshop_wire_point, Some((3.5, 15.0, 77.0)));
        let points = assets[0]
            .workshop_snap_points
            .as_ref()
            .expect("full snap template points");
        assert_eq!(points.len(), 1);
        assert_eq!(points[0].name, "P-76-012595");
        assert_eq!(points[0].translation, (3.5, 15.0, 77.0));
        assert_eq!(points[0].rotation, (1.0, 0.0, 0.0, 0.0));
    }

    #[test]
    fn fo76_snap_template_inherits_overrides_and_expands_angles() {
        let node_a = ParsedRecord {
            signature: SmolStr::new("STND"),
            form_id: 0x0001_0001,
            flags: 0,
            version_control: 0,
            form_version: Some(195),
            version2: None,
            subrecords: vec![
                subrecord("EDID", "SnapNode_A"),
                raw_subrecord("NNAM", 0x0001_0002u32.to_le_bytes().to_vec()),
                raw_subrecord("FLTV", 90.0f32.to_le_bytes().to_vec()),
            ],
            raw_payload: None,
            parse_error: None,
        };
        let node_b = ParsedRecord {
            signature: SmolStr::new("STND"),
            form_id: 0x0001_0002,
            flags: 0,
            version_control: 0,
            form_version: Some(195),
            version2: None,
            subrecords: vec![subrecord("EDID", "SnapNode_B")],
            raw_payload: None,
            parse_error: None,
        };
        let mut parent_enam = Vec::new();
        parent_enam.extend_from_slice(&7u32.to_le_bytes());
        parent_enam.extend_from_slice(&0x0001_0001u32.to_le_bytes());
        for value in [1.0f32, 2.0, 3.0, 0.0, 0.0, 0.0] {
            parent_enam.extend_from_slice(&value.to_le_bytes());
        }
        parent_enam.extend_from_slice(&0u32.to_le_bytes());
        let parent = ParsedRecord {
            signature: SmolStr::new("STMP"),
            form_id: 0x0002_0001,
            flags: 0,
            version_control: 0,
            form_version: Some(195),
            version2: None,
            subrecords: vec![raw_subrecord("ENAM", parent_enam)],
            raw_payload: None,
            parse_error: None,
        };
        let mut override_transform = Vec::new();
        for value in [4.0f32, 5.0, 6.0, 0.0, 0.0, 180.0] {
            override_transform.extend_from_slice(&value.to_le_bytes());
        }
        let child = ParsedRecord {
            signature: SmolStr::new("STMP"),
            form_id: 0x0002_0002,
            flags: 0,
            version_control: 0,
            form_version: Some(195),
            version2: None,
            subrecords: vec![
                raw_subrecord("PNAM", 0x0002_0001u32.to_le_bytes().to_vec()),
                raw_subrecord("ONAM", 7u32.to_le_bytes().to_vec()),
                raw_subrecord("TNAM", override_transform),
            ],
            raw_payload: None,
            parse_error: None,
        };
        let activator = ParsedRecord {
            signature: SmolStr::new("ACTI"),
            form_id: 0x0003_0001,
            flags: 0,
            version_control: 0,
            form_version: Some(195),
            version2: None,
            subrecords: vec![
                raw_subrecord("SNTP", 0x0002_0002u32.to_le_bytes().to_vec()),
                subrecord("MODL", "Workshop/Foundation.nif"),
            ],
            raw_payload: None,
            parse_error: None,
        };
        let plugin = plugin(vec![node_a, node_b, parent, child, activator]);
        let snap_points = WorkshopSnapPointIndex::from_plugin(&plugin);
        let points = snap_points
            .for_record(
                records(&plugin)
                    .find(|record| record.signature.as_str() == "ACTI")
                    .unwrap(),
                &plugin,
            )
            .expect("inherited points");

        assert_eq!(points.len(), 4);
        assert!(
            points
                .iter()
                .all(|point| point.translation == (4.0, 5.0, 6.0))
        );
        assert!(points.iter().any(|point| point.name == "P-76-010001"));
        assert!(
            points
                .iter()
                .any(|point| point.name == "P-76-010001-010002")
        );
    }

    #[test]
    fn fo76_floor_node_adds_single_fo4_alias_without_angle_expansion() {
        let floor_node = ParsedRecord {
            signature: SmolStr::new("STND"),
            form_id: 0x000A_7382,
            flags: 0,
            version_control: 0,
            form_version: Some(181),
            version2: None,
            subrecords: vec![
                subrecord("EDID", "SnapNode_Floor01Foundation"),
                raw_subrecord("FLTV", 90.0f32.to_le_bytes().to_vec()),
            ],
            raw_payload: None,
            parse_error: None,
        };
        let mut node = Vec::new();
        node.extend_from_slice(&0u32.to_le_bytes());
        node.extend_from_slice(&0x000A_7382u32.to_le_bytes());
        for value in [0.0f32, 128.0, 0.0, 0.0, 0.0, 0.0] {
            node.extend_from_slice(&value.to_le_bytes());
        }
        let template = ParsedRecord {
            signature: SmolStr::new("STMP"),
            form_id: 0x0001_596A,
            flags: 0,
            version_control: 0,
            form_version: Some(195),
            version2: None,
            subrecords: vec![raw_subrecord("ENAM", node)],
            raw_payload: None,
            parse_error: None,
        };
        let foundation = ParsedRecord {
            signature: SmolStr::new("STAT"),
            form_id: 0x0049_41FF,
            flags: 0,
            version_control: 0,
            form_version: Some(208),
            version2: None,
            subrecords: vec![
                raw_subrecord("SNTP", 0x0001_596Au32.to_le_bytes().to_vec()),
                subrecord("MODL", "Workshop/Foundation.nif"),
            ],
            raw_payload: None,
            parse_error: None,
        };
        let plugin = plugin(vec![floor_node, template, foundation]);
        let points = WorkshopSnapPointIndex::from_plugin(&plugin)
            .for_record(
                records(&plugin)
                    .find(|record| record.signature.as_str() == "STAT")
                    .unwrap(),
                &plugin,
            )
            .expect("foundation snap points");

        assert_eq!(
            points
                .iter()
                .filter(|point| point.name == "P-Floor")
                .count(),
            1
        );
        assert_eq!(
            points
                .iter()
                .filter(|point| point.name == "P-76-0A7382")
                .count(),
            2
        );
    }

    #[test]
    fn every_fo4_alias_is_declared_for_the_conversion_validator() {
        for editor_id in [
            "defaultSnapNode_Origin",
            "defaultSnapNode_Rotation",
            "defaultSnapNode_SinkMax",
            "SnapNode_Floor01",
            "SnapNode_Floor01_NoFoundationSnap",
            "SnapNode_Floor01Foundation",
            "SnapNode_Floor01Foundation_Ramp",
            "SnapNode_Floor01Foundation_StairSide",
            "SnapNode_StairsWithTurn_Top",
            "SnapNode_Floor01Sm_A",
            "SnapNode_Floor01Sm_B",
            "SnapNode_Roof01TwinPeak",
            "SnapNode_Roof01L",
            "SnapNode_Roof01R",
        ] {
            let name = fo4_workshop_snap_name(editor_id)
                .unwrap_or_else(|| panic!("{editor_id} lost its FO4 alias"));
            assert!(
                FO4_WORKSHOP_SNAP_ALIASES.contains(&name),
                "{name} is missing from FO4_WORKSHOP_SNAP_ALIASES; conversion_native would \
                 reject it as an unsupported generated snap name"
            );
        }
        assert_eq!(
            fo4_workshop_snap_name("SnapNode_Floor01Sm_A_noselfsnap"),
            None
        );
        assert_eq!(fo4_workshop_snap_name("SnapNode_Wall01L"), None);
    }
}
