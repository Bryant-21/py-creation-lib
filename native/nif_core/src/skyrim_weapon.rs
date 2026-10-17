use std::collections::HashSet;

use crate::convert_file::ConvertFileOptions;
use crate::model::{NifFile, NifValue};
use crate::skin::LegacySkinPolicy;

pub(crate) fn source_rig_options(
    nif: &mut NifFile,
    options: &ConvertFileOptions,
) -> Option<ConvertFileOptions> {
    let marker = nif.blocks.iter().find(|block| {
        block.type_name == "NiStringExtraData"
            && matches!(block.get_field("Name"), Some(NifValue::String(name)) if name == "Prn")
            && matches!(block.get_field("String Data"), Some(NifValue::String(name)) if name == "WeaponBow")
    })?.block_id;
    let rig_root = nif.blocks.iter().find(|block| {
        block.type_name == "NiNode"
            && matches!(block.get_field("Name"), Some(NifValue::String(name)) if name == "Bow_MidBone" || name == "CrossbowRoot")
    })?.block_id;
    let mut owned = HashSet::new();
    let mut pending = vec![rig_root];
    while let Some(id) = pending.pop() {
        if !owned.insert(id) {
            continue;
        }
        if let Some(NifValue::Array(children)) = nif.get_block(id)?.get_field("Children") {
            pending.extend(children.iter().filter_map(|child| match child {
                NifValue::Ref(id) if *id >= 0 => Some(*id as usize),
                _ => None,
            }));
        }
    }
    let mut skinned = false;
    for skin in nif.blocks.iter().filter(|block| {
        matches!(
            block.type_name.as_str(),
            "NiSkinInstance" | "BSDismemberSkinInstance"
        )
    }) {
        let Some(NifValue::Ref(root)) = skin.get_field("Skeleton Root") else {
            return None;
        };
        if !nif.header.footer_roots.contains(root) {
            return None;
        }
        let Some(NifValue::Array(bones)) = skin.get_field("Bones") else {
            return None;
        };
        if bones.is_empty() || !bones.iter().all(|bone| matches!(bone, NifValue::Ref(id) if *id >= 0 && owned.contains(&(*id as usize)))) {
            return None;
        }
        skinned = true;
    }
    if !skinned {
        return None;
    }
    // Skyrim's bow grip follows Shield; Weapon is its arrow/nock. Crossbows
    // follow Weapon. WeaponBow is the source holster for both mesh families.
    let attachment = if matches!(nif.blocks[rig_root].get_field("Name"), Some(NifValue::String(name)) if name == "Bow_MidBone")
    {
        "WeaponLeft"
    } else {
        "Weapon"
    };
    nif.blocks[marker].set_field("String Data", NifValue::String(attachment.into()));
    let mut options = options.clone();
    options.skin_policy = LegacySkinPolicy::PreserveSourceRig;
    options.target_skeleton = None;
    options.auto_skin_reference_body = None;
    options.emit_first_person = false;
    options.first_person_reference = None;
    Some(options)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bow_fixture() -> NifFile {
        let mut nif = NifFile::new("skyrimse");
        let marker = nif.add_block("NiStringExtraData", None);
        nif.blocks[marker].set_field("Name", NifValue::String("Prn".into()));
        nif.blocks[marker].set_field("String Data", NifValue::String("WeaponBow".into()));
        let root = nif.add_block("NiNode", None);
        nif.blocks[root].set_field("Name", NifValue::String("Bow_MidBone".into()));
        let skin = nif.add_block("NiSkinInstance", None);
        nif.blocks[skin].set_field("Skeleton Root", NifValue::Ref(0));
        nif.blocks[skin].set_field("Bones", NifValue::Array(vec![NifValue::Ref(root as i32)]));
        nif
    }

    #[test]
    fn bow_rig_disables_humanoid_rebinding_and_body_weight_transfer() {
        let mut nif = bow_fixture();
        let original = ConvertFileOptions {
            target_skeleton: Some("human.nif".into()),
            auto_skin_reference_body: Some("body.nif".into()),
            emit_first_person: true,
            first_person_reference: Some("arms.nif".into()),
            ..Default::default()
        };
        let selected = source_rig_options(&mut nif, &original).unwrap();
        assert_eq!(selected.skin_policy, LegacySkinPolicy::PreserveSourceRig);
        assert!(selected.target_skeleton.is_none());
        assert!(selected.auto_skin_reference_body.is_none());
        assert!(!selected.emit_first_person);
        assert!(selected.first_person_reference.is_none());
        assert!(original.target_skeleton.is_some());
        assert_eq!(
            nif.blocks[1].get_field("String Data"),
            Some(&NifValue::String("WeaponLeft".into()))
        );
    }

    #[test]
    fn bow_marker_cannot_preserve_an_external_or_humanoid_palette() {
        let mut nif = bow_fixture();
        let human = nif.add_block("NiNode", None);
        nif.blocks[human].set_field("Name", NifValue::String("NPC R Hand [RHnd]".into()));
        let skin = nif
            .blocks
            .iter_mut()
            .find(|block| block.type_name == "NiSkinInstance")
            .unwrap();
        skin.set_field("Bones", NifValue::Array(vec![NifValue::Ref(human as i32)]));
        assert!(source_rig_options(&mut nif, &ConvertFileOptions::default()).is_none());
        assert_eq!(
            nif.blocks[1].get_field("String Data"),
            Some(&NifValue::String("WeaponBow".into()))
        );
        let mut unmarked = bow_fixture();
        unmarked.blocks[1].set_field("String Data", NifValue::String("WEAPON".into()));
        assert!(source_rig_options(&mut unmarked, &ConvertFileOptions::default()).is_none());
    }

    #[test]
    fn crossbow_uses_its_right_hand_grip_instead_of_the_bow_grip() {
        let mut nif = bow_fixture();
        nif.blocks[2].set_field("Name", NifValue::String("CrossbowRoot".into()));
        assert!(source_rig_options(&mut nif, &ConvertFileOptions::default()).is_some());
        assert_eq!(
            nif.blocks[1].get_field("String Data"),
            Some(&NifValue::String("Weapon".into()))
        );
    }
}
