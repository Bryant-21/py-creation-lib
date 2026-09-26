// Synthetic .btr writer checks: FO4 center.z zeroing, ShiftZ/zShift, FULLPREC.

mod fixtures;

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
fn fo4_btr_vertex_format_switches_to_fullprec_for_tall_tiles() {
    // z extent > 131008 units -> FULLPREC (Geometry.cs:346-349, TerrainLOD.cs:1445-1448):
    // attributes bit 0x400 set, full-f32 position (vertex size 4 words).
    for (top_z, fullprec) in [(200000.0f32, true), (200.0, false)] {
        let verts = [[0.0f32, 0.0, 0.0], [4096.0, 0.0, 100.0], [0.0, 4096.0, top_z]];
        let uvs = [[0.0f32, 1.0], [1.0, 1.0], [0.0, 0.0]];
        let mut bb = lodgen_native::descriptors::BBox::empty();
        for v in &verts {
            bb.grow_vertex(*v);
        }
        let mut nif = lodgen_native::output::btr::build_btr_nif(
            &verts,
            &uvs,
            &[[0u16, 1, 2]],
            "d.dds",
            "d_msn.dds",
            &bb,
            4.0,
            0.0,
        )
        .unwrap();
        let bytes = nif.to_bytes().unwrap();
        let rt = nif_core_native::model::NifFile::from_bytes(&bytes, None).unwrap();
        let desc = vertex_desc(tri_shape(&rt));
        if fullprec {
            assert_eq!((desc >> 44) & 0x400, 0x400, "FULLPREC bit (desc {desc:#x})");
            assert_eq!(desc & 0xF, 4, "FULLPREC vertex size");
        } else {
            assert_eq!(desc, 52776558133763, "half-prec VERTEX|UV desc");
        }
    }
}
