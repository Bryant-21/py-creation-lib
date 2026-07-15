use super::types::{
    NvnmCoverEntry, NvnmCoverTriangleMapping, NvnmDoorRef, NvnmEdgeLink, NvnmGrid, NvnmGridCell,
    NvnmParent, NvnmPayload, NvnmTriangle, NvnmVertex, NvnmWaypoint,
};

const EDGE_LINK_ROW_SIZE: usize = 11;
const DOOR_REF_ROW_SIZE: usize = 10;

const TRIANGLE_ROW_SIZE: usize = 21;
const TRIANGLE_FLAGS_OFFSET: usize = 17;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum NvnmError {
    #[error("NVNM payload truncated: {label} at offset {offset} (need {need} bytes, have {have})")]
    Truncated {
        label: &'static str,
        offset: usize,
        need: usize,
        have: usize,
    },
    #[error("NVNM error: {0}")]
    Other(String),
}

fn need(bytes: &[u8], offset: usize, count: usize, label: &'static str) -> Result<(), NvnmError> {
    if offset
        .checked_add(count)
        .map_or(true, |end| end > bytes.len())
    {
        return Err(NvnmError::Truncated {
            label,
            offset,
            need: count,
            have: bytes.len().saturating_sub(offset),
        });
    }
    Ok(())
}

/// Bound an attacker-controlled `count` against the bytes actually remaining,
/// so a corrupt `0xFFFFFFFF` count can't make `Vec::with_capacity` allocate
/// gigabytes before the subsequent `need(...)` check would have rejected the
/// payload anyway.
fn safe_capacity(count: u32, remaining_bytes: usize, row_size: usize) -> usize {
    let upper_bound = remaining_bytes / row_size.max(1);
    (count as usize).min(upper_bound)
}

fn read_u32(bytes: &[u8], offset: usize, label: &'static str) -> Result<u32, NvnmError> {
    need(bytes, offset, 4, label)?;
    Ok(u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ]))
}

fn read_u16(bytes: &[u8], offset: usize, label: &'static str) -> Result<u16, NvnmError> {
    need(bytes, offset, 2, label)?;
    Ok(u16::from_le_bytes([bytes[offset], bytes[offset + 1]]))
}

fn read_i16(bytes: &[u8], offset: usize, label: &'static str) -> Result<i16, NvnmError> {
    need(bytes, offset, 2, label)?;
    Ok(i16::from_le_bytes([bytes[offset], bytes[offset + 1]]))
}

fn read_f32(bytes: &[u8], offset: usize, label: &'static str) -> Result<f32, NvnmError> {
    need(bytes, offset, 4, label)?;
    Ok(f32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ]))
}

pub fn parse_nvnm(bytes: &[u8]) -> Result<NvnmPayload, NvnmError> {
    if bytes.len() < 16 {
        return Err(NvnmError::Truncated {
            label: "NVNM header",
            offset: 0,
            need: 16,
            have: bytes.len(),
        });
    }
    // bytes[0..4]   = version (u32, expected 15)
    // bytes[4..8]   = flags (u32, opaque)
    // bytes[8..12]  = parent_world (u32; 0 → interior)
    // bytes[12..16] = if interior: cell u32; else: grid_y i16 (12..14), grid_x i16 (14..16)
    let version = read_u32(bytes, 0, "version")?;
    let flags = read_u32(bytes, 4, "flags")?;
    let parent_world = read_u32(bytes, 8, "parent world")?;
    let parent = if parent_world == 0 {
        let cell = read_u32(bytes, 12, "parent cell")?;
        NvnmParent::Interior { cell }
    } else {
        let grid_y = read_i16(bytes, 12, "grid y")?;
        let grid_x = read_i16(bytes, 14, "grid x")?;
        NvnmParent::Exterior {
            world: parent_world,
            grid_x,
            grid_y,
        }
    };

    let mut offset = 16usize;
    let vertex_count = read_u32(bytes, offset, "vertex count")?;
    offset += 4;
    let remaining = bytes.len().saturating_sub(offset);
    let mut vertices = Vec::with_capacity(safe_capacity(vertex_count, remaining, 12));
    let vertex_count = vertex_count as usize;
    for _ in 0..vertex_count {
        let x = read_f32(bytes, offset, "vertex x")?;
        let y = read_f32(bytes, offset + 4, "vertex y")?;
        let z = read_f32(bytes, offset + 8, "vertex z")?;
        vertices.push(NvnmVertex { x, y, z });
        offset += 12;
    }

    let triangle_count = read_u32(bytes, offset, "triangle count")?;
    offset += 4;
    let remaining = bytes.len().saturating_sub(offset);
    let mut triangles =
        Vec::with_capacity(safe_capacity(triangle_count, remaining, TRIANGLE_ROW_SIZE));
    let triangle_count = triangle_count as usize;
    for _ in 0..triangle_count {
        need(bytes, offset, TRIANGLE_ROW_SIZE, "triangle row")?;
        let v0 = read_u16(bytes, offset, "triangle v0")?;
        let v1 = read_u16(bytes, offset + 2, "triangle v1")?;
        let v2 = read_u16(bytes, offset + 4, "triangle v2")?;
        let l0 = read_i16(bytes, offset + 6, "triangle l0")?;
        let l1 = read_i16(bytes, offset + 8, "triangle l1")?;
        let l2 = read_i16(bytes, offset + 10, "triangle l2")?;
        let mut cover_marker = [0u8; 9];
        cover_marker.copy_from_slice(&bytes[offset + 12..offset + 21]);
        // flags lives at row offset 17..19 (= cover_marker[5..7])
        let flags = read_u16(bytes, offset + TRIANGLE_FLAGS_OFFSET, "triangle flags")?;
        triangles.push(NvnmTriangle {
            vertices: [v0, v1, v2],
            links: [l0, l1, l2],
            cover_marker,
            flags,
        });
        offset += TRIANGLE_ROW_SIZE;
    }

    let edge_link_count = read_u32(bytes, offset, "edge_link count")?;
    offset += 4;
    let remaining = bytes.len().saturating_sub(offset);
    let mut edge_links = Vec::with_capacity(safe_capacity(
        edge_link_count,
        remaining,
        EDGE_LINK_ROW_SIZE,
    ));
    let edge_link_count = edge_link_count as usize;
    for _ in 0..edge_link_count {
        need(bytes, offset, EDGE_LINK_ROW_SIZE, "edge_link row")?;
        let mut row = [0u8; EDGE_LINK_ROW_SIZE];
        row.copy_from_slice(&bytes[offset..offset + EDGE_LINK_ROW_SIZE]);
        edge_links.push(NvnmEdgeLink { row });
        offset += EDGE_LINK_ROW_SIZE;
    }

    let door_ref_count = read_u32(bytes, offset, "door_ref count")?;
    offset += 4;
    let remaining = bytes.len().saturating_sub(offset);
    let mut door_refs =
        Vec::with_capacity(safe_capacity(door_ref_count, remaining, DOOR_REF_ROW_SIZE));
    let door_ref_count = door_ref_count as usize;
    for _ in 0..door_ref_count {
        need(bytes, offset, DOOR_REF_ROW_SIZE, "door_ref row")?;
        let triangle_index = read_i16(bytes, offset, "door_ref triangle_index")?;
        let mut padding = [0u8; 4];
        padding.copy_from_slice(&bytes[offset + 2..offset + 6]);
        let door_ref_form_id = read_u32(bytes, offset + 6, "door_ref form_id")?;
        door_refs.push(NvnmDoorRef {
            triangle_index,
            padding,
            door_ref_form_id,
        });
        offset += DOOR_REF_ROW_SIZE;
    }

    let cover_count = read_u32(bytes, offset, "cover_array count")?;
    offset += 4;
    let remaining = bytes.len().saturating_sub(offset);
    let mut cover_array = Vec::with_capacity(safe_capacity(cover_count, remaining, 8));
    let cover_count = cover_count as usize;
    for _ in 0..cover_count {
        need(bytes, offset, 8, "cover_array row")?;
        let vertex_1 = read_u16(bytes, offset, "cover vertex_1")?;
        let vertex_2 = read_u16(bytes, offset + 2, "cover vertex_2")?;
        let data_byte_1 = bytes[offset + 4];
        let data_byte_2 = bytes[offset + 5];
        let data_byte_3 = bytes[offset + 6];
        let data_byte_4 = bytes[offset + 7];
        cover_array.push(NvnmCoverEntry {
            vertex_1,
            vertex_2,
            data_byte_1,
            data_byte_2,
            data_byte_3,
            data_byte_4,
        });
        offset += 8;
    }

    let mapping_count = read_u32(bytes, offset, "cover_triangle_mappings count")?;
    offset += 4;
    let remaining = bytes.len().saturating_sub(offset);
    let mut cover_triangle_mappings =
        Vec::with_capacity(safe_capacity(mapping_count, remaining, 4));
    let mapping_count = mapping_count as usize;
    for _ in 0..mapping_count {
        need(bytes, offset, 4, "cover_triangle_mappings row")?;
        let cover = read_u16(bytes, offset, "cover_triangle_mapping cover")?;
        let triangle = read_i16(bytes, offset + 2, "cover_triangle_mapping triangle")?;
        cover_triangle_mappings.push(NvnmCoverTriangleMapping { cover, triangle });
        offset += 4;
    }

    let waypoint_count = read_u32(bytes, offset, "waypoints count")?;
    offset += 4;
    let remaining = bytes.len().saturating_sub(offset);
    let mut waypoints = Vec::with_capacity(safe_capacity(waypoint_count, remaining, 18));
    let waypoint_count = waypoint_count as usize;
    for _ in 0..waypoint_count {
        need(bytes, offset, 18, "waypoint row")?;
        let x = read_f32(bytes, offset, "waypoint x")?;
        let y = read_f32(bytes, offset + 4, "waypoint y")?;
        let z = read_f32(bytes, offset + 8, "waypoint z")?;
        let triangle = read_i16(bytes, offset + 12, "waypoint triangle")?;
        let flags = read_u32(bytes, offset + 14, "waypoint flags")?;
        waypoints.push(NvnmWaypoint {
            x,
            y,
            z,
            triangle,
            flags,
        });
        offset += 18;
    }

    let divisor = read_u32(bytes, offset, "navmesh_grid divisor")?;
    offset += 4;
    let grid = if divisor == 0 {
        NvnmGrid::default()
    } else {
        let grid_size_x = read_f32(bytes, offset, "grid_size_x")?;
        let grid_size_y = read_f32(bytes, offset + 4, "grid_size_y")?;
        let bounds_min_x = read_f32(bytes, offset + 8, "bounds_min_x")?;
        let bounds_min_y = read_f32(bytes, offset + 12, "bounds_min_y")?;
        let bounds_min_z = read_f32(bytes, offset + 16, "bounds_min_z")?;
        let bounds_max_x = read_f32(bytes, offset + 20, "bounds_max_x")?;
        let bounds_max_y = read_f32(bytes, offset + 24, "bounds_max_y")?;
        let bounds_max_z = read_f32(bytes, offset + 28, "bounds_max_z")?;
        offset += 32;
        let cell_total = (divisor as usize)
            .checked_mul(divisor as usize)
            .ok_or_else(|| {
                NvnmError::Other(format!("navmesh_grid divisor² overflow ({divisor})"))
            })?;
        // Vec::with_capacity is attacker-bounded: each cell needs at minimum
        // a 4-byte entry_count, so the legit cell_total is capped by
        // remaining_bytes / 4. The actual loop count stays at cell_total —
        // a corrupt large divisor still errors via `need(...)` on the first
        // read past the buffer.
        let cells_cap = cell_total.min(bytes.len().saturating_sub(offset) / 4);
        let mut cells = Vec::with_capacity(cells_cap);
        for _ in 0..cell_total {
            let entry_count = read_u32(bytes, offset, "navmesh_grid cell entry_count")?;
            offset += 4;
            let remaining = bytes.len().saturating_sub(offset);
            let mut triangle_indices = Vec::with_capacity(safe_capacity(entry_count, remaining, 2));
            let entry_count = entry_count as usize;
            for _ in 0..entry_count {
                let v = read_i16(bytes, offset, "navmesh_grid cell triangle_index")?;
                triangle_indices.push(v);
                offset += 2;
            }
            cells.push(NvnmGridCell { triangle_indices });
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

    if offset != bytes.len() {
        return Err(NvnmError::Other(format!(
            "NVNM has {} trailing bytes after structured parse",
            bytes.len() - offset
        )));
    }

    Ok(NvnmPayload {
        version,
        flags,
        parent,
        vertices,
        triangles,
        edge_links,
        door_refs,
        cover_array,
        cover_triangle_mappings,
        waypoints,
        grid,
    })
}
