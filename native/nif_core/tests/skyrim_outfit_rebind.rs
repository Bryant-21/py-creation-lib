use std::path::PathBuf;

use nif_core_native::convert_file::{ConvertFileOptions, convert_nif_file};
use nif_core_native::model::{NifFile, NifValue};
use nif_core_native::skeleton_repose::{collect_bind_matrices_by_name, skeleton_bind_offsets};

fn number(value: &NifValue) -> f64 {
    match value {
        NifValue::Float(value) => *value,
        NifValue::UInt(value) => *value as f64,
        NifValue::Int(value) => *value as f64,
        _ => panic!("expected numeric skin value: {value:?}"),
    }
}

#[test]
#[ignore = "requires extracted Skyrim SE and Fallout 4 game assets"]
fn skyrim_outfit_variants_keep_valid_skin_after_fo4_save_reload() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let skeleton_path =
        root.join("extracted/fo4/meshes/actors/character/characterassets/skeleton.nif");
    let skeleton = NifFile::load(&skeleton_path).unwrap();
    let first_skeleton_path =
        root.join("extracted/fo4/meshes/actors/character/_1stperson/characterassets/skeleton.nif");
    let first_skeleton = NifFile::load(&first_skeleton_path).unwrap();
    let output = tempfile::tempdir().unwrap();
    let options = ConvertFileOptions {
        translation_maps_dir: Some(
            root.join("bacup/py_bacup_lib/native/conversion/src/embedded/translation_maps"),
        ),
        target_skeleton: Some(skeleton_path),
        ..Default::default()
    };

    for (relative, first_person) in [
        ("armor/iron/male/cuirasslight_1.nif", false),
        ("armor/iron/f/cuirasslight_1.nif", false),
        ("armor/iron/male/1stpersoncuirasslight_1.nif", true),
        ("armor/iron/f/1stpersoncuirasslight_1.nif", true),
        ("armor/dragonbone/dragonbonegauntlets_1.nif", false),
        ("armor/dragonbone/dragonbonegauntlets_1.nif", true),
        ("armor/dragonbone/dragonbonegauntletsf_1.nif", false),
        ("armor/dragonbone/dragonbonegauntletsf_1.nif", true),
    ] {
        let mut options = options.clone();
        if first_person {
            options.target_skeleton = Some(first_skeleton_path.clone());
        }
        let destination = output
            .path()
            .join(if first_person { "first" } else { "third" })
            .join(relative);
        let report = convert_nif_file(
            &root.join("extracted/skyrimse/meshes").join(relative),
            &destination,
            "skyrimse",
            "fo4",
            Some(&output.path().join("materials")),
            &options,
        )
        .unwrap();
        assert!(report.supported, "{relative}: {report:?}");
        assert!(report.errors.is_empty(), "{relative}: {report:?}");
        assert!(report.shapes_skinned > 0, "{relative}: {report:?}");
        let mesh = NifFile::load(&destination).unwrap();
        let binds = collect_bind_matrices_by_name(&mesh);
        assert!(!binds.is_empty(), "{relative}");
        let expected_skeleton = if first_person {
            &first_skeleton
        } else {
            &skeleton
        };
        let offsets = skeleton_bind_offsets(expected_skeleton, &binds);
        assert_eq!(offsets.len(), binds.len(), "unmatched bones in {relative}");
        let maximum = offsets
            .iter()
            .flatten()
            .map(|v| v.abs())
            .fold(0.0, f64::max);
        assert!(maximum < 0.001, "{relative}: bind offset {maximum}");
        let mut vertices = 0;
        for shape in mesh
            .blocks
            .iter()
            .filter(|block| block.type_name == "BSSubIndexTriShape")
        {
            let Some(NifValue::Ref(skin_id)) = shape.get_field("Skin") else {
                panic!("missing skin in {relative}");
            };
            let Some(NifValue::Array(bones)) = mesh.blocks[*skin_id as usize].get_field("Bones")
            else {
                panic!("missing bones in {relative}");
            };
            let Some(NifValue::Array(data)) = shape.get_field("Vertex Data") else {
                panic!("missing vertices in {relative}");
            };
            vertices += data.len();
            for vertex in data {
                let NifValue::Struct(fields) = vertex else {
                    panic!("vertex encoding");
                };
                let NifValue::Array(weights) = &fields["Bone Weights"] else {
                    panic!("weight encoding");
                };
                let NifValue::Array(indices) = &fields["Bone Indices"] else {
                    panic!("index encoding");
                };
                assert_eq!(weights.len(), 4);
                assert_eq!(indices.len(), 4);
                let total: f64 = weights.iter().map(number).sum();
                assert!(
                    (total - 1.0).abs() < 0.002,
                    "{relative}: weight sum {total}"
                );
                for (weight, index) in weights.iter().zip(indices) {
                    assert!((0.0..=1.0).contains(&number(weight)), "{relative}");
                    if number(weight) > 0.0 {
                        assert!((number(index) as usize) < bones.len(), "{relative}");
                    }
                }
            }
        }
        assert_eq!(vertices, report.vertices_repacked, "{relative}");
        eprintln!(
            "{relative}: {} shapes, {vertices} vertices, {} binds, target residual {maximum:.6}",
            report.shapes_skinned,
            binds.len()
        );
    }
}
