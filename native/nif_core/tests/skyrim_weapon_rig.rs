use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;

use nif_core_native::convert_file::{ConvertFileOptions, convert_nif_file};
use nif_core_native::model::{NifFile, NifValue};
use nif_core_native::skeleton_repose::collect_bind_matrices_by_name;
use nif_core_native::skin::source::parse_skin_chain;

#[test]
#[ignore = "requires extracted Skyrim SE and Fallout 4 game assets"]
fn bows_and_crossbows_keep_their_local_rigs_without_weapon_metadata() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let output = tempfile::tempdir().unwrap();
    let options =
        ConvertFileOptions {
            translation_maps_dir: Some(
                root.join("bacup/py_bacup_lib/native/conversion/src/embedded/translation_maps"),
            ),
            target_skeleton: Some(
                root.join("extracted/fo4/meshes/actors/character/characterassets/skeleton.nif"),
            ),
            auto_skin_reference_body: Some(
                root.join("extracted/fo4/meshes/actors/character/characterassets/malebody.nif"),
            ),
            emit_first_person: true,
            first_person_reference: Some(root.join(
                "extracted/fo4/meshes/actors/character/characterassets/1stpersonmalebody.nif",
            )),
            ..Default::default()
        };
    for relative in [
        "weapons/steel/steelbow.nif",
        "dlc01/weapons/crossbow/crossbow.nif",
        "dlc01/weapons/dwarven crossbow/dwarvencrossbow.nif",
    ] {
        let source_path = root.join("extracted/skyrimse/meshes").join(relative);
        let source = NifFile::load(&source_path).unwrap();
        let mut expected_bones = BTreeSet::new();
        let mut expected_binds = HashMap::new();
        for block in &source.blocks {
            if let Some(skin) = parse_skin_chain(&source, block.block_id).unwrap() {
                for (bone, bind) in skin.bones.iter().zip(skin.bone_transforms) {
                    expected_binds.insert(bone.name.clone(), bind);
                }
                expected_bones.extend(skin.bones.into_iter().map(|bone| bone.name));
            }
        }
        assert!(!expected_bones.is_empty());
        let destination = output.path().join(relative);
        let report = convert_nif_file(
            &source_path,
            &destination,
            "skyrimse",
            "fo4",
            Some(&output.path().join("materials")),
            &options,
        )
        .unwrap();
        assert!(
            report.supported && report.errors.is_empty(),
            "{relative}: {report:?}"
        );
        assert!(report.shapes_skinned > 0, "{relative}: {report:?}");
        assert_eq!(report.bones_dropped_unmapped, 0, "{relative}: {report:?}");
        assert_eq!(report.vertices_morph_weighted, 0, "{relative}: {report:?}");
        let converted = NifFile::load(&destination).unwrap();
        let binds = collect_bind_matrices_by_name(&converted);
        assert_eq!(
            binds.keys().cloned().collect::<BTreeSet<_>>(),
            expected_bones,
            "{relative}"
        );
        for (name, matrix) in &binds {
            let original = &expected_binds[name];
            for row in 0..3 {
                for column in 0..3 {
                    assert!(
                        (matrix[row][column] - original.rotation[row][column] as f64).abs() < 1e-5,
                        "{relative}: {name} rotation"
                    );
                }
                assert!(
                    (matrix[row][3] - original.translation[row] as f64).abs() < 1e-5,
                    "{relative}: {name} translation"
                );
            }
            let original_node = source.blocks.iter().find(|block| matches!(block.get_field("Name"), Some(NifValue::String(candidate)) if candidate == name)).unwrap();
            let target_node = converted.blocks.iter().find(|block| matches!(block.get_field("Name"), Some(NifValue::String(candidate)) if candidate == name)).unwrap();
            for field in ["Translation", "Rotation", "Scale"] {
                assert_eq!(
                    original_node.get_field(field),
                    target_node.get_field(field),
                    "{relative}: {name}.{field}"
                );
            }
        }
        let attachment = if relative.contains("crossbow") {
            "Weapon"
        } else {
            "WeaponLeft"
        };
        assert!(converted.blocks.iter().any(|block| {
            block.type_name == "NiStringExtraData"
                && matches!(block.get_field("Name"), Some(NifValue::String(name)) if name == "Prn")
                && matches!(block.get_field("String Data"), Some(NifValue::String(name)) if name == attachment)
        }));
        for shape in converted
            .blocks
            .iter()
            .filter(|block| block.type_name == "BSSubIndexTriShape")
        {
            let Some(NifValue::Ref(skin_id)) = shape.get_field("Skin") else {
                panic!("missing skin")
            };
            let Some(NifValue::Array(bones)) =
                converted.blocks[*skin_id as usize].get_field("Bones")
            else {
                panic!("missing palette")
            };
            let Some(NifValue::Array(vertices)) = shape.get_field("Vertex Data") else {
                panic!("missing vertices")
            };
            assert!(!vertices.is_empty());
            for vertex in vertices {
                let NifValue::Struct(fields) = vertex else {
                    panic!("vertex encoding")
                };
                let NifValue::Array(weights) = &fields["Bone Weights"] else {
                    panic!("weight encoding")
                };
                let NifValue::Array(indices) = &fields["Bone Indices"] else {
                    panic!("index encoding")
                };
                let mut total = 0.0;
                for (weight, index) in weights.iter().zip(indices) {
                    let NifValue::Float(weight) = weight else {
                        panic!("weight")
                    };
                    assert!(weight.is_finite() && (0.0..=1.0).contains(weight));
                    total += weight;
                    if *weight > 0.0 {
                        assert!(
                            index.as_i64() >= 0 && (index.as_i64() as usize) < bones.len(),
                            "{relative}"
                        );
                    }
                }
                assert!(
                    (total - 1.0).abs() < 0.002,
                    "{relative}: weight sum {total}"
                );
            }
        }
        eprintln!(
            "{relative}: {} preserved bones, {} skinned vertices",
            binds.len(),
            report.vertices_repacked
        );
    }
}
