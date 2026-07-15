//! Lazy, per-handle lookup table sections built over a `ParsedPlugin`.
//!
//! Invalidated by callers via `NativePluginSlot::invalidate_index_sections`
//! whenever `parsed.root_items` is mutated.

use super::*;
use rustc_hash::FxHashMap;
use smallvec::SmallVec;
use smol_str::SmolStr;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// Identifier for a lazily-built lookup table group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IndexSection {
    Locator,
    Core,
    Records,
    Refs,
    Assets,
}

#[derive(Default, Clone)]
pub struct CoreSection {
    pub form_ids_by_signature: HashMap<SmolStr, Vec<u32>>,
    pub form_ids_by_object_id: HashMap<u32, Vec<u32>>,
    pub by_form_key: FormKeyIndex<RecordIndexEntry>,
    pub by_eid_lower: HashMap<String, Vec<FormKey>>,
    pub by_signature_form_keys: HashMap<SmolStr, Vec<FormKey>>,
}

#[derive(Default, Clone)]
pub struct RecordsSection {
    pub locator: FxHashMap<u32, RecordPath>,
}

impl RecordsSection {
    pub fn record<'a>(&self, plugin: &'a ParsedPlugin, form_id: u32) -> Option<&'a ParsedRecord> {
        let path = self.locator.get(&form_id)?;
        let mut items = plugin.root_items.as_slice();
        for index in &path.group_indices {
            let item = items.get(*index as usize)?;
            let ParsedItem::Group(group) = item else {
                return None;
            };
            items = group.children.as_slice();
        }
        match items.get(path.record_index as usize)? {
            ParsedItem::Record(record) => Some(record),
            ParsedItem::Group(_) => None,
        }
    }
}

#[derive(Default, Clone)]
pub struct LocatorSection {
    pub by_form_key: FormKeyIndex<RecordLocatorEntry>,
    pub by_signature_form_keys: HashMap<SmolStr, Vec<FormKey>>,
}

impl LocatorSection {
    pub fn record<'a>(
        &self,
        plugin: &'a ParsedPlugin,
        entry: &RecordLocatorEntry,
    ) -> Option<&'a ParsedRecord> {
        let mut items = plugin.root_items.as_slice();
        for index in &entry.path.group_indices {
            let item = items.get(*index as usize)?;
            let ParsedItem::Group(group) = item else {
                return None;
            };
            items = group.children.as_slice();
        }
        match items.get(entry.path.record_index as usize)? {
            ParsedItem::Record(record) => Some(record),
            ParsedItem::Group(_) => None,
        }
    }
}

/// Index from `form_id` to a path through `ParsedPlugin::root_items`.
/// Built lazily by `ensure_form_id_paths_section`, invalidated by
/// `WriteEffect::RecordsAddedOrRemoved`.
#[derive(Default, Clone)]
pub struct FormIdPathsSection {
    pub by_form_id: FxHashMap<u32, RecordPath>,
}

/// A path from `ParsedPlugin::root_items` down to a specific `ParsedRecord`.
/// `group_indices` holds the index into `Vec<ParsedItem>` at each nesting
/// level; `record_index` is the index inside the innermost level.
#[derive(Clone, Debug)]
pub struct RecordPath {
    pub group_indices: SmallVec<[u32; 4]>,
    pub record_index: u32,
}

#[derive(Default, Clone)]
pub struct RefsSection {
    pub forward_refs_by_form_key: HashMap<FormKey, Vec<FormKey>>,
    pub reverse_refs_by_form_key: HashMap<FormKey, Vec<FormKey>>,
    pub refs_by_form_key_and_subrecord: HashMap<(FormKey, SmolStr), Vec<FormKey>>,
}

#[derive(Default, Clone)]
pub struct AssetsSection {
    pub assets_by_kind: HashMap<SmolStr, Vec<(Arc<str>, String)>>,
    pub asset_paths_by_form_key: HashMap<Arc<str>, Vec<AssetPathEntry>>,
}

#[derive(Default)]
pub struct PluginIndexSections {
    pub locator: Option<Arc<LocatorSection>>,
    pub core: Option<Arc<CoreSection>>,
    pub records: Option<Arc<RecordsSection>>,
    pub form_id_paths: Option<Arc<FormIdPathsSection>>,
    pub refs: Option<Arc<RefsSection>>,
    pub assets: Option<Arc<AssetsSection>>,
}

/// What the most recent write changed. Used by `PluginIndexSections::apply_effect`
/// to invalidate only the cached sections that actually depend on the change.
#[derive(Debug, Clone)]
pub enum WriteEffect {
    /// Record body changed in place (subrecord bytes, field values).
    /// form_id, signature, EDID, masters, file structure unchanged.
    RecordContents { form_ids: SmallVec<[u32; 4]> },

    /// Records inserted or removed from the parsed tree.
    /// form_id index, by_form_key, by_signature, by_eid all invalid.
    RecordsAddedOrRemoved,

    /// Masters list mutated. Every FormKey rendering may change.
    MastersChanged,

    /// Header-only edit (file path, next_object_id, flags). No record state moves.
    HeaderOnly,
}

impl PluginIndexSections {
    pub fn invalidate_all(&mut self) {
        self.locator = None;
        self.core = None;
        self.records = None;
        self.form_id_paths = None;
        self.refs = None;
        self.assets = None;
    }

    /// Apply a typed write effect, invalidating only the sections it actually
    /// affects. The four `WriteEffect` variants are exhaustive; new mutation
    /// paths must classify themselves.
    pub fn apply_effect(&mut self, effect: &WriteEffect) {
        match effect {
            WriteEffect::RecordContents { .. } => {
                self.refs = None;
                self.assets = None;
            }
            WriteEffect::RecordsAddedOrRemoved | WriteEffect::MastersChanged => {
                self.invalidate_all();
            }
            WriteEffect::HeaderOnly => {}
        }
    }
}

#[derive(Clone)]
pub struct RecordIndexEntry {
    pub form_key: FormKey,
    pub signature: SmolStr,
    pub eid: String,
    pub defined_in: Arc<str>,
    pub master_plugin: Arc<str>,
    pub is_override: bool,
    pub flags: u32,
    pub object_id: u32,
    pub raw_form_id: u32,
}

#[derive(Clone)]
pub struct RecordLocatorEntry {
    pub form_key: FormKey,
    pub signature: SmolStr,
    pub defined_in: Arc<str>,
    pub master_plugin: Arc<str>,
    pub is_override: bool,
    pub flags: u32,
    pub object_id: u32,
    pub raw_form_id: u32,
    pub path: RecordPath,
}

pub trait FormKeyIndexEntry {
    fn form_key(&self) -> &FormKey;
}

impl FormKeyIndexEntry for RecordIndexEntry {
    fn form_key(&self) -> &FormKey {
        &self.form_key
    }
}

impl FormKeyIndexEntry for RecordLocatorEntry {
    fn form_key(&self) -> &FormKey {
        &self.form_key
    }
}

#[derive(Clone)]
pub struct FormKeyIndex<T> {
    entries: HashMap<FormKey, T>,
    normalized_plugins: HashMap<Arc<str>, Arc<str>>,
}

impl<T> Default for FormKeyIndex<T> {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
            normalized_plugins: HashMap::new(),
        }
    }
}

impl<T> FormKeyIndex<T> {
    pub fn get(&self, form_key: &FormKey) -> Option<&T> {
        if form_key.plugin.is_empty() {
            return self.entries.get(form_key);
        }
        if let Some(plugin) = self.normalized_plugins.get(form_key.plugin.as_ref()) {
            if plugin.as_ref() == form_key.plugin.as_ref() {
                return self.entries.get(form_key);
            }
            return self
                .entries
                .get(&FormKey::new(plugin.clone(), form_key.object_id));
        }
        self.entries.get(&FormKey::new(
            Arc::from(form_key.plugin.to_ascii_lowercase()),
            form_key.object_id,
        ))
    }

    fn intern_normalized_key(&mut self, form_key: &FormKey) -> FormKey {
        if form_key.plugin.is_empty() {
            return form_key.clone();
        }
        if let Some(plugin) = self.normalized_plugins.get(form_key.plugin.as_ref()) {
            return FormKey::new(plugin.clone(), form_key.object_id);
        }
        let normalized_text = form_key.plugin.to_ascii_lowercase();
        let normalized = self
            .normalized_plugins
            .get(normalized_text.as_str())
            .cloned()
            .unwrap_or_else(|| Arc::from(normalized_text.as_str()));
        self.normalized_plugins
            .entry(form_key.plugin.clone())
            .or_insert_with(|| normalized.clone());
        self.normalized_plugins
            .entry(normalized.clone())
            .or_insert_with(|| normalized.clone());
        FormKey::new(normalized, form_key.object_id)
    }

    pub fn insert(&mut self, form_key: FormKey, entry: T) -> Option<T> {
        let normalized = self.intern_normalized_key(&form_key);
        self.entries.insert(normalized, entry)
    }

    pub fn insert_if_absent(&mut self, form_key: FormKey, entry: T) {
        let normalized = self.intern_normalized_key(&form_key);
        self.entries.entry(normalized).or_insert(entry);
    }

    pub fn values(&self) -> impl Iterator<Item = &T> {
        self.entries.values()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

impl<T: FormKeyIndexEntry> FormKeyIndex<T> {
    pub fn keys(&self) -> impl Iterator<Item = &FormKey> {
        self.entries.values().map(FormKeyIndexEntry::form_key)
    }
}

#[derive(Clone, Debug, Eq)]
pub struct FormKey {
    pub plugin: Arc<str>,
    pub object_id: u32,
}

impl FormKey {
    pub fn new(plugin: Arc<str>, object_id: u32) -> Self {
        Self {
            plugin,
            object_id: object_id & 0x00FF_FFFF,
        }
    }

    pub fn raw(raw_form_id: u32) -> Self {
        Self {
            plugin: Arc::from(""),
            object_id: raw_form_id,
        }
    }

    pub fn empty() -> Self {
        Self::raw(0)
    }

    pub fn is_empty(&self) -> bool {
        self.plugin.is_empty() && self.object_id == 0
    }

    pub fn render(&self) -> String {
        if self.plugin.is_empty() {
            if self.object_id == 0 {
                String::new()
            } else {
                format!("{:08X}", self.object_id)
            }
        } else {
            format!("{}:{:06X}", self.plugin, self.object_id)
        }
    }

    pub fn render_arc(&self) -> Arc<str> {
        Arc::from(self.render())
    }
}

impl PartialEq for FormKey {
    fn eq(&self, other: &Self) -> bool {
        self.object_id == other.object_id && self.plugin == other.plugin
    }
}

impl Hash for FormKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.plugin.hash(state);
        self.object_id.hash(state);
    }
}

impl fmt::Display for FormKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.plugin.is_empty() {
            if self.object_id == 0 {
                Ok(())
            } else {
                write!(f, "{:08X}", self.object_id)
            }
        } else {
            write!(f, "{}:{:06X}", self.plugin, self.object_id)
        }
    }
}

#[derive(Clone)]
pub struct AssetPathEntry {
    pub kind: SmolStr,
    pub path: String,
    pub source_subrecord_sig: SmolStr,
}

pub fn build_core_section(plugin: &ParsedPlugin) -> CoreSection {
    let mut core = CoreSection::default();
    let mut all: Vec<&ParsedRecord> = Vec::new();
    let mut predicate = |_record: &ParsedRecord| true;
    collect_records(&plugin.root_items, &mut predicate, &mut all);

    let own_plugin_name: Arc<str> = Arc::from(plugin.plugin_name.as_str());

    for record in all {
        let form_id = record.form_id;
        let object_id = form_id & 0x00FF_FFFF;
        let entry = record_index_entry_for_record(record, &own_plugin_name, &plugin.header.masters);
        let form_key = entry.form_key.clone();

        core.form_ids_by_object_id
            .entry(object_id)
            .or_default()
            .push(form_id);
        core.form_ids_by_signature
            .entry(record.signature.clone())
            .or_default()
            .push(form_id);
        if !entry.eid.is_empty() {
            core.by_eid_lower
                .entry(entry.eid.to_ascii_lowercase())
                .or_default()
                .push(form_key.clone());
        }
        core.by_form_key.insert(form_key.clone(), entry);
        core.by_signature_form_keys
            .entry(record.signature.clone())
            .or_default()
            .push(form_key);
    }

    core
}

pub fn record_index_entry_for_record(
    record: &ParsedRecord,
    own_plugin_name: &Arc<str>,
    masters: &[String],
) -> RecordIndexEntry {
    let form_id = record.form_id;
    let object_id = form_id & 0x00FF_FFFF;
    let (form_key, master_plugin, is_override) =
        form_key_for_record(form_id, own_plugin_name, masters);

    RecordIndexEntry {
        form_key,
        signature: record.signature.clone(),
        eid: editor_id_from_parsed(record).unwrap_or_default(),
        defined_in: own_plugin_name.clone(),
        master_plugin,
        is_override,
        flags: record.flags,
        object_id,
        raw_form_id: form_id,
    }
}

pub fn build_locator_section(plugin: &ParsedPlugin) -> LocatorSection {
    let mut locator = LocatorSection::default();
    let own_plugin_name: Arc<str> = Arc::from(plugin.plugin_name.as_str());
    let mut stack = SmallVec::<[u32; 4]>::new();
    walk_for_locator(
        &plugin.root_items,
        &mut stack,
        &own_plugin_name,
        &plugin.header.masters,
        &mut locator,
    );
    locator
}

fn walk_for_locator(
    items: &[ParsedItem],
    stack: &mut SmallVec<[u32; 4]>,
    own_plugin_name: &Arc<str>,
    masters: &[String],
    out: &mut LocatorSection,
) {
    for (index, item) in items.iter().enumerate() {
        match item {
            ParsedItem::Record(record) => {
                let entry = record_locator_entry_for_record(
                    record,
                    own_plugin_name,
                    masters,
                    RecordPath {
                        group_indices: stack.clone(),
                        record_index: index as u32,
                    },
                );
                out.by_signature_form_keys
                    .entry(entry.signature.clone())
                    .or_default()
                    .push(entry.form_key.clone());
                out.by_form_key
                    .insert_if_absent(entry.form_key.clone(), entry);
            }
            ParsedItem::Group(group) => {
                stack.push(index as u32);
                walk_for_locator(&group.children, stack, own_plugin_name, masters, out);
                stack.pop();
            }
        }
    }
}

pub fn record_locator_entry_for_record(
    record: &ParsedRecord,
    own_plugin_name: &Arc<str>,
    masters: &[String],
    path: RecordPath,
) -> RecordLocatorEntry {
    let form_id = record.form_id;
    let object_id = form_id & 0x00FF_FFFF;
    let (form_key, master_plugin, is_override) =
        form_key_for_record(form_id, own_plugin_name, masters);
    RecordLocatorEntry {
        form_key,
        signature: record.signature.clone(),
        defined_in: own_plugin_name.clone(),
        master_plugin,
        is_override,
        flags: record.flags,
        object_id,
        raw_form_id: form_id,
        path,
    }
}

pub fn locator_entry_by_form_key<'a>(
    locator: &'a LocatorSection,
    form_key: &str,
) -> Option<&'a RecordLocatorEntry> {
    let query = form_key.trim();
    if let Some(normalized) = normalize_form_key(query) {
        return locator.by_form_key.get(&normalized);
    }
    None
}

pub fn build_records_section(plugin: &ParsedPlugin) -> RecordsSection {
    RecordsSection {
        locator: build_form_id_paths_section(plugin).by_form_id,
    }
}

pub fn build_form_id_paths_section(plugin: &ParsedPlugin) -> FormIdPathsSection {
    let mut by_form_id = FxHashMap::default();
    let mut path = SmallVec::<[u32; 4]>::new();
    walk_for_paths(&plugin.root_items, &mut path, &mut by_form_id);
    FormIdPathsSection { by_form_id }
}

fn walk_for_paths(
    items: &[ParsedItem],
    path: &mut SmallVec<[u32; 4]>,
    out: &mut FxHashMap<u32, RecordPath>,
) {
    for (index, item) in items.iter().enumerate() {
        match item {
            ParsedItem::Record(record) => {
                out.entry(record.form_id).or_insert_with(|| RecordPath {
                    group_indices: path.clone(),
                    record_index: index as u32,
                });
            }
            ParsedItem::Group(group) => {
                path.push(index as u32);
                walk_for_paths(&group.children, path, out);
                path.pop();
            }
        }
    }
}

/// Subrecords to read for a record: borrows the already-parsed ones for the
/// common (uncompressed) case — no per-record Vec clone — and only allocates
/// when a compressed record must be lazily decompressed. Callers that store the
/// result back into a record call `.into_owned()`; read-only callers iterate
/// the borrow directly.
pub fn effective_subrecords_for_record(record: &ParsedRecord) -> Cow<'_, [ParsedSubrecord]> {
    if !record.subrecords.is_empty() {
        return Cow::Borrowed(&record.subrecords);
    }
    Cow::Owned(
        lazy_subrecords_for_record(record)
            .ok()
            .flatten()
            .unwrap_or_default(),
    )
}

pub fn editor_id_from_effective_subrecords(subrecords: &[ParsedSubrecord]) -> String {
    for subrecord in subrecords {
        if subrecord.signature.as_str() == "EDID" {
            return decode_cp1252(&subrecord.data);
        }
    }
    String::new()
}

pub fn build_refs_section(plugin: &ParsedPlugin) -> RefsSection {
    let mut refs = RefsSection::default();
    let mut all: Vec<&ParsedRecord> = Vec::new();
    let mut predicate = |_record: &ParsedRecord| true;
    collect_records(&plugin.root_items, &mut predicate, &mut all);

    let own_plugin_name: Arc<str> = Arc::from(plugin.plugin_name.as_str());

    // Resolve once for the whole plugin; used by the schema-aware nested FK
    // extractor to walk compound subrecord layouts (e.g. WEAP/ARMO OBTS
    // includes[].mod, OMOD properties). When game is unknown the extractor
    // falls back to flat-only behavior.
    let schema = plugin
        .game
        .as_deref()
        .and_then(|game| compiled_schema_for_game(game).ok());
    let schema_ref = schema.as_deref();

    for record in all {
        let form_id = record.form_id;
        let (form_key, _, _) =
            form_key_for_record(form_id, &own_plugin_name, &plugin.header.masters);

        let referenced_form_ids = iter_referenced_form_ids_by_subrecord(record, schema_ref);
        let mut seen_form_keys = HashSet::new();
        let mut forward_refs = Vec::new();
        for (_, raw_ref_form_id) in &referenced_form_ids {
            let target_fk = resolve_form_id_to_form_key(
                *raw_ref_form_id,
                &own_plugin_name,
                &plugin.header.masters,
            );
            if target_fk.is_empty() || !seen_form_keys.insert(target_fk.clone()) {
                continue;
            }
            forward_refs.push(target_fk);
        }
        let mut refs_by_subrecord: HashMap<SmolStr, Vec<FormKey>> = HashMap::new();
        for (subrecord_sig, raw_ref_form_id) in referenced_form_ids {
            let target_fk = resolve_form_id_to_form_key(
                raw_ref_form_id,
                &own_plugin_name,
                &plugin.header.masters,
            );
            if target_fk.is_empty() {
                continue;
            }
            let refs_for_subrecord = refs_by_subrecord.entry(subrecord_sig).or_default();
            if !refs_for_subrecord
                .iter()
                .any(|existing| existing == &target_fk)
            {
                refs_for_subrecord.push(target_fk);
            }
        }

        refs.forward_refs_by_form_key
            .insert(form_key.clone(), forward_refs.clone());
        for (subrecord_sig, subrecord_refs) in refs_by_subrecord {
            refs.refs_by_form_key_and_subrecord
                .insert((form_key.clone(), subrecord_sig), subrecord_refs);
        }
        for target_fk in forward_refs {
            refs.reverse_refs_by_form_key
                .entry(target_fk)
                .or_default()
                .push(form_key.clone());
        }
    }

    refs
}

pub fn build_assets_section(plugin: &ParsedPlugin) -> AssetsSection {
    let mut assets = AssetsSection::default();
    let mut all: Vec<&ParsedRecord> = Vec::new();
    let mut predicate = |_record: &ParsedRecord| true;
    collect_records(&plugin.root_items, &mut predicate, &mut all);

    let own_plugin_name: Arc<str> = Arc::from(plugin.plugin_name.as_str());

    for record in all {
        let (form_key, _, _) =
            form_key_for_record(record.form_id, &own_plugin_name, &plugin.header.masters);
        let asset_paths = extract_asset_paths(record);
        for asset in &asset_paths {
            assets
                .assets_by_kind
                .entry(asset.kind.clone())
                .or_default()
                .push((form_key.render_arc(), asset.path.clone()));
        }
        if !asset_paths.is_empty() {
            assets
                .asset_paths_by_form_key
                .insert(form_key.render_arc(), asset_paths);
        }
    }

    assets
}

pub fn resolve_form_id_to_form_key(
    raw_form_id: u32,
    own_plugin_name: &Arc<str>,
    masters: &[String],
) -> FormKey {
    let raw_form_id = raw_form_id & 0xFFFF_FFFF;
    if raw_form_id == 0 {
        return FormKey::empty();
    }
    let master_index = ((raw_form_id >> 24) & 0xFF) as u8;
    let object_id = raw_form_id & 0x00FF_FFFF;
    let own_index = (masters.len() & 0xFF) as u8;
    let plugin_name = if master_index == LOCAL_FORM_INDEX || master_index == own_index {
        own_plugin_name.clone()
    } else if (master_index as usize) < masters.len() {
        Arc::from(masters[master_index as usize].as_str())
    } else {
        return FormKey::raw(raw_form_id);
    };
    FormKey::new(plugin_name, object_id)
}

fn form_key_for_record(
    raw_form_id: u32,
    own_plugin_name: &Arc<str>,
    masters: &[String],
) -> (FormKey, Arc<str>, bool) {
    let master_index = ((raw_form_id >> 24) & 0xFF) as u8;
    let own_index = (masters.len() & 0xFF) as u8;
    let is_override = master_index != LOCAL_FORM_INDEX
        && master_index != own_index
        && (master_index as usize) < masters.len();
    let master_plugin = if is_override {
        Arc::from(masters[master_index as usize].as_str())
    } else {
        own_plugin_name.clone()
    };
    let form_key = if master_index != LOCAL_FORM_INDEX
        && master_index != own_index
        && (master_index as usize) >= masters.len()
    {
        FormKey::raw(raw_form_id)
    } else {
        FormKey::new(master_plugin.clone(), raw_form_id & 0x00FF_FFFF)
    };
    (form_key, master_plugin, is_override)
}

pub fn normalize_form_key(value: &str) -> Option<FormKey> {
    let value = value.trim();
    if value.len() == 8 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        let raw_form_id = u32::from_str_radix(value, 16).ok()?;
        return (raw_form_id != 0).then(|| FormKey::raw(raw_form_id));
    }
    let (plugin_name, object_id_text) = value.rsplit_once(':')?;
    let plugin_name = plugin_name.trim();
    if plugin_name.is_empty() {
        return None;
    }
    let object_id = u32::from_str_radix(
        object_id_text
            .trim()
            .trim_start_matches("0x")
            .trim_start_matches("0X"),
        16,
    )
    .ok()?
        & 0x00FF_FFFF;
    Some(FormKey::new(Arc::from(plugin_name), object_id))
}

pub fn form_key_matches(indexed: &FormKey, query: &str) -> bool {
    let Some(query_normalized) = normalize_form_key(query) else {
        return indexed.render().eq_ignore_ascii_case(query);
    };
    indexed
        .plugin
        .eq_ignore_ascii_case(&query_normalized.plugin)
        && indexed.object_id == query_normalized.object_id
}

pub fn record_index_entry_by_form_key<'a>(
    core: &'a CoreSection,
    form_key: &str,
) -> Option<&'a RecordIndexEntry> {
    let query = form_key.trim();
    if let Some(normalized) = normalize_form_key(query) {
        return core.by_form_key.get(&normalized);
    }
    None
}

pub fn form_key_refs<'a>(
    refs_by_form_key: &'a HashMap<FormKey, Vec<FormKey>>,
    form_key: &str,
) -> Option<&'a Vec<FormKey>> {
    let query = form_key.trim();
    if let Some(normalized) = normalize_form_key(query) {
        if let Some(values) = refs_by_form_key.get(&normalized) {
            return Some(values);
        }
    }
    refs_by_form_key
        .iter()
        .find_map(|(key, values)| form_key_matches(key, query).then_some(values))
}

pub fn iter_referenced_form_ids_by_subrecord(
    record: &ParsedRecord,
    schema: Option<&CompiledSchema>,
) -> Vec<(SmolStr, u32)> {
    let subrecords = effective_subrecords_for_record(record);
    iter_referenced_form_ids_from_subrecords(record.signature.as_str(), &subrecords, schema)
}

pub fn iter_referenced_form_ids_from_subrecords(
    record_signature: &str,
    subrecords: &[ParsedSubrecord],
    schema: Option<&CompiledSchema>,
) -> Vec<(SmolStr, u32)> {
    let mut form_ids = Vec::new();
    let record_spec = schema.and_then(|s| schema_record_spec(s, record_signature));
    let mut occurrence_counts: HashMap<SmolStr, usize> = HashMap::new();

    for subrecord in subrecords {
        // Track occurrence index for every subrecord — schema_subrecord_spec
        // resolves repeating sigs by N-th appearance, so we must count even
        // the ones the flat checks consume.
        let occurrence = {
            let counter = occurrence_counts
                .entry(subrecord.signature.clone())
                .or_insert(0);
            let value = *counter;
            *counter += 1;
            value
        };

        if subrecord.semantic_type.as_deref() == Some("formid") && subrecord.data.len() >= 4 {
            form_ids.push((
                subrecord.signature.clone(),
                u32::from_le_bytes([
                    subrecord.data[0],
                    subrecord.data[1],
                    subrecord.data[2],
                    subrecord.data[3],
                ]),
            ));
            continue;
        }
        if subrecord.semantic_type.as_deref() == Some("formid_array")
            && !subrecord.data.is_empty()
            && subrecord.data.len() % 4 == 0
        {
            for chunk in subrecord.data.chunks_exact(4) {
                form_ids.push((
                    subrecord.signature.clone(),
                    u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]),
                ));
            }
            continue;
        }
        if subrecord.signature.as_str() == "LVLO" {
            for raw_ref_form_id in lvlo_referenced_form_ids(subrecord.data.as_ref()) {
                form_ids.push((subrecord.signature.clone(), raw_ref_form_id));
            }
            continue;
        }
        if subrecord.data.len() == 4 && is_known_formid_subrecord(subrecord.signature.as_str()) {
            form_ids.push((
                subrecord.signature.clone(),
                u32::from_le_bytes([
                    subrecord.data[0],
                    subrecord.data[1],
                    subrecord.data[2],
                    subrecord.data[3],
                ]),
            ));
            continue;
        }
        if !subrecord.data.is_empty()
            && subrecord.data.len() % 4 == 0
            && is_known_formid_array_subrecord(subrecord.signature.as_str())
        {
            for chunk in subrecord.data.chunks_exact(4) {
                form_ids.push((
                    subrecord.signature.clone(),
                    u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]),
                ));
            }
            continue;
        }

        // Schema-aware deep extraction for FormIDs nested inside compound
        // subrecord layouts (WEAP/ARMO OBTS includes[].mod → OMOD, OMOD
        // properties referencing keywords/MSWP, etc.).
        if let (Some(schema), Some(record_spec)) = (schema, record_spec) {
            if let Some(sub_spec) =
                schema_subrecord_spec(record_spec, subrecord.signature.as_str(), occurrence)
            {
                let mut nested = Vec::new();
                crate::plugin_runtime::authoring::authoring_serialize::extract_nested_form_ids(
                    sub_spec,
                    schema,
                    &subrecord.data,
                    &mut nested,
                );
                for value in nested {
                    form_ids.push((subrecord.signature.clone(), value));
                }
            }
        }
    }
    form_ids
        .into_iter()
        .filter(|(_, value)| *value != 0)
        .collect()
}

fn lvlo_referenced_form_ids(data: &[u8]) -> Vec<u32> {
    if data.len() == 4 {
        return vec![u32::from_le_bytes([data[0], data[1], data[2], data[3]])];
    }
    if data.len() >= 12 && data.len() % 12 == 0 {
        return data
            .chunks_exact(12)
            .map(|chunk| u32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]))
            .collect();
    }
    Vec::new()
}

/// In-place mutating twin of [`iter_referenced_form_ids_from_subrecords`].
///
/// Walks the same FormID locations — `semantic_type` formids, LVLO entries, the
/// known-FormID-subrecord allowlist, and schema-declared nested FormIDs — and
/// applies `rewrite` to each: it returns `Some(new_raw)` to overwrite the FormID
/// in place, or `None` to leave it untouched. Returns true if any byte changed.
///
/// This is what makes object-id remapping (ESL compaction, renumber, inject)
/// correct on *disk-loaded* plugins, whose subrecords carry no `semantic_type`
/// and are therefore invisible to the semantic-only fast path. Branch order
/// mirrors the read twin exactly so the two never disagree on what is a FormID.
pub fn rewrite_referenced_form_ids_in_subrecords(
    record_signature: &str,
    subrecords: &mut [ParsedSubrecord],
    schema: Option<&CompiledSchema>,
    rewrite: &mut dyn FnMut(u32) -> Option<u32>,
) -> bool {
    let record_spec = schema.and_then(|s| schema_record_spec(s, record_signature));
    let mut occurrence_counts: HashMap<SmolStr, usize> = HashMap::new();
    let mut changed = false;

    for subrecord in subrecords.iter_mut() {
        let occurrence = {
            let counter = occurrence_counts
                .entry(subrecord.signature.clone())
                .or_insert(0);
            let value = *counter;
            *counter += 1;
            value
        };

        if subrecord.semantic_type.as_deref() == Some("formid") && subrecord.data.len() >= 4 {
            changed |= rewrite_formid_bytes_at(&mut subrecord.data, 0, rewrite);
            continue;
        }
        if subrecord.semantic_type.as_deref() == Some("formid_array")
            && !subrecord.data.is_empty()
            && subrecord.data.len() % 4 == 0
        {
            changed |= rewrite_formid_array_bytes(&mut subrecord.data, rewrite);
            continue;
        }
        if subrecord.signature.as_str() == "LVLO" {
            changed |= rewrite_lvlo_form_id_bytes(&mut subrecord.data, rewrite);
            continue;
        }
        if subrecord.data.len() == 4 && is_known_formid_subrecord(subrecord.signature.as_str()) {
            changed |= rewrite_formid_bytes_at(&mut subrecord.data, 0, rewrite);
            continue;
        }
        if !subrecord.data.is_empty()
            && subrecord.data.len() % 4 == 0
            && is_known_formid_array_subrecord(subrecord.signature.as_str())
        {
            changed |= rewrite_formid_array_bytes(&mut subrecord.data, rewrite);
            continue;
        }

        if let (Some(schema), Some(record_spec)) = (schema, record_spec) {
            if let Some(sub_spec) =
                schema_subrecord_spec(record_spec, subrecord.signature.as_str(), occurrence)
            {
                let mut buf = subrecord.data.to_vec();
                if crate::plugin_runtime::authoring::authoring_serialize::rewrite_schema_form_ids_in_subrecord(
                    sub_spec,
                    schema,
                    &mut buf,
                    rewrite,
                ) {
                    subrecord.data = Bytes::from(buf);
                    changed = true;
                }
            }
        }
    }
    changed
}

fn rewrite_formid_bytes_at(
    data: &mut Bytes,
    offset: usize,
    rewrite: &mut dyn FnMut(u32) -> Option<u32>,
) -> bool {
    if data.len() < offset + 4 {
        return false;
    }
    let raw = u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ]);
    match rewrite(raw) {
        Some(new) if new != raw => {
            let mut buf = data.to_vec();
            buf[offset..offset + 4].copy_from_slice(&new.to_le_bytes());
            *data = Bytes::from(buf);
            true
        }
        _ => false,
    }
}

fn rewrite_formid_array_bytes(
    data: &mut Bytes,
    rewrite: &mut dyn FnMut(u32) -> Option<u32>,
) -> bool {
    if data.is_empty() || data.len() % 4 != 0 {
        return false;
    }
    let mut buf: Option<Vec<u8>> = None;
    for (index, chunk) in data.chunks_exact(4).enumerate() {
        let raw = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        if let Some(new) = rewrite(raw) {
            if new != raw {
                let owned = buf.get_or_insert_with(|| data.to_vec());
                owned[index * 4..index * 4 + 4].copy_from_slice(&new.to_le_bytes());
            }
        }
    }
    match buf {
        Some(owned) => {
            *data = Bytes::from(owned);
            true
        }
        None => false,
    }
}

fn rewrite_lvlo_form_id_bytes(
    data: &mut Bytes,
    rewrite: &mut dyn FnMut(u32) -> Option<u32>,
) -> bool {
    // Mirror lvlo_referenced_form_ids: a 4-byte payload is a lone FormID; a
    // 12-byte-stride payload carries the FormID at offset 4 of each entry.
    if data.len() == 4 {
        return rewrite_formid_bytes_at(data, 0, rewrite);
    }
    if data.len() >= 12 && data.len() % 12 == 0 {
        let mut buf: Option<Vec<u8>> = None;
        for entry in 0..(data.len() / 12) {
            let offset = entry * 12 + 4;
            let raw = u32::from_le_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ]);
            if let Some(new) = rewrite(raw) {
                if new != raw {
                    let owned = buf.get_or_insert_with(|| data.to_vec());
                    owned[offset..offset + 4].copy_from_slice(&new.to_le_bytes());
                }
            }
        }
        if let Some(owned) = buf {
            *data = Bytes::from(owned);
            return true;
        }
    }
    false
}

/// Pick the form_id within `form_ids` that belongs to the plugin's own
/// records (master byte == own_index or 0xFF), falling back to the first.
pub fn pick_owned_form_id(form_ids: &[u32], own_index: u8) -> Option<u32> {
    for fid in form_ids {
        let idx_byte = ((fid >> 24) & 0xFF) as u8;
        if idx_byte == LOCAL_FORM_INDEX || idx_byte == own_index {
            return Some(*fid);
        }
    }
    form_ids.first().copied()
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

    fn make_record(index: usize) -> ParsedItem {
        ParsedItem::Record(ParsedRecord {
            signature: SmolStr::new("MISC"),
            form_id: 0xFF00_0800 + index as u32,
            flags: 0,
            version_control: 0,
            form_version: None,
            version2: None,
            subrecords: Vec::new(),
            raw_payload: None,
            parse_error: None,
        })
    }

    fn subrecord(signature: &str, data: Vec<u8>) -> ParsedSubrecord {
        ParsedSubrecord {
            signature: SmolStr::new(signature),
            data: Bytes::from(data),
            semantic_type: None,
        }
    }

    #[test]
    fn references_include_fo76_lvlo_four_byte_formids() {
        let record = ParsedRecord {
            signature: SmolStr::new("LVLN"),
            form_id: 0xFF00_0800,
            flags: 0,
            version_control: 0,
            form_version: None,
            version2: None,
            subrecords: vec![subrecord("LVLO", 0xFF12_3456_u32.to_le_bytes().to_vec())],
            raw_payload: None,
            parse_error: None,
        };

        assert_eq!(
            iter_referenced_form_ids_by_subrecord(&record, None),
            vec![(SmolStr::new("LVLO"), 0xFF12_3456)]
        );
    }

    #[test]
    fn references_include_twelve_byte_lvlo_formids() {
        let mut entry = Vec::new();
        entry.extend_from_slice(&1_u32.to_le_bytes());
        entry.extend_from_slice(&0xFF65_4321_u32.to_le_bytes());
        entry.extend_from_slice(&1_u32.to_le_bytes());
        let record = ParsedRecord {
            signature: SmolStr::new("LVLN"),
            form_id: 0xFF00_0800,
            flags: 0,
            version_control: 0,
            form_version: None,
            version2: None,
            subrecords: vec![subrecord("LVLO", entry)],
            raw_payload: None,
            parse_error: None,
        };

        assert_eq!(
            iter_referenced_form_ids_by_subrecord(&record, None),
            vec![(SmolStr::new("LVLO"), 0xFF65_4321)]
        );
    }

    #[test]
    fn weap_top_level_formid_subrecord_is_followed_as_forward_ref() {
        // Regression: WEAP support subrecords that are top-level `codec: "formid"`
        // (BIDS->IPDS, and siblings like the AnimationSound/PreviewTransform refs)
        // must be extracted as forward edges so the dependency walk reaches and
        // translates them. ETYP works only because it is in KNOWN_FORMID_SUBRECORDS;
        // BIDS is not, so it relies on the schema-driven path.
        let schema = compiled_schema_for_game_str("fo76").expect("fo76 schema");
        let record = ParsedRecord {
            signature: SmolStr::new("WEAP"),
            form_id: 0xFF00_54A1,
            flags: 0,
            version_control: 0,
            form_version: Some(131),
            version2: None,
            subrecords: vec![subrecord("BIDS", 0x0001_83FF_u32.to_le_bytes().to_vec())],
            raw_payload: None,
            parse_error: None,
        };

        let refs = iter_referenced_form_ids_by_subrecord(&record, Some(&schema));
        assert!(
            refs.iter().any(|(_, value)| *value == 0x0001_83FF),
            "WEAP BIDS (Block Bash Impact Data Set -> IPDS) formid not extracted: {refs:?}"
        );
    }

    #[test]
    fn locator_section_indexes_form_keys_without_editor_ids() {
        let plugin = ParsedPlugin {
            plugin_name: "Locator.esp".to_string(),
            file_path: String::new(),
            header_size: MODERN_HEADER_SIZE,
            header: empty_header(),
            root_items: vec![ParsedItem::Record(ParsedRecord {
                signature: SmolStr::new("MISC"),
                form_id: 0xFF00_0800,
                flags: 0,
                version_control: 0,
                form_version: None,
                version2: None,
                subrecords: vec![subrecord("EDID", b"ShouldNotBeIndexed\0".to_vec())],
                raw_payload: None,
                parse_error: None,
            })],
            game: Some("fo4".to_string()),
        };

        let locator = build_locator_section(&plugin);
        let entry =
            locator_entry_by_form_key(&locator, "Locator.esp:000800").expect("locator entry");

        assert_eq!(entry.signature.as_str(), "MISC");
        assert_eq!(entry.raw_form_id, 0xFF00_0800);
    }

    #[test]
    fn mixed_case_lookup_hits_normalized_index_without_fallback() {
        let plugin = ParsedPlugin {
            plugin_name: "SeventySix.esm".to_string(),
            file_path: String::new(),
            header_size: MODERN_HEADER_SIZE,
            header: empty_header(),
            root_items: vec![make_record(0), make_record(1)],
            game: Some("fo76".to_string()),
        };

        let core = build_core_section(&plugin);
        let normalized = FormKey::new(Arc::from("seventysix.esm"), 0x800);
        let original = FormKey::new(Arc::from("SeventySix.esm"), 0x800);

        assert!(core.by_form_key.entries.contains_key(&normalized));
        assert!(!core.by_form_key.entries.contains_key(&original));
        assert!(Arc::ptr_eq(
            core.by_form_key
                .normalized_plugins
                .get("SeventySix.esm")
                .expect("original-case alias"),
            core.by_form_key
                .normalized_plugins
                .get("seventysix.esm")
                .expect("normalized plugin")
        ));
        let entry = record_index_entry_by_form_key(&core, "seventysix.ESM:000800")
            .expect("mixed-case lookup must hit the normalized key directly");
        assert_eq!(entry.form_key.render(), "SeventySix.esm:000800");
        assert_eq!(core.by_form_key.len(), 2);
    }

    #[test]
    fn raw_form_key_queries_hit_core_and_locator_indexes() {
        let raw_form_id = 0x0100_0800;
        let plugin = ParsedPlugin {
            plugin_name: "RawLookup.esp".to_string(),
            file_path: String::new(),
            header_size: MODERN_HEADER_SIZE,
            header: empty_header(),
            root_items: vec![ParsedItem::Record(ParsedRecord {
                signature: SmolStr::new("MISC"),
                form_id: raw_form_id,
                flags: 0,
                version_control: 0,
                form_version: None,
                version2: None,
                subrecords: Vec::new(),
                raw_payload: None,
                parse_error: None,
            })],
            game: Some("fo4".to_string()),
        };

        let query = FormKey::raw(raw_form_id).render();
        let core = build_core_section(&plugin);
        let locator = build_locator_section(&plugin);

        let core_entry =
            record_index_entry_by_form_key(&core, &query).expect("raw core index entry");
        let locator_entry =
            locator_entry_by_form_key(&locator, &query).expect("raw locator index entry");
        assert_eq!(core_entry.form_key, FormKey::raw(raw_form_id));
        assert_eq!(locator_entry.form_key, FormKey::raw(raw_form_id));
        assert_eq!(core_entry.raw_form_id, raw_form_id);
        assert_eq!(locator_entry.raw_form_id, raw_form_id);
    }

    #[test]
    fn records_section_handles_wide_root_item_lists() {
        let target_index = u16::MAX as usize + 1;
        let target_form_id = 0xFF00_0800 + target_index as u32;
        let plugin = ParsedPlugin {
            plugin_name: "WideRoot.esp".to_string(),
            file_path: String::new(),
            header_size: MODERN_HEADER_SIZE,
            header: empty_header(),
            root_items: (0..=target_index).map(make_record).collect(),
            game: Some("fo4".to_string()),
        };

        let records = build_records_section(&plugin);

        assert_eq!(
            records
                .record(&plugin, target_form_id)
                .map(|record| record.form_id),
            Some(target_form_id)
        );
    }
}

#[cfg(test)]
mod form_id_paths_tests {
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

    fn rec(form_id: u32) -> ParsedItem {
        ParsedItem::Record(ParsedRecord {
            form_id,
            signature: SmolStr::new("WEAP"),
            flags: 0,
            version_control: 0,
            form_version: None,
            version2: None,
            subrecords: Vec::new(),
            raw_payload: None,
            parse_error: None,
        })
    }

    #[test]
    fn paths_section_finds_root_level_records() {
        let plugin = ParsedPlugin {
            plugin_name: "Paths.esp".to_string(),
            file_path: String::new(),
            header_size: MODERN_HEADER_SIZE,
            header: empty_header(),
            root_items: vec![rec(0x0000_0800), rec(0x0000_0801)],
            game: Some("fo4".to_string()),
        };

        let section = build_form_id_paths_section(&plugin);
        let p0 = section.by_form_id.get(&0x0000_0800).expect("0x800 indexed");
        assert!(p0.group_indices.is_empty());
        assert_eq!(p0.record_index, 0);

        let p1 = section.by_form_id.get(&0x0000_0801).expect("0x801 indexed");
        assert!(p1.group_indices.is_empty());
        assert_eq!(p1.record_index, 1);
    }

    #[test]
    fn paths_section_finds_nested_group_records() {
        let plugin = ParsedPlugin {
            plugin_name: "Paths.esp".to_string(),
            file_path: String::new(),
            header_size: MODERN_HEADER_SIZE,
            header: empty_header(),
            root_items: vec![ParsedItem::Group(ParsedGroup {
                label: *b"WEAP",
                group_type: 0,
                tail: Bytes::new(),
                children: vec![rec(0x0000_0900), rec(0x0000_0901)],
            })],
            game: Some("fo4".to_string()),
        };

        let section = build_form_id_paths_section(&plugin);
        let p = section.by_form_id.get(&0x0000_0901).expect("0x901 indexed");
        assert_eq!(p.group_indices.as_slice(), &[0u32]);
        assert_eq!(p.record_index, 1);
    }
}

#[cfg(test)]
mod write_effect_tests {
    use super::*;
    use std::sync::Arc;

    fn make_sections_all_populated() -> PluginIndexSections {
        PluginIndexSections {
            locator: Some(Arc::new(LocatorSection::default())),
            core: Some(Arc::new(CoreSection::default())),
            records: Some(Arc::new(RecordsSection::default())),
            form_id_paths: Some(Arc::new(FormIdPathsSection::default())),
            refs: Some(Arc::new(RefsSection::default())),
            assets: Some(Arc::new(AssetsSection::default())),
        }
    }

    #[test]
    fn record_contents_effect_invalidates_content_derived_sections() {
        let mut sections = make_sections_all_populated();
        sections.apply_effect(&WriteEffect::RecordContents {
            form_ids: smallvec::smallvec![0x0000_0800],
        });
        assert!(sections.locator.is_some());
        assert!(sections.core.is_some());
        assert!(sections.records.is_some());
        assert!(sections.form_id_paths.is_some());
        assert!(sections.refs.is_none());
        assert!(sections.assets.is_none());
    }

    #[test]
    fn records_added_or_removed_invalidates_all() {
        let mut sections = make_sections_all_populated();
        sections.apply_effect(&WriteEffect::RecordsAddedOrRemoved);
        assert!(sections.locator.is_none());
        assert!(sections.core.is_none());
        assert!(sections.records.is_none());
        assert!(sections.form_id_paths.is_none());
        assert!(sections.refs.is_none());
        assert!(sections.assets.is_none());
    }

    #[test]
    fn masters_changed_invalidates_all() {
        let mut sections = make_sections_all_populated();
        sections.apply_effect(&WriteEffect::MastersChanged);
        assert!(sections.locator.is_none());
        assert!(sections.core.is_none());
        assert!(sections.records.is_none());
        assert!(sections.form_id_paths.is_none());
        assert!(sections.refs.is_none());
        assert!(sections.assets.is_none());
    }

    #[test]
    fn header_only_invalidates_nothing() {
        let mut sections = make_sections_all_populated();
        sections.apply_effect(&WriteEffect::HeaderOnly);
        assert!(sections.locator.is_some());
        assert!(sections.core.is_some());
        assert!(sections.records.is_some());
        assert!(sections.form_id_paths.is_some());
        assert!(sections.refs.is_some());
        assert!(sections.assets.is_some());
    }
}
