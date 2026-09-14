// Some constants and helpers (LT_TYPE_*, read_u8/read_u32, internal
// class/field structs) are only referenced on certain paths; allow
// dead_code rather than gating each one.
#![allow(dead_code)]

//! Tagfile2014 binary stream reader, plus a narrow writer for simple graphs.
//!
//! Used by FO4 inline animation blobs (BSBound / cloth setup nested in NIFs)
//! and Skyrim SE `.hkt` skeletons. Magic `0xCAB00D1E` `0xD011FACE`, then a
//! stream of VLE-tagged records (TAG_FILE_INFO, TAG_METADATA, TAG_OBJECT,
//! TAG_OBJECT_REMEMBER, TAG_OBJECT_NULL, TAG_FILE_END); unrelated to TAG0's
//! section-based HFF layout. Follows SDK Format/Tagfile2014/
//! hkTagfileReadFormat2014.cpp and hkTagfileCommon2014.h (no SDK code copied).

use crate::error::{HavokError, HavokResult};

use super::model::{HkxFile, HkxMember, HkxObject};
use super::types::HkxValue;

pub const BINARY_MAGIC_0: u32 = 0xCAB0_0D1E;
pub const BINARY_MAGIC_1: u32 = 0xD011_FACE;

/// Compatibility marker used by the repository's earlier synthetic fixtures.
/// Real HCT 2014 stream tagfiles do not store this fixed word after the magic;
/// their first byte at offset 8 is the VLE-encoded TAG_FILE_INFO record.
pub const TAGFILE_VERSION_2014_2: u32 = 13;

/// Sniff a buffer for the binary tagfile v13 magic in either endianness.
/// Used by `py_creation_lib/python/creation_lib/hkxpack/__init__.py::detect_format` and by the native
/// `api::hkx_detect_format_full` "binary_tagfile" branch — but those callers
/// already dispatch on byte ranges, so this helper is provided for unit
/// tests and future native-side routing of `binary_tagfile` to this reader.
pub fn is_binary_tagfile_magic(data: &[u8]) -> bool {
    if data.len() < 8 {
        return false;
    }
    let m0_le = u32::from_le_bytes(data[0..4].try_into().expect("4 bytes"));
    let m1_le = u32::from_le_bytes(data[4..8].try_into().expect("4 bytes"));
    if m0_le == BINARY_MAGIC_0 && m1_le == BINARY_MAGIC_1 {
        return true;
    }
    let m0_be = u32::from_be_bytes(data[0..4].try_into().expect("4 bytes"));
    let m1_be = u32::from_be_bytes(data[4..8].try_into().expect("4 bytes"));
    m0_be == BINARY_MAGIC_0 && m1_be == BINARY_MAGIC_1
}

/// Parsed binary tagfile container prefix. `stream_offset` is normally 8 for
/// HCT 2014 output. Offset 16 is retained only for compatibility with the
/// repository's pre-existing synthetic fixtures, which inserted a private
/// `(tag=1, version=13)` prefix before the real stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tagfile2014Header {
    pub swap_bytes: bool,
    pub stream_offset: usize,
}

const MAGIC_SIZE: usize = 8;
const SYNTHETIC_PREFIX_SIZE: usize = 16;

/// Parse the binary stream prefix and locate its first VLE record.
pub fn parse_header(data: &[u8]) -> HavokResult<Tagfile2014Header> {
    if data.len() < MAGIC_SIZE {
        return Err(HavokError::InvalidInput(format!(
            "Tagfile2014 magic requires {MAGIC_SIZE} bytes, got {}",
            data.len()
        )));
    }
    let magic0_le = u32::from_le_bytes(data[0..4].try_into().expect("4 bytes"));
    let swap_bytes = if magic0_le == BINARY_MAGIC_0 {
        false
    } else if magic0_le == BINARY_MAGIC_0.swap_bytes() {
        true
    } else {
        return Err(HavokError::UnsupportedFormat(format!(
            "Tagfile2014 magic0 mismatch: got {magic0_le:#010X}, expected {BINARY_MAGIC_0:#010X}"
        )));
    };
    let read_u32 = |offset: usize| -> u32 {
        let raw = u32::from_le_bytes(
            data[offset..offset + 4]
                .try_into()
                .expect("4 bytes from header range"),
        );
        if swap_bytes { raw.swap_bytes() } else { raw }
    };
    // We already consumed magic0 (raw); read magic1 with the chosen
    // endianness so the comparison normalizes the byte order.
    let magic1 = read_u32(4);
    if magic1 != BINARY_MAGIC_1 {
        return Err(HavokError::UnsupportedFormat(format!(
            "Tagfile2014 magic1 mismatch: got {magic1:#010X}, expected {BINARY_MAGIC_1:#010X}"
        )));
    }
    let stream_offset = if data.len() >= SYNTHETIC_PREFIX_SIZE
        && read_u32(8) == 1
        && read_u32(12) == TAGFILE_VERSION_2014_2
    {
        SYNTHETIC_PREFIX_SIZE
    } else {
        MAGIC_SIZE
    };
    let (first_tag, _) = read_vle_signed(data, stream_offset)?;
    if first_tag != TAG_FILE_INFO {
        return Err(HavokError::UnsupportedFormat(format!(
            "Tagfile2014 stream at offset {stream_offset:#X} starts with tag {first_tag}; expected TAG_FILE_INFO ({TAG_FILE_INFO})"
        )));
    }
    Ok(Tagfile2014Header {
        swap_bytes,
        stream_offset,
    })
}

/// Read only TAG_FILE_INFO and report the embedded Havok SDK version without
/// decoding the object graph.
pub fn read_tagfile2014_version(blob: &[u8]) -> HavokResult<String> {
    let header = parse_header(blob)?;
    let mut reader = Tagfile2014Reader::new(blob, header.swap_bytes, header.stream_offset);
    let tag_offset = reader.pos;
    let tag = reader.read_int()?;
    if tag != TAG_FILE_INFO {
        return Err(HavokError::UnsupportedFormat(format!(
            "Tagfile2014 stream at offset {tag_offset:#X} starts with tag {tag}; expected TAG_FILE_INFO ({TAG_FILE_INFO})"
        )));
    }
    reader.parse_file_info()?;
    Ok(reader
        .sdk_version
        .unwrap_or_else(|| format!("tagfile-stream-v{}", reader.tagfile_version)))
}

// ---------------------------------------------------------------------------
// Stream tags (TAG_FILE_INFO / TAG_METADATA / TAG_OBJECT*) — see SDK
// hkTagfileCommon2014.h: hkBinaryTagfile2014::TagType enum.
// ---------------------------------------------------------------------------

const TAG_EOF: i64 = -1;
const TAG_FILE_INFO: i64 = 1;
const TAG_METADATA: i64 = 2;
const TAG_OBJECT: i64 = 3;
const TAG_OBJECT_REMEMBER: i64 = 4;
const TAG_OBJECT_BACKREF: i64 = 5;
const TAG_OBJECT_NULL: i64 = 6;
const TAG_FILE_END: i64 = 7;

// hkLegacyType bits (SDK hkLegacyType.h). The basic-type code is in the
// low 4 bits; the array (0x10) and tuple (0x20) bits modify it.
const LT_TYPE_VOID: u32 = 0;
const LT_TYPE_BYTE: u32 = 1;
const LT_TYPE_INT: u32 = 2;
const LT_TYPE_REAL: u32 = 3;
const LT_TYPE_VEC_4: u32 = 4;
const LT_TYPE_VEC_8: u32 = 5;
const LT_TYPE_VEC_12: u32 = 6;
const LT_TYPE_VEC_16: u32 = 7;
const LT_TYPE_OBJECT: u32 = 8;
const LT_TYPE_STRUCT: u32 = 9;
const LT_TYPE_CSTRING: u32 = 10;
const LT_TYPE_MASK_BASIC: u32 = 0xF;
const LT_TYPE_ARRAY: u32 = 0x10;
const LT_TYPE_TUPLE: u32 = 0x20;

// VLE int decoder. SDK ref: hkTagfileReadFormat2014.cpp:307-326.
//
// First byte:
//   bit 0   sign (1 = negative)
//   bits 1-6 magnitude bits (low 6 bits of the unsigned magnitude)
//   bit 7   continuation
// Continuation bytes (while top bit set):
//   bits 0-6 7 magnitude bits, shifted by (6 + 7*n) into the unsigned
//   bit 7    continuation
// Final magnitude is unsigned; if sign was set, result = -magnitude.
const MAX_VLE_MAGNITUDE_BITS: u32 = 64;

fn read_vle_signed(data: &[u8], pos: usize) -> HavokResult<(i64, usize)> {
    let mut cursor = pos;
    let first = *data.get(cursor).ok_or_else(|| {
        HavokError::InvalidInput("Tagfile2014 VLE: truncated at first byte".to_string())
    })?;
    cursor += 1;
    let neg = (first & 1) != 0;
    let mut magnitude: u64 = u64::from((first & 0x7E) >> 1);
    let mut shift: u32 = 6;
    let mut continuation = (first & 0x80) != 0;
    while continuation {
        if shift >= MAX_VLE_MAGNITUDE_BITS {
            return Err(HavokError::InvalidInput(
                "Tagfile2014 VLE: magnitude exceeds 64 bits".to_string(),
            ));
        }
        let next = *data.get(cursor).ok_or_else(|| {
            HavokError::InvalidInput(
                "Tagfile2014 VLE: truncated mid continuation chain".to_string(),
            )
        })?;
        cursor += 1;
        magnitude |= u64::from(next & 0x7F) << shift;
        shift += 7;
        continuation = (next & 0x80) != 0;
    }
    // SDK casts u64→i64, then negates if sign bit was set. We mirror the
    // wrap to preserve round-trip behavior on edge values (i64::MIN).
    let signed = magnitude as i64;
    let result = if neg { -signed } else { signed };
    Ok((result, cursor - pos))
}

/// Class definition parsed out of a TAG_METADATA record. Internal model —
/// not exposed to callers; translated to `HkxObject`/`HkxMember` when the
/// object stream is materialized.
#[derive(Debug, Clone)]
pub(crate) struct ClassDef {
    pub name: String,
    pub version: i32,
    /// Index into `Tagfile2014Reader::classes`, or `None` if no parent
    /// (parent_index < 0 in the stream).
    pub parent_index: Option<usize>,
    pub fields: Vec<FieldDef>,
}

#[derive(Debug, Clone)]
pub(crate) struct FieldDef {
    pub name: String,
    pub legacy_type: u32,
    /// Set only when `legacy_type & TUPLE` (fixed C-array element count).
    pub tuple_count: i32,
    /// Set when basic-type code is STRUCT or OBJECT — the referenced class
    /// name. Forward references resolve later as additional TAG_METADATA
    /// entries arrive.
    pub class_name: Option<String>,
}

/// Reader state. Owns the input slice plus all transient state (cursor,
/// remembered objects, string table, class registry, scratch flags).
pub(crate) struct Tagfile2014Reader<'a> {
    data: &'a [u8],
    pos: usize,
    swap_bytes: bool,
    /// String table for backref decoding. Slot 0 = "" (empty string),
    /// slot 1 = None (HK_NULL sentinel). Subsequent entries are pushed in
    /// stream order as new strings are read.
    prev_strings: Vec<Option<String>>,
    /// Class table. classes[0] is the SDK's null sentinel — left in place
    /// so that `parent_index = 0` (which is what the writer emits for
    /// "no parent") maps cleanly.
    pub(crate) classes: Vec<ClassDef>,
    /// Set after TAG_FILE_INFO is parsed. Drives version-gated parsing.
    pub(crate) tagfile_version: i32,
    /// SDK contents version carried by TAG_FILE_INFO v4+.
    pub(crate) sdk_version: Option<String>,
    /// Set when tagfile_version >= 6 — Real fields are 64-bit doubles, not
    /// 32-bit floats. Bethesda content uses single precision exclusively.
    pub(crate) real_is_double: bool,
    /// remembered_objects[id] = index into `objects` (set when the matching
    /// TAG_OBJECT_REMEMBER is read), or None for forward refs not yet
    /// resolved. SDK calls this `m_rememberedObjects`. Slot 0 is reserved
    /// for the null object (per SDK's pushBack(0) at TAG_FILE_INFO time).
    pub(crate) remembered_objects: Vec<Option<usize>>,
    /// Materialized objects, in stream-arrival order. Indices into this
    /// vector are what `HkxValue::Pointer(Some(_))` ultimately holds.
    pub(crate) objects: Vec<HkxObject>,
}

impl<'a> Tagfile2014Reader<'a> {
    pub(crate) fn new(data: &'a [u8], swap_bytes: bool, stream_offset: usize) -> Self {
        // Sentinel ClassDef at index 0, paralleling SDK m_classes.pushBack(NULL)
        // in Reader::Reader (hkTagfileReadFormat2014.cpp:291).
        let sentinel = ClassDef {
            name: String::new(),
            version: 0,
            parent_index: None,
            fields: Vec::new(),
        };
        Self {
            data,
            pos: stream_offset,
            swap_bytes,
            prev_strings: vec![Some(String::new()), None],
            classes: vec![sentinel],
            tagfile_version: 0,
            sdk_version: None,
            real_is_double: false,
            remembered_objects: Vec::new(),
            objects: Vec::new(),
        }
    }

    fn read_u8(&mut self) -> HavokResult<u8> {
        let value = *self.data.get(self.pos).ok_or_else(|| {
            HavokError::InvalidInput("Tagfile2014: truncated reading u8".to_string())
        })?;
        self.pos += 1;
        Ok(value)
    }

    fn read_u16(&mut self) -> HavokResult<u16> {
        let end = self.pos + 2;
        let bytes = self
            .data
            .get(self.pos..end)
            .ok_or_else(|| HavokError::InvalidInput("Tagfile2014: truncated reading u16".into()))?;
        let raw = u16::from_le_bytes(bytes.try_into().expect("2 bytes"));
        self.pos = end;
        Ok(if self.swap_bytes {
            raw.swap_bytes()
        } else {
            raw
        })
    }

    fn read_u32(&mut self) -> HavokResult<u32> {
        let end = self.pos + 4;
        let bytes = self
            .data
            .get(self.pos..end)
            .ok_or_else(|| HavokError::InvalidInput("Tagfile2014: truncated reading u32".into()))?;
        let raw = u32::from_le_bytes(bytes.try_into().expect("4 bytes"));
        self.pos = end;
        Ok(if self.swap_bytes {
            raw.swap_bytes()
        } else {
            raw
        })
    }

    pub(crate) fn read_int(&mut self) -> HavokResult<i64> {
        let (value, consumed) = read_vle_signed(self.data, self.pos)?;
        self.pos += consumed;
        Ok(value)
    }

    /// Read an SDK-style stream string. Length is a signed VLE int; positive
    /// = inline byte payload (no NUL terminator stored), zero/negative =
    /// backref into prev_strings at slot |len|. Slot 0 is the empty string;
    /// slot 1 is `None` (the HK_NULL sentinel).
    pub(crate) fn read_string(&mut self) -> HavokResult<Option<String>> {
        let len = self.read_int()?;
        if len > 0 {
            let len_us = usize::try_from(len).map_err(|_| {
                HavokError::InvalidInput(format!(
                    "Tagfile2014 string length {len} does not fit in usize"
                ))
            })?;
            let end = self.pos.checked_add(len_us).ok_or_else(|| {
                HavokError::InvalidInput("Tagfile2014 string offset overflow".to_string())
            })?;
            let bytes = self.data.get(self.pos..end).ok_or_else(|| {
                HavokError::InvalidInput(format!(
                    "Tagfile2014 string of length {len_us} extends past EOF"
                ))
            })?;
            let owned = String::from_utf8_lossy(bytes).into_owned();
            self.pos = end;
            self.prev_strings.push(Some(owned.clone()));
            Ok(Some(owned))
        } else {
            // Backref: SDK indexes m_prevStrings at -len (slot 0 = "",
            // slot 1 = HK_NULL). Negate using i64 arithmetic so that
            // -i64::MIN doesn't overflow — that magnitude is much larger
            // than any realistic prev_strings size, so it'll just fail the
            // bounds check below.
            let idx_i64 = -len; // -(-N) = N for the negative case; for len==0 we get 0.
            let idx = usize::try_from(idx_i64).map_err(|_| {
                HavokError::InvalidInput(format!(
                    "Tagfile2014 string backref {idx_i64} does not fit in usize"
                ))
            })?;
            self.prev_strings.get(idx).cloned().ok_or_else(|| {
                HavokError::InvalidInput(format!(
                    "Tagfile2014 string backref {idx} is out of range (table has {} entries)",
                    self.prev_strings.len()
                ))
            })
        }
    }

    /// Convenience helper for sites where None (HK_NULL backref) is not
    /// permitted — e.g. class names and field names.
    fn read_string_required(&mut self, label: &str) -> HavokResult<String> {
        match self.read_string()? {
            Some(s) => Ok(s),
            None => Err(HavokError::InvalidInput(format!(
                "Tagfile2014: {label} must be a non-null string"
            ))),
        }
    }

    /// Process TAG_FILE_INFO. Reads the version int and any version-gated
    /// fields (sdk version string, predicate verifier list, double-precision
    /// flag). Mirrors hkTagfileReadFormat2014.cpp:1210-1276.
    pub(crate) fn parse_file_info(&mut self) -> HavokResult<()> {
        let version = self.read_int()?;
        let version_i32 = i32::try_from(version).map_err(|_| {
            HavokError::InvalidInput(format!(
                "Tagfile2014 TAG_FILE_INFO version {version} does not fit in i32"
            ))
        })?;
        self.tagfile_version = version_i32;
        match version_i32 {
            0 => {
                // No payload.
            }
            1 => {
                // Tag-list version follows; SDK extends prev_strings with
                // the static HK_TAG_STRING_LIST table. Bethesda content
                // doesn't use this layout — refuse rather than silently
                // mis-decode a non-empty stream.
                return Err(HavokError::UnsupportedFormat(
                    "Tagfile2014 TAG_FILE_INFO version 1 (tag-list-extended) not supported"
                        .to_string(),
                ));
            }
            2..=6 => {
                // Push the null slot into remembered_objects (SDK:
                // m_rememberedObjects.pushBack(0) at version 2..6 entry).
                // This makes id 0 the canonical null object reference.
                self.remembered_objects.push(None);
                if version_i32 >= 4 {
                    self.sdk_version = self.read_string()?;
                }
                if version_i32 >= 5 {
                    let _max_predicate = self.read_u16()?;
                    let num_verified = self.read_u16()?;
                    for _ in 0..num_verified {
                        let _verified = self.read_u16()?;
                    }
                }
                if version_i32 >= 6 {
                    self.real_is_double = true;
                }
            }
            other => {
                return Err(HavokError::UnsupportedFormat(format!(
                    "Tagfile2014 TAG_FILE_INFO version {other} not recognized"
                )));
            }
        }
        Ok(())
    }

    /// Process TAG_METADATA — a class definition record. SDK ref:
    /// hkTagfileReadFormat2014.cpp:1114-1191 (readClass).
    pub(crate) fn parse_metadata(&mut self) -> HavokResult<()> {
        let record_name = self.read_string_required("TAG_METADATA class name")?;
        let version = self.read_int()?;
        let version_i32 = i32::try_from(version).map_err(|_| {
            HavokError::InvalidInput(format!("Tagfile2014 class version {version} not in i32"))
        })?;

        let parent_index_signed = self.read_int()?;
        let parent_index = if parent_index_signed < 0 {
            None
        } else {
            let idx = usize::try_from(parent_index_signed).map_err(|_| {
                HavokError::InvalidInput(format!(
                    "Tagfile2014 parent index {parent_index_signed} not in usize"
                ))
            })?;
            // SDK indexes m_classes[parentIndex] without bounds check (the
            // writer guarantees forward order). We bounds-check defensively
            // because a malformed or truncated stream could otherwise lead
            // to an inconsistent parent reference downstream.
            if idx >= self.classes.len() {
                return Err(HavokError::InvalidInput(format!(
                    "Tagfile2014 class {record_name} parent index {idx} >= class table size {}",
                    self.classes.len()
                )));
            }
            // 0 is the null sentinel; treat it as "no parent" to match SDK
            // which writes parentIndex >= 0 only when an actual parent exists.
            if idx == 0 { None } else { Some(idx) }
        };

        let num_fields_i64 = self.read_int()?;
        let num_fields = usize::try_from(num_fields_i64).map_err(|_| {
            HavokError::InvalidInput(format!(
                "Tagfile2014 class {record_name} numFields {num_fields_i64} invalid"
            ))
        })?;
        // Sanity bound — Bethesda's largest classes are ~80 fields. A
        // malformed stream could otherwise drive a huge allocation here.
        if num_fields > 4096 {
            return Err(HavokError::InvalidInput(format!(
                "Tagfile2014 class {record_name} numFields {num_fields} exceeds sanity bound"
            )));
        }
        let mut fields = Vec::with_capacity(num_fields);
        for field_index in 0..num_fields {
            let field_name = self.read_string_required("TAG_METADATA field name")?;
            let legacy_type_i64 = self.read_int()?;
            let legacy_type = u32::try_from(legacy_type_i64 & 0xFFFF_FFFF).map_err(|_| {
                HavokError::InvalidInput(format!(
                    "Tagfile2014 field {field_name} legacy type {legacy_type_i64} invalid"
                ))
            })?;
            let has_tuple = (legacy_type & LT_TYPE_TUPLE) != 0;
            let tuple_count = if has_tuple {
                let n = self.read_int()?;
                i32::try_from(n).map_err(|_| {
                    HavokError::InvalidInput(format!(
                        "Tagfile2014 field {field_name} tuple count {n} not in i32"
                    ))
                })?
            } else {
                0
            };
            let basic = legacy_type & LT_TYPE_MASK_BASIC;
            let class_name = if basic == LT_TYPE_OBJECT || basic == LT_TYPE_STRUCT {
                self.read_string()?
            } else {
                None
            };
            fields.push(FieldDef {
                name: field_name,
                legacy_type,
                tuple_count,
                class_name,
            });
            // Suppress unused warning for the loop counter — kept around so
            // future error messages can localize a bad field.
            let _ = field_index;
        }
        self.classes.push(ClassDef {
            name: record_name,
            version: version_i32,
            parent_index,
            fields,
        });
        Ok(())
    }

    /// Read raw little-endian f32. The SwappingStream in the SDK runtime
    /// flips bytes when `m_swapBytes`. Bethesda content is little-endian
    /// on disk on every platform we target.
    fn read_f32(&mut self) -> HavokResult<f32> {
        let end = self.pos + 4;
        let bytes = self
            .data
            .get(self.pos..end)
            .ok_or_else(|| HavokError::InvalidInput("Tagfile2014: truncated reading f32".into()))?;
        let mut arr = [0u8; 4];
        arr.copy_from_slice(bytes);
        if self.swap_bytes {
            arr.reverse();
        }
        self.pos = end;
        Ok(f32::from_le_bytes(arr))
    }

    fn read_f64(&mut self) -> HavokResult<f64> {
        let end = self.pos + 8;
        let bytes = self
            .data
            .get(self.pos..end)
            .ok_or_else(|| HavokError::InvalidInput("Tagfile2014: truncated reading f64".into()))?;
        let mut arr = [0u8; 8];
        arr.copy_from_slice(bytes);
        if self.swap_bytes {
            arr.reverse();
        }
        self.pos = end;
        Ok(f64::from_le_bytes(arr))
    }

    /// Read one Real as f32. Vanilla content is single precision; doubles
    /// (`m_realIsDouble`, tagfile_version >= 6) are narrowed to f32 to fit
    /// `HkxValue::F32`/`F32List`.
    fn read_real(&mut self) -> HavokResult<f32> {
        if self.real_is_double {
            Ok(self.read_f64()? as f32)
        } else {
            self.read_f32()
        }
    }

    /// Member-presence bitfield. SDK ref: hkTagfileReadFormat2014.cpp:365-386
    /// (readBitfield). Reads ceil(num_members/8) bytes; bit `i` (LSB-first
    /// within each byte) of byte `i/8` is the presence flag for member `i`.
    fn read_bitfield(&mut self, num_members: usize) -> HavokResult<Vec<bool>> {
        if num_members == 0 {
            return Ok(Vec::new());
        }
        let num_bytes = num_members.div_ceil(8);
        let end = self.pos.checked_add(num_bytes).ok_or_else(|| {
            HavokError::InvalidInput("Tagfile2014: bitfield byte-count overflow".into())
        })?;
        let bytes = self.data.get(self.pos..end).ok_or_else(|| {
            HavokError::InvalidInput(format!(
                "Tagfile2014: bitfield of {num_bytes} bytes extends past EOF"
            ))
        })?;
        let mut out = Vec::with_capacity(num_members);
        for byte_index in 0..num_bytes {
            let byte = bytes[byte_index];
            for bit in 0..8 {
                if byte_index * 8 + bit >= num_members {
                    break;
                }
                out.push((byte >> bit) & 1 != 0);
            }
        }
        self.pos = end;
        Ok(out)
    }

    /// Walk a class's full field list, parents-first, matching the SDK
    /// `DeclIter<DataFieldDecl>` traversal order in
    /// hkTagfileReadFormat2014.cpp:997.
    fn collect_fields_with_ancestors(&self, class_index: usize) -> HavokResult<Vec<FieldDef>> {
        let mut out = Vec::new();
        let mut chain = Vec::new();
        let mut depth = 0usize;
        let mut current = Some(class_index);
        while let Some(idx) = current {
            if depth >= 64 {
                return Err(HavokError::InvalidInput(
                    "Tagfile2014: class inheritance depth exceeded 64".to_string(),
                ));
            }
            let class = self.classes.get(idx).ok_or_else(|| {
                HavokError::InvalidInput(format!(
                    "Tagfile2014: class index {idx} out of range during inheritance walk"
                ))
            })?;
            chain.push(idx);
            current = class.parent_index;
            depth += 1;
        }
        for idx in chain.into_iter().rev() {
            for field in &self.classes[idx].fields {
                out.push(field.clone());
            }
        }
        Ok(out)
    }

    /// Read a single field value from the stream per its `FieldDef`. SDK
    /// ref: hkTagfileReadFormat2014.cpp:691-863 (readBinaryValue) +
    /// 421-686 (_readArrayItems). `HkxValue::Pointer(Some(remembered_id))`
    /// is provisional — we patch all pointers through `remembered_objects`
    /// after the stream is fully parsed.
    fn read_value(&mut self, field: &FieldDef) -> HavokResult<HkxValue> {
        let tuple_count = if (field.legacy_type & LT_TYPE_TUPLE) != 0 {
            usize::try_from(field.tuple_count).map_err(|_| {
                HavokError::InvalidInput(format!(
                    "Tagfile2014 field {} tuple count {} not in usize",
                    field.name, field.tuple_count
                ))
            })?
        } else {
            0
        };
        let basic = field.legacy_type & LT_TYPE_MASK_BASIC;

        if (field.legacy_type & LT_TYPE_ARRAY) != 0 {
            // hkArray<T> — dynamic length read from the stream.
            let asize_i64 = self.read_int()?;
            let asize = usize::try_from(asize_i64).map_err(|_| {
                HavokError::InvalidInput(format!(
                    "Tagfile2014 field {} array size {} not in usize",
                    field.name, asize_i64
                ))
            })?;
            // Sanity bound to keep a malformed file from driving a huge
            // allocation; vanilla content rarely exceeds a few thousand
            // bones / poses, and a NIF-embedded inline blob is much smaller.
            const MAX_ARRAY_LEN: usize = 100_000;
            if asize > MAX_ARRAY_LEN {
                return Err(HavokError::InvalidInput(format!(
                    "Tagfile2014 field {} array size {} exceeds sanity bound",
                    field.name, asize
                )));
            }
            if basic == LT_TYPE_BYTE {
                let end = self.pos.checked_add(asize).ok_or_else(|| {
                    HavokError::InvalidInput(format!(
                        "Tagfile2014 field {} byte-array end offset overflow",
                        field.name
                    ))
                })?;
                let bytes = self.data.get(self.pos..end).ok_or_else(|| {
                    HavokError::InvalidInput(format!(
                        "Tagfile2014 field {} byte array declares {} bytes at offset {:#X}, but only {} bytes remain",
                        field.name,
                        asize,
                        self.pos,
                        self.data.len().saturating_sub(self.pos)
                    ))
                })?;
                let elements = bytes.iter().copied().map(HkxValue::U8).collect();
                self.pos = end;
                return Ok(HkxValue::Array(elements));
            }
            return Ok(HkxValue::Array(self.read_array_items(field, asize)?));
        }

        if (field.legacy_type & LT_TYPE_TUPLE) != 0 {
            if basic == LT_TYPE_BYTE {
                let end = self.pos.checked_add(tuple_count).ok_or_else(|| {
                    HavokError::InvalidInput(format!(
                        "Tagfile2014 field {} byte-tuple end offset overflow",
                        field.name
                    ))
                })?;
                let bytes = self.data.get(self.pos..end).ok_or_else(|| {
                    HavokError::InvalidInput(format!(
                        "Tagfile2014 field {} byte tuple requires {} bytes at offset {:#X}",
                        field.name, tuple_count, self.pos
                    ))
                })?;
                let elements = bytes.iter().copied().map(HkxValue::U8).collect();
                self.pos = end;
                return Ok(HkxValue::Array(elements));
            }
            return Ok(HkxValue::Array(self.read_array_items(field, tuple_count)?));
        }

        self.read_basic_value(basic, field)
    }

    /// Arrays in the 2014 stream are type-aware. Integer arrays carry a
    /// storage-width prefix, vec4 arrays carry their component count, and
    /// struct arrays are encoded column-major under one member bitmap shared
    /// by every element.
    fn read_array_items(&mut self, field: &FieldDef, count: usize) -> HavokResult<Vec<HkxValue>> {
        let basic = field.legacy_type & LT_TYPE_MASK_BASIC;
        let prefix = match basic {
            LT_TYPE_INT => Some(self.read_int()?),
            LT_TYPE_VEC_4 => {
                let components = self.read_int()?;
                if !(1..=4).contains(&components) {
                    return Err(HavokError::InvalidInput(format!(
                        "Tagfile2014 field {} vec4 array component count {} is outside 1..=4",
                        field.name, components
                    )));
                }
                Some(components)
            }
            _ => None,
        };

        if basic == LT_TYPE_STRUCT {
            return self.read_struct_array(field, count);
        }

        let mut elements = Vec::with_capacity(count);
        for _ in 0..count {
            if basic == LT_TYPE_VEC_4 {
                let components = usize::try_from(prefix.expect("vec4 prefix validated"))
                    .expect("positive vec4 prefix fits usize");
                let mut values = vec![0.0; 4];
                for value in values.iter_mut().take(components) {
                    *value = self.read_real()?;
                }
                elements.push(HkxValue::F32List(values));
            } else {
                elements.push(self.read_basic_value(basic, field)?);
            }
        }
        Ok(elements)
    }

    fn read_struct_array(&mut self, field: &FieldDef, count: usize) -> HavokResult<Vec<HkxValue>> {
        let class_name = field.class_name.as_deref().ok_or_else(|| {
            HavokError::InvalidInput(format!(
                "Tagfile2014 field {} STRUCT array has no class name",
                field.name
            ))
        })?;
        let class_index = self.find_class_by_name(class_name)?;
        let fields = self.collect_fields_with_ancestors(class_index)?;
        let presence = self.read_bitfield(fields.len())?;
        let mut rows = (0..count)
            .map(|_| HkxValue::TypedObject {
                class_name: class_name.to_string(),
                members: Vec::new(),
            })
            .collect::<Vec<_>>();

        for (field_index, struct_field) in fields.iter().enumerate() {
            if !presence[field_index] {
                continue;
            }
            let is_nested_collection =
                (struct_field.legacy_type & (LT_TYPE_ARRAY | LT_TYPE_TUPLE)) != 0;
            let values = if is_nested_collection {
                if struct_field.legacy_type != (LT_TYPE_ARRAY | LT_TYPE_INT) {
                    return Err(HavokError::UnsupportedFormat(format!(
                        "Tagfile2014 HCT 2014 nested collection payload {}.{} within {} at offset {:#X} has unsupported legacy type {:#X}",
                        class_name,
                        struct_field.name,
                        field.name,
                        self.pos,
                        struct_field.legacy_type
                    )));
                }
                let mut nested_arrays = Vec::with_capacity(count);
                for row_index in 0..count {
                    let row_offset = self.pos;
                    let value = self.read_value(struct_field).map_err(|error| {
                        HavokError::InvalidInput(format!(
                            "Tagfile2014 nested INT array {}.{} within {} row {} at offset {:#X}: {}",
                            class_name,
                            struct_field.name,
                            field.name,
                            row_index,
                            row_offset,
                            error
                        ))
                    })?;
                    nested_arrays.push(value);
                }
                nested_arrays
            } else {
                self.read_array_items(struct_field, count)?
            };
            for (row, value) in rows.iter_mut().zip(values) {
                let HkxValue::TypedObject { members, .. } = row else {
                    unreachable!("struct-array rows are initialized as typed objects");
                };
                members.push(HkxMember {
                    name: struct_field.name.clone(),
                    value,
                });
            }
        }
        Ok(rows)
    }

    /// Decode a single non-array, non-tuple value of the given basic type
    /// (the `legacy_type & TYPE_MASK_BASIC_TYPES` slice). Used both for
    /// scalar fields and as the per-element call from arrays/tuples.
    fn read_basic_value(&mut self, basic: u32, field: &FieldDef) -> HavokResult<HkxValue> {
        match basic {
            LT_TYPE_VOID => Ok(HkxValue::Void),
            LT_TYPE_BYTE => {
                // SDK reads 1 raw byte; we stash it as U8.
                let value = self.read_u8()?;
                Ok(HkxValue::U8(value))
            }
            LT_TYPE_INT => {
                // Always VLE-encoded hkInt64. SDK has a u8 specialization
                // but it's never reached from the dispatch table for
                // INT-typed fields (only inside readBinaryValue's
                // KIND_INT case, which we handle in the BYTE branch above
                // for our minimal coverage).
                let value = self.read_int()?;
                Ok(HkxValue::I64(value))
            }
            LT_TYPE_REAL => Ok(HkxValue::F32(self.read_real()?)),
            LT_TYPE_VEC_4 | LT_TYPE_VEC_8 | LT_TYPE_VEC_12 | LT_TYPE_VEC_16 => {
                let count = match basic {
                    LT_TYPE_VEC_4 => 4,
                    LT_TYPE_VEC_8 => 8,
                    LT_TYPE_VEC_12 => 12,
                    LT_TYPE_VEC_16 => 16,
                    _ => unreachable!(),
                };
                let mut floats = Vec::with_capacity(count);
                for _ in 0..count {
                    floats.push(self.read_real()?);
                }
                Ok(HkxValue::F32List(floats))
            }
            LT_TYPE_CSTRING => {
                let s = self.read_string()?;
                Ok(HkxValue::String {
                    value: s.unwrap_or_default(),
                    is_null: false,
                })
            }
            LT_TYPE_OBJECT => {
                // Pointer field — read a VLE id and emit a provisional
                // Pointer(Some(id)). After the full stream is parsed,
                // `remap_pointer_ids` rewrites every pointer to the real
                // object_index via remembered_objects.
                let id_i64 = self.read_int()?;
                if id_i64 == 0 {
                    return Ok(HkxValue::Pointer(None));
                }
                let id = usize::try_from(id_i64).map_err(|_| {
                    HavokError::InvalidInput(format!(
                        "Tagfile2014 field {} pointer id {} not in usize",
                        field.name, id_i64
                    ))
                })?;
                Ok(HkxValue::Pointer(Some(id)))
            }
            LT_TYPE_STRUCT => {
                // Inline struct — recursively read the named class's
                // bitfield + fields directly into an Object value.
                let class_name = field.class_name.as_deref().ok_or_else(|| {
                    HavokError::InvalidInput(format!(
                        "Tagfile2014 field {} STRUCT-typed but has no class name",
                        field.name
                    ))
                })?;
                let class_index = self.find_class_by_name(class_name)?;
                let members = self.read_struct_members(class_index)?;
                Ok(HkxValue::TypedObject {
                    class_name: class_name.to_string(),
                    members,
                })
            }
            other => Err(HavokError::InvalidInput(format!(
                "Tagfile2014 field {} has unsupported basic type code {other}",
                field.name
            ))),
        }
    }

    fn find_class_by_name(&self, name: &str) -> HavokResult<usize> {
        // classes[0] is the null sentinel; skip it. Iterate in declaration
        // order so the *first* match wins, which matches SDK behavior
        // (m_nameToClassMap returns the first-inserted entry).
        for (idx, class) in self.classes.iter().enumerate().skip(1) {
            if class.name == name {
                return Ok(idx);
            }
        }
        Err(HavokError::InvalidInput(format!(
            "Tagfile2014: unknown class name {name} (no TAG_METADATA seen)"
        )))
    }

    /// Read a class's member-presence bitfield + each present member's
    /// value. Shared between top-level objects (TAG_OBJECT*) and inline
    /// STRUCT fields. Members that are not present in the bitfield receive
    /// no `HkxMember` entry — the SDK writes default-zero into the C++
    /// struct, but our model represents absent members by omission.
    fn read_struct_members(&mut self, class_index: usize) -> HavokResult<Vec<HkxMember>> {
        let fields = self.collect_fields_with_ancestors(class_index)?;
        let presence = self.read_bitfield(fields.len())?;
        let mut members = Vec::new();
        for (i, field) in fields.iter().enumerate() {
            if !presence[i] {
                continue;
            }
            let value = self.read_value(field).map_err(|e| {
                HavokError::InvalidInput(format!(
                    "Tagfile2014 class {} member {}: {}",
                    self.classes[class_index].name, field.name, e
                ))
            })?;
            members.push(HkxMember {
                name: field.name.clone(),
                value,
            });
        }
        Ok(members)
    }

    /// Process a single top-level object record (TAG_OBJECT,
    /// TAG_OBJECT_REMEMBER, TAG_OBJECT_NULL). The opening tag has already
    /// been consumed by the caller. Mirrors SDK
    /// hkTagfileReadFormat2014.cpp:898-1012 (readObjectTopLevel). Returns
    /// the remembered_id (1-based) for REMEMBER objects, 0 for NULL, or
    /// `None` for plain TAG_OBJECT (which is anonymous and not pointable).
    fn read_object_top_level(&mut self, tag: i64) -> HavokResult<Option<usize>> {
        if tag == TAG_OBJECT_NULL {
            return Ok(Some(0));
        }
        if tag != TAG_OBJECT && tag != TAG_OBJECT_REMEMBER {
            return Err(HavokError::InvalidInput(format!(
                "Tagfile2014: read_object_top_level called with tag {tag}"
            )));
        }
        // Read the class id. SDK: at tagfile_version >= 2 this is the
        // inline `objTag` int — only set when caller didn't pre-resolve
        // the type from a parent struct field.
        let class_id_i64 = self.read_int()?;
        let class_index = usize::try_from(class_id_i64).map_err(|_| {
            HavokError::InvalidInput(format!(
                "Tagfile2014: object class id {class_id_i64} not in usize"
            ))
        })?;
        if class_index == 0 || class_index >= self.classes.len() {
            return Err(HavokError::InvalidInput(format!(
                "Tagfile2014: object class id {class_index} not in class table (size {})",
                self.classes.len()
            )));
        }
        let class_name = self.classes[class_index].name.clone();

        // Reserve the object_index BEFORE reading members, so any forward
        // pointer to this remembered id from an inner field can resolve
        // back to a known index (matters when an object's child points at
        // its parent — SDK calls this out via m_forwardRefs patching).
        let object_index = self.objects.len();
        let remembered_id_for_return = if tag == TAG_OBJECT_REMEMBER {
            let id = self.remembered_objects.len();
            self.remembered_objects.push(Some(object_index));
            Some(id)
        } else {
            None
        };

        let members = self.read_struct_members(class_index)?;
        self.objects.push(HkxObject {
            name: Some(format!("#{:04}", object_index + 1)),
            offset: 0,
            signature: 0,
            class_name,
            members,
        });
        Ok(remembered_id_for_return)
    }

    /// After every object has been emitted, walk the result and rewrite
    /// every `Pointer(Some(remembered_id))` to `Pointer(Some(object_index))`
    /// using the `remembered_objects` map. Leaves `Pointer(None)` alone
    /// and errors on unresolved (not-in-map) ids.
    fn remap_pointer_ids(&mut self) -> HavokResult<()> {
        // Snapshot the map so the borrow checker is happy while mutating
        // `self.objects` in place.
        let map: Vec<Option<usize>> = self.remembered_objects.clone();
        let mut errors: Vec<String> = Vec::new();
        for object in &mut self.objects {
            for member in &mut object.members {
                remap_value(&map, &mut member.value, &mut errors);
            }
        }
        if !errors.is_empty() {
            return Err(HavokError::InvalidInput(format!(
                "Tagfile2014: pointer remap failed: {}",
                errors.join("; ")
            )));
        }
        Ok(())
    }

    /// Drive the stream through TAG_FILE_INFO + zero or more TAG_METADATA
    /// records. Stops as soon as it sees a TAG_OBJECT* tag (which is then
    /// the first object's tag — the caller continues from there) or
    /// TAG_FILE_END. Returns the tag that ended the loop.
    ///
    /// Defensive: TAG_FILE_INFO must appear before any TAG_METADATA, and
    /// only one TAG_FILE_INFO is permitted.
    pub(crate) fn parse_classes_until_first_object(&mut self) -> HavokResult<i64> {
        let mut saw_file_info = false;
        loop {
            let tag = self.read_int()?;
            match tag {
                TAG_EOF => {
                    return Err(HavokError::InvalidInput(
                        "Tagfile2014: TAG_EOF before TAG_FILE_END".to_string(),
                    ));
                }
                TAG_FILE_INFO => {
                    if saw_file_info {
                        return Err(HavokError::InvalidInput(
                            "Tagfile2014: duplicate TAG_FILE_INFO".to_string(),
                        ));
                    }
                    self.parse_file_info()?;
                    saw_file_info = true;
                }
                TAG_METADATA => {
                    if !saw_file_info {
                        return Err(HavokError::InvalidInput(
                            "Tagfile2014: TAG_METADATA before TAG_FILE_INFO".to_string(),
                        ));
                    }
                    self.parse_metadata()?;
                }
                TAG_OBJECT | TAG_OBJECT_REMEMBER | TAG_OBJECT_NULL | TAG_FILE_END => {
                    return Ok(tag);
                }
                TAG_OBJECT_BACKREF => {
                    return Err(HavokError::InvalidInput(
                        "Tagfile2014: TAG_OBJECT_BACKREF at top level (must be inside an object)"
                            .to_string(),
                    ));
                }
                other => {
                    return Err(HavokError::InvalidInput(format!(
                        "Tagfile2014: unexpected top-level tag {other}"
                    )));
                }
            }
        }
    }
}

/// Recursively remap every `Pointer(Some(id))` inside a value, where `id`
/// is a remembered_id, to `Pointer(Some(object_index))`. Pushes a string
/// describing each unresolved id into `errors`.
fn remap_value(map: &[Option<usize>], value: &mut HkxValue, errors: &mut Vec<String>) {
    match value {
        HkxValue::Pointer(Some(id)) => {
            if let Some(slot) = map.get(*id) {
                match slot {
                    Some(object_index) => *id = *object_index,
                    None => *value = HkxValue::Pointer(None),
                }
            } else {
                errors.push(format!(
                    "remembered id {id} not in remembered_objects (table size {})",
                    map.len()
                ));
            }
        }
        HkxValue::Pointer(None) => {}
        HkxValue::Array(elements) => {
            for element in elements {
                remap_value(map, element, errors);
            }
        }
        HkxValue::Object(members) | HkxValue::TypedObject { members, .. } => {
            for member in members {
                remap_value(map, &mut member.value, errors);
            }
        }
        _ => {}
    }
}

/// Top-level entry point. Parses a binary tagfile v13 buffer into an
/// `HkxFile`. Returns `UnsupportedFormat` for non-v13 files (older
/// recursive/transitional layouts are not implemented). Other
/// materialization errors surface as `InvalidInput` with descriptive
/// context.
pub fn read_tagfile2014(blob: &[u8]) -> HavokResult<HkxFile> {
    let header = parse_header(blob)?;
    let mut reader = Tagfile2014Reader::new(blob, header.swap_bytes, header.stream_offset);
    let mut next_tag = reader.parse_classes_until_first_object()?;
    // Drive the object stream until TAG_FILE_END.
    loop {
        match next_tag {
            TAG_FILE_END => break,
            TAG_OBJECT | TAG_OBJECT_REMEMBER | TAG_OBJECT_NULL => {
                reader.read_object_top_level(next_tag)?;
            }
            other => {
                return Err(HavokError::InvalidInput(format!(
                    "Tagfile2014: unexpected object-stream tag {other} at offset {:#X}",
                    reader.pos
                )));
            }
        }
        next_tag = reader.read_int()?;
    }
    reader.remap_pointer_ids()?;
    // Determine the contents version. Bethesda's writer always sets it to
    // hk_2014.x.x-rN; we have no string in the header to distinguish flavors,
    // so report a generic 2014 contents-version that downstream callers can
    // map to the FO4/Skyrim-SE descriptor sets via DescriptorRegistry.
    let contents_version = reader
        .sdk_version
        .clone()
        .unwrap_or_else(|| "hk_2014.1.0-r1".to_string());
    Ok(HkxFile::from_tagxml(11, contents_version, reader.objects))
}

/// Serialize a simple `HkxFile` object graph as binary tagfile v13 bytes.
///
/// This is intentionally a narrow writer for edit/test paths where class
/// metadata can be inferred from the current `HkxFile` shape. It emits
/// TAG_FILE_INFO v3, flat no-parent TAG_METADATA classes, and every top-level
/// object as TAG_OBJECT_REMEMBER. Unsupported values return `InvalidInput`
/// with the member path instead of silently falling back to packfile output.
pub fn write_tagfile2014(hkx: &HkxFile) -> HavokResult<Vec<u8>> {
    let classes = infer_writer_classes(hkx)?;
    let mut out = Vec::new();
    out.extend_from_slice(&BINARY_MAGIC_0.to_le_bytes());
    out.extend_from_slice(&BINARY_MAGIC_1.to_le_bytes());

    write_vle_signed(&mut out, TAG_FILE_INFO)?;
    write_vle_signed(&mut out, 3)?;

    for class in classes.iter().skip(1) {
        write_vle_signed(&mut out, TAG_METADATA)?;
        write_stream_string(&mut out, Some(class.name.as_str()))?;
        write_vle_signed(&mut out, i64::from(class.version))?;
        write_vle_signed(&mut out, -1)?;
        write_vle_signed(&mut out, class.fields.len() as i64)?;
        for field in &class.fields {
            write_stream_string(&mut out, Some(field.name.as_str()))?;
            write_vle_signed(&mut out, i64::from(field.legacy_type))?;
            if (field.legacy_type & LT_TYPE_TUPLE) != 0 {
                write_vle_signed(&mut out, i64::from(field.tuple_count))?;
            }
            let basic = field.legacy_type & LT_TYPE_MASK_BASIC;
            if basic == LT_TYPE_OBJECT || basic == LT_TYPE_STRUCT {
                write_stream_string(&mut out, field.class_name.as_deref())?;
            }
        }
    }

    for object in hkx.objects() {
        write_vle_signed(&mut out, TAG_OBJECT_REMEMBER)?;
        let class_index = find_class_index(&classes, &object.class_name).ok_or_else(|| {
            HavokError::InvalidInput(format!(
                "Tagfile2014 writer: missing metadata for top-level class {}",
                object.class_name
            ))
        })?;
        write_vle_signed(&mut out, class_index as i64)?;
        write_struct_members(
            &mut out,
            &classes,
            &object.class_name,
            &object.members,
            hkx.objects().len(),
        )?;
    }

    write_vle_signed(&mut out, TAG_FILE_END)?;
    Ok(out)
}

fn write_vle_signed(out: &mut Vec<u8>, value: i64) -> HavokResult<()> {
    if value == i64::MIN {
        return Err(HavokError::InvalidInput(
            "Tagfile2014 writer: VLE i64::MIN is not supported".to_string(),
        ));
    }
    let neg = value < 0;
    let mag = value.unsigned_abs();
    let first_mag = (mag & 0x3F) as u8;
    let mut remaining = mag >> 6;
    let mut first = (first_mag << 1) | u8::from(neg);
    if remaining != 0 {
        first |= 0x80;
    }
    out.push(first);
    while remaining != 0 {
        let chunk = (remaining & 0x7F) as u8;
        remaining >>= 7;
        out.push(if remaining != 0 { chunk | 0x80 } else { chunk });
    }
    Ok(())
}

fn write_stream_string(out: &mut Vec<u8>, value: Option<&str>) -> HavokResult<()> {
    match value {
        Some("") => write_vle_signed(out, 0),
        Some(s) => {
            let len = i64::try_from(s.len()).map_err(|_| {
                HavokError::InvalidInput(format!(
                    "Tagfile2014 writer: string length {} does not fit in i64",
                    s.len()
                ))
            })?;
            write_vle_signed(out, len)?;
            out.extend_from_slice(s.as_bytes());
            Ok(())
        }
        None => write_vle_signed(out, -1),
    }
}

fn infer_writer_classes(hkx: &HkxFile) -> HavokResult<Vec<ClassDef>> {
    let mut classes = vec![ClassDef {
        name: String::new(),
        version: 0,
        parent_index: None,
        fields: Vec::new(),
    }];
    for object in hkx.objects() {
        ensure_class_shape(&mut classes, &object.class_name, &object.members)?;
    }
    Ok(classes)
}

fn ensure_class_shape(
    classes: &mut Vec<ClassDef>,
    class_name: &str,
    members: &[HkxMember],
) -> HavokResult<usize> {
    if class_name.is_empty() {
        return Err(HavokError::InvalidInput(
            "Tagfile2014 writer: class name must not be empty".to_string(),
        ));
    }

    let class_index = match find_class_index(classes, class_name) {
        Some(index) => index,
        None => {
            classes.push(ClassDef {
                name: class_name.to_string(),
                version: 0,
                parent_index: None,
                fields: Vec::new(),
            });
            classes.len() - 1
        }
    };

    for (index, member) in members.iter().enumerate() {
        if members[index + 1..]
            .iter()
            .any(|other| other.name == member.name)
        {
            return Err(HavokError::InvalidInput(format!(
                "Tagfile2014 writer: class {class_name} has duplicate member {}",
                member.name
            )));
        }
        let field = infer_field(classes, &member.name, &member.value, class_name)?;
        merge_class_field(classes, class_index, field)?;
    }

    Ok(class_index)
}

fn merge_class_field(
    classes: &mut [ClassDef],
    class_index: usize,
    field: FieldDef,
) -> HavokResult<()> {
    let class_name = classes[class_index].name.clone();
    if let Some(existing) = classes[class_index]
        .fields
        .iter()
        .find(|existing| existing.name == field.name)
    {
        if existing.legacy_type != field.legacy_type
            || existing.tuple_count != field.tuple_count
            || existing.class_name != field.class_name
        {
            return Err(HavokError::InvalidInput(format!(
                "Tagfile2014 writer: class {class_name} member {} has conflicting inferred types",
                field.name
            )));
        }
        return Ok(());
    }
    classes[class_index].fields.push(field);
    Ok(())
}

fn infer_field(
    classes: &mut Vec<ClassDef>,
    name: &str,
    value: &HkxValue,
    owner_class: &str,
) -> HavokResult<FieldDef> {
    let (legacy_type, class_name) = match value {
        HkxValue::Array(values) => {
            let first = values.first().ok_or_else(|| {
                HavokError::InvalidInput(format!(
                    "Tagfile2014 writer: class {owner_class} member {name} is an empty array; element type cannot be inferred"
                ))
            })?;
            let first_type = infer_basic_value_type(
                classes,
                first,
                &format!("class {owner_class} member {name}[]"),
            )?;
            for element in &values[1..] {
                let element_type = infer_basic_value_type(
                    classes,
                    element,
                    &format!("class {owner_class} member {name}[]"),
                )?;
                if element_type != first_type {
                    return Err(HavokError::InvalidInput(format!(
                        "Tagfile2014 writer: class {owner_class} member {name} array elements are not homogeneous"
                    )));
                }
            }
            (
                LT_TYPE_ARRAY | first_type.legacy_type,
                first_type.class_name,
            )
        }
        other => {
            let value_type = infer_basic_value_type(
                classes,
                other,
                &format!("class {owner_class} member {name}"),
            )?;
            (value_type.legacy_type, value_type.class_name)
        }
    };
    Ok(FieldDef {
        name: name.to_string(),
        legacy_type,
        tuple_count: 0,
        class_name,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InferredValueType {
    legacy_type: u32,
    class_name: Option<String>,
}

fn infer_basic_value_type(
    classes: &mut Vec<ClassDef>,
    value: &HkxValue,
    label: &str,
) -> HavokResult<InferredValueType> {
    let inferred = match value {
        HkxValue::Void => InferredValueType {
            legacy_type: LT_TYPE_VOID,
            class_name: None,
        },
        HkxValue::U8(_) => InferredValueType {
            legacy_type: LT_TYPE_BYTE,
            class_name: None,
        },
        HkxValue::I64(_) => InferredValueType {
            legacy_type: LT_TYPE_INT,
            class_name: None,
        },
        HkxValue::F32(_) => InferredValueType {
            legacy_type: LT_TYPE_REAL,
            class_name: None,
        },
        HkxValue::F32List(values) => InferredValueType {
            legacy_type: vec_legacy_type(values.len()).ok_or_else(|| {
                HavokError::InvalidInput(format!(
                    "Tagfile2014 writer: {label} F32List length {} is unsupported; expected 4, 8, 12, or 16",
                    values.len()
                ))
            })?,
            class_name: None,
        },
        HkxValue::String { is_null: true, .. } => {
            return Err(HavokError::InvalidInput(format!(
                "Tagfile2014 writer: {label} is a null string; null CSTRING round-trip is not supported"
            )));
        }
        HkxValue::String { .. } => InferredValueType {
            legacy_type: LT_TYPE_CSTRING,
            class_name: None,
        },
        HkxValue::Pointer(_) => InferredValueType {
            legacy_type: LT_TYPE_OBJECT,
            class_name: Some(String::new()),
        },
        HkxValue::TypedObject {
            class_name,
            members,
        } => {
            ensure_class_shape(classes, class_name, members)?;
            InferredValueType {
                legacy_type: LT_TYPE_STRUCT,
                class_name: Some(class_name.clone()),
            }
        }
        HkxValue::Object(_) => {
            return Err(HavokError::InvalidInput(format!(
                "Tagfile2014 writer: {label} is an inline Object without class metadata; use TypedObject"
            )));
        }
        HkxValue::Array(_) => {
            return Err(HavokError::InvalidInput(format!(
                "Tagfile2014 writer: {label} is a nested array; nested hkArray values are not supported"
            )));
        }
        HkxValue::PendingPtr(name) => {
            return Err(HavokError::InvalidInput(format!(
                "Tagfile2014 writer: {label} contains unresolved pointer {name}"
            )));
        }
        other => {
            return Err(HavokError::InvalidInput(format!(
                "Tagfile2014 writer: {label} value {other:?} is not supported by the minimal v13 writer"
            )));
        }
    };
    Ok(inferred)
}

fn vec_legacy_type(len: usize) -> Option<u32> {
    match len {
        4 => Some(LT_TYPE_VEC_4),
        8 => Some(LT_TYPE_VEC_8),
        12 => Some(LT_TYPE_VEC_12),
        16 => Some(LT_TYPE_VEC_16),
        _ => None,
    }
}

fn find_class_index(classes: &[ClassDef], class_name: &str) -> Option<usize> {
    classes
        .iter()
        .enumerate()
        .skip(1)
        .find_map(|(index, class)| (class.name == class_name).then_some(index))
}

fn write_struct_members(
    out: &mut Vec<u8>,
    classes: &[ClassDef],
    class_name: &str,
    members: &[HkxMember],
    object_count: usize,
) -> HavokResult<()> {
    let class_index = find_class_index(classes, class_name).ok_or_else(|| {
        HavokError::InvalidInput(format!(
            "Tagfile2014 writer: missing metadata for struct class {class_name}"
        ))
    })?;
    let fields = &classes[class_index].fields;
    let mut presence = Vec::with_capacity(fields.len());
    for field in fields {
        presence.push(members.iter().any(|member| member.name == field.name));
    }
    write_presence_bytes(out, &presence);
    for field in fields {
        if let Some(member) = members.iter().find(|member| member.name == field.name) {
            write_value(out, classes, field, &member.value, object_count)?;
        }
    }
    Ok(())
}

fn write_presence_bytes(out: &mut Vec<u8>, presence: &[bool]) {
    if presence.is_empty() {
        return;
    }
    let start = out.len();
    out.resize(start + presence.len().div_ceil(8), 0);
    for (index, present) in presence.iter().enumerate() {
        if *present {
            out[start + index / 8] |= 1 << (index % 8);
        }
    }
}

fn write_value(
    out: &mut Vec<u8>,
    classes: &[ClassDef],
    field: &FieldDef,
    value: &HkxValue,
    object_count: usize,
) -> HavokResult<()> {
    let basic = field.legacy_type & LT_TYPE_MASK_BASIC;
    if (field.legacy_type & LT_TYPE_ARRAY) != 0 {
        let values = match value {
            HkxValue::Array(values) => values,
            other => {
                return Err(HavokError::InvalidInput(format!(
                    "Tagfile2014 writer: member {} expected Array for legacy type {}, got {other:?}",
                    field.name, field.legacy_type
                )));
            }
        };
        write_vle_signed(out, values.len() as i64)?;
        for element in values {
            write_basic_value(out, classes, field, basic, element, object_count)?;
        }
        return Ok(());
    }
    write_basic_value(out, classes, field, basic, value, object_count)
}

fn write_basic_value(
    out: &mut Vec<u8>,
    classes: &[ClassDef],
    field: &FieldDef,
    basic: u32,
    value: &HkxValue,
    object_count: usize,
) -> HavokResult<()> {
    match (basic, value) {
        (LT_TYPE_VOID, HkxValue::Void) => Ok(()),
        (LT_TYPE_BYTE, HkxValue::U8(value)) => {
            out.push(*value);
            Ok(())
        }
        (LT_TYPE_INT, HkxValue::I64(value)) => write_vle_signed(out, *value),
        (LT_TYPE_REAL, HkxValue::F32(value)) => {
            out.extend_from_slice(&value.to_le_bytes());
            Ok(())
        }
        (
            LT_TYPE_VEC_4 | LT_TYPE_VEC_8 | LT_TYPE_VEC_12 | LT_TYPE_VEC_16,
            HkxValue::F32List(values),
        ) => {
            let expected_len = match basic {
                LT_TYPE_VEC_4 => 4,
                LT_TYPE_VEC_8 => 8,
                LT_TYPE_VEC_12 => 12,
                LT_TYPE_VEC_16 => 16,
                _ => unreachable!(),
            };
            if values.len() != expected_len {
                return Err(HavokError::InvalidInput(format!(
                    "Tagfile2014 writer: member {} expected F32List length {expected_len}, got {}",
                    field.name,
                    values.len()
                )));
            }
            for value in values {
                out.extend_from_slice(&value.to_le_bytes());
            }
            Ok(())
        }
        (LT_TYPE_CSTRING, HkxValue::String { value, is_null }) => {
            if *is_null {
                return Err(HavokError::InvalidInput(format!(
                    "Tagfile2014 writer: member {} is a null string; null CSTRING round-trip is not supported",
                    field.name
                )));
            }
            write_stream_string(out, Some(value.as_str()))
        }
        (LT_TYPE_OBJECT, HkxValue::Pointer(target)) => {
            let remembered_id = match target {
                Some(index) => {
                    if *index >= object_count {
                        return Err(HavokError::InvalidInput(format!(
                            "Tagfile2014 writer: member {} pointer target {} out of range for {} objects",
                            field.name, index, object_count
                        )));
                    }
                    index + 1
                }
                None => 0,
            };
            write_vle_signed(out, remembered_id as i64)
        }
        (
            LT_TYPE_STRUCT,
            HkxValue::TypedObject {
                class_name,
                members,
            },
        ) => {
            let expected = field.class_name.as_deref().unwrap_or_default();
            if class_name != expected {
                return Err(HavokError::InvalidInput(format!(
                    "Tagfile2014 writer: member {} expected TypedObject class {expected}, got {class_name}",
                    field.name
                )));
            }
            write_struct_members(out, classes, class_name, members, object_count)
        }
        _ => Err(HavokError::InvalidInput(format!(
            "Tagfile2014 writer: member {} value {value:?} does not match legacy type {basic}",
            field.name
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vle_signed(value: i64) -> Vec<u8> {
        // Inverse of read_vle_signed. We always encode minimum-bytes form,
        // matching what the SDK writer would emit.
        let neg = value < 0;
        // Mirror SDK semantics: i64::MIN cannot be safely negated, but no
        // VLE-encoded magnitude in vanilla content is anywhere near that
        // bound — this helper is for tests only.
        let mag = if neg { (-value) as u64 } else { value as u64 };
        let mut out = Vec::new();
        // First byte: bit 0 sign, bits 1..6 lower 6 magnitude bits, bit 7 cont.
        let first_mag = (mag & 0x3F) as u8; // 6 bits
        let mut remaining = mag >> 6;
        let cont = remaining != 0;
        let mut first = (first_mag << 1) | u8::from(neg);
        if cont {
            first |= 0x80;
        }
        out.push(first);
        while remaining != 0 {
            let chunk = (remaining & 0x7F) as u8;
            remaining >>= 7;
            let cont = remaining != 0;
            out.push(if cont { chunk | 0x80 } else { chunk });
        }
        out
    }

    #[test]
    fn vle_round_trip_small_positive_and_negative_values() {
        for v in [0i64, 1, -1, 7, -7, 0x3F, -0x3F] {
            let encoded = vle_signed(v);
            let (decoded, consumed) = read_vle_signed(&encoded, 0).unwrap();
            assert_eq!(decoded, v, "round trip {v}");
            assert_eq!(consumed, encoded.len());
        }
    }

    #[test]
    fn vle_round_trip_continuation_widths() {
        for v in [0x40i64, -0x40, 0x1FFF, -0x1FFF, 0x10_0000, 0x7FFF_FFFF] {
            let encoded = vle_signed(v);
            let (decoded, consumed) = read_vle_signed(&encoded, 0).unwrap();
            assert_eq!(decoded, v, "round trip {v}");
            assert_eq!(consumed, encoded.len());
        }
    }

    #[test]
    fn vle_truncated_input_errors() {
        let err = read_vle_signed(&[0x80], 0).unwrap_err();
        assert!(err.to_string().contains("truncated"));
    }
}
