use super::*;

pub(crate) fn export_plugin_text_native_impl(
    py: Python<'_>,
    plugin_path: &str,
    output_path: &str,
    game: Option<&str>,
    mode: &str,
    format: &str,
) -> PyResult<()> {
    let path = plugin_path.to_string();
    let game_owned = game.map(str::to_string);
    let mode = mode.to_string();
    let format = format.to_string();
    let output_path = output_path.to_string();
    py.detach(move || {
        let parsed = parse_plugin_file(path.as_str(), game_owned, true)?;
        let plugin_name = parsed.plugin_name.clone();
        let strings = if (parsed.header.flags & TES4_FLAG_LOCALIZED) != 0 {
            crate::plugin_runtime::strings::hydrate_strings_state(
                path.as_str(),
                &plugin_name,
                None,
                None,
            )
        } else {
            LocalizedStringsState::default()
        };
        let text = dump_text_payload_value(
            export_text_payload_value_from_parsed(&parsed, &strings, mode.as_str())?,
            format.as_str(),
        )?;
        let output = Path::new(output_path.as_str());
        if let Some(parent) = output.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).map_err(|err| {
                    io_error(format!(
                        "failed to create export directory '{}': {err}",
                        parent.display()
                    ))
                })?;
            }
        }
        fs::write(output, text).map_err(|err| {
            io_error(format!(
                "failed to write export '{}': {err}",
                output.display()
            ))
        })
    })
}

pub(crate) fn export_authoring_dir_native_impl(
    py: Python<'_>,
    plugin_path: &str,
    out_dir: &str,
    game: Option<&str>,
    format: &str,
    jobs: Option<usize>,
    skip_signatures: Option<Vec<String>>,
) -> PyResult<()> {
    let path = plugin_path.to_string();
    let game_owned = game.map(str::to_string);
    let (parsed, strings) = py.detach(move || {
        // inspect-mod / esp export need fully decoded subrecord arrays so the
        // YAML output has no `raw_payload_hex` fallback. Per project memory
        // "localized strings load-all is the default", the central
        // parse_plugin_file_lazy_compressed path stays lazy — only this
        // export-side call flips to eager.
        let parsed = parse_plugin_file_eager_compressed(path.as_str(), game_owned)?;
        let plugin_name = parsed.plugin_name.clone();
        let strings = if (parsed.header.flags & TES4_FLAG_LOCALIZED) != 0 {
            crate::plugin_runtime::strings::hydrate_strings_state(
                path.as_str(),
                &plugin_name,
                None,
                None,
            )
        } else {
            LocalizedStringsState::default()
        };
        Ok::<_, PyErr>((parsed, strings))
    })?;
    let record_count = count_records(&parsed.root_items);
    let fmt = match format.trim().to_ascii_lowercase().as_str() {
        "json" => "json".to_string(),
        "yaml" | "yml" => "yaml".to_string(),
        _ => {
            return Err(value_error(format!(
                "unsupported authoring-dir format: {format:?} (expected json or yaml)"
            )));
        }
    };
    let skip_set: std::collections::HashSet<String> =
        skip_signatures.unwrap_or_default().into_iter().collect();
    let jobs = jobs.unwrap_or(crate::default_job_count());
    if jobs > 1 {
        export_authoring_dir_parallel(
            py,
            &parsed,
            &strings,
            record_count,
            Path::new(out_dir),
            fmt.as_str(),
            jobs,
            &skip_set,
        )
    } else {
        export_authoring_dir_from_parsed(
            py,
            &parsed,
            &strings,
            record_count,
            Path::new(out_dir),
            fmt.as_str(),
            &skip_set,
        )
    }
}
