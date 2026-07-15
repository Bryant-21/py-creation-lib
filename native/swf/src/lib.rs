//! Byte-exact SWF tooling for marker-icon injection (FO76 → FO4).
//!
//! Rust owns the binary-exact work (container (de)compression, tag-stream
//! splice, SymbolClass parse, and — later — char-ID remap and ABC editing);
//! Python orchestrates. The existing pure-Python `creation_lib.swf` codec is
//! byte-lossy and must not be used to edit real menu SWFs.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyBytes;

pub mod abc;
pub mod container;
pub mod inject;
pub mod symbolclass;

use container::{Signature, assemble, decompress, split_tags};
use symbolclass::parse_symbol_table;

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
    m.add_function(wrap_pyfunction!(inject_symbols_into, m)?)?;
    m.add_function(wrap_pyfunction!(inject_symbols_renamed_into, m)?)?;
    Ok(())
}

#[pymodule]
fn swf_native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    register_module(m)
}
