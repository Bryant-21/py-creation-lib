use std::path::{Path, PathBuf};
use std::sync::Arc;

use esp_authoring_core::plugin_runtime::{
    ParsedItem, ParsedPlugin, ParsedRecord, ParsedSubrecord, parse_plugin_file,
    resolve_form_id_to_form_key,
};

use super::emit::{AnimTextDataInputs, SubgraphInput, WeaponProfileInput};
use super::graph::StancePerspective;
use super::stance::{StanceFormKey, WeaponRaceFamily, WeaponSraf, WeaponSubgraphMetadata};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanonicalSubgraphField<S, K, B> {
    BehaviorGraph(S),
    InvalidBehaviorGraph,
    Path(S),
    SubgraphKeyword(K),
    TargetKeyword(K),
    Flags(B),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalSubgraphBlock<S, K, B> {
    pub behavior_graph: S,
    pub paths: Vec<S>,
    pub subgraph_keywords: Vec<K>,
    pub target_keywords: Vec<K>,
    pub flags: Option<B>,
}

pub fn parse_canonical_subgraphs<S, K, B>(
    fields: impl IntoIterator<Item = CanonicalSubgraphField<S, K, B>>,
) -> Vec<CanonicalSubgraphBlock<S, K, B>> {
    let mut blocks = Vec::new();
    let mut current = None;
    let mut pending_subgraph_keywords = Vec::new();
    let mut pending_target_keywords = Vec::new();
    let mut current_has_flags = false;

    for field in fields {
        match field {
            CanonicalSubgraphField::BehaviorGraph(behavior_graph) => {
                if let Some(previous) = current.take() {
                    blocks.push(previous);
                }
                current = Some(CanonicalSubgraphBlock {
                    behavior_graph,
                    paths: Vec::new(),
                    subgraph_keywords: std::mem::take(&mut pending_subgraph_keywords),
                    target_keywords: std::mem::take(&mut pending_target_keywords),
                    flags: None,
                });
                current_has_flags = false;
            }
            CanonicalSubgraphField::InvalidBehaviorGraph => {
                if let Some(previous) = current.take() {
                    blocks.push(previous);
                }
                current_has_flags = false;
            }
            CanonicalSubgraphField::SubgraphKeyword(keyword) => {
                if let Some(block) = current.as_mut().filter(|_| !current_has_flags) {
                    block.subgraph_keywords.push(keyword);
                } else {
                    pending_subgraph_keywords.push(keyword);
                }
            }
            CanonicalSubgraphField::TargetKeyword(keyword) => {
                if let Some(block) = current.as_mut().filter(|_| !current_has_flags) {
                    block.target_keywords.push(keyword);
                } else {
                    pending_target_keywords.push(keyword);
                }
            }
            CanonicalSubgraphField::Path(path) => {
                if let Some(block) = current.as_mut() {
                    block.paths.push(path);
                }
            }
            CanonicalSubgraphField::Flags(flags) => {
                if let Some(block) = current.as_mut() {
                    block.flags = Some(flags);
                    current_has_flags = true;
                }
            }
        }
    }

    if let Some(last) = current {
        blocks.push(last);
    }
    blocks
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DecodedAnimInputs {
    pub race_record_count: usize,
    pub subgraphs: Vec<SubgraphInput>,
    pub weapon_profiles: Vec<WeaponProfileInput>,
    pub base_stance_profiles: Vec<WeaponSubgraphMetadata>,
    pub target_plugin_name: String,
    pub idle_globs: Vec<String>,
    pub event_candidates: Vec<String>,
}

impl From<DecodedAnimInputs> for AnimTextDataInputs {
    fn from(decoded: DecodedAnimInputs) -> Self {
        Self {
            race_record_count: decoded.race_record_count,
            subgraphs: decoded.subgraphs,
            weapon_profiles: decoded.weapon_profiles,
            base_stance_profiles: decoded.base_stance_profiles,
            target_plugin_name: decoded.target_plugin_name,
            idle_globs: decoded.idle_globs,
            event_candidates: decoded.event_candidates,
        }
    }
}

pub fn subgraph_inputs_from_plugin(
    plugin_path: &Path,
    game: &str,
    base_plugin_paths: &[PathBuf],
) -> Result<DecodedAnimInputs, String> {
    let target = parse_plugin(plugin_path, game)?;
    let mut decoded = decode_target_plugin(&target)?;

    for base_path in base_plugin_paths {
        let base = parse_plugin(base_path, game)?;
        for record in records_with_signature(&base.root_items, "RACE") {
            decoded.base_stance_profiles.extend(
                weapon_profiles_from_record(record, &base)?
                    .into_iter()
                    .map(|profile| profile.stance),
            );
        }
    }
    Ok(decoded)
}

fn parse_plugin(path: &Path, game: &str) -> Result<ParsedPlugin, String> {
    let path_string = path
        .to_str()
        .ok_or_else(|| format!("plugin path is not valid UTF-8: {}", path.display()))?;
    parse_plugin_file(path_string, Some(game.to_string()), true)
        .map_err(|error| format!("failed to parse plugin {}: {error}", path.display()))
}

fn decode_target_plugin(plugin: &ParsedPlugin) -> Result<DecodedAnimInputs, String> {
    let races = records_with_signature(&plugin.root_items, "RACE");
    let mut decoded = DecodedAnimInputs {
        race_record_count: races.len(),
        target_plugin_name: plugin.plugin_name.clone(),
        ..DecodedAnimInputs::default()
    };

    for record in races {
        let race_dir = race_dir_from_skeletal_model(record);
        let blocks = canonical_blocks_from_record(record, plugin)?;
        decoded
            .subgraphs
            .extend(blocks.iter().map(|block| SubgraphInput {
                core_behavior: block.behavior_graph.clone(),
                sapt_chain: block.paths.clone(),
                race_dir: race_dir.clone(),
            }));
        decoded
            .weapon_profiles
            .extend(weapon_profiles_from_blocks(record, plugin, blocks)?);
        decoded.event_candidates.extend(
            record
                .subrecords
                .iter()
                .filter(|subrecord| subrecord.signature.as_str() == "ATKE")
                .map(|subrecord| decode_zstring(&subrecord.data)),
        );
    }

    for record in records_with_signature(&plugin.root_items, "IDLE") {
        for subrecord in &record.subrecords {
            match subrecord.signature.as_str() {
                "GNAM" => {
                    let value = decode_zstring(&subrecord.data);
                    if value.contains('*') {
                        decoded.idle_globs.push(value);
                    }
                }
                "ENAM" => {
                    let value = decode_zstring(&subrecord.data);
                    if is_ai_combat_idle_event(&value) {
                        decoded.event_candidates.push(value);
                    }
                }
                _ => {}
            }
        }
    }
    Ok(decoded)
}

fn records_with_signature<'a>(items: &'a [ParsedItem], signature: &str) -> Vec<&'a ParsedRecord> {
    fn collect<'a>(items: &'a [ParsedItem], signature: &str, records: &mut Vec<&'a ParsedRecord>) {
        for item in items {
            match item {
                ParsedItem::Group(group) => collect(&group.children, signature, records),
                ParsedItem::Record(record) if record.signature.as_str() == signature => {
                    records.push(record);
                }
                ParsedItem::Record(_) => {}
            }
        }
    }

    let mut records = Vec::new();
    collect(items, signature, &mut records);
    records
}

/// `Actors\<Race>` for this RACE, from its skeletal model (`ANAM`, e.g.
/// `actors\scorched\characterassets\skeleton.nif`). Humanoid creatures mount the shared
/// `Actors\Character\Behaviors\*` graphs, so a subgraph's core-behavior path names
/// `Character` and cannot identify the race that owns the animation project.
fn race_dir_from_skeletal_model(record: &ParsedRecord) -> Option<String> {
    record
        .subrecords
        .iter()
        .filter(|subrecord| subrecord.signature.as_str() == "ANAM")
        .map(|subrecord| decode_zstring(&subrecord.data))
        .find_map(|model| {
            let norm = model.replace('/', "\\");
            if !norm.to_ascii_lowercase().ends_with(".nif") {
                return None;
            }
            let parts: Vec<&str> = norm.split('\\').filter(|s| !s.is_empty()).collect();
            (parts.len() >= 3).then(|| parts[..2].join("\\"))
        })
}

fn canonical_blocks_from_record(
    record: &ParsedRecord,
    plugin: &ParsedPlugin,
) -> Result<Vec<CanonicalSubgraphBlock<String, StanceFormKey, Vec<u8>>>, String> {
    let mut fields = Vec::new();
    for subrecord in &record.subrecords {
        let field = match subrecord.signature.as_str() {
            "SGNM" => CanonicalSubgraphField::BehaviorGraph(decode_zstring(&subrecord.data)),
            "SAPT" => CanonicalSubgraphField::Path(decode_zstring(&subrecord.data)),
            "SAKD" => {
                CanonicalSubgraphField::SubgraphKeyword(decode_form_key(subrecord, plugin, record)?)
            }
            "STKD" => {
                CanonicalSubgraphField::TargetKeyword(decode_form_key(subrecord, plugin, record)?)
            }
            "SRAF" => CanonicalSubgraphField::Flags(subrecord.data.to_vec()),
            _ => continue,
        };
        fields.push(field);
    }
    Ok(parse_canonical_subgraphs(fields))
}

fn weapon_profiles_from_record(
    record: &ParsedRecord,
    plugin: &ParsedPlugin,
) -> Result<Vec<WeaponProfileInput>, String> {
    let blocks = canonical_blocks_from_record(record, plugin)?;
    weapon_profiles_from_blocks(record, plugin, blocks)
}

fn weapon_profiles_from_blocks(
    record: &ParsedRecord,
    plugin: &ParsedPlugin,
    blocks: Vec<CanonicalSubgraphBlock<String, StanceFormKey, Vec<u8>>>,
) -> Result<Vec<WeaponProfileInput>, String> {
    let own_plugin = Arc::<str>::from(plugin.plugin_name.as_str());
    let race_dir = race_dir_from_skeletal_model(record);
    let owner = resolve_form_id_to_form_key(record.form_id, &own_plugin, &plugin.header.masters);
    let owner_race = StanceFormKey {
        plugin: owner.plugin.to_string(),
        local: owner.object_id,
    };
    let sadd = record
        .subrecords
        .iter()
        .find(|subrecord| subrecord.signature.as_str() == "SADD")
        .map(|subrecord| decode_form_key(subrecord, plugin, record))
        .transpose()?;

    Ok(blocks
        .into_iter()
        .filter_map(|block| {
            let flags = block.flags?;
            let role = u16::from_le_bytes(flags.get(0..2)?.try_into().ok()?);
            let perspective_value = u16::from_le_bytes(flags.get(2..4)?.try_into().ok()?);
            let perspective = if perspective_value != 0 || block.subgraph_keywords.is_empty() {
                StancePerspective::FirstPerson
            } else {
                StancePerspective::ThirdPerson
            };
            let subgraph = SubgraphInput {
                core_behavior: block.behavior_graph.clone(),
                sapt_chain: block.paths.clone(),
                race_dir: race_dir.clone(),
            };
            let id = subgraph.id();
            Some(WeaponProfileInput {
                subgraph,
                stance: WeaponSubgraphMetadata {
                    race_family: WeaponRaceFamily {
                        owner_race: owner_race.clone(),
                        sadd: sadd.clone(),
                    },
                    perspective,
                    sakd: block.subgraph_keywords,
                    stkd: block.target_keywords,
                    core_behavior: block.behavior_graph,
                    sapt: block.paths,
                    sraf: WeaponSraf {
                        role,
                        perspective: perspective_value,
                    },
                    id,
                },
            })
        })
        .collect())
}

fn decode_form_key(
    subrecord: &ParsedSubrecord,
    plugin: &ParsedPlugin,
    record: &ParsedRecord,
) -> Result<StanceFormKey, String> {
    let bytes: [u8; 4] = subrecord
        .data
        .get(..4)
        .ok_or_else(|| {
            format!(
                "{} {:08X}.{} is {} byte(s), expected a four-byte FormID",
                record.signature,
                record.form_id,
                subrecord.signature,
                subrecord.data.len()
            )
        })?
        .try_into()
        .expect("four-byte slice");
    let own_plugin = Arc::<str>::from(plugin.plugin_name.as_str());
    let form_key = resolve_form_id_to_form_key(
        u32::from_le_bytes(bytes),
        &own_plugin,
        &plugin.header.masters,
    );
    Ok(StanceFormKey {
        plugin: form_key.plugin.to_string(),
        local: form_key.object_id,
    })
}

fn decode_zstring(data: &[u8]) -> String {
    let end = data
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(data.len());
    let value = &data[..end];
    match std::str::from_utf8(value) {
        Ok(value) => value.to_string(),
        Err(_) => value.iter().map(|byte| char::from(*byte)).collect(),
    }
}

fn is_ai_combat_idle_event(name: &str) -> bool {
    let lowercase = name.to_ascii_lowercase();
    ["evade", "dodge", "fire", "dynamic"]
        .iter()
        .any(|prefix| lowercase.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use esp_authoring_core::plugin_runtime::{
        ParsedRecord, ParsedSubrecord, insert_parsed_record_in_slot, plugin_handle_close_native,
        plugin_handle_new_native, plugin_handle_save_no_py, plugin_handle_store_ref,
    };

    use super::*;

    fn zstring(signature: &str, value: &str) -> ParsedSubrecord {
        let mut data = value.as_bytes().to_vec();
        data.push(0);
        ParsedSubrecord {
            signature: signature.into(),
            data: data.into(),
            semantic_type: None,
        }
    }

    fn form_id(signature: &str, value: u32) -> ParsedSubrecord {
        ParsedSubrecord {
            signature: signature.into(),
            data: value.to_le_bytes().to_vec().into(),
            semantic_type: Some("formid".to_string()),
        }
    }

    fn bytes(signature: &str, value: &[u8]) -> ParsedSubrecord {
        ParsedSubrecord {
            signature: signature.into(),
            data: value.to_vec().into(),
            semantic_type: None,
        }
    }

    fn record(signature: &str, form_id: u32, subrecords: Vec<ParsedSubrecord>) -> ParsedRecord {
        ParsedRecord {
            signature: signature.into(),
            form_id,
            flags: 0,
            version_control: 0,
            form_version: Some(131),
            version2: None,
            subrecords,
            raw_payload: None,
            parse_error: None,
        }
    }

    fn write_race_idle_fixture(dir: &Path) -> PathBuf {
        let name = "AnimFixture.esp";
        let path = dir.join(name);
        let handle = plugin_handle_new_native(name, Some("fo4")).expect("create fixture plugin");
        {
            let mut store = plugin_handle_store_ref().lock().unwrap();
            let slot = store.get_mut(&handle).unwrap();
            slot.parsed.header.masters = vec!["Fallout4.esm".to_string()];
            insert_parsed_record_in_slot(
                slot,
                record(
                    "RACE",
                    0x0100_0800,
                    vec![
                        zstring("EDID", "AnimFixtureRace"),
                        form_id("SAKD", 0x0001_2345),
                        form_id("STKD", 0x0002_3456),
                        zstring("SGNM", r"Actors\Fixture\Behaviors\Fixture.hkx"),
                        zstring("SAPT", r"Actors\Fixture\Animations"),
                        bytes("SRAF", &[7, 0, 0, 0]),
                        form_id("SADD", 0x0003_4567),
                        zstring("ATKE", "AttackPrimary"),
                    ],
                ),
            );
            insert_parsed_record_in_slot(
                slot,
                record(
                    "IDLE",
                    0x0100_0801,
                    vec![
                        zstring("EDID", "AnimFixtureIdle"),
                        zstring("ENAM", "evadeLeft"),
                        zstring("GNAM", r"Actors\Fixture\Animations\*.hkx"),
                    ],
                ),
            );
        }
        plugin_handle_save_no_py(handle, path.to_str().unwrap()).expect("save fixture plugin");
        assert!(plugin_handle_close_native(handle));
        path
    }

    fn write_humanoid_race_fixture(dir: &Path) -> PathBuf {
        let name = "HumanoidFixture.esp";
        let path = dir.join(name);
        let handle = plugin_handle_new_native(name, Some("fo4")).expect("create fixture plugin");
        {
            let mut store = plugin_handle_store_ref().lock().unwrap();
            let slot = store.get_mut(&handle).unwrap();
            slot.parsed.header.masters = vec!["Fallout4.esm".to_string()];
            insert_parsed_record_in_slot(
                slot,
                record(
                    "RACE",
                    0x0100_0900,
                    vec![
                        zstring("EDID", "ScorchedRace"),
                        zstring("ANAM", r"actors\scorched\characterassets\skeleton.nif"),
                        // Humanoid: the core behavior is the SHARED character graph.
                        zstring("SGNM", r"Actors\Character\Behaviors\GunBehavior.hkx"),
                        zstring("SAPT", r"Actors\Scorched\Animations"),
                        bytes("SRAF", &[7, 0, 0, 0]),
                    ],
                ),
            );
        }
        plugin_handle_save_no_py(handle, path.to_str().unwrap()).expect("save fixture plugin");
        assert!(plugin_handle_close_native(handle));
        path
    }

    /// A humanoid creature mounts the shared `Actors\Character` behaviors, so the
    /// subgraph's core path names `Character` and cannot identify the race that owns the
    /// project. The race dir must come from the RACE's own skeletal model (`ANAM`).
    #[test]
    fn race_dir_comes_from_skeletal_model_not_core_behavior() {
        let temp = tempfile::tempdir().unwrap();
        let plugin = write_humanoid_race_fixture(temp.path());
        let decoded =
            subgraph_inputs_from_plugin(&plugin, "fo4", std::slice::from_ref(&plugin)).unwrap();

        assert_eq!(decoded.subgraphs.len(), 1);
        assert_eq!(
            decoded.subgraphs[0].race_dir.as_deref(),
            Some(r"actors\scorched"),
            "race dir must follow the skeletal model, not the shared core behavior"
        );
    }

    #[test]
    fn canonical_parser_keeps_leading_and_post_flags_keywords_with_the_next_block() {
        use CanonicalSubgraphField::*;

        let blocks = parse_canonical_subgraphs([
            SubgraphKeyword(1u32),
            BehaviorGraph("A"),
            Path("A1"),
            Flags(vec![0]),
            TargetKeyword(2),
            BehaviorGraph("B"),
            TargetKeyword(3),
        ]);

        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].subgraph_keywords, vec![1]);
        assert!(blocks[0].target_keywords.is_empty());
        assert_eq!(blocks[1].target_keywords, vec![2, 3]);
    }

    #[test]
    fn decodes_race_idle_and_master_owned_form_ids_from_paths() {
        let temp = tempfile::tempdir().unwrap();
        let plugin = write_race_idle_fixture(temp.path());
        let decoded =
            subgraph_inputs_from_plugin(&plugin, "fo4", std::slice::from_ref(&plugin)).unwrap();

        assert_eq!(decoded.race_record_count, 1);
        assert_eq!(decoded.subgraphs.len(), 1);
        assert_eq!(
            decoded.subgraphs[0],
            SubgraphInput {
                core_behavior: r"Actors\Fixture\Behaviors\Fixture.hkx".to_string(),
                sapt_chain: vec![r"Actors\Fixture\Animations".to_string()],
                race_dir: None,
            }
        );
        assert_eq!(decoded.weapon_profiles.len(), 1);
        let stance = &decoded.weapon_profiles[0].stance;
        assert_eq!(stance.race_family.owner_race.plugin, "AnimFixture.esp");
        assert_eq!(stance.race_family.owner_race.local, 0x800);
        assert_eq!(
            stance.race_family.sadd,
            Some(StanceFormKey {
                plugin: "Fallout4.esm".to_string(),
                local: 0x34567,
            })
        );
        assert_eq!(stance.sakd[0].plugin, "Fallout4.esm");
        assert_eq!(stance.sakd[0].local, 0x12345);
        assert_eq!(stance.stkd[0].plugin, "Fallout4.esm");
        assert_eq!(stance.stkd[0].local, 0x23456);
        assert_eq!(decoded.base_stance_profiles, vec![stance.clone()]);
        assert_eq!(
            decoded.idle_globs,
            vec![r"Actors\Fixture\Animations\*.hkx".to_string()]
        );
        assert_eq!(decoded.event_candidates, vec!["AttackPrimary", "evadeLeft"]);
    }
}
