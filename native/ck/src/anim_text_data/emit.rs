//! End-to-end AnimTextData emission (CK-free): writes bucket files to disk for a
//! set of subgraphs, deriving the id from the RACE-record fields (core behavior +
//! SAPT chain) and the file list from the behavior graph + on-disk SAPT resolution.
//!
//! This is the production `generate_anim_text_data` emitter. It writes the
//! independently derivable creature buckets plus the serialized project-wide
//! Offsets aggregate and per-combo weapon StanceData (byte-exact base reuse where
//! the combo is a vanilla subgraph, generated from the converted skeleton otherwise).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use rayon::prelude::*;

use super::behavior_index::resolve_subgraph_files;
use super::bucket_files::{
    anim_event_info_body, animation_file_data_body, animation_offsets_empty_body,
    clip_generator_data_body, dynamic_idle_data_body, project_manifest_body, sync_anim_data_body,
    sync_anim_data_body_existing,
};
use super::core::{name_id, subgraph_id};
use super::event_resolver::resolve_anim_events;
use super::extract::{
    clip_generator_entries, expand_idle_glob, extract_fx_manifest, extract_project_manifest,
    fx_project_dirs, project_hkx_relpath, race_dir_of, race_name_of, race_name_of_dir,
};
use super::graph::GraphResolver;
use super::offsets::{
    build_offsets_aggregate, build_subgraph_offsets_body, build_subgraph_offsets_body_furniture,
    build_subgraph_offsets_body_weapon, is_furniture_core_behavior,
};
use super::single_file;
use super::speed::{
    build_speed_info_body, build_speed_info_body_weapon, speed_info_leaf_basenames,
};
use super::stance::{
    WeaponStanceBuilder, WeaponSubgraphMetadata, behavior_wants_head_tracking,
    emit_stance_for_subgraph,
};
use super::sync::{build_plugin_sync_anim_data, plugin_sync_anim_filename, weapon_sync_anim_filenames};

const AUTHORITATIVE_MANIFEST: &str = "AnimTextData/.modkit-authoritative-files.json";

/// A subgraph to emit, as read from a RACE record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubgraphInput {
    /// `SGNM` core behavior path (e.g. `r"Actors\X\Behaviors\XCoreBehavior.hkx"`).
    pub core_behavior: String,
    /// `SAPT` chain, self-first (e.g. `[r"Actors\X\Animations\Injured\RightLeg", r"Actors\X\Animations"]`).
    pub sapt_chain: Vec<String>,
    /// The owning race's own actor dir (`Actors\<Race>`), from the RACE record's
    /// skeletal model. Humanoid creatures (scorched, mole miner) mount the shared
    /// `Actors\Character\Behaviors\*` cores, so the core path names `Character`, not
    /// the race — their project lives under this dir instead. `None` falls back to
    /// deriving the dir from `core_behavior`, which is right for ordinary creatures.
    pub race_dir: Option<String>,
}

/// Weapon-only metadata kept separate so existing `SubgraphInput` literals remain
/// source-compatible. Both views are derived from the same native RACE block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeaponProfileInput {
    pub subgraph: SubgraphInput,
    pub stance: WeaponSubgraphMetadata,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AnimTextDataInputs {
    pub race_record_count: usize,
    pub subgraphs: Vec<SubgraphInput>,
    pub weapon_profiles: Vec<WeaponProfileInput>,
    pub base_stance_profiles: Vec<WeaponSubgraphMetadata>,
    pub target_plugin_name: String,
    pub idle_globs: Vec<String>,
    pub event_candidates: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AnimTextDataReport {
    pub written: u32,
    pub stance_reused: u32,
    pub stance_generated: u32,
    pub stance_skipped: u32,
    pub stance_builder_error: Option<String>,
}

impl SubgraphInput {
    /// The `SubgraphIdentifier` (filename id) for this subgraph.
    pub fn id(&self) -> u64 {
        let chain: Vec<&str> = self.sapt_chain.iter().map(String::as_str).collect();
        subgraph_id(&self.core_behavior, &chain)
    }
}

fn deduplicated_subgraphs(subgraphs: &[SubgraphInput]) -> Vec<SubgraphInput> {
    let mut by_id = BTreeMap::new();
    for subgraph in subgraphs {
        by_id
            .entry(subgraph.id())
            .or_insert_with(|| subgraph.clone());
    }
    by_id.into_values().collect()
}

fn deduplicated_stance_profiles(
    profiles: &[WeaponSubgraphMetadata],
) -> Vec<WeaponSubgraphMetadata> {
    let mut by_id = BTreeMap::new();
    for profile in profiles {
        by_id.entry(profile.id).or_insert_with(|| profile.clone());
    }
    by_id.into_values().collect()
}

fn same_stance_asset_identity(
    target: &WeaponSubgraphMetadata,
    base: &WeaponSubgraphMetadata,
) -> bool {
    target.id == base.id
        && target.perspective == base.perspective
        && target.sraf == base.sraf
        && target
            .core_behavior
            .replace('/', "\\")
            .eq_ignore_ascii_case(&base.core_behavior.replace('/', "\\"))
        && target.sapt.len() == base.sapt.len()
        && target.sapt.iter().zip(&base.sapt).all(|(left, right)| {
            left.replace('/', "\\")
                .eq_ignore_ascii_case(&right.replace('/', "\\"))
        })
}

fn exact_base_stance_body(
    target: &WeaponSubgraphMetadata,
    base_profiles: &[WeaponSubgraphMetadata],
    base_stance_root: &Path,
) -> Result<Option<Vec<u8>>, String> {
    let Some(base) = base_profiles.iter().find(|profile| profile.id == target.id) else {
        return Ok(None);
    };
    if !same_stance_asset_identity(target, base) {
        return Err(format!(
            "weapon StanceData id collision {} has different target/base graph metadata",
            target.id
        ));
    }
    let path = base_stance_root.join(format!("{}.txt", target.id));
    if !path.is_file() {
        return Ok(None);
    }
    std::fs::read(&path).map(Some).map_err(|error| {
        format!(
            "failed to read exact base StanceData {}: {error}",
            path.display()
        )
    })
}

/// Whether a profile carried by the converted plugin's races is a weapon graft.
/// A weapon subgraph grafts onto base-game character behaviors, so its core is
/// never shipped by the mod; a converted creature's core is. Owner-plugin alone
/// is not enough — in a whole-plugin conversion every creature race lives in the
/// target plugin.
fn is_target_weapon_profile(
    profile: &WeaponProfileInput,
    target_plugin_name: &str,
    src_meshes_root: &Path,
) -> bool {
    profile
        .stance
        .race_family
        .owner_race
        .plugin
        .eq_ignore_ascii_case(target_plugin_name)
        && !src_meshes_root
            .join(profile.stance.core_behavior.replace('\\', "/"))
            .is_file()
}

/// Subgraph ids the derivable creature writers must skip because they are weapon
/// grafts onto the base-game character graph.
fn collect_weapon_subgraph_ids(
    weapon_profiles: &[WeaponProfileInput],
    target_plugin_name: &str,
    src_meshes_root: &Path,
) -> BTreeSet<u64> {
    weapon_profiles
        .iter()
        .filter(|profile| is_target_weapon_profile(profile, target_plugin_name, src_meshes_root))
        .map(|profile| profile.subgraph.id())
        .collect()
}

pub fn generate_anim_text_data(
    inputs: &AnimTextDataInputs,
    src_meshes_root: &Path,
    out_meshes_root: &Path,
    base_meshes_root: Option<&Path>,
    mod_prefix: Option<&str>,
) -> Result<AnimTextDataReport, String> {
    generate_anim_text_data_with_progress(
        inputs,
        src_meshes_root,
        out_meshes_root,
        base_meshes_root,
        mod_prefix,
        &mut |_| {},
    )
}

pub fn generate_anim_text_data_with_progress(
    inputs: &AnimTextDataInputs,
    src_meshes_root: &Path,
    out_meshes_root: &Path,
    base_meshes_root: Option<&Path>,
    mod_prefix: Option<&str>,
    progress: &mut dyn FnMut(&str),
) -> Result<AnimTextDataReport, String> {
    let started = Instant::now();
    let subgraphs = deduplicated_subgraphs(&inputs.subgraphs);
    let weapon_subgraph_ids = collect_weapon_subgraph_ids(
        &inputs.weapon_profiles,
        &inputs.target_plugin_name,
        src_meshes_root,
    );
    let base_stance_profiles = deduplicated_stance_profiles(&inputs.base_stance_profiles);
    progress(&format!(
        "records: races={} subgraphs={} weapon_profiles={} base_stance_profiles={} idle_globs={} event_candidates={} elapsed={:.1}s",
        inputs.race_record_count,
        subgraphs.len(),
        inputs.weapon_profiles.len(),
        base_stance_profiles.len(),
        inputs.idle_globs.len(),
        inputs.event_candidates.len(),
        started.elapsed().as_secs_f64(),
    ));

    let phase_started = Instant::now();
    progress("authoritative buckets: starting");
    let authoritative = emit_serialized_production_buckets(
        &subgraphs,
        &inputs.weapon_profiles,
        &base_stance_profiles,
        &inputs.target_plugin_name,
        src_meshes_root,
        out_meshes_root,
        base_meshes_root,
    )?;
    progress(&format!(
        "authoritative buckets: wrote {} file(s) (weapon stance reused={} generated={} skipped={}{}) in {:.1}s",
        authoritative.written,
        authoritative.stance_reused,
        authoritative.stance_generated,
        authoritative.stance_skipped,
        authoritative
            .stance_builder_error
            .as_deref()
            .map(|error| format!(" builder_error={error}"))
            .unwrap_or_default(),
        phase_started.elapsed().as_secs_f64(),
    ));

    let mut written = authoritative.written;
    let phase_started = Instant::now();
    progress(&format!(
        "AnimationFileData: starting {} subgraph(s)",
        subgraphs.len()
    ));
    let animation_file_data_written = emit_animation_file_data_with_weapon_ids(
        &subgraphs,
        &weapon_subgraph_ids,
        src_meshes_root,
        out_meshes_root,
        base_meshes_root,
    )
    .map_err(|error| error.to_string())?;
    written += animation_file_data_written;
    progress(&format!(
        "AnimationFileData: wrote {animation_file_data_written} file(s) in {:.1}s",
        phase_started.elapsed().as_secs_f64(),
    ));

    let phase_started = Instant::now();
    progress("derivable buckets: starting");
    let derivable = emit_derivable_buckets_with_progress(
        &subgraphs,
        &weapon_subgraph_ids,
        &inputs.idle_globs,
        &inputs.event_candidates,
        src_meshes_root,
        out_meshes_root,
        base_meshes_root,
        mod_prefix,
        progress,
    );
    written += derivable.written;
    progress(&format!(
        "derivable buckets: {} in {:.1}s",
        derivable.summary(),
        phase_started.elapsed().as_secs_f64(),
    ));

    if EMIT_STRUCTURAL_AGGREGATES {
        let phase_started = Instant::now();
        progress("structural aggregates: starting");
        let structural = emit_structural_aggregates(out_meshes_root, base_meshes_root, progress)?;
        written += structural;
        progress(&format!(
            "structural aggregates: wrote {structural} file(s) in {:.1}s",
            phase_started.elapsed().as_secs_f64(),
        ));
    }

    progress(&format!(
        "complete: wrote {written} AnimTextData bucket file(s) in {:.1}s",
        started.elapsed().as_secs_f64(),
    ));

    Ok(AnimTextDataReport {
        written,
        stance_reused: authoritative.stance_reused,
        stance_generated: authoritative.stance_generated,
        stance_skipped: authoritative.stance_skipped,
        stance_builder_error: authoritative.stance_builder_error,
    })
}

const SINGLE_FILE_NAME: &str =
    "behaviorclipinformationandsubgraphanimationoffsetssinglefile.txt";

/// Off: mods must not ship dirlists or the singlefile. Shipped CK-built fan mods
/// (B21_PlasmaCaster, Snallygaster) carry neither — only per-bucket data files and a
/// plugin-level SyncAnimData file — and they work, which disproves the "singlefile is
/// the only clip-generator channel" premise this phase was built on. Emitting one is
/// actively harmful: a mod's copy is the merged-VFS winner, so it shadows vanilla's
/// entire clip table. The writers stay tested and available behind this flag.
const EMIT_STRUCTURAL_AGGREGATES: bool = false;

/// Final aggregation phase: CK-parity dirlists + the merged singlefile. Must run after
/// every bucket writer (it aggregates the final on-disk set). Returns files written.
/// Gated off by `EMIT_STRUCTURAL_AGGREGATES`.
///
/// Singlefile policy, when enabled: a mod's copy fully shadows vanilla's, so ours =
/// vanilla's entries verbatim + our ClipGeneratorData entries appended. No vanilla
/// singlefile → emit none (engine reads vanilla's own).
fn emit_structural_aggregates(
    out_meshes_root: &Path,
    base_meshes_root: Option<&Path>,
    progress: &mut dyn FnMut(&str),
) -> Result<u32, String> {
    let mut written = 0u32;

    let vanilla_path = base_meshes_root
        .map(|base| base.join("AnimTextData").join(SINGLE_FILE_NAME))
        .filter(|path| path.is_file());
    match vanilla_path {
        None => progress("singlefile: no vanilla source; skipping (engine falls back to vanilla's)"),
        Some(vanilla_path) => {
            let vanilla = std::fs::read(&vanilla_path).map_err(|error| {
                format!("failed to read {}: {error}", vanilla_path.display())
            })?;
            let clipgen_dir = out_meshes_root
                .join("AnimTextData")
                .join("ClipGeneratorData");
            let mut additions: Vec<(u32, Vec<u8>)> = Vec::new();
            if clipgen_dir.is_dir() {
                let mut keyed: Vec<(u32, PathBuf)> = Vec::new();
                for entry in std::fs::read_dir(&clipgen_dir).map_err(|error| {
                    format!("failed to list {}: {error}", clipgen_dir.display())
                })? {
                    let path = entry
                        .map_err(|error| {
                            format!("failed to list {}: {error}", clipgen_dir.display())
                        })?
                        .path();
                    let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
                        continue;
                    };
                    if let Ok(key) = stem.parse::<u32>() {
                        keyed.push((key, path));
                    }
                }
                keyed.sort_by_key(|(key, _)| *key);
                for (key, path) in keyed {
                    let body = std::fs::read(&path).map_err(|error| {
                        format!("failed to read {}: {error}", path.display())
                    })?;
                    additions.push((key, body));
                }
            }
            let (merged, applied) =
                single_file::compose_merged_single_file(&vanilla, &additions)
                    .map_err(|error| format!("singlefile compose failed: {error}"))?;
            let out_path = out_meshes_root.join("AnimTextData").join(SINGLE_FILE_NAME);
            std::fs::write(&out_path, merged)
                .map_err(|error| format!("failed to write {}: {error}", out_path.display()))?;
            written += 1;
            progress(&format!(
                "singlefile: merged vanilla + {applied} of {} clip entr(ies)",
                additions.len()
            ));
        }
    }

    let dirlists = super::dirlist::emit_dirlists(out_meshes_root)?;
    written += dirlists;
    progress(&format!("dirlists: wrote {dirlists} file(s)"));
    Ok(written)
}

/// Write `AnimTextData/AnimationFileData/<id>.txt` for each subgraph under
/// `out_meshes_root`. Returns the number of files written.
///
/// Resolution dispatches on RACE weapon metadata first, then on where the core
/// behavior lives. Weapon/character subgraphs always use the recursive graph walk;
/// a FO76-specific wrapper may be present in the mod while still depending on the
/// shared character graph. Only non-weapon local cores use the self-contained
/// creature resolver.
///
/// Subgraphs whose body resolves to zero files are skipped (never emit an empty
/// body — the engine rebuilds an absent file at load, but trusts an empty one).
pub fn emit_animation_file_data(
    subgraphs: &[SubgraphInput],
    src_meshes_root: &Path,
    out_meshes_root: &Path,
    base_meshes_root: Option<&Path>,
) -> std::io::Result<u32> {
    emit_animation_file_data_with_weapon_ids(
        subgraphs,
        &BTreeSet::new(),
        src_meshes_root,
        out_meshes_root,
        base_meshes_root,
    )
}

fn emit_animation_file_data_with_weapon_ids(
    subgraphs: &[SubgraphInput],
    weapon_subgraph_ids: &BTreeSet<u64>,
    src_meshes_root: &Path,
    out_meshes_root: &Path,
    base_meshes_root: Option<&Path>,
) -> std::io::Result<u32> {
    let bucket_dir = out_meshes_root
        .join("AnimTextData")
        .join("AnimationFileData");
    std::fs::create_dir_all(&bucket_dir)?;

    // Mod root first so weapon overrides (e.g. gauss-specific anims) win over base.
    let mut roots: Vec<PathBuf> = vec![src_meshes_root.to_path_buf()];
    if let Some(base) = base_meshes_root {
        roots.push(base.to_path_buf());
    }
    let mut resolver = GraphResolver::new(roots);

    let mut written = 0u32;
    for sg in deduplicated_subgraphs(subgraphs) {
        let id = sg.id();
        let core_in_mod = src_meshes_root
            .join(sg.core_behavior.replace('\\', "/"))
            .is_file();
        let mut files = if core_in_mod && !weapon_subgraph_ids.contains(&id) {
            let core_file = src_meshes_root.join(sg.core_behavior.replace('\\', "/"));
            resolve_subgraph_files(&core_file, src_meshes_root, &sg.sapt_chain)
        } else {
            resolver.resolve_body(&sg.core_behavior, &sg.sapt_chain)
        };
        // A reference-only wrapping graph (e.g. `GraftonCore_InjuredWrappingBehavior.hkx`:
        // a `hkbBehaviorReferenceGenerator` with no `hkbClipGenerator`) carries no direct
        // clips, so the single-file creature walk is empty. Fall through to the cross-file
        // resolver, which follows the behavior reference into the referenced core and lists
        // it + its SAPT-resolved clips — exactly what CK caches. Cores that already yield
        // clips never reach this fallthrough, so normal creatures are unaffected.
        if files.is_empty() && core_in_mod && !weapon_subgraph_ids.contains(&id) {
            files = resolver.resolve_body(&sg.core_behavior, &sg.sapt_chain);
        }
        if files.is_empty() {
            continue; // safe-skip: let the engine rebuild rather than ship an empty body
        }
        let body = animation_file_data_body(id, &files);
        std::fs::write(bucket_dir.join(format!("{id}.txt")), body)?;
        written += 1;
    }
    Ok(written)
}

/// Write `AnimTextData/<bucket>/<filename>` under the meshes root. Returns whether
/// the file was written (errors are swallowed — one bucket must not abort others).
fn write_bucket(atd_root: &Path, bucket: &str, filename: &str, body: &[u8]) -> bool {
    let dir = atd_root.join(bucket);
    if std::fs::create_dir_all(&dir).is_err() {
        return false;
    }
    std::fs::write(dir.join(filename), body).is_ok()
}

/// Emit the additional derivable buckets (everything beyond `AnimationFileData`'s
/// numeric per-subgraph files), grouped by race. Each bucket is best-effort: a
/// behavior that cannot be read or a creature without idle/project data simply
/// yields fewer files.
#[derive(Debug, Default)]
struct BucketEmissionReport {
    written: u32,
    by_bucket: BTreeMap<String, u32>,
}

impl BucketEmissionReport {
    fn write(&mut self, atd_root: &Path, bucket: &str, filename: &str, body: &[u8]) {
        if write_bucket(atd_root, bucket, filename, body) {
            self.written += 1;
            *self.by_bucket.entry(bucket.to_string()).or_default() += 1;
        }
    }

    fn summary(&self) -> String {
        self.by_bucket
            .iter()
            .map(|(bucket, count)| format!("{bucket}={count}"))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

struct PendingBucketFile {
    bucket: &'static str,
    filename: String,
    body: Vec<u8>,
}

#[derive(Default)]
struct PendingBucketFiles {
    files: Vec<PendingBucketFile>,
}

impl PendingBucketFiles {
    fn push(&mut self, bucket: &'static str, filename: String, body: &[u8]) {
        self.files.push(PendingBucketFile {
            bucket,
            filename,
            body: body.to_vec(),
        });
    }

    fn write_to(self, atd_root: &Path, report: &mut BucketEmissionReport) {
        for file in self.files {
            report.write(atd_root, file.bucket, &file.filename, &file.body);
        }
    }
}

struct DerivableRaceEmission {
    files: PendingBucketFiles,
    elapsed_seconds: f64,
}

#[derive(Debug, Default)]
pub struct AuthoritativeEmissionReport {
    pub written: u32,
    pub stance_reused: u32,
    pub stance_generated: u32,
    pub stance_skipped: u32,
    pub stance_builder_error: Option<String>,
}

struct AuthoritativeFile {
    relative_path: PathBuf,
    body: Vec<u8>,
}

struct OwnedAuthoritativeFiles {
    relative_paths: BTreeSet<PathBuf>,
}

impl OwnedAuthoritativeFiles {
    fn load(out: &Path) -> Result<Self, String> {
        let manifest_path = out.join(AUTHORITATIVE_MANIFEST);
        let mut relative_paths = BTreeSet::from([PathBuf::from(AUTHORITATIVE_MANIFEST)]);
        if !manifest_path.is_file() {
            return Ok(Self { relative_paths });
        }
        let body = std::fs::read(&manifest_path).map_err(|error| {
            format!(
                "failed to read authoritative AnimTextData manifest {}: {error}",
                manifest_path.display()
            )
        })?;
        let manifest: AuthoritativeManifest = serde_json::from_slice(&body).map_err(|error| {
            format!(
                "failed to parse authoritative AnimTextData manifest {}: {error}",
                manifest_path.display()
            )
        })?;
        if manifest.version != 1 {
            return Err(format!(
                "unsupported authoritative AnimTextData manifest version {}",
                manifest.version
            ));
        }
        for relative in manifest.files {
            let relative = PathBuf::from(relative);
            if !is_authoritative_output(&relative) {
                return Err(format!(
                    "authoritative AnimTextData manifest contains an invalid owned path: {}",
                    relative.display()
                ));
            }
            relative_paths.insert(relative);
        }
        Ok(Self { relative_paths })
    }

    fn add(&mut self, relative: PathBuf) {
        self.relative_paths.insert(relative);
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct AuthoritativeManifest {
    version: u8,
    files: Vec<String>,
}

fn is_authoritative_output(relative: &Path) -> bool {
    if relative
        == Path::new("AnimTextData/AnimationOffsets/PersistantSubgraphInfoAndOffsetData.txt")
    {
        return true;
    }
    let Some(parent) = relative.parent() else {
        return false;
    };
    let Some(filename) = relative.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    if parent == Path::new("AnimTextData/SyncAnimData") {
        return filename.starts_with("ResolvedSyncAnimData") && filename.ends_with(".txt");
    }
    if parent == Path::new("AnimTextData/AnimationStanceData") {
        return filename.strip_suffix(".txt").is_some_and(|stem| {
            !stem.is_empty() && stem.bytes().all(|byte| byte.is_ascii_digit())
        });
    }
    false
}

fn authoritative_manifest_body(files: &[AuthoritativeFile]) -> Result<Vec<u8>, String> {
    let mut paths: Vec<String> = files
        .iter()
        .map(|file| file.relative_path.to_string_lossy().replace('\\', "/"))
        .collect();
    paths.sort();
    paths.dedup();
    serde_json::to_vec_pretty(&AuthoritativeManifest {
        version: 1,
        files: paths,
    })
    .map_err(|error| format!("failed to serialize authoritative AnimTextData manifest: {error}"))
}

fn remove_owned_authoritative_outputs(
    out: &Path,
    owned: &OwnedAuthoritativeFiles,
) -> Result<(), String> {
    let mut failures = Vec::new();
    for relative in &owned.relative_paths {
        let path = out.join(relative);
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => failures.push(format!("{}: {error}", path.display())),
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "failed to remove stale authoritative AnimTextData: {}",
            failures.join("; ")
        ))
    }
}

fn stage_and_publish_authoritative_files(
    out: &Path,
    owned: &OwnedAuthoritativeFiles,
    files: &[AuthoritativeFile],
) -> Result<(), String> {
    std::fs::create_dir_all(out)
        .map_err(|error| format!("failed to create output root {}: {error}", out.display()))?;
    let staging = tempfile::Builder::new()
        .prefix(".anim-text-data-")
        .tempdir_in(out)
        .map_err(|error| format!("failed to create AnimTextData staging directory: {error}"))?;

    let manifest = AuthoritativeFile {
        relative_path: PathBuf::from(AUTHORITATIVE_MANIFEST),
        body: authoritative_manifest_body(files)?,
    };
    for file in files.iter().chain(std::iter::once(&manifest)) {
        let path = staging.path().join(&file.relative_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "failed to create staged directory {}: {error}",
                    parent.display()
                )
            })?;
        }
        std::fs::write(&path, &file.body)
            .map_err(|error| format!("failed to stage {}: {error}", path.display()))?;
    }

    for file in files.iter().chain(std::iter::once(&manifest)) {
        let final_path = out.join(&file.relative_path);
        if let Some(parent) = final_path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "failed to create authoritative directory {}: {error}",
                    parent.display()
                )
            })?;
        }
    }
    remove_owned_authoritative_outputs(out, owned)?;

    for file in files.iter().chain(std::iter::once(&manifest)) {
        let staged_path = staging.path().join(&file.relative_path);
        let final_path = out.join(&file.relative_path);
        if let Err(error) = std::fs::rename(&staged_path, &final_path) {
            let cleanup = remove_owned_authoritative_outputs(out, owned);
            return Err(match cleanup {
                Ok(()) => format!(
                    "failed to publish authoritative {}: {error}",
                    final_path.display()
                ),
                Err(cleanup_error) => format!(
                    "failed to publish authoritative {}: {error}; {cleanup_error}",
                    final_path.display()
                ),
            });
        }
    }
    Ok(())
}

pub fn emit_serialized_production_buckets(
    subgraphs: &[SubgraphInput],
    weapon_profiles: &[WeaponProfileInput],
    base_stance_profiles: &[WeaponSubgraphMetadata],
    target_plugin_name: &str,
    src: &Path,
    out: &Path,
    base_meshes_root: Option<&Path>,
) -> Result<AuthoritativeEmissionReport, String> {
    let target_weapon_profiles: Vec<WeaponProfileInput> = weapon_profiles
        .iter()
        .filter(|profile| is_target_weapon_profile(profile, target_plugin_name, src))
        .cloned()
        .collect();
    let mut targets = Vec::with_capacity(target_weapon_profiles.len());
    for profile in &target_weapon_profiles {
        if profile.subgraph.id() != profile.stance.id {
            return Err(format!(
                "weapon StanceData profile id mismatch: {} != {}",
                profile.subgraph.id(),
                profile.stance.id
            ));
        }
        targets.push(profile.stance.clone());
    }
    let targets = deduplicated_stance_profiles(&targets);
    let weapon_subgraphs: Vec<SubgraphInput> = target_weapon_profiles
        .iter()
        .map(|profile| profile.subgraph.clone())
        .collect();
    let mut owned = OwnedAuthoritativeFiles::load(out)?;
    if !targets.is_empty() {
        owned.add(PathBuf::from(
            "AnimTextData/AnimationOffsets/PersistantSubgraphInfoAndOffsetData.txt",
        ));
        for target in &targets {
            owned.add(
                PathBuf::from("AnimTextData/AnimationStanceData")
                    .join(format!("{}.txt", target.id)),
            );
        }
        if let Ok(filenames) = weapon_sync_anim_filenames(&weapon_subgraphs) {
            for filename in &filenames {
                owned.add(PathBuf::from("AnimTextData/SyncAnimData").join(filename));
            }
        }
    }
    if let Some(filename) = plugin_sync_anim_filename(target_plugin_name) {
        owned.add(PathBuf::from("AnimTextData/SyncAnimData").join(&filename));
    }

    let built = (|| {
        let mut files = Vec::new();
        let mut report = AuthoritativeEmissionReport::default();

        if !targets.is_empty() {
            let base = base_meshes_root
                .ok_or_else(|| "weapon AnimTextData requires a base meshes root".to_string())?;
            let aggregate_name = "PersistantSubgraphInfoAndOffsetData.txt";
            let base_aggregate = base
                .join("AnimTextData")
                .join("AnimationOffsets")
                .join(aggregate_name);
            if !base_aggregate.is_file() {
                return Err(format!(
                    "required base AnimationOffsets aggregate is missing: {}",
                    base_aggregate.display()
                ));
            }
            let aggregate = build_offsets_aggregate(subgraphs, Some(&base_aggregate))
                .ok_or_else(|| "trusted base AnimationOffsets aggregate is invalid".to_string())?
                .concat()
                .into_bytes();
            files.push(AuthoritativeFile {
                relative_path: PathBuf::from("AnimTextData/AnimationOffsets").join(aggregate_name),
                body: aggregate,
            });

            let base_profiles = deduplicated_stance_profiles(base_stance_profiles);
            if base_profiles.iter().any(|profile| {
                profile
                    .race_family
                    .owner_race
                    .plugin
                    .eq_ignore_ascii_case(target_plugin_name)
            }) {
                return Err(
                    "weapon StanceData base donor source contains target-owned RACE profiles"
                        .to_string(),
                );
            }
            let base_stance_root = base.join("AnimTextData").join("AnimationStanceData");
            // Weapon StanceData, one file per subgraph combo. The byte-exact base file is
            // ground truth when the combo is a vanilla reuse (target id == a base id with the
            // same graph identity); every other combo is generated from the converted
            // character skeleton (first section) with the aim grid sidestepped from the
            // base-game donor whose behavior role matches. The builder is constructed once
            // so the donor catalog is decoded a single time. It needs the base RACE donors,
            // so a target-only handle (none supplied) withholds generated stance and lets
            // the engine rebuild it. A builder that cannot be
            // constructed (e.g. no character skeleton) degrades to byte-exact reuse rather
            // than dropping the aggregate that shares this transaction.
            let mut builder = if base_profiles.is_empty() {
                None
            } else {
                match WeaponStanceBuilder::new(&base_profiles, src, base, &base_stance_root) {
                    Ok(builder) => Some(builder),
                    Err(error) => {
                        report.stance_builder_error = Some(error.to_string());
                        None
                    }
                }
            };
            for target in &targets {
                let body = if let Some(body) =
                    exact_base_stance_body(target, &base_profiles, &base_stance_root)?
                {
                    report.stance_reused += 1;
                    Some(body)
                } else if let Some(builder) = builder.as_mut() {
                    match builder.build(target) {
                        Ok(build) => {
                            report.stance_generated += 1;
                            Some(build.body)
                        }
                        Err(_) => {
                            report.stance_skipped += 1;
                            None
                        }
                    }
                } else {
                    report.stance_skipped += 1;
                    None
                };
                if let Some(body) = body {
                    files.push(AuthoritativeFile {
                        relative_path: PathBuf::from("AnimTextData/AnimationStanceData")
                            .join(format!("{}.txt", target.id)),
                        body,
                    });
                }
            }
        }

        if let Some(filename) = plugin_sync_anim_filename(target_plugin_name) {
            match build_plugin_sync_anim_data(subgraphs, src, base_meshes_root) {
                Ok(body) => files.push(AuthoritativeFile {
                    relative_path: PathBuf::from("AnimTextData/SyncAnimData").join(&filename),
                    body,
                }),
                Err(error) => {
                    // absent = graceful engine fallback; never fail the whole emission
                    eprintln!("plugin SyncAnimData skipped: {error}");
                }
            }
        }

        report.written = files.len() as u32;
        Ok((files, report))
    })();

    let (files, report) = match built {
        Ok(built) => built,
        Err(error) => {
            remove_owned_authoritative_outputs(out, &owned)?;
            return Err(error);
        }
    };
    if let Err(error) = stage_and_publish_authoritative_files(out, &owned, &files) {
        remove_owned_authoritative_outputs(out, &owned)?;
        return Err(error);
    }
    Ok(report)
}

fn emit_derivable_buckets_with_progress(
    subgraphs: &[SubgraphInput],
    weapon_subgraph_ids: &BTreeSet<u64>,
    idle_globs: &[String],
    event_candidates: &[String],
    src: &Path,
    out: &Path,
    base_meshes_root: Option<&Path>,
    mod_prefix: Option<&str>,
    progress: &mut dyn FnMut(&str),
) -> BucketEmissionReport {
    let atd = out.join("AnimTextData");
    let mut report = BucketEmissionReport::default();
    let unique_subgraphs = deduplicated_subgraphs(subgraphs);

    let mut by_race: BTreeMap<String, Vec<&SubgraphInput>> = BTreeMap::new();
    for sg in &unique_subgraphs {
        // The race's own dir wins: a humanoid creature's core behavior lives in the
        // shared `Actors\Character` tree, which would file it under the wrong race.
        if let Some(rd) = sg
            .race_dir
            .clone()
            .or_else(|| race_dir_of(&sg.core_behavior))
        {
            by_race.entry(rd).or_default().push(sg);
        }
    }

    let emit_race = |race_dir: &String, sgs: &Vec<&SubgraphInput>| {
        let race_started = Instant::now();
        let mut pending_files = PendingBucketFiles::default();
        // --- ClipGeneratorData: one file per distinct CORE behavior ---
        let mut seen_core: std::collections::HashSet<String> = std::collections::HashSet::new();
        // core (lc) -> AnimEventInfo clip targets (lc); the AnimationOffsets section1 set.
        let mut core_event_clips: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for sg in sgs {
            if weapon_subgraph_ids.contains(&sg.id()) {
                continue;
            }
            let core = &sg.core_behavior;
            if !seen_core.insert(core.to_ascii_lowercase()) {
                continue;
            }
            let core_file = src.join(core.replace('\\', "/"));
            if !core_file.is_file() {
                continue;
            }
            // --- ClipGeneratorData (binary) ---
            let entries = clip_generator_entries(&core_file);
            if !entries.is_empty() {
                let body = clip_generator_data_body(core, &entries);
                pending_files.push("ClipGeneratorData", format!("{}.txt", name_id(core)), &body);
            }
            // --- AnimEventInfo (event→clip table V2), keyed by name_id(core) ---
            // The byte-exact resolver (Snallygaster 15/15) maps each candidate event to
            // its clip(s); candidates that resolve to no clip here are dropped, so root
            // behaviors emit nothing (the engine rebuilds an absent file). Never an empty
            // file (a wrong/empty AnimEventInfo is worse than none).
            if !event_candidates.is_empty() {
                let events = resolve_anim_events(&core_file, event_candidates);
                if !events.is_empty() {
                    // The named event-clip set feeds AnimationOffsets section1.
                    let clips: BTreeSet<String> = events
                        .iter()
                        .flat_map(|e| e.clips.iter().map(|c| c.to_ascii_lowercase()))
                        .collect();
                    core_event_clips.insert(core.to_ascii_lowercase(), clips);
                    let body = anim_event_info_body(core, &events);
                    pending_files.push("AnimEventInfo", format!("{}.txt", name_id(core)), &body);
                }
            }
        }

        // --- DynamicIdleData: the race idle pool, replicated to every subgraph ---
        let race_prefix = format!("{}\\", race_dir.to_ascii_lowercase());
        let mut pool: Vec<String> = Vec::new();
        for g in idle_globs {
            if g.replace('/', "\\")
                .to_ascii_lowercase()
                .starts_with(&race_prefix)
            {
                pool.extend(expand_idle_glob(g, src));
            }
        }
        pool.sort();
        pool.dedup();
        if !pool.is_empty() {
            let body = dynamic_idle_data_body(&pool);
            for sg in sgs {
                if weapon_subgraph_ids.contains(&sg.id()) {
                    continue;
                }
                pending_files.push("DynamicIdleData", format!("{}.txt", sg.id()), &body);
                // Dual-emit the CK-canonical `BothLegs` key alongside the shipped
                // `boothlegs` typo: the engine opens the raw-SAPT (boothlegs) id at
                // runtime, but CK generates the typo-corrected `Injured\BothLegs` id. Same
                // body, both coexist. (RE: dynidle_6th.md TARGET_1, hash-proven.)
                if sg
                    .sapt_chain
                    .first()
                    .is_some_and(|f| f.to_ascii_lowercase().contains("\\injured\\boothlegs"))
                {
                    let first = &sg.sapt_chain[0];
                    let idx = first.to_ascii_lowercase().rfind("\\injured\\").unwrap();
                    let canonical_first = format!("{}\\Injured\\BothLegs", &first[..idx]);
                    let mut chain: Vec<&str> = vec![canonical_first.as_str()];
                    chain.extend(sg.sapt_chain[1..].iter().map(String::as_str));
                    let canonical_id = subgraph_id(&sg.core_behavior, &chain);
                    pending_files.push("DynamicIdleData", format!("{canonical_id}.txt"), &body);
                }
            }
        }

        // --- AnimationStanceData: count=1 creature camera-framing pose, PER SUBGRAPH ---
        // The byte-exact-validated emitter (MirelurkKing container byte-identical) samples
        // the creature skeleton + idle clip frame-0 (Head + torso pivot). Each subgraph's
        // pose comes from ITS OWN idle clip (the SAPT self-leaf dir) — injured-leg subgraphs
        // stand in a distinct limp/crouch, so sourcing the shared base idle is wrong for
        // files 2/3/4. Stance degrades gracefully when absent, so emit none on failure.
        // (RE: stance_pose_perSubgraph.md.)
        let race_disk = src.join(race_dir.replace('\\', "/"));
        // Head-tracking (declared by the core behavior) selects the 174 B converted-
        // creature StanceData vs the 124 B vanilla form; the gate is skeleton-/core-level
        // (not clip-level), so resolve it once from the representative core behavior.
        let head_tracking = sgs
            .iter()
            .find(|sg| {
                !weapon_subgraph_ids.contains(&sg.id())
                    && src.join(sg.core_behavior.replace('\\', "/")).is_file()
            })
            .map(|sg| behavior_wants_head_tracking(&src.join(sg.core_behavior.replace('\\', "/"))))
            .unwrap_or(false);
        for sg in sgs {
            if weapon_subgraph_ids.contains(&sg.id())
                || !src.join(sg.core_behavior.replace('\\', "/")).is_file()
            {
                continue;
            }
            if let Some(body) =
                emit_stance_for_subgraph(&race_disk, src, &sg.sapt_chain, head_tracking)
            {
                pending_files.push("AnimationStanceData", format!("{}.txt", sg.id()), &body);
            }
        }

        // --- AnimationOffsets: populated per-subgraph root motion. ---
        // A moving subgraph MUST ship non-empty trans/rot or the engine treats the
        // (empty) project-level cache as "no root motion" and the creature moonwalks.
        // Samples are keyframe-reduced by `reduce_lanes` (byte-exact selection/count/time
        // vs CK; values within ≤1 ULP). Keyed by subgraph id — distinct from the project-
        // level empty entry emitted below.
        // Weapon subgraphs (core only in the base game) resolve their clip set cross-file
        // through the same GraphResolver AnimationFileData uses. Each parallel job owns its
        // resolver so graph-cache mutation stays thread-local.
        let weapon_speed_info: BTreeMap<u64, (Vec<u8>, BTreeSet<String>)> = base_meshes_root
            .map(|base| {
                let roots = [src, base];
                let entries: Vec<_> =
                    sgs.par_iter()
                        .map(|sg| {
                            if !weapon_subgraph_ids.contains(&sg.id()) {
                                return None;
                            }
                            build_speed_info_body_weapon(&sg.core_behavior, &roots, &sg.sapt_chain)
                                .map(|body| {
                                    let loops = speed_info_leaf_basenames(
                                        &sg.core_behavior,
                                        &roots,
                                        &sg.sapt_chain,
                                    );
                                    (sg.id(), (body, loops))
                                })
                        })
                        .collect();
                entries.into_iter().flatten().collect()
            })
            .unwrap_or_default();
        let offset_files: Vec<_> = sgs
            .par_iter()
            .map_init(
                || {
                    base_meshes_root
                        .map(|base| GraphResolver::new(vec![src.to_path_buf(), base.to_path_buf()]))
                },
                |offsets_resolver, sg| {
                    let core_file = src.join(sg.core_behavior.replace('\\', "/"));
                    let body = if core_file.is_file() && !weapon_subgraph_ids.contains(&sg.id()) {
                        // Creature: self-contained single-file path (byte-exact, unchanged).
                        let empty_clips = BTreeSet::new();
                        let event_clips = core_event_clips
                            .get(&sg.core_behavior.to_ascii_lowercase())
                            .unwrap_or(&empty_clips);
                        build_subgraph_offsets_body(
                            &core_file,
                            &sg.core_behavior,
                            src,
                            &sg.sapt_chain,
                            event_clips,
                        )
                    } else if is_furniture_core_behavior(&sg.core_behavior) {
                        // Furniture cores live in the base game, so they reach here rather
                        // than the creature branch. They need their own builder: CK emits an
                        // offsets entry for every furniture subgraph, motion or not, and FO76
                        // furniture clips ship no baked reference frame.
                        offsets_resolver.as_mut().and_then(|resolver| {
                            build_subgraph_offsets_body_furniture(
                                resolver,
                                &sg.core_behavior,
                                &sg.sapt_chain,
                            )
                        })
                    } else if let (Some(base), Some(resolver)) =
                        (base_meshes_root, offsets_resolver.as_mut())
                    {
                        let empty_loops = BTreeSet::new();
                        let loops = weapon_speed_info
                            .get(&sg.id())
                            .map(|(_, loops)| loops)
                            .unwrap_or(&empty_loops);
                        build_subgraph_offsets_body_weapon(
                            resolver,
                            &sg.core_behavior,
                            &[src, base],
                            &sg.sapt_chain,
                            loops,
                        )
                    } else {
                        None
                    };
                    body.map(|body| PendingBucketFile {
                        bucket: "AnimationOffsets",
                        filename: format!("{}.txt", sg.id()),
                        body,
                    })
                },
            )
            .collect();
        pending_files
            .files
            .extend(offset_files.into_iter().flatten());

        // --- AnimationSpeedInfo: per-subgraph locomotion speed contour (generative). ---
        // The tree is the locomotion SM's generator sub-tree (SM children by stateId, unary
        // collapsed, no-speed pruned); each leaf's value/direction come from ITS subgraph's
        // SAPT-resolved loop clip's binary root motion. Keyed by subgraph id; non-locomotion
        // subgraphs (no contour) emit nothing. (RE: speedinfo_generate.md, weapon_path.md §6a.)
        //
        // CREATURE (core in the mod) → byte-exact single-file path, clips in the mod only.
        // Weapon output is precomputed before Offsets so the two caches switch ownership
        // atomically: a failed SpeedInfo build leaves every locomotion loop in Offsets.
        for sg in sgs {
            let core_file = src.join(sg.core_behavior.replace('\\', "/"));
            let body = if core_file.is_file() && !weapon_subgraph_ids.contains(&sg.id()) {
                build_speed_info_body(&core_file, &[src], &sg.sapt_chain)
            } else {
                weapon_speed_info
                    .get(&sg.id())
                    .map(|(body, _)| body.clone())
            };
            if let Some(body) = body {
                pending_files.push("AnimationSpeedInfo", format!("{}.txt", sg.id()), &body);
            }
        }

        // --- SyncAnimData: creature empty forms stay per project. Generated weapon
        // SyncAnimData remains absent until its CK representation is exact. ---
        if let Some(creature) = sgs.iter().find(|sg| {
            !weapon_subgraph_ids.contains(&sg.id())
                && src.join(sg.core_behavior.replace('\\', "/")).is_file()
        }) {
            // Ordinary creatures keep the core-path-derived name so their authored case
            // (`ScorchBeast`) survives; the race dir comes from a lowercased `ANAM` path
            // and is only authoritative when the core path names a different race — the
            // humanoid case, where it is the sole source of the project name.
            if let Some(race_name) = race_name_of(&creature.core_behavior)
                .filter(|_| {
                    race_dir_of(&creature.core_behavior)
                        .is_some_and(|dir| dir.eq_ignore_ascii_case(race_dir))
                })
                .or_else(|| race_name_of_dir(race_dir))
            {
                // Un-prefixed = the real on-disk FO76 project → existing-project G=1 (`V4\n1\n`).
                pending_files.push(
                    "SyncAnimData",
                    format!("ResolvedSyncAnimData{race_name}.txt"),
                    &sync_anim_data_body_existing(),
                );
                // CK, run on the converted mod, also emits the mod-prefixed project identity
                // (`{MOD_PREFIX}_{race}`). The path-derived name never carries the prefix —
                // conversion doesn't prefix `Actors\<Race>\` mesh paths — so the second file
                // needs the prefix threaded in from the boundary. (RE: count_gaps.md 1→2.)
                if let Some(prefix) = mod_prefix.filter(|p| !p.is_empty()) {
                    pending_files.push(
                        "SyncAnimData",
                        format!("ResolvedSyncAnimData{prefix}_{race_name}.txt"),
                        &sync_anim_data_body(),
                    );
                }
            }
        }

        // --- Main project manifest + project-level entries (root tables / offsets / idle) ---
        let creature = sgs.iter().find(|sg| {
            !weapon_subgraph_ids.contains(&sg.id())
                && src.join(sg.core_behavior.replace('\\', "/")).is_file()
        });
        if let Some((proj_name, files)) =
            creature.and_then(|_| extract_project_manifest(race_dir, src))
        {
            if !files.is_empty() {
                pending_files.push(
                    "AnimationFileData",
                    format!("{}.txt", proj_name.to_ascii_lowercase()),
                    &project_manifest_body(&proj_name, &files),
                );

                // The ROOT behavior (files[0]) gets the EMPTY AnimEventInfo +
                // ClipGeneratorData forms, keyed by name_id(root). CK ships these even
                // though the root carries real (death) clips — its cached root tables are
                // empty. Always the empty form (never run the resolvers on the root).
                // (RE: count_gaps.md — AnimEventInfo 1→2, ClipGeneratorData 1→2.)
                let root_full = format!("{race_dir}\\{}", files[0]);
                pending_files.push(
                    "AnimEventInfo",
                    format!("{}.txt", name_id(&root_full)),
                    &anim_event_info_body(&root_full, &[]),
                );
                pending_files.push(
                    "ClipGeneratorData",
                    format!("{}.txt", name_id(&root_full)),
                    &clip_generator_data_body(&root_full, &[]),
                );

                // The project-level empty AnimationOffsets entry is keyed by the
                // project .hkx path and its body references the ROOT behavior
                // (files[0]). Byte-exact and genuinely empty in CK — safe to ship
                // alongside the independently emitted populated per-subgraph offsets.
                if let Some(proj_rel) =
                    creature.and_then(|sg| project_hkx_relpath(&sg.core_behavior, src))
                {
                    pending_files.push(
                        "AnimationOffsets",
                        format!("{}.txt", name_id(&proj_rel)),
                        &animation_offsets_empty_body(&root_full),
                    );
                    // CK also keys the idle pool under the project name-id, in addition to
                    // the per-subgraph SAPT ids. (RE: count_gaps.md — DynamicIdleData 4→5;
                    // the canonical BothLegs/extra-A files are CK source-skew, not derivable
                    // from the shipped `boothlegs`-spelled RACE.)
                    if !pool.is_empty() {
                        pending_files.push(
                            "DynamicIdleData",
                            format!("{}.txt", name_id(&proj_rel)),
                            &dynamic_idle_data_body(&pool),
                        );
                    }
                }
            }
        }
        DerivableRaceEmission {
            files: pending_files,
            elapsed_seconds: race_started.elapsed().as_secs_f64(),
        }
    };

    let races: Vec<_> = by_race.iter().collect();
    let race_count = races.len();
    for (race_index, &(race_dir, sgs)) in races.iter().enumerate() {
        progress(&format!(
            "derivable race {}/{}: {} ({} subgraph(s))",
            race_index + 1,
            race_count,
            race_dir,
            sgs.len(),
        ));
    }

    // Workers stage bodies so the ordered flush preserves serial overwrite and progress semantics.
    let race_emissions: Vec<_> = races
        .par_iter()
        .map(|race| {
            let (race_dir, sgs) = *race;
            emit_race(race_dir, sgs)
        })
        .collect();

    for (race_index, ((race_dir, _), emission)) in races.into_iter().zip(race_emissions).enumerate()
    {
        let written_before = report.written;
        let write_started = Instant::now();
        emission.files.write_to(&atd, &mut report);
        let elapsed_seconds = emission.elapsed_seconds + write_started.elapsed().as_secs_f64();
        progress(&format!(
            "derivable race {}/{}: {} wrote {} file(s) in {:.1}s",
            race_index + 1,
            race_count,
            race_dir,
            report.written - written_before,
            elapsed_seconds,
        ));
    }

    // --- FX project manifests (Meshes\UniqueBehaviors\<name>fx) ---
    // Independent of the actor race tree: every FX project carried by the mod gets
    // its flag-0 named manifest. ProjectName casing follows the on-disk dir (the
    // FO76 authoring case; BA2-lowercased sources differ from CK only on that line,
    // which is runtime-irrelevant — the lookup is case-insensitive).
    for fx_name in fx_project_dirs(src) {
        if let Some((proj_name, files)) = extract_fx_manifest(src, &fx_name) {
            if !files.is_empty() {
                report.write(
                    &atd,
                    "AnimationFileData",
                    &format!("{}.txt", proj_name.to_ascii_lowercase()),
                    &project_manifest_body(&proj_name, &files),
                );
            }
        }
    }

    report
}

#[cfg(test)]
mod tests {
    use super::super::graph::StancePerspective;
    use super::super::stance::{StanceFormKey, WeaponRaceFamily, WeaponSraf};
    use super::*;
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    fn make_target_profile(core: &str, sapt: &[&str]) -> WeaponProfileInput {
        let subgraph = SubgraphInput {
            core_behavior: core.to_string(),
            sapt_chain: sapt.iter().map(|s| s.to_string()).collect(),
            race_dir: None,
        };
        let id = subgraph.id();
        WeaponProfileInput {
            subgraph,
            stance: WeaponSubgraphMetadata {
                race_family: WeaponRaceFamily {
                    owner_race: StanceFormKey {
                        plugin: "SeventySix.esm".to_string(),
                        local: 0x00D191,
                    },
                    sadd: None,
                },
                perspective: StancePerspective::ThirdPerson,
                sakd: Vec::new(),
                stkd: Vec::new(),
                core_behavior: core.to_string(),
                sapt: sapt.iter().map(|s| s.to_string()).collect(),
                sraf: WeaponSraf {
                    role: 0,
                    perspective: 0,
                },
                id,
            },
        }
    }

    /// A converted creature ships its core behavior in the mod; a weapon graft's
    /// core lives only in the base game. Creature subgraphs classified as weapon
    /// grafts skip every derivable creature bucket (stance/speed/sync/event) —
    /// the whole-plugin T-pose regression.
    #[test]
    fn in_mod_core_behaviors_are_not_weapon_subgraphs() {
        let dir = tempfile::tempdir().unwrap();
        let creature_core = r"Actors\Snallygaster\Behaviors\SnallygasterCoreBehavior.hkx";
        let disk = dir
            .path()
            .join("Actors/Snallygaster/Behaviors/SnallygasterCoreBehavior.hkx");
        std::fs::create_dir_all(disk.parent().unwrap()).unwrap();
        std::fs::write(&disk, b"hkx").unwrap();

        let creature = make_target_profile(creature_core, &[r"Actors\Snallygaster\Animations"]);
        let weapon = make_target_profile(
            r"Actors\Character\Behaviors\RifleWrappingBehavior.hkx",
            &[r"Weapons\TestGun"],
        );

        let ids = collect_weapon_subgraph_ids(
            &[creature.clone(), weapon.clone()],
            "SeventySix.esm",
            dir.path(),
        );
        assert!(
            !ids.contains(&creature.subgraph.id()),
            "creature subgraph (core in mod) must not be weapon-classified"
        );
        assert!(
            ids.contains(&weapon.subgraph.id()),
            "base-game weapon graft must stay weapon-classified"
        );
    }

    /// A humanoid creature (scorched, mole miner) mounts the SHARED
    /// `Actors\Character\Behaviors\*` cores, so its core path names `Character`, not the
    /// race — but its project lives under its own `Actors\<Race>` dir. Keying the project
    /// manifest off the core path looks in `Actors\Character`, finds no project there, and
    /// emits nothing: the race ships no AnimTextData at all and the actor can only idle.
    #[test]
    fn humanoid_creature_project_manifest_follows_race_dir_not_core_path() {
        let src = tempfile::tempdir().unwrap();
        let write = |rel: &str| {
            let p = src.path().join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, b"not a packfile").unwrap();
        };
        // Shared core, shipped by the mod — names `Character`, not the race.
        write("Actors/Character/Behaviors/GunBehavior.hkx");
        // The race's own project + root behavior.
        write("Actors/Scorched/ScorchedProject.hkx");
        write("Actors/Scorched/Behaviors/ScorchedRootBehavior.hkx");

        let subgraphs = vec![SubgraphInput {
            core_behavior: r"Actors\Character\Behaviors\GunBehavior.hkx".to_string(),
            sapt_chain: vec![r"Actors\Scorched\Animations".to_string()],
            race_dir: Some(r"Actors\Scorched".to_string()),
        }];

        let out = tempfile::tempdir().unwrap();
        emit_derivable_buckets_with_progress(
            &subgraphs,
            &BTreeSet::new(),
            &[],
            &[],
            src.path(),
            out.path(),
            None,
            None,
            &mut |_| {},
        );

        let bucket = out.path().join("AnimTextData/AnimationFileData");
        assert!(
            bucket.join("scorchedproject.txt").is_file(),
            "humanoid creature must get its own project manifest; emitted instead: {:?}",
            std::fs::read_dir(&bucket)
                .map(|d| d.flatten().map(|e| e.file_name()).collect::<Vec<_>>())
                .unwrap_or_default()
        );
    }

    /// The race dir is derived from the RACE's `ANAM` skeletal-model path, which FO76
    /// authors lowercase. An ordinary creature — whose core path already names its race —
    /// must keep the authored case (`ResolvedSyncAnimDataScorchBeast.txt`), or every
    /// existing creature's SyncAnimData filename churns.
    #[test]
    fn ordinary_creature_keeps_authored_case_when_race_dir_is_lowercased() {
        let src = tempfile::tempdir().unwrap();
        let core = src
            .path()
            .join("Actors/ScorchBeast/Behaviors/ScorchBeastCoreBehavior.hkx");
        std::fs::create_dir_all(core.parent().unwrap()).unwrap();
        std::fs::write(&core, b"not a packfile").unwrap();

        let subgraphs = vec![SubgraphInput {
            core_behavior: r"Actors\ScorchBeast\Behaviors\ScorchBeastCoreBehavior.hkx".to_string(),
            sapt_chain: vec![r"Actors\ScorchBeast\Animations".to_string()],
            race_dir: Some(r"actors\scorchbeast".to_string()),
        }];

        let out = tempfile::tempdir().unwrap();
        emit_derivable_buckets_with_progress(
            &subgraphs,
            &BTreeSet::new(),
            &[],
            &[],
            src.path(),
            out.path(),
            None,
            None,
            &mut |_| {},
        );

        // Compare the directory entry itself: `is_file()` is case-insensitive on NTFS and
        // would accept the lowercased name this test exists to reject.
        let emitted: Vec<String> = std::fs::read_dir(out.path().join("AnimTextData/SyncAnimData"))
            .map(|dir| {
                dir.flatten()
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        assert!(
            emitted
                .iter()
                .any(|name| name == "ResolvedSyncAnimDataScorchBeast.txt"),
            "authored case must survive; emitted instead: {emitted:?}"
        );
    }

    #[test]
    fn derivable_parallelism_preserves_multi_race_collision_order() {
        let src = tempfile::tempdir().unwrap();
        for race in ["B21_Foo", "Foo"] {
            let core = src
                .path()
                .join(format!("Actors/{race}/Behaviors/CoreBehavior.hkx"));
            std::fs::create_dir_all(core.parent().unwrap()).unwrap();
            std::fs::write(core, b"not a packfile").unwrap();
        }
        let subgraphs: Vec<_> = ["B21_Foo", "Foo"]
            .into_iter()
            .flat_map(|race| {
                ["A", "B"].into_iter().map(move |variant| SubgraphInput {
                    core_behavior: format!(r"Actors\{race}\Behaviors\CoreBehavior.hkx"),
                    sapt_chain: vec![format!(r"Actors\{race}\Animations\{variant}")],
                    race_dir: None,
                })
            })
            .collect();

        let run = |threads: usize, out: &Path| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap()
                .install(|| {
                    let mut messages = Vec::new();
                    let report = emit_derivable_buckets_with_progress(
                        &subgraphs,
                        &BTreeSet::new(),
                        &[],
                        &[],
                        src.path(),
                        out,
                        None,
                        Some("B21"),
                        &mut |message| messages.push(message.to_string()),
                    );
                    (report, messages)
                })
        };

        let serial_out = tempfile::tempdir().unwrap();
        let parallel_out = tempfile::tempdir().unwrap();
        let (serial_report, serial_messages) = run(1, serial_out.path());
        let (parallel_report, parallel_messages) = run(4, parallel_out.path());

        assert_eq!(serial_report.written, 4);
        assert_eq!(parallel_report.written, serial_report.written);
        assert_eq!(parallel_report.by_bucket, serial_report.by_bucket);
        let sync_files = [
            "ResolvedSyncAnimDataB21_B21_Foo.txt",
            "ResolvedSyncAnimDataB21_Foo.txt",
            "ResolvedSyncAnimDataFoo.txt",
        ];
        for filename in sync_files {
            let relative = Path::new("AnimTextData/SyncAnimData").join(filename);
            assert_eq!(
                std::fs::read(serial_out.path().join(&relative)).unwrap(),
                std::fs::read(parallel_out.path().join(&relative)).unwrap(),
                "{filename} differs by thread count"
            );
        }
        assert_eq!(
            std::fs::read(
                parallel_out
                    .path()
                    .join("AnimTextData/SyncAnimData/ResolvedSyncAnimDataB21_Foo.txt")
            )
            .unwrap(),
            sync_anim_data_body(),
            "later Foo prefixed output must win the cross-race filename collision"
        );

        let race_order = |messages: Vec<String>| {
            messages
                .into_iter()
                .filter(|message| message.starts_with("derivable race"))
                .map(|message| {
                    if message.contains(r"Actors\B21_Foo") {
                        "B21_Foo"
                    } else {
                        "Foo"
                    }
                })
                .collect::<Vec<_>>()
        };
        let expected_order = vec!["B21_Foo", "Foo", "B21_Foo", "Foo"];
        assert_eq!(race_order(serial_messages), expected_order);
        assert_eq!(race_order(parallel_messages), expected_order);
    }

    fn base_meshes() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../extracted/fo4/Meshes")
    }

    #[test]
    fn creature_only_authoritative_phase_succeeds_without_base_inputs() {
        let out = tempfile::tempdir().unwrap();
        let subgraphs = vec![SubgraphInput {
            core_behavior: r"Actors\Creature\Behaviors\CreatureBehavior.hkx".to_string(),
            sapt_chain: vec![r"Actors\Creature\Animations".to_string()],
            race_dir: None,
        }];

        let report = emit_serialized_production_buckets(
            &subgraphs,
            &[],
            &[],
            "Creature.esp",
            out.path(),
            out.path(),
            None,
        )
        .unwrap();

        // 1 = the plugin-level ResolvedSyncAnimDataCreature.txt empty form (V4\n0\n),
        // now always emitted alongside the authoritative buckets.
        assert_eq!(report.written, 1);
        assert!(out.path().join(AUTHORITATIVE_MANIFEST).is_file());
        assert!(
            !out.path()
                .join("AnimTextData/AnimationOffsets/PersistantSubgraphInfoAndOffsetData.txt")
                .exists()
        );
    }

    #[test]
    fn manifest_removes_retired_archetypes_and_stance_ids_only() {
        let out = tempfile::tempdir().unwrap();
        let first = vec![
            AuthoritativeFile {
                relative_path: PathBuf::from(
                    "AnimTextData/AnimationOffsets/PersistantSubgraphInfoAndOffsetData.txt",
                ),
                body: b"aggregate-1".to_vec(),
            },
            AuthoritativeFile {
                relative_path: PathBuf::from(
                    "AnimTextData/SyncAnimData/ResolvedSyncAnimDataalpha.txt",
                ),
                body: b"alpha".to_vec(),
            },
            AuthoritativeFile {
                relative_path: PathBuf::from(
                    "AnimTextData/SyncAnimData/ResolvedSyncAnimDatabeta.txt",
                ),
                body: b"beta".to_vec(),
            },
            AuthoritativeFile {
                relative_path: PathBuf::from("AnimTextData/AnimationStanceData/10.txt"),
                body: b"stance-10".to_vec(),
            },
            AuthoritativeFile {
                relative_path: PathBuf::from("AnimTextData/AnimationStanceData/20.txt"),
                body: b"stance-20".to_vec(),
            },
        ];
        stage_and_publish_authoritative_files(
            out.path(),
            &OwnedAuthoritativeFiles::load(out.path()).unwrap(),
            &first,
        )
        .unwrap();
        let unrelated = out.path().join("AnimTextData/SyncAnimData/unrelated.txt");
        std::fs::write(&unrelated, b"keep").unwrap();

        let second = vec![
            AuthoritativeFile {
                relative_path: PathBuf::from(
                    "AnimTextData/AnimationOffsets/PersistantSubgraphInfoAndOffsetData.txt",
                ),
                body: b"aggregate-2".to_vec(),
            },
            AuthoritativeFile {
                relative_path: PathBuf::from(
                    "AnimTextData/SyncAnimData/ResolvedSyncAnimDataalpha.txt",
                ),
                body: b"alpha-2".to_vec(),
            },
            AuthoritativeFile {
                relative_path: PathBuf::from("AnimTextData/AnimationStanceData/10.txt"),
                body: b"stance-10-2".to_vec(),
            },
        ];
        let owned = OwnedAuthoritativeFiles::load(out.path()).unwrap();
        stage_and_publish_authoritative_files(out.path(), &owned, &second).unwrap();

        assert!(
            !out.path()
                .join("AnimTextData/SyncAnimData/ResolvedSyncAnimDatabeta.txt")
                .exists()
        );
        assert!(
            !out.path()
                .join("AnimTextData/AnimationStanceData/20.txt")
                .exists()
        );
        assert_eq!(std::fs::read(unrelated).unwrap(), b"keep");
    }

    fn synthetic_weapon_inputs() -> (Vec<SubgraphInput>, Vec<WeaponProfileInput>) {
        let subgraphs = vec![
            SubgraphInput {
                core_behavior: r"Actors\Character\Behaviors\MissingWeaponBehavior.hkx".to_string(),
                sapt_chain: vec![r"Actors\Character\Animations\Weapon\Missing".to_string()],
                race_dir: None,
            },
            SubgraphInput {
                core_behavior: r"Actors\Character\_1stPerson\Behaviors\MissingGunBehavior.hkx"
                    .to_string(),
                sapt_chain: vec![r"Actors\Character\_1stPerson\Animations\Missing".to_string()],
                race_dir: None,
            },
        ];
        let profiles = subgraphs
            .iter()
            .enumerate()
            .map(|(index, subgraph)| WeaponProfileInput {
                subgraph: subgraph.clone(),
                stance: WeaponSubgraphMetadata {
                    race_family: WeaponRaceFamily {
                        owner_race: StanceFormKey {
                            plugin: "Test.esp".to_string(),
                            local: 1,
                        },
                        sadd: None,
                    },
                    perspective: if index == 0 {
                        StancePerspective::ThirdPerson
                    } else {
                        StancePerspective::FirstPerson
                    },
                    sakd: (index == 0)
                        .then(|| StanceFormKey {
                            plugin: "Fallout4.esm".to_string(),
                            local: 1,
                        })
                        .into_iter()
                        .collect(),
                    stkd: Vec::new(),
                    core_behavior: subgraph.core_behavior.clone(),
                    sapt: subgraph.sapt_chain.clone(),
                    sraf: WeaponSraf {
                        role: 1,
                        perspective: u16::from(index != 0),
                    },
                    id: subgraph.id(),
                },
            })
            .collect();
        (subgraphs, profiles)
    }

    fn synthetic_stance_metadata(
        id: u64,
        owner_plugin: &str,
        core_behavior: &str,
        sapt: &[&str],
    ) -> WeaponSubgraphMetadata {
        WeaponSubgraphMetadata {
            race_family: WeaponRaceFamily {
                owner_race: StanceFormKey {
                    plugin: owner_plugin.to_string(),
                    local: 1,
                },
                sadd: None,
            },
            perspective: StancePerspective::FirstPerson,
            sakd: Vec::new(),
            stkd: Vec::new(),
            core_behavior: core_behavior.to_string(),
            sapt: sapt.iter().map(|path| (*path).to_string()).collect(),
            sraf: WeaponSraf {
                role: 1,
                perspective: 1,
            },
            id,
        }
    }

    #[test]
    fn exact_base_stance_id_reuses_file_when_graph_identity_matches() {
        let root = tempfile::tempdir().unwrap();
        let target = synthetic_stance_metadata(
            42,
            "Target.esm",
            r"Actors\Character\_1stPerson\Behaviors\2HM_MeleeWrappingBehavior.hkx",
            &[
                r"Actors\Character\_1stPerson\Animations\Paired",
                r"Actors\Character\_1stPerson\Animations\2HM",
            ],
        );
        let base = synthetic_stance_metadata(
            42,
            "Fallout4.esm",
            r"actors/character/_1stperson/behaviors/2hm_meleewrappingbehavior.hkx",
            &[
                r"actors/character/_1stperson/animations/paired",
                r"actors/character/_1stperson/animations/2hm",
            ],
        );
        std::fs::write(root.path().join("42.txt"), b"base stance").unwrap();

        let body = exact_base_stance_body(&target, &[base], root.path()).unwrap();
        assert_eq!(body.as_deref(), Some(b"base stance".as_slice()));
    }

    #[test]
    fn exact_base_stance_id_rejects_hash_collision_with_different_graph_identity() {
        let root = tempfile::tempdir().unwrap();
        let target = synthetic_stance_metadata(42, "Target.esm", "target.hkx", &["target"]);
        let base = synthetic_stance_metadata(42, "Fallout4.esm", "base.hkx", &["base"]);

        let error = exact_base_stance_body(&target, &[base], root.path()).unwrap_err();
        assert!(error.contains("id collision 42"), "{error}");
    }

    fn seed_stale_authoritative_outputs(out: &Path, profiles: &[WeaponProfileInput]) {
        let atd = out.join("AnimTextData");
        std::fs::create_dir_all(atd.join("AnimationOffsets")).unwrap();
        std::fs::create_dir_all(atd.join("SyncAnimData")).unwrap();
        std::fs::create_dir_all(atd.join("AnimationStanceData")).unwrap();
        std::fs::write(
            atd.join("AnimationOffsets/PersistantSubgraphInfoAndOffsetData.txt"),
            b"stale",
        )
        .unwrap();
        std::fs::write(
            atd.join("SyncAnimData/ResolvedSyncAnimDatamissing.txt"),
            b"stale",
        )
        .unwrap();
        for profile in profiles {
            std::fs::write(
                atd.join(format!("AnimationStanceData/{}.txt", profile.stance.id)),
                b"stale",
            )
            .unwrap();
        }
        std::fs::write(atd.join("SyncAnimData/unrelated.txt"), b"keep").unwrap();
    }

    fn assert_owned_authoritative_absent(out: &Path, profiles: &[WeaponProfileInput]) {
        let atd = out.join("AnimTextData");
        assert!(
            !atd.join("AnimationOffsets/PersistantSubgraphInfoAndOffsetData.txt")
                .exists()
        );
        assert!(
            !atd.join("SyncAnimData/ResolvedSyncAnimDatamissing.txt")
                .exists()
        );
        for profile in profiles {
            assert!(
                !atd.join(format!("AnimationStanceData/{}.txt", profile.stance.id))
                    .exists()
            );
        }
        assert_eq!(
            std::fs::read(atd.join("SyncAnimData/unrelated.txt")).unwrap(),
            b"keep"
        );
    }

    #[test]
    fn aggregate_failures_remove_only_owned_authoritative_outputs() {
        let src = tempfile::tempdir().unwrap();
        let (subgraphs, profiles) = synthetic_weapon_inputs();
        for invalid in [false, true] {
            let base = tempfile::tempdir().unwrap();
            if invalid {
                let aggregate = base
                    .path()
                    .join("AnimTextData/AnimationOffsets/PersistantSubgraphInfoAndOffsetData.txt");
                std::fs::create_dir_all(aggregate.parent().unwrap()).unwrap();
                std::fs::write(aggregate, b"invalid aggregate").unwrap();
            }
            let out = tempfile::tempdir().unwrap();
            seed_stale_authoritative_outputs(out.path(), &profiles);
            let error = emit_serialized_production_buckets(
                &subgraphs,
                &profiles,
                &[],
                "Test.esp",
                src.path(),
                out.path(),
                Some(base.path()),
            )
            .unwrap_err();
            assert!(
                error.contains(if invalid { "invalid" } else { "missing" }),
                "{error}"
            );
            assert_owned_authoritative_absent(out.path(), &profiles);
        }
    }

    #[test]
    fn generated_weapon_sync_and_stance_are_withheld_and_stale_files_are_removed() {
        let source_base = base_meshes();
        let aggregate_source = source_base
            .join("AnimTextData/AnimationOffsets/PersistantSubgraphInfoAndOffsetData.txt");
        if !aggregate_source.is_file() {
            eprintln!("base aggregate fixture absent; skipping");
            return;
        }
        let src = tempfile::tempdir().unwrap();
        let base = tempfile::tempdir().unwrap();
        let aggregate = base
            .path()
            .join("AnimTextData/AnimationOffsets/PersistantSubgraphInfoAndOffsetData.txt");
        std::fs::create_dir_all(aggregate.parent().unwrap()).unwrap();
        std::fs::copy(aggregate_source, &aggregate).unwrap();
        let donor = WeaponSubgraphMetadata {
            race_family: WeaponRaceFamily {
                owner_race: StanceFormKey {
                    plugin: "Fallout4.esm".to_string(),
                    local: 2,
                },
                sadd: None,
            },
            perspective: StancePerspective::ThirdPerson,
            sakd: Vec::new(),
            stkd: Vec::new(),
            core_behavior: "donor.hkx".to_string(),
            sapt: vec!["donor".to_string()],
            sraf: WeaponSraf {
                role: 1,
                perspective: 0,
            },
            id: 42,
        };
        let donor_file = base.path().join("AnimTextData/AnimationStanceData/42.txt");
        std::fs::create_dir_all(donor_file.parent().unwrap()).unwrap();
        std::fs::write(donor_file, b"file-backed donor").unwrap();

        let (subgraphs, profiles) = synthetic_weapon_inputs();
        let out = tempfile::tempdir().unwrap();
        seed_stale_authoritative_outputs(out.path(), &profiles);
        let report = emit_serialized_production_buckets(
            &subgraphs,
            &profiles,
            &[donor],
            "Test.esp",
            src.path(),
            out.path(),
            Some(base.path()),
        )
        .unwrap();
        // 2 = the trusted aggregate + the plugin-level ResolvedSyncAnimDataTest.txt empty
        // form (V4\n0\n), now always emitted alongside the authoritative buckets.
        assert_eq!(report.written, 2, "only the trusted aggregate + plugin sync file are emitted");
        let atd = out.path().join("AnimTextData");
        assert!(
            atd.join("AnimationOffsets/PersistantSubgraphInfoAndOffsetData.txt")
                .is_file()
        );
        assert!(
            !atd.join("SyncAnimData/ResolvedSyncAnimDatamissing.txt")
                .exists()
        );
        for profile in &profiles {
            assert!(
                !atd.join(format!("AnimationStanceData/{}.txt", profile.stance.id))
                    .exists()
            );
        }
        assert_eq!(
            std::fs::read(atd.join("SyncAnimData/unrelated.txt")).unwrap(),
            b"keep"
        );
    }

    #[test]
    fn structural_aggregates_write_dirlists_and_merged_single_file() {
        let out = tempfile::tempdir().unwrap();
        let base = tempfile::tempdir().unwrap();

        // fabricate our out tree: one clipgen file + one filedata file
        let clipgen_dir = out.path().join("AnimTextData").join("ClipGeneratorData");
        std::fs::create_dir_all(&clipgen_dir).unwrap();
        let our_body = clip_generator_data_body(r"Actors\Mod\Behaviors\ModBehavior.hkx", &[]);
        let our_key = name_id(r"actors\mod\behaviors\modbehavior.hkx");
        std::fs::write(clipgen_dir.join(format!("{our_key}.txt")), &our_body).unwrap();
        let filedata_dir = out.path().join("AnimTextData").join("AnimationFileData");
        std::fs::create_dir_all(&filedata_dir).unwrap();
        std::fs::write(filedata_dir.join("123.txt"), b"x").unwrap();

        // fabricate a tiny "vanilla" singlefile in the base root: 1 block-A entry, 0 block-B
        let vanilla_entry_body =
            clip_generator_data_body(r"Actors\Base\Behaviors\BaseBehavior.hkx", &[]);
        let vanilla = super::single_file::emit_single_file(&super::single_file::SingleFile {
            block_a: vec![super::single_file::SingleFileEntry {
                key: name_id(r"actors\base\behaviors\basebehavior.hkx") as u64,
                body: vanilla_entry_body,
            }],
            block_b: vec![],
        });
        let base_atd = base.path().join("AnimTextData");
        std::fs::create_dir_all(&base_atd).unwrap();
        std::fs::write(
            base_atd.join("behaviorclipinformationandsubgraphanimationoffsetssinglefile.txt"),
            &vanilla,
        )
        .unwrap();

        let written =
            emit_structural_aggregates(out.path(), Some(base.path()), &mut |_| {}).unwrap();
        assert_eq!(written, 2); // 1 dirlist (AnimationFileData) + 1 singlefile; no clipgen dirlist

        let merged = std::fs::read(
            out.path()
                .join("AnimTextData")
                .join("behaviorclipinformationandsubgraphanimationoffsetssinglefile.txt"),
        )
        .unwrap();
        let parsed = super::single_file::parse_single_file(&merged).unwrap();
        assert_eq!(parsed.block_a.len(), 2); // vanilla + ours
        assert_eq!(parsed.block_a[1].key, our_key as u64);
        assert!(out
            .path()
            .join("AnimTextData")
            .join("AnimationFileData")
            .join("dirlist.txt")
            .is_file());
    }
}
