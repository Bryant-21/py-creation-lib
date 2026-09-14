use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use nif_core_native::convert_file::{ConvertFileOptions, ConvertFileReport, convert_nif_file};
use nif_core_native::model::{NifBlock, NifFile, NifValue};
use nif_core_native::schema::NifSchema;
use nif_core_native::skin::LegacySkinPolicy;
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
struct MeleeReceipt {
    source_game: String,
    #[serde(default)]
    source_family: String,
    source_form_key: String,
    editor_id: String,
    status: String,
    #[serde(default)]
    policy: String,
    #[serde(alias = "role")]
    weapon_role: String,
    anim_type: String,
    #[serde(default)]
    world_model: String,
    #[serde(default)]
    first_person_model: String,
    #[serde(default)]
    first_person_resolution: String,
    #[serde(default, alias = "reason_code")]
    rejection_reason: String,
}

#[derive(Debug, Deserialize)]
struct MeleeCorpusManifest {
    schema_version: u32,
    #[serde(default)]
    authoritative: bool,
    #[serde(default)]
    expected_total_receipts: usize,
    #[serde(default)]
    expected_unique_model_claims: usize,
    #[serde(default)]
    #[serde(alias = "expected_unresolved_models")]
    expected_record_only_downgrades: usize,
    #[serde(default)]
    expected_admitted_by_game: BTreeMap<String, usize>,
    #[serde(default)]
    expected_rejected_by_reason: BTreeMap<String, usize>,
    receipts: Vec<MeleeReceipt>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ModelClaim {
    source_game: String,
    model_path: String,
    owners: BTreeSet<String>,
    record_only_downgrade: Option<String>,
}

#[derive(Clone, Debug, Default)]
struct OutcomeCounts {
    converted: usize,
    unsupported: usize,
    failed: usize,
    unresolved: usize,
    record_only: usize,
}

#[derive(Debug, Default)]
struct ReceiptAccounting {
    total: usize,
    admitted_by_game: BTreeMap<String, usize>,
    admitted_model_less: usize,
    rejected_by_reason: BTreeMap<String, usize>,
    ranged_or_throwing_rejected: usize,
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn temp_dir(name: &str) -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "nif_core_native_{name}_{}_{}",
        std::process::id(),
        suffix
    ))
}

fn receipt(
    source_game: &str,
    source_form_key: &str,
    editor_id: &str,
    anim_type: &str,
    world_model: &str,
    first_person_model: &str,
) -> MeleeReceipt {
    MeleeReceipt {
        source_game: source_game.to_string(),
        source_form_key: source_form_key.to_string(),
        editor_id: editor_id.to_string(),
        status: "admitted".to_string(),
        policy: "bulk_melee_v1".to_string(),
        weapon_role: "melee".to_string(),
        anim_type: anim_type.to_string(),
        world_model: world_model.to_string(),
        first_person_model: first_person_model.to_string(),
        source_family: source_game.to_string(),
        first_person_resolution: "direct".to_string(),
        rejection_reason: String::new(),
    }
}

fn representative_manifest() -> MeleeCorpusManifest {
    MeleeCorpusManifest {
        schema_version: 1,
        authoritative: false,
        expected_total_receipts: 0,
        expected_unique_model_claims: 0,
        expected_record_only_downgrades: 0,
        expected_admitted_by_game: BTreeMap::new(),
        expected_rejected_by_reason: BTreeMap::new(),
        receipts: vec![
            receipt(
                "skyrimse",
                "013984@Skyrim.esm",
                "SteelBattleaxe",
                "TwoHandAxe",
                "weapons/steel/steelbattleaxe.nif",
                "weapons/steel/1stpersonsteelbattleaxe.nif",
            ),
            receipt(
                "skyrimse",
                "058F5E@Skyrim.esm",
                "BoundWeaponBattleaxe",
                "TwoHandAxe",
                "weapons/boundweapons/boundaxeholder.nif",
                "weapons/boundweapons/boundaxeholder.nif",
            ),
            receipt(
                "skyrimse",
                "04E4EE@Skyrim.esm",
                "DA09Dawnbreaker",
                "OneHandSword",
                "weapons/dawnbreaker/dawnbreaker.nif",
                "weapons/dawnbreaker/1stpersondawnbreaker.nif",
            ),
            receipt(
                "fnv",
                "11A8E4@FalloutNV.esm",
                "WeapNVHatchet",
                "1",
                "weapons/1handmelee/hatchet.nif",
                "weapons/1handmelee/1stpersonhatchet.nif",
            ),
            receipt(
                "fnv",
                "00043E1E@FalloutNV.esm",
                "WeapRipper",
                "1",
                "weapons/1handmelee/ripper.nif",
                "weapons/1handmelee/ripper.nif",
            ),
            receipt(
                "fnv",
                "000CD50E@FalloutNV.esm",
                "WeapNVChainsaw",
                "2",
                "weapons/2handmelee/chainsaw.nif",
                "weapons/2handmelee/chainsaw.nif",
            ),
            receipt(
                "fo3",
                "00043E1E@Fallout3.esm",
                "WeapRipper",
                "1",
                "weapons/1handmelee/ripper.nif",
                "weapons/1handmelee/ripper.nif",
            ),
            receipt(
                "fo3",
                "0000434E@Fallout3.esm",
                "WeapShishkebab",
                "1",
                "weapons/1handmelee/shishkebab.nif",
                "weapons/1handmelee/shishkebab.nif",
            ),
        ],
    }
}

fn load_authoritative_manifest() -> Option<MeleeCorpusManifest> {
    let Some(path) = std::env::var_os("MELEE_NIF_CORPUS_MANIFEST") else {
        return None;
    };
    let path = PathBuf::from(path);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read melee corpus manifest {}: {error}", path.display()));
    Some(
        serde_json::from_str(&text).unwrap_or_else(|error| {
            panic!("parse melee corpus manifest {}: {error}", path.display())
        }),
    )
}

fn canonical_model_path(path: &str) -> String {
    let normalized = path
        .trim()
        .replace('\\', "/")
        .trim_start_matches('/')
        .to_ascii_lowercase();
    normalized
        .strip_prefix("meshes/")
        .unwrap_or(&normalized)
        .to_string()
}

fn canonical_source_game(source_game: &str, source_family: &str) -> String {
    let family = source_family.trim().to_ascii_lowercase();
    let game = if family.is_empty() {
        source_game.trim().to_ascii_lowercase()
    } else {
        family
    };
    match game.as_str() {
        "skyrim" => "skyrimse".to_string(),
        _ => game,
    }
}

fn admitted_animation(source_game: &str, anim_type: &str) -> bool {
    let normalized = anim_type.trim().to_ascii_lowercase();
    match source_game {
        "skyrim" | "skyrimse" => {
            matches!(
                normalized.as_str(),
                "0" | "1"
                    | "2"
                    | "3"
                    | "4"
                    | "5"
                    | "6"
                    | "handtohandmelee"
                    | "onehandsword"
                    | "onehanddagger"
                    | "onehandaxe"
                    | "onehandmace"
                    | "twohandsword"
                    | "twohandaxe"
            )
        }
        "fnv" | "fo3" => matches!(normalized.as_str(), "0" | "1" | "2"),
        _ => false,
    }
}

fn record_only_downgrade(receipt: &MeleeReceipt, source_game: &str, model: &str) -> Option<String> {
    let is_benthic_lurker_world_model = source_game == "skyrimse"
        && receipt
            .source_form_key
            .eq_ignore_ascii_case("0001E112@Dragonborn.esm")
        && receipt
            .editor_id
            .eq_ignore_ascii_case("DLC2CrBenthicLurkerWeapon")
        && canonical_model_path(&receipt.world_model) == model
        && model == "actors/dlc02/giant_fishman/characterassets/dlc2giantfishmanweapon.nif"
        && receipt
            .first_person_resolution
            .eq_ignore_ascii_case("wnam_stat")
        && canonical_model_path(&receipt.first_person_model) == "weapons/giant/giantclub.nif";
    is_benthic_lurker_world_model.then(|| {
        "Dragonborn does not ship or index the admitted direct world NIF; retain the WEAP and its resolved WNAM GiantClub model, but the distinct Benthic Lurker world/inventory/drop appearance is unavailable"
            .to_string()
    })
}

fn manifest_claims(manifest: &MeleeCorpusManifest) -> (Vec<ModelClaim>, ReceiptAccounting) {
    assert_eq!(manifest.schema_version, 1);
    let mut claims = BTreeMap::<(String, String), (BTreeSet<String>, Vec<Option<String>>)>::new();
    let mut admitted_games = BTreeSet::new();
    let mut accounting = ReceiptAccounting::default();
    for receipt in &manifest.receipts {
        accounting.total += 1;
        let source_game = canonical_source_game(&receipt.source_game, &receipt.source_family);
        let status = receipt.status.trim().to_ascii_lowercase();
        if !receipt.policy.trim().is_empty() {
            assert_eq!(receipt.policy.trim(), "bulk_melee_v1");
        }
        if status == "rejected" {
            let reason = receipt.rejection_reason.trim().to_ascii_lowercase();
            assert!(
                !reason.is_empty(),
                "rejected receipt lacks a typed reason: {} {}",
                receipt.source_form_key,
                receipt.editor_id
            );
            *accounting
                .rejected_by_reason
                .entry(reason.clone())
                .or_default() += 1;
            if reason.contains("ranged") || reason.contains("throwing") {
                accounting.ranged_or_throwing_rejected += 1;
            }
            continue;
        }
        assert_eq!(status, "admitted", "unknown receipt disposition {status}");
        assert_eq!(receipt.weapon_role.trim().to_ascii_lowercase(), "melee");
        assert!(
            admitted_animation(&source_game, &receipt.anim_type),
            "ranged/unknown animation admitted by manifest: {} {} {}",
            receipt.source_form_key,
            receipt.editor_id,
            receipt.anim_type
        );
        admitted_games.insert(source_game.clone());
        *accounting
            .admitted_by_game
            .entry(source_game.clone())
            .or_default() += 1;
        if receipt.world_model.trim().is_empty() && receipt.first_person_model.trim().is_empty() {
            accounting.admitted_model_less += 1;
        }
        if !receipt.first_person_model.trim().is_empty() {
            assert!(
                !receipt.first_person_resolution.trim().is_empty(),
                "first-person model lacks a typed resolution: {} {}",
                receipt.source_form_key,
                receipt.editor_id
            );
        }
        for path in [&receipt.world_model, &receipt.first_person_model] {
            if path.trim().is_empty() {
                continue;
            }
            let canonical = canonical_model_path(path);
            assert!(canonical.ends_with(".nif"), "non-NIF model path {path}");
            let entry = claims.entry((source_game.clone(), canonical)).or_default();
            entry
                .0
                .insert(format!("{}:{}", receipt.source_form_key, receipt.editor_id));
            entry.1.push(record_only_downgrade(
                receipt,
                &source_game,
                &canonical_model_path(path),
            ));
        }
    }
    assert_eq!(
        admitted_games,
        BTreeSet::from(["fnv".into(), "fo3".into(), "skyrimse".into()])
    );
    if manifest.authoritative {
        assert_eq!(accounting.total, manifest.expected_total_receipts);
        let expected = manifest
            .expected_admitted_by_game
            .iter()
            .map(|(game, count)| (canonical_source_game(game, ""), *count))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(accounting.admitted_by_game, expected);
        assert_eq!(
            accounting.rejected_by_reason,
            manifest.expected_rejected_by_reason
        );
        assert_eq!(accounting.admitted_by_game.get("skyrimse"), Some(&2_792));
        assert!(
            accounting.admitted_model_less > 0,
            "authoritative receipts must type admitted model-less weapons"
        );
        assert!(
            accounting.ranged_or_throwing_rejected > 0,
            "authoritative receipts must retain typed ranged/throwing rejections"
        );
    }
    let model_paths = claims
        .keys()
        .map(|(_, path)| path.as_str())
        .collect::<Vec<_>>();
    assert!(model_paths.iter().any(|path| path.contains("boundweapons")));
    assert!(model_paths.iter().any(|path| path.contains("dawnbreaker")));
    assert!(
        model_paths
            .iter()
            .any(|path| path.contains("ripper") || path.contains("chainsaw"))
    );
    let claims = claims
        .into_iter()
        .map(
            |((source_game, model_path), (owners, downgrade_reasons))| ModelClaim {
                source_game,
                model_path,
                owners,
                record_only_downgrade: downgrade_reasons.iter().all(Option::is_some).then(|| {
                    downgrade_reasons
                        .into_iter()
                        .flatten()
                        .collect::<BTreeSet<_>>()
                        .into_iter()
                        .collect::<Vec<_>>()
                        .join("; ")
                }),
            },
        )
        .collect();
    (claims, accounting)
}

fn extra_data_ids(block: &NifBlock) -> Vec<usize> {
    match block.get_field("Extra Data List") {
        Some(NifValue::Array(values)) => values
            .iter()
            .filter_map(|value| match value {
                NifValue::Ref(id) if *id >= 0 => Some(*id as usize),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn reachable_blocks(nif: &NifFile) -> HashSet<usize> {
    let schema = NifSchema::from_generated();
    let mut reachable = HashSet::new();
    let mut pending = VecDeque::from([0usize]);
    while let Some(block_id) = pending.pop_front() {
        if !reachable.insert(block_id) {
            continue;
        }
        let Some(block) = nif.get_block(block_id) else {
            continue;
        };
        for (_, refs) in block.get_all_ref_fields(&schema) {
            for reference in refs {
                if reference >= 0 {
                    pending.push_back(reference as usize);
                }
            }
        }
    }
    reachable
}

fn collision_class(nif: &NifFile) -> &'static str {
    if nif
        .blocks
        .iter()
        .any(|block| block.type_name == "bhkBlendCollisionObject")
    {
        "blend"
    } else if nif
        .blocks
        .iter()
        .any(|block| block.type_name == "bhkRigidBodyT")
    {
        "rigid_body_t"
    } else if nif
        .blocks
        .iter()
        .any(|block| block.type_name == "bhkRigidBody")
    {
        "rigid_body"
    } else if nif
        .blocks
        .iter()
        .any(|block| block.type_name.starts_with("bhk"))
    {
        "legacy_other"
    } else {
        "none"
    }
}

fn controller_class(nif: &NifFile) -> &'static str {
    let names = nif
        .blocks
        .iter()
        .map(|block| block.type_name.as_str())
        .collect::<Vec<_>>();
    if names
        .iter()
        .any(|name| name.contains("PSys") && name.contains("Ctlr"))
    {
        "particle"
    } else if names
        .iter()
        .any(|name| *name == "NiControllerManager" || *name == "NiControllerSequence")
    {
        "sequence"
    } else if names
        .iter()
        .any(|name| name.contains("ShaderProperty") && name.contains("Controller"))
    {
        "shader"
    } else if names
        .iter()
        .any(|name| name.contains("Controller") || name.contains("Interpolator"))
    {
        "generic"
    } else {
        "none"
    }
}

fn validate_references(nif: &NifFile) -> Result<(), String> {
    let schema = NifSchema::from_generated();
    for block in &nif.blocks {
        for (field, refs) in block.get_all_ref_fields(&schema) {
            for reference in refs {
                if reference >= 0 && reference as usize >= nif.blocks.len() {
                    return Err(format!(
                        "block {} {} field {field} has invalid reference {reference}",
                        block.block_id, block.type_name
                    ));
                }
            }
        }
    }
    Ok(())
}

fn validate_root_marker(nif: &NifFile) -> Result<(), String> {
    let root_extra_ids = extra_data_ids(&nif.blocks[0]);
    let root_markers = root_extra_ids
        .iter()
        .copied()
        .filter(|id| {
            nif.get_block(*id).is_some_and(|block| {
                block.type_name == "NiStringExtraData"
                    && matches!(block.get_field("Name"), Some(NifValue::String(name)) if name == "Prn")
                    && matches!(block.get_field("String Data"), Some(NifValue::String(value)) if value == "WEAPON")
            })
        })
        .collect::<Vec<_>>();
    if root_markers.len() != 1 {
        return Err(format!(
            "root has {} Prn=WEAPON markers",
            root_markers.len()
        ));
    }
    for id in &root_extra_ids {
        let Some(block) = nif.get_block(*id) else {
            continue;
        };
        if block.type_name == "NiStringExtraData"
            && (matches!(block.get_field("Name"), Some(NifValue::String(name)) if name == "WEAPON")
                || matches!(block.get_field("String Data"), Some(NifValue::String(value)) if value == "WeaponBack"))
        {
            return Err(format!("root retains conflicting marker block {id}"));
        }
    }
    let marker_id = root_markers[0];
    let parents = nif
        .blocks
        .iter()
        .filter(|block| extra_data_ids(block).contains(&marker_id))
        .map(|block| block.block_id)
        .collect::<Vec<_>>();
    if parents != [0] {
        return Err(format!("root marker has non-exclusive parents {parents:?}"));
    }
    Ok(())
}

fn validate_materials_and_textures(
    nif: &NifFile,
    report: &ConvertFileReport,
    output_data: &Path,
    source_root: &Path,
    asset_prefix: &str,
) -> Result<(), String> {
    for emitted in &report.emitted_bgsms {
        if !Path::new(emitted).is_file() {
            return Err(format!("emitted material is missing: {emitted}"));
        }
    }
    for emitted in &report.emitted_textures {
        let path = Path::new(emitted);
        if !path.is_file() {
            return Err(format!("emitted texture is missing: {emitted}"));
        }
        if path.strip_prefix(output_data).is_err()
            || !path
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("dds"))
        {
            return Err(format!(
                "emitted texture escaped output Data or is not DDS: {emitted}"
            ));
        }
    }
    for shader in nif.blocks.iter().filter(|block| {
        matches!(
            block.type_name.as_str(),
            "BSLightingShaderProperty" | "BSEffectShaderProperty"
        )
    }) {
        if let Some(NifValue::String(material)) = shader.get_field("Name")
            && !material.trim().is_empty()
        {
            let normalized = material.replace('\\', "/");
            if normalized.contains("..")
                || !normalized.to_ascii_lowercase().starts_with("materials/")
            {
                return Err(format!("invalid material path {material}"));
            }
            let resolved = output_data.join(normalized.replace('/', std::path::MAIN_SEPARATOR_STR));
            if !resolved.is_file() {
                return Err(format!("material path does not resolve: {material}"));
            }
        }
    }
    for texture_set in nif
        .blocks
        .iter()
        .filter(|block| block.type_name == "BSShaderTextureSet")
    {
        let Some(NifValue::Array(textures)) = texture_set.get_field("Textures") else {
            continue;
        };
        for texture in textures {
            let NifValue::String(path) = texture else {
                continue;
            };
            let normalized = path.trim_matches('\0').replace('\\', "/");
            if normalized.is_empty() {
                continue;
            }
            if normalized.contains("..")
                || !normalized.to_ascii_lowercase().starts_with("textures/")
            {
                return Err(format!("invalid texture path {path}"));
            }
            let relative = normalized["textures/".len()..].to_string();
            let prefix = format!("{}/", asset_prefix.to_ascii_lowercase());
            let relative = if relative.to_ascii_lowercase().starts_with(&prefix) {
                relative[prefix.len()..].to_string()
            } else {
                relative
            };
            let target = output_data
                .join("textures")
                .join(normalized["textures/".len()..].replace('/', std::path::MAIN_SEPARATOR_STR));
            if target.is_file() {
                continue;
            }
            let resolved = source_root
                .join("textures")
                .join(relative.replace('/', std::path::MAIN_SEPARATOR_STR));
            if !resolved.is_file() {
                return Err(format!(
                    "texture path resolves to neither target nor source: {path}"
                ));
            }
        }
    }
    Ok(())
}

fn validate_converted(
    nif: &NifFile,
    report: &ConvertFileReport,
    output_data: &Path,
    source_root: &Path,
    asset_prefix: &str,
) -> Result<(), String> {
    if nif.header.version != (20, 2, 0, 7)
        || nif.header.user_version != 12
        || nif.header.bs_version != 130
    {
        return Err(format!(
            "wrong FO4 header {:?}/{}/{}",
            nif.header.version, nif.header.user_version, nif.header.bs_version
        ));
    }
    let schema = NifSchema::from_generated();
    if !schema.is_subtype_of(&nif.blocks[0].type_name, "NiNode") {
        return Err(format!("invalid root type {}", nif.blocks[0].type_name));
    }
    validate_root_marker(nif)?;
    validate_references(nif)?;
    let legacy = nif.blocks.iter().find(|block| {
        matches!(
            block.type_name.as_str(),
            "NiTriShape"
                | "NiTriShapeData"
                | "NiTriStrips"
                | "NiTriStripsData"
                | "BSShaderPPLightingProperty"
                | "NiMaterialProperty"
                | "BSDecalPlacementVectorExtraData"
        ) || (block.type_name.starts_with("bhk")
            && !matches!(
                block.type_name.as_str(),
                "bhkNPCollisionObject" | "bhkPhysicsSystem"
            ))
    });
    if let Some(block) = legacy {
        return Err(format!(
            "legacy block remains: {} {}",
            block.block_id, block.type_name
        ));
    }
    if let Some(block) = nif.blocks.iter().find(|block| {
        matches!(block.get_field("Name"), Some(NifValue::String(name)) if name.starts_with("DecalPlacementVector"))
    }) {
        return Err(format!("legacy decal-vector node remains: {}", block.block_id));
    }
    let reachable = reachable_blocks(nif);
    for block in &nif.blocks {
        let controller_related = schema.is_subtype_of(&block.type_name, "NiTimeController")
            || schema.is_subtype_of(&block.type_name, "NiInterpolator")
            || matches!(
                block.type_name.as_str(),
                "NiControllerManager" | "NiControllerSequence"
            );
        if controller_related && !reachable.contains(&block.block_id) {
            return Err(format!(
                "controller graph block is unreachable: {} {}",
                block.block_id, block.type_name
            ));
        }
    }
    let has_collision = nif
        .blocks
        .iter()
        .any(|block| block.type_name == "bhkNPCollisionObject");
    let has_physics = nif
        .blocks
        .iter()
        .any(|block| block.type_name == "bhkPhysicsSystem");
    if has_collision != has_physics {
        return Err(format!(
            "collision/physics mismatch {has_collision}/{has_physics}"
        ));
    }
    let root_bsx = extra_data_ids(&nif.blocks[0])
        .into_iter()
        .filter_map(|id| nif.get_block(id))
        .filter(|block| block.type_name == "BSXFlags")
        .filter_map(|block| block.get_field("Integer Data"))
        .map(NifValue::as_i64)
        .collect::<Vec<_>>();
    if has_collision && !root_bsx.iter().any(|flags| flags & 2 != 0) {
        return Err(format!(
            "live collision lacks root BSX Havok flag: {root_bsx:?}"
        ));
    }
    if !has_collision && root_bsx.iter().any(|flags| flags & 2 != 0) {
        return Err(format!(
            "root BSX Havok flag remains without collision: {root_bsx:?}"
        ));
    }
    validate_materials_and_textures(nif, report, output_data, source_root, asset_prefix)
}

fn source_path(claim: &ModelClaim) -> PathBuf {
    repo_root()
        .join("extracted")
        .join(&claim.source_game)
        .join("meshes")
        .join(claim.model_path.replace('/', std::path::MAIN_SEPARATOR_STR))
}

fn conversion_options(source_game: &str) -> ConvertFileOptions {
    let prefix = match source_game {
        "skyrim" | "skyrimse" => "Skyrim",
        "fnv" => "FNV",
        "fo3" => "FO3",
        other => panic!("unsupported corpus source game {other}"),
    };
    ConvertFileOptions {
        asset_prefix: Some(prefix.to_string()),
        weapon_role: Some("melee".to_string()),
        skin_policy: LegacySkinPolicy::PreserveSourceRig,
        ..ConvertFileOptions::default()
    }
}

fn run_claim(claim: &ModelClaim, root: &Path) -> Result<(), (bool, String)> {
    let source = source_path(claim);
    if !source.is_file() {
        return Err((
            false,
            format!("unresolved source model {}", source.display()),
        ));
    }
    let source_nif = NifFile::load(&source)
        .map_err(|error| (false, format!("load source {}: {error}", source.display())))?;
    let key = format!(
        "{}/{}/{}",
        claim.source_game,
        collision_class(&source_nif),
        controller_class(&source_nif)
    );
    let prefix = conversion_options(&claim.source_game)
        .asset_prefix
        .expect("asset prefix");
    let mut outputs = Vec::new();
    for pass in 0..2 {
        let output_data = root.join(format!("pass_{pass}")).join("Data");
        let destination = output_data
            .join("Meshes")
            .join(&claim.model_path.replace('/', std::path::MAIN_SEPARATOR_STR));
        let material_dir = output_data.join("Materials").join(&prefix);
        let report = convert_nif_file(
            &source,
            &destination,
            &claim.source_game,
            "fo4",
            Some(&material_dir),
            &conversion_options(&claim.source_game),
        )
        .map_err(|error| (false, format!("{key}: converter error: {error}")))?;
        if !report.supported {
            return Err((true, format!("{key}: unsupported: {:?}", report.errors)));
        }
        if !report.errors.is_empty() {
            return Err((false, format!("{key}: report errors: {:?}", report.errors)));
        }
        let converted = NifFile::load(&destination)
            .map_err(|error| (false, format!("{key}: load output: {error}")))?;
        let source_root = repo_root().join("extracted").join(&claim.source_game);
        validate_converted(&converted, &report, &output_data, &source_root, &prefix)
            .map_err(|error| (false, format!("{key}: {error}")))?;
        let nif_bytes = std::fs::read(&destination)
            .map_err(|error| (false, format!("{key}: read output: {error}")))?;
        let mut materials = report
            .emitted_bgsms
            .iter()
            .map(|path| {
                let path = Path::new(path);
                let relative = path.strip_prefix(&output_data).map_err(|_| {
                    (
                        false,
                        format!(
                            "{key}: emitted material escaped output Data: {}",
                            path.display()
                        ),
                    )
                })?;
                let bytes = std::fs::read(path).map_err(|error| {
                    (
                        false,
                        format!("{key}: read material {}: {error}", path.display()),
                    )
                })?;
                Ok((relative.to_string_lossy().replace('\\', "/"), bytes))
            })
            .collect::<Result<Vec<_>, (bool, String)>>()?;
        materials.sort_by(|left, right| left.0.cmp(&right.0));
        let mut emitted_textures = converted
            .blocks
            .iter()
            .filter(|block| block.type_name == "BSShaderTextureSet")
            .flat_map(|block| match block.get_field("Textures") {
                Some(NifValue::Array(textures)) => textures.as_slice(),
                _ => &[],
            })
            .filter_map(|texture| match texture {
                NifValue::String(path) if !path.trim_end_matches('\0').trim().is_empty() => {
                    Some(path.trim_end_matches('\0').replace('\\', "/"))
                }
                _ => None,
            })
            .filter_map(|path| {
                let target = output_data.join(path.replace('/', std::path::MAIN_SEPARATOR_STR));
                target.is_file().then_some((path, target))
            })
            .map(|(path, target)| {
                std::fs::read(&target)
                    .map(|bytes| (path, bytes))
                    .map_err(|error| {
                        (
                            false,
                            format!("{key}: read emitted texture {}: {error}", target.display()),
                        )
                    })
            })
            .collect::<Result<Vec<_>, (bool, String)>>()?;
        emitted_textures.sort_by(|left, right| left.0.cmp(&right.0));
        emitted_textures.dedup_by(|left, right| left.0 == right.0);
        outputs.push((nif_bytes, materials, emitted_textures));
    }
    if outputs[0] != outputs[1] {
        return Err((false, format!("{key}: output is not byte deterministic")));
    }
    Ok(())
}

fn run_manifest_gate(manifest: &MeleeCorpusManifest) {
    let (claims, receipt_accounting) = manifest_claims(manifest);
    assert!(!claims.is_empty());
    if manifest.authoritative {
        assert_eq!(claims.len(), manifest.expected_unique_model_claims);
    }
    let root = temp_dir("melee_corpus");
    std::fs::create_dir_all(&root).expect("create melee corpus temp dir");
    let next_claim = AtomicUsize::new(0);
    let claim_results = Mutex::new(Vec::with_capacity(claims.len()));
    let worker_count = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
        .min(8)
        .min(claims.len());
    std::thread::scope(|scope| {
        for _ in 0..worker_count {
            scope.spawn(|| {
                loop {
                    let index = next_claim.fetch_add(1, Ordering::Relaxed);
                    let Some(claim) = claims.get(index) else {
                        break;
                    };
                    let source = source_path(claim);
                    let class = match NifFile::load(&source) {
                        Ok(nif) => format!(
                            "{}/{}/{}",
                            claim.source_game,
                            collision_class(&nif),
                            controller_class(&nif)
                        ),
                        Err(_) if claim.record_only_downgrade.is_some() => {
                            format!("{}/record_only_missing_world_model/none", claim.source_game)
                        }
                        Err(_) => format!("{}/unresolved/unresolved", claim.source_game),
                    };
                    let result = source
                        .is_file()
                        .then(|| run_claim(claim, &root.join(index.to_string())));
                    claim_results
                        .lock()
                        .expect("claim result lock")
                        .push((index, class, result));
                }
            });
        }
    });
    let mut claim_results = claim_results.into_inner().expect("claim results");
    claim_results.sort_by_key(|(index, _, _)| *index);
    let mut counts = BTreeMap::<String, OutcomeCounts>::new();
    let mut failures = Vec::new();
    for (index, class, result) in claim_results {
        let claim = &claims[index];
        let outcome = counts.entry(class).or_default();
        let Some(result) = result else {
            if let Some(reason) = &claim.record_only_downgrade {
                outcome.record_only += 1;
                eprintln!(
                    "typed record-only downgrade {} owners={:?}: {reason}",
                    claim.model_path, claim.owners
                );
            } else {
                outcome.unresolved += 1;
                failures.push(format!(
                    "{} owners={:?}: source model is missing without a typed terminal disposition",
                    claim.model_path, claim.owners
                ));
            }
            continue;
        };
        match result {
            Ok(()) => outcome.converted += 1,
            Err((true, error)) => {
                outcome.unsupported += 1;
                failures.push(format!(
                    "{} owners={:?}: {error}",
                    claim.model_path, claim.owners
                ));
            }
            Err((false, error)) => {
                outcome.failed += 1;
                failures.push(format!(
                    "{} owners={:?}: {error}",
                    claim.model_path, claim.owners
                ));
            }
        }
    }
    eprintln!("bulk_melee_v1 receipt accounting: {receipt_accounting:#?}");
    eprintln!("melee corpus outcome counts by source/collision/controller: {counts:#?}");
    if manifest.authoritative {
        let unresolved = counts
            .values()
            .map(|counts| counts.unresolved)
            .sum::<usize>();
        let record_only = counts
            .values()
            .map(|counts| counts.record_only)
            .sum::<usize>();
        assert_eq!(unresolved, 0);
        assert_eq!(
            record_only, manifest.expected_record_only_downgrades,
            "authoritative record-only downgrade count drifted"
        );
    }
    let _ = std::fs::remove_dir_all(&root);
    assert!(
        failures.is_empty(),
        "melee corpus failures ({} of {}):\n{}",
        failures.len(),
        claims.len(),
        failures.join("\n")
    );
}

#[test]
fn representative_melee_nif_corpus_smoke() {
    run_manifest_gate(&representative_manifest());
}

#[test]
fn exact_dead_money_knife_and_lonesome_road_bowie_texture_closure() {
    let heated = ModelClaim {
        source_game: "fnv".to_string(),
        model_path: "nvdlc01/weapons/1handmelee/nvdlc01spaceageknifeheated.nif".to_string(),
        owners: BTreeSet::from([
            "00010589@DeadMoney.esm:NVDLC01WeapSpaceAgeKnifeHeated".to_string()
        ]),
        record_only_downgrade: None,
    };
    let bowie = ModelClaim {
        source_game: "fnv".to_string(),
        model_path: "nvdlc04/weapons/1handmelee/bowieknife/nvdlc04bowieknife.nif".to_string(),
        owners: BTreeSet::from([
            "000046C7@LonesomeRoad.esm:NVDLC04WeapBowieKnife".to_string(),
            "0000A606@LonesomeRoad.esm:NVDLC04WeapBowieKnifeUnique".to_string(),
        ]),
        record_only_downgrade: None,
    };
    if !source_path(&heated).is_file() || !source_path(&bowie).is_file() {
        eprintln!("exact FNV DLC melee source corpus is unavailable; regression skipped");
        return;
    }

    let root = temp_dir("fnv_dlc_melee_texture_closure");
    run_claim(&heated, &root.join("heated")).unwrap_or_else(|(_, error)| panic!("{error}"));
    run_claim(&bowie, &root.join("bowie")).unwrap_or_else(|(_, error)| panic!("{error}"));

    let heated_output = root.join("heated/pass_0/Data/Meshes").join(
        heated
            .model_path
            .replace('/', std::path::MAIN_SEPARATOR_STR),
    );
    let heated_nif = NifFile::load(&heated_output).expect("load converted heated cosmic knife");
    for texture_set in heated_nif
        .blocks
        .iter()
        .filter(|block| block.type_name == "BSShaderTextureSet")
    {
        let textures = match texture_set.get_field("Textures") {
            Some(NifValue::Array(textures)) => textures,
            _ => continue,
        };
        assert!(
            !matches!(textures.get(2), Some(NifValue::String(path)) if path.to_ascii_lowercase().ends_with("nvdlc01_knifespear_g.dds")),
            "optional absent heated-knife glow must not remain referenced"
        );
    }
    for shader in heated_nif
        .blocks
        .iter()
        .filter(|block| block.type_name == "BSLightingShaderProperty")
    {
        assert_eq!(
            shader
                .get_field("Shader Flags 2")
                .map(NifValue::as_i64)
                .unwrap_or_default()
                & (1 << 6),
            0,
            "cleared optional glow must not leave the FO4 Glow_Map flag"
        );
    }

    let bowie_output_data = root.join("bowie/pass_0/Data");
    let bowie_output = bowie_output_data
        .join("Meshes")
        .join(bowie.model_path.replace('/', std::path::MAIN_SEPARATOR_STR));
    let bowie_nif = NifFile::load(&bowie_output).expect("load converted Bowie knife");
    let normal_path = bowie_nif
        .blocks
        .iter()
        .filter(|block| block.type_name == "BSShaderTextureSet")
        .filter_map(|block| match block.get_field("Textures") {
            Some(NifValue::Array(textures)) => textures.get(1),
            _ => None,
        })
        .find_map(|texture| match texture {
            NifValue::String(path)
                if path
                    .to_ascii_lowercase()
                    .ends_with("nvdlc04bowieknife_n.dds") =>
            {
                Some(path.clone())
            }
            _ => None,
        })
        .expect("converted Bowie normal slot");
    let normal_output = bowie_output_data.join(
        normal_path
            .replace('\\', std::path::MAIN_SEPARATOR_STR)
            .replace('/', std::path::MAIN_SEPARATOR_STR),
    );
    let bytes = std::fs::read(&normal_output).unwrap_or_else(|error| {
        panic!(
            "read emitted Bowie flat normal {}: {error}",
            normal_output.display()
        )
    });
    assert_eq!(&bytes[0..4], b"DDS ");
    assert_eq!(u32::from_le_bytes(bytes[12..16].try_into().unwrap()), 4);
    assert_eq!(u32::from_le_bytes(bytes[16..20].try_into().unwrap()), 4);
    assert_eq!(u32::from_le_bytes(bytes[28..32].try_into().unwrap()), 3);
    assert_eq!(&bytes[84..88], b"ATI2");
    assert_eq!(bytes.len(), 176);

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn optional_authoritative_bulk_melee_v1_nif_corpus_gate() {
    let Some(manifest) = load_authoritative_manifest() else {
        eprintln!("MELEE_NIF_CORPUS_MANIFEST is unset; authoritative corpus gate skipped");
        return;
    };
    assert!(
        manifest.authoritative,
        "external release-gate manifest must declare authoritative=true"
    );
    run_manifest_gate(&manifest);
}
