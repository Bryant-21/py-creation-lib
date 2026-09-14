//! End-to-end AnimTextData emission (CK-free): writes bucket files for a set of
//! subgraphs, deriving each id from the RACE fields (core behavior + SAPT chain) and each
//! file list from the behavior graph plus on-disk SAPT resolution.
//!
//! Also writes the project-wide Offsets aggregate and per-combo weapon StanceData (base
//! bytes reused where the combo is a vanilla subgraph, else generated from the converted
//! skeleton).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use rayon::prelude::*;

use super::behavior_index::{
    add_direct_sapt_hkx_files, behavior_uses_dynamic_animation_tags, resolve_subgraph_files,
};
use super::bucket_files::{
    anim_event_info_body, animation_file_data_body, animation_offsets_empty_body,
    clip_generator_data_body, dynamic_idle_data_body, project_manifest_body, sync_anim_data_body,
    sync_anim_data_body_existing,
};
use super::core::{name_id, subgraph_id};
use super::event_resolver::resolve_anim_events;
use super::extract::{
    clip_generator_entries, expand_idle_glob, extract_fx_manifest, extract_project_manifest,
    fx_project_dirs, project_hkx_relpath_for_race_dir, race_dir_of, race_name_of, race_name_of_dir,
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
use super::sync::{
    build_plugin_sync_anim_data, plugin_sync_anim_filename, weapon_sync_anim_filenames,
};

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

// Converted weapon subgraphs can ship their own core. Creature body graphs use SRAF
// role 1 too, but only weapon subgraphs are selected by weapon keywords (STKD), and
// the bodies still require the single-file creature writers.
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
        && ((profile.stance.sraf.role == 1 && !profile.stance.stkd.is_empty())
            || !src_meshes_root
                .join(profile.stance.core_behavior.replace('\\', "/"))
                .is_file())
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
    // Repair aliased clips FIRST: BOTH offsets builders now enforce the name-identity guard,
    // which is a faithful reproduction of the format and drops anything still aliased, so this
    // has to land before any bucket is read off these graphs. Every entry point — including
    // `creature_closure` — funnels through here.
    match super::align_clip_names::align_clip_generator_names(inputs, src_meshes_root) {
        Ok(0) => {}
        Ok(n) => progress(&format!("aligned {n} clip generator name(s)")),
        // Advisory: a clip that stays aliased is dropped by the guard, which is the status quo
        // for it — not a reason to abort AnimTextData generation.
        Err(error) => progress(&format!("clip alignment skipped: {error}")),
    }
    // Then give sweep-window attacks the `HitFrame` FO4 needs to time them. Order matters:
    // alignment can rename the animation a generator plays, and the HitFrame pass reads that
    // animation to decide whether the hit is already annotated there.
    match super::synth_hitframe::synthesize_missing_hit_frames(inputs, src_meshes_root) {
        Ok(0) => {}
        Ok(n) => progress(&format!("synthesized {n} HitFrame trigger(s)")),
        // Advisory for the same reason as above: no HitFrame means that attack stays
        // unusable, which is the status quo, not a reason to abort.
        Err(error) => progress(&format!("HitFrame synthesis skipped: {error}")),
    }
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

    progress(&format!(
        "authoritative timings: stance_prepare={:.3}s stance_build={:.3}s sync={:.3}s donors_prepared={}",
        authoritative.stance_prepare_seconds,
        authoritative.stance_build_seconds,
        authoritative.sync_seconds,
        authoritative.stance_donors_prepared,
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

    // The memoized packfiles/skeletons are worth hundreds of MB on a full conversion and
    // are useless past this point — the run continues into the asset phases.
    super::hkx_cache::clear_all();

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

const SINGLE_FILE_NAME: &str = "behaviorclipinformationandsubgraphanimationoffsetssinglefile.txt";

/// Off: mods must not ship dirlists or the singlefile. CK-built fan mods
/// (B21_PlasmaCaster, Snallygaster) carry neither, only per-bucket data files and a
/// plugin-level SyncAnimData file, and they work. A mod's singlefile is the merged-VFS
/// winner, so it shadows vanilla's entire clip table. The writers stay tested behind
/// this flag.
const EMIT_STRUCTURAL_AGGREGATES: bool = false;

/// Write CK-parity dirlists and the merged singlefile; returns files written. Must run
/// after every bucket writer, since it aggregates the final on-disk set. Gated off by
/// `EMIT_STRUCTURAL_AGGREGATES`.
///
/// A mod's singlefile fully shadows vanilla's, so it is vanilla's entries verbatim plus
/// our ClipGeneratorData entries. Without a vanilla singlefile none is emitted.
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
        None => {
            progress("singlefile: no vanilla source; skipping (engine falls back to vanilla's)")
        }
        Some(vanilla_path) => {
            let vanilla = std::fs::read(&vanilla_path)
                .map_err(|error| format!("failed to read {}: {error}", vanilla_path.display()))?;
            let clipgen_dir = out_meshes_root
                .join("AnimTextData")
                .join("ClipGeneratorData");
            let mut additions: Vec<(u32, Vec<u8>)> = Vec::new();
            if clipgen_dir.is_dir() {
                let mut keyed: Vec<(u32, PathBuf)> = Vec::new();
                for entry in std::fs::read_dir(&clipgen_dir)
                    .map_err(|error| format!("failed to list {}: {error}", clipgen_dir.display()))?
                {
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
                    let body = std::fs::read(&path)
                        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
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
/// `out_meshes_root`; returns the number written.
///
/// Weapon subgraphs, furniture cores, cores outside the mod, and cores with behavior
/// references use the recursive graph walk (a FO76 wrapper in the mod may still depend on
/// the shared character graph). Other local cores use the self-contained creature
/// resolver. Subgraphs that resolve to zero files are skipped: the engine rebuilds an
/// absent file at load but trusts an empty one.
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
    // Subgraphs are deduplicated by id, so each iteration owns its output file and the
    // pass is order-independent — parallel over subgraphs, one resolver per worker (the
    // resolver's caches are `&mut self`, so they stay thread-local; the underlying
    // packfile parses are shared through the process-wide memo).
    let written: u32 = deduplicated_subgraphs(subgraphs)
        .par_iter()
        .map_init(
            || GraphResolver::new(roots.clone()),
            |resolver, sg| -> std::io::Result<u32> {
                let id = sg.id();
                let core_in_mod = src_meshes_root
                    .join(sg.core_behavior.replace('\\', "/"))
                    .is_file();
                let has_behavior_references = core_in_mod
                    && super::hkx_cache::behavior_packfile(
                        &src_meshes_root.join(sg.core_behavior.replace('\\', "/")),
                    )
                    .is_some_and(|hkx| hkx.objects().iter().any(|object| {
                        object.class_name == "hkbBehaviorReferenceGenerator"
                    }));
                // A local core can contain both clips and references (FO76 MT -> Dialogue).
                // A nonempty clip-only manifest still strands those referenced graphs.
                let mut files = if core_in_mod
                    && !weapon_subgraph_ids.contains(&id)
                    && !is_furniture_core_behavior(&sg.core_behavior)
                    && !has_behavior_references
                {
                    let core_file = src_meshes_root.join(sg.core_behavior.replace('\\', "/"));
                    resolve_subgraph_files(&core_file, src_meshes_root, &sg.sapt_chain)
                        .into_iter()
                        .filter(|relative| roots.iter().any(|root| {
                            root.join(relative.replace('\\', "/")).is_file()
                        }))
                        .collect()
                } else {
                    resolver.resolve_body(&sg.core_behavior, &sg.sapt_chain)
                };
                // A core with no direct clips (e.g. a reference-only wrapping graph such as
                // `GraftonCore_InjuredWrappingBehavior.hkx`) falls through to the cross-file
                // resolver, which follows the reference into the referenced core and lists it
                // plus its SAPT-resolved clips, as CK caches.
                if files.is_empty() && core_in_mod && !weapon_subgraph_ids.contains(&id) {
                    files = resolver.resolve_body(&sg.core_behavior, &sg.sapt_chain);
                }
                let core_file = if core_in_mod {
                    Some(src_meshes_root.join(sg.core_behavior.replace('\\', "/")))
                } else {
                    base_meshes_root
                        .map(|root| root.join(sg.core_behavior.replace('\\', "/")))
                        .filter(|path| path.is_file())
                };
                if let (Some(core_file), Some(sapt)) = (core_file, sg.sapt_chain.first())
                    && behavior_uses_dynamic_animation_tags(&core_file)
                {
                    // CK keeps the resolver's core-first dependency order
                    // (vanilla SuperMutant melee lists MeleeBehavior.hkx first);
                    // a BTreeSet round-trip here alphabetized the rows and
                    // buried the stance core mid-list. Append the direct-SAPT
                    // files without disturbing the resolved order.
                    let mut seen: std::collections::HashSet<String> =
                        files.iter().map(|f| super::graph::norm_key(f)).collect();
                    let mut direct: BTreeSet<String> = BTreeSet::new();
                    add_direct_sapt_hkx_files(&mut direct, src_meshes_root, sapt);
                    for file in direct {
                        if seen.insert(super::graph::norm_key(&file)) {
                            files.push(file);
                        }
                    }
                }
                if files.is_empty() {
                    // safe-skip: let the engine rebuild rather than ship an empty body
                    return Ok(0);
                }
                let body = animation_file_data_body(id, &files);
                std::fs::write(bucket_dir.join(format!("{id}.txt")), body)?;
                Ok(1)
            },
        )
        .collect::<std::io::Result<Vec<u32>>>()?
        .into_iter()
        .sum();
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
    timings: Vec<(&'static str, f64)>,
    workers: usize,
}

#[derive(Debug, Default)]
pub struct AuthoritativeEmissionReport {
    pub written: u32,
    pub stance_donors_prepared: usize,
    pub stance_reused: u32,
    pub stance_generated: u32,
    pub stance_skipped: u32,
    pub stance_builder_error: Option<String>,
    pub stance_prepare_seconds: f64,
    pub stance_build_seconds: f64,
    pub sync_seconds: f64,
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

            let prepare_started = Instant::now();
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
            let base_bodies: Vec<_> = targets
                .iter()
                .map(|target| exact_base_stance_body(target, &base_profiles, &base_stance_root))
                .collect::<Result<_, _>>()?;
            let generated_targets: Vec<_> = targets
                .iter()
                .zip(&base_bodies)
                .filter_map(|(target, body)| body.is_none().then_some(target))
                .collect();
            // Unavailable donors withhold generated stance so the engine can rebuild
            // it; trusted base bytes and the offsets aggregate still publish.
            let mut builder = if base_profiles.is_empty() || generated_targets.is_empty() {
                None
            } else {
                match WeaponStanceBuilder::for_targets(
                    &base_profiles,
                    &generated_targets,
                    src,
                    base,
                    &base_stance_root,
                ) {
                    Ok(builder) => {
                        report.stance_donors_prepared = builder.donor_count();
                        Some(builder)
                    }
                    Err(error) => {
                        report.stance_builder_error = Some(error.to_string());
                        None
                    }
                }
            };
            report.stance_prepare_seconds = prepare_started.elapsed().as_secs_f64();
            let build_started = Instant::now();
            for (target, base_body) in targets.iter().zip(base_bodies) {
                let body = if let Some(body) = base_body {
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
            report.stance_build_seconds = build_started.elapsed().as_secs_f64();
        }

        let sync_started = Instant::now();
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
        report.sync_seconds = sync_started.elapsed().as_secs_f64();

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
        let mut step_started = Instant::now();
        let mut timings = Vec::new();
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
            // A race that ships its own project.hkx is a real creature: replicate the idle
            // pool to ALL its blocks, weapon-classified or not (vanilla SuperMutant and
            // FO76's own ATD both cover weapon blocks). Weapon-classification only means
            // "true graft" when the race dir ships no project (e.g. Actors\Character).
            let race_ships_project = project_hkx_relpath_for_race_dir(race_dir, src).is_some();
            for sg in sgs {
                if weapon_subgraph_ids.contains(&sg.id()) && !race_ships_project {
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

        timings.push((
            "core_events_and_idles",
            step_started.elapsed().as_secs_f64(),
        ));
        step_started = Instant::now();
        // --- AnimationStanceData: count=1 creature camera-framing pose, per subgraph ---
        // Samples the creature skeleton at idle clip frame 0 (Head + torso pivot). Each
        // subgraph uses its own idle clip (SAPT self-leaf dir), since injured-leg subgraphs
        // stand in a distinct limp/crouch. Stance degrades gracefully when absent, so a
        // failure emits nothing. (RE: stance_pose_perSubgraph.md.)
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
        // Parallel per subgraph; the ordered flush below keeps emission order identical to
        // the serial form.
        let stance_bodies: Vec<_> = sgs
            .par_iter()
            .map(|sg| {
                if weapon_subgraph_ids.contains(&sg.id())
                    || !src.join(sg.core_behavior.replace('\\', "/")).is_file()
                {
                    return None;
                }
                emit_stance_for_subgraph(&race_disk, src, &sg.sapt_chain, head_tracking)
                    .map(|body| (sg.id(), body))
            })
            .collect();
        for (id, body) in stance_bodies.into_iter().flatten() {
            pending_files.push("AnimationStanceData", format!("{id}.txt"), &body);
        }

        // --- AnimationOffsets: populated per-subgraph root motion. ---
        // A moving subgraph must ship non-empty trans/rot, or the engine treats the empty
        // project-level cache as "no root motion" and the creature moonwalks. Keyed by
        // subgraph id, distinct from the project-level empty entry emitted below. Weapon
        // subgraphs (core only in the base game) resolve their clip set cross-file through
        // the GraphResolver AnimationFileData uses; each parallel job owns its resolver so
        // graph-cache mutation stays thread-local.
        timings.push(("stance", step_started.elapsed().as_secs_f64()));
        step_started = Instant::now();
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
        timings.push(("weapon_speed_info", step_started.elapsed().as_secs_f64()));
        step_started = Instant::now();
        let offset_files: Vec<_> = sgs
            .par_iter()
            .map_init(
                || {
                    GraphResolver::new(
                        std::iter::once(src.to_path_buf())
                            .chain(base_meshes_root.map(Path::to_path_buf))
                            .collect(),
                    )
                },
                |offsets_resolver, sg| {
                    let core_file = src.join(sg.core_behavior.replace('\\', "/"));
                    let body = if is_furniture_core_behavior(&sg.core_behavior) {
                        // Furniture requires offsets even when a mod supplies the core and
                        // its clips have no extracted motion.
                        build_subgraph_offsets_body_furniture(
                            offsets_resolver,
                            &sg.core_behavior,
                            &sg.sapt_chain,
                        )
                    } else if core_file.is_file() && !weapon_subgraph_ids.contains(&sg.id()) {
                        // Creature: self-contained single-file path (byte-exact).
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
                    } else if let Some(base) = base_meshes_root {
                        let empty_loops = BTreeSet::new();
                        let loops = weapon_speed_info
                            .get(&sg.id())
                            .map(|(_, loops)| loops)
                            .unwrap_or(&empty_loops);
                        build_subgraph_offsets_body_weapon(
                            offsets_resolver,
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
        // collapsed, no-speed pruned); each leaf's value/direction come from its subgraph's
        // SAPT-resolved loop clip's binary root motion. Keyed by subgraph id; non-locomotion
        // subgraphs (no contour) emit nothing. (RE: speedinfo_generate.md, weapon_path.md.)
        //
        // Creatures (core in the mod) use the byte-exact single-file path, mod clips only.
        // Weapon output is precomputed before Offsets so the two caches switch ownership
        // atomically: a failed SpeedInfo build leaves every locomotion loop in Offsets.
        timings.push(("offsets", step_started.elapsed().as_secs_f64()));
        step_started = Instant::now();
        let speed_bodies: Vec<_> = sgs
            .par_iter()
            .map(|sg| {
                let core_file = src.join(sg.core_behavior.replace('\\', "/"));
                let body = if core_file.is_file() && !weapon_subgraph_ids.contains(&sg.id()) {
                    build_speed_info_body(&core_file, &[src], &sg.sapt_chain)
                } else {
                    weapon_speed_info
                        .get(&sg.id())
                        .map(|(body, _)| body.clone())
                };
                body.map(|body| (sg.id(), body))
            })
            .collect();
        for (id, body) in speed_bodies.into_iter().flatten() {
            pending_files.push("AnimationSpeedInfo", format!("{id}.txt"), &body);
        }

        timings.push(("creature_speed_info", step_started.elapsed().as_secs_f64()));
        step_started = Instant::now();
        // --- SyncAnimData: creature empty forms stay per project. Generated weapon
        // SyncAnimData remains absent until its CK representation is exact. ---
        // The seed is the RACE's own project on disk, not a subgraph whose core the mod ships:
        // a humanoid creature mounts the shared `Actors\Character\Behaviors\*` graphs, so it has
        // no in-mod core and would emit nothing at all (mole miner, scorched).
        let in_mod_core = sgs.iter().find(|sg| {
            !weapon_subgraph_ids.contains(&sg.id())
                && src.join(sg.core_behavior.replace('\\', "/")).is_file()
        });
        if in_mod_core.is_some() || project_hkx_relpath_for_race_dir(race_dir, src).is_some() {
            // Ordinary creatures keep the core-path-derived name so their authored case
            // (`ScorchBeast`) survives; the race dir comes from a lowercased `ANAM` path
            // and is only authoritative when the core path names a different race — the
            // humanoid case, where it is the sole source of the project name.
            if let Some(race_name) = in_mod_core
                .and_then(|creature| {
                    race_name_of(&creature.core_behavior).filter(|_| {
                        race_dir_of(&creature.core_behavior)
                            .is_some_and(|dir| dir.eq_ignore_ascii_case(race_dir))
                    })
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
        // `extract_project_manifest` is its own gate: it returns None unless the mod ships this
        // race's project .hkx. Humanoids have no in-mod core but still ship their project,
        // character and skeleton.
        if let Some((proj_name, files)) = extract_project_manifest(race_dir, src) {
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
                if let Some(proj_rel) = project_hkx_relpath_for_race_dir(race_dir, src) {
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
        timings.push(("project_metadata", step_started.elapsed().as_secs_f64()));
        DerivableRaceEmission {
            files: pending_files,
            elapsed_seconds: race_started.elapsed().as_secs_f64(),
            timings,
            workers: rayon::current_num_threads(),
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

    // Workers stage bodies so the ordered flush preserves serial overwrite and progress
    // semantics. Races are scheduled biggest first: the weapon/character race holds most
    // subgraphs and would otherwise run alone at the end. Emission order is restored before
    // the flush. The slowest race can take minutes, so workers report completions over a
    // channel that the calling thread (the only one that may touch `progress`) drains live.
    let total_subgraphs: usize = races.iter().map(|(_, sgs)| sgs.len()).sum();
    let mut schedule: Vec<usize> = (0..races.len()).collect();
    schedule.sort_by_key(|&index| std::cmp::Reverse(races[index].1.len()));

    let (sender, receiver) = std::sync::mpsc::channel::<(usize, DerivableRaceEmission)>();
    let race_emissions = rayon::in_place_scope(|scope| {
        let races = &races;
        let emit_race = &emit_race;
        scope.spawn(move |_| {
            schedule.par_iter().for_each_with(sender, |sender, &index| {
                let (race_dir, sgs) = races[index];
                let emission = emit_race(race_dir, sgs);
                let _ = sender.send((index, emission));
            });
        });

        let mut done_races = 0usize;
        let mut done_subgraphs = 0usize;
        let mut emissions = Vec::with_capacity(races.len());
        loop {
            let (index, emission) =
                match receiver.recv_timeout(std::time::Duration::from_millis(25)) {
                    Ok(result) => result,
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        // The callback stays on its caller; a one-worker pool must still execute its queued work.
                        rayon::yield_now();
                        continue;
                    }
                };
            done_races += 1;
            done_subgraphs += races[index].1.len();
            progress(&format!(
                "derivable: {done_races}/{race_count} race(s), \
                 {done_subgraphs}/{total_subgraphs} subgraph(s) built \
                 (just finished {})",
                races[index].0,
            ));
            emissions.push((index, emission));
        }
        emissions.sort_by_key(|(index, _)| *index);
        emissions
            .into_iter()
            .map(|(_, emission)| emission)
            .collect::<Vec<_>>()
    });

    for (race_index, ((race_dir, _), emission)) in races.into_iter().zip(race_emissions).enumerate()
    {
        let written_before = report.written;
        let write_started = Instant::now();
        emission.files.write_to(&atd, &mut report);
        let write_seconds = write_started.elapsed().as_secs_f64();
        let elapsed_seconds = emission.elapsed_seconds + write_seconds;
        let timings = emission
            .timings
            .iter()
            .map(|(name, seconds)| format!("{name}={seconds:.3}s"))
            .collect::<Vec<_>>()
            .join(" ");
        progress(&format!(
            "derivable timings: {race_dir} workers={} {timings} write={write_seconds:.3}s",
            emission.workers
        ));
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

    #[test]
    #[ignore = "requires local converted and base game fixtures"]
    fn authoritative_corpus_equivalence() {
        let config_path = std::env::var("MODKIT_ANIM_CORPUS").unwrap();
        let config: serde_json::Value =
            serde_json::from_slice(&std::fs::read(config_path).unwrap()).unwrap();
        let path = |key: &str| PathBuf::from(config[key].as_str().unwrap());
        let inputs = super::super::race_decode::subgraph_inputs_from_plugin(
            &path("plugin"),
            "fo4",
            &[path("base_plugin")],
        )
        .unwrap();
        let out = tempfile::tempdir().unwrap();
        let started = Instant::now();
        let report = emit_serialized_production_buckets(
            &deduplicated_subgraphs(&inputs.subgraphs),
            &inputs.weapon_profiles,
            &inputs.base_stance_profiles,
            &inputs.target_plugin_name,
            &path("meshes"),
            out.path(),
            Some(&path("base_meshes")),
        )
        .unwrap();
        eprintln!(
            "authoritative corpus: {report:?} elapsed={:.6}s profiles={} base_profiles={}",
            started.elapsed().as_secs_f64(),
            inputs.weapon_profiles.len(),
            inputs.base_stance_profiles.len()
        );
        assert_eq!(report.written, 4);
        assert_eq!(report.stance_reused, 1);
        assert_eq!(report.stance_generated, 1);
        assert_eq!(report.stance_skipped, 0);
        let manifest: AuthoritativeManifest = serde_json::from_slice(
            &std::fs::read(out.path().join(AUTHORITATIVE_MANIFEST)).unwrap(),
        )
        .unwrap();
        let record = std::env::var_os("MODKIT_ANIM_RECORD").is_some();
        for relative in manifest
            .files
            .iter()
            .map(String::as_str)
            .chain(std::iter::once(AUTHORITATIVE_MANIFEST))
        {
            let body = std::fs::read(out.path().join(relative)).unwrap();
            let expected = path("baseline").join(relative);
            if record {
                std::fs::create_dir_all(expected.parent().unwrap()).unwrap();
                std::fs::write(expected, body).unwrap();
            } else {
                assert!(
                    std::fs::read(expected).unwrap() == body,
                    "different bytes: {relative}"
                );
            }
        }
    }

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

        let mut creature_body = creature.clone();
        creature_body.stance.sraf.role = 1;
        assert!(
            !is_target_weapon_profile(&creature_body, "SeventySix.esm", dir.path()),
            "role-1 creature body without weapon keywords must stay a creature"
        );

        let mut local_weapon = creature_body.clone();
        local_weapon.stance.stkd = vec![StanceFormKey {
            plugin: "SeventySix.esm".to_string(),
            local: 0x00D192,
        }];
        assert!(is_target_weapon_profile(&local_weapon, "SeventySix.esm", dir.path()));
        assert!(!is_target_weapon_profile(&local_weapon, "Another.esm", dir.path()));
    }

    #[test]
    fn local_furniture_core_emits_stationary_animation_offsets() {
        use havok_native::hkx::descriptors::DescriptorRegistry;
        use havok_native::hkx::types::HkxValue;
        use havok_native::hkx::{HkxFile, HkxMember, HkxObject, write_hkx};

        let src = tempfile::tempdir().unwrap();
        let base = tempfile::tempdir().unwrap();
        let core = r"Actors\Character\Behaviors\CustomFurniture.hkx";
        let sapt = r"Actors\Character\Animations\CustomWorkbench";
        let graph = HkxFile::from_tagxml(
            11,
            "hk_2014.1.0-r1",
            vec![HkxObject {
                name: Some("#0001".to_string()),
                offset: 0,
                signature: 0,
                class_name: "hkbClipGenerator".to_string(),
                members: vec![
                    HkxMember {
                        name: "name".to_string(),
                        value: HkxValue::String {
                            value: "Standing Enter".to_string(),
                            is_null: false,
                        },
                    },
                    HkxMember {
                        name: "animationName".to_string(),
                        value: HkxValue::String {
                            value: r"Animations\EnterFromStand.hkt".to_string(),
                            is_null: false,
                        },
                    },
                ],
            }],
        );
        let core_file = src.path().join(core.replace('\\', "/"));
        std::fs::create_dir_all(core_file.parent().unwrap()).unwrap();
        let mut registry = DescriptorRegistry::for_contents_version("hk_2014.1.0-r1");
        std::fs::write(core_file, write_hkx(&graph, &mut registry)).unwrap();
        let clip = src.path().join(sapt.replace('\\', "/")).join("EnterFromStand.hkx");
        std::fs::create_dir_all(clip.parent().unwrap()).unwrap();
        std::fs::write(clip, b"clip without extracted motion").unwrap();
        let subgraph = SubgraphInput {
            core_behavior: core.to_string(),
            sapt_chain: vec![sapt.to_string()],
            race_dir: Some(r"Actors\Character".to_string()),
        };
        for base_root in [None, Some(base.path())] {
            let out = tempfile::tempdir().unwrap();
            emit_animation_file_data(
                std::slice::from_ref(&subgraph), src.path(), out.path(), base_root,
            ).unwrap();
            let manifest = std::fs::read_to_string(out.path().join("AnimTextData/AnimationFileData")
                .join(format!("{}.txt", subgraph.id()))).unwrap();
            assert_eq!(manifest.lines().nth(4), Some(core));
            emit_derivable_buckets_with_progress(
                std::slice::from_ref(&subgraph),
                &BTreeSet::new(),
                &[],
                &[],
                src.path(),
                out.path(),
                base_root,
                None,
                &mut |_| {},
            );
            let body = std::fs::read(out.path().join("AnimTextData/AnimationOffsets")
                .join(format!("{}.txt", subgraph.id())))
                .expect("local stationary furniture must have offsets, with or without base meshes");
            assert!(body.windows(14).any(|bytes| bytes == b"EnterFromStand"));
            assert!(!body.windows(14).any(|bytes| bytes == b"Standing Enter"));
        }
    }

    #[test]
    fn base_dynamic_furniture_behavior_includes_all_direct_override_clips() {
        use havok_native::hkx::descriptors::DescriptorRegistry;
        use havok_native::hkx::types::HkxValue;
        use havok_native::hkx::{HkxFile, HkxMember, HkxObject, write_hkx};

        let src = tempfile::tempdir().unwrap();
        let base = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        let core_behavior = r"Actors\Character\Behaviors\WorkbenchFurnitureBehavior.hkx";
        let core_file = base.path().join(core_behavior.replace('\\', "/"));
        std::fs::create_dir_all(core_file.parent().unwrap()).unwrap();
        let hkx = HkxFile::from_tagxml(
            11,
            "hk_2014.1.0-r1",
            vec![
                HkxObject {
                    name: Some("#0001".to_string()),
                    offset: 0,
                    signature: 0,
                    class_name: "hkbClipGenerator".to_string(),
                    members: vec![HkxMember {
                        name: "animationName".to_string(),
                        value: HkxValue::String {
                            value: r"Animations\PoseA_IdleFlavor2.hkt".to_string(),
                            is_null: false,
                        },
                    }],
                },
                HkxObject {
                    name: Some("#0002".to_string()),
                    offset: 0,
                    signature: 0,
                    class_name: "DynamicAnimationTaggingGenerator".to_string(),
                    members: Vec::new(),
                },
            ],
        );
        let mut registry = DescriptorRegistry::for_contents_version("hk_2014.1.0-r1");
        std::fs::write(&core_file, write_hkx(&hkx, &mut registry)).unwrap();

        let sapt = r"Actors\Character\Animations\Furniture\WorkbenchTinkers";
        for clip in [
            "PoseA_IdleFlavor1.hkx",
            "PoseA_IdleFlavor2.hkx",
            "PoseA_IdleFlavor3.hkx",
        ] {
            let path = src.path().join(sapt.replace('\\', "/")).join(clip);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"animation").unwrap();
        }

        let subgraph = SubgraphInput {
            core_behavior: core_behavior.to_string(),
            sapt_chain: vec![sapt.to_string()],
            race_dir: Some(r"Actors\Character".to_string()),
        };
        let written = emit_animation_file_data(
            std::slice::from_ref(&subgraph),
            src.path(),
            out.path(),
            Some(base.path()),
        )
        .unwrap();

        assert_eq!(written, 1);
        let body = std::fs::read_to_string(
            out.path()
                .join("AnimTextData/AnimationFileData")
                .join(format!("{}.txt", subgraph.id())),
        )
        .unwrap();
        for clip in [
            "PoseA_IdleFlavor1.hkx",
            "PoseA_IdleFlavor2.hkx",
            "PoseA_IdleFlavor3.hkx",
        ] {
            assert!(body.contains(clip), "{clip} missing from:\n{body}");
        }
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

    /// The realistic humanoid case: the shared core is NOT shipped by the mod (it lives in the
    /// base game), so every one of the race's subgraphs is weapon-classified and the old
    /// in-mod-core seed found nothing — dropping `<race>project.txt` even though the project,
    /// root behavior and character all shipped. The sibling test above writes the shared core
    /// into the mod, which no real conversion does, so it passed while production was broken.
    #[test]
    fn humanoid_creature_project_manifest_emitted_when_shared_core_is_base_game_only() {
        let src = tempfile::tempdir().unwrap();
        let write = |rel: &str| {
            let p = src.path().join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, b"not a packfile").unwrap();
        };
        // The race's own project + root behavior ship; the mounted core does NOT.
        write("Actors/MoleMiner/MoleMinerProject.hkx");
        write("Actors/MoleMiner/Behaviors/MoleMinerRootBehavior.hkx");

        let subgraphs = vec![SubgraphInput {
            core_behavior: r"Actors\Character\Behaviors\MTBehavior.hkx".to_string(),
            sapt_chain: vec![r"Actors\MoleMiner\Animations\MT".to_string()],
            race_dir: Some(r"Actors\MoleMiner".to_string()),
        }];
        // Production reality: core-not-in-mod => the block is weapon-classified.
        let weapon_ids: BTreeSet<u64> = subgraphs.iter().map(|sg| sg.id()).collect();

        let out = tempfile::tempdir().unwrap();
        emit_derivable_buckets_with_progress(
            &subgraphs,
            &weapon_ids,
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
            bucket.join("moleminerproject.txt").is_file(),
            "humanoid creature whose cores live in the base game must still get its project \
             manifest; emitted instead: {:?}",
            std::fs::read_dir(&bucket)
                .map(|d| d.flatten().map(|e| e.file_name()).collect::<Vec<_>>())
                .unwrap_or_default()
        );
        let sync = out.path().join("AnimTextData/SyncAnimData");
        assert!(
            sync.join("ResolvedSyncAnimDataMoleMiner.txt").is_file(),
            "…and its SyncAnimData; emitted instead: {:?}",
            std::fs::read_dir(&sync)
                .map(|d| d.flatten().map(|e| e.file_name()).collect::<Vec<_>>())
                .unwrap_or_default()
        );
    }

    /// DynamicIdleData replicates the race idle pool to every subgraph of a real creature
    /// race — vanilla FO4 (SuperMutant) and FO76's own ATD both ship it for weapon blocks
    /// too. A humanoid creature's blocks are all weapon-classified (cores live in the base
    /// game), which starved the bucket to 0/23 for MoleMiner. The race shipping its own
    /// project.hkx is what separates a creature block from a true weapon graft.
    #[test]
    fn humanoid_creature_dynamic_idle_data_reaches_weapon_classified_subgraphs() {
        let src = tempfile::tempdir().unwrap();
        let write = |rel: &str| {
            let p = src.path().join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, b"not a packfile").unwrap();
        };
        write("Actors/MoleMiner/MoleMinerProject.hkx");
        write("Actors/MoleMiner/Behaviors/MoleMinerRootBehavior.hkx");
        write("Actors/MoleMiner/Animations/PoseA_Idle1.hkx");
        // A true weapon graft's race dir ships no project.
        write("Actors/Character/Animations/PoseB_Idle1.hkx");

        let creature = SubgraphInput {
            core_behavior: r"Actors\Character\Behaviors\WeaponBehavior.hkx".to_string(),
            sapt_chain: vec![r"Actors\MoleMiner\Animations\GripAssault".to_string()],
            race_dir: Some(r"Actors\MoleMiner".to_string()),
        };
        let graft = SubgraphInput {
            core_behavior: r"Actors\Character\Behaviors\WeaponBehavior.hkx".to_string(),
            sapt_chain: vec![r"Actors\Character\Animations\Weapon\PepperShaker".to_string()],
            race_dir: Some(r"Actors\Character".to_string()),
        };
        let subgraphs = vec![creature, graft];
        let weapon_ids: BTreeSet<u64> = subgraphs.iter().map(|sg| sg.id()).collect();
        let idle_globs = vec![
            r"Actors\MoleMiner\Animations\PoseA_Idle*.hkx".to_string(),
            r"Actors\Character\Animations\PoseB_Idle*.hkx".to_string(),
        ];

        let out = tempfile::tempdir().unwrap();
        emit_derivable_buckets_with_progress(
            &subgraphs,
            &weapon_ids,
            &idle_globs,
            &[],
            src.path(),
            out.path(),
            None,
            None,
            &mut |_| {},
        );

        let bucket = out.path().join("AnimTextData/DynamicIdleData");
        assert!(
            bucket.join(format!("{}.txt", subgraphs[0].id())).is_file(),
            "shipped creature race must get DynamicIdleData on its weapon-classified blocks; \
             emitted instead: {:?}",
            std::fs::read_dir(&bucket)
                .map(|d| d.flatten().map(|e| e.file_name()).collect::<Vec<_>>())
                .unwrap_or_default()
        );
        assert!(
            !bucket.join(format!("{}.txt", subgraphs[1].id())).is_file(),
            "a true weapon graft (race dir ships no project.hkx) must stay skipped"
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

        assert!(
            serial_messages
                .iter()
                .any(|message| message.starts_with("derivable timings:")
                    && message.contains("workers=1 "))
        );
        assert!(
            parallel_messages
                .iter()
                .any(|message| message.starts_with("derivable timings:")
                    && message.contains("workers=4 "))
        );

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
        assert_eq!(
            report.written, 2,
            "only the trusted aggregate + plugin sync file are emitted"
        );
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
        assert!(
            out.path()
                .join("AnimTextData")
                .join("AnimationFileData")
                .join("dirlist.txt")
                .is_file()
        );
    }

    /// The engine loads a weapon-role stance from the subgraph's AnimationFileData
    /// manifest, and CK lists the stance's own core graph as the first behavior row
    /// (vanilla SuperMutant melee FileData starts with `MeleeBehavior.hkx`). Without it,
    /// shared-core creature stances (MoleMiner melee/MT/injured wrappers) never run.
    #[test]
    fn shared_core_subgraph_file_data_lists_core_behavior_first() {
        use havok_native::hkx::descriptors::DescriptorRegistry;
        use havok_native::hkx::types::HkxValue;
        use havok_native::hkx::{HkxFile, HkxMember, HkxObject, write_hkx};

        let src = tempfile::tempdir().unwrap();
        let base = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();

        // The shared core lives ONLY in the base game — the mole miner reality.
        let core_behavior = r"Actors\Character\Behaviors\MeleeBehavior.hkx";
        let core_file = base.path().join(core_behavior.replace('\\', "/"));
        std::fs::create_dir_all(core_file.parent().unwrap()).unwrap();
        let hkx = HkxFile::from_tagxml(
            11,
            "hk_2014.1.0-r1",
            vec![HkxObject {
                name: Some("#0001".to_string()),
                offset: 0,
                signature: 0,
                class_name: "hkbClipGenerator".to_string(),
                members: vec![HkxMember {
                    name: "animationName".to_string(),
                    value: HkxValue::String {
                        value: r"Animations\1HM\AttackForwardA.hkt".to_string(),
                        is_null: false,
                    },
                }],
            }],
        );
        let mut registry = DescriptorRegistry::for_contents_version("hk_2014.1.0-r1");
        std::fs::write(&core_file, write_hkx(&hkx, &mut registry)).unwrap();

        let clip = src
            .path()
            .join("Actors/MoleMiner/Animations/H2H/attackforwarda.hkx");
        std::fs::create_dir_all(clip.parent().unwrap()).unwrap();
        std::fs::write(clip, b"animation").unwrap();

        let subgraph = SubgraphInput {
            core_behavior: core_behavior.to_string(),
            sapt_chain: vec![
                r"Actors\MoleMiner\Animations\H2H".to_string(),
                r"Actors\MoleMiner\Animations\Shared".to_string(),
            ],
            race_dir: Some(r"Actors\MoleMiner".to_string()),
        };
        let written = emit_animation_file_data(
            std::slice::from_ref(&subgraph),
            src.path(),
            out.path(),
            Some(base.path()),
        )
        .unwrap();
        assert_eq!(written, 1);

        let body = std::fs::read_to_string(
            out.path()
                .join("AnimTextData/AnimationFileData")
                .join(format!("{}.txt", subgraph.id())),
        )
        .unwrap();
        let lines: Vec<&str> = body.lines().collect();
        let first_file_row = lines
            .iter()
            .position(|l| l.contains('\\'))
            .expect("body must list files");
        assert_eq!(
            lines[first_file_row], core_behavior,
            "CK lists the stance's own core graph first; body:\n{body}"
        );
        assert!(
            body.contains(r"Actors\MoleMiner\Animations\H2H\attackforwarda.hkx"),
            "clip row missing; body:\n{body}"
        );
    }

    #[test]
    fn local_core_manifest_omits_missing_clip_paths() {
        use havok_native::hkx::descriptors::DescriptorRegistry;
        use havok_native::hkx::types::HkxValue;
        use havok_native::hkx::{HkxFile, HkxMember, HkxObject, write_hkx};

        let src = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        let core = r"Actors\Fixture\MoleMiner\Behaviors\Gun.hkx";
        let core_file = src.path().join(core.replace('\\', "/"));
        std::fs::create_dir_all(core_file.parent().unwrap()).unwrap();
        let objects = [r"Animations\Present.hkt", r"..\Source\Missing.hkt"]
            .into_iter()
            .enumerate()
            .map(|(index, animation)| HkxObject {
                name: Some(format!("#{index:04}")),
                offset: 0,
                signature: 0,
                class_name: "hkbClipGenerator".to_string(),
                members: vec![HkxMember {
                    name: "animationName".to_string(),
                    value: HkxValue::String { value: animation.to_string(), is_null: false },
                }],
            })
            .collect();
        let hkx = HkxFile::from_tagxml(11, "hk_2014.1.0-r1", objects);
        let mut registry = DescriptorRegistry::for_contents_version("hk_2014.1.0-r1");
        std::fs::write(core_file, write_hkx(&hkx, &mut registry)).unwrap();
        let clip = r"Actors\Fixture\MoleMiner\Animations\Present.hkx";
        let clip_file = src.path().join(clip.replace('\\', "/"));
        std::fs::create_dir_all(clip_file.parent().unwrap()).unwrap();
        std::fs::write(clip_file, b"animation").unwrap();
        let subgraph = SubgraphInput {
            core_behavior: core.to_string(),
            sapt_chain: vec![r"Actors\Fixture\MoleMiner\Animations".to_string()],
            race_dir: Some(r"Actors\Fixture\MoleMiner".to_string()),
        };
        assert_eq!(emit_animation_file_data(std::slice::from_ref(&subgraph), src.path(), out.path(), None).unwrap(), 1);
        let body = std::fs::read_to_string(out.path().join(format!(
            "AnimTextData/AnimationFileData/{}.txt", subgraph.id()
        ))).unwrap();
        assert_eq!(body.lines().skip(4).collect::<Vec<_>>(), vec![clip]);
    }

    /// CK keeps the resolver's core-first dependency order even for cores carrying a
    /// `DynamicAnimationTaggingGenerator` (vanilla SuperMutant melee lists
    /// `MeleeBehavior.hkx` first, then its support graphs, not alphabetically).
    /// Appending the direct-SAPT files must not alphabetize the manifest.
    #[test]
    fn dynamic_tag_core_subgraph_keeps_core_first_and_appends_sapt_files() {
        assert_dynamic_subgraph_dependencies(false);
    }

    #[test]
    fn local_core_with_clips_keeps_referenced_behavior_in_manifest() {
        assert_dynamic_subgraph_dependencies(true);
    }

    fn assert_dynamic_subgraph_dependencies(core_in_mod: bool) {
        use havok_native::hkx::descriptors::DescriptorRegistry;
        use havok_native::hkx::types::HkxValue;
        use havok_native::hkx::{HkxFile, HkxMember, HkxObject, write_hkx};

        let src = tempfile::tempdir().unwrap();
        let base = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();

        let core_behavior = r"Actors\Character\Behaviors\MeleeBehavior.hkx";
        // Support behavior that sorts BEFORE the core alphabetically — the
        // Dialogue-vs-Melee reality that exposed the reordering.
        let support_behavior = r"Actors\Character\Behaviors\AaaSupportBehavior.hkx";
        let graph_root = if core_in_mod { src.path() } else { base.path() };

        let write_graph = |path: &std::path::Path, objects: Vec<HkxObject>| {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            let hkx = HkxFile::from_tagxml(11, "hk_2014.1.0-r1", objects);
            let mut registry = DescriptorRegistry::for_contents_version("hk_2014.1.0-r1");
            std::fs::write(path, write_hkx(&hkx, &mut registry)).unwrap();
        };

        write_graph(
            &graph_root.join(core_behavior.replace('\\', "/")),
            vec![
                HkxObject {
                    name: Some("#0001".to_string()),
                    offset: 0,
                    signature: 0,
                    class_name: "hkbClipGenerator".to_string(),
                    members: vec![HkxMember {
                        name: "animationName".to_string(),
                        value: HkxValue::String {
                            value: r"Animations\1HM\AttackForwardA.hkt".to_string(),
                            is_null: false,
                        },
                    }],
                },
                HkxObject {
                    name: Some("#0002".to_string()),
                    offset: 0,
                    signature: 0,
                    class_name: "hkbBehaviorReferenceGenerator".to_string(),
                    members: vec![HkxMember {
                        name: "behaviorName".to_string(),
                        value: HkxValue::String {
                            value: r"Behaviors\AaaSupportBehavior.hkx".to_string(),
                            is_null: false,
                        },
                    }],
                },
                HkxObject {
                    name: Some("#0003".to_string()),
                    offset: 0,
                    signature: 0,
                    class_name: "DynamicAnimationTaggingGenerator".to_string(),
                    members: vec![],
                },
            ],
        );
        write_graph(
            &graph_root.join(support_behavior.replace('\\', "/")),
            vec![HkxObject {
                name: Some("#0001".to_string()),
                offset: 0,
                signature: 0,
                class_name: "hkbClipGenerator".to_string(),
                members: vec![HkxMember {
                    name: "animationName".to_string(),
                    value: HkxValue::String {
                        value: r"Animations\1HM\SupportLoop.hkt".to_string(),
                        is_null: false,
                    },
                }],
            }],
        );

        for clip in ["attackforwarda.hkx", "supportloop.hkx", "extraloop.hkx"] {
            let p = src
                .path()
                .join("Actors/MoleMiner/Animations/H2H")
                .join(clip);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, b"animation").unwrap();
        }

        let subgraph = SubgraphInput {
            core_behavior: core_behavior.to_string(),
            sapt_chain: vec![
                r"Actors\MoleMiner\Animations\H2H".to_string(),
                r"Actors\MoleMiner\Animations\Shared".to_string(),
            ],
            race_dir: Some(r"Actors\MoleMiner".to_string()),
        };
        let written = emit_animation_file_data(
            std::slice::from_ref(&subgraph),
            src.path(),
            out.path(),
            Some(base.path()),
        )
        .unwrap();
        assert_eq!(written, 1);

        let body = std::fs::read_to_string(
            out.path()
                .join("AnimTextData/AnimationFileData")
                .join(format!("{}.txt", subgraph.id())),
        )
        .unwrap();
        let lines: Vec<&str> = body.lines().collect();
        assert!(lines.contains(&support_behavior), "referenced behavior missing: {body}");
        let first_file_row = lines
            .iter()
            .position(|l| l.contains('\\'))
            .expect("body must list files");
        assert_eq!(
            lines[first_file_row], core_behavior,
            "core graph must stay the first row despite dynamic-tag SAPT append; body:\n{body}"
        );
        assert!(
            body.contains(r"Actors\MoleMiner\Animations\H2H\extraloop.hkx"),
            "direct-SAPT file must still be appended; body:\n{body}"
        );
        // No duplicate rows after the append.
        let mut sorted = lines[first_file_row..].to_vec();
        sorted.sort_unstable();
        let before = sorted.len();
        sorted.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        assert_eq!(before, sorted.len(), "duplicate rows; body:\n{body}");
    }
}
