use super::*;

fn source_nif() -> NifFile {
    let mut nif = NifFile::new("fo76");
    let root = 0;
    let flags = nif.add_block("BSXFlags", None);
    let graph = nif.add_block("BSBehaviorGraphExtraData", None);
    let manager = nif.add_block("NiControllerManager", None);
    let sequence = nif.add_block("NiControllerSequence", None);
    nif.header.footer_roots = vec![root as i32];
    nif.blocks[root].set_field("Name", NifValue::String("Nuke76Down".into()));
    nif.blocks[root].set_field("Num Extra Data List", NifValue::UInt(2));
    nif.blocks[root].set_field(
        "Extra Data List",
        NifValue::Array(vec![
            NifValue::Ref(flags as i32),
            NifValue::Ref(graph as i32),
        ]),
    );
    nif.blocks[root].set_field("Controller", NifValue::Ref(manager as i32));
    nif.blocks[flags].set_field("Name", NifValue::String("BSX".into()));
    nif.blocks[flags].set_field("Integer Data", NifValue::UInt(1));
    nif.blocks[graph].set_field("Name", NifValue::String("BGED".into()));
    nif.blocks[graph].set_field(
        "Behaviour Graph File",
        NifValue::String("UniqueBehaviors\\Nuke76Synced\\Nuke76Synced.hkx".into()),
    );
    nif.blocks[manager].set_field("Target", NifValue::Ref(root as i32));
    nif.blocks[manager].set_field("Num Controller Sequences", NifValue::UInt(1));
    nif.blocks[manager].set_field(
        "Controller Sequences",
        NifValue::Array(vec![NifValue::Ref(sequence as i32)]),
    );
    nif.blocks[sequence].set_field("Name", NifValue::String("PlayAnim01".into()));
    nif.blocks[sequence].set_field("Manager", NifValue::Ref(manager as i32));
    nif.blocks[sequence].set_field("Stop Time", NifValue::Float(95.1));
    nif
}

fn assert_ready(nif: &NifFile) {
    assert!(
        !nif.blocks
            .iter()
            .any(|block| block.type_name == "BSBehaviorGraphExtraData")
    );
    assert_eq!(
        ref_array(nif.blocks[0].get_field("Extra Data List")),
        vec![1]
    );
    assert_eq!(
        nif.blocks[0]
            .get_field("Num Extra Data List")
            .unwrap()
            .as_i64(),
        1
    );
    assert_eq!(nif.blocks[1].type_name, "BSXFlags");
    let manager = nif.blocks[0].get_field("Controller").unwrap().as_i64() as usize;
    assert_eq!(nif.blocks[manager].type_name, "NiControllerManager");
    let sequence = ref_array(nif.blocks[manager].get_field("Controller Sequences"))[0] as usize;
    assert_eq!(
        string_field(&nif.blocks[sequence], "Name").as_deref(),
        Some("PlayAnim01")
    );
    assert_eq!(
        field_ref(&nif.blocks[sequence], "Manager"),
        Some(manager as i32)
    );
    assert!((value_f64(nif.blocks[sequence].get_field("Stop Time")).unwrap() - 95.1).abs() < 0.001);
}

#[test]
fn nuke_effects_conversion_writes_dense_extra_data_and_playable_sequence() {
    let temp = tempfile::tempdir().unwrap();
    for filename in ["nuke76down.nif", "Nuke76Explosion.NIF"] {
        let source = temp.path().join(filename);
        let output = temp.path().join("converted").join(filename);
        std::fs::write(&source, source_nif().to_bytes().unwrap()).unwrap();
        let report = convert_nif_file(
            &source,
            &output,
            "fo76",
            "fo4",
            None,
            &ConvertFileOptions::default(),
        )
        .unwrap();
        assert!(
            report.supported && report.errors.is_empty(),
            "{:?}",
            report.errors
        );
        assert!(
            report
                .changes
                .iter()
                .any(|change| change.contains("Prepared FO76 nuke effect"))
        );
        assert_ready(&NifFile::load(output).unwrap());
    }
}

#[test]
fn nuke_effects_repair_is_idempotent_and_preserves_sequence_fields() {
    let mut nif = source_nif();
    let before = nif.blocks[4].fields.clone();
    prepare_fo76_nuke_sequences(
        &mut nif,
        Path::new("nuke76down.nif"),
        &mut ConvertFileReport::default(),
    );
    assert_ready(&nif);
    let mut expected = before;
    expected.insert("Manager".into(), NifValue::Ref(2));
    assert_eq!(nif.blocks[3].fields, expected);
    let once = nif.to_bytes().unwrap();
    let mut report = ConvertFileReport::default();
    prepare_fo76_nuke_sequences(&mut nif, Path::new("nuke76down.nif"), &mut report);
    assert!(report.changes.is_empty());
    assert_eq!(nif.to_bytes().unwrap(), once);
}

#[test]
fn nuke_effects_leave_other_meshes_graphs_and_source_game_unchanged() {
    for (filename, graph, sequence) in [
        (
            "other.nif",
            "UniqueBehaviors\\Nuke76Synced\\Nuke76Synced.hkx",
            "PlayAnim01",
        ),
        ("nuke76down.nif", "OtherBehavior.hkx", "PlayAnim01"),
        (
            "nuke76down.nif",
            "UniqueBehaviors\\Nuke76Synced\\Nuke76Synced.hkx",
            "OtherSequence",
        ),
    ] {
        let mut nif = source_nif();
        nif.blocks[2].set_field("Behaviour Graph File", NifValue::String(graph.into()));
        nif.blocks[4].set_field("Name", NifValue::String(sequence.into()));
        let before = nif.to_bytes().unwrap();
        prepare_fo76_nuke_sequences(
            &mut nif,
            Path::new(filename),
            &mut ConvertFileReport::default(),
        );
        assert_eq!(nif.to_bytes().unwrap(), before);
    }
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("nuke76down.nif");
    let output = temp.path().join("copied.nif");
    let bytes = source_nif().to_bytes().unwrap();
    std::fs::write(&source, &bytes).unwrap();
    convert_nif_file(
        &source,
        &output,
        "fo76",
        "fo76",
        None,
        &ConvertFileOptions::default(),
    )
    .unwrap();
    assert_eq!(std::fs::read(output).unwrap(), bytes);
}

#[test]
fn nuke_visuals_hide_only_missile_editor_geometry() {
    let mut nif = source_nif();
    for name in ["EditorMarker:0", "Missile:0", "EmitGeo:0"] {
        let id = nif.add_block("BSTriShape", None);
        nif.blocks[id].set_field("Name", NifValue::String(name.into()));
        nif.blocks[id].set_field("Flags", NifValue::UInt(14));
    }
    let before = nif
        .blocks
        .iter()
        .map(|block| block.fields.clone())
        .collect::<Vec<_>>();
    let mut report = ConvertFileReport::default();
    normalize_fo76_nuke_visuals(&mut nif, Path::new("other.nif"), &mut report);
    assert_eq!(
        nif.blocks
            .iter()
            .map(|block| block.fields.clone())
            .collect::<Vec<_>>(),
        before
    );
    normalize_fo76_nuke_visuals(&mut nif, Path::new("Nuke76Down.NIF"), &mut report);
    assert_eq!(value_u64(nif.blocks[5].get_field("Flags")), Some(15));
    for id in [6, 7] {
        assert_eq!(nif.blocks[id].fields, before[id]);
    }
    assert_eq!(nif.blocks[4].fields, before[4]);
    let once = nif
        .blocks
        .iter()
        .map(|block| block.fields.clone())
        .collect::<Vec<_>>();
    report.changes.clear();
    normalize_fo76_nuke_visuals(&mut nif, Path::new("nuke76down.nif"), &mut report);
    assert_eq!(
        nif.blocks
            .iter()
            .map(|block| block.fields.clone())
            .collect::<Vec<_>>(),
        once
    );
    assert!(report.changes.is_empty());
}

#[test]
fn nuke_visuals_bound_inline_smoke_emission_without_dimming_fire_or_brightening_smoke() {
    let mut nif = source_nif();
    let textures = nif.add_block("BSShaderTextureSet", None);
    nif.blocks[textures].set_field(
        "Textures",
        NifValue::Array(vec![
            NifValue::String("textures\\Effects\\SmokeNuke76PuffsTile_d.dds".into()),
            NifValue::String(String::new()),
            NifValue::String(String::new()),
            NifValue::String("textures\\Effects\\Gradients\\Nuke76SmokeGrad.dds".into()),
        ]),
    );
    for (kind, name, gain) in [
        ("BSLightingShaderProperty", "", 10.0),
        ("BSLightingShaderProperty", "", 5.0),
        ("BSLightingShaderProperty", "", 0.5),
        ("BSLightingShaderProperty", "Materials\\Other.bgsm", 10.0),
        ("BSEffectShaderProperty", "", 20.0),
    ] {
        let id = nif.add_block(kind, None);
        nif.blocks[id].set_field("Name", NifValue::String(name.into()));
        nif.blocks[id].set_field("Texture Set", NifValue::Ref(textures as i32));
        nif.blocks[id].set_field("Emissive Multiple", NifValue::Float(gain));
    }
    let before = nif
        .blocks
        .iter()
        .map(|block| block.fields.clone())
        .collect::<Vec<_>>();
    let mut report = ConvertFileReport::default();
    normalize_fo76_nuke_visuals(&mut nif, Path::new("other.nif"), &mut report);
    assert_eq!(
        nif.blocks
            .iter()
            .map(|block| block.fields.clone())
            .collect::<Vec<_>>(),
        before
    );
    normalize_fo76_nuke_visuals(&mut nif, Path::new("nuke76explosion.nif"), &mut report);
    for id in [6, 7] {
        assert_eq!(
            value_f64(nif.blocks[id].get_field("Emissive Multiple")),
            Some(1.0)
        );
    }
    for id in [4, 5, 8, 9, 10] {
        assert_eq!(nif.blocks[id].fields, before[id]);
    }
    let once = nif
        .blocks
        .iter()
        .map(|block| block.fields.clone())
        .collect::<Vec<_>>();
    report.changes.clear();
    normalize_fo76_nuke_visuals(&mut nif, Path::new("nuke76explosion.nif"), &mut report);
    assert_eq!(
        nif.blocks
            .iter()
            .map(|block| block.fields.clone())
            .collect::<Vec<_>>(),
        once
    );
    assert!(report.changes.is_empty());
}

