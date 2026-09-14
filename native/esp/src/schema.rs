use super::*;

#[derive(Clone, Deserialize)]
pub struct SchemaEnumValueJson {
    pub value: i128,
    pub id: String,
}

#[derive(Clone, Deserialize)]
pub struct SchemaEnumLabelJson {
    pub value: i128,
    pub label: String,
}

#[derive(Clone, Deserialize)]
pub struct SchemaEnumAliasJson {
    pub legacy_id: String,
    pub id: String,
}

#[derive(Clone, Default, Deserialize)]
pub struct SchemaEnumJson {
    pub id: String,
    #[serde(default)]
    pub values: Vec<SchemaEnumValueJson>,
    #[serde(default)]
    pub labels: Vec<SchemaEnumLabelJson>,
    #[serde(default)]
    pub aliases: Vec<SchemaEnumAliasJson>,
    #[serde(default = "default_enum_scope")]
    pub scope: String,
    #[serde(default = "default_enum_storage_kind")]
    pub storage_kind: String,
    #[serde(default = "default_enum_byte_width")]
    pub byte_width: usize,
    /// FO4 fallback value for out-of-range clamp (e.g. KYWD.TNAM -> 0 "None").
    #[serde(default)]
    pub default_value: Option<i128>,
}

fn default_enum_scope() -> String {
    "scoped".to_string()
}

fn default_enum_storage_kind() -> String {
    "enum".to_string()
}

fn default_enum_byte_width() -> usize {
    4
}

impl SchemaEnumJson {
    pub fn is_bool_enum(&self) -> bool {
        self.id == "bool_enum"
    }

    pub fn is_flags(&self) -> bool {
        self.storage_kind == "flags"
    }

    /// OR of every defined flag-bit value. Only meaningful when `is_flags()`.
    /// Class A masking uses this to clear FO76-only bits: `raw & valid_flag_mask`.
    pub fn valid_flag_mask(&self) -> i128 {
        self.values
            .iter()
            .fold(0i128, |acc, entry| acc | entry.value)
    }

    /// True when `value` is one of the enum's defined values. Used by Class A
    /// value-enum clamping to detect out-of-range source values.
    pub fn contains_value(&self, value: i128) -> bool {
        self.values.iter().any(|entry| entry.value == value)
    }

    /// FO4 fallback value to clamp an out-of-range value to (0 "None" for the
    /// in-scope FO4 value enums). None for flag enums.
    pub fn fallback_value(&self) -> Option<i128> {
        self.default_value
    }

    pub fn token_for_value(&self, value: i128) -> Option<&str> {
        self.values
            .iter()
            .find(|entry| entry.value == value)
            .map(|entry| entry.id.as_str())
    }

    pub fn label_for_value(&self, value: i128) -> Option<&str> {
        self.labels
            .iter()
            .find(|entry| entry.value == value)
            .map(|entry| entry.label.as_str())
    }

    pub fn display_for_value(&self, value: i128) -> Option<&str> {
        self.label_for_value(value)
            .or_else(|| self.token_for_value(value))
    }

    pub fn value_for_token_or_label(&self, text: &str) -> Option<i128> {
        let trimmed = text.trim();
        let canonical = self
            .aliases
            .iter()
            .find(|alias| alias.legacy_id == trimmed)
            .map(|alias| alias.id.as_str())
            .unwrap_or(trimmed);
        self.values
            .iter()
            .find(|entry| entry.id == canonical)
            .map(|entry| entry.value)
            .or_else(|| {
                self.labels
                    .iter()
                    .find(|entry| entry.label == canonical)
                    .map(|entry| entry.value)
            })
            .or_else(|| {
                let lower = canonical.to_ascii_lowercase();
                self.values
                    .iter()
                    .find(|entry| entry.id.to_ascii_lowercase() == lower)
                    .map(|entry| entry.value)
                    .or_else(|| {
                        self.labels
                            .iter()
                            .find(|entry| entry.label.to_ascii_lowercase() == lower)
                            .map(|entry| entry.value)
                    })
                    .or_else(|| {
                        self.values
                            .iter()
                            .find(|entry| {
                                authoring_camel_case(entry.id.as_str()).to_ascii_lowercase()
                                    == lower
                            })
                            .map(|entry| entry.value)
                    })
                    .or_else(|| {
                        self.labels
                            .iter()
                            .find(|entry| {
                                authoring_camel_case(entry.label.as_str()).to_ascii_lowercase()
                                    == lower
                            })
                            .map(|entry| entry.value)
                    })
            })
            .or_else(|| {
                if self.is_flags() {
                    unknown_flag_label_value(canonical)
                } else {
                    None
                }
            })
    }
}

fn unknown_flag_label_value(text: &str) -> Option<i128> {
    let trimmed = text.trim();
    let normalized = trimmed.to_ascii_lowercase();
    let suffix = normalized
        .strip_prefix("unknown ")
        .or_else(|| normalized.strip_prefix("unknown_"))
        .or_else(|| normalized.strip_prefix("unknown"))?;
    let bit = suffix.parse::<u32>().ok()?;
    if bit >= 127 {
        return None;
    }
    Some(1i128 << bit)
}

pub fn authoring_camel_case(text: &str) -> String {
    let mut out = String::new();
    let mut capitalize_next = true;
    for ch in text.chars() {
        if ch == '\'' || ch == '`' || ch == '\u{2019}' {
            continue;
        }
        if ch.is_ascii_alphanumeric() {
            if capitalize_next && ch.is_ascii_alphabetic() {
                out.push(ch.to_ascii_uppercase());
            } else {
                out.push(ch);
            }
            capitalize_next = false;
        } else {
            capitalize_next = true;
        }
    }
    out
}

pub fn authoring_key_name(label: Option<&str>, id: &str) -> String {
    let source = label.unwrap_or(id);
    let key = authoring_camel_case(source);
    if key.is_empty() { id.to_string() } else { key }
}

#[derive(Clone, Deserialize)]
pub struct SchemaConditionJson {
    pub field: String,
    #[serde(default = "default_condition_operator")]
    pub operator: String,
    #[serde(default)]
    pub value: Option<serde_json::Value>,
    #[serde(default)]
    pub values: Vec<serde_json::Value>,
}

fn default_condition_operator() -> String {
    "eq".to_string()
}

#[derive(Clone, Deserialize)]
pub struct SchemaArrayJson {
    #[serde(rename = "layout")]
    pub _layout: String,
    #[serde(default)]
    pub element_codec: Option<String>,
    #[serde(default)]
    pub count_field: Option<String>,
    #[serde(default)]
    pub count_codec: Option<String>,
    #[serde(default)]
    pub count_transform: Option<String>,
    // Cross-subrecord count: pull the row count from the record-level context
    // (populated by prior subrecords) under this field name. Used for xEdit's
    // `SetCountPath('..\<SIG>\<Field>')` pattern (e.g. FSTS.DATA arrays sized
    // by FSTS.XCNT counts).
    #[serde(default)]
    pub count_record_field: Option<String>,
}

#[derive(Clone, Default, Deserialize)]
pub struct SchemaFieldJson {
    pub id: String,
    pub kind: String,
    #[serde(default)]
    pub display_label: Option<String>,
    #[serde(default)]
    pub enum_ref: Option<String>,
    #[serde(default)]
    pub formlink_target: Option<String>,
    /// Full ordered allowed-target set from xEdit (NULL stripped out, recorded
    /// in `null_allowed`). Empty ⇒ no FK constraint emitted.
    #[serde(default)]
    pub formlink_targets: Vec<String>,
    #[serde(default)]
    pub null_allowed: bool,
    #[serde(default)]
    pub union_variants: Vec<SchemaUnionVariantJson>,
    #[serde(default)]
    pub array: Option<SchemaArrayJson>,
    #[serde(default)]
    pub fields: Vec<SchemaFieldJson>,
    #[serde(default)]
    pub default_value: Option<serde_json::Value>,
    #[serde(default)]
    pub presence_conditions: Vec<SchemaConditionJson>,
}

impl SchemaFieldJson {
    pub fn formlink_targets(&self) -> &[String] {
        &self.formlink_targets
    }
    pub fn null_allowed(&self) -> bool {
        self.null_allowed
    }
    /// True when `target_sig` ("NULL" for a zero formid) is acceptable here.
    /// Empty target set ⇒ unconstrained (always true).
    pub fn allows_target(&self, target_sig: &str) -> bool {
        ref_target_allowed(&self.formlink_targets, self.null_allowed, target_sig)
    }
}

fn ref_target_allowed(targets: &[String], null_allowed: bool, target_sig: &str) -> bool {
    if targets.is_empty() {
        return true;
    }
    if target_sig.eq_ignore_ascii_case("NULL") {
        return null_allowed;
    }
    targets.iter().any(|t| t == target_sig)
}

#[derive(Clone, Default, Deserialize)]
pub struct SchemaUnionVariantJson {
    pub id: String,
    #[serde(default)]
    pub codec: Option<String>,
    #[serde(default)]
    pub enum_ref: Option<String>,
    #[serde(default)]
    pub fields: Vec<SchemaFieldJson>,
    #[serde(default)]
    pub conditions: Vec<SchemaConditionJson>,
}

#[derive(Clone, Default, Deserialize)]
pub struct SchemaSubrecordJson {
    pub id: String,
    pub kind: String,
    #[serde(default)]
    pub display_label: Option<String>,
    #[serde(default)]
    pub codec: Option<String>,
    #[serde(default)]
    pub fields: Vec<SchemaFieldJson>,
    #[serde(default)]
    pub repeatable: bool,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub localized: bool,
    #[serde(default)]
    pub enum_ref: Option<String>,
    #[serde(default)]
    pub formlink_target: Option<String>,
    #[serde(default)]
    pub formlink_targets: Vec<String>,
    #[serde(default)]
    pub null_allowed: bool,
    #[serde(default)]
    pub union_selector: Option<String>,
    #[serde(default)]
    pub union_variants: Vec<SchemaUnionVariantJson>,
    #[serde(default)]
    #[serde(rename = "array")]
    pub _array: Option<SchemaArrayJson>,
    #[serde(default)]
    pub row_label: Option<String>,
    #[serde(default)]
    pub authoring_layout: Option<String>,
    #[serde(default)]
    pub authoring_key: Option<String>,
    #[serde(default)]
    pub scope_id: Option<String>,
}

impl SchemaSubrecordJson {
    pub fn formlink_targets(&self) -> &[String] {
        &self.formlink_targets
    }
    pub fn null_allowed(&self) -> bool {
        self.null_allowed
    }
    pub fn allows_target(&self, target_sig: &str) -> bool {
        ref_target_allowed(&self.formlink_targets, self.null_allowed, target_sig)
    }
    /// xEdit `.SetRequired` — conv-refs uses this to decide strip-optional vs
    /// leave-for-NULL-where-required when a referenced target is illegal.
    pub fn required(&self) -> bool {
        self.required
    }
}

#[derive(Clone, Deserialize)]
pub struct SchemaRecordFlagBitJson {
    pub bit: u8,
    pub name: String,
}

#[derive(Clone, Deserialize, Default)]
pub struct SchemaRecordFlagsJson {
    #[serde(default)]
    pub valid_mask: u32,
    #[serde(default)]
    pub permissive: bool,
    #[serde(default)]
    pub bits: Vec<SchemaRecordFlagBitJson>,
}

impl SchemaRecordFlagsJson {
    pub fn is_permissive(&self) -> bool {
        self.permissive
    }
    /// OR of valid header-flag bit values (incl. universal bits).
    pub fn valid_mask(&self) -> u32 {
        self.valid_mask
    }
    /// Bits set in `raw` that are NOT valid under this record. 0 when permissive.
    pub fn invalid_bits(&self, raw: u32) -> u32 {
        if self.permissive {
            0
        } else {
            raw & !self.valid_mask
        }
    }
    /// `raw` with all unknown header bits cleared. Identity when permissive.
    pub fn strip(&self, raw: u32) -> u32 {
        if self.permissive {
            raw
        } else {
            raw & self.valid_mask
        }
    }
}

#[derive(Clone, Default, Deserialize)]
pub struct SchemaRecordJson {
    pub id: String,
    #[serde(default)]
    pub subrecords: Vec<SchemaSubrecordJson>,
    #[serde(default)]
    pub record_flags: Option<SchemaRecordFlagsJson>,
}

impl SchemaRecordJson {
    /// Record-header flag metadata. None ⇒ not captured for this record;
    /// callers MUST NOT strip header bits in that case (warn-only).
    pub fn record_flags(&self) -> Option<&SchemaRecordFlagsJson> {
        self.record_flags.as_ref()
    }
}

#[derive(Clone, Deserialize)]
pub struct SchemaDocumentJson {
    #[serde(rename = "game")]
    pub _game: String,
    #[serde(default)]
    pub records: Vec<SchemaRecordJson>,
    #[serde(default)]
    pub enums: Vec<SchemaEnumJson>,
}

#[derive(Clone)]
pub struct CompiledSchema {
    pub records: HashMap<String, SchemaRecordJson>,
    pub enums: HashMap<String, SchemaEnumJson>,
}

impl CompiledSchema {
    pub fn record_def(&self, signature: &str) -> Option<&SchemaRecordJson> {
        self.records.get(signature)
    }

    pub fn enum_def(&self, enum_ref: &str) -> Option<&SchemaEnumJson> {
        self.enums.get(enum_ref)
    }

    /// THE shared reference-target accessor. Both the validator
    /// and conv-refs Pass 2 call this — do not re-implement target lookups
    /// elsewhere. `field_path` is either the subrecord sig (FK is the whole
    /// subrecord) or `"<SUB>.<field_id>"` for a field inside a struct codec.
    /// Returns None when the addressed field carries no FK target constraint.
    pub fn allowed_targets(&self, record_sig: &str, field_path: &str) -> Option<RefTargetSpec<'_>> {
        let record = self.records.get(record_sig)?;
        let (sub_sig, field_id) = match field_path.split_once('.') {
            Some((s, f)) => (s, Some(f)),
            None => (field_path, None),
        };
        // First matching subrecord spec for the sig (occurrence 0).
        let sub = record.subrecords.iter().find(|s| s.id == sub_sig)?;
        if let Some(field_id) = field_id {
            let field = sub.fields.iter().find(|f| f.id == field_id)?;
            if field.formlink_targets.is_empty() {
                return None;
            }
            return Some(RefTargetSpec {
                targets: &field.formlink_targets,
                null_allowed: field.null_allowed,
            });
        }
        // Subrecord-level FK (single-field subrecord promoted to sub level).
        if !sub.formlink_targets.is_empty() {
            return Some(RefTargetSpec {
                targets: &sub.formlink_targets,
                null_allowed: sub.null_allowed,
            });
        }
        // Fall back to the sole FK field, if the subrecord has exactly one.
        let fk_fields: Vec<&SchemaFieldJson> = sub
            .fields
            .iter()
            .filter(|f| !f.formlink_targets.is_empty())
            .collect();
        if fk_fields.len() == 1 {
            let field = fk_fields[0];
            return Some(RefTargetSpec {
                targets: &field.formlink_targets,
                null_allowed: field.null_allowed,
            });
        }
        None
    }

    /// THE shared enum-ref locator — the enum analogue of `allowed_targets`.
    /// `field_path` is the subrecord sig (enum is the whole subrecord) or
    /// `"<SUB>.<field_id>"` for a field inside a struct codec. Returns the
    /// `enum_ref` id; resolve it to a `SchemaEnumJson` via `enum_def()`.
    ///
    /// SCOPE-BLIND: matches the FIRST spec for the sig, ignoring `scope_id`.
    /// For scope-overloaded subrecords (`QUST.FNAM` means objective flags in
    /// `scope_id="objectives"` and a completely different 25-bit alias flag set
    /// in `scope_id="aliases"`) that silently returns the wrong enum. Any caller
    /// that MASKS or CLAMPS a value — where a wrong enum destroys real bits —
    /// must use `enum_ref_at_in_scope` with the scope it is currently walking.
    pub fn enum_ref_at(&self, record_sig: &str, field_path: &str) -> Option<&str> {
        self.enum_ref_at_impl(record_sig, field_path, None)
    }

    /// Scope-aware `enum_ref_at`. `scope` is the `scope_id` of the group the
    /// caller is currently inside (`None` = unscoped/top-level subrecords).
    /// Falls back to the scope-blind match when no spec carries that scope, so
    /// a schema without scope annotations behaves as before.
    pub fn enum_ref_at_in_scope(
        &self,
        record_sig: &str,
        field_path: &str,
        scope: Option<&str>,
    ) -> Option<&str> {
        self.enum_ref_at_impl(record_sig, field_path, Some(scope))
    }

    /// `scope`: `None` = don't filter on scope at all; `Some(s)` = prefer specs
    /// whose `scope_id` equals `s`, falling back to unfiltered.
    fn enum_ref_at_impl(
        &self,
        record_sig: &str,
        field_path: &str,
        scope: Option<Option<&str>>,
    ) -> Option<&str> {
        let record = self.records.get(record_sig)?;
        let (sub_sig, field_id) = match field_path.split_once('.') {
            Some((s, f)) => (s, Some(f)),
            None => (field_path, None),
        };
        let scoped = scope.and_then(|scope| {
            record
                .subrecords
                .iter()
                .find(|s| s.id == sub_sig && s.scope_id.as_deref() == scope)
        });
        let sub = match scoped {
            Some(sub) => sub,
            None => record.subrecords.iter().find(|s| s.id == sub_sig)?,
        };
        if let Some(field_id) = field_id {
            let field = sub.fields.iter().find(|f| f.id == field_id)?;
            return field.enum_ref.as_deref();
        }
        // Subrecord-level enum (single-field subrecord promoted to sub level).
        if let Some(enum_ref) = sub.enum_ref.as_deref() {
            return Some(enum_ref);
        }
        // Fall back to the sole enum-bearing field, if exactly one.
        let enum_fields: Vec<&SchemaFieldJson> =
            sub.fields.iter().filter(|f| f.enum_ref.is_some()).collect();
        if enum_fields.len() == 1 {
            return enum_fields[0].enum_ref.as_deref();
        }
        None
    }

    /// Convenience: resolve `enum_ref_at` straight to the enum definition.
    /// Inherits `enum_ref_at`'s scope-blindness — see its doc.
    pub fn enum_def_at(&self, record_sig: &str, field_path: &str) -> Option<&SchemaEnumJson> {
        let enum_ref = self.enum_ref_at(record_sig, field_path)?;
        self.enums.get(enum_ref)
    }

    /// Every distinct `enum_ref` bound to `field_path` across ALL scope
    /// variants of the subrecord, in schema order.
    ///
    /// For the common (non-overloaded) subrecord this is a 1-element vec equal
    /// to `enum_ref_at`. For a scope-overloaded one (QUST.FNAM: objective flags
    /// vs alias flags) it yields both. A WARN-ONLY consumer that cannot see the
    /// scope it is walking must accept a value valid under ANY candidate rather
    /// than false-positive against an arbitrarily-chosen one. A consumer that
    /// MASKS or CLAMPS must instead use `enum_ref_at_in_scope` — the union is
    /// too permissive to strip against.
    pub fn enum_refs_at_all_scopes(&self, record_sig: &str, field_path: &str) -> Vec<&str> {
        let Some(record) = self.records.get(record_sig) else {
            return Vec::new();
        };
        let (sub_sig, field_id) = match field_path.split_once('.') {
            Some((s, f)) => (s, Some(f)),
            None => (field_path, None),
        };
        let mut out: Vec<&str> = Vec::new();
        for sub in record.subrecords.iter().filter(|s| s.id == sub_sig) {
            let enum_ref = match field_id {
                Some(field_id) => sub
                    .fields
                    .iter()
                    .find(|f| f.id == field_id)
                    .and_then(|f| f.enum_ref.as_deref()),
                None => sub.enum_ref.as_deref().or_else(|| {
                    let enum_fields: Vec<&SchemaFieldJson> =
                        sub.fields.iter().filter(|f| f.enum_ref.is_some()).collect();
                    (enum_fields.len() == 1).then(|| enum_fields[0].enum_ref.as_deref())?
                }),
            };
            if let Some(enum_ref) = enum_ref {
                if !out.contains(&enum_ref) {
                    out.push(enum_ref);
                }
            }
        }
        out
    }

    /// Scope-aware `enum_def_at`.
    pub fn enum_def_at_in_scope(
        &self,
        record_sig: &str,
        field_path: &str,
        scope: Option<&str>,
    ) -> Option<&SchemaEnumJson> {
        let enum_ref = self.enum_ref_at_in_scope(record_sig, field_path, scope)?;
        self.enums.get(enum_ref)
    }

    /// THE shared struct-field byte-offset layout. For a subrecord
    /// whose codec is `struct:<TYPETAGS>`, yields one `StructFieldInfo` per
    /// schema field: its dotted `field_path` ("<SUB>.<field_id>"), byte offset,
    /// width, and the field's `enum_ref` / `formlink_targets` / `null_allowed`.
    /// conv-flags (masking), conv-refs (nested FK validation), and the validator
    /// (nested A1/A2/D) all use it, so they agree by construction. A byte-offset
    /// view, not a struct decoder.
    ///
    /// Returns empty when the subrecord isn't a fixed `struct:` codec (variable-
    /// width tokens like zstring abort: offset past such a field is undefined).
    ///
    /// For a `record_form_version` union (e.g. EFSH.DNAM) the active variant, and
    /// so the offsets/widths, depends on form_version. This form picks the variant
    /// for an unknown version (unconditional / first); consumers masking union
    /// subrecords MUST use `struct_field_layout_versioned` with the record's
    /// `form_version`.
    pub fn struct_field_layout(
        &self,
        record_sig: &str,
        subrecord_sig: &str,
    ) -> Vec<StructFieldInfo<'_>> {
        self.struct_field_layout_versioned(record_sig, subrecord_sig, None)
    }

    /// Union-aware struct-field layout. `form_version` selects the active union
    /// variant; `None` falls back to the first variant whose conditions are
    /// satisfied by an absent version (or the lone/unconditional variant).
    pub fn struct_field_layout_versioned(
        &self,
        record_sig: &str,
        subrecord_sig: &str,
        form_version: Option<u16>,
    ) -> Vec<StructFieldInfo<'_>> {
        let Some(record) = self.records.get(record_sig) else {
            return Vec::new();
        };
        let Some(sub) = record.subrecords.iter().find(|s| s.id == subrecord_sig) else {
            return Vec::new();
        };
        // Union subrecord: resolve the active variant, then lay out its codec.
        if !sub.union_variants.is_empty() {
            if let Some(variant) = select_union_variant(&sub.union_variants, form_version) {
                return struct_field_layout_for(
                    &sub.id,
                    variant.codec.as_deref(),
                    &variant.fields,
                    form_version,
                );
            }
            return Vec::new();
        }
        struct_field_layout_for(&sub.id, sub.codec.as_deref(), &sub.fields, form_version)
    }

    /// THE shared flag-field enumerator. Yields every FLAG-storage enum field in
    /// a record as (field_path, &SchemaEnumJson), flattening struct codecs AND
    /// union variants so struct-nested flag bytes (DSTD@3, BOOK.DNAM@0,
    /// LIGH.DATA, EXPL.DATA, RACE.DATA.flags_2, EFSH.DNAM union variants, ...)
    /// are included. conv-flags masks each (raw & enum.valid_flag_mask()) and the
    /// validator checks the SAME set, so they agree by construction. field_path is
    /// the subrecord sig for a subrecord-level flag enum, or "<SUB>.<field_id>"
    /// for a struct/variant sub-field. UNION subrecords yield every variant's flag
    /// fields (deduped by path); use `struct_field_layout_versioned(form_version)`
    /// for the ACTIVE variant's offset/width before masking.
    pub fn iter_flag_fields(&self, record_sig: &str) -> Vec<(String, &SchemaEnumJson)> {
        let Some(record) = self.records.get(record_sig) else {
            return Vec::new();
        };
        // First collect (field_path, enum_ref) candidates (no enum borrow yet),
        // then resolve to flag enums — keeps the borrow checker happy and dedups.
        let mut candidates: Vec<(String, &str)> = Vec::new();
        for sub in &record.subrecords {
            if let Some(eref) = sub.enum_ref.as_deref() {
                candidates.push((sub.id.clone(), eref));
            }
            for field in &sub.fields {
                if let Some(eref) = field.enum_ref.as_deref() {
                    candidates.push((format!("{}.{}", sub.id, field.id), eref));
                }
            }
            // Union variant flag fields (e.g. EFSH.DNAM). All variants surfaced;
            // the consumer resolves the active one by form_version via
            // struct_field_layout_versioned before masking.
            for variant in &sub.union_variants {
                if let Some(eref) = variant.enum_ref.as_deref() {
                    candidates.push((format!("{}.{}", sub.id, variant.id), eref));
                }
                for field in &variant.fields {
                    if let Some(eref) = field.enum_ref.as_deref() {
                        candidates.push((format!("{}.{}", sub.id, field.id), eref));
                    }
                }
            }
        }
        let mut out: Vec<(String, &SchemaEnumJson)> = Vec::new();
        for (path, eref) in candidates {
            if out.iter().any(|(p, _)| p == &path) {
                continue;
            }
            if let Some(e) = self.enums.get(eref) {
                if e.is_flags() {
                    out.push((path, e));
                }
            }
        }
        out
    }
}

/// Select the active union variant for `form_version` by evaluating each
/// variant's `record_form_version` presence conditions. `None` version matches
/// the first unconditional variant (or the first whose conditions trivially
/// pass). Returns the first matching variant.
fn select_union_variant<'a>(
    variants: &'a [SchemaUnionVariantJson],
    form_version: Option<u16>,
) -> Option<&'a SchemaUnionVariantJson> {
    // First, a variant whose conditions all pass for this version.
    for v in variants {
        if v.conditions.is_empty() {
            continue;
        }
        if v.conditions
            .iter()
            .all(|c| condition_passes(c, form_version))
        {
            return Some(v);
        }
    }
    // No conditional match: fall back to an unconditional variant, else the first.
    variants
        .iter()
        .find(|v| v.conditions.is_empty())
        .or_else(|| variants.first())
}

/// Evaluate one `record_form_version` numeric condition. Only the form-version
/// selector with numeric ops is handled (the union shapes in scope); unknown
/// fields/ops conservatively fail so we don't pick a wrong variant.
fn condition_passes(cond: &SchemaConditionJson, form_version: Option<u16>) -> bool {
    if cond.field != "record_form_version" {
        return false;
    }
    let Some(threshold) = cond.value.as_ref().and_then(|v| v.as_i64()) else {
        return false;
    };
    // Absent version: treat as 0 (pre-versioned record) so "lt N" matches the
    // legacy/old-format variant, matching xEdit's default for unversioned data.
    let fv = form_version.map(|v| v as i64).unwrap_or(0);
    match cond.operator.as_str() {
        "lt" => fv < threshold,
        "le" | "lte" => fv <= threshold,
        "gt" => fv > threshold,
        "ge" | "gte" => fv >= threshold,
        "eq" => fv == threshold,
        "ne" => fv != threshold,
        _ => false,
    }
}

/// One field within a `struct:` codec, with its byte position. Borrowed from
/// the schema; `'a` ties to the `CompiledSchema`.
pub struct StructFieldInfo<'a> {
    /// Dotted path "<SUB>.<field_id>" — the same key `enum_ref_at` /
    /// `allowed_targets` accept.
    pub field_path: String,
    pub field_id: &'a str,
    pub offset: usize,
    pub width: usize,
    pub enum_ref: Option<&'a str>,
    pub formlink_targets: &'a [String],
    pub null_allowed: bool,
}

/// Width in bytes of one struct codec token. `None` for variable-width tokens.
fn struct_token_width(token: &str) -> Option<usize> {
    match token {
        "b" | "B" | "x" => Some(1),
        "h" | "H" => Some(2),
        "i" | "I" | "f" => Some(4),
        "q" | "Q" => Some(8),
        _ if token.starts_with('s') => token[1..].parse::<usize>().ok(),
        _ => None,
    }
}

/// Fixed byte width of a whole-subrecord/whole-variant scalar codec name (the
/// non-`struct:` form, e.g. a union arm `uint8`/`uint16`). `None` for
/// variable-width / unknown codecs.
fn scalar_codec_width(codec: &str) -> Option<usize> {
    match codec {
        "int8" | "uint8" | "bool" => Some(1),
        "int16" | "uint16" | "uint16le" => Some(2),
        "int32" | "uint32" | "float32" | "formid" => Some(4),
        "int64" | "uint64" => Some(8),
        _ => None,
    }
}

fn struct_field_layout_for<'a>(
    sub_sig: &str,
    codec: Option<&str>,
    fields: &'a [SchemaFieldJson],
    form_version: Option<u16>,
) -> Vec<StructFieldInfo<'a>> {
    let Some(codec) = codec else {
        return Vec::new();
    };
    // A field gated by a `record_form_version` presence condition is absent from
    // the on-disk struct when the record's form_version doesn't satisfy it (e.g.
    // RACE.DATA's wbFromVersion(143) floats and wbFromVersion(188) backpack int).
    // Such a field consumes ZERO bytes, so it must be skipped for both width
    // accumulation and emission; otherwise every following field's offset is
    // skewed and RACE.DATA FKs are corrupted. Only filter when an explicit
    // form_version is supplied; the version-less call (validator / class_a
    // masking pass None) keeps the maximal layout.
    let field_present = |field: &SchemaFieldJson| -> bool {
        match form_version {
            Some(_) if !field.presence_conditions.is_empty() => field
                .presence_conditions
                .iter()
                .all(|c| condition_passes(c, form_version)),
            _ => true,
        }
    };
    // Scalar (non-struct) codec, e.g. a union arm `uint8`/`uint16` (LVLN.LVLF,
    // LVLI.LVLF) or a single-field subrecord: the sole field sits at offset 0
    // with the scalar's width. Without this, scalar-codec union arms yield no
    // layout and their flag bytes stay invisible to masking.
    if !codec.starts_with("struct:") {
        if let Some(width) = scalar_codec_width(codec) {
            if fields.len() == 1 {
                let field = &fields[0];
                if !field_present(field) {
                    return Vec::new();
                }
                return vec![StructFieldInfo {
                    field_path: format!("{sub_sig}.{}", field.id),
                    field_id: field.id.as_str(),
                    offset: 0,
                    width,
                    enum_ref: field.enum_ref.as_deref(),
                    formlink_targets: &field.formlink_targets,
                    null_allowed: field.null_allowed,
                }];
            }
        }
        return Vec::new();
    }
    let payload = match codec.strip_prefix("struct:") {
        Some(p) => p,
        None => return Vec::new(),
    };
    let tokens: Vec<&str> = payload
        .split(',')
        .map(|t| t.trim())
        .filter(|t| !t.is_empty())
        .collect();
    // Each token maps 1:1 with a schema field in order. If counts disagree the
    // codec/fields are out of sync — bail rather than emit wrong offsets.
    if tokens.len() != fields.len() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(fields.len());
    let mut offset = 0usize;
    for (token, field) in tokens.iter().zip(fields.iter()) {
        let Some(width) = struct_token_width(token) else {
            // Variable-width field: every following offset is undefined. Stop.
            break;
        };
        // Version-gated-out field: absent from these bytes, consumes 0 bytes —
        // skip it without advancing offset so following fields stay aligned.
        if !field_present(field) {
            continue;
        }
        out.push(StructFieldInfo {
            field_path: format!("{sub_sig}.{}", field.id),
            field_id: field.id.as_str(),
            offset,
            width,
            enum_ref: field.enum_ref.as_deref(),
            formlink_targets: &field.formlink_targets,
            null_allowed: field.null_allowed,
        });
        offset += width;
    }
    out
}

/// Resolved reference-target set returned by `CompiledSchema::allowed_targets`.
pub struct RefTargetSpec<'a> {
    pub targets: &'a [String],
    pub null_allowed: bool,
}

impl RefTargetSpec<'_> {
    pub fn allows_target(&self, target_sig: &str) -> bool {
        ref_target_allowed(self.targets, self.null_allowed, target_sig)
    }

    /// xEdit-format allowed-target label: declared sigs joined by ',' with
    /// `NULL` appended when allowed (e.g. "AACT,IDLE,NULL"). SHARED so the
    /// validator's Class C "expected: ..." wording and conv-refs' diagnostics
    /// can't drift. Mirrors xEdit's `aValidRefs` CommaText output.
    pub fn expected_label(&self) -> String {
        let mut out = self.targets.join(",");
        if self.null_allowed {
            if out.is_empty() {
                out.push_str("NULL");
            } else {
                out.push_str(",NULL");
            }
        }
        out
    }
}

static COMPILED_SCHEMA_CACHE: OnceLock<Mutex<HashMap<String, Arc<CompiledSchema>>>> =
    OnceLock::new();

/// Non-Py schema loader for callers that must not touch Python (e.g. the
/// conversion crate's phases run under `py.detach`). Shares the same cache as
/// `compiled_schema_for_game`.
pub fn compiled_schema_for_game_str(game: &str) -> Result<Arc<CompiledSchema>, String> {
    let normalized = game.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return Err("game is required for native authoring schema lookup".to_string());
    }
    let cache = COMPILED_SCHEMA_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    {
        let locked = cache
            .lock()
            .map_err(|_| "failed to lock schema cache".to_string())?;
        if let Some(schema) = locked.get(normalized.as_str()) {
            return Ok(schema.clone());
        }
    }
    let raw = crate::schema_registry::schema_json_for_game(normalized.as_str())
        .ok_or_else(|| format!("unsupported game for native authoring schema: {normalized}"))?;
    let parsed: SchemaDocumentJson = serde_json::from_str(raw)
        .map_err(|err| format!("failed to parse schema json for {normalized}: {err}"))?;
    let compiled = Arc::new(CompiledSchema {
        records: parsed
            .records
            .into_iter()
            .map(|record| (record.id.clone(), record))
            .collect(),
        enums: parsed
            .enums
            .into_iter()
            .map(|enum_def| (enum_def.id.clone(), enum_def))
            .collect(),
    });
    let mut locked = cache
        .lock()
        .map_err(|_| "failed to lock schema cache".to_string())?;
    locked.insert(normalized, compiled.clone());
    Ok(compiled)
}

pub fn compiled_schema_for_game(game: &str) -> PyResult<Arc<CompiledSchema>> {
    compiled_schema_for_game_str(game).map_err(value_error)
}

pub fn schema_game_for_plugin(
    plugin: &Bound<'_, PyAny>,
    fallback_game: Option<&str>,
) -> PyResult<Option<String>> {
    if let Some(game) = fallback_game {
        let trimmed = game.trim();
        if !trimmed.is_empty() {
            return Ok(Some(trimmed.to_ascii_lowercase()));
        }
    }
    let plugin_game = plugin.getattr("game")?;
    if plugin_game.is_none() {
        return Ok(None);
    }
    let game = plugin_game.extract::<String>()?;
    let trimmed = game.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    Ok(Some(trimmed.to_ascii_lowercase()))
}

pub fn schema_record_spec<'a>(
    schema: &'a CompiledSchema,
    record_signature: &str,
) -> Option<&'a SchemaRecordJson> {
    schema.records.get(record_signature)
}

pub fn schema_subrecord_spec<'a>(
    record_spec: &'a SchemaRecordJson,
    signature: &str,
    occurrence: usize,
) -> Option<&'a SchemaSubrecordJson> {
    let mut seen = 0usize;
    let mut last = None;
    for spec in &record_spec.subrecords {
        if spec.id != signature {
            continue;
        }
        if seen == occurrence {
            return Some(spec);
        }
        seen += 1;
        last = Some(spec);
    }
    last
}

/// Look up the Nth spec for ``signature`` whose ``scope_id`` matches the given
/// scope. ``scope`` of ``None`` means "top-level only" — top-level specs have
/// ``scope_id == None`` in the schema. Returns ``None`` if no spec for the sig
/// exists in that scope (caller can then try a different scope or fall back
/// to ``schema_subrecord_spec``).
///
/// Enables scope-aware dispatch so that, e.g., PACK CNAM
/// dispatches to the top-level Combat Style FormID spec when no Package Data
/// scope is active, and to the nested Value union spec inside Package Data.
pub fn schema_subrecord_spec_in_scope<'a>(
    record_spec: &'a SchemaRecordJson,
    signature: &str,
    scope: Option<&str>,
    occurrence: usize,
) -> Option<&'a SchemaSubrecordJson> {
    let mut seen = 0usize;
    let mut last = None;
    for spec in &record_spec.subrecords {
        if spec.id != signature {
            continue;
        }
        if spec.scope_id.as_deref() != scope {
            continue;
        }
        if seen == occurrence {
            return Some(spec);
        }
        seen += 1;
        last = Some(spec);
    }
    // Overflow fallback only fires when the last matching spec is repeatable —
    // otherwise the caller would silently consume a non-repeatable spec twice.
    // LENS is the motivating case: top-level wbFloat(DNAM) is non-repeatable,
    // so a second DNAM occurrence at scope=None must return None to let the
    // dispatcher roll over to the repeatable DNAM in scope=lens_flare_sprites.
    last.filter(|spec| spec.repeatable)
}

/// Returns true iff ``signature`` has at least one spec in ``record_spec`` for
/// the given scope. Used by the dispatcher to decide whether to stay in the
/// current scope or roll over to a different one.
pub fn schema_has_subrecord_in_scope(
    record_spec: &SchemaRecordJson,
    signature: &str,
    scope: Option<&str>,
) -> bool {
    record_spec
        .subrecords
        .iter()
        .any(|spec| spec.id == signature && spec.scope_id.as_deref() == scope)
}

#[cfg(test)]
mod metadata_tests {
    use super::*;

    fn fo4() -> std::sync::Arc<CompiledSchema> {
        compiled_schema_for_game_str("fo4").expect("fo4 schema parses")
    }

    fn fo76() -> std::sync::Arc<CompiledSchema> {
        compiled_schema_for_game_str("fo76").expect("fo76 schema parses")
    }

    #[test]
    fn struct_field_layout_versioned_skips_gated_fields() {
        // FO76 RACE.DATA carries wbFromVersion(143) floats (unknown_float1/2/3,
        // 12B) and a wbFromVersion(188) backpack int (4B). A form_version-131
        // record omits all four → the FK fields (severable_explosion, ...) must
        // sit at the FO4-equivalent offsets, NOT 16B later — honoring
        // per-field presence_conditions for a NON-union struct.
        let s = fo76();
        let max = s.struct_field_layout("RACE", "DATA"); // version-less = maximal
        let v131 = s.struct_field_layout_versioned("RACE", "DATA", Some(131));

        let off = |layout: &[StructFieldInfo<'_>], id: &str| {
            layout.iter().find(|f| f.field_id == id).map(|f| f.offset)
        };

        // The gated fields are present in the maximal layout, absent at FV-131.
        assert!(
            off(&max, "unknown_float1").is_some(),
            "maximal has gated float"
        );
        assert!(
            off(&max, "backpack_biped_object").is_some(),
            "maximal has backpack"
        );
        assert!(
            off(&v131, "unknown_float1").is_none(),
            "FV-131 omits gte-143 float"
        );
        assert!(
            off(&v131, "backpack_biped_object").is_none(),
            "FV-131 omits gte-188 int"
        );

        // The 3 gated floats precede the FK block, so at FV-131 the FK fields
        // shift 12B earlier than the maximal layout. Verify the relative shift.
        let sev_max = off(&max, "severable_explosion").expect("max severable_explosion");
        let sev_v131 = off(&v131, "severable_explosion").expect("v131 severable_explosion");
        assert_eq!(
            sev_v131 + 12,
            sev_max,
            "FV-131 severable_explosion must be 12B earlier than maximal (gated floats absent)"
        );

        // FV-131 fields stay contiguous (no offset gaps from skipped tokens).
        let mut expect = 0usize;
        for f in &v131 {
            assert_eq!(f.offset, expect, "FV-131 offset gap at {}", f.field_id);
            expect += f.width;
        }
    }

    #[test]
    fn record_flags_mask_and_permissive() {
        let s = fo4();
        let regn = s.record_def("REGN").unwrap().record_flags().unwrap();
        // bit 6 (0x40) + universal Deleted(0x20)|Ignored(0x1000).
        assert_eq!(regn.valid_mask(), 0x1060);
        assert!(!regn.is_permissive());
        // FO76 sets bits 1+3 ($A) on REGN headers — both stripped.
        assert_eq!(regn.strip(0x0A | 0x40), 0x40);
        assert_ne!(regn.invalid_bits(0x0A), 0);

        let parw = s.record_def("PARW").unwrap().record_flags().unwrap();
        assert!(parw.is_permissive());
        assert_eq!(parw.strip(0xDEAD_BEEF), 0xDEAD_BEEF);
        assert_eq!(parw.invalid_bits(0xDEAD_BEEF), 0);

        // BOOK has no explicit flags arg ⇒ strict universal mask (NOT None):
        // every FO4 record header is wbRecordFlags=wbFlagsList([]) at minimum.
        let book = s
            .record_def("BOOK")
            .unwrap()
            .record_flags()
            .expect("BOOK record_flags");
        assert!(!book.is_permissive());
        assert_eq!(book.valid_mask(), 0x1020);
    }

    #[test]
    fn allowed_targets_multi_and_null() {
        let s = fo4();
        // COBJ.CNAM is the 44-sig base-object set, NULL allowed.
        let cnam = s
            .allowed_targets("COBJ", "CNAM")
            .expect("COBJ.CNAM targets");
        assert!(cnam.targets.len() > 40);
        assert!(cnam.allows_target("WEAP"));
        assert!(cnam.allows_target("NULL"));
        assert!(!cnam.allows_target("REFR"));

        // IDLE.ANAM.parent = [AACT, IDLE, NULL].
        let parent = s
            .allowed_targets("IDLE", "ANAM.parent")
            .expect("IDLE.ANAM.parent targets");
        assert!(parent.allows_target("AACT"));
        assert!(parent.allows_target("NULL"));
        assert!(!parent.allows_target("LAND"));

        // REFR.XEZN = [ECZN], NULL not allowed (FO76 LCTN is illegal).
        let xezn = s
            .allowed_targets("REFR", "XEZN")
            .expect("REFR.XEZN targets");
        assert!(xezn.allows_target("ECZN"));
        assert!(!xezn.allows_target("LCTN"));
        assert!(!xezn.allows_target("NULL"));

        // Shared expected_label wording — NULL appended when allowed, absent otherwise.
        assert_eq!(parent.expected_label(), "AACT,IDLE,NULL");
        assert_eq!(xezn.expected_label(), "ECZN");
    }

    #[test]
    fn enum_flag_mask_and_clamp() {
        let s = fo4();
        let book = s.enum_def("BOOK.DNAM.flags").expect("BOOK.DNAM.flags");
        assert!(book.is_flags());
        assert_eq!(book.valid_flag_mask(), 0x1F);

        let kt = s.enum_def("keyword_type_enum").expect("keyword_type_enum");
        assert!(!kt.is_flags());
        assert!(kt.contains_value(0));
        assert!(kt.contains_value(18));
        assert!(!kt.contains_value(24)); // FO76-only kw type
        assert_eq!(kt.fallback_value(), Some(0));
    }

    #[test]
    fn enum_ref_locator_mirrors_targets() {
        let s = fo4();
        // KYWD.TNAM is the keyword-type value enum (subrecord-level or sole field).
        let er = s.enum_ref_at("KYWD", "TNAM").expect("KYWD.TNAM enum_ref");
        let ed = s.enum_def(er).expect("resolves");
        assert!(ed.contains_value(0) && !ed.contains_value(24));
        // enum_def_at convenience resolves to the same enum.
        let ed2 = s
            .enum_def_at("KYWD", "TNAM")
            .expect("KYWD.TNAM enum_def_at");
        assert_eq!(ed.fallback_value(), ed2.fallback_value());
        // A flag-bearing field inside a struct codec (BOOK.DNAM.flags).
        let book = s
            .enum_def_at("BOOK", "DNAM.flags")
            .expect("BOOK.DNAM.flags");
        assert!(book.is_flags());
        // No enum ⇒ None.
        assert!(s.enum_ref_at("KYWD", "EDID").is_none());
    }

    #[test]
    fn enum_ref_locator_disambiguates_scope_overloaded_qust_fnam() {
        let s = fo4();
        // Scope-blind: first FNAM spec wins — the 2-bit objective flag table.
        let blind = s.enum_def_at("QUST", "FNAM").expect("QUST.FNAM enum");
        assert_eq!(blind.valid_flag_mask(), 0x3);

        // Scope-aware: the alias FNAM is a different, 25-bit flag set. Masking
        // an alias row against the objective table would destroy quest_object
        // (0x4), allow_dead (0x10), essential (0x40), is_companion (0x80_0000)…
        let aliases = s
            .enum_def_at_in_scope("QUST", "FNAM", Some("aliases"))
            .expect("QUST.FNAM@aliases enum");
        assert_eq!(aliases.valid_flag_mask(), 0x1ff_ffff);
        for bit in [0x4_i128, 0x10, 0x40, 0x80_0000] {
            assert!(
                aliases.valid_flag_mask() & bit == bit,
                "alias flag {bit:#x} must survive masking"
            );
        }

        let objectives = s
            .enum_def_at_in_scope("QUST", "FNAM", Some("objectives"))
            .expect("QUST.FNAM@objectives enum");
        assert_eq!(objectives.valid_flag_mask(), 0x3);

        // Unknown scope falls back to the scope-blind match rather than None.
        assert!(
            s.enum_ref_at_in_scope("QUST", "FNAM", Some("no_such_scope"))
                .is_some()
        );
    }

    #[test]
    fn struct_field_layout_offsets_and_enum_refs() {
        let s = fo4();
        // BOOK.DNAM codec struct:B,I,I,I → flags(uint8)@0, then three uint32.
        let book = s.struct_field_layout("BOOK", "DNAM");
        assert!(!book.is_empty(), "BOOK.DNAM should have a struct layout");
        let flags = book
            .iter()
            .find(|f| f.field_id == "flags")
            .expect("flags field");
        assert_eq!(flags.offset, 0);
        assert_eq!(flags.width, 1);
        assert_eq!(flags.field_path, "DNAM.flags");
        assert_eq!(flags.enum_ref, Some("BOOK.DNAM.flags"));
        // Offsets are strictly increasing by width.
        let mut expect = 0usize;
        for f in &book {
            assert_eq!(f.offset, expect, "offset for {}", f.field_id);
            expect += f.width;
        }

        // ACTI.DSTD codec struct:B,B,B,B,i,I,I,i → flags is field[3] @ offset 3.
        let dstd = s.struct_field_layout("ACTI", "DSTD");
        let dflags = dstd
            .iter()
            .find(|f| f.field_id == "flags")
            .expect("DSTD flags");
        assert_eq!(dflags.offset, 3);
        assert_eq!(dflags.width, 1);
        assert_eq!(dflags.enum_ref, Some("ACTI.DSTD.flags"));
    }

    #[test]
    fn record_flags_universal_strict_for_no_flag_records() {
        let s = fo4();
        // COBJ has no flags arg in xEdit ⇒ strict universal mask (Deleted|Ignored),
        // NOT None. FO76's bit 26 must be stripped/flagged.
        let cobj = s
            .record_def("COBJ")
            .unwrap()
            .record_flags()
            .expect("COBJ record_flags");
        assert!(!cobj.is_permissive());
        assert_eq!(cobj.valid_mask(), 0x1020);
        assert_ne!(cobj.invalid_bits(1 << 26), 0);
        assert_eq!(cobj.strip(1 << 26 | 0x20), 0x20);
        // REFR's flag-decider union arms are all permissive ⇒ permissive (no over-strip).
        let refr = s
            .record_def("REFR")
            .unwrap()
            .record_flags()
            .expect("REFR record_flags");
        assert!(refr.is_permissive());
        assert_eq!(refr.strip(0xDEAD_BEEF), 0xDEAD_BEEF);
        // Every in-schema record now carries record_flags (no None).
        assert!(s.record_def("EXPL").unwrap().record_flags().is_some());
        assert!(s.record_def("LCRT").unwrap().record_flags().is_some());
    }

    #[test]
    fn subrecord_required_accessor() {
        let s = fo4();
        // ACTI.OBND is SetRequired in xEdit.
        let acti = s.record_def("ACTI").unwrap();
        let obnd = acti
            .subrecords
            .iter()
            .find(|x| x.id == "OBND")
            .expect("OBND");
        assert!(obnd.required());
    }

    #[test]
    fn xlcm_level_modifier_enum() {
        let s = fo4();
        // ACHR/REFR XLCM is xEdit's ordered enum 0..3; FO76 packs $07000001 in
        // the high byte, so class_a needs the enum + fallback to clamp.
        let e = s.enum_def_at("ACHR", "XLCM").expect("ACHR.XLCM enum");
        assert!(!e.is_flags());
        assert!(e.contains_value(0) && e.contains_value(3));
        assert!(!e.contains_value(0x0700_0001)); // FO76 wild value out of range
        assert_eq!(e.fallback_value(), Some(0));
    }

    #[test]
    fn iter_flag_fields_covers_struct_nested() {
        let s = fo4();
        // BOOK: the DNAM.flags struct sub-field must be enumerated.
        let book = s.iter_flag_fields("BOOK");
        assert!(
            book.iter()
                .any(|(path, e)| path == "DNAM.flags" && e.is_flags()),
            "BOOK.DNAM.flags must be enumerated as a struct-nested flag field"
        );
        // ACTI: DSTD.flags (struct sub-field) + FNAM (subrecord-level flag).
        let acti = s.iter_flag_fields("ACTI");
        assert!(
            acti.iter().any(|(p, _)| p == "DSTD.flags"),
            "ACTI.DSTD.flags"
        );
        // Every yielded enum is actually a flag enum.
        for (_p, e) in &acti {
            assert!(e.is_flags());
        }
        // LVLN.LVLF (516 offenders) is a subrecord-level flag enum in FO4 —
        // must be enumerated so conv-flags can mask it.
        let lvln = s.iter_flag_fields("LVLN");
        assert!(
            lvln.iter().any(|(p, _)| p == "LVLF"),
            "LVLN.LVLF flag field"
        );
        // EFSH.DNAM is a form-version UNION; its variant flag fields must be
        // enumerated even though they live under union_variants.
        let efsh = s.iter_flag_fields("EFSH");
        assert!(
            efsh.iter().any(|(p, _)| p.starts_with("DNAM.")),
            "EFSH.DNAM union variant flag fields must be enumerated"
        );
    }

    #[test]
    fn struct_field_layout_union_aware() {
        let s = fo4();
        // EFSH.DNAM: record_form_version union. Old format (<106) and new (>=106)
        // pick different variants ⇒ different field offsets.
        let old = s.struct_field_layout_versioned("EFSH", "DNAM", Some(50));
        let new = s.struct_field_layout_versioned("EFSH", "DNAM", Some(150));
        assert!(!old.is_empty() && !new.is_empty(), "both variants lay out");
        // The two variants differ (old format has more leading fields), so the
        // field count / first-field width should not be identical.
        assert!(
            old.len() != new.len() || old.first().map(|f| f.width) != new.first().map(|f| f.width),
            "form-version union must yield distinct layouts per version"
        );
        // None falls back to a variant (legacy/first) — must not panic / be empty.
        assert!(!s.struct_field_layout("EFSH", "DNAM").is_empty());
    }
}
