//! Offline extractors that feed the AnimTextData bucket writers (CK-free).
//!
//! Sources:
//! * ClipGeneratorData: the behavior `.hkx` (`hkbClipGenerator` + its
//!   `hkbClipTriggerArray` + `hkbBehaviorGraphStringData.eventNames`).
//! * DynamicIdleData: the FO4 IDLE record's `GNAM` animation filename (a `*` glob),
//!   expanded against the converted mod's on-disk `Animations` dir.
//! * Main project manifest: the project + character + root-behavior `.hkx` string
//!   data, with on-disk fallbacks.
//!
//! Content count/order is not CK's (a behavior-arena traversal artifact); each bucket's
//! byte format is exact (see `bucket_files.rs`).
//!
//! Trigger arrays and `eventNames`/`characterFilenames` need the TAG0 8-byte
//! pointer-array stride; a 4-byte stride returns them short or half-null, which yields
//! fewer triggers or a disk-fallback file list rather than wrong bytes.

use std::collections::HashSet;
use std::path::Path;

use havok_native::hkx::types::HkxValue;

use super::bucket_files::{ClipGenEntry, ClipTrigger};

// ---------------------------------------------------------------------------
// Small HkxValue helpers
// ---------------------------------------------------------------------------

fn as_f32(v: &HkxValue) -> Option<f32> {
    match v {
        HkxValue::F32(f) | HkxValue::Half(f) => Some(*f),
        HkxValue::F32List(l) => l.first().copied(),
        _ => None,
    }
}

fn as_i64(v: &HkxValue) -> Option<i64> {
    match v {
        HkxValue::I8(i) => Some(*i as i64),
        HkxValue::U8(i) => Some(*i as i64),
        HkxValue::I16(i) => Some(*i as i64),
        HkxValue::U16(i) => Some(*i as i64),
        HkxValue::I32(i) => Some(*i as i64),
        HkxValue::U32(i) => Some(*i as i64),
        HkxValue::I64(i) => Some(*i),
        HkxValue::U64(i) => Some(*i as i64),
        _ => None,
    }
}

fn as_bool(v: &HkxValue) -> Option<bool> {
    match v {
        HkxValue::Bool(b) => Some(*b),
        _ => as_i64(v).map(|i| i != 0),
    }
}

fn as_str(v: &HkxValue) -> Option<&str> {
    match v {
        HkxValue::String { value, .. } => Some(value.as_str()),
        _ => None,
    }
}

/// Basename of an `animationName`, no extension. `r"Animations\Idle.hkt"` → `Idle`.
fn anim_basename_no_ext(animation_name: &str) -> String {
    let norm = animation_name.replace('/', "\\");
    let last = norm.rsplit('\\').next().unwrap_or(&norm);
    match last.rfind('.') {
        Some(d) => last[..d].to_string(),
        None => last.to_string(),
    }
}

// ---------------------------------------------------------------------------
// ClipGeneratorData extraction
// ---------------------------------------------------------------------------

/// Collect `eventNames` from the behavior's `hkbBehaviorGraphStringData`.
fn collect_event_names(objects: &[havok_native::hkx::HkxObject]) -> Vec<String> {
    for obj in objects {
        if obj.class_name != "hkbBehaviorGraphStringData" {
            continue;
        }
        for m in &obj.members {
            if m.name == "eventNames" {
                if let HkxValue::Array(items) = &m.value {
                    return items
                        .iter()
                        .map(|v| as_str(v).unwrap_or("").to_string())
                        .collect();
                }
            }
        }
    }
    Vec::new()
}

/// Object indices of clips driven by a `DynamicAnim*`/`*TaggingGenerator` — these
/// store `anim=""` + the dynamic flag (RE: `text_clipgeneratordata.md` risk #1).
fn collect_dynamic_clip_targets(objects: &[havok_native::hkx::HkxObject]) -> HashSet<usize> {
    fn gather_ptrs(v: &HkxValue, out: &mut HashSet<usize>) {
        match v {
            HkxValue::Pointer(Some(i)) => {
                out.insert(*i);
            }
            HkxValue::Array(items) => items.iter().for_each(|x| gather_ptrs(x, out)),
            HkxValue::Object(ms) | HkxValue::TypedObject { members: ms, .. } => {
                ms.iter().for_each(|m| gather_ptrs(&m.value, out))
            }
            _ => {}
        }
    }
    let mut targets = HashSet::new();
    for obj in objects {
        let c = &obj.class_name;
        if c.contains("Dynamic") || c.contains("Tagging") {
            for m in &obj.members {
                gather_ptrs(&m.value, &mut targets);
            }
        }
    }
    // Keep only indices that actually point at a clip generator.
    targets
        .into_iter()
        .filter(|&i| {
            objects
                .get(i)
                .is_some_and(|o| o.class_name == "hkbClipGenerator")
        })
        .collect()
}

/// Resolve the trigger list of one `hkbClipTriggerArray` object (by index).
fn extract_triggers(
    objects: &[havok_native::hkx::HkxObject],
    array_idx: usize,
    event_names: &[String],
) -> Vec<ClipTrigger> {
    let Some(arr) = objects.get(array_idx) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for m in &arr.members {
        if m.name != "triggers" {
            continue;
        }
        let HkxValue::Array(items) = &m.value else {
            continue;
        };
        for it in items {
            // Each entry is an inline hkbClipTrigger (Object) or a pointer to one.
            let members = match it {
                HkxValue::Pointer(Some(i)) => match objects.get(*i) {
                    Some(o) => o.members.as_slice(),
                    None => continue,
                },
                _ => match it.as_object_members() {
                    Some(m) => m,
                    None => continue,
                },
            };
            let mut local_time = 0.0f32;
            let mut relative = false;
            let mut event_id: i64 = -1;
            for tm in members {
                match tm.name.as_str() {
                    "localTime" => local_time = as_f32(&tm.value).unwrap_or(0.0),
                    "relativeToEndOfClip" => relative = as_bool(&tm.value).unwrap_or(false),
                    "event" => {
                        if let Some(ems) = tm.value.as_object_members() {
                            for em in ems {
                                if em.name == "id" {
                                    event_id = as_i64(&em.value).unwrap_or(-1);
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            let name = usize::try_from(event_id)
                .ok()
                .and_then(|i| event_names.get(i))
                .cloned()
                .unwrap_or_default();
            // RE: stored time is localTime, sign-flipped when relativeToEndOfClip.
            let time = if relative { -local_time } else { local_time };
            out.push(ClipTrigger {
                name,
                time,
                flag: u8::from(relative),
            });
        }
    }
    out
}

/// Extract the `ClipGeneratorData` entry list from a behavior `.hkx`.
///
/// Every `hkbClipGenerator` becomes one entry, a superset of CK's content (CK keeps
/// only animation-event-target clips); nothing is fabricated. Empty if the behavior
/// cannot be read (caller skips the file).
pub fn clip_generator_entries(behavior_file: &Path) -> Vec<ClipGenEntry> {
    let Some(hkx) = super::hkx_cache::behavior_packfile(behavior_file) else {
        return Vec::new();
    };
    let objects = hkx.objects();
    let event_names = collect_event_names(objects);
    let dynamic = collect_dynamic_clip_targets(objects);

    let mut entries = Vec::new();
    for (idx, obj) in objects.iter().enumerate() {
        if obj.class_name != "hkbClipGenerator" {
            continue;
        }
        let mut clip_name = String::new();
        let mut anim = String::new();
        let mut playback_speed = 1.0f32;
        let mut crop_start = 0.0f32;
        let mut crop_end = 0.0f32;
        let mut trig_idx: Option<usize> = None;
        for m in &obj.members {
            match m.name.as_str() {
                "name" => clip_name = as_str(&m.value).unwrap_or("").to_string(),
                "animationName" => {
                    anim = as_str(&m.value)
                        .map(anim_basename_no_ext)
                        .unwrap_or_default()
                }
                "playbackSpeed" => playback_speed = as_f32(&m.value).unwrap_or(1.0),
                "cropStartAmountLocalTime" => crop_start = as_f32(&m.value).unwrap_or(0.0),
                "cropEndAmountLocalTime" => crop_end = as_f32(&m.value).unwrap_or(0.0),
                "triggers" => {
                    if let HkxValue::Pointer(Some(i)) = &m.value {
                        trig_idx = Some(*i);
                    }
                }
                _ => {}
            }
        }
        let is_dynamic = dynamic.contains(&idx);
        let triggers = trig_idx
            .map(|i| extract_triggers(objects, i, &event_names))
            .unwrap_or_default();
        entries.push(ClipGenEntry {
            clip_name,
            anim_name: if is_dynamic { String::new() } else { anim },
            playback_speed,
            crop_start,
            crop_end,
            x0: 0,
            dynamic: is_dynamic,
            triggers,
        });
    }
    entries
}

// ---------------------------------------------------------------------------
// Path / name helpers (SyncAnimData filename, manifest project name)
// ---------------------------------------------------------------------------

/// `Actors\<Race>\Behaviors\X.hkx` → `Actors\<Race>`.
pub fn race_dir_of(core_behavior: &str) -> Option<String> {
    let norm = core_behavior.replace('/', "\\");
    let parts: Vec<&str> = norm.split('\\').filter(|s| !s.is_empty()).collect();
    if parts.len() >= 3 {
        Some(parts[..parts.len() - 2].join("\\"))
    } else {
        None
    }
}

/// `Actors\<Race>` → `<Race>` (the SyncAnimData project name), from a race DIR rather
/// than a core-behavior path.
pub fn race_name_of_dir(race_dir: &str) -> Option<String> {
    race_dir
        .replace('/', "\\")
        .rsplit('\\')
        .find(|part| !part.is_empty())
        .map(str::to_string)
}

/// `Actors\<Race>\Behaviors\X.hkx` → `<Race>` (the SyncAnimData project name).
pub fn race_name_of(core_behavior: &str) -> Option<String> {
    let norm = core_behavior.replace('/', "\\");
    let parts: Vec<&str> = norm.split('\\').filter(|s| !s.is_empty()).collect();
    if parts.len() >= 3 {
        Some(parts[parts.len() - 3].to_string())
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// DynamicIdleData: expand an IDLE GNAM wildcard against on-disk Animations
// ---------------------------------------------------------------------------

/// Expand a wildcard IDLE `GNAM` path (e.g. `Actors\X\Animations\Idle_Flavor*.hkx`)
/// against `meshes_root`, stripping the extension. Empty if the path has no `*`, the dir
/// is absent, or nothing matches.
///
/// CK canonicalises the result (RE: `dynidle_6th.md`): the directory takes the on-disk
/// case (a `SnallyGaster` typo resolves to disk `Snallygaster`); the filename takes the
/// GNAM prefix casing (`Idle_Flavor`) plus only the wildcard-matched part from disk (`1`).
/// So `Actors\SnallyGaster\Animations\Idle_Flavor*.hkx` over disk `idle_flavor1.hkx`
/// emits `Actors\Snallygaster\Animations\Idle_Flavor1`.
pub fn expand_idle_glob(gnam: &str, meshes_root: &Path) -> Vec<String> {
    let norm = gnam.replace('/', "\\");
    if !norm.contains('*') {
        return Vec::new();
    }
    let Some(slash) = norm.rfind('\\') else {
        return Vec::new();
    };
    let dir = &norm[..slash];
    let pattern = &norm[slash + 1..];
    let star = pattern.find('*').unwrap();
    // Keep the GNAM template prefix casing for OUTPUT; lowercase a copy for MATCHING.
    let prefix_template = &pattern[..star];
    let prefix_lc = prefix_template.to_ascii_lowercase();
    // Suffix after '*', minus the extension we strip anyway.
    let after = &pattern[star + 1..];
    let suffix_no_ext = match after.rfind('.') {
        Some(d) => after[..d].to_ascii_lowercase(),
        None => after.to_ascii_lowercase(),
    };

    let disk_dir = meshes_root.join(dir.replace('\\', "/"));
    let Ok(entries) = std::fs::read_dir(&disk_dir) else {
        return Vec::new();
    };
    // Real on-disk case for every directory component (filesystem truth, not the GNAM case).
    let real_dir = real_dir_case(meshes_root, dir);
    let mut out = Vec::new();
    for e in entries.flatten() {
        let p = e.path();
        if !p.is_file() {
            continue;
        }
        let ext_hkx = p
            .extension()
            .and_then(|x| x.to_str())
            .is_some_and(|x| x.eq_ignore_ascii_case("hkx"));
        if !ext_hkx {
            continue;
        }
        let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let lower = stem.to_ascii_lowercase();
        if lower.starts_with(&prefix_lc) && lower.ends_with(&suffix_no_ext) {
            // GNAM template prefix + the wildcard-matched portion from disk (e.g. "1").
            let end = stem.len().saturating_sub(suffix_no_ext.len());
            let file_part = if end >= prefix_lc.len() {
                format!("{prefix_template}{}", &stem[prefix_lc.len()..end])
            } else {
                stem.to_string()
            };
            out.push(format!("{real_dir}\\{file_part}"));
        }
    }
    out.sort();
    out
}

/// Walk each component of `rel` (a `\`-separated relative dir) under `base`, returning the
/// real on-disk casing of each. Falls back to the authored component if a dir is absent or
/// unreadable. One `read_dir` per component at emit time — not a hot path.
fn real_dir_case(base: &Path, rel: &str) -> String {
    let mut current = base.to_path_buf();
    let mut parts = Vec::new();
    for part in rel.split('\\').filter(|s| !s.is_empty()) {
        let lc = part.to_ascii_lowercase();
        let real = std::fs::read_dir(&current)
            .ok()
            .and_then(|entries| {
                entries.flatten().find_map(|e| {
                    let name = e.file_name();
                    let s = name.to_str()?;
                    if s.to_ascii_lowercase() == lc && e.path().is_dir() {
                        Some(s.to_string())
                    } else {
                        None
                    }
                })
            })
            .unwrap_or_else(|| part.to_string());
        current = current.join(&real);
        parts.push(real);
    }
    parts.join("\\")
}

// ---------------------------------------------------------------------------
// Main project manifest extraction (best-effort, with on-disk fallbacks)
// ---------------------------------------------------------------------------

/// Read the first string member named `field` from any object of `class`.
fn hkx_string_field(
    objects: &[havok_native::hkx::HkxObject],
    class: &str,
    field: &str,
) -> Option<String> {
    for obj in objects {
        if obj.class_name != class {
            continue;
        }
        for m in &obj.members {
            if m.name == field {
                if let Some(s) = as_str(&m.value) {
                    if !s.is_empty() {
                        return Some(s.to_string());
                    }
                }
            }
        }
    }
    None
}

/// Read the first non-empty string array member named `field` from any object of `class`.
fn hkx_string_array(
    objects: &[havok_native::hkx::HkxObject],
    class: &str,
    field: &str,
) -> Vec<String> {
    for obj in objects {
        if obj.class_name != class {
            continue;
        }
        for m in &obj.members {
            if m.name == field {
                if let HkxValue::Array(items) = &m.value {
                    let v: Vec<String> = items
                        .iter()
                        .filter_map(|x| as_str(x).filter(|s| !s.is_empty()).map(str::to_string))
                        .collect();
                    if !v.is_empty() {
                        return v;
                    }
                }
            }
        }
    }
    Vec::new()
}

fn read_objects(file: &Path) -> Vec<havok_native::hkx::HkxObject> {
    super::hkx_cache::behavior_packfile(file)
        .map(|h| h.objects().to_vec())
        .unwrap_or_default()
}

/// Find the single file in `dir` (on disk) whose name matches `pred`, returning its
/// real on-disk filename (case as stored).
fn find_file<P: Fn(&str) -> bool>(dir: &Path, pred: P) -> Option<String> {
    std::fs::read_dir(dir).ok()?.flatten().find_map(|e| {
        let p = e.path();
        if !p.is_file() {
            return None;
        }
        let name = p.file_name()?.to_str()?;
        pred(&name.to_ascii_lowercase()).then(|| name.to_string())
    })
}

fn force_hkx_ext(rel: &str) -> String {
    let norm = rel.replace('/', "\\");
    match norm.rfind('.') {
        Some(d)
            if norm[d..].eq_ignore_ascii_case(".hkt") || norm[d..].eq_ignore_ascii_case(".hkx") =>
        {
            format!("{}.hkx", &norm[..d])
        }
        _ => norm,
    }
}

/// Build the main project manifest `(project_name, project-relative file list)` for a
/// creature from its race dir (`Actors\<Race>`). Reads the project/character/root-behavior
/// `.hkx` strings, falling back to on-disk discovery where a read is empty. `None` when no
/// root behavior can be found.
///
/// Takes the race dir, not a core-behavior path: humanoid creatures mount the shared
/// `Actors\Character\Behaviors\*` cores, whose path names `Character`, not the race.
pub fn extract_project_manifest(
    race_dir: &str,
    meshes_root: &Path,
) -> Option<(String, Vec<String>)> {
    let race_name = race_name_of_dir(race_dir)?; // <Race>
    let race_disk = meshes_root.join(race_dir.replace('\\', "/"));

    // --- project .hkx: name + characterFilenames ---
    let project_file = find_creature_project_file(&race_disk, &race_name);
    let project_objs = project_file
        .as_ref()
        .map(|f| read_objects(&race_disk.join(f)))
        .unwrap_or_default();
    let project_name = manifest_project_name(
        hkx_string_field(&project_objs, "hkbProjectStringData", "name"),
        project_file.as_deref(),
        &race_name,
    );
    let character_filenames =
        hkx_string_array(&project_objs, "hkbProjectStringData", "characterFilenames");

    // --- character .hkx: behaviorFilename (root) + rigName (skeleton) ---
    let char_dir = race_disk.join("Characters");
    let char_file = find_file(&char_dir, |n| n.ends_with(".hkx"));
    let char_objs = char_file
        .as_ref()
        .map(|f| read_objects(&char_dir.join(f)))
        .unwrap_or_default();

    let behavior_filename =
        hkx_string_field(&char_objs, "hkbCharacterStringData", "behaviorFilename").or_else(
            || {
                find_file(&race_disk.join("Behaviors"), |n| {
                    n.ends_with("rootbehavior.hkx")
                })
                .map(|f| format!("Behaviors\\{f}"))
            },
        )?;
    let rig_name =
        hkx_string_field(&char_objs, "hkbCharacterStringData", "rigName").or_else(|| {
            find_file(&race_disk.join("CharacterAssets"), |n| {
                n.ends_with("skeleton.hkx")
            })
            .map(|f| format!("CharacterAssets\\{f}"))
        });
    // The character entry's basename = the character hkx's own `name` (authored mixed
    // case, e.g. `SnallygasterCharacter`), NOT the project's lowercased characterFilenames.
    let char_name = hkx_string_field(&char_objs, "hkbCharacterStringData", "name");

    // --- ROOT behavior clip anims (Animations\<stem>.hkx) ---
    let root_rel = force_hkx_ext(&behavior_filename); // Behaviors\<Root>.hkx
    let root_file = race_disk.join(root_rel.replace('\\', "/"));
    let root_anims = root_behavior_clip_anims(&root_file);

    // --- assemble in the documented order ---
    let mut files: Vec<String> = Vec::new();
    files.push(force_hkx_ext(&behavior_filename)); // Behaviors\…RootBehavior.hkx
    // character: basename from the character hkx `name` (mixed case); dir from
    // characterFilenames (the project stores the correctly-cased `Characters` dir).
    if let Some(name) = char_name.as_deref().filter(|s| !s.is_empty()) {
        let dir = character_filenames
            .first()
            .and_then(|c| {
                c.replace('/', "\\")
                    .rsplit_once('\\')
                    .map(|(d, _)| d.to_string())
            })
            .unwrap_or_else(|| "Characters".to_string());
        files.push(format!("{dir}\\{name}.hkx")); // Characters\SnallygasterCharacter.hkx
    } else if let Some(c) = character_filenames.first() {
        files.push(force_hkx_ext(c)); // Characters\…Character.hkx
    } else if let Some(cf) = char_file {
        files.push(format!("Characters\\{}", force_hkx_ext(&cf)));
    }
    // rig: CK resolves the skeleton asset to its on-disk file (using the disk case), not
    // the rigName field's FO76-authored case. → CharacterAssets\skeleton.hkx.
    if let Some(r) = rig_name {
        files.push(resolve_disk_case(&race_disk, &force_hkx_ext(&r)));
    }
    files.extend(root_anims); // Animations\*.hkx

    Some((project_name, files))
}

/// Replace a relpath's basename with the actual on-disk filename case (CK resolves an
/// asset reference to the file it finds on disk). The directory part is preserved as
/// written; the input is returned unchanged if no case-insensitive match exists.
fn resolve_disk_case(base_dir: &Path, rel: &str) -> String {
    let norm = rel.replace('/', "\\");
    let (dir, file) = match norm.rsplit_once('\\') {
        Some((d, f)) => (d, f),
        None => ("", norm.as_str()),
    };
    if let Ok(entries) = std::fs::read_dir(base_dir.join(dir.replace('\\', "/"))) {
        for e in entries.flatten() {
            if let Some(n) = e.file_name().to_str() {
                if n.eq_ignore_ascii_case(file) {
                    return if dir.is_empty() {
                        n.to_string()
                    } else {
                        format!("{dir}\\{n}")
                    };
                }
            }
        }
    }
    norm
}

/// The creature's project `.hkx` path relative to `meshes_root`, found on disk: usually
/// `Actors\<Race>\<race>project.hkx`, but RadHog uses `Actors\RadHog\RadHog.hkx`. It keys
/// the project-level empty `AnimationOffsets` entry: filename id `name_id(path)` (the
/// Snallygaster `AnimationOffsets/1776463414.txt`), body referencing the ROOT behavior.
pub fn project_hkx_relpath(core_behavior: &str, meshes_root: &Path) -> Option<String> {
    let race_dir = race_dir_of(core_behavior)?;
    project_hkx_relpath_for_race_dir(&race_dir, meshes_root)
}

/// Same, from the race DIR rather than a core-behavior path. A humanoid creature's cores all
/// live in the shared `Actors\Character\Behaviors` tree, so `race_dir_of(core)` names the wrong
/// race (or none) and only the race dir — from the RACE's `ANAM` skeletal model — identifies the
/// project the mod actually ships.
pub fn project_hkx_relpath_for_race_dir(race_dir: &str, meshes_root: &Path) -> Option<String> {
    let race_name = race_name_of_dir(race_dir)?;
    let race_disk = meshes_root.join(race_dir.replace('\\', "/"));
    let project_file = find_creature_project_file(&race_disk, &race_name)?;
    Some(format!("{race_dir}\\{project_file}"))
}

/// Manifest identity for a creature project.
///
/// FO4 looks a project manifest up by the stem of the project `.hkx` the RACE points at
/// (vanilla `baby.hkx` -> `baby.txt`); `hkbProjectStringData` carries no `name`. RadHog
/// ships `RadHog.hkx`, so a `radhogproject.txt` manifest leaves it with no clips. The
/// `<Race>Project` spelling is kept when it already names the shipped file, so conforming
/// creatures round-trip byte-identically.
fn manifest_project_name(
    declared: Option<String>,
    project_file: Option<&str>,
    race_name: &str,
) -> String {
    let fallback = format!("{race_name}Project");
    declared
        .or_else(|| {
            let stem = Path::new(project_file?).file_stem()?.to_str()?.to_string();
            (!stem.eq_ignore_ascii_case(&fallback)).then_some(stem)
        })
        .unwrap_or(fallback)
}

fn find_creature_project_file(race_disk: &Path, race_name: &str) -> Option<String> {
    find_file(race_disk, |name| name.ends_with("project.hkx")).or_else(|| {
        let actor_named_project = format!("{}.hkx", race_name.to_ascii_lowercase());
        find_file(race_disk, |name| name == actor_named_project)
    })
}

/// Root-behavior clip animations as project-relative paths, in behavior object order.
///
/// `animationName` is kept verbatim apart from the `.hkt` -> `.hkx` swap: it may name a
/// subdirectory (`Animations\Shared\DeathChest01`) or escape the project
/// (`..\Character\Animations\Death1`), and CK writes it verbatim (vanilla ships e.g.
/// `..\Bloatfly\Animations\AmbushCeiling\Ambush.hkx`). Flattening it to
/// `Animations\<stem>.hkx` points the manifest at missing files for any creature that
/// uses subfolders.
pub fn root_behavior_clip_anims(root_behavior_file: &Path) -> Vec<String> {
    let objs = read_objects(root_behavior_file);
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for obj in &objs {
        if obj.class_name != "hkbClipGenerator" {
            continue;
        }
        for m in &obj.members {
            if m.name == "animationName" {
                if let Some(s) = as_str(&m.value) {
                    if !s.is_empty() {
                        let rel = force_hkx_ext(s);
                        if seen.insert(rel.to_ascii_lowercase()) {
                            out.push(rel);
                        }
                    }
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// FX project manifest (Meshes\UniqueBehaviors\<name>fx\) — fully derivable
// ---------------------------------------------------------------------------

/// Normalize an FX `rigName` for the manifest's rig line (RE: `holdouts_deep.md`):
/// relativize a `Meshes\…`-rooted skeleton to the FX project dir
/// (`UniqueBehaviors\<name>fx` is 2 levels under Meshes → `..\..\`) and swap the
/// `.hkt` extension to `.hkx`. A non-`Meshes\`-rooted rig is left as-is bar the ext.
fn normalize_fx_rig(rig: &str) -> String {
    let norm = rig.replace('/', "\\");
    let rel = norm
        .strip_prefix("Meshes\\")
        .or_else(|| norm.strip_prefix("meshes\\"))
        .map(|r| format!("..\\..\\{r}"))
        .unwrap_or(norm);
    force_hkx_ext(&rel)
}

/// Every FX project directory name under `Meshes\UniqueBehaviors\` that carries a
/// root `<name>fx.hkx` (the registration gate). Names are returned in the on-disk
/// case (FO76→FO4 carries this tree verbatim; BA2 extraction lowercases it).
pub fn fx_project_dirs(meshes_root: &Path) -> Vec<String> {
    let ub = meshes_root.join("UniqueBehaviors");
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&ub) {
        for e in entries.flatten() {
            let p = e.path();
            if !p.is_dir() {
                continue;
            }
            let Some(name) = p.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            // Gate: a root `<name>fx.hkx` must exist (createabotfx ships no
            // behavior.hkx but still has its root project file).
            if p.join(format!("{name}.hkx")).is_file() && name.to_ascii_lowercase().ends_with("fx")
            {
                out.push(name.to_string());
            }
        }
    }
    out.sort();
    out
}

/// Build the FX project manifest `(project_name, project-relative file list)` for a
/// single `UniqueBehaviors\<fx_dir_name>\` project. Reads the FX project + character
/// `.hkx` string data:
/// * `behaviorFilename` (character): `Behaviors\Behavior.hkx`
/// * `characterFilenames[*]` (project): `Characters\Character.hkx`
/// * `normalize(rigName)` (character): `..\..\GenericBehaviors\…\SingleBoneSkeleton.hkx`
///
/// FX projects carry no root animation names, so the file count is 3. `ProjectName`
/// casing is not stored in any `.hkx`; it is the FX dir name verbatim, which on a
/// BA2-lowercased source differs from CK's output only in case (lookup is
/// case-insensitive). `None` if the project hkx is absent or has no behaviorFilename.
pub fn extract_fx_manifest(meshes_root: &Path, fx_dir_name: &str) -> Option<(String, Vec<String>)> {
    let fx_dir = meshes_root.join("UniqueBehaviors").join(fx_dir_name);
    let project_file = fx_dir.join(format!("{fx_dir_name}.hkx"));
    if !project_file.is_file() {
        return None; // registration gate
    }
    let project_objs = read_objects(&project_file);

    // The character file is conventionally characters\character.hkx; fall back to any.
    let char_dir = fx_dir.join("characters");
    let char_file = find_file(&char_dir, |n| n.ends_with(".hkx"));
    let char_objs = char_file
        .as_ref()
        .map(|f| read_objects(&char_dir.join(f)))
        .unwrap_or_default();

    let behavior = hkx_string_field(&char_objs, "hkbCharacterStringData", "behaviorFilename")?;
    let character_filenames =
        hkx_string_array(&project_objs, "hkbProjectStringData", "characterFilenames");
    let rig = hkx_string_field(&char_objs, "hkbCharacterStringData", "rigName");

    let mut files = Vec::new();
    files.push(force_hkx_ext(&behavior));
    if character_filenames.is_empty() {
        if let Some(cf) = char_file {
            files.push(format!("Characters\\{}", force_hkx_ext(&cf)));
        }
    } else {
        for c in &character_filenames {
            files.push(force_hkx_ext(c));
        }
    }
    if let Some(r) = rig {
        files.push(normalize_fx_rig(&r));
    }
    // FX projects have no root animation names.

    Some((fx_dir_name.to_string(), files))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_trigger_timing_preserves_start_and_end_origins() {
        use havok_native::hkx::{HkxMember, HkxObject};

        let member = |name: &str, value| HkxMember { name: name.into(), value };
        let triggers = [(2.5, false), (-0.5, true), (0.0, true)]
            .into_iter()
            .map(|(time, relative)| HkxValue::Object(vec![
                member("localTime", HkxValue::F32(time)),
                member("relativeToEndOfClip", HkxValue::Bool(relative)),
                member("event", HkxValue::Object(vec![member("id", HkxValue::I32(0))])),
            ]))
            .collect();
        let objects = [HkxObject {
            name: None,
            offset: 0,
            signature: 0,
            class_name: "hkbClipTriggerArray".into(),
            members: vec![member("triggers", HkxValue::Array(triggers))],
        }];
        let extracted = extract_triggers(&objects, 0, &["HitFrame".into()]);
        let engine_times: Vec<_> = extracted.iter().map(|trigger| {
            if trigger.flag != 0 { 9.0 - trigger.time } else { trigger.time }
        }).collect();
        assert_eq!(engine_times, [2.5, 8.5, 9.0]);
    }

    #[test]
    fn race_name_and_dir_from_core_behavior() {
        let core = r"Actors\Snallygaster\Behaviors\SnallygasterCoreBehavior.hkx";
        assert_eq!(race_dir_of(core).as_deref(), Some(r"Actors\Snallygaster"));
        assert_eq!(race_name_of(core).as_deref(), Some("Snallygaster"));
    }

    #[test]
    fn manifest_name_follows_the_shipped_project_file_stem() {
        // RadHog ships `RadHog.hkx`, not `RadHogProject.hkx`. FO4 looks the
        // manifest up by that stem, so emitting `radhogproject.txt` left the
        // creature with no animation data at all.
        assert_eq!(
            manifest_project_name(None, Some("radhog.hkx"), "radhog"),
            "radhog"
        );
    }

    #[test]
    fn conforming_creatures_keep_the_race_project_spelling() {
        // Byte-identical output for every creature that already agreed.
        assert_eq!(
            manifest_project_name(None, Some("snallygasterproject.hkx"), "Snallygaster"),
            "SnallygasterProject"
        );
    }

    #[test]
    fn manifest_name_falls_back_when_no_project_ships() {
        assert_eq!(
            manifest_project_name(None, None, "Snallygaster"),
            "SnallygasterProject"
        );
    }

    #[test]
    fn declared_project_name_wins() {
        assert_eq!(
            manifest_project_name(Some("Authored".into()), Some("radhog.hkx"), "radhog"),
            "Authored"
        );
    }

    #[test]
    fn anim_basename_strips_dir_and_ext() {
        assert_eq!(anim_basename_no_ext(r"Animations\Idle.hkt"), "Idle");
        assert_eq!(anim_basename_no_ext("Attack1"), "Attack1");
    }

    #[test]
    fn force_hkx_ext_keeps_subdirectories_and_parent_escapes() {
        // The manifest must reproduce the clip path, not just its basename:
        // flattening these to `Animations\<stem>.hkx` is what killed the
        // mole miner / scorched project manifests.
        assert_eq!(
            force_hkx_ext(r"Animations\Shared\DeathChest01.hkt"),
            r"Animations\Shared\DeathChest01.hkx"
        );
        assert_eq!(
            force_hkx_ext(r"Animations\AutoGrenadeLauncher\PoseA_Idle1.hkt"),
            r"Animations\AutoGrenadeLauncher\PoseA_Idle1.hkx"
        );
        assert_eq!(
            force_hkx_ext(r"..\Character\Animations\Death1.hkt"),
            r"..\Character\Animations\Death1.hkx"
        );
        // already-correct paths and flat paths are untouched
        assert_eq!(
            force_hkx_ext(r"Animations\Idle.hkx"),
            r"Animations\Idle.hkx"
        );
    }

    #[test]
    fn normalize_fx_rig_relativizes_and_swaps_ext() {
        assert_eq!(
            normalize_fx_rig(r"Meshes\GenericBehaviors\zSingleBoneSkeleton\SingleBoneSkeleton.hkt"),
            r"..\..\GenericBehaviors\zSingleBoneSkeleton\SingleBoneSkeleton.hkx"
        );
    }

    #[test]
    fn finds_actor_named_creature_project() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("RadHog.hkx"), b"project").unwrap();

        assert_eq!(
            find_creature_project_file(temp.path(), "RadHog").as_deref(),
            Some("RadHog.hkx")
        );
    }

    #[test]
    fn resolves_actor_named_project_relpath() {
        let temp = tempfile::tempdir().unwrap();
        let race_dir = temp.path().join("Actors").join("RadHog");
        std::fs::create_dir_all(&race_dir).unwrap();
        std::fs::write(race_dir.join("RadHog.hkx"), b"project").unwrap();

        assert_eq!(
            project_hkx_relpath(
                r"Actors\RadHog\Behaviors\RadHogCoreBehavior.hkx",
                temp.path()
            )
            .as_deref(),
            Some(r"Actors\RadHog\RadHog.hkx")
        );
    }

    #[test]
    fn conventional_project_name_wins_over_actor_named_fallback() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("RadHog.hkx"), b"fallback").unwrap();
        std::fs::write(temp.path().join("RadHogProject.hkx"), b"project").unwrap();

        assert_eq!(
            find_creature_project_file(temp.path(), "RadHog").as_deref(),
            Some("RadHogProject.hkx")
        );
    }
}
