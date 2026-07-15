// Tests for esp_authoring_core::nvnm
use super::*;

macro_rules! corpus_roundtrip_test {
    ($name:ident, $path:literal) => {
        #[test]
        fn $name() {
            let bytes = include_bytes!($path);
            let parsed = parse_nvnm(bytes).expect("parse");
            let rewritten = write_nvnm(&parsed);
            assert_eq!(
                rewritten.as_slice(),
                bytes.as_slice(),
                "byte-roundtrip mismatch"
            );
        }
    };
}

corpus_roundtrip_test!(nvnm_corpus_roundtrip_4ea534, "fixtures/4ea534_fo4.nvnm.bin");
corpus_roundtrip_test!(nvnm_corpus_roundtrip_2b7406, "fixtures/2b7406_fo4.nvnm.bin");
corpus_roundtrip_test!(nvnm_corpus_roundtrip_2b7405, "fixtures/2b7405_fo4.nvnm.bin");
corpus_roundtrip_test!(nvnm_corpus_roundtrip_0a159e, "fixtures/0a159e_fo4.nvnm.bin");
corpus_roundtrip_test!(nvnm_corpus_roundtrip_2b7404, "fixtures/2b7404_fo4.nvnm.bin");
corpus_roundtrip_test!(nvnm_corpus_roundtrip_404836, "fixtures/404836_fo4.nvnm.bin");
corpus_roundtrip_test!(nvnm_corpus_roundtrip_4ea53d, "fixtures/4ea53d_fo4.nvnm.bin");
corpus_roundtrip_test!(nvnm_corpus_roundtrip_024555, "fixtures/024555_fo4.nvnm.bin");
corpus_roundtrip_test!(nvnm_corpus_roundtrip_0818bc, "fixtures/0818bc_fo4.nvnm.bin");

#[test]
fn nvnm_cover_array_structured_4ea534() {
    // cover_array + cover_triangle_mappings decoded from a
    // real corpus fixture. Expected values come from
    // tmp/nvnm_investigation/validate.py + nvnm_phase2_fixture_dump.py.
    let bytes = include_bytes!("fixtures/4ea534_fo4.nvnm.bin");
    let parsed = parse_nvnm(bytes).expect("parse");
    assert_eq!(parsed.cover_array.len(), 135, "cover_array count");
    let c0 = parsed.cover_array[0];
    assert_eq!(c0.vertex_1, 120);
    assert_eq!(c0.vertex_2, 117);
    assert_eq!(c0.data_byte_1, 16);
    assert_eq!(c0.data_byte_2, 8);
    assert_eq!(c0.data_byte_3, 0);
    assert_eq!(c0.data_byte_4, 128);
    assert_eq!(parsed.cover_triangle_mappings.len(), 83, "mapping count");
    let m0 = parsed.cover_triangle_mappings[0];
    assert_eq!(m0.cover, 290);
    assert_eq!(m0.triangle, 108);
}

#[test]
fn nvnm_navmesh_grid_structured_4ea534() {
    // navmesh_grid decoded from a real corpus fixture.
    // Expected values come from validate.py + nvnm_phase2_fixture_dump.py.
    let bytes = include_bytes!("fixtures/4ea534_fo4.nvnm.bin");
    let parsed = parse_nvnm(bytes).expect("parse");
    let g = &parsed.grid;
    assert_eq!(g.divisor, 7);
    assert_eq!(g.grid_size_x, 225.1428680419922);
    assert_eq!(g.grid_size_y, 460.5714416503906);
    assert_eq!(g.bounds_min_x, 2520.0);
    assert_eq!(g.bounds_min_y, 0.0);
    assert_eq!(g.bounds_min_z, 22014.076171875);
    assert_eq!(g.bounds_max_x, 4096.0);
    assert_eq!(g.bounds_max_y, 3224.0);
    assert_eq!(g.cells.len(), 49, "divisor² cells");
    let total_entries: usize = g.cells.iter().map(|c| c.triangle_indices.len()).sum();
    assert_eq!(total_entries, 575);
    // First cell from dump: count=11, entries[0..10]=[7,206,207,208,209,210,211,212,213,214].
    assert_eq!(g.cells[0].triangle_indices.len(), 11);
    assert_eq!(
        &g.cells[0].triangle_indices[..10],
        &[7, 206, 207, 208, 209, 210, 211, 212, 213, 214][..]
    );
}

#[test]
fn nvnm_navmesh_grid_structured_0818bc() {
    // large fixture (divisor=12, 144 cells, 4543 entries).
    let bytes = include_bytes!("fixtures/0818bc_fo4.nvnm.bin");
    let parsed = parse_nvnm(bytes).expect("parse");
    let g = &parsed.grid;
    assert_eq!(g.divisor, 12);
    assert_eq!(g.cells.len(), 144);
    let total_entries: usize = g.cells.iter().map(|c| c.triangle_indices.len()).sum();
    assert_eq!(total_entries, 4543);
    assert_eq!(g.bounds_min_x, -1572.3311767578125);
    assert_eq!(g.bounds_max_z, 1087.44140625);
}

#[test]
fn nvnm_waypoints_structured_024555() {
    // waypoints decoded from a corpus fixture that has
    // non-zero waypoint count. Expected values come from validate.py +
    // nvnm_phase2_fixture_dump.py.
    let bytes = include_bytes!("fixtures/024555_fo4.nvnm.bin");
    let parsed = parse_nvnm(bytes).expect("parse");
    assert_eq!(parsed.waypoints.len(), 2, "waypoint count");
    let w0 = parsed.waypoints[0];
    assert_eq!(w0.x, 2068.89306640625);
    assert_eq!(w0.y, 2022.3629150390625);
    assert_eq!(w0.z, 255.99998474121094);
    assert_eq!(w0.triangle, 0);
    assert_eq!(w0.flags, 0);
}

fn make_minimal_nvnm(parent: NvnmParent, verts: &[(f32, f32, f32)]) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&15u32.to_le_bytes());
    v.extend_from_slice(&0u32.to_le_bytes());
    match parent {
        NvnmParent::Interior { cell } => {
            v.extend_from_slice(&0u32.to_le_bytes());
            v.extend_from_slice(&cell.to_le_bytes());
        }
        NvnmParent::Exterior {
            world,
            grid_x,
            grid_y,
        } => {
            v.extend_from_slice(&world.to_le_bytes());
            v.extend_from_slice(&grid_y.to_le_bytes());
            v.extend_from_slice(&grid_x.to_le_bytes());
        }
    }
    v.extend_from_slice(&(verts.len() as u32).to_le_bytes());
    for &(x, y, z) in verts {
        v.extend_from_slice(&x.to_le_bytes());
        v.extend_from_slice(&y.to_le_bytes());
        v.extend_from_slice(&z.to_le_bytes());
    }
    v.extend_from_slice(&0u32.to_le_bytes()); // triangle count
    v.extend_from_slice(&0u32.to_le_bytes()); // edge_links count
    v.extend_from_slice(&0u32.to_le_bytes()); // door_refs count
    v.extend_from_slice(&0u32.to_le_bytes()); // cover_array count
    v.extend_from_slice(&0u32.to_le_bytes()); // cover_triangle_mappings count
    v.extend_from_slice(&0u32.to_le_bytes()); // waypoints count
    v.extend_from_slice(&0u32.to_le_bytes()); // navmesh_grid divisor (0 = no grid)
    v
}

#[test]
fn nvnm_header_and_vertices_roundtrip_exterior() {
    let bytes = make_minimal_nvnm(
        NvnmParent::Exterior {
            world: 0x0025DA15,
            grid_x: 0,
            grid_y: 1,
        },
        &[(1.0, 2.0, 3.0), (4.0, 5.0, 6.0)],
    );
    let parsed = parse_nvnm(&bytes).expect("parse");
    assert_eq!(
        parsed.parent,
        NvnmParent::Exterior {
            world: 0x0025DA15,
            grid_x: 0,
            grid_y: 1
        }
    );
    assert_eq!(parsed.vertices.len(), 2);
    assert_eq!(
        parsed.vertices[0],
        NvnmVertex {
            x: 1.0,
            y: 2.0,
            z: 3.0
        }
    );
    let rewritten = write_nvnm(&parsed);
    assert_eq!(rewritten, bytes);
}

#[test]
fn nvnm_header_interior_parent() {
    let bytes = make_minimal_nvnm(NvnmParent::Interior { cell: 0x12345 }, &[]);
    let parsed = parse_nvnm(&bytes).expect("parse");
    assert_eq!(parsed.parent, NvnmParent::Interior { cell: 0x12345 });
    assert_eq!(parsed.vertices.len(), 0);
}

fn make_nvnm_with_triangles(
    parent: NvnmParent,
    verts: &[(f32, f32, f32)],
    triangle_rows: &[[u8; 21]],
) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&15u32.to_le_bytes());
    v.extend_from_slice(&0u32.to_le_bytes());
    match parent {
        NvnmParent::Interior { cell } => {
            v.extend_from_slice(&0u32.to_le_bytes());
            v.extend_from_slice(&cell.to_le_bytes());
        }
        NvnmParent::Exterior {
            world,
            grid_x,
            grid_y,
        } => {
            v.extend_from_slice(&world.to_le_bytes());
            v.extend_from_slice(&grid_y.to_le_bytes());
            v.extend_from_slice(&grid_x.to_le_bytes());
        }
    }
    v.extend_from_slice(&(verts.len() as u32).to_le_bytes());
    for &(x, y, z) in verts {
        v.extend_from_slice(&x.to_le_bytes());
        v.extend_from_slice(&y.to_le_bytes());
        v.extend_from_slice(&z.to_le_bytes());
    }
    v.extend_from_slice(&(triangle_rows.len() as u32).to_le_bytes());
    for row in triangle_rows {
        v.extend_from_slice(row);
    }
    v.extend_from_slice(&0u32.to_le_bytes()); // edge_links count
    v.extend_from_slice(&0u32.to_le_bytes()); // door_refs count
    v.extend_from_slice(&0u32.to_le_bytes()); // cover_array count
    v.extend_from_slice(&0u32.to_le_bytes()); // cover_triangle_mappings count
    v.extend_from_slice(&0u32.to_le_bytes()); // waypoints count
    v.extend_from_slice(&0u32.to_le_bytes()); // navmesh_grid divisor (0 = no grid)
    v
}

#[test]
fn nvnm_triangles_roundtrip() {
    // 21-byte triangle row: v0,v1,v2 (u16), l0,l1,l2 (i16), 9-byte cover_marker
    // (flags occupies cover_marker[5..7] = row offset 17..19).
    let mut row = [0u8; 21];
    // vertices
    row[0..2].copy_from_slice(&0u16.to_le_bytes());
    row[2..4].copy_from_slice(&1u16.to_le_bytes());
    row[4..6].copy_from_slice(&2u16.to_le_bytes());
    // links
    row[6..8].copy_from_slice(&(-1i16).to_le_bytes());
    row[8..10].copy_from_slice(&5i16.to_le_bytes());
    row[10..12].copy_from_slice(&(-2i16).to_le_bytes());
    // cover_marker bytes (offsets 12..21)
    row[12] = 0xAA;
    row[13] = 0xBB;
    row[14] = 0xCC;
    row[15] = 0xDD;
    row[16] = 0xEE;
    // flags = 0x1234 at offset 17..19
    row[17] = 0x34;
    row[18] = 0x12;
    row[19] = 0xFA;
    row[20] = 0xCE;

    let bytes = make_nvnm_with_triangles(
        NvnmParent::Exterior {
            world: 0x0125DA15,
            grid_x: 0,
            grid_y: 0,
        },
        &[(0.0, 0.0, 0.0), (1.0, 0.0, 0.0), (0.0, 1.0, 0.0)],
        &[row],
    );
    let parsed = parse_nvnm(&bytes).expect("parse");
    assert_eq!(parsed.triangles.len(), 1);
    let t = &parsed.triangles[0];
    assert_eq!(t.vertices, [0, 1, 2]);
    assert_eq!(t.links, [-1, 5, -2]);
    assert_eq!(
        t.cover_marker,
        [0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0x34, 0x12, 0xFA, 0xCE]
    );
    assert_eq!(t.flags, 0x1234);
    let rewritten = write_nvnm(&parsed);
    assert_eq!(rewritten, bytes);
}

fn make_nvnm_with_edge_links(
    parent: NvnmParent,
    verts: &[(f32, f32, f32)],
    triangle_rows: &[[u8; 21]],
    edge_link_rows: &[[u8; 11]],
) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&15u32.to_le_bytes());
    v.extend_from_slice(&0u32.to_le_bytes());
    match parent {
        NvnmParent::Interior { cell } => {
            v.extend_from_slice(&0u32.to_le_bytes());
            v.extend_from_slice(&cell.to_le_bytes());
        }
        NvnmParent::Exterior {
            world,
            grid_x,
            grid_y,
        } => {
            v.extend_from_slice(&world.to_le_bytes());
            v.extend_from_slice(&grid_y.to_le_bytes());
            v.extend_from_slice(&grid_x.to_le_bytes());
        }
    }
    v.extend_from_slice(&(verts.len() as u32).to_le_bytes());
    for &(x, y, z) in verts {
        v.extend_from_slice(&x.to_le_bytes());
        v.extend_from_slice(&y.to_le_bytes());
        v.extend_from_slice(&z.to_le_bytes());
    }
    v.extend_from_slice(&(triangle_rows.len() as u32).to_le_bytes());
    for row in triangle_rows {
        v.extend_from_slice(row);
    }
    v.extend_from_slice(&(edge_link_rows.len() as u32).to_le_bytes());
    for row in edge_link_rows {
        v.extend_from_slice(row);
    }
    v.extend_from_slice(&0u32.to_le_bytes()); // door_refs count
    v.extend_from_slice(&0u32.to_le_bytes()); // cover_array count
    v.extend_from_slice(&0u32.to_le_bytes()); // cover_triangle_mappings count
    v.extend_from_slice(&0u32.to_le_bytes()); // waypoints count
    v.extend_from_slice(&0u32.to_le_bytes()); // navmesh_grid divisor (0 = no grid)
    v
}

fn make_nvnm_with_door_refs(
    parent: NvnmParent,
    verts: &[(f32, f32, f32)],
    triangle_rows: &[[u8; 21]],
    edge_link_rows: &[[u8; 11]],
    door_refs: &[(i16, [u8; 4], u32)],
) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&15u32.to_le_bytes());
    v.extend_from_slice(&0u32.to_le_bytes());
    match parent {
        NvnmParent::Interior { cell } => {
            v.extend_from_slice(&0u32.to_le_bytes());
            v.extend_from_slice(&cell.to_le_bytes());
        }
        NvnmParent::Exterior {
            world,
            grid_x,
            grid_y,
        } => {
            v.extend_from_slice(&world.to_le_bytes());
            v.extend_from_slice(&grid_y.to_le_bytes());
            v.extend_from_slice(&grid_x.to_le_bytes());
        }
    }
    v.extend_from_slice(&(verts.len() as u32).to_le_bytes());
    for &(x, y, z) in verts {
        v.extend_from_slice(&x.to_le_bytes());
        v.extend_from_slice(&y.to_le_bytes());
        v.extend_from_slice(&z.to_le_bytes());
    }
    v.extend_from_slice(&(triangle_rows.len() as u32).to_le_bytes());
    for row in triangle_rows {
        v.extend_from_slice(row);
    }
    v.extend_from_slice(&(edge_link_rows.len() as u32).to_le_bytes());
    for row in edge_link_rows {
        v.extend_from_slice(row);
    }
    v.extend_from_slice(&(door_refs.len() as u32).to_le_bytes());
    for &(triangle_index, padding, form_id) in door_refs {
        v.extend_from_slice(&triangle_index.to_le_bytes());
        v.extend_from_slice(&padding);
        v.extend_from_slice(&form_id.to_le_bytes());
    }
    v.extend_from_slice(&0u32.to_le_bytes()); // cover_array count
    v.extend_from_slice(&0u32.to_le_bytes()); // cover_triangle_mappings count
    v.extend_from_slice(&0u32.to_le_bytes()); // waypoints count
    v.extend_from_slice(&0u32.to_le_bytes()); // navmesh_grid divisor (0 = no grid)
    v
}

#[test]
fn nvnm_door_refs_roundtrip() {
    let bytes = make_nvnm_with_door_refs(
        NvnmParent::Interior { cell: 0x99 },
        &[],
        &[],
        &[],
        &[(0x0010, [0x01, 0x02, 0x03, 0x04], 0x01ABCDEF)],
    );
    let parsed = parse_nvnm(&bytes).expect("parse");
    assert_eq!(parsed.door_refs.len(), 1);
    let d = &parsed.door_refs[0];
    assert_eq!(d.triangle_index, 0x0010);
    assert_eq!(d.padding, [0x01, 0x02, 0x03, 0x04]);
    assert_eq!(d.door_ref_form_id, 0x01ABCDEF);
    let rewritten = write_nvnm(&parsed);
    assert_eq!(rewritten, bytes);
}

#[test]
fn nvnm_edge_links_roundtrip() {
    let row_a: [u8; 11] = [
        0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B,
    ];
    let row_b: [u8; 11] = [
        0xF0, 0xE1, 0xD2, 0xC3, 0xB4, 0xA5, 0x96, 0x87, 0x78, 0x69, 0x5A,
    ];
    let bytes = make_nvnm_with_edge_links(
        NvnmParent::Interior { cell: 0x42 },
        &[],
        &[],
        &[row_a, row_b],
    );
    let parsed = parse_nvnm(&bytes).expect("parse");
    assert_eq!(parsed.edge_links.len(), 2);
    assert_eq!(parsed.edge_links[0].row, row_a);
    assert_eq!(parsed.edge_links[1].row, row_b);
    let rewritten = write_nvnm(&parsed);
    assert_eq!(rewritten, bytes);
}

// ---------------------------------------------------------------------------
// YAML codec roundtrip
// ---------------------------------------------------------------------------

#[test]
fn nvnm_yaml_roundtrip_minimal() {
    let payload = NvnmPayload {
        version: 15,
        flags: 0,
        parent: NvnmParent::Interior { cell: 0x12345 },
        vertices: vec![NvnmVertex {
            x: 1.0,
            y: 2.0,
            z: 3.0,
        }],
        triangles: vec![],
        edge_links: vec![],
        door_refs: vec![],
        cover_array: vec![],
        cover_triangle_mappings: vec![],
        waypoints: vec![],
        grid: NvnmGrid::default(),
    };
    let yaml = nvnm_to_yaml(&payload);
    let parsed_back = nvnm_from_yaml(&yaml).expect("from_yaml");
    assert_eq!(parsed_back, payload);
}

macro_rules! nvnm_yaml_corpus_test {
    ($name:ident, $path:literal) => {
        #[test]
        fn $name() {
            let bytes = include_bytes!($path);
            let parsed = parse_nvnm(bytes).expect("parse");
            let yaml = nvnm_to_yaml(&parsed);
            let from_yaml = nvnm_from_yaml(&yaml).expect("from_yaml");
            assert_eq!(from_yaml, parsed, "yaml roundtrip mismatch");
            let rewritten = write_nvnm(&from_yaml);
            assert_eq!(
                rewritten.as_slice(),
                bytes.as_slice(),
                "byte-roundtrip mismatch after yaml roundtrip"
            );
        }
    };
}

nvnm_yaml_corpus_test!(nvnm_yaml_roundtrip_4ea534, "fixtures/4ea534_fo4.nvnm.bin");
nvnm_yaml_corpus_test!(nvnm_yaml_roundtrip_2b7406, "fixtures/2b7406_fo4.nvnm.bin");
nvnm_yaml_corpus_test!(nvnm_yaml_roundtrip_2b7405, "fixtures/2b7405_fo4.nvnm.bin");
nvnm_yaml_corpus_test!(nvnm_yaml_roundtrip_0a159e, "fixtures/0a159e_fo4.nvnm.bin");
nvnm_yaml_corpus_test!(nvnm_yaml_roundtrip_2b7404, "fixtures/2b7404_fo4.nvnm.bin");
nvnm_yaml_corpus_test!(nvnm_yaml_roundtrip_404836, "fixtures/404836_fo4.nvnm.bin");
nvnm_yaml_corpus_test!(nvnm_yaml_roundtrip_4ea53d, "fixtures/4ea53d_fo4.nvnm.bin");
nvnm_yaml_corpus_test!(nvnm_yaml_roundtrip_024555, "fixtures/024555_fo4.nvnm.bin");
nvnm_yaml_corpus_test!(nvnm_yaml_roundtrip_0818bc, "fixtures/0818bc_fo4.nvnm.bin");

// ---------------------------------------------------------------------------
// Vec::with_capacity DoS guard — attacker-controlled u32 counts must not
// trigger huge allocations before the truncation check fires.
// ---------------------------------------------------------------------------

#[test]
fn nvnm_huge_vertex_count_is_truncation_error_not_panic() {
    // Build a header with vertex_count = u32::MAX. The bytes after the header
    // can't satisfy a single vertex, so parse must return Truncated, NOT panic
    // attempting to allocate u32::MAX * 12 bytes.
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&15u32.to_le_bytes()); // version
    bytes.extend_from_slice(&0u32.to_le_bytes()); // flags
    bytes.extend_from_slice(&0u32.to_le_bytes()); // parent_world = 0 → interior
    bytes.extend_from_slice(&0x12345u32.to_le_bytes()); // cell
    bytes.extend_from_slice(&u32::MAX.to_le_bytes()); // vertex count
    // No vertex bytes follow.
    let result = parse_nvnm(&bytes);
    assert!(matches!(result, Err(NvnmError::Truncated { .. })));
}

#[test]
fn nvnm_triangle_flags_yaml_edit_lands_in_bytes() {
    // Regression: parser exposes triangle.flags as a view of
    // cover_marker[5..7]. If a YAML editor changes `flags`, write_nvnm must
    // sync the new value back into cover_marker before emit — otherwise the
    // edit is silently overwritten with the stale cover_marker bytes.
    let bytes = include_bytes!("fixtures/4ea534_fo4.nvnm.bin");
    let mut parsed = parse_nvnm(bytes).expect("parse");
    assert!(!parsed.triangles.is_empty());
    let new_flags = !parsed.triangles[0].flags;
    parsed.triangles[0].flags = new_flags;
    // Note: cover_marker[5..7] still holds the OLD flags here — the writer
    // must use t.flags as the source of truth, not t.cover_marker[5..7].
    let rewritten = write_nvnm(&parsed);
    let reparsed = parse_nvnm(&rewritten).expect("parse rewritten");
    assert_eq!(reparsed.triangles[0].flags, new_flags);
    assert_eq!(
        &reparsed.triangles[0].cover_marker[5..7],
        &new_flags.to_le_bytes()
    );
}

#[test]
fn nvnm_huge_triangle_count_is_truncation_error_not_panic() {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&15u32.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&0x12345u32.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes()); // 0 vertices
    bytes.extend_from_slice(&u32::MAX.to_le_bytes()); // triangle count
    let result = parse_nvnm(&bytes);
    assert!(matches!(result, Err(NvnmError::Truncated { .. })));
}
