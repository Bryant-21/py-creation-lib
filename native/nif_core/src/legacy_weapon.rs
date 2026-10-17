use crate::model::{NifFile, NifValue};

const BASIS: [[f32; 3]; 3] = [[0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];

pub(crate) fn align_gun_axes(nif: &mut NifFile) {
    let roots = nif.header.footer_roots.clone();
    for root_id in roots.into_iter().filter_map(|id| usize::try_from(id).ok()) {
        let Some(root) = nif.blocks.get(root_id) else {
            continue;
        };
        let children = root
            .get_field("Children")
            .cloned()
            .unwrap_or(NifValue::Array(vec![]));
        let collision = root
            .get_field("Collision Object")
            .cloned()
            .unwrap_or(NifValue::Ref(-1));
        if matches!(&children, NifValue::Array(values) if values.iter().any(|value| {
            nif.blocks.get(value.as_i64() as usize).is_some_and(|child| {
                child.get_field("Name") == Some(&NifValue::String("LegacyWeaponBasis".into()))
            })
        })) {
            continue;
        }
        // A common parent rotates render geometry, collision and skin together;
        // retaining their local frame keeps the existing inverse binds valid.
        let basis_id = nif.add_block("NiNode", None);
        let basis = &mut nif.blocks[basis_id];
        basis.set_field("Name", NifValue::String("LegacyWeaponBasis".into()));
        basis.set_field("Flags", NifValue::UInt(14));
        basis.set_field("Rotation", NifValue::Matrix33(BASIS));
        basis.set_field("Translation", NifValue::Vec3([0.0; 3]));
        basis.set_field("Scale", NifValue::Float(1.0));
        basis.set_field("Controller", NifValue::Ref(-1));
        let count = if let NifValue::Array(values) = &children {
            values.len()
        } else {
            0
        };
        basis.set_field("Num Children", NifValue::UInt(count as u64));
        basis.set_field("Children", children);
        basis.set_field("Collision Object", collision.clone());
        let root = &mut nif.blocks[root_id];
        root.set_field(
            "Children",
            NifValue::Array(vec![NifValue::Ref(basis_id as i32)]),
        );
        root.set_field("Num Children", NifValue::UInt(1));
        root.set_field("Collision Object", NifValue::Ref(-1));
        if let Some(collision) = nif.blocks.get_mut(collision.as_i64() as usize) {
            collision.set_field("Target", NifValue::Ref(basis_id as i32));
        }
        for block in &mut nif.blocks {
            if matches!(
                block.type_name.as_str(),
                "BSSkin::Instance" | "NiSkinInstance" | "BSDismemberSkinInstance"
            ) {
                if block
                    .get_field("Skeleton Root")
                    .is_some_and(|value| value.as_i64() == root_id as i64)
                {
                    block.set_field("Skeleton Root", NifValue::Ref(basis_id as i32));
                }
                if let Some(NifValue::Array(bones)) = block.fields.get_mut("Bones") {
                    for bone in bones {
                        if bone.as_i64() == root_id as i64 {
                            *bone = NifValue::Ref(basis_id as i32);
                        }
                    }
                }
            }
        }
    }
    nif.rebuild_header();
}

#[cfg(test)]
fn read_rotation(value: &NifValue) -> Option<[[f32; 3]; 3]> {
    match value {
        NifValue::Matrix33(rotation) => Some(*rotation),
        NifValue::Struct(fields) => {
            let keys = [
                ["m11", "m21", "m31"],
                ["m12", "m22", "m32"],
                ["m13", "m23", "m33"],
            ];
            let mut rotation = [[0.0; 3]; 3];
            for row in 0..3 {
                for column in 0..3 {
                    let NifValue::Float(value) = fields.get(keys[row][column])? else {
                        return None;
                    };
                    rotation[row][column] = *value as f32;
                }
            }
            Some(rotation)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires locally extracted Fallout New Vegas weapon assets"]
    fn converts_local_9mm_pistol_with_forward_muzzle() {
        let source = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../extracted/fnv/meshes/weapons/1handpistol/9mm.nif");
        let temp = tempfile::tempdir().unwrap();
        let output = temp.path().join("9mm.nif");
        let report = crate::convert_file::convert_nif_file(
            &source,
            &output,
            "fnv",
            "fo4",
            None,
            &crate::convert_file::ConvertFileOptions {
                weapon_role: Some("gun".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(report.supported, "{:?}", report.errors);
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        let converted = NifFile::load(output).unwrap();
        let muzzle = converted
            .blocks
            .iter()
            .find(|block| {
                block.get_field("Name") == Some(&NifValue::String("ProjectileNode".into()))
            })
            .unwrap();
        let basis = converted
            .blocks
            .iter()
            .find(|block| {
                block.get_field("Name") == Some(&NifValue::String("LegacyWeaponBasis".into()))
            })
            .unwrap();
        assert_eq!(
            basis.get_field("Rotation").and_then(read_rotation),
            Some(BASIS)
        );
        let translation = match muzzle.get_field("Translation").unwrap() {
            NifValue::Vec3(value) => *value,
            NifValue::Struct(fields) => ["x", "y", "z"].map(|key| {
                let NifValue::Float(value) = fields.get(key).unwrap() else {
                    panic!()
                };
                *value as f32
            }),
            _ => panic!(),
        };
        let world = BASIS.map(|row| row.iter().zip(translation).map(|(a, b)| a * b).sum::<f32>());
        assert!(world[0].abs() < 0.001);
        assert!((world[1] - 15.4404).abs() < 0.001);
        assert!((world[2] - 5.1554).abs() < 0.001);
        let rotation = muzzle
            .get_field("Rotation")
            .and_then(read_rotation)
            .unwrap();
        for row in 0..3 {
            for column in 0..3 {
                let value: f32 = (0..3)
                    .map(|index| BASIS[row][index] * rotation[index][column])
                    .sum();
                assert!(
                    (value - if row == column { 1.0 } else { 0.0 }).abs() < 0.001,
                    "converted emitter rotation: {rotation:?}, world[{row}][{column}]={value}"
                );
            }
        }
    }

    #[test]
    fn gun_basis_keeps_muzzle_skin_and_collision_in_one_frame() {
        let mut nif = NifFile::new("fo4");
        let root = nif.add_block("NiNode", None);
        let muzzle = nif.add_block("NiNode", None);
        let skin = nif.add_block("BSSkin::Instance", None);
        let collision = nif.add_block("bhkNPCollisionObject", None);
        nif.header.footer_roots = vec![root as i32];
        nif.blocks[root].set_field("Name", NifValue::String("Weapon".into()));
        nif.blocks[root].set_field(
            "Children",
            NifValue::Array(vec![NifValue::Ref(muzzle as i32)]),
        );
        nif.blocks[root].set_field("Collision Object", NifValue::Ref(collision as i32));
        nif.blocks[muzzle].set_field("Name", NifValue::String("ProjectileNode".into()));
        nif.blocks[muzzle].set_field("Translation", NifValue::Vec3([15.4404, 5.1554, 0.0]));
        nif.blocks[muzzle].set_field(
            "Rotation",
            NifValue::Matrix33([[0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]]),
        );
        nif.blocks[skin].set_field("Skeleton Root", NifValue::Ref(root as i32));
        nif.blocks[skin].set_field("Bones", NifValue::Array(vec![NifValue::Ref(root as i32)]));
        align_gun_axes(&mut nif);
        let basis = nif.blocks.len() - 1;
        assert_eq!(
            nif.blocks[basis].get_field("Rotation"),
            Some(&NifValue::Matrix33(BASIS))
        );
        assert_eq!(
            nif.blocks[skin].get_field("Skeleton Root"),
            Some(&NifValue::Ref(basis as i32))
        );
        assert_eq!(
            nif.blocks[skin].get_field("Bones"),
            Some(&NifValue::Array(vec![NifValue::Ref(basis as i32)]))
        );
        assert_eq!(
            nif.blocks[collision].get_field("Target"),
            Some(&NifValue::Ref(basis as i32))
        );
        let NifValue::Vec3(translation) = nif.blocks[muzzle].get_field("Translation").unwrap()
        else {
            panic!()
        };
        let world = BASIS.map(|row| row.iter().zip(translation).map(|(a, b)| a * b).sum::<f32>());
        assert_eq!(world, [0.0, 15.4404, 5.1554]);
        let NifValue::Matrix33(rotation) = nif.blocks[muzzle].get_field("Rotation").unwrap() else {
            panic!()
        };
        for row in 0..3 {
            for column in 0..3 {
                let value: f32 = (0..3)
                    .map(|index| BASIS[row][index] * rotation[index][column])
                    .sum();
                assert_eq!(value, if row == column { 1.0 } else { 0.0 });
            }
        }
        let count = nif.blocks.len();
        align_gun_axes(&mut nif);
        assert_eq!(nif.blocks.len(), count);
    }
}
