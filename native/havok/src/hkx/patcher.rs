use crate::error::{HavokError, HavokResult};

use super::model::{ArraySource, HkxFile, HkxMember};
use super::types::{HkxType, HkxTypeFamily, HkxValue};
use super::writer::serialize_value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchRange {
    pub offset: usize,
    pub expected_len: usize,
    pub replacement: Vec<u8>,
}

impl PatchRange {
    pub fn new(offset: usize, expected_len: usize, replacement: impl Into<Vec<u8>>) -> Self {
        Self {
            offset,
            expected_len,
            replacement: replacement.into(),
        }
    }
}

/// Produce a patched packfile by overlaying current model values onto
/// the original source bytes.
///
/// Walks every array tracked by the reader (`hkx.array_sources()`),
/// re-serializes its current contents, and overlays them at the original byte
/// range. Struct-element arrays are skipped (their layout needs the writer);
/// the nested arrays inside them have their own tracking entries.
///
/// The result differs from the source only where the model changed. Returns
/// `HavokError::InvalidInput` when an array's serialized length no longer
/// matches the source length, e.g. because the caller resized it.
pub fn patch_hkx(hkx: &HkxFile) -> HavokResult<Vec<u8>> {
    let source = hkx.source_bytes();
    if source.is_empty() {
        return Err(HavokError::InvalidInput(
            "patch_hkx: HkxFile has no source bytes (not loaded from a packfile)".to_string(),
        ));
    }
    let mut buf = source.to_vec();
    for entry in hkx.array_sources() {
        patch_one(&mut buf, hkx, entry)?;
    }
    Ok(buf)
}

fn patch_one(buf: &mut [u8], hkx: &HkxFile, entry: &ArraySource) -> HavokResult<()> {
    // Skip struct-element arrays — the per-element nested arrays inside them
    // are tracked separately and patched independently. Strings/pointers are
    // also skipped (re-serializing them would shift the string-data tail or
    // require a fresh global-fixup table).
    let family = entry.element_subtype.family();
    if !matches!(family, HkxTypeFamily::Direct | HkxTypeFamily::Complex) {
        return Ok(());
    }

    let Some(value) = resolve_array_value(hkx, entry) else {
        // Array vanished — nothing to patch (model edits don't currently
        // remove tracked arrays, but tolerate it so callers can prune freely).
        return Ok(());
    };
    let HkxValue::Array(items) = value else {
        return Err(HavokError::InvalidInput(format!(
            "patch_hkx: expected array at {:?} in object {}",
            entry.member_path, entry.object_index
        )));
    };

    let elem_size = entry.element_subtype.size().max(1);
    let mut new_bytes = Vec::with_capacity(elem_size * items.len());
    for item in items {
        new_bytes.extend_from_slice(&serialize_value(entry.element_subtype, HkxType::Void, item));
    }
    if new_bytes.len() != entry.content_length {
        return Err(HavokError::InvalidInput(format!(
            "patch_hkx: array {:?} (object {}): serialized length {} != source length {} — array length changed; cannot patch, use full writer instead",
            entry.member_path,
            entry.object_index,
            new_bytes.len(),
            entry.content_length
        )));
    }
    let end = entry.content_offset + entry.content_length;
    if end > buf.len() {
        return Err(HavokError::InvalidInput(format!(
            "patch_hkx: array {:?} (object {}): content range {:#x}..{:#x} outside source bytes ({})",
            entry.member_path,
            entry.object_index,
            entry.content_offset,
            end,
            buf.len()
        )));
    }
    if buf[entry.content_offset..end] != new_bytes[..] {
        buf[entry.content_offset..end].copy_from_slice(&new_bytes);
    }
    Ok(())
}

fn resolve_array_value<'a>(hkx: &'a HkxFile, entry: &ArraySource) -> Option<&'a HkxValue> {
    let object = hkx.objects().get(entry.object_index)?;
    let mut value: Option<&HkxValue> = None;
    let mut members: Option<&[HkxMember]> = Some(&object.members);

    for step in &entry.member_path {
        if let Some(idx) = parse_index_step(step) {
            // `[index]` step: descend into an array element.
            let Some(HkxValue::Array(items)) = value else {
                return None;
            };
            let item = items.get(idx)?;
            value = Some(item);
            members = match item {
                HkxValue::Object(m) => Some(m.as_slice()),
                _ => None,
            };
            continue;
        }
        // Named-member step: look up by name in the current member list.
        let m = members?.iter().find(|m| m.name == *step)?;
        value = Some(&m.value);
        members = match &m.value {
            HkxValue::Object(nested) => Some(nested.as_slice()),
            _ => None,
        };
    }
    value
}

fn parse_index_step(step: &str) -> Option<usize> {
    let stripped = step.strip_prefix('[')?.strip_suffix(']')?;
    stripped.parse::<usize>().ok()
}

pub(crate) fn apply_patch_range(source_bytes: &mut [u8], patch: PatchRange) -> HavokResult<bool> {
    if patch.replacement.len() != patch.expected_len {
        return Err(HavokError::InvalidInput(format!(
            "patch replacement must be the same length as the target range: expected {} bytes, got {}",
            patch.expected_len,
            patch.replacement.len()
        )));
    }

    let end = patch
        .offset
        .checked_add(patch.expected_len)
        .ok_or_else(|| HavokError::InvalidInput("patch range overflows".to_string()))?;
    if end > source_bytes.len() {
        return Err(HavokError::InvalidInput(format!(
            "patch range {:#x}..{:#x} is outside source bytes of length {:#x}",
            patch.offset,
            end,
            source_bytes.len()
        )));
    }

    let changed = source_bytes[patch.offset..end] != patch.replacement;
    if changed {
        source_bytes[patch.offset..end].copy_from_slice(&patch.replacement);
    }
    Ok(changed)
}
