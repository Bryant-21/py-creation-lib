//! Recursive behavior-graph resolution for weapon / character AnimTextData (CK-free).
//!
//! Creatures are self-contained (one core behavior in the mod, clips relative to one
//! Animations dir) and use [`super::behavior_index`]. Weapons inject into the base-game
//! character behavior graph; a weapon subgraph's body (byte-set-exact vs CK) is:
//!
//! > **body** = core behavior + other transitive behavior references +
//! > `{ sapt_resolve(leaf) : leaf ∈ recursive-graph clip leaves }`
//!
//! * The recursive graph starts at the core behavior and follows every
//!   `hkbBehaviorReferenceGenerator.behaviorName` (relative to the behavior's
//!   `Actors\<Race>` dir), collecting every `hkbClipGenerator.animationName`.
//! * `sapt_resolve(leaf)` walks the subgraph's `SAPT` chain in order, looking for
//!   `<leaf>.hkx` under the mod root, then the base-game root; first hit wins.
//!   Unresolvable leaves are dropped, as in CK.
//!
//! Behaviors come first, then sorted anims; content-list order has no runtime effect.
//! See `docs/re/animtextdata_generation.md`.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use havok_native::hkx::HkxObject;
use havok_native::hkx::read_packfile;
use havok_native::hkx::types::HkxValue;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StancePerspective {
    FirstPerson,
    ThirdPerson,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PoseRoleSourceKey {
    pub pose_idx: u8,
    pub variant: u8,
    pub behavior: String,
    pub branch_ordinal: u8,
    pub clip_generator: String,
    pub animation_leaf: String,
    pub sapt_index: usize,
    pub sapt_branch: String,
}

/// Provenance for one `(pose, variant)` stance role. `effective_*` identifies the
/// clip that will contribute to the generated first section. `source_*` identifies
/// where the same graph leaf resolves in the base root and is the only permitted
/// donor join key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoseRoleProvenance {
    pub pose_idx: u8,
    pub variant: u8,
    pub behavior: String,
    pub branch_ordinal: u8,
    pub branch_name: String,
    pub clip_generator: String,
    pub animation_leaf: String,
    pub effective_clip: Option<PathBuf>,
    pub effective_sapt_index: Option<usize>,
    pub effective_sapt_branch: Option<String>,
    pub source_clip: Option<PathBuf>,
    pub source_sapt_index: Option<usize>,
    pub source_sapt_branch: Option<String>,
}

impl PoseRoleProvenance {
    pub fn source_key(&self) -> Option<PoseRoleSourceKey> {
        Some(PoseRoleSourceKey {
            pose_idx: self.pose_idx,
            variant: self.variant,
            behavior: norm_key(&self.behavior),
            branch_ordinal: self.branch_ordinal,
            clip_generator: self.clip_generator.to_ascii_lowercase(),
            animation_leaf: self.animation_leaf.to_ascii_lowercase(),
            sapt_index: self.source_sapt_index?,
            sapt_branch: norm_key(self.source_sapt_branch.as_deref()?),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PoseRoleResolveError {
    #[error("stance behavior {0:?} was not found on the supplied roots")]
    BehaviorMissing(String),
    #[error("failed to read stance behavior {path:?}: {message}")]
    BehaviorRead { path: PathBuf, message: String },
    #[error("failed to parse stance behavior {path:?}: {message}")]
    BehaviorParse { path: PathBuf, message: String },
    #[error("no structured stance-role graph was found from {0:?}")]
    NoRoles(String),
    #[error("behavior {behavior:?} exposes {found} capture-pose branches, expected 6")]
    CaptureRoleCount { behavior: String, found: usize },
    #[error("capture branch {branch:?} in {behavior:?} resolves {clips} clip generators")]
    AmbiguousBranch {
        behavior: String,
        branch: String,
        clips: usize,
    },
    #[error("behavior {behavior:?} exposes {found} exact WPNIdleReady clips, expected 1")]
    FirstPersonRoleCount { behavior: String, found: usize },
}

#[derive(Debug, Clone)]
struct PoseRoleDescriptor {
    pose_idx: u8,
    behavior: String,
    branch_ordinal: u8,
    branch_name: String,
    clip_generator: String,
    animation_leaf: String,
}

#[derive(Debug, Clone)]
struct ResolvedRoleClip {
    disk: PathBuf,
    sapt_index: usize,
    sapt_branch: String,
}

/// A single behavior's parsed contribution to the graph.
struct BehaviorParse {
    /// Clip leaves: `hkbClipGenerator.animationName` basenames, lowercased, no ext.
    leaves: Vec<String>,
    /// Behavior references: `hkbBehaviorReferenceGenerator.behaviorName` resolved to
    /// FO4-style relpaths (e.g. `Actors\Character\Behaviors\WeaponBehavior.hkx`).
    refs: Vec<String>,
    /// `(clip_name, animationName_leaf, animationName_leaf_cased)` for each named
    /// `hkbClipGenerator`. Unlike `leaves`, keeps the generator's own `name` for the
    /// section-1 clip→path map and the original-cased anim basename that furniture keys on
    /// (see `resolve_clip_generators`).
    clips: Vec<(String, String, String)>,
}

/// Resolves a weapon/character subgraph body across mod + base-game meshes roots.
/// Caches behavior parses and directory listings so the shared character graph is
/// parsed once even across dozens of subgraphs.
pub struct GraphResolver {
    /// Search order: primary (mod) first, then base-game (extracted/fo4).
    roots: Vec<PathBuf>,
    behavior_cache: HashMap<String, Option<BehaviorParse>>,
    stance_role_cache: HashMap<String, Result<Vec<PoseRoleDescriptor>, PoseRoleResolveError>>,
    /// `relpath_lower -> { stem_lower: actual_filename }`, merged across roots
    /// (primary root wins).
    dir_cache: HashMap<String, HashMap<String, String>>,
}

impl GraphResolver {
    pub fn new(roots: Vec<PathBuf>) -> Self {
        Self {
            roots,
            behavior_cache: HashMap::new(),
            stance_role_cache: HashMap::new(),
            dir_cache: HashMap::new(),
        }
    }

    /// Resolve the full body (behavior files + resolved anim files) for a subgraph.
    pub fn resolve_body(&mut self, core_rel: &str, sapt_chain: &[String]) -> Vec<String> {
        let core_key = norm_key(core_rel);
        let mut visited: HashSet<String> = HashSet::new();
        let mut behaviors: Vec<String> = Vec::new();
        let mut leaves: HashSet<String> = HashSet::new();
        let mut queue: VecDeque<String> = VecDeque::new();
        queue.push_back(core_rel.to_string());

        while let Some(rel) = queue.pop_front() {
            let key = norm_key(&rel);
            if !visited.insert(key.clone()) {
                continue;
            }
            let parsed = match self.parse_behavior(&rel) {
                Some(p) => (p.leaves.clone(), p.refs.clone()),
                None => continue,
            };
            for l in parsed.0 {
                leaves.insert(l);
            }
            for r in parsed.1 {
                let rk = norm_key(&r);
                if rk != core_key && !behaviors.iter().any(|b| norm_key(b) == rk) {
                    behaviors.push(r.clone());
                }
                if !visited.contains(&rk) {
                    queue.push_back(r);
                }
            }
        }

        let mut anims: BTreeSet<String> = BTreeSet::new();
        for leaf in &leaves {
            if let Some(p) = self.sapt_resolve(leaf, sapt_chain) {
                anims.insert(p);
            }
        }

        let mut body = behaviors;
        body.extend(anims);
        // CK lists the stance's own core graph as the FIRST behavior row (vanilla
        // SuperMutant melee FileData starts with `MeleeBehavior.hkx`); the engine
        // loads the stance from this manifest, so omitting the core leaves the
        // subgraph without its own graph. Keep the empty-body skip semantics: a
        // core that resolves nothing still yields an empty body.
        if !body.is_empty() && self.parse_behavior(core_rel).is_some() {
            body.insert(0, core_rel.to_string());
        }
        body
    }

    /// Cross-file clip generators reachable from `core_rel` via `behaviorName` refs, each
    /// SAPT-resolved across the mod+base roots:
    /// `(clip_name, anim_basename_cased, anim_rel_no_ext, disk_path)`.
    ///
    /// The weapon AnimationOffsets section-1 clip universe: the closure `resolve_body` walks,
    /// keyed by generator. Two keys come back because CK keys section 1 per builder:
    /// weapon/creature use the generator's `name`, furniture the animation basename. FO4's
    /// shared furniture graph names a generator `Standing Enter` whose animation is
    /// `EnterFromStand`, and CK writes the latter (`Standing Enter` appears in 0 of 3156
    /// vanilla `AnimationOffsets` files). A generator-name key makes every clip lookup miss,
    /// which deletes the `InteractionData` entry and makes the furniture unusable.
    /// Deduped by clip name (first wins); clips not on disk along the SAPT chain are dropped.
    pub fn resolve_clip_generators(
        &mut self,
        core_rel: &str,
        sapt_chain: &[String],
    ) -> Vec<(String, String, String, PathBuf)> {
        let mut visited: HashSet<String> = HashSet::new();
        let mut queue: VecDeque<String> = VecDeque::new();
        let mut clips: Vec<(String, String, String)> = Vec::new();
        queue.push_back(core_rel.to_string());
        while let Some(rel) = queue.pop_front() {
            let key = norm_key(&rel);
            if !visited.insert(key) {
                continue;
            }
            let (cls, refs) = match self.parse_behavior(&rel) {
                Some(p) => (p.clips.clone(), p.refs.clone()),
                None => continue,
            };
            clips.extend(cls);
            for r in refs {
                if !visited.contains(&norm_key(&r)) {
                    queue.push_back(r);
                }
            }
        }

        // Dedup by RESOLVED path (not by clip name): a clip name can recur across behaviors,
        // and an earlier occurrence whose `animationName` doesn't resolve must not suppress a
        // later one that does.
        let mut seen_path: HashSet<String> = HashSet::new();
        let mut out: Vec<(String, String, String, PathBuf)> = Vec::new();
        for (name, leaf, leaf_cased) in clips {
            if let Some((rel, disk)) = self.sapt_resolve_on_disk(&leaf, sapt_chain) {
                let no_ext = match rel.to_ascii_lowercase().ends_with(".hkx") {
                    true => rel[..rel.len() - 4].to_string(),
                    false => rel.clone(),
                };
                if seen_path.insert(no_ext.to_ascii_lowercase()) {
                    out.push((name, leaf_cased, no_ext, disk));
                }
            }
        }
        out
    }

    /// Resolve the stance-producing graph branches for a weapon subgraph. This is
    /// deliberately independent of `resolve_body`: a flat clip closure loses the
    /// capture modifier and branch that assign a clip to a pose role.
    pub fn resolve_stance_pose_roles(
        &mut self,
        core_rel: &str,
        sapt_chain: &[String],
        perspective: StancePerspective,
    ) -> Result<Vec<PoseRoleProvenance>, PoseRoleResolveError> {
        let cache_key = format!("{}|{perspective:?}", norm_key(core_rel));
        let descriptors = if let Some(cached) = self.stance_role_cache.get(&cache_key) {
            cached.clone()?
        } else {
            let resolved = self.stance_role_descriptors(core_rel, perspective);
            self.stance_role_cache.insert(cache_key, resolved.clone());
            resolved?
        };
        let mut roles = Vec::with_capacity(descriptors.len() * 2);
        for descriptor in descriptors {
            let effective = self.resolve_role_clip(&descriptor.animation_leaf, sapt_chain, false);
            let source = self.resolve_role_clip(&descriptor.animation_leaf, sapt_chain, true);
            for variant in 0..=1 {
                roles.push(PoseRoleProvenance {
                    pose_idx: descriptor.pose_idx,
                    variant,
                    behavior: descriptor.behavior.clone(),
                    branch_ordinal: descriptor.branch_ordinal,
                    branch_name: descriptor.branch_name.clone(),
                    clip_generator: descriptor.clip_generator.clone(),
                    animation_leaf: descriptor.animation_leaf.clone(),
                    effective_clip: effective.as_ref().map(|value| value.disk.clone()),
                    effective_sapt_index: effective.as_ref().map(|value| value.sapt_index),
                    effective_sapt_branch: effective
                        .as_ref()
                        .map(|value| value.sapt_branch.clone()),
                    source_clip: source.as_ref().map(|value| value.disk.clone()),
                    source_sapt_index: source.as_ref().map(|value| value.sapt_index),
                    source_sapt_branch: source.as_ref().map(|value| value.sapt_branch.clone()),
                });
            }
        }
        roles.sort_by_key(|role| (role.pose_idx, role.variant));
        Ok(roles)
    }

    fn stance_role_descriptors(
        &self,
        core_rel: &str,
        perspective: StancePerspective,
    ) -> Result<Vec<PoseRoleDescriptor>, PoseRoleResolveError> {
        let mut queue = VecDeque::from([core_rel.to_string()]);
        let mut visited = HashSet::new();
        let mut saw_behavior = false;
        let mut partial_capture: Option<(String, usize)> = None;
        while let Some(behavior) = queue.pop_front() {
            if !visited.insert(norm_key(&behavior)) {
                continue;
            }
            let Some(path) = self.find_on_disk(&behavior) else {
                if behavior == core_rel {
                    return Err(PoseRoleResolveError::BehaviorMissing(behavior));
                }
                continue;
            };
            saw_behavior = true;
            let data =
                std::fs::read(&path).map_err(|error| PoseRoleResolveError::BehaviorRead {
                    path: path.clone(),
                    message: error.to_string(),
                })?;
            let hkx =
                read_packfile(&data).map_err(|error| PoseRoleResolveError::BehaviorParse {
                    path: path.clone(),
                    message: error.to_string(),
                })?;
            let objects = hkx.objects();
            let descriptors = match perspective {
                StancePerspective::ThirdPerson => capture_pose_descriptors(&behavior, objects)?,
                StancePerspective::FirstPerson => {
                    first_person_pose_descriptors(&behavior, objects)?
                }
            };
            if !descriptors.is_empty() {
                let expected = match perspective {
                    StancePerspective::FirstPerson => 2,
                    StancePerspective::ThirdPerson => 6,
                };
                if descriptors.len() == expected {
                    return Ok(descriptors);
                }
                partial_capture = Some((behavior.clone(), descriptors.len()));
            }
            for reference in behavior_references(&behavior, objects) {
                if !visited.contains(&norm_key(&reference)) {
                    queue.push_back(reference);
                }
            }
        }
        if let Some((behavior, found)) = partial_capture {
            return Err(PoseRoleResolveError::CaptureRoleCount { behavior, found });
        }
        if !saw_behavior {
            return Err(PoseRoleResolveError::BehaviorMissing(core_rel.to_string()));
        }
        Err(PoseRoleResolveError::NoRoles(core_rel.to_string()))
    }

    fn resolve_role_clip(
        &self,
        leaf: &str,
        sapt_chain: &[String],
        base_only: bool,
    ) -> Option<ResolvedRoleClip> {
        let roots: &[PathBuf] = if base_only {
            std::slice::from_ref(self.roots.last()?)
        } else {
            &self.roots
        };
        for (sapt_index, authored_branch) in sapt_chain.iter().enumerate() {
            let branch = sapt_dir(authored_branch);
            for root in roots {
                let dir = root.join(branch.replace('\\', "/"));
                let Ok(entries) = std::fs::read_dir(&dir) else {
                    continue;
                };
                let Some(disk) = entries.flatten().map(|entry| entry.path()).find(|path| {
                    path.is_file()
                        && ext_is_hkx(path)
                        && path
                            .file_stem()
                            .and_then(|value| value.to_str())
                            .is_some_and(|stem| stem.eq_ignore_ascii_case(leaf))
                }) else {
                    continue;
                };
                return Some(ResolvedRoleClip {
                    disk,
                    sapt_index,
                    sapt_branch: branch,
                });
            }
        }
        None
    }

    /// Like `sapt_resolve` but also returns the resolved file's disk path (mod root first,
    /// then base) — the weapon offsets builder needs the on-disk `.hkx` to read root motion.
    fn sapt_resolve_on_disk(
        &mut self,
        leaf: &str,
        sapt_chain: &[String],
    ) -> Option<(String, PathBuf)> {
        for sapt in sapt_chain {
            let dir = sapt_dir(sapt);
            let fname = self.dir_listing(&dir).get(leaf).cloned();
            if let Some(fname) = fname {
                let rel = format!("{dir}\\{fname}");
                for root in &self.roots {
                    let p = root.join(rel.replace('\\', "/"));
                    if p.is_file() {
                        return Some((rel, p));
                    }
                }
            }
        }
        None
    }

    fn parse_behavior(&mut self, rel: &str) -> &Option<BehaviorParse> {
        let key = norm_key(rel);
        if !self.behavior_cache.contains_key(&key) {
            let parsed = self.parse_behavior_uncached(rel);
            self.behavior_cache.insert(key.clone(), parsed);
        }
        self.behavior_cache.get(&key).unwrap()
    }

    fn parse_behavior_uncached(&self, rel: &str) -> Option<BehaviorParse> {
        let path = self.find_on_disk(rel)?;
        let hkx = super::hkx_cache::behavior_packfile(&path)?;
        let parent = drop_last_two(rel);
        let mut leaves = Vec::new();
        let mut refs = Vec::new();
        let mut clips = Vec::new();
        for obj in hkx.objects() {
            match obj.class_name.as_str() {
                "hkbClipGenerator" => {
                    let mut name = None;
                    let mut leaf = None;
                    let mut leaf_cased = None;
                    for m in &obj.members {
                        if let HkxValue::String { value, .. } = &m.value {
                            if value.is_empty() {
                                continue;
                            }
                            match m.name.as_str() {
                                "name" => name = Some(value.clone()),
                                "animationName" => {
                                    leaf = Some(leaf_basename(value));
                                    leaf_cased = Some(leaf_basename_cased(value));
                                }
                                _ => {}
                            }
                        }
                    }
                    if let (Some(leaf), Some(leaf_cased)) = (leaf, leaf_cased) {
                        leaves.push(leaf.clone());
                        if let Some(name) = name {
                            clips.push((name, leaf, leaf_cased));
                        }
                    }
                }
                "hkbBehaviorReferenceGenerator" => {
                    for m in &obj.members {
                        if m.name == "behaviorName" {
                            if let HkxValue::String { value, .. } = &m.value {
                                if !value.is_empty() {
                                    refs.push(join_rel(&parent, value));
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        Some(BehaviorParse {
            leaves,
            refs,
            clips,
        })
    }

    fn sapt_resolve(&mut self, leaf: &str, sapt_chain: &[String]) -> Option<String> {
        for sapt in sapt_chain {
            let dir = sapt_dir(sapt);
            let fname = self.dir_listing(&dir).get(leaf).cloned();
            if let Some(fname) = fname {
                return Some(format!("{dir}\\{fname}"));
            }
        }
        None
    }

    fn dir_listing(&mut self, rel: &str) -> &HashMap<String, String> {
        let key = norm_key(rel);
        if !self.dir_cache.contains_key(&key) {
            let mut map: HashMap<String, String> = HashMap::new();
            for root in &self.roots {
                let dir = root.join(rel.replace('\\', "/"));
                if let Ok(entries) = std::fs::read_dir(&dir) {
                    for e in entries.flatten() {
                        let p = e.path();
                        if !p.is_file() || !ext_is_hkx(&p) {
                            continue;
                        }
                        if let (Some(stem), Some(name)) = (
                            p.file_stem().and_then(|s| s.to_str()),
                            p.file_name().and_then(|s| s.to_str()),
                        ) {
                            map.entry(stem.to_ascii_lowercase())
                                .or_insert_with(|| name.to_string());
                        }
                    }
                }
            }
            self.dir_cache.insert(key.clone(), map);
        }
        self.dir_cache.get(&key).unwrap()
    }

    fn find_on_disk(&self, rel: &str) -> Option<PathBuf> {
        for root in &self.roots {
            let p = root.join(rel.replace('\\', "/"));
            if p.is_file() {
                return Some(p);
            }
        }
        None
    }
}

fn value_member<'a>(object: &'a HkxObject, name: &str) -> Option<&'a HkxValue> {
    object
        .members
        .iter()
        .find(|member| member.name == name)
        .map(|member| &member.value)
}

fn pointer_targets(value: &HkxValue) -> Vec<usize> {
    match value {
        HkxValue::Pointer(Some(index)) => vec![*index],
        HkxValue::Array(values) => values
            .iter()
            .filter_map(|value| match value {
                HkxValue::Pointer(Some(index)) => Some(*index),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn pointer_member(object: &HkxObject, name: &str) -> Vec<usize> {
    value_member(object, name)
        .map(pointer_targets)
        .unwrap_or_default()
}

fn first_pointer(object: &HkxObject, name: &str) -> Option<usize> {
    pointer_member(object, name).into_iter().next()
}

fn string_member<'a>(object: &'a HkxObject, name: &str) -> Option<&'a str> {
    value_member(object, name).and_then(|value| match value {
        HkxValue::String { value, .. } if !value.is_empty() => Some(value.as_str()),
        _ => None,
    })
}

fn integer_member(object: &HkxObject, name: &str) -> Option<i64> {
    value_member(object, name).and_then(|value| match value {
        HkxValue::I8(value) => Some(i64::from(*value)),
        HkxValue::U8(value) => Some(i64::from(*value)),
        HkxValue::I16(value) => Some(i64::from(*value)),
        HkxValue::U16(value) => Some(i64::from(*value)),
        HkxValue::I32(value) => Some(i64::from(*value)),
        HkxValue::U32(value) => Some(i64::from(*value)),
        HkxValue::I64(value) => Some(*value),
        HkxValue::U64(value) => i64::try_from(*value).ok(),
        HkxValue::Bool(value) => Some(i64::from(*value)),
        _ => None,
    })
}

fn behavior_references(behavior_rel: &str, objects: &[HkxObject]) -> Vec<String> {
    let parent = drop_last_two(behavior_rel);
    objects
        .iter()
        .filter(|object| object.class_name == "hkbBehaviorReferenceGenerator")
        .filter_map(|object| string_member(object, "behaviorName"))
        .map(|relative| join_rel(&parent, relative))
        .collect()
}

fn collect_generator_clips(
    index: usize,
    objects: &[HkxObject],
    seen: &mut HashSet<usize>,
    clips: &mut Vec<usize>,
) {
    if !seen.insert(index) {
        return;
    }
    let Some(object) = objects.get(index) else {
        return;
    };
    if object.class_name == "hkbClipGenerator" {
        clips.push(index);
        return;
    }
    const SINGLE_EDGES: &[&str] = &[
        "generator",
        "pDefaultGenerator",
        "pGenerator",
        "pBlenderGenerator",
        "child",
    ];
    const ARRAY_EDGES: &[&str] = &["generators", "children", "layers", "states"];
    for edge in SINGLE_EDGES {
        for child in pointer_member(object, edge) {
            collect_generator_clips(child, objects, seen, clips);
        }
    }
    for edge in ARRAY_EDGES {
        for child in pointer_member(object, edge) {
            let child = objects
                .get(child)
                .filter(|object| {
                    matches!(
                        object.class_name.as_str(),
                        "hkbBlenderGeneratorChild" | "hkbLayer" | "hkbStateMachineStateInfo"
                    )
                })
                .and_then(|object| first_pointer(object, "generator"))
                .unwrap_or(child);
            collect_generator_clips(child, objects, seen, clips);
        }
    }
}

fn capture_pose_descriptors(
    behavior: &str,
    objects: &[HkxObject],
) -> Result<Vec<PoseRoleDescriptor>, PoseRoleResolveError> {
    #[derive(Debug)]
    struct Candidate {
        order: (i64, usize),
        on_activate: bool,
        branch_name: String,
        clip_index: usize,
    }

    let mut candidates = Vec::new();
    for (modifier_index, modifier) in objects.iter().enumerate() {
        if modifier.class_name != "BSDirectAtCapturePoseModifier" {
            continue;
        }
        let on_activate = integer_member(modifier, "capturePoseOnActivate") == Some(1);
        let event_id = integer_member(modifier, "capturePoseEventId").unwrap_or(-1);
        for (generator_index, generator) in objects.iter().enumerate() {
            if generator.class_name != "hkbModifierGenerator"
                || first_pointer(generator, "modifier") != Some(modifier_index)
            {
                continue;
            }
            let branch_name = string_member(generator, "name")
                .unwrap_or("unnamed capture branch")
                .to_string();
            let Some(root) = first_pointer(generator, "generator") else {
                continue;
            };
            let mut clips = Vec::new();
            collect_generator_clips(root, objects, &mut HashSet::new(), &mut clips);
            clips.sort_unstable();
            clips.dedup();
            if clips.len() != 1 {
                return Err(PoseRoleResolveError::AmbiguousBranch {
                    behavior: behavior.to_string(),
                    branch: branch_name,
                    clips: clips.len(),
                });
            }
            candidates.push(Candidate {
                order: (event_id, generator_index),
                on_activate,
                branch_name,
                clip_index: clips[0],
            });
        }
    }
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    candidates.sort_by_key(|candidate| {
        (
            u8::from(candidate.on_activate),
            candidate.order.0,
            candidate.order.1,
        )
    });
    candidates
        .into_iter()
        .enumerate()
        .map(|(pose_idx, candidate)| {
            let clip = &objects[candidate.clip_index];
            let animation = string_member(clip, "animationName").ok_or_else(|| {
                PoseRoleResolveError::AmbiguousBranch {
                    behavior: behavior.to_string(),
                    branch: candidate.branch_name.clone(),
                    clips: 0,
                }
            })?;
            Ok(PoseRoleDescriptor {
                pose_idx: pose_idx as u8,
                behavior: behavior.to_string(),
                branch_ordinal: pose_idx as u8,
                branch_name: candidate.branch_name,
                clip_generator: string_member(clip, "name").unwrap_or("").to_string(),
                animation_leaf: leaf_basename(animation),
            })
        })
        .collect()
}

fn first_person_pose_descriptors(
    behavior: &str,
    objects: &[HkxObject],
) -> Result<Vec<PoseRoleDescriptor>, PoseRoleResolveError> {
    let clips: Vec<&HkxObject> = objects
        .iter()
        .filter(|object| {
            object.class_name == "hkbClipGenerator"
                && string_member(object, "name")
                    .is_some_and(|name| name.eq_ignore_ascii_case("WPNIdleReady"))
        })
        .collect();
    if clips.is_empty() {
        return Ok(Vec::new());
    }
    if clips.len() != 1 {
        return Err(PoseRoleResolveError::FirstPersonRoleCount {
            behavior: behavior.to_string(),
            found: clips.len(),
        });
    }
    let clip = clips[0];
    let animation = string_member(clip, "animationName").ok_or_else(|| {
        PoseRoleResolveError::FirstPersonRoleCount {
            behavior: behavior.to_string(),
            found: 0,
        }
    })?;
    Ok((0..=1)
        .map(|pose_idx| PoseRoleDescriptor {
            pose_idx,
            behavior: behavior.to_string(),
            branch_ordinal: pose_idx,
            branch_name: if pose_idx == 0 {
                "first-person ready".to_string()
            } else {
                "first-person crouch derived from ready".to_string()
            },
            clip_generator: string_member(clip, "name").unwrap_or("").to_string(),
            animation_leaf: leaf_basename(animation),
        })
        .collect())
}

/// Lowercased, backslash-normalised key for case/sep-insensitive comparison.
pub(crate) fn norm_key(rel: &str) -> String {
    rel.replace('/', "\\").to_ascii_lowercase()
}

fn ext_is_hkx(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("hkx"))
        .unwrap_or(false)
}

/// Drop the last two path components (`Behaviors\X.hkx`) → the `Actors\<Race>` dir
/// that a `behaviorName` is relative to.
fn drop_last_two(rel: &str) -> String {
    let norm = rel.replace('/', "\\");
    let parts: Vec<&str> = norm.split('\\').filter(|s| !s.is_empty()).collect();
    if parts.len() >= 2 {
        parts[..parts.len() - 2].join("\\")
    } else {
        String::new()
    }
}

fn join_rel(a: &str, b: &str) -> String {
    let mut b = b.replace('/', "\\");
    if Path::new(&b).extension().is_none() {
        b.push_str(".hkx");
    }
    if a.is_empty() { b } else { format!("{a}\\{b}") }
}

/// Basename of an `animationName`, original casing, no extension. CK writes this
/// verbatim as the section-1 clip key, so the graph's casing must survive.
fn leaf_basename_cased(animation_name: &str) -> String {
    let norm = animation_name.replace('/', "\\");
    let last = norm.rsplit('\\').next().unwrap_or(&norm);
    match last.rfind('.') {
        Some(d) => &last[..d],
        None => last,
    }
    .to_string()
}

/// Basename of an `animationName`, lowercased, no extension.
fn leaf_basename(animation_name: &str) -> String {
    leaf_basename_cased(animation_name).to_ascii_lowercase()
}

/// A `SAPT` string used for disk lookup: trailing control bytes (the authored
/// `\r` quirk) are part of the id hash but not the directory name.
fn sapt_dir(sapt: &str) -> String {
    sapt.trim_end_matches(['\r', '\n', ' ']).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaf_basename_strips_dirs_and_ext() {
        assert_eq!(
            leaf_basename(r"Animations\Weapon\44Pistol\WPNReload.hkt"),
            "wpnreload"
        );
        assert_eq!(
            leaf_basename(r"..\PowerArmor\Animations\1HM\ThrustIdle.HKT"),
            "thrustidle"
        );
        assert_eq!(leaf_basename("Bare"), "bare");
    }

    #[test]
    fn drop_last_two_yields_race_dir() {
        assert_eq!(
            drop_last_two(r"Actors\Character\Behaviors\WeaponBehavior.hkx"),
            r"Actors\Character"
        );
        assert_eq!(
            drop_last_two(r"Actors\Character\_1stPerson\Behaviors\GunBehavior.hkx"),
            r"Actors\Character\_1stPerson"
        );
    }

    #[test]
    fn join_rel_resolves_behavior_reference() {
        assert_eq!(
            join_rel(r"Actors\Character", r"Behaviors\WeaponBehavior.hkx"),
            r"Actors\Character\Behaviors\WeaponBehavior.hkx"
        );
        assert_eq!(
            join_rel(r"Actors\Character\_1stPerson", r"Behaviors\GunBehavior"),
            r"Actors\Character\_1stPerson\Behaviors\GunBehavior.hkx"
        );
    }

    #[test]
    fn sapt_dir_trims_authored_carriage_return() {
        assert_eq!(
            sapt_dir("Actors\\PowerArmor\\Animations\\Paired\r"),
            r"Actors\PowerArmor\Animations\Paired"
        );
    }

    #[test]
    fn missing_behavior_is_a_typed_role_failure() {
        let mut resolver = GraphResolver::new(vec![
            std::env::temp_dir().join("modkit-stance-role-fixture-that-does-not-exist"),
        ]);
        assert!(matches!(
            resolver.resolve_stance_pose_roles(
                r"Actors\Character\Behaviors\Missing.hkx",
                &[],
                StancePerspective::ThirdPerson,
            ),
            Err(PoseRoleResolveError::BehaviorMissing(_))
        ));
    }

    /// CK keys furniture section 1 on the animation basename, never the generator's `name`
    /// (generator `Standing Enter` plays `EnterFromStand`; the former appears in 0 of 3156
    /// vanilla `AnimationOffsets` files). A clip-info miss deletes the `InteractionData`
    /// entry, and an empty array makes workbenches refuse activation with `sFailedActivation`.
    #[test]
    fn furniture_clip_generators_expose_animation_basename_not_generator_name() {
        let meshes =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../extracted/fo4/Meshes");
        let core = r"Actors\Character\Behaviors\WorkbenchFurnitureBehavior.hkx";
        if !meshes
            .join("Actors/Character/Behaviors/WorkbenchFurnitureBehavior.hkx")
            .is_file()
        {
            eprintln!("extracted WorkbenchFurnitureBehavior fixture absent; skipping");
            return;
        }
        let mut resolver = GraphResolver::new(vec![meshes]);
        let clips = resolver.resolve_clip_generators(
            core,
            &[r"Actors\Character\Animations\Furniture\WorkbenchChemistryA".to_string()],
        );
        assert!(
            !clips.is_empty(),
            "chem furniture subgraph resolved no clips"
        );

        // The generator name must still be reported (weapon/creature key on it) ...
        assert!(
            clips.iter().any(|(name, ..)| name == "Standing Enter"),
            "expected the `Standing Enter` generator in the FO4 furniture graph",
        );
        // ... but the furniture key for that same clip is the animation basename, cased as
        // the graph spells it, which is what CK writes and what the engine looks up.
        let entry = clips
            .iter()
            .find(|(name, ..)| name == "Standing Enter")
            .expect("Standing Enter generator");
        assert_eq!(entry.1, "EnterFromStand");
        assert!(
            !clips
                .iter()
                .any(|(_, basename, ..)| basename == "Standing Enter"),
            "no furniture section-1 key may be a generator name",
        );
    }

    #[test]
    fn weapon_behavior_yields_six_owned_branches_and_twelve_role_keys() {
        let meshes =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../extracted/fo4/Meshes");
        let behavior = meshes.join("Actors/Character/Behaviors/WeaponBehavior.hkx");
        if !behavior.is_file() {
            eprintln!("extracted WeaponBehavior fixture absent; skipping");
            return;
        }
        let mut resolver = GraphResolver::new(vec![meshes]);
        let roles = resolver
            .resolve_stance_pose_roles(
                r"Actors\Character\Behaviors\WeaponBehavior.hkx",
                &[
                    r"Actors\Character\Animations\Weapon\Pistol".to_string(),
                    r"Actors\Character\Animations\Weapon\Rifle\Neutral".to_string(),
                    r"Actors\Character\Animations\Paired".to_string(),
                    r"Actors\Character\Animations".to_string(),
                ],
                StancePerspective::ThirdPerson,
            )
            .expect("structured capture-pose graph");

        assert_eq!(roles.len(), 12);
        assert_eq!(
            roles
                .iter()
                .map(|role| role.branch_ordinal)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([0, 1, 2, 3, 4, 5])
        );
        assert!(roles.iter().all(|role| role.source_key().is_some()));
        let source_keys: HashSet<_> = roles.iter().filter_map(|role| role.source_key()).collect();
        assert_eq!(source_keys.len(), 12);
    }
}
