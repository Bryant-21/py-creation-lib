use crate::{
    FileFormat, Reader as _, fo4,
    pack::{self, PackEntrySpec},
    tes4,
};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    time::Instant,
};
use walkdir::WalkDir;

type PackResult<T> = Result<T, String>;

const ARCHIVE_HEADER_OVERHEAD: u64 = 4096;
const ENTRY_OVERHEAD: u64 = 512;
const COMPRESSIBLE_BA2_ESTIMATE_NUMERATOR: u64 = 2;
const COMPRESSIBLE_BA2_ESTIMATE_DENOMINATOR: u64 = 3;
const MAX_ARCHIVE_PACK_CONCURRENCY: usize = 6;
const MAX_TEXTURE_ARCHIVE_CONCURRENCY: usize = 2;
const MAX_TEXTURE_PACK_WORKERS: usize = 8;
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
// etc.), then ".dds". Per the verified audit
// (docs/superpowers/specs/appalachia_family_map_verified.md §2c) this is
// collision-free with convert_terrain output: convert_terrain runs its
// texture-set name through safe_name, which replaces "." and "-" with "_",
// so a convert_terrain filename can never contain this dot-separated
// signed-integer triple.
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

    // Terrain (land) assets — kept separate so upgrade-gen can reuse/regenerate
    // terrain independently of object textures/LOD. Predicate verified against
    // the deployed Appalachia tree (appalachia_family_map_verified.md §2c):
    // convert_terrain output -> Terrain; ALL lodgen output (terrain-LOD quad
    // tiles + object atlas) -> LOD, since a Terrain-only rebuild skips lodgen
    // and would otherwise ship a Terrain archive missing them.
    if suffix == ".btd4" {
        return ArchiveFamily::Terrain;
    }
    if parts.first() == Some(&"textures") && parts.get(1) == Some(&"terrain") {
        let basename = parts.last().copied().unwrap_or("");
        if parts.contains(&"objects") {
            return ArchiveFamily::Lod;
        }
        if parts.get(2) == Some(&"lodgen") {
            return ArchiveFamily::Lod;
        }
        if is_lodgen_quad_tile(basename) {
            return ArchiveFamily::Lod;
        }
        return ArchiveFamily::Terrain;
    }
    if parts.first() == Some(&"materials") && parts.get(1) == Some(&"terrain") {
        return ArchiveFamily::Terrain;
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
        let estimated_size =
            estimate_planned_archive_size(std::slice::from_ref(&entry), archive_ext);
        if estimated_size > cap {
            return Err(format!(
                "{} ({} bytes source, {} bytes estimated packed) exceeds archive cap, exceeding archive max size {} bytes",
                entry.relative_path, entry.size, estimated_size, cap
            ));
        }
        by_family.entry(entry.family).or_default().push(entry);
    }

    let mut planned = Vec::new();
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
    if main_entries.is_empty() {
        return Ok(planned);
    }
    if estimate_planned_archive_size(&main_entries, archive_ext) <= cap {
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
        return Ok(planned);
    }

    let strings_entries = by_family
        .remove(&ArchiveFamily::Strings)
        .unwrap_or_default();
    if !strings_entries.is_empty() {
        let main = by_family.entry(ArchiveFamily::Main).or_default();
        let old_main = std::mem::take(main);
        *main = strings_entries.into_iter().chain(old_main).collect();
    }
    let mut main_archives = Vec::new();
    for family in MAIN_FAMILY_ORDER {
        let family_entries = by_family.get(&family).cloned().unwrap_or_default();
        if family_entries.is_empty() {
            continue;
        }
        let label = family_label(family);
        if estimate_planned_archive_size(&family_entries, archive_ext) <= cap {
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
                None,
            )?);
        }
    }
    main_archives.extend(planned);
    Ok(main_archives)
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

fn native_archive_type(game: &str, texture_archive: bool, xbox: bool) -> PackResult<String> {
    if xbox && game == "fo4" {
        return Ok(if texture_archive {
            "fo4xboxdds"
        } else {
            "fo4xbox"
        }
        .to_string());
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
        })
        .collect()
}

fn default_archive_worker_budget() -> usize {
    std::thread::available_parallelism()
        .map(|value| value.get())
        .unwrap_or(1)
        .max(1)
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

fn pack_batch_policy<T: ScheduledArchivePlan>(
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
    let active_worker_budget = if texture_archive {
        worker_budget.min(MAX_TEXTURE_PACK_WORKERS)
    } else {
        worker_budget
    }
    .max(1);
    let batch_len = group_len
        .min(concurrency_cap)
        .min(active_worker_budget)
        .max(1);
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
    if let Some(stripped) = label.strip_suffix("_xbox") {
        label = stripped;
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
        let platform_match = if platform_suffix == "_xbox" {
            path.file_stem()
                .and_then(|value| value.to_str())
                .is_some_and(|stem| stem.ends_with("_xbox"))
        } else {
            path.file_stem()
                .and_then(|value| value.to_str())
                .is_some_and(|stem| !stem.ends_with("_xbox"))
        };
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
    let archive_type = native_archive_type(&config.game, plan.texture_archive, false)?;
    let level = crate::pack::archive_type_default_level(&archive_type);
    let reference_manifest =
        write_reference_manifest(&output_path, temp_manifest_dir, &plan.label)?;
    let manifest_path = reference_manifest
        .as_deref()
        .or(config.manifest_path.as_deref());
    let specs = planned_entry_specs(plan);
    pack::pack_archive_entries(
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
    })
}

fn pack_archive_plan(
    plan: &PackArchivePlan,
    workers_for_archive: usize,
) -> PackResult<ArchiveSummary> {
    let started = Instant::now();
    let level = crate::pack::archive_type_default_level(&plan.archive_type);
    pack::pack_archive_entries(
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
    })
}

fn pack_scheduled_archives<T: ScheduledArchivePlan + Sync>(
    plans: &[T],
    worker_budget: usize,
    archive_type_for: &(impl Fn(&T) -> PackResult<String> + Sync),
    pack_one: &(impl Fn(&T, usize) -> PackResult<ArchiveSummary> + Sync),
    progress: &mut impl FnMut(PackProgress) -> PackResult<()>,
) -> PackResult<Vec<ArchiveSummary>> {
    let worker_budget = worker_budget.max(1);
    if plans.len() > 1 {
        progress(PackProgress {
            phase: "pack",
            platform: "pc".to_string(),
            message: format!(
                "Packing archives with total_workers={worker_budget} general_concurrency={} texture_concurrency={} texture_worker_cap={}",
                MAX_ARCHIVE_PACK_CONCURRENCY.min(worker_budget),
                MAX_TEXTURE_ARCHIVE_CONCURRENCY.min(worker_budget),
                MAX_TEXTURE_PACK_WORKERS.min(worker_budget),
            ),
            completed: 0,
            total: plans.len(),
        })?;
    }

    let mut summaries = Vec::with_capacity(plans.len());
    let mut completed = 0usize;
    let mut batch_start = 0usize;
    while batch_start < plans.len() {
        let (batch_len, active_worker_budget) =
            pack_batch_policy(worker_budget, plans, batch_start);
        let batch_end = batch_start + batch_len;
        let chunk = &plans[batch_start..batch_end];
        let allocation = allocate_archive_workers(active_worker_budget, chunk.len());
        for (slot, plan) in chunk.iter().enumerate() {
            let workers_for_archive = archive_worker_count(allocation, slot);
            let plan_index = batch_start + slot;
            let plan_bytes = plan.input_bytes();
            let archive_type = archive_type_for(plan)?;
            let level = crate::pack::archive_type_default_level(&archive_type);
            progress(PackProgress {
                phase: "pack",
                platform: "pc".to_string(),
                message: format!(
                    "Packing archive {} ({}/{}) files={} bytes={:.1} MB workers={}/{} archive_concurrency={} compression=zlib:{level}",
                    plan.output_name(),
                    plan_index + 1,
                    plans.len(),
                    plan.file_count(),
                    plan_bytes as f64 / (1024.0 * 1024.0),
                    workers_for_archive,
                    active_worker_budget,
                    allocation.archive_concurrency
                ),
                completed,
                total: plans.len(),
            })?;
        }

        let chunk_summaries = std::thread::scope(|scope| -> PackResult<Vec<ArchiveSummary>> {
            let mut handles = Vec::with_capacity(chunk.len());
            for (slot, plan) in chunk.iter().enumerate() {
                let workers_for_archive = archive_worker_count(allocation, slot);
                handles.push(scope.spawn(move || pack_one(plan, workers_for_archive)));
            }

            let mut chunk_summaries = Vec::with_capacity(handles.len());
            for handle in handles {
                match handle.join() {
                    Ok(result) => chunk_summaries.push(result?),
                    Err(_) => return Err("archive pack worker panicked".to_string()),
                }
            }
            Ok(chunk_summaries)
        })?;

        for summary in chunk_summaries {
            completed += 1;
            progress(PackProgress {
                phase: "pack",
                platform: "pc".to_string(),
                message: format!(
                    "Archive packed native: name={} files={} bytes={:.1} MB elapsed={:.3}s",
                    summary.name,
                    summary.file_count,
                    summary.bytes as f64 / (1024.0 * 1024.0),
                    summary.elapsed_secs
                ),
                completed,
                total: plans.len(),
            })?;
            summaries.push(summary);
        }
        batch_start = batch_end;
    }
    Ok(summaries)
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
        |plan: &PlannedArchive| native_archive_type(&config.game, plan.texture_archive, false);
    let pack_one = |plan: &PlannedArchive, workers_for_archive| {
        pack_planned_archive(config, plan, &temp_manifest_dir, workers_for_archive)
    };
    let summaries = pack_scheduled_archives(
        &plans,
        worker_budget,
        &archive_type_for,
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
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

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
        let plans = plan_archive_outputs("B21_Test", &entries, "ba2", "", 9000, "fo4", false)
            .expect("planning should succeed");
        let names: Vec<_> = plans.iter().map(|plan| plan.output_name.as_str()).collect();

        assert_eq!(
            names,
            vec![
                "B21_Test - Main.ba2",
                "B21_Test - Textures1.ba2",
                "B21_Test - Textures2.ba2",
            ]
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
                archive_concurrency: 3,
                workers_per_archive: 6,
                extra_worker_archives: 2,
            }
        );
        assert_eq!(
            (0..allocation.archive_concurrency)
                .map(|slot| archive_worker_count(allocation, slot))
                .collect::<Vec<_>>(),
            vec![7, 7, 6]
        );

        let allocation = allocate_archive_workers(2, 4);
        assert_eq!(allocation.archive_concurrency, 2);
        assert_eq!(
            (0..allocation.archive_concurrency)
                .map(|slot| archive_worker_count(allocation, slot))
                .collect::<Vec<_>>(),
            vec![1, 1]
        );

        let allocation = allocate_archive_workers(20, 17);
        assert_eq!(allocation.archive_concurrency, MAX_ARCHIVE_PACK_CONCURRENCY);
        assert_eq!(
            (0..allocation.archive_concurrency)
                .map(|slot| archive_worker_count(allocation, slot))
                .collect::<Vec<_>>(),
            vec![4, 4, 3, 3, 3, 3]
        );
    }

    #[test]
    fn texture_batches_cap_archive_and_worker_concurrency() {
        let entries = vec![
            entry("Meshes/a.nif", 10),
            entry("Textures/a.dds", 4000),
            entry("Textures/b.dds", 4000),
            entry("Textures/c.dds", 4000),
            entry("Textures/d.dds", 4000),
        ];
        let plans = plan_archive_outputs("B21_Test", &entries, "ba2", "", 8000, "fo4", false)
            .expect("planning should succeed");

        assert!(!plans[0].texture_archive);
        assert_eq!(pack_batch_policy(32, &plans, 0), (1, 32));
        assert!(plans[1].texture_archive);
        assert_eq!(pack_batch_policy(32, &plans, 1), (2, 8));
        assert_eq!(pack_batch_policy(4, &plans, 1), (2, 4));
        assert_eq!(pack_batch_policy(1, &plans, 1), (1, 1));
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
                .any(|m| m.contains("Textures") && m.contains("compression=zlib:4"))
        );
        assert!(
            messages
                .iter()
                .any(|m| m.contains("Main") && m.contains("compression=zlib:6"))
        );
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
                && m.contains("general_concurrency=6")
                && m.contains("texture_concurrency=2")
        }));
        let pack_starts: Vec<_> = messages
            .iter()
            .filter(|message| message.starts_with("Packing archive "))
            .collect();
        assert_eq!(pack_starts.len(), 3);
        assert!(
            pack_starts
                .iter()
                .all(|message| message.contains("workers=2/6 archive_concurrency=3"))
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
                && message.contains("general_concurrency=6")
                && message.contains("texture_concurrency=2")
        }));
        let pack_starts: Vec<_> = messages
            .iter()
            .filter(|message| message.starts_with("Packing archive "))
            .collect();
        assert_eq!(pack_starts.len(), 3);
        assert!(
            pack_starts
                .iter()
                .all(|message| message.contains("workers=2/6 archive_concurrency=3"))
        );
    }

    #[test]
    fn terrain_family_splits_land_assets_from_lodgen_output() {
        let f = classify_archive_family;
        assert_eq!(
            f("Textures/Terrain/Appalachia/lswamprocks01_g.dds"),
            ArchiveFamily::Terrain
        );
        assert_eq!(
            f("Materials/Terrain/Appalachia/blend.bgsm"),
            ArchiveFamily::Terrain
        );
        assert_eq!(f("Terrain/Appalachia.btd4"), ArchiveFamily::Terrain);
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
    fn planner_splits_lod_and_terrain_dds_into_texture_archives() {
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
        assert!(!by_label["Terrain"].texture_archive);
        assert_eq!(
            by_label["Terrain"]
                .entries
                .iter()
                .map(|entry| entry.relative_path.as_str())
                .collect::<Vec<_>>(),
            vec!["Materials/Terrain/Appalachia/blend.bgsm"]
        );
        assert!(by_label["TerrainTextures"].texture_archive);
        assert_eq!(
            by_label["TerrainTextures"]
                .entries
                .iter()
                .map(|entry| entry.relative_path.as_str())
                .collect::<Vec<_>>(),
            vec!["Textures/Terrain/Appalachia/lswamprocks01_d.dds"]
        );
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
