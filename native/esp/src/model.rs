use super::*;

#[derive(Clone)]
pub struct ParsedSubrecord {
    pub signature: SmolStr,
    /// Refcounted slice. When the parser is reading a real plugin file the
    /// underlying buffer is the source mmap (or the source `Vec<u8>`),
    /// so cloning a `ParsedSubrecord` is a refcount bump rather than a
    /// per-byte copy. For records authored at runtime (tests, imports
    /// from JSON/YAML) this is constructed from an owned `Vec<u8>` via
    /// `Bytes::from(vec)`.
    pub data: Bytes,
    pub semantic_type: Option<String>,
}

#[derive(Clone)]
pub struct ParsedRecord {
    pub signature: SmolStr,
    pub form_id: u32,
    pub flags: u32,
    pub version_control: u32,
    pub form_version: Option<u16>,
    pub version2: Option<u16>,
    pub subrecords: Vec<ParsedSubrecord>,
    /// Stored compressed payload for byte-exact roundtrip. As with
    /// `ParsedSubrecord::data` this is normally a refcount slice into
    /// the source mmap rather than an owned copy.
    pub raw_payload: Option<Bytes>,
    pub parse_error: Option<String>,
}

#[derive(Clone)]
pub struct ParsedGroup {
    pub label: [u8; 4],
    pub group_type: i32,
    pub tail: Bytes,
    pub children: Vec<ParsedItem>,
}

#[derive(Clone)]
pub enum ParsedItem {
    Group(ParsedGroup),
    Record(ParsedRecord),
}

#[derive(Clone)]
pub struct ParsedPluginHeader {
    pub version: f32,
    pub num_records: u32,
    pub next_object_id: u32,
    pub author: String,
    pub description: String,
    pub masters: Vec<String>,
    pub master_sizes: Vec<u64>,
    pub overridden_forms: Vec<u32>,
    pub flags: u32,
    pub extra_subrecords: Vec<ParsedSubrecord>,
    pub version_control: u32,
    pub form_version: Option<u16>,
    pub version2: Option<u16>,
    pub hedr_raw: Option<Bytes>,
    pub raw_subrecords: Vec<ParsedSubrecord>,
}

#[cfg(test)]
impl ParsedPluginHeader {
    pub fn default_for_test() -> Self {
        Self {
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
        }
    }
}

#[derive(Clone)]
pub struct ParsedPlugin {
    pub plugin_name: String,
    pub file_path: String,
    pub header_size: usize,
    pub header: ParsedPluginHeader,
    pub root_items: Vec<ParsedItem>,
    pub game: Option<String>,
}

#[derive(Default, Clone)]
pub struct LocalizedStringsState {
    pub by_language: HashMap<String, HashMap<u32, String>>,
    pub default_language: String,
    pub table_types: HashMap<u32, String>,
    pub is_filtered: bool,
    pub requested_language: Option<String>,
    pub source_strings_dir: Option<String>,
    /// Indexed-but-undecoded tables, for handles opened read-only.
    ///
    /// `by_language` stays authoritative: anything materialized or authored
    /// there wins. This is consulted only for ids it does not hold, so a caller
    /// that resolves single ids never pays to decode the whole corpus. Callers
    /// that enumerate or rewrite the corpus must call
    /// [`LocalizedStringsState::materialize_all`] first.
    pub lazy_tables: Option<Arc<crate::plugin_runtime::strings::LazyStringTables>>,
}

impl LocalizedStringsState {
    /// Every language code this state can answer for, decoded or not.
    pub fn language_codes(&self) -> Vec<String> {
        let mut codes: Vec<String> = self.by_language.keys().cloned().collect();
        if let Some(lazy) = self.lazy_tables.as_ref() {
            for language in lazy.languages() {
                if !codes.contains(&language) {
                    codes.push(language);
                }
            }
        }
        codes.sort();
        codes
    }

    /// Text for `string_id` in `language`, falling through to the lazy tables.
    pub fn resolve(&self, language: &str, string_id: u32) -> Option<String> {
        if let Some(text) = self
            .by_language
            .get(language)
            .and_then(|table| table.get(&string_id))
        {
            return Some(text.clone());
        }
        self.lazy_tables
            .as_ref()
            .and_then(|lazy| lazy.get(language, string_id))
    }

    /// Decode every lazy table into `by_language` and drop the lazy source.
    ///
    /// Required before enumerating `by_language` or writing tables back out;
    /// a lazily indexed state looks empty to code that reads the map directly.
    pub fn materialize_all(&mut self) {
        let Some(lazy) = self.lazy_tables.take() else {
            return;
        };
        let (by_language, table_types) = lazy.decode_all();
        for (language, table) in by_language {
            let target = self.by_language.entry(language).or_default();
            for (string_id, text) in table {
                target.entry(string_id).or_insert(text);
            }
        }
        for (string_id, table_type) in table_types {
            self.table_types.entry(string_id).or_insert(table_type);
        }
    }
}

#[derive(Clone)]
pub struct NativeImportContext {
    pub plugin_name: String,
    pub game: Option<String>,
    pub schema: Option<Arc<CompiledSchema>>,
    pub header_size: usize,
    pub header: ParsedPluginHeader,
    pub strings: LocalizedStringsState,
    next_localized_string_id: u32,
    pub allocated_localized_string_ids: Vec<u32>,
    /// Signature of the record currently being built, so a localized field can
    /// be filed under the right string table. The streaming build writes records
    /// straight to disk and hands `write_localized_strings_for_parsed` a record-less
    /// plugin shell, so `infer_localized_table_types` has nothing to walk and every
    /// id would otherwise default to `strings` -- putting COBJ/BOOK/PERK/RACE
    /// descriptions in .STRINGS, where FO4 never looks for them.
    pub current_record_signature: Option<String>,
}

impl NativeImportContext {
    pub fn new(
        plugin_name: String,
        game: Option<String>,
        header_size: usize,
        header: ParsedPluginHeader,
    ) -> Self {
        let schema = game
            .as_deref()
            .and_then(|game| compiled_schema_for_game(game).ok());
        Self {
            plugin_name,
            game,
            schema,
            header_size,
            header,
            strings: LocalizedStringsState {
                default_language: "en".to_string(),
                ..LocalizedStringsState::default()
            },
            next_localized_string_id: 1,
            allocated_localized_string_ids: Vec::new(),
            current_record_signature: None,
        }
    }

    pub fn own_index(&self) -> u32 {
        self.header.masters.len() as u32
    }

    pub fn ensure_master_index(&mut self, plugin_name: &str) -> usize {
        for (index, item) in self.header.masters.iter().enumerate() {
            if item.eq_ignore_ascii_case(plugin_name) {
                return index;
            }
        }
        self.header.masters.push(plugin_name.to_string());
        self.header.master_sizes.push(0);
        self.header.masters.len() - 1
    }

    pub fn refresh_next_localized_string_id(&mut self, preferred_start: u32) {
        self.next_localized_string_id = next_localized_string_id(&self.strings, preferred_start);
    }

    pub fn set_next_localized_string_id(&mut self, next_id: u32) {
        self.next_localized_string_id = next_id;
    }

    pub fn next_localized_string_id(&self) -> u32 {
        self.next_localized_string_id
    }

    pub fn allocate_localized_string_id(&mut self, preferred_start: u32) -> u32 {
        if self.next_localized_string_id < preferred_start {
            self.next_localized_string_id = preferred_start;
        }
        while self
            .strings
            .by_language
            .values()
            .any(|mapping| mapping.contains_key(&self.next_localized_string_id))
        {
            self.next_localized_string_id = self.next_localized_string_id.saturating_add(1);
        }
        let allocated = self.next_localized_string_id;
        self.next_localized_string_id = self.next_localized_string_id.saturating_add(1);
        self.allocated_localized_string_ids.push(allocated);
        allocated
    }

    pub fn note_localized_string_id(&mut self, string_id: u32) {
        if string_id >= self.next_localized_string_id {
            self.next_localized_string_id = string_id.saturating_add(1);
        }
    }

    pub fn set_localized_field_values(
        &mut self,
        string_id: u32,
        values_by_language: &HashMap<String, String>,
        preferred_language: Option<&str>,
        table_type: Option<&str>,
    ) {
        for (language, text) in values_by_language {
            self.strings
                .by_language
                .entry(normalize_language_key(language.as_str()))
                .or_default()
                .insert(string_id, text.clone());
        }
        self.note_localized_string_id(string_id);
        let preferred = normalize_language_key(
            preferred_language
                .filter(|value| !value.trim().is_empty())
                .unwrap_or("en"),
        );
        self.strings.default_language = preferred;
        if let Some(table_type) = table_type {
            self.strings
                .table_types
                .insert(string_id, table_type.to_string());
        }
    }

    pub fn merge_localized_strings_from(&mut self, other: &LocalizedStringsState) -> PyResult<()> {
        let mut max_seen = 0u32;
        for (language, table) in &other.by_language {
            let target = self
                .strings
                .by_language
                .entry(language.clone())
                .or_default();
            for (string_id, text) in table {
                if let Some(existing) = target.get(string_id) {
                    if existing != text {
                        return Err(value_error(format!(
                            "localized string ID collision for {string_id:08X} in {language}: \
                             existing={existing:?} new={text:?}"
                        )));
                    }
                }
                target.insert(*string_id, text.clone());
                max_seen = max_seen.max(*string_id);
            }
        }
        for (string_id, table_type) in &other.table_types {
            self.strings
                .table_types
                .entry(*string_id)
                .or_insert_with(|| table_type.clone());
            max_seen = max_seen.max(*string_id);
        }
        if !other.default_language.trim().is_empty() {
            self.strings.default_language = other.default_language.clone();
        }
        if max_seen != 0 {
            self.note_localized_string_id(max_seen);
        }
        Ok(())
    }
}

fn next_localized_string_id(strings: &LocalizedStringsState, preferred_start: u32) -> u32 {
    let mut highest = 0u32;
    for mapping in strings.by_language.values() {
        if let Some(candidate) = mapping.keys().max().copied() {
            highest = highest.max(candidate);
        }
    }
    let mut candidate = highest.saturating_add(1).max(preferred_start);
    while strings
        .by_language
        .values()
        .any(|mapping| mapping.contains_key(&candidate))
    {
        candidate = candidate.saturating_add(1);
    }
    candidate
}

pub fn json_object<'a>(
    value: &'a JsonValue,
    field: &str,
) -> PyResult<&'a JsonMap<String, JsonValue>> {
    value
        .as_object()
        .ok_or_else(|| value_error(format!("{field} must be an object")))
}

pub fn json_array<'a>(value: &'a JsonValue, field: &str) -> PyResult<&'a Vec<JsonValue>> {
    value
        .as_array()
        .ok_or_else(|| value_error(format!("{field} must be a list")))
}

pub fn json_optional_string(value: Option<&JsonValue>) -> PyResult<Option<String>> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    value
        .as_str()
        .map(|value| Some(value.to_string()))
        .ok_or_else(|| value_error("value must be a string"))
}

pub fn json_required_string(value: &JsonValue, field: &str) -> PyResult<String> {
    value
        .as_str()
        .map(|value| value.to_string())
        .ok_or_else(|| value_error(format!("{field} must be a string")))
}

pub fn json_parse_int(value: &JsonValue, field: &str, default: i64) -> PyResult<i64> {
    if value.is_null() {
        return Ok(default);
    }
    if let Some(parsed) = value.as_i64() {
        return Ok(parsed);
    }
    if let Some(parsed) = value.as_u64() {
        return Ok(parsed as i64);
    }
    if let Some(text) = value.as_str() {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Ok(default);
        }
        return trimmed
            .parse::<i64>()
            .map_err(|_| value_error(format!("invalid integer for {field}: {text:?}")));
    }
    Err(value_error(format!("invalid integer for {field}")))
}

pub fn json_parse_float(value: &JsonValue, field: &str, default: f32) -> PyResult<f32> {
    if value.is_null() {
        return Ok(default);
    }
    if let Some(parsed) = value.as_f64() {
        return Ok(parsed as f32);
    }
    if let Some(parsed) = value.as_i64() {
        return Ok(parsed as f32);
    }
    if let Some(parsed) = value.as_u64() {
        return Ok(parsed as f32);
    }
    if let Some(text) = value.as_str() {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Ok(default);
        }
        return trimmed
            .parse::<f32>()
            .map_err(|_| value_error(format!("invalid float for {field}: {text:?}")));
    }
    Err(value_error(format!("invalid float for {field}")))
}

pub fn json_parse_hex(value: &JsonValue, field: &str, default: u32) -> PyResult<u32> {
    if value.is_null() {
        return Ok(default);
    }
    if let Some(parsed) = value.as_u64() {
        return Ok(parsed as u32);
    }
    if let Some(parsed) = value.as_i64() {
        return Ok(parsed as u32);
    }
    if let Some(text) = value.as_str() {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Ok(default);
        }
        let normalized = trimmed.trim_start_matches("0x").trim_start_matches("0X");
        return u32::from_str_radix(normalized, 16)
            .map_err(|_| value_error(format!("invalid hex integer for {field}: {text:?}")));
    }
    Err(value_error(format!("invalid hex integer for {field}")))
}

pub fn json_parse_hex_bytes(value: Option<&JsonValue>, field: &str) -> PyResult<Vec<u8>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    if value.is_null() {
        return Ok(Vec::new());
    }
    let text = json_required_string(value, field)?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    hex::decode(trimmed)
        .map_err(|_| value_error(format!("invalid hex string for {field}: {text:?}")))
}

fn header_flag_bit_for_text(text: &str) -> Option<u32> {
    let trimmed = text.trim();
    HEADER_FLAG_DEFINITIONS
        .iter()
        .find_map(|(token, bit, label)| {
            if token.eq_ignore_ascii_case(trimmed)
                || label.eq_ignore_ascii_case(trimmed)
                || authoring_camel_case(label).eq_ignore_ascii_case(trimmed)
                || (*token == "master" && trimmed.eq_ignore_ascii_case("Master"))
            {
                Some(*bit)
            } else {
                None
            }
        })
}

fn header_flag_bit_for_value(value: &JsonValue, field: &str) -> PyResult<u32> {
    if let Some(text) = value.as_str() {
        if let Some(bit) = header_flag_bit_for_text(text) {
            return Ok(bit);
        }
        let trimmed = text
            .trim()
            .trim_start_matches("0x")
            .trim_start_matches("0X");
        return u32::from_str_radix(trimmed, 16)
            .ok()
            .filter(|n| HEADER_FLAG_DEFINITIONS.iter().any(|d| d.1 == *n))
            .ok_or_else(|| value_error(format!("unknown header.flags value: {text:?}")));
    }
    let n = json_parse_hex(value, field, 0)?;
    if !HEADER_FLAG_DEFINITIONS.iter().any(|d| d.1 == n) {
        return Err(value_error(format!("unknown header.flags value: {n}")));
    }
    Ok(n)
}

pub fn json_parse_header_flags(value: Option<&JsonValue>) -> PyResult<u32> {
    let Some(value) = value else {
        return Ok(0);
    };
    if value.is_null() {
        return Ok(0);
    }
    if let Some(items) = value.as_array() {
        let mut wrapper = JsonMap::new();
        wrapper.insert("flags".to_string(), JsonValue::Array(items.clone()));
        return json_parse_header_flags(Some(&JsonValue::Object(wrapper)));
    }
    if let Some(mapping) = value.as_object() {
        let has_raw = mapping.contains_key("raw");
        let has_flags = mapping.contains_key("flags");
        if !has_raw && !has_flags {
            return Err(value_error(
                "header.flags object must contain 'raw' or 'flags'",
            ));
        }

        // raw must be a hex string, not a plain integer
        let raw_opt = if has_raw {
            let raw_val = mapping.get("raw").unwrap();
            let text = raw_val
                .as_str()
                .ok_or_else(|| value_error("header.flags raw must be a hex string"))?;
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return Err(value_error(
                    "header.flags raw must be a non-empty hex string",
                ));
            }
            let normalized = trimmed.trim_start_matches("0x").trim_start_matches("0X");
            let n = u32::from_str_radix(normalized, 16).map_err(|_| {
                value_error(format!("invalid hex string for header.flags raw: {text:?}"))
            })?;
            Some(n)
        } else {
            None
        };

        let mut enabled_bits = 0u32;
        let mut seen_bits = 0u32;

        if let Some(flags) = mapping.get("flags") {
            for entry in json_array(flags, "header.flags.flags")? {
                if !entry.is_object() {
                    let bit = header_flag_bit_for_value(entry, "header.flags.flags[]")?;
                    if seen_bits & bit != 0 {
                        return Err(value_error("duplicate header.flags bits"));
                    }
                    seen_bits |= bit;
                    enabled_bits |= bit;
                    continue;
                }

                let entry = json_object(entry, "header.flags.flags[]")?;

                let enabled = match entry.get("enabled") {
                    None => true,
                    Some(v) => v
                        .as_bool()
                        .ok_or_else(|| value_error("header.flags[].enabled must be a boolean"))?,
                };

                let token_str = entry
                    .get("token")
                    .and_then(|v| v.as_str())
                    .or_else(|| entry.get("id").and_then(|v| v.as_str()));

                let token_bit = token_str
                    .map(|tok| {
                        header_flag_bit_for_text(tok).ok_or_else(|| {
                            value_error(format!("unknown header.flags token: {tok:?}"))
                        })
                    })
                    .transpose()?;

                // If no token, check label doesn't silently act as token identity
                if token_str.is_none() {
                    if let Some(label) = entry.get("label").and_then(|v| v.as_str()) {
                        if HEADER_FLAG_DEFINITIONS
                            .iter()
                            .any(|d| d.0.eq_ignore_ascii_case(label))
                        {
                            return Err(value_error(
                                "header.flags label matches a known token — use 'token' instead",
                            ));
                        }
                    }
                }

                let value_bit = entry
                    .get("value")
                    .map(|v| header_flag_bit_for_value(v, "header.flags[].value"))
                    .transpose()?;

                let bit = match (token_bit, value_bit) {
                    (None, None) => {
                        return Err(value_error(
                            "header.flags entry must have 'token' or 'value'",
                        ));
                    }
                    (Some(t), None) => t,
                    (None, Some(v)) => v,
                    (Some(t), Some(v)) => {
                        if t != v {
                            return Err(value_error(format!(
                                "header.flags token ({t:#010x}) does not match value ({v:#010x})"
                            )));
                        }
                        t
                    }
                };

                if seen_bits & bit != 0 {
                    return Err(value_error("duplicate header.flags bits"));
                }
                seen_bits |= bit;

                if enabled {
                    enabled_bits |= bit;
                }
            }
        }

        if let Some(raw) = raw_opt {
            if has_flags && raw != enabled_bits {
                return Err(value_error(
                    "header.flags raw value does not match enabled flags sum",
                ));
            } else if !has_flags {
                return Ok(raw);
            }
        }

        return Ok(enabled_bits);
    }
    // Top-level scalar: reject blank strings
    if let Some(s) = value.as_str() {
        if s.trim().is_empty() {
            return Err(value_error("header.flags cannot be blank"));
        }
    }
    json_parse_hex(value, "header.flags", 0)
}

pub fn parse_text_payload_value_native(text: &str, format: &str) -> PyResult<JsonValue> {
    if format.eq_ignore_ascii_case("yaml") || format.eq_ignore_ascii_case("yml") {
        // Inputs are produced by our own exporter on local Bethesda ESMs.
        // Starfield.esm yields YAML with tens of millions of nodes, which trips
        // serde-saphyr's default anti-amplification budget (max_nodes=250_000,
        // max_events=1_000_000, max_total_scalar_bytes=64 MiB).
        let options = serde_saphyr::options! {
            budget: None,
        };
        return serde_saphyr::from_str_with_options::<JsonValue>(text, options)
            .map_err(|err| value_error(format!("invalid yaml payload: {err}")));
    }
    serde_json::from_str::<JsonValue>(text)
        .map_err(|err| value_error(format!("invalid json payload: {err}")))
}
