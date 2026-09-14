// .btr structural-parity gate.
// Validates golden .btr from xLODGen corpus and our own synthetic output.
// Tests SKIP (with a notice) if the corpus files are absent.

use std::path::PathBuf;

mod fixtures;

fn corpus(rel: &str) -> Option<PathBuf> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .join(rel);
    // Canonicalize on Windows to handle case-insensitive paths.
    let p = p.canonicalize().unwrap_or(p);
    if p.exists() {
        Some(p)
    } else {
        eprintln!("SKIP: missing {}", p.display());
        None
    }
}

#[test]
fn golden_btr_structural_invariants() {
    let Some(path) =
        corpus("tmp/xlodgen/meshes/terrain/DLC03FarHarbor/DLC03FarHarbor.16.-25.-11.btr")
    else {
        return;
    };
    let nif = nif_core_native::model::NifFile::load(&path).expect("parse golden btr");
    // bs_version 130, version 20.2.0.7
    assert_eq!(nif.header.bs_version, 130);
    assert!(nif.blocks.iter().any(|b| b.type_name == "BSMultiBoundNode"));
    assert!(nif.blocks.iter().any(|b| b.type_name == "BSTriShape"));
    assert!(
        nif.blocks
            .iter()
            .any(|b| b.type_name == "BSLightingShaderProperty")
    );
    assert!(
        nif.blocks
            .iter()
            .any(|b| b.type_name == "BSShaderTextureSet")
    );
}

#[test]
fn our_btr_matches_golden_block_graph() {
    let Some(golden) =
        corpus("tmp/xlodgen/meshes/terrain/DLC03FarHarbor/DLC03FarHarbor.16.-25.-11.btr")
    else {
        return;
    };
    let g = nif_core_native::model::NifFile::load(&golden).unwrap();
    let golden_types: std::collections::BTreeSet<_> =
        g.blocks.iter().map(|b| b.type_name.clone()).collect();

    // Build our own minimal terrain .btr and compare the set of block types present.
    let verts = [
        [0.0f32, 0.0, 0.0],
        [4096.0, 0.0, 0.0],
        [0.0, 4096.0, 0.0],
        [4096.0, 4096.0, 0.0],
    ];
    let uvs = [[0.0f32, 1.0], [1.0, 1.0], [0.0, 0.0], [1.0, 0.0]];
    let tris = [[0u16, 1, 2], [1, 3, 2]];
    let mut bb = lodgen_native::descriptors::BBox::empty();
    for v in &verts {
        bb.grow_vertex(*v);
    }
    let mut ours = lodgen_native::output::btr::build_btr_nif(
        &verts,
        &uvs,
        &tris,
        "d.dds",
        "d_msn.dds",
        &bb,
        16.0,
        0.0,
    )
    .unwrap();
    let our_types: std::collections::BTreeSet<_> =
        ours.blocks.iter().map(|b| b.type_name.clone()).collect();

    // Every land-side block type we emit must exist in the golden.
    // (Golden also has WATER blocks for blocks with water; our flat no-water
    // tile is a subset — just assert the land types.)
    for t in [
        "BSMultiBoundNode",
        "BSTriShape",
        "BSLightingShaderProperty",
        "BSShaderTextureSet",
        "BSMultiBound",
        "BSMultiBoundAABB",
    ] {
        assert!(golden_types.contains(t), "golden missing {t}");
        assert!(our_types.contains(t), "ours missing {t}");
    }
    // Serialize to bytes — must succeed without panicking.
    let _ = ours.to_bytes().unwrap();
}

/// Read a BSLightingShaderProperty bitflag mask. nif_core stores FO4 shader
/// flags as a raw `NifValue::UInt` under the version-suffixed key
/// `<field>:FO4` (BS Version == 130); the modkit display layer decodes that
/// mask into flag NAMES (e.g. ["Model_Space_Normals","Own_Emit","ZBuffer_Test"]).
fn shader_flag_mask(nif: &nif_core_native::model::NifFile, field: &str) -> u64 {
    use nif_core_native::model::NifValue;
    let lsp = nif
        .blocks
        .iter()
        .find(|b| b.type_name == "BSLightingShaderProperty")
        .expect("BSLightingShaderProperty present");
    let key = format!("{field}:FO4");
    match lsp.fields.get(&key).or_else(|| lsp.fields.get(field)) {
        Some(NifValue::UInt(v)) => *v,
        Some(NifValue::Int(v)) => *v as u64,
        other => panic!("{key} not a numeric bitflag mask: {other:?}"),
    }
}

#[test]
fn our_btr_shader_flags_equal_golden() {
    let Some(golden) =
        corpus("tmp/xlodgen/meshes/terrain/DLC03FarHarbor/DLC03FarHarbor.16.-25.-11.btr")
    else {
        return;
    };
    let g = nif_core_native::model::NifFile::load(&golden).unwrap();
    let golden_f1 = shader_flag_mask(&g, "Shader Flags 1");
    let golden_f2 = shader_flag_mask(&g, "Shader Flags 2");
    // Sanity-check the golden carries the expected LOD-landscape masks.
    //  flags1 = Model_Space_Normals(bit12=0x1000) | Own_Emit(bit22=0x400000)
    //         | ZBuffer_Test(bit31=0x80000000) = 0x80401000 = 2151682048.
    //  flags2 = ZBuffer_Write(bit0) | LOD_Landscape(bit1) = 0x3.
    assert_eq!(golden_f1, 0x8040_1000);
    assert_eq!(golden_f2, 0x3);

    // Build our btr, serialize, reload, and compare the round-tripped flag masks.
    let verts = [[0.0f32, 0.0, 0.0], [4096.0, 0.0, 0.0], [0.0, 4096.0, 0.0]];
    let uvs = [[0.0f32, 1.0], [1.0, 1.0], [0.0, 0.0]];
    let tris = [[0u16, 1, 2]];
    let mut bb = lodgen_native::descriptors::BBox::empty();
    for v in &verts {
        bb.grow_vertex(*v);
    }
    let mut ours = lodgen_native::output::btr::build_btr_nif(
        &verts,
        &uvs,
        &tris,
        "d.dds",
        "d_msn.dds",
        &bb,
        16.0,
        0.0,
    )
    .unwrap();
    let bytes = ours.to_bytes().unwrap();
    let rt = nif_core_native::model::NifFile::from_bytes(&bytes, None).expect("reload our btr");

    assert_eq!(
        shader_flag_mask(&rt, "Shader Flags 1"),
        golden_f1,
        "Shader Flags 1 must round-trip equal to golden"
    );
    assert_eq!(
        shader_flag_mask(&rt, "Shader Flags 2"),
        golden_f2,
        "Shader Flags 2 must round-trip equal to golden"
    );
}

#[test]
fn vanilla_btr_structural_invariants() {
    // Commonwealth vanilla .BTR (note uppercase extension on Windows — use case-insensitive lookup)
    let Some(path) =
        corpus("extracted/fo4/Meshes/Terrain/Commonwealth/Commonwealth.16.-16.-16.BTR")
    else {
        return;
    };
    let nif = nif_core_native::model::NifFile::load(&path).expect("parse vanilla btr");
    assert_eq!(nif.header.bs_version, 130);
    assert!(nif.blocks.iter().any(|b| b.type_name == "BSMultiBoundNode"));
    assert!(nif.blocks.iter().any(|b| b.type_name == "BSTriShape"));
}

// ---------------------------------------------------------------------------
// FO4 center.z zeroing, ShiftZ/zShift translation, FULLPREC.
// ---------------------------------------------------------------------------

fn tri_shape<'a>(nif: &'a nif_core_native::model::NifFile) -> &'a nif_core_native::model::NifBlock {
    nif.blocks
        .iter()
        .find(|b| b.type_name == "BSTriShape")
        .expect("BSTriShape present")
}

fn f32_field(s: &nif_core_native::model::NifValue) -> f32 {
    use nif_core_native::model::NifValue;
    match s {
        NifValue::Float(f) => *f as f32,
        NifValue::FloatNan(_) => f32::NAN,
        other => panic!("not a float: {other:?}"),
    }
}

/// BSTriShape bounding-sphere center.z (model: Struct{Center: Vec3, Radius}).
fn bounding_center_z(b: &nif_core_native::model::NifBlock) -> f32 {
    use nif_core_native::model::NifValue;
    let NifValue::Struct(bs) = b.fields.get("Bounding Sphere").expect("Bounding Sphere") else {
        panic!("Bounding Sphere not a struct");
    };
    match bs.get("Center").expect("Center") {
        NifValue::Vec3(v) => v[2],
        NifValue::Struct(m) => f32_field(m.get("z").expect("center.z")),
        other => panic!("center: {other:?}"),
    }
}

/// BSTriShape Translation.z.
fn translation_z(b: &nif_core_native::model::NifBlock) -> f32 {
    use nif_core_native::model::NifValue;
    match b.fields.get("Translation").expect("Translation") {
        NifValue::Vec3(v) => v[2],
        NifValue::Struct(m) => f32_field(m.get("z").expect("translation.z")),
        other => panic!("translation: {other:?}"),
    }
}

fn vertex_desc(b: &nif_core_native::model::NifBlock) -> u64 {
    use nif_core_native::model::NifValue;
    match b.fields.get("Vertex Desc").expect("Vertex Desc") {
        NifValue::UInt(v) => *v,
        NifValue::Int(v) => *v as u64,
        other => panic!("Vertex Desc: {other:?}"),
    }
}

#[test]
fn fo4_btr_center_z_is_zeroed_for_nonflat_terrain() {
    // A non-flat ramp has a real z range; center(true) would NOT yield z==0,
    // so this guards the explicit FO4 center.z := 0 (TerrainLOD.cs:1443).
    let world = fixtures::ramp_world("W", 4, 500.0);
    let s = lodgen_native::settings::LodSettings::fo4_default();
    let quad = lodgen_native::descriptors::quads_for(&world, 4, &s)
        .into_iter()
        .find(|q| q.x == 0 && q.y == 0)
        .unwrap();
    let mesh = lodgen_native::terrain::terrain_lod::build_terrain_mesh(&world, &quad, &s).unwrap();

    // sanity: the mesh really has a non-zero z range (otherwise the test is moot).
    let zmin = mesh.bbox.min[2];
    let zmax = mesh.bbox.max[2];
    assert!(
        zmax - zmin > 1.0,
        "fixture must be non-flat (z range {})",
        zmax - zmin
    );

    let mut nif = lodgen_native::output::btr::build_btr_nif(
        &mesh.verts,
        &mesh.uvs,
        &mesh.tris,
        "W.4.0.0.dds",
        "W.4.0.0_msn.dds",
        &mesh.bbox,
        4.0,
        0.0,
    )
    .unwrap();
    let bytes = nif.to_bytes().unwrap();
    let rt = nif_core_native::model::NifFile::from_bytes(&bytes, None).unwrap();
    let ts = tri_shape(&rt);
    assert_eq!(bounding_center_z(ts), 0.0, "FO4 land center.z must be 0");
}

#[test]
fn fo4_btr_translation_applies_zshift_and_shiftz() {
    // ShiftZ (Geometry.cs:2165-2175): subtract bbox z-center from each vert and
    // push lodLevel * z_center into Translation.z; plus the per-level zShift.
    // TerrainLOD.cs:1432,1438-1441.
    let world = fixtures::ramp_world("W", 4, 500.0);
    let s = lodgen_native::settings::LodSettings::fo4_default();
    let quad = lodgen_native::descriptors::quads_for(&world, 4, &s)
        .into_iter()
        .find(|q| q.x == 0 && q.y == 0)
        .unwrap();
    let mesh = lodgen_native::terrain::terrain_lod::build_terrain_mesh(&world, &quad, &s).unwrap();

    let lod_level = 4.0f32;
    let z_center = (mesh.bbox.min[2] + mesh.bbox.max[2]) / 2.0;
    let z_shift = 0.0f32; // Phase-1 default zShift
    let expected_translation_z = z_shift + lod_level * z_center;

    let mut nif = lodgen_native::output::btr::build_btr_nif(
        &mesh.verts,
        &mesh.uvs,
        &mesh.tris,
        "W.4.0.0.dds",
        "W.4.0.0_msn.dds",
        &mesh.bbox,
        lod_level,
        z_shift,
    )
    .unwrap();
    let bytes = nif.to_bytes().unwrap();
    let rt = nif_core_native::model::NifFile::from_bytes(&bytes, None).unwrap();
    let ts = tri_shape(&rt);
    let got = translation_z(ts);
    assert!(
        (got - expected_translation_z).abs() < 0.5,
        "Translation.z: got {got}, expected {expected_translation_z}"
    );
    // The ramp's z-center is clearly nonzero, so a hardcoded 0.0 would fail here.
    assert!(
        expected_translation_z.abs() > 1.0,
        "fixture z-center must be nonzero ({expected_translation_z})"
    );
}

#[test]
fn fo4_btr_tall_tile_uses_fullprec_vertex_format() {
    // Synthetic tile with z extent > 131008 units -> FULLPREC (Geometry.cs:346-349,
    // TerrainLOD.cs:1445-1448): vertexFlags |= 0x4000, full-f32 position (stride 16).
    let verts = [
        [0.0f32, 0.0, 0.0],
        [4096.0, 0.0, 0.0],
        [0.0, 4096.0, 200000.0], // z extent 200000 > 131008
    ];
    let uvs = [[0.0f32, 1.0], [1.0, 1.0], [0.0, 0.0]];
    let tris = [[0u16, 1, 2]];
    let mut bb = lodgen_native::descriptors::BBox::empty();
    for v in &verts {
        bb.grow_vertex(*v);
    }
    let mut nif = lodgen_native::output::btr::build_btr_nif(
        &verts,
        &uvs,
        &tris,
        "d.dds",
        "d_msn.dds",
        &bb,
        4.0,
        0.0,
    )
    .unwrap();
    let bytes = nif.to_bytes().unwrap();
    let rt = nif_core_native::model::NifFile::from_bytes(&bytes, None).unwrap();
    let ts = tri_shape(&rt);
    let desc = vertex_desc(ts);
    // FULLPREC: attributes (Desc >> 44) has bit 0x400 set, and vertex size (Desc & 0xF) == 4.
    let attributes = desc >> 44;
    assert_eq!(
        attributes & 0x400,
        0x400,
        "FULLPREC bit must be set (desc {desc:#x})"
    );
    assert_eq!(
        desc & 0xF,
        4,
        "FULLPREC vertex size must be 4 words (stride 16)"
    );
}

#[test]
fn fo4_btr_normal_tile_stays_halfprec() {
    // Guard: a normal (short-z) tile must NOT get FULLPREC.
    let verts = [
        [0.0f32, 0.0, 0.0],
        [4096.0, 0.0, 100.0],
        [0.0, 4096.0, 200.0],
    ];
    let uvs = [[0.0f32, 1.0], [1.0, 1.0], [0.0, 0.0]];
    let tris = [[0u16, 1, 2]];
    let mut bb = lodgen_native::descriptors::BBox::empty();
    for v in &verts {
        bb.grow_vertex(*v);
    }
    let mut nif = lodgen_native::output::btr::build_btr_nif(
        &verts,
        &uvs,
        &tris,
        "d.dds",
        "d_msn.dds",
        &bb,
        4.0,
        0.0,
    )
    .unwrap();
    let bytes = nif.to_bytes().unwrap();
    let rt = nif_core_native::model::NifFile::from_bytes(&bytes, None).unwrap();
    let ts = tri_shape(&rt);
    let desc = vertex_desc(ts);
    assert_eq!(desc, 52776558133763, "half-prec VERTEX|UV desc unchanged");
}
