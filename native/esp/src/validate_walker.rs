//! Element-recursive walker for xEdit-parity error checking.
//!
//! Mirrors `CheckForErrorsLinear` in `refs/xedit/xEdit/xeMainForm.pas:3212`.
//! Walks the parsed plugin tree (`ParsedItem::Group` and `ParsedItem::Record`),
//! constructs xEdit-format paths as it descends, and runs per-element checks.

#[derive(Default)]
pub(crate) struct PathBuilder {
    components: Vec<String>,
}

impl PathBuilder {
    pub fn new() -> Self {
        Self {
            components: Vec::new(),
        }
    }

    pub fn push(&mut self, component: String) {
        self.components.push(component);
    }

    pub fn pop(&mut self) {
        self.components.pop();
    }

    /// Render as xEdit-format path: components joined with ` \ ` and a leading ` \ `.
    pub fn render(&self) -> String {
        let mut out = String::new();
        for c in &self.components {
            out.push_str(" \\ ");
            out.push_str(c);
        }
        out
    }
}

use crate::plugin_runtime::{ParsedGroup, ParsedItem, ParsedPlugin, ParsedRecord, ParsedSubrecord};

/// Parse the GRUP label as a little-endian u32 (FormID for child groups,
/// signature for top-level GRUPs, etc.).
fn group_label_as_u32(group: &ParsedGroup) -> u32 {
    u32::from_le_bytes(group.label)
}

/// Parse the GRUP label as 4 ASCII chars. Returns None if any byte is non-printable.
fn group_label_as_sig(group: &ParsedGroup) -> Option<String> {
    if group.label.iter().all(|b| (0x20..=0x7E).contains(b)) {
        Some(String::from_utf8_lossy(&group.label).into_owned())
    } else {
        None
    }
}

/// Editor ID from a record's EDID subrecord, if any.
fn editor_id(record: &ParsedRecord) -> Option<String> {
    for sub in &record.subrecords {
        if sub.signature.as_str() == "EDID" {
            let bytes = sub.data.as_ref();
            let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
            return Some(String::from_utf8_lossy(&bytes[..end]).into_owned());
        }
    }
    None
}

/// Format a GRUP child label for a top-level group: `[N] GRUP Top "SIG"`.
pub(crate) fn group_label(group: &ParsedGroup, index: usize) -> String {
    match group.group_type {
        0 => {
            let sig = group_label_as_sig(group).unwrap_or_else(|| "????".to_string());
            format!("[{index}] GRUP Top \"{sig}\"")
        }
        _ => group_label_with_parent(group, index, None),
    }
}

/// Format a non-top GRUP child label using the parent record (when available).
/// Falls back to a generic label when no parent record is provided (this happens
/// for nested groups whose preceding sibling we haven't tracked yet).
pub(crate) fn group_label_with_parent(
    group: &ParsedGroup,
    index: usize,
    parent: Option<&ParsedRecord>,
) -> String {
    let fid = group_label_as_u32(group);
    match group.group_type {
        1 => {
            if let Some(p) = parent {
                let edid = editor_id(p).unwrap_or_else(|| "<unknown>".to_string());
                format!(
                    "[{index}] GRUP World Children of {edid} [{}:{:08X}]",
                    p.signature.as_str(),
                    fid
                )
            } else {
                format!("[{index}] GRUP World Children of [{:08X}]", fid)
            }
        }
        2 => format!("[{index}] GRUP Interior Cell Block {}", fid),
        3 => format!("[{index}] GRUP Interior Cell Sub-Block {}", fid),
        4 => {
            // Exterior Cell Block X, Y — label bytes are i16 Y, i16 X (xEdit order: X then Y on display).
            let y = i16::from_le_bytes([group.label[0], group.label[1]]);
            let x = i16::from_le_bytes([group.label[2], group.label[3]]);
            format!("[{index}] GRUP Exterior Cell Block {x}, {y}")
        }
        5 => {
            let y = i16::from_le_bytes([group.label[0], group.label[1]]);
            let x = i16::from_le_bytes([group.label[2], group.label[3]]);
            format!("[{index}] GRUP Exterior Cell Sub-Block {x}, {y}")
        }
        6 => {
            if let Some(p) = parent {
                let edid = editor_id(p).unwrap_or_else(|| "<unknown>".to_string());
                format!(
                    "[{index}] GRUP Cell Children of {edid} [{}:{:08X}]",
                    p.signature.as_str(),
                    fid
                )
            } else {
                format!("[{index}] GRUP Cell Children of [{:08X}]", fid)
            }
        }
        7 => {
            if let Some(p) = parent {
                let edid = editor_id(p).unwrap_or_else(|| "<unknown>".to_string());
                format!(
                    "[{index}] GRUP Topic Children of {edid} [{}:{:08X}]",
                    p.signature.as_str(),
                    fid
                )
            } else {
                format!("[{index}] GRUP Topic Children of [{:08X}]", fid)
            }
        }
        8 => {
            if let Some(p) = parent {
                let edid = editor_id(p).unwrap_or_else(|| "<unknown>".to_string());
                format!(
                    "[{index}] GRUP Cell Persistent Children of {edid} [{}:{:08X}]",
                    p.signature.as_str(),
                    fid
                )
            } else {
                format!("[{index}] GRUP Cell Persistent Children of [{:08X}]", fid)
            }
        }
        9 => {
            if let Some(p) = parent {
                let edid = editor_id(p).unwrap_or_else(|| "<unknown>".to_string());
                format!(
                    "[{index}] GRUP Cell Temporary Children of {edid} [{}:{:08X}]",
                    p.signature.as_str(),
                    fid
                )
            } else {
                format!("[{index}] GRUP Cell Temporary Children of [{:08X}]", fid)
            }
        }
        _ => format!("[{index}] GRUP type {} [{:08X}]", group.group_type, fid),
    }
}

/// Format a record label: `[N] [SIG:FORMID]`.
pub(crate) fn record_label(record: &ParsedRecord, index: usize) -> String {
    format!(
        "[{}] [{}:{:08X}]",
        index,
        record.signature.as_str(),
        record.form_id
    )
}

/// Format a subrecord label: `[N] SIG` or `[N] SIG - display_label`.
pub(crate) fn subrecord_label(
    sub: &ParsedSubrecord,
    index: usize,
    display_label: Option<&str>,
) -> String {
    match display_label {
        Some(label) => format!("[{}] {} - {}", index, sub.signature.as_str(), label),
        None => format!("[{}] {}", index, sub.signature.as_str()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VisitedKind {
    Group,
    Record,
    Subrecord,
}

/// Wider context handed to the visitor so checks can inspect the element.
pub(crate) enum WalkContext<'a> {
    Group(&'a ParsedGroup),
    Record(&'a ParsedRecord),
    Subrecord {
        record: &'a ParsedRecord,
        sub: &'a ParsedSubrecord,
        index: usize,
    },
}

/// Walk every element in the plugin in tree order. Calls `visitor(path, kind, ctx)`
/// for each group, record, and subrecord. The visitor sees the path *before*
/// descending — so a group visit fires before its children.
pub(crate) fn walk_plugin<F>(plugin: &ParsedPlugin, plugin_load_index: usize, visitor: &mut F)
where
    F: FnMut(&PathBuilder, VisitedKind, WalkContext<'_>),
{
    let mut path = PathBuilder::new();
    path.push(format!("[{:02}] {}", plugin_load_index, plugin.plugin_name));

    for (i, item) in plugin.root_items.iter().enumerate() {
        walk_item(item, i, None, &mut path, visitor);
    }
}

fn walk_item<F>(
    item: &ParsedItem,
    index: usize,
    preceding_record: Option<&ParsedRecord>,
    path: &mut PathBuilder,
    visitor: &mut F,
) where
    F: FnMut(&PathBuilder, VisitedKind, WalkContext<'_>),
{
    match item {
        ParsedItem::Group(g) => {
            let label = if g.group_type == 0 {
                group_label(g, index)
            } else {
                group_label_with_parent(g, index, preceding_record)
            };
            path.push(label);
            visitor(path, VisitedKind::Group, WalkContext::Group(g));
            // Track the most-recent record sibling so nested child groups can
            // refer to their parent. The standard layout is `[Record, Group]`
            // pairs inside `g.children`.
            let mut last_record: Option<&ParsedRecord> = None;
            for (i, child) in g.children.iter().enumerate() {
                walk_item(child, i, last_record, path, visitor);
                if let ParsedItem::Record(r) = child {
                    last_record = Some(r);
                }
            }
            path.pop();
        }
        ParsedItem::Record(r) => {
            path.push(record_label(r, index));
            visitor(path, VisitedKind::Record, WalkContext::Record(r));
            for (i, sub) in r.subrecords.iter().enumerate() {
                path.push(subrecord_label(sub, i, None));
                visitor(
                    path,
                    VisitedKind::Subrecord,
                    WalkContext::Subrecord {
                        record: r,
                        sub,
                        index: i,
                    },
                );
                path.pop();
            }
            path.pop();
        }
    }
}

use crate::plugin_runtime::{CompiledSchema, SchemaEnumJson, SchemaSubrecordJson};

/// Append ` - {label}` to the last component of a `\ `-delimited path string.
/// Returns the original path unchanged when `label` is None.
fn augment_last_component_with_label(rendered: &str, label: Option<&str>) -> String {
    let Some(label) = label else {
        return rendered.to_string();
    };
    format!("{rendered} - {label}")
}

pub(crate) struct WalkerIssue {
    pub severity: &'static str,
    pub category: &'static str,
    pub plugin_handle: u64,
    pub plugin_name: String,
    pub message: String,
    pub form_id: Option<u32>,
    pub path: Option<String>,
    pub signature: Option<String>,
}

/// Walk a plugin, run all per-element checks, and return issues with full path.
pub(crate) fn walk_and_check(
    plugin: &ParsedPlugin,
    plugin_load_index: usize,
    plugin_handle: u64,
    schema: &CompiledSchema,
    out: &mut Vec<WalkerIssue>,
) {
    let mut visitor =
        |path: &PathBuilder, kind: VisitedKind, ctx: WalkContext<'_>| match (kind, ctx) {
            (VisitedKind::Record, WalkContext::Record(r)) => {
                let record_spec = schema.records.get(r.signature.as_str());
                if let Some(spec) = record_spec {
                    for msg in subrecord_order_errors(r, &spec.subrecords) {
                        out.push(WalkerIssue {
                            severity: "error",
                            category: "field_error",
                            plugin_handle,
                            plugin_name: plugin.plugin_name.clone(),
                            message: msg,
                            form_id: Some(r.form_id),
                            path: Some(path.render()),
                            signature: Some(r.signature.to_string()),
                        });
                    }
                    // Class A1 (record-header flags). xEdit:
                    //   "<record> \ Record Header \ Record Flags -> <Unknown: N $H>".
                    // None record_flags ⇒ not modelled ⇒ warn-only / no emission
                    // (never strip/flag what we didn't capture).
                    if let Some(message) = record_flag_error(r, spec) {
                        out.push(WalkerIssue {
                            severity: "warning",
                            category: "unknown_flag",
                            plugin_handle,
                            plugin_name: plugin.plugin_name.clone(),
                            message,
                            form_id: Some(r.form_id),
                            path: Some(format!(
                                "{} \\ Record Header \\ Record Flags",
                                path.render()
                            )),
                            signature: Some(r.signature.to_string()),
                        });
                    }
                }
            }
            (VisitedKind::Subrecord, WalkContext::Subrecord { record, sub, .. }) => {
                let record_spec = schema.records.get(record.signature.as_str());
                let sub_spec = record_spec.and_then(|rs| {
                    rs.subrecords
                        .iter()
                        .find(|s| s.id.as_str() == sub.signature.as_str())
                });
                if let Some(spec) = sub_spec {
                    let labeled_path = || {
                        augment_last_component_with_label(
                            &path.render(),
                            spec.display_label.as_deref(),
                        )
                    };
                    if subrecord_unused_byte_count(sub, spec).is_some() {
                        let warning_path = labeled_path();
                        out.push(WalkerIssue {
                            severity: "warning",
                            category: "field_error",
                            plugin_handle,
                            plugin_name: plugin.plugin_name.clone(),
                            message: format!("<Warning: Unused data in: {warning_path}>"),
                            form_id: Some(record.form_id),
                            path: Some(path.render()),
                            signature: Some(record.signature.to_string()),
                        });
                    }
                    // Classes A1 (flags), A2 (enums), D (short members) for every
                    // struct member of this subrecord — via the shared keystone
                    // layout so detection descends the IDENTICAL fields conv-flags
                    // masks and conv-refs FK-validates. Covers subrecord-level AND
                    // struct-nested fields, union variants resolved by form_version.
                    for member in struct_member_errors(
                        record.signature.as_str(),
                        sub,
                        record.form_version,
                        schema,
                    ) {
                        out.push(WalkerIssue {
                            severity: member.severity,
                            category: member.category,
                            plugin_handle,
                            plugin_name: plugin.plugin_name.clone(),
                            message: member.message,
                            form_id: Some(record.form_id),
                            path: Some(labeled_path()),
                            signature: Some(record.signature.to_string()),
                        });
                    }
                }
            }
            _ => {}
        };
    walk_plugin(plugin, plugin_load_index, &mut visitor);
}

#[cfg(test)]
pub(crate) fn walk_and_check_for_test(
    plugin: &ParsedPlugin,
    plugin_load_index: usize,
    schema: &CompiledSchema,
) -> Vec<String> {
    let mut issues = Vec::new();
    walk_and_check(plugin, plugin_load_index, 1, schema, &mut issues);
    issues.into_iter().map(|i| i.message).collect()
}

/// xEdit's cursor model: walk actual subrecords advancing an expected-def-pos
/// cursor through the schema's declared subrecord sequence. Emits an error for
/// any subrecord whose signature is unknown OR appears before the cursor.
///
/// Mirrors `refs/xedit/Core/wbImplementation.pas:10337-10443`.
pub(crate) fn subrecord_order_errors(
    record: &ParsedRecord,
    specs: &[SchemaSubrecordJson],
) -> Vec<String> {
    #[derive(Clone, Copy)]
    struct ActiveScope {
        start: usize,
        end: usize,
        next: usize,
        anchor_count: usize,
        batched: bool,
        poisoned: bool,
    }

    let mut errors = Vec::new();
    let mut cursor: usize = 0;
    let mut active_scope: Option<ActiveScope> = None;
    let record_sig = record.signature.as_str();

    for sub in &record.subrecords {
        let sub_sig = sub.signature.as_str();
        if let Some(scope) = active_scope {
            let anchor_sig = specs[scope.start].id.as_str();
            if sub_sig == anchor_sig {
                active_scope = Some(ActiveScope {
                    next: scope.start + 1,
                    anchor_count: scope.anchor_count + 1,
                    batched: false,
                    poisoned: false,
                    ..scope
                });
                continue;
            }

            // A scoped array may contain a nested repeating element whose
            // anchor is not the scope's first member. Re-match only known
            // nested anchors and reset the cursor for that element. This is
            // deliberately grammar-specific so genuine de-interleaving of a
            // single struct's children (SPEL EFID/EFIT) is still caught.
            if let Some(pos) = (scope.start..scope.end).find(|&i| {
                specs[i].id.as_str() == sub_sig
                    && scope_repeating_element_anchor(
                        record_sig,
                        specs[i].scope_id.as_deref(),
                        sub_sig,
                        specs,
                        scope.start,
                        scope.end,
                        scope.next,
                    )
            }) {
                active_scope = Some(ActiveScope {
                    next: pos + 1,
                    poisoned: false,
                    ..scope
                });
                continue;
            }

            if scope.next < scope.end {
                if qust_nested_condition_missing_parent(
                    record_sig,
                    specs[scope.start].scope_id.as_deref(),
                    sub_sig,
                    specs,
                    scope.start,
                    scope.end,
                    scope.next,
                ) {
                    errors.push(subrecord_order_error_message(record_sig, sub_sig));
                    active_scope = Some(ActiveScope {
                        poisoned: true,
                        ..scope
                    });
                    continue;
                }
                if is_qust_alias_scope(record_sig, specs, scope.start)
                    && qust_alias_anchor(sub_sig).is_some()
                {
                    if scope.poisoned {
                        errors.push(subrecord_order_error_message(record_sig, sub_sig));
                        active_scope = Some(ActiveScope {
                            poisoned: true,
                            ..scope
                        });
                        continue;
                    }
                } else {
                    if scope.batched
                        && qust_alias_scoped_sig(specs, sub_sig)
                        && !qust_reference_alias_batched_child(sub_sig)
                    {
                        errors.push(subrecord_order_error_message(record_sig, sub_sig));
                        active_scope = Some(ActiveScope {
                            poisoned: true,
                            ..scope
                        });
                        continue;
                    }

                    let scoped_pos = specs
                        .get(scope.next..scope.end)
                        .and_then(|tail| tail.iter().position(|s| s.id.as_str() == sub_sig))
                        .map(|i| i + scope.next);
                    if let Some(pos) = scoped_pos {
                        if race_body_data_forward_jump_crosses_section_anchor(
                            record_sig,
                            specs,
                            scope.start,
                            scope.next,
                            pos,
                            sub_sig,
                        ) {
                            errors.push(subrecord_order_error_message(record_sig, sub_sig));
                            active_scope = Some(ActiveScope {
                                poisoned: true,
                                ..scope
                            });
                            continue;
                        }
                        active_scope = Some(ActiveScope {
                            next: pos + 1,
                            ..scope
                        });
                        continue;
                    }

                    if scope.anchor_count > 1
                        && is_qust_alias_scope(record_sig, specs, scope.start)
                        && qust_reference_alias_batched_child(sub_sig)
                        && specs[scope.start + 1..scope.next]
                            .iter()
                            .any(|s| s.id.as_str() == sub_sig)
                    {
                        active_scope = Some(ActiveScope {
                            batched: true,
                            ..scope
                        });
                        continue;
                    }

                    if specs[scope.start..scope.end]
                        .iter()
                        .any(|s| s.id.as_str() == sub_sig)
                    {
                        if !race_body_data_behavior_transition(
                            record_sig,
                            specs,
                            scope.start,
                            scope.end,
                            sub_sig,
                        ) {
                            errors.push(subrecord_order_error_message(record_sig, sub_sig));
                            active_scope = Some(ActiveScope {
                                poisoned: true,
                                ..scope
                            });
                            continue;
                        }
                    }
                }
            }

            cursor = scope.end;
            active_scope = None;
        }

        // Find from cursor forward (in-sequence match). A signature can appear
        // both as a child inside a scoped block and later as a normal slot
        // (NPC_.FULL, SCEN.PNAM/INAM). A scoped child is only a valid starting
        // match when it is the scope anchor, so keep scanning for a later slot.
        let forward_pos = forward_starting_spec_position(record_sig, specs, cursor, sub_sig);

        if let Some(pos) = forward_pos {
            let spec = &specs[pos];
            if spec.scope_id.is_some() {
                let (start, end) = scope_segment_bounds(record_sig, specs, pos);
                active_scope = Some(ActiveScope {
                    start,
                    end,
                    next: start + 1,
                    anchor_count: 1,
                    batched: false,
                    poisoned: false,
                });
                continue;
            }
            // Advance cursor: if not repeatable, move past this slot.
            // If repeatable, keep cursor at pos so subsequent same-sig matches stay in-order.
            if !spec.repeatable {
                cursor = pos + 1;
            } else {
                cursor = pos;
            }
            continue;
        }

        // Not found from cursor forward — either unknown sig, or earlier-than-cursor.
        errors.push(subrecord_order_error_message(record_sig, sub_sig));
        // Do not advance cursor — xEdit treats the bad sub as skipped.
    }

    errors
}

fn forward_starting_spec_position(
    record_sig: &str,
    specs: &[SchemaSubrecordJson],
    cursor: usize,
    sub_sig: &str,
) -> Option<usize> {
    (cursor..specs.len()).find(|&pos| {
        let spec = &specs[pos];
        if spec.id.as_str() != sub_sig {
            return false;
        }
        if spec.scope_id.is_none() {
            return true;
        }
        let (start, _) = scope_segment_bounds(record_sig, specs, pos);
        pos == start
    })
}

fn scope_segment_bounds(
    record_sig: &str,
    specs: &[SchemaSubrecordJson],
    pos: usize,
) -> (usize, usize) {
    if is_qust_alias_scope(record_sig, specs, pos) {
        let mut start = pos;
        while start > 0
            && is_qust_alias_scope(record_sig, specs, start - 1)
            && qust_alias_anchor(specs[start].id.as_str()).is_none()
        {
            start -= 1;
        }

        if qust_alias_anchor(specs[start].id.as_str()).is_some() {
            let mut end = start + 1;
            while end < specs.len()
                && is_qust_alias_scope(record_sig, specs, end)
                && qust_alias_anchor(specs[end].id.as_str()).is_none()
            {
                end += 1;
            }
            return (start, end);
        }
    }

    let scope = specs[pos].scope_id.as_deref();
    let mut start = pos;
    while start > 0 && specs[start - 1].scope_id.as_deref() == scope {
        start -= 1;
    }

    let mut end = pos + 1;
    while end < specs.len() && specs[end].scope_id.as_deref() == scope {
        end += 1;
    }

    (start, end)
}

fn is_qust_alias_scope(record_sig: &str, specs: &[SchemaSubrecordJson], pos: usize) -> bool {
    record_sig == "QUST" && specs[pos].scope_id.as_deref() == Some("aliases")
}

/// Whether `sig` begins a nested repeated element inside a flattened scope.
/// The generated schema retains a scope id but not the nested array/union
/// boundaries, so only the grammars proven to contain such elements are
/// enumerated here. Primary scope anchors remain handled by the normal cursor.
fn scope_repeating_element_anchor(
    record_sig: &str,
    scope_id: Option<&str>,
    sig: &str,
    specs: &[SchemaSubrecordJson],
    scope_start: usize,
    scope_end: usize,
    next: usize,
) -> bool {
    let position = |target: &str| {
        specs[scope_start..scope_end]
            .iter()
            .position(|spec| spec.id.as_str() == target)
            .map(|offset| scope_start + offset)
    };
    match scope_id {
        // LAND layers: array of BTXT | (ATXT + VTXT). Both BTXT and ATXT start
        // an element; VTXT is the ATXT alpha-data child.
        Some("layers") => record_sig == "LAND" && matches!(sig, "BTXT" | "ATXT"),
        // Object Template (WEAP/ARMO/...): OBTE count, then a 'Combinations'
        // RArray of [OBTF, FULL, OBTS], then STOP. OBTF starts each combination
        // but is not the scope's first member (OBTE is).
        Some("object_template") => sig == "OBTF",
        // One quest stage index contains repeated log entries; each log entry
        // can in turn contain repeated conditions.
        Some("stages") if record_sig == "QUST" => match sig {
            "QSDT" => true,
            "CTDA" => {
                position("QSDT").is_some_and(|qsdt| next > qsdt)
                    && position("CNAM").is_none_or(|cnam| next <= cnam)
            }
            _ => false,
        },
        // One quest objective contains repeated targets; each target can have
        // repeated conditions.
        Some("objectives") if record_sig == "QUST" => match sig {
            "QSTA" => true,
            "CTDA" => position("QSTA").is_some_and(|qsta| next > qsta),
            _ => false,
        },
        // Alias conditions are a nested repeated condition array, but only
        // while the cursor remains in that alias's CTDA/CIS region. Once a
        // later alias field is consumed, CTDA cannot jump backward.
        Some("aliases") if record_sig == "QUST" && sig == "CTDA" => {
            position("CIS2").is_some_and(|cis2| next <= cis2.saturating_add(1))
        }
        // A terminal menu item can carry more than one condition.
        Some("menu_items") => record_sig == "TERM" && sig == "CTDA",
        // A magic effect can carry a conjunction of multiple conditions.
        Some("effects") => matches!(record_sig, "ALCH" | "ENCH" | "SPEL") && sig == "CTDA",
        // DEST owns an array of destruction stages, each beginning at DSTD and
        // ending at DSTF.
        Some("destructible") => sig == "DSTD",
        _ => false,
    }
}

fn race_body_data_behavior_transition(
    record_sig: &str,
    specs: &[SchemaSubrecordJson],
    scope_start: usize,
    scope_end: usize,
    sub_sig: &str,
) -> bool {
    if record_sig != "RACE"
        || specs[scope_start].scope_id.as_deref() != Some("body_data")
        || !matches!(sub_sig, "MNAM" | "FNAM")
    {
        return false;
    }
    let target_scope = if sub_sig == "MNAM" {
        "male_behavior_graph"
    } else {
        "female_behavior_graph"
    };
    forward_starting_spec_position(record_sig, specs, scope_end, sub_sig).is_some_and(|pos| {
        specs[pos].scope_id.as_deref() == Some(target_scope)
            && scope_segment_bounds(record_sig, specs, pos).0 == pos
    })
}

fn qust_nested_condition_missing_parent(
    record_sig: &str,
    scope_id: Option<&str>,
    sub_sig: &str,
    specs: &[SchemaSubrecordJson],
    scope_start: usize,
    scope_end: usize,
    next: usize,
) -> bool {
    if record_sig != "QUST" || sub_sig != "CTDA" {
        return false;
    }
    let position = |target: &str| {
        specs[scope_start..scope_end]
            .iter()
            .position(|spec| spec.id.as_str() == target)
            .map(|offset| scope_start + offset)
    };
    match scope_id {
        Some("stages") => {
            !position("QSDT").is_some_and(|qsdt| next > qsdt)
                || position("CNAM").is_some_and(|cnam| next > cnam)
        }
        Some("objectives") => !position("QSTA").is_some_and(|qsta| next > qsta),
        _ => false,
    }
}

fn race_body_data_forward_jump_crosses_section_anchor(
    record_sig: &str,
    specs: &[SchemaSubrecordJson],
    scope_start: usize,
    next: usize,
    target: usize,
    sub_sig: &str,
) -> bool {
    record_sig == "RACE"
        && specs[scope_start].scope_id.as_deref() == Some("body_data")
        && matches!(sub_sig, "MODL" | "MODT" | "MODC" | "MODS" | "MODF")
        && specs[next..target]
            .iter()
            .any(|spec| matches!(spec.id.as_str(), "FNAM" | "INDX"))
}

fn qust_alias_anchor(sig: &str) -> Option<&'static str> {
    match sig {
        "ALST" => Some("reference"),
        "ALLS" => Some("location"),
        "ALCS" => Some("ref_collection"),
        _ => None,
    }
}

fn qust_alias_scoped_sig(specs: &[SchemaSubrecordJson], sig: &str) -> bool {
    specs
        .iter()
        .any(|s| s.scope_id.as_deref() == Some("aliases") && s.id.as_str() == sig)
}

fn qust_reference_alias_batched_child(sig: &str) -> bool {
    matches!(sig, "ALCO" | "ALCL" | "ALNT" | "ALLA")
}

fn subrecord_order_error_message(record_sig: &str, sub_sig: &str) -> String {
    let card = subrecord_signature_cardinal(sub_sig);
    format!(
        "Error: record {record_sig} contains unexpected (or out of order) subrecord {sub_sig} {card:08X}"
    )
}

/// Read the 4 signature bytes as a little-endian u32.
/// xEdit: `IntToHex(Int64(Cardinal('XILS')), 8)`.
pub(crate) fn subrecord_signature_cardinal(sig: &str) -> u32 {
    let bytes = sig.as_bytes();
    let mut buf = [0u8; 4];
    for (i, b) in bytes.iter().take(4).enumerate() {
        buf[i] = *b;
    }
    u32::from_le_bytes(buf)
}

/// Return Some(N) where N > 0 if the subrecord payload is longer than what
/// the schema codec declares. Return None for variable-length codecs or when
/// the size is exact. Mirrors `refs/xedit/Core/wbImplementation.pas:15614`.
pub(crate) fn subrecord_unused_byte_count(
    sub: &ParsedSubrecord,
    spec: &SchemaSubrecordJson,
) -> Option<usize> {
    let codec = spec.codec.as_deref()?;
    let declared = codec_declared_size(codec)?;
    let actual = sub.data.len();
    if actual > declared {
        Some(actual - declared)
    } else {
        None
    }
}

/// One struct-member finding (A1 unknown-flag, A2 unknown-enum, or D short).
pub(crate) struct MemberIssue {
    pub severity: &'static str,
    pub category: &'static str,
    pub message: String,
}

/// Classes A1 (flags), A2 (enums) and D (short data) for every struct member of
/// a subrecord, via the shared keystone `struct_field_layout_versioned` so
/// detection descends the SAME fields conv-flags masks / conv-refs FK-validates.
///
/// For each member of the active (version-resolved) layout:
/// - read the little-endian value at `offset`/`width`;
/// - flag enum  → A1 unknown-bit list (`TwbFlagsDef.Check`, comma-joined);
/// - scalar enum → A2 unknown value (`TwbEnumDef.ToString` → `<Unknown: N $H>`);
/// - D (short): the member's `offset+width` exceeds the payload, i.e. the field
///   xEdit reports "Expected {width} bytes of data, found {available}". This is
///   the per-MEMBER check (SNDR.BNAM "Static Attenuation" u16, RACE.HCLF member)
///   xEdit does — NOT the whole-subrecord codec-size check that produced the
///   PACK.CNAM/FURN.WBDT false positives.
///
/// `form_version` (from `ParsedRecord`) selects the active union variant.
pub(crate) fn struct_member_errors(
    record_sig: &str,
    sub: &ParsedSubrecord,
    form_version: Option<u16>,
    schema: &CompiledSchema,
) -> Vec<MemberIssue> {
    let layout =
        schema.struct_field_layout_versioned(record_sig, sub.signature.as_str(), form_version);
    let mut out = Vec::new();
    let data = sub.data.as_ref();
    for member in &layout {
        if member.width == 0 {
            continue;
        }
        let available = data.len().saturating_sub(member.offset);
        // D — member runs past the payload. xEdit only size-checks fixed numeric
        // members; the layout's width is that fixed size. Skip if the member
        // starts beyond the payload AND it's the trailing optional tail (xEdit
        // tolerates a wholly-absent trailing field only when the def marks it
        // optional — but the FO76 truncation cases (BNAM, HCLF member) are an
        // expected fixed member short, which is exactly available < width here).
        if available < member.width {
            out.push(MemberIssue {
                severity: "error",
                category: "data_size",
                message: format!(
                    "Expected {} bytes of data, found {}",
                    member.width, available
                ),
            });
            // Can't read a short member's value for A1/A2 — move on.
            continue;
        }
        let Some(eref) = member.enum_ref else {
            continue;
        };
        let Some(enum_def) = schema.enums.get(eref) else {
            continue;
        };
        let Some(value) = read_enum_value(&data[member.offset..], member.width) else {
            continue;
        };
        if enum_def.is_flags() {
            let known = known_flag_bits(enum_def);
            let bit_count = (member.width * 8).min(128);
            if let Some(msg) = unknown_bits_message((value as u128) & !known, bit_count) {
                out.push(MemberIssue {
                    severity: "warning",
                    category: "unknown_flag",
                    message: msg,
                });
            }
        } else if enum_def.token_for_value(value).is_none() {
            out.push(MemberIssue {
                severity: "warning",
                category: "unknown_enum",
                message: unknown_int_string(value),
            });
        }
    }
    out
}

/// Class A1 (record-header flags). Returns the xEdit-format comma-joined
/// `<Unknown: bit $H>` list for every set header-flag bit not valid under this
/// record's `record_flags` metadata, or None when all bits are valid / the def
/// is permissive / no metadata was captured. xEdit `TwbFlagsDef.Check` over the
/// record-header flags def (32-bit).
///
/// `record_flags() == None` ⇒ no metadata ⇒ None (we never flag header bits we
/// didn't model — warn-only contract).
pub(crate) fn record_flag_error(
    record: &ParsedRecord,
    spec: &crate::plugin_runtime::SchemaRecordJson,
) -> Option<String> {
    let rf = spec.record_flags()?;
    let invalid = rf.invalid_bits(record.flags);
    unknown_bits_message(invalid as u128, 32)
}

/// Render set bits of `bits` (scanning 0..bit_count) as xEdit's comma-joined
/// `<Unknown: i $H>` list. None when no bits are set.
fn unknown_bits_message(bits: u128, bit_count: usize) -> Option<String> {
    let mut parts = Vec::new();
    for bit in 0..bit_count.min(128) {
        if (bits >> bit) & 1 == 1 {
            parts.push(unknown_int_string(bit as i128));
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(", "))
    }
}

/// OR of the bit positions named by a flags enum. Each `values` entry of a flags
/// enum is the bit's *mask* (e.g. `0x400` ⇒ bit 10), so a value that is a single
/// set bit contributes that bit to the known mask. Non-power-of-two entries
/// (rare composite flags) contribute all their set bits.
fn known_flag_bits(enum_def: &SchemaEnumJson) -> u128 {
    let mut mask: u128 = 0;
    for entry in &enum_def.values {
        if entry.value > 0 {
            mask |= entry.value as u128;
        }
    }
    mask
}

/// Read a little-endian unsigned integer of `byte_width` from the front of the
/// payload. Returns None if the payload is shorter than `byte_width`.
fn read_enum_value(data: &[u8], byte_width: usize) -> Option<i128> {
    if byte_width == 0 || data.len() < byte_width || byte_width > 8 {
        return None;
    }
    let mut buf = [0u8; 8];
    buf[..byte_width].copy_from_slice(&data[..byte_width]);
    Some(u64::from_le_bytes(buf) as i128)
}

/// xEdit `wbGetUnknownIntString` (`wbInterface.pas:23990`):
/// `<Unknown: {decimal} ${hex}>` with uppercase hex and leading zeros trimmed.
/// (The 8-digit signature suffix is only appended for full-cardinal values and
/// never appears for the small enum/flag values we report, so it is omitted.)
fn unknown_int_string(value: i128) -> String {
    if value == 0 {
        // IntToHex(0).TrimLeft(['0']) == "" ⇒ no $hex segment.
        return "<Unknown: 0>".to_string();
    }
    format!("<Unknown: {value} ${:X}>", value)
}

/// Return the fixed byte size of a codec, or None if it's variable-length.
fn codec_declared_size(codec: &str) -> Option<usize> {
    match codec {
        "u8" | "i8" | "int8" | "uint8" => Some(1),
        "u16" | "i16" | "int16" | "uint16" => Some(2),
        "u32" | "i32" | "int32" | "uint32" | "f32" | "float" | "float32" | "formid" | "form_id" => {
            Some(4)
        }
        "u64" | "i64" | "int64" | "uint64" | "f64" | "double" => Some(8),
        "empty" => Some(0),
        // Variable-length codecs — never warn.
        "zstring" | "lstring" | "bytes" | "raw" | "string" | "lenstring16" | "omod_data"
        | "model_info" | "formid_array" => None,
        other => {
            if let Some(rest) = other.strip_prefix("struct:") {
                struct_codec_size(rest)
            } else if other.starts_with("array:") || other.starts_with("array_struct:") {
                None
            } else if let Some(rest) = other.strip_prefix("fixed_string:") {
                rest.parse::<usize>().ok()
            } else {
                None
            }
        }
    }
}

/// Sum the byte sizes of a comma-or-space-separated struct codec
/// (e.g. "I,I,f" → 12, "I I f" → 12).
fn struct_codec_size(spec: &str) -> Option<usize> {
    let mut total: usize = 0;
    for tok in spec.split(|c| c == ',' || c == ' ') {
        let tok = tok.trim();
        if tok.is_empty() {
            continue;
        }
        let size = match tok {
            "b" | "B" | "?" => 1,
            "h" | "H" => 2,
            "i" | "I" | "f" | "l" | "L" => 4,
            "q" | "Q" | "d" => 8,
            // s260 — fixed 260-byte string fields used in some schemas
            "s260" => 260,
            _ => return None,
        };
        total += size;
    }
    Some(total)
}

/// Test-only walker wrapper that ignores the rich context for simpler assertions.
#[cfg(test)]
pub(crate) fn walk_plugin_for_test<F>(
    plugin: &ParsedPlugin,
    plugin_load_index: usize,
    visitor: &mut F,
) where
    F: FnMut(&PathBuilder, VisitedKind),
{
    let mut inner = |path: &PathBuilder, kind: VisitedKind, _ctx: WalkContext<'_>| {
        visitor(path, kind);
    };
    walk_plugin(plugin, plugin_load_index, &mut inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin_runtime::{
        ParsedGroup, ParsedPlugin, ParsedPluginHeader, ParsedRecord, ParsedSubrecord,
        SchemaRecordFlagsJson, SchemaRecordJson,
    };
    use bytes::Bytes;
    use smol_str::SmolStr;

    fn empty_subrec(sig: &str) -> ParsedSubrecord {
        ParsedSubrecord {
            signature: SmolStr::new(sig),
            data: Bytes::new(),
            semantic_type: None,
        }
    }

    fn edid_subrec(name: &str) -> ParsedSubrecord {
        let mut bytes = name.as_bytes().to_vec();
        bytes.push(0);
        ParsedSubrecord {
            signature: SmolStr::new("EDID"),
            data: Bytes::from(bytes),
            semantic_type: Some("zstring".to_string()),
        }
    }

    fn record(sig: &str, form_id: u32, edid: Option<&str>) -> ParsedRecord {
        let mut subs = Vec::new();
        if let Some(e) = edid {
            subs.push(edid_subrec(e));
        }
        ParsedRecord {
            signature: SmolStr::new(sig),
            form_id,
            flags: 0,
            version_control: 0,
            form_version: None,
            version2: None,
            subrecords: subs,
            raw_payload: None,
            parse_error: None,
        }
    }

    #[test]
    fn path_builder_empty_renders_empty() {
        let p = PathBuilder::new();
        assert_eq!(p.render(), "");
    }

    #[test]
    fn path_builder_renders_xedit_format() {
        let mut p = PathBuilder::new();
        p.push("[01] B21_AppalachiaCell00Compare.esp".to_string());
        p.push("[8] GRUP Top \"WRLD\"".to_string());
        p.push("[1] GRUP World Children of APPALACHIA [WRLD:0125DA15]".to_string());
        assert_eq!(
            p.render(),
            " \\ [01] B21_AppalachiaCell00Compare.esp \\ [8] GRUP Top \"WRLD\" \\ [1] GRUP World Children of APPALACHIA [WRLD:0125DA15]"
        );
    }

    #[test]
    fn path_builder_pop_drops_last_component() {
        let mut p = PathBuilder::new();
        p.push("A".to_string());
        p.push("B".to_string());
        p.pop();
        assert_eq!(p.render(), " \\ A");
    }

    #[test]
    fn label_top_group_uses_signature() {
        let g = ParsedGroup {
            label: *b"WRLD",
            group_type: 0,
            tail: Bytes::new(),
            children: Vec::new(),
        };
        assert_eq!(group_label(&g, 8), "[8] GRUP Top \"WRLD\"");
    }

    #[test]
    fn label_world_children_uses_parent_edid_and_formid() {
        let g = ParsedGroup {
            label: [0x15, 0xDA, 0x25, 0x01], // little-endian 0x0125DA15
            group_type: 1,
            tail: Bytes::new(),
            children: Vec::new(),
        };
        let parent = record("WRLD", 0x0125_DA15, Some("APPALACHIA"));
        assert_eq!(
            group_label_with_parent(&g, 1, Some(&parent)),
            "[1] GRUP World Children of APPALACHIA [WRLD:0125DA15]"
        );
    }

    #[test]
    fn label_record_has_signature_and_form_id() {
        let r = record("WRLD", 0x0125_DA15, Some("APPALACHIA"));
        assert_eq!(record_label(&r, 0), "[0] [WRLD:0125DA15]");
    }

    #[test]
    fn label_subrecord_has_index_sig_and_display_label() {
        let s = empty_subrec("XPDD");
        assert_eq!(
            subrecord_label(&s, 4, Some("Projected Decal")),
            "[4] XPDD - Projected Decal"
        );
        assert_eq!(subrecord_label(&s, 4, None), "[4] XPDD");
    }

    #[test]
    fn walker_visits_top_level_record() {
        let plugin = ParsedPlugin {
            plugin_name: "p.esp".to_string(),
            file_path: String::new(),
            header_size: 0,
            header: ParsedPluginHeader::default_for_test(),
            root_items: vec![ParsedItem::Record(record(
                "GMST",
                0x0000_0001,
                Some("MyGMST"),
            ))],
            game: None,
        };
        let mut visited = Vec::new();
        let mut collector = |path: &PathBuilder, kind: VisitedKind| {
            visited.push((path.render(), kind));
        };
        walk_plugin_for_test(&plugin, 1, &mut collector);
        // 1 record + 1 subrecord (EDID) = 2 visits
        assert_eq!(visited.len(), 2);
        assert_eq!(visited[0].1, VisitedKind::Record);
        assert!(visited[0].0.contains("[GMST:00000001]"));
    }

    #[test]
    fn walker_visits_nested_cell_in_wrld() {
        let wrld_record = record("WRLD", 0x0125_DA15, Some("APPALACHIA"));
        let cell_record = record("CELL", 0x0126_28FE, Some("OriginExt"));
        let refr_record = record("REFR", 0x0131_B717, None);

        let cell_children = ParsedItem::Group(ParsedGroup {
            label: 0x0126_28FE_u32.to_le_bytes(),
            group_type: 6,
            tail: Bytes::new(),
            children: vec![ParsedItem::Record(refr_record)],
        });
        let world_children = ParsedItem::Group(ParsedGroup {
            label: 0x0125_DA15_u32.to_le_bytes(),
            group_type: 1,
            tail: Bytes::new(),
            children: vec![ParsedItem::Record(cell_record), cell_children],
        });
        let top_wrld = ParsedItem::Group(ParsedGroup {
            label: *b"WRLD",
            group_type: 0,
            tail: Bytes::new(),
            children: vec![ParsedItem::Record(wrld_record), world_children],
        });
        let plugin = ParsedPlugin {
            plugin_name: "p.esp".to_string(),
            file_path: String::new(),
            header_size: 0,
            header: ParsedPluginHeader::default_for_test(),
            root_items: vec![top_wrld],
            game: None,
        };

        let mut record_paths: Vec<(String, String)> = Vec::new();
        let mut collector = |path: &PathBuilder, kind: VisitedKind| {
            if kind == VisitedKind::Record {
                record_paths.push((
                    path.render(),
                    path.components.last().cloned().unwrap_or_default(),
                ));
            }
        };
        walk_plugin_for_test(&plugin, 1, &mut collector);

        // We expect 3 records visited: WRLD, CELL, REFR.
        let sigs: Vec<&str> = record_paths
            .iter()
            .map(|(_, last)| {
                if last.contains("[WRLD") {
                    "WRLD"
                } else if last.contains("[CELL") {
                    "CELL"
                } else if last.contains("[REFR") {
                    "REFR"
                } else {
                    "?"
                }
            })
            .collect();
        assert_eq!(sigs, vec!["WRLD", "CELL", "REFR"]);

        let refr_path = &record_paths[2].0;
        assert!(
            refr_path.contains("GRUP World Children of APPALACHIA [WRLD:0125DA15]"),
            "got: {refr_path}"
        );
        assert!(
            refr_path.contains("GRUP Cell Children of OriginExt [CELL:012628FE]"),
            "got: {refr_path}"
        );
    }

    fn make_subrec_spec(id: &str, repeatable: bool) -> SchemaSubrecordJson {
        SchemaSubrecordJson {
            id: id.to_string(),
            kind: "raw".to_string(),
            repeatable,
            ..Default::default()
        }
    }

    fn make_scoped_subrec_spec(id: &str, scope_id: &str) -> SchemaSubrecordJson {
        let mut spec = make_subrec_spec(id, true);
        spec.scope_id = Some(scope_id.to_string());
        spec
    }

    fn spec_with_codec(id: &str, codec: &str) -> SchemaSubrecordJson {
        let mut s = make_subrec_spec(id, false);
        s.codec = Some(codec.to_string());
        s
    }

    #[test]
    fn unused_data_check_no_warning_for_exact_size_u32() {
        let spec = spec_with_codec("DATA", "u32");
        let sub = ParsedSubrecord {
            signature: SmolStr::new("DATA"),
            data: Bytes::from(vec![1, 2, 3, 4]),
            semantic_type: None,
        };
        assert_eq!(subrecord_unused_byte_count(&sub, &spec), None);
    }

    #[test]
    fn unused_data_check_warns_when_payload_exceeds_declared_size() {
        let spec = spec_with_codec("XPDD", "f32"); // 4 bytes
        let sub = ParsedSubrecord {
            signature: SmolStr::new("XPDD"),
            data: Bytes::from(vec![0u8; 8]),
            semantic_type: None,
        };
        assert_eq!(subrecord_unused_byte_count(&sub, &spec), Some(4));
    }

    #[test]
    fn unused_data_check_no_warning_for_variable_length_codec() {
        let spec = spec_with_codec("EDID", "zstring");
        let sub = ParsedSubrecord {
            signature: SmolStr::new("EDID"),
            data: Bytes::from(b"FooBar\0".to_vec()),
            semantic_type: Some("zstring".to_string()),
        };
        assert_eq!(subrecord_unused_byte_count(&sub, &spec), None);
    }

    #[test]
    fn unused_data_check_handles_struct_codec_size() {
        // struct:I I f → 3*4 = 12 bytes.
        let spec = spec_with_codec("OBTS", "struct:I I f");
        let sub_exact = ParsedSubrecord {
            signature: SmolStr::new("OBTS"),
            data: Bytes::from(vec![0u8; 12]),
            semantic_type: None,
        };
        assert_eq!(subrecord_unused_byte_count(&sub_exact, &spec), None);

        let sub_over = ParsedSubrecord {
            signature: SmolStr::new("OBTS"),
            data: Bytes::from(vec![0u8; 16]),
            semantic_type: None,
        };
        assert_eq!(subrecord_unused_byte_count(&sub_over, &spec), Some(4));
    }

    #[test]
    fn order_check_passes_when_subrecords_match_schema() {
        let specs = vec![
            make_subrec_spec("EDID", false),
            make_subrec_spec("OBND", false),
            make_subrec_spec("FULL", false),
        ];
        let mut rec = record("ARMO", 0x0001_2345, Some("test"));
        rec.subrecords.push(empty_subrec("OBND"));
        rec.subrecords.push(empty_subrec("FULL"));
        let errors = subrecord_order_errors(&rec, &specs);
        assert!(errors.is_empty(), "got: {errors:?}");
    }

    #[test]
    fn order_check_flags_unknown_subrecord() {
        let specs = vec![
            make_subrec_spec("EDID", false),
            make_subrec_spec("OBND", false),
        ];
        let mut rec = record("CELL", 0x0001_2345, Some("test"));
        rec.subrecords.push(empty_subrec("XILS"));
        let errors = subrecord_order_errors(&rec, &specs);
        assert_eq!(errors.len(), 1);
        assert_eq!(
            errors[0],
            "Error: record CELL contains unexpected (or out of order) subrecord XILS 534C4958"
        );
    }

    #[test]
    fn order_check_accepts_land_multi_alpha_layers() {
        // LAND 'layers' is a repeating union: per quadrant a BTXT base plus N
        // ATXT+VTXT alpha layers. The schema lists BTXT/ATXT/VTXT once each
        // under scope_id "layers"; the walker must accept the repeats.
        let specs = vec![
            make_subrec_spec("DATA", false),
            make_subrec_spec("VNML", false),
            make_subrec_spec("VHGT", false),
            make_scoped_subrec_spec("BTXT", "layers"),
            make_scoped_subrec_spec("ATXT", "layers"),
            make_scoped_subrec_spec("VTXT", "layers"),
            make_subrec_spec("MPCD", true),
        ];
        let mut rec = record("LAND", 0x0001_2345, None);
        for sig in ["DATA", "VNML", "VHGT"] {
            rec.subrecords.push(empty_subrec(sig));
        }
        // Two quadrants, each: BTXT + 3x (ATXT, VTXT).
        for _quadrant in 0..2 {
            rec.subrecords.push(empty_subrec("BTXT"));
            for _layer in 0..3 {
                rec.subrecords.push(empty_subrec("ATXT"));
                rec.subrecords.push(empty_subrec("VTXT"));
            }
        }
        let errors = subrecord_order_errors(&rec, &specs);
        assert!(errors.is_empty(), "got: {errors:?}");
    }

    #[test]
    fn order_check_flags_out_of_order_subrecord() {
        let specs = vec![
            make_subrec_spec("EDID", false),
            make_subrec_spec("OBND", false),
            make_subrec_spec("FULL", false),
        ];
        let mut rec = record("ARMO", 0x0001_2345, Some("test"));
        rec.subrecords.push(empty_subrec("FULL"));
        rec.subrecords.push(empty_subrec("OBND")); // out of order
        let errors = subrecord_order_errors(&rec, &specs);
        assert_eq!(errors.len(), 1);
        assert_eq!(
            errors[0],
            "Error: record ARMO contains unexpected (or out of order) subrecord OBND 444E424F"
        );
    }

    #[test]
    fn order_check_allows_repeatable_subrecord() {
        let specs = vec![
            make_subrec_spec("EDID", false),
            make_subrec_spec("LVLO", true),
            make_subrec_spec("LLCT", false),
        ];
        let mut rec = record("LVLI", 0x0001_2345, Some("test"));
        rec.subrecords.push(empty_subrec("LVLO"));
        rec.subrecords.push(empty_subrec("LVLO"));
        rec.subrecords.push(empty_subrec("LVLO"));
        rec.subrecords.push(empty_subrec("LLCT"));
        let errors = subrecord_order_errors(&rec, &specs);
        assert!(errors.is_empty(), "got: {errors:?}");
    }

    #[test]
    fn order_check_allows_interleaved_scoped_segment() {
        let specs = vec![
            make_subrec_spec("EDID", false),
            make_scoped_subrec_spec("EFID", "effects"),
            make_scoped_subrec_spec("EFIT", "effects"),
        ];
        let mut rec = record("SPEL", 0x0001_2345, Some("test"));
        rec.subrecords.push(empty_subrec("EFID"));
        rec.subrecords.push(empty_subrec("EFIT"));
        rec.subrecords.push(empty_subrec("EFID"));
        rec.subrecords.push(empty_subrec("EFIT"));

        let errors = subrecord_order_errors(&rec, &specs);

        assert!(errors.is_empty(), "got: {errors:?}");
    }

    #[test]
    fn order_check_flags_deinterleaved_scoped_children() {
        let specs = vec![
            make_subrec_spec("EDID", false),
            make_scoped_subrec_spec("EFID", "effects"),
            make_scoped_subrec_spec("EFIT", "effects"),
        ];
        let mut rec = record("SPEL", 0x0001_2345, Some("test"));
        rec.subrecords.push(empty_subrec("EFID"));
        rec.subrecords.push(empty_subrec("EFID"));
        rec.subrecords.push(empty_subrec("EFIT"));
        rec.subrecords.push(empty_subrec("EFIT"));

        let errors = subrecord_order_errors(&rec, &specs);

        assert_eq!(errors.len(), 1);
        assert_eq!(
            errors[0],
            "Error: record SPEL contains unexpected (or out of order) subrecord EFIT 54494645"
        );
    }

    #[test]
    fn order_check_accepts_repeated_magic_effect_conditions() {
        let schema = crate::plugin_runtime::compiled_schema_for_game("fo4")
            .expect("fo4 schema must compile");

        for record_sig in ["ALCH", "ENCH", "SPEL"] {
            let specs = &schema
                .records
                .get(record_sig)
                .expect("fo4 schema must contain magic record")
                .subrecords;
            let mut rec = record(record_sig, 0x001F_2D3E, Some("test"));
            for sig in [
                "EFID", "EFIT", "CTDA", "CIS1", "CIS2", "CTDA", "CIS1", "CTDA", "CIS2", "EFID",
                "EFIT", "CTDA",
            ] {
                rec.subrecords.push(empty_subrec(sig));
            }

            let errors = subrecord_order_errors(&rec, specs);

            assert!(errors.is_empty(), "{record_sig}: {errors:?}");
        }
    }

    #[test]
    fn order_check_accepts_multiple_object_template_combinations() {
        // Object Template: OBTE count + RArray of [OBTF, FULL, OBTS] + STOP.
        // OBTF starts each combination yet is not the scope's first member.
        let specs = vec![
            make_subrec_spec("EDID", false),
            make_scoped_subrec_spec("OBTE", "object_template"),
            make_scoped_subrec_spec("OBTF", "object_template"),
            make_scoped_subrec_spec("FULL", "object_template"),
            make_scoped_subrec_spec("OBTS", "object_template"),
            make_scoped_subrec_spec("STOP", "object_template"),
        ];
        let mut rec = record("WEAP", 0x0001_2345, Some("test"));
        rec.subrecords.push(empty_subrec("OBTE"));
        for _combination in 0..2 {
            rec.subrecords.push(empty_subrec("OBTF"));
            rec.subrecords.push(empty_subrec("FULL"));
            rec.subrecords.push(empty_subrec("OBTS"));
        }
        rec.subrecords.push(empty_subrec("STOP"));
        let errors = subrecord_order_errors(&rec, &specs);
        assert!(errors.is_empty(), "got: {errors:?}");
    }

    #[test]
    fn order_check_accepts_repeated_qust_stage_logs_conditions_and_objective_targets() {
        let schema = crate::plugin_runtime::compiled_schema_for_game("fo4")
            .expect("fo4 schema must compile");
        let specs = &schema
            .records
            .get("QUST")
            .expect("fo4 schema must contain QUST")
            .subrecords;
        let mut rec = record("QUST", 0x0715_72E8, Some("vDialogueEDE"));

        for sig in [
            "FULL", "CTDA", "CTDA", "INDX", "QSDT", "CTDA", "CTDA", "CNAM", "QSDT", "CTDA", "CNAM",
            "QOBJ", "NNAM", "QSTA", "QSTA", "QOBJ", "NNAM", "QSTA",
        ] {
            rec.subrecords.push(empty_subrec(sig));
        }

        let errors = subrecord_order_errors(&rec, specs);

        assert!(errors.is_empty(), "got: {errors:?}");
    }

    #[test]
    fn order_check_rejects_qust_stage_condition_without_log_anchor() {
        let schema = crate::plugin_runtime::compiled_schema_for_game("fo4")
            .expect("fo4 schema must compile");
        let specs = &schema
            .records
            .get("QUST")
            .expect("fo4 schema must contain QUST")
            .subrecords;
        let mut rec = record("QUST", 0x0001_2345, Some("test"));
        for sig in ["INDX", "CTDA"] {
            rec.subrecords.push(empty_subrec(sig));
        }

        let errors = subrecord_order_errors(&rec, specs);

        assert_eq!(
            errors,
            vec![
                "Error: record QUST contains unexpected (or out of order) subrecord CTDA 41445443"
                    .to_string()
            ]
        );
    }

    #[test]
    fn order_check_rejects_qust_objective_condition_without_target_anchor() {
        let schema = crate::plugin_runtime::compiled_schema_for_game("fo4")
            .expect("fo4 schema must compile");
        let specs = &schema
            .records
            .get("QUST")
            .expect("fo4 schema must contain QUST")
            .subrecords;
        let mut rec = record("QUST", 0x0001_2345, Some("test"));
        for sig in ["QOBJ", "CTDA"] {
            rec.subrecords.push(empty_subrec(sig));
        }

        let errors = subrecord_order_errors(&rec, specs);

        assert_eq!(
            errors,
            vec![
                "Error: record QUST contains unexpected (or out of order) subrecord CTDA 41445443"
                    .to_string()
            ]
        );
    }

    #[test]
    fn order_check_rejects_qust_target_without_objective_anchor() {
        let schema = crate::plugin_runtime::compiled_schema_for_game("fo4")
            .expect("fo4 schema must compile");
        let specs = &schema
            .records
            .get("QUST")
            .expect("fo4 schema must contain QUST")
            .subrecords;
        let mut rec = record("QUST", 0x0001_2345, Some("test"));
        rec.subrecords.push(empty_subrec("QSTA"));

        let errors = subrecord_order_errors(&rec, specs);

        assert_eq!(
            errors,
            vec![
                "Error: record QUST contains unexpected (or out of order) subrecord QSTA 41545351"
                    .to_string()
            ]
        );
    }

    #[test]
    fn order_check_rejects_qust_stage_condition_after_log_text() {
        let schema = crate::plugin_runtime::compiled_schema_for_game("fo4")
            .expect("fo4 schema must compile");
        let specs = &schema
            .records
            .get("QUST")
            .expect("fo4 schema must contain QUST")
            .subrecords;
        let mut rec = record("QUST", 0x0001_2345, Some("test"));
        for sig in ["INDX", "QSDT", "CNAM", "CTDA"] {
            rec.subrecords.push(empty_subrec(sig));
        }

        let errors = subrecord_order_errors(&rec, specs);

        assert_eq!(
            errors,
            vec![
                "Error: record QUST contains unexpected (or out of order) subrecord CTDA 41445443"
                    .to_string()
            ]
        );
    }

    #[test]
    fn order_check_accepts_repeated_term_menu_item_conditions() {
        let schema = crate::plugin_runtime::compiled_schema_for_game("fo4")
            .expect("fo4 schema must compile");
        let specs = &schema
            .records
            .get("TERM")
            .expect("fo4 schema must contain TERM")
            .subrecords;
        let mut rec = record("TERM", 0x0716_6B54, Some("HVRamosTerminal"));

        for sig in [
            "OBND", "FULL", "MODL", "SNAM", "ITXT", "RNAM", "ANAM", "CTDA", "CTDA", "ITXT", "RNAM",
            "ANAM", "CTDA",
        ] {
            rec.subrecords.push(empty_subrec(sig));
        }

        let errors = subrecord_order_errors(&rec, specs);

        assert!(errors.is_empty(), "got: {errors:?}");
    }

    #[test]
    fn order_check_accepts_repeated_destruction_stages() {
        let schema = crate::plugin_runtime::compiled_schema_for_game("fo4")
            .expect("fo4 schema must compile");
        let specs = &schema
            .records
            .get("ACTI")
            .expect("fo4 schema must contain ACTI")
            .subrecords;
        let mut rec = record("ACTI", 0x0714_B67F, Some("VHDLegionOliverGenerator01"));

        for sig in [
            "OBND", "FULL", "MODL", "DEST", "DSTD", "DSTF", "DSTD", "DSTF", "DSTD", "DMDL", "DSTF",
            "DSTD", "DSTF",
        ] {
            rec.subrecords.push(empty_subrec(sig));
        }

        let errors = subrecord_order_errors(&rec, specs);

        assert!(errors.is_empty(), "got: {errors:?}");
    }

    #[test]
    fn order_check_still_rejects_destruction_stage_without_dest_anchor() {
        let schema = crate::plugin_runtime::compiled_schema_for_game("fo4")
            .expect("fo4 schema must compile");
        let specs = &schema
            .records
            .get("ACTI")
            .expect("fo4 schema must contain ACTI")
            .subrecords;
        let mut rec = record("ACTI", 0x0001_2345, Some("test"));
        rec.subrecords.push(empty_subrec("DSTD"));

        let errors = subrecord_order_errors(&rec, specs);

        assert_eq!(
            errors,
            vec![
                "Error: record ACTI contains unexpected (or out of order) subrecord DSTD 44545344"
                    .to_string()
            ]
        );
    }

    #[test]
    fn order_check_accepts_race_transition_between_body_and_behavior_scopes() {
        let schema = crate::plugin_runtime::compiled_schema_for_game("fo4")
            .expect("fo4 schema must compile");
        let specs = &schema
            .records
            .get("RACE")
            .expect("fo4 schema must contain RACE")
            .subrecords;
        let mut rec = record("RACE", 0x0709_87DF, Some("CaucasianOldAged"));

        for sig in [
            "FULL", "DESC", "MNAM", "MODT", "FNAM", "MODT", "VTCK", "PNAM", "UNAM", "NAM1", "MNAM",
            "INDX", "MODL", "MODT", "FNAM", "INDX", "MODL", "MODT", "MNAM", "MODL", "MODT", "FNAM",
            "MODL", "MODT", "CNAM",
        ] {
            rec.subrecords.push(empty_subrec(sig));
        }

        let errors = subrecord_order_errors(&rec, specs);

        assert!(errors.is_empty(), "got: {errors:?}");
    }

    #[test]
    fn order_check_rejects_generic_scoped_recurrence_despite_later_duplicate_slot() {
        let specs = vec![
            make_scoped_subrec_spec("ANCH", "rows"),
            make_scoped_subrec_spec("DUPL", "rows"),
            make_scoped_subrec_spec("TAIL", "rows"),
            make_subrec_spec("DUPL", false),
        ];
        let mut rec = record("TEST", 0x0001_2345, None);
        for sig in ["ANCH", "DUPL", "DUPL"] {
            rec.subrecords.push(empty_subrec(sig));
        }

        let errors = subrecord_order_errors(&rec, &specs);

        assert_eq!(
            errors,
            vec![
                "Error: record TEST contains unexpected (or out of order) subrecord DUPL 4C505544"
                    .to_string()
            ]
        );
    }

    #[test]
    fn order_check_rejects_malformed_race_body_model_recurrence() {
        let schema = crate::plugin_runtime::compiled_schema_for_game("fo4")
            .expect("fo4 schema must compile");
        let specs = &schema
            .records
            .get("RACE")
            .expect("fo4 schema must contain RACE")
            .subrecords;
        let mut rec = record("RACE", 0x0001_2345, Some("test"));
        for sig in ["NAM1", "MNAM", "INDX", "MODL", "MODL"] {
            rec.subrecords.push(empty_subrec(sig));
        }

        let errors = subrecord_order_errors(&rec, specs);

        assert_eq!(
            errors,
            vec![
                "Error: record RACE contains unexpected (or out of order) subrecord MODL 4C444F4D"
                    .to_string()
            ]
        );
    }

    #[test]
    fn order_check_flags_scoped_child_without_anchor() {
        let specs = vec![
            make_subrec_spec("EDID", false),
            make_scoped_subrec_spec("INDX", "stages"),
            make_scoped_subrec_spec("QSDT", "stages"),
        ];
        let mut rec = record("QUST", 0x0001_2345, Some("test"));
        rec.subrecords.push(empty_subrec("QSDT"));

        let errors = subrecord_order_errors(&rec, &specs);

        assert_eq!(errors.len(), 1);
        assert_eq!(
            errors[0],
            "Error: record QUST contains unexpected (or out of order) subrecord QSDT 54445351"
        );
    }

    #[test]
    fn order_check_uses_later_unscoped_duplicate_after_skipped_scope() {
        let specs = vec![
            make_subrec_spec("EDID", false),
            make_subrec_spec("AIDT", false),
            make_scoped_subrec_spec("OBTE", "object_template"),
            make_scoped_subrec_spec("FULL", "object_template"),
            make_subrec_spec("CNAM", false),
            make_subrec_spec("FULL", false),
            make_subrec_spec("DATA", false),
        ];
        let mut rec = record("NPC_", 0x0001_2345, Some("test"));
        rec.subrecords.push(empty_subrec("AIDT"));
        rec.subrecords.push(empty_subrec("FULL"));
        rec.subrecords.push(empty_subrec("DATA"));

        let errors = subrecord_order_errors(&rec, &specs);

        assert!(errors.is_empty(), "got: {errors:?}");
    }

    #[test]
    fn fo4_qust_batched_alias_fields_match_xedit_errors() {
        let schema = crate::plugin_runtime::compiled_schema_for_game("fo4")
            .expect("fo4 schema must compile");
        let specs = &schema
            .records
            .get("QUST")
            .expect("fo4 schema must contain QUST")
            .subrecords;
        let mut rec = record("QUST", 0x0700_8E69, Some("RE_ObjectKMK01"));

        for sig in ["VMAD", "FULL", "ENAM", "FLTR", "NEXT", "ANAM"] {
            rec.subrecords.push(empty_subrec(sig));
        }
        for _ in 0..6 {
            rec.subrecords.push(empty_subrec("ALST"));
        }
        for _ in 0..4 {
            rec.subrecords.push(empty_subrec("ALCO"));
        }
        for _ in 0..4 {
            rec.subrecords.push(empty_subrec("ALCL"));
        }
        for _ in 0..4 {
            rec.subrecords.push(empty_subrec("ALNT"));
        }
        for _ in 0..4 {
            rec.subrecords.push(empty_subrec("ALLA"));
        }
        for _ in 0..2 {
            rec.subrecords.push(empty_subrec("ALDN"));
        }
        rec.subrecords.push(empty_subrec("ALPC"));
        for _ in 0..3 {
            rec.subrecords.push(empty_subrec("VTCK"));
        }
        rec.subrecords.push(empty_subrec("ALCS"));
        rec.subrecords.push(empty_subrec("ALMI"));

        let errors = subrecord_order_errors(&rec, specs);

        let expected = vec![
            "Error: record QUST contains unexpected (or out of order) subrecord ALDN 4E444C41"
                .to_string(),
            "Error: record QUST contains unexpected (or out of order) subrecord ALDN 4E444C41"
                .to_string(),
            "Error: record QUST contains unexpected (or out of order) subrecord ALPC 43504C41"
                .to_string(),
            "Error: record QUST contains unexpected (or out of order) subrecord VTCK 4B435456"
                .to_string(),
            "Error: record QUST contains unexpected (or out of order) subrecord VTCK 4B435456"
                .to_string(),
            "Error: record QUST contains unexpected (or out of order) subrecord VTCK 4B435456"
                .to_string(),
            "Error: record QUST contains unexpected (or out of order) subrecord ALCS 53434C41"
                .to_string(),
            "Error: record QUST contains unexpected (or out of order) subrecord ALMI 494D4C41"
                .to_string(),
        ];
        assert_eq!(errors, expected);
    }

    #[test]
    fn order_check_qust_reference_alias_scope_resets_batched_state() {
        let schema = crate::plugin_runtime::compiled_schema_for_game("fo4")
            .expect("fo4 schema must compile");
        let specs = &schema
            .records
            .get("QUST")
            .expect("fo4 schema must contain QUST")
            .subrecords;
        let mut rec = record("QUST", 0x0001_2345, Some("test"));

        for sig in ["ALST", "ALST", "ALCO", "ALCO", "ALST", "ALDN"] {
            rec.subrecords.push(empty_subrec(sig));
        }

        let errors = subrecord_order_errors(&rec, specs);

        assert!(errors.is_empty(), "got: {errors:?}");
    }

    #[test]
    fn order_check_accepts_repeated_qust_alias_conditions_with_cis_children() {
        let schema = crate::plugin_runtime::compiled_schema_for_game("fo4")
            .expect("fo4 schema must compile");
        let specs = &schema
            .records
            .get("QUST")
            .expect("fo4 schema must contain QUST")
            .subrecords;
        let mut rec = record("QUST", 0x0001_2345, Some("test"));

        for sig in [
            "ALST", "CTDA", "CIS1", "CIS2", "CTDA", "CIS1", "CIS2", "ALDN",
        ] {
            rec.subrecords.push(empty_subrec(sig));
        }

        let errors = subrecord_order_errors(&rec, specs);

        assert!(errors.is_empty(), "got: {errors:?}");
    }

    #[test]
    fn order_check_rejects_qust_alias_condition_after_later_alias_field() {
        let schema = crate::plugin_runtime::compiled_schema_for_game("fo4")
            .expect("fo4 schema must compile");
        let specs = &schema
            .records
            .get("QUST")
            .expect("fo4 schema must contain QUST")
            .subrecords;
        let mut rec = record("QUST", 0x0001_2345, Some("test"));

        for sig in ["ALST", "CTDA", "ALDN", "CTDA"] {
            rec.subrecords.push(empty_subrec(sig));
        }

        let errors = subrecord_order_errors(&rec, specs);

        assert_eq!(
            errors,
            vec![
                "Error: record QUST contains unexpected (or out of order) subrecord CTDA 41445443"
                    .to_string()
            ]
        );
    }

    #[test]
    fn order_check_qust_can_recover_to_different_alias_anchor_after_batched_reference() {
        let schema = crate::plugin_runtime::compiled_schema_for_game("fo4")
            .expect("fo4 schema must compile");
        let specs = &schema
            .records
            .get("QUST")
            .expect("fo4 schema must contain QUST")
            .subrecords;
        let mut rec = record("QUST", 0x0001_2345, Some("test"));

        for sig in [
            "ALST", "ALST", "ALCO", "ALCO", "ALLS", "ALID", "ALCS", "ALMI",
        ] {
            rec.subrecords.push(empty_subrec(sig));
        }

        let errors = subrecord_order_errors(&rec, specs);

        assert!(errors.is_empty(), "got: {errors:?}");
    }

    fn fake_schema_for_cell() -> CompiledSchema {
        use crate::plugin_runtime::SchemaRecordJson;
        use std::collections::HashMap;

        let mut records = HashMap::new();
        records.insert(
            "CELL".to_string(),
            SchemaRecordJson {
                id: "CELL".to_string(),
                subrecords: vec![
                    make_subrec_spec("EDID", false),
                    make_subrec_spec("FULL", false),
                    make_subrec_spec("DATA", false),
                ],
                ..Default::default()
            },
        );
        records.insert(
            "REFR".to_string(),
            SchemaRecordJson {
                id: "REFR".to_string(),
                subrecords: vec![
                    make_subrec_spec("EDID", false),
                    make_subrec_spec("NAME", false),
                    spec_with_codec("XPDD", "f32"), // 4-byte declared
                ],
                ..Default::default()
            },
        );
        CompiledSchema {
            records,
            enums: HashMap::new(),
        }
    }

    #[test]
    fn walk_and_check_fires_cell_unexpected_subrecord() {
        let mut cell = record("CELL", 0x0126_28FE, Some("OriginExt"));
        cell.subrecords.push(empty_subrec("XILS"));

        let plugin = ParsedPlugin {
            plugin_name: "p.esp".to_string(),
            file_path: String::new(),
            header_size: 0,
            header: ParsedPluginHeader::default_for_test(),
            root_items: vec![ParsedItem::Group(ParsedGroup {
                label: *b"CELL",
                group_type: 0,
                tail: Bytes::new(),
                children: vec![ParsedItem::Record(cell)],
            })],
            game: None,
        };
        let schema = fake_schema_for_cell();
        let messages = walk_and_check_for_test(&plugin, 1, &schema);
        assert!(
            messages.iter().any(|m| m == "Error: record CELL contains unexpected (or out of order) subrecord XILS 534C4958"),
            "got messages: {messages:?}"
        );
    }

    #[test]
    fn walk_and_check_fires_refr_xpdd_unused_data() {
        let mut refr = record("REFR", 0x0131_B717, None);
        refr.subrecords.push(empty_subrec("EDID"));
        refr.subrecords.push(empty_subrec("NAME"));
        refr.subrecords.push(ParsedSubrecord {
            signature: SmolStr::new("XPDD"),
            data: Bytes::from(vec![0u8; 8]), // 4 declared + 4 trailing
            semantic_type: None,
        });

        let plugin = ParsedPlugin {
            plugin_name: "p.esp".to_string(),
            file_path: String::new(),
            header_size: 0,
            header: ParsedPluginHeader::default_for_test(),
            root_items: vec![ParsedItem::Group(ParsedGroup {
                label: *b"REFR",
                group_type: 0,
                tail: Bytes::new(),
                children: vec![ParsedItem::Record(refr)],
            })],
            game: None,
        };
        let schema = fake_schema_for_cell();
        let messages = walk_and_check_for_test(&plugin, 1, &schema);
        let xpdd_warning = messages
            .iter()
            .find(|m| m.contains("Unused data in") && m.contains("XPDD"));
        assert!(xpdd_warning.is_some(), "got: {messages:?}");
        let msg = xpdd_warning.unwrap();
        assert!(msg.starts_with("<Warning: Unused data in:"));
        assert!(msg.ends_with(">"));
        assert!(msg.contains("[REFR:0131B717]"));
        assert!(msg.contains("XPDD"));
    }

    // ----- D / A2 / A1 parity helpers + byte-exact cases (task #7 step 1+2) -----

    use crate::plugin_runtime::{SchemaEnumJson, SchemaEnumValueJson, SchemaFieldJson};
    use std::collections::HashMap;

    fn enum_value(value: i128, id: &str) -> SchemaEnumValueJson {
        SchemaEnumValueJson {
            value,
            id: id.to_string(),
        }
    }

    fn make_enum(
        id: &str,
        storage_kind: &str,
        byte_width: usize,
        values: Vec<SchemaEnumValueJson>,
    ) -> SchemaEnumJson {
        SchemaEnumJson {
            id: id.to_string(),
            values,
            storage_kind: storage_kind.to_string(),
            byte_width,
            ..Default::default()
        }
    }

    fn enum_subrec_spec(id: &str, codec: &str, enum_ref: &str) -> SchemaSubrecordJson {
        // Real subrecord-level enums (KYWD.TNAM etc.) carry a scalar codec + a
        // single field holding the enum_ref; struct_field_layout lays out that
        // field, which is what struct_member_errors reads.
        let mut s = spec_with_codec(id, codec);
        s.enum_ref = Some(enum_ref.to_string());
        s.fields = vec![enum_field("value", codec, Some(enum_ref))];
        s
    }

    fn subrec_with_data(sig: &str, data: Vec<u8>) -> ParsedSubrecord {
        ParsedSubrecord {
            signature: SmolStr::new(sig),
            data: Bytes::from(data),
            semantic_type: None,
        }
    }

    // ---- struct_member_errors: A1 (flags) / A2 (enums) / D (short) ----
    // These exercise the keystone struct_field_layout path end-to-end.

    fn enum_field(id: &str, kind: &str, enum_ref: Option<&str>) -> SchemaFieldJson {
        SchemaFieldJson {
            id: id.to_string(),
            kind: kind.to_string(),
            enum_ref: enum_ref.map(str::to_string),
            ..Default::default()
        }
    }

    /// Build a CompiledSchema with one record -> one subrecord (given codec +
    /// fields) plus the supplied enums, for struct_member_errors tests.
    fn member_schema(
        record_sig: &str,
        sub_sig: &str,
        codec: &str,
        fields: Vec<SchemaFieldJson>,
        enums: Vec<SchemaEnumJson>,
    ) -> CompiledSchema {
        let mut sub = spec_with_codec(sub_sig, codec);
        sub.fields = fields;
        let mut records = HashMap::new();
        records.insert(
            record_sig.to_string(),
            SchemaRecordJson {
                id: record_sig.to_string(),
                subrecords: vec![sub],
                ..Default::default()
            },
        );
        let mut emap = HashMap::new();
        for e in enums {
            emap.insert(e.id.clone(), e);
        }
        CompiledSchema {
            records,
            enums: emap,
        }
    }

    fn keyword_type_enum_0_18() -> SchemaEnumJson {
        let values = (0..=18)
            .map(|v| enum_value(v, &format!("kw_{v}")))
            .collect();
        make_enum("keyword_type_enum", "enum", 4, values)
    }

    fn book_dnam_flags_0_4() -> SchemaEnumJson {
        make_enum(
            "book_dnam_flags",
            "flags",
            4,
            vec![
                enum_value(1, "b0"),
                enum_value(2, "b1"),
                enum_value(4, "b2"),
                enum_value(8, "b3"),
                enum_value(16, "b4"),
            ],
        )
    }

    // ---- Class A2: unknown scalar enum value (subrecord-level + struct-nested) ----

    #[test]
    fn member_enum_unknown_kywd_tnam_24() {
        // "KYWD \ TNAM - Type -> <Unknown: 24 $18>". TNAM = uint32 single enum field.
        let schema = member_schema(
            "KYWD",
            "TNAM",
            "uint32",
            vec![enum_field("type", "uint32", Some("keyword_type_enum"))],
            vec![keyword_type_enum_0_18()],
        );
        let sub = subrec_with_data("TNAM", 24u32.to_le_bytes().to_vec());
        let msgs: Vec<_> = struct_member_errors("KYWD", &sub, None, &schema)
            .into_iter()
            .map(|m| (m.category, m.message))
            .collect();
        assert_eq!(
            msgs,
            vec![("unknown_enum", "<Unknown: 24 $18>".to_string())]
        );
    }

    #[test]
    fn member_enum_known_value_no_error() {
        let schema = member_schema(
            "KYWD",
            "TNAM",
            "uint32",
            vec![enum_field("type", "uint32", Some("keyword_type_enum"))],
            vec![keyword_type_enum_0_18()],
        );
        let sub = subrec_with_data("TNAM", 6u32.to_le_bytes().to_vec());
        assert!(struct_member_errors("KYWD", &sub, None, &schema).is_empty());
    }

    #[test]
    fn member_enum_nested_struct_field_offset() {
        // A2 on a NESTED struct field: struct:I,I with the enum on the SECOND
        // field (offset 4) — proves we read at the field's offset, not 0.
        let schema = member_schema(
            "REC",
            "DNAM",
            "struct:I,I",
            vec![
                enum_field("first", "uint32", None),
                enum_field("type", "uint32", Some("keyword_type_enum")),
            ],
            vec![keyword_type_enum_0_18()],
        );
        // first=6 (would be "known" if read here), second=24 (unknown).
        let mut data = 6u32.to_le_bytes().to_vec();
        data.extend_from_slice(&24u32.to_le_bytes());
        let sub = subrec_with_data("DNAM", data);
        let msgs: Vec<_> = struct_member_errors("REC", &sub, None, &schema)
            .into_iter()
            .map(|m| (m.category, m.message))
            .collect();
        assert_eq!(
            msgs,
            vec![("unknown_enum", "<Unknown: 24 $18>".to_string())]
        );
    }

    // ---- Class A1: nested struct-field flags (the 8,896-error case) ----

    #[test]
    fn member_flag_book_dnam_nested_bit5() {
        // BOOK.DNAM struct:B,I,I,I, flags field at offset 0 (the real BOOK.DNAM).
        // FO76 sets bit 5 ($20) -> "<Unknown: 5 $5>".
        let schema = member_schema(
            "BOOK",
            "DNAM",
            "struct:B,I,I,I",
            vec![
                enum_field("flags", "uint8", Some("book_dnam_flags")),
                enum_field("teaches", "uint32", None),
                enum_field("ox", "uint32", None),
                enum_field("oy", "uint32", None),
            ],
            vec![book_dnam_flags_0_4()],
        );
        let mut data = vec![0x20u8]; // flags byte: bit 5
        data.extend_from_slice(&[0u8; 12]); // teaches + ox + oy
        let sub = subrec_with_data("DNAM", data);
        let msgs: Vec<_> = struct_member_errors("BOOK", &sub, None, &schema)
            .into_iter()
            .map(|m| (m.category, m.message))
            .collect();
        assert_eq!(msgs, vec![("unknown_flag", "<Unknown: 5 $5>".to_string())]);
    }

    #[test]
    fn member_flag_multi_bit_comma_joined() {
        // RACE DATA Flags2-shape: u32 flags field, bits 25/26/27 unknown.
        let schema = member_schema(
            "RACE",
            "DATA",
            "uint32",
            vec![enum_field("flags", "uint32", Some("f"))],
            vec![make_enum("f", "flags", 4, vec![enum_value(1, "b0")])],
        );
        let value = (1u32 << 25) | (1u32 << 26) | (1u32 << 27);
        let sub = subrec_with_data("DATA", value.to_le_bytes().to_vec());
        let msgs: Vec<_> = struct_member_errors("RACE", &sub, None, &schema)
            .into_iter()
            .map(|m| m.message)
            .collect();
        assert_eq!(
            msgs,
            vec!["<Unknown: 25 $19>, <Unknown: 26 $1A>, <Unknown: 27 $1B>".to_string()]
        );
    }

    #[test]
    fn member_flag_known_bits_no_error() {
        let schema = member_schema(
            "REC",
            "DNAM",
            "uint32",
            vec![enum_field("flags", "uint32", Some("f"))],
            vec![make_enum(
                "f",
                "flags",
                4,
                vec![
                    enum_value(1, "b0"),
                    enum_value(2, "b1"),
                    enum_value(4, "b2"),
                ],
            )],
        );
        let sub = subrec_with_data("DNAM", 0x7u32.to_le_bytes().to_vec());
        assert!(struct_member_errors("REC", &sub, None, &schema).is_empty());
    }

    // ---- Class D: short struct member (SNDR.BNAM "Static Attenuation" shape) ----

    #[test]
    fn member_data_size_short_struct_member() {
        // struct:H,H — a 2-byte member at offset 2 with only the first present
        // -> "Expected 2 bytes of data, found 0" for the second member.
        let schema = member_schema(
            "SNDR",
            "BNAM",
            "struct:H,H",
            vec![
                enum_field("first", "uint16", None),
                enum_field("static_attenuation", "uint16", None),
            ],
            vec![],
        );
        let sub = subrec_with_data("BNAM", vec![0, 0]); // only first member present
        let msgs: Vec<_> = struct_member_errors("SNDR", &sub, None, &schema)
            .into_iter()
            .map(|m| (m.category, m.message))
            .collect();
        assert_eq!(
            msgs,
            vec![("data_size", "Expected 2 bytes of data, found 0".to_string())]
        );
    }

    #[test]
    fn member_data_size_no_error_when_full() {
        let schema = member_schema(
            "SNDR",
            "BNAM",
            "struct:H,H",
            vec![
                enum_field("first", "uint16", None),
                enum_field("static_attenuation", "uint16", None),
            ],
            vec![],
        );
        let sub = subrec_with_data("BNAM", vec![0, 0, 0, 0]);
        assert!(struct_member_errors("SNDR", &sub, None, &schema).is_empty());
    }

    #[test]
    fn unknown_int_string_zero_has_no_hex() {
        // xEdit IntToHex(0).TrimLeft(['0']) == "" ⇒ no $hex segment.
        assert_eq!(unknown_int_string(0), "<Unknown: 0>");
        assert_eq!(unknown_int_string(11), "<Unknown: 11 $B>");
        assert_eq!(unknown_int_string(27), "<Unknown: 27 $1B>");
    }

    // ---- Class A1: record-header flags ----

    fn record_spec_with_flags(sig: &str, valid_mask: u32, permissive: bool) -> SchemaRecordJson {
        SchemaRecordJson {
            id: sig.to_string(),
            record_flags: Some(SchemaRecordFlagsJson {
                valid_mask,
                permissive,
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn record_flag_unknown_kywd_bit11() {
        // xedit_errors.txt: "KYWD \ Record Header \ Record Flags -> <Unknown: 11 $B>".
        // KYWD valid_mask 0x9020 (bits 5,12,15). FO76 sets bit 11 ($800).
        let spec = record_spec_with_flags("KYWD", 0x9020, false);
        let mut rec = record("KYWD", 0x078B_0962, None);
        rec.flags = 0x0000_0800; // bit 11
        assert_eq!(
            record_flag_error(&rec, &spec).as_deref(),
            Some("<Unknown: 11 $B>")
        );
    }

    #[test]
    fn record_flag_valid_bits_no_error() {
        let spec = record_spec_with_flags("KYWD", 0x9020, false);
        let mut rec = record("KYWD", 0x1, None);
        rec.flags = 0x0000_8020; // bits 5 + 15 — both valid
        assert_eq!(record_flag_error(&rec, &spec), None);
    }

    #[test]
    fn record_flag_permissive_never_errors() {
        let spec = record_spec_with_flags("REFR", 0x0000_0020, true);
        let mut rec = record("REFR", 0x1, None);
        rec.flags = 0xFFFF_FFFF;
        assert_eq!(record_flag_error(&rec, &spec), None);
    }

    #[test]
    fn record_flag_none_metadata_no_error() {
        // record_flags() == None ⇒ warn-only contract ⇒ no emission.
        let spec = SchemaRecordJson {
            id: "REFR".to_string(),
            ..Default::default()
        };
        let mut rec = record("REFR", 0x1, None);
        rec.flags = 0xFFFF_FFFF;
        assert_eq!(record_flag_error(&rec, &spec), None);
    }

    // ---- End-to-end through the walker ----

    fn schema_with_record_and_enum(
        record_sig: &str,
        subrecs: Vec<SchemaSubrecordJson>,
        enum_def: SchemaEnumJson,
    ) -> CompiledSchema {
        let mut records = HashMap::new();
        records.insert(
            record_sig.to_string(),
            SchemaRecordJson {
                id: record_sig.to_string(),
                subrecords: subrecs,
                ..Default::default()
            },
        );
        let mut enums = HashMap::new();
        enums.insert(enum_def.id.clone(), enum_def);
        CompiledSchema { records, enums }
    }

    #[test]
    fn walk_and_check_fires_kywd_tnam_unknown_enum() {
        let mut tnam_spec = enum_subrec_spec("TNAM", "uint32", "keyword_type_enum");
        tnam_spec.display_label = Some("Type".to_string());
        let schema = schema_with_record_and_enum(
            "KYWD",
            vec![make_subrec_spec("EDID", false), tnam_spec],
            keyword_type_enum_0_18(),
        );

        let mut kywd = record("KYWD", 0x078B_0962, Some("Scrap_Ball_PTS"));
        kywd.subrecords
            .push(subrec_with_data("TNAM", 24u32.to_le_bytes().to_vec()));

        let plugin = ParsedPlugin {
            plugin_name: "p.esp".to_string(),
            file_path: String::new(),
            header_size: 0,
            header: ParsedPluginHeader::default_for_test(),
            root_items: vec![ParsedItem::Group(ParsedGroup {
                label: *b"KYWD",
                group_type: 0,
                tail: Bytes::new(),
                children: vec![ParsedItem::Record(kywd)],
            })],
            game: None,
        };
        let messages = walk_and_check_for_test(&plugin, 1, &schema);
        assert!(
            messages.iter().any(|m| m == "<Unknown: 24 $18>"),
            "got: {messages:?}"
        );
    }
}
