use crate::error::{HavokError, HavokResult};

use super::model::{HkxFile, HkxMember, HkxObject};
use super::types::{HkxType, HkxValue};

const TAG0_MAGIC_OFFSET: usize = 4;
const HFF_HEADER_SIZE: usize = 8;
const ITEM_VAR0: u8 = 1;
const ITEM_VARN: u8 = 2;
const ITEM_NOTE: u8 = 3;
// TAG0 type ids are file-controlled. Keep this high enough for real fixtures,
// but bounded so malformed VLE values cannot drive unbounded allocations.
const MAX_TAG_TYPES: usize = 100_000;
const MAX_NESTED_OBJECT_DEPTH: usize = 64;
const MAX_FIELD_COLLECTION_DEPTH: usize = 256;
const MAX_NOTE_INDIRECTION_DEPTH: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagfileSection {
    pub tag: String,
    pub offset: usize,
    pub size: usize,
    pub content_offset: usize,
    pub content_size: usize,
    pub scope: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagField {
    pub name: String,
    pub type_id: usize,
    pub offset: usize,
    pub flags: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagType {
    pub id: usize,
    pub name: String,
    pub parent_id: usize,
    pub kind: u8,
    pub subtype_id: usize,
    pub size: usize,
    pub align: usize,
    pub version: i64,
    pub format_value: u64,
    pub signed: bool,
    pub fields: Vec<TagField>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagTypeRegistry {
    pub types: Vec<TagType>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagfileItem {
    pub kind: u8,
    pub flags: u8,
    pub type_id: usize,
    pub offset: usize,
    pub count: usize,
}

#[derive(Debug, Clone)]
pub struct Tagfile {
    pub sdk_version: String,
    pub contents_version: String,
    pub sections: Vec<TagfileSection>,
    pub type_strings: Vec<String>,
    pub field_strings: Vec<String>,
    pub type_registry: TagTypeRegistry,
    pub items: Vec<TagfileItem>,
    pub pointer_offsets: Vec<usize>,
    source_bytes: Vec<u8>,
}

impl Tagfile {
    pub fn from_synthetic_parts(
        sdk_version: impl Into<String>,
        contents_version: impl Into<String>,
        sections: Vec<TagfileSection>,
        type_registry: TagTypeRegistry,
        items: Vec<TagfileItem>,
        source_bytes: Vec<u8>,
    ) -> Self {
        Self {
            sdk_version: sdk_version.into(),
            contents_version: contents_version.into(),
            sections,
            type_strings: Vec::new(),
            field_strings: Vec::new(),
            type_registry,
            items,
            pointer_offsets: Vec::new(),
            source_bytes,
        }
    }

    pub fn section(&self, tag: &str) -> Option<&TagfileSection> {
        self.sections.iter().find(|section| section.tag == tag)
    }

    /// Synthetic-test escape hatch: install a PTCH set after construction.
    /// PTCH membership is what gates kind=3 fields between item-indexed
    /// strings and scratch ints — see `materialize_string_field_value`.
    pub fn set_pointer_offsets_for_test(&mut self, offsets: Vec<usize>) {
        self.pointer_offsets = offsets;
    }

    pub fn materialize_hkx(&self) -> HavokResult<HkxFile> {
        let data = section_bytes(&self.source_bytes, &self.sections, "DATA").unwrap_or(&[]);
        let mut objects = Vec::new();
        let item_to_object_index = self.materialized_object_item_indices()?;

        for (item_index, item) in self.items.iter().enumerate().skip(1) {
            let Some(object_index) = item_to_object_index[item_index] else {
                continue;
            };
            let tag_type = self.tag_type(item.type_id)?;
            let fields = self.collect_fields(tag_type)?;

            let class_name = resolve_class_name(tag_type, &self.type_registry);
            let mut members = Vec::with_capacity(fields.len());
            for field in fields {
                let field_type = self.tag_type(field.type_id)?;
                let field_offset = item.offset.checked_add(field.offset).ok_or_else(|| {
                    HavokError::InvalidInput("TAG0 field offset overflow".to_string())
                })?;
                let value = self
                    .materialize_field_value(
                        data,
                        field_offset,
                        field_type,
                        &item_to_object_index,
                        &mut Vec::new(),
                        &mut vec![field.name.clone()],
                    )
                    .map_err(|error| {
                        HavokError::InvalidInput(format!(
                            "TAG0 field materialization failed for class {class_name}, member {}, type id {} ({}) kind {}: {error}",
                            field.name, field_type.id, field_type.name, field_type.kind
                        ))
                    })?;
                members.push(HkxMember {
                    name: field.name.clone(),
                    value,
                });
            }

            objects.push(HkxObject {
                name: Some(format!("#{:04}", object_index + 1)),
                offset: item.offset,
                signature: u32::try_from(tag_type.version).unwrap_or(0),
                class_name,
                members,
            });
        }

        Ok(HkxFile::from_tagxml(
            11,
            self.contents_version.clone(),
            objects,
        ))
    }

    fn materialized_object_item_indices(&self) -> HavokResult<Vec<Option<usize>>> {
        let mut item_to_object_index = vec![None; self.items.len()];
        let mut object_index = 0;
        for (item_index, item) in self.items.iter().enumerate().skip(1) {
            if item.kind != ITEM_VAR0 {
                continue;
            }
            let tag_type = self.tag_type(item.type_id)?;
            if self.collect_fields(tag_type)?.is_empty() {
                continue;
            }
            item_to_object_index[item_index] = Some(object_index);
            object_index += 1;
        }
        Ok(item_to_object_index)
    }

    /// Return the raw source bytes unchanged.
    ///
    /// The name "source_bytes_clone" is intentional: there is no native TAG0
    /// writer, so this method never re-serialises the model. Callers that
    /// mutate the materialised `HkxFile` and expect the changes to persist
    /// must convert to a packfile via `materialize()` + `HkxFile::save()`.
    pub fn source_bytes_clone(&self) -> Vec<u8> {
        self.source_bytes.clone()
    }

    /// Leading year of the `SDKV` string (`20190200` → 2019), 0 when absent.
    fn sdk_year(&self) -> u32 {
        self.sdk_version
            .get(..4)
            .and_then(|year| year.parse().ok())
            .unwrap_or(0)
    }

    fn tag_type(&self, type_id: usize) -> HavokResult<&TagType> {
        self.type_registry.types.get(type_id).ok_or_else(|| {
            HavokError::InvalidInput(format!("TAG0 type id {type_id} is not in registry"))
        })
    }

    fn collect_fields<'a>(&'a self, tag_type: &'a TagType) -> HavokResult<Vec<&'a TagField>> {
        let mut fields = Vec::new();
        self.collect_fields_into(tag_type, &mut fields, &mut Vec::new(), 0)?;
        Ok(fields)
    }

    fn collect_fields_into<'a>(
        &'a self,
        tag_type: &'a TagType,
        fields: &mut Vec<&'a TagField>,
        visited: &mut Vec<usize>,
        depth: usize,
    ) -> HavokResult<()> {
        if depth >= MAX_FIELD_COLLECTION_DEPTH {
            return Err(HavokError::InvalidInput(format!(
                "TAG0 field inheritance depth limit exceeded at type {}",
                tag_type.name
            )));
        }
        if visited.contains(&tag_type.id) {
            return Ok(());
        }
        visited.push(tag_type.id);
        if tag_type.parent_id != tag_type.id {
            if let Some(parent) = self.type_registry.types.get(tag_type.parent_id) {
                if parent.id != 0 || !parent.fields.is_empty() {
                    self.collect_fields_into(parent, fields, visited, depth + 1)?;
                }
            }
        }
        fields.extend(tag_type.fields.iter());
        Ok(())
    }

    fn materialize_field_value(
        &self,
        data: &[u8],
        offset: usize,
        tag_type: &TagType,
        item_to_object_index: &[Option<usize>],
        visited: &mut Vec<usize>,
        path: &mut Vec<String>,
    ) -> HavokResult<HkxValue> {
        if tag_type.kind == 7 {
            return self.materialize_object_value(
                data,
                offset,
                tag_type,
                item_to_object_index,
                visited,
                path,
            );
        }
        if tag_type.kind == 8 {
            return self.materialize_array_value(
                data,
                offset,
                tag_type,
                item_to_object_index,
                visited,
                path,
            );
        }
        if tag_type.kind == 6 {
            return self.materialize_pointer_value(data, offset, tag_type, item_to_object_index);
        }
        if tag_type.kind == 3 {
            return self.materialize_string_field_value(data, offset, tag_type);
        }
        // kind 1 (KIND_DECL) and kind 9 (KIND_TYPE) are type-system metadata
        // entries that can appear as struct fields in some Havok schemas but
        // carry no data payload in the DATA section. Skip them silently.
        if tag_type.kind == 1 || tag_type.kind == 9 {
            return Ok(HkxValue::Void);
        }
        materialize_scalar_or_string_value(data, offset, tag_type, &self.type_registry)
    }

    /// Resolve a kind=3 (KIND_STRING) field. When the field offset is in the
    /// PTCH set, the u32 there is an item index for a VARN char payload; the
    /// VARN bytes decode as a tolerant ASCII string. When the offset is *not*
    /// in PTCH, hkStringPtr is being reused as a scratch int field — fall
    /// through to the scalar path (signed int32 by FORMAT semantics).
    fn materialize_string_field_value(
        &self,
        data: &[u8],
        offset: usize,
        tag_type: &TagType,
    ) -> HavokResult<HkxValue> {
        let in_ptch = self.pointer_offsets.iter().any(|p| *p == offset);
        if !in_ptch {
            return materialize_scalar_or_string_fallback_int(data, offset, tag_type);
        }
        materialize_string_via_item(data, offset, tag_type, &self.items)
    }

    fn materialize_pointer_value(
        &self,
        data: &[u8],
        offset: usize,
        tag_type: &TagType,
        item_to_object_index: &[Option<usize>],
    ) -> HavokResult<HkxValue> {
        if !matches!(tag_type.size, 4 | 8) {
            return Err(HavokError::InvalidInput(format!(
                "TAG0 pointer type {} has unsupported size {}",
                tag_type.name, tag_type.size
            )));
        }
        let end = offset.checked_add(tag_type.size).ok_or_else(|| {
            HavokError::InvalidInput("TAG0 pointer field offset overflow".to_string())
        })?;
        let raw = data.get(offset..end).ok_or_else(|| {
            HavokError::InvalidInput(format!(
                "TAG0 pointer field at offset {offset} size {} is outside DATA",
                tag_type.size
            ))
        })?;
        let item_index = u32::from_le_bytes(
            raw[0..4]
                .try_into()
                .expect("4-byte slice after bounds check"),
        ) as usize;
        if item_index == 0 {
            return Ok(HkxValue::Pointer(None));
        }
        if item_index >= self.items.len() {
            return Err(HavokError::InvalidInput(format!(
                "TAG0 pointer item index {item_index} is not in ITEM table"
            )));
        }
        // KIND_NOTE items are annotation aliases: their `count` is the index
        // of the real object item. SDK ref: hkTagfileReadFormat.cpp:540.
        // Walk the indirection chain (cap depth so a malformed file with a
        // NOTE→NOTE→… cycle cannot stack-overflow the reader).
        let mut resolved = item_index;
        for _ in 0..MAX_NOTE_INDIRECTION_DEPTH {
            if self.items[resolved].kind != ITEM_NOTE {
                break;
            }
            let next = self.items[resolved].count;
            if next == 0 || next >= self.items.len() {
                return Err(HavokError::InvalidInput(format!(
                    "TAG0 KIND_NOTE item {resolved} indirection target {next} is out of range"
                )));
            }
            if next == resolved {
                return Err(HavokError::InvalidInput(format!(
                    "TAG0 KIND_NOTE item {resolved} indirection points to itself"
                )));
            }
            resolved = next;
        }
        if self.items[resolved].kind == ITEM_NOTE {
            return Err(HavokError::InvalidInput(format!(
                "TAG0 KIND_NOTE indirection chain starting at {item_index} exceeds depth {MAX_NOTE_INDIRECTION_DEPTH}"
            )));
        }
        let Some(object_index) = item_to_object_index[resolved] else {
            return Err(HavokError::InvalidInput(format!(
                "TAG0 pointer item index {item_index} (resolved {resolved}) does not reference a materialized VAR0 object"
            )));
        };

        Ok(HkxValue::Pointer(Some(object_index)))
    }

    fn materialize_object_value(
        &self,
        data: &[u8],
        offset: usize,
        tag_type: &TagType,
        item_to_object_index: &[Option<usize>],
        visited: &mut Vec<usize>,
        path: &mut Vec<String>,
    ) -> HavokResult<HkxValue> {
        if visited.len() >= MAX_NESTED_OBJECT_DEPTH {
            return Err(HavokError::InvalidInput(format!(
                "TAG0 nested object materialization depth limit exceeded at path {}",
                member_path(path)
            )));
        }
        if visited.contains(&tag_type.id) {
            return Err(HavokError::InvalidInput(format!(
                "TAG0 recursive nested object type cycle: {}",
                self.type_cycle_path(visited, tag_type.id)
            )));
        }
        if tag_type.size == 0 && !tag_type.fields.is_empty() {
            return Err(HavokError::InvalidInput(format!(
                "TAG0 nested object {} has size 0 with fields at path {}",
                tag_type.name,
                member_path(path)
            )));
        }
        if tag_type.size > 0 {
            let end = offset.checked_add(tag_type.size).ok_or_else(|| {
                HavokError::InvalidInput("TAG0 nested object offset overflow".to_string())
            })?;
            data.get(offset..end).ok_or_else(|| {
                HavokError::InvalidInput(format!(
                    "TAG0 nested object {} at offset {offset} size {} is outside DATA",
                    tag_type.name, tag_type.size
                ))
            })?;
        }

        visited.push(tag_type.id);
        let mut members = Vec::new();
        for field in self.collect_fields(tag_type)? {
            let field_type = self.tag_type(field.type_id)?;
            // A nested field whose type has no materialized size (e.g. FO76
            // typedef'd primitives like hknpConstraintId that were not
            // parent-resolved) is omitted from the parent struct. Erroring here
            // breaks FO76 ragdoll conversion (snallygaster
            // hknpRagdollData.constraintCinfos.desiredConstraintId).
            let Some(field_size) = nested_field_size(field_type, &self.type_registry) else {
                continue;
            };
            let field_end = field.offset.checked_add(field_size).ok_or_else(|| {
                HavokError::InvalidInput(format!(
                    "TAG0 nested field path {} offset overflow",
                    member_path_with(path, &field.name)
                ))
            })?;
            if field_end > tag_type.size {
                // Field declares a size that doesn't fit within the parent
                // struct's declared size. Match Python's tolerance and skip
                // — this can happen with mis-sized typedef chains in FO76.
                continue;
            }
            let field_offset = offset.checked_add(field.offset).ok_or_else(|| {
                HavokError::InvalidInput("TAG0 nested field offset overflow".to_string())
            })?;
            path.push(field.name.clone());
            let value = self
                .materialize_field_value(
                    data,
                    field_offset,
                    field_type,
                    item_to_object_index,
                    visited,
                    path,
                )
                .map_err(|error| {
                    HavokError::InvalidInput(format!(
                        "TAG0 nested member path {}: {error}",
                        member_path(path)
                    ))
                })?;
            path.pop();
            members.push(HkxMember {
                name: field.name.clone(),
                value,
            });
        }
        visited.pop();

        Ok(HkxValue::Object(members))
    }

    fn type_cycle_path(&self, visited: &[usize], repeated_type_id: usize) -> String {
        let start = visited
            .iter()
            .position(|type_id| *type_id == repeated_type_id)
            .unwrap_or(0);
        let mut names: Vec<_> = visited[start..]
            .iter()
            .map(|type_id| self.type_name(*type_id))
            .collect();
        names.push(self.type_name(repeated_type_id));
        names.join(" -> ")
    }

    fn type_name(&self, type_id: usize) -> String {
        self.type_registry
            .types
            .get(type_id)
            .map(|tag_type| tag_type.name.clone())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| format!("type {type_id}"))
    }

    fn materialize_array_value(
        &self,
        data: &[u8],
        offset: usize,
        tag_type: &TagType,
        item_to_object_index: &[Option<usize>],
        visited: &mut Vec<usize>,
        path: &mut Vec<String>,
    ) -> HavokResult<HkxValue> {
        // T[N] (fixed-size inline array) stores its elements inline at the
        // field offset with no runtime header; the byte length lives in
        // tag_type.size and N = size / element_stride. This is distinct from
        // hkArray, which uses a synthetic 16-byte header pointing at a VARN
        // payload. FO76 uses T[N] for fields like
        // hkbGeneratorPartitionInfo.boneMask (hkUint32[8] = 32 bytes).
        if tag_type.name == "T[N]" {
            return self.materialize_inline_fixed_array(
                data,
                offset,
                tag_type,
                item_to_object_index,
                visited,
                path,
            );
        }
        // TAG0 encodes hkArray as a 16-byte synthetic header — m_data (item
        // index, u32 at offset 0) + pad(4) + m_size(4) + m_capacityAndFlags(4)
        // — while hkRelArray uses a bare 4-byte header that is just the item
        // index. Either way the leading u32 is the VARN item index and the real
        // element count + payload offset live on that item (hkArray's trailing
        // 12 bytes are unused: m_size is zero in TAG0), so only the leading u32
        // is read. hknpConvexPolytopeShape's vertices/planes/faces/indices are
        // all hkRelArray; reading them as empty collapses those shapes to an
        // AABB-box fallback.
        //
        // Only scalar/string subtypes are materialized here; inline-struct
        // subtypes (e.g. hkRootLevelContainer::NamedVariant) are handled by the
        // object-pointer pass and read as empty arrays here.
        let item_index = data
            .get(offset..offset + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
            .ok_or_else(|| {
                HavokError::InvalidInput(format!(
                    "TAG0 array header at offset {offset} is outside DATA"
                ))
            })?;

        if item_index == 0 || item_index >= self.items.len() {
            return Ok(HkxValue::Array(Vec::new()));
        }

        let subtype = self.tag_type(tag_type.subtype_id)?;
        let element_type = array_element_layout_type(subtype, &self.type_registry);
        let element_stride = if element_type.kind == 6 {
            // Pointer element: the per-element stride is the in-memory pointer
            // width, which TAG0 records in the pointer type's `size` — 4 on
            // 32-bit packers, 8 on 64-bit packers (FO76 TAG0 2014/2015 is
            // authored 64-bit). The low 4 bytes hold the VARN item index (the
            // high half is zero), as in materialize_pointer_value. A fixed
            // stride of 4 reads half of every 64-bit pointer array as null
            // (hkbLayerGenerator.layers, children, generators, modifiers,
            // hkaAnimationContainer.skeletons).
            if element_type.size == 8 { 8 } else { 4 }
        } else if element_type.kind == 7 {
            // Inline-struct element: each element is the struct's full size.
            if element_type.size == 0 {
                return Ok(HkxValue::Array(Vec::new()));
            }
            element_type.size
        } else {
            match materializable_element_size(element_type, &self.type_registry) {
                Some(n) => n,
                None => {
                    // Unsupported element type — fall through to empty array.
                    return Ok(HkxValue::Array(Vec::new()));
                }
            }
        };
        let payload = &self.items[item_index];
        if payload.kind != ITEM_VARN || payload.count == 0 {
            return Ok(HkxValue::Array(Vec::new()));
        }
        let element_count = payload.count;
        if payload.type_id != subtype.id {
            return Err(HavokError::InvalidInput(format!(
                "TAG0 hkArray VARN item type id {} does not match subtype id {}",
                payload.type_id, subtype.id
            )));
        }
        let byte_len = element_stride.checked_mul(element_count).ok_or_else(|| {
            HavokError::InvalidInput("TAG0 hkArray payload byte length overflow".to_string())
        })?;
        let payload_end = payload.offset.checked_add(byte_len).ok_or_else(|| {
            HavokError::InvalidInput("TAG0 hkArray payload offset overflow".to_string())
        })?;
        data.get(payload.offset..payload_end).ok_or_else(|| {
            HavokError::InvalidInput(format!(
                "TAG0 hkArray VARN payload at offset {} length {byte_len} is outside DATA",
                payload.offset
            ))
        })?;

        let mut values = Vec::with_capacity(element_count);
        for index in 0..element_count {
            let element_offset = payload
                .offset
                .checked_add(element_stride.checked_mul(index).ok_or_else(|| {
                    HavokError::InvalidInput("TAG0 hkArray element offset overflow".to_string())
                })?)
                .ok_or_else(|| {
                    HavokError::InvalidInput("TAG0 hkArray element offset overflow".to_string())
                })?;
            // For kind=3 (string) elements inside an hkArray<hkStringPtr>, the
            // u32/u64 at each element is an item index into the ITEM table —
            // same as for top-level KIND_STRING fields. The non-array helper
            // can't see `self.items`, so dispatch directly here.
            let element = if element_type.kind == 3 {
                materialize_string_via_item(data, element_offset, element_type, &self.items)?
            } else if element_type.kind == 6 {
                materialize_array_pointer_element(
                    data,
                    element_offset,
                    element_type,
                    &self.items,
                    item_to_object_index,
                )?
            } else if element_type.kind == 7 {
                self.materialize_object_value(
                    data,
                    element_offset,
                    element_type,
                    item_to_object_index,
                    visited,
                    path,
                )?
            } else {
                materialize_scalar_or_string_value(
                    data,
                    element_offset,
                    element_type,
                    &self.type_registry,
                )?
            };
            values.push(element);
        }

        Ok(HkxValue::Array(values))
    }

    fn materialize_inline_fixed_array(
        &self,
        data: &[u8],
        offset: usize,
        tag_type: &TagType,
        item_to_object_index: &[Option<usize>],
        visited: &mut Vec<usize>,
        path: &mut Vec<String>,
    ) -> HavokResult<HkxValue> {
        let subtype = self.tag_type(tag_type.subtype_id)?;
        let element_stride = if subtype.kind == 7 {
            subtype.size
        } else if subtype.kind == 6 && self.sdk_year() >= 2018 {
            // Pointer element, same 4/8-byte stride rule as the hkArray path.
            // Starfield's hknpLodShape.variants is hkRefPtr<hknpShape>[8]; a
            // bare `materializable_element_size` lookup returns None for
            // pointers and would drop every LOD variant. Kept version-scoped:
            // on 2015-era files these slots decoded as an empty array, and
            // the FO76 converters are calibrated against that.
            if subtype.size == 8 { 8 } else { 4 }
        } else {
            let Some(element_stride) = materializable_element_size(subtype, &self.type_registry)
            else {
                return Ok(HkxValue::Array(Vec::new()));
            };
            element_stride
        };
        if element_stride == 0 {
            return Ok(HkxValue::Array(Vec::new()));
        }
        if !tag_type.size.is_multiple_of(element_stride) {
            return Err(HavokError::InvalidInput(format!(
                "TAG0 T[N] type size {} is not a multiple of element stride {} for subtype {}",
                tag_type.size, element_stride, subtype.name
            )));
        }
        let element_count = tag_type.size / element_stride;
        if element_count == 0 {
            return Ok(HkxValue::Array(Vec::new()));
        }
        let end = offset.checked_add(tag_type.size).ok_or_else(|| {
            HavokError::InvalidInput("TAG0 T[N] payload offset overflow".to_string())
        })?;
        data.get(offset..end).ok_or_else(|| {
            HavokError::InvalidInput(format!(
                "TAG0 T[N] payload at offset {offset} length {} is outside DATA",
                tag_type.size
            ))
        })?;

        let mut values = Vec::with_capacity(element_count);
        for index in 0..element_count {
            let element_offset = offset
                .checked_add(element_stride.checked_mul(index).ok_or_else(|| {
                    HavokError::InvalidInput("TAG0 T[N] element offset overflow".to_string())
                })?)
                .ok_or_else(|| {
                    HavokError::InvalidInput("TAG0 T[N] element offset overflow".to_string())
                })?;
            let value = if subtype.kind == 7 {
                self.materialize_object_value(
                    data,
                    element_offset,
                    subtype,
                    item_to_object_index,
                    visited,
                    path,
                )?
            } else if subtype.kind == 6 && self.sdk_year() >= 2018 {
                materialize_array_pointer_element(
                    data,
                    element_offset,
                    subtype,
                    &self.items,
                    item_to_object_index,
                )?
            } else {
                materialize_scalar_or_string_value(
                    data,
                    element_offset,
                    subtype,
                    &self.type_registry,
                )?
            };
            values.push(value);
        }

        Ok(HkxValue::Array(values))
    }
}

/// Resolve an array element of pointer subtype: read the u32/u64 item index
/// at `offset`, then map the item index to the materialized object index. A
/// zero index, an out-of-range index, or an unmapped item all produce
/// `Pointer(None)`.
fn materialize_array_pointer_element(
    data: &[u8],
    offset: usize,
    _subtype: &TagType,
    items: &[TagfileItem],
    item_to_object_index: &[Option<usize>],
) -> HavokResult<HkxValue> {
    // Each pointer element is the leading u32 item index at `offset`. The
    // element *stride* (4 on 32-bit, 8 on 64-bit) is applied by the caller in
    // materialize_array_value, so on a 64-bit file this reads the low half of
    // the 8-byte pointer (the high half is zero).
    let raw = data
        .get(offset..)
        .and_then(|tail| tail.get(..4))
        .ok_or_else(|| {
            HavokError::InvalidInput(format!(
                "TAG0 hkArray pointer element at offset {offset} is outside DATA"
            ))
        })?;
    let item_index = u32::from_le_bytes(raw.try_into().expect("4-byte slice")) as usize;

    if item_index == 0 || item_index >= items.len() {
        return Ok(HkxValue::Pointer(None));
    }
    if items[item_index].kind == ITEM_NOTE {
        // NOTE-pointer subtypes aren't materialized yet; treat as null pointer.
        return Ok(HkxValue::Pointer(None));
    }
    let object_index = item_to_object_index.get(item_index).copied().flatten();
    Ok(HkxValue::Pointer(object_index))
}

fn materialize_scalar_or_string_value(
    data: &[u8],
    offset: usize,
    tag_type: &TagType,
    registry: &TagTypeRegistry,
) -> HavokResult<HkxValue> {
    // Note: Tagfile::materialize_field_value handles kind=3 with PTCH/items
    // context. This helper is invoked for non-PTCH-aware contexts (e.g.
    // hkArray scalar elements) and falls back to a raw item-less string read.
    if tag_type.kind == 3 {
        return materialize_string_via_item(data, offset, tag_type, &[]);
    }

    let hkx_type = scalar_hkx_type(tag_type, registry).ok_or_else(|| {
        HavokError::InvalidInput(format!(
            "TAG0 field type {} is not a supported scalar",
            tag_type.name
        ))
    })?;
    hkx_type
        .deserialize(data.get(offset..).unwrap_or(&[]))
        .ok_or_else(|| {
            HavokError::InvalidInput(format!(
                "TAG0 scalar field at offset {offset} is outside DATA"
            ))
        })
}

fn materialize_scalar_or_string_fallback_int(
    data: &[u8],
    offset: usize,
    tag_type: &TagType,
) -> HavokResult<HkxValue> {
    // Python at py_creation_lib/python/creation_lib/hkxpack/tagfile_reader.py:986 falls through to FORMAT-kind
    // dispatch (hkStringPtr without PTCH = signed int32). Its int dispatch
    // table maps size 4 / signed → "<i" (HKXType::Int32). Use that mapping.
    let hkx_type = match tag_type.size {
        1 => HkxType::Int8,
        2 => HkxType::Int16,
        4 => HkxType::Int32,
        8 => HkxType::Int64,
        _ => HkxType::Int32,
    };
    hkx_type
        .deserialize(data.get(offset..).unwrap_or(&[]))
        .ok_or_else(|| {
            HavokError::InvalidInput(format!(
                "TAG0 string-as-int field at offset {offset} is outside DATA"
            ))
        })
}

fn materializable_element_size(tag_type: &TagType, registry: &TagTypeRegistry) -> Option<usize> {
    if tag_type.kind == 3 {
        return matches!(tag_type.size, 4 | 8).then_some(tag_type.size);
    }
    scalar_hkx_type(tag_type, registry).map(HkxType::size)
}

fn array_element_layout_type<'a>(
    tag_type: &'a TagType,
    registry: &'a TagTypeRegistry,
) -> &'a TagType {
    if tag_type.name == "hkFreeListArrayElement"
        && tag_type.kind == 0
        && tag_type.size == 0
        && tag_type.fields.is_empty()
    {
        if let Some(parent) = registry.types.get(tag_type.parent_id) {
            if parent.kind == 7 && parent.size > 0 {
                return parent;
            }
        }
    }
    tag_type
}

fn nested_field_size(tag_type: &TagType, registry: &TagTypeRegistry) -> Option<usize> {
    materializable_element_size(tag_type, registry)
        .or_else(|| (tag_type.size > 0).then_some(tag_type.size))
}

fn member_path(path: &[String]) -> String {
    path.join(".")
}

fn member_path_with(path: &[String], member: &str) -> String {
    if path.is_empty() {
        member.to_string()
    } else {
        format!("{}.{}", member_path(path), member)
    }
}

fn materialize_string_via_item(
    data: &[u8],
    offset: usize,
    tag_type: &TagType,
    items: &[TagfileItem],
) -> HavokResult<HkxValue> {
    let read_size = match tag_type.size {
        4 | 8 => tag_type.size,
        // Match Python: when hkStringPtr/hkRefVariant/const char* has size 0
        // (alias chain), fall back to 4-byte item index (py_creation_lib/python/creation_lib/hkxpack/
        // tagfile_reader.py:1196 default).
        0 => 4,
        other => {
            return Err(HavokError::InvalidInput(format!(
                "TAG0 string type {} has unsupported size {other}",
                tag_type.name
            )));
        }
    };

    let raw = data
        .get(offset..)
        .and_then(|tail| tail.get(..read_size))
        .ok_or_else(|| {
            HavokError::InvalidInput(format!(
                "TAG0 string field at offset {offset} is outside DATA"
            ))
        })?;
    let item_index = match read_size {
        4 => u32::from_le_bytes(raw.try_into().expect("4-byte slice after bounds check")) as usize,
        8 => usize::try_from(u64::from_le_bytes(
            raw.try_into().expect("8-byte slice after bounds check"),
        ))
        .map_err(|_| {
            HavokError::InvalidInput("TAG0 string item index does not fit usize".to_string())
        })?,
        _ => unreachable!(),
    };

    if item_index == 0 || item_index >= items.len() {
        return Ok(HkxValue::String {
            value: String::new(),
            is_null: true,
        });
    }
    let item = &items[item_index];
    if item.kind != ITEM_VARN {
        return Ok(HkxValue::String {
            value: String::new(),
            is_null: true,
        });
    }
    let end = item
        .offset
        .checked_add(item.count)
        .ok_or_else(|| HavokError::InvalidInput("TAG0 VARN string range overflow".to_string()))?;
    let bytes = match data.get(item.offset..end) {
        Some(slice) => slice,
        None => {
            return Ok(HkxValue::String {
                value: String::new(),
                is_null: true,
            });
        }
    };
    // Python: `data[off:off+count].rstrip(b"\x00").decode("ascii", errors="replace")`
    // followed by stripping XML-invalid control bytes — see _read_string_ref at
    // py_creation_lib/python/creation_lib/hkxpack/tagfile_reader.py:1212-1215.
    let trimmed_end = bytes
        .iter()
        .rposition(|byte| *byte != 0)
        .map(|pos| pos + 1)
        .unwrap_or(0);
    Ok(HkxValue::String {
        value: decode_tolerant_string(&bytes[..trimmed_end]),
        is_null: false,
    })
}

fn decode_tolerant_string(raw: &[u8]) -> String {
    String::from_utf8_lossy(raw)
        .chars()
        .filter(|ch| *ch >= ' ' || matches!(*ch, '\n' | '\r' | '\t'))
        .collect()
}

fn scalar_hkx_type(tag_type: &TagType, registry: &TagTypeRegistry) -> Option<HkxType> {
    match tag_type.kind {
        2 => Some(HkxType::Bool),
        4 => match (tag_type.size, tag_type.signed) {
            (1, true) => Some(HkxType::Int8),
            (1, false) => Some(HkxType::Uint8),
            (2, true) => Some(HkxType::Int16),
            (2, false) => Some(HkxType::Uint16),
            (4, true) => Some(HkxType::Int32),
            (4, false) => Some(HkxType::Uint32),
            (8, true) => Some(HkxType::Int64),
            (8, false) => Some(HkxType::Uint64),
            _ => None,
        },
        5 => match tag_type.size {
            2 => Some(HkxType::Uint16),
            4 => Some(HkxType::Real),
            _ => None,
        },
        _ => named_complex_hkx_type(tag_type, registry).or_else(|| {
            known_primitive(&tag_type.name).and_then(|(kind, size, signed)| {
                let mut primitive = tag_type.clone();
                primitive.kind = kind;
                primitive.size = size;
                primitive.signed = signed;
                scalar_hkx_type(&primitive, registry)
            })
        }),
    }
}

fn named_complex_hkx_type(tag_type: &TagType, registry: &TagTypeRegistry) -> Option<HkxType> {
    let (hkx_type, float_parent_name): (HkxType, &str) = match tag_type.name.as_str() {
        "hkVector4" | "hkVector4f" | "Vector4" => (HkxType::Vector4, "hkVector4f"),
        "hkQuaternion" | "hkQuaternionf" => (HkxType::Quaternion, "hkQuaternionf"),
        "hkQsTransform" | "hkQsTransformf" => (HkxType::QsTransform, "hkQsTransformf"),
        "hkMatrix3" | "hkMatrix3f" => (HkxType::Matrix3, "hkMatrix3f"),
        "hkMatrix4" | "hkMatrix4f" => (HkxType::Matrix4, "hkMatrix4f"),
        "hkTransform" | "hkTransformf" => (HkxType::Transform, "hkTransformf"),
        _ => return None,
    };
    // kind 0 is a named alias (e.g. hkVector4 -> hkVector4f). kind 8 is the
    // concrete float-vector layout type itself. hknpConvexPolytopeShape declares
    // its `vertices` hkRelArray element subtype against hkVector4f (kind 8,
    // size 16) directly rather than the kind-0 alias, so the subtype arrives
    // here as kind 8; both resolve to the same HkxType. Field-level kind=8 types
    // are dispatched to the array path before reaching this function
    // (materialize_member), so accepting kind 8 here only affects array-element
    // and T[N] element resolution — exactly where a bare vector layout is meant.
    if tag_type.kind != 0 && tag_type.kind != 8 {
        return None;
    }
    if tag_type.size == hkx_type.size() {
        return Some(hkx_type);
    }
    // FO76 declares complex types with size=0 and a parent chain that may
    // not point directly at the *f variant (Python's _COMPLEX_TYPE_MAP at
    // py_creation_lib/python/creation_lib/hkxpack/tagfile_reader.py:633-646 maps by name alone). Accept
    // the name match unconditionally when size=0 — the size is implicit
    // in the resolved HkxType.
    if tag_type.size == 0 {
        return Some(hkx_type);
    }
    // Size is non-zero but doesn't match — defer to the float-variant
    // parent check as a last resort.
    registry
        .types
        .get(tag_type.parent_id)
        .filter(|parent| {
            parent.name == float_parent_name && parent.kind == 8 && parent.size == hkx_type.size()
        })
        .map(|_| hkx_type)
}

fn resolve_class_name(tag_type: &TagType, registry: &TagTypeRegistry) -> String {
    if !tag_type.name.is_empty() && !is_generic_type_name(&tag_type.name) {
        return tag_type.name.clone();
    }
    let mut current = tag_type;
    let mut visited = vec![current.id];
    while let Some(parent) = registry.types.get(current.parent_id) {
        if visited.contains(&parent.id) {
            break;
        }
        visited.push(parent.id);
        if !parent.name.is_empty() && !is_generic_type_name(&parent.name) {
            return parent.name.clone();
        }
        current = parent;
    }
    tag_type.name.clone()
}

fn is_generic_type_name(name: &str) -> bool {
    matches!(
        name,
        "hkArray"
            | "hkRefPtr"
            | "hkRefVariant"
            | "T*"
            | "T[N]"
            | "hkEnum"
            | "hkFlags"
            | "hkSimpleArray"
            | "hkRelArray"
            | "hkFreeListArray"
            | "const char*"
    )
}

pub fn parse_tagfile(data: &[u8]) -> HavokResult<Tagfile> {
    if data.len() < TAG0_MAGIC_OFFSET + 4
        || &data[TAG0_MAGIC_OFFSET..TAG0_MAGIC_OFFSET + 4] != b"TAG0"
    {
        return Err(HavokError::UnsupportedFormat(
            "not a TAG0 tagfile magic".to_string(),
        ));
    }

    let mut sections = Vec::new();
    parse_hff_sections(data, 0, data.len(), true, &mut sections)?;
    if !sections.iter().any(|section| section.tag == "TAG0") {
        return Err(HavokError::InvalidInput(
            "missing top-level TAG0 section".to_string(),
        ));
    }

    let sdk_version = section_bytes(data, &sections, "SDKV")
        .map(|raw| decode_ascii(raw, "SDKV").map(|sdkv| sdkv.trim_end_matches('\0').to_string()))
        .transpose()?
        .unwrap_or_else(|| "20150100".to_string());
    let contents_version = sdk_contents_version(&sdk_version);

    // TST1/FST1 are the 2018+ SDK spellings of TSTR/FSTR (Starfield ships
    // 20190200); the payload encoding is unchanged.
    let type_strings = parse_string_table(
        section_bytes(data, &sections, "TSTR")
            .or_else(|| section_bytes(data, &sections, "TST1"))
            .unwrap_or(&[]),
    )?;
    let field_strings = parse_string_table(
        section_bytes(data, &sections, "FSTR")
            .or_else(|| section_bytes(data, &sections, "FST1"))
            .unwrap_or(&[]),
    )?;
    let type_registry = build_type_registry(data, &sections, &type_strings, &field_strings)?;
    let items = parse_items(section_bytes(data, &sections, "ITEM").unwrap_or(&[]))?;
    let pointer_offsets = parse_ptch(section_bytes(data, &sections, "PTCH").unwrap_or(&[]))?;

    // SDK sections not consumed here — vanilla Bethesda content does not use them:
    //   ASTR — ATTRIBUTE_STRINGS: runtime attribute annotations on class fields.
    //   THSH — HASHES: per-type CRC hashes for fast type identity checks.
    //   TPRO — PROPERTIES: pluggable property declarations (property bag schema).
    //   TPHS — PROPS_HASHES: CRC hashes corresponding to TPRO entries.
    //   TCRF — COMPENDIUM_REFERENCE: external type-compendium reference (multi-bundle).
    //   TCID — COMPENDIUM_ID: compendium identity token matching TCRF.
    //   TSEQ — SEQUENCE_NUMBER_V0: monotonic sequence number for multi-bundle ordering.
    //   TSHA — ROLLING_HASH: rolling hash for multi-bundle stream integrity.

    Ok(Tagfile {
        sdk_version,
        contents_version,
        sections,
        type_strings,
        field_strings,
        type_registry,
        items,
        pointer_offsets,
        source_bytes: data.to_vec(),
    })
}

pub fn read_vle(data: &[u8], pos: usize) -> HavokResult<(u64, usize)> {
    ensure_len(data, pos + 1, "VLE prefix")?;
    let p0 = data[pos];
    if p0 & 0x80 == 0 {
        return Ok((p0 as u64, pos + 1));
    }

    let prefix = p0 >> 3;
    let low3 = p0 & 0x07;
    let (width, bits) = if prefix <= 0x17 {
        (2, 14)
    } else if prefix <= 0x1B {
        (3, 21)
    } else if prefix == 0x1C {
        (4, 27)
    } else if prefix == 0x1D {
        (5, 35)
    } else if prefix == 0x1E {
        (8, 59)
    } else if low3 == 0 {
        (6, 40)
    } else if low3 == 1 {
        ensure_len(data, pos + 9, "9-byte VLE")?;
        let mut value = 0u64;
        for byte in &data[pos + 1..pos + 9] {
            value = (value << 8) | u64::from(*byte);
        }
        return Ok((value, pos + 9));
    } else {
        return Err(HavokError::InvalidInput(format!(
            "invalid VLE prefix at offset {pos}: {p0:#04X}"
        )));
    };

    ensure_len(data, pos + width, "VLE value")?;
    let mut raw = 0u64;
    for byte in &data[pos..pos + width] {
        raw = (raw << 8) | u64::from(*byte);
    }
    let mask = (1u64 << bits) - 1;
    Ok((raw & mask, pos + width))
}

pub fn read_vle_signed(data: &[u8], pos: usize) -> HavokResult<(i64, usize)> {
    let (value, pos) = read_vle(data, pos)?;
    if value & 1 == 0 {
        Ok(((value >> 1) as i64, pos))
    } else {
        Ok((-((value >> 1) as i64) - 1, pos))
    }
}

fn parse_hff_sections(
    data: &[u8],
    mut offset: usize,
    end: usize,
    keep_branch: bool,
    sections: &mut Vec<TagfileSection>,
) -> HavokResult<()> {
    while offset + HFF_HEADER_SIZE <= end {
        let packed = read_u32_be(data, offset, "HFF section header")?;
        let scope = (packed >> 30) as u8;
        let size = (packed & 0x3FFF_FFFF) as usize;
        if size < HFF_HEADER_SIZE || offset + size > end {
            return Err(HavokError::InvalidInput(format!(
                "invalid HFF section at offset {offset}"
            )));
        }

        let tag = decode_ascii(&data[offset + 4..offset + 8], "HFF tag")?;
        let content_offset = offset + HFF_HEADER_SIZE;
        let content_size = size - HFF_HEADER_SIZE;
        if scope == 0 {
            if keep_branch || tag == "TAG0" {
                sections.push(TagfileSection {
                    tag: tag.clone(),
                    offset,
                    size,
                    content_offset,
                    content_size,
                    scope,
                });
            }
            parse_hff_sections(data, content_offset, offset + size, false, sections)?;
        } else {
            sections.push(TagfileSection {
                tag,
                offset,
                size,
                content_offset,
                content_size,
                scope,
            });
        }

        offset += size;
    }
    if offset < end && data[offset..end].iter().any(|byte| *byte != 0) {
        return Err(HavokError::InvalidInput(format!(
            "trailing HFF bytes at offset {offset}"
        )));
    }
    Ok(())
}

fn build_type_registry(
    data: &[u8],
    sections: &[TagfileSection],
    type_strings: &[String],
    field_strings: &[String],
) -> HavokResult<TagTypeRegistry> {
    let mut registry = TagTypeRegistry { types: Vec::new() };
    if let Some(raw) =
        section_bytes(data, sections, "TNA1").or_else(|| section_bytes(data, sections, "TNAM"))
    {
        parse_type_identities(raw, type_strings, &mut registry)?;
    }
    if let Some(raw) =
        section_bytes(data, sections, "TBDY").or_else(|| section_bytes(data, sections, "TBOD"))
    {
        parse_type_bodies(raw, field_strings, &mut registry)?;
    }
    resolve_primitive_types(&mut registry);
    Ok(registry)
}

fn parse_type_identities(
    data: &[u8],
    type_strings: &[String],
    registry: &mut TagTypeRegistry,
) -> HavokResult<()> {
    if data.is_empty() {
        return Ok(());
    }

    let (next_num_types, mut pos) = read_vle(data, 0)?;
    let next_num_types = checked_type_index(next_num_types, "type count")?;
    let prev_num_types = registry.types.len().max(1);
    ensure_type_slots(registry, next_num_types);

    for type_id in prev_num_types..next_num_types {
        if pos >= data.len() {
            break;
        }
        let (name_sid, next_pos) = read_vle(data, pos)?;
        pos = next_pos;
        if let Some(name) = type_strings.get(name_sid as usize) {
            registry.types[type_id].name = name.clone();
        }

        if pos >= data.len() {
            break;
        }
        let (num_params, next_pos) = read_vle(data, pos)?;
        pos = next_pos;
        for _ in 0..num_params {
            let (_, next_pos) = read_vle(data, pos)?;
            pos = next_pos;
            let (_, next_pos) = read_vle(data, pos)?;
            pos = next_pos;
        }
    }
    Ok(())
}

fn parse_type_bodies(
    data: &[u8],
    field_strings: &[String],
    registry: &mut TagTypeRegistry,
) -> HavokResult<()> {
    let mut pos = 0;
    while pos < data.len() {
        if data[pos] == 0 {
            break;
        }
        let (type_id, next_pos) = read_vle(data, pos)?;
        pos = next_pos;
        let (parent_id, next_pos) = read_vle(data, pos)?;
        pos = next_pos;
        let (optbits, next_pos) = read_vle(data, pos)?;
        pos = next_pos;

        let type_id = checked_type_index(type_id, "type id")?;
        let slot_count = type_id.checked_add(1).ok_or_else(|| {
            HavokError::InvalidInput("TAG0 type id overflows slot count".to_string())
        })?;
        ensure_type_slots(registry, slot_count);
        registry.types[type_id].parent_id = checked_type_index(parent_id, "parent type id")?;

        if optbits & 0x01 != 0 {
            let (format_value, next_pos) = read_vle(data, pos)?;
            pos = next_pos;
            let tag_type = &mut registry.types[type_id];
            tag_type.format_value = format_value;
            tag_type.kind = (format_value & 0x1F) as u8;
            if tag_type.kind == 4 {
                let bit_count = (format_value >> 10) & 0xFF;
                // bit_count==0 means no format bits were encoded; fall back to
                // 1 byte (the optbits 0x08 branch will override with the correct
                // size/align if present).
                tag_type.size = if bit_count == 0 {
                    1
                } else {
                    (bit_count / 8).max(1) as usize
                };
                tag_type.signed = format_value & (1 << 9) != 0;
            } else if tag_type.kind == 5 {
                let exp_bits = (format_value >> 11) & 0x1F;
                let sig_bits = (format_value >> 16) & 0xFF;
                let sign_bits = u64::from(format_value & (1 << 9) != 0);
                let total_bits = sign_bits + exp_bits + sig_bits;
                tag_type.size = if total_bits == 0 {
                    4
                } else {
                    (total_bits / 8).max(1) as usize
                };
            } else if tag_type.kind == 2 {
                let bit_count = (format_value >> 10) & 0xFF;
                tag_type.size = if bit_count == 0 {
                    1
                } else {
                    (bit_count / 8).max(1) as usize
                };
            }
        }
        if optbits & 0x02 != 0 {
            let (subtype_id, next_pos) = read_vle(data, pos)?;
            pos = next_pos;
            registry.types[type_id].subtype_id = checked_type_index(subtype_id, "subtype id")?;
        }
        if optbits & 0x04 != 0 {
            let (version, next_pos) = read_vle_signed(data, pos)?;
            pos = next_pos;
            registry.types[type_id].version = version;
        }
        if optbits & 0x08 != 0 {
            let (size, next_pos) = read_vle(data, pos)?;
            pos = next_pos;
            let (align, next_pos) = read_vle(data, pos)?;
            pos = next_pos;
            registry.types[type_id].size = size as usize;
            registry.types[type_id].align = align as usize;
        }
        if optbits & 0x10 != 0 {
            let (_, next_pos) = read_vle(data, pos)?;
            pos = next_pos;
        }
        if optbits & 0x20 != 0 {
            let (encoded, next_pos) = read_vle(data, pos)?;
            pos = next_pos;
            let num_fields = encoded & 0xFFFF;
            for _ in 0..num_fields {
                let (sid, next_pos) = read_vle(data, pos)?;
                pos = next_pos;
                let (flags, next_pos) = read_vle(data, pos)?;
                pos = next_pos;
                if flags & 0x80 != 0 {
                    // 2018+ SDK: a serialization descriptor precedes the byte
                    // offset. FO76's 2015.1.0 writer never sets this bit.
                    let (_, next_pos) = read_vle(data, pos)?;
                    pos = next_pos;
                }
                let (offset, next_pos) = read_vle(data, pos)?;
                pos = next_pos;
                let (field_type_id, next_pos) = read_vle(data, pos)?;
                pos = next_pos;
                let name = field_strings
                    .get(sid as usize)
                    .cloned()
                    .unwrap_or_else(|| format!("field_{sid}"));
                registry.types[type_id].fields.push(TagField {
                    name,
                    type_id: checked_type_index(field_type_id, "field type id")?,
                    offset: offset as usize,
                    flags,
                });
            }
        }
        if optbits & 0x40 != 0 {
            let (num_interfaces, next_pos) = read_vle(data, pos)?;
            pos = next_pos;
            for _ in 0..num_interfaces {
                let (_, next_pos) = read_vle(data, pos)?;
                pos = next_pos;
                let (_, next_pos) = read_vle(data, pos)?;
                pos = next_pos;
            }
        }
        if optbits & 0x80 != 0 {
            let (_, next_pos) = read_vle(data, pos)?;
            pos = next_pos;
        }
    }
    Ok(())
}

fn parse_items(data: &[u8]) -> HavokResult<Vec<TagfileItem>> {
    if !data.len().is_multiple_of(12) {
        return Err(HavokError::InvalidInput(format!(
            "ITEM section length {} is not divisible by 12",
            data.len()
        )));
    }
    let mut items = Vec::new();
    for chunk in data.chunks_exact(12) {
        let packed = read_u32_le(chunk, 0, "ITEM packed")?;
        items.push(TagfileItem {
            kind: ((packed >> 28) & 0xF) as u8,
            flags: ((packed >> 24) & 0xF) as u8,
            type_id: (packed & 0x00FF_FFFF) as usize,
            offset: read_u32_le(chunk, 4, "ITEM offset")? as usize,
            count: read_u32_le(chunk, 8, "ITEM count")? as usize,
        });
    }
    Ok(items)
}

fn parse_ptch(data: &[u8]) -> HavokResult<Vec<usize>> {
    if !data.len().is_multiple_of(4) {
        return Err(HavokError::InvalidInput(format!(
            "PTCH section length {} is not divisible by 4",
            data.len()
        )));
    }
    let values: Vec<u32> = data
        .chunks_exact(4)
        .map(|chunk| read_u32_le(chunk, 0, "PTCH value"))
        .collect::<HavokResult<_>>()?;
    let mut offsets = Vec::new();
    let mut cur = 0;
    while cur + 2 <= values.len() {
        let src_num = values[cur + 1] as usize;
        cur += 2;
        if cur + src_num > values.len() {
            return Err(HavokError::InvalidInput(
                "incomplete PTCH group".to_string(),
            ));
        }
        offsets.extend(
            values[cur..cur + src_num]
                .iter()
                .map(|value| *value as usize),
        );
        cur += src_num;
    }
    Ok(offsets)
}

fn parse_string_table(raw: &[u8]) -> HavokResult<Vec<String>> {
    // 2018+ SDK writers pad the section tail to alignment with 0xFF.
    let end = raw
        .iter()
        .rposition(|byte| *byte != 0xFF)
        .map_or(0, |index| index + 1);
    raw[..end]
        .split(|byte| *byte == 0)
        .filter(|part| !part.is_empty())
        .map(|part| decode_ascii(part, "string table"))
        .collect()
}

fn section_bytes<'a>(data: &'a [u8], sections: &[TagfileSection], tag: &str) -> Option<&'a [u8]> {
    sections
        .iter()
        .find(|section| section.tag == tag)
        .map(|section| &data[section.content_offset..section.content_offset + section.content_size])
}

fn ensure_type_slots(registry: &mut TagTypeRegistry, len: usize) {
    while registry.types.len() < len {
        registry.types.push(TagType {
            id: registry.types.len(),
            name: String::new(),
            parent_id: 0,
            kind: 0,
            subtype_id: 0,
            size: 0,
            align: 0,
            version: 0,
            format_value: 0,
            signed: false,
            fields: Vec::new(),
        });
    }
}

fn checked_type_index(value: u64, context: &str) -> HavokResult<usize> {
    let value = usize::try_from(value)
        .map_err(|_| HavokError::InvalidInput(format!("TAG0 {context} does not fit in usize")))?;
    if value > MAX_TAG_TYPES {
        return Err(HavokError::InvalidInput(format!(
            "TAG0 {context} {value} exceeds maximum {MAX_TAG_TYPES}"
        )));
    }
    Ok(value)
}

fn resolve_primitive_types(registry: &mut TagTypeRegistry) {
    for index in 0..registry.types.len() {
        if registry.types[index].kind != 0 || registry.types[index].size != 0 {
            continue;
        }
        if let Some((kind, size, signed)) = known_primitive(&registry.types[index].name) {
            registry.types[index].kind = kind;
            registry.types[index].size = size;
            registry.types[index].signed = signed;
            continue;
        }

        let mut current_id = registry.types[index].parent_id;
        let mut visited = vec![index];
        for _ in 0..4 {
            if visited.contains(&current_id) {
                break;
            }
            let Some(parent) = registry.types.get(current_id).cloned() else {
                break;
            };
            visited.push(current_id);
            if let Some((kind, size, signed)) = known_primitive(&parent.name) {
                registry.types[index].kind = kind;
                registry.types[index].size = size;
                registry.types[index].signed = signed;
                break;
            }
            if !matches!(parent.kind, 0 | 7 | 8 | 6) && parent.size > 0 {
                registry.types[index].kind = parent.kind;
                registry.types[index].size = parent.size;
                registry.types[index].signed = parent.signed;
                break;
            }
            current_id = parent.parent_id;
        }
    }
}

fn known_primitive(name: &str) -> Option<(u8, usize, bool)> {
    match name {
        "unsigned short" | "hkUint16" => Some((4, 2, false)),
        "short" | "hkInt16" => Some((4, 2, true)),
        "unsigned int" | "hkUint32" => Some((4, 4, false)),
        "int" | "hkInt32" => Some((4, 4, true)),
        "unsigned char" | "char" | "hkUint8" => Some((4, 1, false)),
        "signed char" | "hkInt8" => Some((4, 1, true)),
        "unsigned long long" | "hkUint64" => Some((4, 8, false)),
        "long long" | "hkInt64" => Some((4, 8, true)),
        "float" | "hkReal" => Some((5, 4, false)),
        "double" => Some((5, 8, false)),
        "hkHalf" => Some((5, 2, false)),
        "hkBool" => Some((2, 1, false)),
        _ => None,
    }
}

fn sdk_contents_version(sdk_version: &str) -> String {
    match sdk_version {
        "20150100" => "hk_2015.1.0-r1".to_string(),
        "20140100" => "hk_2014.1.0-r1".to_string(),
        "20140200" => "hk_2014.2.0-r1".to_string(),
        "20100200" => "hk_2010.2.0-r1".to_string(),
        other if other.len() >= 5 => format!("hk_{}.{}.0-r1", &other[..4], &other[4..5]),
        other => format!("hk_{other}"),
    }
}

fn read_u32_be(data: &[u8], offset: usize, context: &str) -> HavokResult<u32> {
    ensure_len(data, offset + 4, context)?;
    Ok(u32::from_be_bytes(
        data[offset..offset + 4].try_into().unwrap(),
    ))
}

fn read_u32_le(data: &[u8], offset: usize, context: &str) -> HavokResult<u32> {
    ensure_len(data, offset + 4, context)?;
    Ok(u32::from_le_bytes(
        data[offset..offset + 4].try_into().unwrap(),
    ))
}

fn decode_ascii(data: &[u8], context: &str) -> HavokResult<String> {
    std::str::from_utf8(data)
        .map(|value| value.to_string())
        .map_err(|_| HavokError::InvalidInput(format!("{context} is not valid UTF-8")))
}

fn ensure_len(data: &[u8], needed: usize, context: &str) -> HavokResult<()> {
    if data.len() < needed {
        return Err(HavokError::InvalidInput(format!(
            "{context} needs {needed} bytes, got {}",
            data.len()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod pointer_array_stride_tests {
    use super::{TagField, TagType, TagTypeRegistry, Tagfile, TagfileItem, TagfileSection};
    use crate::hkx::types::HkxValue;

    // Build a minimal TAG0 model with an hkArray<TargetClass*> field holding
    // four pointers, each authored at `ptr_size` bytes with the VARN item index
    // in the low 4 bytes (high half zero on 64-bit). Returns the resolved
    // pointer list materialized from that array.
    fn materialize_pointer_array(ptr_size: usize) -> Vec<Option<usize>> {
        let parent_offset = 0usize;
        let payload_offset = 16usize; // after the 16-byte hkArray synthetic header
        let target_item_indices = [2u32, 3, 4, 5];
        let targets_offset = payload_offset + target_item_indices.len() * ptr_size;
        let mut data = vec![0u8; targets_offset + target_item_indices.len() * 4];

        // Parent body: hkArray header whose leading u32 is the VARN payload item.
        let varn_item_index: u32 = 6;
        data[parent_offset..parent_offset + 4].copy_from_slice(&varn_item_index.to_le_bytes());
        // VARN payload: `ptr_size`-byte pointers; low 4 bytes = target item index,
        // high bytes left zero (the interleaved-null pattern a 4-byte stride
        // would mis-read on a 64-bit file).
        for (i, item_idx) in target_item_indices.iter().enumerate() {
            let o = payload_offset + i * ptr_size;
            data[o..o + 4].copy_from_slice(&item_idx.to_le_bytes());
        }

        let registry = TagTypeRegistry {
            types: vec![
                TagType {
                    id: 0,
                    name: "hkRootLevelContainer".into(),
                    parent_id: 0,
                    kind: 0,
                    subtype_id: 0,
                    size: 0,
                    align: 0,
                    version: 0,
                    format_value: 0,
                    signed: false,
                    fields: vec![],
                },
                // id 1: pointee class (one int field so it materializes as an object)
                TagType {
                    id: 1,
                    name: "TargetClass".into(),
                    parent_id: 0,
                    kind: 7,
                    subtype_id: 0,
                    size: 4,
                    align: 4,
                    version: 1,
                    format_value: 0,
                    signed: false,
                    fields: vec![TagField {
                        name: "scratch".into(),
                        type_id: 4,
                        offset: 0,
                        flags: 0,
                    }],
                },
                // id 2: TargetClass* (kind 6). `size` is the pointer width — this
                // is the 32-vs-64-bit signal the stride must follow.
                TagType {
                    id: 2,
                    name: "TargetClass*".into(),
                    parent_id: 0,
                    kind: 6,
                    subtype_id: 1,
                    size: ptr_size,
                    align: ptr_size,
                    version: 0,
                    format_value: 0,
                    signed: false,
                    fields: vec![],
                },
                // id 3: hkArray<TargetClass*> (kind 8, 16-byte synthetic header)
                TagType {
                    id: 3,
                    name: "hkArray".into(),
                    parent_id: 0,
                    kind: 8,
                    subtype_id: 2,
                    size: 16,
                    align: 8,
                    version: 0,
                    format_value: 0,
                    signed: false,
                    fields: vec![],
                },
                // id 4: int32 used as TargetClass.scratch
                TagType {
                    id: 4,
                    name: "int32".into(),
                    parent_id: 0,
                    kind: 4,
                    subtype_id: 0,
                    size: 4,
                    align: 4,
                    version: 0,
                    format_value: 0,
                    signed: true,
                    fields: vec![],
                },
                // id 5: parent object with the array field at offset 0
                TagType {
                    id: 5,
                    name: "ParentClass".into(),
                    parent_id: 0,
                    kind: 7,
                    subtype_id: 0,
                    size: 16,
                    align: 8,
                    version: 1,
                    format_value: 0,
                    signed: false,
                    fields: vec![TagField {
                        name: "layers".into(),
                        type_id: 3,
                        offset: 0,
                        flags: 0,
                    }],
                },
            ],
        };

        let mut items = vec![
            TagfileItem {
                kind: 0,
                flags: 0,
                type_id: 0,
                offset: 0,
                count: 0,
            },
            // index 1: VAR0 parent (object 0)
            TagfileItem {
                kind: 1,
                flags: 0,
                type_id: 5,
                offset: parent_offset,
                count: 1,
            },
        ];
        // indices 2..=5: VAR0 targets (objects 1..=4)
        for i in 0..target_item_indices.len() {
            items.push(TagfileItem {
                kind: 1,
                flags: 0,
                type_id: 1,
                offset: targets_offset + i * 4,
                count: 1,
            });
        }
        // index 6: VARN payload; type id must equal the array subtype (the pointer)
        items.push(TagfileItem {
            kind: 2,
            flags: 0,
            type_id: 2,
            offset: payload_offset,
            count: target_item_indices.len(),
        });

        let tagfile = Tagfile::from_synthetic_parts(
            "20150100",
            "hk_2015.1.0-r1",
            vec![TagfileSection {
                tag: "DATA".into(),
                offset: 0,
                size: data.len(),
                content_offset: 0,
                content_size: data.len(),
                scope: 1,
            }],
            registry,
            items,
            data,
        );

        let hkx = tagfile
            .materialize_hkx()
            .expect("materialize pointer-array fixture");
        let parent = hkx
            .objects()
            .iter()
            .find(|o| o.class_name == "ParentClass")
            .expect("parent object");
        let layers = parent
            .members
            .iter()
            .find(|m| m.name == "layers")
            .expect("layers member");
        match &layers.value {
            HkxValue::Array(values) => values
                .iter()
                .map(|v| match v {
                    HkxValue::Pointer(p) => *p,
                    other => panic!("expected Pointer element, got {other:?}"),
                })
                .collect(),
            other => panic!("expected Array for layers, got {other:?}"),
        }
    }

    #[test]
    fn hkarray_pointer_elements_use_64bit_stride() {
        // 64-bit (FO76) hkArray<T*>: 8-byte pointer elements, item index in the
        // low half. A hardcoded 4-byte stride read only the first count/2 real
        // pointers and interleaved their zero high-halves as nulls
        // ([Some, None, Some, None]). The stride must follow the pointer type's
        // size so every element resolves.
        let resolved = materialize_pointer_array(8);
        assert_eq!(
            resolved,
            vec![Some(1), Some(2), Some(3), Some(4)],
            "every 64-bit pointer element must resolve; none spuriously null"
        );
    }

    #[test]
    fn hkarray_pointer_elements_32bit_stride_unchanged() {
        // 32-bit packers store 4-byte pointer elements; the stride must remain 4
        // so genuine 32-bit arrays (FO4 packfiles, older content) are
        // unaffected by the 64-bit fix.
        let resolved = materialize_pointer_array(4);
        assert_eq!(
            resolved,
            vec![Some(1), Some(2), Some(3), Some(4)],
            "32-bit pointer elements must continue to resolve at a 4-byte stride"
        );
    }

    #[test]
    fn fo76_fixture_pointer_arrays_are_64bit_and_have_no_interleaved_nulls() {
        use super::parse_tagfile;
        use std::path::PathBuf;

        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("python/creation_lib/hkxpack/tests/fixtures/fo76_snallygastercharacter.hkx");
        let data = std::fs::read(&path).expect("read FO76 snallygaster fixture");
        let tagfile = parse_tagfile(&data).expect("parse FO76 fixture");

        // The detection signal: this 64-bit FO76 file declares pointer types
        // (kind 6) with size 8. Confirm at least one hkArray<pointer> exists
        // whose pointer subtype is 8 bytes wide, so the stride gate fires.
        let has_64bit_pointer_array = tagfile.type_registry.types.iter().any(|arr| {
            arr.kind == 8
                && tagfile
                    .type_registry
                    .types
                    .get(arr.subtype_id)
                    .map(|sub| sub.kind == 6 && sub.size == 8)
                    .unwrap_or(false)
        });
        assert!(
            has_64bit_pointer_array,
            "FO76 fixture should declare at least one 64-bit (8-byte) pointer array"
        );

        // Materialization must succeed end-to-end on the real file.
        let hkx = tagfile.materialize_hkx().expect("materialize FO76 fixture");

        // Every materialized pointer array must read its full element count with
        // no interleaved trailing nulls — the half-null signature of the bug was
        // a real pointer in even slots and a null in every odd slot.
        let mut checked_arrays = 0usize;
        for object in hkx.objects() {
            for member in &object.members {
                if let HkxValue::Array(values) = &member.value {
                    let all_pointers = !values.is_empty()
                        && values.iter().all(|v| matches!(v, HkxValue::Pointer(_)));
                    if !all_pointers {
                        continue;
                    }
                    checked_arrays += 1;
                    // A correctly-strided 64-bit pointer array does not produce
                    // the strict alternating real/null pattern the 4-byte stride
                    // did. Guard against that exact signature for arrays of >= 2.
                    if values.len() >= 2 {
                        let alternating_real_null = values.iter().enumerate().all(|(i, v)| {
                            if i % 2 == 0 {
                                matches!(v, HkxValue::Pointer(Some(_)))
                            } else {
                                matches!(v, HkxValue::Pointer(None))
                            }
                        });
                        assert!(
                            !alternating_real_null,
                            "object {} member {} shows the interleaved real/null \
                             signature of the 4-byte-stride bug: {:?}",
                            object.class_name, member.name, values
                        );
                    }
                }
            }
        }
        assert!(
            checked_arrays > 0,
            "fixture should materialize at least one pointer array to validate"
        );
    }
}
