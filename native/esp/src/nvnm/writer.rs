use super::types::{NvnmParent, NvnmPayload};

pub fn write_nvnm(payload: &NvnmPayload) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&payload.version.to_le_bytes());
    out.extend_from_slice(&payload.flags.to_le_bytes());
    match payload.parent {
        NvnmParent::Interior { cell } => {
            out.extend_from_slice(&0u32.to_le_bytes());
            out.extend_from_slice(&cell.to_le_bytes());
        }
        NvnmParent::Exterior {
            world,
            grid_x,
            grid_y,
        } => {
            out.extend_from_slice(&world.to_le_bytes());
            out.extend_from_slice(&grid_y.to_le_bytes());
            out.extend_from_slice(&grid_x.to_le_bytes());
        }
    }
    out.extend_from_slice(&(payload.vertices.len() as u32).to_le_bytes());
    for v in &payload.vertices {
        out.extend_from_slice(&v.x.to_le_bytes());
        out.extend_from_slice(&v.y.to_le_bytes());
        out.extend_from_slice(&v.z.to_le_bytes());
    }
    out.extend_from_slice(&(payload.triangles.len() as u32).to_le_bytes());
    for t in &payload.triangles {
        out.extend_from_slice(&t.vertices[0].to_le_bytes());
        out.extend_from_slice(&t.vertices[1].to_le_bytes());
        out.extend_from_slice(&t.vertices[2].to_le_bytes());
        out.extend_from_slice(&t.links[0].to_le_bytes());
        out.extend_from_slice(&t.links[1].to_le_bytes());
        out.extend_from_slice(&t.links[2].to_le_bytes());
        // cover_marker covers all 9 trailing bytes of the 21-byte row,
        // including the flags u16 at offsets [5..7]. The parser populates
        // `flags` as a view of cover_marker[5..7]; sync that view back into
        // cover_marker before emit so a YAML edit to `flags` actually lands
        // in the bytes (would otherwise be silently overwritten by the
        // stale cover_marker[5..7]).
        let mut cover_marker = t.cover_marker;
        cover_marker[5..7].copy_from_slice(&t.flags.to_le_bytes());
        out.extend_from_slice(&cover_marker);
    }
    out.extend_from_slice(&(payload.edge_links.len() as u32).to_le_bytes());
    for el in &payload.edge_links {
        out.extend_from_slice(&el.row);
    }
    out.extend_from_slice(&(payload.door_refs.len() as u32).to_le_bytes());
    for d in &payload.door_refs {
        out.extend_from_slice(&d.triangle_index.to_le_bytes());
        out.extend_from_slice(&d.padding);
        out.extend_from_slice(&d.door_ref_form_id.to_le_bytes());
    }
    out.extend_from_slice(&(payload.cover_array.len() as u32).to_le_bytes());
    for c in &payload.cover_array {
        out.extend_from_slice(&c.vertex_1.to_le_bytes());
        out.extend_from_slice(&c.vertex_2.to_le_bytes());
        out.push(c.data_byte_1);
        out.push(c.data_byte_2);
        out.push(c.data_byte_3);
        out.push(c.data_byte_4);
    }
    out.extend_from_slice(&(payload.cover_triangle_mappings.len() as u32).to_le_bytes());
    for m in &payload.cover_triangle_mappings {
        out.extend_from_slice(&m.cover.to_le_bytes());
        out.extend_from_slice(&m.triangle.to_le_bytes());
    }
    out.extend_from_slice(&(payload.waypoints.len() as u32).to_le_bytes());
    for w in &payload.waypoints {
        out.extend_from_slice(&w.x.to_le_bytes());
        out.extend_from_slice(&w.y.to_le_bytes());
        out.extend_from_slice(&w.z.to_le_bytes());
        out.extend_from_slice(&w.triangle.to_le_bytes());
        out.extend_from_slice(&w.flags.to_le_bytes());
    }
    out.extend_from_slice(&payload.grid.divisor.to_le_bytes());
    if payload.grid.divisor > 0 {
        out.extend_from_slice(&payload.grid.grid_size_x.to_le_bytes());
        out.extend_from_slice(&payload.grid.grid_size_y.to_le_bytes());
        out.extend_from_slice(&payload.grid.bounds_min_x.to_le_bytes());
        out.extend_from_slice(&payload.grid.bounds_min_y.to_le_bytes());
        out.extend_from_slice(&payload.grid.bounds_min_z.to_le_bytes());
        out.extend_from_slice(&payload.grid.bounds_max_x.to_le_bytes());
        out.extend_from_slice(&payload.grid.bounds_max_y.to_le_bytes());
        out.extend_from_slice(&payload.grid.bounds_max_z.to_le_bytes());
        for cell in &payload.grid.cells {
            out.extend_from_slice(&(cell.triangle_indices.len() as u32).to_le_bytes());
            for idx in &cell.triangle_indices {
                out.extend_from_slice(&idx.to_le_bytes());
            }
        }
    }
    out
}
