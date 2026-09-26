/// Tree LOD + billboard tests.

// ---------------------------------------------------------------------------
// is_tree classification + generate_quad dispatch
// ---------------------------------------------------------------------------

#[cfg(test)]
mod task1_is_tree {
    use lodgen_native::input::RefInput;
    use lodgen_native::trees::is_tree;

    fn make_stat() -> RefInput {
        RefInput {
            ref_id: "00001234".to_string(),
            ref_flags: 0,
            enable_parent: 0,
            cell: (0, 0),
            pos: [0.0; 3],
            rot: [0.0; 3],
            scale: 1.0,
            color: 1.0,
            alpha_threshold: 0,
            is_billboard: false,
            is_grass: false,
            base_name: "SomeTree".to_string(),
            base_flags: 0,
            material_name: String::new(),
            full_model: "meshes/trees/sometree.nif".to_string(),
            lod_models: [None, None, None, None],
            part_transform: lodgen_native::input::identity_part_transform(),
            part_scale: 1.0,
            material_swap: Default::default(),
        }
    }

    #[test]
    fn is_tree_by_base_flag() {
        // IS_TREE ShapeFlag = 0x1000; map to base_flags to indicate TREE record type.
        // LODApp.cs routes refs: tree flag on the base record marks as tree.
        let mut stat = make_stat();
        // 0x1000 in base_flags — TREE base-record marker used by lodgen
        stat.base_flags = 0x1000;
        assert!(is_tree(&stat), "IS_TREE base flag should classify as tree");

        let plain = make_stat(); // base_flags = 0
        assert!(
            !is_tree(&plain),
            "plain STAT with no markers should not be a tree"
        );
    }

    #[test]
    fn generate_quad_dispatch_3d_vs_billboard() {
        use lodgen_native::atlas::atlas::AtlasResult;
        use lodgen_native::descriptors::QuadDesc;
        use lodgen_native::input::WorldspaceInput;
        use lodgen_native::progress::{LodPaths, QuadCtx};
        use lodgen_native::settings::LodSettings;
        use lodgen_native::trees::generate_quad;

        // A quad with no tree statics → generate_quad returns empty QuadOutputs for both modes.
        let world = WorldspaceInput::from_cells("W", vec![]);
        let mut settings = LodSettings::fo4_default();
        let out = std::env::temp_dir().join("lodgen_p3_task1");
        std::fs::create_dir_all(&out).unwrap();
        let paths = LodPaths {
            data_dirs: vec![out.clone()],
            output_dir: out.clone(),
            source_data_dir: None,
        };

        let atlas = AtlasResult {
            map_path: out.join("atlas.json"),
            diffuse: out.join("atlas.dds"),
            normal: out.join("atlas_n.dds"),
            specular: out.join("atlas_s.dds"),
            atlas_size: (0, 0),
            uv: Default::default(),
            list: Default::default(),
            dds_written: 0,
        };

        let quad = QuadDesc {
            z_order: 0,
            x: 0,
            y: 0,
            quad_level: 0,
            quad_index: 0,
            quad_offset: 0.0,
            static_indices: Vec::new(),
            statics: vec![],
            out_values: Default::default(),
        };

        // 3D mode: empty statics → empty outputs
        settings.trees.trees_3d = true;
        let ctx3d = QuadCtx {
            world: &world,
            settings: &settings,
            game: &lodgen_native::game::Game::fo4(),
            paths: &paths,
            level: 4,
        };
        let out3d = generate_quad(&quad, &ctx3d, &atlas).expect("generate_quad 3d mode");
        assert!(out3d.meshes.is_empty(), "no trees → no meshes (3d mode)");

        // Billboard mode: empty statics → empty outputs
        settings.trees.trees_3d = false;
        let ctx2d = QuadCtx {
            world: &world,
            settings: &settings,
            game: &lodgen_native::game::Game::fo4(),
            paths: &paths,
            level: 4,
        };
        let out2d = generate_quad(&quad, &ctx2d, &atlas).expect("generate_quad billboard mode");
        assert!(
            out2d.meshes.is_empty(),
            "no trees → no meshes (billboard mode)"
        );
    }
}

// ---------------------------------------------------------------------------
// FlatDesc + billboard .txt dimension parser
// ---------------------------------------------------------------------------

#[cfg(test)]
mod task2_flatdesc {
    use lodgen_native::trees::tree3d::{FlatDesc, parse_billboard_dimensions};

    #[test]
    fn flatdesc_parse_complex() {
        let mut fd = FlatDesc::default();
        parse_billboard_dimensions("COMPLEX=true\n", &mut fd);
        assert!(fd.complex, "COMPLEX=true should set complex flag");
    }
}

// ---------------------------------------------------------------------------
// output::btt tree-block + .lst writers + naming::btt
// ---------------------------------------------------------------------------

#[cfg(test)]
mod task3_btt {

    use lodgen_native::output::btt::{
        LstEntry, TreeRef, TreeType, encode_tree_block, encode_tree_list,
    };

    fn le_i32(bytes: &[u8], off: usize) -> i32 {
        i32::from_le_bytes(bytes[off..off + 4].try_into().unwrap())
    }
    fn le_f32(bytes: &[u8], off: usize) -> f32 {
        f32::from_le_bytes(bytes[off..off + 4].try_into().unwrap())
    }

    #[test]
    fn tree_block_header_bytes() {
        // TwbLodTES5TreeRef is a `packed record` blitted in DECLARATION order
        // (wbLOD.pas:125-131; SaveToFile :950 `Move`/`Write` of the raw struct):
        //   X(f32), Y(f32), Z(f32), Rotation(f32), Scale(f32),
        //   RefFormID(u32), Unknown1(i32)=0, Unknown2(i32)=0
        // = 8 × 4 = 32 bytes per ref (not 24).
        // Block: [i32 numTypes=1][i32 index=7][i32 count=1][TreeRef: 32 bytes]
        // Total = 4 + 4 + 4 + 32 = 44
        let types = vec![TreeType {
            index: 7,
            refs: vec![TreeRef {
                form_id: 0x0A001234,
                x: 1.0,
                y: 2.0,
                z: 3.0,
                scale: 1.5,
                rotation: 0.25,
            }],
        }];
        let bytes = encode_tree_block(&types);
        assert_eq!(bytes.len(), 44, "expected 44 bytes, got {}", bytes.len());
        assert_eq!(le_i32(&bytes, 0), 1, "numTypes");
        assert_eq!(le_i32(&bytes, 4), 7, "index");
        assert_eq!(le_i32(&bytes, 8), 1, "count");
        // TreeRef field order per Pascal packed record: X, Y, Z, Rotation, Scale, RefFormID, Unknown1, Unknown2
        assert!((le_f32(&bytes, 12) - 1.0).abs() < 1e-6, "x");
        assert!((le_f32(&bytes, 16) - 2.0).abs() < 1e-6, "y");
        assert!((le_f32(&bytes, 20) - 3.0).abs() < 1e-6, "z");
        assert!((le_f32(&bytes, 24) - 0.25).abs() < 1e-6, "rotation");
        assert!((le_f32(&bytes, 28) - 1.5).abs() < 1e-6, "scale");
        assert_eq!(le_i32(&bytes, 32), 0x0A001234_u32 as i32, "form_id");
        assert_eq!(le_i32(&bytes, 36), 0, "Unknown1 must be 0");
        assert_eq!(le_i32(&bytes, 40), 0, "Unknown2 must be 0");
    }

    #[test]
    fn tree_list_header_bytes() {
        // TwbLodTES5TreeType layout (wbLOD.pas:115-122):
        //   Index(i32), Width(f32), Height(f32), UVMinX(f32), UVMinY(f32), UVMaxX(f32), UVMaxY(f32), Unknown(i32)
        // = 8 × 4 = 32 bytes per entry
        let entry = LstEntry {
            index: 5,
            width: 100.0,
            height: 256.0,
            uv_min_x: 0.0,
            uv_max_x: 0.5,
            uv_min_y: 0.0,
            uv_max_y: 0.25,
        };
        let bytes = encode_tree_list(&[entry]);
        // [i32 numTrees=1][TwbLodTES5TreeType × 1]
        assert_eq!(le_i32(&bytes, 0), 1, "numTrees");
        // 4 + 32 = 36 total
        assert_eq!(bytes.len(), 4 + 32, "total len should be 36");
        assert_eq!(le_i32(&bytes, 4), 5, "index");
        assert!((le_f32(&bytes, 8) - 100.0).abs() < 1e-4, "width");
        assert!((le_f32(&bytes, 12) - 256.0).abs() < 1e-4, "height");
    }

}

// ---------------------------------------------------------------------------
// tree3d::generate_quad — FlatTrunk billboard quads + 3D-tree .bto
// ---------------------------------------------------------------------------

#[cfg(test)]
mod task4_tree3d {
    use lodgen_native::input::RefInput;
    use lodgen_native::objects::static_desc::ShapeFlags;
    use lodgen_native::trees::tree3d::{FlatDesc, build_flat_trunk};

    fn make_flat_desc() -> FlatDesc {
        FlatDesc {
            width: 50.0,
            depth: 50.0,
            height: 512.0,
            shift_x: 0.0,
            shift_y: 0.0,
            shift_z: 8.0,
            scale: 1.0,
            complex: false,
            dimensions: vec![[50.0, 512.0, 0.0], [50.0, 512.0, 0.0]],
        }
    }

    fn make_stat_billboard() -> RefInput {
        RefInput {
            ref_id: "DEADBEEF".to_string(),
            ref_flags: 0,
            enable_parent: 0,
            cell: (0, 0),
            pos: [0.0; 3],
            rot: [0.0; 3],
            scale: 1.0,
            color: 1.0,
            alpha_threshold: 0,
            is_billboard: true,
            is_grass: false,
            base_name: "SomeTree".to_string(),
            base_flags: 0x1000,
            material_name: String::new(),
            full_model: "meshes/trees/sometree.nif".to_string(),
            lod_models: [
                Some("textures/trees/sometree.dds".to_string()),
                None,
                None,
                None,
            ],
            part_transform: lodgen_native::input::identity_part_transform(),
            part_scale: 1.0,
            material_swap: Default::default(),
        }
    }

    /// build_flat_trunk with a FlatDesc{width:50,height:512,depth:50,shift_z:8}
    /// must yield exactly 2 ShapeDescs (two crossed quads, LODApp.cs:1442-1457), each
    /// with 4 vertices, 2 triangles, IS_BILLBOARD set, and Z in [shift_z, shift_z+height].
    #[test]
    fn flat_trunk_two_crossed_quads() {
        let fd = make_flat_desc();
        let shapes = build_flat_trunk(&fd, "textures/trees/sometree.dds");
        // Two crossed quads — LODApp.cs:1442 `for (int i = 0; i < 2; i++)`
        assert_eq!(shapes.len(), 2, "expected 2 shapes (two crossed quads)");

        for (i, shape) in shapes.iter().enumerate() {
            // Each quad: 4 vertices, 2 triangles — LODApp.cs:1453 Geometry(4,4,4,4,4,2,...)
            assert_eq!(
                shape.geometry.vertices.len(),
                4,
                "quad {i}: expected 4 vertices"
            );
            assert_eq!(
                shape.geometry.triangles.len(),
                2,
                "quad {i}: expected 2 triangles"
            );

            // IS_BILLBOARD flag set — LODApp.cs:1391 isBillboard=true
            assert!(
                shape.flags.contains(ShapeFlags::IS_BILLBOARD),
                "quad {i}: missing IS_BILLBOARD flag"
            );

            // All Z values must be within [shift_z, shift_z + height] = [8, 520]
            for v in &shape.geometry.vertices {
                assert!(
                    v[2] >= 8.0 - 1e-4 && v[2] <= 520.0 + 1e-4,
                    "quad {i}: vertex z={} out of [8, 520]",
                    v[2]
                );
            }

            // Billboard diffuse set in slot 0
            assert_eq!(
                shape.textures[0], "textures/trees/sometree.dds",
                "quad {i}: billboard diffuse not in slot 0"
            );
        }

        // Quad 0 (LODApp.cs:1454-1457):
        //   v0 = (center_x - dim.x, y_offset, shiftZ)
        //   v1 = (center_x + dim.x, y_offset, shiftZ)
        //   v2 = (center_x + dim.x, y_offset, shiftZ + dim.y)
        //   v3 = (center_x - dim.x, y_offset, shiftZ + dim.y)
        let q0 = &shapes[0];
        let q0v = &q0.geometry.vertices;
        // center_x for i=0 is array[0]=shiftX=0, dim[0].z=0 → center = 0+0=0
        // dim.x = dimensions[0][0] = 50 → corners at ±50
        let shift_z = 8.0_f32;
        let height = 512.0_f32;
        assert!(
            (q0v[0][2] - shift_z).abs() < 1e-4,
            "q0 v0 z expected shiftZ={shift_z}, got {}",
            q0v[0][2]
        );
        assert!(
            (q0v[1][2] - shift_z).abs() < 1e-4,
            "q0 v1 z expected shiftZ={shift_z}, got {}",
            q0v[1][2]
        );
        assert!(
            (q0v[2][2] - (shift_z + height)).abs() < 1e-4,
            "q0 v2 z expected top, got {}",
            q0v[2][2]
        );
        assert!(
            (q0v[3][2] - (shift_z + height)).abs() < 1e-4,
            "q0 v3 z expected top, got {}",
            q0v[3][2]
        );

        // The x-span of quad 0 should be ±dim.x = ±50
        let x_min = q0v.iter().map(|v| v[0]).fold(f32::INFINITY, f32::min);
        let x_max = q0v.iter().map(|v| v[0]).fold(f32::NEG_INFINITY, f32::max);
        assert!(
            (x_min - (-50.0)).abs() < 1e-4,
            "q0 x_min={x_min} expected -50"
        );
        assert!((x_max - 50.0).abs() < 1e-4, "q0 x_max={x_max} expected +50");
    }

    /// tree3d_billboard_dds_model_emits_flattrunk.
    ///
    /// A quad with a tree StaticDesc whose lod_models[0] ends in .dds (+ a synthetic
    /// .txt sidecar in a temp data_dir) → generate_quad builds FlatTrunk shapes and
    /// writes them into the .bto (assert the .bto exists).
    #[test]
    fn tree3d_billboard_dds_model_emits_bto() {
        use lodgen_native::atlas::atlas::AtlasResult;
        use lodgen_native::descriptors::QuadDesc;
        use lodgen_native::input::WorldspaceInput;
        use lodgen_native::progress::{LodPaths, QuadCtx};
        use lodgen_native::settings::LodSettings;
        use lodgen_native::trees::tree3d::generate_quad;

        let out = std::env::temp_dir().join("lodgen_p3_task4_dds");
        std::fs::create_dir_all(&out).unwrap();

        // Write a synthetic .txt sidecar so parse_billboard_dimensions works
        let dds_model = "textures/trees/testbillboard.dds";
        let txt_path = out.join("textures/trees/testbillboard.txt");
        std::fs::create_dir_all(txt_path.parent().unwrap()).unwrap();
        std::fs::write(&txt_path, "WIDTH=200\nHEIGHT=512\nSHIFTZ=8\n").unwrap();

        let mut stat = make_stat_billboard();
        stat.lod_models[0] = Some(dds_model.to_string());

        let world = WorldspaceInput::from_cells("TestWorld", vec![]);
        let settings = LodSettings::fo4_default();
        let paths = LodPaths {
            data_dirs: vec![out.clone()],
            output_dir: out.clone(),
            source_data_dir: None,
        };
        let atlas = AtlasResult {
            map_path: out.join("atlas.json"),
            diffuse: out.join("atlas.dds"),
            normal: out.join("atlas_n.dds"),
            specular: out.join("atlas_s.dds"),
            atlas_size: (0, 0),
            uv: Default::default(),
            list: Default::default(),
            dds_written: 0,
        };
        let quad = QuadDesc {
            z_order: 0,
            x: 0,
            y: 0,
            quad_level: 4,
            quad_index: 0,
            quad_offset: 4096.0,
            static_indices: Vec::new(),
            statics: vec![stat],
            out_values: Default::default(),
        };

        let ctx = QuadCtx {
            world: &world,
            settings: &settings,
            game: &lodgen_native::game::Game::fo4(),
            paths: &paths,
            level: 4,
        };
        let trees: Vec<&lodgen_native::input::StaticDesc> = quad.statics.iter().collect();
        let outputs = generate_quad(&quad, &ctx, &atlas, &trees).expect("generate_quad billboard");
        // A .bto must have been written
        assert!(
            !outputs.meshes.is_empty(),
            "tree3d_billboard: expected .bto in outputs, got empty meshes"
        );
        let bto_path = &outputs.meshes[0];
        assert!(
            bto_path.exists(),
            "tree3d_billboard: .bto not on disk at {bto_path:?}"
        );
    }

}

// ---------------------------------------------------------------------------
// BillboardManifest serde contract + Rust glue
// ---------------------------------------------------------------------------

#[cfg(test)]
mod task5_manifest {
    use lodgen_native::billboards::BillboardManifest;

    #[test]
    fn manifest_load_fixture() {
        // Load the test fixture JSON file and assert key fields.
        use std::path::PathBuf;
        // tests/fixtures/billboard_manifest.json lives next to the tests/ dir
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("billboard_manifest.json");
        let m = BillboardManifest::load(&fixture).expect("load fixture");
        assert_eq!(m.atlas_w, 1024, "atlas_w");
        assert_eq!(m.entries.len(), 2, "entries count");
        // by_model is case-insensitive
        let entry = m
            .by_model("pinetree01")
            .expect("by_model pinetree01 (lowercase)");
        assert_eq!(entry.index, 0, "pinetree01 index");
        let entry_upper = m
            .by_model("PINETREE01")
            .expect("by_model PINETREE01 (uppercase)");
        assert_eq!(entry_upper.index, 0, "uppercase lookup matches same entry");
    }

}

// ---------------------------------------------------------------------------
// billboard_place::generate_quad — 2D placement → .btt
// ---------------------------------------------------------------------------

#[cfg(test)]
mod task8_billboard_place {
    use lodgen_native::atlas::atlas::AtlasResult;
    use lodgen_native::billboards::{BillboardEntry, BillboardManifest};
    use lodgen_native::descriptors::QuadDesc;
    use lodgen_native::input::{RefInput, WorldspaceInput};

    use lodgen_native::progress::{LodPaths, QuadCtx};
    use lodgen_native::settings::LodSettings;
    use lodgen_native::trees::billboard_place::deterministic_rotation;

    fn make_manifest_two_species() -> BillboardManifest {
        BillboardManifest {
            atlas: "Textures\\Terrain\\LODGen\\World\\WorldTreeLod.dds".to_string(),
            atlas_normal: "Textures\\Terrain\\LODGen\\World\\WorldTreeLod_n.dds".to_string(),
            atlas_w: 512,
            atlas_h: 512,
            entries: vec![
                BillboardEntry {
                    model: "meshes/trees/species_a.dds".to_string(),
                    index: 1,
                    width: 100.0,
                    height: 300.0,
                    shift_z: 4.0,
                    uv_min_x: 0.0,
                    uv_max_x: 0.5,
                    uv_min_y: 0.0,
                    uv_max_y: 1.0,
                },
                BillboardEntry {
                    model: "meshes/trees/species_b.dds".to_string(),
                    index: 2,
                    width: 120.0,
                    height: 400.0,
                    shift_z: 0.0,
                    uv_min_x: 0.5,
                    uv_max_x: 1.0,
                    uv_min_y: 0.0,
                    uv_max_y: 1.0,
                },
            ],
        }
    }

    fn make_ref(ref_id: &str, model: &str, pos: [f32; 3]) -> RefInput {
        RefInput {
            ref_id: ref_id.to_string(),
            ref_flags: 0,
            enable_parent: 0,
            cell: (0, 0),
            pos,
            rot: [0.0; 3],
            scale: 1.5,
            color: 1.0,
            alpha_threshold: 0,
            is_billboard: true,
            is_grass: false,
            base_name: "Tree".to_string(),
            base_flags: 0x1000,
            material_name: String::new(),
            full_model: model.to_string(),
            lod_models: [Some(model.to_string()), None, None, None],
            part_transform: lodgen_native::input::identity_part_transform(),
            part_scale: 1.0,
            material_swap: Default::default(),
        }
    }

    fn make_atlas(out: &std::path::Path) -> AtlasResult {
        AtlasResult {
            map_path: out.join("atlas.json"),
            diffuse: out.join("atlas.dds"),
            normal: out.join("atlas_n.dds"),
            specular: out.join("atlas_s.dds"),
            atlas_size: (0, 0),
            uv: Default::default(),
            list: Default::default(),
            dds_written: 0,
        }
    }

    fn decode_block_types(bytes: &[u8]) -> Vec<(i32, usize)> {
        // Returns Vec<(index, ref_count)> from a serialized .btt block
        let num_types = i32::from_le_bytes(bytes[0..4].try_into().unwrap()) as usize;
        let mut types = Vec::with_capacity(num_types);
        let mut off = 4usize;
        for _ in 0..num_types {
            let idx = i32::from_le_bytes(bytes[off..off + 4].try_into().unwrap());
            off += 4;
            let count = i32::from_le_bytes(bytes[off..off + 4].try_into().unwrap()) as usize;
            off += 4;
            // skip count × 32 bytes per TreeRef (packed TwbLodTES5TreeRef)
            off += count * 32;
            types.push((idx, count));
        }
        types
    }

    fn decode_first_ref_rotation(bytes: &[u8]) -> f32 {
        // Reads the rotation of the first TreeRef in the first type.
        // Block header: [4 numTypes][4 index][4 count] = 12 bytes.
        // TreeRef packed layout: [4 X][4 Y][4 Z][4 Rotation][4 Scale][4 RefFormID][4 Unk1][4 Unk2].
        // Rotation is the 4th f32 → offset 12 + 3*4 = 24.
        let off = 12 + 3 * 4;
        f32::from_le_bytes(bytes[off..off + 4].try_into().unwrap())
    }

    #[test]
    fn billboard_place_buckets_by_index() {
        // A manifest with species A(index 1)/B(index 2); a quad with 3 tree refs (A,A,B)
        // → the written .btt has TreeType index 1 with 2 refs, index 2 with 1 ref.
        // Ref X/Y/Z/Scale come from the StaticDesc pos/scale; rotation == deterministic_rotation(ref_id).
        let out = std::env::temp_dir().join("lodgen_p3_task8_bucket");
        std::fs::create_dir_all(&out).unwrap();

        let manifest = make_manifest_two_species();
        // Write manifest to disk so billboard_place can load it
        let manifest_json = serde_json::to_string(&manifest).unwrap();
        let manifest_path = out.join("World_billboard_manifest.json");
        std::fs::write(&manifest_path, &manifest_json).unwrap();

        let world = WorldspaceInput::from_cells("World", vec![]);
        let mut settings = LodSettings::fo4_default();
        settings.trees.trees_3d = false;
        let paths = LodPaths {
            data_dirs: vec![out.clone()],
            output_dir: out.clone(),
            source_data_dir: None,
        };
        let atlas = make_atlas(&out);

        // 3 refs: A, A, B
        let ref_a1 = make_ref("00000001", "meshes/trees/species_a.dds", [10.0, 20.0, 30.0]);
        let ref_a2 = make_ref("00000002", "meshes/trees/species_a.dds", [40.0, 50.0, 60.0]);
        let ref_b1 = make_ref("00000003", "meshes/trees/species_b.dds", [70.0, 80.0, 90.0]);

        let quad = QuadDesc {
            z_order: 0,
            x: 0,
            y: 0,
            quad_level: 4,
            quad_index: 0,
            quad_offset: 4096.0,
            static_indices: Vec::new(),
            statics: vec![ref_a1.clone(), ref_a2.clone(), ref_b1.clone()],
            out_values: Default::default(),
        };
        let ctx = QuadCtx {
            world: &world,
            settings: &settings,
            game: &lodgen_native::game::Game::fo4(),
            paths: &paths,
            level: 4,
        };

        let trees: Vec<&lodgen_native::input::StaticDesc> = quad.statics.iter().collect();
        let outputs = lodgen_native::trees::billboard_place::generate_quad(
            &quad,
            &ctx,
            &atlas,
            &trees,
            Some(&manifest),
        )
        .expect("billboard_place::generate_quad");

        assert!(!outputs.meshes.is_empty(), "expected .btt in outputs");
        let btt_path = &outputs.meshes[0];
        assert!(btt_path.exists(), ".btt not written to disk");

        let bytes = std::fs::read(btt_path).unwrap();
        let types = decode_block_types(&bytes);
        // Expect two types: index 1 (2 refs), index 2 (1 ref) — sorted by BTreeMap key
        assert_eq!(types.len(), 2, "expected 2 tree types, got {}", types.len());
        let t1 = types
            .iter()
            .find(|(idx, _)| *idx == 1)
            .expect("index 1 missing");
        let t2 = types
            .iter()
            .find(|(idx, _)| *idx == 2)
            .expect("index 2 missing");
        assert_eq!(t1.1, 2, "species A should have 2 refs");
        assert_eq!(t2.1, 1, "species B should have 1 ref");

        // Rotation of first ref for index 1 = deterministic_rotation("00000001")
        // (BTreeMap order: index 1 comes first)
        let expected_rot = deterministic_rotation("00000001");
        let actual_rot = decode_first_ref_rotation(&bytes);
        assert!(
            (actual_rot - expected_rot).abs() < 1e-5,
            "rotation mismatch: expected {expected_rot}, got {actual_rot}"
        );
    }

    #[test]
    fn billboard_place_writes_btt() {
        // Assert that outputs include the .btt at naming::btt path.
        let out = std::env::temp_dir().join("lodgen_p3_task8_writes");
        std::fs::create_dir_all(&out).unwrap();

        let manifest = make_manifest_two_species();
        let manifest_json = serde_json::to_string(&manifest).unwrap();
        std::fs::write(out.join("World_billboard_manifest.json"), &manifest_json).unwrap();

        let world = WorldspaceInput::from_cells("World", vec![]);
        let mut settings = LodSettings::fo4_default();
        settings.trees.trees_3d = false;
        let paths = LodPaths {
            data_dirs: vec![out.clone()],
            output_dir: out.clone(),
            source_data_dir: None,
        };
        let atlas = make_atlas(&out);

        let ref_a = make_ref("AABB0001", "meshes/trees/species_a.dds", [0.0, 0.0, 100.0]);
        let quad = QuadDesc {
            z_order: 0,
            x: -3,
            y: 7,
            quad_level: 4,
            quad_index: 0,
            quad_offset: 4096.0,
            static_indices: Vec::new(),
            statics: vec![ref_a],
            out_values: Default::default(),
        };
        let ctx = QuadCtx {
            world: &world,
            settings: &settings,
            game: &lodgen_native::game::Game::fo4(),
            paths: &paths,
            level: 4,
        };
        let trees: Vec<&lodgen_native::input::StaticDesc> = quad.statics.iter().collect();

        let outputs = lodgen_native::trees::billboard_place::generate_quad(
            &quad,
            &ctx,
            &atlas,
            &trees,
            Some(&manifest),
        )
        .expect("generate_quad");

        assert!(
            !outputs.meshes.is_empty(),
            ".btt path should be in outputs.meshes"
        );
        let btt_path = &outputs.meshes[0];
        // The path ends in .btt
        assert!(
            btt_path.to_string_lossy().ends_with(".btt"),
            ".btt path does not end with .btt: {:?}",
            btt_path
        );
        assert!(
            btt_path.exists(),
            ".btt not written to disk: {:?}",
            btt_path
        );
    }

}

// ---------------------------------------------------------------------------
// wire trees::generate_quad billboard branch + manifest load + driver accounting
// ---------------------------------------------------------------------------

#[cfg(test)]
mod task9_generate_quad_wired {
    use lodgen_native::atlas::atlas::AtlasResult;
    use lodgen_native::billboards::{BillboardEntry, BillboardManifest};
    use lodgen_native::descriptors::QuadDesc;
    use lodgen_native::input::{RefInput, WorldspaceInput};
    use lodgen_native::progress::{LodGenStats, LodPaths, QuadCtx};
    use lodgen_native::settings::LodSettings;
    use lodgen_native::trees::generate_quad;

    fn make_manifest(world: &str, out: &std::path::Path) -> BillboardManifest {
        let manifest = BillboardManifest {
            atlas: format!("Textures\\Terrain\\LODGen\\{world}\\{world}TreeLod.dds"),
            atlas_normal: format!("Textures\\Terrain\\LODGen\\{world}\\{world}TreeLod_n.dds"),
            atlas_w: 512,
            atlas_h: 512,
            entries: vec![BillboardEntry {
                model: "meshes/trees/pinetree.dds".to_string(),
                index: 0,
                width: 150.0,
                height: 512.0,
                shift_z: 8.0,
                uv_min_x: 0.0,
                uv_max_x: 1.0,
                uv_min_y: 0.0,
                uv_max_y: 1.0,
            }],
        };
        // Write the manifest to where generate_quad will look for it
        let manifest_json = serde_json::to_string(&manifest).unwrap();
        std::fs::write(
            out.join(format!("{world}_billboard_manifest.json")),
            &manifest_json,
        )
        .unwrap();
        manifest
    }

    fn make_tree_ref(ref_id: &str, model: &str) -> RefInput {
        RefInput {
            ref_id: ref_id.to_string(),
            ref_flags: 0,
            enable_parent: 0,
            cell: (0, 0),
            pos: [100.0, 200.0, 50.0],
            rot: [0.0; 3],
            scale: 1.0,
            color: 1.0,
            alpha_threshold: 0,
            is_billboard: true,
            is_grass: false,
            base_name: "Pine".to_string(),
            base_flags: 0x1000,
            material_name: String::new(),
            full_model: model.to_string(),
            lod_models: [Some(model.to_string()), None, None, None],
            part_transform: lodgen_native::input::identity_part_transform(),
            part_scale: 1.0,
            material_swap: Default::default(),
        }
    }

    fn make_atlas(out: &std::path::Path) -> AtlasResult {
        AtlasResult {
            map_path: out.join("atlas.json"),
            diffuse: out.join("atlas.dds"),
            normal: out.join("atlas_n.dds"),
            specular: out.join("atlas_s.dds"),
            atlas_size: (0, 0),
            uv: Default::default(),
            list: Default::default(),
            dds_written: 0,
        }
    }

    #[test]
    fn generate_quad_billboard_writes_btt() {
        // trees_3d=false + manifest present → QuadOutputs.meshes contains .btt path.
        let out = std::env::temp_dir().join("lodgen_p3_task9_btt");
        std::fs::create_dir_all(&out).unwrap();

        let world_id = "Task9World";
        let _ = make_manifest(world_id, &out);

        let world = WorldspaceInput::from_cells(world_id, vec![]);
        let mut settings = LodSettings::fo4_default();
        settings.trees.trees_3d = false;
        let paths = LodPaths {
            data_dirs: vec![out.clone()],
            output_dir: out.clone(),
            source_data_dir: None,
        };
        let atlas = make_atlas(&out);

        let tree_ref = make_tree_ref("00AABB01", "meshes/trees/pinetree.dds");
        let quad = QuadDesc {
            z_order: 0,
            x: 0,
            y: 0,
            quad_level: 4,
            quad_index: 0,
            quad_offset: 4096.0,
            static_indices: Vec::new(),
            statics: vec![tree_ref],
            out_values: Default::default(),
        };
        let ctx = QuadCtx {
            world: &world,
            settings: &settings,
            game: &lodgen_native::game::Game::fo4(),
            paths: &paths,
            level: 4,
        };

        let outputs = generate_quad(&quad, &ctx, &atlas).expect("generate_quad billboard");
        assert!(
            !outputs.meshes.is_empty(),
            "billboard mode: expected .btt in outputs"
        );
        assert!(
            outputs
                .meshes
                .iter()
                .any(|p| p.to_string_lossy().ends_with(".btt")),
            "no .btt path in outputs.meshes"
        );
    }

    fn build_object_lod_3d_tree_case(generate_trees: bool) -> LodGenStats {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let out = std::env::temp_dir().join(format!(
            "lodgen_driver_3d_tree_{}_{}",
            std::process::id(),
            unique
        ));
        let txt_path = out.join("textures/trees/driver3d.txt");
        std::fs::create_dir_all(txt_path.parent().unwrap()).unwrap();
        std::fs::write(&txt_path, "WIDTH=200\nHEIGHT=512\nSHIFTZ=8\n").unwrap();

        let tree_ref = make_tree_ref("000000A1", "textures/trees/driver3d.dds");
        let world = WorldspaceInput {
            editor_id: format!("Driver3dTreeWorld{generate_trees}"),
            sw_cell: (0, 0),
            ne_cell: (0, 0),
            water_height: 0.0,
            no_lod_water: false,
            default_diffuse: String::new(),
            default_normal: String::new(),
            cells: vec![],
            refs: vec![tree_ref],
        };
        let paths = LodPaths {
            data_dirs: vec![out.clone()],
            output_dir: out.clone(),
            source_data_dir: None,
        };
        let mut settings = LodSettings::fo4_default();
        settings.global.lod_min = 4;
        settings.global.lod_max = 4;
        settings.global.workers = 1;
        settings.global.write_lodsettings = false;
        settings.global.generate_trees = generate_trees;
        settings.trees.trees_3d = true;

        struct NullProgress;
        impl lodgen_native::progress::Progress for NullProgress {
            fn report(&mut self, _m: &str, _f: f32) {}
        }

        let stats = lodgen_native::driver::build_object_lod(
            &world,
            &settings,
            &lodgen_native::game::Game::fo4(),
            &paths,
            &mut NullProgress,
        )
        .expect("3D-tree object LOD pass");

        std::fs::remove_dir_all(out).unwrap();
        stats
    }

    #[test]
    fn build_object_lod_skips_3d_trees_when_tree_generation_is_disabled() {
        let stats = build_object_lod_3d_tree_case(false);
        assert_eq!(stats.bto, 0, "disabled tree generation must emit no BTO");
        assert_eq!(stats.btt, 0, "disabled tree generation must emit no BTT");
    }

    #[test]
    fn driver_lst_excludes_unreferenced_species() {
        // Pascal writes only species actually placed/atlassed (wbLOD.pas:836-855):
        // a manifest species that is never referenced by any placed tree must NOT
        // appear in the world .lst.
        let out = std::env::temp_dir().join("lodgen_p3_task9_lst_exclude");
        std::fs::create_dir_all(&out).unwrap();

        let world_id = "LstExcludeWorld";
        // Two species: index 0 (referenced) and index 7 (never referenced).
        let manifest = BillboardManifest {
            atlas: format!("Textures\\Terrain\\LODGen\\{world_id}\\{world_id}TreeLod.dds"),
            atlas_normal: format!("Textures\\Terrain\\LODGen\\{world_id}\\{world_id}TreeLod_n.dds"),
            atlas_w: 512,
            atlas_h: 512,
            entries: vec![
                BillboardEntry {
                    model: "meshes/trees/referenced.dds".to_string(),
                    index: 0,
                    width: 150.0,
                    height: 512.0,
                    shift_z: 8.0,
                    uv_min_x: 0.0,
                    uv_max_x: 0.5,
                    uv_min_y: 0.0,
                    uv_max_y: 1.0,
                },
                BillboardEntry {
                    model: "meshes/trees/orphan.dds".to_string(),
                    index: 7,
                    width: 200.0,
                    height: 600.0,
                    shift_z: 0.0,
                    uv_min_x: 0.5,
                    uv_max_x: 1.0,
                    uv_min_y: 0.0,
                    uv_max_y: 1.0,
                },
            ],
        };
        let manifest_json = serde_json::to_string(&manifest).unwrap();
        std::fs::write(
            out.join(format!("{world_id}_billboard_manifest.json")),
            &manifest_json,
        )
        .unwrap();

        // Only reference species index 0 (referenced.dds).
        let tree_ref = make_tree_ref("00000077", "meshes/trees/referenced.dds");

        let world = WorldspaceInput {
            editor_id: world_id.to_string(),
            sw_cell: (0, 0),
            ne_cell: (0, 0),
            water_height: 0.0,
            no_lod_water: false,
            default_diffuse: String::new(),
            default_normal: String::new(),
            cells: vec![],
            refs: vec![tree_ref],
        };

        let mut settings = LodSettings::fo4_default();
        settings.trees.trees_3d = false;
        settings.global.lod_min = 4;
        settings.global.lod_max = 4;
        settings.global.write_lodsettings = false;

        let paths = LodPaths {
            data_dirs: vec![out.clone()],
            output_dir: out.clone(),
            source_data_dir: None,
        };
        let game = lodgen_native::game::Game::fo4();

        struct NullProgress;
        impl lodgen_native::progress::Progress for NullProgress {
            fn report(&mut self, _m: &str, _f: f32) {}
        }

        let atlas = lodgen_native::atlas::atlas::AtlasResult {
            map_path: out.join("atlas.txt"),
            diffuse: out.join("atlas.dds"),
            normal: out.join("atlas_n.dds"),
            specular: out.join("atlas_s.dds"),
            atlas_size: (0, 0),
            uv: Default::default(),
            list: Default::default(),
            dds_written: 0,
        };
        let _stats = lodgen_native::driver::run_trees(
            &world,
            &settings,
            &game,
            &paths,
            &atlas,
            &mut NullProgress,
        )
        .expect("run_trees");

        let lst_rel = lodgen_native::naming::tree_list(world_id);
        let lst_path = out.join(lst_rel.replace('\\', "/"));
        assert!(lst_path.exists(), ".lst not written at {:?}", lst_path);

        // Decode the .lst: [i32 numTrees][per entry: i32 index, 7×4 bytes = 32-byte entry].
        let bytes = std::fs::read(&lst_path).unwrap();
        let num_trees = i32::from_le_bytes(bytes[0..4].try_into().unwrap()) as usize;
        let mut indices = Vec::with_capacity(num_trees);
        let mut off = 4usize;
        for _ in 0..num_trees {
            indices.push(i32::from_le_bytes(bytes[off..off + 4].try_into().unwrap()));
            off += 32; // TwbLodTES5TreeType stride
        }
        assert_eq!(
            num_trees, 1,
            "only the referenced species should be listed, got {num_trees}"
        );
        assert!(
            indices.contains(&0),
            ".lst must contain referenced species index 0"
        );
        assert!(
            !indices.contains(&7),
            ".lst must NOT contain unreferenced species index 7"
        );
    }
}
