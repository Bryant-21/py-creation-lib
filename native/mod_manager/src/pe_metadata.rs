use crate::{invalid, reject_links};
use pyo3::{prelude::*, types::PyBytes};
use std::path::PathBuf;

fn word(raw: &[u8], offset: usize) -> PyResult<usize> {
    let bytes = raw
        .get(offset..offset + 2)
        .ok_or_else(|| invalid("Native DLL contains out-of-bounds PE metadata."))?;
    Ok(u16::from_le_bytes(bytes.try_into().unwrap()) as usize)
}
fn dword(raw: &[u8], offset: usize) -> PyResult<usize> {
    let bytes = raw
        .get(offset..offset + 4)
        .ok_or_else(|| invalid("Native DLL contains out-of-bounds PE metadata."))?;
    Ok(u32::from_le_bytes(bytes.try_into().unwrap()) as usize)
}

fn export(raw: &[u8], symbol: &[u8], size: usize) -> PyResult<Option<Vec<u8>>> {
    if !raw.starts_with(b"MZ") {
        return Err(invalid("Native DLL is not a PE image."));
    }
    let offset = dword(raw, 0x3c)?;
    if raw.get(offset..offset + 4) != Some(b"PE\0\0") {
        return Err(invalid("Native DLL has an invalid PE header."));
    }
    let sections = word(raw, offset + 6)?;
    let optional_size = word(raw, offset + 20)?;
    if !(1..=96).contains(&sections) {
        return Err(invalid("Native DLL section count is invalid."));
    }
    let optional = offset + 24;
    let magic = word(raw, optional)?;
    let directory = optional
        + match magic {
            0x10b => 96,
            0x20b => 112,
            _ => return Err(invalid("Native DLL optional header is unsupported.")),
        };
    if directory + 8 > optional + optional_size {
        return Ok(None);
    }
    let export_rva = dword(raw, directory)?;
    let export_size = dword(raw, directory + 4)?;
    if export_rva == 0 {
        return Ok(None);
    }
    let mut mappings = Vec::new();
    for index in 0..sections {
        let position = optional + optional_size + index * 40;
        let virtual_size = dword(raw, position + 8)?;
        let virtual_address = dword(raw, position + 12)?;
        let raw_size = dword(raw, position + 16)?;
        let pointer = dword(raw, position + 20)?;
        if pointer + raw_size > raw.len() {
            return Err(invalid("Native DLL section extends beyond the file."));
        }
        mappings.push((
            virtual_address,
            virtual_size.max(raw_size),
            pointer,
            raw_size,
        ));
    }
    let at = |rva: usize, length: usize| -> PyResult<(usize, usize)> {
        let mut selected = None;
        for &(virtual_address, span, pointer, raw_size) in &mappings {
            if rva >= virtual_address && rva < virtual_address + span {
                let delta = rva - virtual_address;
                if selected.is_some() || delta > raw_size || raw_size - delta < length {
                    return Err(invalid("Native DLL export points outside one raw section."));
                }
                selected = Some((pointer + delta, raw_size - delta));
            }
        }
        selected.ok_or_else(|| invalid("Native DLL export points outside one raw section."))
    };
    let export = at(export_rva, 40)?.0;
    let count = dword(raw, export + 20)?;
    let names = dword(raw, export + 24)?;
    if count > 100000 || names > 100000 {
        return Err(invalid("Native DLL export count exceeds its safety limit."));
    }
    if count == 0 || names == 0 {
        return Ok(None);
    }
    let functions = at(dword(raw, export + 28)?, count * 4)?.0;
    let name_table = at(dword(raw, export + 32)?, names * 4)?.0;
    let ordinals = at(dword(raw, export + 36)?, names * 2)?.0;
    for index in 0..names {
        let (start, available) = at(dword(raw, name_table + index * 4)?, 1)?;
        let length = raw[start..start + available.min(256)]
            .iter()
            .position(|v| *v == 0)
            .ok_or_else(|| invalid("Native DLL export name is unterminated."))?;
        if &raw[start..start + length] != symbol {
            continue;
        }
        let ordinal = word(raw, ordinals + index * 2)?;
        if ordinal >= count {
            return Err(invalid("Native DLL export ordinal is invalid."));
        }
        let rva = dword(raw, functions + ordinal * 4)?;
        if rva >= export_rva && rva < export_rva + export_size {
            return Err(invalid("Native DLL data export is forwarded."));
        }
        let start = at(rva, size)?.0;
        return Ok(Some(raw[start..start + size].to_vec()));
    }
    Ok(None)
}

#[pyfunction]
fn pe_exported_data(
    py: Python<'_>,
    path: PathBuf,
    symbol: &str,
    size: i64,
    fingerprint: &Bound<'_, PyAny>,
) -> PyResult<Option<Py<PyBytes>>> {
    if symbol.is_empty() || !symbol.is_ascii() || symbol.len() > 255 || !(1..=4096).contains(&size)
    {
        return Err(invalid("Invalid bounded native export request."));
    }
    reject_links(&path)?;
    let expected = fingerprint.call1((&path,))?;
    if std::fs::metadata(&path)?.len() > 64 * 1024 * 1024 {
        return Err(invalid("Native DLL metadata inspection exceeds 64 MiB."));
    }
    let raw = std::fs::read(&path)?;
    reject_links(&path)?;
    if !fingerprint.call1((&path,))?.eq(expected)? {
        return Err(invalid("Native DLL changed during metadata inspection."));
    }
    Ok(export(&raw, symbol.as_bytes(), size as usize)?.map(|v| PyBytes::new(py, &v).unbind()))
}

#[cfg(windows)]
fn version_resource(path: &std::path::Path) -> PyResult<Option<[u32; 13]>> {
    use std::{ffi::c_void, os::windows::ffi::OsStrExt};
    #[link(name = "version")]
    unsafe extern "system" {
        fn GetFileVersionInfoSizeW(filename: *const u16, handle: *mut u32) -> u32;
        fn GetFileVersionInfoW(
            filename: *const u16,
            handle: u32,
            length: u32,
            data: *mut c_void,
        ) -> i32;
        fn VerQueryValueW(
            block: *const c_void,
            query: *const u16,
            buffer: *mut *mut c_void,
            length: *mut u32,
        ) -> i32;
    }
    let name: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut ignored = 0;
    let size = unsafe { GetFileVersionInfoSizeW(name.as_ptr(), &mut ignored) };
    if size == 0 {
        return Ok(None);
    }
    if size > 16 * 1024 * 1024 {
        return Err(invalid("PE version resource exceeds its safety limit."));
    }
    let mut data = vec![0u8; size as usize];
    if unsafe { GetFileVersionInfoW(name.as_ptr(), 0, size, data.as_mut_ptr().cast()) } == 0 {
        return Err(invalid("Cannot read the executable version resource."));
    }
    let mut address = std::ptr::null_mut();
    let mut count = 0;
    if unsafe {
        VerQueryValueW(
            data.as_ptr().cast(),
            [92u16, 0].as_ptr(),
            &mut address,
            &mut count,
        )
    } == 0
        || count < 52
    {
        return Ok(None);
    }
    let start = address as usize;
    let base = data.as_ptr() as usize;
    if data.len() < 52 || start < base || start > base + data.len() - 52 {
        return Err(invalid("Executable fixed version resource is invalid."));
    }
    let bytes = &data[start - base..start - base + 52];
    let mut values = [0u32; 13];
    for (index, value) in values.iter_mut().enumerate() {
        *value = u32::from_le_bytes(bytes[index * 4..index * 4 + 4].try_into().unwrap());
    }
    Ok(Some(values))
}

#[cfg(not(windows))]
fn version_resource(_path: &std::path::Path) -> PyResult<Option<[u32; 13]>> {
    Err(invalid("Executable version discovery requires Windows."))
}

#[pyfunction]
fn pe_file_version(
    path: PathBuf,
    fingerprint: &Bound<'_, PyAny>,
) -> PyResult<Option<(u32, u32, u32, u32)>> {
    reject_links(&path)?;
    let expected = fingerprint.call1((&path,))?;
    let Some(values) = version_resource(&path)? else {
        return Ok(None);
    };
    reject_links(&path)?;
    if !fingerprint.call1((&path,))?.eq(expected)? {
        return Err(invalid("Executable changed while reading its build."));
    }
    if values[0] != 0xFEEF04BD {
        return Err(invalid("Executable fixed version resource is invalid."));
    }
    Ok(Some((
        values[2] >> 16,
        values[2] & 0xffff,
        values[3] >> 16,
        values[3] & 0xffff,
    )))
}

#[pyfunction]
fn pe_packed_runtime(version: Vec<i64>) -> PyResult<u32> {
    if version.len() < 3
        || version[0] < 0
        || version[0] > 255
        || version[1] < 0
        || version[1] > 255
        || version[2] < 0
        || version[2] > 4095
    {
        return Err(invalid(
            "Detected runtime build exceeds the F4SE version format.",
        ));
    }
    Ok((version[0] as u32) << 24 | (version[1] as u32) << 16 | (version[2] as u32) << 4)
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(pe_machine, m)?)?;
    m.add_function(wrap_pyfunction!(pe_exported_data, m)?)?;
    m.add_function(wrap_pyfunction!(pe_file_version, m)?)?;
    m.add_function(wrap_pyfunction!(pe_packed_runtime, m)?)?;
    Ok(())
}

#[pyfunction]
pub fn pe_machine(path: PathBuf) -> PyResult<u16> {
    use std::io::{Read, Seek, SeekFrom};
    reject_links(&path)?;
    let mut file = std::fs::File::open(&path)?;
    let mut signature = [0; 2];
    file.read_exact(&mut signature)?;
    if &signature != b"MZ" {
        return Err(invalid(format!(
            "Not a Windows executable image: {}",
            path.display()
        )));
    }
    file.seek(SeekFrom::Start(60))?;
    let mut address = [0; 4];
    file.read_exact(&mut address)?;
    let offset = u32::from_le_bytes(address);
    if offset > 16 * 1024 * 1024 {
        return Err(invalid("Invalid PE header offset."));
    }
    file.seek(SeekFrom::Start(offset as u64))?;
    file.read_exact(&mut address)?;
    if &address != b"PE\0\0" {
        return Err(invalid(format!(
            "Invalid Windows PE header: {}",
            path.display()
        )));
    }
    file.read_exact(&mut signature)?;
    Ok(u16::from_le_bytes(signature))
}
