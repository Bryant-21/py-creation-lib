//! Populated weapon SyncAnimData generation from structured Havok packfiles.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::fmt;
use std::path::{Path, PathBuf};

use havok_native::hkx::types::HkxValue;
use havok_native::hkx::{HkxMember, HkxObject, read_packfile};

use super::core::name_id;
use super::emit::SubgraphInput;

const ROLE_SUFFIXES: &[&str] = &[
    "_attackerlead",
    "_deathclawlead",
    "_victimlead",
    "_humanlead",
    "_humandead",
    "_attacker",
    "_victim",
    "_lead",
    "_human",
    "_npc",
    "_dead",
];

/// One emitter-ready populated weapon SyncAnimData file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeaponSyncAnimData {
    pub filename: String,
    pub body: Vec<u8>,
}

/// A strict SyncAnim build failure. An absent clip outside the supplied animation
/// directories is non-participating; a selected clip that cannot be decoded is an error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncAnimBuildError(String);

impl SyncAnimBuildError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for SyncAnimBuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SyncAnimBuildError {}

#[derive(Debug, Clone, PartialEq)]
struct SyncAnimEntry {
    event: String,
    id: u32,
    translation: [f32; 3],
    rotation_wxyz: [f32; 4],
}

#[derive(Debug, Clone)]
struct SyncCandidate {
    event: String,
    animation_name: String,
}

#[derive(Debug, Clone)]
struct BehaviorInfo {
    references: Vec<String>,
    candidates: Vec<SyncCandidate>,
}

struct ResolvedGroup {
    behavior_roots: Vec<String>,
    animation_dirs: Vec<String>,
    subgraph_ids: Vec<u64>,
}

struct ClipResolver {
    roots: Vec<PathBuf>,
    dir_cache: HashMap<String, HashMap<String, PathBuf>>,
    behavior_cache: HashMap<String, BehaviorInfo>,
}

impl ClipResolver {
    fn new(roots: &[&Path]) -> Self {
        Self {
            roots: roots.iter().map(|root| root.to_path_buf()).collect(),
            dir_cache: HashMap::new(),
            behavior_cache: HashMap::new(),
        }
    }

    fn find_relative_file(&self, relative: &str) -> Result<Option<PathBuf>, SyncAnimBuildError> {
        for root in &self.roots {
            if let Some(path) = resolve_case_insensitive(root, relative)? {
                if path.is_file() {
                    return Ok(Some(path));
                }
            }
        }
        Ok(None)
    }

    fn resolve_animation(
        &mut self,
        animation_name: &str,
        dirs: &[String],
    ) -> Result<Option<PathBuf>, SyncAnimBuildError> {
        let stem = animation_stem(animation_name)
            .ok_or_else(|| {
                SyncAnimBuildError::new(format!("animation has no file stem: {animation_name}"))
            })?
            .to_ascii_lowercase();
        for dir in dirs {
            let key = normalize_rel(dir);
            if !self.dir_cache.contains_key(&key) {
                let mut files = HashMap::new();
                for root in &self.roots {
                    let Some(disk_dir) = resolve_case_insensitive(root, dir)? else {
                        continue;
                    };
                    let entries = std::fs::read_dir(&disk_dir).map_err(|error| {
                        SyncAnimBuildError::new(format!(
                            "failed to read animation directory {}: {error}",
                            disk_dir.display()
                        ))
                    })?;
                    for entry in entries {
                        let entry = entry.map_err(|error| {
                            SyncAnimBuildError::new(format!(
                                "failed to enumerate animation directory {}: {error}",
                                disk_dir.display()
                            ))
                        })?;
                        let path = entry.path();
                        if !path.is_file()
                            || !path
                                .extension()
                                .and_then(|ext| ext.to_str())
                                .is_some_and(|ext| ext.eq_ignore_ascii_case("hkx"))
                        {
                            continue;
                        }
                        if let Some(file_stem) = path.file_stem().and_then(|value| value.to_str()) {
                            files.entry(file_stem.to_ascii_lowercase()).or_insert(path);
                        }
                    }
                }
                self.dir_cache.insert(key.clone(), files);
            }
            if let Some(path) = self.dir_cache.get(&key).and_then(|files| files.get(&stem)) {
                return Ok(Some(path.clone()));
            }
        }
        Ok(None)
    }

    fn behavior_info(&mut self, relative: &str) -> Result<BehaviorInfo, SyncAnimBuildError> {
        let key = normalize_rel(relative);
        if let Some(info) = self.behavior_cache.get(&key) {
            return Ok(info.clone());
        }
        let path = self
            .find_relative_file(relative)?
            .ok_or_else(|| SyncAnimBuildError::new(format!("behavior not found: {relative}")))?;
        let data = std::fs::read(&path).map_err(|error| {
            SyncAnimBuildError::new(format!("failed to read {}: {error}", path.display()))
        })?;
        let hkx = read_packfile(&data).map_err(|error| {
            SyncAnimBuildError::new(format!(
                "failed to parse {} as a packfile: {error}",
                path.display()
            ))
        })?;
        let references = behavior_references(relative, hkx.objects())?;
        let candidates = if has_pa_events(hkx.objects())? {
            collect_sync_candidates(hkx.objects())?
        } else {
            Vec::new()
        };
        let info = BehaviorInfo {
            references,
            candidates,
        };
        self.behavior_cache.insert(key, info.clone());
        Ok(info)
    }

    fn synchronized_behavior_roots(
        &mut self,
        subgraphs: &[&SubgraphInput],
    ) -> Result<Vec<String>, SyncAnimBuildError> {
        let mut roots = Vec::<(usize, String)>::new();
        let mut visited = HashSet::new();
        for subgraph in subgraphs {
            let mut queue = VecDeque::from([subgraph.core_behavior.clone()]);
            while let Some(relative) = queue.pop_front() {
                let key = normalize_rel(&relative);
                if !visited.insert(key) {
                    continue;
                }
                let info = self.behavior_info(&relative)?;
                if !info.candidates.is_empty() {
                    roots.push((info.candidates.len(), relative));
                }
                queue.extend(info.references);
            }
        }
        roots.sort_by_key(|(candidate_count, _)| std::cmp::Reverse(*candidate_count));
        if roots.is_empty() {
            return Err(SyncAnimBuildError::new(
                "weapon subgraph closures contain no synchronized pa_ transitions",
            ));
        }
        Ok(roots.into_iter().map(|(_, relative)| relative).collect())
    }
}

fn normalize_rel(relative: &str) -> String {
    relative.replace('/', "\\").to_ascii_lowercase()
}

fn resolve_case_insensitive(
    root: &Path,
    relative: &str,
) -> Result<Option<PathBuf>, SyncAnimBuildError> {
    let mut current = root.to_path_buf();
    for component in relative
        .replace('/', "\\")
        .split('\\')
        .filter(|component| !component.is_empty() && *component != ".")
    {
        if component == ".." {
            current.pop();
            continue;
        }
        let direct = current.join(component);
        if direct.exists() {
            current = direct;
            continue;
        }
        let entries = match std::fs::read_dir(&current) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(SyncAnimBuildError::new(format!(
                    "failed to resolve {relative} below {}: {error}",
                    root.display()
                )));
            }
        };
        let mut match_entry = None;
        for entry in entries {
            let entry = entry.map_err(|error| {
                SyncAnimBuildError::new(format!(
                    "failed to resolve {relative} below {}: {error}",
                    root.display()
                ))
            })?;
            if entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.eq_ignore_ascii_case(component))
            {
                match_entry = Some(entry);
                break;
            }
        }
        let Some(entry) = match_entry else {
            return Ok(None);
        };
        current = entry.path();
    }
    Ok(current.exists().then_some(current))
}

fn behavior_parent(relative: &str) -> String {
    let normalized = relative.replace('/', "\\");
    let mut components = normalized
        .split('\\')
        .filter(|component| !component.is_empty())
        .collect::<Vec<_>>();
    components.truncate(components.len().saturating_sub(2));
    components.join("\\")
}

fn resolve_behavior_reference(source: &str, reference: &str) -> String {
    let reference = reference.replace('/', "\\");
    if reference
        .get(..7)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("Actors\\"))
    {
        reference
    } else {
        let parent = behavior_parent(source);
        if parent.is_empty() {
            reference
        } else {
            format!("{parent}\\{reference}")
        }
    }
}

fn behavior_references(
    source: &str,
    objects: &[HkxObject],
) -> Result<Vec<String>, SyncAnimBuildError> {
    objects
        .iter()
        .enumerate()
        .filter(|(_, object)| object.class_name == "hkbBehaviorReferenceGenerator")
        .map(|(index, object)| {
            let reference = string_member(object, "behaviorName").ok_or_else(|| {
                SyncAnimBuildError::new(format!(
                    "{source} hkbBehaviorReferenceGenerator object {index} has no behaviorName"
                ))
            })?;
            Ok(resolve_behavior_reference(source, reference))
        })
        .collect()
}

fn as_i64(value: &HkxValue) -> Option<i64> {
    match value {
        HkxValue::I8(value) => Some(*value as i64),
        HkxValue::U8(value) => Some(*value as i64),
        HkxValue::I16(value) => Some(*value as i64),
        HkxValue::U16(value) => Some(*value as i64),
        HkxValue::I32(value) => Some(*value as i64),
        HkxValue::U32(value) => Some(*value as i64),
        HkxValue::I64(value) => Some(*value),
        HkxValue::U64(value) => i64::try_from(*value).ok(),
        _ => None,
    }
}

fn value_member<'a>(object: &'a HkxObject, name: &str) -> Option<&'a HkxValue> {
    object
        .members
        .iter()
        .find(|member| member.name == name)
        .map(|member| &member.value)
}

fn value_in<'a>(members: &'a [HkxMember], name: &str) -> Option<&'a HkxValue> {
    members
        .iter()
        .find(|member| member.name == name)
        .map(|member| &member.value)
}

fn int_member(object: &HkxObject, name: &str) -> Option<i64> {
    value_member(object, name).and_then(as_i64)
}

fn int_in(members: &[HkxMember], name: &str) -> Option<i64> {
    value_in(members, name).and_then(as_i64)
}

fn string_member<'a>(object: &'a HkxObject, name: &str) -> Option<&'a str> {
    value_member(object, name).and_then(|value| match value {
        HkxValue::String { value, .. } if !value.is_empty() => Some(value.as_str()),
        _ => None,
    })
}

fn string_in<'a>(members: &'a [HkxMember], name: &str) -> Option<&'a str> {
    value_in(members, name).and_then(|value| match value {
        HkxValue::String { value, .. } if !value.is_empty() => Some(value.as_str()),
        _ => None,
    })
}

fn array_member<'a>(object: &'a HkxObject, name: &str) -> Option<&'a [HkxValue]> {
    value_member(object, name).and_then(|value| match value {
        HkxValue::Array(values) => Some(values.as_slice()),
        _ => None,
    })
}

fn pointer_targets(value: &HkxValue, context: &str) -> Result<Vec<usize>, SyncAnimBuildError> {
    match value {
        HkxValue::Pointer(Some(index)) => Ok(vec![*index]),
        HkxValue::Pointer(None) => Ok(Vec::new()),
        HkxValue::Array(values) => values
            .iter()
            .enumerate()
            .map(|(index, value)| match value {
                HkxValue::Pointer(Some(target)) => Ok(*target),
                HkxValue::Pointer(None) => Err(SyncAnimBuildError::new(format!(
                    "{context}[{index}] is a null pointer"
                ))),
                _ => Err(SyncAnimBuildError::new(format!(
                    "{context}[{index}] is not a pointer"
                ))),
            })
            .collect(),
        _ => Err(SyncAnimBuildError::new(format!(
            "{context} is not a pointer or pointer array"
        ))),
    }
}

fn pointer_member(object: &HkxObject, name: &str) -> Result<Vec<usize>, SyncAnimBuildError> {
    let Some(value) = value_member(object, name) else {
        return Ok(Vec::new());
    };
    pointer_targets(value, &format!("{}.{}", object.class_name, name))
}

fn event_names(objects: &[HkxObject]) -> Result<Option<Vec<String>>, SyncAnimBuildError> {
    let Some(string_data) = objects
        .iter()
        .find(|object| object.class_name == "hkbBehaviorGraphStringData")
    else {
        return Ok(None);
    };
    let values = array_member(string_data, "eventNames").ok_or_else(|| {
        SyncAnimBuildError::new("hkbBehaviorGraphStringData has no eventNames array")
    })?;
    values
        .iter()
        .enumerate()
        .map(|(index, value)| match value {
            HkxValue::String { value, .. } => Ok(value.clone()),
            _ => Err(SyncAnimBuildError::new(format!(
                "eventNames[{index}] is not a string"
            ))),
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

fn has_pa_events(objects: &[HkxObject]) -> Result<bool, SyncAnimBuildError> {
    Ok(event_names(objects)?.is_some_and(|names| {
        names.iter().any(|event| {
            event
                .get(..3)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("pa_"))
        })
    }))
}

fn state_map(
    state_machine: &HkxObject,
    objects: &[HkxObject],
) -> Result<Vec<(i64, usize)>, SyncAnimBuildError> {
    let state_indices = pointer_member(state_machine, "states")?;
    if state_indices.is_empty() {
        return Err(SyncAnimBuildError::new("hkbStateMachine has no states"));
    }
    state_indices
        .into_iter()
        .map(|index| {
            let state = objects.get(index).ok_or_else(|| {
                SyncAnimBuildError::new(format!(
                    "hkbStateMachine.states points outside the object table: {index}"
                ))
            })?;
            let state_id = int_member(state, "stateId").ok_or_else(|| {
                SyncAnimBuildError::new(format!("state object {index} has no stateId"))
            })?;
            Ok((state_id, index))
        })
        .collect()
}

fn transition_members<'a>(
    value: &'a HkxValue,
    objects: &'a [HkxObject],
) -> Result<&'a [HkxMember], SyncAnimBuildError> {
    match value {
        HkxValue::Pointer(Some(index)) => objects
            .get(*index)
            .map(|object| object.members.as_slice())
            .ok_or_else(|| {
                SyncAnimBuildError::new(format!(
                    "transition points outside the object table: {index}"
                ))
            }),
        HkxValue::Pointer(None) => Err(SyncAnimBuildError::new("transition is a null pointer")),
        _ => value
            .as_object_members()
            .ok_or_else(|| SyncAnimBuildError::new("transition is not an inline object")),
    }
}

fn pa_transition_targets(
    state_machine: &HkxObject,
    objects: &[HkxObject],
    names: &[String],
) -> Result<Vec<(String, usize)>, SyncAnimBuildError> {
    let states = state_map(state_machine, objects)?;
    let mut transition_arrays = pointer_member(state_machine, "wildcardTransitions")?;
    for (_, state_index) in &states {
        let state = objects.get(*state_index).ok_or_else(|| {
            SyncAnimBuildError::new(format!("invalid state pointer: {state_index}"))
        })?;
        transition_arrays.extend(pointer_member(state, "transitions")?);
    }

    let mut events_by_state: HashMap<i64, Vec<String>> = HashMap::new();
    for array_index in transition_arrays {
        let array = objects.get(array_index).ok_or_else(|| {
            SyncAnimBuildError::new(format!(
                "transition array points outside the object table: {array_index}"
            ))
        })?;
        let transitions = array_member(array, "transitions").ok_or_else(|| {
            SyncAnimBuildError::new(format!(
                "transition array object {array_index} has no transitions array"
            ))
        })?;
        for (transition_index, transition) in transitions.iter().enumerate() {
            let members = transition_members(transition, objects)?;
            let event_id = int_in(members, "eventId").ok_or_else(|| {
                SyncAnimBuildError::new(format!(
                    "transition {array_index}[{transition_index}] has no eventId"
                ))
            })?;
            let to_state_id = int_in(members, "toStateId").ok_or_else(|| {
                SyncAnimBuildError::new(format!(
                    "transition {array_index}[{transition_index}] has no toStateId"
                ))
            })?;
            if event_id < 0 {
                continue;
            }
            let event = names.get(event_id as usize).ok_or_else(|| {
                SyncAnimBuildError::new(format!(
                    "transition eventId {event_id} is outside eventNames"
                ))
            })?;
            if !event
                .get(..3)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("pa_"))
            {
                continue;
            }
            events_by_state
                .entry(to_state_id)
                .or_default()
                .push(event.clone());
        }
    }
    let mut targets = Vec::new();
    for (state_id, state_index) in states {
        let Some(events) = events_by_state.remove(&state_id) else {
            continue;
        };
        let state = objects.get(state_index).ok_or_else(|| {
            SyncAnimBuildError::new(format!("invalid state pointer: {state_index}"))
        })?;
        let generators = pointer_member(state, "generator")?;
        let [generator] = generators.as_slice() else {
            return Err(SyncAnimBuildError::new(format!(
                "pa_ transition target stateId {state_id} must have one generator"
            )));
        };
        if objects.get(*generator).is_none() {
            return Err(SyncAnimBuildError::new(format!(
                "stateId {state_id} generator points outside the object table: {generator}"
            )));
        }
        targets.extend(events.into_iter().map(|event| (event, *generator)));
    }
    if let Some(state_id) = events_by_state.keys().next() {
        return Err(SyncAnimBuildError::new(format!(
            "pa_ transition targets missing stateId {state_id}"
        )));
    }
    Ok(targets)
}

fn generator_children(
    object: &HkxObject,
    objects: &[HkxObject],
) -> Result<Vec<usize>, SyncAnimBuildError> {
    const EDGE_MEMBERS: &[&str] = &[
        "pDefaultGenerator",
        "generator",
        "pGenerator",
        "generators",
        "children",
        "ChildrenA",
        "layers",
    ];

    let mut out = Vec::new();
    for name in EDGE_MEMBERS {
        for index in pointer_member(object, name)? {
            let child = objects.get(index).ok_or_else(|| {
                SyncAnimBuildError::new(format!(
                    "{}.{} points outside the object table: {index}",
                    object.class_name, name
                ))
            })?;
            if child.class_name == "hkbBlenderGeneratorChild"
                || child.class_name == "hkbLayer"
                || child.class_name == "BSBoneSwitchGeneratorBoneData"
            {
                out.extend(generator_children(child, objects)?);
            } else {
                out.push(index);
            }
        }
    }
    Ok(out)
}

fn collect_sync_candidates(
    objects: &[HkxObject],
) -> Result<Vec<SyncCandidate>, SyncAnimBuildError> {
    let names = event_names(objects)?.ok_or_else(|| {
        SyncAnimBuildError::new("behavior with pa_ transitions has no eventNames")
    })?;
    let root = objects
        .iter()
        .find(|object| object.class_name == "hkbBehaviorGraph")
        .ok_or_else(|| SyncAnimBuildError::new("behavior has no hkbBehaviorGraph object"))?;
    let roots = pointer_member(root, "rootGenerator")?;
    let [root] = roots.as_slice() else {
        return Err(SyncAnimBuildError::new(
            "behavior must have one root generator",
        ));
    };
    if objects.get(*root).is_none() {
        return Err(SyncAnimBuildError::new(format!(
            "behavior root generator points outside the object table: {root}"
        )));
    }

    let mut queue = VecDeque::from([(*root, None::<String>)]);
    let mut seen = HashSet::new();
    let mut candidates = Vec::new();

    while let Some((index, event)) = queue.pop_front() {
        let seen_key = (
            index,
            event.as_ref().map(|value| value.to_ascii_lowercase()),
        );
        if !seen.insert(seen_key) {
            continue;
        }
        let object = objects.get(index).ok_or_else(|| {
            SyncAnimBuildError::new(format!(
                "generator traversal points outside the object table: {index}"
            ))
        })?;
        match object.class_name.as_str() {
            "hkbClipGenerator" => {
                if let Some(event) = event {
                    let animation_name =
                        string_member(object, "animationName").ok_or_else(|| {
                            SyncAnimBuildError::new(format!(
                                "synchronized clip for {event} has no animationName"
                            ))
                        })?;
                    candidates.push(SyncCandidate {
                        event,
                        animation_name: animation_name.to_string(),
                    });
                }
            }
            "hkbStateMachine" if event.is_none() => {
                let targets = pa_transition_targets(object, objects, &names)?;
                if targets.is_empty() {
                    for (state_id, state_index) in state_map(object, objects)? {
                        let state = objects.get(state_index).ok_or_else(|| {
                            SyncAnimBuildError::new(format!("invalid state pointer: {state_index}"))
                        })?;
                        let generators = pointer_member(state, "generator")?;
                        let [generator] = generators.as_slice() else {
                            return Err(SyncAnimBuildError::new(format!(
                                "stateId {state_id} must have one generator"
                            )));
                        };
                        queue.push_back((*generator, None));
                    }
                } else {
                    for (event, generator) in targets {
                        queue.push_back((generator, Some(event)));
                    }
                }
            }
            "hkbStateMachine" => {
                let start_state_id = int_member(object, "startStateId").ok_or_else(|| {
                    SyncAnimBuildError::new("nested hkbStateMachine has no startStateId")
                })?;
                let (_, state_index) = state_map(object, objects)?
                    .into_iter()
                    .find(|(state_id, _)| *state_id == start_state_id)
                    .ok_or_else(|| {
                        SyncAnimBuildError::new(format!(
                            "nested state machine startStateId {start_state_id} is missing"
                        ))
                    })?;
                let state = objects.get(state_index).ok_or_else(|| {
                    SyncAnimBuildError::new(format!("invalid state pointer: {state_index}"))
                })?;
                let generators = pointer_member(state, "generator")?;
                let [generator] = generators.as_slice() else {
                    return Err(SyncAnimBuildError::new(format!(
                        "nested stateId {start_state_id} must have one generator"
                    )));
                };
                queue.push_back((*generator, event));
            }
            _ => {
                for child in generator_children(object, objects)? {
                    queue.push_back((child, event.clone()));
                }
            }
        }
    }

    Ok(candidates)
}

fn animation_stem(animation_name: &str) -> Option<&str> {
    let basename = animation_name
        .rsplit(['\\', '/'])
        .next()
        .filter(|basename| !basename.is_empty())?;
    Some(
        basename
            .rfind('.')
            .map(|index| &basename[..index])
            .unwrap_or(basename),
    )
}

fn strip_role_suffix(stem: &str) -> &str {
    let lowercase = stem.to_ascii_lowercase();
    ROLE_SUFFIXES
        .iter()
        .find_map(|suffix| {
            lowercase
                .ends_with(suffix)
                .then(|| &stem[..stem.len() - suffix.len()])
        })
        .unwrap_or(stem)
}

fn canonical_event(event: &str, base: &str) -> String {
    let Some(body) = event.get(3..) else {
        return event.to_string();
    };
    if let Some(without_paired) = base.get(6..).filter(|_| {
        base.get(..6)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("paired"))
    }) {
        if body.eq_ignore_ascii_case(without_paired) {
            return format!("pa_{without_paired}");
        }
    }
    event.to_string()
}

fn matrix_to_quaternion_wxyz(transform: &[f32]) -> Option<[f32; 4]> {
    if transform.len() < 12 || transform[..12].iter().any(|value| !value.is_finite()) {
        return None;
    }
    let (r00, r10, r20) = (transform[0], transform[1], transform[2]);
    let (r01, r11, r21) = (transform[4], transform[5], transform[6]);
    let (r02, r12, r22) = (transform[8], transform[9], transform[10]);
    let trace = r00 + r11 + r22;
    let quaternion = if trace > 0.0 {
        let scale = (trace + 1.0).sqrt() * 2.0;
        [
            0.25 * scale,
            (r21 - r12) / scale,
            (r02 - r20) / scale,
            (r10 - r01) / scale,
        ]
    } else if r00 > r11 && r00 > r22 {
        let scale = (1.0 + r00 - r11 - r22).sqrt() * 2.0;
        [
            (r21 - r12) / scale,
            0.25 * scale,
            (r01 + r10) / scale,
            (r02 + r20) / scale,
        ]
    } else if r11 > r22 {
        let scale = (1.0 + r11 - r00 - r22).sqrt() * 2.0;
        [
            (r02 - r20) / scale,
            (r01 + r10) / scale,
            0.25 * scale,
            (r12 + r21) / scale,
        ]
    } else {
        let scale = (1.0 + r22 - r00 - r11).sqrt() * 2.0;
        [
            (r10 - r01) / scale,
            (r02 + r20) / scale,
            (r12 + r21) / scale,
            0.25 * scale,
        ]
    };
    quaternion
        .iter()
        .all(|value| value.is_finite())
        .then_some(quaternion)
}

fn sync_anim_offset(clip_hkx: &Path) -> Result<([f32; 3], [f32; 4]), SyncAnimBuildError> {
    let data = std::fs::read(clip_hkx).map_err(|error| {
        SyncAnimBuildError::new(format!("failed to read {}: {error}", clip_hkx.display()))
    })?;
    let hkx = read_packfile(&data).map_err(|error| {
        SyncAnimBuildError::new(format!(
            "failed to parse {} as a packfile: {error}",
            clip_hkx.display()
        ))
    })?;
    let objects = hkx.objects();
    let root = objects
        .iter()
        .find(|object| object.class_name == "hkRootLevelContainer")
        .ok_or_else(|| {
            SyncAnimBuildError::new(format!(
                "{} has no hkRootLevelContainer",
                clip_hkx.display()
            ))
        })?;
    let variants = array_member(root, "namedVariants").ok_or_else(|| {
        SyncAnimBuildError::new(format!(
            "{} root container has no namedVariants array",
            clip_hkx.display()
        ))
    })?;
    let mut frame_index = None;
    for (index, variant) in variants.iter().enumerate() {
        let members = variant.as_object_members().ok_or_else(|| {
            SyncAnimBuildError::new(format!(
                "{} namedVariants[{index}] is not an object",
                clip_hkx.display()
            ))
        })?;
        if string_in(members, "name") != Some("SyncAnimOffset") {
            continue;
        }
        let pointer = value_in(members, "variant").ok_or_else(|| {
            SyncAnimBuildError::new(format!(
                "{} SyncAnimOffset variant has no pointer",
                clip_hkx.display()
            ))
        })?;
        let targets = pointer_targets(pointer, "SyncAnimOffset.variant")?;
        let [target] = targets.as_slice() else {
            return Err(SyncAnimBuildError::new(format!(
                "{} SyncAnimOffset must point to one frame",
                clip_hkx.display()
            )));
        };
        frame_index = Some(*target);
        break;
    }
    let frame_index = frame_index.ok_or_else(|| {
        SyncAnimBuildError::new(format!(
            "{} has no SyncAnimOffset named variant",
            clip_hkx.display()
        ))
    })?;
    let frame = objects.get(frame_index).ok_or_else(|| {
        SyncAnimBuildError::new(format!(
            "{} has an invalid SyncAnimOffset pointer",
            clip_hkx.display()
        ))
    })?;
    let transform = value_member(frame, "transform")
        .and_then(|value| match value {
            HkxValue::F32List(values) if values.len() == 16 => Some(values.as_slice()),
            _ => None,
        })
        .ok_or_else(|| {
            SyncAnimBuildError::new(format!(
                "{} has no exact 16-float SyncAnimOffset transform",
                clip_hkx.display()
            ))
        })?;
    if transform.iter().any(|value| !value.is_finite()) {
        return Err(SyncAnimBuildError::new(format!(
            "{} has a non-finite SyncAnimOffset transform",
            clip_hkx.display()
        )));
    }
    let translation = [transform[12], transform[13], transform[14]];
    let rotation_wxyz = matrix_to_quaternion_wxyz(transform).ok_or_else(|| {
        SyncAnimBuildError::new(format!(
            "{} has an invalid SyncAnimOffset rotation",
            clip_hkx.display()
        ))
    })?;
    Ok((translation, rotation_wxyz))
}

fn path_components(path: &str) -> Vec<String> {
    path.replace('/', "\\")
        .split('\\')
        .filter(|component| !component.is_empty())
        .map(|component| {
            component
                .trim_end_matches(['\r', '\n', ' '])
                .to_ascii_lowercase()
        })
        .collect()
}

fn is_first_person(subgraph: &SubgraphInput) -> bool {
    normalize_rel(&subgraph.core_behavior).contains("\\_1stperson\\")
        || subgraph
            .sapt_chain
            .iter()
            .any(|path| normalize_rel(path).contains("\\_1stperson\\"))
}

fn third_person_archetype(path: &str) -> Option<String> {
    let components = path_components(path);
    components.windows(5).find_map(|window| {
        (window[0] == "actors"
            && window[1] == "character"
            && window[2] == "animations"
            && window[3] == "weapon")
            .then(|| window[4].clone())
    })
}

fn first_person_branch(path: &str) -> Option<(String, bool)> {
    let components = path_components(path);
    components.windows(5).find_map(|window| {
        (window[0] == "actors"
            && window[1] == "character"
            && window[2] == "_1stperson"
            && window[3] == "animations"
            && window[4] != "paired"
            && window[4] != "common")
            .then(|| {
                let branch_index = components
                    .windows(5)
                    .position(|candidate| candidate == window)
                    .unwrap()
                    + 4;
                (window[4].clone(), components.len() > branch_index + 1)
            })
    })
}

fn contains_third_person_archetype(subgraph: &SubgraphInput, archetype: &str) -> bool {
    !is_first_person(subgraph)
        && subgraph
            .sapt_chain
            .iter()
            .filter_map(|path| third_person_archetype(path))
            .any(|candidate| candidate.eq_ignore_ascii_case(archetype))
}

fn contains_first_person_archetype(
    subgraph: &SubgraphInput,
    archetype: &str,
    specialized_only: bool,
) -> bool {
    is_first_person(subgraph)
        && subgraph
            .sapt_chain
            .iter()
            .filter_map(|path| first_person_branch(path))
            .any(|(candidate, specialized)| {
                candidate.eq_ignore_ascii_case(archetype) && (!specialized_only || specialized)
            })
}

fn deduplicated_subgraphs<'a>(
    subgraphs: impl IntoIterator<Item = &'a SubgraphInput>,
) -> Vec<&'a SubgraphInput> {
    let mut seen = HashSet::new();
    let mut deduplicated: Vec<_> = subgraphs
        .into_iter()
        .filter(|subgraph| seen.insert(subgraph.id()))
        .collect();
    deduplicated.sort_unstable_by_key(|subgraph| subgraph.id());
    deduplicated
}

fn animation_directories(subgraphs: &[&SubgraphInput]) -> Vec<String> {
    let mut seen = HashSet::new();
    subgraphs
        .iter()
        .flat_map(|subgraph| subgraph.sapt_chain.iter())
        .filter(|path| seen.insert(normalize_rel(path)))
        .cloned()
        .collect()
}

type DerivedGroups<'a> = (String, Vec<&'a SubgraphInput>, Vec<&'a SubgraphInput>);

fn complete_group_candidates<'a>(
    subgraphs: &'a [SubgraphInput],
) -> Result<Vec<DerivedGroups<'a>>, SyncAnimBuildError> {
    let unique = deduplicated_subgraphs(subgraphs.iter());
    let archetypes: BTreeSet<String> = unique
        .iter()
        .filter(|subgraph| !is_first_person(subgraph))
        .flat_map(|subgraph| subgraph.sapt_chain.iter())
        .filter_map(|path| third_person_archetype(path))
        .collect();
    if archetypes.is_empty() {
        return Err(SyncAnimBuildError::new(
            "weapon subgraphs contain no Actors\\Character\\Animations\\Weapon archetype",
        ));
    }

    let mut complete = Vec::new();
    for archetype in archetypes {
        let third_person = deduplicated_subgraphs(
            unique
                .iter()
                .copied()
                .filter(|subgraph| contains_third_person_archetype(subgraph, &archetype)),
        );
        let specialized_first_person = deduplicated_subgraphs(
            unique
                .iter()
                .copied()
                .filter(|subgraph| contains_first_person_archetype(subgraph, &archetype, true)),
        );
        let first_person =
            if specialized_first_person.is_empty() {
                deduplicated_subgraphs(unique.iter().copied().filter(|subgraph| {
                    contains_first_person_archetype(subgraph, &archetype, false)
                }))
            } else {
                specialized_first_person
            };
        if !first_person.is_empty() && !third_person.is_empty() {
            complete.push((archetype, first_person, third_person));
        }
    }
    if complete.is_empty() {
        return Err(SyncAnimBuildError::new(
            "weapon subgraphs contain no archetype with both first- and third-person groups",
        ));
    }
    Ok(complete)
}

pub fn weapon_sync_anim_filenames(
    subgraphs: &[SubgraphInput],
) -> Result<Vec<String>, SyncAnimBuildError> {
    Ok(complete_group_candidates(subgraphs)?
        .into_iter()
        .map(|(archetype, _, _)| {
            format!("ResolvedSyncAnimData{}.txt", archetype.to_ascii_lowercase())
        })
        .collect())
}

fn resolve_group(
    resolver: &mut ClipResolver,
    subgraphs: &[&SubgraphInput],
) -> Result<ResolvedGroup, SyncAnimBuildError> {
    Ok(ResolvedGroup {
        behavior_roots: resolver.synchronized_behavior_roots(subgraphs)?,
        animation_dirs: animation_directories(subgraphs),
        subgraph_ids: subgraphs.iter().map(|subgraph| subgraph.id()).collect(),
    })
}

fn build_group(
    resolver: &mut ClipResolver,
    input: &ResolvedGroup,
) -> Result<Vec<SyncAnimEntry>, SyncAnimBuildError> {
    let mut candidates = Vec::new();
    for behavior in &input.behavior_roots {
        candidates.extend(resolver.behavior_info(behavior)?.candidates);
    }

    let mut entries = Vec::new();
    let mut emitted_events = HashSet::new();
    for candidate in candidates {
        let event_key = candidate.event.to_ascii_lowercase();
        if emitted_events.contains(&event_key) {
            continue;
        }
        let Some(clip) =
            resolver.resolve_animation(&candidate.animation_name, &input.animation_dirs)?
        else {
            // SGNM graphs contain branches for other SAPT directory sets. A clip is
            // selected only when its stem exists in this group's derived SAPT roots.
            continue;
        };
        let stem = animation_stem(&candidate.animation_name).ok_or_else(|| {
            SyncAnimBuildError::new(format!(
                "animation has no file stem: {}",
                candidate.animation_name
            ))
        })?;
        let base = strip_role_suffix(stem);
        let (translation, rotation_wxyz) = sync_anim_offset(&clip)
            .map_err(|error| SyncAnimBuildError::new(format!("{} ({})", error, candidate.event)))?;
        emitted_events.insert(event_key);
        entries.push(SyncAnimEntry {
            event: canonical_event(&candidate.event, base),
            id: name_id(base),
            translation,
            rotation_wxyz,
        });
    }
    if entries.is_empty() {
        return Err(SyncAnimBuildError::new(
            "synchronized behavior roots produced no resolved entries",
        ));
    }
    Ok(entries)
}

fn canonical_participant_ids(subgraph_ids: &[u64]) -> Vec<u64> {
    let mut seen = HashSet::new();
    let mut ids: Vec<_> = subgraph_ids
        .iter()
        .copied()
        .filter(|id| seen.insert(*id))
        .collect();
    ids.sort_unstable();
    ids
}

fn serialize_group(out: &mut String, entries: &[SyncAnimEntry], subgraph_ids: &[u64]) {
    out.push_str(&entries.len().to_string());
    out.push('\n');
    for entry in entries {
        out.push_str(&entry.event);
        out.push('\n');
        out.push_str(&entry.id.to_string());
        out.push('\n');
        out.push_str(&format!(
            "{:.6} {:.6} {:.6}\n",
            entry.translation[0], entry.translation[1], entry.translation[2]
        ));
        out.push_str(&format!(
            "{:.6} {:.6} {:.6} {:.6}\n",
            entry.rotation_wxyz[0],
            entry.rotation_wxyz[1],
            entry.rotation_wxyz[2],
            entry.rotation_wxyz[3]
        ));
    }
    let participant_ids = canonical_participant_ids(subgraph_ids);
    out.push_str(&participant_ids.len().to_string());
    out.push('\n');
    for id in participant_ids {
        out.push_str(&id.to_string());
        out.push('\n');
    }
}

/// Derive and build one populated weapon SyncAnimData file per complete archetype.
///
/// `roots` is `[mod meshes, base meshes]`; override resolution is always mod-before-base.
/// Outputs are sorted by canonical filename. Each output's first- and third-person
/// groups are derived from the same archetype.
pub fn build_weapon_sync_anim_data(
    subgraphs: &[SubgraphInput],
    roots: [&Path; 2],
) -> Result<Vec<WeaponSyncAnimData>, SyncAnimBuildError> {
    if subgraphs.is_empty() {
        return Err(SyncAnimBuildError::new("no weapon subgraphs supplied"));
    }
    for (index, root) in roots.iter().enumerate() {
        if !root.is_dir() {
            return Err(SyncAnimBuildError::new(format!(
                "{} meshes root is not a directory: {}",
                if index == 0 { "mod" } else { "base" },
                root.display()
            )));
        }
    }
    let groups = complete_group_candidates(subgraphs)?;
    let mut resolver = ClipResolver::new(&roots);
    let mut outputs = Vec::with_capacity(groups.len());
    for (archetype, first_subgraphs, third_subgraphs) in groups {
        let first_person = resolve_group(&mut resolver, &first_subgraphs)?;
        let third_person = resolve_group(&mut resolver, &third_subgraphs)?;
        let first_person_entries = build_group(&mut resolver, &first_person)?;
        let third_person_entries = build_group(&mut resolver, &third_person)?;

        let mut out = String::from("V4\n2\n");
        serialize_group(&mut out, &first_person_entries, &first_person.subgraph_ids);
        serialize_group(&mut out, &third_person_entries, &third_person.subgraph_ids);
        outputs.push(WeaponSyncAnimData {
            filename: format!("ResolvedSyncAnimData{}.txt", archetype.to_ascii_lowercase()),
            body: out.into_bytes(),
        });
    }
    outputs.sort_by(|left, right| left.filename.cmp(&right.filename));
    Ok(outputs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_suffix_is_stripped_before_hashing() {
        let lead = animation_stem(r"Animations\Paired\PairedFrontBodySlam_AttackerLead.hkt")
            .map(strip_role_suffix)
            .unwrap();
        let victim = animation_stem(r"Animations\Paired\PairedFrontBodySlam_Victim.hkx")
            .map(strip_role_suffix)
            .unwrap();
        assert_eq!(lead, "PairedFrontBodySlam");
        assert_eq!(lead, victim);
        assert_eq!(name_id(lead), name_id(victim));

        let creature =
            animation_stem(r"Animations\Paired\PairedDogmeatAndHumanPetGreet_HumanLead.hkt")
                .map(strip_role_suffix)
                .unwrap();
        assert_eq!(creature, "PairedDogmeatAndHumanPetGreet");
    }

    #[test]
    fn transform_matrix_decodes_translation_and_wxyz_quaternion() {
        let transform = [
            -1.0, 0.0, 0.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 8.0, 55.0, -0.0, 1.0,
        ];
        let rotation = matrix_to_quaternion_wxyz(&transform).unwrap();
        assert_eq!(
            [transform[12], transform[13], transform[14]],
            [8.0, 55.0, -0.0]
        );
        assert_eq!(rotation, [0.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn serializer_uses_two_groups_signed_zero_canonical_ids_and_trailing_newline() {
        let entry = SyncAnimEntry {
            event: "pa_Test".to_string(),
            id: 7,
            translation: [-0.0, 1.25, -0.000_000_4],
            rotation_wxyz: [1.0, 0.0, 0.0, -0.0],
        };
        let mut body = String::from("V4\n2\n");
        serialize_group(&mut body, &[entry], &[11, 11]);
        serialize_group(&mut body, &[], &[13, 12, 13]);
        assert_eq!(
            body,
            "V4\n2\n1\npa_Test\n7\n-0.000000 1.250000 -0.000000\n\
             1.000000 0.000000 0.000000 -0.000000\n1\n11\n0\n2\n12\n13\n"
        );
        assert!(body.is_ascii());
        assert!(body.ends_with('\n'));
        assert!(!body.ends_with("\n\n"));
    }

    #[test]
    fn participant_serialization_is_byte_stable_for_arbitrary_permutations() {
        let permutations = [
            vec![u64::MAX, 7, 0, 42, 7],
            vec![42, 0, 7, u64::MAX, 42],
            vec![7, u64::MAX, 42, 0, 0],
            vec![u64::MAX, 7, 0, 42, 7],
        ];
        let bodies: Vec<_> = permutations
            .iter()
            .map(|ids| {
                let mut body = String::from("V4\n1\n");
                serialize_group(&mut body, &[], ids);
                body.into_bytes()
            })
            .collect();

        assert!(bodies.windows(2).all(|pair| pair[0] == pair[1]));
        let parsed = parse_sync_anim(&bodies[0]).unwrap();
        assert_eq!(parsed.groups[0].participant_ids, vec![0, 7, 42, u64::MAX]);
    }

    fn member(name: &str, value: HkxValue) -> HkxMember {
        HkxMember {
            name: name.to_string(),
            value,
        }
    }

    fn object(class_name: &str, members: Vec<HkxMember>) -> HkxObject {
        HkxObject {
            name: None,
            offset: 0,
            signature: 0,
            class_name: class_name.to_string(),
            members,
        }
    }

    fn string(value: &str) -> HkxValue {
        HkxValue::String {
            value: value.to_string(),
            is_null: false,
        }
    }

    #[test]
    fn malformed_event_table_fails_closed() {
        let objects = vec![object(
            "hkbBehaviorGraphStringData",
            vec![member(
                "eventNames",
                HkxValue::Array(vec![HkxValue::I32(7)]),
            )],
        )];
        let error = event_names(&objects).unwrap_err();
        assert!(error.to_string().contains("eventNames[0] is not a string"));
    }

    #[test]
    fn invalid_required_root_pointer_fails_closed() {
        let objects = vec![
            object(
                "hkbBehaviorGraphStringData",
                vec![member(
                    "eventNames",
                    HkxValue::Array(vec![string("pa_Test")]),
                )],
            ),
            object(
                "hkbBehaviorGraph",
                vec![member("rootGenerator", HkxValue::Pointer(Some(99)))],
            ),
        ];
        let error = collect_sync_candidates(&objects).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("root generator points outside the object table")
        );
    }

    #[test]
    fn graph_branch_outside_derived_sapt_directories_is_not_selected() {
        let temp = tempfile::tempdir().unwrap();
        let roots = [temp.path(), temp.path()];
        let mut resolver = ClipResolver::new(&roots);
        let behavior = r"Actors\Character\Behaviors\SyntheticSync.hkx";
        resolver.behavior_cache.insert(
            normalize_rel(behavior),
            BehaviorInfo {
                references: Vec::new(),
                candidates: vec![SyncCandidate {
                    event: "pa_Test".to_string(),
                    animation_name: "MissingLead.hkt".to_string(),
                }],
            },
        );
        let error = build_group(
            &mut resolver,
            &ResolvedGroup {
                behavior_roots: vec![behavior.to_string()],
                animation_dirs: vec![r"Actors\Character\Animations\Paired".to_string()],
                subgraph_ids: vec![1],
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("produced no resolved entries"));
    }

    #[test]
    fn selected_malformed_clip_fails_closed() {
        let temp = tempfile::tempdir().unwrap();
        let animation_dir = temp.path().join("Actors/Character/Animations/Paired");
        std::fs::create_dir_all(&animation_dir).unwrap();
        std::fs::write(animation_dir.join("BrokenLead.hkx"), b"not a packfile").unwrap();
        let roots = [temp.path(), temp.path()];
        let mut resolver = ClipResolver::new(&roots);
        let behavior = r"Actors\Character\Behaviors\SyntheticSync.hkx";
        resolver.behavior_cache.insert(
            normalize_rel(behavior),
            BehaviorInfo {
                references: Vec::new(),
                candidates: vec![SyncCandidate {
                    event: "pa_Test".to_string(),
                    animation_name: "BrokenLead.hkt".to_string(),
                }],
            },
        );
        let error = build_group(
            &mut resolver,
            &ResolvedGroup {
                behavior_roots: vec![behavior.to_string()],
                animation_dirs: vec![r"Actors\Character\Animations\Paired".to_string()],
                subgraph_ids: vec![1],
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("failed to parse"));
    }

    #[test]
    fn malformed_clip_packfile_fails_closed() {
        let temp = tempfile::tempdir().unwrap();
        let clip = temp.path().join("broken.hkx");
        std::fs::write(&clip, b"not a packfile").unwrap();
        let error = sync_anim_offset(&clip).unwrap_err();
        assert!(error.to_string().contains("failed to parse"));
    }

    #[test]
    fn participation_uses_canonical_archetype_and_person_context() {
        let subgraphs = vec![
            SubgraphInput {
                core_behavior: r"Actors\Character\Behaviors\WeaponBehavior.hkx".to_string(),
                sapt_chain: vec![
                    r"Actors\Character\Animations\Weapon\Alpha".to_string(),
                    r"Actors\Character\Animations\Paired".to_string(),
                ],
            },
            SubgraphInput {
                core_behavior: r"Actors\Character\Behaviors\WeaponBehavior.hkx".to_string(),
                sapt_chain: vec![
                    r"Actors\Character\Animations\Weapon\Beta".to_string(),
                    r"Actors\Character\Animations\Paired".to_string(),
                ],
            },
            SubgraphInput {
                core_behavior: r"Actors\Character\_1stPerson\Behaviors\GunBehavior.hkx".to_string(),
                sapt_chain: vec![
                    r"Actors\Character\_1stPerson\Animations\Alpha".to_string(),
                    r"Actors\Character\_1stPerson\Animations\Paired".to_string(),
                ],
            },
        ];
        let groups = complete_group_candidates(&subgraphs).unwrap();
        assert_eq!(groups.len(), 1);
        let (archetype, first_person, third_person) = &groups[0];
        assert_eq!(archetype, "alpha");
        assert_eq!(first_person.len(), 1);
        assert_eq!(third_person.len(), 1);
        assert_eq!(first_person[0].id(), subgraphs[2].id());
        assert_eq!(third_person[0].id(), subgraphs[0].id());
    }

    #[test]
    fn complete_archetypes_keep_matching_first_and_third_person_groups() {
        let mut subgraphs = Vec::new();
        for archetype in ["Beta", "Alpha"] {
            subgraphs.push(SubgraphInput {
                core_behavior: r"Actors\Character\Behaviors\WeaponBehavior.hkx".to_string(),
                sapt_chain: vec![format!(r"Actors\Character\Animations\Weapon\{archetype}")],
            });
            subgraphs.push(SubgraphInput {
                core_behavior: r"Actors\Character\_1stPerson\Behaviors\GunBehavior.hkx".to_string(),
                sapt_chain: vec![format!(
                    r"Actors\Character\_1stPerson\Animations\{archetype}\Specialized"
                )],
            });
        }

        let groups = complete_group_candidates(&subgraphs).unwrap();
        assert_eq!(
            groups
                .iter()
                .map(|(archetype, _, _)| archetype.as_str())
                .collect::<Vec<_>>(),
            ["alpha", "beta"]
        );
        for (archetype, first_person, third_person) in groups {
            assert!(
                first_person.iter().all(|subgraph| {
                    contains_first_person_archetype(subgraph, &archetype, true)
                })
            );
            assert!(
                third_person
                    .iter()
                    .all(|subgraph| contains_third_person_archetype(subgraph, &archetype))
            );
        }
    }

    #[test]
    fn public_builder_rejects_empty_production_input() {
        let temp = tempfile::tempdir().unwrap();
        let error = build_weapon_sync_anim_data(&[], [temp.path(), temp.path()]).unwrap_err();
        assert_eq!(error.to_string(), "no weapon subgraphs supplied");
    }

    #[derive(Debug, PartialEq, Eq)]
    struct ParsedSyncAnim {
        version: String,
        groups: Vec<ParsedSyncGroup>,
    }

    #[derive(Debug, PartialEq, Eq)]
    struct ParsedSyncGroup {
        entries: Vec<[String; 4]>,
        participant_ids: Vec<u64>,
    }

    fn parse_sync_anim(body: &[u8]) -> Option<ParsedSyncAnim> {
        let text = std::str::from_utf8(body).ok()?;
        let mut lines = text.lines();
        let version = lines.next()?.to_string();
        let group_count = lines.next()?.parse::<usize>().ok()?;
        let mut groups = Vec::new();
        for _ in 0..group_count {
            let entry_count = lines.next()?.parse::<usize>().ok()?;
            let mut entries = Vec::with_capacity(entry_count);
            for _ in 0..entry_count {
                entries.push([
                    lines.next()?.to_string(),
                    lines.next()?.to_string(),
                    lines.next()?.to_string(),
                    lines.next()?.to_string(),
                ]);
            }
            let participant_count = lines.next()?.parse::<usize>().ok()?;
            let participant_ids = (0..participant_count)
                .map(|_| lines.next()?.parse::<u64>().ok())
                .collect::<Option<Vec<_>>>()?;
            groups.push(ParsedSyncGroup {
                entries,
                participant_ids,
            });
        }
        lines
            .all(str::is_empty)
            .then_some(ParsedSyncAnim { version, groups })
    }

}
