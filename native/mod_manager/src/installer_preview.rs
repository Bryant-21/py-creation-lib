use crate::{check_cancel, invalid, python_io::with_context, relative_path, saves::fold};
use pyo3::{prelude::*, types::PyBytes};
use std::collections::BTreeMap;
fn read(
    package: &Bound<'_, PyAny>,
    source: &str,
    limit: usize,
    cancel: &Bound<'_, PyAny>,
    message: &str,
) -> PyResult<Vec<u8>> {
    check_cancel(Some(cancel))?;
    let bytes = with_context(&package.call_method1("open", (source,))?, |stream| {
        stream
            .call_method1("read", (limit + 1,))?
            .extract::<Vec<u8>>()
    })?;
    if bytes.len() > limit {
        return Err(invalid(message));
    }
    Ok(bytes)
}
#[pyfunction]
fn installer_preview_metadata(
    package: &Bound<'_, PyAny>,
    cancel: &Bound<'_, PyAny>,
) -> PyResult<BTreeMap<String, String>> {
    let py = package.py();
    let source = format!(
        "{}fomod/info.xml",
        package.getattr("prefix")?.extract::<String>()?
    );
    let key = fold(&source);
    if !package.getattr("files")?.contains(&key)? {
        return Ok(BTreeMap::new());
    }
    if package.getattr("sizes")?.get_item(&key)?.extract::<u64>()? > 2 * 1024 * 1024 {
        return Err(invalid("Installer info.xml exceeds 2 MiB."));
    }
    let bytes = read(
        package,
        &source,
        2 * 1024 * 1024,
        cancel,
        "Installer info.xml grew beyond 2 MiB.",
    )?;
    let xml = py.import("xml.etree.ElementTree")?;
    let parse_error = xml.getattr("ParseError")?;
    let parsed = (|| -> PyResult<Bound<'_, PyAny>> {
        let encoding = if bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff]) {
            "utf-16"
        } else {
            "utf-8-sig"
        };
        let text = PyBytes::new(py, &bytes)
            .call_method1("decode", (encoding,))?
            .extract::<String>()?;
        let upper = text.to_uppercase();
        let declaration = upper.split("<!").skip(1).any(|v| {
            let v = v.trim_start();
            v.starts_with("DOCTYPE") || v.starts_with("ENTITY")
        });
        if text.contains('\0') || declaration {
            return Err(invalid(
                "Installer info.xml cannot contain DTD/entity declarations or nulls.",
            ));
        }
        xml.call_method1("fromstring", (text,))
    })()
    .map_err(|e| {
        if e.is_instance_of::<pyo3::exceptions::PyUnicodeError>(py)
            || e.is_instance(py, &parse_error)
        {
            invalid("Installer info.xml is invalid UTF-8/BOM-marked UTF-16 XML.")
        } else {
            e
        }
    })?;
    if parsed.getattr("tag")?.extract::<String>()? != "fomod" {
        return Err(invalid("Installer info.xml has an unsupported root."));
    }
    let mut result = BTreeMap::new();
    let mut stack = vec![(parsed, 0)];
    let mut count = 0;
    while let Some((node, depth)) = stack.pop() {
        check_cancel(Some(cancel))?;
        count += 1;
        if depth > 40 || count > 30_000 {
            return Err(invalid(
                "Installer info.xml exceeds its element/depth limits.",
            ));
        }
        let tag = node.getattr("tag")?.extract::<String>()?;
        if ["Name", "Author", "Version", "Id", "Website", "Description"].contains(&tag.as_str()) {
            let text = node
                .getattr("text")?
                .extract::<Option<String>>()?
                .unwrap_or_default();
            if result.contains_key(&tag)
                || node.len()? > 0
                || text.chars().count() > if tag == "Description" { 65536 } else { 4096 }
            {
                return Err(invalid(
                    "Installer info.xml fields must be unique bounded plain text.",
                ));
            }
            result.insert(tag, text.trim().to_string());
        }
        for child in node.try_iter()? {
            stack.push((child?, depth + 1));
        }
    }
    Ok(result)
}
#[pyfunction]
fn installer_preview_image(
    package: &Bound<'_, PyAny>,
    source: &str,
    cancel: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let py = package.py();
    let source = format!(
        "{}{}",
        package.getattr("prefix")?.extract::<String>()?,
        relative_path(source, false)?
    );
    let size = package
        .getattr("sizes")?
        .call_method1("get", (fold(&source),))?;
    if size.is_none() || size.extract::<u64>()? > 8 * 1024 * 1024 {
        return Err(invalid(
            "Installer artwork must be an existing package member within 8 MiB.",
        ));
    }
    let bytes = read(
        package,
        &source,
        8 * 1024 * 1024,
        cancel,
        "Installer artwork grew beyond its 8 MiB limit.",
    )?;
    let result = decode_image(py, &bytes)?;
    check_cancel(Some(cancel))?;
    Ok(result)
}
pub fn decode_image(py: Python<'_>, bytes: &[u8]) -> PyResult<Py<PyAny>> {
    let image = py.import("PIL.Image")?;
    let bomb_warning = image.getattr("DecompressionBombWarning")?;
    let bomb_error = image.getattr("DecompressionBombError")?;
    let warnings = py.import("warnings")?;
    let result = with_context(&warnings.call_method0("catch_warnings")?, |_| {
        warnings.call_method1(
            "simplefilter",
            ("error", image.getattr("DecompressionBombWarning")?),
        )?;
        let buffer = py
            .import("io")?
            .getattr("BytesIO")?
            .call1((PyBytes::new(py, &bytes),))?;
        with_context(&image.call_method1("open", (buffer,))?, |decoded| {
            let format = decoded.getattr("format")?.extract::<String>()?;
            let width = decoded.getattr("width")?.extract::<u64>()?;
            let height = decoded.getattr("height")?.extract::<u64>()?;
            if !["PNG", "JPEG", "BMP", "GIF", "WEBP", "DDS"].contains(&format.as_str())
                || width.saturating_mul(height) > 16_000_000
            {
                return Err(invalid(
                    "Artwork supports PNG/JPEG/BMP/GIF/WebP/DDS first frames within 16 megapixels.",
                ));
            }
            decoded.call_method0("load")?;
            decoded.call_method1("thumbnail", ((1024, 1024),))?;
            Ok(py
                .import("numpy")?
                .call_method1("asarray", (decoded.call_method1("convert", ("RGB",))?,))?
                .call_method0("copy")?
                .unbind())
        })
    })
    .map_err(|e| {
        if e.is_instance_of::<pyo3::exceptions::PyOSError>(py)
            || e.is_instance_of::<pyo3::exceptions::PyValueError>(py)
            || e.is_instance(py, &bomb_warning)
            || e.is_instance(py, &bomb_error)
        {
            invalid(format!(
                "Installer artwork could not be safely decoded: {}",
                e.value(py)
            ))
        } else {
            e
        }
    })?;
    Ok(result)
}
#[pyfunction]
fn package_preview(
    package: &Bound<'_, PyAny>,
    source: &str,
    cancel: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let py = package.py();
    let source = relative_path(source, false)?;
    let key = fold(&source);
    if !package.getattr("files")?.contains(&key)? {
        return Err(invalid("Select an existing package file to preview."));
    }
    package.call_method1("validate_sources", ((&source,), cancel))?;
    let size: usize = package.getattr("sizes")?.get_item(&key)?.extract()?;
    let preview = py
        .import("creation_lib.mod_manager.package_preview")?
        .getattr("Preview")?;
    let kwargs = pyo3::types::PyDict::new(py);
    let ext = std::path::Path::new(&source)
        .extension()
        .and_then(|v| v.to_str())
        .map(fold);
    if matches!(
        ext.as_deref(),
        Some("png" | "jpg" | "jpeg" | "bmp" | "gif" | "webp" | "dds")
    ) {
        let view = py.import("copy")?.call_method1("copy", (package,))?;
        view.setattr("prefix", "")?;
        let pixels = py
            .import("creation_lib.mod_manager.installer_preview")?
            .call_method1("image", (view, &source, cancel))?;
        package.call_method1("validate_sources", ((&source,), cancel))?;
        kwargs.set_item("pixels", pixels)?;
        return Ok(preview
            .call((&source, size, "image"), Some(&kwargs))?
            .unbind());
    }
    if size > 8 * 1024 * 1024 {
        return Err(invalid(
            "Inert previews accept members up to 8 MiB; larger files remain unopened.",
        ));
    }
    check_cancel(Some(cancel))?;
    let data: Vec<u8> = with_context(&package.call_method1("open", (&source,))?, |stream| {
        stream.call_method1("read", (65536,))?.extract()
    })?;
    check_cancel(Some(cancel))?;
    package.call_method1("validate_sources", ((&source,), cancel))?;
    preview_text(py, &source, size, &data)
}
pub fn preview_text(py: Python<'_>, source: &str, size: usize, data: &[u8]) -> PyResult<Py<PyAny>> {
    let preview = py
        .import("creation_lib.mod_manager.package_preview")?
        .getattr("Preview")?;
    let kwargs = pyo3::types::PyDict::new(py);
    let encoding = if data.starts_with(&[0xff, 0xfe]) || data.starts_with(&[0xfe, 0xff]) {
        "utf-16"
    } else {
        "utf-8-sig"
    };
    let decoded = (|| -> PyResult<String> {
        let decoder = py
            .import("codecs")?
            .call_method1("getincrementaldecoder", (encoding,))?
            .call1(("strict",))?;
        decoder
            .call_method1("decode", (PyBytes::new(py, &data), size <= data.len()))?
            .extract()
    })();
    let text = match decoded {
        Ok(text) if !text.chars().any(|c| c < ' ' && !"\t\n\r".contains(c)) => text,
        Ok(_) => String::new(),
        Err(e) if e.is_instance_of::<pyo3::exceptions::PyUnicodeError>(py) => String::new(),
        Err(e) => return Err(e),
    };
    let kind;
    if !text.is_empty() || data.is_empty() {
        kind = "text";
        kwargs.set_item("text", text.chars().take(32768).collect::<String>())?;
        kwargs.set_item(
            "truncated",
            size > data.len() || text.chars().count() > 32768,
        )?;
    } else {
        kind = "hex";
        let lines: Vec<_> = data[..data.len().min(4096)]
            .chunks(16)
            .enumerate()
            .map(|(i, chunk)| {
                format!(
                    "{:08x}  {}",
                    i * 16,
                    chunk
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect::<Vec<_>>()
                        .join(" ")
                )
            })
            .collect();
        kwargs.set_item("text", lines.join("\n"))?;
        kwargs.set_item("truncated", size > 4096)?;
    }
    Ok(preview.call((&source, size, kind), Some(&kwargs))?.unbind())
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(package_preview, m)?)?;
    m.add_function(wrap_pyfunction!(installer_preview_metadata, m)?)?;
    m.add_function(wrap_pyfunction!(installer_preview_image, m)?)?;
    Ok(())
}
