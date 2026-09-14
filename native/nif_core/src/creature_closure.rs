use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::convert_file::{ConvertFileOptions, SourceRigNifKind, stage_preserve_source_rig_nif};
use crate::model::{NifBlock, NifFile, NifValue};
use materials_native::texture_convert::{
    GamebryoSpecParams, gamebryo_normal_envmask_to_fo4_specgloss_buffers,
};

pub const CREATURE_CLOSURE_RECEIPT_VERSION: u32 = 2;
pub const CREATURE_CLOSURE_HASH_ALGORITHM: &str = "blake3";
static TRANSACTION_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CreatureNifRole {
    Body,
    Skeleton,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreatureNifInput {
    pub role: CreatureNifRole,
    pub source_data_relative_path: String,
    pub source_owner: String,
    pub body_variant: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatureClosureRequest {
    pub source_game: String,
    pub source_data_root: PathBuf,
    pub private_staging_root: PathBuf,
    pub target_namespace: String,
    pub texture_fallbacks: BTreeMap<String, String>,
    pub inputs: Vec<CreatureNifInput>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CreatureTerminalKind {
    InvalidRequest,
    InvalidPath,
    MissingNif,
    MissingMaterial,
    MissingTexture,
    InvalidMaterial,
    InvalidTexture,
    UnsupportedBlockClass,
    UnsupportedMaterialClass,
    UnsupportedCollisionClass,
    TargetCollision,
    ConversionFailed,
    ValidationFailed,
    InvalidReceipt,
    Io,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreatureClosureError {
    pub kind: CreatureTerminalKind,
    pub input_key: Option<String>,
    pub source_data_relative_path: Option<String>,
    pub message: String,
}

impl std::fmt::Display for CreatureClosureError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(input_key) = &self.input_key {
            write!(
                formatter,
                "{:?} for {input_key}: {}",
                self.kind, self.message
            )
        } else {
            write!(formatter, "{:?}: {}", self.kind, self.message)
        }
    }
}

impl std::error::Error for CreatureClosureError {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CreatureCollisionDisposition {
    None,
    ArticulatedDeferredForHkx {
        source_block_types: Vec<String>,
    },
    ArticulatedEmbeddedFo4 {
        source_block_types: Vec<String>,
        body_count: u32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CreatureInputDispositionKind {
    ConvertedPreserveSourceRig,
    DeduplicatedPreserveSourceRig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreatureInputDisposition {
    pub input_key: String,
    pub role: CreatureNifRole,
    pub source_data_relative_path: String,
    pub source_byte_len: u64,
    pub source_blake3: String,
    pub source_owner: String,
    pub body_variant: Option<String>,
    pub disposition: CreatureInputDispositionKind,
    pub collision: CreatureCollisionDisposition,
    pub target_data_relative_path: String,
    pub output_fingerprint: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CreatureArtifactKind {
    Nif,
    Bgsm,
    Bgem,
    Dds,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreatureArtifactReceipt {
    pub kind: CreatureArtifactKind,
    pub target_data_relative_path: String,
    pub byte_len: u64,
    pub fingerprint: String,
    pub source_inputs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreatureClosureReceipt {
    pub receipt_version: u32,
    pub source_game: String,
    pub target_game: String,
    pub target_namespace: String,
    pub hash_algorithm: String,
    pub request_blake3: String,
    pub receipt_hash: String,
    pub inputs: Vec<CreatureInputDisposition>,
    pub artifacts: Vec<CreatureArtifactReceipt>,
}

impl CreatureClosureReceipt {
    pub fn canonical_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    pub fn validate(&self) -> Result<(), CreatureClosureError> {
        validate_receipt(self)
    }

    pub fn parse_and_validate(json: &str) -> Result<Self, CreatureClosureError> {
        parse_and_validate_creature_closure_receipt(json)
    }

    pub fn verify_artifact_bytes(
        &self,
        target_data_relative_path: &str,
        bytes: &[u8],
    ) -> Result<&CreatureArtifactReceipt, CreatureClosureError> {
        self.validate()?;
        let artifact = self
            .artifacts
            .iter()
            .find(|artifact| artifact.target_data_relative_path == target_data_relative_path)
            .ok_or_else(|| {
                error(
                    CreatureTerminalKind::InvalidReceipt,
                    None,
                    Some(target_data_relative_path.to_string()),
                    "artifact path is not declared by the receipt",
                )
            })?;
        if artifact.byte_len != bytes.len() as u64 || artifact.fingerprint != fingerprint(bytes) {
            return Err(error(
                CreatureTerminalKind::InvalidReceipt,
                None,
                Some(target_data_relative_path.to_string()),
                "artifact bytes do not match the receipt length and BLAKE3 fingerprint",
            ));
        }
        Ok(artifact)
    }
}

pub fn seal_embedded_creature_collision(
    receipt: &mut CreatureClosureReceipt,
    staged_data_root: &Path,
    target_data_relative_path: &str,
    body_count: usize,
) -> Result<(), CreatureClosureError> {
    if body_count == 0 || body_count > u32::MAX as usize {
        return Err(error(
            CreatureTerminalKind::InvalidRequest,
            None,
            Some(target_data_relative_path.to_string()),
            "embedded creature collision has an invalid body count",
        ));
    }
    let skeleton_input = receipt
        .inputs
        .iter()
        .find(|input| {
            input.role == CreatureNifRole::Skeleton
                && input.target_data_relative_path == target_data_relative_path
        })
        .ok_or_else(|| {
            error(
                CreatureTerminalKind::InvalidReceipt,
                None,
                Some(target_data_relative_path.to_string()),
                "embedded collision target is not the receipt's visual skeleton",
            )
        })?;
    match &skeleton_input.collision {
        CreatureCollisionDisposition::ArticulatedDeferredForHkx { .. } => {}
        _ => {
            return Err(error(
                CreatureTerminalKind::InvalidReceipt,
                Some(skeleton_input.input_key.clone()),
                Some(target_data_relative_path.to_string()),
                "visual skeleton collision was not deferred for reconstruction",
            ));
        }
    }
    let skeleton_input_key = skeleton_input.input_key.clone();
    let staged_path = staged_data_root.join(native_relative(target_data_relative_path));
    let bytes = fs::read(&staged_path).map_err(|io| {
        error(
            CreatureTerminalKind::Io,
            Some(skeleton_input_key.clone()),
            Some(target_data_relative_path.to_string()),
            format!("read collision-bearing visual skeleton: {io}"),
        )
    })?;
    let nif = NifFile::from_bytes(&bytes, Some(staged_path.clone())).map_err(|failure| {
        error(
            CreatureTerminalKind::ValidationFailed,
            Some(skeleton_input_key.clone()),
            Some(target_data_relative_path.to_string()),
            format!("reread collision-bearing visual skeleton: {failure}"),
        )
    })?;
    if nif.find_blocks("bhkRagdollSystem").len() != 1
        || nif.find_blocks("bhkNPCollisionObject").len() < body_count
    {
        return Err(error(
            CreatureTerminalKind::ValidationFailed,
            Some(skeleton_input_key.clone()),
            Some(target_data_relative_path.to_string()),
            "visual skeleton does not contain the complete embedded FO4 ragdoll closure",
        ));
    }

    let output_fingerprint = fingerprint(&bytes);
    for input in receipt
        .inputs
        .iter_mut()
        .filter(|input| input.target_data_relative_path == target_data_relative_path)
    {
        input.output_fingerprint = output_fingerprint.clone();
        if let CreatureCollisionDisposition::ArticulatedDeferredForHkx { source_block_types } =
            &input.collision
        {
            input.collision = CreatureCollisionDisposition::ArticulatedEmbeddedFo4 {
                source_block_types: source_block_types.clone(),
                body_count: body_count as u32,
            };
        }
    }
    let artifact = receipt
        .artifacts
        .iter_mut()
        .find(|artifact| artifact.target_data_relative_path == target_data_relative_path)
        .ok_or_else(|| {
            error(
                CreatureTerminalKind::InvalidReceipt,
                Some(skeleton_input_key),
                Some(target_data_relative_path.to_string()),
                "visual skeleton has no matching staged artifact",
            )
        })?;
    artifact.byte_len = bytes.len() as u64;
    artifact.fingerprint = output_fingerprint;
    receipt.receipt_hash = receipt_hash(receipt)?;
    receipt.validate()
}

pub fn refresh_staged_creature_nif_artifact(
    receipt: &mut CreatureClosureReceipt,
    staged_data_root: &Path,
    target_data_relative_path: &str,
) -> Result<(), CreatureClosureError> {
    let staged_path = staged_data_root.join(native_relative(target_data_relative_path));
    let bytes = fs::read(&staged_path).map_err(|io| {
        error(
            CreatureTerminalKind::Io,
            None,
            Some(target_data_relative_path.to_string()),
            format!("read updated creature NIF: {io}"),
        )
    })?;
    NifFile::from_bytes(&bytes, Some(staged_path)).map_err(|failure| {
        error(
            CreatureTerminalKind::ValidationFailed,
            None,
            Some(target_data_relative_path.to_string()),
            format!("reread updated creature NIF: {failure}"),
        )
    })?;
    let output_fingerprint = fingerprint(&bytes);
    let matching_input_found = receipt
        .inputs
        .iter()
        .any(|input| input.target_data_relative_path == target_data_relative_path);
    if !matching_input_found {
        return Err(error(
            CreatureTerminalKind::InvalidReceipt,
            None,
            Some(target_data_relative_path.to_string()),
            "updated creature NIF is not declared by the receipt",
        ));
    }
    for input in receipt
        .inputs
        .iter_mut()
        .filter(|input| input.target_data_relative_path == target_data_relative_path)
    {
        input.output_fingerprint = output_fingerprint.clone();
    }
    let artifact = receipt
        .artifacts
        .iter_mut()
        .find(|artifact| artifact.target_data_relative_path == target_data_relative_path)
        .ok_or_else(|| {
            error(
                CreatureTerminalKind::InvalidReceipt,
                None,
                Some(target_data_relative_path.to_string()),
                "updated creature NIF has no matching staged artifact",
            )
        })?;
    artifact.byte_len = bytes.len() as u64;
    artifact.fingerprint = output_fingerprint;
    receipt.receipt_hash = receipt_hash(receipt)?;
    receipt.validate()
}

pub fn parse_and_validate_creature_closure_receipt(
    json: &str,
) -> Result<CreatureClosureReceipt, CreatureClosureError> {
    let receipt: CreatureClosureReceipt = serde_json::from_str(json).map_err(|failure| {
        error(
            CreatureTerminalKind::InvalidReceipt,
            None,
            None,
            format!("parse receipt JSON: {failure}"),
        )
    })?;
    receipt.validate()?;
    let canonical = receipt.canonical_json().map_err(|failure| {
        error(
            CreatureTerminalKind::InvalidReceipt,
            None,
            None,
            format!("serialize canonical receipt: {failure}"),
        )
    })?;
    if canonical != json {
        return Err(error(
            CreatureTerminalKind::InvalidReceipt,
            None,
            None,
            "receipt JSON is valid but not canonical",
        ));
    }
    Ok(receipt)
}

pub fn compute_creature_closure_request_blake3(
    request: &CreatureClosureRequest,
) -> Result<String, CreatureClosureError> {
    let source_game = normalize_source_game(&request.source_game)?;
    let namespace = normalize_namespace(&request.target_namespace)?;
    let prepared = prepare_inputs(request, source_game, &namespace)?;
    Ok(request_blake3_from_prepared(&prepared))
}

#[derive(Debug, Clone)]
struct PreparedInput {
    input_key: String,
    role: CreatureNifRole,
    source_relative: String,
    source_bytes: Vec<u8>,
    source_byte_len: u64,
    source_blake3: String,
    source_owner: String,
    body_variant: Option<String>,
    target_relative: String,
    collision: CreatureCollisionDisposition,
    source_materials: Vec<String>,
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Serialize)]
struct CanonicalRequestInput<'a> {
    role: CreatureNifRole,
    source_data_relative_path: &'a str,
    source_owner: &'a str,
    body_variant: Option<&'a str>,
    source_byte_len: u64,
    source_blake3: &'a str,
}

#[derive(Debug)]
struct PendingArtifact {
    kind: CreatureArtifactKind,
    target_relative: String,
    bytes: Vec<u8>,
    source_inputs: BTreeSet<String>,
}

#[derive(Debug)]
struct MaterialTask {
    target_runtime_path: String,
    source_inputs: BTreeSet<String>,
    synthesized_path: Option<PathBuf>,
}

pub fn stage_creature_nif_closure(
    request: &CreatureClosureRequest,
) -> Result<CreatureClosureReceipt, CreatureClosureError> {
    let source_game = normalize_source_game(&request.source_game)?;
    let namespace = normalize_namespace(&request.target_namespace)?;
    if request.inputs.is_empty() {
        return Err(error(
            CreatureTerminalKind::InvalidRequest,
            None,
            None,
            "at least one body or skeleton NIF is required",
        ));
    }
    let published_data = request.private_staging_root.join("data");
    if published_data.exists() {
        return Err(error(
            CreatureTerminalKind::TargetCollision,
            None,
            None,
            format!(
                "private staging output already exists: {}",
                published_data.display()
            ),
        ));
    }

    let mut prepared = prepare_inputs(request, source_game, &namespace)?;
    prepared.sort_by(|left, right| left.input_key.cmp(&right.input_key));

    fs::create_dir_all(&request.private_staging_root).map_err(|io| {
        error(
            CreatureTerminalKind::Io,
            None,
            None,
            format!("create private staging root: {io}"),
        )
    })?;
    let transaction_root = request.private_staging_root.join(format!(
        ".creature-closure-{}-{}",
        std::process::id(),
        TRANSACTION_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    if transaction_root.exists() {
        return Err(error(
            CreatureTerminalKind::TargetCollision,
            None,
            None,
            "private transaction path already exists",
        ));
    }
    fs::create_dir(&transaction_root).map_err(|io| {
        error(
            CreatureTerminalKind::Io,
            None,
            None,
            format!("create private transaction: {io}"),
        )
    })?;

    let result = stage_transaction(
        request,
        source_game,
        &namespace,
        &prepared,
        &transaction_root,
    );
    match result {
        Ok(mut receipt) => {
            receipt.receipt_hash = match receipt_hash(&receipt) {
                Ok(hash) => hash,
                Err(failure) => {
                    let _ = fs::remove_dir_all(&transaction_root);
                    return Err(failure);
                }
            };
            if let Err(failure) = receipt.validate() {
                let _ = fs::remove_dir_all(&transaction_root);
                return Err(failure);
            }
            let transaction_data = transaction_root.join("data");
            fs::rename(&transaction_data, &published_data).map_err(|io| {
                let _ = fs::remove_dir_all(&transaction_root);
                error(
                    CreatureTerminalKind::Io,
                    None,
                    None,
                    format!("publish private staged closure: {io}"),
                )
            })?;
            let _ = fs::remove_dir_all(&transaction_root);
            Ok(receipt)
        }
        Err(failure) => {
            let _ = fs::remove_dir_all(&transaction_root);
            Err(failure)
        }
    }
}

fn stage_transaction(
    request: &CreatureClosureRequest,
    source_game: &str,
    namespace: &str,
    prepared: &[PreparedInput],
    transaction_root: &Path,
) -> Result<CreatureClosureReceipt, CreatureClosureError> {
    let work_root = transaction_root.join("work");
    let output_root = transaction_root.join("data");
    fs::create_dir_all(&work_root).map_err(|io| io_error("create conversion work root", io))?;
    fs::create_dir_all(&output_root).map_err(|io| io_error("create transaction data root", io))?;

    let mut artifacts = BTreeMap::<String, PendingArtifact>::new();
    let mut dispositions = Vec::with_capacity(prepared.len());
    let mut converted_sources = HashMap::<(String, String), (String, String, String)>::new();
    let mut input_aliases = Vec::<(String, String)>::new();
    let mut material_tasks = VecDeque::<MaterialTask>::new();
    let mut texture_tasks = BTreeMap::<String, (String, BTreeSet<String>)>::new();

    for input in prepared {
        let dedupe_key = (input.source_relative.clone(), input.source_blake3.clone());
        if let Some((target, fingerprint, primary_input)) = converted_sources.get(&dedupe_key) {
            dispositions.push(disposition(
                input,
                CreatureInputDispositionKind::DeduplicatedPreserveSourceRig,
                target.clone(),
                fingerprint.clone(),
            ));
            if let Some(artifact) = artifacts.get_mut(target) {
                artifact.source_inputs.insert(input.input_key.clone());
            }
            input_aliases.push((input.input_key.clone(), primary_input.clone()));
            continue;
        }

        let input_work = work_root.join(fingerprint(input.input_key.as_bytes()));
        let converted_path = input_work.join("converted.nif");
        let snapshot_path = input_work
            .join("source-data")
            .join(native_relative(&input.source_relative));
        let synthesized_material_root = input_work.join("data/materials").join(namespace);
        fs::create_dir_all(&input_work)
            .map_err(|io| input_io_error(input, "create input work root", io))?;
        if let Some(parent) = snapshot_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|io| input_io_error(input, "create source snapshot directory", io))?;
        }
        fs::write(&snapshot_path, &input.source_bytes)
            .map_err(|io| input_io_error(input, "write immutable source snapshot", io))?;
        let options = ConvertFileOptions {
            asset_prefix: Some(namespace.to_string()),
            material_namespace: Some(namespace.to_string()),
            source_material_dir: Some(request.source_data_root.clone()),
            ..ConvertFileOptions::default()
        };
        let kind = match input.role {
            CreatureNifRole::Body => SourceRigNifKind::Body,
            CreatureNifRole::Skeleton => SourceRigNifKind::Skeleton,
        };
        let stage = stage_preserve_source_rig_nif(
            kind,
            &snapshot_path,
            &converted_path,
            &input.target_relative,
            source_game,
            (source_game == "skyrimse").then_some(synthesized_material_root.as_path()),
            &options,
        )
        .map_err(|failure| {
            input_error(
                CreatureTerminalKind::ConversionFailed,
                input,
                format!("PreserveSourceRig conversion failed: {failure}"),
            )
        })?;
        if !stage.report.supported || stage.output_len == 0 {
            return Err(input_error(
                CreatureTerminalKind::ConversionFailed,
                input,
                "PreserveSourceRig conversion produced no supported FO4 NIF",
            ));
        }

        let converted = NifFile::load(converted_path.clone()).map_err(|failure| {
            input_error(
                CreatureTerminalKind::ValidationFailed,
                input,
                format!("FO4 reread failed: {failure}"),
            )
        })?;
        validate_converted_nif(&converted, input, &request.source_data_root, namespace)?;
        let bytes = fs::read(&converted_path)
            .map_err(|io| input_io_error(input, "read converted NIF", io))?;
        let output_fingerprint = fingerprint(&bytes);
        add_artifact(
            &mut artifacts,
            CreatureArtifactKind::Nif,
            &input.target_relative,
            bytes,
            &input.input_key,
            input,
        )?;

        for material in referenced_materials(&converted, input)? {
            let synthesized = resolve_case_insensitive(&input_work.join("data"), &material);
            material_tasks.push_back(MaterialTask {
                target_runtime_path: material,
                source_inputs: BTreeSet::from([input.input_key.clone()]),
                synthesized_path: synthesized,
            });
        }
        for source_material in &input.source_materials {
            material_tasks.push_back(MaterialTask {
                target_runtime_path: add_namespace(source_material, "materials", namespace)
                    .map_err(|message| {
                        input_error(CreatureTerminalKind::InvalidPath, input, message)
                    })?,
                source_inputs: BTreeSet::from([input.input_key.clone()]),
                synthesized_path: None,
            });
        }
        for target_texture in referenced_nif_textures(&converted, input, namespace)? {
            let source_texture = remove_texture_namespace(&target_texture, namespace)?;
            merge_texture_task(
                &mut texture_tasks,
                target_texture,
                source_texture,
                &input.input_key,
                input,
            )?;
        }

        converted_sources.insert(
            dedupe_key,
            (
                input.target_relative.clone(),
                output_fingerprint.clone(),
                input.input_key.clone(),
            ),
        );
        dispositions.push(disposition(
            input,
            CreatureInputDispositionKind::ConvertedPreserveSourceRig,
            input.target_relative.clone(),
            output_fingerprint,
        ));
    }

    process_materials(
        request,
        source_game,
        namespace,
        &mut artifacts,
        &mut material_tasks,
        &mut texture_tasks,
        prepared,
    )?;
    process_textures(request, &mut artifacts, texture_tasks, prepared)?;
    for (alias, primary) in input_aliases {
        for artifact in artifacts.values_mut() {
            if artifact.source_inputs.contains(&primary) {
                artifact.source_inputs.insert(alias.clone());
            }
        }
    }

    for artifact in artifacts.values() {
        let destination = output_root.join(native_relative(&artifact.target_relative));
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|io| io_error("create artifact directory", io))?;
        }
        fs::write(&destination, &artifact.bytes)
            .map_err(|io| io_error("write staged artifact", io))?;
    }
    validate_published_closure(&output_root, &artifacts)?;

    dispositions.sort_by(|left, right| left.input_key.cmp(&right.input_key));
    let artifact_receipts = artifacts
        .into_values()
        .map(|artifact| CreatureArtifactReceipt {
            kind: artifact.kind,
            target_data_relative_path: artifact.target_relative,
            byte_len: artifact.bytes.len() as u64,
            fingerprint: fingerprint(&artifact.bytes),
            source_inputs: artifact.source_inputs.into_iter().collect(),
        })
        .collect();
    Ok(CreatureClosureReceipt {
        receipt_version: CREATURE_CLOSURE_RECEIPT_VERSION,
        source_game: source_game.to_string(),
        target_game: "fo4".to_string(),
        target_namespace: namespace.to_string(),
        hash_algorithm: CREATURE_CLOSURE_HASH_ALGORITHM.to_string(),
        request_blake3: request_blake3_from_prepared(prepared),
        receipt_hash: String::new(),
        inputs: dispositions,
        artifacts: artifact_receipts,
    })
}

fn prepare_inputs(
    request: &CreatureClosureRequest,
    source_game: &str,
    namespace: &str,
) -> Result<Vec<PreparedInput>, CreatureClosureError> {
    let mut prepared = Vec::with_capacity(request.inputs.len());
    let mut target_sources = BTreeMap::<String, (String, CreatureNifRole, String)>::new();
    for raw in &request.inputs {
        let source_relative = normalize_data_relative(&raw.source_data_relative_path, "nif")
            .map_err(|message| {
                error(
                    CreatureTerminalKind::InvalidPath,
                    None,
                    Some(raw.source_data_relative_path.clone()),
                    message,
                )
            })?;
        if !source_relative.starts_with("meshes/") {
            return Err(error(
                CreatureTerminalKind::InvalidPath,
                None,
                Some(source_relative),
                "creature NIF inputs must be data-relative under meshes/",
            ));
        }
        let source_owner = raw.source_owner.trim().to_string();
        if source_owner.is_empty() {
            return Err(error(
                CreatureTerminalKind::InvalidRequest,
                None,
                Some(source_relative),
                "source_owner must not be empty",
            ));
        }
        let body_variant = raw
            .body_variant
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        if raw.role == CreatureNifRole::Skeleton && body_variant.is_some() {
            return Err(error(
                CreatureTerminalKind::InvalidRequest,
                None,
                Some(source_relative),
                "body_variant is only valid for body NIF inputs",
            ));
        }
        let source_path = resolve_case_insensitive(&request.source_data_root, &source_relative)
            .ok_or_else(|| {
                error(
                    CreatureTerminalKind::MissingNif,
                    None,
                    Some(source_relative.clone()),
                    "source NIF does not exist below the explicit Data root",
                )
            })?;
        let source_bytes = fs::read(&source_path).map_err(|io| {
            error(
                CreatureTerminalKind::Io,
                None,
                Some(source_relative.clone()),
                format!("read source NIF {}: {io}", source_path.display()),
            )
        })?;
        let source_byte_len = source_bytes.len() as u64;
        let source_blake3 = fingerprint(&source_bytes);
        let input_key = stable_input_key(
            raw.role,
            &source_relative,
            source_byte_len,
            &source_blake3,
            &source_owner,
            body_variant.as_deref(),
        );
        let source_nif =
            NifFile::from_bytes(&source_bytes, Some(source_path.clone())).map_err(|failure| {
                error(
                    CreatureTerminalKind::ValidationFailed,
                    Some(input_key.clone()),
                    Some(source_relative.clone()),
                    format!("source NIF reread failed: {failure}"),
                )
            })?;
        validate_source_blocks(source_game, &source_nif, &input_key, &source_relative)?;
        let source_materials =
            source_external_materials(&source_nif, &input_key, &source_relative)?;
        let collision = classify_collision(&source_nif, &input_key, &source_relative)?;
        let target_relative = format!(
            "meshes/{namespace}/{}",
            source_relative.trim_start_matches("meshes/")
        );
        if let Some((previous_source, previous_role, previous_blake3)) = target_sources.insert(
            target_relative.clone(),
            (source_relative.clone(), raw.role, source_blake3.clone()),
        ) && (previous_source != source_relative || previous_blake3 != source_blake3)
        {
            return Err(error(
                CreatureTerminalKind::TargetCollision,
                Some(input_key),
                Some(source_relative),
                format!(
                    "target {target_relative} also maps from {previous_source} as {previous_role:?}"
                ),
            ));
        }
        prepared.push(PreparedInput {
            input_key,
            role: raw.role,
            source_relative,
            source_bytes,
            source_byte_len,
            source_blake3,
            source_owner,
            body_variant,
            target_relative,
            collision,
            source_materials,
        });
    }
    if prepared
        .iter()
        .all(|input| input.role != CreatureNifRole::Body)
    {
        return Err(error(
            CreatureTerminalKind::InvalidRequest,
            None,
            None,
            format!("{source_game} creature closure requires at least one body NIF"),
        ));
    }
    Ok(prepared)
}

fn source_external_materials(
    nif: &NifFile,
    input_key: &str,
    source_relative: &str,
) -> Result<Vec<String>, CreatureClosureError> {
    let mut paths = BTreeSet::new();
    for block in nif.blocks.iter().filter(|block| {
        matches!(
            block.type_name.as_str(),
            "BSLightingShaderProperty" | "BSEffectShaderProperty"
        )
    }) {
        let Some(name) = string_field(block, "Name") else {
            continue;
        };
        let clean = name.trim_matches('\0').trim();
        if clean.is_empty() {
            continue;
        }
        let lower = clean.to_ascii_lowercase();
        if lower.ends_with(".bgsm") || lower.ends_with(".bgem") {
            paths.insert(
                canonical_runtime_path(clean, "materials").map_err(|message| {
                    error(
                        CreatureTerminalKind::InvalidPath,
                        Some(input_key.to_string()),
                        Some(source_relative.to_string()),
                        message,
                    )
                })?,
            );
        } else if Path::new(clean).extension().is_some() {
            return Err(error(
                CreatureTerminalKind::UnsupportedMaterialClass,
                Some(input_key.to_string()),
                Some(source_relative.to_string()),
                format!(
                    "shader block {} references unsupported material {clean}",
                    block.block_id
                ),
            ));
        }
    }
    Ok(paths.into_iter().collect())
}

fn validate_source_blocks(
    source_game: &str,
    nif: &NifFile,
    input_key: &str,
    source_relative: &str,
) -> Result<(), CreatureClosureError> {
    if source_game == "skyrimse"
        && let Some(block) = nif
            .blocks
            .iter()
            .find(|block| block.type_name == "BSDynamicTriShape" && !is_skinned_shape(block))
    {
        return Err(error(
            CreatureTerminalKind::UnsupportedBlockClass,
            Some(input_key.to_string()),
            Some(source_relative.to_string()),
            format!(
                "unsupported unskinned dynamic block {} ({})",
                block.block_id, block.type_name
            ),
        ));
    }
    Ok(())
}

fn classify_collision(
    nif: &NifFile,
    input_key: &str,
    source_relative: &str,
) -> Result<CreatureCollisionDisposition, CreatureClosureError> {
    let mut block_types = nif
        .blocks
        .iter()
        .filter(|block| block.type_name.starts_with("bhk"))
        .map(|block| block.type_name.clone())
        .collect::<Vec<_>>();
    block_types.sort();
    block_types.dedup();
    if block_types.is_empty() {
        return Ok(CreatureCollisionDisposition::None);
    }
    let articulated = block_types.iter().any(|kind| {
        kind.contains("Ragdoll") || kind.contains("Constraint") || kind == "bhkBlendCollisionObject"
    }) || nif.blocks.iter().any(is_skinned_shape);
    if articulated {
        return Ok(CreatureCollisionDisposition::ArticulatedDeferredForHkx {
            source_block_types: block_types,
        });
    }
    Err(error(
        CreatureTerminalKind::UnsupportedCollisionClass,
        Some(input_key.to_string()),
        Some(source_relative.to_string()),
        format!(
            "non-articulated legacy collision is outside this source-rig closure: {}",
            block_types.join(", ")
        ),
    ))
}

fn validate_converted_nif(
    nif: &NifFile,
    input: &PreparedInput,
    source_root: &Path,
    namespace: &str,
) -> Result<(), CreatureClosureError> {
    if nif.header.bs_version != 130 || nif.header.user_version != 12 {
        return Err(input_error(
            CreatureTerminalKind::ValidationFailed,
            input,
            format!(
                "output is not an FO4 NIF header (user={}, Bethesda={})",
                nif.header.user_version, nif.header.bs_version
            ),
        ));
    }
    let unsupported = nif
        .blocks
        .iter()
        .filter(|block| {
            matches!(
                block.type_name.as_str(),
                "NiSkinInstance" | "BSDismemberSkinInstance" | "NiSkinData" | "NiSkinPartition"
            )
        })
        .map(|block| block.type_name.clone())
        .collect::<BTreeSet<_>>();
    if !unsupported.is_empty() {
        return Err(input_error(
            CreatureTerminalKind::UnsupportedBlockClass,
            input,
            format!(
                "legacy skin blocks remain: {}",
                unsupported.into_iter().collect::<Vec<_>>().join(", ")
            ),
        ));
    }
    if let Some(block) = nif
        .blocks
        .iter()
        .find(|block| block.type_name.starts_with("bhk"))
    {
        return Err(input_error(
            CreatureTerminalKind::ValidationFailed,
            input,
            format!(
                "legacy collision block {} ({}) remains after staging",
                block.block_id, block.type_name
            ),
        ));
    }
    validate_skin_closure(nif, input)?;
    let source_root_text = source_root
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase();
    for block in &nif.blocks {
        for value in block.fields.values() {
            for string in strings(value) {
                let normalized = string
                    .trim_matches('\0')
                    .replace('\\', "/")
                    .to_ascii_lowercase();
                if (!source_root_text.is_empty() && normalized.contains(&source_root_text))
                    || Path::new(string.trim_matches('\0')).is_absolute()
                {
                    return Err(input_error(
                        CreatureTerminalKind::ValidationFailed,
                        input,
                        format!("block {} retains a source-root path", block.block_id),
                    ));
                }
            }
        }
    }
    for path in referenced_nif_textures(nif, input, namespace)? {
        validate_runtime_path(&path, "textures", namespace)
            .map_err(|message| input_error(CreatureTerminalKind::InvalidPath, input, message))?;
    }
    Ok(())
}

fn validate_skin_closure(nif: &NifFile, input: &PreparedInput) -> Result<(), CreatureClosureError> {
    for shape in nif.blocks.iter().filter(|block| is_fo4_geometry(block)) {
        let Some(skin_id) = ref_field(shape, "Skin").filter(|reference| *reference >= 0) else {
            continue;
        };
        let Some(skin) = nif.get_block(skin_id as usize) else {
            return Err(input_error(
                CreatureTerminalKind::ValidationFailed,
                input,
                format!("shape {} has unresolved skin ref {skin_id}", shape.block_id),
            ));
        };
        if skin.type_name != "BSSkin::Instance" {
            return Err(input_error(
                CreatureTerminalKind::UnsupportedBlockClass,
                input,
                format!(
                    "shape {} uses unsupported skin {}",
                    shape.block_id, skin.type_name
                ),
            ));
        }
        let Some(data_id) = ref_field(skin, "Data").filter(|reference| *reference >= 0) else {
            return Err(input_error(
                CreatureTerminalKind::ValidationFailed,
                input,
                format!("skin {} has no bone data", skin.block_id),
            ));
        };
        if nif
            .get_block(data_id as usize)
            .is_none_or(|data| data.type_name != "BSSkin::BoneData")
        {
            return Err(input_error(
                CreatureTerminalKind::ValidationFailed,
                input,
                format!(
                    "skin {} bone data ref is unresolved or non-FO4",
                    skin.block_id
                ),
            ));
        }
        let bones = ref_array(skin.get_field("Bones"));
        if bones.is_empty()
            || bones.iter().any(|bone| {
                *bone < 0
                    || nif
                        .get_block(*bone as usize)
                        .is_none_or(|block| !is_node(block))
            })
        {
            return Err(input_error(
                CreatureTerminalKind::ValidationFailed,
                input,
                format!("skin {} has an unresolved bone palette", skin.block_id),
            ));
        }
    }
    Ok(())
}

fn referenced_materials(
    nif: &NifFile,
    input: &PreparedInput,
) -> Result<Vec<String>, CreatureClosureError> {
    let mut paths = BTreeSet::new();
    for block in nif.blocks.iter().filter(|block| {
        matches!(
            block.type_name.as_str(),
            "BSLightingShaderProperty" | "BSEffectShaderProperty"
        )
    }) {
        let Some(name) = string_field(block, "Name") else {
            continue;
        };
        let clean = name.trim_matches('\0').trim();
        if clean.is_empty() {
            continue;
        }
        let lower = clean.to_ascii_lowercase();
        if !lower.ends_with(".bgsm") && !lower.ends_with(".bgem") {
            if Path::new(clean).extension().is_some() {
                return Err(input_error(
                    CreatureTerminalKind::UnsupportedMaterialClass,
                    input,
                    format!(
                        "shader block {} references unsupported material {clean}",
                        block.block_id
                    ),
                ));
            }
            continue;
        }
        paths.insert(
            canonical_runtime_path(clean, "materials").map_err(|message| {
                input_error(CreatureTerminalKind::InvalidPath, input, message)
            })?,
        );
    }
    Ok(paths.into_iter().collect())
}

fn referenced_nif_textures(
    nif: &NifFile,
    input: &PreparedInput,
    namespace: &str,
) -> Result<Vec<String>, CreatureClosureError> {
    let mut paths = BTreeSet::new();
    for block in nif
        .blocks
        .iter()
        .filter(|block| block.type_name == "BSShaderTextureSet")
    {
        let Some(NifValue::Array(textures)) = block.get_field("Textures") else {
            continue;
        };
        for value in textures {
            let NifValue::String(path) = value else {
                continue;
            };
            let clean = path.trim_matches('\0').trim();
            if clean.is_empty() {
                continue;
            }
            if !clean.to_ascii_lowercase().ends_with(".dds") {
                return Err(input_error(
                    CreatureTerminalKind::UnsupportedMaterialClass,
                    input,
                    format!(
                        "texture set block {} references unsupported texture {clean}",
                        block.block_id
                    ),
                ));
            }
            let canonical = canonical_runtime_path(clean, "textures").map_err(|message| {
                input_error(CreatureTerminalKind::InvalidPath, input, message)
            })?;
            validate_runtime_path(&canonical, "textures", namespace).map_err(|message| {
                input_error(CreatureTerminalKind::InvalidPath, input, message)
            })?;
            paths.insert(canonical);
        }
    }
    Ok(paths.into_iter().collect())
}

fn process_materials(
    request: &CreatureClosureRequest,
    source_game: &str,
    namespace: &str,
    artifacts: &mut BTreeMap<String, PendingArtifact>,
    tasks: &mut VecDeque<MaterialTask>,
    texture_tasks: &mut BTreeMap<String, (String, BTreeSet<String>)>,
    inputs: &[PreparedInput],
) -> Result<(), CreatureClosureError> {
    let mut propagated = BTreeMap::<(String, String), BTreeSet<String>>::new();
    while let Some(task) = tasks.pop_front() {
        let target_runtime = canonical_runtime_path(&task.target_runtime_path, "materials")
            .map_err(|message| {
                task_error(
                    CreatureTerminalKind::InvalidPath,
                    &task.source_inputs,
                    inputs,
                    message,
                )
            })?;
        validate_runtime_path(&target_runtime, "materials", namespace).map_err(|message| {
            task_error(
                CreatureTerminalKind::InvalidPath,
                &task.source_inputs,
                inputs,
                message,
            )
        })?;
        let source_runtime = remove_material_namespace(&target_runtime, namespace)?;
        let (source_path, synthesized) = match task.synthesized_path.filter(|path| path.is_file()) {
            Some(path) => (path, true),
            None => {
                let path = resolve_case_insensitive(&request.source_data_root, &source_runtime)
                    .ok_or_else(|| {
                        task_error(
                            CreatureTerminalKind::MissingMaterial,
                            &task.source_inputs,
                            inputs,
                            format!("material dependency is missing: {source_runtime}"),
                        )
                    })?;
                (path, false)
            }
        };
        let bytes = fs::read(&source_path).map_err(|io| {
            task_error(
                CreatureTerminalKind::Io,
                &task.source_inputs,
                inputs,
                format!("read material {}: {io}", source_path.display()),
            )
        })?;
        let extension = target_runtime.rsplit('.').next().unwrap_or_default();
        let (output, textures, parent_material) = match extension {
            "bgsm" => {
                rewrite_bgsm(&bytes, source_game, synthesized, namespace).map_err(|message| {
                    task_error(
                        CreatureTerminalKind::InvalidMaterial,
                        &task.source_inputs,
                        inputs,
                        message,
                    )
                })?
            }
            "bgem" => {
                rewrite_bgem(&bytes, source_game, synthesized, namespace).map_err(|message| {
                    task_error(
                        CreatureTerminalKind::InvalidMaterial,
                        &task.source_inputs,
                        inputs,
                        message,
                    )
                })?
            }
            _ => {
                return Err(task_error(
                    CreatureTerminalKind::UnsupportedMaterialClass,
                    &task.source_inputs,
                    inputs,
                    format!("unsupported material dependency {target_runtime}"),
                ));
            }
        };
        let kind = if extension == "bgsm" {
            CreatureArtifactKind::Bgsm
        } else {
            CreatureArtifactKind::Bgem
        };
        let first_input = input_for_task(&task.source_inputs, inputs);
        for input_key in &task.source_inputs {
            add_artifact(
                artifacts,
                kind,
                &target_runtime,
                output.clone(),
                input_key,
                first_input,
            )?;
        }
        let propagation_key = (target_runtime.clone(), fingerprint(&output));
        let already_propagated = propagated.entry(propagation_key).or_default();
        let new_inputs = task
            .source_inputs
            .difference(already_propagated)
            .cloned()
            .collect::<BTreeSet<_>>();
        already_propagated.extend(new_inputs.clone());
        if new_inputs.is_empty() {
            continue;
        }
        for (target_texture, source_texture) in textures {
            for input_key in &new_inputs {
                merge_texture_task(
                    texture_tasks,
                    target_texture.clone(),
                    source_texture.clone(),
                    input_key,
                    first_input,
                )?;
            }
        }
        if let Some(parent) = parent_material {
            tasks.push_back(MaterialTask {
                target_runtime_path: parent,
                source_inputs: new_inputs,
                synthesized_path: None,
            });
        }
    }
    Ok(())
}

fn rewrite_bgsm(
    bytes: &[u8],
    source_game: &str,
    synthesized: bool,
    namespace: &str,
) -> Result<(Vec<u8>, Vec<(String, String)>, Option<String>), String> {
    let mut material = materials_native::bgsm::parse(bytes).map_err(|error| error.to_string())?;
    if !synthesized {
        let source = materials_native::convert::Game::from_str(source_game)
            .ok_or_else(|| format!("unsupported material source game {source_game}"))?;
        material = materials_native::convert::downgrade_bgsm(
            material,
            "closure.bgsm",
            source,
            materials_native::convert::Game::Fo4,
        );
    }
    let mut textures = Vec::new();
    rewrite_texture_string(
        &mut material.DiffuseTexture,
        namespace,
        synthesized,
        &mut textures,
    )?;
    rewrite_texture_string(
        &mut material.NormalTexture,
        namespace,
        synthesized,
        &mut textures,
    )?;
    rewrite_texture_string(
        &mut material.SmoothSpecTexture,
        namespace,
        synthesized,
        &mut textures,
    )?;
    rewrite_texture_string(
        &mut material.GreyscaleTexture,
        namespace,
        synthesized,
        &mut textures,
    )?;
    rewrite_optional_texture(
        &mut material.EnvmapTexture,
        namespace,
        synthesized,
        &mut textures,
    )?;
    rewrite_optional_texture(
        &mut material.GlowTexture,
        namespace,
        synthesized,
        &mut textures,
    )?;
    rewrite_optional_texture(
        &mut material.InnerLayerTexture,
        namespace,
        synthesized,
        &mut textures,
    )?;
    rewrite_optional_texture(
        &mut material.WrinklesTexture,
        namespace,
        synthesized,
        &mut textures,
    )?;
    rewrite_optional_texture(
        &mut material.DisplacementTexture,
        namespace,
        synthesized,
        &mut textures,
    )?;
    rewrite_optional_texture(
        &mut material.SpecularTexture,
        namespace,
        synthesized,
        &mut textures,
    )?;
    rewrite_optional_texture(
        &mut material.LightingTexture,
        namespace,
        synthesized,
        &mut textures,
    )?;
    rewrite_optional_texture(
        &mut material.FlowTexture,
        namespace,
        synthesized,
        &mut textures,
    )?;
    rewrite_optional_texture(
        &mut material.DistanceFieldAlphaTexture,
        namespace,
        synthesized,
        &mut textures,
    )?;
    let parent = rewrite_parent_material(&mut material.RootMaterialPath, namespace)?;
    material.CastShadows = true;
    Ok((materials_native::bgsm::write(&material), textures, parent))
}

fn rewrite_bgem(
    bytes: &[u8],
    source_game: &str,
    synthesized: bool,
    namespace: &str,
) -> Result<(Vec<u8>, Vec<(String, String)>, Option<String>), String> {
    let mut material = materials_native::bgem::parse(bytes).map_err(|error| error.to_string())?;
    if !synthesized {
        let source = materials_native::convert::Game::from_str(source_game)
            .ok_or_else(|| format!("unsupported material source game {source_game}"))?;
        material = materials_native::convert::downgrade_bgem(
            material,
            "closure.bgem",
            source,
            materials_native::convert::Game::Fo4,
        );
    }
    let mut textures = Vec::new();
    rewrite_texture_string(
        &mut material.BaseTexture,
        namespace,
        synthesized,
        &mut textures,
    )?;
    rewrite_texture_string(
        &mut material.GrayscaleTexture,
        namespace,
        synthesized,
        &mut textures,
    )?;
    rewrite_texture_string(
        &mut material.EnvmapTexture,
        namespace,
        synthesized,
        &mut textures,
    )?;
    rewrite_texture_string(
        &mut material.NormalTexture,
        namespace,
        synthesized,
        &mut textures,
    )?;
    rewrite_texture_string(
        &mut material.EnvmapMaskTexture,
        namespace,
        synthesized,
        &mut textures,
    )?;
    rewrite_optional_texture(
        &mut material.SpecularTexture,
        namespace,
        synthesized,
        &mut textures,
    )?;
    rewrite_optional_texture(
        &mut material.LightingTexture,
        namespace,
        synthesized,
        &mut textures,
    )?;
    rewrite_optional_texture(
        &mut material.GlowTexture,
        namespace,
        synthesized,
        &mut textures,
    )?;
    rewrite_optional_texture(
        &mut material.GlassRoughnessScratch,
        namespace,
        synthesized,
        &mut textures,
    )?;
    rewrite_optional_texture(
        &mut material.GlassDirtOverlay,
        namespace,
        synthesized,
        &mut textures,
    )?;
    Ok((materials_native::bgem::write(&material), textures, None))
}

fn rewrite_texture_string(
    value: &mut String,
    namespace: &str,
    already_namespaced: bool,
    textures: &mut Vec<(String, String)>,
) -> Result<(), String> {
    let clean = value.trim_matches('\0').trim();
    if clean.is_empty() {
        *value = String::new();
        return Ok(());
    }
    if !clean.to_ascii_lowercase().ends_with(".dds") {
        return Err(format!("unsupported texture class {clean}"));
    }
    let mut source = canonical_runtime_path(clean, "textures")?;
    if already_namespaced {
        let prefix = format!("textures/{namespace}/");
        if source.starts_with(&prefix) {
            source = remove_namespace(&source, "textures", namespace)?;
        }
    }
    let target = add_namespace(&source, "textures", namespace)?;
    *value = target.trim_start_matches("textures/").replace('/', "\\");
    textures.push((target, source));
    Ok(())
}

fn rewrite_optional_texture(
    value: &mut Option<String>,
    namespace: &str,
    already_namespaced: bool,
    textures: &mut Vec<(String, String)>,
) -> Result<(), String> {
    if let Some(value) = value {
        rewrite_texture_string(value, namespace, already_namespaced, textures)?;
    }
    Ok(())
}

fn rewrite_parent_material(value: &mut String, namespace: &str) -> Result<Option<String>, String> {
    let clean = value.trim_matches('\0').trim();
    if clean.is_empty() {
        *value = String::new();
        return Ok(None);
    }
    if !clean.to_ascii_lowercase().ends_with(".bgsm") {
        return Err(format!("unsupported parent material class {clean}"));
    }
    let source = canonical_runtime_path(clean, "materials")?;
    let target = add_namespace(&source, "materials", namespace)?;
    *value = target.replace('/', "\\");
    Ok(Some(target))
}

fn process_textures(
    request: &CreatureClosureRequest,
    artifacts: &mut BTreeMap<String, PendingArtifact>,
    tasks: BTreeMap<String, (String, BTreeSet<String>)>,
    inputs: &[PreparedInput],
) -> Result<(), CreatureClosureError> {
    for (target, (source, source_inputs)) in tasks {
        if let Some(bytes) =
            synthesize_skyrim_fo4_texture(request, &source, &source_inputs, inputs)?
        {
            let first_input = input_for_task(&source_inputs, inputs);
            for input_key in source_inputs {
                add_artifact(
                    artifacts,
                    CreatureArtifactKind::Dds,
                    &target,
                    bytes.clone(),
                    &input_key,
                    first_input,
                )?;
            }
            continue;
        }
        let source_path = match resolve_case_insensitive(&request.source_data_root, &source) {
            Some(path) => path,
            None => {
                let fallback = request
                    .texture_fallbacks
                    .iter()
                    .find_map(|(missing, fallback)| {
                        canonical_runtime_path(missing, "textures")
                            .ok()
                            .filter(|missing| missing == &source)
                            .map(|_| fallback)
                    })
                    .ok_or_else(|| {
                        task_error(
                            CreatureTerminalKind::MissingTexture,
                            &source_inputs,
                            inputs,
                            format!("texture dependency is missing: {source}"),
                        )
                    })?;
                let fallback = canonical_runtime_path(fallback, "textures").map_err(|message| {
                    task_error(
                        CreatureTerminalKind::InvalidPath,
                        &source_inputs,
                        inputs,
                        format!("texture fallback for {source} is invalid: {message}"),
                    )
                })?;
                resolve_case_insensitive(&request.source_data_root, &fallback).ok_or_else(|| {
                    task_error(
                        CreatureTerminalKind::MissingTexture,
                        &source_inputs,
                        inputs,
                        format!("texture dependency {source} and fallback {fallback} are missing"),
                    )
                })?
            }
        };
        let bytes = fs::read(&source_path).map_err(|io| {
            task_error(
                CreatureTerminalKind::Io,
                &source_inputs,
                inputs,
                format!("read texture {}: {io}", source_path.display()),
            )
        })?;
        validate_dds(&bytes).map_err(|message| {
            task_error(
                CreatureTerminalKind::InvalidTexture,
                &source_inputs,
                inputs,
                format!("{source}: {message}"),
            )
        })?;
        let first_input = input_for_task(&source_inputs, inputs);
        for input_key in source_inputs {
            add_artifact(
                artifacts,
                CreatureArtifactKind::Dds,
                &target,
                bytes.clone(),
                &input_key,
                first_input,
            )?;
        }
    }
    Ok(())
}

fn synthesize_skyrim_fo4_texture(
    request: &CreatureClosureRequest,
    source: &str,
    source_inputs: &BTreeSet<String>,
    inputs: &[PreparedInput],
) -> Result<Option<Vec<u8>>, CreatureClosureError> {
    if !matches!(request.source_game.as_str(), "skyrim" | "skyrimse") {
        return Ok(None);
    }
    let source_lower = source.to_ascii_lowercase();
    let (normal_source, output_specgloss) = if source_lower.ends_with("_n.dds") {
        (source.to_string(), false)
    } else if source_lower.ends_with("_s.dds") {
        (
            format!("{}_n.dds", &source[..source.len() - "_s.dds".len()]),
            true,
        )
    } else {
        return Ok(None);
    };
    let Some(normal_path) = resolve_case_insensitive(&request.source_data_root, &normal_source)
    else {
        return Ok(None);
    };
    let normal = directxtex_native::read_dds_float_rgba_image(&normal_path).map_err(|message| {
        task_error(
            CreatureTerminalKind::InvalidTexture,
            source_inputs,
            inputs,
            format!("decode Skyrim normal {normal_source}: {message}"),
        )
    })?;
    let envmask_source = format!(
        "{}_em.dds",
        &normal_source[..normal_source.len() - "_n.dds".len()]
    );
    let envmask = resolve_case_insensitive(&request.source_data_root, &envmask_source)
        .map(|path| directxtex_native::read_dds_float_rgba_image(&path))
        .transpose()
        .map_err(|message| {
            task_error(
                CreatureTerminalKind::InvalidTexture,
                source_inputs,
                inputs,
                format!("decode Skyrim environment mask {envmask_source}: {message}"),
            )
        })?;
    let normal_bytes = f32_slice_as_bytes(&normal.rgba);
    let envmask_bytes = envmask
        .as_ref()
        .map(|image| f32_slice_as_bytes(&image.rgba));
    let converted = gamebryo_normal_envmask_to_fo4_specgloss_buffers(
        &normal_bytes,
        envmask_bytes.as_deref(),
        normal.width as usize,
        normal.height as usize,
        envmask.as_ref().map_or(0, |image| image.width as usize),
        envmask.as_ref().map_or(0, |image| image.height as usize),
        GamebryoSpecParams::default(),
    )
    .map_err(|message| {
        task_error(
            CreatureTerminalKind::ConversionFailed,
            source_inputs,
            inputs,
            format!("convert Skyrim normal {normal_source}: {message}"),
        )
    })?;
    let rgba = if output_specgloss {
        converted.specgloss
    } else {
        converted.normal
    }
    .into_iter()
    .map(|value| (value.clamp(0.0, 1.0) * 255.0).round() as u8)
    .collect::<Vec<_>>();
    let chain = directxtex_native::rgba8_box_mip_chain(normal.width, normal.height, &rgba)
        .map_err(|message| {
            task_error(
                CreatureTerminalKind::ConversionFailed,
                source_inputs,
                inputs,
                format!("build FO4 texture mips for {source}: {message}"),
            )
        })?;
    directxtex_native::encode_dds_from_rgba8_chain(&chain, "BC5_UNORM", true, None)
        .map(Some)
        .map_err(|message| {
            task_error(
                CreatureTerminalKind::ConversionFailed,
                source_inputs,
                inputs,
                format!("encode FO4 texture {source}: {message}"),
            )
        })
}

fn f32_slice_as_bytes(values: &[f32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_ne_bytes())
        .collect()
}

fn validate_dds(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() < 128 || bytes.get(..4) != Some(b"DDS ") {
        return Err("not a complete DDS header".to_string());
    }
    if read_u32(bytes, 4) != 124 || read_u32(bytes, 76) != 32 {
        return Err("invalid DDS header sizes".to_string());
    }
    let height = read_u32(bytes, 12);
    let width = read_u32(bytes, 16);
    if width == 0 || height == 0 {
        return Err("DDS dimensions are zero".to_string());
    }
    let pixel_flags = read_u32(bytes, 80);
    let fourcc = bytes
        .get(84..88)
        .ok_or_else(|| "missing DDS pixel format".to_string())?;
    let is_dx10 = fourcc == b"DX10";
    let header_len = if is_dx10 { 148 } else { 128 };
    if bytes.len() < header_len {
        return Err("truncated DDS DX10 header".to_string());
    }
    let layout = if pixel_flags & 0x4 != 0 {
        match fourcc {
            b"DXT1" | b"ATI1" | b"BC4U" | b"BC4S" => DdsLayout::Block(8),
            b"DXT2" | b"DXT3" | b"DXT4" | b"DXT5" | b"ATI2" | b"BC5U" | b"BC5S" => {
                DdsLayout::Block(16)
            }
            b"DX10" => dxgi_layout(read_u32(bytes, 128))?,
            _ => {
                return Err(format!(
                    "unsupported DDS FOURCC {}",
                    String::from_utf8_lossy(fourcc)
                ));
            }
        }
    } else {
        let bits = read_u32(bytes, 88);
        if pixel_flags & (0x2 | 0x40 | 0x20_000) == 0 || bits == 0 || bits > 128 || bits % 8 != 0 {
            return Err("unsupported DDS pixel layout".to_string());
        }
        DdsLayout::Linear(u64::from(bits / 8))
    };

    let caps2 = read_u32(bytes, 112);
    let legacy_cube = caps2 & 0x200 != 0;
    let legacy_volume = caps2 & 0x20_0000 != 0;
    let (surface_count, volume_depth) = if is_dx10 {
        let dimension = read_u32(bytes, 132);
        let cube = read_u32(bytes, 136) & 0x4 != 0;
        let array_size = read_u32(bytes, 140);
        if array_size == 0 {
            return Err("DDS DX10 array size is zero".to_string());
        }
        match dimension {
            2 => {
                if height != 1 || cube {
                    return Err("invalid DDS DX10 1D dimensions".to_string());
                }
                (u64::from(array_size), None)
            }
            3 => {
                let surfaces = u64::from(array_size)
                    .checked_mul(if cube { 6 } else { 1 })
                    .ok_or_else(|| "DDS surface count overflow".to_string())?;
                (surfaces, None)
            }
            4 if array_size == 1 && !cube => (1, Some(read_u32(bytes, 24).max(1))),
            _ => return Err("unsupported DDS DX10 resource layout".to_string()),
        }
    } else if legacy_volume {
        if legacy_cube {
            return Err("DDS cannot be both a cubemap and volume".to_string());
        }
        (1, Some(read_u32(bytes, 24).max(1)))
    } else if legacy_cube {
        if caps2 & 0xfc00 != 0xfc00 {
            return Err("DDS cubemap does not declare all six faces".to_string());
        }
        (6, None)
    } else {
        (1, None)
    };

    let declared = read_u32(bytes, 28).max(1);
    let full = 32
        - width
            .max(height)
            .max(volume_depth.unwrap_or(1))
            .leading_zeros();
    if declared > full {
        return Err(format!(
            "DDS mip chain exceeds the maximum for its dimensions: declared {declared}, maximum {full}"
        ));
    }
    let mut cursor = u64::try_from(header_len).expect("DDS header length fits u64");
    for _surface in 0..surface_count {
        for mip in 0..declared {
            let mip_width = u64::from((width >> mip).max(1));
            let mip_height = u64::from((height >> mip).max(1));
            let mip_depth = u64::from(volume_depth.map(|depth| (depth >> mip).max(1)).unwrap_or(1));
            let slice_len = match layout {
                DdsLayout::Block(block_len) => mip_width
                    .div_ceil(4)
                    .checked_mul(mip_height.div_ceil(4))
                    .and_then(|value| value.checked_mul(block_len)),
                DdsLayout::Linear(bytes_per_pixel) => mip_width
                    .checked_mul(mip_height)
                    .and_then(|value| value.checked_mul(bytes_per_pixel)),
            }
            .and_then(|value| value.checked_mul(mip_depth))
            .ok_or_else(|| "DDS mip payload length overflow".to_string())?;
            cursor = cursor
                .checked_add(slice_len)
                .ok_or_else(|| "DDS payload length overflow".to_string())?;
            if cursor > bytes.len() as u64 {
                return Err(format!(
                    "DDS mip payload is truncated: needs {cursor} bytes, has {}",
                    bytes.len()
                ));
            }
        }
    }
    if cursor != bytes.len() as u64 {
        return Err(format!(
            "DDS payload length mismatch: declared mip ranges end at {cursor}, file has {} bytes",
            bytes.len()
        ));
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum DdsLayout {
    Block(u64),
    Linear(u64),
}

fn dxgi_layout(format: u32) -> Result<DdsLayout, String> {
    let layout = match format {
        70..=72 | 79..=81 => DdsLayout::Block(8),
        73..=78 | 82..=84 | 94..=99 => DdsLayout::Block(16),
        2..=4 => DdsLayout::Linear(16),
        6..=8 => DdsLayout::Linear(12),
        10..=23 => DdsLayout::Linear(8),
        24..=47 | 67 | 87..=93 => DdsLayout::Linear(4),
        48..=59 | 85..=86 => DdsLayout::Linear(2),
        60..=65 => DdsLayout::Linear(1),
        _ => return Err(format!("unsupported DDS DXGI format {format}")),
    };
    Ok(layout)
}

fn validate_published_closure(
    output_root: &Path,
    artifacts: &BTreeMap<String, PendingArtifact>,
) -> Result<(), CreatureClosureError> {
    for (path, artifact) in artifacts {
        let staged = output_root.join(native_relative(path));
        let bytes = fs::read(&staged).map_err(|io| io_error("reread staged artifact", io))?;
        if bytes != artifact.bytes {
            return Err(error(
                CreatureTerminalKind::ValidationFailed,
                None,
                Some(path.clone()),
                "staged artifact fingerprint changed during publication",
            ));
        }
        match artifact.kind {
            CreatureArtifactKind::Nif => {
                NifFile::load(staged).map_err(|failure| {
                    error(
                        CreatureTerminalKind::ValidationFailed,
                        None,
                        Some(path.clone()),
                        format!("staged FO4 NIF reread failed: {failure}"),
                    )
                })?;
            }
            CreatureArtifactKind::Bgsm => {
                materials_native::bgsm::parse(&bytes).map_err(|failure| {
                    error(
                        CreatureTerminalKind::InvalidMaterial,
                        None,
                        Some(path.clone()),
                        format!("staged BGSM reread failed: {failure}"),
                    )
                })?;
            }
            CreatureArtifactKind::Bgem => {
                materials_native::bgem::parse(&bytes).map_err(|failure| {
                    error(
                        CreatureTerminalKind::InvalidMaterial,
                        None,
                        Some(path.clone()),
                        format!("staged BGEM reread failed: {failure}"),
                    )
                })?;
            }
            CreatureArtifactKind::Dds => validate_dds(&bytes).map_err(|message| {
                error(
                    CreatureTerminalKind::InvalidTexture,
                    None,
                    Some(path.clone()),
                    message,
                )
            })?,
        }
    }
    Ok(())
}

fn add_artifact(
    artifacts: &mut BTreeMap<String, PendingArtifact>,
    kind: CreatureArtifactKind,
    target: &str,
    bytes: Vec<u8>,
    source_input: &str,
    input: &PreparedInput,
) -> Result<(), CreatureClosureError> {
    let target = normalize_data_relative(
        target,
        match kind {
            CreatureArtifactKind::Nif => "nif",
            CreatureArtifactKind::Bgsm => "bgsm",
            CreatureArtifactKind::Bgem => "bgem",
            CreatureArtifactKind::Dds => "dds",
        },
    )
    .map_err(|message| input_error(CreatureTerminalKind::InvalidPath, input, message))?;
    if let Some(existing) = artifacts.get_mut(&target) {
        if existing.kind != kind || existing.bytes != bytes {
            return Err(input_error(
                CreatureTerminalKind::TargetCollision,
                input,
                format!("different outputs map to {target}"),
            ));
        }
        existing.source_inputs.insert(source_input.to_string());
        return Ok(());
    }
    artifacts.insert(
        target.clone(),
        PendingArtifact {
            kind,
            target_relative: target,
            bytes,
            source_inputs: BTreeSet::from([source_input.to_string()]),
        },
    );
    Ok(())
}

fn merge_texture_task(
    tasks: &mut BTreeMap<String, (String, BTreeSet<String>)>,
    target: String,
    source: String,
    input_key: &str,
    input: &PreparedInput,
) -> Result<(), CreatureClosureError> {
    if let Some((existing_source, owners)) = tasks.get_mut(&target) {
        if existing_source != &source {
            return Err(input_error(
                CreatureTerminalKind::TargetCollision,
                input,
                format!("textures {existing_source} and {source} both map to {target}"),
            ));
        }
        owners.insert(input_key.to_string());
    } else {
        tasks.insert(target, (source, BTreeSet::from([input_key.to_string()])));
    }
    Ok(())
}

fn disposition(
    input: &PreparedInput,
    disposition: CreatureInputDispositionKind,
    target: String,
    output_fingerprint: String,
) -> CreatureInputDisposition {
    CreatureInputDisposition {
        input_key: input.input_key.clone(),
        role: input.role,
        source_data_relative_path: input.source_relative.clone(),
        source_byte_len: input.source_byte_len,
        source_blake3: input.source_blake3.clone(),
        source_owner: input.source_owner.clone(),
        body_variant: input.body_variant.clone(),
        disposition,
        collision: input.collision.clone(),
        target_data_relative_path: target,
        output_fingerprint,
    }
}

fn normalize_source_game(game: &str) -> Result<&'static str, CreatureClosureError> {
    match game.trim().to_ascii_lowercase().as_str() {
        "skyrim" | "skyrimse" | "sse" => Ok("skyrimse"),
        "fnv" | "falloutnv" => Ok("fnv"),
        "fo3" | "fallout3" => Ok("fo3"),
        _ => Err(error(
            CreatureTerminalKind::InvalidRequest,
            None,
            None,
            format!("unsupported creature source game {game:?}"),
        )),
    }
}

fn normalize_namespace(namespace: &str) -> Result<String, CreatureClosureError> {
    let raw = namespace.trim().replace('\\', "/");
    let mut parts = Vec::new();
    for part in raw.split('/') {
        if part.is_empty()
            || part == "."
            || part == ".."
            || !part.chars().all(|character| {
                character.is_ascii_alphanumeric() || character == '_' || character == '-'
            })
        {
            return Err(error(
                CreatureTerminalKind::InvalidRequest,
                None,
                None,
                "target_namespace must be a safe relative path of ASCII letters, digits, '_', or '-'",
            ));
        }
        parts.push(part.to_ascii_lowercase());
    }
    if parts.is_empty() {
        return Err(error(
            CreatureTerminalKind::InvalidRequest,
            None,
            None,
            "target_namespace must be a safe relative path of ASCII letters, digits, '_', or '-'",
        ));
    }
    Ok(parts.join("/"))
}

fn normalize_data_relative(path: &str, extension: &str) -> Result<String, String> {
    let raw = path.trim().trim_matches('\0').replace('\\', "/");
    if raw.is_empty() || Path::new(&raw).is_absolute() || raw.starts_with('/') {
        return Err(format!("path must be Data-relative: {path:?}"));
    }
    let mut parts = Vec::new();
    for part in raw.split('/') {
        let part = part.trim();
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." || part.contains(':') {
            return Err(format!("path escapes the Data root: {path:?}"));
        }
        parts.push(part.to_ascii_lowercase());
    }
    let normalized = parts.join("/");
    if !normalized.ends_with(&format!(".{extension}")) {
        return Err(format!("path is not a .{extension} asset: {path:?}"));
    }
    Ok(normalized)
}

fn canonical_runtime_path(path: &str, root: &str) -> Result<String, String> {
    let raw = path.trim().trim_matches('\0').replace('\\', "/");
    if raw.is_empty() || Path::new(&raw).is_absolute() || raw.starts_with('/') {
        return Err(format!("runtime asset path is not relative: {path:?}"));
    }
    let mut parts = raw
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .map(|part| part.to_ascii_lowercase())
        .collect::<Vec<_>>();
    if parts.iter().any(|part| part == ".." || part.contains(':')) {
        return Err(format!("runtime asset path escapes Data: {path:?}"));
    }
    if let Some(index) = parts.iter().rposition(|part| part == root) {
        parts = parts[index..].to_vec();
    } else {
        parts.insert(0, root.to_string());
    }
    Ok(parts.join("/"))
}

fn validate_runtime_path(path: &str, root: &str, namespace: &str) -> Result<(), String> {
    let expected = format!("{root}/{namespace}/");
    if !path.starts_with(&expected) || path.contains('\\') || path.chars().any(char::is_control) {
        return Err(format!(
            "runtime path is not canonical in {expected}: {path}"
        ));
    }
    Ok(())
}

fn add_namespace(path: &str, root: &str, namespace: &str) -> Result<String, String> {
    let canonical = canonical_runtime_path(path, root)?;
    let tail = canonical.trim_start_matches(&format!("{root}/"));
    if tail == namespace || tail.starts_with(&format!("{namespace}/")) {
        Ok(canonical)
    } else {
        Ok(format!("{root}/{namespace}/{tail}"))
    }
}

fn remove_texture_namespace(path: &str, namespace: &str) -> Result<String, CreatureClosureError> {
    remove_namespace(path, "textures", namespace).map_err(|message| {
        error(
            CreatureTerminalKind::InvalidPath,
            None,
            Some(path.to_string()),
            message,
        )
    })
}

fn remove_material_namespace(path: &str, namespace: &str) -> Result<String, CreatureClosureError> {
    remove_namespace(path, "materials", namespace).map_err(|message| {
        error(
            CreatureTerminalKind::InvalidPath,
            None,
            Some(path.to_string()),
            message,
        )
    })
}

fn remove_namespace(path: &str, root: &str, namespace: &str) -> Result<String, String> {
    let canonical = canonical_runtime_path(path, root)?;
    let prefix = format!("{root}/{namespace}/");
    canonical
        .strip_prefix(&prefix)
        .map(|tail| format!("{root}/{tail}"))
        .ok_or_else(|| format!("asset is outside target namespace {prefix}: {path}"))
}

fn resolve_case_insensitive(root: &Path, relative: &str) -> Option<PathBuf> {
    let mut current = root.to_path_buf();
    for component in Path::new(relative).components() {
        let Component::Normal(name) = component else {
            return None;
        };
        let direct = current.join(name);
        if direct.exists() {
            current = direct;
            continue;
        }
        let wanted = name.to_string_lossy();
        let entry = fs::read_dir(&current).ok()?.flatten().find(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .eq_ignore_ascii_case(&wanted)
        })?;
        current = entry.path();
    }
    current.is_file().then_some(current)
}

fn native_relative(path: &str) -> PathBuf {
    path.split('/').collect()
}

fn stable_input_key(
    role: CreatureNifRole,
    source_path: &str,
    source_byte_len: u64,
    source_blake3: &str,
    owner: &str,
    variant: Option<&str>,
) -> String {
    fingerprint(
        &serde_json::to_vec(&CanonicalRequestInput {
            role,
            source_data_relative_path: source_path,
            source_owner: owner.trim(),
            body_variant: variant,
            source_byte_len,
            source_blake3,
        })
        .expect("canonical input identity always serializes"),
    )
}

fn request_blake3_from_prepared(inputs: &[PreparedInput]) -> String {
    request_blake3(
        inputs
            .iter()
            .map(|input| CanonicalRequestInput {
                role: input.role,
                source_data_relative_path: &input.source_relative,
                source_owner: &input.source_owner,
                body_variant: input.body_variant.as_deref(),
                source_byte_len: input.source_byte_len,
                source_blake3: &input.source_blake3,
            })
            .collect(),
    )
}

fn request_blake3_from_dispositions(inputs: &[CreatureInputDisposition]) -> String {
    request_blake3(
        inputs
            .iter()
            .map(|input| CanonicalRequestInput {
                role: input.role,
                source_data_relative_path: &input.source_data_relative_path,
                source_owner: &input.source_owner,
                body_variant: input.body_variant.as_deref(),
                source_byte_len: input.source_byte_len,
                source_blake3: &input.source_blake3,
            })
            .collect(),
    )
}

fn request_blake3(mut inputs: Vec<CanonicalRequestInput<'_>>) -> String {
    inputs.sort();
    fingerprint(
        &serde_json::to_vec(&inputs).expect("canonical request identities always serialize"),
    )
}

fn receipt_hash(receipt: &CreatureClosureReceipt) -> Result<String, CreatureClosureError> {
    let mut payload = receipt.clone();
    payload.receipt_hash.clear();
    let json = serde_json::to_vec(&payload).map_err(|failure| {
        error(
            CreatureTerminalKind::ValidationFailed,
            None,
            None,
            format!("serialize receipt: {failure}"),
        )
    })?;
    Ok(fingerprint(&json))
}

fn validate_receipt(receipt: &CreatureClosureReceipt) -> Result<(), CreatureClosureError> {
    let invalid =
        |message: String| error(CreatureTerminalKind::InvalidReceipt, None, None, message);
    if receipt.receipt_version != CREATURE_CLOSURE_RECEIPT_VERSION {
        return Err(invalid(format!(
            "unsupported receipt version {}",
            receipt.receipt_version
        )));
    }
    if receipt.target_game != "fo4" {
        return Err(invalid(format!(
            "receipt target game must be fo4, got {:?}",
            receipt.target_game
        )));
    }
    let source_game = normalize_source_game(&receipt.source_game).map_err(|_| {
        invalid(format!(
            "invalid receipt source game {:?}",
            receipt.source_game
        ))
    })?;
    if source_game != receipt.source_game {
        return Err(invalid(
            "receipt source game is not canonical lowercase".to_string(),
        ));
    }
    let namespace = normalize_namespace(&receipt.target_namespace)
        .map_err(|_| invalid("invalid receipt target namespace".to_string()))?;
    if namespace != receipt.target_namespace {
        return Err(invalid(
            "receipt target namespace is not canonical lowercase".to_string(),
        ));
    }
    if receipt.hash_algorithm != CREATURE_CLOSURE_HASH_ALGORITHM {
        return Err(invalid(format!(
            "unsupported receipt hash algorithm {:?}",
            receipt.hash_algorithm
        )));
    }
    if !is_lower_hex_64(&receipt.receipt_hash) {
        return Err(invalid(
            "receipt_hash must be lowercase 64-hex BLAKE3".to_string(),
        ));
    }
    if !is_lower_hex_64(&receipt.request_blake3) {
        return Err(invalid(
            "request_blake3 must be lowercase 64-hex BLAKE3".to_string(),
        ));
    }

    let mut input_keys = BTreeSet::new();
    let mut previous_input = None::<&str>;
    for input in &receipt.inputs {
        if previous_input.is_some_and(|previous| previous >= input.input_key.as_str()) {
            return Err(invalid(
                "receipt inputs must be strictly sorted by input_key".to_string(),
            ));
        }
        previous_input = Some(&input.input_key);
        if !is_lower_hex_64(&input.input_key) {
            return Err(invalid(format!(
                "input key {} is not lowercase 64-hex BLAKE3",
                input.input_key
            )));
        }
        let source =
            normalize_data_relative(&input.source_data_relative_path, "nif").map_err(invalid)?;
        if source != input.source_data_relative_path || !source.starts_with("meshes/") {
            return Err(invalid(format!(
                "input {} source path is not canonical under meshes/",
                input.input_key
            )));
        }
        if input.source_byte_len == 0 || !is_lower_hex_64(&input.source_blake3) {
            return Err(invalid(format!(
                "input {} has invalid source length or BLAKE3 fingerprint",
                input.input_key
            )));
        }
        if input.source_owner.is_empty() || input.source_owner.trim() != input.source_owner {
            return Err(invalid(format!(
                "input {} has invalid source_owner",
                input.input_key
            )));
        }
        match input.role {
            CreatureNifRole::Body => {
                if input
                    .body_variant
                    .as_deref()
                    .is_some_and(|variant| variant.is_empty() || variant.trim() != variant)
                {
                    return Err(invalid(format!(
                        "input {} has invalid body_variant",
                        input.input_key
                    )));
                }
            }
            CreatureNifRole::Skeleton if input.body_variant.is_some() => {
                return Err(invalid(format!(
                    "skeleton input {} has a body_variant",
                    input.input_key
                )));
            }
            CreatureNifRole::Skeleton => {}
        }
        let expected_key = stable_input_key(
            input.role,
            &input.source_data_relative_path,
            input.source_byte_len,
            &input.source_blake3,
            &input.source_owner,
            input.body_variant.as_deref(),
        );
        if input.input_key != expected_key {
            return Err(invalid(format!(
                "input key does not match canonical provenance for {}",
                input.source_data_relative_path
            )));
        }
        if !is_lower_hex_64(&input.output_fingerprint) {
            return Err(invalid(format!(
                "input {} output fingerprint is not BLAKE3",
                input.input_key
            )));
        }
        let articulated = match &input.collision {
            CreatureCollisionDisposition::ArticulatedDeferredForHkx { source_block_types } => {
                Some((source_block_types, None))
            }
            CreatureCollisionDisposition::ArticulatedEmbeddedFo4 {
                source_block_types,
                body_count,
            } => Some((source_block_types, Some(*body_count))),
            CreatureCollisionDisposition::None => None,
        };
        if let Some((source_block_types, body_count)) = articulated {
            if source_block_types.is_empty()
                || source_block_types.windows(2).any(|pair| pair[0] >= pair[1])
                || source_block_types.iter().any(|block| block.trim() != block)
                || body_count == Some(0)
            {
                return Err(invalid(format!(
                    "input {} has invalid articulated-collision provenance",
                    input.input_key
                )));
            }
        }
        let target =
            normalize_data_relative(&input.target_data_relative_path, "nif").map_err(invalid)?;
        if target != input.target_data_relative_path
            || !target.starts_with(&format!("meshes/{namespace}/"))
        {
            return Err(invalid(format!(
                "input {} target path is outside its namespace",
                input.input_key
            )));
        }
        input_keys.insert(input.input_key.clone());
    }
    if input_keys.is_empty() {
        return Err(invalid("receipt has no input dispositions".to_string()));
    }
    let expected_request_blake3 = request_blake3_from_dispositions(&receipt.inputs);
    if receipt.request_blake3 != expected_request_blake3 {
        return Err(invalid(
            "request_blake3 does not match canonical source input identities".to_string(),
        ));
    }

    let mut artifact_paths = BTreeSet::new();
    let mut previous_artifact = None::<&str>;
    let mut nif_artifacts = BTreeMap::<&str, &CreatureArtifactReceipt>::new();
    for artifact in &receipt.artifacts {
        if previous_artifact
            .is_some_and(|previous| previous >= artifact.target_data_relative_path.as_str())
        {
            return Err(invalid(
                "receipt artifacts must be strictly sorted by target path".to_string(),
            ));
        }
        previous_artifact = Some(&artifact.target_data_relative_path);
        let (root, extension) = match artifact.kind {
            CreatureArtifactKind::Nif => ("meshes", "nif"),
            CreatureArtifactKind::Bgsm => ("materials", "bgsm"),
            CreatureArtifactKind::Bgem => ("materials", "bgem"),
            CreatureArtifactKind::Dds => ("textures", "dds"),
        };
        let target = normalize_data_relative(&artifact.target_data_relative_path, extension)
            .map_err(invalid)?;
        if target != artifact.target_data_relative_path
            || !target.starts_with(&format!("{root}/{namespace}/"))
        {
            return Err(invalid(format!(
                "artifact path {} is outside its canonical namespace",
                artifact.target_data_relative_path
            )));
        }
        if artifact.byte_len == 0 || !is_lower_hex_64(&artifact.fingerprint) {
            return Err(invalid(format!(
                "artifact {} has invalid length or BLAKE3 fingerprint",
                artifact.target_data_relative_path
            )));
        }
        if artifact.source_inputs.is_empty()
            || artifact
                .source_inputs
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
            || artifact
                .source_inputs
                .iter()
                .any(|key| !input_keys.contains(key))
        {
            return Err(invalid(format!(
                "artifact {} has invalid source-input provenance",
                artifact.target_data_relative_path
            )));
        }
        if artifact.kind == CreatureArtifactKind::Nif {
            nif_artifacts.insert(&artifact.target_data_relative_path, artifact);
        }
        artifact_paths.insert(artifact.target_data_relative_path.clone());
    }
    if artifact_paths.len() != receipt.artifacts.len() {
        return Err(invalid("receipt has duplicate artifact paths".to_string()));
    }
    for input in &receipt.inputs {
        let Some(artifact) = nif_artifacts.get(input.target_data_relative_path.as_str()) else {
            return Err(invalid(format!(
                "input {} has no matching NIF artifact",
                input.input_key
            )));
        };
        if artifact.fingerprint != input.output_fingerprint
            || !artifact.source_inputs.contains(&input.input_key)
        {
            return Err(invalid(format!(
                "input {} does not match its NIF artifact",
                input.input_key
            )));
        }
    }

    let expected_hash = receipt_hash(receipt).map_err(|failure| invalid(failure.message))?;
    if receipt.receipt_hash != expected_hash {
        return Err(invalid(
            "receipt BLAKE3 hash mismatch (tampered receipt)".to_string(),
        ));
    }
    Ok(())
}

fn is_lower_hex_64(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn fingerprint(bytes: &[u8]) -> String {
    blake3_hash(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[derive(Clone, Copy)]
struct Blake3Output {
    input_cv: [u32; 8],
    block_words: [u32; 16],
    counter: u64,
    block_len: u32,
    flags: u32,
}

impl Blake3Output {
    fn chaining_value(self) -> [u32; 8] {
        let words = blake3_compress(
            self.input_cv,
            self.block_words,
            self.counter,
            self.block_len,
            self.flags,
        );
        words[..8].try_into().expect("eight chaining words")
    }

    fn root_hash(self) -> [u8; 32] {
        let words = blake3_compress(
            self.input_cv,
            self.block_words,
            0,
            self.block_len,
            self.flags | 8,
        );
        let mut hash = [0_u8; 32];
        for (index, word) in words[..8].iter().enumerate() {
            hash[index * 4..index * 4 + 4].copy_from_slice(&word.to_le_bytes());
        }
        hash
    }
}

fn blake3_hash(bytes: &[u8]) -> [u8; 32] {
    let chunk_count = bytes.len().div_ceil(1024).max(1);
    let mut cv_stack = Vec::<[u32; 8]>::new();
    for chunk_index in 0..chunk_count.saturating_sub(1) {
        let start = chunk_index * 1024;
        let mut cv =
            blake3_chunk_output(&bytes[start..start + 1024], chunk_index as u64).chaining_value();
        let mut total_chunks = chunk_index + 1;
        while total_chunks & 1 == 0 {
            let left = cv_stack.pop().expect("balanced BLAKE3 CV stack");
            cv = blake3_parent_output(left, cv).chaining_value();
            total_chunks >>= 1;
        }
        cv_stack.push(cv);
    }
    let final_index = chunk_count - 1;
    let final_start = final_index * 1024;
    let mut output = blake3_chunk_output(&bytes[final_start..], final_index as u64);
    while let Some(left) = cv_stack.pop() {
        output = blake3_parent_output(left, output.chaining_value());
    }
    output.root_hash()
}

fn blake3_chunk_output(chunk: &[u8], chunk_counter: u64) -> Blake3Output {
    const IV: [u32; 8] = [
        0x6A09_E667,
        0xBB67_AE85,
        0x3C6E_F372,
        0xA54F_F53A,
        0x510E_527F,
        0x9B05_688C,
        0x1F83_D9AB,
        0x5BE0_CD19,
    ];
    let block_count = chunk.len().div_ceil(64).max(1);
    let mut cv = IV;
    for block_index in 0..block_count {
        let start = block_index * 64;
        let end = (start + 64).min(chunk.len());
        let block = if start < chunk.len() {
            &chunk[start..end]
        } else {
            &[]
        };
        let mut block_bytes = [0_u8; 64];
        block_bytes[..block.len()].copy_from_slice(block);
        let mut block_words = [0_u32; 16];
        for (index, word) in block_bytes.chunks_exact(4).enumerate() {
            block_words[index] = u32::from_le_bytes(word.try_into().expect("four-byte word"));
        }
        let mut flags = 0;
        if block_index == 0 {
            flags |= 1;
        }
        if block_index + 1 == block_count {
            return Blake3Output {
                input_cv: cv,
                block_words,
                counter: chunk_counter,
                block_len: block.len() as u32,
                flags: flags | 2,
            };
        }
        let output = blake3_compress(cv, block_words, chunk_counter, 64, flags);
        cv.copy_from_slice(&output[..8]);
    }
    unreachable!("BLAKE3 chunk always has a final block")
}

fn blake3_parent_output(left: [u32; 8], right: [u32; 8]) -> Blake3Output {
    const IV: [u32; 8] = [
        0x6A09_E667,
        0xBB67_AE85,
        0x3C6E_F372,
        0xA54F_F53A,
        0x510E_527F,
        0x9B05_688C,
        0x1F83_D9AB,
        0x5BE0_CD19,
    ];
    let mut block_words = [0_u32; 16];
    block_words[..8].copy_from_slice(&left);
    block_words[8..].copy_from_slice(&right);
    Blake3Output {
        input_cv: IV,
        block_words,
        counter: 0,
        block_len: 64,
        flags: 4,
    }
}

fn blake3_compress(
    cv: [u32; 8],
    mut message: [u32; 16],
    counter: u64,
    block_len: u32,
    flags: u32,
) -> [u32; 16] {
    const IV: [u32; 4] = [0x6A09_E667, 0xBB67_AE85, 0x3C6E_F372, 0xA54F_F53A];
    const PERMUTATION: [usize; 16] = [2, 6, 3, 10, 7, 0, 4, 13, 1, 11, 12, 5, 9, 14, 15, 8];
    let mut state = [0_u32; 16];
    state[..8].copy_from_slice(&cv);
    state[8..12].copy_from_slice(&IV);
    state[12] = counter as u32;
    state[13] = (counter >> 32) as u32;
    state[14] = block_len;
    state[15] = flags;
    for round in 0..7 {
        blake3_round(&mut state, &message);
        if round != 6 {
            message = PERMUTATION.map(|index| message[index]);
        }
    }
    let mut output = [0_u32; 16];
    for index in 0..8 {
        output[index] = state[index] ^ state[index + 8];
        output[index + 8] = state[index + 8] ^ cv[index];
    }
    output
}

fn blake3_round(state: &mut [u32; 16], message: &[u32; 16]) {
    blake3_mix(state, 0, 4, 8, 12, message[0], message[1]);
    blake3_mix(state, 1, 5, 9, 13, message[2], message[3]);
    blake3_mix(state, 2, 6, 10, 14, message[4], message[5]);
    blake3_mix(state, 3, 7, 11, 15, message[6], message[7]);
    blake3_mix(state, 0, 5, 10, 15, message[8], message[9]);
    blake3_mix(state, 1, 6, 11, 12, message[10], message[11]);
    blake3_mix(state, 2, 7, 8, 13, message[12], message[13]);
    blake3_mix(state, 3, 4, 9, 14, message[14], message[15]);
}

fn blake3_mix(state: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize, mx: u32, my: u32) {
    state[a] = state[a].wrapping_add(state[b]).wrapping_add(mx);
    state[d] = (state[d] ^ state[a]).rotate_right(16);
    state[c] = state[c].wrapping_add(state[d]);
    state[b] = (state[b] ^ state[c]).rotate_right(12);
    state[a] = state[a].wrapping_add(state[b]).wrapping_add(my);
    state[d] = (state[d] ^ state[a]).rotate_right(8);
    state[c] = state[c].wrapping_add(state[d]);
    state[b] = (state[b] ^ state[c]).rotate_right(7);
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("validated DDS header"),
    )
}

fn is_skinned_shape(block: &NifBlock) -> bool {
    is_fo4_geometry(block)
        && (ref_field(block, "Skin").is_some_and(|reference| reference >= 0)
            || ref_field(block, "Skin Instance").is_some_and(|reference| reference >= 0)
            || block
                .get_field("Vertex Desc")
                .is_some_and(|value| ((value.as_i64() >> 44) & 0x40) != 0))
}

fn is_fo4_geometry(block: &NifBlock) -> bool {
    matches!(
        block.type_name.as_str(),
        "BSTriShape" | "BSSubIndexTriShape" | "BSDynamicTriShape"
    )
}

fn is_node(block: &NifBlock) -> bool {
    block.type_name.ends_with("Node") || block.type_name == "NiBone"
}

fn ref_field(block: &NifBlock, field: &str) -> Option<i32> {
    match block.get_field(field) {
        Some(NifValue::Ref(value)) => Some(*value),
        Some(NifValue::Int(value)) => i32::try_from(*value).ok(),
        Some(NifValue::UInt(value)) => i32::try_from(*value).ok(),
        _ => None,
    }
}

fn ref_array(value: Option<&NifValue>) -> Vec<i32> {
    match value {
        Some(NifValue::Array(values)) => values
            .iter()
            .filter_map(|value| match value {
                NifValue::Ref(reference) => Some(*reference),
                NifValue::Int(reference) => i32::try_from(*reference).ok(),
                NifValue::UInt(reference) => i32::try_from(*reference).ok(),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn string_field<'a>(block: &'a NifBlock, field: &str) -> Option<&'a str> {
    match block.get_field(field) {
        Some(NifValue::String(value)) => Some(value),
        _ => None,
    }
}

fn strings(value: &NifValue) -> Vec<&str> {
    let mut result = Vec::new();
    collect_strings(value, &mut result);
    result
}

fn collect_strings<'a>(value: &'a NifValue, result: &mut Vec<&'a str>) {
    match value {
        NifValue::String(value) => result.push(value),
        NifValue::Array(values) => values
            .iter()
            .for_each(|value| collect_strings(value, result)),
        NifValue::Struct(fields) => fields
            .values()
            .for_each(|value| collect_strings(value, result)),
        _ => {}
    }
}

fn input_for_task<'a>(
    source_inputs: &BTreeSet<String>,
    inputs: &'a [PreparedInput],
) -> &'a PreparedInput {
    let key = source_inputs
        .iter()
        .next()
        .expect("material/texture tasks always have provenance");
    inputs
        .iter()
        .find(|input| &input.input_key == key)
        .expect("prepared input provenance")
}

fn error(
    kind: CreatureTerminalKind,
    input_key: Option<String>,
    source_data_relative_path: Option<String>,
    message: impl Into<String>,
) -> CreatureClosureError {
    CreatureClosureError {
        kind,
        input_key,
        source_data_relative_path,
        message: message.into(),
    }
}

fn input_error(
    kind: CreatureTerminalKind,
    input: &PreparedInput,
    message: impl Into<String>,
) -> CreatureClosureError {
    error(
        kind,
        Some(input.input_key.clone()),
        Some(input.source_relative.clone()),
        message,
    )
}

fn task_error(
    kind: CreatureTerminalKind,
    source_inputs: &BTreeSet<String>,
    inputs: &[PreparedInput],
    message: impl Into<String>,
) -> CreatureClosureError {
    input_error(kind, input_for_task(source_inputs, inputs), message)
}

fn io_error(action: &str, io: std::io::Error) -> CreatureClosureError {
    error(
        CreatureTerminalKind::Io,
        None,
        None,
        format!("{action}: {io}"),
    )
}

fn input_io_error(input: &PreparedInput, action: &str, io: std::io::Error) -> CreatureClosureError {
    input_error(CreatureTerminalKind::Io, input, format!("{action}: {io}"))
}

#[cfg(test)]
mod tests {
    use super::{fingerprint, normalize_namespace, rewrite_bgsm};

    #[test]
    fn nested_target_namespace_is_normalized_without_allowing_escape() {
        assert_eq!(
            normalize_namespace(r"Actors\FnvFo3Creature_0123").expect("nested namespace"),
            "actors/fnvfo3creature_0123"
        );
        for invalid in ["", "../escape", "actors//creature", "C:/escape"] {
            assert!(
                normalize_namespace(invalid).is_err(),
                "accepted {invalid:?}"
            );
        }
    }

    #[test]
    fn blake3_known_vectors() {
        assert_eq!(
            fingerprint(b""),
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
        );
        assert_eq!(
            fingerprint(b"abc"),
            "6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85"
        );
        for (length, expected) in [
            (
                1024,
                "42214739f095a406f3fc83deb889744ac00df831c10daa55189b5d121c855af7",
            ),
            (
                1025,
                "d00278ae47eb27b34faecf67b4fe263f82d5412916c1ffd97c8cb7fb814b8444",
            ),
            (
                3073,
                "7124b49501012f81cc7f11ca069ec9226cecb8a2c850cfe644e327d22d3e1cd3",
            ),
            (
                102_400,
                "bc3e3d41a1146b069abffad3c0d44860cf664390afce4d9661f7902e7943e085",
            ),
        ] {
            let input = (0..length)
                .map(|index| (index % 251) as u8)
                .collect::<Vec<_>>();
            assert_eq!(fingerprint(&input), expected);
        }
    }

    #[test]
    fn rewritten_creature_material_casts_shadows() {
        let mut material = materials_native::bgsm::BgsmData::default();
        material.header.signature = materials_native::bgsm::BGSM_SIGNATURE;
        material.header.version = 2;
        material.CastShadows = false;

        let (bytes, _, _) = rewrite_bgsm(
            &materials_native::bgsm::write(&material),
            "fo4",
            true,
            "fo76",
        )
        .expect("rewrite creature BGSM");
        let rewritten = materials_native::bgsm::parse(&bytes).expect("parse rewritten BGSM");

        assert!(rewritten.CastShadows);
    }
}
