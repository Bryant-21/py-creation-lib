//! Byte-exact SWF tooling for marker-icon injection (FO76 → FO4).
//!
//! Rust owns the binary-exact work (container (de)compression, tag-stream
//! splice, SymbolClass parse, char-ID remap, ABC synthesis); Python
//! orchestrates. The pure-Python `creation_lib.swf` codec is byte-lossy and
//! must not be used to edit real menu SWFs.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyBytes;

pub mod abc;
pub mod class_abc;
pub mod container;
pub mod inject;
pub mod symbolclass;

use container::{Signature, assemble, decompress, split_tags};
use symbolclass::parse_symbol_table;

/// Every class name defined by a DoABC in this movie body, and every name a
/// SymbolClass entry binds to — the two halves the validator compares.
pub fn class_and_symbol_names(body: &[u8]) -> Result<(Vec<String>, Vec<String>), String> {
    let spans = split_tags(body)?;
    let mut defined = Vec::new();
    let mut bound = Vec::new();
    for span in &spans {
        if span.code == abc::DO_ABC_DEFINE || span.code == abc::DO_ABC {
            defined.extend(abc::parse_abc_class_names(
                span.code,
                &body[span.body_range()],
            )?);
        } else if span.code == 76 {
            for e in parse_symbol_table(&body[span.body_range()])? {
                bound.push(e.name);
            }
        }
    }
    Ok((defined, bound))
}

/// SymbolClass export names in this movie body that no DoABC defines, in file
/// order and de-duplicated. Empty means every binding resolves.
pub fn unbacked_symbol_class_names(body: &[u8]) -> Result<Vec<String>, String> {
    let (defined, bound) = class_and_symbol_names(body)?;
    let defined: std::collections::HashSet<&str> = defined.iter().map(String::as_str).collect();
    let mut out: Vec<String> = Vec::new();
    for name in bound {
        if !defined.contains(name.as_str()) && !out.contains(&name) {
            out.push(name);
        }
    }
    Ok(out)
}

fn sig_str(s: Signature) -> &'static str {
    match s {
        Signature::Uncompressed => "FWS",
        Signature::Zlib => "CWS",
        Signature::Lzma => "ZWS",
    }
}

/// `(signature, version, file_length_field, decompressed_total, num_tags)`.
#[pyfunction]
fn swf_info(data: &[u8]) -> PyResult<(String, u8, usize, usize, usize)> {
    let movie = decompress(data).map_err(PyValueError::new_err)?;
    let spans = split_tags(&movie.body).map_err(PyValueError::new_err)?;
    let file_length = u32::from_le_bytes([data[4], data[5], data[6], data[7]]) as usize;
    Ok((
        sig_str(movie.signature).to_string(),
        movie.version,
        file_length,
        movie.body.len() + 8,
        spans.len(),
    ))
}

/// `(character_id, export_name)` for every SymbolClass (tag 76) entry, in file
/// order. FO4 binds REFR.TNAM marker types to icons by this order.
#[pyfunction]
fn list_symbols(data: &[u8]) -> PyResult<Vec<(u16, String)>> {
    let movie = decompress(data).map_err(PyValueError::new_err)?;
    let spans = split_tags(&movie.body).map_err(PyValueError::new_err)?;
    let mut out = Vec::new();
    for span in &spans {
        if span.code == 76 {
            let body = &movie.body[span.body_range()];
            for e in parse_symbol_table(body).map_err(PyValueError::new_err)? {
                out.push((e.character_id, e.name));
            }
        }
    }
    Ok(out)
}

/// `(tag_code, count)` sorted by code — a quick structural fingerprint.
#[pyfunction]
fn tag_histogram(data: &[u8]) -> PyResult<Vec<(u16, usize)>> {
    let movie = decompress(data).map_err(PyValueError::new_err)?;
    let spans = split_tags(&movie.body).map_err(PyValueError::new_err)?;
    let mut counts: std::collections::BTreeMap<u16, usize> = std::collections::BTreeMap::new();
    for s in &spans {
        *counts.entry(s.code).or_default() += 1;
    }
    Ok(counts.into_iter().collect())
}

/// True iff the tag splitter consumes the whole movie body and ends on an End
/// tag — i.e. our parse tiles the file exactly.
#[pyfunction]
fn roundtrip_ok(data: &[u8]) -> PyResult<bool> {
    let movie = decompress(data).map_err(PyValueError::new_err)?;
    let spans = split_tags(&movie.body).map_err(PyValueError::new_err)?;
    let consumed = spans.last().map(|s| s.end()).unwrap_or(0);
    let ends_clean = spans.last().map(|s| s.code == 0).unwrap_or(false);
    Ok(consumed == movie.body.len() && ends_clean)
}

/// For each DoABC tag, the constant-pool string table (where every AS3 class name
/// lives): `(tag_code, minor, major, int_count, uint_count, double_count, strings)`.
/// Read-only — parses up through the string pool and stops.
#[pyfunction]
fn abc_string_pools(data: &[u8]) -> PyResult<Vec<(u16, u16, u16, u32, u32, u32, Vec<String>)>> {
    let movie = decompress(data).map_err(PyValueError::new_err)?;
    let spans = split_tags(&movie.body).map_err(PyValueError::new_err)?;
    let mut out = Vec::new();
    for span in &spans {
        if span.code == abc::DO_ABC_DEFINE || span.code == abc::DO_ABC {
            let pool = abc::parse_abc_strings(span.code, &movie.body[span.body_range()])
                .map_err(PyValueError::new_err)?;
            out.push((
                span.code,
                pool.minor,
                pool.major,
                pool.int_count,
                pool.uint_count,
                pool.double_count,
                pool.strings,
            ));
        }
    }
    Ok(out)
}

/// Fully-qualified names (`package.Class`, or bare `Class` in the unnamed
/// package) of every class *defined* by a DoABC tag in this SWF, in file order.
#[pyfunction]
fn abc_class_names(data: &[u8]) -> PyResult<Vec<String>> {
    let movie = decompress(data).map_err(PyValueError::new_err)?;
    let (defined, _) = class_and_symbol_names(&movie.body).map_err(PyValueError::new_err)?;
    Ok(defined)
}

/// SymbolClass export names that no DoABC in the same SWF defines, in file
/// order and de-duplicated. A non-empty result means the file ships a dangling
/// binding: the engine has a character id pointing at a class that does not
/// exist, so construction of that symbol fails.
#[pyfunction]
fn unbacked_symbol_classes(data: &[u8]) -> PyResult<Vec<String>> {
    let movie = decompress(data).map_err(PyValueError::new_err)?;
    unbacked_symbol_class_names(&movie.body).map_err(PyValueError::new_err)
}

/// Build a `DoABCDefine` (tag 82) *body* defining one `flash.display.MovieClip`
/// subclass with an empty constructor per name. The caller writes the tag header
/// and must place the tag before the SymbolClass that binds these names.
#[pyfunction]
fn build_movieclip_class_doabc<'py>(
    py: Python<'py>,
    names: Vec<String>,
) -> PyResult<Bound<'py, PyBytes>> {
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let abc = class_abc::build_movieclip_class_abc(&refs).map_err(PyValueError::new_err)?;
    Ok(PyBytes::new(py, &class_abc::do_abc_define_body(&abc)))
}

/// Compile ActionScript 3 source files to a `DoABCDefine` (tag 82) tag *body*.
///
/// Each element of `sources` is the full text of one `.as` file; AS3 allows a
/// single package per file, so a widget's document class and the interface it
/// implements arrive as separate entries. Order does not matter — types are
/// sorted so a base class or interface is defined before whatever depends on
/// it. The caller writes the tag header and must place the tag ahead of the
/// `SymbolClass` that binds these classes.
#[pyfunction]
fn compile_as3_do_abc<'py>(py: Python<'py>, sources: Vec<String>) -> PyResult<Bound<'py, PyBytes>> {
    let refs: Vec<&str> = sources.iter().map(String::as_str).collect();
    let body = class_abc::compile_sources_to_do_abc(&refs).map_err(PyValueError::new_err)?;
    Ok(PyBytes::new(py, &body))
}

/// Fully-qualified names of every class an ActionScript source set defines.
///
/// A packer needs this to check that each `SymbolClass` export it is about to
/// write is actually backed by a compiled class, before the SWF ships.
#[pyfunction]
fn compile_as3_class_names(sources: Vec<String>) -> PyResult<Vec<String>> {
    let refs: Vec<&str> = sources.iter().map(String::as_str).collect();
    let body = class_abc::compile_sources_to_do_abc(&refs).map_err(PyValueError::new_err)?;
    abc::parse_abc_class_names(abc::DO_ABC_DEFINE, &body).map_err(PyValueError::new_err)
}

/// Inject the named SymbolClass symbols (with their full character closures)
/// from `src` into `dst`, returning the re-assembled destination SWF bytes.
#[pyfunction]
fn inject_symbols_into<'py>(
    py: Python<'py>,
    src: &[u8],
    dst: &[u8],
    names: Vec<String>,
) -> PyResult<Bound<'py, PyBytes>> {
    let src_movie = decompress(src).map_err(PyValueError::new_err)?;
    let dst_movie = decompress(dst).map_err(PyValueError::new_err)?;
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let out = inject::inject_symbols(&src_movie, &dst_movie, &name_refs)
        .map_err(PyValueError::new_err)?;
    let bytes = assemble(out.signature, out.version, &out.body).map_err(PyValueError::new_err)?;
    Ok(PyBytes::new(py, &bytes))
}

/// Like [`inject_symbols_into`] but each entry is `(source_symbol, export_name)`,
/// so a symbol can be registered in `dst` under a different SymbolClass export
/// name than it has in `src` (used to avoid colliding with a stock `dst` export).
#[pyfunction]
fn inject_symbols_renamed_into<'py>(
    py: Python<'py>,
    src: &[u8],
    dst: &[u8],
    pairs: Vec<(String, String)>,
) -> PyResult<Bound<'py, PyBytes>> {
    let src_movie = decompress(src).map_err(PyValueError::new_err)?;
    let dst_movie = decompress(dst).map_err(PyValueError::new_err)?;
    let pair_refs: Vec<(&str, &str)> = pairs
        .iter()
        .map(|(s, e)| (s.as_str(), e.as_str()))
        .collect();
    let out = inject::inject_symbols_renamed(&src_movie, &dst_movie, &pair_refs)
        .map_err(PyValueError::new_err)?;
    let bytes = assemble(out.signature, out.version, &out.body).map_err(PyValueError::new_err)?;
    Ok(PyBytes::new(py, &bytes))
}

pub fn register_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(swf_info, m)?)?;
    m.add_function(wrap_pyfunction!(list_symbols, m)?)?;
    m.add_function(wrap_pyfunction!(tag_histogram, m)?)?;
    m.add_function(wrap_pyfunction!(roundtrip_ok, m)?)?;
    m.add_function(wrap_pyfunction!(abc_string_pools, m)?)?;
    m.add_function(wrap_pyfunction!(abc_class_names, m)?)?;
    m.add_function(wrap_pyfunction!(unbacked_symbol_classes, m)?)?;
    m.add_function(wrap_pyfunction!(build_movieclip_class_doabc, m)?)?;
    m.add_function(wrap_pyfunction!(compile_as3_do_abc, m)?)?;
    m.add_function(wrap_pyfunction!(compile_as3_class_names, m)?)?;
    m.add_function(wrap_pyfunction!(inject_symbols_into, m)?)?;
    m.add_function(wrap_pyfunction!(inject_symbols_renamed_into, m)?)?;
    Ok(())
}

#[pymodule]
fn swf_native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    register_module(m)
}
