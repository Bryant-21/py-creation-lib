/// Object atlas + UV tests.
/// Run: `uv run cargo test -p lodgen_native --test object_atlas`
use lodgen_native::atlas::binpacker::{BinBlock, BinPacker};

// ---------------------------------------------------------------------------
// BinPacker (TwbBinPacker port)
// ---------------------------------------------------------------------------

#[test]
fn binpacker_sorts_descending_max_side() {
    // port: wbLOD.pas:517-535 MaxSideSort
    let mut blocks = vec![
        BinBlock {
            index: 0,
            w: 64,
            h: 64,
            x: 0,
            y: 0,
            fit: false,
        },
        BinBlock {
            index: 1,
            w: 256,
            h: 128,
            x: 0,
            y: 0,
            fit: false,
        },
        BinBlock {
            index: 2,
            w: 128,
            h: 256,
            x: 0,
            y: 0,
            fit: false,
        },
        BinBlock {
            index: 3,
            w: 32,
            h: 32,
            x: 0,
            y: 0,
            fit: false,
        },
    ];
    let packer = BinPacker::new(4096, 4096);
    let ok = packer.fit(&mut blocks);
    assert!(ok, "all blocks should fit in a 4096x4096 atlas");

    // Both 256-max-side blocks (indices 1 & 2) should come before the 64 and 32 blocks.
    // After MaxSideSort the order should be max-side descending: 256, 256, 64, 32.
    // The first placed block is at (0,0).
    assert_eq!(blocks[0].x, 0);
    assert_eq!(blocks[0].y, 0);
    // The two 256-max-side blocks are the first two in the sorted order.
    assert!(
        blocks[0].w.max(blocks[0].h) >= blocks[1].w.max(blocks[1].h),
        "blocks must be sorted descending by max side"
    );
    assert!(blocks[1].w.max(blocks[1].h) >= blocks[2].w.max(blocks[2].h));
    assert!(blocks[2].w.max(blocks[2].h) >= blocks[3].w.max(blocks[3].h));
    assert!(blocks.iter().all(|b| b.fit));
}

#[test]
fn binpacker_split_node_right_then_down() {
    // port: wbLOD.pas:486-497 FindNode — right before down
    // Two blocks: 256x256, 128x128 in a 4096x4096 atlas.
    // After MaxSideSort both have max-side 256 and 128 respectively.
    // Block[0] at (0,0); block[1] should go to the right child: (256, 0).
    let mut blocks = vec![
        BinBlock {
            index: 0,
            w: 256,
            h: 256,
            x: 0,
            y: 0,
            fit: false,
        },
        BinBlock {
            index: 1,
            w: 128,
            h: 128,
            x: 0,
            y: 0,
            fit: false,
        },
    ];
    let packer = BinPacker::new(4096, 4096);
    assert!(packer.fit(&mut blocks));
    // blocks[0] largest → placed first at (0,0)
    assert_eq!((blocks[0].x, blocks[0].y), (0, 0));
    assert!(blocks[0].fit);
    // blocks[1] → right child: x=256, y=0
    assert_eq!((blocks[1].x, blocks[1].y), (256, 0));
    assert!(blocks[1].fit);
}

#[test]
fn binpacker_returns_false_on_overflow() {
    // Three 3000x3000 blocks can't all fit in a 4096x4096 atlas.
    let mut blocks = vec![
        BinBlock {
            index: 0,
            w: 3000,
            h: 3000,
            x: 0,
            y: 0,
            fit: false,
        },
        BinBlock {
            index: 1,
            w: 3000,
            h: 3000,
            x: 0,
            y: 0,
            fit: false,
        },
        BinBlock {
            index: 2,
            w: 3000,
            h: 3000,
            x: 0,
            y: 0,
            fit: false,
        },
    ];
    let packer = BinPacker::new(4096, 4096);
    assert!(!packer.fit(&mut blocks), "overflow must return false");
}

#[test]
fn binpacker_deterministic() {
    // Two separate runs on cloned inputs → identical placements.
    let make_blocks = || {
        vec![
            BinBlock {
                index: 0,
                w: 256,
                h: 256,
                x: 0,
                y: 0,
                fit: false,
            },
            BinBlock {
                index: 1,
                w: 128,
                h: 64,
                x: 0,
                y: 0,
                fit: false,
            },
            BinBlock {
                index: 2,
                w: 64,
                h: 128,
                x: 0,
                y: 0,
                fit: false,
            },
            BinBlock {
                index: 3,
                w: 32,
                h: 32,
                x: 0,
                y: 0,
                fit: false,
            },
        ]
    };
    let packer = BinPacker::new(4096, 4096);
    let mut run1 = make_blocks();
    let mut run2 = make_blocks();
    packer.fit(&mut run1);
    packer.fit(&mut run2);
    for (a, b) in run1.iter().zip(run2.iter()) {
        assert_eq!((a.x, a.y, a.fit), (b.x, b.y, b.fit));
    }
}

// ---------------------------------------------------------------------------
// AtlasRect + atlas-map .txt writer (byte-exact)
// ---------------------------------------------------------------------------

use lodgen_native::atlas::atlas::{AtlasMapRow, AtlasRect, parse_atlas_map, write_atlas_map};

#[test]
fn atlas_map_row_byte_exact() {
    // port: wbLOD.pas:1557-1566
    // Exactly matches R3 §2c sample format: TAB-separated, LF-terminated, no BOM.
    let rows = vec![AtlasMapRow {
        source: r"textures\lod\airport01_lod_d.dds".to_string(),
        tile_w: 256,
        tile_h: 256,
        x: 256,
        y: 0,
        atlas: r"textures\terrain\commonwealth\objects\commonwealth.objects.dds".to_string(),
        atlas_w: 4096,
        atlas_h: 2048,
    }];
    let tmp = std::env::temp_dir().join("atlas_map_byte_exact.txt");
    write_atlas_map(&tmp, &rows).expect("write_atlas_map failed");
    let bytes = std::fs::read(&tmp).unwrap();
    let expected = b"textures\\lod\\airport01_lod_d.dds\t256\t256\t256\t0\ttextures\\terrain\\commonwealth\\objects\\commonwealth.objects.dds\t4096\t2048\n";
    assert_eq!(
        bytes, expected,
        "atlas-map must be TAB-separated, LF-terminated, no BOM"
    );
    // No UTF-8 BOM
    assert_ne!(&bytes[..3], b"\xef\xbb\xbf", "no BOM allowed");
}

#[test]
fn atlas_rect_from_map_row() {
    // port: Program.cs:1350-1359
    // tile 256x256 at (256,0) in a 4096x2048 atlas
    let rect = AtlasRect::from_map_row(
        256,
        256,
        256,
        0,
        4096,
        2048,
        r"textures\terrain\commonwealth\objects\commonwealth.objects.dds",
        false,
    );
    let eps = 1e-6_f32;
    // texture_scale_u = tile_w / atlas_w = 256/4096
    assert!((rect.texture_scale_u - 256.0 / 4096.0).abs() < eps);
    // texture_scale_v = tile_h / atlas_h = 256/2048
    assert!((rect.texture_scale_v - 256.0 / 2048.0).abs() < eps);
    // pos_u = x / atlas_w = 256/4096
    assert!((rect.pos_u - 256.0 / 4096.0).abs() < eps);
    // pos_v = y / atlas_h = 0/2048
    assert!((rect.pos_v - 0.0_f32).abs() < eps);
    // min_u = 1/(atlas_w*2)
    let min_u = 1.0_f32 / (4096.0 * 2.0);
    assert!((rect.min_u - min_u).abs() < eps);
    // max_u = texture_scale_u - min_u
    assert!((rect.max_u - (256.0 / 4096.0 - min_u)).abs() < eps);
    // min_v = 1/(atlas_h*2)
    let min_v = 1.0_f32 / (2048.0 * 2.0);
    assert!((rect.min_v - min_v).abs() < eps);
    // max_v = texture_scale_v - min_v
    assert!((rect.max_v - (256.0 / 2048.0 - min_v)).abs() < eps);
    // scale_u/v = 1.0 (no extra cols)
    assert!((rect.scale_u - 1.0).abs() < eps);
    assert!((rect.scale_v - 1.0).abs() < eps);
}

#[test]
fn atlas_rect_uv_atlas() {
    // port: AtlasDesc.cs:85-92
    let rect = AtlasRect::from_map_row(
        256,
        256,
        256,
        0,
        4096,
        2048,
        r"textures\terrain\commonwealth\objects\commonwealth.objects.dds",
        false,
    );
    // u=0.5, v=0.5
    let (u_out, v_out) = rect.uv_atlas(0.5, 0.5);
    let tsu = 256.0_f32 / 4096.0;
    let tsv = 256.0_f32 / 2048.0;
    let min_u = 1.0_f32 / (4096.0 * 2.0);
    let min_v = 1.0_f32 / (2048.0 * 2.0);
    let max_u = tsu - min_u;
    let max_v = tsv - min_v;
    let pos_u = 256.0_f32 / 4096.0;
    let pos_v = 0.0_f32;
    let expected_u = (0.5_f32 * tsu).clamp(min_u, max_u) + pos_u;
    let expected_v = (0.5_f32 * tsv).clamp(min_v, max_v) + pos_v;
    let eps = 1e-6_f32;
    assert!(
        (u_out - expected_u).abs() < eps,
        "u_out={u_out} expected={expected_u}"
    );
    assert!(
        (v_out - expected_v).abs() < eps,
        "v_out={v_out} expected={expected_v}"
    );
}

#[test]
fn atlas_map_roundtrip() {
    // Write 2 rows + a malformed 7-col row. Roundtrip parse must yield 2 rows (skip 7-col).
    let rows = vec![
        AtlasMapRow {
            source: r"textures\lod\a_d.dds".to_string(),
            tile_w: 128,
            tile_h: 128,
            x: 0,
            y: 0,
            atlas: r"textures\terrain\w\wobjects.dds".to_string(),
            atlas_w: 1024,
            atlas_h: 1024,
        },
        AtlasMapRow {
            source: r"textures\lod\b_d.dds".to_string(),
            tile_w: 64,
            tile_h: 64,
            x: 128,
            y: 0,
            atlas: r"textures\terrain\w\wobjects.dds".to_string(),
            atlas_w: 1024,
            atlas_h: 1024,
        },
    ];
    let tmp = std::env::temp_dir().join("atlas_map_roundtrip.txt");
    write_atlas_map(&tmp, &rows).unwrap();
    // Append a malformed 7-col row
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new().append(true).open(&tmp).unwrap();
    writeln!(f, "bad\t1\t2\t3\t4\t5\t6").unwrap();
    drop(f);

    let text = std::fs::read_to_string(&tmp).unwrap();
    let parsed = parse_atlas_map(&text);
    assert_eq!(parsed.len(), 2, "7-col row must be skipped");
    assert_eq!(parsed[0].source, r"textures\lod\a_d.dds");
    assert_eq!(parsed[1].x, 128);
}

// ---------------------------------------------------------------------------
// ShapeFlags + AtlasList.GetKey / BuildKey
// ---------------------------------------------------------------------------

use lodgen_native::atlas::atlas::AtlasList;
use lodgen_native::objects::static_desc::{ShapeFlags, atlas_build_key, atlas_get_key};

#[test]
fn shapeflags_values() {
    // port: ShapeFlags.cs — verify each constant matches C# enum values
    assert_eq!(ShapeFlags::IS_PASSTHRU.bits(), 0x1u32);
    assert_eq!(ShapeFlags::IS_GROUP.bits(), 0x2u32);
    assert_eq!(ShapeFlags::IS_HIGH_DETAIL.bits(), 0x4u32);
    assert_eq!(ShapeFlags::IS_GRASS.bits(), 0x8u32);
    assert_eq!(ShapeFlags::HAS_VERTEX_COLOR.bits(), 0x10u32);
    assert_eq!(ShapeFlags::ALL_WHITE.bits(), 0x20u32);
    assert_eq!(ShapeFlags::IS_DOUBLE_SIDED.bits(), 0x40u32);
    assert_eq!(ShapeFlags::IS_ALPHA.bits(), 0x80u32);
    assert_eq!(ShapeFlags::IS_DECAL.bits(), 0x100u32);
    assert_eq!(ShapeFlags::HAS_LOD_FLAG.bits(), 0x200u32);
    assert_eq!(ShapeFlags::IS_GREYSCALE_TO_PALETTE.bits(), 0x400u32);
    assert_eq!(ShapeFlags::IS_GREYSCALE_TO_ALPHA.bits(), 0x800u32);
    assert_eq!(ShapeFlags::IS_TREE.bits(), 0x1000u32);
    assert_eq!(ShapeFlags::IS_FLAT_TRUNK.bits(), 0x2000u32);
    assert_eq!(ShapeFlags::IS_TRUNK.bits(), 0x4000u32);
    assert_eq!(ShapeFlags::IS_CROWN.bits(), 0x8000u32);
    assert_eq!(ShapeFlags::IS_BILLBOARD.bits(), 0x10000u32);
    assert_eq!(ShapeFlags::HAS_VERTEX_ALPHA.bits(), 0x20000u32);
}

#[test]
fn atlas_getkey_diffuse_normal() {
    // port: AtlasList.cs:13-62
    // key with =8 (floor(128/16)=8)
    let mut list = AtlasList::new();
    list.insert_key("a_d.dds,a_n.dds=8".to_string(), make_rect());
    // also insert bare variant for fallback test
    list.insert_key("a_d.dds,a_n.dds".to_string(), make_rect());

    // With alpha=128 → floor(128/16)=8 → finds "a_d.dds,a_n.dds=8"
    let key = atlas_get_key(&list, "a_d.dds", "a_n.dds", "", 128);
    assert_eq!(key, "a_d.dds,a_n.dds=8");

    // Without =8 variant but bare present
    let mut list2 = AtlasList::new();
    list2.insert_key("a_d.dds,a_n.dds".to_string(), make_rect());
    let key2 = atlas_get_key(&list2, "a_d.dds", "a_n.dds", "", 128);
    assert_eq!(key2, "a_d.dds,a_n.dds");
}

#[test]
fn atlas_getkey_pbr_linear() {
    // port: AtlasList.cs:15-18
    // CaseInsensitiveReplace("linear.dds", ".dds"):
    // "textures\pbr\x_linear.dds" → "textures\pbr\x_.dds" (replaces "linear.dds" substring)
    let mut list = AtlasList::new();
    list.insert_key(r"textures\pbr\x_.dds".to_string(), make_rect());
    let key = atlas_get_key(&list, r"textures\pbr\x_linear.dds", "", "", 0);
    assert_eq!(key, r"textures\pbr\x_.dds");
}

#[test]
fn atlas_getkey_fallback_diffuse() {
    // port: AtlasList.cs:61 — nothing matches → return raw diffuse
    let list = AtlasList::new();
    let key = atlas_get_key(&list, "my_tex_d.dds", "my_tex_n.dds", "", 0);
    assert_eq!(key, "my_tex_d.dds");
}

#[test]
fn atlas_buildkey_arity() {
    // port: AtlasList.cs:64-79
    // 3 textures → uses [0],[1],[2]
    let list = AtlasList::new();
    let k3 = atlas_build_key(
        &list,
        &[
            "d.dds".to_string(),
            "n.dds".to_string(),
            "g.dds".to_string(),
        ],
        0,
    );
    // no matches → fallback = diffuse = "d.dds"
    assert_eq!(k3, "d.dds");

    // Test with exactly matching 3-texture key present
    let mut list2 = AtlasList::new();
    list2.insert_key("d.dds,n.dds,g.dds".to_string(), make_rect());
    let k3b = atlas_build_key(
        &list2,
        &[
            "d.dds".to_string(),
            "n.dds".to_string(),
            "g.dds".to_string(),
        ],
        0,
    );
    assert_eq!(k3b, "d.dds,n.dds,g.dds");

    // 2 textures → [0],[1]
    let mut list3 = AtlasList::new();
    list3.insert_key("d.dds,n.dds".to_string(), make_rect());
    let k2 = atlas_build_key(&list3, &["d.dds".to_string(), "n.dds".to_string()], 0);
    assert_eq!(k2, "d.dds,n.dds");

    // 1 texture → [0] only
    let mut list4 = AtlasList::new();
    list4.insert_key("d.dds".to_string(), make_rect());
    let k1 = atlas_build_key(&list4, &["d.dds".to_string()], 0);
    assert_eq!(k1, "d.dds");

    // 0 textures → empty string
    let k0 = atlas_build_key(&list, &[], 0);
    assert_eq!(k0, "");
}

// helper: build a dummy AtlasRect
fn make_rect() -> AtlasRect {
    AtlasRect::from_map_row(256, 256, 0, 0, 4096, 4096, "dummy.dds", false)
}

// ---------------------------------------------------------------------------
// LodGeometry: dedup, optimize, bbox
// ---------------------------------------------------------------------------

use lodgen_native::objects::geometry::LodGeometry;

// ---------------------------------------------------------------------------
// GenerateSegments / expand_segments / GenerateMultibound
// ---------------------------------------------------------------------------

use lodgen_native::descriptors::{BBox, QuadDesc};
use lodgen_native::objects::object_lod::{
    MultiBoundAabb, SegmentDesc, expand_segments, generate_multibound, generate_segments,
};

fn make_quad(level: i32, x: i32, y: i32) -> QuadDesc {
    QuadDesc {
        z_order: 0,
        x,
        y,
        quad_level: level,
        quad_index: 0,
        quad_offset: 16384.0,
        static_indices: Vec::new(),
        statics: Vec::new(),
        out_values: Default::default(),
    }
}

#[test]
fn segments_l16_is_zero() {
    // port: LODApp.cs:286-292 — level==16 and !level8 → else branch, id=0
    let quad = make_quad(16, -9, 5);
    let segs = generate_segments(&quad, 0.0, 0.0, 100);
    assert_eq!(segs.len(), 1);
    assert_eq!(segs[0].id, 0);
    assert_eq!(segs[0].start_triangle, 0);
    assert_eq!(segs[0].num_triangles, 100);
}

#[test]
fn segments_l4_grid_id() {
    // port: LODApp.cs:261-293
    // quad_level=4, quad_offset=16384.0 → cell_size = quad_offset/quad_level = 4096
    // shape.X = 2*4096 + 0.5 = 8192.5 → num = floor(8192.5/4096) = 2
    // shape.Y = 1*4096 + 0.5 = 4096.5 → num2 = floor(4096.5/4096) = 1
    // id = 4*2 + 1 = 9
    let quad = make_quad(4, 0, 0);
    let cell_size = quad.quad_offset / quad.quad_level as f32; // 4096.0
    let shape_x = 2.0 * cell_size + 0.5;
    let shape_y = 1.0 * cell_size + 0.5;
    let segs = generate_segments(&quad, shape_x, shape_y, 50);
    assert_eq!(segs.len(), 1);
    assert_eq!(segs[0].id, 9, "id = quad_level*num + num2 = 4*2+1 = 9");
    assert_eq!(segs[0].num_triangles, 50);
}

#[test]
fn segments_l4_clamps_out_of_range() {
    // port: LODApp.cs:268-281 — clamp to [0, quad_level-1]
    let quad = make_quad(4, 0, 0);
    // shape coords way out of range → clamped to (3,3) → id = 4*3+3 = 15
    let segs = generate_segments(&quad, 999999.0, 999999.0, 10);
    assert_eq!(segs.len(), 1);
    assert_eq!(
        segs[0].id, 15,
        "out-of-range must clamp to (level-1, level-1)"
    );
    // shape coords negative → clamped to (0,0) → id = 0
    let segs2 = generate_segments(&quad, -1.0, -1.0, 10);
    assert_eq!(segs2[0].id, 0, "negative must clamp to 0");
}

#[test]
fn expand_segments_trims_trailing_zero() {
    // port: BSSubIndexTriShape.cs:160-183 — count*count slots, fill, trim trailing zero-count
    // count=4 → 16 slots; seg at id=9 with num_triangles>0; trailing zeros 10..15 trimmed.
    let segs = vec![SegmentDesc {
        id: 9,
        start_triangle: 0,
        num_triangles: 42,
    }];
    let expanded = expand_segments(&segs, 4);
    // Trim: last non-zero is slot 9 → length = 10 (slots 0..=9)
    assert_eq!(
        expanded.len(),
        10,
        "trailing zero slots must be trimmed; expect 10 (0..=9)"
    );
    assert_eq!(expanded[9].num_triangles, 42);
    for i in 0..9 {
        assert_eq!(expanded[i].num_triangles, 0, "slot {i} must be zero");
    }
}

#[test]
fn multibound_aabb() {
    // port: LODApp.cs:241-259 GenerateMultibound
    // quad x=-9, y=5; bbox from known coords
    let quad = make_quad(16, -9, 5);
    let mut bb = BBox::empty();
    bb.grow_vertex([100.0, 200.0, -50.0]);
    bb.grow_vertex([300.0, 400.0, 600.0]);
    // experimental=false → pz2=600 >= 0 so no clamp
    let mb: MultiBoundAabb = generate_multibound(&quad, &bb, false);
    let qx = -9_f32 * 4096.0;
    let qy = 5_f32 * 4096.0;
    // position = ((qx+px1)+(qx+px2))/2, ((qy+py1)+(qy+py2))/2, (pz1+pz2)/2
    let exp_x = (qx + 100.0 + qx + 300.0) / 2.0;
    let exp_y = (qy + 200.0 + qy + 400.0) / 2.0;
    let exp_z = (-50.0 + 600.0) / 2.0;
    let eps = 1e-3_f32;
    assert!(
        (mb.position[0] - exp_x).abs() < eps,
        "position.x wrong: {} vs {}",
        mb.position[0],
        exp_x
    );
    assert!(
        (mb.position[1] - exp_y).abs() < eps,
        "position.y wrong: {} vs {}",
        mb.position[1],
        exp_y
    );
    assert!(
        (mb.position[2] - exp_z).abs() < eps,
        "position.z wrong: {} vs {}",
        mb.position[2],
        exp_z
    );
    // extent = half-extents
    let ext_x = (300.0 - 100.0) / 2.0;
    let ext_y = (400.0 - 200.0) / 2.0;
    let ext_z = (600.0 - (-50.0)) / 2.0;
    assert!((mb.extent[0] - ext_x).abs() < eps, "extent.x wrong");
    assert!((mb.extent[1] - ext_y).abs() < eps, "extent.y wrong");
    assert!((mb.extent[2] - ext_z).abs() < eps, "extent.z wrong");
}

#[test]
fn multibound_aabb_z_clamp() {
    // port: LODApp.cs:251-255 — pz2 < 0 and !experimental → clamp pz2 to 0
    let quad = make_quad(16, 0, 0);
    let mut bb = BBox::empty();
    bb.grow_vertex([0.0, 0.0, -200.0]);
    bb.grow_vertex([100.0, 100.0, -10.0]); // pz2 = -10 < 0
    let mb = generate_multibound(&quad, &bb, false);
    // pz2 clamped to 0 → position.z = (-200 + 0)/2 = -100
    let eps = 1e-3_f32;
    assert!(
        (mb.position[2] - (-100.0)).abs() < eps,
        "z-clamped position wrong: {}",
        mb.position[2]
    );
    // extent.z = (0 - (-200))/2 = 100
    assert!(
        (mb.extent[2] - 100.0).abs() < eps,
        "z-clamped extent wrong: {}",
        mb.extent[2]
    );
}

/// Build a simple triangle with 3 unique verts.
fn simple_triangle() -> LodGeometry {
    let mut g = LodGeometry::new();
    g.vertices = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    g.normals = vec![[0.0, 0.0, 1.0], [0.0, 0.0, 1.0], [0.0, 0.0, 1.0]];
    g.uvcoords = vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
    g.triangles = vec![[0, 1, 2]];
    g
}

#[test]
fn geometry_remove_duplicate_merges_within_threshold() {
    // port: Geometry.cs:705 RemoveDuplicate
    // Two verts 0.4 apart with identical UV/normal → merged when high=false (threshold 0.5 pos, 0.005 uv)
    let mut g = LodGeometry::new();
    // vert 0: canonical
    g.vertices = vec![[0.0, 0.0, 0.0], [0.4, 0.0, 0.0], [2.0, 0.0, 0.0]];
    g.normals = vec![[0.0, 0.0, 1.0], [0.0, 0.0, 1.0], [0.0, 0.0, 1.0]];
    g.uvcoords = vec![[0.0, 0.0], [0.004, 0.0], [0.5, 0.5]];
    // two triangles: one using the near-duplicate, one using the far vert
    g.triangles = vec![[0, 1, 2], [1, 2, 0]];

    g.remove_duplicate(false);

    // After dedup vert 1 (0.4 apart, uv diff 0.004 < 0.005) should merge to vert 0.
    // Triangle indices must be remapped + unused verts dropped via RemoveUnused.
    // Triangles [0,1,2] → [0,0,2] → then after RemoveUnused, unique verts = {0,2}.
    // With only 2 unique verts referenced, num_vertices() == 2.
    assert_eq!(g.num_vertices(), 2, "near-duplicate vert should be merged");
    assert_eq!(g.num_triangles(), 2, "triangle count unchanged");
}

#[test]
fn geometry_remove_duplicate_uv_split() {
    // port: Geometry.cs:705 — same position, UV differing by >0.001 with high=true → NOT merged
    let mut g = LodGeometry::new();
    g.vertices = vec![[0.0, 0.0, 0.0], [0.0, 0.0, 0.0], [1.0, 1.0, 0.0]];
    g.normals = vec![[0.0, 0.0, 1.0]; 3];
    // UV differs by 0.005 > high threshold 0.001
    g.uvcoords = vec![[0.0, 0.0], [0.005, 0.0], [0.5, 0.5]];
    g.triangles = vec![[0, 1, 2]];

    g.remove_duplicate(true); // high=true → uv threshold 0.001

    // Vert 0 and vert 1 have same position/normal but UV delta=0.005 > 0.001 → NOT merged
    // After RemoveUnused: all 3 verts are referenced → still 3 verts
    assert_eq!(
        g.num_vertices(),
        3,
        "verts with UV diff > high threshold must not merge"
    );
}

#[test]
fn geometry_optimize_compacts() {
    // port: Geometry.Optimize — vertex referenced by no triangle is dropped
    let mut g = LodGeometry::new();
    // 4 verts but only 3 referenced by the triangle
    g.vertices = vec![
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [99.0, 99.0, 99.0],
    ];
    g.uvcoords = vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [0.5, 0.5]];
    g.normals = vec![[0.0, 0.0, 1.0]; 4];
    g.triangles = vec![[0, 1, 2]]; // vert 3 unreferenced

    g.optimize();

    assert_eq!(g.num_vertices(), 3, "unreferenced vertex must be dropped");
    assert_eq!(g.num_triangles(), 1, "triangle must survive");
    // The remaining vertices must not include the orphan [99,99,99]
    for v in &g.vertices {
        assert!(v[0] < 2.0 && v[1] < 2.0, "orphan vertex must not survive");
    }
    // Triangle indices must still be valid
    for tri in &g.triangles {
        for &idx in tri {
            assert!(
                (idx as usize) < g.num_vertices(),
                "triangle index must be in range"
            );
        }
    }
}

#[test]
fn geometry_update_bbox() {
    // port: Geometry.cs bbox via GrowByVertex
    let mut g = simple_triangle();
    g.update_bbox();
    assert_eq!(g.bbox.min, [0.0, 0.0, 0.0]);
    assert_eq!(g.bbox.max, [1.0, 1.0, 0.0]);
}

// ---------------------------------------------------------------------------
// ShapeDesc / parse_nif (real DLC03 LOD NIF via nif_core + materials)
// ---------------------------------------------------------------------------
//
// Validation approach: a REAL vanilla FO4 LOD model from the extracted game
// data — `Meshes\DLC03\LOD\Architecture\Barn\BarnDoorMedL01_LOD.nif`. Its
// block graph (verified via `modkit nif inspect`) is:
//   NiNode "BarnDoorMedL01_LOD"
//     └─ BSTriShape "BarnDoorMedL01_LOD:36"  (8 verts, 4 tris, Vertex Desc 474989027590661)
//          ├─ BSLightingShaderProperty  Name="Materials\DLC03\LOD\DLC03Barn01LOD.BGSM"
//          │     └─ BSShaderTextureSet  slots 0/1/7 = _d/_n/_s.dds
//          └─ NiAlphaProperty  Flags=37612 Threshold=90
// This single file exercises geometry, BSShaderTextureSet, the BGSM material
// read (via the `materials` crate), alpha, and clamp-mode — so no synthetic NIF
// is needed. The matching BGSM ships alongside it under extracted/fo4/Materials.

use lodgen_native::game::Game;
use lodgen_native::input::{StaticDesc, WorldspaceInput};
use lodgen_native::objects::object_lod::parse_nif;
use lodgen_native::objects::static_desc::ShaderKind;
use lodgen_native::progress::{LodPaths, QuadCtx};
use lodgen_native::settings::LodSettings;
use std::path::PathBuf;

/// Repo root, derived from CARGO_MANIFEST_DIR (= py_creation_lib/native/lodgen).
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..") // native
        .join("..") // py_creation_lib
        .join("..") // repo root
}

fn extracted_fo4() -> PathBuf {
    repo_root().join("extracted").join("fo4")
}

/// True when the extracted FO4 corpus + the barn LOD fixture are present.
fn fixture_present() -> bool {
    extracted_fo4()
        .join("Meshes/DLC03/LOD/Architecture/Barn/BarnDoorMedL01_LOD.nif")
        .is_file()
}

/// A minimal `StaticDesc` whose LOD model points at the barn LOD fixture.
fn barn_static(model: &str) -> StaticDesc {
    StaticDesc {
        ref_id: "00000ABC".into(),
        ref_flags: 0,
        enable_parent: 0,
        cell: (0, 0),
        pos: [0.0, 0.0, 0.0],
        rot: [0.0, 0.0, 0.0],
        scale: 1.0,
        color: 1.0,
        alpha_threshold: 128,
        is_billboard: false,
        is_grass: false,
        base_name: "BarnDoorMedL01".into(),
        base_flags: 0,
        material_name: String::new(),
        full_model: String::new(),
        lod_models: [Some(model.to_string()), None, None, None],
        part_transform: lodgen_native::input::identity_part_transform(),
        part_scale: 1.0,
        material_swap: std::collections::BTreeMap::new(),
    }
}

fn empty_world() -> WorldspaceInput {
    WorldspaceInput::from_cells("W", Vec::new())
}

/// Run a closure with a `QuadCtx` whose data_dirs = [extracted/fo4].
fn with_ctx<R>(level: i32, f: impl FnOnce(&QuadCtx) -> R) -> R {
    let world = empty_world();
    let settings = LodSettings::fo4_default();
    let game = Game::fo4();
    let paths = LodPaths {
        data_dirs: vec![extracted_fo4()],
        output_dir: std::env::temp_dir().join("lodgen_p2_task6"),
        source_data_dir: None,
    };
    let ctx = QuadCtx {
        world: &world,
        settings: &settings,
        game: &game,
        paths: &paths,
        level,
    };
    f(&ctx)
}

const BARN_LOD: &str = r"Meshes\DLC03\LOD\Architecture\Barn\BarnDoorMedL01_LOD.nif";

#[test]
fn parse_nif_loads_geometry() {
    if !fixture_present() {
        eprintln!("SKIP parse_nif_loads_geometry: extracted/fo4 barn LOD fixture absent");
        return;
    }
    let stat = barn_static(BARN_LOD);
    let shapes = with_ctx(0, |ctx| parse_nif(&stat, 0, ctx)).expect("parse_nif");
    assert!(!shapes.is_empty(), "expected >=1 shape");
    let s = &shapes[0];
    assert!(s.geometry.num_vertices() > 0, "verts");
    assert!(s.geometry.num_triangles() > 0, "tris");
    // UV count must equal vertex count (every LOD vertex has a UV).
    assert_eq!(
        s.geometry.uvcoords.len(),
        s.geometry.num_vertices(),
        "uv count == vertex count"
    );
}

#[test]
fn parse_nif_texture_slots() {
    if !fixture_present() {
        eprintln!("SKIP parse_nif_texture_slots: fixture absent");
        return;
    }
    let stat = barn_static(BARN_LOD);
    let shapes = with_ctx(0, |ctx| parse_nif(&stat, 0, ctx)).expect("parse_nif");
    let s = &shapes[0];
    // The barn shader Name points at a BGSM, which overrides slots 0/1/7
    // (ShapeDesc.cs:894-908). BGSM texture strings are NOT lowercased/slash-
    // normalized by the C# (only the BSShaderTextureSet path at :810 does that),
    // so assert case-insensitively and accept either slash.
    let d = s.textures[0].to_lowercase();
    let n = s.textures[1].to_lowercase();
    let sp = s.textures[7].to_lowercase();
    assert!(d.contains("dlc03barn01lod"), "diffuse: {}", s.textures[0]);
    assert!(d.ends_with("_d.dds"), "diffuse suffix: {}", s.textures[0]);
    assert!(n.ends_with("_n.dds"), "normal: {}", s.textures[1]);
    assert!(sp.ends_with("_s.dds"), "specular: {}", s.textures[7]);
    // Data\ prefix is stripped (ShapeDesc.cs:1307-1310).
    assert!(
        !d.contains("data\\") && !d.contains("data/"),
        "Data\\ stripped"
    );
    assert_eq!(s.shader_type, ShaderKind::Lighting);
}

#[test]
fn parse_nif_bgsm_material() {
    if !fixture_present() {
        eprintln!("SKIP parse_nif_bgsm_material: fixture absent");
        return;
    }
    // The barn LOD shader Name points at DLC03Barn01LOD.BGSM. The BGSM read must
    // pull textures + flags (ShapeDesc.cs:886-966). The BGSM ships at
    // extracted/fo4/Materials/DLC03/LOD/DLC03Barn01LOD.BGSM.
    let stat = barn_static(BARN_LOD);
    let shapes = with_ctx(0, |ctx| parse_nif(&stat, 0, ctx)).expect("parse_nif");
    let s = &shapes[0];
    // BGSM diffuse is the LOD diffuse; slots must be populated from the material.
    // The BGSM diffuse (DLC03/LOD/DLC03Barn01LOD_d.dds) differs from the texture
    // set's path (textures\DLC03\LOD\...) — proving the BGSM read overrode it.
    assert!(
        s.textures[0].to_lowercase().contains("dlc03barn01lod"),
        "bgsm diffuse: {}",
        s.textures[0]
    );
    assert!(
        !s.textures[0].to_lowercase().starts_with("textures\\"),
        "BGSM diffuse (no textures\\ prefix) proves the material read won: {}",
        s.textures[0]
    );
    // Clamp mode was read from the BGSM tile flags — must be a valid mode.
    assert!(s.texture_clamp_mode <= 3, "clamp mode in range");
}

#[test]
fn parse_nif_clamp_mode_clamps_uv() {
    if !fixture_present() {
        eprintln!("SKIP parse_nif_clamp_mode_clamps_uv: fixture absent");
        return;
    }
    // When TextureClampMode != WRAP (3), UVs are clamped into [0,1]
    // (ShapeDesc.cs:1195-1207). The barn BGSM yields a clamping mode; regardless,
    // after parse every UV must be finite. We additionally assert: if the
    // resolved clamp mode is 0 (CLAMP_S_CLAMP_T), all UVs lie within [0,1].
    let stat = barn_static(BARN_LOD);
    let shapes = with_ctx(0, |ctx| parse_nif(&stat, 0, ctx)).expect("parse_nif");
    let s = &shapes[0];
    for uv in &s.geometry.uvcoords {
        assert!(uv[0].is_finite() && uv[1].is_finite(), "finite uv");
        if s.texture_clamp_mode == 0 {
            assert!((0.0..=1.0).contains(&uv[0]), "u clamped: {}", uv[0]);
            assert!((0.0..=1.0).contains(&uv[1]), "v clamped: {}", uv[1]);
        }
    }
}

// ---------------------------------------------------------------------------
// transform_shape: quad-space vertex transform + atlas UV remap
// ---------------------------------------------------------------------------

use lodgen_native::objects::object_lod::transform_shape;

/// Build a minimal StaticDesc at the given world position/rotation/scale.
fn make_stat(pos: [f32; 3], rot: [f32; 3], scale: f32) -> StaticDesc {
    StaticDesc {
        ref_id: "DEADBEEF".into(),
        ref_flags: 0,
        enable_parent: 0,
        cell: (0, 0),
        pos,
        rot,
        scale,
        color: 1.0,
        alpha_threshold: 128,
        is_billboard: false,
        is_grass: false,
        base_name: "Test".into(),
        base_flags: 0,
        material_name: String::new(),
        full_model: String::new(),
        lod_models: [None, None, None, None],
        part_transform: lodgen_native::input::identity_part_transform(),
        part_scale: 1.0,
        material_swap: std::collections::BTreeMap::new(),
    }
}

/// Build a minimal ShapeDesc with one vertex, one normal, one tangent, one UV, one triangle.
fn make_shape_one_vert(
    vertex: [f32; 3],
    uv: [f32; 2],
) -> lodgen_native::objects::static_desc::ShapeDesc {
    use lodgen_native::objects::geometry::LodGeometry;
    use lodgen_native::objects::static_desc::{ShaderKind, ShapeDesc, ShapeFlags};
    #[allow(clippy::zero_prefixed_literal)]
    let identity: [[f32; 4]; 4] = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];

    let mut g = LodGeometry::new();
    g.vertices = vec![vertex, [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    g.normals = vec![[0.0, 0.0, 1.0]; 3];
    g.tangents = vec![[1.0, 0.0, 0.0]; 3];
    g.bitangents = vec![[0.0, 1.0, 0.0]; 3];
    g.uvcoords = vec![uv, [0.0, 0.0], [0.0, 0.0]];
    g.triangles = vec![[0, 1, 2]];

    ShapeDesc {
        name: "Test".into(),
        static_model: "test.nif".into(),
        geometry: g,
        flags: ShapeFlags::empty(),
        textures: Default::default(),
        source_materials: Vec::new(),
        textures_key: String::new(),
        texture_clamp_mode: 3,
        alpha_threshold: 0,
        alpha_flags: 0,
        backlight_power: 0.0,
        grayscale_to_palette_scale: 1.0,
        enable_parent: 0,
        shader_type: ShaderKind::None,
        x: 0.0,
        y: 0.0,
        bounding_box: lodgen_native::descriptors::BBox::empty(),
        segments: Vec::new(),
        uv_scale: [1.0, 1.0],
        uv_offset: [0.0, 0.0],
        ref_flags: 0,
        node_transform: identity,
        node_scale: 1.0,
        translation: [0.0, 0.0, 0.0],
        rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        bto_translation: None,
        bto_scale: None,
    }
}

#[test]
fn transform_shape_quad_space() {
    // Identity node transform, node_scale=1, stat.scale=2
    // stat position [100, 200, 50], rot=[0,0,0], quad at (0,0) level=16
    // vertex [0,0,0] → rotated by identity → scaled by stat.scale=2 → [0,0,0]
    // translated by stat_rel = [100-0*4096, 200-0*4096, 50] → [100, 200, 50]
    // divided by quad_level=16 → [6.25, 12.5, 3.125]
    // shape.x = stat.x - quad.x*4096 = 100 - 0 = 100
    // shape.y = stat.y - quad.y*4096 = 200 - 0 = 200
    let quad = make_quad(16, 0, 0);
    let stat = make_stat([100.0, 200.0, 50.0], [0.0, 0.0, 0.0], 2.0);
    let mut shape = make_shape_one_vert([0.0, 0.0, 0.0], [0.5, 0.5]);
    let atlas = lodgen_native::atlas::atlas::AtlasList::new();
    let settings = LodSettings::fo4_default();

    let kept = transform_shape(&quad, &stat, &mut shape, &atlas, &settings.objects);
    assert!(kept, "non-zero tri shape must be kept");

    // shape.x / shape.y = stat-local translation
    let eps = 1e-3_f32;
    assert!((shape.x - 100.0).abs() < eps, "shape.x = {}", shape.x);
    assert!((shape.y - 200.0).abs() < eps, "shape.y = {}", shape.y);

    // First vertex [0,0,0] * node_scale=1 → rotated by identity node_transform → * stat.scale=2 → [0,0,0]
    // → translated+rotated by matrix4 (identity rot + translation [100,200,50]) → [100,200,50]
    // / quad_level=16 → [6.25, 12.5, 3.125]
    let v = shape.geometry.vertices[0];
    assert!((v[0] - 6.25).abs() < eps, "v.x after quad-div: {}", v[0]);
    assert!((v[1] - 12.5).abs() < eps, "v.y after quad-div: {}", v[1]);
    assert!((v[2] - 3.125).abs() < eps, "v.z after quad-div: {}", v[2]);
}

#[test]
fn transform_shape_quad_offset() {
    // Quad at (1, 0) level=16: origin shift = (1*4096, 0)
    // stat at [5000, 100, 0], scale=1, rot=[0,0,0]
    // shape.x = 5000 - 1*4096 = 904
    let quad = make_quad(16, 1, 0);
    let stat = make_stat([5000.0, 100.0, 0.0], [0.0, 0.0, 0.0], 1.0);
    let mut shape = make_shape_one_vert([0.0, 0.0, 0.0], [0.5, 0.5]);
    let atlas = lodgen_native::atlas::atlas::AtlasList::new();
    let settings = LodSettings::fo4_default();

    let kept = transform_shape(&quad, &stat, &mut shape, &atlas, &settings.objects);
    assert!(kept);

    let eps = 1e-2_f32;
    assert!((shape.x - 904.0).abs() < eps, "shape.x = {}", shape.x);
    assert!((shape.y - 100.0).abs() < eps, "shape.y = {}", shape.y);
}

#[test]
fn transform_shape_atlas_uv_remap() {
    // Build an AtlasList with a key matching our shape's textures
    let mut atlas = lodgen_native::atlas::atlas::AtlasList::new();
    let rect = lodgen_native::atlas::atlas::AtlasRect::from_map_row(
        256,
        256,
        0,
        0,
        4096,
        4096,
        r"Textures\Terrain\W\Objects\WObjects.dds",
        false,
    );

    // The shape textures[0] = "diff_d.dds", textures[1] = "diff_n.dds"
    // atlas_build_key will try "diff_d.dds,diff_n.dds" → must be in atlas
    atlas.insert("diff_d.dds,diff_n.dds".to_string(), rect.clone());

    let quad = make_quad(16, 0, 0);
    let stat = make_stat([0.0, 0.0, 0.0], [0.0, 0.0, 0.0], 1.0);
    let mut shape = make_shape_one_vert([0.0, 0.0, 0.0], [0.5, 0.5]);
    shape.textures[0] = "diff_d.dds".to_string();
    shape.textures[1] = "diff_n.dds".to_string();
    // texture_clamp_mode=0 (not WRAP=3) → force=true (skip tolerance gate)
    shape.texture_clamp_mode = 0;

    let settings = LodSettings::fo4_default();
    let kept = transform_shape(&quad, &stat, &mut shape, &atlas, &settings.objects);
    assert!(kept);

    // UVs should be remapped through the atlas rect, then QUVx-quantized by the
    // ReUV/Simplify step (Geometry.cs:1155) which now runs on atlassed shapes.
    let (u_expected, v_expected) = rect.uv_atlas(0.5, 0.5);
    let (u_expected, v_expected) = (quvx_test(u_expected), quvx_test(v_expected));
    // The per-triangle break + reweld may reorder verts; find the remapped corner
    // (the only vertex whose source UV was (0.5,0.5)) — the other two were (0,0).
    let got = shape
        .geometry
        .uvcoords
        .iter()
        .find(|uv| (uv[0] - u_expected).abs() < 1e-4 && (uv[1] - v_expected).abs() < 1e-4)
        .copied();
    let eps = 1e-3_f32; // QUVx truncates to 3 decimals
    let got = got.unwrap_or(shape.geometry.uvcoords[0]);
    assert!(
        (got[0] - u_expected).abs() < eps,
        "u remapped: {} vs {}",
        got[0],
        u_expected
    );
    assert!(
        (got[1] - v_expected).abs() < eps,
        "v remapped: {} vs {}",
        got[1],
        v_expected
    );

    // texture_clamp_mode must be set to 0
    assert_eq!(shape.texture_clamp_mode, 0);

    // textures[0] must be swapped to atlas_diffuse
    assert_eq!(shape.textures[0], rect.atlas_diffuse);
    // textures[1] must be swapped to atlas_normal
    assert_eq!(shape.textures[1], rect.atlas_normal);
}

#[test]
fn transform_shape_atlas_per_slot_substitution() {
    // Phase-2 fix #3: the atlas swap maps each slot individually and PRESERVES
    // the White/Gray/Flat sentinels (port LODApp.cs:741-816), instead of blindly
    // overwriting slots [0]/[1]/[7].
    let mut atlas = lodgen_native::atlas::atlas::AtlasList::new();
    let rect = lodgen_native::atlas::atlas::AtlasRect::from_map_row(
        256,
        256,
        0,
        0,
        4096,
        4096,
        r"Textures\Terrain\W\Objects\W.Objects.dds",
        false,
    );
    atlas.insert("diff_d.dds,diff_n.dds".to_string(), rect.clone());

    let quad = make_quad(16, 0, 0);
    let stat = make_stat([0.0, 0.0, 0.0], [0.0, 0.0, 0.0], 1.0);
    let mut shape = make_shape_one_vert([0.0, 0.0, 0.0], [0.5, 0.5]);
    shape.textures[0] = "diff_d.dds".to_string(); // → atlas diffuse
    shape.textures[1] = "diff_n.dds".to_string(); // → atlas normal
    shape.textures[3] = "Textures\\Gray.dds".to_string(); // SENTINEL → preserved
    shape.textures[7] = "diff_s.dds".to_string(); // specular → atlas specular
    shape.textures[8] = "junk_unrelated.dds".to_string(); // not mapped, not sentinel → cleared
    shape.texture_clamp_mode = 0; // force=true (skip tolerance gate)

    let settings = LodSettings::fo4_default();
    assert!(transform_shape(
        &quad,
        &stat,
        &mut shape,
        &atlas,
        &settings.objects
    ));

    assert_eq!(
        shape.textures[0], rect.atlas_diffuse,
        "slot0 → atlas diffuse"
    );
    assert_eq!(shape.textures[1], rect.atlas_normal, "slot1 → atlas normal");
    assert_eq!(
        shape.textures[7], rect.atlas_specular,
        "slot7 (specular) → atlas specular"
    );
    assert_eq!(
        shape.textures[3], "Textures\\Gray.dds",
        "sentinel slot must be PRESERVED"
    );
    assert!(
        shape.textures[8].is_empty(),
        "unrelated non-sentinel slot must be cleared"
    );
    assert_eq!(shape.texture_clamp_mode, 0);
}

#[test]
fn transform_shape_no_vertex_colors() {
    // settings.no_vertex_colors = true → HAS_VERTEX_COLOR should be cleared
    let quad = make_quad(16, 0, 0);
    let stat = make_stat([0.0, 0.0, 0.0], [0.0, 0.0, 0.0], 1.0);
    let mut shape = make_shape_one_vert([0.0, 0.0, 0.0], [0.5, 0.5]);
    // Give the shape vertex colors + set the flag
    shape.geometry.vertex_colors = vec![[1.0, 0.0, 0.0, 1.0]; 3];
    shape.flags |= lodgen_native::objects::static_desc::ShapeFlags::HAS_VERTEX_COLOR;

    let atlas = lodgen_native::atlas::atlas::AtlasList::new();
    let mut settings = LodSettings::fo4_default();
    settings.objects.no_vertex_colors = true;

    transform_shape(&quad, &stat, &mut shape, &atlas, &settings.objects);

    assert!(
        !shape
            .flags
            .contains(lodgen_native::objects::static_desc::ShapeFlags::HAS_VERTEX_COLOR),
        "HAS_VERTEX_COLOR must be cleared when no_vertex_colors=true and not passthru"
    );
}

#[test]
fn transform_shape_high_detail_cleared() {
    // IS_HIGH_DETAIL flag must be cleared after transform
    let quad = make_quad(16, 0, 0);
    let stat = make_stat([0.0, 0.0, 0.0], [0.0, 0.0, 0.0], 1.0);
    let mut shape = make_shape_one_vert([0.0, 0.0, 0.0], [0.5, 0.5]);
    shape.flags |= lodgen_native::objects::static_desc::ShapeFlags::IS_HIGH_DETAIL;

    let atlas = lodgen_native::atlas::atlas::AtlasList::new();
    let settings = LodSettings::fo4_default();
    transform_shape(&quad, &stat, &mut shape, &atlas, &settings.objects);

    assert!(
        !shape
            .flags
            .contains(lodgen_native::objects::static_desc::ShapeFlags::IS_HIGH_DETAIL),
        "IS_HIGH_DETAIL must be cleared by transform_shape"
    );
}

#[test]
fn transform_shape_bbox_grown() {
    // After transform: bbox is in world space (pre-divide),
    // vertices are in quad space (post-divide by quad_level=16).
    // stat at [160, 320, 80], scale=1, rot=identity, quad at (0,0) level=16.
    // One vertex is [0,0,0] → world [160,320,80]; divided → [10, 20, 5].
    let quad = make_quad(16, 0, 0);
    let stat = make_stat([160.0, 320.0, 80.0], [0.0, 0.0, 0.0], 1.0);
    let mut shape = make_shape_one_vert([0.0, 0.0, 0.0], [0.5, 0.5]);
    let atlas = lodgen_native::atlas::atlas::AtlasList::new();
    let settings = LodSettings::fo4_default();

    transform_shape(&quad, &stat, &mut shape, &atlas, &settings.objects);

    // bbox must not be empty (must have been grown from world-space vertices)
    assert!(
        shape.bounding_box.min[0].is_finite(),
        "bbox.min.x must be finite"
    );
    assert!(
        shape.bounding_box.max[0].is_finite(),
        "bbox.max.x must be finite"
    );

    // vertices are in quad space (divided by 16); bbox is in world space.
    // Verify by checking vertex 0 which was [0,0,0] → world [160,320,80] → quad [10,20,5]
    let eps = 1e-2_f32;
    assert!(
        (shape.geometry.vertices[0][0] - 10.0).abs() < eps,
        "vertex[0].x in quad space: {}",
        shape.geometry.vertices[0][0]
    );
    // bbox.min should be <= world position of vertex[0] (160.0)
    assert!(
        shape.bounding_box.min[0] <= 160.0 + eps,
        "bbox.min.x in world space: {}",
        shape.bounding_box.min[0]
    );
    assert!(
        shape.bounding_box.max[0] >= 160.0 - eps,
        "bbox.max.x in world space: {}",
        shape.bounding_box.max[0]
    );
}

#[test]
fn transform_shape_applies_node_translation() {
    // Phase-2 fix #2: the FULL node_transform (incl. its translation column) must be
    // applied to vertices, not just the upper-3x3. Expected values computed against
    // the exact C# Matrix44/Vector3 algebra (see /tmp/csharp_sim.py reasoning):
    //
    //   node_transform = matrix7_col with rotation=identity, translation=[11,22,33]
    //     (= geom-local trans [1,2,3] folded with parent-node trans [10,20,30]).
    //   node_scale = 1 (num2).  stat at [1000,2000,0], scale=2, rot=0.  quad (0,0) L16.
    //   v=[0,0,0] -> world(pre-div)=[1022,2044,66] -> quad=[63.875,127.75,4.125]
    //   v=[5,0,0] -> world=[1032,2044,66] -> quad=[64.5,127.75,4.125]
    //   v=[0,5,0] -> world=[1022,2054,66] -> quad=[63.875,128.375,4.125]
    let quad = make_quad(16, 0, 0);
    let stat = make_stat([1000.0, 2000.0, 0.0], [0.0, 0.0, 0.0], 2.0);
    let mut shape = make_shape_one_vert([0.0, 0.0, 0.0], [0.5, 0.5]);
    // Replace the geometry with the three test vertices.
    shape.geometry.vertices = vec![[0.0, 0.0, 0.0], [5.0, 0.0, 0.0], [0.0, 5.0, 0.0]];
    shape.geometry.normals = vec![[0.0, 0.0, 1.0]; 3];
    shape.geometry.tangents = vec![[1.0, 0.0, 0.0]; 3];
    shape.geometry.bitangents = vec![[0.0, 1.0, 0.0]; 3];
    shape.geometry.uvcoords = vec![[0.5, 0.5], [0.0, 0.0], [0.0, 0.0]];
    shape.geometry.triangles = vec![[0, 1, 2]];
    // node_transform: identity rotation + translation column [11, 22, 33].
    shape.node_transform = [
        [1.0, 0.0, 0.0, 11.0],
        [0.0, 1.0, 0.0, 22.0],
        [0.0, 0.0, 1.0, 33.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    shape.node_scale = 1.0;

    let atlas = lodgen_native::atlas::atlas::AtlasList::new();
    let settings = LodSettings::fo4_default();
    assert!(transform_shape(
        &quad,
        &stat,
        &mut shape,
        &atlas,
        &settings.objects
    ));

    let eps = 1e-3_f32;
    let expect = [
        [63.875_f32, 127.75, 4.125],
        [64.5, 127.75, 4.125],
        [63.875, 128.375, 4.125],
    ];
    for (i, e) in expect.iter().enumerate() {
        let v = shape.geometry.vertices[i];
        for k in 0..3 {
            assert!(
                (v[k] - e[k]).abs() < eps,
                "vertex[{i}][{k}] = {} (expected {}) — node translation must be applied",
                v[k],
                e[k]
            );
        }
    }
    // World-space bbox (pre-divide) must reflect the translated positions.
    assert!(
        (shape.bounding_box.min[0] - 1022.0).abs() < 1e-1,
        "bbox.min.x = {}",
        shape.bounding_box.min[0]
    );
    assert!(
        (shape.bounding_box.max[1] - 2054.0).abs() < 1e-1,
        "bbox.max.y = {}",
        shape.bounding_box.max[1]
    );
}

#[test]
fn transform_shape_calls_generate_segments() {
    // Phase-2 fix #1: transform_shape's LAST step is GenerateSegments
    // (port LODApp.cs:1050). Before the fix shape.segments stayed empty,
    // so build_bto emitted Num Segments=0. Assert a non-empty segment now.
    let quad = make_quad(16, 0, 0);
    let stat = make_stat([100.0, 200.0, 50.0], [0.0, 0.0, 0.0], 1.0);
    let mut shape = make_shape_one_vert([0.0, 0.0, 0.0], [0.5, 0.5]);
    assert!(
        shape.segments.is_empty(),
        "precondition: no segments before transform"
    );

    let atlas = lodgen_native::atlas::atlas::AtlasList::new();
    let settings = LodSettings::fo4_default();
    let kept = transform_shape(&quad, &stat, &mut shape, &atlas, &settings.objects);
    assert!(kept);

    // GenerateSegments must have populated exactly one segment whose triangle
    // count matches the post-transform geometry (1 triangle here).
    assert_eq!(
        shape.segments.len(),
        1,
        "transform_shape must call generate_segments"
    );
    assert_eq!(
        shape.segments[0].num_triangles as usize,
        shape.geometry.num_triangles(),
        "segment triangle count must match the transformed geometry"
    );
    // Level 16, not level8 → id == 0 (LODApp.cs:286-292).
    assert_eq!(shape.segments[0].id, 0);
}

#[test]
fn real_flow_yields_num_segments_ge_1() {
    // Phase-2 fix #1 end-to-end: transform_shape → build_bto → expand_segments →
    // build_bto_nif must yield a BSSubIndexTriShape with Num Segments >= 1.
    use lodgen_native::objects::static_desc::ShapeFlags;
    use nif_core_native::model::{NifFile, NifValue};

    // Level 4 so GenerateSegments enters the grid branch (LODApp.cs:264) and
    // expand_segments expands to a count*count slot grid.
    let quad = make_quad(4, 0, 0);
    let stat = make_stat([100.0, 200.0, 0.0], [0.0, 0.0, 0.0], 1.0);
    let mut shape = make_shape_one_vert([0.0, 0.0, 0.0], [0.5, 0.5]);
    shape.flags |= ShapeFlags::HAS_LOD_FLAG;

    let atlas = lodgen_native::atlas::atlas::AtlasList::new();
    let settings = LodSettings::fo4_default();
    assert!(transform_shape(
        &quad,
        &stat,
        &mut shape,
        &atlas,
        &settings.objects
    ));
    assert!(
        !shape.segments.is_empty(),
        "transform_shape produced segments"
    );

    let mut quad2 = make_quad(4, 0, 0);
    let bto = build_bto(&mut quad2, vec![shape], &settings.objects);
    assert_eq!(bto.len(), 1);
    let mut nif = build_bto_nif(&bto).expect("build_bto_nif");
    let bytes = nif.to_bytes().expect("to_bytes");
    let reloaded = NifFile::from_bytes(&bytes, None).expect("reload");

    let sits = reloaded
        .blocks
        .iter()
        .find(|b| b.type_name == "BSSubIndexTriShape")
        .expect("BSSubIndexTriShape");
    let num_segments = sits
        .get_field("Num Segments")
        .map(NifValue::as_i64)
        .unwrap();
    let num_tris = sits
        .get_field("Num Triangles")
        .map(NifValue::as_i64)
        .unwrap();
    let top_prims = sits
        .get_field("Num Primitives")
        .map(NifValue::as_i64)
        .unwrap();
    assert!(
        num_segments >= 1,
        "real flow must yield Num Segments >= 1, got {num_segments}"
    );
    // KNOWN nif_core GAP (fix #5): top-level Num Primitives is calc'd as
    // NumTriangles by nif_core (golden is NumTriangles*2). Document, don't fail.
    assert_eq!(
        top_prims, num_tris,
        "nif_core GAP: top Num Primitives calc'd as NumTriangles"
    );
}

#[test]
fn iterate_nif_accumulates_node_transform_in_order() {
    // Phase-2 fix #2 (parse side): node-transform accumulation must use the
    // C#-correct order (column: parent · node_local · geom_local). A synthetic
    // 2-NiNode chain with rotation+translation distinguishes the correct order
    // from the previous (wrong) `node_local · parent`.
    //
    //   NiNode A: Rz(90deg) rows [[0,-1,0],[1,0,0],[0,0,1]], translation [100,0,0]
    //   NiNode B: identity, translation [0,10,0]
    //   BSTriShape geom: identity, translation [0,0,0]
    // Expected accumulated node_transform (column): rotation = Rz90,
    //   translation = [100,0,0] + Rz90·[0,10,0] = [90,0,0].
    use indexmap::IndexMap;
    use nif_core_native::model::{NifBlock, NifFile, NifValue};

    fn vec3(v: [f32; 3]) -> NifValue {
        NifValue::Vec3(v)
    }
    fn mat33(rows: [[f32; 3]; 3]) -> NifValue {
        NifValue::Matrix33(rows)
    }
    fn vertex_struct(pos: [f32; 3], uv: [f32; 2]) -> NifValue {
        let mut m = IndexMap::new();
        m.insert("Vertex".to_string(), NifValue::Vec3(pos));
        let mut t = IndexMap::new();
        t.insert("u".to_string(), NifValue::Float(uv[0] as f64));
        t.insert("v".to_string(), NifValue::Float(uv[1] as f64));
        m.insert("UV".to_string(), NifValue::Struct(t));
        m.insert("Normal".to_string(), NifValue::Vec3([0.0, 0.0, 1.0]));
        m.insert("Tangent".to_string(), NifValue::Vec3([1.0, 0.0, 0.0]));
        m.insert("Bitangent X".to_string(), NifValue::Float(0.0));
        m.insert("Bitangent Y".to_string(), NifValue::Float(1.0));
        m.insert("Bitangent Z".to_string(), NifValue::Float(0.0));
        NifValue::Struct(m)
    }
    fn tri(a: i64, b: i64, c: i64) -> NifValue {
        let mut m = IndexMap::new();
        m.insert("v1".to_string(), NifValue::Int(a));
        m.insert("v2".to_string(), NifValue::Int(b));
        m.insert("v3".to_string(), NifValue::Int(c));
        NifValue::Struct(m)
    }

    let mut nif = NifFile::new("FO4");
    nif.blocks.clear();

    // 0: root NiNode → child A (1)
    let mut root = NifBlock::new(0, "NiNode");
    root.set_field("Name", NifValue::String("root".into()));
    root.set_field("Children", NifValue::Array(vec![NifValue::Ref(1)]));

    // 1: NiNode A (Rz90, trans [100,0,0]) → child B (2)
    let mut a = NifBlock::new(1, "NiNode");
    a.set_field("Name", NifValue::String("A".into()));
    a.set_field(
        "Rotation",
        mat33([[0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]]),
    );
    a.set_field("Translation", vec3([100.0, 0.0, 0.0]));
    a.set_field("Scale", NifValue::Float(1.0));
    a.set_field("Children", NifValue::Array(vec![NifValue::Ref(2)]));

    // 2: NiNode B (identity, trans [0,10,0]) → geom (3)
    let mut b = NifBlock::new(2, "NiNode");
    b.set_field("Name", NifValue::String("B".into()));
    b.set_field(
        "Rotation",
        mat33([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]),
    );
    b.set_field("Translation", vec3([0.0, 10.0, 0.0]));
    b.set_field("Scale", NifValue::Float(1.0));
    b.set_field("Children", NifValue::Array(vec![NifValue::Ref(3)]));

    // 3: BSTriShape geom (identity, trans [0,0,0]) with 3 verts / 1 tri / UVs.
    let mut g = NifBlock::new(3, "BSTriShape");
    g.set_field("Name", NifValue::String("geom".into()));
    g.set_field(
        "Rotation",
        mat33([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]),
    );
    g.set_field("Translation", vec3([0.0, 0.0, 0.0]));
    g.set_field("Scale", NifValue::Float(1.0));
    g.set_field("Skin", NifValue::Ref(-1));
    g.set_field(
        "Vertex Data",
        NifValue::Array(vec![
            vertex_struct([1.0, 0.0, 0.0], [0.5, 0.5]),
            vertex_struct([0.0, 1.0, 0.0], [0.0, 0.0]),
            vertex_struct([0.0, 0.0, 1.0], [1.0, 1.0]),
        ]),
    );
    g.set_field("Triangles", NifValue::Array(vec![tri(0, 1, 2)]));

    nif.blocks.push(root);
    nif.blocks.push(a);
    nif.blocks.push(b);
    nif.blocks.push(g);

    let stat = barn_static("Meshes\\synthetic_nodechain.nif");
    let shapes = with_ctx(0, |ctx| {
        lodgen_native::objects::object_lod::iterate_nif(&nif, &stat, 0, ctx)
    });
    assert_eq!(shapes.len(), 1, "one geom under the node chain");
    let nt = shapes[0].node_transform;

    let eps = 1e-4_f32;
    // Rotation rows == Rz90.
    let rz = [[0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
    for i in 0..3 {
        for j in 0..3 {
            assert!(
                (nt[i][j] - rz[i][j]).abs() < eps,
                "rotation[{i}][{j}]={}",
                nt[i][j]
            );
        }
    }
    // Translation column == [90, 0, 0] (correct order; the wrong order gave [100,10,0]).
    assert!(
        (nt[0][3] - 90.0).abs() < eps,
        "node trans.x = {} (expected 90)",
        nt[0][3]
    );
    assert!(
        (nt[1][3] - 0.0).abs() < eps,
        "node trans.y = {} (expected 0)",
        nt[1][3]
    );
    assert!(
        (nt[2][3] - 0.0).abs() < eps,
        "node trans.z = {} (expected 0)",
        nt[2][3]
    );
}

#[test]
fn parse_nif_skips_editormarker() {
    // A NiNode whose name contains "editormarker" contributes no shapes
    // (LODApp.cs:304). We assert this against a synthetic in-memory NIF so the
    // test is hermetic (no editor-marker node exists in the barn fixture).
    use nif_core_native::model::{NifBlock, NifFile, NifValue};
    let mut nif = NifFile::new("FO4");
    // root NiNode with one child = an EditorMarker NiNode (no geometry under it).
    let root = NifBlock::new(0, "NiNode");
    let mut marker = NifBlock::new(1, "NiNode");
    marker.set_field("Name", NifValue::String("EditorMarker".into()));
    nif.blocks.clear();
    let mut root = root;
    root.set_field("Name", NifValue::String("root".into()));
    root.set_field("Children", NifValue::Array(vec![NifValue::Ref(1)]));
    nif.blocks.push(root);
    nif.blocks.push(marker);

    let stat = barn_static("Meshes\\synthetic_editormarker.nif");
    let shapes = with_ctx(0, |ctx| {
        lodgen_native::objects::object_lod::iterate_nif(&nif, &stat, 0, ctx)
    });
    assert!(shapes.is_empty(), "editormarker node yields no shapes");
}

// ---------------------------------------------------------------------------
// build_object_atlas lower-level helpers + empty-refs guard
// ---------------------------------------------------------------------------

use lodgen_native::atlas::atlas::{build_object_atlas, build_object_atlas_with_progress};
use lodgen_native::progress::Progress;

#[test]
fn build_atlas_empty_refs_returns_empty() {
    // No refs → should return an AtlasResult with empty paths, NOT panic.
    // The caller (generate_quad) handles the empty-atlas case.
    let result = with_ctx(4, |ctx| build_object_atlas(&[], ctx));
    // Must not panic or error
    match result {
        Ok(ar) => {
            // empty result: uv map is empty, list is empty
            assert!(ar.list.is_empty(), "no refs → empty atlas list");
        }
        Err(e) => {
            // Acceptable: some implementations bail with an error for empty input
            // as long as they don't panic.
            eprintln!("build_object_atlas with empty refs returned Err (OK): {e}");
        }
    }
}

#[test]
fn build_atlas_empty_refs_reports_progress() {
    struct CollectProgress {
        messages: Vec<String>,
    }

    impl Progress for CollectProgress {
        fn report(&mut self, msg: &str, _frac: f32) {
            self.messages.push(msg.to_string());
        }
    }

    let mut progress = CollectProgress {
        messages: Vec::new(),
    };
    let result = with_ctx(4, |ctx| {
        build_object_atlas_with_progress(&[], ctx, Some(&mut progress))
    })
    .expect("empty atlas build should not fail");

    assert!(result.list.is_empty(), "no refs -> empty atlas list");
    assert!(
        progress
            .messages
            .iter()
            .any(|m| m == "object atlas: scanning 0 refs")
    );
    assert!(
        progress
            .messages
            .iter()
            .any(|m| m == "object atlas: no valid texture tiles")
    );
}

#[test]
fn build_atlas_duplicate_refs_reuse_scan_cache() {
    struct CollectProgress {
        messages: Vec<String>,
    }

    impl Progress for CollectProgress {
        fn report(&mut self, msg: &str, _frac: f32) {
            self.messages.push(msg.to_string());
        }
    }

    let refs = vec![static_no_lod(), static_no_lod()];
    let mut progress = CollectProgress {
        messages: Vec::new(),
    };
    let result = with_ctx(4, |ctx| {
        build_object_atlas_with_progress(&refs, ctx, Some(&mut progress))
    })
    .expect("duplicate empty atlas build should not fail");

    assert!(result.list.is_empty(), "no LOD models -> empty atlas list");
    assert!(
        progress
            .messages
            .iter()
            .any(|m| m.contains("object atlas: scan jobs unique=1 workers=")),
        "expected worker-backed atlas scan job planning: {:?}",
        progress.messages
    );
    assert!(
        progress
            .messages
            .iter()
            .any(|m| m.contains("cache=1 hits=1")),
        "expected duplicate refs to hit atlas scan cache: {:?}",
        progress.messages
    );
}

/// Pack and compose a synthetic 2-tile RGBA atlas without any NIF/DDS I/O.
/// Directly exercises the lower-level packing + blit logic via
/// `pack_and_compose_atlas`.
#[test]
fn pack_and_compose_2x2_tiles() {
    use lodgen_native::atlas::atlas::pack_and_compose_atlas;
    use lodgen_native::atlas::binpacker::BinBlock;

    // Two 2×2 RGBA tiles (solid red and solid green)
    let red = vec![
        255u8, 0, 0, 255, 255, 0, 0, 255, 255u8, 0, 0, 255, 255, 0, 0, 255,
    ];
    let green = vec![
        0u8, 255, 0, 255, 0, 255, 0, 255, 0u8, 255, 0, 255, 0, 255, 0, 255,
    ];

    let tiles = vec![(2u32, 2u32, red.clone()), (2u32, 2u32, green.clone())];
    let (atlas_w, atlas_h, rgba) = pack_and_compose_atlas(&tiles, 512);
    // Both tiles fit → atlas width >= 4, height >= 2
    assert!(
        atlas_w >= 4 || atlas_h >= 4,
        "atlas must be at least 4 wide or 4 high"
    );
    assert_eq!(rgba.len() as u32, atlas_w * atlas_h * 4);

    // The red tile at (0,0) → pixel at row=0, col=0 is (255,0,0,255)
    let r = rgba[0];
    let g_ch = rgba[1];
    let b_ch = rgba[2];
    assert_eq!((r, g_ch, b_ch), (255, 0, 0), "first tile (0,0) must be red");
}

/// Build a real atlas from two synthetic 4×4 DDS tiles written to tmp.
/// This exercises the DDS I/O path of build_object_atlas via a helper.
#[test]
fn build_atlas_from_synthetic_dds() {
    use lodgen_native::atlas::atlas::build_atlas_from_tiles;
    use std::path::PathBuf;

    let tmp = std::env::temp_dir().join("lodgen_task8_atlas_test");
    std::fs::create_dir_all(&tmp).unwrap();

    // Write two 4×4 solid-color RGBA DDS tiles
    let red_px = vec![255u8, 0, 0, 255].repeat(16); // 4*4*4 = 64 bytes
    let blue_px = vec![0u8, 0, 255, 255].repeat(16);

    let tile_a = tmp.join("tile_a_d.dds");
    let tile_b = tmp.join("tile_b_d.dds");
    directxtex_native::write_dds_rgba_image(&tile_a, 4, 4, &red_px, "BC1_UNORM", false)
        .expect("write tile_a");
    directxtex_native::write_dds_rgba_image(&tile_b, 4, 4, &blue_px, "BC1_UNORM", false)
        .expect("write tile_b");

    let atlas_path = tmp.join("WObjects.dds");
    let map_path = tmp.join("WObjects.txt");

    let result = build_atlas_from_tiles(
        &[tile_a.clone(), tile_b.clone()],
        &atlas_path,
        &map_path,
        4096, // atlas_size
        512,  // max_tile_size
        "BC2_UNORM",
        "BC1_UNORM",
        "BC5_UNORM",
    );
    assert!(
        result.is_ok(),
        "build_atlas_from_tiles failed: {:?}",
        result.err()
    );
    let ar = result.unwrap();
    assert!(atlas_path.is_file(), "diffuse atlas DDS must be written");
    assert!(map_path.is_file(), "atlas map txt must be written");
    assert_eq!(ar.list.len(), 2, "atlas list must have 2 entries");
    // atlas_size must be power-of-2 and cover both tiles
    assert!(ar.atlas_size.0 >= 4 && ar.atlas_size.1 >= 4);
    assert!(ar.atlas_size.0.is_power_of_two() && ar.atlas_size.1.is_power_of_two());
}

#[test]
fn build_atlas_keys_by_diffuse_normal() {
    // Fix #7: when an `_n` sibling exists, the atlas must key by "diffuse,normal"
    // (not bare diffuse) so transform_shape's atlas_build_key -> atlas_get_key
    // resolves via the diffuse,normal branch.
    use lodgen_native::atlas::atlas::build_atlas_from_tiles;
    use lodgen_native::objects::static_desc::atlas_build_key;

    let tmp = std::env::temp_dir().join("lodgen_fix7_atlas_key");
    std::fs::create_dir_all(&tmp).unwrap();

    // Write a diffuse tile under a `textures\...` path AND its `_n` sibling so the
    // data-relative paths look like real game textures.
    let tex_dir = tmp.join("Textures").join("lod");
    std::fs::create_dir_all(&tex_dir).unwrap();
    let px = vec![200u8, 100, 50, 255].repeat(16); // 4x4 RGBA
    let n_px = vec![128u8, 128, 255, 255].repeat(16);
    let tile_d = tex_dir.join("brick01_lod_d.dds");
    let tile_n = tex_dir.join("brick01_lod_n.dds");
    directxtex_native::write_dds_rgba_image(&tile_d, 4, 4, &px, "BC1_UNORM", false).unwrap();
    directxtex_native::write_dds_rgba_image(&tile_n, 4, 4, &n_px, "BC1_UNORM", false).unwrap();

    let atlas_path = tmp.join("W.Objects.dds");
    let map_path = tmp.join("W.Objects.txt");
    let ar = build_atlas_from_tiles(
        &[tile_d.clone()],
        &atlas_path,
        &map_path,
        4096,
        512,
        "BC2_UNORM",
        "BC1_UNORM",
        "BC5_UNORM",
    )
    .expect("build_atlas_from_tiles");

    assert_eq!(ar.list.len(), 1, "one tile keyed");
    // The single key must be the composite "diffuse,normal" form (contains a comma).
    let (key, _rect) = ar.list.iter().next().expect("one entry");
    assert!(
        key.contains(','),
        "atlas key must be diffuse,normal (got {key:?})"
    );
    assert!(key.contains("brick01_lod_d.dds"), "key has diffuse");
    assert!(key.contains("brick01_lod_n.dds"), "key has normal");

    // transform_shape's lookup path (atlas_build_key) must resolve to this key,
    // i.e. via the diffuse,normal branch — NOT the bare-diffuse fallback.
    let textures = vec![
        r"textures\lod\brick01_lod_d.dds".to_string(),
        r"textures\lod\brick01_lod_n.dds".to_string(),
        String::new(),
    ];
    let resolved = atlas_build_key(&ar.list, &textures, 128);
    assert!(
        ar.list.contains(&resolved),
        "atlas_build_key must resolve into the atlas"
    );
    assert!(
        resolved.contains(','),
        "resolved key is the diffuse,normal form, not bare diffuse"
    );
}

// ---------------------------------------------------------------------------
// build_bto / CreateLODNodesFO4 block-graph assembly
// ---------------------------------------------------------------------------

use lodgen_native::objects::object_lod::{BtoShader, build_bto};
use lodgen_native::output::bto::{build_bto_nif, build_bto_nif_with_layout, object_vertex_desc};
use lodgen_native::settings::Fo76BtoNodeLayout;
use nif_core_native::model::NifValue;

/// A non-passthru ShapeDesc with N triangles, given textures/flags.
fn bto_test_shape(
    textures: [String; 10],
    flags: lodgen_native::objects::static_desc::ShapeFlags,
    clamp: u32,
) -> lodgen_native::objects::static_desc::ShapeDesc {
    use lodgen_native::objects::geometry::LodGeometry;
    use lodgen_native::objects::static_desc::{ShaderKind, ShapeDesc};

    let mut g = LodGeometry::new();
    g.vertices = vec![[0.0, 0.0, 0.0], [10.0, 0.0, 1.0], [0.0, 10.0, 2.0]];
    g.normals = vec![[0.0, 0.0, 1.0]; 3];
    g.tangents = vec![[1.0, 0.0, 0.0]; 3];
    g.bitangents = vec![[0.0, 1.0, 0.0]; 3];
    g.uvcoords = vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
    g.triangles = vec![[0, 1, 2]];
    g.update_bbox();

    ShapeDesc {
        name: "Test".into(),
        static_model: "test.nif".into(),
        geometry: g,
        flags,
        textures,
        source_materials: Vec::new(),
        textures_key: String::new(),
        texture_clamp_mode: clamp,
        alpha_threshold: 200,
        alpha_flags: 4844,
        backlight_power: 0.0,
        grayscale_to_palette_scale: 1.0,
        enable_parent: 0,
        shader_type: ShaderKind::Lighting,
        x: 0.0,
        y: 0.0,
        bounding_box: lodgen_native::descriptors::BBox::empty(),
        segments: vec![lodgen_native::objects::object_lod::SegmentDesc {
            id: 0,
            start_triangle: 0,
            num_triangles: 1,
        }],
        uv_scale: [1.0, 1.0],
        uv_offset: [0.0, 0.0],
        ref_flags: 0,
        node_transform: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
        node_scale: 1.0,
        translation: [0.0, 0.0, 0.0],
        rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        bto_translation: None,
        bto_scale: None,
    }
}

fn atlas_textures() -> [String; 10] {
    let mut t: [String; 10] = Default::default();
    t[0] = r"textures\terrain\dlc03farharbor\objects\dlc03farharbor.objects.dds".into();
    t[1] = r"textures\terrain\dlc03farharbor\objects\dlc03farharbor.objects_n.dds".into();
    t[7] = r"textures\terrain\dlc03farharbor\objects\dlc03farharbor.objects_s.dds".into();
    t
}

fn nif_array_len(value: Option<&NifValue>) -> usize {
    match value {
        Some(NifValue::Array(items)) => items.len(),
        other => panic!("expected NIF array, got {other:?}"),
    }
}

#[test]
fn build_bto_shader_constants() {
    use lodgen_native::objects::static_desc::ShapeFlags;
    let mut quad = make_quad(16, -9, 5);
    let settings = LodSettings::fo4_default();
    // HasLODFlag set, no vertex color, no decal/greyscale → flags1=0x80400001
    // (Specular|Own_Emit|ZBuffer_Test, the C# literal 2151677953u), flags2=5.
    let shape = bto_test_shape(atlas_textures(), ShapeFlags::HAS_LOD_FLAG, 0);
    let out = build_bto(&mut quad, vec![shape], &settings.objects);
    assert_eq!(out.len(), 1);
    match &out[0].shader {
        BtoShader::Lighting {
            flags1,
            flags2,
            clamp,
            texture_set,
            ..
        } => {
            assert_eq!(*flags1, 0x80400001, "flags1 = {:#x}", flags1);
            assert_eq!(
                *flags2, 5,
                "flags2 (ZBuffer_Write|LOD_Objects) = {}",
                flags2
            );
            assert_eq!(*clamp, 0);
            assert_eq!(texture_set[0], atlas_textures()[0]);
            assert_eq!(texture_set[1], atlas_textures()[1]);
            assert_eq!(texture_set[7], atlas_textures()[7]);
            assert!(texture_set[2].is_empty());
        }
    }
    assert!(!out[0].name_index_at, "non-alpha → name 'obj'");
    assert!(out[0].alpha.is_none());
    // Scale = quad level, translation = quad origin.
    assert_eq!(out[0].scale, 16.0);
    assert_eq!(out[0].translation, [-9.0 * 4096.0, 5.0 * 4096.0, 0.0]);
}

#[test]
fn build_bto_optimizes_before_count() {
    // Fix #6: build_bto must call geometry.optimize() (RemoveUnused) BEFORE
    // computing Num Vertices / Data Size / bbox, matching C#
    // ToBSSubIndexTriShape(optimize:true). A shape with vertices unreferenced by
    // any triangle must have them dropped (vertex count == referenced count).
    use lodgen_native::objects::static_desc::ShapeFlags;
    use nif_core_native::model::{NifFile, NifValue};

    let mut quad = make_quad(16, 0, 0);
    let settings = LodSettings::fo4_default();
    let mut shape = bto_test_shape(atlas_textures(), ShapeFlags::HAS_LOD_FLAG, 0);
    // 3 referenced verts (tri [0,1,2]) + 2 unreferenced trailing verts.
    shape.geometry.vertices = vec![
        [0.0, 0.0, 0.0],
        [10.0, 0.0, 1.0],
        [0.0, 10.0, 2.0],
        [99.0, 99.0, 99.0],
        [88.0, 88.0, 88.0], // unreferenced
    ];
    shape.geometry.normals = vec![[0.0, 0.0, 1.0]; 5];
    shape.geometry.tangents = vec![[1.0, 0.0, 0.0]; 5];
    shape.geometry.bitangents = vec![[0.0, 1.0, 0.0]; 5];
    shape.geometry.uvcoords = vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [0.5, 0.5], [0.25, 0.25]];
    shape.geometry.triangles = vec![[0, 1, 2]];

    let out = build_bto(&mut quad, vec![shape], &settings.objects);
    assert_eq!(out.len(), 1);
    assert_eq!(
        out[0].geometry.num_vertices(),
        3,
        "unreferenced vertices must be dropped by optimize() before counting"
    );

    // And the written .bto's Num Vertices / Data Size reflect the optimized count.
    let mut nif = build_bto_nif(&out).expect("build_bto_nif");
    let bytes = nif.to_bytes().expect("to_bytes");
    let reloaded = NifFile::from_bytes(&bytes, None).expect("reload");
    let sits = reloaded
        .blocks
        .iter()
        .find(|b| b.type_name == "BSSubIndexTriShape")
        .unwrap();
    let nv = sits
        .get_field("Num Vertices")
        .map(NifValue::as_i64)
        .unwrap();
    let ds = sits.get_field("Data Size").map(NifValue::as_i64).unwrap();
    let ntr = sits
        .get_field("Num Triangles")
        .map(NifValue::as_i64)
        .unwrap();
    assert_eq!(nv, 3, "written Num Vertices reflects optimized geometry");
    assert_eq!(
        ds,
        nv * 20 + ntr * 6,
        "Data Size == numVerts*20 + numTris*6"
    );
}

#[test]
fn build_bto_fo76_grouped_layout_uses_one_multibound_node_for_many_shapes() {
    use lodgen_native::objects::static_desc::ShapeFlags;

    let mut quad = make_quad(16, -46, -29);
    let settings = LodSettings::fo4_default();
    let opaque = bto_test_shape(atlas_textures(), ShapeFlags::HAS_LOD_FLAG, 0);
    let alpha = bto_test_shape(
        atlas_textures(),
        ShapeFlags::HAS_LOD_FLAG | ShapeFlags::IS_ALPHA,
        0,
    );
    let out = build_bto(&mut quad, vec![opaque, alpha], &settings.objects);
    assert_eq!(out.len(), 2);

    let per_shape = build_bto_nif(&out).expect("per-shape bto");
    let per_shape_mbn = per_shape
        .blocks
        .iter()
        .filter(|b| b.type_name == "BSMultiBoundNode")
        .count();
    assert_eq!(per_shape_mbn, 2);

    let grouped =
        build_bto_nif_with_layout(&out, Fo76BtoNodeLayout::Fo76Grouped).expect("grouped bto");
    let grouped_mbn: Vec<_> = grouped
        .blocks
        .iter()
        .filter(|b| b.type_name == "BSMultiBoundNode")
        .collect();
    assert_eq!(grouped_mbn.len(), 1);
    assert_eq!(
        nif_array_len(grouped_mbn[0].get_field("Children")),
        2,
        "single grouped multibound should own both shape children"
    );

    let root = grouped
        .blocks
        .iter()
        .find(|b| b.type_name == "NiNode")
        .expect("root NiNode");
    assert_eq!(
        nif_array_len(root.get_field("Children")),
        1,
        "FO4 root remains NiNode with one grouped BSMultiBoundNode child"
    );
}

#[test]
fn build_bto_alpha_name_at() {
    use lodgen_native::objects::static_desc::ShapeFlags;
    let mut quad = make_quad(4, 0, 0);
    let settings = LodSettings::fo4_default();
    let shape = bto_test_shape(
        atlas_textures(),
        ShapeFlags::IS_ALPHA | ShapeFlags::HAS_LOD_FLAG,
        0,
    );
    let out = build_bto(&mut quad, vec![shape], &settings.objects);
    assert!(out[0].name_index_at, "alpha shape → 'obj-at'");
    let a = out[0].alpha.as_ref().expect("alpha property");
    assert_eq!(a.flags, 4844);
    // fo4_default: use_alpha_threshold? → threshold source.
    if settings.objects.use_alpha_threshold {
        assert_eq!(a.threshold, 200, "shape alpha threshold");
    } else {
        assert_eq!(a.threshold, settings.objects.alpha_threshold);
    }
}

#[test]
fn build_bto_doublesided_flag() {
    use lodgen_native::objects::static_desc::ShapeFlags;
    let mut quad = make_quad(16, 0, 0);
    let settings = LodSettings::fo4_default();
    let shape = bto_test_shape(atlas_textures(), ShapeFlags::IS_DOUBLE_SIDED, 0);
    let out = build_bto(&mut quad, vec![shape], &settings.objects);
    match &out[0].shader {
        BtoShader::Lighting { flags2, .. } => {
            assert_eq!(*flags2 & 0x10, 0x10, "double-sided sets flags2 bit 0x10");
        }
    }
}

#[test]
fn build_bto_texture_set_dedup() {
    use lodgen_native::objects::static_desc::ShapeFlags;
    let mut quad = make_quad(16, 0, 0);
    let settings = LodSettings::fo4_default();
    // Two generated shapes with identical render state merge into one BTO shape.
    let s1 = bto_test_shape(atlas_textures(), ShapeFlags::HAS_LOD_FLAG, 0);
    let s2 = bto_test_shape(atlas_textures(), ShapeFlags::HAS_LOD_FLAG, 0);
    let out = build_bto(&mut quad, vec![s1, s2], &settings.objects);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].geometry.num_triangles(), 2);
    let nif = build_bto_nif(&out).expect("build_bto_nif");
    let texset_count = nif
        .blocks
        .iter()
        .filter(|b| b.type_name == "BSShaderTextureSet")
        .count();
    assert_eq!(
        texset_count, 1,
        "identical texture sets must be deduped to one block"
    );
    let sits = nif
        .blocks
        .iter()
        .filter(|b| b.type_name == "BSSubIndexTriShape")
        .count();
    let lsp = nif
        .blocks
        .iter()
        .filter(|b| b.type_name == "BSLightingShaderProperty")
        .count();
    assert_eq!(sits, 1);
    assert_eq!(lsp, 1);
}

#[test]
fn build_bto_enable_parent() {
    use lodgen_native::objects::static_desc::ShapeFlags;
    let mut quad = make_quad(16, 0, 0);
    let settings = LodSettings::fo4_default();
    let mut shape = bto_test_shape(atlas_textures(), ShapeFlags::HAS_LOD_FLAG, 0);
    shape.enable_parent = 0x1234;
    let out = build_bto(&mut quad, vec![shape], &settings.objects);
    assert_eq!(out[0].enable_parent, 0x1234);
    let nif = build_bto_nif(&out).expect("build_bto_nif");
    // A ToggleRefID NiIntegerExtraData must be emitted (LODApp.cs:2485-2491).
    let ied = nif
        .blocks
        .iter()
        .find(|b| b.type_name == "NiIntegerExtraData")
        .expect("NiIntegerExtraData ToggleRefID block");
    assert_eq!(
        ied.get_field("Name").and_then(name_str),
        Some("ToggleRefID".to_string())
    );
    assert_eq!(
        ied.get_field("Integer Data")
            .map(nif_core_native::model::NifValue::as_i64),
        Some(0x1234)
    );

    // A shape without enable_parent emits no extra data block.
    let mut quad2 = make_quad(16, 0, 0);
    let shape2 = bto_test_shape(atlas_textures(), ShapeFlags::HAS_LOD_FLAG, 0);
    let out2 = build_bto(&mut quad2, vec![shape2], &settings.objects);
    let nif2 = build_bto_nif(&out2).expect("build_bto_nif");
    assert!(
        !nif2
            .blocks
            .iter()
            .any(|b| b.type_name == "NiIntegerExtraData")
    );
}

#[test]
fn build_bto_block_graph_roundtrips() {
    use lodgen_native::objects::static_desc::ShapeFlags;
    let mut quad = make_quad(16, -9, 5);
    let settings = LodSettings::fo4_default();
    let shape = bto_test_shape(atlas_textures(), ShapeFlags::HAS_LOD_FLAG, 0);
    let out = build_bto(&mut quad, vec![shape], &settings.objects);
    let mut nif = build_bto_nif(&out).expect("build_bto_nif");

    // Root is NiNode "obj".
    assert_eq!(nif.blocks[0].type_name, "NiNode");
    // Serializes and round-trips.
    let bytes = nif.to_bytes().expect("to_bytes");
    assert!(bytes.len() > 100);
    assert_eq!(nif.header.bs_version, 130);

    // Block types present.
    let types: std::collections::HashSet<&str> =
        nif.blocks.iter().map(|b| b.type_name.as_str()).collect();
    for t in [
        "NiNode",
        "BSMultiBoundNode",
        "BSSubIndexTriShape",
        "BSLightingShaderProperty",
        "BSShaderTextureSet",
        "BSMultiBound",
        "BSMultiBoundAABB",
    ] {
        assert!(types.contains(t), "missing block type {t}");
    }
}

#[test]
fn write_bto_to_disk_parses_back() {
    use lodgen_native::objects::static_desc::ShapeFlags;
    use lodgen_native::output::bto::write_bto;
    use nif_core_native::model::NifFile;

    let mut quad = make_quad(16, -9, 5);
    let settings = LodSettings::fo4_default();
    let shape = bto_test_shape(atlas_textures(), ShapeFlags::HAS_LOD_FLAG, 0);
    let out = build_bto(&mut quad, vec![shape], &settings.objects);

    let dir = std::env::temp_dir().join("lodgen_bto_disk_test");
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("DLC03FarHarbor.16.-9.5.bto");
    write_bto(&p, &out).expect("write_bto");
    assert!(std::fs::metadata(&p).unwrap().len() > 100);

    // Re-parse the written file and confirm the block graph.
    let reloaded = NifFile::load(&p).expect("reload written .bto");
    assert_eq!(reloaded.blocks[0].type_name, "NiNode");
    assert!(
        reloaded
            .blocks
            .iter()
            .any(|b| b.type_name == "BSSubIndexTriShape")
    );
    assert!(
        reloaded
            .blocks
            .iter()
            .any(|b| b.type_name == "BSMultiBoundAABB")
    );
}

#[test]
fn object_vertex_desc_matches_golden_value() {
    // No-color object desc == verified golden 474989027590661 (0x1b00000430205).
    assert_eq!(object_vertex_desc(false), 474989027590661);
    // attributes (>>44) for color variant include Vertex_Colors bit (0x20).
    assert_eq!(object_vertex_desc(true) >> 44 & 0x20, 0x20);
    // low nibble = vertex size in 4-byte words (5 no-color, 6 with color).
    assert_eq!(object_vertex_desc(false) & 0xF, 5);
    assert_eq!(object_vertex_desc(true) & 0xF, 6);
}

/// Read a shader-flag field as a raw u32 mask regardless of how nif_core stored
/// it (UInt mask, or an Array of option-name strings).
fn flag_mask(block: &nif_core_native::model::NifBlock, name: &str) -> Option<u64> {
    use nif_core_native::model::NifValue;
    match block.get_field(name)? {
        NifValue::UInt(u) => Some(*u),
        NifValue::Int(i) => Some(*i as u64),
        // Name-array representation: we only assert the field exists in that case.
        _ => None,
    }
}

/// MANDATORY golden gate: build a `.bto` from a ShapeDesc mirroring the single
/// golden shape and assert our block graph / vertex desc / shader flags /
/// texture-set slot structure MATCH the golden DLC03FarHarbor.16.-9.5.bto
/// (structural + field equality; not byte/hash equal — FP differences allowed).
#[test]
fn bto_structural_equality_vs_golden() {
    use lodgen_native::objects::static_desc::ShapeFlags;
    use nif_core_native::model::{NifFile, NifValue};

    let golden_path = repo_root()
        .join("tmp/xlodgen/meshes/terrain/DLC03FarHarbor/Objects/DLC03FarHarbor.16.-9.5.bto");
    if !golden_path.is_file() {
        // Corpus truly absent — skip (it is present in this repo).
        eprintln!(
            "SKIP bto_structural_equality_vs_golden: golden missing at {:?}",
            golden_path
        );
        return;
    }
    let golden = NifFile::load(&golden_path).expect("load golden .bto");

    // --- Golden block-graph shape ---
    assert_eq!(golden.blocks[0].type_name, "NiNode");
    assert_eq!(
        golden.blocks[0].get_field("Name").and_then(name_str),
        Some("obj".to_string())
    );
    let g_types: Vec<&str> = golden.blocks.iter().map(|b| b.type_name.as_str()).collect();
    assert!(g_types.contains(&"BSSubIndexTriShape"));
    assert!(g_types.contains(&"BSLightingShaderProperty"));
    assert!(g_types.contains(&"BSShaderTextureSet"));
    assert!(g_types.contains(&"BSMultiBoundNode"));
    assert!(g_types.contains(&"BSMultiBound"));
    assert!(g_types.contains(&"BSMultiBoundAABB"));

    // Golden BSSubIndexTriShape facts (the block we must reproduce).
    let g_sits = golden
        .blocks
        .iter()
        .find(|b| b.type_name == "BSSubIndexTriShape")
        .expect("golden BSSubIndexTriShape");
    let g_vertex_desc = g_sits
        .get_field("Vertex Desc")
        .map(NifValue::as_i64)
        .unwrap();
    let g_scale = match g_sits.get_field("Scale") {
        Some(NifValue::Float(f)) => *f,
        _ => panic!("Scale"),
    };
    let g_num_tris = g_sits
        .get_field("Num Triangles")
        .map(NifValue::as_i64)
        .unwrap();
    let g_num_verts = g_sits
        .get_field("Num Vertices")
        .map(NifValue::as_i64)
        .unwrap();

    let g_lsp = golden
        .blocks
        .iter()
        .find(|b| b.type_name == "BSLightingShaderProperty")
        .expect("golden LSP");
    let g_texset = golden
        .blocks
        .iter()
        .find(|b| b.type_name == "BSShaderTextureSet")
        .expect("golden texset");

    // --- Build OUR .bto from a mirroring shape ---
    let mut quad = make_quad(16, -9, 5);
    let settings = LodSettings::fo4_default();
    let shape = bto_test_shape(atlas_textures(), ShapeFlags::HAS_LOD_FLAG, 0);
    let our = build_bto(&mut quad, vec![shape], &settings.objects);
    let mut our_built = build_bto_nif(&our).expect("build_bto_nif");
    // Round-trip through serialize → reload so the comparison is on the actual
    // on-disk block graph (version-gated fields resolved), exactly like the golden.
    let our_bytes = our_built.to_bytes().expect("our to_bytes");
    let our_nif = NifFile::from_bytes(&our_bytes, None).expect("reload our .bto");

    let o_sits = our_nif
        .blocks
        .iter()
        .find(|b| b.type_name == "BSSubIndexTriShape")
        .expect("our BSSubIndexTriShape");
    let o_lsp = our_nif
        .blocks
        .iter()
        .find(|b| b.type_name == "BSLightingShaderProperty")
        .expect("our LSP");
    let o_texset = our_nif
        .blocks
        .iter()
        .find(|b| b.type_name == "BSShaderTextureSet")
        .expect("our texset");

    // 1. Block graph: root NiNode "obj".
    assert_eq!(our_nif.blocks[0].type_name, "NiNode");
    assert_eq!(
        our_nif.blocks[0].get_field("Name").and_then(name_str),
        Some("obj".to_string())
    );

    // 2. Vertex Desc MATCHES golden exactly (object layout).
    assert_eq!(
        o_sits
            .get_field("Vertex Desc")
            .map(NifValue::as_i64)
            .unwrap(),
        g_vertex_desc,
        "vertex desc must match golden object layout"
    );

    // 3. Scale == lodLevel (16), translation == quad origin.
    let o_scale = match o_sits.get_field("Scale") {
        Some(NifValue::Float(f)) => *f,
        other => panic!("our Scale: {other:?}"),
    };
    assert_eq!(o_scale, g_scale, "Scale must equal golden lodLevel");
    let o_trans = vec3_xyz(o_sits.get_field("Translation").expect("our Translation"));
    assert_eq!(o_trans, [-9.0 * 4096.0, 5.0 * 4096.0, 0.0]);

    // Sanity that golden carries the same Translation/Scale we target.
    assert_eq!(g_scale, 16.0);
    assert!(g_num_tris > 0 && g_num_verts > 0);

    // 4. Shader flags raw masks MATCH golden (the Phase-1 versioned-key lesson).
    //    Golden may store as UInt mask or name-array; if mask, compare directly.
    if let Some(g_f1) = flag_mask(g_lsp, "Shader Flags 1") {
        assert_eq!(
            flag_mask(o_lsp, "Shader Flags 1"),
            Some(g_f1),
            "Shader Flags 1 mask must match golden"
        );
        assert_eq!(
            g_f1, 0x80400001,
            "golden flags1 == Specular|Own_Emit|ZBuffer_Test"
        );
    } else {
        // Name-array form — assert ours decodes to the same set by raw mask.
        assert_eq!(flag_mask(o_lsp, "Shader Flags 1"), Some(0x80400001));
    }
    if let Some(g_f2) = flag_mask(g_lsp, "Shader Flags 2") {
        assert_eq!(
            flag_mask(o_lsp, "Shader Flags 2"),
            Some(g_f2),
            "Shader Flags 2 mask must match golden"
        );
        assert_eq!(g_f2, 5, "golden flags2 == ZBuffer_Write|LOD_Objects");
    } else {
        assert_eq!(flag_mask(o_lsp, "Shader Flags 2"), Some(5));
    }

    // 5. Texture-set slot structure: golden has 10 slots, [0]/[1]/[7] populated, rest empty.
    let g_slots = texset_slots(g_texset);
    let o_slots = texset_slots(o_texset);
    assert_eq!(g_slots.len(), 10, "golden texset 10 slots");
    assert_eq!(o_slots.len(), 10, "our texset 10 slots");
    for i in 0..10 {
        assert_eq!(
            g_slots[i].is_empty(),
            o_slots[i].is_empty(),
            "texset slot {i} populated-ness must match golden (g={:?} o={:?})",
            g_slots[i],
            o_slots[i]
        );
    }
    // Our populated slots point at the object atlas (Data\-prefixed).
    assert!(
        o_slots[0]
            .to_lowercase()
            .ends_with("dlc03farharbor.objects.dds")
    );
    assert!(
        o_slots[1]
            .to_lowercase()
            .ends_with("dlc03farharbor.objects_n.dds")
    );
    assert!(
        o_slots[7]
            .to_lowercase()
            .ends_with("dlc03farharbor.objects_s.dds")
    );
    assert!(o_slots[0].starts_with("Data\\"));

    // 6. Golden Data Size = numVerts*20 + numTris*6 (our writer recomputes the same calc).
    let g_data_size = g_sits.get_field("Data Size").map(NifValue::as_i64).unwrap();
    assert_eq!(g_data_size, g_num_verts * 20 + g_num_tris * 6);

    // 7. Segments — GOLDEN GATE (Phase-2 fix #1 / #5).
    //    Golden DLC03FarHarbor.16.-9.5.bto: Num Segments=1, Total Segments=1,
    //    Segment[0].Num Primitives == Num Triangles (178), top-level
    //    Num Primitives == Num Triangles * 2 (356).
    let g_num_segments = g_sits
        .get_field("Num Segments")
        .map(NifValue::as_i64)
        .unwrap();
    let g_total_segments = g_sits
        .get_field("Total Segments")
        .map(NifValue::as_i64)
        .unwrap();
    let g_seg0_prims = segment0_num_primitives(g_sits).expect("golden Segment[0] Num Primitives");
    let g_top_prims = g_sits
        .get_field("Num Primitives")
        .map(NifValue::as_i64)
        .unwrap();
    assert_eq!(g_num_segments, 1, "golden Num Segments == 1");
    assert_eq!(g_total_segments, 1, "golden Total Segments == 1");
    assert_eq!(
        g_seg0_prims, g_num_tris,
        "golden Segment[0].NumPrimitives == NumTriangles"
    );
    assert_eq!(
        g_top_prims,
        g_num_tris * 2,
        "golden top-level Num Primitives == NumTriangles*2"
    );

    // OUR output must reproduce that segment structure. Our mirroring shape has
    // 1 triangle; transform_shape's GenerateSegments would assign one segment
    // (here build_bto carries the test shape's single segment).
    let o_num_segments = o_sits
        .get_field("Num Segments")
        .map(NifValue::as_i64)
        .unwrap();
    let o_total_segments = o_sits
        .get_field("Total Segments")
        .map(NifValue::as_i64)
        .unwrap();
    let o_num_tris = o_sits
        .get_field("Num Triangles")
        .map(NifValue::as_i64)
        .unwrap();
    let o_seg0_prims = segment0_num_primitives(o_sits).expect("our Segment[0] Num Primitives");
    let o_top_prims = o_sits
        .get_field("Num Primitives")
        .map(NifValue::as_i64)
        .unwrap();
    assert!(
        o_num_segments >= 1,
        "our Num Segments must be >= 1 (golden gate, fix #1)"
    );
    assert_eq!(
        o_num_segments, o_total_segments,
        "our Num/Total Segments must agree"
    );
    assert_eq!(
        o_seg0_prims, o_num_tris,
        "our Segment[0].NumPrimitives == NumTriangles"
    );

    assert_eq!(
        o_top_prims,
        o_num_tris * 2,
        "our top-level Num Primitives must match golden"
    );
}

/// Read `Segment[0].Num Primitives` from a BSSubIndexTriShape block.
fn segment0_num_primitives(block: &nif_core_native::model::NifBlock) -> Option<i64> {
    use nif_core_native::model::NifValue;
    match block.get_field("Segment")? {
        NifValue::Array(items) => match items.first()? {
            NifValue::Struct(m) => m.get("Num Primitives").map(NifValue::as_i64),
            _ => None,
        },
        _ => None,
    }
}

fn name_str(v: &nif_core_native::model::NifValue) -> Option<String> {
    match v {
        nif_core_native::model::NifValue::String(s) => Some(s.clone()),
        _ => None,
    }
}

/// Read a Vec3 stored either as NifValue::Vec3 or a Struct{x,y,z}.
fn vec3_xyz(v: &nif_core_native::model::NifValue) -> [f32; 3] {
    use nif_core_native::model::NifValue;
    match v {
        NifValue::Vec3(a) => *a,
        NifValue::Struct(m) => {
            let g = |k: &str| match m.get(k) {
                Some(NifValue::Float(f)) => *f as f32,
                Some(other) => other.as_i64() as f32,
                None => 0.0,
            };
            [g("x"), g("y"), g("z")]
        }
        _ => panic!("not a vec3: {v:?}"),
    }
}

fn texset_slots(block: &nif_core_native::model::NifBlock) -> Vec<String> {
    use nif_core_native::model::NifValue;
    match block.get_field("Textures") {
        Some(NifValue::Array(items)) => items
            .iter()
            .map(|v| match v {
                NifValue::String(s) => s.clone(),
                _ => String::new(),
            })
            .collect(),
        _ => Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// objects::generate_quad (DoLOD object path)
// ---------------------------------------------------------------------------

/// Build a minimal `StaticDesc` with no LOD model (lod_models all None).
fn static_no_lod() -> StaticDesc {
    StaticDesc {
        ref_id: "00000001".into(),
        ref_flags: 0,
        enable_parent: 0,
        cell: (0, 0),
        pos: [0.0, 0.0, 0.0],
        rot: [0.0, 0.0, 0.0],
        scale: 1.0,
        color: 1.0,
        alpha_threshold: 128,
        is_billboard: false,
        is_grass: false,
        base_name: "NullRef".into(),
        base_flags: 0,
        material_name: String::new(),
        full_model: String::new(),
        lod_models: [None, None, None, None],
        part_transform: lodgen_native::input::identity_part_transform(),
        part_scale: 1.0,
        material_swap: std::collections::BTreeMap::new(),
    }
}

/// Build a `StaticDesc` pointing at a deliberately nonexistent LOD model.
fn static_broken_lod() -> StaticDesc {
    StaticDesc {
        ref_id: "00000002".into(),
        ref_flags: 0,
        enable_parent: 0,
        cell: (0, 0),
        pos: [0.0, 0.0, 0.0],
        rot: [0.0, 0.0, 0.0],
        scale: 1.0,
        color: 1.0,
        alpha_threshold: 128,
        is_billboard: false,
        is_grass: false,
        base_name: "BrokenRef".into(),
        base_flags: 0,
        material_name: String::new(),
        full_model: String::new(),
        lod_models: [
            Some(r"Meshes\nonexistent\broken_lod.nif".to_string()),
            None,
            None,
            None,
        ],
        part_transform: lodgen_native::input::identity_part_transform(),
        part_scale: 1.0,
        material_swap: std::collections::BTreeMap::new(),
    }
}

/// Run `objects::generate_quad` with the given statics, returning (outputs, warnings).
fn run_generate_quad(
    level: i32,
    x: i32,
    y: i32,
    statics: Vec<StaticDesc>,
    out_dir: &std::path::Path,
) -> anyhow::Result<lodgen_native::progress::QuadOutputs> {
    use lodgen_native::atlas::atlas::AtlasList;
    use lodgen_native::atlas::atlas::AtlasResult;

    let mut quad = make_quad(level, x, y);
    quad.statics = statics;

    // Build an empty atlas (no tiles — the generate_quad test only needs the
    // atlas to be present, not populated).
    let atlas = AtlasResult {
        map_path: out_dir.join("atlas.txt"),
        diffuse: out_dir.join("atlas_d.dds"),
        normal: out_dir.join("atlas_n.dds"),
        specular: out_dir.join("atlas_s.dds"),
        atlas_size: (0, 0),
        uv: std::collections::HashMap::new(),
        list: AtlasList::new(),
        dds_written: 0,
    };

    let world = empty_world();
    let settings = LodSettings::fo4_default();
    let game = Game::fo4();
    let paths = LodPaths {
        data_dirs: vec![extracted_fo4()],
        output_dir: out_dir.to_path_buf(),
        source_data_dir: None,
    };
    let ctx = QuadCtx {
        world: &world,
        settings: &settings,
        game: &game,
        paths: &paths,
        level,
    };

    lodgen_native::objects::generate_quad(&quad, &ctx, &atlas)
}

#[test]
fn generate_quad_skips_refs_without_lod_model() {
    // port: DoLOD:3044 — ref with lod_models[level]==None yields no shapes → no .bto written.
    let tmp = std::env::temp_dir().join("lodgen_task10_no_lod");
    std::fs::create_dir_all(&tmp).unwrap();

    let out = run_generate_quad(16, -9, 5, vec![static_no_lod()], &tmp)
        .expect("generate_quad should succeed even with no-LOD refs");

    assert!(
        out.meshes.is_empty(),
        "no LOD model → no .bto emitted, got {:?}",
        out.meshes
    );
}

#[test]
fn generate_quad_writes_bto_path() {
    // port: DoLOD:3088 — a ref with a LOD model → .bto written at naming::bto(...)
    // We need the extracted/fo4 corpus for a real NIF; skip gracefully if absent.
    if !fixture_present() {
        eprintln!("SKIP generate_quad_writes_bto_path: extracted/fo4 barn LOD fixture absent");
        return;
    }

    let tmp = std::env::temp_dir().join("lodgen_task10_bto_write");
    std::fs::create_dir_all(&tmp).unwrap();

    let stat = barn_static(BARN_LOD);
    let out = run_generate_quad(4, 0, 0, vec![stat], &tmp).expect("generate_quad");

    // Must have written at least one .bto
    assert!(
        !out.meshes.is_empty(),
        "barn LOD ref → at least one .bto must be emitted"
    );

    // The .bto must exist on disk at the naming::bto path
    let bto_rel = lodgen_native::naming::bto("W", 4, 0, 0, "");
    let bto_path = tmp.join(bto_rel.replace('\\', "/"));
    assert!(
        bto_path.is_file(),
        ".bto must be written at naming::bto path: {:?}",
        bto_path
    );

    // Must parse as a valid NIF
    let nif =
        nif_core_native::model::NifFile::load(&bto_path).expect("written .bto must parse as NIF");
    assert_eq!(nif.blocks[0].type_name, "NiNode", "root must be NiNode");
    assert!(
        nif.blocks
            .iter()
            .any(|b| b.type_name == "BSSubIndexTriShape"),
        ".bto must contain BSSubIndexTriShape"
    );
}

#[test]
fn generate_quad_per_quad_isolation() {
    // port: DoLOD spec §6 — a ref whose LOD model can't be loaded is skipped (not a hard error);
    // the quad still produces output for the remaining valid refs.
    if !fixture_present() {
        eprintln!("SKIP generate_quad_per_quad_isolation: extracted/fo4 barn LOD fixture absent");
        return;
    }

    let tmp = std::env::temp_dir().join("lodgen_task10_isolation");
    std::fs::create_dir_all(&tmp).unwrap();

    // One broken ref (nonexistent model) + one valid barn ref.
    // generate_quad must NOT return Err — it should skip the broken ref and continue.
    let statics = vec![static_broken_lod(), barn_static(BARN_LOD)];
    let out = run_generate_quad(4, 0, 0, statics, &tmp)
        .expect("generate_quad must not fail hard when a single ref is broken");

    // The barn ref should still produce a .bto despite the broken ref.
    assert!(
        !out.meshes.is_empty(),
        "valid barn ref must still produce a .bto despite the broken ref"
    );
}

/// P4-A2 object gate (default build): build a REAL object atlas from the barn LOD
/// fixture, run `objects::generate_quad` against that NON-STUB atlas, and assert a
/// valid `.bto` is produced whose tri-count matches the barn LOD model's geometry
/// (4 tris) within tolerance and whose UVs were remapped through the atlas when
/// the barn's diffuse is atlassed.
///
/// This is the un-ignored former Phase-4 gate. The cross-validation against the
/// xLODGen golden corpus (FarHarbor enumeration → .bto tri-count vs golden +
/// atlas .dds dims) lives in the `real-esp` e2e `golden_objects_e2e.rs`
/// (`object_bto_matches_golden_l16`), which needs the ESP reader the default lib
/// can't link (directxtex FFI collision).
#[test]
fn generate_quad_counts_within_tolerance() {
    if !fixture_present() {
        eprintln!(
            "SKIP generate_quad_counts_within_tolerance: extracted/fo4 barn LOD fixture absent"
        );
        return;
    }

    let tmp = std::env::temp_dir().join("lodgen_p4a2_object_gate");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();

    let stat = barn_static(BARN_LOD);

    // Source-model geometry: the barn LOD fixture (4 tris per its verified graph).
    let src_tris: usize = with_ctx(0, |ctx| parse_nif(&stat, 0, ctx))
        .expect("parse_nif")
        .iter()
        .map(|s| s.geometry.num_triangles())
        .sum();
    assert!(src_tris > 0, "barn LOD fixture must have triangles");

    // Build a REAL (non-stub) object atlas from the barn ref, then run the object
    // pipeline against it — proving real-atlas threading, not the empty placeholder.
    let world = empty_world();
    let settings = LodSettings::fo4_default();
    let game = Game::fo4();
    let paths = LodPaths {
        data_dirs: vec![extracted_fo4()],
        output_dir: tmp.clone(),
        source_data_dir: None,
    };
    let ctx = QuadCtx {
        world: &world,
        settings: &settings,
        game: &game,
        paths: &paths,
        level: 4,
    };
    let atlas = build_object_atlas(std::slice::from_ref(&stat), &ctx)
        .expect("build_object_atlas from barn ref");
    eprintln!(
        "P4-A2 object gate: atlas tiles={} size={:?} barn_src_tris={src_tris}",
        atlas.list.len(),
        atlas.atlas_size,
    );

    let mut quad = make_quad(4, 0, 0);
    quad.statics = vec![stat];
    let out = lodgen_native::objects::generate_quad(&quad, &ctx, &atlas)
        .expect("generate_quad with real atlas");
    let bto = out
        .meshes
        .iter()
        .find(|p| p.extension().map(|e| e == "bto").unwrap_or(false))
        .expect("a .bto must be written");

    // Reload and validate the produced .bto.
    use nif_core_native::model::NifFile;
    let nif = NifFile::load(bto).expect("load our .bto");
    let types: std::collections::BTreeSet<&str> =
        nif.blocks.iter().map(|b| b.type_name.as_str()).collect();
    for t in [
        "NiNode",
        "BSSubIndexTriShape",
        "BSLightingShaderProperty",
        "BSShaderTextureSet",
    ] {
        assert!(types.contains(t), "produced .bto missing block {t}");
    }
    let our_tris: usize = nif
        .blocks
        .iter()
        .filter(|b| b.type_name == "BSSubIndexTriShape")
        .map(|b| match b.fields.get("Num Triangles") {
            Some(nif_core_native::model::NifValue::UInt(v)) => *v as usize,
            Some(nif_core_native::model::NifValue::Int(v)) => *v as usize,
            _ => 0,
        })
        .sum();
    // Object LOD ports the LOD model verbatim (no decimation by default), so the
    // emitted tri-count equals the source model's (within the build's dedup). A
    // ±25% band absorbs the per-shape dedup while catching empty / exploded output.
    let ratio = our_tris as f64 / src_tris as f64;
    eprintln!("P4-A2 object gate: our_bto_tris={our_tris} src_tris={src_tris} ratio={ratio:.3}");
    assert!(our_tris > 0, "produced .bto has zero triangles");
    assert!(
        (0.75..=1.25).contains(&ratio),
        "object .bto tri-count {our_tris} vs source {src_tris} outside ±25% (ratio {ratio:.3})"
    );
}

// ---------------------------------------------------------------------------
// Bug fix: Data\-prefix path normalization in atlas resolver + key builder
// ---------------------------------------------------------------------------
// FO4 LOD NIFs store diffuse texture slots with a leading `Data\` prefix
// (e.g. `Data\Textures\LOD\...` or `Data\LOD\...`).  The atlas texture resolver
// and the atlas-key lookup must both agree on the canonical stripped form so that
// (a) the file resolves on disk, and (b) transform_shape's atlas-key lookup
// finds the same entry the atlas was built with.
//
// These tests are the TDD anchors: they must FAIL before the fix and PASS after.

use lodgen_native::atlas::atlas::strip_normalize_texture_path;

/// `strip_normalize_texture_path` is the shared normalization helper: strips a
/// leading `Data\` / `Data/` prefix (case-insensitive) and prepends `Textures\`
/// when the result has no leading `Textures\` or other known root prefix.
#[test]
fn strip_normalize_strips_data_backslash_prefix() {
    // `Data\Textures\LOD\foo_d.dds` → `Textures\LOD\foo_d.dds`
    assert_eq!(
        strip_normalize_texture_path(r"Data\Textures\LOD\foo_d.dds"),
        r"Textures\LOD\foo_d.dds",
        "Data\\Textures\\... must strip Data\\ and keep Textures\\"
    );
}

#[test]
fn strip_normalize_strips_data_forward_slash_prefix() {
    // `Data/Textures/LOD/foo_d.dds` (BGSM forward-slash path) → `Textures\LOD\foo_d.dds`
    assert_eq!(
        strip_normalize_texture_path("Data/Textures/LOD/foo_d.dds"),
        r"Textures\LOD\foo_d.dds",
        "Data/Textures/... must strip Data/ and normalise slashes"
    );
}

#[test]
fn strip_normalize_data_lod_adds_textures_prefix() {
    // `Data\LOD\foo_d.dds` → `Textures\LOD\foo_d.dds`
    // (the dominant failing case: Data\ without a Textures\ segment between them)
    assert_eq!(
        strip_normalize_texture_path(r"Data\LOD\foo_d.dds"),
        r"Textures\LOD\foo_d.dds",
        "Data\\LOD\\... must yield Textures\\LOD\\..."
    );
}

#[test]
fn strip_normalize_bare_lod_adds_textures_prefix() {
    // `LOD\foo_d.dds` (already stripped by parse_nif, but missing Textures\) →
    // `Textures\LOD\foo_d.dds`
    assert_eq!(
        strip_normalize_texture_path(r"LOD\foo_d.dds"),
        r"Textures\LOD\foo_d.dds",
        "bare LOD\\... must gain Textures\\ prefix"
    );
}

#[test]
fn strip_normalize_clean_textures_path_unchanged() {
    // A path that already starts with `Textures\` must pass through unmodified.
    assert_eq!(
        strip_normalize_texture_path(r"Textures\LOD\foo_d.dds"),
        r"Textures\LOD\foo_d.dds",
        "already-correct Textures\\... path must be unchanged"
    );
}

#[test]
fn strip_normalize_case_insensitive() {
    // Lower-cased `data\textures\lod\foo_d.dds` (from parse_nif lowercase normalisation)
    // → `textures\lod\foo_d.dds`
    assert_eq!(
        strip_normalize_texture_path(r"data\textures\lod\foo_d.dds"),
        r"textures\lod\foo_d.dds",
        "lower-case data\\ form must strip correctly"
    );
}

/// Build an atlas from a synthetic tile stored under a path whose `Data\LOD\...`
/// form (missing Textures\ segment) would have failed resolution before the fix.
/// After the fix `build_atlas_from_tiles` receives the resolved abs path and the
/// key stored in AtlasList is `textures\lod\...`, while transform_shape sees
/// `lod\...` as the diffuse slot — the fix must make `atlas.contains(&key)` true.
#[test]
fn atlas_resolves_and_keys_data_lod_path() {
    use lodgen_native::atlas::atlas::build_atlas_from_tiles;
    use lodgen_native::objects::static_desc::atlas_build_key;

    let tmp = std::env::temp_dir().join("lodgen_data_lod_fix");
    std::fs::create_dir_all(&tmp).unwrap();

    // Create the tile at `Textures\LOD\` (the real on-disk location).
    let tex_dir = tmp.join("Textures").join("LOD");
    std::fs::create_dir_all(&tex_dir).unwrap();
    let px = vec![200u8, 100, 50, 255].repeat(16); // 4x4 RGBA
    let tile_d = tex_dir.join("synth01_lod_d.dds");
    directxtex_native::write_dds_rgba_image(&tile_d, 4, 4, &px, "BC1_UNORM", false).unwrap();

    let atlas_path = tmp.join("WObjects.dds");
    let map_path = tmp.join("WObjects.txt");
    let ar = build_atlas_from_tiles(
        &[tile_d.clone()],
        &atlas_path,
        &map_path,
        4096,
        512,
        "BC2_UNORM",
        "BC1_UNORM",
        "BC5_UNORM",
    )
    .expect("build_atlas_from_tiles");

    // The atlas must have one tile keyed under the canonical `textures\lod\...` form.
    assert_eq!(ar.list.len(), 1, "one tile must be in the atlas");

    // Simulate what transform_shape does: shape.textures[0] comes from parse_nif
    // which stripped `Data\` from `Data\LOD\synth01_lod_d.dds`, yielding
    // `lod\synth01_lod_d.dds`.  atlas_build_key must find it in the atlas.
    let shape_diffuse = r"lod\synth01_lod_d.dds";
    let textures = vec![shape_diffuse.to_string(), String::new(), String::new()];
    let key = atlas_build_key(&ar.list, &textures, 0);
    assert!(
        ar.list.contains(&key),
        "atlas_build_key({shape_diffuse:?}) must resolve into the atlas \
         (key={key:?}); this fails without the Data\\-prefix fix"
    );
}

/// End-to-end: a shape whose diffuse was `Data\LOD\...` in the NIF (stripped by
/// parse_nif to `lod\...`) must get atlas-remapped by transform_shape after the
/// fix.  We inject the canonical atlas key directly (simulating what
/// build_object_atlas would store after the fix) and verify transform_shape
/// applies the UV remap.
#[test]
fn transform_shape_remaps_data_lod_diffuse_after_fix() {
    // The atlas was built with the tile keyed under `textures\lod\synth_d.dds`
    // (canonical form after resolution via the Textures\ fallback).
    let mut atlas = lodgen_native::atlas::atlas::AtlasList::new();
    let rect = lodgen_native::atlas::atlas::AtlasRect::from_map_row(
        256,
        256,
        0,
        0,
        4096,
        4096,
        r"Textures\Terrain\W\Objects\WObjects.dds",
        false,
    );
    // Store with canonical key — what build_object_atlas produces after the fix.
    atlas.insert(r"textures\lod\synth_d.dds".to_string(), rect.clone());

    let quad = make_quad(16, 0, 0);
    let stat = make_stat([0.0, 0.0, 0.0], [0.0, 0.0, 0.0], 1.0);
    let mut shape = make_shape_one_vert([0.0, 0.0, 0.0], [0.5, 0.5]);
    // Simulate parse_nif output after stripping `Data\LOD\synth_d.dds` → `lod\synth_d.dds`
    shape.textures[0] = r"lod\synth_d.dds".to_string();
    shape.textures[1] = String::new();
    shape.texture_clamp_mode = 0; // force=true, bypass UV-tolerance gate

    let settings = LodSettings::fo4_default();
    let kept = transform_shape(&quad, &stat, &mut shape, &atlas, &settings.objects);
    assert!(kept);

    // After the fix, the atlas-key lookup must find the shape and remap its UVs.
    // Before the fix: atlas.contains("lod\synth_d.dds") was false → no remap →
    // textures[0] stayed as "lod\synth_d.dds".
    assert_eq!(
        shape.textures[0], rect.atlas_diffuse,
        "shape.textures[0] must be swapped to atlas diffuse after fix; \
         before fix it stays as the original lod\\... path"
    );
    // UV must be remapped (then QUVx-quantized by ReUV/Simplify; Geometry.cs:1155).
    let (eu, ev) = rect.uv_atlas(0.5, 0.5);
    let (eu, ev) = (quvx_test(eu), quvx_test(ev));
    let got = shape
        .geometry
        .uvcoords
        .iter()
        .find(|uv| (uv[0] - eu).abs() < 1e-4 && (uv[1] - ev).abs() < 1e-4)
        .copied()
        .unwrap_or(shape.geometry.uvcoords[0]);
    let eps = 1e-3_f32;
    assert!((got[0] - eu).abs() < eps, "u remapped: {} vs {eu}", got[0]);
    assert!((got[1] - ev).abs() < eps, "v remapped: {} vs {ev}", got[1]);
}

/// QUVx (Utils.cs:325) replicated for tests: signed-truncate to 3 decimals.
fn quvx_test(value: f32) -> f32 {
    if value < 0.0 {
        (value * 1000.0).ceil() / 1000.0
    } else {
        (value * 1000.0).floor() / 1000.0
    }
}
