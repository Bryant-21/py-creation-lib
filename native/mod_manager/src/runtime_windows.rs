use pyo3::prelude::*;
#[cfg(windows)]
mod windows {
    use super::*;
    use crate::invalid;
    use std::{collections::BTreeSet, ffi::c_void};
    #[repr(C)]
    struct Guid {
        data1: u32,
        data2: u16,
        data3: u16,
        data4: [u8; 8],
    }
    #[repr(C)]
    struct Entry {
        size: u32,
        usage: u32,
        pid: u32,
        heap: usize,
        module: u32,
        threads: u32,
        parent: u32,
        priority: i32,
        flags: u32,
        exe: [u16; 260],
    }
    #[link(name = "shell32")]
    unsafe extern "system" {
        fn SHGetKnownFolderPath(
            id: *const Guid,
            flags: u32,
            token: *mut c_void,
            result: *mut *mut u16,
        ) -> i32;
    }
    #[link(name = "ole32")]
    unsafe extern "system" {
        fn CoTaskMemFree(memory: *mut c_void);
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> *mut c_void;
        fn Process32FirstW(handle: *mut c_void, entry: *mut Entry) -> i32;
        fn Process32NextW(handle: *mut c_void, entry: *mut Entry) -> i32;
        fn CloseHandle(handle: *mut c_void) -> i32;
        fn GetLastError() -> u32;
    }
    struct Memory(*mut u16);
    impl Drop for Memory {
        fn drop(&mut self) {
            unsafe { CoTaskMemFree(self.0.cast()) };
        }
    }
    struct Handle(*mut c_void);
    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.0) };
        }
    }
    pub fn known_folder(identifier: &str) -> PyResult<String> {
        let id = uuid::Uuid::parse_str(identifier).map_err(|e| invalid(e.to_string()))?;
        let (data1, data2, data3, data4) = id.as_fields();
        let guid = Guid {
            data1,
            data2,
            data3,
            data4: *data4,
        };
        let mut output = std::ptr::null_mut();
        let hr = unsafe { SHGetKnownFolderPath(&guid, 0, std::ptr::null_mut(), &mut output) };
        if hr != 0 {
            return Err(invalid(format!(
                "Windows could not resolve the game user folder ({hr:#x})."
            )));
        }
        let memory = Memory(output);
        if memory.0.is_null() {
            return Err(invalid("Windows returned an empty game user folder."));
        }
        let mut length = 0;
        while length < 32768 && unsafe { *memory.0.add(length) } != 0 {
            length += 1;
        }
        if length == 32768 {
            return Err(invalid(
                "Windows game user folder exceeds its bounded limit.",
            ));
        }
        Ok(String::from_utf16_lossy(unsafe {
            std::slice::from_raw_parts(memory.0, length)
        }))
    }
    pub fn process_names() -> PyResult<BTreeSet<String>> {
        let raw = unsafe { CreateToolhelp32Snapshot(2, 0) };
        if raw as isize == -1 {
            return Err(invalid(
                "Windows could not enumerate running process names.",
            ));
        }
        let handle = Handle(raw);
        let mut entry: Entry = unsafe { std::mem::zeroed() };
        entry.size = std::mem::size_of::<Entry>() as u32;
        let mut present = unsafe { Process32FirstW(handle.0, &mut entry) };
        let mut names = BTreeSet::new();
        let mut count = 0;
        while present != 0 {
            count += 1;
            if count > 32768 {
                return Err(invalid(
                    "Windows process list exceeds its bounded safety limit.",
                ));
            }
            let end = entry.exe.iter().position(|&v| v == 0).unwrap_or(260);
            names.insert(crate::saves::fold(&String::from_utf16_lossy(
                &entry.exe[..end],
            )));
            present = unsafe { Process32NextW(handle.0, &mut entry) };
        }
        if ![0, 18].contains(&unsafe { GetLastError() }) {
            return Err(invalid(
                "Windows process enumeration failed before completion.",
            ));
        }
        Ok(names)
    }
}
#[pyfunction]
fn runtime_known_folder(identifier: &str) -> PyResult<String> {
    #[cfg(windows)]
    {
        windows::known_folder(identifier)
    }
    #[cfg(not(windows))]
    {
        let _ = identifier;
        Err(crate::invalid("Native USVFS launch requires Windows."))
    }
}
#[pyfunction]
fn runtime_process_names() -> PyResult<std::collections::BTreeSet<String>> {
    #[cfg(windows)]
    {
        windows::process_names()
    }
    #[cfg(not(windows))]
    {
        Err(crate::invalid(
            "Windows process enumeration is required for this runtime adapter.",
        ))
    }
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(runtime_known_folder, m)?)?;
    m.add_function(wrap_pyfunction!(runtime_process_names, m)?)?;
    Ok(())
}
