use std::collections::HashMap;
use std::path::PathBuf;

use havok_native::animation::parsers::CharacterRecord;
use havok_native::asset::classxml::parse_patches;
use havok_native::asset::discovery::{FileEntry, classify_category, classify_role};
use havok_native::asset::manifest::build_manifests;

#[test]
fn classify_category_maps_known_roots_case_insensitively() {
    for (path, expected) in [
        ("UniqueBehaviors/X/foo.xml", "Weapon"),
        ("uniquebehaviors/X/foo.xml", "Weapon"),
        ("DLC01/BehaviorsUnique/FlamerFX/foo.xml", "Weapon"),
        ("GenericBehaviors/Workshop/foo.xml", "Generic"),
        ("Actors/Character/Behaviors/foo.xml", "Character"),
        ("Actors/Shared/foo.hkx", "ActorShared"),
        ("Actors/Turret/foo.hkx", "Turret"),
        ("Actors/PowerArmor/foo.hkx", "PowerArmor"),
        ("Actors/Deathclaw/Behaviors/foo.hkx", "Creature"),
        ("SetDressing/farm/foo.hkx", "SetDressing"),
        ("Effects/EffectBehaviors/Fire/foo.xml", "Effect"),
        ("Furniture/Chair/foo.xml", "Furniture"),
        ("Interface/pipboy/foo.xml", "Interface"),
        ("Architecture/Settlement/foo.nif", "Architecture"),
        ("AnimTextData/foo.txt", "AnimGraph"),
        ("SomeRandomDir/foo.xml", "Misc"),
    ] {
        assert_eq!(classify_category(path), expected, "{path}");
    }
}

#[test]
fn classify_role_uses_extension_then_directory() {
    for (path, expected) in [
        ("Actors/SomeCreature/skeleton.rig", "skeleton"),
        ("Actors/SomeCreature/Animations/idle.af", "animation"),
        ("Actors/SomeCreature/Behaviors/graph.agx", "behavior"),
        ("Actors/SomeCreature/CharacterAssets/skeleton.hkt", "skeleton"),
        ("Actors/SomeCreature/CharacterAssets/skeleton.hkx", "skeleton"),
        ("Actors/Deathclaw/Characters/deathclaw.hkx", "character"),
        ("Actors/Deathclaw/Behaviors/deathclaw.hkx", "behavior"),
        ("Actors/Deathclaw/Animations/attack.hkx", "animation"),
        ("Actors/Deathclaw/deathclaw.nif", "asset"),
        ("Actors/Deathclaw/textures/skin.dds", "asset"),
    ] {
        assert_eq!(classify_role(path, None), expected, "{path}");
    }
}

fn make_entry(rel_path: &str, role: &str) -> FileEntry {
    FileEntry {
        abs_path: PathBuf::from(rel_path),
        rel_path: rel_path.to_string(),
        role: role.to_string(),
        category: classify_category(rel_path).to_string(),
        file_type: rel_path.rsplit('.').next().unwrap_or("").to_string(),
        is_xml: rel_path.ends_with(".xml") || rel_path.ends_with(".agx"),
    }
}

#[test]
fn manifests_group_by_root_with_typed_ids_and_skip_unrooted_entries() {
    let entries = vec![
        make_entry("UniqueBehaviors/FlamerFX/Behaviors/flamer.xml", "behavior"),
        make_entry(
            "UniqueBehaviors/FlamerFX/Characters/flamer.xml",
            "character",
        ),
        make_entry(
            "UniqueBehaviors/FlamerFX/CharacterAssets/skeleton.hkx",
            "skeleton",
        ),
        make_entry("Actors/Deathclaw/Behaviors/deathclaw.xml", "behavior"),
        make_entry(
            "GenericBehaviors/Workshop/Behaviors/workshop.xml",
            "behavior",
        ),
        make_entry("AnimTextData/foo.txt", "asset"),
    ];

    let manifests = build_manifests(&entries, &HashMap::new(), "fo4");
    assert_eq!(manifests.len(), 3);
    for (id_part, manifest_type, file_count) in [
        ("FlamerFX", "weapon_fx", 3),
        ("Deathclaw", "actor", 1),
        ("Workshop", "generic_fx", 1),
    ] {
        let manifest = manifests
            .iter()
            .find(|m| m.id.contains(id_part))
            .unwrap_or_else(|| panic!("{id_part} manifest not found"));
        assert!(manifest.id.starts_with("fo4/"), "{}", manifest.id);
        assert_eq!(manifest.manifest_type, manifest_type, "{id_part}");
        assert_eq!(manifest.files.len(), file_count, "{id_part}");
        assert_eq!(manifest.file_count, file_count, "{id_part}");
    }
}

#[test]
fn manifest_skeleton_dependency_only_for_rigs_outside_the_manifest() {
    for (character, rig_name, expected_dependency) in [
        (
            "UniqueBehaviors/FlamerFX/Characters/flamerproject.hkx",
            "../../Actors/Character/CharacterAssets/skeleton.hkx",
            Some("fo4/Actors/Character"),
        ),
        (
            "Actors/Deathclaw/Characters/deathclaw.hkx",
            "CharacterAssets/skeleton.hkx",
            None,
        ),
    ] {
        let entries = vec![make_entry(character, "character")];
        let character_data = HashMap::from([(
            character.to_string(),
            CharacterRecord {
                rig_name: rig_name.to_string(),
                behavior_filename: String::new(),
                model_up: String::new(),
                model_forward: String::new(),
                model_right: String::new(),
            },
        )]);

        let manifests = build_manifests(&entries, &character_data, "fo4");
        assert_eq!(manifests.len(), 1);
        let dependencies = &manifests[0].dependencies;
        match expected_dependency {
            Some(depends_on) => {
                assert_eq!(dependencies.len(), 1, "{character}");
                assert_eq!(dependencies[0].dep_type, "skeleton");
                assert_eq!(dependencies[0].depends_on, depends_on);
            }
            None => assert!(dependencies.is_empty(), "{character}"),
        }
    }
}

#[test]
fn parse_patches_extracts_version_add_remove_and_rename_tuples() {
    let snippet = r#"
// Some class patches
HK_PATCH_BEGIN("hkObject", 1, "hkObject", 2)
HK_PATCH_END()

HK_PATCH_BEGIN(HK_NULL, HK_CLASS_ADDED, "hkNewClass", 3)
HK_PATCH_END()

HK_PATCH_BEGIN("hkOldClass", 5, HK_NULL, HK_CLASS_REMOVED)
HK_PATCH_END()

// Rename
HK_PATCH_BEGIN("hkOldName", 7, "hkNewName", 8)
HK_PATCH_END()
"#;
    let some = |name: &str| Some(name.to_string());
    assert_eq!(
        parse_patches(snippet),
        vec![
            (some("hkObject"), 1, some("hkObject"), 2),
            (None, -1, some("hkNewClass"), 3),
            (some("hkOldClass"), 5, None, -2),
            (some("hkOldName"), 7, some("hkNewName"), 8),
        ]
    );
    assert!(parse_patches("// no patches here\nsome other content").is_empty());
}
