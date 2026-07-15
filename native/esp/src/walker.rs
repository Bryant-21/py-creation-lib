//! Native BFS dependency walker over per-handle plugin indices.

use super::*;
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

#[derive(Default, Clone)]
pub(crate) struct WalkResult {
    pub(crate) reached_records: Vec<ReachedRecord>,
    pub(crate) assets: Vec<WalkAssetRef>,
    pub(crate) errors: Vec<String>,
    pub(crate) unresolved_form_keys: Vec<Arc<str>>,
    pub(crate) timing_ms: HashMap<String, u128>,
}

impl WalkResult {
    fn merge(&mut self, mut other: WalkResult) {
        self.reached_records.append(&mut other.reached_records);
        self.assets.append(&mut other.assets);
        self.errors.append(&mut other.errors);
        self.unresolved_form_keys
            .append(&mut other.unresolved_form_keys);
    }
}

#[derive(Clone)]
pub(crate) struct ReachedRecord {
    pub(crate) form_key: Arc<str>,
    pub(crate) signature: SmolStr,
    pub(crate) defined_in: Arc<str>,
    pub(crate) master_plugin: Arc<str>,
    pub(crate) is_override: bool,
    pub(crate) eid: String,
    pub(crate) walk_depth: u32,
    pub(crate) walker_pass: SmolStr,
    pub(crate) added_by_form_key: Option<Arc<str>>,
}

#[derive(Clone)]
pub(crate) struct WalkAssetRef {
    pub(crate) asset_kind: SmolStr,
    pub(crate) source_path: String,
    pub(crate) source_form_key: Arc<str>,
    pub(crate) source_record_signature: SmolStr,
    pub(crate) source_subrecord_sig: SmolStr,
    pub(crate) walk_depth: u32,
    pub(crate) walker_pass: SmolStr,
}

struct WalkFilters {
    follow_signatures: Option<HashSet<SmolStr>>,
    asset_kinds: Option<HashSet<String>>,
    terminal_signatures: Option<HashSet<SmolStr>>,
}

struct WalkIndex {
    core: Arc<CoreSection>,
    refs: Arc<RefsSection>,
    assets: Arc<AssetsSection>,
}

struct LazyWalkIndex {
    handle_id: u64,
    locator: Arc<LocatorSection>,
    schema: Option<Arc<CompiledSchema>>,
}

#[derive(Clone)]
struct LazyRecordFacts {
    eid: String,
    forward_refs: Vec<FormKey>,
    refs_by_subrecord: HashMap<SmolStr, Vec<FormKey>>,
    assets: Vec<AssetPathEntry>,
}

struct WalkProgress {
    pass: String,
    started_at: Instant,
    last_log_at: Instant,
    processed: u64,
}

type WalkRecordPayload = (
    String,
    String,
    String,
    String,
    bool,
    String,
    u32,
    String,
    Option<String>,
);
type WalkAssetPayload = (String, String, String, String, String, u32, String);
type WalkTimingPayload = (String, u64);
type WalkPayload = (
    Vec<WalkRecordPayload>,
    Vec<WalkAssetPayload>,
    Vec<String>,
    Vec<String>,
    Vec<WalkTimingPayload>,
);

impl WalkProgress {
    fn new(pass: impl Into<String>) -> Self {
        let now = Instant::now();
        Self {
            pass: pass.into(),
            started_at: now,
            last_log_at: now,
            processed: 0,
        }
    }

    fn tick(
        &mut self,
        queue_len: usize,
        visited_len: usize,
        reached_len: usize,
        assets_len: usize,
        facts_len: usize,
        depth: u32,
    ) {
        self.processed += 1;
        if self.last_log_at.elapsed() < Duration::from_secs(5) {
            return;
        }
        eprintln!(
            "[walk_dependencies] pass={} elapsed_ms={} processed={} queue={} visited={} reached={} assets={} facts={} last_depth={}",
            self.pass,
            self.started_at.elapsed().as_millis(),
            self.processed,
            queue_len,
            visited_len,
            reached_len,
            assets_len,
            facts_len,
            depth,
        );
        self.last_log_at = Instant::now();
    }
}

impl WalkFilters {
    fn from_policy(policy: &WalkPolicy) -> Self {
        Self {
            follow_signatures: policy.follow_signatures.as_ref().map(|values| {
                values
                    .iter()
                    .map(|value| SmolStr::new(value.trim().to_ascii_uppercase()))
                    .collect()
            }),
            asset_kinds: policy.asset_kinds.as_ref().map(|values| {
                values
                    .iter()
                    .map(|value| value.trim().to_ascii_lowercase())
                    .filter(|value| !value.is_empty())
                    .collect()
            }),
            terminal_signatures: policy.terminal_signatures.as_ref().map(|values| {
                values
                    .iter()
                    .map(|value| SmolStr::new(value.trim().to_ascii_uppercase()))
                    .collect()
            }),
        }
    }

    fn follows_signature(&self, signature: &SmolStr) -> bool {
        self.follow_signatures
            .as_ref()
            .is_none_or(|values| values.contains(signature))
    }

    fn is_terminal_signature(&self, signature: &SmolStr) -> bool {
        self.terminal_signatures
            .as_ref()
            .is_some_and(|values| values.contains(signature))
    }

    fn includes_asset_kind(&self, kind: &SmolStr) -> bool {
        self.asset_kinds
            .as_ref()
            .is_none_or(|values| values.contains(&kind.as_str().to_ascii_lowercase()))
    }
}

pub(crate) fn walk_main(
    source_indices: &[WalkIndex],
    master_indices: &[WalkIndex],
    roots: &[Arc<str>],
    policy: &WalkPolicy,
    strict: bool,
) -> WalkResult {
    let filters = WalkFilters::from_policy(policy);
    let mut visited: HashSet<Arc<str>> = HashSet::new();
    let mut queue: VecDeque<(Arc<str>, u32, Option<Arc<str>>)> = VecDeque::new();
    let mut result = WalkResult::default();

    if roots.is_empty() {
        if let Some(first_source) = source_indices.first() {
            for form_key in first_source.core.by_form_key.keys() {
                queue.push_back((form_key.render_arc(), 0, None));
            }
        }
    } else {
        for form_key in roots {
            queue.push_back((form_key.clone(), 0, None));
        }
    }

    while let Some((form_key, depth, added_by)) = queue.pop_front() {
        if policy.max_depth.is_some_and(|max_depth| depth > max_depth) {
            continue;
        }
        let Some((entry_idx, entry)) =
            find_entry_with_index(source_indices, master_indices, form_key.as_ref())
        else {
            let unresolved = normalize_form_key(form_key.as_ref())
                .map(|key| key.render_arc())
                .unwrap_or(form_key);
            if !visited.insert(unresolved.clone()) {
                continue;
            }
            if strict {
                result
                    .errors
                    .push(format!("Unresolved FormKey: {}", unresolved));
            } else {
                result.unresolved_form_keys.push(unresolved);
            }
            continue;
        };
        let entry_form_key = entry.form_key.render_arc();
        if !visited.insert(entry_form_key.clone()) {
            continue;
        }
        if !filters.follows_signature(&entry.signature) {
            continue;
        }
        result.reached_records.push(ReachedRecord {
            form_key: entry_form_key.clone(),
            signature: entry.signature.clone(),
            defined_in: entry.defined_in.clone(),
            master_plugin: entry.master_plugin.clone(),
            is_override: entry.is_override,
            eid: entry.eid.clone(),
            walk_depth: depth,
            walker_pass: SmolStr::new("main"),
            added_by_form_key: added_by.clone(),
        });
        for asset in asset_paths_for_entry(entry_idx, entry) {
            if !filters.includes_asset_kind(&asset.kind) {
                continue;
            }
            result.assets.push(WalkAssetRef {
                asset_kind: asset.kind.clone(),
                source_path: asset.path.clone(),
                source_form_key: entry_form_key.clone(),
                source_record_signature: entry.signature.clone(),
                source_subrecord_sig: asset.source_subrecord_sig.clone(),
                walk_depth: depth,
                walker_pass: SmolStr::new("main"),
            });
        }
        if !filters.is_terminal_signature(&entry.signature) {
            let refs_to_follow = entry_idx
                .refs
                .forward_refs_by_form_key
                .get(&entry.form_key)
                .cloned()
                .unwrap_or_default();
            for referenced_form_key in refs_to_follow {
                queue.push_back((
                    referenced_form_key.render_arc(),
                    depth + 1,
                    Some(entry_form_key.clone()),
                ));
            }
        }
    }

    result
}

fn find_entry_with_index<'a>(
    source_indices: &'a [WalkIndex],
    master_indices: &'a [WalkIndex],
    form_key: &str,
) -> Option<(&'a WalkIndex, &'a RecordIndexEntry)> {
    source_indices
        .iter()
        .chain(master_indices.iter())
        .find_map(|idx| {
            record_index_entry_by_form_key(&idx.core, form_key).map(|entry| (idx, entry))
        })
}

fn find_lazy_entry_with_index<'a>(
    source_indices: &'a [LazyWalkIndex],
    master_indices: &'a [LazyWalkIndex],
    form_key: &str,
) -> Option<(&'a LazyWalkIndex, &'a RecordLocatorEntry)> {
    source_indices
        .iter()
        .chain(master_indices.iter())
        .find_map(|idx| locator_entry_by_form_key(&idx.locator, form_key).map(|entry| (idx, entry)))
}

fn lazy_record_facts(
    store: &HashMap<u64, NativePluginSlot>,
    idx: &LazyWalkIndex,
    entry: &RecordLocatorEntry,
) -> LazyRecordFacts {
    let Some(slot) = store.get(&idx.handle_id) else {
        return LazyRecordFacts {
            eid: String::new(),
            forward_refs: Vec::new(),
            refs_by_subrecord: HashMap::new(),
            assets: Vec::new(),
        };
    };
    let Some(record) = idx.locator.record(&slot.parsed, entry) else {
        return LazyRecordFacts {
            eid: String::new(),
            forward_refs: Vec::new(),
            refs_by_subrecord: HashMap::new(),
            assets: Vec::new(),
        };
    };

    let subrecords = effective_subrecords_for_record(record);
    let eid = editor_id_from_effective_subrecords(&subrecords);
    let referenced_form_ids = iter_referenced_form_ids_from_subrecords(
        entry.signature.as_str(),
        &subrecords,
        idx.schema.as_deref(),
    );

    let own_plugin_name = Arc::from(slot.parsed.plugin_name.as_str());
    let mut seen_form_keys = HashSet::new();
    let mut forward_refs = Vec::new();
    let mut refs_by_subrecord: HashMap<SmolStr, Vec<FormKey>> = HashMap::new();
    for (subrecord_sig, raw_ref_form_id) in referenced_form_ids {
        let target_fk = resolve_form_id_to_form_key(
            raw_ref_form_id,
            &own_plugin_name,
            &slot.parsed.header.masters,
        );
        if target_fk.is_empty() {
            continue;
        }
        if seen_form_keys.insert(target_fk.clone()) {
            forward_refs.push(target_fk.clone());
        }
        let refs_for_subrecord = refs_by_subrecord.entry(subrecord_sig).or_default();
        if !refs_for_subrecord
            .iter()
            .any(|existing| existing == &target_fk)
        {
            refs_for_subrecord.push(target_fk);
        }
    }

    LazyRecordFacts {
        eid,
        forward_refs,
        refs_by_subrecord,
        assets: extract_asset_paths_from_subrecords(entry.signature.as_str(), &subrecords),
    }
}

fn walk_main_lazy(
    store: &HashMap<u64, NativePluginSlot>,
    source_indices: &[LazyWalkIndex],
    master_indices: &[LazyWalkIndex],
    roots: &[Arc<str>],
    policy: &WalkPolicy,
    strict: bool,
    facts_cache: &mut HashMap<Arc<str>, LazyRecordFacts>,
) -> WalkResult {
    let filters = WalkFilters::from_policy(policy);
    let mut visited: HashSet<Arc<str>> = HashSet::new();
    let mut queue: VecDeque<(Arc<str>, u32, Option<Arc<str>>)> = VecDeque::new();
    let mut result = WalkResult::default();

    for form_key in roots {
        queue.push_back((form_key.clone(), 0, None));
    }

    let mut progress = WalkProgress::new("main");
    while let Some((form_key, depth, added_by)) = queue.pop_front() {
        progress.tick(
            queue.len(),
            visited.len(),
            result.reached_records.len(),
            result.assets.len(),
            0,
            depth,
        );
        if policy.max_depth.is_some_and(|max_depth| depth > max_depth) {
            continue;
        }
        let Some((entry_idx, entry)) =
            find_lazy_entry_with_index(source_indices, master_indices, form_key.as_ref())
        else {
            let unresolved = normalize_form_key(form_key.as_ref())
                .map(|key| key.render_arc())
                .unwrap_or(form_key);
            if !visited.insert(unresolved.clone()) {
                continue;
            }
            if strict {
                result
                    .errors
                    .push(format!("Unresolved FormKey: {}", unresolved));
            } else {
                result.unresolved_form_keys.push(unresolved);
            }
            continue;
        };

        let entry_form_key = entry.form_key.render_arc();
        if !visited.insert(entry_form_key.clone()) {
            continue;
        }
        if !filters.follows_signature(&entry.signature) {
            continue;
        }

        let facts = facts_cache
            .entry(entry_form_key.clone())
            .or_insert_with(|| lazy_record_facts(store, entry_idx, entry))
            .clone();

        result.reached_records.push(ReachedRecord {
            form_key: entry_form_key.clone(),
            signature: entry.signature.clone(),
            defined_in: entry.defined_in.clone(),
            master_plugin: entry.master_plugin.clone(),
            is_override: entry.is_override,
            eid: facts.eid.clone(),
            walk_depth: depth,
            walker_pass: SmolStr::new("main"),
            added_by_form_key: added_by.clone(),
        });

        for asset in &facts.assets {
            if !filters.includes_asset_kind(&asset.kind) {
                continue;
            }
            result.assets.push(WalkAssetRef {
                asset_kind: asset.kind.clone(),
                source_path: asset.path.clone(),
                source_form_key: entry_form_key.clone(),
                source_record_signature: entry.signature.clone(),
                source_subrecord_sig: asset.source_subrecord_sig.clone(),
                walk_depth: depth,
                walker_pass: SmolStr::new("main"),
            });
        }

        if !filters.is_terminal_signature(&entry.signature) {
            for referenced_form_key in facts.forward_refs {
                queue.push_back((
                    referenced_form_key.render_arc(),
                    depth + 1,
                    Some(entry_form_key.clone()),
                ));
            }
        }
    }

    result
}

fn refs_for_subrecord(
    idx: &WalkIndex,
    entry: &RecordIndexEntry,
    subrecord_sig: &str,
) -> Vec<Arc<str>> {
    idx.refs
        .refs_by_form_key_and_subrecord
        .get(&(entry.form_key.clone(), SmolStr::new(subrecord_sig)))
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|form_key| form_key.render_arc())
        .collect()
}

fn asset_paths_for_entry<'a>(
    idx: &'a WalkIndex,
    entry: &RecordIndexEntry,
) -> impl Iterator<Item = &'a AssetPathEntry> {
    idx.assets
        .asset_paths_by_form_key
        .get(&entry.form_key.render_arc())
        .into_iter()
        .flatten()
}

fn entry_source_index<'a>(
    source_indices: &'a [WalkIndex],
    entry: &RecordIndexEntry,
) -> Option<&'a WalkIndex> {
    source_indices.iter().find_map(|idx| {
        record_index_entry_by_form_key(&idx.core, entry.form_key.render().as_str())
            .is_some()
            .then_some(idx)
    })
}

fn has_reverse_pass(policy: &WalkPolicy, pass: &str) -> bool {
    policy
        .reverse_passes
        .iter()
        .any(|value| value.eq_ignore_ascii_case(pass))
}

fn walk_reverse_race(
    source_indices: &[WalkIndex],
    main_result: &WalkResult,
    visited: &mut HashSet<Arc<str>>,
) -> WalkResult {
    let mut result = WalkResult::default();
    for root in main_result
        .reached_records
        .iter()
        .filter(|record| record.walk_depth == 0)
    {
        if matches!(root.signature.as_str(), "NPC_" | "LVLN") {
            continue;
        }
        let Some(root_entry) = source_indices
            .iter()
            .find_map(|idx| record_index_entry_by_form_key(&idx.core, root.form_key.as_ref()))
        else {
            continue;
        };
        let Some(root_idx) = entry_source_index(source_indices, root_entry) else {
            continue;
        };
        for keyword_fk in refs_for_subrecord(root_idx, root_entry, "KWDA") {
            for idx in source_indices {
                let Some(referencing) =
                    form_key_refs(&idx.refs.reverse_refs_by_form_key, keyword_fk.as_ref())
                else {
                    continue;
                };
                for candidate_fk in referencing {
                    let Some(candidate) =
                        record_index_entry_by_form_key(&idx.core, candidate_fk.render().as_str())
                    else {
                        continue;
                    };
                    if candidate.signature.as_str() != "RACE"
                        || is_reverse_race_denied(candidate.eid.as_str())
                        || !visited.insert(candidate.form_key.render_arc())
                    {
                        continue;
                    }
                    result.reached_records.push(ReachedRecord {
                        form_key: candidate.form_key.render_arc(),
                        signature: candidate.signature.clone(),
                        defined_in: candidate.defined_in.clone(),
                        master_plugin: candidate.master_plugin.clone(),
                        is_override: candidate.is_override,
                        eid: candidate.eid.clone(),
                        walk_depth: root.walk_depth + 1,
                        walker_pass: SmolStr::new("reverse_race"),
                        added_by_form_key: Some(root.form_key.clone()),
                    });
                }
            }
        }
    }
    result
}

fn is_reverse_race_denied(eid: &str) -> bool {
    matches!(
        eid,
        "HumanRace"
            | "PowerArmorRace"
            | "DLC01PowerArmorRace"
            | "DLC03PowerArmorRace"
            | "DLC04PowerArmorRace"
            | "DLC06PowerArmorRace"
            | "ChildRace"
            | "GhoulRace"
            | "FeralGhoulRace"
    )
}

fn walk_reverse_skm(
    source_indices: &[WalkIndex],
    master_indices: &[WalkIndex],
    main_result: &WalkResult,
    policy: &WalkPolicy,
    strict: bool,
    visited: &mut HashSet<Arc<str>>,
) -> WalkResult {
    let mut result = WalkResult::default();
    for root in main_result
        .reached_records
        .iter()
        .filter(|record| record.walk_depth == 0 && record.signature.as_str() == "WEAP")
    {
        let Some(root_entry) = source_indices
            .iter()
            .find_map(|idx| record_index_entry_by_form_key(&idx.core, root.form_key.as_ref()))
        else {
            continue;
        };
        let Some(root_idx) = entry_source_index(source_indices, root_entry) else {
            continue;
        };
        let weapon_keywords = refs_for_subrecord(root_idx, root_entry, "KWDA")
            .into_iter()
            .collect::<HashSet<_>>();
        if weapon_keywords.is_empty() {
            continue;
        }
        let mut candidate_skms = Vec::new();
        for keyword_fk in &weapon_keywords {
            for idx in source_indices {
                let Some(referencing) =
                    form_key_refs(&idx.refs.reverse_refs_by_form_key, keyword_fk.as_ref())
                else {
                    continue;
                };
                for candidate_fk in referencing {
                    let Some(candidate) =
                        record_index_entry_by_form_key(&idx.core, candidate_fk.render().as_str())
                    else {
                        continue;
                    };
                    if candidate.signature.as_str() != "KSSM" {
                        continue;
                    }
                    let skm_keywords = refs_for_subrecord(idx, candidate, "KWDA")
                        .into_iter()
                        .collect::<HashSet<_>>();
                    if skm_keywords.is_empty() || !skm_keywords.is_subset(&weapon_keywords) {
                        continue;
                    }
                    if !candidate_skms.iter().any(|existing: &Arc<str>| {
                        existing.eq_ignore_ascii_case(&candidate.form_key.render())
                    }) {
                        candidate_skms.push(candidate.form_key.render_arc());
                    }
                }
            }
        }
        for skm_fk in candidate_skms {
            result.merge(walk_seed_with_pass(
                source_indices,
                master_indices,
                skm_fk,
                root.form_key.clone(),
                SmolStr::new("reverse_skm"),
                policy,
                strict,
                visited,
            ));
        }
    }
    result
}

fn walk_seed_with_pass(
    source_indices: &[WalkIndex],
    master_indices: &[WalkIndex],
    seed: Arc<str>,
    added_by_root: Arc<str>,
    walker_pass: SmolStr,
    policy: &WalkPolicy,
    strict: bool,
    visited: &mut HashSet<Arc<str>>,
) -> WalkResult {
    let filters = WalkFilters::from_policy(policy);
    let mut queue = VecDeque::from([(seed, 1_u32, Some(added_by_root))]);
    let mut result = WalkResult::default();
    let mut progress = WalkProgress::new(format!("seed:{}", walker_pass.as_str()));
    while let Some((form_key, depth, added_by)) = queue.pop_front() {
        progress.tick(
            queue.len(),
            visited.len(),
            result.reached_records.len(),
            result.assets.len(),
            0,
            depth,
        );
        if policy
            .max_depth
            .is_some_and(|max_depth| depth > max_depth + 1)
        {
            continue;
        }
        let Some((entry_idx, entry)) =
            find_entry_with_index(source_indices, master_indices, form_key.as_ref())
        else {
            let unresolved = normalize_form_key(form_key.as_ref())
                .map(|key| key.render_arc())
                .unwrap_or(form_key);
            if !visited.insert(unresolved.clone()) {
                continue;
            }
            if strict {
                result
                    .errors
                    .push(format!("Unresolved FormKey: {}", unresolved));
            } else {
                result.unresolved_form_keys.push(unresolved);
            }
            continue;
        };
        let entry_form_key = entry.form_key.render_arc();
        if !visited.insert(entry_form_key.clone()) {
            continue;
        }
        if !filters.follows_signature(&entry.signature) {
            continue;
        }
        result.reached_records.push(ReachedRecord {
            form_key: entry_form_key.clone(),
            signature: entry.signature.clone(),
            defined_in: entry.defined_in.clone(),
            master_plugin: entry.master_plugin.clone(),
            is_override: entry.is_override,
            eid: entry.eid.clone(),
            walk_depth: depth,
            walker_pass: walker_pass.clone(),
            added_by_form_key: added_by.clone(),
        });
        for asset in asset_paths_for_entry(entry_idx, entry) {
            if !filters.includes_asset_kind(&asset.kind) {
                continue;
            }
            result.assets.push(WalkAssetRef {
                asset_kind: asset.kind.clone(),
                source_path: asset.path.clone(),
                source_form_key: entry_form_key.clone(),
                source_record_signature: entry.signature.clone(),
                source_subrecord_sig: asset.source_subrecord_sig.clone(),
                walk_depth: depth,
                walker_pass: walker_pass.clone(),
            });
        }
        if policy
            .max_depth
            .is_some_and(|max_depth| depth >= max_depth + 1)
        {
            continue;
        }
        let refs_to_follow = refs_to_follow_for_seed_pass(entry_idx, entry, walker_pass.as_str());
        for referenced_form_key in refs_to_follow {
            queue.push_back((referenced_form_key, depth + 1, Some(entry_form_key.clone())));
        }
    }
    result
}

fn refs_to_follow_for_seed_pass(
    idx: &WalkIndex,
    entry: &RecordIndexEntry,
    walker_pass: &str,
) -> Vec<Arc<str>> {
    let forward_refs = idx
        .refs
        .forward_refs_by_form_key
        .get(&entry.form_key)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|form_key| form_key.render_arc())
        .collect::<Vec<_>>();
    if walker_pass != "reverse_skm" || entry.signature.as_str() != "KSSM" {
        return forward_refs;
    }
    let keyword_refs = refs_for_subrecord(idx, entry, "KWDA");
    forward_refs
        .into_iter()
        .filter(|form_key| {
            !keyword_refs
                .iter()
                .any(|keyword| keyword.as_ref().eq_ignore_ascii_case(form_key.as_ref()))
        })
        .collect()
}

fn lazy_refs_for_subrecord(
    facts_cache: &HashMap<Arc<str>, LazyRecordFacts>,
    form_key: &Arc<str>,
    subrecord_sig: &str,
) -> Vec<Arc<str>> {
    facts_cache
        .get(form_key)
        .and_then(|facts| {
            facts
                .refs_by_subrecord
                .get(&SmolStr::new(subrecord_sig))
                .cloned()
        })
        .unwrap_or_default()
        .into_iter()
        .map(|form_key| form_key.render_arc())
        .collect()
}

fn walk_seed_with_pass_lazy(
    store: &HashMap<u64, NativePluginSlot>,
    source_indices: &[LazyWalkIndex],
    master_indices: &[LazyWalkIndex],
    seed: Arc<str>,
    added_by_root: Arc<str>,
    walker_pass: SmolStr,
    policy: &WalkPolicy,
    strict: bool,
    visited: &mut HashSet<Arc<str>>,
    facts_cache: &mut HashMap<Arc<str>, LazyRecordFacts>,
) -> WalkResult {
    let filters = WalkFilters::from_policy(policy);
    let mut queue = VecDeque::from([(seed, 1_u32, Some(added_by_root))]);
    let mut result = WalkResult::default();
    let mut progress = WalkProgress::new(format!("seed:{}", walker_pass.as_str()));
    while let Some((form_key, depth, added_by)) = queue.pop_front() {
        progress.tick(
            queue.len(),
            visited.len(),
            result.reached_records.len(),
            result.assets.len(),
            facts_cache.len(),
            depth,
        );
        if policy
            .max_depth
            .is_some_and(|max_depth| depth > max_depth + 1)
        {
            continue;
        }
        let Some((entry_idx, entry)) =
            find_lazy_entry_with_index(source_indices, master_indices, form_key.as_ref())
        else {
            let unresolved = normalize_form_key(form_key.as_ref())
                .map(|key| key.render_arc())
                .unwrap_or(form_key);
            if !visited.insert(unresolved.clone()) {
                continue;
            }
            if strict {
                result
                    .errors
                    .push(format!("Unresolved FormKey: {}", unresolved));
            } else {
                result.unresolved_form_keys.push(unresolved);
            }
            continue;
        };
        let entry_form_key = entry.form_key.render_arc();
        if !visited.insert(entry_form_key.clone()) {
            continue;
        }
        if !filters.follows_signature(&entry.signature) {
            continue;
        }
        let facts = facts_cache
            .entry(entry_form_key.clone())
            .or_insert_with(|| lazy_record_facts(store, entry_idx, entry))
            .clone();
        result.reached_records.push(ReachedRecord {
            form_key: entry_form_key.clone(),
            signature: entry.signature.clone(),
            defined_in: entry.defined_in.clone(),
            master_plugin: entry.master_plugin.clone(),
            is_override: entry.is_override,
            eid: facts.eid.clone(),
            walk_depth: depth,
            walker_pass: walker_pass.clone(),
            added_by_form_key: added_by.clone(),
        });
        for asset in &facts.assets {
            if !filters.includes_asset_kind(&asset.kind) {
                continue;
            }
            result.assets.push(WalkAssetRef {
                asset_kind: asset.kind.clone(),
                source_path: asset.path.clone(),
                source_form_key: entry_form_key.clone(),
                source_record_signature: entry.signature.clone(),
                source_subrecord_sig: asset.source_subrecord_sig.clone(),
                walk_depth: depth,
                walker_pass: walker_pass.clone(),
            });
        }
        if policy
            .max_depth
            .is_some_and(|max_depth| depth >= max_depth + 1)
        {
            continue;
        }
        let refs_to_follow =
            if walker_pass.as_str() == "reverse_skm" && entry.signature.as_str() == "KSSM" {
                let keyword_refs = facts
                    .refs_by_subrecord
                    .get(&SmolStr::new("KWDA"))
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|form_key| form_key.render_arc())
                    .collect::<Vec<_>>();
                facts
                    .forward_refs
                    .into_iter()
                    .map(|form_key| form_key.render_arc())
                    .filter(|form_key| {
                        !keyword_refs
                            .iter()
                            .any(|keyword| keyword.as_ref().eq_ignore_ascii_case(form_key.as_ref()))
                    })
                    .collect::<Vec<_>>()
            } else {
                facts
                    .forward_refs
                    .into_iter()
                    .map(|form_key| form_key.render_arc())
                    .collect::<Vec<_>>()
            };
        for referenced_form_key in refs_to_follow {
            queue.push_back((referenced_form_key, depth + 1, Some(entry_form_key.clone())));
        }
    }
    result
}

fn walk_reverse_race_lazy(
    store: &HashMap<u64, NativePluginSlot>,
    source_indices: &[LazyWalkIndex],
    main_result: &WalkResult,
    facts_cache: &mut HashMap<Arc<str>, LazyRecordFacts>,
    visited: &mut HashSet<Arc<str>>,
) -> WalkResult {
    let mut result = WalkResult::default();
    for root in main_result
        .reached_records
        .iter()
        .filter(|record| record.walk_depth == 0)
    {
        if matches!(root.signature.as_str(), "NPC_" | "LVLN") {
            continue;
        }
        let Some((root_idx, root_entry)) =
            find_lazy_entry_with_index(source_indices, &[], root.form_key.as_ref())
        else {
            continue;
        };
        facts_cache
            .entry(root.form_key.clone())
            .or_insert_with(|| lazy_record_facts(store, root_idx, root_entry));
        let keyword_refs = lazy_refs_for_subrecord(facts_cache, &root.form_key, "KWDA");
        if keyword_refs.is_empty() {
            continue;
        }

        for idx in source_indices {
            let Some(race_keys) = idx
                .locator
                .by_signature_form_keys
                .get(&SmolStr::new("RACE"))
            else {
                continue;
            };
            for race_fk in race_keys {
                let Some(candidate) =
                    locator_entry_by_form_key(&idx.locator, race_fk.render().as_str())
                else {
                    continue;
                };
                let candidate_fk = candidate.form_key.render_arc();
                if visited.contains(&candidate_fk) {
                    continue;
                }
                let candidate_facts = facts_cache
                    .entry(candidate_fk.clone())
                    .or_insert_with(|| lazy_record_facts(store, idx, candidate))
                    .clone();
                if is_reverse_race_denied(candidate_facts.eid.as_str()) {
                    continue;
                }
                if !candidate_facts.forward_refs.iter().any(|race_keyword| {
                    keyword_refs
                        .iter()
                        .any(|root_keyword| root_keyword.as_ref() == race_keyword.render().as_str())
                }) {
                    continue;
                }
                if !visited.insert(candidate_fk.clone()) {
                    continue;
                }
                result.reached_records.push(ReachedRecord {
                    form_key: candidate_fk,
                    signature: candidate.signature.clone(),
                    defined_in: candidate.defined_in.clone(),
                    master_plugin: candidate.master_plugin.clone(),
                    is_override: candidate.is_override,
                    eid: candidate_facts.eid,
                    walk_depth: root.walk_depth + 1,
                    walker_pass: SmolStr::new("reverse_race"),
                    added_by_form_key: Some(root.form_key.clone()),
                });
            }
        }
    }
    result
}

fn walk_reverse_skm_lazy(
    store: &HashMap<u64, NativePluginSlot>,
    source_indices: &[LazyWalkIndex],
    master_indices: &[LazyWalkIndex],
    main_result: &WalkResult,
    policy: &WalkPolicy,
    strict: bool,
    facts_cache: &mut HashMap<Arc<str>, LazyRecordFacts>,
    visited: &mut HashSet<Arc<str>>,
) -> WalkResult {
    let mut result = WalkResult::default();
    for root in main_result
        .reached_records
        .iter()
        .filter(|record| record.walk_depth == 0 && record.signature.as_str() == "WEAP")
    {
        let Some((root_idx, root_entry)) =
            find_lazy_entry_with_index(source_indices, &[], root.form_key.as_ref())
        else {
            continue;
        };
        facts_cache
            .entry(root.form_key.clone())
            .or_insert_with(|| lazy_record_facts(store, root_idx, root_entry));
        let weapon_keywords = lazy_refs_for_subrecord(facts_cache, &root.form_key, "KWDA")
            .into_iter()
            .collect::<HashSet<_>>();
        if weapon_keywords.is_empty() {
            continue;
        }

        let mut candidate_skms = Vec::new();
        for idx in source_indices {
            let Some(skm_keys) = idx
                .locator
                .by_signature_form_keys
                .get(&SmolStr::new("KSSM"))
            else {
                continue;
            };
            for skm_fk in skm_keys {
                let Some(candidate) =
                    locator_entry_by_form_key(&idx.locator, skm_fk.render().as_str())
                else {
                    continue;
                };
                let candidate_fk = candidate.form_key.render_arc();
                facts_cache
                    .entry(candidate_fk.clone())
                    .or_insert_with(|| lazy_record_facts(store, idx, candidate));
                let skm_keywords = lazy_refs_for_subrecord(facts_cache, &candidate_fk, "KWDA")
                    .into_iter()
                    .collect::<HashSet<_>>();
                if skm_keywords.is_empty() || !skm_keywords.is_subset(&weapon_keywords) {
                    continue;
                }
                if !candidate_skms
                    .iter()
                    .any(|existing: &Arc<str>| existing.eq_ignore_ascii_case(candidate_fk.as_ref()))
                {
                    candidate_skms.push(candidate_fk);
                }
            }
        }

        for skm_fk in candidate_skms {
            result.merge(walk_seed_with_pass_lazy(
                store,
                source_indices,
                master_indices,
                skm_fk,
                root.form_key.clone(),
                SmolStr::new("reverse_skm"),
                policy,
                strict,
                visited,
                facts_cache,
            ));
        }
    }
    result
}

fn walk_dependencies_full_index_locked(
    store: &mut HashMap<u64, NativePluginSlot>,
    source_handles: Vec<u64>,
    master_handles: Vec<u64>,
    roots: &[Arc<str>],
    policy: &WalkPolicy,
    strict_unresolved_masters: bool,
) -> PyResult<WalkResult> {
    let mut source_indices = Vec::new();
    for handle_id in source_handles {
        let slot = store
            .get_mut(&handle_id)
            .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
        source_indices.push(WalkIndex {
            core: ensure_core_section(slot),
            refs: ensure_refs_section(slot),
            assets: ensure_assets_section(slot),
        });
    }
    let mut master_indices = Vec::new();
    for handle_id in master_handles {
        let slot = store
            .get_mut(&handle_id)
            .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
        master_indices.push(WalkIndex {
            core: ensure_core_section(slot),
            refs: ensure_refs_section(slot),
            assets: ensure_assets_section(slot),
        });
    }
    let mut result = walk_main(
        &source_indices,
        &master_indices,
        roots,
        policy,
        strict_unresolved_masters,
    );
    let mut visited = result
        .reached_records
        .iter()
        .map(|record| record.form_key.clone())
        .collect::<HashSet<_>>();
    if has_reverse_pass(policy, "race") {
        result.merge(walk_reverse_race(
            &source_indices,
            &result.clone(),
            &mut visited,
        ));
    }
    if has_reverse_pass(policy, "skm") {
        result.merge(walk_reverse_skm(
            &source_indices,
            &master_indices,
            &result.clone(),
            policy,
            strict_unresolved_masters,
            &mut visited,
        ));
    }
    Ok(result)
}

#[pyfunction(name = "plugin_handle_walk_dependencies")]
pub(crate) fn plugin_handle_walk_dependencies_native(
    py: Python<'_>,
    source_handles: Vec<u64>,
    master_handles: Vec<u64>,
    root_form_keys: Vec<String>,
    policy_json: &str,
    strict_unresolved_masters: bool,
) -> PyResult<WalkPayload> {
    let policy: WalkPolicy = serde_json::from_str(policy_json)
        .map_err(|error| PyValueError::new_err(format!("invalid policy JSON: {error}")))?;
    let roots = root_form_keys
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(|value| Arc::<str>::from(value.as_str()))
        .collect::<Vec<_>>();
    let result = py.detach(move || {
        let total_start = std::time::Instant::now();
        let mut store = plugin_handle_store().lock().unwrap();

        if roots.is_empty() {
            eprintln!(
                "[walk_dependencies] full-index start source_handles={} master_handles={} roots=0",
                source_handles.len(),
                master_handles.len(),
            );
            let mut result = walk_dependencies_full_index_locked(
                &mut store,
                source_handles,
                master_handles,
                &roots,
                &policy,
                strict_unresolved_masters,
            )?;
            result
                .timing_ms
                .insert("total_ms".to_string(), total_start.elapsed().as_millis());
            eprintln!(
                "[walk_dependencies] full-index done total_ms={} reached={} assets={} unresolved={} errors={}",
                total_start.elapsed().as_millis(),
                result.reached_records.len(),
                result.assets.len(),
                result.unresolved_form_keys.len(),
                result.errors.len(),
            );
            return Ok::<_, PyErr>(result);
        }

        let locator_start = std::time::Instant::now();
        let mut source_indices = Vec::new();
        for handle_id in source_handles {
            let slot = store.get_mut(&handle_id).ok_or_else(|| {
                PyKeyError::new_err(format!("unknown plugin handle: {handle_id}"))
            })?;
            let schema = slot
                .parsed
                .game
                .as_deref()
                .and_then(|game| compiled_schema_for_game(game).ok());
            source_indices.push(LazyWalkIndex {
                handle_id,
                locator: ensure_locator_section(slot),
                schema,
            });
        }
        let mut master_indices = Vec::new();
        for handle_id in master_handles {
            let slot = store.get_mut(&handle_id).ok_or_else(|| {
                PyKeyError::new_err(format!("unknown plugin handle: {handle_id}"))
            })?;
            let schema = slot
                .parsed
                .game
                .as_deref()
                .and_then(|game| compiled_schema_for_game(game).ok());
            master_indices.push(LazyWalkIndex {
                handle_id,
                locator: ensure_locator_section(slot),
                schema,
            });
        }
        let locator_ms = locator_start.elapsed().as_millis();
        eprintln!(
            "[walk_dependencies] lazy locator ready locator_ms={} roots={} source_handles={} master_handles={} reverse_passes={:?} max_depth={:?}",
            locator_ms,
            roots.len(),
            source_indices.len(),
            master_indices.len(),
            &policy.reverse_passes,
            policy.max_depth,
        );

        let main_start = std::time::Instant::now();
        let mut facts_cache = HashMap::new();
        let mut result = walk_main_lazy(
            &store,
            &source_indices,
            &master_indices,
            &roots,
            &policy,
            strict_unresolved_masters,
            &mut facts_cache,
        );
        let main_walk_ms = main_start.elapsed().as_millis();
        eprintln!(
            "[walk_dependencies] main done main_walk_ms={} reached={} assets={} unresolved={} errors={} facts={}",
            main_walk_ms,
            result.reached_records.len(),
            result.assets.len(),
            result.unresolved_form_keys.len(),
            result.errors.len(),
            facts_cache.len(),
        );

        let mut visited = result
            .reached_records
            .iter()
            .map(|record| record.form_key.clone())
            .collect::<HashSet<_>>();

        let race_start = std::time::Instant::now();
        if has_reverse_pass(&policy, "race") {
            eprintln!("[walk_dependencies] reverse_race start");
            result.merge(walk_reverse_race_lazy(
                &store,
                &source_indices,
                &result.clone(),
                &mut facts_cache,
                &mut visited,
            ));
        }
        let reverse_race_ms = race_start.elapsed().as_millis();
        if has_reverse_pass(&policy, "race") {
            eprintln!(
                "[walk_dependencies] reverse_race done reverse_race_ms={} reached={} facts={}",
                reverse_race_ms,
                result.reached_records.len(),
                facts_cache.len(),
            );
        }

        let skm_start = std::time::Instant::now();
        if has_reverse_pass(&policy, "skm") {
            eprintln!("[walk_dependencies] reverse_skm start");
            result.merge(walk_reverse_skm_lazy(
                &store,
                &source_indices,
                &master_indices,
                &result.clone(),
                &policy,
                strict_unresolved_masters,
                &mut facts_cache,
                &mut visited,
            ));
        }
        let reverse_skm_ms = skm_start.elapsed().as_millis();
        if has_reverse_pass(&policy, "skm") {
            eprintln!(
                "[walk_dependencies] reverse_skm done reverse_skm_ms={} reached={} facts={}",
                reverse_skm_ms,
                result.reached_records.len(),
                facts_cache.len(),
            );
        }

        result
            .timing_ms
            .insert("locator_ms".to_string(), locator_ms);
        result
            .timing_ms
            .insert("main_walk_ms".to_string(), main_walk_ms);
        result
            .timing_ms
            .insert("reverse_race_ms".to_string(), reverse_race_ms);
        result
            .timing_ms
            .insert("reverse_skm_ms".to_string(), reverse_skm_ms);
        result
            .timing_ms
            .insert("total_ms".to_string(), total_start.elapsed().as_millis());
        eprintln!(
            "[walk_dependencies] lazy done total_ms={} reached={} assets={} unresolved={} errors={} facts={}",
            total_start.elapsed().as_millis(),
            result.reached_records.len(),
            result.assets.len(),
            result.unresolved_form_keys.len(),
            result.errors.len(),
            facts_cache.len(),
        );
        Ok::<_, PyErr>(result)
    })?;

    Ok(walk_result_to_payload(&result))
}

fn walk_result_to_payload(result: &WalkResult) -> WalkPayload {
    let records = result
        .reached_records
        .iter()
        .map(|record| {
            (
                record.form_key.to_string(),
                record.signature.to_string(),
                record.defined_in.to_string(),
                record.master_plugin.to_string(),
                record.is_override,
                record.eid.clone(),
                record.walk_depth,
                record.walker_pass.to_string(),
                record
                    .added_by_form_key
                    .as_ref()
                    .map(|value| value.to_string()),
            )
        })
        .collect::<Vec<_>>();
    let assets = result
        .assets
        .iter()
        .map(|asset| {
            (
                asset.asset_kind.to_string(),
                asset.source_path.clone(),
                asset.source_form_key.to_string(),
                asset.source_record_signature.to_string(),
                asset.source_subrecord_sig.to_string(),
                asset.walk_depth,
                asset.walker_pass.to_string(),
            )
        })
        .collect::<Vec<_>>();
    let unresolved_form_keys = result
        .unresolved_form_keys
        .iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>();
    let timing = result
        .timing_ms
        .iter()
        .map(|(key, value)| (key.clone(), (*value).min(u64::MAX as u128) as u64))
        .collect::<Vec<_>>();
    (
        records,
        assets,
        result.errors.clone(),
        unresolved_form_keys,
        timing,
    )
}

// ---------------------------------------------------------------------------
// Pure-Rust walker API (no GIL) — for use from conversion_native phases
// ---------------------------------------------------------------------------

/// A single reached record returned by the pure-Rust walk.
#[derive(Debug, Clone)]
pub struct WalkRecord {
    pub form_key: String,
    pub signature: String,
    pub eid: String,
    pub walk_depth: u32,
    pub walker_pass: String,
    pub added_by_form_key: Option<String>,
    pub defined_in: String,
}

/// A single asset reference returned by the pure-Rust walk.
#[derive(Debug, Clone)]
pub struct WalkAsset {
    pub asset_kind: String,
    pub source_path: String,
    pub source_form_key: String,
    pub source_record_signature: String,
    pub source_subrecord_sig: String,
    pub walk_depth: u32,
    pub walker_pass: String,
}

/// The dependency graph output of `plugin_handle_walk_native_rs`.
#[derive(Debug, Clone, Default)]
pub struct WalkOutput {
    pub reached_records: Vec<WalkRecord>,
    pub assets: Vec<WalkAsset>,
    pub errors: Vec<String>,
    pub unresolved_form_keys: Vec<String>,
}

/// Run the dependency walker without touching the Python GIL.
///
/// - `source_handles`  — IDs of the primary (source) plugin handles.
/// - `master_handles`  — IDs of master plugin handles loaded alongside.
/// - `root_form_keys`  — seed form-keys in `"Plugin.esp:XXXXXX"` format.
///   Pass an empty vec for a full-index walk (all records in source).
/// - `policy_json`     — JSON-encoded `WalkPolicy`. Use `"{}"` for defaults.
/// - `strict`          — when `true`, unresolved master refs are errors.
pub fn plugin_handle_walk_native_rs(
    source_handles: Vec<u64>,
    master_handles: Vec<u64>,
    root_form_keys: Vec<String>,
    policy_json: &str,
    strict: bool,
) -> Result<WalkOutput, String> {
    let policy: WalkPolicy =
        serde_json::from_str(policy_json).map_err(|e| format!("invalid policy JSON: {e}"))?;
    let roots: Vec<Arc<str>> = root_form_keys
        .into_iter()
        .filter(|s| !s.is_empty())
        .map(|s| Arc::<str>::from(s.as_str()))
        .collect();

    let result: WalkResult = {
        let mut store = plugin_handle_store()
            .lock()
            .map_err(|e| format!("plugin_handle_store lock poisoned: {e}"))?;

        if roots.is_empty() {
            walk_dependencies_full_index_locked(
                &mut store,
                source_handles,
                master_handles,
                &roots,
                &policy,
                strict,
            )
            .map_err(|e| format!("walk_dependencies: {e}"))?
        } else {
            let mut source_indices = Vec::new();
            for handle_id in source_handles {
                let slot = store
                    .get_mut(&handle_id)
                    .ok_or_else(|| format!("unknown plugin handle: {handle_id}"))?;
                let schema = slot
                    .parsed
                    .game
                    .as_deref()
                    .and_then(|game| compiled_schema_for_game(game).ok());
                source_indices.push(LazyWalkIndex {
                    handle_id,
                    locator: ensure_locator_section(slot),
                    schema,
                });
            }
            let mut master_indices = Vec::new();
            for handle_id in master_handles {
                let slot = store
                    .get_mut(&handle_id)
                    .ok_or_else(|| format!("unknown plugin handle: {handle_id}"))?;
                let schema = slot
                    .parsed
                    .game
                    .as_deref()
                    .and_then(|game| compiled_schema_for_game(game).ok());
                master_indices.push(LazyWalkIndex {
                    handle_id,
                    locator: ensure_locator_section(slot),
                    schema,
                });
            }
            let mut facts_cache = HashMap::new();
            let mut result = walk_main_lazy(
                &store,
                &source_indices,
                &master_indices,
                &roots,
                &policy,
                strict,
                &mut facts_cache,
            );
            let mut visited: HashSet<Arc<str>> = result
                .reached_records
                .iter()
                .map(|r| r.form_key.clone())
                .collect();
            if has_reverse_pass(&policy, "race") {
                result.merge(walk_reverse_race_lazy(
                    &store,
                    &source_indices,
                    &result.clone(),
                    &mut facts_cache,
                    &mut visited,
                ));
            }
            if has_reverse_pass(&policy, "skm") {
                result.merge(walk_reverse_skm_lazy(
                    &store,
                    &source_indices,
                    &master_indices,
                    &result.clone(),
                    &policy,
                    strict,
                    &mut facts_cache,
                    &mut visited,
                ));
            }
            result
        }
    };

    Ok(WalkOutput {
        reached_records: result
            .reached_records
            .into_iter()
            .map(|r| WalkRecord {
                form_key: r.form_key.to_string(),
                signature: r.signature.to_string(),
                eid: r.eid,
                walk_depth: r.walk_depth,
                walker_pass: r.walker_pass.to_string(),
                added_by_form_key: r.added_by_form_key.map(|s| s.to_string()),
                defined_in: r.defined_in.to_string(),
            })
            .collect(),
        assets: result
            .assets
            .into_iter()
            .map(|a| WalkAsset {
                asset_kind: a.asset_kind.to_string(),
                source_path: a.source_path,
                source_form_key: a.source_form_key.to_string(),
                source_record_signature: a.source_record_signature.to_string(),
                source_subrecord_sig: a.source_subrecord_sig.to_string(),
                walk_depth: a.walk_depth,
                walker_pass: a.walker_pass.to_string(),
            })
            .collect(),
        errors: result.errors,
        unresolved_form_keys: result
            .unresolved_form_keys
            .into_iter()
            .map(|s| s.to_string())
            .collect(),
    })
}
