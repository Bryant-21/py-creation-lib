use super::*;

pub(crate) fn import_plugin_text_native_impl(
    py: Python<'_>,
    source_path: &str,
    output_path: &str,
    game: Option<&str>,
    format: Option<&str>,
) -> PyResult<()> {
    let source = Path::new(source_path);
    let text = fs::read_to_string(source).map_err(|err| {
        io_error(format!(
            "failed to read import source '{}': {err}",
            source.display()
        ))
    })?;
    let resolved_format = detect_text_format(source_path, format);
    let lossless = {
        let text = text.clone();
        let resolved_format = resolved_format.clone();
        py.detach(move || {
            import_text_payload_lossless_native(text.as_str(), resolved_format.as_str())
        })?
    };
    if let Some((mut parsed, strings)) = lossless {
        if let Some(game_id) = game {
            parsed.game = Some(game_id.to_string());
        }
        let output_path = output_path.to_string();
        return py.detach(move || save_parsed_plugin(&mut parsed, &strings, output_path.as_str()));
    }
    let compact_result = {
        let text = text.clone();
        let resolved_format = resolved_format.clone();
        py.detach(move || {
            import_text_payload_compact_native(text.as_str(), resolved_format.as_str())
        })?
    };
    let (mut parsed, strings) = compact_result;
    if let Some(game_id) = game {
        parsed.game = Some(game_id.to_string());
    }
    let output_path = output_path.to_string();
    py.detach(move || save_parsed_plugin(&mut parsed, &strings, output_path.as_str()))
}
