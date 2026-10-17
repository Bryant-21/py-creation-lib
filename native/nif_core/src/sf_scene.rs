use std::collections::{HashMap, HashSet};

use crate::model::{NifBlock, NifFile, NifValue};

pub(crate) fn preserve_weapon_scene(
    source: &NifFile,
    dest: &mut NifFile,
    root: usize,
) -> Result<HashMap<usize, usize>, String> {
    let mut parents = HashMap::new();
    let mut visited = HashSet::new();
    copy_node(source, dest, root, 0, &mut parents, &mut visited)?;
    Ok(parents)
}

fn copy_node(
    source: &NifFile,
    dest: &mut NifFile,
    id: usize,
    output: usize,
    parents: &mut HashMap<usize, usize>,
    visited: &mut HashSet<usize>,
) -> Result<(), String> {
    if !visited.insert(id) {
        return Err("Starfield weapon scene has shared or cyclic nodes".into());
    }
    let block = source
        .get_block(id)
        .ok_or("Starfield weapon scene has a missing node")?;
    for key in ["Name", "Translation", "Rotation", "Scale"] {
        if let Some(value) = block.get_field(key) {
            let value = if key == "Translation" {
                scale_translation(value)?
            } else {
                value.clone()
            };
            dest.blocks[output].set_field(key, value);
        }
    }
    dest.blocks[output].set_field("Flags", NifValue::UInt(14));
    copy_extras(source, block, dest, output)?;
    let Some(NifValue::Array(children)) = block.get_field("Children") else {
        return Ok(());
    };
    for child in children {
        let NifValue::Ref(child) = child else {
            return Err("invalid weapon child reference".into());
        };
        if *child < 0 {
            continue;
        }
        let child = *child as usize;
        let child_block = source.get_block(child).ok_or("missing weapon child")?;
        parents.insert(child, output);
        if child_block.type_name == "BSGeometry" {
            continue;
        }
        if child_block.get_field("Children").is_none() {
            continue;
        }
        let new_id = dest.add_block("NiNode", None);
        attach(dest, output, new_id);
        copy_node(source, dest, child, new_id, parents, visited)?;
    }
    Ok(())
}

pub(crate) fn copy_shape_transform(source: &NifBlock, dest: &mut NifBlock) -> Result<(), String> {
    for key in ["Translation", "Rotation", "Scale"] {
        if let Some(value) = source.get_field(key) {
            dest.set_field(
                key,
                if key == "Translation" {
                    scale_translation(value)?
                } else {
                    value.clone()
                },
            );
        }
    }
    Ok(())
}

fn copy_extras(
    source: &NifFile,
    block: &NifBlock,
    dest: &mut NifFile,
    output: usize,
) -> Result<(), String> {
    let Some(NifValue::Array(extras)) = block.get_field("Extra Data List") else {
        return Ok(());
    };
    let mut copied = Vec::new();
    for extra in extras {
        let Some(extra) = (match extra {
            NifValue::Ref(id) if *id >= 0 => source.get_block(*id as usize),
            _ => None,
        }) else {
            continue;
        };
        if !matches!(
            extra.type_name.as_str(),
            "BSConnectPoint::Parents" | "BSConnectPoint::Children" | "NiStringExtraData"
        ) {
            continue;
        }
        let mut fields = extra.fields.clone();
        if extra.type_name == "BSConnectPoint::Parents" {
            if let Some(NifValue::Array(points)) = fields.get_mut("Connect Points") {
                for point in points {
                    let NifValue::Struct(point) = point else {
                        return Err("invalid weapon connect point".into());
                    };
                    if let Some(translation) = point.get_mut("Translation") {
                        *translation = scale_translation(translation)?;
                    }
                }
            }
        }
        let id = dest.add_block(&extra.type_name, Some(fields));
        copied.push(NifValue::Ref(id as i32));
    }
    dest.blocks[output].set_field("Num Extra Data List", NifValue::UInt(copied.len() as u64));
    dest.blocks[output].set_field("Extra Data List", NifValue::Array(copied));
    Ok(())
}

fn scale_translation(value: &NifValue) -> Result<NifValue, String> {
    const SCALE: f64 = 69.99125;
    match value {
        NifValue::Vec3(v) => Ok(NifValue::Vec3(v.map(|v| v * SCALE as f32))),
        NifValue::Struct(fields) => {
            let mut fields = fields.clone();
            for key in ["x", "y", "z"] {
                match fields.get_mut(key) {
                    Some(NifValue::Float(value)) if value.is_finite() => *value *= SCALE,
                    _ => return Err("invalid weapon translation".into()),
                }
            }
            Ok(NifValue::Struct(fields))
        }
        _ => Err("invalid weapon translation representation".into()),
    }
}

fn attach(nif: &mut NifFile, parent: usize, child: usize) {
    let block = &mut nif.blocks[parent];
    let mut children = match block.get_field("Children") {
        Some(NifValue::Array(v)) => v.clone(),
        _ => Vec::new(),
    };
    children.push(NifValue::Ref(child as i32));
    block.set_field("Num Children", NifValue::UInt(children.len() as u64));
    block.set_field("Children", NifValue::Array(children));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    #[ignore = "requires extracted Starfield assets"]
    fn real_weapon_rejects_a_missing_part_without_writing_partial_geometry() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let mut source = NifFile::load(
            repo.join("extracted/starfield/meshes/weapons/stingerpistol/stinger.nif"),
        )
        .unwrap();
        let geometry = source.find_blocks("BSGeometry")[0];
        source.blocks[geometry].set_field("Meshes", NifValue::Array(Vec::new()));
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("broken_receiver.nif");
        let output = dir.path().join("converted.nif");
        source.save(Some(input.clone())).unwrap();
        let opts = crate::sf_convert::SfConvertOptions {
            geometries_root: repo.join("extracted/starfield/geometries"),
            material_path_rewriter: None,
            target_skeleton: None,
            translation_maps_dir: None,
            weapon_role: Some("gun".into()),
            cloth_companions: Vec::new(),
        };
        let error = crate::sf_convert::convert_starfield_nif(&input, &output, &opts).unwrap_err();
        assert!(
            error.contains("incomplete Starfield weapon/outfit geometry"),
            "{error}"
        );
        assert!(!output.exists());
    }

    #[test]
    #[ignore = "requires extracted Starfield assets"]
    fn real_weapon_receivers_keep_connectors_and_animated_nodes() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let dir = tempfile::tempdir().unwrap();
        for (index, model) in [
            "weapons/bullpup/bullpupdummyreceiver.nif",
            "weapons/stingerpistol/stinger.nif",
            "weapons/shotty/shotty.nif",
        ]
        .iter()
        .enumerate()
        {
            let source_path = repo.join("extracted/starfield/meshes").join(model);
            let output = dir.path().join(format!("weapon{index}.nif"));
            let source = NifFile::load(source_path.clone()).unwrap();
            let opts = crate::sf_convert::SfConvertOptions {
                geometries_root: repo.join("extracted/starfield/geometries"),
                material_path_rewriter: None,
                target_skeleton: None,
                translation_maps_dir: None,
                weapon_role: Some("gun".into()),
                cloth_companions: Vec::new(),
            };
            crate::sf_convert::convert_starfield_nif(&source_path, &output, &opts).unwrap();
            let converted = NifFile::load(output).unwrap();
            for kind in ["BSConnectPoint::Parents", "BSConnectPoint::Children"] {
                assert_eq!(
                    source.find_blocks(kind).len(),
                    converted.find_blocks(kind).len(),
                    "{model} {kind}"
                );
            }
            for original in source.find_blocks("NiNode") {
                let node = &source.blocks[original];
                let name = node.get_field("Name");
                let target = converted
                    .find_blocks("NiNode")
                    .into_iter()
                    .map(|id| &converted.blocks[id])
                    .find(|block| block.get_field("Name") == name)
                    .unwrap_or_else(|| panic!("{model} lost node {name:?}"));
                if let Some(translation) = node.get_field("Translation") {
                    let mut expected = node.clone();
                    expected.set_field("Translation", scale_translation(translation).unwrap());
                    let expected = crate::skeleton_repose::local_of(&expected);
                    let actual = crate::skeleton_repose::local_of(target);
                    for axis in 0..3 {
                        assert!((expected[axis][3] - actual[axis][3]).abs() < 0.0001);
                    }
                }
            }
            if index == 0 {
                assert!(converted.find_blocks("BSTriShape").is_empty());
                let physics = converted.find_blocks("bhkPhysicsSystem");
                assert_eq!(
                    physics.len(),
                    1,
                    "geometry-free receiver must retain physics"
                );
                let blob = crate::cloth::byte_array_to_bytes(
                    converted.blocks[physics[0]]
                        .get_field("Binary Data")
                        .unwrap(),
                )
                .unwrap();
                let metadata =
                    crate::fo76_collision::SourceCollisionContext::new(&blob).body_metadata(0);
                assert_eq!(metadata.layer, Some(5));
                let context = havok_native::collision::SourcePhysicsSystemContext::new(&blob);
                let summary = context.collision_summary().unwrap();
                let summary: serde_json::Value = serde_json::from_str(&summary).unwrap();
                let body = &summary["bodies"][0];
                assert_eq!(body["layer"], 5);
                assert_eq!(body["flags"], 128);
                assert_eq!(body["motion_id"], 0);
                assert!((body["inverse_mass"].as_f64().unwrap() - 1.0 / 3.0).abs() < 1e-6);
            } else {
                assert!(!converted.find_blocks("BSTriShape").is_empty());
            }
        }
    }
}
