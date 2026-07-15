//! Skyrim SE NVNM v12 to Fallout 4 NVNM v15 structural bridge.
//!
//! xEdit's TES5/FO4 definitions establish the binary deltas: Skyrim triangles
//! are 16 bytes and edge links are 10 bytes, while FO4 adds a five-byte
//! triangle prefix (height + unknown) and a destination-edge byte. Skyrim's
//! counted cover-triangle list has no lossless FO4 equivalent, so this bridge
//! clears cover flags and emits empty FO4 cover/mapping/waypoint arrays.
//! Callers must pass the complete NAVM set so reciprocal links can supply the
//! destination-edge byte; nonreciprocal links are removed safely.

use std::collections::HashMap;

use super::parser::NvnmError;
use super::types::{
    NvnmDoorRef, NvnmEdgeLink, NvnmGrid, NvnmGridCell, NvnmParent, NvnmPayload, NvnmTriangle,
    NvnmVertex,
};
use super::writer::write_nvnm;

const SKYRIM_NVNM_VERSION: u32 = 12;
const FO4_NVNM_VERSION: u32 = 15;
const SKYRIM_TRIANGLE_ROW_SIZE: usize = 16;
const SKYRIM_EDGE_LINK_ROW_SIZE: usize = 10;
const DOOR_LINK_ROW_SIZE: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkyrimNvnmConversion {
    pub form_id: u32,
    pub bytes: Vec<u8>,
    pub report: SkyrimNvnmConversionReport,
}

#[derive(Debug)]
pub struct SkyrimNvnmConversionFailure {
    pub form_id: u32,
    pub error: NvnmError,
}

#[derive(Debug, Default)]
pub struct SkyrimNvnmConversionBatch {
    pub converted: Vec<SkyrimNvnmConversion>,
    pub failures: Vec<SkyrimNvnmConversionFailure>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SkyrimNvnmConversionReport {
    pub edge_links_resolved: usize,
    pub edge_links_dropped: usize,
    pub cover_triangles_dropped: usize,
    pub triangle_cover_flags_cleared: usize,
}

#[derive(Debug, Clone, Copy)]
struct SkyrimTriangle {
    vertices: [u16; 3],
    links: [i16; 3],
    flags: u16,
    cover_flags: u16,
}

#[derive(Debug, Clone, Copy)]
struct SkyrimEdgeLink {
    kind: u32,
    navmesh: u32,
    triangle: i16,
}

#[derive(Debug)]
struct SkyrimNvnm {
    flags: u32,
    parent: NvnmParent,
    vertices: Vec<NvnmVertex>,
    triangles: Vec<SkyrimTriangle>,
    edge_links: Vec<SkyrimEdgeLink>,
    door_refs: Vec<NvnmDoorRef>,
    cover_triangles: Vec<i16>,
    grid: NvnmGrid,
}

pub fn convert_skyrim_nvnm_set_to_fo4(
    entries: &[(u32, &[u8])],
) -> Result<Vec<SkyrimNvnmConversion>, NvnmError> {
    let SkyrimNvnmConversionBatch {
        converted,
        failures,
    } = convert_skyrim_nvnm_set_to_fo4_lossy(entries);
    if let Some(failure) = failures.into_iter().next() {
        return Err(failure.error);
    }
    Ok(converted)
}

/// Convert every independently valid Skyrim payload and report malformed
/// members without discarding the rest of the plugin's navmesh set.
pub fn convert_skyrim_nvnm_set_to_fo4_lossy(entries: &[(u32, &[u8])]) -> SkyrimNvnmConversionBatch {
    let mut parsed = Vec::with_capacity(entries.len());
    let mut index_by_form_id = HashMap::with_capacity(entries.len());
    let mut failures = Vec::new();
    for &(form_id, bytes) in entries {
        if index_by_form_id.contains_key(&form_id) {
            failures.push(SkyrimNvnmConversionFailure {
                form_id,
                error: NvnmError::Other(format!("duplicate Skyrim NAVM FormID {form_id:08X}")),
            });
            continue;
        }
        match parse_skyrim_nvnm(bytes) {
            Ok(payload) => {
                index_by_form_id.insert(form_id, parsed.len());
                parsed.push((form_id, payload));
            }
            Err(error) => failures.push(SkyrimNvnmConversionFailure { form_id, error }),
        }
    }

    let mut converted = Vec::with_capacity(parsed.len());
    for (form_id, payload) in &parsed {
        let edge_slots = payload
            .edge_links
            .iter()
            .enumerate()
            .map(|(edge_index, edge)| {
                resolve_destination_edge_slot(
                    *form_id,
                    payload,
                    edge_index,
                    *edge,
                    &parsed,
                    &index_by_form_id,
                )
            })
            .collect::<Vec<_>>();
        match convert_payload(*form_id, payload, &edge_slots) {
            Ok(conversion) => converted.push(conversion),
            Err(error) => failures.push(SkyrimNvnmConversionFailure {
                form_id: *form_id,
                error,
            }),
        }
    }
    SkyrimNvnmConversionBatch {
        converted,
        failures,
    }
}

fn resolve_destination_edge_slot(
    source_form_id: u32,
    source: &SkyrimNvnm,
    source_edge_index: usize,
    edge: SkyrimEdgeLink,
    navmeshes: &[(u32, SkyrimNvnm)],
    index_by_form_id: &HashMap<u32, usize>,
) -> Option<u8> {
    let source_edge_index = i16::try_from(source_edge_index).ok()?;
    let &linked_index = index_by_form_id.get(&edge.navmesh)?;
    let linked = &navmeshes.get(linked_index)?.1;
    let linked_triangle = linked.triangles.get(usize::try_from(edge.triangle).ok()?)?;

    let mut resolved = None;
    for (source_triangle_index, triangle) in source.triangles.iter().enumerate() {
        let source_triangle_index = i16::try_from(source_triangle_index).ok()?;
        for source_slot in 0..3 {
            if triangle.flags & (1 << source_slot) == 0
                || triangle.links[source_slot] != source_edge_index
            {
                continue;
            }
            let reciprocal = (0..3).find(|&linked_slot| {
                if linked_triangle.flags & (1 << linked_slot) == 0 {
                    return false;
                }
                let linked_edge_index = linked_triangle.links[linked_slot];
                let Some(linked_edge) = usize::try_from(linked_edge_index)
                    .ok()
                    .and_then(|index| linked.edge_links.get(index))
                else {
                    return false;
                };
                linked_edge.navmesh == source_form_id
                    && linked_edge.triangle == source_triangle_index
            })? as u8;
            match resolved {
                None => resolved = Some(reciprocal),
                Some(existing) if existing == reciprocal => {}
                Some(_) => return None,
            }
        }
    }
    resolved
}

fn convert_payload(
    form_id: u32,
    source: &SkyrimNvnm,
    edge_slots: &[Option<u8>],
) -> Result<SkyrimNvnmConversion, NvnmError> {
    let report = SkyrimNvnmConversionReport {
        edge_links_resolved: edge_slots.iter().filter(|slot| slot.is_some()).count(),
        edge_links_dropped: edge_slots.iter().filter(|slot| slot.is_none()).count(),
        cover_triangles_dropped: source.cover_triangles.len(),
        triangle_cover_flags_cleared: source
            .triangles
            .iter()
            .filter(|triangle| triangle.cover_flags != 0)
            .count(),
    };
    let mut edge_remap = vec![None; source.edge_links.len()];
    let mut edge_links = Vec::with_capacity(report.edge_links_resolved);
    for (old_index, (edge, edge_slot)) in
        source.edge_links.iter().zip(edge_slots.iter()).enumerate()
    {
        let Some(edge_slot) = edge_slot else {
            continue;
        };
        let new_index = i16::try_from(edge_links.len()).map_err(|_| {
            NvnmError::Other(format!(
                "Skyrim NAVM {form_id:08X} has too many retained edge links"
            ))
        })?;
        edge_remap[old_index] = Some(new_index);
        let mut row = [0u8; 11];
        row[0..4].copy_from_slice(&edge.kind.to_le_bytes());
        row[4..8].copy_from_slice(&edge.navmesh.to_le_bytes());
        row[8..10].copy_from_slice(&edge.triangle.to_le_bytes());
        row[10] = *edge_slot;
        edge_links.push(NvnmEdgeLink { row });
    }

    let triangles = source
        .triangles
        .iter()
        .map(|source_triangle| {
            let mut links = source_triangle.links;
            let mut flags = source_triangle.flags;
            for slot in 0..3 {
                if flags & (1 << slot) == 0 {
                    continue;
                }
                let replacement = usize::try_from(links[slot])
                    .ok()
                    .and_then(|index| edge_remap.get(index))
                    .copied()
                    .flatten();
                if let Some(replacement) = replacement {
                    links[slot] = replacement;
                } else {
                    flags &= !(1 << slot);
                    links[slot] = -1;
                }
            }
            let mut cover_marker = [0u8; 9];
            // f32::MAX is the established "unset" height sentinel in the
            // committed FO4 NVNM corpus; zero is the observed unknown byte.
            cover_marker[0..4].copy_from_slice(&f32::MAX.to_le_bytes());
            cover_marker[5..7].copy_from_slice(&flags.to_le_bytes());
            NvnmTriangle {
                vertices: source_triangle.vertices,
                links,
                cover_marker,
                flags,
            }
        })
        .collect();

    let target = NvnmPayload {
        version: FO4_NVNM_VERSION,
        flags: source.flags,
        parent: source.parent,
        vertices: source.vertices.clone(),
        triangles,
        edge_links,
        door_refs: source.door_refs.clone(),
        cover_array: Vec::new(),
        cover_triangle_mappings: Vec::new(),
        waypoints: Vec::new(),
        grid: source.grid.clone(),
    };
    Ok(SkyrimNvnmConversion {
        form_id,
        bytes: write_nvnm(&target),
        report,
    })
}

fn parse_skyrim_nvnm(bytes: &[u8]) -> Result<SkyrimNvnm, NvnmError> {
    let mut cursor = Cursor::new(bytes);
    let version = cursor.u32("version")?;
    if version != SKYRIM_NVNM_VERSION {
        return Err(NvnmError::Other(format!(
            "expected Skyrim NVNM version {SKYRIM_NVNM_VERSION}, got {version}"
        )));
    }
    let flags = cursor.u32("flags")?;
    let parent_world = cursor.u32("parent world")?;
    let parent = if parent_world == 0 {
        NvnmParent::Interior {
            cell: cursor.u32("parent cell")?,
        }
    } else {
        let grid_y = cursor.i16("grid y")?;
        let grid_x = cursor.i16("grid x")?;
        NvnmParent::Exterior {
            world: parent_world,
            grid_x,
            grid_y,
        }
    };

    let vertices = cursor.counted("vertices", 12, |cursor| {
        Ok(NvnmVertex {
            x: cursor.f32("vertex x")?,
            y: cursor.f32("vertex y")?,
            z: cursor.f32("vertex z")?,
        })
    })?;
    let triangles = cursor.counted("triangles", SKYRIM_TRIANGLE_ROW_SIZE, |cursor| {
        Ok(SkyrimTriangle {
            vertices: [
                cursor.u16("triangle vertex 0")?,
                cursor.u16("triangle vertex 1")?,
                cursor.u16("triangle vertex 2")?,
            ],
            links: [
                cursor.i16("triangle edge 0")?,
                cursor.i16("triangle edge 1")?,
                cursor.i16("triangle edge 2")?,
            ],
            flags: cursor.u16("triangle flags")?,
            cover_flags: cursor.u16("triangle cover flags")?,
        })
    })?;
    let edge_links = cursor.counted("edge links", SKYRIM_EDGE_LINK_ROW_SIZE, |cursor| {
        Ok(SkyrimEdgeLink {
            kind: cursor.u32("edge link kind")?,
            navmesh: cursor.u32("edge link navmesh")?,
            triangle: cursor.i16("edge link triangle")?,
        })
    })?;
    let door_refs = cursor.counted("door links", DOOR_LINK_ROW_SIZE, |cursor| {
        let triangle_index = cursor.i16("door link triangle")?;
        let mut padding = [0u8; 4];
        padding.copy_from_slice(cursor.bytes(4, "door link crc")?);
        Ok(NvnmDoorRef {
            triangle_index,
            padding,
            door_ref_form_id: cursor.u32("door link reference")?,
        })
    })?;
    let cover_triangles =
        cursor.counted("cover triangles", 2, |cursor| cursor.i16("cover triangle"))?;
    let divisor = cursor.u32("navmesh grid divisor")?;
    let grid = if divisor == 0 {
        NvnmGrid::default()
    } else {
        let grid_size_x = cursor.f32("grid size x")?;
        let grid_size_y = cursor.f32("grid size y")?;
        let bounds_min_x = cursor.f32("bounds min x")?;
        let bounds_min_y = cursor.f32("bounds min y")?;
        let bounds_min_z = cursor.f32("bounds min z")?;
        let bounds_max_x = cursor.f32("bounds max x")?;
        let bounds_max_y = cursor.f32("bounds max y")?;
        let bounds_max_z = cursor.f32("bounds max z")?;
        let cell_count = (divisor as usize)
            .checked_mul(divisor as usize)
            .ok_or_else(|| NvnmError::Other(format!("grid divisor squared overflow: {divisor}")))?;
        let mut cells = Vec::with_capacity(cell_count.min(cursor.remaining() / 4));
        for _ in 0..cell_count {
            cells.push(NvnmGridCell {
                triangle_indices: cursor
                    .counted("grid cell", 2, |cursor| cursor.i16("grid triangle"))?,
            });
        }
        NvnmGrid {
            divisor,
            grid_size_x,
            grid_size_y,
            bounds_min_x,
            bounds_min_y,
            bounds_min_z,
            bounds_max_x,
            bounds_max_y,
            bounds_max_z,
            cells,
        }
    };
    if cursor.remaining() != 0 {
        return Err(NvnmError::Other(format!(
            "Skyrim NVNM has {} trailing bytes",
            cursor.remaining()
        )));
    }
    Ok(SkyrimNvnm {
        flags,
        parent,
        vertices,
        triangles,
        edge_links,
        door_refs,
        cover_triangles,
        grid,
    })
}

pub(crate) fn collect_skyrim_nvnm_form_ids(bytes: &[u8], out: &mut Vec<u32>) -> bool {
    let Ok(offsets) = skyrim_nvnm_form_id_offsets(bytes) else {
        return false;
    };
    for offset in offsets {
        let raw = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
        if raw != 0 {
            out.push(raw);
        }
    }
    true
}

pub(crate) fn rewrite_skyrim_nvnm_form_ids(
    bytes: &mut [u8],
    rewrite_formid: &mut dyn FnMut(u32) -> Option<u32>,
) -> bool {
    let Ok(offsets) = skyrim_nvnm_form_id_offsets(bytes) else {
        return false;
    };
    let mut changed = false;
    for offset in offsets {
        let raw = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
        if raw == 0 {
            continue;
        }
        if let Some(rewritten) = rewrite_formid(raw)
            && rewritten != raw
        {
            bytes[offset..offset + 4].copy_from_slice(&rewritten.to_le_bytes());
            changed = true;
        }
    }
    changed
}

fn skyrim_nvnm_form_id_offsets(bytes: &[u8]) -> Result<Vec<usize>, NvnmError> {
    parse_skyrim_nvnm(bytes)?;

    let mut cursor = Cursor::new(bytes);
    let version = cursor.u32("version")?;
    if version != SKYRIM_NVNM_VERSION {
        return Err(NvnmError::Other(format!(
            "expected Skyrim NVNM version {SKYRIM_NVNM_VERSION}, got {version}"
        )));
    }
    cursor.u32("flags")?;
    let parent_world_offset = cursor.offset;
    let parent_world = cursor.u32("parent world")?;
    let mut offsets = Vec::new();
    if parent_world == 0 {
        offsets.push(cursor.offset);
        cursor.u32("parent cell")?;
    } else {
        offsets.push(parent_world_offset);
        cursor.i16("grid y")?;
        cursor.i16("grid x")?;
    }

    cursor.counted("vertices", 12, |cursor| {
        cursor.bytes(12, "vertex")?;
        Ok(())
    })?;
    cursor.counted("triangles", SKYRIM_TRIANGLE_ROW_SIZE, |cursor| {
        cursor.bytes(SKYRIM_TRIANGLE_ROW_SIZE, "triangle")?;
        Ok(())
    })?;
    cursor.counted("edge links", SKYRIM_EDGE_LINK_ROW_SIZE, |cursor| {
        cursor.u32("edge link kind")?;
        offsets.push(cursor.offset);
        cursor.u32("edge link navmesh")?;
        cursor.i16("edge link triangle")?;
        Ok(())
    })?;
    cursor.counted("door links", DOOR_LINK_ROW_SIZE, |cursor| {
        cursor.i16("door link triangle")?;
        cursor.bytes(4, "door link crc")?;
        offsets.push(cursor.offset);
        cursor.u32("door link reference")?;
        Ok(())
    })?;
    Ok(offsets)
}

struct Cursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.offset)
    }

    fn bytes(&mut self, count: usize, label: &'static str) -> Result<&'a [u8], NvnmError> {
        let end = self.offset.checked_add(count).ok_or_else(|| {
            NvnmError::Other(format!("Skyrim NVNM offset overflow reading {label}"))
        })?;
        if end > self.bytes.len() {
            return Err(NvnmError::Truncated {
                label,
                offset: self.offset,
                need: count,
                have: self.remaining(),
            });
        }
        let value = &self.bytes[self.offset..end];
        self.offset = end;
        Ok(value)
    }

    fn u16(&mut self, label: &'static str) -> Result<u16, NvnmError> {
        Ok(u16::from_le_bytes(
            self.bytes(2, label)?.try_into().unwrap(),
        ))
    }

    fn i16(&mut self, label: &'static str) -> Result<i16, NvnmError> {
        Ok(i16::from_le_bytes(
            self.bytes(2, label)?.try_into().unwrap(),
        ))
    }

    fn u32(&mut self, label: &'static str) -> Result<u32, NvnmError> {
        Ok(u32::from_le_bytes(
            self.bytes(4, label)?.try_into().unwrap(),
        ))
    }

    fn f32(&mut self, label: &'static str) -> Result<f32, NvnmError> {
        Ok(f32::from_le_bytes(
            self.bytes(4, label)?.try_into().unwrap(),
        ))
    }

    fn counted<T>(
        &mut self,
        label: &'static str,
        row_size: usize,
        mut parse: impl FnMut(&mut Self) -> Result<T, NvnmError>,
    ) -> Result<Vec<T>, NvnmError> {
        let count = self.u32(label)? as usize;
        let required = count
            .checked_mul(row_size)
            .ok_or_else(|| NvnmError::Other(format!("Skyrim NVNM {label} byte count overflow")))?;
        if required > self.remaining() {
            return Err(NvnmError::Truncated {
                label,
                offset: self.offset,
                need: required,
                have: self.remaining(),
            });
        }
        let mut values = Vec::with_capacity(count);
        for _ in 0..count {
            values.push(parse(self)?);
        }
        Ok(values)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nvnm::{parse_nvnm, write_nvnm};

    fn triangle(vertices: [u16; 3], links: [i16; 3], flags: u16, cover: u16) -> [u8; 16] {
        let mut row = [0u8; 16];
        for (index, value) in vertices.into_iter().enumerate() {
            row[index * 2..index * 2 + 2].copy_from_slice(&value.to_le_bytes());
        }
        for (index, value) in links.into_iter().enumerate() {
            let offset = 6 + index * 2;
            row[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
        }
        row[12..14].copy_from_slice(&flags.to_le_bytes());
        row[14..16].copy_from_slice(&cover.to_le_bytes());
        row
    }

    fn skyrim_payload(
        form_id: u32,
        linked_form_id: Option<u32>,
        external_slot: usize,
    ) -> (u32, Vec<u8>) {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&SKYRIM_NVNM_VERSION.to_le_bytes());
        bytes.extend_from_slice(&0xA5A5A5A5u32.to_le_bytes());
        bytes.extend_from_slice(&0x0000003Cu32.to_le_bytes());
        bytes.extend_from_slice(&0i16.to_le_bytes());
        bytes.extend_from_slice(&1i16.to_le_bytes());
        bytes.extend_from_slice(&3u32.to_le_bytes());
        for vertex in [(0.0f32, 0.0f32, 1.0f32), (1.0, 0.0, 2.0), (0.0, 1.0, 3.0)] {
            bytes.extend_from_slice(&vertex.0.to_le_bytes());
            bytes.extend_from_slice(&vertex.1.to_le_bytes());
            bytes.extend_from_slice(&vertex.2.to_le_bytes());
        }
        bytes.extend_from_slice(&1u32.to_le_bytes());
        let mut links = [-1i16; 3];
        let mut flags = 0x0800u16;
        if linked_form_id.is_some() {
            links[external_slot] = 0;
            flags |= 1 << external_slot;
        }
        bytes.extend_from_slice(&triangle([0, 1, 2], links, flags, 0x4001));
        bytes.extend_from_slice(&(linked_form_id.is_some() as u32).to_le_bytes());
        if let Some(linked_form_id) = linked_form_id {
            bytes.extend_from_slice(&0u32.to_le_bytes());
            bytes.extend_from_slice(&linked_form_id.to_le_bytes());
            bytes.extend_from_slice(&0i16.to_le_bytes());
        }
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&0i16.to_le_bytes());
        bytes.extend_from_slice(&[1, 2, 3, 4]);
        bytes.extend_from_slice(&0x00001234u32.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&0i16.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        for value in [1.0f32, 1.0, 0.0, 0.0, 1.0, 1.0, 1.0, 3.0] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&0i16.to_le_bytes());
        (form_id, bytes)
    }

    #[test]
    fn converts_v12_geometry_door_and_grid_to_roundtrippable_v15() {
        let (form_id, source) = skyrim_payload(0x0100, None, 0);
        let converted = convert_skyrim_nvnm_set_to_fo4(&[(form_id, &source)]).unwrap();
        let target = parse_nvnm(&converted[0].bytes).unwrap();

        assert_eq!(target.version, 15);
        assert_eq!(target.flags, 0xA5A5A5A5);
        assert_eq!(target.vertices.len(), 3);
        assert_eq!(target.triangles[0].vertices, [0, 1, 2]);
        assert_eq!(target.triangles[0].links, [-1, -1, -1]);
        assert_eq!(target.triangles[0].flags, 0x0800);
        assert_eq!(
            f32::from_le_bytes(target.triangles[0].cover_marker[0..4].try_into().unwrap()),
            f32::MAX
        );
        assert_eq!(&target.triangles[0].cover_marker[7..9], &[0, 0]);
        assert_eq!(target.door_refs[0].door_ref_form_id, 0x00001234);
        assert_eq!(target.grid.divisor, 1);
        assert_eq!(target.grid.cells[0].triangle_indices, [0]);
        assert!(target.cover_array.is_empty());
        assert!(target.cover_triangle_mappings.is_empty());
        assert!(target.waypoints.is_empty());
        assert_eq!(write_nvnm(&target), converted[0].bytes);
        assert_eq!(converted[0].report.cover_triangles_dropped, 1);
        assert_eq!(converted[0].report.triangle_cover_flags_cleared, 1);
    }

    #[test]
    fn converts_real_skyrim_0e537d_fixture_without_geometry_loss() {
        // Lossless NVNM extraction from Skyrim.esm NAVM:000E537D.
        let source =
            hex::decode(include_str!("tests/fixtures/0e537d_skyrim_v12.nvnm.hex").trim()).unwrap();
        let source_payload = parse_skyrim_nvnm(&source).unwrap();
        let converted = convert_skyrim_nvnm_set_to_fo4(&[(0x000E537D, &source)]).unwrap();
        let target = parse_nvnm(&converted[0].bytes).unwrap();

        assert_eq!(target.version, 15);
        assert_eq!(target.flags, 0xA5E9A03C);
        assert_eq!(target.parent, NvnmParent::Interior { cell: 0x00013A7E });
        assert_eq!(target.vertices.len(), 95);
        assert_eq!(target.triangles.len(), 98);
        assert_eq!(target.door_refs.len(), 1);
        assert_eq!(target.grid.divisor, 3);
        assert_eq!(target.grid.cells.len(), 9);
        assert_eq!(target.vertices, source_payload.vertices);
        assert_eq!(target.door_refs, source_payload.door_refs);
        assert_eq!(target.grid, source_payload.grid);
        for (target_triangle, source_triangle) in
            target.triangles.iter().zip(source_payload.triangles.iter())
        {
            assert_eq!(target_triangle.vertices, source_triangle.vertices);
            assert_eq!(target_triangle.links, source_triangle.links);
            assert_eq!(target_triangle.flags, source_triangle.flags);
        }
        assert_eq!(write_nvnm(&target), converted[0].bytes);
    }

    #[test]
    fn derives_fo4_destination_edge_index_from_reciprocal_skyrim_links() {
        let (form_a, bytes_a) = skyrim_payload(0x0100, Some(0x0200), 2);
        let (form_b, bytes_b) = skyrim_payload(0x0200, Some(0x0100), 1);
        let converted =
            convert_skyrim_nvnm_set_to_fo4(&[(form_a, &bytes_a), (form_b, &bytes_b)]).unwrap();
        let a = parse_nvnm(&converted[0].bytes).unwrap();
        let b = parse_nvnm(&converted[1].bytes).unwrap();

        assert_eq!(a.edge_links[0].row[10], 1);
        assert_eq!(b.edge_links[0].row[10], 2);
        assert_eq!(a.triangles[0].links[2], 0);
        assert_eq!(b.triangles[0].links[1], 0);
        assert_eq!(converted[0].report.edge_links_resolved, 1);
        assert_eq!(converted[1].report.edge_links_resolved, 1);
    }

    #[test]
    fn drops_nonreciprocal_edge_and_repairs_triangle_slot() {
        let (form_id, source) = skyrim_payload(0x0100, Some(0x9999), 0);
        let converted = convert_skyrim_nvnm_set_to_fo4(&[(form_id, &source)]).unwrap();
        let target = parse_nvnm(&converted[0].bytes).unwrap();

        assert!(target.edge_links.is_empty());
        assert_eq!(target.triangles[0].flags & 0x0007, 0);
        assert_eq!(target.triangles[0].links[0], -1);
        assert_eq!(converted[0].report.edge_links_dropped, 1);
    }

    #[test]
    fn lossy_conversion_keeps_valid_navmeshes_and_repairs_links_to_malformed_members() {
        let (form_id, source) = skyrim_payload(0x0100, Some(0x0200), 0);
        let malformed = [12, 0, 0, 0];

        let batch = convert_skyrim_nvnm_set_to_fo4_lossy(&[
            (form_id, &source),
            (0x0200, malformed.as_slice()),
        ]);

        assert_eq!(batch.failures.len(), 1);
        assert_eq!(batch.failures[0].form_id, 0x0200);
        assert_eq!(batch.converted.len(), 1);
        let target = parse_nvnm(&batch.converted[0].bytes).unwrap();
        assert!(target.edge_links.is_empty());
        assert_eq!(target.triangles[0].flags & 0x0007, 0);
        assert_eq!(target.triangles[0].links[0], -1);
    }

    #[test]
    fn rejects_non_v12_and_truncated_payloads() {
        let (_, mut source) = skyrim_payload(0x0100, None, 0);
        source[0..4].copy_from_slice(&15u32.to_le_bytes());
        assert!(convert_skyrim_nvnm_set_to_fo4(&[(0x0100, &source)]).is_err());

        source[0..4].copy_from_slice(&12u32.to_le_bytes());
        source.truncate(source.len() - 1);
        assert!(convert_skyrim_nvnm_set_to_fo4(&[(0x0100, &source)]).is_err());
    }
}
