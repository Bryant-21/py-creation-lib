use std::collections::HashMap;
use std::path::PathBuf;

use havok_native::animation::parsers::CharacterRecord;
use havok_native::asset::classxml::parse_patches;
use havok_native::asset::discovery::{FileEntry, classify_category, classify_role};
use havok_native::asset::manifest::build_manifests;

// ---------------------------------------------------------------------------
// classify_category tests
// ---------------------------------------------------------------------------

#[test]
fn classify_category_unique_behaviors_is_weapon() {
    assert_eq!(classify_category("UniqueBehaviors/X/foo.xml"), "Weapon");
}

#[test]
fn classify_category_generic_behaviors_is_generic() {
    assert_eq!(
        classify_category("GenericBehaviors/Workshop/foo.xml"),
        "Generic"
    );
}

#[test]
fn classify_category_actors_character_is_character() {
    assert_eq!(
        classify_category("Actors/Character/Behaviors/foo.xml"),
        "Character"
    );
}

#[test]
fn classify_category_actors_shared_is_actor_shared() {
    assert_eq!(classify_category("Actors/Shared/foo.hkx"), "ActorShared");
}

#[test]
fn classify_category_actors_turret_is_turret() {
    assert_eq!(classify_category("Actors/Turret/foo.hkx"), "Turret");
}

#[test]
fn classify_category_actors_power_armor_is_power_armor() {
    assert_eq!(classify_category("Actors/PowerArmor/foo.hkx"), "PowerArmor");
}

#[test]
fn classify_category_actors_other_is_creature() {
    assert_eq!(
        classify_category("Actors/Deathclaw/Behaviors/foo.hkx"),
        "Creature"
    );
}

#[test]
fn classify_category_set_dressing_is_set_dressing() {
    assert_eq!(classify_category("SetDressing/farm/foo.hkx"), "SetDressing");
}

#[test]
fn classify_category_effects_is_effect() {
    assert_eq!(
        classify_category("Effects/EffectBehaviors/Fire/foo.xml"),
        "Effect"
    );
}

#[test]
fn classify_category_furniture_is_furniture() {
    assert_eq!(classify_category("Furniture/Chair/foo.xml"), "Furniture");
}

#[test]
fn classify_category_interface_is_interface() {
    assert_eq!(classify_category("Interface/pipboy/foo.xml"), "Interface");
}

#[test]
fn classify_category_architecture_is_architecture() {
    assert_eq!(
        classify_category("Architecture/Settlement/foo.nif"),
        "Architecture"
    );
}

#[test]
fn classify_category_anim_text_data_is_anim_graph() {
    assert_eq!(classify_category("AnimTextData/foo.txt"), "AnimGraph");
}

#[test]
fn classify_category_unknown_path_is_misc() {
    assert_eq!(classify_category("SomeRandomDir/foo.xml"), "Misc");
}

#[test]
fn classify_category_behaviors_unique_dlc_is_weapon() {
    assert_eq!(
        classify_category("DLC01/BehaviorsUnique/FlamerFX/foo.xml"),
        "Weapon"
    );
}

#[test]
fn classify_category_case_insensitive_unique_behaviors() {
    assert_eq!(classify_category("uniquebehaviors/X/foo.xml"), "Weapon");
}

// ---------------------------------------------------------------------------
// classify_role tests
// ---------------------------------------------------------------------------

#[test]
fn classify_role_rig_is_skeleton() {
    assert_eq!(
        classify_role("Actors/SomeCreature/skeleton.rig", None),
        "skeleton"
    );
}

#[test]
fn classify_role_af_is_animation() {
    assert_eq!(
        classify_role("Actors/SomeCreature/Animations/idle.af", None),
        "animation"
    );
}

#[test]
fn classify_role_agx_is_behavior() {
    assert_eq!(
        classify_role("Actors/SomeCreature/Behaviors/graph.agx", None),
        "behavior"
    );
}

#[test]
fn classify_role_hkt_is_skeleton() {
    assert_eq!(
        classify_role("Actors/SomeCreature/CharacterAssets/skeleton.hkt", None),
        "skeleton"
    );
}

#[test]
fn classify_role_skeleton_in_character_assets() {
    assert_eq!(
        classify_role("Actors/SomeCreature/CharacterAssets/skeleton.hkx", None),
        "skeleton"
    );
}

#[test]
fn classify_role_characters_dir_is_character() {
    assert_eq!(
        classify_role("Actors/Deathclaw/Characters/deathclaw.hkx", None),
        "character"
    );
}

#[test]
fn classify_role_behaviors_dir_is_behavior() {
    assert_eq!(
        classify_role("Actors/Deathclaw/Behaviors/deathclaw.hkx", None),
        "behavior"
    );
}

#[test]
fn classify_role_animations_dir_is_animation() {
    assert_eq!(
        classify_role("Actors/Deathclaw/Animations/attack.hkx", None),
        "animation"
    );
}

#[test]
fn classify_role_nif_is_asset() {
    assert_eq!(
        classify_role("Actors/Deathclaw/deathclaw.nif", None),
        "asset"
    );
}

#[test]
fn classify_role_dds_is_asset() {
    assert_eq!(
        classify_role("Actors/Deathclaw/textures/skin.dds", None),
        "asset"
    );
}

// ---------------------------------------------------------------------------
// manifest grouping tests
// ---------------------------------------------------------------------------

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
fn manifest_groups_unique_behaviors_entries_together() {
    let entries = vec![
        make_entry("UniqueBehaviors/FlamerFX/Behaviors/flamer.xml", "behavior"),
        make_entry(
            "UniqueBehaviors/FlamerFX/Characters/flamer.xml",
            "character",
        ),
        make_entry("Actors/Deathclaw/Behaviors/deathclaw.xml", "behavior"),
    ];

    let manifests = build_manifests(&entries, &HashMap::new(), "fo4");

    // Should produce two manifests: FlamerFX and Deathclaw
    assert_eq!(manifests.len(), 2);

    let flamer = manifests
        .iter()
        .find(|m| m.id.contains("FlamerFX"))
        .expect("FlamerFX manifest not found");
    assert_eq!(flamer.manifest_type, "weapon_fx");
    assert_eq!(flamer.files.len(), 2);

    let deathclaw = manifests
        .iter()
        .find(|m| m.id.contains("Deathclaw"))
        .expect("Deathclaw manifest not found");
    assert_eq!(deathclaw.manifest_type, "actor");
    assert_eq!(deathclaw.files.len(), 1);
}

#[test]
fn manifest_actor_type_assigned_correctly() {
    let entries = vec![make_entry(
        "Actors/Deathclaw/Behaviors/deathclaw.xml",
        "behavior",
    )];
    let manifests = build_manifests(&entries, &HashMap::new(), "fo4");
    assert_eq!(manifests.len(), 1);
    assert_eq!(manifests[0].manifest_type, "actor");
}

#[test]
fn manifest_generic_behaviors_type() {
    let entries = vec![make_entry(
        "GenericBehaviors/Workshop/Behaviors/workshop.xml",
        "behavior",
    )];
    let manifests = build_manifests(&entries, &HashMap::new(), "fo4");
    assert_eq!(manifests.len(), 1);
    assert_eq!(manifests[0].manifest_type, "generic_fx");
}

#[test]
fn manifest_id_includes_source() {
    let entries = vec![make_entry(
        "Actors/Deathclaw/Behaviors/deathclaw.xml",
        "behavior",
    )];
    let manifests = build_manifests(&entries, &HashMap::new(), "fo4");
    assert!(manifests[0].id.starts_with("fo4/"));
}

#[test]
fn manifest_file_count_matches() {
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
    ];
    let manifests = build_manifests(&entries, &HashMap::new(), "fo4");
    assert_eq!(manifests.len(), 1);
    assert_eq!(manifests[0].file_count, 3);
}

#[test]
fn manifest_entries_not_matching_known_root_are_ignored() {
    // AnimTextData paths don't match any known manifest root pattern
    let entries = vec![make_entry("AnimTextData/foo.txt", "asset")];
    let manifests = build_manifests(&entries, &HashMap::new(), "fo4");
    // No manifest root inferred for AnimTextData
    assert_eq!(manifests.len(), 0);
}

// ---------------------------------------------------------------------------
// Skeleton dependency tests
// ---------------------------------------------------------------------------

#[test]
fn manifest_skeleton_dep_emitted_for_cross_manifest_rig() {
    // A FlamerFX character file whose rig_name points outside the manifest via ../
    let entries = vec![make_entry(
        "UniqueBehaviors/FlamerFX/Characters/flamerproject.hkx",
        "character",
    )];
    let char_rel = "UniqueBehaviors/FlamerFX/Characters/flamerproject.hkx".to_string();
    let mut character_data: HashMap<String, CharacterRecord> = HashMap::new();
    character_data.insert(
        char_rel,
        CharacterRecord {
            rig_name: "../../Actors/Character/CharacterAssets/skeleton.hkx".to_string(),
            behavior_filename: String::new(),
            model_up: String::new(),
            model_forward: String::new(),
            model_right: String::new(),
        },
    );

    let manifests = build_manifests(&entries, &character_data, "fo4");
    assert_eq!(manifests.len(), 1);

    let m = &manifests[0];
    assert_eq!(m.dependencies.len(), 1);
    let dep = &m.dependencies[0];
    assert_eq!(dep.dep_type, "skeleton");
    assert_eq!(dep.depends_on, "fo4/Actors/Character");
}

#[test]
fn manifest_no_skeleton_dep_when_rig_local() {
    // rig_name without ../ should not produce a dependency
    let entries = vec![make_entry(
        "Actors/Deathclaw/Characters/deathclaw.hkx",
        "character",
    )];
    let char_rel = "Actors/Deathclaw/Characters/deathclaw.hkx".to_string();
    let mut character_data: HashMap<String, CharacterRecord> = HashMap::new();
    character_data.insert(
        char_rel,
        CharacterRecord {
            rig_name: "CharacterAssets/skeleton.hkx".to_string(),
            behavior_filename: String::new(),
            model_up: String::new(),
            model_forward: String::new(),
            model_right: String::new(),
        },
    );

    let manifests = build_manifests(&entries, &character_data, "fo4");
    assert_eq!(manifests.len(), 1);
    assert_eq!(manifests[0].dependencies.len(), 0);
}

// ---------------------------------------------------------------------------
// parse_patches tests
// ---------------------------------------------------------------------------

const PATCH_HXX_SNIPPET: &str = r#"
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

#[test]
fn parse_patches_returns_correct_tuples_for_snippet() {
    let patches = parse_patches(PATCH_HXX_SNIPPET);

    assert_eq!(patches.len(), 4);

    // Version change: ("hkObject", 1) -> ("hkObject", 2)
    assert_eq!(
        patches[0],
        (
            Some("hkObject".to_string()),
            1,
            Some("hkObject".to_string()),
            2
        )
    );

    // Class added: None -> ("hkNewClass", 3)
    assert_eq!(patches[1], (None, -1, Some("hkNewClass".to_string()), 3));

    // Class removed: ("hkOldClass", 5) -> None
    assert_eq!(patches[2], (Some("hkOldClass".to_string()), 5, None, -2));

    // Rename: ("hkOldName", 7) -> ("hkNewName", 8)
    assert_eq!(
        patches[3],
        (
            Some("hkOldName".to_string()),
            7,
            Some("hkNewName".to_string()),
            8
        )
    );
}

#[test]
fn parse_patches_empty_content_returns_empty() {
    let patches = parse_patches("// no patches here\nsome other content");
    assert_eq!(patches.len(), 0);
}

#[test]
fn parse_patches_single_version_bump() {
    let content = r#"HK_PATCH_BEGIN("hkaSkeleton", 3, "hkaSkeleton", 4)"#;
    let patches = parse_patches(content);
    assert_eq!(patches.len(), 1);
    assert_eq!(
        patches[0],
        (
            Some("hkaSkeleton".to_string()),
            3,
            Some("hkaSkeleton".to_string()),
            4
        )
    );
}
