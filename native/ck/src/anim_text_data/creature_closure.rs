use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize, de::DeserializeOwned};

use super::bucket_files::anim_event_info_body;
use super::core::name_id;
use super::emit::{AnimTextDataInputs, SubgraphInput, generate_anim_text_data_with_progress};
use super::event_resolver::resolve_anim_events;
use super::race_decode::{DecodedAnimInputs, DecodedRaceAnimInput, subgraph_inputs_from_plugin};

const CONTRACT_VERSION: u32 = 1;
const POLICY_ID: &str = "all_creatures_v1";

#[derive(Debug, Deserialize)]
pub struct CreatureAnimTextContract {
    version: u32,
    policy_id: String,
    game: String,
    plugin_path: PathBuf,
    source_meshes_root: PathBuf,
    output_meshes_root: PathBuf,
    base_meshes_root: Option<PathBuf>,
    #[serde(default)]
    base_plugin_paths: Vec<PathBuf>,
    corpus_plan_path: PathBuf,
    execution_ledger_path: PathBuf,
    record_commit_ledger_path: PathBuf,
    expected_family_ids: Vec<String>,
    event_source: String,
    resolution_source: String,
    mod_prefix: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CorpusPlan {
    version: u32,
    jobs: Vec<CorpusJob>,
}

#[derive(Debug, Deserialize)]
struct CorpusJob {
    family_id: String,
    #[serde(default)]
    graph_paths: Vec<String>,
    #[serde(default)]
    artifacts: Vec<CorpusArtifact>,
}

#[derive(Debug, Deserialize)]
struct CorpusArtifact {
    kind: String,
    target_path: String,
}

#[derive(Debug, Deserialize)]
struct ExecutionLedger {
    version: u32,
    mode: String,
    strict_aborted: bool,
    families: Vec<ExecutionFamily>,
}

#[derive(Debug, Deserialize)]
struct ExecutionFamily {
    family_id: String,
    disposition: String,
    records_deferred: usize,
}

#[derive(Debug, Deserialize)]
struct RecordCommitReceipt {
    family_ids: Vec<String>,
    families: Vec<RecordFamilyReceipt>,
    record_count: usize,
    mapping_count: usize,
    reserved_form_keys: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct RecordCommitLedger {
    version: u32,
    receipt: RecordCommitReceipt,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RecordCommitDocument {
    Ledger(RecordCommitLedger),
    Receipt(RecordCommitReceipt),
}

impl RecordCommitDocument {
    fn into_receipt(self) -> Result<RecordCommitReceipt, String> {
        match self {
            Self::Ledger(ledger) if ledger.version == 2 => Ok(ledger.receipt),
            Self::Ledger(ledger) => Err(format!(
                "unsupported creature record commit ledger version {}",
                ledger.version
            )),
            Self::Receipt(receipt) => Ok(receipt),
        }
    }
}

#[derive(Debug, Deserialize)]
struct RecordFamilyReceipt {
    family_id: String,
    record_count: usize,
    mapping_count: usize,
    primary_mappings: Vec<serde_json::Value>,
    races: Vec<RecordRaceReceipt>,
}

#[derive(Debug, Deserialize)]
struct RecordRaceReceipt {
    form_key: String,
    attack_events: Vec<String>,
    attack_data_entries: usize,
}

#[derive(Debug)]
struct PlannedFamilyGraphs {
    receipt_paths: Vec<String>,
    by_mesh_relative: BTreeMap<String, String>,
}

#[derive(Debug)]
struct PreparedFamily {
    family_id: String,
    race_form_keys: Vec<String>,
    graph_paths: Vec<String>,
    subgraphs: Vec<SubgraphInput>,
    attacks: Vec<CreatureAttackEventReceipt>,
}

#[derive(Debug, Serialize)]
struct CreatureAnimTextClosureReceipt {
    version: u32,
    policy_id: &'static str,
    written: u32,
    families: Vec<CreatureAnimTextFamilyReceipt>,
}

#[derive(Debug, Serialize)]
struct CreatureAnimTextFamilyReceipt {
    family_id: String,
    status: &'static str,
    race_form_keys: Vec<String>,
    graph_paths: Vec<String>,
    emitted_files: Vec<String>,
    attack_events: Vec<CreatureAttackEventReceipt>,
}

#[derive(Debug, Clone, Serialize)]
struct CreatureAttackEventReceipt {
    event: String,
    race_form_key: String,
    atkd_index: usize,
    source: &'static str,
    targets: Vec<CreatureAnimTextTarget>,
}

#[derive(Debug, Clone, Eq, Ord, PartialEq, PartialOrd, Serialize)]
struct CreatureAnimTextTarget {
    graph_path: String,
    clip_name: String,
    annotation: String,
}

pub fn generate_creature_anim_text_closure(
    contract_json: &str,
    progress: &mut dyn FnMut(&str),
) -> Result<String, String> {
    generate_creature_anim_text_closure_with_decoder(contract_json, None, progress, |contract| {
        subgraph_inputs_from_plugin(
            &contract.plugin_path,
            &contract.game,
            &contract.base_plugin_paths,
        )
    })
}

pub fn generate_creature_anim_text_closure_with_decoder(
    contract_json: &str,
    base_meshes_root_override: Option<PathBuf>,
    progress: &mut dyn FnMut(&str),
    decode: impl FnOnce(&CreatureAnimTextContract) -> Result<DecodedAnimInputs, String>,
) -> Result<String, String> {
    let mut contract: CreatureAnimTextContract = serde_json::from_str(contract_json)
        .map_err(|error| format!("invalid creature AnimText contract: {error}"))?;
    if let Some(base_meshes_root) = base_meshes_root_override {
        contract.base_meshes_root = Some(base_meshes_root);
    }
    validate_contract(&contract)?;

    let plan: CorpusPlan = read_json(&contract.corpus_plan_path, "creature corpus plan")?;
    let execution: ExecutionLedger = read_json(
        &contract.execution_ledger_path,
        "creature corpus execution ledger",
    )?;
    let records: RecordCommitDocument = read_json(
        &contract.record_commit_ledger_path,
        "creature record commit receipt",
    )?;
    let records = records.into_receipt()?;
    let expected = expected_families(&contract.expected_family_ids)?;
    validate_execution(&execution, &expected)?;
    let graphs = planned_graphs(&plan, &expected, &contract.source_meshes_root)?;
    let record_families = validate_record_receipt(&records, &expected)?;

    let decoded = decode(&contract)?;
    let prepared = prepare_families(&contract, &expected, &graphs, &record_families, &decoded)?;

    progress(&format!(
        "creature AnimText: validated {} family contract(s)",
        prepared.len()
    ));
    let staging_parent = contract
        .output_meshes_root
        .parent()
        .ok_or_else(|| "output meshes root has no parent directory".to_string())?;
    fs::create_dir_all(staging_parent)
        .map_err(|error| format!("failed to create AnimText staging parent: {error}"))?;
    let staging = tempfile::Builder::new()
        .prefix("creature-animtext-")
        .tempdir_in(staging_parent)
        .map_err(|error| format!("failed to create AnimText staging directory: {error}"))?;
    let staged_meshes_root = staging.path().join("Meshes");
    fs::create_dir_all(&staged_meshes_root)
        .map_err(|error| format!("failed to create staged Meshes directory: {error}"))?;
    let inputs = AnimTextDataInputs::from(decoded.clone());
    generate_anim_text_data_with_progress(
        &inputs,
        &contract.source_meshes_root,
        &staged_meshes_root,
        contract.base_meshes_root.as_deref(),
        contract.mod_prefix.as_deref(),
        progress,
    )?;

    let receipt = verify_outputs(&contract, &staged_meshes_root, &decoded, prepared)?;
    publish_anim_text_data(
        &staged_meshes_root,
        &contract.output_meshes_root,
        staging.path(),
    )?;
    serde_json::to_string(&receipt)
        .map_err(|error| format!("failed to serialize creature AnimText receipt: {error}"))
}

fn validate_contract(contract: &CreatureAnimTextContract) -> Result<(), String> {
    if contract.version != CONTRACT_VERSION
        || contract.policy_id != POLICY_ID
        || contract.event_source != "emitted_race_atkd_atke"
        || contract.resolution_source != "emitted_family_graphs"
    {
        return Err("unsupported creature AnimText contract".to_string());
    }
    if !contract.game.eq_ignore_ascii_case("fo4") {
        return Err("creature AnimText closure requires an FO4 target".to_string());
    }
    for (label, path) in [
        ("target plugin", &contract.plugin_path),
        ("source meshes", &contract.source_meshes_root),
        ("corpus plan", &contract.corpus_plan_path),
        ("execution ledger", &contract.execution_ledger_path),
        ("record commit receipt", &contract.record_commit_ledger_path),
    ] {
        if !path.exists() {
            return Err(format!(
                "creature AnimText {label} is missing: {}",
                path.display()
            ));
        }
    }
    Ok(())
}

fn read_json<T: DeserializeOwned>(path: &Path, label: &str) -> Result<T, String> {
    let bytes = fs::read(path).map_err(|error| format!("failed to read {label}: {error}"))?;
    serde_json::from_slice(&bytes).map_err(|error| format!("invalid {label}: {error}"))
}

fn folded(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn expected_families(values: &[String]) -> Result<BTreeMap<String, String>, String> {
    let mut expected = BTreeMap::new();
    for value in values {
        let canonical = value.trim();
        if canonical.is_empty()
            || expected
                .insert(folded(canonical), canonical.to_string())
                .is_some()
        {
            return Err("creature AnimText expected families are empty or duplicated".to_string());
        }
    }
    if expected.is_empty() {
        return Err("creature AnimText contract has no families".to_string());
    }
    Ok(expected)
}

fn validate_execution(
    execution: &ExecutionLedger,
    expected: &BTreeMap<String, String>,
) -> Result<(), String> {
    if execution.version != CONTRACT_VERSION
        || execution.mode != "strict"
        || execution.strict_aborted
    {
        return Err("creature corpus did not commit in strict mode".to_string());
    }
    let mut actual = BTreeSet::new();
    for family in &execution.families {
        let family_id = folded(&family.family_id);
        if !expected.contains_key(&family_id)
            || !actual.insert(family_id)
            || family.disposition != "published"
            || family.records_deferred != 0
        {
            return Err("creature execution family receipt is incomplete".to_string());
        }
    }
    if actual != expected.keys().cloned().collect() {
        return Err("creature execution families do not match the contract".to_string());
    }
    Ok(())
}

fn planned_graphs(
    plan: &CorpusPlan,
    expected: &BTreeMap<String, String>,
    source_meshes_root: &Path,
) -> Result<BTreeMap<String, PlannedFamilyGraphs>, String> {
    if plan.version != CONTRACT_VERSION {
        return Err("unsupported creature corpus plan version".to_string());
    }
    let mut graph_maps = expected
        .keys()
        .map(|family| (family.clone(), BTreeMap::new()))
        .collect::<BTreeMap<_, BTreeMap<String, String>>>();
    for job in &plan.jobs {
        let family_id = folded(&job.family_id);
        let Some(family_graphs) = graph_maps.get_mut(&family_id) else {
            return Err("creature corpus plan contains an unexpected family".to_string());
        };
        let graph_paths = if job.graph_paths.is_empty() {
            job.artifacts
                .iter()
                .filter(|artifact| {
                    matches!(
                        artifact.kind.as_str(),
                        "character_hkx" | "root_behavior_hkx" | "core_behavior_hkx"
                    )
                })
                .map(|artifact| artifact.target_path.as_str())
                .collect::<Vec<_>>()
        } else {
            job.graph_paths.iter().map(String::as_str).collect()
        };
        for graph_path in graph_paths {
            let mesh_relative_display = mesh_relative_display_path(graph_path)?;
            let mesh_relative = mesh_relative_display.to_ascii_lowercase();
            if !mesh_relative.ends_with(".hkx")
                || family_graphs
                    .insert(mesh_relative.clone(), graph_path.to_string())
                    .is_some()
            {
                return Err(format!(
                    "creature family {} has an invalid or duplicate graph path",
                    job.family_id
                ));
            }
            if !source_meshes_root.join(&mesh_relative_display).is_file() {
                return Err(format!("staged creature graph is missing: {graph_path}",));
            }
        }
    }
    graph_maps
        .into_iter()
        .map(|(family, by_mesh_relative)| {
            if by_mesh_relative.is_empty() {
                return Err(format!("creature family {family} has no staged graphs"));
            }
            let receipt_paths = by_mesh_relative.values().cloned().collect();
            Ok((
                family,
                PlannedFamilyGraphs {
                    receipt_paths,
                    by_mesh_relative,
                },
            ))
        })
        .collect()
}

fn mesh_relative_path(value: &str) -> Result<String, String> {
    Ok(mesh_relative_display_path(value)?.to_ascii_lowercase())
}

fn mesh_relative_display_path(value: &str) -> Result<String, String> {
    let normalized = value.trim().replace('\\', "/");
    let normalized = normalized
        .strip_prefix("data/")
        .or_else(|| normalized.strip_prefix("Data/"))
        .unwrap_or(&normalized);
    let normalized = normalized
        .strip_prefix("meshes/")
        .or_else(|| normalized.strip_prefix("Meshes/"))
        .unwrap_or(normalized)
        .to_string();
    if normalized.is_empty()
        || normalized.starts_with('/')
        || normalized
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
    {
        return Err(format!("invalid creature graph path {value:?}"));
    }
    Ok(normalized)
}

fn validate_record_receipt<'a>(
    receipt: &'a RecordCommitReceipt,
    expected: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, &'a RecordFamilyReceipt>, String> {
    let family_ids = receipt
        .family_ids
        .iter()
        .map(|family| folded(family))
        .collect::<BTreeSet<_>>();
    if family_ids != expected.keys().cloned().collect() {
        return Err("record commit family IDs do not match the contract".to_string());
    }
    let reserved = receipt
        .reserved_form_keys
        .iter()
        .map(|form_key| folded(form_key))
        .collect::<BTreeSet<_>>();
    if reserved.len() != receipt.record_count {
        return Err("record commit reserved FormKey count does not reconcile".to_string());
    }
    let mut families = BTreeMap::new();
    let mut record_count = 0usize;
    let mut mapping_count = 0usize;
    for family in &receipt.families {
        let family_id = folded(&family.family_id);
        if !expected.contains_key(&family_id)
            || families.insert(family_id, family).is_some()
            || family.record_count == 0
            || family.mapping_count == 0
            || family.primary_mappings.len() != family.mapping_count
            || family.races.is_empty()
        {
            return Err("record commit family receipt is incomplete".to_string());
        }
        record_count += family.record_count;
        mapping_count += family.mapping_count;
        for race in &family.races {
            let events = race
                .attack_events
                .iter()
                .map(|event| folded(event))
                .collect::<BTreeSet<_>>();
            if folded(&race.form_key).is_empty()
                || !reserved.contains(&folded(&race.form_key))
                || events.len() != race.attack_events.len()
                || race.attack_data_entries != race.attack_events.len()
            {
                return Err("record commit RACE attack receipt is incomplete".to_string());
            }
        }
    }
    if families.keys().cloned().collect::<BTreeSet<_>>() != expected.keys().cloned().collect()
        || record_count != receipt.record_count
        || mapping_count != receipt.mapping_count
    {
        return Err("record commit receipt counts do not reconcile".to_string());
    }
    Ok(families)
}

fn prepare_families(
    contract: &CreatureAnimTextContract,
    expected: &BTreeMap<String, String>,
    graphs: &BTreeMap<String, PlannedFamilyGraphs>,
    records: &BTreeMap<String, &RecordFamilyReceipt>,
    decoded: &DecodedAnimInputs,
) -> Result<Vec<PreparedFamily>, String> {
    let mut decoded_races = BTreeMap::new();
    for race in &decoded.races {
        if decoded_races.insert(folded(&race.form_key), race).is_some() {
            return Err(format!(
                "target plugin contains duplicate RACE {}",
                race.form_key
            ));
        }
    }
    let mut prepared = Vec::new();
    for (family_key, family_id) in expected {
        let graph_plan = &graphs[family_key];
        let record_family = records[family_key];
        let mut race_form_keys = Vec::new();
        let mut subgraphs = Vec::new();
        let mut attacks = Vec::new();
        for race_receipt in &record_family.races {
            let Some(race) = decoded_races.get(&folded(&race_receipt.form_key)) else {
                return Err(format!(
                    "committed RACE {} is absent from the target plugin",
                    race_receipt.form_key
                ));
            };
            validate_decoded_race(race_receipt, race)?;
            if race.subgraphs.is_empty() {
                return Err(format!(
                    "committed RACE {} has no emitted subgraph",
                    race_receipt.form_key
                ));
            }
            race_form_keys.push(race_receipt.form_key.clone());
            subgraphs.extend(race.subgraphs.iter().cloned());
            for (atkd_index, event) in race.attack_events.iter().enumerate() {
                let targets = resolve_family_attack(
                    event,
                    &race.subgraphs,
                    graph_plan,
                    &contract.source_meshes_root,
                )?;
                attacks.push(CreatureAttackEventReceipt {
                    event: event.clone(),
                    race_form_key: race_receipt.form_key.clone(),
                    atkd_index,
                    source: "emitted_race_atkd_atke",
                    targets,
                });
            }
        }
        race_form_keys.sort_by_key(|form_key| folded(form_key));
        prepared.push(PreparedFamily {
            family_id: family_id.clone(),
            race_form_keys,
            graph_paths: graph_plan.receipt_paths.clone(),
            subgraphs,
            attacks,
        });
    }
    Ok(prepared)
}

fn validate_decoded_race(
    receipt: &RecordRaceReceipt,
    decoded: &DecodedRaceAnimInput,
) -> Result<(), String> {
    let expected = receipt
        .attack_events
        .iter()
        .map(|event| folded(event))
        .collect::<BTreeSet<_>>();
    let actual = decoded
        .attack_events
        .iter()
        .map(|event| folded(event))
        .collect::<BTreeSet<_>>();
    if actual != expected
        || decoded.attack_events.len() != receipt.attack_events.len()
        || decoded.attack_data_entries != receipt.attack_data_entries
    {
        return Err(format!(
            "target RACE {} ATKD/ATKE fields do not match the commit receipt",
            receipt.form_key
        ));
    }
    Ok(())
}

fn resolve_family_attack(
    event: &str,
    subgraphs: &[SubgraphInput],
    graph_plan: &PlannedFamilyGraphs,
    source_meshes_root: &Path,
) -> Result<Vec<CreatureAnimTextTarget>, String> {
    let mut targets = BTreeSet::new();
    let mut seen_cores = BTreeSet::new();
    for subgraph in subgraphs {
        let core_relative = mesh_relative_path(&subgraph.core_behavior)?;
        if !seen_cores.insert(core_relative.clone()) {
            continue;
        }
        let Some(receipt_graph_path) = graph_plan.by_mesh_relative.get(&core_relative) else {
            return Err(format!(
                "RACE core graph {} is not in its staged family plan",
                subgraph.core_behavior
            ));
        };
        let core_file =
            source_meshes_root.join(mesh_relative_display_path(&subgraph.core_behavior)?);
        for resolved in resolve_anim_events(&core_file, &[event.to_string()]) {
            for clip_name in resolved.clips {
                if !clip_name.trim().is_empty() {
                    targets.insert(CreatureAnimTextTarget {
                        graph_path: receipt_graph_path.clone(),
                        clip_name,
                        annotation: String::new(),
                    });
                }
            }
        }
    }
    if targets.is_empty() {
        return Err(format!(
            "RACE attack event {event:?} does not resolve to a target clip"
        ));
    }
    Ok(targets.into_iter().collect())
}

fn verify_outputs(
    contract: &CreatureAnimTextContract,
    emitted_meshes_root: &Path,
    decoded: &DecodedAnimInputs,
    prepared: Vec<PreparedFamily>,
) -> Result<CreatureAnimTextClosureReceipt, String> {
    let mut all_emitted = BTreeSet::new();
    let mut families = Vec::new();
    for family in prepared {
        let mut emitted = BTreeSet::new();
        let mut seen_cores = BTreeSet::new();
        for subgraph in &family.subgraphs {
            let animation_file = format!("AnimTextData/AnimationFileData/{}.txt", subgraph.id());
            verify_nonempty_file(emitted_meshes_root, &animation_file)?;
            emitted.insert(animation_file);

            let core_relative = mesh_relative_path(&subgraph.core_behavior)?;
            if !seen_cores.insert(core_relative.clone()) {
                continue;
            }
            let core_file = contract
                .source_meshes_root
                .join(mesh_relative_display_path(&subgraph.core_behavior)?);
            let events = resolve_anim_events(&core_file, &decoded.event_candidates);
            if events.is_empty() {
                continue;
            }
            let event_file = format!(
                "AnimTextData/AnimEventInfo/{}.txt",
                name_id(&subgraph.core_behavior)
            );
            let actual = fs::read(emitted_meshes_root.join(&event_file))
                .map_err(|error| format!("failed to read emitted {event_file}: {error}"))?;
            let expected = anim_event_info_body(&subgraph.core_behavior, &events);
            if actual != expected {
                return Err(format!(
                    "emitted {event_file} does not match resolved events"
                ));
            }
            emitted.insert(event_file);
        }
        all_emitted.extend(emitted.iter().cloned());
        families.push(CreatureAnimTextFamilyReceipt {
            family_id: family.family_id,
            status: "passed",
            race_form_keys: family.race_form_keys,
            graph_paths: family.graph_paths,
            emitted_files: emitted.into_iter().collect(),
            attack_events: family.attacks,
        });
    }
    let written = u32::try_from(all_emitted.len())
        .map_err(|_| "creature AnimText receipt exceeds u32 file count".to_string())?;
    if written == 0 {
        return Err("creature AnimText closure emitted no files".to_string());
    }
    Ok(CreatureAnimTextClosureReceipt {
        version: CONTRACT_VERSION,
        policy_id: POLICY_ID,
        written,
        families,
    })
}

fn publish_anim_text_data(
    staged_meshes_root: &Path,
    output_meshes_root: &Path,
    staging_root: &Path,
) -> Result<(), String> {
    let staged = staged_meshes_root.join("AnimTextData");
    if !staged.is_dir() {
        return Err("staged creature AnimTextData tree is missing".to_string());
    }
    fs::create_dir_all(output_meshes_root)
        .map_err(|error| format!("failed to create output Meshes directory: {error}"))?;
    let destination = output_meshes_root.join("AnimTextData");
    let backup = staging_root.join("previous-AnimTextData");
    let had_previous = destination.exists();
    if had_previous {
        fs::rename(&destination, &backup)
            .map_err(|error| format!("failed to stage previous AnimTextData: {error}"))?;
    }
    if let Err(error) = fs::rename(&staged, &destination) {
        if had_previous {
            let _ = fs::rename(&backup, &destination);
        }
        return Err(format!("failed to publish creature AnimTextData: {error}"));
    }
    Ok(())
}

fn verify_nonempty_file(root: &Path, relative: &str) -> Result<(), String> {
    let path = root.join(relative);
    let metadata = fs::metadata(&path).map_err(|error| {
        format!("required creature AnimText output is missing: {relative}: {error}")
    })?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err(format!(
            "required creature AnimText output is empty: {relative}"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mesh_relative_paths_accept_explicit_mesh_roots_only() {
        assert_eq!(
            mesh_relative_path("Meshes/Actors/Canis/Behaviors/Core.hkx").unwrap(),
            "actors/canis/behaviors/core.hkx"
        );
        assert_eq!(
            mesh_relative_path("Actors\\Canis\\Behaviors\\Core.hkx").unwrap(),
            "actors/canis/behaviors/core.hkx"
        );
        assert!(mesh_relative_path("Meshes/../Core.hkx").is_err());
    }

    #[test]
    fn expected_family_contract_rejects_case_duplicates() {
        assert!(
            expected_families(&["Family-Canis".to_string(), "family-canis".to_string()]).is_err()
        );
    }

    #[test]
    fn record_commit_document_accepts_canonical_ledger_envelope() {
        let document: RecordCommitDocument = serde_json::from_value(serde_json::json!({
            "version": 2,
            "intent": {},
            "intent_blake3": "intent",
            "receipt": {
                "family_ids": [],
                "families": [],
                "record_count": 0,
                "mapping_count": 0,
                "reserved_form_keys": []
            },
            "receipt_blake3": "receipt"
        }))
        .unwrap();
        let receipt = document.into_receipt().unwrap();
        assert_eq!(receipt.record_count, 0);
    }

    #[test]
    fn record_receipt_accepts_matching_zero_attack_pair() {
        let expected = expected_families(&["family-passive".to_string()]).unwrap();
        let receipt = RecordCommitReceipt {
            family_ids: vec!["family-passive".to_string()],
            families: vec![RecordFamilyReceipt {
                family_id: "family-passive".to_string(),
                record_count: 2,
                mapping_count: 1,
                primary_mappings: vec![serde_json::json!({})],
                races: vec![RecordRaceReceipt {
                    form_key: "000800@Target.esp".to_string(),
                    attack_events: Vec::new(),
                    attack_data_entries: 0,
                }],
            }],
            record_count: 2,
            mapping_count: 1,
            reserved_form_keys: vec![
                "000800@Target.esp".to_string(),
                "000801@Target.esp".to_string(),
            ],
        };
        assert!(validate_record_receipt(&receipt, &expected).is_ok());
    }

    #[test]
    fn record_receipt_requires_every_race_attack_pair() {
        let expected = expected_families(&["family-canis".to_string()]).unwrap();
        let receipt = RecordCommitReceipt {
            family_ids: vec!["family-canis".to_string()],
            families: vec![RecordFamilyReceipt {
                family_id: "family-canis".to_string(),
                record_count: 2,
                mapping_count: 1,
                primary_mappings: vec![serde_json::json!({})],
                races: vec![RecordRaceReceipt {
                    form_key: "000800@Target.esp".to_string(),
                    attack_events: vec!["attackStart".to_string()],
                    attack_data_entries: 0,
                }],
            }],
            record_count: 2,
            mapping_count: 1,
            reserved_form_keys: vec![
                "000800@Target.esp".to_string(),
                "000801@Target.esp".to_string(),
            ],
        };
        assert!(validate_record_receipt(&receipt, &expected).is_err());
    }

    #[test]
    fn anim_text_tree_publish_replaces_only_after_staging() {
        let temp = tempfile::tempdir().unwrap();
        let output = temp.path().join("output/Meshes");
        let staged = temp.path().join("staging/Meshes");
        fs::create_dir_all(output.join("AnimTextData")).unwrap();
        fs::write(output.join("AnimTextData/old.txt"), b"old").unwrap();
        fs::create_dir_all(staged.join("AnimTextData")).unwrap();
        fs::write(staged.join("AnimTextData/new.txt"), b"new").unwrap();

        publish_anim_text_data(&staged, &output, &temp.path().join("staging")).unwrap();

        assert_eq!(
            fs::read(output.join("AnimTextData/new.txt")).unwrap(),
            b"new"
        );
        assert!(!output.join("AnimTextData/old.txt").exists());
    }
}
