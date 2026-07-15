/*!
Port of `py_creation_lib/python/creation_lib/hkxpack/writer.py` + write-side of `py_creation_lib/python/creation_lib/hkxpack/packfile.py`.

Two-phase approach:
  Phase 1: Write all objects to a data buffer, collecting fixup lists.
  Phase 2: Assemble sections, resolve global fixups, write fixup tables.

The 7 layout rules from `py_creation_lib/python/creation_lib/hkxpack/CLAUDE.md` are reproduced here;
each rule is cited by number in the relevant code section.
*/

use std::collections::HashMap;

use tracing::warn;

use super::descriptors::{DescriptorRegistry, MemberTemplate};
use super::model::{HkxFile, HkxMember, HkxObject};
use super::packfile::{
    ClassnameEntry, PackfileHeader, SectionHeader, snap_to_16, write_classnames,
    write_global_fixups, write_header, write_local_fixups, write_section_header,
    write_section_header_v8, write_virtual_fixups,
};
use super::types::{HkxType, HkxTypeFamily, HkxValue, f32_to_half};

// ─── Serialize a scalar value ─────────────────────────────────────────────

pub(crate) fn serialize_value(vtype: HkxType, vsubtype: HkxType, value: &HkxValue) -> Vec<u8> {
    // For ENUM/FLAGS the storage width is vsubtype.size().
    let effective_type = if vtype.family() == HkxTypeFamily::Enum {
        vsubtype
    } else {
        vtype
    };
    let size = effective_type.size().max(1);
    let mut out = vec![0u8; size];
    match value {
        HkxValue::Bool(b) => {
            out[0] = if *b { 1 } else { 0 };
        }
        HkxValue::I8(v) => out[0] = *v as u8,
        HkxValue::U8(v) => out[0] = *v,
        HkxValue::I16(v) => {
            let bytes = v.to_le_bytes();
            let copy = bytes.len().min(size);
            out[..copy].copy_from_slice(&bytes[..copy]);
        }
        HkxValue::U16(v) => {
            let bytes = v.to_le_bytes();
            let copy = bytes.len().min(size);
            out[..copy].copy_from_slice(&bytes[..copy]);
        }
        HkxValue::I32(v) => {
            let bytes = v.to_le_bytes();
            let copy = bytes.len().min(size);
            out[..copy].copy_from_slice(&bytes[..copy]);
        }
        HkxValue::U32(v) => {
            let bytes = v.to_le_bytes();
            let copy = bytes.len().min(size);
            out[..copy].copy_from_slice(&bytes[..copy]);
        }
        HkxValue::I64(v) => {
            let bytes = v.to_le_bytes();
            let copy = bytes.len().min(size);
            out[..copy].copy_from_slice(&bytes[..copy]);
        }
        HkxValue::U64(v) => {
            let bytes = v.to_le_bytes();
            let copy = bytes.len().min(size);
            out[..copy].copy_from_slice(&bytes[..copy]);
        }
        HkxValue::F32(v) => {
            let bytes = v.to_le_bytes();
            let copy = bytes.len().min(size);
            out[..copy].copy_from_slice(&bytes[..copy]);
        }
        HkxValue::Half(v) => {
            let half = f32_to_half(*v);
            let bytes = half.to_le_bytes();
            let copy = bytes.len().min(size);
            out[..copy].copy_from_slice(&bytes[..copy]);
        }
        HkxValue::F32List(floats) => {
            let mut off = 0;
            for f in floats {
                if off + 4 > size {
                    break;
                }
                out[off..off + 4].copy_from_slice(&f.to_le_bytes());
                off += 4;
            }
        }
        // Containers (String/Pointer/Array/Object/TypedObject) are serialized
        // through other paths (deferred callbacks, write_inline_struct, etc.),
        // so reaching this arm with a payload-carrying variant is a writer-side
        // bug — emit zero bytes and warn rather than silently truncate.
        other => warn!(
            "serialize_value: no scalar encoding for {} (vtype={:?}, vsubtype={:?}); writing {} zero bytes",
            other.variant_name(),
            vtype,
            vsubtype,
            size,
        ),
    }
    out
}

// ─── Layout helpers ───────────────────────────────────────────────────────

/// Calculate the total byte span of one member.
///
/// Rule 4: For ENUM/FLAGS the stored width is in vsubtype, not vtype.
pub fn member_byte_span(mt: &MemberTemplate, registry: &mut DescriptorRegistry) -> usize {
    if mt.vtype.family() == HkxTypeFamily::Object && !mt.ctype.is_empty() {
        let nested = registry.get_all_members(&mt.ctype).unwrap_or_default();
        return calc_inline_struct_size(&nested, registry);
    }
    let elem_size = if mt.vtype.family() == HkxTypeFamily::Enum {
        mt.vsubtype.size().max(1)
    } else {
        mt.vtype.size().max(1)
    };
    let count = if mt.arrsize > 0 { mt.arrsize } else { 1 };
    elem_size * count
}

/// Calculate the natural alignment of a struct from its member families.
///
/// Rule 4: COMPLEX → 16, POINTER/ARRAY/STRING → 8, ENUM → vsubtype.size,
/// OBJECT → recurse, DIRECT scalars → their own width.
pub fn max_member_alignment(
    members: &[MemberTemplate],
    registry: &mut DescriptorRegistry,
) -> usize {
    let mut max_align = 1usize;
    for m in members {
        let a = match m.vtype.family() {
            HkxTypeFamily::Complex => 16, // Rule 4: SIMD types always 16-byte
            HkxTypeFamily::Object if !m.ctype.is_empty() => {
                let nested = registry.get_all_members(&m.ctype).unwrap_or_default();
                max_member_alignment(&nested, registry)
            }
            HkxTypeFamily::Pointer | HkxTypeFamily::Array | HkxTypeFamily::String => 8,
            HkxTypeFamily::Enum => m.vsubtype.size().max(1), // Rule 4: vsubtype width
            _ => m.vtype.size().max(1).min(8),               // Direct scalars, capped at 8
        };
        if a > max_align {
            max_align = a;
        }
    }
    max_align
}

/// Calculate top-level object size (16-byte aligned).
pub fn calc_object_size(members: &[MemberTemplate], registry: &mut DescriptorRegistry) -> usize {
    if members.is_empty() {
        return 16;
    }
    let last = &members[members.len() - 1];
    let raw = last.offset + member_byte_span(last, registry);
    snap_to_16(raw)
}

/// Calculate inline struct size using NATURAL alignment (not 16-byte snap).
///
/// Rule 4: stride = raw_end rounded up to natural alignment.
pub fn calc_inline_struct_size(
    members: &[MemberTemplate],
    registry: &mut DescriptorRegistry,
) -> usize {
    if members.is_empty() {
        return 0;
    }
    let last = &members[members.len() - 1];
    let raw_size = last.offset + member_byte_span(last, registry);
    let max_align = max_member_alignment(members, registry);
    (raw_size + max_align - 1) & !(max_align - 1)
}

// ─── Writer context ───────────────────────────────────────────────────────

struct WriterContext {
    buf: Vec<u8>,
    /// DATA1: local fixups (src_rel, dst_rel) — values are section-relative.
    /// Rule 2: sorted by (dst, src) when written via write_local_fixups.
    data1: Vec<(u32, u32)>,
    /// DATA2: global fixups — (target_obj_index, src_rel).
    /// Deferred until all objects written; resolved to (src_rel, 2, dst_rel).
    data2_deferred: Vec<(usize, u32)>,
    /// DATA3: virtual fixups (from_rel, section=0, cn_pos).
    data3: Vec<(u32, u32, u32)>,
    /// object index → data-section-relative offset.
    object_offsets: Vec<usize>,
    /// classname → ClassnameEntry.
    classnames: Vec<ClassnameEntry>,
    classname_map: HashMap<String, usize>,
    cn_byte_pos: usize,
}

impl WriterContext {
    fn new() -> Self {
        Self {
            buf: Vec::new(),
            data1: Vec::new(),
            data2_deferred: Vec::new(),
            data3: Vec::new(),
            object_offsets: Vec::new(),
            classnames: Vec::new(),
            classname_map: HashMap::new(),
            cn_byte_pos: 0,
        }
    }

    /// Get or create classname entry; return the position of the name STRING
    /// within the classnames section (after 4-byte sig + 1-byte separator).
    fn get_classname_pos(&mut self, class_name: &str) -> u32 {
        if let Some(&idx) = self.classname_map.get(class_name) {
            return self.classnames[idx].position as u32;
        }
        let string_pos = self.cn_byte_pos + 5;
        let entry = ClassnameEntry {
            position: string_pos,
            signature: 0,
            name: class_name.to_string(),
        };
        let idx = self.classnames.len();
        self.classnames.push(entry);
        self.classname_map.insert(class_name.to_string(), idx);
        self.cn_byte_pos += 4 + 1 + class_name.len() + 1;
        string_pos as u32
    }

    fn ensure_len(&mut self, end: usize) {
        if end > self.buf.len() {
            self.buf.resize(end, 0u8);
        }
    }

    fn write_at(&mut self, offset: usize, data: &[u8]) {
        let end = offset + data.len();
        self.ensure_len(end);
        self.buf[offset..end].copy_from_slice(data);
    }
}

// ─── Top-level write function ─────────────────────────────────────────────

/// Write an HkxFile to binary HKX v11 packfile bytes.
pub fn write_hkx(hkx_file: &HkxFile, registry: &mut DescriptorRegistry) -> Vec<u8> {
    let mut ctx = WriterContext::new();

    // Phase 0: collect classnames.
    // Always prepend the four Havok reflection-metadata classes so that the
    // root object's class lands at the expected offset (position 75).
    for reflect_cn in &["hkClass", "hkClassMember", "hkClassEnum", "hkClassEnumItem"] {
        ctx.get_classname_pos(reflect_cn);
    }
    for obj in hkx_file.objects() {
        ctx.get_classname_pos(&obj.class_name);
    }

    // Fill in signatures from descriptors.
    let cn_entries: Vec<ClassnameEntry> = ctx
        .classnames
        .iter()
        .map(|entry| {
            let sig = registry
                .get(&entry.name)
                .ok()
                .flatten()
                .and_then(|desc| {
                    u32::from_str_radix(desc.signature.trim_start_matches("0x"), 16).ok()
                })
                .unwrap_or_else(|| {
                    warn!(
                        class = %entry.name,
                        "no descriptor found for class; writing signature=0x00000000"
                    );
                    0
                });
            ClassnameEntry {
                position: entry.position,
                signature: sig,
                name: entry.name.clone(),
            }
        })
        .collect();
    let cn_data = write_classnames(&cn_entries);

    // Compute `contents_class_name_section_offset` (header 0x24).
    let root_class_name = hkx_file
        .objects()
        .first()
        .map(|o| o.class_name.as_str())
        .unwrap_or("hkRootLevelContainer");
    let contents_cn_offset = ctx
        .classname_map
        .get(root_class_name)
        .map(|&idx| ctx.classnames[idx].position as u32)
        .unwrap_or(0);

    // Determine padding_size: preserve from source bytes if known; otherwise
    // derive from whether any object needs 16-byte data alignment (i.e. has a
    // member with a Vector4/Matrix/Transform/Quaternion/QsTransform field).
    //
    // Exception: vanilla FO4 physics packfiles (root = hknpPhysicsSystemData)
    // never use the file-header padding extension — they achieve 16-byte data
    // alignment via per-object position snapping inside the data section.
    // Emitting padding_size>0 here yields a packfile whose section table sits
    // at 0x50 instead of 0x40; the in-game Havok runtime hardcodes 0x40 as the
    // section table start (only animation/behavior HKX loaders honor the
    // padding byte), so the offsets get misread and the broadphase derefs a
    // null compound sub-shape on load. Animation HKX (hkaRagdollInstance,
    // hkaSplineCompressedAnimation, etc.) DOES use padding=16 in vanilla, so
    // we keep the heuristic for those.
    let is_physics_packfile = root_class_name == "hknpPhysicsSystemData";
    let padding_size = if hkx_file.padding_size() > 0 {
        hkx_file.padding_size()
    } else if !is_physics_packfile && needs_sixteen_byte_data_padding(hkx_file, registry) {
        16
    } else {
        0
    };

    // Preserve the source packfile version (v8 = Skyrim/FO3, v11 = FO4).
    // Default to v11 for new files (from_tagxml always sets class_version=11).
    let packfile_version = match hkx_file.class_version() {
        8 => 8u32,
        _ => 11,
    };
    let section_header_size = if packfile_version == 8 {
        0x30usize
    } else {
        0x40
    };
    let header = PackfileHeader {
        version: packfile_version,
        version_name: hkx_file.contents_version().to_string(),
        padding_size,
        pointer_size: 8,
        section_header_size,
        contents_section_index: 2,
        contents_section_offset: 0,
        contents_class_name_section_index: 0,
        contents_class_name_section_offset: contents_cn_offset,
    };

    let header_size = 64 + header.padding_size;
    let section_headers_size = section_header_size * 3;
    let cn_offset = snap_to_16(header_size + section_headers_size);
    let types_offset = snap_to_16(cn_offset + cn_data.len());
    let data_offset = types_offset; // types section is empty

    // Phase 1: write objects to data buffer.
    ctx.object_offsets = vec![0usize; hkx_file.objects().len()];
    let mut pos = 0usize;
    for (obj_idx, obj) in hkx_file.objects().iter().enumerate() {
        if pos > 0 {
            pos = snap_to_16(pos);
        }
        ctx.object_offsets[obj_idx] = pos;
        pos = write_object(&mut ctx, registry, obj, pos);
    }

    // Pad data content to 16-byte boundary.
    while pos % 16 != 0 {
        pos += 1;
    }
    ctx.buf.resize(pos, 0u8);

    // Phase 2: resolve global fixups.
    let resolved_data2: Vec<(u32, u32, u32)> = ctx
        .data2_deferred
        .iter()
        .map(|&(obj_idx, src_rel)| {
            let dst_rel = ctx.object_offsets.get(obj_idx).copied().unwrap_or(0) as u32;
            (src_rel, 0x02u32, dst_rel)
        })
        .collect();

    let data1_bytes = write_local_fixups(&ctx.data1);
    let data2_bytes = write_global_fixups(&resolved_data2);
    let data3_bytes = write_virtual_fixups(&ctx.data3);

    let data_content = ctx.buf;
    let data1_start = data_content.len();
    let data2_start = data1_start + data1_bytes.len();
    let data3_start = data2_start + data2_bytes.len();
    let data_end = data3_start + data3_bytes.len();

    let cn_section = SectionHeader {
        name: "__classnames__".to_string(),
        offset: cn_offset,
        data1: cn_offset + cn_data.len(),
        data2: cn_offset + cn_data.len(),
        data3: cn_offset + cn_data.len(),
        exports: cn_offset + cn_data.len(),
        imports: cn_offset + cn_data.len(),
        end: cn_offset + cn_data.len(),
    };
    let types_section = SectionHeader {
        name: "__types__".to_string(),
        offset: types_offset,
        data1: types_offset,
        data2: types_offset,
        data3: types_offset,
        exports: types_offset,
        imports: types_offset,
        end: types_offset,
    };
    let data_section = SectionHeader {
        name: "__data__".to_string(),
        offset: data_offset,
        data1: data_offset + data1_start,
        data2: data_offset + data2_start,
        data3: data_offset + data3_start,
        exports: data_offset + data_end,
        imports: data_offset + data_end,
        end: data_offset + data_end,
    };

    let write_sh: fn(&SectionHeader) -> Vec<u8> = if packfile_version == 8 {
        write_section_header_v8
    } else {
        write_section_header
    };
    let mut result = Vec::new();
    result.extend_from_slice(&write_header(&header));
    result.extend_from_slice(&write_sh(&cn_section));
    result.extend_from_slice(&write_sh(&types_section));
    result.extend_from_slice(&write_sh(&data_section));

    while result.len() < cn_offset {
        result.push(0x00);
    }
    result.extend_from_slice(&cn_data);

    while result.len() < data_offset {
        result.push(0x00);
    }
    result.extend_from_slice(&data_content);
    result.extend_from_slice(&data1_bytes);
    result.extend_from_slice(&data2_bytes);
    result.extend_from_slice(&data3_bytes);

    result
}

// ─── Object writing ───────────────────────────────────────────────────────

/// Deferred payload item: either a string slot or an array header + member.
enum Deferred {
    /// (ptr_slot_abs, string_value)
    String(usize, String),
    /// (header_abs_off, member, mt)
    Array(usize, HkxMember, MemberTemplate),
    /// (header_abs_off, member, mt) — hkRelArray contents written between the
    /// object body end and other deferred payloads. Backpatches the
    /// header offset (relative to the header position) and 16-byte aligns.
    RelArray(usize, HkxMember, MemberTemplate),
}

/// Write one top-level object at `pos`. Returns the next free position.
fn write_object(
    ctx: &mut WriterContext,
    registry: &mut DescriptorRegistry,
    obj: &HkxObject,
    pos: usize,
) -> usize {
    // DATA3: virtual fixup for this object.
    let cn_pos = ctx.get_classname_pos(&obj.class_name);
    ctx.data3.push((pos as u32, 0x00, cn_pos));

    let all_members = registry
        .get_all_members(&obj.class_name)
        .unwrap_or_default();
    let obj_size = calc_object_size(&all_members, registry);
    ctx.ensure_len(pos + obj_size);

    // Owned deferred list: (member_obj_off_key, deferred_item).
    // member_obj_off_key is used for Rule 7 interleaving.
    let mut callbacks: Vec<(usize, Deferred)> = Vec::new();
    // Direct-pointer fixups: (member_offset_in_obj, tgt_obj_idx, src_abs).
    let mut direct_pointers: Vec<(usize, usize, usize)> = Vec::new();

    let member_map: HashMap<&str, &HkxMember> =
        obj.members.iter().map(|m| (m.name.as_str(), m)).collect();

    for mt in &all_members {
        // Rule 5: SERIALIZE_IGNORED hkArray headers get capacity = 0x80000000.
        if mt.flags == "SERIALIZE_IGNORED" {
            if mt.vtype == HkxType::Array && mt.arrsize == 0 {
                ctx.write_at(pos + mt.offset + 12, &0x80000000u32.to_le_bytes());
            }
            continue;
        }
        let Some(member) = member_map.get(mt.name.as_str()) else {
            continue;
        };
        let abs_off = pos + mt.offset;
        write_member_body(
            ctx,
            registry,
            member,
            mt,
            abs_off,
            pos,
            mt.offset,
            &mut callbacks,
            &mut direct_pointers,
        );
    }

    // Phase: write hkRelArray contents inside the object's allocation —
    // after the fixed body, before other deferred payloads. Each block is
    // 16-byte aligned; the header's offset field (relative to the header
    // position) is backpatched here. Mirrors `py_creation_lib/python/creation_lib/hkxpack/writer.py`
    // lines 410-440.
    let mut relarray_write_pos = snap_to_16(pos + obj_size);
    for (_, deferred) in &callbacks {
        let Deferred::RelArray(header_abs_off, ra_member, ra_mt) = deferred else {
            continue;
        };
        let HkxValue::Array(values) = &ra_member.value else {
            continue;
        };
        if values.is_empty() {
            continue;
        }
        relarray_write_pos = snap_to_16(relarray_write_pos);
        let ra_offset = relarray_write_pos - *header_abs_off;
        ctx.write_at(*header_abs_off + 2, &(ra_offset as u16).to_le_bytes());

        let sub_family = ra_mt.vsubtype.family();
        match sub_family {
            HkxTypeFamily::Direct | HkxTypeFamily::Complex => {
                let elem_size = ra_mt.vsubtype.size().max(1);
                for val in values {
                    let raw = serialize_value(ra_mt.vsubtype, HkxType::Void, val);
                    let copy = raw.len().min(elem_size);
                    ctx.ensure_len(relarray_write_pos + elem_size);
                    ctx.buf[relarray_write_pos..relarray_write_pos + copy]
                        .copy_from_slice(&raw[..copy]);
                    relarray_write_pos += elem_size;
                }
            }
            HkxTypeFamily::Object if !ra_mt.ctype.is_empty() => {
                // Inline-struct elements (e.g. hknpConvexPolytopeShape::Face).
                let struct_members = registry.get_all_members(&ra_mt.ctype).unwrap_or_default();
                let struct_size = calc_inline_struct_size(&struct_members, registry);
                for val in values {
                    let (elem_class, elem_members): (Option<&str>, Option<&Vec<HkxMember>>) =
                        match val {
                            HkxValue::Object(m) => (Some(ra_mt.ctype.as_str()), Some(m)),
                            HkxValue::TypedObject {
                                class_name,
                                members,
                            } => (Some(class_name.as_str()), Some(members)),
                            _ => (None, None),
                        };
                    if let (Some(cls), Some(members)) = (elem_class, elem_members) {
                        let temp = HkxObject {
                            name: None,
                            offset: relarray_write_pos,
                            signature: 0,
                            class_name: cls.to_string(),
                            members: members.clone(),
                        };
                        // RelArray inline-struct elements have no nested
                        // deferred payloads in vanilla FO4 fixtures (faces are
                        // pure scalar tuples). Use a sink so any surprise
                        // pointer/string still routes to DATA2.
                        let mut sink_cbs: Vec<(usize, Deferred)> = Vec::new();
                        let mut sink_dp: Vec<(usize, usize, usize)> = Vec::new();
                        write_inline_struct(
                            ctx,
                            registry,
                            &temp,
                            relarray_write_pos,
                            cls,
                            &mut sink_cbs,
                            &mut sink_dp,
                            relarray_write_pos,
                        );
                        for (_, tgt_idx, src_abs) in sink_dp {
                            ctx.data2_deferred.push((tgt_idx, src_abs as u32));
                        }
                    }
                    relarray_write_pos += struct_size;
                }
            }
            other => {
                // Other sub-families inside a RelArray are not observed in
                // vanilla FO4 fixtures. Warn so the FO76→FO4 migration path or
                // any new descriptor doesn't silently drop a payload.
                warn!(
                    "write_object: RelArray sub-family {:?} ({:?}) not implemented for class {:?} member {:?}; {} elements skipped",
                    other,
                    ra_mt.vsubtype,
                    obj.class_name,
                    ra_member.name,
                    values.len(),
                );
            }
        }
    }

    let body_end = snap_to_16(pos + obj_size);
    let next_pos = if relarray_write_pos > body_end {
        snap_to_16(relarray_write_pos)
    } else {
        body_end
    };
    let mut write_pos = next_pos;

    // Rule 7: interleave direct pointers and array/string callbacks in
    // member-offset order.
    let mut dp_idx = 0;
    for (cb_off_key, deferred) in &callbacks {
        if matches!(deferred, Deferred::RelArray(_, _, _)) {
            continue;
        }
        while dp_idx < direct_pointers.len() && direct_pointers[dp_idx].0 < *cb_off_key {
            let (_, tgt_idx, src_abs) = direct_pointers[dp_idx];
            ctx.data2_deferred.push((tgt_idx, src_abs as u32));
            dp_idx += 1;
        }
        match deferred {
            Deferred::String(ptr_slot_abs, str_val) => {
                let str_bytes: Vec<u8> = str_val.bytes().chain(std::iter::once(0u8)).collect();
                ctx.write_at(write_pos, &str_bytes);
                ctx.data1.push((*ptr_slot_abs as u32, write_pos as u32));
                write_pos += str_bytes.len();
                while write_pos % 16 != 0 {
                    write_pos += 1;
                }
                ctx.ensure_len(write_pos);
            }
            Deferred::Array(header_abs_off, arr_member, arr_mt) => {
                let obj_pos_for_array = *header_abs_off - arr_mt.offset;
                write_pos = write_array_contents(
                    ctx,
                    registry,
                    arr_member,
                    arr_mt,
                    obj_pos_for_array,
                    write_pos,
                );
            }
            Deferred::RelArray(_, _, _) => unreachable!(),
        }
    }
    while dp_idx < direct_pointers.len() {
        let (_, tgt_idx, src_abs) = direct_pointers[dp_idx];
        ctx.data2_deferred.push((tgt_idx, src_abs as u32));
        dp_idx += 1;
    }

    write_pos
}

/// Write the fixed-size body of one member into `ctx.buf`.
/// `obj_pos` is the start of the containing object.
/// `member_off_key` is used for Rule 7 callback ordering.
fn write_member_body(
    ctx: &mut WriterContext,
    registry: &mut DescriptorRegistry,
    member: &HkxMember,
    mt: &MemberTemplate,
    abs_off: usize,
    obj_pos: usize,
    member_off_key: usize,
    callbacks: &mut Vec<(usize, Deferred)>,
    direct_pointers: &mut Vec<(usize, usize, usize)>,
) {
    // PendingPtr is the bake-pipeline forward-reference sentinel; reaching the
    // writer with one means the resolve pass missed it. Silently zeroing the
    // pointer in release would surface as a null deref in-game far from here.
    debug_assert!(
        !matches!(&member.value, HkxValue::PendingPtr(_)),
        "write_member_body: unresolved PendingPtr on member {:?}",
        member.name,
    );
    match &member.value {
        // ── Direct scalars ───────────────────────────────────────────────
        HkxValue::Bool(_)
        | HkxValue::I8(_)
        | HkxValue::U8(_)
        | HkxValue::I16(_)
        | HkxValue::U16(_)
        | HkxValue::I32(_)
        | HkxValue::U32(_)
        | HkxValue::I64(_)
        | HkxValue::U64(_)
        | HkxValue::F32(_)
        | HkxValue::Half(_) => {
            ctx.write_at(
                abs_off,
                &serialize_value(mt.vtype, mt.vsubtype, &member.value),
            );
        }

        // ── SIMD / complex ───────────────────────────────────────────────
        HkxValue::F32List(floats) if mt.arrsize == 0 => {
            // COMPLEX type (Vector4, Quaternion, etc.).
            ctx.write_at(
                abs_off,
                &serialize_value(mt.vtype, mt.vsubtype, &member.value),
            );
        }
        HkxValue::F32List(floats) => {
            // Fixed C array of floats.
            let elem_size = mt.vtype.size();
            for (j, f) in floats.iter().enumerate() {
                ctx.write_at(abs_off + j * elem_size, &f.to_le_bytes());
            }
        }

        // ── Fixed C-array (non-hkArray arrsize) ─────────────────────────
        HkxValue::Array(values)
            if mt.arrsize > 0
                && !matches!(
                    mt.vtype,
                    HkxType::Array | HkxType::SimpleArray | HkxType::RelArray
                ) =>
        {
            if mt.vtype == HkxType::Struct && !mt.ctype.is_empty() {
                // Fixed C-array of inline structs — write each element via
                // write_inline_struct so nested members are serialized correctly.
                let elem_members = registry.get_all_members(&mt.ctype).unwrap_or_default();
                let elem_size = calc_inline_struct_size(&elem_members, registry).max(1);
                for (j, val) in values.iter().enumerate() {
                    let elem_base = abs_off + j * elem_size;
                    if let HkxValue::Object(inner_members) = val {
                        let temp = HkxObject {
                            name: None,
                            offset: elem_base,
                            signature: 0,
                            class_name: mt.ctype.clone(),
                            members: inner_members.clone(),
                        };
                        write_inline_struct(
                            ctx,
                            registry,
                            &temp,
                            elem_base,
                            &mt.ctype,
                            callbacks,
                            direct_pointers,
                            obj_pos,
                        );
                    }
                }
            } else if mt.vtype == HkxType::Pointer {
                let elem_size = mt.vtype.size().max(1);
                for (j, val) in values.iter().enumerate() {
                    if let HkxValue::Pointer(Some(tgt_idx)) = val {
                        direct_pointers.push((
                            member_off_key + j,
                            *tgt_idx,
                            abs_off + j * elem_size,
                        ));
                    }
                }
            } else {
                let elem_size = if mt.vtype.family() == HkxTypeFamily::Enum {
                    mt.vsubtype.size().max(1)
                } else {
                    mt.vtype.size().max(1)
                };
                for (j, val) in values.iter().enumerate() {
                    let raw = serialize_value(mt.vtype, mt.vsubtype, val);
                    let copy = raw.len().min(elem_size);
                    ctx.write_at(abs_off + j * elem_size, &raw[..copy]);
                }
            }
        }

        // ── Inline struct (TYPE_STRUCT with arrsize==0) ──────────────────
        HkxValue::Object(inner_members) if mt.vtype == HkxType::Struct && mt.arrsize == 0 => {
            let temp = HkxObject {
                name: None,
                offset: abs_off,
                signature: 0,
                class_name: mt.ctype.clone(),
                members: inner_members.clone(),
            };
            write_inline_struct(
                ctx,
                registry,
                &temp,
                abs_off,
                &mt.ctype,
                callbacks,
                direct_pointers,
                obj_pos,
            );
        }
        HkxValue::TypedObject {
            class_name: typed_class,
            members: inner_members,
        } if mt.vtype == HkxType::Struct && mt.arrsize == 0 => {
            // Synthesizer-supplied class name overrides the parent
            // template (used by FO76→FO4 migration when a single
            // `atoms` array holds heterogeneous-class entries).
            let temp = HkxObject {
                name: None,
                offset: abs_off,
                signature: 0,
                class_name: typed_class.clone(),
                members: inner_members.clone(),
            };
            write_inline_struct(
                ctx,
                registry,
                &temp,
                abs_off,
                typed_class,
                callbacks,
                direct_pointers,
                obj_pos,
            );
        }

        // ── Pointer ──────────────────────────────────────────────────────
        HkxValue::Pointer(Some(tgt_idx)) => {
            // Rule 7: record for deferred DATA2 emission in member-offset order.
            direct_pointers.push((member_off_key, *tgt_idx, abs_off));
        }
        HkxValue::Pointer(None) => {}

        // ── String ───────────────────────────────────────────────────────
        HkxValue::String { value, is_null } if !is_null => {
            callbacks.push((member_off_key, Deferred::String(abs_off, value.clone())));
        }
        HkxValue::String { .. } => {}

        // ── hkRelArray ───────────────────────────────────────────────────
        HkxValue::Array(values) if mt.vtype == HkxType::RelArray => {
            // Header: (size: u16, offset: u16). Offset is backpatched after
            // the object body once we know where the data lives.
            ctx.write_at(abs_off, &(values.len() as u16).to_le_bytes());
            ctx.write_at(abs_off + 2, &0u16.to_le_bytes());
            callbacks.push((
                member_off_key,
                Deferred::RelArray(abs_off, member.clone(), mt.clone()),
            ));
        }

        // ── hkArray / hkSimpleArray ───────────────────────────────────────
        HkxValue::Array(values) if matches!(mt.vtype, HkxType::Array | HkxType::SimpleArray) => {
            // Standard array: [ptr(8)][size(4)][capacity(4)|0x80000000]
            let sz = values.len() as u32;
            ctx.write_at(abs_off + 8, &sz.to_le_bytes());
            ctx.write_at(abs_off + 12, &(sz | 0x80000000u32).to_le_bytes());
            // Rule 5: defer unconditionally (empty arrays still get DATA1 fixup).
            // The actual DATA1 fixup is added/removed inside write_array_contents.
            callbacks.push((
                member_off_key,
                Deferred::Array(abs_off, member.clone(), mt.clone()),
            ));
        }

        // Any HkxValue variant not matched above writes zero bytes. Loud warn
        // so the next added variant fails noisily instead of in-game.
        other => warn!(
            "write_member_body: unhandled value variant {} on member {:?} (vtype={:?}, vsubtype={:?}, arrsize={}); destination at offset {} left zero-initialised",
            other.variant_name(),
            member.name,
            mt.vtype,
            mt.vsubtype,
            mt.arrsize,
            abs_off,
        ),
    }
}

/// Write an inline struct's members at `base_off`.
/// Strings → Deferred::String callbacks.
/// Arrays  → Deferred::Array callbacks.
/// Pointers → direct_pointers.
fn write_inline_struct(
    ctx: &mut WriterContext,
    registry: &mut DescriptorRegistry,
    obj: &HkxObject,
    base_off: usize,
    class_name: &str,
    callbacks: &mut Vec<(usize, Deferred)>,
    direct_pointers: &mut Vec<(usize, usize, usize)>,
    obj_pos: usize,
) {
    let all_members = registry.get_all_members(class_name).unwrap_or_default();
    let member_map: HashMap<&str, &HkxMember> =
        obj.members.iter().map(|m| (m.name.as_str(), m)).collect();

    for mt in &all_members {
        if mt.flags == "SERIALIZE_IGNORED" {
            if mt.vtype == HkxType::Array && mt.arrsize == 0 {
                ctx.write_at(base_off + mt.offset + 12, &0x80000000u32.to_le_bytes());
            }
            continue;
        }
        let Some(member) = member_map.get(mt.name.as_str()) else {
            continue;
        };
        let abs_off = base_off + mt.offset;
        // member_off_key relative to obj_pos for Rule 7 ordering.
        let off_key = abs_off.saturating_sub(obj_pos);
        write_member_body(
            ctx,
            registry,
            member,
            mt,
            abs_off,
            obj_pos,
            off_key,
            callbacks,
            direct_pointers,
        );
    }
}

// ─── Array contents ───────────────────────────────────────────────────────

/// Write array element data at `write_pos`. Returns the next free position.
///
/// Rules addressed:
/// - Rule 1: inline struct arrays use per-element interleave of deferred payloads.
/// - Rule 6: consecutive STRINGPTR bodies use 2-byte padding (not 8).
fn write_array_contents(
    ctx: &mut WriterContext,
    registry: &mut DescriptorRegistry,
    member: &HkxMember,
    mt: &MemberTemplate,
    obj_pos: usize,
    write_pos: usize,
) -> usize {
    let values = match &member.value {
        HkxValue::Array(v) => v,
        _ => return write_pos,
    };

    // Snap to 16-byte boundary before array data.
    let mut write_pos = snap_to_16(write_pos);

    // DATA1 fixup: array header pointer → first element.
    // Rule 2: the (src, dst) pair is added; write_local_fixups sorts by (dst,src).
    let arr_header_rel = (obj_pos + mt.offset) as u32;
    ctx.data1.push((arr_header_rel, write_pos as u32));

    // Empty arrays: no DATA1 fixup and no data.
    if values.is_empty() {
        ctx.data1.pop();
        return write_pos;
    }

    let sub_family = mt.vsubtype.family();

    match sub_family {
        HkxTypeFamily::Direct | HkxTypeFamily::Complex => {
            let elem_size = mt.vsubtype.size().max(1);
            for val in values {
                let raw = serialize_value(mt.vsubtype, HkxType::Void, val);
                let copy = raw.len().min(elem_size);
                ctx.ensure_len(write_pos + elem_size);
                ctx.buf[write_pos..write_pos + copy].copy_from_slice(&raw[..copy]);
                write_pos += elem_size;
            }
        }

        HkxTypeFamily::Object => {
            // Rule 1: per-element interleave of deferred payloads.
            let struct_members = if mt.ctype.is_empty() {
                Vec::new()
            } else {
                registry.get_all_members(&mt.ctype).unwrap_or_default()
            };
            // Stride is computed from the parent's ctype (uniform across
            // the array). TypedObject elements override the *class* used
            // to look up descriptors but inherit the parent's stride —
            // required so heterogeneous-class atoms still pack contiguously.
            let struct_size = calc_inline_struct_size(&struct_members, registry);

            // Phase A: write all element bodies back-to-back.
            let mut body_pos = write_pos;
            // Collect per-element deferred items.
            let mut per_elem: Vec<Vec<(usize, Deferred)>> = Vec::new();
            let mut per_elem_dp: Vec<Vec<(usize, usize, usize)>> = Vec::new();

            for val in values {
                let mut elem_cbs: Vec<(usize, Deferred)> = Vec::new();
                let mut elem_dp: Vec<(usize, usize, usize)> = Vec::new();

                match val {
                    HkxValue::Object(inner_members) => {
                        let temp = HkxObject {
                            name: None,
                            offset: body_pos,
                            signature: 0,
                            class_name: mt.ctype.clone(),
                            members: inner_members.clone(),
                        };
                        write_inline_struct(
                            ctx,
                            registry,
                            &temp,
                            body_pos,
                            &mt.ctype,
                            &mut elem_cbs,
                            &mut elem_dp,
                            body_pos,
                        );
                    }
                    HkxValue::TypedObject {
                        class_name: typed_class,
                        members: inner_members,
                    } => {
                        // Heterogeneous-class element (FO76→FO4 atoms).
                        // Look up descriptor with the carried class name.
                        let temp = HkxObject {
                            name: None,
                            offset: body_pos,
                            signature: 0,
                            class_name: typed_class.clone(),
                            members: inner_members.clone(),
                        };
                        write_inline_struct(
                            ctx,
                            registry,
                            &temp,
                            body_pos,
                            typed_class,
                            &mut elem_cbs,
                            &mut elem_dp,
                            body_pos,
                        );
                    }
                    // Tagfile reader unwraps single-field wrapper structs to raw values.
                    _ if !struct_members.is_empty() => {
                        let first_mt = &struct_members[0];
                        let raw = serialize_value(first_mt.vtype, first_mt.vsubtype, val);
                        ctx.write_at(body_pos + first_mt.offset, &raw);
                    }
                    other => warn!(
                        "write_array_contents: cannot write inline-struct element {} in {:?} array (ctype={:?}, struct_members empty); element zero-initialised",
                        other.variant_name(),
                        member.name,
                        mt.ctype,
                    ),
                }

                per_elem.push(elem_cbs);
                per_elem_dp.push(elem_dp);
                body_pos += struct_size;
            }
            write_pos = body_pos;

            // Snap to 16-byte boundary before the first deferred payload so
            // that per-element string bodies align with vanilla FO4 layout
            // (element bodies may leave write_pos on an 8-byte boundary).
            write_pos = snap_to_16(write_pos);

            // Phase B: per-element interleave of deferred payloads (Rule 1).
            for (elem_cbs, elem_dp) in per_elem.into_iter().zip(per_elem_dp.into_iter()) {
                // Resolve direct pointers from this element first.
                for (_, tgt_idx, src_abs) in elem_dp {
                    ctx.data2_deferred.push((tgt_idx, src_abs as u32));
                }
                // Then emit deferred strings and nested array callbacks in callback order.
                // The elem_cbs are already in member-offset order (from write_inline_struct).
                for (_off_key, deferred) in elem_cbs {
                    match deferred {
                        Deferred::String(ptr_slot_abs, str_val) => {
                            // String slots within inline struct elements use 16-byte stride.
                            let str_bytes: Vec<u8> =
                                str_val.bytes().chain(std::iter::once(0u8)).collect();
                            ctx.data1.push((ptr_slot_abs as u32, write_pos as u32));
                            ctx.write_at(write_pos, &str_bytes);
                            write_pos += str_bytes.len();
                            while write_pos % 16 != 0 {
                                write_pos += 1;
                            }
                            ctx.ensure_len(write_pos);
                        }
                        Deferred::Array(header_abs, arr_member, arr_mt) => {
                            // Nested array within the inline struct element.
                            let arr_obj_pos = header_abs - arr_mt.offset;
                            write_pos = write_array_contents(
                                ctx,
                                registry,
                                &arr_member,
                                &arr_mt,
                                arr_obj_pos,
                                write_pos,
                            );
                        }
                        Deferred::RelArray(_, _, _) => {
                            // RelArray payload emission not yet implemented.
                        }
                    }
                }
            }
        }

        HkxTypeFamily::String => {
            // Rule 6: consecutive STRINGPTR bodies use 2-byte padding.
            let ptrs_start = write_pos;
            write_pos += values.len() * 8; // reserve pointer slots
            for (i, val) in values.iter().enumerate() {
                if let HkxValue::String { value, is_null } = val {
                    if !is_null && !value.is_empty() {
                        let str_bytes: Vec<u8> =
                            value.bytes().chain(std::iter::once(0u8)).collect();
                        ctx.data1
                            .push(((ptrs_start + i * 8) as u32, write_pos as u32));
                        ctx.write_at(write_pos, &str_bytes);
                        write_pos += str_bytes.len();
                        // Rule 6: 2-byte alignment.
                        if write_pos % 2 != 0 {
                            write_pos += 1;
                        }
                    }
                }
            }
        }

        HkxTypeFamily::Pointer => {
            for (i, val) in values.iter().enumerate() {
                if let HkxValue::Pointer(Some(tgt_idx)) = val {
                    ctx.data2_deferred
                        .push((*tgt_idx, (write_pos + i * 8) as u32));
                }
            }
            write_pos += values.len() * 8;
        }

        HkxTypeFamily::Enum => {
            for val in values {
                let raw = match val {
                    HkxValue::I32(v) => v.to_le_bytes().to_vec(),
                    HkxValue::U32(v) => v.to_le_bytes().to_vec(),
                    other => {
                        warn!(
                            "write_array_contents: enum-array element {} on {:?} cannot be widened to i32; writing 0",
                            other.variant_name(),
                            member.name,
                        );
                        0i32.to_le_bytes().to_vec()
                    }
                };
                ctx.write_at(write_pos, &raw);
                write_pos += 4;
            }
        }

        other => warn!(
            "write_array_contents: array sub-family {:?} on {:?} (vsubtype={:?}) not implemented; {} elements skipped",
            other,
            member.name,
            mt.vsubtype,
            values.len(),
        ),
    }

    write_pos
}

/// Return true when any member of any object has a type that requires 16-byte
/// data alignment (Vector4, Quaternion, Matrix3, Matrix4, Transform,
/// QsTransform), which in turn requires a non-zero `padding_size` header field.
fn needs_sixteen_byte_data_padding(hkx_file: &HkxFile, registry: &mut DescriptorRegistry) -> bool {
    const ALIGNED_TYPES: &[HkxType] = &[
        HkxType::Vector4,
        HkxType::Quaternion,
        HkxType::Matrix3,
        HkxType::Matrix4,
        HkxType::Transform,
        HkxType::QsTransform,
    ];
    for obj in hkx_file.objects() {
        let Ok(members) = registry.get_all_members(&obj.class_name) else {
            continue;
        };
        for mt in &members {
            if ALIGNED_TYPES.contains(&mt.vtype) || ALIGNED_TYPES.contains(&mt.vsubtype) {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snap_to_16_internal() {
        assert_eq!(snap_to_16(0), 0);
        assert_eq!(snap_to_16(1), 16);
        assert_eq!(snap_to_16(15), 16);
        assert_eq!(snap_to_16(16), 16);
        assert_eq!(snap_to_16(17), 32);
    }
}
