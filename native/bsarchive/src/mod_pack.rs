use crate::{
    FileFormat, Reader as _, fo4,
    incremental::DirectPackStats,
    pack::{self, PackEntrySpec},
    tes4,
};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use walkdir::WalkDir;

type PackResult<T> = Result<T, String>;

const ARCHIVE_HEADER_OVERHEAD: u64 = 4096;
const ENTRY_OVERHEAD: u64 = 512;
const COMPRESSIBLE_BA2_ESTIMATE_NUMERATOR: u64 = 2;
const COMPRESSIBLE_BA2_ESTIMATE_DENOMINATOR: u64 = 3;
const MAX_ARCHIVE_PACK_CONCURRENCY: usize = 2;
const MAX_TEXTURE_ARCHIVE_CONCURRENCY: usize = 2;
const MAX_TEXTURE_WORKERS_PER_ARCHIVE: usize = 16;
#[cfg(not(test))]
const ARCHIVE_PACK_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);
#[cfg(test)]
const ARCHIVE_PACK_HEARTBEAT_INTERVAL: Duration = Duration::from_millis(25);
const MAIN_FAMILY_ORDER: [ArchiveFamily; 10] = [
    ArchiveFamily::Lod,
    ArchiveFamily::Terrain,
    ArchiveFamily::Meshes,
    ArchiveFamily::Sounds,
    ArchiveFamily::Animations,
    ArchiveFamily::Scripts,
    ArchiveFamily::Strings,
    ArchiveFamily::Materials,
    ArchiveFamily::Interface,
    ArchiveFamily::Main,
];
const GENERATED_LABEL_BASES: &[&str] = &[
    "Main",
    "Textures",
    "LODTextures",
    "TerrainTextures",
    "MeshesExtra",
    "Misc",
    "LOD",
    "Terrain",
    "Meshes",
    "Sounds",
    "Animations",
    "Scripts",
    "Strings",
    "Materials",
    "Interface",
];

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum ArchiveFamily {
    Lod,
    Terrain,
    Meshes,
    Sounds,
    Animations,
    Scripts,
    Strings,
    Materials,
    Interface,
    Main,
    Textures,
}

#[derive(Clone, Debug)]
struct ArchiveEntry {
    relative_path: String,
    source_path: PathBuf,
    size: u64,
    family: ArchiveFamily,
}

#[derive(Clone, Debug)]
struct PlannedArchive {
    label: String,
    family: String,
    output_name: String,
    entries: Vec<ArchiveEntry>,
    texture_archive: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct PackArchivePlan {
    pub(crate) output_path: PathBuf,
    pub(crate) output_name: String,
    pub(crate) archive_type: String,
    pub(crate) entries: Vec<PackEntrySpec>,
    pub(crate) input_bytes: u64,
    pub(crate) texture_archive: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct PackModConfig {
    pub(crate) mod_name: String,
    pub(crate) mod_dir: PathBuf,
    pub(crate) data_dir: PathBuf,
    pub(crate) strings_dir: PathBuf,
    pub(crate) game: String,
    pub(crate) archive_ext: String,
    pub(crate) archive_cap: u64,
    pub(crate) expanded_archives: bool,
    pub(crate) pc: bool,
    pub(crate) xbox: bool,
    pub(crate) archive_workers: usize,
    pub(crate) manifest_path: Option<PathBuf>,
    pub(crate) dry_run: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct ArchiveSummary {
    pub(crate) platform: String,
    pub(crate) name: String,
    pub(crate) file_count: usize,
    pub(crate) bytes: u64,
    pub(crate) elapsed_secs: f64,
    pub(crate) direct_pack_stats: Option<DirectPackStats>,
}

#[derive(Clone, Debug)]
pub(crate) struct PackModResult {
    pub(crate) archives: Vec<ArchiveSummary>,
    pub(crate) inventory_elapsed_secs: f64,
    pub(crate) planning_elapsed_secs: f64,
}

#[derive(Clone, Debug)]
pub(crate) struct PackProgress {
    pub(crate) phase: &'static str,
    pub(crate) platform: String,
    pub(crate) message: String,
    pub(crate) completed: usize,
    pub(crate) total: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ArchiveWorkerAllocation {
    archive_concurrency: usize,
    workers_per_archive: usize,
    extra_worker_archives: usize,
}

struct ActiveArchiveProgress {
    plan_index: usize,
    output_path: PathBuf,
    started_at: Instant,
    sampled_at: Instant,
    output_bytes: u64,
    previous_output_bytes: Option<u64>,
}

trait ScheduledArchivePlan {
    fn output_name(&self) -> &str;
    fn file_count(&self) -> usize;
    fn input_bytes(&self) -> u64;
    fn texture_archive(&self) -> bool;
}

impl ScheduledArchivePlan for PlannedArchive {
    fn output_name(&self) -> &str {
        &self.output_name
    }

    fn file_count(&self) -> usize {
        self.entries.len()
    }

    fn input_bytes(&self) -> u64 {
        self.entries.iter().map(|entry| entry.size).sum()
    }

    fn texture_archive(&self) -> bool {
        self.texture_archive
    }
}

impl ScheduledArchivePlan for PackArchivePlan {
    fn output_name(&self) -> &str {
        &self.output_name
    }

    fn file_count(&self) -> usize {
        self.entries.len()
    }

    fn input_bytes(&self) -> u64 {
        self.input_bytes
    }

    fn texture_archive(&self) -> bool {
        self.texture_archive
    }
}

fn normalize_relative_path(relative_path: &str) -> String {
    relative_path
        .replace('\\', "/")
        .trim_start_matches('/')
        .to_string()
}

// Consumes a leading signed-integer run (e.g. "-110" or "77") and returns the
// remainder of the string after it, or None if the string doesn't start with
// one. Used by is_lodgen_quad_tile to walk the ".level.x.y" triple without a
// regex dependency.
fn consume_signed_int(s: &str) -> Option<&str> {
    let body = s.strip_prefix('-').unwrap_or(s);
    let digit_len = body.bytes().take_while(|b| b.is_ascii_digit()).count();
    if digit_len == 0 {
        return None;
    }
    Some(&body[digit_len..])
}

fn consume_signed_int_then_dot(s: &str) -> Option<&str> {
    consume_signed_int(s)?.strip_prefix('.')
}

// Matches lodgen's terrain-LOD tile naming: `<world>.<level>.<x>.<y>` (signed
// ints), then anything (an "_msn" normal-map suffix, a ".season" segment,
// etc.), then ".dds". Can't collide with convert_terrain output: its
// texture-set names go through safe_name, which replaces "." and "-" with "_".
fn is_lodgen_quad_tile(basename: &str) -> bool {
    let lower = basename.to_ascii_lowercase();
    if !lower.ends_with(".dds") {
        return false;
    }
    let Some(dot) = lower.find('.').filter(|&idx| idx > 0) else {
        return false;
    };
    let rest = &lower[dot + 1..];
    let Some(rest) = consume_signed_int_then_dot(rest) else {
        return false;
    };
    let Some(rest) = consume_signed_int_then_dot(rest) else {
        return false;
    };
    consume_signed_int(rest).is_some()
}

fn classify_archive_family(relative_path: &str) -> ArchiveFamily {
    let path = normalize_relative_path(relative_path);
    let lower = path.to_ascii_lowercase();
    // Strip a leading "data/" component, mirroring the Python planner's
    // classify_archive_family (archive_plan.py:62-64).
    let lower = if let Some(rest) = lower.strip_prefix("data/") {
        rest.to_string()
    } else if lower == "data" {
        String::new()
    } else {
        lower
    };
    let parts: Vec<&str> = lower.split('/').filter(|part| !part.is_empty()).collect();
    let suffix = Path::new(&lower)
        .extension()
        .map(|ext| format!(".{}", ext.to_string_lossy().to_ascii_lowercase()))
        .unwrap_or_default();

    // LOD shares Textures/Terrain with full-resolution land textures. Keep only
    // lodgen products in the LOD family; the remaining textures and materials
    // belong in the ordinary Textures/Materials archives.
    if parts.first() == Some(&"textures") && parts.get(1) == Some(&"terrain") {
        let basename = parts.last().copied().unwrap_or("");
        if parts.contains(&"objects") {
            return ArchiveFamily::Lod;
        }
        if parts.contains(&"lodgen") {
            return ArchiveFamily::Lod;
        }
        if is_lodgen_quad_tile(basename) {
            return ArchiveFamily::Lod;
        }
        return ArchiveFamily::Textures;
    }
    if parts.first() == Some(&"materials") && parts.get(1) == Some(&"terrain") {
        return ArchiveFamily::Materials;
    }

    if parts.first() == Some(&"textures") {
        return ArchiveFamily::Textures;
    }
    if parts.first() == Some(&"interface") {
        return ArchiveFamily::Interface;
    }
    if parts.first() == Some(&"materials") || matches!(suffix.as_str(), ".bgsm" | ".bgem") {
        return ArchiveFamily::Materials;
    }
    if parts.first() == Some(&"strings")
        || matches!(suffix.as_str(), ".strings" | ".dlstrings" | ".ilstrings")
    {
        return ArchiveFamily::Strings;
    }
    if matches!(parts.first(), Some(&"sound" | &"music"))
        || matches!(suffix.as_str(), ".xwm" | ".wav")
    {
        return ArchiveFamily::Sounds;
    }
    if suffix == ".hkx" || parts.contains(&"animations") {
        return ArchiveFamily::Animations;
    }
    if parts.first() == Some(&"scripts") || matches!(suffix.as_str(), ".pex" | ".psc") {
        return ArchiveFamily::Scripts;
    }
    if matches!(suffix.as_str(), ".bto" | ".btr") {
        return ArchiveFamily::Lod;
    }
    if parts.first() == Some(&"meshes") || suffix == ".nif" {
        return ArchiveFamily::Meshes;
    }
    ArchiveFamily::Main
}

fn estimate_with_ratio(size: u64, numerator: u64, denominator: u64) -> u64 {
    let estimated = ((size as u128) * (numerator as u128)).div_ceil(denominator as u128);
    estimated.min(u64::MAX as u128) as u64
}

fn uses_ba2_planning_estimates(archive_ext: &str) -> bool {
    archive_ext
        .trim_start_matches('.')
        .eq_ignore_ascii_case("ba2")
}

fn estimated_planned_data_size(entry: &ArchiveEntry, archive_ext: &str) -> u64 {
    if !uses_ba2_planning_estimates(archive_ext) {
        return entry.size;
    }
    match entry.family {
        ArchiveFamily::Lod
        | ArchiveFamily::Terrain
        | ArchiveFamily::Meshes
        | ArchiveFamily::Animations
        | ArchiveFamily::Scripts
        | ArchiveFamily::Strings
        | ArchiveFamily::Materials
        | ArchiveFamily::Interface
        | ArchiveFamily::Textures => estimate_with_ratio(
            entry.size,
            COMPRESSIBLE_BA2_ESTIMATE_NUMERATOR,
            COMPRESSIBLE_BA2_ESTIMATE_DENOMINATOR,
        ),
        ArchiveFamily::Sounds | ArchiveFamily::Main => entry.size,
    }
}

fn estimated_planned_entry_size(entry: &ArchiveEntry, archive_ext: &str) -> u64 {
    estimated_planned_data_size(entry, archive_ext)
        + ENTRY_OVERHEAD
        + entry.relative_path.as_bytes().len() as u64
}

fn estimate_planned_archive_size(entries: &[ArchiveEntry], archive_ext: &str) -> u64 {
    if entries.is_empty() {
        return 0;
    }
    ARCHIVE_HEADER_OVERHEAD
        + entries
            .iter()
            .map(|entry| estimated_planned_entry_size(entry, archive_ext))
            .sum::<u64>()
}

fn output_name(mod_name: &str, label: &str, archive_ext: &str, platform_suffix: &str) -> String {
    let ext = archive_ext.trim_start_matches('.');
    format!("{mod_name} - {label}{platform_suffix}.{ext}")
}

fn make_archive(
    mod_name: &str,
    family: ArchiveFamily,
    label: &str,
    entries: Vec<ArchiveEntry>,
    archive_ext: &str,
    platform_suffix: &str,
    texture_archive: bool,
) -> PlannedArchive {
    PlannedArchive {
        label: label.to_string(),
        family: family_label(family).to_string(),
        output_name: output_name(mod_name, label, archive_ext, platform_suffix),
        entries,
        texture_archive,
    }
}

fn is_dds_entry(entry: &ArchiveEntry) -> bool {
    Path::new(&entry.relative_path)
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("dds"))
}

fn plan_texture_archives(
    mod_name: &str,
    family: ArchiveFamily,
    label: &str,
    entries: Vec<ArchiveEntry>,
    archive_ext: &str,
    platform_suffix: &str,
    cap: u64,
) -> PackResult<Vec<PlannedArchive>> {
    if entries.is_empty() {
        return Ok(Vec::new());
    }
    if estimate_planned_archive_size(&entries, archive_ext) <= cap {
        return Ok(vec![make_archive(
            mod_name,
            family,
            label,
            entries,
            archive_ext,
            platform_suffix,
            true,
        )]);
    }
    shard_entries(
        mod_name,
        family,
        label,
        &entries,
        archive_ext,
        platform_suffix,
        cap,
        true,
        None,
    )
}

pub(crate) struct PlannedArchiveOut {
    pub(crate) family: String,
    pub(crate) label: String,
    pub(crate) output_name: String,
    pub(crate) texture_archive: bool,
    pub(crate) entries: Vec<(String, String, u64)>,
}

pub(crate) fn plan_archives_public(
    mod_name: &str,
    entries: &[(String, String, u64)],
    archive_ext: &str,
    platform_suffix: &str,
    cap: u64,
    game: &str,
    expanded_archives: bool,
) -> PackResult<Vec<PlannedArchiveOut>> {
    let archive_entries: Vec<ArchiveEntry> = entries
        .iter()
        .map(|(rel, src, size)| {
            let relative_path = normalize_relative_path(rel);
            let family = classify_archive_family(&relative_path);
            ArchiveEntry {
                relative_path,
                source_path: PathBuf::from(src),
                size: *size,
                family,
            }
        })
        .collect();
    let plans = plan_archive_outputs(
        mod_name,
        &archive_entries,
        archive_ext,
        platform_suffix,
        cap,
        game,
        expanded_archives,
    )?;
    Ok(plans
        .into_iter()
        .map(|plan| PlannedArchiveOut {
            family: plan.family,
            label: plan.label,
            output_name: plan.output_name,
            texture_archive: plan.texture_archive,
            entries: plan
                .entries
                .into_iter()
                .map(|entry| {
                    (
                        entry.relative_path,
                        entry.source_path.to_string_lossy().into_owned(),
                        entry.size,
                    )
                })
                .collect(),
        })
        .collect())
}

fn uses_fo4_expanded_label_aliases(game: &str, archive_ext: &str) -> bool {
    let ext = archive_ext.trim_start_matches('.');
    game.eq_ignore_ascii_case("fo4") && ext.eq_ignore_ascii_case("ba2")
}

fn family_label_aliases(
    family: ArchiveFamily,
    game: &str,
    archive_ext: &str,
) -> Option<&'static [&'static str]> {
    if !uses_fo4_expanded_label_aliases(game, archive_ext) {
        return None;
    }
    match family {
        ArchiveFamily::Meshes => Some(&["Meshes", "MeshesExtra"]),
        ArchiveFamily::Scripts => Some(&["Misc"]),
        _ => None,
    }
}

fn shard_label(
    _family: ArchiveFamily,
    label_prefix: &str,
    shard_index: usize,
    label_names: Option<&'static [&'static str]>,
) -> PackResult<String> {
    let Some(label_names) = label_names else {
        return Ok(format!("{}{}", label_prefix, shard_index + 1));
    };
    let Some(label) = label_names.get(shard_index) else {
        if let Some(base_label) = label_names.last() {
            let overflow_index = shard_index - label_names.len() + 1;
            return Ok(format!("{base_label}{overflow_index}"));
        }
        return Ok(format!("{}{}", label_prefix, shard_index + 1));
    };
    Ok((*label).to_string())
}

fn shard_entries(
    mod_name: &str,
    family: ArchiveFamily,
    label_prefix: &str,
    entries: &[ArchiveEntry],
    archive_ext: &str,
    platform_suffix: &str,
    cap: u64,
    texture_archive: bool,
    label_names: Option<&'static [&'static str]>,
) -> PackResult<Vec<PlannedArchive>> {
    let mut shards = Vec::new();
    let mut current = Vec::new();
    let mut current_estimated_size = 0u64;
    for entry in entries {
        let entry_estimated_size = estimated_planned_entry_size(entry, archive_ext);
        let single_size = ARCHIVE_HEADER_OVERHEAD + entry_estimated_size;
        if single_size > cap {
            return Err(format!(
                "{} ({} bytes source, {} bytes estimated packed) exceeds archive cap, exceeding archive max size {} bytes",
                entry.relative_path, entry.size, single_size, cap
            ));
        }
        let candidate_size = if current.is_empty() {
            single_size
        } else {
            current_estimated_size + entry_estimated_size
        };
        if !current.is_empty() && candidate_size > cap {
            let label = shard_label(family, label_prefix, shards.len(), label_names)?;
            shards.push(make_archive(
                mod_name,
                family,
                &label,
                current,
                archive_ext,
                platform_suffix,
                texture_archive,
            ));
            current = vec![entry.clone()];
            current_estimated_size = single_size;
        } else {
            current.push(entry.clone());
            current_estimated_size = candidate_size;
        }
    }
    if !current.is_empty() {
        let label = shard_label(family, label_prefix, shards.len(), label_names)?;
        shards.push(make_archive(
            mod_name,
            family,
            &label,
            current,
            archive_ext,
            platform_suffix,
            texture_archive,
        ));
    }
    Ok(shards)
}

fn plan_archive_outputs(
    mod_name: &str,
    entries: &[ArchiveEntry],
    archive_ext: &str,
    platform_suffix: &str,
    cap: u64,
    game: &str,
    expanded_archives: bool,
) -> PackResult<Vec<PlannedArchive>> {
    let mut by_family: HashMap<ArchiveFamily, Vec<ArchiveEntry>> = HashMap::new();
    let mut normalized = entries.to_vec();
    normalized.sort_by(|a, b| {
        let ak = a.relative_path.to_ascii_lowercase();
        let bk = b.relative_path.to_ascii_lowercase();
        ak.cmp(&bk)
            .then_with(|| a.relative_path.cmp(&b.relative_path))
    });
    for entry in normalized {
        if expanded_archives {
            let estimated_size =
                estimate_planned_archive_size(std::slice::from_ref(&entry), archive_ext);
            if estimated_size > cap {
                return Err(format!(
                    "{} ({} bytes source, {} bytes estimated packed) exceeds archive cap, exceeding archive max size {} bytes",
                    entry.relative_path, entry.size, estimated_size, cap
                ));
            }
        }
        by_family.entry(entry.family).or_default().push(entry);
    }

    let mut planned = Vec::new();
    if expanded_archives {
        for (family, texture_label) in [
            (ArchiveFamily::Textures, "Textures"),
            (ArchiveFamily::Lod, "LODTextures"),
            (ArchiveFamily::Terrain, "TerrainTextures"),
        ] {
            let family_entries = by_family.remove(&family).unwrap_or_default();
            let (texture_entries, general_entries) = if family == ArchiveFamily::Textures {
                (family_entries, Vec::new())
            } else {
                family_entries.into_iter().partition(is_dds_entry)
            };
            if !general_entries.is_empty() {
                by_family.insert(family, general_entries);
            }
            planned.extend(plan_texture_archives(
                mod_name,
                family,
                texture_label,
                texture_entries,
                archive_ext,
                platform_suffix,
                cap,
            )?);
        }
    } else {
        let mut texture_entries = by_family
            .remove(&ArchiveFamily::Textures)
            .unwrap_or_default();
        for family in [ArchiveFamily::Lod, ArchiveFamily::Terrain] {
            let family_entries = by_family.remove(&family).unwrap_or_default();
            let (family_textures, general_entries): (Vec<_>, Vec<_>) =
                family_entries.into_iter().partition(is_dds_entry);
            texture_entries.extend(family_textures);
            if !general_entries.is_empty() {
                by_family.insert(family, general_entries);
            }
        }
        texture_entries.sort_by(|a, b| {
            let ak = a.relative_path.to_ascii_lowercase();
            let bk = b.relative_path.to_ascii_lowercase();
            ak.cmp(&bk)
                .then_with(|| a.relative_path.cmp(&b.relative_path))
        });
        if !texture_entries.is_empty() {
            planned.push(make_archive(
                mod_name,
                ArchiveFamily::Textures,
                "Textures",
                texture_entries,
                archive_ext,
                platform_suffix,
                true,
            ));
        }
    }

    if expanded_archives {
        let strings_entries = by_family
            .remove(&ArchiveFamily::Strings)
            .unwrap_or_default();
        if !strings_entries.is_empty() {
            let main = by_family.entry(ArchiveFamily::Main).or_default();
            let old_main = std::mem::take(main);
            *main = strings_entries.into_iter().chain(old_main).collect();
        }
        if uses_fo4_expanded_label_aliases(game, archive_ext)
            && by_family.contains_key(&ArchiveFamily::Scripts)
            && by_family.contains_key(&ArchiveFamily::Main)
        {
            let main_entries = by_family.remove(&ArchiveFamily::Main).unwrap_or_default();
            by_family
                .entry(ArchiveFamily::Scripts)
                .or_default()
                .extend(main_entries);
        }
        let mut main_archives = Vec::new();
        for family in MAIN_FAMILY_ORDER {
            let family_entries = by_family.get(&family).cloned().unwrap_or_default();
            if family_entries.is_empty() {
                continue;
            }
            let label = family_label(family);
            let label_names = family_label_aliases(family, game, archive_ext);
            if estimate_planned_archive_size(&family_entries, archive_ext) <= cap {
                let label = label_names
                    .and_then(|names| names.first().copied())
                    .unwrap_or(label);
                main_archives.push(make_archive(
                    mod_name,
                    family,
                    label,
                    family_entries,
                    archive_ext,
                    platform_suffix,
                    false,
                ));
            } else {
                main_archives.extend(shard_entries(
                    mod_name,
                    family,
                    label,
                    &family_entries,
                    archive_ext,
                    platform_suffix,
                    cap,
                    false,
                    label_names,
                )?);
            }
        }
        main_archives.extend(planned);
        return Ok(main_archives);
    }

    let main_entries: Vec<ArchiveEntry> = MAIN_FAMILY_ORDER
        .iter()
        .flat_map(|family| by_family.get(family).cloned().unwrap_or_default())
        .collect();
    if !main_entries.is_empty() {
        planned.insert(
            0,
            make_archive(
                mod_name,
                ArchiveFamily::Main,
                "Main",
                main_entries,
                archive_ext,
                platform_suffix,
                false,
            ),
        );
    }
    Ok(planned)
}

fn family_label(family: ArchiveFamily) -> &'static str {
    match family {
        ArchiveFamily::Lod => "LOD",
        ArchiveFamily::Terrain => "Terrain",
        ArchiveFamily::Meshes => "Meshes",
        ArchiveFamily::Sounds => "Sounds",
        ArchiveFamily::Animations => "Animations",
        ArchiveFamily::Scripts => "Scripts",
        ArchiveFamily::Strings => "Strings",
        ArchiveFamily::Materials => "Materials",
        ArchiveFamily::Interface => "Interface",
        ArchiveFamily::Main => "Main",
        ArchiveFamily::Textures => "Textures",
    }
}

fn inventory_tree(
    root: &Path,
    prefix: Option<&str>,
    entries: &mut Vec<ArchiveEntry>,
    progress: &mut impl FnMut(PackProgress) -> PackResult<()>,
) -> PackResult<()> {
    if !root.is_dir() {
        return Ok(());
    }
    let mut completed = 0usize;
    for entry in WalkDir::new(root) {
        let entry = entry.map_err(|err| err.to_string())?;
        if !entry.file_type().is_file() {
            continue;
        }
        let rel = entry
            .path()
            .strip_prefix(root)
            .map_err(|err| err.to_string())?;
        let mut rel_slash = normalize_relative_path(&rel.to_string_lossy());
        if let Some(prefix) = prefix {
            rel_slash = format!("{}/{}", prefix.trim_matches('/'), rel_slash);
        }
        let size = entry.metadata().map_err(|err| err.to_string())?.len();
        let family = classify_archive_family(&rel_slash);
        entries.push(ArchiveEntry {
            relative_path: rel_slash,
            source_path: entry.path().to_path_buf(),
            size,
            family,
        });
        completed += 1;
        if completed % 10_000 == 0 {
            progress(PackProgress {
                phase: "inventory",
                platform: "pc".to_string(),
                message: format!("Inventory scanned {completed} files"),
                completed,
                total: 0,
            })?;
        }
    }
    Ok(())
}

fn inventory_mod_entries(
    config: &PackModConfig,
    progress: &mut impl FnMut(PackProgress) -> PackResult<()>,
) -> PackResult<Vec<ArchiveEntry>> {
    let mut entries = Vec::new();
    inventory_tree(&config.data_dir, None, &mut entries, progress)?;
    inventory_tree(&config.strings_dir, Some("Strings"), &mut entries, progress)?;
    entries.sort_by(|a, b| {
        let ak = a.relative_path.to_ascii_lowercase();
        let bk = b.relative_path.to_ascii_lowercase();
        ak.cmp(&bk)
            .then_with(|| a.relative_path.cmp(&b.relative_path))
    });
    Ok(entries)
}

fn native_archive_type(game: &str, texture_archive: bool, platform: &str) -> PackResult<String> {
    if platform == "xbox" && game == "fo4" {
        return Ok(if texture_archive {
            "fo4xboxdds"
        } else {
            "fo4xbox"
        }
        .to_string());
    }
    if platform == "ps" && game == "fo4" {
        return Ok(if texture_archive { "fo4psdds" } else { "fo4ps" }.to_string());
    }
    let suffix = if texture_archive { "dds" } else { "" };
    match game {
        "fo4" => Ok(format!("fo4{suffix}")),
        "fo76" => Ok(format!("fo76{suffix}")),
        "starfield" => Ok(if texture_archive {
            "starfielddds"
        } else {
            "starfield"
        }
        .to_string()),
        "skyrimse" => Ok("sse".to_string()),
        "skyrim" => Ok("tes5".to_string()),
        "oblivion" => Ok("tes4".to_string()),
        "fo3" => Ok("fo3".to_string()),
        "fnv" | "fonv" => Ok("fonv".to_string()),
        _ => Err(format!("native packer does not support game: {game}")),
    }
}

fn planned_entry_specs(plan: &PlannedArchive) -> Vec<PackEntrySpec> {
    plan.entries
        .iter()
        .map(|entry| PackEntrySpec {
            source_path: entry.source_path.clone(),
            archive_path: entry.relative_path.clone(),
            source_size: Some(entry.size),
        })
        .collect()
}

fn default_archive_worker_budget() -> usize {
    1
}

fn allocate_archive_workers(worker_budget: usize, archive_count: usize) -> ArchiveWorkerAllocation {
    if archive_count == 0 {
        return ArchiveWorkerAllocation {
            archive_concurrency: 0,
            workers_per_archive: 0,
            extra_worker_archives: 0,
        };
    }
    let worker_budget = worker_budget.max(1);
    let archive_concurrency = worker_budget
        .min(archive_count)
        .min(MAX_ARCHIVE_PACK_CONCURRENCY);
    ArchiveWorkerAllocation {
        archive_concurrency,
        workers_per_archive: (worker_budget / archive_concurrency).max(1),
        extra_worker_archives: worker_budget % archive_concurrency,
    }
}

fn archive_worker_count(allocation: ArchiveWorkerAllocation, archive_slot: usize) -> usize {
    allocation.workers_per_archive + usize::from(archive_slot < allocation.extra_worker_archives)
}

fn pack_group_policy<T: ScheduledArchivePlan>(
    worker_budget: usize,
    plans: &[T],
    start: usize,
) -> (usize, usize) {
    let texture_archive = plans[start].texture_archive();
    let group_len = plans[start..]
        .iter()
        .take_while(|plan| plan.texture_archive() == texture_archive)
        .count();
    let concurrency_cap = if texture_archive {
        MAX_TEXTURE_ARCHIVE_CONCURRENCY
    } else {
        MAX_ARCHIVE_PACK_CONCURRENCY
    };
    let batch_len = group_len
        .min(concurrency_cap)
        .min(worker_budget.max(1))
        .max(1);
    let active_worker_budget = if texture_archive {
        worker_budget
            .min(batch_len.saturating_mul(MAX_TEXTURE_WORKERS_PER_ARCHIVE))
            .max(1)
    } else {
        worker_budget.max(1)
    };
    (batch_len, active_worker_budget)
}

fn archive_member_list(path: &Path) -> PackResult<Vec<String>> {
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let mut file = fs::File::open(path).map_err(|err| err.to_string())?;
    let Some(format) = crate::guess_format(&mut file) else {
        return Ok(Vec::new());
    };
    match format {
        FileFormat::TES4 => {
            let Ok((archive, _)) = tes4::Archive::read(path) else {
                return Ok(Vec::new());
            };
            Ok(archive
                .iter()
                .flat_map(|(dir_key, directory)| {
                    directory.iter().map(move |(file_key, _)| {
                        format!("{}\\{}", dir_key.name(), file_key.name())
                    })
                })
                .collect())
        }
        FileFormat::FO4 => {
            let Ok((archive, _)) = fo4::Archive::read(path) else {
                return Ok(Vec::new());
            };
            Ok(archive
                .keys()
                .map(|key| key.name().to_string().replace('/', "\\"))
                .collect())
        }
    }
}

fn write_reference_manifest(
    output_path: &Path,
    temp_dir: &Path,
    label: &str,
) -> PackResult<Option<PathBuf>> {
    let members = archive_member_list(output_path)?;
    if members.is_empty() {
        return Ok(None);
    }
    fs::create_dir_all(temp_dir).map_err(|err| err.to_string())?;
    let manifest_path = temp_dir.join(format!(
        "{}_reference_manifest.json",
        label.to_ascii_lowercase()
    ));
    let json = Value::Array(members.into_iter().map(Value::String).collect());
    fs::write(&manifest_path, json.to_string()).map_err(|err| err.to_string())?;
    Ok(Some(manifest_path))
}

fn is_generated_archive_name(path: &Path, prefix: &str) -> bool {
    let Some(stem) = path.file_stem().and_then(|value| value.to_str()) else {
        return false;
    };
    let Some(mut label) = stem.strip_prefix(prefix) else {
        return false;
    };
    for suffix in ["_xbox", "_ps"] {
        if let Some(stripped) = label.strip_suffix(suffix) {
            label = stripped;
            break;
        }
    }
    let label_base = label.trim_end_matches(|ch: char| ch.is_ascii_digit());
    !label_base.is_empty() && GENERATED_LABEL_BASES.contains(&label_base)
}

fn cleanup_obsolete_archives(
    mod_dir: &Path,
    mod_name: &str,
    archive_ext: &str,
    platform_suffix: &str,
    expected_names: &HashSet<String>,
) -> PackResult<()> {
    let prefix = format!("{mod_name} - ");
    let wanted_ext = archive_ext.trim_start_matches('.').to_ascii_lowercase();
    if !mod_dir.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(mod_dir).map_err(|err| err.to_string())? {
        let entry = entry.map_err(|err| err.to_string())?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if expected_names.contains(name) {
            continue;
        }
        let Some(ext) = path.extension().and_then(|value| value.to_str()) else {
            continue;
        };
        if ext.to_ascii_lowercase() != wanted_ext {
            continue;
        }
        if !name.starts_with(&prefix) || !is_generated_archive_name(&path, &prefix) {
            continue;
        }
        let platform_match = path
            .file_stem()
            .and_then(|value| value.to_str())
            .is_some_and(|stem| match platform_suffix {
                "_xbox" => stem.ends_with("_xbox"),
                "_ps" => stem.ends_with("_ps"),
                _ => !stem.ends_with("_xbox") && !stem.ends_with("_ps"),
            });
        if platform_match {
            fs::remove_file(path).map_err(|err| err.to_string())?;
        }
    }
    Ok(())
}

fn pack_planned_archive(
    config: &PackModConfig,
    plan: &PlannedArchive,
    temp_manifest_dir: &Path,
    workers_for_archive: usize,
) -> PackResult<ArchiveSummary> {
    let output_path = config.mod_dir.join(&plan.output_name);
    let started = Instant::now();
    let plan_bytes = plan.entries.iter().map(|entry| entry.size).sum::<u64>();
    let archive_type = native_archive_type(&config.game, plan.texture_archive, "pc")?;
    let level = crate::pack::archive_type_default_level(&archive_type);
    let reference_manifest =
        write_reference_manifest(&output_path, temp_manifest_dir, &plan.label)?;
    let manifest_path = reference_manifest
        .as_deref()
        .or(config.manifest_path.as_deref());
    let specs = planned_entry_specs(plan);
    let (_, direct_pack_stats) = pack::pack_archive_entries_with_stats(
        &specs,
        &output_path,
        &archive_type,
        true,
        level,
        false,
        manifest_path,
        Some(workers_for_archive.max(1)),
    )?;
    Ok(ArchiveSummary {
        platform: "pc".to_string(),
        name: plan.output_name.clone(),
        file_count: plan.entries.len(),
        bytes: plan_bytes,
        elapsed_secs: started.elapsed().as_secs_f64(),
        direct_pack_stats,
    })
}

fn pack_archive_plan(
    plan: &PackArchivePlan,
    workers_for_archive: usize,
) -> PackResult<ArchiveSummary> {
    let started = Instant::now();
    let level = crate::pack::archive_type_default_level(&plan.archive_type);
    let (_, direct_pack_stats) = pack::pack_archive_entries_with_stats(
        &plan.entries,
        &plan.output_path,
        &plan.archive_type,
        true,
        level,
        false,
        None,
        Some(workers_for_archive.max(1)),
    )?;
    Ok(ArchiveSummary {
        platform: "pc".to_string(),
        name: plan.output_name.clone(),
        file_count: plan.entries.len(),
        bytes: plan.input_bytes,
        elapsed_secs: started.elapsed().as_secs_f64(),
        direct_pack_stats,
    })
}

fn report_archive_pack_start<T, A, P>(
    plan: &T,
    plan_index: usize,
    plan_count: usize,
    workers_for_archive: usize,
    worker_budget: usize,
    archive_concurrency: usize,
    completed: usize,
    archive_type_for: &A,
    progress: &mut P,
) -> PackResult<()>
where
    T: ScheduledArchivePlan,
    A: Fn(&T) -> PackResult<String>,
    P: FnMut(PackProgress) -> PackResult<()>,
{
    let archive_type = archive_type_for(plan)?;
    let level = crate::pack::archive_type_default_level(&archive_type);
    let codec = crate::pack::archive_type_compression_codec(&archive_type);
    progress(PackProgress {
        phase: "pack",
        platform: "pc".to_string(),
        message: format!(
            "Packing archive {} ({}/{}) files={} bytes={:.1} MB workers={}/{} archive_concurrency={} compression={codec}:{level}",
            plan.output_name(),
            plan_index + 1,
            plan_count,
            plan.file_count(),
            plan.input_bytes() as f64 / (1024.0 * 1024.0),
            workers_for_archive,
            worker_budget,
            archive_concurrency
        ),
        completed,
        total: plan_count,
    })
}

fn archive_pack_completion_message(summary: &ArchiveSummary) -> String {
    let mut message = format!(
        "Archive packed native: name={} files={} bytes={:.1} MB elapsed={:.3}s",
        summary.name,
        summary.file_count,
        summary.bytes as f64 / (1024.0 * 1024.0),
        summary.elapsed_secs
    );
    if let Some(stats) = summary.direct_pack_stats {
        let writer_io_secs = stats.writer_write_secs + stats.writer_flush_secs;
        let writer_rate = if writer_io_secs > 0.0 {
            stats.payload_bytes as f64 / (1024.0 * 1024.0) / writer_io_secs
        } else {
            0.0
        };
        write!(
            message,
            " payloads={} payload_bytes={:.1} MB writer_io={writer_io_secs:.3}s writer_rate={writer_rate:.1} MB/s writer_idle={:.3}s prepare_worker={:.3}s source_read_worker={:.3}s compression_worker={:.3}s fallback_worker={:.3}s chunks={} fallbacks={} memory_budget={:.1} MB peak_in_flight={:.1} MB throttle_wait={:.3}s throttle_events={}",
            stats.payload_write_calls,
            stats.payload_bytes as f64 / (1024.0 * 1024.0),
            stats.writer_idle_secs,
            stats.prepare_worker_secs,
            stats.source_read_worker_secs,
            stats.compression_worker_secs,
            stats.fallback_worker_secs,
            stats.prepared_chunk_count,
            stats.fallback_file_count,
            stats.memory_budget_bytes as f64 / (1024.0 * 1024.0),
            stats.peak_in_flight_bytes as f64 / (1024.0 * 1024.0),
            stats.throttle_wait_secs,
            stats.throttle_wait_count,
        )
        .expect("writing to String cannot fail");
    }
    message
}

fn archive_pack_progress_message(
    name: &str,
    elapsed: Duration,
    output_bytes: Option<u64>,
    interval_bytes: u64,
    interval: Duration,
    previous_output_bytes: Option<u64>,
) -> String {
    let Some(output_bytes) = output_bytes else {
        return format!(
            "Archive packing progress: name={name} elapsed={:.1}s output=pending",
            elapsed.as_secs_f64()
        );
    };
    let interval_rate = if interval.is_zero() {
        0.0
    } else {
        interval_bytes as f64 / (1024.0 * 1024.0) / interval.as_secs_f64()
    };
    let output_mb = output_bytes as f64 / (1024.0 * 1024.0);
    match previous_output_bytes.filter(|bytes| *bytes != 0) {
        Some(previous_output_bytes) => format!(
            "Archive packing progress: name={name} elapsed={:.1}s output={output_mb:.1}/{:.1} MB approx={:.1}% interval_rate={interval_rate:.1} MB/s",
            elapsed.as_secs_f64(),
            previous_output_bytes as f64 / (1024.0 * 1024.0),
            output_bytes as f64 * 100.0 / previous_output_bytes as f64,
        ),
        None => format!(
            "Archive packing progress: name={name} elapsed={:.1}s output={output_mb:.1} MB interval_rate={interval_rate:.1} MB/s",
            elapsed.as_secs_f64(),
        ),
    }
}

impl ActiveArchiveProgress {
    fn new(plan_index: usize, output_path: PathBuf) -> Self {
        let previous_output_bytes = fs::metadata(&output_path)
            .ok()
            .map(|metadata| metadata.len())
            .filter(|bytes| *bytes != 0);
        let now = Instant::now();
        Self {
            plan_index,
            output_path,
            started_at: now,
            sampled_at: now,
            output_bytes: previous_output_bytes.unwrap_or(0),
            previous_output_bytes,
        }
    }

    fn sample(&mut self, name: &str, now: Instant) -> String {
        let output_bytes = fs::metadata(&self.output_path)
            .ok()
            .map(|metadata| metadata.len());
        let interval_bytes = output_bytes
            .unwrap_or(self.output_bytes)
            .saturating_sub(self.output_bytes);
        let message = archive_pack_progress_message(
            name,
            now.duration_since(self.started_at),
            output_bytes,
            interval_bytes,
            now.duration_since(self.sampled_at),
            self.previous_output_bytes,
        );
        if let Some(output_bytes) = output_bytes {
            self.output_bytes = output_bytes;
        }
        self.sampled_at = now;
        message
    }
}

fn pack_scheduled_archives<T: ScheduledArchivePlan + Sync>(
    plans: &[T],
    worker_budget: usize,
    archive_type_for: &(impl Fn(&T) -> PackResult<String> + Sync),
    output_path_for: &(impl Fn(&T) -> PathBuf + Sync),
    pack_one: &(impl Fn(&T, usize) -> PackResult<ArchiveSummary> + Sync),
    progress: &mut impl FnMut(PackProgress) -> PackResult<()>,
) -> PackResult<Vec<ArchiveSummary>> {
    let worker_budget = worker_budget.max(1);
    if plans.len() > 1 {
        progress(PackProgress {
            phase: "pack",
            platform: "pc".to_string(),
            message: format!(
                "Packing archives with total_workers={worker_budget} general_concurrency={} texture_concurrency={}",
                MAX_ARCHIVE_PACK_CONCURRENCY.min(worker_budget),
                MAX_TEXTURE_ARCHIVE_CONCURRENCY.min(worker_budget),
            ),
            completed: 0,
            total: plans.len(),
        })?;
    }

    let mut summaries = vec![None; plans.len()];
    let mut completed = 0usize;
    let mut group_start = 0usize;
    while group_start < plans.len() {
        let texture_archive = plans[group_start].texture_archive();
        let group_len = plans[group_start..]
            .iter()
            .take_while(|plan| plan.texture_archive() == texture_archive)
            .count();
        let group_end = group_start + group_len;
        let (archive_concurrency, active_worker_budget) =
            pack_group_policy(worker_budget, plans, group_start);
        let allocation = allocate_archive_workers(active_worker_budget, archive_concurrency);

        std::thread::scope(|scope| -> PackResult<()> {
            let (sender, receiver) =
                std::sync::mpsc::channel::<(usize, usize, PackResult<ArchiveSummary>)>();
            let mut free_slots: Vec<_> = (0..archive_concurrency).rev().collect();
            let mut active_progress: Vec<Option<ActiveArchiveProgress>> =
                std::iter::repeat_with(|| None)
                    .take(archive_concurrency)
                    .collect();
            let mut next_plan_index = group_start;
            let mut active = 0usize;
            let mut first_error = None;

            while active < archive_concurrency && next_plan_index < group_end {
                let slot = free_slots.pop().expect("archive slot should be available");
                let workers_for_archive = archive_worker_count(allocation, slot);
                report_archive_pack_start(
                    &plans[next_plan_index],
                    next_plan_index,
                    plans.len(),
                    workers_for_archive,
                    active_worker_budget,
                    archive_concurrency,
                    completed,
                    archive_type_for,
                    progress,
                )?;
                let plan_index = next_plan_index;
                let plan = &plans[plan_index];
                active_progress[slot] = Some(ActiveArchiveProgress::new(
                    plan_index,
                    output_path_for(plan),
                ));
                let sender = sender.clone();
                scope.spawn(move || {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        pack_one(plan, workers_for_archive)
                    }))
                    .unwrap_or_else(|_| Err("archive pack worker panicked".to_string()));
                    let _ = sender.send((slot, plan_index, result));
                });
                active += 1;
                next_plan_index += 1;
            }

            let mut next_heartbeat = Instant::now() + ARCHIVE_PACK_HEARTBEAT_INTERVAL;
            while active != 0 {
                let wait = next_heartbeat.saturating_duration_since(Instant::now());
                let (slot, plan_index, result) = match receiver.recv_timeout(wait) {
                    Ok(result) => result,
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        let now = Instant::now();
                        if first_error.is_none() {
                            for active_archive in active_progress.iter_mut().flatten() {
                                let plan = &plans[active_archive.plan_index];
                                if let Err(err) = progress(PackProgress {
                                    phase: "pack",
                                    platform: "pc".to_string(),
                                    message: active_archive.sample(plan.output_name(), now),
                                    completed,
                                    total: plans.len(),
                                }) {
                                    first_error = Some(err);
                                    break;
                                }
                            }
                        }
                        next_heartbeat = now + ARCHIVE_PACK_HEARTBEAT_INTERVAL;
                        continue;
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                        return Err("archive pack worker stopped without a result".to_string());
                    }
                };
                active -= 1;
                free_slots.push(slot);
                active_progress[slot] = None;
                match result {
                    Ok(summary) => {
                        completed += 1;
                        let progress_result = progress(PackProgress {
                            phase: "pack",
                            platform: "pc".to_string(),
                            message: archive_pack_completion_message(&summary),
                            completed,
                            total: plans.len(),
                        });
                        summaries[plan_index] = Some(summary);
                        if let Err(err) = progress_result {
                            first_error.get_or_insert(err);
                        }
                    }
                    Err(err) => {
                        first_error.get_or_insert(err);
                    }
                }

                if first_error.is_none() && next_plan_index < group_end {
                    let slot = free_slots.pop().expect("archive slot should be available");
                    let workers_for_archive = archive_worker_count(allocation, slot);
                    if let Err(err) = report_archive_pack_start(
                        &plans[next_plan_index],
                        next_plan_index,
                        plans.len(),
                        workers_for_archive,
                        active_worker_budget,
                        archive_concurrency,
                        completed,
                        archive_type_for,
                        progress,
                    ) {
                        first_error = Some(err);
                        continue;
                    }
                    let plan_index = next_plan_index;
                    let plan = &plans[plan_index];
                    active_progress[slot] = Some(ActiveArchiveProgress::new(
                        plan_index,
                        output_path_for(plan),
                    ));
                    let sender = sender.clone();
                    scope.spawn(move || {
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            pack_one(plan, workers_for_archive)
                        }))
                        .unwrap_or_else(|_| Err("archive pack worker panicked".to_string()));
                        let _ = sender.send((slot, plan_index, result));
                    });
                    active += 1;
                    next_plan_index += 1;
                }
            }

            match first_error {
                Some(err) => Err(err),
                None => Ok(()),
            }
        })?;
        group_start = group_end;
    }

    summaries
        .into_iter()
        .map(|summary| summary.ok_or_else(|| "archive pack result missing".to_string()))
        .collect()
}

pub(crate) fn pack_archive_plans(
    plans: &[PackArchivePlan],
    worker_budget: usize,
    mut progress: impl FnMut(PackProgress) -> PackResult<()>,
) -> PackResult<Vec<ArchiveSummary>> {
    let worker_budget = if worker_budget == 0 {
        default_archive_worker_budget()
    } else {
        worker_budget
    };
    pack_scheduled_archives(
        plans,
        worker_budget,
        &|plan| Ok(plan.archive_type.clone()),
        &|plan| plan.output_path.clone(),
        &pack_archive_plan,
        &mut progress,
    )
}

pub(crate) fn pack_mod_archives(
    config: &PackModConfig,
    mut progress: impl FnMut(PackProgress) -> PackResult<()>,
) -> PackResult<PackModResult> {
    if config.xbox {
        return Err(
            "native pack_mod_archives does not yet support Xbox texture tiling".to_string(),
        );
    }
    if !config.pc {
        return Ok(PackModResult {
            archives: Vec::new(),
            inventory_elapsed_secs: 0.0,
            planning_elapsed_secs: 0.0,
        });
    }
    if !config.data_dir.is_dir() {
        return Err(format!(
            "{} not found -- nothing to pack",
            config.data_dir.display()
        ));
    }

    progress(PackProgress {
        phase: "inventory",
        platform: "pc".to_string(),
        message: "Scanning archive inputs".to_string(),
        completed: 0,
        total: 0,
    })?;
    let inventory_started = Instant::now();
    let entries = inventory_mod_entries(config, &mut progress)?;
    let inventory_elapsed_secs = inventory_started.elapsed().as_secs_f64();
    let bytes = entries.iter().map(|entry| entry.size).sum::<u64>();
    progress(PackProgress {
        phase: "inventory",
        platform: "pc".to_string(),
        message: format!(
            "Archive inventory: platform=pc files={} bytes={:.1} MB elapsed={:.3}s",
            entries.len(),
            bytes as f64 / (1024.0 * 1024.0),
            inventory_elapsed_secs
        ),
        completed: entries.len(),
        total: entries.len(),
    })?;

    let planning_started = Instant::now();
    let plans = plan_archive_outputs(
        &config.mod_name,
        &entries,
        &config.archive_ext,
        "",
        config.archive_cap,
        &config.game,
        config.expanded_archives,
    )?;
    let planning_elapsed_secs = planning_started.elapsed().as_secs_f64();
    progress(PackProgress {
        phase: "planning",
        platform: "pc".to_string(),
        message: format!(
            "Archive planning: platform=pc plans={} elapsed={:.3}s",
            plans.len(),
            planning_elapsed_secs
        ),
        completed: plans.len(),
        total: plans.len(),
    })?;

    let expected_names: HashSet<String> =
        plans.iter().map(|plan| plan.output_name.clone()).collect();
    if config.dry_run {
        let archives = plans
            .iter()
            .map(|plan| ArchiveSummary {
                platform: "pc".to_string(),
                name: plan.output_name.clone(),
                file_count: plan.entries.len(),
                bytes: plan.entries.iter().map(|entry| entry.size).sum(),
                elapsed_secs: 0.0,
                direct_pack_stats: None,
            })
            .collect();
        return Ok(PackModResult {
            archives,
            inventory_elapsed_secs,
            planning_elapsed_secs,
        });
    }

    let temp_manifest_dir = config.mod_dir.join("_deploy_tmp").join("native_manifests");
    let worker_budget = if config.archive_workers == 0 {
        default_archive_worker_budget()
    } else {
        config.archive_workers
    }
    .max(1);
    let archive_type_for =
        |plan: &PlannedArchive| native_archive_type(&config.game, plan.texture_archive, "pc");
    let output_path_for = |plan: &PlannedArchive| config.mod_dir.join(&plan.output_name);
    let pack_one = |plan: &PlannedArchive, workers_for_archive| {
        pack_planned_archive(config, plan, &temp_manifest_dir, workers_for_archive)
    };
    let summaries = pack_scheduled_archives(
        &plans,
        worker_budget,
        &archive_type_for,
        &output_path_for,
        &pack_one,
        &mut progress,
    )?;
    cleanup_obsolete_archives(
        &config.mod_dir,
        &config.mod_name,
        &config.archive_ext,
        "",
        &expected_names,
    )?;

    Ok(PackModResult {
        archives: summaries,
        inventory_elapsed_secs,
        planning_elapsed_secs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
    };
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    static NEXT_ID: AtomicU64 = AtomicU64::new(0);

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new() -> Self {
            let unique = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time went backwards")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "bsarchive-native-mod-pack-{}-{nanos}-{unique}",
                std::process::id()
            ));
            fs::create_dir_all(&path).expect("failed to create temp dir");
            Self { path }
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn playstation_archive_types_use_gnrl_profiles() {
        assert_eq!(native_archive_type("fo4", false, "ps").unwrap(), "fo4ps");
        assert_eq!(native_archive_type("fo4", true, "ps").unwrap(), "fo4psdds");
    }

    #[test]
    fn generated_archive_names_accept_playstation_suffix() {
        assert!(is_generated_archive_name(
            Path::new("B21_Test - Textures_ps.ba2"),
            "B21_Test - "
        ));
    }

    fn entry(rel: &str, size: u64) -> ArchiveEntry {
        ArchiveEntry {
            relative_path: rel.to_string(),
            source_path: PathBuf::from(rel),
            size,
            family: classify_archive_family(rel),
        }
    }

    #[test]
    fn planner_shards_texture_archives_by_cap() {
        let entries = vec![
            entry("Meshes/a.nif", 10),
            entry("Textures/a.dds", 4000),
            entry("Textures/b.dds", 4000),
        ];
        let plans = plan_archive_outputs("B21_Test", &entries, "ba2", "", 9000, "fo4", true)
            .expect("planning should succeed");
        let names: Vec<_> = plans.iter().map(|plan| plan.output_name.as_str()).collect();

        assert_eq!(
            names,
            vec![
                "B21_Test - Meshes.ba2",
                "B21_Test - Textures1.ba2",
                "B21_Test - Textures2.ba2",
            ]
        );
    }

    #[test]
    fn planner_compact_ignores_archive_cap() {
        let entries = vec![
            entry("Meshes/a.nif", 10),
            entry("Scripts/a.pex", 10),
            entry("Textures/a.dds", 10),
        ];
        let plans = plan_archive_outputs("B21_Test", &entries, "ba2", "", 1, "fo4", false)
            .expect("planning should succeed");
        let names: Vec<_> = plans.iter().map(|plan| plan.output_name.as_str()).collect();

        assert_eq!(
            names,
            vec!["B21_Test - Main.ba2", "B21_Test - Textures.ba2"]
        );
    }

    #[test]
    fn planner_uses_ba2_estimate_for_compressible_lod_archives() {
        let entries: Vec<_> = (0..12)
            .map(|idx| entry(&format!("Meshes/Terrain/Appalachia/{idx:02}.bto"), 30_000))
            .collect();
        let plans = plan_archive_outputs("B21_Test", &entries, "ba2", "", 100_000, "fo4", true)
            .expect("planning should succeed");
        let names: Vec<_> = plans.iter().map(|plan| plan.output_name.as_str()).collect();
        let counts: Vec<_> = plans.iter().map(|plan| plan.entries.len()).collect();

        assert_eq!(
            names,
            vec![
                "B21_Test - LOD1.ba2",
                "B21_Test - LOD2.ba2",
                "B21_Test - LOD3.ba2",
            ]
        );
        assert_eq!(counts, vec![4, 4, 4]);
    }

    #[test]
    fn planner_uses_ba2_estimate_for_texture_archives() {
        let entries = vec![entry("Textures/a.dds", 5000), entry("Textures/b.dds", 5000)];
        let plans = plan_archive_outputs("B21_Test", &entries, "ba2", "", 14_000, "fo4", true)
            .expect("planning should succeed");
        let names: Vec<_> = plans.iter().map(|plan| plan.output_name.as_str()).collect();
        let counts: Vec<_> = plans.iter().map(|plan| plan.entries.len()).collect();

        assert_eq!(names, vec!["B21_Test - Textures.ba2"]);
        assert_eq!(counts, vec![2]);
    }

    #[test]
    fn planner_shards_sounds_without_relying_on_compression() {
        let entries = vec![entry("Sound/a.fuz", 5000), entry("Sound/b.fuz", 5000)];
        let plans = plan_archive_outputs("B21_Test", &entries, "ba2", "", 14_000, "fo4", true)
            .expect("planning should succeed");
        let names: Vec<_> = plans.iter().map(|plan| plan.output_name.as_str()).collect();
        let counts: Vec<_> = plans.iter().map(|plan| plan.entries.len()).collect();

        assert_eq!(
            names,
            vec!["B21_Test - Sounds1.ba2", "B21_Test - Sounds2.ba2"]
        );
        assert_eq!(counts, vec![1, 1]);
    }

    #[test]
    fn archive_worker_allocation_splits_budget_across_archives() {
        let allocation = allocate_archive_workers(20, 3);
        assert_eq!(
            allocation,
            ArchiveWorkerAllocation {
                archive_concurrency: 2,
                workers_per_archive: 10,
                extra_worker_archives: 0,
            }
        );
        assert_eq!(
            (0..allocation.archive_concurrency)
                .map(|slot| archive_worker_count(allocation, slot))
                .collect::<Vec<_>>(),
            vec![10, 10]
        );

        let allocation = allocate_archive_workers(2, 4);
        assert_eq!(allocation.archive_concurrency, 2);
        assert_eq!(
            (0..allocation.archive_concurrency)
                .map(|slot| archive_worker_count(allocation, slot))
                .collect::<Vec<_>>(),
            vec![1, 1]
        );

        let allocation = allocate_archive_workers(7, 17);
        assert_eq!(allocation.archive_concurrency, MAX_ARCHIVE_PACK_CONCURRENCY);
        assert_eq!(
            (0..allocation.archive_concurrency)
                .map(|slot| archive_worker_count(allocation, slot))
                .collect::<Vec<_>>(),
            vec![4, 3]
        );
    }

    #[test]
    fn scheduler_refills_a_finished_archive_slot_without_waiting_for_its_peer() {
        #[derive(Clone)]
        struct TestPlan {
            index: usize,
            name: String,
        }

        impl ScheduledArchivePlan for TestPlan {
            fn output_name(&self) -> &str {
                &self.name
            }

            fn file_count(&self) -> usize {
                1
            }

            fn input_bytes(&self) -> u64 {
                1
            }

            fn texture_archive(&self) -> bool {
                true
            }
        }

        let plans: Vec<_> = (0..3)
            .map(|index| TestPlan {
                index,
                name: format!("Textures{index}.ba2"),
            })
            .collect();
        let third_started = Arc::new((Mutex::new(false), Condvar::new()));
        let pack_signal = Arc::clone(&third_started);
        let pack_one = move |plan: &TestPlan, _workers| {
            if plan.index == 0 {
                let (started, wake) = &*pack_signal;
                let started = started.lock().expect("third-started mutex poisoned");
                let (started, _) = wake
                    .wait_timeout_while(started, Duration::from_secs(2), |value| !*value)
                    .expect("third-started mutex poisoned");
                if !*started {
                    return Err(
                        "third archive did not start while the first was active".to_string()
                    );
                }
            } else if plan.index == 2 {
                let (started, wake) = &*pack_signal;
                *started.lock().expect("third-started mutex poisoned") = true;
                wake.notify_all();
            }
            Ok(ArchiveSummary {
                platform: "pc".to_string(),
                name: plan.name.clone(),
                file_count: 1,
                bytes: 1,
                elapsed_secs: 0.0,
                direct_pack_stats: None,
            })
        };
        let mut messages = Vec::new();

        let summaries = pack_scheduled_archives(
            &plans,
            4,
            &|_| Ok("fo4dds".to_string()),
            &|plan| PathBuf::from(&plan.name),
            &pack_one,
            &mut |event| {
                messages.push(event.message);
                Ok(())
            },
        )
        .expect("rolling scheduler should refill the free slot");

        assert_eq!(
            summaries
                .iter()
                .map(|summary| summary.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Textures0.ba2", "Textures1.ba2", "Textures2.ba2"]
        );
        let completions: Vec<_> = messages
            .iter()
            .filter(|message| message.starts_with("Archive packed native:"))
            .collect();
        assert!(completions[0].contains("Textures1.ba2"));
    }

    #[test]
    fn archive_pack_progress_reports_growth_and_previous_size() {
        let message = archive_pack_progress_message(
            "Textures.ba2",
            Duration::from_secs(65),
            Some(256 * 1024 * 1024),
            64 * 1024 * 1024,
            Duration::from_secs(10),
            Some(512 * 1024 * 1024),
        );

        assert!(message.contains("name=Textures.ba2"));
        assert!(message.contains("elapsed=65.0s"));
        assert!(message.contains("output=256.0/512.0 MB"));
        assert!(message.contains("approx=50.0%"));
        assert!(message.contains("interval_rate=6.4 MB/s"));
    }

    #[test]
    fn scheduler_reports_progress_while_archive_is_active() {
        let dir = TestDir::new();
        let output_path = dir.path.join("Textures.ba2");
        fs::write(&output_path, vec![0; 1024 * 1024]).unwrap();
        let plans = vec![PlannedArchive {
            label: "Textures".to_string(),
            family: "Textures".to_string(),
            output_name: "Textures.ba2".to_string(),
            entries: Vec::new(),
            texture_archive: true,
        }];
        let progress_output_path = output_path.clone();
        let pack_output_path = output_path.clone();
        let mut messages = Vec::new();

        pack_scheduled_archives(
            &plans,
            1,
            &|_| Ok("fo4dds".to_string()),
            &|_| progress_output_path.clone(),
            &|plan, _workers| {
                fs::write(&pack_output_path, vec![0; 2 * 1024 * 1024]).unwrap();
                std::thread::sleep(Duration::from_millis(75));
                Ok(ArchiveSummary {
                    platform: "pc".to_string(),
                    name: plan.output_name.clone(),
                    file_count: 0,
                    bytes: 0,
                    elapsed_secs: 0.075,
                    direct_pack_stats: None,
                })
            },
            &mut |event| {
                messages.push(event.message);
                Ok(())
            },
        )
        .unwrap();

        assert!(messages.iter().any(|message| {
            message.starts_with("Archive packing progress:")
                && message.contains("name=Textures.ba2")
                && message.contains("output=2.0/1.0 MB")
                && message.contains("approx=200.0%")
        }));
    }

    #[test]
    fn omitted_archive_worker_budget_is_single_threaded() {
        assert_eq!(default_archive_worker_budget(), 1);
    }

    #[test]
    fn texture_groups_cap_archive_concurrency_and_use_worker_budget() {
        let entries = vec![
            entry("Meshes/a.nif", 10),
            entry("Textures/a.dds", 4000),
            entry("Textures/b.dds", 4000),
            entry("Textures/c.dds", 4000),
            entry("Textures/d.dds", 4000),
        ];
        let plans = plan_archive_outputs("B21_Test", &entries, "ba2", "", 8000, "fo4", true)
            .expect("planning should succeed");

        assert!(!plans[0].texture_archive);
        assert_eq!(pack_group_policy(32, &plans, 0), (1, 32));
        assert!(plans[1].texture_archive);
        assert_eq!(pack_group_policy(32, &plans, 1), (2, 32));
        assert_eq!(pack_group_policy(32, &plans[1..2], 0), (1, 16));
        assert_eq!(pack_group_policy(4, &plans, 1), (2, 4));
        assert_eq!(pack_group_policy(1, &plans, 1), (1, 1));
    }

    #[test]
    fn inventory_separates_texture_and_main_families() {
        let dir = TestDir::new();
        let data_dir = dir.path.join("data");
        fs::create_dir_all(data_dir.join("Textures")).unwrap();
        fs::create_dir_all(data_dir.join("Meshes")).unwrap();
        fs::write(data_dir.join("Textures").join("a.dds"), b"dds").unwrap();
        fs::write(data_dir.join("Meshes").join("a.nif"), b"nif").unwrap();

        let config = PackModConfig {
            mod_name: "B21_Test".to_string(),
            mod_dir: dir.path.clone(),
            data_dir,
            strings_dir: dir.path.join("Strings"),
            game: "fo4".to_string(),
            archive_ext: "ba2".to_string(),
            archive_cap: 16 * 1024 * 1024 * 1024,
            expanded_archives: false,
            pc: true,
            xbox: false,
            archive_workers: 1,
            manifest_path: None,
            dry_run: false,
        };

        let entries = inventory_mod_entries(&config, &mut |_| Ok(())).unwrap();
        let families: HashMap<_, _> = entries
            .iter()
            .map(|entry| (entry.relative_path.as_str(), entry.family))
            .collect();

        assert_eq!(families["Textures/a.dds"], ArchiveFamily::Textures);
        assert_eq!(families["Meshes/a.nif"], ArchiveFamily::Meshes);
    }

    #[test]
    fn unreadable_reference_archive_is_ignored() {
        let dir = TestDir::new();
        let archive_path = dir.path.join("B21_Test - Main.ba2");
        fs::write(&archive_path, b"not a ba2").unwrap();

        let members =
            archive_member_list(&archive_path).expect("corrupt archive should be ignored");

        assert!(members.is_empty());
    }

    #[test]
    fn planner_uses_expanded_fo4_label_aliases() {
        let entries = vec![
            entry("Meshes/test.nif", 10),
            entry("Scripts/test.pex", 10),
            entry("readme.txt", 10),
            entry("Textures/test.dds", 10),
        ];
        let plans = plan_archive_outputs("B21_Test", &entries, "ba2", "", 1024 * 1024, "fo4", true)
            .expect("planning should succeed");

        let names: Vec<_> = plans.iter().map(|plan| plan.output_name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "B21_Test - Meshes.ba2",
                "B21_Test - Misc.ba2",
                "B21_Test - Textures.ba2",
            ]
        );
        let misc_entries: Vec<_> = plans[1]
            .entries
            .iter()
            .map(|entry| entry.relative_path.as_str())
            .collect();
        assert_eq!(misc_entries, vec!["Scripts/test.pex", "readme.txt"]);
    }

    #[test]
    fn planner_uses_meshes_extra_for_second_expanded_fo4_mesh_archive() {
        let entries = vec![entry("Meshes/a.nif", 4000), entry("Meshes/b.nif", 4000)];
        let plans = plan_archive_outputs("B21_Test", &entries, "ba2", "", 9000, "fo4", true)
            .expect("planning should succeed");
        let names: Vec<_> = plans.iter().map(|plan| plan.output_name.as_str()).collect();

        assert_eq!(
            names,
            vec!["B21_Test - Meshes.ba2", "B21_Test - MeshesExtra.ba2"]
        );
    }

    #[test]
    fn planner_numbers_additional_expanded_fo4_mesh_archives() {
        let entries = vec![
            entry("Meshes/a.nif", 4000),
            entry("Meshes/b.nif", 4000),
            entry("Meshes/c.nif", 4000),
            entry("Meshes/d.nif", 4000),
        ];
        let plans = plan_archive_outputs("B21_Test", &entries, "ba2", "", 9000, "fo4", true)
            .expect("planning should succeed");
        let names: Vec<_> = plans.iter().map(|plan| plan.output_name.as_str()).collect();

        assert_eq!(
            names,
            vec![
                "B21_Test - Meshes.ba2",
                "B21_Test - MeshesExtra.ba2",
                "B21_Test - MeshesExtra1.ba2",
                "B21_Test - MeshesExtra2.ba2",
            ]
        );
    }

    #[test]
    fn planner_numbers_additional_expanded_fo4_misc_archives() {
        let entries = vec![
            entry("Scripts/a.pex", 4000),
            entry("Scripts/b.pex", 4000),
            entry("Scripts/c.pex", 4000),
        ];
        let plans = plan_archive_outputs("B21_Test", &entries, "ba2", "", 9000, "fo4", true)
            .expect("planning should succeed");
        let names: Vec<_> = plans.iter().map(|plan| plan.output_name.as_str()).collect();

        assert_eq!(
            names,
            vec![
                "B21_Test - Misc.ba2",
                "B21_Test - Misc1.ba2",
                "B21_Test - Misc2.ba2",
            ]
        );
    }

    #[test]
    fn pack_progress_reports_per_format_levels() {
        let dir = TestDir::new();
        let data_dir = dir.path.join("data");
        fs::create_dir_all(data_dir.join("Textures")).unwrap();
        fs::create_dir_all(data_dir.join("Meshes")).unwrap();
        // A minimal valid DDS is needed for the DX10 writer; copy the test fixture.
        fs::copy(
            std::path::Path::new("data/fo4_dds_test/Fence006_1K_Roughness.dds"),
            data_dir.join("Textures").join("a.dds"),
        )
        .unwrap();
        fs::write(data_dir.join("Meshes").join("a.nif"), b"nif").unwrap();

        let config = PackModConfig {
            mod_name: "B21_Test".to_string(),
            mod_dir: dir.path.clone(),
            data_dir,
            strings_dir: dir.path.join("Strings"),
            game: "fo4".to_string(),
            archive_ext: "ba2".to_string(),
            archive_cap: 16 * 1024 * 1024 * 1024,
            expanded_archives: false,
            pc: true,
            xbox: false,
            archive_workers: 1,
            manifest_path: None,
            dry_run: false,
        };

        let mut messages = Vec::new();
        pack_mod_archives(&config, |event| {
            messages.push(event.message);
            Ok(())
        })
        .unwrap();

        assert!(
            messages
                .iter()
                .any(|m| m.contains("Textures") && m.contains("compression=libdeflate:4"))
        );
        assert!(
            messages
                .iter()
                .any(|m| m.contains("Main") && m.contains("compression=zlib:6"))
        );
        assert!(messages.iter().any(|m| {
            m.contains("Archive packed native:")
                && m.contains("Textures")
                && m.contains("payloads=1")
                && m.contains("writer_io=")
                && m.contains("writer_idle=")
                && m.contains("source_read_worker=")
                && m.contains("compression_worker=")
                && m.contains("memory_budget=")
                && m.contains("throttle_wait=")
        }));
    }

    #[test]
    fn pack_progress_reports_archive_worker_split() {
        let dir = TestDir::new();
        let data_dir = dir.path.join("data");
        fs::create_dir_all(data_dir.join("Meshes")).unwrap();
        fs::create_dir_all(data_dir.join("Scripts")).unwrap();
        fs::create_dir_all(data_dir.join("Materials")).unwrap();
        fs::write(data_dir.join("Meshes").join("a.nif"), b"nif").unwrap();
        fs::write(data_dir.join("Scripts").join("a.pex"), b"pex").unwrap();
        fs::write(data_dir.join("Materials").join("a.bgsm"), b"bgsm").unwrap();

        let config = PackModConfig {
            mod_name: "B21_Test".to_string(),
            mod_dir: dir.path.clone(),
            data_dir,
            strings_dir: dir.path.join("Strings"),
            game: "fo4".to_string(),
            archive_ext: "ba2".to_string(),
            archive_cap: 16 * 1024 * 1024 * 1024,
            expanded_archives: true,
            pc: true,
            xbox: false,
            archive_workers: 6,
            manifest_path: None,
            dry_run: false,
        };

        let mut messages = Vec::new();
        pack_mod_archives(&config, |event| {
            messages.push(event.message);
            Ok(())
        })
        .unwrap();

        assert!(messages.iter().any(|m| {
            m.contains("Packing archives with total_workers=6")
                && m.contains("general_concurrency=2")
                && m.contains("texture_concurrency=2")
        }));
        let pack_starts: Vec<_> = messages
            .iter()
            .filter(|message| message.starts_with("Packing archive "))
            .collect();
        assert_eq!(pack_starts.len(), 3);
        assert_eq!(
            pack_starts
                .iter()
                .filter(|message| message.contains("workers=3/6 archive_concurrency=2"))
                .count(),
            3
        );
        assert_eq!(
            pack_starts
                .iter()
                .filter(|message| message.contains("workers=6/6 archive_concurrency=1"))
                .count(),
            0
        );
    }

    #[test]
    fn preplanned_archives_use_shared_worker_scheduler() {
        let dir = TestDir::new();
        let source_path = dir.path.join("source.nif");
        fs::write(&source_path, b"nif").unwrap();
        let plans: Vec<_> = (0..3)
            .map(|index| PackArchivePlan {
                output_path: dir.path.join(format!("B21_Test - Meshes{index}.ba2")),
                output_name: format!("B21_Test - Meshes{index}.ba2"),
                archive_type: "fo4".to_string(),
                entries: vec![PackEntrySpec {
                    source_path: source_path.clone(),
                    archive_path: format!("Meshes/test{index}.nif"),
                    source_size: Some(3),
                }],
                input_bytes: 3,
                texture_archive: false,
            })
            .collect();

        let mut messages = Vec::new();
        let summaries = pack_archive_plans(&plans, 6, |event| {
            messages.push(event.message);
            Ok(())
        })
        .unwrap();

        assert_eq!(summaries.len(), 3);
        assert!(plans.iter().all(|plan| plan.output_path.is_file()));
        assert!(messages.iter().any(|message| {
            message.contains("Packing archives with total_workers=6")
                && message.contains("general_concurrency=2")
                && message.contains("texture_concurrency=2")
        }));
        let pack_starts: Vec<_> = messages
            .iter()
            .filter(|message| message.starts_with("Packing archive "))
            .collect();
        assert_eq!(pack_starts.len(), 3);
        assert_eq!(
            pack_starts
                .iter()
                .filter(|message| message.contains("workers=3/6 archive_concurrency=2"))
                .count(),
            3
        );
        assert_eq!(
            pack_starts
                .iter()
                .filter(|message| message.contains("workers=6/6 archive_concurrency=1"))
                .count(),
            0
        );
    }

    #[test]
    fn terrain_paths_route_land_assets_to_generic_families() {
        let f = classify_archive_family;
        assert_eq!(
            f("Textures/Terrain/Appalachia/lswamprocks01_g.dds"),
            ArchiveFamily::Textures
        );
        assert_eq!(
            f("Materials/Terrain/Appalachia/blend.bgsm"),
            ArchiveFamily::Materials
        );
        assert_eq!(f("Terrain/Appalachia.btd4"), ArchiveFamily::Main);
        // lodgen terrain-LOD quad tiles stay LOD, not Terrain.
        assert_eq!(
            f("Textures/Terrain/Appalachia/appalachia.16.-110.-77.dds"),
            ArchiveFamily::Lod
        );
        assert_eq!(
            f("Textures/Terrain/Appalachia/appalachia.16.-110.-77_msn.dds"),
            ArchiveFamily::Lod
        );
        // object atlas stays LOD.
        assert_eq!(
            f("Textures/Terrain/Appalachia/Objects/hybrid/lod/x_lod_0_d.dds"),
            ArchiveFamily::Lod
        );
        // existing .bto rule unaffected.
        assert_eq!(
            f("Meshes/Terrain/Appalachia/Objects/App.4.0.0.bto"),
            ArchiveFamily::Lod
        );
        // object textures/materials unaffected.
        assert_eq!(f("Textures/Weapons/gun_d.dds"), ArchiveFamily::Textures);
        assert_eq!(f("Materials/Weapons/gun.bgsm"), ArchiveFamily::Materials);
    }

    #[test]
    fn planner_routes_land_assets_to_generic_archives() {
        let entries = vec![
            entry("Meshes/Terrain/Appalachia/Objects/a.bto", 10),
            entry("Textures/Terrain/Appalachia/Appalachia.4.0.0.dds", 10),
            entry("Materials/Terrain/Appalachia/blend.bgsm", 10),
            entry("Textures/Terrain/Appalachia/lswamprocks01_d.dds", 10),
        ];
        let plans = plan_archive_outputs("B21_Test", &entries, "ba2", "", 1024 * 1024, "fo4", true)
            .expect("planning should succeed");
        let by_label: HashMap<_, _> = plans
            .iter()
            .map(|plan| (plan.label.as_str(), plan))
            .collect();

        assert!(!by_label["LOD"].texture_archive);
        assert_eq!(
            by_label["LOD"]
                .entries
                .iter()
                .map(|entry| entry.relative_path.as_str())
                .collect::<Vec<_>>(),
            vec!["Meshes/Terrain/Appalachia/Objects/a.bto"]
        );
        assert!(by_label["LODTextures"].texture_archive);
        assert_eq!(
            by_label["LODTextures"]
                .entries
                .iter()
                .map(|entry| entry.relative_path.as_str())
                .collect::<Vec<_>>(),
            vec!["Textures/Terrain/Appalachia/Appalachia.4.0.0.dds"]
        );
        assert!(!by_label["Materials"].texture_archive);
        assert_eq!(
            by_label["Materials"]
                .entries
                .iter()
                .map(|entry| entry.relative_path.as_str())
                .collect::<Vec<_>>(),
            vec!["Materials/Terrain/Appalachia/blend.bgsm"]
        );
        assert!(by_label["Textures"].texture_archive);
        assert_eq!(
            by_label["Textures"]
                .entries
                .iter()
                .map(|entry| entry.relative_path.as_str())
                .collect::<Vec<_>>(),
            vec!["Textures/Terrain/Appalachia/lswamprocks01_d.dds"]
        );
    }

    #[test]
    fn planner_compacts_lod_and_terrain_dds_into_textures() {
        let entries = vec![
            entry("Meshes/Terrain/Appalachia/Objects/a.bto", 10),
            entry("Textures/Actors/a.dds", 10),
            entry("Textures/Terrain/Appalachia/Appalachia.4.0.0.dds", 10),
            entry("Materials/Terrain/Appalachia/blend.bgsm", 10),
            entry("Textures/Terrain/Appalachia/lswamprocks01_d.dds", 10),
        ];
        let plans =
            plan_archive_outputs("B21_Test", &entries, "ba2", "", 1024 * 1024, "fo4", false)
                .expect("planning should succeed");
        let by_label: HashMap<_, _> = plans
            .iter()
            .map(|plan| (plan.label.as_str(), plan))
            .collect();

        assert_eq!(plans.len(), 2);
        assert_eq!(plans[0].label, "Main");
        assert_eq!(plans[1].label, "Textures");
        assert_eq!(
            by_label["Main"]
                .entries
                .iter()
                .map(|entry| entry.relative_path.as_str())
                .collect::<Vec<_>>(),
            vec![
                "Meshes/Terrain/Appalachia/Objects/a.bto",
                "Materials/Terrain/Appalachia/blend.bgsm",
            ]
        );
        assert_eq!(
            by_label["Textures"]
                .entries
                .iter()
                .map(|entry| entry.relative_path.as_str())
                .collect::<Vec<_>>(),
            vec![
                "Textures/Actors/a.dds",
                "Textures/Terrain/Appalachia/Appalachia.4.0.0.dds",
                "Textures/Terrain/Appalachia/lswamprocks01_d.dds",
            ]
        );
        assert!(by_label["Textures"].texture_archive);
        assert!(!by_label.contains_key("LODTextures"));
        assert!(!by_label.contains_key("TerrainTextures"));
    }

    #[test]
    fn classify_strips_leading_data_prefix() {
        assert_eq!(
            classify_archive_family("data/Textures/a.dds"),
            ArchiveFamily::Textures
        );
        assert_eq!(
            classify_archive_family("Data/Meshes/a.nif"),
            ArchiveFamily::Meshes
        );
        assert_eq!(
            classify_archive_family("Textures/a.dds"),
            ArchiveFamily::Textures
        );
        assert_eq!(
            classify_archive_family("Meshes/AnimTextData/AnimationEventInfo/123.txt"),
            ArchiveFamily::Meshes
        );
    }

    #[test]
    fn plan_archives_public_returns_family_and_entries() {
        let entries = vec![
            ("Meshes/a.nif".to_string(), "/src/a.nif".to_string(), 10u64),
            (
                "Textures/a.dds".to_string(),
                "/src/a.dds".to_string(),
                20u64,
            ),
        ];
        let plans = plan_archives_public(
            "B21_Test",
            &entries,
            "ba2",
            "",
            16 * 1024 * 1024 * 1024,
            "fo4",
            false,
        )
        .expect("planning should succeed");
        let by_label: std::collections::HashMap<_, _> =
            plans.iter().map(|p| (p.label.as_str(), p)).collect();
        assert_eq!(by_label["Main"].family, "Main");
        assert_eq!(by_label["Textures"].family, "Textures");
        assert!(by_label["Textures"].texture_archive);
        assert_eq!(
            by_label["Textures"].entries,
            vec![(
                "Textures/a.dds".to_string(),
                "/src/a.dds".to_string(),
                20u64
            )]
        );
    }
}
