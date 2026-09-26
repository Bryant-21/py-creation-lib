/// Object atlas + UV tests.
/// Run: `uv run cargo test -p lodgen_native --test object_atlas`
use lodgen_native::atlas::binpacker::{BinBlock, BinPacker};

// ---------------------------------------------------------------------------
// BinPacker (TwbBinPacker port)
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// AtlasRect + atlas-map .txt writer (byte-exact)
// ---------------------------------------------------------------------------

use lodgen_native::atlas::atlas::{AtlasMapRow, AtlasRect, parse_atlas_map, write_atlas_map};

#[test]
fn atlas_map_row_byte_exact() {
    // port: wbLOD.pas:1557-1566
    // TAB-separated, LF-terminated, no BOM.
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
use lodgen_native::objects::static_desc::atlas_get_key;

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
    MultiBoundAabb, generate_multibound, generate_segments,
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

// parse_nif / iterate_nif over synthetic NIFs

use lodgen_native::game::Game;
use lodgen_native::input::{StaticDesc, WorldspaceInput};
use lodgen_native::progress::{LodPaths, QuadCtx};
use lodgen_native::settings::LodSettings;

/// A minimal `StaticDesc` whose LOD model points at `model`.
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

/// Run a closure with a `QuadCtx` over an empty data dir.
fn with_ctx<R>(level: i32, f: impl FnOnce(&QuadCtx) -> R) -> R {
    let world = empty_world();
    let settings = LodSettings::fo4_default();
    let game = Game::fo4();
    let paths = LodPaths {
        data_dirs: vec![std::env::temp_dir().join("lodgen_no_data_dir")],
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
    // Identity node transform, node_scale=1, stat.scale=2, stat at [100,200,50],
    // rot=0, quad (0,0) L16. shape.x/y = stat.x/y - quad.x/y*4096 = 100/200.
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
fn transform_shape_applies_node_translation() {
    // The full node_transform (including its translation column) must be applied to
    // vertices, not just the upper 3x3. Expected values follow the C# Matrix44/Vector3
    // algebra:
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
fn iterate_nif_accumulates_node_transform_in_order() {
    // Parse side: node-transform accumulation must use the C# order (column:
    // parent · node_local · geom_local). A synthetic 2-NiNode chain with
    // rotation+translation distinguishes it from `node_local · parent`.
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


/// Build a real atlas from two synthetic 4×4 DDS tiles written to tmp.
/// This exercises the DDS I/O path of build_object_atlas via a helper.
#[test]
fn build_atlas_from_synthetic_dds() {
    use lodgen_native::atlas::atlas::build_atlas_from_tiles;

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
fn missing_specular_tiles_and_atlas_padding_are_neutral_black() {
    use lodgen_native::atlas::atlas::build_atlas_from_tiles;

    let tmp = std::env::temp_dir().join(format!(
        "lodgen_neutral_specular_atlas_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();

    let tile_a = tmp.join("tile_a_d.dds");
    let tile_b = tmp.join("tile_b_d.dds");
    directxtex_native::write_dds_rgba_image(
        &tile_a,
        4,
        4,
        &vec![255u8, 0, 0, 255].repeat(16),
        "BC1_UNORM",
        false,
    )
    .unwrap();
    directxtex_native::write_dds_rgba_image(
        &tile_b,
        2,
        2,
        &vec![0u8, 0, 255, 255].repeat(4),
        "BC1_UNORM",
        false,
    )
    .unwrap();

    let atlas = build_atlas_from_tiles(
        &[tile_a, tile_b],
        &tmp.join("World.Objects.dds"),
        &tmp.join("World.Objects.txt"),
        4096,
        512,
        "BC2_UNORM",
        "BC1_UNORM",
        "BC5_UNORM",
    )
    .unwrap();
    assert!(
        atlas.atlas_size.0 * atlas.atlas_size.1 > 20,
        "unequal tiles must leave unused atlas padding"
    );

    let decoded = directxtex_native::read_dds_mips_rgba8(&atlas.specular).unwrap();
    assert!(
        decoded.mips.iter().all(|mip| mip
            .2
            .chunks_exact(4)
            .all(|pixel| { pixel[0] == 0 && pixel[1] == 0 })),
        "missing specular data and atlas padding must stay neutral through every mip"
    );

    let _ = std::fs::remove_dir_all(tmp);
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
use lodgen_native::output::bto::{build_bto_nif, build_bto_nif_with_layout};
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
        data_dirs: vec![std::env::temp_dir().join("lodgen_no_data_dir")],
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

// ---------------------------------------------------------------------------
// Data\-prefix path normalization in atlas resolver + key builder
// ---------------------------------------------------------------------------
// FO4 LOD NIFs store diffuse texture slots with a leading `Data\` prefix
// (e.g. `Data\Textures\LOD\...` or `Data\LOD\...`). The atlas texture resolver
// and the atlas-key lookup must agree on the canonical stripped form so the file
// resolves on disk and transform_shape's lookup finds the entry the atlas was
// built with.

use lodgen_native::atlas::atlas::strip_normalize_texture_path;

#[test]
fn strip_normalize_texture_path_cases() {
    for (input, want) in [
        (r"Data\Textures\LOD\foo_d.dds", r"Textures\LOD\foo_d.dds"),
        ("Data/Textures/LOD/foo_d.dds", r"Textures\LOD\foo_d.dds"),
        (r"Data\LOD\foo_d.dds", r"Textures\LOD\foo_d.dds"),
        (r"LOD\foo_d.dds", r"Textures\LOD\foo_d.dds"),
        (r"Textures\LOD\foo_d.dds", r"Textures\LOD\foo_d.dds"),
        (r"data\textures\lod\foo_d.dds", r"textures\lod\foo_d.dds"),
    ] {
        assert_eq!(strip_normalize_texture_path(input), want, "{input}");
    }
}

/// Atlas a synthetic tile from `Textures\LOD\`: AtlasList keys it as `textures\lod\...`,
/// while transform_shape sees `lod\...` (from a NIF `Data\LOD\...` slot) as the diffuse.
/// `atlas_build_key` must bridge the two so `atlas.contains(&key)` holds.
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

/// QUVx (Utils.cs:325) replicated for tests: signed-truncate to 3 decimals.
fn quvx_test(value: f32) -> f32 {
    if value < 0.0 {
        (value * 1000.0).ceil() / 1000.0
    } else {
        (value * 1000.0).floor() / 1000.0
    }
}
