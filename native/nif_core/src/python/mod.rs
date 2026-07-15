pub mod value;

use std::collections::HashMap;
use std::path::PathBuf;

use pyo3::exceptions::{PyIOError, PyKeyError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyList};

use crate::convert_file::{ConvertFileOptions, ConvertFileReport};
use crate::model::{NifBlock, NifFile, NifHeader};
use crate::schema::SCHEMA;
use crate::weapon_attachment::extract_attachment as native_extract_attachment;
use crate::weapon_diff::weapon_block_diff as native_weapon_block_diff;

use self::value::{nif_to_py, py_to_nif};

pub fn register_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(nif_from_bytes, m)?)?;
    m.add_function(wrap_pyfunction!(nif_to_bytes, m)?)?;
    m.add_function(wrap_pyfunction!(load_nif, m)?)?;
    m.add_function(wrap_pyfunction!(save_nif, m)?)?;
    m.add_function(wrap_pyfunction!(new_nif, m)?)?;
    m.add_function(wrap_pyfunction!(convert_nif_file, m)?)?;
    m.add_function(wrap_pyfunction!(weapon_block_diff, m)?)?;
    m.add_function(wrap_pyfunction!(extract_attachment, m)?)?;
    m.add_function(wrap_pyfunction!(cloth_extract_blob, m)?)?;
    m.add_function(wrap_pyfunction!(cloth_extract_blobs, m)?)?;
    m.add_function(wrap_pyfunction!(cloth_pack_blob, m)?)?;
    m.add_function(wrap_pyfunction!(cloth_template_apply, m)?)?;
    m.add_function(wrap_pyfunction!(is_subtype_of, m)?)?;
    m.add_function(wrap_pyfunction!(get_type_hierarchy, m)?)?;
    m.add_function(wrap_pyfunction!(get_all_fields, m)?)?;
    m.add_function(wrap_pyfunction!(schema_metadata, m)?)?;
    Ok(())
}

#[pymodule]
fn nif_core_native(_py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    register_module(m)
}

#[pyfunction]
fn nif_from_bytes(py: Python<'_>, data: &Bound<'_, PyBytes>) -> PyResult<Py<PyAny>> {
    let bytes = data.as_bytes().to_vec();
    let nif = py
        .detach(|| NifFile::from_bytes(&bytes, None))
        .map_err(|e| PyIOError::new_err(e.to_string()))?;
    nif_to_payload(py, &nif)
}

#[pyfunction]
fn nif_to_bytes(py: Python<'_>, payload: &Bound<'_, PyDict>) -> PyResult<Py<PyBytes>> {
    let mut nif = payload_to_nif(payload)?;
    let bytes = py
        .detach(|| nif.to_bytes())
        .map_err(|e| PyIOError::new_err(e.to_string()))?;
    Ok(PyBytes::new(py, &bytes).unbind())
}

#[pyfunction]
fn load_nif(py: Python<'_>, path: &str) -> PyResult<Py<PyAny>> {
    let nif = py
        .detach(|| NifFile::load(path))
        .map_err(|e| PyIOError::new_err(e.to_string()))?;
    nif_to_payload(py, &nif)
}

#[pyfunction]
fn save_nif(py: Python<'_>, payload: &Bound<'_, PyDict>, path: &str) -> PyResult<()> {
    let mut nif = payload_to_nif(payload)?;
    py.detach(|| nif.save(Some(PathBuf::from(path))))
        .map_err(|e| PyIOError::new_err(e.to_string()))
}

#[pyfunction]
fn new_nif(py: Python<'_>, game: &str) -> PyResult<Py<PyAny>> {
    let nif = NifFile::new(game);
    nif_to_payload(py, &nif)
}

#[pyfunction(signature = (src, dst, source_game, target_game, bgsm_output_dir=None, options=None))]
fn convert_nif_file(
    py: Python<'_>,
    src: String,
    dst: String,
    source_game: String,
    target_game: String,
    bgsm_output_dir: Option<String>,
    options: Option<&Bound<'_, PyDict>>,
) -> PyResult<Py<PyAny>> {
    let options = convert_file_options_from_py(options)?;
    let src = PathBuf::from(src);
    let dst = PathBuf::from(dst);
    let bgsm_output_dir = bgsm_output_dir.map(PathBuf::from);
    let report = py
        .detach(move || {
            crate::convert_file::convert_nif_file(
                &src,
                &dst,
                &source_game,
                &target_game,
                bgsm_output_dir.as_deref(),
                &options,
            )
        })
        .map_err(|e| PyIOError::new_err(e.to_string()))?;
    convert_file_report_to_py(py, &report)
}

#[pyfunction]
fn weapon_block_diff(py: Python<'_>, base_path: &str, mod_path: &str) -> PyResult<Vec<i32>> {
    let base = py
        .detach(|| NifFile::load(base_path))
        .map_err(|e| PyIOError::new_err(format!("base load failed: {e}")))?;
    let mod_nif = py
        .detach(|| NifFile::load(mod_path))
        .map_err(|e| PyIOError::new_err(format!("mod load failed: {e}")))?;
    Ok(native_weapon_block_diff(&base, &mod_nif))
}

#[pyfunction]
fn extract_attachment(
    py: Python<'_>,
    base_path: String,
    sibling_path: String,
    slot: u8,
    output_attachment_path: String,
    anchor_node_name: String,
) -> PyResult<Py<PyAny>> {
    let report = py
        .detach(move || {
            native_extract_attachment(
                std::path::Path::new(&base_path),
                std::path::Path::new(&sibling_path),
                slot,
                std::path::Path::new(&output_attachment_path),
                &anchor_node_name,
            )
        })
        .map_err(|e| PyIOError::new_err(e.to_string()))?;
    let d = PyDict::new(py);
    d.set_item("blocks_copied", report.blocks_copied)?;
    d.set_item("changes", &report.changes)?;
    d.set_item("warnings", &report.warnings)?;
    Ok(d.into_any().unbind())
}

#[pyfunction]
fn cloth_extract_blob<'py>(
    py: Python<'py>,
    nif_bytes: &Bound<'_, PyBytes>,
) -> PyResult<Bound<'py, PyBytes>> {
    let bytes = nif_bytes.as_bytes().to_vec();
    let result = py
        .detach(move || crate::cloth::extract_cloth_blob(&bytes))
        .map_err(|e| PyValueError::new_err(e.to_string()))?;
    Ok(PyBytes::new(py, &result))
}

#[pyfunction]
fn cloth_extract_blobs<'py>(
    py: Python<'py>,
    nif_bytes: &Bound<'_, PyBytes>,
) -> PyResult<Vec<Bound<'py, PyBytes>>> {
    let bytes = nif_bytes.as_bytes().to_vec();
    let blobs = py
        .detach(move || crate::cloth::extract_cloth_blobs(&bytes))
        .map_err(|e| PyValueError::new_err(e.to_string()))?;
    Ok(blobs.iter().map(|blob| PyBytes::new(py, blob)).collect())
}

#[pyfunction]
fn cloth_pack_blob<'py>(
    py: Python<'py>,
    nif_bytes: &Bound<'_, PyBytes>,
    blob_bytes: &Bound<'_, PyBytes>,
) -> PyResult<Bound<'py, PyBytes>> {
    let nif = nif_bytes.as_bytes().to_vec();
    let blob = blob_bytes.as_bytes().to_vec();
    let result = py
        .detach(move || crate::cloth::pack_cloth_blob(&nif, &blob))
        .map_err(|e| PyValueError::new_err(e.to_string()))?;
    Ok(PyBytes::new(py, &result))
}

#[pyfunction]
fn cloth_template_apply<'py>(
    py: Python<'py>,
    name: String,
    source_nif_bytes: &Bound<'_, PyBytes>,
    args_json: String,
) -> PyResult<Bound<'py, PyBytes>> {
    let bytes = source_nif_bytes.as_bytes().to_vec();
    let result = py
        .detach(move || crate::cloth::apply_cloth_template(&name, &bytes, &args_json))
        .map_err(|e| PyValueError::new_err(e.to_string()))?;
    Ok(PyBytes::new(py, &result))
}

#[pyfunction]
fn is_subtype_of(type_name: &str, base: &str) -> bool {
    SCHEMA.is_subtype_of(type_name, base)
}

#[pyfunction]
fn get_type_hierarchy(type_name: &str) -> Vec<String> {
    SCHEMA.get_type_hierarchy(type_name)
}

#[pyfunction]
fn get_all_fields(py: Python<'_>, type_name: &str) -> PyResult<Py<PyAny>> {
    let fields = SCHEMA.get_all_fields(type_name);
    let list = PyList::empty(py);
    for fdef in fields {
        let d = field_def_to_dict(py, fdef)?;
        list.append(d)?;
    }
    Ok(list.into_any().unbind())
}

#[pyfunction]
fn schema_metadata(py: Python<'_>) -> PyResult<Py<PyAny>> {
    let d = PyDict::new(py);
    d.set_item("api", "dict-payload")?;
    d.set_item("native_api", "functions")?;
    d.set_item("payload_version", 1)?;
    Ok(d.into_any().unbind())
}

fn convert_file_report_to_py(py: Python<'_>, report: &ConvertFileReport) -> PyResult<Py<PyAny>> {
    let d = PyDict::new(py);
    d.set_item("supported", report.supported)?;
    d.set_item("changes", &report.changes)?;
    d.set_item("warnings", &report.warnings)?;
    d.set_item("errors", &report.errors)?;
    d.set_item("emitted_bgsms", &report.emitted_bgsms)?;
    d.set_item("emitted_first_person", &report.emitted_first_person)?;
    d.set_item("shapes_skinned", report.shapes_skinned)?;
    d.set_item("vertices_repacked", report.vertices_repacked)?;
    d.set_item("bones_remapped", report.bones_remapped)?;
    d.set_item("bones_dropped_unmapped", report.bones_dropped_unmapped)?;
    d.set_item("weights_redistributed", report.weights_redistributed)?;
    d.set_item("vertices_morph_weighted", report.vertices_morph_weighted)?;
    d.set_item("timings_ms", &report.timings_ms)?;
    Ok(d.into_any().unbind())
}

fn convert_file_options_from_py(
    options: Option<&Bound<'_, PyDict>>,
) -> PyResult<ConvertFileOptions> {
    let Some(options) = options else {
        return Ok(ConvertFileOptions::default());
    };
    let mut out = ConvertFileOptions::default();
    out.asset_prefix = optional_string_item(options, "asset_prefix")?;
    out.material_namespace = optional_string_item(options, "material_namespace")?;
    out.addon_index_map = optional_i64_map_item(options, "addon_index_map")?;
    out.translation_maps_dir = optional_path_item(options, "translation_maps_dir")?;
    out.auto_skin_reference_body = optional_path_item(options, "auto_skin_reference_body")?;
    out.emit_first_person = optional_bool_item(options, "emit_first_person")?.unwrap_or(false);
    out.first_person_reference = optional_path_item(options, "first_person_reference")?;
    out.morph_weight_cap = optional_f32_item(options, "morph_weight_cap")?.unwrap_or(0.5);
    out.weapon_role = optional_string_item(options, "weapon_role")?;
    Ok(out)
}

fn optional_i64_map_item(d: &Bound<'_, PyDict>, key: &str) -> PyResult<HashMap<i64, i64>> {
    let Some(value) = d.get_item(key)? else {
        return Ok(HashMap::new());
    };
    if value.is_none() {
        return Ok(HashMap::new());
    }
    let dict = value.cast::<PyDict>()?;
    let mut out = HashMap::new();
    for (k, v) in dict.iter() {
        out.insert(k.extract::<i64>()?, v.extract::<i64>()?);
    }
    Ok(out)
}

fn optional_string_item(d: &Bound<'_, PyDict>, key: &str) -> PyResult<Option<String>> {
    let Some(value) = d.get_item(key)? else {
        return Ok(None);
    };
    if value.is_none() {
        return Ok(None);
    }
    Ok(Some(value.extract::<String>()?))
}

fn optional_path_item(d: &Bound<'_, PyDict>, key: &str) -> PyResult<Option<PathBuf>> {
    Ok(optional_string_item(d, key)?.map(PathBuf::from))
}

fn optional_bool_item(d: &Bound<'_, PyDict>, key: &str) -> PyResult<Option<bool>> {
    let Some(value) = d.get_item(key)? else {
        return Ok(None);
    };
    if value.is_none() {
        return Ok(None);
    }
    Ok(Some(value.extract::<bool>()?))
}

fn optional_f32_item(d: &Bound<'_, PyDict>, key: &str) -> PyResult<Option<f32>> {
    let Some(value) = d.get_item(key)? else {
        return Ok(None);
    };
    if value.is_none() {
        return Ok(None);
    }
    Ok(Some(value.extract::<f32>()?))
}

fn nif_to_payload(py: Python<'_>, nif: &NifFile) -> PyResult<Py<PyAny>> {
    let d = PyDict::new(py);
    d.set_item("header", header_to_dict(py, &nif.header)?)?;

    let blocks = PyList::empty(py);
    for block in nif.blocks.iter() {
        blocks.append(block_to_dict(py, block)?)?;
    }
    d.set_item("blocks", blocks)?;

    if let Some(path) = &nif.path {
        d.set_item("path", path.to_string_lossy().as_ref())?;
    }
    Ok(d.into_any().unbind())
}

fn header_to_dict<'py>(py: Python<'py>, h: &NifHeader) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    d.set_item("header_string", &h.header_string)?;
    d.set_item("version", h.version)?;
    d.set_item("version_packed", h.version_packed)?;
    d.set_item("endian_type", h.endian_type)?;
    d.set_item("user_version", h.user_version)?;
    d.set_item("bs_version", h.bs_version)?;
    d.set_item("num_blocks", h.num_blocks)?;
    d.set_item("creator", &h.creator)?;
    d.set_item("export_info", &h.export_info)?;
    d.set_item("sf_export_data", PyBytes::new(py, &h.sf_export_data))?;
    d.set_item("block_type_names", &h.block_type_names)?;
    d.set_item("block_type_index", &h.block_type_index)?;
    d.set_item("block_sizes", &h.block_sizes)?;
    d.set_item("strings", &h.strings)?;
    d.set_item("max_string_length", h.max_string_length)?;
    d.set_item("num_groups", h.num_groups)?;
    d.set_item("groups", &h.groups)?;
    d.set_item("footer_roots", &h.footer_roots)?;
    Ok(d)
}

fn block_to_dict<'py>(py: Python<'py>, block: &NifBlock) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    d.set_item("block_id", block.block_id)?;
    d.set_item("type_name", &block.type_name)?;

    let fields = PyDict::new(py);
    for (name, value) in block.fields.iter() {
        fields.set_item(name, nif_to_py(py, value))?;
    }
    d.set_item("fields", fields)?;
    d.set_item("remainder", PyBytes::new(py, &block.remainder))?;
    Ok(d)
}

fn field_def_to_dict<'py>(
    py: Python<'py>,
    fdef: &crate::schema::FieldDef,
) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    d.set_item("name", fdef.name)?;
    d.set_item("type_name", fdef.type_name)?;
    d.set_item("type", fdef.type_name)?;
    set_optional_str(&d, "template", fdef.template)?;
    set_optional_str(&d, "suffix", fdef.suffix)?;
    set_optional_str(&d, "default", fdef.default)?;
    set_optional_str(&d, "length", fdef.length)?;
    set_optional_str(&d, "width", fdef.width)?;
    set_optional_str(&d, "cond", fdef.cond)?;
    set_optional_str(&d, "vercond", fdef.vercond)?;
    set_optional_str(&d, "since", fdef.since)?;
    set_optional_str(&d, "until", fdef.until)?;
    set_optional_str(&d, "arg", fdef.arg)?;
    set_optional_str(&d, "calc", fdef.calc)?;
    set_optional_str(&d, "only_t", fdef.only_t)?;
    set_optional_str(&d, "exclude_t", fdef.exclude_t)?;
    d.set_item("is_abstract", fdef.is_abstract)?;
    d.set_item("is_binary", fdef.is_binary)?;
    d.set_item("recursive", fdef.recursive)?;
    Ok(d)
}

fn set_optional_str(d: &Bound<'_, PyDict>, key: &str, value: Option<&str>) -> PyResult<()> {
    if let Some(value) = value {
        d.set_item(key, value)?;
    }
    Ok(())
}

fn payload_to_nif(payload: &Bound<'_, PyDict>) -> PyResult<NifFile> {
    let mut nif = NifFile::default();

    let header_obj = required_item(payload, "header")?;
    let header = header_obj
        .cast::<PyDict>()
        .map_err(|_| PyTypeError::new_err("payload['header'] must be a dict"))?;
    nif.header = dict_to_header(header)?;

    let blocks_obj = required_item(payload, "blocks")?;
    let blocks = blocks_obj
        .cast::<PyList>()
        .map_err(|_| PyTypeError::new_err("payload['blocks'] must be a list"))?;
    for item in blocks.iter() {
        let block_dict = item
            .cast::<PyDict>()
            .map_err(|_| PyTypeError::new_err("each block payload must be a dict"))?;
        nif.blocks.push(dict_to_block(block_dict)?);
    }

    if let Some(path_obj) = payload.get_item("path")? {
        nif.path = Some(PathBuf::from(path_obj.extract::<String>()?));
    }
    Ok(nif)
}

fn dict_to_header(d: &Bound<'_, PyDict>) -> PyResult<NifHeader> {
    let mut h = NifHeader::default();
    set_string_field(d, "header_string", &mut h.header_string)?;
    set_version_field(d, &mut h.version)?;
    set_u32_field(d, "version_packed", &mut h.version_packed)?;
    set_u8_field(d, "endian_type", &mut h.endian_type)?;
    set_u32_field(d, "user_version", &mut h.user_version)?;
    set_u32_field(d, "bs_version", &mut h.bs_version)?;
    set_u32_field(d, "num_blocks", &mut h.num_blocks)?;
    set_string_field(d, "creator", &mut h.creator)?;
    set_vec_string_field(d, "export_info", &mut h.export_info)?;
    set_vec_u8_field(d, "sf_export_data", &mut h.sf_export_data)?;
    set_vec_string_field(d, "block_type_names", &mut h.block_type_names)?;
    set_vec_u16_field(d, "block_type_index", &mut h.block_type_index)?;
    set_vec_u32_field(d, "block_sizes", &mut h.block_sizes)?;
    set_vec_string_field(d, "strings", &mut h.strings)?;
    set_u32_field(d, "max_string_length", &mut h.max_string_length)?;
    set_u32_field(d, "num_groups", &mut h.num_groups)?;
    set_vec_u32_field(d, "groups", &mut h.groups)?;
    set_vec_i32_field(d, "footer_roots", &mut h.footer_roots)?;
    Ok(h)
}

fn dict_to_block(d: &Bound<'_, PyDict>) -> PyResult<NifBlock> {
    let block_id = required_item(d, "block_id")?.extract::<usize>()?;
    let type_name = required_item(d, "type_name")?.extract::<String>()?;
    let mut block = NifBlock::new(block_id, type_name);

    let fields_obj = required_item(d, "fields")?;
    let fields = fields_obj
        .cast::<PyDict>()
        .map_err(|_| PyTypeError::new_err("block['fields'] must be a dict"))?;
    for (key, value) in fields.iter() {
        block
            .fields
            .insert(key.extract::<String>()?, py_to_nif(&value));
    }

    if let Some(remainder) = d.get_item("remainder")? {
        block.remainder = remainder.extract::<Vec<u8>>()?;
    }
    Ok(block)
}

fn required_item<'py>(d: &Bound<'py, PyDict>, key: &str) -> PyResult<Bound<'py, PyAny>> {
    d.get_item(key)?
        .ok_or_else(|| PyKeyError::new_err(format!("missing payload key: {key}")))
}

fn set_string_field(d: &Bound<'_, PyDict>, key: &str, target: &mut String) -> PyResult<()> {
    if let Some(value) = d.get_item(key)? {
        *target = value.extract::<String>()?;
    }
    Ok(())
}

fn set_version_field(d: &Bound<'_, PyDict>, target: &mut (u8, u8, u8, u8)) -> PyResult<()> {
    if let Some(value) = d.get_item("version")? {
        if let Ok(tuple_value) = value.extract::<(u8, u8, u8, u8)>() {
            *target = tuple_value;
        } else {
            let values = value.extract::<Vec<u8>>()?;
            if values.len() != 4 {
                return Err(PyTypeError::new_err("header['version'] must have 4 items"));
            }
            *target = (values[0], values[1], values[2], values[3]);
        }
    }
    Ok(())
}

fn set_u8_field(d: &Bound<'_, PyDict>, key: &str, target: &mut u8) -> PyResult<()> {
    if let Some(value) = d.get_item(key)? {
        *target = value.extract::<u8>()?;
    }
    Ok(())
}

fn set_u32_field(d: &Bound<'_, PyDict>, key: &str, target: &mut u32) -> PyResult<()> {
    if let Some(value) = d.get_item(key)? {
        *target = value.extract::<u32>()?;
    }
    Ok(())
}

fn set_vec_string_field(
    d: &Bound<'_, PyDict>,
    key: &str,
    target: &mut Vec<String>,
) -> PyResult<()> {
    if let Some(value) = d.get_item(key)? {
        *target = value.extract::<Vec<String>>()?;
    }
    Ok(())
}

fn set_vec_u8_field(d: &Bound<'_, PyDict>, key: &str, target: &mut Vec<u8>) -> PyResult<()> {
    if let Some(value) = d.get_item(key)? {
        *target = value.extract::<Vec<u8>>()?;
    }
    Ok(())
}

fn set_vec_u16_field(d: &Bound<'_, PyDict>, key: &str, target: &mut Vec<u16>) -> PyResult<()> {
    if let Some(value) = d.get_item(key)? {
        *target = value.extract::<Vec<u16>>()?;
    }
    Ok(())
}

fn set_vec_u32_field(d: &Bound<'_, PyDict>, key: &str, target: &mut Vec<u32>) -> PyResult<()> {
    if let Some(value) = d.get_item(key)? {
        *target = value.extract::<Vec<u32>>()?;
    }
    Ok(())
}

fn set_vec_i32_field(d: &Bound<'_, PyDict>, key: &str, target: &mut Vec<i32>) -> PyResult<()> {
    if let Some(value) = d.get_item(key)? {
        *target = value.extract::<Vec<i32>>()?;
    }
    Ok(())
}
