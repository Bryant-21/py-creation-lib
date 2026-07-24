pub mod aabb_tree;
pub mod capsule;
pub mod compound;
pub mod compressed_mesh;
pub mod constants;
pub mod constraints;
pub mod convex;
pub mod hull;
pub mod mass_properties;
pub mod multi_body;
pub mod payload;
pub mod polytope;
pub mod preview;
pub mod sphere;
pub mod tagged_writer;
pub mod validate;

use std::cell::RefCell;

pub use capsule::{
    SourceCapsuleShape, build_fo4_capsule_collision, build_fo4_source_capsule_collision,
};
pub use compound::{
    CompoundChild, CompoundChildKind, SHAPE_INST_DEPRECATED, SHAPE_INST_HAS_ROTATION,
    SHAPE_INST_HAS_SCALE, SHAPE_INST_HAS_TRANSLATION, SHAPE_INST_IS_ENABLED,
    SHAPE_INST_SCALE_SURFACE, build_fo4_compound_collision, pack_inst_row_w,
};
pub use compressed_mesh::{
    BuildOptions, CompressedMeshData, CompressedMeshSection, RawCompressedMeshBitField,
    RawCompressedMeshData, RawCompressedMeshDataRun, RawCompressedMeshSection,
    RawCompressedMeshSparseMap, build_compressed_mesh_collision,
    build_compressed_mesh_collision_from_raw, compressed_triangle_is_safe, pack_vertex_11_11_10,
    pack_vertex_21_21_22, triangle_area_squared, validate_compressed_triangle, validate_vertices,
    vertex_is_finite,
};
pub use constraints::{GraftCinfo, GraftedConstraints, extract_grafted_constraints};
pub use convex::{SourceConvexShape, build_fo4_source_convex_collision};
pub use mass_properties::{SourceMassDistribution, mass_properties_from_source};
pub use multi_body::{MultiBodyShape, build_fo4_multi_body_collision};
pub use payload::{
    ItemRecord, ParsedCollision, PatchRecord, build_convex_collision, parse_tagged_collision,
    rebuild_tag0_collision,
};
pub use polytope::{
    SourcePolytopeShape, build_fo4_polytope_collision, build_fo4_source_polytope_collision,
};
pub use preview::{
    PreviewMesh, SourceBodyTransform, SourcePrimitiveShape, collision_preview_json,
    decode_source_body_transforms, decode_source_mass_distributions,
    extract_direct_raw_compressed_mesh_from_blob, extract_direct_source_primitive_from_blob,
    extract_preview_meshes_from_blob, extract_preview_meshes_from_hkx,
    extract_raw_compressed_meshes_from_blob, extract_raw_compressed_meshes_from_hkx,
    extract_source_compound_children_from_blob, extract_source_polytopes_from_blob,
};
pub use sphere::build_fo4_sphere_collision;
pub use tagged_writer::{PatchEntry, TaggedBlobBuilder, TaggedItem};

use crate::error::{HavokError, HavokResult};
use crate::hkx::tagfile::{TagfileItem, TagfileSection};

const HFF_HEADER_SIZE: usize = 8;

thread_local! {
    static COLLISION_DIAGNOSTIC_CONTEXT: RefCell<Vec<String>> = RefCell::new(Vec::new());
}

struct CollisionDiagnosticContextGuard;

impl Drop for CollisionDiagnosticContextGuard {
    fn drop(&mut self) {
        COLLISION_DIAGNOSTIC_CONTEXT.with(|stack| {
            stack.borrow_mut().pop();
        });
    }
}

pub fn with_collision_diagnostic_context<T, F>(context: impl Into<String>, f: F) -> T
where
    F: FnOnce() -> T,
{
    let context = context.into();
    if context.trim().is_empty() {
        return f();
    }
    COLLISION_DIAGNOSTIC_CONTEXT.with(|stack| {
        stack.borrow_mut().push(context);
    });
    let _guard = CollisionDiagnosticContextGuard;
    f()
}

pub(crate) fn collision_diagnostic_context() -> Option<String> {
    COLLISION_DIAGNOSTIC_CONTEXT.with(|stack| stack.borrow().last().cloned())
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tag0CollisionPayload {
    pub format: String,
    pub sections: Vec<TagfileSection>,
    pub items: Vec<TagfileItem>,
    pub vertices: Vec<[f32; 3]>,
    pub planes: Vec<[f32; 4]>,
    pub faces: Vec<(u16, u8, u8)>,
    pub edges: Vec<(u16, u8, u8)>,
    pub indices: Vec<u8>,
    pub vertex_edges: Vec<u32>,
}

pub fn parse_tag0_collision_payload(blob: &[u8]) -> HavokResult<Tag0CollisionPayload> {
    let mut sections = Vec::new();
    parse_hff_sections(blob, 0, blob.len(), true, &mut sections)?;
    let data_section = sections
        .iter()
        .find(|section| section.tag == "DATA")
        .ok_or_else(|| {
            HavokError::InvalidInput("TAG0 collision payload missing DATA section".to_string())
        })?;
    let data_start = data_section.content_offset;
    let data_size = data_section.content_size;
    let item_raw = section_bytes(blob, &sections, "ITEM").ok_or_else(|| {
        HavokError::InvalidInput("TAG0 collision payload missing ITEM section".to_string())
    })?;
    let items = parse_items(item_raw)?;
    let mut sorted_items: Vec<(usize, &TagfileItem)> = items.iter().enumerate().collect();
    sorted_items.sort_by_key(|(_, item)| item.offset);

    let mut vertices = Vec::new();
    let mut planes = Vec::new();
    let mut vec4_arrays: Vec<(usize, usize)> = Vec::new();
    let mut four_byte_arrays: Vec<(&TagfileItem, usize)> = Vec::new();
    let mut indices = Vec::new();

    for (sorted_index, (_, item)) in sorted_items.iter().enumerate() {
        if item.offset >= data_size || item.count == 0 {
            continue;
        }
        let end = sorted_items
            .iter()
            .skip(sorted_index + 1)
            .map(|(_, next)| next.offset)
            .find(|offset| *offset > item.offset)
            .unwrap_or(data_size)
            .min(data_size);
        if end <= item.offset {
            continue;
        }
        let abs_start = data_start + item.offset;
        let abs_end = data_start + end;
        let total_bytes = abs_end - abs_start;
        let elem_size = total_bytes as f32 / item.count as f32;

        if item.kind != 2 || item.count <= 1 {
            continue;
        }
        if (elem_size - 12.0).abs() < 0.5 {
            vertices = (0..item.count)
                .filter_map(|index| read_f32x3(blob, abs_start + index * 12))
                .collect();
        } else if (elem_size - 16.0).abs() < 0.5 {
            vec4_arrays.push((item.count, abs_start));
        } else if (elem_size - 4.0).abs() < 0.5 {
            four_byte_arrays.push((item, abs_start));
        } else if elem_size < 2.0 {
            indices = blob[abs_start..abs_start + item.count.min(total_bytes)].to_vec();
        }
    }

    if vertices.is_empty() {
        if let Some((count, abs_start)) = vec4_arrays.first().copied() {
            vertices = (0..count)
                .filter_map(|index| read_f32x4(blob, abs_start + index * 16))
                .map(|[x, y, z, _]| [x, y, z])
                .collect();
        }
    }
    if planes.is_empty() {
        let plane_source = vec4_arrays.get(1).or_else(|| vec4_arrays.first()).copied();
        if let Some((count, abs_start)) = plane_source {
            planes = (0..count)
                .filter_map(|index| read_f32x4(blob, abs_start + index * 16))
                .collect();
        }
    }

    let mut faces = Vec::new();
    let mut edges = Vec::new();
    let mut vertex_edges = Vec::new();
    for (item, abs_start) in four_byte_arrays {
        if item.count == planes.len() {
            faces = read_u16_u8_u8_records(blob, abs_start, item.count);
        } else if item.count == vertices.len() {
            vertex_edges = read_u16_u8_u8_records(blob, abs_start, item.count)
                .into_iter()
                .map(|(face, edge, _)| u32::from(face) | (u32::from(edge) << 16))
                .collect();
        } else {
            edges = read_u16_u8_u8_records(blob, abs_start, item.count);
        }
    }

    Ok(Tag0CollisionPayload {
        format: "tag0".to_string(),
        sections,
        items,
        vertices,
        planes,
        faces,
        edges,
        indices,
        vertex_edges,
    })
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
        let tag = std::str::from_utf8(&data[offset + 4..offset + 8])
            .map_err(|_| HavokError::InvalidInput("HFF tag is not valid UTF-8".to_string()))?
            .to_string();
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

fn parse_items(data: &[u8]) -> HavokResult<Vec<TagfileItem>> {
    if !data.len().is_multiple_of(12) {
        return Err(HavokError::InvalidInput(format!(
            "ITEM section length {} is not divisible by 12",
            data.len()
        )));
    }
    data.chunks_exact(12)
        .map(|chunk| {
            let packed = read_u32_le(chunk, 0, "ITEM packed")?;
            Ok(TagfileItem {
                kind: ((packed >> 28) & 0xF) as u8,
                flags: ((packed >> 24) & 0xF) as u8,
                type_id: (packed & 0x00FF_FFFF) as usize,
                offset: read_u32_le(chunk, 4, "ITEM offset")? as usize,
                count: read_u32_le(chunk, 8, "ITEM count")? as usize,
            })
        })
        .collect()
}

fn section_bytes<'a>(data: &'a [u8], sections: &[TagfileSection], tag: &str) -> Option<&'a [u8]> {
    sections
        .iter()
        .find(|section| section.tag == tag)
        .map(|section| &data[section.content_offset..section.content_offset + section.content_size])
}

pub fn parse_fo4_compressed_mesh(blob: &[u8]) -> HavokResult<CompressedMeshData> {
    compressed_mesh::parse_fo4_compressed_mesh(blob)
}

pub fn unpack_vertex_11_11_10(packed: u32) -> (u32, u32, u32) {
    (
        packed & 0x7FF,
        (packed >> 11) & 0x7FF,
        (packed >> 22) & 0x3FF,
    )
}

pub fn unpack_vertex_21_21_22(packed: u64) -> (u64, u64, u64) {
    (
        packed & 0x1F_FFFF,
        (packed >> 21) & 0x1F_FFFF,
        (packed >> 42) & 0x3F_FFFF,
    )
}

fn read_f32x3(data: &[u8], offset: usize) -> Option<[f32; 3]> {
    if offset + 12 > data.len() {
        return None;
    }
    Some([
        read_f32_le(data, offset),
        read_f32_le(data, offset + 4),
        read_f32_le(data, offset + 8),
    ])
}

fn read_f32x4(data: &[u8], offset: usize) -> Option<[f32; 4]> {
    if offset + 16 > data.len() {
        return None;
    }
    Some([
        read_f32_le(data, offset),
        read_f32_le(data, offset + 4),
        read_f32_le(data, offset + 8),
        read_f32_le(data, offset + 12),
    ])
}

fn read_u16_u8_u8_records(data: &[u8], offset: usize, count: usize) -> Vec<(u16, u8, u8)> {
    (0..count)
        .filter_map(|index| {
            let offset = offset + index * 4;
            if offset + 4 > data.len() {
                return None;
            }
            Some((
                u16::from_le_bytes(data[offset..offset + 2].try_into().unwrap()),
                data[offset + 2],
                data[offset + 3],
            ))
        })
        .collect()
}

fn read_f32_le(data: &[u8], offset: usize) -> f32 {
    f32::from_le_bytes(data[offset..offset + 4].try_into().unwrap())
}

fn read_u32_be(data: &[u8], offset: usize, context: &str) -> HavokResult<u32> {
    if data.len() < offset + 4 {
        return Err(HavokError::InvalidInput(format!(
            "{context} needs {} bytes, got {}",
            offset + 4,
            data.len()
        )));
    }
    Ok(u32::from_be_bytes(
        data[offset..offset + 4].try_into().unwrap(),
    ))
}

fn read_u32_le(data: &[u8], offset: usize, context: &str) -> HavokResult<u32> {
    if data.len() < offset + 4 {
        return Err(HavokError::InvalidInput(format!(
            "{context} needs {} bytes, got {}",
            offset + 4,
            data.len()
        )));
    }
    Ok(u32::from_le_bytes(
        data[offset..offset + 4].try_into().unwrap(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collision_diagnostic_context_is_scoped() {
        assert_eq!(collision_diagnostic_context(), None);
        let inner = with_collision_diagnostic_context(
            "nif=meshes/example.nif",
            collision_diagnostic_context,
        );
        assert_eq!(inner.as_deref(), Some("nif=meshes/example.nif"));
        assert_eq!(collision_diagnostic_context(), None);
    }
}
