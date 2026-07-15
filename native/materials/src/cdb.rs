use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::bsrefl::{Chunk, Stream, chunk_type};
use crate::error::Result;
use crate::string_table::STRING_TABLE;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub struct ResourceId {
    pub dir: u32,
    pub file: u32,
    pub ext: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CdbPayload {
    pub class_defs: Vec<ClassDefPayload>,
    pub objects: Vec<MaterialObjectPayload>,
    pub component_info: Vec<ComponentInfoPayload>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ClassDefPayload {
    pub class_name: String,
    pub class_name_index: u32,
    pub class_version: u32,
    pub class_flags: u16,
    pub field_count: u16,
    pub fields: Vec<FieldDefPayload>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FieldDefPayload {
    pub name_index: u32,
    pub type_index: u32,
    pub data_offset: u16,
    pub data_size: u16,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ComponentBlobPayload {
    pub class_name: String,
    pub is_diff: bool,
    pub key: u32,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MaterialObjectPayload {
    pub persistent_id: ResourceId,
    pub db_id: u32,
    pub base_object_db_id: u32,
    pub has_data: bool,
    pub parent_db_id: Option<u32>,
    pub components: Vec<ComponentBlobPayload>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ComponentInfoPayload {
    pub db_id: u32,
    pub key: u32,
}

pub fn parse_cdb(data: &[u8]) -> Result<CdbPayload> {
    let mut stream = Stream::new(data)?;
    let mut payload = CdbPayload {
        class_defs: Vec::new(),
        objects: Vec::new(),
        component_info: Vec::new(),
    };
    let mut object_info_size = 21usize;
    let mut object_index_by_db_id = HashMap::<u32, usize>::new();
    let mut object_index_by_persistent_id = HashMap::<ResourceId, usize>::new();
    let mut component_cursor = 0usize;

    while let Some((kind, mut chunk)) = stream.read_chunk()? {
        let mut class_name_index = 0u32;
        let mut class_name = string_by_index(class_name_index).to_owned();
        if kind != chunk_type::TYPE && chunk.size() >= 4 {
            if let Some(strt_offs) = chunk.read_u32() {
                class_name_index = stream.find_string_by_offset(strt_offs);
                class_name = string_by_index(class_name_index).to_owned();
            }
        }

        match kind {
            chunk_type::OBJT | chunk_type::DIFF => handle_obj_chunk(
                &mut chunk,
                kind == chunk_type::DIFF,
                class_name_index,
                class_name,
                &mut payload,
                &object_index_by_db_id,
                &mut component_cursor,
            ),
            chunk_type::TYPE => {
                handle_type_chunk(&mut stream, &mut chunk, &mut payload, &mut object_info_size)?
            }
            chunk_type::LIST => {
                if let Some(n) = chunk.read_u32() {
                    if class_name == "BSComponentDB2::DBFileIndex::ObjectInfo" {
                        handle_object_info_list(
                            &mut chunk,
                            n,
                            object_info_size,
                            &mut payload,
                            &mut object_index_by_db_id,
                            &mut object_index_by_persistent_id,
                        );
                    } else if class_name == "BSComponentDB2::DBFileIndex::ComponentInfo" {
                        handle_component_info_list(
                            &mut chunk,
                            n,
                            &mut payload.component_info,
                            &mut component_cursor,
                        );
                    } else if class_name == "BSComponentDB2::DBFileIndex::EdgeInfo" {
                        handle_edge_info_list(&mut chunk, n, &mut payload, &object_index_by_db_id);
                    }
                }
            }
            _ => {}
        }
    }

    Ok(payload)
}

fn handle_type_chunk(
    stream: &mut Stream<'_>,
    type_chunk: &mut Chunk<'_>,
    payload: &mut CdbPayload,
    object_info_size: &mut usize,
) -> Result<()> {
    let Some(class_count) = type_chunk.read_u32() else {
        return Ok(());
    };

    for _ in 0..class_count {
        let Some((kind, mut clas_chunk)) = stream.read_chunk()? else {
            return Ok(());
        };
        if kind != chunk_type::CLAS || clas_chunk.size() < 4 {
            return Ok(());
        }
        let Some(class_name_offs) = clas_chunk.read_u32() else {
            continue;
        };
        let class_name_index = stream.find_string_by_offset(class_name_offs);
        if class_name_index < 18 {
            continue;
        }
        if class_name_index == 18 {
            continue;
        }
        let class_name = string_by_index(class_name_index).to_owned();
        let Some(class_version) = clas_chunk.read_u32() else {
            continue;
        };
        let Some(class_flags) = clas_chunk.read_u16() else {
            continue;
        };
        let Some(field_count) = clas_chunk.read_u16() else {
            continue;
        };

        if class_name == "BSComponentDB2::DBFileIndex::ObjectInfo" && field_count > 4 {
            *object_info_size = 33;
        }

        if let Some(existing) = payload
            .class_defs
            .iter()
            .find(|class_def| class_def.class_name == class_name)
        {
            if existing.field_count != field_count {
                return Ok(());
            }
            for _ in 0..field_count {
                skip_field_def(&mut clas_chunk);
            }
            continue;
        }

        let mut class_def = ClassDefPayload {
            class_name,
            class_name_index,
            class_version,
            class_flags,
            field_count,
            fields: Vec::new(),
        };
        for _ in 0..field_count {
            let Some(name_offs) = clas_chunk.read_u32() else {
                break;
            };
            let Some(type_offs) = clas_chunk.read_u32() else {
                break;
            };
            let Some(data_offset) = clas_chunk.read_u16() else {
                break;
            };
            let Some(data_size) = clas_chunk.read_u16() else {
                break;
            };
            class_def.fields.push(FieldDefPayload {
                name_index: stream.find_string_by_offset(name_offs),
                type_index: stream.find_string_by_offset(type_offs),
                data_offset,
                data_size,
            });
        }
        payload.class_defs.push(class_def);
    }

    Ok(())
}

fn handle_object_info_list(
    chunk: &mut Chunk<'_>,
    n: u32,
    rec_size: usize,
    payload: &mut CdbPayload,
    object_index_by_db_id: &mut HashMap<u32, usize>,
    object_index_by_persistent_id: &mut HashMap<ResourceId, usize>,
) {
    let Some(record_bytes_len) = (n as usize).checked_mul(rec_size) else {
        return;
    };
    let Some(records) = read_byte_block(chunk, record_bytes_len) else {
        return;
    };

    for record in records.chunks_exact(rec_size) {
        let file = read_u32_from(record, 0);
        let ext = read_u32_from(record, 4);
        let dir = read_u32_from(record, 8);
        let db_id = read_u32_from(record, 12);
        let base_object_db_id = read_u32_from(record, 16);
        let has_data = record[rec_size - 1] != 0;
        let persistent_id = ResourceId { dir, file, ext };
        if db_id == 0 {
            continue;
        }
        if object_index_by_persistent_id.contains_key(&persistent_id) && has_data {
            continue;
        }
        if object_index_by_db_id.contains_key(&db_id) {
            continue;
        }

        let object_index = payload.objects.len();
        payload.objects.push(MaterialObjectPayload {
            persistent_id,
            db_id,
            base_object_db_id,
            has_data,
            parent_db_id: None,
            components: Vec::new(),
        });
        object_index_by_db_id.insert(db_id, object_index);
        if persistent_id.dir != 0 || persistent_id.file != 0 || persistent_id.ext != 0 {
            object_index_by_persistent_id.insert(persistent_id, object_index);
        }
    }
}

fn handle_component_info_list(
    chunk: &mut Chunk<'_>,
    n: u32,
    component_info: &mut Vec<ComponentInfoPayload>,
    component_cursor: &mut usize,
) {
    component_info.clear();
    *component_cursor = 0;
    for _ in 0..n {
        let Some(db_id) = chunk.read_u32() else {
            return;
        };
        let Some(key) = chunk.read_u32() else {
            return;
        };
        component_info.push(ComponentInfoPayload { db_id, key });
    }
}

fn handle_edge_info_list(
    chunk: &mut Chunk<'_>,
    n: u32,
    payload: &mut CdbPayload,
    object_index_by_db_id: &HashMap<u32, usize>,
) {
    let rec_size = 12usize;
    let Some(record_bytes_len) = (n as usize).checked_mul(rec_size) else {
        return;
    };
    let Some(records) = read_byte_block(chunk, record_bytes_len) else {
        return;
    };

    for record in records.chunks_exact(rec_size) {
        let source_id = read_u32_from(record, 0);
        let target_id = read_u32_from(record, 4);
        let Some(source_index) = object_index_by_db_id.get(&source_id) else {
            continue;
        };
        if object_index_by_db_id.contains_key(&target_id) {
            payload.objects[*source_index].parent_db_id = Some(target_id);
        }
    }
}

fn handle_obj_chunk(
    chunk: &mut Chunk<'_>,
    is_diff: bool,
    class_name_index: u32,
    class_name: String,
    payload: &mut CdbPayload,
    object_index_by_db_id: &HashMap<u32, usize>,
    component_cursor: &mut usize,
) {
    if *component_cursor >= payload.component_info.len() {
        return;
    }
    let component_info = &payload.component_info[*component_cursor];
    *component_cursor += 1;

    let key = (component_info.key & 0xFFFF) | (class_name_index << 16);
    let Some(object_index) = object_index_by_db_id.get(&component_info.db_id) else {
        return;
    };
    if class_name_index <= 18 {
        return;
    }

    payload.objects[*object_index]
        .components
        .push(ComponentBlobPayload {
            class_name,
            is_diff,
            key,
            body: drain_remaining_bytes(chunk),
        });
}

fn string_by_index(index: u32) -> &'static str {
    STRING_TABLE
        .get(index as usize)
        .copied()
        .unwrap_or(STRING_TABLE[18])
}

fn skip_field_def(chunk: &mut Chunk<'_>) {
    let _ = chunk.read_u32();
    let _ = chunk.read_u32();
    let _ = chunk.read_u16();
    let _ = chunk.read_u16();
}

fn read_byte_block(chunk: &mut Chunk<'_>, count: usize) -> Option<Vec<u8>> {
    let mut bytes = Vec::with_capacity(count);
    for _ in 0..count {
        bytes.push(chunk.read_u8()?);
    }
    Some(bytes)
}

fn read_u32_from(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(data[offset..offset + 4].try_into().expect("u32 field"))
}

fn drain_remaining_bytes(chunk: &mut Chunk<'_>) -> Vec<u8> {
    let mut body = Vec::new();
    while let Some(byte) = chunk.read_u8() {
        body.push(byte);
    }
    body
}

pub fn bethesda_crc32(data: &[u8]) -> u32 {
    let mut hash = 0u32;
    for byte in data {
        let mut c = (hash ^ u32::from(*byte)) & 0xFF;
        for _ in 0..8 {
            c = if (c & 1) != 0 {
                (c >> 1) ^ 0xEDB88320
            } else {
                c >> 1
            };
        }
        hash = (hash >> 8) ^ c;
    }
    hash
}

pub fn resource_id_from_path(path: &str) -> ResourceId {
    let chars: Vec<char> = path.chars().collect();
    let extension_bytes: Vec<u8> = chars.iter().map(|c| latin1_replace_byte(*c)).collect();
    let pos_slash = chars.iter().rposition(|c| *c == '/');
    let pos_backslash = chars.iter().rposition(|c| *c == '\\');
    let base_name_pos = match (pos_slash, pos_backslash) {
        (None, None) => None,
        (Some(pos), None) => Some(pos),
        (None, Some(pos)) => Some(pos),
        (Some(a), Some(b)) => Some(a.max(b)),
    };
    let mut ext_pos = chars.iter().rposition(|c| *c == '.').unwrap_or(chars.len());
    if base_name_pos.is_some_and(|base| ext_pos < base) {
        ext_pos = chars.len();
    }

    let mut i = 0usize;
    let mut crc_value = 0u32;
    if let Some(base_pos) = base_name_pos {
        while i < base_pos {
            let mut c = chars[i] as u32;
            if (0x41..=0x5A).contains(&c) {
                c |= 0x20;
            } else if c == 0x2F {
                c = 0x5C;
            }
            crc_value = crc_step(crc_value, c as u8);
            i += 1;
        }
        i += 1;
    }
    let dir = crc_value;

    crc_value = 0;
    while i < ext_pos {
        let mut c = chars[i] as u32;
        if (0x41..=0x5A).contains(&c) {
            c |= 0x20;
        }
        crc_value = crc_step(crc_value, c as u8);
        i += 1;
    }
    let file = crc_value;

    let remaining = chars.len().saturating_sub(i);
    let mut ext = if remaining <= 1 {
        0
    } else if remaining == 2 {
        u32::from(extension_bytes[i + 1])
    } else if remaining == 3 {
        u32::from(extension_bytes[i + 1]) | (u32::from(extension_bytes[i + 2]) << 8)
    } else if remaining == 4 {
        let raw = u32::from(extension_bytes[i])
            | (u32::from(extension_bytes[i + 1]) << 8)
            | (u32::from(extension_bytes[i + 2]) << 16)
            | (u32::from(extension_bytes[i + 3]) << 24);
        raw >> 8
    } else {
        u32::from(extension_bytes[i + 1])
            | (u32::from(extension_bytes[i + 2]) << 8)
            | (u32::from(extension_bytes[i + 3]) << 16)
            | (u32::from(extension_bytes[i + 4]) << 24)
    };
    ext = (ext | ((ext >> 1) & 0x20202020)) & 0xFFFF_FFFF;

    ResourceId { dir, file, ext }
}

fn latin1_replace_byte(c: char) -> u8 {
    let value = c as u32;
    if value <= 0xFF { value as u8 } else { b'?' }
}

fn crc_step(hash: u32, byte: u8) -> u32 {
    let mut c = (hash ^ u32::from(byte)) & 0xFF;
    for _ in 0..8 {
        c = if (c & 1) != 0 {
            (c >> 1) ^ 0xEDB88320
        } else {
            c >> 1
        };
    }
    (hash >> 8) ^ c
}
