use bytes::Bytes;
use encoding_rs::WINDOWS_1252;
use flate2::Compression;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use pyo3::IntoPyObjectExt;
use pyo3::exceptions::{PyIOError, PyKeyError, PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyByteArray, PyBytes, PyList, PyModule, PyTuple};
use serde::Deserialize;
use serde_json::{Map as JsonMap, Value as JsonValue};
use sha2::{Digest, Sha256};
use smol_str::SmolStr;
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::fs::File;
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex, OnceLock,
    atomic::{AtomicU64, Ordering},
};

const LEGACY_HEADER_SIZE: usize = 20;
const MODERN_HEADER_SIZE: usize = 24;
/// Record-header COMPRESSED flag (bit 18). Single source of truth — other
/// crates that need to test or set this flag import it from
/// `esp_authoring_core::plugin_runtime::COMPRESSED_RECORD_FLAG`.
pub const COMPRESSED_RECORD_FLAG: u32 = 1 << 18;
const LOCAL_FORM_INDEX: u8 = 0xFF;
const TES4_FLAG_LOCALIZED: u32 = 0x00000080;
const RECORD_FILE_SUFFIXES: [&str; 3] = ["json", "yaml", "yml"];
const GROUP_DIR_PREFIX: &str = "__group_";
const RECORD_DATA_STEM: &str = "RecordData";
const GROUP_RECORD_DATA_STEM: &str = "GroupRecordData";
const INTERIOR_CELL_BLOCK: i32 = 2;
const INTERIOR_CELL_SUBBLOCK: i32 = 3;
const EXTERIOR_CELL_BLOCK: i32 = 4;
const EXTERIOR_CELL_SUBBLOCK: i32 = 5;
const CELL_CHILD_GROUP: i32 = 6;
const TOPIC_CHILD_GROUP: i32 = 7;
const PERSISTENT_GROUP: i32 = 8;
const TEMPORARY_GROUP: i32 = 9;
const VISIBLE_DISTANT_GROUP: i32 = 10;
const QUEST_CHILD_GROUP: i32 = 10;
const SCHEMA_FORGE_LARGE_SAMPLE_THRESHOLD: usize = 64 * 1024;
const SCHEMA_FORGE_LARGE_SAMPLE_TRUNCATE: usize = 4 * 1024;
const DEFAULT_SYNTHETIC_OBJECT_ID: u32 = 0x0000_0800;
const NAVI_ISLAND_TRIANGLE_LIMIT: usize = 240;
const NAVI_ISLAND_VERTEX_LIMIT: usize = 240;
const NAVI_NVMI_FLAG_IS_ISLAND: u32 = 0x20;
pub const FO4_CANONICAL_NAVI_FORM_ID: u32 = 0x0000_0FF1;
pub const FO4_PATHING_CELL_CRC_HASH: u32 = 0xA5E9_A03C;
const RECORD_FLAG_DELETED: u32 = 0x0000_0020;
const RECORD_FLAG_INITIALLY_DISABLED: u32 = 0x0000_0800;
const PLAYER_FORM_ID: u32 = 0x0000_0014;
const LVLO_STRIDE: usize = 12;
const LVLO_FORMID_OFFSET: usize = 4;

#[path = "model.rs"]
mod model;
pub use model::*;
#[path = "asset_index.rs"]
mod asset_index;
pub use asset_index::*;
#[path = "plugin_index.rs"]
mod plugin_index;
pub use plugin_index::*;
#[path = "master_edit.rs"]
mod master_edit;
#[path = "cell_slice.rs"]
mod cell_slice;
pub use cell_slice::*;
#[path = "marker_type.rs"]
mod marker_type;
pub use marker_type::*;
#[path = "worldspace_header.rs"]
mod worldspace_header;
pub use worldspace_header::*;
#[path = "worldspace_offsets.rs"]
mod worldspace_offsets;
pub use worldspace_offsets::*;
#[path = "asset_collect.rs"]
mod asset_collect;
pub use asset_collect::*;
#[path = "walker_policy.rs"]
mod walker_policy;
pub use walker_policy::*;
#[path = "walker.rs"]
mod walker;
pub use walker::*;
#[path = "import_py.rs"]
mod import_py;
pub use import_py::*;
#[path = "export_py.rs"]
mod export_py;
pub use export_py::*;
#[path = "schema.rs"]
mod schema;
pub use schema::*;
#[path = "condition_functions.rs"]
pub mod condition_functions;
#[path = "text_payload_py.rs"]
mod text_payload_py;
pub use text_payload_py::*;
#[path = "authoring_dir.rs"]
mod authoring_dir;
pub use authoring_dir::*;
#[path = "authoring_validate.rs"]
mod authoring_validate;
pub use authoring_validate::*;
#[path = "io.rs"]
mod io;
pub use io::*;
#[path = "authoring.rs"]
pub mod authoring;
#[path = "codec_constants.rs"]
pub mod codec_constants;
#[path = "../generated/group_order.rs"]
mod runtime_group_order;
#[path = "strings.rs"]
pub mod strings;

fn value_error(message: impl Into<String>) -> PyErr {
    PyValueError::new_err(message.into())
}

fn io_error(message: impl Into<String>) -> PyErr {
    PyIOError::new_err(message.into())
}

fn clone_subrecord_from_python(
    py: Python<'_>,
    subrecord: &Bound<'_, PyAny>,
) -> PyResult<ParsedSubrecord> {
    Ok(ParsedSubrecord {
        signature: SmolStr::new(subrecord.getattr("signature")?.extract::<String>()?),
        data: Bytes::from(bytes_like_to_vec_owned(py, subrecord.getattr("data")?)?),
        semantic_type: optional_string_owned(subrecord.getattr("semantic_type")?)?,
    })
}

fn clone_subrecord_from_python_owned(
    py: Python<'_>,
    subrecord: Bound<'_, PyAny>,
) -> PyResult<ParsedSubrecord> {
    clone_subrecord_from_python(py, &subrecord)
}

pub fn clone_record_from_python(
    py: Python<'_>,
    record: &Bound<'_, PyAny>,
) -> PyResult<ParsedRecord> {
    let mut subrecords = Vec::new();
    for subrecord in record.getattr("subrecords")?.try_iter()? {
        subrecords.push(clone_subrecord_from_python_owned(py, subrecord?)?);
    }
    Ok(ParsedRecord {
        signature: SmolStr::new(record.getattr("signature")?.extract::<String>()?),
        form_id: record.getattr("form_id")?.extract::<u32>()?,
        flags: record.getattr("flags")?.extract::<u32>()?,
        version_control: record.getattr("version_control")?.extract::<u32>()?,
        form_version: match record.getattr("form_version")? {
            value if value.is_none() => None,
            value => Some(value.extract::<u16>()?),
        },
        version2: match record.getattr("version2")? {
            value if value.is_none() => None,
            value => Some(value.extract::<u16>()?),
        },
        subrecords,
        raw_payload: match record.getattr("raw_payload")? {
            value if value.is_none() => None,
            value => Some(Bytes::from(bytes_like_to_vec_owned(py, value)?)),
        },
        parse_error: optional_string_owned(record.getattr("parse_error")?)?,
    })
}

pub fn clone_group_from_python(py: Python<'_>, group: &Bound<'_, PyAny>) -> PyResult<ParsedGroup> {
    let label_vec = bytes_like_to_vec_owned(py, group.getattr("label")?)?;
    let mut label = [0u8; 4];
    for (index, value) in label_vec.into_iter().take(4).enumerate() {
        label[index] = value;
    }
    let tail = Bytes::from(bytes_like_to_vec_owned(py, group.getattr("tail")?)?);
    let mut children = Vec::new();
    for child in group.getattr("children")?.try_iter()? {
        children.push(clone_item_from_python_owned(py, child?)?);
    }
    Ok(ParsedGroup {
        label,
        group_type: group.getattr("group_type")?.extract::<i32>()?,
        tail,
        children,
    })
}

pub fn clone_item_from_python(py: Python<'_>, item: &Bound<'_, PyAny>) -> PyResult<ParsedItem> {
    if item.hasattr("group_type")? && item.hasattr("children")? && !item.hasattr("form_id")? {
        Ok(ParsedItem::Group(clone_group_from_python(py, item)?))
    } else {
        Ok(ParsedItem::Record(clone_record_from_python(py, item)?))
    }
}

fn clone_item_from_python_owned(py: Python<'_>, item: Bound<'_, PyAny>) -> PyResult<ParsedItem> {
    clone_item_from_python(py, &item)
}

pub fn parsed_header_from_python_plugin(
    py: Python<'_>,
    plugin: &Bound<'_, PyAny>,
) -> PyResult<ParsedPluginHeader> {
    let header = plugin.getattr("header")?;
    let masters: Vec<String> = header.getattr("masters")?.extract()?;
    let master_sizes: Vec<u64> = header.getattr("master_sizes")?.extract()?;
    let overridden_forms: Vec<u32> = header.getattr("overridden_forms")?.extract()?;
    let mut extra_subrecords = Vec::new();
    for subrecord in header.getattr("extra_subrecords")?.try_iter()? {
        extra_subrecords.push(clone_subrecord_from_python_owned(py, subrecord?)?);
    }
    let raw_subrecords = match header.getattr("_raw_subrecords")? {
        value if value.is_none() => Vec::new(),
        value => {
            let mut out = Vec::new();
            for sub in value.try_iter()? {
                out.push(clone_subrecord_from_python_owned(py, sub?)?);
            }
            out
        }
    };
    let hedr_raw = match header.getattr("hedr_raw")? {
        value if value.is_none() => None,
        value => Some(Bytes::from(bytes_like_to_vec_owned(py, value)?)),
    };
    Ok(ParsedPluginHeader {
        version: header.getattr("version")?.extract()?,
        num_records: header.getattr("num_records")?.extract()?,
        next_object_id: header.getattr("next_object_id")?.extract()?,
        author: header.getattr("author")?.extract()?,
        description: header.getattr("description")?.extract()?,
        masters,
        master_sizes,
        overridden_forms,
        flags: header.getattr("flags")?.extract()?,
        extra_subrecords,
        version_control: header.getattr("version_control")?.extract()?,
        form_version: match header.getattr("form_version")? {
            value if value.is_none() => None,
            value => Some(value.extract()?),
        },
        version2: match header.getattr("version2")? {
            value if value.is_none() => None,
            value => Some(value.extract()?),
        },
        hedr_raw,
        raw_subrecords,
    })
}

pub fn parsed_plugin_from_python(
    py: Python<'_>,
    plugin: &Bound<'_, PyAny>,
) -> PyResult<ParsedPlugin> {
    let header = parsed_header_from_python_plugin(py, plugin)?;
    let plugin_name: String = plugin.getattr("plugin_name")?.extract()?;
    let file_path = match plugin.getattr("file_path")? {
        value if value.is_none() => String::new(),
        value => value.call_method0("__fspath__")?.extract::<String>()?,
    };
    let header_size: usize = plugin.getattr("header_size")?.extract()?;
    let game = match plugin.getattr("game")? {
        value if value.is_none() => None,
        value => Some(value.extract::<String>()?),
    };
    let mut root_items = Vec::new();
    for item in plugin.getattr("root_items")?.try_iter()? {
        root_items.push(clone_item_from_python_owned(py, item?)?);
    }
    Ok(ParsedPlugin {
        plugin_name,
        file_path,
        header_size,
        header,
        root_items,
        game,
    })
}

pub fn localized_strings_from_python_plugin(
    py: Python<'_>,
    plugin: &Bound<'_, PyAny>,
) -> PyResult<LocalizedStringsState> {
    let mut state = LocalizedStringsState::default();
    let by_language_obj = plugin.getattr("localized_strings_by_language")?;
    if !by_language_obj.is_none() {
        state.by_language = by_language_obj.extract::<HashMap<String, HashMap<u32, String>>>()?;
    }
    let default_language_obj = plugin.getattr("localized_default_language")?;
    if !default_language_obj.is_none() {
        state.default_language = default_language_obj.extract::<String>()?;
    }
    let table_types_obj = plugin.getattr("localized_string_table_types")?;
    if !table_types_obj.is_none() {
        state.table_types = table_types_obj.extract::<HashMap<u32, String>>()?;
    }
    let _ = py;
    Ok(state)
}

fn bytes_like_to_vec(py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<Vec<u8>> {
    if let Ok(bytes) = value.cast::<PyBytes>() {
        return Ok(bytes.as_bytes().to_vec());
    }
    if let Ok(bytearray) = value.cast::<PyByteArray>() {
        return Ok(unsafe { bytearray.as_bytes() }.to_vec());
    }
    let builtins = PyModule::import(py, "builtins")?;
    let bytes_type = builtins.getattr("bytes")?;
    let coerced = bytes_type.call1((value,))?;
    Ok(coerced.cast::<PyBytes>()?.as_bytes().to_vec())
}

fn bytes_like_to_vec_owned(py: Python<'_>, value: Bound<'_, PyAny>) -> PyResult<Vec<u8>> {
    bytes_like_to_vec(py, &value)
}

fn optional_string(value: &Bound<'_, PyAny>) -> PyResult<Option<String>> {
    if value.is_none() {
        return Ok(None);
    }
    Ok(Some(value.extract::<String>()?))
}

fn optional_string_owned(value: Bound<'_, PyAny>) -> PyResult<Option<String>> {
    optional_string(&value)
}

/// Backing store for a lazy ("index-only") plugin handle. The full
/// `ParsedItem` tree is dropped after load; the file `Bytes` (mmap-backed for
/// large masters, so evictable) plus a `form_id -> offset` map let a single
/// record be re-parsed on demand. Used for read-only target masters, whose
/// 7+ GB parsed trees otherwise sit resident through the whole conversion even
/// though only a handful of records are ever read (plus formid->sig/eid lookups
/// served from the pre-built `CoreSection`).
pub struct LazyRecordStore {
    buffer: Bytes,
    header_size: usize,
    /// Offset of the first record after the TES4 header.
    root_start: usize,
    /// Built on first use. A direct form-id read early-exits through
    /// `RecordCursor` and never touches this; materializing it at load cost
    /// 218 MB on SeventySix.esm for lookups that did not need it.
    offsets: std::sync::OnceLock<rustc_hash::FxHashMap<u32, usize>>,
    /// Lookups served before the map existed. See [`Self::offset_of`].
    probes: std::sync::atomic::AtomicUsize,
}

impl LazyRecordStore {
    fn cursor(&self) -> crate::record_cursor::RecordCursor<'_> {
        crate::record_cursor::RecordCursor::new(&self.buffer, self.header_size, self.root_start)
    }

    /// Full `form_id -> offset` map, built on first call. Only callers that
    /// genuinely need every record (an object-id search) should reach for this.
    pub(crate) fn offsets(&self) -> &rustc_hash::FxHashMap<u32, usize> {
        self.offsets.get_or_init(|| {
            let mut map = rustc_hash::FxHashMap::default();
            self.cursor().scan(&mut |view| {
                map.insert(view.form_id, view.offset);
                std::ops::ControlFlow::Continue(())
            });
            map
        })
    }

    /// Offset of a single record, indexing the file only once it is clear the
    /// caller wants more than one record.
    ///
    /// Scanning for a single form id beats indexing 5.6M records, but only for
    /// the first lookup: each scan is O(records), so a caller that probes once
    /// per record (BACUP on its lazy read-only masters) is O(records^2). The
    /// second probe pays for the map and every later one is O(1).
    pub(crate) fn offset_of(&self, form_id: u32) -> Option<usize> {
        if let Some(map) = self.offsets.get() {
            return map.get(&form_id).copied();
        }
        if self
            .probes
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            == 0
        {
            return self.cursor().find_form_id(form_id);
        }
        self.offsets().get(&form_id).copied()
    }
}

pub struct NativePluginSlot {
    pub parsed: ParsedPlugin,
    strings: LocalizedStringsState,
    localized_text_index: Option<HashMap<(String, String), u32>>,
    record_count_cache: Option<usize>,
    sections: PluginIndexSections,
    /// `Some` for lazy/index-only handles (read-only masters); `None` for the
    /// normal eager handles that own a full `parsed.root_items` tree.
    lazy: Option<LazyRecordStore>,
}

impl NativePluginSlot {
    fn invalidate_index_sections(&mut self) {
        self.sections.invalidate_all();
    }

    pub fn invalidate_sections(&mut self) {
        self.invalidate_index_sections();
    }

    pub fn apply_write_effect(&mut self, effect: &WriteEffect) {
        self.sections.apply_effect(effect);
    }

    pub fn clear_record_count_cache(&mut self) {
        self.record_count_cache = None;
    }

    pub fn has_core_section(&self) -> bool {
        self.sections.core.is_some()
    }

    pub fn has_form_id_paths_section(&self) -> bool {
        self.sections.form_id_paths.is_some()
    }

    /// `true` for index-only handles whose `parsed.root_items` tree was dropped
    /// after load (read-only target masters). Tree walkers must materialize
    /// records via [`Self::lazy_record`] instead of indexing the empty tree.
    pub fn is_lazy(&self) -> bool {
        self.lazy.is_some()
    }

    /// Re-parse a single record from the retained (mmap-backed) buffer by its
    /// raw 32-bit form id. `None` if this is not a lazy handle or the form id
    /// has no recorded offset. The returned record's subrecord `Bytes` are
    /// refcount slices into the buffer — no per-byte copy.
    pub fn lazy_record(&self, raw_form_id: u32) -> Option<ParsedRecord> {
        lazy_materialize_record(self, raw_form_id)
    }

    /// Re-parse every record under the locally owned CELL's child group from
    /// an index-only handle. The returned group type is the immediate group
    /// containing each record (normally persistent=8 or temporary=9).
    pub fn lazy_cell_children(
        &self,
        requested_cell_form_id: u32,
    ) -> Result<Vec<(i32, ParsedRecord)>, String> {
        let lazy = self.lazy.as_ref().ok_or_else(|| {
            "lazy_cell_children requires an index-only (lazy) plugin handle".to_string()
        })?;
        let own_index = u32::try_from(self.parsed.header.masters.len())
            .map_err(|_| "plugin has too many masters to encode a local FormID".to_string())?;
        if own_index > u8::MAX as u32 {
            return Err("plugin has too many masters to encode a local FormID".to_string());
        }
        let object_id = requested_cell_form_id & 0x00FF_FFFF;
        let local_form_id = (own_index << 24) | object_id;
        // Resolve through `offset_of` so a targeted CELL lookup can early-exit
        // instead of indexing every record in the plugin.
        let requested_offset = lazy.offset_of(requested_cell_form_id);
        let (cell_form_id, cell_offset) = if requested_cell_form_id > 0x00FF_FFFF
            || requested_offset.is_some()
        {
            match requested_offset {
                Some(offset) => (requested_cell_form_id, offset),
                None => return Ok(Vec::new()),
            }
        } else {
            match lazy.offset_of(local_form_id) {
                Some(offset) => (local_form_id, offset),
                None => return Ok(Vec::new()),
            }
        };
        let (cell, after_cell) =
            parse_record(&lazy.buffer, cell_offset, lazy.header_size, false)
                .map_err(|err| format!("re-parsing CELL {cell_form_id:08X}: {err}"))?;
        if cell.signature.as_str() != "CELL" {
            return Err(format!(
                "indexed record {cell_form_id:08X} is {}, not CELL",
                cell.signature
            ));
        }
        let cell_label = cell_form_id.to_le_bytes();
        let Some(children_offset) =
            cell_children_group_offset(&lazy.buffer, after_cell, lazy.header_size, cell_label)?
        else {
            return Ok(Vec::new());
        };
        let (children_group, _) =
            parse_group(&lazy.buffer, children_offset, lazy.header_size, true).map_err(|err| {
                format!("parsing CELL {cell_form_id:08X} Cell-Children GRUP: {err}")
            })?;

        fn collect(group: &ParsedGroup, out: &mut Vec<(i32, ParsedRecord)>) {
            for child in &group.children {
                match child {
                    ParsedItem::Record(record) => out.push((group.group_type, record.clone())),
                    ParsedItem::Group(nested) => collect(nested, out),
                }
            }
        }

        let mut records = Vec::new();
        collect(&children_group, &mut records);
        Ok(records)
    }

    /// Read-only accessor for the slot's `LocalizedStringsState`.
    ///
    /// Used by `conversion::struct_codec` to snapshot string state without
    /// going through the `PyResult`-returning `clone_plugin_handle_state`
    /// (which formats errors that require the Python GIL).
    pub fn strings_ref(&self) -> &LocalizedStringsState {
        &self.strings
    }

    pub fn strings_mut(&mut self) -> &mut LocalizedStringsState {
        self.localized_text_index = None;
        &mut self.strings
    }

    pub fn localized_string_id_for_text(
        &mut self,
        text: &str,
        table_type: Option<&str>,
    ) -> Option<u32> {
        self.ensure_localized_text_index();
        let index = self.localized_text_index.as_ref()?;
        if let Some(table_type) = table_type {
            if let Some(string_id) = index.get(&(table_type.to_string(), text.to_string())) {
                return Some(*string_id);
            }
        }
        index.get(&(String::new(), text.to_string())).copied()
    }

    pub fn ensure_localized_string_id(
        &mut self,
        string_id: u32,
        text: String,
        table_type: Option<&str>,
    ) {
        if self
            .strings
            .by_language
            .values()
            .any(|table| table.contains_key(&string_id))
        {
            return;
        }
        let language = self.default_localized_language();
        self.strings
            .by_language
            .entry(language)
            .or_default()
            .insert(string_id, text.clone());
        if let Some(table_type) = table_type {
            self.strings
                .table_types
                .entry(string_id)
                .or_insert_with(|| table_type.to_string());
        }
        self.note_localized_index_entry(string_id, &text, table_type);
    }

    pub fn allocate_localized_string_for_text(
        &mut self,
        text: &str,
        table_type: Option<&str>,
    ) -> u32 {
        let string_id = self.next_available_localized_string_id(1);
        let language = self.default_localized_language();
        self.strings
            .by_language
            .entry(language)
            .or_default()
            .insert(string_id, text.to_string());
        if let Some(table_type) = table_type {
            self.strings
                .table_types
                .entry(string_id)
                .or_insert_with(|| table_type.to_string());
        }
        self.note_localized_index_entry(string_id, text, table_type);
        string_id
    }

    fn default_localized_language(&mut self) -> String {
        let language = self.strings.default_language.trim();
        if !language.is_empty() {
            return language.to_string();
        }
        self.strings.default_language = "en".to_string();
        "en".to_string()
    }

    fn next_available_localized_string_id(&self, preferred_start: u32) -> u32 {
        let mut used: HashSet<u32> = self.strings.table_types.keys().copied().collect();
        for table in self.strings.by_language.values() {
            used.extend(table.keys().copied());
        }
        let mut candidate = preferred_start;
        while used.contains(&candidate) {
            candidate = candidate.saturating_add(1);
            if candidate == u32::MAX {
                return candidate;
            }
        }
        candidate
    }

    fn ensure_localized_text_index(&mut self) {
        if self.localized_text_index.is_some() {
            return;
        }
        let mut index: HashMap<(String, String), u32> = HashMap::new();
        let mut languages = Vec::new();
        let default_language = self.strings.default_language.trim();
        if !default_language.is_empty() {
            languages.push(default_language.to_string());
        }
        if default_language != "en" {
            languages.push("en".to_string());
        }
        if languages.is_empty() {
            let mut keys: Vec<&String> = self.strings.by_language.keys().collect();
            keys.sort();
            if let Some(language) = keys.first() {
                languages.push((*language).clone());
            }
        }
        for language in languages {
            let Some(table) = self.strings.by_language.get(language.as_str()) else {
                continue;
            };
            let mut rows: Vec<(u32, String, String)> = table
                .iter()
                .map(|(string_id, text)| {
                    let table_type = self
                        .strings
                        .table_types
                        .get(string_id)
                        .cloned()
                        .unwrap_or_default();
                    (*string_id, table_type, text.clone())
                })
                .collect();
            rows.sort_by_key(|(string_id, _, _)| *string_id);
            for (string_id, table_type, text) in rows {
                index
                    .entry((String::new(), text.clone()))
                    .or_insert(string_id);
                if !table_type.is_empty() {
                    index.entry((table_type, text)).or_insert(string_id);
                }
            }
        }
        self.localized_text_index = Some(index);
    }

    fn note_localized_index_entry(&mut self, string_id: u32, text: &str, table_type: Option<&str>) {
        let Some(index) = self.localized_text_index.as_mut() else {
            return;
        };
        index
            .entry((String::new(), text.to_string()))
            .or_insert(string_id);
        if let Some(table_type) = table_type {
            index
                .entry((table_type.to_string(), text.to_string()))
                .or_insert(string_id);
        }
    }
}

pub fn ensure_core_section(slot: &mut NativePluginSlot) -> Arc<CoreSection> {
    if slot.sections.core.is_none() {
        let built = if slot.lazy.is_some() {
            build_core_section_streaming(slot)
        } else {
            build_core_section(&slot.parsed)
        };
        slot.sections.core = Some(Arc::new(built));
    }
    slot.sections
        .core
        .as_ref()
        .expect("core section populated above")
        .clone()
}

/// `build_core_section` for a lazy handle, whose `root_items` is empty.
///
/// Streams the source bytes and hands `record_index_entry_for_record` a stub
/// carrying only the fields an index entry reads, so the output is identical to
/// the tree-built section without ever materializing the tree. The stub is
/// dropped each iteration; only the resulting entries persist.
fn build_core_section_streaming(slot: &NativePluginSlot) -> CoreSection {
    let Some(lazy) = slot.lazy.as_ref() else {
        return CoreSection::default();
    };
    let own_plugin_name: Arc<str> = Arc::from(slot.parsed.plugin_name.as_str());
    let masters = &slot.parsed.header.masters;
    let mut core = CoreSection::default();

    lazy.cursor().scan(&mut |view| {
        let signature = SmolStr::new(String::from_utf8_lossy(view.signature).as_ref());
        let stub = ParsedRecord {
            signature: signature.clone(),
            form_id: view.form_id,
            flags: view.flags,
            version_control: 0,
            form_version: None,
            version2: None,
            subrecords: first_edid_subrecord(view.payload, view.flags)
                .into_iter()
                .collect(),
            raw_payload: None,
            parse_error: None,
        };
        let entry = record_index_entry_for_record(&stub, &own_plugin_name, masters);
        let form_key = entry.form_key.clone();

        core.form_ids_by_object_id
            .entry(view.form_id & 0x00FF_FFFF)
            .or_default()
            .push(view.form_id);
        core.form_ids_by_signature
            .entry(signature.clone())
            .or_default()
            .push(view.form_id);
        if !entry.eid.is_empty() {
            core.by_eid_lower
                .entry(entry.eid.to_ascii_lowercase())
                .or_default()
                .push(form_key.clone());
        }
        core.by_form_key.insert(form_key.clone(), entry);
        core.by_signature_form_keys
            .entry(signature)
            .or_default()
            .push(form_key);
        std::ops::ControlFlow::Continue(())
    });

    core
}

/// The `EDID` subrecord of an encoded payload, if it has one.
///
/// Only 5.7% of records in SeventySix.esm carry an EditorID, so this gives up
/// after a few subrecords instead of walking every subrecord of every record
/// looking for one that usually is not there. COMPRESSED payloads are zlib
/// framed, so `EDID` is invisible without inflating first - that is 1.3% of
/// records and about 0.65 s across the whole plugin.
fn first_edid_subrecord(payload: &[u8], flags: u32) -> Option<ParsedSubrecord> {
    const MAX_SUBRECORDS_SCANNED: usize = 3;

    if (flags & COMPRESSED_RECORD_FLAG) != 0 {
        if payload.len() < 4 {
            return None;
        }
        let mut inflated = Vec::new();
        ZlibDecoder::new(&payload[4..])
            .read_to_end(&mut inflated)
            .ok()?;
        return first_edid_subrecord(&inflated, 0);
    }

    let mut cursor = 0usize;
    for _ in 0..MAX_SUBRECORDS_SCANNED {
        if cursor + 6 > payload.len() {
            return None;
        }
        let length = u16::from_le_bytes([payload[cursor + 4], payload[cursor + 5]]) as usize;
        let start = cursor + 6;
        let end = (start + length).min(payload.len());
        if &payload[cursor..cursor + 4] == b"EDID" {
            return Some(ParsedSubrecord {
                signature: SmolStr::new_static("EDID"),
                data: Bytes::copy_from_slice(&payload[start..end]),
                semantic_type: None,
            });
        }
        cursor = start + length;
    }
    None
}

pub fn ensure_records_section(slot: &mut NativePluginSlot) -> Arc<RecordsSection> {
    if slot.sections.records.is_none() {
        slot.sections.records = Some(Arc::new(build_records_section(&slot.parsed)));
    }
    slot.sections
        .records
        .as_ref()
        .expect("records section populated above")
        .clone()
}

pub fn ensure_form_id_paths_section(slot: &mut NativePluginSlot) -> Arc<FormIdPathsSection> {
    if slot.sections.form_id_paths.is_none() {
        slot.sections.form_id_paths = Some(Arc::new(build_form_id_paths_section(&slot.parsed)));
    }
    slot.sections
        .form_id_paths
        .as_ref()
        .expect("form_id_paths populated above")
        .clone()
}

pub fn ensure_locator_section(slot: &mut NativePluginSlot) -> Arc<LocatorSection> {
    if slot.sections.locator.is_none() {
        slot.sections.locator = Some(Arc::new(build_locator_section(&slot.parsed)));
    }
    slot.sections
        .locator
        .as_ref()
        .expect("locator section populated above")
        .clone()
}

pub fn ensure_refs_section(slot: &mut NativePluginSlot) -> Arc<RefsSection> {
    if slot.sections.refs.is_none() {
        slot.sections.refs = Some(Arc::new(build_refs_section(&slot.parsed)));
    }
    slot.sections
        .refs
        .as_ref()
        .expect("refs section populated above")
        .clone()
}

pub fn ensure_assets_section(slot: &mut NativePluginSlot) -> Arc<AssetsSection> {
    if slot.sections.assets.is_none() {
        slot.sections.assets = Some(Arc::new(build_assets_section(&slot.parsed)));
    }
    slot.sections
        .assets
        .as_ref()
        .expect("assets section populated above")
        .clone()
}

struct PluginMetadataSnapshot {
    plugin_name: String,
    file_path: String,
    game: Option<String>,
    header_size: usize,
    header: ParsedPluginHeader,
    record_count: usize,
    localized_default_language: String,
}

type ParsedSubrecordPayload = (String, Vec<u8>, Option<String>);
type ParsedHeaderPayload = (
    f32,
    u32,
    u32,
    String,
    String,
    Vec<String>,
    Vec<u64>,
    Vec<u32>,
    u32,
    Vec<ParsedSubrecordPayload>,
    u32,
    Option<u16>,
    Option<u16>,
    Option<Vec<u8>>,
    Vec<ParsedSubrecordPayload>,
);
type PluginMetadataPayload = (
    String,
    String,
    Option<String>,
    usize,
    ParsedHeaderPayload,
    usize,
    String,
);
type PluginLegacyMetadataPayload = (PluginMetadataPayload, PluginStringsPayload);
type RecordContextPayload = (String, Option<u16>, Option<u16>);

static NEXT_PLUGIN_HANDLE_ID: AtomicU64 = AtomicU64::new(1);
static PLUGIN_HANDLES: OnceLock<Mutex<HashMap<u64, NativePluginSlot>>> = OnceLock::new();

fn plugin_handle_store() -> &'static Mutex<HashMap<u64, NativePluginSlot>> {
    PLUGIN_HANDLES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Crate-visible accessor for the global handle store. Used by sibling
/// modules (e.g. `conflicts`) that need to read parsed plugin data.
pub fn plugin_handle_store_ref() -> &'static Mutex<HashMap<u64, NativePluginSlot>> {
    plugin_handle_store()
}

fn insert_plugin_handle(parsed: ParsedPlugin, strings: LocalizedStringsState) -> u64 {
    let id = NEXT_PLUGIN_HANDLE_ID.fetch_add(1, Ordering::Relaxed);
    plugin_handle_store().lock().unwrap().insert(
        id,
        NativePluginSlot {
            parsed,
            strings,
            localized_text_index: None,
            record_count_cache: None,
            sections: PluginIndexSections::default(),
            lazy: None,
        },
    );
    id
}

/// Insert a lazy/index-only handle: the `parsed` stub keeps the header +
/// metadata but has an empty `root_items`; the pre-built `CoreSection` answers
/// formid->sig / eid lookups, and `lazy` re-parses individual records on demand.
fn insert_plugin_handle_lazy(
    parsed: ParsedPlugin,
    strings: LocalizedStringsState,
    lazy: LazyRecordStore,
) -> u64 {
    let id = NEXT_PLUGIN_HANDLE_ID.fetch_add(1, Ordering::Relaxed);
    plugin_handle_store().lock().unwrap().insert(
        id,
        NativePluginSlot {
            parsed,
            strings,
            localized_text_index: None,
            record_count_cache: None,
            sections: PluginIndexSections::default(),
            lazy: Some(lazy),
        },
    );
    id
}

/// Re-parse a single record from a lazy handle's retained buffer. Returns an
/// owned `ParsedRecord` whose subrecord `Bytes` are refcount slices into the
/// (mmap-backed) buffer — no per-byte copy.
pub(crate) fn lazy_materialize_record(
    slot: &NativePluginSlot,
    raw_form_id: u32,
) -> Option<ParsedRecord> {
    let lazy = slot.lazy.as_ref()?;
    let offset = lazy.offset_of(raw_form_id)?;
    parse_record(&lazy.buffer, offset, lazy.header_size, true)
        .ok()
        .map(|(record, _)| record)
}

fn cell_children_group_offset(
    buffer: &Bytes,
    offset: usize,
    header_size: usize,
    cell_label: [u8; 4],
) -> Result<Option<usize>, String> {
    if offset >= buffer.len() {
        return Ok(None);
    }
    if offset + 4 > buffer.len() {
        return Err(format!(
            "malformed plugin: {} trailing byte(s) after CELL {:08X}",
            buffer.len() - offset,
            u32::from_le_bytes(cell_label)
        ));
    }
    if &buffer[offset..offset + 4] != b"GRUP" {
        return Ok(None);
    }
    if offset + header_size > buffer.len() {
        return Err(format!(
            "malformed plugin: truncated GRUP header after CELL {:08X}",
            u32::from_le_bytes(cell_label)
        ));
    }
    let group_type = read_i32(buffer, offset + 12).map_err(|err| err.to_string())?;
    if group_type != CELL_CHILD_GROUP {
        return Ok(None);
    }
    let mut label = [0u8; 4];
    label.copy_from_slice(&buffer[offset + 8..offset + 12]);
    if label != cell_label {
        return Err(format!(
            "malformed plugin: Cell-Children GRUP after CELL {:08X} is labelled {:08X}",
            u32::from_le_bytes(cell_label),
            u32::from_le_bytes(label)
        ));
    }
    let group_size = read_u32(buffer, offset + 4).map_err(|err| err.to_string())? as usize;
    let group_end = offset.checked_add(group_size).ok_or_else(|| {
        format!(
            "malformed plugin: Cell-Children GRUP size overflow for CELL {:08X}",
            u32::from_le_bytes(cell_label)
        )
    })?;
    if group_size < header_size || group_end > buffer.len() {
        return Err(format!(
            "malformed plugin: Cell-Children GRUP for CELL {:08X} is truncated \
             (declared size {group_size}, {} byte(s) remain)",
            u32::from_le_bytes(cell_label),
            buffer.len() - offset
        ));
    }
    Ok(Some(offset))
}

/// Byte-level plugin builders shared by the tests in this crate.
#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    pub(crate) fn record(signature: &[u8; 4], form_id: u32, flags: u32, payload: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(MODERN_HEADER_SIZE + payload.len());
        bytes.extend_from_slice(signature);
        bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&flags.to_le_bytes());
        bytes.extend_from_slice(&form_id.to_le_bytes());
        bytes.extend_from_slice(&[0u8; 8]);
        bytes.extend_from_slice(payload);
        bytes
    }

    pub(crate) fn subrecord(signature: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(6 + data.len());
        bytes.extend_from_slice(signature);
        bytes.extend_from_slice(&(data.len() as u16).to_le_bytes());
        bytes.extend_from_slice(data);
        bytes
    }

    pub(crate) fn group(label: [u8; 4], group_type: i32, children: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(MODERN_HEADER_SIZE + children.len());
        bytes.extend_from_slice(b"GRUP");
        bytes.extend_from_slice(&((MODERN_HEADER_SIZE + children.len()) as u32).to_le_bytes());
        bytes.extend_from_slice(&label);
        bytes.extend_from_slice(&group_type.to_le_bytes());
        bytes.extend_from_slice(&[0u8; 8]);
        bytes.extend_from_slice(children);
        bytes
    }

    pub(crate) fn compressed_record(signature: &[u8; 4], form_id: u32, payload: &[u8]) -> Vec<u8> {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(payload).unwrap();
        let compressed = encoder.finish().unwrap();
        let mut encoded = Vec::with_capacity(4 + compressed.len());
        encoded.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        encoded.extend_from_slice(&compressed);
        record(signature, form_id, COMPRESSED_RECORD_FLAG, &encoded)
    }
}

#[cfg(test)]
mod lazy_offset_probe_tests {
    use super::test_support::{group, record, subrecord};
    use super::{LazyRecordStore, MODERN_HEADER_SIZE};
    use bytes::Bytes;

    fn store(record_count: u32) -> LazyRecordStore {
        let mut children = Vec::new();
        for index in 0..record_count {
            children.extend_from_slice(&record(
                b"REFR",
                0x0100_0000 + index,
                0,
                &subrecord(b"EDID", b"B21_Probe\0"),
            ));
        }
        let mut data = Vec::new();
        data.extend_from_slice(&record(b"TES4", 0, 0, &[]));
        let root_start = data.len();
        data.extend_from_slice(&group(*b"REFR", 0, &children));
        LazyRecordStore {
            buffer: Bytes::from(data),
            header_size: MODERN_HEADER_SIZE,
            root_start,
            offsets: std::sync::OnceLock::new(),
            probes: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    #[test]
    fn one_lookup_does_not_index_the_file() {
        let store = store(64);
        assert!(store.offset_of(0x0100_0005).is_some());
        assert!(
            store.offsets.get().is_none(),
            "a single form-id read must not build the offsets map"
        );
    }

    #[test]
    fn repeated_lookups_switch_to_the_index() {
        // A caller that probes once per record - BACUP's conversion fixups scan
        // their read-only masters that way - must not pay a full scan per probe.
        let store = store(64);
        assert!(store.offset_of(0x0100_0005).is_some());
        assert!(store.offset_of(0x0100_0006).is_some());
        assert!(
            store.offsets.get().is_some(),
            "the second lookup must build the offsets map, not scan again"
        );
        assert_eq!(store.offsets.get().unwrap().len(), 64);
        for index in 0..64u32 {
            assert!(store.offset_of(0x0100_0000 + index).is_some());
        }
        assert!(store.offset_of(0x0200_0000).is_none());
    }
}

#[cfg(test)]
mod lazy_cell_children_tests {
    use super::*;
    use crate::plugin_runtime::test_support::{compressed_record, group, record, subrecord};

    fn lazy_slot(buffer: Vec<u8>, masters: Vec<String>) -> NativePluginSlot {
        let buffer = Bytes::from(buffer);
        let mut offsets = rustc_hash::FxHashMap::default();
        scan_record_offsets(&buffer, 0, buffer.len(), MODERN_HEADER_SIZE, &mut offsets);
        let mut header = ParsedPluginHeader::default_for_test();
        header.masters = masters;
        NativePluginSlot {
            parsed: ParsedPlugin {
                plugin_name: "Test.esp".to_string(),
                file_path: String::new(),
                header_size: MODERN_HEADER_SIZE,
                header,
                root_items: Vec::new(),
                game: Some("fo4".to_string()),
            },
            strings: LocalizedStringsState::default(),
            localized_text_index: None,
            record_count_cache: Some(offsets.len()),
            sections: PluginIndexSections::default(),
            lazy: Some(LazyRecordStore {
                buffer,
                header_size: MODERN_HEADER_SIZE,
                root_start: 0,
                offsets: std::sync::OnceLock::from(offsets),
                probes: std::sync::atomic::AtomicUsize::new(0),
            }),
        }
    }

    #[test]
    fn reads_owned_cell_sections_from_plugin_with_master_and_inflates_records() {
        let cell = 0x0100_0010u32;
        let persistent = record(b"REFR", 0x0100_0011, 0, &subrecord(b"NAME", &[1, 0, 0, 0]));
        let temporary = compressed_record(b"REFR", 0x0100_0012, &subrecord(b"NAME", &[2, 0, 0, 0]));
        let direct = record(b"LAND", 0x0100_0013, 0, &[]);
        let children = group(
            cell.to_le_bytes(),
            CELL_CHILD_GROUP,
            &[
                direct,
                group(cell.to_le_bytes(), PERSISTENT_GROUP, &persistent),
                group(cell.to_le_bytes(), TEMPORARY_GROUP, &temporary),
            ]
            .concat(),
        );
        let slot = lazy_slot(
            [record(b"CELL", cell, 0, &[]), children].concat(),
            vec!["Fallout4.esm".to_string()],
        );

        let got = slot.lazy_cell_children(0x000010).unwrap();

        assert_eq!(
            got.iter()
                .map(|(group_type, record)| (
                    *group_type,
                    record.signature.as_str(),
                    record.form_id
                ))
                .collect::<Vec<_>>(),
            vec![
                (CELL_CHILD_GROUP, "LAND", 0x0100_0013),
                (PERSISTENT_GROUP, "REFR", 0x0100_0011),
                (TEMPORARY_GROUP, "REFR", 0x0100_0012),
            ]
        );
        assert!(got[2].1.subrecords.iter().any(|subrecord| {
            subrecord.signature.as_str() == "NAME" && subrecord.data.as_ref() == [2, 0, 0, 0]
        }));
    }

    #[test]
    fn childless_cell_returns_empty_when_sibling_record_follows() {
        let cell = 0x0000_0020u32;
        let slot = lazy_slot(
            [
                record(b"CELL", cell, 0, &[]),
                record(b"CELL", 0x0000_0021, 0, &[]),
            ]
            .concat(),
            Vec::new(),
        );

        assert!(slot.lazy_cell_children(cell).unwrap().is_empty());
    }

    #[test]
    fn raw_form_id_distinguishes_master_override_from_local_cell() {
        let object_id = 0x0000_0020u32;
        let local_cell = 0x0100_0020u32;
        let base_children = group(
            object_id.to_le_bytes(),
            CELL_CHILD_GROUP,
            &group(
                object_id.to_le_bytes(),
                TEMPORARY_GROUP,
                &record(b"REFR", 0x0000_0021, 0, &[]),
            ),
        );
        let local_children = group(
            local_cell.to_le_bytes(),
            CELL_CHILD_GROUP,
            &group(
                local_cell.to_le_bytes(),
                TEMPORARY_GROUP,
                &record(b"REFR", 0x0100_0021, 0, &[]),
            ),
        );
        let slot = lazy_slot(
            [
                record(b"CELL", object_id, 0, &[]),
                base_children,
                record(b"CELL", local_cell, 0, &[]),
                local_children,
            ]
            .concat(),
            vec!["Fallout4.esm".to_string()],
        );

        assert_eq!(
            slot.lazy_cell_children(object_id).unwrap()[0].1.form_id,
            0x0000_0021
        );
        assert_eq!(
            slot.lazy_cell_children(local_cell).unwrap()[0].1.form_id,
            0x0100_0021
        );
    }

    #[test]
    fn mislabelled_cell_children_group_is_an_error() {
        let cell = 0x0000_0030u32;
        let children = group(
            0x0000_0031u32.to_le_bytes(),
            CELL_CHILD_GROUP,
            &group(cell.to_le_bytes(), TEMPORARY_GROUP, &[]),
        );
        let slot = lazy_slot(
            [record(b"CELL", cell, 0, &[]), children].concat(),
            Vec::new(),
        );

        let Err(error) = slot.lazy_cell_children(cell) else {
            panic!("mislabelled Cell-Children group must fail");
        };

        assert!(error.contains("labelled 00000031"), "got: {error}");
    }

    #[test]
    fn eager_handle_is_rejected() {
        let mut slot = lazy_slot(Vec::new(), Vec::new());
        slot.lazy = None;

        let Err(error) = slot.lazy_cell_children(0x10) else {
            panic!("eager handle must fail");
        };

        assert!(error.contains("index-only"), "got: {error}");
    }
}

pub fn clone_plugin_handle_state(
    handle_id: u64,
) -> PyResult<(ParsedPlugin, LocalizedStringsState)> {
    let store = plugin_handle_store().lock().unwrap();
    let slot = store
        .get(&handle_id)
        .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
    Ok((slot.parsed.clone(), slot.strings.clone()))
}

/// GIL-free variant — returns `Err(String)` so callers in Rust-only phases
/// don't require an initialized Python interpreter.
pub fn clone_plugin_handle_state_no_py(
    handle_id: u64,
) -> Result<(ParsedPlugin, LocalizedStringsState), String> {
    let store = plugin_handle_store()
        .lock()
        .map_err(|e| format!("plugin handle store lock poisoned: {e}"))?;
    let slot = store
        .get(&handle_id)
        .ok_or_else(|| format!("unknown plugin handle: {handle_id}"))?;
    Ok((slot.parsed.clone(), slot.strings.clone()))
}

pub fn plugin_handle_next_available_object_id_no_py(handle_id: u64) -> Result<u32, String> {
    let store = plugin_handle_store()
        .lock()
        .map_err(|e| format!("plugin handle store lock poisoned: {e}"))?;
    let slot = store
        .get(&handle_id)
        .ok_or_else(|| format!("unknown plugin handle: {handle_id}"))?;
    let header_next = slot.parsed.header.next_object_id & 0x00FF_FFFF;
    let max_record = max_record_object_id_in_items(&slot.parsed.root_items).unwrap_or(0);
    Ok(header_next.max(max_record.saturating_add(1)).max(0x000800))
}

pub fn plugin_handle_max_object_id_no_py(handle_id: u64) -> Result<u32, String> {
    let store = plugin_handle_store()
        .lock()
        .map_err(|e| format!("plugin handle store lock poisoned: {e}"))?;
    let slot = store
        .get(&handle_id)
        .ok_or_else(|| format!("unknown plugin handle: {handle_id}"))?;
    Ok(max_record_object_id_in_items(&slot.parsed.root_items).unwrap_or(0))
}

pub fn plugin_handle_raise_next_object_id_no_py(handle_id: u64, floor: u32) -> Result<(), String> {
    if floor == 0 {
        return Ok(());
    }
    if floor > 0x00FF_FFFF {
        return Err(format!(
            "object-id floor is outside local FormID range: 0x{floor:08X}"
        ));
    }
    let mut store = plugin_handle_store()
        .lock()
        .map_err(|e| format!("plugin handle store lock poisoned: {e}"))?;
    let slot = store
        .get_mut(&handle_id)
        .ok_or_else(|| format!("unknown plugin handle: {handle_id}"))?;
    let header_next = slot.parsed.header.next_object_id & 0x00FF_FFFF;
    let max_record = max_record_object_id_in_items(&slot.parsed.root_items).unwrap_or(0);
    let current_next = header_next.max(max_record.saturating_add(1)).max(0x000800);
    slot.parsed.header.next_object_id = current_next.max(floor);
    Ok(())
}

pub fn plugin_handle_used_object_ids_no_py(handle_id: u64) -> Result<Vec<u32>, String> {
    let store = plugin_handle_store()
        .lock()
        .map_err(|e| format!("plugin handle store lock poisoned: {e}"))?;
    let slot = store
        .get(&handle_id)
        .ok_or_else(|| format!("unknown plugin handle: {handle_id}"))?;
    let mut used_object_ids = BTreeSet::new();
    collect_record_object_ids(&slot.parsed.root_items, &mut used_object_ids);
    Ok(used_object_ids.into_iter().collect())
}

pub fn plugin_handle_find_record_object_id_by_editor_id_no_py(
    handle_id: u64,
    signature: &str,
    editor_id: &str,
) -> Result<Option<u32>, String> {
    let mut store = plugin_handle_store()
        .lock()
        .map_err(|e| format!("plugin handle store lock poisoned: {e}"))?;
    let slot = store
        .get_mut(&handle_id)
        .ok_or_else(|| format!("unknown plugin handle: {handle_id}"))?;
    if slot.lazy.is_some() {
        // Lazy handle: the tree is dropped, so answer from the core index
        // (`by_eid_lower` keyed by ascii-lowercased editor_id, matching
        // `build_core_section`). Built on demand by streaming the source bytes.
        let core = ensure_core_section(slot);
        let wanted = editor_id.to_ascii_lowercase();
        let sig = smol_str::SmolStr::new(signature);
        if let Some(form_keys) = core.by_eid_lower.get(&wanted) {
            for fk in form_keys {
                if let Some(entry) = core.by_form_key.get(fk) {
                    if entry.signature == sig {
                        return Ok(Some(entry.raw_form_id & 0x00FF_FFFF));
                    }
                }
            }
        }
        return Ok(None);
    }
    let wanted_editor_id = normalize_editor_id_for_match(editor_id);
    Ok(find_record_object_id_by_editor_id_in_items(
        &slot.parsed.root_items,
        signature,
        &wanted_editor_id,
    ))
}

/// Return authoring FormKeys for every record with `signature`.
///
/// This is the non-Python counterpart to `plugin_handle_record_form_ids` for
/// native consumers that need to enumerate a small record family and then
/// materialize individual records through the authoring JSON reader.
pub fn plugin_handle_record_form_keys_by_signature_no_py(
    handle_id: u64,
    signature: &str,
) -> Result<Vec<String>, String> {
    let mut store = plugin_handle_store()
        .lock()
        .map_err(|e| format!("plugin handle store lock poisoned: {e}"))?;
    let slot = store
        .get_mut(&handle_id)
        .ok_or_else(|| format!("unknown plugin handle: {handle_id}"))?;
    let core = ensure_core_section(slot);
    let signature = SmolStr::new(signature);
    let mut form_keys = core
        .by_signature_form_keys
        .get(&signature)
        .into_iter()
        .flatten()
        .map(FormKey::render)
        .collect::<Vec<_>>();
    form_keys.sort_unstable();
    Ok(form_keys)
}

/// Resolve a plugin-local/raw FormID using the handle's own master table.
pub fn plugin_handle_resolve_form_id_no_py(
    handle_id: u64,
    raw_form_id: u32,
) -> Result<String, String> {
    let store = plugin_handle_store()
        .lock()
        .map_err(|e| format!("plugin handle store lock poisoned: {e}"))?;
    let slot = store
        .get(&handle_id)
        .ok_or_else(|| format!("unknown plugin handle: {handle_id}"))?;
    let own_plugin_name: Arc<str> = Arc::from(slot.parsed.plugin_name.as_str());
    Ok(
        resolve_form_id_to_form_key(raw_form_id, &own_plugin_name, &slot.parsed.header.masters)
            .render(),
    )
}

fn max_record_object_id_in_items(items: &[ParsedItem]) -> Option<u32> {
    let mut max_object_id: Option<u32> = None;
    for item in items {
        match item {
            ParsedItem::Record(record) => {
                let object_id = record.form_id & 0x00FF_FFFF;
                max_object_id = Some(max_object_id.map_or(object_id, |max| max.max(object_id)));
            }
            ParsedItem::Group(group) => {
                if let Some(object_id) = max_record_object_id_in_items(&group.children) {
                    max_object_id = Some(max_object_id.map_or(object_id, |max| max.max(object_id)));
                }
            }
        }
    }
    max_object_id
}

fn collect_record_object_ids(items: &[ParsedItem], used_object_ids: &mut BTreeSet<u32>) {
    for item in items {
        match item {
            ParsedItem::Record(record) => {
                let object_id = record.form_id & 0x00FF_FFFF;
                if object_id != 0 {
                    used_object_ids.insert(object_id);
                }
            }
            ParsedItem::Group(group) => collect_record_object_ids(&group.children, used_object_ids),
        }
    }
}

fn collect_record_form_ids(items: &[ParsedItem], used_form_ids: &mut HashSet<u32>) {
    for item in items {
        match item {
            ParsedItem::Record(record) => {
                used_form_ids.insert(record.form_id);
            }
            ParsedItem::Group(group) => collect_record_form_ids(&group.children, used_form_ids),
        }
    }
}

fn find_record_object_id_by_editor_id_in_items(
    items: &[ParsedItem],
    signature: &str,
    editor_id: &str,
) -> Option<u32> {
    for item in items {
        match item {
            ParsedItem::Record(record)
                if record.signature.as_str() == signature
                    && record_editor_id_matches(record, editor_id) =>
            {
                return Some(record.form_id & 0x00FF_FFFF);
            }
            ParsedItem::Group(group) => {
                if let Some(form_id) = find_record_object_id_by_editor_id_in_items(
                    &group.children,
                    signature,
                    editor_id,
                ) {
                    return Some(form_id);
                }
            }
            _ => {}
        }
    }
    None
}

pub fn clone_plugin_handle_state_for_authoring(
    handle_id: u64,
) -> PyResult<(ParsedPlugin, LocalizedStringsState)> {
    let mut store = plugin_handle_store().lock().unwrap();
    let slot = store
        .get_mut(&handle_id)
        .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
    rehydrate_filtered_strings_for_authoring(slot);
    slot.strings.materialize_all();
    Ok((slot.parsed.clone(), slot.strings.clone()))
}

/// GIL-free variant for use in Rust-only phases.
pub fn clone_plugin_handle_state_for_authoring_no_py(
    handle_id: u64,
) -> Result<(ParsedPlugin, LocalizedStringsState), String> {
    let mut store = plugin_handle_store()
        .lock()
        .map_err(|e| format!("plugin handle store lock poisoned: {e}"))?;
    let slot = store
        .get_mut(&handle_id)
        .ok_or_else(|| format!("unknown plugin handle: {handle_id}"))?;
    rehydrate_filtered_strings_for_authoring(slot);
    slot.strings.materialize_all();
    Ok((slot.parsed.clone(), slot.strings.clone()))
}

pub fn rehydrate_filtered_strings_for_authoring(slot: &mut NativePluginSlot) {
    if !slot.strings.is_filtered {
        return;
    }
    strings::rehydrate_all(
        &mut slot.strings,
        slot.parsed.file_path.as_str(),
        slot.parsed.plugin_name.as_str(),
        None,
    );
    // rehydrate_all reads the same loose files and archives an index would,
    // so the decoded tables now supersede it.
    slot.strings.lazy_tables = None;
    slot.localized_text_index = None;
}

pub fn insert_authoring_record_value(handle_id: u64, value: &JsonValue) -> PyResult<String> {
    let mut store = plugin_handle_store().lock().unwrap();
    let slot = store
        .get_mut(&handle_id)
        .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
    let old_masters = slot.parsed.header.masters.clone();
    let old_own_index = old_masters.len() as u8;
    slot.strings.materialize_all();
    let old_strings = slot.strings.clone();
    let mut context = NativeImportContext::new(
        slot.parsed.plugin_name.clone(),
        slot.parsed.game.clone(),
        slot.parsed.header_size,
        slot.parsed.header.clone(),
    );
    context.strings = old_strings.clone();

    let mut record = authoring_value_to_record(value, &mut context)?;
    if context.header.masters != old_masters {
        let mut reparsed_context = NativeImportContext::new(
            slot.parsed.plugin_name.clone(),
            slot.parsed.game.clone(),
            slot.parsed.header_size,
            context.header.clone(),
        );
        reparsed_context.strings = old_strings;
        record = authoring_value_to_record(value, &mut reparsed_context)?;
        context = reparsed_context;
    }
    if context.header.masters != old_masters {
        let target_masters = context.header.masters.clone();
        let target_own_index = target_masters.len() as u8;
        remap_formids_in_items(
            &mut slot.parsed.root_items,
            &old_masters,
            &target_masters,
            old_own_index,
            target_own_index,
        );
    }
    let masters_changed = context.header.masters != old_masters;
    slot.parsed.header = context.header;
    slot.strings = context.strings;
    slot.localized_text_index = None;

    if record.form_id == 0 {
        let object_id = slot.parsed.header.next_object_id & 0x00FF_FFFF;
        record.form_id = ((slot.parsed.header.masters.len() as u32) << 24) | object_id;
        slot.parsed.header.next_object_id = (object_id + 1) & 0x00FF_FFFF;
    } else {
        let master_index = ((record.form_id >> 24) & 0xFF) as u8;
        let own_index = (slot.parsed.header.masters.len() & 0xFF) as u8;
        if master_index == LOCAL_FORM_INDEX
            || master_index == own_index
            || master_index as usize >= slot.parsed.header.masters.len()
        {
            let object_id = record.form_id & 0x00FF_FFFF;
            let next_object_id = slot.parsed.header.next_object_id & 0x00FF_FFFF;
            if object_id >= next_object_id {
                slot.parsed.header.next_object_id = (object_id + 1) & 0x00FF_FFFF;
            }
        }
    }

    let own_plugin_name: Arc<str> = Arc::from(slot.parsed.plugin_name.as_str());
    let form_key = resolve_form_id_to_form_key(
        record.form_id,
        &own_plugin_name,
        &slot.parsed.header.masters,
    )
    .to_string();
    let header_size = slot.parsed.header_size;
    let game = slot.parsed.game.clone();
    ensure_top_group_and_add(
        &mut slot.parsed.root_items,
        record,
        header_size,
        game.as_deref(),
    );
    slot.record_count_cache = None;
    slot.sections.apply_effect(if masters_changed {
        &WriteEffect::MastersChanged
    } else {
        &WriteEffect::RecordsAddedOrRemoved
    });
    Ok(form_key)
}

pub fn plugin_handle_read_authoring_record_value_json(
    handle_id: u64,
    form_key: &str,
) -> PyResult<Option<JsonValue>> {
    let mut store = plugin_handle_store().lock().unwrap();
    let slot = store
        .get_mut(&handle_id)
        .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
    let core = ensure_core_section(slot);
    let Some(entry) = record_index_entry_by_form_key(&core, form_key) else {
        return Ok(None);
    };
    let raw_form_id = entry.raw_form_id;
    if slot.lazy.is_some() {
        let Some(record) = lazy_materialize_record(slot, raw_form_id) else {
            return Ok(None);
        };
        return Ok(Some(serialize_record_payload_to_json(
            &record,
            &slot.parsed,
            &slot.strings,
        )));
    }
    let records = ensure_records_section(slot);
    let Some(record) = records.record(&slot.parsed, raw_form_id) else {
        return Ok(None);
    };
    Ok(Some(serialize_record_payload_to_json(
        record,
        &slot.parsed,
        &slot.strings,
    )))
}

/// Read exact subrecord payloads without schema decoding.
///
/// Native conversion code uses this for signatures that are overloaded by
/// Starfield base-form component scopes, where authoring-key dispatch can be
/// intentionally lossy even though the underlying plugin bytes are valid.
pub fn plugin_handle_read_raw_subrecords_no_py(
    handle_id: u64,
    form_key: &str,
    subrecord_signature: &str,
) -> Result<Vec<Vec<u8>>, String> {
    fn collect(record: &ParsedRecord, signature: &str) -> Vec<Vec<u8>> {
        effective_subrecords_for_record(record)
            .iter()
            .filter(|subrecord| subrecord.signature == signature)
            .map(|subrecord| subrecord.data.to_vec())
            .collect()
    }

    let mut store = plugin_handle_store()
        .lock()
        .map_err(|error| format!("plugin handle store lock poisoned: {error}"))?;
    let slot = store
        .get_mut(&handle_id)
        .ok_or_else(|| format!("unknown plugin handle: {handle_id}"))?;
    let core = ensure_core_section(slot);
    let Some(entry) = record_index_entry_by_form_key(&core, form_key) else {
        return Ok(Vec::new());
    };
    let raw_form_id = entry.raw_form_id;
    if slot.lazy.is_some() {
        let Some(record) = lazy_materialize_record(slot, raw_form_id) else {
            return Ok(Vec::new());
        };
        return Ok(collect(&record, subrecord_signature));
    }
    let records = ensure_records_section(slot);
    let Some(record) = records.record(&slot.parsed, raw_form_id) else {
        return Ok(Vec::new());
    };
    Ok(collect(record, subrecord_signature))
}

/// Look up a record by `(editor_id, signature)` and return `(form_key, authoring_dict_json)`.
/// `editor_id` is matched case-insensitively; `signature` is matched exactly (e.g. `"TXST"`).
/// Returns `None` if no matching record exists. Callable from non-Python Rust (no GIL needed).
pub fn plugin_handle_read_authoring_record_by_editor_id_and_signature_json(
    handle_id: u64,
    editor_id: &str,
    signature: &str,
) -> PyResult<Option<(String, JsonValue)>> {
    use smol_str::SmolStr;
    let target = editor_id.to_ascii_lowercase();
    let sig = SmolStr::new(signature);
    let mut store = plugin_handle_store().lock().unwrap();
    let slot = store
        .get_mut(&handle_id)
        .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
    let core = ensure_core_section(slot);
    let Some(form_keys) = core.by_eid_lower.get(&target) else {
        return Ok(None);
    };
    let Some(entry) = form_keys
        .iter()
        .filter_map(|fk| core.by_form_key.get(fk))
        .find(|e| e.signature == sig)
        .cloned()
    else {
        return Ok(None);
    };
    let form_key_str = entry.form_key.render();
    let raw_form_id = entry.raw_form_id;
    if slot.lazy.is_some() {
        let Some(record) = lazy_materialize_record(slot, raw_form_id) else {
            return Ok(None);
        };
        let json = serialize_record_payload_to_json(&record, &slot.parsed, &slot.strings);
        return Ok(Some((form_key_str, json)));
    }
    let records = ensure_records_section(slot);
    let Some(record) = records.record(&slot.parsed, raw_form_id) else {
        return Ok(None);
    };
    let json = serialize_record_payload_to_json(record, &slot.parsed, &slot.strings);
    Ok(Some((form_key_str, json)))
}

pub fn plugin_handle_replace_authoring_record_value(
    handle_id: u64,
    value: &JsonValue,
) -> PyResult<String> {
    let mut store = plugin_handle_store().lock().unwrap();
    let slot = store
        .get_mut(&handle_id)
        .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
    let old_masters = slot.parsed.header.masters.clone();
    let old_own_index = old_masters.len() as u8;
    slot.strings.materialize_all();
    let old_strings = slot.strings.clone();
    let mut context = NativeImportContext::new(
        slot.parsed.plugin_name.clone(),
        slot.parsed.game.clone(),
        slot.parsed.header_size,
        slot.parsed.header.clone(),
    );
    context.strings = old_strings.clone();

    let mut record = authoring_value_to_record(value, &mut context)?;
    if context.header.masters != old_masters {
        let mut reparsed_context = NativeImportContext::new(
            slot.parsed.plugin_name.clone(),
            slot.parsed.game.clone(),
            slot.parsed.header_size,
            context.header.clone(),
        );
        reparsed_context.strings = old_strings;
        record = authoring_value_to_record(value, &mut reparsed_context)?;
        context = reparsed_context;
    }
    if context.header.masters != old_masters {
        let target_masters = context.header.masters.clone();
        let target_own_index = target_masters.len() as u8;
        remap_formids_in_items(
            &mut slot.parsed.root_items,
            &old_masters,
            &target_masters,
            old_own_index,
            target_own_index,
        );
    }
    let masters_changed = context.header.masters != old_masters;
    slot.parsed.header = context.header;
    slot.strings = context.strings;

    if record.form_id == 0 {
        let object_id = slot.parsed.header.next_object_id & 0x00FF_FFFF;
        record.form_id = ((slot.parsed.header.masters.len() as u32) << 24) | object_id;
        slot.parsed.header.next_object_id = (object_id + 1) & 0x00FF_FFFF;
    }

    let own_plugin_name: Arc<str> = Arc::from(slot.parsed.plugin_name.as_str());
    let form_key = resolve_form_id_to_form_key(
        record.form_id,
        &own_plugin_name,
        &slot.parsed.header.masters,
    )
    .to_string();
    let header_size = slot.parsed.header_size;
    let game = slot.parsed.game.clone();
    let _ = remove_record_from_items(&mut slot.parsed.root_items, record.form_id);
    ensure_top_group_and_add(
        &mut slot.parsed.root_items,
        record,
        header_size,
        game.as_deref(),
    );
    slot.record_count_cache = None;
    slot.sections.apply_effect(if masters_changed {
        &WriteEffect::MastersChanged
    } else {
        &WriteEffect::RecordsAddedOrRemoved
    });
    Ok(form_key)
}

pub fn plugin_handle_replace_authoring_record_values(
    handle_id: u64,
    values: &[JsonValue],
) -> PyResult<Vec<String>> {
    if values.is_empty() {
        return Ok(Vec::new());
    }

    fn parse_values(
        values: &[JsonValue],
        context: &mut NativeImportContext,
    ) -> PyResult<Vec<ParsedRecord>> {
        values
            .iter()
            .map(|value| authoring_value_to_record(value, context))
            .collect()
    }

    let mut store = plugin_handle_store().lock().unwrap();
    let slot = store
        .get_mut(&handle_id)
        .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
    let old_masters = slot.parsed.header.masters.clone();
    let old_own_index = old_masters.len() as u8;
    slot.strings.materialize_all();
    let old_strings = slot.strings.clone();
    let mut context = NativeImportContext::new(
        slot.parsed.plugin_name.clone(),
        slot.parsed.game.clone(),
        slot.parsed.header_size,
        slot.parsed.header.clone(),
    );
    context.strings = old_strings.clone();

    let mut records = parse_values(values, &mut context)?;
    if context.header.masters != old_masters {
        let mut reparsed_context = NativeImportContext::new(
            slot.parsed.plugin_name.clone(),
            slot.parsed.game.clone(),
            slot.parsed.header_size,
            context.header.clone(),
        );
        reparsed_context.strings = old_strings;
        records = parse_values(values, &mut reparsed_context)?;
        context = reparsed_context;
    }
    if context.header.masters != old_masters {
        let target_masters = context.header.masters.clone();
        let target_own_index = target_masters.len() as u8;
        remap_formids_in_items(
            &mut slot.parsed.root_items,
            &old_masters,
            &target_masters,
            old_own_index,
            target_own_index,
        );
    }
    let masters_changed = context.header.masters != old_masters;
    slot.parsed.header = context.header;
    slot.strings = context.strings;

    for record in &mut records {
        if record.form_id == 0 {
            let object_id = slot.parsed.header.next_object_id & 0x00FF_FFFF;
            record.form_id = ((slot.parsed.header.masters.len() as u32) << 24) | object_id;
            slot.parsed.header.next_object_id = (object_id + 1) & 0x00FF_FFFF;
        }
    }

    let own_plugin_name: Arc<str> = Arc::from(slot.parsed.plugin_name.as_str());
    let form_keys = records
        .iter()
        .map(|record| {
            resolve_form_id_to_form_key(
                record.form_id,
                &own_plugin_name,
                &slot.parsed.header.masters,
            )
            .to_string()
        })
        .collect();
    replace_parsed_records_in_slot_batch(slot, records);
    slot.record_count_cache = None;
    slot.sections.apply_effect(if masters_changed {
        &WriteEffect::MastersChanged
    } else {
        &WriteEffect::RecordsAddedOrRemoved
    });
    Ok(form_keys)
}

pub fn plugin_handle_replace_projected_cell_authoring_record_value(
    handle_id: u64,
    value: &JsonValue,
    relative_path: &str,
) -> PyResult<Vec<String>> {
    let payload = json_object(value, "projected CELL")?;
    let location = parse_projected_cell_relative_path(relative_path)?;
    let mut store = plugin_handle_store().lock().unwrap();
    let slot = store
        .get_mut(&handle_id)
        .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
    let old_masters = slot.parsed.header.masters.clone();
    let old_own_index = old_masters.len() as u8;
    slot.strings.materialize_all();
    let old_strings = slot.strings.clone();
    let mut context = NativeImportContext::new(
        slot.parsed.plugin_name.clone(),
        slot.parsed.game.clone(),
        slot.parsed.header_size,
        slot.parsed.header.clone(),
    );
    context.strings = old_strings.clone();

    let mut projected = parse_projected_cell_authoring_payload(payload, &mut context)?;
    if context.header.masters != old_masters {
        let mut reparsed_context = NativeImportContext::new(
            slot.parsed.plugin_name.clone(),
            slot.parsed.game.clone(),
            slot.parsed.header_size,
            context.header.clone(),
        );
        reparsed_context.strings = old_strings;
        projected = parse_projected_cell_authoring_payload(payload, &mut reparsed_context)?;
        context = reparsed_context;
    }
    if context.header.masters != old_masters {
        let target_masters = context.header.masters.clone();
        let target_own_index = target_masters.len() as u8;
        remap_formids_in_items(
            &mut slot.parsed.root_items,
            &old_masters,
            &target_masters,
            old_own_index,
            target_own_index,
        );
    }
    let masters_changed = context.header.masters != old_masters;
    slot.parsed.header = context.header;
    slot.strings = context.strings;

    assign_missing_projected_cell_form_ids(&mut projected, &mut slot.parsed.header);
    ensure_projected_cell_grid_subrecord(&mut projected.cell, location.cell);
    advance_next_object_id_for_projected_cell(&projected, &mut slot.parsed.header);

    let own_plugin_name: Arc<str> = Arc::from(slot.parsed.plugin_name.as_str());
    let mut form_keys = Vec::new();
    form_keys.push(
        resolve_form_id_to_form_key(
            projected.cell.form_id,
            &own_plugin_name,
            &slot.parsed.header.masters,
        )
        .to_string(),
    );
    for record in projected_child_records(projected.child_group.as_ref()) {
        form_keys.push(
            resolve_form_id_to_form_key(
                record.form_id,
                &own_plugin_name,
                &slot.parsed.header.masters,
            )
            .to_string(),
        );
    }

    let header_size = slot.parsed.header_size;
    let plugin_name = slot.parsed.plugin_name.clone();
    let cell_form_id = projected.cell.form_id;
    let _ = remove_record_from_items(&mut slot.parsed.root_items, cell_form_id);
    let _ = remove_cell_child_group_from_items(&mut slot.parsed.root_items, cell_form_id);
    ensure_wrld_group_and_add_projected_cell(
        &mut slot.parsed.root_items,
        projected,
        &location,
        header_size,
        &plugin_name,
    )?;
    slot.record_count_cache = None;
    slot.sections.apply_effect(if masters_changed {
        &WriteEffect::MastersChanged
    } else {
        &WriteEffect::RecordsAddedOrRemoved
    });
    Ok(form_keys)
}

pub fn plugin_handle_replace_projected_cell_authoring_record_values_at_locations(
    handle_id: u64,
    values: Vec<(JsonValue, String)>,
) -> PyResult<usize> {
    if values.is_empty() {
        return Ok(0);
    }

    let mut payloads = Vec::with_capacity(values.len());
    for (value, relative_path) in values {
        payloads.push(ProjectedCellAuthoringPayload {
            payload: json_object(&value, "projected CELL")?.clone(),
            location: parse_projected_cell_relative_path(&relative_path)?,
        });
    }

    plugin_handle_import_projected_cell_authoring_payloads(handle_id, &payloads)
}

struct ProjectedCellAuthoringPayload {
    payload: JsonMap<String, JsonValue>,
    location: ProjectedCellLocation,
}

struct ProjectedCellImport {
    cell: ParsedRecord,
    child_group: Option<ParsedGroup>,
}

#[derive(Default)]
struct ExtractedProjectedCell {
    cell: Option<ParsedRecord>,
    child_groups: Vec<ParsedGroup>,
}

#[derive(Clone)]
struct ProjectedCellLocation {
    world_dir: String,
    block: (i16, i16),
    subblock: (i16, i16),
    cell: (i16, i16),
}

fn plugin_handle_import_projected_cell_authoring_payloads(
    handle_id: u64,
    payloads: &[ProjectedCellAuthoringPayload],
) -> PyResult<usize> {
    let mut store = plugin_handle_store().lock().unwrap();
    let slot = store
        .get_mut(&handle_id)
        .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
    let old_masters = slot.parsed.header.masters.clone();
    let old_own_index = old_masters.len() as u8;
    slot.strings.materialize_all();
    let old_strings = slot.strings.clone();
    let mut context = NativeImportContext::new(
        slot.parsed.plugin_name.clone(),
        slot.parsed.game.clone(),
        slot.parsed.header_size,
        slot.parsed.header.clone(),
    );
    context.strings = old_strings.clone();

    let mut projected_cells = parse_projected_cell_authoring_payloads(payloads, &mut context)?;
    if context.header.masters != old_masters {
        let mut reparsed_context = NativeImportContext::new(
            slot.parsed.plugin_name.clone(),
            slot.parsed.game.clone(),
            slot.parsed.header_size,
            context.header.clone(),
        );
        reparsed_context.strings = old_strings;
        projected_cells = parse_projected_cell_authoring_payloads(payloads, &mut reparsed_context)?;
        context = reparsed_context;
    }
    if context.header.masters != old_masters {
        let target_masters = context.header.masters.clone();
        let target_own_index = target_masters.len() as u8;
        remap_formids_in_items(
            &mut slot.parsed.root_items,
            &old_masters,
            &target_masters,
            old_own_index,
            target_own_index,
        );
    }
    let masters_changed = context.header.masters != old_masters;
    slot.parsed.header = context.header;
    slot.strings = context.strings;

    for (projected, location) in projected_cells.iter_mut() {
        assign_missing_projected_cell_form_ids(projected, &mut slot.parsed.header);
        ensure_projected_cell_grid_subrecord(&mut projected.cell, location.cell);
        advance_next_object_id_for_projected_cell(projected, &mut slot.parsed.header);
    }

    let imported_count: usize = projected_cells
        .iter()
        .map(|(projected, _)| 1 + count_projected_child_records(projected.child_group.as_ref()))
        .sum();

    let header_size = slot.parsed.header_size;
    let plugin_name = slot.parsed.plugin_name.clone();
    let mut extracted_matches = extract_projected_cell_matches_for_batch(
        &mut slot.parsed.root_items,
        &projected_cells,
        &plugin_name,
    );
    for (projected, location) in projected_cells {
        let existing_match =
            take_extracted_projected_cell_match(&mut extracted_matches, &projected, &location);
        ensure_wrld_group_and_add_projected_cell_fast(
            &mut slot.parsed.root_items,
            projected,
            &location,
            existing_match,
            header_size,
            &plugin_name,
        )?;
    }
    slot.record_count_cache = None;
    slot.sections.apply_effect(if masters_changed {
        &WriteEffect::MastersChanged
    } else {
        &WriteEffect::RecordsAddedOrRemoved
    });
    Ok(imported_count)
}

fn parse_projected_cell_authoring_payloads(
    payloads: &[ProjectedCellAuthoringPayload],
    context: &mut NativeImportContext,
) -> PyResult<Vec<(ProjectedCellImport, ProjectedCellLocation)>> {
    let mut projected_cells = Vec::with_capacity(payloads.len());
    for payload in payloads {
        projected_cells.push((
            parse_projected_cell_authoring_payload(&payload.payload, context)?,
            payload.location.clone(),
        ));
    }
    Ok(projected_cells)
}

fn extract_projected_cell_matches_for_batch(
    root_items: &mut Vec<ParsedItem>,
    projected_cells: &[(ProjectedCellImport, ProjectedCellLocation)],
    plugin_name: &str,
) -> HashMap<(String, String), ExtractedProjectedCell> {
    let mut wanted_editor_ids_by_world: HashMap<String, HashSet<String>> = HashMap::new();
    for (projected, location) in projected_cells {
        if let Some(editor_id) = record_editor_id_value(&projected.cell) {
            wanted_editor_ids_by_world
                .entry(location.world_dir.clone())
                .or_default()
                .insert(normalize_editor_id_for_match(&editor_id));
        }
    }

    let mut extracted = HashMap::new();
    for (world_dir, wanted_editor_ids) in wanted_editor_ids_by_world {
        let Some(world_form_id) =
            find_projected_cell_world_form_id(root_items, &world_dir, plugin_name)
        else {
            continue;
        };
        let Some(world_group) = projected_world_children_group_mut(root_items, world_form_id)
        else {
            continue;
        };
        extract_projected_cell_matches_from_items(
            &mut world_group.children,
            &world_dir,
            &wanted_editor_ids,
            &mut extracted,
        );
    }
    extracted
}

fn take_extracted_projected_cell_match(
    extracted_matches: &mut HashMap<(String, String), ExtractedProjectedCell>,
    projected: &ProjectedCellImport,
    location: &ProjectedCellLocation,
) -> Option<ExtractedProjectedCell> {
    let editor_id = record_editor_id_value(&projected.cell)?;
    extracted_matches.remove(&(
        location.world_dir.clone(),
        normalize_editor_id_for_match(&editor_id),
    ))
}

fn parse_projected_cell_relative_path(relative_path: &str) -> PyResult<ProjectedCellLocation> {
    let normalized = relative_path.replace('\\', "/");
    let parts: Vec<&str> = normalized
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    if parts.len() != 7
        || parts[0] != "records"
        || parts[1] != "WRLD"
        || parts[6] != "RecordData.yaml"
    {
        return Err(value_error(format!(
            "projected CELL relative path must be records/WRLD/<world>/<block>/<subblock>/<cell>/RecordData.yaml: {relative_path}"
        )));
    }
    let block = parse_grid_dir_name(parts[3]).ok_or_else(|| {
        value_error(format!(
            "projected CELL block directory must be '<x>, <y>': {}",
            parts[3]
        ))
    })?;
    let subblock = parse_grid_dir_name(parts[4]).ok_or_else(|| {
        value_error(format!(
            "projected CELL subblock directory must be '<x>, <y>': {}",
            parts[4]
        ))
    })?;
    let cell = parse_grid_dir_name(parts[5]).ok_or_else(|| {
        value_error(format!(
            "projected CELL directory must be '<x>, <y>': {}",
            parts[5]
        ))
    })?;
    Ok(ProjectedCellLocation {
        world_dir: parts[2].to_string(),
        block,
        subblock,
        cell,
    })
}

fn parse_projected_cell_authoring_payload(
    payload: &JsonMap<String, JsonValue>,
    context: &mut NativeImportContext,
) -> PyResult<ProjectedCellImport> {
    let mut cell_only = JsonMap::with_capacity(8);
    for key in [
        "signature",
        "form_id",
        "flags",
        "version_control",
        "form_version",
        "version2",
        "subrecords",
        "fields",
        "fields_by_signature",
        "eid",
        "editor_id",
        "raw_payload_hex",
        "parse_error",
    ] {
        if let Some(value) = payload.get(key) {
            cell_only.insert(key.to_string(), value.clone());
        }
    }
    if !cell_only.contains_key("signature") {
        cell_only.insert(
            "signature".to_string(),
            JsonValue::String("CELL".to_string()),
        );
    }
    let cell = parse_record_from_json_compact_native(&cell_only, context)?;
    let child_group = parse_projected_cell_child_group(payload, cell.form_id, context)?;
    Ok(ProjectedCellImport { cell, child_group })
}

fn parse_projected_cell_child_group(
    payload: &JsonMap<String, JsonValue>,
    cell_form_id: u32,
    context: &mut NativeImportContext,
) -> PyResult<Option<ParsedGroup>> {
    let mut children = Vec::new();
    let mut temporary_children = Vec::new();

    if let Some(value) = payload.get("Landscape") {
        let mut mapping = json_object(value, "Landscape")?.clone();
        mapping.insert(
            "signature".to_string(),
            JsonValue::String("LAND".to_string()),
        );
        temporary_children.push(ParsedItem::Record(parse_record_from_json_compact_native(
            &mapping, context,
        )?));
    }

    if let Some(value) = payload.get("NavigationMeshes") {
        for entry in json_array(value, "NavigationMeshes")? {
            let mut mapping = json_object(entry, "NavigationMeshes[]")?.clone();
            mapping.insert(
                "signature".to_string(),
                JsonValue::String("NAVM".to_string()),
            );
            temporary_children.push(ParsedItem::Record(parse_record_from_json_compact_native(
                &mapping, context,
            )?));
        }
    }

    for (section_name, group_type) in [
        ("Persistent", PERSISTENT_GROUP),
        ("Temporary", TEMPORARY_GROUP),
        ("VisibleWhenDistant", VISIBLE_DISTANT_GROUP),
    ] {
        let section_present = payload.get(section_name).is_some();
        let mut section_children = Vec::new();
        if let Some(value) = payload.get(section_name) {
            for entry in json_array(value, section_name)? {
                let mut mapping = json_object(entry, section_name)?.clone();
                if !mapping.contains_key("signature") {
                    mapping.insert(
                        "signature".to_string(),
                        JsonValue::String("REFR".to_string()),
                    );
                }
                section_children.push(ParsedItem::Record(parse_record_from_json_compact_native(
                    &mapping, context,
                )?));
            }
        }
        if group_type == TEMPORARY_GROUP {
            temporary_children.extend(section_children);
            if !section_present && temporary_children.is_empty() {
                continue;
            }
            children.push(ParsedItem::Group(ParsedGroup {
                label: cell_form_id.to_le_bytes(),
                group_type,
                tail: Bytes::from(vec![0u8; context.header_size.saturating_sub(16)]),
                children: std::mem::take(&mut temporary_children),
            }));
            continue;
        }
        if !section_present {
            continue;
        }
        children.push(ParsedItem::Group(ParsedGroup {
            label: cell_form_id.to_le_bytes(),
            group_type,
            tail: Bytes::from(vec![0u8; context.header_size.saturating_sub(16)]),
            children: section_children,
        }));
    }

    if children.is_empty() {
        return Ok(None);
    }
    Ok(Some(ParsedGroup {
        label: cell_form_id.to_le_bytes(),
        group_type: CELL_CHILD_GROUP,
        tail: Bytes::from(vec![0u8; context.header_size.saturating_sub(16)]),
        children,
    }))
}

fn assign_missing_projected_cell_form_ids(
    projected: &mut ProjectedCellImport,
    header: &mut ParsedPluginHeader,
) {
    let own_index = header.masters.len() as u32;
    if projected.cell.form_id == 0 {
        let object_id = header.next_object_id & 0x00FF_FFFF;
        projected.cell.form_id = (own_index << 24) | object_id;
        header.next_object_id = (object_id + 1) & 0x00FF_FFFF;
    }
    if let Some(group) = projected.child_group.as_mut() {
        group.label = projected.cell.form_id.to_le_bytes();
        update_cell_section_group_labels(&mut group.children, projected.cell.form_id);
        assign_missing_form_ids_in_items(&mut group.children, header);
    }
}

fn update_cell_section_group_labels(items: &mut [ParsedItem], cell_form_id: u32) {
    let label = cell_form_id.to_le_bytes();
    for item in items {
        if let ParsedItem::Group(group) = item {
            if matches!(
                group.group_type,
                PERSISTENT_GROUP | TEMPORARY_GROUP | VISIBLE_DISTANT_GROUP
            ) {
                group.label = label;
            }
            update_cell_section_group_labels(&mut group.children, cell_form_id);
        }
    }
}

fn assign_missing_form_ids_in_items(items: &mut [ParsedItem], header: &mut ParsedPluginHeader) {
    let own_index = header.masters.len() as u32;
    for item in items {
        match item {
            ParsedItem::Record(record) if record.form_id == 0 => {
                let object_id = header.next_object_id & 0x00FF_FFFF;
                record.form_id = (own_index << 24) | object_id;
                header.next_object_id = (object_id + 1) & 0x00FF_FFFF;
            }
            ParsedItem::Group(group) => {
                assign_missing_form_ids_in_items(&mut group.children, header)
            }
            _ => {}
        }
    }
}

fn advance_next_object_id_for_projected_cell(
    projected: &ProjectedCellImport,
    header: &mut ParsedPluginHeader,
) {
    advance_next_object_id_for_form_id(projected.cell.form_id, header);
    if let Some(group) = projected.child_group.as_ref() {
        for record in projected_child_records(Some(group)) {
            advance_next_object_id_for_form_id(record.form_id, header);
        }
    }
}

fn advance_next_object_id_for_form_id(form_id: u32, header: &mut ParsedPluginHeader) {
    let object_id = form_id & 0x00FF_FFFF;
    let next_object_id = header.next_object_id & 0x00FF_FFFF;
    if object_id != 0 && object_id >= next_object_id {
        header.next_object_id = (object_id + 1) & 0x00FF_FFFF;
    }
}

fn projected_child_records(group: Option<&ParsedGroup>) -> Vec<&ParsedRecord> {
    fn collect<'a>(items: &'a [ParsedItem], out: &mut Vec<&'a ParsedRecord>) {
        for item in items {
            match item {
                ParsedItem::Record(record) => out.push(record),
                ParsedItem::Group(group) => collect(&group.children, out),
            }
        }
    }
    let mut records = Vec::new();
    if let Some(group) = group {
        collect(&group.children, &mut records);
    }
    records
}

fn count_projected_child_records(group: Option<&ParsedGroup>) -> usize {
    fn count(items: &[ParsedItem]) -> usize {
        items
            .iter()
            .map(|item| match item {
                ParsedItem::Record(_) => 1,
                ParsedItem::Group(group) => count(&group.children),
            })
            .sum()
    }
    group.map(|group| count(&group.children)).unwrap_or(0)
}

/// Insert a pre-built `ParsedRecord` into a plugin handle.
///
/// The function advances `next_object_id` if the record's object-ID exceeds
/// the current watermark, and invalidates the index sections.
pub fn insert_parsed_record(handle_id: u64, record: ParsedRecord) -> Result<(), String> {
    let mut store = plugin_handle_store().lock().unwrap();
    let slot = store
        .get_mut(&handle_id)
        .ok_or_else(|| format!("no plugin handle: {handle_id}"))?;

    let object_id = record.form_id & 0x00FF_FFFF;
    let next_object_id = slot.parsed.header.next_object_id & 0x00FF_FFFF;
    if object_id != 0
        && object_id >= next_object_id
        && should_advance_next_object_id_for_form_id(&slot.parsed.header, record.form_id)
    {
        slot.parsed.header.next_object_id = (object_id + 1) & 0x00FF_FFFF;
    }

    let header_size = slot.parsed.header_size;
    let game = slot.parsed.game.clone();
    ensure_top_group_and_add(
        &mut slot.parsed.root_items,
        record,
        header_size,
        game.as_deref(),
    );
    slot.record_count_cache = None;
    slot.sections
        .apply_effect(&WriteEffect::RecordsAddedOrRemoved);
    Ok(())
}

fn should_advance_next_object_id_for_form_id(header: &ParsedPluginHeader, form_id: u32) -> bool {
    let master_index = ((form_id >> 24) & 0xFF) as u8;
    let own_index = (header.masters.len() & 0xFF) as u8;
    master_index == LOCAL_FORM_INDEX
        || master_index == own_index
        || master_index as usize >= header.masters.len()
}

pub fn replace_parsed_records(handle_id: u64, records: Vec<ParsedRecord>) -> Result<(), String> {
    if records.is_empty() {
        return Ok(());
    }

    let mut store = plugin_handle_store().lock().unwrap();
    let slot = store
        .get_mut(&handle_id)
        .ok_or_else(|| format!("no plugin handle: {handle_id}"))?;

    let header_size = slot.parsed.header_size;
    let game = slot.parsed.game.clone();
    for record in records {
        let object_id = record.form_id & 0x00FF_FFFF;
        let next_object_id = slot.parsed.header.next_object_id & 0x00FF_FFFF;
        if object_id != 0
            && object_id >= next_object_id
            && should_advance_next_object_id_for_form_id(&slot.parsed.header, record.form_id)
        {
            slot.parsed.header.next_object_id = (object_id + 1) & 0x00FF_FFFF;
        }

        let _ = remove_record_from_items(&mut slot.parsed.root_items, record.form_id);
        ensure_top_group_and_add(
            &mut slot.parsed.root_items,
            record,
            header_size,
            game.as_deref(),
        );
    }

    slot.record_count_cache = None;
    slot.sections
        .apply_effect(&WriteEffect::RecordsAddedOrRemoved);
    Ok(())
}

pub fn replace_parsed_record_in_slot(slot: &mut NativePluginSlot, record: ParsedRecord) {
    let object_id = record.form_id & 0x00FF_FFFF;
    let next_object_id = slot.parsed.header.next_object_id & 0x00FF_FFFF;
    if object_id != 0
        && object_id >= next_object_id
        && should_advance_next_object_id_for_form_id(&slot.parsed.header, record.form_id)
    {
        slot.parsed.header.next_object_id = (object_id + 1) & 0x00FF_FFFF;
    }

    let header_size = slot.parsed.header_size;
    let game = slot.parsed.game.clone();
    let _ = remove_record_from_items(&mut slot.parsed.root_items, record.form_id);
    ensure_top_group_and_add(
        &mut slot.parsed.root_items,
        record,
        header_size,
        game.as_deref(),
    );
    slot.clear_record_count_cache();
}

/// Structurally replace many records while matching repeated single-record
/// replacement order. Unique form IDs use one tree-removal pass; duplicate
/// replacement IDs fall back to the sequential path because later replacements
/// may remove records appended by earlier ones.
pub fn replace_parsed_records_in_slot_batch(
    slot: &mut NativePluginSlot,
    records: Vec<ParsedRecord>,
) {
    if records.is_empty() {
        return;
    }

    let mut targets = HashSet::with_capacity(records.len());
    if records.iter().any(|record| !targets.insert(record.form_id)) {
        for record in records {
            replace_parsed_record_in_slot(slot, record);
        }
        return;
    }

    for record in &records {
        let object_id = record.form_id & 0x00FF_FFFF;
        let next_object_id = slot.parsed.header.next_object_id & 0x00FF_FFFF;
        if object_id != 0
            && object_id >= next_object_id
            && should_advance_next_object_id_for_form_id(&slot.parsed.header, record.form_id)
        {
            slot.parsed.header.next_object_id = (object_id + 1) & 0x00FF_FFFF;
        }
    }

    remove_first_records_from_items(&mut slot.parsed.root_items, &mut targets);
    let header_size = slot.parsed.header_size;
    let game = slot.parsed.game.clone();
    for record in records {
        ensure_top_group_and_add(
            &mut slot.parsed.root_items,
            record,
            header_size,
            game.as_deref(),
        );
    }
    slot.clear_record_count_cache();
}

pub fn insert_parsed_record_in_slot(slot: &mut NativePluginSlot, record: ParsedRecord) {
    let object_id = record.form_id & 0x00FF_FFFF;
    let next_object_id = slot.parsed.header.next_object_id & 0x00FF_FFFF;
    if object_id != 0 && object_id >= next_object_id {
        slot.parsed.header.next_object_id = (object_id + 1) & 0x00FF_FFFF;
    }

    let header_size = slot.parsed.header_size;
    let game = slot.parsed.game.clone();
    ensure_top_group_and_add(
        &mut slot.parsed.root_items,
        record,
        header_size,
        game.as_deref(),
    );
    slot.clear_record_count_cache();
}

pub fn replace_parsed_record_contents_in_slot(
    slot: &mut NativePluginSlot,
    replacement: ParsedRecord,
) -> bool {
    let form_id = replacement.form_id;
    let replaced = replace_record_contents_in_items(&mut slot.parsed.root_items, &replacement);
    if replaced {
        slot.sections.apply_effect(&WriteEffect::RecordContents {
            form_ids: smallvec::smallvec![form_id],
        });
    }
    replaced
}

/// Walk every record in the target plugin handle and rewrite form-key reference
/// strings in their authoring-dict fields using `mappings` (keys and values in
/// `"OBJID:Plugin"` format, matching the authoring-dir convention).
///
/// Returns the number of records that had at least one reference rewritten.
pub fn plugin_handle_rewrite_references(
    handle_id: u64,
    mappings: &std::collections::HashMap<String, String>,
) -> Result<usize, String> {
    if mappings.is_empty() {
        return Ok(0);
    }

    // Serialize mappings once as a JSON object string for replace_formkeys_batch_json.
    // replace_formkeys accepts flat string→string mappings (same format used by
    // the Python FormKeyMapper.rewrite_formkeys and ConversionFixups._replace_formkeys).
    let mappings_json = serde_json::to_string(
        &mappings
            .iter()
            .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
            .collect::<serde_json::Map<_, _>>(),
    )
    .map_err(|e| format!("failed to serialize mappings: {e}"))?;

    // Phase 1: collect all (form_key_str → rewritten_json) pairs that need updating.
    // We lock the store once for the read pass, then release before writing so that
    // plugin_handle_replace_authoring_record_value can acquire the lock per-record.
    let updates: Vec<(String, serde_json::Value)> = {
        let mut store = plugin_handle_store().lock().unwrap();
        let slot = store
            .get_mut(&handle_id)
            .ok_or_else(|| format!("unknown plugin handle: {handle_id}"))?;

        // Collect all form key strings from the core index.
        let core = ensure_core_section(slot);
        let all_fk_strings: Vec<String> = core.by_form_key.keys().map(|fk| fk.render()).collect();
        drop(core);

        let mut pending: Vec<(String, serde_json::Value)> = Vec::new();

        for fk_str in &all_fk_strings {
            let core2 = ensure_core_section(slot);
            let Some(entry) = record_index_entry_by_form_key(&core2, fk_str) else {
                continue;
            };
            let raw_form_id = entry.raw_form_id;
            drop(core2);

            let records = ensure_records_section(slot);
            let Some(record) = records.record(&slot.parsed, raw_form_id) else {
                continue;
            };
            let sig_str = record.signature.to_string();
            let mut original_json = record_as_authoring_value(record, &slot.parsed, &slot.strings);

            // Ensure the authoring value includes "signature" so that the replace
            // path can reconstruct the record (record_as_authoring_value omits it).
            if let serde_json::Value::Object(ref mut map) = original_json {
                map.entry("signature".to_string())
                    .or_insert_with(|| serde_json::Value::String(sig_str));
            }

            // Rewrite FK references via replace_formkeys (flat string→string mapping).
            let original_str = serde_json::to_string(&original_json)
                .map_err(|e| format!("serialize error: {e}"))?;
            let array_str = format!("[{}]", original_str);
            let rewritten_array =
                crate::formkey_ops::replace_formkeys_batch_json(&array_str, &mappings_json)?;

            if rewritten_array == array_str {
                continue;
            }

            let mut rewritten_values: Vec<serde_json::Value> =
                serde_json::from_str(&rewritten_array)
                    .map_err(|e| format!("parse rewritten error: {e}"))?;
            if let Some(rewritten_value) = rewritten_values.drain(..).next() {
                pending.push((fk_str.clone(), rewritten_value));
            }
        }
        pending
    }; // lock released here

    // Phase 2: write back each changed record via the full replace path
    // (which handles master-list updates, form-id remapping, etc.).
    let count = updates.len();
    for (_fk_str, rewritten_value) in updates {
        plugin_handle_replace_authoring_record_value(handle_id, &rewritten_value)
            .map_err(|e| format!("replace error: {e}"))?;
    }

    Ok(count)
}

#[allow(dead_code)]
fn clone_plugin_handle_parsed(handle_id: u64) -> PyResult<ParsedPlugin> {
    let store = plugin_handle_store().lock().unwrap();
    let slot = store
        .get(&handle_id)
        .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
    Ok(slot.parsed.clone())
}

pub fn update_plugin_handle_saved_path(handle_id: u64, path: &str) {
    let mut store = plugin_handle_store().lock().unwrap();
    if let Some(slot) = store.get_mut(&handle_id) {
        let old_plugin_name = slot.parsed.plugin_name.clone();
        slot.parsed.file_path = path.to_string();
        if let Some(name) = Path::new(path).file_name().and_then(|value| value.to_str()) {
            slot.parsed.plugin_name = name.to_string();
        }
        slot.sections
            .apply_effect(if slot.parsed.plugin_name != old_plugin_name {
                &WriteEffect::MastersChanged
            } else {
                &WriteEffect::HeaderOnly
            });
    }
}

/// Save a plugin handle to disk without requiring the Python GIL.
///
/// Called from Rust-native phases (e.g. `build_esp`) that run after
/// `py.allow_threads` has been entered, making the GIL unavailable.
/// Maps internal `PyErr` failures to `String` errors so callers need
/// not deal with PyO3 types.
fn plugin_handle_write_no_py(handle_id: u64, output_path: &str) -> Result<(), String> {
    // Save the slot's tree in place rather than cloning it: the ParsedPlugin can
    // be tens of GB on a full conversion, and a clone lands on the Build-ESP
    // peak. The write path mutates via `rewrite_semantic_formids_in_place`
    // (idempotent: it only rewrites FF-prefixed FormIDs to the own-master index)
    // and never re-locks the handle store, so holding the lock can't deadlock.
    {
        let mut store = plugin_handle_store()
            .lock()
            .map_err(|e| format!("plugin handle store lock poisoned: {e}"))?;
        let slot = store
            .get_mut(&handle_id)
            .ok_or_else(|| format!("unknown plugin handle: {handle_id}"))?;
        io::save_parsed_plugin_no_py(&mut slot.parsed, &slot.strings, output_path)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Write a mid-run snapshot without changing the handle's plugin name or path.
pub fn plugin_handle_save_preserving_identity_no_py(
    handle_id: u64,
    output_path: &str,
) -> Result<(), String> {
    plugin_handle_write_no_py(handle_id, output_path)
}

pub fn plugin_handle_save_no_py(handle_id: u64, output_path: &str) -> Result<(), String> {
    plugin_handle_write_no_py(handle_id, output_path)?;
    update_plugin_handle_saved_path(handle_id, output_path);
    Ok(())
}

fn empty_parsed_plugin(plugin_name: &str, game: Option<&str>) -> ParsedPlugin {
    let legacy = matches!(game, Some("oblivion" | "fo3" | "fnv"));
    let header_size = if legacy {
        LEGACY_HEADER_SIZE
    } else {
        MODERN_HEADER_SIZE
    };
    let hedr_version: f32 = match game {
        Some("oblivion") => 0.8,
        Some("fo3" | "fnv") => 0.94,
        _ => 1.0,
    };
    ParsedPlugin {
        plugin_name: plugin_name.to_string(),
        file_path: String::new(),
        header_size,
        header: ParsedPluginHeader {
            version: hedr_version,
            num_records: 0,
            next_object_id: 0x000800,
            author: String::new(),
            description: String::new(),
            masters: Vec::new(),
            master_sizes: Vec::new(),
            overridden_forms: Vec::new(),
            flags: 0,
            extra_subrecords: Vec::new(),
            version_control: 0,
            form_version: if legacy { None } else { Some(131) },
            version2: if legacy { None } else { Some(0) },
            hedr_raw: None,
            raw_subrecords: Vec::new(),
        },
        root_items: Vec::new(),
        game: game.map(|g| g.to_string()),
    }
}

fn parsed_subrecord_payload(subrecord: &ParsedSubrecord) -> ParsedSubrecordPayload {
    (
        subrecord.signature.to_string(),
        subrecord.data.as_ref().to_vec(),
        subrecord
            .semantic_type
            .as_ref()
            .map(|value| value.to_string()),
    )
}

fn parsed_header_payload(header: &ParsedPluginHeader) -> ParsedHeaderPayload {
    (
        header.version,
        header.num_records,
        header.next_object_id,
        header.author.clone(),
        header.description.clone(),
        header.masters.clone(),
        header.master_sizes.clone(),
        header.overridden_forms.clone(),
        header.flags,
        header
            .extra_subrecords
            .iter()
            .map(parsed_subrecord_payload)
            .collect(),
        header.version_control,
        header.form_version,
        header.version2,
        header.hedr_raw.as_ref().map(|raw| raw.as_ref().to_vec()),
        header
            .raw_subrecords
            .iter()
            .map(parsed_subrecord_payload)
            .collect(),
    )
}

fn plugin_metadata_payload(snapshot: &PluginMetadataSnapshot) -> PluginMetadataPayload {
    (
        snapshot.plugin_name.clone(),
        snapshot.file_path.clone(),
        snapshot.game.clone(),
        snapshot.header_size,
        parsed_header_payload(&snapshot.header),
        snapshot.record_count,
        snapshot.localized_default_language.clone(),
    )
}

fn plugin_legacy_metadata_payload(
    snapshot: &PluginMetadataSnapshot,
    strings_state: &LocalizedStringsState,
) -> PluginLegacyMetadataPayload {
    (
        plugin_metadata_payload(snapshot),
        plugin_strings_payload(strings_state, None),
    )
}

fn parsed_subrecord_payload_to_py(
    py: Python<'_>,
    payload: &ParsedSubrecordPayload,
) -> PyResult<Py<PyAny>> {
    PyTuple::new(
        py,
        [
            payload.0.clone().into_py_any(py)?,
            PyBytes::new(py, &payload.1).into_any().unbind(),
            match &payload.2 {
                Some(value) => value.clone().into_py_any(py)?,
                None => py.None(),
            },
        ],
    )?
    .into_py_any(py)
}

fn parsed_header_payload_to_py(
    py: Python<'_>,
    payload: &ParsedHeaderPayload,
) -> PyResult<Py<PyAny>> {
    let extra_subrecords = payload
        .9
        .iter()
        .map(|item| parsed_subrecord_payload_to_py(py, item))
        .collect::<PyResult<Vec<_>>>()?;
    let raw_subrecords = payload
        .14
        .iter()
        .map(|item| parsed_subrecord_payload_to_py(py, item))
        .collect::<PyResult<Vec<_>>>()?;
    PyTuple::new(
        py,
        [
            payload.0.into_py_any(py)?,
            payload.1.into_py_any(py)?,
            payload.2.into_py_any(py)?,
            payload.3.clone().into_py_any(py)?,
            payload.4.clone().into_py_any(py)?,
            payload.5.clone().into_py_any(py)?,
            payload.6.clone().into_py_any(py)?,
            payload.7.clone().into_py_any(py)?,
            payload.8.into_py_any(py)?,
            extra_subrecords.into_py_any(py)?,
            payload.10.into_py_any(py)?,
            payload.11.into_py_any(py)?,
            payload.12.into_py_any(py)?,
            match &payload.13 {
                Some(raw) => PyBytes::new(py, raw).into_any().unbind(),
                None => py.None(),
            },
            raw_subrecords.into_py_any(py)?,
        ],
    )?
    .into_py_any(py)
}

fn plugin_metadata_payload_to_py(
    py: Python<'_>,
    payload: &PluginMetadataPayload,
) -> PyResult<Py<PyAny>> {
    PyTuple::new(
        py,
        [
            payload.0.clone().into_py_any(py)?,
            payload.1.clone().into_py_any(py)?,
            payload.2.clone().into_py_any(py)?,
            payload.3.into_py_any(py)?,
            parsed_header_payload_to_py(py, &payload.4)?,
            payload.5.into_py_any(py)?,
            payload.6.clone().into_py_any(py)?,
        ],
    )?
    .into_py_any(py)
}

fn plugin_strings_payload_to_py(
    py: Python<'_>,
    payload: &PluginStringsPayload,
) -> PyResult<Py<PyAny>> {
    (
        payload.0.clone(),
        payload.1.clone(),
        payload.2.clone(),
        payload.3.clone(),
    )
        .into_py_any(py)
}

fn plugin_legacy_metadata_payload_to_py(
    py: Python<'_>,
    payload: &PluginLegacyMetadataPayload,
) -> PyResult<Py<PyAny>> {
    PyTuple::new(
        py,
        [
            plugin_metadata_payload_to_py(py, &payload.0)?,
            plugin_strings_payload_to_py(py, &payload.1)?,
        ],
    )?
    .into_py_any(py)
}

type PluginStringsPayload = (
    String,
    Vec<(u32, String)>,
    Vec<(String, u32, String)>,
    Vec<(u32, String)>,
);

fn plugin_strings_payload(
    strings_state: &LocalizedStringsState,
    language: Option<&str>,
) -> PluginStringsPayload {
    if let Some(language) = language.and_then(strings::normalize_language) {
        let table = strings_state
            .by_language
            .get(language.as_str())
            .cloned()
            .unwrap_or_default();
        let target_ids: HashSet<u32> = table.keys().copied().collect();
        let table_types: HashMap<u32, String> = strings_state
            .table_types
            .iter()
            .filter_map(|(string_id, table_type)| {
                target_ids
                    .contains(string_id)
                    .then(|| (*string_id, table_type.clone()))
            })
            .collect();
        let localized_strings = table.into_iter().collect::<Vec<_>>();
        let by_language = localized_strings
            .iter()
            .map(|(string_id, text)| (language.clone(), *string_id, text.clone()))
            .collect::<Vec<_>>();
        let table_types = table_types.into_iter().collect::<Vec<_>>();
        (
            strings_state.default_language.clone(),
            localized_strings,
            by_language,
            table_types,
        )
    } else {
        let by_language = strings_state
            .by_language
            .iter()
            .flat_map(|(language, table)| {
                table
                    .iter()
                    .map(move |(string_id, text)| (language.clone(), *string_id, text.clone()))
            })
            .collect::<Vec<_>>();
        let table_types = strings_state
            .table_types
            .clone()
            .into_iter()
            .collect::<Vec<_>>();
        (
            strings_state.default_language.clone(),
            Vec::new(),
            by_language,
            table_types,
        )
    }
}

#[pyfunction(name = "plugin_handle_load")]
#[pyo3(signature = (plugin_path, game=None, strings_dir=None, language=None, eager_compressed=true))]
pub fn plugin_handle_load_native(
    py: Python<'_>,
    plugin_path: &str,
    game: Option<&str>,
    strings_dir: Option<&str>,
    language: Option<&str>,
    eager_compressed: bool,
) -> PyResult<u64> {
    let plugin_path = plugin_path.to_string();
    let game = game.map(str::to_string);
    let strings_dir = strings_dir.map(str::to_string);
    let language = language.map(str::to_string);
    py.detach(move || {
        plugin_handle_load_no_py(
            plugin_path.as_str(),
            game.as_deref(),
            strings_dir.as_deref(),
            language.as_deref(),
            eager_compressed,
        )
        .map_err(PyRuntimeError::new_err)
    })
}

/// Load a plugin into the native handle store without requiring the Python GIL.
pub fn plugin_handle_load_no_py(
    plugin_path: &str,
    game: Option<&str>,
    strings_dir: Option<&str>,
    language: Option<&str>,
    eager_compressed: bool,
) -> Result<u64, String> {
    let path = plugin_path.to_string();
    let game_owned = game.map(str::to_string);
    let strings_dir_owned = strings_dir.map(str::to_string);
    let language_owned = language.map(str::to_string);
    let parsed = parse_plugin_file(path.as_str(), game_owned, eager_compressed)
        .map_err(|e| e.to_string())?;
    let plugin_name = parsed.plugin_name.clone();
    let is_localized = (parsed.header.flags & TES4_FLAG_LOCALIZED) != 0;
    let strings = if is_localized {
        strings::hydrate_strings_state(
            path.as_str(),
            plugin_name.as_str(),
            strings_dir_owned.as_deref(),
            language_owned.as_deref(),
        )
    } else {
        LocalizedStringsState::default()
    };
    Ok(insert_plugin_handle(parsed, strings))
}

/// Lazy/index-only load for read-only handles (target masters). Parses only the
/// TES4 header and keeps the (mmap-backed) file buffer; the `CoreSection`
/// (formid/eid/sig index) is built by streaming those bytes, and records are
/// re-parsed on demand. Avoids the ~7+ GB resident master trees while keeping
/// formid->sig / eid lookups and record reads byte-identical.
#[pyfunction(name = "plugin_handle_load_index")]
#[pyo3(signature = (plugin_path, game=None, strings_dir=None, language=None))]
pub fn plugin_handle_load_index_native(
    py: Python<'_>,
    plugin_path: &str,
    game: Option<&str>,
    strings_dir: Option<&str>,
    language: Option<&str>,
) -> PyResult<u64> {
    let path = plugin_path.to_string();
    let game_owned = game.map(str::to_string);
    let strings_dir_owned = strings_dir.map(str::to_string);
    let language_owned = language.map(str::to_string);
    py.detach(move || {
        plugin_handle_load_index_no_py(
            path.as_str(),
            game_owned.as_deref(),
            strings_dir_owned.as_deref(),
            language_owned.as_deref(),
        )
        .map_err(PyRuntimeError::new_err)
    })
}

/// Lazy/index-only load without requiring the Python GIL.
pub fn plugin_handle_load_index_no_py(
    plugin_path: &str,
    game: Option<&str>,
    strings_dir: Option<&str>,
    language: Option<&str>,
) -> Result<u64, String> {
    let file_path = std::path::Path::new(plugin_path);
    let plugin_name = file_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Plugin.esp")
        .to_string();
    let file_path_str = file_path.to_string_lossy().into_owned();
    let data = read_plugin_source_bytes(file_path).map_err(|error| error.to_string())?;

    // Parse the TES4 header and nothing else. Parsing the whole plugin to build
    // an index and then dropping the tree peaks at 8.30 GB on SeventySix.esm,
    // worse than a full load.
    let header_size = detect_header_size(&data);
    let (header_record, root_start) =
        parse_record(&data, 0, header_size, false).map_err(|error| error.to_string())?;
    if header_record.signature != "TES4" {
        return Err(format!(
            "expected TES4 header record, got {}",
            header_record.signature
        ));
    }
    let header = parse_plugin_header(&header_record);
    let is_localized = (header.flags & TES4_FLAG_LOCALIZED) != 0;
    // Index the tables rather than decoding them: 207 MB of loose tables across
    // 13 languages on SeventySix.esm expand to roughly 1 GB once decoded.
    let strings = if is_localized {
        strings::index_strings_state(plugin_path, &plugin_name, strings_dir, language)
    } else {
        LocalizedStringsState::default()
    };
    let parsed = ParsedPlugin {
        plugin_name,
        file_path: file_path_str,
        header_size,
        header,
        root_items: Vec::new(),
        game: game.map(str::to_string),
    };
    let lazy = LazyRecordStore {
        buffer: data,
        header_size,
        root_start,
        offsets: std::sync::OnceLock::new(),
            probes: std::sync::atomic::AtomicUsize::new(0),
    };
    Ok(insert_plugin_handle_lazy(parsed, strings, lazy))
}

#[pyfunction(name = "plugin_handle_from_bytes")]
#[pyo3(signature = (data, plugin_name, game=None, auto_load_strings=false, strings_dir=None, language=None, file_path=None))]
pub fn plugin_handle_from_bytes_native(
    py: Python<'_>,
    data: &Bound<'_, PyAny>,
    plugin_name: &str,
    game: Option<&str>,
    auto_load_strings: bool,
    strings_dir: Option<&str>,
    language: Option<&str>,
    file_path: Option<&str>,
) -> PyResult<u64> {
    let bytes = Bytes::from(bytes_like_to_vec(py, data)?);
    let plugin_name_owned = plugin_name.to_string();
    let game_owned = game.map(str::to_string);
    let strings_dir_owned = strings_dir.map(str::to_string);
    let language_owned = language.map(str::to_string);
    let file_path_owned = file_path.unwrap_or("").to_string();
    let (parsed, strings) = py.detach(move || {
        let parsed = parse_plugin_bytes(
            bytes,
            plugin_name_owned.clone(),
            file_path_owned.clone(),
            game_owned,
        )?;
        let is_localized = (parsed.header.flags & TES4_FLAG_LOCALIZED) != 0;
        let strings = if auto_load_strings && is_localized {
            let hydrate_path = if file_path_owned.is_empty() {
                plugin_name_owned.as_str()
            } else {
                file_path_owned.as_str()
            };
            strings::hydrate_strings_state(
                hydrate_path,
                plugin_name_owned.as_str(),
                strings_dir_owned.as_deref(),
                language_owned.as_deref(),
            )
        } else {
            LocalizedStringsState::default()
        };
        Ok::<_, PyErr>((parsed, strings))
    })?;
    Ok(insert_plugin_handle(parsed, strings))
}

#[pyfunction(name = "plugin_handle_new")]
#[pyo3(signature = (plugin_name, game=None))]
pub fn plugin_handle_new_native(plugin_name: &str, game: Option<&str>) -> PyResult<u64> {
    Ok(plugin_handle_new_no_py(plugin_name, game))
}

/// Create an empty plugin handle without requiring the Python GIL.
pub fn plugin_handle_new_no_py(plugin_name: &str, game: Option<&str>) -> u64 {
    insert_plugin_handle(
        empty_parsed_plugin(plugin_name, game),
        LocalizedStringsState {
            default_language: "en".to_string(),
            ..LocalizedStringsState::default()
        },
    )
}

#[pyfunction(name = "plugin_handle_close")]
pub fn plugin_handle_close_native(handle_id: u64) -> bool {
    plugin_handle_store()
        .lock()
        .unwrap()
        .remove(&handle_id)
        .is_some()
}

/// Read a plugin handle's canonical master order without requiring the Python GIL.
pub fn plugin_handle_master_names_no_py(handle_id: u64) -> Result<Vec<String>, String> {
    let store = plugin_handle_store()
        .lock()
        .map_err(|error| format!("plugin handle store lock poisoned: {error}"))?;
    let slot = store
        .get(&handle_id)
        .ok_or_else(|| format!("unknown plugin handle: {handle_id}"))?;
    Ok(slot.parsed.header.masters.clone())
}

/// Read the game recorded on a plugin handle without requiring the Python GIL.
pub fn plugin_handle_game_no_py(handle_id: u64) -> Result<Option<String>, String> {
    let store = plugin_handle_store()
        .lock()
        .map_err(|error| format!("plugin handle store lock poisoned: {error}"))?;
    let slot = store
        .get(&handle_id)
        .ok_or_else(|| format!("unknown plugin handle: {handle_id}"))?;
    Ok(slot.parsed.game.clone())
}

/// Record count for a handle, whether or not it owns a tree.
///
/// A lazy handle's `root_items` is empty, so counting it returned 0 and
/// `modkit esp count` reported an empty plugin.
fn slot_record_count(slot: &mut NativePluginSlot) -> usize {
    if let Some(value) = slot.record_count_cache {
        return value;
    }
    let value = match slot.lazy.as_ref() {
        // Reuse the offsets map when something already built it; otherwise walk
        // headers only. Either way this is the one metadata field that costs a
        // pass over the file, so `plugin_handle_get_meta` must not ask for it
        // just to answer `plugin_name`.
        Some(lazy) => match lazy.offsets.get() {
            Some(offsets) => offsets.len(),
            None => lazy.cursor().count_records(),
        },
        None => count_records(&slot.parsed.root_items),
    };
    slot.record_count_cache = Some(value);
    value
}

/// Identity fields that cost nothing to read, for callers that only need to
/// name a handle. `plugin_handle_get_meta` computes the record count, which on
/// a lazy handle is a pass over the whole file - 2.25 s on SeventySix.esm just
/// to learn the plugin is called "SeventySix.esm".
#[pyfunction(name = "plugin_handle_identity")]
pub fn plugin_handle_identity_native(
    py: Python<'_>,
    handle_id: u64,
) -> PyResult<(String, String, Option<String>, String)> {
    py.detach(move || {
        let store = plugin_handle_store().lock().unwrap();
        let slot = store
            .get(&handle_id)
            .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
        Ok((
            slot.parsed.plugin_name.clone(),
            slot.parsed.file_path.clone(),
            slot.parsed.game.clone(),
            slot.strings.default_language.clone(),
        ))
    })
}

#[pyfunction(name = "plugin_handle_get_meta")]
pub fn plugin_handle_get_meta_native(py: Python<'_>, handle_id: u64) -> PyResult<Py<PyAny>> {
    let snapshot = py.detach(move || {
        let mut store = plugin_handle_store().lock().unwrap();
        let slot = store
            .get_mut(&handle_id)
            .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
        let record_count = slot_record_count(slot);
        Ok::<_, PyErr>(PluginMetadataSnapshot {
            plugin_name: slot.parsed.plugin_name.clone(),
            file_path: slot.parsed.file_path.clone(),
            game: slot.parsed.game.clone(),
            header_size: slot.parsed.header_size,
            header: slot.parsed.header.clone(),
            record_count,
            localized_default_language: slot.strings.default_language.clone(),
        })
    })?;
    plugin_metadata_payload_to_py(py, &plugin_metadata_payload(&snapshot))
}

#[pyfunction(name = "plugin_handle_metadata")]
pub fn plugin_handle_metadata_native(py: Python<'_>, handle_id: u64) -> PyResult<Py<PyAny>> {
    let (snapshot, strings_state) = py.detach(move || {
        let mut store = plugin_handle_store().lock().unwrap();
        let slot = store
            .get_mut(&handle_id)
            .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
        let record_count = slot_record_count(slot);
        slot.strings.materialize_all();
        Ok::<_, PyErr>((
            PluginMetadataSnapshot {
                plugin_name: slot.parsed.plugin_name.clone(),
                file_path: slot.parsed.file_path.clone(),
                game: slot.parsed.game.clone(),
                header_size: slot.parsed.header_size,
                header: slot.parsed.header.clone(),
                record_count,
                localized_default_language: slot.strings.default_language.clone(),
            },
            slot.strings.clone(),
        ))
    })?;
    plugin_legacy_metadata_payload_to_py(
        py,
        &plugin_legacy_metadata_payload(&snapshot, &strings_state),
    )
}

#[pyfunction(name = "plugin_handle_get_strings")]
#[pyo3(signature = (handle_id, language=None))]
pub fn plugin_handle_get_strings_native(
    py: Python<'_>,
    handle_id: u64,
    language: Option<&str>,
) -> PyResult<PluginStringsPayload> {
    let language_owned = language.map(str::to_string);
    let strings_state = py.detach(move || {
        let mut store = plugin_handle_store().lock().unwrap();
        let slot = store
            .get_mut(&handle_id)
            .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
        slot.strings.materialize_all();
        Ok::<_, PyErr>(slot.strings.clone())
    })?;
    Ok(plugin_strings_payload(
        &strings_state,
        language_owned.as_deref(),
    ))
}

#[derive(Default)]
struct SchemaForgeCorpusSummary {
    total_records: usize,
    total_subrecords: usize,
    observations: HashMap<(String, String, usize), SchemaForgeObservation>,
}

struct SchemaForgeObservation {
    record_sig: String,
    subrecord_sig: String,
    occurrence_index: usize,
    count: usize,
    length_histogram: HashMap<usize, usize>,
    byte_samples: Vec<Vec<u8>>,
    rng_state: u64,
}

type SchemaForgeObservationPayload = (
    String,
    String,
    usize,
    usize,
    Vec<(usize, usize)>,
    Vec<Vec<u8>>,
);
type SchemaForgeCorpusPayload = (String, usize, usize, Vec<SchemaForgeObservationPayload>);

impl SchemaForgeObservation {
    fn new(record_sig: &str, subrecord_sig: &str, occurrence_index: usize) -> Self {
        Self {
            record_sig: record_sig.to_string(),
            subrecord_sig: subrecord_sig.to_string(),
            occurrence_index,
            count: 0,
            length_histogram: HashMap::new(),
            byte_samples: Vec::new(),
            rng_state: schema_forge_sample_seed(record_sig, subrecord_sig, occurrence_index),
        }
    }

    fn add(&mut self, payload: &[u8], sample_cap: usize) {
        self.count += 1;
        *self.length_histogram.entry(payload.len()).or_insert(0) += 1;
        if sample_cap == 0 {
            return;
        }
        let sample = prepare_schema_forge_sample(payload);
        if self.byte_samples.len() < sample_cap {
            self.byte_samples.push(sample);
            return;
        }
        self.rng_state = self
            .rng_state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let replacement_index = (self.rng_state % self.count as u64) as usize;
        if replacement_index < sample_cap {
            self.byte_samples[replacement_index] = sample;
        }
    }
}

fn schema_forge_sample_seed(record_sig: &str, subrecord_sig: &str, occurrence_index: usize) -> u64 {
    let mut state = 0xcbf29ce484222325u64;
    for value in record_sig
        .as_bytes()
        .iter()
        .chain(subrecord_sig.as_bytes())
        .chain(occurrence_index.to_le_bytes().iter())
    {
        state ^= u64::from(*value);
        state = state.wrapping_mul(0x100000001b3);
    }
    state
}

fn prepare_schema_forge_sample(payload: &[u8]) -> Vec<u8> {
    if payload.len() <= SCHEMA_FORGE_LARGE_SAMPLE_THRESHOLD {
        return payload.to_vec();
    }
    let mut out = Vec::with_capacity(SCHEMA_FORGE_LARGE_SAMPLE_TRUNCATE + 4 + 32);
    out.extend_from_slice(&payload[..SCHEMA_FORGE_LARGE_SAMPLE_TRUNCATE]);
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    let digest = Sha256::digest(payload);
    out.extend_from_slice(digest.as_slice());
    out
}

fn schema_forge_collect_record(
    record: &ParsedRecord,
    summary: &mut SchemaForgeCorpusSummary,
    sample_cap: usize,
) -> PyResult<()> {
    summary.total_records += 1;
    let lazy_subrecords = lazy_subrecords_for_record(record)?;
    let subrecords = lazy_subrecords
        .as_deref()
        .unwrap_or(record.subrecords.as_slice());
    let mut occurrence_counters: HashMap<&str, usize> = HashMap::new();
    for subrecord in subrecords {
        let sub_sig = subrecord.signature.as_str();
        let occurrence_index = *occurrence_counters.get(sub_sig).unwrap_or(&0);
        occurrence_counters.insert(sub_sig, occurrence_index + 1);
        let record_sig = record.signature.as_str();
        let key = (
            record_sig.to_string(),
            sub_sig.to_string(),
            occurrence_index,
        );
        summary
            .observations
            .entry(key)
            .or_insert_with(|| SchemaForgeObservation::new(record_sig, sub_sig, occurrence_index))
            .add(&subrecord.data, sample_cap);
        summary.total_subrecords += 1;
    }
    Ok(())
}

fn schema_forge_collect_items(
    items: &[ParsedItem],
    summary: &mut SchemaForgeCorpusSummary,
    sample_cap: usize,
) -> PyResult<()> {
    for item in items {
        match item {
            ParsedItem::Record(record) => {
                schema_forge_collect_record(record, summary, sample_cap)?;
            }
            ParsedItem::Group(group) => {
                schema_forge_collect_items(&group.children, summary, sample_cap)?;
            }
        }
    }
    Ok(())
}

fn schema_forge_corpus_payload(
    plugin_name: &str,
    summary: SchemaForgeCorpusSummary,
) -> SchemaForgeCorpusPayload {
    let mut values: Vec<_> = summary.observations.into_values().collect();
    values.sort_by(|left, right| {
        (
            left.record_sig.as_str(),
            left.subrecord_sig.as_str(),
            left.occurrence_index,
        )
            .cmp(&(
                right.record_sig.as_str(),
                right.subrecord_sig.as_str(),
                right.occurrence_index,
            ))
    });
    let observations = values
        .into_iter()
        .map(|observation| {
            (
                observation.record_sig,
                observation.subrecord_sig,
                observation.occurrence_index,
                observation.count,
                observation.length_histogram.into_iter().collect::<Vec<_>>(),
                observation.byte_samples,
            )
        })
        .collect::<Vec<_>>();
    (
        plugin_name.to_string(),
        summary.total_records,
        summary.total_subrecords,
        observations,
    )
}

#[pyfunction(name = "schema_forge_collect_corpus_native")]
#[pyo3(signature = (plugin_path, game, sample_cap))]
pub fn schema_forge_collect_corpus_native(
    py: Python<'_>,
    plugin_path: &str,
    game: &str,
    sample_cap: usize,
) -> PyResult<SchemaForgeCorpusPayload> {
    let path = plugin_path.to_string();
    let game_owned = Some(game.to_string());
    let (plugin_name, summary) = py.detach(move || {
        let parsed = parse_plugin_file_lazy_compressed(path.as_str(), game_owned)?;
        let plugin_name = parsed.plugin_name.clone();
        let mut summary = SchemaForgeCorpusSummary::default();
        schema_forge_collect_items(&parsed.root_items, &mut summary, sample_cap)?;
        Ok::<_, PyErr>((plugin_name, summary))
    })?;
    Ok(schema_forge_corpus_payload(&plugin_name, summary))
}

#[pyfunction(name = "plugin_handle_group_signatures")]
pub fn plugin_handle_group_signatures_native(
    py: Python<'_>,
    handle_id: u64,
) -> PyResult<Py<PyAny>> {
    let groups = py.detach(move || {
        let store = plugin_handle_store().lock().unwrap();
        let slot = store
            .get(&handle_id)
            .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
        let mut groups: Vec<(String, usize)> = Vec::new();
        if let Some(lazy) = slot.lazy.as_ref() {
            for (label, count) in lazy.cursor().top_level_groups() {
                groups.push((String::from_utf8_lossy(&label).into_owned(), count));
            }
        } else {
            for item in &slot.parsed.root_items {
                if let ParsedItem::Group(group) = item {
                    groups.push((
                        String::from_utf8_lossy(&group.label).into_owned(),
                        group.children.len(),
                    ));
                }
            }
        }
        Ok::<_, PyErr>(groups)
    })?;
    let result = PyList::empty(py);
    for pair in groups {
        result.append(pair.into_pyobject(py)?)?;
    }
    Ok(result.into_any().unbind())
}

#[pyfunction(name = "plugin_handle_record_counts")]
pub fn plugin_handle_record_counts_native(
    py: Python<'_>, handle_id: u64,
) -> PyResult<BTreeMap<String, usize>> {
    fn count_items(items: &[ParsedItem], counts: &mut BTreeMap<String, usize>) {
        for item in items {
            match item {
                ParsedItem::Group(group) => count_items(&group.children, counts),
                ParsedItem::Record(record) => {
                    *counts.entry(record.signature.to_string()).or_default() += 1;
                }
            }
        }
    }
    py.detach(move || {
        let store = plugin_handle_store().lock().unwrap();
        let slot = store.get(&handle_id)
            .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
        let mut counts = BTreeMap::new();
        if let Some(lazy) = slot.lazy.as_ref() {
            lazy.cursor().scan(&mut |view| {
                *counts.entry(String::from_utf8_lossy(view.signature).into_owned()).or_default() += 1;
                std::ops::ControlFlow::Continue(())
            });
        } else {
            count_items(&slot.parsed.root_items, &mut counts);
        }
        Ok(counts)
    })
}

#[pyfunction(name = "plugin_handle_group_record_summaries")]
pub fn plugin_handle_group_record_summaries_native(
    py: Python<'_>,
    handle_id: u64,
    group_signature: &str,
) -> PyResult<Vec<(u32, String, Option<String>)>> {
    let sig_owned = group_signature.to_string();
    py.detach(move || {
        let store = plugin_handle_store().lock().unwrap();
        let slot = store
            .get(&handle_id)
            .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
        let mut out = Vec::new();
        for item in &slot.parsed.root_items {
            if let ParsedItem::Group(group) = item {
                if String::from_utf8_lossy(&group.label) == sig_owned.as_str() {
                    collect_record_summaries(&group.children, None, &mut out);
                    break;
                }
            }
        }
        Ok(out)
    })
}

#[pyfunction(name = "plugin_handle_to_bytes")]
pub fn plugin_handle_to_bytes_native(py: Python<'_>, handle_id: u64) -> PyResult<Py<PyAny>> {
    let bytes = py.detach(move || {
        let (mut parsed, _) = clone_plugin_handle_state(handle_id)?;
        build_plugin_bytes(&mut parsed)
    })?;
    Ok(PyBytes::new(py, &bytes).into_any().unbind())
}

#[pyfunction(name = "plugin_handle_save")]
pub fn plugin_handle_save_native(
    py: Python<'_>,
    handle_id: u64,
    output_path: &str,
) -> PyResult<()> {
    let path = output_path.to_string();
    py.detach(move || {
        plugin_handle_save_no_py(handle_id, path.as_str()).map_err(PyRuntimeError::new_err)
    })
}

#[pyfunction(name = "plugin_handle_max_object_id")]
pub fn plugin_handle_max_object_id_native(py: Python<'_>, handle_id: u64) -> PyResult<u32> {
    py.detach(move || plugin_handle_max_object_id_no_py(handle_id).map_err(PyRuntimeError::new_err))
}

#[pyfunction(name = "plugin_handle_allocate_form_id")]
pub fn plugin_handle_allocate_form_id_native(handle_id: u64) -> PyResult<u32> {
    let mut store = plugin_handle_store().lock().unwrap();
    let slot = store
        .get_mut(&handle_id)
        .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
    let object_id = slot.parsed.header.next_object_id & 0x00FF_FFFF;
    slot.parsed.header.next_object_id = (object_id + 1) & 0x00FF_FFFF;
    Ok((LOCAL_FORM_INDEX as u32) << 24 | object_id)
}

#[pyfunction(name = "plugin_handle_add_master")]
#[pyo3(signature = (handle_id, master_name, size=None))]
pub fn plugin_handle_add_master_native(
    handle_id: u64,
    master_name: &str,
    size: Option<u64>,
) -> PyResult<()> {
    plugin_handle_add_master_no_py(handle_id, master_name, size).map_err(PyRuntimeError::new_err)
}

/// Append a master to a plugin handle without requiring the Python GIL.
pub fn plugin_handle_add_master_no_py(
    handle_id: u64,
    master_name: &str,
    size: Option<u64>,
) -> Result<(), String> {
    let mut store = plugin_handle_store()
        .lock()
        .map_err(|error| format!("plugin handle store lock poisoned: {error}"))?;
    let slot = store
        .get_mut(&handle_id)
        .ok_or_else(|| format!("unknown plugin handle: {handle_id}"))?;
    if !slot
        .parsed
        .header
        .masters
        .iter()
        .any(|m| m.eq_ignore_ascii_case(master_name))
    {
        let old_masters = slot.parsed.header.masters.clone();
        let mut new_masters = old_masters.clone();
        new_masters.push(master_name.to_string());
        remap_slot_formids_for_masters(slot, &old_masters, &new_masters)?;
        slot.parsed.header.masters = new_masters;
        slot.parsed.header.master_sizes.push(size.unwrap_or(0));
        slot.sections.apply_effect(&WriteEffect::MastersChanged);
    }
    Ok(())
}

#[pyfunction(name = "plugin_handle_ensure_source_masters")]
#[pyo3(signature = (handle_id, masters, include_plugin=None))]
pub fn plugin_handle_ensure_source_masters_native(
    handle_id: u64,
    masters: Vec<String>,
    include_plugin: Option<String>,
) -> PyResult<()> {
    let mut store = plugin_handle_store().lock().unwrap();
    let slot = store
        .get_mut(&handle_id)
        .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
    let mut required = masters;
    if let Some(name) = include_plugin {
        required.push(name);
    }
    let overlap = required.len().min(slot.parsed.header.masters.len());
    for (index, expected) in required.iter().take(overlap).enumerate() {
        if !expected.eq_ignore_ascii_case(&slot.parsed.header.masters[index]) {
            return Err(PyValueError::new_err(format!(
                "incompatible master prefix at index {index}: expected {:?} got {:?}",
                expected, slot.parsed.header.masters[index]
            )));
        }
    }
    let old_masters = slot.parsed.header.masters.clone();
    if required.len() > old_masters.len() {
        remap_slot_formids_for_masters(slot, &old_masters, &required).map_err(PyValueError::new_err)?;
        slot.parsed.header.master_sizes.resize(required.len(), 0);
        slot.parsed.header.masters = required;
        slot.sections.apply_effect(&WriteEffect::MastersChanged);
    }
    Ok(())
}

fn remap_slot_formids_for_masters(
    slot: &mut NativePluginSlot,
    old_masters: &[String],
    new_masters: &[String],
) -> Result<(), String> {
    if old_masters == new_masters {
        return Ok(());
    }
    // The TES4 header keeps a verbatim copy of its on-disk subrecords
    // (`raw_subrecords`) that the serializer writes in preference to the parsed
    // master list. Once masters change that cache is stale, so drop it and let
    // the save regenerate MAST/DATA from `header.masters` (every header field is
    // losslessly captured at load, so regeneration is equivalent).
    master_edit::remap(slot, old_masters, new_masters)
}

#[pyfunction(name = "plugin_handle_set_masters")]
#[pyo3(signature = (handle_id, masters))]
pub fn plugin_handle_set_masters_native(
    handle_id: u64,
    masters: Vec<(String, u64)>,
) -> PyResult<()> {
    let mut store = plugin_handle_store().lock().unwrap();
    let slot = store
        .get_mut(&handle_id)
        .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
    let old_masters = slot.parsed.header.masters.clone();
    let new_masters: Vec<String> = masters.iter().map(|(name, _)| name.clone()).collect();
    let new_sizes: Vec<u64> = masters.iter().map(|(_, size)| *size).collect();
    remap_slot_formids_for_masters(slot, &old_masters, &new_masters).map_err(PyValueError::new_err)?;
    slot.parsed.header.masters = new_masters;
    slot.parsed.header.master_sizes = new_sizes;
    slot.sections.apply_effect(&WriteEffect::MastersChanged);
    Ok(())
}

#[pyfunction(name = "plugin_handle_set_header_field")]
pub fn plugin_handle_set_header_field_native(
    handle_id: u64,
    field: &str,
    value: &Bound<'_, PyAny>,
) -> PyResult<()> {
    let mut store = plugin_handle_store().lock().unwrap();
    let slot = store
        .get_mut(&handle_id)
        .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
    match field {
        "author" => slot.parsed.header.author = value.extract::<String>()?,
        "description" => slot.parsed.header.description = value.extract::<String>()?,
        "flags" => slot.parsed.header.flags = value.extract::<u32>()?,
        "next_object_id" => slot.parsed.header.next_object_id = value.extract::<u32>()?,
        "version" => slot.parsed.header.version = value.extract::<f64>()? as f32,
        "header_size" => slot.parsed.header_size = value.extract::<usize>()?,
        "is_localized" => {
            if value.extract::<bool>()? {
                slot.parsed.header.flags |= TES4_FLAG_LOCALIZED;
            } else {
                slot.parsed.header.flags &= !TES4_FLAG_LOCALIZED;
            }
        }
        _ => {
            return Err(PyKeyError::new_err(format!(
                "unsupported header field: {field}"
            )));
        }
    }
    // The TES4 header keeps a verbatim copy of its on-disk subrecords
    // (`raw_subrecords`) that the serializer writes in preference to the parsed
    // header fields. The edit above just changed one of those fields, so the
    // cache is stale — drop it and let the save regenerate HEDR/CNAM/SNAM/MAST
    // from the parsed header (every field is losslessly captured at load).
    slot.parsed.header.raw_subrecords.clear();
    Ok(())
}

#[pyfunction(name = "plugin_handle_set_logical_identity")]
#[pyo3(signature = (handle_id, plugin_name, game=None, file_path=None))]
pub fn plugin_handle_set_logical_identity_native(
    handle_id: u64,
    plugin_name: &str,
    game: Option<&str>,
    file_path: Option<&str>,
) -> PyResult<()> {
    let mut store = plugin_handle_store().lock().unwrap();
    let slot = store
        .get_mut(&handle_id)
        .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
    let old_plugin_name = slot.parsed.plugin_name.clone();
    slot.parsed.plugin_name = plugin_name.to_string();
    slot.parsed.game = game.map(str::to_string);
    slot.parsed.file_path = file_path.unwrap_or("").to_string();
    slot.sections
        .apply_effect(if slot.parsed.plugin_name != old_plugin_name {
            &WriteEffect::MastersChanged
        } else {
            &WriteEffect::HeaderOnly
        });
    Ok(())
}

#[pyfunction(name = "plugin_handle_remove_record")]
pub fn plugin_handle_remove_record_native(handle_id: u64, form_id: u32) -> PyResult<bool> {
    let mut store = plugin_handle_store().lock().unwrap();
    let slot = store
        .get_mut(&handle_id)
        .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
    let removed = remove_record_from_items(&mut slot.parsed.root_items, form_id);
    if removed {
        slot.record_count_cache = None;
        slot.sections
            .apply_effect(&WriteEffect::RecordsAddedOrRemoved);
    }
    Ok(removed)
}

#[pyfunction(name = "plugin_handle_remove_records")]
pub fn plugin_handle_remove_records_native(handle_id: u64, form_ids: Vec<u32>) -> PyResult<usize> {
    let mut store = plugin_handle_store().lock().unwrap();
    let slot = store
        .get_mut(&handle_id)
        .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
    let targets: std::collections::HashSet<u32> =
        form_ids.into_iter().map(|f| f & 0xFFFF_FFFF).collect();
    let mut removed = 0usize;
    remove_records_from_items(&mut slot.parsed.root_items, &targets, &mut removed);
    if removed > 0 {
        slot.record_count_cache = None;
        slot.sections
            .apply_effect(&WriteEffect::RecordsAddedOrRemoved);
    }
    Ok(removed)
}

#[pyfunction(name = "plugin_handle_delete_records")]
#[pyo3(signature = (handle_id, form_ids, cascade=false))]
pub fn plugin_handle_delete_records_native(
    py: Python<'_>,
    handle_id: u64,
    form_ids: Vec<u32>,
    cascade: bool,
) -> PyResult<(usize, usize, usize)> {
    py.detach(move || delete_records_native(handle_id, &form_ids, cascade))
}

fn delete_records_native(
    handle_id: u64,
    form_ids: &[u32],
    cascade: bool,
) -> PyResult<(usize, usize, usize)> {
    let target_form_ids: HashSet<u32> = form_ids.iter().map(|value| *value & 0xFFFF_FFFF).collect();
    if target_form_ids.is_empty() {
        return Ok((0, 0, 0));
    }

    let (updates, refs_removed) = if cascade {
        use rayon::prelude::*;

        let store = plugin_handle_store().lock().unwrap();
        let slot = store
            .get(&handle_id)
            .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
        let target_object_ids: HashSet<u32> = target_form_ids
            .iter()
            .map(|value| value & 0x00FF_FFFF)
            .collect();
        let own_plugin = slot.parsed.plugin_name.clone();
        let mut records = Vec::new();
        collect_records(&slot.parsed.root_items, &mut |_| true, &mut records);
        let updates: Vec<(JsonValue, usize)> = records
            .par_iter()
            .filter_map(|record| {
                if target_form_ids.contains(&(record.form_id & 0xFFFF_FFFF)) {
                    return None;
                }
                let mut value =
                    serialize_record_payload_to_json(record, &slot.parsed, &slot.strings);
                let fields = value.get_mut("fields")?.as_array_mut()?;
                let removed = strip_target_references(fields, &own_plugin, &target_object_ids);
                if removed == 0 {
                    return None;
                }
                value.as_object_mut()?.insert(
                    "signature".to_string(),
                    JsonValue::String(record.signature.to_string()),
                );
                Some((value, removed))
            })
            .collect();
        let refs_removed = updates.iter().map(|(_, removed)| removed).sum();
        (updates, refs_removed)
    } else {
        (Vec::new(), 0)
    };

    let records_modified = updates.len();
    for (value, _) in updates {
        plugin_handle_replace_authoring_record_value(handle_id, &value)?;
    }

    let mut store = plugin_handle_store().lock().unwrap();
    let slot = store
        .get_mut(&handle_id)
        .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
    let mut removed = 0;
    remove_records_from_items(&mut slot.parsed.root_items, &target_form_ids, &mut removed);
    if removed > 0 {
        slot.record_count_cache = None;
        slot.sections
            .apply_effect(&WriteEffect::RecordsAddedOrRemoved);
    }
    Ok((removed, records_modified, refs_removed))
}

#[pyfunction(name = "plugin_handle_add_record_raw")]
pub fn plugin_handle_add_record_raw_native(
    py: Python<'_>,
    handle_id: u64,
    signature: String,
    form_id: u32,
    flags: u32,
    version_control: u32,
    form_version: Option<u16>,
    version2: Option<u16>,
    subrecords: Vec<(String, Vec<u8>, Option<String>)>,
) -> PyResult<u32> {
    if signature.len() != 4 {
        return Err(value_error(format!(
            "record signature must be 4 chars: {signature:?}"
        )));
    }
    for (sub_sig, _, _) in &subrecords {
        if sub_sig.len() != 4 {
            return Err(value_error(format!(
                "subrecord signature must be 4 chars: {sub_sig:?}"
            )));
        }
    }
    py.detach(move || {
        let record = ParsedRecord {
            signature: SmolStr::new(signature.as_str()),
            form_id,
            flags,
            version_control,
            form_version,
            version2,
            subrecords: subrecords
                .into_iter()
                .map(|(sub_sig, data, semantic_type)| ParsedSubrecord {
                    signature: SmolStr::new(sub_sig.as_str()),
                    data: Bytes::from(data),
                    semantic_type,
                })
                .collect(),
            raw_payload: None,
            parse_error: None,
        };
        let inserted_form_id = record.form_id;
        insert_parsed_record(handle_id, record).map_err(PyRuntimeError::new_err)?;
        Ok(inserted_form_id)
    })
}

#[pyfunction(name = "plugin_handle_replace_authoring_record")]
pub fn plugin_handle_replace_authoring_record_native(
    handle_id: u64,
    json_text: String,
) -> PyResult<String> {
    let value: JsonValue = serde_json::from_str(&json_text)
        .map_err(|e| value_error(format!("invalid record JSON: {e}")))?;
    plugin_handle_replace_authoring_record_value(handle_id, &value)
}

#[pyfunction(name = "plugin_handle_replace_projected_cell_authoring_record_values_at_locations")]
pub fn plugin_handle_replace_projected_cell_authoring_record_values_at_locations_native(
    py: Python<'_>,
    handle_id: u64,
    values_json: String,
) -> PyResult<usize> {
    py.detach(move || {
        let values: Vec<(JsonValue, String)> = serde_json::from_str(&values_json)
            .map_err(|e| value_error(format!("invalid projected CELL batch JSON: {e}")))?;
        plugin_handle_replace_projected_cell_authoring_record_values_at_locations(handle_id, values)
    })
}

#[pyfunction(name = "plugin_handle_read_authoring_record")]
pub fn plugin_handle_read_authoring_record_native(
    handle_id: u64,
    form_id: u32,
) -> PyResult<Option<String>> {
    let mut store = plugin_handle_store().lock().unwrap();
    let slot = store
        .get_mut(&handle_id)
        .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
    let raw_form_id = form_id & 0xFFFF_FFFF;
    let own_index = (slot.parsed.header.masters.len() & 0xFF) as u8;
    let resolved = {
        let records = ensure_records_section(slot);
        if records.record(&slot.parsed, raw_form_id).is_some() {
            Some(raw_form_id)
        } else {
            let object_id = raw_form_id & 0x00FF_FFFF;
            let core = ensure_core_section(slot);
            core.form_ids_by_object_id
                .get(&object_id)
                .and_then(|ids| pick_owned_form_id(ids, own_index))
        }
    };
    let Some(resolved) = resolved else {
        return Ok(None);
    };
    let value = if slot.lazy.is_some() {
        let Some(record) = lazy_materialize_record(slot, resolved) else {
            return Ok(None);
        };
        serialize_record_payload_to_json(&record, &slot.parsed, &slot.strings)
    } else {
        let records = ensure_records_section(slot);
        let Some(record) = records.record(&slot.parsed, resolved) else {
            return Ok(None);
        };
        serialize_record_payload_to_json(record, &slot.parsed, &slot.strings)
    };
    serde_json::to_string(&value)
        .map(Some)
        .map_err(|e| value_error(format!("serialize record: {e}")))
}

#[pyfunction(name = "plugin_handle_inspect_record")]
pub fn plugin_handle_inspect_record_native(
    py: Python<'_>,
    handle_id: u64,
    raw_form_id: u32,
) -> PyResult<Option<String>> {
    py.detach(move || {
        let mut store = plugin_handle_store().lock().unwrap();
        let slot = store
            .get_mut(&handle_id)
            .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
        rehydrate_filtered_strings_for_authoring(slot);
        let Some(record) = resolve_record_indexed(slot, raw_form_id).map(|record| record.into_owned()) else {
            return Ok(None);
        };
        if record.form_id != raw_form_id {
            return Ok(None);
        }
        let assets = asset_index::extract_asset_paths(&record)
            .into_iter()
            .map(|asset| serde_json::json!({
                "kind": asset.kind.as_str(),
                "path": asset.path,
                "field": asset.source_subrecord_sig.as_str(),
            }))
            .collect::<Vec<_>>();
        let value = serde_json::json!({
            "signature": record.signature.as_str(),
            "raw_form_id": record.form_id,
            "base_form_id": effective_subrecords_for_record(&record).iter()
                .find(|subrecord| subrecord.signature == "NAME" && subrecord.data.len() == 4)
                .map(|subrecord| u32::from_le_bytes(subrecord.data.as_ref().try_into().unwrap())),
            "record": serialize_record_payload_to_json(&record, &slot.parsed, &slot.strings),
            "assets": assets,
        });
        serde_json::to_string(&value)
            .map(Some)
            .map_err(|error| value_error(format!("serialize inspection: {error}")))
    })
}

#[pyfunction(name = "plugin_handle_apply_placed_record_position_offset")]
pub fn plugin_handle_apply_placed_record_position_offset_native(
    handle_id: u64,
    x: f64,
    y: f64,
    z: f64,
) -> PyResult<usize> {
    if x == 0.0 && y == 0.0 && z == 0.0 {
        return Ok(0);
    }

    let mut store = plugin_handle_store().lock().unwrap();
    let slot = store
        .get_mut(&handle_id)
        .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
    let changed = apply_placed_record_position_offset_in_items(
        &mut slot.parsed.root_items,
        (x as f32, y as f32, z as f32),
    );
    if changed > 0 {
        slot.sections.apply_effect(&WriteEffect::RecordContents {
            form_ids: smallvec::SmallVec::new(),
        });
    }
    Ok(changed)
}

#[pyfunction(name = "plugin_handle_sanitize_subrecord_payloads")]
pub fn plugin_handle_sanitize_subrecord_payloads_native(
    py: Python<'_>,
    handle_id: u64,
    max_lengths: Vec<(String, String, usize)>,
    row_projections: Vec<(String, String, usize, usize)>,
) -> PyResult<usize> {
    py.detach(move || {
        let max_lengths: HashMap<(SmolStr, SmolStr), usize> = max_lengths
            .into_iter()
            .map(|(record_sig, subrecord_sig, max_len)| {
                (
                    (SmolStr::new(record_sig), SmolStr::new(subrecord_sig)),
                    max_len,
                )
            })
            .collect();
        let row_projections: HashMap<(SmolStr, SmolStr), (usize, usize)> = row_projections
            .into_iter()
            .filter(|(_, _, source_len, target_len)| *source_len > 0 && *target_len <= *source_len)
            .map(|(record_sig, subrecord_sig, source_len, target_len)| {
                (
                    (SmolStr::new(record_sig), SmolStr::new(subrecord_sig)),
                    (source_len, target_len),
                )
            })
            .collect();
        let mut store = plugin_handle_store().lock().unwrap();
        let slot = store
            .get_mut(&handle_id)
            .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
        let mut changed_form_ids = smallvec::SmallVec::<[u32; 4]>::new();
        let changed = sanitize_subrecord_payloads_in_items(
            &mut slot.parsed.root_items,
            &max_lengths,
            &row_projections,
            &mut changed_form_ids,
        );
        if changed > 0 {
            slot.sections.apply_effect(&WriteEffect::RecordContents {
                form_ids: changed_form_ids,
            });
        }
        Ok::<_, PyErr>(changed)
    })
}

#[pyfunction(name = "plugin_handle_record_summary")]
pub fn plugin_handle_record_summary_native(
    py: Python<'_>,
    handle_id: u64,
    form_id: u32,
) -> PyResult<Option<(u32, String, Option<String>)>> {
    py.detach(move || {
        let mut store = plugin_handle_store().lock().unwrap();
        let slot = store
            .get_mut(&handle_id)
            .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
        let raw_form_id = form_id & 0xFFFF_FFFF;
        let own_index = (slot.parsed.header.masters.len() & 0xFF) as u8;
        let records = ensure_records_section(slot);
        if let Some(record) = records.record(&slot.parsed, raw_form_id) {
            return Ok(Some(record_summary_tuple(record)));
        }
        let object_id = raw_form_id & 0x00FF_FFFF;
        let core = ensure_core_section(slot);
        if let Some(form_ids) = core.form_ids_by_object_id.get(&object_id) {
            if let Some(picked) = pick_owned_form_id(form_ids, own_index) {
                return Ok(records
                    .record(&slot.parsed, picked)
                    .map(record_summary_tuple));
            }
        }
        Ok(None)
    })
}

#[pyfunction(name = "plugin_handle_has_record")]
pub fn plugin_handle_has_record_native(
    py: Python<'_>,
    handle_id: u64,
    form_id: u32,
) -> PyResult<bool> {
    py.detach(move || {
        let mut store = plugin_handle_store().lock().unwrap();
        let slot = store
            .get_mut(&handle_id)
            .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
        let raw_form_id = form_id & 0xFFFF_FFFF;
        let own_index = (slot.parsed.header.masters.len() & 0xFF) as u8;
        let records = ensure_records_section(slot);
        if records.record(&slot.parsed, raw_form_id).is_some() {
            return Ok(true);
        }
        let object_id = raw_form_id & 0x00FF_FFFF;
        let core = ensure_core_section(slot);
        Ok(core
            .form_ids_by_object_id
            .get(&object_id)
            .and_then(|form_ids| pick_owned_form_id(form_ids, own_index))
            .and_then(|picked| records.record(&slot.parsed, picked))
            .is_some())
    })
}

#[pyfunction(name = "plugin_handle_record_payload_hash")]
pub fn plugin_handle_record_payload_hash_native(
    py: Python<'_>,
    handle_id: u64,
    form_id: u32,
) -> PyResult<Option<String>> {
    py.detach(move || {
        let mut store = plugin_handle_store().lock().unwrap();
        let slot = store
            .get_mut(&handle_id)
            .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
        let raw_form_id = form_id & 0xFFFF_FFFF;
        let own_index = (slot.parsed.header.masters.len() & 0xFF) as u8;
        let header_size = slot.parsed.header_size;
        let records = ensure_records_section(slot);
        let record = if let Some(record) = records.record(&slot.parsed, raw_form_id) {
            Some(record)
        } else {
            let object_id = raw_form_id & 0x00FF_FFFF;
            let core = ensure_core_section(slot);
            core.form_ids_by_object_id
                .get(&object_id)
                .and_then(|form_ids| pick_owned_form_id(form_ids, own_index))
                .and_then(|picked| records.record(&slot.parsed, picked))
        };
        let Some(record) = record else {
            return Ok(None);
        };
        let bytes = record_bytes_from_parsed(record, header_size)?;
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        Ok(Some(format!("{:x}", hasher.finalize())))
    })
}

#[pyfunction(name = "plugin_handle_debug_section_loaded")]
pub fn plugin_handle_debug_section_loaded_native(handle_id: u64, section: &str) -> PyResult<bool> {
    let store = plugin_handle_store().lock().unwrap();
    let slot = store
        .get(&handle_id)
        .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
    match section {
        "locator" => Ok(slot.sections.locator.is_some()),
        "core" => Ok(slot.sections.core.is_some()),
        "records" => Ok(slot.sections.records.is_some()),
        "refs" => Ok(slot.sections.refs.is_some()),
        "assets" => Ok(slot.sections.assets.is_some()),
        other => Err(PyValueError::new_err(format!(
            "unknown plugin index section: {other}"
        ))),
    }
}

#[pyfunction(name = "plugin_handle_force_build_records_section")]
pub fn plugin_handle_force_build_records_section_native(handle_id: u64) -> PyResult<()> {
    let mut store = plugin_handle_store().lock().unwrap();
    let slot = store
        .get_mut(&handle_id)
        .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
    ensure_records_section(slot);
    Ok(())
}

#[pyfunction(name = "plugin_handle_force_build_refs_section")]
pub fn plugin_handle_force_build_refs_section_native(
    py: Python<'_>,
    handle_id: u64,
) -> PyResult<usize> {
    py.detach(move || {
        let mut store = plugin_handle_store().lock().unwrap();
        let slot = store
            .get_mut(&handle_id)
            .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
        let refs = ensure_refs_section(slot);
        Ok(refs.forward_refs_by_form_key.len())
    })
}

#[pyfunction(name = "plugin_handle_assets_by_kind")]
pub fn plugin_handle_assets_by_kind_native(
    py: Python<'_>,
    handle_id: u64,
    kind: &str,
) -> PyResult<Vec<(String, String)>> {
    let kind = kind.to_ascii_lowercase();
    py.detach(move || {
        let mut store = plugin_handle_store().lock().unwrap();
        let slot = store
            .get_mut(&handle_id)
            .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
        let assets = ensure_assets_section(slot);
        Ok(assets
            .assets_by_kind
            .get(&SmolStr::new(kind.as_str()))
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|(form_key, path)| (form_key.to_string(), path))
            .collect())
    })
}

#[pyfunction(name = "plugin_handle_record_eid_index")]
pub fn plugin_handle_record_eid_index_native(
    py: Python<'_>,
    handle_id: u64,
) -> PyResult<Py<PyAny>> {
    let index = py.detach(move || {
        let mut store = plugin_handle_store().lock().unwrap();
        let slot = store
            .get_mut(&handle_id)
            .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
        let core = ensure_core_section(slot);
        Ok::<_, PyErr>(
            core.by_eid_lower
                .iter()
                .map(|(eid, form_keys)| {
                    (
                        eid.clone(),
                        form_keys
                            .iter()
                            .map(|value| value.render())
                            .collect::<Vec<_>>(),
                    )
                })
                .collect::<HashMap<_, _>>(),
        )
    })?;
    index.into_py_any(py)
}

type RecordIndexRow = (String, String, String, u32, u32);

#[pyfunction(name = "plugin_handle_record_index_rows")]
#[pyo3(signature = (handle_id, signatures=None, form_keys=None))]
pub fn plugin_handle_record_index_rows_native(
    py: Python<'_>,
    handle_id: u64,
    signatures: Option<Vec<String>>,
    form_keys: Option<Vec<String>>,
) -> PyResult<Vec<RecordIndexRow>> {
    py.detach(move || {
        let mut store = plugin_handle_store().lock().unwrap();
        let slot = store
            .get_mut(&handle_id)
            .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
        let core = ensure_core_section(slot);
        let wanted_signatures = signatures.map(|values| {
            values
                .into_iter()
                .map(|value| SmolStr::new(value.trim().to_ascii_uppercase()))
                .collect::<HashSet<_>>()
        });
        let include = |entry: &&RecordIndexEntry| {
            wanted_signatures
                .as_ref()
                .map(|wanted| wanted.contains(&entry.signature))
                .unwrap_or(true)
        };
        let preserve_input_order = form_keys.is_some();
        let mut entries: Vec<&RecordIndexEntry> = match form_keys {
            Some(values) => values
                .iter()
                .filter_map(|value| record_index_entry_by_form_key(&core, value))
                .filter(include)
                .collect(),
            None => core.by_form_key.values().filter(include).collect(),
        };
        if !preserve_input_order {
            entries.sort_unstable_by(|left, right| {
                left.form_key.render().cmp(&right.form_key.render())
            });
        }
        Ok(entries
            .into_iter()
            .map(|entry| {
                (
                    entry.form_key.render(),
                    entry.eid.clone(),
                    entry.signature.to_string(),
                    entry.object_id,
                    entry.raw_form_id,
                )
            })
            .collect())
    })
}

#[pyfunction(name = "plugin_handle_local_object_ids")]
pub fn plugin_handle_local_object_ids_native(py: Python<'_>, handle_id: u64) -> PyResult<Vec<u32>> {
    py.detach(move || {
        let store = plugin_handle_store().lock().unwrap();
        let slot = store
            .get(&handle_id)
            .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
        let mut records = Vec::new();
        let mut predicate = |_record: &ParsedRecord| true;
        collect_records(&slot.parsed.root_items, &mut predicate, &mut records);
        Ok(records
            .into_iter()
            .filter_map(|record| {
                let object_id = record.form_id & 0x00FF_FFFF;
                (object_id != 0).then_some(object_id)
            })
            .collect())
    })
}

#[pyfunction(name = "plugin_handle_owned_object_ids")]
pub fn plugin_handle_owned_object_ids_native(py: Python<'_>, handle_id: u64) -> PyResult<Vec<u32>> {
    py.detach(move || {
        let store = plugin_handle_store().lock().unwrap();
        let slot = store
            .get(&handle_id)
            .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
        let own_index = (slot.parsed.header.masters.len() & 0xFF) as u8;
        let mut records = Vec::new();
        let mut predicate = |_record: &ParsedRecord| true;
        collect_records(&slot.parsed.root_items, &mut predicate, &mut records);
        let mut out = BTreeSet::new();
        for record in records {
            if ((record.form_id >> 24) & 0xFF) as u8 == own_index {
                let object_id = record.form_id & 0x00FF_FFFF;
                if object_id != 0 {
                    out.insert(object_id);
                }
            }
        }
        Ok(out.into_iter().collect())
    })
}

#[pyfunction(name = "plugin_handle_record_form_ids")]
#[pyo3(signature = (handle_id, signatures=None))]
pub fn plugin_handle_record_form_ids_native(
    py: Python<'_>,
    handle_id: u64,
    signatures: Option<Vec<String>>,
) -> PyResult<Vec<u32>> {
    py.detach(move || {
        let mut store = plugin_handle_store().lock().unwrap();
        let slot = store
            .get_mut(&handle_id)
            .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
        let wanted = signatures.map(|values| {
            values
                .into_iter()
                .map(|value| SmolStr::new(value.as_str()))
                .collect::<HashSet<_>>()
        });
        if slot.lazy.is_some() {
            let core = ensure_core_section(slot);
            let lazy = slot.lazy.as_ref().expect("checked above");
            let mut indexed = core
                .by_form_key
                .values()
                .filter(|entry| {
                    wanted
                        .as_ref()
                        .map(|set| set.contains(&entry.signature))
                        .unwrap_or(true)
                })
                .filter_map(|entry| {
                    lazy.offsets()
                        .get(&entry.raw_form_id)
                        .map(|offset| (*offset, entry.raw_form_id))
                })
                .collect::<Vec<_>>();
            indexed.sort_unstable_by_key(|(offset, _)| *offset);
            return Ok(indexed.into_iter().map(|(_, form_id)| form_id).collect());
        }
        let mut records = Vec::new();
        let mut predicate = |record: &ParsedRecord| {
            wanted
                .as_ref()
                .map(|set| set.contains(&record.signature))
                .unwrap_or(true)
        };
        collect_records(&slot.parsed.root_items, &mut predicate, &mut records);
        Ok(records.into_iter().map(|record| record.form_id).collect())
    })
}

type InspectionRecordPayload = (String, u32, Option<u16>, Vec<(String, Bytes)>);

#[pyfunction(name = "plugin_handle_inspection_records")]
#[pyo3(signature = (handle_id, signatures, subrecord_signatures))]
pub fn plugin_handle_inspection_records_native<'py>(
    py: Python<'py>,
    handle_id: u64,
    signatures: Vec<String>,
    subrecord_signatures: Vec<String>,
) -> PyResult<Bound<'py, PyList>> {
    let records = py.detach(move || -> PyResult<Vec<InspectionRecordPayload>> {
        let wanted: HashSet<SmolStr> = signatures.into_iter().map(SmolStr::new).collect();
        let wanted_subrecords: HashSet<SmolStr> =
            subrecord_signatures.into_iter().map(SmolStr::new).collect();
        let store = plugin_handle_store().lock().unwrap();
        let slot = store
            .get(&handle_id)
            .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
        let mut rows = Vec::new();
        let mut inspect = |record: &ParsedRecord| {
            if !wanted.contains(&record.signature) && wanted_subrecords.is_empty() {
                return;
            }
            let subrecords = effective_subrecords_for_record(record);
            if !wanted.contains(&record.signature)
                && !subrecords
                    .iter()
                    .any(|subrecord| wanted_subrecords.contains(&subrecord.signature))
            {
                return;
            }
            rows.push((
                record.signature.to_string(),
                record.form_id,
                record.form_version,
                subrecords
                    .iter()
                    .map(|subrecord| (subrecord.signature.to_string(), subrecord.data.clone()))
                    .collect(),
            ));
        };
        if let Some(lazy) = slot.lazy.as_ref() {
            let cursor = lazy.cursor();
            let mut parse_error = None;
            let outcome = cursor.scan(&mut |view| {
                if wanted_subrecords.is_empty()
                    && !wanted.iter().any(|sig| sig.as_bytes() == view.signature)
                {
                    return std::ops::ControlFlow::Continue(());
                }
                match cursor.parse_at(view.offset) {
                    Ok(record) => inspect(&record),
                    Err(error) => {
                        parse_error = Some(error);
                        return std::ops::ControlFlow::Break(());
                    }
                }
                std::ops::ControlFlow::Continue(())
            });
            if let Some(error) = parse_error {
                return Err(error);
            }
            if let crate::record_cursor::ScanOutcome::Truncated { offset } = outcome {
                return Err(value_error(format!(
                    "truncated plugin during record inspection at byte {offset}"
                )));
            }
        } else {
            let mut records = Vec::new();
            collect_records(&slot.parsed.root_items, &mut |_| true, &mut records);
            for record in records {
                inspect(record);
            }
        }
        Ok(rows)
    })?;
    let rows = PyList::empty(py);
    for (signature, form_id, form_version, subrecords) in records {
        let payloads = PyList::empty(py);
        for (subrecord_signature, data) in subrecords {
            payloads.append((subrecord_signature, PyBytes::new(py, &data)))?;
        }
        rows.append((signature, form_id, form_version, payloads))?;
    }
    Ok(rows)
}

type ValidationRecordPayload = (String, u32, Vec<(String, Vec<u8>)>);

#[pyfunction(name = "plugin_handle_validation_records")]
pub fn plugin_handle_validation_records_native(
    py: Python<'_>,
    handle_id: u64,
) -> PyResult<Vec<ValidationRecordPayload>> {
    py.detach(move || {
        use rayon::prelude::*;

        let store = plugin_handle_store().lock().unwrap();
        let slot = store
            .get(&handle_id)
            .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
        let mut records = Vec::new();
        collect_records(&slot.parsed.root_items, &mut |_| true, &mut records);
        Ok(records
            .par_iter()
            .filter_map(|record| {
                let subrecords = effective_subrecords_for_record(record);
                let signature = record.signature.as_str();
                let relevant_signature = matches!(
                    signature,
                    "ARMO"
                        | "FURN"
                        | "MOVT"
                        | "TERM"
                        | "WEAP"
                        | "MGEF"
                        | "LVLI"
                        | "LVLN"
                        | "LVLC"
                        | "LVSP"
                );
                let has_condition = subrecords
                    .iter()
                    .any(|subrecord| matches!(subrecord.signature.as_str(), "CTDA" | "CTDT"));
                if !relevant_signature && !has_condition {
                    return None;
                }
                Some((
                    signature.to_string(),
                    record.form_id,
                    subrecords
                        .iter()
                        .map(|subrecord| (subrecord.signature.to_string(), subrecord.data.to_vec()))
                        .collect(),
                ))
            })
            .collect())
    })
}

#[derive(Clone)]
enum RecordSearchMatcher {
    Substring {
        needle: String,
        case_sensitive: bool,
    },
    Regex(regex::Regex),
}

impl RecordSearchMatcher {
    fn compile(pattern: &str, mode: &str, case_sensitive: bool) -> PyResult<Self> {
        match mode {
            "substring" => Ok(Self::Substring {
                needle: if case_sensitive {
                    pattern.to_string()
                } else {
                    pattern.to_lowercase()
                },
                case_sensitive,
            }),
            "regex" => regex::RegexBuilder::new(pattern)
                .case_insensitive(!case_sensitive)
                .build()
                .map(Self::Regex)
                .map_err(|error| value_error(format!("invalid search regex: {error}"))),
            "glob" => regex::RegexBuilder::new(glob_search_regex(pattern).as_str())
                .case_insensitive(!case_sensitive)
                .dot_matches_new_line(true)
                .build()
                .map(Self::Regex)
                .map_err(|error| value_error(format!("invalid search glob: {error}"))),
            other => Err(value_error(format!(
                "unknown search mode: {other:?} (expected glob, substring, or regex)"
            ))),
        }
    }

    fn is_match(&self, value: &str) -> bool {
        match self {
            Self::Substring {
                needle,
                case_sensitive,
            } => {
                if *case_sensitive {
                    value.contains(needle)
                } else {
                    value.to_lowercase().contains(needle)
                }
            }
            Self::Regex(regex) => regex.is_match(value),
        }
    }
}

fn glob_search_regex(pattern: &str) -> String {
    let mut regex = String::from("^(?:");
    let mut chars = pattern.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '*' => regex.push_str(".*"),
            '?' => regex.push('.'),
            '[' => {
                let mut class = String::new();
                let mut closed = false;
                if matches!(chars.peek(), Some('!' | '^')) {
                    chars.next();
                    class.push('^');
                }
                if matches!(chars.peek(), Some(']')) {
                    chars.next();
                    class.push_str(r"\]");
                }
                for class_ch in chars.by_ref() {
                    if class_ch == ']' {
                        closed = true;
                        break;
                    }
                    if class_ch == '\\' {
                        class.push_str(r"\\");
                    } else {
                        class.push(class_ch);
                    }
                }
                if closed {
                    regex.push('[');
                    regex.push_str(&class);
                    regex.push(']');
                } else {
                    regex.push_str(r"\[");
                    regex.push_str(&regex::escape(&class));
                }
            }
            _ => regex.push_str(&regex::escape(ch.to_string().as_str())),
        }
    }
    regex.push_str(")$");
    regex
}

#[pyfunction(name = "plugin_handle_search_records")]
#[pyo3(signature = (handle_id, pattern, mode="glob", match_full=false, read_full=false, signatures=None, case_sensitive=false, limit=None))]
pub fn plugin_handle_search_records_native(
    py: Python<'_>,
    handle_id: u64,
    pattern: String,
    mode: &str,
    match_full: bool,
    read_full: bool,
    signatures: Option<Vec<String>>,
    case_sensitive: bool,
    limit: Option<usize>,
) -> PyResult<Vec<(u32, String, Option<String>, Option<String>)>> {
    let matcher = RecordSearchMatcher::compile(pattern.as_str(), mode, case_sensitive)?;
    py.detach(move || {
        use rayon::prelude::*;

        let store = plugin_handle_store().lock().unwrap();
        let slot = store
            .get(&handle_id)
            .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
        let wanted = signatures.map(|values| {
            values
                .into_iter()
                .map(|value| SmolStr::new(value.as_str()))
                .collect::<HashSet<_>>()
        });
        let want_full = match_full || read_full;

        // A lazy handle has no tree to collect from. Stream instead, parsing
        // each record only long enough to test it and dropping it again, so the
        // scan stays flat in memory.
        if let Some(lazy) = slot.lazy.as_ref() {
            let cursor = lazy.cursor();
            let mut matches = Vec::new();
            cursor.scan(&mut |view| {
                if let Some(wanted) = wanted.as_ref() {
                    let signature = SmolStr::new(String::from_utf8_lossy(view.signature).as_ref());
                    if !wanted.contains(&signature) {
                        return std::ops::ControlFlow::Continue(());
                    }
                }
                let Ok(record) = cursor.parse_at(view.offset) else {
                    return std::ops::ControlFlow::Continue(());
                };
                let editor_id = record_editor_id_value(&record);
                let full_name = want_full.then(|| full_name_from_parsed(&record)).flatten();
                let matched = editor_id
                    .as_deref()
                    .is_some_and(|value| matcher.is_match(value))
                    || (match_full
                        && full_name
                            .as_deref()
                            .is_some_and(|value| matcher.is_match(value)));
                if matched {
                    matches.push((
                        record.form_id,
                        record.signature.to_string(),
                        editor_id,
                        full_name,
                    ));
                    // Both paths walk in file order, so stopping here yields the
                    // same first N the eager path's trailing truncate would.
                    if limit.is_some_and(|limit| matches.len() >= limit) {
                        return std::ops::ControlFlow::Break(());
                    }
                }
                std::ops::ControlFlow::Continue(())
            });
            return Ok(matches);
        }

        let mut records = Vec::new();
        collect_records(&slot.parsed.root_items, &mut |_| true, &mut records);
        let mut matches: Vec<_> = records
            .par_iter()
            .filter_map(|record| {
                if wanted
                    .as_ref()
                    .is_some_and(|signatures| !signatures.contains(&record.signature))
                {
                    return None;
                }
                let editor_id = record_editor_id_value(record);
                let full_name = want_full.then(|| full_name_from_parsed(record)).flatten();
                let matched = editor_id
                    .as_deref()
                    .is_some_and(|value| matcher.is_match(value))
                    || (match_full
                        && full_name
                            .as_deref()
                            .is_some_and(|value| matcher.is_match(value)));
                matched.then(|| {
                    (
                        record.form_id,
                        record.signature.to_string(),
                        editor_id,
                        full_name,
                    )
                })
            })
            .collect();
        if let Some(limit) = limit {
            matches.truncate(limit);
        }
        Ok(matches)
    })
}

#[pyfunction(name = "plugin_handle_record_form_ids_with_subrecords")]
pub fn plugin_handle_record_form_ids_with_subrecords_native(
    py: Python<'_>,
    handle_id: u64,
    subrecord_signatures: Vec<String>,
) -> PyResult<Vec<u32>> {
    py.detach(move || {
        let wanted = subrecord_signatures
            .into_iter()
            .map(|value| SmolStr::new(value.as_str()))
            .collect::<HashSet<_>>();
        if wanted.is_empty() {
            return Ok(Vec::new());
        }
        let store = plugin_handle_store().lock().unwrap();
        let slot = store
            .get(&handle_id)
            .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
        if let Some(lazy) = slot.lazy.as_ref() {
            let cursor = lazy.cursor();
            let mut form_ids = Vec::new();
            cursor.scan(&mut |view| {
                if let Ok(record) = cursor.parse_at(view.offset) {
                    if record
                        .subrecords
                        .iter()
                        .any(|subrecord| wanted.contains(&subrecord.signature))
                    {
                        form_ids.push(record.form_id);
                    }
                }
                std::ops::ControlFlow::Continue(())
            });
            return Ok(form_ids);
        }
        let mut records = Vec::new();
        let mut predicate = |record: &ParsedRecord| {
            record
                .subrecords
                .iter()
                .any(|subrecord| wanted.contains(&subrecord.signature))
        };
        collect_records(&slot.parsed.root_items, &mut predicate, &mut records);
        Ok(records.into_iter().map(|record| record.form_id).collect())
    })
}

#[pyfunction(name = "plugin_handle_record_subrecords")]
pub fn plugin_handle_record_subrecords_native(
    py: Python<'_>,
    handle_id: u64,
    form_id: u32,
) -> PyResult<Option<Vec<ParsedSubrecordPayload>>> {
    py.detach(move || {
        let mut store = plugin_handle_store().lock().unwrap();
        let slot = store
            .get_mut(&handle_id)
            .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
        let raw_form_id = form_id & 0xFFFF_FFFF;
        let own_index = (slot.parsed.header.masters.len() & 0xFF) as u8;
        let resolved = {
            let records = ensure_records_section(slot);
            if records.record(&slot.parsed, raw_form_id).is_some() {
                Some(raw_form_id)
            } else {
                let object_id = raw_form_id & 0x00FF_FFFF;
                let core = ensure_core_section(slot);
                core.form_ids_by_object_id
                    .get(&object_id)
                    .and_then(|ids| pick_owned_form_id(ids, own_index))
            }
        };
        let Some(resolved) = resolved else {
            return Ok(None);
        };
        let payloads = if slot.lazy.is_some() {
            let Some(record) = lazy_materialize_record(slot, resolved) else {
                return Ok(None);
            };
            record
                .subrecords
                .iter()
                .map(|subrecord| {
                    (
                        subrecord.signature.to_string(),
                        subrecord.data.to_vec(),
                        subrecord.semantic_type.clone(),
                    )
                })
                .collect()
        } else {
            let records = ensure_records_section(slot);
            let Some(record) = records.record(&slot.parsed, resolved) else {
                return Ok(None);
            };
            record
                .subrecords
                .iter()
                .map(|subrecord| {
                    (
                        subrecord.signature.to_string(),
                        subrecord.data.to_vec(),
                        subrecord.semantic_type.clone(),
                    )
                })
                .collect()
        };
        Ok(Some(payloads))
    })
}

#[pyfunction(name = "plugin_handle_set_record_subrecords")]
pub fn plugin_handle_set_record_subrecords_native(
    py: Python<'_>,
    handle_id: u64,
    form_id: u32,
    subrecords: Vec<ParsedSubrecordPayload>,
) -> PyResult<bool> {
    py.detach(move || {
        let mut store = plugin_handle_store().lock().unwrap();
        let slot = store
            .get_mut(&handle_id)
            .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
        let raw_form_id = form_id & 0xFFFF_FFFF;
        let record =
            find_record_mut(&mut slot.parsed.root_items, raw_form_id).ok_or_else(|| {
                PyKeyError::new_err(format!("unknown record form_id: {raw_form_id:08X}"))
            })?;
        record.subrecords = subrecords
            .into_iter()
            .map(|(signature, data, semantic_type)| ParsedSubrecord {
                signature: SmolStr::new(signature.as_str()),
                data: Bytes::from(data),
                semantic_type,
            })
            .collect();
        slot.sections.apply_effect(&WriteEffect::RecordContents {
            form_ids: smallvec::smallvec![raw_form_id],
        });
        Ok(true)
    })
}

fn remove_formid_subrecords_in_items(
    items: &mut [ParsedItem],
    record_signature: &str,
    subrecord_signature: &str,
    target_form_id: u32,
    dry_run: bool,
    changes: &mut Vec<(u32, Option<String>, usize)>,
) {
    let target_payload = target_form_id.to_le_bytes();
    for item in items {
        match item {
            ParsedItem::Record(record) if record.signature.as_str() == record_signature => {
                let removed = record
                    .subrecords
                    .iter()
                    .filter(|subrecord| {
                        subrecord.signature.as_str() == subrecord_signature
                            && subrecord.data.as_ref() == target_payload
                    })
                    .count();
                if removed == 0 {
                    continue;
                }
                let editor_id = record_editor_id_value(record);
                if !dry_run {
                    record.subrecords.retain(|subrecord| {
                        subrecord.signature.as_str() != subrecord_signature
                            || subrecord.data.as_ref() != target_payload
                    });
                    record.raw_payload = None;
                }
                changes.push((record.form_id, editor_id, removed));
            }
            ParsedItem::Group(group) => remove_formid_subrecords_in_items(
                &mut group.children,
                record_signature,
                subrecord_signature,
                target_form_id,
                dry_run,
                changes,
            ),
            ParsedItem::Record(_) => {}
        }
    }
}

#[pyfunction(name = "plugin_handle_remove_formid_subrecords")]
#[pyo3(signature = (handle_id, record_signature, subrecord_signature, target_form_id, dry_run=false))]
pub fn plugin_handle_remove_formid_subrecords_native(
    py: Python<'_>,
    handle_id: u64,
    record_signature: String,
    subrecord_signature: String,
    target_form_id: u32,
    dry_run: bool,
) -> PyResult<Vec<(u32, Option<String>, usize)>> {
    py.detach(move || {
        let mut store = plugin_handle_store().lock().unwrap();
        let slot = store
            .get_mut(&handle_id)
            .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
        if slot.lazy.is_some() {
            return Err(PyRuntimeError::new_err(
                "remove-formid-subrecords requires an editable eager plugin handle",
            ));
        }
        let mut changes = Vec::new();
        remove_formid_subrecords_in_items(
            &mut slot.parsed.root_items,
            record_signature.as_str(),
            subrecord_signature.as_str(),
            target_form_id,
            dry_run,
            &mut changes,
        );
        if !dry_run && !changes.is_empty() {
            slot.sections.apply_effect(&WriteEffect::RecordContents {
                form_ids: changes.iter().map(|(form_id, _, _)| *form_id).collect(),
            });
        }
        Ok(changes)
    })
}

fn collect_owned_term_marker_parameters(
    items: &[ParsedItem],
    own_index: u8,
    markers_by_object_id: &mut HashMap<u32, Vec<Bytes>>,
) {
    for item in items {
        match item {
            ParsedItem::Record(record)
                if record.signature.as_str() == "TERM"
                    && matches!(((record.form_id >> 24) & 0xFF) as u8, index if index == own_index || index == LOCAL_FORM_INDEX) =>
            {
                let subrecords = effective_subrecords_for_record(record);
                let markers = subrecords
                    .iter()
                    .filter(|subrecord| {
                        subrecord.signature.as_str() == "ZNAM"
                            && !subrecord.data.is_empty()
                            && subrecord.data.len() % 24 == 0
                    })
                    .map(|subrecord| subrecord.data.clone())
                    .collect::<Vec<_>>();
                if !markers.is_empty() {
                    markers_by_object_id.insert(record.form_id & 0x00FF_FFFF, markers);
                }
            }
            ParsedItem::Group(group) => collect_owned_term_marker_parameters(
                &group.children,
                own_index,
                markers_by_object_id,
            ),
            ParsedItem::Record(_) => {}
        }
    }
}

fn repair_term_marker_parameters_in_items(
    items: &mut [ParsedItem],
    own_index: u8,
    markers_by_object_id: &HashMap<u32, Vec<Bytes>>,
    dry_run: bool,
    changes: &mut Vec<(u32, Option<String>, usize, usize)>,
) {
    for item in items {
        match item {
            ParsedItem::Record(record)
                if record.signature.as_str() == "TERM"
                    && matches!(((record.form_id >> 24) & 0xFF) as u8, index if index == own_index || index == LOCAL_FORM_INDEX) =>
            {
                let Some(source_markers) =
                    markers_by_object_id.get(&(record.form_id & 0x00FF_FFFF))
                else {
                    continue;
                };
                let was_compressed = record.raw_payload.is_some();
                let effective_subrecords = effective_subrecords_for_record(record);
                let Some(marker_anchor) = effective_subrecords
                    .iter()
                    .rposition(|subrecord| subrecord.signature.as_str() == "XMRK")
                else {
                    continue;
                };
                let existing_markers = effective_subrecords
                    .iter()
                    .skip(marker_anchor + 1)
                    .filter(|subrecord| subrecord.signature.as_str() == "SNAM")
                    .map(|subrecord| subrecord.data.as_ref())
                    .collect::<Vec<_>>();
                if existing_markers.len() == source_markers.len()
                    && existing_markers
                        .iter()
                        .zip(source_markers)
                        .all(|(target, source)| *target == source.as_ref())
                {
                    continue;
                }

                let editor_id = record_editor_id_value(record);
                let removed = existing_markers.len();
                drop(existing_markers);
                if !dry_run {
                    if was_compressed {
                        record.subrecords = effective_subrecords.into_owned();
                        record.raw_payload = None;
                    } else {
                        drop(effective_subrecords);
                    }
                    let old_subrecords = std::mem::take(&mut record.subrecords);
                    let mut rebuilt = Vec::with_capacity(
                        old_subrecords.len() + source_markers.len().saturating_sub(removed),
                    );
                    for (index, subrecord) in old_subrecords.into_iter().enumerate() {
                        if index > marker_anchor && subrecord.signature.as_str() == "SNAM" {
                            continue;
                        }
                        rebuilt.push(subrecord);
                        if index == marker_anchor {
                            rebuilt.extend(source_markers.iter().cloned().map(|data| {
                                ParsedSubrecord {
                                    signature: SmolStr::new_static("SNAM"),
                                    data,
                                    semantic_type: None,
                                }
                            }));
                        }
                    }
                    record.subrecords = rebuilt;
                    record.raw_payload = None;
                }
                changes.push((record.form_id, editor_id, removed, source_markers.len()));
            }
            ParsedItem::Group(group) => repair_term_marker_parameters_in_items(
                &mut group.children,
                own_index,
                markers_by_object_id,
                dry_run,
                changes,
            ),
            ParsedItem::Record(_) => {}
        }
    }
}

#[pyfunction(name = "plugin_handle_repair_term_marker_parameters_from_source")]
#[pyo3(signature = (target_handle_id, source_handle_id, dry_run=false))]
pub fn plugin_handle_repair_term_marker_parameters_from_source_native(
    py: Python<'_>,
    target_handle_id: u64,
    source_handle_id: u64,
    dry_run: bool,
) -> PyResult<Vec<(u32, Option<String>, usize, usize)>> {
    let result = py.detach(move || {
        plugin_handle_repair_term_marker_parameters_from_source_no_py(
            target_handle_id,
            source_handle_id,
            dry_run,
        )
    });
    result.map_err(|error| {
        if error == "target and source plugin handles must be different" {
            PyValueError::new_err(error)
        } else if error.starts_with("unknown source plugin handle:")
            || error.starts_with("unknown target plugin handle:")
        {
            PyKeyError::new_err(error)
        } else {
            PyRuntimeError::new_err(error)
        }
    })
}

pub fn plugin_handle_repair_term_marker_parameters_from_source_no_py(
    target_handle_id: u64,
    source_handle_id: u64,
    dry_run: bool,
) -> Result<Vec<(u32, Option<String>, usize, usize)>, String> {
    if target_handle_id == source_handle_id {
        return Err("target and source plugin handles must be different".to_string());
    }
    let mut store = plugin_handle_store().lock().unwrap();
    let markers_by_object_id = {
        let source = store
            .get(&source_handle_id)
            .ok_or_else(|| format!("unknown source plugin handle: {source_handle_id}"))?;
        if source.lazy.is_some() {
            return Err(
                "repair-term-marker-parameters requires an eager source plugin handle".to_string(),
            );
        }
        let source_own_index = (source.parsed.header.masters.len() & 0xFF) as u8;
        let mut markers = HashMap::new();
        collect_owned_term_marker_parameters(
            &source.parsed.root_items,
            source_own_index,
            &mut markers,
        );
        markers
    };
    let target = store
        .get_mut(&target_handle_id)
        .ok_or_else(|| format!("unknown target plugin handle: {target_handle_id}"))?;
    if target.lazy.is_some() {
        return Err(
            "repair-term-marker-parameters requires an editable eager target plugin handle"
                .to_string(),
        );
    }
    let target_own_index = (target.parsed.header.masters.len() & 0xFF) as u8;
    let mut changes = Vec::new();
    repair_term_marker_parameters_in_items(
        &mut target.parsed.root_items,
        target_own_index,
        &markers_by_object_id,
        dry_run,
        &mut changes,
    );
    if !dry_run && !changes.is_empty() {
        target.sections.apply_effect(&WriteEffect::RecordContents {
            form_ids: changes.iter().map(|(form_id, _, _, _)| *form_id).collect(),
        });
    }
    Ok(changes)
}

#[pyfunction(name = "plugin_handle_record_flags")]
pub fn plugin_handle_record_flags_native(
    py: Python<'_>,
    handle_id: u64,
    form_id: u32,
) -> PyResult<Option<u32>> {
    py.detach(move || {
        let mut store = plugin_handle_store().lock().unwrap();
        let slot = store
            .get_mut(&handle_id)
            .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
        let raw_form_id = form_id & 0xFFFF_FFFF;
        let own_index = (slot.parsed.header.masters.len() & 0xFF) as u8;
        let records = ensure_records_section(slot);
        if let Some(record) = records.record(&slot.parsed, raw_form_id) {
            return Ok(Some(record.flags));
        }
        let object_id = raw_form_id & 0x00FF_FFFF;
        let core = ensure_core_section(slot);
        if let Some(form_ids) = core.form_ids_by_object_id.get(&object_id) {
            if let Some(picked) = pick_owned_form_id(form_ids, own_index) {
                return Ok(records
                    .record(&slot.parsed, picked)
                    .map(|record| record.flags));
            }
        }
        Ok(None)
    })
}

#[pyfunction(name = "plugin_handle_set_record_flags")]
pub fn plugin_handle_set_record_flags_native(
    py: Python<'_>,
    handle_id: u64,
    form_id: u32,
    flags: u32,
) -> PyResult<Option<u32>> {
    py.detach(move || {
        let mut store = plugin_handle_store().lock().unwrap();
        let slot = store
            .get_mut(&handle_id)
            .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
        let raw_form_id = form_id & 0xFFFF_FFFF;
        let Some(record) = find_record_mut(&mut slot.parsed.root_items, raw_form_id) else {
            return Ok(None);
        };
        let previous = record.flags;
        record.flags = flags;
        slot.sections.apply_effect(&WriteEffect::RecordContents {
            form_ids: smallvec::smallvec![raw_form_id],
        });
        Ok(Some(previous))
    })
}

#[pyfunction(name = "plugin_handle_used_master_indices")]
pub fn plugin_handle_used_master_indices_native(
    py: Python<'_>,
    handle_id: u64,
) -> PyResult<Vec<u8>> {
    py.detach(move || {
        let store = plugin_handle_store().lock().unwrap();
        let slot = store
            .get(&handle_id)
            .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
        master_edit::used_indices(slot).map_err(PyValueError::new_err)
    })
}

#[pyfunction(name = "plugin_handle_apply_object_id_mapping")]
pub fn plugin_handle_apply_object_id_mapping_native(
    py: Python<'_>,
    handle_id: u64,
    old_high: u8,
    new_high: u8,
    object_id_map: Vec<(u32, u32)>,
) -> PyResult<usize> {
    py.detach(move || {
        let mapping: HashMap<u32, u32> = object_id_map
            .into_iter()
            .map(|(old_id, new_id)| (old_id & 0x00FF_FFFF, new_id & 0x00FF_FFFF))
            .collect();
        if mapping.is_empty() {
            return Ok(0);
        }
        let mut store = plugin_handle_store().lock().unwrap();
        let slot = store
            .get_mut(&handle_id)
            .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
        // Schema-aware so subrecord references on disk-loaded plugins (which
        // carry no semantic_type) get remapped too — without this, ESL
        // compaction / renumber would renumber records but leave every internal
        // reference dangling.
        let schema = slot
            .parsed
            .game
            .as_deref()
            .and_then(|game| compiled_schema_for_game(game).ok());
        let changed = apply_object_id_mapping_in_items(
            &mut slot.parsed.root_items,
            old_high,
            new_high,
            &mapping,
            schema.as_deref(),
        );
        if changed > 0 {
            slot.record_count_cache = None;
            slot.sections
                .apply_effect(&WriteEffect::RecordsAddedOrRemoved);
        }
        Ok(changed)
    })
}

/// Null every formid/formid_array subrecord value whose master high byte equals
/// `index`. Used by `esp masters remove --force` to scrub references to a master
/// being dropped (record-level overrides of that master are removed separately
/// in Python). Returns the count of reference slots set to NULL.
#[pyfunction(name = "plugin_handle_null_refs_to_master")]
pub fn plugin_handle_null_refs_to_master_native(
    py: Python<'_>,
    handle_id: u64,
    index: u8,
) -> PyResult<usize> {
    py.detach(move || {
        let mut store = plugin_handle_store().lock().unwrap();
        let slot = store
            .get_mut(&handle_id)
            .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
        let nulled = master_edit::null_refs(slot, index).map_err(PyValueError::new_err)?;
        if nulled > 0 {
            slot.sections
                .apply_effect(&WriteEffect::RecordsAddedOrRemoved);
        }
        Ok(nulled)
    })
}

#[pyfunction(name = "plugin_handle_copy_record")]
pub fn plugin_handle_copy_record_native(
    py: Python<'_>,
    source_handle_id: u64,
    source_form_id: u32,
    target_handle_id: u64,
    as_new: bool,
) -> PyResult<Option<u32>> {
    py.detach(move || {
        let mut store = plugin_handle_store().lock().unwrap();
        let (mut record, source_masters, source_plugin_name, source_own_index, source_game) = {
            let source_slot = store.get_mut(&source_handle_id).ok_or_else(|| {
                PyKeyError::new_err(format!("unknown plugin handle: {source_handle_id}"))
            })?;
            let Some(record) = cloned_record_for_form_id(source_slot, source_form_id) else {
                return Ok(None);
            };
            (
                record,
                source_slot.parsed.header.masters.clone(),
                source_slot.parsed.plugin_name.clone(),
                (source_slot.parsed.header.masters.len() & 0xFF) as u8,
                source_slot.parsed.game.clone(),
            )
        };
        let target_slot = store.get_mut(&target_handle_id).ok_or_else(|| {
            PyKeyError::new_err(format!("unknown plugin handle: {target_handle_id}"))
        })?;
        let target_masters = target_slot.parsed.header.masters.clone();
        let schema = source_game.as_deref().map(compiled_schema_for_game).transpose()?;
        master_edit::copy_record(
            &mut record,
            &source_masters,
            &source_plugin_name,
            &target_masters,
            source_own_index,
            schema.as_deref(),
        ).map_err(value_error)?;
        record.raw_payload = None;
        if as_new {
            let object_id = target_slot.parsed.header.next_object_id & 0x00FF_FFFF;
            target_slot.parsed.header.next_object_id = (object_id + 1) & 0x00FF_FFFF;
            record.form_id = ((LOCAL_FORM_INDEX as u32) << 24) | object_id;
        }
        let inserted_form_id = record.form_id;
        replace_parsed_record_in_slot(target_slot, record);
        target_slot
            .sections
            .apply_effect(&WriteEffect::RecordsAddedOrRemoved);
        Ok(Some(inserted_form_id))
    })
}

#[derive(Clone)]
struct MergeSourceRecord {
    plugin_name: String,
    masters: Vec<String>,
    own_index: u8,
    record: ParsedRecord,
}

#[pyfunction(name = "plugin_handle_merge_conflict_to_patch")]
pub fn plugin_handle_merge_conflict_to_patch_native(
    py: Python<'_>,
    target_handle_id: u64,
    signature: String,
    chain: Vec<(u64, String, i32, u32)>,
) -> PyResult<bool> {
    py.detach(move || {
        if chain.is_empty() {
            return Err(value_error("merge conflict chain is empty"));
        }
        let mut sorted_chain = chain;
        sorted_chain.sort_by_key(|(_, _, load_order_index, _)| *load_order_index);

        let mut store = plugin_handle_store().lock().unwrap();
        let mut sources = Vec::with_capacity(sorted_chain.len());
        for (handle_id, plugin_name, _load_order_index, form_id) in sorted_chain {
            let source_slot = store.get_mut(&handle_id).ok_or_else(|| {
                PyKeyError::new_err(format!("unknown plugin handle: {handle_id}"))
            })?;
            let Some(record) = cloned_record_for_form_id(source_slot, form_id) else {
                return Ok(false);
            };
            sources.push(MergeSourceRecord {
                plugin_name,
                masters: source_slot.parsed.header.masters.clone(),
                own_index: (source_slot.parsed.header.masters.len() & 0xFF) as u8,
                record,
            });
        }

        let target_masters = {
            let target_slot = store.get(&target_handle_id).ok_or_else(|| {
                PyKeyError::new_err(format!("unknown plugin handle: {target_handle_id}"))
            })?;
            target_slot.parsed.header.masters.clone()
        };
        let Some(mut merged) = merge_conflict_record_chain(&signature, &sources, &target_masters)?
        else {
            return Ok(false);
        };
        merged.raw_payload = None;
        merged.parse_error = None;

        let target_slot = store.get_mut(&target_handle_id).ok_or_else(|| {
            PyKeyError::new_err(format!("unknown plugin handle: {target_handle_id}"))
        })?;
        let inserted_form_id = merged.form_id;
        replace_parsed_record_in_slot(target_slot, merged);
        target_slot
            .sections
            .apply_effect(&WriteEffect::RecordsAddedOrRemoved);
        Ok(record_exists_in_items(
            &target_slot.parsed.root_items,
            signature.as_str(),
            inserted_form_id,
        ))
    })
}

#[pyfunction(name = "plugin_handle_undelete_and_disable_refs")]
pub fn plugin_handle_undelete_and_disable_refs_native(
    py: Python<'_>,
    handle_id: u64,
    signatures: Vec<String>,
) -> PyResult<Vec<u32>> {
    py.detach(move || {
        let signature_set: HashSet<SmolStr> = signatures
            .into_iter()
            .map(|value| SmolStr::new(value.as_str()))
            .collect();
        if signature_set.is_empty() {
            return Ok(Vec::new());
        }
        let mut store = plugin_handle_store().lock().unwrap();
        let slot = store
            .get_mut(&handle_id)
            .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
        let own_index = (slot.parsed.header.masters.len() & 0xFF) as u8;
        let mut changed = Vec::new();
        undelete_and_disable_refs_in_items(
            &mut slot.parsed.root_items,
            own_index,
            &signature_set,
            &mut changed,
        );
        if !changed.is_empty() {
            slot.sections
                .apply_effect(&WriteEffect::RecordsAddedOrRemoved);
        }
        Ok(changed)
    })
}

#[pyfunction(name = "plugin_handle_resolve_string")]
#[pyo3(signature = (handle_id, string_id, language=None))]
pub fn plugin_handle_resolve_string_native(
    handle_id: u64,
    string_id: u32,
    language: Option<&str>,
) -> PyResult<Option<String>> {
    let store = plugin_handle_store().lock().unwrap();
    let slot = store
        .get(&handle_id)
        .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
    Ok(resolve_localized_string_with_language(
        &slot.strings,
        string_id,
        language,
    ))
}

#[pyfunction(name = "plugin_handle_resolve_string_values")]
pub fn plugin_handle_resolve_string_values_native(
    py: Python<'_>,
    handle_id: u64,
    string_id: u32,
) -> PyResult<Vec<(String, String)>> {
    py.detach(move || {
        let store = plugin_handle_store().lock().unwrap();
        let slot = store
            .get(&handle_id)
            .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
        Ok::<_, PyErr>(
            slot.strings
                .by_language
                .iter()
                .filter_map(|(language, table)| {
                    table
                        .get(&string_id)
                        .map(|value| (language.clone(), value.clone()))
                })
                .collect::<Vec<_>>(),
        )
    })
}

#[pyfunction(name = "plugin_handle_set_localized_strings")]
#[pyo3(signature = (handle_id, values, language=None, table_types=None))]
pub fn plugin_handle_set_localized_strings_native(
    handle_id: u64,
    values: HashMap<u32, String>,
    language: Option<&str>,
    table_types: Option<HashMap<u32, String>>,
) -> PyResult<()> {
    let mut store = plugin_handle_store().lock().unwrap();
    let slot = store
        .get_mut(&handle_id)
        .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
    rehydrate_filtered_strings_for_authoring(slot);
    slot.strings.materialize_all();
    let language = strings::language_code(language);
    slot.strings.default_language = language.clone();
    slot.strings.by_language.insert(language, values);
    if let Some(table_types) = table_types {
        for (string_id, table_type) in table_types {
            slot.strings.table_types.insert(string_id, table_type);
        }
    }
    slot.localized_text_index = None;
    Ok(())
}

#[pyfunction(name = "plugin_handle_set_localized_strings_by_language")]
#[pyo3(signature = (handle_id, values_by_language, preferred_language=None, table_types=None))]
pub fn plugin_handle_set_localized_strings_by_language_native(
    handle_id: u64,
    values_by_language: HashMap<String, HashMap<u32, String>>,
    preferred_language: Option<&str>,
    table_types: Option<HashMap<u32, String>>,
) -> PyResult<()> {
    let mut tables = HashMap::new();
    for (language, values) in values_by_language {
        tables.insert(strings::language_code(Some(language.as_str())), values);
    }
    let mut store = plugin_handle_store().lock().unwrap();
    let slot = store
        .get_mut(&handle_id)
        .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
    rehydrate_filtered_strings_for_authoring(slot);
    // The caller supplies the whole corpus, so the index has nothing to add.
    slot.strings.lazy_tables = None;
    slot.strings.by_language = tables;
    if let Some(table_types) = table_types {
        slot.strings.table_types = table_types;
    }
    if let Some(language) = preferred_language {
        slot.strings.default_language = strings::language_code(Some(language));
    } else if !slot
        .strings
        .by_language
        .contains_key(slot.strings.default_language.as_str())
    {
        slot.strings.default_language = if slot.strings.by_language.contains_key("en") {
            "en".to_string()
        } else {
            let mut languages: Vec<&String> = slot.strings.by_language.keys().collect();
            languages.sort();
            languages
                .first()
                .map(|value| (*value).clone())
                .unwrap_or_default()
        };
    }
    slot.localized_text_index = None;
    Ok(())
}

#[pyfunction(name = "plugin_handle_set_localized_field_values")]
#[pyo3(signature = (handle_id, string_id, values_by_language, preferred_language=None, table_type=None))]
pub fn plugin_handle_set_localized_field_values_native(
    handle_id: u64,
    string_id: u32,
    values_by_language: HashMap<String, String>,
    preferred_language: Option<&str>,
    table_type: Option<&str>,
) -> PyResult<()> {
    let mut store = plugin_handle_store().lock().unwrap();
    let slot = store
        .get_mut(&handle_id)
        .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
    rehydrate_filtered_strings_for_authoring(slot);
    slot.strings.materialize_all();
    for (language, text) in values_by_language {
        let language = strings::language_code(Some(language.as_str()));
        slot.strings
            .by_language
            .entry(language)
            .or_default()
            .insert(string_id, text);
    }
    slot.strings.default_language = strings::language_code(preferred_language);
    if let Some(table_type) = table_type {
        slot.strings
            .table_types
            .insert(string_id, table_type.to_string());
    }
    slot.localized_text_index = None;
    Ok(())
}

#[pyfunction(name = "plugin_handle_allocate_localized_string_id")]
#[pyo3(signature = (handle_id, preferred_start=1))]
pub fn plugin_handle_allocate_localized_string_id_native(
    handle_id: u64,
    preferred_start: u32,
) -> PyResult<u32> {
    let store = plugin_handle_store().lock().unwrap();
    let slot = store
        .get(&handle_id)
        .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
    let mut used: HashSet<u32> = slot.strings.table_types.keys().copied().collect();
    for table in slot.strings.by_language.values() {
        used.extend(table.keys().copied());
    }
    let mut candidate = preferred_start;
    while used.contains(&candidate) {
        candidate = candidate.saturating_add(1);
        if candidate == u32::MAX {
            return Err(value_error("no localized string IDs are available"));
        }
    }
    Ok(candidate)
}

#[pyfunction(name = "plugin_handle_save_localized_strings")]
#[pyo3(signature = (handle_id, plugin_path=None))]
pub fn plugin_handle_save_localized_strings_native(
    py: Python<'_>,
    handle_id: u64,
    plugin_path: Option<&str>,
) -> PyResult<Vec<String>> {
    let (target, parsed, strings) = {
        let store = plugin_handle_store().lock().unwrap();
        let slot = store
            .get(&handle_id)
            .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
        if (slot.parsed.header.flags & TES4_FLAG_LOCALIZED) == 0 {
            return Ok(Vec::new());
        }
        let target = plugin_path
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| slot.parsed.file_path.clone());
        (target, slot.parsed.clone(), slot.strings.clone())
    };
    if target.trim().is_empty() {
        return Err(value_error("No output path specified"));
    }
    py.detach(move || save_localized_strings_snapshot(&parsed, &strings, target.as_str()))
}

fn save_localized_strings_snapshot(
    parsed: &ParsedPlugin,
    strings: &LocalizedStringsState,
    target: &str,
) -> PyResult<Vec<String>> {
    write_localized_strings_for_parsed(parsed, strings, target).map(|written| {
        written
            .into_iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect()
    })
}

/// [`resolve_record`] with the form-id index materialized first.
///
/// A full handle already holds the entire record tree, so an index beside it is
/// cheap and turns every read from a linear walk of the whole plugin into a hash
/// lookup — without it a single read of SeventySix.esm costs ~200 ms. Lazy
/// handles skip it: they have no tree to index, and their own offset store
/// already answers by form id.
pub(crate) fn resolve_record_indexed(
    slot: &mut NativePluginSlot,
    raw_form_id: u32,
) -> Option<Cow<'_, ParsedRecord>> {
    if slot.lazy.is_none() {
        ensure_records_section(slot);
    }
    resolve_record(slot, raw_form_id)
}

/// Resolve a record by raw form id for read-only paths.
///
/// Tries, in order: an already-built index section, the lazy record store, then
/// a tree walk. A lazy handle's `root_items` is empty, so only the store can
/// serve it.
/// Never forces an index section to be built; callers that want one go through
/// [`resolve_record_indexed`].
pub(crate) fn resolve_record(
    slot: &NativePluginSlot,
    raw_form_id: u32,
) -> Option<Cow<'_, ParsedRecord>> {
    if let Some(records) = slot.sections.records.as_ref() {
        if let Some(record) = records.record(&slot.parsed, raw_form_id) {
            return Some(Cow::Borrowed(record));
        }
    }
    if let Some(record) = slot.lazy_record(raw_form_id) {
        return Some(Cow::Owned(record));
    }
    let mut exact = |record: &ParsedRecord| record.form_id == raw_form_id;
    if let Some(record) = find_first_record(&slot.parsed.root_items, &mut exact) {
        return Some(Cow::Borrowed(record));
    }
    resolve_record_by_object_id(slot, raw_form_id)
}

/// Fallback used when an exact form id misses: match on the low 24 bits and
/// prefer a candidate owned by this plugin.
///
/// The lazy branch ties on lowest file offset so it selects the same record the
/// tree walk would. `offsets` is a hash map, so without that tie-break the two
/// handle kinds would disagree whenever two masters share an object id.
fn resolve_record_by_object_id(
    slot: &NativePluginSlot,
    raw_form_id: u32,
) -> Option<Cow<'_, ParsedRecord>> {
    let object_id = raw_form_id & 0x00FF_FFFF;
    let own_index = (slot.parsed.header.masters.len() & 0xFF) as u8;

    if let Some(lazy) = slot.lazy.as_ref() {
        // A plugin-local id (`000800`) is written with this plugin's own master
        // index, so the exact-match attempt above always misses it. Try that one
        // id by early-exit before resorting to indexing every record.
        let local_form_id = ((own_index as u32) << 24) | object_id;
        if local_form_id != raw_form_id {
            if let Some(record) = slot.lazy_record(local_form_id) {
                return Some(Cow::Owned(record));
            }
        }
        let mut selected: Option<(u32, usize)> = None;
        for (&form_id, &offset) in lazy.offsets() {
            if form_id & 0x00FF_FFFF != object_id {
                continue;
            }
            selected = Some(match selected {
                None => (form_id, offset),
                Some((existing_id, existing_offset)) => {
                    if prefer_object_id_lookup_form_id(form_id, existing_id, own_index) {
                        (form_id, offset)
                    } else if prefer_object_id_lookup_form_id(existing_id, form_id, own_index) {
                        (existing_id, existing_offset)
                    } else if offset < existing_offset {
                        (form_id, offset)
                    } else {
                        (existing_id, existing_offset)
                    }
                }
            });
        }
        return slot.lazy_record(selected?.0).map(Cow::Owned);
    }

    let mut matches = Vec::new();
    let mut predicate = |record: &ParsedRecord| (record.form_id & 0x00FF_FFFF) == object_id;
    collect_records(&slot.parsed.root_items, &mut predicate, &mut matches);
    let mut selected: Option<&ParsedRecord> = None;
    for candidate in matches {
        match selected {
            None => selected = Some(candidate),
            Some(existing) if prefer_object_id_lookup_candidate(candidate, existing, own_index) => {
                selected = Some(candidate)
            }
            _ => {}
        }
    }
    selected.map(Cow::Borrowed)
}

/// Form-id-only form of [`prefer_object_id_lookup_candidate`], for the lazy
/// branch where no `ParsedRecord` has been materialized yet.
fn prefer_object_id_lookup_form_id(candidate: u32, existing: u32, own_index: u8) -> bool {
    let candidate_index = ((candidate >> 24) & 0xFF) as u8;
    let existing_index = ((existing >> 24) & 0xFF) as u8;
    let candidate_is_own = candidate_index == LOCAL_FORM_INDEX || candidate_index == own_index;
    let existing_is_own = existing_index == LOCAL_FORM_INDEX || existing_index == own_index;
    candidate_is_own && !existing_is_own
}

#[pyfunction(name = "plugin_handle_record_context_for_form_id")]
pub fn plugin_handle_record_context_for_form_id_native(
    _py: Python<'_>,
    handle_id: u64,
    form_id: u32,
) -> PyResult<Option<RecordContextPayload>> {
    let mut store = plugin_handle_store().lock().unwrap();
    let slot = store
        .get_mut(&handle_id)
        .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
    Ok(resolve_record_indexed(slot, form_id).map(|record| record_context_from_parsed(record.as_ref())))
}

#[pyfunction(name = "plugin_handle_addon_node_summaries_by_index_id")]
pub fn plugin_handle_addon_node_summaries_by_index_id_native(
    py: Python<'_>,
    handle_id: u64,
    index_id: u32,
) -> PyResult<Vec<(u32, String, Option<String>)>> {
    py.detach(move || {
        let store = plugin_handle_store().lock().unwrap();
        let slot = store
            .get(&handle_id)
            .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
        let mut records = Vec::new();
        let mut predicate = |record: &ParsedRecord| {
            if record.signature != "ADDN" {
                return false;
            }
            record.subrecords.iter().any(|subrecord| {
                subrecord.signature == "DATA"
                    && subrecord.data.len() >= 4
                    && u32::from_le_bytes([
                        subrecord.data[0],
                        subrecord.data[1],
                        subrecord.data[2],
                        subrecord.data[3],
                    ]) == index_id
            })
        };
        collect_records(&slot.parsed.root_items, &mut predicate, &mut records);
        Ok(records.into_iter().map(record_summary_tuple).collect())
    })
}

#[pyfunction(name = "plugin_handle_get_referenced_form_ids")]
pub fn plugin_handle_get_referenced_form_ids_native(
    py: Python<'_>,
    handle_id: u64,
    form_id: u32,
) -> PyResult<Vec<u32>> {
    py.detach(move || {
        let mut store = plugin_handle_store().lock().unwrap();
        let slot = store
            .get_mut(&handle_id)
            .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
        let Some(source_object_id) = coerce_local_object_id(&slot.parsed, form_id) else {
            return Ok(Vec::new());
        };
        if source_object_id == 0 {
            return Ok(Vec::new());
        }
        let core = ensure_core_section(slot);
        let Some(source_form_key) = form_key_for_object_id(&slot.parsed, &core, source_object_id)
        else {
            return Ok(Vec::new());
        };
        let refs = ensure_refs_section(slot);
        Ok(refs
            .forward_refs_by_form_key
            .get(&source_form_key)
            .map(|values| form_keys_to_local_object_ids(&slot.parsed, values))
            .unwrap_or_default())
    })
}

#[pyfunction(name = "plugin_handle_get_referenced_form_keys")]
pub fn plugin_handle_get_referenced_form_keys_native(
    py: Python<'_>,
    handle_id: u64,
    form_key: &str,
) -> PyResult<Vec<String>> {
    let form_key = form_key.to_string();
    py.detach(move || {
        let mut store = plugin_handle_store().lock().unwrap();
        let slot = store
            .get_mut(&handle_id)
            .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
        let refs = ensure_refs_section(slot);
        Ok(
            form_key_refs(&refs.forward_refs_by_form_key, form_key.as_str())
                .map(|values| values.iter().map(|value| value.render()).collect())
                .unwrap_or_default(),
        )
    })
}

#[pyfunction(name = "plugin_handle_get_referenced_form_keys_by_subrecord")]
pub fn plugin_handle_get_referenced_form_keys_by_subrecord_native(
    py: Python<'_>,
    handle_id: u64,
    form_key: &str,
    subrecord_sig: &str,
) -> PyResult<Vec<String>> {
    let form_key = form_key.to_string();
    let subrecord_sig = SmolStr::new(subrecord_sig);
    py.detach(move || {
        let mut store = plugin_handle_store().lock().unwrap();
        let slot = store
            .get_mut(&handle_id)
            .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
        let refs = ensure_refs_section(slot);
        let Some(normalized) = normalize_form_key(&form_key) else {
            return Ok(Vec::new());
        };
        Ok(refs
            .refs_by_form_key_and_subrecord
            .get(&(normalized, subrecord_sig))
            .map(|values| values.iter().map(|value| value.render()).collect())
            .unwrap_or_default())
    })
}

#[pyfunction(name = "plugin_handle_get_referencing_form_ids")]
pub fn plugin_handle_get_referencing_form_ids_native(
    py: Python<'_>,
    handle_id: u64,
    form_id: u32,
) -> PyResult<Vec<u32>> {
    py.detach(move || {
        let mut store = plugin_handle_store().lock().unwrap();
        let slot = store
            .get_mut(&handle_id)
            .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
        let Some(target_object_id) = coerce_local_object_id(&slot.parsed, form_id) else {
            return Ok(Vec::new());
        };
        if target_object_id == 0 {
            return Ok(Vec::new());
        }
        let core = ensure_core_section(slot);
        let Some(target_form_key) = form_key_for_object_id(&slot.parsed, &core, target_object_id)
        else {
            return Ok(Vec::new());
        };
        let refs = ensure_refs_section(slot);
        Ok(refs
            .reverse_refs_by_form_key
            .get(&target_form_key)
            .map(|values| form_keys_to_local_object_ids(&slot.parsed, values))
            .unwrap_or_default())
    })
}

#[pyfunction(name = "plugin_handle_get_referencing_form_keys")]
pub fn plugin_handle_get_referencing_form_keys_native(
    py: Python<'_>,
    handle_id: u64,
    form_key: &str,
) -> PyResult<Vec<String>> {
    let form_key = form_key.to_string();
    py.detach(move || {
        let mut store = plugin_handle_store().lock().unwrap();
        let slot = store
            .get_mut(&handle_id)
            .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
        let refs = ensure_refs_section(slot);
        Ok(
            form_key_refs(&refs.reverse_refs_by_form_key, form_key.as_str())
                .map(|values| values.iter().map(|value| value.render()).collect())
                .unwrap_or_default(),
        )
    })
}

#[pyfunction(name = "plugin_handle_index_stats")]
pub fn plugin_handle_index_stats_native(py: Python<'_>, handle_id: u64) -> PyResult<Py<PyAny>> {
    let stats = py.detach(move || {
        let mut store = plugin_handle_store().lock().unwrap();
        let slot = store
            .get_mut(&handle_id)
            .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
        let core = ensure_core_section(slot);
        let refs = ensure_refs_section(slot);
        let assets = ensure_assets_section(slot);
        let mut stats = HashMap::new();
        stats.insert("record_count".to_string(), core.by_form_key.len());
        stats.insert(
            "object_id_count".to_string(),
            core.form_ids_by_object_id.len(),
        );
        stats.insert("editor_id_count".to_string(), core.by_eid_lower.len());
        stats.insert(
            "signature_count".to_string(),
            core.by_signature_form_keys.len(),
        );
        stats.insert("form_key_count".to_string(), core.by_form_key.len());
        stats.insert(
            "outbound_source_count".to_string(),
            refs.forward_refs_by_form_key.len(),
        );
        stats.insert(
            "inbound_target_count".to_string(),
            refs.reverse_refs_by_form_key.len(),
        );
        stats.insert("asset_kind_count".to_string(), assets.assets_by_kind.len());
        Ok::<_, PyErr>(stats)
    })?;
    stats.into_py_any(py)
}

#[pyfunction(name = "plugin_handle_get_form_id_chain")]
pub fn plugin_handle_get_form_id_chain_native(
    py: Python<'_>,
    handle_id: u64,
    form_id: u32,
) -> PyResult<Vec<u32>> {
    py.detach(move || {
        let mut store = plugin_handle_store().lock().unwrap();
        let slot = store
            .get_mut(&handle_id)
            .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
        let Some(start_object_id) = coerce_local_object_id(&slot.parsed, form_id) else {
            return Ok(Vec::new());
        };
        if start_object_id == 0 {
            return Ok(Vec::new());
        }
        let core = ensure_core_section(slot);
        if !core.form_ids_by_object_id.contains_key(&start_object_id) {
            return Ok(Vec::new());
        }
        let refs = ensure_refs_section(slot);
        let mut visited = HashSet::from([start_object_id]);
        let mut ordered = vec![start_object_id];
        let mut pending = std::collections::VecDeque::from([start_object_id]);
        while let Some(current_object_id) = pending.pop_front() {
            let Some(current_form_key) =
                form_key_for_object_id(&slot.parsed, &core, current_object_id)
            else {
                continue;
            };
            let mut neighbors = Vec::new();
            if let Some(values) = refs.forward_refs_by_form_key.get(&current_form_key) {
                neighbors.extend(form_keys_to_local_object_ids(&slot.parsed, values));
            }
            if let Some(values) = refs.reverse_refs_by_form_key.get(&current_form_key) {
                neighbors.extend(form_keys_to_local_object_ids(&slot.parsed, values));
            }
            for neighbor_object_id in neighbors {
                if visited.insert(neighbor_object_id) {
                    ordered.push(neighbor_object_id);
                    pending.push_back(neighbor_object_id);
                }
            }
        }
        Ok(ordered)
    })
}

#[pyfunction(name = "plugin_handle_export_plugin_text")]
#[pyo3(signature = (handle_id, mode="lossless", format="json"))]
pub fn plugin_handle_export_plugin_text_native(
    py: Python<'_>,
    handle_id: u64,
    mode: &str,
    format: &str,
) -> PyResult<String> {
    let mode = mode.to_string();
    let (parsed, strings) = if mode.eq_ignore_ascii_case("authoring") {
        py.detach(move || clone_plugin_handle_state_for_authoring(handle_id))?
    } else {
        py.detach(move || clone_plugin_handle_state(handle_id))?
    };
    let format = format.to_string();
    py.detach(move || {
        dump_text_payload_value(
            export_text_payload_value_from_parsed(&parsed, &strings, mode.as_str())?,
            format.as_str(),
        )
    })
}

#[pyfunction(name = "plugin_handle_export_record_text")]
#[pyo3(signature = (handle_id, form_id, format="json"))]
pub fn plugin_handle_export_record_text_native(
    py: Python<'_>,
    handle_id: u64,
    form_id: u32,
    format: &str,
) -> PyResult<String> {
    let format = format.to_string();
    // Resolve and serialize under one lock, borrowing the slot rather than
    // copying it. Cloning the plugin cost seconds per read; cloning just the
    // localized string table still cost ~200 ms on a localized master, because
    // SeventySix.esm's tables hold roughly a million entries.
    py.detach(move || {
        let raw_form_id = form_id & 0xFFFF_FFFF;
        let mut store = plugin_handle_store().lock().unwrap();
        let slot = store
            .get_mut(&handle_id)
            .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
        rehydrate_filtered_strings_for_authoring(slot);
        let record = resolve_record_indexed(slot, raw_form_id)
            .map(|record| record.into_owned())
            .ok_or_else(|| {
                PyKeyError::new_err(format!("unknown record form_id: {raw_form_id:08X}"))
            })?;
        dump_text_payload_value(
            serialize_record_payload_text_value(
                &slot.parsed,
                &slot.strings,
                &record,
                "authoring",
                false,
            )?,
            format.as_str(),
        )
    })
}

#[pyfunction(name = "plugin_handle_extract_dialogue_text")]
#[pyo3(signature = (handle_id, format="json"))]
pub fn plugin_handle_extract_dialogue_text_native(
    py: Python<'_>,
    handle_id: u64,
    format: &str,
) -> PyResult<String> {
    let (parsed, strings) = py.detach(move || clone_plugin_handle_state(handle_id))?;
    let format = format.to_string();
    py.detach(move || {
        dump_text_payload_value(
            extract_dialogue_payload_value(&parsed, &strings)?,
            format.as_str(),
        )
    })
}

#[pyfunction(name = "plugin_handle_export_authoring_dir")]
#[pyo3(signature = (handle_id, out_dir, format=None, jobs=None))]
pub fn plugin_handle_export_authoring_dir_native(
    py: Python<'_>,
    handle_id: u64,
    out_dir: &str,
    format: Option<&str>,
    jobs: Option<usize>,
) -> PyResult<()> {
    let fmt = match format.map(|f| f.trim().to_ascii_lowercase()).as_deref() {
        Some("yaml") | Some("yml") => "yaml",
        _ => "json",
    };
    let mut store = plugin_handle_store().lock().unwrap();
    let slot = store
        .get_mut(&handle_id)
        .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
    rehydrate_filtered_strings_for_authoring(slot);
    slot.strings.materialize_all();
    let record_count = match slot.record_count_cache {
        Some(value) => value,
        None => {
            let value = count_records(&slot.parsed.root_items);
            slot.record_count_cache = Some(value);
            value
        }
    };
    let jobs = jobs.unwrap_or(crate::default_job_count());
    let skip_set = std::collections::HashSet::new();
    if jobs > 1 {
        export_authoring_dir_parallel(
            py,
            &slot.parsed,
            &slot.strings,
            record_count,
            Path::new(out_dir),
            fmt,
            jobs,
            &skip_set,
        )
    } else {
        export_authoring_dir_from_parsed(
            py,
            &slot.parsed,
            &slot.strings,
            record_count,
            Path::new(out_dir),
            fmt,
            &skip_set,
        )
    }
}

#[pyfunction(name = "plugin_handle_import_text")]
#[pyo3(signature = (text, format = "json", game = None))]
pub fn plugin_handle_import_text_native(
    py: Python<'_>,
    text: &str,
    format: &str,
    game: Option<&str>,
) -> PyResult<u64> {
    let text = text.to_string();
    let format = format.to_string();
    let game_owned = game.map(str::to_string);
    let lossless = py.detach({
        let text = text.clone();
        let format = format.clone();
        let game_owned = game_owned.clone();
        move || {
            if let Some((mut parsed, strings)) =
                import_text_payload_lossless_native(text.as_str(), format.as_str())?
            {
                if let Some(game_id) = game_owned {
                    parsed.game = Some(game_id);
                }
                return Ok::<_, PyErr>(Some((parsed, strings)));
            }
            Ok(None)
        }
    })?;
    let (mut parsed, strings) = match lossless {
        Some(result) => result,
        None => {
            let text = text.clone();
            let format = format.clone();
            py.detach(move || import_text_payload_compact_native(text.as_str(), format.as_str()))?
        }
    };
    if let Some(game_id) = game {
        parsed.game = Some(game_id.to_string());
    }
    if parsed.plugin_name.is_empty() {
        parsed.plugin_name = "Plugin.esp".to_string();
    }
    Ok(insert_plugin_handle(parsed, strings))
}

// Python uses integer handle IDs through the plugin_handle_* pyfunctions above.

fn count_records(items: &[ParsedItem]) -> usize {
    let mut n = 0;
    for item in items {
        match item {
            ParsedItem::Record(_) => n += 1,
            ParsedItem::Group(group) => n += count_records(&group.children),
        }
    }
    n
}

// HEDR.NumRecords stores "records and groups (excluding TES4)" per UESP spec.
// Use this for HEDR writes; use `count_records` for user-facing record counts.
fn count_hedr_entries(items: &[ParsedItem]) -> usize {
    let mut n = 0;
    for item in items {
        match item {
            ParsedItem::Record(_) => n += 1,
            ParsedItem::Group(group) => {
                n += 1;
                n += count_hedr_entries(&group.children);
            }
        }
    }
    n
}

// ---------------------------------------------------------------------------
// Write-model helpers
// ---------------------------------------------------------------------------

/// Locate a record by raw form_id in `items` (recursing into groups) and
/// return a mutable reference to it. Returns `None` if not found.
fn find_record_mut(items: &mut Vec<ParsedItem>, form_id: u32) -> Option<&mut ParsedRecord> {
    let target = form_id & 0xFFFF_FFFF;
    for item in items.iter_mut() {
        match item {
            ParsedItem::Record(r) if (r.form_id & 0xFFFF_FFFF) == target => {
                return Some(r);
            }
            ParsedItem::Group(g) => {
                if let Some(r) = find_record_mut(&mut g.children, form_id) {
                    return Some(r);
                }
            }
            _ => {}
        }
    }
    None
}

/// In-place subrecord byte patch: locate `form_key_str` in `handle_id`, find
/// the first subrecord with signature `sig`, and call `f` with a mutable copy
/// of its bytes (`Bytes` is refcounted, so it can't be mutated in place). If
/// `f` returns true the bytes are written back.
///
/// Skips the schema decode/encode round trip of `read_record` +
/// `replace_record_native`. Returns `f`'s result, or `Err` when the handle,
/// record, or subrecord can't be located.
pub fn patch_record_subrecord_bytes<F>(
    handle_id: u64,
    form_key_str: &str,
    sig: &str,
    f: F,
) -> Result<bool, String>
where
    F: FnOnce(&mut [u8]) -> bool,
{
    let mut store = plugin_handle_store_ref().lock().unwrap();
    let slot = store
        .get_mut(&handle_id)
        .ok_or_else(|| format!("unknown plugin handle: {handle_id}"))?;

    // Resolve form_key string → raw form_id via the core section index.
    let raw_form_id = {
        let core = ensure_core_section(slot);
        let entry = record_index_entry_by_form_key(&core, form_key_str)
            .ok_or_else(|| format!("record not found: {form_key_str}"))?;
        entry.raw_form_id
    };

    // Walk the parsed tree to the record itself.
    let record = find_record_mut(&mut slot.parsed.root_items, raw_form_id)
        .ok_or_else(|| {
            format!(
                "record present in index but absent from parsed tree: {form_key_str} (0x{raw_form_id:08X})"
            )
        })?;

    let old_edid = record_editor_id_value(record);
    let subrec = record
        .subrecords
        .iter_mut()
        .find(|sr| sr.signature.as_str() == sig)
        .ok_or_else(|| format!("subrecord {sig} not in {form_key_str}"))?;

    let mut buf: Vec<u8> = subrec.data.to_vec();
    let changed = f(&mut buf);
    if changed {
        subrec.data = Bytes::from(buf);
        let new_edid = record_editor_id_value(record);
        slot.record_count_cache = None;
        if old_edid != new_edid {
            slot.sections
                .apply_effect(&WriteEffect::RecordsAddedOrRemoved);
        } else {
            slot.sections.apply_effect(&WriteEffect::RecordContents {
                form_ids: smallvec::smallvec![raw_form_id],
            });
        }
    }
    Ok(changed)
}

fn reference_object_targets(
    value: &JsonValue,
    own_plugin: &str,
    target_object_ids: &HashSet<u32>,
) -> bool {
    let Some(reference) = value
        .as_object()
        .and_then(|object| object.get("reference"))
        .and_then(JsonValue::as_object)
    else {
        return false;
    };
    let object_id = match reference.get("object_id") {
        Some(JsonValue::String(value)) => u32::from_str_radix(value, 16).ok(),
        Some(JsonValue::Number(value)) => u32::from_str_radix(value.to_string().as_str(), 16).ok(),
        _ => None,
    };
    let Some(object_id) = object_id.map(|value| value & 0x00FF_FFFF) else {
        return false;
    };
    if !target_object_ids.contains(&object_id) {
        return false;
    }
    match reference.get("plugin") {
        None | Some(JsonValue::Null) => true,
        Some(JsonValue::String(plugin)) => plugin.eq_ignore_ascii_case(own_plugin),
        Some(plugin) => plugin
            .as_str()
            .is_some_and(|plugin| plugin.eq_ignore_ascii_case(own_plugin)),
    }
}

fn contains_target_reference(
    value: &JsonValue,
    own_plugin: &str,
    target_object_ids: &HashSet<u32>,
) -> bool {
    if reference_object_targets(value, own_plugin, target_object_ids) {
        return true;
    }
    match value {
        JsonValue::Array(values) => values
            .iter()
            .any(|value| contains_target_reference(value, own_plugin, target_object_ids)),
        JsonValue::Object(values) => values
            .values()
            .any(|value| contains_target_reference(value, own_plugin, target_object_ids)),
        _ => false,
    }
}

fn strip_target_references_from_value(
    value: &JsonValue,
    own_plugin: &str,
    target_object_ids: &HashSet<u32>,
) -> (JsonValue, usize) {
    match value {
        JsonValue::Array(values) => {
            let mut rewritten = Vec::with_capacity(values.len());
            let mut removed = 0;
            for value in values {
                let direct_reference = value
                    .as_object()
                    .and_then(|object| object.get("reference"))
                    .and_then(JsonValue::as_object)
                    .is_some();
                if reference_object_targets(value, own_plugin, target_object_ids)
                    || (!direct_reference
                        && value.is_object()
                        && contains_target_reference(value, own_plugin, target_object_ids))
                {
                    removed += 1;
                    continue;
                }
                let (value, nested_removed) =
                    strip_target_references_from_value(value, own_plugin, target_object_ids);
                removed += nested_removed;
                rewritten.push(value);
            }
            (JsonValue::Array(rewritten), removed)
        }
        JsonValue::Object(values) => {
            if values
                .get("reference")
                .and_then(JsonValue::as_object)
                .is_some()
            {
                return if reference_object_targets(value, own_plugin, target_object_ids) {
                    (JsonValue::Null, 1)
                } else {
                    (value.clone(), 0)
                };
            }
            let mut rewritten = JsonMap::new();
            let mut removed = 0;
            for (key, value) in values {
                let (value, nested_removed) =
                    strip_target_references_from_value(value, own_plugin, target_object_ids);
                removed += nested_removed;
                rewritten.insert(key.clone(), value);
            }
            (JsonValue::Object(rewritten), removed)
        }
        _ => (value.clone(), 0),
    }
}

fn set_first_authoring_count(fields: &mut [JsonValue], label: &str, count: usize) {
    for field in fields {
        let Some(object) = field.as_object_mut() else {
            continue;
        };
        if object.len() == 1 && object.contains_key(label) {
            object.insert(label.to_string(), JsonValue::Number((count as u64).into()));
            return;
        }
    }
}

fn strip_target_references(
    fields: &mut Vec<JsonValue>,
    own_plugin: &str,
    target_object_ids: &HashSet<u32>,
) -> usize {
    let mut rewritten = Vec::with_capacity(fields.len());
    let mut removed_total = 0;
    let mut dropped_labels = HashSet::new();
    for field in fields.drain(..) {
        let Some(object) = field.as_object() else {
            rewritten.push(field);
            continue;
        };
        if object.len() != 1 {
            rewritten.push(field);
            continue;
        }
        let (label, value) = object.iter().next().expect("single field entry");
        let is_drop_entry = matches!(label.as_str(), "LVLO" | "LeveledEntry" | "CNTO")
            && value.is_object()
            && value
                .as_object()
                .and_then(|value| value.get("reference"))
                .and_then(JsonValue::as_object)
                .is_none()
            && contains_target_reference(value, own_plugin, target_object_ids);
        if is_drop_entry {
            removed_total += 1;
            dropped_labels.insert(label.clone());
            continue;
        }
        let (value, removed) =
            strip_target_references_from_value(value, own_plugin, target_object_ids);
        removed_total += removed;
        rewritten.push(JsonValue::Object(JsonMap::from_iter([(
            label.clone(),
            value,
        )])));
    }

    if removed_total == 0 {
        *fields = rewritten;
        return 0;
    }
    for label in dropped_labels {
        let count = rewritten
            .iter()
            .filter(|field| {
                field
                    .as_object()
                    .is_some_and(|field| field.len() == 1 && field.contains_key(label.as_str()))
            })
            .count();
        set_first_authoring_count(&mut rewritten, "Count", count);
    }
    if let Some(keyword_count) = rewritten.iter().find_map(|field| {
        field
            .as_object()
            .and_then(|field| field.get("Keywords"))
            .and_then(JsonValue::as_array)
            .map(Vec::len)
    }) {
        set_first_authoring_count(&mut rewritten, "Keyword Count", keyword_count);
    }
    *fields = rewritten;
    removed_total
}

/// Remove every record whose form id is in `targets` in a single recursive
/// pass over the tree (O(total records)), versus one full walk per record.
fn remove_records_from_items(
    items: &mut Vec<ParsedItem>,
    targets: &std::collections::HashSet<u32>,
    removed: &mut usize,
) {
    for item in items.iter_mut() {
        if let ParsedItem::Group(group) = item {
            remove_records_from_items(&mut group.children, targets, removed);
        }
    }
    let before = items.len();
    items.retain(|item| match item {
        ParsedItem::Record(record) => !targets.contains(&(record.form_id & 0xFFFF_FFFF)),
        _ => true,
    });
    *removed += before - items.len();
}

fn remove_record_from_items(items: &mut Vec<ParsedItem>, form_id: u32) -> bool {
    let target = form_id & 0xFFFF_FFFF;
    let mut i = 0;
    while i < items.len() {
        match &items[i] {
            ParsedItem::Record(r) if (r.form_id & 0xFFFF_FFFF) == target => {
                items.remove(i);
                return true;
            }
            ParsedItem::Group(_) => {
                if let ParsedItem::Group(ref mut g) = items[i] {
                    if remove_record_from_items(&mut g.children, form_id) {
                        return true;
                    }
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    false
}

fn remove_first_records_from_items(items: &mut Vec<ParsedItem>, targets: &mut HashSet<u32>) {
    let mut index = 0;
    while index < items.len() && !targets.is_empty() {
        match &mut items[index] {
            ParsedItem::Record(record) if targets.remove(&record.form_id) => {
                items.remove(index);
            }
            ParsedItem::Group(group) => {
                remove_first_records_from_items(&mut group.children, targets);
                index += 1;
            }
            _ => index += 1,
        }
    }
}

fn record_exists_in_items(items: &[ParsedItem], signature: &str, form_id: u32) -> bool {
    let target = form_id & 0xFFFF_FFFF;
    for item in items {
        match item {
            ParsedItem::Record(record)
                if record.signature.as_str() == signature
                    && (record.form_id & 0xFFFF_FFFF) == target =>
            {
                return true;
            }
            ParsedItem::Group(group)
                if record_exists_in_items(&group.children, signature, form_id) =>
            {
                return true;
            }
            _ => {}
        }
    }
    false
}

/// Find a record of `signature` whose object-id (low 24 bits) matches that of
/// `form_id`, returning its FULL form_id (including the owning plugin's
/// master-index byte). Used to reconcile a caller that only knows the 24-bit
/// object-id (e.g. the conversion mapper's `target.local`) with records emitted
/// into the tree carrying the output plugin's own-index byte.
fn find_record_full_form_id_by_object_id(
    items: &[ParsedItem],
    signature: &str,
    form_id: u32,
) -> Option<u32> {
    let target_obj = form_id & 0x00FF_FFFF;
    for item in items {
        match item {
            ParsedItem::Record(record)
                if record.signature.as_str() == signature
                    && (record.form_id & 0x00FF_FFFF) == target_obj =>
            {
                return Some(record.form_id);
            }
            ParsedItem::Group(group) => {
                if let Some(found) =
                    find_record_full_form_id_by_object_id(&group.children, signature, form_id)
                {
                    return Some(found);
                }
            }
            _ => {}
        }
    }
    None
}

fn remove_cell_child_group_from_items(items: &mut Vec<ParsedItem>, cell_form_id: u32) -> bool {
    let target_label = cell_form_id.to_le_bytes();
    let mut removed = false;
    let mut i = 0;
    while i < items.len() {
        match &mut items[i] {
            ParsedItem::Group(group)
                if group.group_type == CELL_CHILD_GROUP && group.label == target_label =>
            {
                items.remove(i);
                removed = true;
            }
            ParsedItem::Group(group) => {
                if remove_cell_child_group_from_items(&mut group.children, cell_form_id) {
                    removed = true;
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    removed
}

/// Apply a content-only replacement to `record` in place, enforcing the
/// invariant that signature/flags/EditorID are unchanged (those drive core
/// indexes, so a mismatch must be rejected rather than silently corrupting the
/// index). Returns `true` when applied, `false` when rejected. Shared by the
/// single-record and batch replace paths so both enforce identical guards.
fn apply_record_contents_replacement(
    record: &mut ParsedRecord,
    replacement: &ParsedRecord,
) -> bool {
    // The COMPRESSED storage bit is set deterministically by the encoder for
    // CELL/LAND (force-compress); it is NOT a content identity. A decoded→re-encoded
    // CELL therefore differs from an uncompressed tree CELL only in this bit, which
    // must not block an in-place content swap (the record keeps its own flags). For
    // every other signature the encoder never sets this bit, so masking is a no-op.
    if record.signature != replacement.signature
        || (record.flags & !COMPRESSED_RECORD_FLAG) != (replacement.flags & !COMPRESSED_RECORD_FLAG)
        || record_editor_id_value(record) != record_editor_id_value(replacement)
    {
        return false;
    }
    record.version_control = replacement.version_control;
    record.form_version = replacement.form_version;
    record.version2 = replacement.version2;
    record.subrecords = replacement.subrecords.clone();
    record.raw_payload = None;
    record.parse_error = None;
    true
}

fn replace_record_contents_in_items(items: &mut [ParsedItem], replacement: &ParsedRecord) -> bool {
    for item in items.iter_mut() {
        match item {
            ParsedItem::Record(record) if record.form_id == replacement.form_id => {
                return apply_record_contents_replacement(record, replacement);
            }
            ParsedItem::Group(group) => {
                if replace_record_contents_in_items(&mut group.children, replacement) {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

/// Single-pass batch content-replace. Walks `items` ONCE, looking each visited
/// record up in `replacements` (form_id → replacement) and applying it in place
/// with the same guards as `replace_record_contents_in_items`. Applied entries
/// are REMOVED from `replacements`, so an empty map short-circuits remaining
/// sibling subtrees — total work is O(visited_nodes + applied), not
/// O(replacements × records) like calling the single-record path in a loop. A
/// replacement whose target record is missing, or rejected by the
/// signature/flags/EditorID guard, is simply left in `replacements` (the caller
/// reports it as not-applied). `visits` counts every node touched (test hook for
/// the linearity assertion).
fn replace_record_contents_in_items_batch(
    items: &mut [ParsedItem],
    replacements: &mut HashMap<u32, ParsedRecord>,
    applied: &mut Vec<u32>,
    visits: &mut usize,
) {
    for item in items.iter_mut() {
        if replacements.is_empty() {
            return;
        }
        *visits += 1;
        match item {
            ParsedItem::Record(record) => {
                if let Some(replacement) = replacements.get(&record.form_id) {
                    if apply_record_contents_replacement(record, replacement) {
                        let form_id = record.form_id;
                        applied.push(form_id);
                        replacements.remove(&form_id);
                    }
                }
            }
            ParsedItem::Group(group) => {
                replace_record_contents_in_items_batch(
                    &mut group.children,
                    replacements,
                    applied,
                    visits,
                );
            }
        }
    }
}

/// Replace the contents of many records in a single tree traversal. Returns the
/// form_ids actually applied (in tree order). Records present in `replacements`
/// but absent from the plugin, or rejected by the content guard, are omitted
/// from the result (mirrors the single-record path returning `false`).
pub fn replace_parsed_records_contents_in_slot_batch(
    slot: &mut NativePluginSlot,
    replacements: Vec<ParsedRecord>,
) -> smallvec::SmallVec<[u32; 4]> {
    let mut by_form_id: HashMap<u32, ParsedRecord> = HashMap::with_capacity(replacements.len());
    for replacement in replacements {
        // Last write wins on a duplicate form_id (matches the per-record loop,
        // which would have applied them in order ending on the last).
        by_form_id.insert(replacement.form_id, replacement);
    }
    let mut applied = Vec::with_capacity(by_form_id.len());
    let mut visits = 0usize;
    replace_record_contents_in_items_batch(
        &mut slot.parsed.root_items,
        &mut by_form_id,
        &mut applied,
        &mut visits,
    );
    if !applied.is_empty() {
        slot.sections.apply_effect(&WriteEffect::RecordContents {
            form_ids: applied.iter().copied().collect(),
        });
    }
    applied.into_iter().collect()
}

fn record_editor_id_value(record: &ParsedRecord) -> Option<String> {
    let subrecords = effective_subrecords_for_record(record);
    subrecords
        .iter()
        .find(|subrecord| subrecord.signature.as_str() == "EDID")
        .map(|subrecord| decode_cp1252(&subrecord.data))
}

fn normalize_editor_id_for_match(value: &str) -> String {
    value.trim_end_matches('\0').to_ascii_lowercase()
}

fn record_editor_id_matches(record: &ParsedRecord, editor_id: &str) -> bool {
    record_editor_id_value(record)
        .map(|value| normalize_editor_id_for_match(&value) == editor_id)
        .unwrap_or(false)
}

fn projected_world_children_group_mut(
    items: &mut [ParsedItem],
    world_form_id: u32,
) -> Option<&mut ParsedGroup> {
    let label = world_form_id.to_le_bytes();
    items.iter_mut().find_map(|item| match item {
        ParsedItem::Group(group) if group.group_type == 0 && group.label == *b"WRLD" => {
            group.children.iter_mut().find_map(|child| match child {
                ParsedItem::Group(child_group)
                    if child_group.group_type == 1 && child_group.label == label =>
                {
                    Some(child_group)
                }
                _ => None,
            })
        }
        _ => None,
    })
}

fn extract_projected_cell_matches_from_items(
    items: &mut Vec<ParsedItem>,
    world_dir: &str,
    wanted_editor_ids: &HashSet<String>,
    extracted: &mut HashMap<(String, String), ExtractedProjectedCell>,
) {
    let mut child_group_keys: HashMap<[u8; 4], (String, String)> = HashMap::new();
    let mut i = 0;
    while i < items.len() {
        let key = match &items[i] {
            ParsedItem::Record(record) if record.signature.as_str() == "CELL" => {
                record_editor_id_value(record)
                    .map(|editor_id| normalize_editor_id_for_match(&editor_id))
                    .filter(|editor_id| wanted_editor_ids.contains(editor_id))
                    .map(|editor_id| (world_dir.to_owned(), editor_id))
            }
            _ => None,
        };
        if let Some(key) = key {
            if let ParsedItem::Record(record) = items.remove(i) {
                child_group_keys.insert(record.form_id.to_le_bytes(), key.clone());
                let entry = extracted.entry(key).or_default();
                if entry.cell.is_none() {
                    entry.cell = Some(record);
                }
            }
        } else {
            i += 1;
        }
    }

    let mut i = 0;
    while i < items.len() {
        let key = match &items[i] {
            ParsedItem::Group(group) if group.group_type == CELL_CHILD_GROUP => {
                child_group_keys.get(&group.label).cloned()
            }
            _ => None,
        };
        if let Some(key) = key {
            if let ParsedItem::Group(group) = items.remove(i) {
                extracted.entry(key).or_default().child_groups.push(group);
            }
        } else {
            i += 1;
        }
    }

    for item in items {
        if let ParsedItem::Group(group) = item {
            extract_projected_cell_matches_from_items(
                &mut group.children,
                world_dir,
                wanted_editor_ids,
                extracted,
            );
        }
    }
}

fn apply_placed_record_position_offset_in_items(
    items: &mut [ParsedItem],
    offset: (f32, f32, f32),
) -> usize {
    let mut changed = 0usize;
    for item in items {
        match item {
            ParsedItem::Record(record) => {
                if apply_placed_record_position_offset_to_record(record, offset) {
                    changed += 1;
                }
            }
            ParsedItem::Group(group) => {
                changed +=
                    apply_placed_record_position_offset_in_items(&mut group.children, offset);
            }
        }
    }
    changed
}

fn sanitize_subrecord_payloads_in_items(
    items: &mut [ParsedItem],
    max_lengths: &HashMap<(SmolStr, SmolStr), usize>,
    row_projections: &HashMap<(SmolStr, SmolStr), (usize, usize)>,
    changed_form_ids: &mut smallvec::SmallVec<[u32; 4]>,
) -> usize {
    let mut changed = 0usize;
    for item in items {
        match item {
            ParsedItem::Record(record) => {
                let record_sig = record.signature.clone();
                let mut record_changed = false;
                for subrecord in &mut record.subrecords {
                    let key = (record_sig.clone(), subrecord.signature.clone());
                    if let Some(max_len) = max_lengths.get(&key) {
                        if subrecord.data.len() > *max_len {
                            subrecord.data = Bytes::copy_from_slice(&subrecord.data[..*max_len]);
                            changed += 1;
                            record_changed = true;
                        }
                    }
                    if let Some((source_row_len, target_row_len)) = row_projections.get(&key) {
                        if !subrecord.data.is_empty()
                            && *source_row_len > 0
                            && subrecord.data.len() % *source_row_len == 0
                        {
                            let mut projected = Vec::with_capacity(
                                (subrecord.data.len() / *source_row_len) * *target_row_len,
                            );
                            for row in subrecord.data.chunks(*source_row_len) {
                                projected.extend_from_slice(&row[..*target_row_len]);
                            }
                            if projected.len() != subrecord.data.len() {
                                subrecord.data = Bytes::from(projected);
                                changed += 1;
                                record_changed = true;
                            }
                        }
                    }
                }
                if record_changed {
                    changed_form_ids.push(record.form_id);
                }
            }
            ParsedItem::Group(group) => {
                changed += sanitize_subrecord_payloads_in_items(
                    &mut group.children,
                    max_lengths,
                    row_projections,
                    changed_form_ids,
                );
            }
        }
    }
    changed
}

fn apply_placed_record_position_offset_to_record(
    record: &mut ParsedRecord,
    offset: (f32, f32, f32),
) -> bool {
    if !is_placed_child_signature(record.signature.as_str()) {
        return false;
    }
    if record.subrecords.is_empty() {
        record.subrecords = effective_subrecords_for_record(record).into_owned();
    }

    let mut changed = false;
    for subrecord in &mut record.subrecords {
        if subrecord.signature.as_str() != "DATA" || subrecord.data.len() < 24 {
            continue;
        }
        let mut data = subrecord.data.to_vec();
        for (index, delta) in [offset.0, offset.1, offset.2].into_iter().enumerate() {
            let start = index * 4;
            let value = f32::from_le_bytes([
                data[start],
                data[start + 1],
                data[start + 2],
                data[start + 3],
            ]) + delta;
            data[start..start + 4].copy_from_slice(&value.to_le_bytes());
        }
        subrecord.data = Bytes::from(data);
        changed = true;
    }
    if changed {
        record.raw_payload = None;
    }
    changed
}

fn is_placed_child_signature(signature: &str) -> bool {
    matches!(signature, "REFR" | "ACHR" | "PHZD" | "PGRE" | "PGRD")
}

fn ensure_top_group_and_add(
    root_items: &mut Vec<ParsedItem>,
    record: ParsedRecord,
    header_size: usize,
    game: Option<&str>,
) {
    let sig_bytes = {
        let mut b = [0u8; 4];
        let sig = record.signature.as_bytes();
        let len = sig.len().min(4);
        b[..len].copy_from_slice(&sig[..len]);
        b
    };
    for item in root_items.iter_mut() {
        if let ParsedItem::Group(g) = item {
            if g.group_type == 0 && g.label == sig_bytes {
                g.children.push(ParsedItem::Record(record));
                return;
            }
        }
    }
    let tail_len = header_size.saturating_sub(16);
    let tail = Bytes::from(vec![0u8; tail_len]);
    let insert_index = top_group_insert_index(root_items, record.signature.as_str(), game);
    let mut group = ParsedGroup {
        label: sig_bytes,
        group_type: 0,
        tail,
        children: Vec::new(),
    };
    group.children.push(ParsedItem::Record(record));
    if let Some(index) = insert_index {
        root_items.insert(index, ParsedItem::Group(group));
    } else {
        root_items.push(ParsedItem::Group(group));
    }
}

fn ensure_wrld_group_and_add_projected_cell(
    root_items: &mut Vec<ParsedItem>,
    projected: ProjectedCellImport,
    location: &ProjectedCellLocation,
    header_size: usize,
    plugin_name: &str,
) -> PyResult<()> {
    ensure_wrld_group_and_add_projected_cell_inner(
        root_items,
        projected,
        location,
        header_size,
        plugin_name,
        true,
    )
}

fn ensure_wrld_group_and_add_projected_cell_fast(
    root_items: &mut Vec<ParsedItem>,
    projected: ProjectedCellImport,
    location: &ProjectedCellLocation,
    existing_match: Option<ExtractedProjectedCell>,
    header_size: usize,
    plugin_name: &str,
) -> PyResult<()> {
    let world_form_id =
        find_projected_cell_world_form_id(root_items, &location.world_dir, plugin_name)
            .ok_or_else(|| {
                value_error(format!(
                    "projected CELL target WRLD '{}' was not found in the target plugin",
                    location.world_dir
                ))
            })?;
    let wrld_group = root_items
        .iter_mut()
        .find_map(|item| match item {
            ParsedItem::Group(group) if group.group_type == 0 && group.label == *b"WRLD" => {
                Some(group)
            }
            _ => None,
        })
        .ok_or_else(|| value_error("projected CELL import requires a WRLD top group"))?;
    let world_group =
        ensure_world_children_group(&mut wrld_group.children, world_form_id, header_size);
    let block_group = ensure_exterior_grid_group(
        &mut world_group.children,
        EXTERIOR_CELL_BLOCK,
        location.block,
        header_size,
    );
    let subblock_group = ensure_exterior_grid_group(
        &mut block_group.children,
        EXTERIOR_CELL_SUBBLOCK,
        location.subblock,
        header_size,
    );
    merge_projected_cell_into_subblock(
        &mut subblock_group.children,
        projected,
        location,
        existing_match,
    );
    Ok(())
}

fn ensure_wrld_group_and_add_projected_cell_inner(
    root_items: &mut Vec<ParsedItem>,
    projected: ProjectedCellImport,
    location: &ProjectedCellLocation,
    header_size: usize,
    plugin_name: &str,
    remove_existing_by_form_id: bool,
) -> PyResult<()> {
    let world_form_id =
        find_projected_cell_world_form_id(root_items, &location.world_dir, plugin_name)
            .ok_or_else(|| {
                value_error(format!(
                    "projected CELL target WRLD '{}' was not found in the target plugin",
                    location.world_dir
                ))
            })?;
    let mut children = vec![ParsedItem::Record(projected.cell)];
    if let Some(group) = projected.child_group {
        children.push(ParsedItem::Group(group));
    }
    let cell_form_id = match &children[0] {
        ParsedItem::Record(record) => record.form_id,
        ParsedItem::Group(_) => unreachable!(),
    };

    let wrld_group = root_items
        .iter_mut()
        .find_map(|item| match item {
            ParsedItem::Group(group) if group.group_type == 0 && group.label == *b"WRLD" => {
                Some(group)
            }
            _ => None,
        })
        .ok_or_else(|| value_error("projected CELL import requires a WRLD top group"))?;
    let world_group =
        ensure_world_children_group(&mut wrld_group.children, world_form_id, header_size);
    if remove_existing_by_form_id {
        let _ = remove_record_from_items(&mut world_group.children, cell_form_id);
        let _ = remove_cell_child_group_from_items(&mut world_group.children, cell_form_id);
    }
    let block_group = ensure_exterior_grid_group(
        &mut world_group.children,
        EXTERIOR_CELL_BLOCK,
        location.block,
        header_size,
    );
    let subblock_group = ensure_exterior_grid_group(
        &mut block_group.children,
        EXTERIOR_CELL_SUBBLOCK,
        location.subblock,
        header_size,
    );
    let _ = remove_projected_cell_and_children_at_location(&mut subblock_group.children, location);
    subblock_group.children.extend(children);
    Ok(())
}

fn merge_projected_cell_into_subblock(
    items: &mut Vec<ParsedItem>,
    mut projected: ProjectedCellImport,
    location: &ProjectedCellLocation,
    existing_match: Option<ExtractedProjectedCell>,
) {
    let projected_editor_id =
        record_editor_id_value(&projected.cell).map(|value| normalize_editor_id_for_match(&value));
    let mut removed_cell_form_ids = Vec::new();
    let mut preserved_child_group_labels = std::collections::BTreeSet::new();
    let mut target_cell_form_id = None;
    let mut existing_child_groups = Vec::new();
    if let Some(existing_match) = existing_match {
        if let Some(cell) = existing_match.cell {
            target_cell_form_id.get_or_insert(cell.form_id);
            removed_cell_form_ids.push(cell.form_id);
        }
        existing_child_groups.extend(existing_match.child_groups);
    }
    let mut i = 0;
    while i < items.len() {
        let preserve_existing_id = matches!(
            &items[i],
            ParsedItem::Record(record)
                if record.signature == "CELL"
                    && projected_editor_id
                        .as_deref()
                        .is_some_and(|editor_id| record_editor_id_matches(record, editor_id))
        );
        let remove = preserve_existing_id
            || matches!(
                &items[i],
                ParsedItem::Record(record)
                    if record.signature == "CELL"
                        && projected_cell_grid_from_record(record) == Some(location.cell)
            );
        if remove {
            if let ParsedItem::Record(record) = items.remove(i) {
                if preserve_existing_id {
                    target_cell_form_id.get_or_insert(record.form_id);
                    preserved_child_group_labels.insert(record.form_id.to_le_bytes());
                }
                removed_cell_form_ids.push(record.form_id);
            }
        } else {
            i += 1;
        }
    }

    if let Some(form_id) = target_cell_form_id {
        projected.cell.form_id = form_id;
        relabel_projected_cell_children(&mut projected, form_id);
    }
    let cell_form_id = projected.cell.form_id;
    removed_cell_form_ids.push(cell_form_id);

    let labels: std::collections::BTreeSet<[u8; 4]> = removed_cell_form_ids
        .into_iter()
        .map(u32::to_le_bytes)
        .collect();
    let mut i = 0;
    while i < items.len() {
        let remove = matches!(
            &items[i],
            ParsedItem::Group(group)
                if group.group_type == CELL_CHILD_GROUP && labels.contains(&group.label)
        );
        if remove {
            if let ParsedItem::Group(group) = items.remove(i) {
                if preserved_child_group_labels.contains(&group.label) {
                    existing_child_groups.push(group);
                }
            }
        } else {
            i += 1;
        }
    }

    let child_group = merge_projected_cell_child_groups(
        existing_child_groups,
        projected.child_group,
        cell_form_id,
    );
    items.push(ParsedItem::Record(projected.cell));
    if let Some(group) = child_group {
        items.push(ParsedItem::Group(group));
    }
}

fn relabel_projected_cell_children(projected: &mut ProjectedCellImport, cell_form_id: u32) {
    if let Some(group) = projected.child_group.as_mut() {
        group.label = cell_form_id.to_le_bytes();
        update_cell_section_group_labels(&mut group.children, cell_form_id);
    }
}

fn merge_projected_cell_child_groups(
    existing_groups: Vec<ParsedGroup>,
    projected_group: Option<ParsedGroup>,
    cell_form_id: u32,
) -> Option<ParsedGroup> {
    let mut tail = None;
    let mut children = Vec::new();
    let mut existing_land_form_id = None;
    for group in existing_groups {
        tail.get_or_insert_with(|| group.tail.clone());
        for child in group.children {
            match child {
                ParsedItem::Record(record) if record.signature.as_str() == "LAND" => {
                    existing_land_form_id.get_or_insert(record.form_id);
                }
                other => children.push(other),
            }
        }
    }

    if let Some(mut group) = projected_group {
        tail.get_or_insert_with(|| group.tail.clone());
        group.label = cell_form_id.to_le_bytes();
        update_cell_section_group_labels(&mut group.children, cell_form_id);
        if let Some(land_form_id) = existing_land_form_id {
            replace_first_land_form_id(&mut group.children, land_form_id);
        }
        children.extend(group.children);
    }

    if children.is_empty() {
        return None;
    }
    Some(ParsedGroup {
        label: cell_form_id.to_le_bytes(),
        group_type: CELL_CHILD_GROUP,
        tail: tail.unwrap_or_else(|| Bytes::new()),
        children,
    })
}

fn replace_first_land_form_id(items: &mut [ParsedItem], land_form_id: u32) -> bool {
    for item in items {
        match item {
            ParsedItem::Record(record) if record.signature.as_str() == "LAND" => {
                record.form_id = land_form_id;
                return true;
            }
            ParsedItem::Group(group) => {
                if replace_first_land_form_id(&mut group.children, land_form_id) {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

fn remove_projected_cell_and_children_at_location(
    items: &mut Vec<ParsedItem>,
    location: &ProjectedCellLocation,
) -> bool {
    let mut removed_cell_form_ids = Vec::new();
    let mut i = 0;
    while i < items.len() {
        let remove = matches!(
            &items[i],
            ParsedItem::Record(record)
                if record.signature == "CELL"
                    && projected_cell_grid_from_record(record) == Some(location.cell)
        );
        if remove {
            if let ParsedItem::Record(record) = &items[i] {
                removed_cell_form_ids.push(record.form_id);
            }
            items.remove(i);
        } else {
            i += 1;
        }
    }

    if removed_cell_form_ids.is_empty() {
        return false;
    }

    let labels: std::collections::BTreeSet<[u8; 4]> = removed_cell_form_ids
        .into_iter()
        .map(u32::to_le_bytes)
        .collect();
    items.retain(|item| {
        !matches!(
            item,
            ParsedItem::Group(group)
                if group.group_type == CELL_CHILD_GROUP && labels.contains(&group.label)
        )
    });
    true
}

fn projected_cell_grid_from_record(record: &ParsedRecord) -> Option<(i16, i16)> {
    let subrecords = effective_subrecords_for_record(record);
    let xclc = subrecords
        .iter()
        .find(|subrecord| subrecord.signature.as_str() == "XCLC")
        .map(|subrecord| &subrecord.data)?;
    if xclc.len() < 8 {
        return None;
    }
    let x = i32::from_le_bytes([xclc[0], xclc[1], xclc[2], xclc[3]]);
    let y = i32::from_le_bytes([xclc[4], xclc[5], xclc[6], xclc[7]]);
    Some((i16::try_from(x).ok()?, i16::try_from(y).ok()?))
}

fn ensure_projected_cell_grid_subrecord(record: &mut ParsedRecord, cell: (i16, i16)) {
    if record.subrecords.is_empty() {
        record.subrecords = effective_subrecords_for_record(record).into_owned();
    }

    let mut data = Vec::with_capacity(8);
    data.extend_from_slice(&(cell.0 as i32).to_le_bytes());
    data.extend_from_slice(&(cell.1 as i32).to_le_bytes());
    if let Some(subrecord) = record
        .subrecords
        .iter_mut()
        .find(|subrecord| subrecord.signature.as_str() == "XCLC")
    {
        let mut replacement = data;
        if subrecord.data.len() >= 12 {
            replacement.extend_from_slice(&subrecord.data[8..12]);
        } else {
            replacement.extend_from_slice(&[0, 0, 0, 0]);
        }
        subrecord.data = Bytes::from(replacement);
    } else {
        data.extend_from_slice(&[0, 0, 0, 0]);
        record.subrecords.push(ParsedSubrecord {
            signature: SmolStr::new_static("XCLC"),
            data: Bytes::from(data),
            semantic_type: None,
        });
    }
    record.raw_payload = None;
    record.parse_error = None;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NavmeshParent {
    Exterior { world_form_id: u32, x: i16, y: i16 },
    Interior { cell_form_id: u32 },
}

fn navmesh_parent_from_record(record: &ParsedRecord) -> Result<Option<NavmeshParent>, String> {
    if record.signature.as_str() != "NAVM" {
        return Ok(None);
    }
    let subrecords = effective_subrecords_for_record(record);
    let Some(nvnm) = subrecords
        .iter()
        .find(|subrecord| subrecord.signature.as_str() == "NVNM")
        .map(|subrecord| &subrecord.data)
    else {
        return Ok(None);
    };
    if nvnm.is_empty() {
        return Ok(None);
    }
    if nvnm.len() < 16 {
        return Err(format!(
            "NAVM {:08X} NVNM payload is {} bytes; expected at least 16",
            record.form_id,
            nvnm.len()
        ));
    }
    let world_form_id = u32::from_le_bytes([nvnm[8], nvnm[9], nvnm[10], nvnm[11]]);
    if world_form_id == 0 {
        let cell_form_id = u32::from_le_bytes([nvnm[12], nvnm[13], nvnm[14], nvnm[15]]);
        return Ok((cell_form_id != 0).then_some(NavmeshParent::Interior { cell_form_id }));
    }
    let y = i16::from_le_bytes([nvnm[12], nvnm[13]]);
    let x = i16::from_le_bytes([nvnm[14], nvnm[15]]);
    Ok(Some(NavmeshParent::Exterior {
        world_form_id,
        x,
        y,
    }))
}

fn find_cell_form_id_by_grid(items: &[ParsedItem], cell: (i16, i16)) -> Option<u32> {
    build_cell_grid_index(items).get(&cell).copied()
}

fn find_cell_child_group_mut_in_items(
    items: &mut [ParsedItem],
    cell_form_id: u32,
) -> Option<&mut ParsedGroup> {
    let label = cell_form_id.to_le_bytes();
    for item in items {
        let ParsedItem::Group(group) = item else {
            continue;
        };
        if group.group_type == CELL_CHILD_GROUP && group.label == label {
            return Some(group);
        }
        if let Some(found) = find_cell_child_group_mut_in_items(&mut group.children, cell_form_id) {
            return Some(found);
        }
    }
    None
}

fn ensure_cell_section_group_mut(
    parent: &mut ParsedGroup,
    group_type: i32,
    cell_form_id: u32,
    header_size: usize,
) -> &mut ParsedGroup {
    let label = cell_form_id.to_le_bytes();
    if let Some(index) = parent.children.iter().position(|item| {
        matches!(
            item,
            ParsedItem::Group(group) if group.group_type == group_type && group.label == label
        )
    }) {
        let ParsedItem::Group(group) = &mut parent.children[index] else {
            unreachable!();
        };
        return group;
    }
    parent.children.push(ParsedItem::Group(ParsedGroup {
        label,
        group_type,
        tail: Bytes::from(vec![0u8; header_size.saturating_sub(16)]),
        children: Vec::new(),
    }));
    let ParsedItem::Group(group) = parent.children.last_mut().expect("just pushed group") else {
        unreachable!();
    };
    group
}

fn find_top_group_mut<'a>(
    items: &'a mut [ParsedItem],
    label: &[u8; 4],
) -> Option<&'a mut ParsedGroup> {
    for item in items {
        let ParsedItem::Group(group) = item else {
            continue;
        };
        if group.group_type == 0 && group.label == *label {
            return Some(group);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Interior cell topology helpers
// ---------------------------------------------------------------------------

/// Remove any `ParsedItem::Record` with signature `CELL` and the given
/// FormID from `items`, recursing into groups.  Returns `true` if one was
/// removed.
fn remove_existing_cell_record(items: &mut Vec<ParsedItem>, form_id: u32) -> bool {
    let target = form_id & 0xFFFF_FFFF;
    let mut i = 0;
    while i < items.len() {
        match &items[i] {
            ParsedItem::Record(r)
                if r.signature.as_str() == "CELL" && (r.form_id & 0xFFFF_FFFF) == target =>
            {
                items.remove(i);
                return true;
            }
            ParsedItem::Group(_) => {
                if let ParsedItem::Group(ref mut g) = items[i] {
                    if remove_existing_cell_record(&mut g.children, form_id) {
                        return true;
                    }
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    false
}

/// Ensure a group of `group_type` with label = `bucket` (little-endian i32)
/// exists in `parent.children`, creating it if necessary.
fn ensure_interior_bucket_group(
    parent: &mut ParsedGroup,
    group_type: i32,
    bucket: i32,
    header_size: usize,
) -> &mut ParsedGroup {
    let label = bucket.to_le_bytes();
    if let Some(index) = parent.children.iter().position(|item| {
        matches!(
            item,
            ParsedItem::Group(g) if g.group_type == group_type && g.label == label
        )
    }) {
        let ParsedItem::Group(g) = &mut parent.children[index] else {
            unreachable!();
        };
        return g;
    }
    parent.children.push(ParsedItem::Group(ParsedGroup {
        label,
        group_type,
        tail: Bytes::from(vec![0u8; header_size.saturating_sub(16)]),
        children: Vec::new(),
    }));
    let ParsedItem::Group(g) = parent.children.last_mut().expect("just pushed") else {
        unreachable!();
    };
    g
}

/// Ensure a top-level CELL group (group_type 0, label b"CELL") exists in
/// `root_items`, creating it if absent, and return a mutable reference.
fn ensure_cell_top_group_mut(
    root_items: &mut Vec<ParsedItem>,
    header_size: usize,
) -> &mut ParsedGroup {
    if let Some(index) = root_items.iter().position(|item| {
        matches!(
            item,
            ParsedItem::Group(g) if g.group_type == 0 && g.label == *b"CELL"
        )
    }) {
        let ParsedItem::Group(g) = &mut root_items[index] else {
            unreachable!();
        };
        return g;
    }
    root_items.push(ParsedItem::Group(ParsedGroup {
        label: *b"CELL",
        group_type: 0,
        tail: Bytes::from(vec![0u8; header_size.saturating_sub(16)]),
        children: Vec::new(),
    }));
    let ParsedItem::Group(g) = root_items.last_mut().expect("just pushed") else {
        unreachable!();
    };
    g
}

fn interior_bucket_indices(form_id: u32) -> (i32, i32) {
    let object_id = form_id & 0x00FF_FFFF;
    ((object_id % 10) as i32, ((object_id / 10) % 10) as i32)
}

/// Insert `cell_record` (an interior CELL) into the top-level CELL group under
/// its Interior Block(2)/Sub-Block(3) bucket, and ensure an empty Cell-Children
/// group (type 6) follows it.  If a CELL record with the same FormID already
/// exists anywhere in the tree (e.g. a DATA-only PKIN stub) it is removed first.
pub fn ensure_interior_cell_and_child_group(
    handle_id: u64,
    cell_record: ParsedRecord,
) -> Result<(), String> {
    let mut store = plugin_handle_store().lock().unwrap();
    let slot = store
        .get_mut(&handle_id)
        .ok_or_else(|| format!("no plugin handle: {handle_id}"))?;
    ensure_interior_cell_and_child_group_in_slot(slot, cell_record);
    Ok(())
}

fn ensure_interior_cell_and_child_group_in_slot(
    slot: &mut NativePluginSlot,
    cell_record: ParsedRecord,
) {
    let header_size = slot.parsed.header_size;
    let form_id = cell_record.form_id;
    let (block, subblock) = interior_bucket_indices(form_id);

    remove_existing_cell_record(&mut slot.parsed.root_items, form_id);

    let cell_top = ensure_cell_top_group_mut(&mut slot.parsed.root_items, header_size);
    let block_group =
        ensure_interior_bucket_group(cell_top, INTERIOR_CELL_BLOCK, block, header_size);
    let subblock_group =
        ensure_interior_bucket_group(block_group, INTERIOR_CELL_SUBBLOCK, subblock, header_size);

    subblock_group
        .children
        .push(ParsedItem::Record(cell_record));
    // Ensure an empty Cell-Children group (type 6, label = cell FormID).
    ensure_cell_section_group_mut(subblock_group, CELL_CHILD_GROUP, form_id, header_size);
}

/// Find the Cell-Children group (type 6) for `cell_form_id` anywhere in the
/// tree, ensure the requested Persistent(8)/Temporary(9) section exists, and
/// append `child` to it.  Returns `false` if no Cell-Children group exists
/// (caller must emit the cell first via `ensure_interior_cell_and_child_group`).
pub fn insert_placed_child_into_cell_group(
    handle_id: u64,
    cell_form_id: u32,
    group_type: i32,
    child: ParsedRecord,
) -> Result<bool, String> {
    let mut store = plugin_handle_store().lock().unwrap();
    let slot = store
        .get_mut(&handle_id)
        .ok_or_else(|| format!("no plugin handle: {handle_id}"))?;
    let header_size = slot.parsed.header_size;
    let Some(cell_child_group) =
        find_cell_child_group_mut_in_items(&mut slot.parsed.root_items, cell_form_id)
    else {
        return Ok(false);
    };
    let section =
        ensure_cell_section_group_mut(cell_child_group, group_type, cell_form_id, header_size);
    section.children.push(ParsedItem::Record(child));
    Ok(true)
}

/// Insert an interior `cell_record` plus its Persistent(8)/Temporary(9) placed
/// children in a single shot. Builds the Block(2)/Sub-Block(3)/CELL/
/// Cell-Children(6) subtree and attaches it without any whole-tree search.
///
/// Unlike [`ensure_interior_cell_and_child_group`] this does NOT dedup a
/// pre-existing stub for the same FormID; that O(tree) per-cell walk makes bulk
/// interior conversion quadratic. Callers strip stubs once up
/// front via [`remove_cell_records_by_object_id`] before the insert loop.
pub fn insert_interior_cell_with_children(
    handle_id: u64,
    cell_record: ParsedRecord,
    persistent: Vec<ParsedRecord>,
    temporary: Vec<ParsedRecord>,
) -> Result<(), String> {
    let mut store = plugin_handle_store().lock().unwrap();
    let slot = store
        .get_mut(&handle_id)
        .ok_or_else(|| format!("no plugin handle: {handle_id}"))?;
    let header_size = slot.parsed.header_size;
    let form_id = cell_record.form_id;
    let (block, subblock) = interior_bucket_indices(form_id);

    let cell_top = ensure_cell_top_group_mut(&mut slot.parsed.root_items, header_size);
    let block_group =
        ensure_interior_bucket_group(cell_top, INTERIOR_CELL_BLOCK, block, header_size);
    let subblock_group =
        ensure_interior_bucket_group(block_group, INTERIOR_CELL_SUBBLOCK, subblock, header_size);

    subblock_group
        .children
        .push(ParsedItem::Record(cell_record));
    let cell_child =
        ensure_cell_section_group_mut(subblock_group, CELL_CHILD_GROUP, form_id, header_size);
    if !persistent.is_empty() {
        let section =
            ensure_cell_section_group_mut(cell_child, PERSISTENT_GROUP, form_id, header_size);
        section
            .children
            .extend(persistent.into_iter().map(ParsedItem::Record));
    }
    if !temporary.is_empty() {
        let section =
            ensure_cell_section_group_mut(cell_child, TEMPORARY_GROUP, form_id, header_size);
        section
            .children
            .extend(temporary.into_iter().map(ParsedItem::Record));
    }
    Ok(())
}

/// Remove every `CELL` record whose object id (low 24 bits) is in `object_ids`,
/// anywhere in the tree, in a single pass. Returns the number removed. Used by
/// the interior-cell phase to strip PKIN storage-cell stubs once before the
/// real interior cells are emitted.
pub fn remove_cell_records_by_object_id(
    handle_id: u64,
    object_ids: &[u32],
) -> Result<usize, String> {
    let mut store = plugin_handle_store().lock().unwrap();
    let slot = store
        .get_mut(&handle_id)
        .ok_or_else(|| format!("no plugin handle: {handle_id}"))?;
    let set: std::collections::HashSet<u32> =
        object_ids.iter().map(|id| id & 0x00FF_FFFF).collect();
    if set.is_empty() {
        return Ok(0);
    }

    fn walk(
        items: &mut Vec<ParsedItem>,
        set: &std::collections::HashSet<u32>,
        removed: &mut usize,
    ) {
        items.retain(|item| match item {
            ParsedItem::Record(r)
                if r.signature.as_str() == "CELL" && set.contains(&(r.form_id & 0x00FF_FFFF)) =>
            {
                *removed += 1;
                false
            }
            _ => true,
        });
        for item in items.iter_mut() {
            if let ParsedItem::Group(group) = item {
                walk(&mut group.children, set, removed);
            }
        }
    }

    let mut removed = 0;
    walk(&mut slot.parsed.root_items, &set, &mut removed);
    Ok(removed)
}

fn ensure_quest_child_group_mut(
    parent: &mut ParsedGroup,
    quest_form_id: u32,
    header_size: usize,
) -> &mut ParsedGroup {
    let label = quest_form_id.to_le_bytes();
    if let Some(index) = parent.children.iter().position(|item| {
        matches!(
            item,
            ParsedItem::Group(group) if group.group_type == QUEST_CHILD_GROUP && group.label == label
        )
    }) {
        let ParsedItem::Group(group) = &mut parent.children[index] else {
            unreachable!();
        };
        return group;
    }

    let insert_index = parent
        .children
        .iter()
        .position(|item| {
            matches!(
                item,
                ParsedItem::Record(record)
                    if record.signature.as_str() == "QUST"
                        && (record.form_id & 0xFFFF_FFFF) == (quest_form_id & 0xFFFF_FFFF)
            )
        })
        .map(|index| index + 1)
        .unwrap_or(parent.children.len());
    parent.children.insert(
        insert_index,
        ParsedItem::Group(ParsedGroup {
            label,
            group_type: QUEST_CHILD_GROUP,
            tail: Bytes::from(vec![0u8; header_size.saturating_sub(16)]),
            children: Vec::new(),
        }),
    );
    let ParsedItem::Group(group) = &mut parent.children[insert_index] else {
        unreachable!();
    };
    group
}

fn ensure_topic_child_group_mut(
    parent: &mut ParsedGroup,
    dialogue_form_id: u32,
    header_size: usize,
) -> &mut ParsedGroup {
    let label = dialogue_form_id.to_le_bytes();
    if let Some(index) = parent.children.iter().position(|item| {
        matches!(
            item,
            ParsedItem::Group(group) if group.group_type == TOPIC_CHILD_GROUP && group.label == label
        )
    }) {
        let ParsedItem::Group(group) = &mut parent.children[index] else {
            unreachable!();
        };
        return group;
    }

    let insert_index = parent
        .children
        .iter()
        .position(|item| {
            matches!(
                item,
                ParsedItem::Record(record)
                    if record.signature.as_str() == "DIAL"
                        && (record.form_id & 0xFFFF_FFFF) == (dialogue_form_id & 0xFFFF_FFFF)
            )
        })
        .map(|index| index + 1)
        .unwrap_or(parent.children.len());
    parent.children.insert(
        insert_index,
        ParsedItem::Group(ParsedGroup {
            label,
            group_type: TOPIC_CHILD_GROUP,
            tail: Bytes::from(vec![0u8; header_size.saturating_sub(16)]),
            children: Vec::new(),
        }),
    );
    let ParsedItem::Group(group) = &mut parent.children[insert_index] else {
        unreachable!();
    };
    group
}

fn insert_navmesh_into_cell_child_group(
    items: &mut [ParsedItem],
    cell_form_id: u32,
    header_size: usize,
    record: ParsedRecord,
) -> bool {
    let Some(cell_child_group) = find_cell_child_group_mut_in_items(items, cell_form_id) else {
        return false;
    };
    let navmesh_form_id = record.form_id;
    remove_record_from_items(&mut cell_child_group.children, navmesh_form_id);
    let temporary_group =
        ensure_cell_section_group_mut(cell_child_group, TEMPORARY_GROUP, cell_form_id, header_size);
    temporary_group.children.push(ParsedItem::Record(record));
    true
}

pub fn insert_quest_child_record_in_slot(
    slot: &mut NativePluginSlot,
    parent_quest_form_id: u32,
    record: ParsedRecord,
) -> Result<bool, String> {
    if parent_quest_form_id == 0 {
        return Ok(false);
    }
    if !record_exists_in_items(&slot.parsed.root_items, "QUST", parent_quest_form_id) {
        return Ok(false);
    }

    let object_id = record.form_id & 0x00FF_FFFF;
    let next_object_id = slot.parsed.header.next_object_id & 0x00FF_FFFF;
    if object_id != 0 && object_id >= next_object_id {
        slot.parsed.header.next_object_id = (object_id + 1) & 0x00FF_FFFF;
    }

    let header_size = slot.parsed.header_size;
    remove_record_from_items(&mut slot.parsed.root_items, record.form_id);
    let Some(quest_group) = find_top_group_mut(&mut slot.parsed.root_items, b"QUST") else {
        return Ok(false);
    };
    let child_group = ensure_quest_child_group_mut(quest_group, parent_quest_form_id, header_size);
    child_group.children.push(ParsedItem::Record(record));
    slot.clear_record_count_cache();
    Ok(true)
}

/// Insert an encoded INFO under its parent DIAL's Topic-Child group.
///
/// `parent_dialogue_form_id` is the TARGET form_id of the parent DIAL. INFO->DIAL
/// parentage is expressed only by group nesting (a Topic-Child group, type 7,
/// whose label is the DIAL form_id) — DIAL records carry no subrecord that lists
/// their child INFOs — so the caller must supply the parent it recovered from the
/// source group nesting. Returns `Ok(false)` when the parent DIAL is not present
/// in the target plugin yet.
pub fn insert_topic_child_record_in_slot(
    slot: &mut NativePluginSlot,
    parent_dialogue_form_id: u32,
    record: ParsedRecord,
) -> Result<bool, String> {
    if record.signature.as_str() != "INFO" {
        return Ok(false);
    }

    let object_id = record.form_id & 0x00FF_FFFF;
    let next_object_id = slot.parsed.header.next_object_id & 0x00FF_FFFF;
    if object_id != 0 && object_id >= next_object_id {
        slot.parsed.header.next_object_id = (object_id + 1) & 0x00FF_FFFF;
    }

    let header_size = slot.parsed.header_size;
    remove_record_from_items(&mut slot.parsed.root_items, record.form_id);
    let mut record = Some(record);
    if !insert_info_under_dialogue(
        &mut slot.parsed.root_items,
        parent_dialogue_form_id,
        header_size,
        &mut record,
    ) {
        return Ok(false);
    }
    slot.clear_record_count_cache();
    Ok(true)
}

#[derive(Clone, Copy)]
struct QuestChildParentIndex {
    quest_record_index: usize,
    child_group_index: Option<usize>,
}

pub struct QuestChildInsertIndex {
    top_group_index: Option<usize>,
    record_form_ids: HashSet<u32>,
    parents: HashMap<u32, QuestChildParentIndex>,
    fast_inserts: usize,
    serial_fallbacks: usize,
}

pub struct TopicChildInsertIndex {
    top_group_index: Option<usize>,
    record_form_ids: HashSet<u32>,
    dialogues: HashMap<u32, (usize, u32)>,
    fast_inserts: usize,
    serial_fallbacks: usize,
}

fn collect_child_insert_record_form_ids(
    items: &[ParsedItem],
    record_form_ids: &mut HashSet<u32>,
) {
    for item in items {
        match item {
            ParsedItem::Record(record) => {
                record_form_ids.insert(record.form_id);
            }
            ParsedItem::Group(group) => {
                collect_child_insert_record_form_ids(&group.children, record_form_ids);
            }
        }
    }
}

pub fn build_quest_child_insert_index(slot: &NativePluginSlot) -> QuestChildInsertIndex {
    let mut record_form_ids = HashSet::new();
    collect_child_insert_record_form_ids(&slot.parsed.root_items, &mut record_form_ids);
    let top_group_index = slot.parsed.root_items.iter().position(|item| {
        matches!(
            item,
            ParsedItem::Group(group) if group.group_type == 0 && group.label == *b"QUST"
        )
    });
    let mut parents = HashMap::new();
    let mut ambiguous_parents = HashSet::new();
    if let Some(top_group_index) = top_group_index
        && let ParsedItem::Group(group) = &slot.parsed.root_items[top_group_index]
    {
        for (index, item) in group.children.iter().enumerate() {
            if let ParsedItem::Record(record) = item
                && record.signature.as_str() == "QUST"
            {
                if parents
                    .insert(
                        record.form_id,
                        QuestChildParentIndex {
                            quest_record_index: index,
                            child_group_index: None,
                        },
                    )
                    .is_some()
                {
                    ambiguous_parents.insert(record.form_id);
                }
            }
        }
        for (index, item) in group.children.iter().enumerate() {
            if let ParsedItem::Group(child) = item
                && child.group_type == QUEST_CHILD_GROUP
                && let Some(parent) = parents.get_mut(&u32::from_le_bytes(child.label))
            {
                if parent.child_group_index.replace(index).is_some() {
                    ambiguous_parents.insert(u32::from_le_bytes(child.label));
                }
            }
        }
    }
    for parent in ambiguous_parents {
        parents.remove(&parent);
    }
    QuestChildInsertIndex {
        top_group_index,
        record_form_ids,
        parents,
        fast_inserts: 0,
        serial_fallbacks: 0,
    }
}

fn collect_serial_topic_dialogue_matches(
    items: &[ParsedItem],
    inside_quest_child: bool,
    matches: &mut HashMap<u32, (u32, usize)>,
) {
    for item in items {
        match item {
            ParsedItem::Record(record)
                if inside_quest_child && record.signature.as_str() == "DIAL" =>
            {
                let object_id = record.form_id & 0x00FF_FFFF;
                let entry = matches.entry(object_id).or_insert((record.form_id, 0));
                entry.1 += 1;
            }
            ParsedItem::Group(group) => collect_serial_topic_dialogue_matches(
                &group.children,
                inside_quest_child || group.group_type == QUEST_CHILD_GROUP,
                matches,
            ),
            _ => {}
        }
    }
}

pub fn build_topic_child_insert_index(slot: &NativePluginSlot) -> TopicChildInsertIndex {
    let mut record_form_ids = HashSet::new();
    collect_child_insert_record_form_ids(&slot.parsed.root_items, &mut record_form_ids);
    let top_group_index = slot.parsed.root_items.iter().position(|item| {
        matches!(
            item,
            ParsedItem::Group(group) if group.group_type == 0 && group.label == *b"QUST"
        )
    });
    let mut dialogues = HashMap::new();
    let mut ambiguous_direct_dialogues = HashSet::new();
    if let Some(top_group_index) = top_group_index
        && let ParsedItem::Group(group) = &slot.parsed.root_items[top_group_index]
    {
        for (quest_child_index, item) in group.children.iter().enumerate() {
            let ParsedItem::Group(quest_child) = item else {
                continue;
            };
            if quest_child.group_type != QUEST_CHILD_GROUP {
                continue;
            }
            for child in &quest_child.children {
                if let ParsedItem::Record(record) = child
                    && record.signature.as_str() == "DIAL"
                {
                    let object_id = record.form_id & 0x00FF_FFFF;
                    if dialogues
                        .insert(object_id, (quest_child_index, record.form_id))
                        .is_some()
                    {
                        ambiguous_direct_dialogues.insert(object_id);
                    }
                }
            }
        }
    }
    let mut serial_matches = HashMap::new();
    collect_serial_topic_dialogue_matches(
        &slot.parsed.root_items,
        false,
        &mut serial_matches,
    );
    dialogues.retain(|object_id, (_, full_form_id)| {
        !ambiguous_direct_dialogues.contains(object_id)
            && serial_matches
                .get(object_id)
                .is_some_and(|(serial_full_form_id, count)| {
                    *count == 1 && serial_full_form_id == full_form_id
                })
    });
    TopicChildInsertIndex {
        top_group_index,
        record_form_ids,
        dialogues,
        fast_inserts: 0,
        serial_fallbacks: 0,
    }
}

impl QuestChildInsertIndex {
    pub fn fast_inserts(&self) -> usize {
        self.fast_inserts
    }

    pub fn serial_fallbacks(&self) -> usize {
        self.serial_fallbacks
    }
}

impl TopicChildInsertIndex {
    pub fn fast_inserts(&self) -> usize {
        self.fast_inserts
    }

    pub fn serial_fallbacks(&self) -> usize {
        self.serial_fallbacks
    }
}

fn rebuild_quest_child_insert_index(
    slot: &NativePluginSlot,
    index: &mut QuestChildInsertIndex,
) {
    let fast_inserts = index.fast_inserts;
    let serial_fallbacks = index.serial_fallbacks;
    *index = build_quest_child_insert_index(slot);
    index.fast_inserts = fast_inserts;
    index.serial_fallbacks = serial_fallbacks;
}

fn rebuild_topic_child_insert_index(
    slot: &NativePluginSlot,
    index: &mut TopicChildInsertIndex,
) {
    let fast_inserts = index.fast_inserts;
    let serial_fallbacks = index.serial_fallbacks;
    *index = build_topic_child_insert_index(slot);
    index.fast_inserts = fast_inserts;
    index.serial_fallbacks = serial_fallbacks;
}

pub fn insert_quest_child_record_indexed_in_slot(
    slot: &mut NativePluginSlot,
    index: &mut QuestChildInsertIndex,
    parent_quest_form_id: u32,
    record: ParsedRecord,
) -> Result<bool, String> {
    let placement = index.parents.get(&parent_quest_form_id).copied();
    let fast_path = parent_quest_form_id != 0
        && !index.record_form_ids.contains(&record.form_id)
        && index.top_group_index.is_some()
        && placement.is_some();
    if !fast_path {
        index.serial_fallbacks += 1;
        let result = insert_quest_child_record_in_slot(slot, parent_quest_form_id, record);
        rebuild_quest_child_insert_index(slot, index);
        return result;
    }

    let top_group_index = index.top_group_index.unwrap();
    let placement = placement.unwrap();
    let valid_placement = matches!(
        slot.parsed.root_items.get(top_group_index),
        Some(ParsedItem::Group(group))
            if group.group_type == 0
                && group.label == *b"QUST"
                && matches!(
                    group.children.get(placement.quest_record_index),
                    Some(ParsedItem::Record(parent))
                        if parent.signature.as_str() == "QUST"
                            && parent.form_id == parent_quest_form_id
                )
                && placement.child_group_index.is_none_or(|child_index| matches!(
                    group.children.get(child_index),
                    Some(ParsedItem::Group(child))
                        if child.group_type == QUEST_CHILD_GROUP
                            && child.label == parent_quest_form_id.to_le_bytes()
                ))
    );
    if !valid_placement {
        index.serial_fallbacks += 1;
        let result = insert_quest_child_record_in_slot(slot, parent_quest_form_id, record);
        rebuild_quest_child_insert_index(slot, index);
        return result;
    }

    let object_id = record.form_id & 0x00FF_FFFF;
    let next_object_id = slot.parsed.header.next_object_id & 0x00FF_FFFF;
    if object_id != 0 && object_id >= next_object_id {
        slot.parsed.header.next_object_id = (object_id + 1) & 0x00FF_FFFF;
    }
    let record_form_id = record.form_id;
    let header_size = slot.parsed.header_size;
    let ParsedItem::Group(top_group) = &mut slot.parsed.root_items[top_group_index] else {
        unreachable!();
    };
    if let Some(child_group_index) = placement.child_group_index {
        let ParsedItem::Group(child_group) = &mut top_group.children[child_group_index] else {
            unreachable!();
        };
        child_group.children.push(ParsedItem::Record(record));
    } else {
        let child_group_index = placement.quest_record_index + 1;
        top_group.children.insert(
            child_group_index,
            ParsedItem::Group(ParsedGroup {
                label: parent_quest_form_id.to_le_bytes(),
                group_type: QUEST_CHILD_GROUP,
                tail: Bytes::from(vec![0u8; header_size.saturating_sub(16)]),
                children: vec![ParsedItem::Record(record)],
            }),
        );
        for parent in index.parents.values_mut() {
            if parent.quest_record_index >= child_group_index {
                parent.quest_record_index += 1;
            }
            if let Some(existing_child_index) = parent.child_group_index
                && existing_child_index >= child_group_index
            {
                parent.child_group_index = Some(existing_child_index + 1);
            }
        }
        index
            .parents
            .get_mut(&parent_quest_form_id)
            .unwrap()
            .child_group_index = Some(child_group_index);
    }
    index.record_form_ids.insert(record_form_id);
    index.fast_inserts += 1;
    slot.clear_record_count_cache();
    Ok(true)
}

pub fn insert_topic_child_record_indexed_in_slot(
    slot: &mut NativePluginSlot,
    index: &mut TopicChildInsertIndex,
    parent_dialogue_form_id: u32,
    record: ParsedRecord,
) -> Result<bool, String> {
    let parent_object_id = parent_dialogue_form_id & 0x00FF_FFFF;
    let placement = index.dialogues.get(&parent_object_id).copied();
    let fast_path = record.signature.as_str() == "INFO"
        && !index.record_form_ids.contains(&record.form_id)
        && index.top_group_index.is_some()
        && placement.is_some();
    if !fast_path {
        index.serial_fallbacks += 1;
        let result = insert_topic_child_record_in_slot(slot, parent_dialogue_form_id, record);
        rebuild_topic_child_insert_index(slot, index);
        return result;
    }

    let top_group_index = index.top_group_index.unwrap();
    let (quest_child_index, dialogue_full_form_id) = placement.unwrap();
    let valid_placement = matches!(
        slot.parsed.root_items.get(top_group_index),
        Some(ParsedItem::Group(group))
            if group.group_type == 0
                && group.label == *b"QUST"
                && matches!(
                    group.children.get(quest_child_index),
                    Some(ParsedItem::Group(quest_child))
                        if quest_child.group_type == QUEST_CHILD_GROUP
                            && find_record_full_form_id_by_object_id(
                                &quest_child.children,
                                "DIAL",
                                parent_object_id,
                            ) == Some(dialogue_full_form_id)
                )
    );
    if !valid_placement {
        index.serial_fallbacks += 1;
        let result = insert_topic_child_record_in_slot(slot, parent_dialogue_form_id, record);
        rebuild_topic_child_insert_index(slot, index);
        return result;
    }

    let object_id = record.form_id & 0x00FF_FFFF;
    let next_object_id = slot.parsed.header.next_object_id & 0x00FF_FFFF;
    if object_id != 0 && object_id >= next_object_id {
        slot.parsed.header.next_object_id = (object_id + 1) & 0x00FF_FFFF;
    }
    let record_form_id = record.form_id;
    let header_size = slot.parsed.header_size;
    let ParsedItem::Group(top_group) = &mut slot.parsed.root_items[top_group_index] else {
        unreachable!();
    };
    let ParsedItem::Group(quest_child) = &mut top_group.children[quest_child_index] else {
        unreachable!();
    };
    let topic_group = ensure_topic_child_group_mut(
        quest_child,
        dialogue_full_form_id,
        header_size,
    );
    topic_group.children.push(ParsedItem::Record(record));
    index.record_form_ids.insert(record_form_id);
    index.fast_inserts += 1;
    slot.clear_record_count_cache();
    Ok(true)
}

fn insert_info_under_dialogue(
    items: &mut [ParsedItem],
    dialogue_form_id: u32,
    header_size: usize,
    record: &mut Option<ParsedRecord>,
) -> bool {
    for item in items {
        let ParsedItem::Group(group) = item else {
            continue;
        };
        // Match the parent DIAL by object-id and recover its FULL form_id: the
        // caller passes the mapper's 24-bit `target.local`, but the emitted DIAL
        // carries the output plugin's own-index byte, and the FO4 Topic-Child
        // group label must be that full form_id.
        if group.group_type == QUEST_CHILD_GROUP {
            if let Some(dial_full_form_id) =
                find_record_full_form_id_by_object_id(&group.children, "DIAL", dialogue_form_id)
            {
                let topic_group =
                    ensure_topic_child_group_mut(group, dial_full_form_id, header_size);
                topic_group.children.push(ParsedItem::Record(
                    record.take().expect("record inserted once"),
                ));
                return true;
            }
        }
        if insert_info_under_dialogue(&mut group.children, dialogue_form_id, header_size, record) {
            return true;
        }
    }
    false
}

// ── TEMP NAVDIAG (remove after navmesh=0 root-cause) ───────────────────
// Gated on MODBOX_NAVDIAG=1. Reports which precondition of the exterior
// projected-navmesh insert fails (world-children-group lookup vs cell-grid
// lookup) and dumps the available labels/grids for the first few failures.
fn navdiag_enabled() -> bool {
    std::env::var("MODBOX_NAVDIAG")
        .map(|v| v == "1")
        .unwrap_or(false)
}

fn navdiag_world_children_labels(items: &[ParsedItem]) -> Vec<String> {
    let mut out = Vec::new();
    for item in items {
        if let ParsedItem::Group(group) = item {
            if group.group_type == 0 && group.label == *b"WRLD" {
                for child in &group.children {
                    if let ParsedItem::Group(cg) = child {
                        if cg.group_type == 1 {
                            out.push(format!("{:08X}", u32::from_le_bytes(cg.label)));
                        }
                    }
                }
            }
        }
    }
    out
}

fn navdiag_cell_grids(items: &[ParsedItem]) -> Vec<(i16, i16)> {
    fn walk(items: &[ParsedItem], out: &mut Vec<(i16, i16)>) {
        for item in items {
            match item {
                ParsedItem::Record(r) if r.signature.as_str() == "CELL" => {
                    if let Some(g) = projected_cell_grid_from_record(r) {
                        out.push(g);
                    }
                }
                ParsedItem::Group(g) => walk(&g.children, out),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(items, &mut out);
    out
}

fn navdiag_world_fail(root_items: &[ParsedItem], navm_form_id: u32, world_form_id: u32) {
    if !navdiag_enabled() {
        return;
    }
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    if n < 5 {
        eprintln!(
            "NAVDIAG world-fail #{n}: navm={:08X} want_world_label={:08X} available_world_children_labels={:?}",
            navm_form_id,
            world_form_id,
            navdiag_world_children_labels(root_items)
        );
    }
}

fn navdiag_cell_fail(
    world_children: &[ParsedItem],
    navm_form_id: u32,
    world_form_id: u32,
    x: i16,
    y: i16,
) {
    if !navdiag_enabled() {
        return;
    }
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    if n < 5 {
        let grids = navdiag_cell_grids(world_children);
        eprintln!(
            "NAVDIAG cell-fail #{n}: navm={:08X} world={:08X} want_grid=({x},{y}) cell_count={} first_grids={:?}",
            navm_form_id,
            world_form_id,
            grids.len(),
            grids.iter().take(40).collect::<Vec<_>>()
        );
    }
}

/// Insert an encoded NAVM into the cell child group described by its NVNM
/// parent fields. Returns `Ok(false)` when the parent cell/world is not present
/// in the target plugin yet.
pub fn insert_projected_navmesh_record_in_slot(
    slot: &mut NativePluginSlot,
    record: ParsedRecord,
) -> Result<bool, String> {
    let Some(parent) = navmesh_parent_from_record(&record)? else {
        return Ok(false);
    };
    let header_size = slot.parsed.header_size;
    match parent {
        NavmeshParent::Interior { cell_form_id } => {
            if find_cell_child_group_mut_in_items(&mut slot.parsed.root_items, cell_form_id)
                .is_none()
            {
                let cell_record = find_record_mut(&mut slot.parsed.root_items, cell_form_id)
                    .filter(|cell| cell.signature.as_str() == "CELL")
                    .cloned();
                let Some(cell_record) = cell_record else {
                    return Ok(false);
                };
                ensure_interior_cell_and_child_group_in_slot(slot, cell_record);
            }
            Ok(insert_navmesh_into_cell_child_group(
                &mut slot.parsed.root_items,
                cell_form_id,
                header_size,
                record,
            ))
        }
        NavmeshParent::Exterior {
            world_form_id,
            x,
            y,
        } => {
            let Some(world_children) =
                projected_world_children_group_mut(&mut slot.parsed.root_items, world_form_id)
            else {
                navdiag_world_fail(&slot.parsed.root_items, record.form_id, world_form_id);
                return Ok(false);
            };
            let Some(cell_form_id) = find_cell_form_id_by_grid(&world_children.children, (x, y))
            else {
                navdiag_cell_fail(
                    &world_children.children,
                    record.form_id,
                    world_form_id,
                    x,
                    y,
                );
                return Ok(false);
            };
            Ok(insert_navmesh_into_cell_child_group(
                &mut world_children.children,
                cell_form_id,
                header_size,
                record,
            ))
        }
    }
}

/// First-match grid index over true exterior CELL block/subblock topology.
/// Worldspace persistent CELLs can carry XCLC=(0,0), but they are not valid
/// parents for exterior NAVM records and must never win the grid lookup.
fn build_cell_grid_index(items: &[ParsedItem]) -> HashMap<(i16, i16), u32> {
    fn walk(items: &[ParsedItem], inside_exterior_group: bool, out: &mut HashMap<(i16, i16), u32>) {
        for item in items {
            match item {
                ParsedItem::Record(record)
                    if inside_exterior_group && record.signature.as_str() == "CELL" =>
                {
                    if let Some(grid) = projected_cell_grid_from_record(record) {
                        out.entry(grid).or_insert(record.form_id);
                    }
                }
                ParsedItem::Group(group)
                    if matches!(
                        group.group_type,
                        EXTERIOR_CELL_BLOCK | EXTERIOR_CELL_SUBBLOCK
                    ) =>
                {
                    walk(&group.children, true, out)
                }
                _ => {}
            }
        }
    }
    let mut out = HashMap::new();
    walk(items, false, &mut out);
    out
}

/// Batch form of `insert_navmesh_into_cell_child_group`: resolves the cell
/// child group once, then applies the same remove-then-append per record in
/// batch order. Records whose cell child group is missing keep `Ok(false)`,
/// matching the single-record fn.
fn insert_navmesh_batch_into_cell_child_group(
    items: &mut [ParsedItem],
    cell_form_id: u32,
    header_size: usize,
    batch: Vec<(usize, ParsedRecord)>,
    outcomes: &mut [Result<bool, String>],
) {
    let Some(cell_child_group) = find_cell_child_group_mut_in_items(items, cell_form_id) else {
        return;
    };
    for (idx, record) in batch {
        remove_record_from_items(&mut cell_child_group.children, record.form_id);
        let temporary_group = ensure_cell_section_group_mut(
            cell_child_group,
            TEMPORARY_GROUP,
            cell_form_id,
            header_size,
        );
        temporary_group.children.push(ParsedItem::Record(record));
        outcomes[idx] = Ok(true);
    }
}

/// Batch form of `insert_projected_navmesh_record_in_slot`: one parent-cell
/// grid index per world, one cell-child-group walk per destination cell.
/// Outcomes are returned in input order and match the single-record fn
/// (`Ok(true)`/`Ok(false)`/`Err`). Equivalent to repeated single-record calls
/// because emitted NAVMs never add CELL records or reorder existing groups, so
/// destination resolution is insert-invariant; within a destination cell,
/// batch appends preserve input order.
pub fn insert_projected_navmeshes_batch_in_slot(
    slot: &mut NativePluginSlot,
    records: Vec<ParsedRecord>,
) -> Vec<Result<bool, String>> {
    let header_size = slot.parsed.header_size;
    let mut outcomes: Vec<Result<bool, String>> = Vec::with_capacity(records.len());
    // (world_form_id, cell_form_id, input_index, record) for exterior records;
    // interiors + parent-parse failures resolved inline (rare on this pipeline).
    let mut exterior: Vec<(u32, u32, usize, ParsedRecord)> = Vec::new();
    // Lazily-built first-match grid index per world.
    let mut grid_index: HashMap<u32, HashMap<(i16, i16), u32>> = HashMap::new();

    for (idx, record) in records.into_iter().enumerate() {
        outcomes.push(Ok(false));
        match navmesh_parent_from_record(&record) {
            Err(e) => outcomes[idx] = Err(e),
            Ok(None) => {}
            Ok(Some(NavmeshParent::Interior { cell_form_id })) => {
                if find_cell_child_group_mut_in_items(&mut slot.parsed.root_items, cell_form_id)
                    .is_none()
                {
                    let cell_record = find_record_mut(&mut slot.parsed.root_items, cell_form_id)
                        .filter(|cell| cell.signature.as_str() == "CELL")
                        .cloned();
                    let Some(cell_record) = cell_record else {
                        continue;
                    };
                    ensure_interior_cell_and_child_group_in_slot(slot, cell_record);
                }
                outcomes[idx] = Ok(insert_navmesh_into_cell_child_group(
                    &mut slot.parsed.root_items,
                    cell_form_id,
                    header_size,
                    record,
                ));
            }
            Ok(Some(NavmeshParent::Exterior {
                world_form_id,
                x,
                y,
            })) => {
                let index = match grid_index.entry(world_form_id) {
                    std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                    std::collections::hash_map::Entry::Vacant(entry) => {
                        let Some(world_children) = projected_world_children_group_mut(
                            &mut slot.parsed.root_items,
                            world_form_id,
                        ) else {
                            navdiag_world_fail(
                                &slot.parsed.root_items,
                                record.form_id,
                                world_form_id,
                            );
                            continue;
                        };
                        entry.insert(build_cell_grid_index(&world_children.children))
                    }
                };
                match index.get(&(x, y)).copied() {
                    Some(cell_form_id) => {
                        exterior.push((world_form_id, cell_form_id, idx, record));
                    }
                    None => {
                        if let Some(world_children) = projected_world_children_group_mut(
                            &mut slot.parsed.root_items,
                            world_form_id,
                        ) {
                            navdiag_cell_fail(
                                &world_children.children,
                                record.form_id,
                                world_form_id,
                                x,
                                y,
                            );
                        }
                    }
                }
            }
        }
    }

    // Group by (world, cell) preserving input order within each bucket, then
    // ONE cell-child-group walk per destination cell. BTreeMap keying keeps the
    // bucket visit order deterministic; per-bucket Vec keeps input order.
    let mut buckets: std::collections::BTreeMap<(u32, u32), Vec<(usize, ParsedRecord)>> =
        std::collections::BTreeMap::new();
    for (world_form_id, cell_form_id, idx, record) in exterior {
        buckets
            .entry((world_form_id, cell_form_id))
            .or_default()
            .push((idx, record));
    }
    for ((world_form_id, cell_form_id), batch) in buckets {
        let Some(world_children) =
            projected_world_children_group_mut(&mut slot.parsed.root_items, world_form_id)
        else {
            continue;
        };
        insert_navmesh_batch_into_cell_child_group(
            &mut world_children.children,
            cell_form_id,
            header_size,
            batch,
            &mut outcomes,
        );
    }
    outcomes
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NaviRebuildStats {
    pub records_added: u32,
    pub records_replaced: u32,
    pub records_removed: u32,
    pub navmesh_infos: u32,
    pub edge_links: u32,
    pub stale_edge_links_dropped: u32,
    pub warnings: u32,
    pub navmeshes_seen: u32,
    pub navmeshes_touched: u32,
    pub navmesh_bad_internal_links: u32,
    pub navmesh_linked_edge_vertex_mismatches: u32,
    pub navmesh_opposite_normal_linked_pairs: u32,
    pub navmesh_missing_internal_links: u32,
    pub navmesh_same_direction_internal_edges: u32,
    pub navmesh_ambiguous_local_edges: u32,
    pub navmesh_external_links_added: u32,
    pub navmesh_missing_external_links: u32,
    pub navmesh_ambiguous_external_edges: u32,
    pub navmesh_external_link_caps_hit: u32,
    pub navmesh_winding_conflicts: u32,
}

/// Field-by-field add of every counter — used to fold per-item local stats
/// from parallel NAVI sections back into the shared stats in input order.
fn merge_navi_stats(into: &mut NaviRebuildStats, from: &NaviRebuildStats) {
    into.records_added += from.records_added;
    into.records_replaced += from.records_replaced;
    into.records_removed += from.records_removed;
    into.navmesh_infos += from.navmesh_infos;
    into.edge_links += from.edge_links;
    into.stale_edge_links_dropped += from.stale_edge_links_dropped;
    into.warnings += from.warnings;
    into.navmeshes_seen += from.navmeshes_seen;
    into.navmeshes_touched += from.navmeshes_touched;
    into.navmesh_bad_internal_links += from.navmesh_bad_internal_links;
    into.navmesh_linked_edge_vertex_mismatches += from.navmesh_linked_edge_vertex_mismatches;
    into.navmesh_opposite_normal_linked_pairs += from.navmesh_opposite_normal_linked_pairs;
    into.navmesh_missing_internal_links += from.navmesh_missing_internal_links;
    into.navmesh_same_direction_internal_edges += from.navmesh_same_direction_internal_edges;
    into.navmesh_ambiguous_local_edges += from.navmesh_ambiguous_local_edges;
    into.navmesh_external_links_added += from.navmesh_external_links_added;
    into.navmesh_missing_external_links += from.navmesh_missing_external_links;
    into.navmesh_ambiguous_external_edges += from.navmesh_ambiguous_external_edges;
    into.navmesh_external_link_caps_hit += from.navmesh_external_link_caps_hit;
    into.navmesh_winding_conflicts += from.navmesh_winding_conflicts;
}

#[derive(Debug, Clone)]
struct NavmeshInfoInput {
    form_id: u32,
    pathing_cell_crc_hash: u32,
    parent: NavmeshParent,
    approx_location: (f32, f32, f32),
    edge_links: Vec<u32>,
    island_data: Option<NavmeshIslandData>,
}

#[derive(Debug, Clone)]
struct NavmeshIslandData {
    min: (f32, f32, f32),
    max: (f32, f32, f32),
    triangles: Vec<[u16; 3]>,
    vertices: Vec<(f32, f32, f32)>,
}

/// Rebuild the top-level NAVI record from the NAVM records currently emitted
/// into the target plugin. Source NAVI records cannot be safely copied after
/// NAVM filtering/remapping because they may reference dropped meshes.
pub fn rebuild_projected_navi_record_in_slot(
    slot: &mut NativePluginSlot,
    preferred_form_id: Option<u32>,
) -> Result<NaviRebuildStats, String> {
    normalize_finalized_fo4_navmesh_versions(&mut slot.parsed.root_items)?;
    let mut stats = NaviRebuildStats::default();
    let mut navmesh_ids = HashSet::new();
    collect_navmesh_form_ids(&slot.parsed.root_items, &mut navmesh_ids);

    let existing_navi_form_id =
        find_first_record_form_id_by_signature(&slot.parsed.root_items, "NAVI");
    stats.records_removed = remove_records_by_signature(&mut slot.parsed.root_items, "NAVI") as u32;

    if navmesh_ids.is_empty() {
        if stats.records_removed > 0 {
            slot.apply_write_effect(&WriteEffect::RecordsAddedOrRemoved);
        }
        return Ok(stats);
    }

    let mut infos = Vec::new();
    collect_navmesh_info_inputs(
        &slot.parsed.root_items,
        &navmesh_ids,
        &mut infos,
        &mut stats,
    );
    infos.sort_by_key(|info| info.form_id);
    if infos.is_empty() {
        if stats.records_removed > 0 {
            slot.apply_write_effect(&WriteEffect::RecordsAddedOrRemoved);
        }
        return Ok(stats);
    }

    let mut used_object_ids = BTreeSet::new();
    collect_record_object_ids(&slot.parsed.root_items, &mut used_object_ids);
    let mut used_form_ids = HashSet::new();
    collect_record_form_ids(&slot.parsed.root_items, &mut used_form_ids);
    let preferred_form_id = match slot.parsed.game.as_deref() {
        Some("fo4") => Some(FO4_CANONICAL_NAVI_FORM_ID),
        _ => preferred_form_id,
    };
    let navi_form_id = choose_projected_navi_form_id(
        &mut slot.parsed.header,
        existing_navi_form_id,
        preferred_form_id,
        &mut used_object_ids,
        &used_form_ids,
    );
    let mut subrecords = Vec::with_capacity(infos.len() + 2);
    subrecords.push(ParsedSubrecord {
        signature: SmolStr::new_static("NVER"),
        data: Bytes::from(15u32.to_le_bytes().to_vec()),
        semantic_type: None,
    });
    for info in &infos {
        stats.navmesh_infos += 1;
        stats.edge_links += info.edge_links.len() as u32;
        subrecords.push(ParsedSubrecord {
            signature: SmolStr::new_static("NVMI"),
            data: Bytes::from(encode_nvmi(info)),
            semantic_type: None,
        });
    }
    subrecords.push(ParsedSubrecord {
        signature: SmolStr::new_static("NVPP"),
        data: Bytes::from(vec![0u8; 8]),
        semantic_type: None,
    });

    let record = ParsedRecord {
        signature: SmolStr::new_static("NAVI"),
        form_id: navi_form_id,
        flags: 0,
        version_control: 0,
        form_version: match slot.parsed.game.as_deref() {
            Some("fo4") => Some(131),
            _ => None,
        },
        version2: None,
        subrecords,
        raw_payload: None,
        parse_error: None,
    };
    insert_parsed_record_in_slot(slot, record);
    if stats.records_removed > 0 {
        stats.records_replaced = 1;
    } else {
        stats.records_added = 1;
    }
    slot.apply_write_effect(&WriteEffect::RecordsAddedOrRemoved);
    Ok(stats)
}

pub fn rebuild_projected_navi_record_from_source_in_slot(
    slot: &mut NativePluginSlot,
    source_root_items: &[ParsedItem],
    source_to_target_formids: &[(u32, u32)],
    preferred_form_id: Option<u32>,
) -> Result<NaviRebuildStats, String> {
    rebuild_projected_navi_record_from_source_in_slot_with_nver(
        slot,
        source_root_items,
        source_to_target_formids,
        preferred_form_id,
        None,
    )
}

pub fn rebuild_projected_navi_record_from_source_in_slot_with_nver(
    slot: &mut NativePluginSlot,
    source_root_items: &[ParsedItem],
    source_to_target_formids: &[(u32, u32)],
    preferred_form_id: Option<u32>,
    target_nver: Option<u32>,
) -> Result<NaviRebuildStats, String> {
    let Some(source_navi) = find_first_record_by_signature(source_root_items, "NAVI") else {
        return rebuild_projected_navi_record_in_slot(slot, preferred_form_id);
    };
    normalize_finalized_fo4_navmesh_versions(&mut slot.parsed.root_items)?;

    let mut stats = NaviRebuildStats::default();
    let mut navmesh_ids = HashSet::new();
    collect_navmesh_form_ids(&slot.parsed.root_items, &mut navmesh_ids);
    if navmesh_ids.is_empty() {
        stats.records_removed =
            remove_records_by_signature(&mut slot.parsed.root_items, "NAVI") as u32;
        if stats.records_removed > 0 {
            slot.apply_write_effect(&WriteEffect::RecordsAddedOrRemoved);
        }
        return Ok(stats);
    }

    let mapping: HashMap<u32, u32> = source_to_target_formids.iter().copied().collect();
    // Emitted-REFR index for door-link validation. A source NVMI door link
    // remaps its Door Ref through `mapping`, but the target REFR may not have
    // been emitted into this projected slice (cross-slice doors to interiors
    // outside the converted worldspace). A door link to a non-emitted REFR is a
    // wild pointer the FO4 engine dereferences on cell entry -> CTD. Keep a door
    // link only when its remapped ref is an emitted REFR in this slot.
    let mut refr_ids = HashSet::new();
    collect_refr_form_ids(&slot.parsed.root_items, &mut refr_ids);
    // Rebuild edge_links for each target NAVM from its (already-finalized)
    // NVNM bytes; this replaces the source NVMI.edge_links list during the
    // per-NVMI remap below. Preserving FO76 source NVMI.edge_links produces a
    // strict superset of
    // the actual finalized topology and tripped CK's PATHFINDING validator.
    let target_navmesh_edge_links =
        collect_target_navmesh_edge_links(&slot.parsed.root_items, &navmesh_ids, &mut stats);
    let source_subrecords = effective_subrecords_for_record(source_navi);
    let mut nver = None;
    let mut nvpp = None;
    // Serial pre-scan preserves the legacy first-NVER / first-NVPP picks and
    // the NVMI subrecord order; the per-NVMI remaps (pure over the shared
    // read-only maps) run in parallel with per-item local stats, then fold
    // serially in subrecord order.
    let mut nvmi_inputs: Vec<&ParsedSubrecord> = Vec::new();
    for subrecord in source_subrecords.iter() {
        match subrecord.signature.as_str() {
            "NVER" if nver.is_none() => nver = Some(subrecord.clone()),
            "NVMI" => nvmi_inputs.push(subrecord),
            "NVPP" if nvpp.is_none() => nvpp = Some(subrecord.clone()),
            _ => {}
        }
    }
    let remapped: Vec<(
        Result<Option<Vec<u8>>, String>,
        NaviRebuildStats,
        Option<String>,
    )> = {
        use rayon::prelude::*;
        nvmi_inputs
            .par_iter()
            .map(|subrecord| {
                let mut local = NaviRebuildStats::default();
                let result = remap_source_nvmi_for_projected_navi(
                    subrecord.data.as_ref(),
                    &mapping,
                    &navmesh_ids,
                    &refr_ids,
                    &target_navmesh_edge_links,
                    &mut local,
                );
                (result, local, subrecord.semantic_type.clone())
            })
            .collect()
    };
    let mut nvmis = Vec::new();
    for (result, local, semantic_type) in remapped {
        merge_navi_stats(&mut stats, &local);
        match result {
            Ok(Some(data)) => {
                stats.navmesh_infos += 1;
                nvmis.push(ParsedSubrecord {
                    signature: SmolStr::new_static("NVMI"),
                    data: Bytes::from(data),
                    semantic_type,
                });
            }
            Ok(None) => {}
            Err(_) => stats.warnings += 1,
        }
    }

    if nvmis.is_empty() {
        stats.records_removed =
            remove_records_by_signature(&mut slot.parsed.root_items, "NAVI") as u32;
        if stats.records_removed > 0 {
            slot.apply_write_effect(&WriteEffect::RecordsAddedOrRemoved);
        }
        return Ok(stats);
    }

    let existing_navi_form_id =
        find_first_record_form_id_by_signature(&slot.parsed.root_items, "NAVI");
    stats.records_removed = remove_records_by_signature(&mut slot.parsed.root_items, "NAVI") as u32;

    let mut used_object_ids = BTreeSet::new();
    collect_record_object_ids(&slot.parsed.root_items, &mut used_object_ids);
    let mut used_form_ids = HashSet::new();
    collect_record_form_ids(&slot.parsed.root_items, &mut used_form_ids);
    let preferred_form_id = match slot.parsed.game.as_deref() {
        Some("fo4") => Some(FO4_CANONICAL_NAVI_FORM_ID),
        _ => preferred_form_id,
    };
    let navi_form_id = choose_projected_navi_form_id(
        &mut slot.parsed.header,
        existing_navi_form_id,
        preferred_form_id,
        &mut used_object_ids,
        &used_form_ids,
    );

    let target_nver = if slot.parsed.game.as_deref() == Some("fo4") {
        Some(15)
    } else {
        target_nver
    };
    let mut subrecords = Vec::with_capacity(nvmis.len() + 2);
    subrecords.push(
        target_nver
            .map(|version| ParsedSubrecord {
                signature: SmolStr::new_static("NVER"),
                data: Bytes::from(version.to_le_bytes().to_vec()),
                semantic_type: None,
            })
            .or(nver)
            .unwrap_or_else(|| ParsedSubrecord {
                signature: SmolStr::new_static("NVER"),
                data: Bytes::from(15u32.to_le_bytes().to_vec()),
                semantic_type: None,
            }),
    );
    subrecords.extend(nvmis);
    let nvpp = if target_nver == Some(15) {
        // Skyrim NVPP embeds source NAVM FormIDs. Until its full layout is
        // translated, an empty FO4 NVPP is valid and safer than stale pointers.
        None
    } else {
        nvpp
    };
    subrecords.push(nvpp.unwrap_or_else(|| ParsedSubrecord {
        signature: SmolStr::new_static("NVPP"),
        data: Bytes::from(vec![0u8; 8]),
        semantic_type: None,
    }));

    let record = ParsedRecord {
        signature: SmolStr::new_static("NAVI"),
        form_id: navi_form_id,
        flags: 0,
        version_control: 0,
        form_version: match slot.parsed.game.as_deref() {
            Some("fo4") => Some(131),
            _ => None,
        },
        version2: None,
        subrecords,
        raw_payload: None,
        parse_error: None,
    };
    insert_parsed_record_in_slot(slot, record);
    if stats.records_removed > 0 {
        stats.records_replaced = 1;
    } else {
        stats.records_added = 1;
    }
    slot.apply_write_effect(&WriteEffect::RecordsAddedOrRemoved);
    Ok(stats)
}

fn find_first_record_by_signature<'a>(
    items: &'a [ParsedItem],
    signature: &str,
) -> Option<&'a ParsedRecord> {
    for item in items {
        match item {
            ParsedItem::Record(record) if record.signature.as_str() == signature => {
                return Some(record);
            }
            ParsedItem::Group(group) => {
                if let Some(record) = find_first_record_by_signature(&group.children, signature) {
                    return Some(record);
                }
            }
            _ => {}
        }
    }
    None
}

fn collect_navmesh_form_ids(items: &[ParsedItem], out: &mut HashSet<u32>) {
    for item in items {
        match item {
            ParsedItem::Record(record) if record.signature.as_str() == "NAVM" => {
                out.insert(record.form_id);
            }
            ParsedItem::Group(group) => collect_navmesh_form_ids(&group.children, out),
            _ => {}
        }
    }
}

/// Collect every emitted REFR form_id in the slot, used to validate NVMI
/// door-link Door Refs (a door link to a non-emitted REFR is a wild pointer).
fn collect_refr_form_ids(items: &[ParsedItem], out: &mut HashSet<u32>) {
    for item in items {
        match item {
            ParsedItem::Record(record) if record.signature.as_str() == "REFR" => {
                out.insert(record.form_id);
            }
            ParsedItem::Group(group) => collect_refr_form_ids(&group.children, out),
            _ => {}
        }
    }
}

/// For each NAVM record in `items`, parse its NVNM payload's edge_links
/// table and return a map from NAVM form_id -> sorted unique list of
/// referenced (emitted) navmesh form_ids. NAVMs whose NVNM parse fails are
/// skipped silently (the per-NVMI handler will see an empty list and emit
/// an empty NVMI.edge_links section for them, which is the correct
/// behaviour for a NAVM with no cross-navmesh edges).
fn collect_target_navmesh_edge_links(
    items: &[ParsedItem],
    emitted_navmesh_ids: &HashSet<u32>,
    stats: &mut NaviRebuildStats,
) -> HashMap<u32, Vec<u32>> {
    use rayon::prelude::*;
    // Serial walk gathers NVNM payloads in walk order (Bytes clones are
    // refcount bumps); the parses run in parallel with per-item local stats;
    // the serial fold merges stats and inserts in walk order, matching the
    // legacy serial recursion (stats are commutative u32 sums).
    let mut gathered: Vec<(u32, Bytes)> = Vec::new();
    gather_navmesh_nvnm_for_navi(items, &mut gathered);
    let parsed: Vec<(u32, Result<NvnmNaviParts, String>, NaviRebuildStats)> = gathered
        .par_iter()
        .map(|(form_id, nvnm)| {
            let mut local = NaviRebuildStats::default();
            let result = parse_nvnm_for_navi(nvnm.as_ref(), emitted_navmesh_ids, &mut local);
            (*form_id, result, local)
        })
        .collect();
    let mut out = HashMap::new();
    for (form_id, result, local) in parsed {
        merge_navi_stats(stats, &local);
        match result {
            Ok(parts) => {
                out.insert(form_id, parts.edge_links);
            }
            Err(_) => {
                stats.warnings += 1;
            }
        }
    }
    out
}

fn gather_navmesh_nvnm_for_navi(items: &[ParsedItem], out: &mut Vec<(u32, Bytes)>) {
    for item in items {
        match item {
            ParsedItem::Record(record) if record.signature.as_str() == "NAVM" => {
                let subrecords = effective_subrecords_for_record(record);
                let Some(nvnm) = subrecords
                    .iter()
                    .find(|s| s.signature.as_str() == "NVNM")
                    .map(|s| s.data.clone())
                else {
                    continue;
                };
                out.push((record.form_id, nvnm));
            }
            ParsedItem::Group(group) => gather_navmesh_nvnm_for_navi(&group.children, out),
            _ => {}
        }
    }
}

fn find_first_record_form_id_by_signature(items: &[ParsedItem], signature: &str) -> Option<u32> {
    for item in items {
        match item {
            ParsedItem::Record(record) if record.signature.as_str() == signature => {
                return Some(record.form_id);
            }
            ParsedItem::Group(group) => {
                if let Some(form_id) =
                    find_first_record_form_id_by_signature(&group.children, signature)
                {
                    return Some(form_id);
                }
            }
            _ => {}
        }
    }
    None
}

fn remove_records_by_signature(items: &mut Vec<ParsedItem>, signature: &str) -> usize {
    let mut removed = 0usize;
    let mut i = 0usize;
    while i < items.len() {
        match &mut items[i] {
            ParsedItem::Record(record) if record.signature.as_str() == signature => {
                items.remove(i);
                removed += 1;
            }
            ParsedItem::Group(group) => {
                removed += remove_records_by_signature(&mut group.children, signature);
                i += 1;
            }
            _ => i += 1,
        }
    }
    removed
}

fn collect_navmesh_info_inputs(
    items: &[ParsedItem],
    emitted_navmesh_ids: &HashSet<u32>,
    out: &mut Vec<NavmeshInfoInput>,
    stats: &mut NaviRebuildStats,
) {
    for item in items {
        match item {
            ParsedItem::Record(record) if record.signature.as_str() == "NAVM" => {
                match navmesh_info_input_from_record(record, emitted_navmesh_ids, stats) {
                    Ok(Some(info)) => out.push(info),
                    Ok(None) => {}
                    Err(_) => stats.warnings += 1,
                }
            }
            ParsedItem::Group(group) => {
                collect_navmesh_info_inputs(&group.children, emitted_navmesh_ids, out, stats)
            }
            _ => {}
        }
    }
}

/// Normalize FO4's version and PathingCell markers on otherwise valid NVNM
/// payloads. A true legacy layout is rejected before any target record is
/// changed; stamping target markers onto bytes that do not fully parse as FO4
/// would merely hide corruption and make the NAVI rebuild unsafe.
fn normalize_finalized_fo4_navmesh_versions(items: &mut [ParsedItem]) -> Result<(), String> {
    let mut errors = Vec::new();
    collect_non_fo4_navmesh_version_errors(items, &mut errors);
    if !errors.is_empty() {
        errors.sort();
        return Err(format!(
            "finalized target NAVM contains non-normalizable NVNM: {}",
            errors.join("; ")
        ));
    }
    normalize_fo4_navmesh_markers(items);
    Ok(())
}

fn collect_non_fo4_navmesh_version_errors(items: &[ParsedItem], errors: &mut Vec<String>) {
    for item in items {
        match item {
            ParsedItem::Record(record) if record.signature.as_str() == "NAVM" => {
                let subrecords = effective_subrecords_for_record(record);
                let Some(nvnm) = subrecords
                    .iter()
                    .find(|subrecord| subrecord.signature.as_str() == "NVNM")
                    .map(|subrecord| subrecord.data.as_ref())
                else {
                    continue;
                };
                if nvnm.is_empty() {
                    continue;
                }
                if nvnm.len() < 8 {
                    errors.push(format!(
                        "NAVM {:08X} NVNM has {} bytes; FO4 markers require 8",
                        record.form_id,
                        nvnm.len()
                    ));
                    continue;
                }
                let version = u32::from_le_bytes(nvnm[0..4].try_into().unwrap());
                if version == 15 {
                    continue;
                }
                let mut normalized = nvnm.to_vec();
                normalized[0..4].copy_from_slice(&15u32.to_le_bytes());
                if let Err(error) = crate::nvnm::parse_nvnm(&normalized) {
                    errors.push(format!(
                        "NAVM {:08X} NVNM version {}: {}",
                        record.form_id, version, error
                    ));
                }
            }
            ParsedItem::Group(group) => {
                collect_non_fo4_navmesh_version_errors(&group.children, errors)
            }
            _ => {}
        }
    }
}

fn normalize_fo4_navmesh_markers(items: &mut [ParsedItem]) {
    for item in items {
        match item {
            ParsedItem::Record(record) if record.signature.as_str() == "NAVM" => {
                let mut subrecords = effective_subrecords_for_record(record).into_owned();
                let mut changed = false;
                for subrecord in &mut subrecords {
                    if subrecord.signature.as_str() != "NVNM" || subrecord.data.len() < 8 {
                        continue;
                    }
                    let version = u32::from_le_bytes(subrecord.data[0..4].try_into().unwrap());
                    let pathing_cell_crc =
                        u32::from_le_bytes(subrecord.data[4..8].try_into().unwrap());
                    if version == 15 && pathing_cell_crc == FO4_PATHING_CELL_CRC_HASH {
                        continue;
                    }
                    let mut data = subrecord.data.to_vec();
                    data[0..4].copy_from_slice(&15u32.to_le_bytes());
                    data[4..8].copy_from_slice(&FO4_PATHING_CELL_CRC_HASH.to_le_bytes());
                    subrecord.data = Bytes::from(data);
                    changed = true;
                }
                if changed {
                    record.subrecords = subrecords;
                    record.raw_payload = None;
                }
            }
            ParsedItem::Group(group) => normalize_fo4_navmesh_markers(&mut group.children),
            _ => {}
        }
    }
}

fn navmesh_info_input_from_record(
    record: &ParsedRecord,
    emitted_navmesh_ids: &HashSet<u32>,
    stats: &mut NaviRebuildStats,
) -> Result<Option<NavmeshInfoInput>, String> {
    let subrecords = effective_subrecords_for_record(record);
    let Some(nvnm) = subrecords
        .iter()
        .find(|subrecord| subrecord.signature.as_str() == "NVNM")
        .map(|subrecord| subrecord.data.as_ref())
    else {
        return Ok(None);
    };
    if nvnm.is_empty() {
        return Ok(None);
    }
    let parent = match navmesh_parent_from_record(record)? {
        Some(parent) => parent,
        None => return Ok(None),
    };
    let parsed = parse_nvnm_for_navi(nvnm, emitted_navmesh_ids, stats)?;
    Ok(Some(NavmeshInfoInput {
        form_id: record.form_id,
        pathing_cell_crc_hash: parsed.pathing_cell_crc_hash,
        parent,
        approx_location: parsed.approx_location,
        edge_links: parsed.edge_links,
        island_data: parsed.island_data,
    }))
}

#[derive(Debug, Clone)]
struct NvnmNaviParts {
    pathing_cell_crc_hash: u32,
    approx_location: (f32, f32, f32),
    edge_links: Vec<u32>,
    island_data: Option<NavmeshIslandData>,
}

fn parse_nvnm_for_navi(
    data: &[u8],
    emitted_navmesh_ids: &HashSet<u32>,
    stats: &mut NaviRebuildStats,
) -> Result<NvnmNaviParts, String> {
    if data.len() < 16 {
        return Err(format!(
            "NVNM payload is {} bytes; expected at least 16",
            data.len()
        ));
    }
    let version = read_u32_le(data, 0, "NVNM version")?;
    if version != 15 {
        return Err(format!(
            "finalized target NVNM version is {version}; expected 15"
        ));
    }
    let pathing_cell_crc_hash = read_u32_le(data, 4, "NVNM pathing cell CRC")?;
    let mut offset = 16usize;
    let vertices = read_vertices(data, &mut offset)?;
    let approx_location = vertex_centroid(&vertices);
    let triangles = read_triangle_vertex_indices(data, &mut offset)?;
    let edge_links = read_filtered_edge_links(data, &mut offset, emitted_navmesh_ids, stats)?;
    let island_data = navmesh_island_data(vertices, triangles);
    Ok(NvnmNaviParts {
        pathing_cell_crc_hash,
        approx_location,
        edge_links,
        island_data,
    })
}

fn read_vertices(data: &[u8], offset: &mut usize) -> Result<Vec<(f32, f32, f32)>, String> {
    let count = read_count_le(data, offset, "NVNM vertices")?;
    let rows_start = *offset;
    let rows_end = checked_rows_end_plugin(rows_start, count, 12, data.len(), "NVNM vertices")?;
    let mut vertices = Vec::with_capacity(count);
    for index in 0..count {
        let row = rows_start + index * 12;
        vertices.push((
            read_f32_le(data, row, "NVNM vertex x")?,
            read_f32_le(data, row + 4, "NVNM vertex y")?,
            read_f32_le(data, row + 8, "NVNM vertex z")?,
        ));
    }
    *offset = rows_end;
    Ok(vertices)
}

fn read_triangle_vertex_indices(data: &[u8], offset: &mut usize) -> Result<Vec<[u16; 3]>, String> {
    let count = read_count_le(data, offset, "NVNM triangles")?;
    let rows_start = *offset;
    let rows_end = checked_rows_end_plugin(rows_start, count, 21, data.len(), "NVNM triangles")?;
    let mut triangles = Vec::with_capacity(count);
    for index in 0..count {
        let row = rows_start + index * 21;
        triangles.push([
            read_u16_le(data, row, "NVNM triangle vertex 0")?,
            read_u16_le(data, row + 2, "NVNM triangle vertex 1")?,
            read_u16_le(data, row + 4, "NVNM triangle vertex 2")?,
        ]);
    }
    *offset = rows_end;
    Ok(triangles)
}

fn vertex_centroid(vertices: &[(f32, f32, f32)]) -> (f32, f32, f32) {
    if vertices.is_empty() {
        return (0.0, 0.0, 0.0);
    }
    let mut x = 0.0f64;
    let mut y = 0.0f64;
    let mut z = 0.0f64;
    for vertex in vertices {
        x += vertex.0 as f64;
        y += vertex.1 as f64;
        z += vertex.2 as f64;
    }
    let denom = vertices.len() as f64;
    ((x / denom) as f32, (y / denom) as f32, (z / denom) as f32)
}

fn navmesh_island_data(
    vertices: Vec<(f32, f32, f32)>,
    triangles: Vec<[u16; 3]>,
) -> Option<NavmeshIslandData> {
    let (min, max) = navmesh_vertex_bounds(&vertices)?;
    let (triangles, vertices) = limited_navmesh_island_geometry(&vertices, &triangles, min, max);
    Some(NavmeshIslandData {
        min,
        max,
        triangles,
        vertices,
    })
}

fn navmesh_vertex_bounds(
    vertices: &[(f32, f32, f32)],
) -> Option<((f32, f32, f32), (f32, f32, f32))> {
    let first = vertices.first().copied()?;
    let mut min = first;
    let mut max = first;
    for &(x, y, z) in &vertices[1..] {
        min.0 = min.0.min(x);
        min.1 = min.1.min(y);
        min.2 = min.2.min(z);
        max.0 = max.0.max(x);
        max.1 = max.1.max(y);
        max.2 = max.2.max(z);
    }
    Some((min, max))
}

fn limited_navmesh_island_geometry(
    vertices: &[(f32, f32, f32)],
    triangles: &[[u16; 3]],
    min: (f32, f32, f32),
    max: (f32, f32, f32),
) -> (Vec<[u16; 3]>, Vec<(f32, f32, f32)>) {
    if !triangles.is_empty()
        && triangles.len() <= NAVI_ISLAND_TRIANGLE_LIMIT
        && vertices.len() <= NAVI_ISLAND_VERTEX_LIMIT
    {
        return (triangles.to_vec(), vertices.to_vec());
    }

    let sampled = sample_navmesh_island_geometry(vertices, triangles);
    if !sampled.0.is_empty() && !sampled.1.is_empty() {
        return sampled;
    }

    bounding_box_island_geometry(min, max)
}

fn sample_navmesh_island_geometry(
    vertices: &[(f32, f32, f32)],
    triangles: &[[u16; 3]],
) -> (Vec<[u16; 3]>, Vec<(f32, f32, f32)>) {
    if vertices.is_empty() || triangles.is_empty() {
        return (Vec::new(), Vec::new());
    }

    let stride = triangles.len().div_ceil(NAVI_ISLAND_TRIANGLE_LIMIT).max(1);
    let mut remap: HashMap<u16, u16> = HashMap::new();
    let mut sampled_vertices = Vec::with_capacity(vertices.len().min(NAVI_ISLAND_VERTEX_LIMIT));
    let mut sampled_triangles = Vec::with_capacity(triangles.len().min(NAVI_ISLAND_TRIANGLE_LIMIT));

    for triangle in triangles.iter().step_by(stride) {
        if sampled_triangles.len() >= NAVI_ISLAND_TRIANGLE_LIMIT {
            break;
        }
        let missing_vertices = triangle
            .iter()
            .filter(|vertex_index| {
                !remap.contains_key(vertex_index) && (**vertex_index as usize) < vertices.len()
            })
            .count();
        if sampled_vertices.len() + missing_vertices > NAVI_ISLAND_VERTEX_LIMIT {
            continue;
        }

        let mut remapped = [0u16; 3];
        let mut valid = true;
        for (slot, source_index) in triangle.iter().enumerate() {
            if (*source_index as usize) >= vertices.len() {
                valid = false;
                break;
            }
            let mapped = match remap.get(source_index) {
                Some(mapped) => *mapped,
                None => {
                    let mapped = sampled_vertices.len() as u16;
                    remap.insert(*source_index, mapped);
                    sampled_vertices.push(vertices[*source_index as usize]);
                    mapped
                }
            };
            remapped[slot] = mapped;
        }
        if valid {
            sampled_triangles.push(remapped);
        }
    }

    (sampled_triangles, sampled_vertices)
}

fn bounding_box_island_geometry(
    min: (f32, f32, f32),
    max: (f32, f32, f32),
) -> (Vec<[u16; 3]>, Vec<(f32, f32, f32)>) {
    let vertices = vec![
        (min.0, min.1, min.2),
        (max.0, min.1, min.2),
        (max.0, max.1, min.2),
        (min.0, max.1, min.2),
        (min.0, min.1, max.2),
        (max.0, min.1, max.2),
        (max.0, max.1, max.2),
        (min.0, max.1, max.2),
    ];
    let triangles = vec![
        [0, 1, 2],
        [0, 2, 3],
        [4, 6, 5],
        [4, 7, 6],
        [0, 4, 5],
        [0, 5, 1],
        [1, 5, 6],
        [1, 6, 2],
        [2, 6, 7],
        [2, 7, 3],
        [3, 7, 4],
        [3, 4, 0],
    ];
    (triangles, vertices)
}

fn read_filtered_edge_links(
    data: &[u8],
    offset: &mut usize,
    emitted_navmesh_ids: &HashSet<u32>,
    stats: &mut NaviRebuildStats,
) -> Result<Vec<u32>, String> {
    let count = read_count_le(data, offset, "NVNM edge links")?;
    let rows_start = *offset;
    let rows_end = checked_rows_end_plugin(rows_start, count, 11, data.len(), "NVNM edge links")?;
    let mut links = Vec::new();
    let mut seen = HashSet::new();
    for index in 0..count {
        let row = rows_start + index * 11;
        let linked_navmesh = read_u32_le(data, row + 4, "NVNM edge link navmesh")?;
        if linked_navmesh == 0 {
            continue;
        }
        if emitted_navmesh_ids.contains(&linked_navmesh) {
            if seen.insert(linked_navmesh) {
                links.push(linked_navmesh);
            }
        } else {
            stats.stale_edge_links_dropped += 1;
        }
    }
    *offset = rows_end;
    links.sort_unstable();
    Ok(links)
}

fn remap_source_nvmi_for_projected_navi(
    data: &[u8],
    source_to_target: &HashMap<u32, u32>,
    emitted_navmesh_ids: &HashSet<u32>,
    emitted_refr_ids: &HashSet<u32>,
    target_navmesh_edge_links: &HashMap<u32, Vec<u32>>,
    stats: &mut NaviRebuildStats,
) -> Result<Option<Vec<u8>>, String> {
    if data.len() < 49 {
        return Err(format!(
            "NVMI payload is {} bytes; expected at least 49",
            data.len()
        ));
    }

    let source_navmesh = read_u32_le(data, 0, "NVMI navmesh")?;
    let Some(&target_navmesh) = source_to_target.get(&source_navmesh) else {
        return Ok(None);
    };
    if !emitted_navmesh_ids.contains(&target_navmesh) {
        return Ok(None);
    }

    let mut offset = 24usize;
    let mut out = Vec::with_capacity(data.len());
    out.extend_from_slice(&target_navmesh.to_le_bytes());
    out.extend_from_slice(&data[4..24]);
    // NVMI.edge_links MUST be rebuilt from the finalized target NVNM's
    // edge_links table — preserving the FO76 source NVMI.edge_links (which
    // is what `remap_nvmi_navmesh_array` would do) produces a STRICT
    // SUPERSET of the target NVNM's actual cross-navmesh edges, because
    // FO76 NVMI can list neighbours that no longer share a portal after
    // FO4 finalize.
    skip_nvmi_navmesh_array(data, &mut offset, "NVMI edge links")?;
    let rebuilt_edge_links = target_navmesh_edge_links
        .get(&target_navmesh)
        .cloned()
        .unwrap_or_default();
    stats.edge_links += rebuilt_edge_links.len() as u32;
    out.extend_from_slice(&(rebuilt_edge_links.len() as u32).to_le_bytes());
    for link in rebuilt_edge_links {
        out.extend_from_slice(&link.to_le_bytes());
    }
    remap_nvmi_navmesh_array(
        data,
        &mut offset,
        &mut out,
        "NVMI preferred edge links",
        source_to_target,
        emitted_navmesh_ids,
        stats,
    )?;
    remap_nvmi_door_links(
        data,
        &mut offset,
        &mut out,
        source_to_target,
        emitted_refr_ids,
        stats,
    )?;

    let has_island_data = *data
        .get(offset)
        .ok_or_else(|| "NVMI missing island-data selector".to_string())?;
    out.push(has_island_data);
    offset += 1;
    if has_island_data != 0 {
        let island_start = offset;
        offset = checked_rows_end_plugin(offset, 1, 24, data.len(), "NVMI island bounds")?;
        out.extend_from_slice(&data[island_start..offset]);
        // Preserve island triangles+vertices verbatim. CK keeps the full FO76
        // island geometry through Finalize; an empty-array drop makes our NVMI
        // bytes diverge from CK's.
        let tri_start = offset;
        let triangle_count = read_u32_le(data, offset, "NVMI island triangles")? as usize;
        offset += 4;
        offset = checked_rows_end_plugin(
            offset,
            triangle_count,
            6,
            data.len(),
            "NVMI island triangles",
        )?;
        out.extend_from_slice(&data[tri_start..offset]);
        let vert_start = offset;
        let vertex_count = read_u32_le(data, offset, "NVMI island vertices")? as usize;
        offset += 4;
        offset =
            checked_rows_end_plugin(offset, vertex_count, 12, data.len(), "NVMI island vertices")?;
        out.extend_from_slice(&data[vert_start..offset]);
    }

    let _source_crc_hash = read_u32_le(data, offset, "NVMI pathing cell CRC")?;
    offset += 4;
    let parent_world = read_u32_le(data, offset, "NVMI parent world")?;
    offset += 4;
    out.extend_from_slice(&FO4_PATHING_CELL_CRC_HASH.to_le_bytes());
    if parent_world == 0 {
        out.extend_from_slice(&0u32.to_le_bytes());
        let parent_cell = read_u32_le(data, offset, "NVMI parent cell")?;
        offset += 4;
        out.extend_from_slice(
            &source_to_target
                .get(&parent_cell)
                .copied()
                .unwrap_or(parent_cell)
                .to_le_bytes(),
        );
    } else {
        out.extend_from_slice(
            &source_to_target
                .get(&parent_world)
                .copied()
                .unwrap_or(parent_world)
                .to_le_bytes(),
        );
        let cell_coords_end =
            checked_rows_end_plugin(offset, 1, 4, data.len(), "NVMI cell coords")?;
        out.extend_from_slice(&data[offset..cell_coords_end]);
        offset = cell_coords_end;
    }

    if offset != data.len() {
        return Err(format!(
            "NVMI parser ended at {offset}, expected len {}",
            data.len()
        ));
    }
    Ok(Some(out))
}

/// Advance `offset` past a length-prefixed array of u32 navmesh form_ids
/// WITHOUT emitting anything to `out`. Used by callers that replace the
/// source-derived list with a freshly-computed one.
fn skip_nvmi_navmesh_array(data: &[u8], offset: &mut usize, label: &str) -> Result<(), String> {
    let count = read_count_le(data, offset, label)?;
    let rows_start = *offset;
    *offset = checked_rows_end_plugin(rows_start, count, 4, data.len(), label)?;
    Ok(())
}

fn remap_nvmi_navmesh_array(
    data: &[u8],
    offset: &mut usize,
    out: &mut Vec<u8>,
    label: &str,
    source_to_target: &HashMap<u32, u32>,
    emitted_navmesh_ids: &HashSet<u32>,
    stats: &mut NaviRebuildStats,
) -> Result<(), String> {
    let count = read_count_le(data, offset, label)?;
    let rows_start = *offset;
    let rows_end = checked_rows_end_plugin(rows_start, count, 4, data.len(), label)?;
    let mut links = Vec::new();
    for index in 0..count {
        let row = rows_start + index * 4;
        let source_navmesh = read_u32_le(data, row, label)?;
        let Some(&target_navmesh) = source_to_target.get(&source_navmesh) else {
            stats.stale_edge_links_dropped += 1;
            continue;
        };
        if emitted_navmesh_ids.contains(&target_navmesh) {
            links.push(target_navmesh);
        } else {
            stats.stale_edge_links_dropped += 1;
        }
    }
    links.sort_unstable();
    links.dedup();
    stats.edge_links += links.len() as u32;
    out.extend_from_slice(&(links.len() as u32).to_le_bytes());
    for link in links {
        out.extend_from_slice(&link.to_le_bytes());
    }
    *offset = rows_end;
    Ok(())
}

fn remap_nvmi_door_links(
    data: &[u8],
    offset: &mut usize,
    out: &mut Vec<u8>,
    source_to_target: &HashMap<u32, u32>,
    emitted_refr_ids: &HashSet<u32>,
    stats: &mut NaviRebuildStats,
) -> Result<(), String> {
    let count = read_count_le(data, offset, "NVMI door links")?;
    let rows_start = *offset;
    let rows_end = checked_rows_end_plugin(rows_start, count, 8, data.len(), "NVMI door links")?;
    let mut links = Vec::new();
    for index in 0..count {
        let row = rows_start + index * 8;
        let crc_hash = read_u32_le(data, row, "NVMI door link CRC")?;
        let source_ref = read_u32_le(data, row + 4, "NVMI door link ref")?;
        // Keep a door link only when the Door Ref both remaps AND resolves to a
        // REFR actually emitted into this slice. A remapped-but-not-emitted ref
        // (cross-slice door whose REFR was never copied) is a wild pointer that
        // CTDs FO4 on cell entry, so drop the row instead.
        match source_to_target.get(&source_ref) {
            Some(&target_ref) if emitted_refr_ids.contains(&target_ref) => {
                links.push((crc_hash, target_ref));
            }
            _ => {
                stats.stale_edge_links_dropped += 1;
            }
        }
    }
    out.extend_from_slice(&(links.len() as u32).to_le_bytes());
    for (crc_hash, target_ref) in links {
        out.extend_from_slice(&crc_hash.to_le_bytes());
        out.extend_from_slice(&target_ref.to_le_bytes());
    }
    *offset = rows_end;
    Ok(())
}

fn read_count_le(data: &[u8], offset: &mut usize, label: &str) -> Result<usize, String> {
    let count = read_u32_le(data, *offset, label)? as usize;
    *offset += 4;
    Ok(count)
}

fn checked_rows_end_plugin(
    offset: usize,
    count: usize,
    row_size: usize,
    len: usize,
    label: &str,
) -> Result<usize, String> {
    let bytes = count
        .checked_mul(row_size)
        .ok_or_else(|| format!("{label} row byte count overflow"))?;
    let end = offset
        .checked_add(bytes)
        .ok_or_else(|| format!("{label} row end overflow"))?;
    if end > len {
        return Err(format!(
            "{label} rows exceed payload: offset={offset} count={count} row_size={row_size} len={len}"
        ));
    }
    Ok(end)
}

fn read_u32_le(data: &[u8], offset: usize, label: &str) -> Result<u32, String> {
    let end = offset + 4;
    if end > data.len() {
        return Err(format!(
            "{label} u32 at offset {offset} exceeds len {}",
            data.len()
        ));
    }
    Ok(u32::from_le_bytes(data[offset..end].try_into().unwrap()))
}

fn read_u16_le(data: &[u8], offset: usize, label: &str) -> Result<u16, String> {
    let end = offset + 2;
    if end > data.len() {
        return Err(format!(
            "{label} u16 at offset {offset} exceeds len {}",
            data.len()
        ));
    }
    Ok(u16::from_le_bytes(data[offset..end].try_into().unwrap()))
}

fn read_f32_le(data: &[u8], offset: usize, label: &str) -> Result<f32, String> {
    Ok(f32::from_bits(read_u32_le(data, offset, label)?))
}

fn encode_nvmi(info: &NavmeshInfoInput) -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(&info.form_id.to_le_bytes());
    let flags = if info.island_data.is_some() {
        NAVI_NVMI_FLAG_IS_ISLAND
    } else {
        0
    };
    data.extend_from_slice(&flags.to_le_bytes());
    data.extend_from_slice(&info.approx_location.0.to_le_bytes());
    data.extend_from_slice(&info.approx_location.1.to_le_bytes());
    data.extend_from_slice(&info.approx_location.2.to_le_bytes());
    data.extend_from_slice(&0.0f32.to_le_bytes());
    data.extend_from_slice(&(info.edge_links.len() as u32).to_le_bytes());
    for linked_navmesh in &info.edge_links {
        data.extend_from_slice(&linked_navmesh.to_le_bytes());
    }
    data.extend_from_slice(&0u32.to_le_bytes());
    data.extend_from_slice(&0u32.to_le_bytes());
    match &info.island_data {
        Some(island) => {
            data.push(1);
            encode_navmesh_island_data(&mut data, island);
        }
        None => data.push(0),
    }
    data.extend_from_slice(&info.pathing_cell_crc_hash.to_le_bytes());
    match info.parent {
        NavmeshParent::Exterior {
            world_form_id,
            x,
            y,
        } => {
            data.extend_from_slice(&world_form_id.to_le_bytes());
            data.extend_from_slice(&y.to_le_bytes());
            data.extend_from_slice(&x.to_le_bytes());
        }
        NavmeshParent::Interior { cell_form_id } => {
            data.extend_from_slice(&0u32.to_le_bytes());
            data.extend_from_slice(&cell_form_id.to_le_bytes());
        }
    }
    data
}

fn encode_navmesh_island_data(data: &mut Vec<u8>, island: &NavmeshIslandData) {
    for value in [
        island.min.0,
        island.min.1,
        island.min.2,
        island.max.0,
        island.max.1,
        island.max.2,
    ] {
        data.extend_from_slice(&value.to_le_bytes());
    }
    data.extend_from_slice(&(island.triangles.len() as u32).to_le_bytes());
    for triangle in &island.triangles {
        for vertex_index in triangle {
            data.extend_from_slice(&vertex_index.to_le_bytes());
        }
    }
    data.extend_from_slice(&(island.vertices.len() as u32).to_le_bytes());
    for vertex in &island.vertices {
        data.extend_from_slice(&vertex.0.to_le_bytes());
        data.extend_from_slice(&vertex.1.to_le_bytes());
        data.extend_from_slice(&vertex.2.to_le_bytes());
    }
}

fn choose_projected_navi_form_id(
    header: &mut ParsedPluginHeader,
    existing_navi_form_id: Option<u32>,
    preferred_form_id: Option<u32>,
    used_object_ids: &mut BTreeSet<u32>,
    used_form_ids: &HashSet<u32>,
) -> u32 {
    // Official FO4 DLC masters keep the top-level NAVI raw FormID as 00000FF1;
    // rewriting it to the file's own master index makes CK report a duplicate map.
    if let Some(form_id) = preferred_form_id {
        let object_id = form_id & 0x00FF_FFFF;
        let canonical_fo4_override =
            form_id == FO4_CANONICAL_NAVI_FORM_ID && !used_form_ids.contains(&form_id);
        if object_id != 0 && (canonical_fo4_override || used_object_ids.insert(object_id)) {
            advance_next_object_id_past(header, object_id);
            return form_id;
        }
    }

    if let Some(form_id) = existing_navi_form_id {
        let object_id = form_id & 0x00FF_FFFF;
        let canonical_fo4_override =
            form_id == FO4_CANONICAL_NAVI_FORM_ID && !used_form_ids.contains(&form_id);
        if object_id != 0 && (canonical_fo4_override || used_object_ids.insert(object_id)) {
            advance_next_object_id_past(header, object_id);
            return form_id;
        }
    }

    allocate_unused_raw_form_id(header, used_object_ids)
}

fn allocate_unused_raw_form_id(
    header: &mut ParsedPluginHeader,
    used_object_ids: &mut BTreeSet<u32>,
) -> u32 {
    let own_index = header.masters.len() as u32;
    let mut object_id = (header.next_object_id & 0x00FF_FFFF).max(DEFAULT_SYNTHETIC_OBJECT_ID);
    while object_id == 0 || used_object_ids.contains(&object_id) {
        object_id = (object_id + 1) & 0x00FF_FFFF;
        if object_id == 0 {
            object_id = DEFAULT_SYNTHETIC_OBJECT_ID;
        }
    }
    used_object_ids.insert(object_id);
    advance_next_object_id_past(header, object_id);
    (own_index << 24) | object_id
}

fn advance_next_object_id_past(header: &mut ParsedPluginHeader, object_id: u32) {
    if object_id >= (header.next_object_id & 0x00FF_FFFF) {
        header.next_object_id = (object_id + 1) & 0x00FF_FFFF;
    }
}

fn find_projected_cell_world_form_id(
    root_items: &[ParsedItem],
    world_dir: &str,
    plugin_name: &str,
) -> Option<u32> {
    root_items.iter().find_map(|item| match item {
        ParsedItem::Group(group) if group.group_type == 0 && group.label == *b"WRLD" => {
            group.children.iter().find_map(|child| match child {
                ParsedItem::Record(record)
                    if record.signature == "WRLD"
                        && special_record_dir_name_native(record, plugin_name) == world_dir =>
                {
                    Some(record.form_id)
                }
                _ => None,
            })
        }
        _ => None,
    })
}

fn ensure_world_children_group(
    items: &mut Vec<ParsedItem>,
    world_form_id: u32,
    header_size: usize,
) -> &mut ParsedGroup {
    let label = world_form_id.to_le_bytes();
    if let Some(index) = items.iter().position(|item| {
        matches!(
            item,
            ParsedItem::Group(group) if group.group_type == 1 && group.label == label
        )
    }) {
        return match &mut items[index] {
            ParsedItem::Group(group) => group,
            _ => unreachable!(),
        };
    }

    let insert_index = items
        .iter()
        .position(|item| {
            matches!(
                item,
                ParsedItem::Record(record)
                    if record.signature == "WRLD" && record.form_id == world_form_id
            )
        })
        .map(|index| index + 1)
        .unwrap_or(items.len());
    items.insert(
        insert_index,
        ParsedItem::Group(ParsedGroup {
            label,
            group_type: 1,
            tail: Bytes::from(vec![0u8; header_size.saturating_sub(16)]),
            children: Vec::new(),
        }),
    );
    match &mut items[insert_index] {
        ParsedItem::Group(group) => group,
        _ => unreachable!(),
    }
}

fn ensure_exterior_grid_group(
    items: &mut Vec<ParsedItem>,
    group_type: i32,
    grid: (i16, i16),
    header_size: usize,
) -> &mut ParsedGroup {
    let label = encode_exterior_grid_label(grid.0, grid.1);
    if let Some(index) = items.iter().position(|item| {
        matches!(
            item,
            ParsedItem::Group(group) if group.group_type == group_type && group.label == label
        )
    }) {
        return match &mut items[index] {
            ParsedItem::Group(group) => group,
            _ => unreachable!(),
        };
    }
    items.push(ParsedItem::Group(ParsedGroup {
        label,
        group_type,
        tail: Bytes::from(vec![0u8; header_size.saturating_sub(16)]),
        children: Vec::new(),
    }));
    match items.last_mut().expect("inserted grid group") {
        ParsedItem::Group(group) => group,
        _ => unreachable!(),
    }
}

fn top_group_insert_index(
    root_items: &[ParsedItem],
    signature: &str,
    game: Option<&str>,
) -> Option<usize> {
    let order = top_level_group_order_for_game(game)?;
    let new_rank = group_order_rank(order, signature)?;
    root_items.iter().position(|item| {
        let ParsedItem::Group(group) = item else {
            return false;
        };
        if group.group_type != 0 {
            return false;
        }
        let Some(existing_sig) = std::str::from_utf8(&group.label).ok() else {
            return false;
        };
        group_order_rank(order, existing_sig).is_some_and(|rank| rank > new_rank)
    })
}

/// The canonical top-level GRUP signature order for a game's plugins, as the
/// resident serializer inserts groups (`top_group_insert_index`).
fn top_level_group_order_for_game(game: Option<&str>) -> Option<&'static [&'static str]> {
    match game? {
        "fo4" => Some(runtime_group_order::FO4_GROUP_ORDER),
        "skyrimse" => Some(runtime_group_order::SKYRIMSE_GROUP_ORDER),
        "starfield" => Some(runtime_group_order::STARFIELD_GROUP_ORDER),
        "fo76" => Some(runtime_group_order::FO76_GROUP_ORDER),
        "fo3" => Some(runtime_group_order::FO3_GROUP_ORDER),
        "fnv" => Some(runtime_group_order::FNV_GROUP_ORDER),
        _ => None,
    }
}

fn group_order_rank(order: &[&str], signature: &str) -> Option<usize> {
    order.iter().position(|candidate| *candidate == signature)
}

fn rewrite_semantic_formids_in_place(plugin: &mut ParsedPlugin) {
    let own_index = plugin.header.masters.len() as u8;
    rewrite_formids_in_items(&mut plugin.root_items, own_index);
    for fid in plugin.header.overridden_forms.iter_mut() {
        if ((*fid >> 24) & 0xFF) == 0xFF {
            *fid = ((own_index as u32) << 24) | (*fid & 0x00FF_FFFF);
        }
    }
}

pub fn remap_formids_in_items(
    items: &mut [ParsedItem],
    source_masters: &[String],
    target_masters: &[String],
    source_own_index: u8,
    target_own_index: u8,
) {
    for item in items.iter_mut() {
        match item {
            ParsedItem::Record(record) => remap_formids_in_record(
                record,
                source_masters,
                target_masters,
                source_own_index,
                target_own_index,
            ),
            ParsedItem::Group(group) => remap_formids_in_group(
                group,
                source_masters,
                target_masters,
                source_own_index,
                target_own_index,
            ),
        }
    }
}

fn remap_formids_in_group(
    group: &mut ParsedGroup,
    source_masters: &[String],
    target_masters: &[String],
    source_own_index: u8,
    target_own_index: u8,
) {
    if matches!(
        group.group_type,
        1 | CELL_CHILD_GROUP | PERSISTENT_GROUP | TEMPORARY_GROUP | VISIBLE_DISTANT_GROUP
    ) {
        let raw = u32::from_le_bytes(group.label);
        let remapped = remap_formid_index(
            raw,
            source_masters,
            target_masters,
            source_own_index,
            target_own_index,
        );
        group.label = remapped.to_le_bytes();
    }
    remap_formids_in_items(
        &mut group.children,
        source_masters,
        target_masters,
        source_own_index,
        target_own_index,
    );
}

pub fn remap_formids_in_record(
    record: &mut ParsedRecord,
    source_masters: &[String],
    target_masters: &[String],
    source_own_index: u8,
    target_own_index: u8,
) {
    // NAVI record headers are CK navmesh-map IDs, not ordinary owned record IDs.
    if record.signature.as_str() != "NAVI" {
        record.form_id = remap_formid_index(
            record.form_id,
            source_masters,
            target_masters,
            source_own_index,
            target_own_index,
        );
    }
    // sub.data is `Bytes` (refcount slice into the source mmap) and
    // therefore immutable. Materialize an owned copy only when at least
    // one FormID actually changes after remapping; this keeps the
    // common no-op path (matched master tables) zero-alloc.
    let is_land_record = record.signature.as_str() == "LAND";
    let is_navm_record = record.signature.as_str() == "NAVM";
    for sub in record.subrecords.iter_mut() {
        if is_navm_record && sub.signature.as_str() == "NVNM" {
            if let Some(remapped) = remap_nvnm_formids(
                sub.data.as_ref(),
                source_masters,
                target_masters,
                source_own_index,
                target_own_index,
            ) {
                sub.data = Bytes::from(remapped);
            }
            continue;
        }
        let is_land_texture_layer =
            is_land_record && matches!(sub.signature.as_str(), "BTXT" | "ATXT");
        if (sub.semantic_type.as_deref() == Some("formid") || is_land_texture_layer)
            && sub.data.len() >= 4
        {
            let raw = u32::from_le_bytes([sub.data[0], sub.data[1], sub.data[2], sub.data[3]]);
            let remapped = remap_formid_index(
                raw,
                source_masters,
                target_masters,
                source_own_index,
                target_own_index,
            );
            if remapped != raw {
                let mut buf = sub.data.to_vec();
                buf[0..4].copy_from_slice(&remapped.to_le_bytes());
                sub.data = Bytes::from(buf);
            }
        } else if sub.semantic_type.as_deref() == Some("formid_array") && sub.data.len() % 4 == 0 {
            let needs_remap = sub.data.chunks_exact(4).any(|chunk| {
                let raw = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                let remapped = remap_formid_index(
                    raw,
                    source_masters,
                    target_masters,
                    source_own_index,
                    target_own_index,
                );
                remapped != raw
            });
            if needs_remap {
                let mut buf = sub.data.to_vec();
                for chunk in buf.chunks_exact_mut(4) {
                    let raw = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                    let remapped = remap_formid_index(
                        raw,
                        source_masters,
                        target_masters,
                        source_own_index,
                        target_own_index,
                    );
                    chunk.copy_from_slice(&remapped.to_le_bytes());
                }
                sub.data = Bytes::from(buf);
            }
        }
    }
}

fn remap_nvnm_formids(
    data: &[u8],
    source_masters: &[String],
    target_masters: &[String],
    source_own_index: u8,
    target_own_index: u8,
) -> Option<Vec<u8>> {
    let mut payload = crate::nvnm::parse_nvnm(data).ok()?;
    let mut changed = false;
    match &mut payload.parent {
        crate::nvnm::NvnmParent::Interior { cell } => {
            let remapped = remap_formid_index(
                *cell,
                source_masters,
                target_masters,
                source_own_index,
                target_own_index,
            );
            changed |= remapped != *cell;
            *cell = remapped;
        }
        crate::nvnm::NvnmParent::Exterior { world, .. } => {
            let remapped = remap_formid_index(
                *world,
                source_masters,
                target_masters,
                source_own_index,
                target_own_index,
            );
            changed |= remapped != *world;
            *world = remapped;
        }
    }
    for edge_link in &mut payload.edge_links {
        let raw = u32::from_le_bytes(edge_link.row[4..8].try_into().ok()?);
        let remapped = remap_formid_index(
            raw,
            source_masters,
            target_masters,
            source_own_index,
            target_own_index,
        );
        changed |= remapped != raw;
        edge_link.row[4..8].copy_from_slice(&remapped.to_le_bytes());
    }
    for door_ref in &mut payload.door_refs {
        let remapped = remap_formid_index(
            door_ref.door_ref_form_id,
            source_masters,
            target_masters,
            source_own_index,
            target_own_index,
        );
        changed |= remapped != door_ref.door_ref_form_id;
        door_ref.door_ref_form_id = remapped;
    }
    changed.then(|| crate::nvnm::write_nvnm(&payload))
}

fn remap_formid_index(
    raw: u32,
    source_masters: &[String],
    target_masters: &[String],
    source_own_index: u8,
    target_own_index: u8,
) -> u32 {
    if raw == 0 || raw == u32::MAX {
        return raw;
    }
    let source_index = ((raw >> 24) & 0xFF) as u8;
    let object_id = raw & 0x00FF_FFFF;
    if source_index == source_own_index {
        return ((target_own_index as u32) << 24) | object_id;
    }
    if (source_index as usize) < source_masters.len() {
        let source_master = &source_masters[source_index as usize];
        for (target_index, candidate) in target_masters.iter().enumerate() {
            if candidate.eq_ignore_ascii_case(source_master.as_str()) {
                return ((target_index as u32) << 24) | object_id;
            }
        }
    }
    raw
}

fn rewrite_formids_in_items(items: &mut Vec<ParsedItem>, own_index: u8) {
    for item in items.iter_mut() {
        match item {
            ParsedItem::Record(record) => rewrite_formids_in_record(record, own_index),
            ParsedItem::Group(group) => rewrite_formids_in_items(&mut group.children, own_index),
        }
    }
}

fn rewrite_formids_in_record(record: &mut ParsedRecord, own_index: u8) {
    if ((record.form_id >> 24) & 0xFF) == 0xFF {
        record.form_id = ((own_index as u32) << 24) | (record.form_id & 0x00FF_FFFF);
    }
    // Subrecord data is `Bytes` (refcounted slice into the source mmap).
    // Mutation requires materializing an owned copy — but in practice the
    // 0xFF sentinel is only used by records authored at runtime, never by
    // records loaded from disk. We branch on "is any rewrite actually
    // needed?" first and only allocate when the answer is yes.
    let is_land_record = record.signature.as_str() == "LAND";
    for sub in record.subrecords.iter_mut() {
        let is_land_texture_layer =
            is_land_record && matches!(sub.signature.as_str(), "BTXT" | "ATXT");
        if (sub.semantic_type.as_deref() == Some("formid") || is_land_texture_layer)
            && sub.data.len() >= 4
        {
            let raw = u32::from_le_bytes([sub.data[0], sub.data[1], sub.data[2], sub.data[3]]);
            if raw != u32::MAX && ((raw >> 24) & 0xFF) == 0xFF {
                let rewritten = ((own_index as u32) << 24) | (raw & 0x00FF_FFFF);
                let mut buf = sub.data.to_vec();
                buf[0..4].copy_from_slice(&rewritten.to_le_bytes());
                sub.data = Bytes::from(buf);
            }
        } else if sub.semantic_type.as_deref() == Some("formid_array") && sub.data.len() % 4 == 0 {
            let needs_rewrite = sub.data.chunks_exact(4).any(|chunk| {
                let raw = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                raw != u32::MAX && ((raw >> 24) & 0xFF) == 0xFF
            });
            if needs_rewrite {
                let mut buf = sub.data.to_vec();
                for chunk in buf.chunks_exact_mut(4) {
                    let raw = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                    if raw != u32::MAX && ((raw >> 24) & 0xFF) == 0xFF {
                        let rewritten = ((own_index as u32) << 24) | (raw & 0x00FF_FFFF);
                        chunk.copy_from_slice(&rewritten.to_le_bytes());
                    }
                }
                sub.data = Bytes::from(buf);
            }
        }
    }
}

fn find_first_record<'a, F>(items: &'a [ParsedItem], predicate: &mut F) -> Option<&'a ParsedRecord>
where
    F: FnMut(&'a ParsedRecord) -> bool,
{
    for item in items {
        match item {
            ParsedItem::Record(record) => {
                if predicate(record) {
                    return Some(record);
                }
            }
            ParsedItem::Group(group) => {
                if let Some(record) = find_first_record(&group.children, predicate) {
                    return Some(record);
                }
            }
        }
    }
    None
}

fn collect_records<'a, F>(
    items: &'a [ParsedItem],
    predicate: &mut F,
    out: &mut Vec<&'a ParsedRecord>,
) where
    F: FnMut(&'a ParsedRecord) -> bool,
{
    for item in items {
        match item {
            ParsedItem::Record(record) => {
                if predicate(record) {
                    out.push(record);
                }
            }
            ParsedItem::Group(group) => collect_records(&group.children, predicate, out),
        }
    }
}

fn record_summary_tuple(record: &ParsedRecord) -> (u32, String, Option<String>) {
    (
        record.form_id,
        record.signature.to_string(),
        record_editor_id_value(record).map(|value| value.trim_end_matches('\0').to_string()),
    )
}

fn collect_record_summaries(
    items: &[ParsedItem],
    signature: Option<&str>,
    out: &mut Vec<(u32, String, Option<String>)>,
) {
    for item in items {
        match item {
            ParsedItem::Group(group) => collect_record_summaries(&group.children, signature, out),
            ParsedItem::Record(record) => {
                if signature
                    .map(|target| record.signature.as_str() == target)
                    .unwrap_or(true)
                {
                    out.push(record_summary_tuple(record));
                }
            }
        }
    }
}

fn apply_object_id_mapping_in_items(
    items: &mut [ParsedItem],
    old_high: u8,
    new_high: u8,
    object_id_map: &HashMap<u32, u32>,
    schema: Option<&CompiledSchema>,
) -> usize {
    let mut changed = 0;
    for item in items {
        match item {
            ParsedItem::Group(group) => {
                changed += apply_object_id_mapping_in_items(
                    &mut group.children,
                    old_high,
                    new_high,
                    object_id_map,
                    schema,
                );
            }
            ParsedItem::Record(record) => {
                if apply_object_id_mapping_to_record(
                    record,
                    old_high,
                    new_high,
                    object_id_map,
                    schema,
                ) {
                    changed += 1;
                }
            }
        }
    }
    changed
}

fn apply_object_id_mapping_to_record(
    record: &mut ParsedRecord,
    old_high: u8,
    new_high: u8,
    object_id_map: &HashMap<u32, u32>,
    schema: Option<&CompiledSchema>,
) -> bool {
    let mut changed = false;
    let high = ((record.form_id >> 24) & 0xFF) as u8;
    let object_id = record.form_id & 0x00FF_FFFF;
    if high == old_high {
        if let Some(new_object_id) = object_id_map.get(&object_id) {
            record.form_id = ((new_high as u32) << 24) | (new_object_id & 0x00FF_FFFF);
            changed = true;
        }
    }
    let mut rewrite = |raw: u32| rewrite_object_id_formid(raw, old_high, new_high, object_id_map);
    changed |= rewrite_referenced_form_ids_in_subrecords(
        record.signature.as_str(),
        &mut record.subrecords,
        schema,
        &mut rewrite,
    );
    changed
}

fn rewrite_object_id_formid(
    raw: u32,
    old_high: u8,
    new_high: u8,
    object_id_map: &HashMap<u32, u32>,
) -> Option<u32> {
    if ((raw >> 24) & 0xFF) as u8 != old_high {
        return None;
    }
    let object_id = raw & 0x00FF_FFFF;
    object_id_map
        .get(&object_id)
        .map(|new_object_id| ((new_high as u32) << 24) | (new_object_id & 0x00FF_FFFF))
}

fn cloned_record_for_form_id(slot: &mut NativePluginSlot, form_id: u32) -> Option<ParsedRecord> {
    let raw_form_id = form_id & 0xFFFF_FFFF;
    let own_index = (slot.parsed.header.masters.len() & 0xFF) as u8;
    let records = ensure_records_section(slot);
    if let Some(record) = records.record(&slot.parsed, raw_form_id) {
        return Some(record.clone());
    }
    let object_id = raw_form_id & 0x00FF_FFFF;
    let picked = {
        let core = ensure_core_section(slot);
        core.form_ids_by_object_id
            .get(&object_id)
            .and_then(|form_ids| pick_owned_form_id(form_ids, own_index))
    };
    picked.and_then(|form_id| {
        let records = ensure_records_section(slot);
        records.record(&slot.parsed, form_id).cloned()
    })
}

fn remap_formids_for_copy_in_record(
    record: &mut ParsedRecord,
    source_masters: &[String],
    source_plugin_name: &str,
    target_masters: &[String],
    source_own_index: u8,
) {
    record.form_id = remap_formid_for_copy(
        record.form_id,
        source_masters,
        source_plugin_name,
        target_masters,
        source_own_index,
    );
    for subrecord in record.subrecords.iter_mut() {
        match subrecord.semantic_type.as_deref() {
            Some("formid") if subrecord.data.len() >= 4 => {
                let raw = u32::from_le_bytes([
                    subrecord.data[0],
                    subrecord.data[1],
                    subrecord.data[2],
                    subrecord.data[3],
                ]);
                let remapped = remap_formid_for_copy(
                    raw,
                    source_masters,
                    source_plugin_name,
                    target_masters,
                    source_own_index,
                );
                if remapped != raw {
                    let mut buf = subrecord.data.to_vec();
                    buf[0..4].copy_from_slice(&remapped.to_le_bytes());
                    subrecord.data = Bytes::from(buf);
                }
            }
            Some("formid_array") if subrecord.data.len() >= 4 && subrecord.data.len() % 4 == 0 => {
                let needs_remap = subrecord.data.chunks_exact(4).any(|chunk| {
                    let raw = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                    remap_formid_for_copy(
                        raw,
                        source_masters,
                        source_plugin_name,
                        target_masters,
                        source_own_index,
                    ) != raw
                });
                if needs_remap {
                    let mut buf = subrecord.data.to_vec();
                    for chunk in buf.chunks_exact_mut(4) {
                        let raw = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                        let remapped = remap_formid_for_copy(
                            raw,
                            source_masters,
                            source_plugin_name,
                            target_masters,
                            source_own_index,
                        );
                        chunk.copy_from_slice(&remapped.to_le_bytes());
                    }
                    subrecord.data = Bytes::from(buf);
                }
            }
            _ => {}
        }
    }
}

fn remap_formid_for_copy(
    raw: u32,
    source_masters: &[String],
    source_plugin_name: &str,
    target_masters: &[String],
    source_own_index: u8,
) -> u32 {
    if raw == 0 || raw == u32::MAX {
        return raw;
    }
    let source_index = ((raw >> 24) & 0xFF) as u8;
    let object_id = raw & 0x00FF_FFFF;
    let target_name = if source_index == source_own_index {
        Some(source_plugin_name)
    } else {
        source_masters
            .get(source_index as usize)
            .map(|value| value.as_str())
    };
    let Some(target_name) = target_name else {
        return raw;
    };
    target_masters
        .iter()
        .position(|candidate| candidate.eq_ignore_ascii_case(target_name))
        .map(|target_index| ((target_index as u32) << 24) | object_id)
        .unwrap_or(raw)
}

fn merge_conflict_record_chain(
    signature: &str,
    sources: &[MergeSourceRecord],
    target_masters: &[String],
) -> PyResult<Option<ParsedRecord>> {
    if sources.is_empty() {
        return Err(value_error("merge conflict chain is empty"));
    }
    match signature {
        "LVLI" | "LVLN" | "LVSP" => Ok(Some(merge_lvlo_conflict_chain(sources, target_masters))),
        "FLST" => Ok(Some(merge_flst_conflict_chain(sources, target_masters))),
        "MUSC" => Ok(Some(merge_musc_conflict_chain(sources, target_masters))),
        _ if sources
            .iter()
            .any(|source| record_has_subrecord(&source.record, "KWDA")) =>
        {
            Ok(Some(merge_kwda_conflict_chain(sources, target_masters)))
        }
        _ => Ok(None),
    }
}

fn remapped_merge_base(source: &MergeSourceRecord, target_masters: &[String]) -> ParsedRecord {
    let mut record = source.record.clone();
    remap_formids_for_copy_in_record(
        &mut record,
        &source.masters,
        &source.plugin_name,
        target_masters,
        source.own_index,
    );
    record.raw_payload = None;
    record.parse_error = None;
    record
}

fn record_has_subrecord(record: &ParsedRecord, signature: &str) -> bool {
    record
        .subrecords
        .iter()
        .any(|subrecord| subrecord.signature.as_str() == signature)
}

fn find_subrecord_index(record: &ParsedRecord, signature: &str) -> Option<usize> {
    record
        .subrecords
        .iter()
        .position(|subrecord| subrecord.signature.as_str() == signature)
}

fn remove_subrecords_by_signature(record: &mut ParsedRecord, signatures: &[&str]) {
    record.subrecords.retain(|subrecord| {
        !signatures
            .iter()
            .any(|sig| subrecord.signature.as_str() == *sig)
    });
}

fn parsed_subrecord(signature: &str, data: Vec<u8>) -> ParsedSubrecord {
    ParsedSubrecord {
        signature: SmolStr::new(signature),
        data: Bytes::from(data),
        semantic_type: None,
    }
}

fn insert_or_replace_subrecord_data(
    record: &mut ParsedRecord,
    signature: &str,
    data: Vec<u8>,
    create_after: Option<&str>,
) -> usize {
    if let Some(index) = find_subrecord_index(record, signature) {
        record.subrecords[index].data = Bytes::from(data);
        return index;
    }
    let subrecord = parsed_subrecord(signature, data);
    if let Some(anchor) = create_after.and_then(|sig| find_subrecord_index(record, sig)) {
        let index = anchor + 1;
        record.subrecords.insert(index, subrecord);
        return index;
    }
    record.subrecords.push(subrecord);
    record.subrecords.len() - 1
}

fn read_u32_at(data: &[u8], offset: usize) -> Option<u32> {
    if offset + 4 > data.len() {
        return None;
    }
    Some(u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ]))
}

fn read_u16_at(data: &[u8], offset: usize) -> Option<u16> {
    if offset + 2 > data.len() {
        return None;
    }
    Some(u16::from_le_bytes([data[offset], data[offset + 1]]))
}

fn collect_subrecord_formids(record: &ParsedRecord, subrecord_signature: &str) -> Vec<u32> {
    let mut out = Vec::new();
    for subrecord in &record.subrecords {
        if subrecord.signature.as_str() != subrecord_signature {
            continue;
        }
        for chunk in subrecord.data.chunks_exact(4) {
            out.push(u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
        }
    }
    out
}

fn push_unique_remapped_formids(
    source: &MergeSourceRecord,
    subrecord_signature: &str,
    target_masters: &[String],
    seen: &mut HashSet<u32>,
    merged: &mut Vec<u32>,
) {
    for raw in collect_subrecord_formids(&source.record, subrecord_signature) {
        let translated = remap_formid_for_copy(
            raw,
            &source.masters,
            &source.plugin_name,
            target_masters,
            source.own_index,
        );
        if translated == 0 || !seen.insert(translated) {
            continue;
        }
        merged.push(translated);
    }
}

fn pack_formids(form_ids: &[u32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(form_ids.len() * 4);
    for form_id in form_ids {
        out.extend_from_slice(&form_id.to_le_bytes());
    }
    out
}

fn lvlo_entry_key(entry: &[u8]) -> Option<(u16, u32, u16)> {
    Some((
        read_u16_at(entry, 0)?,
        read_u32_at(entry, LVLO_FORMID_OFFSET)?,
        read_u16_at(entry, 8)?,
    ))
}

fn gather_remapped_lvlo_entries(
    source: &MergeSourceRecord,
    target_masters: &[String],
) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for subrecord in &source.record.subrecords {
        if subrecord.signature.as_str() != "LVLO" {
            continue;
        }
        for entry in subrecord.data.chunks_exact(LVLO_STRIDE) {
            let mut translated = entry.to_vec();
            if let Some(raw) = read_u32_at(&translated, LVLO_FORMID_OFFSET) {
                let remapped = remap_formid_for_copy(
                    raw,
                    &source.masters,
                    &source.plugin_name,
                    target_masters,
                    source.own_index,
                );
                translated[LVLO_FORMID_OFFSET..LVLO_FORMID_OFFSET + 4]
                    .copy_from_slice(&remapped.to_le_bytes());
            }
            out.push(translated);
        }
    }
    out
}

fn merge_lvlo_conflict_chain(
    sources: &[MergeSourceRecord],
    target_masters: &[String],
) -> ParsedRecord {
    let mut target_record = remapped_merge_base(&sources[0], target_masters);
    let mut seen = HashSet::new();
    let mut merged = Vec::new();
    for source in sources {
        for entry in gather_remapped_lvlo_entries(source, target_masters) {
            let Some(key) = lvlo_entry_key(&entry) else {
                continue;
            };
            if !seen.insert(key) {
                continue;
            }
            merged.push(entry);
        }
    }

    remove_subrecords_by_signature(&mut target_record, &["LLCT", "LVLO"]);
    let anchor = if record_has_subrecord(&target_record, "LVLF") {
        Some("LVLF")
    } else if record_has_subrecord(&target_record, "LVLD") {
        Some("LVLD")
    } else {
        None
    };
    let llct_index = insert_or_replace_subrecord_data(
        &mut target_record,
        "LLCT",
        vec![merged.len().min(0xFF) as u8],
        anchor,
    );
    let mut insert_at = llct_index + 1;
    for entry in merged {
        target_record
            .subrecords
            .insert(insert_at, parsed_subrecord("LVLO", entry));
        insert_at += 1;
    }
    target_record
}

fn merge_flst_conflict_chain(
    sources: &[MergeSourceRecord],
    target_masters: &[String],
) -> ParsedRecord {
    let mut target_record = remapped_merge_base(&sources[0], target_masters);
    let mut seen = HashSet::new();
    let mut merged = Vec::new();
    for source in sources {
        push_unique_remapped_formids(source, "LNAM", target_masters, &mut seen, &mut merged);
    }
    remove_subrecords_by_signature(&mut target_record, &["LNAM"]);
    for form_id in merged {
        target_record
            .subrecords
            .push(parsed_subrecord("LNAM", form_id.to_le_bytes().to_vec()));
    }
    target_record
}

fn merge_musc_conflict_chain(
    sources: &[MergeSourceRecord],
    target_masters: &[String],
) -> ParsedRecord {
    let mut target_record = remapped_merge_base(&sources[0], target_masters);
    let mut seen = HashSet::new();
    let mut merged = Vec::new();
    for source in sources {
        push_unique_remapped_formids(source, "TNAM", target_masters, &mut seen, &mut merged);
    }
    remove_subrecords_by_signature(&mut target_record, &["TNAM"]);
    if !merged.is_empty() {
        insert_or_replace_subrecord_data(&mut target_record, "TNAM", pack_formids(&merged), None);
    }
    target_record
}

fn merge_kwda_conflict_chain(
    sources: &[MergeSourceRecord],
    target_masters: &[String],
) -> ParsedRecord {
    let winner = sources.last().expect("non-empty merge chain");
    let mut target_record = remapped_merge_base(winner, target_masters);
    let mut seen = HashSet::new();
    let mut merged = Vec::new();
    for source in sources {
        push_unique_remapped_formids(source, "KWDA", target_masters, &mut seen, &mut merged);
    }
    remove_subrecords_by_signature(&mut target_record, &["KSIZ", "KWDA"]);
    if merged.is_empty() {
        return target_record;
    }
    let ksiz_index = insert_or_replace_subrecord_data(
        &mut target_record,
        "KSIZ",
        (merged.len() as u32).to_le_bytes().to_vec(),
        None,
    );
    target_record.subrecords.insert(
        ksiz_index + 1,
        parsed_subrecord("KWDA", pack_formids(&merged)),
    );
    target_record
}

fn undelete_and_disable_refs_in_items(
    items: &mut [ParsedItem],
    own_index: u8,
    signatures: &HashSet<SmolStr>,
    changed: &mut Vec<u32>,
) {
    for item in items {
        match item {
            ParsedItem::Group(group) => {
                undelete_and_disable_refs_in_items(
                    &mut group.children,
                    own_index,
                    signatures,
                    changed,
                );
            }
            ParsedItem::Record(record) => {
                if undelete_and_disable_record(record, own_index, signatures) {
                    changed.push(record.form_id);
                }
            }
        }
    }
}

fn undelete_and_disable_record(
    record: &mut ParsedRecord,
    own_index: u8,
    signatures: &HashSet<SmolStr>,
) -> bool {
    if !signatures.contains(&record.signature) {
        return false;
    }
    if record.flags & RECORD_FLAG_DELETED == 0 {
        return false;
    }
    if ((record.form_id >> 24) & 0xFF) as u8 != own_index {
        return false;
    }

    record.flags = (record.flags & !RECORD_FLAG_DELETED) | RECORD_FLAG_INITIALLY_DISABLED;
    record.raw_payload = None;
    record
        .subrecords
        .retain(|subrecord| subrecord.signature != "XTEL");
    let xesp_data = {
        let mut data = Vec::with_capacity(8);
        data.extend_from_slice(&PLAYER_FORM_ID.to_le_bytes());
        data.extend_from_slice(&1u32.to_le_bytes());
        Bytes::from(data)
    };
    if let Some(subrecord) = record
        .subrecords
        .iter_mut()
        .find(|subrecord| subrecord.signature == "XESP")
    {
        subrecord.data = xesp_data;
        subrecord.semantic_type = None;
    } else {
        record.subrecords.push(ParsedSubrecord {
            signature: SmolStr::new("XESP"),
            data: xesp_data,
            semantic_type: None,
        });
    }
    true
}

fn prefer_object_id_lookup_candidate(
    candidate: &ParsedRecord,
    existing: &ParsedRecord,
    own_index: u8,
) -> bool {
    let candidate_index = ((candidate.form_id >> 24) & 0xFF) as u8;
    let existing_index = ((existing.form_id >> 24) & 0xFF) as u8;
    let candidate_is_own = candidate_index == LOCAL_FORM_INDEX || candidate_index == own_index;
    let existing_is_own = existing_index == LOCAL_FORM_INDEX || existing_index == own_index;
    candidate_is_own && !existing_is_own
}

pub fn is_known_formid_subrecord(signature: &str) -> bool {
    matches!(
        signature,
        "ANAM"
            | "ATKR"
            | "CNAM"
            | "ECOR"
            | "EFID"
            | "EITM"
            | "ETYP"
            | "FTSF"
            | "FTSM"
            | "INAM"
            | "LNAM"
            | "PNAM"
            | "RNAM"
            | "SADD"
            | "SAKD"
            | "SNAM"
            | "SOFT"
            | "SPLO"
            | "STKD"
            | "TNAM"
            | "VNAM"
            | "VTCK"
            | "WNAM"
            | "YNAM"
            | "ZNAM"
    )
}

pub fn is_known_formid_array_subrecord(signature: &str) -> bool {
    matches!(signature, "KWDA" | "MODS" | "ONAM" | "SPOR")
}

fn coerce_local_object_id(plugin: &ParsedPlugin, raw_form_id: u32) -> Option<u32> {
    let normalized = raw_form_id & 0xFFFF_FFFF;
    if normalized == 0 {
        return Some(0);
    }
    if normalized <= 0x00FF_FFFF {
        return Some(normalized);
    }
    let object_id = normalized & 0x00FF_FFFF;
    if object_id == 0 {
        return Some(0);
    }
    let index = ((normalized >> 24) & 0xFF) as u8;
    let own_index = (plugin.header.masters.len() & 0xFF) as u8;
    if index == LOCAL_FORM_INDEX || index == own_index {
        return Some(object_id);
    }
    if (index as usize) < plugin.header.masters.len() {
        return None;
    }
    Some(object_id)
}

fn form_key_for_object_id(
    plugin: &ParsedPlugin,
    core: &CoreSection,
    object_id: u32,
) -> Option<FormKey> {
    let form_ids = core.form_ids_by_object_id.get(&object_id)?;
    let own_index = (plugin.header.masters.len() & 0xFF) as u8;
    let form_id = pick_owned_form_id(form_ids, own_index)?;
    let own_plugin_name: Arc<str> = Arc::from(plugin.plugin_name.as_str());
    Some(resolve_form_id_to_form_key(
        form_id,
        &own_plugin_name,
        &plugin.header.masters,
    ))
}

fn form_keys_to_local_object_ids(plugin: &ParsedPlugin, form_keys: &[FormKey]) -> Vec<u32> {
    let mut seen = HashSet::new();
    let mut object_ids = Vec::new();
    for form_key in form_keys {
        if let Some(object_id) = form_key_to_local_object_id(plugin, form_key) {
            if seen.insert(object_id) {
                object_ids.push(object_id);
            }
        }
    }
    object_ids
}

fn form_key_to_local_object_id(plugin: &ParsedPlugin, form_key: &FormKey) -> Option<u32> {
    if !form_key.plugin.is_empty() {
        if !form_key
            .plugin
            .eq_ignore_ascii_case(plugin.plugin_name.as_str())
        {
            return None;
        }
        return Some(form_key.object_id);
    }
    coerce_local_object_id(plugin, form_key.object_id).filter(|object_id| *object_id != 0)
}

// ---------------------------------------------------------------------------
// Leaf helpers (Rust-native twins of PyAny-based helpers elsewhere in this file)
// ---------------------------------------------------------------------------

fn editor_id_from_subrecords(subrecords: &[ParsedSubrecord]) -> Option<String> {
    for sub in subrecords {
        if sub.signature == "EDID" {
            return Some(decode_cp1252(&sub.data));
        }
    }
    None
}

fn editor_id_from_parsed(record: &ParsedRecord) -> Option<String> {
    editor_id_from_subrecords(&record.subrecords).or_else(|| {
        lazy_subrecords_for_record(record)
            .ok()
            .flatten()
            .and_then(|subrecords| editor_id_from_subrecords(&subrecords))
    })
}

fn full_name_from_parsed(record: &ParsedRecord) -> Option<String> {
    fn from_subrecords(subrecords: &[ParsedSubrecord]) -> Option<String> {
        for signature in ["FULL", "RNAM"] {
            for sub in subrecords {
                if sub.signature == signature {
                    return Some(decode_cp1252(&sub.data));
                }
            }
        }
        None
    }

    from_subrecords(&record.subrecords).or_else(|| {
        lazy_subrecords_for_record(record)
            .ok()
            .flatten()
            .and_then(|subrecords| from_subrecords(&subrecords))
    })
}

fn record_filename_native(record: &ParsedRecord, plugin_name: &str, extension: &str) -> String {
    let form_id = format_object_id(record.form_id);
    let plugin_name_safe = filename_safe(Some(plugin_name.to_string()), "Plugin.esp");
    let editor_id = filename_safe(
        editor_id_from_parsed(record).filter(|value| !value.is_empty()),
        "",
    );
    if !editor_id.is_empty() {
        format!("{editor_id} - {form_id}_{plugin_name_safe}{extension}")
    } else {
        format!("{form_id}_{plugin_name_safe}{extension}")
    }
}

fn special_record_dir_name_native(record: &ParsedRecord, plugin_name: &str) -> String {
    let object_id = format_object_id(record.form_id);
    let plugin_name_safe = filename_safe(Some(plugin_name.to_string()), "Plugin.esp");
    let editor_id = filename_safe(
        editor_id_from_parsed(record).filter(|value| !value.is_empty()),
        "",
    );
    if !editor_id.is_empty() {
        format!("{editor_id} - {object_id}_{plugin_name_safe}")
    } else {
        format!("{object_id}_{plugin_name_safe}")
    }
}

fn group_label_text_native(group: &ParsedGroup) -> Option<String> {
    if group.group_type != 0 {
        return None;
    }
    let text = String::from_utf8_lossy(&group.label)
        .trim_end_matches('\0')
        .to_string();
    if text.len() == 4 { Some(text) } else { None }
}

fn group_display_prefix_native(
    group: &ParsedGroup,
    plugin_index: &PluginIndex<'_>,
    plugin_name: &str,
    is_root: bool,
) -> String {
    if is_root && group.group_type == 0 {
        if let Some(label_text) = group_label_text_native(group) {
            return label_text;
        }
    }
    let label = group.label;
    match group.group_type {
        1 | 6 | 7 => {
            let object_id =
                u32::from_le_bytes([label[0], label[1], label[2], label[3]]) & 0x00FF_FFFF;
            if let Some(target) = plugin_index.get(object_id) {
                let form_id = format_object_id(target.form_id);
                let editor_id = filename_safe(
                    editor_id_from_parsed(target).filter(|value| !value.is_empty()),
                    "",
                );
                if !editor_id.is_empty() {
                    return format!("{editor_id} - {form_id}");
                }
                return form_id;
            }
            let _ = plugin_name;
        }
        4 => {
            let x = i16::from_le_bytes([label[0], label[1]]);
            let y = i16::from_le_bytes([label[2], label[3]]);
            return format!("Block {x}, {y}");
        }
        5 => {
            let x = i16::from_le_bytes([label[0], label[1]]);
            let y = i16::from_le_bytes([label[2], label[3]]);
            return format!("SubBlock {x}, {y}");
        }
        8 => return "Persistent".to_string(),
        9 => return "Temporary".to_string(),
        10 => return "Visible Distant".to_string(),
        _ => {}
    }
    String::new()
}

fn group_dir_basename_native(
    group: &ParsedGroup,
    plugin_index: &PluginIndex<'_>,
    plugin_name: &str,
    is_root: bool,
) -> String {
    if is_root {
        if let Some(label_text) = group_label_text_native(group) {
            return label_text;
        }
    }
    let prefix = filename_safe(
        Some(group_display_prefix_native(
            group,
            plugin_index,
            plugin_name,
            is_root,
        )),
        "",
    );
    let label_hex = hex::encode_upper(group.label);
    let group_type = group.group_type;
    if !prefix.is_empty() {
        format!("{prefix}{GROUP_DIR_PREFIX}{group_type}__{label_hex}")
    } else {
        format!("{GROUP_DIR_PREFIX}{group_type}__{label_hex}")
    }
}

fn grid_dir_name_from_group_native(group: &ParsedGroup) -> PyResult<String> {
    let (y, x) = decode_group_grid(&group.label)
        .ok_or_else(|| value_error("invalid projected grid label"))?;
    Ok(format!("{x}, {y}"))
}

fn index_dir_name_from_group_native(group: &ParsedGroup) -> PyResult<String> {
    let value = decode_group_index(&group.label)
        .ok_or_else(|| value_error("invalid projected index label"))?;
    Ok(value.to_string())
}

fn can_project_cell_child_group_native(group: &ParsedGroup) -> bool {
    if !matches!(
        group.group_type,
        PERSISTENT_GROUP | TEMPORARY_GROUP | VISIBLE_DISTANT_GROUP
    ) {
        return false;
    }
    for child in &group.children {
        if !matches!(child, ParsedItem::Record(_)) {
            return false;
        }
    }
    true
}

fn can_project_cell_children_native(group: &ParsedGroup) -> bool {
    for child in &group.children {
        match child {
            ParsedItem::Record(r) => {
                if !matches!(r.signature.as_str(), "LAND" | "NAVM") {
                    return false;
                }
            }
            ParsedItem::Group(g) => {
                if !can_project_cell_child_group_native(g) {
                    return false;
                }
            }
        }
    }
    true
}

// cell_records / cell_children use BTreeSet for deterministic comparison
// semantics. cell_children rejects duplicate form_ids (mirrors Python
// `authoring_dir.py:444-445` — `if form_id in cell_children: return None`).
// cell_records silently overwrites on dup (Python dict assignment) — that
// path is fine because equality against cell_children still catches drift.

fn can_project_wrld_group_native(group: &ParsedGroup) -> bool {
    use std::collections::BTreeSet;
    for child in &group.children {
        match child {
            ParsedItem::Record(r) => {
                if r.signature != "WRLD" {
                    return false;
                }
            }
            ParsedItem::Group(world_group) => {
                if world_group.group_type != 1 {
                    return false;
                }
                let mut cell_records: BTreeSet<u32> = BTreeSet::new();
                let mut cell_children: BTreeSet<u32> = BTreeSet::new();
                for world_child in &world_group.children {
                    match world_child {
                        ParsedItem::Record(wr) => {
                            if wr.signature != "CELL" {
                                return false;
                            }
                            cell_records.insert(wr.form_id);
                        }
                        ParsedItem::Group(block) => {
                            if block.group_type == CELL_CHILD_GROUP {
                                if !can_project_cell_children_native(block) {
                                    return false;
                                }
                                let Some(form_id) = decode_group_index(&block.label) else {
                                    return false;
                                };
                                if !cell_children.insert(form_id) {
                                    return false;
                                }
                                continue;
                            }
                            if block.group_type != EXTERIOR_CELL_BLOCK {
                                return false;
                            }
                            for subblock_item in &block.children {
                                let subblock = match subblock_item {
                                    ParsedItem::Group(g) => g,
                                    _ => return false,
                                };
                                if subblock.group_type != EXTERIOR_CELL_SUBBLOCK {
                                    return false;
                                }
                                for cell_item in &subblock.children {
                                    match cell_item {
                                        ParsedItem::Record(r) => {
                                            if r.signature != "CELL" {
                                                return false;
                                            }
                                            cell_records.insert(r.form_id);
                                        }
                                        ParsedItem::Group(cell_group) => {
                                            if cell_group.group_type != CELL_CHILD_GROUP
                                                || !can_project_cell_children_native(cell_group)
                                            {
                                                return false;
                                            }
                                            let Some(form_id) =
                                                decode_group_index(&cell_group.label)
                                            else {
                                                return false;
                                            };
                                            if !cell_children.insert(form_id) {
                                                return false;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                if !cell_children.is_subset(&cell_records) {
                    return false;
                }
            }
        }
    }
    true
}

fn can_project_cell_group_native(group: &ParsedGroup) -> bool {
    use std::collections::BTreeSet;
    for block_item in &group.children {
        let block = match block_item {
            ParsedItem::Group(g) => g,
            _ => return false,
        };
        if block.group_type != INTERIOR_CELL_BLOCK {
            return false;
        }
        for subblock_item in &block.children {
            let subblock = match subblock_item {
                ParsedItem::Group(g) => g,
                _ => return false,
            };
            if subblock.group_type != INTERIOR_CELL_SUBBLOCK {
                return false;
            }
            let mut cell_records: BTreeSet<u32> = BTreeSet::new();
            let mut cell_children: BTreeSet<u32> = BTreeSet::new();
            for item in &subblock.children {
                match item {
                    ParsedItem::Record(r) => {
                        if r.signature != "CELL" {
                            return false;
                        }
                        cell_records.insert(r.form_id);
                    }
                    ParsedItem::Group(cg) => {
                        if cg.group_type != CELL_CHILD_GROUP
                            || !can_project_cell_children_native(cg)
                        {
                            return false;
                        }
                        let Some(form_id) = decode_group_index(&cg.label) else {
                            return false;
                        };
                        // Python interior path (_collect_projected_interior_structure)
                        // does NOT reject dups — it overwrites via dict assignment.
                        // Mirror exactly. The subsequent set-equality check still
                        // catches mismatches with cell_records.
                        cell_children.insert(form_id);
                    }
                }
            }
            if !cell_children.is_subset(&cell_records) {
                return false;
            }
        }
    }
    true
}

/// Sorted `Vec<(object_id, &ParsedRecord)>` indexed by binary search.
///
/// For Starfield-class plugins (~3.8M records) this is roughly half the
/// memory of `HashMap<u32, &ParsedRecord>` (no buckets, no per-entry
/// hash slot, single contiguous Vec) and has better cache behavior on
/// the one-pass walk that builds it. Lookup is O(log n) instead of O(1)
/// average, but n is small enough (and lookups are rare enough relative
/// to construction) that the constant-factor win dominates.
pub struct PluginIndex<'a> {
    entries: Vec<(u32, &'a ParsedRecord)>,
}

impl<'a> PluginIndex<'a> {
    pub fn build(plugin: &'a ParsedPlugin) -> Self {
        let mut entries: Vec<(u32, &'a ParsedRecord)> = Vec::new();
        fn walk<'b>(items: &'b [ParsedItem], out: &mut Vec<(u32, &'b ParsedRecord)>) {
            for item in items {
                match item {
                    ParsedItem::Record(r) => {
                        let obj = r.form_id & 0x00FF_FFFF;
                        out.push((obj, r));
                    }
                    ParsedItem::Group(g) => walk(&g.children, out),
                }
            }
        }
        walk(&plugin.root_items, &mut entries);
        // Preserve the old HashMap behavior for duplicate lower-24-bit object
        // IDs: traversal's last record wins. Reversing first lets stable sort
        // keep that last record ahead of earlier duplicates for the same key,
        // and dedup_by_key then keeps it deterministically.
        entries.reverse();
        entries.sort_by_key(|(id, _)| *id);
        entries.dedup_by_key(|(id, _)| *id);
        entries.shrink_to_fit();
        Self { entries }
    }

    pub fn get(&self, object_id: u32) -> Option<&'a ParsedRecord> {
        let key = object_id & 0x00FF_FFFF;
        self.entries
            .binary_search_by_key(&key, |(id, _)| *id)
            .ok()
            .map(|i| self.entries[i].1)
    }
}

/// Rust-native mirror of `FormRef.from_raw` from `py_creation_lib/python/creation_lib/esp/model.py`.
/// Returns (plugin_name_ref, object_id, raw, missing_index).
fn form_ref_from_raw_native(
    raw: u32,
    masters: &[String],
    plugin_name: &str,
) -> (Option<String>, u32, Option<u32>, Option<u8>) {
    if raw == 0 {
        return (None, 0, Some(0), None);
    }
    let index = ((raw >> 24) & 0xFF) as u8;
    let object_id = raw & 0x00FF_FFFF;
    const LOCAL_FORM_INDEX: u8 = 0xFF;
    if index == LOCAL_FORM_INDEX {
        return (None, object_id, Some(raw), None);
    }
    // mapping = [*masters, plugin_name]
    let idx = index as usize;
    if idx < masters.len() {
        return (Some(masters[idx].clone()), object_id, Some(raw), None);
    }
    if idx == masters.len() {
        return (Some(plugin_name.to_string()), object_id, Some(raw), None);
    }
    (None, object_id, Some(raw), Some(index))
}

fn format_record_form_id_native(raw: u32, masters: &[String], plugin_name: &str) -> String {
    let (owner_plugin, object_id, _, missing_index) =
        form_ref_from_raw_native(raw, masters, plugin_name);
    match (owner_plugin, missing_index) {
        (_, Some(_)) => format!("{raw:08X}"),
        (Some(owner), _) if !owner.eq_ignore_ascii_case(plugin_name) => {
            format!("{object_id:06X}:{owner}")
        }
        _ => format!("{object_id:06X}"),
    }
}

fn extract_dialogue_payload_value(
    plugin: &ParsedPlugin,
    strings: &LocalizedStringsState,
) -> PyResult<JsonValue> {
    let mut topic_children: HashMap<u32, Vec<&ParsedRecord>> = HashMap::new();
    collect_topic_child_records(&plugin.root_items, &mut topic_children);

    let mut dials = Vec::new();
    let mut predicate = |record: &ParsedRecord| record.signature.as_str() == "DIAL";
    collect_records(&plugin.root_items, &mut predicate, &mut dials);

    let mut topics = Vec::with_capacity(dials.len());
    for dial in dials {
        let mut topic = JsonMap::new();
        topic.insert(
            "plugin".to_string(),
            JsonValue::String(plugin.plugin_name.clone()),
        );
        topic.insert(
            "dial_form_id".to_string(),
            JsonValue::String(format!("{:08X}", dial.form_id & 0xFFFF_FFFF)),
        );
        topic.insert(
            "editor_id".to_string(),
            optional_string_json(record_editor_id_value(dial)),
        );
        topic.insert(
            "topic".to_string(),
            dialogue_text_for_first_subrecord(plugin, strings, dial, &["FULL", "RNAM"])
                .unwrap_or(JsonValue::Null),
        );

        let mut infos_json = Vec::new();
        let mut response_count = 0usize;
        if let Some(infos) = topic_children.get(&(dial.form_id & 0xFFFF_FFFF)) {
            for info in infos {
                let info_payload = dialogue_info_payload_value(plugin, strings, info)?;
                if let Some(count) = info_payload
                    .get("responses")
                    .and_then(|value| value.as_array())
                    .map(Vec::len)
                {
                    response_count += count;
                }
                infos_json.push(JsonValue::Object(info_payload));
            }
        }
        topic.insert("infos".to_string(), JsonValue::Array(infos_json));
        topic.insert(
            "response_count".to_string(),
            JsonValue::Number(response_count.into()),
        );
        topics.push(JsonValue::Object(topic));
    }

    Ok(JsonValue::Array(topics))
}

fn optional_string_json(value: Option<String>) -> JsonValue {
    value
        .map(|text| JsonValue::String(text.trim_end_matches('\0').to_string()))
        .unwrap_or(JsonValue::Null)
}

fn collect_topic_child_records<'a>(
    items: &'a [ParsedItem],
    mapping: &mut HashMap<u32, Vec<&'a ParsedRecord>>,
) {
    for item in items {
        if let ParsedItem::Group(group) = item {
            if group.group_type == TOPIC_CHILD_GROUP {
                let topic_form_id = u32::from_le_bytes(group.label);
                let mut infos = Vec::new();
                collect_info_records(&group.children, &mut infos);
                if !infos.is_empty() {
                    mapping.entry(topic_form_id).or_default().extend(infos);
                }
            }
            collect_topic_child_records(&group.children, mapping);
        }
    }
}

fn collect_info_records<'a>(items: &'a [ParsedItem], out: &mut Vec<&'a ParsedRecord>) {
    for item in items {
        match item {
            ParsedItem::Record(record) if record.signature.as_str() == "INFO" => out.push(record),
            ParsedItem::Group(group) => collect_info_records(&group.children, out),
            _ => {}
        }
    }
}

fn dialogue_info_payload_value(
    plugin: &ParsedPlugin,
    strings: &LocalizedStringsState,
    info: &ParsedRecord,
) -> PyResult<JsonMap<String, JsonValue>> {
    let mut payload = JsonMap::new();
    payload.insert(
        "form_id".to_string(),
        JsonValue::String(format!("{:08X}", info.form_id & 0xFFFF_FFFF)),
    );
    payload.insert(
        "editor_id".to_string(),
        optional_string_json(record_editor_id_value(info)),
    );
    payload.insert(
        "prompt".to_string(),
        dialogue_text_for_first_subrecord(plugin, strings, info, &["RNAM"])
            .unwrap_or(JsonValue::Null),
    );

    let mut responses = Vec::new();
    for subrecord in &info.subrecords {
        if subrecord.signature.as_str() == "NAM1" {
            responses.push(dialogue_text_subrecord_value(plugin, strings, subrecord));
        }
    }
    payload.insert("responses".to_string(), JsonValue::Array(responses));

    let conditions = dialogue_conditions_payload_value(plugin, info)?;
    if !conditions.is_empty() {
        payload.insert("conditions".to_string(), JsonValue::Array(conditions));
    }
    Ok(payload)
}

fn dialogue_text_for_first_subrecord(
    plugin: &ParsedPlugin,
    strings: &LocalizedStringsState,
    record: &ParsedRecord,
    signatures: &[&str],
) -> Option<JsonValue> {
    record
        .subrecords
        .iter()
        .find(|subrecord| {
            signatures
                .iter()
                .any(|signature| subrecord.signature.as_str() == *signature)
        })
        .map(|subrecord| dialogue_text_subrecord_value(plugin, strings, subrecord))
}

fn dialogue_text_subrecord_value(
    plugin: &ParsedPlugin,
    strings: &LocalizedStringsState,
    subrecord: &ParsedSubrecord,
) -> JsonValue {
    let mut payload = JsonMap::new();
    let raw = subrecord.data.as_ref();
    if (plugin.header.flags & TES4_FLAG_LOCALIZED) != 0 && raw.len() == 4 {
        let string_id = u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]);
        payload.insert(
            "kind".to_string(),
            JsonValue::String("localized_string".to_string()),
        );
        payload.insert("string_id".to_string(), JsonValue::Number(string_id.into()));
        payload.insert(
            "text".to_string(),
            resolve_localized_string_with_language(strings, string_id, None)
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        );
    } else {
        payload.insert("kind".to_string(), JsonValue::String("string".to_string()));
        payload.insert(
            "text".to_string(),
            JsonValue::String(decode_cp1252(raw).trim_end_matches('\0').to_string()),
        );
    }
    payload.insert(
        "raw_hex".to_string(),
        JsonValue::String(hex::encode_upper(raw)),
    );
    JsonValue::Object(payload)
}

fn dialogue_conditions_payload_value(
    plugin: &ParsedPlugin,
    record: &ParsedRecord,
) -> PyResult<Vec<JsonValue>> {
    let mut out = Vec::new();
    for subrecord in &record.subrecords {
        if !matches!(subrecord.signature.as_str(), "CTDA" | "CTDT") {
            continue;
        }
        let mut payload = dialogue_ctda_payload_value(plugin, subrecord.data.as_ref())?;
        payload.insert(
            "signature".to_string(),
            JsonValue::String(subrecord.signature.to_string()),
        );
        out.push(JsonValue::Object(payload));
    }
    Ok(out)
}

fn dialogue_ctda_payload_value(
    plugin: &ParsedPlugin,
    raw: &[u8],
) -> PyResult<JsonMap<String, JsonValue>> {
    let mut payload = JsonMap::new();
    payload.insert(
        "kind".to_string(),
        JsonValue::String("condition".to_string()),
    );
    payload.insert("size".to_string(), JsonValue::Number(raw.len().into()));
    payload.insert(
        "variant".to_string(),
        JsonValue::String(format!("size_{}", raw.len())),
    );
    payload.insert(
        "raw_hex".to_string(),
        JsonValue::String(hex::encode_upper(raw)),
    );
    if !raw.is_empty() {
        payload.insert(
            "operator_flags".to_string(),
            JsonValue::Number(raw[0].into()),
        );
    }
    if raw.len() >= 4 {
        payload.insert(
            "operator_padding_hex".to_string(),
            JsonValue::String(hex::encode_upper(&raw[1..4])),
        );
    }
    if raw.len() >= 8 {
        let value = f32::from_le_bytes([raw[4], raw[5], raw[6], raw[7]]);
        payload.insert("comparison_value".to_string(), serde_json::json!(value));
    }
    if raw.len() >= 10 {
        let function_index = u16::from_le_bytes([raw[8], raw[9]]);
        payload.insert(
            "function_index".to_string(),
            JsonValue::Number(function_index.into()),
        );
    }
    if raw.len() >= 12 {
        payload.insert(
            "function_padding_hex".to_string(),
            JsonValue::String(hex::encode_upper(&raw[10..12])),
        );
    }
    if raw.len() >= 16 {
        let parameter_1 = u32::from_le_bytes([raw[12], raw[13], raw[14], raw[15]]);
        payload.insert(
            "parameter_1".to_string(),
            dialogue_condition_parameter_value(plugin, parameter_1),
        );
    }
    if raw.len() >= 20 {
        let parameter_2 = u32::from_le_bytes([raw[16], raw[17], raw[18], raw[19]]);
        payload.insert(
            "parameter_2".to_string(),
            dialogue_condition_parameter_value(plugin, parameter_2),
        );
    }
    if raw.len() > 20 {
        payload.insert(
            "tail_hex".to_string(),
            JsonValue::String(hex::encode_upper(&raw[20..])),
        );
        if raw.len() >= 24 {
            let tail_uint32 = u32::from_le_bytes([
                raw[raw.len() - 4],
                raw[raw.len() - 3],
                raw[raw.len() - 2],
                raw[raw.len() - 1],
            ]);
            payload.insert(
                "tail_uint32".to_string(),
                JsonValue::Number(tail_uint32.into()),
            );
        }
    }
    Ok(payload)
}

fn dialogue_condition_parameter_value(plugin: &ParsedPlugin, raw: u32) -> JsonValue {
    let mut payload = JsonMap::new();
    payload.insert("raw".to_string(), JsonValue::String(format!("{raw:08X}")));
    payload.insert(
        "reference".to_string(),
        dialogue_form_ref_value(plugin, raw),
    );
    JsonValue::Object(payload)
}

fn dialogue_form_ref_value(plugin: &ParsedPlugin, raw: u32) -> JsonValue {
    let (plugin_name, object_id, raw_value, missing_index) =
        form_ref_from_raw_native(raw, &plugin.header.masters, &plugin.plugin_name);
    let mut payload = JsonMap::new();
    payload.insert(
        "plugin".to_string(),
        plugin_name
            .map(JsonValue::String)
            .unwrap_or(JsonValue::Null),
    );
    payload.insert(
        "object_id".to_string(),
        JsonValue::String(format!("{object_id:06X}")),
    );
    payload.insert(
        "raw".to_string(),
        raw_value
            .map(|value| JsonValue::String(format!("{value:08X}")))
            .unwrap_or(JsonValue::Null),
    );
    payload.insert(
        "missing_index".to_string(),
        missing_index
            .map(|value| JsonValue::Number(value.into()))
            .unwrap_or(JsonValue::Null),
    );
    JsonValue::Object(payload)
}

// ---------------------------------------------------------------------------
// Language / localization helpers. Language lookup helpers live here because
// they are also consumed by handle entrypoints and model.rs.
// ---------------------------------------------------------------------------

fn resolve_localized_string_with_language(
    strings: &LocalizedStringsState,
    string_id: u32,
    language: Option<&str>,
) -> Option<String> {
    if let Some(language) = language {
        if let Some(table) = localized_table_for_language(strings, language) {
            if let Some(value) = table.get(&string_id) {
                return Some(value.clone());
            }
        }
    }
    if let Some(table) = strings.by_language.get(strings.default_language.as_str()) {
        if let Some(value) = table.get(&string_id) {
            return Some(value.clone());
        }
    }
    if let Some(table) = strings.by_language.get("en") {
        if let Some(value) = table.get(&string_id) {
            return Some(value.clone());
        }
    }
    for table in strings.by_language.values() {
        if let Some(value) = table.get(&string_id) {
            return Some(value.clone());
        }
    }
    None
}

fn normalize_language_key(language: &str) -> String {
    let normalized = language
        .trim()
        .to_ascii_lowercase()
        .replace('-', "_")
        .replace(' ', "_");
    match normalized.as_str() {
        "chinese" | "cn" => "cn",
        "chinesesimplified" | "chinese_simplified" | "zhhans" => "zhhans",
        "chinesetraditional" | "chinese_traditional" | "zhhant" => "zhhant",
        "german" | "de" => "de",
        "english" | "en" => "en",
        "spanish" | "es" => "es",
        "spanish_mexico" | "esmx" => "esmx",
        "french" | "fr" => "fr",
        "italian" | "it" => "it",
        "japanese" | "ja" => "ja",
        "korean" | "ko" => "ko",
        "polish" | "pl" => "pl",
        "portuguese_brazil" | "ptbr" => "ptbr",
        "russian" | "ru" => "ru",
        _ => normalized.as_str(),
    }
    .to_string()
}

fn language_display_name_native(language: &str) -> String {
    match normalize_language_key(language).as_str() {
        "cn" => "Chinese",
        "zhhans" => "ChineseSimplified",
        "zhhant" => "ChineseTraditional",
        "de" => "German",
        "en" => "English",
        "es" => "Spanish",
        "esmx" => "Spanish_Mexico",
        "fr" => "French",
        "it" => "Italian",
        "ja" => "Japanese",
        "ko" => "Korean",
        "pl" => "Polish",
        "ptbr" => "Portuguese_Brazil",
        "ru" => "Russian",
        other => other,
    }
    .to_string()
}

fn localized_table_for_language<'a>(
    strings: &'a LocalizedStringsState,
    language: &str,
) -> Option<&'a HashMap<u32, String>> {
    let trimmed = language.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(table) = strings.by_language.get(trimmed) {
        return Some(table);
    }
    let normalized = normalize_language_key(trimmed);
    if let Some(table) = strings.by_language.get(normalized.as_str()) {
        return Some(table);
    }
    let display = language_display_name_native(trimmed);
    if let Some(table) = strings.by_language.get(display.as_str()) {
        return Some(table);
    }
    strings.by_language.iter().find_map(|(key, table)| {
        if key.eq_ignore_ascii_case(trimmed) || normalize_language_key(key.as_str()) == normalized {
            Some(table)
        } else {
            None
        }
    })
}

fn resolve_localized_string<'a>(
    strings: &'a LocalizedStringsState,
    string_id: u32,
) -> Option<&'a String> {
    let lang = &strings.default_language;
    strings
        .by_language
        .get(lang)
        .and_then(|table| table.get(&string_id))
}

// ---------------------------------------------------------------------------
// Rust-native record payload serializer (hot path).
// Works over &ParsedRecord + a shared PyAny plugin proxy.
// ---------------------------------------------------------------------------

fn schema_game_for_parsed(plugin: &ParsedPlugin) -> Option<String> {
    let game = plugin.game.as_ref()?.trim();
    if game.is_empty() {
        return None;
    }
    Some(game.to_ascii_lowercase())
}

fn lookup_subrecord_spec_for_parsed<'a>(
    schema: Option<&'a CompiledSchema>,
    record_spec: Option<&'a SchemaRecordJson>,
    signature: &str,
    occurrence: usize,
) -> Option<(&'a SchemaSubrecordJson, &'a CompiledSchema)> {
    let schema = schema?;
    let record_spec = record_spec?;
    let sub = schema_subrecord_spec(record_spec, signature, occurrence)?;
    Some((sub, schema))
}

/// Publish synthetic context aliases for sibling-subrecord union deciders
/// whose selector names don't match the standard field-name population path.
///
/// xEdit decoders read sibling subrecord values via signature lookups, e.g.
/// ``Container.ElementNativeValues['EPFT']`` for ``wbEPFDDecider`` or
/// ``KNAM.EditValue`` for ``wbAECHDataDecider``. The runtime context
/// otherwise only carries the field names emitted by each subrecord's spec
/// (``type`` for EPFT, ``descriptor_type`` for SNDR.CNAM, etc.), so union
/// conditions referencing ``epft`` / ``knam_edit_value`` / ``cnam_edit_value``
/// would never match. This inserts the decoded sibling value under the alias
/// key the union conditions reference.
///
/// The sources are single-field ``parsed`` specs whose post-compact
/// ``field_payload`` is a bare scalar (Number for EPFT uint8, String for
/// AECH.KNAM / SNDR.CNAM uint32-with-enum); an Object containing the source
/// field is accepted too.
fn publish_sibling_decider_context_aliases(
    record_sig: &str,
    subrecord_sig: &str,
    field_payload: &serde_json::Value,
    context: &mut HashMap<String, serde_json::Value>,
) {
    let alias = match (record_sig, subrecord_sig) {
        ("PERK", "EPFT") => Some(("type", "epft")),
        ("AECH", "KNAM") => Some(("type", "knam_edit_value")),
        ("SNDR", "CNAM") => Some(("descriptor_type", "cnam_edit_value")),
        _ => None,
    };
    if let Some((source, target)) = alias {
        let value: Option<serde_json::Value> = match field_payload {
            serde_json::Value::Object(m) => m.get(source).cloned(),
            serde_json::Value::Null => None,
            scalar => Some(scalar.clone()),
        };
        if let Some(v) = value {
            context.insert(target.to_string(), v);
        }
    }
    // PERK.PRKE.Type drives the wbPerkDATADecider union on the following DATA
    // (Effect Data) subrecord. Standard Object→context merge runs over the
    // *compact* payload, where authoring_field_is_default_json strips Type=0
    // (QuestStage). Without this alias, a row with PRKE.Type=0 sees no key
    // for "Type" and the decider either (a) falls through to raw_hex when
    // there is no prior PRKE in scope, or (b) reuses a stale "Type" value
    // from the previous Effect's PRKE. Always publish "Type" — using the
    // decoded label when present, "QuestStage" as the type=0 default —
    // so each Effect's union dispatches against its own PRKE.
    if (record_sig, subrecord_sig) == ("PERK", "PRKE") {
        let resolved = match field_payload {
            serde_json::Value::Object(m) => m
                .get("Type")
                .cloned()
                .unwrap_or_else(|| serde_json::Value::String("QuestStage".to_string())),
            _ => serde_json::Value::String("QuestStage".to_string()),
        };
        context.insert("Type".to_string(), resolved);
    }
}

/// Publish ``editor_id_prefix`` (first char of EDID) into record context so
/// union conditions keyed on EditorID prefix can resolve. xEdit's
/// ``wbGMSTUnionDecider`` reads ``Container.RecordBySignature['EDID'].Value[1]``
/// to pick a GMST.DATA variant (``s``→String, ``i``→Int, ``f``→Float,
/// ``b``→Bool, ``u``→UInt32). The runtime's standard field merge only inserts
/// Object payloads into context, but EDID is a single-field parsed spec that
/// decodes as a bare String, so ``editor_id`` never lands in context. This
/// helper bridges the gap by extracting the first char and publishing it
/// under ``editor_id_prefix``.
fn publish_editor_id_prefix_alias(
    subrecord_sig: &str,
    field_payload: &serde_json::Value,
    context: &mut HashMap<String, serde_json::Value>,
) {
    if subrecord_sig != "EDID" {
        return;
    }
    let editor_id = match field_payload {
        serde_json::Value::String(s) => Some(s.as_str()),
        serde_json::Value::Object(m) => m.get("editor_id").and_then(|v| v.as_str()),
        _ => None,
    };
    if let Some(id) = editor_id {
        if let Some(first) = id.chars().next() {
            context.insert(
                "editor_id_prefix".to_string(),
                serde_json::Value::String(first.to_string()),
            );
        }
    }
}

/// When the current scope holds multiple specs for
/// ``signature`` whose codecs disagree on byte length, pick the unique one
/// that accepts ``payload_len``. Returns ``None`` if zero or two-plus
/// candidates accept the length, leaving disambiguation to occurrence-based
/// dispatch.
///
/// Motivating case: TERM has two SNAM specs at top-level — Looping Sound
/// (formid, exactly 4 bytes) and Marker Parameters (array_struct, multiples
/// of 24 bytes). Records that omit Looping Sound but carry Marker Parameters
/// otherwise route to the wrong spec because the occurrence counter starts
/// at 0 and picks the first declared spec.
fn pick_unique_spec_by_payload_length<'a>(
    record_spec: &'a SchemaRecordJson,
    signature: &str,
    current_scope: Option<&str>,
    payload_len: usize,
) -> Option<&'a SchemaSubrecordJson> {
    // First try same-scope disambiguation (TERM SNAM motivating case: two
    // top-level SNAM specs at scope=None disambiguate by payload length).
    let same_scope: Vec<&SchemaSubrecordJson> = record_spec
        .subrecords
        .iter()
        .filter(|s| s.id == signature && s.scope_id.as_deref() == current_scope)
        .collect();
    if same_scope.len() >= 2 {
        if let Some(unique) = unique_spec_accepting_length(&same_scope, payload_len) {
            return Some(unique);
        }
    }
    // Fall through to cross-scope disambiguation. LENS motivating case:
    // optional top-level wbFloat(DNAM) is missing in some records, so the
    // first array DNAM dispatches at occurrence 0 and would otherwise mis-
    // route to the top-level float32 spec. The 16-byte zstring payload is
    // accepted only by the repeatable DNAM in scope=lens_flare_sprites, so
    // payload-length disambiguation correctly routes there. Only fires when
    // multiple scopes contain a spec for ``signature``.
    let all_specs: Vec<&SchemaSubrecordJson> = record_spec
        .subrecords
        .iter()
        .filter(|s| s.id == signature)
        .collect();
    if all_specs.len() < 2 {
        return None;
    }
    let distinct_scopes = all_specs
        .iter()
        .map(|s| s.scope_id.as_deref())
        .collect::<std::collections::HashSet<_>>()
        .len();
    if distinct_scopes < 2 {
        return None;
    }
    unique_spec_accepting_length(&all_specs, payload_len)
}

fn unique_spec_accepting_length<'a>(
    candidates: &[&'a SchemaSubrecordJson],
    payload_len: usize,
) -> Option<&'a SchemaSubrecordJson> {
    let mut accepted: Vec<&SchemaSubrecordJson> = Vec::with_capacity(candidates.len());
    for spec in candidates {
        let codec = spec.codec.as_deref().unwrap_or("");
        match self::authoring::authoring_serialize::codec_accepts_payload_length(codec, payload_len)
        {
            Some(false) => {}
            _ => accepted.push(*spec),
        }
    }
    if accepted.len() == 1 {
        Some(accepted[0])
    } else {
        None
    }
}

/// Scope-aware spec lookup. Returns the chosen ``(spec,
/// scope_id)`` pair. ``current_scope`` is the dispatcher's tracked scope
/// (the scope of the most-recently-seen subrecord whose spec lived in a
/// non-``None`` scope). The fallback chain:
///
/// 0. If the current scope holds multiple specs for ``signature`` and the
///    payload byte length uniquely identifies one, use it (overrides the
///    occurrence counter).
/// 1. If the current scope contains a spec for ``signature``, use it.
///    Multi-occurrence specs within the same scope use the per-scope
///    occurrence counter.
/// 2. Otherwise, try every other scope present on the record (in spec
///    declaration order, deduplicated) — the first one with an unconsumed
///    spec wins.
/// 3. Otherwise, return ``None`` so the caller can fall back to the
///    occurrence-based lookup.
///
/// The returned scope id is what the caller should bump in
/// ``occurrence_counts`` and adopt as ``current_scope`` for the next sig.
fn dispatch_subrecord_spec_in_scope<'a>(
    record_spec: &'a SchemaRecordJson,
    signature: &str,
    payload_len: usize,
    current_scope: Option<&str>,
    occurrence_counts: &HashMap<(Option<String>, String), usize>,
) -> Option<(&'a SchemaSubrecordJson, Option<String>)> {
    if let Some(spec) =
        pick_unique_spec_by_payload_length(record_spec, signature, current_scope, payload_len)
    {
        // Use the spec's own scope, not current_scope — the disambiguator may
        // have crossed a scope boundary (LENS case: array DNAM picked up while
        // current_scope is still None because top-level CNAM/DNAM were absent).
        return Some((spec, spec.scope_id.clone()));
    }
    if schema_has_subrecord_in_scope(record_spec, signature, current_scope) {
        let key = (current_scope.map(|s| s.to_string()), signature.to_string());
        let occ = *occurrence_counts.get(&key).unwrap_or(&0);
        if let Some(spec) =
            schema_subrecord_spec_in_scope(record_spec, signature, current_scope, occ)
        {
            return Some((spec, current_scope.map(|s| s.to_string())));
        }
    }
    // Fall through: try every distinct scope (in declaration order, dedup'd).
    let mut tried: HashSet<Option<&str>> = HashSet::new();
    tried.insert(current_scope);
    for spec in &record_spec.subrecords {
        if spec.id != signature {
            continue;
        }
        let scope = spec.scope_id.as_deref();
        if !tried.insert(scope) {
            continue;
        }
        let key = (scope.map(|s| s.to_string()), signature.to_string());
        let occ = *occurrence_counts.get(&key).unwrap_or(&0);
        if let Some(found) = schema_subrecord_spec_in_scope(record_spec, signature, scope, occ) {
            return Some((found, scope.map(|s| s.to_string())));
        }
    }
    None
}

fn record_context_from_parsed(record: &ParsedRecord) -> RecordContextPayload {
    (
        record.signature.to_string(),
        record.form_version,
        record.version2,
    )
}

/// Pure-Rust, GIL-free serialization of a single ESP record to `serde_json::Value`.
/// Decoding, FormID resolution, enum lookup, and localized-string resolution all
/// run in Rust, so it can run inside the rayon export.
///
/// Output is the compact authoring-dir record shape:
///   * `form_id` — "XXXXXX" or "XXXXXX:Plugin.esm"
///   * `flags`, `version_control`, `form_version`, `version2` — omitted when zero
///   * `raw_payload_hex` — present only for compressed/undecodable records
///   * `parse_error` — present only on parse failure
///   * `eid` — top-level EditorID string, when the record has an EDID subrecord
///   * `fields` — array of `{<sig>: <payload>}` dicts (authoring-dir format),
///     excluding EDID because `eid` owns that universal record identity field
pub fn serialize_record_payload_to_json(
    record: &ParsedRecord,
    plugin: &ParsedPlugin,
    strings: &LocalizedStringsState,
) -> serde_json::Value {
    let masters: &[String] = plugin.header.masters.as_slice();
    let plugin_name: &str = plugin.plugin_name.as_str();

    let mut map = serde_json::Map::new();

    // form_id
    map.insert(
        "form_id".to_string(),
        serde_json::Value::String(format_record_form_id_native(
            record.form_id,
            masters,
            plugin_name,
        )),
    );

    // flags — omit when zero (authoring-dir compact mode)
    if record.flags != 0 {
        map.insert(
            "flags".to_string(),
            serde_json::Value::String(format!("{:08X}", record.flags)),
        );
    }
    // version_control — omit when zero
    if record.version_control != 0 {
        map.insert(
            "version_control".to_string(),
            serde_json::Value::Number(record.version_control.into()),
        );
    }
    // form_version — omit when zero
    if let Some(fv) = record.form_version {
        if fv != 0 {
            map.insert(
                "form_version".to_string(),
                serde_json::Value::Number(fv.into()),
            );
        }
    }
    // version2 — omit when zero
    if let Some(v2) = record.version2 {
        if v2 != 0 {
            map.insert("version2".to_string(), serde_json::Value::Number(v2.into()));
        }
    }
    // raw_payload_hex
    if let Some(raw_bytes) = &record.raw_payload {
        map.insert(
            "raw_payload_hex".to_string(),
            serde_json::Value::String(hex::encode_upper(raw_bytes)),
        );
    }
    // parse_error
    if let Some(parse_error) = &record.parse_error {
        map.insert(
            "parse_error".to_string(),
            serde_json::Value::String(parse_error.clone()),
        );
    }

    // Resolve schema spec for this record (once per record).
    let schema_game = schema_game_for_parsed(plugin);
    let schema = schema_game
        .as_deref()
        .and_then(|game| compiled_schema_for_game(game).ok());
    let record_spec = schema
        .as_ref()
        .and_then(|schema| schema_record_spec(schema.as_ref(), record.signature.as_str()).cloned());

    // Build context for conditional decode (union conditions, etc.).
    let mut context: HashMap<String, serde_json::Value> = HashMap::new();
    context.insert(
        "record_signature".to_string(),
        serde_json::Value::String(record.signature.to_string()),
    );
    match record.form_version {
        Some(v) => context.insert(
            "record_form_version".to_string(),
            serde_json::Value::Number(v.into()),
        ),
        None => context.insert("record_form_version".to_string(), serde_json::Value::Null),
    };
    match record.version2 {
        Some(v) => context.insert(
            "record_version2".to_string(),
            serde_json::Value::Number(v.into()),
        ),
        None => context.insert("record_version2".to_string(), serde_json::Value::Null),
    };

    let lazy_subrecords = lazy_subrecords_for_record(record).ok().flatten();
    let subrecords: &[ParsedSubrecord] = lazy_subrecords
        .as_deref()
        .unwrap_or(record.subrecords.as_slice());

    let mut fields: Vec<CompactAuthoringEntry> = Vec::with_capacity(subrecords.len());
    // Dispatch keys by (scope_id, sig) so PACK and similar
    // records can route nested subrecords to scope-specific specs (Combat
    // Style FormID at top-level vs Value union inside Package Data, etc.).
    // ``occurrence_counts_by_scope`` is keyed on (scope_id, sig). Records
    // without a schema, or sigs absent from the schema, fall back to the
    // global counter in ``occurrence_counts``.
    let mut occurrence_counts: HashMap<&str, usize> = HashMap::new();
    let mut occurrence_counts_by_scope: HashMap<(Option<String>, String), usize> = HashMap::new();
    let mut current_scope: Option<String> = None;

    for sub in subrecords {
        let signature = sub.signature.as_str();
        if signature == "EDID" {
            let eid = decode_cp1252(&sub.data);
            if !eid.is_empty() {
                publish_editor_id_prefix_alias(
                    "EDID",
                    &serde_json::Value::String(eid.clone()),
                    &mut context,
                );
                map.insert("eid".to_string(), serde_json::Value::String(eid));
            }
            occurrence_counts.insert(
                signature,
                occurrence_counts.get(signature).copied().unwrap_or(0) + 1,
            );
            continue;
        }

        let scoped: Option<(&SchemaSubrecordJson, Option<String>)> =
            record_spec.as_ref().and_then(|rec| {
                dispatch_subrecord_spec_in_scope(
                    rec,
                    signature,
                    sub.data.len(),
                    current_scope.as_deref(),
                    &occurrence_counts_by_scope,
                )
            });

        let occurrence = *occurrence_counts.get(signature).unwrap_or(&0);

        let spec_json: Option<(&SchemaSubrecordJson, &CompiledSchema)> =
            match (&scoped, schema.as_ref()) {
                (Some((spec, _)), Some(sch)) => Some((*spec, sch.as_ref())),
                _ => lookup_subrecord_spec_for_parsed(
                    schema.as_deref(),
                    record_spec.as_ref(),
                    signature,
                    occurrence,
                ),
            };

        // Compute the DecodeSpec once (no Python).
        let decode_spec: Option<crate::DecodeSpec> =
            spec_json.as_ref().and_then(|(sub_spec, schema)| {
                self::authoring::authoring_serialize::schema_subrecord_to_decode_spec(
                    sub_spec, schema,
                )
            });

        let diagnostics_context;
        let diagnostics_context_ref =
            if self::authoring::authoring_serialize::export_decode_diagnostics_enabled() {
                diagnostics_context = format!(
                    "record_sig={} form_id={:08X} subrecord_sig={} occurrence={} subrecord_bytes={}",
                    record.signature,
                    record.form_id,
                    signature,
                    occurrence,
                    sub.data.len()
                );
                Some(diagnostics_context.as_str())
            } else {
                None
            };

        // Surface the host record signature as semantic_type
        // for VMAD subrecords so compact_vmad_payload_json can dispatch the
        // right fragment decoder (INFO/PACK/QUST/SCEN/PERK/TERM). Existing
        // typed semantic_type values (formid, formid_array, ...) take priority.
        let vmad_record_hint = if signature == "VMAD" && sub.semantic_type.is_none() {
            Some(record.signature.as_str())
        } else {
            None
        };
        let semantic_for_subrecord = sub.semantic_type.as_deref().or(vmad_record_hint);
        let field_payload = self::authoring::authoring_serialize::compact_subrecord_to_json(
            &sub.data,
            decode_spec.as_ref(),
            spec_json
                .as_ref()
                .map(|(sub_spec, schema)| (*sub_spec, *schema)),
            strings,
            masters,
            plugin_name,
            semantic_for_subrecord,
            Some(&context),
            diagnostics_context_ref,
        );

        // Build the authoring key from the schema label when available. Raw
        // signatures still import as a compatibility fallback.
        let field_key = match (spec_json.as_ref(), record_spec.as_ref()) {
            (Some((spec, _)), Some(record_spec)) => {
                schema_subrecord_authoring_key(record_spec, spec)
            }
            (Some((spec, _)), None) => schema_subrecord_key(record.signature.as_str(), spec),
            _ => signature.to_string(),
        };

        // Compact the payload (strip envelope wrapper for authoring-dir format).
        let compact_value =
            compact_field_payload_json(&field_payload, spec_json.as_ref().map(|(s, _)| *s));

        let group_key = spec_json.as_ref().and_then(|(spec, _)| {
            if spec.authoring_layout.as_deref() == Some("row_group") {
                spec.authoring_key.clone()
            } else {
                None
            }
        });
        let (group_order, group_anchor) = match (spec_json.as_ref(), record_spec.as_ref()) {
            (Some((spec, _)), Some(record_spec))
                if spec.authoring_layout.as_deref() == Some("row_group") =>
            {
                let group_key = spec.authoring_key.as_deref();
                let mut anchor_id: Option<&str> = None;
                let mut order = None;
                let mut group_index = 0usize;
                for candidate in &record_spec.subrecords {
                    if candidate.authoring_layout.as_deref() != Some("row_group")
                        || candidate.authoring_key.as_deref() != group_key
                    {
                        continue;
                    }
                    if anchor_id.is_none() {
                        anchor_id = Some(candidate.id.as_str());
                    }
                    if candidate.id == spec.id && order.is_none() {
                        order = Some(group_index);
                    }
                    group_index += 1;
                }
                (
                    order,
                    anchor_id
                        .map(|candidate| candidate == spec.id.as_str())
                        .unwrap_or(false),
                )
            }
            _ => (None, false),
        };
        fields.push(CompactAuthoringEntry {
            signature: signature.to_string(),
            key: field_key,
            value: compact_value,
            group_key,
            group_order,
            group_anchor,
            ordinal: fields.len(),
        });

        // Add decoded fields to context so subsequent subrecords' union
        // conditions can reference them. Most-recent value wins: this is
        // what sibling-subrecord deciders like wbPubPackCNAMDecider need —
        // each PACK Data Input Value entry rewrites context["type"] from
        // its own ANAM, so the immediately-following CNAM picks the right
        // variant. It also lets ANAM(scope=package_data) shadow PKDT's
        // earlier "type" field for the duration of the Package Data block.
        if let serde_json::Value::Object(ref m) = field_payload {
            for (k, v) in m {
                context.insert(k.clone(), v.clone());
            }
        }
        publish_sibling_decider_context_aliases(
            record.signature.as_str(),
            signature,
            &field_payload,
            &mut context,
        );

        occurrence_counts.insert(signature, occurrence + 1);
        if let Some((_, chosen_scope)) = scoped {
            *occurrence_counts_by_scope
                .entry((chosen_scope.clone(), signature.to_string()))
                .or_insert(0) += 1;
            current_scope = chosen_scope;
        }
    }

    let mut fields = group_schema_row_group_fields_json(fields);
    if is_starfield_game_name(plugin.game.as_deref()) {
        fields = group_starfield_component_fields_json(fields);
    }
    fields = group_object_template_fields_json(record.signature.as_str(), fields);
    map.insert("fields".to_string(), serde_json::Value::Array(fields));
    serde_json::Value::Object(map)
}

/// GIL-free equivalent of `compact_field_payload` — strips the outer
/// preservation-mode envelope from a subrecord payload for authoring-dir output.
/// Matches the shape produced by the existing Python/PyAny `compact_field_payload`.
fn compact_field_payload_json(
    payload: &serde_json::Value,
    _spec: Option<&SchemaSubrecordJson>,
) -> serde_json::Value {
    let serde_json::Value::Object(m) = payload else {
        return payload.clone();
    };
    // Detect the preservation-mode wrapper: {value, raw_hex, semantic_type?}.
    // If present, return the unwrapped value. Otherwise return the payload as-is.
    // Hybrid kinds (parsed_with_raw_fallback) round-trip via the encoder's
    // `truncate_trailing_absent` path (variable-length structs) instead of
    // keeping raw_hex in the YAML, so we drop the wrapper unconditionally.
    let has_wrapper = m.contains_key("raw_hex") && (m.contains_key("value") || m.len() <= 3);
    if has_wrapper {
        if let Some(value) = m.get("value") {
            // A decode that produced no members describes none of the payload,
            // and re-encoding that empty mapping yields a zero-length
            // subrecord — FO4 `MODT` (codec `model_info`) degrades this way and
            // a zero-length MODT faults TESRace::Load. Keep the bytes instead.
            if !value.as_object().is_some_and(serde_json::Map::is_empty) {
                return value.clone();
            }
            let mut raw_only = serde_json::Map::new();
            for key in ["raw_hex", "semantic_type", "display_value"] {
                if let Some(kept) = m.get(key) {
                    raw_only.insert(key.to_string(), kept.clone());
                }
            }
            return serde_json::Value::Object(raw_only);
        }
        // raw_only shape: {raw_hex, semantic_type?, display_value?} — keep as-is.
    }
    payload.clone()
}

#[derive(Clone)]
struct CompactAuthoringEntry {
    signature: String,
    key: String,
    value: serde_json::Value,
    group_key: Option<String>,
    group_order: Option<usize>,
    group_anchor: bool,
    ordinal: usize,
}

impl CompactAuthoringEntry {
    fn into_json(self) -> serde_json::Value {
        compact_entry_with_key(self.key.as_str(), self.value)
    }
}

fn compact_entry_with_key(key: &str, value: serde_json::Value) -> serde_json::Value {
    let mut entry = serde_json::Map::new();
    entry.insert(key.to_string(), value);
    serde_json::Value::Object(entry)
}

fn authoring_group_display_key(group_key: &str) -> String {
    let stem = group_key
        .strip_prefix("xedit_group_")
        .or_else(|| group_key.strip_prefix("group_"))
        .unwrap_or(group_key)
        .trim_matches('_');
    let mut out = String::new();
    for part in stem.split('_').filter(|part| !part.is_empty()) {
        let mut chars = part.chars();
        if let Some(first) = chars.next() {
            out.extend(first.to_uppercase());
            out.push_str(chars.as_str());
        }
    }
    if out.is_empty() {
        return group_key.to_string();
    }
    if out.ends_with('y') {
        out.pop();
        out.push_str("ies");
    } else if !out.ends_with('s') {
        out.push('s');
    }
    out
}

fn row_group_entry_json(group_key: &str, rows: Vec<serde_json::Value>) -> serde_json::Value {
    compact_entry_with_key(
        authoring_group_display_key(group_key).as_str(),
        serde_json::Value::Array(rows),
    )
}

fn build_schema_row_group_row_json(
    current_row: &mut Vec<CompactAuthoringEntry>,
    current_signatures: &mut HashSet<String>,
) -> serde_json::Value {
    current_row.sort_by_key(|field| (field.group_order.unwrap_or(usize::MAX), field.ordinal));
    let mut row = serde_json::Map::new();
    for field in std::mem::take(current_row) {
        row.insert(field.key, field.value);
    }
    current_signatures.clear();
    serde_json::Value::Object(row)
}

fn flush_schema_row_group_json(
    grouped: &mut Vec<serde_json::Value>,
    current_key: &mut Option<String>,
    current_row: &mut Vec<CompactAuthoringEntry>,
    current_signatures: &mut HashSet<String>,
    rows: &mut Vec<serde_json::Value>,
) {
    if !current_row.is_empty() {
        rows.push(build_schema_row_group_row_json(
            current_row,
            current_signatures,
        ));
    }
    if let Some(key) = current_key.take() {
        if !rows.is_empty() {
            grouped.push(row_group_entry_json(key.as_str(), std::mem::take(rows)));
        }
    }
}

fn group_schema_row_group_fields_json(
    fields: Vec<CompactAuthoringEntry>,
) -> Vec<serde_json::Value> {
    let mut grouped = Vec::with_capacity(fields.len());
    let mut current_key: Option<String> = None;
    let mut current_row: Vec<CompactAuthoringEntry> = Vec::new();
    let mut current_signatures = HashSet::new();
    let mut rows: Vec<serde_json::Value> = Vec::new();
    let mut current_last_order: Option<usize> = None;

    for field in fields {
        let group_key = field.group_key.as_deref();
        if group_key.is_none()
            || group_key == Some("group_object_template")
            || group_key == Some("xedit_group_object_template")
        {
            flush_schema_row_group_json(
                &mut grouped,
                &mut current_key,
                &mut current_row,
                &mut current_signatures,
                &mut rows,
            );
            current_last_order = None;
            grouped.push(field.into_json());
            continue;
        }

        let group_key = group_key.unwrap();
        if current_key.as_deref() != Some(group_key) {
            flush_schema_row_group_json(
                &mut grouped,
                &mut current_key,
                &mut current_row,
                &mut current_signatures,
                &mut rows,
            );
            current_last_order = None;
            current_key = Some(group_key.to_string());
        }
        let order_rewinds = match (current_last_order, field.group_order) {
            (Some(current), Some(next)) => next < current,
            _ => false,
        };
        if (field.group_anchor || order_rewinds) && !current_row.is_empty() {
            rows.push(build_schema_row_group_row_json(
                &mut current_row,
                &mut current_signatures,
            ));
            current_last_order = None;
        } else if current_signatures.contains(field.signature.as_str()) && !current_row.is_empty() {
            rows.push(build_schema_row_group_row_json(
                &mut current_row,
                &mut current_signatures,
            ));
            current_last_order = None;
        }
        current_signatures.insert(field.signature.clone());
        if let Some(order) = field.group_order {
            current_last_order = Some(order);
        }
        current_row.push(field);
    }
    flush_schema_row_group_json(
        &mut grouped,
        &mut current_key,
        &mut current_row,
        &mut current_signatures,
        &mut rows,
    );
    grouped
}

fn compact_entry_key_is(key: &str, raw: &str, label: &str) -> bool {
    key == raw || key == label || key == authoring_camel_case(label)
}

fn compact_authoring_key_is_eid(key: &str) -> bool {
    compact_entry_key_is(key, "EDID", "Editor ID")
}

fn flush_object_template_group_json(
    grouped: &mut Vec<serde_json::Value>,
    templates: &mut Option<Vec<serde_json::Value>>,
) {
    if let Some(items) = templates.take() {
        grouped.push(compact_entry_with_key(
            "ObjectTemplates",
            serde_json::Value::Array(items),
        ));
    }
}

fn flush_pending_object_template_fields_json(
    grouped: &mut Vec<serde_json::Value>,
    pending_count: &mut Option<serde_json::Value>,
    pending_editor_only: &mut Option<serde_json::Value>,
    pending_name: &mut Option<serde_json::Value>,
) {
    if let Some(entry) = pending_count.take() {
        grouped.push(entry);
    }
    if let Some(entry) = pending_editor_only.take() {
        grouped.push(entry);
    }
    if let Some(entry) = pending_name.take() {
        grouped.push(entry);
    }
}

fn group_object_template_fields_json(
    record_signature: &str,
    fields: Vec<serde_json::Value>,
) -> Vec<serde_json::Value> {
    let _ = record_signature;
    if !fields.iter().any(|entry| {
        single_key_mapping_json(entry)
            .map(|(key, _)| compact_entry_key_is(key, "OBTS", "Object Mod Template Item"))
            .unwrap_or(false)
    }) {
        return fields;
    }

    let mut grouped = Vec::with_capacity(fields.len());
    let mut templates: Option<Vec<serde_json::Value>> = None;
    let mut pending_count: Option<serde_json::Value> = None;
    let mut pending_editor_only: Option<serde_json::Value> = None;
    let mut pending_name: Option<serde_json::Value> = None;

    for entry in fields {
        let Some((key, value)) = single_key_mapping_json(&entry) else {
            flush_object_template_group_json(&mut grouped, &mut templates);
            flush_pending_object_template_fields_json(
                &mut grouped,
                &mut pending_count,
                &mut pending_editor_only,
                &mut pending_name,
            );
            grouped.push(entry);
            continue;
        };

        if compact_entry_key_is(key, "OBTE", "Count") {
            flush_object_template_group_json(&mut grouped, &mut templates);
            flush_pending_object_template_fields_json(
                &mut grouped,
                &mut pending_count,
                &mut pending_editor_only,
                &mut pending_name,
            );
            pending_count = Some(entry);
            continue;
        }

        if (pending_count.is_some() || templates.is_some())
            && compact_entry_key_is(key, "OBTF", "Editor Only")
        {
            pending_editor_only = Some(entry);
            continue;
        }

        if (pending_count.is_some() || templates.is_some())
            && compact_entry_key_is(key, "FULL", "Name")
        {
            pending_name = Some(entry);
            continue;
        }

        if compact_entry_key_is(key, "OBTS", "Object Mod Template Item") {
            let templates = templates.get_or_insert_with(Vec::new);
            pending_count = None;
            let mut template = match value {
                serde_json::Value::Object(map) => map.clone(),
                other => {
                    let mut map = serde_json::Map::new();
                    map.insert("value".to_string(), other.clone());
                    map
                }
            };
            if pending_editor_only.take().is_some() {
                template.insert("IsEditorOnly".to_string(), serde_json::Value::Bool(true));
            }
            if let Some(name_entry) = pending_name.take() {
                if let Some((_, name_value)) = single_key_mapping_json(&name_entry) {
                    template.insert("Name".to_string(), name_value.clone());
                }
            }
            templates.push(serde_json::Value::Object(template));
            continue;
        }

        if templates.is_some() && compact_entry_key_is(key, "STOP", "Marker") {
            if let Some(serde_json::Value::Object(template)) =
                templates.as_mut().and_then(|items| items.last_mut())
            {
                template.insert("Marker".to_string(), value.clone());
                continue;
            }
        }

        flush_object_template_group_json(&mut grouped, &mut templates);
        flush_pending_object_template_fields_json(
            &mut grouped,
            &mut pending_count,
            &mut pending_editor_only,
            &mut pending_name,
        );
        grouped.push(entry);
    }

    flush_object_template_group_json(&mut grouped, &mut templates);
    flush_pending_object_template_fields_json(
        &mut grouped,
        &mut pending_count,
        &mut pending_editor_only,
        &mut pending_name,
    );
    grouped
}

fn is_starfield_game_name(game: Option<&str>) -> bool {
    game.map(|value| value.eq_ignore_ascii_case("starfield"))
        .unwrap_or(false)
}

fn single_key_mapping_json(value: &serde_json::Value) -> Option<(&str, &serde_json::Value)> {
    let serde_json::Value::Object(mapping) = value else {
        return None;
    };
    if mapping.len() != 1 {
        return None;
    }
    mapping
        .iter()
        .next()
        .map(|(key, value)| (key.as_str(), value))
}

fn starfield_component_authoring_key(component_type: &str) -> String {
    let mut text = component_type.trim().trim_end_matches('\0');
    if let Some(stripped) = text.strip_prefix("BGS") {
        text = stripped;
    }
    let mut out = String::new();
    for part in text
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|part| !part.is_empty())
    {
        let mut chars = part.chars();
        if let Some(first) = chars.next() {
            out.push(first.to_ascii_uppercase());
            out.extend(chars);
        }
    }
    if out.is_empty() {
        out.push_str("UnknownComponent");
    } else if !out.ends_with("Component") {
        out.push_str("Component");
    }
    out
}

fn starfield_component_type_from_authoring_key(key: &str) -> String {
    let stem = key.strip_suffix("Component").unwrap_or(key);
    format!("BGS{stem}_Component")
}

fn starfield_component_value_json(
    component_type: &str,
    payload_entries: &[serde_json::Value],
) -> serde_json::Value {
    let component_key = starfield_component_authoring_key(component_type);
    let mut body = serde_json::Map::new();
    body.insert(
        "Type".to_string(),
        serde_json::Value::String(component_type.to_string()),
    );

    let mut seen = HashSet::new();
    let mut direct_payload = true;
    for entry in payload_entries {
        let Some((key, value)) = single_key_mapping_json(entry) else {
            direct_payload = false;
            break;
        };
        if !seen.insert(key.to_string()) {
            direct_payload = false;
            break;
        }
        body.insert(key.to_string(), value.clone());
    }
    if !direct_payload {
        body.clear();
        body.insert(
            "Type".to_string(),
            serde_json::Value::String(component_type.to_string()),
        );
        body.insert(
            "fields".to_string(),
            serde_json::Value::Array(payload_entries.to_vec()),
        );
    }

    let mut typed = serde_json::Map::new();
    typed.insert(component_key, serde_json::Value::Object(body));
    serde_json::Value::Object(typed)
}

fn append_starfield_component_entry_json(
    fields: &mut Vec<serde_json::Value>,
    component: serde_json::Value,
) {
    if let Some(serde_json::Value::Object(last)) = fields.last_mut() {
        if last.len() == 1 {
            if let Some(serde_json::Value::Array(components)) = last.get_mut("Components") {
                components.push(component);
                return;
            }
        }
    }

    let mut entry = serde_json::Map::new();
    entry.insert(
        "Components".to_string(),
        serde_json::Value::Array(vec![component]),
    );
    fields.push(serde_json::Value::Object(entry));
}

fn starfield_component_type_from_field_json(field: &serde_json::Value) -> Option<&str> {
    let Some((key, value)) = single_key_mapping_json(field) else {
        return None;
    };
    if !matches!(key, "BFCB" | "Component Type" | "ComponentType") {
        return None;
    }
    value.as_str()
}

fn is_starfield_component_end_field_json(field: &serde_json::Value) -> bool {
    matches!(
        single_key_mapping_json(field),
        Some(("BFCE", _)) | Some(("End Marker", _)) | Some(("EndMarker", _))
    )
}

fn group_starfield_component_fields_json(fields: Vec<serde_json::Value>) -> Vec<serde_json::Value> {
    let mut grouped = Vec::with_capacity(fields.len());
    let mut index = 0usize;
    while index < fields.len() {
        let Some(component_type) = starfield_component_type_from_field_json(&fields[index]) else {
            grouped.push(fields[index].clone());
            index += 1;
            continue;
        };

        let mut end_index = index + 1;
        let mut found_end = false;
        while end_index < fields.len() {
            if is_starfield_component_end_field_json(&fields[end_index]) {
                found_end = true;
                break;
            }
            if starfield_component_type_from_field_json(&fields[end_index]).is_some() {
                break;
            }
            end_index += 1;
        }
        if !found_end {
            grouped.push(fields[index].clone());
            index += 1;
            continue;
        }

        let component =
            starfield_component_value_json(component_type, &fields[index + 1..end_index]);
        append_starfield_component_entry_json(&mut grouped, component);
        index = end_index + 1;
    }
    grouped
}

// ---------------------------------------------------------------------------
// Header manifest + write helpers, Rust-native twins.
// ---------------------------------------------------------------------------

const HEADER_FLAG_DEFINITIONS: [(&str, u32, &str); 3] = [
    ("master", 0x0000_0001, "Master File"),
    ("localized", 0x0000_0080, "Localized"),
    ("light", 0x0000_0200, "Light Plugin"),
];
const AUTHORING_PAYLOAD_MARKERS: [&str; 8] = [
    "value",
    "preservation_mode",
    "raw_hex",
    "fields",
    "rows",
    "variant",
    "semantic_type",
    "fill_missing_struct_fields",
];

fn looks_like_text(data: &[u8]) -> bool {
    if data.is_empty() {
        return true;
    }
    let mut end = data.len();
    while end > 0 && data[end - 1] == 0 {
        end -= 1;
    }
    let candidate = &data[..end];
    if candidate.is_empty() {
        return true;
    }
    if candidate.contains(&0) {
        return false;
    }
    let (decoded, _, _) = WINDOWS_1252.decode(candidate);
    let decoded = decoded.as_ref();
    let total = decoded.chars().count();
    if total == 0 {
        return true;
    }
    let printable = decoded
        .chars()
        .filter(|ch| ch.is_ascii_graphic() || ch.is_ascii_whitespace() || (*ch as u32) >= 0xA0)
        .count();
    (printable as f64 / total as f64) >= 0.85
}

fn native_preservation_mode(kind: &str) -> &'static str {
    // "custom_codec" falls through to "raw_only" — it delegates parse/write
    // to an external Rust module (e.g. esp_authoring_core::nvnm); until that
    // codec is wired through it round-trips raw bytes.
    match kind {
        "parsed" => "typed",
        "parsed_with_raw_fallback" => "hybrid",
        _ => "raw_only",
    }
}

fn native_layout_for_subrecord(subrecord: &SchemaSubrecordJson) -> Option<String> {
    if let Some(layout) = subrecord.authoring_layout.as_ref() {
        return Some(layout.clone());
    }
    if !subrecord.union_variants.is_empty() {
        return Some("union".to_string());
    }
    if let Some(codec) = subrecord.codec.as_ref() {
        if codec.starts_with("array_struct:") {
            return Some("row_array".to_string());
        }
        if codec.starts_with("struct:") {
            return Some("mapping".to_string());
        }
    }
    if subrecord.id == "VMAD" {
        return Some("vmad".to_string());
    }
    None
}

fn struct_tokens(codec: &str) -> Vec<&str> {
    let payload = codec
        .strip_prefix("struct:")
        .or_else(|| codec.strip_prefix("array_struct:"))
        .unwrap_or_default();
    if payload.is_empty() {
        return Vec::new();
    }
    payload
        .split(',')
        .map(|token| token.trim())
        .filter(|token| !token.is_empty())
        .collect()
}

fn token_width(token: &str) -> Option<usize> {
    match token {
        "b" | "B" => Some(1),
        "h" | "H" => Some(2),
        "i" | "I" | "f" => Some(4),
        "q" | "Q" => Some(8),
        "x" => Some(1),
        _ if token.starts_with('s') => token[1..].parse::<usize>().ok(),
        _ => None,
    }
}

fn scalar_codec_supported(codec: &str) -> bool {
    matches!(
        codec,
        "bytes"
            | "zstring"
            | "lenstring8"
            | "lenstring16"
            | "lenstring32"
            | "lstring"
            | "int8"
            | "uint8"
            | "int16"
            | "uint16"
            | "uint16le"
            | "int32"
            | "uint32"
            | "int64"
            | "uint64"
            | "float32"
            | "formid"
            | "formid_array"
    ) || codec.starts_with("fixed_string:")
}

fn schema_field_key(field: &SchemaFieldJson) -> String {
    authoring_key_name(field.display_label.as_deref(), field.id.as_str())
}

fn schema_field_legacy_key(field: &SchemaFieldJson) -> Option<&str> {
    field.display_label.as_deref()
}

fn schema_subrecord_key(record_signature: &str, spec: &SchemaSubrecordJson) -> String {
    let _ = record_signature;
    if let Some(label) = spec.display_label.as_deref() {
        return authoring_key_name(Some(label), spec.id.as_str());
    }
    if spec.fields.len() == 1 {
        if let Some(label) = spec.fields[0].display_label.as_deref() {
            return authoring_key_name(Some(label), spec.id.as_str());
        }
    }
    spec.id.clone()
}

fn schema_subrecord_legacy_key(spec: &SchemaSubrecordJson) -> Option<&str> {
    spec.display_label.as_deref().or_else(|| {
        (spec.fields.len() == 1)
            .then(|| spec.fields[0].display_label.as_deref())
            .flatten()
    })
}

fn schema_subrecord_authoring_key(
    record_spec: &SchemaRecordJson,
    spec: &SchemaSubrecordJson,
) -> String {
    let preferred = schema_subrecord_key(record_spec.id.as_str(), spec);
    if preferred == spec.id {
        return preferred;
    }
    let duplicate_count = record_spec
        .subrecords
        .iter()
        .filter(|candidate| schema_subrecord_key(record_spec.id.as_str(), candidate) == preferred)
        .take(2)
        .count();
    if duplicate_count > 1 {
        spec.id.clone()
    } else {
        preferred
    }
}

/// If `raw_key` matches exactly one subrecord spec under `record_spec` by
/// authoring label (and that spec has the resolved signature), return it.
/// Lets the caller bypass positional occurrence lookup when the label alone
/// uniquely identifies the spec — e.g. TERM has two SNAM specs but only the
/// second carries the "MarkerParameters" label.
fn find_unique_label_spec<'a>(
    record_spec: &'a SchemaRecordJson,
    raw_key: &str,
    resolved_signature: &str,
) -> Option<&'a SchemaSubrecordJson> {
    let mut matched: Option<&'a SchemaSubrecordJson> = None;
    for spec in &record_spec.subrecords {
        if spec.id != resolved_signature {
            continue;
        }
        let key_match = schema_subrecord_key(record_spec.id.as_str(), spec) == raw_key
            || schema_subrecord_legacy_key(spec).is_some_and(|label| label == raw_key);
        if !key_match {
            continue;
        }
        if matched.is_some() {
            return None;
        }
        matched = Some(spec);
    }
    matched
}

fn resolve_compact_signature_from_schema(
    record_spec: Option<&SchemaRecordJson>,
    raw_key: &str,
    label_counts: &mut HashMap<String, usize>,
) -> PyResult<String> {
    let Some(record_spec) = record_spec else {
        return Ok(raw_key.to_string());
    };
    if record_spec.subrecords.iter().any(|spec| spec.id == raw_key) {
        return Ok(raw_key.to_string());
    }
    let label_matches: Vec<(&SchemaSubrecordJson, bool)> = record_spec
        .subrecords
        .iter()
        .filter(|spec| {
            schema_subrecord_key(record_spec.id.as_str(), spec) == raw_key
                || schema_subrecord_legacy_key(spec).is_some_and(|label| label == raw_key)
        })
        .map(|spec| (spec, spec.repeatable))
        .collect();
    if label_matches.len() == 1 {
        return Ok(label_matches[0].0.id.clone());
    }
    if label_matches.len() > 1 {
        let label_occurrence = *label_counts.get(raw_key).unwrap_or(&0);
        let non_repeatable: Vec<_> = label_matches
            .iter()
            .filter(|(_, repeatable)| !*repeatable)
            .map(|(spec, _)| spec.id.clone())
            .collect();
        let repeatable: Vec<_> = label_matches
            .iter()
            .filter(|(_, repeatable)| *repeatable)
            .map(|(spec, _)| spec.id.clone())
            .collect();
        let resolved = if label_occurrence < non_repeatable.len() {
            Some(non_repeatable[label_occurrence].clone())
        } else if !repeatable.is_empty() {
            Some(repeatable[(label_occurrence - non_repeatable.len()) % repeatable.len()].clone())
        } else {
            None
        };
        label_counts.insert(raw_key.to_string(), label_occurrence + 1);
        return resolved
            .ok_or_else(|| value_error(format!("compact field key {raw_key:?} is ambiguous")));
    }
    Ok(raw_key.to_string())
}

fn codec_from_token(token: &str) -> Option<String> {
    match token {
        "b" => Some("int8".to_string()),
        "B" => Some("uint8".to_string()),
        "h" => Some("int16".to_string()),
        "H" => Some("uint16".to_string()),
        "i" => Some("int32".to_string()),
        "I" => Some("uint32".to_string()),
        "q" => Some("int64".to_string()),
        "Q" => Some("uint64".to_string()),
        "f" => Some("float32".to_string()),
        _ if token.starts_with('s') => Some(format!("fixed_string:{}", &token[1..])),
        _ => None,
    }
}

fn json_parse_int_authoring(value: &JsonValue, field: &str, default: i64) -> PyResult<i64> {
    if value.is_null() {
        return Ok(default);
    }
    if let Some(value) = value.as_bool() {
        return Ok(if value { 1 } else { 0 });
    }
    if let Some(value) = value.as_i64() {
        return Ok(value);
    }
    if let Some(value) = value.as_u64() {
        return Ok(value as i64);
    }
    if let Some(value) = value.as_str() {
        let text = value.trim();
        if text.is_empty() {
            return Ok(default);
        }
        if text.to_ascii_lowercase().starts_with("0x") {
            return i64::from_str_radix(text.trim_start_matches("0x").trim_start_matches("0X"), 16)
                .map_err(|_| value_error(format!("invalid integer for {field}: {value:?}")));
        }
        if text
            .chars()
            .any(|char| matches!(char, 'A'..='F' | 'a'..='f'))
        {
            return i64::from_str_radix(text, 16)
                .map_err(|_| value_error(format!("invalid integer for {field}: {value:?}")));
        }
        return text
            .parse::<i64>()
            .map_err(|_| value_error(format!("invalid integer for {field}: {value:?}")));
    }
    Err(value_error(format!("invalid integer for {field}")))
}

fn json_parse_hex_authoring(value: &JsonValue, field: &str, default: u32) -> PyResult<u32> {
    if value.is_null() {
        return Ok(default);
    }
    if let Some(value) = value.as_bool() {
        return Ok(if value { 1 } else { 0 });
    }
    if let Some(value) = value.as_u64() {
        return Ok(value as u32);
    }
    if let Some(value) = value.as_i64() {
        return Ok(value as u32);
    }
    if let Some(value) = value.as_str() {
        let text = value.trim();
        if text.is_empty() {
            return Ok(default);
        }
        let trimmed = text.trim_start_matches("0x").trim_start_matches("0X");
        return u32::from_str_radix(trimmed, 16)
            .map_err(|_| value_error(format!("invalid hex integer for {field}: {value:?}")));
    }
    Err(value_error(format!("invalid hex integer for {field}")))
}

fn json_parse_float_authoring(value: &JsonValue, field: &str, default: f32) -> PyResult<f32> {
    if value.is_null() {
        return Ok(default);
    }
    if let Some(value) = value.as_f64() {
        return Ok(value as f32);
    }
    if let Some(value) = value.as_i64() {
        return Ok(value as f32);
    }
    if let Some(value) = value.as_str() {
        let text = value.trim();
        if text.is_empty() {
            return Ok(default);
        }
        return text
            .parse::<f32>()
            .map_err(|_| value_error(format!("invalid float for {field}: {value:?}")));
    }
    Err(value_error(format!("invalid float for {field}")))
}

fn json_compact_authoring_entry(
    field: &JsonMap<String, JsonValue>,
) -> PyResult<Option<(String, JsonValue)>> {
    if field.contains_key("signature") {
        return Ok(None);
    }
    for marker in AUTHORING_PAYLOAD_MARKERS {
        if field.contains_key(marker) {
            return Ok(None);
        }
    }
    if field.len() != 1 {
        return Ok(None);
    }
    let Some((key, value)) = field.iter().next() else {
        return Ok(None);
    };
    Ok(Some((key.clone(), value.clone())))
}

fn default_field_input_from_schema_json(field: &SchemaFieldJson) -> JsonValue {
    if field.array.is_some() {
        return JsonValue::Array(Vec::new());
    }
    if let Some(default) = &field.default_value {
        if field.union_variants.is_empty() && field.fields.is_empty() {
            return default.clone();
        }
    }
    if let Some(variant) = field.union_variants.first() {
        let mut payload = JsonMap::new();
        payload.insert("variant".to_string(), JsonValue::String(variant.id.clone()));
        if variant.fields.len() == 1
            && variant.fields[0].fields.is_empty()
            && variant.fields[0].array.is_none()
        {
            payload.insert(
                "value".to_string(),
                default_field_input_from_schema_json(&variant.fields[0]),
            );
        } else if !variant.fields.is_empty() {
            let mut mapping = JsonMap::new();
            for nested in &variant.fields {
                mapping.insert(
                    schema_field_key(nested).to_string(),
                    default_field_input_from_schema_json(nested),
                );
            }
            payload.insert("value".to_string(), JsonValue::Object(mapping));
        } else {
            payload.insert("value".to_string(), JsonValue::Number(0.into()));
        }
        return JsonValue::Object(payload);
    }
    if !field.fields.is_empty() {
        let mut mapping = JsonMap::new();
        for nested in &field.fields {
            mapping.insert(
                schema_field_key(nested).to_string(),
                default_field_input_from_schema_json(nested),
            );
        }
        return JsonValue::Object(mapping);
    }
    match field.kind.as_str() {
        "float32" => serde_json::Number::from_f64(0.0)
            .map(JsonValue::Number)
            .unwrap_or(JsonValue::Null),
        "bytes" => JsonValue::String(String::new()),
        "formid" => JsonValue::Number(0.into()),
        "string" | "zstring" | "lstring" | "cstring" | "fixed_string" => {
            JsonValue::String(String::new())
        }
        _ => JsonValue::Number(0.into()),
    }
}

fn schema_mapping_value_json<'a>(
    mapping: &'a JsonMap<String, JsonValue>,
    field: &SchemaFieldJson,
) -> Option<&'a JsonValue> {
    let key = schema_field_key(field);
    mapping
        .get(key.as_str())
        .or_else(|| schema_field_legacy_key(field).and_then(|label| mapping.get(label)))
        .or_else(|| mapping.get(field.id.as_str()))
}

fn schema_condition_matches_record_form_version(
    condition: &SchemaConditionJson,
    record_form_version: Option<u16>,
) -> bool {
    if condition.field != "record_form_version" {
        return false;
    }
    let Some(actual) = record_form_version.map(i128::from) else {
        return false;
    };
    let Some(expected) = condition.value.as_ref().and_then(json_value_to_i128_json) else {
        return false;
    };
    match condition.operator.as_str() {
        "" | "eq" => actual == expected,
        "ne" => actual != expected,
        "lt" => actual < expected,
        "lte" => actual <= expected,
        "gt" => actual > expected,
        "gte" => actual >= expected,
        _ => false,
    }
}

fn json_value_to_i128_json(value: &JsonValue) -> Option<i128> {
    value
        .as_i64()
        .map(i128::from)
        .or_else(|| value.as_u64().map(i128::from))
}

fn field_is_present_for_record_form_version(
    field: &SchemaFieldJson,
    record_form_version: Option<u16>,
) -> bool {
    field
        .presence_conditions
        .iter()
        .filter(|condition| condition.field == "record_form_version")
        .all(|condition| {
            schema_condition_matches_record_form_version(condition, record_form_version)
        })
}

fn fill_missing_mapping_fields_from_schema_json(
    mapping: &JsonMap<String, JsonValue>,
    fields: &[SchemaFieldJson],
    record_form_version: Option<u16>,
) -> JsonMap<String, JsonValue> {
    let mut normalized = mapping.clone();
    for (index, field) in fields.iter().enumerate() {
        if !field_is_present_for_record_form_version(field, record_form_version) {
            continue;
        }
        let has_runtime_presence_condition = field
            .presence_conditions
            .iter()
            .any(|condition| condition.field != "record_form_version");
        let later_field_is_explicit = fields[index + 1..]
            .iter()
            .any(|later| schema_mapping_value_json(mapping, later).is_some());
        if has_runtime_presence_condition && !later_field_is_explicit {
            continue;
        }
        if schema_mapping_value_json(&normalized, field).is_none() {
            normalized.insert(
                schema_field_key(field).to_string(),
                default_field_input_from_schema_json(field),
            );
        }
    }
    normalized
}

fn fill_missing_structured_fields_from_schema_json(
    raw_value: &JsonValue,
    spec: &SchemaSubrecordJson,
    record_form_version: Option<u16>,
) -> JsonValue {
    let codec = spec.codec.as_deref().unwrap_or_default();
    if codec.starts_with("array_struct:") {
        let Some(rows) = raw_value.as_array() else {
            return raw_value.clone();
        };
        let filled = rows
            .iter()
            .map(|row| {
                let Some(mapping) = row.as_object() else {
                    return row.clone();
                };
                JsonValue::Object(fill_missing_mapping_fields_from_schema_json(
                    mapping,
                    &spec.fields,
                    record_form_version,
                ))
            })
            .collect();
        return JsonValue::Array(filled);
    }
    if codec.starts_with("struct:") {
        let Some(mapping) = raw_value.as_object() else {
            return raw_value.clone();
        };
        return JsonValue::Object(fill_missing_mapping_fields_from_schema_json(
            mapping,
            &spec.fields,
            record_form_version,
        ));
    }
    raw_value.clone()
}

fn fill_missing_union_variant_fields_from_schema_json(
    raw_value: &JsonValue,
    spec: &SchemaSubrecordJson,
    record_form_version: Option<u16>,
) -> JsonValue {
    let Some(mapping) = raw_value.as_object() else {
        return raw_value.clone();
    };
    let Some(variant_name) = mapping.get("variant").and_then(|value| value.as_str()) else {
        return raw_value.clone();
    };
    let Some(value) = mapping.get("value") else {
        return raw_value.clone();
    };
    let Some(variant) = spec
        .union_variants
        .iter()
        .find(|candidate| candidate.id == variant_name)
    else {
        return raw_value.clone();
    };
    let Some(codec) = variant.codec.as_deref() else {
        return raw_value.clone();
    };
    let filled_value = if codec.starts_with("array_struct:") {
        let Some(rows) = value.as_array() else {
            return raw_value.clone();
        };
        JsonValue::Array(
            rows.iter()
                .map(|row| {
                    let Some(row_mapping) = row.as_object() else {
                        return row.clone();
                    };
                    JsonValue::Object(fill_missing_mapping_fields_from_schema_json(
                        row_mapping,
                        &variant.fields,
                        record_form_version,
                    ))
                })
                .collect(),
        )
    } else if codec.starts_with("struct:")
        && !schema_fields_need_variable_struct(codec, &variant.fields)
    {
        let Some(value_mapping) = value.as_object() else {
            return raw_value.clone();
        };
        JsonValue::Object(fill_missing_mapping_fields_from_schema_json(
            value_mapping,
            &variant.fields,
            record_form_version,
        ))
    } else {
        return raw_value.clone();
    };
    let mut normalized = mapping.clone();
    normalized.insert("value".to_string(), filled_value);
    JsonValue::Object(normalized)
}

fn expand_compact_field_payload_from_schema_json(
    signature: &str,
    compact_payload: &JsonValue,
    spec: Option<&SchemaSubrecordJson>,
    record_form_version: Option<u16>,
) -> PyResult<JsonMap<String, JsonValue>> {
    let mut expanded = JsonMap::new();
    expanded.insert(
        "signature".to_string(),
        JsonValue::String(signature.to_string()),
    );
    if compact_payload.is_null() {
        if spec
            .map(|spec| native_preservation_mode(spec.kind.as_str()) != "raw_only")
            .unwrap_or(false)
        {
            expanded.insert("value".to_string(), JsonValue::Null);
        }
        return Ok(expanded);
    }
    let preservation_mode = spec
        .map(|spec| native_preservation_mode(spec.kind.as_str()))
        .unwrap_or("raw_only");
    if let Some(raw_hex) = compact_payload.as_str() {
        if json_string_is_even_hex(raw_hex) && compact_bare_hex_string_should_be_raw(spec) {
            expanded.insert(
                "preservation_mode".to_string(),
                JsonValue::String("raw_only".to_string()),
            );
            expanded.insert(
                "raw_hex".to_string(),
                JsonValue::String(raw_hex.to_string()),
            );
            return Ok(expanded);
        }
    }
    if preservation_mode == "raw_only" {
        if let Some(raw_hex) = compact_payload.as_str() {
            if json_string_is_even_hex(raw_hex) {
                expanded.insert(
                    "preservation_mode".to_string(),
                    JsonValue::String("raw_only".to_string()),
                );
                expanded.insert(
                    "raw_hex".to_string(),
                    JsonValue::String(raw_hex.to_string()),
                );
            } else {
                expanded.insert("value".to_string(), compact_payload.clone());
            }
            return Ok(expanded);
        }
    }
    let is_structured_spec = spec
        .and_then(|spec| spec.codec.as_ref())
        .map(|codec| {
            codec.starts_with("struct:")
                || codec.starts_with("array_struct:")
                || codec == "omod_data"
                || codec == "model_info"
        })
        .unwrap_or(false);

    if let Some(mapping) = compact_payload.as_object() {
        if let Some(value) = mapping.get("semantic_type") {
            expanded.insert("semantic_type".to_string(), value.clone());
        }
        let is_vmad_layout = spec
            .and_then(self::authoring::authoring_serialize::runtime_layout_for_subrecord_schema)
            == Some("vmad")
            || signature == "VMAD";
        if is_vmad_layout && mapping.is_empty() {
            expanded.insert(
                "preservation_mode".to_string(),
                JsonValue::String("raw_only".to_string()),
            );
            expanded.insert("raw_hex".to_string(), JsonValue::String(String::new()));
            return Ok(expanded);
        }
        if is_vmad_layout {
            if let Some(raw_hex) = mapping.get("raw_hex") {
                expanded.insert(
                    "preservation_mode".to_string(),
                    JsonValue::String("hybrid".to_string()),
                );
                expanded.insert("raw_hex".to_string(), raw_hex.clone());
                expanded.insert("value".to_string(), JsonValue::Object(mapping.clone()));
                return Ok(expanded);
            }
            // Parsed VMAD payload without raw_hex: the build path reconstructs
            // the bytes from Scripts/Version/Object Format via
            // build_vmad_bytes_from_payload. Wrap the mapping under "value" so
            // it isn't redirected into the structured "fields" branch below.
            expanded.insert(
                "preservation_mode".to_string(),
                JsonValue::String("hybrid".to_string()),
            );
            expanded.insert("value".to_string(), JsonValue::Object(mapping.clone()));
            return Ok(expanded);
        }
        if spec.map(|spec| spec.localized).unwrap_or(false)
            && (mapping.contains_key("TargetLanguage")
                || mapping.contains_key("Values")
                || mapping.contains_key("Value"))
        {
            let mut localized_payload = JsonMap::new();
            for (key, value) in mapping {
                if key == "raw_hex" || key == "semantic_type" {
                    continue;
                }
                localized_payload.insert(key.clone(), value.clone());
            }
            expanded.insert("value".to_string(), JsonValue::Object(localized_payload));
            if let Some(raw_hex) = mapping.get("raw_hex") {
                expanded.insert(
                    "preservation_mode".to_string(),
                    JsonValue::String("hybrid".to_string()),
                );
                expanded.insert("raw_hex".to_string(), raw_hex.clone());
            }
            return Ok(expanded);
        }
        if mapping.contains_key("variant") {
            if let Some(value) = mapping.get("variant") {
                expanded.insert("variant".to_string(), value.clone());
            }
            if let Some(raw_hex) = mapping.get("raw_hex") {
                expanded.insert(
                    "preservation_mode".to_string(),
                    JsonValue::String("hybrid".to_string()),
                );
                expanded.insert("raw_hex".to_string(), raw_hex.clone());
            }
            if let Some(value) = mapping.get("value") {
                let filled = spec
                    .map(|spec| {
                        fill_missing_union_variant_fields_from_schema_json(
                            compact_payload,
                            spec,
                            record_form_version,
                        )
                    })
                    .and_then(|payload| {
                        payload
                            .as_object()
                            .and_then(|mapping| mapping.get("value").cloned())
                    })
                    .unwrap_or_else(|| value.clone());
                expanded.insert("value".to_string(), filled);
            }
            return Ok(expanded);
        }
        let only_raw_payload = mapping
            .keys()
            .all(|key| matches!(key.as_str(), "raw_hex" | "semantic_type" | "display_value"));
        // An all-default structured payload compacts to `{}`; without this
        // fall-through it exits here with no `fields` and writes a zero-length
        // subrecord. FO4 `MODT` (codec `model_info`) still owes a 20-byte
        // header in that state, and a zero-length one faults TESRace::Load.
        let fall_through_for_structured_defaults =
            mapping.is_empty() && is_structured_spec && !mapping.contains_key("raw_hex");
        if only_raw_payload && !fall_through_for_structured_defaults {
            if let Some(raw_hex) = mapping.get("raw_hex") {
                expanded.insert(
                    "preservation_mode".to_string(),
                    JsonValue::String("raw_only".to_string()),
                );
                expanded.insert("raw_hex".to_string(), raw_hex.clone());
            }
            if let Some(display_value) = mapping.get("display_value") {
                expanded.insert("display_value".to_string(), display_value.clone());
            }
            return Ok(expanded);
        }
        if let Some(value) = mapping.get("value") {
            expanded.insert("value".to_string(), value.clone());
            if let Some(raw_hex) = mapping.get("raw_hex") {
                expanded.insert(
                    "preservation_mode".to_string(),
                    JsonValue::String("hybrid".to_string()),
                );
                expanded.insert("raw_hex".to_string(), raw_hex.clone());
            }
            return Ok(expanded);
        }
        if is_structured_spec {
            let filled = fill_missing_structured_fields_from_schema_json(
                compact_payload,
                spec.unwrap(),
                record_form_version,
            );
            let codec = spec
                .and_then(|spec| spec.codec.as_ref())
                .cloned()
                .unwrap_or_default();
            if codec.starts_with("array_struct:") {
                expanded.insert("rows".to_string(), filled);
            } else {
                expanded.insert("fields".to_string(), filled);
            }
            return Ok(expanded);
        }
        if let Some(raw_hex) = mapping.get("raw_hex") {
            expanded.insert(
                "preservation_mode".to_string(),
                JsonValue::String("raw_only".to_string()),
            );
            expanded.insert("raw_hex".to_string(), raw_hex.clone());
            if let Some(display_value) = mapping.get("display_value") {
                expanded.insert("display_value".to_string(), display_value.clone());
            }
            return Ok(expanded);
        }
        expanded.insert("value".to_string(), JsonValue::Object(mapping.clone()));
        return Ok(expanded);
    }

    if is_structured_spec {
        let filled = fill_missing_structured_fields_from_schema_json(
            compact_payload,
            spec.unwrap(),
            record_form_version,
        );
        let codec = spec
            .and_then(|spec| spec.codec.as_ref())
            .cloned()
            .unwrap_or_default();
        if codec.starts_with("array_struct:") {
            expanded.insert("rows".to_string(), filled);
        } else {
            expanded.insert("fields".to_string(), filled);
        }
        return Ok(expanded);
    }
    expanded.insert("value".to_string(), compact_payload.clone());
    Ok(expanded)
}

fn parse_localized_authoring_values_json(
    value: &JsonMap<String, JsonValue>,
) -> PyResult<(String, HashMap<String, String>)> {
    let target_language = value
        .get("TargetLanguage")
        .and_then(|entry| entry.as_str())
        .filter(|entry| !entry.trim().is_empty())
        .unwrap_or("English")
        .to_string();
    let mut values = HashMap::new();
    if let Some(values_payload) = value.get("Values") {
        for (index, row) in json_array(values_payload, "Localized Values")?
            .iter()
            .enumerate()
        {
            let mapping = json_object(row, &format!("Localized Values[{index}]"))?;
            let language = mapping
                .get("Language")
                .and_then(|entry| entry.as_str())
                .filter(|entry| !entry.trim().is_empty())
                .unwrap_or(target_language.as_str())
                .to_string();
            let string_value = mapping
                .get("String")
                .and_then(|entry| entry.as_str())
                .ok_or_else(|| {
                    value_error(format!("Localized Values[{index}] must include String"))
                })?
                .to_string();
            values.insert(language, string_value);
        }
    } else if let Some(single_value) = value.get("Value") {
        values.insert(
            target_language.clone(),
            single_value
                .as_str()
                .ok_or_else(|| value_error("Localized Value must be a string"))?
                .to_string(),
        );
    }
    Ok((target_language, values))
}

fn compact_value_raw_hex_candidate(payload: &JsonMap<String, JsonValue>) -> Option<&JsonValue> {
    let value = payload.get("value")?;
    let text = value.as_str()?.trim();
    if !json_string_is_even_hex(text) {
        return None;
    }
    Some(value)
}

fn compact_value_text_candidate(payload: &JsonMap<String, JsonValue>) -> Option<&str> {
    let value = payload.get("value")?;
    let text = value.as_str()?;
    if json_string_is_even_hex(text.trim()) {
        return None;
    }
    Some(text)
}

fn json_string_is_even_hex(text: &str) -> bool {
    !text.is_empty() && text.len() % 2 == 0 && text.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn compact_bare_hex_string_should_be_raw(spec: Option<&SchemaSubrecordJson>) -> bool {
    let Some(spec) = spec else {
        return true;
    };
    if native_preservation_mode(spec.kind.as_str()) == "raw_only" || spec.enum_ref.is_some() {
        return true;
    }
    matches!(
        spec.codec.as_deref(),
        Some(
            "int8"
                | "uint8"
                | "int16"
                | "uint16"
                | "uint16le"
                | "int32"
                | "uint32"
                | "int64"
                | "uint64"
                | "float32"
                | "formid"
                | "formid_array"
        )
    )
}

fn encode_form_reference_json(
    context: &mut NativeImportContext,
    value: &JsonValue,
    field: &str,
) -> PyResult<u32> {
    if value.is_null() {
        return Ok(0);
    }
    if let Some(parsed) = value.as_u64() {
        return Ok(parsed as u32);
    }
    if let Some(parsed) = value.as_i64() {
        return Ok(parsed as u32);
    }
    let mapping = json_object(value, field).map_err(|_| {
        value_error(format!(
            "{field} form reference must be an object or integer"
        ))
    })?;
    if let Some(raw) = mapping.get("raw") {
        return json_parse_hex_authoring(raw, field, 0);
    }
    let Some(reference) = mapping.get("reference") else {
        return Ok(0);
    };
    let reference = json_object(reference, &format!("{field}.reference"))?;
    let object_id = json_parse_hex_authoring(
        reference
            .get("object_id")
            .ok_or_else(|| value_error(format!("{field}.reference.object_id is required")))?,
        &format!("{field}.reference.object_id"),
        0,
    )? & 0x00FF_FFFF;
    if let Some(raw_plugin) = reference.get("plugin") {
        let target_plugin = raw_plugin
            .as_str()
            .ok_or_else(|| value_error(format!("{field}.reference.plugin must be a string")))?;
        if target_plugin.eq_ignore_ascii_case(context.plugin_name.as_str()) {
            return Ok((context.own_index() << 24) | object_id);
        }
        let master_index = context.ensure_master_index(target_plugin) as u32;
        return Ok((master_index << 24) | object_id);
    }
    if let Some(missing_index) = reference.get("missing_index") {
        return Ok(
            ((json_parse_int_authoring(missing_index, field, 0)? as u32 & 0xFF) << 24) | object_id,
        );
    }
    Ok((LOCAL_FORM_INDEX as u32) << 24 | object_id)
}

fn enum_numeric_value_json(
    value: &JsonValue,
    enum_def: Option<&SchemaEnumJson>,
    field: &str,
) -> PyResult<i128> {
    if let Some(parsed) = value.as_bool() {
        return Ok(if parsed { 1 } else { 0 });
    }
    if let Some(parsed) = value.as_i64() {
        return Ok(parsed as i128);
    }
    if let Some(parsed) = value.as_u64() {
        return Ok(parsed as i128);
    }
    if let Some(text) = value.as_str() {
        let trimmed = text.trim();
        if let Some(enum_def) = enum_def {
            if let Some(parsed) = enum_def.value_for_token_or_label(trimmed) {
                return Ok(parsed);
            }
        }
        if trimmed.to_ascii_lowercase().starts_with("0x") {
            return i128::from_str_radix(
                trimmed.trim_start_matches("0x").trim_start_matches("0X"),
                16,
            )
            .map_err(|_| value_error(format!("invalid enum value for {field}: {text:?}")));
        }
        if trimmed
            .chars()
            .any(|char| matches!(char, 'A'..='F' | 'a'..='f'))
        {
            return i128::from_str_radix(trimmed, 16)
                .map_err(|_| value_error(format!("invalid enum value for {field}: {text:?}")));
        }
        return trimmed
            .parse::<i128>()
            .map_err(|_| value_error(format!("invalid enum value for {field}: {text:?}")));
    }
    if let Some(items) = value.as_array() {
        let enum_def = enum_def.ok_or_else(|| {
            value_error(format!("{field} enum list requires schema enum metadata"))
        })?;
        if !enum_def.is_flags() {
            return Err(value_error(format!(
                "{field} enum list is only valid for flags"
            )));
        }
        let mut parsed = 0i128;
        for (index, item) in items.iter().enumerate() {
            parsed |= enum_numeric_value_json(item, Some(enum_def), &format!("{field}[{index}]"))?;
        }
        return Ok(parsed);
    }
    let mapping = json_object(value, field)
        .map_err(|_| value_error(format!("{field} enum value must be an object or integer")))?;
    if let Some(raw_value) = mapping.get("value") {
        return enum_numeric_value_json(raw_value, enum_def, &format!("{field}.value"));
    }
    if let Some(token_value) = mapping.get("token") {
        let token = token_value
            .as_str()
            .ok_or_else(|| value_error(format!("{field}.token must be a string")))?;
        if let Some(enum_def) = enum_def {
            if let Some(parsed) = enum_def.value_for_token_or_label(token) {
                return Ok(parsed);
            }
        }
        return Err(value_error(format!(
            "{field}.token {token:?} is not defined"
        )));
    }
    if let Some(label_value) = mapping.get("label") {
        let label = label_value
            .as_str()
            .ok_or_else(|| value_error(format!("{field}.label must be a string")))?;
        if let Some(enum_def) = enum_def {
            if let Some(parsed) = enum_def.value_for_token_or_label(label) {
                return Ok(parsed);
            }
        }
        return Err(value_error(format!(
            "{field}.label {label:?} is not defined"
        )));
    }
    Err(value_error(format!(
        "{field} enum payload must include value, token, or label"
    )))
}

fn encode_integer_mapping_codec_json(
    codec: &str,
    value: &JsonValue,
    field: &str,
    enum_def: Option<&SchemaEnumJson>,
) -> PyResult<Option<Vec<u8>>> {
    if enum_def.is_some() {
        let parsed = enum_numeric_value_json(value, enum_def, field)?;
        let encoded = match codec {
            "int8" => (parsed as i8).to_le_bytes().to_vec(),
            "uint8" => vec![parsed as u8],
            "int16" => (parsed as i16).to_le_bytes().to_vec(),
            "uint16" | "uint16le" => (parsed as u16).to_le_bytes().to_vec(),
            "int32" => (parsed as i32).to_le_bytes().to_vec(),
            "uint32" => (parsed as u32).to_le_bytes().to_vec(),
            "int64" => (parsed as i64).to_le_bytes().to_vec(),
            "uint64" => (parsed as u64).to_le_bytes().to_vec(),
            _ => return Ok(None),
        };
        return Ok(Some(encoded));
    }
    let Some(mapping) = value.as_object() else {
        return Ok(None);
    };
    if !mapping.contains_key("value")
        && !mapping.contains_key("token")
        && !mapping.contains_key("label")
    {
        return Ok(None);
    }
    let parsed = enum_numeric_value_json(value, enum_def, field)?;
    let encoded = match codec {
        "int8" => (parsed as i8).to_le_bytes().to_vec(),
        "uint8" => vec![parsed as u8],
        "int16" => (parsed as i16).to_le_bytes().to_vec(),
        "uint16" | "uint16le" => (parsed as u16).to_le_bytes().to_vec(),
        "int32" => (parsed as i32).to_le_bytes().to_vec(),
        "uint32" => (parsed as u32).to_le_bytes().to_vec(),
        "int64" => (parsed as i64).to_le_bytes().to_vec(),
        "uint64" => (parsed as u64).to_le_bytes().to_vec(),
        _ => return Ok(None),
    };
    Ok(Some(encoded))
}

fn encode_scalar_codec_json(
    codec: &str,
    value: &JsonValue,
    field: &str,
    enum_def: Option<&SchemaEnumJson>,
    context: &mut NativeImportContext,
    target: Option<&str>,
) -> PyResult<Vec<u8>> {
    match codec {
        "empty" => Ok(Vec::new()),
        "bytes" => {
            if let Some(text) = value.as_str() {
                let trimmed = text.trim();
                if trimmed.is_empty() {
                    return Ok(Vec::new());
                }
                return hex::decode(trimmed).map_err(|_| {
                    value_error(format!("{field} bytes value must be raw bytes or hex"))
                });
            }
            if let Some(items) = value.as_array() {
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    out.push(json_parse_int_authoring(item, field, 0)? as u8);
                }
                return Ok(out);
            }
            Err(value_error(format!(
                "{field} bytes value must be raw bytes"
            )))
        }
        "zstring" => Ok(encode_cp1252(
            value
                .as_str()
                .ok_or_else(|| value_error(format!("{field} string value must be a string")))?,
            true,
        )),
        "lenstring8" | "lenstring16" | "lenstring32" => {
            let text = value
                .as_str()
                .ok_or_else(|| value_error(format!("{field} string value must be a string")))?;
            let encoded = encode_cp1252(text, false);
            let mut out = Vec::new();
            match codec {
                "lenstring8" => out.push((encoded.len() & 0xFF) as u8),
                "lenstring16" => {
                    out.extend_from_slice(&((encoded.len() & 0xFFFF) as u16).to_le_bytes())
                }
                _ => out.extend_from_slice(&(encoded.len() as u32).to_le_bytes()),
            }
            out.extend_from_slice(&encoded);
            Ok(out)
        }
        "lstring" => {
            if let Some(parsed) = value.as_u64() {
                return Ok((parsed as u32).to_le_bytes().to_vec());
            }
            Ok(encode_cp1252(
                value.as_str().ok_or_else(|| {
                    value_error(format!("{field} lstring value must be a string"))
                })?,
                true,
            ))
        }
        "int8" => match encode_integer_mapping_codec_json(codec, value, field, enum_def)? {
            Some(encoded) => Ok(encoded),
            None => Ok((json_parse_int_authoring(value, field, 0)? as i8)
                .to_le_bytes()
                .to_vec()),
        },
        "uint8" => match encode_integer_mapping_codec_json(codec, value, field, enum_def)? {
            Some(encoded) => Ok(encoded),
            None => Ok(vec![json_parse_int_authoring(value, field, 0)? as u8]),
        },
        "int16" => match encode_integer_mapping_codec_json(codec, value, field, enum_def)? {
            Some(encoded) => Ok(encoded),
            None => Ok((json_parse_int_authoring(value, field, 0)? as i16)
                .to_le_bytes()
                .to_vec()),
        },
        "uint16" | "uint16le" => {
            match encode_integer_mapping_codec_json(codec, value, field, enum_def)? {
                Some(encoded) => Ok(encoded),
                None => Ok((json_parse_int_authoring(value, field, 0)? as u16)
                    .to_le_bytes()
                    .to_vec()),
            }
        }
        "int32" => match encode_integer_mapping_codec_json(codec, value, field, enum_def)? {
            Some(encoded) => Ok(encoded),
            None => Ok((json_parse_int_authoring(value, field, 0)? as i32)
                .to_le_bytes()
                .to_vec()),
        },
        "uint32" => match encode_integer_mapping_codec_json(codec, value, field, enum_def)? {
            Some(encoded) => Ok(encoded),
            None => Ok((json_parse_int_authoring(value, field, 0)? as u32)
                .to_le_bytes()
                .to_vec()),
        },
        "int64" => match encode_integer_mapping_codec_json(codec, value, field, enum_def)? {
            Some(encoded) => Ok(encoded),
            None => Ok((json_parse_int_authoring(value, field, 0)? as i64)
                .to_le_bytes()
                .to_vec()),
        },
        "uint64" => match encode_integer_mapping_codec_json(codec, value, field, enum_def)? {
            Some(encoded) => Ok(encoded),
            None => Ok((json_parse_int_authoring(value, field, 0)? as u64)
                .to_le_bytes()
                .to_vec()),
        },
        "float32" => Ok(json_parse_float_authoring(value, field, 0.0)?
            .to_le_bytes()
            .to_vec()),
        "formid" => Ok(encode_form_reference_json(context, value, field)?
            .to_le_bytes()
            .to_vec()),
        "formid_array" => {
            let rows = json_array(value, field).map_err(|_| {
                value_error(format!("{field} formid_array value must be a sequence"))
            })?;
            let mut out = Vec::new();
            for (index, item) in rows.iter().enumerate() {
                out.extend_from_slice(
                    &encode_form_reference_json(context, item, &format!("{field}[{index}]"))?
                        .to_le_bytes(),
                );
            }
            Ok(out)
        }
        _ if codec.starts_with("fixed_string:") => {
            let size = codec["fixed_string:".len()..]
                .parse::<usize>()
                .map_err(|_| value_error(format!("{field} fixed string codec is invalid")))?;
            let raw = encode_cp1252(
                value.as_str().ok_or_else(|| {
                    value_error(format!("{field} fixed string value must be a string"))
                })?,
                false,
            );
            Ok(raw[..raw.len().min(size)]
                .iter()
                .copied()
                .chain(std::iter::repeat(0).take(size.saturating_sub(raw.len())))
                .collect())
        }
        _ if enum_def.is_some() => Ok((enum_numeric_value_json(value, enum_def, field)? as u32)
            .to_le_bytes()
            .to_vec()),
        _ => {
            let _ = target;
            Err(value_error(format!(
                "unsupported native authoring codec {codec:?} for {field}"
            )))
        }
    }
}

fn encode_union_value_json(
    signature: &str,
    variants: &[SchemaUnionVariantJson],
    value: &JsonValue,
    context: &mut NativeImportContext,
    field_name: &str,
    truncate_trailing_absent: bool,
) -> PyResult<Vec<u8>> {
    let mapping = json_object(value, field_name)
        .map_err(|_| value_error(format!("{field_name} union value must be an object")))?;
    let variant_name = mapping
        .get("variant")
        .and_then(|value| value.as_str())
        .ok_or_else(|| value_error(format!("{field_name} union value must declare a variant")))?;
    let variant = variants
        .iter()
        .find(|candidate| candidate.id == variant_name)
        .ok_or_else(|| {
            value_error(format!(
                "{field_name} union variant {variant_name:?} is not defined"
            ))
        })?;
    let variant_value = mapping
        .get("fields")
        .or_else(|| mapping.get("rows"))
        .or_else(|| mapping.get("value"))
        .unwrap_or(value);
    let codec = variant.codec.as_deref().ok_or_else(|| {
        value_error(format!(
            "{field_name} union variant {variant_name:?} has no codec"
        ))
    })?;
    if codec.starts_with("struct:") {
        let row_mapping = variant_value.as_object().unwrap_or(mapping);
        if schema_fields_need_variable_struct(codec, &variant.fields) {
            return encode_variable_struct_json(
                signature,
                &variant.fields,
                row_mapping,
                context,
                field_name,
            );
        }
        return encode_structured_mapping_json(
            signature,
            codec,
            &variant.fields,
            row_mapping,
            context,
            field_name,
            truncate_trailing_absent,
        );
    }
    if codec.starts_with("array_struct:") {
        return encode_array_struct_value_json(
            signature,
            codec,
            &variant.fields,
            variant_value,
            context,
            field_name,
        );
    }
    encode_scalar_codec_json(codec, variant_value, field_name, None, context, None)
}

fn encode_structured_mapping_json(
    signature: &str,
    codec: &str,
    fields: &[SchemaFieldJson],
    mapping: &JsonMap<String, JsonValue>,
    context: &mut NativeImportContext,
    field_name: &str,
    truncate_trailing_absent: bool,
) -> PyResult<Vec<u8>> {
    let tokens = struct_tokens(codec);
    let mut token_index = 0usize;
    let mut encoded = Vec::new();
    for field in fields {
        while token_index < tokens.len() && tokens[token_index] == "x" {
            encoded
                .extend(std::iter::repeat(0).take(token_width(tokens[token_index]).unwrap_or(1)));
            token_index += 1;
        }
        let default_field_value;
        let field_value = match schema_mapping_value_json(mapping, field) {
            Some(value) => value,
            None => {
                // Field has `presence_conditions` (e.g. `wbFromVersion(152, …)`
                // for ARMO/WEAP DAMA curve_table) and is absent from the
                // mapping — the source record was an older form_version that
                // didn't carry this field. Skip emission so the re-encoded
                // row matches the original byte width. The decoder filters
                // these segments by the same conditions on read.
                if !field.presence_conditions.is_empty() {
                    break;
                }
                // Variable-length structs (xEdit `nil, MinSize` — CSME, CSLR,
                // WTHR.FNAM, …) declare more schema fields than may be
                // present in any given record. When the input mapping is
                // missing a field, all subsequent fields are also missing —
                // the source ESP simply ended early. Stop emitting so the
                // re-encoded byte length matches the original (byte-exact
                // YAML→ESP round-trip without needing a raw_hex backup).
                if truncate_trailing_absent {
                    break;
                }
                default_field_value = default_field_input_from_schema_json(field);
                &default_field_value
            }
        };
        let field_bytes = if !field.union_variants.is_empty() {
            encode_union_value_json(
                signature,
                &field.union_variants,
                field_value,
                context,
                &format!("{field_name}.{}", field.id),
                truncate_trailing_absent,
            )?
        } else {
            let token_codec = tokens
                .get(token_index)
                .and_then(|token| codec_from_token(token));
            let enum_def = field.enum_ref.as_ref().and_then(|enum_ref| {
                context
                    .game
                    .as_deref()
                    .and_then(|game| compiled_schema_for_game(game).ok())
                    .and_then(|schema| schema.enums.get(enum_ref).cloned())
            });
            let scalar_codec = match field.kind.as_str() {
                "enum" => token_codec.clone().unwrap_or_else(|| "uint32".to_string()),
                "formid" => "formid".to_string(),
                kind if scalar_codec_supported(kind) => kind.to_string(),
                _ => token_codec.unwrap_or_else(|| field.kind.clone()),
            };
            encode_scalar_codec_json(
                scalar_codec.as_str(),
                field_value,
                &format!("{field_name}.{}", field.id),
                enum_def.as_ref(),
                context,
                field.formlink_target.as_deref(),
            )?
        };
        if token_index < tokens.len() && tokens[token_index] != "x" {
            if let Some(expected) = token_width(tokens[token_index]) {
                if field_bytes.len() != expected {
                    return Err(value_error(format!(
                        "{field_name}.{} encoded size {} does not match {}",
                        field.id,
                        field_bytes.len(),
                        expected
                    )));
                }
            }
            token_index += 1;
        }
        encoded.extend_from_slice(&field_bytes);
    }
    while token_index < tokens.len() && tokens[token_index] == "x" {
        encoded.extend(std::iter::repeat(0).take(token_width(tokens[token_index]).unwrap_or(1)));
        token_index += 1;
    }
    Ok(encoded)
}

fn encode_array_struct_value_json(
    signature: &str,
    codec: &str,
    fields: &[SchemaFieldJson],
    value: &JsonValue,
    context: &mut NativeImportContext,
    field_name: &str,
) -> PyResult<Vec<u8>> {
    let rows = json_array(value, field_name)
        .map_err(|_| value_error(format!("{field_name} array_struct value must be a list")))?;
    let mut out = Vec::new();
    let row_codec = format!("struct:{}", codec.trim_start_matches("array_struct:"));
    for (index, row) in rows.iter().enumerate() {
        let mapping = json_object(row, &format!("{field_name}[{index}]")).map_err(|_| {
            value_error(format!(
                "{field_name}[{index}] array_struct row must be an object"
            ))
        })?;
        out.extend_from_slice(&encode_structured_mapping_json(
            signature,
            row_codec.as_str(),
            fields,
            mapping,
            context,
            &format!("{field_name}[{index}]"),
            false,
        )?);
    }
    Ok(out)
}

/// Encode an OMOD DATA structured mapping back into raw bytes.
///
/// Inverse of `decode_omod_data_to_field_map_json`. Layout (FO4/FO76):
///   * 20-byte header `<IIBBIBBI` (include_count, property_count, two
///     unknown bools, form_type, max_rank, level_tier_scaled_offset,
///     attach_point).
///   * u32 attach-parent slot count, then that many u32 slots.
///   * 1 item row of `<I` (4 bytes).
///   * `include_count` rows of `<IBBB` (7 bytes each).
///   * `property_count` rows of `<B3xB3xH2xIIf` (24 bytes each, explicit pad).
///
/// `include_count` and `property_count` are derived from the array lengths.
fn encode_omod_data_json(
    spec: &SchemaSubrecordJson,
    mapping: &JsonMap<String, JsonValue>,
    context: &mut NativeImportContext,
    field_name: &str,
) -> PyResult<Vec<u8>> {
    let lookup = |id: &str| -> Option<&JsonValue> {
        spec.fields
            .iter()
            .find(|f| f.id == id)
            .and_then(|field| schema_mapping_value_json(mapping, field))
    };

    let attach_parent_slots = match lookup("attach_parent_slots") {
        Some(value) if !value.is_null() => {
            json_array(value, &format!("{field_name}.AttachParentSlots"))
                .map_err(|_| value_error(format!("{field_name}.AttachParentSlots must be a list")))?
                .to_vec()
        }
        _ => Vec::new(),
    };
    let items_value = lookup("items").cloned().unwrap_or(JsonValue::Null);
    let items_array: Vec<JsonValue> = if items_value.is_null() {
        Vec::new()
    } else {
        json_array(&items_value, &format!("{field_name}.Items"))
            .map_err(|_| value_error(format!("{field_name}.Items must be a list")))?
            .to_vec()
    };
    let includes_value = lookup("includes").cloned().unwrap_or(JsonValue::Null);
    let includes_array: Vec<JsonValue> = if includes_value.is_null() {
        Vec::new()
    } else {
        json_array(&includes_value, &format!("{field_name}.Includes"))
            .map_err(|_| value_error(format!("{field_name}.Includes must be a list")))?
            .to_vec()
    };
    let properties_value = lookup("properties").cloned().unwrap_or(JsonValue::Null);
    let properties_array: Vec<JsonValue> = if properties_value.is_null() {
        Vec::new()
    } else {
        json_array(&properties_value, &format!("{field_name}.Properties"))
            .map_err(|_| value_error(format!("{field_name}.Properties must be a list")))?
            .to_vec()
    };

    let include_count = includes_array.len() as u32;
    let property_count = properties_array.len() as u32;
    let unknown_bool_1 = lookup("unknown_bool_1")
        .map(|v| json_parse_int_authoring(v, &format!("{field_name}.UnknownBool1"), 0))
        .transpose()?
        .unwrap_or(0) as u8;
    let unknown_bool_2 = lookup("unknown_bool_2")
        .map(|v| json_parse_int_authoring(v, &format!("{field_name}.UnknownBool2"), 0))
        .transpose()?
        .unwrap_or(0) as u8;
    let form_type = lookup("form_type")
        .map(|v| json_parse_int_authoring(v, &format!("{field_name}.FormType"), 0))
        .transpose()?
        .unwrap_or(0) as u32;
    let max_rank = lookup("max_rank")
        .map(|v| json_parse_int_authoring(v, &format!("{field_name}.MaxRank"), 0))
        .transpose()?
        .unwrap_or(0) as u8;
    let level_tier_scaled_offset = lookup("level_tier_scaled_offset")
        .map(|v| json_parse_int_authoring(v, &format!("{field_name}.LevelTierScaledOffset"), 0))
        .transpose()?
        .unwrap_or(0) as u8;
    let attach_point = match lookup("attach_point") {
        Some(value) if !value.is_null() => {
            encode_form_reference_json(context, value, &format!("{field_name}.AttachPoint"))?
        }
        _ => 0,
    };

    let mut out: Vec<u8> = Vec::new();
    out.extend_from_slice(&include_count.to_le_bytes());
    out.extend_from_slice(&property_count.to_le_bytes());
    out.push(unknown_bool_1);
    out.push(unknown_bool_2);
    out.extend_from_slice(&form_type.to_le_bytes());
    out.push(max_rank);
    out.push(level_tier_scaled_offset);
    out.extend_from_slice(&attach_point.to_le_bytes());
    out.extend_from_slice(&(attach_parent_slots.len() as u32).to_le_bytes());

    for (index, slot_value) in attach_parent_slots.iter().enumerate() {
        let slot_field = format!("{field_name}.AttachParentSlots[{index}]");
        let slot = if slot_value.is_null() {
            0
        } else {
            encode_form_reference_json(context, slot_value, &slot_field)?
        };
        out.extend_from_slice(&slot.to_le_bytes());
    }

    let item_row = items_array
        .first()
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    let item_value_1 = item_row
        .get("Value1")
        .or_else(|| item_row.get("value_1"))
        .map(|v| json_parse_int_authoring(v, &format!("{field_name}.Items[0].Value1"), 0))
        .transpose()?
        .unwrap_or(0) as u32;
    out.extend_from_slice(&item_value_1.to_le_bytes());

    for (index, row) in includes_array.iter().enumerate() {
        let row_name = format!("{field_name}.Includes[{index}]");
        let row_obj = row
            .as_object()
            .ok_or_else(|| value_error(format!("{row_name} must be an object")))?;
        let mod_value = row_obj
            .get("Mod")
            .or_else(|| row_obj.get("mod"))
            .unwrap_or(&JsonValue::Null);
        let mod_id = if mod_value.is_null() {
            0
        } else {
            encode_form_reference_json(context, mod_value, &format!("{row_name}.Mod"))?
        };
        let minimum_level = row_obj
            .get("MinimumLevel")
            .or_else(|| row_obj.get("minimum_level"))
            .map(|v| json_parse_int_authoring(v, &format!("{row_name}.MinimumLevel"), 0))
            .transpose()?
            .unwrap_or(0) as u8;
        let optional = row_obj
            .get("Optional")
            .or_else(|| row_obj.get("optional"))
            .map(|v| json_parse_int_authoring(v, &format!("{row_name}.Optional"), 0))
            .transpose()?
            .unwrap_or(0) as u8;
        let dont_use_all = row_obj
            .get("DontUseAll")
            .or_else(|| row_obj.get("dont_use_all"))
            .map(|v| json_parse_int_authoring(v, &format!("{row_name}.DontUseAll"), 0))
            .transpose()?
            .unwrap_or(0) as u8;
        out.extend_from_slice(&mod_id.to_le_bytes());
        out.push(minimum_level);
        out.push(optional);
        out.push(dont_use_all);
    }

    for (index, row) in properties_array.iter().enumerate() {
        let row_name = format!("{field_name}.Properties[{index}]");
        let row_obj = row
            .as_object()
            .ok_or_else(|| value_error(format!("{row_name} must be an object")))?;
        let value_type = row_obj
            .get("ValueType")
            .or_else(|| row_obj.get("value_type"))
            .map(|v| json_parse_int_authoring(v, &format!("{row_name}.ValueType"), 0))
            .transpose()?
            .unwrap_or(0) as u8;
        let function_type = row_obj
            .get("FunctionType")
            .or_else(|| row_obj.get("function_type"))
            .map(|v| json_parse_int_authoring(v, &format!("{row_name}.FunctionType"), 0))
            .transpose()?
            .unwrap_or(0) as u8;
        let property_id = row_obj
            .get("Property")
            .or_else(|| row_obj.get("property"))
            .map(|v| json_parse_int_authoring(v, &format!("{row_name}.Property"), 0))
            .transpose()?
            .unwrap_or(0) as u16;
        let value_1 = row_obj
            .get("Value1")
            .or_else(|| row_obj.get("value_1"))
            .map(|v| json_parse_int_authoring(v, &format!("{row_name}.Value1"), 0))
            .transpose()?
            .unwrap_or(0) as u32;
        let value_2 = row_obj
            .get("Value2")
            .or_else(|| row_obj.get("value_2"))
            .map(|v| json_parse_int_authoring(v, &format!("{row_name}.Value2"), 0))
            .transpose()?
            .unwrap_or(0) as u32;
        let step = row_obj
            .get("Step")
            .or_else(|| row_obj.get("step"))
            .map(|v| json_parse_float_authoring(v, &format!("{row_name}.Step"), 0.0))
            .transpose()?
            .unwrap_or(0.0) as f32;
        out.push(value_type);
        out.extend_from_slice(&[0, 0, 0]);
        out.push(function_type);
        out.extend_from_slice(&[0, 0, 0]);
        out.extend_from_slice(&property_id.to_le_bytes());
        out.extend_from_slice(&[0, 0]);
        out.extend_from_slice(&value_1.to_le_bytes());
        out.extend_from_slice(&value_2.to_le_bytes());
        out.extend_from_slice(&step.to_le_bytes());
    }

    Ok(out)
}

/// Encode a model_info structured mapping back into raw MODT bytes.
///
/// Inverse of `decode_model_info_to_field_map_json`. Layout (FO4/FO76):
///   * 20-byte header: counter_count=4, then counters[4] = [num_textures,
///     num_addon_nodes, srgb_count, num_materials]. `num_textures`,
///     `num_addon_nodes`, and `num_materials` are derived from the array
///     lengths; `srgb_count` is a bare count with no backing array and is
///     taken verbatim from the JSON.
///   * Texture[num_textures], addon_nodes[num_addon_nodes] (u32 each),
///     Material[num_materials]. Texture/Material rows are 12 bytes each.
///
/// MODT holds content hashes, never FormIDs, so unlike `encode_omod_data_json`
/// this needs no `NativeImportContext` for reference rewriting.
///
/// `pub` so `esp/tests/modt_roundtrip.rs` can pair it with
/// `compact_model_info_payload_json` against the real generated schema.
pub fn encode_model_info_json(
    spec: &SchemaSubrecordJson,
    mapping: &JsonMap<String, JsonValue>,
    field_name: &str,
) -> PyResult<Vec<u8>> {
    let lookup = |id: &str| -> Option<&JsonValue> {
        spec.fields
            .iter()
            .find(|f| f.id == id)
            .and_then(|field| schema_mapping_value_json(mapping, field))
    };

    let textures_value = lookup("textures").cloned().unwrap_or(JsonValue::Null);
    let textures_array: Vec<JsonValue> = if textures_value.is_null() {
        Vec::new()
    } else {
        json_array(&textures_value, &format!("{field_name}.Textures"))
            .map_err(|_| value_error(format!("{field_name}.Textures must be a list")))?
            .to_vec()
    };
    let addon_nodes_value = lookup("addon_nodes").cloned().unwrap_or(JsonValue::Null);
    let addon_nodes_array: Vec<JsonValue> = if addon_nodes_value.is_null() {
        Vec::new()
    } else {
        json_array(&addon_nodes_value, &format!("{field_name}.AddonNodes"))
            .map_err(|_| value_error(format!("{field_name}.AddonNodes must be a list")))?
            .to_vec()
    };
    let materials_value = lookup("materials").cloned().unwrap_or(JsonValue::Null);
    let materials_array: Vec<JsonValue> = if materials_value.is_null() {
        Vec::new()
    } else {
        json_array(&materials_value, &format!("{field_name}.Materials"))
            .map_err(|_| value_error(format!("{field_name}.Materials must be a list")))?
            .to_vec()
    };
    let srgb_count = lookup("srgb_count")
        .map(|v| json_parse_int_authoring(v, &format!("{field_name}.SrgbCount"), 0))
        .transpose()?
        .unwrap_or(0) as u32;

    let mut out: Vec<u8> = Vec::new();
    out.extend_from_slice(&4_u32.to_le_bytes());
    out.extend_from_slice(&(textures_array.len() as u32).to_le_bytes());
    out.extend_from_slice(&(addon_nodes_array.len() as u32).to_le_bytes());
    out.extend_from_slice(&srgb_count.to_le_bytes());
    out.extend_from_slice(&(materials_array.len() as u32).to_le_bytes());

    for (index, row) in textures_array.iter().enumerate() {
        out.extend_from_slice(&encode_model_info_entry_json(
            row,
            &format!("{field_name}.Textures[{index}]"),
        )?);
    }
    for (index, value) in addon_nodes_array.iter().enumerate() {
        let node = json_parse_int_authoring(value, &format!("{field_name}.AddonNodes[{index}]"), 0)?
            as u32;
        out.extend_from_slice(&node.to_le_bytes());
    }
    for (index, row) in materials_array.iter().enumerate() {
        out.extend_from_slice(&encode_model_info_entry_json(
            row,
            &format!("{field_name}.Materials[{index}]"),
        )?);
    }

    Ok(out)
}

/// Encode one Texture/Material row (12 bytes: file_hash, 4-byte ascii
/// extension NUL-padded, folder_hash). Byte-exact inverse of
/// `decode_model_info_entry_json`.
fn encode_model_info_entry_json(row: &JsonValue, row_name: &str) -> PyResult<[u8; 12]> {
    let row_obj = row
        .as_object()
        .ok_or_else(|| value_error(format!("{row_name} must be an object")))?;
    let file_hash = row_obj
        .get("FileHash")
        .or_else(|| row_obj.get("file_hash"))
        .map(|v| json_parse_int_authoring(v, &format!("{row_name}.FileHash"), 0))
        .transpose()?
        .unwrap_or(0) as u32;
    let extension = row_obj
        .get("Extension")
        .or_else(|| row_obj.get("extension"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if extension.len() > 4 || !extension.is_ascii() {
        return Err(value_error(format!(
            "{row_name}.Extension must be at most 4 ASCII characters"
        )));
    }
    let folder_hash = row_obj
        .get("FolderHash")
        .or_else(|| row_obj.get("folder_hash"))
        .map(|v| json_parse_int_authoring(v, &format!("{row_name}.FolderHash"), 0))
        .transpose()?
        .unwrap_or(0) as u32;

    let mut out = [0u8; 12];
    out[0..4].copy_from_slice(&file_hash.to_le_bytes());
    out[4..4 + extension.len()].copy_from_slice(extension.as_bytes());
    out[8..12].copy_from_slice(&folder_hash.to_le_bytes());
    Ok(out)
}

fn field_enum_def_json(
    context: &NativeImportContext,
    field: &SchemaFieldJson,
) -> Option<SchemaEnumJson> {
    field.enum_ref.as_ref().and_then(|enum_ref| {
        context
            .game
            .as_deref()
            .and_then(|game| compiled_schema_for_game(game).ok())
            .and_then(|schema| schema.enums.get(enum_ref).cloned())
    })
}

fn encode_schema_field_token_json(
    signature: &str,
    token: &str,
    field: &SchemaFieldJson,
    value: &JsonValue,
    context: &mut NativeImportContext,
    field_name: &str,
) -> PyResult<Vec<u8>> {
    let token_codec = codec_from_token(token);
    let enum_def = field_enum_def_json(context, field);
    let scalar_codec = match field.kind.as_str() {
        "enum" => token_codec.clone().unwrap_or_else(|| "uint32".to_string()),
        "formid" => "formid".to_string(),
        kind if scalar_codec_supported(kind) => kind.to_string(),
        _ => token_codec.unwrap_or_else(|| field.kind.clone()),
    };
    let encoded = encode_scalar_codec_json(
        scalar_codec.as_str(),
        value,
        field_name,
        enum_def.as_ref(),
        context,
        field.formlink_target.as_deref(),
    )?;
    if let Some(expected) = token_width(token) {
        if encoded.len() != expected {
            return Err(value_error(format!(
                "{signature}.{field_name} encoded size {} does not match {}",
                encoded.len(),
                expected
            )));
        }
    }
    Ok(encoded)
}

fn encode_schema_array_elements_json(
    signature: &str,
    field: &SchemaFieldJson,
    element_codec: &str,
    values: &[JsonValue],
    context: &mut NativeImportContext,
    field_name: &str,
) -> PyResult<Vec<u8>> {
    let mut out = Vec::new();
    let row_codec = format!("struct:{element_codec}");
    let tokens = struct_tokens(row_codec.as_str());
    if tokens.is_empty() && !values.is_empty() {
        return Err(value_error(format!(
            "{field_name} array field has no element codec"
        )));
    }
    if field.fields.is_empty() {
        if tokens.len() != 1 {
            return Err(value_error(format!(
                "{field_name} scalar array element codec must have one token"
            )));
        }
        for (index, value) in values.iter().enumerate() {
            out.extend_from_slice(&encode_schema_field_token_json(
                signature,
                tokens[0],
                field,
                value,
                context,
                &format!("{field_name}[{index}]"),
            )?);
        }
        return Ok(out);
    }
    for (index, value) in values.iter().enumerate() {
        let row = json_object(value, &format!("{field_name}[{index}]"))?;
        if field
            .fields
            .iter()
            .any(|nested| !nested.union_variants.is_empty())
        {
            out.extend_from_slice(&encode_variable_struct_json(
                signature,
                &field.fields,
                row,
                context,
                &format!("{field_name}[{index}]"),
            )?);
            continue;
        }
        out.extend_from_slice(&encode_schema_struct_with_arrays_json(
            signature,
            row_codec.as_str(),
            &field.fields,
            row,
            context,
            &format!("{field_name}[{index}]"),
        )?);
    }
    Ok(out)
}

fn encode_schema_array_count_json(
    count: usize,
    count_codec: Option<&str>,
    field_name: &str,
) -> PyResult<Vec<u8>> {
    match count_codec {
        None => {
            if count > u8::MAX as usize {
                return Err(value_error(format!(
                    "{field_name} has too many entries for an implicit u8 count"
                )));
            }
            Ok(vec![count as u8])
        }
        Some("uint8" | "B") => {
            if count > u8::MAX as usize {
                return Err(value_error(format!(
                    "{field_name} has too many entries for a uint8 count"
                )));
            }
            Ok(vec![count as u8])
        }
        Some("uint16" | "H") => {
            if count > u16::MAX as usize {
                return Err(value_error(format!(
                    "{field_name} has too many entries for a uint16 count"
                )));
            }
            Ok((count as u16).to_le_bytes().to_vec())
        }
        Some("uint32" | "I") => Ok((count as u32).to_le_bytes().to_vec()),
        Some("payload_div_76") => Ok(Vec::new()),
        Some(other) => Err(value_error(format!(
            "{field_name} uses unsupported array count codec {other:?}"
        ))),
    }
}

fn encode_schema_struct_with_arrays_json(
    signature: &str,
    codec: &str,
    fields: &[SchemaFieldJson],
    mapping: &JsonMap<String, JsonValue>,
    context: &mut NativeImportContext,
    field_name: &str,
) -> PyResult<Vec<u8>> {
    let tokens = struct_tokens(codec);
    let mut token_index = 0usize;
    let mut encoded = Vec::new();
    let empty_array = JsonValue::Array(Vec::new());
    let mut count_values: HashMap<String, usize> = HashMap::new();
    for field in fields {
        let Some(array) = field.array.as_ref() else {
            continue;
        };
        let Some(count_field) = array.count_field.as_ref() else {
            continue;
        };
        let field_value = schema_mapping_value_json(mapping, field).unwrap_or(&empty_array);
        let values = json_array(
            field_value,
            &format!("{field_name}.{}", schema_field_key(field)),
        )?;
        count_values.insert(count_field.clone(), values.len());
    }

    for field in fields {
        while token_index < tokens.len() && tokens[token_index] == "x" {
            encoded
                .extend(std::iter::repeat(0).take(token_width(tokens[token_index]).unwrap_or(1)));
            token_index += 1;
        }
        if field.kind == "empty" {
            continue;
        }
        let default_value;
        let mut field_value = schema_mapping_value_json(mapping, field);
        let count_value;
        if let Some(count) = count_values.get(field.id.as_str()) {
            count_value = JsonValue::Number((*count as u64).into());
            field_value = Some(&count_value);
        }
        if let Some(array) = field.array.as_ref() {
            let values_payload = field_value.unwrap_or(&empty_array);
            let values = json_array(
                values_payload,
                &format!("{field_name}.{}", schema_field_key(field)),
            )?;
            if array.count_field.is_none() && array.count_record_field.is_none() {
                // Mirror the decoder: when count_codec is None, the array is
                // "fill remaining" — no count prefix on the wire (LCTN.LCEC
                // cells, etc.). Only emit a count byte/word when an explicit
                // count_codec is set.
                if array.count_codec.is_some() {
                    encoded.extend_from_slice(&encode_schema_array_count_json(
                        values.len(),
                        array.count_codec.as_deref(),
                        &format!("{field_name}.{}", field.id),
                    )?);
                }
            }
            let element_codec = array.element_codec.as_deref().ok_or_else(|| {
                value_error(format!(
                    "{field_name}.{} array has no element_codec",
                    field.id
                ))
            })?;
            encoded.extend_from_slice(&encode_schema_array_elements_json(
                signature,
                field,
                element_codec,
                values,
                context,
                &format!("{field_name}.{}", schema_field_key(field)),
            )?);
            continue;
        }
        let token = tokens.get(token_index).ok_or_else(|| {
            value_error(format!("{field_name}.{} missing struct token", field.id))
        })?;
        if field_value.is_none() {
            default_value = default_field_input_from_schema_json(field);
            field_value = Some(&default_value);
        }
        encoded.extend_from_slice(&encode_schema_field_token_json(
            signature,
            token,
            field,
            field_value.expect("field value is populated"),
            context,
            &format!("{field_name}.{}", schema_field_key(field)),
        )?);
        token_index += 1;
    }
    while token_index < tokens.len() && tokens[token_index] == "x" {
        encoded.extend(std::iter::repeat(0).take(token_width(tokens[token_index]).unwrap_or(1)));
        token_index += 1;
    }
    if token_index != tokens.len() {
        return Err(value_error(format!(
            "{field_name} did not consume all struct tokens"
        )));
    }
    Ok(encoded)
}

/// True when a field list cannot be encoded by the fixed
/// `encode_structured_mapping_json` / `encode_schema_struct_with_arrays_json`
/// paths and must use the VariableStruct encoder instead. Mirrors the decoder
/// builder's branching condition (array, nested struct, or field-level union)
/// so encode/decode stay in lockstep — any field shape that triggers
/// `decode_spec_for_var_fields_rs` on the read side must also route through
/// `encode_variable_struct_json` on the write side.
pub fn schema_fields_need_variable_struct(codec: &str, fields: &[SchemaFieldJson]) -> bool {
    let codec_has_variable_string = codec.strip_prefix("struct:").is_some_and(|rest| {
        rest.split(',').map(|token| token.trim()).any(|token| {
            matches!(
                token,
                "zstring" | "lenstring8" | "lenstring16" | "lenstring32"
            )
        })
    });
    codec_has_variable_string
        || fields.iter().any(|field| {
            field.array.is_some() || !field.fields.is_empty() || !field.union_variants.is_empty()
        })
}

/// Encoder for layouts whose decoder produces `DecodeSpec::VariableStruct`.
/// Walks fields left-to-right, emitting bytes per field shape:
///   - length-prefixed array  → count + N elements
///   - sibling-discriminated union → {"variant": ..., "value": ...} envelope
///   - plain scalar → encode with field.kind codec
///
/// Does NOT consume parent codec tokens; the codec is informational only.
fn encode_variable_struct_json(
    signature: &str,
    fields: &[SchemaFieldJson],
    mapping: &JsonMap<String, JsonValue>,
    context: &mut NativeImportContext,
    field_name: &str,
) -> PyResult<Vec<u8>> {
    let empty_array = JsonValue::Array(Vec::new());
    let null_value = JsonValue::Null;
    let mut encoded = Vec::new();
    for field in fields {
        if field.kind == "empty" {
            continue;
        }
        let key_path = format!("{field_name}.{}", schema_field_key(field));
        // Length-prefixed array.
        if let Some(array) = field.array.as_ref() {
            let values_payload = schema_mapping_value_json(mapping, field).unwrap_or(&empty_array);
            let values = json_array(values_payload, &key_path)?;
            if array.count_field.is_none() && array.count_record_field.is_none() {
                if array.count_codec.is_some() {
                    encoded.extend_from_slice(&encode_schema_array_count_json(
                        values.len(),
                        array.count_codec.as_deref(),
                        &key_path,
                    )?);
                }
            }
            let element_codec = array
                .element_codec
                .as_deref()
                .ok_or_else(|| value_error(format!("{key_path} array has no element_codec")))?;
            encoded.extend_from_slice(&encode_schema_array_elements_json(
                signature,
                field,
                element_codec,
                values,
                context,
                &key_path,
            )?);
            continue;
        }
        // Sibling-discriminated union.
        if !field.union_variants.is_empty() {
            let value = schema_mapping_value_json(mapping, field).unwrap_or(&null_value);
            encoded.extend_from_slice(&encode_union_value_json(
                signature,
                &field.union_variants,
                value,
                context,
                &key_path,
                false,
            )?);
            continue;
        }
        // Nested struct without an array (rare, e.g. an embedded fixed-size
        // sub-struct). Defer until a real record exercises this case.
        if !field.fields.is_empty() {
            return Err(value_error(format!(
                "{key_path} nested-struct field is not yet supported by the VariableStruct encoder"
            )));
        }
        // Plain scalar.
        let scalar_codec = match field.kind.as_str() {
            "int8" | "uint8" | "int16" | "uint16" | "int32" | "uint32" | "int64" | "uint64"
            | "float32" | "formid" | "zstring" | "lenstring8" | "lenstring16" | "lenstring32" => {
                field.kind.clone()
            }
            "enum" | "flags" => "uint32".to_string(),
            _ => {
                return Err(value_error(format!(
                    "{key_path} unsupported scalar kind {} for VariableStruct",
                    field.kind
                )));
            }
        };
        let value = schema_mapping_value_json(mapping, field).unwrap_or(&null_value);
        let enum_def = field_enum_def_json(context, field);
        encoded.extend_from_slice(&encode_scalar_codec_json(
            scalar_codec.as_str(),
            value,
            &key_path,
            enum_def.as_ref(),
            context,
            field.formlink_target.as_deref(),
        )?);
    }
    Ok(encoded)
}

/// Re-encode a `custom_codec` subrecord (NVNM, LAND VHGT/VNML) from its
/// structured authoring payload. The export side flattens the codec's
/// `to_yaml` Value into the subrecord payload alongside `raw_hex`, so reading
/// the payload as a JSON object and passing it back through `*_from_yaml`
/// reconstructs the bytes. Returns `None` when the codec name is not handled
/// or when re-encoding fails — callers then fall back to the raw_hex path.
fn try_encode_custom_codec_subrecord(
    payload: &JsonMap<String, JsonValue>,
    spec: &SchemaSubrecordJson,
) -> Option<Vec<u8>> {
    let codec = spec.codec.as_deref()?;
    let value = JsonValue::Object(payload.clone());
    match codec {
        "esp_authoring_core::nvnm" => {
            let nvnm = crate::nvnm::nvnm_from_yaml(&value).ok()?;
            Some(crate::nvnm::write_nvnm(&nvnm))
        }
        "esp_authoring_core::land::heightmap" => match spec.id.as_str() {
            "VHGT" => {
                let map = crate::land::heightmap::heightmap_from_yaml(&value).ok()?;
                Some(crate::land::heightmap::write_heightmap(&map))
            }
            "VNML" => {
                let n = crate::land::heightmap::vertex_normals_from_yaml(&value).ok()?;
                Some(crate::land::heightmap::write_vertex_normals(&n))
            }
            _ => None,
        },
        _ => None,
    }
}

fn encode_typed_subrecord_value_json(
    signature: &str,
    payload: &JsonMap<String, JsonValue>,
    spec: &SchemaSubrecordJson,
    context: &mut NativeImportContext,
) -> PyResult<Vec<u8>> {
    if !spec.union_variants.is_empty() {
        // Truncatable wbUnion-of-wbStruct: parent kind drives whether each
        // variant struct truncates trailing absent fields, mirroring the
        // decoder's `parse_partial` path in decode_spec_for_union_rs.
        let truncate_trailing_absent = spec.kind == "parsed_with_raw_fallback";
        return encode_union_value_json(
            signature,
            &spec.union_variants,
            &JsonValue::Object(payload.clone()),
            context,
            signature,
            truncate_trailing_absent,
        );
    }
    let codec = spec.codec.as_deref().unwrap_or_default();
    if codec == "omod_data" {
        let mapping_value = payload
            .get("fields")
            .or_else(|| payload.get("value"))
            .ok_or_else(|| value_error(format!("{signature} omod_data value is required")))?;
        let mapping = json_object(mapping_value, signature)
            .map_err(|_| value_error(format!("{signature} omod_data value must be an object")))?;
        return encode_omod_data_json(spec, mapping, context, signature);
    }
    if codec == "model_info" {
        let mapping_value = payload
            .get("fields")
            .or_else(|| payload.get("value"))
            .ok_or_else(|| value_error(format!("{signature} model_info value is required")))?;
        let mapping = json_object(mapping_value, signature)
            .map_err(|_| value_error(format!("{signature} model_info value must be an object")))?;
        return encode_model_info_json(spec, mapping, signature);
    }
    if codec.starts_with("struct:") && schema_fields_need_variable_struct(codec, &spec.fields) {
        let mapping_value = payload
            .get("fields")
            .or_else(|| payload.get("value"))
            .ok_or_else(|| value_error(format!("{signature} structured value is required")))?;
        let mapping = json_object(mapping_value, signature)
            .map_err(|_| value_error(format!("{signature} structured value must be an object")))?;
        return encode_variable_struct_json(signature, &spec.fields, mapping, context, signature);
    }
    if codec.starts_with("struct:") && spec.fields.iter().any(|field| field.array.is_some()) {
        let mapping_value = payload
            .get("fields")
            .or_else(|| payload.get("value"))
            .ok_or_else(|| value_error(format!("{signature} structured value is required")))?;
        let mapping = json_object(mapping_value, signature)
            .map_err(|_| value_error(format!("{signature} structured value must be an object")))?;
        return encode_schema_struct_with_arrays_json(
            signature,
            codec,
            &spec.fields,
            mapping,
            context,
            signature,
        );
    }
    if codec.starts_with("array_struct:") {
        let rows = payload
            .get("rows")
            .or_else(|| payload.get("value"))
            .ok_or_else(|| value_error(format!("{signature} array_struct value is required")))?;
        return encode_array_struct_value_json(
            signature,
            codec,
            &spec.fields,
            rows,
            context,
            signature,
        );
    }
    if codec.starts_with("struct:") {
        let mapping_value = payload
            .get("fields")
            .or_else(|| payload.get("value"))
            .ok_or_else(|| value_error(format!("{signature} structured value is required")))?;
        let mapping = json_object(mapping_value, signature)
            .map_err(|_| value_error(format!("{signature} structured value must be an object")))?;
        // Variable-length structs (kind=parsed_with_raw_fallback) skip the
        // default-fill for trailing absent fields so the re-encoded byte
        // length matches the original source.
        let truncate_trailing_absent = spec.kind == "parsed_with_raw_fallback";
        return encode_structured_mapping_json(
            signature,
            codec,
            &spec.fields,
            mapping,
            context,
            signature,
            truncate_trailing_absent,
        );
    }
    let value = payload
        .get("value")
        .unwrap_or_else(|| payload.get("raw_hex").unwrap_or(&JsonValue::Null));
    let enum_def = spec.enum_ref.as_ref().and_then(|enum_ref| {
        context
            .game
            .as_deref()
            .and_then(|game| compiled_schema_for_game(game).ok())
            .and_then(|schema| schema.enums.get(enum_ref).cloned())
    });
    encode_scalar_codec_json(
        codec,
        value,
        signature,
        enum_def.as_ref(),
        context,
        spec.formlink_target.as_deref(),
    )
}

/// Encode a VMAD subrecord from a parsed authoring payload (the JSON shape
/// produced by `compact_vmad_payload_json`). Handles scalar property types
/// 0–7 and array property types 11–17, plus fragment blocks for INFO/PACK/
/// QUST/SCEN/PERK/TERM. Returns `None` when the payload references property
/// types or fragment data not yet supported.
///
/// Wired into the main encoder path via
/// `build_subrecord_from_authoring_field_json_native`; the decoder uses it
/// as a defensive round-trip check before omitting `raw_hex`.
pub fn build_vmad_bytes_from_payload(
    payload: &JsonValue,
    masters: &[String],
    plugin_name: &str,
) -> Option<Vec<u8>> {
    let payload = payload.as_object()?;
    let version = payload.get("Version")?.as_u64()? as u16;
    let object_format = payload.get("Object Format")?.as_u64()? as u16;
    let scripts = payload.get("Scripts")?.as_array()?;

    let mut out = Vec::new();
    out.extend_from_slice(&version.to_le_bytes());
    out.extend_from_slice(&object_format.to_le_bytes());
    out.extend_from_slice(&(scripts.len() as u16).to_le_bytes());

    for script in scripts {
        write_vmad_script_entry(&mut out, script, object_format, masters, plugin_name)?;
    }

    if let Some(fragments) = payload.get("Script Fragments") {
        let semantic = payload
            .get("semantic_type")
            .and_then(|value| value.as_str());
        match semantic {
            Some("INFO") | Some("PACK") => {
                write_vmad_fragments_info_or_pack(
                    &mut out,
                    fragments,
                    object_format,
                    masters,
                    plugin_name,
                )?;
            }
            Some("SCEN") => {
                write_vmad_fragments_scen(
                    &mut out,
                    fragments,
                    object_format,
                    masters,
                    plugin_name,
                )?;
            }
            Some("PERK") | Some("TERM") => {
                write_vmad_fragments_perk_term(
                    &mut out,
                    fragments,
                    object_format,
                    masters,
                    plugin_name,
                )?;
            }
            Some("QUST") => {
                write_vmad_fragments_quest(
                    &mut out,
                    fragments,
                    object_format,
                    masters,
                    plugin_name,
                )?;
            }
            _ => return None,
        }
    } else if let Some(tail_hex) = payload.get("tail_hex").and_then(|value| value.as_str()) {
        // No parsed fragments but the source had unparsed tail bytes — fall
        // back to raw_hex preservation by signalling failure to the caller.
        if !tail_hex.is_empty() {
            return None;
        }
    }

    Some(out)
}

fn write_vmad_script_entry(
    out: &mut Vec<u8>,
    script: &JsonValue,
    object_format: u16,
    masters: &[String],
    plugin_name: &str,
) -> Option<()> {
    let script_obj = script.as_object()?;
    let script_name = script_obj.get("ScriptName")?.as_str()?;
    write_vmad_string(out, script_name);
    let flags = script_obj
        .get("Flags")
        .and_then(|value| value.as_u64())
        .unwrap_or(0) as u8;
    out.push(flags);

    let properties = script_obj.get("Properties")?.as_array()?;
    out.extend_from_slice(&(properties.len() as u16).to_le_bytes());

    for property in properties {
        write_vmad_property_entry(out, property, object_format, masters, plugin_name)?;
    }
    Some(())
}

fn write_vmad_property_entry(
    out: &mut Vec<u8>,
    property: &JsonValue,
    object_format: u16,
    masters: &[String],
    plugin_name: &str,
) -> Option<()> {
    let property_obj = property.as_object()?;
    let property_name = property_obj.get("propertyName")?.as_str()?;
    let type_label = property_obj.get("Type")?.as_str()?;
    let property_type = vmad_type_value_for_label(type_label)?;
    let property_flags = property_obj
        .get("Flags")
        .and_then(|value| value.as_u64())
        .unwrap_or(0) as u8;
    let value = property_obj.get("Value")?;
    write_vmad_string(out, property_name);
    out.push(property_type);
    out.push(property_flags);
    write_vmad_property_value(
        out,
        property_type,
        object_format,
        value,
        masters,
        plugin_name,
    )
}

fn write_vmad_simple_fragment(out: &mut Vec<u8>, fragment: &JsonValue) -> Option<()> {
    let entry = fragment.as_object()?;
    let unknown = entry.get("Unknown")?.as_i64()? as i8;
    out.push(unknown as u8);
    write_vmad_string(out, entry.get("ScriptName")?.as_str()?);
    write_vmad_string(out, entry.get("FragmentName")?.as_str()?);
    Some(())
}

fn write_vmad_fragments_info_or_pack(
    out: &mut Vec<u8>,
    fragments: &JsonValue,
    object_format: u16,
    masters: &[String],
    plugin_name: &str,
) -> Option<()> {
    let block = fragments.as_object()?;
    let version = block.get("Version")?.as_i64()? as i8;
    let flags = block.get("Flags")?.as_u64()? as u8;
    out.push(version as u8);
    out.push(flags);
    write_vmad_script_entry(
        out,
        block.get("Script")?,
        object_format,
        masters,
        plugin_name,
    )?;
    let entries = block.get("Fragments")?.as_array()?;
    let expected = (flags as u32).count_ones() as usize;
    if entries.len() != expected {
        return None;
    }
    for entry in entries {
        write_vmad_simple_fragment(out, entry)?;
    }
    Some(())
}

fn write_vmad_fragments_scen(
    out: &mut Vec<u8>,
    fragments: &JsonValue,
    object_format: u16,
    masters: &[String],
    plugin_name: &str,
) -> Option<()> {
    write_vmad_fragments_info_or_pack(out, fragments, object_format, masters, plugin_name)?;
    let block = fragments.as_object()?;
    let phases = block.get("Phase Fragments")?.as_array()?;
    out.extend_from_slice(&(phases.len() as u16).to_le_bytes());
    for phase in phases {
        let entry = phase.as_object()?;
        out.push(entry.get("Phase Flag")?.as_u64()? as u8);
        out.push(entry.get("Phase Index")?.as_u64()? as u8);
        let unknown_s16 = entry.get("Unknown")?.as_i64()? as i16;
        out.extend_from_slice(&unknown_s16.to_le_bytes());
        out.push(entry.get("Unknown1")?.as_i64()? as i8 as u8);
        out.push(entry.get("Unknown2")?.as_i64()? as i8 as u8);
        write_vmad_string(out, entry.get("ScriptName")?.as_str()?);
        write_vmad_string(out, entry.get("FragmentName")?.as_str()?);
    }
    Some(())
}

fn write_vmad_fragments_perk_term(
    out: &mut Vec<u8>,
    fragments: &JsonValue,
    object_format: u16,
    masters: &[String],
    plugin_name: &str,
) -> Option<()> {
    let block = fragments.as_object()?;
    let version = block.get("Version")?.as_i64()? as i8;
    out.push(version as u8);
    write_vmad_script_entry(
        out,
        block.get("Script")?,
        object_format,
        masters,
        plugin_name,
    )?;
    let entries = block.get("Fragments")?.as_array()?;
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    for entry in entries {
        let fragment = entry.as_object()?;
        let fragment_index = fragment.get("Fragment Index")?.as_u64()? as u16;
        out.extend_from_slice(&fragment_index.to_le_bytes());
        let unused = fragment.get("Unused")?.as_i64()? as i16;
        out.extend_from_slice(&unused.to_le_bytes());
        let unknown = fragment.get("Unknown")?.as_i64()? as i8;
        out.push(unknown as u8);
        write_vmad_string(out, fragment.get("ScriptName")?.as_str()?);
        write_vmad_string(out, fragment.get("FragmentName")?.as_str()?);
    }
    Some(())
}

fn write_vmad_fragments_quest(
    out: &mut Vec<u8>,
    fragments: &JsonValue,
    object_format: u16,
    masters: &[String],
    plugin_name: &str,
) -> Option<()> {
    let block = fragments.as_object()?;
    let version = block.get("Version")?.as_i64()? as i8;
    out.push(version as u8);
    let frag_array = block.get("Fragments")?.as_array()?;
    out.extend_from_slice(&(frag_array.len() as u16).to_le_bytes());

    let script = block.get("Script")?.as_object()?;
    let script_name = script.get("ScriptName")?.as_str()?;
    write_vmad_string(out, script_name);
    if !script_name.is_empty() {
        let script_flags = script.get("Flags").and_then(|v| v.as_u64()).unwrap_or(0) as u8;
        out.push(script_flags);
        let properties = script.get("Properties")?.as_array()?;
        out.extend_from_slice(&(properties.len() as u16).to_le_bytes());
        for property in properties {
            write_vmad_property_entry(out, property, object_format, masters, plugin_name)?;
        }
    }

    for entry in frag_array {
        let fragment = entry.as_object()?;
        out.extend_from_slice(&(fragment.get("Quest Stage")?.as_u64()? as u16).to_le_bytes());
        out.extend_from_slice(&(fragment.get("Unknown")?.as_i64()? as i16).to_le_bytes());
        out.extend_from_slice(&(fragment.get("Quest Stage Index")?.as_i64()? as i32).to_le_bytes());
        out.push(fragment.get("Unknown1")?.as_i64()? as i8 as u8);
        write_vmad_string(out, fragment.get("ScriptName")?.as_str()?);
        write_vmad_string(out, fragment.get("FragmentName")?.as_str()?);
    }

    let aliases = block.get("Aliases")?.as_array()?;
    out.extend_from_slice(&(aliases.len() as u16).to_le_bytes());
    for alias in aliases {
        let alias_obj = alias.as_object()?;
        write_vmad_object(
            out,
            object_format,
            alias_obj.get("Object")?,
            masters,
            plugin_name,
        )?;
        let alias_version = alias_obj.get("Version")?.as_i64()? as i16;
        out.extend_from_slice(&alias_version.to_le_bytes());
        let alias_object_format = alias_obj.get("Object Format")?.as_i64()? as i16;
        out.extend_from_slice(&alias_object_format.to_le_bytes());
        let alias_scripts = alias_obj.get("Alias Scripts")?.as_array()?;
        out.extend_from_slice(&(alias_scripts.len() as u16).to_le_bytes());
        for alias_script in alias_scripts {
            write_vmad_script_entry(
                out,
                alias_script,
                alias_object_format as u16,
                masters,
                plugin_name,
            )?;
        }
    }

    Some(())
}

fn write_vmad_string(out: &mut Vec<u8>, value: &str) {
    let (encoded, _, _) = WINDOWS_1252.encode(value);
    let bytes = encoded.into_owned();
    let len = bytes.len() as u16;
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&bytes);
}

fn vmad_type_value_for_label(label: &str) -> Option<u8> {
    // Inverse of `vmad_type_label`. Labels match xEdit's wbPropTypeEnum so
    // round-trip through the YAML preserves the original numeric type byte.
    match label {
        "None" => Some(0),
        "Object" => Some(1),
        "String" => Some(2),
        "Int32" => Some(3),
        "Float" => Some(4),
        "Bool" => Some(5),
        "Variable" => Some(6),
        "Struct" => Some(7),
        "Array of Object" => Some(11),
        "Array of String" => Some(12),
        "Array of Int32" => Some(13),
        "Array of Float" => Some(14),
        "Array of Bool" => Some(15),
        "Array of Variable" => Some(16),
        "Array of Struct" => Some(17),
        _ => None,
    }
}

fn write_vmad_property_value(
    out: &mut Vec<u8>,
    property_type: u8,
    object_format: u16,
    value: &JsonValue,
    masters: &[String],
    plugin_name: &str,
) -> Option<()> {
    match property_type {
        0 | 6 => Some(()),
        1 => write_vmad_object(out, object_format, value, masters, plugin_name),
        2 => {
            let s = value.as_str()?;
            write_vmad_string(out, s);
            Some(())
        }
        3 => {
            let n = value.as_i64()? as i32;
            out.extend_from_slice(&n.to_le_bytes());
            Some(())
        }
        4 => {
            let n = value.as_f64()? as f32;
            out.extend_from_slice(&n.to_le_bytes());
            Some(())
        }
        5 => {
            let b = value.as_bool()?;
            out.push(if b { 1 } else { 0 });
            Some(())
        }
        11 => {
            let elements = value.as_array()?;
            out.extend_from_slice(&(elements.len() as i32).to_le_bytes());
            for element in elements {
                write_vmad_object(out, object_format, element, masters, plugin_name)?;
            }
            Some(())
        }
        12 => {
            let elements = value.as_array()?;
            out.extend_from_slice(&(elements.len() as i32).to_le_bytes());
            for element in elements {
                write_vmad_string(out, element.as_str()?);
            }
            Some(())
        }
        13 => {
            let elements = value.as_array()?;
            out.extend_from_slice(&(elements.len() as i32).to_le_bytes());
            for element in elements {
                let n = element.as_i64()? as i32;
                out.extend_from_slice(&n.to_le_bytes());
            }
            Some(())
        }
        14 => {
            let elements = value.as_array()?;
            out.extend_from_slice(&(elements.len() as i32).to_le_bytes());
            for element in elements {
                let n = element.as_f64()? as f32;
                out.extend_from_slice(&n.to_le_bytes());
            }
            Some(())
        }
        15 => {
            let elements = value.as_array()?;
            out.extend_from_slice(&(elements.len() as i32).to_le_bytes());
            for element in elements {
                out.push(if element.as_bool()? { 1 } else { 0 });
            }
            Some(())
        }
        7 => write_vmad_struct(out, object_format, value, masters, plugin_name),
        16 => {
            // xEdit FO4 wbScriptPropertyDecider:{16} models Array of Variable
            // as a single u32 element-count field (no element bytes follow).
            let count = value
                .as_object()
                .and_then(|obj| obj.get("Element Count"))
                .and_then(|v| v.as_u64())? as u32;
            out.extend_from_slice(&count.to_le_bytes());
            Some(())
        }
        17 => {
            let elements = value.as_array()?;
            out.extend_from_slice(&(elements.len() as i32).to_le_bytes());
            for element in elements {
                write_vmad_struct(out, object_format, element, masters, plugin_name)?;
            }
            Some(())
        }
        _ => None,
    }
}

/// Encode a single VMAD `wbScriptPropertyStruct`. Layout per xEdit FO4
/// wbScriptPropertyStruct (refs/xedit/Core/wbDefinitionsFO4.pas:4137):
/// `<i32 member_count><member...>` where each member is
/// `<u16 name_len><name><u8 type><u8 flags><value bytes per type>`.
fn write_vmad_struct(
    out: &mut Vec<u8>,
    object_format: u16,
    value: &JsonValue,
    masters: &[String],
    plugin_name: &str,
) -> Option<()> {
    let members = value.as_array()?;
    out.extend_from_slice(&(members.len() as i32).to_le_bytes());
    for member in members {
        let entry = member.as_object()?;
        let member_name = entry.get("memberName")?.as_str()?;
        let type_label = entry.get("Type")?.as_str()?;
        let member_type = vmad_type_value_for_label(type_label)?;
        let member_flags = entry.get("Flags").and_then(|v| v.as_u64()).unwrap_or(0) as u8;
        let member_value = entry.get("Value")?;
        write_vmad_string(out, member_name);
        out.push(member_type);
        out.push(member_flags);
        write_vmad_property_value(
            out,
            member_type,
            object_format,
            member_value,
            masters,
            plugin_name,
        )?;
    }
    Some(())
}

fn write_vmad_object(
    out: &mut Vec<u8>,
    object_format: u16,
    value: &JsonValue,
    masters: &[String],
    plugin_name: &str,
) -> Option<()> {
    let object = value.as_object()?;
    let alias = object.get("Alias").and_then(|v| v.as_i64()).unwrap_or(0) as i16;
    let unused = object.get("Unused").and_then(|v| v.as_u64()).unwrap_or(0) as u16;
    let formid = parse_vmad_formid(object.get("FormID"), masters, plugin_name)?;
    if object_format == 2 {
        out.extend_from_slice(&unused.to_le_bytes());
        out.extend_from_slice(&alias.to_le_bytes());
        out.extend_from_slice(&formid.to_le_bytes());
    } else {
        out.extend_from_slice(&formid.to_le_bytes());
        out.extend_from_slice(&alias.to_le_bytes());
        out.extend_from_slice(&unused.to_le_bytes());
    }
    Some(())
}

fn parse_vmad_formid(
    value: Option<&JsonValue>,
    masters: &[String],
    plugin_name: &str,
) -> Option<u32> {
    let value = value?;
    if value.is_null() {
        return Some(0);
    }
    let mapping = value.as_object()?;
    if let Some(raw) = mapping.get("raw").and_then(|v| v.as_str()) {
        return u32::from_str_radix(raw.trim(), 16).ok();
    }
    let reference = mapping.get("reference")?.as_object()?;
    let object_id_text = reference.get("object_id")?.as_str()?;
    let object_id = u32::from_str_radix(object_id_text.trim(), 16).ok()? & 0x00FF_FFFF;
    let index: u32 = if let Some(plugin) = reference.get("plugin").and_then(|v| v.as_str()) {
        if plugin.eq_ignore_ascii_case(plugin_name) {
            masters.len() as u32
        } else {
            masters
                .iter()
                .position(|m| m.eq_ignore_ascii_case(plugin))
                .map(|p| p as u32)?
        }
    } else if let Some(missing) = reference.get("missing_index").and_then(|v| v.as_u64()) {
        missing as u32 & 0xFF
    } else {
        0xFF
    };
    Some((index << 24) | object_id)
}

fn build_subrecord_from_authoring_field_json_native(
    signature: &str,
    payload: &JsonMap<String, JsonValue>,
    spec: Option<&SchemaSubrecordJson>,
    context: &mut NativeImportContext,
    localized: bool,
) -> PyResult<ParsedSubrecord> {
    let mut semantic_type = json_optional_string(payload.get("semantic_type"))?;
    if semantic_type.is_none() {
        if let Some(spec) = spec {
            if matches!(spec.codec.as_deref(), Some("formid") | Some("formid_array")) {
                semantic_type = spec.codec.clone();
            }
        }
    }
    let preservation_mode = payload
        .get("preservation_mode")
        .and_then(|value| value.as_str())
        .map(|value| value.to_string())
        .unwrap_or_else(|| {
            spec.map(|value| native_preservation_mode(value.kind.as_str()).to_string())
                .unwrap_or_else(|| "raw_only".to_string())
        });
    let has_typed_payload = ["value", "fields", "rows", "variant"]
        .iter()
        .any(|key| payload.contains_key(*key));
    let raw_hex = payload.get("raw_hex");
    let is_vmad_layout = spec
        .and_then(self::authoring::authoring_serialize::runtime_layout_for_subrecord_schema)
        == Some("vmad")
        || (signature == "VMAD" && spec.is_none());
    let data = if is_vmad_layout {
        // Prefer the parsed payload via
        // build_vmad_bytes_from_payload when present; raw_hex is only the
        // fallback when no parsed structure exists or when re-encoding fails.
        // This is what lets us drop the unconditional raw_hex tail in the
        // decoder — we trust the parsed payload to reconstruct the bytes.
        let parsed_value = payload
            .get("value")
            .and_then(|v| v.as_object())
            .map(|m| JsonValue::Object(m.clone()))
            .or_else(|| {
                if payload.contains_key("Scripts") {
                    Some(JsonValue::Object(payload.clone()))
                } else {
                    None
                }
            });
        let parsed_bytes = parsed_value.as_ref().and_then(|value| {
            build_vmad_bytes_from_payload(
                value,
                &context.header.masters,
                context.plugin_name.as_str(),
            )
        });
        if let Some(bytes) = parsed_bytes {
            bytes
        } else if raw_hex.is_some() {
            json_parse_hex_bytes(raw_hex, &format!("{signature}.raw_hex"))?
        } else if preservation_mode == "raw_only" || !has_typed_payload {
            json_parse_hex_bytes(raw_hex, &format!("{signature}.raw_hex"))?
        } else if let Some(raw_value) = payload.get("value") {
            if let Some(mapping) = raw_value.as_object() {
                let hex_source = mapping.get("raw_hex").or(raw_hex);
                json_parse_hex_bytes(hex_source, &format!("{signature}.raw_hex"))?
            } else {
                return Err(value_error("VMAD authoring payload must be an object"));
            }
        } else {
            return Err(value_error("VMAD authoring payload must be an object"));
        }
    } else if let Some(bytes) = spec
        .filter(|s| s.kind == "custom_codec")
        .and_then(|s| try_encode_custom_codec_subrecord(payload, s))
    {
        // `custom_codec` subrecords (NVNM, LAND VHGT/VNML) ship structured
        // fields alongside `raw_hex`. preservation_mode defaults to raw_only
        // for kind=custom_codec, so without this branch edits to those fields
        // in the authoring-dir YAML never reach the bytes. An unrecognised
        // codec or failed re-encode falls through to the raw_only path below.
        bytes
    } else if spec.is_none() || preservation_mode == "raw_only" || !has_typed_payload {
        if let Some(text) = compact_value_text_candidate(payload) {
            encode_cp1252(text, true)
        } else {
            json_parse_hex_bytes(raw_hex, &format!("{signature}.raw_hex"))?
        }
    } else if let Some(spec) = spec {
        if localized && spec.localized {
            if let Some(raw_value) = payload.get("value") {
                if let Some(mapping) = raw_value.as_object() {
                    if mapping.contains_key("TargetLanguage")
                        || mapping.contains_key("Values")
                        || mapping.contains_key("Value")
                    {
                        let (target_language, values_by_language) =
                            parse_localized_authoring_values_json(mapping)?;
                        let string_id = match raw_hex {
                            Some(value) if !value.is_null() => {
                                let raw = json_parse_hex_bytes(
                                    Some(value),
                                    &format!("{signature}.raw_hex"),
                                )?;
                                if raw.len() != 4 {
                                    return Err(value_error(format!(
                                        "{signature} localized raw_hex must be 4 bytes"
                                    )));
                                }
                                u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]])
                            }
                            _ if values_by_language.is_empty() => 0,
                            _ => context.allocate_localized_string_id(1),
                        };
                        if string_id != 0 {
                            let record_signature = context.current_record_signature.clone();
                            let table_type = io::localized_table_type_for_signature(
                                record_signature.as_deref(),
                                signature,
                            );
                            context.set_localized_field_values(
                                string_id,
                                &values_by_language,
                                Some(target_language.as_str()),
                                table_type,
                            );
                        }
                        string_id.to_le_bytes().to_vec()
                    } else {
                        encode_typed_subrecord_value_json(signature, payload, spec, context)?
                    }
                } else {
                    encode_typed_subrecord_value_json(signature, payload, spec, context)?
                }
            } else {
                encode_typed_subrecord_value_json(signature, payload, spec, context)?
            }
        } else {
            match encode_typed_subrecord_value_json(signature, payload, spec, context) {
                Ok(data) => data,
                Err(err) if preservation_mode == "hybrid" && raw_hex.is_some() => {
                    let _ = err;
                    json_parse_hex_bytes(raw_hex, &format!("{signature}.raw_hex"))?
                }
                Err(err) if compact_value_raw_hex_candidate(payload).is_some() => {
                    let _ = err;
                    json_parse_hex_bytes(
                        compact_value_raw_hex_candidate(payload),
                        &format!("{signature}.raw_hex"),
                    )?
                }
                Err(err) => return Err(err),
            }
        }
    } else {
        json_parse_hex_bytes(raw_hex, &format!("{signature}.raw_hex"))?
    };
    if semantic_type.is_none() {
        if matches!(spec, Some(spec) if localized && spec.localized && data.len() == 4) {
            semantic_type = Some("localized_string".to_string());
        }
    }
    Ok(ParsedSubrecord {
        signature: SmolStr::new(signature),
        data: Bytes::from(data),
        semantic_type,
    })
}

#[derive(Default)]
struct CompactAuthoringDispatchState {
    occurrence_counts: HashMap<String, usize>,
    occurrence_counts_by_scope: HashMap<(Option<String>, String), usize>,
    current_scope: Option<String>,
}

fn select_compact_authoring_subrecord_spec<'a>(
    record_spec: &'a SchemaRecordJson,
    raw_key: &str,
    signature: &str,
    state: &CompactAuthoringDispatchState,
) -> Option<&'a SchemaSubrecordJson> {
    if raw_key != signature {
        if let Some(matched) = find_unique_label_spec(record_spec, raw_key, signature) {
            return Some(matched);
        }
    }

    let current_scope = state.current_scope.as_deref();
    if schema_has_subrecord_in_scope(record_spec, signature, current_scope) {
        let key = (current_scope.map(str::to_string), signature.to_string());
        let occurrence = *state.occurrence_counts_by_scope.get(&key).unwrap_or(&0);
        if let Some(spec) =
            schema_subrecord_spec_in_scope(record_spec, signature, current_scope, occurrence)
        {
            return Some(spec);
        }
    }

    let mut tried: HashSet<Option<&str>> = HashSet::new();
    tried.insert(current_scope);
    for spec in &record_spec.subrecords {
        if spec.id != signature {
            continue;
        }
        let scope = spec.scope_id.as_deref();
        if !tried.insert(scope) {
            continue;
        }
        let key = (scope.map(str::to_string), signature.to_string());
        let occurrence = *state.occurrence_counts_by_scope.get(&key).unwrap_or(&0);
        if let Some(found) =
            schema_subrecord_spec_in_scope(record_spec, signature, scope, occurrence)
        {
            return Some(found);
        }
    }

    let occurrence = *state.occurrence_counts.get(signature).unwrap_or(&0);
    schema_subrecord_spec(record_spec, signature, occurrence)
}

fn advance_compact_authoring_dispatch_state(
    state: &mut CompactAuthoringDispatchState,
    signature: &str,
    spec: Option<&SchemaSubrecordJson>,
) {
    *state
        .occurrence_counts
        .entry(signature.to_string())
        .or_insert(0) += 1;
    if let Some(spec) = spec {
        let scope = spec.scope_id.clone();
        *state
            .occurrence_counts_by_scope
            .entry((scope.clone(), signature.to_string()))
            .or_insert(0) += 1;
        state.current_scope = scope;
    }
}

fn append_compact_authoring_subrecord_json_native(
    raw_key: &str,
    compact_payload: &JsonValue,
    record_spec: Option<&SchemaRecordJson>,
    dispatch_state: &mut CompactAuthoringDispatchState,
    localized_label_counts: &mut HashMap<String, usize>,
    subrecords: &mut Vec<ParsedSubrecord>,
    context: &mut NativeImportContext,
    localized: bool,
    record_form_version: Option<u16>,
) -> PyResult<()> {
    let resolved_signature =
        resolve_compact_signature_from_schema(record_spec, raw_key, localized_label_counts)?;
    let spec = record_spec.and_then(|record_spec| {
        select_compact_authoring_subrecord_spec(
            record_spec,
            raw_key,
            resolved_signature.as_str(),
            dispatch_state,
        )
    });
    let expanded = expand_compact_field_payload_from_schema_json(
        resolved_signature.as_str(),
        compact_payload,
        spec,
        record_form_version,
    )?;
    subrecords.push(build_subrecord_from_authoring_field_json_native(
        resolved_signature.as_str(),
        &expanded,
        spec,
        context,
        localized,
    )?);
    advance_compact_authoring_dispatch_state(dispatch_state, resolved_signature.as_str(), spec);
    Ok(())
}

fn append_top_level_eid_subrecord_json_native(
    eid: &str,
    record_spec: Option<&SchemaRecordJson>,
    dispatch_state: &mut CompactAuthoringDispatchState,
    localized_label_counts: &mut HashMap<String, usize>,
    subrecords: &mut Vec<ParsedSubrecord>,
    context: &mut NativeImportContext,
    localized: bool,
) -> PyResult<()> {
    append_compact_authoring_subrecord_json_native(
        "EDID",
        &JsonValue::String(eid.to_string()),
        record_spec,
        dispatch_state,
        localized_label_counts,
        subrecords,
        context,
        localized,
        None,
    )
}

fn append_starfield_component_subrecords_json_native(
    components_value: &JsonValue,
    record_spec: Option<&SchemaRecordJson>,
    dispatch_state: &mut CompactAuthoringDispatchState,
    localized_label_counts: &mut HashMap<String, usize>,
    subrecords: &mut Vec<ParsedSubrecord>,
    context: &mut NativeImportContext,
    localized: bool,
    record_form_version: Option<u16>,
) -> PyResult<()> {
    for (component_index, component_value) in json_array(components_value, "Components")?
        .iter()
        .enumerate()
    {
        let component_mapping =
            json_object(component_value, &format!("Components[{component_index}]"))?;
        let (component_key, component_body_value) = component_mapping
            .iter()
            .find(|(key, _)| key.as_str() != "Type")
            .ok_or_else(|| {
                value_error(format!(
                    "Components[{component_index}] must include a typed component payload"
                ))
            })?;
        let component_body = json_object(
            component_body_value,
            &format!("Components[{component_index}].{component_key}"),
        )?;
        let component_type = component_body
            .get("Type")
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| starfield_component_type_from_authoring_key(component_key));

        append_compact_authoring_subrecord_json_native(
            "BFCB",
            &JsonValue::String(component_type),
            record_spec,
            dispatch_state,
            localized_label_counts,
            subrecords,
            context,
            localized,
            record_form_version,
        )?;

        if let Some(raw_fields) = component_body.get("fields") {
            for (field_index, field_value) in
                json_array(raw_fields, &format!("Components[{component_index}].fields"))?
                    .iter()
                    .enumerate()
            {
                let field_mapping = json_object(
                    field_value,
                    &format!("Components[{component_index}].fields[{field_index}]"),
                )?;
                let Some((raw_key, compact_payload)) = json_compact_authoring_entry(field_mapping)?
                else {
                    return Err(value_error(format!(
                        "Components[{component_index}].fields[{field_index}] must be a compact field entry"
                    )));
                };
                append_compact_authoring_subrecord_json_native(
                    raw_key.as_str(),
                    &compact_payload,
                    record_spec,
                    dispatch_state,
                    localized_label_counts,
                    subrecords,
                    context,
                    localized,
                    record_form_version,
                )?;
            }
        } else {
            for (raw_key, compact_payload) in component_body {
                if raw_key == "Type" {
                    continue;
                }
                append_compact_authoring_subrecord_json_native(
                    raw_key.as_str(),
                    compact_payload,
                    record_spec,
                    dispatch_state,
                    localized_label_counts,
                    subrecords,
                    context,
                    localized,
                    record_form_version,
                )?;
            }
        }

        append_compact_authoring_subrecord_json_native(
            "BFCE",
            &JsonValue::Bool(true),
            record_spec,
            dispatch_state,
            localized_label_counts,
            subrecords,
            context,
            localized,
            record_form_version,
        )?;
    }
    Ok(())
}

fn append_object_template_subrecords_json_native(
    templates_value: &JsonValue,
    record_spec: Option<&SchemaRecordJson>,
    dispatch_state: &mut CompactAuthoringDispatchState,
    label_counts: &mut HashMap<String, usize>,
    subrecords: &mut Vec<ParsedSubrecord>,
    context: &mut NativeImportContext,
    localized: bool,
    record_form_version: Option<u16>,
) -> PyResult<()> {
    let templates = json_array(templates_value, "ObjectTemplates")?;
    append_compact_authoring_subrecord_json_native(
        "OBTE",
        &JsonValue::Number((templates.len() as u64).into()),
        record_spec,
        dispatch_state,
        label_counts,
        subrecords,
        context,
        localized,
        record_form_version,
    )?;

    for (index, template_value) in templates.iter().enumerate() {
        let template = json_object(template_value, &format!("ObjectTemplates[{index}]"))?;
        let editor_only = template
            .get("IsEditorOnly")
            .or_else(|| template.get("Editor Only"))
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        if editor_only {
            append_compact_authoring_subrecord_json_native(
                "OBTF",
                &JsonValue::Bool(true),
                record_spec,
                dispatch_state,
                label_counts,
                subrecords,
                context,
                localized,
                record_form_version,
            )?;
        }
        if let Some(name_value) = template.get("Name") {
            append_compact_authoring_subrecord_json_native(
                "Name",
                name_value,
                record_spec,
                dispatch_state,
                label_counts,
                subrecords,
                context,
                localized,
                record_form_version,
            )?;
        }

        let mut obts = template.clone();
        obts.remove("IsEditorOnly");
        obts.remove("Editor Only");
        obts.remove("Name");
        obts.remove("Marker");
        if !obts.contains_key("Include Count") && !obts.contains_key("include_count") {
            if let Some(include_count) = obts
                .get("Includes")
                .or_else(|| obts.get("includes"))
                .and_then(JsonValue::as_array)
                .map(|includes| includes.len() as u64)
            {
                obts.insert(
                    "Include Count".to_string(),
                    JsonValue::Number(include_count.into()),
                );
            }
        }
        if !obts.contains_key("Property Count") && !obts.contains_key("property_count") {
            if let Some(property_count) = obts
                .get("Properties")
                .or_else(|| obts.get("properties"))
                .and_then(JsonValue::as_array)
                .map(|properties| properties.len() as u64)
            {
                obts.insert(
                    "Property Count".to_string(),
                    JsonValue::Number(property_count.into()),
                );
            }
        }
        append_compact_authoring_subrecord_json_native(
            "OBTS",
            &JsonValue::Object(obts),
            record_spec,
            dispatch_state,
            label_counts,
            subrecords,
            context,
            localized,
            record_form_version,
        )?;
        if template
            .get("Marker")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false)
        {
            append_compact_authoring_subrecord_json_native(
                "STOP",
                &JsonValue::Bool(true),
                record_spec,
                dispatch_state,
                label_counts,
                subrecords,
                context,
                localized,
                record_form_version,
            )?;
        }
    }
    Ok(())
}

fn schema_row_group_key_from_mapping_json(
    field_mapping: &JsonMap<String, JsonValue>,
    record_spec: Option<&SchemaRecordJson>,
) -> Option<(String, String)> {
    let record_spec = record_spec?;
    for spec in &record_spec.subrecords {
        if spec.authoring_layout.as_deref() != Some("row_group") {
            continue;
        }
        let Some(group_key) = spec.authoring_key.as_deref() else {
            continue;
        };
        if group_key == "group_object_template" || group_key == "xedit_group_object_template" {
            continue;
        }
        let display_key = authoring_group_display_key(group_key);
        if field_mapping.contains_key(display_key.as_str()) {
            return Some((group_key.to_string(), display_key));
        }
    }
    None
}

fn append_schema_row_group_subrecords_json_native(
    group_key: &str,
    rows_value: &JsonValue,
    record_spec: Option<&SchemaRecordJson>,
    dispatch_state: &mut CompactAuthoringDispatchState,
    label_counts: &mut HashMap<String, usize>,
    subrecords: &mut Vec<ParsedSubrecord>,
    context: &mut NativeImportContext,
    localized: bool,
    record_form_version: Option<u16>,
) -> PyResult<()> {
    let Some(record_spec) = record_spec else {
        return Err(value_error(format!(
            "{group_key} row group requires schema metadata"
        )));
    };
    let specs: Vec<&SchemaSubrecordJson> = record_spec
        .subrecords
        .iter()
        .filter(|spec| {
            spec.authoring_layout.as_deref() == Some("row_group")
                && spec.authoring_key.as_deref() == Some(group_key)
        })
        .collect();
    for (row_index, row_value) in json_array(rows_value, group_key)?.iter().enumerate() {
        let row = json_object(row_value, &format!("{group_key}[{row_index}]"))?;
        // Every entry in a row belongs to exactly one subrecord. Sibling specs
        // can collapse onto one authoring key when they share a signature and
        // label — RACE splits BodyData into male and female halves that both
        // expose INDX/MODL/MODT — and claiming an entry once per spec doubled
        // the block on every re-encode.
        let mut consumed_keys: HashSet<&str> = HashSet::new();
        for spec in &specs {
            let field_key = schema_subrecord_authoring_key(record_spec, spec);
            let preferred_key = schema_subrecord_key(record_spec.id.as_str(), spec);
            let candidate_keys = [
                field_key.as_str(),
                preferred_key.as_str(),
                schema_subrecord_legacy_key(spec).unwrap_or_default(),
                spec.id.as_str(),
            ];
            let matched = candidate_keys
                .into_iter()
                .find_map(|key| (!key.is_empty()).then(|| row.get_key_value(key)).flatten());
            let Some((matched_key, compact_payload)) = matched else {
                continue;
            };
            if !consumed_keys.insert(matched_key.as_str()) {
                continue;
            }
            append_compact_authoring_subrecord_json_native(
                spec.id.as_str(),
                compact_payload,
                Some(record_spec),
                dispatch_state,
                label_counts,
                subrecords,
                context,
                localized,
                record_form_version,
            )?;
        }
    }
    Ok(())
}

fn parse_record_from_json_compact_native(
    payload: &JsonMap<String, JsonValue>,
    context: &mut NativeImportContext,
) -> PyResult<ParsedRecord> {
    let signature = json_required_string(
        payload
            .get("signature")
            .ok_or_else(|| value_error("missing record signature"))?,
        "record.signature",
    )?;
    if signature.len() != 4 {
        return Err(value_error(format!(
            "invalid record signature: {signature:?}"
        )));
    }
    // Localized fields are filed under a table type chosen from (record, subrecord).
    // Cleared below so a record built through another path falls back to the
    // signature-only rule rather than inheriting this record's type.
    context.current_record_signature = Some(signature.to_string());
    let form_version = match payload.get("form_version") {
        Some(value) if !value.is_null() => {
            Some(json_parse_int_authoring(value, &format!("{signature}.form_version"), 0)? as u16)
        }
        _ => None,
    };
    let version2 = match payload.get("version2") {
        Some(value) if !value.is_null() => {
            Some(json_parse_int_authoring(value, &format!("{signature}.version2"), 0)? as u16)
        }
        _ => None,
    };
    let mut subrecords = Vec::new();
    let top_level_eid = payload
        .get("eid")
        .or_else(|| payload.get("editor_id"))
        .and_then(|value| value.as_str())
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let authored_fields = if let Some(fields) = payload.get("fields") {
        Some(json_array(fields, &format!("{signature}.fields"))?.clone())
    } else if let Some(fields_by_signature) = payload.get("fields_by_signature") {
        let fields_by_signature = json_object(
            fields_by_signature,
            &format!("{signature}.fields_by_signature"),
        )?;
        let mut flattened = Vec::new();
        for (signature_key, raw_payload) in fields_by_signature {
            if let Some(entries) = raw_payload.as_array() {
                for entry in entries {
                    if let Some(mapping) = entry.as_object() {
                        if !mapping.contains_key("signature") {
                            let mut normalized = mapping.clone();
                            normalized.insert(
                                "signature".to_string(),
                                JsonValue::String(signature_key.clone()),
                            );
                            flattened.push(JsonValue::Object(normalized));
                        } else {
                            flattened.push(entry.clone());
                        }
                    } else {
                        let mut normalized = JsonMap::new();
                        normalized.insert(
                            "signature".to_string(),
                            JsonValue::String(signature_key.clone()),
                        );
                        normalized.insert("value".to_string(), entry.clone());
                        flattened.push(JsonValue::Object(normalized));
                    }
                }
            } else if let Some(mapping) = raw_payload.as_object() {
                if !mapping.contains_key("signature") {
                    let mut normalized = mapping.clone();
                    normalized.insert(
                        "signature".to_string(),
                        JsonValue::String(signature_key.clone()),
                    );
                    flattened.push(JsonValue::Object(normalized));
                } else {
                    flattened.push(raw_payload.clone());
                }
            } else {
                let mut normalized = JsonMap::new();
                normalized.insert(
                    "signature".to_string(),
                    JsonValue::String(signature_key.clone()),
                );
                normalized.insert("value".to_string(), raw_payload.clone());
                flattened.push(JsonValue::Object(normalized));
            }
        }
        Some(flattened)
    } else if top_level_eid.is_some() && !payload.contains_key("subrecords") {
        Some(Vec::new())
    } else {
        None
    };

    if let Some(fields) = authored_fields {
        let schema = context.schema.clone();
        let record_spec = schema
            .as_ref()
            .and_then(|schema| schema_record_spec(schema.as_ref(), signature.as_str()));
        let mut dispatch_state = CompactAuthoringDispatchState::default();
        let mut localized_label_counts: HashMap<String, usize> = HashMap::new();
        let localized = (context.header.flags & TES4_FLAG_LOCALIZED) != 0;
        if let Some(eid) = top_level_eid.as_deref() {
            append_top_level_eid_subrecord_json_native(
                eid,
                record_spec,
                &mut dispatch_state,
                &mut localized_label_counts,
                &mut subrecords,
                context,
                localized,
            )?;
        }
        for field in fields {
            let (field_signature, expanded_field) = if let Some(field_mapping) = field.as_object() {
                if is_starfield_game_name(context.game.as_deref()) {
                    if let Some(components_value) = field_mapping.get("Components") {
                        append_starfield_component_subrecords_json_native(
                            components_value,
                            record_spec,
                            &mut dispatch_state,
                            &mut localized_label_counts,
                            &mut subrecords,
                            context,
                            localized,
                            form_version,
                        )?;
                        continue;
                    }
                }
                if let Some(templates_value) = field_mapping.get("ObjectTemplates") {
                    append_object_template_subrecords_json_native(
                        templates_value,
                        record_spec,
                        &mut dispatch_state,
                        &mut localized_label_counts,
                        &mut subrecords,
                        context,
                        localized,
                        form_version,
                    )?;
                    continue;
                }
                if let Some((group_key, display_key)) =
                    schema_row_group_key_from_mapping_json(field_mapping, record_spec)
                {
                    let rows_value = field_mapping.get(display_key.as_str()).ok_or_else(|| {
                        value_error(format!("missing row group payload for {display_key}"))
                    })?;
                    append_schema_row_group_subrecords_json_native(
                        group_key.as_str(),
                        rows_value,
                        record_spec,
                        &mut dispatch_state,
                        &mut localized_label_counts,
                        &mut subrecords,
                        context,
                        localized,
                        form_version,
                    )?;
                    continue;
                }
                if let Some((raw_key, compact_payload)) =
                    json_compact_authoring_entry(field_mapping)?
                {
                    if top_level_eid.is_some() && compact_authoring_key_is_eid(raw_key.as_str()) {
                        continue;
                    }
                    append_compact_authoring_subrecord_json_native(
                        raw_key.as_str(),
                        &compact_payload,
                        record_spec,
                        &mut dispatch_state,
                        &mut localized_label_counts,
                        &mut subrecords,
                        context,
                        localized,
                        form_version,
                    )?;
                    continue;
                } else {
                    (
                        field_mapping
                            .get("signature")
                            .and_then(|value| value.as_str())
                            .unwrap_or(signature.as_str())
                            .to_string(),
                        field_mapping.clone(),
                    )
                }
            } else {
                let mut expanded = JsonMap::new();
                expanded.insert(
                    "signature".to_string(),
                    JsonValue::String(signature.clone()),
                );
                expanded.insert("value".to_string(), field);
                (signature.clone(), expanded)
            };
            if top_level_eid.is_some() && field_signature == "EDID" {
                continue;
            }
            let spec = record_spec.and_then(|record_spec| {
                select_compact_authoring_subrecord_spec(
                    record_spec,
                    field_signature.as_str(),
                    field_signature.as_str(),
                    &dispatch_state,
                )
            });
            subrecords.push(build_subrecord_from_authoring_field_json_native(
                field_signature.as_str(),
                &expanded_field,
                spec,
                context,
                localized,
            )?);
            advance_compact_authoring_dispatch_state(
                &mut dispatch_state,
                field_signature.as_str(),
                spec,
            );
        }
    } else if let Some(raw_subrecords) = payload.get("subrecords") {
        for subrecord in json_array(raw_subrecords, &format!("{signature}.subrecords"))? {
            subrecords.push(parse_subrecord_from_json_native(json_object(
                subrecord,
                &format!("{signature}.subrecords[]"),
            )?)?);
        }
    }

    let record = ParsedRecord {
        signature: SmolStr::new(signature.clone()),
        form_id: match payload.get("form_id") {
            Some(value) => parse_record_form_id_value_native(value, context, "record.form_id")?,
            None => 0,
        },
        flags: match payload.get("flags") {
            Some(value) => json_parse_hex_authoring(value, &format!("{signature}.flags"), 0)?,
            None => 0,
        },
        version_control: match payload.get("version_control") {
            Some(value) => {
                json_parse_int_authoring(value, &format!("{signature}.version_control"), 0)? as u32
            }
            None => 0,
        },
        form_version,
        version2,
        subrecords,
        raw_payload: match payload.get("raw_payload_hex") {
            Some(value) if !value.is_null() => Some(Bytes::from(json_parse_hex_bytes(
                Some(value),
                "record.raw_payload_hex",
            )?)),
            _ => None,
        },
        parse_error: json_optional_string(payload.get("parse_error"))?,
    };
    context.current_record_signature = None;
    Ok(record)
}

fn parse_group_from_json_compact_native(
    payload: &JsonMap<String, JsonValue>,
    context: &mut NativeImportContext,
) -> PyResult<ParsedGroup> {
    let group_type = match payload.get("group_type") {
        Some(value) => json_parse_int_authoring(value, "group.group_type", 0)? as i32,
        None => 0,
    };
    let label_vec = match payload.get("label_hex") {
        Some(value) if !value.is_null() => json_parse_hex_bytes(Some(value), "group.label_hex")?,
        _ => payload
            .get("label_text")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .as_bytes()
            .to_vec(),
    };
    if label_vec.len() > 4 {
        return Err(value_error("group label must be at most 4 bytes"));
    }
    let mut label = [0u8; 4];
    for (index, value) in label_vec.iter().copied().enumerate() {
        label[index] = value;
    }
    let tail = Bytes::from(json_parse_hex_bytes(
        payload.get("tail_hex"),
        "group.tail_hex",
    )?);
    let mut children = Vec::new();
    if let Some(raw_children) = payload.get("children") {
        for child in json_array(raw_children, "group.children")? {
            let child = json_object(child, "group.children[]")?;
            let item_type = child
                .get("type")
                .and_then(|value| value.as_str())
                .unwrap_or("record")
                .to_ascii_lowercase();
            if item_type == "group" {
                children.push(ParsedItem::Group(parse_group_from_json_compact_native(
                    child, context,
                )?));
            } else {
                children.push(ParsedItem::Record(parse_record_from_json_compact_native(
                    child, context,
                )?));
            }
        }
    }
    Ok(ParsedGroup {
        label,
        group_type,
        tail,
        children,
    })
}

fn record_payload_requires_python_import(payload: &JsonMap<String, JsonValue>) -> bool {
    if payload.contains_key("fields") || payload.contains_key("fields_by_signature") {
        return true;
    }
    if payload.contains_key("Landscape")
        || payload.contains_key("TopCell")
        || payload.contains_key("NavigationMeshes")
        || payload.contains_key("Persistent")
        || payload.contains_key("Temporary")
        || payload.contains_key("VisibleWhenDistant")
    {
        return true;
    }
    payload
        .keys()
        .any(|key| AUTHORING_PAYLOAD_MARKERS.contains(&key.as_str()))
        && !payload.contains_key("subrecords")
}

fn parse_record_form_id_value_native(
    value: &JsonValue,
    context: &mut NativeImportContext,
    field: &str,
) -> PyResult<u32> {
    if value.is_null() {
        return Ok(0);
    }
    if let Some(text) = value.as_str() {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Ok(0);
        }
        if let Some((object_id_text, owner_plugin_text)) = trimmed.split_once(':') {
            let owner_plugin_name = owner_plugin_text.trim();
            if owner_plugin_name.is_empty() {
                return Err(value_error(format!(
                    "invalid record form_id for {field}: {text:?}"
                )));
            }
            let object_id = u32::from_str_radix(
                object_id_text
                    .trim()
                    .trim_start_matches("0x")
                    .trim_start_matches("0X"),
                16,
            )
            .map_err(|_| value_error(format!("invalid record form_id for {field}: {text:?}")))?
                & 0x00FF_FFFF;
            if object_id == 0 {
                return Ok(0);
            }
            if owner_plugin_name.eq_ignore_ascii_case(context.plugin_name.as_str()) {
                return Ok((context.own_index() << 24) | object_id);
            }
            let master_index = context.ensure_master_index(owner_plugin_name) as u32;
            return Ok((master_index << 24) | object_id);
        }
        // Hex-only string: text_payload exports as 8-hex `{:08X}` (full 32-bit
        // FormID, high byte = master index). Honor high byte as-is so records
        // overriding master plugins (mod-index 0x00) do not get restamped with
        // the current plugin's own_index.
        let normalized = trimmed.trim_start_matches("0x").trim_start_matches("0X");
        let parsed = u32::from_str_radix(normalized, 16)
            .map_err(|_| value_error(format!("invalid hex integer for {field}: {text:?}")))?;
        // Legacy bare-object_id fallback: if the hex string was <= 6 digits
        // (no master byte), stamp the plugin's own index.
        if normalized.len() <= 6 && parsed > 0 && parsed <= 0x00FF_FFFF {
            return Ok((context.own_index() << 24) | parsed);
        }
        return Ok(parsed);
    }
    // Numeric JSON value: treat as full 32-bit FormID.
    Ok(json_parse_hex(value, field, 0)?)
}

fn parse_subrecord_from_json_native(
    payload: &JsonMap<String, JsonValue>,
) -> PyResult<ParsedSubrecord> {
    let signature = json_required_string(
        payload
            .get("signature")
            .ok_or_else(|| value_error("missing subrecord signature"))?,
        "subrecord.signature",
    )?;
    if signature.len() != 4 {
        return Err(value_error(format!(
            "invalid subrecord signature: {signature:?}"
        )));
    }
    let data = json_parse_hex_bytes(payload.get("data_hex"), &format!("{signature}.data_hex"))?;
    Ok(ParsedSubrecord {
        signature: SmolStr::new(signature),
        data: Bytes::from(data),
        semantic_type: json_optional_string(payload.get("semantic_type"))?,
    })
}

fn parse_record_from_json_lossless_native(
    payload: &JsonMap<String, JsonValue>,
    context: &mut NativeImportContext,
) -> PyResult<Option<ParsedRecord>> {
    if record_payload_requires_python_import(payload) {
        return Ok(None);
    }
    let signature = json_required_string(
        payload
            .get("signature")
            .ok_or_else(|| value_error("missing record signature"))?,
        "record.signature",
    )?;
    if signature.len() != 4 {
        return Err(value_error(format!(
            "invalid record signature: {signature:?}"
        )));
    }
    let subrecords_value = match payload.get("subrecords") {
        Some(value) => value,
        None => return Ok(None),
    };
    let mut subrecords = Vec::new();
    for value in json_array(subrecords_value, &format!("{signature}.subrecords"))? {
        subrecords.push(parse_subrecord_from_json_native(json_object(
            value,
            &format!("{signature}.subrecords[]"),
        )?)?);
    }
    Ok(Some(ParsedRecord {
        signature: SmolStr::new(signature),
        form_id: match payload.get("form_id") {
            Some(value) => parse_record_form_id_value_native(value, context, "record.form_id")?,
            None => 0,
        },
        flags: match payload.get("flags") {
            Some(value) => json_parse_hex(value, "record.flags", 0)?,
            None => 0,
        },
        version_control: match payload.get("version_control") {
            Some(value) => json_parse_int(value, "record.version_control", 0)? as u32,
            None => 0,
        },
        form_version: match payload.get("form_version") {
            Some(value) if !value.is_null() => {
                Some(json_parse_int(value, "record.form_version", 0)? as u16)
            }
            _ => None,
        },
        version2: match payload.get("version2") {
            Some(value) if !value.is_null() => {
                Some(json_parse_int(value, "record.version2", 0)? as u16)
            }
            _ => None,
        },
        subrecords,
        raw_payload: match payload.get("raw_payload_hex") {
            Some(value) if !value.is_null() => Some(Bytes::from(json_parse_hex_bytes(
                Some(value),
                "record.raw_payload_hex",
            )?)),
            _ => None,
        },
        parse_error: json_optional_string(payload.get("parse_error"))?,
    }))
}

fn parse_group_from_json_lossless_native(
    payload: &JsonMap<String, JsonValue>,
    context: &mut NativeImportContext,
) -> PyResult<Option<ParsedGroup>> {
    let group_type = match payload.get("group_type") {
        Some(value) => json_parse_int(value, "group.group_type", 0)? as i32,
        None => 0,
    };
    let label_vec = match payload.get("label_hex") {
        Some(value) if !value.is_null() => json_parse_hex_bytes(Some(value), "group.label_hex")?,
        _ => payload
            .get("label_text")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .as_bytes()
            .to_vec(),
    };
    if label_vec.len() > 4 {
        return Ok(None);
    }
    let mut label = [0u8; 4];
    for (index, value) in label_vec.iter().copied().enumerate() {
        label[index] = value;
    }
    let mut children = Vec::new();
    if let Some(raw_children) = payload.get("children") {
        for child in json_array(raw_children, "group.children")? {
            let child = json_object(child, "group.children[]")?;
            let item_type = child
                .get("type")
                .and_then(|value| value.as_str())
                .unwrap_or("record")
                .to_ascii_lowercase();
            if item_type == "group" {
                let Some(group) = parse_group_from_json_lossless_native(child, context)? else {
                    return Ok(None);
                };
                children.push(ParsedItem::Group(group));
            } else {
                let Some(record) = parse_record_from_json_lossless_native(child, context)? else {
                    return Ok(None);
                };
                children.push(ParsedItem::Record(record));
            }
        }
    }
    Ok(Some(ParsedGroup {
        label,
        group_type,
        tail: Bytes::from(json_parse_hex_bytes(
            payload.get("tail_hex"),
            "group.tail_hex",
        )?),
        children,
    }))
}

fn parse_plugin_header_from_json_native(
    payload: &JsonMap<String, JsonValue>,
) -> PyResult<ParsedPluginHeader> {
    let version = match payload.get("version") {
        Some(value) => json_parse_float(value, "header.version", 1.0)?,
        None => 1.0,
    };
    let num_records = match payload.get("num_records") {
        Some(value) => json_parse_int(value, "header.num_records", 0)? as u32,
        None => 0,
    };
    let next_object_id = match payload.get("next_object_id") {
        Some(value) => json_parse_hex(value, "header.next_object_id", 0x0800)?,
        None => 0x0800,
    };
    let author = payload
        .get("author")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string();
    let description = payload
        .get("description")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string();
    let masters = match payload.get("masters") {
        Some(value) => json_array(value, "header.masters")?
            .iter()
            .map(|item| json_required_string(item, "header.masters[]"))
            .collect::<PyResult<Vec<_>>>()?,
        None => Vec::new(),
    };
    let master_sizes = match payload.get("master_sizes") {
        Some(value) => json_array(value, "header.master_sizes")?
            .iter()
            .map(|item| Ok(json_parse_int(item, "header.master_sizes[]", 0)? as u64))
            .collect::<PyResult<Vec<_>>>()?,
        None => Vec::new(),
    };
    let overridden_forms = match payload.get("overridden_forms") {
        Some(value) => json_array(value, "header.overridden_forms")?
            .iter()
            .map(|item| json_parse_hex(item, "header.overridden_forms[]", 0))
            .collect::<PyResult<Vec<_>>>()?,
        None => Vec::new(),
    };
    let extra_subrecords = match payload.get("extra_subrecords") {
        Some(value) => json_array(value, "header.extra_subrecords")?
            .iter()
            .map(|item| {
                parse_subrecord_from_json_native(json_object(item, "header.extra_subrecords[]")?)
            })
            .collect::<PyResult<Vec<_>>>()?,
        None => Vec::new(),
    };
    Ok(ParsedPluginHeader {
        version,
        num_records,
        next_object_id,
        author,
        description,
        masters,
        master_sizes,
        overridden_forms,
        flags: json_parse_header_flags(payload.get("flags"))?,
        extra_subrecords,
        version_control: match payload.get("version_control") {
            Some(value) => json_parse_int(value, "header.version_control", 0)? as u32,
            None => 0,
        },
        form_version: match payload.get("form_version") {
            Some(value) if !value.is_null() => {
                Some(json_parse_int(value, "header.form_version", 0)? as u16)
            }
            _ => None,
        },
        version2: match payload.get("version2") {
            Some(value) if !value.is_null() => {
                Some(json_parse_int(value, "header.version2", 0)? as u16)
            }
            _ => None,
        },
        // Reconstruct the raw 12-byte HEDR payload from the three scalars.
        // Needed so Plugin.to_bytes() preserves Bethesda's stored num_records
        // (often inflated past the real record count) instead of recomputing
        // from len(records). Shape: f32 LE version + u32 LE num_records + u32
        // LE next_object_id, matching TES4 HEDR for all games.
        hedr_raw: Some({
            let mut buf = Vec::with_capacity(12);
            buf.extend_from_slice(&(version as f32).to_le_bytes());
            buf.extend_from_slice(&num_records.to_le_bytes());
            buf.extend_from_slice(&next_object_id.to_le_bytes());
            Bytes::from(buf)
        }),
        raw_subrecords: Vec::new(),
    })
}

fn filename_safe(text: Option<String>, fallback: &str) -> String {
    let mut value = text.unwrap_or_default().trim().to_string();
    if value.is_empty() {
        return fallback.to_string();
    }
    value = value
        .chars()
        .map(|ch| match ch {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            _ => ch,
        })
        .collect::<String>();
    let collapsed = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        fallback.to_string()
    } else {
        collapsed
    }
}

fn format_object_id(raw_form_id: u32) -> String {
    format!("{:06X}", raw_form_id & 0x00FF_FFFF)
}

fn unique_child_name(base_name: &str, used_names: &mut HashSet<String>) -> String {
    if used_names.insert(base_name.to_string()) {
        return base_name.to_string();
    }
    let mut index = 2usize;
    loop {
        let candidate = format!("{base_name}__{index:04}");
        if used_names.insert(candidate.clone()) {
            return candidate;
        }
        index += 1;
    }
}

fn write_text_file(path: &Path, text: &str) -> PyResult<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|err| {
                io_error(format!(
                    "failed to create directory '{}': {err}",
                    parent.display()
                ))
            })?;
        }
    }
    fs::write(path, text)
        .map_err(|err| io_error(format!("failed to write '{}': {err}", path.display())))?;
    Ok(())
}

fn is_record_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| RECORD_FILE_SUFFIXES.contains(&ext.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

fn sorted_directory_entries(path: &Path) -> PyResult<Vec<PathBuf>> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(path).map_err(|err| {
        io_error(format!(
            "failed to read directory '{}': {err}",
            path.display()
        ))
    })? {
        let entry = entry.map_err(|err| {
            io_error(format!(
                "failed to enumerate directory '{}': {err}",
                path.display()
            ))
        })?;
        entries.push(entry.path());
    }
    entries.sort_by(|left, right| {
        let left_is_file = left.is_file();
        let right_is_file = right.is_file();
        (!left_is_file)
            .cmp(&(!right_is_file))
            .then_with(|| {
                left.file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or_default()
                    .to_ascii_lowercase()
                    .cmp(
                        &right
                            .file_name()
                            .and_then(|value| value.to_str())
                            .unwrap_or_default()
                            .to_ascii_lowercase(),
                    )
            })
            .then_with(|| left.file_name().cmp(&right.file_name()))
    });
    Ok(entries)
}

fn parse_group_dir_name(name: &str, is_root: bool) -> Option<(i32, Vec<u8>)> {
    if is_root && name.len() == 4 {
        return Some((0, name.as_bytes().to_vec()));
    }
    let marker_index = name.find(GROUP_DIR_PREFIX)?;
    let raw = &name[marker_index + GROUP_DIR_PREFIX.len()..];
    let (group_type_text, rest) = raw.split_once("__")?;
    let group_type = group_type_text.parse::<i32>().ok()?;
    let label_hex = rest
        .split_once("__")
        .map(|(label, _)| label)
        .unwrap_or(rest);
    let label = hex::decode(label_hex).ok()?;
    Some((group_type, label))
}

fn record_data_filename(format: &str) -> &'static str {
    if format == "yaml" {
        "RecordData.yaml"
    } else {
        "RecordData.json"
    }
}

fn group_record_data_filename(format: &str) -> &'static str {
    if format == "yaml" {
        "GroupRecordData.yaml"
    } else {
        "GroupRecordData.json"
    }
}

fn group_type_name(group_type: i32) -> Option<&'static str> {
    match group_type {
        INTERIOR_CELL_BLOCK => Some("InteriorCellBlock"),
        INTERIOR_CELL_SUBBLOCK => Some("InteriorCellSubBlock"),
        EXTERIOR_CELL_BLOCK => Some("ExteriorCellBlock"),
        EXTERIOR_CELL_SUBBLOCK => Some("ExteriorCellSubBlock"),
        _ => None,
    }
}

fn parse_group_type_name(name: &str) -> Option<i32> {
    match name.trim() {
        "InteriorCellBlock" => Some(INTERIOR_CELL_BLOCK),
        "InteriorCellSubBlock" => Some(INTERIOR_CELL_SUBBLOCK),
        "ExteriorCellBlock" => Some(EXTERIOR_CELL_BLOCK),
        "ExteriorCellSubBlock" => Some(EXTERIOR_CELL_SUBBLOCK),
        _ => None,
    }
}

fn decode_group_grid(label: &[u8]) -> Option<(i16, i16)> {
    if label.len() < 4 {
        return None;
    }
    Some((
        i16::from_le_bytes([label[0], label[1]]),
        i16::from_le_bytes([label[2], label[3]]),
    ))
}

fn decode_group_index(label: &[u8]) -> Option<u32> {
    if label.len() < 4 {
        return None;
    }
    Some(u32::from_le_bytes([label[0], label[1], label[2], label[3]]))
}

fn parse_grid_dir_name(name: &str) -> Option<(i16, i16)> {
    let (left, right) = name.split_once(',')?;
    let left = left.trim();
    let right = right.trim();
    Some((left.parse::<i16>().ok()?, right.parse::<i16>().ok()?))
}

fn encode_exterior_grid_label(x: i16, y: i16) -> [u8; 4] {
    [
        y.to_le_bytes()[0],
        y.to_le_bytes()[1],
        x.to_le_bytes()[0],
        x.to_le_bytes()[1],
    ]
}

fn parse_index_dir_name(name: &str) -> Option<u32> {
    name.parse::<i64>()
        .ok()
        .and_then(|value| u32::try_from(value).ok())
}

fn find_named_payload_file(directory: &Path, stem: &str) -> Option<PathBuf> {
    for extension in ["json", "yaml", "yml"] {
        let candidate = directory.join(format!("{stem}.{extension}"));
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn mixed_layout_error(directory: &Path) -> PyErr {
    value_error(format!(
        "Mixed legacy/new authoring layout under {}",
        directory.display()
    ))
}

fn detect_special_layout(directory: &Path, signature: &str) -> PyResult<&'static str> {
    let mut has_legacy = false;
    let mut has_projected = false;
    for entry in sorted_directory_entries(directory)? {
        if entry.is_file() {
            let entry_name = entry
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or_default();
            if entry_name.starts_with(GROUP_RECORD_DATA_STEM) {
                has_projected = true;
                continue;
            }
            if is_record_file(entry.as_path()) {
                has_legacy = true;
            }
            continue;
        }
        let entry_name = entry
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| value_error(format!("invalid directory name '{}'", entry.display())))?;
        if parse_group_dir_name(entry_name, false).is_some() {
            has_legacy = true;
            continue;
        }
        match signature {
            "WRLD" => {
                if find_named_payload_file(entry.as_path(), RECORD_DATA_STEM).is_some() {
                    has_projected = true;
                } else {
                    has_legacy = true;
                }
            }
            "CELL" => {
                if parse_index_dir_name(entry_name).is_some()
                    || find_named_payload_file(entry.as_path(), GROUP_RECORD_DATA_STEM).is_some()
                {
                    has_projected = true;
                } else {
                    has_legacy = true;
                }
            }
            _ => {}
        }
    }
    if has_legacy && has_projected {
        return Err(mixed_layout_error(directory));
    }
    Ok(if has_projected { "projected" } else { "legacy" })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_context_no_py_preserves_game_and_master_order_and_rejects_closed_handle() {
        let handle = plugin_handle_new_no_py("Source.esp", Some("fnv"));
        plugin_handle_add_master_no_py(handle, "FalloutNV.esm", None).expect("first master");
        plugin_handle_add_master_no_py(handle, "DeadMoney.esm", None).expect("second master");

        assert_eq!(
            plugin_handle_master_names_no_py(handle).expect("master names"),
            ["FalloutNV.esm", "DeadMoney.esm"]
        );
        assert_eq!(
            plugin_handle_game_no_py(handle).expect("game").as_deref(),
            Some("fnv")
        );
        assert!(plugin_handle_close_native(handle));
        assert!(plugin_handle_master_names_no_py(handle).is_err());
        assert!(plugin_handle_game_no_py(handle).is_err());
    }

    fn make_record(signature: &str, form_id: u32, editor_id: Option<&str>) -> ParsedRecord {
        let mut subrecords = Vec::new();
        if let Some(eid) = editor_id {
            let mut data = eid.as_bytes().to_vec();
            data.push(0);
            subrecords.push(ParsedSubrecord {
                signature: SmolStr::new_static("EDID"),
                data: Bytes::from(data),
                semantic_type: None,
            });
        }
        ParsedRecord {
            signature: SmolStr::new(signature),
            form_id,
            flags: 0,
            version_control: 0,
            form_version: None,
            version2: None,
            subrecords,
            raw_payload: None,
            parse_error: None,
        }
    }

    fn formid_subrecord(signature: &str, form_id: u32) -> ParsedSubrecord {
        ParsedSubrecord {
            signature: SmolStr::new(signature),
            data: Bytes::from(form_id.to_le_bytes().to_vec()),
            semantic_type: Some("formid".to_string()),
        }
    }

    #[test]
    fn remove_formid_subrecords_is_exact_and_type_scoped() {
        let mut terminal = make_record(
            "TERM",
            0x0784_F1B1,
            Some("ATX_Collectrons_Terminal_Liberator"),
        );
        terminal.raw_payload = Some(Bytes::from_static(b"original"));
        terminal.subrecords.extend([
            formid_subrecord("SNAM", 0),
            formid_subrecord("SNAM", 0x0009_80FB),
            ParsedSubrecord {
                signature: SmolStr::new_static("SNAM"),
                data: Bytes::from(vec![0; 24]),
                semantic_type: None,
            },
        ]);
        let mut other = make_record("CONT", 0x0100_0001, Some("UnrelatedContainer"));
        other.subrecords.push(formid_subrecord("SNAM", 0x0009_80FB));
        let mut items = vec![ParsedItem::Record(terminal), ParsedItem::Record(other)];

        let mut dry_run_changes = Vec::new();
        remove_formid_subrecords_in_items(
            &mut items,
            "TERM",
            "SNAM",
            0x0009_80FB,
            true,
            &mut dry_run_changes,
        );
        assert_eq!(
            dry_run_changes,
            vec![(
                0x0784_F1B1,
                Some("ATX_Collectrons_Terminal_Liberator".to_string()),
                1
            )]
        );
        let ParsedItem::Record(terminal) = &items[0] else {
            panic!("expected terminal record");
        };
        assert_eq!(terminal.subrecords.len(), 4);
        assert!(terminal.raw_payload.is_some());

        let mut changes = Vec::new();
        remove_formid_subrecords_in_items(
            &mut items,
            "TERM",
            "SNAM",
            0x0009_80FB,
            false,
            &mut changes,
        );
        assert_eq!(changes, dry_run_changes);
        let ParsedItem::Record(terminal) = &items[0] else {
            panic!("expected terminal record");
        };
        let remaining_snam: Vec<&[u8]> = terminal
            .subrecords
            .iter()
            .filter(|subrecord| subrecord.signature.as_str() == "SNAM")
            .map(|subrecord| subrecord.data.as_ref())
            .collect();
        assert_eq!(remaining_snam.len(), 2);
        assert_eq!(remaining_snam[0], 0_u32.to_le_bytes());
        assert_eq!(remaining_snam[1].len(), 24);
        assert!(terminal.raw_payload.is_none());
        let ParsedItem::Record(other) = &items[1] else {
            panic!("expected non-terminal record");
        };
        assert_eq!(
            other.subrecords.last().unwrap().data.as_ref(),
            0x0009_80FB_u32.to_le_bytes()
        );

        let mut null_changes = Vec::new();
        remove_formid_subrecords_in_items(&mut items, "TERM", "SNAM", 0, false, &mut null_changes);
        assert_eq!(null_changes.len(), 1);
    }

    #[test]
    fn repair_term_marker_parameters_restores_source_row_and_preserves_sound() {
        let mut marker_parameters = Vec::new();
        marker_parameters.extend_from_slice(&1.0_f32.to_le_bytes());
        marker_parameters.extend_from_slice(&(-59.0_f32).to_le_bytes());
        marker_parameters.extend_from_slice(&1.0_f32.to_le_bytes());
        marker_parameters.extend_from_slice(&0.0_f32.to_le_bytes());
        marker_parameters.extend_from_slice(&0_u32.to_le_bytes());
        marker_parameters.extend_from_slice(&[0xFF, 1, 0, 0]);

        let mut source = make_record(
            "TERM",
            0x0072_6E6C,
            Some("Storm_UpperAtrium_ClinicTerminal"),
        );
        source.subrecords.push(ParsedSubrecord {
            signature: SmolStr::new_static("ZNAM"),
            data: Bytes::from(marker_parameters.clone()),
            semantic_type: None,
        });
        let mut markers_by_object_id = HashMap::new();
        collect_owned_term_marker_parameters(
            &[ParsedItem::Record(source)],
            0,
            &mut markers_by_object_id,
        );

        let mut target = make_record(
            "TERM",
            0x0772_6E6C,
            Some("Storm_UpperAtrium_ClinicTerminal"),
        );
        target.raw_payload = Some(Bytes::from_static(b"original"));
        target.subrecords.extend([
            formid_subrecord("SNAM", 0x0009_80FB),
            ParsedSubrecord {
                signature: SmolStr::new_static("XMRK"),
                data: Bytes::from_static(b"Markers\\MarkerDeskTerminal3rdP.nif\0"),
                semantic_type: None,
            },
            formid_subrecord("SNAM", 0x0780_0000),
            ParsedSubrecord {
                signature: SmolStr::new_static("BSIZ"),
                data: Bytes::from(1_u32.to_le_bytes().to_vec()),
                semantic_type: None,
            },
        ]);
        let mut items = vec![ParsedItem::Record(target)];

        let mut dry_run_changes = Vec::new();
        repair_term_marker_parameters_in_items(
            &mut items,
            7,
            &markers_by_object_id,
            true,
            &mut dry_run_changes,
        );
        assert_eq!(
            dry_run_changes,
            vec![(
                0x0772_6E6C,
                Some("Storm_UpperAtrium_ClinicTerminal".to_string()),
                1,
                1
            )]
        );
        let ParsedItem::Record(target) = &items[0] else {
            panic!("expected terminal record");
        };
        assert!(target.subrecords.iter().any(|subrecord| {
            subrecord.signature.as_str() == "SNAM"
                && subrecord.data.as_ref() == 0x0780_0000_u32.to_le_bytes()
        }));

        let mut changes = Vec::new();
        repair_term_marker_parameters_in_items(
            &mut items,
            7,
            &markers_by_object_id,
            false,
            &mut changes,
        );
        assert_eq!(changes, dry_run_changes);
        let ParsedItem::Record(target) = &items[0] else {
            panic!("expected terminal record");
        };
        let xmrk = target
            .subrecords
            .iter()
            .position(|subrecord| subrecord.signature.as_str() == "XMRK")
            .unwrap();
        let snam = target
            .subrecords
            .iter()
            .enumerate()
            .filter(|(_, subrecord)| subrecord.signature.as_str() == "SNAM")
            .collect::<Vec<_>>();
        assert_eq!(snam.len(), 2);
        assert!(snam[0].0 < xmrk);
        assert_eq!(snam[0].1.data.as_ref(), 0x0009_80FB_u32.to_le_bytes());
        assert_eq!(snam[1].0, xmrk + 1);
        assert_eq!(snam[1].1.data.as_ref(), marker_parameters.as_slice());
        assert!(target.raw_payload.is_none());
    }

    fn land_layer_subrecord(signature: &str, texture_form_id: u32) -> ParsedSubrecord {
        let mut data = Vec::new();
        data.extend_from_slice(&texture_form_id.to_le_bytes());
        data.extend_from_slice(&[0, 0, 0, 0]);
        ParsedSubrecord {
            signature: SmolStr::new(signature),
            data: Bytes::from(data),
            semantic_type: None,
        }
    }

    fn empty_header() -> ParsedPluginHeader {
        ParsedPluginHeader {
            version: 1.0,
            num_records: 0,
            next_object_id: 0,
            author: String::new(),
            description: String::new(),
            masters: Vec::new(),
            master_sizes: Vec::new(),
            overridden_forms: Vec::new(),
            flags: 0,
            extra_subrecords: Vec::new(),
            version_control: 0,
            form_version: None,
            version2: None,
            hedr_raw: None,
            raw_subrecords: Vec::new(),
        }
    }

    fn empty_plugin(game: Option<&str>) -> ParsedPlugin {
        ParsedPlugin {
            plugin_name: "Test.esp".to_string(),
            file_path: String::new(),
            header_size: MODERN_HEADER_SIZE,
            header: empty_header(),
            root_items: Vec::new(),
            game: game.map(str::to_string),
        }
    }

    fn create_empty_plugin_handle(plugin_name: &str, game: Option<&str>) -> u64 {
        let mut plugin = empty_plugin(game);
        plugin.plugin_name = plugin_name.to_string();
        insert_plugin_handle(plugin, LocalizedStringsState::default())
    }

    #[test]
    fn authoring_record_batch_matches_sequential_replacement() {
        let sequential_handle = create_empty_plugin_handle("Test.esp", Some("fo4"));
        let batch_handle = create_empty_plugin_handle("Test.esp", Some("fo4"));
        let values = vec![
            serde_json::json!({
                "signature": "WRLD",
                "form_id": "000800:Test.esp",
                "eid": "TestWorld",
                "subrecords": [
                    { "signature": "EDID", "data_hex": "54657374576F726C6400" }
                ]
            }),
            serde_json::json!({
                "signature": "LTEX",
                "form_id": "000801:Test.esp",
                "eid": "TestLandTexture",
                "subrecords": [
                    { "signature": "EDID", "data_hex": "546573744C616E645465787475726500" }
                ]
            }),
        ];

        let sequential_form_keys = values
            .iter()
            .map(|value| {
                plugin_handle_replace_authoring_record_value(sequential_handle, value)
                    .expect("sequential replacement")
            })
            .collect::<Vec<_>>();
        let batch_form_keys = plugin_handle_replace_authoring_record_values(batch_handle, &values)
            .expect("batch replacement");

        assert_eq!(batch_form_keys, sequential_form_keys);
        let store = plugin_handle_store_ref().lock().unwrap();
        let mut sequential_fingerprint = String::new();
        tree_fingerprint(
            &store.get(&sequential_handle).unwrap().parsed.root_items,
            0,
            &mut sequential_fingerprint,
        );
        let mut batch_fingerprint = String::new();
        tree_fingerprint(
            &store.get(&batch_handle).unwrap().parsed.root_items,
            0,
            &mut batch_fingerprint,
        );
        assert_eq!(batch_fingerprint, sequential_fingerprint);
    }

    fn populate_all_sections(handle_id: u64) {
        let mut store = plugin_handle_store_ref().lock().unwrap();
        let slot = store.get_mut(&handle_id).expect("plugin handle present");
        let _ = ensure_locator_section(slot);
        let _ = ensure_core_section(slot);
        let _ = ensure_records_section(slot);
        let _ = ensure_form_id_paths_section(slot);
        let _ = ensure_refs_section(slot);
        let _ = ensure_assets_section(slot);
    }

    fn temp_plugin_path(test_name: &str) -> PathBuf {
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "modkit21-{test_name}-{}-{suffix}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir.join("SeventySix.esm")
    }

    fn localized_subrecord(signature: &str, string_id: u32) -> ParsedSubrecord {
        ParsedSubrecord {
            signature: SmolStr::new(signature),
            data: Bytes::from(string_id.to_le_bytes().to_vec()),
            semantic_type: None,
        }
    }

    fn data_subrecord(values: [f32; 6]) -> ParsedSubrecord {
        let mut data = Vec::new();
        for value in values {
            data.extend_from_slice(&value.to_le_bytes());
        }
        ParsedSubrecord {
            signature: SmolStr::new_static("DATA"),
            data: Bytes::from(data),
            semantic_type: None,
        }
    }

    fn exterior_navmesh_record(form_id: u32, world_form_id: u32, cell: (i16, i16)) -> ParsedRecord {
        let mut record = make_record("NAVM", form_id, None);
        let mut nvnm = vec![0u8; 16];
        nvnm[8..12].copy_from_slice(&world_form_id.to_le_bytes());
        nvnm[12..14].copy_from_slice(&cell.1.to_le_bytes());
        nvnm[14..16].copy_from_slice(&cell.0.to_le_bytes());
        record.subrecords.push(ParsedSubrecord {
            signature: SmolStr::new_static("NVNM"),
            data: Bytes::from(nvnm),
            semantic_type: None,
        });
        record
    }

    fn interior_navmesh_record(form_id: u32, cell_form_id: u32) -> ParsedRecord {
        let mut record = make_record("NAVM", form_id, None);
        let mut nvnm = vec![0u8; 16];
        nvnm[12..16].copy_from_slice(&cell_form_id.to_le_bytes());
        record.subrecords.push(ParsedSubrecord {
            signature: SmolStr::new_static("NVNM"),
            data: Bytes::from(nvnm),
            semantic_type: None,
        });
        record
    }

    fn exterior_navmesh_record_with_edges(
        form_id: u32,
        world_form_id: u32,
        cell: (i16, i16),
        edge_links: &[u32],
    ) -> ParsedRecord {
        exterior_navmesh_record_with_geometry(
            form_id,
            world_form_id,
            cell,
            &[
                (8.0f32, 18.0f32, 28.0f32),
                (10.0, 20.0, 30.0),
                (12.0, 22.0, 32.0),
            ],
            &[[0, 1, 2]],
            edge_links,
        )
    }

    fn exterior_navmesh_record_with_geometry(
        form_id: u32,
        world_form_id: u32,
        cell: (i16, i16),
        vertices: &[(f32, f32, f32)],
        triangles: &[[u16; 3]],
        edge_links: &[u32],
    ) -> ParsedRecord {
        let mut record = make_record("NAVM", form_id, None);
        let mut nvnm = Vec::new();
        nvnm.extend_from_slice(&15u32.to_le_bytes());
        nvnm.extend_from_slice(&0xAABB_CCDDu32.to_le_bytes());
        nvnm.extend_from_slice(&world_form_id.to_le_bytes());
        nvnm.extend_from_slice(&cell.1.to_le_bytes());
        nvnm.extend_from_slice(&cell.0.to_le_bytes());
        nvnm.extend_from_slice(&(vertices.len() as u32).to_le_bytes());
        for vertex in vertices {
            nvnm.extend_from_slice(&vertex.0.to_le_bytes());
            nvnm.extend_from_slice(&vertex.1.to_le_bytes());
            nvnm.extend_from_slice(&vertex.2.to_le_bytes());
        }
        nvnm.extend_from_slice(&(triangles.len() as u32).to_le_bytes());
        for triangle in triangles {
            for vertex_index in triangle {
                nvnm.extend_from_slice(&vertex_index.to_le_bytes());
            }
            for _ in 0..3 {
                nvnm.extend_from_slice(&(-1i16).to_le_bytes());
            }
            nvnm.extend_from_slice(&0.0f32.to_le_bytes());
            nvnm.push(0);
            nvnm.extend_from_slice(&0u16.to_le_bytes());
            nvnm.extend_from_slice(&0u16.to_le_bytes());
        }
        nvnm.extend_from_slice(&(edge_links.len() as u32).to_le_bytes());
        for linked_navmesh in edge_links {
            nvnm.extend_from_slice(&0u32.to_le_bytes());
            nvnm.extend_from_slice(&linked_navmesh.to_le_bytes());
            nvnm.extend_from_slice(&0i16.to_le_bytes());
            nvnm.push(0);
        }
        for _ in 0..5 {
            nvnm.extend_from_slice(&0u32.to_le_bytes());
        }
        record.subrecords.push(ParsedSubrecord {
            signature: SmolStr::new_static("NVNM"),
            data: Bytes::from(nvnm),
            semantic_type: None,
        });
        record
    }

    fn first_top_level_record<'a>(
        items: &'a [ParsedItem],
        signature: &str,
    ) -> Option<&'a ParsedRecord> {
        let sig_bytes: [u8; 4] = signature.as_bytes().try_into().ok()?;
        items.iter().find_map(|item| match item {
            ParsedItem::Group(group) if group.group_type == 0 && group.label == sig_bytes => {
                group.children.iter().find_map(|child| match child {
                    ParsedItem::Record(record) if record.signature.as_str() == signature => {
                        Some(record)
                    }
                    _ => None,
                })
            }
            _ => None,
        })
    }

    fn count_test_records_by_signature(items: &[ParsedItem], signature: &str) -> usize {
        items
            .iter()
            .map(|item| match item {
                ParsedItem::Record(record) if record.signature.as_str() == signature => 1,
                ParsedItem::Group(group) => {
                    count_test_records_by_signature(&group.children, signature)
                }
                _ => 0,
            })
            .sum()
    }

    fn navmesh_info_subrecord<'a>(record: &'a ParsedRecord, navmesh_form_id: u32) -> &'a [u8] {
        record
            .subrecords
            .iter()
            .find(|subrecord| {
                subrecord.signature.as_str() == "NVMI"
                    && subrecord.data.len() >= 4
                    && u32::from_le_bytes(subrecord.data[0..4].try_into().unwrap())
                        == navmesh_form_id
            })
            .map(|subrecord| subrecord.data.as_ref())
            .expect("NVMI for navmesh")
    }

    fn source_navi_with_nvmi(form_id: u32, nvmi: Vec<u8>) -> ParsedRecord {
        ParsedRecord {
            signature: SmolStr::new_static("NAVI"),
            form_id,
            flags: 0,
            version_control: 0,
            form_version: None,
            version2: None,
            subrecords: vec![
                ParsedSubrecord {
                    signature: SmolStr::new_static("NVER"),
                    data: Bytes::from(15u32.to_le_bytes().to_vec()),
                    semantic_type: None,
                },
                ParsedSubrecord {
                    signature: SmolStr::new_static("NVMI"),
                    data: Bytes::from(nvmi),
                    semantic_type: None,
                },
                ParsedSubrecord {
                    signature: SmolStr::new_static("NVPP"),
                    data: Bytes::from(vec![0u8; 8]),
                    semantic_type: None,
                },
            ],
            raw_payload: None,
            parse_error: None,
        }
    }

    fn source_nvmi_with_island(
        navmesh_form_id: u32,
        parent_world_form_id: u32,
        edge_links: &[u32],
    ) -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(&navmesh_form_id.to_le_bytes());
        data.extend_from_slice(&NAVI_NVMI_FLAG_IS_ISLAND.to_le_bytes());
        for value in [10.0f32, 20.0, 30.0, 0.0] {
            data.extend_from_slice(&value.to_le_bytes());
        }
        data.extend_from_slice(&(edge_links.len() as u32).to_le_bytes());
        for link in edge_links {
            data.extend_from_slice(&link.to_le_bytes());
        }
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.push(1);
        for value in [8.0f32, 18.0, 28.0, 12.0, 22.0, 32.0] {
            data.extend_from_slice(&value.to_le_bytes());
        }
        data.extend_from_slice(&1u32.to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes());
        data.extend_from_slice(&1u16.to_le_bytes());
        data.extend_from_slice(&2u16.to_le_bytes());
        data.extend_from_slice(&3u32.to_le_bytes());
        for vertex in [
            (8.0f32, 18.0f32, 28.0f32),
            (10.0, 20.0, 30.0),
            (12.0, 22.0, 32.0),
        ] {
            data.extend_from_slice(&vertex.0.to_le_bytes());
            data.extend_from_slice(&vertex.1.to_le_bytes());
            data.extend_from_slice(&vertex.2.to_le_bytes());
        }
        data.extend_from_slice(&0xAABB_CCDDu32.to_le_bytes());
        data.extend_from_slice(&parent_world_form_id.to_le_bytes());
        data.extend_from_slice(&(-2i16).to_le_bytes());
        data.extend_from_slice(&3i16.to_le_bytes());
        data
    }

    /// Source NVMI (no island) with a door-link section. Each door link is
    /// (crc_hash, source_ref). Layout matches remap_source_nvmi_for_projected_navi.
    fn source_nvmi_with_doors(
        navmesh_form_id: u32,
        parent_world_form_id: u32,
        edge_links: &[u32],
        door_links: &[(u32, u32)],
    ) -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(&navmesh_form_id.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes()); // flags (no island)
        for value in [10.0f32, 20.0, 30.0, 0.0] {
            data.extend_from_slice(&value.to_le_bytes());
        }
        data.extend_from_slice(&(edge_links.len() as u32).to_le_bytes());
        for link in edge_links {
            data.extend_from_slice(&link.to_le_bytes());
        }
        data.extend_from_slice(&0u32.to_le_bytes()); // preferred edge link count
        data.extend_from_slice(&(door_links.len() as u32).to_le_bytes());
        for (crc, refr) in door_links {
            data.extend_from_slice(&crc.to_le_bytes());
            data.extend_from_slice(&refr.to_le_bytes());
        }
        data.push(0); // has_island = false
        data.extend_from_slice(&0xAABB_CCDDu32.to_le_bytes()); // pathing cell crc
        data.extend_from_slice(&parent_world_form_id.to_le_bytes());
        data.extend_from_slice(&(-2i16).to_le_bytes()); // y
        data.extend_from_slice(&3i16.to_le_bytes()); // x
        data
    }

    fn refr_record(form_id: u32) -> ParsedRecord {
        make_record("REFR", form_id, None)
    }

    /// Regression: NVMI door links whose remapped Door Ref is NOT an emitted
    /// REFR are wild pointers (CTD on cell entry) and must be dropped; links to
    /// emitted REFRs survive.
    #[test]
    fn rebuild_projected_navi_drops_door_links_to_non_emitted_refrs() {
        let handle_id = create_empty_plugin_handle("Test.esp", Some("fo4"));
        // Two source door links: ref 0x000700 -> target 0x000C00 (REFR will be
        // emitted), ref 0x000701 -> target 0x000C99 (NOT emitted -> wild ptr).
        let source_nvmi = source_nvmi_with_doors(
            0x000900,
            0x000800,
            &[],
            &[(0x1111, 0x000700), (0x2222, 0x000701)],
        );
        let source_root_items = vec![ParsedItem::Record(source_navi_with_nvmi(
            0x000FF1,
            source_nvmi,
        ))];

        {
            let mut store = plugin_handle_store_ref().lock().unwrap();
            let slot = store.get_mut(&handle_id).unwrap();
            // Target NAVM for the NVMI.
            slot.parsed
                .root_items
                .push(ParsedItem::Record(exterior_navmesh_record_with_edges(
                    0x000A00,
                    0x000B00,
                    (3, -2),
                    &[],
                )));
            // Only the FIRST door's target REFR (0x000C00) is emitted; 0x000C99 is not.
            slot.parsed
                .root_items
                .push(ParsedItem::Record(refr_record(0x000C00)));

            let stats = rebuild_projected_navi_record_from_source_in_slot(
                slot,
                &source_root_items,
                &[
                    (0x000800, 0x000B00),
                    (0x000900, 0x000A00),
                    (0x000700, 0x000C00),
                    (0x000701, 0x000C99),
                ],
                Some(0x000FF1),
            )
            .expect("NAVI rebuild");
            assert_eq!(stats.navmesh_infos, 1);
            // One door link dropped (the non-emitted 0x000C99 wild pointer).
            assert_eq!(stats.stale_edge_links_dropped, 1);
        }

        let store = plugin_handle_store_ref().lock().unwrap();
        let slot = store.get(&handle_id).unwrap();
        let navi = first_top_level_record(&slot.parsed.root_items, "NAVI").expect("top-level NAVI");
        let nvmi = navmesh_info_subrecord(navi, 0x000A00);
        // Re-parse the NVMI to the door-link section and assert exactly 1 link
        // survives, pointing at the emitted REFR 0x000C00.
        let mut o = 24usize; // navmesh + flags + loc + f0
        let ec = u32::from_le_bytes(nvmi[o..o + 4].try_into().unwrap()) as usize;
        o += 4 + 4 * ec;
        let pc = u32::from_le_bytes(nvmi[o..o + 4].try_into().unwrap()) as usize;
        o += 4 + 4 * pc;
        let dc = u32::from_le_bytes(nvmi[o..o + 4].try_into().unwrap()) as usize;
        o += 4;
        assert_eq!(dc, 1, "exactly one door link survives");
        let surviving_ref = u32::from_le_bytes(nvmi[o + 4..o + 8].try_into().unwrap());
        assert_eq!(
            surviving_ref, 0x000C00,
            "surviving link points at emitted REFR"
        );
    }

    #[test]
    fn rebuild_projected_navi_can_override_source_nver() {
        let handle_id = create_empty_plugin_handle("Test.esp", Some("fo4"));
        let mut source_navi = source_navi_with_nvmi(
            0x000FF1,
            source_nvmi_with_doors(0x000900, 0x000800, &[], &[]),
        );
        source_navi.subrecords[0].data = Bytes::from(12u32.to_le_bytes().to_vec());
        source_navi
            .subrecords
            .iter_mut()
            .find(|subrecord| subrecord.signature.as_str() == "NVPP")
            .unwrap()
            .data = Bytes::from(vec![1, 0, 0, 0, 0x34, 0x12, 0, 0, 0, 0, 0, 0]);
        let source_root_items = vec![ParsedItem::Record(source_navi)];

        {
            let mut store = plugin_handle_store_ref().lock().unwrap();
            let slot = store.get_mut(&handle_id).unwrap();
            slot.parsed
                .root_items
                .push(ParsedItem::Record(exterior_navmesh_record_with_edges(
                    0x000A00,
                    0x000B00,
                    (3, -2),
                    &[],
                )));

            rebuild_projected_navi_record_from_source_in_slot_with_nver(
                slot,
                &source_root_items,
                &[(0x000800, 0x000B00), (0x000900, 0x000A00)],
                Some(0x000FF1),
                Some(15),
            )
            .expect("NAVI rebuild");

            let navi =
                first_top_level_record(&slot.parsed.root_items, "NAVI").expect("top-level NAVI");
            let nver = navi
                .subrecords
                .iter()
                .find(|subrecord| subrecord.signature.as_str() == "NVER")
                .expect("NVER");
            assert_eq!(nver.data.as_ref(), 15u32.to_le_bytes());
            let nvpp = navi
                .subrecords
                .iter()
                .find(|subrecord| subrecord.signature.as_str() == "NVPP")
                .expect("NVPP");
            assert_eq!(nvpp.data.as_ref(), &[0u8; 8]);
        }

        assert!(plugin_handle_close_native(handle_id));
    }

    #[test]
    fn rebuild_projected_navi_forces_fo4_nver_by_default() {
        let handle_id = create_empty_plugin_handle("Test.esp", Some("fo4"));
        let mut source_navi = source_navi_with_nvmi(
            0x000FF1,
            source_nvmi_with_doors(0x000900, 0x000800, &[], &[]),
        );
        source_navi.subrecords[0].data = Bytes::from(12u32.to_le_bytes().to_vec());
        source_navi
            .subrecords
            .iter_mut()
            .find(|subrecord| subrecord.signature.as_str() == "NVPP")
            .unwrap()
            .data = Bytes::from(vec![1, 0, 0, 0, 0x34, 0x12, 0, 0, 0, 0, 0, 0]);
        let source_root_items = vec![ParsedItem::Record(source_navi)];

        {
            let mut store = plugin_handle_store_ref().lock().unwrap();
            let slot = store.get_mut(&handle_id).unwrap();
            slot.parsed
                .root_items
                .push(ParsedItem::Record(exterior_navmesh_record_with_edges(
                    0x000A00,
                    0x000B00,
                    (3, -2),
                    &[],
                )));

            rebuild_projected_navi_record_from_source_in_slot(
                slot,
                &source_root_items,
                &[(0x000800, 0x000B00), (0x000900, 0x000A00)],
                Some(0x000FF1),
            )
            .expect("NAVI rebuild");

            let navi =
                first_top_level_record(&slot.parsed.root_items, "NAVI").expect("top-level NAVI");
            let nver = navi
                .subrecords
                .iter()
                .find(|subrecord| subrecord.signature.as_str() == "NVER")
                .expect("NVER");
            assert_eq!(nver.data.as_ref(), 15u32.to_le_bytes());
            let nvpp = navi
                .subrecords
                .iter()
                .find(|subrecord| subrecord.signature.as_str() == "NVPP")
                .expect("NVPP");
            assert_eq!(nvpp.data.as_ref(), &[0u8; 8]);
        }

        assert!(plugin_handle_close_native(handle_id));
    }

    fn rebuild_mixed_version_navi(order: &[(u32, u32)]) -> Vec<(String, Vec<u8>)> {
        let handle_id =
            create_empty_plugin_handle(&format!("MixedNavi{:08X}.esp", order[0].0), Some("fo4"));
        {
            let mut store = plugin_handle_store_ref().lock().unwrap();
            let slot = store.get_mut(&handle_id).unwrap();
            for &(form_id, version) in order {
                let mut record = exterior_navmesh_record_with_edges(
                    form_id,
                    0x000B00,
                    ((form_id & 0xFF) as i16, -2),
                    &[],
                );
                let nvnm = record
                    .subrecords
                    .iter_mut()
                    .find(|subrecord| subrecord.signature.as_str() == "NVNM")
                    .expect("NVNM");
                let mut data = nvnm.data.to_vec();
                data[0..4].copy_from_slice(&version.to_le_bytes());
                nvnm.data = Bytes::from(data);
                slot.parsed.root_items.push(ParsedItem::Record(record));
            }
            rebuild_projected_navi_record_in_slot(slot, Some(0x000FF1))
                .expect("mixed-version NAVI rebuild");

            let mut normalized_versions = Vec::new();
            fn collect_versions(items: &[ParsedItem], out: &mut Vec<(u32, u32)>) {
                for item in items {
                    match item {
                        ParsedItem::Record(record) if record.signature.as_str() == "NAVM" => {
                            let nvnm = record
                                .subrecords
                                .iter()
                                .find(|subrecord| subrecord.signature.as_str() == "NVNM")
                                .expect("normalized NVNM");
                            out.push((
                                record.form_id,
                                u32::from_le_bytes(nvnm.data[0..4].try_into().unwrap()),
                            ));
                        }
                        ParsedItem::Group(group) => collect_versions(&group.children, out),
                        _ => {}
                    }
                }
            }
            collect_versions(&slot.parsed.root_items, &mut normalized_versions);
            normalized_versions.sort_unstable();
            assert_eq!(
                normalized_versions,
                vec![(0x000900, 15), (0x000901, 15)],
                "every finalized target NVNM must be normalized to v15"
            );

            let navi = first_top_level_record(&slot.parsed.root_items, "NAVI").expect("NAVI");
            let nver = navi
                .subrecords
                .iter()
                .find(|subrecord| subrecord.signature.as_str() == "NVER")
                .expect("NVER");
            assert_eq!(nver.data.as_ref(), 15u32.to_le_bytes());
            assert_eq!(
                navi.subrecords
                    .iter()
                    .filter(|subrecord| subrecord.signature.as_str() == "NVMI")
                    .count(),
                2
            );
            let mut navmesh_ids = navi
                .subrecords
                .iter()
                .filter(|subrecord| subrecord.signature.as_str() == "NVMI")
                .map(|subrecord| u32::from_le_bytes(subrecord.data[0..4].try_into().unwrap()))
                .collect::<Vec<_>>();
            navmesh_ids.sort_unstable();
            assert_eq!(navmesh_ids, vec![0x000900, 0x000901]);
        }

        let store = plugin_handle_store_ref().lock().unwrap();
        let slot = store.get(&handle_id).unwrap();
        let navi = first_top_level_record(&slot.parsed.root_items, "NAVI").expect("NAVI");
        let snapshot = navi
            .subrecords
            .iter()
            .map(|subrecord| (subrecord.signature.to_string(), subrecord.data.to_vec()))
            .collect();
        drop(store);
        assert!(plugin_handle_close_native(handle_id));
        snapshot
    }

    #[test]
    fn rebuild_projected_navi_normalizes_mixed_versions_deterministically() {
        let legacy_first = rebuild_mixed_version_navi(&[(0x000900, 11), (0x000901, 15)]);
        let fo4_first = rebuild_mixed_version_navi(&[(0x000901, 15), (0x000900, 11)]);
        assert_eq!(legacy_first, fo4_first, "input order must not affect NAVI");
    }

    #[test]
    fn rebuild_projected_navi_rejects_true_legacy_layout_before_mutation() {
        let handle_id = create_empty_plugin_handle("LegacyNaviReject.esp", Some("fo4"));
        {
            let mut store = plugin_handle_store_ref().lock().unwrap();
            let slot = store.get_mut(&handle_id).unwrap();
            slot.parsed
                .root_items
                .push(ParsedItem::Record(source_navi_with_nvmi(
                    0x000FF1,
                    source_nvmi_with_doors(0x000900, 0x000800, &[], &[]),
                )));
            let mut malformed = make_record("NAVM", 0x000900, None);
            let mut bytes = vec![0u8; 20];
            bytes[0..4].copy_from_slice(&11u32.to_le_bytes());
            malformed.subrecords.push(ParsedSubrecord {
                signature: SmolStr::new_static("NVNM"),
                data: Bytes::from(bytes),
                semantic_type: None,
            });
            slot.parsed.root_items.push(ParsedItem::Record(malformed));

            let error = rebuild_projected_navi_record_in_slot(slot, Some(0x000FF1))
                .expect_err("true legacy layout must be rejected");
            assert!(error.contains("non-normalizable NVNM"), "{error}");
            assert_eq!(
                count_test_records_by_signature(&slot.parsed.root_items, "NAVI"),
                1,
                "pre-existing NAVI must survive a rejected rebuild"
            );
        }
        assert!(plugin_handle_close_native(handle_id));
    }

    #[test]
    fn handle_localized_save_uses_schema_aware_string_writer() {
        let string_id = 0x110;
        let mut plugin = empty_plugin(Some("fo4"));
        plugin.plugin_name = "SeventySix.esm".to_string();
        plugin.header.flags = TES4_FLAG_LOCALIZED;
        let mut record = make_record("BOOK", 0x0700_0800, None);
        record
            .subrecords
            .push(localized_subrecord("FULL", string_id));
        record
            .subrecords
            .push(localized_subrecord("DESC", string_id));
        plugin.root_items.push(ParsedItem::Record(record));

        let mut strings = LocalizedStringsState {
            default_language: "en".to_string(),
            ..LocalizedStringsState::default()
        };
        strings
            .by_language
            .entry("en".to_string())
            .or_default()
            .insert(string_id, "Shared text".to_string());
        strings.table_types.insert(string_id, "strings".to_string());

        let output_path = temp_plugin_path("handle-localized-save");
        let root = output_path.parent().unwrap().to_path_buf();

        save_localized_strings_snapshot(&plugin, &strings, output_path.to_str().unwrap()).unwrap();

        let strings_values =
            strings::parse_string_table(&root.join("Strings").join("SeventySix_en.STRINGS"))
                .unwrap();
        let dlstrings_values =
            strings::parse_string_table(&root.join("Strings").join("SeventySix_en.DLSTRINGS"))
                .unwrap();
        assert_eq!(
            strings_values.get(&string_id).map(String::as_str),
            Some("Shared text")
        );
        assert_eq!(
            dlstrings_values.get(&string_id).map(String::as_str),
            Some("Shared text")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn placed_record_position_offset_updates_only_placed_data() {
        let mut placed = make_record("REFR", 0x01000800, None);
        placed
            .subrecords
            .push(data_subrecord([1.0, 2.0, 3.0, 4.0, 5.0, 6.0]));
        placed.raw_payload = Some(Bytes::from_static(b"stale"));
        let mut non_placed = make_record("STAT", 0x01000801, None);
        non_placed
            .subrecords
            .push(data_subrecord([10.0, 20.0, 30.0, 40.0, 50.0, 60.0]));
        let mut items = vec![ParsedItem::Record(placed), ParsedItem::Record(non_placed)];

        let changed =
            apply_placed_record_position_offset_in_items(&mut items, (2048.0, 1024.0, -10.0));

        assert_eq!(changed, 1);
        let ParsedItem::Record(placed) = &items[0] else {
            panic!("expected record");
        };
        let data = &placed.subrecords[0].data;
        assert_eq!(f32::from_le_bytes(data[0..4].try_into().unwrap()), 2049.0);
        assert_eq!(f32::from_le_bytes(data[4..8].try_into().unwrap()), 1026.0);
        assert_eq!(f32::from_le_bytes(data[8..12].try_into().unwrap()), -7.0);
        assert_eq!(f32::from_le_bytes(data[12..16].try_into().unwrap()), 4.0);
        assert!(placed.raw_payload.is_none());

        let ParsedItem::Record(non_placed) = &items[1] else {
            panic!("expected record");
        };
        let data = &non_placed.subrecords[0].data;
        assert_eq!(f32::from_le_bytes(data[0..4].try_into().unwrap()), 10.0);
    }

    #[test]
    fn insert_parsed_record_creates_top_group_in_game_order() {
        let handle_id = create_empty_plugin_handle("Test.esp", Some("fo4"));

        insert_parsed_record(handle_id, make_record("WRLD", 0xFF000800, Some("World"))).unwrap();
        insert_parsed_record(handle_id, make_record("STAT", 0xFF000801, Some("Static"))).unwrap();

        let store = plugin_handle_store_ref().lock().unwrap();
        let slot = store.get(&handle_id).unwrap();
        let labels: Vec<String> = slot
            .parsed
            .root_items
            .iter()
            .filter_map(|item| match item {
                ParsedItem::Group(group) if group.group_type == 0 => {
                    Some(std::str::from_utf8(&group.label).unwrap().to_string())
                }
                _ => None,
            })
            .collect();

        assert_eq!(labels, vec!["STAT", "WRLD"]);
    }

    #[test]
    fn projected_cell_import_preserves_landscape_child_group() {
        let handle_id = create_empty_plugin_handle("Test.esp", Some("fo4"));
        let world_payload = serde_json::json!({
            "signature": "WRLD",
            "form_id": "000800:Test.esp",
            "eid": "TestWorld",
            "subrecords": [
                { "signature": "EDID", "data_hex": "54657374576F726C6400" }
            ]
        });
        let payload = serde_json::json!({
            "signature": "CELL",
            "form_id": "000801:Test.esp",
            "eid": "TestCell",
            "subrecords": [
                { "signature": "EDID", "data_hex": "5465737443656C6C00" }
            ],
            "Landscape": {
                "form_id": "000802:Test.esp",
                "subrecords": []
            }
        });
        let replacement_payload = serde_json::json!({
            "signature": "CELL",
            "form_id": "000801:Test.esp",
            "eid": "TestCell",
            "subrecords": [
                { "signature": "EDID", "data_hex": "5465737443656C6C00" },
                { "signature": "DATA", "data_hex": "0200" },
                { "signature": "XCLC", "data_hex": "03000000FEFFFFFF00000000" },
                { "signature": "LTMP", "data_hex": "00000000" },
                { "signature": "XCLW", "data_hex": "FFFF7F7F" }
            ],
            "Landscape": {
                "form_id": "000803:Test.esp",
                "subrecords": []
            }
        });
        let relative_path =
            "records/WRLD/TestWorld - 000800_Test.esp/0, 0/0, 0/0, 0/RecordData.yaml";

        plugin_handle_replace_authoring_record_value(handle_id, &world_payload)
            .expect("WRLD import");
        let imported = plugin_handle_replace_projected_cell_authoring_record_value(
            handle_id,
            &payload,
            relative_path,
        )
        .expect("projected CELL import");
        plugin_handle_replace_projected_cell_authoring_record_value(
            handle_id,
            &replacement_payload,
            relative_path,
        )
        .expect("projected CELL replacement");

        assert_eq!(imported.len(), 2);
        let store = plugin_handle_store_ref().lock().unwrap();
        let slot = store.get(&handle_id).unwrap();
        assert!(slot.parsed.root_items.iter().all(|item| !matches!(
            item,
            ParsedItem::Group(group) if group.group_type == 0 && group.label == *b"CELL"
        )));
        let wrld_group = slot
            .parsed
            .root_items
            .iter()
            .find_map(|item| match item {
                ParsedItem::Group(group) if group.group_type == 0 && group.label == *b"WRLD" => {
                    Some(group)
                }
                _ => None,
            })
            .expect("WRLD top group");
        assert!(matches!(
            wrld_group.children.first(),
            Some(ParsedItem::Record(record))
                if record.signature.as_str() == "WRLD" && record.form_id == 0x000800
        ));
        let world_children = wrld_group
            .children
            .iter()
            .find_map(|item| match item {
                ParsedItem::Group(group) if group.group_type == 1 => Some(group),
                _ => None,
            })
            .expect("WRLD children group");
        assert_eq!(world_children.label, 0x000800u32.to_le_bytes());
        let block_group = world_children
            .children
            .iter()
            .find_map(|item| match item {
                ParsedItem::Group(group) if group.group_type == EXTERIOR_CELL_BLOCK => Some(group),
                _ => None,
            })
            .expect("exterior block group");
        assert_eq!(block_group.label, encode_exterior_grid_label(0, 0));
        let subblock_group = block_group
            .children
            .iter()
            .find_map(|item| match item {
                ParsedItem::Group(group) if group.group_type == EXTERIOR_CELL_SUBBLOCK => {
                    Some(group)
                }
                _ => None,
            })
            .expect("exterior subblock group");
        assert_eq!(subblock_group.label, encode_exterior_grid_label(0, 0));
        let cell_records: Vec<&ParsedRecord> = subblock_group
            .children
            .iter()
            .filter_map(|item| match item {
                ParsedItem::Record(record) if record.signature.as_str() == "CELL" => Some(record),
                _ => None,
            })
            .collect();
        assert_eq!(cell_records.len(), 1);
        assert_eq!(cell_records[0].form_id, 0x000801);
        assert!(cell_records[0].subrecords.iter().any(|subrecord| {
            subrecord.signature.as_str() == "DATA" && subrecord.data.as_ref() == [0x02, 0x00]
        }));
        assert!(cell_records[0].subrecords.iter().any(|subrecord| {
            subrecord.signature.as_str() == "XCLW"
                && subrecord.data.as_ref() == [0xFF, 0xFF, 0x7F, 0x7F]
        }));
        assert!(cell_records[0].subrecords.iter().any(|subrecord| {
            subrecord.signature.as_str() == "LTMP"
                && subrecord.data.as_ref() == [0x00, 0x00, 0x00, 0x00]
        }));
        let child_groups: Vec<&ParsedGroup> = subblock_group
            .children
            .iter()
            .filter_map(|item| match item {
                ParsedItem::Group(group) if group.group_type == CELL_CHILD_GROUP => Some(group),
                _ => None,
            })
            .collect();
        assert_eq!(child_groups.len(), 1);
        let child_group = child_groups[0];

        assert_eq!(child_group.label, 0x000801u32.to_le_bytes());
        assert!(
            child_group.children.iter().all(|item| {
                !matches!(item, ParsedItem::Record(record) if record.signature.as_str() == "LAND")
            }),
            "LAND must be nested in the CELL temporary group",
        );
        let temporary_group = child_group
            .children
            .iter()
            .find_map(|item| match item {
                ParsedItem::Group(group) if group.group_type == TEMPORARY_GROUP => Some(group),
                _ => None,
            })
            .expect("temporary group");
        assert!(matches!(
            temporary_group.children.first(),
            Some(ParsedItem::Record(record))
                if record.signature.as_str() == "LAND" && record.form_id == 0x000803
        ));
    }

    #[test]
    fn projected_cell_import_preserves_land_vtxt_raw_hex() {
        let handle_id = create_empty_plugin_handle("Test.esp", Some("fo4"));
        let world_payload = serde_json::json!({
            "signature": "WRLD",
            "form_id": "000800:Test.esp",
            "eid": "TestWorld",
            "subrecords": [
                { "signature": "EDID", "data_hex": "54657374576F726C6400" }
            ]
        });
        let mut vtxt = Vec::new();
        vtxt.extend_from_slice(&0u16.to_le_bytes());
        vtxt.extend_from_slice(&[0, 0]);
        vtxt.extend_from_slice(&(254.0f32 / 255.0).to_le_bytes());
        let vtxt_hex = hex::encode_upper(&vtxt);
        let payload = serde_json::json!({
            "signature": "CELL",
            "form_id": "000801:Test.esp",
            "eid": "TestCell",
            "fields": [
                { "XCLC": { "raw_hex": "000000000000000000000000" } }
            ],
            "Landscape": {
                "form_id": "000802:Test.esp",
                "fields": [
                    { "BTXT": {
                        "Texture": { "reference": { "plugin": "Test.esp", "object_id": "000900" } },
                        "Quadrant": "BottomLeft",
                        "UnknownByte3": 2,
                        "Layer": -1
                    }},
                    { "ATXT": {
                        "Texture": { "reference": { "plugin": "Test.esp", "object_id": "000901" } },
                        "Quadrant": "BottomLeft",
                        "UnknownByte3": 0,
                        "Layer": 0
                    }},
                    { "AlphaLayerData": { "raw_hex": vtxt_hex } }
                ]
            }
        });
        let relative_path =
            "records/WRLD/TestWorld - 000800_Test.esp/0, 0/0, 0/0, 0/RecordData.yaml";

        plugin_handle_replace_authoring_record_value(handle_id, &world_payload)
            .expect("WRLD import");
        plugin_handle_replace_projected_cell_authoring_record_value(
            handle_id,
            &payload,
            relative_path,
        )
        .expect("projected CELL import");

        let store = plugin_handle_store_ref().lock().unwrap();
        let slot = store.get(&handle_id).unwrap();
        let land = find_first_record(&slot.parsed.root_items, &mut |record| {
            record.signature.as_str() == "LAND"
        })
        .expect("LAND record");
        let vtxt_subrecord = land
            .subrecords
            .iter()
            .find(|subrecord| subrecord.signature.as_str() == "VTXT")
            .expect("VTXT subrecord");
        assert_eq!(vtxt_subrecord.data.as_ref(), vtxt.as_slice());
    }

    #[test]
    fn projected_cell_batch_import_preserves_land_vtxt_raw_hex() {
        let handle_id = create_empty_plugin_handle("Test.esp", Some("fo4"));
        let world_payload = serde_json::json!({
            "signature": "WRLD",
            "form_id": "000800:Test.esp",
            "eid": "TestWorld",
            "subrecords": [
                { "signature": "EDID", "data_hex": "54657374576F726C6400" }
            ]
        });
        let mut vtxt = Vec::new();
        vtxt.extend_from_slice(&0u16.to_le_bytes());
        vtxt.extend_from_slice(&[0, 0]);
        vtxt.extend_from_slice(&(254.0f32 / 255.0).to_le_bytes());
        let vtxt_hex = hex::encode_upper(&vtxt);
        let payload = serde_json::json!({
            "signature": "CELL",
            "form_id": "000801:Test.esp",
            "eid": "TestCell",
            "fields": [
                { "XCLC": { "raw_hex": "000000000000000000000000" } }
            ],
            "Landscape": {
                "form_id": "000802:Test.esp",
                "fields": [
                    { "BTXT": {
                        "Texture": { "reference": { "plugin": "Test.esp", "object_id": "000900" } },
                        "Quadrant": "BottomLeft",
                        "UnknownByte3": 2,
                        "Layer": -1
                    }},
                    { "ATXT": {
                        "Texture": { "reference": { "plugin": "Test.esp", "object_id": "000901" } },
                        "Quadrant": "BottomLeft",
                        "UnknownByte3": 0,
                        "Layer": 0
                    }},
                    { "AlphaLayerData": { "raw_hex": vtxt_hex } }
                ]
            }
        });
        let relative_path =
            "records/WRLD/TestWorld - 000800_Test.esp/0, 0/0, 0/0, 0/RecordData.yaml";

        plugin_handle_replace_authoring_record_value(handle_id, &world_payload)
            .expect("WRLD import");
        plugin_handle_replace_projected_cell_authoring_record_values_at_locations(
            handle_id,
            vec![(payload, relative_path.to_string())],
        )
        .expect("projected CELL batch import");

        let store = plugin_handle_store_ref().lock().unwrap();
        let slot = store.get(&handle_id).unwrap();
        let land = find_first_record(&slot.parsed.root_items, &mut |record| {
            record.signature.as_str() == "LAND"
        })
        .expect("LAND record");
        let vtxt_subrecord = land
            .subrecords
            .iter()
            .find(|subrecord| subrecord.signature.as_str() == "VTXT")
            .expect("VTXT subrecord");
        assert_eq!(vtxt_subrecord.data.as_ref(), vtxt.as_slice());
    }

    #[test]
    fn projected_navmesh_insertion_ignores_persistent_cell_origin_grid() {
        let handle_id = create_empty_plugin_handle("Test.esp", Some("fo4"));
        let world_payload = serde_json::json!({
            "signature": "WRLD",
            "form_id": "000800:Test.esp",
            "eid": "TestWorld",
            "subrecords": [
                { "signature": "EDID", "data_hex": "54657374576F726C6400" }
            ]
        });
        let cell_payload = serde_json::json!({
            "signature": "CELL",
            "form_id": "000801:Test.esp",
            "eid": "TestExteriorOrigin",
            "subrecords": [
                { "signature": "EDID", "data_hex": "546573744578746572696F724F726967696E00" },
                { "signature": "XCLC", "data_hex": "000000000000000000000000" }
            ],
            "Landscape": {
                "form_id": "000803:Test.esp",
                "subrecords": []
            }
        });
        let relative_path = "records/WRLD/TestWorld - 000800_Test.esp/0,0/0,0/0,0/RecordData.yaml";

        plugin_handle_replace_authoring_record_value(handle_id, &world_payload)
            .expect("WRLD import");
        plugin_handle_replace_projected_cell_authoring_record_value(
            handle_id,
            &cell_payload,
            relative_path,
        )
        .expect("projected CELL import");

        let mut store = plugin_handle_store_ref().lock().unwrap();
        let slot = store.get_mut(&handle_id).unwrap();
        {
            let world_children =
                projected_world_children_group_mut(&mut slot.parsed.root_items, 0x000800)
                    .expect("WRLD children");
            let mut persistent_cell = make_record("CELL", 0x000802, Some("TestPersistentCell"));
            ensure_projected_cell_grid_subrecord(&mut persistent_cell, (0, 0));
            world_children.children.insert(
                0,
                ParsedItem::Group(ParsedGroup {
                    label: 0x000802u32.to_le_bytes(),
                    group_type: CELL_CHILD_GROUP,
                    tail: Bytes::from(vec![0u8; MODERN_HEADER_SIZE - 16]),
                    children: Vec::new(),
                }),
            );
            world_children
                .children
                .insert(0, ParsedItem::Record(persistent_cell));
            assert_eq!(
                build_cell_grid_index(&world_children.children).get(&(0, 0)),
                Some(&0x000801)
            );
        }

        assert!(
            insert_projected_navmesh_record_in_slot(
                slot,
                exterior_navmesh_record(0x000900, 0x000800, (0, 0)),
            )
            .expect("single NAVM insert")
        );
        assert_eq!(
            insert_projected_navmeshes_batch_in_slot(
                slot,
                vec![exterior_navmesh_record(0x000901, 0x000800, (0, 0))],
            ),
            vec![Ok(true)]
        );

        let world_children =
            projected_world_children_group_mut(&mut slot.parsed.root_items, 0x000800)
                .expect("WRLD children");
        let exterior_group =
            find_cell_child_group_mut_in_items(&mut world_children.children, 0x000801)
                .expect("exterior CELL children");
        assert_eq!(
            count_test_records_by_signature(&exterior_group.children, "NAVM"),
            2
        );
        let persistent_group =
            find_cell_child_group_mut_in_items(&mut world_children.children, 0x000802)
                .expect("persistent CELL children");
        assert_eq!(
            count_test_records_by_signature(&persistent_group.children, "NAVM"),
            0
        );
    }

    #[test]
    fn insert_projected_navmesh_record_uses_exterior_cell_temporary_group() {
        let handle_id = create_empty_plugin_handle("Test.esp", Some("fo4"));
        let world_payload = serde_json::json!({
            "signature": "WRLD",
            "form_id": "000800:Test.esp",
            "eid": "TestWorld",
            "subrecords": [
                { "signature": "EDID", "data_hex": "54657374576F726C6400" }
            ]
        });
        let cell_payload = serde_json::json!({
            "signature": "CELL",
            "form_id": "000801:Test.esp",
            "eid": "TestCell",
            "subrecords": [
                { "signature": "EDID", "data_hex": "5465737443656C6C00" },
                { "signature": "XCLC", "data_hex": "03000000FEFFFFFF00000000" }
            ],
            "Landscape": {
                "form_id": "000803:Test.esp",
                "subrecords": []
            }
        });
        let relative_path = "records/WRLD/TestWorld - 000800_Test.esp/0,0/0,0/3,-2/RecordData.yaml";

        plugin_handle_replace_authoring_record_value(handle_id, &world_payload)
            .expect("WRLD import");
        plugin_handle_replace_projected_cell_authoring_record_value(
            handle_id,
            &cell_payload,
            relative_path,
        )
        .expect("projected CELL import");

        {
            let mut store = plugin_handle_store_ref().lock().unwrap();
            let slot = store.get_mut(&handle_id).unwrap();
            assert!(
                insert_projected_navmesh_record_in_slot(
                    slot,
                    exterior_navmesh_record(0x000900, 0x000800, (3, -2)),
                )
                .expect("first NAVM insert")
            );
            assert!(
                insert_projected_navmesh_record_in_slot(
                    slot,
                    exterior_navmesh_record(0x000900, 0x000800, (3, -2)),
                )
                .expect("replacement NAVM insert")
            );
        }

        let store = plugin_handle_store_ref().lock().unwrap();
        let slot = store.get(&handle_id).unwrap();
        let wrld_group = slot
            .parsed
            .root_items
            .iter()
            .find_map(|item| match item {
                ParsedItem::Group(group) if group.group_type == 0 && group.label == *b"WRLD" => {
                    Some(group)
                }
                _ => None,
            })
            .expect("WRLD top group");
        let world_children = wrld_group
            .children
            .iter()
            .find_map(|item| match item {
                ParsedItem::Group(group) if group.group_type == 1 => Some(group),
                _ => None,
            })
            .expect("WRLD children group");
        let block_group = world_children
            .children
            .iter()
            .find_map(|item| match item {
                ParsedItem::Group(group)
                    if group.group_type == EXTERIOR_CELL_BLOCK
                        && group.label == encode_exterior_grid_label(0, 0) =>
                {
                    Some(group)
                }
                _ => None,
            })
            .expect("target exterior block group");
        let subblock_group = block_group
            .children
            .iter()
            .find_map(|item| match item {
                ParsedItem::Group(group)
                    if group.group_type == EXTERIOR_CELL_SUBBLOCK
                        && group.label == encode_exterior_grid_label(0, 0) =>
                {
                    Some(group)
                }
                _ => None,
            })
            .expect("target exterior subblock group");
        let child_group = subblock_group
            .children
            .iter()
            .find_map(|item| match item {
                ParsedItem::Group(group) if group.group_type == CELL_CHILD_GROUP => Some(group),
                _ => None,
            })
            .expect("CELL child group");
        assert!(
            child_group.children.iter().all(|item| {
                !matches!(item, ParsedItem::Record(record) if record.signature.as_str() == "NAVM")
            }),
            "NAVM must be nested in the CELL temporary group",
        );
        let temporary_group = child_group
            .children
            .iter()
            .find_map(|item| match item {
                ParsedItem::Group(group) if group.group_type == TEMPORARY_GROUP => Some(group),
                _ => None,
            })
            .expect("temporary group");
        let navmesh_records: Vec<&ParsedRecord> = temporary_group
            .children
            .iter()
            .filter_map(|item| match item {
                ParsedItem::Record(record) if record.signature.as_str() == "NAVM" => Some(record),
                _ => None,
            })
            .collect();
        assert_eq!(navmesh_records.len(), 1);
        assert_eq!(navmesh_records[0].form_id, 0x000900);
        assert_eq!(
            navmesh_parent_from_record(navmesh_records[0]).expect("NAVM parent"),
            Some(NavmeshParent::Exterior {
                world_form_id: 0x000800,
                x: 3,
                y: -2,
            })
        );
    }

    fn setup_projected_world_handle_for_batch(plugin_name: &str) -> u64 {
        let handle_id = create_empty_plugin_handle(plugin_name, Some("fo4"));
        let world_payload = serde_json::json!({
            "signature": "WRLD",
            "form_id": format!("000800:{plugin_name}"),
            "eid": "TestWorld",
            "subrecords": [
                { "signature": "EDID", "data_hex": "54657374576F726C6400" }
            ]
        });
        plugin_handle_replace_authoring_record_value(handle_id, &world_payload)
            .expect("WRLD import");
        for (cell_id, land_id, xclc_hex, grid_dir) in [
            (0x000801u32, 0x000810u32, "03000000FEFFFFFF00000000", "3,-2"),
            (0x000802, 0x000811, "04000000FEFFFFFF00000000", "4,-2"),
            (0x000803, 0x000812, "05000000FEFFFFFF00000000", "5,-2"),
        ] {
            let cell_payload = serde_json::json!({
                "signature": "CELL",
                "form_id": format!("{cell_id:06X}:{plugin_name}"),
                "eid": format!("TestCell{cell_id:06X}"),
                "subrecords": [
                    { "signature": "XCLC", "data_hex": xclc_hex }
                ],
                "Landscape": {
                    "form_id": format!("{land_id:06X}:{plugin_name}"),
                    "subrecords": []
                }
            });
            let relative_path = format!(
                "records/WRLD/TestWorld - 000800_{plugin_name}/0,0/0,0/{grid_dir}/RecordData.yaml"
            );
            plugin_handle_replace_projected_cell_authoring_record_value(
                handle_id,
                &cell_payload,
                &relative_path,
            )
            .expect("projected CELL import");
        }
        handle_id
    }

    fn tree_fingerprint(items: &[ParsedItem], depth: usize, out: &mut String) {
        for item in items {
            match item {
                ParsedItem::Group(group) => {
                    out.push_str(&format!(
                        "{depth}|G {:?} t={}\n",
                        group.label, group.group_type
                    ));
                    tree_fingerprint(&group.children, depth + 1, out);
                }
                ParsedItem::Record(record) => {
                    out.push_str(&format!(
                        "{depth}|R {} {:08X}",
                        record.signature, record.form_id
                    ));
                    for subrecord in &record.subrecords {
                        out.push_str(&format!(
                            " {}:{:02X?}",
                            subrecord.signature,
                            subrecord.data.as_ref()
                        ));
                    }
                    out.push('\n');
                }
            }
        }
    }

    fn setup_structural_replace_handle(plugin_name: &str) -> u64 {
        let handle = create_empty_plugin_handle(plugin_name, Some("fo4"));
        let mut store = plugin_handle_store_ref().lock().unwrap();
        let slot = store.get_mut(&handle).unwrap();
        slot.parsed.root_items = vec![group(
            0,
            *b"WEAP",
            vec![
                ParsedItem::Record(make_record("WEAP", 0x800, Some("A-old-first"))),
                ParsedItem::Record(make_record("WEAP", 0x801, Some("B-old"))),
                ParsedItem::Record(make_record("WEAP", 0x800, Some("A-old-second"))),
                ParsedItem::Record(make_record("WEAP", 0x802, Some("C-old"))),
            ],
        )];
        slot.invalidate_sections();
        handle
    }

    fn structural_replace_fingerprint(handle: u64) -> String {
        let store = plugin_handle_store_ref().lock().unwrap();
        let mut fingerprint = String::new();
        tree_fingerprint(
            &store.get(&handle).unwrap().parsed.root_items,
            0,
            &mut fingerprint,
        );
        fingerprint
    }

    #[test]
    fn structural_batch_matches_sequential_order_upsert_and_signature_change() {
        let sequential = setup_structural_replace_handle("StructuralSequential.esp");
        let batched = setup_structural_replace_handle("StructuralBatched.esp");
        let replacements = || {
            vec![
                make_record("WEAP", 0x800, Some("A-new")),
                make_record("ARMO", 0x802, Some("C-new-signature")),
                make_record("WEAP", 0x803, Some("D-upsert")),
            ]
        };

        {
            let mut store = plugin_handle_store_ref().lock().unwrap();
            let slot = store.get_mut(&sequential).unwrap();
            for record in replacements() {
                replace_parsed_record_in_slot(slot, record);
            }
            replace_parsed_records_in_slot_batch(store.get_mut(&batched).unwrap(), replacements());
        }

        assert_eq!(
            structural_replace_fingerprint(sequential),
            structural_replace_fingerprint(batched)
        );
    }

    #[test]
    fn structural_batch_duplicate_inputs_match_sequential_fallback() {
        let sequential = setup_structural_replace_handle("DuplicateSequential.esp");
        let batched = setup_structural_replace_handle("DuplicateBatched.esp");
        let replacements = || {
            vec![
                make_record("WEAP", 0x800, Some("A-new-first")),
                make_record("WEAP", 0x801, Some("B-new")),
                make_record("WEAP", 0x800, Some("A-new-last")),
            ]
        };

        {
            let mut store = plugin_handle_store_ref().lock().unwrap();
            let slot = store.get_mut(&sequential).unwrap();
            for record in replacements() {
                replace_parsed_record_in_slot(slot, record);
            }
            replace_parsed_records_in_slot_batch(store.get_mut(&batched).unwrap(), replacements());
        }

        assert_eq!(
            structural_replace_fingerprint(sequential),
            structural_replace_fingerprint(batched)
        );
    }

    /// The batch insert must be observationally identical to repeated
    /// single-record inserts — same outcomes in input order, same tree shape,
    /// same per-cell record order — including duplicate-form replacement and a
    /// grid that matches no cell.
    #[test]
    fn batch_navmesh_insert_matches_sequential_inserts() {
        let seq_handle = setup_projected_world_handle_for_batch("Seq.esp");
        let batch_handle = setup_projected_world_handle_for_batch("Batch.esp");

        let records = || {
            vec![
                exterior_navmesh_record(0x000900, 0x000800, (3, -2)),
                exterior_navmesh_record(0x000901, 0x000800, (4, -2)),
                exterior_navmesh_record(0x000900, 0x000800, (3, -2)), // duplicate -> replacement
                exterior_navmesh_record(0x000902, 0x000800, (5, -2)),
                exterior_navmesh_record(0x000903, 0x000800, (4, -2)),
                exterior_navmesh_record(0x000904, 0x000800, (9, 9)), // no matching cell -> Ok(false)
                exterior_navmesh_record(0x000905, 0x000800, (3, -2)),
            ]
        };

        let mut store = plugin_handle_store_ref().lock().unwrap();

        let mut seq_outcomes: Vec<Result<bool, String>> = Vec::new();
        {
            let slot = store.get_mut(&seq_handle).unwrap();
            for record in records() {
                seq_outcomes.push(insert_projected_navmesh_record_in_slot(slot, record));
            }
        }
        let batch_outcomes = {
            let slot = store.get_mut(&batch_handle).unwrap();
            insert_projected_navmeshes_batch_in_slot(slot, records())
        };

        assert_eq!(format!("{seq_outcomes:?}"), format!("{batch_outcomes:?}"));
        assert!(
            seq_outcomes
                .iter()
                .filter(|o| matches!(o, Ok(true)))
                .count()
                == 6,
            "fixture sanity: six inserts succeed, one grid misses"
        );

        let mut seq_fp = String::new();
        tree_fingerprint(
            &store.get(&seq_handle).unwrap().parsed.root_items,
            0,
            &mut seq_fp,
        );
        let mut batch_fp = String::new();
        tree_fingerprint(
            &store.get(&batch_handle).unwrap().parsed.root_items,
            0,
            &mut batch_fp,
        );
        assert_eq!(
            seq_fp, batch_fp,
            "tree shape + record order must be identical"
        );
    }

    #[test]
    fn batch_interior_navmesh_insert_matches_sequential_without_existing_child_group() {
        let seq_handle = create_empty_plugin_handle("SeqInterior.esp", Some("fo4"));
        let batch_handle = create_empty_plugin_handle("BatchInterior.esp", Some("fo4"));
        let mut store = plugin_handle_store_ref().lock().unwrap();
        for handle in [seq_handle, batch_handle] {
            store
                .get_mut(&handle)
                .unwrap()
                .parsed
                .root_items
                .push(ParsedItem::Record(make_record("CELL", 0x000801, None)));
        }

        let seq_outcome = insert_projected_navmesh_record_in_slot(
            store.get_mut(&seq_handle).unwrap(),
            interior_navmesh_record(0x000900, 0x000801),
        );
        let batch_outcome = insert_projected_navmeshes_batch_in_slot(
            store.get_mut(&batch_handle).unwrap(),
            vec![interior_navmesh_record(0x000900, 0x000801)],
        );
        assert_eq!(
            format!("{seq_outcome:?}"),
            format!("{:?}", batch_outcome[0])
        );
        assert!(seq_outcome.expect("sequential insert"));

        let mut seq_fp = String::new();
        tree_fingerprint(
            &store.get(&seq_handle).unwrap().parsed.root_items,
            0,
            &mut seq_fp,
        );
        let mut batch_fp = String::new();
        tree_fingerprint(
            &store.get(&batch_handle).unwrap().parsed.root_items,
            0,
            &mut batch_fp,
        );
        assert_eq!(seq_fp, batch_fp);
    }

    #[test]
    fn rebuild_projected_navi_uses_emitted_navmeshes_and_filters_stale_edges() {
        let handle_id = create_empty_plugin_handle("Test.esp", Some("fo4"));
        let world_payload = serde_json::json!({
            "signature": "WRLD",
            "form_id": "000800:Test.esp",
            "eid": "TestWorld",
            "subrecords": [
                { "signature": "EDID", "data_hex": "54657374576F726C6400" }
            ]
        });
        let cell_payload = serde_json::json!({
            "signature": "CELL",
            "form_id": "000801:Test.esp",
            "eid": "TestCell",
            "subrecords": [
                { "signature": "EDID", "data_hex": "5465737443656C6C00" },
                { "signature": "XCLC", "data_hex": "03000000FEFFFFFF00000000" }
            ],
            "Landscape": {
                "form_id": "000803:Test.esp",
                "subrecords": []
            }
        });
        let relative_path = "records/WRLD/TestWorld - 000800_Test.esp/0,0/0,0/3,-2/RecordData.yaml";

        plugin_handle_replace_authoring_record_value(handle_id, &world_payload)
            .expect("WRLD import");
        plugin_handle_replace_projected_cell_authoring_record_value(
            handle_id,
            &cell_payload,
            relative_path,
        )
        .expect("projected CELL import");

        {
            let mut store = plugin_handle_store_ref().lock().unwrap();
            let slot = store.get_mut(&handle_id).unwrap();
            insert_projected_navmesh_record_in_slot(
                slot,
                exterior_navmesh_record_with_edges(
                    0x000900,
                    0x000800,
                    (3, -2),
                    &[0x000901, 0x000999],
                ),
            )
            .expect("first NAVM insert");
            insert_projected_navmesh_record_in_slot(
                slot,
                exterior_navmesh_record_with_edges(0x000901, 0x000800, (3, -2), &[]),
            )
            .expect("second NAVM insert");

            let stats = rebuild_projected_navi_record_in_slot(slot, None).expect("NAVI rebuild");
            assert_eq!(stats.records_added, 1);
            assert_eq!(stats.navmesh_infos, 2);
            assert_eq!(stats.edge_links, 1);
            assert_eq!(stats.stale_edge_links_dropped, 1);
            assert_eq!(stats.warnings, 0);
        }

        let store = plugin_handle_store_ref().lock().unwrap();
        let slot = store.get(&handle_id).unwrap();
        let navi = first_top_level_record(&slot.parsed.root_items, "NAVI").expect("top-level NAVI");
        assert_eq!(navi.form_version, Some(131));
        assert_eq!(
            navi.subrecords
                .iter()
                .filter(|subrecord| subrecord.signature.as_str() == "NVMI")
                .count(),
            2
        );
        let first_nvmi = navmesh_info_subrecord(navi, 0x000900);
        assert_eq!(
            u32::from_le_bytes(first_nvmi[0..4].try_into().unwrap()),
            0x000900
        );
        assert_eq!(
            u32::from_le_bytes(first_nvmi[4..8].try_into().unwrap()),
            NAVI_NVMI_FLAG_IS_ISLAND
        );
        assert_eq!(
            f32::from_le_bytes(first_nvmi[8..12].try_into().unwrap()),
            10.0
        );
        assert_eq!(
            f32::from_le_bytes(first_nvmi[12..16].try_into().unwrap()),
            20.0
        );
        assert_eq!(
            f32::from_le_bytes(first_nvmi[16..20].try_into().unwrap()),
            30.0
        );
        assert_eq!(
            u32::from_le_bytes(first_nvmi[24..28].try_into().unwrap()),
            1
        );
        assert_eq!(
            u32::from_le_bytes(first_nvmi[28..32].try_into().unwrap()),
            0x000901
        );

        let mut pathing_offset = 32;
        assert_eq!(
            u32::from_le_bytes(
                first_nvmi[pathing_offset..pathing_offset + 4]
                    .try_into()
                    .unwrap()
            ),
            0
        );
        pathing_offset += 4;
        assert_eq!(
            u32::from_le_bytes(
                first_nvmi[pathing_offset..pathing_offset + 4]
                    .try_into()
                    .unwrap()
            ),
            0
        );
        pathing_offset += 4;
        assert_eq!(first_nvmi[pathing_offset], 1);
        pathing_offset += 1;
        for (index, expected) in [8.0f32, 18.0, 28.0, 12.0, 22.0, 32.0]
            .into_iter()
            .enumerate()
        {
            let start = pathing_offset + index * 4;
            assert_eq!(
                f32::from_le_bytes(first_nvmi[start..start + 4].try_into().unwrap()),
                expected
            );
        }
        pathing_offset += 24;
        assert_eq!(
            u32::from_le_bytes(
                first_nvmi[pathing_offset..pathing_offset + 4]
                    .try_into()
                    .unwrap()
            ),
            1
        );
        pathing_offset += 4;
        assert_eq!(
            &first_nvmi[pathing_offset..pathing_offset + 6],
            &[0, 0, 1, 0, 2, 0]
        );
        pathing_offset += 6;
        assert_eq!(
            u32::from_le_bytes(
                first_nvmi[pathing_offset..pathing_offset + 4]
                    .try_into()
                    .unwrap()
            ),
            3
        );
        pathing_offset += 4 + 3 * 12;
        assert_eq!(
            u32::from_le_bytes(
                first_nvmi[pathing_offset..pathing_offset + 4]
                    .try_into()
                    .unwrap()
            ),
            0xAABB_CCDD
        );
        assert_eq!(
            u32::from_le_bytes(
                first_nvmi[pathing_offset + 4..pathing_offset + 8]
                    .try_into()
                    .unwrap()
            ),
            0x000800
        );
        assert_eq!(
            i16::from_le_bytes(
                first_nvmi[pathing_offset + 8..pathing_offset + 10]
                    .try_into()
                    .unwrap()
            ),
            -2
        );
        assert_eq!(
            i16::from_le_bytes(
                first_nvmi[pathing_offset + 10..pathing_offset + 12]
                    .try_into()
                    .unwrap()
            ),
            3
        );
    }

    #[test]
    fn rebuild_projected_navi_preserves_source_nvmi_metadata_when_available() {
        let handle_id = create_empty_plugin_handle("Test.esp", Some("fo4"));
        let source_nvmi = source_nvmi_with_island(0x000900, 0x000800, &[0x000901, 0x000999]);
        let source_root_items = vec![ParsedItem::Record(source_navi_with_nvmi(
            0x000FF1,
            source_nvmi.clone(),
        ))];

        {
            let mut store = plugin_handle_store_ref().lock().unwrap();
            let slot = store.get_mut(&handle_id).unwrap();
            // NVMI.edge_links is rebuilt from
            // the TARGET NAVM's NVNM topology, not preserved from the source
            // NVMI. Give the target NAVMs real NVNMs with edge_links so the
            // test still exercises the edge_links path. NAVM 0x000A00 edges
            // out to 0x000A01 (in-slot) and 0x000B99 (NOT in-slot — verifies
            // stale filtering still happens during the NVNM rebuild).
            slot.parsed
                .root_items
                .push(ParsedItem::Record(exterior_navmesh_record_with_edges(
                    0x000A00,
                    0x000B00,
                    (3, -2),
                    &[0x000A01, 0x000B99],
                )));
            slot.parsed
                .root_items
                .push(ParsedItem::Record(exterior_navmesh_record_with_edges(
                    0x000A01,
                    0x000B00,
                    (3, -2),
                    &[],
                )));

            let stats = rebuild_projected_navi_record_from_source_in_slot(
                slot,
                &source_root_items,
                &[
                    (0x000800, 0x000B00),
                    (0x000900, 0x000A00),
                    (0x000901, 0x000A01),
                ],
                Some(0x000FF1),
            )
            .expect("NAVI rebuild");
            assert_eq!(stats.records_added, 1);
            // Source NAVI has 1 NVMI (for navmesh 0x000900 -> target 0x000A00);
            // the from-source path only emits NVMIs that appear in the source.
            assert_eq!(stats.navmesh_infos, 1);
            assert_eq!(stats.edge_links, 1);
            // The 0x000B99 in NAVM 0x000A00's NVNM is filtered as stale
            // (not in emitted_navmesh_ids). The source NVMI's 0x000999 is
            // never iterated, so stale_edge_links_dropped counts only the
            // NVNM-derived stale filter.
            assert_eq!(stats.stale_edge_links_dropped, 1);
        }

        let store = plugin_handle_store_ref().lock().unwrap();
        let slot = store.get(&handle_id).unwrap();
        let navi = first_top_level_record(&slot.parsed.root_items, "NAVI").expect("top-level NAVI");
        let nvmi = navmesh_info_subrecord(navi, 0x000A00);
        assert_eq!(
            u32::from_le_bytes(nvmi[4..8].try_into().unwrap()),
            NAVI_NVMI_FLAG_IS_ISLAND
        );
        assert!(nvmi.len() < source_nvmi.len());
        assert_eq!(u32::from_le_bytes(nvmi[24..28].try_into().unwrap()), 1);
        assert_eq!(
            u32::from_le_bytes(nvmi[28..32].try_into().unwrap()),
            0x000A01
        );
        assert_eq!(nvmi[40], 1);
        assert_eq!(u32::from_le_bytes(nvmi[65..69].try_into().unwrap()), 1);
        assert_eq!(&nvmi[69..75], &[0, 0, 1, 0, 2, 0]);
        assert_eq!(u32::from_le_bytes(nvmi[75..79].try_into().unwrap()), 3);
        assert_eq!(
            u32::from_le_bytes(nvmi[nvmi.len() - 12..nvmi.len() - 8].try_into().unwrap()),
            FO4_PATHING_CELL_CRC_HASH
        );
        assert_eq!(
            u32::from_le_bytes(nvmi[nvmi.len() - 8..nvmi.len() - 4].try_into().unwrap()),
            0x000B00
        );
    }

    #[test]
    fn rebuild_projected_navi_stamps_fo4_pathing_cell_crc() {
        let handle_id = create_empty_plugin_handle("Test.esp", Some("fo4"));
        let mut store = plugin_handle_store_ref().lock().unwrap();
        let slot = store.get_mut(&handle_id).unwrap();
        slot.parsed
            .root_items
            .push(ParsedItem::Record(exterior_navmesh_record_with_edges(
                0x000A00,
                0x000B00,
                (3, -2),
                &[],
            )));

        rebuild_projected_navi_record_in_slot(slot, Some(0x000FF1)).expect("NAVI rebuild");

        let navm = find_record_mut(&mut slot.parsed.root_items, 0x000A00).expect("target NAVM");
        let nvnm = effective_subrecords_for_record(navm)
            .iter()
            .find(|subrecord| subrecord.signature.as_str() == "NVNM")
            .expect("target NVNM")
            .data
            .clone();
        assert_eq!(
            u32::from_le_bytes(nvnm[4..8].try_into().unwrap()),
            FO4_PATHING_CELL_CRC_HASH
        );

        let navi = first_top_level_record(&slot.parsed.root_items, "NAVI").expect("top-level NAVI");
        let nvmi = navmesh_info_subrecord(navi, 0x000A00);
        assert_eq!(
            u32::from_le_bytes(nvmi[nvmi.len() - 12..nvmi.len() - 8].try_into().unwrap()),
            FO4_PATHING_CELL_CRC_HASH
        );
    }

    #[test]
    fn rebuild_projected_navi_rebuilds_nvmi_edge_links_from_target_nvnm_not_source() {
        // Regression: NVMI.edge_links for
        // the "from source" rebuild path must come from the TARGET (finalized)
        // NAVM's NVNM edge_links table — NOT from the source NVMI. Source
        // can list neighbours that finalize dropped, producing a strict
        // superset of CK's output and tripping CK's PATHFINDING validator.
        //
        // Setup: source NVMI for navmesh 0x000900 lists edge_links
        // [0x000901, 0x000902, 0x000999]. After source->target remap that
        // would become [0x000A01, 0x000A02, ...stale]. But the TARGET NAVM
        // 0x000A00's NVNM only links to 0x000A02 — the 0x000A01 portal got
        // pruned during finalize. The emitted NVMI.edge_links must reflect
        // the NVNM (the 0x000A02 only), not the stale source list.
        let handle_id = create_empty_plugin_handle("Test.esp", Some("fo4"));
        let source_nvmi =
            source_nvmi_with_island(0x000900, 0x000800, &[0x000901, 0x000902, 0x000999]);
        let source_root_items = vec![ParsedItem::Record(source_navi_with_nvmi(
            0x000FF1,
            source_nvmi.clone(),
        ))];

        {
            let mut store = plugin_handle_store_ref().lock().unwrap();
            let slot = store.get_mut(&handle_id).unwrap();
            // Target NAVM 0x000A00: NVNM only edges out to 0x000A02 (the
            // 0x000A01 edge that source predicted no longer exists).
            slot.parsed
                .root_items
                .push(ParsedItem::Record(exterior_navmesh_record_with_edges(
                    0x000A00,
                    0x000B00,
                    (3, -2),
                    &[0x000A02],
                )));
            slot.parsed
                .root_items
                .push(ParsedItem::Record(exterior_navmesh_record_with_edges(
                    0x000A01,
                    0x000B00,
                    (3, -2),
                    &[],
                )));
            slot.parsed
                .root_items
                .push(ParsedItem::Record(exterior_navmesh_record_with_edges(
                    0x000A02,
                    0x000B00,
                    (3, -2),
                    &[],
                )));

            let stats = rebuild_projected_navi_record_from_source_in_slot(
                slot,
                &source_root_items,
                &[
                    (0x000800, 0x000B00),
                    (0x000900, 0x000A00),
                    (0x000901, 0x000A01),
                    (0x000902, 0x000A02),
                ],
                Some(0x000FF1),
            )
            .expect("NAVI rebuild");
            assert_eq!(
                stats.edge_links, 1,
                "edge_links should be NVNM-derived (1), not source-NVMI-derived (3 remapped or 2 mapped)"
            );
        }

        let store = plugin_handle_store_ref().lock().unwrap();
        let slot = store.get(&handle_id).unwrap();
        let navi = first_top_level_record(&slot.parsed.root_items, "NAVI").expect("NAVI");
        let nvmi = navmesh_info_subrecord(navi, 0x000A00);
        // At offset 24 lives the edge_links count. Must be 1, not 2 or 3.
        let edge_count = u32::from_le_bytes(nvmi[24..28].try_into().unwrap());
        assert_eq!(
            edge_count, 1,
            "NVMI.edge_links count must be NVNM-derived (1), not source-derived"
        );
        // The single edge target must be 0x000A02 (the NVNM target), not
        // 0x000A01 or any source-derived value.
        let edge_target = u32::from_le_bytes(nvmi[28..32].try_into().unwrap());
        assert_eq!(
            edge_target, 0x000A02,
            "NVMI.edge_links[0] must be the NVNM-derived target, not a source artifact"
        );
    }

    #[test]
    fn rebuild_projected_navi_uses_fo4_canonical_form_id_with_owned_object_id_collision() {
        let handle_id = create_empty_plugin_handle("Test.esp", Some("fo4"));
        let world_payload = serde_json::json!({
            "signature": "WRLD",
            "form_id": "000800:Test.esp",
            "eid": "TestWorld",
            "subrecords": [
                { "signature": "EDID", "data_hex": "54657374576F726C6400" }
            ]
        });
        let cell_payload = serde_json::json!({
            "signature": "CELL",
            "form_id": "000801:Test.esp",
            "eid": "TestCell",
            "subrecords": [
                { "signature": "EDID", "data_hex": "5465737443656C6C00" },
                { "signature": "XCLC", "data_hex": "03000000FEFFFFFF00000000" }
            ],
            "Landscape": {
                "form_id": "000803:Test.esp",
                "subrecords": []
            }
        });
        let relative_path = "records/WRLD/TestWorld - 000800_Test.esp/0,0/0,0/3,-2/RecordData.yaml";

        plugin_handle_replace_authoring_record_value(handle_id, &world_payload)
            .expect("WRLD import");
        plugin_handle_replace_projected_cell_authoring_record_value(
            handle_id,
            &cell_payload,
            relative_path,
        )
        .expect("projected CELL import");

        {
            let mut store = plugin_handle_store_ref().lock().unwrap();
            let slot = store.get_mut(&handle_id).unwrap();
            slot.parsed.header.masters = vec![
                "Fallout4.esm".into(),
                "DLCRobot.esm".into(),
                "DLCworkshop01.esm".into(),
                "DLCCoast.esm".into(),
                "DLCworkshop02.esm".into(),
                "DLCworkshop03.esm".into(),
                "DLCNukaWorld.esm".into(),
            ];
            insert_parsed_record_in_slot(slot, make_record("REFR", 0x07000FF1, None));
            insert_projected_navmesh_record_in_slot(
                slot,
                exterior_navmesh_record_with_edges(0x000900, 0x000800, (3, -2), &[]),
            )
            .expect("NAVM insert");

            rebuild_projected_navi_record_in_slot(slot, Some(0x0001_4B92)).expect("NAVI rebuild");
            assert_eq!(
                first_top_level_record(&slot.parsed.root_items, "NAVI")
                    .expect("top-level NAVI")
                    .form_id,
                FO4_CANONICAL_NAVI_FORM_ID
            );
            assert!(slot.parsed.header.next_object_id > 0x000FF1);
            assert_eq!(
                find_first_record_form_id_by_signature(&slot.parsed.root_items, "REFR"),
                Some(0x0700_0FF1)
            );
        }
    }

    #[test]
    fn master_remap_preserves_navi_record_form_id() {
        let mut items = vec![
            ParsedItem::Record(make_record("NAVI", 0x000FF1, None)),
            ParsedItem::Record(make_record("TXST", 0x000800, None)),
        ];
        let target_masters = vec![
            "Fallout4.esm".to_string(),
            "DLCRobot.esm".to_string(),
            "DLCworkshop01.esm".to_string(),
            "DLCCoast.esm".to_string(),
            "DLCworkshop02.esm".to_string(),
            "DLCworkshop03.esm".to_string(),
            "DLCNukaWorld.esm".to_string(),
        ];

        remap_formids_in_items(&mut items, &[], &target_masters, 0, 7);

        let ParsedItem::Record(navi) = &items[0] else {
            panic!("expected NAVI record");
        };
        assert_eq!(navi.form_id, 0x000FF1);
        let ParsedItem::Record(txst) = &items[1] else {
            panic!("expected TXST record");
        };
        assert_eq!(txst.form_id, 0x07000800);
    }

    #[test]
    fn add_master_rebases_existing_local_records_and_groups() {
        let handle_id = create_empty_plugin_handle("Test.esp", Some("fo4"));
        {
            let mut store = plugin_handle_store_ref().lock().unwrap();
            let slot = store.get_mut(&handle_id).unwrap();
            slot.parsed.header.masters = vec!["Fallout4.esm".to_string()];
            slot.parsed.header.master_sizes = vec![0];
            slot.parsed.header.overridden_forms = vec![0x01000900];

            let mut world = make_record("WRLD", 0x01000800, Some("TestWorld"));
            world.subrecords.push(formid_subrecord("XLCN", 0x01000801));
            let cell = make_record("CELL", 0x01000801, Some("TestCell"));
            let mut land = make_record("LAND", 0x01000802, None);
            land.subrecords
                .push(land_layer_subrecord("BTXT", 0x01021C68));
            land.subrecords
                .push(land_layer_subrecord("ATXT", 0x01011981));
            slot.parsed.root_items = vec![
                ParsedItem::Record(world),
                ParsedItem::Group(ParsedGroup {
                    label: 0x01000800u32.to_le_bytes(),
                    group_type: 1,
                    tail: Bytes::from(Vec::new()),
                    children: vec![ParsedItem::Record(cell), ParsedItem::Record(land)],
                }),
            ];
        }

        plugin_handle_add_master_native(handle_id, "DLCRobot.esm", None).expect("add master");

        let store = plugin_handle_store_ref().lock().unwrap();
        let slot = store.get(&handle_id).unwrap();
        assert_eq!(
            slot.parsed.header.masters,
            vec!["Fallout4.esm".to_string(), "DLCRobot.esm".to_string()]
        );
        assert_eq!(slot.parsed.header.overridden_forms, vec![0x02000900]);
        let ParsedItem::Record(world) = &slot.parsed.root_items[0] else {
            panic!("expected WRLD record");
        };
        assert_eq!(world.form_id, 0x02000800);
        assert_eq!(
            u32::from_le_bytes(
                world.subrecords.last().unwrap().data[..4]
                    .try_into()
                    .unwrap()
            ),
            0x02000801
        );
        let ParsedItem::Group(group) = &slot.parsed.root_items[1] else {
            panic!("expected world children group");
        };
        assert_eq!(u32::from_le_bytes(group.label), 0x02000800);
        let ParsedItem::Record(cell) = &group.children[0] else {
            panic!("expected CELL record");
        };
        assert_eq!(cell.form_id, 0x02000801);
        let ParsedItem::Record(land) = &group.children[1] else {
            panic!("expected LAND record");
        };
        assert_eq!(land.form_id, 0x02000802);
        assert_eq!(
            u32::from_le_bytes(land.subrecords[0].data[..4].try_into().unwrap()),
            0x02021C68
        );
        assert_eq!(
            u32::from_le_bytes(land.subrecords[1].data[..4].try_into().unwrap()),
            0x02011981
        );
    }

    #[test]
    fn ensure_source_masters_rebases_existing_local_records_on_append() {
        let handle_id = create_empty_plugin_handle("Test.esp", Some("fo4"));
        {
            let mut store = plugin_handle_store_ref().lock().unwrap();
            let slot = store.get_mut(&handle_id).unwrap();
            slot.parsed.header.masters = vec!["Fallout4.esm".to_string()];
            slot.parsed.header.master_sizes = vec![0];
            slot.parsed.root_items.push(ParsedItem::Record(make_record(
                "WRLD",
                0x010025DA,
                Some("TestWorld"),
            )));
        }

        plugin_handle_ensure_source_masters_native(
            handle_id,
            vec![
                "Fallout4.esm".to_string(),
                "DLCRobot.esm".to_string(),
                "DLCworkshop01.esm".to_string(),
            ],
            None,
        )
        .expect("ensure source masters");

        let store = plugin_handle_store_ref().lock().unwrap();
        let slot = store.get(&handle_id).unwrap();
        let ParsedItem::Record(world) = &slot.parsed.root_items[0] else {
            panic!("expected WRLD record");
        };
        assert_eq!(world.form_id, 0x030025DA);
    }

    #[test]
    fn rebuild_projected_navi_skips_preferred_source_object_id_when_reserved() {
        let handle_id = create_empty_plugin_handle("Test.esp", Some("fo4"));
        let world_payload = serde_json::json!({
            "signature": "WRLD",
            "form_id": "000800:Test.esp",
            "eid": "TestWorld",
            "subrecords": [
                { "signature": "EDID", "data_hex": "54657374576F726C6400" }
            ]
        });
        let cell_payload = serde_json::json!({
            "signature": "CELL",
            "form_id": "000801:Test.esp",
            "eid": "TestCell",
            "subrecords": [
                { "signature": "EDID", "data_hex": "5465737443656C6C00" },
                { "signature": "XCLC", "data_hex": "03000000FEFFFFFF00000000" }
            ],
            "Landscape": {
                "form_id": "000803:Test.esp",
                "subrecords": []
            }
        });
        let relative_path = "records/WRLD/TestWorld - 000800_Test.esp/0,0/0,0/3,-2/RecordData.yaml";

        plugin_handle_replace_authoring_record_value(handle_id, &world_payload)
            .expect("WRLD import");
        plugin_handle_replace_projected_cell_authoring_record_value(
            handle_id,
            &cell_payload,
            relative_path,
        )
        .expect("projected CELL import");

        {
            let mut store = plugin_handle_store_ref().lock().unwrap();
            let slot = store.get_mut(&handle_id).unwrap();
            insert_parsed_record_in_slot(
                slot,
                make_record("TXST", 0x000FF1, Some("ReservedTextureSet")),
            );
            insert_projected_navmesh_record_in_slot(
                slot,
                exterior_navmesh_record_with_edges(0x000900, 0x000800, (3, -2), &[]),
            )
            .expect("NAVM insert");

            rebuild_projected_navi_record_in_slot(slot, Some(0x000FF1)).expect("NAVI rebuild");
            assert_ne!(
                first_top_level_record(&slot.parsed.root_items, "NAVI")
                    .expect("top-level NAVI")
                    .form_id,
                0x000FF1
            );
            assert!(
                find_first_record_form_id_by_signature(&slot.parsed.root_items, "TXST").is_some()
            );
        }
    }

    #[test]
    fn navi_island_data_samples_dense_navmesh_for_ck_bounds_limits() {
        let vertices: Vec<(f32, f32, f32)> = (0..600)
            .map(|index| {
                (
                    (index % 30) as f32 * 16.0,
                    (index / 30) as f32 * 16.0,
                    (index % 7) as f32,
                )
            })
            .collect();
        let triangles: Vec<[u16; 3]> = (0..598)
            .map(|index| [index as u16, (index + 1) as u16, (index + 2) as u16])
            .collect();
        let record = exterior_navmesh_record_with_geometry(
            0x000900,
            0x000800,
            (3, -2),
            &vertices,
            &triangles,
            &[],
        );
        let emitted_navmesh_ids = HashSet::from([0x000900]);
        let mut stats = NaviRebuildStats::default();

        let info = navmesh_info_input_from_record(&record, &emitted_navmesh_ids, &mut stats)
            .expect("NAVM parsed")
            .expect("NAVI info");
        let island = info.island_data.expect("island data");

        assert_eq!(island.min, (0.0, 0.0, 0.0));
        assert_eq!(island.max, (464.0, 304.0, 6.0));
        assert!(!island.triangles.is_empty());
        assert!(!island.vertices.is_empty());
        assert!(island.triangles.len() <= NAVI_ISLAND_TRIANGLE_LIMIT);
        assert!(island.vertices.len() <= NAVI_ISLAND_VERTEX_LIMIT);
        for triangle in &island.triangles {
            for vertex_index in triangle {
                assert!((*vertex_index as usize) < island.vertices.len());
            }
        }
        assert_eq!(stats.warnings, 0);
    }

    #[test]
    fn projected_cell_import_replaces_existing_cell_at_same_location() {
        let handle_id = create_empty_plugin_handle("Test.esp", Some("fo4"));
        let world_payload = serde_json::json!({
            "signature": "WRLD",
            "form_id": "000800:Test.esp",
            "eid": "TestWorld",
            "subrecords": [
                { "signature": "EDID", "data_hex": "54657374576F726C6400" }
            ]
        });
        let first_payload = serde_json::json!({
            "signature": "CELL",
            "form_id": "000801:Test.esp",
            "eid": "TestCell",
            "subrecords": [
                { "signature": "EDID", "data_hex": "5465737443656C6C00" },
                { "signature": "XCLC", "data_hex": "03000000FEFFFFFF" }
            ],
            "Landscape": {
                "form_id": "000802:Test.esp",
                "subrecords": []
            }
        });
        let second_payload = serde_json::json!({
            "signature": "CELL",
            "form_id": "000901:Test.esp",
            "eid": "TestCellRegen",
            "subrecords": [
                { "signature": "EDID", "data_hex": "5465737443656C6C526567656E00" },
                { "signature": "XCLC", "data_hex": "03000000FEFFFFFF" }
            ],
            "Landscape": {
                "form_id": "000902:Test.esp",
                "subrecords": []
            }
        });
        let relative_path = "records/WRLD/TestWorld - 000800_Test.esp/0,0/0,0/3,-2/RecordData.yaml";

        plugin_handle_replace_authoring_record_value(handle_id, &world_payload)
            .expect("WRLD import");
        plugin_handle_replace_projected_cell_authoring_record_value(
            handle_id,
            &first_payload,
            relative_path,
        )
        .expect("first projected CELL import");
        plugin_handle_replace_projected_cell_authoring_record_value(
            handle_id,
            &second_payload,
            relative_path,
        )
        .expect("second projected CELL import");

        let store = plugin_handle_store_ref().lock().unwrap();
        let slot = store.get(&handle_id).unwrap();
        let wrld_group = slot
            .parsed
            .root_items
            .iter()
            .find_map(|item| match item {
                ParsedItem::Group(group) if group.group_type == 0 && group.label == *b"WRLD" => {
                    Some(group)
                }
                _ => None,
            })
            .expect("WRLD top group");
        let world_children = wrld_group
            .children
            .iter()
            .find_map(|item| match item {
                ParsedItem::Group(group) if group.group_type == 1 => Some(group),
                _ => None,
            })
            .expect("WRLD children group");
        let block_group = world_children
            .children
            .iter()
            .find_map(|item| match item {
                ParsedItem::Group(group)
                    if group.group_type == EXTERIOR_CELL_BLOCK
                        && group.label == encode_exterior_grid_label(0, 0) =>
                {
                    Some(group)
                }
                _ => None,
            })
            .expect("target exterior block group");
        let subblock_group = block_group
            .children
            .iter()
            .find_map(|item| match item {
                ParsedItem::Group(group)
                    if group.group_type == EXTERIOR_CELL_SUBBLOCK
                        && group.label == encode_exterior_grid_label(0, 0) =>
                {
                    Some(group)
                }
                _ => None,
            })
            .expect("target exterior subblock group");

        let cell_records: Vec<&ParsedRecord> = subblock_group
            .children
            .iter()
            .filter_map(|item| match item {
                ParsedItem::Record(record) if record.signature.as_str() == "CELL" => Some(record),
                _ => None,
            })
            .collect();
        assert_eq!(cell_records.len(), 1);
        assert_eq!(cell_records[0].form_id, 0x000901);
        assert_eq!(
            projected_cell_grid_from_record(cell_records[0]),
            Some((3, -2))
        );

        let child_groups: Vec<&ParsedGroup> = subblock_group
            .children
            .iter()
            .filter_map(|item| match item {
                ParsedItem::Group(group) if group.group_type == CELL_CHILD_GROUP => Some(group),
                _ => None,
            })
            .collect();
        assert_eq!(child_groups.len(), 1);
        let child_group = child_groups[0];
        assert_eq!(child_group.label, 0x000901u32.to_le_bytes());
        assert!(matches!(
            child_group.children.first(),
            Some(ParsedItem::Record(record))
                if record.signature.as_str() == "LAND" && record.form_id == 0x000902
        ));

        assert!(
            !subblock_group.children.iter().any(|item| {
                matches!(item, ParsedItem::Record(record) if record.form_id == 0x000801)
            }),
            "old projected CELL should be removed",
        );
        assert!(
            !subblock_group.children.iter().any(|item| {
                matches!(item, ParsedItem::Group(group) if group.group_type == CELL_CHILD_GROUP && group.label == 0x000801u32.to_le_bytes())
            }),
            "old projected CELL child group should be removed",
        );
    }

    #[test]
    fn projected_cell_batch_import_replaces_cells_by_grid_location() {
        let handle_id = create_empty_plugin_handle("Test.esp", Some("fo4"));
        let world_payload = serde_json::json!({
            "signature": "WRLD",
            "form_id": "000800:Test.esp",
            "eid": "TestWorld",
            "subrecords": [
                { "signature": "EDID", "data_hex": "54657374576F726C6400" }
            ]
        });
        let first_payload = serde_json::json!({
            "signature": "CELL",
            "form_id": "000801:Test.esp",
            "eid": "TestCell",
            "subrecords": [
                { "signature": "EDID", "data_hex": "5465737443656C6C00" },
                { "signature": "XCLC", "data_hex": "03000000FEFFFFFF" }
            ],
            "Landscape": {
                "form_id": "000802:Test.esp",
                "subrecords": []
            }
        });
        let replacement_payload = serde_json::json!({
            "signature": "CELL",
            "form_id": "000901:Test.esp",
            "eid": "TestCellRegen",
            "subrecords": [
                { "signature": "EDID", "data_hex": "5465737443656C6C526567656E00" },
                { "signature": "XCLC", "data_hex": "03000000FEFFFFFF" }
            ],
            "Landscape": {
                "form_id": "000902:Test.esp",
                "subrecords": []
            }
        });
        let relative_path = "records/WRLD/TestWorld - 000800_Test.esp/0,0/0,0/3,-2/RecordData.yaml";

        plugin_handle_replace_authoring_record_value(handle_id, &world_payload)
            .expect("WRLD import");
        let imported = plugin_handle_replace_projected_cell_authoring_record_values_at_locations(
            handle_id,
            vec![
                (first_payload, relative_path.to_string()),
                (replacement_payload, relative_path.to_string()),
            ],
        )
        .expect("projected CELL batch import");

        assert_eq!(imported, 4);
        let store = plugin_handle_store_ref().lock().unwrap();
        let slot = store.get(&handle_id).unwrap();
        let wrld_group = slot
            .parsed
            .root_items
            .iter()
            .find_map(|item| match item {
                ParsedItem::Group(group) if group.group_type == 0 && group.label == *b"WRLD" => {
                    Some(group)
                }
                _ => None,
            })
            .expect("WRLD top group");
        let world_children = wrld_group
            .children
            .iter()
            .find_map(|item| match item {
                ParsedItem::Group(group) if group.group_type == 1 => Some(group),
                _ => None,
            })
            .expect("WRLD children group");
        let block_group = world_children
            .children
            .iter()
            .find_map(|item| match item {
                ParsedItem::Group(group)
                    if group.group_type == EXTERIOR_CELL_BLOCK
                        && group.label == encode_exterior_grid_label(0, 0) =>
                {
                    Some(group)
                }
                _ => None,
            })
            .expect("target exterior block group");
        let subblock_group = block_group
            .children
            .iter()
            .find_map(|item| match item {
                ParsedItem::Group(group)
                    if group.group_type == EXTERIOR_CELL_SUBBLOCK
                        && group.label == encode_exterior_grid_label(0, 0) =>
                {
                    Some(group)
                }
                _ => None,
            })
            .expect("target exterior subblock group");

        let cell_records: Vec<&ParsedRecord> = subblock_group
            .children
            .iter()
            .filter_map(|item| match item {
                ParsedItem::Record(record) if record.signature.as_str() == "CELL" => Some(record),
                _ => None,
            })
            .collect();
        assert_eq!(cell_records.len(), 1);
        assert_eq!(cell_records[0].form_id, 0x000901);
        assert!(
            !subblock_group.children.iter().any(
                |item| matches!(item, ParsedItem::Record(record) if record.form_id == 0x000801)
            )
        );
    }

    #[test]
    fn projected_cell_batch_import_reuses_existing_cell_and_land_ids() {
        let handle_id = create_empty_plugin_handle("Test.esp", Some("fo4"));
        let world_payload = serde_json::json!({
            "signature": "WRLD",
            "form_id": "000800:Test.esp",
            "eid": "TestWorld",
            "subrecords": [
                { "signature": "EDID", "data_hex": "54657374576F726C6400" }
            ]
        });
        plugin_handle_replace_authoring_record_value(handle_id, &world_payload)
            .expect("WRLD import");

        {
            let mut existing_cell = make_record("CELL", 0x18D2755, Some("TestWorldCellXP003YN002"));
            existing_cell.subrecords.push(ParsedSubrecord {
                signature: SmolStr::new_static("XCLC"),
                data: Bytes::from_static(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
                semantic_type: None,
            });
            let existing_land = make_record("LAND", 0x18D2756, None);
            let existing_ref = make_record("REFR", 0x18D2757, Some("PlacedRef"));
            let mut store = plugin_handle_store_ref().lock().unwrap();
            let slot = store.get_mut(&handle_id).unwrap();
            let wrld_group = slot
                .parsed
                .root_items
                .iter_mut()
                .find_map(|item| match item {
                    ParsedItem::Group(group)
                        if group.group_type == 0 && group.label == *b"WRLD" =>
                    {
                        Some(group)
                    }
                    _ => None,
                })
                .expect("WRLD top group");
            let world_children =
                ensure_world_children_group(&mut wrld_group.children, 0x800, MODERN_HEADER_SIZE);
            let block_group = ensure_exterior_grid_group(
                &mut world_children.children,
                EXTERIOR_CELL_BLOCK,
                (0, 0),
                MODERN_HEADER_SIZE,
            );
            let subblock_group = ensure_exterior_grid_group(
                &mut block_group.children,
                EXTERIOR_CELL_SUBBLOCK,
                (0, 0),
                MODERN_HEADER_SIZE,
            );
            subblock_group
                .children
                .push(ParsedItem::Record(existing_cell));
            subblock_group.children.push(ParsedItem::Group(ParsedGroup {
                label: 0x18D2755u32.to_le_bytes(),
                group_type: CELL_CHILD_GROUP,
                tail: Bytes::from(vec![0u8; MODERN_HEADER_SIZE - 16]),
                children: vec![
                    ParsedItem::Record(existing_land),
                    ParsedItem::Group(ParsedGroup {
                        label: 0x18D2755u32.to_le_bytes(),
                        group_type: TEMPORARY_GROUP,
                        tail: Bytes::from(vec![0u8; MODERN_HEADER_SIZE - 16]),
                        children: vec![ParsedItem::Record(existing_ref)],
                    }),
                ],
            }));
        }

        let projected_payload = serde_json::json!({
            "signature": "CELL",
            "form_id": "190000:Test.esp",
            "eid": "TestWorldCellXP003YN002",
            "subrecords": [
                { "signature": "EDID", "data_hex": "54657374576F726C6443656C6C5850303033594E30303200" },
                { "signature": "XCLC", "data_hex": "03000000FEFFFFFF00000000" }
            ],
            "Landscape": {
                "form_id": "190001:Test.esp",
                "subrecords": []
            }
        });
        let relative_path = "records/WRLD/TestWorld - 000800_Test.esp/0,0/0,0/3,-2/RecordData.yaml";

        plugin_handle_replace_projected_cell_authoring_record_values_at_locations(
            handle_id,
            vec![(projected_payload, relative_path.to_string())],
        )
        .expect("projected CELL batch import");

        let store = plugin_handle_store_ref().lock().unwrap();
        let slot = store.get(&handle_id).unwrap();
        let wrld_group = slot
            .parsed
            .root_items
            .iter()
            .find_map(|item| match item {
                ParsedItem::Group(group) if group.group_type == 0 && group.label == *b"WRLD" => {
                    Some(group)
                }
                _ => None,
            })
            .expect("WRLD top group");
        let world_children = wrld_group
            .children
            .iter()
            .find_map(|item| match item {
                ParsedItem::Group(group) if group.group_type == 1 => Some(group),
                _ => None,
            })
            .expect("WRLD children group");
        let block_group = world_children
            .children
            .iter()
            .find_map(|item| match item {
                ParsedItem::Group(group)
                    if group.group_type == EXTERIOR_CELL_BLOCK
                        && group.label == encode_exterior_grid_label(0, 0) =>
                {
                    Some(group)
                }
                _ => None,
            })
            .expect("target exterior block group");
        let subblock_group = block_group
            .children
            .iter()
            .find_map(|item| match item {
                ParsedItem::Group(group)
                    if group.group_type == EXTERIOR_CELL_SUBBLOCK
                        && group.label == encode_exterior_grid_label(0, 0) =>
                {
                    Some(group)
                }
                _ => None,
            })
            .expect("target exterior subblock group");
        let cell = subblock_group
            .children
            .iter()
            .find_map(|item| match item {
                ParsedItem::Record(record) if record.signature.as_str() == "CELL" => Some(record),
                _ => None,
            })
            .expect("merged CELL");
        assert_eq!(cell.form_id, 0x18D2755);
        assert_eq!(projected_cell_grid_from_record(cell), Some((3, -2)));

        let child_group = subblock_group
            .children
            .iter()
            .find_map(|item| match item {
                ParsedItem::Group(group) if group.group_type == CELL_CHILD_GROUP => Some(group),
                _ => None,
            })
            .expect("merged CELL child group");
        assert_eq!(child_group.label, 0x18D2755u32.to_le_bytes());
        assert!(child_group.children.iter().any(|item| {
            matches!(item, ParsedItem::Record(record) if record.signature.as_str() == "LAND" && record.form_id == 0x18D2756)
        }));
        assert!(child_group.children.iter().any(|item| {
            matches!(item, ParsedItem::Group(group) if group.group_type == TEMPORARY_GROUP)
        }));
    }

    #[test]
    fn projected_cell_batch_import_moves_existing_cell_from_wrong_subblock_by_editor_id() {
        let handle_id = create_empty_plugin_handle("Test.esp", Some("fo4"));
        let world_payload = serde_json::json!({
            "signature": "WRLD",
            "form_id": "000800:Test.esp",
            "eid": "TestWorld",
            "subrecords": [
                { "signature": "EDID", "data_hex": "54657374576F726C6400" }
            ]
        });
        plugin_handle_replace_authoring_record_value(handle_id, &world_payload)
            .expect("WRLD import");

        {
            let mut existing_cell = make_record("CELL", 0x18D2755, Some("TestWorldCellXN100YN100"));
            existing_cell.subrecords.push(ParsedSubrecord {
                signature: SmolStr::new_static("XCLC"),
                data: Bytes::from_static(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
                semantic_type: None,
            });
            let existing_land = make_record("LAND", 0x18D2756, None);
            let existing_ref = make_record("REFR", 0x18D2757, Some("PlacedRef"));
            let mut store = plugin_handle_store_ref().lock().unwrap();
            let slot = store.get_mut(&handle_id).unwrap();
            let wrld_group = slot
                .parsed
                .root_items
                .iter_mut()
                .find_map(|item| match item {
                    ParsedItem::Group(group)
                        if group.group_type == 0 && group.label == *b"WRLD" =>
                    {
                        Some(group)
                    }
                    _ => None,
                })
                .expect("WRLD top group");
            let world_children =
                ensure_world_children_group(&mut wrld_group.children, 0x800, MODERN_HEADER_SIZE);
            let block_group = ensure_exterior_grid_group(
                &mut world_children.children,
                EXTERIOR_CELL_BLOCK,
                (0, 0),
                MODERN_HEADER_SIZE,
            );
            let subblock_group = ensure_exterior_grid_group(
                &mut block_group.children,
                EXTERIOR_CELL_SUBBLOCK,
                (0, 0),
                MODERN_HEADER_SIZE,
            );
            subblock_group
                .children
                .push(ParsedItem::Record(existing_cell));
            subblock_group.children.push(ParsedItem::Group(ParsedGroup {
                label: 0x18D2755u32.to_le_bytes(),
                group_type: CELL_CHILD_GROUP,
                tail: Bytes::from(vec![0u8; MODERN_HEADER_SIZE - 16]),
                children: vec![
                    ParsedItem::Record(existing_land),
                    ParsedItem::Group(ParsedGroup {
                        label: 0x18D2755u32.to_le_bytes(),
                        group_type: TEMPORARY_GROUP,
                        tail: Bytes::from(vec![0u8; MODERN_HEADER_SIZE - 16]),
                        children: vec![ParsedItem::Record(existing_ref)],
                    }),
                ],
            }));
        }

        let projected_payload = serde_json::json!({
            "signature": "CELL",
            "form_id": "190000:Test.esp",
            "eid": "TestWorldCellXN100YN100",
            "subrecords": [
                { "signature": "EDID", "data_hex": "54657374576F726C6443656C6C584E313030594E31303000" },
                { "signature": "XCLC", "data_hex": "9CFFFFFF9CFFFFFF00000000" }
            ],
            "Landscape": {
                "form_id": "190001:Test.esp",
                "subrecords": []
            }
        });
        let relative_path =
            "records/WRLD/TestWorld - 000800_Test.esp/-4,-4/-13,-13/-100,-100/RecordData.yaml";

        plugin_handle_replace_projected_cell_authoring_record_values_at_locations(
            handle_id,
            vec![(projected_payload, relative_path.to_string())],
        )
        .expect("projected CELL batch import");

        let store = plugin_handle_store_ref().lock().unwrap();
        let slot = store.get(&handle_id).unwrap();
        let wrld_group = slot
            .parsed
            .root_items
            .iter()
            .find_map(|item| match item {
                ParsedItem::Group(group) if group.group_type == 0 && group.label == *b"WRLD" => {
                    Some(group)
                }
                _ => None,
            })
            .expect("WRLD top group");
        let world_children = wrld_group
            .children
            .iter()
            .find_map(|item| match item {
                ParsedItem::Group(group) if group.group_type == 1 => Some(group),
                _ => None,
            })
            .expect("WRLD children group");
        let old_subblock_has_cell = world_children
            .children
            .iter()
            .find_map(|item| match item {
                ParsedItem::Group(group)
                    if group.group_type == EXTERIOR_CELL_BLOCK
                        && group.label == encode_exterior_grid_label(0, 0) =>
                {
                    Some(group)
                }
                _ => None,
            })
            .and_then(|block| {
                block.children.iter().find_map(|item| match item {
                    ParsedItem::Group(group)
                        if group.group_type == EXTERIOR_CELL_SUBBLOCK
                            && group.label == encode_exterior_grid_label(0, 0) =>
                    {
                        Some(group)
                    }
                    _ => None,
                })
            })
            .is_some_and(|subblock| {
                subblock.children.iter().any(
                    |item| matches!(item, ParsedItem::Record(record) if record.form_id == 0x18D2755),
                )
            });
        assert!(!old_subblock_has_cell);

        let target_block = world_children
            .children
            .iter()
            .find_map(|item| match item {
                ParsedItem::Group(group)
                    if group.group_type == EXTERIOR_CELL_BLOCK
                        && group.label == encode_exterior_grid_label(-4, -4) =>
                {
                    Some(group)
                }
                _ => None,
            })
            .expect("target exterior block group");
        let target_subblock = target_block
            .children
            .iter()
            .find_map(|item| match item {
                ParsedItem::Group(group)
                    if group.group_type == EXTERIOR_CELL_SUBBLOCK
                        && group.label == encode_exterior_grid_label(-13, -13) =>
                {
                    Some(group)
                }
                _ => None,
            })
            .expect("target exterior subblock group");
        let cell = target_subblock
            .children
            .iter()
            .find_map(|item| match item {
                ParsedItem::Record(record) if record.signature.as_str() == "CELL" => Some(record),
                _ => None,
            })
            .expect("moved CELL");
        assert_eq!(cell.form_id, 0x18D2755);
        assert_eq!(projected_cell_grid_from_record(cell), Some((-100, -100)));

        let child_group = target_subblock
            .children
            .iter()
            .find_map(|item| match item {
                ParsedItem::Group(group) if group.group_type == CELL_CHILD_GROUP => Some(group),
                _ => None,
            })
            .expect("moved CELL child group");
        assert_eq!(child_group.label, 0x18D2755u32.to_le_bytes());
        assert!(child_group.children.iter().any(|item| {
            matches!(item, ParsedItem::Record(record) if record.signature.as_str() == "LAND" && record.form_id == 0x18D2756)
        }));
        assert!(child_group.children.iter().any(|item| {
            matches!(item, ParsedItem::Group(group) if group.group_type == TEMPORARY_GROUP)
        }));
    }

    #[test]
    fn parse_grid_dir_name_accepts_optional_space_after_comma() {
        assert_eq!(parse_grid_dir_name("3,-2"), Some((3, -2)));
        assert_eq!(parse_grid_dir_name("3, -2"), Some((3, -2)));
    }

    mod invalidation_classification_tests {
        use super::*;

        #[test]
        fn insert_parsed_record_invalidates_all_sections() {
            let handle_id = create_empty_plugin_handle("Test.esp", Some("fo4"));
            populate_all_sections(handle_id);

            insert_parsed_record(handle_id, make_record("WEAP", 0xFF000800, Some("Inserted")))
                .unwrap();

            let store = plugin_handle_store_ref().lock().unwrap();
            let slot = store.get(&handle_id).unwrap();
            assert!(slot.sections.locator.is_none());
            assert!(slot.sections.core.is_none());
            assert!(slot.sections.records.is_none());
            assert!(slot.sections.form_id_paths.is_none());
            assert!(slot.sections.refs.is_none());
            assert!(slot.sections.assets.is_none());
        }

        #[test]
        fn update_saved_path_preserves_sections_when_plugin_name_is_stable() {
            let handle_id = create_empty_plugin_handle("Test.esp", Some("fo4"));
            populate_all_sections(handle_id);

            update_plugin_handle_saved_path(handle_id, r"C:\mods\Test.esp");

            let store = plugin_handle_store_ref().lock().unwrap();
            let slot = store.get(&handle_id).unwrap();
            assert_eq!(slot.parsed.plugin_name, "Test.esp");
            assert_eq!(slot.parsed.file_path, r"C:\mods\Test.esp");
            assert!(slot.sections.locator.is_some());
            assert!(slot.sections.core.is_some());
            assert!(slot.sections.records.is_some());
            assert!(slot.sections.form_id_paths.is_some());
            assert!(slot.sections.refs.is_some());
            assert!(slot.sections.assets.is_some());
        }

        #[test]
        fn update_saved_path_invalidates_when_plugin_name_changes() {
            let handle_id = create_empty_plugin_handle("Test.esp", Some("fo4"));
            populate_all_sections(handle_id);

            update_plugin_handle_saved_path(handle_id, r"C:\mods\Renamed.esp");

            let store = plugin_handle_store_ref().lock().unwrap();
            let slot = store.get(&handle_id).unwrap();
            assert_eq!(slot.parsed.plugin_name, "Renamed.esp");
            assert!(slot.sections.locator.is_none());
            assert!(slot.sections.core.is_none());
            assert!(slot.sections.records.is_none());
            assert!(slot.sections.form_id_paths.is_none());
            assert!(slot.sections.refs.is_none());
            assert!(slot.sections.assets.is_none());
        }

        #[test]
        fn snapshot_save_preserves_plugin_identity() {
            let handle_id = create_empty_plugin_handle("Test.esp", Some("fo4"));
            let temp = tempfile::tempdir().unwrap();
            let snapshot_path = temp.path().join("Snapshot.esp.tmp");

            plugin_handle_save_preserving_identity_no_py(
                handle_id,
                snapshot_path.to_str().unwrap(),
            )
            .unwrap();

            let store = plugin_handle_store_ref().lock().unwrap();
            let slot = store.get(&handle_id).unwrap();
            assert_eq!(slot.parsed.plugin_name, "Test.esp");
            assert_eq!(slot.parsed.file_path, "");
            assert!(snapshot_path.is_file());
        }

        #[test]
        fn patch_subrecord_bytes_preserves_form_id_index() {
            let mut record = make_record("WEAP", 0xFF000800, Some("PatchedWeap"));
            record.subrecords.push(ParsedSubrecord {
                signature: SmolStr::new("DNAM"),
                data: Bytes::from_static(&[0x00, 0x01, 0x02, 0x03]),
                semantic_type: None,
            });

            let mut plugin = empty_plugin(Some("fo4"));
            plugin.root_items.push(ParsedItem::Record(record));
            let handle_id = insert_plugin_handle(plugin, LocalizedStringsState::default());
            populate_all_sections(handle_id);

            let changed =
                patch_record_subrecord_bytes(handle_id, "Test.esp:000800", "DNAM", |bytes| {
                    bytes[0] = 0xAB;
                    true
                })
                .unwrap();
            assert!(changed);

            let store = plugin_handle_store_ref().lock().unwrap();
            let slot = store.get(&handle_id).unwrap();
            assert!(slot.sections.locator.is_some());
            assert!(slot.sections.core.is_some());
            assert!(slot.sections.records.is_some());
            assert!(slot.sections.form_id_paths.is_some());
            assert!(slot.sections.refs.is_none());
            assert!(slot.sections.assets.is_none());
        }

        #[test]
        fn patch_subrecord_bytes_edid_change_invalidates_core_indexes() {
            let record = make_record("WEAP", 0xFF000800, Some("PatchedWeap"));

            let mut plugin = empty_plugin(Some("fo4"));
            plugin.root_items.push(ParsedItem::Record(record));
            let handle_id = insert_plugin_handle(plugin, LocalizedStringsState::default());
            populate_all_sections(handle_id);

            let changed =
                patch_record_subrecord_bytes(handle_id, "Test.esp:000800", "EDID", |bytes| {
                    bytes[0] = b'X';
                    true
                })
                .unwrap();
            assert!(changed);

            let store = plugin_handle_store_ref().lock().unwrap();
            let slot = store.get(&handle_id).unwrap();
            assert!(slot.sections.locator.is_none());
            assert!(slot.sections.core.is_none());
            assert!(slot.sections.records.is_none());
            assert!(slot.sections.form_id_paths.is_none());
            assert!(slot.sections.refs.is_none());
            assert!(slot.sections.assets.is_none());
        }

        #[test]
        fn replace_parsed_record_contents_preserves_runtime_indexes() {
            let mut record = make_record("WEAP", 0xFF000800, Some("PatchedWeap"));
            record.raw_payload = Some(Bytes::from_static(b"stale"));

            let mut plugin = empty_plugin(Some("fo4"));
            plugin.root_items.push(ParsedItem::Record(record));
            let handle_id = insert_plugin_handle(plugin, LocalizedStringsState::default());
            populate_all_sections(handle_id);

            let mut replacement = make_record("WEAP", 0xFF000800, Some("PatchedWeap"));
            replacement.subrecords.push(ParsedSubrecord {
                signature: SmolStr::new("DNAM"),
                data: Bytes::from_static(&[0xAA, 0xBB]),
                semantic_type: None,
            });

            let mut store = plugin_handle_store_ref().lock().unwrap();
            let slot = store.get_mut(&handle_id).expect("plugin handle present");
            assert!(replace_parsed_record_contents_in_slot(slot, replacement));

            let ParsedItem::Record(updated) = &slot.parsed.root_items[0] else {
                panic!("expected root record");
            };
            assert!(updated.raw_payload.is_none());
            assert!(updated.parse_error.is_none());
            assert_eq!(updated.subrecords.len(), 2);
            assert_eq!(updated.subrecords[1].signature.as_str(), "DNAM");
            assert_eq!(updated.subrecords[1].data.as_ref(), &[0xAA, 0xBB]);
            assert!(slot.sections.locator.is_some());
            assert!(slot.sections.core.is_some());
            assert!(slot.sections.records.is_some());
            assert!(slot.sections.form_id_paths.is_some());
            assert!(slot.sections.refs.is_none());
            assert!(slot.sections.assets.is_none());
        }

        #[test]
        fn replace_parsed_record_contents_rejects_missing_record() {
            let mut plugin = empty_plugin(Some("fo4"));
            plugin.root_items.push(ParsedItem::Record(make_record(
                "WEAP",
                0xFF000800,
                Some("Weap"),
            )));
            let handle_id = insert_plugin_handle(plugin, LocalizedStringsState::default());
            populate_all_sections(handle_id);

            let replacement = make_record("WEAP", 0xFF000801, Some("OtherWeap"));

            let mut store = plugin_handle_store_ref().lock().unwrap();
            let slot = store.get_mut(&handle_id).expect("plugin handle present");
            assert!(!replace_parsed_record_contents_in_slot(slot, replacement));
            assert!(slot.sections.core.is_some());
            assert!(slot.sections.form_id_paths.is_some());
        }

        #[test]
        fn replace_parsed_record_contents_rejects_signature_mismatch() {
            let mut plugin = empty_plugin(Some("fo4"));
            plugin.root_items.push(ParsedItem::Record(make_record(
                "WEAP",
                0xFF000800,
                Some("Weap"),
            )));
            let handle_id = insert_plugin_handle(plugin, LocalizedStringsState::default());
            populate_all_sections(handle_id);

            let replacement = make_record("AMMO", 0xFF000800, Some("Ammo"));

            let mut store = plugin_handle_store_ref().lock().unwrap();
            let slot = store.get_mut(&handle_id).expect("plugin handle present");
            assert!(!replace_parsed_record_contents_in_slot(slot, replacement));

            let ParsedItem::Record(existing) = &slot.parsed.root_items[0] else {
                panic!("expected root record");
            };
            assert_eq!(existing.signature.as_str(), "WEAP");
            assert!(slot.sections.core.is_some());
            assert!(slot.sections.form_id_paths.is_some());
        }

        #[test]
        fn replace_parsed_record_contents_rejects_edid_mismatch() {
            let mut plugin = empty_plugin(Some("fo4"));
            plugin.root_items.push(ParsedItem::Record(make_record(
                "WEAP",
                0xFF000800,
                Some("Weap"),
            )));
            let handle_id = insert_plugin_handle(plugin, LocalizedStringsState::default());
            populate_all_sections(handle_id);

            let mut replacement = make_record("WEAP", 0xFF000800, Some("OtherWeap"));
            replacement.subrecords.push(ParsedSubrecord {
                signature: SmolStr::new("DNAM"),
                data: Bytes::from_static(&[0xAA, 0xBB]),
                semantic_type: None,
            });

            let mut store = plugin_handle_store_ref().lock().unwrap();
            let slot = store.get_mut(&handle_id).expect("plugin handle present");
            assert!(!replace_parsed_record_contents_in_slot(slot, replacement));

            let ParsedItem::Record(existing) = &slot.parsed.root_items[0] else {
                panic!("expected root record");
            };
            assert_eq!(record_editor_id_value(existing).as_deref(), Some("Weap"));
            assert_eq!(existing.subrecords.len(), 1);
            assert!(slot.sections.core.is_some());
            assert!(slot.sections.form_id_paths.is_some());
        }

        #[test]
        fn replace_parsed_record_contents_rejects_flag_mismatch() {
            let mut plugin = empty_plugin(Some("fo4"));
            let mut record = make_record("WEAP", 0xFF000800, Some("Weap"));
            record.flags = 0x0000_0001;
            plugin.root_items.push(ParsedItem::Record(record));
            let handle_id = insert_plugin_handle(plugin, LocalizedStringsState::default());
            populate_all_sections(handle_id);

            let mut replacement = make_record("WEAP", 0xFF000800, Some("Weap"));
            replacement.flags = 0x0000_0002;
            replacement.subrecords.push(ParsedSubrecord {
                signature: SmolStr::new("DNAM"),
                data: Bytes::from_static(&[0xAA, 0xBB]),
                semantic_type: None,
            });

            let mut store = plugin_handle_store_ref().lock().unwrap();
            let slot = store.get_mut(&handle_id).expect("plugin handle present");
            assert!(!replace_parsed_record_contents_in_slot(slot, replacement));

            let ParsedItem::Record(existing) = &slot.parsed.root_items[0] else {
                panic!("expected root record");
            };
            assert_eq!(existing.flags, 0x0000_0001);
            assert_eq!(existing.subrecords.len(), 1);
            assert!(slot.sections.core.is_some());
            assert!(slot.sections.form_id_paths.is_some());
        }

        #[test]
        fn replace_parsed_record_contents_allows_compressed_bit_only_flag_delta() {
            // Re-encoding a CELL/LAND deterministically sets the COMPRESSED storage
            // bit; an in-place content swap must still apply when that is the ONLY
            // flag difference (the record keeps its own flags — the body is what
            // changes). Without masking this bit, XEZN stamping every footprint cell
            // would silently no-op.
            let mut plugin = empty_plugin(Some("fo4"));
            let mut record = make_record("CELL", 0xFF000800, Some("Cell"));
            record.flags = 0x0000_0001; // uncompressed in the tree
            plugin.root_items.push(ParsedItem::Record(record));
            let handle_id = insert_plugin_handle(plugin, LocalizedStringsState::default());
            populate_all_sections(handle_id);

            let mut replacement = make_record("CELL", 0xFF000800, Some("Cell"));
            replacement.flags = 0x0000_0001 | COMPRESSED_RECORD_FLAG; // re-encoded ⇒ compressed
            replacement.subrecords.push(ParsedSubrecord {
                signature: SmolStr::new("XEZN"),
                data: Bytes::from_static(&[0x01, 0x02, 0x03, 0x04]),
                semantic_type: None,
            });

            let mut store = plugin_handle_store_ref().lock().unwrap();
            let slot = store.get_mut(&handle_id).expect("plugin handle present");
            assert!(replace_parsed_record_contents_in_slot(slot, replacement));

            let ParsedItem::Record(updated) = &slot.parsed.root_items[0] else {
                panic!("expected root record");
            };
            assert_eq!(updated.subrecords.len(), 2);
            assert_eq!(updated.subrecords[1].signature.as_str(), "XEZN");
            // Content-replace never rewrites flags: the masked compressed bit is not
            // adopted, the record keeps its own header flags.
            assert_eq!(updated.flags, 0x0000_0001);
        }
    }

    fn make_bool_enum() -> SchemaEnumJson {
        SchemaEnumJson {
            id: "bool_enum".to_string(),
            values: vec![
                SchemaEnumValueJson {
                    value: 0,
                    id: "false".to_string(),
                },
                SchemaEnumValueJson {
                    value: 1,
                    id: "true".to_string(),
                },
            ],
            labels: vec![
                SchemaEnumLabelJson {
                    value: 0,
                    label: "False".to_string(),
                },
                SchemaEnumLabelJson {
                    value: 1,
                    label: "True".to_string(),
                },
            ],
            aliases: Vec::new(),
            scope: "scoped".to_string(),
            storage_kind: "enum".to_string(),
            byte_width: 1,
            default_value: None,
        }
    }

    fn make_flag_enum() -> SchemaEnumJson {
        SchemaEnumJson {
            id: "WEAP.DNAM.flags".to_string(),
            values: vec![
                SchemaEnumValueJson {
                    value: 256,
                    id: "crit_effect_on_death".to_string(),
                },
                SchemaEnumValueJson {
                    value: 4_194_304,
                    id: "bolt_action".to_string(),
                },
            ],
            labels: vec![
                SchemaEnumLabelJson {
                    value: 256,
                    label: "Crit Effect - on Death".to_string(),
                },
                SchemaEnumLabelJson {
                    value: 4_194_304,
                    label: "Bolt Action".to_string(),
                },
            ],
            aliases: vec![SchemaEnumAliasJson {
                legacy_id: "CritEffectOnDeath".to_string(),
                id: "crit_effect_on_death".to_string(),
            }],
            scope: "scoped".to_string(),
            storage_kind: "flags".to_string(),
            byte_width: 4,
            default_value: None,
        }
    }

    fn make_vmad_subrecord_spec() -> SchemaSubrecordJson {
        SchemaSubrecordJson {
            id: "VMAD".to_string(),
            kind: "parsed".to_string(),
            display_label: Some("Virtual Machine Adapter".to_string()),
            codec: Some("struct:h,h".to_string()),
            fields: Vec::new(),
            repeatable: false,
            required: false,
            localized: false,
            enum_ref: None,
            formlink_target: None,
            formlink_targets: Vec::new(),
            null_allowed: false,
            union_selector: None,
            union_variants: Vec::new(),
            _array: None,
            row_label: None,
            authoring_layout: Some("vmad".to_string()),
            authoring_key: None,
            scope_id: None,
        }
    }

    fn make_formid_subrecord_spec() -> SchemaSubrecordJson {
        SchemaSubrecordJson {
            id: "PNAM".to_string(),
            kind: "parsed".to_string(),
            display_label: Some("Previous INFO".to_string()),
            codec: Some("formid".to_string()),
            fields: Vec::new(),
            repeatable: false,
            required: false,
            localized: false,
            enum_ref: None,
            formlink_target: Some("INFO".to_string()),
            formlink_targets: Vec::new(),
            null_allowed: false,
            union_selector: None,
            union_variants: Vec::new(),
            _array: None,
            row_label: None,
            authoring_layout: None,
            authoring_key: None,
            scope_id: None,
        }
    }

    fn make_schema_field(id: &str, kind: &str, display_label: &str) -> SchemaFieldJson {
        SchemaFieldJson {
            id: id.to_string(),
            kind: kind.to_string(),
            display_label: Some(display_label.to_string()),
            enum_ref: None,
            formlink_target: None,
            formlink_targets: Vec::new(),
            null_allowed: false,
            union_variants: Vec::new(),
            array: None,
            fields: Vec::new(),
            default_value: None,
            presence_conditions: Vec::new(),
        }
    }

    fn make_union_struct_subrecord_spec() -> SchemaSubrecordJson {
        SchemaSubrecordJson {
            id: "DNAM".to_string(),
            kind: "parsed_with_raw_fallback".to_string(),
            display_label: Some("Data".to_string()),
            codec: None,
            fields: Vec::new(),
            repeatable: false,
            required: false,
            localized: false,
            enum_ref: None,
            formlink_target: None,
            formlink_targets: Vec::new(),
            null_allowed: false,
            union_selector: None,
            union_variants: vec![SchemaUnionVariantJson {
                id: "data".to_string(),
                codec: Some("struct:I,I,I,B,f".to_string()),
                enum_ref: None,
                fields: vec![
                    make_schema_field("field_a", "uint32", "FieldA"),
                    make_schema_field("field_b", "uint32", "FieldB"),
                    make_schema_field("field_c", "uint32", "FieldC"),
                    make_schema_field("field_d", "uint8", "FieldD"),
                    make_schema_field("field_e", "float32", "FieldE"),
                ],
                conditions: Vec::new(),
            }],
            _array: None,
            row_label: None,
            authoring_layout: None,
            authoring_key: None,
            scope_id: None,
        }
    }

    #[test]
    fn enum_numeric_value_json_accepts_bool_scalars() {
        let enum_def = make_bool_enum();
        assert_eq!(
            enum_numeric_value_json(&JsonValue::Bool(true), Some(&enum_def), "field").unwrap(),
            1
        );
        assert_eq!(
            enum_numeric_value_json(
                &JsonValue::String("False".to_string()),
                Some(&enum_def),
                "field",
            )
            .unwrap(),
            0
        );
    }

    #[test]
    fn compact_null_typed_subrecord_expands_to_null_value() {
        let spec = make_formid_subrecord_spec();
        let expanded = expand_compact_field_payload_from_schema_json(
            "PNAM",
            &JsonValue::Null,
            Some(&spec),
            None,
        )
        .expect("expand null formid");

        assert_eq!(expanded.get("value"), Some(&JsonValue::Null));
    }

    #[test]
    fn compact_union_struct_payload_fills_omitted_defaults() {
        let spec = make_union_struct_subrecord_spec();
        let compact = serde_json::json!({
            "variant": "data",
            "value": {
                "FieldA": 1,
                "FieldB": 2,
                "FieldC": 3,
                "FieldE": 1.5
            }
        });
        let expanded =
            expand_compact_field_payload_from_schema_json("DNAM", &compact, Some(&spec), Some(131))
                .expect("expand union struct");
        let value = expanded
            .get("value")
            .and_then(|value| value.as_object())
            .expect("expanded value mapping");

        assert_eq!(value.get("FieldD"), Some(&JsonValue::Number(0.into())));
        assert!(value.contains_key("FieldE"));
    }

    #[test]
    fn enum_numeric_value_json_accepts_flag_label_lists() {
        let enum_def = make_flag_enum();
        let value = JsonValue::Array(vec![
            JsonValue::String("CritEffectOnDeath".to_string()),
            JsonValue::String("BoltAction".to_string()),
        ]);
        assert_eq!(
            enum_numeric_value_json(&value, Some(&enum_def), "field").unwrap(),
            4_194_560
        );
    }

    #[test]
    fn enum_numeric_value_json_accepts_unknown_flag_labels() {
        let enum_def = make_flag_enum();
        assert_eq!(
            enum_numeric_value_json(
                &JsonValue::String("Unknown3".to_string()),
                Some(&enum_def),
                "field",
            )
            .unwrap(),
            8
        );
        assert_eq!(
            enum_numeric_value_json(
                &JsonValue::String("unknown_5".to_string()),
                Some(&enum_def),
                "field",
            )
            .unwrap(),
            32
        );
    }

    #[test]
    fn enum_numeric_value_json_accepts_aliases() {
        let enum_def = make_flag_enum();
        assert_eq!(
            enum_numeric_value_json(
                &JsonValue::String("CritEffectOnDeath".to_string()),
                Some(&enum_def),
                "field",
            )
            .unwrap(),
            256
        );
    }

    #[test]
    fn compact_vmad_authoring_payload_expands_as_hybrid_raw_source() {
        let spec = make_vmad_subrecord_spec();
        let compact = serde_json::json!({
            "kind": "vmad",
            "size": 6,
            "Version": 6,
            "Object Format": 2,
            "Scripts": [],
            "raw_hex": "060002000000"
        });
        let expanded =
            expand_compact_field_payload_from_schema_json("VMAD", &compact, Some(&spec), None)
                .unwrap();

        assert_eq!(
            expanded.get("preservation_mode"),
            Some(&JsonValue::String("hybrid".to_string()))
        );
        assert_eq!(
            expanded.get("raw_hex"),
            Some(&JsonValue::String("060002000000".to_string()))
        );
        assert_eq!(
            expanded.get("value").and_then(|value| value.get("Version")),
            Some(&JsonValue::Number(6.into()))
        );
    }

    fn make_custom_codec_spec(id: &str, codec: &str) -> SchemaSubrecordJson {
        SchemaSubrecordJson {
            id: id.to_string(),
            kind: "custom_codec".to_string(),
            display_label: None,
            codec: Some(codec.to_string()),
            fields: Vec::new(),
            repeatable: false,
            required: false,
            localized: false,
            enum_ref: None,
            formlink_target: None,
            formlink_targets: Vec::new(),
            null_allowed: false,
            union_selector: None,
            union_variants: Vec::new(),
            _array: None,
            row_label: None,
            authoring_layout: None,
            authoring_key: None,
            scope_id: None,
        }
    }

    fn empty_native_import_context() -> NativeImportContext {
        let header = ParsedPluginHeader {
            version: 1.0,
            num_records: 0,
            next_object_id: 0,
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
        };
        NativeImportContext::new("Patch.esp".to_string(), Some("fo4".to_string()), 24, header)
    }

    #[test]
    fn build_nvnm_authoring_field_uses_structured_payload_over_raw_hex() {
        // Regression for fix #2: when the authoring-dir YAML carries an
        // edited NVNM structured payload alongside `raw_hex`, the encoder
        // must re-serialize from the structured fields, not silently fall
        // back to the (now-stale) raw_hex.
        use crate::nvnm::{
            NvnmDoorRef, NvnmGrid, NvnmParent, NvnmPayload, NvnmTriangle, NvnmVertex,
        };
        let spec = make_custom_codec_spec("NVNM", "esp_authoring_core::nvnm");
        let edited = NvnmPayload {
            version: 15,
            flags: 0,
            parent: NvnmParent::Interior { cell: 0x0001_2345 },
            vertices: vec![NvnmVertex {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            }],
            triangles: vec![NvnmTriangle {
                vertices: [0, 0, 0],
                links: [-1, -1, -1],
                cover_marker: [0u8; 9],
                flags: 0,
            }],
            edge_links: vec![],
            door_refs: vec![NvnmDoorRef {
                triangle_index: 0,
                padding: [0u8; 4],
                door_ref_form_id: 0x0099_8877,
            }],
            cover_array: vec![],
            cover_triangle_mappings: vec![],
            waypoints: vec![],
            grid: NvnmGrid::default(),
        };
        let edited_bytes = crate::nvnm::write_nvnm(&edited);

        let mut payload = match crate::nvnm::nvnm_to_yaml(&edited) {
            JsonValue::Object(map) => map,
            _ => panic!("nvnm_to_yaml returned non-object"),
        };
        // The export flattens nvnm_to_yaml into the subrecord payload alongside
        // a stale raw_hex (representing the on-disk bytes before the YAML edit).
        // Pre-fix, the encoder would emit those stale bytes and lose the edit.
        payload.insert(
            "raw_hex".to_string(),
            JsonValue::String("DEADBEEF".to_string()),
        );

        let mut context = empty_native_import_context();
        let subrecord = build_subrecord_from_authoring_field_json_native(
            "NVNM",
            &payload,
            Some(&spec),
            &mut context,
            false,
        )
        .unwrap();
        assert_eq!(subrecord.signature, "NVNM");
        assert_eq!(subrecord.data.as_ref(), edited_bytes.as_slice());
    }

    #[test]
    fn build_vhgt_authoring_field_uses_structured_payload() {
        let spec = make_custom_codec_spec("VHGT", "esp_authoring_core::land::heightmap");
        let mut deltas = [[0i8; 33]; 33];
        deltas[10][20] = 17;
        let edited = crate::land::heightmap::LandHeightMap { base: 42.5, deltas };
        let edited_bytes = crate::land::heightmap::write_heightmap(&edited);

        let mut payload = match crate::land::heightmap::heightmap_to_yaml(&edited) {
            JsonValue::Object(map) => map,
            _ => panic!("heightmap_to_yaml returned non-object"),
        };
        payload.insert(
            "raw_hex".to_string(),
            JsonValue::String("DEADBEEF".to_string()),
        );

        let mut context = empty_native_import_context();
        let subrecord = build_subrecord_from_authoring_field_json_native(
            "VHGT",
            &payload,
            Some(&spec),
            &mut context,
            false,
        )
        .unwrap();
        assert_eq!(subrecord.data.as_ref(), edited_bytes.as_slice());
    }

    #[test]
    fn build_vnml_authoring_field_uses_structured_payload() {
        let spec = make_custom_codec_spec("VNML", "esp_authoring_core::land::heightmap");
        let mut normals = [[(0i8, 0i8, 0i8); 33]; 33];
        normals[5][7] = (1, 2, 3);
        let edited = crate::land::heightmap::LandVertexNormals { normals };
        let edited_bytes = crate::land::heightmap::write_vertex_normals(&edited);

        let mut payload = match crate::land::heightmap::vertex_normals_to_yaml(&edited) {
            JsonValue::Object(map) => map,
            _ => panic!("vertex_normals_to_yaml returned non-object"),
        };
        payload.insert(
            "raw_hex".to_string(),
            JsonValue::String("DEADBEEF".to_string()),
        );

        let mut context = empty_native_import_context();
        let subrecord = build_subrecord_from_authoring_field_json_native(
            "VNML",
            &payload,
            Some(&spec),
            &mut context,
            false,
        )
        .unwrap();
        assert_eq!(subrecord.data.as_ref(), edited_bytes.as_slice());
    }

    #[test]
    fn build_nvnm_authoring_field_falls_back_to_raw_hex_when_structured_fails() {
        // Resilience: when the structured payload is malformed (e.g. an
        // invalid `parent`), the encoder must fall back to raw_hex rather
        // than propagate the codec error.
        let spec = make_custom_codec_spec("NVNM", "esp_authoring_core::nvnm");
        let mut payload = JsonMap::new();
        payload.insert("version".to_string(), JsonValue::Number(15.into()));
        // Invalid: parent must carry interior_cell or exterior_world.
        payload.insert("parent".to_string(), serde_json::json!({}));
        payload.insert(
            "raw_hex".to_string(),
            JsonValue::String("DEADBEEF".to_string()),
        );

        let mut context = empty_native_import_context();
        let subrecord = build_subrecord_from_authoring_field_json_native(
            "NVNM",
            &payload,
            Some(&spec),
            &mut context,
            false,
        )
        .unwrap();
        assert_eq!(subrecord.data.as_ref(), &[0xDE, 0xAD, 0xBE, 0xEF][..]);
    }

    #[test]
    fn build_vmad_authoring_field_uses_parsed_payload_when_complete() {
        // The parsed payload wins over raw_hex when build_vmad_bytes_from_payload
        // succeeds; raw_hex is only the fallback for unparsable blobs.
        let spec = make_vmad_subrecord_spec();
        let mut payload = JsonMap::new();
        payload.insert(
            "preservation_mode".to_string(),
            JsonValue::String("hybrid".to_string()),
        );
        // raw_hex carries different bytes than the parsed payload would
        // produce. With the new behaviour, the parsed payload wins.
        payload.insert(
            "raw_hex".to_string(),
            JsonValue::String("DEADBEEFCAFE".to_string()),
        );
        payload.insert(
            "value".to_string(),
            serde_json::json!({
                "kind": "vmad",
                "Version": 6,
                "Object Format": 2,
                "Scripts": []
            }),
        );
        let header = ParsedPluginHeader {
            version: 1.0,
            num_records: 0,
            next_object_id: 0,
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
        };
        let mut context =
            NativeImportContext::new("Patch.esp".to_string(), Some("fo4".to_string()), 24, header);

        let subrecord = build_subrecord_from_authoring_field_json_native(
            "VMAD",
            &payload,
            Some(&spec),
            &mut context,
            false,
        )
        .unwrap();

        assert_eq!(subrecord.signature, "VMAD");
        assert_eq!(subrecord.data, vec![0x06, 0x00, 0x02, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn record_filename_with_editor_id() {
        let r = make_record("ARMO", 0x010023AB, Some("MyArmor"));
        assert_eq!(
            record_filename_native(&r, "MyMod.esp", ".json"),
            "MyArmor - 0023AB_MyMod.esp.json"
        );
    }

    #[test]
    fn record_filename_without_editor_id() {
        let r = make_record("WEAP", 0x0200FF42, None);
        assert_eq!(
            record_filename_native(&r, "MyMod.esp", ".yaml"),
            "00FF42_MyMod.esp.yaml"
        );
    }

    #[test]
    fn record_filename_strips_editor_id_trailing_null() {
        let r = make_record("NPC_", 0x00ABCDEF, Some("Actor1"));
        assert_eq!(
            record_filename_native(&r, "P.esp", ".json"),
            "Actor1 - ABCDEF_P.esp.json"
        );
    }

    #[test]
    fn form_ref_from_raw_handles_local_and_master() {
        let masters = vec!["Master.esm".to_string(), "Other.esm".to_string()];
        // master index 0
        let (p, obj, raw, mi) = form_ref_from_raw_native(0x00_0000AB, &masters, "Me.esp");
        assert_eq!(p.as_deref(), Some("Master.esm"));
        assert_eq!(obj, 0xAB);
        assert_eq!(raw, Some(0xAB));
        assert!(mi.is_none());
        // master index 1
        let (p, _, _, _) = form_ref_from_raw_native(0x01_0000FF, &masters, "Me.esp");
        assert_eq!(p.as_deref(), Some("Other.esm"));
        // own plugin (index == len(masters))
        let (p, _, _, _) = form_ref_from_raw_native(0x02_0000AB, &masters, "Me.esp");
        assert_eq!(p.as_deref(), Some("Me.esp"));
        // LOCAL_FORM_INDEX (0xFF)
        let (p, obj, _, _) = form_ref_from_raw_native(0xFF_001234, &masters, "Me.esp");
        assert_eq!(p, None);
        assert_eq!(obj, 0x1234);
        // null form
        let (p, obj, raw, _) = form_ref_from_raw_native(0, &masters, "Me.esp");
        assert_eq!(p, None);
        assert_eq!(obj, 0);
        assert_eq!(raw, Some(0));
        // missing index
        let (p, _, _, mi) = form_ref_from_raw_native(0x05_0000AB, &masters, "Me.esp");
        assert!(p.is_none());
        assert_eq!(mi, Some(5));
    }

    #[test]
    fn record_form_id_format_preserves_missing_master_index() {
        let masters = vec!["FalloutNV.esm".to_string()];
        assert_eq!(
            format_record_form_id_native(0x0200_0801, &masters, "GunRunnersArsenal.esm"),
            "02000801"
        );
    }

    #[test]
    fn authoring_export_lifts_editor_id_to_top_level_eid() {
        let plugin = empty_plugin(Some("fo4"));
        let strings = LocalizedStringsState::default();
        let mut record = make_record("KYWD", 0xFF001234, Some("NativeKeyword"));
        record.subrecords.push(ParsedSubrecord {
            signature: SmolStr::new_static("FULL"),
            data: Bytes::from_static(b"Keyword Name\0"),
            semantic_type: None,
        });

        let payload = serialize_record_payload_to_json(&record, &plugin, &strings);
        assert_eq!(
            payload.get("eid").and_then(JsonValue::as_str),
            Some("NativeKeyword")
        );
        let fields = payload
            .get("fields")
            .and_then(JsonValue::as_array)
            .expect("fields");
        assert!(fields.iter().all(|entry| {
            single_key_mapping_json(entry)
                .map(|(key, _)| !compact_authoring_key_is_eid(key))
                .unwrap_or(true)
        }));
    }

    #[test]
    fn authoring_import_builds_edid_from_top_level_eid() {
        let mut context = NativeImportContext::new(
            "Test.esp".to_string(),
            Some("fo4".to_string()),
            MODERN_HEADER_SIZE,
            empty_header(),
        );
        let payload = serde_json::json!({
            "signature": "KYWD",
            "form_id": "000123",
            "eid": "TopLevelKeyword",
            "fields": []
        });

        let record = parse_record_from_json_compact_native(
            payload.as_object().expect("record payload"),
            &mut context,
        )
        .unwrap();

        assert_eq!(record.subrecords.len(), 1);
        assert_eq!(record.subrecords[0].signature.as_str(), "EDID");
        assert_eq!(&record.subrecords[0].data[..], b"TopLevelKeyword\0");
    }

    #[test]
    fn authoring_import_uses_alias_flags_for_qust_fnam_inside_alias_block() {
        let mut context = NativeImportContext::new(
            "Test.esp".to_string(),
            Some("fo4".to_string()),
            MODERN_HEADER_SIZE,
            empty_header(),
        );
        let payload = serde_json::json!({
            "signature": "QUST",
            "form_id": "000123",
            "eid": "TestQuest",
            "fields": [
                {"ANAM": 1},
                {"ALST": 0},
                {"ALID": "TestAlias"},
                {"FNAM": ["Optional", "Allow Dead"]},
                {"ALED": true}
            ]
        });

        let record = parse_record_from_json_compact_native(
            payload.as_object().expect("record payload"),
            &mut context,
        )
        .unwrap();
        let flags = record
            .subrecords
            .iter()
            .find(|subrecord| subrecord.signature.as_str() == "FNAM")
            .expect("alias FNAM");

        assert_eq!(
            u32::from_le_bytes(flags.data[..4].try_into().unwrap()),
            0x12
        );
    }

    #[test]
    fn authoring_import_uses_action_flags_for_scoped_scen_fnam() {
        let mut context = NativeImportContext::new(
            "Test.esp".to_string(),
            Some("fo4".to_string()),
            MODERN_HEADER_SIZE,
            empty_header(),
        );
        let payload = serde_json::json!({
            "signature": "SCEN",
            "form_id": "000123",
            "eid": "TestScene",
            "fields": [
                {"FNAM": ["Show All Text"]},
                {"ALID": 0},
                {"INAM": 1},
                {"FNAM": ["Face Target"]}
            ]
        });

        let record = parse_record_from_json_compact_native(
            payload.as_object().expect("record payload"),
            &mut context,
        )
        .unwrap();
        let flags = record
            .subrecords
            .iter()
            .filter(|subrecord| subrecord.signature.as_str() == "FNAM")
            .nth(1)
            .expect("action FNAM");

        assert_eq!(
            u32::from_le_bytes(flags.data[..4].try_into().unwrap()),
            0x8000
        );
    }

    #[test]
    fn authoring_import_fills_presence_gated_gap_before_explicit_trailing_field() {
        let mut context = NativeImportContext::new(
            "Test.esp".to_string(),
            Some("fo4".to_string()),
            MODERN_HEADER_SIZE,
            empty_header(),
        );
        let payload = serde_json::json!({
            "signature": "PERK",
            "form_id": "000123",
            "fields": [
                {"DATA": {"NumRanks": 1, "Hidden": true}}
            ]
        });

        let record = parse_record_from_json_compact_native(
            payload.as_object().expect("record payload"),
            &mut context,
        )
        .unwrap();
        let data = record
            .subrecords
            .iter()
            .find(|subrecord| subrecord.signature.as_str() == "DATA")
            .expect("PERK DATA");

        assert_eq!(&data.data[..], &[0, 0, 1, 0, 1]);
    }

    #[test]
    fn authoring_import_fills_race_data_fields_for_matching_form_version() {
        let mut context = NativeImportContext::new(
            "Test.esp".to_string(),
            Some("fo4".to_string()),
            MODERN_HEADER_SIZE,
            empty_header(),
        );
        let payload = serde_json::json!({
            "signature": "RACE",
            "form_id": "000123",
            "form_version": 208,
            "eid": "TestRace",
            "fields": [
                {
                    "Data": {
                        "MaleHeight": 1.0,
                        "FemaleHeight": 1.0,
                        "MaleDefaultWeightThin": 0.5,
                        "MaleDefaultWeightMuscular": 0.5
                    }
                }
            ]
        });

        let record = parse_record_from_json_compact_native(
            payload.as_object().expect("record payload"),
            &mut context,
        )
        .unwrap();
        let data = record
            .subrecords
            .iter()
            .find(|subrecord| subrecord.signature.as_str() == "DATA")
            .expect("DATA subrecord");

        assert_eq!(data.data.len(), 200);
    }

    #[test]
    fn rewrite_formids_preserves_invalid_formid_sentinel() {
        let mut record = make_record("EQUP", 0xFF001234, None);
        record.subrecords.push(ParsedSubrecord {
            signature: SmolStr::new_static("ANAM"),
            data: Bytes::from(u32::MAX.to_le_bytes().to_vec()),
            semantic_type: Some("formid".to_string()),
        });
        record.subrecords.push(ParsedSubrecord {
            signature: SmolStr::new_static("PNAM"),
            data: Bytes::from([u32::MAX.to_le_bytes(), 0xFF00_5678u32.to_le_bytes()].concat()),
            semantic_type: Some("formid_array".to_string()),
        });

        rewrite_formids_in_record(&mut record, 0);

        assert_eq!(
            u32::from_le_bytes(record.subrecords[0].data[0..4].try_into().unwrap()),
            u32::MAX
        );
        assert_eq!(
            u32::from_le_bytes(record.subrecords[1].data[0..4].try_into().unwrap()),
            u32::MAX
        );
        assert_eq!(
            u32::from_le_bytes(record.subrecords[1].data[4..8].try_into().unwrap()),
            0x0000_5678
        );
    }

    #[test]
    fn starfield_component_streams_export_as_components_field() {
        let plugin = empty_plugin(Some("starfield"));
        let strings = LocalizedStringsState::default();
        let mut record = make_record("WEAP", 0x0002_BF65B, None);
        record.subrecords.push(ParsedSubrecord {
            signature: SmolStr::new_static("BFCB"),
            data: Bytes::from_static(b"BGSAnimationGraph_Component\0"),
            semantic_type: None,
        });
        record.subrecords.push(ParsedSubrecord {
            signature: SmolStr::new_static("ANAM"),
            data: Bytes::from_static(b"graph\0"),
            semantic_type: None,
        });
        record.subrecords.push(ParsedSubrecord {
            signature: SmolStr::new_static("BFCE"),
            data: Bytes::new(),
            semantic_type: None,
        });

        let payload = serialize_record_payload_to_json(&record, &plugin, &strings);
        let fields = payload
            .get("fields")
            .and_then(JsonValue::as_array)
            .expect("fields");

        assert_eq!(fields.len(), 1);
        let components = fields[0]
            .get("Components")
            .and_then(JsonValue::as_array)
            .expect("Components");
        let component = components[0]
            .get("AnimationGraphComponent")
            .and_then(JsonValue::as_object)
            .expect("AnimationGraphComponent");
        assert_eq!(
            component.get("Type").and_then(JsonValue::as_str),
            Some("BGSAnimationGraph_Component")
        );
        assert_eq!(
            component
                .get("ANAM")
                .or_else(|| component.get("Root Animation Graph"))
                .or_else(|| component.get("RootAnimationGraph"))
                .and_then(JsonValue::as_str),
            Some("graph")
        );
        assert!(fields.iter().all(|entry| {
            single_key_mapping_json(entry)
                .map(|(key, _)| key != "BFCB" && key != "BFCE")
                .unwrap_or(true)
        }));
    }

    #[test]
    fn starfield_components_field_imports_as_bfcb_payload_bfce_stream() {
        let mut context = NativeImportContext::new(
            "Test.esp".to_string(),
            Some("starfield".to_string()),
            MODERN_HEADER_SIZE,
            empty_header(),
        );
        let payload = serde_json::json!({
            "signature": "WEAP",
            "form_id": "000123",
            "fields": [
                {
                    "Components": [
                        {
                            "AnimationGraphComponent": {
                                "Type": "BGSAnimationGraph_Component",
                                "ANAM": "graph"
                            }
                        }
                    ]
                }
            ]
        });
        let record = parse_record_from_json_compact_native(
            payload.as_object().expect("record payload"),
            &mut context,
        )
        .unwrap();

        assert_eq!(
            record
                .subrecords
                .iter()
                .map(|subrecord| subrecord.signature.as_str())
                .collect::<Vec<_>>(),
            vec!["BFCB", "ANAM", "BFCE"]
        );
        assert_eq!(
            &record.subrecords[0].data[..],
            b"BGSAnimationGraph_Component\0"
        );
        assert_eq!(&record.subrecords[1].data[..], b"graph\0");
        assert!(record.subrecords[2].data.is_empty());
    }

    #[test]
    fn group_label_text_returns_sig_for_type_zero() {
        let g = ParsedGroup {
            label: *b"WEAP",
            group_type: 0,
            tail: Bytes::new(),
            children: Vec::new(),
        };
        assert_eq!(group_label_text_native(&g).as_deref(), Some("WEAP"));
    }

    #[test]
    fn group_label_text_rejects_nonzero_type() {
        let g = ParsedGroup {
            label: *b"WEAP",
            group_type: 1,
            tail: Bytes::new(),
            children: Vec::new(),
        };
        assert!(group_label_text_native(&g).is_none());
    }

    #[test]
    fn plugin_index_duplicate_object_id_keeps_last_traversal_record() {
        let mut plugin = empty_plugin(None);
        plugin.root_items = vec![
            ParsedItem::Record(make_record("ARMO", 0x0100_1234, Some("First"))),
            ParsedItem::Record(make_record("WEAP", 0x0200_1234, Some("Second"))),
        ];

        let index = PluginIndex::build(&plugin);
        let record = index.get(0x001234).expect("indexed record");

        assert_eq!(record.signature.as_str(), "WEAP");
        assert_eq!(record.form_id, 0x0200_1234);
    }

    #[test]
    fn row_group_export_splits_on_anchor_and_emits_schema_order() {
        let grouped = group_schema_row_group_fields_json(vec![
            CompactAuthoringEntry {
                signature: "EFID".to_string(),
                key: "Base Effect".to_string(),
                value: serde_json::json!({"reference": {"plugin": "DLCCoast.esm", "object_id": "04B5FB"}}),
                group_key: Some("group_effect".to_string()),
                group_order: Some(0),
                group_anchor: true,
                ordinal: 0,
            },
            CompactAuthoringEntry {
                signature: "EFIT".to_string(),
                key: "EFIT".to_string(),
                value: serde_json::json!({"Magnitude": 10.0, "Duration": 480}),
                group_key: Some("group_effect".to_string()),
                group_order: Some(1),
                group_anchor: false,
                ordinal: 1,
            },
            CompactAuthoringEntry {
                signature: "CTDA".to_string(),
                key: "CTDA".to_string(),
                value: serde_json::json!({"Type": 160}),
                group_key: Some("group_effect".to_string()),
                group_order: Some(2),
                group_anchor: false,
                ordinal: 2,
            },
            CompactAuthoringEntry {
                signature: "CTDA".to_string(),
                key: "CTDA".to_string(),
                value: serde_json::json!({"Type": 64}),
                group_key: Some("group_effect".to_string()),
                group_order: Some(2),
                group_anchor: false,
                ordinal: 3,
            },
            CompactAuthoringEntry {
                signature: "EFID".to_string(),
                key: "Base Effect".to_string(),
                value: serde_json::json!({"reference": {"plugin": "DLCCoast.esm", "object_id": "04B5FD"}}),
                group_key: Some("group_effect".to_string()),
                group_order: Some(0),
                group_anchor: true,
                ordinal: 4,
            },
            CompactAuthoringEntry {
                signature: "EFIT".to_string(),
                key: "EFIT".to_string(),
                value: serde_json::json!({"Magnitude": 20.0, "Duration": 480}),
                group_key: Some("group_effect".to_string()),
                group_order: Some(1),
                group_anchor: false,
                ordinal: 5,
            },
            CompactAuthoringEntry {
                signature: "CTDA".to_string(),
                key: "CTDA".to_string(),
                value: serde_json::json!({"Type": 160}),
                group_key: Some("group_effect".to_string()),
                group_order: Some(2),
                group_anchor: false,
                ordinal: 6,
            },
        ]);

        let effects = grouped[0]
            .get("Effects")
            .and_then(JsonValue::as_array)
            .expect("Effects row group");
        let row0_keys: Vec<&str> = effects[0]
            .as_object()
            .expect("row 0")
            .keys()
            .map(String::as_str)
            .collect();
        let row1_keys: Vec<&str> = effects[1]
            .as_object()
            .expect("row 1")
            .keys()
            .map(String::as_str)
            .collect();
        let row2_keys: Vec<&str> = effects[2]
            .as_object()
            .expect("row 2")
            .keys()
            .map(String::as_str)
            .collect();

        assert_eq!(row0_keys, vec!["Base Effect", "EFIT", "CTDA"]);
        assert_eq!(row1_keys, vec!["CTDA"]);
        assert_eq!(row2_keys, vec!["Base Effect", "EFIT", "CTDA"]);
    }

    /// RACE's BodyData group repeats INDX/MODL/MODT across a male and a female
    /// half, and both halves resolve to one authoring key. Each row entry must
    /// still produce exactly one subrecord.
    fn race_body_data_record_spec() -> SchemaRecordJson {
        let half = |marker: &str| {
            serde_json::json!([
                {"id": marker, "kind": "parsed", "codec": "empty", "fields": [],
                 "authoring_layout": "row_group", "authoring_key": "group_body_data",
                 "scope_id": "body_data"},
                {"id": "INDX", "kind": "raw", "fields": [],
                 "authoring_layout": "row_group", "authoring_key": "group_body_data",
                 "scope_id": "body_data"},
                {"id": "MODL", "kind": "parsed", "display_label": "Model FileName",
                 "codec": "zstring", "fields": [],
                 "authoring_layout": "row_group", "authoring_key": "group_body_data",
                 "scope_id": "body_data"},
            ])
        };
        let mut subrecords = half("MNAM").as_array().unwrap().clone();
        subrecords.extend(half("FNAM").as_array().unwrap().clone());
        serde_json::from_value(serde_json::json!({
            "id": "RACE",
            "subrecords": subrecords,
        }))
        .expect("race body data fixture")
    }

    #[test]
    fn row_group_import_emits_one_subrecord_per_row_entry() {
        let record_spec = race_body_data_record_spec();
        let rows = serde_json::json!([
            {"MNAM": true, "INDX": "00000000", "FNAM": true},
            {"INDX": "00000000", "MODL": "Actors\\Character\\Body.egt"},
        ]);
        let mut subrecords = Vec::new();
        let mut dispatch_state = CompactAuthoringDispatchState::default();
        let mut label_counts = HashMap::new();
        let mut context = NativeImportContext::new(
            "Test.esp".to_string(),
            Some("fo4".to_string()),
            24,
            ParsedPluginHeader::default_for_test(),
        );

        append_schema_row_group_subrecords_json_native(
            "group_body_data",
            &rows,
            Some(&record_spec),
            &mut dispatch_state,
            &mut label_counts,
            &mut subrecords,
            &mut context,
            false,
            None,
        )
        .expect("row group import");

        let emitted: Vec<&str> = subrecords.iter().map(|s| s.signature.as_str()).collect();
        assert_eq!(
            emitted,
            vec!["MNAM", "INDX", "FNAM", "INDX", "MODL"],
            "each row entry must map to exactly one subrecord"
        );
    }

    fn make_record_with_formid_ref(
        signature: &str,
        form_id: u32,
        editor_id: Option<&str>,
        ref_subrecord_signature: &str,
        target_form_id: u32,
    ) -> ParsedRecord {
        let mut record = make_record(signature, form_id, editor_id);
        record.subrecords.push(ParsedSubrecord {
            signature: SmolStr::new(ref_subrecord_signature),
            data: Bytes::from(target_form_id.to_le_bytes().to_vec()),
            semantic_type: Some("formid".to_string()),
        });
        record
    }

    #[test]
    fn plugin_index_sections_lookup_and_reverse_refs() {
        let mut plugin = empty_plugin(Some("fo4"));
        // Two records; second references the first via a formid subrecord.
        let weap = make_record("WEAP", 0xFF000001, Some("MyWeapon"));
        let omod =
            make_record_with_formid_ref("OMOD", 0xFF000002, Some("MyOmod"), "MNAM", 0xFF000001);
        plugin.root_items.push(ParsedItem::Group(ParsedGroup {
            label: *b"WEAP",
            group_type: 0,
            tail: Bytes::new(),
            children: vec![ParsedItem::Record(weap)],
        }));
        plugin.root_items.push(ParsedItem::Group(ParsedGroup {
            label: *b"OMOD",
            group_type: 0,
            tail: Bytes::new(),
            children: vec![ParsedItem::Record(omod)],
        }));

        let core = build_core_section(&plugin);
        let refs = build_refs_section(&plugin);

        let records = build_records_section(&plugin);
        assert!(records.record(&plugin, 0xFF000001).is_some());
        assert!(records.record(&plugin, 0xFF000002).is_some());
        // EditorID lookup is case-insensitive
        assert_eq!(
            core.by_eid_lower
                .get("myweapon")
                .and_then(|values| values.first())
                .map(|form_key| form_key.render()),
            Some("Test.esp:000001".to_string()),
        );
        // Signature index
        assert_eq!(
            core.form_ids_by_signature
                .get(&SmolStr::new("WEAP"))
                .map(Vec::as_slice),
            Some([0xFF000001].as_slice()),
        );
        assert_eq!(
            refs.forward_refs_by_form_key
                .get(&normalize_form_key("Test.esp:000002").unwrap())
                .map(|values| values.iter().map(|v| v.render()).collect::<Vec<_>>()),
            Some(vec!["Test.esp:000001".to_string()]),
        );
        assert_eq!(
            refs.reverse_refs_by_form_key
                .get(&normalize_form_key("Test.esp:000001").unwrap())
                .map(|values| values.iter().map(|v| v.render()).collect::<Vec<_>>()),
            Some(vec!["Test.esp:000002".to_string()]),
        );
    }

    #[test]
    fn plugin_index_sections_formkey_tables_classify_overrides_and_refs() {
        let mut plugin = empty_plugin(Some("fo4"));
        plugin.header.masters.push("Fallout4.esm".to_string());
        plugin.header.master_sizes.push(0);

        let target = make_record("MISC", 0xFF000800, Some("NativeTarget"));
        let source = make_record_with_formid_ref(
            "MISC",
            0xFF000801,
            Some("NativeSource"),
            "YNAM",
            0xFF000800,
        );
        let override_record = make_record("MISC", 0x0000ABCD, Some("NativeOverride"));
        plugin.root_items = vec![
            ParsedItem::Record(target),
            ParsedItem::Record(source),
            ParsedItem::Record(override_record),
        ];

        let core = build_core_section(&plugin);
        let refs = build_refs_section(&plugin);

        let local = core
            .by_form_key
            .get(&normalize_form_key("Test.esp:000800").unwrap())
            .expect("local FormKey indexed");
        assert_eq!(local.eid, "NativeTarget");
        assert!(!local.is_override);

        let override_entry = core
            .by_form_key
            .get(&normalize_form_key("Fallout4.esm:00ABCD").unwrap())
            .expect("override FormKey indexed under master");
        assert_eq!(override_entry.defined_in.as_ref(), "Test.esp");
        assert_eq!(override_entry.master_plugin.as_ref(), "Fallout4.esm");
        assert!(override_entry.is_override);

        assert_eq!(
            core.by_eid_lower
                .get("nativesource")
                .map(|values| values.iter().map(|v| v.render()).collect::<Vec<_>>()),
            Some(vec!["Test.esp:000801".to_string()]),
        );
        assert_eq!(
            refs.forward_refs_by_form_key
                .get(&normalize_form_key("Test.esp:000801").unwrap())
                .map(|values| values.iter().map(|v| v.render()).collect::<Vec<_>>()),
            Some(vec!["Test.esp:000800".to_string()]),
        );
        assert_eq!(
            refs.reverse_refs_by_form_key
                .get(&normalize_form_key("Test.esp:000800").unwrap())
                .map(|values| values.iter().map(|v| v.render()).collect::<Vec<_>>()),
            Some(vec!["Test.esp:000801".to_string()]),
        );
        assert_eq!(
            normalize_form_key("Test.esp:800")
                .map(|key| key.render())
                .as_deref(),
            Some("Test.esp:000800"),
        );
    }

    #[test]
    fn plugin_index_sections_missing_master_refs_preserve_raw_form_id() {
        let mut plugin = empty_plugin(Some("fo4"));
        plugin.header.masters.push("Fallout4.esm".to_string());
        plugin.header.master_sizes.push(0);

        let source = make_record_with_formid_ref(
            "MISC",
            0xFF000801,
            Some("NativeSource"),
            "YNAM",
            0x050000AB,
        );
        plugin.root_items = vec![ParsedItem::Record(source)];

        let refs = build_refs_section(&plugin);

        assert_eq!(
            refs.forward_refs_by_form_key
                .get(&normalize_form_key("Test.esp:000801").unwrap())
                .map(|values| values.iter().map(|v| v.render()).collect::<Vec<_>>()),
            Some(vec!["050000AB".to_string()]),
        );
        assert_eq!(
            resolve_form_id_to_form_key(
                0x050000AB,
                &Arc::from("Test.esp"),
                &plugin.header.masters,
            )
            .render()
            .as_str(),
            "050000AB",
        );
    }

    #[test]
    fn parsed_plugin_index_pick_owned_form_id_prefers_local() {
        // 0x00 = master 0, 0xFF = local. own_index = 1 (one master), so 0x01 also counts as own.
        let candidates = vec![0x00ABCDEF, 0xFFABCDEF, 0x01ABCDEF];
        assert_eq!(
            pick_owned_form_id(&candidates, 1),
            Some(0xFFABCDEF), // 0xFF wins because LOCAL_FORM_INDEX matches first
        );
        // Without a local match, falls back to first
        let masters_only = vec![0x00ABCDEF, 0x02ABCDEF];
        assert_eq!(pick_owned_form_id(&masters_only, 1), Some(0x00ABCDEF));
    }

    // Sibling-subrecord union deciders (PERK.EPFD, AECH.Data,
    // SNDR.Data) reference selectors whose names don't match the standard
    // field-name population path. The runtime publishes synthetic context
    // aliases right after the source subrecord decodes so that union
    // conditions evaluate against the correct sibling value.

    // Production shape: PERK.EPFT uint8 single-field → bare Number.
    #[test]
    fn publish_decider_aliases_perk_epft_bare_scalar() {
        let mut ctx: HashMap<String, JsonValue> = HashMap::new();
        let payload = JsonValue::Number(6_u64.into());
        publish_sibling_decider_context_aliases("PERK", "EPFT", &payload, &mut ctx);
        assert_eq!(ctx.get("epft"), Some(&JsonValue::Number(6_u64.into())));
    }

    // Production shape: AECH.KNAM uint32 + enum_ref → bare String label.
    #[test]
    fn publish_decider_aliases_aech_knam_bare_scalar_string() {
        let mut ctx: HashMap<String, JsonValue> = HashMap::new();
        let payload = JsonValue::String("BSOverdrive".to_string());
        publish_sibling_decider_context_aliases("AECH", "KNAM", &payload, &mut ctx);
        assert_eq!(
            ctx.get("knam_edit_value"),
            Some(&JsonValue::String("BSOverdrive".to_string()))
        );
    }

    // Production shape: SNDR.CNAM uint32 + enum_ref → bare String label.
    #[test]
    fn publish_decider_aliases_sndr_cnam_bare_scalar_string() {
        let mut ctx: HashMap<String, JsonValue> = HashMap::new();
        let payload = JsonValue::String("AutoWeapon".to_string());
        publish_sibling_decider_context_aliases("SNDR", "CNAM", &payload, &mut ctx);
        assert_eq!(
            ctx.get("cnam_edit_value"),
            Some(&JsonValue::String("AutoWeapon".to_string()))
        );
    }

    // PERK.PRKE Type=Ability/EntryPoint round-trips its own label and
    // overwrites context["Type"] from any prior Effect.
    #[test]
    fn publish_decider_aliases_perk_prke_passes_through_decoded_type() {
        let mut ctx: HashMap<String, JsonValue> = HashMap::new();
        ctx.insert("Type".to_string(), JsonValue::String("Ability".to_string()));
        let payload = serde_json::json!({"Type": "EntryPoint", "Rank": 0_u64, "Priority": 0_u64});
        publish_sibling_decider_context_aliases("PERK", "PRKE", &payload, &mut ctx);
        assert_eq!(
            ctx.get("Type"),
            Some(&JsonValue::String("EntryPoint".to_string()))
        );
    }

    // PERK.PRKE Type=0 (QuestStage) is default-stripped to {} by
    // authoring_field_is_default_json. The alias must default-publish
    // "QuestStage" so the wbPerkDATADecider union still dispatches —
    // and crucially must overwrite a stale "Type" left by a prior Effect.
    #[test]
    fn publish_decider_aliases_perk_prke_default_strip_publishes_quest_stage() {
        let mut ctx: HashMap<String, JsonValue> = HashMap::new();
        ctx.insert("Type".to_string(), JsonValue::String("Ability".to_string()));
        let payload = serde_json::json!({});
        publish_sibling_decider_context_aliases("PERK", "PRKE", &payload, &mut ctx);
        assert_eq!(
            ctx.get("Type"),
            Some(&JsonValue::String("QuestStage".to_string()))
        );
    }

    // Forward-compat: Object payload variant still works (defensive).
    #[test]
    fn publish_decider_aliases_object_payload_extracts_named_field() {
        let mut ctx: HashMap<String, JsonValue> = HashMap::new();
        let payload = serde_json::json!({"type": 6_u64, "other": "ignored"});
        publish_sibling_decider_context_aliases("PERK", "EPFT", &payload, &mut ctx);
        assert_eq!(ctx.get("epft"), Some(&JsonValue::Number(6_u64.into())));
    }

    #[test]
    fn publish_decider_aliases_unrelated_record_is_noop() {
        let mut ctx: HashMap<String, JsonValue> = HashMap::new();
        let payload = JsonValue::Number(1_u64.into());
        publish_sibling_decider_context_aliases("WEAP", "EPFT", &payload, &mut ctx);
        assert!(ctx.is_empty());
    }

    #[test]
    fn publish_decider_aliases_null_payload_is_noop() {
        let mut ctx: HashMap<String, JsonValue> = HashMap::new();
        publish_sibling_decider_context_aliases("PERK", "EPFT", &JsonValue::Null, &mut ctx);
        assert!(ctx.is_empty());
    }

    // EDID single-field parsed payload decodes as a bare String — that is the
    // production shape the GMST.DATA union selector consumes.
    #[test]
    fn publish_editor_id_prefix_alias_bare_string_extracts_first_char() {
        let mut ctx: HashMap<String, JsonValue> = HashMap::new();
        let payload = JsonValue::String("fNearDistance".to_string());
        publish_editor_id_prefix_alias("EDID", &payload, &mut ctx);
        assert_eq!(
            ctx.get("editor_id_prefix"),
            Some(&JsonValue::String("f".to_string()))
        );
    }

    // Defensive: future decoders may wrap EDID in an Object — still works.
    #[test]
    fn publish_editor_id_prefix_alias_object_payload_extracts_first_char() {
        let mut ctx: HashMap<String, JsonValue> = HashMap::new();
        let payload = serde_json::json!({"editor_id": "iMaxAllocatedActorsPerLocation"});
        publish_editor_id_prefix_alias("EDID", &payload, &mut ctx);
        assert_eq!(
            ctx.get("editor_id_prefix"),
            Some(&JsonValue::String("i".to_string()))
        );
    }

    #[test]
    fn publish_editor_id_prefix_alias_non_edid_is_noop() {
        let mut ctx: HashMap<String, JsonValue> = HashMap::new();
        let payload = JsonValue::String("anything".to_string());
        publish_editor_id_prefix_alias("DATA", &payload, &mut ctx);
        assert!(ctx.is_empty());
    }

    #[test]
    fn publish_editor_id_prefix_alias_empty_string_is_noop() {
        let mut ctx: HashMap<String, JsonValue> = HashMap::new();
        let payload = JsonValue::String(String::new());
        publish_editor_id_prefix_alias("EDID", &payload, &mut ctx);
        assert!(ctx.is_empty());
    }

    // Payload-length disambiguation between two
    // specs at the same (sig, scope) — TERM has SNAM=Looping Sound (formid)
    // and SNAM=Marker Parameters (array_struct of 24-byte rows) both at
    // top-level.
    fn term_snam_record_spec() -> SchemaRecordJson {
        serde_json::from_value(serde_json::json!({
            "id": "TERM",
            "subrecords": [
                {
                    "id": "SNAM",
                    "kind": "parsed",
                    "display_label": "Looping Sound",
                    "codec": "formid",
                    "fields": []
                },
                {
                    "id": "SNAM",
                    "kind": "parsed",
                    "display_label": "Marker Parameters",
                    "codec": "array_struct:f,f,f,f,I,B,B,B,B",
                    "fields": []
                }
            ]
        }))
        .expect("term snam fixture")
    }

    #[test]
    fn pick_unique_spec_by_payload_length_routes_4_bytes_to_formid() {
        let rec = term_snam_record_spec();
        let spec = pick_unique_spec_by_payload_length(&rec, "SNAM", None, 4)
            .expect("formid spec for 4-byte payload");
        assert_eq!(spec.display_label.as_deref(), Some("Looping Sound"));
    }

    #[test]
    fn pick_unique_spec_by_payload_length_routes_24_bytes_to_array_struct() {
        let rec = term_snam_record_spec();
        let spec = pick_unique_spec_by_payload_length(&rec, "SNAM", None, 24)
            .expect("array_struct spec for 24-byte payload");
        assert_eq!(spec.display_label.as_deref(), Some("Marker Parameters"));
    }

    #[test]
    fn pick_unique_spec_by_payload_length_routes_48_bytes_to_array_struct() {
        // Two-row Marker Parameters payload: 2 * 24 = 48.
        let rec = term_snam_record_spec();
        let spec = pick_unique_spec_by_payload_length(&rec, "SNAM", None, 48)
            .expect("array_struct spec for 48-byte payload");
        assert_eq!(spec.display_label.as_deref(), Some("Marker Parameters"));
    }

    #[test]
    fn pick_unique_spec_by_payload_length_returns_none_when_both_reject() {
        // 7 bytes: not 4 (formid) and not a multiple of 24 (Marker Parameters).
        let rec = term_snam_record_spec();
        assert!(pick_unique_spec_by_payload_length(&rec, "SNAM", None, 7).is_none());
    }

    #[test]
    fn pick_unique_spec_by_payload_length_returns_none_when_only_one_spec() {
        let rec: SchemaRecordJson = serde_json::from_value(serde_json::json!({
            "id": "TERM",
            "subrecords": [
                {"id": "SNAM", "kind": "parsed", "codec": "formid", "fields": []}
            ]
        }))
        .expect("single-spec fixture");
        assert!(pick_unique_spec_by_payload_length(&rec, "SNAM", None, 4).is_none());
    }

    #[test]
    fn dispatch_with_payload_length_routes_term_marker_params_correctly() {
        let rec = term_snam_record_spec();
        let counts: HashMap<(Option<String>, String), usize> = HashMap::new();
        // 24-byte SNAM: must route to Marker Parameters even at occurrence=0.
        let (spec, scope) = dispatch_subrecord_spec_in_scope(&rec, "SNAM", 24, None, &counts)
            .expect("dispatch found a spec");
        assert_eq!(spec.display_label.as_deref(), Some("Marker Parameters"));
        assert_eq!(scope, None);
    }

    #[test]
    fn dispatch_with_payload_length_falls_through_to_occurrence_when_ambiguous() {
        // Both specs accept 0 bytes (formid says no, array_struct says no — 0
        // is technically a multiple of 24). Verify behavior matches the
        // legacy occurrence-based dispatch in this case.
        let rec = term_snam_record_spec();
        let counts: HashMap<(Option<String>, String), usize> = HashMap::new();
        // 4-byte SNAM at occurrence=0 → Looping Sound (formid accepts, array
        // rejects → unique) — confirms length-aware path is correct.
        let (spec, _) = dispatch_subrecord_spec_in_scope(&rec, "SNAM", 4, None, &counts)
            .expect("dispatch found a spec");
        assert_eq!(spec.display_label.as_deref(), Some("Looping Sound"));
    }

    #[test]
    fn insert_topic_child_places_info_under_explicit_parent_dialogue() {
        const QUEST_FORM_ID: u32 = 0x0100_1000;
        const DIAL_FORM_ID: u32 = 0x0100_2000;
        const INFO_FORM_ID: u32 = 0x0100_3000;

        let handle_id = create_empty_plugin_handle("Test.esp", Some("fo4"));
        let inserted = {
            let mut store = plugin_handle_store_ref().lock().unwrap();
            let slot = store.get_mut(&handle_id).unwrap();
            // Quest-child group holding the parent DIAL (mirrors the target layout
            // the structured dialogue emitter builds: DIAL under QUST).
            slot.parsed.root_items.push(ParsedItem::Group(ParsedGroup {
                label: QUEST_FORM_ID.to_le_bytes(),
                group_type: QUEST_CHILD_GROUP,
                tail: Bytes::new(),
                children: vec![ParsedItem::Record(make_record("DIAL", DIAL_FORM_ID, None))],
            }));

            insert_topic_child_record_in_slot(
                slot,
                DIAL_FORM_ID,
                make_record("INFO", INFO_FORM_ID, None),
            )
            .expect("insert")
        };
        assert!(
            inserted,
            "INFO should be inserted under its explicit parent DIAL"
        );

        let store = plugin_handle_store_ref().lock().unwrap();
        let slot = store.get(&handle_id).unwrap();
        // The INFO must live in a Topic-Child group (type 7) labelled with the
        // parent DIAL's form_id, nested inside the quest-child group.
        let ParsedItem::Group(quest_group) = &slot.parsed.root_items[0] else {
            panic!("expected quest-child group");
        };
        let topic_group = quest_group
            .children
            .iter()
            .find_map(|item| match item {
                ParsedItem::Group(group)
                    if group.group_type == TOPIC_CHILD_GROUP
                        && group.label == DIAL_FORM_ID.to_le_bytes() =>
                {
                    Some(group)
                }
                _ => None,
            })
            .expect("topic-child group labelled with the DIAL form_id");
        assert!(topic_group.children.iter().any(|item| matches!(
            item,
            ParsedItem::Record(record)
                if record.signature.as_str() == "INFO" && record.form_id == INFO_FORM_ID
        )));
    }

    #[test]
    fn insert_topic_child_matches_parent_dialogue_by_object_id() {
        // Regression for the Stage-A INFO=0 bug: the conversion mapper passes the
        // parent DIAL's 24-bit object-id (`target.local`), but the emitted DIAL
        // record carries the output plugin's own-index byte (e.g. 0x07). The
        // insert must match on object-id and label the Topic-Child group with the
        // DIAL's FULL form_id.
        const QUEST_OBJ: u32 = 0x0000_2315;
        const DIAL_OBJ: u32 = 0x004E_314B;
        const OWN_INDEX: u32 = 0x07 << 24;
        let quest_full = OWN_INDEX | QUEST_OBJ;
        let dial_full = OWN_INDEX | DIAL_OBJ;
        let info_full = OWN_INDEX | 0x004E_315F;

        let handle_id = create_empty_plugin_handle("Test.esp", Some("fo4"));
        let inserted = {
            let mut store = plugin_handle_store_ref().lock().unwrap();
            let slot = store.get_mut(&handle_id).unwrap();
            slot.parsed.root_items.push(ParsedItem::Group(ParsedGroup {
                label: quest_full.to_le_bytes(),
                group_type: QUEST_CHILD_GROUP,
                tail: Bytes::new(),
                children: vec![ParsedItem::Record(make_record("DIAL", dial_full, None))],
            }));

            // Caller passes the 24-bit object-id, NOT the full form_id.
            insert_topic_child_record_in_slot(slot, DIAL_OBJ, make_record("INFO", info_full, None))
                .expect("insert")
        };
        assert!(
            inserted,
            "INFO must insert even when the caller passes the 24-bit object-id"
        );

        let store = plugin_handle_store_ref().lock().unwrap();
        let slot = store.get(&handle_id).unwrap();
        let ParsedItem::Group(quest_group) = &slot.parsed.root_items[0] else {
            panic!("expected quest-child group");
        };
        let topic_group = quest_group
            .children
            .iter()
            .find_map(|item| match item {
                ParsedItem::Group(group)
                    if group.group_type == TOPIC_CHILD_GROUP
                        // Label must be the FULL DIAL form_id, not the 24-bit id.
                        && group.label == dial_full.to_le_bytes() =>
                {
                    Some(group)
                }
                _ => None,
            })
            .expect("topic-child group labelled with the FULL DIAL form_id");
        assert!(topic_group.children.iter().any(|item| matches!(
            item,
            ParsedItem::Record(record)
                if record.signature.as_str() == "INFO" && record.form_id == info_full
        )));
    }

    fn child_insert_fixture() -> ParsedPlugin {
        const QUEST_A: u32 = 0x0100_1000;
        const QUEST_B: u32 = 0x0100_1001;
        const EXISTING_DIAL: u32 = 0x0100_2000;
        let mut plugin = empty_plugin(Some("fo4"));
        plugin.header.next_object_id = 0x3000;
        plugin.root_items.push(group(
            0,
            *b"QUST",
            vec![
                ParsedItem::Record(make_record("QUST", QUEST_A, Some("QuestA"))),
                group(
                    QUEST_CHILD_GROUP,
                    QUEST_A.to_le_bytes(),
                    vec![ParsedItem::Record(make_record(
                        "DIAL",
                        EXISTING_DIAL,
                        Some("ExistingDial"),
                    ))],
                ),
                ParsedItem::Record(make_record("QUST", QUEST_B, Some("QuestB"))),
            ],
        ));
        plugin
    }

    fn child_insert_bytes(handle_id: u64) -> Vec<u8> {
        let store = plugin_handle_store_ref().lock().unwrap();
        let mut parsed = store.get(&handle_id).unwrap().parsed.clone();
        build_plugin_bytes(&mut parsed).expect("serialize child-insert fixture")
    }

    #[test]
    fn indexed_child_inserts_match_serial_bytes_for_interleaved_parents() {
        const QUEST_A: u32 = 0x0100_1000;
        const QUEST_B: u32 = 0x0100_1001;
        const DIAL_A: u32 = 0x0100_2100;
        const DIAL_B: u32 = 0x0100_2101;
        const DIAL_C: u32 = 0x0100_2102;
        let serial_handle = insert_plugin_handle(
            child_insert_fixture(),
            LocalizedStringsState::default(),
        );
        let indexed_handle = insert_plugin_handle(
            child_insert_fixture(),
            LocalizedStringsState::default(),
        );
        let quest_records = [
            (QUEST_B, make_record("DIAL", DIAL_A, Some("DialA"))),
            (QUEST_A, make_record("SCEN", 0x0100_2200, Some("SceneA"))),
            (QUEST_B, make_record("DIAL", DIAL_B, Some("DialB"))),
            (QUEST_A, make_record("DIAL", DIAL_C, Some("DialC"))),
        ];
        let mut serial_outcomes = Vec::new();
        let mut indexed_outcomes = Vec::new();
        {
            let mut store = plugin_handle_store_ref().lock().unwrap();
            let serial = store.get_mut(&serial_handle).unwrap();
            for (parent, record) in quest_records.iter().cloned() {
                serial_outcomes.push(
                    insert_quest_child_record_in_slot(serial, parent, record).expect("serial quest"),
                );
            }
            let indexed = store.get_mut(&indexed_handle).unwrap();
            let mut index = build_quest_child_insert_index(indexed);
            for (parent, record) in quest_records.iter().cloned() {
                indexed_outcomes.push(
                    insert_quest_child_record_indexed_in_slot(
                        indexed,
                        &mut index,
                        parent,
                        record,
                    )
                    .expect("indexed quest"),
                );
            }
            assert_eq!(index.fast_inserts(), quest_records.len());
            assert_eq!(index.serial_fallbacks(), 0);
        }
        assert_eq!(indexed_outcomes, serial_outcomes);

        let info_records = [
            (DIAL_B & 0x00FF_FFFF, make_record("INFO", 0x0100_3100, None)),
            (DIAL_A & 0x00FF_FFFF, make_record("INFO", 0x0100_3101, None)),
            (DIAL_B & 0x00FF_FFFF, make_record("INFO", 0x0100_3102, None)),
            (DIAL_C & 0x00FF_FFFF, make_record("INFO", 0x0100_3103, None)),
        ];
        serial_outcomes.clear();
        indexed_outcomes.clear();
        {
            let mut store = plugin_handle_store_ref().lock().unwrap();
            let serial = store.get_mut(&serial_handle).unwrap();
            for (parent, record) in info_records.iter().cloned() {
                serial_outcomes.push(
                    insert_topic_child_record_in_slot(serial, parent, record).expect("serial topic"),
                );
            }
            let indexed = store.get_mut(&indexed_handle).unwrap();
            let mut index = build_topic_child_insert_index(indexed);
            for (parent, record) in info_records.iter().cloned() {
                indexed_outcomes.push(
                    insert_topic_child_record_indexed_in_slot(
                        indexed,
                        &mut index,
                        parent,
                        record,
                    )
                    .expect("indexed topic"),
                );
            }
            assert_eq!(index.fast_inserts(), info_records.len());
            assert_eq!(index.serial_fallbacks(), 0);
        }
        assert_eq!(indexed_outcomes, serial_outcomes);
        assert_eq!(child_insert_bytes(indexed_handle), child_insert_bytes(serial_handle));
        plugin_handle_close_native(serial_handle);
        plugin_handle_close_native(indexed_handle);
    }

    #[test]
    fn indexed_child_insert_fallbacks_match_serial_side_effects() {
        const QUEST_A: u32 = 0x0100_1000;
        const DIAL_A: u32 = 0x0100_2000;
        const REPLACED: u32 = 0x0100_3200;
        const MISSING_PARENT: u32 = 0x0000_DEAD;
        let mut fixture = child_insert_fixture();
        let ParsedItem::Group(top_quest_group) = &mut fixture.root_items[0] else {
            panic!("expected top QUST group");
        };
        top_quest_group
            .children
            .push(ParsedItem::Record(make_record("QUST", 0, Some("MalformedZeroQuest"))));
        fixture.root_items.insert(
            0,
            group(
                QUEST_CHILD_GROUP,
                0x0200_1000u32.to_le_bytes(),
                vec![ParsedItem::Record(make_record(
                    "DIAL",
                    0x0200_2000,
                    Some("EarlierDuplicateDial"),
                ))],
            ),
        );
        fixture.root_items.push(group(
            0,
            *b"INFO",
            vec![ParsedItem::Record(make_record("INFO", REPLACED, Some("OldInfo")))],
        ));
        let serial_handle = insert_plugin_handle(fixture.clone(), LocalizedStringsState::default());
        let indexed_handle = insert_plugin_handle(fixture, LocalizedStringsState::default());
        let operations = [
            (DIAL_A, make_record("INFO", 0x0100_3100, Some("FreshAmbiguousParent"))),
            (DIAL_A, make_record("INFO", REPLACED, Some("Replacement"))),
            (DIAL_A, make_record("INFO", REPLACED, Some("LastReplacement"))),
            (
                MISSING_PARENT,
                make_record("INFO", 0x0100_FFFF, Some("MissingParent")),
            ),
            (DIAL_A, make_record("DIAL", 0x0100_3300, Some("WrongType"))),
        ];
        let mut serial_outcomes = Vec::new();
        let mut indexed_outcomes = Vec::new();
        {
            let mut store = plugin_handle_store_ref().lock().unwrap();
            let serial = store.get_mut(&serial_handle).unwrap();
            for (parent, record) in operations.iter().cloned() {
                serial_outcomes.push(
                    insert_topic_child_record_in_slot(serial, parent, record).expect("serial topic"),
                );
            }
            let indexed = store.get_mut(&indexed_handle).unwrap();
            let mut index = build_topic_child_insert_index(indexed);
            for (parent, record) in operations.iter().cloned() {
                indexed_outcomes.push(
                    insert_topic_child_record_indexed_in_slot(
                        indexed,
                        &mut index,
                        parent,
                        record,
                    )
                    .expect("indexed topic"),
                );
            }
            assert_eq!(index.fast_inserts(), 0);
            assert_eq!(index.serial_fallbacks(), operations.len());
        }
        assert_eq!(indexed_outcomes, serial_outcomes);

        let quest_operations = [
            (QUEST_A, make_record("SCEN", 0x0100_3400, Some("Scene"))),
            (QUEST_A, make_record("SCEN", 0x0100_3400, Some("SceneReplacement"))),
            (
                MISSING_PARENT,
                make_record("DIAL", 0x0100_3401, Some("MissingQuest")),
            ),
            (0, make_record("DIAL", 0x0100_3402, Some("ZeroQuest"))),
        ];
        {
            let mut store = plugin_handle_store_ref().lock().unwrap();
            let serial = store.get_mut(&serial_handle).unwrap();
            for (parent, record) in quest_operations.iter().cloned() {
                serial_outcomes.push(
                    insert_quest_child_record_in_slot(serial, parent, record).expect("serial quest"),
                );
            }
            let indexed = store.get_mut(&indexed_handle).unwrap();
            let mut index = build_quest_child_insert_index(indexed);
            for (parent, record) in quest_operations.iter().cloned() {
                indexed_outcomes.push(
                    insert_quest_child_record_indexed_in_slot(
                        indexed,
                        &mut index,
                        parent,
                        record,
                    )
                    .expect("indexed quest"),
                );
            }
            assert_eq!(index.fast_inserts(), 1);
            assert_eq!(index.serial_fallbacks(), 3);
        }
        assert_eq!(indexed_outcomes, serial_outcomes);
        assert_eq!(child_insert_bytes(indexed_handle), child_insert_bytes(serial_handle));
        plugin_handle_close_native(serial_handle);
        plugin_handle_close_native(indexed_handle);
    }

    #[test]
    fn indexed_quest_child_insert_falls_back_for_ambiguous_parent_topology() {
        const QUEST_A: u32 = 0x0100_1000;
        const QUEST_B: u32 = 0x0100_1001;
        let mut fixture = child_insert_fixture();
        let ParsedItem::Group(top_quest_group) = &mut fixture.root_items[0] else {
            panic!("expected top QUST group");
        };
        top_quest_group.children.extend([
            ParsedItem::Record(make_record("QUST", QUEST_B, Some("DuplicateQuestB"))),
            group(
                QUEST_CHILD_GROUP,
                QUEST_A.to_le_bytes(),
                vec![ParsedItem::Record(make_record(
                    "DIAL",
                    0x0100_2001,
                    Some("DuplicateQuestAChildGroup"),
                ))],
            ),
        ]);
        let serial_handle = insert_plugin_handle(fixture.clone(), LocalizedStringsState::default());
        let indexed_handle = insert_plugin_handle(fixture, LocalizedStringsState::default());
        let operations = [
            (QUEST_A, make_record("SCEN", 0x0100_3500, Some("AmbiguousGroup"))),
            (QUEST_B, make_record("DIAL", 0x0100_3501, Some("AmbiguousQuest"))),
        ];

        let mut serial_outcomes = Vec::new();
        let mut indexed_outcomes = Vec::new();
        {
            let mut store = plugin_handle_store_ref().lock().unwrap();
            let serial = store.get_mut(&serial_handle).unwrap();
            for (parent, record) in operations.iter().cloned() {
                serial_outcomes.push(
                    insert_quest_child_record_in_slot(serial, parent, record).expect("serial quest"),
                );
            }
            let indexed = store.get_mut(&indexed_handle).unwrap();
            let mut index = build_quest_child_insert_index(indexed);
            for (parent, record) in operations.iter().cloned() {
                indexed_outcomes.push(
                    insert_quest_child_record_indexed_in_slot(
                        indexed,
                        &mut index,
                        parent,
                        record,
                    )
                    .expect("indexed quest"),
                );
            }
            assert_eq!(index.fast_inserts(), 0);
            assert_eq!(index.serial_fallbacks(), operations.len());
        }

        assert_eq!(indexed_outcomes, serial_outcomes);
        assert_eq!(child_insert_bytes(indexed_handle), child_insert_bytes(serial_handle));
        plugin_handle_close_native(serial_handle);
        plugin_handle_close_native(indexed_handle);
    }

    fn child_insert_scaling_fixture(filler_records: u32, quest_count: u32) -> ParsedPlugin {
        let mut plugin = empty_plugin(Some("fo4"));
        plugin.root_items.push(group(
            0,
            *b"WRLD",
            (0..filler_records)
                .map(|index| {
                    ParsedItem::Record(make_record("REFR", 0x0200_0000 + index, None))
                })
                .collect(),
        ));
        let mut quest_items = Vec::with_capacity((quest_count * 2) as usize);
        for index in 0..quest_count {
            let quest = 0x0101_0000 + index;
            let dialogue = 0x0102_0000 + index;
            quest_items.push(ParsedItem::Record(make_record("QUST", quest, None)));
            quest_items.push(group(
                QUEST_CHILD_GROUP,
                quest.to_le_bytes(),
                vec![ParsedItem::Record(make_record("DIAL", dialogue, None))],
            ));
        }
        plugin.root_items.push(group(0, *b"QUST", quest_items));
        plugin
    }

    #[test]
    #[ignore = "release scaling benchmark"]
    fn indexed_child_insert_scaling() {
        use std::time::Instant;

        const QUEST_COUNT: u32 = 32;
        const DIALOGUE_INSERTS: u32 = 256;
        const INFO_INSERTS: u32 = 512;
        for filler_records in [5_000, 20_000] {
            let fixture = child_insert_scaling_fixture(filler_records, QUEST_COUNT);
            let serial_handle =
                insert_plugin_handle(fixture.clone(), LocalizedStringsState::default());
            let indexed_handle = insert_plugin_handle(fixture, LocalizedStringsState::default());
            let quest_operations = (0..DIALOGUE_INSERTS)
                .map(|index| {
                    (
                        0x0101_0000 + index % QUEST_COUNT,
                        make_record("DIAL", 0x0103_0000 + index, None),
                    )
                })
                .collect::<Vec<_>>();
            let topic_operations = (0..INFO_INSERTS)
                .map(|index| {
                    (
                        (0x0103_0000 + index % DIALOGUE_INSERTS) & 0x00FF_FFFF,
                        make_record("INFO", 0x0104_0000 + index, None),
                    )
                })
                .collect::<Vec<_>>();

            let (serial_quest, indexed_quest, indexed_quest_records) = {
                let mut store = plugin_handle_store_ref().lock().unwrap();
                let serial_started = Instant::now();
                let serial = store.get_mut(&serial_handle).unwrap();
                for (parent, record) in quest_operations.iter().cloned() {
                    assert!(insert_quest_child_record_in_slot(serial, parent, record).unwrap());
                }
                let serial_elapsed = serial_started.elapsed();

                let indexed_started = Instant::now();
                let indexed = store.get_mut(&indexed_handle).unwrap();
                let mut index = build_quest_child_insert_index(indexed);
                let indexed_record_count = index.record_form_ids.len();
                for (parent, record) in quest_operations.iter().cloned() {
                    assert!(
                        insert_quest_child_record_indexed_in_slot(
                            indexed,
                            &mut index,
                            parent,
                            record,
                        )
                        .unwrap()
                    );
                }
                assert_eq!(index.fast_inserts(), quest_operations.len());
                assert_eq!(index.serial_fallbacks(), 0);
                (serial_elapsed, indexed_started.elapsed(), indexed_record_count)
            };

            let (serial_topic, indexed_topic, indexed_topic_records) = {
                let mut store = plugin_handle_store_ref().lock().unwrap();
                let serial_started = Instant::now();
                let serial = store.get_mut(&serial_handle).unwrap();
                for (parent, record) in topic_operations.iter().cloned() {
                    assert!(insert_topic_child_record_in_slot(serial, parent, record).unwrap());
                }
                let serial_elapsed = serial_started.elapsed();

                let indexed_started = Instant::now();
                let indexed = store.get_mut(&indexed_handle).unwrap();
                let mut index = build_topic_child_insert_index(indexed);
                let indexed_record_count = index.record_form_ids.len();
                for (parent, record) in topic_operations.iter().cloned() {
                    assert!(
                        insert_topic_child_record_indexed_in_slot(
                            indexed,
                            &mut index,
                            parent,
                            record,
                        )
                        .unwrap()
                    );
                }
                assert_eq!(index.fast_inserts(), topic_operations.len());
                assert_eq!(index.serial_fallbacks(), 0);
                (serial_elapsed, indexed_started.elapsed(), indexed_record_count)
            };

            assert_eq!(child_insert_bytes(indexed_handle), child_insert_bytes(serial_handle));
            eprintln!(
                "child_insert_scaling filler_records={filler_records} dialogues={} infos={} quest_index_records={indexed_quest_records} topic_index_records={indexed_topic_records} serial_quest_ms={:.3} indexed_quest_ms={:.3} serial_topic_ms={:.3} indexed_topic_ms={:.3}",
                quest_operations.len(),
                topic_operations.len(),
                serial_quest.as_secs_f64() * 1000.0,
                indexed_quest.as_secs_f64() * 1000.0,
                serial_topic.as_secs_f64() * 1000.0,
                indexed_topic.as_secs_f64() * 1000.0,
            );
            plugin_handle_close_native(serial_handle);
            plugin_handle_close_native(indexed_handle);
        }
    }

    // ── Single-pass batch content-replace (perf + correctness) ─────────────────
    //
    // The per-record replace path scans the whole GRUP tree once per record
    // (O(changed × n)); the batch path must do ONE traversal (O(n + changed)).
    // These tests pin both correctness and that linearity.

    fn group(group_type: i32, label: [u8; 4], children: Vec<ParsedItem>) -> ParsedItem {
        ParsedItem::Group(ParsedGroup {
            label,
            group_type,
            tail: Bytes::new(),
            children,
        })
    }

    /// Build a plugin with `records_per_group` WEAP records in each of two
    /// nested top-level groups, the SECOND group placed last so a naive
    /// per-record scan of a record in it would traverse the whole first group.
    fn nested_plugin(records_per_group: u32) -> ParsedPlugin {
        let mut plugin = empty_plugin(Some("fo4"));
        for g in 0..2u32 {
            let mut children = Vec::new();
            for i in 0..records_per_group {
                let fid = 0xFF00_0000 | (g << 16) | i;
                children.push(ParsedItem::Record(make_record("WEAP", fid, Some("W"))));
            }
            // Wrap each block in a top-level group then a nested child group so
            // the walker has to recurse (group_type values are arbitrary here).
            let inner = group(6, [g as u8, 0, 0, 0], children);
            plugin.root_items.push(group(0, *b"WEAP", vec![inner]));
        }
        plugin
    }

    #[test]
    fn batch_replace_applies_exactly_the_targeted_records() {
        let mut plugin = nested_plugin(50);
        // Replace one record in the FIRST group and one in the SECOND (last) group,
        // plus include a non-existent form_id (must be ignored) and a
        // signature-mismatch (must be rejected, left untouched).
        let target_a = 0xFF00_0000 | (0 << 16) | 7; // first group
        let target_b = 0xFF00_0000 | (1 << 16) | 42; // second (last) group
        let missing = 0xFF00_0000 | (9 << 16) | 1;
        let mismatch = 0xFF00_0000 | (1 << 16) | 5; // exists but we give wrong sig

        let mut repl_a = make_record("WEAP", target_a, Some("W"));
        repl_a.subrecords.push(ParsedSubrecord {
            signature: SmolStr::new_static("DNAM"),
            data: Bytes::from_static(&[1, 2, 3, 4]),
            semantic_type: None,
        });
        let mut repl_b = make_record("WEAP", target_b, Some("W"));
        repl_b.subrecords.push(ParsedSubrecord {
            signature: SmolStr::new_static("DNAM"),
            data: Bytes::from_static(&[5, 6, 7, 8]),
            semantic_type: None,
        });
        let repl_missing = make_record("WEAP", missing, Some("W"));
        let repl_mismatch = make_record("ARMO", mismatch, Some("W")); // wrong signature

        let handle_id = insert_plugin_handle(plugin, LocalizedStringsState::default());
        populate_all_sections(handle_id);
        let mut store = plugin_handle_store_ref().lock().unwrap();
        let slot = store.get_mut(&handle_id).expect("handle");

        let applied = replace_parsed_records_contents_in_slot_batch(
            slot,
            vec![repl_a, repl_b, repl_missing, repl_mismatch],
        );

        // Exactly the two valid, present, guard-passing targets applied.
        let applied_set: std::collections::HashSet<u32> = applied.iter().copied().collect();
        assert_eq!(applied_set.len(), 2, "exactly 2 applied, got {applied:?}");
        assert!(applied_set.contains(&target_a));
        assert!(applied_set.contains(&target_b));

        // Verify the two targets carry the new DNAM and others are untouched.
        let mut found_a = false;
        let mut found_b = false;
        let mut others_intact = 0usize;
        fn walk<'a>(items: &'a [ParsedItem], f: &mut impl FnMut(&'a ParsedRecord)) {
            for item in items {
                match item {
                    ParsedItem::Record(r) => f(r),
                    ParsedItem::Group(g) => walk(&g.children, f),
                }
            }
        }
        walk(&slot.parsed.root_items, &mut |r| {
            if r.form_id == target_a {
                found_a = r.subrecords.iter().any(|s| s.signature.as_str() == "DNAM");
            } else if r.form_id == target_b {
                found_b = r.subrecords.iter().any(|s| s.signature.as_str() == "DNAM");
            } else {
                // every other record keeps its single EDID subrecord only
                if r.subrecords.iter().all(|s| s.signature.as_str() != "DNAM") {
                    others_intact += 1;
                }
            }
        });
        assert!(found_a, "target_a got new DNAM");
        assert!(found_b, "target_b got new DNAM");
        assert_eq!(others_intact, 98, "all 98 non-target records untouched");
    }

    #[test]
    fn batch_replace_is_single_pass_linear() {
        // N records; replace K. A single traversal visits at most N_nodes (records
        // + groups) once. Prove visits ≤ N + groups + K, i.e. NOT K × N.
        let per_group = 500u32;
        let mut plugin = nested_plugin(per_group); // 1000 records, 4 groups
        // sanity: count nodes
        fn count_nodes(items: &[ParsedItem]) -> usize {
            items
                .iter()
                .map(|i| match i {
                    ParsedItem::Record(_) => 1,
                    ParsedItem::Group(g) => 1 + count_nodes(&g.children),
                })
                .sum()
        }
        let total_nodes = count_nodes(&plugin.root_items);

        // Replace K records spread across BOTH groups, incl. the last record of
        // the last group (worst case for a linear scan).
        let mut replacements = Vec::new();
        let mut expected_targets = Vec::new();
        for &(g, i) in &[(0u32, 0u32), (0, 250), (1, 0), (1, 250), (1, per_group - 1)] {
            let fid = 0xFF00_0000 | (g << 16) | i;
            replacements.push(make_record("WEAP", fid, Some("W")));
            expected_targets.push(fid);
        }
        let k = replacements.len();

        let mut by_form_id: HashMap<u32, ParsedRecord> = HashMap::new();
        for r in replacements {
            by_form_id.insert(r.form_id, r);
        }
        let mut applied = Vec::new();
        let mut visits = 0usize;
        replace_record_contents_in_items_batch(
            &mut plugin.root_items,
            &mut by_form_id,
            &mut applied,
            &mut visits,
        );

        assert_eq!(applied.len(), k, "all {k} targets applied");
        // LINEARITY: a single pass visits each node at most once → visits ≤ total
        // nodes. (A K×N per-record scan would be ~K × total_nodes.) Allow ==.
        assert!(
            visits <= total_nodes,
            "batch replace must be single-pass: visits={visits} total_nodes={total_nodes} (K×N would be ~{})",
            k * total_nodes
        );
        // And it must have visited enough to reach the last-group records.
        assert!(
            visits >= total_nodes / 2,
            "should traverse into the second group"
        );
    }
}

#[cfg(test)]
mod interior_cell_tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Test helpers
    // -----------------------------------------------------------------------

    /// Make a minimal interior CELL record: sig=CELL, form_id, DATA subrecord
    /// with IsInteriorCell bit (bit 0) set.
    fn make_test_cell_record(form_id: u32) -> ParsedRecord {
        let data_byte: u8 = 0x01; // IsInteriorCell
        ParsedRecord {
            signature: SmolStr::new_static("CELL"),
            form_id,
            flags: 0,
            version_control: 0,
            form_version: None,
            version2: None,
            subrecords: vec![ParsedSubrecord {
                signature: SmolStr::new_static("DATA"),
                data: Bytes::from(vec![data_byte]),
                semantic_type: None,
            }],
            raw_payload: None,
            parse_error: None,
        }
    }

    fn make_test_refr_record(form_id: u32) -> ParsedRecord {
        ParsedRecord {
            signature: SmolStr::new_static("REFR"),
            form_id,
            flags: 0,
            version_control: 0,
            form_version: None,
            version2: None,
            subrecords: Vec::new(),
            raw_payload: None,
            parse_error: None,
        }
    }

    fn new_empty_plugin_handle() -> u64 {
        let plugin = ParsedPlugin {
            plugin_name: "Test.esp".to_string(),
            file_path: String::new(),
            header_size: MODERN_HEADER_SIZE,
            header: ParsedPluginHeader {
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
                form_version: None,
                version2: None,
                hedr_raw: None,
                raw_subrecords: Vec::new(),
            },
            root_items: Vec::new(),
            game: Some("fo4".to_string()),
        };
        insert_plugin_handle(plugin, LocalizedStringsState::default())
    }

    /// Find the top-level group with sig `label` in the plugin's root_items.
    fn find_top_group_in_items<'a>(
        items: &'a [ParsedItem],
        label: &[u8; 4],
    ) -> Option<&'a ParsedGroup> {
        items.iter().find_map(|item| match item {
            ParsedItem::Group(g) if g.group_type == 0 && g.label == *label => Some(g),
            _ => None,
        })
    }

    /// Find a child group of `parent` with the given group_type and integer
    /// label (encoded little-endian).
    fn find_child_group_by_int<'a>(
        parent: &'a ParsedGroup,
        group_type: i32,
        bucket: i32,
    ) -> Option<&'a ParsedGroup> {
        let label = bucket.to_le_bytes();
        parent.children.iter().find_map(|item| match item {
            ParsedItem::Group(g) if g.group_type == group_type && g.label == label => Some(g),
            _ => None,
        })
    }

    /// Find a child group of `parent` with the given group_type and u32 label
    /// (encoded little-endian), used for Cell-Children groups keyed by FormID.
    fn find_child_group_by_formid<'a>(
        parent: &'a ParsedGroup,
        group_type: i32,
        form_id: u32,
    ) -> Option<&'a ParsedGroup> {
        let label = form_id.to_le_bytes();
        parent.children.iter().find_map(|item| match item {
            ParsedItem::Group(g) if g.group_type == group_type && g.label == label => Some(g),
            _ => None,
        })
    }

    /// Return true if `group.children` contains a Record with the given FormID.
    fn group_has_record(group: &ParsedGroup, form_id: u32) -> bool {
        group.children.iter().any(|item| match item {
            ParsedItem::Record(r) if r.form_id == form_id => true,
            _ => false,
        })
    }

    /// Count CELL records with the given FormID anywhere in `items`.
    fn count_cell_records_in_items(items: &[ParsedItem], form_id: u32) -> usize {
        let mut count = 0;
        for item in items {
            match item {
                ParsedItem::Record(r) if r.signature.as_str() == "CELL" && r.form_id == form_id => {
                    count += 1;
                }
                ParsedItem::Group(g) => {
                    count += count_cell_records_in_items(&g.children, form_id);
                }
                _ => {}
            }
        }
        count
    }

    /// Find the Cell-Children group (type 6) for `cell_form_id` anywhere in
    /// root_items.
    fn find_cell_child_group_in_items<'a>(
        items: &'a [ParsedItem],
        cell_form_id: u32,
    ) -> Option<&'a ParsedGroup> {
        let label = cell_form_id.to_le_bytes();
        for item in items {
            if let ParsedItem::Group(g) = item {
                if g.group_type == CELL_CHILD_GROUP && g.label == label {
                    return Some(g);
                }
                if let Some(found) = find_cell_child_group_in_items(&g.children, cell_form_id) {
                    return Some(found);
                }
            }
        }
        None
    }

    // -----------------------------------------------------------------------
    // Tests
    // -----------------------------------------------------------------------

    /// 0x00275EDE = 2580190 decimal  -> block = 2580190 % 10 = 0
    ///                               -> subblock = (2580190 / 10) % 10 = 9
    #[test]
    fn interior_cell_lands_in_block_and_subblock_by_formid() {
        let handle = new_empty_plugin_handle();
        let cell = make_test_cell_record(0x00275EDE);
        ensure_interior_cell_and_child_group(handle, cell).expect("emit interior cell");

        let store = plugin_handle_store_ref().lock().unwrap();
        let slot = store.get(&handle).unwrap();
        let items = &slot.parsed.root_items;

        let cell_top = find_top_group_in_items(items, b"CELL").expect("CELL top group");
        let block = find_child_group_by_int(cell_top, INTERIOR_CELL_BLOCK, 0).expect("block 0");
        let subblock =
            find_child_group_by_int(block, INTERIOR_CELL_SUBBLOCK, 9).expect("subblock 9");

        assert!(
            group_has_record(subblock, 0x00275EDE),
            "CELL record must be present in subblock"
        );
        assert!(
            find_child_group_by_formid(subblock, CELL_CHILD_GROUP, 0x00275EDE).is_some(),
            "Cell-Children group (type 6) must follow the CELL record"
        );
    }

    #[test]
    fn interior_cell_bucket_uses_local_object_id() {
        let handle = new_empty_plugin_handle();
        let cell = make_test_cell_record(0x076240BB);
        ensure_interior_cell_and_child_group(handle, cell).expect("emit interior cell");

        let store = plugin_handle_store_ref().lock().unwrap();
        let slot = store.get(&handle).unwrap();
        let cell_top = find_top_group_in_items(&slot.parsed.root_items, b"CELL").expect("CELL top");
        let block = find_child_group_by_int(cell_top, INTERIOR_CELL_BLOCK, 9).expect("block 9");
        let subblock =
            find_child_group_by_int(block, INTERIOR_CELL_SUBBLOCK, 9).expect("subblock 9");

        assert!(
            group_has_record(subblock, 0x076240BB),
            "CELL record must be bucketed by local object id"
        );
        assert!(
            find_child_group_by_int(cell_top, INTERIOR_CELL_BLOCK, 1).is_none(),
            "raw FormID bucketing would incorrectly create block 1"
        );
    }

    #[test]
    fn insert_interior_cell_with_children_bucket_uses_local_object_id() {
        let handle = new_empty_plugin_handle();
        let cell = make_test_cell_record(0x076240BB);
        let temporary = vec![make_test_refr_record(0x0762EE15)];
        insert_interior_cell_with_children(handle, cell, Vec::new(), temporary)
            .expect("insert interior cell with children");

        let store = plugin_handle_store_ref().lock().unwrap();
        let slot = store.get(&handle).unwrap();
        let cell_top = find_top_group_in_items(&slot.parsed.root_items, b"CELL").expect("CELL top");
        let block = find_child_group_by_int(cell_top, INTERIOR_CELL_BLOCK, 9).expect("block 9");
        let subblock =
            find_child_group_by_int(block, INTERIOR_CELL_SUBBLOCK, 9).expect("subblock 9");
        assert!(
            group_has_record(subblock, 0x076240BB),
            "CELL record present"
        );

        let cell_child = find_child_group_by_formid(subblock, CELL_CHILD_GROUP, 0x076240BB)
            .expect("Cell-Children group");
        let temporary_group = find_child_group_by_formid(cell_child, TEMPORARY_GROUP, 0x076240BB)
            .expect("Temporary section");
        assert!(
            group_has_record(temporary_group, 0x0762EE15),
            "temporary REFR present"
        );
    }

    /// Pre-insert a DATA-only stub for FormID 0x00275EDE, then emit the real
    /// CELL and assert exactly one CELL record for that FormID carrying the new fields.
    #[test]
    fn interior_cell_emit_replaces_existing_stub() {
        let handle = new_empty_plugin_handle();

        // Insert a stub CELL into the tree before calling our fn.
        {
            let mut store = plugin_handle_store_ref().lock().unwrap();
            let slot = store.get_mut(&handle).unwrap();
            let cell_stub = make_test_cell_record(0x00275EDE);
            slot.parsed.root_items.push(ParsedItem::Record(cell_stub));
        }

        // Now emit the real cell (with an extra subrecord to distinguish it).
        let mut real_cell = make_test_cell_record(0x00275EDE);
        real_cell.subrecords.push(ParsedSubrecord {
            signature: SmolStr::new_static("FULL"),
            data: Bytes::from(b"Interior Cell\0".to_vec()),
            semantic_type: None,
        });
        ensure_interior_cell_and_child_group(handle, real_cell).expect("emit real cell");

        let store = plugin_handle_store_ref().lock().unwrap();
        let slot = store.get(&handle).unwrap();
        let count = count_cell_records_in_items(&slot.parsed.root_items, 0x00275EDE);
        assert_eq!(count, 1, "exactly one CELL record for this FormID");

        // Verify the surviving record carries the FULL subrecord (is the real one).
        let cell_top = find_top_group_in_items(&slot.parsed.root_items, b"CELL").expect("CELL top");
        // 0x00275EDE = 2580190 decimal -> block = 2580190 % 10 = 0; subblock = (2580190/10) % 10 = 9
        let block = find_child_group_by_int(cell_top, INTERIOR_CELL_BLOCK, 0).expect("block 0");
        let subblock =
            find_child_group_by_int(block, INTERIOR_CELL_SUBBLOCK, 9).expect("subblock 9");
        let cell_record = subblock
            .children
            .iter()
            .find_map(|item| match item {
                ParsedItem::Record(r) if r.form_id == 0x00275EDE => Some(r),
                _ => None,
            })
            .expect("CELL record in subblock");
        assert!(
            cell_record
                .subrecords
                .iter()
                .any(|s| s.signature.as_str() == "FULL"),
            "surviving record must carry FULL subrecord (the real cell, not the stub)"
        );
    }

    #[test]
    fn placed_child_lands_in_temporary_group_of_existing_cell() {
        let handle = new_empty_plugin_handle();
        ensure_interior_cell_and_child_group(handle, make_test_cell_record(0x00275EDE)).unwrap();

        let refr = make_test_refr_record(0x002F74A0);
        let inserted =
            insert_placed_child_into_cell_group(handle, 0x00275EDE, TEMPORARY_GROUP, refr)
                .expect("insert child");
        assert!(inserted, "child must be inserted into existing cell");

        let store = plugin_handle_store_ref().lock().unwrap();
        let slot = store.get(&handle).unwrap();
        let cell_child = find_cell_child_group_in_items(&slot.parsed.root_items, 0x00275EDE)
            .expect("Cell-Children group");
        let temp = find_child_group_by_formid(cell_child, TEMPORARY_GROUP, 0x00275EDE)
            .expect("Temporary group (type 9) inside cell children");
        assert!(
            group_has_record(temp, 0x002F74A0),
            "REFR must appear in the Temporary section"
        );
    }

    #[test]
    fn placed_child_returns_false_when_cell_missing() {
        let handle = new_empty_plugin_handle();
        let refr = make_test_refr_record(0x002F74A0);
        let inserted =
            insert_placed_child_into_cell_group(handle, 0x00DEAD00, TEMPORARY_GROUP, refr).unwrap();
        assert!(
            !inserted,
            "must return false when no cell-child group exists"
        );
    }

    /// Batch insert: one call builds the full Block/Sub-Block/CELL/Cell-Children
    /// subtree with both placed-child sections populated, no whole-tree search.
    #[test]
    fn insert_interior_cell_with_children_builds_full_subtree() {
        let handle = new_empty_plugin_handle();
        let cell = make_test_cell_record(0x00275EDE);
        let persistent = vec![make_test_refr_record(0x002F749F)];
        let temporary = vec![make_test_refr_record(0x002F74A0)];
        insert_interior_cell_with_children(handle, cell, persistent, temporary)
            .expect("insert interior cell with children");

        let store = plugin_handle_store_ref().lock().unwrap();
        let slot = store.get(&handle).unwrap();
        let items = &slot.parsed.root_items;

        // 0x00275EDE -> block 0, subblock 9
        let cell_top = find_top_group_in_items(items, b"CELL").expect("CELL top group");
        let block = find_child_group_by_int(cell_top, INTERIOR_CELL_BLOCK, 0).expect("block 0");
        let subblock =
            find_child_group_by_int(block, INTERIOR_CELL_SUBBLOCK, 9).expect("subblock 9");
        assert!(
            group_has_record(subblock, 0x00275EDE),
            "CELL record present in subblock"
        );

        let cell_child =
            find_cell_child_group_in_items(items, 0x00275EDE).expect("Cell-Children group");
        let persistent_group = find_child_group_by_formid(cell_child, PERSISTENT_GROUP, 0x00275EDE)
            .expect("Persistent section (type 8)");
        assert!(
            group_has_record(persistent_group, 0x002F749F),
            "persistent REFR present"
        );
        let temporary_group = find_child_group_by_formid(cell_child, TEMPORARY_GROUP, 0x00275EDE)
            .expect("Temporary section (type 9)");
        assert!(
            group_has_record(temporary_group, 0x002F74A0),
            "temporary REFR present"
        );
    }

    /// One-pass dedup: removes only the CELL records whose object id is in the
    /// set, leaving other CELLs untouched.
    #[test]
    fn remove_cell_records_by_object_id_drops_matching_stubs() {
        let handle = new_empty_plugin_handle();
        {
            let mut store = plugin_handle_store_ref().lock().unwrap();
            let slot = store.get_mut(&handle).unwrap();
            slot.parsed
                .root_items
                .push(ParsedItem::Record(make_test_cell_record(0x00275EDE)));
            slot.parsed
                .root_items
                .push(ParsedItem::Record(make_test_cell_record(0x00280000)));
        }
        let removed = remove_cell_records_by_object_id(handle, &[0x275EDE]).expect("remove");
        assert_eq!(removed, 1, "exactly one stub removed");

        let store = plugin_handle_store_ref().lock().unwrap();
        let slot = store.get(&handle).unwrap();
        assert_eq!(
            count_cell_records_in_items(&slot.parsed.root_items, 0x00275EDE),
            0,
            "matching stub removed"
        );
        assert_eq!(
            count_cell_records_in_items(&slot.parsed.root_items, 0x00280000),
            1,
            "non-matching stub kept"
        );
    }
}
