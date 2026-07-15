use plugin_runtime::{
    export_authoring_dir_native_impl, export_plugin_text_native_impl,
    import_plugin_text_native_impl, load_plugin_native_impl, plugin_to_bytes_native_impl,
    save_plugin_native_impl, validate_authoring_native,
};

// Swap the process-wide allocator to mimalloc. Windows' default heap is a
// well-known contention bottleneck for parallel Rust workloads — the rayon
// pass in `build_refs_section` was net-slower than sequential because of
// allocator serialization on FormKey / Vec / HashMap construction. mimalloc
// is a near-zero-effort drop-in that fixes the contention without any code
// changes elsewhere; it benefits every hot path that allocates (walker,
// fixups, EDID-decode, translate). On Linux this is also a real win for
// the same reason (glibc malloc is fine but mimalloc is faster on multi-
// threaded allocator-heavy workloads). Profiling builds can opt into DHAT
// instead; normal builds keep mimalloc.
#[cfg(feature = "dhat-heap")]
#[global_allocator]
static GLOBAL: dhat::Alloc = dhat::Alloc;

#[cfg(all(not(feature = "dhat-heap"), feature = "mimalloc-allocator"))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

// Return freed mimalloc pages to the OS. mimalloc keeps freed pages in its
// segment cache by default, so a phase that drops gigabytes shows no RSS fall
// until those pages are decommitted — earlier per-phase RSS even *rose* after a
// confirmed 2.56 GB free. `mi_collect(true)` forces the decommit; the conversion
// phase boundaries call it (via `trim_allocator_native`) so each track's frees
// actually land in peak RSS. No-op build unless mimalloc is the live allocator.
#[cfg(all(not(feature = "dhat-heap"), feature = "mimalloc-allocator"))]
pub fn trim_allocator() {
    // SAFETY: mi_collect has no preconditions — it walks the active mimalloc
    // heap and decommits cached pages; callable any time from any thread.
    unsafe {
        libmimalloc_sys::mi_collect(true);
    }
}

#[cfg(not(all(not(feature = "dhat-heap"), feature = "mimalloc-allocator")))]
pub fn trim_allocator() {}

use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
#[cfg(feature = "dhat-heap")]
use std::sync::Mutex;

#[cfg(feature = "dhat-heap")]
static DHAT_PROFILER: Mutex<Option<dhat::Profiler>> = Mutex::new(None);

#[path = "conflicts.rs"]
mod conflicts;
mod formkey_ops;
pub mod land;
pub mod nvnm;
pub mod plugin_runtime;
mod previs_merge;
pub mod schema_registry;
mod strings_py;
mod translated_store;
#[path = "validate.rs"]
mod validate;
mod validate_walker;
mod voice_reference;

pub(crate) fn default_job_count() -> usize {
    std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(8)
        .clamp(1, 8)
}

// ---------------------------------------------------------------------------
// Rust-native decode type tree — no Py<PyAny> retained after parse
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub enum FieldCodec {
    Bytes,
    ZString,
    LenString8,
    LenString16,
    LenString32,
    LString,
    Int8,
    Uint8,
    Int16,
    Uint16,
    Int32,
    Uint32,
    FormId,
    Float32,
    Int64,
    Uint64,
    FormIdArray,
    FixedString(usize),
}

#[derive(Clone)]
pub enum ConditionValue {
    Int(i128),
    Float(f64),
    Str(String),
    Bool(bool),
}

#[derive(Clone)]
pub struct Condition {
    field: String,
    operator: String,
    value: Option<ConditionValue>,
    values: Vec<ConditionValue>,
}

#[derive(Clone)]
pub struct Segment {
    name: String,
    codec: FieldCodec,
    offset: usize,
    size: usize,
    nested_spec: Option<Box<DecodeSpec>>,
    /// Conditions that must match the decode context for this segment to be
    /// present. Empty = always present. Used for `wbFromVersion`-style fields
    /// whose presence depends on `record_form_version` (e.g. ARMO/WEAP DAMA
    /// curve-table tail introduced in form_version 152).
    presence_conditions: Vec<Condition>,
}

#[derive(Clone)]
pub struct TailSegment {
    name: String,
    offset: usize,
    /// Codec for the trailing bytes. ``"bytes"`` (default) emits hex; the JSON-
    /// side decoder also honours ``"zstring"`` / ``"lstring"`` / ``"lenstring8"``
    /// / ``"lenstring16"`` / ``"lenstring32"`` for structs whose last field is a
    /// variable-length string outside the fixed ``struct:tokens`` codec
    /// (e.g. STAG.TNAM = ``struct:I`` formid + trailing zstring action).
    kind: String,
}

#[derive(Clone)]
pub struct Variant {
    name: String,
    spec: Box<DecodeSpec>,
    conditions: Vec<Condition>,
}

#[derive(Clone)]
pub enum DecodeSpec {
    Empty {
        empty_fields: Vec<String>,
    },
    Scalar {
        codec: FieldCodec,
    },
    Struct {
        row_size: usize,
        segments: Vec<Segment>,
        tail_segment: Option<TailSegment>,
        // If true, accept data shorter than row_size: decode only the segments
        // that fit within the input, drop the rest. Round-trip safety relies
        // on the caller using `kind=parsed_with_raw_fallback` so the encoder
        // falls back to raw_hex when re-encoding the partial value would
        // produce the wrong byte length. Used for variable-length xEdit
        // structs (e.g. CSTY.CSME / CSTY.CSLR with `nil, MinSize`).
        parse_partial: bool,
    },
    ArrayStruct {
        row_size: usize,
        segments: Vec<Segment>,
    },
    Union {
        variants: Vec<Variant>,
    },
    // Variable-offset struct whose segments may be of variable byte length
    // (interleaved scalars, length-prefixed arrays, nested structs, sibling-
    // discriminated unions). Used by NAVI/RACE/WTHR record types whose layout
    // cannot be expressed as a fixed `struct:tokens` codec.
    VariableStruct {
        segments: Vec<VarSegment>,
        parse_partial: bool,
    },
}

#[derive(Clone)]
pub struct VarUnionVariant {
    name: String,
    spec: DecodeSpec,
    condition: SelectorCondition,
}

#[derive(Clone)]
pub enum VarSegment {
    Scalar {
        name: String,
        codec: FieldCodec,
        presence_conditions: Vec<Condition>,
    },
    VariableString {
        name: String,
        codec: FieldCodec,
        presence_conditions: Vec<Condition>,
    },
    Array {
        name: String,
        count_codec: FieldCodec,
        element_spec: Box<DecodeSpec>,
        element_size: Option<usize>,
        presence_conditions: Vec<Condition>,
    },
    NestedStruct {
        name: String,
        spec: Box<DecodeSpec>,
        presence_conditions: Vec<Condition>,
    },
    Union {
        name: String,
        variants: Vec<VarUnionVariant>,
        selector: SelectorRef,
        presence_conditions: Vec<Condition>,
    },
    UnsupportedConditional {
        name: String,
        presence_conditions: Vec<Condition>,
    },
}

#[derive(Clone)]
enum SelectorRef {
    // Reference to a previously-decoded sibling segment in the same struct.
    Sibling(String),
}

#[derive(Clone)]
enum SelectorCondition {
    Equals(ConditionValue),
    NotEquals(ConditionValue),
    LessThan(ConditionValue),
    LessThanOrEqual(ConditionValue),
    GreaterThan(ConditionValue),
    GreaterThanOrEqual(ConditionValue),
    Always,
}

impl VarSegment {
    fn name(&self) -> &str {
        match self {
            VarSegment::Scalar { name, .. }
            | VarSegment::VariableString { name, .. }
            | VarSegment::Array { name, .. }
            | VarSegment::NestedStruct { name, .. }
            | VarSegment::Union { name, .. }
            | VarSegment::UnsupportedConditional { name, .. } => name.as_str(),
        }
    }

    fn presence_conditions(&self) -> &[Condition] {
        match self {
            VarSegment::Scalar {
                presence_conditions,
                ..
            }
            | VarSegment::VariableString {
                presence_conditions,
                ..
            }
            | VarSegment::Array {
                presence_conditions,
                ..
            }
            | VarSegment::NestedStruct {
                presence_conditions,
                ..
            }
            | VarSegment::Union {
                presence_conditions,
                ..
            }
            | VarSegment::UnsupportedConditional {
                presence_conditions,
                ..
            } => presence_conditions.as_slice(),
        }
    }
}

#[pyfunction]
fn validate_record_native(
    _py: Python<'_>,
    plugin: &Bound<'_, PyAny>,
    record: &Bound<'_, PyAny>,
) -> PyResult<()> {
    crate::plugin_runtime::authoring::authoring_serialize::validate_record_impl(plugin, record)
}

#[pyfunction]
fn supported_games() -> Vec<&'static str> {
    schema_registry::supported_games()
}

/// The canonical FO76→FO4 custom map-marker icon table:
/// `(fo76_type, fo4_custom_byte, fo76_source_symbol, fo4_export_symbol)`. Single
/// source of truth shared by the schema enum, the SWF marker-injection build, and
/// the F4SE hook — Python derives its marker maps from here rather than
/// duplicating them. The source symbol names the art in FO76's
/// `mapmarkerlibrary.swf`; the export is the (possibly renamed) SymbolClass export
/// to inject under in FO4 (differs only where it would collide with a stock FO4
/// export — see `FO4_MARKER_EXPORT_OVERRIDES`).
#[pyfunction]
fn fo76_custom_marker_icons() -> Vec<(u16, u8, String, String)> {
    plugin_runtime::FO76_CUSTOM_ICONS
        .iter()
        .map(|&(src, fo4, symbol)| {
            let export = plugin_runtime::fo4_marker_export_name(fo4, symbol);
            (src, fo4, symbol.to_string(), export.to_string())
        })
        .collect()
}

#[pyfunction]
fn schema_json_for_game(game: &str) -> PyResult<&'static str> {
    schema_registry::schema_json_for_game(game).ok_or_else(|| {
        let supported = schema_registry::supported_games().join(", ");
        PyValueError::new_err(format!(
            "unsupported game '{game}'. supported games: {supported}"
        ))
    })
}

#[cfg(feature = "dhat-heap")]
#[pyfunction]
fn dhat_heap_start_native(output_path: Option<String>) -> PyResult<bool> {
    let mut guard = DHAT_PROFILER
        .lock()
        .map_err(|_| PyRuntimeError::new_err("DHAT profiler lock poisoned"))?;
    if guard.is_some() {
        return Ok(false);
    }
    let builder = dhat::Profiler::builder();
    *guard = Some(match output_path {
        Some(path) if !path.is_empty() => builder.file_name(path).build(),
        _ => builder.build(),
    });
    Ok(true)
}

#[cfg(not(feature = "dhat-heap"))]
#[pyfunction]
fn dhat_heap_start_native(_output_path: Option<String>) -> PyResult<bool> {
    Err(PyRuntimeError::new_err(
        "esp_authoring_core was not built with the dhat-heap feature",
    ))
}

#[cfg(feature = "dhat-heap")]
#[pyfunction]
fn dhat_heap_stop_native() -> PyResult<bool> {
    let mut guard = DHAT_PROFILER
        .lock()
        .map_err(|_| PyRuntimeError::new_err("DHAT profiler lock poisoned"))?;
    Ok(guard.take().is_some())
}

#[cfg(not(feature = "dhat-heap"))]
#[pyfunction]
fn dhat_heap_stop_native() -> PyResult<bool> {
    Err(PyRuntimeError::new_err(
        "esp_authoring_core was not built with the dhat-heap feature",
    ))
}

#[pyfunction]
fn trim_allocator_native(py: Python<'_>) {
    py.detach(trim_allocator);
}

#[pyfunction]
#[pyo3(signature = (plugin_path, game=None, jobs=None, strings_dir=None, language=None, eager_compressed=true))]
fn load_plugin_native(
    py: Python<'_>,
    plugin_path: &str,
    game: Option<&str>,
    jobs: Option<usize>,
    strings_dir: Option<&str>,
    language: Option<&str>,
    eager_compressed: bool,
) -> PyResult<Py<PyAny>> {
    load_plugin_native_impl(
        py,
        plugin_path,
        game,
        jobs,
        strings_dir,
        language,
        eager_compressed,
    )
}

#[pyfunction]
fn save_plugin_native(
    py: Python<'_>,
    plugin: &Bound<'_, PyAny>,
    output_path: &str,
    game: Option<&str>,
) -> PyResult<()> {
    save_plugin_native_impl(py, plugin, output_path, game)
}

#[pyfunction]
fn plugin_to_bytes_native(py: Python<'_>, plugin: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    plugin_to_bytes_native_impl(py, plugin)
}

#[pyfunction]
fn export_plugin_text_native(
    py: Python<'_>,
    plugin_path: &str,
    output_path: &str,
    game: Option<&str>,
    mode: Option<&str>,
    format: Option<&str>,
) -> PyResult<()> {
    export_plugin_text_native_impl(
        py,
        plugin_path,
        output_path,
        game,
        mode.unwrap_or("lossless"),
        format.unwrap_or("json"),
    )
}

#[pyfunction]
fn import_plugin_text_native(
    py: Python<'_>,
    source_path: &str,
    output_path: &str,
    game: Option<&str>,
    format: Option<&str>,
) -> PyResult<()> {
    import_plugin_text_native_impl(py, source_path, output_path, game, format)
}

#[pyfunction]
#[pyo3(signature = (plugin_path, out_dir, game=None, format=None, jobs=None, skip_signatures=None))]
fn export_authoring_dir_native(
    py: Python<'_>,
    plugin_path: &str,
    out_dir: &str,
    game: Option<&str>,
    format: Option<&str>,
    jobs: Option<usize>,
    skip_signatures: Option<Vec<String>>,
) -> PyResult<()> {
    export_authoring_dir_native_impl(
        py,
        plugin_path,
        out_dir,
        game,
        format.unwrap_or("json"),
        jobs,
        skip_signatures,
    )
}

#[pyfunction]
#[pyo3(signature = (source_dir, output_path, game=None, jobs=None, master_esm_paths=None))]
fn build_authoring_dir_streaming_native(
    py: Python<'_>,
    source_dir: &str,
    output_path: &str,
    game: Option<&str>,
    jobs: Option<usize>,
    master_esm_paths: Option<Vec<String>>,
) -> PyResult<()> {
    let source_dir = source_dir.to_string();
    let output_path = output_path.to_string();
    let game_owned = game.map(str::to_string);
    let masters = master_esm_paths;
    py.detach(move || {
        crate::plugin_runtime::build_authoring_dir_streaming_native(
            source_dir.as_str(),
            output_path.as_str(),
            game_owned.as_deref(),
            jobs,
            masters.as_deref(),
        )
    })
}

#[pyfunction(name = "plugin_handle_collect_cell_slice_roots")]
#[pyo3(signature = (handle_id, worldspace_editor_id, min_x, min_y, max_x, max_y, include_worldspace_persistent_cell, worker_count=None))]
fn plugin_handle_collect_cell_slice_roots_native(
    py: Python<'_>,
    handle_id: u64,
    worldspace_editor_id: &str,
    min_x: i32,
    min_y: i32,
    max_x: i32,
    max_y: i32,
    include_worldspace_persistent_cell: bool,
    worker_count: Option<usize>,
) -> PyResult<Py<PyAny>> {
    let worldspace_editor_id = worldspace_editor_id.to_string();
    let text = py.detach(move || {
        crate::plugin_runtime::plugin_handle_collect_cell_slice_roots_json(
            handle_id,
            worldspace_editor_id.as_str(),
            min_x,
            min_y,
            max_x,
            max_y,
            include_worldspace_persistent_cell,
            worker_count,
        )
    })?;
    let json = PyModule::import(py, "json")?;
    Ok(json.call_method1("loads", (text,))?.into_any().unbind())
}

#[pyfunction(name = "plugin_handle_collect_cell_children")]
#[pyo3(signature = (handle_id, cell_form_id))]
fn plugin_handle_collect_cell_children_native(
    py: Python<'_>,
    handle_id: u64,
    cell_form_id: u32,
) -> PyResult<Py<PyAny>> {
    let text = py.detach(move || {
        crate::plugin_runtime::plugin_handle_collect_cell_children_json(handle_id, cell_form_id)
    })?;
    let json = PyModule::import(py, "json")?;
    Ok(json.call_method1("loads", (text,))?.into_any().unbind())
}

#[pyfunction(name = "plugin_handle_collect_worldspace_terrain_ids")]
#[pyo3(signature = (handle_id, worldspace_editor_id, min_x, min_y, max_x, max_y))]
fn plugin_handle_collect_worldspace_terrain_ids_native(
    py: Python<'_>,
    handle_id: u64,
    worldspace_editor_id: &str,
    min_x: i32,
    min_y: i32,
    max_x: i32,
    max_y: i32,
) -> PyResult<Py<PyAny>> {
    let worldspace_editor_id = worldspace_editor_id.to_string();
    let text = py.detach(move || {
        crate::plugin_runtime::plugin_handle_collect_worldspace_terrain_ids_json(
            handle_id,
            worldspace_editor_id.as_str(),
            min_x,
            min_y,
            max_x,
            max_y,
        )
        .map_err(PyRuntimeError::new_err)
    })?;
    let json = PyModule::import(py, "json")?;
    Ok(json.call_method1("loads", (text,))?.into_any().unbind())
}

#[pyfunction(name = "plugin_handle_insert_cell_slice_children")]
fn plugin_handle_insert_cell_slice_children_native(
    py: Python<'_>,
    target_handle_id: u64,
    children_by_target_cell_json: &str,
) -> PyResult<Py<PyAny>> {
    let payload = children_by_target_cell_json.to_string();
    let text = py.detach(move || {
        crate::plugin_runtime::plugin_handle_insert_cell_slice_children_json(
            target_handle_id,
            payload.as_str(),
        )
    })?;
    let json = PyModule::import(py, "json")?;
    Ok(json.call_method1("loads", (text,))?.into_any().unbind())
}

#[pyfunction(name = "plugin_handle_sync_cell_locations_from_lctn")]
fn plugin_handle_sync_cell_locations_from_lctn_native(
    py: Python<'_>,
    handle_id: u64,
) -> PyResult<Py<PyAny>> {
    let text = py.detach(move || {
        crate::plugin_runtime::plugin_handle_sync_cell_locations_from_lctn_json(handle_id)
    })?;
    let json = PyModule::import(py, "json")?;
    Ok(json.call_method1("loads", (text,))?.into_any().unbind())
}

#[pyfunction(name = "plugin_handle_sync_cell_regions_from_source")]
fn plugin_handle_sync_cell_regions_from_source_native(
    py: Python<'_>,
    source_handle_id: u64,
    target_handle_id: u64,
    source_worldspace_editor_id: &str,
    target_worldspace_editor_id: &str,
) -> PyResult<Py<PyAny>> {
    let source_worldspace_editor_id = source_worldspace_editor_id.to_string();
    let target_worldspace_editor_id = target_worldspace_editor_id.to_string();
    let text = py.detach(move || {
        crate::plugin_runtime::plugin_handle_sync_cell_regions_from_source_json(
            source_handle_id,
            target_handle_id,
            source_worldspace_editor_id.as_str(),
            target_worldspace_editor_id.as_str(),
        )
    })?;
    let json = PyModule::import(py, "json")?;
    Ok(json.call_method1("loads", (text,))?.into_any().unbind())
}

#[pyfunction(name = "plugin_handle_sync_cell_max_height_from_source")]
fn plugin_handle_sync_cell_max_height_from_source_native(
    py: Python<'_>,
    source_handle_id: u64,
    target_handle_id: u64,
    source_worldspace_editor_id: &str,
    target_worldspace_editor_id: &str,
) -> PyResult<Py<PyAny>> {
    let source_worldspace_editor_id = source_worldspace_editor_id.to_string();
    let target_worldspace_editor_id = target_worldspace_editor_id.to_string();
    let text = py.detach(move || {
        crate::plugin_runtime::plugin_handle_sync_cell_max_height_from_source_json(
            source_handle_id,
            target_handle_id,
            source_worldspace_editor_id.as_str(),
            target_worldspace_editor_id.as_str(),
        )
    })?;
    let json = PyModule::import(py, "json")?;
    Ok(json.call_method1("loads", (text,))?.into_any().unbind())
}

#[pyfunction(name = "plugin_handle_carry_worldspace_header_from_source")]
fn plugin_handle_carry_worldspace_header_from_source_native(
    py: Python<'_>,
    source_handle_id: u64,
    target_handle_id: u64,
    source_worldspace_editor_id: &str,
    target_worldspace_editor_id: &str,
) -> PyResult<Py<PyAny>> {
    let source_worldspace_editor_id = source_worldspace_editor_id.to_string();
    let target_worldspace_editor_id = target_worldspace_editor_id.to_string();
    let text = py.detach(move || {
        crate::plugin_runtime::plugin_handle_carry_worldspace_header_from_source_json(
            source_handle_id,
            target_handle_id,
            source_worldspace_editor_id.as_str(),
            target_worldspace_editor_id.as_str(),
        )
    })?;
    let json = PyModule::import(py, "json")?;
    Ok(json.call_method1("loads", (text,))?.into_any().unbind())
}

#[pyfunction(name = "plugin_handle_copy_cell_slice_children")]
#[pyo3(signature = (
    source_handle_id,
    target_handle_id,
    children_by_target_cell_json,
    offset_x,
    offset_y,
    offset_z,
    form_key_map_json = None
))]
fn plugin_handle_copy_cell_slice_children_native(
    py: Python<'_>,
    source_handle_id: u64,
    target_handle_id: u64,
    children_by_target_cell_json: &str,
    offset_x: f32,
    offset_y: f32,
    offset_z: f32,
    form_key_map_json: Option<&str>,
) -> PyResult<Py<PyAny>> {
    let payload = children_by_target_cell_json.to_string();
    let form_key_map = form_key_map_json.map(str::to_string);
    let text = py.detach(move || {
        crate::plugin_runtime::plugin_handle_copy_cell_slice_children_json(
            source_handle_id,
            target_handle_id,
            payload.as_str(),
            offset_x,
            offset_y,
            offset_z,
            form_key_map.as_deref(),
        )
    })?;
    let json = PyModule::import(py, "json")?;
    Ok(json.call_method1("loads", (text,))?.into_any().unbind())
}

#[pyfunction(name = "plugin_handle_synthesize_worldspace_persistent_cell")]
#[pyo3(signature = (
    source_handle_id,
    target_handle_id,
    worldspace_editor_id,
    offset_x,
    offset_y,
    offset_z,
    form_key_map_json = None
))]
fn plugin_handle_synthesize_worldspace_persistent_cell_native(
    py: Python<'_>,
    source_handle_id: u64,
    target_handle_id: u64,
    worldspace_editor_id: &str,
    offset_x: f32,
    offset_y: f32,
    offset_z: f32,
    form_key_map_json: Option<&str>,
) -> PyResult<Py<PyAny>> {
    let worldspace_editor_id = worldspace_editor_id.to_string();
    let form_key_map = form_key_map_json.map(str::to_string);
    let text = py.detach(move || {
        crate::plugin_runtime::plugin_handle_synthesize_worldspace_persistent_cell_json(
            source_handle_id,
            target_handle_id,
            worldspace_editor_id.as_str(),
            offset_x,
            offset_y,
            offset_z,
            form_key_map.as_deref(),
        )
    })?;
    let json = PyModule::import(py, "json")?;
    Ok(json.call_method1("loads", (text,))?.into_any().unbind())
}

#[pyfunction(name = "plugin_handle_collect_worldspace_persistent_base_keys")]
#[pyo3(signature = (source_handle_id, worldspace_editor_id))]
fn plugin_handle_collect_worldspace_persistent_base_keys_native(
    py: Python<'_>,
    source_handle_id: u64,
    worldspace_editor_id: &str,
) -> PyResult<Py<PyAny>> {
    let worldspace_editor_id = worldspace_editor_id.to_string();
    let text = py.detach(move || {
        crate::plugin_runtime::plugin_handle_collect_worldspace_persistent_base_keys_json(
            source_handle_id,
            worldspace_editor_id.as_str(),
        )
    })?;
    let json = PyModule::import(py, "json")?;
    Ok(json.call_method1("loads", (text,))?.into_any().unbind())
}

#[pyfunction(name = "plugin_handle_collect_worldspace_persistent_base_keys_in_bounds")]
#[pyo3(signature = (source_handle_id, worldspace_editor_id, min_x, min_y, max_x, max_y))]
fn plugin_handle_collect_worldspace_persistent_base_keys_in_bounds_native(
    py: Python<'_>,
    source_handle_id: u64,
    worldspace_editor_id: &str,
    min_x: i32,
    min_y: i32,
    max_x: i32,
    max_y: i32,
) -> PyResult<Py<PyAny>> {
    let worldspace_editor_id = worldspace_editor_id.to_string();
    let text = py.detach(move || {
        crate::plugin_runtime::plugin_handle_collect_worldspace_persistent_base_keys_in_bounds_json(
            source_handle_id,
            worldspace_editor_id.as_str(),
            min_x,
            min_y,
            max_x,
            max_y,
        )
    })?;
    let json = PyModule::import(py, "json")?;
    Ok(json.call_method1("loads", (text,))?.into_any().unbind())
}

#[pyfunction(name = "plugin_handle_collect_water_manifest")]
#[pyo3(signature = (handle_id, worldspace_editor_id, min_x, min_y, max_x, max_y))]
fn plugin_handle_collect_water_manifest_native(
    py: Python<'_>,
    handle_id: u64,
    worldspace_editor_id: &str,
    min_x: i32,
    min_y: i32,
    max_x: i32,
    max_y: i32,
) -> PyResult<Py<PyAny>> {
    let worldspace_editor_id = worldspace_editor_id.to_string();
    let text = py.detach(move || {
        crate::plugin_runtime::plugin_handle_collect_water_manifest_json(
            handle_id,
            worldspace_editor_id.as_str(),
            min_x,
            min_y,
            max_x,
            max_y,
        )
    })?;
    let json = PyModule::import(py, "json")?;
    Ok(json.call_method1("loads", (text,))?.into_any().unbind())
}

/// Batch FormKey rewrite over a list of records. Mirrors
/// `FormKeyMapper.rewrite_formkeys` Python semantics on every record in one
/// native pass — no per-record GIL re-acquisition.
#[pyfunction(name = "rewrite_formkeys_batch")]
fn rewrite_formkeys_batch_native(
    py: Python<'_>,
    records: &Bound<'_, PyAny>,
    mappings: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let json = PyModule::import(py, "json")?;
    let records_text: String = json.call_method1("dumps", (records,))?.extract()?;
    let mappings_text: String = json.call_method1("dumps", (mappings,))?.extract()?;
    let rewritten = py.detach(move || {
        crate::formkey_ops::rewrite_formkeys_batch_json(&records_text, &mappings_text)
            .map_err(PyValueError::new_err)
    })?;
    Ok(json
        .call_method1("loads", (rewritten,))?
        .into_any()
        .unbind())
}

/// Batch stale-FormKey scan. Returns a sorted list of unique FK strings whose
/// plugin (case-insensitive) is in `source_plugins`.
#[pyfunction(name = "find_stale_formkeys_batch")]
fn find_stale_formkeys_batch_native(
    py: Python<'_>,
    records: &Bound<'_, PyAny>,
    source_plugins: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let json = PyModule::import(py, "json")?;
    let records_text: String = json.call_method1("dumps", (records,))?.extract()?;
    let plugins_text: String = json.call_method1("dumps", (source_plugins,))?.extract()?;
    let result = py.detach(move || {
        crate::formkey_ops::find_stale_formkeys_batch_json(&records_text, &plugins_text)
            .map_err(PyValueError::new_err)
    })?;
    Ok(json.call_method1("loads", (result,))?.into_any().unbind())
}

/// Batch FormKey replacement with null=remove semantics. Mirrors
/// `ConversionFixups._replace_formkeys` over every record in one pass.
#[pyfunction(name = "replace_formkeys_batch")]
fn replace_formkeys_batch_native(
    py: Python<'_>,
    records: &Bound<'_, PyAny>,
    replacements: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let json = PyModule::import(py, "json")?;
    let records_text: String = json.call_method1("dumps", (records,))?.extract()?;
    let replacements_text: String = json.call_method1("dumps", (replacements,))?.extract()?;
    let rewritten = py.detach(move || {
        crate::formkey_ops::replace_formkeys_batch_json(&records_text, &replacements_text)
            .map_err(PyValueError::new_err)
    })?;
    Ok(json
        .call_method1("loads", (rewritten,))?
        .into_any()
        .unbind())
}

// ---------------------------------------------------------------------------
// TranslatedStore handle pyfunctions
// ---------------------------------------------------------------------------

#[pyfunction(name = "translated_store_create")]
fn translated_store_create_native(
    py: Python<'_>,
    records: &Bound<'_, PyAny>,
    warnings: &Bound<'_, PyAny>,
) -> PyResult<u64> {
    let json = PyModule::import(py, "json")?;
    let records_text: String = json.call_method1("dumps", (records,))?.extract()?;
    let warnings_text: String = json.call_method1("dumps", (warnings,))?.extract()?;
    py.detach(move || {
        crate::translated_store::create_from_json(&records_text, &warnings_text)
            .map_err(PyValueError::new_err)
    })
}

#[pyfunction(name = "translated_store_create_empty")]
fn translated_store_create_empty_native() -> u64 {
    crate::translated_store::create_empty()
}

#[pyfunction(name = "translated_store_free")]
fn translated_store_free_native(handle: u64) -> PyResult<()> {
    crate::translated_store::free(handle).map_err(PyValueError::new_err)
}

#[pyfunction(name = "translated_store_len")]
fn translated_store_len_native(handle: u64) -> PyResult<usize> {
    crate::translated_store::len(handle).map_err(PyValueError::new_err)
}

#[pyfunction(name = "translated_store_get_record")]
fn translated_store_get_record_native(
    py: Python<'_>,
    handle: u64,
    index: usize,
) -> PyResult<Py<PyAny>> {
    let json = PyModule::import(py, "json")?;
    let text = py.detach(move || {
        crate::translated_store::get_record_json(handle, index).map_err(PyValueError::new_err)
    })?;
    Ok(json.call_method1("loads", (text,))?.into_any().unbind())
}

#[pyfunction(name = "translated_store_get_warnings")]
fn translated_store_get_warnings_native(handle: u64, index: usize) -> PyResult<Vec<String>> {
    crate::translated_store::get_warnings(handle, index).map_err(PyValueError::new_err)
}

#[pyfunction(name = "translated_store_set_record")]
fn translated_store_set_record_native(
    py: Python<'_>,
    handle: u64,
    index: usize,
    record: &Bound<'_, PyAny>,
) -> PyResult<()> {
    let json = PyModule::import(py, "json")?;
    let text: String = json.call_method1("dumps", (record,))?.extract()?;
    py.detach(move || {
        crate::translated_store::set_record_json(handle, index, &text)
            .map_err(PyValueError::new_err)
    })
}

#[pyfunction(name = "translated_store_set_warnings")]
fn translated_store_set_warnings_native(
    handle: u64,
    index: usize,
    warnings: Vec<String>,
) -> PyResult<()> {
    crate::translated_store::set_warnings(handle, index, warnings).map_err(PyValueError::new_err)
}

#[pyfunction(name = "translated_store_set_pair")]
fn translated_store_set_pair_native(
    py: Python<'_>,
    handle: u64,
    index: usize,
    record: &Bound<'_, PyAny>,
    warnings: Vec<String>,
) -> PyResult<()> {
    let json = PyModule::import(py, "json")?;
    let text: String = json.call_method1("dumps", (record,))?.extract()?;
    py.detach(move || {
        crate::translated_store::set_pair_json(handle, index, &text, warnings)
            .map_err(PyValueError::new_err)
    })
}

#[pyfunction(name = "translated_store_append")]
fn translated_store_append_native(
    py: Python<'_>,
    handle: u64,
    record: &Bound<'_, PyAny>,
    warnings: Vec<String>,
) -> PyResult<()> {
    let json = PyModule::import(py, "json")?;
    let text: String = json.call_method1("dumps", (record,))?.extract()?;
    py.detach(move || {
        crate::translated_store::append_json(handle, &text, warnings).map_err(PyValueError::new_err)
    })
}

#[pyfunction(name = "translated_store_fetch_chunk")]
fn translated_store_fetch_chunk_native(
    py: Python<'_>,
    handle: u64,
    start: usize,
    end: usize,
) -> PyResult<Py<PyAny>> {
    let json = PyModule::import(py, "json")?;
    let text = py.detach(move || {
        crate::translated_store::fetch_chunk_json(handle, start, end).map_err(PyValueError::new_err)
    })?;
    Ok(json.call_method1("loads", (text,))?.into_any().unbind())
}

#[pyfunction(name = "translated_store_prune_indices")]
fn translated_store_prune_indices_native(handle: u64, indices: Vec<usize>) -> PyResult<usize> {
    crate::translated_store::prune_indices(handle, indices).map_err(PyValueError::new_err)
}

#[pyfunction(name = "translated_store_rewrite_formkeys")]
fn translated_store_rewrite_formkeys_native(
    py: Python<'_>,
    handle: u64,
    mappings: &Bound<'_, PyAny>,
) -> PyResult<usize> {
    let json = PyModule::import(py, "json")?;
    let text: String = json.call_method1("dumps", (mappings,))?.extract()?;
    py.detach(move || {
        crate::translated_store::rewrite_formkeys_inplace(handle, &text)
            .map_err(PyValueError::new_err)
    })
}

#[pyfunction(name = "translated_store_replace_formkeys")]
fn translated_store_replace_formkeys_native(
    py: Python<'_>,
    handle: u64,
    replacements: &Bound<'_, PyAny>,
) -> PyResult<usize> {
    let json = PyModule::import(py, "json")?;
    let text: String = json.call_method1("dumps", (replacements,))?.extract()?;
    py.detach(move || {
        crate::translated_store::replace_formkeys_inplace(handle, &text)
            .map_err(PyValueError::new_err)
    })
}

#[pyfunction(name = "translated_store_find_stale_formkeys")]
fn translated_store_find_stale_formkeys_native(
    py: Python<'_>,
    handle: u64,
    source_plugins: Vec<String>,
) -> PyResult<Py<PyAny>> {
    let json = PyModule::import(py, "json")?;
    let plugins_text: String = json.call_method1("dumps", (source_plugins,))?.extract()?;
    let result = py.detach(move || {
        crate::translated_store::find_stale_formkeys(handle, &plugins_text)
            .map_err(PyValueError::new_err)
    })?;
    Ok(json.call_method1("loads", (result,))?.into_any().unbind())
}

#[pyfunction(name = "translated_store_indices_by_signature")]
fn translated_store_indices_by_signature_native(
    py: Python<'_>,
    handle: u64,
    signatures: Vec<String>,
) -> PyResult<Vec<usize>> {
    let json = PyModule::import(py, "json")?;
    let sigs_text: String = json.call_method1("dumps", (signatures,))?.extract()?;
    py.detach(move || {
        crate::translated_store::indices_by_signature(handle, &sigs_text)
            .map_err(PyValueError::new_err)
    })
}

#[pyfunction(name = "translated_store_take_record")]
fn translated_store_take_record_native(
    py: Python<'_>,
    handle: u64,
    index: usize,
) -> PyResult<Py<PyAny>> {
    let json = PyModule::import(py, "json")?;
    let text = py.detach(move || {
        crate::translated_store::take_record_json(handle, index).map_err(PyValueError::new_err)
    })?;
    Ok(json.call_method1("loads", (text,))?.into_any().unbind())
}

#[pyfunction]
fn known_formid_subrecords_native() -> Vec<&'static str> {
    crate::plugin_runtime::codec_constants::KNOWN_FORMID_SUBRECORDS.to_vec()
}

#[pyfunction]
fn known_formid_array_subrecords_native() -> Vec<&'static str> {
    crate::plugin_runtime::codec_constants::KNOWN_FORMID_ARRAY_SUBRECORDS.to_vec()
}

#[pyfunction]
fn localized_string_subrecords_native() -> Vec<&'static str> {
    crate::plugin_runtime::codec_constants::LOCALIZED_STRING_SUBRECORDS.to_vec()
}

#[pyfunction]
fn textual_subrecords_native() -> Vec<&'static str> {
    crate::plugin_runtime::codec_constants::TEXTUAL_SUBRECORDS.to_vec()
}

/// Parse an NVNM subrecord payload and return on success. Raises ValueError on
/// any parse error, including trailing residue past the structured tail. Used
/// by Python harness tests to gate that emitted NVNM is byte-exact (no
/// untracked extra bytes after the navmesh grid).
#[pyfunction]
fn parse_nvnm(data: &[u8]) -> PyResult<()> {
    crate::nvnm::parse_nvnm(data)
        .map(|_| ())
        .map_err(|err| PyValueError::new_err(err.to_string()))
}

fn nvnm_validator_form_key(raw: u32, masters: &[String], plugin_name: &str) -> String {
    if raw == 0 {
        return "00000000".to_string();
    }
    let index = ((raw >> 24) & 0xFF) as usize;
    let object_id = raw & 0x00FF_FFFF;
    if index == 0xFF {
        return format!("{raw:08X}");
    }
    if index < masters.len() {
        return format!("{object_id:06X}:{}", masters[index]);
    }
    if index == masters.len() {
        return format!("{object_id:06X}:{plugin_name}");
    }
    format!("{raw:08X}")
}

fn collect_nvnm_payloads_for_validation(
    items: &[crate::plugin_runtime::ParsedItem],
    masters: &[String],
    plugin_name: &str,
    out: &mut Vec<(String, crate::nvnm::NvnmPayload)>,
    parse_failures: &mut Vec<(String, String)>,
) {
    use crate::plugin_runtime::ParsedItem;
    for item in items {
        match item {
            ParsedItem::Record(record) if record.signature.as_str() == "NAVM" => {
                let Some(nvnm) = record
                    .subrecords
                    .iter()
                    .find(|sr| sr.signature.as_str() == "NVNM")
                else {
                    continue;
                };
                if nvnm.data.is_empty() {
                    continue;
                }
                let form_key = nvnm_validator_form_key(record.form_id, masters, plugin_name);
                match crate::nvnm::parse_nvnm(nvnm.data.as_ref()) {
                    Ok(payload) => out.push((form_key, payload)),
                    Err(e) => parse_failures.push((form_key, e.to_string())),
                }
            }
            ParsedItem::Group(group) => {
                collect_nvnm_payloads_for_validation(
                    &group.children,
                    masters,
                    plugin_name,
                    out,
                    parse_failures,
                );
            }
            _ => {}
        }
    }
}

/// Load a plugin and run the NVNM structural validator across every NAVM in
/// the file. The validator is the automated CK PATHFINDING-warnings gate —
/// when this returns `ok=True`, the regen output is structurally clean.
///
/// Returns `(ok, errors)`, where each error is
/// `(mesh_form_key, kind, detail)`.
///
/// Raises ValueError if the plugin can't be loaded or any NVNM fails to parse
/// (parse failures are a precondition for validation, not validator findings).
#[pyfunction]
#[pyo3(signature = (plugin_path, game = None))]
fn nvnm_validate_plugin_navmeshes(
    py: Python<'_>,
    plugin_path: &str,
    game: Option<&str>,
) -> PyResult<(bool, Vec<(String, String, String)>)> {
    let path = plugin_path.to_string();
    let game_owned = game.map(str::to_string);
    let (meshes, parse_failures) = py.detach(move || -> PyResult<_> {
        let parsed =
            crate::plugin_runtime::parse_plugin_file_eager_compressed(path.as_str(), game_owned)?;
        let masters = parsed.header.masters.clone();
        let plugin_name = parsed.plugin_name.clone();
        let mut meshes: Vec<(String, crate::nvnm::NvnmPayload)> = Vec::new();
        let mut parse_failures: Vec<(String, String)> = Vec::new();
        collect_nvnm_payloads_for_validation(
            &parsed.root_items,
            &masters,
            &plugin_name,
            &mut meshes,
            &mut parse_failures,
        );
        Ok((meshes, parse_failures))
    })?;
    if !parse_failures.is_empty() {
        let preview = parse_failures
            .iter()
            .take(5)
            .map(|(k, e)| format!("{k}: {e}"))
            .collect::<Vec<_>>()
            .join("; ");
        return Err(PyValueError::new_err(format!(
            "NVNM parse failed for {} record(s) before validation could run (first 5: {})",
            parse_failures.len(),
            preview
        )));
    }

    let report = crate::nvnm::validate_navmesh_set(&meshes);
    let ok = report.is_ok();
    let errors = report
        .errors
        .into_iter()
        .map(|err| (err.mesh_form_key, err.kind.as_str().to_string(), err.detail))
        .collect();
    Ok((ok, errors))
}

pub fn register_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(validate_record_native, m)?)?;
    m.add_function(wrap_pyfunction!(supported_games, m)?)?;
    m.add_function(wrap_pyfunction!(fo76_custom_marker_icons, m)?)?;
    m.add_function(wrap_pyfunction!(schema_json_for_game, m)?)?;
    m.add_function(wrap_pyfunction!(dhat_heap_start_native, m)?)?;
    m.add_function(wrap_pyfunction!(dhat_heap_stop_native, m)?)?;
    m.add_function(wrap_pyfunction!(trim_allocator_native, m)?)?;
    m.add_function(wrap_pyfunction!(previs_merge::merge_previs_native, m)?)?;
    m.add_function(wrap_pyfunction!(known_formid_subrecords_native, m)?)?;
    m.add_function(wrap_pyfunction!(known_formid_array_subrecords_native, m)?)?;
    m.add_function(wrap_pyfunction!(localized_string_subrecords_native, m)?)?;
    m.add_function(wrap_pyfunction!(textual_subrecords_native, m)?)?;
    m.add_function(wrap_pyfunction!(parse_nvnm, m)?)?;
    m.add_function(wrap_pyfunction!(nvnm_validate_plugin_navmeshes, m)?)?;
    m.add_function(wrap_pyfunction!(
        plugin_handle_collect_cell_slice_roots_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        plugin_handle_collect_cell_children_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        plugin_handle_collect_worldspace_terrain_ids_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        plugin_handle_insert_cell_slice_children_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        plugin_handle_sync_cell_locations_from_lctn_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        plugin_handle_sync_cell_regions_from_source_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        plugin_handle_sync_cell_max_height_from_source_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        plugin_handle_carry_worldspace_header_from_source_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        plugin_handle_copy_cell_slice_children_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        plugin_handle_synthesize_worldspace_persistent_cell_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        plugin_handle_collect_worldspace_persistent_base_keys_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        plugin_handle_collect_worldspace_persistent_base_keys_in_bounds_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        plugin_handle_collect_water_manifest_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(rewrite_formkeys_batch_native, m)?)?;
    m.add_function(wrap_pyfunction!(find_stale_formkeys_batch_native, m)?)?;
    m.add_function(wrap_pyfunction!(replace_formkeys_batch_native, m)?)?;
    m.add_function(wrap_pyfunction!(translated_store_create_native, m)?)?;
    m.add_function(wrap_pyfunction!(translated_store_create_empty_native, m)?)?;
    m.add_function(wrap_pyfunction!(translated_store_free_native, m)?)?;
    m.add_function(wrap_pyfunction!(translated_store_len_native, m)?)?;
    m.add_function(wrap_pyfunction!(translated_store_get_record_native, m)?)?;
    m.add_function(wrap_pyfunction!(translated_store_get_warnings_native, m)?)?;
    m.add_function(wrap_pyfunction!(translated_store_set_record_native, m)?)?;
    m.add_function(wrap_pyfunction!(translated_store_set_warnings_native, m)?)?;
    m.add_function(wrap_pyfunction!(translated_store_set_pair_native, m)?)?;
    m.add_function(wrap_pyfunction!(translated_store_append_native, m)?)?;
    m.add_function(wrap_pyfunction!(translated_store_fetch_chunk_native, m)?)?;
    m.add_function(wrap_pyfunction!(translated_store_prune_indices_native, m)?)?;
    m.add_function(wrap_pyfunction!(
        translated_store_rewrite_formkeys_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        translated_store_replace_formkeys_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        translated_store_find_stale_formkeys_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        translated_store_indices_by_signature_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(translated_store_take_record_native, m)?)?;
    m.add_function(wrap_pyfunction!(load_plugin_native, m)?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_load_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_load_index_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_from_bytes_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_new_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_close_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_metadata_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_get_meta_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_get_strings_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::schema_forge_collect_corpus_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::conflicts::scan_conflicts_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::validate::validate_plugin_deep_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_group_signatures_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_group_record_summaries_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_to_bytes_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_save_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_max_object_id_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_allocate_form_id_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_add_master_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_ensure_source_masters_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_set_header_field_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_set_masters_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_set_logical_identity_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_remove_record_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_remove_records_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_delete_records_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_add_record_raw_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_replace_authoring_record_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_replace_projected_cell_authoring_record_values_at_locations_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_read_authoring_record_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_record_form_ids_with_subrecords_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_record_subrecords_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_set_record_subrecords_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_remove_formid_subrecords_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_repair_term_marker_parameters_from_source_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_record_flags_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_set_record_flags_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_apply_placed_record_position_offset_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_sanitize_subrecord_payloads_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_record_summary_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_has_record_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_record_payload_hash_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_debug_section_loaded_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_force_build_records_section_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_force_build_refs_section_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_assets_by_kind_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_record_eid_index_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_record_index_rows_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_local_object_ids_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_owned_object_ids_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_record_form_ids_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_validation_records_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_search_records_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_used_master_indices_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_apply_object_id_mapping_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_null_refs_to_master_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_copy_record_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_merge_conflict_to_patch_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_undelete_and_disable_refs_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::voice_reference::voice_reference_build_index_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::voice_reference::voice_reference_read_index_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_resolve_string_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_resolve_string_values_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_set_localized_strings_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_set_localized_strings_by_language_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_set_localized_field_values_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_allocate_localized_string_id_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_save_localized_strings_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_record_context_for_form_id_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_addon_node_summaries_by_index_id_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_get_referenced_form_ids_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_get_referenced_form_keys_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_get_referenced_form_keys_by_subrecord_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_get_referencing_form_ids_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_get_referencing_form_keys_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_index_stats_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_collect_assets_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_walk_dependencies_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_get_form_id_chain_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_export_plugin_text_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_export_record_text_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_extract_dialogue_text_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_export_authoring_dir_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::plugin_runtime::plugin_handle_import_text_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(save_plugin_native, m)?)?;
    m.add_function(wrap_pyfunction!(plugin_to_bytes_native, m)?)?;
    m.add_function(wrap_pyfunction!(export_plugin_text_native, m)?)?;
    m.add_function(wrap_pyfunction!(import_plugin_text_native, m)?)?;
    m.add_function(wrap_pyfunction!(export_authoring_dir_native, m)?)?;
    m.add_function(wrap_pyfunction!(build_authoring_dir_streaming_native, m)?)?;
    m.add_function(wrap_pyfunction!(validate_authoring_native, m)?)?;
    m.add_function(wrap_pyfunction!(
        crate::strings_py::parse_string_table_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::strings_py::write_string_table_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::strings_py::load_string_tables_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        crate::strings_py::load_all_string_tables_native,
        m
    )?)?;
    Ok(())
}

#[pymodule]
fn esp_authoring_core(_py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    register_module(m)
}
